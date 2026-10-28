//! `spec/profiles/arbitration/scenarios.md` §1–§9 and §11 (`arbitration.v1`,
//! #909, BB), one test per section, named after it. §10 is a tool's (BC).
//!
//! The text's setup: R1 a router; the commanders `C_s`, `C_t`, `C_a`
//! owners of `steer.v1` (`tests/contracts/steer.v1.toml`) at `lab/safety`,
//! `lab/teleop` and `lab/autopilot`, each on a session of its own with its
//! HLC on, behind a link of its own the test can cut (its kill: R1 closes
//! the link and withdraws its tokens at once, as a SIGKILL's closed links
//! do); each puts `cmd` every 50 ms, `<service> <n>`, from 500 ms after
//! its tokens (text 0.2, F-269). The actuators `A_l` and `A_r` implement
//! `rudder.v1` at `lab/rudder-l` and `lab/rudder-r`, `cmd` bound to
//! `["lab/safety", "lab/teleop", "lab/autopilot"]`, each on three
//! sessions: its service's, which holds its subscription and makes its
//! reads again; a watch session; and the commanders' own are apart. Each
//! puts every command it delivers on `angle`. S, the tool, subscribes to
//! every `angle`. Every decision is `zenkey_model::arbitration`'s,
//! driven by the runtime (`zenkey::arbitration`).
//!
//! **Margins** (the text's conventions): a switch's own instants
//! (`at − last`) are the actuator's, allowed [`MARGIN`] beyond the 306 ms
//! detection bound for a loaded runner's scheduler.

mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{SETTLE, T, client, contract, eventually, router};
use zenkey::arbitration::{Arbitration, ArbitrationEvent, Cause, Switched};
use zenkey::model::contract::{Contract, load_str};
use zenkey::model::grammar::{Addr, IfaceId, InstanceId};
use zenkey::model::template::Bindings;
use zenkey::redundancy::{RedundancyStartup, Role};
use zenkey::state::StateWriter;
use zenkey::writer::Writer;
use zenkey::{Error, Service, ServiceBuilder, ServiceConfig};
use zenoh::sample::{Sample, SampleKind};

/// The deadline, its detection bound, and what a loaded runner may add.
const DEADLINE: Duration = Duration::from_millis(300);
const BOUND: Duration = Duration::from_millis(306);
const MARGIN: Duration = Duration::from_millis(50);
/// A commander's period, and how long after its tokens it starts putting
/// (text 0.2, F-269).
const PERIOD: Duration = Duration::from_millis(50);
const LEAD: Duration = Duration::from_millis(500);
/// The first 16 digits of `steer.v1`'s fingerprint (§4's X).
const FP16: &str = "5162db057a949bfe";
const BINDING: [&str; 3] = ["lab/safety", "lab/teleop", "lab/autopilot"];

fn steer() -> IfaceId {
    "steer.v1".parse().unwrap()
}

fn rudder() -> IfaceId {
    "rudder.v1".parse().unwrap()
}

fn a(s: &str) -> Addr {
    s.parse().unwrap()
}

fn cmd_key(service: &str) -> String {
    format!("zk2/lab/{service}/steer.v1/stream/cmd")
}

/// A contract from its text, as a copy of a test contract is.
fn from_text(text: &str, name: &str) -> Contract {
    let l = load_str(text, Path::new("."), Some(name));
    l.contract
        .unwrap_or_else(|| panic!("{name} does not load:\n{}", l.report))
}

/// A test contract's text, with `from` replaced by `to`.
fn copy(name: &str, from: &str, to: &str) -> Contract {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/contracts")
        .join(format!("{name}.toml"));
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains(from), "{name} holds {from:?}");
    from_text(&text.replace(from, to), &format!("{name}.toml"))
}

/// A client of `ep` with its HLC on, retrying fast after a cut.
async fn session(ep: &str) -> zenoh::Session {
    let mut c = zenoh::Config::default();
    c.insert_json5("mode", "\"client\"").unwrap();
    c.insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    c.insert_json5("scouting/gossip/enabled", "false").unwrap();
    c.insert_json5("connect/endpoints", &format!("[\"{ep}\"]"))
        .unwrap();
    c.insert_json5(
        "connect/retry",
        "{ period_init_ms: 100, period_max_ms: 500, period_increase_factor: 1.5 }",
    )
    .unwrap();
    c.insert_json5("timestamping/enabled", "true").unwrap();
    common::panic_watchdog::install();
    zenoh::open(c).await.expect("a session")
}

// --- the commanders --------------------------------------------------------------

/// A commander: its service, its link, and its putting loop.
struct Commander {
    svc: Option<Service>,
    session: zenoh::Session,
    link: common::Link,
    on: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl Commander {
    /// `C_<name>` at `lab/<name>`, behind a link of its own, putting from
    /// [`LEAD`] after its start.
    async fn start(ep: &str, name: &'static str) -> Self {
        Self::start_on(ep, name, contract("steer.v1"), None).await
    }

    /// [`Commander::start`] on the contract `c`, and, when `instance` is
    /// given, a candidate of its governed interface with that id
    /// (`redundancy.v1`, §11).
    async fn start_on(ep: &str, name: &'static str, c: Contract, instance: Option<u64>) -> Self {
        let link = common::link_to(ep).await;
        let s = session(link.endpoint()).await;
        let mut b = ServiceBuilder::new(&s, ServiceConfig::new(a(&format!("lab/{name}"))));
        if let Some(i) = instance {
            b.set_instance_id(InstanceId::from_u64(i));
        }
        b.implement(zenkey::Implementation::new(c)).unwrap();
        let w = b
            .declare_writer(&steer(), "stream/cmd", &Bindings::new())
            .await
            .unwrap();
        if instance.is_some() {
            let (watch, reader) = (
                session(link.endpoint()).await,
                session(link.endpoint()).await,
            );
            b.redundancy(RedundancyStartup::new(reader).with_watch(watch))
                .unwrap();
        }
        let svc = b.start().await.unwrap();
        let on = Arc::new(AtomicBool::new(true));
        let task = put_loop(Arc::new(w), name, Arc::clone(&on));
        Self {
            svc: Some(svc),
            session: s,
            link,
            on,
            task,
        }
    }

    fn hold(&self) {
        self.on.store(false, Ordering::SeqCst);
    }

    fn resume(&self) {
        self.on.store(true, Ordering::SeqCst);
    }

    /// Its kill: its link closed, R1 withdrawing its tokens at once.
    fn kill(&self) {
        self.task.abort();
        self.link.cut();
    }

    fn svc(&mut self) -> &mut Service {
        self.svc.as_mut().unwrap()
    }
}

impl Drop for Commander {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Puts `<name> <n>` on `w` every [`PERIOD`] while `on`, from [`LEAD`]
/// on; a write a standby's writer refuses is skipped.
fn put_loop(
    w: Arc<Writer>,
    name: &'static str,
    on: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        tokio::time::sleep(LEAD).await;
        let mut tick = tokio::time::interval(PERIOD);
        let mut n = 0u64;
        loop {
            tick.tick().await;
            if on.load(Ordering::SeqCst) && w.put(format!("{name} {n}")).await.is_ok() {
                n += 1;
            }
        }
    })
}

// --- the actuators -----------------------------------------------------------------

/// What an actuator told its application.
#[derive(Default)]
struct Rec {
    switches: Mutex<Vec<Switched>>,
    /// Each delivery: its arrival, its source and its payload.
    delivered: Mutex<Vec<(Instant, Addr, String)>>,
}

impl Rec {
    fn switches(&self) -> Vec<Switched> {
        self.switches.lock().unwrap().clone()
    }

    fn lines(&self) -> Vec<(Option<String>, Option<String>, Cause)> {
        self.switches()
            .iter()
            .map(|s| {
                (
                    s.from.as_ref().map(ToString::to_string),
                    s.to.as_ref().map(ToString::to_string),
                    s.cause,
                )
            })
            .collect()
    }

    fn delivered(&self) -> Vec<(Instant, Addr, String)> {
        self.delivered.lock().unwrap().clone()
    }
}

/// An actuator: its service, its arbitration, what it told, its sessions.
struct Actuator {
    name: String,
    svc: Option<Service>,
    arb: Option<Arbitration>,
    rec: Arc<Rec>,
    sessions: Vec<zenoh::Session>,
}

impl Actuator {
    fn arb(&self) -> &Arbitration {
        self.arb.as_ref().unwrap()
    }

    fn active(&self) -> Option<String> {
        self.arb().active().map(|a| a.to_string())
    }

    fn angle_key(&self) -> String {
        format!("zk2/lab/{}/rudder.v1/state/angle", self.name)
    }

    async fn stop(mut self) {
        drop(self.arb.take());
        if let Some(s) = self.svc.take() {
            s.close().await.unwrap();
        }
        for s in &self.sessions {
            let _ = s.close().await;
        }
    }
}

/// How an actuator is built.
struct Build<'a> {
    name: &'a str,
    binding: &'a [&'a str],
    rudder: Contract,
    steer: Contract,
    /// Its watch session's endpoint, `None` for none.
    watch: Option<&'a str>,
}

impl<'a> Build<'a> {
    fn new(name: &'a str, watch: &'a str) -> Self {
        Self {
            name,
            binding: &BINDING,
            rudder: contract("rudder.v1"),
            steer: contract("steer.v1"),
            watch: Some(watch),
        }
    }
}

/// Starts an actuator through `ep`, or why it refused.
async fn actuator(ep: &str, o: Build<'_>) -> Result<Actuator, Error> {
    let s = session(ep).await;
    let cfg = ServiceConfig::new(a(&format!("lab/{}", o.name))).bind("cmd", o.binding);
    let mut b = ServiceBuilder::new(&s, cfg);
    b.implement(zenkey::Implementation::new(o.rudder))?;
    let steer = Arc::new(o.steer);
    b.arbitrate("cmd", Arc::clone(&steer))?;
    let angle = Arc::new(
        b.declare_state_writer(&rudder(), "state/angle", &Bindings::new())
            .await?,
    );
    let svc = b.start().await?;
    let mut sessions = vec![s.clone()];
    let mut c = svc.consumer("cmd", steer)?;
    if let Some(w) = o.watch {
        let w = session(w).await;
        c = c.with_watch(w.clone());
        sessions.push(w);
    }
    let rec = Arc::new(Rec::default());
    let r = Arc::clone(&rec);
    let arb = c
        .arbitrate(move |ev| match ev {
            ArbitrationEvent::Switched(s) => r.switches.lock().unwrap().push(s),
            ArbitrationEvent::Delivered { source, delivery } => {
                let payload =
                    String::from_utf8_lossy(&delivery.sample.payload().to_bytes()).into_owned();
                r.delivered
                    .lock()
                    .unwrap()
                    .push((delivery.arrival, source, payload.clone()));
                let angle: Arc<StateWriter> = Arc::clone(&angle);
                tokio::spawn(async move {
                    let _ = angle.put(payload).await;
                });
            }
        })
        .await?;
    Ok(Actuator {
        name: o.name.to_owned(),
        svc: Some(svc),
        arb: Some(arb),
        rec,
        sessions,
    })
}

// --- S ---------------------------------------------------------------------------------

/// Every `angle` put S saw, by key, each with its arrival.
type Angles = Arc<Mutex<BTreeMap<String, Vec<(Instant, String)>>>>;

/// S: every `angle` put, by key, with its arrival.
struct Tool {
    angles: Angles,
    _sub: zenoh::pubsub::Subscriber<()>,
    session: zenoh::Session,
}

impl Tool {
    async fn new(ep: &str) -> Self {
        let session = client(ep).await;
        let angles: Angles = Arc::default();
        let g = Arc::clone(&angles);
        let sub = session
            .declare_subscriber("zk2/lab/*/rudder.v1/state/angle")
            .callback(move |s: Sample| {
                if s.kind() != SampleKind::Put {
                    return;
                }
                let payload = String::from_utf8_lossy(&s.payload().to_bytes()).into_owned();
                g.lock()
                    .unwrap()
                    .entry(s.key_expr().as_str().to_owned())
                    .or_default()
                    .push((Instant::now(), payload));
            })
            .await
            .unwrap();
        Self {
            angles,
            _sub: sub,
            session,
        }
    }

    /// The payloads S saw on `act`'s `angle`.
    fn angle(&self, act: &Actuator) -> Vec<(Instant, String)> {
        self.angles
            .lock()
            .unwrap()
            .get(&act.angle_key())
            .cloned()
            .unwrap_or_default()
    }

    /// The payloads S saw on `act`'s `angle` from `since` on.
    fn angle_since(&self, act: &Actuator, since: Instant) -> Vec<String> {
        self.angle(act)
            .into_iter()
            .filter(|(at, _)| *at >= since)
            .map(|(_, p)| p)
            .collect()
    }
}

// --- the setups ----------------------------------------------------------------------

/// §2's setup: R1, S, both actuators, then all three commanders, each
/// actuator following safety.
struct World {
    _r1: zenoh::Session,
    ep: String,
    s: Tool,
    acts: Vec<Actuator>,
    c: BTreeMap<&'static str, Commander>,
    /// The actuators' watch sessions' link (§7), when they share one.
    watch_link: Option<common::Link>,
}

impl World {
    async fn following_safety(watch_link: bool) -> Self {
        let (r1, ep) = router(None).await;
        let s = Tool::new(&ep).await;
        let link = if watch_link {
            Some(common::link_to(&ep).await)
        } else {
            None
        };
        let watch_ep = link
            .as_ref()
            .map_or(ep.clone(), |l| l.endpoint().to_owned());
        let mut acts = Vec::new();
        for n in ["rudder-l", "rudder-r"] {
            acts.push(actuator(&ep, Build::new(n, &watch_ep)).await.unwrap());
        }
        let mut c = BTreeMap::new();
        for n in ["autopilot", "teleop", "safety"] {
            c.insert(n, Commander::start(&ep, n).await);
        }
        let w = Self {
            _r1: r1,
            ep,
            s,
            acts,
            c,
            watch_link: link,
        };
        w.all_follow("lab/safety").await;
        w
    }

    async fn all_follow(&self, who: &str) {
        for act in &self.acts {
            eventually(&format!("{} follows {who}", act.name), || async {
                act.active().as_deref() == Some(who)
            })
            .await;
        }
    }

    fn c(&self, n: &str) -> &Commander {
        &self.c[n]
    }

    async fn stop(self) {
        for a in self.acts {
            a.stop().await;
        }
    }
}

/// Every switch of `act` after its first `n`.
fn after(act: &Actuator, n: usize) -> Vec<Switched> {
    act.rec.switches().into_iter().skip(n).collect()
}

/// Waits for `act`'s switch number `n` (0-based), and returns it.
async fn nth_switch(act: &Actuator, n: usize, what: &str) -> Switched {
    eventually(what, || async { act.rec.switches().len() > n }).await;
    act.rec.switches()[n].clone()
}

/// A switch, as a line's `from`, `to` and `cause`.
fn line(s: &Switched) -> (Option<String>, Option<String>, Cause) {
    (
        s.from.as_ref().map(ToString::to_string),
        s.to.as_ref().map(ToString::to_string),
        s.cause,
    )
}

fn sw(
    from: Option<&str>,
    to: Option<&str>,
    cause: Cause,
) -> (Option<String>, Option<String>, Cause) {
    (from.map(str::to_owned), to.map(str::to_owned), cause)
}

/// A `deadline` switch's own instants: never before the deadline, at
/// most the detection bound plus the runner's margin. Returns `at − last`.
fn check_deadline(s: &Switched, what: &str) -> Duration {
    assert_eq!(s.cause, Cause::Deadline, "{what}: {s:?}");
    let gap =
        s.at.duration_since(s.last.expect("a provider's last arrival"));
    assert!(
        gap > DEADLINE,
        "{what}: switched {gap:?} after the last arrival"
    );
    assert!(
        gap <= BOUND + MARGIN,
        "{what}: switched {gap:?} after the last arrival, past the bound and the margin"
    );
    gap
}

// --- §1 -------------------------------------------------------------------------------

/// §1 The dead-man at start, and each higher priority taking over (§2.9–§2.11).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_the_dead_man_at_start_and_each_higher_priority_taking_over() {
    let (_r1, ep) = router(None).await;
    let s = Tool::new(&ep).await;
    let mut c = BTreeMap::new();
    c.insert("autopilot", Commander::start(&ep, "autopilot").await);
    tokio::time::sleep(LEAD + PERIOD * 2).await;
    // 1. A_l and A_r start.
    let mut acts = Vec::new();
    for n in ["rudder-l", "rudder-r"] {
        let started = Instant::now();
        let act = actuator(&ep, Build::new(n, &ep)).await.unwrap();
        let first = nth_switch(&act, 0, "the start line").await;
        assert_eq!(line(&first), sw(None, None, Cause::Start), "{n}: first");
        let second = nth_switch(&act, 1, "to autopilot").await;
        assert_eq!(
            line(&second),
            sw(None, Some("lab/autopilot"), Cause::Sample)
        );
        assert!(
            second.at.duration_since(started) <= Duration::from_secs(1),
            "{n}: within 1 s of its start"
        );
        acts.push(act);
    }
    for act in &acts {
        eventually("S sees it follow autopilot", || async {
            s.angle(act)
                .iter()
                .any(|(_, p)| p.starts_with("autopilot "))
        })
        .await;
    }
    // 2. 2 s later, C_t starts.
    tokio::time::sleep(Duration::from_secs(2)).await;
    c.insert("teleop", Commander::start(&ep, "teleop").await);
    for act in &acts {
        let s2 = nth_switch(act, 2, "to teleop").await;
        assert_eq!(
            line(&s2),
            sw(Some("lab/autopilot"), Some("lab/teleop"), Cause::Sample)
        );
    }
    // 3. 2 s later, C_s starts.
    tokio::time::sleep(Duration::from_secs(2)).await;
    for act in &acts {
        // No hysteresis: teleop's first arrival was delivered.
        assert_eq!(
            act.arb().counts(&a("lab/teleop")).not_active,
            0,
            "{}: teleop's every arrival delivered while it was highest",
            act.name
        );
    }
    c.insert("safety", Commander::start(&ep, "safety").await);
    for act in &acts {
        let s3 = nth_switch(act, 3, "to safety").await;
        assert_eq!(
            line(&s3),
            sw(Some("lab/teleop"), Some("lab/safety"), Cause::Sample)
        );
    }
    // 4. 2 s later, the counts.
    tokio::time::sleep(Duration::from_secs(2)).await;
    for act in &acts {
        let d = act.rec.delivered();
        let first_teleop = d
            .iter()
            .position(|(_, src, _)| src.to_string() == "lab/teleop")
            .unwrap();
        assert!(
            d[first_teleop..]
                .iter()
                .all(|(_, src, _)| src.to_string() != "lab/autopilot"),
            "{}: no autopilot payload after teleop's first",
            act.name
        );
        let first_safety = d
            .iter()
            .position(|(_, src, _)| src.to_string() == "lab/safety")
            .unwrap();
        assert!(
            d[first_safety..]
                .iter()
                .all(|(_, src, _)| src.to_string() == "lab/safety"),
            "{}: only safety after its first",
            act.name
        );
        let seen = s.angle(act);
        let first_seen_teleop = seen
            .iter()
            .find(|(_, p)| p.starts_with("teleop "))
            .map(|(_, p)| p.clone());
        assert_eq!(
            first_seen_teleop.as_deref(),
            Some(d[first_teleop].2.as_str()),
            "S's first teleop payload on {}'s angle is the first it delivered",
            act.name
        );
        for (p, prefix) in [
            ("lab/safety", "safety "),
            ("lab/teleop", "teleop "),
            ("lab/autopilot", "autopilot "),
        ] {
            let counts = act.arb().counts(&a(p));
            let on_angle = seen.iter().filter(|(_, x)| x.starts_with(prefix)).count() as u64;
            assert!(
                counts.delivered.abs_diff(on_angle) <= 1,
                "{}: {p} delivered {}, S saw {on_angle}",
                act.name,
                counts.delivered
            );
        }
        assert!(act.arb().counts(&a("lab/teleop")).not_active > 0);
        assert!(act.arb().counts(&a("lab/autopilot")).not_active > 0);
        let d = act.arb().discards();
        assert_eq!((d.wild, d.unresolved, d.unbound), (0, 0, 0));
        assert_eq!(act.rec.switches().len(), 4, "{:?}", act.rec.lines());
    }
    drop(c);
    for act in acts {
        act.stop().await;
    }
}

// --- §2 -------------------------------------------------------------------------------

/// §2 A commander silent past its deadline (§2.3, §2.6, §2.9; `timing.v1` §2.5).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_a_commander_silent_past_its_deadline() {
    let w = World::following_safety(false).await;
    let base: Vec<usize> = w.acts.iter().map(|a| a.rec.switches().len()).collect();
    let mut gaps = Vec::new();
    let mut holds = Vec::new();
    for round in 0..5 {
        // 1. C_s holds for 2 s, then resumes.
        let held = Instant::now();
        w.c("safety").hold();
        tokio::time::sleep(Duration::from_secs(2)).await;
        w.c("safety").resume();
        let resumed = Instant::now();
        holds.push((held, resumed));
        for (act, b) in w.acts.iter().zip(&base) {
            let to_teleop = nth_switch(act, b + 2 * round, "to teleop").await;
            assert_eq!(
                line(&to_teleop),
                sw(Some("lab/safety"), Some("lab/teleop"), Cause::Deadline)
            );
            gaps.push(check_deadline(&to_teleop, "§2"));
            let back = nth_switch(act, b + 2 * round + 1, "back to safety").await;
            assert_eq!(
                line(&back),
                sw(Some("lab/teleop"), Some("lab/safety"), Cause::Sample),
                "a miss needs no re-admission"
            );
            assert!(
                back.at >= resumed - PERIOD,
                "back at safety's arrival after it resumed"
            );
        }
        // 2. 3 s apart.
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    for (act, b) in w.acts.iter().zip(&base) {
        let lines = after(act, *b);
        assert_eq!(lines.len(), 10, "{:?}", act.rec.lines());
        assert!(lines.iter().all(|s| s.to.is_some()), "none to the dead-man");
        // S sees no teleop payload outside a hold.
        for (at, p) in w.s.angle(act) {
            if p.starts_with("teleop ") && at > holds[0].0 {
                assert!(
                    holds
                        .iter()
                        .any(|(h, r)| at >= *h && at <= *r + Duration::from_millis(200)),
                    "{}: teleop {p} outside a hold",
                    act.name
                );
            }
        }
    }
    eprintln!("§2: at − last of each deadline switch: {gaps:?}");
    w.stop().await;
}

// --- §3 -------------------------------------------------------------------------------

/// §3 A commander killed (§2.7).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_a_commander_killed() {
    let mut w = World::following_safety(false).await;
    let base: Vec<usize> = w.acts.iter().map(|a| a.rec.switches().len()).collect();
    let reads: Vec<u64> = w.acts.iter().map(|a| a.arb().reads()).collect();
    // 1. The kill, at K.
    let k = Instant::now();
    w.c("safety").kill();
    for (act, b) in w.acts.iter().zip(&base) {
        let s = nth_switch(act, *b, "to teleop").await;
        assert_eq!(
            line(&s),
            sw(Some("lab/safety"), Some("lab/teleop"), Cause::Withdrawn)
        );
        let gap = s.at.duration_since(s.last.unwrap());
        assert!(gap < DEADLINE, "before safety's deadline: {gap:?}");
        eprintln!(
            "§3: {} switched {:?} after the kill, {gap:?} after safety's last arrival",
            act.name,
            s.at.duration_since(k)
        );
    }
    for (act, r) in w.acts.iter().zip(&reads) {
        eventually("the read again", || async { act.arb().reads() > *r }).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    for ((act, r), b) in w.acts.iter().zip(&reads).zip(&base) {
        assert_eq!(act.arb().reads(), r + 1, "{}: one read again", act.name);
        assert_eq!(
            act.active().as_deref(),
            Some("lab/teleop"),
            "stays on teleop"
        );
        assert_eq!(after(act, *b).len(), 1, "{:?}", act.rec.lines());
    }
    // 2. 2 s later, C_s starts again, a new instance.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let killed = w.c.remove("safety").unwrap();
    let ep = w.ep.clone();
    let token_seen = Arc::new(Mutex::new(None::<Instant>));
    let t = Arc::clone(&token_seen);
    let _watch = zenkey::presence::declare_subscriber(&w.s.session, "zk2/lab/safety/@zk/alive/**")
        .callback(move |s: Sample| {
            if s.kind() == SampleKind::Put {
                t.lock().unwrap().get_or_insert(Instant::now());
            }
        })
        .await
        .unwrap();
    w.c.insert("safety", Commander::start(&ep, "safety").await);
    eventually("S sees the new instance's token", || async {
        token_seen.lock().unwrap().is_some()
    })
    .await;
    for (act, b) in w.acts.iter().zip(&base) {
        let s = nth_switch(act, b + 1, "back to safety").await;
        assert_eq!(
            line(&s),
            sw(Some("lab/teleop"), Some("lab/safety"), Cause::Sample)
        );
        // F-269: the commander puts 500 ms after its tokens, so the switch
        // is at its first arrival: none of the new instance's was counted
        // not delivered.
        let first = act
            .rec
            .delivered()
            .into_iter()
            .find(|(at, src, _)| src.to_string() == "lab/safety" && *at > k)
            .unwrap();
        assert_eq!(first.2, "safety 0", "{}: at the first arrival", act.name);
    }
    drop(killed);
    w.stop().await;
}

// --- §4 -------------------------------------------------------------------------------

/// X: a raw session playing safety, its tokens declared by hand.
struct RawSafety {
    session: zenoh::Session,
    instance: Option<zenoh::liveliness::LivelinessToken>,
    alive: Option<zenoh::liveliness::LivelinessToken>,
    on: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl RawSafety {
    async fn new(ep: &str, id: &str) -> Self {
        let session = session(ep).await;
        let mut x = Self {
            session: session.clone(),
            instance: None,
            alive: None,
            on: Arc::new(AtomicBool::new(false)),
            task: tokio::spawn(async {}),
        };
        x.declare(id).await;
        let (on, s) = (Arc::clone(&x.on), session);
        x.task = tokio::spawn(async move {
            let mut tick = tokio::time::interval(PERIOD);
            let mut n = 0u64;
            loop {
                tick.tick().await;
                if on.load(Ordering::SeqCst) {
                    let _ = s.put(cmd_key("safety"), format!("safety {n}")).await;
                    n += 1;
                }
            }
        });
        x
    }

    async fn declare(&mut self, id: &str) {
        let l = self.session.liveliness();
        self.instance = Some(
            l.declare_token(format!("zk2/lab/safety/@zk/instance/{id}"))
                .await
                .unwrap(),
        );
        self.alive = Some(
            l.declare_token(format!("zk2/lab/safety/@zk/alive/steer.v1/{id}/{FP16}"))
                .await
                .unwrap(),
        );
    }

    /// Core §8.2's stop: the interface token, then the instance token.
    async fn undeclare(&mut self) {
        if let Some(t) = self.alive.take() {
            t.undeclare().await.unwrap();
        }
        if let Some(t) = self.instance.take() {
            t.undeclare().await.unwrap();
        }
    }
}

impl Drop for RawSafety {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// §4 A clean stop that publishes after its withdrawal (§2.7; core §8.2).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_a_clean_stop_that_publishes_after_its_withdrawal() {
    let (_r1, ep) = router(None).await;
    let s = Tool::new(&ep).await;
    let mut acts = Vec::new();
    for n in ["rudder-l", "rudder-r"] {
        acts.push(actuator(&ep, Build::new(n, &ep)).await.unwrap());
    }
    let _t = Commander::start(&ep, "teleop").await;
    let mut x = RawSafety::new(&ep, "00000000000000a1").await;
    tokio::time::sleep(LEAD).await;
    x.on.store(true, Ordering::SeqCst);
    for act in &acts {
        eventually("follows safety", || async {
            act.active().as_deref() == Some("lab/safety")
        })
        .await;
    }
    let base: Vec<usize> = acts.iter().map(|a| a.rec.switches().len()).collect();
    let tail_from: Vec<u64> = acts
        .iter()
        .map(|a| a.arb().counts(&a_("lab/safety")).not_active)
        .collect();
    let reads: Vec<u64> = acts.iter().map(|a| a.arb().reads()).collect();
    // 1. Its tokens undeclared, then 20 samples more over 1 s.
    x.undeclare().await;
    let withdrawn = Instant::now();
    for (act, b) in acts.iter().zip(&base) {
        let s = nth_switch(act, *b, "to teleop").await;
        assert_eq!(
            line(&s),
            sw(Some("lab/safety"), Some("lab/teleop"), Cause::Withdrawn)
        );
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    x.on.store(false, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(300)).await;
    for (((act, b), t), r) in acts.iter().zip(&base).zip(&tail_from).zip(&reads) {
        assert_eq!(
            after(act, *b).len(),
            1,
            "no line back: {:?}",
            act.rec.lines()
        );
        let tail = act.arb().counts(&a_("lab/safety")).not_active - t;
        assert!(
            tail >= 15,
            "{}: the tail's samples counted not_active, {tail}",
            act.name
        );
        let seen = s.angle_since(act, withdrawn + Duration::from_millis(100));
        assert!(
            seen.iter().all(|p| p.starts_with("teleop ")),
            "{}: only teleop on angle: {seen:?}",
            act.name
        );
        assert_eq!(act.arb().reads(), r + 1, "{}: one read again", act.name);
        assert!(act.arb().withdrawn(&a_("lab/safety")), "re-admits nothing");
    }
    // 2. 2 s later, new tokens; 500 ms later, putting again.
    tokio::time::sleep(Duration::from_secs(2)).await;
    x.declare("00000000000000a2").await;
    tokio::time::sleep(LEAD).await;
    for (act, b) in acts.iter().zip(&base) {
        assert_eq!(
            after(act, *b).len(),
            1,
            "a re-admission alone switches nothing"
        );
        assert!(!act.arb().withdrawn(&a_("lab/safety")), "re-admitted");
    }
    x.on.store(true, Ordering::SeqCst);
    for (act, b) in acts.iter().zip(&base) {
        let s = nth_switch(act, b + 1, "back to safety").await;
        assert_eq!(
            line(&s),
            sw(Some("lab/teleop"), Some("lab/safety"), Cause::Sample)
        );
    }
    for act in acts {
        act.stop().await;
    }
}

fn a_(s: &str) -> Addr {
    a(s)
}

// --- §5 -------------------------------------------------------------------------------

/// §5 A re-mint with no drop (§2.6, §2.7; core §8.1).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_a_re_mint_with_no_drop() {
    let mut w = World::following_safety(false).await;
    let base: Vec<usize> = w.acts.iter().map(|a| a.rec.switches().len()).collect();
    let reads: Vec<u64> = w.acts.iter().map(|a| a.arb().reads()).collect();
    let since = Instant::now();
    for _ in 0..10 {
        w.c.get_mut("safety")
            .unwrap()
            .svc()
            .new_epoch()
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    for ((act, b), r) in w.acts.iter().zip(&base).zip(&reads) {
        assert!(after(act, *b).is_empty(), "no line: {:?}", act.rec.lines());
        let seen = w.s.angle_since(act, since);
        assert!(seen.len() > 100, "{}", seen.len());
        assert!(seen.iter().all(|p| p.starts_with("safety ")), "{seen:?}");
        assert_eq!(act.arb().reads(), *r, "{}: no read again", act.name);
    }
    w.stop().await;
}

// --- §6 -------------------------------------------------------------------------------

/// §6 All silent: the dead-man (§2.11).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s6_all_silent_the_dead_man() {
    let w = World::following_safety(false).await;
    let base: Vec<usize> = w.acts.iter().map(|a| a.rec.switches().len()).collect();
    // 1.–3.
    w.c("safety").hold();
    tokio::time::sleep(Duration::from_secs(1)).await;
    w.c("teleop").hold();
    tokio::time::sleep(Duration::from_secs(1)).await;
    w.c("autopilot").hold();
    let mut dead = Vec::new();
    for (act, b) in w.acts.iter().zip(&base) {
        let s1 = nth_switch(act, *b, "to teleop").await;
        assert_eq!(
            line(&s1),
            sw(Some("lab/safety"), Some("lab/teleop"), Cause::Deadline)
        );
        let s2 = nth_switch(act, b + 1, "to autopilot").await;
        assert_eq!(
            line(&s2),
            sw(Some("lab/teleop"), Some("lab/autopilot"), Cause::Deadline)
        );
        let s3 = nth_switch(act, b + 2, "to the dead-man").await;
        assert_eq!(line(&s3), sw(Some("lab/autopilot"), None, Cause::Deadline));
        check_deadline(&s3, "§6 the dead-man");
        assert!(act.arb().dead_man());
        dead.push(s3.at);
    }
    // 4. 2 s later, C_a resumes.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let resumed = Instant::now();
    for (act, at) in w.acts.iter().zip(&dead) {
        let quiet = w.s.angle_since(act, *at + Duration::from_millis(100));
        assert!(quiet.is_empty(), "{}: no angle put: {quiet:?}", act.name);
        assert!(resumed > *at);
    }
    w.c("autopilot").resume();
    for (act, b) in w.acts.iter().zip(&base) {
        let s4 = nth_switch(act, b + 3, "to autopilot").await;
        assert_eq!(line(&s4), sw(None, Some("lab/autopilot"), Cause::Sample));
    }
    w.stop().await;
}

// --- §7 -------------------------------------------------------------------------------

/// The control (text 0.2, F-268): a watch session of the test's own,
/// behind the actuators' watch link, recording at each cut whether its
/// subscriber heard safety's interface token withdrawn before the
/// session's transport event told the close.
struct Control {
    told: Arc<Mutex<Vec<(Instant, bool)>>>,
    events: Arc<Mutex<Vec<(Instant, bool)>>>,
    _sub: zenoh::pubsub::Subscriber<()>,
    _listener: zenoh::session::TransportEventsListener<()>,
    _session: zenoh::Session,
}

impl Control {
    async fn new(ep: &str) -> Self {
        let session = session(ep).await;
        let told: Arc<Mutex<Vec<(Instant, bool)>>> = Arc::default();
        let events: Arc<Mutex<Vec<(Instant, bool)>>> = Arc::default();
        let (t, e) = (Arc::clone(&told), Arc::clone(&events));
        let sub = zenkey::presence::declare_subscriber(&session, "zk2/lab/safety/@zk/alive/**")
            .callback(move |s: Sample| {
                t.lock()
                    .unwrap()
                    .push((Instant::now(), s.kind() == SampleKind::Put));
            })
            .await
            .unwrap();
        let listener = session
            .info()
            .transport_events_listener()
            .callback(move |ev| {
                e.lock()
                    .unwrap()
                    .push((Instant::now(), ev.kind() == SampleKind::Put));
            })
            .await
            .unwrap();
        Self {
            told,
            events,
            _sub: sub,
            _listener: listener,
            _session: session,
        }
    }

    /// Per close since `since`: whether a withdrawal was heard before it.
    fn ahead(&self, since: Instant) -> Vec<bool> {
        let told = self.told.lock().unwrap().clone();
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(at, opened)| !opened && *at >= since)
            .map(|(closed, _)| {
                told.iter().any(|(at, put)| {
                    !put && at <= closed && *at + Duration::from_millis(50) >= *closed
                })
            })
            .collect()
    }
}

/// §7 The watch session's own transport moves (§2.8; core §8.1).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_the_watch_sessions_own_transport_moves() {
    let mut w = World::following_safety(true).await;
    let link = w.watch_link.take().unwrap();
    let control = Control::new(link.endpoint()).await;
    // The control's subscriber has no history: one round trip of its own,
    // and the first cut tells it nothing (core §8.1, 0.41).
    let base: Vec<usize> = w.acts.iter().map(|a| a.rec.switches().len()).collect();
    let since = Instant::now();
    // 1. Five cuts of 1 s, 3 s apart.
    for _ in 0..5 {
        let trips: Vec<u64> = w.acts.iter().map(|a| a.arb().round_trips()).collect();
        link.cut();
        tokio::time::sleep(Duration::from_secs(1)).await;
        link.heal();
        for (act, t) in w.acts.iter().zip(&trips) {
            eventually("the round trip after the heal", || async {
                act.arb().round_trips() > *t && !act.arb().moving()
            })
            .await;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    for (act, b) in w.acts.iter().zip(&base) {
        assert!(after(act, *b).is_empty(), "no line: {:?}", act.rec.lines());
        let seen = w.s.angle_since(act, since);
        assert!(seen.iter().all(|p| p.starts_with("safety ")), "{seen:?}");
        assert!(!act.arb().withdrawn(&a("lab/safety")));
    }
    // The control: zenoh told the close's withdrawals before the close.
    let ahead = control.ahead(since);
    eprintln!("§7 control: a withdrawal heard before each close's event: {ahead:?}");
    assert!(ahead.len() >= 5, "{ahead:?}");
    assert!(
        ahead.iter().skip(1).any(|x| *x),
        "a close's withdrawal heard before its transport event at a cut after the first: \
         {ahead:?}"
    );
    // 2. A cut, the kill under it, the heal 1 s after the kill.
    link.cut();
    tokio::time::sleep(Duration::from_millis(300)).await;
    w.c("safety").kill();
    tokio::time::sleep(Duration::from_secs(1)).await;
    link.heal();
    for (act, b) in w.acts.iter().zip(&base) {
        let s = nth_switch(act, *b, "to teleop").await;
        assert_eq!(
            line(&s),
            sw(Some("lab/safety"), Some("lab/teleop"), Cause::Deadline)
        );
        check_deadline(&s, "§7 step 2");
        eventually("the round trip after the heal", || async {
            !act.arb().moving()
        })
        .await;
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    for (act, b) in w.acts.iter().zip(&base) {
        assert_eq!(
            after(act, *b).len(),
            1,
            "nothing more at the heal: {:?}",
            act.rec.lines()
        );
    }
    w.stop().await;
}

// --- §8 -------------------------------------------------------------------------------

/// §8 An invalid declaration, refused at start (§2.2; core §8.2).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s8_an_invalid_declaration_refused_at_start() {
    let (_r1, ep) = router(None).await;
    let s = Tool::new(&ep).await;
    let heard: Arc<Mutex<Vec<String>>> = Arc::default();
    let h = Arc::clone(&heard);
    let watch = client(&ep).await;
    let _w = zenkey::presence::declare_subscriber(&watch, "zk2/lab/rudder-l/@zk/**")
        .callback(move |x: Sample| h.lock().unwrap().push(x.key_expr().as_str().to_owned()))
        .await
        .unwrap();
    let data: Arc<Mutex<Vec<String>>> = Arc::default();
    let d = Arc::clone(&data);
    let _d = s
        .session
        .declare_subscriber("zk2/lab/rudder-l/**")
        .callback(move |x: Sample| d.lock().unwrap().push(x.key_expr().as_str().to_owned()))
        .await
        .unwrap();
    let _c: Vec<Commander> = vec![
        Commander::start(&ep, "safety").await,
        Commander::start(&ep, "teleop").await,
        Commander::start(&ep, "autopilot").await,
    ];
    // The control: configured plainly, it prints its start line and serves.
    let control = actuator(&ep, Build::new("rudder-l", &ep)).await.unwrap();
    let first = nth_switch(&control, 0, "the control's start line").await;
    assert_eq!(first.cause, Cause::Start);
    eventually("the control serves", || async {
        heard
            .lock()
            .unwrap()
            .iter()
            .any(|k| k.contains("/@zk/instance/"))
    })
    .await;
    control.stop().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    heard.lock().unwrap().clear();
    data.lock().unwrap().clear();
    let policy = "\"arbitration.policy\" = \"priority\"";
    let cases = [
        (
            copy("rudder.v1", policy, "\"arbitration.policy\" = \"freshest\""),
            contract("steer.v1"),
            "policy_unsupported",
        ),
        (
            copy("rudder.v1", policy, "\"arbitration.policy\" = \"vote\""),
            contract("steer.v1"),
            "value",
        ),
        (
            copy(
                "rudder.v1",
                "cardinality = \"many\"",
                "cardinality = \"one\"",
            ),
            contract("steer.v1"),
            "cardinality",
        ),
        (
            contract("rudder.v1"),
            copy("steer.v1", ", \"timing.deadline_ms\" = 300", ""),
            "no_deadline",
        ),
        (
            contract("rudder.v1"),
            copy(
                "steer.v1",
                "\"timing.deadline_ms\" = 300",
                "\"timing.deadline_ms\" = 20",
            ),
            "no_deadline",
        ),
    ];
    for (rudder_c, steer_c, why) in cases {
        let mut o = Build::new("rudder-l", &ep);
        o.rudder = rudder_c;
        o.steer = steer_c;
        let e = actuator(&ep, o).await.err().expect("refused");
        let text = e.to_string();
        assert!(matches!(e, Error::NotExposed(_)), "{text}");
        assert!(text.starts_with("not started: "), "{text}");
        assert!(text.contains(&format!("invalid: {why}")), "{why}: {text}");
    }
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(
        heard.lock().unwrap().is_empty(),
        "no token: {:?}",
        heard.lock().unwrap()
    );
    assert!(
        data.lock().unwrap().is_empty(),
        "nothing put: {:?}",
        data.lock().unwrap()
    );
}

// --- §9 -------------------------------------------------------------------------------

/// §9 A wildcard binding's order (§2.4; core R1, R3).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s9_a_wildcard_bindings_order() {
    let (_r1, ep) = router(None).await;
    let tool = client(&ep).await;
    let mut c = BTreeMap::new();
    c.insert("teleop", Commander::start(&ep, "teleop").await);
    c.insert("autopilot", Commander::start(&ep, "autopilot").await);
    tokio::time::sleep(LEAD + PERIOD * 2).await;
    // 1. A_l starts, bound to ["lab/safety", "lab/*"].
    let mut o = Build::new("rudder-l", &ep);
    o.binding = &["lab/safety", "lab/*"];
    let started = Instant::now();
    let act = actuator(&ep, o).await.unwrap();
    let id = act.svc.as_ref().unwrap().instance().clone();
    let found = zenkey::presence::descriptor(&tool, &a("lab/rudder-l"), &id, T)
        .await
        .unwrap()
        .into_descriptor()
        .unwrap();
    let cmd = found.requires.iter().find(|r| r.role == "cmd").unwrap();
    assert_eq!(cmd.bindings, ["lab/safety", "lab/*"], "in that order");
    eventually("follows autopilot", || async {
        act.active().as_deref() == Some("lab/autopilot")
    })
    .await;
    // §2.4: an address `lab/*` covers is one provider, each sample its
    // arbiter's once, though two subscribers hear it.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let payloads: Vec<String> = act.rec.delivered().into_iter().map(|(_, _, p)| p).collect();
    let distinct: std::collections::BTreeSet<&String> = payloads.iter().collect();
    assert_eq!(
        distinct.len(),
        payloads.len(),
        "each delivered once: {payloads:?}"
    );
    let lines = act.rec.lines();
    let to_autopilot = act
        .rec
        .switches()
        .into_iter()
        .find(|s| {
            s.to.as_ref()
                .is_some_and(|t| t.to_string() == "lab/autopilot")
        })
        .unwrap();
    assert!(to_autopilot.at.duration_since(started) <= Duration::from_secs(1) + T);
    assert_eq!(to_autopilot.cause, Cause::Sample, "{lines:?}");
    assert!(
        act.arb()
            .providers()
            .iter()
            .all(|p| p.to_string() != "lab/rudder-l")
            || act.arb().counts(&a("lab/rudder-l")) == Default::default(),
        "the actuator itself is never a provider with arrivals"
    );
    // 2. 2 s later, C_a holds.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let n = act.rec.switches().len();
    c["autopilot"].hold();
    let s = nth_switch(&act, n, "to teleop").await;
    assert_eq!(
        line(&s),
        sw(Some("lab/autopilot"), Some("lab/teleop"), Cause::Deadline)
    );
    // 3. 1 s later, C_a resumes.
    tokio::time::sleep(Duration::from_secs(1)).await;
    c["autopilot"].resume();
    let s = nth_switch(&act, n + 1, "to autopilot").await;
    assert_eq!(
        line(&s),
        sw(Some("lab/teleop"), Some("lab/autopilot"), Cause::Sample)
    );
    // 4. 1 s later, C_s starts.
    tokio::time::sleep(Duration::from_secs(1)).await;
    c.insert("safety", Commander::start(&ep, "safety").await);
    let s = nth_switch(&act, n + 2, "to safety").await;
    assert_eq!(
        line(&s),
        sw(Some("lab/autopilot"), Some("lab/safety"), Cause::Sample),
        "the exact entry above the wildcard"
    );
    // `lab/safety` is matched by both entries, and counted at its first.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let delivered = act
        .rec
        .delivered()
        .into_iter()
        .filter(|(_, src, _)| src.to_string() == "lab/safety")
        .count() as u64;
    assert_eq!(act.arb().counts(&a("lab/safety")).delivered, delivered);
    let payloads: Vec<String> = act.rec.delivered().into_iter().map(|(_, _, p)| p).collect();
    let distinct: std::collections::BTreeSet<&String> = payloads.iter().collect();
    assert_eq!(distinct.len(), payloads.len(), "each delivered once");
    act.stop().await;
}

// --- §11 ------------------------------------------------------------------------------

/// §11 A governed commander's standby (§2.5's `not_incumbent`;
/// `redundancy.v1` §2.10; text 0.2, F-270).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s11_a_governed_commanders_standby() {
    let (_r1, ep) = router(None).await;
    let s = Tool::new(&ep).await;
    let governed = contract("steer-governed.v1");
    // The commanders first: C_t, then C_s1, which serves (the lower id),
    // then C_s2, which stands by.
    let _t = Commander::start_on(&ep, "teleop", governed.clone(), Some(0x0b)).await;
    let s1 = Commander::start_on(&ep, "safety", governed.clone(), Some(0x0a)).await;
    let s1_red = s1.svc.as_ref().unwrap().redundancy().unwrap().clone();
    tokio::time::timeout(SETTLE, s1_red.until(&steer(), Role::Serving))
        .await
        .expect("C_s1 serves");
    let s2 = Commander::start_on(&ep, "safety", governed.clone(), Some(0x0c)).await;
    let s2_red = s2.svc.as_ref().unwrap().redundancy().unwrap().clone();
    tokio::time::timeout(SETTLE, s2_red.until(&steer(), Role::Standby))
        .await
        .expect("C_s2 stands by");
    // Then the actuators, whose watch was told no token: each learns the
    // incumbents by its read at start, and their descriptors (§2.5).
    let mut acts = Vec::new();
    for n in ["rudder-l", "rudder-r"] {
        let mut o = Build::new(n, &ep);
        o.steer = governed.clone();
        o.binding = &["lab/safety", "lab/teleop"];
        let started = Instant::now();
        let act = actuator(&ep, o).await.unwrap();
        eventually("follows safety", || async {
            act.active().as_deref() == Some("lab/safety")
        })
        .await;
        assert!(
            started.elapsed() <= Duration::from_secs(1) + T,
            "{n}: following safety within 1 s of its start"
        );
        acts.push(act);
    }
    // 1. The standby puts raw samples on safety's key, its own stamps.
    let raw = s2.session.clone();
    let raw_on = Arc::new(AtomicBool::new(true));
    let on = Arc::clone(&raw_on);
    let raw_task = tokio::spawn(async move {
        let mut tick = tokio::time::interval(PERIOD);
        let mut n = 0u64;
        while on.load(Ordering::SeqCst) {
            tick.tick().await;
            let _ = raw.put(cmd_key("safety"), format!("standby {n}")).await;
            n += 1;
        }
    });
    let base: Vec<usize> = acts.iter().map(|a| a.rec.switches().len()).collect();
    let since = Instant::now();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let refused: Vec<u64> = acts
        .iter()
        .map(|a| a.arb().counts(&a_("lab/safety")).not_incumbent)
        .collect();
    for (act, b) in acts.iter().zip(&base) {
        assert!(
            after(act, *b).is_empty(),
            "no switch: {:?}",
            act.rec.lines()
        );
        let c = act.arb().counts(&a_("lab/safety"));
        assert!(c.not_incumbent >= 10, "{}: {c:?}", act.name);
        let seen = s.angle_since(act, since);
        assert!(
            seen.iter().all(|p| p.starts_with("safety ")),
            "never a standby payload: {seen:?}"
        );
        assert!(
            act.rec
                .delivered()
                .iter()
                .all(|(_, _, p)| !p.starts_with("standby ")),
            "never delivered"
        );
    }
    // 2. The incumbent holds; the standby goes on putting.
    let held = Instant::now();
    s1.hold();
    for (act, b) in acts.iter().zip(&base) {
        let sw_ = nth_switch(act, *b, "to teleop").await;
        assert_eq!(
            line(&sw_),
            sw(Some("lab/safety"), Some("lab/teleop"), Cause::Deadline),
            "the standby's samples keep nothing eligible"
        );
        check_deadline(&sw_, "§11 step 2");
    }
    // 3. 2 s after the hold, the incumbent resumes.
    tokio::time::sleep(held + Duration::from_secs(2) - Instant::now()).await;
    for (act, r) in acts.iter().zip(&refused) {
        assert!(
            act.arb().counts(&a_("lab/safety")).not_incumbent > *r,
            "still counted not_incumbent"
        );
        assert_eq!(act.active().as_deref(), Some("lab/teleop"));
    }
    s1.resume();
    for (act, b) in acts.iter().zip(&base) {
        let back = nth_switch(act, b + 1, "back to safety").await;
        assert_eq!(
            line(&back),
            sw(Some("lab/teleop"), Some("lab/safety"), Cause::Sample)
        );
    }
    raw_on.store(false, Ordering::SeqCst);
    let _ = raw_task.await;
    for act in acts {
        act.stop().await;
    }
}

// --- the declaration at start, without the scenarios ------------------------------------

/// §2.2: a role carrying a policy whose revision is not given at the
/// builder is still checked at start by checks 1 to 5; `arbitrate` is
/// refused for a role carrying none, and for a manifest's role.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_declaration_at_the_builder_and_at_start() {
    let (_r1, ep) = router(None).await;
    let s = client(&ep).await;
    let cfg = || ServiceConfig::new(a("lab/rudder-l")).bind("cmd", &BINDING);
    // Checks 1 to 5 with no revision given: `freshest` refused.
    let mut b = ServiceBuilder::new(&s, cfg());
    b.implement(zenkey::Implementation::new(copy(
        "rudder.v1",
        "\"priority\"",
        "\"lease\"",
    )))
    .unwrap();
    b.expose(&rudder(), "state/angle").unwrap();
    let e = b.start().await.err().expect("refused");
    assert!(e.to_string().contains("invalid: policy_unsupported"), "{e}");
    // A manifest's role.
    let mut b = ServiceBuilder::new(&s, cfg());
    b.require("cmd", steer(), false);
    let e = b
        .arbitrate("cmd", Arc::new(contract("steer.v1")))
        .err()
        .expect("refused");
    assert!(e.to_string().contains("manifest_role"), "{e}");
    // Another interface's revision.
    let mut b = ServiceBuilder::new(&s, cfg());
    b.implement(zenkey::Implementation::new(contract("rudder.v1")))
        .unwrap();
    let e = b
        .arbitrate("cmd", Arc::new(contract("pace.v1")))
        .err()
        .expect("refused");
    assert!(e.to_string().contains("requires steer.v1"), "{e}");
    // Consumer::arbitrate on a role carrying no policy: refused.
    let mut b = ServiceBuilder::new(&s, cfg());
    b.require("cmd", steer(), false);
    let svc = b.start().await.unwrap();
    let e = svc
        .consumer("cmd", Arc::new(contract("steer.v1")))
        .unwrap()
        .arbitrate(|_| {})
        .await
        .expect_err("refused");
    assert!(e.to_string().contains("not arbitrated"), "{e}");
    svc.close().await.unwrap();
}
