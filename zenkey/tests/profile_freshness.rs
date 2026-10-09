//! `spec/profiles/freshness/scenarios.md` §1–§5 (`freshness.v1`, #720),
//! one test per section, named after it. §6 is a tool's, and runs in
//! `zenctl/tests/live_zk2.rs`.
//!
//! Common setup: R1 is a router (timestamping on, a router's default); the
//! owner `lab/beacon` implements `beacon.v1` with every resource exposed,
//! as a client of R1; S and G are a tool's consumer on another client, S
//! subscribing and G GETting. Every judgement is `zenkey_model`'s, over the
//! observations the runtime's helpers make.

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{T, client, config, contract, eventually, imp, router};
use zenkey::consumer::{Consumer, Delivery, Subscription};
use zenkey::model::freshness::{ClockTrust, DEFAULT_DELTA, Judged, Reason, Verdict};
use zenkey::model::grammar::IfaceId;
use zenkey::model::template::Bindings;
use zenkey::state::{Current, StateGet, StateWriter};
use zenkey::{Service, ServiceBuilder};
use zenoh::sample::SampleKind;
use zenoh::time::Timestamp;

const STATUS: &str = "state/status";
const INTENT: &str = "state/intent";
const NOTE: &str = "state/note";
const LEVEL: &str = "stream/level";

/// The scenarios' jitter allowance on loopback.
const JITTER: Duration = Duration::from_millis(200);

fn beacon() -> IfaceId {
    "beacon.v1".parse().expect("an interface")
}

/// One delivery, as S records it.
#[derive(Debug, Clone)]
struct Rec {
    at: Instant,
    key: String,
    value: String,
    delete: bool,
    encoding: String,
    stamp: Option<Timestamp>,
}

type Log = Arc<Mutex<Vec<Rec>>>;

/// The owner's writers: one per state member, and the stream's.
struct Owner {
    svc: Service,
    status: Option<StateWriter>,
    intent: StateWriter,
    note: StateWriter,
    level: zenkey::writer::Writer,
}

/// Brings up `address` implementing `beacon.v1`, every resource exposed,
/// its clock shifted by `offset_ms` and watching `clock_reference` when
/// given.
async fn owner(
    s: &zenoh::Session,
    address: &str,
    offset_ms: i64,
    clock_reference: Option<&str>,
) -> Owner {
    let mut cfg = config(address);
    cfg.clock_reference = clock_reference.map(str::to_owned);
    let mut b = ServiceBuilder::new(s, cfg);
    b.implement(imp("beacon.v1")).unwrap();
    b.minter().simulate_offset(offset_ms);
    let none = Bindings::new();
    let status = b
        .declare_state_writer(&beacon(), STATUS, &none)
        .await
        .unwrap();
    let intent = b
        .declare_state_writer(&beacon(), INTENT, &none)
        .await
        .unwrap();
    let note = b
        .declare_state_writer(&beacon(), NOTE, &none)
        .await
        .unwrap();
    let level = b.declare_writer(&beacon(), LEVEL, &none).await.unwrap();
    let svc = b.start().await.unwrap();
    Owner {
        svc,
        status: Some(status),
        intent,
        note,
        level,
    }
}

/// A tool's consumer of `address`'s `beacon.v1`: S and G.
fn reader(s: &zenoh::Session, address: &str) -> Consumer {
    Consumer::for_tool(
        s,
        Arc::new(contract("beacon.v1")),
        &[address],
        &Default::default(),
    )
    .unwrap()
}

/// S on `resource`, recording every delivery.
async fn listen(c: &Consumer, resource: &str) -> (Subscription, Log) {
    let log: Log = Arc::default();
    let w = Arc::clone(&log);
    let sub = c
        .subscribe(resource, move |d: Delivery| {
            w.lock().unwrap().push(Rec {
                at: Instant::now(),
                key: d.sample.key_expr().as_str().to_owned(),
                value: String::from_utf8_lossy(&d.sample.payload().to_bytes()).into_owned(),
                delete: d.sample.kind() == SampleKind::Delete,
                encoding: d.sample.encoding().to_string(),
                stamp: d.sample.timestamp().copied(),
            });
        })
        .await
        .unwrap();
    (sub, log)
}

fn snapshot(log: &Log) -> Vec<Rec> {
    log.lock().unwrap().clone()
}

/// Waits until `w`'s publisher sees a subscriber: S is declared.
async fn matched(w: &zenkey::writer::Writer) {
    eventually("the subscriber is matched", || async {
        w.matching().await.unwrap()
    })
    .await;
}

/// Waits until the log holds at least `n` deliveries; returns them.
async fn at_least(log: &Log, n: usize) -> Vec<Rec> {
    eventually(&format!("{n} deliveries"), || async {
        log.lock().unwrap().len() >= n
    })
    .await;
    snapshot(log)
}

/// G's GET of one member: its one current reply.
async fn get_one(c: &Consumer, resource: &str) -> Current {
    match c.get(resource, None, T).await.unwrap() {
        StateGet::Answered(mut v) => {
            assert_eq!(v.len(), 1, "one member: {v:?}");
            v.remove(0)
        }
        StateGet::Silent => panic!("{resource}: the owner did not answer"),
    }
}

fn verdict(j: Judged) -> (Verdict, Reason) {
    (j.verdict, j.reason)
}

/// Sleeps until `at + d`.
async fn until(at: Instant, d: Duration) {
    tokio::time::sleep_until((at + d).into()).await;
}

/// §1: re-puts at least every ttl/2 with the value unchanged and a fresh
/// owner's stamp, which the GET answers carry; none without a horizon,
/// after a delete, after the writer closes, after the service closes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_the_re_put_cadence() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let mut o = owner(&os, "lab/beacon", 0, None).await;
    let owner_zid = os.zid().to_string();
    let c = reader(&rs, "lab/beacon");
    let (_s, log) = listen(&c, STATUS).await;
    let (_n, notes) = listen(&c, NOTE).await;
    let status = o.status.take().unwrap();
    matched(status.writer()).await;
    matched(o.note.writer()).await;
    assert_eq!(
        status.refresh_period(),
        Some(Duration::from_millis(950)),
        "95 % of ttl/2 (ttl 2 s)"
    );
    assert_eq!(o.note.refresh_period(), None, "no horizon, no re-put");
    assert_eq!(o.intent.refresh_period(), None, "ttl 0, no re-put");

    // 1. One put of `up`, then 5 s; 4. `note` once, alongside.
    let first = status.put("up").await.unwrap();
    o.note.put("n").await.unwrap();
    let t0 = Instant::now();
    until(t0, Duration::from_secs(5)).await;
    let ups = snapshot(&log);
    assert!(ups.len() >= 5, "the put and at least 4 re-puts: {ups:#?}");
    for w in ups.windows(2) {
        assert!(
            w[1].at - w[0].at <= Duration::from_secs(1) + JITTER,
            "no gap above ttl/2: {:?}",
            w[1].at - w[0].at
        );
        assert!(
            w[1].stamp.unwrap() > w[0].stamp.unwrap(),
            "each stamp above"
        );
    }
    for r in &ups {
        assert_eq!((r.value.as_str(), r.delete), ("up", false), "unchanged");
        assert_eq!(r.encoding, "text/plain");
        assert_eq!(
            r.stamp.unwrap().get_id().to_string(),
            owner_zid,
            "the owner's own stamp (S1)"
        );
    }
    assert_eq!(ups[0].stamp, Some(first), "the first is the put itself");
    assert_eq!(
        snapshot(&notes).len(),
        1,
        "4. a member with no horizon is not re-put"
    );

    // 2. `down`: the change starts a new interval, and `up` is gone.
    let change = status.put("down").await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let after = since(&log, change);
    assert!(after.len() >= 2, "the change and a re-put: {after:#?}");
    assert!(after.iter().all(|r| r.value == "down"), "{after:#?}");
    assert_eq!(after[0].stamp, Some(change));

    // 3. G's reply carries the latest re-put's stamp, not the change's.
    let reply = get_one(&c, STATUS).await;
    let stamp = reply.timestamp().expect("stamped (S2)");
    assert!(stamp > change, "a re-put's stamp, above the change's");
    eventually("the reply's stamp is a delivery's", || async {
        snapshot(&log).iter().any(|r| r.stamp == Some(stamp))
    })
    .await;

    // 5. A delete, then nothing.
    let deleted = status.delete().await.unwrap();
    tokio::time::sleep(Duration::from_millis(2200)).await;
    let tail = since(&log, deleted);
    assert_eq!(tail.len(), 1, "the delete alone: {tail:#?}");
    assert!(tail[0].delete && tail[0].stamp == Some(deleted));

    // 6. A put, then the writer closes: its puts hold the refresher off,
    // and it is due 0.95 s later, so nothing follows the put.
    let again = status.put("up").await.unwrap();
    drop(status);
    tokio::time::sleep(Duration::from_millis(2200)).await;
    let tail = since(&log, again);
    assert_eq!(
        tail.len(),
        1,
        "the put, and no re-put after the close: {tail:#?}"
    );
    let held = get_one(&c, STATUS).await;
    assert_eq!(
        held.timestamp(),
        Some(again),
        "the owner still answers the value, and nobody confirms it"
    );

    // 7. A writer of the running service, then the service closes.
    let w = o
        .svc
        .state_writer(&beacon(), STATUS, &Bindings::new())
        .await
        .unwrap();
    assert!(w.is_refreshing());
    let last = w.put("up").await.unwrap();
    o.svc.close().await.unwrap();
    tokio::time::sleep(Duration::from_millis(2200)).await;
    let tail = since(&log, last);
    assert!(
        !tail.is_empty() && tail.iter().all(|r| r.stamp == Some(last)),
        "no re-put once the service closed: {tail:#?}"
    );
    drop(w);
}

/// The deliveries stamped at or above `ts`: the owner's mutations from
/// `ts` on, whatever was in flight when it was minted.
fn since(log: &Log, ts: Timestamp) -> Vec<Rec> {
    snapshot(log)
        .into_iter()
        .filter(|r| r.stamp.is_some_and(|s| s >= ts))
        .collect()
}

/// §2: a writer closed, its service up: S and G judge `status` fresh, then
/// stale, while presence says the owner is present; an owner whose clock
/// guard holds its writes stops its re-puts, and goes stale.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_a_stopped_refresher_goes_stale() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let mut o = owner(&os, "lab/beacon", 0, None).await;
    let owner_zid = os.zid().to_string();
    let c = reader(&rs, "lab/beacon");
    let (sub, log) = listen(&c, STATUS).await;
    let status = o.status.take().unwrap();
    matched(status.writer()).await;

    // 1. The put, and re-puts G measures its clock from.
    status.put("up").await.unwrap();
    at_least(&log, 2).await;
    let trust = sub.clock_trust(&owner_zid, DEFAULT_DELTA);
    assert!(
        matches!(trust, ClockTrust::Trusted { .. }),
        "measured on the owner's own re-puts"
    );
    let key = snapshot(&log)[0].key.clone();
    assert_eq!(
        verdict(c.freshness(STATUS, &[sub.freshness(&key)]).unwrap()),
        (Verdict::Fresh, Reason::WithinHorizon)
    );

    // 2. The writer closes; the service stays up.
    drop(status);
    let closed = Instant::now();

    // 3. G at 0.2 s: fresh against its measured clock.
    until(closed, Duration::from_millis(200)).await;
    let reply = get_one(&c, STATUS).await;
    assert_eq!(
        verdict(c.freshness(STATUS, &[reply.freshness(trust)]).unwrap()),
        (Verdict::Fresh, Reason::WithinHorizon)
    );
    // S at 1 s, then 3 s after its last delivery.
    let last = snapshot(&log).last().unwrap().at;
    until(last, Duration::from_secs(1)).await;
    assert_eq!(
        verdict(c.freshness(STATUS, &[sub.freshness(&key)]).unwrap()),
        (Verdict::Fresh, Reason::WithinHorizon)
    );
    until(last, Duration::from_secs(3)).await;
    assert_eq!(
        verdict(c.freshness(STATUS, &[sub.freshness(&key)]).unwrap()),
        (Verdict::Stale, Reason::BeyondHorizon),
        "S: the last delivery more than 2 s old"
    );
    // G at 3.5 s after the close: above ttl + delta.
    until(closed, Duration::from_millis(3500)).await;
    let reply = get_one(&c, STATUS).await;
    assert_eq!(
        verdict(c.freshness(STATUS, &[reply.freshness(trust)]).unwrap()),
        (Verdict::Stale, Reason::BeyondHorizon)
    );

    // 4. Present, and stale: neither folded into the other.
    let present = c.present(T).await.unwrap();
    assert!(
        present.contains(&common::addr("lab/beacon")),
        "the owner holds its tokens: {present:?}"
    );

    // 5. Clock ahead: the guard holds the re-puts.
    let hb = client(&ep).await;
    let ahead_session = client(&ep).await;
    let a = reader(&rs, "lab/ahead");
    let (asub, alog) = listen(&a, STATUS).await;
    let mut ahead = owner(&ahead_session, "lab/ahead", 2000, Some("clock/heartbeat")).await;
    let astatus = ahead.status.take().unwrap();
    matched(astatus.writer()).await;
    astatus.put("up").await.unwrap();
    at_least(&alog, 1).await;
    eventually("the drift is detected", || async {
        hb.put("clock/heartbeat", "tick").await.unwrap();
        ahead.svc.minter().is_ahead()
    })
    .await;
    let detected = Instant::now();
    assert!(astatus.is_refreshing(), "on, and held by the guard");
    tokio::time::sleep(Duration::from_secs(3)).await;
    let late = snapshot(&alog)
        .into_iter()
        .filter(|r| r.at > detected + JITTER)
        .count();
    assert_eq!(late, 0, "no re-put while the guard holds writes");
    let akey = snapshot(&alog)[0].key.clone();
    assert_eq!(
        verdict(a.freshness(STATUS, &[asub.freshness(&akey)]).unwrap()),
        (Verdict::Stale, Reason::BeyondHorizon)
    );
    assert!(
        a.present(T)
            .await
            .unwrap()
            .contains(&common::addr("lab/ahead")),
        "present, and stale"
    );
}

/// §3: ttl 0 is never re-put, and never stale, with no clock needed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_never_stale() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let o = owner(&os, "lab/beacon", 0, None).await;
    let c = reader(&rs, "lab/beacon");
    let (sub, log) = listen(&c, INTENT).await;
    matched(o.intent.writer()).await;
    o.intent.put("i").await.unwrap();
    let put = Instant::now();
    until(put, Duration::from_secs(3)).await;
    let got = snapshot(&log);
    assert_eq!(got.len(), 1, "no re-put at ttl 0: {got:#?}");
    assert_eq!(
        verdict(c.freshness(INTENT, &[sub.freshness(&got[0].key)]).unwrap()),
        (Verdict::Fresh, Reason::NeverStale)
    );
    let reply = get_one(&c, INTENT).await;
    assert_eq!(
        verdict(
            c.freshness(INTENT, &[reply.freshness(ClockTrust::Untrusted)])
                .unwrap()
        ),
        (Verdict::Fresh, Reason::NeverStale),
        "no clock needed"
    );
}

/// §4: a reader whose clock disagrees with the owner's by 5 s: S stays
/// fresh on its receive clock; G's measurement fails, so its age is
/// unobservable; on the deployment's word it reads stale.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_a_skewed_get_reader() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let mut o = owner(&os, "lab/beacon", -5000, None).await;
    let owner_zid = os.zid().to_string();
    let c = reader(&rs, "lab/beacon");
    let (sub, log) = listen(&c, STATUS).await;
    let status = o.status.take().unwrap();
    matched(status.writer()).await;
    status.put("up").await.unwrap();
    let got = at_least(&log, 2).await;
    let key = got[0].key.clone();
    assert_eq!(
        verdict(c.freshness(STATUS, &[sub.freshness(&key)]).unwrap()),
        (Verdict::Fresh, Reason::WithinHorizon),
        "1. the receive clock is not skewed"
    );
    let trust = sub.clock_trust(&owner_zid, DEFAULT_DELTA);
    assert_eq!(trust, ClockTrust::Untrusted, "1. the measurement fails");
    let reply = get_one(&c, STATUS).await;
    assert_eq!(
        verdict(c.freshness(STATUS, &[reply.freshness(trust)]).unwrap()),
        (Verdict::Unobservable, Reason::ClockUntrusted),
        "2. never fresh, never stale"
    );
    let word = ClockTrust::Trusted {
        delta: DEFAULT_DELTA,
    };
    assert_eq!(
        verdict(c.freshness(STATUS, &[reply.freshness(word)]).unwrap()),
        (Verdict::Stale, Reason::BeyondHorizon),
        "3. the deployment's word, which the clocks do not keep"
    );
    drop(status);
}

/// §5: a stream sample every 200 ms is fresh; once the owner stops, it is
/// stale after ttl, and the runtime publishes nothing on its behalf.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_the_stream_case() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let o = owner(&os, "lab/beacon", 0, None).await;
    let c = reader(&rs, "lab/beacon");
    let (sub, log) = listen(&c, LEVEL).await;
    matched(&o.level).await;
    for n in 0..10 {
        o.level.put(format!("{n}")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let got = at_least(&log, 10).await;
    for w in got.windows(2) {
        assert!(
            w[1].at - w[0].at <= Duration::from_millis(500) + JITTER,
            "within the ttl/2 bound"
        );
    }
    let key = got[0].key.clone();
    let last = got.last().unwrap().at;
    until(last, Duration::from_millis(500)).await;
    assert_eq!(
        verdict(c.freshness(LEVEL, &[sub.freshness(&key)]).unwrap()),
        (Verdict::Fresh, Reason::WithinHorizon)
    );
    until(last, Duration::from_millis(1500)).await;
    assert_eq!(
        verdict(c.freshness(LEVEL, &[sub.freshness(&key)]).unwrap()),
        (Verdict::Stale, Reason::BeyondHorizon)
    );
    assert_eq!(snapshot(&log).len(), 10, "no sample on the owner's behalf");
    // A stream has no queryable: its freshness is a subscriber's only.
    assert!(
        c.get(LEVEL, None, T).await.is_err(),
        "a stream is not state"
    );
}
