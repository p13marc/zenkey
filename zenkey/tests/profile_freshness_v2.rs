//! `spec/profiles/freshness/scenarios-v2.md` §1–§7 (`freshness.v2`, #978's
//! F3), one test per section, named after it.
//!
//! Common setup: R1 is a router (timestamping on, a router's default); the
//! owner `lab/beacon` implements `beacon.v2` with every resource exposed
//! but `route` (gated on a capability it does not hold), as a client of
//! R1; H reads its heartbeats raw; S is a tool's consumer subscribing, G
//! the same consumer GETting. Every judgement is `zenkey_model`'s, over
//! the observations the runtime's helpers make.

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use common::{
    T, client, config, contract, eventually, imp, link_to, retrying_client, router, router_via,
    router_with,
};
use zenkey::consumer::{Confirmation, Consumer, Delivery, Subscription};
use zenkey::model::freshness::{
    ClockTrust, DEFAULT_DELTA, Horizon, Observation, Reason, Verdict, judge,
};
use zenkey::model::grammar::IfaceId;
use zenkey::model::heartbeat::{Digest, Heartbeat, Layout, Members, Time};
use zenkey::model::template::Bindings;
use zenkey::model::wire::{Envelope, Form, Stamp, Tag};
use zenkey::state::{StateGet, StateWriter};
use zenkey::{Service, ServiceBuilder};
use zenoh::sample::SampleKind;

const STATUS: &str = "state/status";
const PEER: &str = "state/peer/{peer}";
const LEVEL: &str = "stream/level";
const STATUS_KEY: &str = "zk2/lab/beacon/beacon.v2/state/status";
const HB_KEY: &str = "zk2/lab/beacon/freshness.v2/stream/heartbeat/beacon.v2";

/// The scenarios' jitter allowance on loopback.
const JITTER: Duration = Duration::from_millis(200);

fn beacon() -> IfaceId {
    "beacon.v2".parse().expect("an interface")
}

fn peer_key(p: &str) -> String {
    format!("zk2/lab/beacon/beacon.v2/state/peer/{p}")
}

fn layout() -> Layout {
    Layout::of(&contract("beacon.v2"))
}

fn bind(peer: &str) -> Bindings {
    [("peer".to_owned(), vec![peer.to_owned()])].into()
}

/// The owner's writers.
struct Owner {
    svc: Service,
    status: StateWriter,
    peers: BTreeMap<String, StateWriter>,
    _intent: StateWriter,
    _note: StateWriter,
    level: zenkey::writer::Writer,
}

/// Brings up `address` implementing `beacon.v2`, every resource exposed
/// but `route`, a writer for each of `peers`, its clock shifted by
/// `offset_ms` and watching `clock_reference` when given.
async fn owner(
    s: &zenoh::Session,
    address: &str,
    peers: &[&str],
    offset_ms: i64,
    clock_reference: Option<&str>,
) -> Owner {
    let mut cfg = config(address);
    cfg.clock_reference = clock_reference.map(str::to_owned);
    let mut b = ServiceBuilder::new(s, cfg);
    b.implement(imp("beacon.v2")).unwrap();
    b.minter().simulate_offset(offset_ms);
    let none = Bindings::new();
    let status = b
        .declare_state_writer(&beacon(), STATUS, &none)
        .await
        .unwrap();
    let mut by = BTreeMap::new();
    for p in peers {
        by.insert(
            (*p).to_owned(),
            b.declare_state_writer(&beacon(), PEER, &bind(p))
                .await
                .unwrap(),
        );
    }
    b.expose(&beacon(), PEER).unwrap();
    let intent = b
        .declare_state_writer(&beacon(), "state/intent", &none)
        .await
        .unwrap();
    let note = b
        .declare_state_writer(&beacon(), "state/note", &none)
        .await
        .unwrap();
    let level = b.declare_writer(&beacon(), LEVEL, &none).await.unwrap();
    let svc = b.start().await.unwrap();
    Owner {
        svc,
        status,
        peers: by,
        _intent: intent,
        _note: note,
        level,
    }
}

/// One heartbeat as H reads it.
#[derive(Debug, Clone)]
struct Beat {
    at: Instant,
    tag: Tag,
    stamp: Stamp,
    hb: Heartbeat,
}

type Beats = Arc<Mutex<Vec<Beat>>>;

/// H: a raw subscriber on `key`, each heartbeat read with `beacon.v2`'s
/// collections.
async fn heartbeats(s: &zenoh::Session, key: &str) -> (zenoh::pubsub::Subscriber<()>, Beats) {
    let beats: Beats = Arc::default();
    let w = Arc::clone(&beats);
    let l = layout();
    let sub = s
        .declare_subscriber(key)
        .callback(move |sample| {
            let at = Instant::now();
            let bytes = sample.payload().to_bytes();
            let e = Envelope::parse(&bytes).expect("an envelope");
            assert_eq!(e.form, Form::Heartbeat, "core §7.5's form 6");
            let hb = Heartbeat::parse(e.record, &l).expect("a body of beacon.v2's");
            w.lock().unwrap().push(Beat {
                at,
                tag: e.tag,
                stamp: e.stamp,
                hb,
            });
        })
        .await
        .unwrap();
    (sub, beats)
}

fn beats_after(b: &Beats, t: Instant) -> Vec<Beat> {
    b.lock()
        .unwrap()
        .iter()
        .filter(|x| x.at > t)
        .cloned()
        .collect()
}

/// One delivery, as S records it.
#[derive(Debug, Clone)]
struct Rec {
    stamp: Option<zenoh::time::Timestamp>,
    inband: Option<Stamp>,
}

type Log = Arc<Mutex<Vec<Rec>>>;
type Confirmed = Arc<Mutex<Vec<Confirmation>>>;

/// A tool's consumer of `address`'s `beacon.v2`: S and G.
fn reader(s: &zenoh::Session, address: &str) -> Consumer {
    Consumer::for_tool(
        s,
        Arc::new(contract("beacon.v2")),
        &[address],
        &Default::default(),
    )
    .unwrap()
    .with_reread_timeout(T)
}

/// S on `resource`, recording every delivery and every confirmation.
async fn listen(c: &Consumer, resource: &str) -> (Subscription, Log, Confirmed) {
    let log: Log = Arc::default();
    let confirmed: Confirmed = Arc::default();
    let (w, k) = (Arc::clone(&log), Arc::clone(&confirmed));
    let sub = c
        .subscribe_confirmed(
            resource,
            move |d: Delivery| {
                w.lock().unwrap().push(Rec {
                    stamp: d.sample.timestamp().copied(),
                    inband: d.inband,
                });
            },
            move |x: Confirmation| k.lock().unwrap().push(x),
        )
        .await
        .unwrap();
    (sub, log, confirmed)
}

/// Waits until `w`'s publisher sees a subscriber.
async fn matched(w: &zenkey::writer::Writer) {
    eventually("the subscriber is matched", || async {
        w.matching().await.unwrap()
    })
    .await;
}

async fn until(at: Instant, d: Duration) {
    tokio::time::sleep_until((at + d).into()).await;
}

fn verdict(h: &Horizon, o: &Observation) -> (Verdict, Reason) {
    let j = judge(h, o);
    (j.verdict, j.reason)
}

fn ttl(s: u64) -> Horizon {
    Horizon::Within(Duration::from_secs(s))
}

/// §1: a heartbeat at least every ttl/2 from the tokens on, form 6 under
/// beacon.v2's tag, its map `1 0 1`, its digests over what the owner
/// holds, each bit while the owner vouches; no re-put; none after the
/// close.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_the_cadence_and_the_bits() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let (_h, beats) = heartbeats(&rs, HB_KEY).await;
    let c = reader(&rs, "lab/beacon");
    let (status_sub, status_log, _) = listen(&c, STATUS).await;
    let (_peer_sub, peer_log, _) = listen(&c, PEER).await;
    let o = owner(&os, "lab/beacon", &["a", "b"], 0, None).await;
    let l = layout();
    assert_eq!(
        l.collections
            .iter()
            .map(|c| c.resource())
            .collect::<Vec<_>>(),
        ["state/peer/{peer}", "state/route", "state/status"],
        "beacon.v2's collections, in map order"
    );
    let descriptor: serde_json::Value = serde_json::from_slice(&o.svc.descriptor_bytes()).unwrap();
    let listed = descriptor.to_string();
    assert!(
        listed.contains("\"freshness.v2\"") && listed.contains("\"beacon.v2\""),
        "the descriptor lists beacon.v2 and freshness.v2: {descriptor}"
    );
    matched(o.status.writer()).await;
    matched(o.peers["a"].writer()).await;

    // 1. The puts, then 5 s with nothing else.
    let s_status = o.status.put("up").await.unwrap().stamp;
    let s_a = o.peers["a"].put("1").await.unwrap().stamp;
    let s_b = o.peers["b"].put("1").await.unwrap().stamp;
    let t0 = Instant::now();
    until(t0, Duration::from_secs(5)).await;
    let got = beats_after(&beats, t0);
    assert!(
        got.len() >= 5,
        "a heartbeat at least every 1 s: {}",
        got.len()
    );
    for w in got.windows(2) {
        assert!(
            w[1].at.duration_since(w[0].at) <= Duration::from_secs(1) + JITTER,
            "at most ttl/2 apart, plus jitter"
        );
        assert_eq!(w[0].stamp.run, w[1].stamp.run, "one run");
        assert_eq!(
            w[1].stamp.seq.get(),
            w[0].stamp.seq.get() + 1,
            "consecutive seqs"
        );
    }
    let last = got.last().unwrap();
    assert_eq!(last.tag, l.tag, "beacon.v2's tag");
    let s = &last.hb.sections;
    assert!(
        s[0].is_some() && s[1].is_none() && s[2].is_some(),
        "the map 1 0 1"
    );
    let status = s[2].as_ref().unwrap();
    assert_eq!(status.digest, Digest::of([(STATUS_KEY, s_status)]));
    assert_eq!(status.members, Members::One { vouched: true });
    let peers = s[0].as_ref().unwrap();
    let (ka, kb) = (peer_key("a"), peer_key("b"));
    assert_eq!(
        peers.digest,
        Digest::of([(ka.as_str(), s_a), (kb.as_str(), s_b)])
    );
    assert_eq!(peers.members.live(), Some(2));
    assert_eq!(peers.members.vouched(), [0, 1]);
    // No re-put: each put heard once.
    let ups: Vec<Rec> = status_log.lock().unwrap().clone();
    assert_eq!(ups.len(), 1, "the put alone: {ups:#?}");
    assert_eq!(peer_log.lock().unwrap().len(), 2, "the two puts alone");
    assert!(!o.status.is_refreshing(), "no refresher under freshness.v2");
    assert!(o.status.is_covered());
    assert_eq!(
        verdict(&ttl(2), &status_sub.freshness(STATUS_KEY)),
        (Verdict::Fresh, Reason::WithinHorizon),
        "5 s after its put, the heartbeats confirmed it"
    );

    // 2. peer/b's bit cleared.
    o.peers["b"].set_vouched(false);
    let t1 = Instant::now();
    until(t1, Duration::from_secs(2)).await;
    let after = beats_after(&beats, t1 + JITTER);
    let p = after.last().unwrap().hb.sections[0].clone().unwrap();
    assert_eq!(p.members.live(), Some(2), "peer/b still live");
    assert_eq!(p.members.vouched(), [0], "only peer/a vouched for");

    // 3. peer/a deleted.
    let mut o = o;
    let a = o.peers.remove("a").unwrap();
    a.delete().await.unwrap();
    let t2 = Instant::now();
    until(t2, Duration::from_secs(2)).await;
    let after = beats_after(&beats, t2 + JITTER);
    let p = after.last().unwrap().hb.sections[0].clone().unwrap();
    assert_eq!(p.members.live(), Some(1), "peer/b alone");
    assert_eq!(p.digest, Digest::of([(kb.as_str(), s_b)]));
    assert!(p.members.vouched().is_empty(), "its bit is still clear");

    // 4. The close: no heartbeat after the tokens went.
    o.svc.close().await.unwrap();
    let closed = Instant::now();
    until(closed, Duration::from_secs(2)).await;
    assert!(
        beats_after(&beats, closed + JITTER).is_empty(),
        "no heartbeat after the close"
    );
}

/// §2: a cleared bit is no confirmation, not stale at once; set again, the
/// next heartbeat confirms; the clock guard holds every bit and the time;
/// a silent heartbeat goes stale, never "down".
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_a_vouch_withdrawn_goes_stale() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let c = reader(&rs, "lab/beacon");
    let (sub, _log, _) = listen(&c, STATUS).await;
    let o = owner(&os, "lab/beacon", &[], 0, None).await;
    matched(o.status.writer()).await;
    let h = ttl(2);
    let judged = || verdict(&h, &sub.freshness(STATUS_KEY));

    // 1. One put; fresh at 3 s.
    o.status.put("up").await.unwrap();
    let t0 = Instant::now();
    until(t0, Duration::from_secs(3)).await;
    assert_eq!(judged(), (Verdict::Fresh, Reason::WithinHorizon));

    // 2. The bit cleared: fresh just after, stale 2 s after the last
    // heartbeat that vouched for it.
    o.status.set_vouched(false);
    let cleared = Instant::now();
    until(cleared, Duration::from_millis(300)).await;
    assert_eq!(judged().0, Verdict::Fresh, "never stale at once");
    until(cleared, Duration::from_secs(4)).await;
    let last = sub.confirmed(STATUS_KEY).expect("a confirmation before");
    assert!(last <= cleared + JITTER, "none after the bit cleared");
    assert_eq!(judged(), (Verdict::Stale, Reason::BeyondHorizon));

    // 3. Set again: fresh from the next heartbeat.
    o.status.set_vouched(true);
    eventually("the next heartbeat confirms it", || async {
        judged().0 == Verdict::Fresh
    })
    .await;

    // 4. A clock ahead: the heartbeats go on, every bit clear and the time
    // unknown.
    let tick = client(&ep).await;
    let ahead_session = client(&ep).await;
    let ahead_key = "zk2/lab/ahead/freshness.v2/stream/heartbeat/beacon.v2";
    let (_h2, beats) = heartbeats(&rs, ahead_key).await;
    let a = reader(&rs, "lab/ahead");
    let (asub, _alog, _) = listen(&a, STATUS).await;
    let ahead = owner(
        &ahead_session,
        "lab/ahead",
        &[],
        2000,
        Some("clock/heartbeat"),
    )
    .await;
    matched(ahead.status.writer()).await;
    ahead.status.put("up").await.unwrap();
    eventually("the drift is detected", || async {
        tick.put("clock/heartbeat", "tick").await.unwrap();
        ahead.svc.minter().is_ahead()
    })
    .await;
    let detected = Instant::now();
    until(detected, Duration::from_secs(4)).await;
    let held = beats_after(&beats, detected + Duration::from_millis(1200));
    assert!(held.len() >= 2, "heartbeats go on: {}", held.len());
    for b in &held {
        assert_eq!(b.hb.time, Time::Unknown, "the time unknown while held");
        let s = b.hb.sections[2].as_ref().expect("status's section");
        assert_eq!(
            s.members,
            Members::One { vouched: false },
            "every bit clear"
        );
    }
    assert_eq!(
        verdict(&h, &asub.freshness("zk2/lab/ahead/beacon.v2/state/status")),
        (Verdict::Stale, Reason::BeyondHorizon)
    );

    // 5. The first owner closes: its heartbeat falls silent, and S judges
    // stale, which says nothing of presence.
    o.svc.close().await.unwrap();
    let closed = Instant::now();
    until(closed, Duration::from_secs(3)).await;
    assert_eq!(judged(), (Verdict::Stale, Reason::BeyondHorizon));
}

/// §3: the put carries no owner's Zenoh timestamp; the reply carries the
/// last confirmation, which ages it for a GET reader.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_a_get_readers_age_from_the_confirmation_stamp() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let c = reader(&rs, "lab/beacon");
    let (_sub, log, _) = listen(&c, STATUS).await;
    let o = owner(&os, "lab/beacon", &[], 0, None).await;
    let owner_zid = os.zid().to_string();
    matched(o.status.writer()).await;
    let word = ClockTrust::Trusted {
        delta: DEFAULT_DELTA,
    };
    let get = || async {
        match c.get(STATUS, None, T).await.unwrap() {
            StateGet::Answered(mut v) => v.remove(0),
            StateGet::Silent => panic!("the owner did not answer"),
        }
    };

    // 1. The put; G at 3 s.
    let put_wall = SystemTime::now();
    let put = o.status.put("up").await.unwrap();
    assert!(
        put.time.is_none(),
        "no Zenoh timestamp on a freshness.v2 put"
    );
    let t0 = Instant::now();
    eventually("S hears the put", || async {
        !log.lock().unwrap().is_empty()
    })
    .await;
    let heard = log.lock().unwrap()[0].clone();
    assert!(
        heard
            .stamp
            .is_none_or(|t| t.get_id().to_string() != owner_zid),
        "a router's stamp, if any: {:?}",
        heard.stamp
    );
    assert!(heard.inband.is_some(), "the in-band stamp");
    until(t0, Duration::from_secs(3)).await;
    let reply = get().await;
    let ts = reply.timestamp().expect("the last confirmation");
    assert_eq!(ts.get_id().to_string(), owner_zid, "the owner's clock");
    let at = ts.get_time().to_system_time();
    assert!(at > put_wall, "later than the put");
    let age = SystemTime::now().duration_since(at).unwrap_or_default();
    assert!(
        age <= Duration::from_secs(1) + JITTER,
        "the last heartbeat's: {age:?}"
    );
    assert_eq!(
        c.freshness(STATUS, &[reply.freshness(word)])
            .unwrap()
            .verdict,
        Verdict::Fresh,
        "3 s after its put"
    );

    // 2. The bit cleared; 3.5 s later, stale.
    o.status.set_vouched(false);
    let cleared = Instant::now();
    until(cleared, Duration::from_millis(3500)).await;
    let reply = get().await;
    let j = c.freshness(STATUS, &[reply.freshness(word)]).unwrap();
    assert_eq!(
        (j.verdict, j.reason),
        (Verdict::Stale, Reason::BeyondHorizon)
    );
}

/// §4: a change lost behind a cut link: a heartbeat lost, then
/// divergences, one re-read, and confirmations again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_a_lost_change_detected_and_re_read() {
    let (_r1, ep1) = router(None).await;
    let link = link_to(&ep1).await;
    let (_r2, ep2) = router_via(&link).await;
    let os = client(&ep1).await;
    let rs = retrying_client(&ep2).await;
    let c = reader(&rs, "lab/beacon");
    let (sub, log, confirmed) = listen(&c, PEER).await;
    let o = owner(&os, "lab/beacon", &["a", "b"], 0, None).await;
    matched(o.peers["a"].writer()).await;
    let (ka, kb) = (peer_key("a"), peer_key("b"));

    // 1. Both put and confirmed.
    o.peers["a"].put("1").await.unwrap();
    o.peers["b"].put("1").await.unwrap();
    eventually("both are confirmed", || async {
        sub.confirmed(&ka).is_some() && sub.confirmed(&kb).is_some()
    })
    .await;
    assert_eq!(log.lock().unwrap().len(), 2);

    // 2. The cut, the lost change, the heal.
    link.cut();
    let cut = Instant::now();
    tokio::time::sleep(Duration::from_millis(300)).await;
    o.peers["a"].put("2").await.unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    link.heal();

    // 3. A heartbeat lost: unobservable until one in sequence.
    eventually("a heartbeat lost reads incomplete", || async {
        matches!(
            sub.freshness(&ka),
            Observation::Subscribed {
                complete: false,
                ..
            }
        )
    })
    .await;
    // The divergences, one re-read, and a confirmation of both.
    eventually("one re-read", || async { sub.rereads() == 1 }).await;
    let reread = sub.last_reread().unwrap();
    eventually("both confirmed after the re-read", || async {
        confirmed
            .lock()
            .unwrap()
            .iter()
            .any(|x| x.arrival > reread && x.keys == [ka.clone(), kb.clone()])
    })
    .await;
    assert!(
        !confirmed
            .lock()
            .unwrap()
            .iter()
            .any(|x| x.arrival > cut && x.arrival < reread && !x.keys.is_empty()),
        "no heartbeat confirms peer/{{peer}} between the loss and the re-read"
    );
    assert_eq!(
        log.lock().unwrap().len(),
        2,
        "the lost change was never delivered"
    );
    assert_eq!(sub.rereads(), 1, "once");
    assert!(matches!(
        sub.freshness(&ka),
        Observation::Subscribed { complete: true, .. }
    ));
}

/// §5: a late joiner seeded by its own S4 GET confirms at its first
/// heartbeat; one that does not GET re-reads once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_a_late_joiner_by_s4() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let o = owner(&os, "lab/beacon", &["a", "b"], 0, None).await;
    let (ka, kb) = (peer_key("a"), peer_key("b"));

    // 1. The puts, then nothing.
    o.peers["a"].put("1").await.unwrap();
    o.peers["b"].put("1").await.unwrap();
    let put = Instant::now();
    until(put, Duration::from_secs(1)).await;

    // 2. S subscribes, then GETs, and seeds. 3. S′ subscribes alone.
    let c = reader(&rs, "lab/beacon");
    let (sub, _log, confirmed) = listen(&c, PEER).await;
    let asked = Instant::now();
    let answer = c.get_answer(PEER, None, T).await.unwrap();
    assert!(answer.complete && answer.current.len() == 2, "{answer:?}");
    sub.seed(&answer, asked);
    let (late, _llog, lconfirmed) = listen(&c, PEER).await;

    eventually("S confirms both", || async {
        confirmed
            .lock()
            .unwrap()
            .iter()
            .any(|x| x.keys == [ka.clone(), kb.clone()])
    })
    .await;
    assert_eq!(sub.rereads(), 0, "seeded: no re-read");
    until(put, Duration::from_secs(6)).await;
    for k in [&ka, &kb] {
        assert_eq!(
            verdict(&ttl(4), &sub.freshness(k)),
            (Verdict::Fresh, Reason::WithinHorizon),
            "{k}, 6 s after its put"
        );
    }
    eventually("S′ re-reads once", || async { late.rereads() == 1 }).await;
    eventually("S′ confirms both", || async {
        lconfirmed
            .lock()
            .unwrap()
            .iter()
            .any(|x| x.keys == [ka.clone(), kb.clone()])
    })
    .await;
}

/// §6: a new instance under a new run: no gap, the leftover diverges, the
/// re-read drops it, and no heartbeat confirms it again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s6_a_restart_the_leftovers_leave_the_view() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let c = reader(&rs, "lab/beacon");
    let (sub, _log, confirmed) = listen(&c, PEER).await;
    let (ka, kb) = (peer_key("a"), peer_key("b"));

    // 1. The first instance.
    let o = owner(&os, "lab/beacon", &["a", "b"], 0, None).await;
    matched(o.peers["a"].writer()).await;
    let first_run = o.svc.minter().run();
    o.peers["a"].put("1").await.unwrap();
    o.peers["b"].put("1").await.unwrap();
    eventually("both are confirmed", || async {
        sub.confirmed(&ka).is_some() && sub.confirmed(&kb).is_some()
    })
    .await;

    // 2. It closes; a new instance puts peer/a alone.
    o.svc.close().await.unwrap();
    let os2 = client(&ep).await;
    let n = owner(&os2, "lab/beacon", &["a"], 0, None).await;
    assert_ne!(n.svc.minter().run(), first_run, "a new run");
    matched(n.peers["a"].writer()).await;
    n.peers["a"].put("1").await.unwrap();
    let restarted = Instant::now();

    // 3. The divergences, the re-read, peer/a confirmed, peer/b gone.
    eventually("one re-read", || async { sub.rereads() >= 1 }).await;
    let reread = sub.last_reread().unwrap();
    eventually("peer/a confirmed after the re-read", || async {
        confirmed
            .lock()
            .unwrap()
            .iter()
            .any(|x| x.arrival > reread && x.keys == [ka.clone()])
    })
    .await;
    assert!(
        matches!(
            sub.freshness(&ka),
            Observation::Subscribed { complete: true, .. }
        ),
        "another run's heartbeats follow no lost one"
    );
    assert!(sub.confirmed(&kb).is_none(), "peer/b left the view");
    assert!(
        !confirmed
            .lock()
            .unwrap()
            .iter()
            .any(|x| x.arrival > restarted && x.keys.contains(&kb)),
        "no heartbeat confirms peer/b again"
    );
    eventually("peer/b goes stale", || async {
        verdict(&ttl(4), &sub.freshness(&kb)).0 == Verdict::Stale
    })
    .await;
}

/// §7: the plan lets the heartbeat cross, never downsampled, as liveness
/// evidence; through R1 running the plan's downsampling block, F hears
/// every heartbeat and `level` at most once an interval, and judges the
/// heartbeat fresh.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_across_a_face_liveness_and_no_downsampling() {
    use zenkey::model::link;
    // 1. The plan.
    let policy = r#"
system   = "lab"
services = ["beacon"]

[face.rf]
far          = "far"
attach       = "client"
select       = { link_protocols = ["tcp"] }
class        = "constrained"
expose       = "link"
interval_s   = 2
max_outage_s = 60
batch_bytes  = 1000
link_bps     = 64000
"#;
    let parsed = link::parse(policy);
    let policy = parsed.policy.expect("a face policy");
    let mut d = link::Deployment::default();
    d.implements.insert(
        "beacon".to_owned(),
        vec!["beacon.v2".to_owned(), "freshness.v2".to_owned()],
    );
    let mut b = contract("beacon.v2");
    for r in &mut b.resources {
        if ["status", "level"].contains(&r.template.as_str()) {
            r.annotations.insert(
                link::EXPOSURE.to_owned(),
                serde_json::Value::String("link".to_owned()),
            );
        }
    }
    d.add_contract(&b);
    d.add_contract(zenkey::heartbeat::implementation().contract());
    let plans = link::plan(&policy, &d).expect("a plan");
    let rf = &plans[0];
    let ds = rf.downsampling.as_ref().expect("a downsampling block");
    assert_eq!(
        ds.rules
            .iter()
            .map(|r| r.key_expr.as_str())
            .collect::<Vec<_>>(),
        ["zk2/lab/beacon/beacon.v2/stream/level"],
        "one rule, level's, none for the heartbeat"
    );
    let hb = zenoh::key_expr::KeyExpr::try_from(HB_KEY).unwrap();
    for deny in &rf.deny {
        for k in &deny.key_exprs {
            let k = zenoh::key_expr::KeyExpr::try_from(k.as_str()).unwrap();
            assert!(!k.intersects(&hb), "{k} denies the heartbeat");
        }
    }
    let live = rf
        .liveness
        .iter()
        .find(|l| l.service == "lab/beacon")
        .unwrap();
    assert!(
        live.evidence
            .contains(&"freshness.v2/stream/heartbeat/beacon.v2".to_owned()),
        "{live:?}"
    );

    // 2. R1 runs the block; the owner publishes; F listens 5 s.
    let block = serde_json::to_string(&[ds]).unwrap();
    let (_r1, ep) = router_with(None, &[("downsampling", &block)]).await;
    let os = client(&ep).await;
    let fs = client(&ep).await;
    let (_h, beats) = heartbeats(&fs, HB_KEY).await;
    let levels: Arc<Mutex<Vec<Instant>>> = Arc::default();
    let w = Arc::clone(&levels);
    let _l = fs
        .declare_subscriber("zk2/lab/beacon/beacon.v2/stream/level")
        .callback(move |s| {
            if s.kind() == SampleKind::Put {
                w.lock().unwrap().push(Instant::now());
            }
        })
        .await
        .unwrap();
    let o = owner(&os, "lab/beacon", &[], 0, None).await;
    matched(&o.level).await;
    o.status.put("up").await.unwrap();
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(5) {
        o.level.put("1").await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let got = beats_after(&beats, t0);
    assert!(got.len() >= 4, "every heartbeat crosses: {}", got.len());
    for w in got.windows(2) {
        assert!(w[1].at.duration_since(w[0].at) <= Duration::from_secs(1) + JITTER);
    }
    let lv: Vec<Instant> = levels
        .lock()
        .unwrap()
        .iter()
        .copied()
        .filter(|t| *t > t0)
        .collect();
    assert!(
        (1..=4).contains(&lv.len()),
        "level downsampled to one every 2 s: {}",
        lv.len()
    );
    for w in lv.windows(2) {
        assert!(w[1].duration_since(w[0]) + JITTER >= Duration::from_secs(2));
    }
    // Liveness: the heartbeat key fresh by its horizon, the shortest of its
    // collections' (2 s).
    let horizon = Horizon::Within(layout().horizon().unwrap());
    let last = got.last().unwrap().at;
    let o = Observation::Subscribed {
        last: Some(zenkey::model::freshness::Last::Put(last.elapsed())),
        listened: t0.elapsed(),
        complete: true,
    };
    assert_eq!(
        verdict(&horizon, &o).0,
        Verdict::Fresh,
        "live across the face"
    );
}
