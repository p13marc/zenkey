//! `spec/profiles/telemetry/scenarios.md` §1–§8 (`telemetry.v1`, #819,
//! TB), one test per section, named after it. §8 runs here as R0 alone; the
//! tool's run behind a real face is #819's TC.
//!
//! Common setup, the text's: R1 is a router with timestamping on, a
//! router's default; each test brings up its own, on an ephemeral port. The
//! owner O is `lab/meter`, implementing `meter.v1`
//! (`tests/contracts/meter.v1.toml`) with every resource exposed, through
//! the runtime's writers, as a client of R1: each Point is taken with
//! [`telemetry::point`], its `timestamp` this host's clock when the value
//! is taken, and the writer writes its `epoch`. R subscribes to
//! `zk2/lab/meter/meter.v1/stream/*` and holds a liveliness subscription,
//! with history, to `zk2/lab/meter/@zk/instance/*`, both declared before O
//! starts, recording each arrival on this host's monotonic clock. "R reads"
//! is `zenkey_model::telemetry::check` against the resource's declaration;
//! "R pairs" is `zenkey_model::telemetry::pair` over consecutive conforming
//! samples of a member, with the presence R saw. R0 pairs the same samples
//! with no presence at all, as a reader across a constrained face does
//! (§2.14). Every verdict is the model's.
//!
//! A *raw* step puts from O's session, bypassing the writer, after the
//! writer refused the same value in the same step.

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{client, config, contract, eventually, imp, router};
use zenkey::model::contract::Contract;
use zenkey::model::grammar::IfaceId;
use zenkey::model::telemetry::{
    self as tm, Basis, Checked, Conformance, Continuity, Datum, Declaration, Measure, NoRate,
    Payload, Presence, Problem, Rate, Sampled, Source, Via,
};
use zenkey::model::template::Bindings;
use zenkey::telemetry::{self, Histogram, Point, Reason, Value};
use zenkey::writer::Writer;
use zenkey::{Error, Service, ServiceBuilder};
use zenoh::bytes::Encoding;
use zenoh::sample::{Sample, SampleKind};

/// The owner of the scenarios' conventions.
const SVC: &str = "lab/meter";

/// Samples of one member are at least this far apart.
const SPACING: Duration = Duration::from_millis(200);

/// A section waits this long after a token's appearance or withdrawal
/// before its next put.
const AFTER_TOKEN: Duration = Duration::from_millis(500);

/// The declared bounds of `latency`.
const BOUNDS: [f64; 3] = [0.005, 0.01, 0.025];

/// `meter.v1`'s resources, by template.
const RESOURCES: [&str; 5] = ["frames", "bytes", "drops", "latency", "level"];

fn meter() -> IfaceId {
    "meter.v1".parse().expect("an interface")
}

fn key_of(resource: &str) -> String {
    format!("zk2/{SVC}/meter.v1/stream/{resource}")
}

// ---------------------------------------------------------------------------
// O, the owner
// ---------------------------------------------------------------------------

/// O: its session, its service and a writer per resource.
struct Owner {
    session: zenoh::Session,
    svc: Service,
    writers: BTreeMap<&'static str, Writer>,
}

/// O started on a session of its own, as a client of the router at `ep`.
async fn start(ep: &str) -> Owner {
    let session = client(ep).await;
    let mut b = ServiceBuilder::new(&session, config(SVC));
    b.implement(imp("meter.v1")).unwrap();
    let mut writers = BTreeMap::new();
    for r in RESOURCES {
        let w = b
            .declare_writer(&meter(), &format!("stream/{r}"), &Bindings::new())
            .await
            .unwrap();
        writers.insert(r, w);
    }
    let svc = b.start().await.unwrap();
    let o = Owner {
        session,
        svc,
        writers,
    };
    for w in o.writers.values() {
        eventually("R's subscription reaches O", || async {
            w.matching().await.unwrap()
        })
        .await;
    }
    o
}

impl Owner {
    fn w(&self, r: &str) -> &Writer {
        &self.writers[r]
    }

    fn instance(&self) -> String {
        self.svc.instance().to_string()
    }

    /// `frames` at `n`, a counter Point taken now.
    async fn frames(&self, n: u64) {
        self.w("frames")
            .put_value(&telemetry::point(n))
            .await
            .unwrap();
    }

    /// A bare counter on `drops` or `bytes`, its epoch the writer's.
    async fn count(&self, r: &str, n: u64) {
        self.w(r).put_value(&n).await.unwrap();
    }

    /// The process ends: its session closes, and its tokens with it.
    async fn end(self) {
        self.session.close().await.unwrap();
        drop(self.svc);
    }

    /// A *raw* put of `payload` on `resource`, from O's session.
    async fn raw(&self, resource: &str, payload: &serde_json::Value, attachment: Option<&str>) {
        let mut put = self
            .session
            .put(key_of(resource), serde_json::to_vec(payload).unwrap())
            .encoding(Encoding::APPLICATION_JSON);
        if let Some(a) = attachment {
            put = put.attachment(serde_json::to_vec(a).unwrap());
        }
        put.await.unwrap();
    }
}

/// The writer's refusal of a put, as `telemetry.v1` reasons it.
fn refusal(e: Error) -> Reason {
    match e {
        Error::Telemetry { reason, .. } => *reason,
        other => panic!("not a telemetry.v1 refusal: {other}"),
    }
}

/// A Point the owner would put, as JSON, its epoch `epoch`.
fn point_json(value: impl Into<Value>, epoch: Option<&str>) -> serde_json::Value {
    let mut p = telemetry::point(value);
    p.epoch = epoch.map(str::to_owned);
    serde_json::to_value(p).unwrap()
}

// ---------------------------------------------------------------------------
// R and R0, the readers
// ---------------------------------------------------------------------------

/// One sample, as R recorded it.
#[derive(Debug, Clone)]
struct Rec {
    /// Arrival, in nanoseconds on R's monotonic clock since it started.
    at: i128,
    resource: String,
    payload: Vec<u8>,
    attachment: Option<Vec<u8>>,
}

/// One instance token's appearance or withdrawal, as R saw it.
#[derive(Debug, Clone)]
struct Token {
    at: i128,
    instance: String,
    up: bool,
}

#[derive(Debug, Default)]
struct Log {
    samples: Vec<Rec>,
    tokens: Vec<Token>,
}

/// R: its subscriptions, declared before O starts, and what they saw.
struct Reader {
    _data: zenoh::pubsub::Subscriber<()>,
    _tokens: zenoh::pubsub::Subscriber<()>,
    log: Arc<Mutex<Log>>,
    t0: Instant,
    contract: Contract,
}

async fn reader(s: &zenoh::Session) -> Reader {
    let log: Arc<Mutex<Log>> = Arc::default();
    let t0 = Instant::now();
    let since = move || i128::try_from(t0.elapsed().as_nanos()).unwrap();
    let w = Arc::clone(&log);
    let data = s
        .declare_subscriber(format!("zk2/{SVC}/meter.v1/stream/*"))
        .callback(move |x: Sample| {
            let key = x.key_expr().as_str();
            let resource = key.rsplit('/').next().unwrap().to_owned();
            w.lock().unwrap().samples.push(Rec {
                at: since(),
                resource,
                payload: x.payload().to_bytes().into_owned(),
                attachment: x.attachment().map(|a| a.to_bytes().into_owned()),
            });
        })
        .await
        .unwrap();
    let w = Arc::clone(&log);
    let tokens = s
        .liveliness()
        .declare_subscriber(format!("zk2/{SVC}/@zk/instance/*"))
        .history(true)
        .callback(move |x: Sample| {
            let instance = x.key_expr().as_str().rsplit('/').next().unwrap().to_owned();
            w.lock().unwrap().tokens.push(Token {
                at: since(),
                instance,
                up: x.kind() == SampleKind::Put,
            });
        })
        .await
        .unwrap();
    Reader {
        _data: data,
        _tokens: tokens,
        log,
        t0,
        contract: contract("meter.v1"),
    }
}

/// A sample R read (§2.6), with when it arrived.
#[derive(Debug, Clone)]
struct Read {
    at: i128,
    checked: Checked,
    attachment: Option<Vec<u8>>,
    payload: serde_json::Value,
}

impl Read {
    fn conforms(&self) -> bool {
        self.checked.conformance == Conformance::Conforms
    }

    fn reasons(&self) -> Vec<Problem> {
        match &self.checked.conformance {
            Conformance::DoesNot(p) => p.clone(),
            other => panic!("not does_not: {other:?}"),
        }
    }

    fn measure(&self) -> &Measure {
        &self.checked.reading.as_ref().expect("read").value
    }

    fn counter(&self) -> u64 {
        match self.measure() {
            Measure::Counter(n) => *n,
            m => panic!("not a counter: {m:?}"),
        }
    }

    fn epoch(&self) -> Option<&str> {
        self.checked.reading.as_ref()?.epoch.as_deref()
    }
}

/// A conforming sample as a pair holds it (§2.10, §2.11): a stream's,
/// so no owner's stamp.
fn sampled(r: &Read) -> Sampled<'_> {
    Sampled {
        value: r.measure(),
        epoch: r.epoch(),
        timestamp_ms: r.checked.reading.as_ref().unwrap().timestamp_ms,
        stamp_ns: None,
        arrival_ns: r.at,
    }
}

/// One pair R or R0 judged (§2.10, §2.11).
#[derive(Debug, Clone)]
struct Judged {
    earlier: Read,
    later: Read,
    pair: tm::Pair,
}

impl Judged {
    fn counters(&self) -> (u64, u64) {
        (self.earlier.counter(), self.later.counter())
    }

    fn verdict(&self) -> (Continuity, Option<Basis>) {
        (self.pair.continuity, self.pair.basis)
    }
}

const IN_BAND: Option<Basis> = Some(Basis::Established(Via::InBand));
const BY_PRESENCE: Option<Basis> = Some(Basis::Established(Via::Presence));
const ASSUMED: Option<Basis> = Some(Basis::Assumed);

impl Reader {
    fn declaration(&self, resource: &str) -> Declaration {
        let r = self
            .contract
            .resources
            .iter()
            .find(|r| r.template.as_str() == resource)
            .expect("a resource of meter.v1");
        tm::declaration_of(&self.contract, r)
    }

    /// The instance tokens R saw, so far.
    fn tokens(&self) -> Vec<Token> {
        self.log.lock().unwrap().tokens.clone()
    }

    /// Waits until R has seen `instance`'s token appear (`up`) or go, then
    /// the section's 500 ms.
    async fn token(&self, instance: &str, up: bool) {
        eventually(
            &format!("R sees {instance} {}", if up { "up" } else { "gone" }),
            || async {
                self.tokens()
                    .iter()
                    .any(|t| t.instance == instance && t.up == up)
            },
        )
        .await;
        tokio::time::sleep(AFTER_TOKEN).await;
    }

    /// Every sample R received of `resource`, read (§2.6), in arrival
    /// order.
    fn reads(&self, resource: &str) -> Vec<Read> {
        let decl = self.declaration(resource);
        let log = self.log.lock().unwrap();
        log.samples
            .iter()
            .filter(|r| r.resource == resource)
            .map(|r| {
                let value = Datum::from_json_bytes(&r.payload);
                let attachment = r.attachment.as_deref().and_then(Datum::from_json_bytes);
                let payload = match &value {
                    Some(value) => Payload::Decoded {
                        value,
                        attachment: attachment.as_ref(),
                    },
                    None => Payload::Undecodable,
                };
                Read {
                    at: r.at,
                    checked: tm::check(&decl, &payload),
                    attachment: r.attachment.clone(),
                    payload: serde_json::from_slice(&r.payload).unwrap_or_default(),
                }
            })
            .collect()
    }

    /// Waits until R holds `n` samples of `resource`; returns them read.
    async fn wait(&self, resource: &str, n: usize) -> Vec<Read> {
        eventually(&format!("{n} samples of {resource}"), || async {
            let log = self.log.lock().unwrap();
            log.samples
                .iter()
                .filter(|r| r.resource == resource)
                .count()
                >= n
        })
        .await;
        self.reads(resource)
    }

    /// What R saw of presence over two arrivals (§2.9, scenarios.md's
    /// conventions): one token alone throughout, and the only one held at
    /// each arrival.
    fn presence(&self, a: i128, b: i128) -> (bool, Option<String>, Option<String>) {
        let tokens = self.tokens();
        let held = |t: i128| {
            let mut held = std::collections::BTreeSet::new();
            for e in tokens.iter().filter(|e| e.at <= t) {
                if e.up {
                    held.insert(e.instance.clone());
                } else {
                    held.remove(&e.instance);
                }
            }
            held
        };
        let only = |t: i128| {
            let h = held(t);
            (h.len() == 1).then(|| h.into_iter().next().unwrap())
        };
        let quiet = !tokens.iter().any(|e| a < e.at && e.at <= b);
        let alone = quiet && held(a).len() == 1;
        (alone, only(a), only(b))
    }

    /// R pairs each two consecutive conforming samples of `resource`, with
    /// its presence, or, for R0, with none.
    fn pairs(&self, resource: &str, with_presence: bool) -> Vec<Judged> {
        let unit = |r: &Read| r.checked.unit.as_ref().map(|(u, _)| u.clone());
        let reads: Vec<Read> = self
            .reads(resource)
            .into_iter()
            .filter(Read::conforms)
            .collect();
        reads
            .windows(2)
            .map(|w| {
                let (a, b) = (&w[0], &w[1]);
                let (alone, earlier, later) = self.presence(a.at, b.at);
                let presence = Presence {
                    alone_throughout: alone,
                    earlier: earlier.as_deref(),
                    later: later.as_deref(),
                };
                let pair = tm::pair(
                    &sampled(a),
                    &sampled(b),
                    unit(b).as_deref(),
                    with_presence.then_some(&presence),
                );
                Judged {
                    earlier: a.clone(),
                    later: b.clone(),
                    pair,
                }
            })
            .collect()
    }

    /// R's pairs of a counter member, by value.
    fn counter_pairs(&self, resource: &str, with_presence: bool) -> Vec<(u64, u64, Judged)> {
        self.pairs(resource, with_presence)
            .into_iter()
            .map(|j| {
                let (a, b) = j.counters();
                (a, b, j)
            })
            .collect()
    }
}

/// The pair `a → b` of `pairs`.
fn find(pairs: &[(u64, u64, Judged)], a: u64, b: u64) -> &Judged {
    &pairs
        .iter()
        .find(|(x, y, _)| (*x, *y) == (a, b))
        .unwrap_or_else(|| panic!("no pair {a} → {b} in {pairs:#?}"))
        .2
}

/// §2.11's rate of `d` over two Points' timestamps.
fn by_timestamps(j: &Judged, d: u64) -> f64 {
    let t = |r: &Read| r.checked.reading.as_ref().unwrap().timestamp_ms.unwrap();
    tm::rate_per_s(
        d,
        (i128::from(t(&j.later)) - i128::from(t(&j.earlier))) * 1_000_000,
    )
}

/// §2.11's rate of `d` over the two arrivals.
fn by_arrivals(j: &Judged, d: u64) -> f64 {
    tm::rate_per_s(d, j.later.at - j.earlier.at)
}

fn no_decrease(pairs: &[Judged]) {
    for j in pairs {
        assert_ne!(j.pair.continuity, Continuity::Decreased, "{j:#?}");
        if let Ok(r) = &j.pair.rate {
            assert!(r.per_s >= 0.0, "no negative rate: {j:#?}");
        }
    }
}

// ---------------------------------------------------------------------------
// The sections
// ---------------------------------------------------------------------------

/// §1: a counter across a restart reads as a reset, established in-band
/// for the Point and by presence for the bare counter, with no rate; within
/// each run, the same epoch with rates.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_counter_across_a_restart() {
    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let r = reader(&rs).await;

    // 1. I1: frames 0, 10, 20 and drops 0, 5, 9, 1 s apart.
    let o = start(&ep).await;
    let i1 = o.instance();
    r.token(&i1, true).await;
    for (f, d) in [(0, 0), (10, 5), (20, 9)] {
        o.frames(f).await;
        o.count("drops", d).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    r.wait("frames", 3).await;
    r.wait("drops", 3).await;

    // 2. The process ends; R sees I1 withdrawn.
    o.end().await;
    r.token(&i1, false).await;

    // 3. I2: frames 0, 4 and drops 0, 2, 1 s apart.
    let o = start(&ep).await;
    let i2 = o.instance();
    assert_ne!(i1, i2);
    r.token(&i2, true).await;
    for (i, (f, d)) in [(0, 0), (4, 2)].into_iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        o.frames(f).await;
        o.count("drops", d).await;
    }
    let frames = r.wait("frames", 5).await;
    let drops = r.wait("drops", 5).await;

    // 1. Every sample conforms, each Point carrying its instance's epoch.
    assert!(frames.iter().chain(&drops).all(Read::conforms));
    let epochs: Vec<_> = frames.iter().map(|x| x.epoch().unwrap()).collect();
    assert_eq!(epochs, [&i1, &i1, &i1, &i2, &i2]);
    assert!(drops.iter().all(|x| x.epoch().is_none()), "drops has none");

    let f = r.counter_pairs("frames", true);
    let d = r.counter_pairs("drops", true);
    // 2. Within step 1: frames by its timestamps, in 1/s; drops by
    // presence, its rate by the arrivals, with no unit.
    for (a, b) in [(0, 10), (10, 20)] {
        let j = find(&f, a, b);
        assert_eq!(j.verdict(), (Continuity::SameEpoch, IN_BAND));
        let rate = j.pair.rate.as_ref().unwrap();
        assert_eq!(rate.per_s.to_bits(), by_timestamps(j, 10).to_bits());
        assert_eq!(rate.unit.as_deref(), Some("1/s"));
    }
    for (a, b) in [(0, 5), (5, 9)] {
        let j = find(&d, a, b);
        assert_eq!(j.verdict(), (Continuity::SameEpoch, BY_PRESENCE));
        assert_eq!(
            j.pair.rate,
            Ok(Rate {
                per_s: by_arrivals(j, b - a),
                unit: None
            })
        );
    }
    // 3. Across the restart: a reset, neither with a rate.
    let j = find(&f, 20, 0);
    assert_eq!(j.verdict(), (Continuity::Reset, IN_BAND));
    assert_eq!(j.pair.rate, Err(NoRate::Reset));
    let j = find(&d, 9, 0);
    assert_eq!(j.verdict(), (Continuity::Reset, BY_PRESENCE));
    assert_eq!(j.pair.rate, Err(NoRate::Reset));
    // 4. Within step 3: the same epoch again, with rates.
    let j = find(&f, 0, 4);
    assert_eq!(j.verdict(), (Continuity::SameEpoch, IN_BAND));
    assert!(j.pair.rate.is_ok());
    let j = find(&d, 0, 2);
    assert_eq!(j.verdict(), (Continuity::SameEpoch, BY_PRESENCE));
    assert!(j.pair.rate.is_ok());
    // 5. Nothing decreased, and no rate across the restart.
    let all: Vec<Judged> = f.into_iter().chain(d).map(|x| x.2).collect();
    no_decrease(&all);
    assert_eq!(all.iter().filter(|j| j.pair.rate.is_ok()).count(), 6);
    o.end().await;
}

/// §2: a counter across `new_epoch` in one process: the Point carries the
/// new instance at once, and both pair a reset across it; R0 reads the
/// bare one's reset as assumed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_counter_across_new_epoch_in_one_process() {
    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let r = reader(&rs).await;

    // 1. I1: frames 0, 10, 20 and drops 0, 5, 9.
    let mut o = start(&ep).await;
    let i1 = o.instance();
    r.token(&i1, true).await;
    for (f, d) in [(0, 0), (10, 5), (20, 9)] {
        o.frames(f).await;
        o.count("drops", d).await;
        tokio::time::sleep(SPACING).await;
    }
    r.wait("frames", 3).await;
    r.wait("drops", 3).await;

    // 2. A new epoch, make-before-break: R sees I2 appear, then I1 go.
    let i2 = o.svc.new_epoch().await.unwrap().to_string();
    assert_ne!(i1, i2);
    assert_eq!(o.w("frames").epoch().unwrap().as_str(), i2);
    r.token(&i2, true).await;
    r.token(&i1, false).await;
    let t = r.tokens();
    let (up2, gone1) = (
        t.iter().find(|e| e.instance == i2 && e.up).unwrap().at,
        t.iter().find(|e| e.instance == i1 && !e.up).unwrap().at,
    );
    assert!(up2 <= gone1, "make-before-break: {t:?}");

    // 3. 500 ms after I1's withdrawal: frames 0, 3 and drops 0, 3.
    for (f, d) in [(0, 0), (3, 3)] {
        o.frames(f).await;
        o.count("drops", d).await;
        tokio::time::sleep(SPACING).await;
    }
    let frames = r.wait("frames", 5).await;
    r.wait("drops", 5).await;

    // 1. Every Point after step 2 carries I2, the process unchanged.
    let epochs: Vec<_> = frames.iter().map(|x| x.epoch().unwrap()).collect();
    assert_eq!(epochs, [&i1, &i1, &i1, &i2, &i2]);
    let f = r.counter_pairs("frames", true);
    let d = r.counter_pairs("drops", true);
    // 2. A reset across it, neither with a rate.
    let j = find(&f, 20, 0);
    assert_eq!(j.verdict(), (Continuity::Reset, IN_BAND));
    assert_eq!(j.pair.rate, Err(NoRate::Reset));
    let j = find(&d, 9, 0);
    assert_eq!(j.verdict(), (Continuity::Reset, BY_PRESENCE));
    assert_eq!(j.pair.rate, Err(NoRate::Reset));
    // 3. Then the same epoch, established, with a rate.
    let j = find(&f, 0, 3);
    assert_eq!(j.verdict(), (Continuity::SameEpoch, IN_BAND));
    assert!(j.pair.rate.is_ok());
    let j = find(&d, 0, 3);
    assert_eq!(j.verdict(), (Continuity::SameEpoch, BY_PRESENCE));
    assert!(j.pair.rate.is_ok());
    // 4. R0: the same answer without the evidence.
    let d0 = r.counter_pairs("drops", false);
    let j = find(&d0, 9, 0);
    assert_eq!(j.verdict(), (Continuity::Reset, ASSUMED));
    assert_eq!(j.pair.rate, Err(NoRate::Reset));
    no_decrease(&f.into_iter().chain(d).map(|x| x.2).collect::<Vec<_>>());
    o.end().await;
}

/// §3: a decrease within one epoch is the finding, by the Point's epoch or
/// by presence; R0 reads the bare one's as a reset; the writer refuses both.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_a_decrease_within_one_epoch() {
    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let r = reader(&rs).await;

    // 1. I1: frames 10, drops 10.
    let o = start(&ep).await;
    let i1 = o.instance();
    r.token(&i1, true).await;
    o.frames(10).await;
    o.count("drops", 10).await;
    tokio::time::sleep(SPACING).await;

    // 2. Raw: frames 7, a Point carrying I1, and drops 7. 4. The writer
    // refuses both first.
    let e = o.w("frames").put_value(&telemetry::point(7u64)).await;
    let Reason::Decreased {
        epoch,
        before,
        after,
    } = refusal(e.unwrap_err())
    else {
        panic!("not a decrease");
    };
    assert_eq!(
        (epoch.as_str(), *before, *after),
        (i1.as_str(), Measure::Counter(10), Measure::Counter(7))
    );
    let e = o.w("drops").put_value(&7u64).await.unwrap_err();
    assert!(matches!(refusal(e), Reason::Decreased { .. }));
    o.raw("frames", &point_json(7u64, Some(&i1)), None).await;
    o.raw("drops", &serde_json::json!(7), None).await;
    let frames = r.wait("frames", 2).await;
    let drops = r.wait("drops", 2).await;
    assert_eq!(frames.len(), 2, "the refused put published nothing");
    assert_eq!(drops.len(), 2);

    // 1. Every sample conforms: conformance is per sample.
    assert!(frames.iter().chain(&drops).all(Read::conforms));
    // 2. R: decreased, established in-band and by presence; no rate.
    let j = find(&r.counter_pairs("frames", true), 10, 7).clone();
    assert_eq!(j.verdict(), (Continuity::Decreased, IN_BAND));
    assert_eq!(j.pair.rate, Err(NoRate::Decreased));
    let j = find(&r.counter_pairs("drops", true), 10, 7).clone();
    assert_eq!(j.verdict(), (Continuity::Decreased, BY_PRESENCE));
    assert_eq!(j.pair.rate, Err(NoRate::Decreased));
    // 3. R0: the bare one is a reset, assumed; the Point's epoch rides it.
    let j = find(&r.counter_pairs("drops", false), 10, 7).clone();
    assert_eq!(j.verdict(), (Continuity::Reset, ASSUMED));
    assert_eq!(j.pair.rate, Err(NoRate::Reset));
    let j = find(&r.counter_pairs("frames", false), 10, 7).clone();
    assert_eq!(j.verdict(), (Continuity::Decreased, IN_BAND));
    o.end().await;
}

/// §4: a Point whose tag or unit disagrees, and a gauge that is not a
/// number, do not conform and pair with nothing; the writer refuses each.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_a_point_whose_tag_disagrees() {
    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let r = reader(&rs).await;

    // 1. I1: frames 1.
    let o = start(&ep).await;
    r.token(&o.instance(), true).await;
    o.frames(1).await;
    tokio::time::sleep(SPACING).await;

    // 2.–4. Raw, each refused by the writer first.
    let gauge = telemetry::point(3.5);
    let e = o.w("frames").put_value(&gauge).await.unwrap_err();
    assert_eq!(refusal(e).problems(), [Problem::TagDisagrees]);
    o.raw("frames", &serde_json::to_value(&gauge).unwrap(), None)
        .await;
    tokio::time::sleep(SPACING).await;
    let bytes = telemetry::point(2u64).with_unit("By");
    let e = o.w("frames").put_value(&bytes).await.unwrap_err();
    assert_eq!(refusal(e).problems(), [Problem::UnitDisagrees]);
    o.raw("frames", &serde_json::to_value(&bytes).unwrap(), None)
        .await;
    tokio::time::sleep(SPACING).await;
    let e = o.w("level").put_value(&"x").await.unwrap_err();
    assert_eq!(refusal(e).problems(), [Problem::WrongType]);
    o.raw("level", &serde_json::json!("x"), None).await;
    tokio::time::sleep(SPACING).await;

    // 5. frames 5.
    o.frames(5).await;
    let frames = r.wait("frames", 4).await;
    let level = r.wait("level", 1).await;

    // 1. Step 2: tag_disagrees, the member's kind the declared counter.
    assert_eq!(frames[1].reasons(), [Problem::TagDisagrees]);
    assert_eq!(
        frames[1].checked.kind,
        Some((tm::MetricKind::Counter, Source::Declared))
    );
    // 2. Step 3: unit_disagrees, the member's unit the declared 1.
    assert_eq!(frames[2].reasons(), [Problem::UnitDisagrees]);
    assert_eq!(
        frames[2].checked.unit,
        Some(("1".to_owned(), Source::Declared))
    );
    // 3. Step 4: wrong_type.
    assert_eq!(level[0].reasons(), [Problem::WrongType]);
    // 4. None of them pairs: 1 → 5, the same epoch.
    let f = r.counter_pairs("frames", true);
    assert_eq!(f.len(), 1, "{f:#?}");
    assert_eq!(
        find(&f, 1, 5).verdict(),
        (Continuity::SameEpoch, IN_BAND),
        "the Points carry the epoch"
    );
    assert!(r.pairs("level", true).is_empty());
    o.end().await;
}

/// §5: a histogram over other bounds does not conform; one whose counts
/// fell is the finding; `sum` may fall; no histogram has a rate; the
/// writer refuses the two raw steps.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_a_histogram_over_other_bounds_and_one_that_goes_backwards() {
    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let r = reader(&rs).await;
    let o = start(&ep).await;
    let i1 = o.instance();
    r.token(&i1, true).await;
    let h = |bounds: &[f64], counts: [u64; 4], sum: f64| Histogram {
        buckets: bounds.to_vec(),
        counts: counts.to_vec(),
        count: counts.iter().sum(),
        sum,
    };
    let latency = o.w("latency");
    assert_eq!(
        latency.telemetry().unwrap().buckets.as_deref(),
        Some(&BOUNDS[..])
    );

    // 1. Over the declared bounds.
    latency
        .put_value(&telemetry::point(h(&BOUNDS, [3, 10, 2, 1], 0.19)))
        .await
        .unwrap();
    tokio::time::sleep(SPACING).await;
    // 2. Raw: other bounds, refused by the writer first.
    let other = telemetry::point(h(&[0.005, 0.01, 0.05], [3, 10, 2, 1], 0.19));
    let e = latency.put_value(&other).await.unwrap_err();
    assert_eq!(refusal(e).problems(), [Problem::BoundsDiffer]);
    o.raw("latency", &point_json(other.value, Some(&i1)), None)
        .await;
    tokio::time::sleep(SPACING).await;
    // 3. Raw: a bucket's count and `count` fell, refused by the writer.
    let fell = telemetry::point(h(&BOUNDS, [3, 9, 2, 1], 0.18));
    let e = latency.put_value(&fell).await.unwrap_err();
    assert!(matches!(refusal(e), Reason::Decreased { .. }));
    o.raw("latency", &point_json(fell.value, Some(&i1)), None)
        .await;
    tokio::time::sleep(SPACING).await;
    // 4. Every count grew; `sum` fell, which it may.
    latency
        .put_value(&telemetry::point(h(&BOUNDS, [4, 12, 2, 1], -1.0)))
        .await
        .unwrap();
    let reads = r.wait("latency", 4).await;

    // 1. Steps 1, 3 and 4 conform; step 2 is bounds_differ.
    assert!(reads[0].conforms() && reads[2].conforms() && reads[3].conforms());
    assert_eq!(reads[1].reasons(), [Problem::BoundsDiffer]);
    assert!(
        reads
            .iter()
            .filter(|x| x.conforms())
            .all(|x| x.epoch() == Some(i1.as_str()))
    );
    let p = r.pairs("latency", true);
    assert_eq!(p.len(), 2);
    // 2. Step 1 with step 3: decreased, established in-band.
    assert_eq!(p[0].verdict(), (Continuity::Decreased, IN_BAND));
    // 3. Step 3 with step 4: the same epoch.
    assert_eq!(p[1].verdict(), (Continuity::SameEpoch, IN_BAND));
    // 4. No pair has a rate.
    assert!(p.iter().all(|j| j.pair.rate == Err(NoRate::Kind)));
    o.end().await;
}

/// §6: absence is not zero: after the service closes, no sample arrives
/// and none is read as 0; a member with no sample forms no pair.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s6_absence_is_not_zero() {
    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let r = reader(&rs).await;

    // 1. I1: level −70.0 once, and drops 5, 6, 7, 1 s apart.
    let o = start(&ep).await;
    r.token(&o.instance(), true).await;
    o.w("level").put_value(&-70.0f64).await.unwrap();
    for (i, n) in [5, 6, 7].into_iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        o.count("drops", n).await;
    }
    r.wait("drops", 3).await;
    r.wait("level", 1).await;

    // 2. O closes its service. 3. R waits 3 s.
    let Owner { session, svc, .. } = o;
    let i1 = svc.instance().to_string();
    let before = r.log.lock().unwrap().samples.len();
    svc.close().await.unwrap();
    r.token(&i1, false).await;
    tokio::time::sleep(Duration::from_secs(3)).await;

    // 1. R's last level is −70.0, and no step gave it another value.
    let level = r.reads("level");
    assert_eq!(level.len(), 1);
    assert_eq!(level[0].measure(), &Measure::Gauge(-70.0));
    // 2. drops 5 → 6 and 6 → 7, and nothing after 7: no rate of 0 from
    // the silence.
    let d = r.counter_pairs("drops", true);
    let got: Vec<(u64, u64)> = d.iter().map(|(a, b, _)| (*a, *b)).collect();
    assert_eq!(got, [(5, 6), (6, 7)]);
    assert!(
        d.iter()
            .all(|(_, _, j)| j.pair.rate.as_ref().is_ok_and(|r| r.per_s > 0.0))
    );
    // 3. Nothing new after step 2, so nothing read as 0.
    assert_eq!(r.log.lock().unwrap().samples.len(), before);
    assert!(r.t0.elapsed() > Duration::from_secs(5));
    session.close().await.unwrap();
}

/// §7: a bare counter's in-band epoch rides its attachment, which the
/// writer writes as its instance id; R0 pairs it established in-band; an
/// empty epoch does not conform and pairs with nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_an_in_band_epoch_on_a_bare_counter() {
    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let r = reader(&rs).await;

    // 1. I1: bytes 100, 200, each with I1 as its attachment.
    let mut o = start(&ep).await;
    let i1 = o.instance();
    r.token(&i1, true).await;
    for n in [100, 200] {
        o.count("bytes", n).await;
        tokio::time::sleep(SPACING).await;
    }
    // 2. A new epoch; 500 ms later, bytes 0, 50 with I2.
    let i2 = o.svc.new_epoch().await.unwrap().to_string();
    r.token(&i1, false).await;
    for n in [0, 50] {
        o.count("bytes", n).await;
        tokio::time::sleep(SPACING).await;
    }
    // 3. Raw: bytes 60, its attachment the empty string, which the writer
    // refuses first.
    let e = o
        .w("bytes")
        .put_with(serde_json::to_vec(&60).unwrap(), Some(b"\"\"".to_vec()))
        .await
        .unwrap_err();
    assert_eq!(refusal(e).problems(), [Problem::EpochInvalid]);
    let e = o
        .w("bytes")
        .put_with(serde_json::to_vec(&60).unwrap(), Some(b"\"i9\"".to_vec()))
        .await
        .unwrap_err();
    assert!(
        matches!(refusal(e), Reason::Epoch { given, current } if given == "i9" && current == i2),
        "an epoch other than the owner's"
    );
    o.raw("bytes", &serde_json::json!(60), Some("")).await;
    let reads = r.wait("bytes", 5).await;

    // 1. Each attachment of steps 1 and 2 is the instance id then.
    let attached: Vec<String> = reads[..4]
        .iter()
        .map(|x| serde_json::from_slice(x.attachment.as_deref().unwrap()).unwrap())
        .collect();
    assert_eq!(attached, [i1.clone(), i1.clone(), i2.clone(), i2.clone()]);
    assert_eq!(
        reads[0].attachment.as_deref().unwrap().len(),
        18,
        "an 18-byte JSON attachment (§2.14)"
    );
    // 2. R0, with no presence, pairs established in-band.
    let p = r.counter_pairs("bytes", false);
    let j = find(&p, 100, 200);
    assert_eq!(j.verdict(), (Continuity::SameEpoch, IN_BAND));
    assert_eq!(j.pair.rate.as_ref().unwrap().unit.as_deref(), Some("By/s"));
    assert_eq!(find(&p, 200, 0).verdict(), (Continuity::Reset, IN_BAND));
    assert_eq!(find(&p, 0, 50).verdict(), (Continuity::SameEpoch, IN_BAND));
    // 3. Step 3 is epoch_invalid, and pairs with nothing.
    assert_eq!(reads[4].reasons(), [Problem::EpochInvalid]);
    assert_eq!(p.len(), 3, "{p:#?}");
    o.end().await;
}

/// §8: a counter that crosses a face, read by R0 alone: the bare one's
/// decrease across a restart is a reset, assumed; the Point's is
/// established in-band, its epoch having crossed with it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s8_counter_across_a_face() {
    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let r = reader(&rs).await;

    // 1. I1: drops 10, 20 and frames 10, 20.
    let o = start(&ep).await;
    let i1 = o.instance();
    r.token(&i1, true).await;
    for n in [10, 20] {
        o.count("drops", n).await;
        o.frames(n).await;
        tokio::time::sleep(SPACING).await;
    }
    r.wait("drops", 2).await;
    r.wait("frames", 2).await;
    // 2. The process ends; I2: drops 3, 8 and frames 3, 8.
    o.end().await;
    r.token(&i1, false).await;
    let o = start(&ep).await;
    r.token(&o.instance(), true).await;
    for n in [3, 8] {
        o.count("drops", n).await;
        o.frames(n).await;
        tokio::time::sleep(SPACING).await;
    }
    r.wait("drops", 4).await;
    r.wait("frames", 4).await;

    // 1. R0's drops: every answer assumed.
    let d = r.counter_pairs("drops", false);
    let j = find(&d, 10, 20);
    assert_eq!(j.verdict(), (Continuity::SameEpoch, ASSUMED));
    assert!(j.pair.rate.is_ok());
    let j = find(&d, 20, 3);
    assert_eq!(j.verdict(), (Continuity::Reset, ASSUMED));
    assert_eq!(j.pair.rate, Err(NoRate::Reset));
    assert_eq!(find(&d, 3, 8).verdict(), (Continuity::SameEpoch, ASSUMED));
    // 2. R0's frames: the reset established in-band.
    let f = r.counter_pairs("frames", false);
    assert_eq!(find(&f, 20, 3).verdict(), (Continuity::Reset, IN_BAND));
    // 3. Nothing decreased, and no negative rate.
    no_decrease(&d.into_iter().chain(f).map(|x| x.2).collect::<Vec<_>>());
    o.end().await;
}

/// The writer's epoch, filled where the application leaves it out and
/// refused where it differs, on the wire as R reads it: a counter Point
/// gains `epoch` and keeps every other byte; a Point with another epoch,
/// and bytes that are not JSON, are refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_writer_writes_the_epoch() {
    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let r = reader(&rs).await;
    let o = start(&ep).await;
    let i1 = o.instance();
    r.token(&i1, true).await;
    let text = br#" {"timestamp": 1, "value": {"type": "counter", "value": 3}}"#;
    o.w("frames").put(text.to_vec()).await.unwrap();
    let e = o
        .w("frames")
        .put_value(&Point::new(2, 4u64).with_epoch("someone-else"))
        .await
        .unwrap_err();
    assert!(matches!(refusal(e), Reason::Epoch { .. }));
    let e = o.w("frames").put(b"not json".to_vec()).await.unwrap_err();
    assert!(matches!(refusal(e), Reason::Undecodable(_)));
    o.w("frames")
        .put_value(&Point::new(3, 5u64).with_epoch(i1.clone()))
        .await
        .unwrap();
    let frames = r.wait("frames", 2).await;
    let got = String::from_utf8(r.log.lock().unwrap().samples[0].payload.clone()).unwrap();
    assert_eq!(
        got,
        format!(
            r#" {{"epoch":"{i1}","timestamp": 1, "value": {{"type": "counter", "value": 3}}}}"#
        )
    );
    assert_eq!(frames[1].payload["epoch"], serde_json::json!(i1));
    o.end().await;
}

// ---------------------------------------------------------------------------
// The writer beyond the scenarios: state, a member's epoch, CBOR
// ---------------------------------------------------------------------------

/// A contract of the kinds `meter.v1` leaves out: a state counter Point, a
/// counter Point on the template that declares `epoch`, and a CBOR one.
fn gauges() -> zenkey::Implementation {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/contracts");
    let text = r#"
[interface]
name  = "gauges"
major = 1
uses  = ["telemetry.v1"]

[schemas]
jsonschema = ["../../../spec/profiles/telemetry/telemetry.v1.json"]

[resources.total]
kind        = "state"
type        = "json:telemetry.v1.Point"
annotations = { "telemetry.kind" = "counter" }

[resources."ports/{port}/rx"]
kind        = "stream"
type        = "json:telemetry.v1.Point"
params      = { port = "string" }
cardinality = 16
epoch       = "port"
annotations = { "telemetry.kind" = "counter", "telemetry.unit" = "By" }

[resources.packed]
kind        = "stream"
type        = "json:telemetry.v1.Point"
encoding    = "cbor"
annotations = { "telemetry.kind" = "counter" }
"#;
    let l = zenkey::model::contract::load_str(text, &dir, None);
    zenkey::Implementation::new(l.contract.unwrap_or_else(|| panic!("{}", l.report)))
}

fn gauges_id() -> IfaceId {
    "gauges.v1".parse().unwrap()
}

/// A state member's writer keeps its last value across a delete, since the
/// epoch does not move (§2.7, §2.10); a new epoch forgets it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_state_writers_last_value_outlives_a_delete_not_an_epoch() {
    let (_r1, ep) = router(None).await;
    let s = client(&ep).await;
    let mut b = ServiceBuilder::new(&s, config(SVC));
    b.implement(gauges()).unwrap();
    b.expose(&gauges_id(), "stream/ports/{port}/rx").unwrap();
    b.expose(&gauges_id(), "stream/packed").unwrap();
    let total = b
        .declare_state_writer(&gauges_id(), "state/total", &Bindings::new())
        .await
        .unwrap();
    assert_eq!(
        total.telemetry().and_then(|d| d.kind),
        Some(tm::MetricKind::Counter)
    );
    total.put_value(&telemetry::point(10u64)).await.unwrap();
    let mut svc = b.start().await.unwrap();
    total.delete().await.unwrap();
    let e = total.put_value(&telemetry::point(5u64)).await.unwrap_err();
    assert!(matches!(refusal(e), Reason::Decreased { .. }), "a delete");
    svc.new_epoch().await.unwrap();
    total.put_value(&telemetry::point(5u64)).await.unwrap();
    s.close().await.unwrap();
}

/// A member of the template that declares `epoch` writes its own epoch
/// (§2.9, core §8.1): the one its first write gave it, which its token
/// takes, then each cycle's; the instance id is not its epoch.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_member_writes_its_own_epoch() {
    let (_r1, ep) = router(None).await;
    let s = client(&ep).await;
    let mut b = ServiceBuilder::new(&s, config(SVC));
    b.implement(gauges()).unwrap();
    b.expose(&gauges_id(), "stream/ports/{port}/rx").unwrap();
    b.expose(&gauges_id(), "stream/packed").unwrap();
    b.serve_state(&gauges_id()).unwrap();
    b.expose(&gauges_id(), "state/total").unwrap();
    let mut svc = b.start().await.unwrap();
    let eth0: Bindings = [("port".to_owned(), vec!["eth0".to_owned()])].into();
    let w = svc
        .writer(&gauges_id(), "stream/ports/{port}/rx", &eth0)
        .await
        .unwrap();
    let first = w.epoch().unwrap();
    assert_ne!(&first, svc.instance(), "the member's, not the instance's");
    w.put_value(&telemetry::point(7u64)).await.unwrap();
    let token = svc.declare_member(&gauges_id(), "eth0").await.unwrap();
    assert_eq!(
        token, first,
        "the token takes the epoch the first write gave"
    );
    let e = w
        .put_value(&telemetry::point(8u64).with_epoch(svc.instance().to_string()))
        .await
        .unwrap_err();
    assert!(matches!(refusal(e), Reason::Epoch { .. }));
    let e = w.put_value(&telemetry::point(6u64)).await.unwrap_err();
    assert!(matches!(refusal(e), Reason::Decreased { .. }));
    let cycled = svc.cycle_member(&gauges_id(), "eth0").await.unwrap();
    assert_ne!(cycled, first);
    assert_eq!(w.epoch().unwrap(), cycled);
    w.put_value(&telemetry::point(0u64)).await.unwrap();
    svc.new_epoch().await.unwrap();
    assert_eq!(w.epoch().unwrap(), cycled, "the instance's epoch is not it");
    s.close().await.unwrap();
}

/// A CBOR Point takes its epoch as one more entry of its map, and R reads
/// it in-band.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cbor_point_takes_its_epoch() {
    let (_r1, ep) = router(None).await;
    let s = client(&ep).await;
    let rs = client(&ep).await;
    let got: Arc<Mutex<Vec<Vec<u8>>>> = Arc::default();
    let g = Arc::clone(&got);
    let _sub = rs
        .declare_subscriber(format!("zk2/{SVC}/gauges.v1/stream/packed"))
        .callback(move |x: Sample| g.lock().unwrap().push(x.payload().to_bytes().into_owned()))
        .await
        .unwrap();
    let mut b = ServiceBuilder::new(&s, config(SVC));
    b.implement(gauges()).unwrap();
    b.expose(&gauges_id(), "stream/ports/{port}/rx").unwrap();
    b.serve_state(&gauges_id()).unwrap();
    b.expose(&gauges_id(), "state/total").unwrap();
    let w = b
        .declare_writer(&gauges_id(), "stream/packed", &Bindings::new())
        .await
        .unwrap();
    let svc = b.start().await.unwrap();
    eventually("the subscriber is matched", || async {
        w.matching().await.unwrap()
    })
    .await;
    w.put_value(&Point::new(9, 4u64).with_label("dev", "a"))
        .await
        .unwrap();
    eventually("a sample", || async { !got.lock().unwrap().is_empty() }).await;
    let bytes = got.lock().unwrap()[0].clone();
    let p: Point = ciborium::from_reader(bytes.as_slice()).unwrap();
    assert_eq!(p.epoch.as_deref(), Some(svc.instance().as_str()));
    assert_eq!(
        (p.timestamp, p.value, p.labels.len()),
        (9, Value::Counter(4), 1)
    );
    let e = w.put(b"{}".to_vec()).await.unwrap_err();
    assert!(
        matches!(refusal(e), Reason::Undecodable(_)),
        "JSON is not CBOR"
    );
    s.close().await.unwrap();
}
