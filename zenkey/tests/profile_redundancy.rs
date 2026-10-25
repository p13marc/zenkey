//! `spec/profiles/redundancy/scenarios.md` §1–§11 (`redundancy.v1`, #841,
//! RB), one test per section, named after it. §12 is a tool's (#841, RC).
//!
//! Common setup, the text's: R1 is a router; where a section cuts a link,
//! R2 is a second router linked to R1 through a [`common::Link`]. The
//! instances A, B, C of `lab/ledger` implement `ledger.v1` (`claim`), V1,
//! V2, V3 of `lab/valve` `valve.v1` (`external`), each through
//! [`ServiceBuilder::redundancy`] on three sessions of its own: the claim's
//! (its HLC on, §2.10), a watch and a reader (§2.5 step 1, F-204), clients
//! of the router its section names. Their ids are the test's
//! ([`ServiceBuilder::set_instance_id`]), A's lowest. Each answers
//! `append` with its name and counts its executions, and puts `balance`
//! with its name when it takes over ([`Takeover`]). The retry interval and
//! the grace are 2 s.
//!
//! S is a tool session following `zk2/lab/*/@zk/**` with a liveliness
//! subscriber and subscribing to the interfaces' keys and descriptors,
//! recording each delivery's arrival, key, kind, payload and stamp. "Before
//! the token" is observed by a GET of the instance's descriptor made the
//! moment S hears each token (core `presence.md` §1; text 0.2, F-202), and
//! S's arrival order of a put and a token is never judged. G reads presence
//! on a session that never held a liveliness subscriber (core §8.1, 0.38).
//!
//! **A crash.** An in-process instance cannot be killed with SIGKILL (the
//! owner example's test does that, `tests/owner_example.rs`). Here a crash
//! is the instance dropped without its close, its links first made silent
//! (its [`common::Link`] stalled: a lost host, seen at the lease, set short
//! on the instance's sessions), or cut (a kernel that closes the links,
//! seen at once): §2.8, text 0.2, F-201.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use common::{SETTLE, T, client, eventually, router};
use zenkey::call::BoxFuture;
use zenkey::model::grammar::{Addr, IfaceId, InstanceId, ZkKey, parse};
use zenkey::model::redundancy as model;
use zenkey::model::template::Bindings;
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

fn id(n: u64) -> InstanceId {
    InstanceId::from_u64(n)
}

/// A session of an instance's: a client of `ep`, its HLC on (§2.10),
/// retrying fast after a cut; its transport's lease `lease` when given,
/// which its router reads to expire it (core Appendix B).
async fn session(ep: &str, lease: Option<Duration>) -> zenoh::Session {
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
}

impl Opts {
    fn ledger(name: &'static str, id: u64) -> Self {
        Self {
            name,
            id,
            address: LEDGER,
            imp: common::imp("ledger.v1"),
            lease: None,
            watch: true,
            lock: None,
            refuse_without_lock: false,
            tokenless: false,
        }
    }

    fn valve(name: &'static str, id: u64, lock: Arc<dyn Lock>) -> Self {
        Self {
            address: VALVE,
            imp: common::imp("valve.v1"),
            lock: Some(lock),
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
    name: &'static str,
}

impl Takeover for PutName {
    fn taking_over<'a>(&'a self, _iface: &'a IfaceId) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.state.put(self.name).await.expect("a takeover's put");
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
    let claim = session(ep, o.lease).await;
    let watch = session(ep, o.lease).await;
    let reader = session(ep, o.lease).await;
    let iface = o.imp.iface().clone();
    let mut cfg = ServiceConfig::new(o.address.parse::<Addr>().unwrap());
    if o.tokenless {
        cfg = cfg.tokenless(iface.clone());
    }
    let mut b = ServiceBuilder::new(&claim, cfg);
    b.set_instance_id(id(o.id));
    b.implement(o.imp.clone())?;
    let none = Bindings::new();
    let (state_res, op_res) = if iface == ledger() {
        ("state/balance", "@op/append")
    } else {
        ("state/position", "@op/set")
    };
    let state = Arc::new(b.declare_state_writer(&iface, state_res, &none).await?);
    let stream = if iface == ledger() {
        Some(b.declare_writer(&iface, "stream/entries", &none).await?)
    } else {
        None
    };
    let calls = Arc::new(AtomicU64::new(0));
    let (n, name) = (Arc::clone(&calls), o.name);
    let server = b
        .serve(&iface, op_res, Some(&none), move |call| {
            let n = Arc::clone(&n);
            async move {
                n.fetch_add(1, Ordering::SeqCst);
                call.reply(name)
                    .await
                    .map_err(|e| zenkey::OpError::internal(e.to_string()))
            }
        })
        .await?;
    let mut startup = RedundancyStartup::new(reader.clone())
        .with_retry(RETRY)
        .with_grace(GRACE)
        .with_takeover(Arc::new(PutName {
            state: Arc::clone(&state),
            name: o.name,
        }));
    if o.watch {
        startup = startup.with_watch(watch.clone());
    }
    if let Some(l) = &o.lock {
        startup = startup.with_lock(Arc::clone(l));
    }
    startup.refuse_without_lock = o.refuse_without_lock;
    let red = b.redundancy(startup)?;
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

impl Candidate {
    fn role(&self) -> Role {
        self.red.role(&self.iface)
    }

    fn why(&self) -> Option<Why> {
        self.red.standing(&self.iface).why
    }

    /// Its claim session's zid, which its stamps carry (§2.10).
    fn zid(&self) -> String {
        self.sessions[0].zid().to_string()
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
    /// unmarked, or nothing (F-202).
    AtToken { key: ZkKey, standby: Option<bool> },
    /// A descriptor put: the interface marked or not.
    Descriptor {
        instance: InstanceId,
        standby: Option<bool>,
    },
    /// A put or delete of one of the interface's keys.
    Data {
        at: Instant,
        key: String,
        delete: bool,
        payload: String,
        stamp: Option<zenoh::time::Timestamp>,
    },
}

type Log = Arc<Mutex<Vec<Rec>>>;

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
                    let found = presence::descriptor(&getter, &addr, &instance, T)
                        .await
                        .unwrap_or(Found::Nothing);
                    w.lock().unwrap().push(Rec::AtToken {
                        key,
                        standby: mark_of(&found, &iface),
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
                let standby = d.and_then(|d| {
                    d.interfaces
                        .iter()
                        .find(|e| e.iface == iface_c.to_string())
                        .map(|e| e.standby)
                });
                w.lock()
                    .unwrap()
                    .push(Rec::Descriptor { instance, standby });
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
                    delete: x.kind() == SampleKind::Delete,
                    payload: String::from_utf8_lossy(&x.payload().to_bytes()).into_owned(),
                    stamp: x.timestamp().copied(),
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
            if let Rec::AtToken { key, standby } = r {
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

    /// Whether `instance`'s token of `iface` reached S before the GET at
    /// its instance token answered: then that GET may read it unmarked
    /// (F-202).
    fn token_beat_the_get(&self, iface: &IfaceId, instance: &InstanceId) -> bool {
        let log = self.snapshot();
        let alive = log.iter().position(|r| {
            matches!(r, Rec::Token { key: ZkKey::Alive { iface: i, instance: x, .. }, put: true, .. }
                if i == iface && x == instance)
        });
        let answered = log.iter().position(|r| {
            matches!(r, Rec::AtToken { key: ZkKey::Instance { instance: x, .. }, .. } if x == instance)
        });
        matches!((alive, answered), (Some(a), Some(g)) if a < g)
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

    /// The interface's puts and deletes S received, stamped by `zid`.
    fn data_by(&self, zid: &str) -> Vec<(Instant, String, bool, String, SystemTime)> {
        self.snapshot()
            .into_iter()
            .filter_map(|r| match r {
                Rec::Data {
                    at,
                    key,
                    delete,
                    payload,
                    stamp: Some(ts),
                } if ts.get_id().to_string() == zid => {
                    Some((at, key, delete, payload, ts.get_time().to_system_time()))
                }
                _ => None,
            })
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
    let replies = s
        .get(key)
        .payload("x")
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
            Ok(s) => String::from_utf8_lossy(&s.payload().to_bytes()).into_owned(),
            Err(_) => "!".to_owned(),
        });
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
    let (_r1, ep) = router(None).await;
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
    // F-202: marked at its instance token, unless its interface token beat
    // that GET; unmarked at its interface token.
    assert!(
        at_instance == Some(true) || s.token_beat_the_get(&ledger(), &a.id),
        "A's descriptor marked at its instance token: {at_instance:?}"
    );
    assert_eq!(at_alive, Some(false), "A unmarked at its interface token");

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
    let a_zid = a.zid();

    // 1. A closes cleanly, at Q.
    let q = Instant::now();
    let q_wall = SystemTime::now();
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
    // A put and deleted nothing of ledger.v1's keys while it closed.
    let late: Vec<_> = s
        .data_by(&a_zid)
        .into_iter()
        .filter(|(_, _, _, _, stamp)| *stamp >= q_wall)
        .collect();
    assert!(late.is_empty(), "nothing of A's from Q on: {late:?}");
    b.close().await;
}

// --- §3 ------------------------------------------------------------------------------

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
    // again, and counted apart.
    let (mut run, mut apart) = (0u64, 0u64);
    while run < 20 {
        let lo: u64 = rand_id();
        let (a_id, b_id) = (lo, lo + 1 + rand_id() % 1000);
        let (a, b) = tokio::join!(
            candidate(&ep, Opts::ledger("A", a_id)),
            candidate(&ep, Opts::ledger("B", b_id)),
        );
        let (a, b) = (a.unwrap(), b.unwrap());
        eventually("S heard both instance tokens", || async {
            s.instance_at(&a.id).is_some() && s.instance_at(&b.id).is_some()
        })
        .await;
        let (ta, tb) = (s.instance_at(&a.id).unwrap(), s.instance_at(&b.id).unwrap());
        if ta.max(tb) - ta.min(tb) > Duration::from_millis(10) {
            apart += 1;
            assert!(apart < 200, "two hundred starts more than 10 ms apart");
            a.close().await;
            b.close().await;
            quiet(&s, &g).await;
            continue;
        }
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
        // instance token, unless its interface token beat that GET (F-202).
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
         it; {apart} start(s) more than 10 ms apart made again"
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
/// most (F-205).
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
                name: o.name,
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
    // F-204: its claim's own session would have read its claim.
    let own = presence::liveliness_read(&a.sessions[0], &format!("zk2/{LEDGER}/@zk/**"), T)
        .await
        .unwrap();
    assert!(
        own.complete && !own.keys.is_empty(),
        "the claim's session answers itself: {own:?}"
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
    let b_zid = b.zid();
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
    let (w, _) = *s2
        .alive_events(&ledger(), &b.id)
        .iter()
        .rev()
        .find(|(_, put)| !put)
        .unwrap();
    let w_wall = SystemTime::now() - w.elapsed();
    // Nothing of B's from its token's withdrawal on: every sample stamped
    // with its id dated before it.
    for spy in [&s, &s2] {
        for (_, key, _, _, stamp) in spy.data_by(&b_zid) {
            assert!(
                stamp <= w_wall + Duration::from_millis(50),
                "{key} put by B after its step-down"
            );
        }
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
    // 2. A fan-out's GET, target All: one reply, A's.
    assert_eq!(call(&x, LEDGER, &ledger(), "append", true).await, ["A"]);
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

// --- §9 ------------------------------------------------------------------------------

/// K: a consumer of `ledger.v1` bound to `lab/ledger` that follows the
/// incumbent (§2.10): a watch on the address's `@zk/**`, each holder's
/// `meta.zid` from its descriptor, and `balance`'s samples accepted only
/// under the lowest holder's.
struct Follower {
    holders: Arc<Mutex<BTreeMap<InstanceId, Option<String>>>>,
    /// The values it holds, in the order it took them.
    values: Arc<Mutex<Vec<String>>>,
    /// Every sample it discarded, by its stamp's id.
    discarded: Arc<Mutex<Vec<(String, String)>>>,
    sessions: Vec<zenoh::Session>,
    _subs: Vec<zenoh::pubsub::Subscriber<()>>,
}

impl Follower {
    async fn new(ep: &str) -> Self {
        let watch = client(ep).await;
        let reader = client(ep).await;
        let holders: Arc<Mutex<BTreeMap<InstanceId, Option<String>>>> = Arc::default();
        let rt = tokio::runtime::Handle::current();
        let (h, r) = (Arc::clone(&holders), reader.clone());
        let watch_sub =
            presence::declare_subscriber(&watch, format!("zk2/{LEDGER}/@zk/alive/ledger.v1/**"))
                .history(true)
                .callback(move |x: Sample| {
                    let Ok(ZkKey::Alive { instance, .. }) = parse(x.key_expr().as_str()) else {
                        return;
                    };
                    if x.kind() == SampleKind::Delete {
                        h.lock().unwrap().remove(&instance);
                        return;
                    }
                    h.lock().unwrap().insert(instance.clone(), None);
                    let (h, r) = (Arc::clone(&h), r.clone());
                    rt.spawn(async move {
                        let found =
                            presence::descriptor(&r, &LEDGER.parse().unwrap(), &instance, T)
                                .await
                                .unwrap_or(Found::Nothing);
                        if let Found::Descriptor(d, _) = found {
                            let zid = d
                                .meta
                                .get("zid")
                                .and_then(|z| z.as_str())
                                .map(str::to_owned);
                            if let Some(slot) = h.lock().unwrap().get_mut(&instance) {
                                *slot = zid;
                            }
                        }
                    });
                })
                .await
                .unwrap();
        let _ = presence::settle(&watch, &format!("zk2/{LEDGER}/@zk/**"), T).await;
        let values: Arc<Mutex<Vec<String>>> = Arc::default();
        let discarded: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
        let (h, v, d) = (
            Arc::clone(&holders),
            Arc::clone(&values),
            Arc::clone(&discarded),
        );
        let data = reader
            .declare_subscriber(format!("zk2/{LEDGER}/ledger.v1/state/balance"))
            .callback(move |x: Sample| {
                let payload = String::from_utf8_lossy(&x.payload().to_bytes()).into_owned();
                Self::take(&h, &v, &d, x.timestamp(), payload);
            })
            .await
            .unwrap();
        Self {
            holders,
            values,
            discarded,
            sessions: vec![watch, reader],
            _subs: vec![watch_sub, data],
        }
    }

    /// The incumbent it follows: the lowest holder, by its `meta.zid`.
    fn incumbent(h: &Mutex<BTreeMap<InstanceId, Option<String>>>) -> Option<String> {
        h.lock().unwrap().values().next().cloned().flatten()
    }

    fn take(
        h: &Mutex<BTreeMap<InstanceId, Option<String>>>,
        v: &Mutex<Vec<String>>,
        d: &Mutex<Vec<(String, String)>>,
        stamp: Option<&zenoh::time::Timestamp>,
        payload: String,
    ) {
        let by = stamp.map(|t| t.get_id().to_string());
        match (Self::incumbent(h), by) {
            (Some(i), Some(by)) if i == by => v.lock().unwrap().push(payload),
            (_, by) => d.lock().unwrap().push((by.unwrap_or_default(), payload)),
        }
    }

    /// Its seed (core §4.2): S4's GET, `All` + `Latest`, each reply taken
    /// as a sample.
    async fn seed(&self) -> Option<(String, String)> {
        let replies = self.sessions[1]
            .get(format!("zk2/{LEDGER}/ledger.v1/state/balance"))
            .target(QueryTarget::All)
            .consolidation(ConsolidationMode::Latest)
            .timeout(T)
            .await
            .unwrap();
        let mut got = None;
        while let Ok(r) = replies.recv_async().await {
            if let Ok(s) = r.result() {
                let payload = String::from_utf8_lossy(&s.payload().to_bytes()).into_owned();
                let by = s
                    .timestamp()
                    .map(|t| t.get_id().to_string())
                    .unwrap_or_default();
                got = Some((by, payload.clone()));
                Self::take(
                    &self.holders,
                    &self.values,
                    &self.discarded,
                    s.timestamp(),
                    payload,
                );
            }
        }
        got
    }
}

/// §9 A consumer following the incumbent during a split (§2.10; core S7).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn s9_a_consumer_following_the_incumbent_during_a_split() {
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
    let (a_zid, b_zid) = (a.zid(), b.zid());
    // P: a consumer by core S7 alone.
    let p_session = client(&ep1).await;
    let p_values: Arc<Mutex<Vec<String>>> = Arc::default();
    let order = Arc::new(Mutex::new(zenkey::state::ValueOrder::new()));
    let (pv, po) = (Arc::clone(&p_values), Arc::clone(&order));
    let _p = p_session
        .declare_subscriber(format!("zk2/{LEDGER}/ledger.v1/state/balance"))
        .callback(move |x: Sample| {
            if let Some(ts) = x.timestamp()
                && po.lock().unwrap().accept(x.key_expr().as_str(), ts)
            {
                pv.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&x.payload().to_bytes()).into_owned());
            }
        })
        .await
        .unwrap();
    let k = Follower::new(&ep1).await;

    // 1. The link healed, B's step-down held; A and B put balance five
    //    times each, alternately, 100 ms apart.
    b.red.hold_heal(true);
    link.heal();
    s.holders_are(&ledger(), &[&a.id, &b.id]).await;
    eventually("K follows A, the lowest holder", || async {
        Follower::incumbent(&k.holders) == Some(a_zid.clone())
    })
    .await;
    let mut k2 = None;
    for n in 1..=5 {
        a.state.put(format!("a{n}")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        b.state.put(format!("b{n}")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        if n == 3 {
            // 2. K restarts during step 1, and seeds.
            let next = Follower::new(&ep1).await;
            eventually("the new K follows A", || async {
                Follower::incumbent(&next.holders) == Some(a_zid.clone())
            })
            .await;
            let seeded = next.seed().await;
            if let Some((by, _)) = &seeded {
                if *by == b_zid {
                    assert!(next.values.lock().unwrap().is_empty(), "never B's seed");
                } else {
                    assert_eq!(*by, a_zid);
                }
            }
            k2 = Some(next);
        }
    }
    let want_p: Vec<String> = (1..=5)
        .flat_map(|n| [format!("a{n}"), format!("b{n}")])
        .collect();
    eventually("P took all ten, flapping", || async {
        *p_values.lock().unwrap() == want_p
    })
    .await;
    let want_k: Vec<String> = (1..=5).map(|n| format!("a{n}")).collect();
    eventually("K took A's five, in order", || async {
        *k.values.lock().unwrap() == want_k
    })
    .await;
    assert!(
        k.discarded
            .lock()
            .unwrap()
            .iter()
            .all(|(by, _)| *by == b_zid),
        "K discarded B's only"
    );
    assert_eq!(k.discarded.lock().unwrap().len(), 5, "every one of B's");
    let k2 = k2.unwrap();
    assert!(
        k2.values.lock().unwrap().iter().all(|v| v.starts_with('a')),
        "the restarted K holds A's values only: {:?}",
        k2.values.lock().unwrap()
    );

    // 3. B steps down; K's value stays A's.
    b.red.hold_heal(false);
    b.until(Role::Standby).await;
    s.holders_are(&ledger(), &[&a.id]).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        k.values.lock().unwrap().last().map(String::as_str),
        Some("a5")
    );
    a.close().await;
    b.close().await;
    for s in k.sessions.into_iter().chain(k2.sessions) {
        s.close().await.unwrap();
    }
}

// --- §10 -----------------------------------------------------------------------------

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
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/contracts/ledger.v1.toml");
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
    // Neither declared or put anything of lab/ledger within 5 s.
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(
        s.snapshot().len(),
        quiet,
        "nothing of lab/ledger: {:?}",
        &s.snapshot()[quiet..]
    );
}

// --- Beyond the sections: the runtime's poll, and a re-mint ------------------------

/// §2.7's poll (0.2), where no watch session is held: B reads the address
/// every 500 ms, sees A's token gone from a complete read, and takes over
/// as §2 expects, a period later at most.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn the_poll_without_a_watch_session() {
    let (_r1, ep) = router(None).await;
    let a = candidate(&ep, Opts::ledger("A", 0xa)).await.unwrap();
    a.until(Role::Serving).await;
    let mut o = Opts::ledger("B", 0xb);
    o.watch = false;
    let b = candidate(&ep, o).await.unwrap();
    eventually("B stands by on A", || async {
        b.why() == Some(Why::Incumbent(a.id.clone()))
    })
    .await;
    assert!(
        presence::liveliness_gets(&b.sessions[1]) == presence::LivelinessGets::default(),
        "no watch session in use"
    );
    let q = Instant::now();
    a.close().await;
    b.until(Role::Serving).await;
    let took = q.elapsed();
    assert!(
        took < zenkey::redundancy::POLL + Duration::from_secs(2) + JITTER,
        "within a poll period plus 2 s: {took:?}"
    );
    assert!(b.red.reads(&ledger()).polls >= 1);
    b.close().await;
}

/// §2.9's re-mint (0.2, F-209): the incumbent re-mints make-before-break;
/// its own new token is no appearance, so it never heals against itself,
/// and its standby hears no vacancy.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_re_mint_is_no_appearance() {
    let (_r1, ep) = router(None).await;
    let s = spy(&ep, LEDGER, &ledger()).await;
    let mut a = candidate(&ep, Opts::ledger("A", 0xa)).await.unwrap();
    a.until(Role::Serving).await;
    let b = candidate(&ep, Opts::ledger("B", 0xb)).await.unwrap();
    eventually("B stands by on A", || async {
        b.why() == Some(Why::Incumbent(a.id.clone()))
    })
    .await;
    let reads = a.red.reads(&ledger()).made;
    let next = a.svc.new_epoch().await.unwrap();
    s.holders_are(&ledger(), &[&next]).await;
    tokio::time::sleep(2 * GRACE + Duration::from_secs(1)).await;
    assert_eq!(a.role(), Role::Serving, "no step-down to itself");
    assert_eq!(
        a.red.reads(&ledger()).made,
        reads,
        "no heal read for its own token"
    );
    assert_eq!(b.role(), Role::Standby, "no vacancy");
    assert!(s.alive_events(&ledger(), &b.id).is_empty());
    assert_eq!(
        call(&client(&ep).await, LEDGER, &ledger(), "append", false).await,
        ["A"]
    );
    a.close().await;
    b.close().await;
}

// --- Beyond the sections: the step-down's write fence, and an act that fails ------

/// §2.9's step-down stops writes before step 1 (0.2): a write in flight
/// while B steps down is published before B withdraws its token, or is
/// refused, never after. B writes `balance` and `entries` as fast as it
/// can across the heal of §7's split.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_write_in_flight_during_a_step_down_lands_before_its_token_goes() {
    let Split {
        _r1,
        _r2,
        link,
        ep1: _ep1,
        s,
        s2,
        a,
        mut b,
    } = split().await;
    let b_zid = b.zid();
    let state = Arc::clone(&b.state);
    let stream = Arc::new(b.stream.take().expect("ledger.v1 has a stream"));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // Each write: when its call returned, and whether it was published.
    type Writes = Arc<Mutex<Vec<(Instant, Result<(), String>)>>>;
    let writes: Writes = Arc::default();
    let hammer = {
        let (stop, writes, state, stream) = (
            Arc::clone(&stop),
            Arc::clone(&writes),
            Arc::clone(&state),
            Arc::clone(&stream),
        );
        tokio::spawn(async move {
            let mut n = 0u64;
            while !stop.load(Ordering::SeqCst) {
                n += 1;
                let put = state.put(format!("b{n}")).await.map(|_| ());
                let sent = stream.put(format!("e{n}")).await;
                {
                    let mut w = writes.lock().unwrap();
                    for r in [put, sent] {
                        w.push((Instant::now(), r.map_err(|e| e.to_string())));
                        if let Err(e) = &w.last().unwrap().1 {
                            assert!(e.contains("stands by"), "only Error::Standby: {e}");
                        }
                    }
                }
                tokio::task::yield_now().await;
            }
        })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    link.heal();
    b.until(Role::Standby).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    stop.store(true, Ordering::SeqCst);
    hammer.await.unwrap();
    s2.holders_are(&ledger(), &[&a.id]).await;
    let (withdrawn, _) = *s2
        .alive_events(&ledger(), &b.id)
        .iter()
        .rev()
        .find(|(_, put)| !put)
        .expect("B's token withdrawn");
    let withdrawn_wall = SystemTime::now() - withdrawn.elapsed();
    let writes = writes.lock().unwrap().clone();
    let first_refused = writes
        .iter()
        .position(|(_, r)| r.is_err())
        .expect("writes refused once B stands by");
    assert!(first_refused > 0, "writes published while B served");
    assert!(
        writes[first_refused..].iter().all(|(_, r)| r.is_err()),
        "no write passes once one is refused"
    );
    for (at, _) in &writes[..first_refused] {
        assert!(*at < withdrawn, "a write published after B's token went");
    }
    // What the subscribers received of B's: all of it dated before.
    for spy in [&s, &s2] {
        for (_, key, _, _, stamp) in spy.data_by(&b_zid) {
            assert!(
                stamp <= withdrawn_wall + Duration::from_millis(50),
                "{key} stamped by B after its token went"
            );
        }
    }
    a.close().await;
    b.close().await;
}

/// An act that fails (text 0.2, "Decided here"): a takeover whose step
/// zenoh refuses (here a test's injected failure, at each of its three
/// steps) is undone in §2.9's order, and the instance stands by, its
/// standing saying why, and elects no more.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_takeover_that_fails_is_undone() {
    use zenkey::redundancy::Fail;
    for at in [Fail::Resources, Fail::Descriptor, Fail::Token] {
        let (_r1, ep) = router(None).await;
        let s = spy(&ep, LEDGER, &ledger()).await;
        let g = client(&ep).await;
        let a = candidate(&ep, Opts::ledger("A", 0xa)).await.unwrap();
        a.until(Role::Serving).await;
        let b = candidate(&ep, Opts::ledger("B", 0xb)).await.unwrap();
        eventually("B stands by on A", || async {
            b.why() == Some(Why::Incumbent(a.id.clone()))
        })
        .await;
        b.red.fail_next(&ledger(), at);
        a.close().await;
        eventually(&format!("{at:?}: B's takeover failed"), || async {
            b.red.standing(&ledger()).fault.is_some()
        })
        .await;
        let st = b.red.standing(&ledger());
        assert_eq!(st.role, Role::Standby, "{at:?}");
        assert!(
            st.fault.as_deref().unwrap().contains("takeover failed"),
            "{st:?}"
        );
        // Undone: no write passes, no queryable, no token, marked.
        assert!(
            matches!(b.state.put("x").await, Err(Error::Standby(_))),
            "{at:?}"
        );
        assert!(
            !b.servers[0].is_declared(),
            "{at:?}: the queryable undeclared"
        );
        let found = presence::descriptor(&g, &LEDGER.parse().unwrap(), &b.id, T)
            .await
            .unwrap();
        assert_eq!(
            mark_of(&found, &ledger()),
            Some(true),
            "{at:?}: marked again"
        );
        let made = b.red.reads(&ledger()).made;
        tokio::time::sleep(RETRY + Duration::from_secs(1)).await;
        assert!(
            s.alive_events(&ledger(), &b.id).is_empty(),
            "{at:?}: no token of B's"
        );
        assert_eq!(
            b.red.reads(&ledger()).made,
            made,
            "{at:?}: it elects no more"
        );
        assert_eq!(b.role(), Role::Standby);
        b.close().await;
    }
}

/// A step-down that fails is never swallowed: B's step-down at the heal of
/// §7's split, its descriptor's put refused (a test's injected failure),
/// still withdraws its token and undeclares its resources, and its
/// standing says the step-down failed; it elects no more.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_step_down_that_fails_is_reported() {
    use zenkey::redundancy::Fail;
    let Split {
        _r1,
        _r2,
        link,
        ep1: _ep1,
        s: _s,
        s2,
        a,
        b,
    } = split().await;
    b.red.fail_next(&ledger(), Fail::StepDown);
    link.heal();
    eventually("B's step-down failed", || async {
        b.red.standing(&ledger()).fault.is_some()
    })
    .await;
    let st = b.red.standing(&ledger());
    assert_eq!(st.role, Role::Standby);
    assert!(
        st.fault.as_deref().unwrap().contains("step-down failed"),
        "{st:?}"
    );
    s2.holders_are(&ledger(), &[&a.id]).await;
    assert!(!b.servers[0].is_declared(), "its queryable undeclared");
    assert!(matches!(b.state.put("x").await, Err(Error::Standby(_))));
    let made = b.red.reads(&ledger()).made;
    tokio::time::sleep(RETRY + Duration::from_secs(1)).await;
    assert_eq!(b.red.reads(&ledger()).made, made, "it elects no more");
    a.close().await;
    b.close().await;
}

/// §2.4's write fence has no way around it: a profile's owner the runtime
/// implements, whose writes go outside a governed interface's gate or
/// before any election, is refused on a governed interface (#841 RB's
/// review): a desired.v1 author of `ledger.v1`, and alarms.v1's owner on a
/// revision that lists `redundancy.v1`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_profiles_owner_is_refused_on_a_governed_interface() {
    let (_r1, ep) = router(None).await;
    let claim = session(&ep, None).await;
    let mut b = ServiceBuilder::new(&claim, ServiceConfig::new(LEDGER.parse::<Addr>().unwrap()));
    b.implement(common::imp("ledger.v1")).unwrap();
    let refused = b
        .desired_author(
            &ledger(),
            "state/balance",
            zenkey::desired::AuthorStartup::default(),
        )
        .await;
    assert!(
        matches!(&refused, Err(Error::Contract(why)) if why.contains("redundancy.v1 governs")),
        "{:?}",
        refused.err()
    );
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../spec/profiles/alarms/alarms.v1.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    let governed = text.replace(
        "uses    = [\"freshness.v1\"]",
        "uses    = [\"freshness.v1\", \"redundancy.v1\"]",
    );
    assert_ne!(governed, text);
    let l = zenkey::model::contract::load_str(&governed, path.parent().unwrap(), None);
    let imp = Implementation::new(l.contract.unwrap_or_else(|| panic!("{}", l.report)));
    let refused = b.alarms_with(imp, zenkey::alarms::Startup::default()).await;
    assert!(
        matches!(&refused, Err(Error::Contract(why)) if why.contains("redundancy.v1 governs")),
        "{:?}",
        refused.err()
    );
    claim.close().await.unwrap();
}
