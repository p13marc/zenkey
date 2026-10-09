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
struct Bus {
    endpoint: String,
    home: PathBuf,
    _router: zenoh::Session,
    owners: zenoh::Session,
    services: Vec<Service>,
    keep: Vec<Box<dyn std::any::Any + Send>>,
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
        static NTH: AtomicU64 = AtomicU64::new(0);
        let (router, endpoint) = router().await;
        let owners = client(&endpoint, namespace).await;
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
            owners,
            services: Vec::new(),
            keep: Vec::new(),
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
    use std::io::{Read as _, Write as _};
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
    let run = bus
        .until(&expect(&["--valid-payload"]), |r| r.code == 1)
        .await;
    exits(&run, 1);
    let doc = run.json();
    assert_eq!(doc["verdict"], "not_met", "{run}");
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
