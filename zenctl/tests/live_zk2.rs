//! zk2's inspection verbs against live services (#612, FJ4): the real
//! `zenctl` binary, run as a script would run it, against services the zk2
//! runtime brings up with its own `ServiceBuilder` from the tcgui pilot's
//! contracts (`examples/zk2/tcgui`).
//!
//! The deployment, per case and on a router of its own (an ephemeral port,
//! nothing shared between cases):
//!
//! * `host-a/tc` implements `tc.netif.v1` and `tc.netem.v1`, every resource
//!   exposed, a token for each;
//! * `host-b/tc` implements the same two, with `tc.netem.v1` in the
//!   deployment's tokenless set (U22): known from its descriptor alone;
//! * `ws-01/tcgui-frontend` implements nothing and binds three roles to
//!   `*/tc` — `netif` and `netem`, which select both hosts, and `scenario`,
//!   which no service provides.
//!
//! What is asserted is what zenctl adds on top of the fleet's zk2 core (whose
//! own suite, `zenkey-fleet/tests/zk2_core.rs`, pins the projections): the
//! namespaced session, the flags, the JSON each verb prints and its exit
//! code. A `zenctl` run is a new process with a new session, so a case that
//! expects an answer reruns until it appears, within [`SETTLE`] — returning
//! the moment it does.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;
use zk2::model::contract::{Contract, load_path};
use zk2::model::grammar::IfaceId;
use zk2::{Implementation, Service, ServiceBuilder, ServiceConfig};

/// The net a case waits under before calling a hang a hang.
const SETTLE: Duration = Duration::from_secs(20);
/// One `zenctl` run's limit: above any wait a case asks for.
const RUN_LIMIT: Duration = Duration::from_secs(60);

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2")
}

fn contract(path: &str) -> Contract {
    let full = examples().join(format!("{path}.toml"));
    let l = load_path(&full);
    l.contract
        .unwrap_or_else(|| panic!("{path} does not load:\n{}", l.report))
}

fn iface(s: &str) -> IfaceId {
    s.parse().expect("an interface")
}

/// The fingerprint a service implementing `path` serves.
fn fingerprint(path: &str) -> String {
    Implementation::new(contract(path))
        .fingerprint()
        .to_string()
}

/// A fingerprint's first 16 hex digits: what its interface token carries.
fn fp16(fp: &str) -> &str {
    let hex = fp.strip_prefix("sha256:").expect("a fingerprint");
    &hex[..16]
}

fn base_config() -> zenoh::Config {
    let mut c = zenoh::Config::default();
    c.insert_json5("scouting/multicast/enabled", "false")
        .expect("config");
    c.insert_json5("scouting/gossip/enabled", "false")
        .expect("config");
    c
}

/// A router on an ephemeral loopback port, and the endpoint it bound.
async fn router() -> (zenoh::Session, String) {
    let mut c = base_config();
    c.insert_json5("mode", "\"router\"").expect("config");
    c.insert_json5("listen/endpoints", r#"["tcp/127.0.0.1:0"]"#)
        .expect("config");
    let r = zenoh::open(c).await.expect("a router");
    let ep = r
        .info()
        .locators()
        .await
        .into_iter()
        .map(|l| l.to_string())
        .find(|l| l.starts_with("tcp/127.0.0.1:"))
        .expect("the router listens on loopback");
    (r, ep)
}

/// A client of the router, in `namespace` when given: the owners' session.
async fn client(endpoint: &str, namespace: Option<&str>) -> zenoh::Session {
    let mut c = base_config();
    c.insert_json5("mode", "\"client\"").expect("config");
    c.insert_json5("connect/endpoints", &format!("[\"{endpoint}\"]"))
        .expect("config");
    if let Some(ns) = namespace {
        c.insert_json5("namespace", &format!("\"{ns}\""))
            .expect("config");
    }
    zenoh::open(c).await.expect("a client")
}

/// Brings up `cfg` implementing every contract in `paths`, every resource
/// exposed.
async fn bring_up(session: &zenoh::Session, cfg: ServiceConfig, paths: &[&str]) -> Service {
    let mut b = ServiceBuilder::new(session, cfg);
    for path in paths {
        let c = contract(path);
        let id = c.iface.clone();
        let names: Vec<String> = c
            .resources
            .iter()
            .map(zk2::implementation::resource_name)
            .collect();
        b.implement(Implementation::new(c)).expect("implement");
        for n in names {
            let _ = b.expose(&id, &n);
        }
    }
    b.start().await.expect("start")
}

const ADDRESSES: [&str; 3] = ["host-a/tc", "host-b/tc", "ws-01/tcgui-frontend"];

/// The deployment of the module doc, on `owners`.
async fn tcgui(owners: &zenoh::Session) -> Vec<Service> {
    let addr = |s: &str| s.parse().expect("an address");
    let both = ["tcgui/tc.netif.v1", "tcgui/tc.netem.v1"];
    let a = bring_up(owners, ServiceConfig::new(addr("host-a/tc")), &both).await;
    let b = bring_up(
        owners,
        ServiceConfig::new(addr("host-b/tc")).tokenless(iface("tc.netem.v1")),
        &both,
    )
    .await;
    let mut gui = ServiceBuilder::new(
        owners,
        ServiceConfig::new(addr("ws-01/tcgui-frontend"))
            .bind("netif", &["*/tc"])
            .bind("netem", &["*/tc"])
            .bind("scenario", &["*/tc"]),
    );
    gui.require("netif", iface("tc.netif.v1"), false);
    gui.require("netem", iface("tc.netem.v1"), false);
    gui.require("scenario", iface("tc.scenario.v1"), false);
    vec![a, b, gui.start().await.expect("the frontend")]
}

/// One case's bus: the router, the owners and their services, and where
/// zenctl connects.
struct Bus {
    endpoint: String,
    home: PathBuf,
    _router: zenoh::Session,
    _owners: zenoh::Session,
    _services: Vec<Service>,
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

impl Bus {
    /// The tcgui deployment, in `namespace` when given.
    async fn up(namespace: Option<&str>) -> Bus {
        static NTH: AtomicU64 = AtomicU64::new(0);
        let (router, endpoint) = router().await;
        let owners = client(&endpoint, namespace).await;
        let services = tcgui(&owners).await;
        let home = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("live-zk2-home")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NTH.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::create_dir_all(&home).expect("a config root of its own");
        Bus {
            endpoint,
            home,
            _router: router,
            _owners: owners,
            _services: services,
        }
    }

    /// `zenctl <args> -c <endpoint>`, once.
    async fn zenctl(&self, args: &[&str]) -> Run {
        let mut argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        argv.extend(["-c".into(), self.endpoint.clone()]);
        let home = self.home.clone();
        tokio::task::spawn_blocking(move || run(&argv, &home))
            .await
            .expect("the zenctl runner")
    }

    /// Rerun until `done` holds, within [`SETTLE`]; the last run either way.
    async fn until(&self, args: &[&str], done: impl Fn(&Run) -> bool) -> Run {
        let deadline = Instant::now() + SETTLE;
        loop {
            let r = self.zenctl(args).await;
            if done(&r) || Instant::now() >= deadline {
                return r;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

/// One finished `zenctl` run.
#[derive(Debug)]
struct Run {
    argv: Vec<String>,
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    fn json(&self) -> Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|e| panic!("stdout is not one JSON document ({e}):\n{self}"))
    }
}

impl std::fmt::Display for Run {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "$ zenctl {}\nexit {}\n--- stdout\n{}--- stderr\n{}",
            self.argv.join(" "),
            self.code,
            self.stdout,
            self.stderr
        )
    }
}

/// The run itself: a hermetic environment, stdin closed, both pipes
/// drained on threads of their own, killed at [`RUN_LIMIT`].
fn run(argv: &[String], home: &Path) -> Run {
    use std::io::Read as _;
    let mut child = Command::new(env!("CARGO_BIN_EXE_zenctl"))
        .args(argv)
        .env_remove("ZENCTL_CONTEXT")
        .env_remove("ZENKEY_EXPLORER_CONTEXT")
        .env_remove("ZENCTL_ZENOH_CONFIG")
        .env_remove("ZENCTL_FORMAT")
        .env_remove("ZENCTL_BASE")
        .env("ZENKEY_EXPLORER_CONFIG_DIR", home)
        .env("NO_COLOR", "1")
        .env("RUST_LOG", "off")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn zenctl");
    let drain = |mut pipe: Box<dyn std::io::Read + Send>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = pipe.read_to_string(&mut s);
            s
        })
    };
    let out = drain(Box::new(child.stdout.take().expect("stdout")));
    let err = drain(Box::new(child.stderr.take().expect("stderr")));
    let deadline = Instant::now() + RUN_LIMIT;
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait on zenctl") {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    Run {
        argv: argv.to_vec(),
        code: status.and_then(|s| s.code()).unwrap_or(-1),
        stdout: out.join().expect("stdout drain"),
        stderr: err.join().expect("stderr drain"),
    }
}

#[track_caller]
fn exits(run: &Run, code: i32) {
    assert_eq!(run.code, code, "wrong exit code\n{run}");
}

/// Every address listed, every descriptor served: what a case waits for
/// before it asserts anything, because a service is up when presence says
/// so, not when its owner's `start()` returned.
fn settled(r: &Run) -> bool {
    if r.code != 0 {
        return false;
    }
    let Ok(doc) = serde_json::from_str::<Value>(&r.stdout) else {
        return false;
    };
    let rows = doc["rows"].as_array().cloned().unwrap_or_default();
    let seen: BTreeSet<&str> = rows.iter().filter_map(|i| i["address"].as_str()).collect();
    seen == BTreeSet::from(ADDRESSES) && rows.iter().all(|i| i["descriptor"]["answer"] == "served")
}

/// `service list --format json` until settled.
async fn listing(bus: &Bus, extra: &[&str]) -> Run {
    let mut args = vec!["service", "list", "--timeout", "2", "--format", "json"];
    args.extend_from_slice(extra);
    bus.until(&args, settled).await
}

/// The instance row of `address` in a `service list` document.
fn instance<'a>(doc: &'a Value, address: &str) -> &'a Value {
    doc["rows"]
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["address"] == address))
        .unwrap_or_else(|| panic!("{address} is listed: {doc}"))
}

/// One interface row of an instance.
fn iface_row<'a>(instance: &'a Value, iface: &str) -> &'a Value {
    instance["interfaces"]
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["iface"] == iface))
        .unwrap_or_else(|| panic!("{iface} is a row: {instance}"))
}

/// `service list`: every service from its tokens and its descriptor, the
/// read complete; each interface keeps the token's prefix beside the
/// descriptor's fingerprint, which agree; the tokenless interface has the
/// descriptor's row and no token; the pure consumer holds an instance token
/// and provides nothing. `iface list` reads the same presence.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn service_list_shows_tokens_descriptors_and_the_tokenless_set() {
    let bus = Bus::up(None).await;
    let run = listing(&bus, &[]).await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["report"], "service-list");
    assert_eq!(doc["complete"], true, "{run}");
    assert_eq!(doc["selector"], "zk2/*/*/@zk/**");

    let (netif, netem) = (
        fingerprint("tcgui/tc.netif.v1"),
        fingerprint("tcgui/tc.netem.v1"),
    );
    let a = instance(&doc, "host-a/tc");
    assert_eq!(a["instance_token"], true);
    for (name, fp) in [("tc.netif.v1", &netif), ("tc.netem.v1", &netem)] {
        let row = iface_row(a, name);
        assert_eq!(row["contract"], fp.as_str(), "{row}");
        // The token carries the fingerprint's first 64 bits (§8.1).
        assert_eq!(row["token"], fp16(fp), "{row}");
    }
    let b = instance(&doc, "host-b/tc");
    let tokenless = iface_row(b, "tc.netem.v1");
    assert_eq!(tokenless["tokenless"], true, "{tokenless}");
    assert!(tokenless.get("token").is_none(), "no token: {tokenless}");
    assert_eq!(tokenless["contract"], netem.as_str());
    let gui = instance(&doc, "ws-01/tcgui-frontend");
    assert_eq!(gui["instance_token"], true);
    assert_eq!(gui["interfaces"], serde_json::json!([]));

    // One system: the same read, scoped.
    let run = bus
        .zenctl(&["service", "list", "--system", "host-b", "--format", "json"])
        .await;
    exits(&run, 0);
    let doc = run.json();
    let addresses: Vec<&str> = doc["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .filter_map(|r| r["address"].as_str())
        .collect();
    assert_eq!(addresses, ["host-b/tc"], "{run}");

    let run = bus.zenctl(&["iface", "list", "--format", "json"]).await;
    exits(&run, 0);
    let rows = run.json()["rows"].as_array().cloned().expect("rows");
    let by: std::collections::BTreeMap<String, Value> = rows
        .into_iter()
        .map(|r| (r["iface"].as_str().expect("iface").to_owned(), r))
        .collect();
    assert_eq!(
        by.keys().map(String::as_str).collect::<Vec<_>>(),
        ["tc.netem.v1", "tc.netif.v1", "tc.scenario.v1"]
    );
    assert_eq!(by["tc.netem.v1"]["tokenless"], true);
    assert_eq!(
        by["tc.netif.v1"]["providers"],
        serde_json::json!(["host-a/tc", "host-b/tc"])
    );
    assert_eq!(
        by["tc.scenario.v1"]["consumers"],
        serde_json::json!(["ws-01/tcgui-frontend"])
    );
    assert!(by["tc.scenario.v1"].get("providers").is_none());
}

/// `service show`: the descriptor as served; an address presence does not
/// show is silence, and silence is exit 2 — never an empty 0.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn service_show_is_the_descriptor_and_exits_2_on_silence() {
    let bus = Bus::up(None).await;
    exits(&listing(&bus, &[]).await, 0);

    let run = bus
        .zenctl(&["service", "show", "host-b/tc", "--format", "json"])
        .await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["report"], "service-show");
    assert_eq!(doc["address"], "host-b/tc");
    assert_eq!(doc["selector"], "zk2/host-b/tc/@zk/**");
    let rows = doc["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 1, "{run}");
    let d = &rows[0]["descriptor"];
    assert_eq!(d["answer"], "served");
    assert_eq!(d["descriptor"]["service"], "host-b/tc");
    let entries: Vec<(&str, bool)> = d["descriptor"]["interfaces"]
        .as_array()
        .expect("interfaces")
        .iter()
        .map(|e| {
            (
                e["iface"].as_str().expect("iface"),
                e["token"].as_bool().expect("token"),
            )
        })
        .collect();
    assert_eq!(
        entries.into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([("tc.netem.v1", false), ("tc.netif.v1", true)])
    );

    // The frontend's roles, as its descriptor declares them (R3).
    let run = bus
        .zenctl(&[
            "service",
            "show",
            "ws-01/tcgui-frontend",
            "--format",
            "json",
        ])
        .await;
    exits(&run, 0);
    let requires = &run.json()["rows"][0]["descriptor"]["descriptor"]["requires"];
    let roles: BTreeSet<(&str, &str)> = requires
        .as_array()
        .expect("requires")
        .iter()
        .map(|r| {
            (
                r["role"].as_str().expect("role"),
                r["interface"].as_str().expect("interface"),
            )
        })
        .collect();
    assert_eq!(
        roles,
        BTreeSet::from([
            ("netem", "tc.netem.v1"),
            ("netif", "tc.netif.v1"),
            ("scenario", "tc.scenario.v1"),
        ])
    );

    let run = bus
        .zenctl(&[
            "service",
            "show",
            "host-z/tc",
            "--timeout",
            "1",
            "--format",
            "json",
        ])
        .await;
    exits(&run, 2);
    assert_eq!(run.json()["instances"], Value::Null, "rows, not a field");
    assert!(run.json().get("rows").is_none(), "no instance rows: {run}");
    assert!(run.stderr.contains("silence is not a verdict"), "{run}");
}

/// `iface show`: providers by token and descriptor, the role bound to the
/// interface, and the revision's contract retrieved from its holders —
/// or, with `--contracts`, held offline and never retrieved.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn iface_show_names_providers_consumers_and_the_retrieved_contract() {
    let bus = Bus::up(None).await;
    exits(&listing(&bus, &[]).await, 0);
    let netif = fingerprint("tcgui/tc.netif.v1");

    let run = bus
        .zenctl(&["iface", "show", "tc.netif.v1", "--format", "ndjson"])
        .await;
    exits(&run, 0);
    let lines: Vec<Value> = run
        .stdout
        .lines()
        .map(|l| serde_json::from_str(l).expect("a JSON line"))
        .collect();
    assert_eq!(lines[0]["report"], "iface-show");
    assert_eq!(lines[0]["complete"], true);
    let rows = |kind: &str| -> Vec<&Value> { lines.iter().filter(|l| l["row"] == kind).collect() };
    let providers: Vec<&str> = rows("provider")
        .iter()
        .filter_map(|p| p["address"].as_str())
        .collect();
    assert_eq!(providers, ["host-a/tc", "host-b/tc"], "{run}");
    let names: BTreeSet<String> = contract("tcgui/tc.netif.v1")
        .resources
        .iter()
        .map(zk2::implementation::resource_name)
        .collect();
    for p in rows("provider") {
        let exposes: BTreeSet<String> = p["exposes"]
            .as_array()
            .unwrap_or_else(|| panic!("exposure is computed when the revision is held: {p}"))
            .iter()
            .map(|e| e.as_str().expect("a name").to_owned())
            .collect();
        assert_eq!(exposes, names);
    }
    let consumers = rows("consumer");
    assert_eq!(consumers.len(), 1, "{run}");
    assert_eq!(consumers[0]["address"], "ws-01/tcgui-frontend");
    assert_eq!(consumers[0]["role"], "netif");
    assert_eq!(consumers[0]["bindings"], serde_json::json!(["*/tc"]));
    let revisions = rows("revision");
    assert_eq!(revisions.len(), 1, "both hosts at one revision: {run}");
    assert_eq!(revisions[0]["fingerprint"], netif.as_str());
    let held = &revisions[0]["contract"];
    assert_eq!(held["answer"], "held", "{run}");
    assert_eq!(held["source"], "bus", "retrieved from its holders (§8.4)");
    assert_eq!(held["contract"]["iface"], "tc.netif.v1");

    // A fingerprint prefix picks the revision; `--contracts` holds it, so
    // it is never retrieved.
    let at = format!("tc.netif.v1@{}", &fp16(&netif)[..12]);
    let history = examples().join(".history");
    let run = bus
        .zenctl(&[
            "iface",
            "show",
            &at,
            "--contracts",
            history.to_str().expect("a path"),
            "--format",
            "json",
        ])
        .await;
    exits(&run, 0);
    let doc = run.json();
    let revision = doc["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|r| r["row"] == "revision")
        .expect("the revision");
    assert_eq!(revision["contract"]["source"], "history", "{run}");

    // Nobody provides, requires or describes it: silence, exit 2.
    let run = bus
        .zenctl(&[
            "iface",
            "show",
            "nope.v1",
            "--timeout",
            "1",
            "--format",
            "json",
        ])
        .await;
    exits(&run, 2);
}

/// `graph`: the bindings the runtime computes — each role to every provider
/// it selects, the tokenless provider included (its descriptor names it) —
/// and the role that selects nothing stays on its node with no edge.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn graph_draws_the_bindings_and_leaves_the_unbound_role_unbound() {
    let bus = Bus::up(None).await;
    exits(&listing(&bus, &[]).await, 0);

    let run = bus.zenctl(&["graph", "--format", "json"]).await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["report"], "graph");
    assert_eq!(doc["complete"], true);
    let rows = doc["rows"].as_array().expect("rows");
    let edges: BTreeSet<(&str, &str, &str, &str)> = rows
        .iter()
        .filter(|r| r["row"] == "edge")
        .map(|e| {
            (
                e["consumer"].as_str().expect("consumer"),
                e["role"].as_str().expect("role"),
                e["interface"].as_str().expect("interface"),
                e["provider"].as_str().expect("provider"),
            )
        })
        .collect();
    let gui = "ws-01/tcgui-frontend";
    assert_eq!(
        edges,
        BTreeSet::from([
            (gui, "netem", "tc.netem.v1", "host-a/tc"),
            (gui, "netem", "tc.netem.v1", "host-b/tc"),
            (gui, "netif", "tc.netif.v1", "host-a/tc"),
            (gui, "netif", "tc.netif.v1", "host-b/tc"),
        ]),
        "{run}"
    );
    let node = rows
        .iter()
        .find(|r| r["row"] == "node" && r["address"] == gui)
        .expect("the frontend is a node");
    assert_eq!(
        node["requires"]["scenario"]["interface"], "tc.scenario.v1",
        "the unbound role stays on its node"
    );

    let run = bus.zenctl(&["graph", "--dot"]).await;
    exits(&run, 0);
    assert!(run.stdout.starts_with("digraph zk2 {"), "{run}");
    assert!(
        run.stdout
            .contains(r#""host-b/tc" -> "ws-01/tcgui-frontend" [label="netem (tc.netem.v1)"];"#),
        "{run}"
    );
    assert!(
        run.stdout
            .contains("scenario (tc.scenario.v1) ← */tc: no provider"),
        "{run}"
    );
}

/// A namespaced deployment: `namespace list` finds it from a session in no
/// namespace; the resolved verbs read it with `--namespace` (or its alias
/// `--base`) and see nothing at the bus root; a contract is retrieved, and
/// compared, through the same namespaced session.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_namespaced_deployment_is_found_and_read_in_its_namespace() {
    let ns = format!("fj4/live-{}", std::process::id());
    let bus = Bus::up(Some(&ns)).await;

    let run = bus
        .until(&["namespace", "list", "--format", "json"], |r| {
            r.code == 0
                && serde_json::from_str::<Value>(&r.stdout).is_ok_and(|d| {
                    d["rows"].as_array().is_some_and(|rows| {
                        rows.iter()
                            .any(|n| n["namespace"] == ns.as_str() && n["instances"] == 3)
                    })
                })
        })
        .await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["selector"], "**/zk2/*/*/@zk/instance/*");
    let row = doc["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|n| n["namespace"] == ns.as_str())
        .expect("the namespace");
    assert_eq!(row["services"], serde_json::json!(ADDRESSES));

    for flag in ["--namespace", "--base"] {
        let run = listing(&bus, &[flag, &ns]).await;
        exits(&run, 0);
        assert!(settled(&run), "{flag}: {run}");
    }
    // The bus root holds nothing in this case's router: an empty answer is
    // still an answer (0), and the note says it is not a verdict.
    let run = bus.zenctl(&["service", "list", "--format", "json"]).await;
    exits(&run, 0);
    assert!(run.json().get("rows").is_none(), "{run}");

    let run = bus
        .zenctl(&["graph", "--namespace", &ns, "--format", "json"])
        .await;
    exits(&run, 0);
    assert_eq!(
        run.json()["rows"]
            .as_array()
            .expect("rows")
            .iter()
            .filter(|r| r["row"] == "edge")
            .count(),
        4,
        "{run}"
    );

    // A contract retrieved from its holders through the namespaced session,
    // its schemas shown, and the live revision compared with its authoring
    // file: the same revision, compatible.
    let run = bus
        .zenctl(&[
            "schema",
            "show",
            "tc.netif.v1",
            "--namespace",
            &ns,
            "--format",
            "json",
        ])
        .await;
    exits(&run, 0);
    assert_eq!(run.json()["source"], "bus", "{run}");
    assert_eq!(
        run.json()["fingerprint"],
        fingerprint("tcgui/tc.netif.v1").as_str()
    );

    let file = examples().join("tcgui/tc.netif.v1.toml");
    let run = bus
        .zenctl(&[
            "compat",
            "tc.netif.v1",
            file.to_str().expect("a path"),
            "--namespace",
            &ns,
            "--format",
            "json",
        ])
        .await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["class"], "compatible");
    assert_eq!(doc["old"]["source"], "bus");
    assert_eq!(doc["new"]["source"], "file");
    assert_eq!(doc["old"]["fingerprint"], doc["new"]["fingerprint"]);

    // At the bus root there is no such revision to compare: no verdict.
    let run = bus
        .zenctl(&[
            "compat",
            "tc.netif.v1",
            file.to_str().expect("a path"),
            "--timeout",
            "1",
        ])
        .await;
    exits(&run, 2);
    assert!(run.stderr.contains("no revision of tc.netif.v1"), "{run}");
}
