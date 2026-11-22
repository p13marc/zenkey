//! `spec/profiles/freshness/scenarios-v2.md` §1–§8 (`freshness.v2`, #978's
//! F3; §8 is #1110's H3, an owner sending owner heartbeats), one test per
//! section, named after it. §2's step 6, a candidate's, has a test of its
//! own, and so have §8's steps 3 and 4, which run a face's plan on R1.
//! #1080's heartbeat time under a catalog's epoch, beside the same owner's
//! frames, follows §7; three tests of the owner heartbeat that are no
//! scenario's are last.
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
    SETTLE, T, client, config, contract, eventually, imp, link_to, retrying_client, router,
    router_via, router_with,
};
use zenkey::consumer::{Confirmation, Consumer, Delivery, Subscription};
use zenkey::model::freshness::{
    ClockTrust, DEFAULT_DELTA, Horizon, Observation, Reason, Verdict, judge,
};
use zenkey::model::grammar::IfaceId;
use zenkey::model::heartbeat::{Digest, Heartbeat, Layout, Members, Time};
use zenkey::model::template::Bindings;
use zenkey::model::wire::{Envelope, Form, Stamp, Tag};
use zenkey::redundancy::{RedundancyStartup, Role};
use zenkey::state::{StateGet, StateWriter};
use zenkey::{Error, Service, ServiceBuilder};
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
    heartbeats_of(s, key, layout()).await
}

/// H on `key`, each heartbeat read with `l`'s collections.
async fn heartbeats_of(
    s: &zenoh::Session,
    key: &str,
    l: Layout,
) -> (zenoh::pubsub::Subscriber<()>, Beats) {
    let beats: Beats = Arc::default();
    let w = Arc::clone(&beats);
    let sub = s
        .declare_subscriber(key)
        .callback(move |sample| {
            let at = Instant::now();
            let bytes = sample.payload().to_bytes();
            let e = Envelope::parse(&bytes).expect("an envelope");
            assert_eq!(e.form, Form::Heartbeat, "core §7.5's form 6");
            let hb = Heartbeat::parse(e.record, &l).expect("a body of the layout's");
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
/// next heartbeat confirms; the clock guard holds no put and no bit, only
/// the time (#978's F6), and a GET reader finds the reply's stamp ahead; a
/// silent heartbeat goes stale, never "down".
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

    // 4. A clock ahead: the guard holds no write of beacon.v2's, which
    // carries no Zenoh timestamp, and the heartbeats go on vouching, their
    // time the ahead time (§2.10, core §7.5). S2 judges `status` fresh past its horizon; G,
    // on the deployment's word, finds the reply stamped ahead of its clock.
    let tick = client(&ep).await;
    let ahead_session = client(&ep).await;
    let ahead_key = "zk2/lab/ahead/freshness.v2/stream/heartbeat/beacon.v2";
    let ahead_status = "zk2/lab/ahead/beacon.v2/state/status";
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
    let held_put = ahead
        .status
        .put("still up")
        .await
        .expect("not held: it carries no Zenoh timestamp")
        .stamp;
    until(detected, Duration::from_secs(4)).await;
    let held = beats_after(&beats, detected + Duration::from_millis(1200));
    assert!(held.len() >= 2, "heartbeats go on: {}", held.len());
    for b in &held {
        assert_eq!(
            b.hb.time,
            Time::Ahead,
            "the ahead time while held, never the unknown time (§2.10)"
        );
        assert!(
            b.hb.sections[0].is_some() && b.hb.sections[1].is_none(),
            "the map 1 0 1"
        );
        let s = b.hb.sections[2].as_ref().expect("status's section");
        assert_eq!(
            s.members,
            Members::One { vouched: true },
            "vouched for, as at any other time"
        );
        assert_eq!(
            s.digest,
            Digest::of([(ahead_status, held_put)]),
            "the digest over the put made while the guard held"
        );
    }
    assert!(ahead.svc.minter().is_ahead(), "the guard still holds");
    assert_eq!(
        verdict(&h, &asub.freshness(ahead_status)),
        (Verdict::Fresh, Reason::WithinHorizon),
        "confirmed past its horizon"
    );
    let reply = match a.get(STATUS, None, T).await.unwrap() {
        StateGet::Answered(mut v) => v.remove(0),
        StateGet::Silent => panic!("lab/ahead did not answer"),
    };
    let word = ClockTrust::Trusted {
        delta: DEFAULT_DELTA,
    };
    let j = a.freshness(STATUS, &[reply.freshness(word)]).unwrap();
    assert_eq!(
        (j.verdict, j.reason),
        (Verdict::Unobservable, Reason::ClockDisagrees),
        "the last confirmation, from the clock ahead"
    );

    // 5. The first owner closes: its heartbeat falls silent, and S judges
    // stale, which says nothing of presence.
    o.svc.close().await.unwrap();
    let closed = Instant::now();
    until(closed, Duration::from_secs(3)).await;
    assert_eq!(judged(), (Verdict::Stale, Reason::BeyondHorizon));
}

/// A client of `ep` whose session stamps every write, as a candidate's
/// does (core §8.2, `redundancy.v1` §2.10).
async fn stamping(ep: &str) -> zenoh::Session {
    common::panic_watchdog::install();
    let mut c = zenoh::Config::default();
    c.insert_json5("mode", "\"client\"").unwrap();
    c.insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    c.insert_json5("scouting/gossip/enabled", "false").unwrap();
    c.insert_json5("connect/endpoints", &format!("[\"{ep}\"]"))
        .unwrap();
    c.insert_json5("timestamping/enabled", "true").unwrap();
    zenoh::open(c).await.expect("a candidate's session")
}

/// §2 step 6 (F-461): a candidate's session stamps every write (core
/// §8.2), so its clock guard holds a put of `status`. Its heartbeats go on,
/// their time the ahead time, and clear `status`'s bit from the held put until a
/// put goes through (§2.10), the guard's release alone setting nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_a_candidates_held_put_clears_its_bit() {
    const KEY: &str = "zk2/lab/candidate/freshness.v2/stream/heartbeat/beacon.v2";
    const STATUS_OF: &str = "zk2/lab/candidate/beacon.v2/state/status";
    let (_r1, ep) = router(None).await;
    let tick = client(&ep).await;
    let hs = client(&ep).await;
    let governed = contract("beacon-governed.v2");
    let (_h3, beats) = heartbeats_of(&hs, KEY, Layout::of(&governed)).await;
    let (claim, watch, read) = (
        stamping(&ep).await,
        stamping(&ep).await,
        stamping(&ep).await,
    );
    let mut cfg = config("lab/candidate");
    cfg.clock_reference = Some("clock/heartbeat".to_owned());
    let mut b = ServiceBuilder::new(&claim, cfg);
    b.implement(zenkey::Implementation::new(governed)).unwrap();
    b.minter().simulate_offset(2000);
    let status = b
        .declare_state_writer(&beacon(), STATUS, &Bindings::new())
        .await
        .unwrap();
    let red = b
        .redundancy(RedundancyStartup::new(read).with_watch(watch))
        .unwrap();
    let svc = b.start().await.unwrap();
    tokio::time::timeout(SETTLE, red.until(&beacon(), Role::Serving))
        .await
        .expect("the candidate serves");
    assert!(
        svc.minter().session_timestamping(),
        "a candidate's session stamps every write (core §8.2)"
    );
    let section = |b: &Beat| b.hb.sections[0].clone().expect("status's section");

    // Its put of `up`, then the drift detected: no write held yet, so the
    // heartbeats vouch for it, their time the ahead time.
    let up = status.put("up").await.unwrap().stamp;
    eventually("the drift is detected", || async {
        tick.put("clock/heartbeat", "tick").await.unwrap();
        svc.minter().is_ahead()
    })
    .await;
    let detected = Instant::now();
    eventually("a heartbeat while the guard holds", || async {
        beats_after(&beats, detected + JITTER)
            .iter()
            .any(|b| b.hb.time == Time::Ahead)
    })
    .await;
    for b in beats_after(&beats, detected + JITTER) {
        assert_eq!(b.hb.time, Time::Ahead, "the ahead time while held");
        assert_eq!(
            section(&b).members,
            Members::One { vouched: true },
            "no write held yet"
        );
    }

    // The put of `still up` held: each heartbeat from it on vouches for
    // nothing, its digest over the value the owner still holds.
    assert!(
        matches!(status.put("still up").await, Err(Error::ClockAhead)),
        "held: the candidate's session stamps it"
    );
    let held = Instant::now();
    until(held, Duration::from_millis(2500)).await;
    let after_hold = beats_after(&beats, held + JITTER);
    assert!(
        after_hold.len() >= 2,
        "heartbeats go on: {}",
        after_hold.len()
    );
    for b in &after_hold {
        assert_eq!(b.hb.time, Time::Ahead, "the ahead time while held");
        let s = section(b);
        assert_eq!(
            s.members,
            Members::One { vouched: false },
            "its bit cleared"
        );
        assert_eq!(s.digest, Digest::of([(STATUS_OF, up)]), "the value held");
    }

    // The guard releases: the time is an instant again, the bit still
    // clear.
    svc.minter().simulate_offset(0);
    eventually("the drift is gone", || async {
        tick.put("clock/heartbeat", "tick").await.unwrap();
        !svc.minter().is_ahead()
    })
    .await;
    let released = Instant::now();
    until(released, Duration::from_millis(2500)).await;
    let after_release = beats_after(&beats, released + JITTER);
    assert!(after_release.len() >= 2, "{}", after_release.len());
    for b in &after_release {
        assert!(b.hb.time.is_instant(), "an instant again: {:?}", b.hb.time);
        assert_eq!(
            section(b).members,
            Members::One { vouched: false },
            "clear until a write goes through"
        );
    }

    // A put goes through: its bit is set again, over the new value.
    let put = Instant::now();
    let again = status.put("still up").await.unwrap().stamp;
    eventually("the next heartbeat vouches for the put", || async {
        beats_after(&beats, put + JITTER).iter().any(|b| {
            let s = section(b);
            s.members == (Members::One { vouched: true })
                && s.digest == Digest::of([(STATUS_OF, again)])
        })
    })
    .await;
    svc.close().await.unwrap();
}

/// §3: the put carries no owner's Zenoh timestamp; the reply carries the
/// last confirmation, which ages it for a GET reader, its instant a
/// heartbeat's time to the millisecond (core §4.3, "Minting").
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_a_get_readers_age_from_the_confirmation_stamp() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let hs = client(&ep).await;
    let (_h, beats) = heartbeats(&hs, HB_KEY).await;
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
    // v2 0.7 §2.6 (#977's E3): the owner answers by snapshot, the entry
    // vouched for at the snapshot's time (confirmation 1), which the
    // consumer stamps it with, on the owner's clock named by its address.
    let answer = c.get_answer(STATUS, None, T).await.unwrap();
    let key = answer.current[0].key().to_owned();
    assert_eq!(
        answer.confirmations.get(&key).map(|c| c.confirmation),
        Some(zenkey::model::collection::Confirmation::AtTime),
        "vouched for now"
    );
    let reply = get().await;
    let ts = reply.timestamp().expect("the confirmation");
    assert_eq!(
        *ts.get_id(),
        zenkey::inband::owner_clock(&key),
        "the owner's clock"
    );
    let at = ts.get_time().to_system_time();
    assert!(at > put_wall, "later than the put");
    let age = SystemTime::now().duration_since(at).unwrap_or_default();
    assert!(
        age <= Duration::from_secs(1) + JITTER,
        "the snapshot's: {age:?}"
    );
    assert_eq!(
        c.freshness(STATUS, &[reply.freshness(word)])
            .unwrap()
            .verdict,
        Verdict::Fresh,
        "3 s after its put"
    );
    // The heartbeats go on vouching for it.
    tokio::time::sleep(JITTER).await;
    assert!(
        beats.lock().unwrap().iter().any(|b| {
            b.hb.sections[2]
                .as_ref()
                .is_some_and(|s| s.members == Members::One { vouched: true })
        }),
        "a heartbeat vouching for status"
    );
    let _ = &owner_zid;

    // 2. The bit cleared; 3.5 s later, stale: confirmation 2, the last
    // confirmation.
    o.status.set_vouched(false);
    let cleared = Instant::now();
    until(cleared, Duration::from_millis(3500)).await;
    let answer = c.get_answer(STATUS, None, T).await.unwrap();
    assert!(matches!(
        answer.confirmations.values().next().map(|c| c.confirmation),
        Some(zenkey::model::collection::Confirmation::At(_))
    ));
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
    // The contract the owner runs, as the conventions write it: `status`
    // and `level` marked `link.exposure = "link"`.
    d.add_contract(&contract("beacon.v2"));
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

/// #1080 (v2 §2.4, core §7.6, §8.6), no scenario's: an owner given a
/// catalog counts its heartbeats' time from the catalog's epoch, as its
/// frames' record 0 counts. `lab/beacon` implements `beacon.v2` and
/// `linkframes.v1` under zenoh-modem's catalog (epoch 1767225600); H reads
/// its heartbeats and its frames raw. Each heartbeat's time, read under the
/// epoch, is the owner's wall clock, exactly the instant a GET's reply
/// carries as the status's last confirmation, and its seconds are the
/// frame's, never Unix seconds (which would read as 2082).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_heartbeats_time_counts_from_the_catalogs_epoch() {
    use zenkey::model::catalog::Catalog;
    use zenkey::model::frame::{Frame, Layout as FrameLayout};
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../examples/zk2/zenoh-modem/catalog.json");
    let catalog = Catalog::parse_bytes(&std::fs::read(file).unwrap()).unwrap();
    let epoch = catalog.epoch();
    assert_eq!(epoch, 1_767_225_600, "zenoh-modem's epoch, 2026-01-01");
    let unix_ms = |t: SystemTime| {
        i128::try_from(
            t.duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap()
    };

    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let hs = client(&ep).await;
    let (_h, beats) = heartbeats(&hs, HB_KEY).await;
    let frames_c = contract("frames/linkframes.v1");
    let link_r = frames_c
        .resources
        .iter()
        .find(|r| r.template.as_str() == "frames/link")
        .unwrap();
    let link_layout = FrameLayout::of_resource(&frames_c, link_r)
        .unwrap()
        .expect("a frame resource");
    let frames: Arc<Mutex<Vec<Frame>>> = Arc::default();
    let f = Arc::clone(&frames);
    let _fs = hs
        .declare_subscriber("zk2/lab/beacon/linkframes.v1/stream/frames/link")
        .callback(move |sample| {
            let bytes = sample.payload().to_bytes();
            let e = Envelope::parse(&bytes).expect("an envelope");
            assert!(e.is_frame(), "core §7.7's form 5");
            f.lock()
                .unwrap()
                .push(Frame::parse(e.tag, e.record, &link_layout).expect("a frame of its layout"));
        })
        .await
        .unwrap();

    let linkframes: IfaceId = "linkframes.v1".parse().unwrap();
    let none = Bindings::new();
    let mut b = ServiceBuilder::new(&os, config("lab/beacon"));
    b.catalog(catalog).unwrap();
    b.implement(imp("beacon.v2")).unwrap();
    b.implement(imp("frames/linkframes.v1")).unwrap();
    let status = b
        .declare_state_writer(&beacon(), STATUS, &none)
        .await
        .unwrap();
    for r in [PEER, "state/intent", "state/note", LEVEL] {
        b.expose(&beacon(), r).unwrap();
    }
    let link = b
        .declare_frame_writer(&linkframes, "stream/frames/link")
        .await
        .unwrap();
    let _sample = b
        .declare_frame_writer(&linkframes, "stream/frames/sample")
        .await
        .unwrap();
    b.expose(&linkframes, LEVEL).unwrap();
    let svc = b.start().await.unwrap();
    assert_eq!(svc.deployment_epoch(), epoch, "the catalog's");
    matched(link.writer()).await;

    // The status put and vouched for; a frame's one record.
    let before = unix_ms(SystemTime::now());
    status.put("up").await.unwrap();
    link.record_json(&serde_json::json!({
        "dropped": 7, "queue_depth": 12, "duty_available": 1, "duty_wait_ms": "1500",
    }))
    .await
    .unwrap()
    .expect("a frame of one record");
    let t0 = Instant::now();
    eventually("H hears the frame and two heartbeats", || async {
        !frames.lock().unwrap().is_empty() && beats_after(&beats, t0).len() >= 2
    })
    .await;
    let after = unix_ms(SystemTime::now());

    // Each heartbeat's time is the wall clock under the epoch.
    let heard = beats_after(&beats, t0);
    let wall = before - 1_000..=after + 1_000;
    for b in &heard {
        let at = b.hb.time.unix_ms(epoch).expect("a known time");
        assert!(
            wall.contains(&at),
            "{at} ms under the epoch, the wall clock between {before} and {after}"
        );
    }
    // Beside the frame's record 0: the same seconds, give or take the
    // heartbeat's period.
    let frame = frames.lock().unwrap()[0].clone();
    let ft = frame.time.expect("record 0's time");
    let fat = ft.unix_ms(epoch);
    assert!(
        wall.contains(&fat),
        "the frame's {fat} ms, the wall clock between {before} and {after}"
    );
    let Time::At { secs, .. } = heard[0].hb.time else {
        unreachable!("a known time")
    };
    assert!(
        (i64::from(secs) - i64::from(ft.s)).abs() <= 2,
        "the heartbeat's {secs} s beside the frame's {} s",
        ft.s
    );

    // A GET's snapshot counts its times from the same epoch (core 0.100
    // §4.3, "Minting"; #977's E3): the status vouched for, its confirmation
    // is the snapshot's time, which a reader under the deployment's epoch
    // reads as the wall clock.
    let c = reader(&rs, "lab/beacon").with_deployment_epoch(epoch);
    let answer = c.get_answer(STATUS, None, T).await.unwrap();
    let key = answer.current[0].key().to_owned();
    assert_eq!(
        answer.confirmations.get(&key).map(|c| c.confirmation),
        Some(zenkey::model::collection::Confirmation::AtTime),
        "vouched for now"
    );
    let ts = answer.current[0].timestamp().expect("its confirmation");
    let reply_ms = unix_ms(ts.get_time().to_system_time());
    let wall = before - 1_000..=unix_ms(SystemTime::now()) + 1_000;
    assert!(
        wall.contains(&reply_ms),
        "the snapshot's time under the epoch, {reply_ms} ms, the wall clock {wall:?}"
    );
    svc.close().await.unwrap();
}

// --- §8: an owner heartbeat (#1110's H3) ------------------------------------

use zenkey::consumer::{HeartbeatRead, HeartbeatSection};
use zenkey::model::canonical::Fingerprint;
use zenkey::model::catalog::Catalog;
use zenkey::model::heartbeat::{Class, OwnerHeartbeat, Section};
use zenkey::model::wire::Run;

const W2: &str = "lab/w2";
const W2_HOST: &str = "zk2/lab/w2/freshness.v2/stream/heartbeats/host";
const W2_LINK: &str = "zk2/lab/w2/freshness.v2/stream/heartbeats/link";
const W2_HEARTBEATS: &str = "zk2/lab/w2/freshness.v2/stream/heartbeat/*";
/// §8's three interfaces, in the catalog's order: indices 0, 1 and 2.
const W2_IFACES: [&str; 3] = ["beacon.v2", "dial.v1", "pulse.v1"];

fn w2_key(iface: &str, rest: &str) -> String {
    format!("zk2/lab/w2/{iface}/state/{rest}")
}

/// §8's catalog: `beacon.v2` at index 0, `dial.v1` at 1, `pulse.v1` at 2.
fn w2_catalog() -> Catalog {
    let mut c = Catalog::new(0).unwrap();
    for name in W2_IFACES {
        let k = contract(name);
        c.push(k.iface.clone(), Fingerprint::of(&k)).unwrap();
    }
    c
}

/// `lab/w2`'s writers.
struct W2Owner {
    svc: Service,
    status: StateWriter,
    peers: BTreeMap<String, StateWriter>,
    config: StateWriter,
    pulse: StateWriter,
    _rest: Vec<StateWriter>,
}

/// Brings up `lab/w2` under §8's catalog, covering its three interfaces by
/// owner heartbeats: `beacon.v2` as the conventions say (every resource
/// exposed but `route`, a writer for each of `peers`), `dial.v1` and
/// `pulse.v1`. Its runs are 16-bit ones from `runs` when given, else
/// random 32-bit draws.
async fn w2_owner(s: &zenoh::Session, peers: &[&str], runs: Option<&std::path::Path>) -> W2Owner {
    let mut b = ServiceBuilder::new(s, config(W2).owner_heartbeat());
    if let Some(file) = runs {
        b.run_store(Arc::new(zenkey::run::FileRunStore::new(file)))
            .unwrap();
    }
    b.catalog(w2_catalog()).unwrap();
    for name in W2_IFACES {
        b.implement(imp(name)).unwrap();
    }
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
    let mut rest = Vec::new();
    for r in ["state/intent", "state/note"] {
        rest.push(b.declare_state_writer(&beacon(), r, &none).await.unwrap());
    }
    b.expose(&beacon(), LEVEL).unwrap();
    let config = b
        .declare_state_writer(&"dial.v1".parse().unwrap(), "state/config", &none)
        .await
        .unwrap();
    let pulse = b
        .declare_state_writer(&"pulse.v1".parse().unwrap(), STATUS, &none)
        .await
        .unwrap();
    let svc = b.start().await.unwrap();
    W2Owner {
        svc,
        status,
        peers: by,
        config,
        pulse,
        _rest: rest,
    }
}

/// One put on an owner heartbeat key, as H reads it with no contract.
#[derive(Debug, Clone)]
struct OwnerBeat {
    at: Instant,
    /// The payload's length: CH, the run, the seq and the body.
    len: usize,
    stamp: Stamp,
    hb: OwnerHeartbeat,
}

type OwnerBeats = Arc<Mutex<Vec<OwnerBeat>>>;

/// H: a raw subscriber on an owner heartbeat key. Each put is core §7.5's
/// form 6 with CH's tag 0, read as the key says: an owner heartbeat.
async fn owner_heartbeats(
    s: &zenoh::Session,
    key: &str,
) -> (zenoh::pubsub::Subscriber<()>, OwnerBeats) {
    let beats: OwnerBeats = Arc::default();
    let w = Arc::clone(&beats);
    let sub = s
        .declare_subscriber(key)
        .callback(move |sample| {
            let at = Instant::now();
            assert_eq!(sample.kind(), SampleKind::Put, "a put");
            let bytes = sample.payload().to_bytes();
            let e = Envelope::parse(&bytes).expect("an envelope");
            assert_eq!(e.form, Form::Heartbeat, "core §7.5's form 6");
            assert_eq!(e.tag, Tag::NONE, "CH's tag 0: each group carries its own");
            let hb = OwnerHeartbeat::of(&e).expect("an owner heartbeat");
            w.lock().unwrap().push(OwnerBeat {
                at,
                len: bytes.len(),
                stamp: e.stamp,
                hb,
            });
        })
        .await
        .unwrap();
    (sub, beats)
}

fn owner_beats_after(b: &OwnerBeats, t: Instant) -> Vec<OwnerBeat> {
    b.lock()
        .unwrap()
        .iter()
        .filter(|x| x.at > t)
        .cloned()
        .collect()
}

/// The puts heard on `key`, counted: for a key nothing may be put on.
async fn puts_on(
    s: &zenoh::Session,
    key: &str,
) -> (zenoh::pubsub::Subscriber<()>, Arc<Mutex<Vec<String>>>) {
    let heard: Arc<Mutex<Vec<String>>> = Arc::default();
    let w = Arc::clone(&heard);
    let sub = s
        .declare_subscriber(key)
        .callback(move |sample| w.lock().unwrap().push(sample.key_expr().to_string()))
        .await
        .unwrap();
    (sub, heard)
}

/// The sections of `name`'s group in `hb`, read as a holder of `cat` and
/// of the revision reads them (core §7.5): the index through the catalog,
/// the tag the revision's, then the sections with its layout.
fn group_of(hb: &OwnerHeartbeat, cat: &Catalog, name: &str) -> Vec<Option<Section>> {
    let l = Layout::of(&contract(name));
    let index = cat.index_of(&l.iface, &l.fingerprint).expect("listed");
    let g = hb
        .group(index)
        .unwrap_or_else(|| panic!("no group at index {index} ({name}): {hb:?}"));
    let entry = g
        .entry(std::slice::from_ref(cat), Some(cat.id()))
        .expect("an index of the catalog");
    assert_eq!(entry.iface, l.iface, "the index names {name}");
    assert!(g.tag_is(&l), "{name}'s group carries its revision's tag");
    g.read(&l).expect("sections of the revision's layout")
}

fn indices(hb: &OwnerHeartbeat) -> Vec<u16> {
    hb.groups.iter().map(|g| g.index).collect()
}

/// The owner heartbeats of one key over a window: at least every 1 s plus
/// jitter, their seqs consecutive within one run.
fn assert_owner_cadence(got: &[OwnerBeat], what: &str) {
    assert!(
        got.len() >= 5,
        "{what}: an owner heartbeat at least every 1 s: {}",
        got.len()
    );
    for w in got.windows(2) {
        assert!(
            w[1].at.duration_since(w[0].at) <= Duration::from_secs(1) + JITTER,
            "{what}: at most half the shortest horizon apart, plus jitter"
        );
        assert_eq!(w[0].stamp.run, w[1].stamp.run, "{what}: one run");
        assert_eq!(
            w[1].stamp.seq.get(),
            w[0].stamp.seq.get() + 1,
            "{what}: the key's own consecutive seqs"
        );
    }
}

/// One collection as S or P follows it: its deliveries, and each
/// heartbeat or group its view read.
struct Follower {
    sub: Subscription,
    log: Log,
    read: Arc<Mutex<Vec<HeartbeatRead>>>,
}

impl Follower {
    fn reads_after(&self, t: Instant) -> Vec<HeartbeatRead> {
        self.read
            .lock()
            .unwrap()
            .iter()
            .filter(|h| h.arrival > t)
            .cloned()
            .collect()
    }
}

/// A tool's consumer of `iface` at `lab/w2`, holding that revision and the
/// catalog and no other contract, following `resource`.
async fn follow(s: &zenoh::Session, iface: &str, resource: &str, cat: &Catalog) -> Follower {
    let c = Consumer::for_tool(s, Arc::new(contract(iface)), &[W2], &Default::default())
        .unwrap()
        .with_reread_timeout(T)
        .with_catalog(cat);
    let log: Log = Arc::default();
    let read: Arc<Mutex<Vec<HeartbeatRead>>> = Arc::default();
    let (w, k) = (Arc::clone(&log), Arc::clone(&read));
    let sub = c
        .subscribe_heartbeats(
            resource,
            move |d: Delivery| {
                w.lock().unwrap().push(Rec {
                    stamp: d.sample.timestamp().copied(),
                    inband: d.inband,
                });
            },
            move |h: HeartbeatRead| k.lock().unwrap().push(h),
        )
        .await
        .unwrap();
    Follower { sub, log, read }
}

/// Waits until `f`'s view has read an owner heartbeat. Its subscription to
/// the collection was declared before the one to the class key, on one
/// session, so the owner's writers match it by then.
async fn following(f: &Follower, what: &str) {
    eventually(what, || async { f.sub.owner_heartbeats() >= 1 }).await;
}

/// §8, steps 1 and 2: `lab/w2` covers three interfaces by owner
/// heartbeats under a catalog. No heartbeat of an interface is sent; the
/// `host` key carries three groups and the `link` key one, each at least
/// every half of its shortest horizon on its own seqs, with no bit of
/// the other class; S, and P bound to one interface alone, judge every
/// member fresh past its horizon. Then a cut: one owner heartbeat lost
/// on each key is lost for every group, a missed put diverges one group
/// while the groups beside it confirm, and S re-reads that collection
/// once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s8_an_owner_heartbeat() {
    let (_r1, ep1) = router(None).await;
    let link = link_to(&ep1).await;
    let (_r2, ep2) = router_via(&link).await;
    let os = client(&ep1).await;
    let hs = client(&ep1).await;
    let ps = client(&ep1).await;
    let ss = retrying_client(&ep2).await;
    let cat = w2_catalog();
    assert_eq!(
        cat.entries()
            .iter()
            .map(|e| e.iface.to_string())
            .collect::<Vec<_>>(),
        W2_IFACES,
        "the catalog's three entries, at indices 0, 1 and 2"
    );

    // H: both owner heartbeat keys, and every heartbeat key of the owner.
    let (_hh, host) = owner_heartbeats(&hs, W2_HOST).await;
    let (_hl, linkk) = owner_heartbeats(&hs, W2_LINK).await;
    let (_hi, per_iface) = puts_on(&hs, W2_HEARTBEATS).await;

    // The owner, on 16-bit runs.
    let runs = tempfile::tempdir().unwrap();
    let o = w2_owner(&os, &["a", "b", "c"], Some(&runs.path().join("run"))).await;
    let descriptor: serde_json::Value = serde_json::from_slice(&o.svc.descriptor_bytes()).unwrap();
    let listed: Vec<&str> = descriptor["interfaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["iface"].as_str().unwrap())
        .collect();
    for iface in ["beacon.v2", "dial.v1", "pulse.v1", "freshness.v2"] {
        assert!(listed.contains(&iface), "the descriptor lists {iface}");
    }
    assert_eq!(
        descriptor["catalog"],
        cat.id().hex(),
        "the descriptor states the catalog's id"
    );
    assert_eq!(
        o.svc.owner_heartbeat_cover(),
        [
            (beacon(), 0),
            ("dial.v1".parse().unwrap(), 1),
            ("pulse.v1".parse().unwrap(), 2)
        ],
        "the owner covers the three interfaces"
    );
    let fv2 = descriptor["interfaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["iface"] == "freshness.v2")
        .unwrap();
    assert!(
        fv2.get("unavailable").is_none(),
        "it serves `heartbeats/{{class}}`, so lists nothing unavailable: {fv2}"
    );

    // S: every collection of the three interfaces. P: pulse.v1's alone.
    let s_peer = follow(&ss, "beacon.v2", PEER, &cat).await;
    let s_route = follow(&ss, "beacon.v2", "state/route", &cat).await;
    let s_status = follow(&ss, "beacon.v2", STATUS, &cat).await;
    let s_config = follow(&ss, "dial.v1", "state/config", &cat).await;
    let s_pulse = follow(&ss, "pulse.v1", STATUS, &cat).await;
    let p = follow(&ps, "pulse.v1", STATUS, &cat).await;
    for (f, what) in [
        (&s_peer, "S follows peer/{peer}"),
        (&s_route, "S follows route"),
        (&s_status, "S follows status"),
        (&s_config, "S follows dial.v1's config"),
        (&s_pulse, "S follows pulse.v1's status"),
        (&p, "P follows pulse.v1's status"),
    ] {
        following(f, what).await;
    }

    // 1. The puts, then 5 s with nothing else.
    let st_status = o.status.put("up").await.unwrap().stamp;
    let mut st_peers = Vec::new();
    for peer in ["a", "b", "c"] {
        st_peers.push(o.peers[peer].put("1").await.unwrap().stamp);
    }
    let st_config = o.config.put("x").await.unwrap().stamp;
    let st_pulse = o.pulse.put("ok").await.unwrap().stamp;
    let t0 = Instant::now();
    until(t0, Duration::from_secs(5)).await;

    // One form: no heartbeat of an interface, from the owner's start on.
    assert_eq!(
        *per_iface.lock().unwrap(),
        Vec::<String>::new(),
        "H hears no put on any heartbeat/<iface> key"
    );

    // The `host` key.
    let got = owner_beats_after(&host, t0);
    assert_owner_cadence(&got, "host");
    let Run::U16(_) = got[0].stamp.run else {
        panic!("a 16-bit run from the record: {:?}", got[0].stamp.run)
    };
    assert_eq!(
        descriptor["run"],
        got[0].stamp.run.hex(),
        "the key's stamp carries the run the descriptor states"
    );
    let (ka, kb, kc) = (
        w2_key("beacon.v2", "peer/a"),
        w2_key("beacon.v2", "peer/b"),
        w2_key("beacon.v2", "peer/c"),
    );
    let (k_status, k_config, k_pulse) = (
        w2_key("beacon.v2", "status"),
        w2_key("dial.v1", "config"),
        w2_key("pulse.v1", "status"),
    );
    // What each holds, read from those written after the last put: one
    // written between two of the puts can arrive just after them.
    for b in got.iter().filter(|b| b.at > t0 + JITTER) {
        assert_eq!(indices(&b.hb), [0, 1, 2], "three groups, ascending");
        let s = group_of(&b.hb, &cat, "beacon.v2");
        assert!(
            s[0].is_some() && s[1].is_none() && s[2].is_none(),
            "beacon.v2's map 1 0 0: status is another class's, route not exposed"
        );
        let peers = s[0].as_ref().unwrap();
        assert_eq!(
            peers.digest,
            Digest::of([
                (ka.as_str(), st_peers[0]),
                (kb.as_str(), st_peers[1]),
                (kc.as_str(), st_peers[2])
            ]),
            "over the three peers' stamps"
        );
        assert_eq!(peers.members.live(), Some(3));
        assert_eq!(peers.members.vouched(), [0, 1, 2], "all vouched for");
        let d = group_of(&b.hb, &cat, "dial.v1");
        assert_eq!(
            d,
            [Some(Section {
                digest: Digest::of([(k_config.as_str(), st_config)]),
                members: Members::One { vouched: true }
            })],
            "dial.v1's group vouches for its one member"
        );
        let q = group_of(&b.hb, &cat, "pulse.v1");
        assert_eq!(
            q,
            [Some(Section {
                digest: Digest::of([(k_pulse.as_str(), st_pulse)]),
                members: Members::One { vouched: true }
            })],
            "pulse.v1's group vouches for its one member"
        );
        assert_eq!(b.hb.record().unwrap().bits, 223, "223 bits of body");
        assert_eq!(b.len, 34, "34 bytes on a 16-bit run");
    }

    // The `link` key: its own seqs, one group, no bit of a `host`
    // collection.
    let got_link = owner_beats_after(&linkk, t0);
    assert_owner_cadence(&got_link, "link");
    for b in got_link.iter().filter(|b| b.at > t0 + JITTER) {
        assert_eq!(indices(&b.hb), [0], "beacon.v2's group alone");
        let s = group_of(&b.hb, &cat, "beacon.v2");
        assert_eq!(
            s,
            [
                None,
                None,
                Some(Section {
                    digest: Digest::of([(k_status.as_str(), st_status)]),
                    members: Members::One { vouched: true }
                })
            ],
            "the map 0 0 1: status, vouched for"
        );
        assert_eq!(b.len, 19, "19 bytes on a 16-bit run");
    }

    // Confirmed: S heard each put once, and judges every member fresh past
    // its horizon, 5 s after the puts.
    for (f, n, what) in [
        (&s_peer, 3, "peer/{peer}"),
        (&s_route, 0, "route"),
        (&s_status, 1, "status"),
        (&s_config, 1, "config"),
        (&s_pulse, 1, "pulse.v1's status"),
    ] {
        assert_eq!(
            f.log.lock().unwrap().len(),
            n,
            "S hears each put of {what} once"
        );
    }
    for (f, key, horizon) in [
        (&s_peer, &ka, 4),
        (&s_peer, &kb, 4),
        (&s_peer, &kc, 4),
        (&s_status, &k_status, 2),
        (&s_config, &k_config, 2),
        (&s_pulse, &k_pulse, 2),
        // One interface's reader.
        (&p, &k_pulse, 2),
    ] {
        assert_eq!(
            verdict(&ttl(horizon), &f.sub.freshness(key)),
            (Verdict::Fresh, Reason::WithinHorizon),
            "{key}: fresh 5 s after its put, past its horizon"
        );
        assert_eq!(f.sub.rereads(), 0, "{key}: confirmed with no re-read");
    }
    // P read its group behind two groups whose revisions it does not hold.
    let p_reads = p.reads_after(t0);
    assert!(p_reads.len() >= 5, "P reads each `host` owner heartbeat");
    for h in &p_reads {
        let g = h.group.expect("a group of an owner heartbeat");
        assert_eq!((g.class, g.index), (Class::Host, 2), "pulse.v1's group");
        assert_eq!(
            h.section,
            HeartbeatSection::Confirmed(vec![k_pulse.clone()]),
            "its one member confirmed"
        );
        assert!(!h.lost);
    }

    // 2. The cut, a put S never hears, the heal 2.5 s later, then 6 s.
    link.cut();
    let cut = Instant::now();
    o.peers["a"].put("2").await.unwrap();
    until(cut, Duration::from_millis(2500)).await;
    link.heal();
    let healed = Instant::now();

    // A lost owner heartbeat: the first S reads on each key after the heal
    // follows a gap, lost for every group on it; the next is in sequence.
    let hosts = [
        (&s_peer, "peer/{peer}"),
        (&s_route, "route"),
        (&s_config, "dial.v1's config"),
        (&s_pulse, "pulse.v1's status"),
    ];
    for (f, what, class) in hosts
        .iter()
        .map(|(f, what)| (*f, *what, Class::Host))
        .chain([(&s_status, "status", Class::Link)])
    {
        eventually(
            &format!("{what}: two owner heartbeats after the heal"),
            || async { f.reads_after(cut).len() >= 2 },
        )
        .await;
        let reads = f.reads_after(cut);
        assert!(reads[0].arrival > healed, "{what}: nothing crossed the cut");
        assert!(
            reads[0].lost,
            "{what}: the first owner heartbeat after the heal follows a gap on its key"
        );
        assert_eq!(reads[0].group.map(|g| g.class), Some(class), "{what}");
        assert!(
            !reads[1].lost
                && reads[1].heartbeat.seq.get() == reads[0].heartbeat.seq.get() + 1
                && reads[1].heartbeat.run == reads[0].heartbeat.run,
            "{what}: the next one arrives in sequence, and ends that"
        );
    }

    // A divergence in one group: S's view holds peer/a's old stamp, so
    // beacon.v2's group diverges on peer/{peer}, twice in a row, while the
    // groups beside it confirm their members in the same owner heartbeats.
    eventually("S re-reads peer/{peer}", || async {
        s_peer.sub.rereads() == 1
    })
    .await;
    let reread = s_peer.sub.last_reread().unwrap();
    let diverged: Vec<HeartbeatRead> = s_peer
        .reads_after(cut)
        .into_iter()
        .filter(|h| h.arrival < reread)
        .collect();
    // Two in a row ask for the re-read. Its answer replaces the view a
    // moment later, so under load a third can be read before it lands.
    assert!(
        (2..=3).contains(&diverged.len()),
        "two in a row, then the re-read: {diverged:#?}"
    );
    assert_eq!(
        diverged[1].heartbeat.seq.get(),
        diverged[0].heartbeat.seq.get() + 1,
        "in a row"
    );
    for h in &diverged {
        assert_eq!(h.section, HeartbeatSection::Diverged, "{h:?}");
        for (f, key) in [(&s_config, &k_config), (&s_pulse, &k_pulse)] {
            let beside = f
                .reads_after(cut)
                .into_iter()
                .find(|x| x.heartbeat == h.heartbeat)
                .unwrap_or_else(|| panic!("{key}: no group read in {:?}", h.heartbeat));
            assert_eq!(
                beside.section,
                HeartbeatSection::Confirmed(vec![key.clone()]),
                "{key}: its group confirms in the owner heartbeat where peer/{{peer}} diverges"
            );
        }
        // route's map bit is 0 in the same group: nothing confirmed there.
        let route = s_route
            .reads_after(cut)
            .into_iter()
            .find(|x| x.heartbeat == h.heartbeat)
            .expect("the group read for route");
        assert_eq!(route.section, HeartbeatSection::Absent);
    }
    for h in s_status.reads_after(cut) {
        assert_eq!(
            h.section,
            HeartbeatSection::Confirmed(vec![k_status.clone()]),
            "status confirms on the `link` key throughout"
        );
    }
    // The next `host` owner heartbeat confirms its three members.
    eventually("the three peers confirmed after the re-read", || async {
        s_peer.reads_after(reread).iter().any(|h| {
            h.section == HeartbeatSection::Confirmed(vec![ka.clone(), kb.clone(), kc.clone()])
        })
    })
    .await;
    let after = s_peer.reads_after(reread);
    assert_eq!(
        after[0].section,
        HeartbeatSection::Confirmed(vec![ka.clone(), kb.clone(), kc.clone()]),
        "the next one, no later"
    );
    until(healed, Duration::from_secs(6)).await;
    assert_eq!(s_peer.sub.rereads(), 1, "once");
    for f in [&s_route, &s_status, &s_config, &s_pulse] {
        assert_eq!(f.sub.rereads(), 0, "no other collection is re-read");
    }
    assert_eq!(
        s_peer.log.lock().unwrap().len(),
        3,
        "the lost change was never delivered"
    );
    for (f, key) in [
        (&s_peer, &ka),
        (&s_status, &k_status),
        (&s_config, &k_config),
        (&s_pulse, &k_pulse),
    ] {
        assert!(!f.sub.lost(key), "{key}: no longer read as a lost delivery");
        assert!(
            matches!(
                f.sub.freshness(key),
                Observation::Subscribed { complete: true, .. }
            ),
            "{key}: complete again"
        );
    }
    assert_eq!(
        *per_iface.lock().unwrap(),
        Vec::<String>::new(),
        "still no heartbeat of an interface"
    );
    o.svc.close().await.unwrap();
}

/// §8's faces: R1's one face `rf` toward F, selected by its link protocol,
/// a unix stream socket here, where the owner and H are R1's clients over
/// tcp.
const FACE_PROTOCOL: &str = "unixsock-stream";

/// Step 3's `link.v2` policy: `expose = "link"`, `interval_s = 2`, and a
/// `[transport]` its linter accepts.
const W2_V2_POLICY: &str = r#"profile  = "link.v2"
system   = "lab"
services = ["w2"]
binding  = "zenoh-1.10.1"

[transport]
lease_ms             = 10000
keep_alive           = 4
wait_before_close_ms = 10000
sequence             = "16bit"
batch_bytes          = 1000

[face.rf]
far              = "far"
attach           = "client"
select           = { link_protocols = ["unixsock-stream"] }
class            = "constrained"
expose           = "link"
interval_s       = 2
max_outage_s     = 60
link_bps         = 64000
loss_ppm         = 0
arq              = true
reply_timeout_ms = 1000
"#;

/// Step 4's policy: §7's `link.v1` one, with `lab/w2` listed.
const W2_V1_POLICY: &str = r#"
system   = "lab"
services = ["w2"]

[face.rf]
far          = "far"
attach       = "client"
select       = { link_protocols = ["unixsock-stream"] }
class        = "constrained"
expose       = "link"
interval_s   = 2
max_outage_s = 60
batch_bytes  = 1000
link_bps     = 64000
"#;

/// The face `rf`'s plan under `policy`: the deployment lists `lab/w2`
/// implementing the three interfaces and `freshness.v2`, and the far side
/// holds §8's catalog.
fn w2_plan(policy: &str) -> zenkey::model::link::FacePlan {
    use zenkey::model::link;
    let parsed = link::parse(policy);
    let policy = parsed
        .policy
        .unwrap_or_else(|| panic!("a face policy: {:?}", parsed.findings));
    let mut d = link::Deployment::default();
    d.implements.insert(
        "w2".to_owned(),
        W2_IFACES
            .iter()
            .map(|i| (*i).to_owned())
            .chain(["freshness.v2".to_owned()])
            .collect(),
    );
    for name in W2_IFACES {
        d.add_contract(&contract(name));
    }
    d.add_contract(zenkey::heartbeat::implementation().contract());
    d.catalog = Some(w2_catalog());
    let mut plans = link::plan(&policy, &d).expect("a plan");
    assert_eq!(plans.len(), 1, "one face");
    plans.remove(0)
}

/// Every key expression of `plan`'s deny `id`.
fn denied<'p>(plan: &'p zenkey::model::link::FacePlan, id: &str) -> &'p [String] {
    plan.deny
        .iter()
        .find(|d| d.id == id)
        .map(|d| d.key_exprs.as_slice())
        .unwrap_or_else(|| {
            panic!(
                "no deny {id:?} among {:?}",
                plan.deny.iter().map(|d| &d.id).collect::<Vec<_>>()
            )
        })
}

/// R1 running `plan`: its denies on the sessions the face selects, by
/// their link protocol, under `allow`, and its downsampling block. It
/// listens on loopback tcp for the near side and on a unix stream socket
/// for the face. Returns the router, the near endpoint and the face's.
async fn r1_running(
    plan: &zenkey::model::link::FacePlan,
    dir: &std::path::Path,
) -> (zenoh::Session, String, String) {
    let rules: Vec<serde_json::Value> = plan
        .deny
        .iter()
        .map(|d| {
            let mut rule = serde_json::to_value(d).unwrap();
            rule["permission"] = "deny".into();
            rule
        })
        .collect();
    let ids: Vec<&serde_json::Value> = rules.iter().map(|r| &r["id"]).collect();
    let acl = serde_json::json!({
        "enabled": true,
        "default_permission": "allow",
        "rules": rules,
        "subjects": [{"id": "far", "link_protocols": [FACE_PROTOCOL]}],
        "policies": [{"rules": ids, "subjects": ["far"]}],
    })
    .to_string();
    let face = format!("{FACE_PROTOCOL}/{}", dir.join("rf.sock").display());
    let listen = format!("[\"tcp/127.0.0.1:0\", \"{face}\"]");
    let mut extra = vec![("access_control", acl), ("listen/endpoints", listen)];
    if let Some(ds) = &plan.downsampling {
        extra.push(("downsampling", serde_json::to_string(&[ds]).unwrap()));
    }
    let extra: Vec<(&str, &str)> = extra.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let (r1, near) = router_with(None, &extra).await;
    (r1, near, face)
}

/// The owner `lab/w2` behind R1 at `near`, its members put; F on `face`,
/// subscribed to `beacon.v2`'s `status`, which crosses, and heard its put:
/// so F's session is across the face and its subscriptions stand before
/// the window opens.
async fn w2_across(near: &str, face: &str) -> (W2Owner, zenoh::Session, zenoh::Session) {
    let os = client(near).await;
    let fs = client(face).await;
    let heard: Arc<Mutex<Vec<String>>> = Arc::default();
    let w = Arc::clone(&heard);
    let status = fs
        .declare_subscriber(w2_key("beacon.v2", "status"))
        .callback(move |sample| w.lock().unwrap().push(sample.key_expr().to_string()))
        .await
        .unwrap();
    let o = w2_owner(&os, &["a", "b", "c"], None).await;
    matched(o.status.writer()).await;
    o.status.put("up").await.unwrap();
    for peer in ["a", "b", "c"] {
        o.peers[peer].put("1").await.unwrap();
    }
    o.config.put("x").await.unwrap();
    o.pulse.put("ok").await.unwrap();
    eventually("F hears status across the face", || async {
        !heard.lock().unwrap().is_empty()
    })
    .await;
    status.undeclare().await.unwrap();
    (o, os, fs)
}

/// §8, steps 3 and 4: across a `link.v2` face the plan denies the `host`
/// class's key and lets the `link` class's cross, never downsampled, with
/// `beacon.v2`'s group alone and no bit of a `host` collection, and it is
/// the far side's liveness evidence. Under `link.v1`'s policy neither key
/// crosses. R1 runs each plan: its denies on the face's sessions, and its
/// downsampling block.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s8_an_owner_heartbeat_across_a_face() {
    let cat = w2_catalog();
    let link_key = zenoh::key_expr::KeyExpr::try_from(W2_LINK).unwrap();
    let host_key = zenoh::key_expr::KeyExpr::try_from(W2_HOST).unwrap();
    let reaches = |plan: &zenkey::model::link::FacePlan, key: &zenoh::key_expr::KeyExpr| {
        plan.deny
            .iter()
            .flat_map(|d| &d.key_exprs)
            .filter(|k| {
                zenoh::key_expr::KeyExpr::try_from(k.as_str())
                    .unwrap()
                    .intersects(key)
            })
            .cloned()
            .collect::<Vec<_>>()
    };

    // 3. The plan under link.v2.
    let plan = w2_plan(W2_V2_POLICY);
    let stream = denied(&plan, "stream:freshness.v2");
    assert_eq!(
        stream,
        [
            "zk2/*/*/freshness.v2/stream/heartbeat/dial.v1/**",
            "zk2/*/*/freshness.v2/stream/heartbeat/pulse.v1/**",
            "zk2/*/*/freshness.v2/stream/heartbeats/host/**",
        ],
        "the `host` key, and the heartbeat keys of dial.v1 and pulse.v1"
    );
    assert_eq!(
        reaches(&plan, &link_key),
        Vec::<String>::new(),
        "it holds nothing of heartbeats/link"
    );
    let live = plan
        .liveness
        .iter()
        .find(|l| l.service == W2)
        .expect("lab/w2's liveness");
    assert!(
        live.evidence
            .contains(&"freshness.v2/stream/heartbeats/link".to_owned()),
        "{live:?}"
    );
    if let Some(ds) = &plan.downsampling {
        for rule in &ds.rules {
            let k = zenoh::key_expr::KeyExpr::try_from(rule.key_expr.as_str()).unwrap();
            assert!(
                !k.intersects(&link_key),
                "{k}: an owner heartbeat is never downsampled"
            );
        }
    }

    // R1 runs it. The owner and H are its clients on the near side, F over
    // the face's link protocol.
    let dir = tempfile::tempdir().unwrap();
    {
        let (_r1, near, face) = r1_running(&plan, dir.path()).await;
        let hs = client(&near).await;
        let (_hh, h_host) = owner_heartbeats(&hs, W2_HOST).await;
        let (o, _os, fs) = w2_across(&near, &face).await;
        let (_fh, f_host) = puts_on(&fs, W2_HOST).await;
        let (_fl, f_link) = owner_heartbeats(&fs, W2_LINK).await;
        eventually("F hears a `link` owner heartbeat", || async {
            !f_link.lock().unwrap().is_empty()
        })
        .await;
        let t0 = Instant::now();
        until(t0, Duration::from_secs(5)).await;

        // The `host` class stays home: put on the near side all along, in
        // 36 bytes on this owner's 32-bit run, and never heard by F.
        let near_host = owner_beats_after(&h_host, t0);
        assert_owner_cadence(&near_host, "host, on the near side");
        for b in &near_host {
            assert!(matches!(b.stamp.run, Run::U32(_)), "a 32-bit run");
            assert_eq!(indices(&b.hb), [0, 1, 2]);
            assert_eq!(b.len, 36, "36 bytes on a 32-bit run");
        }
        assert_eq!(
            *f_host.lock().unwrap(),
            Vec::<String>::new(),
            "F hears no put on the `host` key"
        );
        // The `link` key crosses, never downsampled, with beacon.v2's
        // group alone and no bit of peer/{peer}.
        let got = owner_beats_after(&f_link, t0);
        assert_owner_cadence(&got, "link, across the face");
        for b in &got {
            assert_eq!(indices(&b.hb), [0], "beacon.v2's group alone");
            let s = group_of(&b.hb, &cat, "beacon.v2");
            assert!(
                s[0].is_none() && s[1].is_none(),
                "the map 0 0 1: no bit of peer/{{peer}} crosses"
            );
            assert_eq!(
                s[2].as_ref().map(|x| x.members.clone()),
                Some(Members::One { vouched: true }),
                "status, vouched for"
            );
        }
        // Liveness: the `link` key fresh by its horizon, the shortest among
        // the `link` collections its groups cover (2 s).
        let horizon = layout().class_horizon(Class::Link).unwrap();
        assert_eq!(horizon, Duration::from_secs(2));
        let last = got.last().unwrap().at;
        let seen = Observation::Subscribed {
            last: Some(zenkey::model::freshness::Last::Put(last.elapsed())),
            listened: t0.elapsed(),
            complete: true,
        };
        assert_eq!(
            verdict(&Horizon::Within(horizon), &seen).0,
            Verdict::Fresh,
            "lab/w2 is live across the face"
        );
        o.svc.close().await.unwrap();
    }

    // 4. The same under §7's link.v1 policy, with lab/w2 listed.
    let plan = w2_plan(W2_V1_POLICY);
    let stream = denied(&plan, "stream:freshness.v2");
    assert!(
        stream.contains(&"zk2/*/*/freshness.v2/stream/heartbeats/**".to_owned()),
        "both classes' keys: {stream:?}"
    );
    assert!(!reaches(&plan, &link_key).is_empty() && !reaches(&plan, &host_key).is_empty());
    let dir = tempfile::tempdir().unwrap();
    let (_r1, near, face) = r1_running(&plan, dir.path()).await;
    let hs = client(&near).await;
    let (_hh, h_host) = owner_heartbeats(&hs, W2_HOST).await;
    let (_hl, h_link) = owner_heartbeats(&hs, W2_LINK).await;
    let (o, _os, fs) = w2_across(&near, &face).await;
    let (_fh, f_host) = puts_on(&fs, W2_HOST).await;
    let (_fl, f_link) = puts_on(&fs, W2_LINK).await;
    eventually("the owner heartbeats go out on the near side", || async {
        !h_host.lock().unwrap().is_empty() && !h_link.lock().unwrap().is_empty()
    })
    .await;
    let t0 = Instant::now();
    until(t0, Duration::from_secs(5)).await;
    assert_owner_cadence(&owner_beats_after(&h_host, t0), "host, on the near side");
    assert_owner_cadence(&owner_beats_after(&h_link, t0), "link, on the near side");
    assert_eq!(
        (f_host.lock().unwrap().len(), f_link.lock().unwrap().len()),
        (0, 0),
        "F hears no owner heartbeat on either key"
    );
    o.svc.close().await.unwrap();
}

// --- The owner heartbeat, no scenario's (#1110's H3) ------------------------

/// `freshness.v2`'s entry in a descriptor: what it lists unavailable.
fn fv2_unavailable(svc: &Service) -> serde_json::Value {
    let d: serde_json::Value = serde_json::from_slice(&svc.descriptor_bytes()).unwrap();
    d["interfaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["iface"] == "freshness.v2")
        .expect("the descriptor lists freshness.v2")
        .get("unavailable")
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

/// The owner heartbeat is the deployment's to turn on (v2 §2.4, "Readers
/// first"). An owner given a catalog and no `owner_heartbeat` sends its
/// interfaces' heartbeats alone, and lists `heartbeats/{class}`
/// unavailable, cause `config` (core §3.3: why the resource is absent
/// here). One with the option and no catalog does not start, nothing
/// declared: a group names its interface by its index in the catalog.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owner_heartbeat_is_off_unless_the_deployment_sets_it() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let hs = client(&ep).await;
    let (_hh, host) = puts_on(&hs, W2_HOST).await;
    let (_hl, linkk) = puts_on(&hs, W2_LINK).await;
    let (_hi, per_iface) = puts_on(&hs, W2_HEARTBEATS).await;
    let none = Bindings::new();

    // Off: the catalog alone turns nothing on.
    let mut b = ServiceBuilder::new(&os, config(W2));
    b.catalog(w2_catalog()).unwrap();
    for name in ["dial.v1", "pulse.v1"] {
        b.implement(imp(name)).unwrap();
    }
    let _config = b
        .declare_state_writer(&"dial.v1".parse().unwrap(), "state/config", &none)
        .await
        .unwrap();
    let _pulse = b
        .declare_state_writer(&"pulse.v1".parse().unwrap(), STATUS, &none)
        .await
        .unwrap();
    let svc = b.start().await.unwrap();
    assert!(svc.owner_heartbeat_cover().is_empty());
    let listed = fv2_unavailable(&svc);
    assert_eq!(listed.as_array().map(Vec::len), Some(1), "{listed}");
    assert_eq!(
        (&listed[0]["resource"], &listed[0]["cause"]),
        (
            &serde_json::json!("stream/heartbeats/{class}"),
            &serde_json::json!("config")
        ),
        "left out by the deployment's configuration: {listed}"
    );
    eventually("each interface's heartbeat, twice", || async {
        let heard = per_iface.lock().unwrap();
        ["dial.v1", "pulse.v1"].iter().all(|i| {
            heard
                .iter()
                .filter(|k| k.ends_with(&format!("/heartbeat/{i}")))
                .count()
                >= 2
        })
    })
    .await;
    assert_eq!(
        (host.lock().unwrap().len(), linkk.lock().unwrap().len()),
        (0, 0),
        "no owner heartbeat"
    );
    svc.close().await.unwrap();
    // The first service's tokens are withdrawn on `os`, and `hs` reads them
    // gone only once the router has heard it. Read at once, a suite running
    // beside this one left both in the count below (#1151).
    let tokens = || async {
        hs.liveliness()
            .get("zk2/lab/w2/@zk/**")
            .await
            .unwrap()
            .into_iter()
            .count()
    };
    eventually("the closed service's tokens gone", || async {
        tokens().await == 0
    })
    .await;

    // On, with no catalog: refused at step 2, nothing declared.
    let mut b = ServiceBuilder::new(&os, config(W2).owner_heartbeat());
    b.implement(imp("pulse.v1")).unwrap();
    let _pulse = b
        .declare_state_writer(&"pulse.v1".parse().unwrap(), STATUS, &none)
        .await
        .unwrap();
    let Err(e) = b.start().await else {
        panic!("a service with `owner_heartbeat` and no catalog started")
    };
    assert!(
        matches!(&e, Error::NotExposed(why) if why.contains("owner_heartbeat") && why.contains("catalog")),
        "{e}"
    );
    assert_eq!(
        tokens().await,
        0,
        "no token of an instance that never started"
    );
}

/// v2 0.12 §2.10 (#986's Q0): while its clock guard holds, an owner writes
/// core §7.5's ahead time as its owner heartbeat's time, one time for
/// every group, never the unknown time. Each group goes on vouching, and a
/// subscriber reads the group as under any other time: it confirms, and
/// the time it is handed is the ahead time (§2.5, "The time takes no
/// part"). Before the guard holds and after it releases the time is an
/// instant.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_owner_heartbeat_says_ahead_while_the_guard_holds() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let hs = client(&ep).await;
    let ss = client(&ep).await;
    let tick = client(&ep).await;
    let cat = w2_catalog();
    let (_hh, host) = owner_heartbeats(&hs, W2_HOST).await;
    let none = Bindings::new();

    // The owner: `dial.v1` and `pulse.v1` covered by the `host` key, its
    // clock 2 s ahead, watching the clock reference.
    let mut cfg = config(W2).owner_heartbeat();
    cfg.clock_reference = Some("clock/heartbeat".to_owned());
    let mut b = ServiceBuilder::new(&os, cfg);
    b.catalog(cat.clone()).unwrap();
    for name in ["dial.v1", "pulse.v1"] {
        b.implement(imp(name)).unwrap();
    }
    b.minter().simulate_offset(2000);
    let dial = b
        .declare_state_writer(&"dial.v1".parse().unwrap(), "state/config", &none)
        .await
        .unwrap();
    let pulse = b
        .declare_state_writer(&"pulse.v1".parse().unwrap(), STATUS, &none)
        .await
        .unwrap();
    let svc = b.start().await.unwrap();
    let s_pulse = follow(&ss, "pulse.v1", STATUS, &cat).await;
    following(&s_pulse, "S follows pulse.v1's status").await;
    dial.put("x").await.unwrap();
    pulse.put("ok").await.unwrap();
    let pulse_key = "zk2/lab/w2/pulse.v1/state/status";
    let confirms = |h: &HeartbeatRead| matches!(&h.section, HeartbeatSection::Confirmed(keys) if keys == &[pulse_key.to_owned()]);
    let groups = |b: &OwnerBeat| indices(&b.hb) == [1, 2];

    // Before the drift is detected: an instant.
    let t0 = Instant::now();
    eventually("an owner heartbeat before the hold", || async {
        !owner_beats_after(&host, t0).is_empty()
    })
    .await;
    assert!(
        !svc.minter().is_ahead(),
        "no sample of the clock reference yet: the guard does not hold"
    );
    for b in owner_beats_after(&host, t0) {
        assert!(b.hb.time.is_instant(), "an instant: {:?}", b.hb.time);
    }

    // The guard holds: the ahead time on every owner heartbeat from the
    // detection on, both groups in each.
    eventually("the drift is detected", || async {
        tick.put("clock/heartbeat", "tick").await.unwrap();
        svc.minter().is_ahead()
    })
    .await;
    let detected = Instant::now();
    until(detected, Duration::from_millis(3200)).await;
    let held = owner_beats_after(&host, detected + JITTER);
    assert!(held.len() >= 2, "owner heartbeats go on: {}", held.len());
    for b in &held {
        assert_eq!(
            b.hb.time,
            Time::Ahead,
            "the ahead time while held, never the unknown time (§2.10)"
        );
        assert!(groups(b), "one time for both groups: {:?}", b.hb);
    }
    let read = s_pulse.reads_after(detected + JITTER);
    assert!(read.len() >= 2, "S reads them: {}", read.len());
    for h in &read {
        assert_eq!(h.owner_time, Some(Time::Ahead), "read as ahead");
        assert!(confirms(h), "it confirms as under any time (§2.5): {h:?}");
    }
    assert!(svc.minter().is_ahead(), "the guard still holds");

    // The guard releases: an instant again.
    svc.minter().simulate_offset(0);
    eventually("the drift is gone", || async {
        tick.put("clock/heartbeat", "tick").await.unwrap();
        !svc.minter().is_ahead()
    })
    .await;
    let released = Instant::now();
    until(released, Duration::from_millis(2500)).await;
    let after = owner_beats_after(&host, released + JITTER);
    assert!(after.len() >= 2, "{}", after.len());
    for b in &after {
        assert!(b.hb.time.is_instant(), "an instant again: {:?}", b.hb.time);
    }
    let read = s_pulse.reads_after(released + JITTER);
    assert!(
        read.iter()
            .all(|h| h.owner_time.is_some_and(Time::is_instant) && confirms(h)),
        "{read:?}"
    );
    svc.close().await.unwrap();
}

/// Which interfaces is the owner's choice, interface by interface (v2
/// §2.4): one its catalog does not list keeps its heartbeat, beside the
/// group of one it lists, and never both forms for either.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_interface_the_catalog_does_not_list_keeps_its_heartbeat() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let hs = client(&ep).await;
    let (_hh, host) = owner_heartbeats(&hs, W2_HOST).await;
    let (_hl, linkk) = puts_on(&hs, W2_LINK).await;
    let (_hi, per_iface) = puts_on(&hs, W2_HEARTBEATS).await;
    // The catalog lists dial.v1 alone.
    let mut cat = Catalog::new(0).unwrap();
    let dial = contract("dial.v1");
    cat.push(dial.iface.clone(), Fingerprint::of(&dial))
        .unwrap();

    let none = Bindings::new();
    let mut b = ServiceBuilder::new(&os, config(W2).owner_heartbeat());
    b.catalog(cat.clone()).unwrap();
    for name in ["dial.v1", "pulse.v1"] {
        b.implement(imp(name)).unwrap();
    }
    let _config = b
        .declare_state_writer(&dial.iface, "state/config", &none)
        .await
        .unwrap();
    let _pulse = b
        .declare_state_writer(&"pulse.v1".parse().unwrap(), STATUS, &none)
        .await
        .unwrap();
    let svc = b.start().await.unwrap();
    assert_eq!(svc.owner_heartbeat_cover(), [(dial.iface.clone(), 0)]);
    assert_eq!(fv2_unavailable(&svc), serde_json::Value::Null);
    eventually("two of each form", || async {
        host.lock().unwrap().len() >= 2 && per_iface.lock().unwrap().len() >= 2
    })
    .await;
    for b in host.lock().unwrap().iter() {
        assert_eq!(indices(&b.hb), [0], "dial.v1's group alone");
        group_of(&b.hb, &cat, "dial.v1");
    }
    for key in per_iface.lock().unwrap().iter() {
        assert!(
            key.ends_with("/heartbeat/pulse.v1"),
            "{key}: pulse.v1 keeps its heartbeat, and dial.v1 sends none"
        );
    }
    assert_eq!(
        linkk.lock().unwrap().len(),
        0,
        "no collection of the `link` class, so no put on its key"
    );
    svc.close().await.unwrap();
}

/// A standby's group is left out (v2 §2.4; `redundancy.v1` §2.4), and an
/// owner heartbeat that would hold no group is not sent. A serves the
/// governed `beacon.v2` and `pulse.v1`; B, a candidate of the governed
/// interface alone, stands by and puts nothing on either owner heartbeat
/// key. Once A closes, B takes over, and its owner heartbeats begin, in
/// its own run, with the governed interface's group.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_standbys_group_is_left_out() {
    const PAIR: &str = "lab/pair";
    const HOST: &str = "zk2/lab/pair/freshness.v2/stream/heartbeats/host";
    let (_r1, ep) = router(None).await;
    let hs = client(&ep).await;
    let governed = contract("beacon-governed.v2");
    let mut cat = Catalog::new(0).unwrap();
    cat.push(governed.iface.clone(), Fingerprint::of(&governed))
        .unwrap();
    let pulse = contract("pulse.v1");
    cat.push(pulse.iface.clone(), Fingerprint::of(&pulse))
        .unwrap();
    let (_hh, host) = owner_heartbeats(&hs, HOST).await;
    let (_hi, per_iface) = puts_on(&hs, "zk2/lab/pair/freshness.v2/stream/heartbeat/*").await;
    let none = Bindings::new();

    // A: both interfaces, serving.
    let (a_claim, a_watch, a_read) = (
        stamping(&ep).await,
        stamping(&ep).await,
        stamping(&ep).await,
    );
    let mut a = ServiceBuilder::new(&a_claim, config(PAIR).owner_heartbeat());
    a.catalog(cat.clone()).unwrap();
    a.implement(zenkey::Implementation::new(governed.clone()))
        .unwrap();
    a.implement(imp("pulse.v1")).unwrap();
    let _a_status = a
        .declare_state_writer(&beacon(), STATUS, &none)
        .await
        .unwrap();
    let _a_pulse = a
        .declare_state_writer(&pulse.iface, STATUS, &none)
        .await
        .unwrap();
    let a_red = a
        .redundancy(RedundancyStartup::new(a_read).with_watch(a_watch))
        .unwrap();
    let a_svc = a.start().await.unwrap();
    tokio::time::timeout(SETTLE, a_red.until(&beacon(), Role::Serving))
        .await
        .expect("A serves");
    eventually("A's owner heartbeat holds both groups", || async {
        host.lock()
            .unwrap()
            .last()
            .is_some_and(|b| indices(&b.hb) == [0, 1])
    })
    .await;
    let a_run = host.lock().unwrap().last().unwrap().stamp.run;

    // B: the governed interface alone, standing by behind A.
    let (b_claim, b_watch, b_read) = (
        stamping(&ep).await,
        stamping(&ep).await,
        stamping(&ep).await,
    );
    let mut b = ServiceBuilder::new(&b_claim, config(PAIR).owner_heartbeat());
    b.catalog(cat.clone()).unwrap();
    b.implement(zenkey::Implementation::new(governed)).unwrap();
    let _b_status = b
        .declare_state_writer(&beacon(), STATUS, &none)
        .await
        .unwrap();
    let b_red = b
        .redundancy(RedundancyStartup::new(b_read).with_watch(b_watch))
        .unwrap();
    let b_svc = b.start().await.unwrap();
    assert_eq!(b_svc.owner_heartbeat_cover(), [(beacon(), 0)]);
    eventually("B stands by behind A", || async {
        b_red.standing(&beacon()).why
            == Some(zenkey::model::redundancy::Why::Incumbent(
                a_svc.instance().clone(),
            ))
    })
    .await;
    // Over two of its periods and more: nothing in B's run.
    let standing = Instant::now();
    until(standing, Duration::from_millis(2500)).await;
    assert_eq!(b_red.role(&beacon()), Role::Standby);
    let all = host.lock().unwrap().clone();
    assert!(
        all.iter().all(|x| x.stamp.run == a_run),
        "a standby sends no owner heartbeat that would hold no group"
    );
    assert!(
        owner_beats_after(&host, standing)
            .iter()
            .all(|x| indices(&x.hb) == [0, 1]),
        "A's go on, both groups"
    );

    // A closes; B takes over, and its group goes out.
    a_svc.close().await.unwrap();
    tokio::time::timeout(SETTLE, b_red.until(&beacon(), Role::Serving))
        .await
        .expect("B takes over");
    eventually(
        "B's owner heartbeat, the governed interface's group",
        || async {
            host.lock()
                .unwrap()
                .last()
                .is_some_and(|x| x.stamp.run != a_run && indices(&x.hb) == [0])
        },
    )
    .await;
    assert_eq!(
        *per_iface.lock().unwrap(),
        Vec::<String>::new(),
        "never a heartbeat of a covered interface"
    );
    b_svc.close().await.unwrap();
}

/// A standby interface never stalls its class key (review of #1144). B
/// implements the governed `beacon.v2` and the ungoverned `pulse.v1`, and
/// stands by on the first behind A. Its `host` owner heartbeat keeps
/// coming at its period, in its own run, with `pulse.v1`'s group alone:
/// the governed interface's gate is asked at once and never waited for.
/// Once A closes, B takes over, and the governed interface's group
/// returns beside it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_standby_interface_never_stalls_the_groups_beside_it() {
    const PAIR: &str = "lab/pair";
    const HOST: &str = "zk2/lab/pair/freshness.v2/stream/heartbeats/host";
    let (_r1, ep) = router(None).await;
    let hs = client(&ep).await;
    let governed = contract("beacon-governed.v2");
    let pulse = contract("pulse.v1");
    let mut cat = Catalog::new(0).unwrap();
    cat.push(governed.iface.clone(), Fingerprint::of(&governed))
        .unwrap();
    cat.push(pulse.iface.clone(), Fingerprint::of(&pulse))
        .unwrap();
    let (_hh, host) = owner_heartbeats(&hs, HOST).await;
    let (_hi, per_iface) = puts_on(&hs, "zk2/lab/pair/freshness.v2/stream/heartbeat/*").await;
    let none = Bindings::new();

    // A: the governed interface alone, serving.
    let (a_claim, a_watch, a_read) = (
        stamping(&ep).await,
        stamping(&ep).await,
        stamping(&ep).await,
    );
    let mut a = ServiceBuilder::new(&a_claim, config(PAIR).owner_heartbeat());
    a.catalog(cat.clone()).unwrap();
    a.implement(zenkey::Implementation::new(governed.clone()))
        .unwrap();
    let _a_status = a
        .declare_state_writer(&beacon(), STATUS, &none)
        .await
        .unwrap();
    let a_red = a
        .redundancy(RedundancyStartup::new(a_read).with_watch(a_watch))
        .unwrap();
    let a_svc = a.start().await.unwrap();
    tokio::time::timeout(SETTLE, a_red.until(&beacon(), Role::Serving))
        .await
        .expect("A serves");
    eventually("A's owner heartbeat, its one group", || async {
        host.lock()
            .unwrap()
            .last()
            .is_some_and(|b| indices(&b.hb) == [0])
    })
    .await;
    let a_run = host.lock().unwrap().last().unwrap().stamp.run;

    // B: both interfaces, standing by on the governed one behind A.
    let (b_claim, b_watch, b_read) = (
        stamping(&ep).await,
        stamping(&ep).await,
        stamping(&ep).await,
    );
    let mut b = ServiceBuilder::new(&b_claim, config(PAIR).owner_heartbeat());
    b.catalog(cat.clone()).unwrap();
    b.implement(zenkey::Implementation::new(governed)).unwrap();
    b.implement(imp("pulse.v1")).unwrap();
    let _b_status = b
        .declare_state_writer(&beacon(), STATUS, &none)
        .await
        .unwrap();
    let b_pulse = b
        .declare_state_writer(&pulse.iface, STATUS, &none)
        .await
        .unwrap();
    let b_red = b
        .redundancy(RedundancyStartup::new(b_read).with_watch(b_watch))
        .unwrap();
    let b_svc = b.start().await.unwrap();
    assert_eq!(
        b_svc.owner_heartbeat_cover(),
        [(beacon(), 0), (pulse.iface.clone(), 1)]
    );
    b_pulse.put("ok").await.unwrap();
    eventually("B stands by behind A", || async {
        b_red.standing(&beacon()).why
            == Some(zenkey::model::redundancy::Why::Incumbent(
                a_svc.instance().clone(),
            ))
    })
    .await;
    let of_b = |since: Instant| -> Vec<OwnerBeat> {
        owner_beats_after(&host, since)
            .into_iter()
            .filter(|x| x.stamp.run != a_run)
            .collect()
    };

    // 5 s standing by: B's class key at its period, pulse.v1's group alone.
    let standing = Instant::now();
    until(standing, Duration::from_secs(5)).await;
    assert_eq!(b_red.role(&beacon()), Role::Standby);
    let while_standing = of_b(standing);
    assert_owner_cadence(&while_standing, "B's host key while it stands by");
    for x in &while_standing {
        assert_eq!(
            indices(&x.hb),
            [1],
            "pulse.v1's group alone: the standby's is left out"
        );
        assert_eq!(
            group_of(&x.hb, &cat, "pulse.v1")[0]
                .as_ref()
                .map(|s| s.members.clone()),
            Some(Members::One { vouched: true }),
            "and its member is vouched for all along"
        );
    }

    // A closes; B takes over, and the governed interface's group returns.
    a_svc.close().await.unwrap();
    tokio::time::timeout(SETTLE, b_red.until(&beacon(), Role::Serving))
        .await
        .expect("B takes over");
    let taken = Instant::now();
    eventually("B's owner heartbeat holds both groups", || async {
        of_b(taken).last().is_some_and(|x| indices(&x.hb) == [0, 1])
    })
    .await;
    // One sequence throughout: the group's return opens no gap on the key.
    let all = of_b(standing);
    for w in all.windows(2) {
        assert_eq!(
            w[1].stamp.seq.get(),
            w[0].stamp.seq.get() + 1,
            "the key's own consecutive seqs across the takeover"
        );
    }
    assert_eq!(
        *per_iface.lock().unwrap(),
        Vec::<String>::new(),
        "never a heartbeat of a covered interface"
    );
    b_svc.close().await.unwrap();
}

/// An owner on `freshness.v2`'s revision 1.0, which declares no owner
/// heartbeat key (review of #1144): implemented by the application before
/// any contract that uses the profile, it stays the service's revision.
/// The service starts, sends each interface's heartbeat, and lists no
/// `heartbeats/{class}` row, which its contract does not declare (D005).
/// With `owner_heartbeat` set under a catalog it starts all the same and
/// covers nothing: it has no key to put an owner heartbeat on.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_owner_on_the_first_revision_sends_heartbeats_alone() {
    const FIRST: &str = "8756e6a64a25424c962af374d844cf56c2ed1e831a4bac349fc17f6c0544a890";
    let bundle = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../spec/profiles/.history/freshness.v2/{FIRST}.bundle.json"
        )),
    )
    .unwrap();
    let first = zenkey::Implementation::from_bundle(&bundle).unwrap();
    assert!(
        first.resource(zenkey::heartbeat::HEARTBEAT).is_ok()
            && first.resource(zenkey::heartbeat::OWNER_HEARTBEATS).is_err(),
        "revision 1.0: the heartbeat's key alone"
    );
    let (_r1, ep) = router(None).await;
    let hs = client(&ep).await;
    let (_hh, host) = puts_on(&hs, W2_HOST).await;
    let (_hi, per_iface) = puts_on(&hs, W2_HEARTBEATS).await;
    let none = Bindings::new();
    for on in [false, true] {
        let os = client(&ep).await;
        let cfg = if on {
            config(W2).owner_heartbeat()
        } else {
            config(W2)
        };
        let mut b = ServiceBuilder::new(&os, cfg);
        b.catalog(w2_catalog()).unwrap();
        b.implement(first.clone()).unwrap();
        b.implement(imp("pulse.v1")).unwrap();
        let _pulse = b
            .declare_state_writer(&"pulse.v1".parse().unwrap(), STATUS, &none)
            .await
            .unwrap();
        let svc = b
            .start()
            .await
            .unwrap_or_else(|e| panic!("owner_heartbeat {on}: {e}"));
        assert!(
            svc.owner_heartbeat_cover().is_empty(),
            "nothing to cover with"
        );
        let d: serde_json::Value = serde_json::from_slice(&svc.descriptor_bytes()).unwrap();
        let fv2 = d["interfaces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["iface"] == "freshness.v2")
            .expect("freshness.v2 listed")
            .clone();
        assert_eq!(fv2["contract"], format!("sha256:{FIRST}"), "{fv2}");
        assert!(
            fv2.get("unavailable").is_none(),
            "no row for a resource its revision does not declare: {fv2}"
        );
        let before = per_iface.lock().unwrap().len();
        eventually("pulse.v1's heartbeat, twice", || async {
            per_iface.lock().unwrap().len() >= before + 2
        })
        .await;
        assert!(
            per_iface
                .lock()
                .unwrap()
                .iter()
                .all(|k| k.ends_with("/heartbeat/pulse.v1"))
        );
        svc.close().await.unwrap();
    }
    assert_eq!(host.lock().unwrap().len(), 0, "no owner heartbeat");
}
