//! `spec/profiles/config/scenarios.md` §1–§12 (`config.v1`, #768, CB), one
//! test per section, named after it. §13 (the compact status across a
//! constrained face) and §14 (a tool's answers) are a deployment's and a
//! tool's, and run as tests of the tool that reads `config.v1` (#768, CC).
//!
//! Common setup, the text's: R1 is a router with timestamping on, a
//! router's default; the owner, C, S and T are its clients. The owner is
//! `lab/rf0`, implementing `config.v1` through
//! [`ServiceBuilder::config_v1`] on `conformance/lifecycle.json`'s `rf`,
//! with a persisted layer, its floor 10 s, its `apply_at` 1 s after it
//! receives a reach `set`. Its device is the owner example's simulated one
//! (`examples/owner/config.rs`), which the black-box runner drives too: a
//! test sets each act's result, and reads what the device holds. S
//! subscribes to the owner's `config.v1/**` and its `@zk/**` tokens before
//! the owner starts, and records each delivery's arrival, key, kind and
//! payload. C calls through a [`Client`] with the call metadata
//! `{"actor": "ops", "request_id": …}` (core O7) and a 2 s timeout, and
//! GETs as core S4 says (target `All`, consolidation `Latest`).
//!
//! Each section runs on a router of its own, so the sections run at once.
//! §10's 300 s are the owner's clock moved
//! ([`Configuration::simulate_offset`]), as health.v1's suite moves a
//! clock: the key is forgotten at 300 s on the owner's clock, which is all
//! the step observes. §6 waits its 80 s, §1 its 35 s.
//!
//! §8's access control is R1's, with users authenticated by name and
//! password: `default_permission: "deny"`, `ops` granted every act but
//! `persist` and the GET of the view and the status, `admin` those and
//! `persist`, the owner and the tool everything under `zk2/**`.
//!
//! §11 runs on a tokio runtime of its own, every thread of which writes the
//! runtime's log lines into one buffer: "the owner's log" the step reads.

mod common;

#[allow(dead_code)]
#[path = "../examples/owner/config.rs"]
mod sim;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use common::{SETTLE, T, client, config, eventually, router, router_with};
use serde_json::{Value as Json, json};
use sim::{SimDevice, UndoFailure};
use zenkey::config_v1::{self, ConfigStatus, ConfigView, Configuration, DesiredConfig, Startup};
use zenkey::model::config::{ConfigChangeEvent, Outcome as Ending, Source, Time, Value};
use zenkey::model::envelope::{Detail, Envelope};
use zenkey::model::grammar::{ZkKey, parse};
use zenkey::model::template::Bindings;
use zenkey::{CallMetadata, Client, Outcome, Service, ServiceBuilder, presence};
use zenoh::query::{ConsolidationMode, QueryTarget, Reply};
use zenoh::sample::{Sample, SampleKind};

/// The owner of `scenarios.md`'s conventions.
const RF0: &str = "lab/rf0";

/// Its floor (§2.8).
const FLOOR: u64 = 10;

/// How long after a reach `set` its device applies it (§2.9).
const APPLY_AFTER: Duration = Duration::from_secs(1);

/// A call's timeout, the scenarios' (`scenarios.md`, "The bus").
const CALL: Duration = Duration::from_secs(2);

/// The scenarios' jitter allowance on loopback.
const JITTER: Duration = Duration::from_millis(500);

/// `rf`'s startup, with a persisted layer unless `persist` is false.
fn rf(persist: bool) -> Startup {
    sim::rf_startup(false, FLOOR, APPLY_AFTER, persist)
}

// --- S: the recording subscriber -----------------------------------------------

/// One delivery, as S records it.
#[derive(Debug, Clone)]
struct Rec {
    at: Instant,
    key: String,
    delete: bool,
    payload: Vec<u8>,
}

type Log = Arc<Mutex<Vec<Rec>>>;

fn push(log: &Log, x: &Sample) {
    log.lock().unwrap().push(Rec {
        at: Instant::now(),
        key: x.key_expr().as_str().to_owned(),
        delete: x.kind() == SampleKind::Delete,
        payload: x.payload().to_bytes().into_owned(),
    });
}

/// S: the owner's `config.v1/**` and its `@zk/**` tokens, declared before
/// the owner starts.
struct Recorder {
    _data: zenoh::pubsub::Subscriber<()>,
    _tokens: zenoh::pubsub::Subscriber<()>,
    log: Log,
}

async fn recorder(s: &zenoh::Session, address: &str) -> Recorder {
    let log: Log = Arc::default();
    let (a, b) = (Arc::clone(&log), Arc::clone(&log));
    let data = s
        .declare_subscriber(format!("zk2/{address}/config.v1/**"))
        .callback(move |x: Sample| push(&a, &x))
        .await
        .unwrap();
    let tokens = s
        .liveliness()
        .declare_subscriber(format!("zk2/{address}/@zk/**"))
        .callback(move |x: Sample| push(&b, &x))
        .await
        .unwrap();
    Recorder {
        _data: data,
        _tokens: tokens,
        log,
    }
}

impl Recorder {
    fn all(&self) -> Vec<Rec> {
        self.log.lock().unwrap().clone()
    }

    fn of<T: serde::de::DeserializeOwned>(&self, suffix: &str) -> Vec<(Instant, T)> {
        self.all()
            .into_iter()
            .filter(|r| !r.delete && r.key.contains(suffix))
            .map(|r| (r.at, serde_json::from_slice(&r.payload).expect(suffix)))
            .collect()
    }

    fn events(&self) -> Vec<ConfigChangeEvent> {
        self.of("/config.v1/events/change/")
            .into_iter()
            .map(|(_, e)| e)
            .collect()
    }

    fn statuses(&self) -> Vec<(Instant, ConfigStatus)> {
        self.of("/config.v1/state/status")
    }

    fn views(&self) -> Vec<(Instant, ConfigView)> {
        self.of("/config.v1/state/view")
    }

    /// Waits until the log holds what `f` looks for.
    async fn until(&self, what: &str, f: impl Fn(&Recorder) -> bool) {
        eventually(what, || async { f(self) }).await;
    }

    /// The deletes S saw of the view or the status: never, while the owner
    /// runs (§2.3).
    fn state_deletes(&self) -> usize {
        self.all()
            .iter()
            .filter(|r| r.delete && r.key.contains("/config.v1/state/"))
            .count()
    }
}

// --- the owner ---------------------------------------------------------------------

/// An `rf` owner, started.
struct Rf {
    address: String,
    svc: Service,
    cfg: Configuration,
    device: Arc<SimDevice>,
    startup: Startup,
}

async fn rf_owner(
    session: &zenoh::Session,
    address: &str,
    startup: Startup,
    device: Arc<SimDevice>,
) -> Rf {
    let mut b = ServiceBuilder::new(session, config(address));
    let cfg = b
        .config_v1(
            startup.clone(),
            Arc::clone(&device) as Arc<dyn config_v1::Device>,
        )
        .await
        .unwrap();
    let svc = b.start().await.unwrap();
    Rf {
        address: address.to_owned(),
        svc,
        cfg,
        device,
        startup,
    }
}

/// `rf_owner` with a fresh device.
async fn rf_fresh(session: &zenoh::Session, address: &str, persist: bool) -> Rf {
    let startup = rf(persist);
    let device = Arc::new(SimDevice::new(&startup));
    rf_owner(session, address, startup, device).await
}

impl Rf {
    /// §2.12: the process stops and starts again. The old instance goes
    /// first; the device runs its startup values again; the new instance
    /// starts from them, the persisted layer, and the change events of the
    /// last hour.
    async fn restart(self, session: &zenoh::Session) -> Rf {
        let events = self.cfg.published().await;
        self.svc.close().await.unwrap();
        self.device.restart(&self.startup);
        let next = self.device.next_startup(&self.startup, events);
        rf_owner(session, &self.address, next, self.device).await
    }

    fn instance(&self) -> String {
        self.svc.instance().to_string()
    }
}

/// Waits until `tool` sees an instance token of `address`; returns its id.
async fn present(tool: &zenoh::Session, address: &str) -> String {
    let deadline = Instant::now() + SETTLE;
    loop {
        let rx = tool
            .liveliness()
            .get(format!("zk2/{address}/@zk/instance/*"))
            .timeout(T)
            .with(flume::unbounded::<Reply>())
            .await
            .unwrap();
        while let Ok(r) = rx.recv_async().await {
            if let Ok(s) = r.into_result()
                && let Ok(ZkKey::Instance { instance, .. }) = parse(s.key_expr().as_str())
            {
                return instance.to_string();
            }
        }
        assert!(Instant::now() < deadline, "never: {address} present");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

// --- C: calls and GETs -------------------------------------------------------------

/// C's call of `resource` at `address`, with `{"actor", "request_id"}`.
async fn call_as(
    s: &zenoh::Session,
    address: &str,
    resource: &str,
    group: Option<&str>,
    body: &Json,
    actor: &str,
    request_id: &str,
) -> Outcome {
    let c = Client::new(s, config_v1::implementation().shared_contract(), &[address])
        .unwrap()
        .with_timeout(CALL)
        .with_metadata(CallMetadata::new(actor, request_id));
    let values: Bindings = group
        .map(|g| [("group".to_owned(), vec![g.to_owned()])].into())
        .unwrap_or_default();
    c.call_value(&address.parse().unwrap(), resource, &values, body)
        .await
        .unwrap()
}

/// `set/{group}` as `ops`.
async fn set(s: &zenoh::Session, address: &str, group: &str, body: Json, rid: &str) -> Outcome {
    call_as(s, address, config_v1::SET, Some(group), &body, "ops", rid).await
}

/// A control operation as `ops`.
async fn control(s: &zenoh::Session, address: &str, op: &str, body: Json, rid: &str) -> Outcome {
    call_as(s, address, op, None, &body, "ops", rid).await
}

/// The value replied, as JSON; anything else fails the test.
fn value(o: &Outcome) -> Json {
    match o {
        Outcome::Value(a) => a.value::<Json>().unwrap(),
        other => panic!("a value reply, not {other:?}"),
    }
}

/// The refusal replied; anything else fails the test.
fn refusal(o: &Outcome) -> &Envelope {
    match o {
        Outcome::Refused(e) => e,
        other => panic!("a refusal, not {other:?}"),
    }
}

/// G: an S4 GET of one of the owner's state keys (core S4).
async fn get(s: &zenoh::Session, key: &str) -> Option<Json> {
    let rx = s
        .get(key)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::Latest)
        .timeout(T)
        .with(flume::unbounded::<Reply>())
        .await
        .unwrap();
    let mut out = None;
    while let Ok(r) = rx.recv_async().await {
        if let Ok(sample) = r.into_result()
            && sample.kind() == SampleKind::Put
        {
            out = Some(serde_json::from_slice(&sample.payload().to_bytes()).unwrap());
        }
    }
    out
}

async fn get_view(s: &zenoh::Session, address: &str) -> ConfigView {
    let v = get(s, &format!("zk2/{address}/config.v1/state/view"))
        .await
        .expect("the view answers");
    serde_json::from_value(v).unwrap()
}

async fn get_status(s: &zenoh::Session, address: &str) -> ConfigStatus {
    let v = get(s, &format!("zk2/{address}/config.v1/state/status"))
        .await
        .expect("the status answers");
    serde_json::from_value(v).unwrap()
}

/// A parameter as a view shows it.
fn param<'v>(v: &'v ConfigView, group: &str, name: &str) -> &'v zenkey::model::config::Parameter {
    v.groups
        .iter()
        .find(|g| g.name == group)
        .and_then(|g| g.parameters.iter().find(|p| p.name == name))
        .unwrap_or_else(|| panic!("{group}.{name} in the view"))
}

fn int(v: i64) -> Option<Value> {
    Some(Value::Integer(v))
}

/// An instant of the wire, as this host's clock reads it (the owner's is
/// this host's, §2.8).
fn instant(s: &str) -> SystemTime {
    let t = Time::parse(s).unwrap_or_else(|| panic!("{s:?} is RFC 3339"));
    SystemTime::UNIX_EPOCH + Duration::from_millis(u64::try_from(t.millis()).unwrap())
}

/// `|a − b|`.
fn apart(a: SystemTime, b: SystemTime) -> Duration {
    a.duration_since(b)
        .or_else(|_| b.duration_since(a))
        .unwrap_or_default()
}

/// A log writer into a shared buffer.
#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Buffer {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The owners' log: every `zenkey::config_v1` line this suite's owners
/// write, each naming its owner's service and instance. Installed as the
/// process's subscriber before any owner runs, as every section's bus is
/// made.
fn logs() -> &'static Buffer {
    static LOGS: std::sync::OnceLock<Buffer> = std::sync::OnceLock::new();
    LOGS.get_or_init(|| {
        let buffer = Buffer::default();
        let w = buffer.clone();
        let _ = tracing::subscriber::set_global_default(
            tracing_subscriber::fmt()
                .with_writer(move || w.clone())
                .with_ansi(false)
                .without_time()
                .with_env_filter(tracing_subscriber::EnvFilter::new(
                    "zenkey::config_v1=trace",
                ))
                .finish(),
        );
        buffer
    })
}

/// A bus for one section: R1, the owner's session, C, and S's session.
struct Bus {
    _r1: zenoh::Session,
    owner: zenoh::Session,
    c: zenoh::Session,
    s: zenoh::Session,
}

async fn bus() -> Bus {
    logs();
    let (r1, ep) = router(None).await;
    Bus {
        _r1: r1,
        owner: client(&ep).await,
        c: client(&ep).await,
        s: client(&ep).await,
    }
}

// --- the sections ---------------------------------------------------------------

/// §1 A hot set, read back (§2.3, §2.9, §2.17).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_a_hot_set_read_back() {
    let bus = bus().await;
    let s = recorder(&bus.s, RF0).await;
    let tokens = bus
        .c
        .liveliness()
        .declare_subscriber("zk2/lab/*/@zk/**")
        .history(true)
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();

    // 1. The owner starts; T GETs V and St the moment it sees the token.
    let owner = bus.owner.clone();
    let starting = tokio::spawn(async move { rf_fresh(&owner, RF0, true).await });
    let (v0, st0) = loop {
        let t = tokio::time::timeout(SETTLE, tokens.recv_async())
            .await
            .expect("a token")
            .unwrap();
        if t.kind() == SampleKind::Put
            && matches!(parse(t.key_expr().as_str()), Ok(ZkKey::Instance { .. }))
        {
            break (get_view(&bus.c, RF0).await, get_status(&bus.c, RF0).await);
        }
    };
    let rf0 = starting.await.unwrap();
    assert_eq!((v0.revision, v0.confirm_floor_s), (0, FLOOR));
    assert_eq!((v0.pending.as_ref(), v0.last_change.as_ref()), (None, None));
    assert_eq!(v0.instance, rf0.instance());
    let writable: Vec<(&str, bool)> = v0
        .groups
        .iter()
        .map(|g| (g.name.as_str(), g.writable))
        .collect();
    assert_eq!(
        writable,
        [
            ("duty", true),
            ("radio", true),
            ("link", true),
            ("sim", true),
            ("queue", false)
        ]
    );
    let pin = param(&v0, "sim", "pin");
    assert_eq!((pin.value.as_ref(), pin.sensitive), (None, true));
    assert_eq!(
        st0,
        ConfigStatus::of(&v0),
        "the status is the view's projection"
    );

    // 2. A hot set without a window.
    let reply = set(
        &bus.c,
        RF0,
        "duty",
        json!({"values": {"fraction_ppm": 20000}}),
        "r1",
    )
    .await;
    let applied: ConfigView = serde_json::from_value(value(&reply)["applied"].clone()).unwrap();
    let duty = param(&applied, "duty", "fraction_ppm");
    assert_eq!(
        (duty.value.clone(), duty.source, duty.startup.clone()),
        (int(20000), Some(Source::Runtime), int(10000))
    );
    assert_eq!(applied.revision, 1);
    let last = applied.last_change.clone().expect("the last change");
    assert_eq!(last.groups, ["duty"]);
    s.until("the applied event, and V and St at revision 1", |s| {
        !s.events().is_empty()
            && s.views().iter().any(|(_, v)| v.revision == 1)
            && s.statuses().iter().any(|(_, v)| v.revision == 1)
    })
    .await;
    let events = s.events();
    assert_eq!(events.len(), 1);
    let e = &events[0];
    assert_eq!(
        (e.outcome, e.token.as_str()),
        (Ending::Applied, last.token.as_str())
    );
    assert_eq!(
        (e.edits[0].old.clone(), e.edits[0].new.clone()),
        (int(10000), int(20000))
    );
    assert_eq!(
        (e.actor.as_deref(), e.request_id.as_deref()),
        (Some("ops"), Some("r1"))
    );
    let once = |n: usize| n == 1;
    assert!(once(
        s.views().iter().filter(|(_, v)| v.revision == 1).count()
    ));
    assert!(once(
        s.statuses().iter().filter(|(_, v)| v.revision == 1).count()
    ));

    // 3. V and St answer revision 1, and agree (§2.19).
    let (v1, st1) = (get_view(&bus.c, RF0).await, get_status(&bus.c, RF0).await);
    assert_eq!((v1.revision, st1.revision), (1, 1));
    assert_eq!(st1, ConfigStatus::of(&v1));

    // 4. Nothing put for 35 s: both re-put within 30 s, unchanged, and
    // never deleted (freshness.v1 §2.4).
    let at = s
        .views()
        .iter()
        .find(|(_, v)| v.revision == 1)
        .map(|(t, _)| *t)
        .unwrap();
    tokio::time::sleep(Duration::from_secs(35).saturating_sub(at.elapsed())).await;
    let again = |xs: Vec<(Instant, Json)>| -> Vec<Duration> {
        xs.iter()
            .filter(|(t, _)| *t > at)
            .map(|(t, _)| t.duration_since(at))
            .collect()
    };
    let views: Vec<(Instant, Json)> = s.of("/config.v1/state/view");
    let statuses: Vec<(Instant, Json)> = s.of("/config.v1/state/status");
    let re_views = again(views.clone());
    let re_statuses = again(statuses.clone());
    assert!(
        re_views
            .first()
            .is_some_and(|d| *d <= Duration::from_secs(30)),
        "{re_views:?}"
    );
    assert!(
        re_statuses
            .first()
            .is_some_and(|d| *d <= Duration::from_secs(30) + JITTER),
        "{re_statuses:?}"
    );
    let last_view = views.last().unwrap().1.clone();
    assert_eq!(serde_json::to_value(&v1).unwrap(), last_view, "unchanged");
    assert_eq!(s.state_deletes(), 0);
    rf0.svc.close().await.unwrap();
}

/// §2 A reach set, confirmed (§2.8, §2.9, §2.10).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_a_reach_set_confirmed() {
    let bus = bus().await;
    let s = recorder(&bus.s, RF0).await;
    let rf0 = rf_fresh(&bus.owner, RF0, true).await;
    present(&bus.c, RF0).await;

    // 1. A pending reply, before the apply.
    let sent = SystemTime::now();
    let reply = set(
        &bus.c,
        RF0,
        "radio",
        json!({"values": {"frequency_hz": 869525000}, "confirm_s": 120}),
        "r1",
    )
    .await;
    let (arrived, arrived_mono) = (SystemTime::now(), Instant::now());
    let pending = value(&reply)["pending"].clone();
    let token = pending["token"].as_str().expect("a token").to_owned();
    let (apply_at, deadline) = (
        instant(pending["apply_at"].as_str().unwrap()),
        instant(pending["deadline"].as_str().unwrap()),
    );
    assert_eq!(
        deadline.duration_since(apply_at).unwrap(),
        Duration::from_secs(119),
        "apply_at 1 s and the deadline 120 s after the owner received it"
    );
    assert!(apply_at >= sent + APPLY_AFTER - JITTER && apply_at <= arrived + APPLY_AFTER);
    assert_eq!(rf0.device.holds("radio", "frequency_hz"), int(868_100_000));

    // 2. Before apply_at: busy, not yet applied (§2.9).
    let early = control(
        &bus.c,
        RF0,
        config_v1::CONFIRM,
        json!({"token": token}),
        "r2",
    )
    .await;
    assert_eq!(refusal(&early).code, "busy");

    // 3. Once St shows it applied: confirmed.
    s.until("St applied", |s| {
        s.statuses().iter().any(|(_, st)| {
            st.pending.as_ref().is_some_and(|p| p.applied)
                && st.values["radio"]["frequency_hz"] == Value::Integer(869_525_000)
        })
    })
    .await;
    let apply = rf0.device.with(|d| d.applies.clone());
    assert_eq!(apply.len(), 1);
    assert!(apply[0].0 > arrived_mono, "the reply went before the apply");
    let shown: Vec<bool> = s
        .statuses()
        .iter()
        .filter_map(|(_, st)| st.pending.as_ref().map(|p| p.applied))
        .collect();
    assert_eq!(shown, [false, true], "St with applied false, then true");
    let ok = control(
        &bus.c,
        RF0,
        config_v1::CONFIRM,
        json!({"token": token}),
        "r3",
    )
    .await;
    let view: ConfigView = serde_json::from_value(value(&ok)["view"].clone()).unwrap();
    assert_eq!(view.pending, None);
    assert_eq!(view.last_change.map(|l| l.token), Some(token.clone()));
    s.until("the confirmed event", |s| !s.events().is_empty())
        .await;
    let events = s.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].outcome, Ending::Confirmed);
    assert_eq!(
        (events[0].actor.as_deref(), events[0].request_id.as_deref()),
        (Some("ops"), Some("r1")),
        "the set that started it, not the confirm (§2.17)"
    );

    // 4. Again: not_found (§2.10).
    let again = control(
        &bus.c,
        RF0,
        config_v1::CONFIRM,
        json!({"token": token}),
        "r4",
    )
    .await;
    assert_eq!(refusal(&again).code, "not_found");
    rf0.svc.close().await.unwrap();
}

/// §3 A reach set rolled back at its deadline (§2.11, §2.17).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_a_reach_set_rolled_back_at_its_deadline() {
    let bus = bus().await;
    let s = recorder(&bus.s, RF0).await;
    let s1 = recorder(&bus.s, "lab/rf1").await;
    let rf0 = rf_fresh(&bus.owner, RF0, true).await;
    let rf1 = rf_fresh(&bus.owner, "lab/rf1", true).await;
    present(&bus.c, RF0).await;
    present(&bus.c, "lab/rf1").await;
    let body = json!({"values": {"frequency_hz": 869525000}, "confirm_s": 5});

    // 1. confirm_s 5, raised to the floor (§2.8).
    let called = SystemTime::now();
    let reply = set(&bus.c, RF0, "radio", body.clone(), "r1").await;
    let pending = value(&reply)["pending"].clone();
    let token = pending["token"].as_str().unwrap().to_owned();
    let deadline = instant(pending["deadline"].as_str().unwrap());
    assert!(apart(deadline, called + Duration::from_secs(FLOOR)) <= JITTER);

    // 3. rf1's device refuses the apply at apply_at.
    rf1.device
        .with(|d| d.refuse_apply = Some("no lock on 869.525 MHz".into()));
    let r1 = set(&bus.c, "lab/rf1", "radio", body, "r1").await;
    let rf1_apply_at = instant(value(&r1)["pending"]["apply_at"].as_str().unwrap());

    // 2. 10.5 s after the call.
    tokio::time::sleep(
        Duration::from_millis(10_500).saturating_sub(called.elapsed().unwrap_or_default()),
    )
    .await;
    let events = s.events();
    assert_eq!(events.len(), 1, "one event before the GET");
    assert_eq!(
        (events[0].outcome, events[0].token.as_str()),
        (Ending::RolledBack, token.as_str())
    );
    let st = get_status(&bus.c, RF0).await;
    assert_eq!(
        st.values["radio"]["frequency_hz"],
        Value::Integer(868_100_000)
    );
    assert_eq!(st.revision, 2, "the apply and the undo");
    let v = get_view(&bus.c, RF0).await;
    assert_eq!(
        param(&v, "radio", "frequency_hz").source,
        Some(Source::File)
    );
    assert_eq!(rf0.device.holds("radio", "frequency_hz"), int(868_100_000));
    let late = control(
        &bus.c,
        RF0,
        config_v1::CONFIRM,
        json!({"token": token}),
        "r2",
    )
    .await;
    assert_eq!(refusal(&late).code, "not_found");
    assert_eq!(rf0.device.with(|d| d.undos.len()), 1, "exactly one undo");

    // 3. rf1: one rolled-back event at apply_at, and no undo made.
    let ended = s1.events();
    assert_eq!(ended.len(), 1);
    assert_eq!(
        (ended[0].outcome, ended[0].revision),
        (Ending::RolledBack, 0)
    );
    let at = s1
        .all()
        .into_iter()
        .find(|r| r.key.contains("/events/change/"))
        .unwrap();
    let ulid = at.key.rsplit('/').next().unwrap();
    let published = zenkey::writer::ulid_time(ulid).unwrap();
    assert!(apart(published, rf1_apply_at) <= JITTER, "at apply_at");
    assert_eq!(
        rf1.device.with(|d| (d.applies.len(), d.undos.len())),
        (0, 0)
    );
    rf0.svc.close().await.unwrap();
    rf1.svc.close().await.unwrap();
}

/// §4 An undo that fails (§2.11). `lab/rf0`'s device reads back what it
/// holds; `lab/rf4`'s reads back nothing, so the value is unknown.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_an_undo_that_fails() {
    let bus = bus().await;
    let s = recorder(&bus.s, RF0).await;
    let s4 = recorder(&bus.s, "lab/rf4").await;
    let rf0 = rf_fresh(&bus.owner, RF0, true).await;
    let rf4 = rf_fresh(&bus.owner, "lab/rf4", true).await;
    present(&bus.c, RF0).await;
    present(&bus.c, "lab/rf4").await;
    rf0.device.with(|d| d.fail_undo = Some(UndoFailure::Held));
    rf4.device
        .with(|d| d.fail_undo = Some(UndoFailure::Unknown));

    // 1. confirm_s 10; the undo fails.
    let body = json!({"values": {"frequency_hz": 869525000}, "confirm_s": 10});
    let called = Instant::now();
    let r0 = set(&bus.c, RF0, "radio", body.clone(), "r1").await;
    let r4 = set(&bus.c, "lab/rf4", "radio", body, "r1").await;
    let deadline = instant(value(&r0)["pending"]["deadline"].as_str().unwrap());
    assert!(value(&r4)["pending"].is_object());
    s.until("the partial event", |s| !s.events().is_empty())
        .await;
    let ended = s.events();
    assert_eq!(ended[0].outcome, Ending::Partial);
    assert!(
        apart(SystemTime::now(), deadline) <= JITTER * 2,
        "at the deadline"
    );

    // 2. 20 s after the call: the read-back is the truth, once.
    tokio::time::sleep(Duration::from_secs(20).saturating_sub(called.elapsed())).await;
    let v = get_view(&bus.c, RF0).await;
    let f = param(&v, "radio", "frequency_hz");
    assert_eq!(
        (f.value.clone(), f.source),
        (int(869_525_000), Some(Source::Runtime)),
        "as the device reports it"
    );
    assert_eq!(v.pending, None);
    let v4 = get_view(&bus.c, "lab/rf4").await;
    let f4 = param(&v4, "radio", "frequency_hz");
    assert_eq!((f4.value.as_ref(), f4.source), (None, None), "unknown");
    assert_eq!(s4.events()[0].outcome, Ending::Partial);
    for (o, n) in [(&rf0, "rf0"), (&rf4, "rf4")] {
        let undos = o.device.with(|d| d.undos.clone());
        assert_eq!(undos.len(), 1, "{n}: never retried");
    }
    rf0.svc.close().await.unwrap();
    rf4.svc.close().await.unwrap();
}

/// §5 A join, and a second writer (§2.7).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_a_join_and_a_second_writer() {
    let bus = bus().await;
    let s = recorder(&bus.s, RF0).await;
    let rf0 = rf_fresh(&bus.owner, RF0, true).await;
    present(&bus.c, RF0).await;

    // 1. A pending reply.
    let r = set(
        &bus.c,
        RF0,
        "radio",
        json!({"values": {"frequency_hz": 869525000}, "confirm_s": 120}),
        "r1",
    )
    .await;
    let pending = value(&r)["pending"].clone();
    let token = pending["token"].as_str().unwrap().to_owned();

    // 2. C2: busy, naming the token; C2 reads it from St.
    let busy = call_as(
        &bus.s,
        RF0,
        config_v1::SET,
        Some("link"),
        &json!({"values": {"net_id": 9}, "confirm_s": 60}),
        "c2",
        "q1",
    )
    .await;
    let e = refusal(&busy);
    assert_eq!(e.code, "busy");
    assert!(e.message.contains(&token), "{}", e.message);
    let st = get_status(&bus.s, RF0).await;
    assert_eq!(st.pending.map(|p| p.token), Some(token.clone()));

    // 3. Two joins.
    let link = set(
        &bus.c,
        RF0,
        "link",
        json!({"values": {"net_id": 7}, "token": token}),
        "r2",
    )
    .await;
    let joined = value(&link)["pending"].clone();
    assert_eq!(joined["token"], pending["token"]);
    assert_eq!(
        joined["deadline"], pending["deadline"],
        "joining moves nothing"
    );
    assert_ne!(joined["apply_at"], pending["apply_at"], "its own apply_at");
    let duty = set(
        &bus.c,
        RF0,
        "duty",
        json!({"values": {"fraction_ppm": 20000}, "token": token}),
        "r3",
    )
    .await;
    let view: ConfigView = serde_json::from_value(value(&duty)["applied"].clone()).unwrap();
    assert_eq!(view.pending.unwrap().groups, ["radio", "link", "duty"]);

    // 4. Confirmed once applied: one event, the three groups' edits.
    s.until("St applied", |s| {
        s.statuses()
            .last()
            .and_then(|(_, st)| st.pending.as_ref())
            .is_some_and(|p| p.applied)
    })
    .await;
    assert!(s.events().is_empty(), "no event at either join");
    let ok = control(
        &bus.c,
        RF0,
        config_v1::CONFIRM,
        json!({"token": token}),
        "r4",
    )
    .await;
    assert!(value(&ok)["view"].is_object());
    s.until("the confirmed event", |s| !s.events().is_empty())
        .await;
    let events = s.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].outcome, Ending::Confirmed);
    let edits: Vec<(&str, &str, Option<Value>, Option<Value>)> = events[0]
        .edits
        .iter()
        .map(|e| {
            (
                e.group.as_str(),
                e.parameter.as_str(),
                e.old.clone(),
                e.new.clone(),
            )
        })
        .collect();
    assert_eq!(
        edits,
        [
            ("radio", "frequency_hz", int(868_100_000), int(869_525_000)),
            ("link", "net_id", int(1), int(7)),
            ("duty", "fraction_ppm", int(10_000), int(20_000)),
        ]
    );
    rf0.svc.close().await.unwrap();
}

/// §6 An extend (§2.10).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s6_an_extend() {
    let bus = bus().await;
    let s = recorder(&bus.s, RF0).await;
    let rf0 = rf_fresh(&bus.owner, RF0, true).await;
    present(&bus.c, RF0).await;

    // 1. A hot change under a 30 s window.
    let step1 = (SystemTime::now(), Instant::now());
    let r = set(
        &bus.c,
        RF0,
        "duty",
        json!({"values": {"fraction_ppm": 20000}, "confirm_s": 30}),
        "r1",
    )
    .await;
    let view: ConfigView = serde_json::from_value(value(&r)["applied"].clone()).unwrap();
    let pending = view.pending.expect("pending");
    let token = pending.token.clone();
    assert!(
        apart(
            instant(&pending.deadline),
            step1.0 + Duration::from_secs(30)
        ) <= JITTER
    );

    // 2. 20 s later: extended to 60 s from now, answered with the status.
    tokio::time::sleep(Duration::from_secs(20).saturating_sub(step1.1.elapsed())).await;
    let extended_at = SystemTime::now();
    let r = control(
        &bus.c,
        RF0,
        config_v1::EXTEND,
        json!({"token": token, "confirm_s": 60, "answer": "status"}),
        "r2",
    )
    .await;
    let st: ConfigStatus = serde_json::from_value(value(&r)["status"].clone()).unwrap();
    let deadline = instant(&st.pending.expect("still pending").deadline);
    assert!(apart(deadline, extended_at + Duration::from_secs(60)) <= JITTER);
    assert!(s.events().is_empty(), "no event");

    // 3. The undo at the new deadline: 80 s after step 1, ± 0.5 s.
    tokio::time::sleep(Duration::from_secs(78).saturating_sub(step1.1.elapsed())).await;
    assert!(s.events().is_empty(), "not at the first deadline");
    s.until("the rolled-back event", |s| !s.events().is_empty())
        .await;
    let at = s
        .all()
        .into_iter()
        .find(|r| r.key.contains("/events/change/"))
        .unwrap()
        .at;
    let after = at.duration_since(step1.1);
    assert!(
        after >= Duration::from_secs(80) - JITTER && after <= Duration::from_secs(80) + JITTER,
        "{after:?}"
    );
    assert_eq!(s.events()[0].outcome, Ending::RolledBack);
    rf0.svc.close().await.unwrap();
}

/// §7 A restart during a pending change (§2.6, §2.12).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_a_restart_during_a_pending_change() {
    let bus = bus().await;
    let s = recorder(&bus.s, RF0).await;
    let rf0 = rf_fresh(&bus.owner, RF0, true).await;
    let old = present(&bus.c, RF0).await;

    // 1. A pending reach change, applied.
    let r = set(
        &bus.c,
        RF0,
        "radio",
        json!({"values": {"frequency_hz": 869525000}, "confirm_s": 120}),
        "r1",
    )
    .await;
    let token = value(&r)["pending"]["token"].as_str().unwrap().to_owned();
    s.until("St applied", |s| {
        s.statuses()
            .last()
            .and_then(|(_, st)| st.pending.as_ref())
            .is_some_and(|p| p.applied)
    })
    .await;
    assert_eq!(rf0.device.holds("radio", "frequency_hz"), int(869_525_000));

    // 2. The process stops and starts again; the new instance's V and St,
    // GET the moment its token appears.
    let tokens = bus
        .c
        .liveliness()
        .declare_subscriber("zk2/lab/rf0/@zk/instance/*")
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();
    let owner = bus.owner.clone();
    let restarting = tokio::spawn(async move { rf0.restart(&owner).await });
    let (v, st) = loop {
        let t = tokio::time::timeout(SETTLE, tokens.recv_async())
            .await
            .expect("a token")
            .unwrap();
        if t.kind() == SampleKind::Put && !t.key_expr().as_str().ends_with(&old) {
            break (get_view(&bus.c, RF0).await, get_status(&bus.c, RF0).await);
        }
    };
    let rf0 = restarting.await.unwrap();
    let new = rf0.instance();
    assert_ne!(new, old);
    let withdrawn = s
        .all()
        .iter()
        .any(|r| r.delete && r.key.ends_with(&format!("/@zk/instance/{old}")));
    assert!(withdrawn, "the old instance token withdrawn");
    assert_eq!((v.revision, v.instance.as_str()), (0, new.as_str()));
    assert_eq!((v.pending.as_ref(), v.last_change.as_ref()), (None, None));
    let f = param(&v, "radio", "frequency_hz");
    assert_eq!(
        (f.value.clone(), f.source),
        (int(868_100_000), Some(Source::File))
    );
    assert_eq!(st.instance, new);
    assert_eq!(rf0.device.holds("radio", "frequency_hz"), int(868_100_000));
    assert!(s.events().is_empty(), "nothing for the old change");

    // 3. A late confirm: not_found.
    let late = control(
        &bus.c,
        RF0,
        config_v1::CONFIRM,
        json!({"token": token}),
        "r2",
    )
    .await;
    assert_eq!(refusal(&late).code, "not_found");
    assert!(s.events().is_empty());
    rf0.svc.close().await.unwrap();
}

/// R1 for §8: users by name and password, and the grants of the setup.
async fn acl_router() -> (zenoh::Session, String, tempfile::TempDir) {
    logs();
    let dir = tempfile::tempdir().unwrap();
    let dict = dir.path().join("users.txt");
    std::fs::write(
        &dict,
        "router:router-pw\nowner:owner-pw\ntool:tool-pw\nops:ops-pw\nadmin:admin-pw\n",
    )
    .unwrap();
    let all = "[\"put\", \"delete\", \"declare_subscriber\", \"query\", \"declare_queryable\", \
               \"reply\", \"liveliness_token\", \"declare_liveliness_subscriber\", \
               \"liveliness_query\"]";
    let op = |x: &str| format!("\"zk2/lab/rf0/config.v1/{x}\"");
    let acts = [
        op("@op/set/*"),
        op("@op/confirm"),
        op("@op/cancel"),
        op("@op/extend"),
        op("state/view"),
        op("state/status"),
    ]
    .join(", ");
    // A caller sees what any client sees (declarations, tokens, replies);
    // what it may call is its grant: the query, on the keys granted.
    let seen = "[\"declare_subscriber\", \"declare_queryable\", \"reply\", \"liveliness_token\", \
                \"declare_liveliness_subscriber\", \"liveliness_query\"]";
    let everywhere = "[\"zk2/**\", \"zk2/@zk/**\", \"zk2/*/*/@zk/**\", \"zk2/*/*/*/@op/**\"]";
    let acl = format!(
        "{{ enabled: true, default_permission: \"deny\", rules: [\
         {{ id: \"everything\", permission: \"allow\", flows: [\"egress\", \"ingress\"], \
            messages: {all}, key_exprs: {everywhere} }}, \
         {{ id: \"see\", permission: \"allow\", flows: [\"egress\", \"ingress\"], \
            messages: {seen}, key_exprs: {everywhere} }}, \
         {{ id: \"acts\", permission: \"allow\", flows: [\"egress\", \"ingress\"], \
            messages: [\"query\"], key_exprs: [{acts}] }}, \
         {{ id: \"persist\", permission: \"allow\", flows: [\"egress\", \"ingress\"], \
            messages: [\"query\"], \
            key_exprs: [{}, \"zk2/lab/rf2/**\", \"zk2/lab/rf2/@zk/**\", \
            \"zk2/lab/rf2/*/@op/**\"] }}], \
         subjects: [{{ id: \"owner\", usernames: [\"owner\", \"tool\"] }}, \
            {{ id: \"ops\", usernames: [\"ops\"] }}, {{ id: \"admin\", usernames: [\"admin\"] }}], \
         policies: [{{ id: \"p-owner\", rules: [\"everything\"], subjects: [\"owner\"] }}, \
            {{ id: \"p-ops\", rules: [\"see\", \"acts\"], subjects: [\"ops\", \"admin\"] }}, \
            {{ id: \"p-admin\", rules: [\"persist\"], subjects: [\"admin\"] }}] }}",
        op("@op/persist")
    );
    let usrpwd = format!(
        "{{ user: \"router\", password: \"router-pw\", dictionary_file: {:?} }}",
        dict.display().to_string()
    );
    let (r1, ep) = router_with(
        None,
        &[("transport/auth/usrpwd", &usrpwd), ("access_control", &acl)],
    )
    .await;
    (r1, ep, dir)
}

/// A client of `ep` authenticated as `user`.
async fn as_user(ep: &str, user: &str) -> zenoh::Session {
    let mut c = zenoh::Config::default();
    c.insert_json5("mode", "\"client\"").unwrap();
    c.insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    c.insert_json5("connect/endpoints", &format!("[\"{ep}\"]"))
        .unwrap();
    c.insert_json5(
        "transport/auth/usrpwd",
        &format!("{{ user: \"{user}\", password: \"{user}-pw\" }}"),
    )
    .unwrap();
    zenoh::open(c).await.expect("an authenticated client")
}

/// §8 Persist, granted and not (§2.13, §2.21).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s8_persist_granted_and_not() {
    let (_r1, ep, _dir) = acl_router().await;
    let owner = as_user(&ep, "owner").await;
    let tool = as_user(&ep, "tool").await;
    let ops = as_user(&ep, "ops").await;
    let admin = as_user(&ep, "admin").await;
    let s = recorder(&tool, RF0).await;
    let rf0 = rf_fresh(&owner, RF0, true).await;
    let rf2 = rf_fresh(&owner, "lab/rf2", false).await;
    present(&tool, RF0).await;
    let rf2_instance = present(&tool, "lab/rf2").await;

    // 1. As admin: a set, then its persist.
    let r = call_as(
        &admin,
        RF0,
        config_v1::SET,
        Some("duty"),
        &json!({"values": {"fraction_ppm": 20000}}),
        "admin",
        "a1",
    )
    .await;
    let token = value(&r)["applied"]["last_change"]["token"]
        .as_str()
        .unwrap()
        .to_owned();
    s.until("the applied event", |s| s.events().len() == 1)
        .await;
    let p = call_as(
        &admin,
        RF0,
        config_v1::PERSIST,
        None,
        &json!({"token": token}),
        "admin",
        "a2",
    )
    .await;
    let view: ConfigView = serde_json::from_value(value(&p)["view"].clone()).unwrap();
    let duty = param(&view, "duty", "fraction_ppm");
    assert_eq!(
        (duty.value.clone(), duty.source, duty.startup.as_ref()),
        (int(20000), Some(Source::Overlay), None)
    );

    // 2. As ops: the set is answered; the persist draws nothing, silence,
    // never not_found (core O5).
    let r = call_as(
        &ops,
        RF0,
        config_v1::SET,
        Some("duty"),
        &json!({"values": {"fraction_ppm": 30000}}),
        "ops",
        "o1",
    )
    .await;
    let token2 = value(&r)["applied"]["last_change"]["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let silent = call_as(
        &ops,
        RF0,
        config_v1::PERSIST,
        None,
        &json!({"token": token2}),
        "ops",
        "o2",
    )
    .await;
    assert!(
        matches!(silent, Outcome::NoAnswer(_)),
        "silence: {silent:?}"
    );
    s.until("the second applied event", |s| s.events().len() == 2)
        .await;
    let events = s.events();
    assert!(
        events.iter().all(|e| e.outcome == Ending::Applied),
        "no event for the persist"
    );

    // 3. lab/rf2, with no persisted layer: unavailable, cause config, and
    // its descriptor says so (core §3.3).
    let r = call_as(
        &admin,
        "lab/rf2",
        config_v1::SET,
        Some("duty"),
        &json!({"values": {"fraction_ppm": 20000}}),
        "admin",
        "a3",
    )
    .await;
    let t2 = value(&r)["applied"]["last_change"]["token"].clone();
    let refused = call_as(
        &admin,
        "lab/rf2",
        config_v1::PERSIST,
        None,
        &json!({"token": t2}),
        "admin",
        "a4",
    )
    .await;
    let e = refusal(&refused);
    assert_eq!(
        (e.code.as_str(), e.cause.as_deref()),
        ("unavailable", Some("config"))
    );
    let d = presence::descriptor(
        &admin,
        &"lab/rf2".parse().unwrap(),
        &rf2_instance.parse().unwrap(),
        T,
    )
    .await
    .unwrap()
    .into_descriptor()
    .unwrap();
    let entry = d
        .interfaces
        .iter()
        .find(|i| i.iface == "config.v1")
        .expect("config.v1 in the descriptor");
    let listed: Vec<(&str, zenkey::model::descriptor::Cause)> = entry
        .unavailable
        .iter()
        .map(|u| (u.resource.as_str(), u.cause))
        .collect();
    assert_eq!(
        listed,
        [("@op/persist", zenkey::model::descriptor::Cause::Config)]
    );

    // 4. A restart: step 1's change survived, step 2's did not.
    let rf0 = rf0.restart(&owner).await;
    present(&tool, RF0).await;
    let v = get_view(&admin, RF0).await;
    let duty = param(&v, "duty", "fraction_ppm");
    assert_eq!(
        (duty.value.clone(), duty.source),
        (int(20000), Some(Source::Overlay))
    );
    rf0.svc.close().await.unwrap();
    rf2.svc.close().await.unwrap();
}

/// §9 A group outside the allowlist (§2.16).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s9_a_group_outside_the_allowlist() {
    let bus = bus().await;
    let s = recorder(&bus.s, RF0).await;
    let rf0 = rf_fresh(&bus.owner, RF0, true).await;
    present(&bus.c, RF0).await;

    // 1. Every group but queue is writable.
    let v = get_view(&bus.c, RF0).await;
    let not: Vec<&str> = v
        .groups
        .iter()
        .filter(|g| !g.writable)
        .map(|g| g.name.as_str())
        .collect();
    assert_eq!(not, ["queue"]);

    // 2. and 3. unavailable, cause config: the allowlist before the values.
    for body in [
        json!({"values": {"tx_queue_len": 10}}),
        json!({"values": {"nope": 1}}),
    ] {
        let r = set(&bus.c, RF0, "queue", body, "r1").await;
        let e = refusal(&r);
        assert_eq!(
            (e.code.as_str(), e.cause.as_deref()),
            ("unavailable", Some("config"))
        );
    }
    let after = get_view(&bus.c, RF0).await;
    assert_eq!(after, v, "nothing changed");
    assert!(s.events().is_empty(), "no event");
    rf0.svc.close().await.unwrap();
}

/// §10 An idempotent retry (§2.14).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s10_an_idempotent_retry() {
    let bus = bus().await;
    let s = recorder(&bus.s, RF0).await;
    let rf0 = rf_fresh(&bus.owner, RF0, true).await;
    present(&bus.c, RF0).await;
    let body =
        json!({"values": {"frequency_hz": 869525000}, "confirm_s": 60, "idempotency_key": "k1"});

    // 1. The reply is dropped before it reaches C: C's query goes, and
    // whatever comes back is discarded.
    let step1 = Instant::now();
    let key = "zk2/lab/rf0/config.v1/@op/set/radio";
    drop(
        bus.c
            .get(key)
            .payload(serde_json::to_vec(&body).unwrap())
            .encoding(zenoh::bytes::Encoding::APPLICATION_JSON)
            .attachment(CallMetadata::new("ops", "r1").to_bytes())
            .target(QueryTarget::BestMatching)
            .timeout(CALL)
            .with(flume::bounded::<Reply>(0))
            .await
            .unwrap(),
    );
    s.until("St pending", |s| {
        s.statuses()
            .last()
            .is_some_and(|(_, st)| st.pending.is_some())
    })
    .await;
    let armed = s.statuses().last().unwrap().1.pending.clone().unwrap();

    // 2. The same request, 5 s later: step 1's reply, nothing new.
    tokio::time::sleep(Duration::from_secs(5).saturating_sub(step1.elapsed())).await;
    let puts_before = s.all().len();
    let again_at = SystemTime::now();
    let r = set(&bus.c, RF0, "radio", body.clone(), "r2").await;
    let first = value(&r)["pending"].clone();
    assert_eq!(first["token"].as_str(), Some(armed.token.as_str()));
    assert_eq!(first["deadline"].as_str(), Some(armed.deadline.as_str()));
    let apply_at = instant(first["apply_at"].as_str().unwrap());
    assert!(
        apply_at + Duration::from_secs(3) < again_at,
        "step 1's instant"
    );
    assert_eq!(rf0.device.with(|d| d.applies.len()), 1, "one apply");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let new_since: Vec<Rec> = s.all().split_off(puts_before);
    assert!(
        new_since.iter().all(|r| !r.key.contains("/events/")),
        "no event"
    );
    assert!(
        new_since
            .iter()
            .filter(|r| r.key.contains("/state/"))
            .all(|r| serde_json::from_slice::<Json>(&r.payload).unwrap()["revision"] == 1),
        "the state unchanged"
    );

    // 3. The same key, another request: invalid_request.
    let other =
        json!({"values": {"frequency_hz": 869000000}, "confirm_s": 60, "idempotency_key": "k1"});
    let r = set(&bus.c, RF0, "radio", other, "r3").await;
    assert_eq!(refusal(&r).code, "invalid_request");

    // 4. Cancelled, then 300 s after step 1 on the owner's clock: a new
    // change, with a new token.
    let r = control(
        &bus.c,
        RF0,
        config_v1::CANCEL,
        json!({"token": armed.token}),
        "r4",
    )
    .await;
    assert!(value(&r)["view"].is_object());
    let shift = Duration::from_secs(300).saturating_sub(step1.elapsed()) + Duration::from_secs(1);
    rf0.cfg
        .simulate_offset(i64::try_from(shift.as_millis()).unwrap());
    let r = set(&bus.c, RF0, "radio", body, "r5").await;
    let fresh = value(&r)["pending"].clone();
    assert_ne!(fresh["token"].as_str(), Some(armed.token.as_str()));
    rf0.svc.close().await.unwrap();
}

/// §11 Sensitive values (§2.15). "The owner's log" is its lines of the
/// suite's log ([`logs`]), each naming its instance.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s11_sensitive_values() {
    let bus = bus().await;
    let s = recorder(&bus.s, RF0).await;
    let rf0 = rf_fresh(&bus.owner, RF0, true).await;
    present(&bus.c, RF0).await;
    let mut replies: Vec<String> = Vec::new();
    let mut keep = |o: &Outcome| {
        replies.push(match o {
            Outcome::Value(a) => a.value::<Json>().unwrap().to_string(),
            Outcome::Refused(e) => format!(
                "{} {} {:?}",
                e.code,
                e.message,
                e.detail.as_ref().map(|d| match d {
                    Detail::Value(v) => v.to_string(),
                    Detail::Bytes(b) => String::from_utf8_lossy(b).into_owned(),
                })
            ),
            other => panic!("an answer, not {other:?}"),
        });
    };

    // 1. Applied, its edit redacted; then a pin of the wrong kind.
    let r = set(&bus.c, RF0, "sim", json!({"values": {"pin": "1234"}}), "r1").await;
    keep(&r);
    let token = value(&r)["applied"]["last_change"]["token"]
        .as_str()
        .unwrap()
        .to_owned();
    s.until("the applied event", |s| s.events().len() == 1)
        .await;
    let edit = serde_json::to_value(&s.events()[0].edits).unwrap();
    assert_eq!(
        edit,
        json!([{"group": "sim", "parameter": "pin", "redacted": true}])
    );
    let r = set(&bus.c, RF0, "sim", json!({"values": {"pin": 1234}}), "r2").await;
    keep(&r);
    let e = refusal(&r);
    assert_eq!(e.code, "invalid_request");
    assert!(
        e.message.contains("pin") && !e.message.contains("1234"),
        "{}",
        e.message
    );

    // 2. The device refuses a third, in words holding the PIN: no detail.
    rf0.device
        .with(|d| d.refuse_apply = Some("PIN 1234 rejected by the SIM".into()));
    let r = set(&bus.c, RF0, "sim", json!({"values": {"pin": "1234"}}), "r3").await;
    keep(&r);
    let e = refusal(&r);
    assert_eq!(e.code, "app");
    assert_eq!(
        e.detail,
        Some(Detail::Value(json!({"code": "device-refused"})))
    );

    // 3. V and St; the persist is forbidden.
    let v = get_view(&bus.c, RF0).await;
    let pin = param(&v, "sim", "pin");
    assert_eq!(
        (pin.source, pin.value.as_ref(), pin.startup.as_ref()),
        (Some(Source::Runtime), None, None)
    );
    let st = get_status(&bus.c, RF0).await;
    assert!(!st.values.contains_key("sim"));
    let p = control(
        &bus.c,
        RF0,
        config_v1::PERSIST,
        json!({"token": token}),
        "r4",
    )
    .await;
    keep(&p);
    assert_eq!(refusal(&p).code, "forbidden");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let instance = rf0.instance();
    rf0.svc.close().await.unwrap();

    // 4. The log: no line holds 1234, and a logged request writes
    // "<redacted>". Nor does any delivery or reply. The instance id is
    // random hex, and is taken out first.
    let log = String::from_utf8(logs().0.lock().unwrap().clone()).unwrap();
    let mine: Vec<&str> = log
        .lines()
        .filter(|l| l.contains(&format!("instance={instance}")))
        .collect();
    let scrub = |t: &str| t.replace(&instance, "<instance>");
    assert!(mine.len() >= 4, "the owner logged its calls: {mine:?}");
    for line in &mine {
        assert!(!scrub(line).contains("1234"), "{line}");
    }
    assert!(
        mine.iter()
            .any(|l| l.contains("config.v1: set") && l.contains("\"pin\":\"<redacted>\"")),
        "{mine:?}"
    );
    for r in s.all() {
        let text = scrub(&String::from_utf8_lossy(&r.payload));
        assert!(!text.contains("1234"), "{} {text}", r.key);
    }
    for r in &replies {
        assert!(!scrub(r).contains("1234"), "{r}");
    }
}

/// §12 Desired state refused (§2.18). The deployment binds `lab/rf0`'s
/// role `desired` to `lab/author`, whose state carries the document; the
/// owner's application follows it into [`Configuration::set_desired`].
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s12_desired_state_refused() {
    let bus = bus().await;
    let s = recorder(&bus.s, RF0).await;
    let author_contract = common::contract("author.v1");
    let author_iface = author_contract.iface.clone();

    // The author service.
    let mut ab = ServiceBuilder::new(&bus.c, config("lab/author"));
    ab.implement(zenkey::Implementation::new(author_contract.clone()))
        .unwrap();
    let doc_writer = ab
        .declare_state_writer(&author_iface, "state/desired", &Bindings::new())
        .await
        .unwrap();
    let author = ab.start().await.unwrap();

    // The owner, its role bound to the author, following it.
    let startup = rf(true);
    let device = Arc::new(SimDevice::new(&startup));
    let mut b = ServiceBuilder::new(&bus.owner, config(RF0).bind("desired", &["lab/author"]));
    b.require("desired", author_iface.clone(), false);
    let cfg = b
        .config_v1(startup, Arc::clone(&device) as Arc<dyn config_v1::Device>)
        .await
        .unwrap();
    let svc = b.start().await.unwrap();
    let consumer = svc.consumer("desired", Arc::new(author_contract)).unwrap();
    let follow = cfg.clone();
    let rt = tokio::runtime::Handle::current();
    let _sub = consumer
        .subscribe("state/desired", move |d| {
            let Ok(doc) = serde_json::from_slice::<DesiredConfig>(&d.sample.payload().to_bytes())
            else {
                return;
            };
            let f = follow.clone();
            rt.spawn(async move { f.set_desired(doc).await });
        })
        .await
        .unwrap();
    present(&bus.c, RF0).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let desired = |s: &Recorder| -> Option<Json> {
        s.statuses()
            .last()
            .and_then(|(_, st)| st.desired.as_ref())
            .map(|d| serde_json::to_value(d).unwrap())
    };

    // 1. Converged: one applied event.
    doc_writer
        .put_value(&json!({"revision": 1, "values": {"duty": {"fraction_ppm": 20000}}}))
        .await
        .unwrap();
    s.until("desired revision 1", |s| {
        desired(s).is_some_and(|d| d["observed_revision"] == 1)
    })
    .await;
    assert_eq!(
        desired(&s),
        Some(json!({"observed_revision": 1, "outcome": "converged"}))
    );
    let st = s.statuses().last().unwrap().1.clone();
    assert_eq!(st.values["duty"]["fraction_ppm"], Value::Integer(20000));
    let events = s.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].outcome, Ending::Applied);
    let applies = device.with(|d| d.applies.len());

    // 2. Refused whole: the reach group; the legal duty change not applied.
    doc_writer
        .put_value(&json!({"revision": 2, "values": {
            "duty": {"fraction_ppm": 30000}, "radio": {"frequency_hz": 869525000}}}))
        .await
        .unwrap();
    s.until("desired revision 2", |s| {
        desired(s).is_some_and(|d| d["observed_revision"] == 2)
    })
    .await;
    assert_eq!(
        desired(&s),
        Some(json!({"observed_revision": 2, "outcome": "refused",
            "refusal": {"code": "reach", "group": "radio"}}))
    );
    let st = s.statuses().last().unwrap().1.clone();
    assert_eq!(st.values["duty"]["fraction_ppm"], Value::Integer(20000));
    assert_eq!(device.with(|d| d.applies.len()), applies, "no act");
    assert_eq!(s.events().len(), 1, "no event");

    // 3. The sensitive parameter: refused, no act.
    doc_writer
        .put_value(&json!({"revision": 3, "values": {"sim": {"pin": "1234"}}}))
        .await
        .unwrap();
    s.until("desired revision 3", |s| {
        desired(s).is_some_and(|d| d["observed_revision"] == 3)
    })
    .await;
    assert_eq!(
        desired(&s),
        Some(json!({"observed_revision": 3, "outcome": "refused",
            "refusal": {"code": "sensitive", "group": "sim", "parameter": "pin"}}))
    );
    assert_eq!(device.with(|d| d.applies.len()), applies, "no act");
    let view = get_view(&bus.c, RF0).await;
    assert_eq!(view.desired, cfg.desired().await);
    svc.close().await.unwrap();
    author.close().await.unwrap();
}
