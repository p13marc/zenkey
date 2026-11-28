//! `spec/profiles/desired/scenarios-v2.md` §1–§8 and §10–§16 (`desired.v2`,
//! #986's J3), one test per section, named after it. §9 is a tool's (J4).
//! §16 runs its five runs at once, each on a router of its own.
//! `config_v2_as_a_target` runs `desired.v2` §2.16 beside them.
//!
//! Common setup, the text's: R1 is a router with timestamping on, a
//! router's default. Where a section cuts a link, the author's side is R1
//! and the target's side R2, a router linked to R1 through a [`Link`] the
//! test cuts and heals; elsewhere everything is a client of R1. The author
//! `lab/author` implements `tasking.v2` through
//! [`ServiceBuilder::desired_author`], `tasking.v2` in its tokenless set,
//! its runs from a record of its own, so 16-bit ones (core §4.3); the
//! target `lab/worker` implements `worker.v2`, its role `task` bound to the
//! author with `{system} = self.system` and `{service} = self.service`,
//! through [`ServiceBuilder::desired_target`], on the owner example's
//! simulated drive (`examples/owner/desired.rs`), which refuses with the
//! lines `worker.v2`'s tables hold. S records every delivery under
//! `zk2/lab/**` and every token under it on R1; where a link is cut, S2
//! records the same on R2. G GETs as core S4 says.
//!
//! What a section observes of a read is the author's own word
//! ([`desired::Author::reads`]): for each GET of the slice, the held stamp
//! it carried and whether it was answered with that stamp alone. In §13 a
//! session of the test's plays the author, and counts what it receives.
//!
//! Each section runs on routers of its own, so the sections run at once.

mod common;

#[allow(dead_code)]
#[path = "../examples/owner/desired.rs"]
mod drive;

#[allow(dead_code)]
#[path = "../examples/owner/config.rs"]
mod sim;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{Link, SETTLE, T, addr, client, config, eventually, imp, link_to, router, router_via};
use drive::{Drive, REPORT, ROLE, RUNNING, TASKS};
use serde_json::{Value, json};
use zenkey::desired::{
    self, Asked, AuthorStartup, FileRecord, Found, Major, ReportAt, TargetStartup, Written,
};
use zenkey::model::collection::Page;
use zenkey::model::desired::Source;
use zenkey::model::desired::v2::{self as model, Code, Index, Observed, Outcome, Report};
use zenkey::model::template::Bindings;
use zenkey::model::wire::{Envelope, Run, Seq, Stamp, Tag};
use zenkey::{Implementation, Service, ServiceBuilder, presence};
use zenoh::query::{ConsolidationMode, QueryTarget, Reply};
use zenoh::sample::{Sample, SampleKind};

const AUTHOR: &str = "lab/author";
const WORKER: &str = "lab/worker";
/// The target's slice.
const D: &str = "zk2/lab/author/tasking.v2/state/tasks/lab/worker";
/// A second slice of the author's (§6, §14).
const D2: &str = "zk2/lab/author/tasking.v2/state/tasks/lab/spare";
/// The target's report.
const RP: &str = "zk2/lab/worker/worker.v2/state/desired/task";
/// What the target runs.
const RUN: &str = "zk2/lab/worker/worker.v2/state/running";
/// The author's instance tokens.
const AUTHOR_TOKENS: &str = "zk2/lab/author/@zk/instance/*";

/// `v<n>`: a Task with revision n, mode run and rate_hz 10.
fn v(n: u64) -> Value {
    json!({"revision": n, "mode": "run", "rate_hz": 10})
}

fn bytes(doc: &Value) -> Vec<u8> {
    serde_json::to_vec(doc).unwrap()
}

fn observed(version: Stamp, revision: Option<u64>) -> Option<Observed> {
    Some(Observed { version, revision })
}

/// "Converged on v*n*" (`scenarios-v2.md`): `source` desired, `outcome`
/// converged, `in_force` observed, the observed version `s`, `revision` n.
fn converged_on(r: &Report, s: Stamp, revision: u64) -> bool {
    let want = observed(s, Some(revision));
    r.source == Source::Desired
        && r.outcome == Outcome::Converged
        && r.observed == want
        && r.in_force == want
        && r.refusal.is_none()
}

/// The version a `desired.v2` author wrote under.
fn version(w: Written) -> Stamp {
    w.version()
        .unwrap_or_else(|| panic!("not written under a version: {w:?}"))
}

fn is_16_bit(s: Stamp) -> bool {
    matches!(s.run, Run::U16(_))
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

/// G's GET of `key`, as core S4 says: each reply as it arrived.
async fn get_replies(s: &zenoh::Session, key: &str) -> Vec<Sample> {
    let rx = s
        .get(key)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::None)
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

/// G's GET of a member's key, which draws a snapshot (core §4.2): its one
/// page. Asked again within [`SETTLE`] while it draws nothing, since a GET
/// can end at its timeout before the owner's reply under load.
async fn snapshot(s: &zenoh::Session, key: &str) -> Page {
    let deadline = Instant::now() + SETTLE;
    loop {
        let replies = get_replies(s, key).await;
        if !replies.is_empty() {
            assert_eq!(replies.len(), 1, "one page for a member read of {key}");
            assert_eq!(replies[0].key_expr().as_str(), key);
            return match zenkey::inband::read(replies[0].clone()) {
                zenkey::inband::Read::Snapshot { page: Ok(p), .. } => {
                    assert!(p.index == 0 && !p.more, "one page");
                    *p
                }
                other => panic!("a GET of {key} draws a snapshot, not {other:?}"),
            };
        }
        assert!(Instant::now() < deadline, "no reply to a GET of {key}");
    }
}

/// A page's one live entry at the page's own key: its stamp and its
/// record.
fn live_entry(p: &Page) -> (Stamp, Vec<u8>) {
    assert!(
        p.live.len() == 1 && p.tombs.is_empty() && p.live[0].id.is_empty(),
        "one live entry, with the empty id: {p:?}"
    );
    let e = &p.live[0];
    (
        Stamp {
            run: e.run.unwrap_or(p.run),
            seq: e.seq,
        },
        e.record.clone(),
    )
}

/// A page's one tombstone at the page's own key: its stamp.
fn tombstone(p: &Page) -> Stamp {
    assert!(
        p.tombs.len() == 1 && p.live.is_empty() && p.tombs[0].id.is_empty(),
        "one tombstone, with the empty id: {p:?}"
    );
    Stamp {
        run: p.tombs[0].run.unwrap_or(p.run),
        seq: p.tombs[0].seq,
    }
}

// --- S: the recording subscriber --------------------------------------------------

/// One delivery, as S records it.
#[derive(Debug, Clone)]
struct Rec {
    key: String,
    delete: bool,
    /// Its record, without its envelope.
    payload: Vec<u8>,
    /// Its Zenoh timestamp's id, where it carries one.
    stamped_by: Option<String>,
    /// Its in-band stamp (core §4.3).
    inband: Option<Stamp>,
    /// CH's tag.
    tag: Option<Tag>,
    token: bool,
}

impl Rec {
    /// A delivery on Rp, read as §2.10 says: under tag 12, with no finding.
    fn report(&self) -> Report {
        assert_eq!(self.tag, Tag::new(model::TAG), "a Report's tag is 12");
        let read = model::read(model::TAG, &self.payload).expect("a packed Report");
        assert!(read.findings.is_empty(), "consistent: {:?}", read.findings);
        read.report.expect("a report with no finding")
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
            payload: common::record(x),
            stamped_by: x.timestamp().map(|t| t.get_id().to_string()),
            inband: common::inband(x),
            tag: zenkey::inband::read(x.clone()).tag(),
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

    /// "The report": Rp's record, the latest by seq within its run. One
    /// writer puts it, so S's latest delivery is it.
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

    /// Waits until a delivery on `key` since `mark` holds `f`.
    async fn until_on(&self, key: &str, since: usize, what: &str, f: impl Fn(&Rec) -> bool) {
        let deadline = tokio::time::Instant::now() + SETTLE;
        while !self.on(key, since).iter().any(&f) {
            assert!(tokio::time::Instant::now() < deadline, "never: {what}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

// --- the author and the target ---------------------------------------------------

/// `tasking.v2`, or "tasking.v2, watched" with `ttl_s` above 0
/// (`scenarios-v2.md`: the same file with that `freshness.ttl_s`).
fn tasking(ttl_s: u64) -> Implementation {
    if ttl_s == 0 {
        return imp("tasking.v2");
    }
    copy_of("tasking.v2", |t| {
        let watched = t.replace(
            "\"freshness.ttl_s\" = 0 }",
            &format!("\"freshness.ttl_s\" = {ttl_s} }}"),
        );
        assert_ne!(watched, t);
        watched
    })
}

/// A copy of a scenarios' contract, edited by `edit`, read from
/// `tests/contracts/` so that its schemas resolve.
fn copy_of(name: &str, edit: impl Fn(String) -> String) -> Implementation {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/contracts");
    let file = format!("{name}.toml");
    let text = edit(std::fs::read_to_string(dir.join(&file)).unwrap());
    let l = zenkey::model::contract::load_str(&text, &dir, Some(file.as_str()));
    Implementation::new(l.contract.unwrap_or_else(|| panic!("{text}\n{}", l.report)))
}

/// How the author starts: its record, its window, its runs' record, and
/// its clock.
#[derive(Clone, Default)]
struct AuthorAt {
    startup: AuthorStartup,
    window_s: Option<u64>,
    /// Its runs' record (core §4.3): with one it writes under 16-bit runs,
    /// without one under a random 32-bit run.
    runs: Option<PathBuf>,
    offset_ms: i64,
    ttl_s: u64,
}

struct AuthorUp {
    session: zenoh::Session,
    svc: Service,
    author: desired::Author,
}

/// `lab/author` on a client of `ep`: `tasking.v2` in its tokenless set.
async fn author_up(ep: &str, at: &AuthorAt) -> AuthorUp {
    let session = client(ep).await;
    let tasking = tasking(at.ttl_s);
    let iface = tasking.iface().clone();
    let mut cfg = config(AUTHOR).tokenless(iface.clone());
    cfg.tombstone_window_s = at.window_s;
    let mut b = ServiceBuilder::new(&session, cfg);
    b.minter().simulate_offset(at.offset_ms);
    if let Some(runs) = &at.runs {
        b.run_store(Arc::new(zenkey::run::FileRunStore::new(runs)))
            .unwrap();
    }
    b.implement(tasking).unwrap();
    let author = b
        .desired_author(&iface, TASKS, at.startup.clone())
        .await
        .unwrap();
    assert_eq!(author.major(), Major::V2);
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

    /// The latest GET of the target's slice the author answered.
    fn last_read(&self) -> Option<Asked> {
        self.author.reads(WORKER).unwrap().last().copied()
    }
}

/// A directory of the test's, and the author's two records in it.
struct Records {
    _dir: tempfile::TempDir,
    record: PathBuf,
    runs: PathBuf,
}

fn records() -> Records {
    let dir = tempfile::tempdir().unwrap();
    Records {
        record: dir.path().join("author.json"),
        runs: dir.path().join("author.run"),
        _dir: dir,
    }
}

impl Records {
    /// The scenarios' author: its runs' record kept, and its documents'
    /// when `record`.
    fn author(&self, record: bool) -> AuthorAt {
        AuthorAt {
            startup: if record {
                AuthorStartup::default().with_record(Arc::new(FileRecord::new(&self.record)))
            } else {
                AuthorStartup::default()
            },
            runs: Some(self.runs.clone()),
            ..AuthorAt::default()
        }
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

/// `lab/worker`'s builder on `session`, its role bound (or, `bound` false,
/// the kill switch), given the role against `tasking`.
async fn worker_builder(
    session: &zenoh::Session,
    bound: bool,
    tasking: &Implementation,
    startup: TargetStartup,
    hold: bool,
) -> (ServiceBuilder, zenkey::Result<desired::Target>, Arc<Drive>) {
    let worker = imp("worker.v2");
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
    let mut b = ServiceBuilder::new(session, cfg);
    b.implement(worker).unwrap();
    let run = b
        .declare_state_writer(&wi, RUNNING, &Bindings::new())
        .await
        .unwrap();
    run.put_value(&drive::baseline()).await.unwrap();
    let d = Arc::new(Drive::new_v2(run));
    if hold {
        d.hold();
    }
    let target = b
        .desired_target(
            ROLE,
            tasking,
            TASKS,
            ReportAt::new(wi, REPORT),
            Arc::clone(&d) as Arc<dyn desired::Handler>,
            startup,
        )
        .await;
    (b, target, d)
}

/// `lab/worker` on a client of `ep`, started.
async fn worker_up(
    ep: &str,
    bound: bool,
    follow: Follow,
    startup: TargetStartup,
    hold: bool,
) -> WorkerUp {
    worker_up_against(ep, bound, follow, startup, hold, &tasking(0)).await
}

async fn worker_up_against(
    ep: &str,
    bound: bool,
    follow: Follow,
    startup: TargetStartup,
    hold: bool,
    tasking: &Implementation,
) -> WorkerUp {
    let session = client(ep).await;
    let watch = match follow {
        Follow::Watch => Some(client(ep).await),
        Follow::Poll => None,
    };
    worker_on(session, watch, bound, startup, hold, tasking).await
}

/// `lab/worker` on `session`, started, its watch session `watch` if any.
async fn worker_on(
    session: zenoh::Session,
    watch: Option<zenoh::Session>,
    bound: bool,
    startup: TargetStartup,
    hold: bool,
    tasking: &Implementation,
) -> WorkerUp {
    let mut startup = startup;
    startup.watch = watch.clone();
    let (b, target, d) = worker_builder(&session, bound, tasking, startup, hold).await;
    let target = target.unwrap();
    assert_eq!(target.major(), Major::V2);
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

/// The author and the target, the target converged on v1, `s1` its
/// version.
async fn converged_on_v1(
    s: &Recorder,
    author_ep: &str,
    target_ep: &str,
    author: &AuthorAt,
    follow: Follow,
    target: TargetStartup,
) -> (AuthorUp, WorkerUp, Stamp) {
    let a = author_up(author_ep, author).await;
    let w = worker_up(target_ep, true, follow, target, false).await;
    w.seeded().await;
    let s1 = version(a.author.put(WORKER, bytes(&v(1))).await.unwrap());
    s.until("converged on v1", |r| converged_on(r, s1, 1)).await;
    (a, w, s1)
}

// --- the sections -----------------------------------------------------------------

/// §1 Converge after an outage (§2.4, §2.5, §2.6, §2.10, §2.15).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_converge_after_an_outage() {
    let bus = two_routers().await;
    let rec = records();
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
    //    S sees the target's instance token: a snapshot whose entry is the
    //    baseline's report, under tag 12.
    let a = author_up(&bus.ep1, &rec.author(false)).await;
    assert!(a.svc.tokens_held().is_empty(), "tasking.v2 is tokenless");
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
    let at_token = snapshot(&g, RP).await;
    assert_eq!(at_token.tag, Tag::new(model::TAG).unwrap(), "tag 12");
    let (_, baseline) = live_entry(&at_token);
    assert_eq!(baseline, [0, 0, 0], "the baseline's report: 000000");
    assert_eq!(
        model::read(model::TAG, &baseline).unwrap().report,
        Some(Report::baseline())
    );
    let (_, run) = live_entry(&snapshot(&g, RUN).await);
    assert_eq!(
        serde_json::from_slice::<Value>(&run).unwrap(),
        drive::baseline()
    );
    w.seeded().await;

    // 2. v1: a put on D in a mutation's envelope, with no Zenoh timestamp
    //    of the author's; G's GET of D a snapshot whose one live entry
    //    carries that put's run and seq; then a report converged on v1.
    let s1 = version(a.author.put(WORKER, bytes(&v(1))).await.unwrap());
    assert!(is_16_bit(s1), "the author keeps its run's record");
    s.until("converged on v1", |r| converged_on(r, s1, 1)).await;
    let puts = s.on(D, 0);
    assert_eq!(puts.len(), 1);
    assert_eq!(puts[0].inband, Some(s1), "its envelope's run and seq");
    assert_ne!(
        puts[0].stamped_by,
        Some(a.session.zid().to_string()),
        "a Zenoh timestamp on it is R1's, never the author's (core §4.2)"
    );
    let (entry, record) = live_entry(&snapshot(&g, D).await);
    assert_eq!(entry, s1, "the entry's stamp is the put's");
    assert_eq!(serde_json::from_slice::<Value>(&record).unwrap(), v(1));
    assert_eq!(s.running(), Some(v(1)));

    // 3. The cut: v2 put; nothing reaches the target's side.
    bus.link().cut();
    author_seen(&bus.ep2, false).await;
    let mark2 = s2.mark();
    let sv2 = version(a.author.put(WORKER, bytes(&v(2))).await.unwrap());
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        s2.on(RP, mark2).is_empty() && s2.on(RUN, mark2).is_empty(),
        "nothing on Rp or Run during the cut"
    );
    assert!(converged_on(&w.target.report_v2().await.unwrap(), s1, 1));
    let acts = w.target.acts();

    // 4. The heal: the target's seed carries the held stamp s1, the author
    //    answers whole since its last mutation of D is v2's, and the target
    //    acts on v2 once, within 5 s.
    let healed = Instant::now();
    bus.link().heal();
    s.until("converged on v2", |r| converged_on(r, sv2, 2))
        .await;
    let took = healed.elapsed();
    assert!(
        took < Duration::from_secs(5),
        "converged {took:?} after the heal"
    );
    assert!(
        a.author.reads(WORKER).unwrap().contains(&Asked {
            held: Some(s1),
            unchanged: false
        }),
        "the seed after the heal carried s1, and was answered whole: {:?}",
        a.author.reads(WORKER)
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

/// X's put on D with an envelope it writes (§2): `doc` under `stamp`.
async fn put_as(x: &zenoh::Session, stamp: Stamp, doc: &Value) {
    x.put(D, Envelope::mutation(Tag::NONE, stamp, &bytes(doc)).write())
        .encoding(zenoh::bytes::Encoding::ZENOH_BYTES)
        .await
        .unwrap();
}

/// The author's descriptor, as a played author answers it (§13, §16):
/// read from the scenarios' author on `tasking.v2`, started alone on `ep`
/// and gone again before this returns.
async fn author_descriptor(ep: &str) -> zenkey::model::descriptor::Descriptor {
    let rec = records();
    let a = author_of(ep, tasking(0), &rec).await;
    author_seen(ep, true).await;
    let g = client(ep).await;
    let desc = presence::descriptor(&g, &addr(AUTHOR), a.svc.instance(), T)
        .await
        .unwrap()
        .into_descriptor()
        .unwrap();
    a.kill().await;
    author_seen(ep, false).await;
    g.close().await.unwrap();
    desc
}

/// X as a new instance of the author's (§2, §16): the author's descriptor
/// `desc`, its instance `instance` and its run `run`, answered on its
/// instance key by X's queryable. Returns the queryable and the key, where
/// X then declares its instance token.
async fn answer_descriptor(
    x: &zenoh::Session,
    mut desc: zenkey::model::descriptor::Descriptor,
    instance: &str,
    run: Run,
) -> (zenoh::query::Queryable<()>, String) {
    use zenoh::Wait as _;
    desc.instance = instance.to_owned();
    desc.run = Some(run.hex());
    let served = serde_json::to_vec(&desc).unwrap();
    let key = format!("zk2/lab/author/@zk/instance/{instance}");
    let reply_key = key.clone();
    let q = x
        .declare_queryable(key.clone())
        .complete(true)
        .callback(move |q| {
            let _ = q
                .reply(reply_key.clone(), served.clone())
                .encoding(zenoh::bytes::Encoding::APPLICATION_JSON)
                .wait();
        })
        .await
        .unwrap();
    (q, key)
}

/// §2 Never an older version (§2.6, §2.7).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_never_an_older_version() {
    let bus = one_router().await;
    let rec = records();
    let s = record(&bus.ep1).await;
    let (a, w, _) = converged_on_v1(
        &s,
        &bus.ep1,
        &bus.ep2,
        &rec.author(false),
        Follow::Poll,
        TargetStartup::default(),
    )
    .await;
    // Setup: converged on v2, s2 its version, the author's run R at seq n.
    let sv2 = version(a.author.put(WORKER, bytes(&v(2))).await.unwrap());
    s.until("converged on v2", |r| converged_on(r, sv2, 2))
        .await;
    let acts = w.target.acts();
    let x = client(&bus.ep1).await;
    let mark = s.mark();
    let below = Stamp {
        run: sv2.run,
        seq: Seq::new(sv2.seq.get() - 1).unwrap(),
    };

    // 1. v1's content under R at seq n − 1: a lower seq of the held run is
    //    older. 2. v3's content under R at seq n: the same version.
    put_as(&x, below, &v(1)).await;
    put_as(&x, sv2, &v(3)).await;
    eventually("both of X's puts delivered", || async {
        s.on(D, mark).len() >= 2
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(s.on(RP, mark).is_empty(), "nothing put on Rp");
    assert!(s.on(RUN, mark).is_empty(), "nothing put on Run");
    assert_eq!(w.target.acts(), acts);

    // 3. A re-read on a 2-s period: it carries the held stamp s2, and the
    //    author answers with that stamp alone.
    let reads = a.author.reads(WORKER).unwrap().len();
    w.target.set_reread(Some(Duration::from_secs(2))).unwrap();
    eventually("a re-read", || async {
        a.author.reads(WORKER).unwrap().len() > reads
    })
    .await;
    assert_eq!(
        a.last_read(),
        Some(Asked {
            held: Some(sv2),
            unchanged: true
        })
    );
    // A period below the read's timeout, 1 s, set on a running target is
    // refused as it is at the start (§2.5), and the caller is told; the
    // 2-s period it had stands, so the re-reads go on.
    refused_for(
        w.target.set_reread(Some(Duration::from_millis(500))),
        "below its read's timeout",
        "a period set below the timeout",
    );
    let reads = a.author.reads(WORKER).unwrap().len();
    eventually("a re-read on the period it had", || async {
        a.author.reads(WORKER).unwrap().len() > reads
    })
    .await;
    w.target.set_reread(None).unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(s.on(RP, mark).is_empty(), "unchanged: nothing put on Rp");
    assert_eq!(w.target.acts(), acts, "acted on nothing");
    assert!(converged_on(&w.target.report_v2().await.unwrap(), sv2, 2));

    // 4. X, as a new instance of the author, puts v3's content under a
    //    32-bit run of its own, at seq 0, then answers a descriptor stating
    //    that run and declares its instance token. The put waits, since no
    //    descriptor states X's run yet (§2.18); at X's token the target
    //    reads again, another run is admitted, and its document differs,
    //    so the target acts on it.
    let xs = Stamp {
        run: Run::U32(0x0bad_cafe),
        seq: Seq::new(0).unwrap(),
    };
    let g = client(&bus.ep1).await;
    let desc = presence::descriptor(&g, &addr(AUTHOR), a.svc.instance(), T)
        .await
        .unwrap()
        .into_descriptor()
        .unwrap();
    let mark = s.mark();
    put_as(&x, xs, &v(3)).await;
    eventually("X's put delivered", || async { !s.on(D, mark).is_empty() }).await;
    let (_descriptor, key) = answer_descriptor(&x, desc, "00000000000000cc", xs.run).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        s.on(RP, mark).is_empty() && s.on(RUN, mark).is_empty(),
        "the put waits for X's token"
    );
    assert_eq!(w.target.acts(), acts, "acted on nothing before the token");
    let _token = x.liveliness().declare_token(key).await.unwrap();
    s.until("converged on v3, under X's version", |r| {
        converged_on(r, xs, 3)
    })
    .await;
    assert_eq!(w.target.acts(), acts + 1);

    // 5. v1's content under R at seq n − 1 again: R is a run the target
    //    remembers, whichever run it holds.
    let mark = s.mark();
    put_as(&x, below, &v(1)).await;
    eventually("X's put delivered", || async { !s.on(D, mark).is_empty() }).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(s.on(RP, mark).is_empty() && s.on(RUN, mark).is_empty());
    assert_eq!(w.target.acts(), acts + 1, "acted on nothing");
    assert!(converged_on(&w.target.report_v2().await.unwrap(), xs, 3));
    w.svc.close().await.unwrap();
    a.svc.close().await.unwrap();
}

/// §3 A delete reverts, within the window and past it (§2.8).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_a_delete_reverts_within_the_window_and_past_it() {
    let bus = two_routers().await;
    let rec = records();
    let s = record(&bus.ep1).await;
    let g = client(&bus.ep1).await;
    let author = AuthorAt {
        window_s: Some(10),
        ..rec.author(false)
    };
    let (a, w, _) = converged_on_v1(
        &s,
        &bus.ep1,
        &bus.ep2,
        &author,
        Follow::Watch,
        TargetStartup::default(),
    )
    .await;
    // Reverted, the observed version the delete's with no revision; `None`
    // for a revert on a snapshot with no entry.
    let reverted = |r: &Report, del: Option<Stamp>| {
        *r == Report {
            source: Source::Baseline,
            outcome: Outcome::Reverted,
            last_known: false,
            observed: del.map(|version| Observed {
                version,
                revision: None,
            }),
            in_force: None,
            refusal: None,
        }
    };

    // 1. The delete: reverted to the baseline.
    let del = version(a.author.delete(WORKER).await.unwrap());
    s.until("reverted", |r| reverted(r, Some(del))).await;
    assert_eq!(s.running(), Some(drive::baseline()));

    // 2. v2.
    let sv2 = version(a.author.put(WORKER, bytes(&v(2))).await.unwrap());
    s.until("converged on v2", |r| converged_on(r, sv2, 2))
        .await;

    // 3. A delete during a 3 s cut: the seed after the heal carries s2,
    //    and the author answers a snapshot whose one entry is a tombstone
    //    with the delete's stamp.
    bus.link().cut();
    author_seen(&bus.ep2, false).await;
    let del3 = version(a.author.delete(WORKER).await.unwrap());
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(tombstone(&snapshot(&g, D).await), del3);
    bus.link().heal();
    s.until("reverted after the heal", |r| reverted(r, Some(del3)))
        .await;
    assert!(
        a.author.reads(WORKER).unwrap().contains(&Asked {
            held: Some(sv2),
            unchanged: false
        }),
        "the seed carried s2, and was answered whole"
    );
    assert_eq!(s.running(), Some(drive::baseline()));

    // 4. v3, then a delete during a 15 s cut, past the 10 s window: the
    //    author holds no member at D any more, so it answers a snapshot
    //    with no entry under the run it put v3 under. The target holds v3
    //    of that run, at or below the asof: it reverts, with no observed
    //    version. Under desired.v1 this target kept v3 in force.
    let sv3 = version(a.author.put(WORKER, bytes(&v(3))).await.unwrap());
    s.until("converged on v3", |r| converged_on(r, sv3, 3))
        .await;
    bus.link().cut();
    author_seen(&bus.ep2, false).await;
    a.author.delete(WORKER).await.unwrap();
    tokio::time::sleep(Duration::from_secs(15)).await;
    let empty = snapshot(&g, D).await;
    assert!(empty.live.is_empty() && empty.tombs.is_empty(), "{empty:?}");
    assert_eq!(empty.run, sv3.run, "under the run it put v3 under");
    assert!(empty.asof >= sv3.seq);
    let mark = s.mark();
    bus.link().heal();
    s.until("reverted on a snapshot with no entry", |r| {
        reverted(r, None)
    })
    .await;
    assert!(
        a.author.reads(WORKER).unwrap().contains(&Asked {
            held: Some(sv3),
            unchanged: false
        }),
        "the seed carried s3"
    );
    s.until_on(RUN, mark, "the baseline on Run", |r| {
        r.value() == drive::baseline()
    })
    .await;
    w.svc.close().await.unwrap();
    a.svc.close().await.unwrap();
}

/// §4 A refused document keeps the previous in force (§2.4, §2.7, §2.10,
/// §2.11).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_a_refused_document_keeps_the_previous_in_force() {
    let bus = one_router().await;
    let rec = records();
    let s = record(&bus.ep1).await;
    let (mut a, w, s1) = converged_on_v1(
        &s,
        &bus.ep1,
        &bus.ep2,
        &rec.author(false),
        Follow::Poll,
        TargetStartup::default(),
    )
    .await;
    // Refused over v1 in force: `in_force` other, naming s1 with revision 1.
    let refused = |r: &Report, seen: Stamp, rev: u64, code: Code, pointer: u8, detail: u8| {
        *r == Report {
            source: Source::Desired,
            outcome: Outcome::Refused,
            last_known: false,
            observed: observed(seen, Some(rev)),
            in_force: observed(s1, Some(1)),
            refusal: Some(model::Refusal {
                code,
                pointer: Some(Index::At(pointer)),
                detail: Some(Index::At(detail)),
            }),
        }
    };

    // 1. v2 naming the endpoint: refused reach, at /endpoint (3), line 1.
    let mut v2 = v(2);
    v2["endpoint"] = json!("tcp/10.0.0.9:7447");
    let sv2 = version(a.author.put(WORKER, bytes(&v2)).await.unwrap());
    s.until("v2 refused", |r| refused(r, sv2, 2, Code::Reach, 3, 1))
        .await;
    assert_eq!(s.running(), Some(v(1)));

    // 2. v3 at 500 Hz: refused invalid, at /rate_hz (2), line 0.
    let mut v3 = v(3);
    v3["rate_hz"] = json!(500);
    let sv3 = version(a.author.put(WORKER, bytes(&v3)).await.unwrap());
    s.until("v3 refused", |r| refused(r, sv3, 3, Code::Invalid, 2, 0))
        .await;
    assert_eq!(s.running(), Some(v(1)));

    // 3. A re-mint in the author's own session: a new run, v3's content
    //    put again under it. Another run's version of the document the
    //    target last acted on: adopted, never refused again.
    let acts = w.target.acts();
    let seeds = w.target.seeds();
    let mark = s.mark();
    a.svc.new_epoch().await.unwrap();
    eventually("the re-put", || async { !s.on(D, mark).is_empty() }).await;
    let reput = s.on(D, mark)[0].inband.unwrap();
    assert_ne!(reput.run, sv3.run, "the new instance takes a new run");
    s.until("the re-put adopted", |r| {
        refused(r, reput, 3, Code::Invalid, 2, 0)
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
    let sv4 = version(a.author.put(WORKER, bytes(&v(4))).await.unwrap());
    s.until("converged on v4", |r| converged_on(r, sv4, 4))
        .await;

    // 5. The drive fails its next act: v5 at 20 Hz, halt.
    w.drive.fail_next();
    let mut v5 = v(5);
    v5["rate_hz"] = json!(20);
    v5["mode"] = json!("halt");
    let sv5 = version(a.author.put(WORKER, bytes(&v5)).await.unwrap());
    let r = s.until("v5 failed", |r| r.outcome == Outcome::Failed).await;
    assert_eq!(r.source, Source::Desired);
    assert_eq!(r.observed, observed(sv5, Some(5)));
    assert_eq!(r.in_force, None, "in_force none");
    assert_eq!(r.refusal.unwrap().code, Code::Apply);
    let running = s.running().unwrap();
    assert_eq!(
        (running["rate_hz"].clone(), running["mode"].clone()),
        (json!(20), json!("run"))
    );

    // 6. v6 with revision 4294967296: refused whole, invalid, at /revision
    //    (4) with line 2. The report's observed version is s6, and it
    //    carries no revision. Run is unchanged.
    let applies = w.drive.with(|d| d.applies);
    let mut v6 = v(6);
    v6["revision"] = json!(4_294_967_296_u64);
    let sv6 = version(a.author.put(WORKER, bytes(&v6)).await.unwrap());
    let r = s
        .until("v6 refused", |r| r.outcome == Outcome::Refused)
        .await;
    assert_eq!(r.observed, observed(sv6, None), "no revision");
    assert_eq!(
        r.refusal,
        Some(model::Refusal {
            code: Code::Invalid,
            pointer: Some(Index::At(4)),
            detail: Some(Index::At(2)),
        })
    );
    assert_eq!(s.running().unwrap(), running, "Run is unchanged");
    assert_eq!(
        w.drive.with(|d| d.applies),
        applies,
        "refused before the drive"
    );
    w.svc.close().await.unwrap();
    a.svc.close().await.unwrap();
}

/// §5 A restart acts again (§2.7, §2.5).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_a_restart_acts_again() {
    let bus = one_router().await;
    let rec = records();
    let s = record(&bus.ep1).await;
    let g = client(&bus.ep1).await;
    let (a, w, s1) = converged_on_v1(
        &s,
        &bus.ep1,
        &bus.ep2,
        &rec.author(false),
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
    let reads = a.author.reads(WORKER).unwrap().len();
    let w = worker_up(
        &bus.ep2,
        true,
        Follow::Watch,
        TargetStartup::default(),
        true,
    )
    .await;
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
    // G's reply holds the baseline's report under the new instance's run.
    let at_token = snapshot(&g, RP).await;
    let (stamp, baseline) = live_entry(&at_token);
    assert_eq!(baseline, [0, 0, 0]);
    let run = w.svc.minter().run();
    assert_eq!(stamp.run, run, "under the new instance's run");
    let mark = s.mark();
    w.drive.release();
    s.until("converged on v1 again", |r| converged_on(r, s1, 1))
        .await;
    s.until_on(RUN, mark, "v1 put on Run by the new instance", |r| {
        r.value() == v(1) && r.inband.is_some_and(|s| s.run == run)
    })
    .await;
    assert_eq!(w.target.acts(), 1, "it acted on v1 again");
    // The new process seeds with no held stamp.
    assert_eq!(
        a.author.reads(WORKER).unwrap()[reads],
        Asked {
            held: None,
            unchanged: false
        }
    );
    w.svc.close().await.unwrap();
    a.svc.close().await.unwrap();
}

/// §6 The author's restart moves no target to act again (§2.6, §2.11).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s6_the_authors_restart_moves_no_target_to_act_again() {
    let bus = one_router().await;
    let rec = records();
    let s = record(&bus.ep1).await;
    let g = client(&bus.ep1).await;
    let (a, w, s1) = converged_on_v1(
        &s,
        &bus.ep1,
        &bus.ep2,
        &rec.author(true),
        Follow::Poll,
        TargetStartup::default(),
    )
    .await;
    a.author.put("lab/spare", bytes(&v(9))).await.unwrap();
    a.author.delete("lab/spare").await.unwrap();
    // §2.11: the record holds each document and each delete within the
    // window with when it was made, and no stamp.
    let kept: Value = serde_json::from_str(&std::fs::read_to_string(&rec.record).unwrap()).unwrap();
    assert_eq!(kept["profile"], json!("desired.v2"));
    assert_eq!(kept["documents"]["lab/worker"], json!({"document": v(1)}));
    let spare = kept["deletes"]["lab/spare"].as_object().unwrap();
    assert!(
        spare.len() == 1 && spare["at_ms"].is_u64(),
        "a delete with when it was made, and no stamp: {kept}"
    );
    let acts = w.target.acts();

    // 1. The author's process ends, and restarts from its record, its
    //    clock 5 s behind: v1's content put again under a new run, and D2
    //    deleted again under the same run. The clock plays no part.
    a.kill().await;
    let mark = s.mark();
    let behind = AuthorAt {
        offset_ms: -5000,
        ..rec.author(true)
    };
    let a = author_up(&bus.ep1, &behind).await;
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
    let run = a.svc.minter().run();
    assert!(
        matches!(run, Run::U16(_)) && run != s1.run,
        "a new 16-bit run"
    );
    // G's GETs, made as soon as the new instance token was seen: D's
    // snapshot's entry carries the re-put's stamp, D2's is a tombstone.
    let (reput, record) = live_entry(&snapshot(&g, D).await);
    assert_eq!(reput.run, run);
    assert_eq!(serde_json::from_slice::<Value>(&record).unwrap(), v(1));
    assert_eq!(tombstone(&snapshot(&g, D2).await).run, run);
    s.until("the re-put adopted", |r| converged_on(r, reput, 1))
        .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let log: Vec<Rec> = s.all().into_iter().skip(mark).collect();
    assert!(
        log.iter()
            .any(|r| !r.token && r.key == D && !r.delete && r.inband == Some(reput)),
        "S received the re-put under the new run"
    );
    assert!(
        log.iter()
            .any(|r| !r.token && r.key == D2 && r.delete && r.inband.is_some_and(|i| i.run == run)),
        "and the delete of D2 under the same run"
    );
    assert!(s.on(RUN, mark).is_empty(), "Run receives nothing");
    assert_eq!(w.target.acts(), acts, "acted on nothing");

    // 2. v2: acted on.
    let sv2 = version(a.author.put(WORKER, bytes(&v(2))).await.unwrap());
    s.until("converged on v2", |r| converged_on(r, sv2, 2))
        .await;
    assert_eq!(w.target.acts(), acts + 1);

    // 3. The author's process ends, its records removed, its run's with
    //    its documents': a 32-bit run, under which its source yields v2's
    //    content again. Adopted; D2's delete was lost with the record.
    a.kill().await;
    std::fs::remove_file(&rec.record).unwrap();
    std::fs::remove_file(&rec.runs).unwrap();
    let mark = s.mark();
    let bare = AuthorAt {
        runs: None,
        ..rec.author(true)
    };
    let a = author_up(&bus.ep1, &bare).await;
    assert_eq!(a.author.adoption().found, Found::Nothing);
    let s2b = version(a.author.put(WORKER, bytes(&v(2))).await.unwrap());
    assert!(matches!(s2b.run, Run::U32(_)), "a 32-bit run");
    s.until("the new run's v2 adopted", |r| converged_on(r, s2b, 2))
        .await;
    assert!(s.on(RUN, mark).is_empty(), "Run receives nothing");
    assert_eq!(w.target.acts(), acts + 1, "adopted, not acted on");
    let d2 = snapshot(&g, D2).await;
    assert!(
        d2.live.is_empty() && d2.tombs.is_empty(),
        "D2's delete went with the record: a snapshot with no entry"
    );

    // 4. v3: acted on.
    let sv3 = version(a.author.put(WORKER, bytes(&v(3))).await.unwrap());
    s.until("converged on v3", |r| converged_on(r, sv3, 3))
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
                selector: "zk2/lab/author/tasking.v2/state/**".to_owned(),
                implementation: imp("tasking.v2"),
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

/// Converged on `s`, `revision`, read from an archive.
fn last_known_on(r: &Report, s: Stamp, revision: u64) -> bool {
    r.last_known && {
        let mut r = *r;
        r.last_known = false;
        converged_on(&r, s, revision)
    }
}

/// §7 Last-known from an archive, the author absent (§2.9).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_last_known_from_an_archive_the_author_absent() {
    let bus = one_router().await;
    let rec = records();
    let s = record(&bus.ep1).await;
    let (_as, archive) = archive_on(&bus.ep1).await;
    let (a, w, s1) = converged_on_v1(
        &s,
        &bus.ep1,
        &bus.ep2,
        &rec.author(true),
        Follow::Poll,
        with_archive(),
    )
    .await;
    eventually("the archive recorded v1", || async {
        archive.confirmed(D) == Some(true)
    })
    .await;

    // 1. The author closes cleanly; the target restarts. The archive's
    //    record of D keeps v1's envelope, so the target acts on v1 under
    //    s1, last_known true.
    a.svc.close().await.unwrap();
    a.session.close().await.unwrap();
    author_seen(&bus.ep1, false).await;
    w.kill().await;
    let mark = s.mark();
    let w = worker_up(&bus.ep2, true, Follow::Poll, with_archive(), false).await;
    s.until("converged on v1, last-known", |r| last_known_on(r, s1, 1))
        .await;
    let seed = w.target.last_seed().unwrap();
    assert_eq!(seed.answered, 0, "the GET drew no reply");
    assert_eq!(seed.author, desired::AuthorSeen::Absent);
    assert_eq!(seed.archive, Some(true));
    s.until_on(RUN, mark, "v1 put on Run", |r| r.value() == v(1))
        .await;
    let acts = w.target.acts();
    assert_eq!(acts, 1);

    // 2. The author restarts from its record: v1 again under a new run,
    //    adopted, no longer last-known.
    let mark = s.mark();
    let a = author_up(&bus.ep1, &rec.author(true)).await;
    let reput = loop {
        if let Some(r) = s.on(D, mark).first() {
            break r.inband.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_ne!(reput.run, s1.run, "another run than s1's");
    s.until("the re-put adopted", |r| {
        converged_on(r, reput, 1) && !r.last_known
    })
    .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(s.on(RUN, mark).is_empty(), "Run receives nothing");
    assert_eq!(w.target.acts(), acts, "acted on nothing");
    assert_eq!(
        s.on(RP, mark).len(),
        1,
        "the seed's answer is the same version"
    );
    w.svc.close().await.unwrap();
    a.svc.close().await.unwrap();
}

/// §8 The kill switch (§2.12).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s8_the_kill_switch() {
    let bus = one_router().await;
    let rec = records();
    let s = record(&bus.ep1).await;
    let g = client(&bus.ep1).await;
    let a = author_up(&bus.ep1, &rec.author(false)).await;
    a.author.put(WORKER, bytes(&v(1))).await.unwrap();
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
    let (_, rp) = live_entry(&snapshot(&g, RP).await);
    assert_eq!(rp, [0, 0, 0], "Rp holds the baseline's report");
    tokio::time::sleep(Duration::from_secs(3)).await;
    let (_, run) = live_entry(&snapshot(&g, RUN).await);
    assert_eq!(
        serde_json::from_slice::<Value>(&run).unwrap(),
        drive::baseline()
    );
    // The author receives no GET of D (G's own aside: it made none of D).
    assert!(a.author.reads(WORKER).unwrap().is_empty(), "no GET of D");
    assert_eq!(a.author.gets(WORKER).unwrap(), 0);
    assert_eq!((w.target.seeds(), w.target.acts()), (0, 0));
    assert_eq!((w.target.key(), w.target.reads()), (None, 0));
    assert_eq!(s.on(RUN, 0).len(), 1, "Run: the baseline only");
    assert_eq!(s.on(RP, 0).len(), 1, "Rp: the baseline report only");
    w.svc.close().await.unwrap();
    a.svc.close().await.unwrap();
}

/// §10 Last-known confirmed by its author; a present author's silence
/// (§2.5, §2.9).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s10_last_known_confirmed_by_its_author_and_a_present_authors_silence() {
    let bus = two_routers().await;
    let rec = records();
    let s = record(&bus.ep1).await;
    let s2 = record(&bus.ep2).await;
    let (_as, archive) = archive_on(&bus.ep2).await;
    author_seen(&bus.ep1, false).await;
    let a = author_up(&bus.ep1, &rec.author(false)).await;
    author_seen(&bus.ep2, true).await;
    let w = worker_up(&bus.ep2, true, Follow::Watch, with_archive(), false).await;
    w.seeded().await;
    let s1 = version(a.author.put(WORKER, bytes(&v(1))).await.unwrap());
    s.until("converged on v1", |r| converged_on(r, s1, 1)).await;
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
    s2.until("converged on v1, last-known", |r| last_known_on(r, s1, 1))
        .await;
    assert_eq!(
        w.target.last_seed().unwrap().author,
        desired::AuthorSeen::Absent
    );
    assert!(s2.on(RUN, mark).iter().any(|r| r.value() == v(1)));
    let acts = w.target.acts();

    // 2. The heal: the target's seed carries the held stamp s1, the
    //    archive's record's. The author, the same instance throughout,
    //    answers with that stamp alone: its own word of the version,
    //    confirmed. Nothing acted on, the report put again, last_known
    //    false.
    let mark = s2.mark();
    let reads = a.author.reads(WORKER).unwrap().len();
    bus.link().heal();
    s2.until("confirmed", |r| converged_on(r, s1, 1) && !r.last_known)
        .await;
    assert!(
        a.author.reads(WORKER).unwrap()[reads..].contains(&Asked {
            held: Some(s1),
            unchanged: true
        }),
        "answered with the held stamp alone: {:?}",
        a.author.reads(WORKER)
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(s2.on(RUN, mark).is_empty(), "Run receives nothing");
    assert_eq!(w.target.acts(), acts, "acted on nothing");

    // 3. The cut again: the author puts v2, which neither the target nor
    //    the archive receives. X holds an instance token at the author's
    //    address on R2, and no queryable; the target's process ends, and
    //    restarts.
    bus.link().cut();
    author_seen(&bus.ep2, false).await;
    let sv2 = version(a.author.put(WORKER, bytes(&v(2))).await.unwrap());
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
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        w.target.last_seed(),
        Some(desired::Seeded {
            answered: 0,
            author: desired::AuthorSeen::Present,
            archive: None,
            // X answers no descriptor: the deployment's word stands
            // (desired.v2 §2.18).
            revision: desired::AuthorRevision::Word,
        }),
        "the reference reads no archive while its author is present"
    );
    assert_eq!(w.target.acts(), 0, "acted on nothing");
    assert_eq!(
        s2.on(RP, mark).last().map(Rec::report),
        Some(Report::baseline())
    );
    assert!(
        s2.on(RUN, mark)
            .iter()
            .all(|r| r.value() == drive::baseline()),
        "Run holds the baseline"
    );

    // 4. The heal: the author's own token appears on R2, and v2 converges;
    //    then X withdraws its token, which moves nothing.
    bus.link().heal();
    s2.until("converged on v2", |r| {
        converged_on(r, sv2, 2) && !r.last_known
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

// --- §11: one document type per role --------------------------------------------

/// §11's tasking.v2, two documents: a second desired resource,
/// `plans/{system}/{service}`, declared as `tasks/…` is.
fn tasking_two() -> Implementation {
    copy_of("tasking.v2", |t| {
        t + "\n[resources.\"plans/{system}/{service}\"]\nkind        = \"state\"\n\
             type        = \"json:Task\"\n\
             params      = { system = \"string\", service = \"string\" }\n\
             cardinality = 16\n\
             annotations = { \"desired.target_param\" = [\"system\", \"service\"], \
             \"freshness.ttl_s\" = 0 }\n"
    })
}

/// worker.v2 with its requirement naming both of §11's documents.
fn worker_both() -> Implementation {
    copy_of("worker.v2", |t| {
        let both = t.replace(
            "resources = [\"tasks/{system}/{service}\"]\n",
            "resources = [\"tasks/{system}/{service}\", \"plans/{system}/{service}\"]\n",
        );
        assert_ne!(both, t);
        both
    })
}

/// A start's refusal by its reason: `not started: …` (core §8.2 step 2).
fn refused_for<T>(r: zenkey::Result<T>, reason: &str, what: &str) {
    match r {
        Err(e @ zenkey::Error::NotExposed(_)) => {
            let why = e.to_string();
            assert!(why.starts_with("not started: "), "{what}: {why}");
            assert!(why.contains(reason), "{what}: {why}");
        }
        Err(other) => panic!("{what}: not the start's refusal: {other}"),
        Ok(_) => panic!("{what}: refused"),
    }
}

/// S's instance tokens of `lab/worker`, put (appeared).
fn worker_tokens(s: &Recorder) -> usize {
    s.all()
        .iter()
        .filter(|r| r.token && !r.delete && r.key.starts_with("zk2/lab/worker/@zk/instance/"))
        .count()
}

/// `lab/worker`'s builder given `worker` and the role against `tasking`.
async fn given_the_role(
    session: &zenoh::Session,
    worker: Implementation,
    tasking: &Implementation,
    startup: TargetStartup,
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
    let run = b
        .declare_state_writer(&wi, RUNNING, &Bindings::new())
        .await
        .unwrap();
    let r = b
        .desired_target(
            ROLE,
            tasking,
            TASKS,
            ReportAt::new(wi, REPORT),
            Arc::new(Drive::new_v2(run)) as Arc<dyn desired::Handler>,
            startup,
        )
        .await;
    (b, r)
}

/// §11 A role on two documents refuses the start (§2.1, §2.3), with
/// `tasking.v2` and `worker.v2`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s11_a_role_on_two_documents_refuses_the_start() {
    let bus = one_router().await;
    let s = record(&bus.ep1).await;
    let g = client(&bus.ep1).await;
    let tasking = tasking_two();
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 1. worker.v2, both: the role refused, and the start.
    let session = client(&bus.ep2).await;
    let (b, given) =
        given_the_role(&session, worker_both(), &tasking, TargetStartup::default()).await;
    refused_for(given, "(two_documents)", "the role");
    refused_for(b.start().await, "(two_documents)", "the start");
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(
        worker_tokens(&s),
        0,
        "no instance token of lab/worker in 5 s"
    );
    assert!(s.on(RP, 0).is_empty(), "no report on Rp");
    assert!(
        get_replies(&g, RP).await.is_empty(),
        "nothing answers on Rp"
    );

    // 2. The control: the scenarios' worker.v2, naming one of the two.
    let session = client(&bus.ep2).await;
    let (b, given) = given_the_role(
        &session,
        imp("worker.v2"),
        &tasking,
        TargetStartup::default(),
    )
    .await;
    let _target = given.expect("a role on one document type");
    let svc = b.start().await.expect("the control starts");
    eventually("the control's instance token at S", || async {
        worker_tokens(&s) == 1
    })
    .await;
    s.until("its baseline report (§2.10)", |r| *r == Report::baseline())
        .await;
    svc.close().await.unwrap();
}

// --- §13: a played author ---------------------------------------------------------

/// What P answers a GET with (§13).
#[derive(Debug, Clone, Copy)]
enum Answer {
    /// As step 1 says: the held stamp the GET carried, and for a GET that
    /// carried none the snapshot it holds.
    Echo,
    /// The snapshot it holds, whatever the GET carried.
    Whole,
    /// This held stamp.
    Held(Stamp),
    /// Nothing: left unanswered past its timeout.
    Silent,
}

/// One GET of D, as P received it.
#[derive(Debug, Clone)]
struct PlayedGet {
    at: Instant,
    /// Its payload and its `Encoding`, `None` for a GET with no payload.
    carried: Option<(Vec<u8>, String)>,
    /// When P answered it.
    answered: Option<Instant>,
}

struct PlayedState {
    log: Vec<PlayedGet>,
    next: std::collections::VecDeque<Answer>,
    page: Vec<u8>,
    delay: Duration,
}

/// The instance §13's played author holds its token at.
const PLAYED: &str = "00000000000000aa";

/// §13's played author: a session of the test's own holding an instance
/// token at `lab/author` and a queryable on D. It puts nothing, and answers
/// each GET as the step says.
struct Played {
    _token: zenoh::liveliness::LivelinessToken,
    _queryable: zenoh::query::Queryable<()>,
    state: Arc<Mutex<PlayedState>>,
}

/// D's snapshot as an author answers it: one page, its one entry `doc` at
/// `at`, as of that seq.
fn page_of(at: Stamp, doc: &Value) -> Vec<u8> {
    use zenkey::model::collection::{Confirmation, Entry, Snapshot, Time};
    let snapshot = Snapshot {
        tag: Tag::NONE,
        run: at.run,
        asof: at.seq,
        time: Time::Unknown,
        coll: 0,
        live: vec![Entry {
            id: String::new(),
            run: None,
            seq: at.seq,
            confirmation: Confirmation::None,
            record: bytes(doc),
        }],
        tombs: Vec::new(),
    };
    snapshot.paginate(1024).expect("a page")[0]
        .write()
        .expect("a page writes")
}

/// The held stamp `at` as a GET carries it (core §7.5): its bytes, under
/// `zenoh/bytes`.
fn held(at: Stamp) -> Option<(Vec<u8>, String)> {
    Some((
        zenkey::model::wire::HeldStamp::new(at).write(),
        "zenoh/bytes".to_owned(),
    ))
}

impl Played {
    async fn new(s: &zenoh::Session, page: Vec<u8>) -> Self {
        let state = Arc::new(Mutex::new(PlayedState {
            log: Vec::new(),
            next: std::collections::VecDeque::new(),
            page,
            delay: Duration::ZERO,
        }));
        let (st, rt) = (Arc::clone(&state), tokio::runtime::Handle::current());
        let queryable = s
            .declare_queryable(D)
            .complete(true)
            .callback(move |q| {
                let carried = q
                    .payload()
                    .map(|b| b.to_bytes().to_vec())
                    .filter(|b| !b.is_empty())
                    .map(|b| (b, q.encoding().map(ToString::to_string).unwrap_or_default()));
                let (i, answer, page, delay) = {
                    let mut st = st.lock().unwrap();
                    st.log.push(PlayedGet {
                        at: Instant::now(),
                        carried: carried.clone(),
                        answered: None,
                    });
                    (
                        st.log.len() - 1,
                        st.next.pop_front().unwrap_or(Answer::Echo),
                        st.page.clone(),
                        st.delay,
                    )
                };
                let st = Arc::clone(&st);
                rt.spawn(async move {
                    tokio::time::sleep(delay).await;
                    let payload = match answer {
                        // The query kept past its timeout: dropped, zenoh
                        // would end the GET at once with no reply.
                        Answer::Silent => {
                            tokio::time::sleep(Duration::from_secs(8)).await;
                            return;
                        }
                        Answer::Echo => carried.map_or(page, |(b, _)| b),
                        Answer::Whole => page,
                        Answer::Held(at) => zenkey::model::wire::HeldStamp::new(at).write(),
                    };
                    q.reply(q.key_expr().clone(), payload)
                        .encoding(zenoh::bytes::Encoding::ZENOH_BYTES)
                        .await
                        .expect("a reply");
                    st.lock().unwrap().log[i].answered = Some(Instant::now());
                });
            })
            .await
            .unwrap();
        let token = s
            .liveliness()
            .declare_token(format!("zk2/lab/author/@zk/instance/{PLAYED}"))
            .await
            .unwrap();
        Self {
            _token: token,
            _queryable: queryable,
            state,
        }
    }

    fn gets(&self) -> Vec<PlayedGet> {
        self.state.lock().unwrap().log.clone()
    }

    /// The next GET not yet scripted is answered so.
    fn next(&self, answer: Answer) {
        self.state.lock().unwrap().next.push_back(answer);
    }

    fn holds(&self, page: Vec<u8>) {
        self.state.lock().unwrap().page = page;
    }

    fn delays(&self, delay: Duration) {
        self.state.lock().unwrap().delay = delay;
    }

    /// Waits for the GET at `index`, answered unless `silent`.
    async fn got(&self, index: usize, silent: bool) -> PlayedGet {
        eventually("a GET of D at P", || async {
            self.gets()
                .get(index)
                .is_some_and(|g| silent || g.answered.is_some())
        })
        .await;
        self.gets()[index].clone()
    }
}

/// §13 A conditional re-read, and what a read waits (§2.5). The figures the
/// scenario fixes: the period, 2 s; the read's timeout, 1 s; the face's
/// reply timeout, 6000 ms, P's delay, 4 s, and the period across it, 8 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s13_a_conditional_re_read_and_what_a_read_waits() {
    const R: Run = Run::U16(0x0042);
    let at = |seq: u32| Stamp {
        run: R,
        seq: Seq::new(seq).unwrap(),
    };
    let period = |p: u64| TargetStartup::default().with_reread(Duration::from_secs(p));
    let bus = one_router().await;
    let s = record(&bus.ep1).await;
    let ps = client(&bus.ep1).await;
    // Setup: P plays the author: it answers a descriptor stating R and
    // declares its instance token (§2.18); the seed draws v1's snapshot,
    // its entry at (R, 7); the target's period is 2 s.
    let (_descriptor, _) =
        answer_descriptor(&ps, author_descriptor(&bus.ep1).await, PLAYED, R).await;
    let p = Played::new(&ps, page_of(at(7), &v(1))).await;
    author_seen(&bus.ep2, true).await;
    let w = worker_up(&bus.ep2, true, Follow::Poll, period(2), false).await;
    s.until("converged on v1", |r| converged_on(r, at(7), 1))
        .await;
    assert_eq!(p.gets()[0].carried, None, "the seed carries no held stamp");
    let acts = w.target.acts();

    // 1. Unchanged: each re-read carries the 6 bytes 0xe0, R and seq 7,
    //    under zenoh/bytes, and P answers with them. Nothing is acted on.
    let (mark, n) = (s.mark(), p.gets().len());
    tokio::time::sleep(Duration::from_secs(5)).await;
    let rereads = p.gets().split_off(n);
    assert!(rereads.len() >= 2, "{rereads:?}");
    for g in &rereads {
        assert_eq!(
            g.carried,
            Some((vec![0xe0, 0x00, 0x42, 0, 0, 7], "zenoh/bytes".to_owned())),
            "an unchanged reply is these 6 bytes, where the document is {}",
            bytes(&v(1)).len()
        );
    }
    assert!(s.on(RP, mark).is_empty() && s.on(RUN, mark).is_empty());
    assert_eq!(w.target.acts(), acts);

    // 2. Changed: the next re-read is answered with v2's snapshot, its
    //    entry at (R, 9), a higher seq of the held run. The one after
    //    carries (R, 9).
    p.holds(page_of(at(9), &v(2)));
    p.next(Answer::Whole);
    s.until("converged on v2", |r| converged_on(r, at(9), 2))
        .await;
    assert_eq!(w.target.acts(), acts + 1);
    let n = p.gets().len();
    assert_eq!(p.got(n, false).await.carried, held(at(9)));

    // 3. Not the one it sent: the held stamp (R, 8) is no answer. Its next
    //    re-read carries (R, 9) again.
    let (mark, n) = (s.mark(), p.gets().len());
    p.next(Answer::Held(at(8)));
    assert_eq!(p.got(n, false).await.carried, held(at(9)));
    assert_eq!(p.got(n + 1, false).await.carried, held(at(9)));
    assert!(s.on(RP, mark).is_empty() && s.on(RUN, mark).is_empty());
    assert_eq!(w.target.acts(), acts + 1, "acted on nothing");
    assert!(converged_on(&w.target.report_v2().await.unwrap(), at(9), 2));

    // 4. An author that reads no payload answers v2's snapshot again: the
    //    same version, nothing acted on.
    let n = p.gets().len();
    p.next(Answer::Whole);
    p.got(n, false).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(s.on(RP, mark).is_empty() && s.on(RUN, mark).is_empty());
    assert_eq!(w.target.acts(), acts + 1);

    // 5. One at a time: P leaves the next re-read unanswered, and 0.2 s
    //    after it arrives declares a second instance token. No second GET
    //    while the first is unanswered; when it ends at its timeout, 1 s,
    //    the target makes one GET, the seed that fell due.
    let n = p.gets().len();
    p.next(Answer::Silent);
    let first = p.got(n, true).await.at;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let second = ps
        .liveliness()
        .declare_token("zk2/lab/author/@zk/instance/00000000000000bb")
        .await
        .unwrap();
    let seed = p.got(n + 1, false).await;
    let waited = seed.at - first;
    assert!(
        waited >= Duration::from_millis(950),
        "no second GET while the first was unanswered: {waited:?}"
    );
    assert!(
        waited < Duration::from_millis(1900),
        "the seed that fell due, made when the first read ended, not the period's: {waited:?}"
    );
    assert_eq!(seed.carried, held(at(9)));

    // 6. Across a face whose policy states reply_timeout_ms = 6000, with a
    //    period of 8 s, P answering each GET 4 s after it arrives: the seed
    //    draws v2's snapshot 4 s after it was asked. At most 3 GETs in the
    //    20 s, and none while another is unanswered.
    w.svc.close().await.unwrap();
    w.session.close().await.unwrap();
    drop(second);
    let far = zenkey::model::link::FarSide {
        face: "rf".into(),
        reply_timeout_ms: 6000,
        arq: true,
        transport: zenkey::model::link::Binding::Zenoh1_10_1.defaults(),
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    p.delays(Duration::from_secs(4));
    let (mark, n) = (s.mark(), p.gets().len());
    let started = Instant::now();
    let w = worker_up(&bus.ep2, true, Follow::Poll, period(8).across(&far), false).await;
    s.until_on(RP, mark, "converged on v2 across the face", |r| {
        converged_on(&r.report(), at(9), 2)
    })
    .await;
    assert!(
        started.elapsed() >= Duration::from_secs(4),
        "a target that waited 1 s would have drawn nothing"
    );
    tokio::time::sleep(Duration::from_secs(20).saturating_sub(started.elapsed())).await;
    let gets = p.gets().split_off(n);
    assert_eq!(gets[0].carried, None, "the seed");
    assert!(gets.len() <= 3, "at most 3 GETs in the 20 s: {gets:?}");
    for pair in gets.windows(2) {
        assert_eq!(pair[1].carried, held(at(9)), "each re-read conditional");
        assert!(
            pair[0].answered.is_some_and(|a| pair[1].at >= a),
            "none while another is unanswered: {gets:?}"
        );
    }
    w.svc.close().await.unwrap();
    w.session.close().await.unwrap();

    // 7. A period below the timeout: across the same face with a period of
    //    2 s, the target refuses to start. S sees no instance token of it.
    let mark = s.mark();
    let session = client(&bus.ep2).await;
    let (b, given, _) =
        worker_builder(&session, true, &tasking(0), period(2).across(&far), false).await;
    refused_for(given, "below its read's timeout", "the role");
    refused_for(b.start().await, "below its read's timeout", "the start");
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(
        !s.all()
            .iter()
            .skip(mark)
            .any(|r| r.token && !r.delete && r.key.starts_with("zk2/lab/worker/@zk/instance/")),
        "no instance token of lab/worker"
    );
}

// --- §14: the watch --------------------------------------------------------------

/// What §14's author is told, in a process of its own.
enum Cmd {
    Put(String, Vec<u8>, tokio::sync::oneshot::Sender<Stamp>),
    Reads(tokio::sync::oneshot::Sender<Vec<Asked>>),
    Stop(Duration, tokio::sync::oneshot::Sender<()>),
    Kill(tokio::sync::oneshot::Sender<()>),
}

/// §14's author, on a runtime of its own with one worker thread, so that
/// the test can stop it as a stopped process is: [`Remote::stop`] holds
/// that thread, and with it every task of the service, its heartbeats
/// among them. Its zenoh session's own threads keep its link up.
struct Remote {
    tx: tokio::sync::mpsc::UnboundedSender<Cmd>,
    adoption: desired::Adoption,
}

impl Remote {
    async fn start(ep: &str, at: &AuthorAt) -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let (up, started) = tokio::sync::oneshot::channel();
        let (ep, at) = (ep.to_owned(), at.clone());
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let a = author_up(&ep, &at).await;
                let _ = up.send(a.author.adoption());
                while let Some(cmd) = rx.recv().await {
                    match cmd {
                        Cmd::Put(target, doc, back) => {
                            let w = a.author.put(&target, doc).await.unwrap();
                            let _ = back.send(version(w));
                        }
                        Cmd::Reads(back) => {
                            let _ = back.send(a.author.reads(WORKER).unwrap());
                        }
                        Cmd::Stop(d, back) => {
                            tokio::spawn(async move { std::thread::sleep(d) })
                                .await
                                .unwrap();
                            let _ = back.send(());
                        }
                        Cmd::Kill(back) => {
                            a.kill().await;
                            let _ = back.send(());
                            return;
                        }
                    }
                }
            });
        });
        Self {
            tx,
            adoption: started.await.expect("the author started"),
        }
    }

    async fn put(&self, target: &str, doc: &Value) -> Stamp {
        let (back, got) = tokio::sync::oneshot::channel();
        let _ = self.tx.send(Cmd::Put(target.to_owned(), bytes(doc), back));
        got.await.unwrap()
    }

    /// The GETs of the target's slice this process of the author answered.
    async fn reads(&self) -> Vec<Asked> {
        let (back, got) = tokio::sync::oneshot::channel();
        let _ = self.tx.send(Cmd::Reads(back));
        got.await.unwrap()
    }

    /// Stops the author's process for `d`, then lets it run again.
    fn stop(&self, d: Duration) -> tokio::sync::oneshot::Receiver<()> {
        let (back, done) = tokio::sync::oneshot::channel();
        let _ = self.tx.send(Cmd::Stop(d, back));
        done
    }

    /// The process ends without closing.
    async fn kill(self) {
        let (back, done) = tokio::sync::oneshot::channel();
        let _ = self.tx.send(Cmd::Kill(back));
        done.await.unwrap();
    }
}

/// A client of `ep` with the zid `zid`, retrying fast after losing it.
async fn client_as(ep: &str, zid: &str) -> zenoh::Session {
    let mut c = zenoh::Config::default();
    for (k, v) in [
        ("scouting/multicast/enabled", "false".to_owned()),
        ("scouting/gossip/enabled", "false".to_owned()),
        ("mode", "\"client\"".to_owned()),
        ("id", format!("\"{zid}\"")),
        ("connect/endpoints", format!("[\"{ep}\"]")),
        (
            "connect/retry",
            "{ period_init_ms: 100, period_max_ms: 500, period_increase_factor: 1.5 }".to_owned(),
        ),
    ] {
        c.insert_json5(k, &v).unwrap();
    }
    zenoh::open(c).await.expect("the target's session")
}

/// How many heartbeats the target's watch read.
async fn beats(w: &WorkerUp) -> u64 {
    w.target.watching().await.unwrap().read
}

/// Waits for the target's next heartbeat, so that what the test does next
/// falls early in a heartbeat's interval.
async fn after_a_heartbeat(w: &WorkerUp) {
    let n = beats(w).await;
    eventually("a heartbeat at the target", || async { beats(w).await > n }).await;
}

/// §14 The watch (§2.5), over "tasking.v2, watched": a horizon of 4 s, the
/// figure the scenario fixes, so a heartbeat at most every 2 s. The face
/// is R2's, which denies the target's session every key under `@zk`
/// (`link.v1` §2.6's deny), so no presence crosses and the target never
/// sees its author's instance token; the test cuts and heals the link
/// behind it. The reference moves what it expects at a version heard live.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s14_the_watch() {
    use zenkey::model::link;
    const WORKER_ZID: &str = "e1e1e1e1e1e1e1e1";
    let rec = records();
    let (_r1, ep1) = router(None).await;
    let face = link_to(&ep1).await;
    let acl = json!({
        "enabled": true,
        "default_permission": "allow",
        "rules": [{
            "id": "zk", "permission": "deny", "flows": link::FLOWS,
            "messages": link::MESSAGES, "key_exprs": [link::ZK_DENY],
        }],
        "subjects": [{"id": "far", "zids": [WORKER_ZID]}],
        "policies": [{"rules": ["zk"], "subjects": ["far"]}],
    })
    .to_string();
    let (_r2, ep2) = common::router_via_with(&face, &[("access_control", &acl)]).await;
    let s = record(&ep1).await;
    let watched = AuthorAt {
        ttl_s: 4,
        ..rec.author(true)
    };
    let a = Remote::start(&ep1, &watched).await;
    let s1 = a.put(WORKER, &v(1)).await;
    let asked = |held: Stamp, unchanged: bool| Asked {
        held: Some(held),
        unchanged,
    };

    // 1. The target starts, with no period: it watches. It seeds and
    //    converges on v1. The first heartbeat it reads makes it read once,
    //    since it expects none: that read carries s1 and is answered with
    //    it alone. No further GET in the 10 s, though 4 heartbeats or more
    //    crossed.
    let session = client_as(&ep2, WORKER_ZID).await;
    // The deployment's word that the target is across a face (§2.18): no
    // descriptor reaches it, so a read drawing none takes the word.
    let across = TargetStartup {
        across_face: true,
        ..TargetStartup::default()
    };
    let w = worker_on(session, None, true, across, false, &tasking(4)).await;
    s.until("converged on v1", |r| converged_on(r, s1, 1)).await;
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(
        a.reads().await,
        [
            Asked {
                held: None,
                unchanged: false
            },
            asked(s1, true)
        ]
    );
    let watching = w.target.watching().await.unwrap();
    assert!(watching.watches && !watching.silent, "{watching:?}");
    assert!(
        watching.read >= 4 && watching.expects.is_some(),
        "{watching:?}"
    );
    assert_eq!(
        w.target.last_seed().unwrap().author,
        desired::AuthorSeen::NotRead
    );

    // 2. A document on D2: the next heartbeat's digest differs, since
    //    another member changed. One conditional read, answered unchanged.
    let mark = s.mark();
    after_a_heartbeat(&w).await;
    a.put("lab/spare", &v(9)).await;
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(a.reads().await[2..], [asked(s1, true)]);
    assert!(s.on(RP, mark).is_empty(), "Rp receives nothing");

    // 3. v2 on D: the target hears it live, and converges on it. It moves
    //    what it expects, so it makes no GET.
    after_a_heartbeat(&w).await;
    let s2 = a.put(WORKER, &v(2)).await;
    s.until("converged on v2", |r| converged_on(r, s2, 2)).await;
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(a.reads().await.len(), 3, "its own change costs no GET");

    // 4. The face cut, v3 put, the face healed 5 s later: the first
    //    heartbeat after the heal follows a gap in the seqs. The target
    //    reads, with s2; the author answers whole, and the target acts on
    //    v3. The next heartbeat makes it read once more, answered unchanged.
    face.cut();
    let s3 = a.put(WORKER, &v(3)).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    face.heal();
    s.until("converged on v3", |r| converged_on(r, s3, 3)).await;
    tokio::time::sleep(Duration::from_secs(8)).await;
    assert_eq!(a.reads().await[3..], [asked(s2, false), asked(s3, true)]);

    // 5. The author's process ends without closing, and restarts from its
    //    record: v3's content under a new run, heard and adopted. The
    //    first heartbeat of the new instance is of another run: the target
    //    reads, and is answered unchanged. Run receives nothing.
    let acts = w.target.acts();
    a.kill().await;
    let mark = s.mark();
    let a = Remote::start(&ep1, &watched).await;
    assert_eq!(a.adoption.reput, ["lab/spare", "lab/worker"]);
    let reput = loop {
        if let Some(r) = s.on(D, mark).first() {
            break r.inband.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_ne!(reput.run, s3.run, "a new run");
    s.until("the re-put adopted", |r| converged_on(r, reput, 3))
        .await;
    tokio::time::sleep(Duration::from_secs(8)).await;
    assert_eq!(a.reads().await, [asked(reput, true)]);
    assert!(s.on(RUN, mark).is_empty(), "Run receives nothing");
    assert_eq!(w.target.acts(), acts);

    // 6. The author's process stopped for 6 s, past the horizon: no
    //    heartbeat crosses, and the document passes its horizon. The target
    //    keeps v3 in force and makes no GET. When the author runs again,
    //    its next heartbeat is in sequence and unmoved: no GET either.
    let mark = s.mark();
    let reads = w.target.reads();
    let stopped = a.stop(Duration::from_secs(6));
    tokio::time::sleep(Duration::from_secs(1)).await;
    let during = beats(&w).await;
    tokio::time::sleep(Duration::from_millis(4500)).await;
    assert_eq!(beats(&w).await, during, "no heartbeat crosses meanwhile");
    assert!(
        w.target.watching().await.unwrap().silent,
        "past its horizon"
    );
    stopped.await.unwrap();
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(beats(&w).await > during, "the author runs again");
    assert_eq!(w.target.reads(), reads, "the target made no GET");
    assert_eq!(a.reads().await.len(), 1);
    assert!(s.on(RP, mark).is_empty() && s.on(RUN, mark).is_empty());
    assert!(converged_on(&w.target.report_v2().await.unwrap(), reput, 3));
    w.svc.close().await.unwrap();
    a.kill().await;
}

/// §2.5, "The watch", by its other form: the author confirms "tasking.v2,
/// watched" by an owner heartbeat (`freshness.v2` §2.4), never by a
/// heartbeat of the interface's own, and the target, given the
/// deployment's catalog, reads the author's group as it reads a heartbeat:
/// one read at its first, answered unchanged, and one more when another
/// member's change moves the digest.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_watch_reads_an_owner_heartbeats_group() {
    use zenkey::model::catalog::Catalog;
    let bus = one_router().await;
    let rec = records();
    let s = record(&bus.ep1).await;
    let watched = tasking(4);
    let iface = watched.iface().clone();
    let catalog = || {
        let mut c = Catalog::new(0).unwrap();
        c.push(iface.clone(), watched.fingerprint().clone())
            .unwrap();
        c
    };

    let session = client(&bus.ep1).await;
    let cfg = config(AUTHOR).tokenless(iface.clone()).owner_heartbeat();
    let mut b = ServiceBuilder::new(&session, cfg);
    b.run_store(Arc::new(zenkey::run::FileRunStore::new(&rec.runs)))
        .unwrap();
    b.catalog(catalog()).unwrap();
    b.implement(watched.clone()).unwrap();
    let author = b
        .desired_author(&iface, TASKS, AuthorStartup::default())
        .await
        .unwrap();
    let asvc = b.start().await.unwrap();
    assert!(
        !asvc.owner_heartbeat_cover().is_empty(),
        "tasking.v2 is confirmed by the owner heartbeat"
    );
    let s1 = version(author.put(WORKER, bytes(&v(1))).await.unwrap());

    let ws = client(&bus.ep2).await;
    let worker = imp("worker.v2");
    let wi = worker.iface().clone();
    let mut cfg = config(WORKER).bind(ROLE, &[AUTHOR]);
    cfg.bindings.get_mut(ROLE).unwrap().params = [
        ("system".to_owned(), "self.system".to_owned()),
        ("service".to_owned(), "self.service".to_owned()),
    ]
    .into();
    let mut wb = ServiceBuilder::new(&ws, cfg);
    wb.catalog(catalog()).unwrap();
    wb.implement(worker).unwrap();
    let run = wb
        .declare_state_writer(&wi, RUNNING, &Bindings::new())
        .await
        .unwrap();
    run.put_value(&drive::baseline()).await.unwrap();
    let target = wb
        .desired_target(
            ROLE,
            &watched,
            TASKS,
            ReportAt::new(wi, REPORT),
            Arc::new(Drive::new_v2(run)) as Arc<dyn desired::Handler>,
            TargetStartup::default(),
        )
        .await
        .unwrap();
    let wsvc = wb.start().await.unwrap();
    s.until("converged on v1", |r| converged_on(r, s1, 1)).await;
    let unchanged = Asked {
        held: Some(s1),
        unchanged: true,
    };
    eventually("the watch's first read, at an owner heartbeat", || async {
        author.reads(WORKER).unwrap().last() == Some(&unchanged)
    })
    .await;
    eventually("an unmoved group", || async {
        let w = target.watching().await.unwrap();
        w.expects.is_some() && w.read >= 3 && w.unread == 0
    })
    .await;
    assert_eq!(author.reads(WORKER).unwrap().len(), 2, "the seed, and one");

    author.put("lab/spare", bytes(&v(9))).await.unwrap();
    eventually("one read more, the digest moved", || async {
        author.reads(WORKER).unwrap().len() == 3
    })
    .await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(author.reads(WORKER).unwrap()[2..], [unchanged]);
    wsvc.close().await.unwrap();
    asvc.close().await.unwrap();
}

/// §15 A delete past the window, then a restart (§2.8).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s15_a_delete_past_the_window_then_a_restart() {
    let bus = two_routers().await;
    let rec = records();
    let s = record(&bus.ep1).await;
    let g = client(&bus.ep1).await;
    let author = AuthorAt {
        window_s: Some(10),
        ..rec.author(true)
    };
    let (a, w, s1) = converged_on_v1(
        &s,
        &bus.ep1,
        &bus.ep2,
        &author,
        Follow::Watch,
        TargetStartup::default(),
    )
    .await;
    let acts = w.target.acts();

    // 1. The link cut, D deleted, and 15 s later, past the window, the
    //    author's process ends and restarts from its record, which has
    //    dropped the delete. The heal: the target seeds with the held
    //    stamp s1; the new instance holds no member at D, so it answers a
    //    snapshot with no entry under its new run. No verdict: v1 stays in
    //    force. This is the residual §2.8 leaves.
    bus.link().cut();
    author_seen(&bus.ep2, false).await;
    a.author.delete(WORKER).await.unwrap();
    tokio::time::sleep(Duration::from_secs(15)).await;
    a.kill().await;
    let a = author_up(&bus.ep1, &author).await;
    let found = a.author.adoption();
    assert_eq!(found.found, Found::Record);
    assert_eq!(found.expired, ["lab/worker"], "the delete, past its window");
    assert!(found.reput.is_empty() && found.redeleted.is_empty());
    let run = a.svc.minter().run();
    assert_ne!(run, s1.run, "another run");
    let empty = snapshot(&g, D).await;
    assert!(empty.live.is_empty() && empty.tombs.is_empty(), "{empty:?}");
    assert_eq!(empty.run, run);
    bus.link().heal();
    eventually("the seed after the heal, with s1", || async {
        a.author.reads(WORKER).unwrap().contains(&Asked {
            held: Some(s1),
            unchanged: false,
        })
    })
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        converged_on(&w.target.report_v2().await.unwrap(), s1, 1),
        "another run's snapshot with no entry is no verdict"
    );
    assert!(converged_on(&s.report().unwrap(), s1, 1));
    assert_eq!(s.running(), Some(v(1)));
    assert_eq!(w.target.acts(), acts);

    // 2. The author deletes D again: a delete it puts on the bus, though
    //    it holds no member there. Another run, admitted: the target
    //    reverts, its observed version that delete's.
    assert_eq!(a.author.delete(WORKER).await.unwrap(), Written::Nothing);
    let mark = s.mark();
    let del = version(a.author.delete_unheld(WORKER).await.unwrap());
    assert_eq!(del.run, run);
    s.until("reverted", |r| {
        *r == Report {
            source: Source::Baseline,
            outcome: Outcome::Reverted,
            last_known: false,
            observed: observed(del, None),
            in_force: None,
            refusal: None,
        }
    })
    .await;
    s.until_on(RUN, mark, "the baseline on Run", |r| {
        r.value() == drive::baseline()
    })
    .await;
    w.svc.close().await.unwrap();
    a.svc.close().await.unwrap();
}

// --- §12, §2.16: config owners as targets ----------------------------------------

const RF0: &str = "lab/rf0";
const CONFIGS: &str = "state/configs/{system}/{service}";

/// `configs.v1`, its contract listing `desired.v2` (§12: "the author's
/// contract listing `desired.v2`").
fn configs_v2() -> Implementation {
    copy_of("configs.v1", |t| {
        // §12: minor 2, its `uses` moved (F-606).
        let listed = t
            .replace("uses  = [\"desired.v1\"", "uses  = [\"desired.v2\"")
            .replace("minor = 1", "minor = 2");
        assert!(listed.contains("minor = 2") && listed.contains("\"desired.v2\""));
        listed
    })
}

/// `lab/author` authoring `configs` under `desired.v2`, on a client of
/// `ep`.
async fn configs_author(ep: &str) -> (zenoh::Session, Service, desired::Author) {
    let configs = configs_v2();
    let ci = configs.iface().clone();
    let session = client(ep).await;
    let mut ab = ServiceBuilder::new(&session, config(AUTHOR));
    ab.implement(configs).unwrap();
    let author = ab
        .desired_author(&ci, CONFIGS, AuthorStartup::default())
        .await
        .unwrap();
    assert_eq!(author.major(), Major::V2);
    let asvc = ab.start().await.unwrap();
    (session, asvc, author)
}

fn rf0_config() -> zenkey::ServiceConfig {
    let mut cfg = config(RF0).bind("configs", &[AUTHOR]);
    cfg.bindings.get_mut("configs").unwrap().params = [
        ("system".to_owned(), "self.system".to_owned()),
        ("service".to_owned(), "self.service".to_owned()),
    ]
    .into();
    cfg
}

/// `lab/rf0` on `session`: `config.v1`'s owner from `startup` on `device`,
/// fed through `desired.v2`; started, its first seed made.
async fn config_owner(
    session: &zenoh::Session,
    startup: zenkey::config_v1::Startup,
    device: Arc<dyn zenkey::config_v1::Device>,
) -> (Service, zenkey::config_v1::Configuration, desired::Target) {
    let configs = configs_v2();
    let ci = configs.iface().clone();
    let mut b = ServiceBuilder::new(session, rf0_config());
    b.require("configs", ci, true);
    let c = b.config_v1(startup, device).await.unwrap();
    let target = b
        .desired_config("configs", &configs, CONFIGS, None, TargetStartup::default())
        .await
        .unwrap();
    assert_eq!(target.major(), Major::V2);
    let svc = b.start().await.unwrap();
    eventually("the target's first seed", || async {
        target.last_seed().is_some()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    (svc, c, target)
}

/// §12 A `config.v1` or `config.v2` owner's re-mint is a restart (§2.7,
/// §2.16): as v1's, the author's contract listing `desired.v2`, here a
/// `config.v1` owner. Its report is its own, in its view and its status.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s12_a_config_owners_re_mint_is_a_restart() {
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
    let (_author_session, asvc, author) = configs_author(&bus.ep1).await;
    let owner = client(&bus.ep1).await;
    let (mut svc, cfg, target) = config_owner(
        &owner,
        startup.clone(),
        Arc::clone(&device) as Arc<dyn config_v1::Device>,
    )
    .await;
    let duty = || device.holds("duty", "fraction_ppm");

    // Setup: converged on revision 1.
    let s1 = version(
        author
            .put(
                RF0,
                bytes(&json!({"revision": 1, "values": {"duty": {"fraction_ppm": 20000}}})),
            )
            .await
            .unwrap(),
    );
    eventually("revision 1 converged", || async {
        converged(cfg.desired().await)
    })
    .await;
    assert_eq!(duty(), Some(config_v1::Value::Integer(20000)));
    eventually("the role's own report", || async {
        target
            .report_v2()
            .await
            .is_some_and(|r| converged_on(&r, s1, 1))
    })
    .await;

    // 1. A re-mint while it runs: refused, and nothing moves.
    let old = svc.instance().to_string();
    let mark = s.mark();
    let err = svc.new_epoch().await.expect_err("the re-mint refused");
    assert!(err.to_string().contains("config.v1 §2.12"), "{err}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(svc.instance().to_string(), old);
    assert!(
        !s.all()
            .iter()
            .skip(mark)
            .any(|r| r.token && r.key.starts_with(TOKENS)),
        "no instance token of lab/rf0 withdrawn or declared"
    );
    assert!(converged(cfg.desired().await), "St still converged");
    assert_eq!(duty(), Some(config_v1::Value::Integer(20000)));

    // 2. The process stops, and starts again: a new process, from the
    //    startup values, which acts on revision 1 again.
    let mark = s.mark();
    svc.close().await.unwrap();
    device.restart(&startup);
    let (svc, cfg, target) = config_owner(
        &owner,
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
    assert_eq!(
        target.acts(),
        1,
        "the new process acted on revision 1 again"
    );
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
    let applied = since
        .iter()
        .filter(|r| !r.token && !r.delete && r.key.starts_with(EV))
        .map(Rec::value)
        .filter(|e| e["instance"] == json!(new) && e["outcome"] == json!("applied"))
        .count();
    assert_eq!(applied, 1, "the new process's act, one `set`, applied");
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

// --- §16: both majors side by side -------------------------------------------------

/// §16's author in runs A and B: v1's `tasking.v1` at minor 1, its `uses`
/// moved to `desired.v2` (§2.18, "An author moves by a revision").
fn tasking_v1_moved() -> Implementation {
    copy_of("tasking.v1", |t| {
        let moved = t.replace("minor = 0", "minor = 1").replace(
            "uses  = [\"desired.v1\", \"freshness.v1\"]",
            "uses  = [\"desired.v2\", \"freshness.v1\"]",
        );
        assert_eq!(moved.matches("desired.v2").count(), 1, "{moved}");
        moved
    })
}

/// §16's run C revision: `tasking.v2` in a revision whose `uses` lists
/// `desired.v1`.
fn tasking_v2_on_v1() -> Implementation {
    copy_of("tasking.v2", |t| {
        let back = t.replace(
            "uses  = [\"desired.v2\", \"freshness.v2\"]",
            "uses  = [\"desired.v1\", \"freshness.v2\"]",
        );
        assert_ne!(back, t);
        back
    })
}

/// §16's author on a client of `ep`, implementing `tasking`: its runs from
/// a record of its own, its interface in its tokenless set.
async fn author_of(ep: &str, tasking: Implementation, rec: &Records) -> AuthorUp {
    let session = client(ep).await;
    let iface = tasking.iface().clone();
    let mut b = ServiceBuilder::new(&session, config(AUTHOR).tokenless(iface.clone()));
    b.run_store(Arc::new(zenkey::run::FileRunStore::new(&rec.runs)))
        .unwrap();
    b.implement(tasking).unwrap();
    let author = b
        .desired_author(&iface, TASKS, AuthorStartup::default())
        .await
        .unwrap();
    let svc = b.start().await.unwrap();
    AuthorUp {
        session,
        svc,
        author,
    }
}

/// §16's target on a client of `ep`: `worker`, its role `task` against
/// `tasking`, on the owner example's drive (`v2`: worker.v2's lines).
async fn target_of(
    ep: &str,
    worker: Implementation,
    tasking: &Implementation,
    v2: bool,
    startup: TargetStartup,
) -> (
    zenoh::Session,
    ServiceBuilder,
    zenkey::Result<desired::Target>,
    Arc<Drive>,
) {
    let session = client(ep).await;
    let wi = worker.iface().clone();
    let mut cfg = config(WORKER).bind(ROLE, &[AUTHOR]);
    cfg.bindings.get_mut(ROLE).unwrap().params = [
        ("system".to_owned(), "self.system".to_owned()),
        ("service".to_owned(), "self.service".to_owned()),
    ]
    .into();
    let mut b = ServiceBuilder::new(&session, cfg);
    b.implement(worker).unwrap();
    let run = b
        .declare_state_writer(&wi, RUNNING, &Bindings::new())
        .await
        .unwrap();
    run.put_value(&drive::baseline()).await.unwrap();
    let d = Arc::new(if v2 {
        Drive::new_v2(run)
    } else {
        Drive::new(run)
    });
    let target = b
        .desired_target(
            ROLE,
            tasking,
            TASKS,
            ReportAt::new(wi, REPORT),
            Arc::clone(&d) as Arc<dyn desired::Handler>,
            startup,
        )
        .await;
    (session, b, target, d)
}

/// §16's runs A and B: a target built for `desired.v1` (`worker.v1`, told
/// `tasking.v1` at minor 0 and reading that word alone, as a target built
/// before this major does) under `tasking.v1` at minor 1, which uses
/// `desired.v2`, through R1 timestamping or not.
async fn s16_a_target_built_before(timestamping: bool) {
    const D1: &str = "zk2/lab/author/tasking.v1/state/tasks/lab/worker";
    const RP1: &str = "zk2/lab/worker/worker.v1/state/desired/task";
    let run = if timestamping { "run B" } else { "run A" };
    let (r1, ep) = common::router_with(
        None,
        &[(
            "timestamping/enabled",
            if timestamping { "true" } else { "false" },
        )],
    )
    .await;
    let rec = records();
    let s = record(&ep).await;
    let a = author_of(&ep, tasking_v1_moved(), &rec).await;
    assert_eq!(a.author.major(), Major::V2, "{run}: the author moved");
    author_seen(&ep, true).await;
    let (session, b, target, d) = target_of(
        &ep,
        imp("worker.v1"),
        &imp("tasking.v1"),
        false,
        TargetStartup::default().by_word_alone(),
    )
    .await;
    let target = target.unwrap();
    assert_eq!(target.major(), Major::V1, "{run}: built for desired.v1");
    let svc = b.start().await.unwrap();
    eventually("the target's first seed", || async {
        target.last_seed().is_some()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    // Its seed acts on nothing: the author answers by snapshot, whose
    // entry carries no Zenoh timestamp.
    let seeded = target.last_seed().unwrap();
    assert_eq!(seeded.revision, desired::AuthorRevision::NotRead, "{run}");
    let baseline = zenkey::model::desired::Report::baseline(Some(D1.to_owned()));
    assert_eq!(target.report().await, baseline, "{run}: the baseline's");
    let v1_report =
        |r: &Rec| zenkey::model::desired::decode(&r.payload).expect("a desired.v1.Report");

    // 1. The author puts v1.
    let mark = s.mark();
    a.author.put(WORKER, bytes(&v(1))).await.unwrap();
    eventually("the put at S", || async { !s.on(D1, mark).is_empty() }).await;
    let put = s.on(D1, mark).remove(0);
    let wait = Duration::from_secs(2);
    if timestamping {
        assert_eq!(
            put.stamped_by,
            Some(r1.zid().to_string()),
            "run B: the put arrives with R1's Zenoh timestamp"
        );
        eventually("run B: converged on v1", || async {
            let r = target.report().await;
            r.outcome == Some(zenkey::model::desired::Outcome::Converged)
                && r.observed.as_ref().is_some_and(|o| o.revision == Some(1))
        })
        .await;
        let r = target.report().await;
        let o = r.observed.expect("observed");
        assert_eq!(
            format!("{:x}", o.stamp.id),
            r1.zid().to_string(),
            "run B: under R1's id"
        );
        assert_eq!(v1_report(s.on(RP1, 0).last().unwrap()), r, "run B: put");
    } else {
        assert_eq!(put.stamped_by, None, "run A: no Zenoh timestamp");
        tokio::time::sleep(wait).await;
        assert_eq!(target.acts(), 0, "run A: acts on nothing");
        assert_eq!(target.report().await, baseline, "run A: the baseline's");
    }

    // 2. The author puts v2.
    a.author.put(WORKER, bytes(&v(2))).await.unwrap();
    if timestamping {
        eventually("run B: acts on v2", || async {
            target
                .report()
                .await
                .observed
                .is_some_and(|o| o.revision == Some(2))
                && d.with(|x| x.running.clone()) == v(2)
        })
        .await;
    } else {
        tokio::time::sleep(wait).await;
        assert_eq!(target.acts(), 0, "run A: nothing");
    }
    let acts = target.acts();
    let held = target.report().await;

    // 3. The author deletes D.
    let mark = s.mark();
    a.author.delete(WORKER).await.unwrap();
    eventually("the delete at S", || async {
        s.on(D1, mark).iter().any(|r| r.delete)
    })
    .await;
    assert_eq!(
        s.on(D1, mark).iter().find(|r| r.delete).unwrap().stamped_by,
        None,
        "{run}: the delete arrives with no Zenoh timestamp"
    );
    tokio::time::sleep(wait).await;
    assert_eq!(target.acts(), acts, "{run}: the delete is ignored");
    assert_eq!(target.report().await, held, "{run}: the report as it was");
    if timestamping {
        assert_eq!(
            d.with(|x| x.running.clone()),
            v(2),
            "run B: it runs a document its author deleted"
        );
    } else {
        assert_eq!(target.report().await, baseline, "run A: inert");
    }
    svc.close().await.unwrap();
    session.close().await.unwrap();
    a.kill().await;
}

/// §16's run C: `worker.v2`'s target, whose contract declares the packed
/// `Report` alone, told its author's revision of `tasking.v2` that uses
/// `desired.v1`: it refuses to start, naming its role.
async fn s16_run_c() {
    let bus = one_router().await;
    let s = record(&bus.ep1).await;
    let (session, b, target, _) = target_of(
        &bus.ep1,
        imp("worker.v2"),
        &tasking_v2_on_v1(),
        true,
        TargetStartup::default(),
    )
    .await;
    refused_for(target, "role \"task\"", "run C: the role");
    let svc = b.start().await;
    refused_for(svc, "desired.v2 §2.18", "run C: the start");
    // The other way (§2.18): `worker.v1`'s report alone, told runs A and
    // B's author's revision, which uses desired.v2, refuses too.
    let (other, b, target, _) = target_of(
        &bus.ep1,
        imp("worker.v1"),
        &tasking_v1_moved(),
        false,
        TargetStartup::default(),
    )
    .await;
    refused_for(target, "uses desired.v2", "run C, the other way: the role");
    refused_for(
        b.start().await,
        "role \"task\"",
        "run C, the other way: the start",
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        worker_tokens(&s),
        0,
        "run C: no instance token of lab/worker"
    );
    session.close().await.unwrap();
    other.close().await.unwrap();
}

/// §16's run D: the same target, told the scenarios' `tasking.v2`, its
/// author on run C's revision, whose descriptor reaches the target.
async fn s16_run_d() {
    let bus = one_router().await;
    let rec = records();
    let s = record(&bus.ep1).await;
    let theirs = tasking_v2_on_v1();
    let fingerprint = zenkey::model::canonical::Fingerprint::of(theirs.contract()).to_string();
    let a = author_of(&bus.ep1, theirs, &rec).await;
    assert_eq!(a.author.major(), Major::V1, "run D: the author's revision");
    author_seen(&bus.ep1, true).await;

    // 5. The target starts, and reads its author's descriptor: it starts,
    //    says so where it says what its seed found, and Rp holds the
    //    baseline's report.
    let (session, b, target, d) = target_of(
        &bus.ep1,
        imp("worker.v2"),
        &tasking(0),
        true,
        TargetStartup::default(),
    )
    .await;
    let target = target.expect("run D: its word names a revision it can report");
    assert_eq!(target.major(), Major::V2);
    let svc = b.start().await.expect("run D: the target starts");
    eventually("run D: the target's first seed", || async {
        target.last_seed().is_some()
    })
    .await;
    assert_eq!(
        target.last_seed().unwrap().revision,
        desired::AuthorRevision::Stopped {
            fingerprint,
            major: Major::V1
        },
        "run D: its author's descriptor names one that uses desired.v1"
    );
    s.until("run D: the baseline's report", |r| *r == Report::baseline())
        .await;

    // 6. The author puts v1: the target acts on nothing, and Rp and Run
    //    receive nothing.
    let mark = s.mark();
    let put = a.author.put(WORKER, bytes(&v(1))).await.unwrap();
    assert!(matches!(put, Written::Put(_)), "a desired.v1 put: {put:?}");
    eventually("run D: the put at S", || async {
        !s.on(D, mark).is_empty()
    })
    .await;
    assert!(
        s.on(D, mark)[0].inband.is_some(),
        "run D: the put carries an in-band stamp, which the target would order"
    );
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(target.acts(), 0, "run D: acts on nothing");
    assert!(s.on(RP, mark).is_empty(), "run D: Rp receives nothing");
    assert!(s.on(RUN, mark).is_empty(), "run D: Run receives nothing");
    assert_eq!(d.with(|x| x.running.clone()), drive::baseline());
    assert_eq!(target.report_v2().await, Some(Report::baseline()));
    svc.close().await.unwrap();
    session.close().await.unwrap();
    a.kill().await;
}

/// The targets' log: every `zenkey::desired` warning this binary's targets
/// write, installed as the process's subscriber once, by the first run
/// that reads it.
fn desired_log() -> &'static Arc<Mutex<Vec<u8>>> {
    #[derive(Clone)]
    struct Buf(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    static LOG: std::sync::OnceLock<Arc<Mutex<Vec<u8>>>> = std::sync::OnceLock::new();
    LOG.get_or_init(|| {
        let buf = Arc::<Mutex<Vec<u8>>>::default();
        let w = Buf(Arc::clone(&buf));
        let _ = tracing::subscriber::set_global_default(
            tracing_subscriber::fmt()
                .with_writer(move || w.clone())
                .with_ansi(false)
                .without_time()
                .with_env_filter(tracing_subscriber::EnvFilter::new("zenkey::desired=warn"))
                .finish(),
        );
        buf
    })
}

/// The target's lines saying its role stopped under `run`, at the
/// revision `fingerprint` (§2.18).
fn stops_said(run: Run, fingerprint: &str) -> usize {
    let text = String::from_utf8(desired_log().lock().unwrap().clone()).unwrap();
    let at = format!("run={run}");
    text.lines()
        .filter(|l| l.contains("the role is stopped") && l.contains(fingerprint))
        .filter(|l| l.split_whitespace().any(|w| w == at))
        .count()
}

/// §16's run E: run D's target, told the scenarios' `tasking.v2`, its
/// author on that revision, whose descriptor reaches the target. The
/// author's process ends without closing, and it starts again on run C's
/// revision, under a new run from its record, putting v1 again and then
/// v2: the target reads the descriptor stating the new run before it
/// judges anything of it (§2.18, "A new run is read first"), finds the
/// other major, says the role is stopped, and acts on neither put.
async fn s16_run_e() {
    desired_log();
    let bus = one_router().await;
    let rec = records();
    let s = record(&bus.ep1).await;
    let a = author_of(&bus.ep1, tasking(0), &rec).await;
    assert_eq!(a.author.major(), Major::V2, "run E: the author's revision");
    author_seen(&bus.ep1, true).await;

    // 7. The target starts, and the author puts v1: the target acts on v1,
    //    and the report is converged on it.
    let (session, b, target, d) = target_of(
        &bus.ep1,
        imp("worker.v2"),
        &tasking(0),
        true,
        TargetStartup::default(),
    )
    .await;
    let target = target.expect("run E: its word names a revision it can report");
    let svc = b.start().await.expect("run E: the target starts");
    eventually("run E: the target's first seed", || async {
        target.last_seed().is_some()
    })
    .await;
    assert_eq!(
        target.last_seed().unwrap().revision,
        desired::AuthorRevision::Given,
        "run E: its author's descriptor names the revision it was given"
    );
    let s1 = version(a.author.put(WORKER, bytes(&v(1))).await.unwrap());
    s.until("run E: converged on v1", |r| converged_on(r, s1, 1))
        .await;
    assert_eq!(d.with(|x| x.running.clone()), v(1), "run E: v1 runs");
    let acts = target.acts();
    assert!(acts >= 1, "run E: acts on v1");

    // 8. The author's process ends without closing, and starts again on run
    //    C's revision, under a new run, from its record. It puts v1 again,
    //    and then v2.
    a.kill().await;
    let theirs = tasking_v2_on_v1();
    let fingerprint = zenkey::model::canonical::Fingerprint::of(theirs.contract()).to_string();
    let mark = s.mark();
    let a = author_of(&bus.ep1, theirs, &rec).await;
    assert_eq!(a.author.major(), Major::V1, "run E: run C's revision");
    for n in [1, 2] {
        let put = a.author.put(WORKER, bytes(&v(n))).await.unwrap();
        assert!(matches!(put, Written::Put(_)), "a desired.v1 put: {put:?}");
    }
    eventually("run E: both puts at S", || async {
        s.on(D, mark).iter().filter(|r| !r.delete).count() >= 2
    })
    .await;
    let run = s.on(D, mark)[0]
        .inband
        .expect("run E: the put carries an in-band stamp, which the target would order")
        .run;
    assert_ne!(run, s1.run, "run E: a new run");
    // The target judges nothing of the new run before it reads the
    // descriptor stating it, which names run C's revision: the role stops,
    // and the target says so, once.
    eventually("run E: the target says the role stopped", || async {
        stops_said(run, &fingerprint) > 0
    })
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(stops_said(run, &fingerprint), 1, "run E: said once");
    assert_eq!(target.acts(), acts, "run E: acts on neither put");
    assert!(s.on(RP, mark).is_empty(), "run E: Rp receives nothing");
    assert!(s.on(RUN, mark).is_empty(), "run E: Run receives nothing");
    assert_eq!(d.with(|x| x.running.clone()), v(1), "run E: v1 still runs");
    assert!(
        converged_on(&target.report_v2().await.unwrap(), s1, 1),
        "run E: the report as it was"
    );
    svc.close().await.unwrap();
    session.close().await.unwrap();
    a.kill().await;
}

/// §16, the new run's hold (§2.18, "None states the run yet", text 0.3,
/// zk2py's round 127, F-621): an author MAY put a version before its
/// descriptor (core §8.2). The target is converged on v1; its author's
/// process ends without closing, and X, a session of the test's, plays its
/// next instance, putting v2 under a new run before anything else. The
/// target's read of the author's descriptors finds none stating the run,
/// where descriptors reach it, and holds v2: it acts on nothing, and `Rp`
/// and `Run` receive nothing. X serves the descriptor stating the run and
/// naming the revision the target was given, and still nothing moves, since
/// the target reads again when the author's instance token appears. X
/// declares it: the target reads, and acts on the v2 it held, its report
/// converged on it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s16_a_version_put_before_its_descriptor_waits_for_its_token() {
    let bus = one_router().await;
    let rec = records();
    let s = record(&bus.ep1).await;
    let a = author_of(&bus.ep1, tasking(0), &rec).await;
    author_seen(&bus.ep1, true).await;
    let (session, b, target, d) = target_of(
        &bus.ep1,
        imp("worker.v2"),
        &tasking(0),
        true,
        TargetStartup::default(),
    )
    .await;
    let target = target.expect("its word names a revision it can report");
    let svc = b.start().await.expect("the target starts");
    eventually("the target's first seed", || async {
        target.last_seed().is_some()
    })
    .await;
    let s1 = version(a.author.put(WORKER, bytes(&v(1))).await.unwrap());
    s.until("converged on v1", |r| converged_on(r, s1, 1)).await;
    let acts = target.acts();
    // The author's descriptor, which X serves for its next instance.
    let g = client(&bus.ep1).await;
    let desc = presence::descriptor(&g, &addr(AUTHOR), a.svc.instance(), T)
        .await
        .unwrap()
        .into_descriptor()
        .unwrap();
    a.kill().await;
    author_seen(&bus.ep1, false).await;

    // X puts v2 under a new run, before any descriptor.
    let run = Run::U16(0x0bad);
    assert_ne!(run, s1.run, "a new run");
    let s2 = Stamp {
        run,
        seq: Seq::new(1).unwrap(),
    };
    let x = client(&bus.ep1).await;
    let mark = s.mark();
    put_as(&x, s2, &v(2)).await;
    eventually("X's put at S", || async { !s.on(D, mark).is_empty() }).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(target.acts(), acts, "held: no descriptor states its run");
    assert!(s.on(RP, mark).is_empty(), "Rp receives nothing");
    assert!(s.on(RUN, mark).is_empty(), "Run receives nothing");

    // Its descriptor, stating the run: no read before the token.
    let (_descriptor, key) = answer_descriptor(&x, desc, "00000000000000bb", run).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(target.acts(), acts, "held until the token");
    assert!(s.on(RUN, mark).is_empty(), "Run receives nothing yet");

    // Its instance token: the target reads, and acts on the v2 it held.
    let _token = x.liveliness().declare_token(key).await.unwrap();
    s.until("converged on v2, held", |r| converged_on(r, s2, 2))
        .await;
    assert_eq!(d.with(|x| x.running.clone()), v(2), "v2 runs");
    assert_eq!(target.acts(), acts + 1, "acted on v2 once");
    svc.close().await.unwrap();
    session.close().await.unwrap();
}

/// §16, the new run's hold where no descriptor has ever arrived (§2.18,
/// text 0.3, round 127's mend, F-627 and F-628): where descriptors reach a
/// target is the deployment's, never what a read drew, so a target that
/// starts before its author, on the near side, holds too. The target
/// starts, and no author has started. X, a session of the test's, plays
/// the author, putting v1 under its run before anything else: the target's
/// read draws no descriptor, and it holds v1, acting on nothing. X answers
/// a descriptor stating the run: still nothing. X declares its instance
/// token: the target reads, and acts on v1. Then X puts v2 under a second
/// run, and declares a second instance token with no descriptor stating
/// that run: the read at the token, made again a few times and soon,
/// names none, and only then leaves the deployment's word: the target acts
/// on v2.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s16_a_target_before_its_author_holds_a_version_put_before_its_descriptor() {
    let bus = one_router().await;
    let s = record(&bus.ep1).await;
    // The author's descriptor as X answers it, read from an author that
    // runs alone, before the target starts, and is gone again.
    let desc = author_descriptor(&bus.ep1).await;

    // The target starts: no author yet, nor any descriptor.
    let (session, b, target, d) = target_of(
        &bus.ep1,
        imp("worker.v2"),
        &tasking(0),
        true,
        TargetStartup::default(),
    )
    .await;
    let target = target.expect("its word names a revision it can report");
    let svc = b.start().await.expect("the target starts");
    eventually("the target's first seed", || async {
        target.last_seed().is_some()
    })
    .await;
    s.until("the baseline's report", |r| *r == Report::baseline())
        .await;
    let acts = target.acts();

    // X puts v1 under its run, before any descriptor: held.
    let x = client(&bus.ep1).await;
    let s1 = Stamp {
        run: Run::U16(0x0a11),
        seq: Seq::new(1).unwrap(),
    };
    let mark = s.mark();
    put_as(&x, s1, &v(1)).await;
    eventually("X's put at S", || async { !s.on(D, mark).is_empty() }).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(target.acts(), acts, "held: no descriptor states its run");
    assert!(s.on(RP, mark).is_empty(), "Rp receives nothing");
    assert!(s.on(RUN, mark).is_empty(), "Run receives nothing");
    // Its descriptor, stating the run: no read before the token.
    let (_descriptor, key) = answer_descriptor(&x, desc.clone(), "00000000000000dd", s1.run).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(target.acts(), acts, "held until the token");
    // Its instance token: the target reads, and acts on the v1 it held.
    let _token = x.liveliness().declare_token(key).await.unwrap();
    s.until("converged on v1, held", |r| converged_on(r, s1, 1))
        .await;
    assert_eq!(d.with(|x| x.running.clone()), v(1), "v1 runs");
    let acts = target.acts();

    // v2 under a second run, then a second instance token, with no
    // descriptor stating that run: the read at the token names none, and
    // the deployment's word stands.
    let s2 = Stamp {
        run: Run::U16(0x0a12),
        seq: Seq::new(1).unwrap(),
    };
    let mark = s.mark();
    put_as(&x, s2, &v(2)).await;
    eventually("X's second put at S", || async {
        !s.on(D, mark).is_empty()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(target.acts(), acts, "held: no descriptor states the run");
    let _second = x
        .liveliness()
        .declare_token("zk2/lab/author/@zk/instance/00000000000000de")
        .await
        .unwrap();
    s.until("converged on v2, by the deployment's word", |r| {
        converged_on(r, s2, 2)
    })
    .await;
    assert_eq!(target.acts(), acts + 1, "acted on v2 once");
    svc.close().await.unwrap();
    session.close().await.unwrap();
}

/// §16 Both majors side by side (§2.18): runs A to E, each on a router of
/// its own, at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s16_both_majors_side_by_side() {
    tokio::join!(
        s16_a_target_built_before(false),
        s16_a_target_built_before(true),
        s16_run_c(),
        s16_run_d(),
        s16_run_e(),
    );
}

/// `desired.v2` §2.16: `config.v2`'s owner `lab/rf0` as a target of an
/// author whose contract lists `desired.v2`. A put converges through `set`
/// and is reported in the record's fields; a refused one keeps what was in
/// force; a delete reverts each parameter the documents named to its
/// startup value. With no report resource given, its refusal's place has
/// no table to index: the role's own report says `out_of_range`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn config_v2_as_a_target() {
    use zenkey::config_v2;
    use zenkey::model::config::v2::ParamState;
    use zenkey::model::config::{DesiredOutcome, Value as CValue};

    let bus = one_router().await;
    let configs = configs_v2();
    let ci = configs.iface().clone();
    let (_author_session, asvc, author) = configs_author(&bus.ep1).await;
    let owner = client(&bus.ep1).await;
    let startup = sim::rfcfg_startup(10, Duration::from_secs(1), true);
    let device = Arc::new(sim::SimDevice::new_v2(&startup));
    let mut b = ServiceBuilder::new(&owner, rf0_config());
    b.require("configs", ci.clone(), true);
    b.implement(Implementation::new(sim::rfcfg().unwrap()))
        .unwrap();
    let c = b
        .config_v2(startup, Arc::clone(&device) as Arc<dyn config_v2::Device>)
        .await
        .unwrap();
    let t = b
        .desired_config("configs", &configs, CONFIGS, None, TargetStartup::default())
        .await
        .unwrap();
    let svc = b.start().await.unwrap();
    eventually("the target's first seed", || async {
        t.last_seed().is_some()
    })
    .await;
    let duty = |s: &config_v2::State| match &s.groups["duty"].params["fraction_ppm"] {
        ParamState::Plain { value, .. } => value.clone(),
        ParamState::Sensitive { .. } => None,
    };
    assert_eq!(duty(&c.state().await), Some(CValue::Integer(10000)));
    let outcome = || async { t.report_v2().await.unwrap().outcome };

    // A put: converged, in the record's fields.
    author
        .put(
            RF0,
            bytes(&json!({"revision": 1, "values": {"duty": {"fraction_ppm": 20000}}})),
        )
        .await
        .unwrap();
    eventually("revision 1 converged", || async {
        c.state().await.desired.is_some_and(|d| {
            d.observed_revision == 1 && d.outcome == Some(DesiredOutcome::Converged)
        })
    })
    .await;
    assert_eq!(duty(&c.state().await), Some(CValue::Integer(20000)));
    eventually("the role's report", || async {
        outcome().await == Outcome::Converged
    })
    .await;

    // Refused whole: a reach group, the record naming it.
    author
        .put(
            RF0,
            bytes(&json!({"revision": 2, "values": {
                "duty": {"fraction_ppm": 30000}, "radio": {"frequency_hz": 869525000}}})),
        )
        .await
        .unwrap();
    eventually("revision 2 refused", || async {
        outcome().await == Outcome::Refused
    })
    .await;
    let refusal = t.report_v2().await.unwrap().refusal.unwrap();
    assert_eq!(
        (refusal.code, refusal.pointer),
        (Code::Reach, Some(Index::OutOfRange))
    );
    let d = c.state().await.desired.unwrap();
    assert_eq!(
        (d.outcome, d.refusal.and_then(|r| r.group)),
        (Some(DesiredOutcome::Refused), Some("radio".to_owned()))
    );
    assert_eq!(duty(&c.state().await), Some(CValue::Integer(20000)));

    // The delete: the parameters named back at their startup values, and
    // no desired fields in the record.
    author.delete(RF0).await.unwrap();
    eventually("reverted", || async {
        outcome().await == Outcome::Reverted
    })
    .await;
    let s = c.state().await;
    assert_eq!(duty(&s), Some(CValue::Integer(10000)), "the file's value");
    assert_eq!(s.desired, None);
    svc.close().await.unwrap();
    asvc.close().await.unwrap();
}

/// The two contracts the scenarios give load as the owner example embeds
/// them, byte for byte, and "tasking.v2, watched" is the same file with
/// its horizon.
#[test]
fn the_examples_contracts_are_the_scenarios() {
    let (tasking_v2, worker) = drive::contracts_v2(0).unwrap();
    assert_eq!(
        Implementation::new(tasking_v2).fingerprint(),
        imp("tasking.v2").fingerprint()
    );
    assert_eq!(
        Implementation::new(worker).fingerprint(),
        imp("worker.v2").fingerprint()
    );
    let (watched, _) = drive::contracts_v2(4).unwrap();
    assert_eq!(
        Implementation::new(watched).fingerprint(),
        tasking(4).fingerprint()
    );
}
