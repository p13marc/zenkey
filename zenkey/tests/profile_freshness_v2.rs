//! `spec/profiles/freshness/scenarios-v2.md` §1–§7 (`freshness.v2`, #978's
//! F3), one test per section, named after it, and §2's step 6, a
//! candidate's, in a test of its own; #1080's heartbeat time under a
//! catalog's epoch, beside the same owner's frames, is the last test.
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
    // carries no Zenoh timestamp, and the heartbeats go on vouching, the
    // time unknown (§2.10). S2 judges `status` fresh past its horizon; G,
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
        assert_eq!(b.hb.time, Time::Unknown, "the time unknown while held");
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
/// the time unknown, and clear `status`'s bit from the held put until a
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
    // heartbeats vouch for it, the time unknown.
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
            .any(|b| b.hb.time == Time::Unknown)
    })
    .await;
    for b in beats_after(&beats, detected + JITTER) {
        assert_eq!(b.hb.time, Time::Unknown, "the time unknown while held");
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
        assert_eq!(b.hb.time, Time::Unknown, "the time unknown while held");
        let s = section(b);
        assert_eq!(
            s.members,
            Members::One { vouched: false },
            "its bit cleared"
        );
        assert_eq!(s.digest, Digest::of([(STATUS_OF, up)]), "the value held");
    }

    // The guard releases: the time is known again, the bit still clear.
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
        assert_ne!(b.hb.time, Time::Unknown, "the time known again");
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
