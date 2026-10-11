//! The zk2 demo traffic for zengui (#614, FL5): a deployment of real zk2
//! services, and beside it the traffic no zk2 service would ever send.
//!
//! A conforming service cannot produce *non-conforming* traffic — that is
//! the whole point of one — so the key-agnostic path and the lens's honest
//! rungs need a source that sends both. This is it.
//!
//! **The owners** are real services, brought up in this process by the
//! engine `zenctl gen` runs (`zenkey_fleet::run_gen`): each a descriptor,
//! its bundles served on the bus (core §8.4, so a viewer retrieves the
//! contracts with no `--contracts`), its tokens, and every stream, state
//! and event of its contracts published through the runtime's writers,
//! which keep the contract's QoS and `Encoding` and the owner's stamps, and
//! keep core §2.7: a templated resource stays within its cardinality and an
//! event within its rate. Payloads are synthesized per (seed, tick); every
//! operation is served. All on one session, in the deployment namespace
//! (`--namespace`, `spray` by default), over `examples/zk2/walkthrough` and
//! `examples/zk2/tcgui`:
//!
//! | address | implements | what it shows |
//! |---|---|---|
//! | `robot1/nav`, `robot2/nav` | `nav.v2` | a protobuf stream, state with a freshness ttl, `set_origin`, and `reset` behind a capability |
//! | `robot1/camera` | `camera.v1` | an explicit `@stream`: no `**` reaches it |
//! | `robot1/perception` | `detections.v1` | a required role, `input`, bound to `robot1/camera` |
//! | `robot1/teleop` | `twist_cmd.v1` | a best-effort, express, real-time stream |
//! | `robot1/thrusters` | `thruster.v1` | operations, and a required role, `cmd`, bound to `robot1/teleop` |
//! | `fleet/planner` | `mission_plan.v1` | `plans/{vehicle}`, two members of a bound of 1000 |
//! | `host-a/tc` | `tc.netif.v1`, `tc.netem.v1` | JSON Schema, templated `{ns}/{iface}`, an event at rate `low` |
//! | `host-a/sysmon` | `health.v1` only | a status cycling OK → DEGRADED → OK |
//!
//! An event's rate caps it per run, so each owner runs in **terms** of an
//! hour (`TERM`): at the end of one its service is closed and brought up
//! again, as a new instance. A long session sees that restart once an hour.
//!
//! **The non-conforming traffic** comes from a second session, in no
//! namespace, as a stranger to the deployment would send it:
//!
//! - an **interface token with no instance token** under `robot9/phantom`
//!   — the doctor's `token-missing`;
//! - a **second answerer** on `host-a/tc`'s `namespaces` state, stamped by
//!   this session's clock — `zenctl doctor --deep`'s `state-stamp-foreign`,
//!   and a stamp that is not the owner's to any reader (O7);
//! - **garbage on an owned key**, `robot1/nav`'s `position` stream, stamped
//!   by this session's clock: undecodable, and not the owner's stamp;
//! - an **unowned zk2 key**, `robot1/ghost`'s `position`: no provider;
//! - a zk2 key **in another namespace**, `elsewhere/zk2/…`: not in this one;
//! - **foreign keys**: plain ones, a CBOR payload, a two-chunk key, a 512
//!   KiB payload, a v1 key (foreign to zk2) whose `value` wanders, a key
//!   retired by tombstone and revived on a cycle, and `--keys N` more under
//!   `soak/`.
//!
//! One process, no router to install: with `-l` the owners' session is a
//! router listening there, with the admin space on, and everything else,
//! zengui and zenctl included, connects to it as a client. With `-c` it
//! joins a router as a client. Two sprays on one bus are two writers of
//! every key: run one.
//!
//! ```text
//! cargo run -p zengui --example spray -- -l tcp/127.0.0.1:7449   # listen: no router needed
//! cargo run -p zengui --example spray -- -c tcp/127.0.0.1:7447   # or join one
//! cargo run -p zenctl -- service list --namespace spray -c tcp/127.0.0.1:7449
//! cargo run -p zenctl -- doctor --namespace spray --deep --clocks-synced -c tcp/127.0.0.1:7449
//! ```
//!
//! zengui reads it on zk2 (#614): `just gui-demo` opens it with
//! `--namespace spray`, and each key here resolves through presence and the
//! contracts to its rung — the owners' keys resolved and checked valid, the
//! squatter's invalid, the garbage undecodable, the unowned key at "no
//! provider", the other namespace's at "not in namespace" — while the
//! foreign keys keep their counts, rates and structural payloads, which is
//! the key-agnostic core's promise.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use zenkey::health::Level;
use zenkey::{ServiceBuilder, ServiceConfig};
use zenkey_fleet::report::GenReport;
use zenkey_fleet::{ContractSet, GenPattern, GenSpec, MemberArg, Revision};
use zenkey_model::grammar::{Addr, IfaceId, InstanceId, KindToken, alive_key, data_key};
use zenoh::Wait as _;

#[derive(Parser, Debug, Clone)]
#[command(about = "a zk2 deployment and the traffic no zk2 service would send, for zengui")]
pub struct Args {
    /// A router to join as a client, repeatable.
    #[arg(long, short = 'c')]
    pub connect: Vec<String>,
    /// An endpoint to listen on, repeatable: the owners' session is then a
    /// router, and the demo needs nothing else running.
    #[arg(long, short = 'l')]
    pub listen: Vec<String>,
    /// The deployment namespace the owners run in. Empty is the bus root.
    #[arg(long, default_value = "spray")]
    pub namespace: String,
    /// Publications per second of each raw key (the owners publish at their
    /// contracts' pace: a stream at 1 Hz, a state at 0.5 Hz, an event within
    /// its rate).
    #[arg(long, default_value_t = 5.0)]
    pub hz: f64,
    /// Extra foreign keys, `soak/g<n>/k<i>` in groups of a hundred: each put
    /// once at start, then a rotating slice refreshed every tick.
    #[arg(long, default_value_t = 0)]
    pub keys: usize,
    /// Drives every owner's synthesized payloads: the same seed is the same
    /// demo.
    #[arg(long, default_value_t = 42)]
    pub seed: u64,
    /// The directory holding the `walkthrough` and `tcgui` example
    /// contracts.
    #[arg(long, default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/zk2"))]
    pub contracts: PathBuf,
}

/// One owner: an address, the interfaces it implements, its role bindings,
/// and the members of its templated resources (`zenctl gen --member`).
struct Owner {
    address: &'static str,
    interfaces: &'static [&'static str],
    bindings: &'static [(&'static str, &'static [&'static str])],
    members: &'static [&'static str],
}

/// The deployment, in bring-up order: a provider before the role that
/// binds it.
const OWNERS: &[Owner] = &[
    Owner {
        address: "robot1/nav",
        interfaces: &["nav.v2"],
        bindings: &[],
        members: &[],
    },
    Owner {
        address: "robot2/nav",
        interfaces: &["nav.v2"],
        bindings: &[],
        members: &[],
    },
    Owner {
        address: "robot1/camera",
        interfaces: &["camera.v1"],
        bindings: &[],
        members: &[],
    },
    Owner {
        address: "robot1/perception",
        interfaces: &["detections.v1"],
        bindings: &[("input", &["robot1/camera"])],
        members: &[],
    },
    Owner {
        address: "robot1/teleop",
        interfaces: &["twist_cmd.v1"],
        bindings: &[],
        members: &[],
    },
    Owner {
        address: "robot1/thrusters",
        interfaces: &["thruster.v1"],
        bindings: &[("cmd", &["robot1/teleop"])],
        members: &[],
    },
    Owner {
        address: "fleet/planner",
        interfaces: &["mission_plan.v1"],
        bindings: &[],
        members: &["plans/{vehicle}=robot1,robot2"],
    },
    Owner {
        address: "host-a/tc",
        interfaces: &["tc.netif.v1", "tc.netem.v1"],
        bindings: &[],
        members: &[
            "interfaces/{ns}/{iface}=default/eth0,default/wlan0",
            "bandwidth/{ns}/{iface}=default/eth0,default/wlan0",
            "config/{ns}/{iface}=default/eth0,default/wlan0",
            "qdisc/{ns}/{iface}=default/eth0,default/wlan0",
            "plug/{ns}/{iface}=default/eth0,default/wlan0",
        ],
    },
];

/// The service implementing only `health.v1`, its status cycling.
pub const SYSMON: &str = "host-a/sysmon";

/// How long a status holds before the next one in the cycle.
const HEALTH_PHASE: Duration = Duration::from_secs(30);

/// One owner's run: an event's rate caps its occurrences per run, so an
/// owner closes and comes up again every term (the module doc).
const TERM: Duration = Duration::from_secs(3600);

/// The address holding an interface token and no instance token.
pub const PHANTOM: &str = "robot9/phantom";

/// The owner, interface and resource the squatter answers for.
pub const SQUATTED: (&str, &str, &str) = ("host-a/tc", "tc.netif.v1", "namespaces");

/// How many ticks one retire/revive cycle takes; its last is the tombstone.
const CYCLE: u64 = 20;

/// A raw key's payload on each tick.
enum Payload {
    /// The same bytes every tick.
    Fixed(Vec<u8>),
    /// A JSON document whose `value` wanders: something to plot and diff.
    Wander,
    /// Puts, then a tombstone on the cycle's last tick, then puts again.
    Cycle,
}

impl Payload {
    /// The bytes for this tick, or `None` for a tombstone.
    fn bytes(&self, tick: u64) -> Option<Vec<u8>> {
        match self {
            Payload::Fixed(b) => Some(b.clone()),
            Payload::Wander => Some(
                format!(
                    r#"{{"value":{},"unit":"percent"}}"#,
                    wander(tick, 20.0, 90.0)
                )
                .into_bytes(),
            ),
            Payload::Cycle => (tick % CYCLE != CYCLE - 1)
                .then(|| format!(r#"{{"status":"ok","epoch":{}}}"#, tick / CYCLE).into_bytes()),
        }
    }
}

/// A deterministic wander in `[lo, hi]`: a sine of the tick, so the demo
/// looks the same on every run.
fn wander(tick: u64, lo: f64, hi: f64) -> f64 {
    let unit = (((tick as f64) * 0.37).sin() + 1.0) / 2.0;
    ((lo + unit * (hi - lo)) * 10.0).round() / 10.0
}

/// One raw key: where, what, how often, and whether it carries this
/// session's stamp.
struct Raw {
    key: String,
    payload: Payload,
    /// Published every `every` ticks.
    every: u64,
    stamped: bool,
    why: &'static str,
}

/// A running spray. [`Spray::stop`] takes it down.
pub struct Spray {
    /// Where to connect: the owners' router's own locators when it
    /// listens, else the routers it joined.
    pub endpoints: Vec<String>,
    /// Each owner's instance, as its first term came up.
    pub instances: BTreeMap<String, String>,
    owners: zenoh::Session,
    raw: zenoh::Session,
    tasks: tokio::task::JoinSet<()>,
    _phantom: zenoh::liveliness::LivelinessToken,
    _squatter: zenoh::query::Queryable<()>,
}

/// The contracts the owners implement, under `examples/zk2`: the files by
/// name, since `tcgui/` also holds a bindings file, which is no contract.
const CONTRACTS: &[&str] = &[
    "walkthrough/nav.v2.toml",
    "walkthrough/camera.v1.toml",
    "walkthrough/detections.v1.toml",
    "walkthrough/twist_cmd.v1.toml",
    "walkthrough/thruster.v1.toml",
    "walkthrough/mission_plan.v1.toml",
    "tcgui/tc.netif.v1.toml",
    "tcgui/tc.netem.v1.toml",
];

/// Every file of [`CONTRACTS`] under `root`, loaded; a file that does not
/// load stops the demo, with what was wrong.
fn load(root: &Path) -> anyhow::Result<ContractSet> {
    let files: Vec<PathBuf> = CONTRACTS.iter().map(|f| root.join(f)).collect();
    let (set, problems) = ContractSet::load_files(&files);
    if !problems.is_empty() {
        let said: Vec<String> = problems
            .iter()
            .map(|p| format!("{}: {}", p.at, p.message))
            .collect();
        anyhow::bail!(
            "the example contracts did not load (pass --contracts <examples/zk2>):\n{}",
            said.join("\n")
        );
    }
    Ok(set)
}

/// The one revision of `iface` the examples hold.
fn revision(set: &ContractSet, iface: &str) -> anyhow::Result<Arc<Revision>> {
    let id: IfaceId = iface.parse()?;
    set.of_iface(&id)
        .next()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("the examples hold no revision of {iface}"))
}

fn endpoint_list(endpoints: &[String]) -> anyhow::Result<String> {
    Ok(serde_json::to_string(endpoints)?)
}

/// The owners' session: a router when it listens, a client of the routers
/// it joins otherwise, in the deployment namespace.
fn owners_config(args: &Args) -> anyhow::Result<zenoh::Config> {
    anyhow::ensure!(
        !args.listen.is_empty() || !args.connect.is_empty(),
        "say where: -l <endpoint> to listen (no router needed), or -c <endpoint> to join one"
    );
    let mut c = zenoh::Config::default();
    let set = |c: &mut zenoh::Config, k: &str, v: &str| {
        c.insert_json5(k, v)
            .map_err(|e| anyhow::anyhow!("{k} = {v}: {e}"))
    };
    set(&mut c, "scouting/multicast/enabled", "false")?;
    if args.listen.is_empty() {
        set(&mut c, "mode", "\"client\"")?;
        // Core §4.3: a session that serves state runs its HLC.
        set(&mut c, "timestamping/enabled", "true")?;
    } else {
        set(&mut c, "mode", "\"router\"")?;
        set(&mut c, "listen/endpoints", &endpoint_list(&args.listen)?)?;
        // So `zenctl admin` and the doctor's router checks have a router
        // to read.
        set(&mut c, "adminspace/enabled", "true")?;
    }
    if !args.connect.is_empty() {
        set(&mut c, "connect/endpoints", &endpoint_list(&args.connect)?)?;
    }
    if !args.namespace.is_empty() {
        zenkey_fleet::check_namespace(&args.namespace)?;
        set(
            &mut c,
            "namespace",
            &serde_json::to_string(&args.namespace)?,
        )?;
    }
    Ok(c)
}

/// The stranger's session: a client of the owners' router, in no namespace.
async fn raw_session(endpoints: &[String]) -> anyhow::Result<zenoh::Session> {
    let mut c = zenoh::Config::default();
    c.insert_json5("scouting/multicast/enabled", "false")
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    c.insert_json5("mode", "\"client\"")
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    c.insert_json5("connect/endpoints", &endpoint_list(endpoints)?)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    zenoh::open(c).await.map_err(|e| anyhow::anyhow!("{e}"))
}

/// One owner's terms, forever. The first term's instance, or why it did not
/// come up, goes to `up`; later terms are said on stdout.
async fn own(
    session: zenoh::Session,
    spec: GenSpec,
    up: tokio::sync::oneshot::Sender<Result<String, String>>,
) {
    let mut up = Some(up);
    loop {
        let plan = match zenkey_fleet::gen_plan(&spec) {
            Ok(p) => p,
            Err(e) => {
                if let Some(tx) = up.take() {
                    let _ = tx.send(Err(e.to_string()));
                }
                return;
            }
        };
        let ran = zenkey_fleet::run_gen(&session, &spec, &plan, |instance| match up.take() {
            Some(tx) => {
                let _ = tx.send(Ok(instance.to_owned()));
            }
            None => println!("{}: a new term, as instance {instance}", spec.address),
        })
        .await;
        match ran {
            Ok(GenReport {
                sent,
                failed,
                calls,
                first_errors,
                ..
            }) => {
                println!(
                    "{}: term over — {sent} sent, {failed} failed, {calls} call(s) answered",
                    spec.address
                );
                for e in first_errors {
                    println!("  {e}");
                }
            }
            Err(e) => {
                if let Some(tx) = up.take() {
                    let _ = tx.send(Err(e.to_string()));
                    return;
                }
                eprintln!("{}: {e}; trying again in 5 s", spec.address);
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

/// `host-a/sysmon`: a service implementing `health.v1` and nothing else,
/// its status cycling OK → DEGRADED → OK every [`HEALTH_PHASE`].
async fn sysmon(
    session: &zenoh::Session,
) -> anyhow::Result<(zenkey::Service, zenkey::health::Health)> {
    let addr: Addr = SYSMON.parse()?;
    let mut b = ServiceBuilder::new(session, ServiceConfig::new(addr));
    let health = b.health().await?;
    // Before the tokens (health.v1 §2.3), so a GET made the moment they
    // appear finds a level.
    health.set_status(Level::Ok, "nominal").await?;
    let svc = b.start().await?;
    Ok((svc, health))
}

impl Spray {
    /// Brings every owner up and starts the raw traffic. Returns once every
    /// owner's first term is up, or the first that could not come up.
    pub async fn start(args: &Args) -> anyhow::Result<Spray> {
        let contracts = load(&args.contracts)?;

        // Every plan before anything is brought up: a contract that will
        // not plan stops the demo with nothing declared.
        let mut specs = Vec::new();
        for (i, o) in OWNERS.iter().enumerate() {
            let spec = GenSpec {
                address: o.address.parse()?,
                revisions: o
                    .interfaces
                    .iter()
                    .map(|iface| revision(&contracts, iface))
                    .collect::<anyhow::Result<_>>()?,
                members: o
                    .members
                    .iter()
                    .map(|m| MemberArg::parse(m))
                    .collect::<Result<_, _>>()?,
                bindings: o
                    .bindings
                    .iter()
                    .map(|(role, providers)| {
                        (
                            (*role).to_owned(),
                            providers.iter().map(|p| (*p).to_owned()).collect(),
                        )
                    })
                    .collect(),
                rate_hz: None,
                pattern: GenPattern::Steady,
                duration: TERM,
                seed: args.seed.wrapping_add(i as u64),
                tool: "zengui spray".into(),
            };
            zenkey_fleet::gen_plan(&spec).map_err(|e| anyhow::anyhow!("{}: {e}", o.address))?;
            specs.push(spec);
        }

        let owners = zenoh::open(owners_config(args)?)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let endpoints: Vec<String> = if args.listen.is_empty() {
            args.connect.clone()
        } else {
            owners
                .info()
                .locators()
                .await
                .into_iter()
                .map(|l| l.to_string())
                .collect()
        };
        anyhow::ensure!(!endpoints.is_empty(), "the router listens nowhere");
        let raw = raw_session(&endpoints).await?;

        let mut tasks = tokio::task::JoinSet::new();
        let mut ups = Vec::new();
        for spec in specs {
            let (tx, rx) = tokio::sync::oneshot::channel();
            ups.push((spec.address.to_string(), rx));
            tasks.spawn(own(owners.clone(), spec, tx));
        }
        let mut instances = BTreeMap::new();
        for (address, rx) in ups {
            let came = tokio::time::timeout(Duration::from_secs(60), rx)
                .await
                .map_err(|_| anyhow::anyhow!("{address} did not come up within 60 s"))?
                .map_err(|_| anyhow::anyhow!("{address}'s task ended"))?
                .map_err(|e| anyhow::anyhow!("{address}: {e}"))?;
            instances.insert(address, came);
        }

        let (svc, health) = sysmon(&owners).await?;
        instances.insert(SYSMON.to_owned(), svc.instance().to_string());
        tasks.spawn(async move {
            let _svc = svc;
            let mut degraded = false;
            loop {
                tokio::time::sleep(HEALTH_PHASE).await;
                degraded = !degraded;
                let set = if degraded {
                    health
                        .set_status(Level::Degraded, "demo: /var/log is 91% full")
                        .await
                } else {
                    health.set_status(Level::Ok, "nominal").await
                };
                if let Err(e) = set {
                    eprintln!("{SYSMON}: {e}");
                }
            }
        });

        let ns = args.namespace.as_str();
        let in_ns = |k: &zenkey_model::grammar::Key| zenkey_fleet::with_namespace(ns, k.as_str());

        // The phantom: an interface token under an address no instance
        // token names (core §8.1), naming nav.v2's revision.
        let nav = revision(&contracts, "nav.v2")?;
        let phantom = in_ns(&alive_key(
            &PHANTOM.parse()?,
            nav.iface(),
            &InstanceId::from_u64(0x5ca1_ab1e_0000_0001),
            &nav.fingerprint().hex().fp16(),
        )?);
        let _phantom = raw
            .liveliness()
            .declare_token(phantom.clone())
            .await
            .map_err(|e| anyhow::anyhow!("token {phantom}: {e}"))?;

        // The squatter: a second answerer on an owner's state, with this
        // session's stamp (core §4.2 S1).
        let (owner, iface, resource) = SQUATTED;
        let squatted = in_ns(&data_key(
            &owner.parse()?,
            &iface.parse()?,
            KindToken::State,
            &[resource],
        )?);
        let stamper = raw.clone();
        let reply_key = squatted.clone();
        let _squatter = raw
            .declare_queryable(squatted.clone())
            .callback(move |q| {
                let _ = q
                    .reply(
                        reply_key.clone(),
                        r#"{"namespaces":["default","squatted"]}"#,
                    )
                    .timestamp(stamper.new_timestamp())
                    .wait();
            })
            .await
            .map_err(|e| anyhow::anyhow!("queryable {squatted}: {e}"))?;

        let nav_position = |addr: &str| -> anyhow::Result<zenkey_model::grammar::Key> {
            Ok(data_key(
                &addr.parse()?,
                nav.iface(),
                KindToken::Stream,
                &["position"],
            )?)
        };
        let per_second = (args.hz.max(0.1).round() as u64).max(1);
        let raws = vec![
            Raw {
                key: in_ns(&nav_position("robot1/nav")?),
                payload: Payload::Fixed(b"\xff\xfe not a nav.v2.Position".to_vec()),
                every: per_second,
                stamped: true,
                why: "garbage on an owned key, with a stamp not the owner's",
            },
            Raw {
                key: in_ns(&nav_position("robot1/ghost")?),
                payload: Payload::Fixed(vec![0x09, 0, 0, 0, 0, 0, 0, 0xf0, 0x3f]),
                every: 1,
                stamped: false,
                why: "an unowned zk2 key: no provider",
            },
            Raw {
                key: zenkey_fleet::with_namespace(
                    "elsewhere",
                    nav_position("robot9/nav")?.as_str(),
                ),
                payload: Payload::Fixed(br#"{"another":"deployment"}"#.to_vec()),
                every: 1,
                stamped: false,
                why: "a zk2 key in another namespace",
            },
            Raw {
                key: "demo/example/foo".into(),
                payload: Payload::Fixed(b"just a plain string".to_vec()),
                every: 1,
                stamped: false,
                why: "foreign",
            },
            Raw {
                key: "demo/cbor".into(),
                payload: Payload::Fixed(vec![0xA1, 0x61, 0x61, 0x01]),
                every: 1,
                stamped: false,
                why: "foreign, CBOR",
            },
            Raw {
                key: "two/chunks".into(),
                payload: Payload::Fixed(b"{}".to_vec()),
                every: 1,
                stamped: false,
                why: "foreign, two chunks",
            },
            Raw {
                key: "demo/huge".into(),
                payload: Payload::Fixed(vec![b'x'; 512 * 1024]),
                every: 1,
                stamped: false,
                why: "foreign, 512 KiB",
            },
            Raw {
                key: "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used".into(),
                payload: Payload::Wander,
                every: 1,
                stamped: false,
                why: "a v1 key, foreign to zk2; its value wanders",
            },
            Raw {
                key: "demo/cycle".into(),
                payload: Payload::Cycle,
                every: 1,
                stamped: false,
                why: "foreign, retired by tombstone and revived",
            },
        ];
        let soak: Vec<String> = (0..args.keys)
            .map(|i| format!("soak/g{}/k{i}", i / 100))
            .collect();
        for key in &soak {
            let _ = raw.put(key, b"1".to_vec()).await;
        }

        println!(
            "non-conforming, from a session in no namespace:\n  {phantom}\n      an interface \
             token and no instance token (doctor: token-missing)\n  {squatted}\n      a second \
             answerer, stamped by another clock (doctor --deep: state-stamp-foreign)"
        );
        for r in &raws {
            println!("  {}\n      {}", r.key, r.why);
        }
        if !soak.is_empty() {
            println!(
                "  soak/g*/k* ({} keys)\n      foreign, the big-tree soak",
                soak.len()
            );
        }

        let publisher = raw.clone();
        let period = Duration::from_secs_f64(1.0 / args.hz.max(0.1));
        let refresh = (args.keys / 1000).max(1);
        tasks.spawn(async move {
            let mut ticker = tokio::time::interval(period);
            let (mut tick, mut cursor) = (0u64, 0usize);
            loop {
                ticker.tick().await;
                for r in raws.iter().filter(|r| tick.is_multiple_of(r.every)) {
                    match r.payload.bytes(tick) {
                        Some(bytes) if r.stamped => {
                            let _ = publisher
                                .put(&r.key, bytes)
                                .timestamp(publisher.new_timestamp())
                                .await;
                        }
                        Some(bytes) => {
                            let _ = publisher.put(&r.key, bytes).await;
                        }
                        None => {
                            let _ = publisher.delete(&r.key).await;
                        }
                    }
                }
                for _ in 0..refresh.min(soak.len()) {
                    let _ = publisher
                        .put(&soak[cursor % soak.len()], b"1".to_vec())
                        .await;
                    cursor += 1;
                }
                tick = tick.wrapping_add(1);
            }
        });

        Ok(Spray {
            endpoints,
            instances,
            owners,
            raw,
            tasks,
            _phantom,
            _squatter,
        })
    }

    /// Stops every task and closes both sessions: the owners' tokens and
    /// queryables go with theirs.
    pub async fn stop(mut self) {
        self.tasks.shutdown().await;
        let _ = self.raw.close().await;
        let _ = self.owners.close().await;
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let spray = Spray::start(&args).await?;
    let ns = if args.namespace.is_empty() {
        "the bus root".to_owned()
    } else {
        format!("namespace {:?}", args.namespace)
    };
    println!("owners, in {ns}:");
    for (address, instance) in &spray.instances {
        println!("  {address:<18} instance {instance}");
    }
    println!(
        "{} {}",
        if args.listen.is_empty() {
            "joined"
        } else {
            "listening on"
        },
        spray.endpoints.join(", ")
    );
    println!("\nCtrl-C to stop.");
    tokio::signal::ctrl_c().await?;
    spray.stop().await;
    Ok(())
}
