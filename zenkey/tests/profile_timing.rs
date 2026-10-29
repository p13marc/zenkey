//! `spec/profiles/timing/scenarios.md` §1–§5 (`timing.v1`, #908, MB), one
//! test per section, named after it. §6 is a tool's.
//!
//! Common setup, the text's: R1 is a router with timestamping on, a
//! router's default, its HLC delta 500 ms; the owner `lab/pace` implements
//! `pace.v1` (`tests/contracts/pace.v1.toml`) with every resource exposed,
//! a client of R1; C is a consumer of it, binding `lab/pace`, following its
//! presence on a watch session of its own and reading on its own session.
//! Every judgement is `zenkey_model::timing`'s, driven by the runtime
//! (`zenkey::timing`).
//!
//! **The host** (the text's conventions): a report's due instant is
//! checked exactly against C's recorded arrivals, and its report instant
//! against the due plus the 10 ms bound, with [`ALLOWANCE`] for a loaded
//! runner's scheduler.

mod common;

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{client, config, contract, eventually, imp, router, router_with};
use zenkey::consumer::{Consumer, Delivery, Subscription};
use zenkey::model::grammar::IfaceId;
use zenkey::model::template::Bindings;
use zenkey::timing::{EndReason, OwnerState, Reason, SampleVerdict, TimingEvent};
use zenkey::writer::Writer;
use zenkey::{Service, ServiceBuilder};

const ADDR: &str = "lab/pace";
const CMD: &str = "stream/cmd";
const AXIS: &str = "stream/axis/{axis}";
const CMD_KEY: &str = "zk2/lab/pace/pace.v1/stream/cmd";

/// The owner's period, the deadline and its bound (scenarios.md).
const PERIOD: Duration = Duration::from_millis(100);
const DEADLINE: Duration = Duration::from_millis(500);
const BOUND: Duration = Duration::from_millis(10);
/// What a loaded runner's scheduler may add to a report's instant, beyond
/// the bound (the text's conventions).
const ALLOWANCE: Duration = Duration::from_millis(50);
/// ε, where a section states one (§4, §5).
const EPSILON_MS: u64 = 100;

fn pace() -> IfaceId {
    "pace.v1".parse().expect("an interface")
}

fn axis_key(a: &str) -> String {
    format!("zk2/lab/pace/pace.v1/stream/axis/{a}")
}

fn one(param: &str, value: &str) -> Bindings {
    [(param.to_owned(), vec![value.to_owned()])].into()
}

/// One delivery, as C records it.
#[derive(Debug, Clone)]
struct Rec {
    key: String,
    arrival: Instant,
    stamp: Option<String>,
    verdict: SampleVerdict,
    reason: Reason,
}

/// What C heard: its deliveries and its timing events, each in order.
#[derive(Default)]
struct Heard {
    deliveries: Mutex<Vec<Rec>>,
    events: Mutex<Vec<TimingEvent>>,
}

impl Heard {
    fn deliveries(&self) -> Vec<Rec> {
        self.deliveries.lock().unwrap().clone()
    }
    fn of(&self, key: &str) -> Vec<Rec> {
        self.deliveries()
            .into_iter()
            .filter(|r| r.key == key)
            .collect()
    }
    fn events(&self) -> Vec<TimingEvent> {
        self.events.lock().unwrap().clone()
    }
    fn events_of(&self, key: &str) -> Vec<TimingEvent> {
        self.events()
            .into_iter()
            .filter(|e| e.key() == key)
            .collect()
    }
}

/// C's subscription to `resource` through `c`, recording into `heard`.
async fn listen(c: &Consumer, resource: &str, heard: &Arc<Heard>) -> Subscription {
    let (d, e) = (Arc::clone(heard), Arc::clone(heard));
    c.subscribe_timed(
        resource,
        move |d_: Delivery| {
            d.deliveries.lock().unwrap().push(Rec {
                key: d_.sample.key_expr().as_str().to_owned(),
                arrival: d_.arrival,
                stamp: d_.stamp.map(|t| t.get_id().to_string()),
                verdict: d_.lifespan.verdict,
                reason: d_.lifespan.reason,
            });
        },
        move |ev| e.events.lock().unwrap().push(ev),
    )
    .await
    .unwrap()
}

/// A tool's consumer of `lab/pace`'s `pace.v1` on `s`, its watch session
/// `watch`, under ε `epsilon_ms`.
fn consumer(
    s: &zenoh::Session,
    watch: Option<&zenoh::Session>,
    epsilon_ms: Option<u64>,
) -> Consumer {
    let c = Consumer::for_tool(
        s,
        Arc::new(contract("pace.v1")),
        &[ADDR],
        &Default::default(),
    )
    .unwrap()
    .with_clock_bound(epsilon_ms.map(Duration::from_millis));
    match watch {
        Some(w) => c.with_watch(w.clone()),
        None => c,
    }
}

/// The owner, its `cmd` writer declared before start, and a publishing
/// loop per member it is told to run.
struct Owner {
    svc: Service,
    cmd: Arc<Writer>,
}

/// Brings up `lab/pace` on `s`, its clock offset by `offset_ms`.
async fn owner(s: &zenoh::Session, offset_ms: i64) -> Owner {
    let mut b = ServiceBuilder::new(s, config(ADDR));
    b.implement(imp("pace.v1")).unwrap();
    b.minter().simulate_offset(offset_ms);
    let cmd = b
        .declare_writer(&pace(), CMD, &Bindings::new())
        .await
        .unwrap();
    b.expose(&pace(), AXIS).unwrap();
    let svc = b.start().await.unwrap();
    assert!(cmd.is_stamped(), "§2.6: a lifespan's writer stamps");
    assert_eq!(cmd.timing().and_then(|t| t.period), Some(PERIOD));
    Owner {
        svc,
        cmd: Arc::new(cmd),
    }
}

/// Publishes on `w` every period while `on` holds, until `stop`; each
/// payload its sequence number.
fn publish(
    w: Arc<Writer>,
    on: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(PERIOD);
        let mut seq = 0u64;
        while !stop.load(Ordering::Relaxed) {
            tick.tick().await;
            if on.load(Ordering::Relaxed) {
                let _ = w.put(seq.to_string()).await;
                seq += 1;
            }
        }
    })
}

/// A miss's report, checked against the bound: due as the caller expects,
/// reported after it and within the bound plus the runner's allowance.
fn check_miss(ev: &TimingEvent, due: Instant, what: &str) {
    let TimingEvent::Missed {
        at, due: got, by, ..
    } = ev
    else {
        panic!("{what}: not a miss: {ev:?}");
    };
    assert_eq!(*got, due, "{what}: the due");
    assert_eq!(*by, due + BOUND, "{what}: the bound");
    assert!(*at > due, "{what}: reported after its due");
    assert!(
        at.duration_since(due) <= BOUND + ALLOWANCE,
        "{what}: reported {:?} after its due, beyond the bound and the allowance",
        at.duration_since(due)
    );
}

/// §1: a commander at its period, paused, then resumed (§2.4, §2.5).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_a_commander_paused_then_resumed() {
    let (_r1, ep) = router(None).await;
    let (cs, cw, os) = (client(&ep).await, client(&ep).await, client(&ep).await);
    let heard = Arc::new(Heard::default());
    let c = consumer(&cs, Some(&cw), None);
    let sub = listen(&c, CMD, &heard).await;
    let o = owner(&os, 0).await;
    eventually("C sees the owner present", || async {
        sub.owner_state(&ADDR.parse().unwrap()) == OwnerState::Present
    })
    .await;
    let (on, stop) = (
        Arc::new(AtomicBool::new(true)),
        Arc::new(AtomicBool::new(false)),
    );
    let task = publish(Arc::clone(&o.cmd), Arc::clone(&on), Arc::clone(&stop));

    // 1. Every 100 ms for 2 s: nothing reported.
    tokio::time::sleep(Duration::from_secs(2)).await;
    on.store(false, Ordering::Relaxed);
    // 2. Paused 2 s, its service, publisher and tokens kept.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let step1 = heard.of(CMD_KEY);
    assert!(step1.len() >= 15, "step 1 arrived: {}", step1.len());
    for w in step1.windows(2) {
        assert!(
            w[1].arrival - w[0].arrival <= DEADLINE,
            "a gap above the deadline"
        );
    }
    let last = step1.last().unwrap().arrival;
    let events = heard.events_of(CMD_KEY);
    assert_eq!(events.len(), 1, "one missed, nothing else: {events:?}");
    check_miss(&events[0], last + DEADLINE, "§1 step 2");
    assert!(
        heard.of(CMD_KEY).iter().all(|r| r.arrival <= last),
        "§2.4: no sample while paused"
    );
    // 4. T reads presence: the owner is present, its miss beside it.
    let t = client(&ep).await;
    let tokens = zenkey::presence::liveliness_keys(&t, "zk2/lab/pace/@zk/**", common::T)
        .await
        .unwrap();
    assert!(
        tokens.iter().any(|k| k.contains("/@zk/instance/")),
        "{tokens:?}"
    );
    assert!(
        tokens.iter().any(|k| k.contains("/@zk/alive/pace.v1/")),
        "{tokens:?}"
    );
    assert_eq!(sub.owner_state(&ADDR.parse().unwrap()), OwnerState::Present);

    // 3. Resumed for 1 s: met once, at the first arrival.
    on.store(true, Ordering::Relaxed);
    tokio::time::sleep(Duration::from_secs(1)).await;
    stop.store(true, Ordering::Relaxed);
    task.await.unwrap();
    let resumed: Vec<Rec> = heard
        .of(CMD_KEY)
        .into_iter()
        .filter(|r| r.arrival > last)
        .collect();
    assert!(!resumed.is_empty(), "step 3 arrived");
    let events = heard.events_of(CMD_KEY);
    assert_eq!(
        events.len(),
        2,
        "missed, then met, nothing after: {events:?}"
    );
    match &events[1] {
        TimingEvent::Met { at, .. } => {
            assert_eq!(*at, resumed[0].arrival, "met at the first arrival")
        }
        e => panic!("not met: {e:?}"),
    }
    drop(sub);
    o.svc.close().await.unwrap();
}

/// §2: a member gone, and a member silent (§2.5).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_a_member_gone_and_a_member_silent() {
    let (_r1, ep) = router(None).await;
    let (cs, cw, os) = (client(&ep).await, client(&ep).await, client(&ep).await);
    let heard = Arc::new(Heard::default());
    let c = consumer(&cs, Some(&cw), None);
    let sub = listen(&c, AXIS, &heard).await;
    let mut o = owner(&os, 0).await;
    let x = Arc::new(
        o.svc
            .writer(&pace(), AXIS, &one("axis", "x"))
            .await
            .unwrap(),
    );
    let y = Arc::new(
        o.svc
            .writer(&pace(), AXIS, &one("axis", "y"))
            .await
            .unwrap(),
    );
    o.svc.declare_member(&pace(), "x").await.unwrap();
    o.svc.declare_member(&pace(), "y").await.unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let (on_x, on_y) = (
        Arc::new(AtomicBool::new(true)),
        Arc::new(AtomicBool::new(true)),
    );
    let tx = publish(Arc::clone(&x), Arc::clone(&on_x), Arc::clone(&stop));
    let ty = publish(Arc::clone(&y), Arc::clone(&on_y), Arc::clone(&stop));

    // 1. Both every 100 ms: nothing reported.
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(heard.events().is_empty(), "{:?}", heard.events());
    // 2. y stops, and its member token is withdrawn.
    on_y.store(false, Ordering::Relaxed);
    o.svc.remove_member(&pace(), "y").await.unwrap();
    let (yk, xk) = (axis_key("y"), axis_key("x"));
    eventually("C reports axis/y ended", || async {
        !heard.events_of(&yk).is_empty()
    })
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let ys = heard.events_of(&yk);
    assert_eq!(ys.len(), 1, "ended, never missed: {ys:?}");
    assert!(
        matches!(
            ys[0],
            TimingEvent::Ended {
                reason: EndReason::Gone,
                ..
            }
        ),
        "{ys:?}"
    );
    assert!(
        heard.events_of(&xk).is_empty(),
        "{:?}",
        heard.events_of(&xk)
    );
    // 3. x stops, its member token kept: missed.
    on_x.store(false, Ordering::Relaxed);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let last_x = heard.of(&xk).last().unwrap().arrival;
    let xs = heard.events_of(&xk);
    assert_eq!(xs.len(), 1, "{xs:?}");
    check_miss(&xs[0], last_x + DEADLINE, "§2 step 3");
    stop.store(true, Ordering::Relaxed);
    tx.await.unwrap();
    ty.await.unwrap();
    drop(sub);
    o.svc.close().await.unwrap();
}

/// §3: a member never heard (§2.5).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_a_member_never_heard() {
    let (_r1, ep) = router(None).await;
    let (cs, cw, os) = (client(&ep).await, client(&ep).await, client(&ep).await);
    let heard = Arc::new(Heard::default());
    let c = consumer(&cs, Some(&cw), None);
    // 1. Subscribed at S, no owner: nothing waited for.
    let sub = listen(&c, CMD, &heard).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(heard.events().is_empty(), "{:?}", heard.events());
    assert_eq!(sub.owner_state(&ADDR.parse().unwrap()), OwnerState::Absent);
    // 2. The owner starts, and publishes nothing: missed at P + 500 ms.
    let t0 = Instant::now();
    let o = owner(&os, 0).await;
    let t1 = Instant::now();
    eventually("C reports cmd missed", || async {
        !heard.events_of(CMD_KEY).is_empty()
    })
    .await;
    let ev = heard.events_of(CMD_KEY)[0].clone();
    let TimingEvent::Missed { due, at, owner, .. } = &ev else {
        panic!("{ev:?}");
    };
    assert_eq!(*owner, OwnerState::Present);
    // P, when C first saw the owner's token, lies between the start's
    // first step and a round trip after it returned.
    assert!(*due >= t0 + DEADLINE, "due before the owner started");
    assert!(
        *due <= t1 + DEADLINE + Duration::from_millis(500),
        "due long after P"
    );
    assert!(
        at.duration_since(*due) <= BOUND + ALLOWANCE,
        "{:?}",
        at.duration_since(*due)
    );
    // 3. One sample: met at its arrival.
    tokio::time::sleep(Duration::from_millis(500)).await;
    o.cmd.put("0").await.unwrap();
    eventually("C reports cmd met", || async {
        heard.events_of(CMD_KEY).len() >= 2
    })
    .await;
    let arrival = heard.of(CMD_KEY)[0].arrival;
    match &heard.events_of(CMD_KEY)[1] {
        TimingEvent::Met { at, .. } => assert_eq!(*at, arrival),
        e => panic!("not met: {e:?}"),
    }
    // 4. C2 on axis/*, no axis published: nothing, none being known.
    let (c2s, c2w) = (client(&ep).await, client(&ep).await);
    let heard2 = Arc::new(Heard::default());
    let c2 = consumer(&c2s, Some(&c2w), None);
    let sub2 = listen(&c2, AXIS, &heard2).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(heard2.events().is_empty(), "{:?}", heard2.events());
    // 5. C3 binds axis to x, a known member of an epoch template (text
    // 0.2, F-261): its owner present, it is waited for by nothing until a
    // member token of x is seen, then missed from that instant.
    let mut o = o;
    let (c3s, c3w) = (client(&ep).await, client(&ep).await);
    let heard3 = Arc::new(Heard::default());
    let c3 = Consumer::for_tool(
        &c3s,
        Arc::new(contract("pace.v1")),
        &[ADDR],
        &[("axis".to_owned(), "x".to_owned())].into(),
    )
    .unwrap()
    .with_watch(c3w.clone());
    let sub3 = listen(&c3, AXIS, &heard3).await;
    assert_eq!(
        sub3.owner_state(&ADDR.parse().unwrap()),
        OwnerState::Present
    );
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        heard3.events().is_empty(),
        "nothing waited for: {:?}",
        heard3.events()
    );
    let m0 = Instant::now();
    o.svc.declare_member(&pace(), "x").await.unwrap();
    let m1 = Instant::now();
    let xk = axis_key("x");
    eventually("C3 reports axis/x missed", || async {
        !heard3.events_of(&xk).is_empty()
    })
    .await;
    let ev = heard3.events_of(&xk)[0].clone();
    let TimingEvent::Missed { due, at, .. } = &ev else {
        panic!("{ev:?}");
    };
    assert!(*due >= m0 + DEADLINE, "due before the member token");
    assert!(
        *due <= m1 + DEADLINE + Duration::from_millis(500),
        "due long after it"
    );
    assert!(
        at.duration_since(*due) <= BOUND + ALLOWANCE,
        "{:?}",
        at.duration_since(*due)
    );
    drop((sub, sub2, sub3));
    o.svc.close().await.unwrap();
}

/// A client of the router at `ep` whose HLC is off, stated rather than
/// left to a client's default: `new_timestamp()` is then its wall clock
/// with its zid (core §4.1).
async fn client_without_hlc(ep: &str) -> zenoh::Session {
    let mut c = zenoh::Config::default();
    c.insert_json5("mode", "\"client\"").unwrap();
    c.insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    c.insert_json5("scouting/gossip/enabled", "false").unwrap();
    c.insert_json5("connect/endpoints", &format!("[\"{ep}\"]"))
        .unwrap();
    c.insert_json5("timestamping/enabled", "false").unwrap();
    zenoh::open(c).await.unwrap()
}

/// One step of §4: a new owner, its clock offset, and a new C under ε.
async fn lifespan_step(
    ep: &str,
    epsilon_ms: Option<u64>,
    offset_ms: Arc<AtomicI64>,
    change_after: Option<(Duration, i64)>,
) -> (Arc<Heard>, u64, zenoh::Session, Instant, Instant) {
    // §4's setup (text 0.3, F-273): the owner's session has no HLC, so the
    // offset moves its stamps alone; one would follow its own first stamp
    // ahead (core §4.1, 0.53), and its clock would stand still.
    let (cs, cw, os) = (
        client(ep).await,
        client(ep).await,
        client_without_hlc(ep).await,
    );
    let heard = Arc::new(Heard::default());
    // C is a service's consumer here, its ε its binding's clock_bound_ms.
    let mut cfg = config("lab/reader").bind("cmd", &[ADDR]);
    cfg.bindings.get_mut("cmd").unwrap().clock_bound_ms = epsilon_ms;
    let mut b = ServiceBuilder::new(&cs, cfg);
    b.require("cmd", pace(), false);
    let reader = b.start().await.unwrap();
    let c = reader
        .consumer("cmd", Arc::new(contract("pace.v1")))
        .unwrap()
        .with_watch(cw.clone());
    assert_eq!(c.clock_bound(), epsilon_ms.map(Duration::from_millis));
    let sub = listen(&c, CMD, &heard).await;
    let t0 = Instant::now();
    let o = owner(&os, offset_ms.load(Ordering::Relaxed)).await;
    let t1 = Instant::now();
    // F-266: the owner publishes once C holds its descriptor, so no
    // sample reads unattributed (§2.6 step 6).
    if epsilon_ms.is_some() {
        let zid = zenkey::timing::zid_value(&os.zid().to_string()).unwrap();
        eventually("C holds the owner's meta.zid", || async {
            sub.owner_zids(&ADDR.parse().unwrap()).contains(&zid)
        })
        .await;
    }
    let (on, stop) = (
        Arc::new(AtomicBool::new(true)),
        Arc::new(AtomicBool::new(false)),
    );
    let task = publish(Arc::clone(&o.cmd), on, Arc::clone(&stop));
    match change_after {
        Some((after, ms)) => {
            tokio::time::sleep(after).await;
            o.svc.minter().simulate_offset(ms);
            tokio::time::sleep(Duration::from_secs(2) - after).await;
        }
        None => tokio::time::sleep(Duration::from_secs(2)).await,
    }
    stop.store(true, Ordering::Relaxed);
    task.await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let expired = sub.expired();
    drop(sub);
    o.svc.close().await.unwrap();
    reader.close().await.unwrap();
    (heard, expired, os, t0, t1)
}

/// §4: the lifespan on a stated bound (§2.5, §2.6).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_the_lifespan_on_a_stated_bound() {
    let (_r1, ep) = router(None).await;

    // 1. Clocks agreeing: every sample live, delivered, its stamp the
    // owner's; the deadline met.
    let (h, expired, os, ..) =
        lifespan_step(&ep, Some(EPSILON_MS), Arc::new(AtomicI64::new(0)), None).await;
    let d = h.of(CMD_KEY);
    assert!(d.len() >= 15, "{}", d.len());
    let zid = os.zid().to_string();
    for r in &d {
        assert_eq!(r.stamp.as_deref(), Some(zid.as_str()), "the owner's stamp");
        assert_eq!(
            (r.verdict, r.reason),
            (SampleVerdict::Live, Reason::Live),
            "{r:?}"
        );
    }
    assert_eq!(expired, 0);
    assert!(h.events().is_empty(), "{:?}", h.events());

    // 2. The owner 700 ms behind: every sample expired, none delivered;
    // missed from the owner's presence, never met.
    let (h, expired, _os, t0, t1) =
        lifespan_step(&ep, Some(EPSILON_MS), Arc::new(AtomicI64::new(-700)), None).await;
    assert!(h.of(CMD_KEY).is_empty(), "{:?}", h.of(CMD_KEY));
    let evs = h.events_of(CMD_KEY);
    let expired_evs = evs
        .iter()
        .filter(|e| matches!(e, TimingEvent::Expired { .. }))
        .count();
    assert!(expired_evs >= 15, "{expired_evs}");
    assert_eq!(expired, expired_evs as u64, "every discard counted");
    let misses: Vec<&TimingEvent> = evs
        .iter()
        .filter(|e| matches!(e, TimingEvent::Missed { .. }))
        .collect();
    assert_eq!(misses.len(), 1, "{evs:?}");
    let TimingEvent::Missed { due, .. } = misses[0] else {
        unreachable!()
    };
    assert!(*due >= t0 + DEADLINE && *due <= t1 + DEADLINE + Duration::from_millis(500));
    assert!(
        !evs.iter().any(|e| matches!(e, TimingEvent::Met { .. })),
        "{evs:?}"
    );

    // 3. The owner 400 ms ahead for a second, then agreeing: every sample
    // `clock_disagrees`, delivered; the deadline met.
    let (h, expired, ..) = lifespan_step(
        &ep,
        Some(EPSILON_MS),
        Arc::new(AtomicI64::new(400)),
        Some((Duration::from_secs(1), 0)),
    )
    .await;
    let d = h.of(CMD_KEY);
    assert!(d.len() >= 15, "{}", d.len());
    for r in &d {
        assert_eq!(
            (r.verdict, r.reason),
            (SampleVerdict::Unobservable, Reason::ClockDisagrees),
            "falsified for the reading: {r:?}"
        );
    }
    assert_eq!(expired, 0);
    assert!(h.events().is_empty(), "{:?}", h.events());

    // 4. No ε, the owner 700 ms behind: unobservable, nothing discarded,
    // though R1's delta is 500 ms.
    let (h, expired, ..) = lifespan_step(&ep, None, Arc::new(AtomicI64::new(-700)), None).await;
    let d = h.of(CMD_KEY);
    assert!(d.len() >= 15, "{}", d.len());
    for r in &d {
        assert_eq!(
            (r.verdict, r.reason),
            (SampleVerdict::Unobservable, Reason::NoBound),
            "{r:?}"
        );
    }
    assert_eq!(expired, 0);
    assert!(h.events().is_empty(), "{:?}", h.events());
}

/// One step of §5: the owner publishes `cmd` raw through `ep`, its own
/// wall clock less 10 s as the payload, every period for 2 s; C reads.
async fn unstamped_step(ep: &str, owner_hlc: bool) -> (Arc<Heard>, String) {
    let cs = client(ep).await;
    let cw = client(ep).await;
    let os = if owner_hlc {
        let mut c = zenoh::Config::default();
        c.insert_json5("mode", "\"client\"").unwrap();
        c.insert_json5("scouting/multicast/enabled", "false")
            .unwrap();
        c.insert_json5("scouting/gossip/enabled", "false").unwrap();
        c.insert_json5("connect/endpoints", &format!("[\"{ep}\"]"))
            .unwrap();
        c.insert_json5("timestamping/enabled", "true").unwrap();
        zenoh::open(c).await.unwrap()
    } else {
        client(ep).await
    };
    let heard = Arc::new(Heard::default());
    let c = consumer(&cs, Some(&cw), Some(EPSILON_MS));
    let sub = listen(&c, CMD, &heard).await;
    // The owner's service states its session's zid as meta.zid; the
    // samples go round its writer, raw, as the section means them to.
    let mut b = ServiceBuilder::new(&os, config(ADDR));
    b.implement(imp("pace.v1")).unwrap();
    b.expose(&pace(), CMD).unwrap();
    b.expose(&pace(), AXIS).unwrap();
    let svc = b.start().await.unwrap();
    // F-266: C holds the owner's meta.zid before the first sample.
    let zid = zenkey::timing::zid_value(&os.zid().to_string()).unwrap();
    eventually("C holds the owner's meta.zid", || async {
        sub.owner_zids(&ADDR.parse().unwrap()).contains(&zid)
    })
    .await;
    let publisher = os.declare_publisher(CMD_KEY).await.unwrap();
    for _ in 0..20 {
        let ten_s_ago = std::time::SystemTime::now() - Duration::from_secs(10);
        let ns = ten_s_ago
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        publisher.put(ns.to_string()).await.unwrap();
        tokio::time::sleep(PERIOD).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(sub.expired(), 0, "never discarded");
    let zid = os.zid().to_string();
    drop(sub);
    drop(publisher);
    svc.close().await.unwrap();
    (heard, zid)
}

/// §5: an unstamped owner (§2.6).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_an_unstamped_owner() {
    // 1. Through R1, which stamps: the router's stamp, foreign.
    let (r1, ep1) = router(None).await;
    let (h, _) = unstamped_step(&ep1, false).await;
    let d = h.of(CMD_KEY);
    assert!(d.len() >= 15, "{}", d.len());
    let r1_zid = r1.zid().to_string();
    for r in &d {
        assert_eq!(r.stamp.as_deref(), Some(r1_zid.as_str()), "R1's stamp");
        assert_eq!(
            (r.verdict, r.reason),
            (SampleVerdict::Unobservable, Reason::ForeignStamp),
            "{r:?}"
        );
    }
    assert!(h.events().is_empty(), "the deadline met: {:?}", h.events());

    // 2. Through R2, which stamps nothing: unstamped.
    let (_r2, ep2) = router_with(None, &[("timestamping/enabled", "false")]).await;
    let (h, _) = unstamped_step(&ep2, false).await;
    let d = h.of(CMD_KEY);
    assert!(d.len() >= 15, "{}", d.len());
    for r in &d {
        assert_eq!(r.stamp, None);
        assert_eq!(
            (r.verdict, r.reason),
            (SampleVerdict::Unobservable, Reason::Unstamped),
            "{r:?}"
        );
    }

    // 3. Control: the owner's HLC on, through R1: its own stamp, live. In
    // no step is the payload's time read: it says 10 s old.
    let (h, zid) = unstamped_step(&ep1, true).await;
    let d = h.of(CMD_KEY);
    assert!(d.len() >= 15, "{}", d.len());
    for r in &d {
        assert_eq!(r.stamp.as_deref(), Some(zid.as_str()), "the owner's stamp");
        assert_eq!(
            (r.verdict, r.reason),
            (SampleVerdict::Live, Reason::Live),
            "{r:?}"
        );
    }
}

/// §2.6 at the owner: a configuration stating another `meta.zid` than the
/// session's is refused at start, nothing declared.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lifespan_owner_states_its_own_zid() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let mut cfg = config(ADDR);
    cfg.meta
        .insert("zid".to_owned(), serde_json::json!("1234abcd"));
    let mut b = ServiceBuilder::new(&os, cfg);
    b.implement(imp("pace.v1")).unwrap();
    b.expose(&pace(), CMD).unwrap();
    b.expose(&pace(), AXIS).unwrap();
    let e = b.start().await.err().expect("refused");
    assert!(e.to_string().contains("timing.v1 §2.6"), "{e}");
    let t = client(&ep).await;
    let tokens = zenkey::presence::liveliness_keys(&t, "zk2/lab/pace/@zk/**", common::T)
        .await
        .unwrap();
    assert!(tokens.is_empty(), "nothing declared: {tokens:?}");

    // Its own zid, however spelled, is accepted.
    let mut cfg = config(ADDR);
    let own = os.zid().to_string().to_uppercase();
    cfg.meta.insert("zid".to_owned(), serde_json::json!(own));
    let mut b = ServiceBuilder::new(&os, cfg);
    b.implement(imp("pace.v1")).unwrap();
    b.expose(&pace(), CMD).unwrap();
    b.expose(&pace(), AXIS).unwrap();
    b.start().await.unwrap().close().await.unwrap();
}
