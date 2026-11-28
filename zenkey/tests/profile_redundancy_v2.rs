//! `spec/profiles/redundancy/scenarios-v2.md` §1–§11 and §13–§15
//! (`redundancy.v2`, #987's N2), one test per section, named after it. §12
//! is a tool's (#987's N3).
//!
//! The setup is `profile_redundancy.rs`'s, v1's, with the readings
//! `scenarios-v2.md` states: the instances implement `ledger-run.v1` and
//! `valve-run.v1` (`tests/contracts/`), `ledger.v1` and `valve.v1` at minor
//! 1 with `uses = ["redundancy.v2"]`, and none of their sessions stamps
//! (v2 §2.10). Each draws a 32-bit run and states it from its claim on; a
//! test chooses one where a section asks (`Minter::simulate_run`) and takes
//! a new one with `Service::new_run`. R1 and R2 run zenoh's defaults, so
//! each stamps a put that arrives unstamped (core §4.1): S records whose id
//! each delivery carries. Where a section of v1's reads a stamp's id or
//! `meta.zid`, this one reads a run.
//!
//! K is the runtime's follower (`Consumer::follow`, v2 §2.10), a tool's
//! consumer of `ledger-run.v1` bound to `lab/ledger`, a client of R1 with a
//! watch session of its own: it seeds by core S4, holds `balance` as its
//! incumbent's samples and seeds leave it, and tells each sample it
//! accepted or discarded with the rule's reason. K1 (§15) is a follower
//! built for `redundancy.v1` alone, here, over the model's fence with every
//! instance read as v1's: it follows by `meta.zid`.
//!
//! **A crash.** As v1's: an instance dropped without its close, its links
//! first made silent (a lost host, seen at the lease) or cut (seen at once).

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use common::{SETTLE, T, client, eventually, eventually_some, router};
use zenkey::call::BoxFuture;
use zenkey::follow::{FollowEvent, FollowStartup, Following};
use zenkey::model::grammar::{Addr, IfaceId, InstanceId, ZkKey, parse};
use zenkey::model::redundancy::{self as model, Discard};
use zenkey::model::template::Bindings;
use zenkey::model::wire::{Run, Stamp};
use zenkey::operation::OperationServer;
use zenkey::presence::{self, Found};
use zenkey::redundancy::{FileLock, Lock, Redundancy, RedundancyStartup, Role, Takeover, Why};
use zenkey::state::StateWriter;
use zenkey::writer::Writer;
use zenkey::{Error, Implementation, Service, ServiceBuilder, ServiceConfig};
use zenoh::query::{ConsolidationMode, QueryTarget};
use zenoh::sample::{Sample, SampleKind};

const LEDGER: &str = "lab/ledger";
const VALVE: &str = "lab/valve";
const RETRY: Duration = Duration::from_secs(2);
const GRACE: Duration = Duration::from_secs(2);
/// The short lease of a crash seen at the lease: zenoh's 10 s default
/// would only slow the suite.
const LEASE: Duration = Duration::from_secs(3);
/// Loopback's allowance under a loaded host.
const JITTER: Duration = Duration::from_millis(500);

fn ledger() -> IfaceId {
    "ledger.v1".parse().unwrap()
}

fn valve() -> IfaceId {
    "valve.v1".parse().unwrap()
}

fn beacon() -> IfaceId {
    "beacon.v2".parse().unwrap()
}

fn id(n: u64) -> InstanceId {
    InstanceId::from_u64(n)
}

/// A session of an instance's: a client of `ep`, its HLC off as any
/// owner's (v2 §2.10), on for a `redundancy.v1` candidate's (`stamping`,
/// §15's A), retrying fast after a cut; its transport's lease `lease` when
/// given, which its router reads to expire it (core Appendix B).
async fn session(ep: &str, lease: Option<Duration>, stamping: bool) -> zenoh::Session {
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
    if stamping {
        c.insert_json5("timestamping/enabled", "true").unwrap();
    }
    if let Some(l) = lease {
        c.insert_json5("transport/link/tx/lease", &l.as_millis().to_string())
            .unwrap();
    }
    common::panic_watchdog::install();
    zenoh::open(c).await.expect("an instance's session")
}

/// How an instance is configured.
#[derive(Clone)]
struct Opts {
    name: &'static str,
    id: u64,
    address: &'static str,
    imp: Implementation,
    /// Through which endpoints its three sessions connect: one each, or the
    /// first for all.
    lease: Option<Duration>,
    watch: bool,
    lock: Option<Arc<dyn Lock>>,
    refuse_without_lock: bool,
    tokenless: bool,
    /// `ledger.v1` itself, `redundancy.v1`'s, its sessions stamping (§15).
    v1: bool,
    /// Its first run, chosen (the scenarios' test hook, §14).
    first_run: Option<Run>,
    /// What its takeover puts on `balance`: its name unless given, nothing
    /// for `Some(None)` (§13 step 3).
    takeover: Option<Option<&'static str>>,
    /// Configured for a first hop (core §8.1; §11 step 4).
    first_hop: bool,
}

impl Opts {
    fn ledger(name: &'static str, id: u64) -> Self {
        Self {
            name,
            id,
            address: LEDGER,
            imp: common::imp("ledger-run.v1"),
            lease: None,
            watch: true,
            lock: None,
            refuse_without_lock: false,
            tokenless: false,
            v1: false,
            first_run: None,
            takeover: None,
            first_hop: false,
        }
    }

    /// `ledger.v1` at minor 0, `redundancy.v1`'s, its sessions stamping.
    fn v1(mut self) -> Self {
        self.imp = common::imp("ledger.v1");
        self.v1 = true;
        self
    }

    fn valve(name: &'static str, id: u64, lock: Arc<dyn Lock>) -> Self {
        Self {
            address: VALVE,
            imp: common::imp("valve-run.v1"),
            lock: Some(lock),
            ..Self::ledger(name, id)
        }
    }

    /// `beacon-governed-run.v2` at `lab/beacon`: `status` followed by
    /// heartbeats (§13's step over 39 takeovers), no operation, a takeover
    /// that puts nothing.
    fn beacon(name: &'static str, id: u64) -> Self {
        Self {
            address: BEACON,
            imp: common::imp("beacon-governed-run.v2"),
            takeover: Some(None),
            ..Self::ledger(name, id)
        }
    }

    fn lease(mut self, lease: Duration) -> Self {
        self.lease = Some(lease);
        self
    }
}

/// The application's takeover: `balance` (or `position`) put with its
/// name (§2.6 step 3).
struct PutName {
    state: Arc<StateWriter>,
    name: Option<&'static str>,
}

impl Takeover for PutName {
    fn taking_over<'a>(&'a self, _iface: &'a IfaceId) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if let Some(name) = self.name {
                self.state.put(name).await.expect("a takeover's put");
            }
        })
    }
}

/// One instance.
struct Candidate {
    name: &'static str,
    id: InstanceId,
    iface: IfaceId,
    svc: Service,
    red: Redundancy,
    calls: Arc<AtomicU64>,
    state: Arc<StateWriter>,
    stream: Option<Writer>,
    servers: Vec<OperationServer>,
    /// The claim's, the watch, the reader.
    sessions: Vec<zenoh::Session>,
}

/// Brings an instance up on sessions of `ep` (one endpoint for all three),
/// or returns why its start refused.
async fn candidate(ep: &str, o: Opts) -> Result<Candidate, Error> {
    prepare(ep, o).await?.start().await
}

/// An instance before its start: its three sessions open and its service
/// built, nothing alive declared. [`Prepared::start`] makes the claim
/// (§2.5 step 1), so that §3 can open both instances' sessions first and
/// release both starts together.
struct Prepared {
    name: &'static str,
    id: InstanceId,
    iface: IfaceId,
    builder: ServiceBuilder,
    red: Redundancy,
    calls: Arc<AtomicU64>,
    state: Arc<StateWriter>,
    stream: Option<Writer>,
    servers: Vec<OperationServer>,
    sessions: Vec<zenoh::Session>,
}

impl Prepared {
    /// The start (core §8.2), or why it refused.
    async fn start(self) -> Result<Candidate, Error> {
        let svc = self.builder.start().await?;
        Ok(Candidate {
            name: self.name,
            id: self.id,
            iface: self.iface,
            svc,
            red: self.red,
            calls: self.calls,
            state: self.state,
            stream: self.stream,
            servers: self.servers,
            sessions: self.sessions,
        })
    }
}

/// An instance's sessions of `ep` opened and its service built, up to its
/// start.
async fn prepare(ep: &str, o: Opts) -> Result<Prepared, Error> {
    let claim = session(ep, o.lease, o.v1).await;
    let watch = session(ep, o.lease, false).await;
    let reader = session(ep, o.lease, false).await;
    let iface = o.imp.iface().clone();
    let mut cfg = ServiceConfig::new(o.address.parse::<Addr>().unwrap());
    if o.tokenless {
        cfg = cfg.tokenless(iface.clone());
    }
    cfg.first_hop = o.first_hop;
    let mut b = ServiceBuilder::new(&claim, cfg);
    b.set_instance_id(id(o.id));
    if let Some(r) = o.first_run {
        b.minter().simulate_run(r)?;
    }
    b.implement(o.imp.clone())?;
    let none = Bindings::new();
    let (state_res, op_res) = if iface == ledger() {
        ("state/balance", Some("@op/append"))
    } else if iface == beacon() {
        ("state/status", None)
    } else {
        ("state/position", Some("@op/set"))
    };
    let state = Arc::new(b.declare_state_writer(&iface, state_res, &none).await?);
    let stream = if iface == ledger() {
        // `stream/entries`, or `@stream/entries` in §11 step 3's copy.
        let entries = o
            .imp
            .contract()
            .resources
            .iter()
            .find(|r| r.template.to_string() == "entries")
            .map_or_else(
                || "stream/entries".to_owned(),
                zenkey::implementation::resource_name,
            );
        Some(b.declare_writer(&iface, &entries, &none).await?)
    } else {
        None
    };
    let calls = Arc::new(AtomicU64::new(0));
    let (n, name) = (Arc::clone(&calls), o.name);
    let mut servers = Vec::new();
    if let Some(op_res) = op_res {
        servers.push(
            b.serve(&iface, op_res, Some(&none), move |call| {
                let n = Arc::clone(&n);
                async move {
                    n.fetch_add(1, Ordering::SeqCst);
                    call.reply(name)
                        .await
                        .map_err(|e| zenkey::OpError::internal(e.to_string()))
                }
            })
            .await?,
        );
    }
    let mut startup = RedundancyStartup::new(reader.clone())
        .with_retry(RETRY)
        .with_grace(GRACE)
        .with_takeover(Arc::new(PutName {
            state: Arc::clone(&state),
            name: o.takeover.unwrap_or(Some(o.name)),
        }));
    if o.watch {
        startup = startup.with_watch(watch.clone());
    }
    if let Some(l) = &o.lock {
        startup = startup.with_lock(Arc::clone(l));
    }
    startup.refuse_without_lock = o.refuse_without_lock;
    let red = b.redundancy(startup)?;
    Ok(Prepared {
        name: o.name,
        id: id(o.id),
        iface,
        builder: b,
        red,
        calls,
        state,
        stream,
        servers,
        sessions: vec![claim, watch, reader],
    })
}

impl Candidate {
    fn role(&self) -> Role {
        self.red.role(&self.iface)
    }

    fn why(&self) -> Option<Why> {
        self.red.standing(&self.iface).why
    }

    /// Its claim session's zid, which a `redundancy.v1` candidate's stamps
    /// carry (v1 §2.10), and its descriptor's `meta.zid`.
    fn zid(&self) -> String {
        self.sessions[0].zid().to_string()
    }

    /// The run it writes under now, as its descriptor states it (v2 §2.10).
    fn run(&self) -> Run {
        self.svc.minter().run()
    }

    async fn until(&self, role: Role) {
        tokio::time::timeout(SETTLE, self.red.until(&self.iface, role))
            .await
            .unwrap_or_else(|_| panic!("never: {} {role:?}", self.name));
    }

    fn reader(&self) -> &zenoh::Session {
        &self.sessions[2]
    }

    /// A clean close (§2.8): the service, then its sessions.
    async fn close(self) {
        self.svc.close().await.unwrap();
        drop(self.servers);
        for s in self.sessions {
            s.close().await.unwrap();
        }
    }

    /// A crash: dropped without its close, its sessions closed in the
    /// background, where its links may be stalled or cut.
    fn crash(self) {
        let Candidate {
            svc,
            red,
            servers,
            sessions,
            state,
            stream,
            ..
        } = self;
        drop((svc, red, servers, state, stream));
        tokio::spawn(async move {
            for s in sessions {
                let _ = tokio::time::timeout(Duration::from_secs(5), s.close()).await;
            }
        });
    }
}

// --- S, the tool session that records ----------------------------------------------

/// One delivery, as S records it.
#[derive(Debug, Clone)]
enum Rec {
    /// A token declared or withdrawn.
    Token { at: Instant, key: ZkKey, put: bool },
    /// What a GET of an instance's descriptor, made the moment S heard one
    /// of its tokens declared, answered of the interface: marked standby,
    /// unmarked, or nothing (F-202); when it was asked and when answered
    /// (text 0.4, F-223).
    AtToken {
        key: ZkKey,
        standby: Option<bool>,
        run: Option<Run>,
        asked: Instant,
        answered: Instant,
    },
    /// A descriptor put: the interface marked or not, and the run it
    /// states.
    Descriptor {
        at: Instant,
        instance: InstanceId,
        standby: Option<bool>,
        run: Option<Run>,
    },
    /// A put or delete of one of the interface's keys: its Zenoh
    /// timestamp, and its envelope's run and seq.
    Data {
        at: Instant,
        key: String,
        payload: String,
        stamp: Option<zenoh::time::Timestamp>,
        inband: Option<Stamp>,
    },
}

type Log = Arc<Mutex<Vec<Rec>>>;

/// A put or delete S received: its arrival, key, payload, Zenoh timestamp
/// and envelope stamp.
type Heard = (
    Instant,
    String,
    String,
    Option<zenoh::time::Timestamp>,
    Option<Stamp>,
);

/// The run a descriptor states (v2 §2.10).
fn run_of(found: &Found) -> Option<Run> {
    match found {
        Found::Descriptor(d, _) => d.stated_run(),
        _ => None,
    }
}

/// What a descriptor says of `iface`: its mark, if it lists it.
fn mark_of(found: &Found, iface: &IfaceId) -> Option<bool> {
    match found {
        Found::Descriptor(d, _) => {
            let want = iface.to_string();
            d.interfaces
                .iter()
                .find(|e| e.iface == want)
                .map(|e| e.standby)
        }
        _ => None,
    }
}

struct Spy {
    log: Log,
    _subs: Vec<zenoh::pubsub::Subscriber<()>>,
}

/// S on `ep`, recording `address` and `iface`.
async fn spy(ep: &str, address: &str, iface: &IfaceId) -> Spy {
    let s = client(ep).await;
    let log: Log = Arc::default();
    let rt = tokio::runtime::Handle::current();
    let addr: Addr = address.parse().unwrap();
    let mut subs = Vec::new();
    // The tokens, each declared one met by a GET of its descriptor.
    let (w, getter, iface_c, addr_c) = (Arc::clone(&log), s.clone(), iface.clone(), addr.clone());
    subs.push(
        presence::declare_subscriber(&s, format!("zk2/{address}/@zk/**"))
            .history(true)
            .callback(move |x: Sample| {
                let Ok(key) = parse(x.key_expr().as_str()) else {
                    return;
                };
                let put = x.kind() == SampleKind::Put;
                w.lock().unwrap().push(Rec::Token {
                    at: Instant::now(),
                    key: key.clone(),
                    put,
                });
                let instance = match &key {
                    ZkKey::Instance { instance, .. } | ZkKey::Alive { instance, .. } => {
                        instance.clone()
                    }
                    _ => return,
                };
                if !put {
                    return;
                }
                let (w, getter, iface, addr) = (
                    Arc::clone(&w),
                    getter.clone(),
                    iface_c.clone(),
                    addr_c.clone(),
                );
                rt.spawn(async move {
                    let asked = Instant::now();
                    let found = presence::descriptor(&getter, &addr, &instance, T)
                        .await
                        .unwrap_or(Found::Nothing);
                    w.lock().unwrap().push(Rec::AtToken {
                        key,
                        standby: mark_of(&found, &iface),
                        run: run_of(&found),
                        asked,
                        answered: Instant::now(),
                    });
                });
            })
            .await
            .unwrap(),
    );
    // The descriptors as put.
    let (w, iface_c) = (Arc::clone(&log), iface.clone());
    subs.push(
        s.declare_subscriber(format!("zk2/{address}/@zk/instance/*"))
            .callback(move |x: Sample| {
                let Ok(ZkKey::Instance { instance, .. }) = parse(x.key_expr().as_str()) else {
                    return;
                };
                let text = String::from_utf8_lossy(&x.payload().to_bytes()).into_owned();
                let (d, _) = zenkey::model::descriptor::check(&text, &[]);
                let run = d
                    .as_ref()
                    .and_then(zenkey::model::descriptor::Descriptor::stated_run);
                let standby = d.and_then(|d| {
                    d.interfaces
                        .iter()
                        .find(|e| e.iface == iface_c.to_string())
                        .map(|e| e.standby)
                });
                w.lock().unwrap().push(Rec::Descriptor {
                    at: Instant::now(),
                    instance,
                    standby,
                    run,
                });
            })
            .await
            .unwrap(),
    );
    // The interface's keys.
    let w = Arc::clone(&log);
    subs.push(
        s.declare_subscriber(format!("zk2/{address}/{iface}/**"))
            .callback(move |x: Sample| {
                w.lock().unwrap().push(Rec::Data {
                    at: Instant::now(),
                    key: x.key_expr().as_str().to_owned(),
                    payload: String::from_utf8_lossy(&common::record(&x)).into_owned(),
                    stamp: x.timestamp().copied(),
                    inband: common::inband(&x),
                });
            })
            .await
            .unwrap(),
    );
    Spy { log, _subs: subs }
}

impl Spy {
    fn snapshot(&self) -> Vec<Rec> {
        self.log.lock().unwrap().clone()
    }

    /// The holders of `iface`'s token, as S's deliveries leave them.
    fn holders(&self, iface: &IfaceId) -> BTreeSet<InstanceId> {
        let mut h = BTreeSet::new();
        for r in self.snapshot() {
            if let Rec::Token {
                key: ZkKey::Alive {
                    iface: i, instance, ..
                },
                put,
                ..
            } = r
                && i == *iface
            {
                if put {
                    h.insert(instance);
                } else {
                    h.remove(&instance);
                }
            }
        }
        h
    }

    /// When S heard `instance`'s token of `iface` declared or withdrawn,
    /// in order.
    fn alive_events(&self, iface: &IfaceId, instance: &InstanceId) -> Vec<(Instant, bool)> {
        self.snapshot()
            .into_iter()
            .filter_map(|r| match r {
                Rec::Token {
                    at,
                    key:
                        ZkKey::Alive {
                            iface: i,
                            instance: x,
                            ..
                        },
                    put,
                } if i == *iface && x == *instance => Some((at, put)),
                _ => None,
            })
            .collect()
    }

    /// When S first heard `instance`'s instance token declared.
    fn instance_at(&self, instance: &InstanceId) -> Option<Instant> {
        self.snapshot().into_iter().find_map(|r| match r {
            Rec::Token {
                at,
                key: ZkKey::Instance { instance: x, .. },
                put: true,
            } if x == *instance => Some(at),
            _ => None,
        })
    }

    /// Whether S heard `instance`'s instance token declared.
    fn has_instance(&self, instance: &InstanceId) -> bool {
        self.snapshot().iter().any(|r| {
            matches!(r, Rec::Token { key: ZkKey::Instance { instance: x, .. }, put: true, .. }
                if x == instance)
        })
    }

    /// What the GETs at `instance`'s tokens answered: at its instance
    /// token, and at its token of `iface`, each first one (F-202).
    fn at_tokens(&self, iface: &IfaceId, instance: &InstanceId) -> (Option<bool>, Option<bool>) {
        let (mut at_instance, mut at_alive) = (None, None);
        for r in self.snapshot() {
            if let Rec::AtToken { key, standby, .. } = r {
                match key {
                    ZkKey::Instance { instance: x, .. } if x == *instance => {
                        at_instance = at_instance.or(Some(standby));
                    }
                    ZkKey::Alive {
                        iface: i,
                        instance: x,
                        ..
                    } if i == *iface && x == *instance => {
                        at_alive = at_alive.or(Some(standby));
                    }
                    _ => {}
                }
            }
        }
        (at_instance.flatten(), at_alive.flatten())
    }

    /// Whether `instance` took over before the GET at its instance token
    /// was answered (text 0.4, F-223): its token of `iface` at S before
    /// that answer, or within a round trip of it, since the answer and the
    /// token ride two priorities (F-202). Then that GET may read it
    /// unmarked: the takeover's step 3 put the descriptor unmarked within
    /// the GET's round trip.
    fn token_beat_the_get(&self, iface: &IfaceId, instance: &InstanceId) -> bool {
        let log = self.snapshot();
        let alive = log.iter().find_map(|r| match r {
            Rec::Token {
                at,
                key:
                    ZkKey::Alive {
                        iface: i,
                        instance: x,
                        ..
                    },
                put: true,
            } if i == iface && x == instance => Some(*at),
            _ => None,
        });
        let get = log.iter().find_map(|r| match r {
            Rec::AtToken {
                key: ZkKey::Instance { instance: x, .. },
                asked,
                answered,
                ..
            } if x == instance => Some((*asked, *answered)),
            _ => None,
        });
        matches!((alive, get), (Some(a), Some((asked, answered)))
            if a <= answered + answered.saturating_duration_since(asked))
    }

    /// Every descriptor S received of `instance`: its mark of the interface.
    fn descriptors(&self, instance: &InstanceId) -> Vec<Option<bool>> {
        self.snapshot()
            .into_iter()
            .filter_map(|r| match r {
                Rec::Descriptor {
                    instance: x,
                    standby,
                    ..
                } if x == *instance => Some(standby),
                _ => None,
            })
            .collect()
    }

    /// What the GETs at `instance`'s tokens answered of its run: at its
    /// instance token, and at its token of `iface`, each first one.
    fn at_token_runs(&self, iface: &IfaceId, instance: &InstanceId) -> (Option<Run>, Option<Run>) {
        let (mut at_instance, mut at_alive) = (None, None);
        for r in self.snapshot() {
            if let Rec::AtToken { key, run, .. } = r {
                match key {
                    ZkKey::Instance { instance: x, .. } if x == *instance => {
                        at_instance = at_instance.or(Some(run));
                    }
                    ZkKey::Alive {
                        iface: i,
                        instance: x,
                        ..
                    } if i == *iface && x == *instance => {
                        at_alive = at_alive.or(Some(run));
                    }
                    _ => {}
                }
            }
        }
        (at_instance.flatten(), at_alive.flatten())
    }

    /// Every descriptor S received of `instance`: when, its mark of the
    /// interface, and its run.
    fn descriptor_puts(&self, instance: &InstanceId) -> Vec<(Instant, Option<bool>, Option<Run>)> {
        self.snapshot()
            .into_iter()
            .filter_map(|r| match r {
                Rec::Descriptor {
                    at,
                    instance: x,
                    standby,
                    run,
                } if x == *instance => Some((at, standby, run)),
                _ => None,
            })
            .collect()
    }

    /// The interface's puts and deletes S received: each one's arrival,
    /// key, payload, Zenoh timestamp and envelope stamp.
    fn data(&self) -> Vec<Heard> {
        self.snapshot()
            .into_iter()
            .filter_map(|r| match r {
                Rec::Data {
                    at,
                    key,
                    payload,
                    stamp,
                    inband,
                    ..
                } => Some((at, key, payload, stamp, inband)),
                _ => None,
            })
            .collect()
    }

    /// The interface's puts and deletes S received under `run`: arrival,
    /// key, payload.
    fn data_under(&self, run: Run) -> Vec<(Instant, String, String)> {
        self.data()
            .into_iter()
            .filter(|(.., inband)| inband.is_some_and(|s| s.run == run))
            .map(|(at, key, payload, ..)| (at, key, payload))
            .collect()
    }

    /// Waits until S's holders of `iface` are exactly `want`.
    async fn holders_are(&self, iface: &IfaceId, want: &[&InstanceId]) {
        let want: BTreeSet<InstanceId> = want.iter().map(|i| (*i).clone()).collect();
        eventually(&format!("S's holders are {want:?}"), || async {
            self.holders(iface) == want
        })
        .await;
    }
}

/// A call of `address`'s operation `op`: `BestMatching` (O1), or `All`
/// (O2), consolidation `None`. Each reply's payload, or `!` for an error
/// reply.
async fn call(
    s: &zenoh::Session,
    address: &str,
    iface: &IfaceId,
    op: &str,
    all: bool,
) -> Vec<String> {
    let key = format!("zk2/{address}/{iface}/@op/{op}");
    // The request form (core §5.1): CH `0x40`, then the record.
    let replies = s
        .get(key)
        .payload(vec![0x40, b'x'])
        .target(if all {
            QueryTarget::All
        } else {
            QueryTarget::BestMatching
        })
        .consolidation(ConsolidationMode::None)
        .timeout(T)
        .await
        .unwrap();
    let mut out = Vec::new();
    while let Ok(r) = replies.recv_async().await {
        out.push(match r.result() {
            Ok(s) => {
                let bytes = s.payload().to_bytes();
                let value = zenkey::model::wire::read_reply(&bytes).expect("a value form");
                String::from_utf8_lossy(value.record).into_owned()
            }
            Err(_) => "!".to_owned(),
        });
    }
    out
}

/// [`call`]'s replies by run: each value's as-of mark's (core §5.1), `None`
/// for an error reply. A caller tells which instance answered by it (v2
/// §2.10, "Callers").
async fn call_runs(
    s: &zenoh::Session,
    address: &str,
    iface: &IfaceId,
    op: &str,
    all: bool,
) -> Vec<Option<Run>> {
    let key = format!("zk2/{address}/{iface}/@op/{op}");
    let replies = s
        .get(key)
        .payload(vec![0x40, b'x'])
        .target(if all {
            QueryTarget::All
        } else {
            QueryTarget::BestMatching
        })
        .consolidation(ConsolidationMode::None)
        .timeout(T)
        .await
        .unwrap();
    let mut out = Vec::new();
    while let Ok(r) = replies.recv_async().await {
        out.push(r.result().ok().map(|s| {
            let bytes = s.payload().to_bytes();
            zenkey::model::wire::read_reply(&bytes)
                .expect("a value form")
                .stamp
                .run
        }));
    }
    out
}

/// G's read of an address (core §8.1): its holders of `iface`, and
/// whether the read is complete.
async fn g_read(
    g: &zenoh::Session,
    address: &str,
    iface: &IfaceId,
) -> (bool, BTreeSet<InstanceId>) {
    let r = presence::liveliness_read(g, &format!("zk2/{address}/@zk/**"), T)
        .await
        .unwrap();
    let holders = r
        .keys
        .iter()
        .filter_map(|k| match parse(k).ok()? {
            ZkKey::Alive {
                iface: i, instance, ..
            } if i == *iface => Some(instance),
            _ => None,
        })
        .collect();
    (r.complete, holders)
}

// --- §1 ------------------------------------------------------------------------------

/// §1, step 4's crash: `silent` makes A's link silent first, a lost host
/// seen at the lease; otherwise its links are cut, as its kernel closes
/// them, seen at once (§2.8; text 0.2, F-201).
async fn s1_run(silent: bool) {
    let (r1, ep) = router(None).await;
    let s = spy(&ep, LEDGER, &ledger()).await;
    let x = client(&ep).await;
    let link_a = common::link_to(&ep).await;
    let link_b = common::link_to(&ep).await;

    // 1. A starts, and serves within 3 s.
    let started = Instant::now();
    let a = candidate(link_a.endpoint(), Opts::ledger("A", 0xa).lease(LEASE))
        .await
        .unwrap();
    a.until(Role::Serving).await;
    assert!(
        started.elapsed() < Duration::from_secs(3) + JITTER,
        "A serves within 3 s"
    );
    s.holders_are(&ledger(), &[&a.id]).await;
    eventually("the GETs at A's tokens answered", || async {
        let (i, al) = s.at_tokens(&ledger(), &a.id);
        i.is_some() && al.is_some()
    })
    .await;
    let (at_instance, at_alive) = s.at_tokens(&ledger(), &a.id);
    // F-202: marked at its instance token, unless A took over before that
    // GET was answered, its interface token at S within a round trip of the
    // answer (text 0.4, F-223); unmarked at its interface token.
    assert!(
        at_instance == Some(true) || s.token_beat_the_get(&ledger(), &a.id),
        "A's descriptor marked at its instance token: {at_instance:?}"
    );
    assert_eq!(at_alive, Some(false), "A unmarked at its interface token");
    // v2's §1, to Expected 1: both GETs answer A's descriptor stating A's
    // run (v2 §2.10).
    assert_eq!(
        s.at_token_runs(&ledger(), &a.id),
        (Some(a.run()), Some(a.run())),
        "A's run at its instance token and at its interface token"
    );

    // 2. B starts 2 s later, and stands by.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let b = candidate(link_b.endpoint(), Opts::ledger("B", 0xb).lease(LEASE))
        .await
        .unwrap();
    eventually("B stands by on A", || async {
        b.role() == Role::Standby && b.why() == Some(Why::Incumbent(a.id.clone()))
    })
    .await;
    eventually("S's GET at B's instance token answered", || async {
        s.at_tokens(&ledger(), &b.id).0.is_some()
    })
    .await;
    assert_eq!(
        s.at_tokens(&ledger(), &b.id).0,
        Some(true),
        "B marked at its instance token"
    );

    // 3. 20 calls, each after the last reply: every one on A.
    for _ in 0..20 {
        assert_eq!(call(&x, LEDGER, &ledger(), "append", false).await, ["A"]);
    }
    assert_eq!(a.calls.load(Ordering::SeqCst), 20);
    assert_eq!(b.calls.load(Ordering::SeqCst), 0, "none on B (spike S6)");
    assert!(
        s.alive_events(&ledger(), &b.id).is_empty(),
        "no token of B's"
    );
    assert!(
        s.descriptors(&b.id).iter().all(|m| *m == Some(true)),
        "B's descriptor never loses the mark"
    );
    // v2's §1, to Expected 2: every descriptor of B's S received, marked,
    // states one run, B's, from its claim on.
    let b_run = b.run();
    let stood_by = s.descriptor_puts(&b.id);
    assert!(
        !stood_by.is_empty()
            && stood_by
                .iter()
                .all(|(_, m, r)| *m == Some(true) && *r == Some(b_run)),
        "B's marked descriptors state its run {b_run}: {stood_by:?}"
    );

    // 4. The crash, at K.
    let reads_at_k = b.red.reads(&ledger());
    let gets_at_k = presence::liveliness_gets(b.reader());
    let k = Instant::now();
    if silent {
        link_a.stall();
    } else {
        link_a.cut();
    }
    a.crash();
    let withdrawn = loop {
        let ev = s.alive_events(&ledger(), &id(0xa));
        if let Some((at, _)) = ev.iter().find(|(_, put)| !put) {
            break *at;
        }
        // No read of B's, and no poll, before the withdrawal (§2.7).
        assert_eq!(
            b.red.reads(&ledger()).made,
            reads_at_k.made,
            "a read before the withdrawal"
        );
        assert_eq!(b.red.reads(&ledger()).polls, 0, "B never polls");
        assert!(k.elapsed() < SETTLE, "never: A's tokens withdrawn");
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    let seen_after = withdrawn - k;
    if silent {
        assert!(
            seen_after >= LEASE / 2 && seen_after < LEASE + Duration::from_secs(2),
            "seen at the lease, give or take a keep-alive: {seen_after:?}"
        );
    } else {
        assert!(
            seen_after < Duration::from_secs(1),
            "seen at once: {seen_after:?}"
        );
    }
    b.until(Role::Serving).await;
    s.holders_are(&ledger(), &[&b.id]).await;
    let (at, _) = s.alive_events(&ledger(), &b.id)[0];
    assert!(
        at - withdrawn < Duration::from_secs(3) + JITTER,
        "B serves within 3 s of the withdrawal"
    );
    eventually("S's GET at B's interface token answered", || async {
        s.at_tokens(&ledger(), &b.id).1.is_some()
    })
    .await;
    assert_eq!(
        s.at_tokens(&ledger(), &b.id).1,
        Some(false),
        "B unmarked at its token"
    );
    let reads = b.red.reads(&ledger());
    assert!(reads.made > reads_at_k.made, "B read after the withdrawal");
    assert_eq!(reads.polls, 0);
    assert!(
        presence::liveliness_gets(b.reader()).reads > gets_at_k.reads,
        "its reads on its reader"
    );

    // v2's §1, to Expected 4: B's descriptor put unmarked at its takeover
    // states the run it stood by under, and its put of `balance` is under
    // that run at seq 0, with no Zenoh timestamp of B's: R1's id.
    eventually("B's unmarked descriptor at S", || async {
        s.descriptor_puts(&b.id)
            .iter()
            .any(|(_, m, _)| *m == Some(false))
    })
    .await;
    let unmarked: Vec<_> = s
        .descriptor_puts(&b.id)
        .into_iter()
        .filter(|(_, m, _)| *m == Some(false))
        .collect();
    assert!(
        unmarked.iter().all(|(.., r)| *r == Some(b_run)),
        "B serves under the run it stood by under: {unmarked:?}"
    );
    assert_eq!(b.run(), b_run);
    eventually("B's takeover put at S", || async {
        s.data_under(b_run).iter().any(|(_, _, p)| p == "B")
    })
    .await;
    let (.., stamp, inband) = s
        .data()
        .into_iter()
        .find(|(_, key, p, ..)| key.ends_with("/state/balance") && p == "B")
        .unwrap();
    assert_eq!(
        inband.map(|s| (s.run, s.seq.get())),
        Some((b_run, 0)),
        "B's first put, under its run at seq 0"
    );
    assert_eq!(
        stamp.map(|t| t.get_id().to_string()),
        Some(r1.zid().to_string()),
        "R1's stamp, never B's (v2 §2.10)"
    );

    // 5. The call executes on B.
    assert_eq!(call(&x, LEDGER, &ledger(), "append", false).await, ["B"]);
    assert_eq!(b.calls.load(Ordering::SeqCst), 1);
    b.close().await;
}

/// §1 Takeover after a crash, seen at the lease (§2.4–§2.8; core §6,
/// §8.1, §8.2): run on a silent link, then on one cut (text 0.2, F-201).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn s1_takeover_after_a_crash_seen_at_the_lease() {
    s1_run(true).await;
    s1_run(false).await;
}

// --- §2 ------------------------------------------------------------------------------

/// §2 Takeover after a clean stop (§2.6, §2.8).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn s2_takeover_after_a_clean_stop() {
    let (_r1, ep) = router(None).await;
    let s = spy(&ep, LEDGER, &ledger()).await;
    let a = candidate(&ep, Opts::ledger("A", 0xa)).await.unwrap();
    a.until(Role::Serving).await;
    let b = candidate(&ep, Opts::ledger("B", 0xb)).await.unwrap();
    eventually("B stands by on A", || async {
        b.why() == Some(Why::Incumbent(a.id.clone()))
    })
    .await;
    s.holders_are(&ledger(), &[&a.id]).await;
    let a_run = a.run();

    // 1. A closes cleanly, at Q.
    let q = Instant::now();
    a.close().await;
    eventually("A's token withdrawn", || async {
        s.alive_events(&ledger(), &id(0xa))
            .iter()
            .any(|(_, put)| !put)
    })
    .await;
    let (w, _) = *s
        .alive_events(&ledger(), &id(0xa))
        .iter()
        .find(|(_, put)| !put)
        .unwrap();
    assert!(
        w - q < Duration::from_secs(1),
        "at once, without a lease: {:?}",
        w - q
    );
    b.until(Role::Serving).await;
    s.holders_are(&ledger(), &[&b.id]).await;
    let (at, _) = s.alive_events(&ledger(), &b.id)[0];
    assert!(
        at - q < Duration::from_secs(2) + JITTER,
        "B serves within 2 s of Q: {:?}",
        at - q
    );
    eventually("S's GET at B's token answered", || async {
        s.at_tokens(&ledger(), &b.id).1.is_some()
    })
    .await;
    assert_eq!(s.at_tokens(&ledger(), &b.id).1, Some(false), "§2.6's order");
    // A put and deleted nothing of ledger.v1's keys while it closed: told
    // by its run, its puts carrying no stamp of its own (v2 §2.10).
    let late: Vec<_> = s
        .data_under(a_run)
        .into_iter()
        .filter(|(at, ..)| *at >= q)
        .collect();
    assert!(late.is_empty(), "nothing of A's from Q on: {late:?}");
    b.close().await;
}

// --- §3 ------------------------------------------------------------------------------

/// Two instances started together (§3; text 0.3, F-216): both opened
/// first, their six sessions and services, then both starts released from
/// one barrier, each on a thread of its own, so that nothing but the
/// claims themselves lies between the two instance tokens. Started with
/// `tokio::join!` over the whole bring-up, the two were as far apart as
/// their sessions' opens: A's always first, by a median 7 ms on a quiet
/// host and 56 ms with eight tests and two CPU hogs on two cores, where no
/// start of 200 was within 10 ms at S. Released together, 199 were (#905).
async fn start_together(ep: &str, a: Opts, b: Opts) -> (Candidate, Candidate) {
    let (a, b) = tokio::join!(prepare(ep, a), prepare(ep, b));
    let rt = tokio::runtime::Handle::current();
    let gate = Arc::new(std::sync::Barrier::new(2));
    let release = |p: Prepared| {
        let (rt, gate) = (rt.clone(), Arc::clone(&gate));
        tokio::task::spawn_blocking(move || {
            gate.wait();
            rt.block_on(p.start())
        })
    };
    // Both prepared before either is released: a thread waiting at the
    // barrier for a partner that never comes would hang the test.
    let (a, b) = (a.unwrap(), b.unwrap());
    let (a, b) = (release(a), release(b));
    let started = |r: Result<Result<Candidate, Error>, tokio::task::JoinError>| match r {
        Ok(c) => c.unwrap(),
        Err(e) => std::panic::resume_unwind(e.into_panic()),
    };
    (started(a.await), started(b.await))
}

/// The least, the median and the greatest of `spreads`.
fn spread_of(spreads: &[Duration]) -> String {
    let mut s = spreads.to_vec();
    s.sort();
    match (s.first(), s.last()) {
        (Some(lo), Some(hi)) => {
            let n = s.len();
            let median = if n.is_multiple_of(2) {
                (s[n / 2 - 1] + s[n / 2]) / 2
            } else {
                s[n / 2]
            };
            format!("{lo:?} to {hi:?}, median {median:?}")
        }
        _ => "none".to_owned(),
    }
}

/// §3 A simultaneous start (§2.5, §2.9): 20 runs, fresh ids, A's the
/// lower. One serves and the other stands by: A, unless B took over before
/// A's read, A then standing by on B (§2.5 step 2; text 0.2, F-203).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn s3_a_simultaneous_start() {
    let (_r1, ep) = router(None).await;
    let s = spy(&ep, LEDGER, &ledger()).await;
    let g = client(&ep).await;
    let (mut healed, mut b_kept) = (0, 0);
    // F-216 (text 0.3): a start is measured at S, by its instance token's
    // arrival; a run whose two tokens are more than 10 ms apart is made
    // again, and counted apart. A host that cannot start two within 10 ms
    // fails the section, saying how far apart its starts were (#905).
    let (mut run, mut kept, mut apart) = (0u64, Vec::new(), Vec::new());
    while run < 20 {
        let lo: u64 = rand_id();
        let (a_id, b_id) = (lo, lo + 1 + rand_id() % 1000);
        let (a, b) = start_together(&ep, Opts::ledger("A", a_id), Opts::ledger("B", b_id)).await;
        eventually("S heard both instance tokens", || async {
            s.instance_at(&a.id).is_some() && s.instance_at(&b.id).is_some()
        })
        .await;
        let (ta, tb) = (s.instance_at(&a.id).unwrap(), s.instance_at(&b.id).unwrap());
        let spread = ta.max(tb) - ta.min(tb);
        if spread > Duration::from_millis(10) {
            apart.push(spread);
            assert!(
                apart.len() < 200,
                "§3 cannot run on this host: 200 starts more than 10 ms apart at S, {} \
                 (F-216), with {run} of 20 runs made",
                spread_of(&apart)
            );
            a.close().await;
            b.close().await;
            quiet(&s, &g).await;
            continue;
        }
        kept.push(spread);
        let later = Instant::now();
        let bound = Duration::from_secs(5) + 2 * GRACE;
        let settled = loop {
            match (a.role(), b.role()) {
                (Role::Serving, Role::Standby) => break "a",
                (Role::Standby, Role::Serving) if a.why() == Some(Why::Incumbent(b.id.clone())) => {
                    break "b";
                }
                _ => {}
            }
            assert!(
                later.elapsed() < bound + JITTER,
                "run {run}: one serving and the other standing by within {bound:?}: A {:?} {:?}, B {:?} {:?}",
                a.role(),
                a.why(),
                b.role(),
                b.why()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        let (holder, standby) = if settled == "a" { (&a, &b) } else { (&b, &a) };
        if settled == "b" {
            b_kept += 1;
        }
        s.holders_are(&ledger(), &[&holder.id]).await;
        // Both reads agree on the bus: one holder, the other marked.
        eventually("G reads one holder", || async {
            let (complete, h) = g_read(&g, LEDGER, &ledger()).await;
            complete && h == BTreeSet::from([holder.id.clone()])
        })
        .await;
        let found = presence::descriptor(&g, &LEDGER.parse().unwrap(), &standby.id, T)
            .await
            .unwrap();
        assert_eq!(
            mark_of(&found, &ledger()),
            Some(true),
            "run {run}: the standby is marked"
        );
        // Each claim before its token: its descriptor, marked, at its
        // instance token, unless it took over before that GET was answered,
        // its interface token at S within a round trip of the answer
        // (F-202; text 0.4, F-223).
        for c in [&a, &b] {
            eventually("S's GET at the instance token answered", || async {
                s.at_tokens(&ledger(), &c.id).0.is_some()
            })
            .await;
            let (at_instance, _) = s.at_tokens(&ledger(), &c.id);
            assert!(
                at_instance == Some(true) || s.token_beat_the_get(&ledger(), &c.id),
                "run {run}: {} marked at its instance token: {at_instance:?}",
                c.name
            );
        }
        if s.alive_events(&ledger(), &standby.id)
            .iter()
            .any(|(_, p)| *p)
        {
            healed += 1;
        }
        a.close().await;
        b.close().await;
        quiet(&s, &g).await;
        run += 1;
    }
    println!(
        "§3: {healed} of 20 runs needed the heal; in {b_kept} B kept serving, A standing by on \
         it; their starts {} apart at S; {} start(s) more than 10 ms apart made again, {}",
        spread_of(&kept),
        apart.len(),
        spread_of(&apart)
    );
}

/// Waits until `lab/ledger` holds no token, at S and in G's read.
async fn quiet(s: &Spy, g: &zenoh::Session) {
    s.holders_are(&ledger(), &[]).await;
    eventually("no instance token left", || async {
        let r = presence::liveliness_read(g, &format!("zk2/{LEDGER}/@zk/**"), T)
            .await
            .unwrap();
        r.complete && r.keys.is_empty()
    })
    .await;
}

/// A fresh instance id below 2^62, so that the next ones stay ids.
fn rand_id() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    );
    h.finish() >> 2
}

// --- §4 ------------------------------------------------------------------------------

/// §4 A live incumbent, and a lower newcomer (§2.5 step 2).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn s4_a_live_incumbent_and_a_lower_newcomer() {
    let (_r1, ep) = router(None).await;
    let s = spy(&ep, LEDGER, &ledger()).await;
    let b = candidate(&ep, Opts::ledger("B", 0xb)).await.unwrap();
    b.until(Role::Serving).await;
    s.holders_are(&ledger(), &[&b.id]).await;

    // 1. A starts, its id lower than B's. 2. The test waits 10 s.
    let a = candidate(&ep, Opts::ledger("A", 0xa)).await.unwrap();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        assert_ne!(a.role(), Role::Serving, "A never serves");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        a.why(),
        Some(Why::Incumbent(b.id.clone())),
        "A stands by on B"
    );
    assert!(
        s.alive_events(&ledger(), &a.id).is_empty(),
        "no token of A's, ever"
    );
    assert_eq!(
        s.alive_events(&ledger(), &b.id),
        s.alive_events(&ledger(), &b.id)
            .into_iter()
            .filter(|(_, p)| *p)
            .collect::<Vec<_>>(),
        "B never stepped down"
    );
    assert_eq!(b.role(), Role::Serving);
    assert!(
        s.descriptors(&a.id).iter().all(|m| *m == Some(true)),
        "A's descriptor marked throughout"
    );
    a.close().await;
    b.close().await;
}

// --- §5 ------------------------------------------------------------------------------

/// A free loopback port, for a router that starts later (§5).
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// A peer instance's sessions (§5): each a peer whose only endpoint is
/// `ep`, listening on loopback and gossiping, as core §8.1 asks of peers
/// that read each other (0.40, F-206), its connect retry period 0.5 s at
/// most (F-205); none stamping (v2 §2.10).
async fn peer_candidate(ep: &str, o: Opts) -> Candidate {
    let claim = common::peer(ep, true, true).await;
    let watch = common::peer(ep, true, true).await;
    let reader = common::peer(ep, true, true).await;
    with_sessions(claim, watch, reader, o).await.unwrap()
}

/// [`candidate`] on sessions given.
async fn with_sessions(
    claim: zenoh::Session,
    watch: zenoh::Session,
    reader: zenoh::Session,
    o: Opts,
) -> Result<Candidate, Error> {
    let iface = o.imp.iface().clone();
    let mut b = ServiceBuilder::new(
        &claim,
        ServiceConfig::new(o.address.parse::<Addr>().unwrap()),
    );
    b.set_instance_id(id(o.id));
    b.implement(o.imp.clone())?;
    let none = Bindings::new();
    let state = Arc::new(
        b.declare_state_writer(&iface, "state/balance", &none)
            .await?,
    );
    let stream = Some(b.declare_writer(&iface, "stream/entries", &none).await?);
    let calls = Arc::new(AtomicU64::new(0));
    let (n, name) = (Arc::clone(&calls), o.name);
    let server = b
        .serve(&iface, "@op/append", Some(&none), move |call| {
            let n = Arc::clone(&n);
            async move {
                n.fetch_add(1, Ordering::SeqCst);
                call.reply(name)
                    .await
                    .map_err(|e| zenkey::OpError::internal(e.to_string()))
            }
        })
        .await?;
    let red = b.redundancy(
        RedundancyStartup::new(reader.clone())
            .with_watch(watch.clone())
            .with_retry(RETRY)
            .with_grace(GRACE)
            .with_takeover(Arc::new(PutName {
                state: Arc::clone(&state),
                name: Some(o.name),
            })),
    )?;
    let svc = b.start().await?;
    Ok(Candidate {
        name: o.name,
        id: id(o.id),
        iface,
        svc,
        red,
        calls,
        state,
        stream,
        servers: vec![server],
        sessions: vec![claim, watch, reader],
    })
}

/// §5 An unreadable claim set (§2.5 step 3).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn s5_an_unreadable_claim_set() {
    // 1. A starts as a peer of R1, while R1 is down: it reaches no node.
    let port = free_port();
    let r1_ep = format!("tcp/127.0.0.1:{port}");
    let a = peer_candidate(&r1_ep, Opts::ledger("A", 0xa)).await;
    tokio::time::sleep(RETRY + Duration::from_secs(1)).await;
    let reads = a.red.reads(&ledger());
    assert_eq!(
        a.role(),
        Role::Standby,
        "no takeover through a session cut off"
    );
    assert_eq!(
        a.why(),
        Some(Why::Incomplete),
        "a read not made stands it by"
    );
    assert_eq!(
        reads.made, 0,
        "no read made through its cut-off reader: {reads:?}"
    );
    assert!(
        reads.not_made >= 2,
        "its reads not made, said so: {reads:?}"
    );
    assert!(
        reads
            .last
            .as_deref()
            .is_some_and(|l| l.starts_with("not_made"))
    );
    assert_eq!(presence::liveliness_gets(a.reader()).reads, 0);
    assert!(!a.servers[0].is_declared(), "no queryable of ledger.v1");
    assert!(a.svc.tokens_held().is_empty(), "no interface token");

    // 2. R1 starts: within the retry interval plus its reconnect, plus 2 s,
    //    A reads complete, finds its own claim, and serves (F-205).
    let (_r1, ep) = common::router_with(
        None,
        &[
            ("listen/endpoints", &format!("[\"{r1_ep}\"]")),
            ("scouting/gossip/enabled", "true"),
        ],
    )
    .await;
    assert_eq!(ep, r1_ep);
    let up = Instant::now();
    a.until(Role::Serving).await;
    let took = up.elapsed();
    assert!(
        took < RETRY + Duration::from_secs(1) + Duration::from_secs(2) + JITTER,
        "A serves within the retry interval plus its reconnect plus 2 s: {took:?}"
    );
    assert!(a.red.reads(&ledger()).complete >= 1);
    a.close().await;

    // 3. Separately: through R3, which refuses its reads, every read is
    //    complete and empty, its own claim missing; A never serves.
    let acl = r#"{
        enabled: true,
        default_permission: "allow",
        rules: [{
            id: "no-presence-reads",
            messages: ["liveliness_query"],
            flows: ["ingress"],
            permission: "deny",
            key_exprs: ["zk2/*/*/@zk/**"],
        }],
        subjects: [{ id: "anyone" }],
        policies: [{ rules: ["no-presence-reads"], subjects: ["anyone"] }],
    }"#;
    let (_r3, ep3) = common::router_with(None, &[("access_control", acl)]).await;
    let s = spy(&ep3, LEDGER, &ledger()).await;
    let a = candidate(&ep3, Opts::ledger("A", 0xa)).await.unwrap();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        assert_ne!(a.role(), Role::Serving, "never the only candidate");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let reads = a.red.reads(&ledger());
    assert!(
        reads.made >= 4,
        "a campaign at each retry interval: {reads:?}"
    );
    assert_eq!(reads.made, reads.complete, "every read complete: {reads:?}");
    assert_eq!(a.why(), Some(Why::OwnClaimMissing), "its own claim missing");
    assert!(s.alive_events(&ledger(), &a.id).is_empty());
    // F-204: its claim's own session can read its claim. Not every time:
    // the router's final reply can end the read before the session's own
    // answer (observer §3.8, #992), so five reads, each complete, and one
    // at least holding the claim.
    let claim = a.svc.instance_key().unwrap().to_string();
    let mut held = 0;
    for _ in 0..5 {
        let own = presence::liveliness_read(&a.sessions[0], &format!("zk2/{LEDGER}/@zk/**"), T)
            .await
            .unwrap();
        assert!(own.complete, "the claim's session answers itself: {own:?}");
        assert!(
            own.keys.iter().all(|k| *k == claim),
            "nothing but its claim, from the session itself: {own:?}"
        );
        held += usize::from(!own.keys.is_empty());
    }
    assert!(
        held > 0,
        "the claim's session held its claim in none of 5 reads"
    );
    a.close().await;

    // The control: the same instance, as a client of a router with no
    // access control, serves within 3 s.
    let (_r4, ep4) = router(None).await;
    let started = Instant::now();
    let a = candidate(&ep4, Opts::ledger("A", 0xa)).await.unwrap();
    a.until(Role::Serving).await;
    assert!(started.elapsed() < Duration::from_secs(3) + JITTER);
    a.close().await;
}

// --- §6 ------------------------------------------------------------------------------

/// §6 A withdrawal that is no vacancy (§2.7, §2.8; core §8.1).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn s6_a_withdrawal_that_is_no_vacancy() {
    let (_r1, ep1) = router(None).await;
    let (_r2, ep2) = router(Some(&ep1)).await;
    let s = spy(&ep1, LEDGER, &ledger()).await;
    let a = candidate(&ep1, Opts::ledger("A", 0xa)).await.unwrap();
    a.until(Role::Serving).await;
    let link = common::link_to(&ep2).await;
    let b = candidate(link.endpoint(), Opts::ledger("B", 0xb))
        .await
        .unwrap();
    eventually("B stands by on A", || async {
        b.why() == Some(Why::Incumbent(a.id.clone()))
    })
    .await;
    let before = b.red.reads(&ledger());

    // 1. B's own link to R2 cut for 1 s, then restored.
    link.cut();
    eventually("B's session reaches no node", || async {
        presence::cut_off(b.reader()).await
    })
    .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let during = b.red.reads(&ledger());
    link.heal();
    eventually("B stands by on A again, its read made", || async {
        let r = b.red.reads(&ledger());
        r.made > during.made && b.why() == Some(Why::Incumbent(a.id.clone()))
    })
    .await;
    // Its watch heard A's tokens withdrawn (its own session's view), and
    // it made no read while its session reached no node.
    assert!(
        during.not_made > before.not_made,
        "the withdrawal's read, not made: {during:?}"
    );
    assert_eq!(
        during.made, before.made,
        "no read made while cut off: {during:?}"
    );
    tokio::time::sleep(RETRY + Duration::from_secs(1)).await;
    assert_eq!(b.role(), Role::Standby, "B never takes over");
    assert!(
        s.alive_events(&ledger(), &b.id).is_empty(),
        "no token of B's at R1, ever"
    );
    assert_eq!(s.holders(&ledger()), BTreeSet::from([a.id.clone()]));
    a.close().await;
    b.close().await;
}

// --- §7 ------------------------------------------------------------------------------

/// §7's and §9's split: A, a client of R1, serves; B, a client of R2,
/// stands by; the link between R1 and R2 is cut, and B takes over behind
/// it (§7 steps 1 and 2).
struct Split {
    _r1: zenoh::Session,
    _r2: zenoh::Session,
    link: common::Link,
    ep1: String,
    s: Spy,
    s2: Spy,
    a: Candidate,
    b: Candidate,
}

async fn split() -> Split {
    let (r1, ep1) = router(None).await;
    let link = common::link_to(&ep1).await;
    let (r2, ep2) = common::router_via(&link).await;
    let s = spy(&ep1, LEDGER, &ledger()).await;
    let s2 = spy(&ep2, LEDGER, &ledger()).await;
    let a = candidate(&ep1, Opts::ledger("A", 0xa)).await.unwrap();
    a.until(Role::Serving).await;
    s2.holders_are(&ledger(), &[&a.id]).await;
    let b = candidate(&ep2, Opts::ledger("B", 0xb)).await.unwrap();
    eventually("B stands by on A", || async {
        b.why() == Some(Why::Incumbent(a.id.clone()))
    })
    .await;

    // 1. The link between R1 and R2 cut. 2. Until S2 sees B's token.
    link.cut();
    s2.holders_are(&ledger(), &[&b.id]).await;
    b.until(Role::Serving).await;
    assert_eq!(a.role(), Role::Serving, "A still serves");
    assert_eq!(
        s.holders(&ledger()),
        BTreeSet::from([a.id.clone()]),
        "S sees nothing of B's token"
    );
    Split {
        _r1: r1,
        _r2: r2,
        link,
        ep1,
        s,
        s2,
        a,
        b,
    }
}

/// §7 The heal of a split (§2.9; core §6).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn s7_the_heal_of_a_split() {
    let Split {
        _r1,
        _r2,
        link,
        ep1,
        s,
        s2,
        a,
        b,
    } = split().await;
    // 3. A puts a1; B puts b1. Each side receives its own incumbent's.
    a.state.put("a1").await.unwrap();
    b.state.put("b1").await.unwrap();
    let got = |spy: &Spy, v: &str| {
        spy.snapshot()
            .iter()
            .any(|r| matches!(r, Rec::Data { payload, .. } if payload == v))
    };
    eventually("each side's own put", || async {
        got(&s, "a1") && got(&s2, "b1")
    })
    .await;
    assert!(!got(&s, "b1") && !got(&s2, "a1"), "only its own side's");

    // 4. The link healed at H: B, the higher, steps down within two graces
    //    plus 2 s; A never does.
    let h = Instant::now();
    link.heal();
    let b_run = b.run();
    b.until(Role::Standby).await;
    let stepped = h.elapsed();
    assert!(
        stepped < 2 * GRACE + Duration::from_secs(2) + JITTER,
        "B steps down within two graces plus 2 s of H: {stepped:?}"
    );
    s.holders_are(&ledger(), &[&a.id]).await;
    s2.holders_are(&ledger(), &[&a.id]).await;
    // S and S2 each saw both tokens after H: each the other side's.
    for (spy, other) in [(&s, &b.id), (&s2, &a.id)] {
        assert!(
            spy.alive_events(&ledger(), other)
                .iter()
                .any(|(at, put)| *put && *at >= h),
            "{other}'s token seen across the healed link"
        );
    }
    // v2's §7, Expected 4 by run: S and S2 record no put or delete under
    // B's run after B's descriptor put marked standby, which B makes once
    // its writes ended (v1 §2.9 step 3), one session's puts on one
    // priority arriving in order.
    for spy in [&s, &s2] {
        let marked = eventually_some("B's descriptor put marked, after H", || async {
            spy.descriptor_puts(&b.id)
                .into_iter()
                .find(|(at, m, _)| *m == Some(true) && *at >= h)
                .map(|(at, ..)| at)
        })
        .await;
        let late: Vec<_> = spy
            .data_under(b_run)
            .into_iter()
            .filter(|(at, ..)| *at > marked)
            .collect();
        assert!(
            late.is_empty(),
            "under B's run after its step-down: {late:?}"
        );
    }
    // Its queryables gone, its descriptor marked.
    let x = client(&ep1).await;
    eventually("an All GET answered by A alone", || async {
        call(&x, LEDGER, &ledger(), "append", true).await == ["A"]
    })
    .await;
    assert!(
        !b.servers[0].is_declared(),
        "B's operation queryable undeclared"
    );
    let found = presence::descriptor(&x, &LEDGER.parse().unwrap(), &b.id, T)
        .await
        .unwrap();
    assert_eq!(
        mark_of(&found, &ledger()),
        Some(true),
        "B's descriptor marked"
    );
    assert!(
        !s.alive_events(&ledger(), &a.id).iter().any(|(_, put)| !put),
        "A never steps down"
    );
    assert!(
        matches!(b.state.put("late").await, Err(Error::Standby(_))),
        "a standby writes nothing"
    );
    tokio::time::sleep(2 * GRACE).await;
    assert_eq!(a.role(), Role::Serving, "A stands alone as the incumbent");
    assert_eq!(s.holders(&ledger()), BTreeSet::from([a.id.clone()]));
    a.close().await;
    b.close().await;
}

// --- §8 ------------------------------------------------------------------------------

/// §8 A call finds no standby (§2.4; core O1, O3).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn s8_a_call_finds_no_standby() {
    let (_r1, ep) = router(None).await;
    let x = client(&ep).await;
    let a = candidate(&ep, Opts::ledger("A", 0xa)).await.unwrap();
    a.until(Role::Serving).await;
    let b = candidate(&ep, Opts::ledger("B", 0xb)).await.unwrap();
    eventually("B stands by on A", || async {
        b.why() == Some(Why::Incumbent(a.id.clone()))
    })
    .await;
    assert!(
        !b.servers[0].is_declared(),
        "B declares no operation queryable"
    );
    assert!(
        b.svc.unavailable_queryables().is_empty(),
        "and no unavailable one"
    );

    // 1. 20 calls, BestMatching, consolidation None: all on A.
    for _ in 0..20 {
        assert_eq!(call(&x, LEDGER, &ledger(), "append", false).await, ["A"]);
    }
    assert_eq!(
        (
            a.calls.load(Ordering::SeqCst),
            b.calls.load(Ordering::SeqCst)
        ),
        (20, 0)
    );
    // 2. A fan-out's GET, target All: one reply, A's; v2's Expected 2 by
    //    run: its value's run is A's (core §5.1), and X reads it so.
    assert_eq!(call(&x, LEDGER, &ledger(), "append", true).await, ["A"]);
    assert_eq!(
        call_runs(&x, LEDGER, &ledger(), "append", true).await,
        [Some(a.run())],
        "the one reply's as-of mark names A's run"
    );
    // 3. A closes; a call at once, before B serves: no value and no
    //    envelope from B.
    a.close().await;
    let before = b.role();
    let replies = call(&x, LEDGER, &ledger(), "append", false).await;
    if before == Role::Standby {
        assert!(
            replies.iter().all(|r| r == "!"),
            "silence, or zenoh's own error reply: {replies:?}"
        );
    }
    assert_eq!(
        b.calls.load(Ordering::SeqCst),
        u64::from(replies.contains(&"B".to_owned()))
    );
    b.until(Role::Serving).await;
    b.close().await;
}

/// A lock file in a directory of the test's.
fn lock_file(dir: &tempfile::TempDir, name: &str) -> Arc<FileLock> {
    Arc::new(FileLock::new(dir.path().join(name)))
}

/// §10 step 3's crash: `silent` makes V1's link silent first (a lost host,
/// at the lease), else cuts it (seen at once; text 0.2, F-201). The
/// kernel releases V1's lock either way: its handle drops.
async fn s10_crash(silent: bool) {
    let dir = tempfile::tempdir().unwrap();
    let (_r1, ep) = router(None).await;
    let s = spy(&ep, VALVE, &valve()).await;
    let g = client(&ep).await;
    let link = common::link_to(&ep).await;
    // 1. V1 starts and takes the lock: it serves.
    let v1_lock = lock_file(&dir, "valve.lock");
    let v1 = candidate(
        link.endpoint(),
        Opts::valve("V1", 0x2, Arc::clone(&v1_lock) as Arc<dyn Lock>).lease(LEASE),
    )
    .await
    .unwrap();
    v1.until(Role::Serving).await;
    assert!(v1_lock.held());
    drop(v1_lock);
    // 2. V2, its id lower, starts; ten seconds: it stands by, marked.
    let v2_lock = lock_file(&dir, "valve.lock");
    let v2 = candidate(
        &ep,
        Opts::valve("V2", 0x1, Arc::clone(&v2_lock) as Arc<dyn Lock>),
    )
    .await
    .unwrap();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        assert_ne!(v2.role(), Role::Serving, "no election by ids moves V2");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(v2.why(), Some(Why::NoLock));
    assert!(s.descriptors(&v2.id).iter().all(|m| *m == Some(true)));
    // 3. V1 crashes at K, and its lock goes with it.
    let k = Instant::now();
    if silent {
        link.stall();
    } else {
        link.cut();
    }
    v1.crash();
    eventually("V2 takes the lock at once", || async { v2_lock.held() }).await;
    assert!(
        k.elapsed() < Duration::from_secs(2) + JITTER,
        "within its lock period"
    );
    // Two reads a grace apart never both show V1 and V2 holding.
    let watch = async {
        loop {
            let (_, first) = g_read(&g, VALVE, &valve()).await;
            tokio::time::sleep(GRACE).await;
            let (_, second) = g_read(&g, VALVE, &valve()).await;
            assert!(
                first.len() < 2 || second.len() < 2,
                "a split: {first:?}, {second:?}"
            );
            if second == BTreeSet::from([v2.id.clone()]) {
                return;
            }
        }
    };
    tokio::time::timeout(SETTLE, watch)
        .await
        .expect("V2 serves");
    let w = s
        .alive_events(&valve(), &id(0x2))
        .iter()
        .find(|(_, put)| !put)
        .map(|(at, _)| *at)
        .expect("V1's token withdrawn");
    if silent {
        assert!(w - k >= LEASE / 2, "V2 waited for V1's lease: {:?}", w - k);
    }
    let (served, _) = s.alive_events(&valve(), &v2.id)[0];
    assert!(served > w, "V2's token after V1's withdrawal");
    assert!(
        served - w < Duration::from_secs(3) + JITTER,
        "within 3 s of it"
    );
    eventually("S's GET at V2's token answered", || async {
        s.at_tokens(&valve(), &v2.id).1.is_some()
    })
    .await;
    assert_eq!(s.at_tokens(&valve(), &v2.id).1, Some(false), "§2.6's order");
    v2.close().await;
}

/// §10 `external` (§2.11).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn s10_external() {
    // Steps 1 to 3, the crash on a silent link, then on one cut.
    s10_crash(true).await;
    s10_crash(false).await;

    // 4. Separately, two lock files, V2 first: V2 serves; V1, its own lock
    //    held, stands by on V2's token (text 0.2, F-200).
    let dir = tempfile::tempdir().unwrap();
    let (_r1, ep) = router(None).await;
    let v2 = candidate(&ep, Opts::valve("V2", 0x1, lock_file(&dir, "a.lock")))
        .await
        .unwrap();
    v2.until(Role::Serving).await;
    let v1 = candidate(&ep, Opts::valve("V1", 0x2, lock_file(&dir, "b.lock")))
        .await
        .unwrap();
    eventually("V1 stands by on V2", || async {
        v1.why() == Some(Why::Incumbent(v2.id.clone()))
    })
    .await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!((v1.role(), v2.role()), (Role::Standby, Role::Serving));
    v1.close().await;
    v2.close().await;

    // 4 (F-200's deterministic step): two lock files, behind two routers
    //    whose link is cut; both serve. Restored, neither steps down within
    //    10 s, and G's two reads read a split.
    let (_r1, ep1) = router(None).await;
    let link = common::link_to(&ep1).await;
    link.cut();
    let (_r2, ep2) = common::router_via(&link).await;
    let s = spy(&ep1, VALVE, &valve()).await;
    let g = client(&ep1).await;
    let v1 = candidate(&ep1, Opts::valve("V1", 0x2, lock_file(&dir, "c.lock")))
        .await
        .unwrap();
    let v2 = candidate(&ep2, Opts::valve("V2", 0x1, lock_file(&dir, "d.lock")))
        .await
        .unwrap();
    v1.until(Role::Serving).await;
    v2.until(Role::Serving).await;
    link.heal();
    s.holders_are(&valve(), &[&v1.id, &v2.id]).await;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        assert_eq!(
            (v1.role(), v2.role()),
            (Role::Serving, Role::Serving),
            "external never heals"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let read = |r: (bool, BTreeSet<InstanceId>)| model::Read {
        complete: r.0,
        instances: r
            .1
            .into_iter()
            .map(|i| model::Seen {
                id: i,
                token: true,
                descriptor: model::Listing::Listed { standby: false },
            })
            .collect(),
    };
    let first = read(g_read(&g, VALVE, &valve()).await);
    tokio::time::sleep(GRACE).await;
    let second = read(g_read(&g, VALVE, &valve()).await);
    let j = model::one_incumbent(&model::Reading {
        across_face: false,
        governance: model::Governance::Governed(model::Election::External),
        first,
        second,
    });
    assert_eq!(
        (j.answer, j.reason),
        (model::Answer::No, model::Reason::Split)
    );
    v1.close().await;
    v2.close().await;

    // 5. V3, configured to refuse, starts while V1 holds the lock: no
    //    instance token of V3's within 5 s, against a control.
    let (_r1, ep) = router(None).await;
    let s = spy(&ep, VALVE, &valve()).await;
    let held = lock_file(&dir, "valve.lock");
    let v1 = candidate(
        &ep,
        Opts::valve("V1", 0x2, Arc::clone(&held) as Arc<dyn Lock>),
    )
    .await
    .unwrap();
    v1.until(Role::Serving).await;
    let mut o = Opts::valve("V3", 0x3, lock_file(&dir, "valve.lock"));
    o.refuse_without_lock = true;
    let refused = candidate(&ep, o).await;
    assert!(
        matches!(&refused, Err(Error::NotExposed(why)) if why.contains("§2.11")),
        "V3 refuses to start: {:?}",
        refused.as_ref().err()
    );
    let control = candidate(&ep, Opts::valve("C", 0x4, lock_file(&dir, "valve.lock")))
        .await
        .unwrap();
    eventually("the control's instance token reaches S", || async {
        s.has_instance(&control.id)
    })
    .await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(!s.has_instance(&id(0x3)), "no instance token of V3's");
    control.close().await;
    v1.close().await;
}

// --- §11 -----------------------------------------------------------------------------

/// §11 The tokenless set, and an invalid declaration (§2.2, §2.14; core
/// §8.2).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn s11_the_tokenless_set_and_an_invalid_declaration() {
    let (_r1, ep) = router(None).await;
    let s = spy(&ep, LEDGER, &ledger()).await;
    // The control: A, configured plainly, serves within 3 s.
    let started = Instant::now();
    let a = candidate(&ep, Opts::ledger("A", 0xa)).await.unwrap();
    a.until(Role::Serving).await;
    assert!(started.elapsed() < Duration::from_secs(3) + JITTER);
    s.holders_are(&ledger(), &[&a.id]).await;
    a.close().await;
    s.holders_are(&ledger(), &[]).await;
    eventually("S heard A's instance token withdrawn", || async {
        s.snapshot().iter().any(|r| {
            matches!(r, Rec::Token { key: ZkKey::Instance { instance, .. }, put: false, .. }
                if *instance == id(0xa))
        })
    })
    .await;
    // The GET S made at A's interface token has answered, or never will.
    tokio::time::sleep(T + Duration::from_millis(200)).await;
    let quiet = s.snapshot().len();

    // 1. ledger.v1 in its tokenless set.
    let mut o = Opts::ledger("A", 0xb);
    o.tokenless = true;
    let refused = candidate(&ep, o).await;
    assert!(
        matches!(&refused, Err(Error::NotExposed(why)) if why.contains("tokenless")),
        "{:?}",
        refused.as_ref().err()
    );

    // 2. A copy of ledger.v1 whose election is "vote".
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/contracts/ledger-run.v1.toml");
    let text = std::fs::read_to_string(&path).unwrap().replace(
        "\"redundancy.election\" = \"claim\"",
        "\"redundancy.election\" = \"vote\"",
    );
    let l = zenkey::model::contract::load_str(&text, path.parent().unwrap(), None);
    let mut o = Opts::ledger("A", 0xc);
    o.imp = Implementation::new(l.contract.unwrap_or_else(|| panic!("{}", l.report)));
    let refused = candidate(&ep, o).await;
    assert!(
        matches!(&refused, Err(Error::NotExposed(why)) if why.contains("value")),
        "{:?}",
        refused.as_ref().err()
    );

    // 3. A copy of ledger-run.v1 whose `entries` is an explicit stream: its
    //    samples carry no envelope, so no run (v2 §2.2), `explicit_stream`.
    let text = std::fs::read_to_string(&path).unwrap().replace(
        "[resources.entries]\nkind = \"stream\"",
        "[resources.entries]\nkind = \"stream\"\nexplicit = true",
    );
    assert!(text.contains("explicit = true"));
    let l = zenkey::model::contract::load_str(&text, path.parent().unwrap(), None);
    let mut o = Opts::ledger("A", 0xd);
    o.imp = Implementation::new(l.contract.unwrap_or_else(|| panic!("{}", l.report)));
    let refused = candidate(&ep, o).await;
    assert!(
        matches!(&refused, Err(Error::NotExposed(why)) if why.contains("invalid: explicit_stream")),
        "{:?}",
        refused.as_ref().err()
    );

    // 4. A configured for a first hop (core §8.1), every interface in its
    //    tokenless set: refused, its election needing presence (v2 §2.13).
    let mut o = Opts::ledger("A", 0xe);
    o.first_hop = true;
    let refused = candidate(&ep, o).await;
    assert!(
        matches!(&refused, Err(Error::NotExposed(why))
            if why.contains("first hop") && why.contains("redundancy.v2")),
        "{:?}",
        refused.as_ref().err()
    );
    // None of the four declared or put anything of lab/ledger within 5 s:
    // no instance token of A's crosses to R1.
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(
        s.snapshot().len(),
        quiet,
        "nothing of lab/ledger: {:?}",
        &s.snapshot()[quiet..]
    );
}

// --- K, the follower, and X's puts ---------------------------------------------------

/// `balance`'s key, as every session here spells it.
const BALANCE: &str = "zk2/lab/ledger/ledger.v1/state/balance";

fn ledger_addr() -> Addr {
    LEDGER.parse().unwrap()
}

/// What K told, each when it was told.
type Told = Arc<Mutex<Vec<(Instant, FollowEvent)>>>;

/// K (v2 §2.10): the runtime's follower of `revision`, bound to
/// `lab/ledger`, a client of `ep` with a watch session of its own, its
/// descriptor subscriber delayed by `delay` (§14's hook; its GETs are not).
struct K {
    following: Following,
    told: Told,
    sessions: Vec<zenoh::Session>,
}

async fn follower(ep: &str, revision: &str, delay: Duration) -> K {
    let session = client(ep).await;
    let watch = client(ep).await;
    let consumer = zenkey::consumer::Consumer::for_tool(
        &session,
        Arc::new(common::contract(revision)),
        &[LEDGER],
        &BTreeMap::new(),
    )
    .unwrap()
    .with_watch(watch.clone());
    let told: Told = Arc::default();
    let t = Arc::clone(&told);
    let following = consumer
        .follow_with(
            "state/balance",
            FollowStartup {
                descriptor_delay: delay,
                ..FollowStartup::default()
            },
            move |e| t.lock().unwrap().push((Instant::now(), e)),
        )
        .await
        .unwrap();
    K {
        following,
        told,
        sessions: vec![session, watch],
    }
}

/// A delivery's record, lossy UTF-8.
fn record_of(d: &zenkey::consumer::Delivery) -> String {
    String::from_utf8_lossy(&d.sample.payload().to_bytes()).into_owned()
}

impl K {
    fn told(&self) -> Vec<(Instant, FollowEvent)> {
        self.told.lock().unwrap().clone()
    }

    /// What it holds of `balance`: the record, and the run it was put
    /// under.
    fn value(&self) -> Option<(String, Option<Run>)> {
        self.following
            .value(BALANCE)
            .map(|d| (record_of(&d), d.inband.map(|s| s.run)))
    }

    fn holds(&self, v: &str) -> bool {
        self.value().is_some_and(|(r, _)| r == v)
    }

    fn incumbent(&self) -> Option<InstanceId> {
        self.following.incumbent(&ledger_addr())
    }

    /// Each sample it accepted, live or `seeded`: its record and run, in
    /// order.
    fn accepted(&self, seeded: bool) -> Vec<(String, Option<Run>)> {
        self.told()
            .into_iter()
            .filter_map(|(_, e)| match e {
                FollowEvent::Accepted {
                    delivery,
                    seeded: s,
                } if s == seeded => Some((record_of(&delivery), delivery.inband.map(|s| s.run))),
                _ => None,
            })
            .collect()
    }

    /// Each live sample it discarded: its record, run and the reason.
    fn discarded(&self) -> Vec<(String, Option<Run>, Discard)> {
        self.told()
            .into_iter()
            .filter_map(|(_, e)| match e {
                FollowEvent::Discarded { delivery, why } => {
                    Some((record_of(&delivery), delivery.inband.map(|s| s.run), why))
                }
                _ => None,
            })
            .collect()
    }

    /// Each reply of a seed's it discarded: its run and the reason.
    fn seed_discarded(&self) -> Vec<(Option<Run>, Discard)> {
        self.told()
            .into_iter()
            .filter_map(|(_, e)| match e {
                FollowEvent::SeedDiscarded { run, why, .. } => Some((run, why)),
                _ => None,
            })
            .collect()
    }

    /// Each seed it made: the incumbent it read, and how many of its
    /// snapshots it merged.
    fn seeds(&self) -> Vec<(Option<InstanceId>, usize)> {
        self.told()
            .into_iter()
            .filter_map(|(_, e)| match e {
                FollowEvent::Seeded {
                    incumbent,
                    snapshots,
                    ..
                } => Some((incumbent, snapshots)),
                _ => None,
            })
            .collect()
    }

    /// When it told `want` of the sample whose record is `record`.
    fn when(
        &self,
        record: &str,
        want: fn(&FollowEvent) -> Option<&zenkey::consumer::Delivery>,
    ) -> Option<Instant> {
        self.told()
            .iter()
            .find(|(_, e)| want(e).is_some_and(|d| record_of(d) == record))
            .map(|(at, _)| *at)
    }

    async fn close(self) {
        drop(self.following);
        for s in self.sessions {
            s.close().await.unwrap();
        }
    }
}

fn held(e: &FollowEvent) -> Option<&zenkey::consumer::Delivery> {
    match e {
        FollowEvent::Held { delivery } => Some(delivery),
        _ => None,
    }
}

fn accepted_live(e: &FollowEvent) -> Option<&zenkey::consumer::Delivery> {
    match e {
        FollowEvent::Accepted {
            delivery,
            seeded: false,
        } => Some(delivery),
        _ => None,
    }
}

/// X's put of `balance`: `record` under `run` at `seq`, as an owner's
/// mutation would carry it (core §7.5), from a session no descriptor names.
async fn put_under(x: &zenoh::Session, run: Run, seq: u32, record: &str) {
    let stamp = Stamp {
        run,
        seq: zenkey::model::wire::Seq::new(seq).unwrap(),
    };
    let bytes = zenkey::model::wire::Envelope::mutation(
        zenkey::model::wire::Tag::NONE,
        stamp,
        record.as_bytes(),
    )
    .write();
    x.put(BALANCE, bytes)
        .encoding(zenoh::bytes::Encoding::ZENOH_BYTES)
        .await
        .unwrap();
}

/// P (core S7 alone): every value of `balance` its consumer rule accepts,
/// in order, by seq within a run and the first value under a new run.
async fn s7_reader(
    ep: &str,
) -> (
    zenoh::Session,
    zenoh::pubsub::Subscriber<()>,
    Arc<Mutex<Vec<String>>>,
) {
    let session = client(ep).await;
    let values: Arc<Mutex<Vec<String>>> = Arc::default();
    let order = Arc::new(Mutex::new(zenkey::state::ValueOrder::new()));
    let (pv, po) = (Arc::clone(&values), Arc::clone(&order));
    let sub = session
        .declare_subscriber(BALANCE)
        .callback(move |x: Sample| {
            let mark = zenkey::inband::read(x.clone()).mark();
            if let Some(m) = mark
                && po.lock().unwrap().accept_mark(x.key_expr().as_str(), m)
            {
                pv.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&common::record(&x)).into_owned());
            }
        })
        .await
        .unwrap();
    (session, sub, values)
}

// --- §9 ------------------------------------------------------------------------------

/// §9 A follower during a split (v2 §2.10; core S4, S7).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn s9_a_follower_during_a_split() {
    let Split {
        _r1,
        _r2,
        link,
        ep1,
        s,
        s2: _s2,
        a,
        b,
    } = split().await;
    let (a_run, b_run) = (a.run(), b.run());
    assert_ne!(a_run, b_run, "each states its own run");
    let (p_session, _p, p_values) = s7_reader(&ep1).await;
    let k = follower(&ep1, "ledger-run.v1", Duration::ZERO).await;
    eventually("K follows A", || async {
        k.incumbent() == Some(a.id.clone())
    })
    .await;

    // 1. The link healed, B's step-down held (`hold_heal`); A and B put
    //    balance five times each, alternately, 100 ms apart.
    b.red.hold_heal(true);
    link.heal();
    s.holders_are(&ledger(), &[&a.id, &b.id]).await;
    assert_eq!(k.incumbent(), Some(a.id.clone()), "the lowest holder");
    let mut k2 = None;
    for n in 1..=5 {
        a.state.put(format!("a{n}")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        b.state.put(format!("b{n}")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        if n == 3 {
            // 2. K restarts during step 1, and seeds (core S4).
            let next = follower(&ep1, "ledger-run.v1", Duration::ZERO).await;
            eventually("the restarted K seeded", || async {
                !next.seeds().is_empty()
            })
            .await;
            k2 = Some(next);
        }
    }

    // Expected 1. P's value moves at each put; K's to A's values only, in
    // A's order, every sample under B's run discarded `not_incumbent`.
    let want_p: Vec<String> = (1..=5)
        .flat_map(|n| [format!("a{n}"), format!("b{n}")])
        .collect();
    eventually("P took all ten, flapping", || async {
        *p_values.lock().unwrap() == want_p
    })
    .await;
    let want_k: Vec<(String, Option<Run>)> =
        (1..=5).map(|n| (format!("a{n}"), Some(a_run))).collect();
    eventually("K took A's five, in order", || async {
        k.accepted(false) == want_k
    })
    .await;
    eventually("K discarded B's five", || async {
        k.discarded().len() == 5
    })
    .await;
    let want_d: Vec<(String, Option<Run>, Discard)> = (1..=5)
        .map(|n| (format!("b{n}"), Some(b_run), Discard::NotIncumbent))
        .collect();
    assert_eq!(
        k.discarded(),
        want_d,
        "every one under B's run, not_incumbent"
    );

    // Expected 2. No put carries a Zenoh timestamp of its own: S records
    // each with its router's id, never A's nor B's meta.zid.
    let puts: Vec<_> = s
        .data()
        .into_iter()
        .filter(|(_, key, p, ..)| key == BALANCE && want_p.contains(p))
        .collect();
    assert_eq!(puts.len(), 10, "S heard the ten");
    for (_, _, p, stamp, _) in &puts {
        let id = stamp.map(|t| t.get_id().to_string());
        let router = if p.starts_with('a') {
            _r1.zid()
        } else {
            _r2.zid()
        };
        assert_eq!(id, Some(router.to_string()), "{p}: its router's id");
        assert!(
            id != Some(a.zid()) && id != Some(b.zid()),
            "{p}: no stamp of A's or B's"
        );
    }

    // Expected 3. The restarted K's seed: under S4's None both snapshots
    // of balance arrive, A's under A's run, B's under B's; it merges A's and
    // discards B's, not_incumbent; it holds no value of B's.
    let k2 = k2.unwrap();
    assert_eq!(k2.seeds()[0].0, Some(a.id.clone()), "seeded under A");
    assert_eq!(
        k2.seed_discarded(),
        [(Some(b_run), Discard::NotIncumbent)],
        "B's snapshot arrived, and was discarded"
    );
    assert!(
        k2.accepted(true)
            .iter()
            .all(|(v, r)| v.starts_with('a') && *r == Some(a_run)),
        "only A's snapshot merged: {:?}",
        k2.accepted(true)
    );
    assert!(
        k2.value()
            .is_none_or(|(v, r)| v.starts_with('a') && r == Some(a_run)),
        "never B's: {:?}",
        k2.value()
    );

    // 3. B steps down. Expected 4: K's value stays A's, and a sample of B's
    //    in flight when it stepped down, arriving after, is discarded
    //    not_incumbent.
    b.red.hold_heal(false);
    b.until(Role::Standby).await;
    s.holders_are(&ledger(), &[&a.id]).await;
    let x = client(&ep1).await;
    put_under(&x, b_run, 99, "b-late").await;
    eventually("K discarded B's late sample", || async {
        k.discarded()
            .iter()
            .any(|(v, r, why)| v == "b-late" && *r == Some(b_run) && *why == Discard::NotIncumbent)
    })
    .await;
    assert_eq!(k.value(), Some(("a5".to_owned(), Some(a_run))));
    a.close().await;
    b.close().await;
    k.close().await;
    k2.close().await;
    p_session.close().await.unwrap();
}

// --- §13 -----------------------------------------------------------------------------

/// §13's steps 1 and 2, `b_puts` B's takeover's put (step 3: none).
async fn s13_run(b_puts: Option<&'static str>) {
    let (_r1, ep) = router(None).await;
    let x = client(&ep).await;
    let link_a = common::link_to(&ep).await;
    let a = candidate(link_a.endpoint(), Opts::ledger("A", 0xa))
        .await
        .unwrap();
    a.until(Role::Serving).await;
    a.state.put("a1").await.unwrap();
    let mut ob = Opts::ledger("B", 0xb);
    ob.takeover = Some(b_puts);
    let b = candidate(&ep, ob).await.unwrap();
    eventually("B stands by on A", || async {
        b.why() == Some(Why::Incumbent(a.id.clone()))
    })
    .await;
    // Setup: K follows lab/ledger, and has seeded: it holds a1.
    let k = follower(&ep, "ledger-run.v1", Duration::ZERO).await;
    eventually("K seeded A's a1", || async { k.holds("a1") }).await;
    let (a_run, b_run) = (a.run(), b.run());

    // 1. A puts a2. Expected 1: K holds a2, under A's run.
    a.state.put("a2").await.unwrap();
    eventually("K holds a2", || async { k.holds("a2") }).await;
    assert_eq!(k.value(), Some(("a2".to_owned(), Some(a_run))));

    // 2. The test kills A's process. B takes over, and puts its value.
    link_a.cut();
    a.crash();
    // Expected 2: K hears A's tokens withdrawn, and sees no incumbent.
    eventually("K sees no incumbent", || async {
        k.told().iter().any(|(_, e)| {
            matches!(
                e,
                FollowEvent::Incumbent {
                    incumbent: None,
                    ..
                }
            )
        })
    })
    .await;
    b.until(Role::Serving).await;
    eventually("K follows B", || async {
        k.incumbent() == Some(b.id.clone())
    })
    .await;
    // K seeds again when B's token appears: one GET, which B answers with
    // one snapshot of balance under B's run, never a reply per key.
    let seed = eventually_some("K's seed under B", || async {
        k.seeds()
            .into_iter()
            .find(|(i, _)| *i == Some(b.id.clone()))
    })
    .await;
    assert_eq!(seed.1, 1, "one snapshot of B's merged");
    let replies = x
        .get(BALANCE)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::None)
        .timeout(T)
        .await
        .unwrap();
    let mut answered = Vec::new();
    while let Ok(r) = replies.recv_async().await {
        if let Ok(s) = r.result() {
            answered.push(zenkey::inband::is_snapshot(s));
        }
    }
    assert_eq!(answered, [true], "one reply, a snapshot (v2 §2.6)");
    match b_puts {
        Some(v) => {
            // K holds b1 either way, within a round trip of B's token. B's
            // live put of it: before its token discarded no_incumbent, after
            // it accepted under B's run.
            eventually("K holds b1", || async { k.holds(v) }).await;
            assert_eq!(k.value(), Some((v.to_owned(), Some(b_run))));
            let live: Vec<_> = k
                .told()
                .into_iter()
                .filter_map(|(_, e)| match e {
                    FollowEvent::Accepted {
                        delivery,
                        seeded: false,
                    } if record_of(&delivery) == v => Some(None),
                    FollowEvent::Discarded { delivery, why } if record_of(&delivery) == v => {
                        Some(Some(why))
                    }
                    _ => None,
                })
                .collect();
            assert!(
                live == [None] || live == [Some(Discard::NoIncumbent)],
                "B's put: accepted after its token, or no_incumbent before it: {live:?}"
            );
        }
        None => {
            // Expected 3: B's snapshot is complete and lists no entry: K
            // drops a2 by core §4.2's omission rule, and holds no value.
            eventually("K dropped a2", || async { k.value().is_none() }).await;
            assert!(
                k.told().iter().any(|(_, e)| matches!(e,
                    FollowEvent::Omitted { key, .. } if key == BALANCE)),
                "by the omission rule"
            );
        }
    }
    b.close().await;
    k.close().await;
}

/// §13 A takeover seen by a follower (v2 §2.6, §2.10): steps 1 and 2, then
/// their second run, a takeover that puts nothing (step 3).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn s13_a_takeover_seen_by_a_follower() {
    s13_run(Some("b1")).await;
    s13_run(None).await;
}

// --- §14 -----------------------------------------------------------------------------

/// §14 An incumbent's new run, and a run another instance states (v2
/// §2.10).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn s14_an_incumbents_new_run_and_a_run_another_instance_states() {
    let (_r1, ep) = router(None).await;
    let s = spy(&ep, LEDGER, &ledger()).await;
    let x = client(&ep).await;
    let a = candidate(&ep, Opts::ledger("A", 0xa)).await.unwrap();
    a.until(Role::Serving).await;
    a.state.put("a1").await.unwrap();
    // Setup: K holds A's balance, its descriptor subscriber delayed 500 ms.
    let delay = Duration::from_millis(500);
    let k = follower(&ep, "ledger-run.v1", delay).await;
    eventually("K holds a1", || async { k.holds("a1") }).await;
    // A's descriptor, for X's in step 3.
    let Found::Descriptor(_, a_descriptor) = presence::descriptor(&x, &ledger_addr(), &a.id, T)
        .await
        .unwrap()
    else {
        panic!("A's descriptor");
    };

    // 1. A takes a new run, puts its descriptor stating it, then puts a2
    //    under it.
    let first = a.run();
    let next = a.svc.new_run();
    assert_ne!(first, next);
    a.state.put("a2").await.unwrap();
    // Expected 1: K holds a2 unattributed and asks one read; when it ends,
    // K accepts a2 under A's new run, before the delayed put arrives.
    let held_at = eventually_some("K held a2", || async { k.when("a2", held) }).await;
    let accepted_at =
        eventually_some("K accepted a2", || async { k.when("a2", accepted_live) }).await;
    let put_at = eventually_some("A's descriptor put stating its new run, at S", || async {
        s.descriptor_puts(&a.id)
            .into_iter()
            .find(|(_, _, r)| *r == Some(next))
            .map(|(at, ..)| at)
    })
    .await;
    assert!(held_at <= accepted_at);
    assert!(
        accepted_at < put_at + delay,
        "accepted by the read, {:?} after the put reached S, before K handled it",
        accepted_at.saturating_duration_since(put_at)
    );
    assert_eq!(k.value(), Some(("a2".to_owned(), Some(next))));
    assert!(
        k.following.counts().reads >= 1,
        "one read of the descriptors"
    );
    assert!(
        !k.discarded().iter().any(|(v, ..)| v == "a2"),
        "never discarded"
    );

    // 2. X puts x1 under a run no descriptor states. Expected 2: K holds
    //    it, asks a read, and discards it when the read ends, unattributed;
    //    K's value stays a2.
    let stray = loop {
        let r = Run::U32(rand::random());
        if r != first && r != next {
            break r;
        }
    };
    put_under(&x, stray, 0, "x1").await;
    eventually("K discarded x1, unattributed", || async {
        k.discarded()
            .iter()
            .any(|(v, r, why)| v == "x1" && *r == Some(stray) && *why == Discard::Unattributed)
    })
    .await;
    assert!(k.when("x1", held).is_some(), "held before the read ended");
    assert_eq!(k.value(), Some(("a2".to_owned(), Some(next))));
    a.close().await;
    k.close().await;

    // 3. Separately, no instance of lab/ledger runs. X declares an instance
    //    token whose id is lower than C's, and serves a descriptor marking
    //    ledger.v1 standby whose run is R. C starts, its first run R.
    s.holders_are(&ledger(), &[]).await;
    let r = loop {
        let r = Run::U32(rand::random());
        if r != first && r != next && r != stray {
            break r;
        }
    };
    let x_id = id(0x1);
    let mut d: serde_json::Value = serde_json::from_slice(&a_descriptor).unwrap();
    d["instance"] = x_id.as_str().into();
    d["run"] = r.hex().into();
    for e in d["interfaces"].as_array_mut().unwrap() {
        if e["iface"] == "ledger.v1" {
            e["standby"] = true.into();
        }
    }
    let x_key = zenkey::model::grammar::instance_key(&ledger_addr(), &x_id)
        .unwrap()
        .to_string();
    let body = serde_json::to_vec(&d).unwrap();
    let x_descriptor = x
        .declare_queryable(x_key.clone())
        .callback(move |q| {
            let body = body.clone();
            let key = q.key_expr().clone().into_owned();
            tokio::spawn(async move {
                let _ = q
                    .reply(key, body)
                    .encoding(zenoh::bytes::Encoding::APPLICATION_JSON)
                    .await;
            });
        })
        .await
        .unwrap();
    let x_token = x.liveliness().declare_token(x_key).await.unwrap();
    let mut oc = Opts::ledger("C", 0xc);
    oc.first_run = Some(r);
    let c = candidate(&ep, oc).await.unwrap();
    // Expected 3: C's first campaign reads X's descriptor, a lower id's,
    // and finds its own run: it draws a new run and puts its descriptor
    // stating it before it serves; it stands by, X a lower candidate.
    eventually("C stands by on X, a lower candidate", || async {
        c.why() == Some(Why::LowerCandidate(x_id.clone()))
    })
    .await;
    let drawn = c.run();
    assert_ne!(drawn, r, "C drew a new run");
    eventually(
        "C's descriptor stating its new run, marked, at S",
        || async {
            s.descriptor_puts(&c.id)
                .iter()
                .any(|(_, m, run)| *m == Some(true) && *run == Some(drawn))
        },
    )
    .await;
    let c_puts = s.descriptor_puts(&c.id);
    assert_eq!(c_puts[0].2, Some(r), "its claim stated R: {c_puts:?}");
    assert!(
        c_puts.iter().all(|(_, m, _)| *m == Some(true)),
        "all before it serves: {c_puts:?}"
    );

    // 4. X withdraws its instance token. Expected 4: C takes over under its
    //    new run, never under R.
    x_token.undeclare().await.unwrap();
    drop(x_descriptor);
    c.until(Role::Serving).await;
    eventually("C's takeover put at S, under its new run", || async {
        s.data_under(drawn).iter().any(|(_, _, p)| p == "C")
    })
    .await;
    assert!(s.data_under(r).is_empty(), "nothing under R");
    assert_eq!(c.run(), drawn);
    c.close().await;
}

// --- §15 -----------------------------------------------------------------------------

/// K1: a follower built for `redundancy.v1` alone (v1 §2.10), bound to
/// `lab/ledger`: every instance it hears hold the interface's token read as
/// v1's, by its descriptor's `meta.zid`, and `balance`'s samples accepted
/// only under the lowest holder's.
struct K1 {
    values: Arc<Mutex<Vec<String>>>,
    discarded: Arc<Mutex<Vec<(String, Discard)>>>,
    sessions: Vec<zenoh::Session>,
    _subs: Vec<zenoh::pubsub::Subscriber<()>>,
}

async fn v1_follower(ep: &str) -> K1 {
    let watch = client(ep).await;
    let reader = client(ep).await;
    let fence: Arc<Mutex<model::Fence>> = Arc::default();
    let rt = tokio::runtime::Handle::current();
    let (f, r) = (Arc::clone(&fence), reader.clone());
    let watch_sub =
        presence::declare_subscriber(&watch, format!("zk2/{LEDGER}/@zk/alive/ledger.v1/**"))
            .history(true)
            .callback(move |x: Sample| {
                let Ok(ZkKey::Alive { addr, instance, .. }) = parse(x.key_expr().as_str()) else {
                    return;
                };
                if x.kind() == SampleKind::Delete {
                    f.lock()
                        .unwrap()
                        .hear(model::Holding::Withdrawn(addr, instance), false);
                    return;
                }
                f.lock().unwrap().hear(
                    model::Holding::Declared(addr.clone(), instance.clone()),
                    false,
                );
                let (f, r) = (Arc::clone(&f), r.clone());
                rt.spawn(async move {
                    if let Ok(Found::Descriptor(d, _)) =
                        presence::descriptor(&r, &addr, &instance, T).await
                        && let Some(zid) = d
                            .meta
                            .get("zid")
                            .and_then(|z| z.as_str())
                            .and_then(zenkey::timing::zid_value)
                    {
                        f.lock().unwrap().described(addr, instance, zid);
                    }
                });
            })
            .await
            .unwrap();
    let _ = presence::settle(&watch, &format!("zk2/{LEDGER}/@zk/**"), T).await;
    let values: Arc<Mutex<Vec<String>>> = Arc::default();
    let discarded: Arc<Mutex<Vec<(String, Discard)>>> = Arc::default();
    let (f, v, d) = (
        Arc::clone(&fence),
        Arc::clone(&values),
        Arc::clone(&discarded),
    );
    let data = reader
        .declare_subscriber(BALANCE)
        .callback(move |x: Sample| {
            let record = String::from_utf8_lossy(&common::record(&x)).into_owned();
            let sample = model::Sample {
                stamp: common::inband(&x),
                timestamp_id: x.timestamp().and_then(zenkey::timing::stamp_id),
            };
            match f.lock().unwrap().judge(&ledger_addr(), &sample, true) {
                model::Fenced::Accepted => v.lock().unwrap().push(record),
                model::Fenced::Discarded(why) => d.lock().unwrap().push((record, why)),
                model::Fenced::Pending => unreachable!("decided at once"),
            }
        })
        .await
        .unwrap();
    K1 {
        values,
        discarded,
        sessions: vec![watch, reader],
        _subs: vec![watch_sub, data],
    }
}

/// §15 Both majors side by side (v2 §2.17).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn s15_both_majors_side_by_side() {
    let (r1, ep) = router(None).await;
    let s = spy(&ep, LEDGER, &ledger()).await;
    let link_a = common::link_to(&ep).await;
    let k = follower(&ep, "ledger-run.v1", Duration::ZERO).await;
    let k1 = v1_follower(&ep).await;

    // 1. A (ledger.v1, redundancy.v1, its sessions stamping) starts, then
    //    B (ledger-run.v1, redundancy.v2). A puts a1.
    let a = candidate(link_a.endpoint(), Opts::ledger("A", 0xa).v1())
        .await
        .unwrap();
    a.until(Role::Serving).await;
    let b = candidate(&ep, Opts::ledger("B", 0xb)).await.unwrap();
    // Expected 1: A serves and B stands by, one election; a tool reads the
    // address governed, claim, never revisions_disagree (v2 §2.2).
    eventually("B stands by on A", || async {
        b.why() == Some(Why::Incumbent(a.id.clone()))
    })
    .await;
    assert_eq!(
        model::governance([
            model::Revision::of(&common::contract("ledger.v1")),
            model::Revision::of(&common::contract("ledger-run.v1")),
        ]),
        model::Governance::Governed(model::Election::Claim)
    );
    eventually("K follows A", || async {
        k.incumbent() == Some(a.id.clone())
    })
    .await;
    a.state.put("a1").await.unwrap();
    // A's put carries A's Zenoh timestamp, its id A's meta.zid: K follows A
    // by it and accepts a1, and so does K1.
    eventually("S heard a1", || async {
        s.data()
            .iter()
            .any(|(_, key, p, ..)| key == BALANCE && p == "a1")
    })
    .await;
    let (.., stamp, _) = s
        .data()
        .into_iter()
        .find(|(_, key, p, ..)| key == BALANCE && p == "a1")
        .unwrap();
    assert_eq!(stamp.map(|t| t.get_id().to_string()), Some(a.zid()));
    eventually("K holds a1", || async { k.holds("a1") }).await;
    eventually("K1 holds a1", || async {
        k1.values.lock().unwrap().last().is_some_and(|v| v == "a1")
    })
    .await;

    // 2. The test kills A's process. B takes over, and puts b1.
    link_a.cut();
    a.crash();
    b.until(Role::Serving).await;
    b.state.put("b1").await.unwrap();
    // Expected 2: B's puts carry no Zenoh timestamp of B's; K follows B by
    // run from B's token on, seeds again, and holds b1.
    eventually("K follows B, and holds b1", || async {
        k.incumbent() == Some(b.id.clone()) && k.holds("b1")
    })
    .await;
    assert!(
        k.seeds().iter().any(|(i, _)| *i == Some(b.id.clone())),
        "K seeded again under B"
    );
    eventually("S heard B's two puts", || async {
        s.data_under(b.run()).len() >= 2
    })
    .await;
    for (_, key, p, stamp, _) in s
        .data()
        .into_iter()
        .filter(|(.., i)| i.is_some_and(|i| i.run == b.run()))
    {
        assert_eq!(
            stamp.map(|t| t.get_id().to_string()),
            Some(r1.zid().to_string()),
            "{key} {p}: R1's stamp, never B's"
        );
    }
    // K1 finds no stamp of B's meta.zid on any of B's samples: it discards
    // each, and holds a1, safe and inert.
    eventually("K1 discarded b1", || async {
        k1.discarded.lock().unwrap().iter().any(|(v, _)| v == "b1")
    })
    .await;
    assert_eq!(
        k1.values.lock().unwrap().last().map(String::as_str),
        Some("a1")
    );
    assert!(
        k1.values
            .lock()
            .unwrap()
            .iter()
            .all(|v| v != "b1" && v != "B"),
        "no sample of B's"
    );

    // 3. A starts again, a new instance, its id lower than B's. Expected 3:
    //    it stands by on B's token; B keeps serving, K's value stays B's.
    let a2 = candidate(&ep, Opts::ledger("A", 0x9).v1()).await.unwrap();
    eventually("the new A stands by on B", || async {
        a2.why() == Some(Why::Incumbent(b.id.clone()))
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(b.role(), Role::Serving);
    assert_eq!(k.incumbent(), Some(b.id.clone()));
    assert!(k.holds("b1"));
    a2.close().await;
    b.close().await;
    k.close().await;
    for s in k1.sessions {
        s.close().await.unwrap();
    }
}

// ─── v2 §2.10, "A heartbeat" (#987's N3) ─────────────────────────────────

const BEACON: &str = "lab/beacon";
const STATUS: &str = "zk2/lab/beacon/beacon.v2/state/status";

/// One holder of `beacon-governed-run.v2` at `lab/beacon`, built by hand on
/// a session of its own: its instance token, its interface token, and a
/// descriptor stating the run `run` holds, served on a GET (core §3.3).
struct Holder {
    session: zenoh::Session,
    run: Arc<Mutex<Run>>,
    _tokens: Vec<zenoh::liveliness::LivelinessToken>,
    _descriptor: zenoh::query::Queryable<()>,
}

async fn holder(ep: &str, instance: u64, run: Run) -> Holder {
    use zenoh::Wait;
    let session = client(ep).await;
    let c = common::contract("beacon-governed-run.v2");
    let fp = zenkey::model::canonical::Fingerprint::of(&c);
    let id = id(instance);
    let run = Arc::new(Mutex::new(run));
    let r = Arc::clone(&run);
    let (fps, ids) = (fp.to_string(), id.to_string());
    let key = format!("zk2/{BEACON}/@zk/instance/{id}");
    let on = key.clone();
    let descriptor = session
        .declare_queryable(key.clone())
        .callback(move |q| {
            let body = serde_json::to_vec(&serde_json::json!({
                "format": "zk2-descriptor/0.1",
                "service": BEACON,
                "instance": ids,
                "run": r.lock().unwrap().hex(),
                "interfaces": [{"iface": "beacon.v2", "contract": fps, "minor": 2}],
                "profiles": ["freshness.v2", "redundancy.v2"],
            }))
            .unwrap();
            // On its own key, as an owner answers a read of
            // `@zk/instance/*` (core §3.3).
            let _ = q
                .reply(on.as_str(), body)
                .encoding(zenoh::bytes::Encoding::APPLICATION_JSON)
                .wait();
        })
        .await
        .unwrap();
    let tokens = vec![
        session.liveliness().declare_token(key).await.unwrap(),
        session
            .liveliness()
            .declare_token(format!(
                "zk2/{BEACON}/@zk/alive/beacon.v2/{id}/{}",
                fp.hex().fp16()
            ))
            .await
            .unwrap(),
    ];
    Holder {
        session,
        run,
        _tokens: tokens,
        _descriptor: descriptor,
    }
}

impl Holder {
    /// Its put of `status`: `record` under its run at `seq`.
    async fn put(&self, seq: u32, record: &str) -> Stamp {
        let stamp = Stamp {
            run: *self.run.lock().unwrap(),
            seq: zenkey::model::wire::Seq::new(seq).unwrap(),
        };
        let bytes = zenkey::model::wire::Envelope::mutation(
            zenkey::model::wire::Tag::NONE,
            stamp,
            record.as_bytes(),
        )
        .write();
        self.session
            .put(STATUS, bytes)
            .encoding(zenoh::bytes::Encoding::ZENOH_BYTES)
            .await
            .unwrap();
        stamp
    }

    /// Its heartbeat covering `beacon.v2` (core §7.5's form 6), under its
    /// run at `seq`, its section's digest over `held`.
    async fn beat(&self, seq: u32, held: &[(&str, Stamp)]) {
        let run = *self.run.lock().unwrap();
        put_beat(&self.session, run, seq, held).await;
    }
}

/// A heartbeat covering `beacon.v2` at `lab/beacon`, under `run` at `seq`,
/// its one section's digest over `held`, vouched for.
async fn put_beat(s: &zenoh::Session, run: Run, seq: u32, held: &[(&str, Stamp)]) {
    use zenkey::model::heartbeat::{Digest, Heartbeat, Layout, Members, Section, Time};
    let layout = Layout::of(&common::contract("beacon-governed-run.v2"));
    let sections = layout
        .collections
        .iter()
        .map(|_| {
            Some(Section {
                digest: Digest::of(held.iter().copied()),
                members: Members::One { vouched: true },
            })
        })
        .collect();
    let body = Heartbeat {
        time: Time::Unknown,
        sections,
    }
    .envelope(
        &layout,
        Stamp {
            run,
            seq: zenkey::model::wire::Seq::new(seq).unwrap(),
        },
    )
    .unwrap();
    let iface: IfaceId = "beacon.v2".parse().unwrap();
    s.put(zenkey::model::heartbeat::key("lab", "beacon", &iface), body)
        .encoding(zenoh::bytes::Encoding::ZENOH_BYTES)
        .await
        .unwrap();
}

/// v2 §2.10, "A heartbeat" (#987's N3): a follower reads the heartbeats
/// that cover its interface under its incumbent's runs alone.
///
/// A (id 0xa) and B (id 0xb) both hold `beacon-governed-run.v2`'s token at
/// `lab/beacon`, a split: K follows A, the lowest. A puts `status` and a
/// heartbeat whose digest is over it, which confirms it. B puts two
/// heartbeats whose digest is over nothing: K discards both,
/// `not_incumbent`, and its view never reads them, so it is neither due a
/// read nor counts a lost heartbeat. P, a consumer that does not follow,
/// reads all three, and its view is due a read after B's two. X puts one
/// under a run no descriptor states: K holds it, and discards it
/// `unattributed` once its read of the descriptors ends. A takes a new run
/// and puts a heartbeat under it before any descriptor put: K holds it,
/// reads A's descriptor stating the run, and its view reads it then.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_follower_reads_its_incumbents_heartbeats_alone() {
    use zenkey::consumer::{Consumer, Delivery, HeartbeatRead};
    let (_r1, ep) = router(None).await;
    let (ra, rb, rx, ra2) = (
        Run::U32(0x0a0a_0a0a),
        Run::U32(0x0b0b_0b0b),
        Run::U32(0x0c0c_0c0c),
        Run::U32(0x0a0a_0a0b),
    );
    let a = holder(&ep, 0xa, ra).await;
    let b = holder(&ep, 0xb, rb).await;
    let x = client(&ep).await;
    let beacon: Addr = BEACON.parse().unwrap();

    // K, the follower, and P, a consumer that does not follow.
    let (ks, kw, ps) = (client(&ep).await, client(&ep).await, client(&ep).await);
    let contract = Arc::new(common::contract("beacon-governed-run.v2"));
    let k = Consumer::for_tool(&ks, Arc::clone(&contract), &[BEACON], &BTreeMap::new())
        .unwrap()
        .with_watch(kw.clone());
    let told: Told = Arc::default();
    let t = Arc::clone(&told);
    let following = k
        .follow("state/status", move |e| {
            t.lock().unwrap().push((Instant::now(), e))
        })
        .await
        .unwrap();
    let p = Consumer::for_tool(&ps, contract, &[BEACON], &BTreeMap::new()).unwrap();
    let p_reads: Arc<Mutex<Vec<HeartbeatRead>>> = Arc::default();
    let pr = Arc::clone(&p_reads);
    let p_sub = p
        .subscribe_heartbeats(
            "state/status",
            |_: Delivery| {},
            move |h| pr.lock().unwrap().push(h),
        )
        .await
        .unwrap();
    eventually("K follows A, the lowest holder", || async {
        following.incumbent(&beacon) == Some(id(0xa))
    })
    .await;
    let beats = |why: Option<Discard>| -> Vec<Run> {
        told.lock()
            .unwrap()
            .iter()
            .filter_map(|(_, e)| match (e, why) {
                (FollowEvent::Heartbeat { stamp, .. }, None) => Some(stamp.run),
                (FollowEvent::HeartbeatDiscarded { stamp, why: w, .. }, Some(want))
                    if *w == want =>
                {
                    Some(stamp.run)
                }
                _ => None,
            })
            .collect()
    };

    // A's status, then its heartbeat over it: K's view confirms it.
    let up = a.put(0, "up").await;
    eventually("K holds A's status", || async {
        following
            .value(STATUS)
            .is_some_and(|d| d.inband == Some(up))
    })
    .await;
    a.beat(0, &[(STATUS, up)]).await;
    eventually("A's heartbeat is read", || async { beats(None) == [ra] }).await;
    let sub = following.subscription();
    assert!(sub.confirmed(STATUS).is_some(), "A's heartbeat confirms");
    assert_eq!(sub.heartbeats(), 1);

    // B's two, over nothing: discarded, never read.
    for seq in [0, 1] {
        b.beat(seq, &[]).await;
    }
    eventually("K discards B's two", || async {
        beats(Some(Discard::NotIncumbent)) == [rb, rb]
    })
    .await;
    eventually("P read all three", || async {
        p_reads.lock().unwrap().len() == 3
    })
    .await;
    assert_eq!(sub.heartbeats(), 1, "K's view read A's alone");
    assert!(
        !sub.reread_due(&beacon) && sub.rereads() == 0,
        "no divergence of K's: B's is no heartbeat of its incumbent's"
    );
    assert!(
        p_sub.reread_due(&beacon) || p_sub.rereads() > 0,
        "P's view, which reads every run, is due a read after B's two"
    );

    // X's, under a run no descriptor states: held, then discarded.
    put_beat(&x, rx, 0, &[(STATUS, up)]).await;
    eventually("K discards X's, unattributed", || async {
        beats(Some(Discard::Unattributed)) == [rx]
    })
    .await;
    assert!(
        told.lock().unwrap().iter().any(|(_, e)| matches!(
            e,
            FollowEvent::HeartbeatHeld { stamp, .. } if stamp.run == rx
        )),
        "held for one read first"
    );
    assert_eq!(sub.heartbeats(), 1);

    // A's new run, stated by its descriptor's GET alone: K holds the
    // heartbeat, its read finds the run A's, and its view reads it.
    *a.run.lock().unwrap() = ra2;
    a.beat(0, &[(STATUS, up)]).await;
    eventually("A's heartbeat under its new run is read", || async {
        beats(None) == [ra, ra2]
    })
    .await;
    assert_eq!(sub.heartbeats(), 2);
    let counts = following.counts();
    assert_eq!(counts.heartbeats, 2);
    assert_eq!(counts.heartbeats_held, 2, "X's, and A's under its new run");
    assert_eq!(counts.heartbeats_discarded.get("not_incumbent"), Some(&2));
    assert_eq!(counts.heartbeats_discarded.get("unattributed"), Some(&1));
    assert!(counts.reads >= 2, "a read for each held heartbeat");

    drop((following, p_sub));
    for s in [a.session.clone(), b.session.clone(), x, ks, kw, ps] {
        let _ = s.close().await;
    }
}

// ─── observer §3.6: a read never undoes a token heard since it was asked ─

/// Each incumbent K told, in order.
fn incumbents(told: &Told) -> Vec<Option<InstanceId>> {
    told.lock()
        .unwrap()
        .iter()
        .filter_map(|(_, e)| match e {
            FollowEvent::Incumbent { incumbent, .. } => Some(incumbent.clone()),
            _ => None,
        })
        .collect()
}

/// Observer §3.6 (0.38; round 128's F-629): a read again never undoes a
/// token heard since it was asked.
///
/// A (id 0xa) holds `beacon-governed-run.v2`'s token at `lab/beacon`, and K
/// follows it. A's tokens go: K reads again a burst later, and that read's
/// answer is applied `HOLD` after it came back
/// (`FollowStartup::reread_hold`), as an answer the watch's news overtook.
/// B (id 0xb) declares its tokens while the read is held: K's watch hears
/// them, and K follows B. The read, complete and without B, is applied
/// after: K keeps B, and accepts B's put. Before the rule K read B gone,
/// and discarded every later sample of B's `no_incumbent`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_read_again_never_undoes_a_token_heard_since_it_was_asked() {
    use zenkey::consumer::Consumer;
    const HOLD: Duration = Duration::from_millis(1500);
    let (_r1, ep) = router(None).await;
    let (ra, rb) = (Run::U32(0x0a0a_0a0a), Run::U32(0x0b0b_0b0b));
    let a = holder(&ep, 0xa, ra).await;
    let beacon: Addr = BEACON.parse().unwrap();
    let (ks, kw) = (client(&ep).await, client(&ep).await);
    let contract = Arc::new(common::contract("beacon-governed-run.v2"));
    let k = Consumer::for_tool(&ks, contract, &[BEACON], &BTreeMap::new())
        .unwrap()
        .with_watch(kw.clone());
    let told: Told = Arc::default();
    let t = Arc::clone(&told);
    let following = k
        .follow_with(
            "state/status",
            FollowStartup {
                reread_hold: HOLD,
                ..FollowStartup::default()
            },
            move |e| t.lock().unwrap().push((Instant::now(), e)),
        )
        .await
        .unwrap();
    eventually("K follows A", || async {
        following.incumbent(&beacon) == Some(id(0xa))
    })
    .await;

    // A's tokens go. K sees no incumbent, and asks its read again a burst
    // later: answered without A or B, then held.
    a.session.close().await.unwrap();
    eventually("K sees no incumbent", || async {
        following.incumbent(&beacon).is_none()
    })
    .await;
    let gone = Instant::now();
    tokio::time::sleep(zenkey::follow::BURST + Duration::from_millis(300)).await;
    let b = holder(&ep, 0xb, rb).await;
    eventually("K follows B", || async {
        following.incumbent(&beacon) == Some(id(0xb))
    })
    .await;
    assert!(
        gone.elapsed() < zenkey::follow::BURST + HOLD,
        "the test's premise: K heard B while its read again was held"
    );

    // The held read is applied: complete, and without B.
    tokio::time::sleep_until((gone + zenkey::follow::BURST + HOLD + T).into()).await;
    assert_eq!(
        following.incumbent(&beacon),
        Some(id(0xb)),
        "the read establishes nothing of B: {:?}",
        incumbents(&told)
    );
    assert_eq!(
        incumbents(&told),
        [Some(id(0xa)), None, Some(id(0xb))],
        "never B gone"
    );
    let up = b.put(0, "up").await;
    eventually("K accepts B's put", || async {
        following
            .value(STATUS)
            .is_some_and(|d| d.inband == Some(up))
    })
    .await;
    assert!(
        following.counts().discarded.is_empty(),
        "{:?}",
        following.counts()
    );

    drop(following);
    for s in [b.session.clone(), ks, kw] {
        let _ = s.close().await;
    }
}

/// `scenarios-v2.md` §13 step 2's read again (observer §3.6, 0.38; round
/// 128's F-629), over `beacon-governed-run.v2` as round 128 measured it: A
/// serves, B stands by, K starts after B's claim, and A crashes, its links
/// cut. K's read again after A's withdrawal may come back without B's
/// token, which its watch heard after the read was asked: K keeps B as its
/// incumbent, never reads B gone, and accepts each later heartbeat of
/// B's. Over `ZK2_S13_TAKEOVERS` takeovers (the text's 39; 3 in the suite).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn s13_k_never_reads_b_gone_over_the_takeovers() {
    use zenkey::consumer::Consumer;
    let takeovers: u32 = std::env::var("ZK2_S13_TAKEOVERS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(3);
    let beacon: Addr = BEACON.parse().unwrap();
    for run in 0..takeovers {
        let (_r1, ep) = router(None).await;
        let lo = rand_id();
        let (a_id, b_id) = (lo, lo + 1);
        let link_a = common::link_to(&ep).await;
        let a = candidate(link_a.endpoint(), Opts::beacon("A", a_id))
            .await
            .unwrap();
        a.until(Role::Serving).await;
        let b = candidate(&ep, Opts::beacon("B", b_id)).await.unwrap();
        eventually("B stands by on A", || async {
            b.why() == Some(Why::Incumbent(a.id.clone()))
        })
        .await;
        // K, started after B's claim.
        let (ks, kw) = (client(&ep).await, client(&ep).await);
        let contract = Arc::new(common::contract("beacon-governed-run.v2"));
        let k = Consumer::for_tool(&ks, contract, &[BEACON], &BTreeMap::new())
            .unwrap()
            .with_watch(kw.clone());
        let told: Told = Arc::default();
        let t = Arc::clone(&told);
        let following = k
            .follow("state/status", move |e| {
                t.lock().unwrap().push((Instant::now(), e))
            })
            .await
            .unwrap();
        eventually("K follows A", || async {
            following.incumbent(&beacon) == Some(a.id.clone())
        })
        .await;

        // A crashes; B takes over.
        link_a.cut();
        a.crash();
        b.until(Role::Serving).await;
        let b_run = b.run();
        eventually("K follows B", || async {
            following.incumbent(&beacon) == Some(b.id.clone())
        })
        .await;
        // What K told once it followed B, in decision order: past its read
        // again, and two of B's heartbeats.
        let since_b = || -> Vec<FollowEvent> {
            let told = told.lock().unwrap();
            let at = told.iter().position(|(_, e)| {
                matches!(e, FollowEvent::Incumbent { incumbent: Some(i), .. } if *i == b.id)
            });
            at.map_or_else(Vec::new, |at| {
                told[at + 1..].iter().map(|(_, e)| e.clone()).collect()
            })
        };
        let beats = |events: &[FollowEvent]| {
            events
                .iter()
                .filter(|e| matches!(e, FollowEvent::Heartbeat { stamp, .. } if stamp.run == b_run))
                .count()
        };
        eventually("K accepts B's heartbeats", || async {
            beats(&since_b()) >= 2
        })
        .await;
        assert_eq!(
            following.incumbent(&beacon),
            Some(b.id.clone()),
            "run {run}: K reads B gone: {:?}",
            incumbents(&told)
        );
        let late: Vec<String> = since_b()
            .iter()
            .filter_map(|e| match e {
                FollowEvent::Incumbent { incumbent, .. } => {
                    Some(format!("incumbent {incumbent:?}"))
                }
                FollowEvent::HeartbeatDiscarded { why, .. } => Some(format!("discarded {why}")),
                _ => None,
            })
            .collect();
        assert!(
            late.is_empty(),
            "run {run}: once K follows B, never B gone, and none of B's heartbeats \
             discarded: {late:?}"
        );

        drop(following);
        b.close().await;
        for s in [ks, kw] {
            let _ = s.close().await;
        }
    }
}
