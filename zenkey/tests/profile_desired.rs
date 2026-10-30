//! `spec/profiles/desired/scenarios.md` §1–§8 and §10–§12 (`desired.v1`,
//! #840, DC; §12 #935), one test per section, named after it. §9 is a
//! tool's (#840, DD). `config_v1_as_a_target` runs `desired.v1` §2.16
//! beside them.
//!
//! Common setup, the text's: R1 is a router with timestamping on, a
//! router's default. Where a section cuts a link, the author's side is R1
//! and the target's side R2, a router linked to R1 through a [`Link`] the
//! test cuts and heals; elsewhere everything is a client of R1. The author
//! `lab/author` implements `tasking.v1` through
//! [`ServiceBuilder::desired_author`], `tasking.v1` in its tokenless set;
//! the target `lab/worker` implements `worker.v1`, its role `task` bound to
//! the author with `{system} = self.system` and `{service} = self.service`,
//! through [`ServiceBuilder::desired_target`], on the owner example's
//! simulated drive (`examples/owner/desired.rs`). S records every delivery
//! under `zk2/lab/**` and every token under it on R1; where a link is cut,
//! S2 records the same on R2. G GETs as core S4 says.
//!
//! Each section runs on routers of its own, so the sections run at once.
//! Where the author's presence must have reached the target's router before
//! the target acts, the test waits for it (`eventually`): two sessions'
//! messages are not ordered (core §8.1).

mod common;

#[allow(dead_code)]
#[path = "../examples/owner/desired.rs"]
mod drive;

#[allow(dead_code)]
#[path = "../examples/owner/config.rs"]
mod sim;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{Link, SETTLE, T, addr, client, config, eventually, imp, link_to, router, router_via};
use drive::{Drive, REPORT, ROLE, RUNNING, TASKS};
use serde_json::{Value, json};
use zenkey::desired::{
    self, AuthorStartup, FileRecord, Found, ReportAt, TargetStartup, Version, Written,
};
use zenkey::model::desired::{Outcome, Report, Source};
use zenkey::model::template::Bindings;
use zenkey::{Implementation, Service, ServiceBuilder, presence};
use zenoh::query::{ConsolidationMode, QueryTarget, Reply};
use zenoh::sample::{Sample, SampleKind};
use zenoh::time::Timestamp;

const AUTHOR: &str = "lab/author";
const WORKER: &str = "lab/worker";
/// The target's slice.
const D: &str = "zk2/lab/author/tasking.v1/state/tasks/lab/worker";
/// A second slice of the author's (§6).
const D2: &str = "zk2/lab/author/tasking.v1/state/tasks/lab/spare";
/// The target's report.
const RP: &str = "zk2/lab/worker/worker.v1/state/desired/task";
/// What the target runs.
const RUN: &str = "zk2/lab/worker/worker.v1/state/running";
/// The author's instance tokens.
const AUTHOR_TOKENS: &str = "zk2/lab/author/@zk/instance/*";

/// `v<n>`: a Task with revision n, mode run and rate_hz 10.
fn v(n: u64) -> Value {
    json!({"revision": n, "mode": "run", "rate_hz": 10})
}

fn bytes(doc: &Value) -> Vec<u8> {
    serde_json::to_vec(doc).unwrap()
}

/// A version as a report writes it.
fn version(ts: &Timestamp, revision: Option<u64>) -> Version {
    Version {
        stamp: desired::stamp(ts),
        revision,
    }
}

/// Converged on the version `ts`, `revision` (`scenarios.md`): `source`
/// desired, `in_force` and `observed` both it, `converged`.
fn converged_on(r: &Report, ts: &Timestamp, revision: u64) -> bool {
    let want = Some(version(ts, Some(revision)));
    r.key.as_deref() == Some(D)
        && r.source == Source::Desired
        && r.outcome == Some(Outcome::Converged)
        && r.in_force == want
        && r.observed == want
        && r.refusal.is_none()
}

fn put_stamp(w: Written) -> Timestamp {
    match w {
        Written::Put(ts) | Written::Deleted(ts) => ts,
        other => panic!("not written: {other:?}"),
    }
}

// --- the bus ---------------------------------------------------------------------

/// R1, and R2 behind a cuttable link where a section cuts one.
struct Bus {
    _r1: zenoh::Session,
    ep1: String,
    link: Option<Link>,
    _r2: Option<zenoh::Session>,
    /// Where the target's side connects: R2's endpoint, or R1's.
    ep2: String,
}

async fn one_router() -> Bus {
    let (r1, ep1) = router(None).await;
    Bus {
        _r1: r1,
        ep2: ep1.clone(),
        ep1,
        link: None,
        _r2: None,
    }
}

async fn two_routers() -> Bus {
    let (r1, ep1) = router(None).await;
    let link = link_to(&ep1).await;
    let (r2, ep2) = router_via(&link).await;
    Bus {
        _r1: r1,
        ep1,
        link: Some(link),
        _r2: Some(r2),
        ep2,
    }
}

impl Bus {
    fn link(&self) -> &Link {
        self.link.as_ref().expect("a cuttable link")
    }
}

/// Waits until a fresh client of `ep` reads the author present or absent
/// by a complete read (core §8.1): its presence has reached that router.
async fn author_seen(ep: &str, present: bool) {
    let g = client(ep).await;
    eventually(
        if present {
            "the author present"
        } else {
            "the author absent"
        },
        || async {
            match presence::liveliness_read(&g, AUTHOR_TOKENS, T).await {
                Ok(r) if present => !r.keys.is_empty(),
                Ok(r) => r.complete && r.keys.is_empty(),
                Err(_) => false,
            }
        },
    )
    .await;
    g.close().await.unwrap();
}

/// G's GET of `key`, as core S4 says.
async fn get(s: &zenoh::Session, key: &str) -> Vec<Sample> {
    let rx = s
        .get(key)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::Latest)
        .timeout(T)
        .with(flume::unbounded::<Reply>())
        .await
        .unwrap();
    let mut out = Vec::new();
    while let Ok(r) = rx.recv_async().await {
        if let Ok(s) = r.into_result() {
            out.push(s);
        }
    }
    out
}

// --- S: the recording subscriber --------------------------------------------------

/// One delivery, as S records it.
#[derive(Debug, Clone)]
struct Rec {
    key: String,
    delete: bool,
    payload: Vec<u8>,
    stamp: Option<Timestamp>,
    token: bool,
}

impl Rec {
    fn report(&self) -> Report {
        zenkey::model::desired::decode(&self.payload).expect("a desired.v1.Report")
    }

    fn value(&self) -> Value {
        serde_json::from_slice(&self.payload).expect("JSON")
    }
}

type Log = Arc<Mutex<Vec<Rec>>>;

/// S: deliveries under `zk2/lab/**` and tokens under `zk2/lab/*/@zk/**`,
/// in one log in arrival order.
struct Recorder {
    _data: zenoh::pubsub::Subscriber<()>,
    _tokens: zenoh::pubsub::Subscriber<()>,
    log: Log,
    _session: zenoh::Session,
}

async fn record(ep: &str) -> Recorder {
    let s = client(ep).await;
    let log: Log = Arc::default();
    let push = |log: &Log, x: &Sample, token: bool| {
        log.lock().unwrap().push(Rec {
            key: x.key_expr().as_str().to_owned(),
            delete: x.kind() == SampleKind::Delete,
            payload: x.payload().to_bytes().into_owned(),
            stamp: x.timestamp().copied(),
            token,
        });
    };
    let l = Arc::clone(&log);
    let data = s
        .declare_subscriber("zk2/lab/**")
        .callback(move |x| push(&l, &x, false))
        .await
        .unwrap();
    // `**` never crosses a verbatim chunk (core §1.3): the tokens are
    // under `@zk`.
    let l = Arc::clone(&log);
    let tokens = s
        .liveliness()
        .declare_subscriber("zk2/lab/*/@zk/**")
        .callback(move |x| push(&l, &x, true))
        .await
        .unwrap();
    Recorder {
        _data: data,
        _tokens: tokens,
        log,
        _session: s,
    }
}

impl Recorder {
    fn all(&self) -> Vec<Rec> {
        self.log.lock().unwrap().clone()
    }

    fn mark(&self) -> usize {
        self.log.lock().unwrap().len()
    }

    /// Deliveries on `key` since `mark`, data only.
    fn on(&self, key: &str, since: usize) -> Vec<Rec> {
        self.all()
            .into_iter()
            .skip(since)
            .filter(|r| !r.token && r.key == key)
            .collect()
    }

    /// The report: the latest delivery on Rp.
    fn report(&self) -> Option<Report> {
        self.on(RP, 0).last().map(Rec::report)
    }

    /// The latest delivery on Run.
    fn running(&self) -> Option<Value> {
        self.on(RUN, 0).last().map(Rec::value)
    }

    /// Waits until the report holds `f`.
    async fn until(&self, what: &str, f: impl Fn(&Report) -> bool) -> Report {
        let deadline = tokio::time::Instant::now() + SETTLE;
        loop {
            if let Some(r) = self.report()
                && f(&r)
            {
                return r;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "never: {what}; the report is {:?}",
                self.report()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Waits until a delivery on `key` since `mark` holds `f`. The report
    /// and Run are two writers, so a test that saw the report converge waits
    /// for Run's put rather than read it at once.
    async fn until_on(&self, key: &str, since: usize, what: &str, f: impl Fn(&Rec) -> bool) {
        let deadline = tokio::time::Instant::now() + SETTLE;
        while !self.on(key, since).iter().any(&f) {
            assert!(tokio::time::Instant::now() < deadline, "never: {what}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

// --- the author and the target ---------------------------------------------------

struct AuthorUp {
    session: zenoh::Session,
    svc: Service,
    author: desired::Author,
}

/// `lab/author` on a client of `ep`: `tasking.v1` in its tokenless set, its
/// window `window_s` (60 s unless given).
async fn author_up(ep: &str, startup: AuthorStartup, window_s: Option<u64>) -> AuthorUp {
    let session = client(ep).await;
    let tasking = imp("tasking.v1");
    let iface = tasking.iface().clone();
    let mut cfg = config(AUTHOR).tokenless(iface.clone());
    cfg.tombstone_window_s = window_s;
    let mut b = ServiceBuilder::new(&session, cfg);
    b.implement(tasking).unwrap();
    let author = b.desired_author(&iface, TASKS, startup).await.unwrap();
    let svc = b.start().await.unwrap();
    AuthorUp {
        session,
        svc,
        author,
    }
}

impl AuthorUp {
    /// The process ends without closing: the service dropped, its session
    /// with it.
    async fn kill(self) {
        drop(self.svc);
        self.session.close().await.unwrap();
    }
}

struct WorkerUp {
    session: zenoh::Session,
    _watch: Option<zenoh::Session>,
    svc: Service,
    target: desired::Target,
    drive: Arc<Drive>,
}

/// How the target follows its author's presence.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Follow {
    /// A watch session of its own (core §8.1, 0.37).
    Watch,
    /// A presence read every 500 ms.
    Poll,
}

/// `lab/worker` on a client of `ep`, its role bound (or, `bound` false,
/// the kill switch), its drive held from its first act when `hold`.
async fn worker_up(
    ep: &str,
    bound: bool,
    follow: Follow,
    startup: TargetStartup,
    hold: bool,
) -> WorkerUp {
    let session = client(ep).await;
    let worker = imp("worker.v1");
    let wi = worker.iface().clone();
    let mut cfg = config(WORKER);
    if bound {
        cfg = cfg.bind(ROLE, &[AUTHOR]);
        cfg.bindings.get_mut(ROLE).unwrap().params = [
            ("system".to_owned(), "self.system".to_owned()),
            ("service".to_owned(), "self.service".to_owned()),
        ]
        .into();
    }
    let watch = match follow {
        Follow::Watch => Some(client(ep).await),
        Follow::Poll => None,
    };
    let mut startup = startup;
    startup.watch = watch.clone();
    let mut b = ServiceBuilder::new(&session, cfg);
    b.implement(worker).unwrap();
    let run = b
        .declare_state_writer(&wi, RUNNING, &Bindings::new())
        .await
        .unwrap();
    run.put_value(&drive::baseline()).await.unwrap();
    let d = Arc::new(Drive::new(run));
    if hold {
        d.hold();
    }
    let target = b
        .desired_target(
            ROLE,
            &imp("tasking.v1"),
            TASKS,
            ReportAt::new(wi, REPORT),
            Arc::clone(&d) as Arc<dyn desired::Handler>,
            startup,
        )
        .await
        .unwrap();
    let svc = b.start().await.unwrap();
    WorkerUp {
        session,
        _watch: watch,
        svc,
        target,
        drive: d,
    }
}

impl WorkerUp {
    /// Waits until its first seed has run: its subscription has reached
    /// the author's router, ahead of the seed's GET on the same link.
    async fn seeded(&self) {
        eventually("the target's first seed", || async {
            self.target.last_seed().is_some()
        })
        .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    /// The process ends without closing.
    async fn kill(self) {
        drop(self.svc);
        self.session.close().await.unwrap();
    }
}

/// The author and the target, the target converged on v1, `s1` its stamp.
async fn converged_on_v1(
    s: &Recorder,
    author_ep: &str,
    target_ep: &str,
    author: AuthorStartup,
    window_s: Option<u64>,
    follow: Follow,
    target: TargetStartup,
) -> (AuthorUp, WorkerUp, Timestamp) {
    let a = author_up(author_ep, author, window_s).await;
    let w = worker_up(target_ep, true, follow, target, false).await;
    w.seeded().await;
    let s1 = put_stamp(a.author.put(WORKER, bytes(&v(1))).await.unwrap());
    s.until("converged on v1", |r| converged_on(r, &s1, 1))
        .await;
    (a, w, s1)
}

// --- the sections -----------------------------------------------------------------

/// §1 Converge after an outage (§2.5, §2.6, §2.10, §2.15).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_converge_after_an_outage() {
    let bus = two_routers().await;
    let s = record(&bus.ep1).await;
    let s2 = record(&bus.ep2).await;
    let g = client(&bus.ep1).await;
    let tokens = g
        .liveliness()
        .declare_subscriber("zk2/lab/worker/@zk/instance/*")
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();

    // 1. The author with no document; the target; G's GET of Rp the moment
    //    S sees the target's instance token.
    let a = author_up(&bus.ep1, AuthorStartup::default(), None).await;
    assert!(a.svc.tokens_held().is_empty(), "tasking.v1 is tokenless");
    author_seen(&bus.ep2, true).await;
    let w = worker_up(
        &bus.ep2,
        true,
        Follow::Poll,
        TargetStartup::default(),
        false,
    )
    .await;
    let token = tokio::time::timeout(SETTLE, tokens.recv_async())
        .await
        .expect("the target's instance token")
        .unwrap();
    assert_eq!(token.kind(), SampleKind::Put);
    let at_token = get(&g, RP).await;
    assert_eq!(at_token.len(), 1, "the report was put before the tokens");
    let r = zenkey::model::desired::decode(&at_token[0].payload().to_bytes()).unwrap();
    assert_eq!(r, Report::baseline(Some(D.to_owned())));
    let run = get(&g, RUN).await;
    assert_eq!(
        serde_json::from_slice::<Value>(&run[0].payload().to_bytes()).unwrap(),
        drive::baseline()
    );
    w.seeded().await;

    // 2. v1: put on D stamped by the author (S1), then a report converged.
    let s1 = put_stamp(a.author.put(WORKER, bytes(&v(1))).await.unwrap());
    s.until("converged on v1", |r| converged_on(r, &s1, 1))
        .await;
    let puts = s.on(D, 0);
    assert_eq!(puts.len(), 1);
    assert_eq!(
        puts[0].stamp.unwrap().get_id().to_string(),
        a.session.zid().to_string(),
        "stamped by the author (core S1)"
    );
    assert_eq!(s.running(), Some(v(1)));

    // 3. The cut: v2 put; nothing reaches the target's side.
    bus.link().cut();
    author_seen(&bus.ep2, false).await;
    let mark2 = s2.mark();
    let s2_stamp = put_stamp(a.author.put(WORKER, bytes(&v(2))).await.unwrap());
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        s2.on(RP, mark2).is_empty() && s2.on(RUN, mark2).is_empty(),
        "nothing on Rp or Run during the cut"
    );
    assert!(converged_on(&w.target.report().await, &s1, 1));
    let acts = w.target.acts();

    // 4. The heal: the target sees the author's instance token, seeds, and
    //    acts on v2 once, within 5 s.
    let healed = Instant::now();
    bus.link().heal();
    s.until("converged on v2", |r| converged_on(r, &s2_stamp, 2))
        .await;
    let took = healed.elapsed();
    assert!(
        took < Duration::from_secs(5),
        "converged {took:?} after the heal"
    );
    assert_eq!(s.running(), Some(v(2)));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(w.target.acts(), acts + 1, "acted on v2 once");
    // §2.15: no interface token of the author's ever appeared.
    assert!(
        !s.all()
            .iter()
            .any(|r| r.token && r.key.starts_with("zk2/lab/author/@zk/alive/")),
        "the author held no interface token"
    );
    w.svc.close().await.unwrap();
    a.svc.close().await.unwrap();
}

/// §2 Never an older version (§2.6, §2.7).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_never_an_older_version() {
    let bus = one_router().await;
    let s = record(&bus.ep1).await;
    let (a, w, _) = converged_on_v1(
        &s,
        &bus.ep1,
        &bus.ep2,
        AuthorStartup::default(),
        None,
        Follow::Poll,
        TargetStartup::default(),
    )
    .await;
    let s2 = put_stamp(a.author.put(WORKER, bytes(&v(2))).await.unwrap());
    s.until("converged on v2", |r| converged_on(r, &s2, 2))
        .await;
    let acts = w.target.acts();
    let x = client(&bus.ep1).await;
    let mark = s.mark();

    // 1. v1's content, a second earlier under s2's id: older.
    let earlier = Timestamp::new(
        zenoh::time::NTP64(s2.get_time().as_u64() - (1 << 32)),
        *s2.get_id(),
    );
    x.put(D, bytes(&v(1))).timestamp(earlier).await.unwrap();
    // 2. v3's content under s2 exactly: the same version.
    x.put(D, bytes(&v(3))).timestamp(s2).await.unwrap();
    eventually("both of X's puts delivered", || async {
        s.on(D, mark).len() >= 2
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(s.on(RP, mark).is_empty(), "nothing put on Rp");
    assert!(s.on(RUN, mark).is_empty(), "nothing put on Run");
    let held = get(&x, RP).await;
    assert!(converged_on(
        &zenkey::model::desired::decode(&held[0].payload().to_bytes()).unwrap(),
        &s2,
        2
    ));

    // 3. A re-read every 2 s: the author answers v2 under s2.
    let seeds = w.target.seeds();
    w.target.set_reread(Some(Duration::from_secs(2)));
    eventually("a re-read", || async { w.target.seeds() > seeds }).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(w.target.last_seed().unwrap().answered, 1);
    assert!(s.on(RP, mark).is_empty(), "the same version: nothing put");
    assert_eq!(w.target.acts(), acts, "acted on nothing");
    assert!(converged_on(&w.target.report().await, &s2, 2));
    w.target.set_reread(None);
    w.svc.close().await.unwrap();
    a.svc.close().await.unwrap();
}

/// §3 A delete reverts, within the window (§2.8).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_a_delete_reverts_within_the_window() {
    let bus = two_routers().await;
    let s = record(&bus.ep1).await;
    let (a, w, _) = converged_on_v1(
        &s,
        &bus.ep1,
        &bus.ep2,
        AuthorStartup::default(),
        Some(10),
        Follow::Watch,
        TargetStartup::default(),
    )
    .await;
    let reverted = |r: &Report, ts: &Timestamp| {
        *r == Report {
            key: Some(D.to_owned()),
            source: Source::Baseline,
            in_force: None,
            observed: Some(version(ts, None)),
            outcome: Some(Outcome::Reverted),
            refusal: None,
            last_known: false,
        }
    };

    // 1. The delete: reverted to the baseline.
    let del = put_stamp(a.author.delete(WORKER).await.unwrap());
    s.until("reverted", |r| reverted(r, &del)).await;
    assert_eq!(s.running(), Some(drive::baseline()));

    // 2. v2.
    let s2 = put_stamp(a.author.put(WORKER, bytes(&v(2))).await.unwrap());
    s.until("converged on v2", |r| converged_on(r, &s2, 2))
        .await;

    // 3. A delete during a 3 s cut: answered by the seed after the heal.
    bus.link().cut();
    author_seen(&bus.ep2, false).await;
    let del3 = put_stamp(a.author.delete(WORKER).await.unwrap());
    tokio::time::sleep(Duration::from_secs(3)).await;
    bus.link().heal();
    s.until("reverted after the heal", |r| reverted(r, &del3))
        .await;
    assert_eq!(s.running(), Some(drive::baseline()));

    // 4. v3, then a delete during a 15 s cut, past the 10 s window.
    let s3 = put_stamp(a.author.put(WORKER, bytes(&v(3))).await.unwrap());
    s.until("converged on v3", |r| converged_on(r, &s3, 3))
        .await;
    bus.link().cut();
    author_seen(&bus.ep2, false).await;
    a.author.delete(WORKER).await.unwrap();
    tokio::time::sleep(Duration::from_secs(15)).await;
    let seeds = w.target.seeds();
    bus.link().heal();
    author_seen(&bus.ep2, true).await;
    eventually("a seed after the heal", || async {
        w.target.seeds() > seeds
    })
    .await;
    // Its retries while the author is present and silent run out in 1 s.
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        w.target.last_seed().unwrap().answered,
        0,
        "drew nothing for D"
    );
    assert!(
        converged_on(&w.target.report().await, &s3, 3),
        "silence is no verdict: v3 stays in force"
    );
    assert!(converged_on(&s.report().unwrap(), &s3, 3));
    assert_eq!(s.running(), Some(v(3)));
    w.svc.close().await.unwrap();
    a.svc.close().await.unwrap();
}

/// §4 A refused document keeps the previous in force (§2.7, §2.10).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_a_refused_document_keeps_the_previous_in_force() {
    let bus = one_router().await;
    let s = record(&bus.ep1).await;
    let (mut a, w, s1) = converged_on_v1(
        &s,
        &bus.ep1,
        &bus.ep2,
        AuthorStartup::default(),
        None,
        Follow::Poll,
        TargetStartup::default(),
    )
    .await;
    let refused = |r: &Report, observed: &Timestamp, rev: u64, code: &str, pointer: &str| {
        r.key.as_deref() == Some(D)
            && r.source == Source::Desired
            && r.in_force == Some(version(&s1, Some(1)))
            && r.observed == Some(version(observed, Some(rev)))
            && r.outcome == Some(Outcome::Refused)
            && r.refusal
                .as_ref()
                .is_some_and(|f| f.code == code && f.pointer.as_deref() == Some(pointer))
    };

    // 1. v2 naming the endpoint: refused reach.
    let mut v2 = v(2);
    v2["endpoint"] = json!("tcp/10.0.0.9:7447");
    let s2 = put_stamp(a.author.put(WORKER, bytes(&v2)).await.unwrap());
    s.until("v2 refused", |r| refused(r, &s2, 2, "reach", "/endpoint"))
        .await;
    assert_eq!(s.running(), Some(v(1)));

    // 2. v3 at 500 Hz: refused invalid.
    let mut v3 = v(3);
    v3["rate_hz"] = json!(500);
    let s3 = put_stamp(a.author.put(WORKER, bytes(&v3)).await.unwrap());
    s.until("v3 refused", |r| refused(r, &s3, 3, "invalid", "/rate_hz"))
        .await;
    assert_eq!(s.running(), Some(v(1)));

    // 3. A re-mint in the author's own session: v3 put again under its zid
    //    and a later time, adopted, refused still; the seed is the same
    //    version.
    let acts = w.target.acts();
    let seeds = w.target.seeds();
    let mark = s.mark();
    a.svc.new_epoch().await.unwrap();
    eventually("the re-put", || async { !s.on(D, mark).is_empty() }).await;
    let reput_stamp = s.on(D, mark)[0].stamp.unwrap();
    assert_eq!(reput_stamp.get_id(), s3.get_id(), "its zid kept");
    assert!(reput_stamp.get_time() > s3.get_time());
    s.until("the re-put adopted", |r| {
        refused(r, &reput_stamp, 3, "invalid", "/rate_hz")
    })
    .await;
    eventually("the target's seed at the new instance", || async {
        w.target.seeds() > seeds
    })
    .await;
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert_eq!(s.on(RP, mark).len(), 1, "one report put: the adoption");
    assert_eq!(w.target.acts(), acts, "nothing applied");
    assert_eq!(s.running(), Some(v(1)));

    // 4. v4: converged.
    let s4 = put_stamp(a.author.put(WORKER, bytes(&v(4))).await.unwrap());
    s.until("converged on v4", |r| converged_on(r, &s4, 4))
        .await;

    // 5. The drive fails its next act: v5 at 20 Hz, halt.
    w.drive.fail_next();
    let mut v5 = v(5);
    v5["rate_hz"] = json!(20);
    v5["mode"] = json!("halt");
    let s5 = put_stamp(a.author.put(WORKER, bytes(&v5)).await.unwrap());
    let r = s
        .until("v5 failed", |r| r.outcome == Some(Outcome::Failed))
        .await;
    assert_eq!(r.source, Source::Desired);
    assert_eq!(r.observed, Some(version(&s5, Some(5))));
    assert_eq!(r.in_force, None);
    assert_eq!(r.refusal.unwrap().code, "apply");
    let running = s.running().unwrap();
    assert_eq!(
        (running["rate_hz"].clone(), running["mode"].clone()),
        (json!(20), json!("run"))
    );
    w.svc.close().await.unwrap();
    a.svc.close().await.unwrap();
}

/// §5 A restart acts again (§2.7, §2.5).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_a_restart_acts_again() {
    let bus = one_router().await;
    let s = record(&bus.ep1).await;
    let g = client(&bus.ep1).await;
    let (a, w, s1) = converged_on_v1(
        &s,
        &bus.ep1,
        &bus.ep2,
        AuthorStartup::default(),
        None,
        Follow::Watch,
        TargetStartup::default(),
    )
    .await;
    let tokens = g
        .liveliness()
        .declare_subscriber("zk2/lab/worker/@zk/instance/*")
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();

    // 1. The target's process ends; a new instance, its next act held.
    w.kill().await;
    let w = worker_up(
        &bus.ep2,
        true,
        Follow::Watch,
        TargetStartup::default(),
        true,
    )
    .await;
    let zid = w.session.zid().to_string();
    loop {
        let t = tokio::time::timeout(SETTLE, tokens.recv_async())
            .await
            .expect("the new instance's token")
            .unwrap();
        if t.kind() == SampleKind::Put
            && t.key_expr()
                .as_str()
                .contains(&w.svc.instance().to_string())
        {
            break;
        }
    }
    let at_token = get(&g, RP).await;
    assert_eq!(at_token.len(), 1);
    assert_eq!(
        zenkey::model::desired::decode(&at_token[0].payload().to_bytes()).unwrap(),
        Report::baseline(Some(D.to_owned())),
        "the new instance holds nothing of its previous one's acts"
    );
    assert_eq!(
        at_token[0].timestamp().unwrap().get_id().to_string(),
        zid,
        "under the new instance's stamp"
    );
    let mark = s.mark();
    w.drive.release();
    s.until("converged on v1 again", |r| converged_on(r, &s1, 1))
        .await;
    s.until_on(RUN, mark, "v1 put on Run by the new instance", |r| {
        r.value() == v(1) && r.stamp.unwrap().get_id().to_string() == zid
    })
    .await;
    assert_eq!(w.target.acts(), 1, "it acted on v1 again");
    w.svc.close().await.unwrap();
    a.svc.close().await.unwrap();
}

/// §6 The author's restart moves no target to act again (§2.7, §2.11).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s6_the_authors_restart_moves_no_target_to_act_again() {
    let bus = one_router().await;
    let s = record(&bus.ep1).await;
    let g = client(&bus.ep1).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("author.json");
    let with_record = || AuthorStartup::default().with_record(Arc::new(FileRecord::new(&path)));
    let (a, w, _) = converged_on_v1(
        &s,
        &bus.ep1,
        &bus.ep2,
        with_record(),
        None,
        Follow::Poll,
        TargetStartup::default(),
    )
    .await;
    a.author.put("lab/spare", bytes(&v(9))).await.unwrap();
    a.author.delete("lab/spare").await.unwrap();
    let record: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(record["deletes"]["lab/spare"].is_object(), "{record}");
    let acts = w.target.acts();

    // 1. The author's process ends, and restarts from its record: a fresh
    //    zid.
    a.kill().await;
    let mark = s.mark();
    let a = author_up(&bus.ep1, with_record(), None).await;
    let zid = a.session.zid().to_string();
    let found = a.author.adoption();
    assert_eq!(found.found, Found::Record);
    assert_eq!(found.reput, ["lab/worker"]);
    assert_eq!(found.redeleted, ["lab/spare"]);
    eventually("the author's new instance token", || async {
        s.all()
            .iter()
            .skip(mark)
            .any(|r| r.token && !r.delete && r.key.starts_with("zk2/lab/author/@zk/instance/"))
    })
    .await;
    let d = get(&g, D).await;
    let d2 = get(&g, D2).await;
    assert_eq!(d.len(), 1);
    assert_eq!(
        serde_json::from_slice::<Value>(&d[0].payload().to_bytes()).unwrap(),
        v(1)
    );
    assert_eq!(d[0].timestamp().unwrap().get_id().to_string(), zid);
    assert_eq!(d2.len(), 1);
    assert_eq!(d2[0].kind(), SampleKind::Delete, "a reply_del for D2");
    assert_eq!(d2[0].timestamp().unwrap().get_id().to_string(), zid);
    let desc = presence::descriptor(&g, &addr(AUTHOR), a.svc.instance(), T)
        .await
        .unwrap()
        .into_descriptor()
        .unwrap();
    assert_eq!(desc.meta["zid"].as_str(), Some(zid.as_str()));
    // "Before the token" is what the GETs above, made as soon as the new
    // instance token was seen, show: the owner held the re-put and the
    // re-delete, under the new zid. S's receipt order is not the owner's:
    // a put and a token reach it on two subscriptions, at two priorities,
    // with no order kept between them (as `profile_alarms.rs`'s
    // `at_new_token` says).
    // The re-put and the re-delete are found by the new zid, never by
    // arrival: the old instance's own delete of D2, made before its kill,
    // can still reach S after `mark` under load, for the same reason.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let log: Vec<Rec> = s.all().into_iter().skip(mark).collect();
    let new_id = |r: &Rec| r.stamp.is_some_and(|t| t.get_id().to_string() == zid);
    let put_at = log
        .iter()
        .position(|r| !r.token && r.key == D && new_id(r))
        .expect("the re-put, under the new zid");
    assert!(
        log.iter()
            .any(|r| !r.token && r.key == D2 && r.delete && new_id(r)),
        "the re-delete, under the new zid"
    );
    let reput = log[put_at].stamp.unwrap();
    assert!(s.on(RUN, mark).is_empty(), "Run receives nothing");
    let adopted = s.on(RP, mark);
    assert!(converged_on(&adopted.last().unwrap().report(), &reput, 1));
    assert_eq!(w.target.acts(), acts, "acted on nothing");

    // 2. v2, later than the re-put: acted on.
    let s2 = put_stamp(a.author.put(WORKER, bytes(&v(2))).await.unwrap());
    assert!(s2.get_time() > reput.get_time());
    s.until("converged on v2", |r| converged_on(r, &s2, 2))
        .await;
    assert_eq!(w.target.acts(), acts + 1);

    // 3. The author's process ends, its record removed: a new epoch, whose
    //    source yields v2's content again.
    a.kill().await;
    std::fs::remove_file(&path).unwrap();
    let mark = s.mark();
    let a = author_up(&bus.ep1, with_record(), None).await;
    assert_eq!(a.author.adoption().found, Found::Nothing);
    let s2b = put_stamp(a.author.put(WORKER, bytes(&v(2))).await.unwrap());
    assert_eq!(s2b.get_id().to_string(), a.session.zid().to_string());
    s.until("the new epoch's v2 adopted", |r| converged_on(r, &s2b, 2))
        .await;
    assert!(s.on(RUN, mark).is_empty(), "Run receives nothing");
    assert_eq!(w.target.acts(), acts + 1, "adopted, not acted on");
    assert!(
        get(&g, D2).await.is_empty(),
        "D2's delete went with the record"
    );

    // 4. v3: acted on.
    let s3 = put_stamp(a.author.put(WORKER, bytes(&v(3))).await.unwrap());
    s.until("converged on v3", |r| converged_on(r, &s3, 3))
        .await;
    assert_eq!(w.target.acts(), acts + 2);
    w.svc.close().await.unwrap();
    a.svc.close().await.unwrap();
}

/// An archive, `lab/archive`, recording the author's state, on `ep`.
async fn archive_on(ep: &str) -> (zenoh::Session, zenkey::archive::Archive) {
    let session = client(ep).await;
    let a = zenkey::archive::Archive::start(
        &session,
        zenkey::archive::ArchiveConfig {
            service: config("lab/archive"),
            records: vec![zenkey::archive::Recorded {
                owner: addr(AUTHOR),
                selector: "zk2/lab/author/tasking.v1/state/**".to_owned(),
                implementation: imp("tasking.v1"),
            }],
            peers: vec![],
            unconfirmed_horizon: None,
            watch: None,
        },
    )
    .await
    .unwrap();
    (session, a)
}

/// The target reading the archive `lab/archive`'s last-known.
fn with_archive() -> TargetStartup {
    TargetStartup::default().with_archive(addr("lab/archive"), T)
}

/// §7 Last-known from an archive, the author absent (§2.9).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_last_known_from_an_archive_the_author_absent() {
    let bus = one_router().await;
    let s = record(&bus.ep1).await;
    let (_as, archive) = archive_on(&bus.ep1).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("author.json");
    let with_record = || AuthorStartup::default().with_record(Arc::new(FileRecord::new(&path)));
    let (a, w, s1) = converged_on_v1(
        &s,
        &bus.ep1,
        &bus.ep2,
        with_record(),
        None,
        Follow::Poll,
        with_archive(),
    )
    .await;
    eventually("the archive recorded v1", || async {
        archive.confirmed(D) == Some(true)
    })
    .await;

    // 1. The author closes cleanly; the target restarts.
    a.svc.close().await.unwrap();
    a.session.close().await.unwrap();
    author_seen(&bus.ep1, false).await;
    w.kill().await;
    let mark = s.mark();
    let w = worker_up(&bus.ep2, true, Follow::Poll, with_archive(), false).await;
    let r = s
        .until("converged on v1, last-known", |r| {
            r.last_known && {
                let mut r = r.clone();
                r.last_known = false;
                converged_on(&r, &s1, 1)
            }
        })
        .await;
    assert!(r.last_known);
    let seed = w.target.last_seed().unwrap();
    assert_eq!(seed.answered, 0, "the GET drew no reply");
    assert_eq!(seed.author, desired::AuthorSeen::Absent);
    assert_eq!(seed.archive, Some(true));
    s.until_on(RUN, mark, "v1 put on Run", |r| r.value() == v(1))
        .await;
    let acts = w.target.acts();
    assert_eq!(acts, 1);

    // 2. The author restarts from its record: v1 again under its new zid,
    //    adopted, no longer last-known.
    let mark = s.mark();
    let a = author_up(&bus.ep1, with_record(), None).await;
    let reput = loop {
        if let Some(r) = s.on(D, mark).first() {
            break r.stamp.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_ne!(reput.get_id(), s1.get_id(), "another id than s1's");
    s.until("the re-put adopted", |r| {
        converged_on(r, &reput, 1) && !r.last_known
    })
    .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(s.on(RUN, mark).is_empty(), "Run receives nothing");
    assert_eq!(w.target.acts(), acts, "acted on nothing");
    assert_eq!(
        s.on(RP, mark).len(),
        1,
        "the other path's delivery is the same version"
    );
    w.svc.close().await.unwrap();
    a.svc.close().await.unwrap();
}

/// §8 The kill switch (§2.12).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s8_the_kill_switch() {
    let bus = one_router().await;
    let s = record(&bus.ep1).await;
    let g = client(&bus.ep1).await;
    let a = author_up(&bus.ep1, AuthorStartup::default(), None).await;
    a.author.put(WORKER, bytes(&v(1))).await.unwrap();
    // A probe on D: it counts the GETs the author's queryable would get.
    let probe = client(&bus.ep1).await;
    let gets = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let n = Arc::clone(&gets);
    let _q = probe
        .declare_queryable(D)
        .callback(move |_| {
            n.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        })
        .await
        .unwrap();
    let tokens = g
        .liveliness()
        .declare_subscriber("zk2/lab/worker/@zk/instance/*")
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 1. The target, unbound; G's GETs the moment its token appears.
    let w = worker_up(
        &bus.ep2,
        false,
        Follow::Poll,
        TargetStartup::default(),
        false,
    )
    .await;
    tokio::time::timeout(SETTLE, tokens.recv_async())
        .await
        .expect("the target's token")
        .unwrap();
    let desc = presence::descriptor(&g, &addr(WORKER), w.svc.instance(), T)
        .await
        .unwrap()
        .into_descriptor()
        .unwrap();
    let req = desc.requires.iter().find(|r| r.role == ROLE).unwrap();
    assert!(req.bindings.is_empty() && req.params.is_empty());
    let rp = get(&g, RP).await;
    assert_eq!(
        serde_json::from_slice::<Value>(&rp[0].payload().to_bytes()).unwrap(),
        json!({"source": "baseline"})
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        serde_json::from_slice::<Value>(&get(&g, RUN).await[0].payload().to_bytes()).unwrap(),
        drive::baseline()
    );
    assert_eq!(
        gets.load(std::sync::atomic::Ordering::Relaxed),
        0,
        "no GET of D"
    );
    assert_eq!((w.target.seeds(), w.target.acts()), (0, 0));
    assert_eq!(w.target.key(), None);
    // S's receipts are not ordered against G's token (two sessions, core
    // §8.1): count what S received in all, each one put before the tokens.
    assert_eq!(s.on(RUN, 0).len(), 1, "Run: the baseline only");
    assert_eq!(s.on(RP, 0).len(), 1, "Rp: the baseline report only");
    w.svc.close().await.unwrap();
    a.svc.close().await.unwrap();
}

/// §10 Last-known confirmed by its author; a present author's silence
/// (§2.5, §2.9, 0.2; steps 3 and 4 text 0.4, F-197).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s10_last_known_confirmed_by_its_author_and_a_present_authors_silence() {
    let bus = two_routers().await;
    let s = record(&bus.ep1).await;
    let s2 = record(&bus.ep2).await;
    let (_as, archive) = archive_on(&bus.ep2).await;
    author_seen(&bus.ep1, false).await;
    let a = author_up(&bus.ep1, AuthorStartup::default(), None).await;
    author_seen(&bus.ep2, true).await;
    let w = worker_up(&bus.ep2, true, Follow::Watch, with_archive(), false).await;
    w.seeded().await;
    let s1 = put_stamp(a.author.put(WORKER, bytes(&v(1))).await.unwrap());
    s.until("converged on v1", |r| converged_on(r, &s1, 1))
        .await;
    eventually("the archive recorded v1", || async {
        archive.confirmed(D) == Some(true)
    })
    .await;

    // 1. The cut; the target's process ends, and restarts.
    bus.link().cut();
    author_seen(&bus.ep2, false).await;
    w.kill().await;
    let mark = s2.mark();
    let w = worker_up(&bus.ep2, true, Follow::Watch, with_archive(), false).await;
    s2.until("converged on v1, last-known", |r| {
        r.last_known && {
            let mut r = r.clone();
            r.last_known = false;
            converged_on(&r, &s1, 1)
        }
    })
    .await;
    assert_eq!(
        w.target.last_seed().unwrap().author,
        desired::AuthorSeen::Absent
    );
    assert!(s2.on(RUN, mark).iter().any(|r| r.value() == v(1)));
    let acts = w.target.acts();

    // 2. The heal: the author's instance token appears again; its answer is
    //    s1, confirmed.
    let mark = s2.mark();
    bus.link().heal();
    s2.until("confirmed", |r| converged_on(r, &s1, 1) && !r.last_known)
        .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(s2.on(RUN, mark).is_empty(), "Run receives nothing");
    assert_eq!(w.target.acts(), acts, "acted on nothing");

    // 3. (text 0.4, F-197.) The cut again: the author puts v2, which
    //    neither the target nor the archive receives. X holds an instance
    //    token at the author's address on R2, and no queryable; the
    //    target's process ends, and restarts, a new run.
    bus.link().cut();
    author_seen(&bus.ep2, false).await;
    let sv2 = put_stamp(a.author.put(WORKER, bytes(&v(2))).await.unwrap());
    let x = client(&bus.ep2).await;
    let x_token = x
        .liveliness()
        .declare_token("zk2/lab/author/@zk/instance/00000000000000aa")
        .await
        .unwrap();
    author_seen(&bus.ep2, true).await;
    w.kill().await;
    let mark = s2.mark();
    let w = worker_up(&bus.ep2, true, Follow::Watch, with_archive(), false).await;
    w.seeded().await;
    // Its author present, its GET silent: no archive read, nothing acted
    // on, the baseline reported.
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        w.target.last_seed(),
        Some(desired::Seeded {
            answered: 0,
            author: desired::AuthorSeen::Present,
            archive: None,
        }),
        "the reference reads no archive while its author is present"
    );
    assert_eq!(w.target.acts(), 0, "acted on nothing");
    assert_eq!(
        s2.on(RP, mark).last().map(Rec::report),
        Some(Report::baseline(Some(D.to_owned())))
    );
    assert!(
        s2.on(RUN, mark)
            .iter()
            .all(|r| r.value() == drive::baseline()),
        "Run holds the baseline"
    );
    let lk = zenkey::archive::last_known(&x, &addr("lab/archive"), D, T)
        .await
        .unwrap()
        .expect("the archive's last-known of D");
    assert_eq!(
        lk.value.as_deref(),
        Some(&bytes(&v(1))[..]),
        "the archive holds v1, older than v2"
    );

    // 4. The heal: the author's own token appears on R2, and v2 converges;
    //    then X withdraws its token, which moves nothing.
    bus.link().heal();
    s2.until("converged on v2", |r| {
        converged_on(r, &sv2, 2) && !r.last_known
    })
    .await;
    let acts = w.target.acts();
    drop(x_token);
    x.close().await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(w.target.acts(), acts, "X's withdrawal moves nothing");
    w.svc.close().await.unwrap();
    a.svc.close().await.unwrap();
}

/// `desired.v1` §2.16: `config.v1`'s owner `lab/rf0` as a target, its role
/// `configs` bound to the author's slice. A put converges through `set`
/// and is reported in the view and the status; a refused one keeps what
/// was in force; a delete reverts each parameter the documents named to its
/// startup value, and the view carries no `desired` after it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn config_v1_as_a_target() {
    use zenkey::config_v1;
    use zenkey::model::config::{DesiredOutcome, Value as CValue};

    const RES: &str = CONFIGS;
    let bus = one_router().await;
    let configs = imp("configs.v1");
    let ci = configs.iface().clone();

    let author_session = client(&bus.ep1).await;
    let mut ab = ServiceBuilder::new(&author_session, config(AUTHOR));
    ab.implement(configs.clone()).unwrap();
    let author = ab
        .desired_author(&ci, RES, AuthorStartup::default())
        .await
        .unwrap();
    let asvc = ab.start().await.unwrap();

    let owner = client(&bus.ep1).await;
    let mut cfg = config(RF0).bind("configs", &[AUTHOR]);
    cfg.bindings.get_mut("configs").unwrap().params = [
        ("system".to_owned(), "self.system".to_owned()),
        ("service".to_owned(), "self.service".to_owned()),
    ]
    .into();
    let startup = sim::rf_startup(false, 10, Duration::from_secs(1), true);
    let device = Arc::new(sim::SimDevice::new(&startup));
    let mut b = ServiceBuilder::new(&owner, cfg);
    b.require("configs", ci.clone(), true);
    let c = b
        .config_v1(startup, Arc::clone(&device) as Arc<dyn config_v1::Device>)
        .await
        .unwrap();
    let t = b
        .desired_config("configs", &configs, RES, None, TargetStartup::default())
        .await
        .unwrap();
    let svc = b.start().await.unwrap();
    eventually("the target's first seed", || async {
        t.last_seed().is_some()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let duty = |c: config_v1::ConfigStatus| c.values["duty"]["fraction_ppm"].clone();
    assert_eq!(duty(c.status().await), CValue::Integer(10000));

    // A put: converged, in config.v1's words.
    let r1 = put_stamp(
        author
            .put(
                "lab/rf0",
                bytes(&json!({"revision": 1, "values": {"duty": {"fraction_ppm": 20000}}})),
            )
            .await
            .unwrap(),
    );
    eventually("revision 1 converged", || async {
        c.desired()
            .await
            .is_some_and(|d| d.observed_revision == 1 && d.outcome == DesiredOutcome::Converged)
    })
    .await;
    assert_eq!(duty(c.status().await), CValue::Integer(20000));
    eventually("the role's report", || async {
        t.report().await.outcome == Some(Outcome::Converged)
    })
    .await;
    assert_eq!(
        t.report().await.in_force,
        Some(version(&r1, Some(1))),
        "desired.v1's report beside config.v1's"
    );

    // Refused whole: a reach group.
    let r2 = put_stamp(
        author
            .put(
                "lab/rf0",
                bytes(&json!({"revision": 2, "values": {
                    "duty": {"fraction_ppm": 30000}, "radio": {"frequency_hz": 869525000}}})),
            )
            .await
            .unwrap(),
    );
    eventually("revision 2 refused", || async {
        t.report().await.observed == Some(version(&r2, Some(2)))
    })
    .await;
    let rep = t.report().await;
    assert_eq!(rep.outcome, Some(Outcome::Refused));
    let refusal = rep.refusal.unwrap();
    assert_eq!(
        (refusal.code.as_str(), refusal.pointer.as_deref()),
        ("reach", Some("/values/radio"))
    );
    assert_eq!(rep.in_force, Some(version(&r1, Some(1))));
    assert_eq!(c.desired().await.unwrap().outcome, DesiredOutcome::Refused);
    assert_eq!(duty(c.status().await), CValue::Integer(20000));

    // The delete: the parameters the documents named back at their startup
    // values, and no desired in the view.
    let del = put_stamp(author.delete("lab/rf0").await.unwrap());
    eventually("reverted", || async {
        t.report().await.outcome == Some(Outcome::Reverted)
    })
    .await;
    assert_eq!(t.report().await.observed, Some(version(&del, None)));
    assert_eq!(
        duty(c.status().await),
        CValue::Integer(10000),
        "the file's value"
    );
    assert_eq!(c.desired().await, None);
    assert_eq!(
        c.view().await.desired,
        None,
        "the view carries no desired: no document is bound"
    );
    svc.close().await.unwrap();
    asvc.close().await.unwrap();
}

/// `config.v1`'s owner `lab/rf0` as a target of `lab/author`'s
/// `configs.v1`, on `device`, its floor `floor_s`.
struct ConfigUp {
    _sessions: (zenoh::Session, zenoh::Session),
    author: desired::Author,
    asvc: Service,
    svc: Service,
    cfg: zenkey::config_v1::Configuration,
    target: desired::Target,
}

const RF0: &str = "lab/rf0";
const CONFIGS: &str = "state/configs/{system}/{service}";

async fn config_up(
    bus: &Bus,
    startup: zenkey::config_v1::Startup,
    device: Arc<dyn zenkey::config_v1::Device>,
) -> ConfigUp {
    let configs = imp("configs.v1");
    let ci = configs.iface().clone();
    let author_session = client(&bus.ep1).await;
    let mut ab = ServiceBuilder::new(&author_session, config(AUTHOR));
    ab.implement(configs.clone()).unwrap();
    let author = ab
        .desired_author(&ci, CONFIGS, AuthorStartup::default())
        .await
        .unwrap();
    let asvc = ab.start().await.unwrap();
    let owner = client(&bus.ep1).await;
    let (svc, c, target) = config_owner(&owner, startup, device).await;
    ConfigUp {
        _sessions: (author_session, owner),
        author,
        asvc,
        svc,
        cfg: c,
        target,
    }
}

/// `lab/rf0` on `session`: `config.v1`'s owner from `startup` on `device`,
/// fed through `desired.v1`, its role `configs` bound to `lab/author` with
/// `{system} = self.system` and `{service} = self.service`; started, its
/// first seed made.
async fn config_owner(
    session: &zenoh::Session,
    startup: zenkey::config_v1::Startup,
    device: Arc<dyn zenkey::config_v1::Device>,
) -> (Service, zenkey::config_v1::Configuration, desired::Target) {
    let configs = imp("configs.v1");
    let ci = configs.iface().clone();
    let mut cfg = config(RF0).bind("configs", &[AUTHOR]);
    cfg.bindings.get_mut("configs").unwrap().params = [
        ("system".to_owned(), "self.system".to_owned()),
        ("service".to_owned(), "self.service".to_owned()),
    ]
    .into();
    let mut b = ServiceBuilder::new(session, cfg);
    b.require("configs", ci, true);
    let c = b.config_v1(startup, device).await.unwrap();
    let target = b
        .desired_config("configs", &configs, CONFIGS, None, TargetStartup::default())
        .await
        .unwrap();
    let svc = b.start().await.unwrap();
    eventually("the target's first seed", || async {
        target.last_seed().is_some()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    (svc, c, target)
}

impl ConfigUp {
    async fn duty(&self) -> zenkey::model::config::Value {
        self.cfg.status().await.values["duty"]["fraction_ppm"].clone()
    }

    async fn until(&self, what: &str, f: impl Fn(&Report) -> bool) -> Report {
        let deadline = tokio::time::Instant::now() + SETTLE;
        loop {
            let r = self.target.report().await;
            if f(&r) {
                return r;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "never: {what}: {r:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn close(self) {
        self.svc.close().await.unwrap();
        self.asvc.close().await.unwrap();
    }
}

/// `desired.v1` §2.16, a revert that fails: the device refuses the `set`
/// that would restore `duty`, so the revert ends `failed` and `duty` stays
/// the delete's to revert: a document naming nothing, then its delete,
/// restores it (review of PR #850).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn config_v1_a_failed_revert_keeps_what_it_did_not_restore() {
    use zenkey::model::config::Value as CValue;

    let bus = one_router().await;
    let startup = sim::rf_startup(false, 10, Duration::from_secs(1), true);
    let device = Arc::new(sim::SimDevice::new(&startup));
    let up = config_up(
        &bus,
        startup,
        Arc::clone(&device) as Arc<dyn zenkey::config_v1::Device>,
    )
    .await;
    let doc = |rev: u64, values: Value| bytes(&json!({"revision": rev, "values": values}));
    up.author
        .put(RF0, doc(1, json!({"duty": {"fraction_ppm": 20000}})))
        .await
        .unwrap();
    up.until("revision 1 converged", |r| {
        r.outcome == Some(Outcome::Converged)
    })
    .await;
    assert_eq!(up.duty().await, CValue::Integer(20000));

    // The revert's set refused: failed, duty not restored.
    device.with(|s| s.refuse_apply = Some("the radio is busy".to_owned()));
    up.author.delete(RF0).await.unwrap();
    let r = up
        .until("the revert failed", |r| r.outcome == Some(Outcome::Failed))
        .await;
    assert_eq!(r.source, Source::Baseline);
    let f = r.refusal.unwrap();
    assert_eq!(
        (f.code.as_str(), f.pointer.as_deref()),
        ("device-refused", Some("/values/duty"))
    );
    assert_eq!(up.duty().await, CValue::Integer(20000));
    assert_eq!(up.cfg.desired().await, None);

    // A document that names nothing converges, and its delete reverts what
    // the first delete could not.
    let r2 = put_stamp(up.author.put(RF0, doc(2, json!({}))).await.unwrap());
    up.until("revision 2 converged", |r| {
        r.observed == Some(version(&r2, Some(2))) && r.outcome == Some(Outcome::Converged)
    })
    .await;
    assert_eq!(up.duty().await, CValue::Integer(20000), "nothing set");
    up.author.delete(RF0).await.unwrap();
    up.until("reverted", |r| r.outcome == Some(Outcome::Reverted))
        .await;
    assert_eq!(
        up.duty().await,
        CValue::Integer(10000),
        "the parameter the failed revert kept named, restored"
    );
    up.close().await;
}

/// A device whose undo never returns: a pending change it holds never ends.
struct HangingUndo(Arc<sim::SimDevice>);

impl zenkey::config_v1::Device for HangingUndo {
    fn apply<'a>(
        &'a self,
        group: &'a str,
        values: &'a std::collections::BTreeMap<String, zenkey::config_v1::Value>,
    ) -> zenkey::call::BoxFuture<'a, Result<(), String>> {
        zenkey::config_v1::Device::apply(&*self.0, group, values)
    }

    fn undo<'a>(
        &'a self,
        _restore: &'a [zenkey::config_v1::Restore],
    ) -> zenkey::call::BoxFuture<'a, Result<(), zenkey::config_v1::UndoFailed>> {
        Box::pin(std::future::pending())
    }
}

/// `desired.v1` §2.16, §2.7, a wait that cannot end: an operator's change
/// with a 2 s window is pending, and its undo at the deadline never
/// returns. The document's act gives up 5 s past the deadline, `failed`,
/// `apply`, and the next version is acted on, never held behind it
/// (review of PR #850).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn config_v1_a_wait_that_cannot_end_gives_up() {
    use zenkey::config_v1;

    let bus = one_router().await;
    let startup = sim::rf_startup(false, 2, Duration::from_millis(500), true);
    let device = Arc::new(HangingUndo(Arc::new(sim::SimDevice::new(&startup))));
    let up = config_up(&bus, startup, device as Arc<dyn config_v1::Device>).await;

    // The operator's change, its window 2 s: pending.
    let tool = client(&bus.ep1).await;
    let c = zenkey::Client::new(&tool, config_v1::implementation().shared_contract(), &[RF0])
        .unwrap()
        .with_timeout(Duration::from_secs(2));
    let values: Bindings = [("group".to_owned(), vec!["duty".to_owned()])].into();
    let out = c
        .call_value(
            &addr(RF0),
            config_v1::SET,
            &values,
            &json!({"values": {"fraction_ppm": 50000}, "confirm_s": 2}),
        )
        .await
        .unwrap();
    assert!(matches!(out, zenkey::Outcome::Value(_)), "{out:?}");
    assert!(up.cfg.view().await.pending.is_some(), "a change pending");

    // A document: its act waits, past the deadline, and gives up.
    let asked = Instant::now();
    let r1 = put_stamp(
        up.author
            .put(
                RF0,
                bytes(&json!({"revision": 1, "values": {"duty": {"fraction_ppm": 20000}}})),
            )
            .await
            .unwrap(),
    );
    let r = up
        .until("the wait given up", |r| {
            r.observed == Some(version(&r1, Some(1)))
        })
        .await;
    let waited = asked.elapsed();
    assert!(
        waited >= config_v1::DESIRED_WAIT_PAST_DEADLINE,
        "gave up {waited:?} after the put: past the deadline's grace"
    );
    assert_eq!(r.outcome, Some(Outcome::Failed));
    let f = r.refusal.unwrap();
    assert_eq!(f.code, "apply");
    assert!(
        f.detail
            .as_deref()
            .is_some_and(|d| d.contains("pending change")),
        "{f:?}"
    );

    // The next version is acted on, not held behind the stuck one.
    let r2 = put_stamp(
        up.author
            .put(
                RF0,
                bytes(&json!({"revision": 2, "values": {"duty": {"fraction_ppm": 30000}}})),
            )
            .await
            .unwrap(),
    );
    up.until("revision 2 acted on", |r| {
        r.observed == Some(version(&r2, Some(2))) && r.outcome == Some(Outcome::Failed)
    })
    .await;
    assert_eq!(up.target.acts(), 2);
    up.close().await;
}

/// `scenarios.md` §12: a `config.v1` owner fed through `desired.v1`,
/// converged on revision 1. Its re-mint is refused while it runs, as
/// `config.v1` §2.12 lets an owner keep its rule, and nothing moves; its
/// restart begins a new run, which acts on revision 1 again (`desired.v1`
/// §2.7, §2.16; zk2py round 87's F-336, #935).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s12_a_config_v1_owners_re_mint_is_a_restart() {
    use zenkey::config_v1;
    use zenkey::model::config::DesiredOutcome;

    const ST: &str = "zk2/lab/rf0/config.v1/state/status";
    const EV: &str = "zk2/lab/rf0/config.v1/events/change/";
    const TOKENS: &str = "zk2/lab/rf0/@zk/instance/";
    let converged = |d: Option<config_v1::DesiredReport>| {
        d.is_some_and(|d| d.observed_revision == 1 && d.outcome == DesiredOutcome::Converged)
    };
    let bus = one_router().await;
    let s = record(&bus.ep1).await;
    let startup = sim::rf_startup(false, 10, Duration::from_secs(1), true);
    let device = Arc::new(sim::SimDevice::new(&startup));
    let mut up = config_up(
        &bus,
        startup.clone(),
        Arc::clone(&device) as Arc<dyn config_v1::Device>,
    )
    .await;
    let duty = || device.holds("duty", "fraction_ppm");

    // Setup: converged on revision 1.
    up.author
        .put(
            RF0,
            bytes(&json!({"revision": 1, "values": {"duty": {"fraction_ppm": 20000}}})),
        )
        .await
        .unwrap();
    eventually("revision 1 converged", || async {
        converged(up.cfg.desired().await)
    })
    .await;
    assert_eq!(duty(), Some(config_v1::Value::Integer(20000)));

    // 1. A re-mint while it runs: refused, and nothing moves.
    let old = up.svc.instance().to_string();
    let mark = s.mark();
    let err = up.svc.new_epoch().await.expect_err("the re-mint refused");
    assert!(err.to_string().contains("config.v1 §2.12"), "{err}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(up.svc.instance().to_string(), old);
    assert!(
        !s.all()
            .iter()
            .skip(mark)
            .any(|r| r.token && r.key.starts_with(TOKENS)),
        "no instance token of lab/rf0 withdrawn or declared"
    );
    assert!(converged(up.cfg.desired().await), "St still converged");
    assert_eq!(duty(), Some(config_v1::Value::Integer(20000)));

    // 2. The process stops, and starts again: a new run, from the startup
    // values, which acts on revision 1 again.
    let ConfigUp {
        _sessions: sessions,
        asvc,
        svc,
        ..
    } = up;
    let mark = s.mark();
    svc.close().await.unwrap();
    device.restart(&startup);
    let (svc, cfg, _target) = config_owner(
        &sessions.1,
        startup,
        Arc::clone(&device) as Arc<dyn config_v1::Device>,
    )
    .await;
    let new = svc.instance().to_string();
    assert_ne!(new, old);
    eventually("revision 1 converged again", || async {
        converged(cfg.desired().await)
    })
    .await;
    assert_eq!(duty(), Some(config_v1::Value::Integer(20000)));
    let since = s.all().into_iter().skip(mark).collect::<Vec<_>>();
    assert!(
        since
            .iter()
            .any(|r| r.token && r.delete && r.key == format!("{TOKENS}{old}")),
        "the old instance token withdrawn"
    );
    assert!(
        since
            .iter()
            .any(|r| r.token && !r.delete && r.key == format!("{TOKENS}{new}")),
        "the new instance token declared"
    );
    let statuses: Vec<Value> = since
        .iter()
        .filter(|r| !r.token && !r.delete && r.key == ST)
        .map(Rec::value)
        .filter(|v| v["instance"] == json!(new))
        .collect();
    let first = statuses.first().expect("a status of the new instance");
    assert_eq!(first["values"]["duty"]["fraction_ppm"], json!(10000));
    assert!(
        first.get("desired").is_none(),
        "the new instance knows nothing of the old one's report: {first}"
    );
    let applied = since
        .iter()
        .filter(|r| !r.token && !r.delete && r.key.starts_with(EV))
        .map(Rec::value)
        .filter(|e| e["instance"] == json!(new) && e["outcome"] == json!("applied"))
        .count();
    assert_eq!(applied, 1, "the new run's act, one `set`, applied");
    eventually("S's latest status converged", || async {
        s.on(ST, mark).last().map(Rec::value).is_some_and(|v| {
            v["instance"] == json!(new)
                && v["values"]["duty"]["fraction_ppm"] == json!(20000)
                && v["desired"] == json!({"observed_revision": 1, "outcome": "converged"})
        })
    })
    .await;
    svc.close().await.unwrap();
    asvc.close().await.unwrap();
}

/// The two contracts the scenarios give load as the owner example embeds
/// them, byte for byte.
#[test]
fn the_examples_contracts_are_the_scenarios() {
    let (tasking, worker) = drive::contracts().unwrap();
    assert_eq!(
        Implementation::new(tasking).fingerprint(),
        imp("tasking.v1").fingerprint()
    );
    assert_eq!(
        Implementation::new(worker).fingerprint(),
        imp("worker.v1").fingerprint()
    );
}

// --- §11: one document type per role --------------------------------------------

/// A drive that never runs: §11's roles are refused before any act.
struct Never;

impl desired::Handler for Never {
    fn apply<'a>(
        &'a self,
        _: &'a desired::Document,
    ) -> zenkey::call::BoxFuture<'a, desired::Ended> {
        Box::pin(async { desired::Ended::Done })
    }
    fn revert<'a>(&'a self, _: &'a Version) -> zenkey::call::BoxFuture<'a, desired::Ended> {
        Box::pin(async { desired::Ended::Done })
    }
}

/// A copy of a scenarios' contract, edited by `edit`, read from
/// `tests/contracts/` so that its schemas resolve (§11's Setup).
fn copy_of(name: &str, edit: impl Fn(String) -> String) -> Implementation {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/contracts");
    let file = format!("{name}.toml");
    let text = edit(std::fs::read_to_string(dir.join(&file)).unwrap());
    let l = zenkey::model::contract::load_str(&text, &dir, Some(file.as_str()));
    Implementation::new(l.contract.unwrap_or_else(|| panic!("{text}\n{}", l.report)))
}

/// §11's tasking.v1, two documents: a second desired resource,
/// `plans/{system}/{service}`, declared as `tasks/…` is.
fn tasking_two() -> Implementation {
    copy_of("tasking.v1", |t| {
        t + "\n[resources.\"plans/{system}/{service}\"]\nkind        = \"state\"\n\
             type        = \"json:Task\"\n\
             params      = { system = \"string\", service = \"string\" }\n\
             cardinality = 16\n\
             annotations = { \"desired.target_param\" = [\"system\", \"service\"], \
             \"freshness.ttl_s\" = 0 }\n"
    })
}

/// worker.v1 with its requirement's `resources` line replaced by `line`
/// (removed when empty), or with no `[requires.task]` at all (`None`).
fn worker_with(line: Option<&str>) -> Implementation {
    copy_of("worker.v1", |t| {
        let at = t.find("[requires.task]").unwrap();
        match line {
            None => t[..at].to_owned(),
            Some(l) => t.replace("resources = [\"tasks/{system}/{service}\"]\n", l),
        }
    })
}

/// `lab/worker`'s builder, its role bound as "The target" says, given
/// `worker`, with a role of the manifest's when `manifest`, and the role
/// given against `tasking`.
async fn given_the_role(
    session: &zenoh::Session,
    worker: Implementation,
    manifest: bool,
    tasking: &Implementation,
) -> (ServiceBuilder, zenkey::Result<desired::Target>) {
    let wi = worker.iface().clone();
    let mut cfg = config(WORKER).bind(ROLE, &[AUTHOR]);
    cfg.bindings.get_mut(ROLE).unwrap().params = [
        ("system".to_owned(), "self.system".to_owned()),
        ("service".to_owned(), "self.service".to_owned()),
    ]
    .into();
    let mut b = ServiceBuilder::new(session, cfg);
    b.implement(worker).unwrap();
    if manifest {
        b.require(ROLE, tasking.iface().clone(), true);
    }
    let r = b
        .desired_target(
            ROLE,
            tasking,
            TASKS,
            ReportAt::new(wi, REPORT),
            Arc::new(Never) as Arc<dyn desired::Handler>,
            TargetStartup::default(),
        )
        .await;
    (b, r)
}

/// A start's refusal by its reason: `not started: …` (core §8.2 step 2).
fn refused_two_documents<T>(r: zenkey::Result<T>, what: &str) {
    match r {
        Err(e @ zenkey::Error::NotExposed(_)) => {
            let why = e.to_string();
            assert!(why.starts_with("not started: "), "{what}: {why}");
            assert!(why.contains("(two_documents)"), "{what}: {why}");
        }
        Err(other) => panic!("{what}: not the start's refusal: {other}"),
        Ok(_) => panic!("{what}: a role on two documents is refused"),
    }
}

/// S's instance tokens of `lab/worker`, put (appeared).
fn worker_tokens(s: &Recorder) -> usize {
    s.all()
        .iter()
        .filter(|r| r.token && !r.delete && r.key.starts_with("zk2/lab/worker/@zk/instance/"))
        .count()
}

/// §11 A role on two documents refuses the start (§2.1, §2.3, text 0.7,
/// F-224): the target given worker.v1, both, against tasking.v1, two
/// documents, refuses the role and the start, nothing declared: S sees no
/// instance token of it in 5 s, and no report. The control, worker.v1
/// naming one of the two, starts.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s11_a_role_on_two_documents_refuses_the_start() {
    let bus = one_router().await;
    let s = record(&bus.ep1).await;
    let g = client(&bus.ep1).await;
    let tasking = tasking_two();
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 1. worker.v1, both: the role refused, and the start.
    let session = client(&bus.ep2).await;
    let both = worker_with(Some(
        "resources = [\"tasks/{system}/{service}\", \"plans/{system}/{service}\"]\n",
    ));
    let (b, given) = given_the_role(&session, both, false, &tasking).await;
    refused_two_documents(given, "the role");
    refused_two_documents(b.start().await, "the start");
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(
        worker_tokens(&s),
        0,
        "no instance token of lab/worker in 5 s"
    );
    assert!(s.on(RP, 0).is_empty(), "no report on Rp");
    assert!(get(&g, RP).await.is_empty(), "nothing answers on Rp");

    // 2. The control: the scenarios' worker.v1, naming one of the two.
    let session = client(&bus.ep2).await;
    let (mut b, given) = given_the_role(&session, imp("worker.v1"), false, &tasking).await;
    let _target = given.expect("a role on one document type");
    let wi = imp("worker.v1").iface().clone();
    let run = b
        .declare_state_writer(&wi, RUNNING, &Bindings::new())
        .await
        .unwrap();
    run.put_value(&drive::baseline()).await.unwrap();
    let svc = b.start().await.expect("the control starts");
    eventually("the control's instance token at S", || async {
        worker_tokens(&s) == 1
    })
    .await;
    s.until("its baseline report (§2.10)", |r| {
        *r == Report::baseline(Some(D.to_owned()))
    })
    .await;
    svc.close().await.unwrap();
}

/// What a requirement names (core §3.1; desired.v1 §2.1, text 0.7): one
/// naming no resource names every one, and so does a role of the
/// component's manifest, whose descriptor names no declaring contract:
/// against tasking.v1, two documents, either is `two_documents`, refused
/// with the start; against the scenarios' tasking.v1, one document, a
/// manifest's role is given; and a requirement naming one of two is.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_requirement_names_every_resource_or_those_it_lists() {
    let bus = one_router().await;
    let session = client(&bus.ep1).await;
    let two = tasking_two();
    let (b, given) = given_the_role(&session, worker_with(Some("")), false, &two).await;
    refused_two_documents(given, "a requirement naming none");
    refused_two_documents(b.start().await, "its start");
    let (b, given) = given_the_role(&session, worker_with(None), true, &two).await;
    refused_two_documents(given, "a manifest's role");
    refused_two_documents(b.start().await, "its start");
    let (_b, given) = given_the_role(&session, worker_with(None), true, &imp("tasking.v1")).await;
    let _ = given.expect("a manifest's role on one desired resource");
    let (_b, given) = given_the_role(&session, imp("worker.v1"), false, &two).await;
    let _ = given.expect("a requirement naming one of two");
}
