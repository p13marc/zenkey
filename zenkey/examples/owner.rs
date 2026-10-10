//! A zk2 owner to interoperate with (#609's live half, #610): it brings up
//! one service implementing the given contracts and runs until its standard
//! input closes.
//!
//! ```text
//! cargo run -p zenkey --example owner -- [--connect <endpoint>] [--hostid-root <dir>] [--hostid-ephemeral]
//!     [--health] [--health-status <LEVEL>[:<reason>]] [--health-check <name>=<LEVEL>[:<detail>]]...
//!     [--tokenless <iface>]... [--clock-reference <key>] [--clock-offset-ms <ms>]
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
//! ```
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

use std::future::Future;
use std::io::BufRead;
use std::path::PathBuf;

use zenkey::health::{self, Effective, Health, Level};
use zenkey::hostid::{HostIdMinter, HostIdSource};
use zenkey::model::authoring::Kind;
use zenkey::model::contract::{Body, load_path};
use zenkey::model::schema::TypeId;
use zenkey::model::template::Bindings;
use zenkey::state::Minter;
use zenkey::{Address, Implementation, OpError, ServiceBuilder, ServiceConfig};

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

    let mut b = ServiceBuilder::with_hostid(&session, config, minter);
    b.minter().simulate_offset(opts.clock_offset_ms);
    let mut states = Vec::new();
    let mut servers = Vec::new();
    let none = Bindings::new();
    for imp in imps {
        let iface = imp.iface().clone();
        let resources = imp.contract().resources.clone();
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
    // §8.2 "State values" (F-68): the value is put before the tokens, so a
    // GET made the moment they appear finds it.
    for w in &states {
        w.put("ok").await?;
    }
    let svc = b.start().await?;
    say(format!("ready {}", svc.instance_key()?));

    let mut until = std::pin::pin!(until);
    let mut open = true;
    loop {
        tokio::select! {
            () = &mut until => break,
            line = commands.recv_async(), if open => match line {
                Ok(line) => say(command(&line, health.as_ref(), svc.minter()).await),
                Err(_) => open = false,
            },
        }
    }
    while let Ok(line) = commands.try_recv() {
        say(command(&line, health.as_ref(), svc.minter()).await);
    }
    svc.close().await?;
    drop(servers);
    session.close().await?;
    Ok(())
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

/// Answers one command line (the module doc's).
async fn command(line: &str, health: Option<&Health>, minter: &Minter) -> String {
    let words: Vec<&str> = line.split_whitespace().collect();
    let refuse = |why: &str| format!("refused {}: {why}", words.join(" "));
    let rest = |n: usize| words.get(n..).map(|w| w.join(" ")).unwrap_or_default();
    if let ["clock-offset", ms] = words.as_slice() {
        return match ms.parse::<i64>() {
            Ok(ms) => {
                minter.simulate_offset(ms);
                format!("clock-offset {ms}")
            }
            Err(e) => refuse(&e.to_string()),
        };
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
    // the runtime says, such as hostid.v1's ephemeral warning (§2.6, 0.3).
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
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
