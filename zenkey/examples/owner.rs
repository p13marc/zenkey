//! A zk2 owner to interoperate with (#609's live half, #610): it brings up
//! one service implementing the given contracts and runs until its standard
//! input closes.
//!
//! ```text
//! cargo run -p zenkey --example owner -- [--connect <endpoint>] [--hostid-root <dir>] [--hostid-ephemeral]
//!     [--health] [--health-status <LEVEL>[:<reason>]] [--health-check <name>=<LEVEL>[:<detail>]]...
//!     [--tokenless <iface>]... [--clock-reference <key>] [--clock-offset-ms <ms>]
//!     [--cardinality <iface>:<resource>=<n>]...
//!     [--config] [--config-floor <s>] [--config-apply-after-ms <ms>] [--config-no-persist]
//!     <address> <contract.toml>...
//! ```
//!
//! The address is `<system>/<service>`, or `@hostid.v1/<service>` for a
//! system minted from the machine id (`spec/profiles/hostid/v1.md` §2.3,
//! #719). The system is minted before the session opens (§2.7), and a host
//! with no id stops the owner with nothing declared (§2.6): the error names
//! every path, and the exit is 1. `--hostid-root <dir>` reads the inputs
//! under `<dir>` instead of `/`, its links resolved in it, as a scenario's
//! root (`spec/profiles/hostid/scenarios.md`); `--hostid-ephemeral` is
//! `hostid = { ephemeral = true }`.
//!
//! By default the session is a router listening on an ephemeral loopback
//! port, so a test connects to it as a client. Two lines on standard output
//! say what to connect to and what came up:
//!
//! ```text
//! listening tcp/127.0.0.1:<port>
//! ready zk2/<system>/<service>/@zk/instance/<instance>
//! ```
//!
//! With `--connect <endpoint>`, the session is instead a client of the
//! router at `<endpoint>`, with its HLC on (§4.3), and the first line is
//! `connected <endpoint>`. That is the owner the black-box scenarios watch
//! through a separate router (#660): `state.md §1` tells the owner's stamp
//! from the router's only when the two sessions differ (§4.2, "Observing
//! S1"), and `presence.md §2` sees a refusal only through a router that
//! outlives the owner.
//!
//! Every resource of every contract is exposed (§8.1–§8.4), and for a
//! counterpart to read:
//! - a state resource with no template parameters and a `raw` type holds
//!   the value `ok`, stamped (S1), answered on GET (S2), and put before the
//!   tokens, so a GET made the moment they appear finds it (§8.2, "State
//!   values", F-68);
//! - every operation is served, so everything exposed is callable (§8.2,
//!   "Exposed"; #670): a `raw` request and response echo the request; any
//!   other types refuse with `app` and no detail (O3, §5.2), since this
//!   owner decodes no schema, and an operation with no `error` type has no
//!   detail to send (F-65). An operation with template parameters is served
//!   over the whole template (§5.1): a call whose key binds every parameter
//!   answers for that member, and an echo to a fan-out that leaves one
//!   unbound names no member, so it is refused `internal`, as §5.1 says of
//!   a server that cannot name one.
//!
//! Capabilities named by `capability:` gates are all held, so gated
//! resources are exposed too.
//!
//! **`health.v1`** (`spec/profiles/health/v1.md`, #721) is implemented by
//! the runtime ([`zenkey::health`]) with `--health`, or when a contract
//! file given is the standard one (its fingerprint is `e9dbcdcb…`, as
//! `spec/profiles/health/health.v1.toml` is): the status is put before the
//! tokens, `UNSPECIFIED` with reason `starting` unless
//! `--health-status <LEVEL>[:<reason>]` sets one (§2.3); each
//! `--health-check <name>=<LEVEL>[:<detail>]` is a check put before the
//! tokens too, the status raised to the worst of them (§2.2). A level is
//! `OK`, `DEGRADED` or `FAILED`, in any case; a check may be `UNSPECIFIED`,
//! one it cannot run (§2.3). The status is re-put every 28.5 s.
//!
//! **Commands**, one per line on standard input, each answered by one line
//! on standard output, while the owner runs:
//!
//! ```text
//! status <LEVEL> [<reason>...]           set_status
//! check <name> <LEVEL> [<detail>...]     set_check (UNSPECIFIED: set_check_unknown)
//! retire <name>                          retire_check
//! fault <code> <LEVEL> [<detail>...]     fault: an application's code (§2.10)
//! confirm on|off                         set_confirming: off lets the status go stale (§2.4)
//! clock-offset <ms>                      the clock's simulated offset (0: set right)
//! put <iface> <resource> [<value>...]    one write on a member (core §2.7)
//! delete <iface> <resource> [<value>...] a state member's delete
//! fill <iface> <resource> <n>            put members m0 … m<n−1>, one write each
//! ```
//!
//! **The budget** (core §2.7, #741), for a runner to reach each bound:
//! `put` writes once on the member of a stream, state or event resource
//! whose template parameters the values bind, in order (a rest parameter
//! takes its value's `/`-separated chunks): a stamped state put, a stream
//! sample, or one event occurrence, each of the bytes `ok`, which is a
//! value only of a raw type. It is answered `put <key>`, an occurrence's
//! key with its ULID. `delete` deletes a state member, answered `deleted
//! <key>`. `fill` puts members `m0` to `m<n−1>` (every parameter bound to
//! `m<i>`), stopping at the first refusal, answered `filled <n>`. A write
//! the runtime refuses — a new member while the live members are at the
//! bound, an occurrence beyond the member's rate — is answered `refused
//! <command>: <why>`, `why` naming the resource, the bound and whose it is
//! (or the rate), and for `fill` how many it put first. `--cardinality
//! <iface>:<resource>=<n>` lowers a template's bound in the descriptor
//! (core §3.3), e.g. `--cardinality budget.v1:state/items/{item}=2`.
//!
//! A `health.v1` command is answered `health status=<LEVEL> declared=<LEVEL>
//! raised_by=<name>` (`UNSPECIFIED` for no level, `-` for none): the status
//! as published, the level declared, and the check that raised it (§2.2).
//! `clock-offset` is answered `clock-offset <ms>`. Anything refused, by the
//! runtime or here, is answered `refused <command>: <why>`, and changes
//! nothing the runtime did not report.
//!
//! **A clock ahead** (§2.5): `--clock-reference <key>` is the router-stamped
//! heartbeat the clock guard watches (core §4.3), and `--clock-offset-ms`
//! shifts this owner's clock (the runtime's `Minter::simulate_offset`).
//! With `--connect`, a nonzero offset leaves the session's HLC off: a
//! session whose HLC runs re-stamps a stamp beyond its own delta before it
//! leaves (measured in `tests/profile_health.rs`), so the simulated stamps
//! would never reach the router. Off, they reach it as a session whose HLC
//! is itself ahead would send them, and the router re-stamps the
//! `clock_ahead` fault (core §4.1).
//!
//! `--tokenless <iface>` puts an interface in the deployment's tokenless
//! set (core §8.1, U22): `health.v1`, for one.
//!
//! **`config.v1`** (`spec/profiles/config/v1.md`, #768) is implemented by
//! the runtime ([`zenkey::config_v1`]) with `--config`, or when a contract
//! file given is the standard one (its fingerprint is `727ff009…`, as
//! `spec/profiles/config/config.v1.toml` is). The schema is
//! `conformance/lifecycle.json`'s `rf`, as `scenarios.md` runs it, and a
//! contract group after it:
//!
//! | Group | Class | Parameter | Startup |
//! |---|---|---|---|
//! | `duty` | hot | `fraction_ppm`, 0 to 1000000 ppm | 10000, `file` |
//! | `radio` | reach | `frequency_hz`, 150000000 to 960000000 Hz | 868100000, `file` |
//! | `link` | reach | `net_id`, 0 to 255 | 1, `default` |
//! | `sim` | hot | `pin`, text, sensitive | `"0000"`, `file` |
//! | `queue` | hot, outside the allowlist | `tx_queue_len`, at least 0 packets | 100, `default` |
//! | `modulation` | contract | `spreading_factor`, 7 to 12 | 9, `file` |
//!
//! `--config-floor <s>` is the floor (10 by default, §2.8);
//! `--config-apply-after-ms <ms>` is how long after a reach `set` the
//! device applies it, the `apply_at` it states (1000 by default, below the
//! floor, §2.9); `--config-no-persist` leaves the owner without a persisted
//! layer, so `persist` is listed unavailable, cause `config` (§2.13). The
//! device is simulated: it holds what it is told, in memory, and its
//! persisted layer survives `config restart`. The runtime logs each request
//! on standard error, a sensitive value written `"<redacted>"` (§2.15):
//! `zenkey::config_v1` logs at `info` by default here.
//!
//! Its commands, each answered by one line:
//!
//! ```text
//! config fail-apply [<words>...]     the device refuses its next apply, in these words
//! config fail-undo [unknown]         its next undo fails, reading back what it holds (unknown: nothing)
//! config fail-persist [<words>...]   its next persisted-layer write fails: persist answers internal
//! config block-persist [<words>...]  its persisted layer cannot be written until cleared: unavailable, cause config
//! config clear                       every failure above cleared
//! config device                      what the device holds and did
//! config desired <json>              a DesiredConfig to converge (§2.18); the report is the view's and the status's
//! config offset <ms>                 the owner's clock shifted from this host's (a test's: §2.8, §2.14)
//! config restart                     a restart (§2.12): the service closes and comes up as a new instance
//! ```
//!
//! A failure command is answered `config device` followed by the device's
//! state, as `config device` is: `held=<json> applies=<n> undos=<n>
//! persisted=<json|none> next=<the failures set, or ->`, `held` being what
//! the device runs with, a sensitive value written `"<redacted>"` (§2.15),
//! `applies` the applies it made (a refused one is not), and `undos` every
//! undo it was asked for, a failed one included. `desired` is
//! answered `config desired <revision>`, `offset` `config offset <ms>`, and
//! `restart` `config restarted zk2/<system>/<service>/@zk/instance/<id>`, the
//! new instance's key, once the new instance's tokens are declared. A
//! restart keeps the persisted layer and the change events of the last
//! hour (§2.17), and the device runs its startup values again; anything else
//! of the old instance is gone, as a process restart leaves it.

use std::collections::HashMap;
use std::future::Future;
use std::io::BufRead;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use zenkey::config_v1::{self, Configuration, DesiredConfig, Startup};
use zenkey::health::{self, Effective, Health, Level};
use zenkey::hostid::{HostIdMinter, HostIdSource};
use zenkey::model::authoring::Kind;
use zenkey::model::contract::{Body, Contract, Resource, load_path};
use zenkey::model::grammar::IfaceId;
use zenkey::model::schema::TypeId;
use zenkey::model::template::{Bindings, Segment};
use zenkey::operation::OperationServer;
use zenkey::state::StateWriter;
use zenkey::writer::{EventWriter, Writer};
use zenkey::{Address, Implementation, OpError, Service, ServiceBuilder, ServiceConfig};

#[path = "owner/config.rs"]
pub mod config;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// The command line.
pub struct Options {
    /// `--connect <endpoint>`: a client of that router, not a router.
    pub connect: Option<String>,
    /// `--hostid-root <dir>`: the directory standing in for `/` when the
    /// address is minted (hostid.v1); `/` otherwise.
    pub hostid_root: Option<PathBuf>,
    /// `--hostid-ephemeral`: `hostid.ephemeral` (hostid.v1 §2.6).
    pub hostid_ephemeral: bool,
    /// `--health`, or implied by `--health-status`, `--health-check` or
    /// the standard contract among the files: `health.v1` by the runtime.
    pub health: bool,
    /// `--health-status <LEVEL>[:<reason>]`: the status put before start.
    pub health_status: Option<(Level, String)>,
    /// `--health-check <name>=<LEVEL>[:<detail>]`: checks put before start
    /// (`None`: `UNSPECIFIED`).
    pub health_checks: Vec<(String, Option<Level>, String)>,
    /// `--tokenless <iface>`: the deployment's tokenless set (core §8.1).
    pub tokenless: Vec<String>,
    /// `--clock-reference <key>`: the clock guard's reference (core §4.3).
    pub clock_reference: Option<String>,
    /// `--clock-offset-ms <ms>`: this owner's clock, shifted.
    pub clock_offset_ms: i64,
    /// `--cardinality <iface>:<resource>=<n>`: a template's bound lowered in
    /// the descriptor (core §3.3, §2.7).
    pub cardinality: Vec<(String, String, u64)>,
    /// `--config`, or implied by a `--config-*` option or the standard
    /// contract among the files: `config.v1` by the runtime.
    pub config: bool,
    /// `--config-floor <s>`: the floor (config.v1 §2.8).
    pub config_floor_s: u64,
    /// `--config-apply-after-ms <ms>`: the reach apply's delay (§2.9).
    pub config_apply_after_ms: u64,
    /// Without `--config-no-persist`: a persisted layer (§2.13).
    pub config_persist: bool,
    pub address: String,
    pub contracts: Vec<PathBuf>,
}

/// A level as the command line spells it: `Some(None)` for `UNSPECIFIED`.
fn parse_level(s: &str) -> Option<Option<Level>> {
    match s.to_ascii_uppercase().as_str() {
        "OK" => Some(Some(Level::Ok)),
        "DEGRADED" => Some(Some(Level::Degraded)),
        "FAILED" => Some(Some(Level::Failed)),
        "UNSPECIFIED" => Some(None),
        _ => None,
    }
}

fn level_name(l: Option<Level>) -> String {
    l.map_or_else(|| "UNSPECIFIED".to_owned(), |l| l.to_string())
}

impl Options {
    /// Reads the options of the module doc's usage, in any order before
    /// the address.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let usage = || {
            "usage: owner [--connect <endpoint>] [--hostid-root <dir>] [--hostid-ephemeral] \
             [--health] [--health-status <LEVEL>[:<reason>]] \
             [--health-check <name>=<LEVEL>[:<detail>]]... [--tokenless <iface>]... \
             [--clock-reference <key>] [--clock-offset-ms <ms>] \
             [--cardinality <iface>:<resource>=<n>]... \
             [--config] [--config-floor <s>] [--config-apply-after-ms <ms>] [--config-no-persist] \
             <system>/<service>|@hostid.v1/<service> <contract.toml>..."
                .to_owned()
        };
        let mut o = Self {
            connect: None,
            hostid_root: None,
            hostid_ephemeral: false,
            health: false,
            health_status: None,
            health_checks: Vec::new(),
            tokenless: Vec::new(),
            clock_reference: None,
            clock_offset_ms: 0,
            cardinality: Vec::new(),
            config: false,
            config_floor_s: 10,
            config_apply_after_ms: 1000,
            config_persist: true,
            address: String::new(),
            contracts: Vec::new(),
        };
        let mut rest = args;
        loop {
            match rest {
                [flag, value, tail @ ..] if flag == "--connect" => {
                    o.connect = Some(value.clone());
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--hostid-root" => {
                    o.hostid_root = Some(PathBuf::from(value));
                    rest = tail;
                }
                [flag, tail @ ..] if flag == "--hostid-ephemeral" => {
                    o.hostid_ephemeral = true;
                    rest = tail;
                }
                [flag, tail @ ..] if flag == "--health" => {
                    o.health = true;
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--health-status" => {
                    let (level, reason) = value.split_once(':').unwrap_or((value, ""));
                    let Some(Some(level)) = parse_level(level) else {
                        return Err(format!(
                            "--health-status {value:?}: OK, DEGRADED or FAILED; a status is \
                             UNSPECIFIED only by default, at start (health.v1 §2.6)"
                        ));
                    };
                    o.health = true;
                    o.health_status = Some((level, reason.to_owned()));
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--health-check" => {
                    let parsed = value.split_once('=').and_then(|(name, l)| {
                        let (level, detail) = l.split_once(':').unwrap_or((l, ""));
                        Some((name.to_owned(), parse_level(level)?, detail.to_owned()))
                    });
                    let Some(check) = parsed else {
                        return Err(format!(
                            "--health-check {value:?}: <name>=<LEVEL>[:<detail>], LEVEL one of \
                             OK, DEGRADED, FAILED, UNSPECIFIED"
                        ));
                    };
                    o.health = true;
                    o.health_checks.push(check);
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--tokenless" => {
                    o.tokenless.push(value.clone());
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--clock-reference" => {
                    o.clock_reference = Some(value.clone());
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--clock-offset-ms" => {
                    o.clock_offset_ms = value
                        .parse()
                        .map_err(|e| format!("--clock-offset-ms {value:?}: {e}"))?;
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--cardinality" => {
                    let parsed = value.split_once(':').and_then(|(iface, r)| {
                        let (resource, n) = r.rsplit_once('=')?;
                        Some((iface.to_owned(), resource.to_owned(), n.parse().ok()?))
                    });
                    let Some(c) = parsed else {
                        return Err(format!(
                            "--cardinality {value:?}: <iface>:<resource>=<n>, as \
                             budget.v1:state/items/{{item}}=2"
                        ));
                    };
                    o.cardinality.push(c);
                    rest = tail;
                }
                [flag, tail @ ..] if flag == "--config" => {
                    o.config = true;
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--config-floor" => {
                    o.config_floor_s = value
                        .parse()
                        .map_err(|e| format!("--config-floor {value:?}: {e}"))?;
                    o.config = true;
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--config-apply-after-ms" => {
                    o.config_apply_after_ms = value
                        .parse()
                        .map_err(|e| format!("--config-apply-after-ms {value:?}: {e}"))?;
                    o.config = true;
                    rest = tail;
                }
                [flag, tail @ ..] if flag == "--config-no-persist" => {
                    o.config_persist = false;
                    o.config = true;
                    rest = tail;
                }
                [flag, ..] if flag.starts_with("--") => return Err(usage()),
                _ => break,
            }
        }
        let [address, files @ ..] = rest else {
            return Err(usage());
        };
        o.address = address.clone();
        o.contracts = files.iter().map(PathBuf::from).collect();
        Ok(o)
    }
}

/// Brings the owner up, says each output line through `say`, and runs until
/// `until` resolves.
pub async fn run(
    opts: Options,
    say: impl Fn(String),
    until: impl Future<Output = ()>,
) -> Result<(), BoxError> {
    let (_, none) = flume::unbounded();
    run_with(opts, say, none, until).await
}

/// [`run`], answering each line of `commands` (the module doc's) through
/// `say` while it runs. When `until` resolves, the lines already sent are
/// answered first.
pub async fn run_with(
    mut opts: Options,
    say: impl Fn(String),
    commands: flume::Receiver<String>,
    until: impl Future<Output = ()>,
) -> Result<(), BoxError> {
    let mut address: Address = opts.address.parse()?;
    if let Address::HostId { ephemeral, .. } = &mut address {
        *ephemeral = opts.hostid_ephemeral;
    }
    let mut config = ServiceConfig::at(address);
    for t in &opts.tokenless {
        config = config.tokenless(t.parse()?);
    }
    config.clock_reference = opts.clock_reference.clone();
    let mut imps = Vec::new();
    for f in &opts.contracts {
        let l = load_path(f);
        let Some(c) = l.contract else {
            return Err(format!("{}:\n{}", f.display(), l.report).into());
        };
        for r in &c.resources {
            for cap in r.gate.iter().filter_map(|g| g.strip_prefix("capability:")) {
                config = config.capability(cap);
            }
        }
        let imp = Implementation::new(c);
        // The standard health.v1 is the runtime's to implement (#721).
        if imp.iface() == &health::iface()
            && imp.fingerprint().hex().to_string() == health::FINGERPRINT
        {
            opts.health = true;
            continue;
        }
        // So is the standard config.v1 (#768).
        if imp.iface() == &config_v1::iface()
            && imp.fingerprint().hex().to_string() == config_v1::FINGERPRINT
        {
            opts.config = true;
            continue;
        }
        imps.push(imp);
    }

    // hostid.v1 §2.7: the system is minted before the session opens, and a
    // host without an id stops here, with nothing declared (§2.6).
    let under_root;
    let minter = match &opts.hostid_root {
        Some(root) => {
            under_root = HostIdMinter::new(HostIdSource::at(root));
            &under_root
        }
        None => HostIdMinter::global(),
    };
    config.resolve_with(minter)?;

    let mut z = zenoh::Config::default();
    z.insert_json5("scouting/multicast/enabled", "false")?;
    let session = match &opts.connect {
        None => {
            z.insert_json5("mode", "\"router\"")?;
            z.insert_json5("listen/endpoints", "[\"tcp/127.0.0.1:0\"]")?;
            let session = zenoh::open(z).await?;
            let endpoint = session
                .info()
                .locators()
                .await
                .into_iter()
                .map(|l| l.to_string())
                .find(|l| l.starts_with("tcp/127.0.0.1:"))
                .ok_or("no loopback listener")?;
            say(format!("listening {endpoint}"));
            session
        }
        Some(endpoint) => {
            z.insert_json5("mode", "\"client\"")?;
            z.insert_json5("connect/endpoints", &serde_json::to_string(&[endpoint])?)?;
            // §4.3: a session that serves state enables its HLC. A simulated
            // offset leaves it off (the module doc): the HLC would re-stamp
            // the offset clock's stamps before they leave.
            if opts.clock_offset_ms == 0 {
                z.insert_json5("timestamping/enabled", "true")?;
            }
            let session = zenoh::open(z).await?;
            say(format!("connected {endpoint}"));
            session
        }
    };

    // config.v1: the rf owner's startup, and its simulated device.
    let configured = opts.config.then(|| {
        let startup = config::rf_startup(
            true,
            opts.config_floor_s,
            Duration::from_millis(opts.config_apply_after_ms),
            opts.config_persist,
        );
        let device = Arc::new(config::SimDevice::new(&startup));
        (startup, device)
    });
    let ctx = Ctx {
        opts: &opts,
        session: &session,
        config,
        minter,
        imps,
        configured,
    };
    let mut up = ctx.up(Vec::new()).await?;
    say(format!("ready {}", up.svc.instance_key()?));

    let mut until = std::pin::pin!(until);
    let mut open = true;
    loop {
        tokio::select! {
            () = &mut until => break,
            line = commands.recv_async(), if open => match line {
                Ok(line) => {
                    let answer;
                    (answer, up) = ctx.command(&line, up).await?;
                    say(answer);
                }
                Err(_) => open = false,
            },
        }
    }
    while let Ok(line) = commands.try_recv() {
        let answer;
        (answer, up) = ctx.command(&line, up).await?;
        say(answer);
    }
    up.close().await?;
    session.close().await?;
    Ok(())
}

/// What every bring-up of the service is made from: the first, and each
/// `config restart`'s.
struct Ctx<'a> {
    opts: &'a Options,
    session: &'a zenoh::Session,
    config: ServiceConfig,
    minter: &'a HostIdMinter,
    imps: Vec<Implementation>,
    /// config.v1's startup and device, with `--config`.
    configured: Option<(Startup, Arc<config::SimDevice>)>,
}

/// One running instance of the service, and what it holds.
struct Up {
    svc: Service,
    servers: Vec<OperationServer>,
    _states: Vec<StateWriter>,
    health: Option<Health>,
    members: Members,
    config: Option<Configuration>,
}

impl Up {
    async fn close(self) -> Result<(), BoxError> {
        drop(self.members);
        self.svc.close().await?;
        drop(self.servers);
        Ok(())
    }
}

impl Ctx<'_> {
    /// Answers one command line (the module doc's): `config restart` here,
    /// since it replaces the instance; every other one by [`command`].
    async fn command(&self, line: &str, mut up: Up) -> Result<(String, Up), BoxError> {
        if line.split_whitespace().collect::<Vec<_>>() != ["config", "restart"] {
            let answer = command(line, &mut up, self.configured.as_ref()).await;
            return Ok((answer, up));
        }
        let (Some(cfg), Some((startup, device))) = (up.config.clone(), &self.configured) else {
            let refused = format!("refused {line}: config.v1 is not implemented here (--config)");
            return Ok((refused, up));
        };
        // §2.12: the old instance goes first, the device runs its startup
        // values again, and the new instance starts from them, with the
        // persisted layer and the change events of the last hour (§2.17).
        let events = cfg.published().await;
        up.close().await?;
        device.restart(startup);
        let up = self.up(events).await?;
        let answer = format!("config restarted {}", up.svc.instance_key()?);
        Ok((answer, up))
    }

    /// Brings the service up: every contract's resources, `health.v1` and
    /// `config.v1` as asked, then `start()`. `events` are config.v1's
    /// change events of the last hour (§2.17).
    async fn up(&self, events: Vec<SystemTime>) -> Result<Up, BoxError> {
        let opts = self.opts;
        let mut b = ServiceBuilder::with_hostid(self.session, self.config.clone(), self.minter);
        b.minter().simulate_offset(opts.clock_offset_ms);
        let mut states = Vec::new();
        let mut servers = Vec::new();
        let mut members = Members::default();
        let none = Bindings::new();
        for imp in self.imps.iter().cloned() {
            let iface = imp.iface().clone();
            let resources = imp.contract().resources.clone();
            members
                .contracts
                .insert(iface.clone(), imp.contract().clone());
            b.implement(imp)?;
            for r in resources {
                let name = zenkey::implementation::resource_name(&r);
                let raw = |t: &TypeId| matches!(t, TypeId::Raw { .. });
                match &r.body {
                    Body::Data(d)
                        if r.kind == Kind::State && !r.template.has_params() && raw(&d.type_) =>
                    {
                        states.push(b.declare_state_writer(&iface, &name, &none).await?);
                    }
                    Body::Operation(o) => {
                        let echo = raw(&o.request) && raw(&o.response);
                        // Over the whole template when it has parameters (§5.1),
                        // else on the operation's one key.
                        let values = (!r.template.has_params()).then_some(&none);
                        servers.push(
                            b.serve(&iface, &name, values, move |call| async move {
                                if !echo {
                                    // §5.2: `app` from any operation, and no detail
                                    // where there is no `error` type to carry one.
                                    return Err(OpError::app_without_detail(
                                        "this interop owner decodes no schema",
                                    ));
                                }
                                let body = call
                                    .payload()
                                    .map(|p| p.to_bytes().into_owned())
                                    .unwrap_or_default();
                                call.reply(body)
                                    .await
                                    .map_err(|e| OpError::internal(e.to_string()))
                            })
                            .await?,
                        );
                    }
                    _ => {
                        b.expose(&iface, &name)?;
                    }
                }
            }
        }
        // Core §3.3: a template's bound lowered in the descriptor, which the
        // runtime then keeps (§2.7).
        for (iface, resource, n) in &opts.cardinality {
            b.cardinality(&iface.parse()?, resource, *n)?;
        }
        // health.v1 §2.3: its status and checks before the tokens; the status
        // is UNSPECIFIED, "starting", when none is given.
        let health = if opts.health {
            let h = b.health().await?;
            if let Some((level, reason)) = &opts.health_status {
                h.set_status(*level, reason.as_str()).await?;
            }
            for (name, level, detail) in &opts.health_checks {
                match level {
                    Some(l) => h.set_check(name, *l, detail.as_str()).await?,
                    None => h.set_check_unknown(name, detail.as_str()).await?,
                };
            }
            Some(h)
        } else {
            None
        };
        // config.v1 §2.3: the view and the status are put here, before the
        // tokens; the owner starts from its startup values and the persisted
        // layer the device holds (§2.12).
        let config = match &self.configured {
            Some((startup, device)) => {
                let next = device.next_startup(startup, events);
                Some(
                    b.config_v1(next, Arc::clone(device) as Arc<dyn config_v1::Device>)
                        .await?,
                )
            }
            None => None,
        };
        // §8.2 "State values" (F-68): the value is put before the tokens, so a
        // GET made the moment they appear finds it.
        for w in &states {
            w.put("ok").await?;
        }
        let svc = b.start().await?;
        Ok(Up {
            svc,
            servers,
            _states: states,
            health,
            members,
            config,
        })
    }
}

/// The status as a command's answer.
fn describe(e: &Effective) -> String {
    format!(
        "health status={} declared={} raised_by={}",
        level_name(e.level),
        e.declared.map_or_else(|| "-".to_owned(), |l| l.to_string()),
        e.raised_by.as_deref().unwrap_or("-")
    )
}

/// A member's writer, kept for its next write.
enum Held {
    State(StateWriter),
    Stream(Box<Writer>),
    Event(EventWriter),
}

/// The members `put`, `delete` and `fill` write (core §2.7): each
/// implemented contract, and a writer per member written.
#[derive(Default)]
struct Members {
    contracts: HashMap<IfaceId, Contract>,
    writers: HashMap<String, Held>,
}

/// The template's parameters bound to `values`, in order: a rest parameter
/// takes its value's `/`-separated chunks.
fn bind(r: &Resource, values: &[String]) -> Result<Bindings, String> {
    let params: Vec<(String, bool)> = r
        .template
        .segments()
        .iter()
        .filter_map(|s| match s {
            Segment::Param(n) => Some((n.clone(), false)),
            Segment::Rest(n) => Some((n.clone(), true)),
            Segment::Literal(_) => None,
        })
        .collect();
    if params.len() != values.len() {
        return Err(format!(
            "{} takes {} value(s), one per parameter, and {} were given",
            r.template,
            params.len(),
            values.len()
        ));
    }
    Ok(params
        .into_iter()
        .zip(values)
        .map(|((name, rest), v)| {
            let chunks = if rest {
                v.split('/').map(str::to_owned).collect()
            } else {
                vec![v.clone()]
            };
            (name, chunks)
        })
        .collect())
}

impl Members {
    /// One write on the member of `resource` the values bind (§2.7), or why
    /// not; for `delete`, the state member's delete. Answers its key.
    async fn write(
        &mut self,
        svc: &mut Service,
        iface: &str,
        resource: &str,
        values: &[String],
        delete: bool,
    ) -> Result<String, String> {
        let iface: IfaceId = iface.parse().map_err(|e| format!("{e}"))?;
        let r = self
            .contracts
            .get(&iface)
            .ok_or_else(|| format!("{iface} is not implemented here"))?
            .resources
            .iter()
            .find(|r| zenkey::implementation::resource_name(r) == resource)
            .ok_or_else(|| format!("{iface} has no resource {resource:?}"))?
            .clone();
        let bound = bind(&r, values)?;
        let key = svc
            .key(&iface, resource, &bound)
            .map_err(|e| e.to_string())?
            .to_string();
        let err = |e: zenkey::Error| e.to_string();
        if !self.writers.contains_key(&key) {
            let held = match r.kind {
                Kind::State => Held::State(
                    svc.state_writer(&iface, resource, &bound)
                        .await
                        .map_err(err)?,
                ),
                Kind::Stream => Held::Stream(Box::new(
                    svc.writer(&iface, resource, &bound).await.map_err(err)?,
                )),
                Kind::Event => {
                    Held::Event(svc.event_writer(&iface, resource, &bound).map_err(err)?)
                }
                Kind::Operation => return Err(format!("{resource:?} is an operation")),
            };
            self.writers.insert(key.clone(), held);
        }
        match (self.writers.get(&key).expect("held"), delete) {
            (Held::State(w), false) => w.put("ok").await.map(|_| key).map_err(err),
            (Held::State(w), true) => w.delete().await.map(|_| key).map_err(err),
            (Held::Stream(w), false) => w.put("ok").await.map(|()| key).map_err(err),
            (Held::Event(w), false) => w.put("ok").await.map_err(err),
            (_, true) => Err(format!(
                "{resource:?} is not state: only a state is deleted"
            )),
        }
    }
}

/// The simulated device's state, as a `config` command answers it: a
/// sensitive value written `"<redacted>"`, as everything this owner says is
/// (config.v1 §2.15).
fn device_line(d: &config::SimDevice, schema: &config_v1::Schema) -> String {
    d.with(|s| {
        let mut held = serde_json::to_value(&s.held).unwrap_or_default();
        for g in &schema.groups {
            for p in g.parameters.iter().filter(|p| p.sensitive) {
                if let Some(v) = held.get_mut(&g.name).and_then(|g| g.get_mut(&p.name)) {
                    *v = serde_json::Value::String(zenkey::model::config::REDACTED.to_owned());
                }
            }
        }
        let mut next = Vec::new();
        if s.refuse_apply.is_some() {
            next.push("fail-apply");
        }
        match s.fail_undo {
            Some(config::UndoFailure::Held) => next.push("fail-undo"),
            Some(config::UndoFailure::Unknown) => next.push("fail-undo-unknown"),
            None => {}
        }
        if s.fail_persist.is_some() {
            next.push("fail-persist");
        }
        if s.block_persist.is_some() {
            next.push("block-persist");
        }
        let json = |v: &config_v1::Values| serde_json::to_string(v).unwrap_or_default();
        format!(
            "config device held={held} applies={} undos={} persisted={} next={}",
            s.applies.len(),
            s.undos.len(),
            s.persisted.as_ref().map_or_else(|| "none".to_owned(), json),
            if next.is_empty() {
                "-".to_owned()
            } else {
                next.join(",")
            }
        )
    })
}

/// Answers a `config` command but `restart` (the module doc's).
async fn config_command(
    words: &[&str],
    cfg: &Configuration,
    device: &config::SimDevice,
) -> Result<String, String> {
    let words_from = |n: usize| words.get(n..).map(|w| w.join(" ")).unwrap_or_default();
    let said = |n: usize, default: &str| {
        let w = words_from(n);
        if w.is_empty() { default.to_owned() } else { w }
    };
    match words {
        ["config", "fail-apply", ..] => {
            let w = said(2, "the simulated device refused the apply");
            device.with(|s| s.refuse_apply = Some(w));
        }
        ["config", "fail-undo"] => device.with(|s| s.fail_undo = Some(config::UndoFailure::Held)),
        ["config", "fail-undo", "unknown"] => {
            device.with(|s| s.fail_undo = Some(config::UndoFailure::Unknown));
        }
        ["config", "fail-persist", ..] => {
            let w = said(2, "the simulated persisted layer failed its write");
            device.with(|s| s.fail_persist = Some(w));
        }
        ["config", "block-persist", ..] => {
            let w = said(2, "the simulated persisted layer is read-only");
            device.with(|s| s.block_persist = Some(w));
        }
        ["config", "clear"] => device.with(|s| {
            s.refuse_apply = None;
            s.fail_undo = None;
            s.fail_persist = None;
            s.block_persist = None;
        }),
        ["config", "device"] => {}
        ["config", "desired", ..] => {
            let doc: DesiredConfig = serde_json::from_str(&words_from(2))
                .map_err(|e| format!("not a DesiredConfig: {e}"))?;
            let revision = doc.revision;
            cfg.set_desired(doc).await;
            return Ok(format!("config desired {revision}"));
        }
        ["config", "offset", ms] => {
            let ms: i64 = ms.parse().map_err(|e| format!("{e}"))?;
            cfg.simulate_offset(ms);
            return Ok(format!("config offset {ms}"));
        }
        _ => {
            return Err(
                "usage: config fail-apply [<words>] | fail-undo [unknown] | fail-persist [<words>] | \
                 block-persist [<words>] | clear | device | desired <json> | offset <ms> | restart"
                    .to_owned(),
            );
        }
    }
    Ok(device_line(device, cfg.schema()))
}

/// Answers one command line (the module doc's) but `config restart`.
async fn command(
    line: &str,
    up: &mut Up,
    configured: Option<&(Startup, Arc<config::SimDevice>)>,
) -> String {
    let words: Vec<&str> = line.split_whitespace().collect();
    let refuse = |why: &str| format!("refused {}: {why}", words.join(" "));
    let rest = |n: usize| words.get(n..).map(|w| w.join(" ")).unwrap_or_default();
    if words.first() == Some(&"config") {
        let (Some(cfg), Some((_, device))) = (&up.config, configured) else {
            return refuse("config.v1 is not implemented here (--config)");
        };
        return match config_command(&words, cfg, device).await {
            Ok(answer) => answer,
            Err(why) => refuse(&why),
        };
    }
    let health = up.health.as_ref();
    let svc = &mut up.svc;
    let members = &mut up.members;
    if let ["clock-offset", ms] = words.as_slice() {
        return match ms.parse::<i64>() {
            Ok(ms) => {
                svc.minter().simulate_offset(ms);
                format!("clock-offset {ms}")
            }
            Err(e) => refuse(&e.to_string()),
        };
    }
    // Core §2.7: one write on a member, its delete, or a run of members.
    let owned = |from: usize| -> Vec<String> {
        words
            .get(from..)
            .unwrap_or_default()
            .iter()
            .map(|w| (*w).to_owned())
            .collect()
    };
    match words.as_slice() {
        ["put", iface, resource, ..] => {
            return match members.write(svc, iface, resource, &owned(3), false).await {
                Ok(key) => format!("put {key}"),
                Err(why) => refuse(&why),
            };
        }
        ["delete", iface, resource, ..] => {
            return match members.write(svc, iface, resource, &owned(3), true).await {
                Ok(key) => format!("deleted {key}"),
                Err(why) => refuse(&why),
            };
        }
        ["fill", iface, resource, n] => {
            let Ok(n) = n.parse::<u64>() else {
                return refuse("fill takes a count");
            };
            let Ok(id) = iface.parse::<IfaceId>() else {
                return refuse("not an interface id");
            };
            let Some(params) = members
                .contracts
                .get(&id)
                .and_then(|c| {
                    c.resources
                        .iter()
                        .find(|r| zenkey::implementation::resource_name(r) == *resource)
                })
                .map(|r| r.template.params().count())
            else {
                return refuse(&format!("{id} has no resource {resource:?} here"));
            };
            for i in 0..n {
                let values = vec![format!("m{i}"); params];
                if let Err(why) = members.write(svc, iface, resource, &values, false).await {
                    return refuse(&format!("{i} put, then: {why}"));
                }
            }
            return format!("filled {n}");
        }
        ["put" | "delete" | "fill", ..] => {
            return refuse(
                "usage: put <iface> <resource> [<value>...] | delete <iface> <resource> \
                 [<value>...] | fill <iface> <resource> <n>",
            );
        }
        _ => {}
    }
    let Some(verb) = words.first() else {
        return refuse("an empty line");
    };
    if !["status", "check", "retire", "fault", "confirm"].contains(verb) {
        return refuse("unknown command");
    }
    let Some(h) = health else {
        return refuse("health.v1 is not implemented here (--health)");
    };
    let done = match words.as_slice() {
        ["status", level, ..] => match parse_level(level) {
            Some(Some(l)) => h.set_status(l, rest(2)).await,
            Some(None) => {
                return refuse("a status is UNSPECIFIED only at start (health.v1 §2.6)");
            }
            None => return refuse("a level is OK, DEGRADED or FAILED"),
        },
        ["check", name, level, ..] => match parse_level(level) {
            Some(Some(l)) => h.set_check(name, l, rest(3)).await,
            Some(None) => h.set_check_unknown(name, rest(3)).await,
            None => return refuse("a level is OK, DEGRADED, FAILED or UNSPECIFIED"),
        },
        ["retire", name] => h.retire_check(name).await,
        ["fault", code, level, ..] => match parse_level(level) {
            Some(Some(l)) => h
                .fault(code, l, rest(3))
                .await
                .map(|()| h.status().expect("put at start")),
            _ => return refuse("a fault's level is OK, DEGRADED or FAILED (health.v1 §2.3)"),
        },
        ["confirm", on @ ("on" | "off")] => {
            h.set_confirming(*on == "on");
            Ok(h.status().expect("put at start"))
        }
        _ => {
            return refuse(
                "usage: status <LEVEL> [<reason>] | check <name> <LEVEL> [<detail>] | \
                 retire <name> | fault <code> <LEVEL> [<detail>] | confirm on|off",
            );
        }
    };
    match done {
        Ok(e) => describe(&e),
        Err(e) => refuse(&e.to_string()),
    }
}

#[tokio::main]
async fn main() {
    // The runtime's logs go to stderr (warnings unless `RUST_LOG` says
    // otherwise), so a runner driving this owner as a black box sees what
    // the runtime says, such as hostid.v1's ephemeral warning (§2.6, 0.3),
    // and config.v1's requests, redacted (§2.15).
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("warn,zenkey::config_v1=info")
            }),
        )
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = match Options::parse(&args) {
        Ok(o) => o,
        Err(usage) => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    };
    // Answer each line of standard input, and run until it closes.
    let (lines, commands) = flume::unbounded::<String>();
    let stdin_closed = async {
        let _ = tokio::task::spawn_blocking(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if lines.send(line).is_err() {
                    break;
                }
            }
        })
        .await;
    };
    if let Err(e) = run_with(opts, |line| println!("{line}"), commands, stdin_closed).await {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
