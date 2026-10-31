//! `redundancy.v1` in the runtime: one instance serves a governed
//! interface, the others stand by to take over
//! (`spec/profiles/redundancy/v1.md`, #841).
//!
//! The candidate's half of the profile. The declaration, how ids compare,
//! the election over one read, the heal over two and the state machine
//! that drives a candidate are `zenkey-model`'s
//! ([`zenkey_model::redundancy`]); this module makes the claim, the reads
//! and the acts its [`Elector`](zenkey_model::redundancy::Elector) asks for, on the service's sessions, and
//! never decides anything itself.
//!
//! ```text
//! ServiceBuilder::redundancy(startup) ─▶ Redundancy: every governed interface the service
//!        │                               implements is elected; each of its resources is born
//!        │                               standing by, declared only while it serves
//!        ▼ start()
//!   claim (§2.5 step 1): the descriptor marks it standby, the instance token, the watch and
//!        │               its round trip, a round trip on the claim's session
//!        ▼
//!   Elector ◀── reads (the reader session) · the watch's tokens · timers · the lock
//!        │
//!        ├─ take over (§2.6):  resources ─▶ Takeover::taking_over ─▶ descriptor unmarked ─▶ token
//!        └─ step down (§2.9):  writes stop ─▶ tokens ─▶ resources ─▶ descriptor marked
//!   Redundancy::role / subscribe / until ─ Role::Standby │ Role::Serving, never a guess
//! ```
//!
//! | Rule | Here |
//! |---|---|
//! | §2.2 the declaration; an invalid one refused at start, nothing declared | [`crate::ServiceBuilder::start`] reads each implemented contract with [`zenkey_model::redundancy::declaration_of`]: invalid is [`Error::NotExposed`] naming the reasons, before step 1; governed without [`crate::ServiceBuilder::redundancy`] the same |
//! | §2.3 the claim set, the incumbent, ids compared | each read parsed into [`zenkey_model::redundancy::Read`]: instance tokens and the interface's tokens at any revision, the descriptors of the ids below its own |
//! | §2.4 a standby holds its instance token, exposes nothing of the interface, writes none of its keys | every publisher, state, operation and `unavailable` queryable and member token of a governed interface is declared only while it serves (`gate`); a write while it stands by is [`Error::Standby`], nothing published; the descriptor marks it `"standby": true` |
//! | §2.4 a call finds no standby | its operation queryables are declared only while it serves ([`crate::operation::OperationServer::is_declared`]) |
//! | §2.5 step 1 claim before reading; the read on a session holding neither a liveliness subscriber nor the claim, after a round trip on the claim's (0.2, F-204) | the descriptor and instance token at start, then the watch and its round trip, then one round trip on the service's session; every read on [`RedundancyStartup::reader`], refused at [`crate::ServiceBuilder::redundancy`] when it is the service's or the watch's |
//! | §2.5 steps 2–4 a live incumbent wins; a read not complete, or missing its own claim, or a lower descriptor unread, or a lower candidate, stands by; else take over | the [`Elector`](zenkey_model::redundancy::Elector) decides; the runtime arms the retry ([`RedundancyStartup::retry`], 2 s) on its stand-by |
//! | §2.6 take over in core §8.2's order | resources declared, [`Takeover::taking_over`] puts what it holds, the descriptor put unmarked, the interface token last |
//! | §2.7 follow the address on a watch session | a liveliness subscriber on `@zk/**` on [`RedundancyStartup::watch`], with one round trip before the first read; without one, a read every [`RedundancyStartup::poll`] (the poll, below) |
//! | §2.8 a withdrawal heard live is a read, never a vacancy | the watch's withdrawal, once no other holder is heard, is the [`Elector`](zenkey_model::redundancy::Elector)'s event; its read waits 0.1 s for the burst |
//! | §2.9 the heal: two reads a grace apart; the higher steps down in reverse order | [`RedundancyStartup::grace`], 2 s; writes stop, then the tokens, the resources, the descriptor put marked; the interface's state forgotten |
//! | §2.9 a candidate's own re-mint is no appearance (0.2, F-209) | [`crate::Service::new_epoch`] tells every elector before the new tokens, and re-mints a serving interface's token make-before-break |
//! | §2.10 every sample stamped with its own id; `meta.zid` | the service's session, its HLC on; `meta.zid` as every descriptor states it |
//! | §2.11 `external`: serve only with the lock, never by ids, step down when it is lost; refuse or stand by without it | [`Lock`], asked at the start, every [`RedundancyStartup::lock_period`], and on [`Redundancy::lock_changed`]; [`RedundancyStartup::refuse_without_lock`] |
//! | §2.14 a governed interface never tokenless | refused at start, `tokenless` |
//!
//! **Three sessions** (core §8.1, 0.37, 0.38; text 0.2, F-204). The claim
//! is the service's session's: its descriptor and instance token. The
//! watch, [`RedundancyStartup::watch`], holds the liveliness subscriber.
//! The reader, [`RedundancyStartup::reader`], makes every read: zenoh
//! can answer a session's liveliness GET with that session's own tokens
//! itself (observer §3.8; not always, since the router's final reply can
//! end the GET first, #992), so a read on the claim's session could find
//! its own claim even when its router refuses the read, and §2.5 step 3's
//! own-claim check would then miss a refusal. The reader may be shared
//! with the application's other reads, which meet core §8.1 the same way.
//!
//! **The poll** (no watch session): a read of the address on the reader
//! every [`RedundancyStartup::poll`] (500 ms by default), its holders
//! compared with the last poll's. A holder new to it is an appearance; a
//! complete poll with no other holder, after one with some, a withdrawal.
//! It misses what comes and goes within one period: a router's restart
//! that withdraws and restores the incumbent's token, a split shorter than
//! the period. And it costs one presence read of the address per period,
//! where the watch costs none.
//!
//! **A takeover's state** (§2.6). A new incumbent starts from nothing of
//! the interface's: a standby writes none of its keys, and a step-down
//! forgets what the instance had put (its owner GET answers, its
//! refreshers' values). What a takeover puts is the application's, in
//! [`Takeover::taking_over`], between the resources and the token, so that
//! a GET made at the token finds it.
//!
//! **An act that fails** (text 0.2, "Decided here"). zenoh refuses a
//! declaration, a put or an undeclaration only when its session is closing
//! or broken, so a takeover whose step fails is undone at once, every step
//! it made reversed in §2.9's order (writes stopped, any token withdrawn,
//! the resources undeclared and their state forgotten, the descriptor put
//! marked), and a step-down makes every step whatever one before it gave.
//! Either way the instance stands by and elects no more, and its
//! [`Standing::fault`] says why: retrying on a session that cannot declare
//! would flap, and the application restarts the service on a session that
//! can.
//!
//! **What a governed interface's handles do while it stands by.** A
//! [`crate::writer::Writer`], [`crate::state::StateWriter`] or
//! [`crate::writer::EventWriter`] refuses every write,
//! [`Error::Standby`], with nothing published; an
//! [`crate::operation::OperationServer`] holds no queryable; a member token
//! is refused. Each is declared at the takeover's step 1 and undeclared at
//! the step-down's step 2, and a handle made while the instance serves is
//! declared at once. A raw zenoh publisher or queryable is refused on a
//! governed interface ([`crate::ServiceBuilder::declare_publisher`],
//! [`crate::ServiceBuilder::declare_queryable`]): the runtime could not
//! undeclare it. Every stamped put and delete takes its gate's admission,
//! so none publishes around it, and a profile's owner whose writes go
//! outside the gate or before any election (a `desired.v1` author or
//! target's report, `alarms.v1`'s owner on a governed revision) is refused
//! on a governed interface.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zenkey_model::grammar::{Addr, Fp16, IfaceId, InstanceId, ZkKey, alive_key, instance_key};
use zenkey_model::redundancy::{self as model, Event, Listing, Read, Seen, Step};
use zenoh::liveliness::LivelinessToken;
use zenoh::sample::{Sample, SampleKind};

pub use zenkey_model::redundancy::{Election, Role, StepDownWhy, Why};

use crate::call::BoxFuture;
use crate::error::{Error, Result, zenoh};
use crate::gate::Gate;
use crate::presence;
use crate::service::{Served, Service};
use crate::state::Store;

/// How often a candidate without a watch session reads its address, by
/// default (the module doc's poll).
pub const POLL: Duration = Duration::from_millis(500);

/// The retry interval, by default (§2.5: "a few seconds"; ZenSight polls
/// every 2 s).
pub const RETRY: Duration = Duration::from_secs(2);

/// The grace between a heal's two reads, by default (core §6).
pub const GRACE: Duration = Duration::from_secs(2);

/// How long a read after a withdrawal waits for the rest of its burst
/// (§2.8's reference).
pub const BURST: Duration = Duration::from_millis(100);

/// The deployment's lock, under `election = "external"` (§2.11): what the
/// runtime asks before it takes over, and while it serves.
pub trait Lock: Send + Sync + 'static {
    /// Whether this instance holds the lock now, taking it when it is free.
    /// Asked at the start, every [`RedundancyStartup::lock_period`], and at
    /// once after [`Redundancy::lock_changed`]: a `false` while serving is
    /// the lock lost, and the instance steps down.
    fn hold(&self) -> BoxFuture<'_, bool>;
}

/// What the application puts when it takes over (§2.6 step 3): its
/// writers of the interface pass from here on, and the token is declared
/// after it returns.
pub trait Takeover: Send + Sync + 'static {
    /// The interface's resources are declared, and its descriptor still
    /// marks it standby: put each state value the new incumbent holds.
    fn taking_over<'a>(&'a self, iface: &'a IfaceId) -> BoxFuture<'a, ()>;
}

/// A lock the application sets: held or not, as it says
/// ([`ManualLock::set`], then [`Redundancy::lock_changed`]).
#[derive(Debug, Default)]
pub struct ManualLock(std::sync::atomic::AtomicBool);

impl ManualLock {
    /// A lock, held or not.
    #[must_use]
    pub fn new(held: bool) -> Self {
        Self(std::sync::atomic::AtomicBool::new(held))
    }

    /// Holds it, or loses it.
    pub fn set(&self, held: bool) {
        self.0.store(held, std::sync::atomic::Ordering::SeqCst);
    }

    /// Whether it is held.
    #[must_use]
    pub fn get(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Lock for ManualLock {
    fn hold(&self) -> BoxFuture<'_, bool> {
        let held = self.get();
        Box::pin(async move { held })
    }
}

/// A lock file, `flock(LOCK_EX | LOCK_NB)` (§2.11's reference, as
/// zenoh-modem takes its device's): taken when free, held until
/// [`FileLock::release`] or the process ends, when the kernel releases it.
/// Released, it is not taken again until [`FileLock::want`].
#[derive(Debug)]
pub struct FileLock {
    path: std::path::PathBuf,
    file: Mutex<Option<std::fs::File>>,
    wanted: std::sync::atomic::AtomicBool,
}

impl FileLock {
    /// The lock at `path`, not taken yet. The file is created at the first
    /// [`Lock::hold`] if it does not exist.
    #[must_use]
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            path: path.into(),
            file: Mutex::new(None),
            wanted: std::sync::atomic::AtomicBool::new(true),
        }
    }

    /// Releases it, if held, and takes it no more until [`FileLock::want`].
    pub fn release(&self) {
        self.wanted
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.file.lock().expect("not poisoned").take();
    }

    /// Takes it again when free, at the next [`Lock::hold`].
    pub fn want(&self) {
        self.wanted.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Whether this process holds it now.
    #[must_use]
    pub fn held(&self) -> bool {
        self.file.lock().expect("not poisoned").is_some()
    }

    fn try_take(&self) -> bool {
        use std::os::fd::AsRawFd;
        let mut f = self.file.lock().expect("not poisoned");
        if f.is_some() {
            return true;
        }
        if !self.wanted.load(std::sync::atomic::Ordering::SeqCst) {
            return false;
        }
        let Ok(file) = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&self.path)
        else {
            return false;
        };
        // SAFETY: a valid descriptor, owned by `file` for the call.
        let r = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if r == 0 {
            *f = Some(file);
            true
        } else {
            false
        }
    }
}

impl Lock for FileLock {
    fn hold(&self) -> BoxFuture<'_, bool> {
        Box::pin(async move { self.try_take() })
    }
}

/// How a candidate runs (the module doc).
#[derive(Clone)]
pub struct RedundancyStartup {
    /// The reader: every read goes on it. It holds no liveliness
    /// subscriber and no claim (§2.5 step 1, 0.2, F-204), so it is neither
    /// the service's session nor the watch.
    pub reader: zenoh::Session,
    /// The watch session (§2.7), holding the liveliness subscriber on the
    /// address's `@zk/**`. `None`: a poll every [`RedundancyStartup::poll`].
    pub watch: Option<zenoh::Session>,
    /// The poll's period, without a watch session.
    pub poll: Duration,
    /// The retry interval (§2.5).
    pub retry: Duration,
    /// The grace between a heal's two reads (§2.9, core §6).
    pub grace: Duration,
    /// Every read's, round trip's and descriptor GET's timeout.
    pub timeout: Duration,
    /// How long a read after a withdrawal waits for its burst (§2.8).
    pub burst: Duration,
    /// The deployment's lock, under `external` (§2.11).
    pub lock: Option<Arc<dyn Lock>>,
    /// How often a held lock is asked whether it still is, and a free one
    /// whether it can be taken.
    pub lock_period: Duration,
    /// Under `external`, refuse to start without the lock, rather than
    /// stand by (§2.11).
    pub refuse_without_lock: bool,
    /// What the application puts at a takeover (§2.6).
    pub takeover: Option<Arc<dyn Takeover>>,
}

impl std::fmt::Debug for RedundancyStartup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedundancyStartup")
            .field("reader", &self.reader.zid())
            .field("watch", &self.watch.as_ref().map(zenoh::Session::zid))
            .field("poll", &self.poll)
            .field("retry", &self.retry)
            .field("grace", &self.grace)
            .field("timeout", &self.timeout)
            .field("burst", &self.burst)
            .field("lock", &self.lock.is_some())
            .field("lock_period", &self.lock_period)
            .field("refuse_without_lock", &self.refuse_without_lock)
            .field("takeover", &self.takeover.is_some())
            .finish()
    }
}

impl RedundancyStartup {
    /// A candidate reading on `reader`, polling, with the defaults.
    #[must_use]
    pub fn new(reader: zenoh::Session) -> Self {
        Self {
            reader,
            watch: None,
            poll: POLL,
            retry: RETRY,
            grace: GRACE,
            timeout: Duration::from_secs(1),
            burst: BURST,
            lock: None,
            lock_period: Duration::from_secs(1),
            refuse_without_lock: false,
            takeover: None,
        }
    }

    /// With `watch` as its watch session.
    #[must_use]
    pub fn with_watch(mut self, watch: zenoh::Session) -> Self {
        self.watch = Some(watch);
        self
    }

    /// With a retry interval.
    #[must_use]
    pub fn with_retry(mut self, retry: Duration) -> Self {
        self.retry = retry;
        self
    }

    /// With a grace.
    #[must_use]
    pub fn with_grace(mut self, grace: Duration) -> Self {
        self.grace = grace;
        self
    }

    /// With the deployment's lock (`external`).
    #[must_use]
    pub fn with_lock(mut self, lock: Arc<dyn Lock>) -> Self {
        self.lock = Some(lock);
        self
    }

    /// With what the application puts at a takeover.
    #[must_use]
    pub fn with_takeover(mut self, takeover: Arc<dyn Takeover>) -> Self {
        self.takeover = Some(takeover);
        self
    }
}

/// What a candidate's reads have been, for a test or a tool.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reads {
    /// Reads made for the election (§2.5, §2.8, §2.9), each counted when
    /// its answer came.
    pub made: u64,
    /// Of those, complete (core §8.1).
    pub complete: u64,
    /// Reads not made: the reader cut off (core §8.1, 0.37), or refused.
    pub not_made: u64,
    /// The poll's reads, without a watch session.
    pub polls: u64,
    /// The last read's reading: `complete`, `incomplete` with its reason,
    /// or `not_made` with why.
    pub last: Option<String>,
    /// What the poll's last reads found, the latest last, at most
    /// [`POLL_LOG`] of them.
    pub poll_log: Vec<PollRead>,
}

/// How many of the poll's reads [`Reads::poll_log`] keeps.
pub const POLL_LOG: usize = 32;

/// One read of the poll (§2.7, 0.3), as the runtime took it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PollRead {
    /// Whether it was made: a reader cut off makes none.
    pub made: bool,
    /// Whether it was complete (core §8.1).
    pub complete: bool,
    /// The holders of the interface's token it showed.
    pub holders: Vec<InstanceId>,
    /// The holders new to the poll's last: appearances.
    pub appeared: Vec<InstanceId>,
    /// The holders a complete read showed gone: withdrawals.
    pub withdrawn: Vec<InstanceId>,
}

/// One poll read compared with the last's (§2.7; text 0.3, F-215): a
/// holder it shows that is new is an appearance, and only a complete read
/// can show one gone, so a read not complete removes no holder from the
/// last's. Returns what it found and the holders the next read is compared
/// with.
#[must_use]
pub fn poll_step(
    last: &BTreeSet<InstanceId>,
    complete: bool,
    holders: &BTreeSet<InstanceId>,
) -> (PollRead, BTreeSet<InstanceId>) {
    // The model's rule, the fixture's (`polls.json`).
    let p = model::poll(last, complete, holders);
    (
        PollRead {
            made: true,
            complete,
            holders: holders.iter().cloned().collect(),
            appeared: p.appeared,
            withdrawn: p.withdrawn,
        },
        p.next,
    )
}

/// An interface's standing, for the application (never a guess).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Standing {
    /// What its descriptor says.
    pub role: Role,
    /// Why it stands by, as its last stand-by said; `None` while it
    /// serves.
    pub why: Option<Why>,
    /// Why it elects no more: a takeover or a step-down whose declaration,
    /// put or undeclaration zenoh refused, undone in §2.9's order (the
    /// module doc). `None` while it elects.
    pub fault: Option<String>,
}

/// Where a test makes an act fail ([`Redundancy::fail_next`]).
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fail {
    /// A takeover's step 1, the resources.
    Resources,
    /// A takeover's step 3, the descriptor put unmarked.
    Descriptor,
    /// A takeover's step 4, the token.
    Token,
    /// A step-down's step 3, the descriptor put marked.
    StepDown,
}

/// The candidate's handle (the module doc): roles, reads, the lock.
/// Clones share it.
#[derive(Clone)]
pub struct Redundancy {
    inner: Arc<Inner>,
}

struct Inner {
    startup: RedundancyStartup,
    roles: Mutex<BTreeMap<IfaceId, tokio::sync::watch::Sender<Standing>>>,
    drivers: Mutex<Vec<Arc<Driver>>>,
    stop: tokio::sync::watch::Sender<bool>,
    lock_asked: tokio::sync::watch::Sender<u64>,
    heal_hold: tokio::sync::watch::Sender<bool>,
}

impl std::fmt::Debug for Redundancy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Redundancy")
            .field("startup", &self.inner.startup)
            .field("roles", &self.roles())
            .finish()
    }
}

impl Redundancy {
    pub(crate) fn new(
        builder_session: &zenoh::Session,
        startup: RedundancyStartup,
    ) -> Result<Self> {
        let claim = builder_session.zid();
        if startup.reader.zid() == claim {
            return Err(Error::Contract(
                "redundancy.v1 §2.5 step 1: the reader is the service's own session, which \
                 can answer its own claim itself, even through a router that refuses the read: \
                 give a reader of its own (F-204)"
                    .to_owned(),
            ));
        }
        if presence::watching(&startup.reader) {
            return Err(Error::ReadBesideSubscriber {
                selector: "redundancy.v1's reads".to_owned(),
                zid: startup.reader.zid().to_string(),
            });
        }
        if let Some(w) = &startup.watch
            && (w.zid() == claim || w.zid() == startup.reader.zid())
        {
            return Err(Error::Contract(
                "redundancy.v1 §2.7: the watch session is the service's or the reader: give a \
                 session of its own (core §8.1, 0.37)"
                    .to_owned(),
            ));
        }
        Ok(Self {
            inner: Arc::new(Inner {
                startup,
                roles: Mutex::default(),
                drivers: Mutex::default(),
                stop: tokio::sync::watch::Sender::new(false),
                lock_asked: tokio::sync::watch::Sender::new(0),
                heal_hold: tokio::sync::watch::Sender::new(false),
            }),
        })
    }

    pub(crate) fn startup(&self) -> &RedundancyStartup {
        &self.inner.startup
    }

    fn role_tx(&self, iface: &IfaceId) -> tokio::sync::watch::Sender<Standing> {
        self.inner
            .roles
            .lock()
            .expect("not poisoned")
            .entry(iface.clone())
            .or_insert_with(|| {
                tokio::sync::watch::Sender::new(Standing {
                    role: Role::Unclaimed,
                    why: None,
                    fault: None,
                })
            })
            .clone()
    }

    /// `iface`'s role now: [`Role::Unclaimed`] before the start, then
    /// [`Role::Standby`] or [`Role::Serving`].
    #[must_use]
    pub fn role(&self, iface: &IfaceId) -> Role {
        self.role_tx(iface).borrow().role
    }

    /// `iface`'s standing now: its role, and why it stands by.
    #[must_use]
    pub fn standing(&self, iface: &IfaceId) -> Standing {
        self.role_tx(iface).borrow().clone()
    }

    /// Every governed interface's role, once started.
    #[must_use]
    pub fn roles(&self) -> Vec<(IfaceId, Role)> {
        self.inner
            .roles
            .lock()
            .expect("not poisoned")
            .iter()
            .map(|(i, s)| (i.clone(), s.borrow().role))
            .collect()
    }

    /// A watch of `iface`'s standing: each change, after the act that made
    /// it (a takeover once its token is declared, a step-down once its
    /// descriptor is put marked).
    #[must_use]
    pub fn subscribe(&self, iface: &IfaceId) -> tokio::sync::watch::Receiver<Standing> {
        self.role_tx(iface).subscribe()
    }

    /// Waits until `iface`'s role is `role`.
    pub async fn until(&self, iface: &IfaceId, role: Role) {
        let mut rx = self.subscribe(iface);
        let _ = rx.wait_for(|s| s.role == role).await;
    }

    /// `iface`'s reads so far.
    #[must_use]
    pub fn reads(&self, iface: &IfaceId) -> Reads {
        self.driver(iface)
            .map(|d| d.reads.lock().expect("not poisoned").clone())
            .unwrap_or_default()
    }

    /// The governed interfaces, once started.
    #[must_use]
    pub fn governed(&self) -> Vec<IfaceId> {
        self.inner
            .drivers
            .lock()
            .expect("not poisoned")
            .iter()
            .map(|d| d.iface.clone())
            .collect()
    }

    /// The lock changed (`external`): ask it now rather than at the next
    /// period.
    pub fn lock_changed(&self) {
        self.inner.lock_asked.send_modify(|n| *n += 1);
    }

    /// A test's hook (`scenarios.md` §9, §12): while held, an incumbent
    /// that heard another holder appear makes none of its heal's reads, so
    /// a split lasts until released.
    pub fn hold_heal(&self, held: bool) {
        self.inner.heal_hold.send_replace(held);
    }

    /// A test's hook: `iface`'s next act fails at `at`, as if zenoh refused
    /// that declaration or put, so that its undoing is observed.
    #[doc(hidden)]
    pub fn fail_next(&self, iface: &IfaceId, at: Fail) {
        if let Some(d) = self.driver(iface) {
            *d.fail.lock().expect("not poisoned") = Some(at);
        }
    }

    fn driver(&self, iface: &IfaceId) -> Option<Arc<Driver>> {
        self.inner
            .drivers
            .lock()
            .expect("not poisoned")
            .iter()
            .find(|d| &d.iface == iface)
            .cloned()
    }

    pub(crate) fn drivers(&self) -> Vec<Arc<Driver>> {
        self.inner.drivers.lock().expect("not poisoned").clone()
    }

    /// The interfaces whose token this instance holds now.
    pub(crate) fn tokens_held(&self) -> Vec<IfaceId> {
        self.drivers()
            .iter()
            .filter(|d| d.token.lock().expect("not poisoned").is_some())
            .map(|d| d.iface.clone())
            .collect()
    }

    /// Under `external` with [`RedundancyStartup::refuse_without_lock`]:
    /// whether the lock is held, asked before anything is declared.
    pub(crate) async fn may_start(&self) -> bool {
        match (
            &self.inner.startup.lock,
            self.inner.startup.refuse_without_lock,
        ) {
            (Some(l), true) => l.hold().await,
            _ => true,
        }
    }

    /// The claim's rest and the election, from [`crate::ServiceBuilder::start`]
    /// once the instance token is declared (§2.5 step 1): per governed
    /// interface, the watch and its round trip, a round trip on the claim's
    /// session, then the first read.
    pub(crate) async fn start(&self, svc: &Service, governed: Vec<Governed>) -> Result<()> {
        let st = &self.inner.startup;
        let addr = svc.address().clone();
        let own = svc.instance().clone();
        let selector = format!("zk2/{}/{}/@zk/**", addr.system, addr.service);
        for (iface, election, gate, fp16) in governed {
            let mut elector = model::Elector::new(own.clone(), election);
            let step = elector.start();
            debug_assert_eq!(step, Step::Claim);
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            // §2.7: the watch, before the first read, with its round trip.
            let watch = match &st.watch {
                Some(w) => {
                    let tx = tx.clone();
                    let iface_c = iface.clone();
                    let addr_c = addr.clone();
                    // With history (#851): the watch's first state is the
                    // instances it has heard put (`heard`), which a
                    // withdrawal is judged against, and the address's
                    // tokens are a handful, so the round trip below waits
                    // for a history of a few tokens, bounded by the
                    // timeout and the session's interests timeout.
                    let sub = presence::declare_subscriber(w, selector.clone())
                        .history(true)
                        .callback(move |s: Sample| {
                            if let Some(ev) = watch_event(&addr_c, &iface_c, &s) {
                                let _ = tx.send(ev);
                            }
                        })
                        .await
                        .map_err(zenoh)?;
                    if !presence::settle(w, &selector, st.timeout)
                        .await
                        .unwrap_or(false)
                    {
                        tracing::warn!(%iface, "redundancy.v1: the watch's round trip did not come back: a withdrawal right after the first read may not be heard (core §8.1)");
                    }
                    Some(sub)
                }
                None => None,
            };
            // F-204: the claim at the router before the read on another
            // session; scoped by its own instance key, whose barrier key
            // (`…/@zk/settle`) carries nothing (#851).
            let own_key = instance_key(&addr, &own)?.to_string();
            let _ = presence::settle(svc.session(), &own_key, st.timeout).await;
            if election == Election::External
                && let Some(l) = &st.lock
            {
                let held = l.hold().await;
                elector.observe(Event::Lock(held));
            }
            let first = elector.finish();
            let d = Arc::new(Driver {
                iface: iface.clone(),
                election,
                gate,
                fp16,
                addr: addr.clone(),
                selector: selector.clone(),
                session: svc.session().clone(),
                served: svc.served(),
                store: svc.store(),
                core: Arc::new(tokio::sync::Mutex::new(elector)),
                role: self.role_tx(&iface),
                token: Mutex::new(None),
                reads: Mutex::default(),
                startup: st.clone(),
                tx,
                _watch: Mutex::new(watch),
                task: Mutex::new(None),
                fail: Mutex::new(None),
            });
            d.publish(&first, &*d.core.lock().await);
            let task = tokio::spawn(Arc::clone(&d).run(
                first,
                rx,
                self.inner.stop.subscribe(),
                self.inner.lock_asked.subscribe(),
                self.inner.heal_hold.subscribe(),
            ));
            *d.task.lock().expect("not poisoned") = Some(task);
            self.inner.drivers.lock().expect("not poisoned").push(d);
        }
        Ok(())
    }

    /// A clean close, before the service's own (§2.8: tokens first): every
    /// election stopped once its act in progress ends, writes stopped, and
    /// each governed interface's tokens undeclared, then its resources.
    pub(crate) async fn shutdown(&self) -> Result<()> {
        self.inner.stop.send_replace(true);
        for d in self.drivers() {
            let _core = d.core.lock().await;
            if let Some(t) = d.task.lock().expect("not poisoned").take() {
                t.abort();
            }
            d.gate.close_writes().await;
            let token = d.token.lock().expect("not poisoned").take();
            if let Some(t) = token {
                t.undeclare().await.map_err(zenoh)?;
            }
            for t in d.gate.take_members() {
                t.undeclare().await.map_err(zenoh)?;
            }
        }
        Ok(())
    }

    /// After the tokens, at a close: every governed interface's resources
    /// undeclared.
    pub(crate) async fn unexpose_all(&self) -> Result<()> {
        let mut first = Ok(());
        for d in self.drivers() {
            if let Err(e) = d.gate.unexpose().await
                && first.is_ok()
            {
                first = Err(e);
            }
        }
        first
    }

    /// A drop without a close: the elections stop and the tokens go at
    /// once.
    pub(crate) fn abort(&self) {
        self.inner.stop.send_replace(true);
        for d in self.drivers() {
            if let Some(t) = d.task.lock().expect("not poisoned").take() {
                t.abort();
            }
            drop(d.token.lock().expect("not poisoned").take());
            drop(d.gate.take_members());
        }
    }

    /// [`crate::Service::new_epoch`]'s part (F-209): every elector locked,
    /// so no act runs meanwhile, and told of `next` before its tokens.
    pub(crate) async fn remint(&self, next: &InstanceId) -> Vec<Remint> {
        let mut out = Vec::new();
        for d in self.drivers() {
            let mut guard = Arc::clone(&d.core).lock_owned().await;
            guard.observe(Event::Reminted(next.clone()));
            out.push(Remint {
                driver: d,
                _guard: guard,
            });
        }
        out
    }
}

/// One governed interface held through a re-mint ([`Redundancy::remint`]).
pub(crate) struct Remint {
    driver: Arc<Driver>,
    /// Held through the re-mint: no act runs meanwhile.
    _guard: tokio::sync::OwnedMutexGuard<model::Elector>,
}

impl Remint {
    /// Make: the interface's token under `next`, when it serves.
    pub(crate) async fn make(&self, next: &InstanceId) -> Result<Option<LivelinessToken>> {
        let d = &self.driver;
        if d.token.lock().expect("not poisoned").is_none() {
            return Ok(None);
        }
        let key = alive_key(&d.addr, &d.iface, next, &d.fp16)?;
        Ok(Some(
            d.session
                .liveliness()
                .declare_token(key.into_keyexpr())
                .await
                .map_err(zenoh)?,
        ))
    }

    /// Break: `new` held, and the old token returned to be undeclared.
    pub(crate) fn swap(&self, new: Option<LivelinessToken>) -> Option<LivelinessToken> {
        let mut t = self.driver.token.lock().expect("not poisoned");
        match new {
            Some(n) => t.replace(n),
            None => None,
        }
    }
}

/// An event its loop handles, the watch's included.
enum Ev {
    /// The watch heard an interface token of the interface declared.
    Put(InstanceId),
    /// The watch heard one withdrawn.
    Delete(InstanceId),
}

fn watch_event(addr: &Addr, iface: &IfaceId, s: &Sample) -> Option<Ev> {
    match zenkey_model::grammar::parse(s.key_expr().as_str()).ok()? {
        ZkKey::Alive {
            addr: a,
            iface: i,
            instance,
            ..
        } if a == *addr && i == *iface => Some(match s.kind() {
            SampleKind::Put => Ev::Put(instance),
            SampleKind::Delete => Ev::Delete(instance),
        }),
        _ => None,
    }
}

/// A task aborted when its handle drops: the poll, with its election.
struct AbortOnDrop(Option<tokio::task::JoinHandle<()>>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(t) = self.0.take() {
            t.abort();
        }
    }
}

/// One governed interface's election (the module doc).
pub(crate) struct Driver {
    iface: IfaceId,
    election: Election,
    gate: Arc<Gate>,
    fp16: Fp16,
    addr: Addr,
    selector: String,
    /// The claim's session: the service's.
    session: zenoh::Session,
    served: Arc<Served>,
    store: Arc<Store>,
    core: Arc<tokio::sync::Mutex<model::Elector>>,
    role: tokio::sync::watch::Sender<Standing>,
    /// The interface's token, while it serves.
    token: Mutex<Option<LivelinessToken>>,
    reads: Mutex<Reads>,
    startup: RedundancyStartup,
    tx: tokio::sync::mpsc::UnboundedSender<Ev>,
    _watch: Mutex<Option<zenoh::pubsub::Subscriber<()>>>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// A test's injected failure ([`Redundancy::fail_next`]).
    fail: Mutex<Option<Fail>>,
}

impl Driver {
    /// Says the elector's role, and why it stands by, after a step.
    fn publish(&self, step: &Step, e: &model::Elector) {
        let role = e.role();
        self.role.send_if_modified(|s| {
            let why = match (role, step) {
                (Role::Serving, _) => None,
                (_, Step::StandBy { why, .. }) => Some(why.clone()),
                _ => s.why.clone(),
            };
            let next = Standing {
                role,
                why,
                fault: s.fault.clone(),
            };
            if *s == next {
                false
            } else {
                *s = next;
                true
            }
        });
    }

    async fn run(
        self: Arc<Self>,
        first: Step,
        mut rx: tokio::sync::mpsc::UnboundedReceiver<Ev>,
        mut stop: tokio::sync::watch::Receiver<bool>,
        mut lock_asked: tokio::sync::watch::Receiver<u64>,
        heal_hold: tokio::sync::watch::Receiver<bool>,
    ) {
        let st = self.startup.clone();
        let mut step = first;
        // Whether the step to act on came from a withdrawal: its read waits
        // for the burst (§2.8).
        let mut after_withdrawal = false;
        let mut timer: Option<tokio::time::Instant> = None;
        let mut reading: Option<tokio::task::JoinHandle<Read>> = None;
        // The holders the watch heard, or the poll found.
        let mut heard: BTreeSet<InstanceId> = BTreeSet::new();
        let mut lock_held: Option<bool> = None;
        let external = self.election == Election::External && st.lock.is_some();
        let mut lock_tick = tokio::time::interval(st.lock_period);
        lock_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Without a watch session, the poll (the module doc), on a task of
        // its own that hands the loop what it finds.
        let poller = st
            .watch
            .is_none()
            .then(|| tokio::spawn(Arc::clone(&self).poll(stop.clone())));
        let _poller = AbortOnDrop(poller);
        loop {
            if *stop.borrow() {
                return;
            }
            // Act on the step: acts run under the elector's lock, so a
            // re-mint never interleaves with one (F-209).
            while matches!(step, Step::TakeOver | Step::StepDown(_)) {
                let mut e = self.core.lock().await;
                if *stop.borrow() {
                    return;
                }
                let (act, acted) = if step == Step::TakeOver {
                    ("takeover", self.take_over(e.own()).await)
                } else {
                    ("step-down", self.step_down().await)
                };
                if let Err(err) = acted {
                    // A declaration, a put or an undeclaration zenoh
                    // refused: its session is closing, or broken. The act is
                    // undone (§2.9's order), and the instance stands by and
                    // elects no more, saying why (text 0.2, "Decided here").
                    let fault = format!(
                        "the {act} failed, was undone, and this instance elects no more: {err}"
                    );
                    tracing::error!(iface = %self.iface, "redundancy.v1: {fault}");
                    self.role.send_modify(|s| {
                        s.role = Role::Standby;
                        s.why = None;
                        s.fault = Some(fault);
                    });
                    return;
                }
                step = e.finish();
                self.publish(&step, &e);
                after_withdrawal = false;
            }
            match &step {
                // The model never asks a read while one it asked is
                // unanswered (zenkey_model::redundancy's
                // `a_read_asked_is_never_asked_again_before_its_answer`): a
                // read in flight is never aborted, so no phase waiting on it
                // is stranded.
                Step::Read if reading.is_some() => {
                    tracing::error!(
                        iface = %self.iface,
                        "redundancy.v1: a read asked while one runs; the one in flight is kept"
                    );
                }
                Step::Read => {
                    let delay = if after_withdrawal {
                        st.burst
                    } else {
                        Duration::ZERO
                    };
                    reading = Some(tokio::spawn(
                        Arc::clone(&self).read_after(delay, heal_hold.clone()),
                    ));
                }
                Step::Grace => timer = Some(tokio::time::Instant::now() + st.grace),
                Step::Retry => timer = Some(tokio::time::Instant::now() + st.retry),
                Step::StandBy { retry, .. } => {
                    timer = retry.then(|| tokio::time::Instant::now() + st.retry);
                }
                Step::Claim | Step::Nothing | Step::TakeOver | Step::StepDown(_) => {}
            }
            step = Step::Nothing;
            after_withdrawal = false;
            let sleep = async move {
                match timer {
                    Some(t) => tokio::time::sleep_until(t).await,
                    None => std::future::pending::<()>().await,
                }
            };
            let read = async {
                match reading.as_mut() {
                    Some(r) => r.await.ok(),
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                ev = rx.recv() => {
                    let Some(ev) = ev else { return };
                    let mut e = self.core.lock().await;
                    match ev {
                        Ev::Put(id) => {
                            heard.insert(id.clone());
                            if !e.is_own(&id) {
                                step = e.observe(Event::Appeared(id));
                            }
                        }
                        Ev::Delete(id) => {
                            let was = heard.remove(&id);
                            if was && !e.is_own(&id) && heard.iter().all(|h| e.is_own(h)) {
                                step = e.observe(Event::Withdrawn);
                                after_withdrawal = true;
                            }
                        }
                    }
                    self.publish(&step, &e);
                }
                r = read => {
                    reading = None;
                    // A read task that panicked made no read.
                    let r = r.unwrap_or_default();
                    let mut e = self.core.lock().await;
                    let standby = e.role() == Role::Standby;
                    step = e.observe(Event::Read(r));
                    // A read decided into another while standing by: a
                    // withdrawal kept meanwhile (F-199), whose read waits
                    // for the burst.
                    after_withdrawal = standby && step == Step::Read;
                    self.publish(&step, &e);
                }
                () = sleep => {
                    timer = None;
                    let mut e = self.core.lock().await;
                    step = e.observe(Event::Timer);
                    self.publish(&step, &e);
                }
                _ = lock_tick.tick(), if external => {
                    if let Some(l) = &st.lock {
                        let held = l.hold().await;
                        if lock_held != Some(held) {
                            lock_held = Some(held);
                            let mut e = self.core.lock().await;
                            step = e.observe(Event::Lock(held));
                            self.publish(&step, &e);
                        }
                    }
                }
                changed = lock_asked.changed(), if external => {
                    if changed.is_err() {
                        return;
                    }
                    lock_tick.reset_immediately();
                }

                changed = stop.changed() => {
                    if changed.is_err() || *stop.borrow() {
                        return;
                    }
                }
            }
        }
    }

    /// The poll (the module doc): a read of the address every period, its
    /// holders compared with the last poll's, each change handed to the
    /// loop as the watch's would be, until the election stops.
    async fn poll(self: Arc<Self>, stop: tokio::sync::watch::Receiver<bool>) {
        let mut polled = None;
        let mut tick = tokio::time::interval(self.startup.poll);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        while !*stop.borrow() {
            tick.tick().await;
            self.poll_once(&mut polled).await;
        }
    }

    async fn poll_once(&self, polled: &mut Option<BTreeSet<InstanceId>>) {
        let st = &self.startup;
        let Ok(read) = presence::liveliness_read(&st.reader, &self.selector, st.timeout).await
        else {
            self.log_poll(PollRead::default());
            return;
        };
        let now: BTreeSet<InstanceId> = read
            .keys
            .iter()
            .filter_map(|k| match zenkey_model::grammar::parse(k).ok()? {
                ZkKey::Alive {
                    addr,
                    iface,
                    instance,
                    ..
                } if addr == self.addr && iface == self.iface => Some(instance),
                _ => None,
            })
            .collect();
        let before = polled.clone().unwrap_or_default();
        // A read not complete establishes no absence (core §8.1).
        let (found, next) = poll_step(&before, read.complete, &now);
        for id in &found.appeared {
            let _ = self.tx.send(Ev::Put(id.clone()));
        }
        for id in &found.withdrawn {
            let _ = self.tx.send(Ev::Delete(id.clone()));
        }
        *polled = Some(next);
        self.log_poll(found);
    }

    fn log_poll(&self, found: PollRead) {
        let mut r = self.reads.lock().expect("not poisoned");
        if found.made {
            r.polls += 1;
        }
        r.poll_log.push(found);
        if r.poll_log.len() > POLL_LOG {
            r.poll_log.remove(0);
        }
    }

    /// One read, after `delay`, and, for a heal's, after the test's hold.
    async fn read_after(
        self: Arc<Self>,
        delay: Duration,
        mut heal_hold: tokio::sync::watch::Receiver<bool>,
    ) -> Read {
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        if self.role.borrow().role == Role::Serving {
            let _ = heal_hold.wait_for(|held| !held).await;
        }
        self.read().await
    }

    /// One read of the address (§0): its instance tokens and the
    /// interface's tokens, on the reader, with the descriptors of the ids
    /// below its own.
    async fn read(&self) -> Read {
        let st = &self.startup;
        let own = self.core.lock().await.own().clone();
        let pr = match presence::liveliness_read(&st.reader, &self.selector, st.timeout).await {
            Ok(pr) => pr,
            Err(e) => {
                let mut r = self.reads.lock().expect("not poisoned");
                r.not_made += 1;
                r.last = Some(format!("not_made: {e}"));
                // Core §8.1 (0.38): a read not made is read as one not
                // complete, and shows none.
                return Read {
                    complete: false,
                    instances: Vec::new(),
                };
            }
        };
        let mut by_id: BTreeMap<InstanceId, Seen> = BTreeMap::new();
        let unread = |id: &InstanceId| Seen {
            id: id.clone(),
            token: false,
            descriptor: Listing::Unread,
        };
        for k in &pr.keys {
            match zenkey_model::grammar::parse(k) {
                Ok(ZkKey::Instance { addr, instance }) if addr == self.addr => {
                    by_id
                        .entry(instance.clone())
                        .or_insert_with(|| unread(&instance));
                }
                Ok(ZkKey::Alive {
                    addr,
                    iface,
                    instance,
                    ..
                }) if addr == self.addr && iface == self.iface => {
                    by_id
                        .entry(instance.clone())
                        .or_insert_with(|| unread(&instance))
                        .token = true;
                }
                _ => {}
            }
        }
        for (id, seen) in &mut by_id {
            if *id == own {
                seen.descriptor = Listing::Listed {
                    standby: self.gate.is_marked(),
                };
            } else if model::lower(id, &own) {
                seen.descriptor = self.listing(id).await;
            }
        }
        {
            let mut r = self.reads.lock().expect("not poisoned");
            r.made += 1;
            if pr.complete {
                r.complete += 1;
            }
            r.last = Some(match pr.incomplete_reason() {
                None => "complete".to_owned(),
                Some(why) => format!("incomplete: the read {why}"),
            });
        }
        Read {
            complete: pr.complete,
            instances: by_id.into_values().collect(),
        }
    }

    /// What `id`'s descriptor says of the interface (§2.3).
    async fn listing(&self, id: &InstanceId) -> Listing {
        let st = &self.startup;
        match presence::descriptor(&st.reader, &self.addr, id, st.timeout).await {
            Ok(presence::Found::Descriptor(d, _)) => {
                let want = self.iface.to_string();
                match d.interfaces.iter().find(|e| e.iface == want) {
                    Some(e) => Listing::Listed { standby: e.standby },
                    None => Listing::NotListed,
                }
            }
            _ => Listing::Unread,
        }
    }

    /// A test's injected failure at `at`, once (`Redundancy::fail_next`).
    fn injected(&self, at: Fail) -> Result<()> {
        let mut f = self.fail.lock().expect("not poisoned");
        if *f == Some(at) {
            *f = None;
            return Err(Error::Zenoh(format!(
                "a test's failure, injected at {at:?}"
            )));
        }
        Ok(())
    }

    /// §2.6: the resources, the application's state, the descriptor
    /// unmarked, the token last. A step that fails undoes every step made
    /// before it, in §2.9's order ([`Driver::step_down`]), and the failure
    /// is returned, with the undoing's own failure if it had one.
    async fn take_over(&self, own: &InstanceId) -> Result<()> {
        // 1. The gate undoes its own declarations when one fails.
        self.injected(Fail::Resources)?;
        self.gate.expose().await?;
        let rest = async {
            // 3.
            if let Some(t) = &self.startup.takeover {
                t.taking_over(&self.iface).await;
            }
            self.gate.set_marked(false);
            self.injected(Fail::Descriptor)?;
            self.served.publish()?;
            // 4.
            self.injected(Fail::Token)?;
            let key = alive_key(&self.addr, &self.iface, own, &self.fp16)?;
            let t = self
                .session
                .liveliness()
                .declare_token(key.into_keyexpr())
                .await
                .map_err(zenoh)?;
            *self.token.lock().expect("not poisoned") = Some(t);
            Ok::<_, Error>(())
        };
        if let Err(e) = rest.await {
            return Err(match self.step_down().await {
                Ok(()) => e,
                Err(undo) => Error::Zenoh(format!("{e}; and undoing it: {undo}")),
            });
        }
        tracing::info!(iface = %self.iface, instance = %own, "redundancy.v1: took over (§2.6)");
        Ok(())
    }

    /// §2.9: writes stopped, the tokens, the resources and what it put,
    /// the descriptor marked. Every step is made, whatever one before it
    /// gave; the first failure is returned.
    async fn step_down(&self) -> Result<()> {
        let mut first: Result<()> = Ok(());
        let mut keep = |r: Result<()>| {
            if let Err(e) = r
                && first.is_ok()
            {
                first = Err(e);
            }
        };
        self.gate.close_writes().await;
        // 1.
        let token = self.token.lock().expect("not poisoned").take();
        if let Some(t) = token {
            keep(t.undeclare().await.map_err(zenoh));
        }
        for t in self.gate.take_members() {
            keep(t.undeclare().await.map_err(zenoh));
        }
        // 2.
        keep(self.gate.unexpose().await);
        self.store.forget_prefix(&format!(
            "zk2/{}/{}/{}/",
            self.addr.system, self.addr.service, self.iface
        ));
        // 3.
        self.gate.set_marked(true);
        match self.injected(Fail::StepDown) {
            Err(e) => keep(Err(e)),
            Ok(()) => keep(self.served.publish()),
        }
        tracing::info!(iface = %self.iface, "redundancy.v1: stepped down (§2.9)");
        first
    }
}

/// A governed interface as [`Redundancy::start`] elects it: its id, its
/// election, its gate, and its token's `fp16`.
pub(crate) type Governed = (IfaceId, Election, Arc<Gate>, Fp16);

/// The governed interfaces of `impls`, their elections, refusing what
/// §2.2 and §2.14 refuse (before step 1: nothing is declared).
pub(crate) fn elections(
    impls: &[crate::service::ImplState],
    tokenless: &BTreeSet<IfaceId>,
    configured: bool,
) -> Result<Vec<Governed>> {
    let mut out = Vec::new();
    let mut refused = Vec::new();
    for s in impls {
        let iface = s.imp.iface();
        match model::declaration_of(s.imp.contract(), tokenless.contains(iface)) {
            model::Declaration::NotGoverned => {}
            model::Declaration::Invalid(why) => refused.push(format!(
                "{iface}'s redundancy.v1 declaration is invalid: {} (redundancy.v1 §2.2, §2.14)",
                why.iter()
                    .map(|w| format!("{} ({})", w.as_str(), w.says()))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            model::Declaration::Governed(e) => match &s.gate {
                Some(g) if configured => {
                    out.push((
                        iface.clone(),
                        e,
                        Arc::clone(g),
                        s.imp.fingerprint().hex().fp16(),
                    ));
                }
                _ => refused.push(format!(
                    "{iface} is governed by redundancy.v1, and this service takes part in no \
                     election: call ServiceBuilder::redundancy (redundancy.v1 §2.5)"
                )),
            },
        }
    }
    if !refused.is_empty() {
        return Err(Error::NotExposed(refused.join("; ")));
    }
    if configured && out.is_empty() {
        return Err(Error::NotExposed(
            "ServiceBuilder::redundancy was called, and no interface this service implements is \
             governed by redundancy.v1 (§2.2)"
                .to_owned(),
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(ids: &[u64]) -> BTreeSet<InstanceId> {
        ids.iter().map(|n| InstanceId::from_u64(*n)).collect()
    }

    /// §2.7's poll (text 0.3, F-215): a read not complete removes no holder
    /// from the last's, and only a complete read shows one gone.
    #[test]
    fn a_poll_read_not_complete_removes_no_holder() {
        let a = InstanceId::from_u64(0xa);
        // complete {A}, then not complete {}, then complete {}: a withdrawal.
        let (first, last) = poll_step(&set(&[]), true, &set(&[0xa]));
        assert_eq!(first.appeared, std::slice::from_ref(&a));
        let (second, last) = poll_step(&last, false, &set(&[]));
        assert!(second.appeared.is_empty() && second.withdrawn.is_empty());
        assert_eq!(last, set(&[0xa]), "A kept");
        let (third, last) = poll_step(&last, true, &set(&[]));
        assert_eq!(
            third.withdrawn,
            std::slice::from_ref(&a),
            "only a complete read shows it gone"
        );
        assert!(last.is_empty());
        // complete {A}, then not complete {}, then complete {A}: no appearance.
        let (_, last) = poll_step(&set(&[]), true, &set(&[0xa]));
        let (_, last) = poll_step(&last, false, &set(&[]));
        let (third, _) = poll_step(&last, true, &set(&[0xa]));
        assert!(third.appeared.is_empty(), "A was never gone");
        assert!(third.withdrawn.is_empty());
    }
}
