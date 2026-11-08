//! `spec/profiles/config/scenarios-v2.md` §1–§12 (`config.v2`, #985 G2),
//! one test per section, named after it. §13 (the record across a
//! constrained face) and §14 (a tool's answers) are a deployment's and a
//! tool's, and run as tests of the tool that reads `config.v2` (G3).
//!
//! Common setup, the text's: R1 is a router with timestamping on, a
//! router's default; the owner, C, S and T are its clients. The contract is
//! the spec's test contract, `rfcfg.v1`
//! (`spec/profiles/config/conformance/contracts/`), with its packed file
//! and the standard schema. The owner is `lab/rf0`, implementing it through
//! [`ServiceBuilder::config_v2`] on the startup values of
//! `conformance/v2/lifecycle.json`, with a persisted layer, its floor 10 s,
//! its device stating `apply_at` 1 s after it receives a reach `set`. Its
//! device is the owner example's simulated one (`examples/owner/config.rs`),
//! which the black-box runner drives too. No catalog is in force, so the
//! deployment epoch is 0. S subscribes to the owner's `rfcfg.v1/**` and its
//! `@zk/**` tokens before the owner starts, and records each delivery's
//! arrival, key, kind and payload, its envelope's stamp in it; it reads each record as
//! v2.md §3 lays it out. C writes each request as its `Set<T>` or `Control`
//! record through a [`Client`], which puts it in the request form (core
//! §7.5), with the call metadata `{"actor": "ops", "request_id": …}` (core
//! O7) and a 2 s timeout, and reads each reply as the value form or the
//! error form; it GETs as core S4 says.
//!
//! Each section runs on a router of its own, so the sections run at once.
//! §10's 300 s are the owner's clock moved
//! ([`Configuration::simulate_offset`]). §6 waits its 81 s, §1 its 65 s.
//!
//! §11 writes the runtime's log lines into one buffer, every section's
//! owner's: "the owner's log" the step reads is its instance's lines.

mod common;

#[allow(dead_code)]
#[path = "../examples/owner/config.rs"]
mod sim;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use common::{SETTLE, T, client, config, eventually, router, router_with};
use serde_json::{Value as Json, json};
use sim::{SimDevice, UndoFailure};
use zenkey::config_v2::{self, Configuration, DesiredConfig, Interface, Startup, State};
use zenkey::consumer::Consumer;
use zenkey::model::config::v2::{ChangeEvent, ParamState};
use zenkey::model::config::{Outcome as Ending, Source, Value};
use zenkey::model::contract::{Contract, load_path};
use zenkey::model::envelope::Detail;
use zenkey::model::freshness::Verdict;
use zenkey::model::grammar::{ZkKey, parse};
use zenkey::model::template::Bindings;
use zenkey::model::wire::{Envelope, Stamp, read_event};
use zenkey::{CallMetadata, Client, Implementation, Outcome, Service, ServiceBuilder, presence};
use zenoh::query::{ConsolidationMode, QueryTarget, Reply};
use zenoh::sample::{Sample, SampleKind};

/// The owner of the scenarios' conventions.
const RF0: &str = "lab/rf0";

/// The config interface's id.
const IFACE: &str = "rfcfg.v1";

/// Its floor (§2.8).
const FLOOR: u16 = 10;

/// How long after a reach `set` its device states `apply_at` (§2.9).
const APPLY_AFTER: Duration = Duration::from_secs(1);

/// A call's timeout, the scenarios' (`scenarios-v2.md`, "The bus").
const CALL: Duration = Duration::from_secs(2);

/// The scenarios' jitter allowance on loopback.
const JITTER: Duration = Duration::from_millis(500);

/// The spec's test contract, `rfcfg.v1`.
fn contract() -> Arc<Contract> {
    static C: std::sync::OnceLock<Arc<Contract>> = std::sync::OnceLock::new();
    Arc::clone(C.get_or_init(|| {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../spec/profiles/config/conformance/contracts/rfcfg.v1.toml");
        let l = load_path(&path);
        Arc::new(
            l.contract
                .unwrap_or_else(|| panic!("rfcfg.v1 does not load:\n{}", l.report)),
        )
    }))
}

/// Its declaration and derived types.
fn iface() -> Interface {
    Interface::of(&contract()).expect("rfcfg.v1 declares its groups")
}

/// The owner's startup, with a persisted layer unless `persist` is false.
fn rf(persist: bool) -> Startup {
    sim::rfcfg_startup(FLOOR, APPLY_AFTER, persist)
}

/// The instant `ms` milliseconds since the Unix epoch.
fn at_ms(ms: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(u64::try_from(ms).unwrap())
}

/// `|a − b|`.
fn apart(a: SystemTime, b: SystemTime) -> Duration {
    a.duration_since(b)
        .or_else(|_| b.duration_since(a))
        .unwrap_or_default()
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

/// S: the owner's `rfcfg.v1/**` and its `@zk/**` tokens, declared before
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
        .declare_subscriber(format!("zk2/{address}/{IFACE}/**"))
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

/// A record put, as S read it.
#[derive(Debug, Clone)]
struct Put {
    at: Instant,
    stamp: Stamp,
    /// The record's bytes, after the envelope.
    bytes: Vec<u8>,
    state: State,
}

impl Recorder {
    fn all(&self) -> Vec<Rec> {
        self.log.lock().unwrap().clone()
    }

    /// Every put of `state/config`, read as v2.md §3 lays it out.
    fn records(&self) -> Vec<Put> {
        let i = iface();
        self.all()
            .into_iter()
            .filter(|r| !r.delete && r.key.ends_with(&format!("/{IFACE}/state/config")))
            .map(|r| {
                let e = Envelope::parse(&r.payload).expect("a mutation");
                Put {
                    at: r.at,
                    stamp: e.stamp,
                    bytes: e.record.to_vec(),
                    state: i.read_record(e.tag.get(), e.record, 0).expect("a record"),
                }
            })
            .collect()
    }

    /// Every change event, with its in-band stamp.
    fn events(&self) -> Vec<(Instant, Stamp, ChangeEvent)> {
        self.all()
            .into_iter()
            .filter(|r| r.key.contains(&format!("/{IFACE}/events/change/")))
            .map(|r| {
                let e = read_event(&r.key, &r.payload)
                    .expect("an event")
                    .expect("an event's envelope");
                (
                    r.at,
                    e.stamp,
                    serde_json::from_slice(e.record).expect("a ConfigChangeEvent"),
                )
            })
            .collect()
    }

    /// The last record put.
    fn last(&self) -> Option<State> {
        self.records().pop().map(|p| p.state)
    }

    /// Waits until the log holds what `f` looks for.
    async fn until(&self, what: &str, f: impl Fn(&Recorder) -> bool) {
        eventually(what, || async { f(self) }).await;
    }

    /// The deletes S saw of the record: never, while the owner runs (§2.3).
    fn record_deletes(&self) -> usize {
        self.all()
            .iter()
            .filter(|r| r.delete && r.key.contains(&format!("/{IFACE}/state/")))
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
    b.implement(Implementation::new((*contract()).clone()))
        .unwrap();
    let cfg = b
        .config_v2(
            startup.clone(),
            Arc::clone(&device) as Arc<dyn config_v2::Device>,
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
    let device = Arc::new(SimDevice::new_v2(&startup));
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
        self.device.restart_v2(&self.startup);
        let next = self.device.next_startup_v2(&self.startup, events);
        rf_owner(session, &self.address, next, self.device).await
    }

    fn instance_id(&self) -> String {
        self.svc.instance().to_string()
    }
}

/// Waits until `tool` sees an instance token of `address`, or the one of
/// `instance`; returns its id.
async fn present_as(tool: &zenoh::Session, address: &str, instance: Option<&str>) -> String {
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
                && let Ok(ZkKey::Instance { instance: seen, .. }) = parse(s.key_expr().as_str())
                && instance.is_none_or(|want| seen.to_string() == want)
            {
                return seen.to_string();
            }
        }
        assert!(Instant::now() < deadline, "never: {address} present");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn present(tool: &zenoh::Session, address: &str) -> String {
    present_as(tool, address, None).await
}

// --- C: calls and GETs -------------------------------------------------------------

/// A `set/<group>`'s record: `fields`, the `Set<T>`'s JSON reading, with
/// `dry_run` false unless given.
fn set_record(group: &str, fields: &Json) -> Vec<u8> {
    let mut f = fields.clone();
    f.as_object_mut()
        .unwrap()
        .entry("dry_run")
        .or_insert(json!(false));
    iface()
        .derived
        .set(group)
        .unwrap_or_else(|| panic!("{group} is settable"))
        .1
        .write(&f, None)
        .unwrap_or_else(|e| panic!("{f}: {e:?}"))
        .bytes
}

/// A control operation's record, `Control`.
fn control_record(token: u64, confirm_s: Option<u16>) -> Vec<u8> {
    let mut f = json!({"token": token});
    if let Some(c) = confirm_s {
        f["confirm_s"] = json!(c);
    }
    iface().derived.control().write(&f, None).unwrap().bytes
}

/// C's call of `resource` at `address` with `record`, as `actor`.
async fn call_as(
    s: &zenoh::Session,
    address: &str,
    resource: &str,
    record: Vec<u8>,
    actor: &str,
    request_id: &str,
) -> Outcome {
    let c = Client::new(s, contract(), &[address])
        .unwrap()
        .with_timeout(CALL)
        .with_metadata(CallMetadata::new(actor, request_id));
    c.call(
        &address.parse().unwrap(),
        resource,
        &Bindings::new(),
        record,
    )
    .await
    .unwrap()
}

/// `set/<group>` as `ops`.
async fn set(s: &zenoh::Session, address: &str, group: &str, fields: Json, rid: &str) -> Outcome {
    call_as(
        s,
        address,
        &config_v2::set(group),
        set_record(group, &fields),
        "ops",
        rid,
    )
    .await
}

/// A control operation as `ops`.
async fn control(
    s: &zenoh::Session,
    address: &str,
    op: &str,
    token: u64,
    confirm_s: Option<u16>,
    rid: &str,
) -> Outcome {
    call_as(s, address, op, control_record(token, confirm_s), "ops", rid).await
}

/// A value reply: its as-of mark, its record's bytes and the record read;
/// anything else fails the test.
fn value(o: &Outcome) -> (Stamp, Vec<u8>, State) {
    let Outcome::Value(a) = o else {
        panic!("a value reply, not {o:?}")
    };
    let tag = a.tag().expect("the value form");
    let bytes = a.record().expect("the value form's record");
    let state = iface()
        .read_record(tag.get(), &bytes, 0)
        .expect("the record");
    (a.mark().expect("a mark"), bytes, state)
}

/// The record a value reply carries.
fn record(o: &Outcome) -> State {
    value(o).2
}

/// A refusal in the error form: its code, cause, mark, whether a message
/// came, and its `Error` detail read; anything else fails the test.
struct Refused {
    code: String,
    cause: Option<String>,
    mark: Stamp,
    message: Option<String>,
    detail: Option<Json>,
}

fn refusal(o: &Outcome) -> Refused {
    let Outcome::Refused(r) = o else {
        panic!("a refusal, not {o:?}")
    };
    let form = r.form.expect("the error form");
    let detail = r.detail.as_ref().map(|d| {
        let Detail::Bytes(b) = d else {
            panic!("a detail in its type's own bytes: {d:?}")
        };
        iface()
            .derived
            .error()
            .read(form.tag.get(), b, None)
            .expect("an Error record")
    });
    Refused {
        code: r.code.clone(),
        cause: r.cause.clone(),
        mark: form.stamp,
        message: form.message.then(|| r.message.clone()),
        detail,
    }
}

/// G: an S4 GET of the record (core S4): its stamp, bytes and record.
async fn get_record(s: &zenoh::Session, address: &str) -> Option<(Stamp, Vec<u8>, State)> {
    let rx = s
        .get(format!("zk2/{address}/{IFACE}/state/config"))
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::None)
        .timeout(T)
        .with(flume::unbounded::<Reply>())
        .await
        .unwrap();
    let mut out = None;
    while let Ok(r) = rx.recv_async().await {
        // A snapshot's page is its entries (core §4.2; #977's E3).
        for sample in r.into_result().into_iter().flat_map(common::replies) {
            if sample.kind() != SampleKind::Put {
                continue;
            }
            let bytes = sample.payload().to_bytes();
            let e = Envelope::parse(&bytes).expect("a mutation");
            out = Some((
                e.stamp,
                e.record.to_vec(),
                iface().read_record(e.tag.get(), e.record, 0).unwrap(),
            ));
        }
    }
    out
}

async fn get(s: &zenoh::Session, address: &str) -> State {
    get_record(s, address).await.expect("the record answers").2
}

/// A parameter as a record shows it: its source, differs and value.
fn param(s: &State, group: &str, name: &str) -> (Option<Source>, bool, Option<Value>) {
    match s
        .groups
        .get(group)
        .and_then(|g| g.params.get(name))
        .unwrap_or_else(|| panic!("{group}.{name} in the record"))
    {
        ParamState::Plain {
            source,
            differs,
            value,
        } => (*source, *differs, value.clone()),
        ParamState::Sensitive { source } => (*source, false, None),
    }
}

fn int(v: i64) -> Option<Value> {
    Some(Value::Integer(v))
}

/// The writable bit of each settable group, in the contract's order.
fn writable(s: &State) -> Vec<(String, bool)> {
    iface()
        .groups
        .iter()
        .filter(|g| g.settable())
        .map(|g| {
            (
                g.name.clone(),
                s.groups[&g.name].writable.expect("a settable group's bit"),
            )
        })
        .collect()
}

/// A token's run, its high 32 bits, and its counter, its low 16 (§2.6).
fn split(token: u64) -> (u32, u16) {
    (
        u32::try_from(token >> 16).unwrap(),
        u16::try_from(token & 0xffff).unwrap(),
    )
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

/// The owners' log: every `zenkey::config_v2` line this suite's owners
/// write, each naming its owner's service and instance number.
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
                    "zenkey::config_v2=trace",
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

/// §1 A hot set, read back, and a dry run (§2.3, §2.9, §2.17).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_a_hot_set_read_back_and_a_dry_run() {
    let bus = bus().await;
    let s = recorder(&bus.s, RF0).await;
    let layout = zenkey::model::heartbeat::Layout::of(&contract());
    let tag = layout.tag;
    let (_h, beats) = common::heartbeats(
        &bus.s,
        &format!("zk2/{RF0}/freshness.v2/stream/heartbeat/{IFACE}"),
        layout,
    )
    .await;
    let tool = Consumer::for_tool(&bus.s, contract(), &[RF0], &Default::default()).unwrap();
    let sub = tool.subscribe(config_v2::RECORD, |_| {}).await.unwrap();
    let tokens = bus
        .c
        .liveliness()
        .declare_subscriber("zk2/lab/*/@zk/**")
        .history(true)
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();

    // 1. The owner starts; T GETs the record the moment it sees the token.
    let owner = bus.owner.clone();
    let starting = tokio::spawn(async move { rf_fresh(&owner, RF0, true).await });
    let r0 = loop {
        let t = tokio::time::timeout(SETTLE, tokens.recv_async())
            .await
            .expect("a token")
            .unwrap();
        if t.kind() == SampleKind::Put
            && matches!(parse(t.key_expr().as_str()), Ok(ZkKey::Instance { .. }))
        {
            break get(&bus.c, RF0).await;
        }
    };
    let rf0 = starting.await.unwrap();
    assert_eq!((r0.revision, r0.floor_s, r0.would_be), (0, FLOOR, false));
    assert_eq!((r0.pending.as_ref(), r0.last.as_ref()), (None, None));
    assert_eq!(r0.instance, rf0.cfg.instance());
    assert_eq!(
        writable(&r0),
        [
            ("duty".to_owned(), true),
            ("radio".to_owned(), true),
            ("link".to_owned(), true),
            ("sim".to_owned(), true),
            ("queue".to_owned(), false)
        ]
    );
    assert_eq!(
        r0.groups["sim"].params["pin"],
        ParamState::Sensitive {
            source: Some(Source::File)
        },
        "pin with its source alone"
    );
    // The GET the moment the token appeared found the record: it was put
    // before the tokens (§2.3).
    assert_eq!(
        s.records().first().map(|p| p.state.clone()),
        Some(r0.clone())
    );

    // 2. A hot set without a window: the record put, then the reply under
    // its seq, then the event one past it.
    let reply = set(&bus.c, RF0, "duty", json!({"fraction_ppm": 20000}), "r1").await;
    let (mark, bytes, applied) = value(&reply);
    assert_eq!(
        param(&applied, "duty", "fraction_ppm"),
        (Some(Source::Runtime), true, int(20000))
    );
    assert_eq!(applied.revision, 1);
    let last = applied.last.clone().expect("the last change");
    assert_eq!(last.groups, ["duty"]);
    s.until("the applied event", |s| !s.events().is_empty())
        .await;
    let put = s
        .records()
        .into_iter()
        .find(|p| p.state.revision == 1)
        .expect("the record at revision 1");
    assert_eq!(mark, put.stamp, "the reply's seq is the put's (§2.9)");
    assert_eq!(bytes, put.bytes, "the reply is the record put");
    let events = s.events();
    assert_eq!(events.len(), 1);
    let (_, ev_stamp, e) = &events[0];
    assert_eq!((e.outcome, e.token), (Ending::Applied, last.token));
    assert_eq!(e.instance, rf0.cfg.instance());
    assert_eq!(
        serde_json::to_value(&e.edits).unwrap(),
        json!([{"group": "duty", "parameter": "fraction_ppm", "old": 10000, "new": 20000}])
    );
    assert_eq!(
        (e.actor.as_deref(), e.request_id.as_deref()),
        (Some("ops"), Some("r1"))
    );
    assert_eq!(
        (ev_stamp.run, ev_stamp.seq.get()),
        (put.stamp.run, put.stamp.seq.get() + 1),
        "the event one past the put"
    );

    // 3. The GET answers the reply's record, byte for byte.
    let (_, got, _) = get_record(&bus.c, RF0).await.unwrap();
    assert_eq!(got, bytes);

    // 4. A dry run: the record as it would read, under the last seq minted,
    // and nothing put.
    let puts = s.records().len();
    let dry = set(
        &bus.c,
        RF0,
        "duty",
        json!({"period_s": 60, "dry_run": true}),
        "r2",
    )
    .await;
    let (dry_mark, _, would) = value(&dry);
    assert!(would.would_be);
    assert_eq!(
        param(&would, "duty", "period_s"),
        (Some(Source::Runtime), true, int(60))
    );
    assert_eq!(dry_mark, *ev_stamp, "the event's seq, the last minted");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(s.records().len(), puts, "a dry run puts nothing");
    assert_eq!(get_record(&bus.c, RF0).await.unwrap().1, bytes);

    // 5. Nothing put for 65 s: the heartbeat vouches for the record at
    // least every 30 s, and S judges it fresh past the horizon.
    let end = put.at + Duration::from_secs(65);
    tokio::time::sleep_until(end.into()).await;
    assert_eq!(
        s.records().iter().filter(|p| p.at > put.at).count(),
        0,
        "never re-put"
    );
    let seen = common::assert_cadence(&beats, put.at, end, Duration::from_secs(30), JITTER, tag);
    assert!(
        seen.iter().all(|b| b.hb.sections.iter().all(|s| matches!(
            s,
            Some(zenkey::model::heartbeat::Section {
                members: zenkey::model::heartbeat::Members::One { vouched: true },
                ..
            })
        ))),
        "each vouches for the record"
    );
    let key = format!("zk2/{RF0}/{IFACE}/state/config");
    let j = tool
        .freshness(config_v2::RECORD, &[sub.freshness(&key)])
        .unwrap();
    assert_eq!(j.verdict, Verdict::Fresh, "65 s after its put");
    assert_eq!(s.record_deletes(), 0);
    rf0.svc.close().await.unwrap();
}

/// §2 A reach set, confirmed (§2.8, §2.9, §2.10).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_a_reach_set_confirmed() {
    let bus = bus().await;
    let s = recorder(&bus.s, RF0).await;
    let rf0 = rf_fresh(&bus.owner, RF0, true).await;
    present(&bus.c, RF0).await;

    // 1. The record before the apply: pending, its times whole seconds.
    let sent = SystemTime::now();
    let reply = set(
        &bus.c,
        RF0,
        "radio",
        json!({"frequency_hz": 869525000, "confirm_s": 120}),
        "r1",
    )
    .await;
    let (arrived, arrived_mono) = (SystemTime::now(), Instant::now());
    let (mark, _, r) = value(&reply);
    let pending = r.pending.clone().expect("pending");
    assert_eq!(pending.groups, ["radio"]);
    let apply_at = pending.apply_at.expect("apply_at, while radio waits");
    assert_eq!((apply_at % 1000, pending.deadline % 1000), (0, 0));
    assert_eq!(
        pending.deadline - apply_at,
        119_000,
        "1 s and 120 s, rounded up"
    );
    let lo = sent + APPLY_AFTER;
    let hi = arrived + APPLY_AFTER + Duration::from_secs(1);
    assert!(at_ms(apply_at) >= lo && at_ms(apply_at) <= hi, "{apply_at}");
    assert_eq!(rf0.device.holds("radio", "frequency_hz"), int(868_100_000));
    let puts = s.records();
    let armed = puts.last().expect("the record put before the reply");
    assert_eq!(mark, armed.stamp);

    // 2. Before apply_at: busy, under the first put's seq.
    let early = control(&bus.c, RF0, config_v2::CONFIRM, pending.token, None, "r2").await;
    let e = refusal(&early);
    assert_eq!(e.code, "busy");
    assert_eq!(e.mark, armed.stamp, "not yet applied (§2.9)");

    // 3. Once a record shows it applied: confirmed.
    s.until("the applied record", |s| {
        s.last().is_some_and(|r| {
            r.pending.as_ref().is_some_and(|p| p.applied)
                && param(&r, "radio", "frequency_hz").2 == int(869_525_000)
        })
    })
    .await;
    let applies = rf0.device.with(|d| d.applies.clone());
    assert_eq!(applies.len(), 1);
    assert!(
        applies[0].0 > arrived_mono,
        "the reply went before the apply"
    );
    let ok = control(&bus.c, RF0, config_v2::CONFIRM, pending.token, None, "r3").await;
    let done = record(&ok);
    assert_eq!(done.pending, None);
    assert_eq!(done.last.map(|l| l.token), Some(pending.token));
    s.until("the confirmed event", |s| !s.events().is_empty())
        .await;
    let events = s.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].2.outcome, Ending::Confirmed);
    assert_eq!(
        (
            events[0].2.actor.as_deref(),
            events[0].2.request_id.as_deref()
        ),
        (Some("ops"), Some("r1")),
        "the set that started it (v1 §2.17)"
    );
    let (ok_mark, ..) = value(&ok);
    assert!(
        events[0].1.seq.get() > ok_mark.seq.get(),
        "the event after the reply"
    );

    // 4. Again: not_found.
    let again = control(&bus.c, RF0, config_v2::CONFIRM, pending.token, None, "r4").await;
    assert_eq!(refusal(&again).code, "not_found");
    rf0.svc.close().await.unwrap();
}

/// Waits until `ms` milliseconds into a second of this host's clock.
async fn into_a_second(ms: u64) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let into = u64::from(now.subsec_millis());
    let wait = if into <= ms {
        ms - into
    } else {
        1000 + ms - into
    };
    tokio::time::sleep(Duration::from_millis(wait)).await;
}

/// §3 A reach set rolled back at its deadline (§2.8, §2.11, §2.17).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_a_reach_set_rolled_back_at_its_deadline() {
    let bus = bus().await;
    let s = recorder(&bus.s, RF0).await;
    let s1 = recorder(&bus.s, "lab/rf1").await;
    let rf0 = rf_fresh(&bus.owner, RF0, true).await;
    let rf1 = rf_fresh(&bus.owner, "lab/rf1", true).await;
    present(&bus.c, RF0).await;
    present(&bus.c, "lab/rf1").await;
    let body = json!({"frequency_hz": 869525000, "confirm_s": 5});

    // 1. 300 ms into a second, confirm_s 5, raised to the floor and
    // rounded up: 10.7 s after the call, never 10.3 s.
    into_a_second(300).await;
    let called = (SystemTime::now(), Instant::now());
    let reply = set(&bus.c, RF0, "radio", body.clone(), "r1").await;
    let p = record(&reply).pending.expect("pending");
    let deadline = at_ms(p.deadline);
    let after = deadline.duration_since(called.0).unwrap();
    assert!(
        after >= Duration::from_millis(10_500) && after <= Duration::from_millis(10_800),
        "the next whole second after the receipt plus 10 s: {after:?}"
    );

    // 3. rf1's device refuses the apply at apply_at.
    rf1.device
        .with(|d| d.refuse_apply = Some("no lock on 869.525 MHz".into()));
    let r1 = set(&bus.c, "lab/rf1", "radio", body, "r1").await;
    let rf1_apply_at = at_ms(record(&r1).pending.unwrap().apply_at.unwrap());

    // 2. 11.5 s after the call: undone at the deadline, and not before.
    tokio::time::sleep(Duration::from_millis(11_500).saturating_sub(called.1.elapsed())).await;
    let undos = rf0.device.with(|d| d.undos.clone());
    assert_eq!(undos.len(), 1, "exactly one undo");
    let undone = called.0 + (undos[0].0 - called.1);
    assert!(undone >= deadline, "not before the deadline");
    assert!(
        undone.duration_since(deadline).unwrap() <= JITTER / 2,
        "at the deadline: {:?} after it",
        undone.duration_since(deadline).unwrap()
    );
    let last = s.last().unwrap();
    assert_eq!(
        param(&last, "radio", "frequency_hz"),
        (Some(Source::File), false, int(868_100_000))
    );
    assert_eq!(last.revision, 2, "the apply and the undo");
    let events = s.events();
    assert_eq!(events.len(), 1);
    assert_eq!(
        (events[0].2.outcome, events[0].2.token),
        (Ending::RolledBack, p.token)
    );
    assert_eq!(get(&bus.c, RF0).await, last);
    let late = control(&bus.c, RF0, config_v2::CONFIRM, p.token, None, "r2").await;
    assert_eq!(refusal(&late).code, "not_found");
    assert_eq!(rf0.device.holds("radio", "frequency_hz"), int(868_100_000));

    // 3. rf1: one rolled-back event at apply_at, and no undo made.
    let ended = s1.events();
    assert_eq!(ended.len(), 1);
    assert_eq!(
        (ended[0].2.outcome, ended[0].2.revision),
        (Ending::RolledBack, 0)
    );
    let at = s1
        .all()
        .into_iter()
        .find(|r| r.key.contains("/events/change/"))
        .unwrap();
    let e = read_event(&at.key, &at.payload).unwrap().unwrap();
    let published = common::instant(e.time, 0).expect("the owner's time");
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

    // 1. confirm_s 10; the undo fails: the record, then partial.
    let body = json!({"frequency_hz": 869525000, "confirm_s": 10});
    let called = Instant::now();
    let r0 = set(&bus.c, RF0, "radio", body.clone(), "r1").await;
    let r4 = set(&bus.c, "lab/rf4", "radio", body, "r1").await;
    let deadline = at_ms(record(&r0).pending.unwrap().deadline);
    assert!(record(&r4).pending.is_some());
    s.until("the partial event", |s| !s.events().is_empty())
        .await;
    let (ended_at, ended_stamp, ended) = s.events()[0].clone();
    assert_eq!(ended.outcome, Ending::Partial);
    assert!(
        apart(SystemTime::now(), deadline) <= JITTER * 2,
        "at the deadline"
    );
    let undone = s
        .records()
        .into_iter()
        .rfind(|p| p.state.pending.is_none())
        .expect("the record after the undo");
    assert!(
        undone.at <= ended_at && undone.stamp.seq.get() < ended_stamp.seq.get(),
        "the record, then the event"
    );

    // 2. 20 s after the call: the read-back is the truth, once.
    tokio::time::sleep(Duration::from_secs(20).saturating_sub(called.elapsed())).await;
    let r = get(&bus.c, RF0).await;
    assert_eq!(
        param(&r, "radio", "frequency_hz"),
        (Some(Source::Runtime), true, int(869_525_000)),
        "as the device reports it"
    );
    assert_eq!(r.pending, None);
    let r4 = get(&bus.c, "lab/rf4").await;
    assert_eq!(
        param(&r4, "radio", "frequency_hz"),
        (None, false, None),
        "neither source nor value"
    );
    assert_eq!(s4.events()[0].2.outcome, Ending::Partial);
    for (o, n) in [(&rf0, "rf0"), (&rf4, "rf4")] {
        assert_eq!(o.device.with(|d| d.undos.len()), 1, "{n}: never retried");
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

    // 1. The record, pending.
    let r = set(
        &bus.c,
        RF0,
        "radio",
        json!({"frequency_hz": 869525000, "confirm_s": 120}),
        "r1",
    )
    .await;
    let pending = record(&r).pending.expect("pending");

    // 2. C2: busy; C2 reads the token from the record it GETs.
    let busy = call_as(
        &bus.s,
        RF0,
        &config_v2::set("link"),
        set_record("link", &json!({"net_id": 9, "confirm_s": 60})),
        "c2",
        "q1",
    )
    .await;
    assert_eq!(refusal(&busy).code, "busy");
    let got = get(&bus.s, RF0).await;
    assert_eq!(got.pending.map(|p| p.token), Some(pending.token));

    // 3. Two joins.
    let link = set(
        &bus.c,
        RF0,
        "link",
        json!({"net_id": 7, "token": pending.token}),
        "r2",
    )
    .await;
    let joined = record(&link).pending.unwrap();
    assert_eq!(
        (joined.token, joined.deadline),
        (pending.token, pending.deadline),
        "joining moves nothing"
    );
    assert_eq!(joined.groups, ["radio", "link"]);
    assert!(
        joined
            .apply_at
            .is_some_and(|a| a >= pending.apply_at.unwrap()),
        "link's apply_at, the last reach group to join (§2.9)"
    );
    let duty = set(
        &bus.c,
        RF0,
        "duty",
        json!({"fraction_ppm": 20000, "token": pending.token}),
        "r3",
    )
    .await;
    assert_eq!(
        record(&duty).pending.unwrap().groups,
        ["duty", "radio", "link"],
        "the contract's order (§2.7)"
    );

    // 4. Confirmed once applied: one event, its edits in the contract's
    // order.
    s.until("the applied record", |s| {
        s.last().and_then(|r| r.pending).is_some_and(|p| p.applied)
    })
    .await;
    assert!(s.events().is_empty(), "no event at either join");
    let ok = control(&bus.c, RF0, config_v2::CONFIRM, pending.token, None, "r4").await;
    assert_eq!(record(&ok).pending, None);
    s.until("the confirmed event", |s| !s.events().is_empty())
        .await;
    let events = s.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].2.outcome, Ending::Confirmed);
    assert_eq!(
        serde_json::to_value(&events[0].2.edits).unwrap(),
        json!([
            {"group": "duty", "parameter": "fraction_ppm", "old": 10000, "new": 20000},
            {"group": "radio", "parameter": "frequency_hz", "old": 868100000, "new": 869525000},
            {"group": "link", "parameter": "net_id", "old": 1, "new": 7}
        ])
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

    // 1. A hot change under a 30 s window. Its deadline and the extend's
    // are rounded up to the second (§2.8), so the call is made 850 ms into
    // a second: rounded up, they fall 30.15 s and 81.15 s after it, within
    // the scenarios' jitter of their 30 s and 81 s.
    into_a_second(850).await;
    let step1 = (SystemTime::now(), Instant::now());
    let r = set(
        &bus.c,
        RF0,
        "duty",
        json!({"fraction_ppm": 20000, "confirm_s": 30}),
        "r1",
    )
    .await;
    let p = record(&r).pending.expect("pending");
    assert!(p.applied, "a hot group, applied");
    assert!(apart(at_ms(p.deadline), step1.0 + Duration::from_secs(30)) <= JITTER);

    // 2. 20.4 s later: the extend's receipt plus 60 s, rounded up, 81 s
    // after step 1's call.
    tokio::time::sleep(Duration::from_millis(20_400).saturating_sub(step1.1.elapsed())).await;
    let r = control(&bus.c, RF0, config_v2::EXTEND, p.token, Some(60), "r2").await;
    let extended = record(&r).pending.expect("still pending");
    assert!(
        apart(at_ms(extended.deadline), step1.0 + Duration::from_secs(81)) <= JITTER,
        "{:?}",
        at_ms(extended.deadline).duration_since(step1.0)
    );
    assert!(s.events().is_empty(), "no event");

    // 3. The undo at the new deadline, not the first: 81 s after step 1.
    tokio::time::sleep(Duration::from_secs(79).saturating_sub(step1.1.elapsed())).await;
    assert!(s.events().is_empty(), "not at the first deadline");
    s.until("the rolled-back event", |s| !s.events().is_empty())
        .await;
    let (at, _, e) = s.events()[0].clone();
    let after = at.duration_since(step1.1);
    assert!(
        after >= Duration::from_secs(81) - JITTER && after <= Duration::from_secs(81) + JITTER,
        "{after:?}"
    );
    assert_eq!(e.outcome, Ending::RolledBack);
    rf0.svc.close().await.unwrap();
}

/// §7 Tokens, and a restart during a pending change (§2.6, §2.12).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_tokens_and_a_restart_during_a_pending_change() {
    let bus = bus().await;
    let s = recorder(&bus.s, RF0).await;
    let rf0 = rf_fresh(&bus.owner, RF0, true).await;
    let old = present(&bus.c, RF0).await;

    // 1. A pending reach change, applied; the token's high 32 bits are the
    // instance number.
    let r = set(
        &bus.c,
        RF0,
        "radio",
        json!({"frequency_hz": 869525000, "confirm_s": 120}),
        "r1",
    )
    .await;
    let first = record(&r);
    let token = first.pending.as_ref().unwrap().token;
    assert_eq!(split(token).0, first.instance);
    s.until("the applied record", |s| {
        s.last().and_then(|r| r.pending).is_some_and(|p| p.applied)
    })
    .await;
    assert_eq!(rf0.device.holds("radio", "frequency_hz"), int(869_525_000));

    // 2. The process stops and starts again.
    let rf0 = rf0.restart(&bus.owner).await;
    let new = present_as(&bus.c, RF0, Some(&rf0.instance_id())).await;
    assert_ne!(new, old);
    let withdrawn = s
        .all()
        .iter()
        .any(|r| r.delete && r.key.ends_with(&format!("/@zk/instance/{old}")));
    assert!(withdrawn, "the old instance token withdrawn");
    let r = get(&bus.c, RF0).await;
    assert_ne!(r.instance, first.instance, "another instance number");
    assert_eq!(r.instance, rf0.cfg.instance());
    assert_eq!(
        (r.revision, r.pending.as_ref(), r.last.as_ref()),
        (0, None, None)
    );
    assert_eq!(
        param(&r, "radio", "frequency_hz"),
        (Some(Source::File), false, int(868_100_000))
    );
    assert!(
        s.records().iter().any(|p| p.state == r),
        "the new record put, before its tokens"
    );
    assert_eq!(rf0.device.holds("radio", "frequency_hz"), int(868_100_000));
    assert!(s.events().is_empty(), "nothing for the old change");

    // 3. A late confirm: not_found.
    let late = control(&bus.c, RF0, config_v2::CONFIRM, token, None, "r2").await;
    assert_eq!(refusal(&late).code, "not_found");

    // 4. Twice more; each run's counter starts at random.
    let mut rf0 = rf0;
    let mut counters = Vec::new();
    for i in 0..3 {
        if i > 0 {
            rf0 = rf0.restart(&bus.owner).await;
            present_as(&bus.c, RF0, Some(&rf0.instance_id())).await;
        }
        let r = set(&bus.c, RF0, "duty", json!({"fraction_ppm": 20000}), "r3").await;
        counters.push(split(record(&r).last.unwrap().token).1);
    }
    assert!(
        counters.windows(2).any(|w| w[0] != w[1]),
        "drawn at random: {counters:?}"
    );

    // 5. The counter started at 65,534: two tokens in one run, then a new
    // run, its descriptor stating it, the instance number kept.
    let before = s.records().len();
    rf0.cfg.simulate_counter_start(65_534).await;
    let instance = rf0.cfg.instance();
    let mut tokens = Vec::new();
    let mut marks = Vec::new();
    for i in 0..3 {
        let r = set(
            &bus.c,
            RF0,
            "duty",
            json!({"fraction_ppm": 30000 + i}),
            "r5",
        )
        .await;
        let (mark, _, rec) = value(&r);
        assert_eq!(rec.instance, instance, "the same instance number");
        tokens.push(split(rec.last.unwrap().token));
        marks.push(mark);
    }
    assert_eq!((tokens[0].1, tokens[1].1), (65_534, 65_535));
    assert_eq!(tokens[0].0, tokens[1].0, "one run");
    assert_ne!(
        tokens[2].0, tokens[1].0,
        "a new run before the counter passes 65,535"
    );
    assert_eq!(marks[2].run.value(), tokens[2].0, "the third reply's run");
    let d = presence::descriptor(
        &bus.c,
        &RF0.parse().unwrap(),
        &rf0.instance_id().parse().unwrap(),
        T,
    )
    .await
    .unwrap()
    .into_descriptor()
    .unwrap();
    assert_eq!(
        d.stated_run(),
        Some(marks[2].run),
        "the descriptor states it"
    );
    s.until("the three records", |s| s.records().len() >= before + 3)
        .await;
    assert!(
        s.records()[before..]
            .iter()
            .all(|p| p.state.instance == instance),
        "every record keeps the instance number across the run"
    );
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
    let op = |x: &str| format!("\"zk2/lab/rf0/{IFACE}/{x}\"");
    let acts = [
        op("@op/set/duty"),
        op("@op/set/radio"),
        op("@op/confirm"),
        op("@op/cancel"),
        op("@op/extend"),
        op("state/config"),
    ]
    .join(", ");
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
    let duty = |v: i64| set_record("duty", &json!({"fraction_ppm": v}));

    // 1. As admin: a set, then its persist, both answered with the record.
    let r = call_as(
        &admin,
        RF0,
        &config_v2::set("duty"),
        duty(20000),
        "admin",
        "a1",
    )
    .await;
    let token = record(&r).last.unwrap().token;
    s.until("the applied event", |s| s.events().len() == 1)
        .await;
    let p = call_as(
        &admin,
        RF0,
        config_v2::PERSIST,
        control_record(token, None),
        "admin",
        "a2",
    )
    .await;
    assert_eq!(
        param(&record(&p), "duty", "fraction_ppm"),
        (Some(Source::Overlay), false, int(20000))
    );

    // 2. As ops: the set is answered; the persist draws nothing.
    let r = call_as(&ops, RF0, &config_v2::set("duty"), duty(30000), "ops", "o1").await;
    let token2 = record(&r).last.unwrap().token;
    let silent = call_as(
        &ops,
        RF0,
        config_v2::PERSIST,
        control_record(token2, None),
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
    assert!(
        s.events().iter().all(|e| e.2.outcome == Ending::Applied),
        "no event for the persist"
    );

    // 3. lab/rf2, with no persisted layer: unavailable, cause config, and
    // its descriptor says so (core §3.3).
    let r = call_as(
        &admin,
        "lab/rf2",
        &config_v2::set("duty"),
        duty(20000),
        "admin",
        "a3",
    )
    .await;
    let t2 = record(&r).last.unwrap().token;
    let refused = call_as(
        &admin,
        "lab/rf2",
        config_v2::PERSIST,
        control_record(t2, None),
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
        .find(|i| i.iface == IFACE)
        .expect("rfcfg.v1 in the descriptor");
    let listed: Vec<(&str, zenkey::model::descriptor::Cause)> = entry
        .unavailable
        .iter()
        .map(|u| (u.resource.as_str(), u.cause))
        .collect();
    assert_eq!(
        listed,
        [("@op/persist", zenkey::model::descriptor::Cause::Config)]
    );
    assert!(
        d.profiles.iter().any(|p| p == "config.v2"),
        "the profile is the gate (§2.1): {:?}",
        d.profiles
    );

    // 4. A restart: step 1's change survived, step 2's did not.
    let rf0 = rf0.restart(&owner).await;
    present_as(&tool, RF0, Some(&rf0.instance_id())).await;
    let r = get(&admin, RF0).await;
    assert_eq!(
        param(&r, "duty", "fraction_ppm"),
        (Some(Source::Overlay), false, int(20000))
    );
    rf0.svc.close().await.unwrap();
    rf2.svc.close().await.unwrap();
}

/// §9 A group outside the allowlist (§2.16), and a contract group (§2.4).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s9_a_group_outside_the_allowlist() {
    let bus = bus().await;
    let s = recorder(&bus.s, RF0).await;
    let rf0 = rf_fresh(&bus.owner, RF0, true).await;
    present(&bus.c, RF0).await;

    // 1. Every settable group but queue is writable.
    let (_, before, r) = get_record(&bus.c, RF0).await.unwrap();
    let not: Vec<String> = writable(&r)
        .into_iter()
        .filter(|(_, w)| !w)
        .map(|(g, _)| g)
        .collect();
    assert_eq!(not, ["queue"]);

    // 2. unavailable, cause config; nothing changed, no event.
    let q = set(&bus.c, RF0, "queue", json!({"tx_queue_len": 10}), "r1").await;
    let e = refusal(&q);
    assert_eq!(
        (e.code.as_str(), e.cause.as_deref()),
        ("unavailable", Some("config"))
    );
    assert_eq!(get_record(&bus.c, RF0).await.unwrap().1, before);
    assert!(s.events().is_empty(), "no event");

    // 3. set/modulation: no queryable answers, silence.
    let mut request = vec![0x40];
    request.extend_from_slice(&[0]);
    let rx = bus
        .c
        .get(format!("zk2/{RF0}/{IFACE}/@op/set/modulation"))
        .payload(request)
        .encoding(zenoh::bytes::Encoding::ZENOH_BYTES)
        .target(QueryTarget::BestMatching)
        .timeout(CALL)
        .with(flume::unbounded::<Reply>())
        .await
        .unwrap();
    let mut replies = 0;
    while rx.recv_async().await.is_ok() {
        replies += 1;
    }
    assert_eq!(replies, 0, "a contract group has no set");
    rf0.svc.close().await.unwrap();
}

/// §10 An idempotent retry (§2.14).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s10_an_idempotent_retry() {
    let bus = bus().await;
    let s = recorder(&bus.s, RF0).await;
    let rf0 = rf_fresh(&bus.owner, RF0, true).await;
    present(&bus.c, RF0).await;
    let body = json!({"frequency_hz": 869525000, "confirm_s": 60, "idempotency_key": 1001});

    // 1. The reply is dropped before it reaches C.
    let step1 = Instant::now();
    let mut request = vec![0x40 | iface().derived.set("radio").unwrap().1.tag()];
    request.extend_from_slice(&set_record("radio", &body));
    drop(
        bus.c
            .get(format!("zk2/{RF0}/{IFACE}/@op/set/radio"))
            .payload(request)
            .encoding(zenoh::bytes::Encoding::ZENOH_BYTES)
            .attachment(CallMetadata::new("ops", "r1").to_bytes())
            .target(QueryTarget::BestMatching)
            .timeout(CALL)
            .with(flume::bounded::<Reply>(0))
            .await
            .unwrap(),
    );
    s.until("the armed record", |s| {
        s.last().is_some_and(|r| r.pending.is_some())
    })
    .await;
    let armed = s.records().pop().unwrap();

    // 2. The same request 5 s later, after the apply: step 1's answer.
    s.until("the applied record", |s| {
        s.last().and_then(|r| r.pending).is_some_and(|p| p.applied)
    })
    .await;
    tokio::time::sleep(Duration::from_secs(5).saturating_sub(step1.elapsed())).await;
    let puts = s.records().len();
    let r = set(&bus.c, RF0, "radio", body.clone(), "r2").await;
    let (mark, bytes, first) = value(&r);
    assert_eq!(mark, armed.stamp, "the first put's run and seq");
    assert_eq!(bytes, armed.bytes, "byte for byte");
    assert!(
        first.pending.as_ref().is_some_and(|p| !p.applied),
        "pending and not applied, as first answered"
    );
    assert_eq!(rf0.device.with(|d| d.applies.len()), 1, "one apply");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(s.records().len(), puts, "nothing new");
    assert!(s.events().is_empty(), "no event");

    // 3. The same key, another request: invalid_request, under the
    // current mark.
    let applied = s.records().pop().unwrap();
    let other = json!({"frequency_hz": 869000000, "confirm_s": 60, "idempotency_key": 1001});
    let r = set(&bus.c, RF0, "radio", other, "r3").await;
    let e = refusal(&r);
    assert_eq!(e.code, "invalid_request");
    assert_eq!(e.mark, applied.stamp, "the current mark");

    // 4. Cancelled; 300 s after step 1 on the owner's clock, the key is
    // forgotten: a new change, with a new token.
    let token = armed.state.pending.as_ref().unwrap().token;
    let r = control(&bus.c, RF0, config_v2::CANCEL, token, None, "r4").await;
    assert_eq!(record(&r).pending, None);
    let shift = Duration::from_secs(300).saturating_sub(step1.elapsed()) + Duration::from_secs(1);
    rf0.cfg
        .simulate_offset(i64::try_from(shift.as_millis()).unwrap());
    let r = set(&bus.c, RF0, "radio", body, "r5").await;
    let fresh = record(&r).pending.expect("a new pending record");
    assert_ne!(fresh.token, token);
    rf0.svc.close().await.unwrap();
}

/// §11 Sensitive values (§2.15). "The owner's log" is its lines of the
/// suite's log ([`logs`]), each naming its instance number.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s11_sensitive_values() {
    let bus = bus().await;
    let s = recorder(&bus.s, RF0).await;
    let rf0 = rf_fresh(&bus.owner, RF0, true).await;
    present(&bus.c, RF0).await;
    let mut replies: Vec<Vec<u8>> = Vec::new();
    let mut keep = |o: &Outcome| match o {
        Outcome::Value(a) => replies.push(a.payload().to_bytes().into_owned()),
        Outcome::Refused(r) => replies.push(format!("{r:?}").into_bytes()),
        other => panic!("an answer, not {other:?}"),
    };

    // 1. Two pins applied; the records differ only in their revision and
    // last token; each edit redacted.
    let r = set(&bus.c, RF0, "sim", json!({"pin": 1234}), "r1").await;
    keep(&r);
    let t1 = record(&r).last.unwrap().token;
    let r = set(&bus.c, RF0, "sim", json!({"pin": 9999}), "r2").await;
    keep(&r);
    let t2 = record(&r).last.unwrap().token;
    s.until("two applied events", |s| s.events().len() == 2)
        .await;
    let puts = s.records();
    let (a, b) = (&puts[puts.len() - 2].state, &puts[puts.len() - 1].state);
    let blank = |x: &State| State {
        revision: 0,
        last: None,
        ..x.clone()
    };
    assert_eq!(blank(a), blank(b), "no bit depends on the PIN");
    for (_, _, e) in s.events() {
        assert_eq!(
            serde_json::to_value(&e.edits).unwrap(),
            json!([{"group": "sim", "parameter": "pin", "redacted": true}])
        );
    }

    // 2. The device refuses a third, in words holding the PIN: app,
    // device_refused, no message.
    rf0.device
        .with(|d| d.refuse_apply = Some("PIN 1234 rejected by the SIM".into()));
    let r = set(
        &bus.c,
        RF0,
        "sim",
        json!({"pin": 1234, "attempts": 5}),
        "r3",
    )
    .await;
    keep(&r);
    let e = refusal(&r);
    assert_eq!(e.code, "app");
    assert_eq!(e.detail, Some(json!({"code": "device_refused"})));
    assert_eq!(e.message, None, "no message: the change carries the PIN");

    // 3. The record shows the pin's source alone; the first persist is
    // not_found, the second forbidden.
    let r = get(&bus.c, RF0).await;
    assert_eq!(
        r.groups["sim"].params["pin"],
        ParamState::Sensitive {
            source: Some(Source::Runtime)
        }
    );
    let p1 = control(&bus.c, RF0, config_v2::PERSIST, t1, None, "r4").await;
    keep(&p1);
    assert_eq!(refusal(&p1).code, "not_found");
    let p2 = control(&bus.c, RF0, config_v2::PERSIST, t2, None, "r5").await;
    keep(&p2);
    assert_eq!(refusal(&p2).code, "forbidden");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let instance = rf0.cfg.instance();
    rf0.svc.close().await.unwrap();

    // 4. The log: no line holds 1234 or 9999, and a logged request writes
    // "<redacted>"; nor does any delivery or reply.
    let log = String::from_utf8(logs().0.lock().unwrap().clone()).unwrap();
    let mine: Vec<&str> = log
        .lines()
        .filter(|l| l.contains(&format!("instance={instance}")))
        .collect();
    assert!(mine.len() >= 4, "the owner logged its calls: {mine:?}");
    // A text holds a PIN when one of its runs of digits is the PIN: the
    // instance number, tokens and revisions are numbers of their own.
    let holds = |t: &str| {
        t.split(|c: char| !c.is_ascii_digit())
            .any(|run| run == "1234" || run == "9999")
    };
    for line in &mine {
        assert!(!holds(line), "{line}");
    }
    assert!(
        mine.iter()
            .any(|l| l.contains("config.v2: set") && l.contains("\"pin\":\"<redacted>\"")),
        "{mine:?}"
    );
    for r in s.all() {
        let text = String::from_utf8_lossy(&r.payload);
        assert!(!holds(&text), "{} {text}", r.key);
    }
    for r in &replies {
        let text = String::from_utf8_lossy(r);
        assert!(!holds(&text), "{text}");
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
    ab.implement(Implementation::new(author_contract.clone()))
        .unwrap();
    let doc_writer = ab
        .declare_state_writer(&author_iface, "state/desired", &Bindings::new())
        .await
        .unwrap();
    let author = ab.start().await.unwrap();

    // The owner, its role bound to the author, following it.
    let startup = rf(true);
    let device = Arc::new(SimDevice::new_v2(&startup));
    let mut b = ServiceBuilder::new(&bus.owner, config(RF0).bind("desired", &["lab/author"]));
    b.require("desired", author_iface.clone(), false);
    b.implement(Implementation::new((*contract()).clone()))
        .unwrap();
    let cfg = b
        .config_v2(startup, Arc::clone(&device) as Arc<dyn config_v2::Device>)
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
    // The record's desired fields as its JSON reading carries them.
    let fields = |s: &Recorder| -> Json {
        let p = s.records().pop().unwrap();
        let mut f = iface()
            .derived
            .record()
            .read(iface().derived.record().tag(), &p.bytes, Some(0))
            .unwrap();
        let m = f.as_object_mut().unwrap();
        m.retain(|k, _| k.starts_with("desired"));
        f
    };

    // 1. Converged: the record, then one applied event.
    doc_writer
        .put_value(&json!({"revision": 1, "values": {"duty": {"fraction_ppm": 20000}}}))
        .await
        .unwrap();
    s.until("desired revision 1", |s| fields(s)["desired_revision"] == 1)
        .await;
    assert_eq!(
        fields(&s),
        json!({"desired": "converged", "desired_revision": 1})
    );
    assert_eq!(
        param(&s.last().unwrap(), "duty", "fraction_ppm").2,
        int(20000)
    );
    let events = s.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].2.outcome, Ending::Applied);
    let applies = device.with(|d| d.applies.len());

    // 2. Refused whole: the reach group, by index; the legal duty change
    // not applied.
    doc_writer
        .put_value(&json!({"revision": 2, "values": {
            "duty": {"fraction_ppm": 30000}, "radio": {"frequency_hz": 869525000}}}))
        .await
        .unwrap();
    s.until("desired revision 2", |s| fields(s)["desired_revision"] == 2)
        .await;
    assert_eq!(
        fields(&s),
        json!({"desired": "refused", "desired_revision": 2, "desired_code": "reach",
            "desired_group": 1})
    );
    assert_eq!(
        param(&s.last().unwrap(), "duty", "fraction_ppm").2,
        int(20000)
    );
    assert_eq!(device.with(|d| d.applies.len()), applies, "no act");
    assert_eq!(s.events().len(), 1, "no event");

    // 3. The sensitive parameter: sim's pin, by index; no act.
    doc_writer
        .put_value(&json!({"revision": 3, "values": {"sim": {"pin": 1234}}}))
        .await
        .unwrap();
    s.until("desired revision 3", |s| fields(s)["desired_revision"] == 3)
        .await;
    assert_eq!(
        fields(&s),
        json!({"desired": "refused", "desired_revision": 3, "desired_code": "sensitive",
            "desired_group": 3, "desired_parameter": 0})
    );
    assert_eq!(device.with(|d| d.applies.len()), applies, "no act");
    assert_eq!(cfg.desired().await.map(|d| d.observed_revision), Some(3));
    svc.close().await.unwrap();
    author.close().await.unwrap();
}
