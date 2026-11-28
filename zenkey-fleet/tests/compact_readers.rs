//! Every profile reader of the fleet reads an owner's compact descriptor
//! (core §3.3; #980 C5b2) to the answer it reads from the owner's JSON one,
//! and, given no catalog that reads it, reads it `unread`: never a guess
//! (§8.6).
//!
//! The owner is the runtime's, unchanged, since no runtime writes the
//! compact form before C5c. So on the compact bed a router's access control
//! keeps the owner's own descriptor from every reader
//! (`compact_owner::deny_descriptors`, by its session's zid), and a raw
//! session answers its compact twin in its place (`compact_owner::answer`),
//! under a catalog of the revisions it names. Each test reads one bed of
//! each form, then the compact one with no catalog.

use std::sync::Arc;
use std::time::Duration;

mod util;
use util::zk2::{T, config};

/// An owner answering core §3.3's compact descriptor (#980's C5b), the
/// runtime's suites' and zenctl's too.
#[path = "../../zenkey/tests/common/compact_owner.rs"]
mod compact_owner;

#[allow(dead_code)]
#[path = "../../zenkey/examples/owner/config.rs"]
mod sim;

#[allow(dead_code)]
#[path = "../../zenkey/examples/owner/desired.rs"]
mod drive;

use zenkey::{Implementation, Service, ServiceBuilder, ServiceConfig};
use zenkey_fleet::Catalogs;
use zenkey_model::descriptor::Form;
use zenkey_model::grammar::Addr;
use zenkey_model::template::Bindings;

/// The owner's session's zid: what the compact bed's router keeps its
/// descriptor from readers by.
const OWNER_ZID: &str = "c5b2c5b2c5b2c5b2";

fn addr(s: &str) -> Addr {
    s.parse().expect("an address")
}

fn base() -> zenoh::Config {
    let mut c = zenoh::Config::default();
    for (k, v) in [
        ("scouting/multicast/enabled", "false"),
        ("scouting/gossip/enabled", "false"),
    ] {
        c.insert_json5(k, v).expect("config");
    }
    c
}

/// One router and its sessions: the owner's (zid [`OWNER_ZID`]), the
/// tool's, a watch session's and a raw one, which answers a compact twin.
struct Bed {
    _router: zenoh::Session,
    ep: String,
    owner: zenoh::Session,
    tool: zenoh::Session,
    watch: zenoh::Session,
    raw: zenoh::Session,
    compact: bool,
}

impl Bed {
    /// A JSON bed, or with `compact` one whose router keeps the owner's
    /// descriptor from every reader.
    async fn up(compact: bool) -> Bed {
        let mut c = base();
        c.insert_json5("mode", "\"router\"").expect("config");
        c.insert_json5("listen/endpoints", &format!("[\"{}\"]", util::ANY_PORT))
            .expect("config");
        if compact {
            c.insert_json5(
                "access_control",
                &compact_owner::deny_descriptors(OWNER_ZID),
            )
            .expect("config");
        }
        let router = zenoh::open(c).await.expect("router");
        let ep = util::bound(&router).await;
        let client = |zid: Option<&'static str>| {
            let ep = ep.clone();
            async move {
                let mut c = base();
                c.insert_json5("mode", "\"client\"").expect("config");
                c.insert_json5("connect/endpoints", &format!("[\"{ep}\"]"))
                    .expect("config");
                if let Some(z) = zid {
                    c.insert_json5("id", &format!("\"{z}\"")).expect("config");
                }
                zenoh::open(c).await.expect("client")
            }
        };
        Bed {
            owner: client(Some(OWNER_ZID)).await,
            tool: client(None).await,
            watch: client(None).await,
            raw: client(None).await,
            ep: ep.clone(),
            _router: router,
            compact,
        }
    }

    /// On the compact bed, `svc`'s compact twin answered in its place, and
    /// the catalog that reads it; on the JSON bed, nothing and no catalog.
    async fn twin(&self, svc: &Service) -> (Catalogs, Option<compact_owner::Owner>) {
        if !self.compact {
            return (Catalogs::default(), None);
        }
        let (catalog, d, bytes, _) =
            compact_owner::twin_of(&self.tool, &svc.descriptor_bytes()).await;
        assert!(bytes.len() < 64, "a compact twin of {} B", bytes.len());
        let twin = compact_owner::answer(&self.raw, &d, Form::Compact, bytes).await;
        (Catalogs::new(vec![catalog]), Some(twin))
    }
}

// ─── health.v1 and alarms.v1 ────────────────────────────────────────────────

const NODE: &str = "lab/node";

/// `lab/node`: `health.v1` (status `OK`) and `alarms.v1` (one alarm
/// firing), the key of its alarm beside it.
async fn node(owners: &zenoh::Session) -> (Service, String) {
    let mut b = ServiceBuilder::new(owners, config(NODE));
    let h = b.health().await.expect("health.v1");
    let a = b
        .alarms(zenkey::alarms::Startup::default())
        .await
        .expect("alarms.v1");
    h.set_status(zenkey::health::Level::Ok, "serving")
        .await
        .expect("a status");
    let svc = b.start().await.expect("started");
    let raised = a
        .raise(zenkey::alarms::Condition::new(
            "link_down",
            zenkey_model::alarms::Severity::Warning,
            "link down",
        ))
        .await
        .expect("raised");
    (svc, raised.alert_key)
}

/// `run_health`'s row of `lab/node`: its verdict and its descriptor answer.
async fn health(tool: &zenoh::Session, catalogs: Catalogs) -> String {
    let spec = zenkey_fleet::HealthSpec {
        contracts: Default::default(),
        timeout: T,
        window: None,
        grace: Duration::from_millis(200),
        clocks_synced: true,
        face: None,
        catalogs,
    };
    let report = zenkey_fleet::run_health(
        tool,
        "",
        zenkey_fleet::HealthTarget::One(addr(NODE)),
        spec,
        |_| {},
    )
    .await;
    let row = report
        .services
        .iter()
        .find(|r| r.address == NODE)
        .map(|r| serde_json::to_value(r).expect("a row"));
    summary(
        row.as_ref(),
        &["verdict", "status.level", "descriptor.answer"],
    )
}

/// `run_alarms`'s row of `key`: whether it fires.
async fn alarms(tool: &zenoh::Session, catalogs: Catalogs, key: &str) -> String {
    let mut spec = zenkey_fleet::AlarmSpec::new(T);
    spec.clocks_synced = true;
    spec.archives = false;
    spec.catalogs = catalogs;
    let report = zenkey_fleet::run_alarms(
        tool,
        None,
        "",
        zenkey_fleet::AlarmScope::of(&addr(NODE)),
        spec,
        zenkey_model::alarms::Severity::Warning,
        || {},
    )
    .await;
    let row = report
        .alarms
        .iter()
        .find(|r| r.alarm.as_ref().is_some_and(|a| a.alert_key == key))
        .map(|r| serde_json::to_value(r).expect("a row"));
    summary(row.as_ref(), &["firing.answer", "firing.reason"])
}

/// `f`'s answer once `ok` holds of it, within [`util::SETTLE`]; the last
/// one either way, so a failure shows it.
async fn settled<T: std::fmt::Debug, F, Fut>(what: &str, mut f: F, ok: impl Fn(&T) -> bool) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let deadline = std::time::Instant::now() + util::SETTLE;
    loop {
        let got = f().await;
        if ok(&got) {
            return got;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "never {what}: {got:?}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// The members of `row` named, each a dotted path, as JSON text; `none`
/// with no row.
fn summary(row: Option<&serde_json::Value>, members: &[&str]) -> String {
    match row {
        None => "none".to_owned(),
        Some(r) => members
            .iter()
            .map(|m| {
                let v = m.split('.').fold(r, |v, k| &v[k]);
                format!("{m}={v}")
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// health.v1's and alarms.v1's readers: the compact twin reads to the JSON
/// descriptor's answers, healthy and firing; with no catalog it is not
/// read, and the health reader does not call the service healthy on it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn health_and_alarms_read_a_compact_descriptor_as_the_json_one() {
    let json = Bed::up(false).await;
    let (_svc, key) = node(&json.owner).await;
    let want = settled(
        "the JSON owner healthy and firing",
        || async {
            (
                health(&json.tool, Catalogs::default()).await,
                alarms(&json.tool, Catalogs::default(), &key).await,
            )
        },
        |w| w.0.contains("\"healthy\"") && w.1.contains("\"yes\""),
    )
    .await;

    let bed = Bed::up(true).await;
    let (svc, key) = node(&bed.owner).await;
    let (catalogs, _twin) = bed.twin(&svc).await;
    let got = settled(
        "the compact twin read as the JSON one",
        || async {
            (
                health(&bed.tool, catalogs.clone()).await,
                alarms(&bed.tool, catalogs.clone(), &key).await,
            )
        },
        |g| *g == want,
    )
    .await;
    let unread = health(&bed.tool, Catalogs::default()).await;
    assert!(!unread.contains("\"healthy\""), "never a guess: {unread}");
    eprintln!("json {want:?}\ncompact {got:?}\nunread {unread}");
}

// ─── config.v1 ──────────────────────────────────────────────────────────────

const RF: &str = "lab/rf0";

/// `lab/rf0`, a `config.v1` owner over the example's simulated device.
async fn rf(owners: &zenoh::Session) -> Service {
    let startup = sim::rf_startup(false, 5, Duration::from_millis(50), true);
    let device = Arc::new(sim::SimDevice::new(&startup));
    let mut b = ServiceBuilder::new(owners, ServiceConfig::new(addr(RF)));
    b.config_v1(startup, device as Arc<dyn zenkey::config_v1::Device>)
        .await
        .expect("config.v1 declared");
    b.start().await.expect("the owner starts")
}

/// `run_config`'s answers of `lab/rf0`.
async fn configured(tool: &zenoh::Session, catalogs: Catalogs) -> String {
    let mut spec = zenkey_fleet::ConfigSpec::new(T);
    spec.clocks_synced = true;
    spec.catalogs = catalogs;
    let report = zenkey_fleet::run_config(tool, "", addr(RF), spec).await;
    let v = serde_json::to_value(&report).expect("a report");
    summary(
        Some(&v),
        &[
            "premise",
            "projection.answer",
            "pending.answer",
            "converged.answer",
        ],
    )
}

/// config.v1's reader: the compact twin reads to the JSON descriptor's
/// answers; with no catalog it is not read, and the answers are not the
/// JSON one's.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn config_reads_a_compact_descriptor_as_the_json_one() {
    let json = Bed::up(false).await;
    let _svc = rf(&json.owner).await;
    let want = settled(
        "the JSON owner's configuration read",
        || configured(&json.tool, Catalogs::default()),
        |w| w.contains("premise=\"listed\"") && w.contains("projection.answer=\"yes\""),
    )
    .await;
    let bed = Bed::up(true).await;
    let svc = rf(&bed.owner).await;
    let (catalogs, _twin) = bed.twin(&svc).await;
    let got = settled(
        "the compact twin read as the JSON one",
        || configured(&bed.tool, catalogs.clone()),
        |g| *g == want,
    )
    .await;
    let unread = configured(&bed.tool, Catalogs::default()).await;
    assert!(
        unread.contains("premise=\"descriptor_unread\""),
        "never a guess: {unread}"
    );
    eprintln!("json {want}\ncompact {got}\nunread {unread}");
}

// ─── desired.v1 ─────────────────────────────────────────────────────────────

/// The author, `lab/author`, on the owner's session: the descriptor the
/// compact bed answers in its place.
async fn author(owners: &zenoh::Session) -> (Service, zenkey::desired::Author) {
    let imp = Implementation::new(drive::contracts().expect("the contracts").0);
    let iface = imp.iface().clone();
    let mut b = ServiceBuilder::new(owners, ServiceConfig::new(addr("lab/author")));
    b.implement(imp).expect("tasking.v1");
    let a = b
        .desired_author(
            &iface,
            drive::TASKS,
            zenkey::desired::AuthorStartup::default(),
        )
        .await
        .expect("the author");
    (b.start().await.expect("start"), a)
}

/// `lab/worker`, its role bound to `lab/author`, on a session of its own:
/// a target lists a requirement, so its descriptor is always JSON.
async fn worker(ep: &str) -> (zenoh::Session, Service, zenkey::desired::Target) {
    let session = util::zk2::client(ep).await;
    let (tasking, worker) = drive::contracts().expect("the contracts");
    let worker = Implementation::new(worker);
    let wi = worker.iface().clone();
    let mut cfg = ServiceConfig::new(addr("lab/worker")).bind(drive::ROLE, &["lab/author"]);
    cfg.bindings.get_mut(drive::ROLE).expect("bound").params = [
        ("system".to_owned(), "self.system".to_owned()),
        ("service".to_owned(), "self.service".to_owned()),
    ]
    .into();
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
    let target = b
        .desired_target(
            drive::ROLE,
            &Implementation::new(tasking),
            drive::TASKS,
            zenkey::desired::ReportAt::new(wi, drive::REPORT),
            d as Arc<dyn zenkey::desired::Handler>,
            zenkey::desired::TargetStartup::default(),
        )
        .await
        .expect("the target");
    let svc = b.start().await.expect("start");
    (session, svc, target)
}

/// `run_desired`'s answers of `lab/worker`.
async fn desired(tool: &zenoh::Session, catalogs: Catalogs) -> String {
    let mut spec = zenkey_fleet::DesiredSpec::new(T);
    spec.catalogs = catalogs;
    spec.archives = false;
    let store = zenkey_fleet::BundleStore::new(T);
    let report = zenkey_fleet::run_desired(
        tool,
        "",
        Some(addr("lab/worker")),
        spec,
        &store,
        &zenkey_fleet::ContractSet::new(),
        |_| {},
    )
    .await;
    let v = serde_json::to_value(&report).expect("a report");
    let rows: Vec<String> = v["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            summary(
                Some(r),
                &[
                    "answers.converged.answer",
                    "answers.converged.reason",
                    "author.presence",
                    "author.why",
                ],
            )
        })
        .collect();
    format!("{} unobservable={}", rows.join("; "), v["unobservable"])
}

/// desired.v1's reader: the author's compact twin reads to the JSON
/// descriptor's answers, converged on its v1. A target lists a
/// requirement, so its own descriptor is always JSON. With no catalog the
/// author's is not read, and names no revision, and the answers, which
/// stand on the target's descriptor and the slice, are unchanged.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn desired_reads_an_authors_compact_descriptor_as_the_json_one() {
    let json = Bed::up(false).await;
    let (_a, author_json) = author(&json.owner).await;
    let (_ws, _w, _t) = worker(&json.ep).await;
    let task = br#"{"revision": 1, "mode": "run", "rate_hz": 10}"#.to_vec();
    author_json
        .put("lab/worker", task.clone())
        .await
        .expect("v1");
    let want = settled(
        "the JSON target converged",
        || desired(&json.tool, Catalogs::default()),
        |w| w.contains("converged.answer=\"yes\""),
    )
    .await;
    let bed = Bed::up(true).await;
    let (svc, author_c) = author(&bed.owner).await;
    let (_ws, _w, _t) = worker(&bed.ep).await;
    author_c.put("lab/worker", task).await.expect("v1");
    let (catalogs, _twin) = bed.twin(&svc).await;
    let got = settled(
        "the compact twin read as the JSON one",
        || desired(&bed.tool, catalogs.clone()),
        |g| *g == want,
    )
    .await;
    // Given no catalog the author's descriptor is not read, and names no
    // revision: desired.v1's answers stand on the target's own descriptor
    // and the slice, which name the author's interface, so they are the
    // JSON one's still, and nothing is read from the author's bytes.
    let unread = desired(&bed.tool, Catalogs::default()).await;
    eprintln!("json {want}\ncompact {got}\nunread {unread}");
    assert_eq!(unread, want);
    let o = zenkey_fleet::observe_presence(
        &bed.tool,
        &zenkey_fleet::PresenceScope::service(&addr("lab/author")),
        T,
    )
    .await
    .expect("a read");
    let reads: Vec<_> = o.descriptors.iter().flat_map(|d| d.values()).collect();
    assert!(
        matches!(reads.as_slice(), [zenkey_fleet::DescriptorRead::Unread(why)] if why.starts_with("unknown_catalog")),
        "never a guess: {reads:?}"
    );
}

// ─── telemetry.v1 ───────────────────────────────────────────────────────────

const METER: &str = "lab/meter";

/// `lab/meter` on `meter.v1`, its `drops` and `frames` counters written.
async fn meter(owners: &zenoh::Session) -> (Service, zenkey::writer::Writer) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../zenkey/tests/contracts/meter.v1.toml");
    let c = zenkey_model::contract::load_path(&path)
        .contract
        .expect("meter.v1");
    let id = c.iface.clone();
    let names: Vec<String> = c
        .resources
        .iter()
        .map(zenkey::implementation::resource_name)
        .collect();
    let mut b = ServiceBuilder::new(owners, ServiceConfig::new(addr(METER)));
    b.implement(Implementation::new(c)).expect("implement");
    for n in names {
        let _ = b.expose(&id, &n);
    }
    let drops = b
        .declare_writer(&id, "stream/drops", &Bindings::new())
        .await
        .expect("drops");
    (b.start().await.expect("start"), drops)
}

/// `run_telemetry` over `lab/meter` for 2 s: the resources it found in
/// scope and their samples' count.
async fn telemetry(bed: &Bed, catalogs: Catalogs, drops: &zenkey::writer::Writer) -> String {
    let mut spec = zenkey_fleet::TelemetrySpec::new(T, Duration::from_secs(2));
    spec.catalogs = catalogs;
    let store = zenkey_fleet::BundleStore::new(T);
    let offline = zenkey_fleet::ContractSet::new();
    let watch = util::zk2::watch_of(&bed.watch, "");
    let run = zenkey_fleet::run_telemetry(
        &bed.tool,
        Some(&watch),
        "",
        zenkey_fleet::TelemetryScope::of(&addr(METER)),
        &store,
        &offline,
        spec,
        |_| {},
    );
    let put = async {
        for n in 1..=4_u64 {
            tokio::time::sleep(Duration::from_millis(400)).await;
            let _ = drops.put_value(&serde_json::json!(n)).await;
        }
    };
    let (report, ()) = tokio::join!(run, put);
    let v = serde_json::to_value(&report).expect("a report");
    let mut resources: Vec<String> = v["resources"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| summary(Some(r), &["resource"]))
        .collect();
    resources.sort();
    format!(
        "{} presence={} unobservable={}",
        resources.join("; "),
        v["presence"]["presence"],
        v["unobservable"]
    )
}

/// telemetry.v1's reader: the compact twin reads to the JSON descriptor's
/// metric resources; with no catalog it is not read, and none is in scope.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn telemetry_reads_a_compact_descriptor_as_the_json_one() {
    let json = Bed::up(false).await;
    let (_svc, drops) = meter(&json.owner).await;
    let want = telemetry(&json, Catalogs::default(), &drops).await;
    assert!(want.contains("drops"), "{want}");
    let bed = Bed::up(true).await;
    let (svc, drops) = meter(&bed.owner).await;
    let (catalogs, _twin) = bed.twin(&svc).await;
    let got = telemetry(&bed, catalogs, &drops).await;
    let unread = telemetry(&bed, Catalogs::default(), &drops).await;
    eprintln!("json {want}\ncompact {got}\nunread {unread}");
    assert_eq!(got, want);
    assert!(!unread.contains("drops"), "never a guess: {unread}");
}

// ─── why ────────────────────────────────────────────────────────────────────

/// `run_why_through` of `lab/node`'s status: where the ladder stopped,
/// and its verdict.
async fn why(tool: &zenoh::Session, catalogs: Catalogs) -> String {
    let spec = zenkey_fleet::WhySpec {
        timeout: T,
        window: Duration::from_secs(1),
    };
    let target =
        zenkey_fleet::WhyTarget::parse("zk2/lab/node/health.v1/state/status").expect("a key");
    let store = zenkey_fleet::BundleStore::new(T);
    let report = zenkey_fleet::run_why_through(tool, &store, &catalogs, "", target, spec).await;
    let v = serde_json::to_value(&report).expect("a report");
    summary(Some(&v), &["stopped_at", "verdict.answer"])
}

/// `why`'s ladder: through the compact twin it reaches where the JSON
/// descriptor lets it; with no catalog it stops short of the answer, never
/// reading the status through a guessed revision.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn why_reads_a_compact_descriptor_as_the_json_one() {
    let json = Bed::up(false).await;
    let (_svc, _) = node(&json.owner).await;
    let want = settled(
        "the JSON owner's status answered",
        || why(&json.tool, Catalogs::default()),
        |w| w.contains("stopped_at=null"),
    )
    .await;
    let bed = Bed::up(true).await;
    let (svc, _) = node(&bed.owner).await;
    let (catalogs, _twin) = bed.twin(&svc).await;
    let got = settled(
        "the compact twin read as the JSON one",
        || why(&bed.tool, catalogs.clone()),
        |g| *g == want,
    )
    .await;
    let unread = why(&bed.tool, Catalogs::default()).await;
    eprintln!("json {want}\ncompact {got}\nunread {unread}");
    assert_ne!(unread, want, "never a guess");
}
