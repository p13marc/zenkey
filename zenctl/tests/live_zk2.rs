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
//! FJ5's acts and reads (`call`, `get state`, `watch`, `replay
//! --namespace`) bring up services of their own on a bare bus
//! ([`Bus::bare`]): operations.md's common setup (`tc.v1`, from the
//! runtime's scenario contracts), a `tc.netif.v1` owner with its state and
//! its stream, and an `archive.v1` recording it. Each case cites the
//! scenario section it runs.
//!
//! FJ8b's raw observers and checks (`echo`, `rate`, `field`, `timeline`,
//! `snapshot`, `check expect|schema|probe`, `watchdog`) watch a publishing
//! `tc.netif.v1` owner on a bare bus, beside a principal that is not the
//! owner putting what P3 forbids — a key that is not zk2, bytes that do not
//! decode as the type, a sample off the declared QoS — which is what each
//! verb must name apart (the tooling guide's O1–O7).
//!
//! CC's `config.v1` (#768) brings up `scenarios.md`'s `lab/rf0` through
//! `ServiceBuilder::config_v1` on the owner example's simulated device: the
//! `config` verbs end to end, §13 (the compact status across a constrained
//! face, a router whose access control denies the face's `@zk` and the
//! state subscription), §14 (a tool's answers), and the doctor's and
//! `check conform`'s config checks.
//!
//! DD's `desired.v1` (#840) brings up `scenarios.md`'s `lab/author` and
//! `lab/worker` as the owner example's `--desired-author` and
//! `--desired-target` do, on its simulated drive, and runs §9, a tool's
//! questions: `zenctl desired`'s answers and exits, the doctor's
//! `desired-*` checks and `check conform`'s `desired-*` cases.
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
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use zenkey::archive::{Archive, ArchiveConfig, Recorded};
use zenkey::model::contract::{Contract, load_path};
use zenkey::model::grammar::IfaceId;
use zenkey::model::template::Bindings;
use zenkey::{Call, Implementation, OpError, Service, ServiceBuilder, ServiceConfig};
use zenoh::qos::Priority;

/// A panic whose unwind hangs ends in seconds, with stacks (#804).
#[path = "../../zenkey/tests/common/panic_watchdog.rs"]
mod panic_watchdog;

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
    panic_watchdog::install();
    let mut c = zenoh::Config::default();
    c.insert_json5("scouting/multicast/enabled", "false")
        .expect("config");
    c.insert_json5("scouting/gossip/enabled", "false")
        .expect("config");
    c
}

/// A router on an ephemeral loopback port, and the endpoint it bound.
async fn router() -> (zenoh::Session, String) {
    router_with(false, None).await
}

/// [`router`], its admin space on or off (zenoh's default is off), linked
/// to the router at `upstream` when given (FK1: `admin graph`, `storage gen
/// --check` beside a live router).
async fn router_with(admin: bool, upstream: Option<&str>) -> (zenoh::Session, String) {
    let mut c = base_config();
    c.insert_json5("mode", "\"router\"").expect("config");
    c.insert_json5("listen/endpoints", r#"["tcp/127.0.0.1:0"]"#)
        .expect("config");
    c.insert_json5("adminspace/enabled", if admin { "true" } else { "false" })
        .expect("config");
    if let Some(up) = upstream {
        c.insert_json5("connect/endpoints", &format!("[\"{up}\"]"))
            .expect("config");
    }
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
            .map(zenkey::implementation::resource_name)
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
/// zenctl connects. `keep` holds whatever else a case brings up and must
/// outlive its runs: operation servers, an archive, a publishing task.
///
/// Fields drop in declaration order, so what a case kept goes first and
/// the router last: every client closes while its router still answers
/// (#804).
struct Bus {
    endpoint: String,
    home: PathBuf,
    keep: Vec<Box<dyn std::any::Any + Send>>,
    services: Vec<Service>,
    owners: zenoh::Session,
    _router: zenoh::Session,
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

impl Bus {
    /// The tcgui deployment, in `namespace` when given.
    async fn up(namespace: Option<&str>) -> Bus {
        let mut bus = Bus::bare(namespace).await;
        bus.services = tcgui(&bus.owners).await;
        bus
    }

    /// A router and an owners' session, in `namespace` when given, and no
    /// service yet: for a case that brings up its own (FJ5).
    async fn bare(namespace: Option<&str>) -> Bus {
        Bus::on(router().await, namespace).await
    }

    /// [`Bus::bare`] on a router whose admin space is on (FK1).
    async fn admin(namespace: Option<&str>) -> Bus {
        Bus::on(router_with(true, None).await, namespace).await
    }

    /// The bus around a router already up.
    async fn on((router, endpoint): (zenoh::Session, String), namespace: Option<&str>) -> Bus {
        let owners = client(&endpoint, namespace).await;
        Bus::with_owners(router, endpoint, owners)
    }

    /// The bus around a router already up and an owners' session already
    /// open: a router that authenticates its clients (#768, CC).
    fn with_owners(router: zenoh::Session, endpoint: String, owners: zenoh::Session) -> Bus {
        static NTH: AtomicU64 = AtomicU64::new(0);
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
            keep: Vec::new(),
            services: Vec::new(),
            owners,
            _router: router,
        }
    }

    /// Keeps `t` alive as long as the bus.
    fn keep<T: Send + 'static>(&mut self, t: T) {
        self.keep.push(Box::new(t));
    }

    /// `zenctl <args> -c <endpoint>`, once.
    async fn zenctl(&self, args: &[&str]) -> Run {
        self.zenctl_limited(args, None).await
    }

    /// [`Bus::zenctl`] under a soft `RLIMIT_MEMLOCK` of `memlock_kib`
    /// (`ulimit -l`), when given: what the doctor's SHM check reads (FJ6).
    async fn zenctl_limited(&self, args: &[&str], memlock_kib: Option<u64>) -> Run {
        let mut argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        argv.extend(["-c".into(), self.endpoint.clone()]);
        let home = self.home.clone();
        tokio::task::spawn_blocking(move || run(&argv, &home, memlock_kib, None))
            .await
            .expect("the zenctl runner")
    }

    /// `zenctl <args>` with no endpoint: a verb that opens no session
    /// (FJ8b: `snapshot diff`), which a `-c` would be refused by.
    async fn offline(&self, args: &[&str]) -> Run {
        let argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let home = self.home.clone();
        tokio::task::spawn_blocking(move || run(&argv, &home, None, None))
            .await
            .expect("the zenctl runner")
    }

    /// [`Bus::zenctl`] with `stdin` piped in (FJ8a: `pub --from ndjson`).
    async fn zenctl_stdin(&self, args: &[&str], stdin: &str) -> Run {
        let mut argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        argv.extend(["-c".into(), self.endpoint.clone()]);
        let home = self.home.clone();
        let stdin = stdin.as_bytes().to_vec();
        tokio::task::spawn_blocking(move || run(&argv, &home, None, Some(stdin)))
            .await
            .expect("the zenctl runner")
    }

    /// `zenctl <args>` in the background: a mock owner (`gen`, `serve`)
    /// that runs while a case asks it things (FJ8a).
    fn spawn(&self, args: &[&str]) -> tokio::task::JoinHandle<Run> {
        let mut argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        argv.extend(["-c".into(), self.endpoint.clone()]);
        let home = self.home.clone();
        tokio::task::spawn_blocking(move || run(&argv, &home, None, None))
    }

    /// [`Bus::spawn`], killed at `limit` rather than [`RUN_LIMIT`]: a
    /// window longer than a minute (#721).
    fn spawn_within(&self, args: &[&str], limit: Duration) -> tokio::task::JoinHandle<Run> {
        let mut argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        argv.extend(["-c".into(), self.endpoint.clone()]);
        let home = self.home.clone();
        tokio::task::spawn_blocking(move || run_within(&argv, &home, None, None, limit))
    }

    /// [`Bus::spawn`], and a signal on the first line of its stderr that
    /// holds `needle`, the moment a case acts on (#819 TE, F-154: the
    /// "listening" line).
    fn spawn_on_line(
        &self,
        args: &[&str],
        needle: &'static str,
    ) -> (
        tokio::sync::oneshot::Receiver<()>,
        tokio::task::JoinHandle<Run>,
    ) {
        let mut argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        argv.extend(["-c".into(), self.endpoint.clone()]);
        let home = self.home.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let run = tokio::task::spawn_blocking(move || {
            run_watched(&argv, &home, None, None, RUN_LIMIT, Some((needle, tx)))
        });
        (rx, run)
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
/// drained on threads of their own, killed at [`RUN_LIMIT`]. Under a
/// memlock limit, through `sh` and `ulimit -l` — lowering a soft limit
/// needs no privilege.
fn run(argv: &[String], home: &Path, memlock_kib: Option<u64>, stdin: Option<Vec<u8>>) -> Run {
    run_within(argv, home, memlock_kib, stdin, RUN_LIMIT)
}

/// [`run`], killed at `limit`: a case whose window is longer than
/// [`RUN_LIMIT`] (health.v1's 65 s waits, #721).
fn run_within(
    argv: &[String],
    home: &Path,
    memlock_kib: Option<u64>,
    stdin: Option<Vec<u8>>,
    limit: Duration,
) -> Run {
    run_watched(argv, home, memlock_kib, stdin, limit, None)
}

/// [`run_within`], signalling `on_line`'s sender at the first stderr line
/// that holds its needle, as the line is read.
fn run_watched(
    argv: &[String],
    home: &Path,
    memlock_kib: Option<u64>,
    stdin: Option<Vec<u8>>,
    limit: Duration,
    on_line: Option<(&'static str, tokio::sync::oneshot::Sender<()>)>,
) -> Run {
    use std::io::{BufRead as _, Read as _, Write as _};
    let zenctl = env!("CARGO_BIN_EXE_zenctl");
    let mut command = match memlock_kib {
        Some(kib) => {
            let mut c = Command::new("sh");
            c.arg("-c")
                .arg(format!("ulimit -l {kib} && exec \"$0\" \"$@\""))
                .arg(zenctl);
            c
        }
        None => Command::new(zenctl),
    };
    let mut child = command
        .args(argv)
        .env_remove("ZENCTL_CONTEXT")
        .env_remove("ZENKEY_EXPLORER_CONTEXT")
        .env_remove("ZENCTL_ZENOH_CONFIG")
        .env_remove("ZENCTL_FORMAT")
        .env_remove("ZENCTL_BASE")
        .env("ZENKEY_EXPLORER_CONFIG_DIR", home)
        .env("NO_COLOR", "1")
        .env("RUST_LOG", "off")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn zenctl");
    if let Some(bytes) = stdin {
        // Written whole, then closed: the end of stdin is the end of the
        // pipe the verb reads.
        let mut pipe = child.stdin.take().expect("stdin");
        pipe.write_all(&bytes).expect("write stdin");
    }
    let drain = |mut pipe: Box<dyn std::io::Read + Send>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = pipe.read_to_string(&mut s);
            s
        })
    };
    let out = drain(Box::new(child.stdout.take().expect("stdout")));
    let err = {
        let pipe = child.stderr.take().expect("stderr");
        std::thread::spawn(move || {
            let mut on_line = on_line;
            let mut s = String::new();
            let mut lines = std::io::BufReader::new(pipe);
            let mut line = String::new();
            while lines.read_line(&mut line).is_ok_and(|n| n > 0) {
                if on_line.as_ref().is_some_and(|(n, _)| line.contains(n)) {
                    let (_, tx) = on_line.take().expect("just held");
                    let _ = tx.send(());
                }
                s.push_str(&line);
                line.clear();
            }
            s
        })
    };
    let deadline = Instant::now() + limit;
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
        .map(zenkey::implementation::resource_name)
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

// ── FJ5: acting and reading through a contract ─────────────────────────────

/// The runtime's scenario contracts (`zenkey/tests/contracts`): `tc.v1` is
/// operations.md's common setup — an exclusive templated `set` with a
/// `TcError`, a fan-out `diagnostics`, and a many-reply `listing` with a
/// per-replier summary.
fn scenario_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../zenkey/tests/contracts/{name}.toml"))
}

fn scenario_contract(name: &str) -> Contract {
    let l = load_path(&scenario_path(name));
    l.contract
        .unwrap_or_else(|| panic!("{name} does not load:\n{}", l.report))
}

/// How one operations.md `tc` instance answers.
#[derive(Clone, Copy, Default)]
struct Tc {
    /// `diagnostics` holds every call past any caller's timeout.
    frozen: bool,
    /// `diagnostics` refuses every call `busy`.
    busy: bool,
    /// `listing` sends its values and returns before its summary.
    cut: bool,
}

/// The executions an instance saw: every call that reached a handler.
#[derive(Default)]
struct Seen {
    set: AtomicU64,
}

/// A task that stops when the case ends.
struct Task(tokio::task::JoinHandle<()>);

impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// operations.md's `tc` at `address`, on the owners' session: `set` over its
/// template (`eth9` is refused `app`, with a `TcError` detail),
/// `diagnostics`, `listing` (three values, then the summary unless `cut`),
/// and `reset` on its members `eth0` and `eth1`, a queryable each. Every
/// other resource is exposed and left unserved.
async fn tc_instance(bus: &mut Bus, address: &str, how: Tc) -> Arc<Seen> {
    let tc = iface("tc.v1");
    let seen = Arc::new(Seen::default());
    let mut b = ServiceBuilder::new(
        &bus.owners,
        ServiceConfig::new(address.parse().expect("an address")),
    );
    b.implement(Implementation::new(scenario_contract("tc.v1")))
        .expect("implement");
    let s = Arc::clone(&seen);
    let set = b
        .serve(&tc, "@op/interfaces/{if}/set", None, move |call: Call| {
            let s = Arc::clone(&s);
            async move {
                let req: Value = call.request()?;
                s.set.fetch_add(1, Ordering::SeqCst);
                let eth9 = call
                    .values()
                    .and_then(|v| v.get("if"))
                    .is_some_and(|v| v.first().is_some_and(|i| i == "eth9"));
                if eth9 {
                    return Err(OpError::app(
                        "the kernel refused",
                        &json!({"kind": "kernel", "message": "no such device"}),
                    ));
                }
                call.reply_value(&json!({"up": req["up"]})).await?;
                Ok(())
            }
        })
        .await
        .expect("serve set");
    let host = address.to_owned();
    let diagnostics = b
        .serve_value(
            &tc,
            "@op/diagnostics",
            None,
            move |_call: Call, _req: Value| {
                let host = host.clone();
                async move {
                    if how.frozen {
                        tokio::time::sleep(Duration::from_secs(4)).await;
                    }
                    if how.busy {
                        return Err(OpError::busy("a scan is running"));
                    }
                    Ok(json!({"ok": true, "host": host}))
                }
            },
        )
        .await
        .expect("serve diagnostics");
    let listing = b
        .serve(&tc, "@op/listing", None, move |call: Call| async move {
            for n in 0..3 {
                call.reply_value(&json!({"n": n})).await?;
            }
            if !how.cut {
                call.summary_value(&json!({"count": 3})).await?;
            }
            Ok(())
        })
        .await
        .expect("serve listing");
    // §2's `reset`, a fan-out over a template, served per member.
    let mut resets = Vec::new();
    for member in ["eth0", "eth1"] {
        let host = address.to_owned();
        let values: Bindings = [("if".to_owned(), vec![member.to_owned()])].into();
        resets.push(
            b.serve_value(
                &tc,
                "@op/interfaces/{if}/reset",
                Some(&values),
                move |_call: Call, _req: Value| {
                    let host = host.clone();
                    async move { Ok(json!({"ok": true, "host": host, "if": member})) }
                },
            )
            .await
            .expect("serve reset"),
        );
    }
    let names: Vec<String> = scenario_contract("tc.v1")
        .resources
        .iter()
        .map(zenkey::implementation::resource_name)
        .collect();
    for n in names {
        let _ = b.expose(&tc, &n);
    }
    bus.services.push(b.start().await.expect("start"));
    bus.keep((set, diagnostics, listing, resets));
    seen
}

/// A tcgui `host-a/tc` implementing `tc.netif.v1`, every resource exposed,
/// its state served: the owner of the state and the stream cases.
async fn netif_owner(bus: &Bus) -> Service {
    let netif = iface("tc.netif.v1");
    let mut b = ServiceBuilder::new(
        &bus.owners,
        ServiceConfig::new("host-a/tc".parse().expect("an address")),
    );
    b.implement(Implementation::new(contract("tcgui/tc.netif.v1")))
        .expect("implement");
    for r in &contract("tcgui/tc.netif.v1").resources {
        let _ = b.expose(&netif, &zenkey::implementation::resource_name(r));
    }
    b.serve_state(&netif).expect("serve state");
    b.start().await.expect("start")
}

fn member(ns: &str, iface_name: &str) -> Bindings {
    [
        ("ns".to_owned(), vec![ns.to_owned()]),
        ("iface".to_owned(), vec![iface_name.to_owned()]),
    ]
    .into()
}

/// Until `service list` shows every one of `addresses` with its descriptor
/// served: a service is up when presence says so.
async fn wait_for(bus: &Bus, addresses: &[&str]) {
    let want: BTreeSet<String> = addresses.iter().map(|a| (*a).to_owned()).collect();
    let run = bus
        .until(
            &["service", "list", "--timeout", "2", "--format", "json"],
            |r| {
                r.code == 0
                    && serde_json::from_str::<Value>(&r.stdout).is_ok_and(|d| {
                        let served: BTreeSet<String> = d["rows"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|i| i["descriptor"]["answer"] == "served")
                            .filter_map(|i| i["address"].as_str().map(str::to_owned))
                            .collect();
                        want.is_subset(&served)
                    })
            },
        )
        .await;
    exits(&run, 0);
}

/// The rows of a `--format json` document, by tag.
fn rows_of<'a>(doc: &'a Value, tag: &str) -> Vec<&'a Value> {
    doc["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["row"] == tag)
        .collect()
}

/// operations.md §3 and §5 (with §4's retry), through `call`: a value on
/// the call's own key, rendered through the contract retrieved from the
/// service (§8.4); an `app` refusal with its detail rendered through the
/// `error` type; silence from a frozen server attributed `present`, an
/// idempotent operation called again after it; and silence from an address
/// no token names, worded as what this reader could see.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn call_keeps_a_value_a_refusal_and_an_attributed_silence_apart() {
    let mut bus = Bus::bare(None).await;
    let seen = tc_instance(
        &mut bus,
        "h1/tc",
        Tc {
            frozen: true,
            ..Tc::default()
        },
    )
    .await;
    wait_for(&bus, &["h1/tc"]).await;

    let set = |member: &'static str| {
        vec![
            "call",
            "h1/tc",
            "tc.v1",
            "interfaces/{if}/set",
            r#"{"up":true}"#,
            "--param",
            member,
            "--format",
            "json",
        ]
    };
    let run = bus.until(&set("if=eth0"), |r| r.code == 0).await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["report"], "operation");
    assert_eq!(doc["answer"], "value", "{run}");
    assert_eq!(doc["mode"], "concrete");
    assert_eq!(
        doc["selectors"],
        json!(["zk2/h1/tc/tc.v1/@op/interfaces/eth0/set"])
    );
    assert_eq!(
        doc["fingerprint"],
        Implementation::new(scenario_contract("tc.v1"))
            .fingerprint()
            .to_string()
    );
    assert_eq!(
        doc["reply"]["key"],
        "zk2/h1/tc/tc.v1/@op/interfaces/eth0/set"
    );
    assert_eq!(doc["reply"]["as"], "value");
    assert_eq!(doc["reply"]["declared"], "json:SetResponse");
    assert_eq!(doc["reply"]["value"], json!({"up": true}));
    assert_eq!(doc["reply"]["resource"]["member"], "response");

    let run = bus.zenctl(&set("if=eth9")).await;
    exits(&run, 1);
    let doc = run.json();
    assert_eq!(doc["answer"], "refused", "{run}");
    assert_eq!(doc["envelope"]["code"], "app");
    assert_eq!(doc["envelope"]["message"], "the kernel refused");
    assert_eq!(doc["envelope"]["detail"]["declared"], "json:TcError");
    assert_eq!(doc["envelope"]["detail"]["value"]["kind"], "kernel");
    assert!(
        seen.set.load(Ordering::SeqCst) >= 2,
        "both reached the server"
    );

    let run = bus
        .zenctl(&[
            "call",
            "h1/tc",
            "tc.v1",
            "diagnostics",
            "--timeout",
            "1",
            "--retries",
            "1",
            "--format",
            "json",
        ])
        .await;
    exits(&run, 2);
    let doc = run.json();
    assert_eq!(doc["answer"], "silent", "{run}");
    assert_eq!(doc["silence"]["presence"], "present");
    assert_eq!(
        doc["silence"]["attempts"], 2,
        "idempotent: called again after silence (O4)"
    );
    assert!(
        doc["notes"]
            .to_string()
            .contains("never \\\"no such operation\\\""),
        "{run}"
    );

    // No token names `h9/tc`: the revision comes from --contracts, and the
    // silence is what this reader could see, not a verdict that it is gone.
    let path = scenario_path("tc.v1");
    let run = bus
        .zenctl(&[
            "call",
            "h9/tc",
            "tc.v1",
            "diagnostics",
            "--timeout",
            "1",
            "--contracts",
            path.to_str().expect("a UTF-8 path"),
            "--format",
            "json",
        ])
        .await;
    exits(&run, 2);
    assert_eq!(
        run.json()["silence"]["presence"],
        "no_token_visible",
        "{run}"
    );
}

/// operations.md §2 and §5, through `call`: a fan-out to an operation that
/// allows one reaches every holder of the interface, each value attributed
/// by its key; a `busy` envelope is reported unattributed, and the holder
/// that sent no value is named from presence. Over a template (step 3),
/// the parameter left out is a wildcard, and each member's reply is
/// attributed by its own concrete key. A fan-out to an operation
/// that forbids one — a `*` in the address, or a parameter left out — is
/// refused before anything is sent: no handler runs (O2).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fan_out_is_attributed_by_key_and_one_the_operation_forbids_is_never_sent() {
    let mut bus = Bus::bare(None).await;
    let h1 = tc_instance(&mut bus, "h1/tc", Tc::default()).await;
    let h2 = tc_instance(&mut bus, "h2/tc", Tc::default()).await;
    let h3 = tc_instance(
        &mut bus,
        "h3/tc",
        Tc {
            busy: true,
            ..Tc::default()
        },
    )
    .await;
    wait_for(&bus, &["h1/tc", "h2/tc", "h3/tc"]).await;

    let run = bus
        .until(
            &[
                "call",
                "*/tc",
                "tc.v1",
                "diagnostics",
                "--timeout",
                "2",
                "--format",
                "json",
            ],
            |r| {
                r.code == 1
                    && serde_json::from_str::<Value>(&r.stdout).is_ok_and(|d| {
                        rows_of(&d, "replier").len() == 2
                            && d["replies"]["presence"]["unheard"] == json!(["h3/tc"])
                    })
            },
        )
        .await;
    exits(&run, 1);
    let doc = run.json();
    assert_eq!(doc["answer"], "replies", "{run}");
    assert_eq!(doc["mode"], "fanout");
    assert_eq!(doc["selectors"], json!(["zk2/*/tc/tc.v1/@op/diagnostics"]));
    let mut heard = BTreeSet::new();
    for r in rows_of(&doc, "replier") {
        let address = r["address"].as_str().expect("an address");
        assert_eq!(r["key"], format!("zk2/{address}/tc.v1/@op/diagnostics"));
        assert_eq!(r["replies"][0]["value"]["host"], address, "{r}");
        heard.insert(address.to_owned());
    }
    assert_eq!(
        heard,
        BTreeSet::from(["h1/tc".to_owned(), "h2/tc".to_owned()])
    );
    assert_eq!(
        doc["replies"]["refusals"],
        json!([{"code": "busy", "message": "a scan is running"}]),
        "unattributed: a reply_err carries no key"
    );
    assert_eq!(doc["replies"]["presence"]["complete"], true);

    // Over a template: the parameter left out is a wildcard in the key, and
    // each member's reply is attributed by its own concrete key.
    let run = bus
        .until(
            &[
                "call",
                "*/tc",
                "tc.v1",
                "interfaces/{if}/reset",
                "--timeout",
                "2",
                "--format",
                "json",
            ],
            |r| {
                r.code == 0
                    && serde_json::from_str::<Value>(&r.stdout)
                        .is_ok_and(|d| rows_of(&d, "replier").len() == 6)
            },
        )
        .await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["mode"], "fanout", "{run}");
    assert_eq!(
        doc["selectors"],
        json!(["zk2/*/tc/tc.v1/@op/interfaces/*/reset"])
    );
    let mut members = BTreeSet::new();
    for r in rows_of(&doc, "replier") {
        let (address, member) = (
            r["address"].as_str().expect("an address"),
            r["values"]["if"][0].as_str().expect("a member"),
        );
        assert_eq!(
            r["key"],
            format!("zk2/{address}/tc.v1/@op/interfaces/{member}/reset")
        );
        assert_eq!(r["replies"][0]["value"]["if"], member, "{r}");
        members.insert(format!("{address} {member}"));
    }
    assert_eq!(members.len(), 6, "every host, every member: {members:?}");

    for args in [
        vec![
            "call",
            "*/tc",
            "tc.v1",
            "interfaces/{if}/set",
            r#"{"up":true}"#,
            "--param",
            "if=eth0",
        ],
        vec![
            "call",
            "h1/tc",
            "tc.v1",
            "interfaces/{if}/set",
            r#"{"up":true}"#,
        ],
    ] {
        let run = bus.zenctl(&args).await;
        exits(&run, 2);
        assert!(run.stdout.is_empty(), "{run}");
        assert!(
            run.stderr.contains(r#"fanout = "forbidden""#)
                && run.stderr.contains("nothing was sent"),
            "{run}"
        );
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    for seen in [&h1, &h2, &h3] {
        assert_eq!(seen.set.load(Ordering::SeqCst), 0, "no handler ran");
    }
}

/// operations.md §6, through `call`: a many-reply fan-out keeps every value
/// and each replier's summary, and a replier whose handler returned before
/// its summary is reported possibly partial, beside the runtime's
/// `internal` envelope; one address alone is a concrete many-reply call.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_many_reply_call_reports_a_replier_without_its_summary_possibly_partial() {
    let mut bus = Bus::bare(None).await;
    tc_instance(&mut bus, "h1/tc", Tc::default()).await;
    tc_instance(
        &mut bus,
        "h2/tc",
        Tc {
            cut: true,
            ..Tc::default()
        },
    )
    .await;
    wait_for(&bus, &["h1/tc", "h2/tc"]).await;

    let run = bus
        .until(
            &[
                "call",
                "*/tc",
                "tc.v1",
                "listing",
                "--timeout",
                "2",
                "--format",
                "json",
            ],
            |r| {
                serde_json::from_str::<Value>(&r.stdout)
                    .is_ok_and(|d| rows_of(&d, "replier").len() == 2)
            },
        )
        .await;
    exits(&run, 1);
    let doc = run.json();
    assert_eq!(doc["replies"]["summary_declared"], true, "{run}");
    let by: std::collections::BTreeMap<String, &Value> = rows_of(&doc, "replier")
        .into_iter()
        .map(|r| (r["address"].as_str().expect("an address").to_owned(), r))
        .collect();
    let (whole, cut) = (by["h1/tc"], by["h2/tc"]);
    assert_eq!(whole["replies"].as_array().map(Vec::len), Some(3));
    assert_eq!(whole["summaries"][0]["value"], json!({"count": 3}));
    assert_eq!(whole["summaries"][0]["resource"]["member"], "summary");
    assert_eq!(whole["possibly_partial"], false);
    assert_eq!(cut["replies"].as_array().map(Vec::len), Some(3));
    assert!(cut.get("summaries").is_none(), "{cut}");
    assert_eq!(cut["possibly_partial"], true);
    assert_eq!(doc["replies"]["refusals"][0]["code"], "internal", "{run}");

    let run = bus
        .zenctl(&["call", "h1/tc", "tc.v1", "listing", "--format", "json"])
        .await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["mode"], "concrete");
    assert_eq!(doc["selectors"], json!(["zk2/h1/tc/tc.v1/@op/listing"]));
    let rows = rows_of(&doc, "replier");
    assert_eq!(rows.len(), 1, "{run}");
    assert_eq!(rows[0]["possibly_partial"], false);
}

/// state.md §3 and §4, through `get state`: the owner's current state with
/// the stamp of the owner's own clock (S1, S4); every member when a
/// parameter is left out; silence once the owner is gone — exit 2, never
/// "no value" (S6); and the same key's last-known state from a running
/// archive, labelled last-known (S5).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn get_state_reads_the_owner_then_an_archive_and_never_confuses_them() {
    let bus = Bus::bare(None).await;
    let origin = "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0";
    let archiving = client(&bus.endpoint, None).await;
    let archive = Archive::start(
        &archiving,
        ArchiveConfig {
            service: ServiceConfig::new("ground/archive".parse().expect("an address")),
            records: vec![Recorded {
                owner: "host-a/tc".parse().expect("an address"),
                selector: "zk2/host-a/tc/tc.netif.v1/state/interfaces/*/*".to_owned(),
                implementation: Implementation::new(contract("tcgui/tc.netif.v1")),
            }],
            peers: Vec::new(),
            unconfirmed_horizon: None,
            watch: None,
        },
    )
    .await
    .expect("an archive");
    let mut owner = netif_owner(&bus).await;
    let writer = owner
        .state_writer(
            &iface("tc.netif.v1"),
            "state/interfaces/{ns}/{iface}",
            &member("default", "eth0"),
        )
        .await
        .expect("a state writer");
    let value = json!({"name": "eth0", "is_up": true});
    writer.put_value(&value).await.expect("a put");
    let deadline = Instant::now() + SETTLE;
    while archive.confirmed(origin).is_none() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        archive.confirmed(origin).is_some(),
        "the archive recorded it"
    );

    let one = [
        "get",
        "state",
        "host-a/tc",
        "tc.netif.v1",
        "interfaces/{ns}/{iface}",
        "--param",
        "ns=default",
        "--param",
        "iface=eth0",
        "--format",
        "json",
    ];
    let run = bus.until(&one, |r| r.code == 0).await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["report"], "state");
    assert_eq!(doc["reading"], "current", "{run}");
    assert!(doc.get("archive").is_none());
    assert_eq!(doc["selectors"], json!([origin]));
    let row = &doc["rows"][0];
    assert_eq!(row["key"], origin);
    assert_eq!(row["state"], "value");
    assert_eq!(row["payload"]["declared"], "json:NetworkInterface");
    assert_eq!(row["payload"]["value"], value);
    assert_eq!(
        row["timestamp"]["clock"],
        bus.owners.zid().to_string(),
        "the owner's own stamp (S1)"
    );
    assert!(row.get("confirmed").is_none(), "a current read has none");

    let run = bus
        .zenctl(&[
            "get",
            "state",
            "host-a/tc",
            "tc.netif.v1",
            "interfaces/{ns}/{iface}",
            "--format",
            "json",
        ])
        .await;
    exits(&run, 0);
    assert_eq!(
        run.json()["selectors"],
        json!(["zk2/host-a/tc/tc.netif.v1/state/interfaces/*/*"]),
        "{run}"
    );

    drop(writer);
    owner.close().await.expect("the owner closes");
    let netif = examples().join("tcgui/tc.netif.v1.toml");
    let netif = netif.to_str().expect("a UTF-8 path");
    let mut silent = one.to_vec();
    silent.extend(["--timeout", "1", "--contracts", netif]);
    let run = bus.until(&silent, |r| r.code == 2).await;
    exits(&run, 2);
    let doc = run.json();
    assert_eq!(doc["reading"], "current");
    assert!(
        doc.get("rows").is_none(),
        "silence is no rows, never \"no value\": {run}"
    );
    assert!(doc["notes"].to_string().contains("silence"), "{run}");

    let mut last = silent.clone();
    last.extend(["--last-known", "ground/archive"]);
    let run = bus.until(&last, |r| r.code == 0).await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["reading"], "last_known", "{run}");
    assert_eq!(doc["archive"], "ground/archive");
    assert_eq!(
        doc["selectors"],
        json!([zenkey::archive::archive_key(
            &"ground/archive".parse().expect("an address"),
            origin
        )])
    );
    let row = &doc["rows"][0];
    assert_eq!(row["key"], origin);
    assert_eq!(row["payload"]["value"], value);
    assert!(row["confirmed"].is_boolean(), "{row}");
    assert_eq!(row["identity"]["iface"], "tc.netif.v1");
    assert!(
        doc["notes"]
            .to_string()
            .contains("last-known, never current"),
        "{run}"
    );

    // #671 (FJ8b): a parameter left out is a wildcard over what the archive
    // holds (`archive::last_known_all`), one row per origin, never refused.
    let pattern = [
        "get",
        "state",
        "host-a/tc",
        "tc.netif.v1",
        "interfaces/{ns}/{iface}",
        "--param",
        "ns=default",
        "--last-known",
        "ground/archive",
        "--timeout",
        "1",
        "--contracts",
        netif,
        "--format",
        "json",
    ];
    let run = bus.until(&pattern, |r| r.code == 0).await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["reading"], "last_known", "{run}");
    let rows = doc["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 1, "the one origin the archive holds: {run}");
    assert_eq!(rows[0]["key"], origin);
    assert_eq!(rows[0]["payload"]["value"], value);
    drop(archive);
}

/// bindings.md §4 (R6), through `watch`: every sample the provider puts on
/// its own concrete key is delivered and rendered through the contract,
/// with its provider and its template values; a put on a wildcard key
/// reaches the subscription and is discarded by rule, counted apart from
/// any loss. `--count` and `--for` both end it, and say which did.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn watch_decodes_each_sample_and_counts_r6_discards_apart() {
    let mut bus = Bus::bare(None).await;
    let owner = netif_owner(&bus).await;
    let writer = owner
        .writer(
            &iface("tc.netif.v1"),
            "stream/bandwidth/{ns}/{iface}",
            &member("default", "eth0"),
        )
        .await
        .expect("a stream writer");
    bus.services.push(owner);
    let principal = client(&bus.endpoint, None).await;
    bus.keep(Task(tokio::spawn(async move {
        let mut n = 0u64;
        loop {
            n += 1;
            let _ = writer.put_value(&json!({"rx_bps": n, "tx_bps": n})).await;
            let _ = principal
                .put(
                    "zk2/host-a/*/tc.netif.v1/stream/bandwidth/default/eth0",
                    r#"{"rx_bps":0,"tx_bps":0}"#,
                )
                .await;
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })));
    wait_for(&bus, &["host-a/tc"]).await;

    let summary_of = |r: &Run| -> Option<Value> {
        r.stdout
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .find(|v| v["row"] == "summary")
    };
    let run = bus
        .until(
            &[
                "watch",
                "*/tc",
                "tc.netif.v1",
                "bandwidth/{ns}/{iface}",
                "--for",
                "2",
                "--format",
                "ndjson",
            ],
            |r| {
                r.code == 0
                    && summary_of(r).is_some_and(|s| {
                        s["received"].as_u64() > Some(0) && s["discarded"].as_u64() > Some(0)
                    })
            },
        )
        .await;
    exits(&run, 0);
    let lines: Vec<Value> = run
        .stdout
        .lines()
        .map(|l| serde_json::from_str(l).expect("one JSON object per line"))
        .collect();
    assert!(
        lines.iter().all(|l| l.get("row").is_some()),
        "a stream has no envelope"
    );
    let samples: Vec<&Value> = lines.iter().filter(|l| l["row"] == "sample").collect();
    for s in &samples {
        assert_eq!(s["provider"], "host-a/tc");
        assert_eq!(
            s["key"],
            "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0"
        );
        assert_eq!(s["values"], json!({"iface": ["eth0"], "ns": ["default"]}));
        assert_eq!(s["kind"], "put");
        assert_eq!(s["payload"]["as"], "value", "{s}");
        assert_eq!(s["payload"]["declared"], "json:BandwidthUpdate");
    }
    let summary = summary_of(&run).expect("a summary");
    assert_eq!(summary["received"], samples.len() as u64);
    assert_eq!(summary["ended"], "window");
    assert_eq!(summary["lagged"], 0);
    assert_eq!(
        summary["selectors"],
        json!(["zk2/*/tc/tc.netif.v1/stream/bandwidth/*/*"])
    );
    let discarded: u64 = lines
        .iter()
        .filter(|l| l["row"] == "discarded")
        .filter_map(|l| l["discarded"].as_u64())
        .sum();
    assert_eq!(summary["discarded"], discarded, "every discard was said");

    let run = bus
        .until(
            &[
                "watch",
                "host-a/tc",
                "tc.netif.v1",
                "stream/bandwidth/{ns}/{iface}",
                "--param",
                "iface=eth0",
                "--count",
                "3",
                "--for",
                "15",
                "--format",
                "ndjson",
            ],
            |r| r.code == 0,
        )
        .await;
    exits(&run, 0);
    let summary = summary_of(&run).expect("a summary");
    assert_eq!(summary["ended"], "count", "{run}");
    assert_eq!(summary["received"], 3);
    assert_eq!(
        summary["selectors"],
        json!(["zk2/host-a/tc/tc.netif.v1/stream/bandwidth/*/eth0"])
    );
}

/// The walkthrough's §4.1 replay (spike S13), through `replay --namespace`:
/// a capture recorded at the bus root, republished into namespace
/// `replay`, reaches a consumer there on the original address, and nothing
/// reaches the bus root. Replayed as recorded instead, every row is a key
/// its owner owns, where the owner runs: refused, and nothing published
/// (P3, the tooling guide's §5).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replay_into_a_namespace_reaches_its_consumers_and_never_the_owners() {
    let bus = Bus::bare(None).await;
    let key = "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0";
    let mut zrec = String::from(
        r#"{"zrec":1,"selectors":["zk2/host-a/tc/tc.netif.v1/stream/**"],"base":"","captured_at":"2026-10-08T00:00:00Z"}"#,
    );
    for n in 0..5 {
        let row = json!({
            "key": key,
            "t": n * 20_000,
            "delete": false,
            "value": {"rx_bps": n, "tx_bps": n},
            "encoding": "application/json",
        });
        zrec.push('\n');
        zrec.push_str(&row.to_string());
    }
    zrec.push('\n');
    let file = bus.home.join("bus.zrec");
    std::fs::write(&file, zrec).expect("the capture");
    let file = file.to_str().expect("a UTF-8 path");

    let in_replay = client(&bus.endpoint, Some("replay")).await;
    let heard = in_replay
        .declare_subscriber("zk2/host-a/tc/tc.netif.v1/stream/**")
        .await
        .expect("a consumer in `replay`");
    let at_root = client(&bus.endpoint, None).await;
    let root = at_root
        .declare_subscriber("zk2/**")
        .await
        .expect("a consumer at the bus root");

    let deadline = Instant::now() + SETTLE;
    let (run, sample) = loop {
        let run = bus
            .zenctl(&["replay", file, "--namespace", "replay", "--format", "json"])
            .await;
        let got = tokio::time::timeout(Duration::from_millis(500), heard.recv_async()).await;
        if let Ok(Ok(sample)) = got {
            break (run, sample);
        }
        assert!(Instant::now() < deadline, "nothing reached `replay`\n{run}");
    };
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["namespace"], "replay", "{run}");
    assert_eq!(doc["published"], 5);
    assert_eq!(doc["refused"], 0);
    assert_eq!(
        sample.key_expr().as_str(),
        key,
        "the original address, in the namespace"
    );

    let run = bus
        .zenctl(&["replay", file, "--force-base", "--format", "json"])
        .await;
    exits(&run, 1);
    let doc = run.json();
    assert_eq!(doc["published"], 0, "{run}");
    assert_eq!(doc["refused"], 5);
    assert!(run.stderr.contains("(P3, spec §6)"), "{run}");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        root.try_recv().ok().flatten().is_none(),
        "nothing reached the bus root, where the owners run"
    );
}

// ── doctor (FJ6) ───────────────────────────────────────────────────────────

/// A doctor run's settings for these cases: a grace period above a
/// re-mint's overlap and short enough to keep the suite quick.
const DOCTOR: [&str; 5] = ["doctor", "--grace", "0.5", "--timeout", "2"];

fn doctor_args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut args: Vec<&str> = DOCTOR.to_vec();
    args.extend_from_slice(extra);
    args
}

/// One check's row in a `doctor --format json` document.
fn check_row<'a>(doc: &'a Value, id: &str) -> &'a Value {
    rows_of(doc, "check")
        .into_iter()
        .find(|r| r["check"] == id)
        .unwrap_or_else(|| panic!("{id} has a row: {doc}"))
}

/// `doctor` over the tcgui deployment (presence.md §1, §2, §5): the
/// frontend's `scenario` role selects no provider, which is the finding and
/// exit 1; asked without it — and without the checks the routers' admin
/// space answers, which this router keeps off — every check asked is clean,
/// exit 0; pointed at a namespace nobody runs in, the scope is empty and
/// the run is no verdict, exit 2, never green.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn doctor_exits_on_its_findings_and_never_green_on_an_empty_scope() {
    let bus = Bus::up(None).await;
    let _ = listing(&bus, &[]).await;

    let run = bus
        .until(&doctor_args(&["--format", "json"]), |r| r.code == 1)
        .await;
    exits(&run, 1);
    let doc = run.json();
    assert_eq!(doc["report"], "doctor");
    assert_eq!(doc["scope"]["namespace"], "");
    assert_eq!(doc["scope"]["presence"]["services"], 3, "{run}");
    let binding = check_row(&doc, "binding-unsatisfied");
    assert_eq!(binding["verdict"]["answer"], "established", "{run}");
    assert_eq!(binding["section"], "§3.2 R5");
    assert_eq!(
        binding["findings"][0]["subject"],
        "ws-01/tcgui-frontend scenario"
    );
    // A required manifest role, as its descriptor says (§3.3, 0.10).
    assert_eq!(binding["findings"][0]["severity"], "error");
    for clean in [
        "split-brain",
        "token-missing",
        "descriptor-invalid",
        "contract-unavailable",
    ] {
        assert_eq!(
            check_row(&doc, clean)["verdict"]["answer"],
            "not_established",
            "{clean}: {run}"
        );
    }
    // The admin space is off on this router: S4 cannot be judged.
    assert_eq!(
        check_row(&doc, "storage-on-state")["verdict"]["answer"],
        "unobservable",
        "{run}"
    );
    assert_eq!(
        check_row(&doc, "state-stamp-foreign")["verdict"]["answer"],
        "not_asked"
    );

    let run = bus
        .zenctl(&doctor_args(&[
            "--skip",
            "binding-unsatisfied",
            "--skip",
            "storage-on-state",
            "--skip",
            "router-version-skew",
            "--format",
            "json",
        ]))
        .await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(
        check_row(&doc, "binding-unsatisfied")["verdict"]["answer"],
        "not_asked",
        "{run}"
    );
    // The info finding — no router answered the admin space — is worth
    // knowing and below the floor.
    assert_eq!(
        check_row(&doc, "admin-unreachable")["findings"][0]["severity"],
        "info",
        "{run}"
    );

    let run = bus
        .zenctl(&doctor_args(&["--namespace", "nobody", "--format", "json"]))
        .await;
    exits(&run, 2);
    let doc = run.json();
    let why = doc["unobservable"].as_str().expect("the empty scope");
    assert!(why.contains("no zk2 token visible to this reader"), "{run}");
    assert!(why.contains("\"nobody\""), "{run}");
    assert_eq!(
        check_row(&doc, "split-brain")["verdict"]["answer"],
        "unobservable"
    );
    assert!(
        run.stderr.contains("exit 2, the reserved non-verdict"),
        "{run}"
    );
}

/// The doctor reads a namespaced deployment through a session in its
/// namespace: `--namespace acme` finds the three services, and the bus
/// root, where none runs, is an empty scope.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn doctor_reads_a_deployment_in_its_namespace() {
    let bus = Bus::up(Some("acme")).await;
    let _ = listing(&bus, &["--namespace", "acme"]).await;
    let only = ["--check", "token-missing", "--check", "split-brain"];

    let mut args = doctor_args(&only);
    args.extend(["--namespace", "acme", "--format", "json"]);
    let run = bus.until(&args, |r| r.code == 0).await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["scope"]["namespace"], "acme");
    assert_eq!(doc["scope"]["presence"]["services"], 3, "{run}");
    assert_eq!(
        check_row(&doc, "token-missing")["verdict"]["answer"],
        "not_established"
    );

    let mut args = doctor_args(&only);
    args.extend(["--namespace", "", "--format", "json"]);
    let run = bus.zenctl(&args).await;
    exits(&run, 2);
    assert!(run.json()["unobservable"].is_string(), "{run}");
}

/// `--transitions` states one baseline line per check asked — firing on a
/// finding, ok when clean — and a watchdog's `doctor <CHECK-ID>` rule runs
/// the same doctor in the deployment's namespace, ending firing: exit 1.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn doctor_transitions_and_a_watchdog_rule_read_the_same_checks() {
    let bus = Bus::up(None).await;
    let _ = listing(&bus, &[]).await;

    let run = bus
        .until(
            &doctor_args(&[
                "--transitions",
                "--count",
                "1",
                "--every",
                "1",
                "--check",
                "binding-unsatisfied",
                "--check",
                "split-brain",
            ]),
            |r| r.code == 0 && r.stdout.contains("firing"),
        )
        .await;
    exits(&run, 0);
    let lines: Vec<Value> = run
        .stdout
        .lines()
        .map(|l| serde_json::from_str(l).expect("ndjson"))
        .collect();
    assert_eq!(lines.len(), 2, "one baseline line per check asked: {run}");
    let state = |rule: &str| {
        lines
            .iter()
            .find(|t| t["rule"] == rule)
            .unwrap_or_else(|| panic!("{rule}: {run}"))["to"]
            .clone()
    };
    assert_eq!(state("doctor binding-unsatisfied"), "firing");
    assert_eq!(state("doctor split-brain"), "ok");
    assert!(lines.iter().all(|t| t["from"].is_null()), "{run}");

    let run = bus
        .until(
            &[
                "watchdog",
                "--rule",
                "doctor binding-unsatisfied",
                "--every",
                "1",
                "--count",
                "1",
                "--timeout",
                "2",
            ],
            |r| r.code == 1,
        )
        .await;
    exits(&run, 1);
    assert!(run.stdout.contains("\"to\":\"firing\""), "{run}");
    assert!(
        run.stdout.contains("ws-01/tcgui-frontend scenario"),
        "{run}"
    );
}

/// §7.4 and §8.3 from the flags a run is given: under `ulimit -l 64` the
/// memlock is below the floor, an info finding that exits 0, and at the
/// floor it is clean; and a presence budget the deployment is over is a
/// warning, exit 1.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_local_memlock_and_the_presence_budget_are_judged() {
    let bus = Bus::up(None).await;
    let _ = listing(&bus, &[]).await;

    let run = bus
        .zenctl_limited(
            &doctor_args(&["--check", "shm-memlock-low", "--format", "json"]),
            Some(64),
        )
        .await;
    exits(&run, 0);
    let doc = run.json();
    let shm = check_row(&doc, "shm-memlock-low");
    assert_eq!(shm["verdict"]["answer"], "established", "{run}");
    assert_eq!(shm["findings"][0]["severity"], "info");
    assert!(
        shm["findings"][0]["evidence"]
            .as_str()
            .is_some_and(|e| e.contains("64 KiB")),
        "{run}"
    );
    // At the floor (8 MiB, Debian's default hard limit too): clean.
    let run = bus
        .zenctl_limited(
            &doctor_args(&["--check", "shm-memlock-low", "--format", "json"]),
            Some(8192),
        )
        .await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(
        check_row(&doc, "shm-memlock-low")["verdict"]["answer"],
        "not_established",
        "{run}"
    );

    let run = bus
        .until(
            &doctor_args(&[
                "--check",
                "presence-over-budget",
                "--presence-budget",
                "3",
                "--format",
                "json",
            ]),
            |r| r.code == 1,
        )
        .await;
    exits(&run, 1);
    let doc = run.json();
    let budget = check_row(&doc, "presence-over-budget");
    assert_eq!(budget["findings"][0]["severity"], "warning", "{run}");
    assert_eq!(budget["findings"][0]["subject"], "presence domain");
}

// ── FJ8a: writes and captures ──────────────────────────────────────────────

/// Every JSON document on a stdout that holds several (`gen` prints its
/// plan, then its report).
fn documents(s: &str) -> Vec<Value> {
    serde_json::Deserializer::from_str(s)
        .into_iter::<Value>()
        .map(|d| d.expect("a JSON document"))
        .collect()
}

/// Every ndjson line of a stream, by its `row` tag.
fn stream_rows<'a>(lines: &'a [Value], tag: &str) -> Vec<&'a Value> {
    lines.iter().filter(|l| l["row"] == tag).collect()
}

fn ndjson_lines(run: &Run) -> Vec<Value> {
    run.stdout
        .lines()
        .map(|l| serde_json::from_str(l).expect("one JSON object per line"))
        .collect()
}

/// `gen` (FJ8a) is a mock owner the deployment sees: presence lists it, its
/// descriptor carries the synthetic marker in `meta`, `watch` decodes every
/// sample through the contract the mock itself serves (§8.4) with no R6
/// discard and no rung short of a value, and `get state` reads the mock's
/// own stamp (S1). A second mock at the address is refused — the live half
/// of the guard `mock-owner.trycmd` cannot reach.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gen_brings_up_a_visible_owner_that_watch_and_get_state_read() {
    let bus = Bus::bare(None).await;
    let netif = examples().join("tcgui/tc.netif.v1.toml");
    let netif = netif.to_str().expect("a UTF-8 path").to_owned();
    let mock = bus.spawn(&[
        "gen",
        "host-a/tc",
        "tc.netif.v1",
        "--contracts",
        &netif,
        "--member",
        "bandwidth/{ns}/{iface}=default/eth0",
        "--rate",
        "20",
        "--duration",
        "12",
        "--format",
        "json",
    ]);
    wait_for(&bus, &["host-a/tc"]).await;

    let show = bus
        .until(&["service", "show", "host-a/tc", "--format", "json"], |r| {
            r.code == 0
        })
        .await;
    let doc = show.json();
    let d = &doc["rows"][0]["descriptor"]["descriptor"];
    assert_eq!(d["meta"]["synthetic"]["synthetic"], true, "{show}");
    assert_eq!(d["meta"]["synthetic"]["tool"], "zenctl gen");
    let zid = d["meta"]["zid"]
        .as_str()
        .expect("the owner's zid")
        .to_owned();

    let summary_of = |r: &Run| -> Option<Value> {
        r.stdout
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .find(|v| v["row"] == "summary")
    };
    let run = bus
        .until(
            &[
                "watch",
                "host-a/tc",
                "tc.netif.v1",
                "bandwidth/{ns}/{iface}",
                "--count",
                "5",
                "--for",
                "5",
                "--format",
                "ndjson",
            ],
            |r| r.code == 0 && summary_of(r).is_some_and(|s| s["received"] == 5),
        )
        .await;
    exits(&run, 0);
    let lines = ndjson_lines(&run);
    for s in stream_rows(&lines, "sample") {
        assert_eq!(
            s["key"],
            "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0"
        );
        assert_eq!(s["payload"]["as"], "value", "decoded, not a fallback: {s}");
        assert_eq!(s["payload"]["declared"], "json:BandwidthUpdate");
    }
    let summary = summary_of(&run).expect("a summary");
    assert_eq!(summary["discarded"], 0, "no sample on a wildcard key (R6)");
    assert_eq!(summary["lagged"], 0);

    let run = bus
        .until(
            &[
                "get",
                "state",
                "host-a/tc",
                "tc.netif.v1",
                "namespaces",
                "--format",
                "json",
            ],
            |r| r.code == 0,
        )
        .await;
    exits(&run, 0);
    let row = &run.json()["rows"][0];
    assert_eq!(row["state"], "value", "{run}");
    assert_eq!(row["payload"]["declared"], "json:Namespaces");
    assert_eq!(
        u128::from_str_radix(row["timestamp"]["clock"].as_str().expect("a clock"), 16).unwrap(),
        u128::from_str_radix(&zid, 16).unwrap(),
        "the mock's own stamp (S1)"
    );

    let again = bus
        .zenctl(&[
            "gen",
            "host-a/tc",
            "tc.netif.v1",
            "--contracts",
            &netif,
            "--duration",
            "1",
            "--format",
            "json",
        ])
        .await;
    exits(&again, 2);
    assert!(again.stderr.contains("is running already"), "{again}");
    assert!(again.stderr.contains("--i-know"), "{again}");

    let generated = mock.await.expect("the gen runner");
    exits(&generated, 0);
    let docs = documents(&generated.stdout);
    assert_eq!(docs[0]["report"], "gen-plan", "{generated}");
    assert_eq!(docs[1]["report"], "gen");
    assert!(docs[1]["sent"].as_u64() > Some(10), "{generated}");
    assert_eq!(docs[1]["failed"], 0, "{generated}");
    assert_eq!(docs[1]["refused"], 0, "{generated}");
}

/// `serve` (FJ8a) answers `call` with its fixed reply and logs the call:
/// its key, the request decoded through the bundle, the answer. A second
/// mock at the address is refused while the first serves.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn serve_answers_a_call_and_logs_it() {
    let bus = Bus::bare(None).await;
    let netif = examples().join("tcgui/tc.netif.v1.toml");
    let netif = netif.to_str().expect("a UTF-8 path").to_owned();
    let serve = bus.spawn(&[
        "serve",
        "host-a/tc",
        "tc.netif.v1",
        "diagnostics",
        r#"{"ok": true, "served_by": "mock"}"#,
        "--contracts",
        &netif,
        "--count",
        "1",
        "--for",
        "40",
        "--format",
        "ndjson",
    ]);
    wait_for(&bus, &["host-a/tc"]).await;

    let refused = bus
        .zenctl(&[
            "serve",
            "host-a/tc",
            "tc.netif.v1",
            "diagnostics",
            "--contracts",
            &netif,
            "--count",
            "1",
        ])
        .await;
    exits(&refused, 2);
    assert!(refused.stderr.contains("is running already"), "{refused}");

    let call = bus
        .until(
            &[
                "call",
                "host-a/tc",
                "tc.netif.v1",
                "diagnostics",
                r#"{"namespace": "default", "interface": "eth0"}"#,
                "--format",
                "json",
            ],
            |r| r.code == 0,
        )
        .await;
    exits(&call, 0);
    let doc = call.json();
    assert_eq!(doc["answer"], "value", "{call}");
    assert_eq!(
        doc["reply"]["value"],
        json!({"ok": true, "served_by": "mock"})
    );

    let served = serve.await.expect("the serve runner");
    exits(&served, 0);
    let lines = ndjson_lines(&served);
    let calls = stream_rows(&lines, "call");
    assert_eq!(calls.len(), 1, "{served}");
    let c = calls[0];
    assert_eq!(c["key"], "zk2/host-a/tc/tc.netif.v1/@op/diagnostics");
    assert_eq!(c["concrete"], true);
    assert_eq!(c["operation"], "@op/diagnostics");
    assert_eq!(c["request"]["as"], "value", "{c}");
    assert_eq!(c["request"]["declared"], "json:DiagnosticsRequest");
    assert_eq!(
        c["request"]["value"],
        json!({"namespace": "default", "interface": "eth0"}),
        "a request its type accepts: one it refuses is never sent (#671)"
    );
    assert_eq!(c["answer"], "reply");
    let summary = stream_rows(&lines, "summary");
    assert_eq!(summary[0]["ended"], "count");
    assert_eq!(summary[0]["calls"], 1);
}

/// `record` then `replay --namespace` (FJ8a, the tooling guide's §5): a
/// version-3 capture of an owner's stream at the bus root — its header
/// naming what the selector excludes, each row its QoS axes — republished
/// into namespace `replay` reaches a consumer there on the original
/// address, with the owner's QoS.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn record_then_replay_into_a_namespace_round_trips() {
    let mut bus = Bus::bare(None).await;
    let owner = netif_owner(&bus).await;
    let writer = owner
        .writer(
            &iface("tc.netif.v1"),
            "stream/bandwidth/{ns}/{iface}",
            &member("default", "eth0"),
        )
        .await
        .expect("a stream writer");
    bus.services.push(owner);
    bus.keep(Task(tokio::spawn(async move {
        let mut n = 0u64;
        loop {
            n += 1;
            let _ = writer.put_value(&json!({"rx_bps": n, "tx_bps": n})).await;
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })));
    wait_for(&bus, &["host-a/tc"]).await;

    let file = bus.home.join("owner.zrec");
    let file = file.to_str().expect("a UTF-8 path").to_owned();
    let run = bus
        .until(
            &[
                "record",
                "zk2/host-a/tc/tc.netif.v1/stream/**",
                "-o",
                &file,
                "--overwrite",
                "--count",
                "5",
                "--for",
                "10",
                "--format",
                "json",
            ],
            |r| r.code == 0 && r.json()["samples"] == 5,
        )
        .await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["header"]["zrec"], 3);
    assert_eq!(doc["header"]["base"], "");
    assert_eq!(
        doc["header"]["selectors"],
        json!(["zk2/host-a/tc/tc.netif.v1/stream/**"])
    );
    assert_eq!(
        doc["header"]["excluded"],
        json!(["@stream", "@state", "@op", "@zk", "@adv"])
    );
    let text = std::fs::read_to_string(&file).expect("the capture");
    let rows: Vec<Value> = text
        .lines()
        .skip(1)
        .map(|l| serde_json::from_str(l).expect("a row"))
        .collect();
    assert_eq!(rows.len(), 5, "{text}");
    for r in &rows {
        assert_eq!(
            r["key"],
            "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0"
        );
        assert!(r["bytes"].is_string(), "lossless: {r}");
        assert_eq!(
            r["qos_axes"], "data/drop/best_effort",
            "the stream's QoS (§2.4)"
        );
    }

    let in_replay = client(&bus.endpoint, Some("replay")).await;
    let heard = in_replay
        .declare_subscriber("zk2/host-a/tc/tc.netif.v1/stream/**")
        .await
        .expect("a consumer in `replay`");
    let deadline = Instant::now() + SETTLE;
    let (run, sample) = loop {
        let run = bus
            .zenctl(&["replay", &file, "--namespace", "replay", "--format", "json"])
            .await;
        let got = tokio::time::timeout(Duration::from_millis(500), heard.recv_async()).await;
        if let Ok(Ok(sample)) = got {
            break (run, sample);
        }
        assert!(Instant::now() < deadline, "nothing reached `replay`\n{run}");
    };
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["published"], 5, "{run}");
    assert_eq!(doc["refused"], 0);
    assert_eq!(doc["header"]["zrec"], 3);
    assert_eq!(
        sample.key_expr().as_str(),
        "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0"
    );
    assert_eq!(sample.priority(), zenoh::qos::Priority::Data);
    assert_eq!(
        sample.congestion_control(),
        zenoh::qos::CongestionControl::Drop
    );
}

/// `bench call` (FJ8a): a fan-out's value replies attributed by key, a
/// refusal a population of its own, a token holder that sent nothing
/// tallied; then a call to a frozen service, whose silence is counted and
/// never averaged in — exit 2, no value measured.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bench_call_attributes_replies_by_key_and_counts_silence_apart() {
    let mut bus = Bus::bare(None).await;
    tc_instance(&mut bus, "h1/tc", Tc::default()).await;
    tc_instance(
        &mut bus,
        "h2/tc",
        Tc {
            busy: true,
            ..Tc::default()
        },
    )
    .await;
    tc_instance(
        &mut bus,
        "h3/tc",
        Tc {
            frozen: true,
            ..Tc::default()
        },
    )
    .await;
    wait_for(&bus, &["h1/tc", "h2/tc", "h3/tc"]).await;
    let tc = scenario_path("tc.v1");
    let tc = tc.to_str().expect("a UTF-8 path").to_owned();

    let run = bus
        .until(
            &[
                "bench",
                "call",
                "*/tc",
                "tc.v1",
                "diagnostics",
                "--calls",
                "3",
                "--timeout",
                "2",
                "--contracts",
                &tc,
                "--format",
                "json",
            ],
            |r| r.code == 1 && r.json()["refusals"]["count"] == 3,
        )
        .await;
    exits(&run, 1);
    let doc = run.json();
    assert_eq!(doc["report"], "bench");
    assert_eq!(doc["mode"], "fanout");
    assert_eq!(doc["clock"], "round_trip");
    let repliers = doc["rows"].as_array().expect("rows");
    assert_eq!(repliers.len(), 1, "{run}");
    assert_eq!(repliers[0]["address"], "h1/tc");
    assert_eq!(repliers[0]["key"], "zk2/h1/tc/tc.v1/@op/diagnostics");
    assert_eq!(repliers[0]["replies"], 3);
    assert_eq!(doc["refusals"]["codes"], json!({"busy": 3}));
    assert_eq!(doc["silent"], 0, "every call drew a value");
    let holders: Vec<(&str, u64)> = doc["presence"]["holders"]
        .as_array()
        .expect("holders")
        .iter()
        .map(|h| {
            (
                h["address"].as_str().expect("an address"),
                h["without_value"].as_u64().expect("a count"),
            )
        })
        .collect();
    assert_eq!(holders, [("h1/tc", 0), ("h2/tc", 3), ("h3/tc", 3)]);

    let run = bus
        .zenctl(&[
            "bench",
            "call",
            "h3/tc",
            "tc.v1",
            "diagnostics",
            "--calls",
            "2",
            "--timeout",
            "1",
            "--contracts",
            &tc,
            "--format",
            "json",
        ])
        .await;
    exits(&run, 2);
    let doc = run.json();
    assert_eq!(doc["mode"], "concrete");
    assert_eq!(doc["silent"], 2, "{run}");
    assert!(doc["rows"].as_array().is_none_or(Vec::is_empty));
    assert_eq!(doc["completed"], 2);
}

/// `pub --from ndjson` (FJ8a): a row whose key a zk2 service owns is
/// refused as `pub` refuses it, counted, and never written; a foreign row is
/// written, with the QoS axes it recorded.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pub_from_ndjson_refuses_an_owned_row_and_writes_a_foreign_one() {
    let bus = Bus::bare(None).await;
    let at_root = client(&bus.endpoint, None).await;
    let foreign = at_root
        .declare_subscriber("legacy/**")
        .await
        .expect("a consumer of the foreign key");
    let owned = at_root
        .declare_subscriber("zk2/**")
        .await
        .expect("a consumer of zk2 keys");
    let rows = concat!(
        r#"{"key":"zk2/host-a/tc/tc.netif.v1/state/namespaces","value":[]}"#,
        "\n",
        r#"{"key":"legacy/temperature","value":21,"qos_axes":"interactive_high/block/reliable"}"#,
        "\n",
    );
    let deadline = Instant::now() + SETTLE;
    let (run, sample) = loop {
        let run = bus.zenctl_stdin(&["pub", "--from", "ndjson"], rows).await;
        let got = tokio::time::timeout(Duration::from_millis(500), foreign.recv_async()).await;
        if let Ok(Ok(sample)) = got {
            break (run, sample);
        }
        assert!(
            Instant::now() < deadline,
            "the foreign row never arrived\n{run}"
        );
    };
    exits(&run, 1);
    assert!(run.stderr.contains("1 refused row(s)"), "{run}");
    assert!(run.stderr.contains("(P3, spec §6)"), "{run}");
    assert_eq!(sample.key_expr().as_str(), "legacy/temperature");
    assert_eq!(sample.payload().to_bytes().as_ref(), b"21");
    assert_eq!(sample.priority(), zenoh::qos::Priority::InteractiveHigh);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        owned.try_recv().ok().flatten().is_none(),
        "the owned row was never written"
    );
}

// ── FJ8b: the raw observers and the checks over zk2 (#612) ─────────────────

/// A valid `json:BandwidthUpdate` for `eth0`, its counters at `n`, with
/// `extra` merged in at the top.
fn bandwidth(n: u64, extra: &Value) -> Value {
    let mut v = json!({
        "namespace": "default",
        "interface": "eth0",
        "stats": {"rx_bytes": n, "tx_bytes": n},
    });
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    v
}

/// The `eth0` interface as `json:NetworkInterface` declares it.
fn eth0(up: bool) -> Value {
    json!({"name": "eth0", "index": 2, "namespace": "default", "is_up": up})
}

/// [`netif_owner`] publishing: its `eth0` bandwidth every 50 ms through the
/// stream writer (unstamped: the router stamps it), and, with `state`, its
/// `eth0` interface every 500 ms through the state writer (the owner's own
/// stamp, S1). The service is returned, for a case that closes it; the
/// publishing task is too, and stops when dropped.
async fn publishing_owner(bus: &Bus, extra: Value, state: bool) -> (Service, Task) {
    let mut owner = netif_owner(bus).await;
    let netif = iface("tc.netif.v1");
    let stream = owner
        .writer(
            &netif,
            "stream/bandwidth/{ns}/{iface}",
            &member("default", "eth0"),
        )
        .await
        .expect("a stream writer");
    let iface_state = if state {
        Some(
            owner
                .state_writer(
                    &netif,
                    "state/interfaces/{ns}/{iface}",
                    &member("default", "eth0"),
                )
                .await
                .expect("a state writer"),
        )
    } else {
        None
    };
    let task = Task(tokio::spawn(async move {
        let mut n = 0u64;
        loop {
            n += 1;
            let _ = stream.put_value(&bandwidth(n, &extra)).await;
            if let Some(s) = &iface_state
                && n % 10 == 1
            {
                let _ = s.put_value(&eth0(true)).await;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }));
    (owner, task)
}

/// A principal that is not the owner, putting `value` on `key` every 50 ms
/// at `priority`: what P3 forbids, and what the raw observers must name.
async fn intruder(bus: &Bus, key: &'static str, value: &'static str, priority: Priority) -> Task {
    let principal = client(&bus.endpoint, None).await;
    Task(tokio::spawn(async move {
        loop {
            let _ = principal.put(key, value).priority(priority).await;
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }))
}

/// The tooling guide's O1 and O2, through `echo` on the raw bus: a zk2 key
/// resolved through presence and the contract its owner serves and decoded
/// as its declared type; a key that is not zk2 rendered structurally and
/// said to be (rung 2); and bytes on a zk2 key that do not decode as its
/// type, named as that type and undecodable — three rows, never folded.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn echo_decodes_zk2_and_names_foreign_and_undecodable_apart() {
    let mut bus = Bus::bare(None).await;
    let (owner, task) = publishing_owner(&bus, json!({}), false).await;
    bus.services.push(owner);
    bus.keep(task);
    bus.keep(intruder(&bus, "rt/chatter", "hello", Priority::Data).await);
    bus.keep(
        intruder(
            &bus,
            "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth1",
            "not json",
            Priority::Data,
        )
        .await,
    );
    wait_for(&bus, &["host-a/tc"]).await;

    let eth0 = "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0";
    let eth1 = "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth1";
    let run = bus
        .until(
            &["echo", "**", "--count", "60", "--format", "ndjson"],
            |r| {
                let lines: Vec<Value> = r
                    .stdout
                    .lines()
                    .filter_map(|l| serde_json::from_str(l).ok())
                    .collect();
                r.code == 0
                    && lines
                        .iter()
                        .any(|l| l["key"] == eth0 && l["verdict"] == "valid")
                    && lines.iter().any(|l| l["key"] == "rt/chatter")
                    && lines
                        .iter()
                        .any(|l| l["key"] == eth1 && l["verdict"] == "undecodable")
            },
        )
        .await;
    exits(&run, 0);
    let lines = ndjson_lines(&run);
    assert_eq!(
        lines.iter().filter(|l| l.get("key").is_some()).count(),
        60,
        "--count stops it after exactly that many samples: {run}"
    );

    let zk2 = lines
        .iter()
        .find(|l| l["key"] == eth0 && l["verdict"] == "valid")
        .expect("a decoded row");
    assert_eq!(zk2["identity"]["is"], "resource", "{zk2}");
    assert_eq!(zk2["identity"]["address"], "host-a/tc");
    assert_eq!(zk2["identity"]["resource"], "stream/bandwidth/{ns}/{iface}");
    assert_eq!(
        zk2["identity"]["values"],
        json!({"iface": ["eth0"], "ns": ["default"]})
    );
    assert_eq!(zk2["type"], "json:BandwidthUpdate");
    assert_eq!(zk2["typed"], true);
    assert_eq!(zk2["value"]["interface"], "eth0");
    assert_eq!(zk2["qos_axes"], "data/drop/best_effort");

    let foreign = lines
        .iter()
        .find(|l| l["key"] == "rt/chatter")
        .expect("a foreign row");
    assert_eq!(foreign["identity"]["is"], "not_zk2", "{foreign}");
    assert_eq!(foreign["identity"]["unresolved"]["reason"], "not_zk2");
    assert!(
        foreign.get("type").is_none(),
        "no type is claimed: {foreign}"
    );
    assert_eq!(foreign["typed"], false);
    assert_eq!(foreign["value"], "hello", "rendered structurally");
    assert!(
        foreign["verdict"]
            .as_str()
            .is_some_and(|v| v.starts_with("not-checked:")),
        "not checked is not valid: {foreign}"
    );

    let bad = lines
        .iter()
        .find(|l| l["key"] == eth1 && l["verdict"] == "undecodable")
        .expect("an undecodable row");
    assert_eq!(bad["identity"]["resource"], "stream/bandwidth/{ns}/{iface}");
    assert_eq!(bad["type"], "json:BandwidthUpdate", "named as its type");
    assert_eq!(bad["typed"], false);
    assert!(bad["decode_error"].is_string(), "{bad}");
    assert_eq!(bad["value"], "not json");
}

/// `rate` and `field` over the same raw window: rate groups its keys by zk2
/// address and resource, a key that is not zk2 its own group; field judges
/// each path against the declared type, a path the type never declares
/// drift (field-new), and the declared ones not.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rate_groups_by_resource_and_field_judges_paths_against_the_type() {
    let mut bus = Bus::bare(None).await;
    let (owner, task) = publishing_owner(&bus, json!({"burst": 1}), false).await;
    bus.services.push(owner);
    bus.keep(task);
    bus.keep(intruder(&bus, "rt/chatter", "hello", Priority::Data).await);
    wait_for(&bus, &["host-a/tc"]).await;

    let resource = |g: &&Value| g["is"] == "resource" && g["address"] == "host-a/tc";
    let run = bus
        .until(&["rate", "**", "--for", "2", "--format", "json"], |r| {
            r.code == 0
                && serde_json::from_str::<Value>(&r.stdout).is_ok_and(|d| {
                    let groups = rows_of(&d, "group");
                    groups
                        .iter()
                        .any(|g| resource(g) && g.get("resource").is_some())
                        && groups.iter().any(|g| g["is"] == "not_zk2")
                })
        })
        .await;
    exits(&run, 0);
    let doc = run.json();
    let groups = rows_of(&doc, "group");
    let stream = *groups
        .iter()
        .find(|g| resource(g))
        .expect("the owner's group");
    assert_eq!(stream["iface"], "tc.netif.v1");
    assert_eq!(stream["resource"], "stream/bandwidth/{ns}/{iface}");
    assert_eq!(stream["keys"], 1);
    assert!(stream["count"].as_u64() > Some(10), "{stream}");
    let foreign = *groups
        .iter()
        .find(|g| g["is"] == "not_zk2")
        .expect("a group");
    assert_eq!(foreign["keys"], 1);
    assert_eq!(doc["lens"]["presence"]["complete"], true, "{run}");

    let run = bus
        .until(
            &[
                "field",
                "zk2/host-a/tc/tc.netif.v1/stream/**",
                "--for",
                "2",
                "--format",
                "json",
            ],
            |r| r.code == 0 && r.stdout.contains("field-new"),
        )
        .await;
    exits(&run, 0);
    let doc = run.json();
    let path = |p: &str| {
        rows_of(&doc, "path")
            .into_iter()
            .find(|r| r["path"] == p)
            .unwrap_or_else(|| panic!("{p}: {doc}"))
            .clone()
    };
    assert_eq!(path("stats.rx_bytes")["declared"], true);
    assert_eq!(path("interface")["declared"], true);
    assert_eq!(path("burst")["declared"], false);
    let findings = rows_of(&doc, "finding");
    assert!(
        findings.iter().any(|f| f["check"] == "field-new"
            && f["subject"]
                .as_str()
                .is_some_and(|s| s.ends_with("· burst"))),
        "{doc}"
    );
    assert!(
        findings
            .iter()
            .all(|f| !f["subject"].as_str().unwrap_or("").ends_with("· interface")),
        "a declared path is not drift: {doc}"
    );
    assert!(
        doc["notes"]
            .to_string()
            .contains("field-stuck was not asked"),
        "{run}"
    );
}

/// The timeline over a live window and over its `.zrec`: the lanes are zk2
/// resources either way, and the clocks are named — live, a state put is
/// stamped by its owner (S1, the zid its descriptor states) and a stream
/// sample by the router (another clock); from the file, which names no
/// owner, the same lanes with every stamp unattributable (O7), never
/// guessed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn timeline_names_its_clocks_live_and_reads_its_zrec_through_the_same_lanes() {
    let mut bus = Bus::bare(None).await;
    let (owner, task) = publishing_owner(&bus, json!({}), true).await;
    bus.services.push(owner);
    bus.keep(task);
    wait_for(&bus, &["host-a/tc"]).await;

    let lane = |doc: &Value, resource: &str| -> Value {
        doc["lanes"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|l| l["lane"]["kind"] == "resource" && l["lane"]["resource"] == resource)
            .cloned()
            .unwrap_or(Value::Null)
    };
    let state = "state/interfaces/{ns}/{iface}";
    let stream = "stream/bandwidth/{ns}/{iface}";
    let run = bus
        .until(
            &[
                "timeline",
                "zk2/host-a/tc/tc.netif.v1/**",
                "--for",
                "2",
                "--format",
                "json",
            ],
            |r| {
                r.code == 0
                    && serde_json::from_str::<Value>(&r.stdout).is_ok_and(|d| {
                        lane(&d, state)["provenance"]["owner"].as_u64() > Some(0)
                            && lane(&d, stream)["samples"].as_u64() > Some(0)
                    })
            },
        )
        .await;
    exits(&run, 0);
    let live = run.json();
    assert_eq!(live["source"]["kind"], "live");
    assert_eq!(live["lens"]["presence"]["complete"], true);
    let owned = lane(&live, state);
    assert_eq!(owned["lane"]["address"], "host-a/tc");
    assert_eq!(owned["provenance"]["other"], 0, "{owned}");
    assert_eq!(
        owned["stampers"],
        json!([bus.owners.zid().to_string()]),
        "the owner's own clock (S1)"
    );
    let routed = lane(&live, stream);
    assert_eq!(routed["provenance"]["owner"], 0, "{routed}");
    assert!(
        routed["provenance"]["other"].as_u64() > Some(0),
        "an unstamped put is the router's clock, not the owner's: {routed}"
    );

    let file = bus.home.join("window.zrec");
    let file = file.to_str().expect("a UTF-8 path").to_owned();
    let run = bus
        .until(
            &[
                "record",
                "zk2/host-a/tc/tc.netif.v1/**",
                "-o",
                &file,
                "--overwrite",
                "--for",
                "2",
                "--format",
                "json",
            ],
            |r| {
                r.code == 0
                    && std::fs::read_to_string(&file)
                        .is_ok_and(|t| t.contains("/state/interfaces/") && t.contains("/stream/"))
            },
        )
        .await;
    exits(&run, 0);
    let netif = examples().join("tcgui/tc.netif.v1.toml");
    let run = bus
        .zenctl(&[
            "timeline",
            "--from",
            &file,
            "--contracts",
            netif.to_str().expect("a UTF-8 path"),
            "--format",
            "json",
        ])
        .await;
    exits(&run, 0);
    let replayed = run.json();
    assert_eq!(replayed["source"]["kind"], "zrec", "{run}");
    assert!(replayed.get("window_s").is_none(), "nothing was asked (O4)");
    for resource in [state, stream] {
        let l = lane(&replayed, resource);
        assert_eq!(l["lane"], lane(&live, resource)["lane"], "one projection");
        assert_eq!(l["provenance"]["owner"], 0, "a file names no owner: {l}");
        assert_eq!(l["provenance"]["other"], 0, "{l}");
        assert!(l["provenance"]["unattributable"].as_u64() > Some(0), "{l}");
    }
}

/// `snapshot` takes the owner's state through S4 — every row answered by
/// the owner, on its own stamp, its payload valid against its type — and
/// `snapshot diff` compares two takes by zk2 key: a changed value and an
/// added member are the finding (1), a take against itself is clean (0).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn snapshot_takes_the_owners_state_and_diff_compares_by_zk2_key() {
    let mut bus = Bus::bare(None).await;
    let mut owner = netif_owner(&bus).await;
    let netif = iface("tc.netif.v1");
    let mut writers = Vec::new();
    for name in ["eth0", "eth1", "eth2"] {
        writers.push(
            owner
                .state_writer(
                    &netif,
                    "state/interfaces/{ns}/{iface}",
                    &member("default", name),
                )
                .await
                .expect("a state writer"),
        );
    }
    let iface_of = |name: &str, up: bool| json!({"name": name, "index": 2, "namespace": "default", "is_up": up});
    writers[0]
        .put_value(&iface_of("eth0", true))
        .await
        .expect("put");
    writers[1]
        .put_value(&iface_of("eth1", true))
        .await
        .expect("put");
    bus.services.push(owner);
    wait_for(&bus, &["host-a/tc"]).await;

    let take = |name: &str| bus.home.join(name).to_str().expect("UTF-8").to_owned();
    let (a, b) = (take("a.zsnap"), take("b.zsnap"));
    let snap = |out: &str| {
        vec![
            "snapshot".to_owned(),
            "-o".to_owned(),
            out.to_owned(),
            "--overwrite".to_owned(),
            "--format".to_owned(),
            "json".to_owned(),
        ]
    };
    let args = snap(&a);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let run = bus
        .until(&args, |r| r.code == 0 && r.json()["live"] == 2)
        .await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["no_instance"], 0, "{run}");
    assert_eq!(doc["nonconforming"], 0);
    assert_eq!(doc["header"]["zsnap"], 2);
    assert_eq!(doc["header"]["presence"]["complete"], true);
    let text = std::fs::read_to_string(&a).expect("the file");
    let rows: Vec<Value> = text
        .lines()
        .skip(1)
        .map(|l| serde_json::from_str(l).expect("a row"))
        .collect();
    assert_eq!(rows.len(), 2, "{text}");
    for r in &rows {
        assert_eq!(
            r["holder"],
            json!({"kind": "live", "address": "host-a/tc", "answered_by": "owner"}),
            "{r}"
        );
        assert_eq!(r["stamper"]["kind"], "owner", "S1: {r}");
        assert_eq!(r["conformance"]["state"], "valid");
        assert_eq!(r["identity"]["resource"], "state/interfaces/{ns}/{iface}");
    }

    writers[0]
        .put_value(&iface_of("eth0", false))
        .await
        .expect("put");
    writers[2]
        .put_value(&iface_of("eth2", true))
        .await
        .expect("put");
    let args = snap(&b);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let run = bus
        .until(&args, |r| r.code == 0 && r.json()["live"] == 3)
        .await;
    exits(&run, 0);

    let run = bus
        .offline(&["snapshot", "diff", &a, &b, "--format", "json"])
        .await;
    exits(&run, 1);
    let doc = run.json();
    assert_eq!(
        rows_of(&doc, "added")
            .iter()
            .map(|r| r["key"].clone())
            .collect::<Vec<_>>(),
        [json!(
            "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth2"
        )],
        "{run}"
    );
    let changed = rows_of(&doc, "changed");
    assert_eq!(changed.len(), 1, "{run}");
    assert_eq!(
        changed[0]["key"],
        "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0"
    );
    assert_eq!(changed[0]["value"]["changes"][0]["path"], "is_up");
    assert!(
        changed[0].get("holder").is_none(),
        "the holder did not move"
    );
    let run = bus
        .offline(&["snapshot", "diff", &a, &a, "--format", "json"])
        .await;
    exits(&run, 0);
    drop(writers);
}

/// `check expect` over a zk2 resource, at each exit: samples, a rate, the
/// payloads against their type, the declared QoS and presence all met (0);
/// a rate ceiling the stream exceeds, and then payloads a principal puts
/// that do not decode as the type, not met (1); a resource the contract
/// does not declare, a question that cannot be asked (2).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn check_expect_judges_samples_values_qos_and_presence() {
    let mut bus = Bus::bare(None).await;
    let (owner, task) = publishing_owner(&bus, json!({}), false).await;
    bus.services.push(owner);
    bus.keep(task);
    wait_for(&bus, &["host-a/tc"]).await;

    let expect = |extra: &[&'static str]| {
        let mut a = vec![
            "check",
            "expect",
            "*/tc",
            "tc.netif.v1",
            "bandwidth/{ns}/{iface}",
            "--for",
            "1",
            "--format",
            "json",
        ];
        a.extend_from_slice(extra);
        a
    };
    let run = bus
        .until(
            &expect(&[
                "--at-least",
                "5",
                "--valid-payload",
                "--qos",
                "declared",
                "--present",
            ]),
            |r| r.code == 0,
        )
        .await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["verdict"], "met", "{run}");
    // Universal claims read the window whole (#801): met at the fifth
    // sample, it still ran its second.
    assert_eq!(doc["ended_early"], false, "{run}");
    assert_eq!(doc["window_s"], 1.0, "{run}");
    assert_eq!(doc["presence"]["holders"], json!(["host-a/tc"]));
    assert_eq!(doc["presence"]["complete"], true);
    assert_eq!(
        doc["selectors"],
        json!(["zk2/*/tc/tc.netif.v1/stream/bandwidth/*/*"])
    );
    assert_eq!(doc["violations_total"], 0);

    let run = bus
        .until(&expect(&["--rate-max", "1"]), |r| r.code == 1)
        .await;
    exits(&run, 1);
    let doc = run.json();
    assert_eq!(doc["verdict"], "not_met", "{run}");
    assert!(doc["unmet"].to_string().contains("Hz"), "{run}");

    bus.keep(
        intruder(
            &bus,
            "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth1",
            "not json",
            Priority::Data,
        )
        .await,
    );
    // `--valid-payload` holds the window open its whole second, so the
    // intruder's samples are read whatever arrives first. Before #801 early
    // success ended it at the first valid sample, which under load was the
    // owner's every time (#781).
    let run = bus
        .until(&expect(&["--valid-payload"]), |r| r.code == 1)
        .await;
    exits(&run, 1);
    let doc = run.json();
    assert_eq!(doc["verdict"], "not_met", "{run}");
    assert_eq!(doc["ended_early"], false, "{run}");
    assert!(
        doc["violations"].to_string().contains("/default/eth1"),
        "the violation names its key: {run}"
    );

    let run = bus
        .zenctl(&[
            "check",
            "expect",
            "*/tc",
            "tc.netif.v1",
            "nope",
            "--for",
            "1",
        ])
        .await;
    exits(&run, 2);
    assert!(
        run.stderr.contains("the question could not be asked"),
        "{run}"
    );
}

/// `check schema` with no `--contracts`: the type is retrieved from the
/// revision's holders (§8.4), and the three exits are three facts — the
/// payload conforms (0), it does not (1), the check never happened (2).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn check_schema_retrieves_the_type_from_its_holders() {
    let mut bus = Bus::bare(None).await;
    bus.services.push(netif_owner(&bus).await);
    wait_for(&bus, &["host-a/tc"]).await;

    let check = |payload: &'static str, member: &'static str| {
        vec![
            "check",
            "schema",
            "tc.netif.v1",
            "interfaces/{ns}/{iface}",
            "--member",
            member,
            "--from",
            payload,
            "--format",
            "json",
        ]
    };
    let valid = r#"{"name":"eth0","index":2,"namespace":"default","is_up":true}"#;
    let run = bus.until(&check(valid, "type"), |r| r.code == 0).await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["conformance"]["state"], "valid", "{run}");
    assert_eq!(doc["declared"], "json:NetworkInterface");
    assert_eq!(doc["fingerprint"], fingerprint("tcgui/tc.netif.v1"));

    let invalid = r#"{"name":"eth0","index":2,"namespace":"default","is_up":"yes"}"#;
    let run = bus.zenctl(&check(invalid, "type")).await;
    exits(&run, 1);
    let doc = run.json();
    assert_eq!(doc["conformance"]["state"], "invalid", "{run}");
    assert!(
        doc["conformance"]["violations"][0]
            .as_str()
            .is_some_and(|v| v.starts_with("/is_up"))
    );

    let run = bus.zenctl(&check(valid, "request")).await;
    exits(&run, 2);
    assert!(run.stderr.contains("not checked"), "{run}");
}

/// `check probe`, consumer-shaped: a value of the stream arrives and
/// conforms (0); a state resource nobody puts, its owner up and holding the
/// token, is up-and-silent — the finding (1); a resource the contract does
/// not declare cannot be probed (2).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn check_probe_hears_a_value_or_attributes_its_silence() {
    let mut bus = Bus::bare(None).await;
    let (owner, task) = publishing_owner(&bus, json!({}), false).await;
    bus.services.push(owner);
    bus.keep(task);
    wait_for(&bus, &["host-a/tc"]).await;

    let probe = |resource: &'static str| {
        vec![
            "check",
            "probe",
            "host-a/tc",
            "tc.netif.v1",
            resource,
            "--for",
            "2",
            "--format",
            "json",
        ]
    };
    let run = bus
        .until(&probe("bandwidth/{ns}/{iface}"), |r| r.code == 0)
        .await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["verdict"]["answer"], "not_established", "{run}");
    assert!(doc["conforming"].as_u64() > Some(0), "{run}");
    assert!(
        doc.get("presence").is_none(),
        "a value needs no attribution"
    );
    assert!(
        doc.get("current").is_none(),
        "a stream has no state to read"
    );
    assert_eq!(
        doc["first"]["key"],
        "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0"
    );

    let run = bus.until(&probe("namespaces"), |r| r.code == 1).await;
    exits(&run, 1);
    let doc = run.json();
    assert_eq!(doc["verdict"]["answer"], "established", "{run}");
    assert_eq!(doc["received"], 0);
    assert_eq!(doc["current"]["answered"], 0, "S4 read first: {run}");
    assert_eq!(doc["presence"]["holders"], json!(["host-a/tc"]));
    assert_eq!(doc["presence"]["complete"], true);

    let run = bus.zenctl(&probe("nope")).await;
    exits(&run, 2);
}

/// `watch` (#671): a sample on the owner's key that rode another QoS than
/// its resource declares (§2.4) is named with the axis that differs, and
/// counted; a sample on a key that resolves to no member of the resource
/// (`X` is no canonical slug, §1.4) is counted apart, never delivered.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn watch_names_a_qos_mismatch_and_counts_unresolved_samples_apart() {
    let mut bus = Bus::bare(None).await;
    let (owner, task) = publishing_owner(&bus, json!({}), false).await;
    bus.services.push(owner);
    bus.keep(task);
    bus.keep(
        intruder(
            &bus,
            "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth2",
            r#"{"stats":{}}"#,
            Priority::RealTime,
        )
        .await,
    );
    bus.keep(
        intruder(
            &bus,
            "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/X",
            r#"{"stats":{}}"#,
            Priority::Data,
        )
        .await,
    );
    wait_for(&bus, &["host-a/tc"]).await;

    let summary_of = |r: &Run| -> Option<Value> {
        r.stdout
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .find(|v| v["row"] == "summary")
    };
    let run = bus
        .until(
            &[
                "watch",
                "host-a/tc",
                "tc.netif.v1",
                "bandwidth/{ns}/{iface}",
                "--for",
                "2",
                "--format",
                "ndjson",
            ],
            |r| {
                r.code == 0
                    && summary_of(r).is_some_and(|s| {
                        s["qos_mismatched"].as_u64() > Some(0) && s["unresolved"].as_u64() > Some(0)
                    })
            },
        )
        .await;
    exits(&run, 0);
    let lines = ndjson_lines(&run);
    let samples = stream_rows(&lines, "sample");
    let off: Vec<&&Value> = samples
        .iter()
        .filter(|s| s["key"] == "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth2")
        .collect();
    assert!(!off.is_empty(), "{run}");
    for s in &off {
        assert_eq!(s["qos_mismatch"]["differs"], json!(["priority"]), "{s}");
        assert_eq!(s["qos_mismatch"]["declared"]["priority"], "data");
        assert_eq!(s["qos_mismatch"]["observed"]["priority"], "real_time");
    }
    assert!(
        samples
            .iter()
            .filter(|s| s["key"] == "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0")
            .all(|s| s.get("qos_mismatch").is_none()),
        "the owner rode its declared QoS: {run}"
    );
    assert!(
        samples
            .iter()
            .all(|s| s["key"].as_str().is_some_and(|k| !k.ends_with("/X"))),
        "an unresolved sample is never delivered: {run}"
    );
    let summary = summary_of(&run).expect("a summary");
    assert_eq!(summary["qos_mismatched"], off.len() as u64);
}

/// `call` (#671): a request that does not satisfy its JSON Schema type is
/// refused with every violation before anything is sent — exit 2, and the
/// service's handler never ran.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn call_refuses_a_request_its_type_refuses_and_sends_nothing() {
    let mut bus = Bus::bare(None).await;
    let seen = tc_instance(&mut bus, "h1/tc", Tc::default()).await;
    wait_for(&bus, &["h1/tc"]).await;

    let run = bus
        .zenctl(&[
            "call",
            "h1/tc",
            "tc.v1",
            "interfaces/{if}/set",
            r#"{"up":"yes","why":1}"#,
            "--param",
            "if=eth0",
        ])
        .await;
    exits(&run, 2);
    assert!(
        run.stderr.contains("does not satisfy json:SetRequest"),
        "{run}"
    );
    assert!(run.stderr.contains("nothing was sent"), "{run}");
    assert_eq!(seen.set.load(Ordering::SeqCst), 0, "no handler ran");

    let run = bus
        .until(
            &[
                "call",
                "h1/tc",
                "tc.v1",
                "interfaces/{if}/set",
                r#"{"up":true}"#,
                "--param",
                "if=eth0",
            ],
            |r| r.code == 0,
        )
        .await;
    exits(&run, 0);
    assert_eq!(seen.set.load(Ordering::SeqCst), 1, "the valid one ran");
}

/// The watchdog's vocabulary over zk2 keys, each rule firing and clearing
/// through one bounded run: a principal's undecodable payload and its
/// off-QoS sample fire `invalid-payload` and `qos-mismatch`, which clear
/// when it stops; the owner closing fires `instance-gone`, `rate-below` and
/// `silent-for`, which clear when it comes back. `rate-above` and
/// `dropped` hold `ok` throughout, and a bounded run that ended healthy
/// exits 0.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn watchdog_rules_fire_and_clear_over_zk2_keys() {
    let mut bus = Bus::bare(None).await;
    let (owner, task) = publishing_owner(&bus, json!({}), false).await;
    wait_for(&bus, &["host-a/tc"]).await;

    let sel = "zk2/host-a/tc/tc.netif.v1/stream/**";
    let rule = |s: &str| s.replace("SEL", sel);
    let rules = [
        rule("rate-below SEL 5"),
        rule("rate-above SEL 100000"),
        rule("silent-for SEL 2"),
        rule("invalid-payload SEL"),
        rule("qos-mismatch SEL"),
        "instance-gone host-a/tc".to_owned(),
        "dropped".to_owned(),
    ];
    let mut args = vec!["watchdog"];
    for r in &rules {
        args.extend(["--rule", r.as_str()]);
    }
    args.extend(["--every", "1", "--count", "24", "--timeout", "1"]);
    let watchdog = bus.spawn(&args);

    // Healthy, then a principal misbehaving on the owner's keys.
    tokio::time::sleep(Duration::from_secs(4)).await;
    let bad = (
        intruder(
            &bus,
            "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth1",
            "not json",
            Priority::Data,
        )
        .await,
        intruder(
            &bus,
            "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth2",
            r#"{"stats":{}}"#,
            Priority::RealTime,
        )
        .await,
    );
    tokio::time::sleep(Duration::from_secs(4)).await;
    drop(bad);
    // Healthy again, then the owner gone, then back.
    tokio::time::sleep(Duration::from_secs(4)).await;
    drop(task);
    owner.close().await.expect("the owner closes");
    tokio::time::sleep(Duration::from_secs(5)).await;
    let (back, task) = publishing_owner(&bus, json!({}), false).await;
    bus.services.push(back);
    bus.keep(task);

    let run = watchdog.await.expect("the watchdog ran");
    let lines = ndjson_lines(&run);
    let transitions = stream_rows(&lines, "transition");
    let path = |name: &str| -> Vec<String> {
        transitions
            .iter()
            .filter(|t| t["rule"].as_str().is_some_and(|r| r.starts_with(name)))
            .map(|t| t["to"].as_str().unwrap_or("?").to_owned())
            .collect()
    };
    let fired_and_cleared = |name: &str| {
        let p = path(name);
        let fired = p.iter().position(|s| s == "firing");
        assert!(fired.is_some(), "{name} never fired: {p:?}\n{run}");
        assert!(
            p[fired.unwrap_or(0)..].iter().any(|s| s == "ok"),
            "{name} never cleared: {p:?}\n{run}"
        );
    };
    for name in [
        "invalid-payload",
        "qos-mismatch",
        "instance-gone",
        "rate-below",
        "silent-for",
    ] {
        fired_and_cleared(name);
    }
    for name in ["rate-above", "dropped"] {
        assert_eq!(path(name), ["ok"], "{name}\n{run}");
    }
    assert!(
        transitions
            .iter()
            .filter(|t| t["rule"]
                .as_str()
                .is_some_and(|r| r.starts_with("invalid-payload")))
            .any(|t| t["to"] == "firing" && t["evidence"].to_string().contains("/default/eth1")),
        "the evidence names the key: {run}"
    );
    exits(&run, 0);
}

/// A base-relative zk2 selector under `--namespace` is a subscription to
/// nothing — the raw observers take wire keys — so stderr says once which
/// wire key was meant (#512, re-cut for zk2 in FJ8b); stdout stays the
/// document, and the wire form is not hinted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_base_relative_zk2_selector_is_hinted_on_stderr() {
    let mut bus = Bus::bare(Some("prod")).await;
    let (owner, task) = publishing_owner(&bus, json!({}), false).await;
    bus.services.push(owner);
    bus.keep(task);
    let hint = r#"hint: "zk2/**" does not sit under namespace "prod" — selectors are wire keys; did you mean "prod/zk2/**"?"#;

    let run = bus
        .zenctl(&[
            "rate",
            "zk2/**",
            "--for",
            "1",
            "--namespace",
            "prod",
            "--format",
            "json",
        ])
        .await;
    exits(&run, 0);
    assert_eq!(
        run.json()["total_count"],
        0,
        "nothing at the bus root\n{run}"
    );
    assert_eq!(
        run.stderr.matches(hint).count(),
        1,
        "exactly one hint, on stderr\n{run}"
    );
    assert!(!run.stdout.contains("hint:"), "{run}");

    let run = bus
        .until(
            &[
                "rate",
                "prod/zk2/**",
                "--for",
                "1",
                "--namespace",
                "prod",
                "--format",
                "json",
            ],
            |r| r.code == 0 && r.json()["total_count"].as_u64() > Some(0),
        )
        .await;
    exits(&run, 0);
    assert!(!run.stderr.contains("hint:"), "{run}");
    assert!(
        rows_of(&run.json(), "group")
            .iter()
            .any(|g| g["is"] == "resource" && g["address"] == "host-a/tc"),
        "the wire key under the namespace resolves: {run}"
    );
}

// ── FK1: why, check conform, storage gen, admin graph (#702–#705) ──────────

/// The rows of a `--format json` document tagged `instance`, by address.
fn instance_row<'a>(doc: &'a Value, address: &str) -> Option<&'a Value> {
    rows_of(doc, "instance")
        .into_iter()
        .find(|r| r["address"] == address)
}

/// #705: `admin graph` joins each instance onto the router whose verified
/// document lists its session (spec §3.3, §4.2). `host-a/tc` is a client of
/// the router zenctl reads, whose admin space is on: attached, listed as a
/// client, its zid the one its descriptor states. `host-b/tc` is a client
/// of a second router, linked to the first, whose admin space is off: the
/// first lists that router and not its clients, so `host-b/tc` is reported
/// unattached — never omitted — and its router is only heard of.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admin_graph_attaches_an_instance_to_its_router_and_reports_the_unattached() {
    let mut bus = Bus::admin(Some("acme")).await;
    let (far, far_endpoint) = router_with(false, Some(&bus.endpoint)).await;
    let far_owners = client(&far_endpoint, Some("acme")).await;
    let addr = |s: &str| s.parse().expect("an address");
    let near = bring_up(
        &bus.owners,
        ServiceConfig::new(addr("host-a/tc")),
        &["tcgui/tc.netif.v1"],
    )
    .await;
    let remote = bring_up(
        &far_owners,
        ServiceConfig::new(addr("host-b/tc")),
        &["tcgui/tc.netif.v1"],
    )
    .await;
    bus.services.extend([near, remote]);
    let far_zid = far.zid().to_string();
    bus.keep((far, far_owners.clone()));

    let args = [
        "admin",
        "graph",
        "--namespace",
        "acme",
        "--timeout",
        "2",
        "--format",
        "json",
    ];
    let run = bus
        .until(&args, |r| {
            r.code == 0
                && serde_json::from_str::<Value>(&r.stdout).is_ok_and(|d| {
                    ["host-a/tc", "host-b/tc"].iter().all(|a| {
                        instance_row(&d, a).is_some_and(|i| i["attachment"] != "unattributable")
                    })
                })
        })
        .await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["report"], "admin-graph");
    assert_eq!(doc["instances"]["namespace"], "acme", "{run}");
    let router_zid = bus._router.zid().to_string();
    assert_eq!(
        doc["instances"]["verified"],
        json!([router_zid]),
        "the router zenctl is connected to, verified by its own answer: {run}"
    );
    let a = instance_row(&doc, "host-a/tc").expect("host-a/tc");
    assert_eq!(a["attachment"], "attached", "{run}");
    assert_eq!(a["zid"], bus.owners.zid().to_string(), "its meta.zid");
    assert_eq!(a["routers"][0]["router"], router_zid);
    assert_eq!(a["routers"][0]["listed_as"], "client");
    let b = instance_row(&doc, "host-b/tc").expect("host-b/tc is never omitted");
    assert_eq!(b["attachment"], "unattached", "{run}");
    assert_eq!(b["zid"], far_owners.zid().to_string());
    assert!(
        b["reason"]
            .as_str()
            .is_some_and(|r| r.contains("no verified router")),
        "{run}"
    );
    assert!(
        rows_of(&doc, "node")
            .iter()
            .any(|n| n["zid"] == far_zid.as_str() && n["answered"] == false),
        "the far router is heard of, not queryable: {run}"
    );

    // The picture carries the join too, and its notes ride stderr.
    let run = bus
        .zenctl(&[
            "admin",
            "graph",
            "--namespace",
            "acme",
            "--timeout",
            "2",
            "--dot",
        ])
        .await;
    exits(&run, 0);
    assert!(
        run.stdout
            .contains(&format!("\"{router_zid}\" -- \"host-a/tc@")),
        "{run}"
    );
    assert!(run.stdout.contains("(unattached)"), "{run}");
    assert!(run.stderr.contains("1 attached, 1 unattached"), "{run}");
}

/// #704: the union storage `storage gen` derives from the tcgui enrollment
/// takes the occurrences a real owner publishes (spec §2.6). A `tc.netem.v1`
/// owner puts one audit event through the runtime's event writer, a raw
/// subscriber on the planned key expression receives it, and `--explain`
/// names the derived storage for its key. Checked against a live router
/// whose admin space answers and runs no storage manager, the plan has
/// nothing to be compared with: exit 2, never clean.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn storage_gen_derives_a_union_storage_that_takes_an_owners_events() {
    let mut bus = Bus::admin(None).await;
    let enrollment = examples().join("acl/tcgui.enrollment.toml");
    let history = examples().join(".history");
    let (enrollment, history) = (
        enrollment.to_str().expect("a UTF-8 path").to_owned(),
        history.to_str().expect("a UTF-8 path").to_owned(),
    );
    let plan_args = [
        "storage",
        "gen",
        "--enrollment",
        &enrollment,
        "--contracts",
        &history,
        "--format",
        "json",
    ];
    let run = bus.offline(&plan_args).await;
    exits(&run, 0);
    let doc = run.json();
    let storage = rows_of(&doc, "storage")
        .into_iter()
        .find(|s| s["name"] == "events-tc.netem.v1-applied")
        .unwrap_or_else(|| panic!("the audit event's union storage: {run}"));
    assert_eq!(storage["garbage_collection"]["lifespan_s"], 604_800);
    assert_eq!(storage["derived"]["retention_s"], 604_800);
    let key_expr = storage["key_expr"]
        .as_str()
        .expect("a key expression")
        .to_owned();
    assert!(
        rows_of(&doc, "storage").iter().all(|s| {
            let k = s["key_expr"].as_str().unwrap_or_default();
            !k.contains("/state/") && !k.contains("/@state/")
        }),
        "nothing on an owner's state (S4): {run}"
    );

    // An owner's occurrence lands under the planned key expression.
    let owner = bring_up(
        &bus.owners,
        ServiceConfig::new("h-3fa9c2d41b7e/tc".parse().expect("an address")),
        &["tcgui/tc.netem.v1"],
    )
    .await;
    let tool = client(&bus.endpoint, None).await;
    let sub = tool
        .declare_subscriber(key_expr.as_str())
        .await
        .expect("a subscriber on the planned key expression");
    let events = owner
        .event_writer(&iface("tc.netem.v1"), "events/applied", &Bindings::new())
        .expect("an event writer");
    // One occurrence only: the event's rate is `low`, which the runtime
    // keeps (§2.7, #741), so the subscriber is matched first, through a
    // probe publisher on the event's keys, rather than put until heard.
    let probe = bus
        .owners
        .declare_publisher(
            "zk2/h-3fa9c2d41b7e/tc/tc.netem.v1/events/applied/01hzzzzzzzzzzzzzzzzzzzzzzz",
        )
        .await
        .expect("a probe publisher");
    let deadline = Instant::now() + SETTLE;
    while !probe.matching_status().await.expect("matching").matching() {
        assert!(Instant::now() < deadline, "the subscriber never matched");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let put = events
        .put(br#"{"ulid":"x"}"#.to_vec())
        .await
        .expect("an occurrence");
    let key = match tokio::time::timeout(SETTLE, sub.recv_async()).await {
        Ok(Ok(sample)) => {
            assert_eq!(sample.key_expr().as_str(), put);
            put
        }
        _ => panic!("no occurrence reached {key_expr}"),
    };
    bus.services.push(owner);

    let mut explain = plan_args.to_vec();
    explain.extend(["--explain", &key]);
    let run = bus.offline(&explain).await;
    exits(&run, 0);
    let doc = run.json();
    let taker = &rows_of(&doc, "taker")[0];
    assert_eq!(taker["storage"], "events-tc.netem.v1-applied", "{run}");
    assert_eq!(taker["relation"], "includes");

    // The admin space answers, and no storage runs: nothing to compare.
    let mut check = plan_args.to_vec();
    check.extend(["--check", "--timeout", "2"]);
    let run = bus.zenctl(&check).await;
    exits(&run, 2);
    let doc = run.json();
    assert_eq!(doc["source"], "admin_space");
    assert_eq!(doc["judgement"]["answer"], "unobservable", "{run}");
}

/// One rung's row in a `why --format json` document.
fn rung<'a>(doc: &'a Value, id: &str) -> &'a Value {
    rows_of(doc, "rung")
        .into_iter()
        .find(|r| r["rung"] == id)
        .unwrap_or_else(|| panic!("{id} has a row: {doc}"))
}

/// #702: `why` over a live tcgui owner in a namespace, through the runtime
/// (state.md §1's owner, the FJ8b publishing owner). Healthy — the owner
/// answers its state GET with its own stamp (S1), and its stream within the
/// window — exit 0; a service judged up to its contracts, exit 0. A cause
/// at each live rung, exit 1: no token visible (presence), an interface
/// the descriptor does not list (descriptor), a resource the revision does
/// not declare (contract). And the owner's silence on a member it never
/// wrote, exit 2: unobservable, the archives asked after it (S6).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn why_explains_a_silence_rung_by_rung_over_a_live_owner() {
    let mut bus = Bus::bare(Some("acme")).await;
    let (owner, task) = publishing_owner(&bus, json!({}), true).await;
    bus.services.push(owner);
    bus.keep(task);
    wait_for_ns(&bus, "acme", &["host-a/tc"]).await;
    let why = |target: &str| -> Vec<String> {
        [
            "why",
            target,
            "--namespace",
            "acme",
            "--timeout",
            "2",
            "--for",
            "1",
            "--format",
            "json",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect()
    };
    let run_of = |args: Vec<String>| {
        let bus = &bus;
        async move {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            bus.until(&refs, |r| r.code != 2).await
        }
    };

    // Healthy: the owner's current state, its own stamp.
    let state = "acme/zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0";
    let run = run_of(why(state)).await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["report"], "why");
    assert_eq!(doc["verdict"]["answer"], "not_established", "{run}");
    assert!(doc.get("stopped_at").is_none());
    let answer = rung(&doc, "answer")["verdict"]["reason"].to_string();
    assert!(answer.contains("the owner answers its state GET"), "{run}");
    assert!(answer.contains("the owner's own clock"), "S1: {run}");
    assert_eq!(
        doc["value"]["value"],
        eth0(true),
        "rendered through the contract"
    );
    assert_eq!(rung(&doc, "last-known")["verdict"]["answer"], "not_asked");

    // Healthy: a stream sample within the window.
    let run = run_of(why(
        "acme/zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0",
    ))
    .await;
    exits(&run, 0);
    assert_eq!(run.json()["window_s"], 1.0, "{run}");

    // Healthy: the service, up to its contracts.
    let run = run_of(why("host-a/tc")).await;
    exits(&run, 0);
    assert_eq!(run.json()["subject"], "service", "{run}");

    // A cause at presence: nothing of host-z/tc is visible to this reader.
    let run = bus
        .zenctl(
            &why("acme/zk2/host-z/tc/tc.netif.v1/state/interfaces/default/eth0")
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        )
        .await;
    exits(&run, 1);
    let doc = run.json();
    assert_eq!(doc["stopped_at"], "presence", "{run}");
    assert!(
        doc["rows"]
            .to_string()
            .contains("no token of host-z/tc visible to this reader"),
        "{run}"
    );

    // A cause at the descriptor: host-a/tc does not implement tc.netem.v1.
    let run = run_of(why(
        "acme/zk2/host-a/tc/tc.netem.v1/state/qdisc/default/eth0",
    ))
    .await;
    exits(&run, 1);
    let doc = run.json();
    assert_eq!(doc["stopped_at"], "descriptor", "{run}");
    assert!(
        rung(&doc, "descriptor")["cause"]
            .as_str()
            .is_some_and(|c| c.contains("does not implement tc.netem.v1")),
        "{run}"
    );

    // A cause at the contract: no state resource of tc.netif.v1 matches.
    let run = run_of(why(
        "acme/zk2/host-a/tc/tc.netif.v1/state/nothing/here/at/all",
    ))
    .await;
    exits(&run, 1);
    let doc = run.json();
    assert_eq!(doc["stopped_at"], "contract", "{run}");
    assert_eq!(rung(&doc, "contract")["verdict"]["answer"], "established");

    // Unobservable: the owner never wrote eth9 — silence, never a verdict;
    // the archives asked after it, and none visible.
    let args = why("acme/zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth9");
    let run = bus
        .zenctl(&args.iter().map(String::as_str).collect::<Vec<_>>())
        .await;
    exits(&run, 2);
    let doc = run.json();
    assert_eq!(doc["stopped_at"], "answer", "{run}");
    assert_eq!(doc["verdict"]["answer"], "unobservable");
    assert!(
        doc["verdict"]["reason"]
            .as_str()
            .is_some_and(|r| r.contains("silence is not a verdict") && r.contains("last-known")),
        "{run}"
    );
    assert_eq!(
        rung(&doc, "last-known")["verdict"]["answer"],
        "unobservable"
    );
    assert!(
        run.stderr.contains("exit 2, the reserved non-verdict"),
        "{run}"
    );
}

/// Until `service list` in `namespace` shows every one of `addresses`
/// with its descriptor served.
async fn wait_for_ns(bus: &Bus, namespace: &str, addresses: &[&str]) {
    let want: BTreeSet<String> = addresses.iter().map(|a| (*a).to_owned()).collect();
    let run = bus
        .until(
            &[
                "service",
                "list",
                "--namespace",
                namespace,
                "--timeout",
                "2",
                "--format",
                "json",
            ],
            |r| {
                r.code == 0
                    && serde_json::from_str::<Value>(&r.stdout).is_ok_and(|d| {
                        let served: BTreeSet<String> = d["rows"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|i| i["descriptor"]["answer"] == "served")
                            .filter_map(|i| i["address"].as_str().map(str::to_owned))
                            .collect();
                        want.is_subset(&served)
                    })
            },
        )
        .await;
    exits(&run, 0);
}

/// One case's row in a `check conform --format json` document.
fn conform_case<'a>(doc: &'a Value, case: &str, subject: &str) -> &'a Value {
    rows_of(doc, "case")
        .into_iter()
        .find(|r| r["case"] == case && r["subject"] == subject)
        .unwrap_or_else(|| panic!("{case} {subject} has a row: {doc}"))
}

/// #703: a conforming service — FJ8a's mock owner, `gen`, publishing every
/// resource of `tc.netif.v1` through the runtime's writers and answering
/// every operation — passes every case asked, exit 0; the operation that
/// is not idempotent is not called, and a resource with no horizon is not
/// asked its freshness. Its state's population passes `budget` by the
/// owner's complete GET, and its best-effort stream's `budget-window` is
/// unobservable whatever the window, since no window of it is lossless
/// (§2.7, #735; 0.25, #740): exit 2 until `--skip budget-window`, which
/// leaves the state's `budget` asked and passing.
/// `--junit` writes the suite. The router's admin space is on: the owner's
/// own stamp passes only against a router this run verified (S1, §4.2,
/// 0.17).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn check_conform_passes_a_conforming_service() {
    let bus = Bus::admin(Some("acme")).await;
    let netif = examples().join("tcgui/tc.netif.v1.toml");
    let netif = netif.to_str().expect("a UTF-8 path").to_owned();
    let _mock = bus.spawn(&[
        "gen",
        "host-a/tc",
        "tc.netif.v1",
        "--contracts",
        &netif,
        "--rate",
        "5",
        "--duration",
        "15",
        "--namespace",
        "acme",
        "--format",
        "json",
    ]);
    wait_for_ns(&bus, "acme", &["host-a/tc"]).await;
    let junit = bus.home.join("conform.xml");
    let junit = junit.to_str().expect("a UTF-8 path").to_owned();
    // §2.7 (0.25): the stream is best-effort, so its `budget-window` is the
    // one case left unobservable, and says why; the state's `budget` passes
    // by the owner's GET.
    let unskipped = [
        "check",
        "conform",
        "host-a/tc",
        "tc.netif.v1",
        "--namespace",
        "acme",
        "--for",
        "2",
        "--timeout",
        "2",
        "--format",
        "json",
    ];
    let run = bus
        .until(&unskipped, |r| {
            r.code == 2
                && serde_json::from_str::<Value>(&r.stdout).is_ok_and(|d| {
                    rows_of(&d, "case").iter().any(|c| {
                        c["case"] == "budget"
                            && c["subject"] == "state/interfaces/{ns}/{iface}"
                            && c["verdict"]["answer"] == "not_established"
                    })
                })
        })
        .await;
    exits(&run, 2);
    let doc = run.json();
    let bw = conform_case(&doc, "budget-window", "stream/bandwidth/{ns}/{iface}");
    assert_eq!(bw["verdict"]["answer"], "unobservable", "{run}");
    assert!(
        bw["verdict"]["reason"]
            .to_string()
            .contains("best-effort: a dropped sample leaves no trace"),
        "{run}"
    );
    assert!(
        doc["judgement"]["reason"].to_string().contains(
            "1 case(s) could not be established: budget-window stream/bandwidth/{ns}/{iface}"
        ),
        "{run}"
    );
    let args = [
        "check",
        "conform",
        "host-a/tc",
        "tc.netif.v1",
        "--namespace",
        "acme",
        "--for",
        "2",
        "--timeout",
        "2",
        "--skip",
        "budget-window",
        "--junit",
        &junit,
        "--format",
        "json",
    ];
    let run = bus.until(&args, |r| r.code == 0).await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["report"], "conform");
    assert_eq!(doc["judgement"]["answer"], "not_established", "{run}");
    for (case, subject) in [
        ("resource-served", "stream/bandwidth/{ns}/{iface}"),
        ("resource-served", "state/namespaces"),
        ("payload-type", "stream/bandwidth/{ns}/{iface}"),
        ("payload-type", "state/interfaces/{ns}/{iface}"),
        ("qos", "stream/bandwidth/{ns}/{iface}"),
        ("state-stamp", "state/interfaces/{ns}/{iface}"),
        ("state-get", "state/namespaces"),
        ("operation", "@op/diagnostics"),
    ] {
        assert_eq!(
            conform_case(&doc, case, subject)["verdict"]["answer"],
            "not_established",
            "{case} {subject}: {run}"
        );
    }
    assert!(
        rows_of(&doc, "case")
            .iter()
            .any(|c| c["case"] == "contract-served" && c["verdict"]["answer"] == "not_established"),
        "{run}"
    );
    let set = conform_case(&doc, "operation", "@op/interfaces/{ns}/{iface}/set");
    assert_eq!(
        set["verdict"]["answer"], "not_asked",
        "not idempotent: {run}"
    );
    // freshness.v1 (#720): `gen` puts its state at 5 Hz, so the window
    // hears it fresh, and its replies' stamps age against a clock measured
    // on those puts; `state/namespaces` declares no horizon.
    assert_eq!(
        conform_case(&doc, "freshness", "state/interfaces/{ns}/{iface}")["verdict"]["answer"],
        "not_established",
        "{run}"
    );
    assert_eq!(
        conform_case(&doc, "freshness", "state/namespaces")["verdict"]["answer"],
        "not_asked"
    );
    // Skipped, the stream's window case; still asked, and passing by the
    // owner's GET of gen's two members, the state's budget.
    let bw = conform_case(&doc, "budget-window", "stream/bandwidth/{ns}/{iface}");
    assert_eq!(bw["verdict"]["answer"], "not_asked", "{run}");
    assert_eq!(bw["detail"], "skipped by the operator", "{run}");
    let state = conform_case(&doc, "budget", "state/interfaces/{ns}/{iface}");
    assert_eq!(state["verdict"]["answer"], "not_established", "{run}");
    assert!(
        state["verdict"]["reason"]
            .to_string()
            .contains("ran to its final reply"),
        "{run}"
    );
    let xml = std::fs::read_to_string(&junit).expect("the JUnit file");
    assert!(xml.contains("failures=\"0\" errors=\"0\""), "{xml}");
    assert!(xml.contains("<skipped message=\"not asked:"), "{xml}");
}

// ── the population budget (core §2.7, 0.24; #735) ─────────────────────────

/// A contract written for a budget test, from its text.
fn contract_text(name: &str, text: &str) -> Contract {
    let l = zenkey::model::contract::load_str(text, Path::new(env!("CARGO_MANIFEST_DIR")), None);
    l.contract
        .unwrap_or_else(|| panic!("{name} does not load:\n{}", l.report))
}

/// `inv.v1`: one templated state of bound 4, raw text, no horizon.
fn inv() -> Contract {
    contract_text(
        "inv.v1",
        "[interface]\nname = \"inv\"\nmajor = 1\n\
         [resources.\"items/{item}\"]\nkind = \"state\"\ntype = { raw = \"text/plain\" }\n\
         params = { item = \"string\" }\ncardinality = 4\n",
    )
}

/// An instance of `inv.v1` at `address` on `session`, its descriptor
/// lowering the bound to `bound`, holding `items` it re-puts every 200 ms.
/// The runtime keeps the bound (§2.7, #741): it refuses more items than it.
async fn inventory(
    session: &zenoh::Session,
    address: &str,
    bound: u64,
    items: &[&str],
) -> (Service, Task) {
    let id = iface("inv.v1");
    let mut b = ServiceBuilder::new(
        session,
        ServiceConfig::new(address.parse().expect("an address")),
    );
    b.implement(Implementation::new(inv())).expect("implement");
    b.cardinality(&id, "state/items/{item}", bound)
        .expect("a lowering");
    let mut writers = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let item: Bindings = [("item".to_owned(), vec![(*item).to_owned()])].into();
        let w = b
            .declare_state_writer(&id, "state/items/{item}", &item)
            .await
            .expect("a member");
        w.put(format!("{i}")).await.expect("put");
        writers.push(w);
    }
    let service = b.start().await.expect("start");
    let task = Task(tokio::spawn(async move {
        loop {
            for w in &writers {
                let _ = w.put("x").await;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }));
    (service, task)
}

/// Core §2.7 (#735): two instances of one service, each holding one member
/// within its descriptor's bound of one — the runtime keeps each instance's
/// (#741) — hold two members of one population, since a key names no
/// instance: by the owners' complete GET, `check conform`'s `budget` case
/// is the violation, exit 1, naming the descriptor's bound. Beside it, an
/// owner of the same contract holding two members within its lowered bound
/// of two passes every case asked, exit 0: a state's population is the one
/// a reading can show within its bound.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn check_conform_judges_a_population_against_its_lowered_bound() {
    let mut bus = Bus::admin(None).await;
    let other = client(&bus.endpoint, None).await;
    let mut up = Vec::new();
    for (session, address, bound, items) in [
        (&bus.owners, "host-a/inv", 1, &["i0"][..]),
        (&other, "host-a/inv", 1, &["i1"][..]),
        (&bus.owners, "host-b/inv", 2, &["i0", "i1"][..]),
    ] {
        up.push(inventory(session, address, bound, items).await);
    }
    for (owner, task) in up {
        bus.services.push(owner);
        bus.keep(task);
    }
    bus.keep(other);
    wait_for(&bus, &["host-a/inv", "host-b/inv"]).await;
    let conform = |address: &'static str| {
        [
            "check",
            "conform",
            address,
            "inv.v1",
            "--for",
            "2",
            "--timeout",
            "2",
            "--format",
            "json",
        ]
    };
    let items = "state/items/{item}";
    let over = conform("host-a/inv");
    let run = bus.until(&over, |r| r.code == 1).await;
    exits(&run, 1);
    let doc = run.json();
    assert_eq!(doc["judgement"]["answer"], "established", "{run}");
    let b = conform_case(&doc, "budget", items);
    assert_eq!(b["verdict"]["answer"], "established", "{run}");
    let said = b["detail"].to_string();
    assert!(
        said.contains("2 member(s) answered with a value")
            && said.contains("its bound of 1 (its descriptor's; the contract's is 4)"),
        "{run}"
    );
    let within = conform("host-b/inv");
    let run = bus.until(&within, |r| r.code == 0).await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["judgement"]["answer"], "not_established", "{run}");
    let b = conform_case(&doc, "budget", items);
    assert_eq!(b["verdict"]["answer"], "not_established", "{run}");
    assert!(
        b["verdict"]["reason"]
            .to_string()
            .contains("within its bound of 2"),
        "{run}"
    );
}

/// `al.v1`: an event, at most once a minute per source (`low`), at most
/// two sources within its hour's retention.
fn alarms() -> Contract {
    contract_text(
        "al.v1",
        "[interface]\nname = \"al\"\nmajor = 1\n\
         [resources.\"alarms/{src}\"]\nkind = \"event\"\ntype = { raw = \"text/plain\" }\n\
         params = { src = \"string\" }\ncardinality = 2\nrate = \"low\"\n\
         retention = \"1h\"\n",
    )
}

/// Core §2.7 (#735): an owner publishing an occurrence of each of three
/// sources every 300 ms breaks both of an event's budgets — two occurrences
/// of one source within a minute, beyond `low`, and three sources within
/// the retention, above its two — and a window of two seconds shows both:
/// `rate` and `budget-window` are violations, exit 1. A window that heard
/// no excess could only say `low` kept after a whole minute (unit-tested; a
/// live minute is not spent here). The reference runtime refuses both
/// (#741), so the occurrences are the owner's session's own puts, on the
/// event's keys and QoS: an owner whose runtime does not keep §2.7.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn check_conform_judges_an_events_rate_and_population() {
    let mut bus = Bus::admin(None).await;
    let id = iface("al.v1");
    let mut b = ServiceBuilder::new(
        &bus.owners,
        ServiceConfig::new("host-a/al".parse().expect("an address")),
    );
    b.implement(Implementation::new(alarms()))
        .expect("implement");
    b.expose(&id, "events/alarms/{src}").expect("exposed");
    bus.services.push(b.start().await.expect("start"));
    let session = bus.owners.clone();
    bus.keep(Task(tokio::spawn(async move {
        loop {
            for src in ["a", "b", "c"] {
                let ulid = zenkey::writer::ulid(std::time::SystemTime::now());
                let _ = session
                    .put(
                        format!("zk2/host-a/al/al.v1/events/alarms/{src}/{ulid}"),
                        "alarm",
                    )
                    .encoding(zenoh::bytes::Encoding::TEXT_PLAIN)
                    .congestion_control(zenoh::qos::CongestionControl::Block)
                    .await;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    })));
    wait_for(&bus, &["host-a/al"]).await;
    let args = [
        "check",
        "conform",
        "host-a/al",
        "al.v1",
        "--for",
        "2",
        "--timeout",
        "2",
        "--format",
        "json",
    ];
    let both = |r: &Run| {
        r.code == 1
            && serde_json::from_str::<Value>(&r.stdout).is_ok_and(|d| {
                ["rate", "budget-window"].iter().all(|case| {
                    rows_of(&d, "case")
                        .iter()
                        .any(|c| c["case"] == *case && c["verdict"]["answer"] == "established")
                })
            })
    };
    let run = bus.until(&args, both).await;
    exits(&run, 1);
    let doc = run.json();
    let events = "events/alarms/{src}";
    let rate = conform_case(&doc, "rate", events);
    assert!(
        rate["detail"].to_string().contains("within a minute")
            && rate["detail"].to_string().contains("its rate low allows 1"),
        "{run}"
    );
    let budget = conform_case(&doc, "budget-window", events);
    assert!(
        budget["detail"]
            .to_string()
            .contains("3 member(s) with an occurrence within its retention of an hour"),
        "{run}"
    );
}

/// #703: a service that breaks its contract three ways — a stream value
/// that does not satisfy its type, a sample off the declared QoS, an
/// operation it exposes and leaves silent while holding its tokens — is a
/// violation per case, exit 1, and `--junit` says so.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn check_conform_names_a_wrong_type_a_qos_mismatch_and_a_silent_operation() {
    let mut bus = Bus::bare(Some("acme")).await;
    // `stats` a string, where `json:BandwidthUpdate` declares an object.
    let (owner, task) = publishing_owner(&bus, json!({"stats": "broken"}), true).await;
    bus.services.push(owner);
    bus.keep(task);
    // A sample on the owner's stream at a priority its contract does not
    // declare.
    let off = intruder(
        &bus,
        "acme/zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth1",
        r#"{"namespace":"default","interface":"eth1","stats":{"rx_bytes":1,"tx_bytes":1}}"#,
        Priority::RealTime,
    )
    .await;
    bus.keep(off);
    wait_for_ns(&bus, "acme", &["host-a/tc"]).await;
    let junit = bus.home.join("conform.xml");
    let junit = junit.to_str().expect("a UTF-8 path").to_owned();
    let args = [
        "check",
        "conform",
        "host-a/tc",
        "tc.netif.v1",
        "--namespace",
        "acme",
        "--for",
        "2",
        "--timeout",
        "1",
        "--junit",
        &junit,
        "--format",
        "json",
        // No access control here: the operator's word that a silent call is
        // the O3 finding (§5.1, 0.17).
        "--calls-granted",
    ];
    let run = bus
        .until(&args, |r| {
            r.code == 1
                && serde_json::from_str::<Value>(&r.stdout).is_ok_and(|d| {
                    rows_of(&d, "case")
                        .iter()
                        .filter(|c| c["verdict"]["answer"] == "established")
                        .count()
                        >= 3
                })
        })
        .await;
    exits(&run, 1);
    let doc = run.json();
    assert_eq!(doc["judgement"]["answer"], "established");
    let bandwidth = "stream/bandwidth/{ns}/{iface}";
    let t = conform_case(&doc, "payload-type", bandwidth);
    assert_eq!(t["verdict"]["answer"], "established", "{run}");
    assert!(t["detail"].to_string().contains("/stats"), "{run}");
    let q = conform_case(&doc, "qos", bandwidth);
    assert_eq!(q["verdict"]["answer"], "established", "{run}");
    assert!(q["detail"].to_string().contains("real_time"), "{run}");
    let s = conform_case(&doc, "operation", "@op/diagnostics");
    assert_eq!(s["verdict"]["answer"], "established", "{run}");
    assert!(
        s["detail"].to_string().contains("never silence (O3)"),
        "{run}"
    );
    let xml = std::fs::read_to_string(&junit).expect("the JUnit file");
    assert!(xml.contains("<failure type=\"violation\""), "{xml}");
    // Without that word, the same silence is unobservable: no tool can
    // observe its grants (§5.1, §11.3).
    let unsaid = &args[..args.len() - 1];
    let run = bus
        .until(unsaid, |r| {
            serde_json::from_str::<Value>(&r.stdout).is_ok_and(|d| {
                conform_case(&d, "operation", "@op/diagnostics")["verdict"]["answer"]
                    == "unobservable"
            })
        })
        .await;
    let doc = run.json();
    let s = conform_case(&doc, "operation", "@op/diagnostics");
    assert!(
        s["verdict"]["reason"].to_string().contains("§11.3"),
        "{run}"
    );
}

/// `beacon.v1`, freshness.v1's scenario contract, kept with the runtime's.
fn beacon() -> Contract {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../zenkey/tests/contracts/beacon.v1.toml");
    let l = load_path(&path);
    l.contract
        .unwrap_or_else(|| panic!("beacon.v1 does not load:\n{}", l.report))
}

/// `spec/profiles/freshness/scenarios.md` §6 (#720): `check conform` judges
/// freshness per resource at the end of its window. An owner refreshing
/// `status`, holding `intent` (never stale) and `note` (no horizon), and
/// publishing `level` every 200 ms reads fresh, fresh, not asked, fresh.
/// Read again, with the `status` writer closed and `level` stopped 0.5 s
/// into the window, it reads `status` and `level` stale: violations, exit
/// 1, though the owner holds its tokens throughout.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn check_conform_judges_a_stopped_refresher_stale() {
    let mut bus = Bus::admin(None).await;
    let id = iface("beacon.v1");
    let mut b = ServiceBuilder::new(
        &bus.owners,
        ServiceConfig::new("lab/beacon".parse().expect("an address")),
    );
    b.implement(Implementation::new(beacon()))
        .expect("implement");
    let none = Bindings::new();
    let status = b
        .declare_state_writer(&id, "state/status", &none)
        .await
        .expect("status");
    let intent = b
        .declare_state_writer(&id, "state/intent", &none)
        .await
        .expect("intent");
    let note = b
        .declare_state_writer(&id, "state/note", &none)
        .await
        .expect("note");
    let level = Arc::new(
        b.declare_writer(&id, "stream/level", &none)
            .await
            .expect("level"),
    );
    status.put("up").await.expect("put");
    intent.put("i").await.expect("put");
    note.put("n").await.expect("put");
    bus.services.push(b.start().await.expect("start"));
    let l = Arc::clone(&level);
    let publishing = Task(tokio::spawn(async move {
        let mut n = 0u64;
        loop {
            n += 1;
            let _ = l.put(format!("{n}")).await;
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }));
    bus.keep((intent, note));
    wait_for(&bus, &["lab/beacon"]).await;
    let args = [
        "check",
        "conform",
        "lab/beacon",
        "beacon.v1",
        "--for",
        "4",
        "--timeout",
        "2",
        "--format",
        "json",
    ];
    let fresh = |r: &Run| {
        serde_json::from_str::<Value>(&r.stdout).is_ok_and(|d| {
            ["state/status", "state/intent", "stream/level"]
                .iter()
                .all(|s| {
                    rows_of(&d, "case").iter().any(|c| {
                        c["case"] == "freshness"
                            && c["subject"] == *s
                            && c["verdict"]["answer"] == "not_established"
                    })
                })
        })
    };
    let run = bus.until(&args, fresh).await;
    let doc = run.json();
    for s in ["state/status", "state/intent", "stream/level"] {
        assert_eq!(
            conform_case(&doc, "freshness", s)["verdict"]["answer"],
            "not_established",
            "{s}: {run}"
        );
    }
    assert!(
        conform_case(&doc, "freshness", "state/intent")["verdict"]["reason"]
            .to_string()
            .contains("never stale"),
        "{run}"
    );
    assert_eq!(
        conform_case(&doc, "freshness", "state/note")["verdict"]["answer"],
        "not_asked",
        "{run}"
    );

    // Step 2: the tool reads again; once its subscription on `level` is
    // declared (the last it declares, just before its window opens) and
    // 0.5 s have passed, the refresher and the stream stop.
    let matching = |want: bool| {
        let level = Arc::clone(&level);
        async move {
            let deadline = Instant::now() + SETTLE;
            while level.matching().await.expect("matching") != want {
                assert!(Instant::now() < deadline, "level never matched {want}");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    };
    matching(false).await;
    let pending = bus.spawn(&args);
    matching(true).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    drop(status);
    drop(publishing);
    let run = pending.await.expect("the zenctl runner");
    exits(&run, 1);
    let doc = run.json();
    assert_eq!(doc["judgement"]["answer"], "established", "{run}");
    for s in ["state/status", "stream/level"] {
        let c = conform_case(&doc, "freshness", s);
        assert_eq!(c["verdict"]["answer"], "established", "{s}: {run}");
        assert!(
            c["detail"]
                .to_string()
                .contains("never the owner's presence"),
            "{s}: {run}"
        );
    }
    assert_eq!(
        conform_case(&doc, "freshness", "state/intent")["verdict"]["answer"],
        "not_established",
        "{run}"
    );
    assert_eq!(
        conform_case(&doc, "freshness", "state/note")["verdict"]["answer"],
        "not_asked",
        "{run}"
    );
    // Present throughout: stale is the value's, never the owner's presence.
    wait_for(&bus, &["lab/beacon"]).await;
}

// ── health.v1 (#721, PF): spec/profiles/health/scenarios.md §6–§8 ──────────
//
// The tool's sections. T takes the deployment's word for its clock in §6
// and §7 (`--clocks-synced`, the conventions, text 0.2): every session here
// runs on one host. §8's ground measures nothing either: across the face it
// judges by its subscription's receive clock.

/// One service's row in a `health --format json` document.
fn health_row<'a>(doc: &'a Value, address: &str) -> &'a Value {
    rows_of(doc, "service")
        .into_iter()
        .find(|r| r["address"] == address)
        .unwrap_or_else(|| panic!("{address} has a row: {doc}"))
}

/// `health`'s run, its document, and `address`'s row in it, when it parses.
fn health_of(run: &Run, address: &str) -> Option<Value> {
    let doc: Value = serde_json::from_str(&run.stdout).ok()?;
    rows_of(&doc, "service")
        .into_iter()
        .find(|r| r["address"] == address)
        .cloned()
}

/// An owner of `health.v1` at `address` through the runtime's `Health`, its
/// status declared at `level` with `reason`, started.
async fn health_owner(
    s: &zenoh::Session,
    address: &str,
    level: zenkey::health::Level,
    reason: &str,
) -> (Service, zenkey::health::Health) {
    let mut b = ServiceBuilder::new(s, ServiceConfig::new(address.parse().expect("an address")));
    let h = b.health().await.expect("health.v1");
    h.set_status(level, reason).await.expect("a status");
    (b.start().await.expect("start"), h)
}

/// health.v1 `scenarios.md` §6 (§2.1): an owner holding DEGRADED reads
/// unhealthy, degraded. Once its process ends, a complete presence read
/// holds no token of it and its GET draws no reply: not asked, absent —
/// never FAILED, never stale — with the archive's status shown last-known,
/// its stamp and the archive's `confirmed`, never as current. Nothing was
/// asked, so the run is no verdict, exit 2.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn health_s6_an_absent_owner_is_not_asked_and_its_archive_last_known() {
    use zenkey::health::Level;
    let bus = Bus::bare(None).await;
    let origin = "zk2/lab/svc/health.v1/state/status";
    let archiving = client(&bus.endpoint, None).await;
    let archive = Archive::start(
        &archiving,
        ArchiveConfig {
            service: ServiceConfig::new("lab/archive".parse().expect("an address")),
            records: vec![Recorded {
                owner: "lab/svc".parse().expect("an address"),
                selector: "zk2/lab/svc/health.v1/state/**".to_owned(),
                implementation: zenkey::health::implementation(),
            }],
            peers: Vec::new(),
            unconfirmed_horizon: None,
            watch: None,
        },
    )
    .await
    .expect("an archive");
    let owning = client(&bus.endpoint, None).await;
    let (svc, h) = health_owner(&owning, "lab/svc", Level::Degraded, "upstream lost").await;
    let deadline = Instant::now() + SETTLE;
    while archive.confirmed(origin).is_none() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        archive.confirmed(origin).is_some(),
        "the archive recorded it"
    );

    // Step 1: T reads the owner.
    let args = [
        "health",
        "lab/svc",
        "--clocks-synced",
        "--timeout",
        "1",
        "--format",
        "json",
    ];
    let run = bus.until(&args, |r| r.code == 1).await;
    exits(&run, 1);
    let doc = run.json();
    assert_eq!(doc["report"], "health");
    assert_eq!(doc["clock"], "deployment_word");
    let row = health_row(&doc, "lab/svc");
    assert_eq!(row["verdict"], "unhealthy", "{run}");
    assert_eq!(row["reason"], "degraded");
    assert_eq!(row["level"], "degraded");
    assert_eq!(row["status"]["reason"], "upstream lost");

    // Step 2: the owner's process ends.
    drop(h);
    svc.close().await.expect("the owner closes");
    owning.close().await.expect("its session ends");

    // Step 3: a complete presence read, the owner's GET, the archive's form.
    let run = bus
        .until(&args, |r| {
            health_of(r, "lab/svc").is_some_and(|row| row["verdict"] == "not_asked")
        })
        .await;
    exits(&run, 2);
    let doc = run.json();
    assert_eq!(doc["presence"]["complete"], true, "{run}");
    let row = health_row(&doc, "lab/svc");
    assert_eq!(row["reason"], "absent", "{run}");
    assert!(row.get("level").is_none(), "absent is no level: {run}");
    assert!(
        row.get("status").is_none(),
        "the GET drew no reply, and nothing is shown as current: {run}"
    );
    for r in row["readings"].as_array().expect("readings") {
        assert_eq!(
            r["verdict"], "not_asked",
            "never FAILED, never stale: {run}"
        );
    }
    let lk = &row["last_known"];
    assert_eq!(lk["archive"], "lab/archive", "{run}");
    assert_eq!(lk["level"], "degraded");
    assert_eq!(lk["reason"], "upstream lost");
    assert!(lk["confirmed"].is_boolean(), "{run}");
    assert!(lk["stamp"]["clock"].is_string(), "its stamp: {run}");
    let table = bus
        .zenctl(&[
            "health",
            "lab/svc",
            "--clocks-synced",
            "--timeout",
            "1",
            "--format",
            "table",
        ])
        .await;
    assert!(
        table.stdout.contains("last-known, never current"),
        "{table}"
    );
    drop(archive);
}

/// health.v1 `scenarios.md` §7 (§2.2): `lab/liar` holds status OK and check
/// `disk` FAILED, which §2.2 forbids and `Health` would never put, so it is
/// written through the bare contract; `lab/frank` holds FAILED and `disk`
/// OK. Read twice, 2 s apart: the liar reads unhealthy, inconsistent, at
/// FAILED, in each reading, and the break is the finding about the owner,
/// seen in both; frank reads unhealthy, failed, with no break — a status may
/// be worse than its checks.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn health_s7_an_inconsistent_status_is_a_finding_in_both_readings() {
    use zenkey::health::{Level, v1};
    use zenkey::prost::Message as _;
    let mut bus = Bus::bare(None).await;
    let id = zenkey::health::iface();
    let none = Bindings::new();
    let mut b = ServiceBuilder::new(
        &bus.owners,
        ServiceConfig::new("lab/liar".parse().expect("an address")),
    );
    b.implement(zenkey::health::implementation())
        .expect("implement");
    let status = b
        .declare_state_writer(&id, zenkey::health::STATUS, &none)
        .await
        .expect("the status");
    b.expose(&id, zenkey::health::CHECKS).expect("the checks");
    let faults = b
        .declare_writer(&id, zenkey::health::FAULTS, &none)
        .await
        .expect("the faults");
    status
        .put(
            v1::Status {
                level: v1::Level::Ok as i32,
                reason: "serving".into(),
                since_ns: 1,
            }
            .encode_to_vec(),
        )
        .await
        .expect("a status");
    let mut liar = b.start().await.expect("start");
    let disk = liar
        .state_writer(
            &id,
            zenkey::health::CHECKS,
            &[("check".to_owned(), vec!["disk".to_owned()])].into(),
        )
        .await
        .expect("a check");
    disk.put(
        v1::Check {
            level: v1::Level::Failed as i32,
            detail: "full".into(),
            checked_at_ns: 1,
        }
        .encode_to_vec(),
    )
    .await
    .expect("a check put");
    let (frank, fh) = {
        let mut b = ServiceBuilder::new(
            &bus.owners,
            ServiceConfig::new("lab/frank".parse().expect("an address")),
        );
        let h = b.health().await.expect("health.v1");
        h.set_status(Level::Failed, "down").await.expect("a status");
        h.set_check("disk", Level::Ok, "").await.expect("a check");
        (b.start().await.expect("start"), h)
    };
    bus.services.extend([liar, frank]);
    bus.keep((status, disk, faults, fh));
    wait_for(&bus, &["lab/liar", "lab/frank"]).await;

    let args = [
        "health",
        "--clocks-synced",
        "--timeout",
        "1",
        "--format",
        "json",
    ];
    let run = bus
        .until(&args, |r| {
            r.code == 1
                && health_of(r, "lab/liar").is_some_and(|row| row["agrees"]["answer"] == "no")
                && health_of(r, "lab/frank").is_some_and(|row| row["reason"] == "failed")
        })
        .await;
    exits(&run, 1);
    let doc = run.json();
    assert!(
        doc["apart_s"].as_f64().expect("apart_s") >= 1.9,
        "two readings 2 s apart: {run}"
    );
    let liar = health_row(&doc, "lab/liar");
    assert_eq!(
        (&liar["verdict"], &liar["reason"], &liar["level"]),
        (
            &json!("unhealthy"),
            &json!("inconsistent"),
            &json!("failed")
        ),
        "{run}"
    );
    let readings = liar["readings"].as_array().expect("readings");
    assert_eq!(readings.len(), 2);
    for r in readings {
        assert_eq!(
            r,
            &json!({"verdict": "unhealthy", "reason": "inconsistent", "level": "failed"}),
            "each reading, never healthy: {run}"
        );
    }
    assert_eq!(liar["agrees"]["answer"], "no");
    assert_eq!(liar["agrees"]["reason"], "inconsistent");
    let frank = health_row(&doc, "lab/frank");
    assert_eq!(frank["verdict"], "unhealthy");
    assert_eq!(frank["level"], "failed");
    assert_eq!(frank["agrees"]["answer"], "yes", "no break: {run}");
    assert_eq!(doc["rollup"]["worst"], "failed");
    assert_eq!(doc["rollup"]["unhealthy"], 2);
}

/// The vehicle router of health.v1 `scenarios.md` §8: the ground segment G
/// attached as a client (`spec/scenarios/constrained.md` §1, step 2) and
/// authenticated as the usrpwd principal `ground`, denied `zk2/**/@zk/**`
/// on its face both ways; with `deny_state`, `zk2/vehicle-01/*/*/state/**`
/// too, as zenoh-modem's `face-deny-device-state` does (run B). The owners
/// authenticate as `vehicle`, whom nothing denies. The returned path is the
/// ground's `--zenoh-config`.
async fn face_bus(deny_state: bool) -> (Bus, PathBuf) {
    static NTH: AtomicU64 = AtomicU64::new(0);
    let home = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("live-zk2-home")
        .join(format!(
            "face-{}-{}",
            std::process::id(),
            NTH.fetch_add(1, Ordering::Relaxed)
        ));
    std::fs::create_dir_all(&home).expect("a config root of its own");
    let dict = home.join("users.txt");
    std::fs::write(
        &dict,
        "router:router-pw\nvehicle:vehicle-pw\nground:ground-pw\n",
    )
    .expect("the dictionary");
    let messages = "[\"put\", \"delete\", \"declare_subscriber\", \"query\", \
                    \"declare_queryable\", \"reply\", \"liveliness_token\", \
                    \"declare_liveliness_subscriber\", \"liveliness_query\"]";
    let rule = |id: &str, ke: &str| {
        format!(
            "{{ id: \"{id}\", permission: \"deny\", flows: [\"egress\", \"ingress\"], \
             messages: {messages}, key_exprs: [\"{ke}\"] }}"
        )
    };
    let mut rules = vec![rule("face-deny-presence", "zk2/**/@zk/**")];
    let mut ids = vec!["\"face-deny-presence\""];
    if deny_state {
        rules.push(rule(
            "face-deny-device-state",
            "zk2/vehicle-01/*/*/state/**",
        ));
        ids.push("\"face-deny-device-state\"");
    }
    let text = format!(
        "{{\n  mode: \"router\",\n  \
         scouting: {{ multicast: {{ enabled: false }}, gossip: {{ enabled: false }} }},\n  \
         listen: {{ endpoints: [\"tcp/127.0.0.1:0\"] }},\n  \
         adminspace: {{ enabled: false }},\n  \
         transport: {{ auth: {{ usrpwd: {{ user: \"router\", password: \"router-pw\", \
         dictionary_file: {:?} }} }} }},\n  \
         access_control: {{\n    enabled: true,\n    default_permission: \"allow\",\n    \
         rules: [{}],\n    subjects: [{{ id: \"ground\", usernames: [\"ground\"] }}],\n    \
         policies: [{{ id: \"face\", rules: [{}], subjects: [\"ground\"] }}],\n  }},\n}}\n",
        dict.display().to_string(),
        rules.join(", "),
        ids.join(", ")
    );
    let router = zenoh::open(
        zenoh::Config::from_json5(&text)
            .unwrap_or_else(|e| panic!("the vehicle router's config: {e}\n{text}")),
    )
    .await
    .expect("the vehicle router accepts the face");
    let endpoint = router
        .info()
        .locators()
        .await
        .into_iter()
        .map(|l| l.to_string())
        .find(|l| l.starts_with("tcp/127.0.0.1:"))
        .expect("the router listens on loopback");
    let mut c = base_config();
    c.insert_json5("mode", "\"client\"").expect("config");
    c.insert_json5("connect/endpoints", &format!("[\"{endpoint}\"]"))
        .expect("config");
    c.insert_json5(
        "transport/auth/usrpwd",
        "{ user: \"vehicle\", password: \"vehicle-pw\" }",
    )
    .expect("config");
    let owners = zenoh::open(c).await.expect("the vehicle's session");
    let ground = home.join("ground.json5");
    std::fs::write(
        &ground,
        "{ mode: \"client\", transport: { auth: { usrpwd: { user: \"ground\", password: \
         \"ground-pw\" } } } }\n",
    )
    .expect("the ground's config");
    (
        Bus {
            endpoint,
            home,
            keep: Vec::new(),
            services: Vec::new(),
            owners,
            _router: router,
        },
        ground,
    )
}

/// health.v1 `scenarios.md` §8 (§2.8): across a constrained face, where
/// presence does not cross and G knows from its configuration that
/// `vehicle-01/nav` implements health.v1 (`--across-face`). Run A, the face
/// letting the status cross: before the owner starts, G hears nothing and
/// reads unobservable, `nothing_crossed` — it knows of no status, and calls
/// none stale; once the owner runs, healthy, `ok`; once it stops confirming,
/// stale — never down, never FAILED, never absent. Run B, the face denying
/// the vehicle's state: nothing of the status crosses at any step, and G
/// reads unobservable, `face_closed`, each time. The two runs share no
/// router and run at once; each waits 65 s a step, as the scenario does.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn health_s8_across_a_constrained_face() {
    use zenkey::health::Level;
    const NAV: &str = "vehicle-01/nav";
    const LIMIT: Duration = Duration::from_secs(120);
    let health = |ground: &Path, face: &str, window: &str| -> Vec<String> {
        [
            "health",
            NAV,
            "--across-face",
            face,
            "--for",
            window,
            "--timeout",
            "1",
            "--zenoh-config",
            ground.to_str().expect("a UTF-8 path"),
            "--format",
            "json",
        ]
        .map(str::to_owned)
        .to_vec()
    };
    let run_a = async {
        let (bus, ground) = face_bus(false).await;
        let args = health(&ground, "crosses", "65");
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        // Step 1: G listens before the owner starts.
        let run = bus.spawn_within(&args, LIMIT).await.expect("the runner");
        exits(&run, 2);
        let row = health_of(&run, NAV).unwrap_or_else(|| panic!("a row: {run}"));
        assert_eq!(row["verdict"], "unobservable", "{run}");
        assert_eq!(row["reason"], "nothing_crossed", "never stale: {run}");
        // Step 2: the owner starts; G listens 65 s.
        let (svc, h) = health_owner(&bus.owners, NAV, Level::Ok, "serving").await;
        let run = bus.spawn_within(&args, LIMIT).await.expect("the runner");
        exits(&run, 0);
        let doc = run.json();
        assert!(
            doc.get("presence").is_none(),
            "presence does not cross: {run}"
        );
        assert_eq!(doc["face"]["status_crosses"], true);
        let row = health_row(&doc, NAV);
        assert_eq!(
            (&row["verdict"], &row["reason"], &row["level"]),
            (&json!("healthy"), &json!("ok"), &json!("ok")),
            "{run}"
        );
        // Step 3: the owner stops confirming its status, once G hears it.
        let writer = h.status_writer().writer();
        let matching = |want: bool| async move {
            let deadline = Instant::now() + SETTLE;
            while writer.matching().await.expect("matching") != want {
                assert!(Instant::now() < deadline, "the status never matched {want}");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        matching(false).await;
        let args = health(&ground, "crosses", "70");
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let pending = bus.spawn_within(&args, LIMIT);
        matching(true).await;
        h.set_status(Level::Ok, "serving, its last word")
            .await
            .expect("a last put");
        h.set_confirming(false);
        let run = pending.await.expect("the runner");
        exits(&run, 1);
        let row = health_of(&run, NAV).unwrap_or_else(|| panic!("a row: {run}"));
        assert_eq!(
            row["verdict"], "stale",
            "never down, FAILED or absent: {run}"
        );
        assert_eq!(row["reason"], "beyond_horizon", "{run}");
        assert!(row.get("level").is_none(), "stale is no level: {run}");
        drop((svc, h, bus));
    };
    let run_b = async {
        let (bus, ground) = face_bus(true).await;
        let args = health(&ground, "denied", "65");
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let closed = |run: &Run| {
            exits(run, 2);
            let row = health_of(run, NAV).unwrap_or_else(|| panic!("a row: {run}"));
            assert_eq!(row["verdict"], "unobservable", "{run}");
            assert_eq!(row["reason"], "face_closed", "{run}");
            assert!(
                row.get("status").is_none() && row.get("faults").is_none(),
                "nothing of the status crossed: {run}"
            );
        };
        // Step 1, before the owner starts.
        closed(&bus.spawn_within(&args, LIMIT).await.expect("the runner"));
        // Step 2, once it runs.
        let (svc, h) = health_owner(&bus.owners, NAV, Level::Ok, "serving").await;
        closed(&bus.spawn_within(&args, LIMIT).await.expect("the runner"));
        drop((svc, h, bus));
    };
    tokio::join!(run_a, run_b);
}

/// `pulse.v1`: an event of two sources at most, live for its 2 s retention,
/// its rate no question here.
fn pulses() -> Contract {
    contract_text(
        "pulse.v1",
        "[interface]\nname = \"pulse\"\nmajor = 1\n\
         [resources.\"pulses/{src}\"]\nkind = \"event\"\ntype = { raw = \"text/plain\" }\n\
         params = { src = \"string\" }\ncardinality = 2\nrate = \"burst(100000/h)\"\n\
         retention = \"2s\"\n",
    )
}

/// Core §2.7 (#735), a window's clean pole: an owner publishing two
/// sources every 200 ms, within its bound of two, read over a 4 s window
/// whose 3 s past the 1 s reply timeout are longer than its 2 s retention
/// (0.25) — the event reliable with `block` and nothing lost, its instance
/// token held throughout and watched a lease (`--lease 1`) and the timeout
/// past it — passes `budget-window`. The same owner closed half a second
/// into the window was not present throughout: unobservable, and said so.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn check_conform_passes_a_complete_window_and_not_an_absent_owner() {
    complete_window_then_an_absent_owner(None).await;
}

/// #872 (WC): [`check_conform_passes_a_complete_window_and_not_an_absent_owner`]
/// in a namespace, where zenoh 1.10.1 never tells a liveliness subscriber
/// without history that a token declared before it went: the router tells
/// it by key, and a namespaced session leaves that key unstripped
/// (`routing/namespace.rs`). The owner's instance token predates zenctl's
/// watch session, as every owner's does, and goes half a second into the
/// window. Without history that withdrawal was never heard, and the window
/// read the owner present throughout: `not_established` with "the owner
/// present throughout", a false clean. zenctl's watch session is in no
/// namespace, on the wire keys, its subscriber without history (#875), and
/// hears it go.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn check_conform_in_a_namespace_hears_its_owner_leave_the_window() {
    complete_window_then_an_absent_owner(Some("ns")).await;
}

/// The two runs of the window's poles, in `namespace` when given: clean
/// with the owner present throughout, then unobservable with the owner
/// closed half a second into the window.
async fn complete_window_then_an_absent_owner(namespace: Option<&str>) {
    let mut bus = Bus::admin(namespace).await;
    let id = iface("pulse.v1");
    let start = |bus: &Bus| {
        let mut b = ServiceBuilder::new(
            &bus.owners,
            ServiceConfig::new("host-a/pulse".parse().expect("an address")),
        );
        b.implement(Implementation::new(pulses()))
            .expect("implement");
        let writers: Vec<_> = ["a", "b"]
            .into_iter()
            .map(|src| {
                let v: Bindings = [("src".to_owned(), vec![src.to_owned()])].into();
                b.event_writer(&id, "events/pulses/{src}", &v)
                    .expect("an event writer")
            })
            .collect();
        (b, writers)
    };
    let (b, writers) = start(&bus);
    let owner = b.start().await.expect("start");
    bus.keep(Task(tokio::spawn(async move {
        loop {
            for w in &writers {
                let _ = w.put("pulse").await;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })));
    let mut args = vec![
        "check",
        "conform",
        "host-a/pulse",
        "pulse.v1",
        "--for",
        "4",
        "--timeout",
        "1",
        "--lease",
        "1",
        "--format",
        "json",
    ];
    match namespace {
        Some(ns) => {
            wait_for_ns(&bus, ns, &["host-a/pulse"]).await;
            args.extend(["--namespace", ns]);
        }
        None => wait_for(&bus, &["host-a/pulse"]).await,
    }
    let events = "events/pulses/{src}";
    let run = bus
        .until(&args, |r| {
            serde_json::from_str::<Value>(&r.stdout).is_ok_and(|d| {
                conform_case(&d, "budget-window", events)["verdict"]["answer"] == "not_established"
            })
        })
        .await;
    let doc = run.json();
    let w = conform_case(&doc, "budget-window", events);
    assert!(
        w["verdict"]["reason"]
            .to_string()
            .contains("reliable with block and nothing lost by this run, which never stalled; the owner present throughout and a lease after"),
        "{run}"
    );
    // Half a second into the window, the owner closes: its token goes. The
    // window has opened once the tool's subscription matches a probe
    // publisher on the event's keys (its presence read comes before it).
    let probe = bus
        .owners
        .declare_publisher(
            "zk2/host-a/pulse/pulse.v1/events/pulses/probe/01hzzzzzzzzzzzzzzzzzzzzzzz",
        )
        .await
        .expect("a probe publisher");
    let matching = |want: bool| {
        let probe = &probe;
        async move {
            let deadline = Instant::now() + Duration::from_secs(20);
            while probe.matching_status().await.expect("matching").matching() != want {
                assert!(Instant::now() < deadline, "the probe never matched {want}");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    };
    matching(false).await;
    let pending = bus.spawn(&args);
    matching(true).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let instance = owner
        .instance_key()
        .expect("its instance key")
        .as_str()
        .to_owned();
    owner.close().await.expect("close");
    // Gone where the owners' session reads, before the window ends: the
    // router has told every subscriber it tells.
    let deadline = Instant::now() + SETTLE;
    while !zenkey::presence::liveliness_read(&bus.owners, &instance, Duration::from_secs(1))
        .await
        .expect("a read")
        .keys
        .is_empty()
    {
        assert!(Instant::now() < deadline, "the owner's token never went");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let run = pending.await.expect("the zenctl runner");
    let doc = run.json();
    let w = conform_case(&doc, "budget-window", events);
    assert_eq!(w["verdict"]["answer"], "unobservable", "{run}");
    assert!(
        w["verdict"]["reason"]
            .to_string()
            .contains("not present throughout"),
        "{run}"
    );
}

/// A TCP proxy in front of `upstream` (`tcp/127.0.0.1:<port>`) that a case
/// can cut: every forwarded connection closed at once, new ones forwarded
/// again. What zenctl connects through when a case closes its own
/// transport (core §2.7, 0.27).
struct Proxy {
    endpoint: String,
    conns: Arc<std::sync::Mutex<Vec<tokio::task::AbortHandle>>>,
    /// While set, a new connection is closed at once, unforwarded: a
    /// partition (redundancy.v1 §12's step 2, as its §7 cuts R1 from R2).
    refused: Arc<std::sync::atomic::AtomicBool>,
    _accept: Task,
}

impl Proxy {
    async fn to(upstream: &str) -> Proxy {
        let target = upstream.trim_start_matches("tcp/").to_owned();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a proxy port");
        let endpoint = format!("tcp/{}", listener.local_addr().expect("its address"));
        let conns: Arc<std::sync::Mutex<Vec<tokio::task::AbortHandle>>> = Arc::default();
        let held = Arc::clone(&conns);
        let refused: Arc<std::sync::atomic::AtomicBool> = Arc::default();
        let refusing = Arc::clone(&refused);
        let accept = tokio::spawn(async move {
            while let Ok((mut down, _)) = listener.accept().await {
                if refusing.load(Ordering::SeqCst) {
                    continue;
                }
                let Ok(mut up) = tokio::net::TcpStream::connect(&target).await else {
                    continue;
                };
                let pump = tokio::spawn(async move {
                    let _ = tokio::io::copy_bidirectional(&mut down, &mut up).await;
                });
                held.lock().expect("not poisoned").push(pump.abort_handle());
            }
        });
        Proxy {
            endpoint,
            conns,
            refused,
            _accept: Task(accept),
        }
    }

    /// Cuts, and refuses every new connection until [`Proxy::heal`].
    fn partition(&self) {
        self.refused.store(true, Ordering::SeqCst);
        self.cut();
    }

    /// Forwards new connections again.
    fn heal(&self) {
        self.refused.store(false, Ordering::SeqCst);
    }

    /// Closes every connection forwarded so far.
    fn cut(&self) {
        for c in self.conns.lock().expect("not poisoned").drain(..) {
            c.abort();
        }
    }
}

/// Core §2.7 (0.27, #753): a stalled reader's window reads unobservable.
/// The clean window above — two sources every 200 ms, within the bound,
/// over a counted span longer than the retention, the owner present a
/// lease and the timeout after — read by a zenctl whose own transport
/// closes mid-window: zenctl connects through a proxy, which the case cuts
/// once the window has opened. What the router held for zenctl went with
/// the transport, so the window is not lossless whatever it heard, and the
/// case says so; zenctl's session saw its transport close, which is what a
/// router giving up on a stalled reader looks like at the reader.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn check_conform_reads_a_window_unobservable_when_its_own_transport_closes() {
    let mut bus = Bus::admin(None).await;
    let id = iface("pulse.v1");
    let mut b = ServiceBuilder::new(
        &bus.owners,
        ServiceConfig::new("host-a/pulse".parse().expect("an address")),
    );
    b.implement(Implementation::new(pulses()))
        .expect("implement");
    let writers: Vec<_> = ["a", "b"]
        .into_iter()
        .map(|src| {
            let v: Bindings = [("src".to_owned(), vec![src.to_owned()])].into();
            b.event_writer(&id, "events/pulses/{src}", &v)
                .expect("an event writer")
        })
        .collect();
    let _owner = b.start().await.expect("start");
    bus.keep(Task(tokio::spawn(async move {
        loop {
            for w in &writers {
                let _ = w.put("pulse").await;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })));
    wait_for(&bus, &["host-a/pulse"]).await;
    let proxy = Proxy::to(&bus.endpoint).await;
    let events = "events/pulses/{src}";
    let mut argv: Vec<String> = [
        "check",
        "conform",
        "host-a/pulse",
        "pulse.v1",
        "--for",
        "4",
        "--timeout",
        "1",
        "--lease",
        "1",
        "--format",
        "json",
        "-c",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    argv.push(proxy.endpoint.clone());
    // The window has opened once zenctl's subscription matches a probe
    // publisher on the event's keys.
    let probe = bus
        .owners
        .declare_publisher(
            "zk2/host-a/pulse/pulse.v1/events/pulses/probe/01hzzzzzzzzzzzzzzzzzzzzzzz",
        )
        .await
        .expect("a probe publisher");
    let deadline = Instant::now() + SETTLE;
    while probe.matching_status().await.expect("matching").matching() {
        assert!(Instant::now() < deadline, "a subscription is left over");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let home = bus.home.clone();
    let pending = tokio::task::spawn_blocking(move || run(&argv, &home, None, None));
    while !probe.matching_status().await.expect("matching").matching() {
        assert!(Instant::now() < deadline, "zenctl's window never opened");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    proxy.cut();
    let run = pending.await.expect("the zenctl runner");
    let doc = run.json();
    let w = conform_case(&doc, "budget-window", events);
    assert_eq!(w["verdict"]["answer"], "unobservable", "{run}");
    let reason = w["verdict"]["reason"].to_string();
    assert!(
        reason.contains("not shown to have lost no delivery")
            && reason.contains("of its own transport(s) close"),
        "{run}"
    );
}

// ─── config.v1 (#768, CC) ────────────────────────────────────────────────────
//
// The owner is `scenarios.md`'s: `lab/rf0` on `conformance/lifecycle.json`'s
// `rf`, through `ServiceBuilder::config_v1`, on the owner example's simulated
// device (`zenkey/examples/owner/config.rs`, compiled in as zenkey's own
// scenario suite compiles it). zenctl reads it and acts on it as a script
// would; each case cites the section it runs.

#[allow(dead_code)]
#[path = "../../zenkey/examples/owner/config.rs"]
mod sim;

/// One running `config.v1` owner, and what restarts it.
struct Rf {
    address: String,
    svc: Service,
    cfg: zenkey::config_v1::Configuration,
    device: Arc<sim::SimDevice>,
    startup: zenkey::config_v1::Startup,
}

impl Rf {
    async fn up(
        session: &zenoh::Session,
        address: &str,
        startup: zenkey::config_v1::Startup,
        device: Arc<sim::SimDevice>,
    ) -> Rf {
        let mut b = ServiceBuilder::new(
            session,
            ServiceConfig::new(address.parse().expect("an address")),
        );
        let cfg = b
            .config_v1(
                startup.clone(),
                Arc::clone(&device) as Arc<dyn zenkey::config_v1::Device>,
            )
            .await
            .expect("config.v1 declared");
        let svc = b.start().await.expect("the owner starts");
        Rf {
            address: address.to_owned(),
            svc,
            cfg,
            device,
            startup,
        }
    }

    /// `lab/rf0`'s `rf`, a persisted layer, the floor and the delay its
    /// device states for a reach `apply_at`.
    async fn fresh(session: &zenoh::Session, address: &str, floor_s: u64, apply_ms: u64) -> Rf {
        let startup = sim::rf_startup(false, floor_s, Duration::from_millis(apply_ms), true);
        let device = Arc::new(sim::SimDevice::new(&startup));
        Rf::up(session, address, startup, device).await
    }

    /// A restart (§2.12): the old instance goes, the device runs its
    /// startup values again, a new instance starts from them.
    async fn restart(self, session: &zenoh::Session) -> Rf {
        let events = self.cfg.published().await;
        self.svc.close().await.expect("the old instance closes");
        self.device.restart(&self.startup);
        let next = self.device.next_startup(&self.startup, events);
        Rf::up(session, &self.address, next, self.device).await
    }
}

/// What a run printed, as JSON, or null: what a retry loop reads.
fn parsed(r: &Run) -> Value {
    serde_json::from_str(&r.stdout).unwrap_or(Value::Null)
}

/// The JSON document a `--format json` run printed, its rows beside.
fn row_of<'a>(doc: &'a Value, kind: &str, key: &str, value: &str) -> &'a Value {
    doc["rows"]
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["row"] == kind && r[key] == value))
        .unwrap_or_else(|| panic!("a {kind} row with {key} = {value}: {doc}"))
}

/// The verbs end to end against one owner: a reading with §5's answers, a
/// hot set applied and read back, the owner's own refusals made here with
/// nothing sent, a sensitive value only from standard input and never
/// echoed, a reach change that asks first, then its pending reply,
/// confirmed with the compact status, persisted, and confirmed again
/// `not_found`; silence from a service that is not there, never a failure.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn config_verbs_read_change_and_end_a_change() {
    let mut bus = Bus::bare(None).await;
    let rf = Rf::fresh(&bus.owners, "lab/rf0", 2, 500).await;

    // config get: the premise, both documents fresh on the deployment's
    // word for the clock (one host), and every answer.
    let get = [
        "config",
        "get",
        "lab/rf0",
        "--clocks-synced",
        "--format",
        "json",
    ];
    let run = bus.until(&get, |r| r.code == 0).await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["premise"], "listed", "{run}");
    assert_eq!(doc["view"]["freshness"]["verdict"], "fresh", "{run}");
    assert_eq!(doc["pending"]["answer"], "no", "{run}");
    assert_eq!(doc["projection"]["answer"], "yes", "{run}");
    assert_eq!(row_of(&doc, "group", "group", "queue")["answer"], "no");
    assert_eq!(row_of(&doc, "group", "group", "radio")["reason"], "reach");
    assert_eq!(
        row_of(&doc, "parameter", "parameter", "fraction_ppm")["answer"],
        "yes"
    );
    assert_eq!(
        row_of(&doc, "parameter", "parameter", "pin")["reason"],
        "sensitive"
    );

    // A hot set: applied, the read-back after the apply (§2.9).
    let run = bus
        .zenctl(&[
            "config",
            "set",
            "lab/rf0",
            "duty",
            "fraction_ppm=20000",
            "--actor",
            "ops",
            "--request-id",
            "r-1",
            "--format",
            "json",
        ])
        .await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["answer"], "applied", "{run}");
    assert_eq!(doc["view"]["revision"], 1);
    assert_eq!(doc["checked"]["read"], "view");
    assert_eq!(doc["checked"]["class"], "hot");
    assert_eq!(
        (doc["actor"].as_str(), doc["request_id"].as_str()),
        (Some("ops"), Some("r-1"))
    );
    let run = bus.zenctl(&get).await;
    let doc = run.json();
    let p = row_of(&doc, "parameter", "parameter", "fraction_ppm");
    assert_eq!(
        p["answer"], "no",
        "a runtime change a restart would not keep: {run}"
    );

    // The owner's own refusals, made here against the view just read:
    // nothing is sent (§2.5).
    let run = bus
        .zenctl(&["config", "set", "lab/rf0", "queue", "tx_queue_len=5"])
        .await;
    exits(&run, 2);
    assert!(
        run.stderr.contains("refused here, nothing sent") && run.stderr.contains("unavailable"),
        "{run}"
    );
    let run = bus
        .zenctl(&["config", "set", "lab/rf0", "duty", "fraction_ppm=lots"])
        .await;
    exits(&run, 2);
    assert!(run.stderr.contains("fraction_ppm"), "{run}");

    // A sensitive value: never inline; from standard input, applied, and
    // echoed nowhere (§2.15).
    let run = bus
        .zenctl(&["config", "set", "lab/rf0", "sim", "pin=1234"])
        .await;
    exits(&run, 2);
    assert!(
        run.stderr.contains("pin=@-") && !run.stderr.contains("1234"),
        "{run}"
    );
    let run = bus
        .zenctl_stdin(
            &[
                "config", "set", "lab/rf0", "sim", "pin=@-", "--format", "json",
            ],
            "4321\n",
        )
        .await;
    exits(&run, 0);
    assert!(
        !run.stdout.contains("4321") && !run.stderr.contains("4321"),
        "{run}"
    );
    assert_eq!(run.json()["request"]["values"]["pin"], "<redacted>");
    assert_eq!(
        rf.device.holds("sim", "pin"),
        Some(zenkey::config_v1::Value::Text("4321".into()))
    );

    // A reach change asks first (§2.4): from a script, --i-know.
    let reach = [
        "config",
        "set",
        "lab/rf0",
        "radio",
        "frequency_hz=869525000",
        "--confirm-s",
        "30",
        "--format",
        "json",
    ];
    let run = bus.zenctl(&reach).await;
    exits(&run, 2);
    assert!(
        run.stderr.contains("--i-know") && run.stderr.contains("not sent"),
        "{run}"
    );
    let mut sent = reach.to_vec();
    sent.push("--i-know");
    let run = bus.zenctl(&sent).await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["answer"], "pending", "{run}");
    let token = doc["pending"]["token"]
        .as_str()
        .expect("a token")
        .to_owned();
    assert!(
        run.stderr.contains(&format!("--token {token}")),
        "the hint names how to ask whether it landed: {run}"
    );
    // Not yet landed, then applied: confirmed with the compact status.
    let run = bus
        .until(
            &[
                "config",
                "get",
                "lab/rf0",
                "--clocks-synced",
                "--token",
                &token,
                "--format",
                "json",
            ],
            |r| r.code == 0 && parsed(r)["pending_change"]["applied"] == true,
        )
        .await;
    let doc = run.json();
    assert_eq!(doc["change"]["landed"]["answer"], "not_yet", "{run}");
    let run = bus
        .zenctl(&[
            "config", "confirm", "lab/rf0", &token, "--status", "--format", "json",
        ])
        .await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["answer"], "status", "{run}");
    assert_eq!(doc["status"]["last_change"]["token"], token.as_str());
    assert_eq!(doc["request"]["answer"], "status");
    // Persisted: the view after it, at source overlay (§2.13).
    let run = bus
        .zenctl(&["config", "persist", "lab/rf0", &token, "--format", "json"])
        .await;
    exits(&run, 0);
    let doc = run.json();
    let radio = &doc["view"]["groups"][1]["parameters"][0];
    assert_eq!(radio["source"], "overlay", "{run}");
    // A confirm retried finds nothing pending: refused, the act's 1.
    let run = bus
        .zenctl(&["config", "confirm", "lab/rf0", &token, "--format", "json"])
        .await;
    exits(&run, 1);
    assert_eq!(run.json()["refusal"]["code"], "not_found", "{run}");
    let run = bus
        .zenctl(&["config", "extend", "lab/rf0", "nope", "--confirm-s", "5"])
        .await;
    exits(&run, 1);

    // A service that is not there: no document to check against, typed by
    // spelling, and silence — never a failure (§5, core O5).
    let run = bus
        .zenctl(&[
            "config",
            "set",
            "lab/zz",
            "duty",
            "fraction_ppm=1",
            "--timeout",
            "1",
            "--format",
            "json",
        ])
        .await;
    exits(&run, 2);
    let doc = run.json();
    assert_eq!(doc["answer"], "silent", "{run}");
    assert_eq!(doc["checked"]["typed_by"], "spelling");
    let run = bus
        .zenctl(&[
            "config",
            "get",
            "lab/zz",
            "--timeout",
            "1",
            "--format",
            "json",
        ])
        .await;
    exits(&run, 2);
    assert_eq!(run.json()["premise"], "absent", "{run}");
    bus.keep(rf.svc);
}

/// `scenarios.md` §13, the compact status across a constrained face: the
/// caller is a client of R1 with the face's `@zk` deny on it and the
/// subscription to the owner's state denied, so it binds statically
/// (`--across-face`) and reads no presence. It sizes its window from the
/// status, gets a reach set's pending reply in at most 160 bytes and the
/// confirm's status in at most 400, and learns of a restart from the
/// status's `instance` alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scenario_13_the_compact_status_across_a_constrained_face() {
    static NTH: AtomicU64 = AtomicU64::new(0);
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "live-zk2-face-{}-{}",
        std::process::id(),
        NTH.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&scratch).unwrap();
    let dict = scratch.join("usrpwd.txt");
    std::fs::write(&dict, "router:router-pw\nowner:owner-pw\nfar:far-pw\n").unwrap();
    // The face, on the far principal's policy: the `@zk` deny (§8.5), and
    // the subscription to the owner's state, whose GET crosses.
    let acl = r#"access_control: {
    enabled: true,
    default_permission: "allow",
    rules: [
      { id: "face-zk", permission: "deny", flows: ["ingress", "egress"],
        messages: ["put", "delete", "declare_subscriber", "query", "reply", "liveliness_token",
                   "declare_liveliness_subscriber", "liveliness_query"],
        key_exprs: ["zk2/*/*/@zk/**"] },
      { id: "face-state-push", permission: "deny", flows: ["ingress", "egress"],
        messages: ["declare_subscriber", "put", "delete"],
        key_exprs: ["zk2/lab/rf0/*/state/**"] },
    ],
    subjects: [{ id: "far", usernames: ["far"] }],
    policies: [{ id: "far", rules: ["face-zk", "face-state-push"], subjects: ["far"] }],
  },"#;
    let text = format!(
        "{{\n  mode: \"router\",\n  scouting: {{ multicast: {{ enabled: false }}, gossip: {{ enabled: false }} }},\n  \
         listen: {{ endpoints: [\"tcp/127.0.0.1:0\"] }},\n  adminspace: {{ enabled: false }},\n  \
         transport: {{ auth: {{ usrpwd: {{ user: \"router\", password: \"router-pw\", dictionary_file: {:?} }} }} }},\n  {acl}\n}}\n",
        dict.display().to_string()
    );
    let router = zenoh::open(zenoh::Config::from_json5(&text).expect("the router config"))
        .await
        .expect("a router with the face");
    let endpoint = router
        .info()
        .locators()
        .await
        .into_iter()
        .map(|l| l.to_string())
        .find(|l| l.starts_with("tcp/127.0.0.1:"))
        .expect("the router listens on loopback");
    let mut c = base_config();
    c.insert_json5("mode", "\"client\"").unwrap();
    c.insert_json5("connect/endpoints", &format!("[\"{endpoint}\"]"))
        .unwrap();
    c.insert_json5(
        "transport/auth/usrpwd",
        r#"{ user: "owner", password: "owner-pw" }"#,
    )
    .unwrap();
    let owners = zenoh::open(c).await.expect("the owner's session");
    let mut bus = Bus::with_owners(router, endpoint, owners);
    let far = scratch.join("far.json5");
    std::fs::write(
        &far,
        r#"{ transport: { auth: { usrpwd: { user: "far", password: "far-pw" } } } }"#,
    )
    .unwrap();
    let far = far.display().to_string();
    let rf = Rf::fresh(&bus.owners, "lab/rf0", 10, 1000).await;
    let face = ["--across-face", "--zenoh-config", far.as_str()];
    let with = |args: &[&str]| -> Vec<String> {
        args.iter()
            .chain(face.iter())
            .map(|s| s.to_string())
            .collect()
    };

    // 1. C GETs St: the floor it sizes its window from, and nothing of `@zk`.
    let status = with(&["config", "get", "lab/rf0", "--status", "--format", "json"]);
    let status: Vec<&str> = status.iter().map(String::as_str).collect();
    let run = bus.until(&status, |r| r.code == 0).await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["premise"], "across_face", "{run}");
    assert!(
        doc.get("presence").is_none(),
        "no presence read across a face"
    );
    assert_eq!(doc["status"]["value"]["confirm_floor_s"], 10, "{run}");
    assert!(
        doc["asked"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| !a.as_str().unwrap().contains("@zk")),
        "{run}"
    );
    let first = doc["status"]["value"]["instance"]
        .as_str()
        .unwrap()
        .to_owned();
    // The face denies `@zk`: a presence read from C is complete and empty.
    let run = bus
        .zenctl(&[
            "service",
            "list",
            "--zenoh-config",
            &far,
            "--format",
            "json",
        ])
        .await;
    assert!(
        !run.stdout.contains("lab/rf0"),
        "no @zk delivery crosses the face: {run}"
    );

    // 2. A reach set: the pending reply, at most 160 bytes of payload.
    let set = with(&[
        "config",
        "set",
        "lab/rf0",
        "radio",
        "frequency_hz=869525000",
        "--confirm-s",
        "120",
        "--i-know",
        "--format",
        "json",
    ]);
    let set: Vec<&str> = set.iter().map(String::as_str).collect();
    let run = bus.zenctl(&set).await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["answer"], "pending", "{run}");
    assert_eq!(
        doc["checked"]["read"], "status",
        "the status, not the view, read first"
    );
    let reply = json!({"pending": doc["pending"]});
    let size = serde_json::to_vec(&reply).unwrap().len();
    assert!(size <= 160, "the pending reply is {size} bytes: {reply}");
    let token = doc["pending"]["token"].as_str().unwrap().to_owned();

    // 3. Once St shows it applied, confirm with answer = "status": at most
    //    400 bytes, the status and not the view.
    let run = bus
        .until(&status, |r| {
            r.code == 0 && parsed(r)["status"]["value"]["pending"]["applied"] == true
        })
        .await;
    exits(&run, 0);
    let confirm = with(&[
        "config", "confirm", "lab/rf0", &token, "--status", "--format", "json",
    ]);
    let confirm: Vec<&str> = confirm.iter().map(String::as_str).collect();
    let run = bus.zenctl(&confirm).await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["answer"], "status", "{run}");
    let reply = json!({"status": doc["status"]});
    let size = serde_json::to_vec(&reply).unwrap().len();
    assert!(size <= 400, "the confirm's status is {size} bytes: {reply}");
    assert_eq!(doc["status"]["last_change"]["token"], token.as_str());

    // 4. The owner restarts: St's instance differs, which C learns without
    //    presence (§2.12).
    let rf = rf.restart(&bus.owners).await;
    let run = bus
        .until(&status, |r| {
            r.code == 0 && parsed(r)["status"]["value"]["instance"] != first.as_str()
        })
        .await;
    exits(&run, 0);
    assert_eq!(run.json()["status"]["value"]["revision"], 0, "{run}");
    bus.keep(rf.svc);
    let _ = std::fs::remove_dir_all(&scratch);
}

/// `scenarios.md` §14, a tool's answers (§5): "is a change pending?" no of
/// an idle owner and not asked of a service that does not implement
/// config.v1; yes of an armed reach change, which has not landed yet; once
/// it rolls back at its deadline, it did not land and its undo was clean;
/// a value set without a window will not survive a restart; and after the
/// owner stops, not asked on presence's word, and, for a reader that reads
/// no presence and holds the status it last heard, unobservable, the
/// status stale — never no.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scenario_14_a_tools_answers() {
    let mut bus = Bus::bare(None).await;
    let rf = Rf::fresh(&bus.owners, "lab/rf0", 2, 500).await;
    let plain = bring_up(
        &bus.owners,
        ServiceConfig::new("lab/plain".parse().unwrap()),
        &["tcgui/tc.netif.v1"],
    )
    .await;
    let get = |addr: &'static str| -> Vec<&'static str> {
        vec!["config", "get", addr, "--clocks-synced", "--format", "json"]
    };

    // 1. Idle: no; lab/plain: not asked, its descriptor not listing config.v1.
    let run = bus.until(&get("lab/rf0"), |r| r.code == 0).await;
    assert_eq!(run.json()["pending"]["answer"], "no", "{run}");
    let run = bus
        .until(&get("lab/plain"), |r| parsed(r)["premise"] == "not_listed")
        .await;
    let doc = run.json();
    assert_eq!(doc["pending"]["answer"], "not_asked", "{run}");
    assert_eq!(doc["pending"]["reason"], "not_listed");
    exits(&run, 2);

    // 2. A reach change armed: pending yes, with the token and the
    //    deadline; did it land: not yet.
    let run = bus
        .zenctl(&[
            "config",
            "set",
            "lab/rf0",
            "radio",
            "frequency_hz=869525000",
            "--confirm-s",
            "3",
            "--i-know",
            "--format",
            "json",
        ])
        .await;
    exits(&run, 0);
    let doc = run.json();
    let token = doc["pending"]["token"].as_str().unwrap().to_owned();
    let instance = doc["checked"]["instance"].as_str().unwrap().to_owned();
    let asked = [
        "config",
        "get",
        "lab/rf0",
        "--clocks-synced",
        "--token",
        token.as_str(),
        "--instance",
        instance.as_str(),
        "--format",
        "json",
    ];
    let run = bus.zenctl(&asked).await;
    let doc = run.json();
    assert_eq!(doc["pending"]["answer"], "yes", "{run}");
    assert_eq!(doc["pending_change"]["token"], token.as_str());
    assert!(doc["pending_change"]["deadline"].is_string());
    assert_eq!(doc["change"]["landed"]["answer"], "not_yet", "{run}");

    // 3. It rolls back at its deadline: a window across it hears the event.
    //    Did it land: no, rolled-back; was its undo clean: yes.
    let mut window = asked.to_vec();
    window.extend(["--for", "5"]);
    let run = bus.zenctl(&window).await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["change"]["landed"]["answer"], "no", "{run}");
    assert_eq!(doc["change"]["landed"]["reason"], "rolled-back", "{run}");
    assert_eq!(doc["change"]["undo_clean"]["answer"], "yes", "{run}");
    assert_eq!(doc["change"]["event"]["outcome"], "rolled-back");

    // 4. duty set without a window: will fraction_ppm survive a restart?
    //    No, it runs a value with startup 10000.
    let run = bus
        .zenctl(&["config", "set", "lab/rf0", "duty", "fraction_ppm=20000"])
        .await;
    exits(&run, 0);
    let run = bus.zenctl(&get("lab/rf0")).await;
    let doc = run.json();
    let p = row_of(&doc, "parameter", "parameter", "fraction_ppm");
    assert_eq!(p["answer"], "no", "{run}");
    assert!(p["says"].as_str().unwrap().contains("10000"), "{run}");

    // 5. A reader that reads no presence holds the status it last heard:
    //    its window hears a put (a change made after it opened), then the
    //    owner stops; at the window's end the status is stale.
    let held = bus.spawn_within(
        &[
            "config",
            "get",
            "lab/rf0",
            "--status",
            "--across-face",
            "--for",
            "72",
            "--format",
            "json",
        ],
        Duration::from_secs(150),
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    let run = bus
        .zenctl(&["config", "set", "lab/rf0", "duty", "fraction_ppm=30000"])
        .await;
    exits(&run, 0);
    tokio::time::sleep(Duration::from_millis(500)).await;
    rf.svc.close().await.expect("the owner stops");
    // Within 60 s: not asked, the owner absent (presence's word).
    let run = bus
        .until(&get("lab/rf0"), |r| parsed(r)["premise"] == "absent")
        .await;
    let doc = run.json();
    assert_eq!(doc["pending"]["answer"], "not_asked", "{run}");
    assert_eq!(doc["pending"]["reason"], "absent");
    // After it: unobservable, the status stale — never no.
    let run = held.await.expect("the held reading");
    let doc = run.json();
    assert_eq!(doc["status"]["freshness"]["verdict"], "stale", "{run}");
    assert_eq!(doc["pending"]["answer"], "unobservable", "{run}");
    exits(&run, 0);
    bus.keep(plain);
}

/// The doctor's and `check conform`'s config.v1 checks against a live
/// owner (#768, CC): the suite's probes refused as the text names them, the
/// status its view's projection, both documents fresh on the deployment's
/// word, a dry run of `duty`'s running value; and, after a hot set, the
/// doctor's `config-unpersisted` naming the value a restart would not keep,
/// beside its clean checks.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_doctor_and_check_conform_judge_a_config_owner() {
    let mut bus = Bus::bare(None).await;
    let rf = Rf::fresh(&bus.owners, "lab/rf0", 2, 500).await;
    bus.until(
        &[
            "config",
            "get",
            "lab/rf0",
            "--clocks-synced",
            "--format",
            "json",
        ],
        |r| r.code == 0,
    )
    .await;
    let run = bus
        .zenctl(&[
            "check",
            "conform",
            "lab/rf0",
            "config.v1",
            "--for",
            "2",
            "--timeout",
            "2",
            "--clocks-synced",
            "--calls-granted",
            "--format",
            "json",
        ])
        .await;
    let doc = run.json();
    let case = |id: &str, subject: &str| -> Value {
        rows_of(&doc, "case")
            .into_iter()
            .find(|c| c["case"] == id && c["subject"] == subject)
            .cloned()
            .unwrap_or_else(|| panic!("{id} {subject}: {run}"))
    };
    let passed = |id: &str, subject: &str| {
        let c = case(id, subject);
        assert_eq!(
            c["verdict"]["answer"], "not_established",
            "{id} {subject}: {c}\n{run}"
        );
    };
    passed("config-projection", "service");
    passed("config-fresh", "state/view");
    passed("config-fresh", "state/status");
    passed("config-view", "state/view");
    for probe in [
        "@op/set/zk2-conform-none",
        "@op/set/queue",
        "@op/set/*",
        "@op/confirm",
        "@op/cancel",
        "@op/extend",
        "@op/persist",
    ] {
        passed("config-refusal", probe);
    }
    passed("config-dry-run", "@op/set/duty");
    // The probes changed nothing: the revision is still 0.
    assert_eq!(rf.cfg.view().await.revision, 0);

    let run = bus
        .zenctl(&["config", "set", "lab/rf0", "duty", "fraction_ppm=20000"])
        .await;
    exits(&run, 0);
    let mut args = doctor_args(&["--clocks-synced", "--format", "json"]);
    for c in [
        "config-projection",
        "config-overdue",
        "config-unpersisted",
        "config-desired",
    ] {
        args.extend(["--check", c]);
    }
    let run = bus.zenctl(&args).await;
    let doc = run.json();
    let f = &check_row(&doc, "config-unpersisted")["findings"][0];
    assert_eq!(
        (f["subject"].as_str(), f["severity"].as_str()),
        (Some("lab/rf0 duty.fraction_ppm"), Some("info")),
        "{run}"
    );
    for id in ["config-projection", "config-overdue", "config-desired"] {
        assert_eq!(
            check_row(&doc, id)["verdict"]["answer"],
            "not_established",
            "{id}: {run}"
        );
    }
    assert_eq!(doc["scope"]["config"]["services"], 1, "{run}");
    // An info finding is below the default floor: the run is clean.
    exits(&run, 0);
    bus.keep(rf.svc);
}

// ─── alarms.v1: scenarios.md §8–§10, the tool's sections (#800, AC) ────────
//
// Common setup, the text's: R1 a router with timestamping on, owners and the
// tool T its clients. Owners are the runtime's (`zenkey::alarms`), but for
// §9's liar, whose documents only a raw writer puts. T is `zenctl alarms`,
// and in §8 the watchdog's `alert-firing` rule too; T takes the
// deployment's word for its clock (`--clocks-synced`: every session runs on
// this host, the conventions).

/// The alarm A of the scenarios' conventions: rule `link_down`, labels
/// `{peer: r2, port: eth0}`, at `severity`.
fn alarm_a(severity: zenkey::alarms::Severity) -> zenkey::alarms::Condition {
    zenkey::alarms::Condition::new("link_down", severity, "link to r2 down on eth0")
        .label("peer", "r2")
        .label("port", "eth0")
}

/// A's alert key (alarms.v1 §2.2).
const A_KEY: &str = "a659f813308ad1da";

/// An owner of `alarms.v1` at `address` through the runtime, started, on
/// `session`; its alarms in the tokenless set when `tokenless`.
async fn alarms_owner(
    session: &zenoh::Session,
    address: &str,
    tokenless: bool,
) -> (Service, zenkey::alarms::Alarms) {
    let mut cfg = ServiceConfig::new(address.parse().expect("an address"));
    if tokenless {
        cfg = cfg.tokenless(zenkey::alarms::iface());
    }
    let mut b = ServiceBuilder::new(session, cfg);
    let a = b
        .alarms(zenkey::alarms::Startup::default())
        .await
        .expect("alarms.v1");
    (b.start().await.expect("start"), a)
}

/// `alarms`'s run, its document's row of the alarm `alert_key` of
/// `address`, when it parses.
fn alarm_of(run: &Run, address: &str, alert_key: &str) -> Option<Value> {
    let doc: Value = serde_json::from_str(&run.stdout).ok()?;
    let (system, service) = address.split_once('/')?;
    rows_of(&doc, "alarm")
        .into_iter()
        .find(|r| {
            r["alarm"]["system"] == system
                && r["alarm"]["service"] == service
                && r["alarm"]["alert_key"] == alert_key
        })
        .cloned()
}

/// `alarms`'s owner row of `address`.
fn alarm_owner<'a>(doc: &'a Value, address: &str) -> &'a Value {
    rows_of(doc, "owner")
        .into_iter()
        .find(|r| r["address"] == address)
        .unwrap_or_else(|| panic!("{address} has an owner row: {doc}"))
}

/// The alarms' wire selector T reads, every owner of `lab`.
const LAB: &str = "zk2/lab/*/alarms.v1/state/alarms/*";

/// alarms.v1 `scenarios.md` §8 (§2.7, §2.9, §2.14, §2.15): an absent
/// owner, last-known, for a tool. T reads `lab/svc` holding A firing: yes,
/// `firing`, and the watchdog's `alert-firing` fires. The owner closes
/// cleanly: S receives no resolved put and no delete of A (§2.9). Then a
/// complete presence read holds no token of it and its GET draws no reply:
/// T judges A not asked, `absent`, showing the archive's document firing as
/// last-known with its stamp, never as firing now; "is anything firing at
/// or above warning?" over `lab` counts the owner not asked and is not yes.
/// T knows of the owner from the archive's keys, and the watchdog from its
/// own earlier reading: both keep it in scope though no token of it is
/// found (§2.15, F-131), and the rule goes unobservable, never ok.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn alarms_s8_an_absent_owner_last_known_for_a_tool() {
    use zenkey::alarms::Severity;
    let bus = Bus::bare(None).await;
    let key = format!("zk2/lab/svc/alarms.v1/state/alarms/{A_KEY}");
    let archiving = client(&bus.endpoint, None).await;
    let archive = Archive::start(
        &archiving,
        ArchiveConfig {
            service: ServiceConfig::new("lab/archive".parse().expect("an address")),
            records: vec![Recorded {
                owner: "lab/svc".parse().expect("an address"),
                selector: "zk2/lab/svc/alarms.v1/state/**".to_owned(),
                implementation: zenkey::alarms::implementation(),
            }],
            peers: Vec::new(),
            unconfirmed_horizon: None,
            watch: None,
        },
    )
    .await
    .expect("an archive");
    let owning = client(&bus.endpoint, None).await;
    let (svc, alarms) = alarms_owner(&owning, "lab/svc", false).await;
    wait_for(&bus, &["lab/svc", "lab/archive"]).await;
    alarms
        .raise(alarm_a(Severity::Warning))
        .await
        .expect("A raised");
    let deadline = Instant::now() + SETTLE;
    while archive.confirmed(&key).is_none() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(archive.confirmed(&key).is_some(), "the archive records A");

    // Step 1: T reads the owner and judges A.
    let args = [
        "alarms",
        LAB,
        "--clocks-synced",
        "--timeout",
        "1",
        "--format",
        "json",
    ];
    let run = bus.until(&args, |r| r.code == 1).await;
    exits(&run, 1);
    let doc = run.json();
    assert_eq!(doc["report"], "alarms");
    assert_eq!(doc["any_firing"]["answer"], "yes", "{run}");
    let a = alarm_of(&run, "lab/svc", A_KEY).expect("A's row");
    assert_eq!(a["firing"]["answer"], "yes", "{run}");
    assert_eq!(a["firing"]["reason"], "firing");
    assert_eq!(a["rederives"]["reason"], "strict");
    assert_eq!(a["latest"]["document"]["state"], "firing");

    // The watchdog's alert-firing holds the same reader live across steps
    // 2 and 3: it knows the owner from its own earlier reading.
    let watchdog = bus.spawn(&[
        "watchdog",
        "--rule",
        "alert-firing zk2/lab/** warning",
        "--every",
        "1",
        "--count",
        "18",
        "--clocks-synced",
        "--timeout",
        "1",
    ]);
    // Its reader opens and its first ticks judge the owner holding A.
    tokio::time::sleep(Duration::from_secs(8)).await;

    // Step 2: the owner closes cleanly; S hears no resolve and no delete.
    let listening = client(&bus.endpoint, None).await;
    let heard: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let h = Arc::clone(&heard);
    let s = listening
        .declare_subscriber("zk2/lab/svc/alarms.v1/**")
        .callback(move |x: zenoh::sample::Sample| {
            let what = match x.kind() {
                zenoh::sample::SampleKind::Delete => "delete".to_owned(),
                zenoh::sample::SampleKind::Put => {
                    String::from_utf8_lossy(&x.payload().to_bytes()).into_owned()
                }
            };
            h.lock().unwrap().push(what);
        })
        .await
        .expect("S");
    drop(alarms);
    svc.close().await.expect("the owner closes");
    owning.close().await.expect("its session ends");
    tokio::time::sleep(Duration::from_secs(3)).await;
    let seen = heard.lock().unwrap().clone();
    assert!(
        seen.iter()
            .all(|x| x != "delete" && !x.contains("\"resolved\"")),
        "a clean close resolves and deletes nothing (§2.9): {seen:?}"
    );
    drop(s);

    // Step 3: a complete presence read, the owner's GET, the archive's form.
    let run = bus
        .until(&args, |r| {
            alarm_of(r, "lab/svc", A_KEY).is_some_and(|a| a["firing"]["reason"] == "absent")
        })
        .await;
    exits(&run, 2);
    let doc = run.json();
    assert_eq!(doc["presence"]["complete"], true, "{run}");
    let a = alarm_of(&run, "lab/svc", A_KEY).expect("A's row");
    assert_eq!(a["firing"]["answer"], "not_asked", "{run}");
    assert!(a.get("latest").is_none(), "never shown as current: {run}");
    let lk = &a["last_known"];
    assert_eq!(lk["archive"], "lab/archive", "{run}");
    assert_eq!(lk["document"]["state"], "firing");
    assert!(lk["stamp"]["clock"].is_string(), "its stamp: {run}");
    let owner = alarm_owner(&doc, "lab/svc");
    assert_eq!(owner["verdict"], "not_asked", "{run}");
    assert_eq!(owner["reason"], "absent");
    assert_eq!(owner["known_by"], json!(["archive"]), "{run}");
    assert_ne!(doc["any_firing"]["answer"], "yes", "{run}");
    assert_eq!(
        doc["any_firing"]["not_asked"], 2,
        "the archive, and the absent owner: {run}"
    );
    let table = bus
        .zenctl(&[
            "alarms",
            LAB,
            "--clocks-synced",
            "--timeout",
            "1",
            "--format",
            "table",
        ])
        .await;
    assert!(
        table.stdout.contains("last-known, never current"),
        "{table}"
    );

    // The watchdog: firing while the owner held A, then never ok.
    let run = watchdog.await.expect("the watchdog");
    let lines = ndjson_lines(&run);
    let to: Vec<&str> = lines
        .iter()
        .filter(|l| l["row"] == "transition")
        .filter_map(|l| l["to"].as_str())
        .collect();
    assert_eq!(to.first(), Some(&"firing"), "{run}");
    assert_eq!(to.last(), Some(&"unobservable"), "{run}");
    assert!(!to.contains(&"ok"), "an absent owner is never ok: {run}");
    drop(archive);
}

/// alarms.v1 `scenarios.md` §9 (§2.2, §2.3, §5): a key that does not
/// re-derive. `lab/liar` puts, through raw state writers on keys of its
/// choosing, A's document on `…/0000000000000001`, a document with rule
/// `link\ndown` and labels `{port: 22}` on its sanitized key
/// `…/16991d91fb818ca6`, and A's document on its own key. T asks of each
/// whether it re-derives: no, `mismatch`; yes, `sanitized`; yes, `strict`.
/// The doctor reports the first as an error and the second as a warning.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn alarms_s9_a_key_that_does_not_rederive_for_a_tool() {
    let mut bus = Bus::bare(None).await;
    let id = zenkey::alarms::iface();
    let mut b = ServiceBuilder::new(
        &bus.owners,
        ServiceConfig::new("lab/liar".parse().expect("an address")),
    );
    b.implement(zenkey::alarms::implementation())
        .expect("implement");
    let a = json!({"timestamp": 1_760_950_000_000_u64, "rule": "link_down",
        "severity": "warning", "state": "firing", "summary": "link to r2 down on eth0",
        "labels": {"peer": "r2", "port": "eth0"}});
    let unframed = json!({"timestamp": 1_760_950_000_000_u64, "rule": "link\ndown",
        "severity": "warning", "state": "firing", "summary": "s", "labels": {"port": "22"}});
    let sanitized = zenkey::model::alarms::derive("link\ndown", [("port", "22")]).key;
    assert_eq!(sanitized, "16991d91fb818ca6", "the text's sanitized key");
    let mut writers = Vec::new();
    for (k, doc) in [
        ("0000000000000001", &a),
        (sanitized.as_str(), &unframed),
        (A_KEY, &a),
    ] {
        let w = b
            .declare_state_writer(
                &id,
                zenkey::alarms::ALARMS,
                &[("alert_key".to_owned(), vec![k.to_owned()])].into(),
            )
            .await
            .expect("a writer");
        w.put(serde_json::to_vec(doc).expect("json"))
            .await
            .expect("a put");
        writers.push(w);
    }
    bus.services.push(b.start().await.expect("start"));
    bus.keep(writers);
    wait_for(&bus, &["lab/liar"]).await;

    let args = [
        "alarms",
        "zk2/lab/liar/**",
        "--clocks-synced",
        "--timeout",
        "1",
        "--format",
        "json",
    ];
    let run = bus
        .until(&args, |r| {
            [("0000000000000001"), (sanitized.as_str()), (A_KEY)]
                .iter()
                .all(|k| alarm_of(r, "lab/liar", k).is_some())
        })
        .await;
    let row = |k: &str| alarm_of(&run, "lab/liar", k).unwrap_or_else(|| panic!("{k}: {run}"));
    for (k, answer, reason) in [
        ("0000000000000001", "no", "mismatch"),
        (sanitized.as_str(), "yes", "sanitized"),
        (A_KEY, "yes", "strict"),
    ] {
        let r = row(k);
        assert_eq!(
            (
                r["rederives"]["answer"].as_str(),
                r["rederives"]["reason"].as_str()
            ),
            (Some(answer), Some(reason)),
            "{k}: {run}"
        );
    }
    assert_eq!(
        row(&sanitized)["problems"],
        json!(["unframed"]),
        "§2.3's SHOULD, named: {run}"
    );

    // The doctor: the mismatch an error, the sanitized key a warning.
    let run = bus
        .zenctl(&doctor_args(&["--check", "alarm-key", "--format", "json"]))
        .await;
    exits(&run, 1);
    let doc = run.json();
    let findings = check_row(&doc, "alarm-key")["findings"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let by: Vec<(String, String)> = findings
        .iter()
        .map(|f| {
            (
                f["subject"]
                    .as_str()
                    .unwrap_or_default()
                    .rsplit('/')
                    .next()
                    .unwrap_or_default()
                    .to_owned(),
                f["severity"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        by,
        [
            ("0000000000000001".to_owned(), "error".to_owned()),
            (sanitized.clone(), "warning".to_owned()),
        ],
        "{run}"
    );
}

/// The owners' tokens under `lab/b`, read once: what T would find by token.
async fn alive_alarms_tokens(bus: &Bus, address: &str) -> Vec<String> {
    let read = zenkey::presence::liveliness_read(
        &bus.owners,
        &format!("zk2/{address}/@zk/alive/alarms.v1/**"),
        Duration::from_secs(1),
    )
    .await
    .expect("a liveliness read");
    read.keys
}

/// alarms.v1 `scenarios.md` §10's two owners: `lab/a` holding A firing at
/// warning, and `lab/b`, holding nothing, with `alarms.v1` in its tokenless
/// set (core §8.1).
async fn s10_owners(bus: &mut Bus) -> zenkey::alarms::Alarms {
    use zenkey::alarms::Severity;
    let (a, alarms_a) = alarms_owner(&bus.owners, "lab/a", false).await;
    alarms_a
        .raise(alarm_a(Severity::Warning))
        .await
        .expect("A raised");
    let (b, alarms_b) = alarms_owner(&bus.owners, "lab/b", true).await;
    bus.services.extend([a, b]);
    bus.keep(alarms_a);
    wait_for(bus, &["lab/a", "lab/b"]).await;
    assert!(
        alive_alarms_tokens(bus, "lab/b").await.is_empty(),
        "lab/b holds no alive/alarms.v1/… token"
    );
    alarms_b
}

/// §10's step 1 at `at_least`: T subscribes, then GETs, and asks at once.
async fn s10_step1(bus: &Bus, at_least: &str) -> Run {
    bus.zenctl(&[
        "alarms",
        LAB,
        "--for",
        "1",
        "--at-least",
        at_least,
        "--clocks-synced",
        "--timeout",
        "1",
        "--format",
        "json",
    ])
    .await
}

/// alarms.v1 `scenarios.md` §10 (§2.15, §5), steps 1 and 3: anything
/// firing, for a tool. At warning: yes, `any_firing`, 1. At critical:
/// unobservable, `owner_unobservable` — `lab/a` is established
/// (`answered`), `lab/b` is not (`window_short`): its GET drew nothing,
/// which alone is no verdict, and its window is not yet longer than 900 s.
/// T finds `lab/b` by its descriptor though it holds no `alive/alarms.v1/…`
/// token (§2.13). Then `lab/b` raises B at critical: yes, `any_firing`, 1.
/// Step 2, 905 s of listening, is the slow tier's
/// ([`alarms_s10_anything_firing_for_a_tool_at_900_s`]).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn alarms_s10_anything_firing_for_a_tool() {
    use zenkey::alarms::Severity;
    let mut bus = Bus::bare(None).await;
    let alarms_b = s10_owners(&mut bus).await;

    // Step 1, at warning and at critical.
    let run = s10_step1(&bus, "warning").await;
    exits(&run, 1);
    let doc = run.json();
    assert_eq!(doc["any_firing"]["answer"], "yes", "{run}");
    assert_eq!(doc["any_firing"]["reason"], "any_firing");
    assert_eq!(doc["any_firing"]["firing"], 1);
    let run = s10_step1(&bus, "critical").await;
    exits(&run, 2);
    let doc = run.json();
    assert_eq!(
        (
            doc["any_firing"]["answer"].as_str(),
            doc["any_firing"]["reason"].as_str()
        ),
        (Some("unobservable"), Some("owner_unobservable")),
        "{run}"
    );
    let a = alarm_owner(&doc, "lab/a");
    assert_eq!(
        (a["verdict"].as_str(), a["reason"].as_str()),
        (Some("established"), Some("answered")),
        "{run}"
    );
    let b = alarm_owner(&doc, "lab/b");
    assert_eq!(
        (b["verdict"].as_str(), b["reason"].as_str()),
        (Some("unobservable"), Some("window_short")),
        "{run}"
    );
    assert_eq!(
        b["known_by"],
        json!(["presence"]),
        "by its descriptor: {run}"
    );

    // Step 3: lab/b raises B at critical.
    alarms_b
        .raise(
            zenkey::alarms::Condition::new("disk_full", Severity::Critical, "/var is 97% full")
                .label("mount", "/var"),
        )
        .await
        .expect("B raised");
    let args = [
        "alarms",
        LAB,
        "--at-least",
        "critical",
        "--clocks-synced",
        "--timeout",
        "1",
        "--format",
        "json",
    ];
    let run = bus.until(&args, |r| r.code == 1).await;
    exits(&run, 1);
    let doc = run.json();
    assert_eq!(doc["any_firing"]["reason"], "any_firing", "{run}");
    assert_eq!(doc["any_firing"]["firing"], 1, "{run}");
    bus.keep(alarms_b);
}

/// alarms.v1 `scenarios.md` §10 at the standard contract, whole: step 1,
/// then step 2 — T listens 905 s, losing no delivery, both owners present
/// throughout, and asks again at critical: no, `none_firing`, `lab/b`
/// established by its window (`window`) — then step 3. The window counts
/// from the subscription's declaration plus the reply timeout, and the
/// owners are watched a lease and the timeout past it (core §2.7), so the
/// run takes about 917 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "slow tier: §10's step 2 listens 905 s at the contract's 900 s horizon"]
async fn alarms_s10_anything_firing_for_a_tool_at_900_s() {
    use zenkey::alarms::Severity;
    let mut bus = Bus::bare(None).await;
    let alarms_b = s10_owners(&mut bus).await;
    let run = s10_step1(&bus, "critical").await;
    exits(&run, 2);
    assert_eq!(
        alarm_owner(&run.json(), "lab/b")["reason"],
        "window_short",
        "{run}"
    );

    // Step 2: 905 s of listening.
    let run = bus
        .spawn_within(
            &[
                "alarms",
                LAB,
                "--for",
                "905",
                "--at-least",
                "critical",
                "--clocks-synced",
                "--timeout",
                "1",
                "--format",
                "json",
            ],
            Duration::from_secs(1000),
        )
        .await
        .expect("the runner");
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(
        (
            doc["any_firing"]["answer"].as_str(),
            doc["any_firing"]["reason"].as_str()
        ),
        (Some("no"), Some("none_firing")),
        "{run}"
    );
    let b = alarm_owner(&doc, "lab/b");
    assert_eq!(
        (b["verdict"].as_str(), b["reason"].as_str()),
        (Some("established"), Some("window")),
        "{run}"
    );

    // Step 3.
    alarms_b
        .raise(
            zenkey::alarms::Condition::new("disk_full", Severity::Critical, "/var is 97% full")
                .label("mount", "/var"),
        )
        .await
        .expect("B raised");
    let run = bus
        .until(
            &[
                "alarms",
                LAB,
                "--at-least",
                "critical",
                "--clocks-synced",
                "--timeout",
                "1",
                "--format",
                "json",
            ],
            |r| r.code == 1,
        )
        .await;
    exits(&run, 1);
    bus.keep(alarms_b);
}

/// Polls `session`'s liveliness read of `key` until it holds no token: the
/// router has told every subscriber it tells that the token went.
async fn until_gone(session: &zenoh::Session, key: &str) {
    let deadline = Instant::now() + SETTLE;
    while !zenkey::presence::liveliness_read(session, key, Duration::from_secs(1))
        .await
        .expect("a read")
        .keys
        .is_empty()
    {
        assert!(Instant::now() < deadline, "{key} never went");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Polls `session`'s liveliness read of `key` until it holds the token.
async fn until_held(session: &zenoh::Session, key: &str) {
    let deadline = Instant::now() + SETTLE;
    while zenkey::presence::liveliness_read(session, key, Duration::from_secs(1))
        .await
        .expect("a read")
        .keys
        .is_empty()
    {
        assert!(Instant::now() < deadline, "{key} never came");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// #872 (WC): in a namespace, an owner that leaves during `alarms --for`.
/// `lab/a` holds A firing at warning, its instance token declared before
/// T's watch session subscribes. Half a second into T's window it closes:
/// a clean close resolves nothing (§2.9), so only its token's withdrawal
/// says it went, and zenoh 1.10.1 never tells a namespaced subscriber
/// without history of a token declared before it (`routing/namespace.rs`).
/// Without history T read the owner present throughout and A firing now:
/// yes, exit 1, of an owner gone. T watches the wire keys from a watch
/// session in no namespace (#875), hears the withdrawal, and the owner is
/// possibly incomplete until a presence read says where it went: no
/// verdict, exit 2. The watchdog's `alert-firing` holds the same reader and
/// re-reads presence each tick, so it goes from firing to unobservable,
/// never ok, either way.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn alarms_in_a_namespace_hear_an_owner_leave_the_window() {
    use zenkey::alarms::Severity;
    let bus = Bus::bare(Some("ns")).await;
    let owning = client(&bus.endpoint, Some("ns")).await;
    let (svc, alarms) = alarms_owner(&owning, "lab/a", false).await;
    alarms
        .raise(alarm_a(Severity::Warning))
        .await
        .expect("A raised");
    wait_for_ns(&bus, "ns", &["lab/a"]).await;
    let instance = svc
        .instance_key()
        .expect("its instance key")
        .as_str()
        .to_owned();
    let watchdog = bus.spawn(&[
        "watchdog",
        "--rule",
        "alert-firing zk2/lab/** warning",
        "--every",
        "1",
        "--count",
        "10",
        "--namespace",
        "ns",
        "--clocks-synced",
        "--timeout",
        "1",
    ]);
    // Its reader opens and its first ticks judge the owner holding A.
    tokio::time::sleep(Duration::from_secs(4)).await;
    let (listening, t) = bus.spawn_on_line(
        &[
            "alarms",
            LAB,
            "--namespace",
            "ns",
            "--for",
            "3",
            "--lease",
            "1",
            "--at-least",
            "warning",
            "--clocks-synced",
            "--timeout",
            "1",
            "--format",
            "json",
        ],
        "alarms: listening",
    );
    listening.await.expect("T says it listens");
    tokio::time::sleep(Duration::from_millis(500)).await;
    drop(alarms);
    svc.close().await.expect("the owner closes");
    owning.close().await.expect("its session ends");
    until_gone(&bus.owners, &instance).await;
    let run = t.await.expect("the alarms run");
    let ruled = watchdog.await.expect("the watchdog");
    let doc = run.json();
    let a = alarm_of(&run, "lab/a", A_KEY);
    let lines = ndjson_lines(&ruled);
    let to: Vec<&str> = lines
        .iter()
        .filter(|l| l["row"] == "transition")
        .filter_map(|l| l["to"].as_str())
        .collect();
    exits(&run, 2);
    assert_ne!(doc["any_firing"]["answer"], "yes", "{run}");
    assert_ne!(
        a.as_ref().map(|a| a["firing"]["answer"].clone()),
        Some(Value::from("yes")),
        "A is never firing now of an owner heard gone: {run}"
    );
    assert_eq!(to.first(), Some(&"firing"), "{ruled}");
    assert_eq!(to.last(), Some(&"unobservable"), "{ruled}");
    assert!(!to.contains(&"ok"), "an owner gone is never ok: {ruled}");
}

// ─── telemetry.v1: scenarios.md §8, the tool's section (#819, TC) ──────────
//
// Common setup, the text's: R1 a router with timestamping on, the owner O
// and the tool T its clients. O is `lab/meter` over `meter.v1`
// (`zenkey/tests/contracts/meter.v1.toml`) through the runtime's checked
// writers, which write each counter Point's epoch as O's instance id. T is
// `zenctl telemetry`, every verdict the fleet's continuity reader's.

fn meter_contract_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../zenkey/tests/contracts/meter.v1.toml")
}

/// O: its service, and the writers of `frames` (a Point declared a counter),
/// `drops` (a bare counter with no epoch) and `total` (a bare counter in
/// state).
struct Meter {
    svc: Service,
    frames: zenkey::writer::Writer,
    drops: zenkey::writer::Writer,
    total: zenkey::state::StateWriter,
}

/// O started on `session`, every resource exposed, once a subscriber
/// matches both writers.
async fn meter_owner(session: &zenoh::Session) -> Meter {
    meter_owner_with(session, true).await
}

/// O started on `session`, every resource exposed; once a subscriber
/// matches both writers when `matched`, at once otherwise: an owner up
/// before its reader (#819 TE).
async fn meter_owner_with(session: &zenoh::Session, matched: bool) -> Meter {
    let l = load_path(&meter_contract_path());
    let c = l
        .contract
        .unwrap_or_else(|| panic!("meter.v1 does not load:\n{}", l.report));
    let id = c.iface.clone();
    let names: Vec<String> = c
        .resources
        .iter()
        .map(zenkey::implementation::resource_name)
        .collect();
    let mut b = ServiceBuilder::new(session, ServiceConfig::new("lab/meter".parse().unwrap()));
    b.implement(Implementation::new(c)).expect("implement");
    for n in names {
        let _ = b.expose(&id, &n);
    }
    let frames = b
        .declare_writer(&id, "stream/frames", &Bindings::new())
        .await
        .expect("frames");
    let drops = b
        .declare_writer(&id, "stream/drops", &Bindings::new())
        .await
        .expect("drops");
    let total = b
        .declare_state_writer(&id, "state/total", &Bindings::new())
        .await
        .expect("total");
    let svc = b.start().await.expect("start");
    let deadline = Instant::now() + SETTLE;
    for w in [&frames, &drops].into_iter().filter(|_| matched) {
        while !w.matching().await.expect("matching") {
            assert!(
                Instant::now() < deadline,
                "T's subscription never reached O"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    Meter {
        svc,
        frames,
        drops,
        total,
    }
}

impl Meter {
    /// `frames` and `drops` at `n`, the frames Point taken now.
    async fn put(&self, n: u64) {
        self.frames
            .put_value(&zenkey::telemetry::point(n))
            .await
            .expect("frames");
        self.drops.put_value(&n).await.expect("drops");
    }
}

/// A session of the vehicle's, authenticated as `vehicle`, whom the face
/// denies nothing: one process of O's.
async fn vehicle(endpoint: &str) -> zenoh::Session {
    let mut c = base_config();
    c.insert_json5("mode", "\"client\"").expect("config");
    c.insert_json5("connect/endpoints", &format!("[\"{endpoint}\"]"))
        .expect("config");
    c.insert_json5(
        "transport/auth/usrpwd",
        "{ user: \"vehicle\", password: \"vehicle-pw\" }",
    )
    .expect("config");
    zenoh::open(c).await.expect("the vehicle's session")
}

/// A member row's pairs, as `(earlier, later, continuity, epoch, via,
/// rated)`.
fn telemetry_pairs(doc: &Value, key: &str) -> Vec<(u64, u64, String, String, String, bool)> {
    let m = rows_of(doc, "member")
        .into_iter()
        .find(|r| r["key"] == key)
        .unwrap_or_else(|| panic!("{key} has a member row: {doc}"));
    let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
    m["pairs"]
        .as_array()
        .expect("pairs")
        .iter()
        .map(|p| {
            if let Some(r) = p["rate"].as_f64() {
                assert!(r >= 0.0, "never a negative rate: {p}");
            }
            (
                p["earlier"].as_u64().expect("a counter"),
                p["later"].as_u64().expect("a counter"),
                s(&p["continuity"]),
                s(&p["epoch"]),
                s(&p["via"]),
                p["rate"].is_number(),
            )
        })
        .collect()
}

/// telemetry.v1 `scenarios.md` §8 (§2.10, §2.14), the tool's run behind a
/// real face: T stands where a reader across a constrained face stands, on
/// the vehicle router whose access control denies the ground `@zk/**`, and
/// takes the deployment's word for O (`--across-face`, `meter.v1` in hand).
/// O publishes `drops` and `frames` at 10 and 20, its process ends, and it
/// starts again as I2 and publishes 3 and 8. T reads the pairs R0 reads:
/// `drops` 10 → 20 same_epoch, assumed, with a rate; 20 → 3 a reset,
/// assumed, with none; 3 → 8 same_epoch, assumed — and `frames` 20 → 3 a
/// reset established in-band, the Point's epoch having crossed with it.
/// Nothing decreased and no rate is negative. Since 0.4 (#819 TE, zk2py
/// round 47's F-152) the run exits 2: `frames` answers, but every pair of
/// `drops` is assumed, so of `drops` T could not tell, and `frames`' in-band
/// epoch establishes nothing of it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn telemetry_s8_a_counter_that_crosses_a_face() {
    let (bus, ground) = face_bus(false).await;
    let contracts = meter_contract_path();
    let t = bus.spawn(&[
        "telemetry",
        "zk2/lab/meter/**",
        "--across-face",
        "--contracts",
        contracts.to_str().expect("a UTF-8 path"),
        "--for",
        "12",
        "--zenoh-config",
        ground.to_str().expect("a UTF-8 path"),
        "--format",
        "json",
    ]);
    // Step 1: O, as I1.
    let o1 = vehicle(&bus.endpoint).await;
    let i1 = meter_owner(&o1).await;
    let first = i1.svc.instance().to_string();
    for n in [10, 20] {
        i1.put(n).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    // Step 2: O's process ends; it starts again, as I2.
    drop(i1);
    o1.close().await.expect("O's process ends");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let o2 = vehicle(&bus.endpoint).await;
    let i2 = meter_owner(&o2).await;
    assert_ne!(i2.svc.instance().to_string(), first, "a new instance");
    for n in [3, 8] {
        i2.put(n).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let run = t.await.expect("the telemetry run");
    exits(&run, 2);
    let doc = run.json();
    assert_eq!(doc["across_face"], true, "{run}");
    assert!(doc.get("presence").is_none(), "no presence crosses: {run}");
    assert_eq!(doc["totals"]["decreased"], 0, "{run}");
    assert_eq!(doc["totals"]["does_not"], 0, "{run}");
    assert_eq!(doc["totals"]["unanswered"], 1, "{run}");
    assert_eq!(doc["judgement"]["answer"], "unobservable", "{run}");
    let answer = |key: &str| {
        rows_of(&doc, "member")
            .into_iter()
            .find(|r| r["key"] == key)
            .map(|r| r["continuity"]["answer"].clone())
    };
    assert_eq!(
        answer("zk2/lab/meter/meter.v1/stream/drops"),
        Some(Value::from("unobservable")),
        "{run}"
    );
    assert_eq!(
        answer("zk2/lab/meter/meter.v1/stream/frames"),
        Some(Value::from("not_established")),
        "{run}"
    );
    let row = |a: u64, b: u64, c: &str, e: &str, v: &str, rated: bool| {
        (a, b, c.to_owned(), e.to_owned(), v.to_owned(), rated)
    };
    assert_eq!(
        telemetry_pairs(&doc, "zk2/lab/meter/meter.v1/stream/drops"),
        [
            row(10, 20, "same_epoch", "assumed", "", true),
            row(20, 3, "reset", "assumed", "", false),
            row(3, 8, "same_epoch", "assumed", "", true),
        ],
        "R0's pairs: no epoch, no presence: {run}"
    );
    assert_eq!(
        telemetry_pairs(&doc, "zk2/lab/meter/meter.v1/stream/frames"),
        [
            row(10, 20, "same_epoch", "established", "in_band", true),
            row(20, 3, "reset", "established", "in_band", false),
            row(3, 8, "same_epoch", "established", "in_band", true),
        ],
        "the Point's epoch crossed with it: {run}"
    );
    drop(i2);
    o2.close().await.expect("closes");
}

/// `zenctl telemetry` of `drops` alone, once O is up and listed: T's run,
/// and the moment its "listening" line is read.
async fn telemetry_of_drops(
    bus: &Bus,
) -> (
    tokio::sync::oneshot::Receiver<()>,
    tokio::task::JoinHandle<Run>,
) {
    telemetry_of(bus, DROPS).await
}

/// [`telemetry_of_drops`], of the members `selector` reaches.
async fn telemetry_of(
    bus: &Bus,
    selector: &str,
) -> (
    tokio::sync::oneshot::Receiver<()>,
    tokio::task::JoinHandle<Run>,
) {
    wait_for(bus, &["lab/meter"]).await;
    let contracts = meter_contract_path();
    let contracts = contracts.to_str().expect("a UTF-8 path").to_owned();
    bus.spawn_on_line(
        &[
            "telemetry",
            selector,
            "--contracts",
            &contracts,
            "--for",
            "3",
            "--format",
            "json",
        ],
        "telemetry: listening",
    )
}

const DROPS: &str = "zk2/lab/meter/meter.v1/stream/drops";

/// zk2py round 47's F-154 (#819 TE): T's "listening" line comes once its
/// subscriptions are declared and presence is seeded, and `--for` counts
/// from then. O is up before T, so T's seed finds its one instance. The
/// moment T's line is read, O puts `drops` at 10 and then at 7, raw, since
/// O's own writer refuses a decrease. T reads both, pairs 10 → 7 as
/// `decreased` within the epoch presence established (O alone
/// throughout), and exits 1. Before, the line came before T had read
/// presence, and a sample put on it was lost.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn telemetry_hears_a_decrease_put_right_after_its_listening_line() {
    let bus = Bus::bare(None).await;
    let o = meter_owner_with(&bus.owners, false).await;
    let (listening, t) = telemetry_of_drops(&bus).await;
    listening.await.expect("T says it listens");
    for n in ["10", "7"] {
        bus.owners.put(DROPS, n).await.expect("a put");
    }
    let run = t.await.expect("the telemetry run");
    exits(&run, 1);
    let doc = run.json();
    assert_eq!(doc["totals"]["samples"], 2, "both samples heard: {run}");
    assert_eq!(
        telemetry_pairs(&doc, DROPS),
        [(
            10,
            7,
            "decreased".to_owned(),
            "established".to_owned(),
            "presence".to_owned(),
            false
        )],
        "{run}"
    );
    drop(o);
}

/// #872 (WC): in a namespace, a pair across an owner's restart. O, as I1,
/// is up before T, its instance token declared before T's watch session
/// subscribes; once T listens, I1 puts `drops` (a bare counter, no epoch)
/// at 20, then O's process ends and it starts again as I2, which puts 3
/// and 8. zenoh 1.10.1 never tells a namespaced subscriber without history
/// that a token declared before it went (`routing/namespace.rs`), so T
/// held I1 and I2 both, and had no single instance on either side of a
/// pair: 20 → 3 a reset and 3 → 8 the same epoch, each assumed, so of
/// `drops` T could not tell, exit 2. T watches the wire keys from a watch
/// session in no namespace (#875), and hears I1 go: 20 → 3 a reset and
/// 3 → 8 the same epoch, both established by presence (I1 alone, then I2
/// alone), exit 0.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn telemetry_in_a_namespace_hears_an_owner_restart_between_a_pair() {
    let bus = Bus::bare(Some("ns")).await;
    let o1 = client(&bus.endpoint, Some("ns")).await;
    let i1 = meter_owner_with(&o1, false).await;
    let first = i1.svc.instance_key().expect("a key").as_str().to_owned();
    wait_for_ns(&bus, "ns", &["lab/meter"]).await;
    let contracts = meter_contract_path();
    let contracts = contracts.to_str().expect("a UTF-8 path").to_owned();
    let (listening, t) = bus.spawn_on_line(
        &[
            "telemetry",
            DROPS,
            "--namespace",
            "ns",
            "--contracts",
            &contracts,
            "--for",
            "4",
            "--format",
            "json",
        ],
        "telemetry: listening",
    );
    listening.await.expect("T says it listens");
    i1.drops.put_value(&20u64).await.expect("drops");
    tokio::time::sleep(Duration::from_millis(200)).await;
    // O's process ends; its token is gone where the owners' session reads.
    drop(i1);
    o1.close().await.expect("O's process ends");
    until_gone(&bus.owners, &first).await;
    let o2 = client(&bus.endpoint, Some("ns")).await;
    let i2 = meter_owner_with(&o2, false).await;
    let second = i2.svc.instance_key().expect("a key").as_str().to_owned();
    assert_ne!(second, first, "a new instance");
    until_held(&bus.owners, &second).await;
    for n in [3u64, 8] {
        i2.drops.put_value(&n).await.expect("drops");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let run = t.await.expect("the telemetry run");
    let doc = run.json();
    let pairs = telemetry_pairs(&doc, DROPS);
    let row = |a: u64, b: u64, c: &str| {
        (
            a,
            b,
            c.to_owned(),
            "established".to_owned(),
            "presence".to_owned(),
            c == "same_epoch",
        )
    };
    assert_eq!(
        pairs,
        [row(20, 3, "reset"), row(3, 8, "same_epoch")],
        "I1 alone, then I2 alone: {run}"
    );
    exits(&run, 0);
    drop(i2);
    o2.close().await.expect("closes");
}

/// zk2py round 47's F-152 (telemetry.v1 0.4 §5, #819 TE): one conforming
/// sample of a counter forms no pair, so T could not tell whether it went
/// backwards: exit 2, never "none decreased". Its member row says so, and
/// lists no pair of none.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn telemetry_could_not_tell_of_a_counter_heard_once() {
    let bus = Bus::bare(None).await;
    let o = meter_owner_with(&bus.owners, false).await;
    let (listening, t) = telemetry_of_drops(&bus).await;
    listening.await.expect("T says it listens");
    o.drops.put_value(&10u64).await.expect("drops");
    let run = t.await.expect("the telemetry run");
    exits(&run, 2);
    let doc = run.json();
    assert_eq!(doc["totals"]["conforms"], 1, "{run}");
    assert_eq!(doc["totals"]["unanswered"], 1, "{run}");
    assert_eq!(doc["judgement"]["answer"], "unobservable", "{run}");
    let drops = rows_of(&doc, "member")
        .into_iter()
        .find(|r| r["key"] == DROPS)
        .unwrap_or_else(|| panic!("a member row: {run}"));
    assert_eq!(drops["continuity"]["answer"], "unobservable", "{run}");
    assert!(
        drops["continuity"]["reason"]
            .as_str()
            .is_some_and(|r| r.starts_with("no pair formed")),
        "{run}"
    );
    assert_eq!(drops["continuity"]["pairs"], 0, "{run}");
    assert_eq!(drops["continuity"]["pairs_listed"], 0, "{run}");
    assert!(
        run.stderr.contains("could not tell"),
        "the exit's reason names it: {run}"
    );
}

/// zk2py round 48's F-157 (telemetry.v1 0.5 §5, #819 TF): a gauge's
/// conforming sample beside one that does not decode could not tell
/// whether every sample conformed: exit 2, its reason naming the member,
/// where 0.4's tool exited 0. The undecodable one is raw: O's writer
/// refuses it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn telemetry_could_not_tell_beside_an_undecodable_sample() {
    const LEVEL: &str = "zk2/lab/meter/meter.v1/stream/level";
    let bus = Bus::bare(None).await;
    let o = meter_owner_with(&bus.owners, false).await;
    let (listening, t) = telemetry_of(&bus, LEVEL).await;
    listening.await.expect("T says it listens");
    for v in ["-70.5", "1e400"] {
        bus.owners.put(LEVEL, v).await.expect("a put");
    }
    let run = t.await.expect("the telemetry run");
    exits(&run, 2);
    let doc = run.json();
    assert_eq!(doc["totals"]["conforms"], 1, "{run}");
    assert_eq!(doc["totals"]["unobservable"], 1, "{run}");
    assert_eq!(doc["judgement"]["answer"], "unobservable", "{run}");
    assert!(
        doc["judgement"]["reason"]
            .as_str()
            .is_some_and(|r| r.contains(&format!("{LEVEL}: 1 sample(s) undecodable"))),
        "{run}"
    );
    drop(o);
}

/// F-157's other half: a member under an invalid declaration answers no
/// question, so T could not tell, whatever conformed beside it (exit 2),
/// naming the resource and its declaration's reasons. T takes the
/// deployment's word across a face for `lab/meter` implementing `meter.v1`
/// and an `odd.v1` whose `blob` declares a counter its type cannot carry;
/// no owner runs, and every sample is raw.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn telemetry_could_not_tell_of_a_member_under_an_invalid_declaration() {
    let bus = Bus::bare(None).await;
    let odd = bus.home.join("odd.v1.toml");
    std::fs::write(
        &odd,
        "[interface]\nname = \"odd\"\nmajor = 1\nminor = 0\nuses = [\"telemetry.v1\"]\n\n\
         [resources.blob]\nkind = \"stream\"\ntype = { raw = \"application/octet-stream\" }\n\
         annotations = { \"telemetry.kind\" = \"counter\" }\n",
    )
    .expect("odd.v1");
    let meter = meter_contract_path();
    let (listening, t) = bus.spawn_on_line(
        &[
            "telemetry",
            "zk2/lab/meter/**",
            "--across-face",
            "--contracts",
            meter.to_str().expect("a UTF-8 path"),
            "--contracts",
            odd.to_str().expect("a UTF-8 path"),
            "--for",
            "3",
            "--format",
            "json",
        ],
        "telemetry: listening",
    );
    listening.await.expect("T says it listens");
    // Across a face the window opens on the subscriptions' declaration, with
    // no read after them: give the router a moment to hold them.
    tokio::time::sleep(Duration::from_millis(500)).await;
    bus.owners
        .put("zk2/lab/meter/meter.v1/stream/level", "-70.5")
        .await
        .expect("a put");
    bus.owners
        .put("zk2/lab/meter/odd.v1/stream/blob", "12")
        .await
        .expect("a put");
    let run = t.await.expect("the telemetry run");
    exits(&run, 2);
    let doc = run.json();
    assert_eq!(doc["totals"]["conforms"], 1, "{run}");
    assert_eq!(doc["totals"]["invalid"], 1, "{run}");
    assert!(
        doc["judgement"]["reason"].as_str().is_some_and(|r| r
            .contains("lab/meter odd.v1 stream/blob: its declaration is invalid (not_a_carrier)")),
        "{run}"
    );
    let blob = rows_of(&doc, "member")
        .into_iter()
        .find(|r| r["resource"] == "stream/blob")
        .unwrap_or_else(|| panic!("a member row: {run}"));
    assert_eq!(blob["continuity"]["answer"], "unobservable", "{run}");
    assert_eq!(
        blob["samples"]["unobservable_reason"], "declaration_invalid",
        "{run}"
    );
}

/// T with no face, `tries` times over `drops`, O up and listed before it;
/// the moment T's "listening" line is read, O puts `drops` through its
/// writer. The tries whose sample T did not hear, whose writer did not yet
/// match T's subscription, or whose line did not say the owner's
/// descriptor GET proved the opening (zk2py round 50's F-160), with the
/// first one's run.
async fn no_face_on_the_line(tries: usize) -> (Vec<usize>, Option<String>) {
    let bus = Bus::bare(None).await;
    let o = meter_owner_with(&bus.owners, false).await;
    wait_for(&bus, &["lab/meter"]).await;
    let contracts = meter_contract_path();
    let (mut lost, mut first) = (Vec::new(), None);
    for i in 0..tries {
        let (listening, t) = bus.spawn_on_line(
            &[
                "telemetry",
                DROPS,
                "--contracts",
                contracts.to_str().expect("a UTF-8 path"),
                "--for",
                "1",
                "--format",
                "json",
            ],
            "telemetry: listening",
        );
        listening.await.expect("T says it listens");
        let matched = o.drops.matching().await.expect("matching");
        o.drops.put_value(&(i as u64)).await.expect("drops");
        let run = t.await.expect("the telemetry run");
        let heard = rows_of(&run.json(), "member")
            .into_iter()
            .find(|r| r["key"] == DROPS)
            .and_then(|r| r["samples"]["received"].as_u64());
        let proved = run.stderr.contains(
            "each of the 1 owner instance(s) in scope answered its descriptor GET after the \
             subscriptions",
        );
        if heard != Some(1) || !matched || !proved {
            lost.push(i);
            first.get_or_insert_with(|| run.to_string());
        }
    }
    drop(o);
    (lost, first)
}

/// #819 TG's gate: with no face, the presence seed proves the router holds
/// T's subscriptions, and not that O's session does, and zenoh drops a put
/// at its source until it does: under load, 11 of 400 samples put on the
/// line were lost, each with O's writer unmatched. Now each owner
/// instance's descriptor GET, answered by O's session after the
/// declarations reach it, comes before the line: O's writer matches when
/// the line is read, and T hears every sample (800 of 800 under load).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn telemetry_hears_a_sample_put_on_its_listening_line() {
    let (lost, first) = no_face_on_the_line(ON_THE_LINE).await;
    assert!(
        lost.is_empty(),
        "{} of {ON_THE_LINE} samples put on the line were lost or unmatched: {lost:?}; the \
         first:\n{}",
        lost.len(),
        first.unwrap_or_default()
    );
}

/// T across the face, 24 times over `selector`, O on the vehicle before
/// it; the moment T's "listening" line is read, O puts `drops`, raw. The
/// tries whose sample T did not hear, with the first one's run, and the
/// first run's stderr.
async fn across_a_face_on_the_line(selector: &str) -> (Vec<usize>, Option<String>, String) {
    let (bus, ground) = face_bus(false).await;
    let contracts = meter_contract_path();
    let vehicle_session = vehicle(&bus.endpoint).await;
    let o = meter_owner_with(&vehicle_session, false).await;
    // O holds a member of `total`, so its S4 queryable answers with it: a
    // GET whose final comes with no reply proves nothing.
    o.total.put_value(&1u64).await.expect("total");
    let (mut lost, mut first, mut said) = (Vec::new(), None, String::new());
    for i in 0..ON_THE_LINE {
        let (listening, t) = bus.spawn_on_line(
            &[
                "telemetry",
                selector,
                "--across-face",
                "--contracts",
                contracts.to_str().expect("a UTF-8 path"),
                "--for",
                "1",
                "--zenoh-config",
                ground.to_str().expect("a UTF-8 path"),
                "--format",
                "json",
            ],
            "telemetry: listening",
        );
        listening.await.expect("T says it listens");
        vehicle_session
            .put(DROPS, i.to_string())
            .await
            .expect("a put");
        let run = t.await.expect("the telemetry run");
        if i == 0 {
            said.clone_from(&run.stderr);
        }
        let heard = rows_of(&run.json(), "member")
            .into_iter()
            .find(|r| r["key"] == DROPS)
            .and_then(|r| r["samples"]["received"].as_u64());
        if heard != Some(1) {
            lost.push(i);
            first.get_or_insert_with(|| run.to_string());
        }
    }
    drop(o);
    vehicle_session.close().await.expect("closes");
    (lost, first, said)
}

/// How many times the across-face tests put a sample on the line.
const ON_THE_LINE: usize = 24;

/// zk2py round 49's F-159 (#819 TG): across a face no presence is seeded,
/// so before TG nothing made a round trip after T's subscriptions, and a
/// sample put the moment its "listening" line appeared was lost (here 47
/// of 48 tries). Now T's S4 GET of the state metric resource in scope
/// (`total`), answered by O after the subscriptions, comes before the line,
/// and T hears every one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn telemetry_across_a_face_hears_a_sample_put_on_its_listening_line() {
    let (lost, first, said) = across_a_face_on_the_line("zk2/lab/meter/**").await;
    assert!(!said.contains("never proof"), "the GET proved it: {said}");
    assert!(
        said.contains("across the face the owner answered")
            && said
                .contains("S4 GET(s) of a state metric resource in scope after the subscriptions"),
        "the line says what proved it (F-160): {said}"
    );
    assert!(
        lost.is_empty(),
        "{} of {ON_THE_LINE} samples put on the line were lost; the first:\n{}",
        lost.len(),
        first.unwrap_or_default()
    );
}

/// F-159's other half: over `drops` alone there is no state metric resource
/// to GET, and nothing else crosses the face to answer, so the window opens
/// after `--settle` (0.25 s by default), a margin and never proof, and the
/// line says so. Here it is enough: before TG every sample was lost, and
/// 50 ms was measured enough.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn telemetry_across_a_face_with_no_state_opens_after_its_margin() {
    let (lost, first, said) = across_a_face_on_the_line(DROPS).await;
    assert!(
        said.contains("this line comes after a 0.25s margin (--settle), never proof"),
        "{said}"
    );
    assert!(
        lost.is_empty(),
        "{} of {ON_THE_LINE} samples put on the line were lost; the first:\n{}",
        lost.len(),
        first.unwrap_or_default()
    );
}

/// #825, and a tool's reading of a mock owner's metrics: `gen` brings up an
/// owner of `zs.snmp.v1`, whose state resources keep history, on a session
/// with timestamping on, as a real owner's is (it failed at the first such
/// resource while its session had it off). T reads its Points near it, with
/// presence: every sample conforms, its kind its tag.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gen_brings_up_an_owner_of_history_resources_and_telemetry_reads_it() {
    let bus = Bus::bare(None).await;
    let zensight = examples().join("zensight");
    let zensight = zensight.to_str().expect("a UTF-8 path").to_owned();
    let mock = bus.spawn(&[
        "gen",
        "lab/snmp",
        "zs.snmp.v1",
        "--contracts",
        &zensight,
        "--duration",
        "30",
        "--format",
        "json",
    ]);
    let run = bus
        .until(
            &[
                "telemetry",
                "zk2/lab/snmp/**",
                "--contracts",
                &zensight,
                "--for",
                "3",
                "--format",
                "json",
            ],
            |r| r.code == 0,
        )
        .await;
    exits(&run, 0);
    let doc = run.json();
    assert!(doc["totals"]["conforms"].as_u64() > Some(0), "{run}");
    assert_eq!(doc["totals"]["does_not"], 0, "{run}");
    assert_eq!(doc["presence"]["complete"], true, "{run}");
    let generated = mock.await.expect("the gen runner");
    exits(&generated, 0);
    let docs = documents(&generated.stdout);
    assert_eq!(docs[1]["report"], "gen", "{generated}");
    assert_eq!(docs[1]["failed"], 0, "{generated}");
    assert!(docs[1]["sent"].as_u64() > Some(10), "{generated}");
}

// ─── desired.v1 (#840, DD): scenarios.md §9, a tool's questions ──────────
//
// The scenarios' author and target (`lab/author` on `tasking.v1`,
// `lab/worker` on `worker.v1`), brought up through the runtime as the
// owner example's `--desired-author` and `--desired-target` bring them up,
// on the owner example's own simulated drive (`zenkey/examples/owner/
// desired.rs`, compiled in): it refuses a document holding `endpoint`
// (`reach`), fails an act on `fail-next`, and holds one on `hold`. T is
// `zenctl desired`, and the doctor's `desired-*` checks beside it.

#[allow(dead_code)]
#[path = "../../zenkey/examples/owner/desired.rs"]
mod drive;

/// The scenarios' `tasking.v1`, the author's.
fn tasking() -> Implementation {
    Implementation::new(drive::contracts().expect("the scenarios' contracts").0)
}

/// The author, `lab/author`, on a session of its own.
async fn desired_author(endpoint: &str) -> (zenoh::Session, Service, zenkey::desired::Author) {
    let session = client(endpoint, None).await;
    let imp = tasking();
    let iface = imp.iface().clone();
    let mut b = ServiceBuilder::new(
        &session,
        ServiceConfig::new("lab/author".parse().expect("an address")),
    );
    b.implement(imp).expect("tasking.v1");
    let a = b
        .desired_author(
            &iface,
            drive::TASKS,
            zenkey::desired::AuthorStartup::default(),
        )
        .await
        .expect("the author");
    let svc = b.start().await.expect("start");
    (session, svc, a)
}

/// A target of the scenarios' `worker.v1` at `address`, on a session of
/// its own.
struct DesiredTarget {
    session: zenoh::Session,
    svc: Service,
    target: zenkey::desired::Target,
    drive: Arc<drive::Drive>,
}

impl DesiredTarget {
    /// The process ends without closing.
    async fn kill(self) {
        drop(self.svc);
        self.session.close().await.expect("its session ends");
    }

    /// Until its report holds `f`, within [`SETTLE`].
    async fn until(&self, what: &str, f: impl Fn(&zenkey::desired::Report) -> bool) {
        let deadline = Instant::now() + SETTLE;
        loop {
            let r = self.target.report().await;
            if f(&r) {
                return;
            }
            assert!(Instant::now() < deadline, "never {what}: {r:?}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// `address` on `worker.v1`, its role `task` bound to `lab/author` (or,
/// `bound` false, the kill switch), reading `lab/archive`'s last-known
/// when `archive`.
async fn desired_target(
    endpoint: &str,
    address: &str,
    bound: bool,
    archive: bool,
) -> DesiredTarget {
    let session = client(endpoint, None).await;
    let worker = Implementation::new(drive::contracts().expect("the scenarios' contracts").1);
    let wi = worker.iface().clone();
    let mut cfg = ServiceConfig::new(address.parse().expect("an address"));
    if bound {
        cfg = cfg.bind(drive::ROLE, &["lab/author"]);
        cfg.bindings.get_mut(drive::ROLE).expect("bound").params = [
            ("system".to_owned(), "self.system".to_owned()),
            ("service".to_owned(), "self.service".to_owned()),
        ]
        .into();
    }
    let mut b = ServiceBuilder::new(&session, cfg);
    b.implement(worker).expect("worker.v1");
    let run = b
        .declare_state_writer(&wi, drive::RUNNING, &Bindings::new())
        .await
        .expect("Run");
    run.put_value(&drive::baseline())
        .await
        .expect("the baseline");
    let d = Arc::new(drive::Drive::new(run));
    let mut st = zenkey::desired::TargetStartup::default();
    if archive {
        st = st.with_archive(
            "lab/archive".parse().expect("an address"),
            Duration::from_secs(1),
        );
    }
    let target = b
        .desired_target(
            drive::ROLE,
            &tasking(),
            drive::TASKS,
            zenkey::desired::ReportAt::new(wi, drive::REPORT),
            Arc::clone(&d) as Arc<dyn zenkey::desired::Handler>,
            st,
        )
        .await
        .expect("the target");
    let svc = b.start().await.expect("start");
    let deadline = Instant::now() + SETTLE;
    while target.last_seed().is_none() && bound {
        assert!(Instant::now() < deadline, "the target's first seed");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // Its subscription has reached the router, ahead of the author's puts.
    tokio::time::sleep(Duration::from_millis(200)).await;
    DesiredTarget {
        session,
        svc,
        target,
        drive: d,
    }
}

/// `v<n>` of the scenarios: a `Task` with `revision` n.
fn task(n: u64, extra: &str) -> Vec<u8> {
    format!(r#"{{"revision": {n}, "mode": "run", "rate_hz": 10{extra}}}"#).into_bytes()
}

/// A `desired` run's one row's four answers, `answer:reason`.
fn desired_answers(run: &Run) -> Vec<String> {
    let doc = run.json();
    let rows = rows_of(&doc, "desired");
    assert_eq!(rows.len(), 1, "{run}");
    let a = &rows[0]["answers"];
    ["converged", "refused", "last_known", "in_force"]
        .iter()
        .map(|q| {
            format!(
                "{}:{}",
                a[q]["answer"].as_str().unwrap_or("?"),
                a[q]["reason"].as_str().unwrap_or("?")
            )
        })
        .collect()
}

/// The doctor's three desired checks, their verdicts by id.
async fn desired_doctor(bus: &Bus, extra: &[&str]) -> (Run, Vec<(String, Value)>) {
    let mut args = doctor_args(&[
        "--check",
        "desired-refused",
        "--check",
        "desired-behind",
        "--check",
        "desired-last-known",
        "--format",
        "json",
    ]);
    args.extend_from_slice(extra);
    let run = bus.zenctl(&args).await;
    let doc = run.json();
    let rows = ["desired-refused", "desired-behind", "desired-last-known"]
        .iter()
        .map(|id| ((*id).to_owned(), check_row(&doc, id).clone()))
        .collect();
    (run, rows)
}

fn verdict(rows: &[(String, Value)], id: &str) -> String {
    rows.iter()
        .find(|(i, _)| i == id)
        .map(|(_, r)| r["verdict"]["answer"].as_str().unwrap_or("?").to_owned())
        .unwrap_or_default()
}

/// desired.v1 `scenarios.md` §9, steps 1 to 3 (§2.17, §5), and the window
/// (text 0.4): T asks of a target converged on its author's v2 (§1's end):
/// yes, converged, exit 0; of one that refused v3 for its `endpoint`
/// (§4's step 1): no, refused, exit 1, and the doctor's `desired-refused`;
/// of one whose v4 failed partway (§4's step 5): no, failed, in force in
/// part, exit 1. Then v5 converges and the drive holds the act on v6: over
/// a window, a target "not yet" on its author's v6 for all of it is behind,
/// exit 1, and with
/// `--clocks-synced` the doctor's `desired-behind` finds it once v6 is older
/// than its bound.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn desired_s9_converged_refused_failed_and_behind_for_a_tool() {
    use zenkey::desired::Outcome;
    let bus = Bus::bare(None).await;
    let (_as, _author_svc, author) = desired_author(&bus.endpoint).await;
    let w = desired_target(&bus.endpoint, "lab/worker", true, false).await;
    let args = [
        "desired",
        "lab/worker",
        "--timeout",
        "2",
        "--format",
        "json",
    ];

    // 1. Converged on v2.
    author.put("lab/worker", task(1, "")).await.expect("v1");
    author.put("lab/worker", task(2, "")).await.expect("v2");
    w.until("converged on v2", |r| {
        r.outcome == Some(Outcome::Converged) && r.observed.is_some_and(|o| o.revision == Some(2))
    })
    .await;
    let run = bus.until(&args, |r| r.code == 0).await;
    exits(&run, 0);
    assert_eq!(
        desired_answers(&run),
        ["yes:converged", "no:converged", "no:current", "yes:desired"],
        "{run}"
    );
    let doc = run.json();
    let row = &rows_of(&doc, "desired")[0];
    assert_eq!(row["slice"], DESIRED_D, "{run}");
    assert_eq!(row["slice_from"], "author_contract");
    assert_eq!(doc["judgement"]["answer"], "not_established");
    let (dr, checks) = desired_doctor(&bus, &[]).await;
    exits(&dr, 0);
    for id in ["desired-refused", "desired-behind", "desired-last-known"] {
        assert_eq!(verdict(&checks, id), "not_established", "{id}: {dr}");
    }
    // check conform's desired cases: the target's report and binding, and
    // the author's declaration.
    let conform = |address: &'static str, iface: &'static str| {
        [
            "check",
            "conform",
            address,
            iface,
            "--for",
            "1",
            "--timeout",
            "2",
            "--format",
            "json",
        ]
    };
    let run = bus.zenctl(&conform("lab/worker", "worker.v1")).await;
    let doc = run.json();
    for (case, subject) in [
        ("desired-report", "service"),
        ("desired-binding", "requires.task"),
    ] {
        assert_eq!(
            conform_case(&doc, case, subject)["verdict"]["answer"],
            "not_established",
            "{case}: {run}"
        );
    }
    assert_eq!(
        conform_case(&doc, "desired-author", "worker.v1")["verdict"]["answer"],
        "not_asked",
        "{run}"
    );
    let run = bus.zenctl(&conform("lab/author", "tasking.v1")).await;
    let doc = run.json();
    assert_eq!(
        conform_case(&doc, "desired-author", "state/tasks/{system}/{service}")["verdict"]["answer"],
        "not_established",
        "{run}"
    );

    // 2. v3 names the worker's endpoint: refused, reach, v2 still in force.
    author
        .put(
            "lab/worker",
            task(3, r#", "endpoint": "tcp/10.0.0.9:7447""#),
        )
        .await
        .expect("v3");
    w.until("refused v3", |r| r.outcome == Some(Outcome::Refused))
        .await;
    let run = bus.until(&args, |r| r.code == 1).await;
    exits(&run, 1);
    assert_eq!(
        desired_answers(&run),
        ["no:refused", "yes:refused", "no:current", "yes:desired"],
        "{run}"
    );
    let doc = run.json();
    let report = &rows_of(&doc, "desired")[0]["report"]["document"];
    assert_eq!(report["refusal"]["code"], "reach", "{run}");
    assert_eq!(report["refusal"]["pointer"], "/endpoint");
    let (dr, checks) = desired_doctor(&bus, &[]).await;
    exits(&dr, 1);
    assert_eq!(verdict(&checks, "desired-refused"), "established", "{dr}");

    // 3. v4 fails partway: applied in part, no one version wholly in force.
    w.drive.fail_next();
    author
        .put(
            "lab/worker",
            br#"{"revision": 4, "mode": "halt", "rate_hz": 20}"#.to_vec(),
        )
        .await
        .expect("v4");
    w.until("failed v4", |r| r.outcome == Some(Outcome::Failed))
        .await;
    let run = bus
        .until(&args, |r| {
            r.code == 1 && desired_answers(r)[0] == "no:failed"
        })
        .await;
    exits(&run, 1);
    assert_eq!(
        desired_answers(&run),
        ["no:failed", "yes:failed", "no:current", "yes:partial"],
        "{run}"
    );

    // The window: v5 converges, then the drive holds the act on v6, so the
    // report stays on v5, its last act converged.
    author.put("lab/worker", task(5, "")).await.expect("v5");
    w.until("converged on v5", |r| {
        r.outcome == Some(Outcome::Converged) && r.observed.is_some_and(|o| o.revision == Some(5))
    })
    .await;
    w.drive.hold();
    author.put("lab/worker", task(6, "")).await.expect("v6");
    let deadline = Instant::now() + SETTLE;
    while !w.drive.with(|s| s.held) {
        assert!(Instant::now() < deadline, "the act on v6 held");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let one = bus.zenctl(&args).await;
    assert_eq!(
        desired_answers(&one)[0],
        "not_yet:behind",
        "one reading: not yet, no verdict: {one}"
    );
    let windowed = [
        "desired",
        "lab/worker",
        "--for",
        "2",
        "--timeout",
        "2",
        "--format",
        "json",
    ];
    let run = bus.zenctl(&windowed).await;
    exits(&run, 1);
    let doc = run.json();
    let row = &rows_of(&doc, "desired")[0];
    assert_eq!(row["answers"]["converged"]["answer"], "not_yet", "{run}");
    assert_eq!(row["behind"]["by"], "window", "{run}");
    assert!(
        row["behind"]["since_s"].as_f64().unwrap_or(0.0) >= 2.0,
        "{run}"
    );
    assert!(doc["readings"].as_u64().unwrap_or(0) >= 2, "{run}");
    let (dr, checks) = desired_doctor(&bus, &[]).await;
    assert_eq!(
        verdict(&checks, "desired-behind"),
        "unobservable",
        "one reading, no trusted clock: {dr}"
    );
    let (dr, checks) = desired_doctor(&bus, &["--clocks-synced", "--desired-behind", "1"]).await;
    exits(&dr, 1);
    assert_eq!(verdict(&checks, "desired-behind"), "established", "{dr}");
    w.drive.release();
    w.until("converged on v6", |r| {
        r.outcome == Some(Outcome::Converged) && r.observed.is_some_and(|o| o.revision == Some(6))
    })
    .await;
    let run = bus.until(&windowed, |r| r.code == 0).await;
    exits(&run, 0);
    w.kill().await;
}

/// desired.v1 `scenarios.md` §9, steps 4 and 5 (§2.9, §2.12, §2.17): after
/// §7's step 1 — an archive on the target's side recorded v1, the author
/// closed cleanly and the target restarted, reading v1 from the archive —
/// T asks: converged not asked, `author_absent`; refused no; last-known
/// yes; in force yes; and shows the archive's v1 as last-known, never
/// current: exit 2. The doctor's `desired-last-known` names it (info).
/// Against §8's target, its role unbound: not asked, `unbound`, and in
/// force no, `unbound`: nothing asked, exit 2.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn desired_s9_last_known_and_the_kill_switch_for_a_tool() {
    use zenkey::desired::Outcome;
    let bus = Bus::bare(None).await;
    let archiving = client(&bus.endpoint, None).await;
    let archive = Archive::start(
        &archiving,
        ArchiveConfig {
            service: ServiceConfig::new("lab/archive".parse().expect("an address")),
            records: vec![Recorded {
                owner: "lab/author".parse().expect("an address"),
                selector: "zk2/lab/author/tasking.v1/state/**".to_owned(),
                implementation: tasking(),
            }],
            peers: Vec::new(),
            unconfirmed_horizon: None,
            watch: None,
        },
    )
    .await
    .expect("an archive");
    let (author_session, author_svc, author) = desired_author(&bus.endpoint).await;
    wait_for(&bus, &["lab/author", "lab/archive"]).await;
    let w = desired_target(&bus.endpoint, "lab/worker", true, true).await;
    let s1 = match author.put("lab/worker", task(1, "")).await.expect("v1") {
        zenkey::desired::Written::Put(ts) => zenkey::desired::stamp(&ts).to_string(),
        other => panic!("{other:?}"),
    };
    w.until("converged on v1", |r| r.outcome == Some(Outcome::Converged))
        .await;
    let deadline = Instant::now() + SETTLE;
    while archive.confirmed(DESIRED_D) != Some(true) {
        assert!(Instant::now() < deadline, "the archive records v1");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // §7's step 1: the author closes cleanly; the target restarts, a new
    // run, and reads v1 from the archive.
    drop(author);
    author_svc.close().await.expect("the author closes");
    author_session.close().await.expect("its session ends");
    w.kill().await;
    let w = desired_target(&bus.endpoint, "lab/worker", true, true).await;
    w.until("converged on v1, last-known", |r| {
        r.last_known && r.outcome == Some(Outcome::Converged)
    })
    .await;
    let args = [
        "desired",
        "lab/worker",
        "--timeout",
        "2",
        "--format",
        "json",
    ];
    let run = bus
        .until(&args, |r| {
            r.code == 2 && desired_answers(r)[0] == "not_asked:author_absent"
        })
        .await;
    exits(&run, 2);
    assert_eq!(
        desired_answers(&run),
        [
            "not_asked:author_absent",
            "no:converged",
            "yes:last_known",
            "yes:desired"
        ],
        "{run}"
    );
    let doc = run.json();
    let row = &rows_of(&doc, "desired")[0];
    assert_eq!(row["archived"][0]["archive"], "lab/archive", "{run}");
    assert_eq!(row["archived"][0]["stamp"], s1.as_str(), "{run}");
    assert!(
        row["author"].get("stamp").is_none(),
        "the archive's v1 is never shown as the author's current: {run}"
    );
    assert_eq!(
        row["slice_from"], "requirement",
        "no contract of the absent author's: {run}"
    );
    let table = bus
        .zenctl(&[
            "desired",
            "lab/worker",
            "--timeout",
            "2",
            "--format",
            "table",
        ])
        .await;
    assert!(
        table.stdout.contains("last-known, never current"),
        "{table}"
    );
    let (dr, checks) = desired_doctor(&bus, &[]).await;
    assert_eq!(
        verdict(&checks, "desired-last-known"),
        "established",
        "{dr}"
    );
    assert_eq!(
        verdict(&checks, "desired-refused"),
        "not_established",
        "{dr}"
    );
    exits(&dr, 0);

    // §9's step 5: §8's target, its role unbound.
    let spare = desired_target(&bus.endpoint, "lab/spare", false, false).await;
    let args = ["desired", "lab/spare", "--timeout", "2", "--format", "json"];
    let run = bus
        .until(&args, |r| {
            r.code == 2 && desired_answers(r)[0] == "not_asked:unbound"
        })
        .await;
    exits(&run, 2);
    assert_eq!(
        desired_answers(&run),
        [
            "not_asked:unbound",
            "not_asked:unbound",
            "not_asked:unbound",
            "no:unbound"
        ],
        "{run}"
    );
    assert_eq!(run.json()["judgement"]["answer"], "not_asked", "{run}");
    spare.kill().await;
    w.kill().await;
    drop(archive);
}

/// The slice of `lab/worker`'s task (`scenarios.md`, "The target").
const DESIRED_D: &str = "zk2/lab/author/tasking.v1/state/tasks/lab/worker";

// ─── redundancy.v1 (#841, RC): scenarios.md §12, a tool's questions ──────
//
// `lab/ledger` on `ledger.v1` (`zenkey/tests/contracts/ledger.v1.toml`),
// governed under `claim`. A and B are candidates brought up through the
// runtime as the scenarios' conventions say: three sessions each (the
// claim's, its HLC on; a watch; a reader), A's id lower than B's, each
// putting `balance` with its name at its takeover and answering `append`
// with it. X is a test session that declares tokens under `lab/ledger` and
// serves a descriptor it writes. T is `zenctl redundancy`, which reads
// twice, 2 s apart, on a session that has never held a liveliness
// subscriber; the contract is given it offline, so a shape with no live
// candidate to serve the bundle is still read as governed.

const LEDGER: &str = "lab/ledger";
const LEDGER_A: &str = "000000000000000a";
const LEDGER_B: &str = "000000000000000b";
const LEDGER_X: &str = "000000000000000c";

fn ledger_path() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../zenkey/tests/contracts/ledger.v1.toml")
        .to_str()
        .expect("a UTF-8 path")
        .to_owned()
}

fn ledger_contract() -> Contract {
    let l = load_path(Path::new(&ledger_path()));
    l.contract.unwrap_or_else(|| panic!("{}", l.report))
}

/// A candidate's session: a client of `ep`, its HLC on (§2.10), retrying
/// fast after a cut.
async fn candidate_session(ep: &str) -> zenoh::Session {
    let mut c = base_config();
    c.insert_json5("mode", "\"client\"").expect("config");
    c.insert_json5("connect/endpoints", &format!("[\"{ep}\"]"))
        .expect("config");
    c.insert_json5(
        "connect/retry",
        "{ period_init_ms: 100, period_max_ms: 500, period_increase_factor: 1.5 }",
    )
    .expect("config");
    c.insert_json5("timestamping/enabled", "true")
        .expect("config");
    zenoh::open(c).await.expect("a candidate's session")
}

/// A router linked to R1 through `proxy`, retrying fast after a cut.
async fn router_via(proxy: &Proxy) -> (zenoh::Session, String) {
    let mut c = base_config();
    c.insert_json5("mode", "\"router\"").expect("config");
    c.insert_json5("listen/endpoints", r#"["tcp/127.0.0.1:0"]"#)
        .expect("config");
    c.insert_json5("connect/endpoints", &format!("[\"{}\"]", proxy.endpoint))
        .expect("config");
    c.insert_json5(
        "connect/retry",
        "{ period_init_ms: 100, period_max_ms: 500, period_increase_factor: 1.5 }",
    )
    .expect("config");
    let r = zenoh::open(c).await.expect("R2");
    let ep = r
        .info()
        .locators()
        .await
        .into_iter()
        .map(|l| l.to_string())
        .find(|l| l.starts_with("tcp/127.0.0.1:"))
        .expect("R2 listens on loopback");
    (r, ep)
}

/// The takeover's put (§2.6 step 3): `balance`, its name.
struct PutName {
    state: Arc<zenkey::state::StateWriter>,
    name: &'static str,
}

impl zenkey::redundancy::Takeover for PutName {
    fn taking_over<'a>(&'a self, _iface: &'a IfaceId) -> zenkey::call::BoxFuture<'a, ()> {
        Box::pin(async move {
            let _ = self.state.put(self.name).await;
        })
    }
}

/// One candidate of `lab/ledger` (or of `@hostid.v1/ledger` on `minter`).
struct LedgerCandidate {
    svc: Service,
    red: zenkey::redundancy::Redundancy,
    _servers: Vec<zenkey::operation::OperationServer>,
    _state: Arc<zenkey::state::StateWriter>,
    _entries: Option<zenkey::writer::Writer>,
    /// `appendall`'s executions, on `ledger-appendall.v1` (F-221, F-222).
    appendall: Arc<AtomicU64>,
    sessions: Vec<zenoh::Session>,
}

impl LedgerCandidate {
    async fn up(
        ep: &str,
        name: &'static str,
        id: u64,
        minter: Option<&zenkey::hostid::HostIdMinter>,
    ) -> LedgerCandidate {
        LedgerCandidate::up_on(ep, name, id, minter, ledger_contract()).await
    }

    /// [`LedgerCandidate::up`] on `contract`, a revision of `ledger.v1`:
    /// `ledger-appendall.v1`'s `appendall` (F-221) is served too, its
    /// executions counted.
    async fn up_on(
        ep: &str,
        name: &'static str,
        id: u64,
        minter: Option<&zenkey::hostid::HostIdMinter>,
        contract: Contract,
    ) -> LedgerCandidate {
        let claim = candidate_session(ep).await;
        let watch = candidate_session(ep).await;
        let reader = candidate_session(ep).await;
        let has = |r: &str| {
            contract
                .resources
                .iter()
                .any(|x| zenkey::implementation::resource_name(x) == r)
        };
        let (entries_declared, appendall_served) = (has("stream/entries"), has("@op/appendall"));
        let imp = Implementation::new(contract);
        let iface = imp.iface().clone();
        let mut b = match minter {
            Some(m) => ServiceBuilder::with_hostid(
                &claim,
                ServiceConfig::minted_ephemeral(
                    zenkey::model::grammar::Name::new("service", "ledger").expect("a name"),
                ),
                m,
            ),
            None => ServiceBuilder::new(
                &claim,
                ServiceConfig::new(LEDGER.parse().expect("an address")),
            ),
        };
        b.set_instance_id(zenkey::model::grammar::InstanceId::from_u64(id));
        b.implement(imp).expect("ledger.v1");
        let none = Bindings::new();
        let state = Arc::new(
            b.declare_state_writer(&iface, "state/balance", &none)
                .await
                .expect("balance"),
        );
        let entries = if entries_declared {
            Some(
                b.declare_writer(&iface, "stream/entries", &none)
                    .await
                    .expect("entries"),
            )
        } else {
            None
        };
        let appendall = Arc::new(AtomicU64::new(0));
        let mut servers = Vec::new();
        if appendall_served {
            let n = Arc::clone(&appendall);
            servers.push(
                b.serve(&iface, "@op/appendall", Some(&none), move |call: Call| {
                    n.fetch_add(1, Ordering::SeqCst);
                    async move {
                        call.reply(name)
                            .await
                            .map_err(|e| OpError::internal(e.to_string()))
                    }
                })
                .await
                .expect("appendall"),
            );
        }
        let server = b
            .serve(
                &iface,
                "@op/append",
                Some(&none),
                move |call: Call| async move {
                    call.reply(name)
                        .await
                        .map_err(|e| OpError::internal(e.to_string()))
                },
            )
            .await
            .expect("append");
        let startup = zenkey::redundancy::RedundancyStartup::new(reader.clone())
            .with_watch(watch.clone())
            .with_retry(Duration::from_secs(2))
            .with_grace(Duration::from_secs(2))
            .with_takeover(Arc::new(PutName {
                state: Arc::clone(&state),
                name,
            }));
        let red = b.redundancy(startup).expect("redundancy.v1");
        let svc = b.start().await.expect("a candidate starts");
        servers.push(server);
        LedgerCandidate {
            svc,
            red,
            _servers: servers,
            _state: state,
            _entries: entries,
            appendall,
            sessions: vec![claim, watch, reader],
        }
    }

    fn role(&self) -> zenkey::redundancy::Role {
        self.red.role(&"ledger.v1".parse().expect("an interface"))
    }

    async fn until(&self, role: zenkey::redundancy::Role) {
        let deadline = Instant::now() + SETTLE;
        while self.role() != role {
            assert!(Instant::now() < deadline, "never {role:?}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn close(self) {
        self.svc.close().await.expect("a clean close");
        for s in self.sessions {
            let _ = s.close().await;
        }
    }
}

/// X (§12's setup): an instance token at `lab/ledger` as instance
/// [`LEDGER_X`], `ledger.v1`'s interface token when `holds`, and a
/// descriptor it serves listing `ledger.v1`, marked standby or not —
/// served before the tokens, in core §8.2's order.
struct Shape {
    _tokens: Vec<zenoh::liveliness::LivelinessToken>,
    _descriptor: zenoh::query::Queryable<()>,
    session: zenoh::Session,
}

impl Shape {
    async fn up(ep: &str, standby: bool, holds: bool) -> Shape {
        use zenoh::Wait;
        let session = client(ep, None).await;
        let fp = zenkey::model::canonical::Fingerprint::of(&ledger_contract());
        let instance = format!("zk2/{LEDGER}/@zk/instance/{LEDGER_X}");
        let body = serde_json::to_vec(&json!({
            "format": "zk2-descriptor/0.1",
            "service": LEDGER,
            "instance": LEDGER_X,
            "interfaces": [{"iface": "ledger.v1", "contract": fp.to_string(), "minor": 0,
                            "standby": standby, "token": true}],
            "meta": {"zid": session.zid().to_string()},
        }))
        .expect("a descriptor");
        let descriptor = session
            .declare_queryable(instance.clone())
            .callback(move |q| {
                let _ = q
                    .reply(q.key_expr().clone(), body.clone())
                    .encoding(zenoh::bytes::Encoding::APPLICATION_JSON)
                    .wait();
            })
            .await
            .expect("X's descriptor");
        let mut tokens = vec![
            session
                .liveliness()
                .declare_token(instance)
                .await
                .expect("X's instance token"),
        ];
        if holds {
            tokens.push(
                session
                    .liveliness()
                    .declare_token(format!(
                        "zk2/{LEDGER}/@zk/alive/ledger.v1/{LEDGER_X}/{}",
                        fp.hex().fp16()
                    ))
                    .await
                    .expect("X's interface token"),
            );
        }
        Shape {
            _tokens: tokens,
            _descriptor: descriptor,
            session,
        }
    }

    async fn down(self) {
        drop(self._tokens);
        drop(self._descriptor);
        let _ = self.session.close().await;
    }
}

/// T: `zenctl redundancy <address> --contracts ledger.v1 --format json`.
fn t_args(address: &str, path: &str) -> Vec<String> {
    [
        "redundancy",
        address,
        "--contracts",
        path,
        "--format",
        "json",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect()
}

/// T's one row, and an instance's role and fourth answer in it.
fn t_row(run: &Run) -> Value {
    rows_of(&run.json(), "interface")
        .first()
        .map(|r| (*r).clone())
        .unwrap_or(Value::Null)
}

fn t_instance(row: &Value, id: &str) -> (String, String) {
    let i = row["instances"]
        .as_array()
        .and_then(|a| a.iter().find(|i| i["instance"] == id))
        .unwrap_or_else(|| panic!("{id} in {row}"));
    (
        i["role"].as_str().unwrap_or("?").to_owned(),
        format!(
            "{}:{}",
            i["twin"]["answer"].as_str().unwrap_or("?"),
            i["twin"]["reason"].as_str().unwrap_or("?")
        ),
    )
}

fn t_answer(row: &Value, q: &str) -> String {
    format!(
        "{}:{}",
        row[q]["answer"].as_str().unwrap_or("?"),
        row[q]["reason"].as_str().unwrap_or("?")
    )
}

async fn t_until(bus: &Bus, address: &str, done: impl Fn(&Run) -> bool) -> Run {
    let path = ledger_path();
    let args = t_args(address, &path);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    bus.until(&args, |r| !r.stdout.is_empty() && done(r)).await
}

fn has_row(r: &Run) -> bool {
    serde_json::from_str::<Value>(&r.stdout)
        .ok()
        .is_some_and(|v| !rows_of(&v, "interface").is_empty())
}

/// §12 steps 1 and 7 (its first half): A serves and B stands by. T: one
/// incumbent, no vacancy, one standby, exact, A the incumbent and B a
/// standby, exit 0; the doctor finds no split-brain and no token-missing
/// for B (core §6, 0.39), nor a lasting vacancy.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn redundancy_s12_one_serves_one_stands_by() {
    use zenkey::redundancy::Role;
    let bus = Bus::bare(None).await;
    let a = LedgerCandidate::up(&bus.endpoint, "A", 0xa, None).await;
    a.until(Role::Serving).await;
    let b = LedgerCandidate::up(&bus.endpoint, "B", 0xb, None).await;
    let run = t_until(&bus, LEDGER, |r| {
        r.code == 0 && has_row(r) && t_row(r)["standbys"]["count"] == 1
    })
    .await;
    exits(&run, 0);
    let row = t_row(&run);
    assert_eq!(t_answer(&row, "one_incumbent"), "yes:one", "{run}");
    assert_eq!(t_answer(&row, "vacancy"), "no:held", "{run}");
    assert_eq!(
        (
            t_answer(&row, "standbys"),
            &row["standbys"]["count"],
            &row["standbys"]["exact"]
        ),
        ("yes:standing_by".to_owned(), &json!(1), &json!(true)),
        "{run}"
    );
    assert_eq!(
        t_instance(&row, LEDGER_A),
        ("incumbent".into(), "no:incumbent".into()),
        "{run}"
    );
    assert_eq!(
        t_instance(&row, LEDGER_B),
        ("standby".into(), "no:standby".into()),
        "{run}"
    );
    assert_eq!(b.role(), Role::Standby);

    // Step 7, step 1's shape.
    let dr = bus
        .zenctl(&doctor_args(&[
            "--check",
            "split-brain",
            "--check",
            "token-missing",
            "--check",
            "redundancy-vacancy",
            "--format",
            "json",
        ]))
        .await;
    let doc = dr.json();
    for id in ["split-brain", "token-missing", "redundancy-vacancy"] {
        assert_eq!(
            check_row(&doc, id)["verdict"]["answer"],
            "not_established",
            "{id}: {dr}"
        );
    }
    exits(&dr, 0);
    b.close().await;
    a.close().await;
}

/// §12 steps 2 and 7 (its second half): the split of §7 step 2, T a client
/// of R1 after the heal and before B's step-down, which the test holds.
/// T: no, `split`; A yes, `split_lowest`; B yes, `split`; exit 1. The
/// doctor: one `split-brain` finding naming A and B. Released, B steps
/// down, and T reads one incumbent again.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn redundancy_s12_a_split_held_after_the_heal() {
    use zenkey::redundancy::Role;
    let bus = Bus::bare(None).await;
    let proxy = Proxy::to(&bus.endpoint).await;
    let (_r2, ep2) = router_via(&proxy).await;
    let a = LedgerCandidate::up(&bus.endpoint, "A", 0xa, None).await;
    a.until(Role::Serving).await;
    let b = LedgerCandidate::up(&ep2, "B", 0xb, None).await;
    // B reads A's token across the link, and stands by on it.
    let run = t_until(&bus, LEDGER, |r| {
        r.code == 0 && has_row(r) && t_row(r)["standbys"]["count"] == 1
    })
    .await;
    exits(&run, 0);
    assert_eq!(b.role(), Role::Standby);
    // §7 steps 1 and 2: the link cut, B takes over behind it.
    proxy.partition();
    b.until(Role::Serving).await;
    assert_eq!(a.role(), Role::Serving, "A still serves");
    // The heal, B's step-down held (§9's hold).
    b.red.hold_heal(true);
    proxy.heal();
    let run = t_until(&bus, LEDGER, |r| {
        r.code == 1 && has_row(r) && t_answer(&t_row(r), "one_incumbent") == "no:split"
    })
    .await;
    exits(&run, 1);
    let row = t_row(&run);
    assert_eq!(t_answer(&row, "one_incumbent"), "no:split", "{run}");
    assert_eq!(
        t_instance(&row, LEDGER_A),
        ("twin".into(), "yes:split_lowest".into()),
        "{run}"
    );
    assert_eq!(
        t_instance(&row, LEDGER_B),
        ("twin".into(), "yes:split".into()),
        "{run}"
    );
    // Step 7, step 2's shape: one split-brain finding, naming A and B.
    let dr = bus
        .zenctl(&doctor_args(&[
            "--check",
            "split-brain",
            "--format",
            "json",
        ]))
        .await;
    let doc = dr.json();
    let sb = check_row(&doc, "split-brain");
    assert_eq!(sb["verdict"]["answer"], "established", "{dr}");
    let findings = sb["findings"].as_array().expect("findings");
    assert_eq!(findings.len(), 1, "{dr}");
    let evidence = findings[0]["evidence"].as_str().unwrap_or_default();
    assert!(
        evidence.contains(LEDGER_A) && evidence.contains(LEDGER_B),
        "{dr}"
    );
    assert!(evidence.contains("redundancy.v1 governs it"), "{dr}");
    exits(&dr, 1);
    // Released: B, the higher, steps down (§2.9), and A stands alone.
    b.red.hold_heal(false);
    b.until(Role::Standby).await;
    let run = t_until(&bus, LEDGER, |r| {
        r.code == 0 && has_row(r) && t_answer(&t_row(r), "one_incumbent") == "yes:one"
    })
    .await;
    exits(&run, 0);
    assert_eq!(t_instance(&t_row(&run), LEDGER_A).0, "incumbent", "{run}");
    b.close().await;
    a.close().await;
}

/// §12 steps 3 to 5, X's shapes. 3: no instance serves, X marked standby:
/// no, `vacant`; a vacancy yes, `lasting`; exit 1 (a lasting vacancy is
/// a finding, zenctl's exit contract). 4: A serves, X listing it unmarked
/// with no token: X yes, `unmarked`; exit 1. 5: no instance runs, X holding
/// the token marked standby: X yes, `marked_holding`; one incumbent yes,
/// `one`, a holder being an incumbent whatever its mark; exit 1.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn redundancy_s12_the_shapes_x_builds() {
    use zenkey::redundancy::Role;
    let bus = Bus::bare(None).await;
    // Step 3.
    let x = Shape::up(&bus.endpoint, true, false).await;
    let run = t_until(&bus, LEDGER, |r| {
        r.code == 1 && has_row(r) && t_answer(&t_row(r), "vacancy") == "yes:lasting"
    })
    .await;
    exits(&run, 1);
    let row = t_row(&run);
    assert_eq!(t_answer(&row, "one_incumbent"), "no:vacant", "{run}");
    assert_eq!(t_answer(&row, "vacancy"), "yes:lasting", "{run}");
    x.down().await;
    // Step 4.
    let a = LedgerCandidate::up(&bus.endpoint, "A", 0xa, None).await;
    a.until(Role::Serving).await;
    let x = Shape::up(&bus.endpoint, false, false).await;
    let run = t_until(&bus, LEDGER, |r| {
        r.code == 1
            && has_row(r)
            && t_row(r)["instances"]
                .as_array()
                .is_some_and(|a| a.iter().any(|i| i["instance"] == LEDGER_X))
            && t_instance(&t_row(r), LEDGER_X).1 == "yes:unmarked"
    })
    .await;
    exits(&run, 1);
    assert_eq!(
        t_instance(&t_row(&run), LEDGER_X),
        ("twin".into(), "yes:unmarked".into()),
        "{run}"
    );
    assert_eq!(t_answer(&t_row(&run), "one_incumbent"), "yes:one", "{run}");
    x.down().await;
    a.close().await;
    // Step 5.
    let x = Shape::up(&bus.endpoint, true, true).await;
    let run = t_until(&bus, LEDGER, |r| {
        r.code == 1
            && has_row(r)
            && t_row(r)["instances"]
                .as_array()
                .is_some_and(|a| a.len() == 1)
            && t_instance(&t_row(r), LEDGER_X).1 == "yes:marked_holding"
    })
    .await;
    exits(&run, 1);
    let row = t_row(&run);
    assert_eq!(
        t_instance(&row, LEDGER_X),
        ("twin".into(), "yes:marked_holding".into()),
        "{run}"
    );
    assert_eq!(t_answer(&row, "one_incumbent"), "yes:one", "{run}");
    x.down().await;
}

/// §12 step 6: A serves and B stands by on a system `hostid.v1` minted,
/// each descriptor listing the profile. Two session zids at one minted
/// address, and B recognised a standby: not counted, and the doctor says
/// so, no finding (hostid.v1 §2.12, 0.6, 0.7). T reads them as step 1.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn redundancy_s12_a_standby_on_a_minted_system() {
    use zenkey::redundancy::Role;
    let bus = Bus::bare(None).await;
    let root = tempfile::tempdir().expect("a root");
    std::fs::create_dir_all(root.path().join("var/lib")).expect("var/lib");
    let minter = zenkey::hostid::HostIdMinter::new(zenkey::hostid::HostIdSource::at(root.path()));
    let a = LedgerCandidate::up(&bus.endpoint, "A", 0xa, Some(&minter)).await;
    a.until(Role::Serving).await;
    let b = LedgerCandidate::up(&bus.endpoint, "B", 0xb, Some(&minter)).await;
    let address = a.svc.address().to_string();
    assert!(address.starts_with("h-"), "{address}");
    let run = t_until(&bus, &address, |r| {
        r.code == 0 && has_row(r) && t_row(r)["standbys"]["count"] == 1
    })
    .await;
    exits(&run, 0);
    let dr = bus
        .zenctl(&doctor_args(&[
            "--check",
            "hostid-duplicate",
            "--format",
            "json",
        ]))
        .await;
    let doc = dr.json();
    let h = check_row(&doc, "hostid-duplicate");
    assert_eq!(h["verdict"]["answer"], "not_established", "{dr}");
    let said = h["verdict"]["reason"].as_str().unwrap_or_default();
    assert!(said.contains("1 recognised standby"), "{dr}");
    assert!(said.contains(LEDGER_B), "{dr}");
    exits(&dr, 0);
    assert_eq!(b.role(), Role::Standby);
    b.close().await;
    a.close().await;
}

/// §12 step 8: `check conform`'s `redundancy-*` cases (#841, RC) against
/// step 1's pair: the declaration passes; B, marked standby with no token in both
/// reads, answers none of the probes over `ledger.v1`'s wildcards — the
/// `append` probe answered by A alone, by replier zid across the router —
/// and the case passes.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn redundancy_s12_a_probe_of_a_standby_and_of_a_twin() {
    use zenkey::redundancy::Role;
    let bus = Bus::bare(None).await;
    let a = LedgerCandidate::up(&bus.endpoint, "A", 0xa, None).await;
    a.until(Role::Serving).await;
    let b = LedgerCandidate::up(&bus.endpoint, "B", 0xb, None).await;
    let _ = t_until(&bus, LEDGER, |r| {
        r.code == 0 && has_row(r) && t_row(r)["standbys"]["count"] == 1
    })
    .await;
    let path = ledger_path();
    let args = [
        "check",
        "conform",
        LEDGER,
        "ledger.v1",
        "--contracts",
        &path,
        "--for",
        "1",
        "--timeout",
        "2",
        "--format",
        "json",
    ];
    let run = bus
        .until(&args, |r| {
            serde_json::from_str::<Value>(&r.stdout).is_ok_and(|d| {
                rows_of(&d, "case")
                    .iter()
                    .any(|c| c["case"] == "redundancy-standby")
            })
        })
        .await;
    let doc = run.json();
    let decl = conform_case(&doc, "redundancy-declaration", "ledger.v1");
    assert_eq!(decl["verdict"]["answer"], "not_established", "{run}");
    let standby = conform_case(&doc, "redundancy-standby", "service");
    assert_eq!(standby["verdict"]["answer"], "not_established", "{run}");
    let said = standby["verdict"]["reason"].as_str().unwrap_or_default();
    assert!(said.contains(LEDGER_B), "{run}");
    assert!(said.contains("@op/append"), "{run}");
    assert!(said.contains("1 session(s) answered them"), "{run}");
    // X marks itself standby and holds `append`'s queryable anyway: its
    // session's reply to the probe is the violation, by its zid.
    let x = Shape::up(&bus.endpoint, true, false).await;
    let held = x
        .session
        .declare_queryable(format!("zk2/{LEDGER}/ledger.v1/@op/append"))
        .callback(|q| {
            use zenoh::Wait;
            let _ = q.reply(q.key_expr().clone(), "X").wait();
        })
        .await
        .expect("X's queryable");
    let run = bus
        .until(&args, |r| {
            serde_json::from_str::<Value>(&r.stdout).is_ok_and(|d| {
                rows_of(&d, "case").iter().any(|c| {
                    c["case"] == "redundancy-standby" && c["verdict"]["answer"] == "established"
                })
            })
        })
        .await;
    let doc = run.json();
    let standby = conform_case(&doc, "redundancy-standby", "service");
    assert_eq!(standby["verdict"]["answer"], "established", "{run}");
    let detail = standby["detail"].as_str().unwrap_or_default();
    assert!(
        detail.contains(LEDGER_X) && detail.contains("answered the probe of @op/append"),
        "{run}"
    );
    assert!(
        !detail.contains(LEDGER_B),
        "B stands by as it should: {run}"
    );
    exits(&run, 1);
    drop(held);
    x.down().await;
    b.close().await;
    a.close().await;
}

/// §12 step 8's second half (text 0.4, F-221, F-222): A and B on
/// `ledger-appendall.v1`, whose `appendall` allows fan-out and begins with
/// `append`. `append`'s probe, `append$*`, would reach `appendall` and run
/// it, so it is never asked: `redundancy-standby` reads unobservable,
/// naming `appendall`, and `appendall`'s handler never runs. Every case
/// skipped, `check conform` sends nothing at all: `appendall`, idempotent,
/// is not called either.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn redundancy_s12_a_probe_never_runs_an_operation_that_allows_fan_out() {
    use zenkey::redundancy::Role;
    let bus = Bus::bare(None).await;
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../zenkey/tests/contracts/ledger-appendall.v1.toml");
    let variant = {
        let l = load_path(&path);
        l.contract.unwrap_or_else(|| panic!("{}", l.report))
    };
    let path = path.to_str().expect("a UTF-8 path").to_owned();
    let a = LedgerCandidate::up_on(&bus.endpoint, "A", 0xa, None, variant.clone()).await;
    a.until(Role::Serving).await;
    let b = LedgerCandidate::up_on(&bus.endpoint, "B", 0xb, None, variant).await;
    let _ = t_until(&bus, LEDGER, |r| {
        r.code == 0 && has_row(r) && t_row(r)["standbys"]["count"] == 1
    })
    .await;
    let ran = || a.appendall.load(Ordering::SeqCst) + b.appendall.load(Ordering::SeqCst);
    // Every case skipped: nothing is sent (F-222).
    let mut every: Vec<String> = Vec::new();
    for c in zenkey_fleet::report::CaseId::ALL {
        every.extend(["--skip".to_owned(), c.as_str().to_owned()]);
    }
    let base = [
        "check",
        "conform",
        LEDGER,
        "ledger.v1",
        "--contracts",
        &path,
        "--for",
        "1",
        "--timeout",
        "2",
        "--format",
        "json",
    ];
    let mut args: Vec<&str> = base.to_vec();
    args.extend(every.iter().map(String::as_str));
    let run = bus.zenctl(&args).await;
    let doc = run.json();
    assert!(
        rows_of(&doc, "case")
            .iter()
            .all(|c| c["verdict"]["answer"] == "not_asked"),
        "{run}"
    );
    assert_eq!(ran(), 0, "a skipped case calls nothing: {run}");
    // Only redundancy-standby asked: append's probe is never made.
    let mut args: Vec<&str> = base.to_vec();
    let others: Vec<String> = zenkey_fleet::report::CaseId::ALL
        .iter()
        .filter(|c| **c != zenkey_fleet::report::CaseId::RedundancyStandby)
        .flat_map(|c| ["--skip".to_owned(), c.as_str().to_owned()])
        .collect();
    args.extend(others.iter().map(String::as_str));
    let run = bus.zenctl(&args).await;
    let doc = run.json();
    let standby = conform_case(&doc, "redundancy-standby", "service");
    assert_eq!(standby["verdict"]["answer"], "unobservable", "{run}");
    let said = standby["verdict"]["reason"].as_str().unwrap_or_default();
    assert!(
        said.contains("@op/append not probed") && said.contains("@op/appendall"),
        "{run}"
    );
    exits(&run, 2);
    assert_eq!(ran(), 0, "the probe ran no allowed operation: {run}");
    b.close().await;
    a.close().await;
}

/// §12 step 9 (text 0.5, F-226): A and B run `ledger.v1`, all the
/// address's descriptors name; T holds `ledger-appendall.v1` offline,
/// which no instance runs. `append`'s probe is still never made, its
/// reason naming `appendall` and the revision held offline: the guard
/// reads every revision in hand, not the named one alone (§2.15).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn redundancy_s12_a_probe_guarded_by_a_revision_held_offline() {
    use zenkey::redundancy::Role;
    let bus = Bus::bare(None).await;
    let a = LedgerCandidate::up(&bus.endpoint, "A", 0xa, None).await;
    a.until(Role::Serving).await;
    let b = LedgerCandidate::up(&bus.endpoint, "B", 0xb, None).await;
    let _ = t_until(&bus, LEDGER, |r| {
        r.code == 0 && has_row(r) && t_row(r)["standbys"]["count"] == 1
    })
    .await;
    let offline = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../zenkey/tests/contracts/ledger-appendall.v1.toml");
    let fp = {
        let l = load_path(&offline);
        zenkey::model::canonical::Fingerprint::of(
            &l.contract.unwrap_or_else(|| panic!("{}", l.report)),
        )
        .to_string()
    };
    let offline = offline.to_str().expect("a UTF-8 path").to_owned();
    let mut args: Vec<&str> = vec![
        "check",
        "conform",
        LEDGER,
        "ledger.v1",
        "--contracts",
        &offline,
        "--for",
        "1",
        "--timeout",
        "2",
        "--format",
        "json",
    ];
    let others: Vec<String> = zenkey_fleet::report::CaseId::ALL
        .iter()
        .filter(|c| **c != zenkey_fleet::report::CaseId::RedundancyStandby)
        .flat_map(|c| ["--skip".to_owned(), c.as_str().to_owned()])
        .collect();
    args.extend(others.iter().map(String::as_str));
    let run = bus.zenctl(&args).await;
    let doc = run.json();
    let standby = conform_case(&doc, "redundancy-standby", "service");
    assert_eq!(standby["verdict"]["answer"], "unobservable", "{run}");
    let said = standby["verdict"]["reason"].as_str().unwrap_or_default();
    assert!(
        said.contains("@op/append not probed")
            && said.contains("@op/appendall")
            && said.contains(&fp),
        "named: append, appendall and the revision held offline: {run}"
    );
    exits(&run, 2);
    b.close().await;
    a.close().await;
}
