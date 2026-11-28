//! Declaring a subscription again after silence, and repairing it (core
//! §8.5, `link.v2` §2.21; #988 L4, #1185).
//!
//! zenoh 1.10.1 never repairs a declaration its medium lost while the
//! transport stays up: a subscriber whose declaration a radio dropped is
//! never heard of again by its router, and nothing at either end says so
//! (core Appendix B). A reader across a constrained face whose medium
//! repairs no lost frame (`arq = false`) therefore declares a subscription
//! again when it has heard nothing on it for two of its horizons
//! ([`zenkey_model::link::SILENT_HORIZONS`]) while its session's transport
//! stayed up, and repairs it when that changed nothing.
//!
//! - **Its horizon** is that of what it hears
//!   ([`zenkey_model::link::redeclare_horizon`],
//!   [`zenkey_model::link::redeclare_heartbeat_horizon`]).
//! - **What has none** (an event's subscription; a `freshness.v2` state's)
//!   follows the subscriptions here that have one: at a check where one
//!   of them is due, it is due too if it has heard nothing for as long.
//!   Its declaration rode with theirs, and is lost with them (#1185).
//! - **The silence** counts from the later of the subscription's
//!   declaration, its last sample (any sample, delivered or refused: the
//!   declaration stands), its last re-declaration or repair, and its
//!   session's last transport event. A transport that opens again declares
//!   the session's subscriptions anew, so a silence across one says
//!   nothing of a lost declaration.
//! - **The first time it is due, break, then make.** The old subscriber is
//!   undeclared first, then the same key expression declared anew with the
//!   same callback: zenoh 1.10.1 joins a subscriber on the key of one the
//!   session holds to it, and declares nothing (`register_subscriber`). A
//!   sample put in the instant between is missed; the subscription has
//!   heard nothing for two horizons.
//! - **Each later time, a repair** (#1185): a subscriber on
//!   [`zenkey_model::link::repair_key`], the key expression with `$*` in
//!   the last chunk of its resource that holds no wildcard. Declaring
//!   again changes nothing at a near router that holds another
//!   subscription to the key: it declares toward the far side once, for
//!   its first subscriber. No other reader declares the repair, so the
//!   router declares it across, with a key expression of its own, and the
//!   far router routes the key's samples to it, for every reader behind
//!   it. So it repairs a lost `DeclareKeyExpr` too.
//!   - **It is kept** while the subscription stands: undeclaring it would
//!     undeclare it at the far router too, where the subscription's own
//!     declaration is still missing.
//!   - **It delivers nothing.** A sample reaches the session once, and the
//!     subscription's own subscriber delivers what it matches; a key that
//!     only extends the repair's chunk is dropped.
//!   - **Up to [`zenkey_model::link::REPAIRS`] in its life**, each further
//!     left in the chunk, since another reader behind the router may hold
//!     the same one. Past them, the subscription is declared again as at
//!     first, and nothing more is declared beside it: a repair held again
//!     would join the one held, and declare nothing (#1185's review).
//!   - **A sample heard** ends the run: the next silence starts from a
//!     re-declaration again, the repairs held staying, and a repair after
//!     it is the next one not held yet.
//! - **Not through a session cut off**: it holds no link
//!   ([`crate::presence::cut_off`]), so a declaration goes nowhere, and the
//!   transport that opens again declares everything anyway.
//! - **A declaration that fails** leaves the subscription with no
//!   subscriber, declared again at the next check; a repair that fails is
//!   tried again at the next time due. Each failure is counted
//!   ([`Redeclaring::failed`]) and warned of, at most once a horizon for
//!   each subscription, so a watch that stays dead says so (#1047).
//! - **No lock across the network** (#1047): the checks take the due
//!   subscriptions' subscribers under the lock, undeclare and declare with
//!   it released, on the blocking pool, and take it again to store what
//!   they declared. zenoh 1.10.1 declares in place, so a declaration that
//!   waits for room behind a slow link would otherwise hold a runtime
//!   worker, and the runtime's timers with it. An
//!   [`Redeclaring::undeclare`] meanwhile returns at once, and what was
//!   declared for the slot it removed is undeclared, not stored.
//!
//! [`Redeclaring`] is the one place it happens: the runtime's
//! [`crate::consumer::Consumer::across`] subscriptions, one for each
//! consumer, and the fleet's monitor watches both declare through it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use zenoh::Wait as _;
use zenoh::key_expr::OwnedKeyExpr;
use zenoh::pubsub::Subscriber;
use zenoh::sample::Sample;

use crate::error::{Result, zenoh};

/// A subscriber's callback, shared by each of its declarations.
pub type Callback = Arc<dyn Fn(Sample) + Send + Sync>;

/// How often the silences are checked, at most: a tenth of the shortest
/// silence, within these bounds.
const TICK_MIN: Duration = Duration::from_millis(20);
const TICK_MAX: Duration = Duration::from_secs(1);

/// A slot's id, to undeclare it by ([`Redeclaring::undeclare`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SlotId(u64);

/// What was done for some subscriptions ([`Redeclaring::counts`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Declared again after silence (break, then make).
    pub redeclared: u64,
    /// Repair subscriptions declared (core §8.5).
    pub repaired: u64,
    /// Declarations again, or repairs, that failed.
    pub failed: u64,
}

impl Counts {
    fn add(&mut self, other: Counts) {
        self.redeclared += other.redeclared;
        self.repaired += other.repaired;
        self.failed += other.failed;
    }
}

/// One subscription that is declared again, and repaired, after silence.
struct Slot {
    id: SlotId,
    key: OwnedKeyExpr,
    /// What it hears: two of it are the silence that makes it due, and a
    /// failed declaration warns at most once in one. `None` for what has
    /// none: it follows the slots that have one.
    horizon: Option<Duration>,
    callback: Callback,
    /// When it last heard a sample, or was declared, declared again or
    /// repaired: ns since `base`.
    heard: Arc<AtomicU64>,
    /// `None` between an undeclaration and a declaration that failed, and
    /// while a check declares it again: the next check declares it again.
    subscriber: Option<Subscriber<()>>,
    /// Its repair subscriptions, kept while it stands.
    repairs: Vec<(String, Subscriber<()>)>,
    /// The times it was due since it last heard a sample: the first
    /// declares it again, each later one a repair while it holds fewer
    /// than [`zenkey_model::link::REPAIRS`].
    dues: u32,
    /// When it last was declared again or repaired, ns since `base`: a
    /// sample after it ends the dues.
    acted: u64,
    /// When a failed declaration last warned: ns since `base`.
    warned: Option<u64>,
    counts: Counts,
}

impl Slot {
    fn silence(&self) -> Option<Duration> {
        self.horizon.map(zenkey_model::link::FarSide::silence)
    }

    /// Whether it has heard nothing for `silence` since the later of its
    /// last sample (or act) and the transport's last move.
    fn quiet_for(&self, now: u64, moved: u64, silence: Duration) -> bool {
        let quiet = self.heard.load(Ordering::SeqCst).max(moved);
        now.saturating_sub(quiet) >= nanos(silence)
    }

    /// Whether a check acts on it: it holds no subscriber, or has heard
    /// nothing for its silence, or, with no horizon of its own, for
    /// `followed`, the shortest silence of the slots due at this check.
    fn due(&self, now: u64, moved: u64, followed: Option<Duration>) -> bool {
        if self.subscriber.is_none() {
            return true;
        }
        // A slot with a horizon has a silence of its own; one with none
        // takes the followed one.
        self.silence()
            .or(followed)
            .is_some_and(|s| self.quiet_for(now, moved, s))
    }

    /// What a check does for it now that it is due.
    fn act(&mut self, now: u64) -> Act {
        let what = if self.subscriber.is_none() {
            // A declaration that failed: declared again, its run as it was.
            What::Declare { old: None }
        } else {
            if self.heard.load(Ordering::SeqCst) > self.acted {
                self.dues = 0;
            }
            self.dues += 1;
            self.heard.store(now, Ordering::SeqCst);
            self.acted = now;
            // The next repair is the first it does not hold: the repairs
            // stay across runs, so three in its life (core §8.5).
            let next = u32::try_from(self.repairs.len() + 1).unwrap_or(u32::MAX);
            let repair = (self.dues > 1)
                .then(|| zenkey_model::link::repair_key(self.key.as_str(), next))
                .flatten();
            match repair.map(OwnedKeyExpr::new) {
                Some(Ok(key)) => What::Repair(key),
                _ => What::Declare {
                    old: self.subscriber.take(),
                },
            }
        };
        Act {
            id: self.id,
            key: self.key.clone(),
            callback: Arc::clone(&self.callback),
            heard: Arc::clone(&self.heard),
            what,
        }
    }
}

/// A due slot's declaration in flight, made with the lock released.
struct Act {
    id: SlotId,
    key: OwnedKeyExpr,
    callback: Callback,
    heard: Arc<AtomicU64>,
    what: What,
}

enum What {
    /// Break, then make: its old subscriber, undeclared first.
    Declare { old: Option<Subscriber<()>> },
    /// A repair subscription on this key expression.
    Repair(OwnedKeyExpr),
}

struct Inner {
    session: zenoh::Session,
    base: Instant,
    slots: Mutex<Vec<Slot>>,
    /// The session's last transport event, ns since `base`.
    moved: Arc<AtomicU64>,
    /// Every slot's, those undeclared included.
    totals: Mutex<Counts>,
    next: AtomicU64,
    /// The shortest silence of any slot declared with a horizon, for the
    /// tick.
    shortest: Mutex<Option<Duration>>,
    _events: zenoh::session::TransportEventsListener<()>,
    /// How many declarations to refuse, the tests' failure, after
    /// `allow` more pass.
    #[cfg(test)]
    refuse: std::sync::atomic::AtomicU32,
    #[cfg(test)]
    allow: std::sync::atomic::AtomicU32,
}

/// The subscriptions of one reader across a face that are declared again,
/// and repaired, after silence (see the module doc). Dropping it stops
/// the checks and undeclares every subscriber it holds.
pub struct Redeclaring {
    inner: Arc<Inner>,
    task: tokio::task::JoinHandle<()>,
}

impl std::fmt::Debug for Redeclaring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Redeclaring")
            .field("redeclared", &self.redeclared())
            .field("repaired", &self.repaired())
            .field("failed", &self.failed())
            .finish_non_exhaustive()
    }
}

impl Drop for Redeclaring {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn since(base: Instant) -> u64 {
    nanos(base.elapsed())
}

fn nanos(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Redeclaring {
    /// Starts watching `session`'s transport events, and the checks.
    pub async fn start(session: &zenoh::Session) -> Result<Redeclaring> {
        let base = Instant::now();
        let moved = Arc::new(AtomicU64::new(0));
        let m = Arc::clone(&moved);
        let events = session
            .info()
            .transport_events_listener()
            .callback(move |_| m.store(since(base), Ordering::SeqCst))
            .await
            .map_err(zenoh)?;
        let inner = Arc::new(Inner {
            session: session.clone(),
            base,
            slots: Mutex::new(Vec::new()),
            moved,
            totals: Mutex::new(Counts::default()),
            next: AtomicU64::new(0),
            shortest: Mutex::new(None),
            _events: events,
            #[cfg(test)]
            refuse: std::sync::atomic::AtomicU32::new(0),
            #[cfg(test)]
            allow: std::sync::atomic::AtomicU32::new(0),
        });
        let weak = Arc::downgrade(&inner);
        let task = tokio::spawn(async move {
            loop {
                let Some(inner) = weak.upgrade() else { return };
                let tick =
                    lock(&inner.shortest).map_or(TICK_MAX, |s| (s / 10).clamp(TICK_MIN, TICK_MAX));
                inner.check().await;
                drop(inner);
                tokio::time::sleep(tick).await;
            }
        });
        Ok(Redeclaring { inner, task })
    }

    /// Declares a subscriber to `key` with `callback`, declared again, and
    /// repaired, whenever it hears nothing for two `horizon`s while the
    /// session's transport stays up. With no `horizon` (what has none, an
    /// event's), it follows the subscriptions here that have one (core
    /// §8.5).
    pub async fn declare(
        &self,
        key: OwnedKeyExpr,
        horizon: Option<Duration>,
        callback: Callback,
    ) -> Result<SlotId> {
        let now = since(self.inner.base);
        let heard = Arc::new(AtomicU64::new(now));
        let subscriber = self.inner.subscribe(&key, &callback, &heard)?;
        let id = SlotId(self.inner.next.fetch_add(1, Ordering::Relaxed));
        if let Some(silence) = horizon.map(zenkey_model::link::FarSide::silence) {
            let mut shortest = lock(&self.inner.shortest);
            *shortest = Some(shortest.map_or(silence, |s| s.min(silence)));
        }
        lock(&self.inner.slots).push(Slot {
            id,
            key,
            horizon,
            callback,
            heard,
            subscriber: Some(subscriber),
            repairs: Vec::new(),
            dues: 0,
            acted: now,
            warned: None,
            counts: Counts::default(),
        });
        Ok(id)
    }

    /// Undeclares one slot's subscriber and its repairs, acknowledged.
    /// `false` when it holds none of that id. A check declaring the slot
    /// again meanwhile holds its subscriber, so this returns at once, and
    /// the check undeclares what it declared.
    pub async fn undeclare(&self, id: SlotId) -> Result<bool> {
        let slot = {
            let mut slots = lock(&self.inner.slots);
            let Some(i) = slots.iter().position(|s| s.id == id) else {
                return Ok(false);
            };
            slots.remove(i)
        };
        if let Some(s) = slot.subscriber {
            s.undeclare().await.map_err(zenoh)?;
        }
        for (_, r) in slot.repairs {
            r.undeclare().await.map_err(zenoh)?;
        }
        Ok(true)
    }

    /// Drops these slots, unacknowledged, as a dropped subscriber is: for
    /// a holder that cannot wait, such as a subscription's `Drop`.
    pub fn forget(&self, ids: &[SlotId]) {
        let gone: Vec<Slot> = {
            let mut slots = lock(&self.inner.slots);
            let (gone, kept) = std::mem::take(&mut *slots)
                .into_iter()
                .partition(|s| ids.contains(&s.id));
            *slots = kept;
            gone
        };
        drop(gone);
    }

    /// How many times a silent subscription was declared again.
    #[must_use]
    pub fn redeclared(&self) -> u64 {
        lock(&self.inner.totals).redeclared
    }

    /// How many repair subscriptions were declared (core §8.5).
    #[must_use]
    pub fn repaired(&self) -> u64 {
        lock(&self.inner.totals).repaired
    }

    /// How many times declaring a subscription again, or a repair, failed:
    /// each is retried, and warned of at most once a horizon.
    #[must_use]
    pub fn failed(&self) -> u64 {
        lock(&self.inner.totals).failed
    }

    /// What was done for these slots, those it still holds.
    #[must_use]
    pub fn counts(&self, ids: &[SlotId]) -> Counts {
        let mut c = Counts::default();
        for s in lock(&self.inner.slots)
            .iter()
            .filter(|s| ids.contains(&s.id))
        {
            c.add(s.counts);
        }
        c
    }

    /// The key expressions it holds a subscriber to, in declaration order.
    #[must_use]
    pub fn keys(&self) -> Vec<String> {
        lock(&self.inner.slots)
            .iter()
            .map(|s| s.key.to_string())
            .collect()
    }

    /// The tests' failure (`cfg(test)`): `allowed` declarations pass,
    /// then the next `refused` fail.
    #[cfg(test)]
    pub(crate) fn refuse_after(&self, allowed: u32, refused: u32) {
        self.inner.allow.store(allowed, Ordering::SeqCst);
        self.inner.refuse.store(refused, Ordering::SeqCst);
    }

    /// The key expressions of the repair subscriptions it holds, each
    /// slot's in the order declared.
    #[must_use]
    pub fn repair_keys(&self) -> Vec<String> {
        lock(&self.inner.slots)
            .iter()
            .flat_map(|s| s.repairs.iter().map(|(k, _)| k.clone()))
            .collect()
    }
}

/// Declares a subscriber to `key` on `session`, in place: zenoh 1.10.1's
/// declaration does not await, whatever it waits for.
fn subscribe(
    session: &zenoh::Session,
    key: &OwnedKeyExpr,
    callback: &Callback,
    heard: &Arc<AtomicU64>,
    base: Instant,
) -> Result<Subscriber<()>> {
    let (callback, heard) = (Arc::clone(callback), Arc::clone(heard));
    session
        .declare_subscriber(key.clone())
        .callback(move |sample: Sample| {
            heard.store(since(base), Ordering::SeqCst);
            callback(sample);
        })
        .wait()
        .map_err(zenoh)
}

/// Declares a repair subscriber to `key` on `session`, in place. It
/// delivers nothing: the session hands each sample to every subscriber
/// that matches it, so the subscription's own delivers what it matches,
/// and what the repair alone matches is dropped (core §8.5).
fn repair(session: &zenoh::Session, key: &OwnedKeyExpr) -> Result<Subscriber<()>> {
    session
        .declare_subscriber(key.clone())
        .callback(|_: Sample| {})
        .wait()
        .map_err(zenoh)
}

impl Inner {
    fn subscribe(
        &self,
        key: &OwnedKeyExpr,
        callback: &Callback,
        heard: &Arc<AtomicU64>,
    ) -> Result<Subscriber<()>> {
        self.refused()?;
        subscribe(&self.session, key, callback, heard, self.base)
    }

    /// The tests' failure: a declaration refused while `refuse` counts,
    /// once `allow` more have passed.
    fn refused(&self) -> Result<()> {
        #[cfg(test)]
        if self
            .allow
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_err()
            && self
                .refuse
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
        {
            return Err(zenoh("refused for the test"));
        }
        Ok(())
    }

    /// Declares again, or repairs, each slot due (see [`Slot::due`]),
    /// unless the session is cut off. Nothing is awaited with the lock
    /// held, and the network calls run on the blocking pool (#1047): zenoh
    /// 1.10.1 makes them in place, and one waiting for room behind a slow
    /// link would hold a runtime worker, and with it the runtime's timers,
    /// for as long.
    async fn check(&self) {
        let now = since(self.base);
        let moved = self.moved.load(Ordering::SeqCst);
        // A slot with no horizon is due only beside one that has.
        let any = lock(&self.slots).iter().any(|s| s.due(now, moved, None));
        if !any || crate::presence::cut_off(&self.session).await {
            return;
        }
        // The due slots' acts, taken under the lock: a slot declared again
        // holds no subscriber meanwhile, so `undeclare` awaits nothing.
        let acts: Vec<Act> = {
            let moved = self.moved.load(Ordering::SeqCst);
            let mut slots = lock(&self.slots);
            let followed = slots
                .iter()
                .filter(|s| s.subscriber.is_some())
                .filter_map(|s| s.silence().filter(|&d| s.quiet_for(now, moved, d)))
                .min();
            slots
                .iter_mut()
                .filter(|s| s.due(now, moved, followed))
                .map(|s| s.act(now))
                .collect()
        };
        for act in acts {
            let done = self
                .perform(act.key.clone(), &act.callback, &act.heard, act.what)
                .await;
            let unwatched = {
                let mut slots = lock(&self.slots);
                match slots.iter_mut().find(|s| s.id == act.id) {
                    // Undeclared meanwhile: what was declared goes.
                    None => done.ok(),
                    Some(s) => {
                        self.store(s, done);
                        None
                    }
                }
            };
            if let Some(fresh) = unwatched {
                let key = act.key;
                tokio::task::spawn_blocking(move || {
                    if let Err(e) = fresh.subscriber().undeclare().wait() {
                        tracing::debug!(key = %key, "undeclaring an unwatched subscriber: {e}");
                    }
                });
            }
        }
    }

    /// One act's network calls, on the blocking pool.
    async fn perform(
        &self,
        key: OwnedKeyExpr,
        callback: &Callback,
        heard: &Arc<AtomicU64>,
        what: What,
    ) -> Result<Done> {
        self.refused()?;
        let (session, callback, heard, base) = (
            self.session.clone(),
            Arc::clone(callback),
            Arc::clone(heard),
            self.base,
        );
        tokio::task::spawn_blocking(move || match what {
            What::Declare { old } => {
                // Break, then make (the module doc): the old subscriber,
                // and with it the session's declaration of its key
                // expression, go first.
                if let Some(old) = old
                    && let Err(e) = old.undeclare().wait()
                {
                    tracing::debug!(key = %key, "undeclaring the silent subscriber: {e}");
                }
                subscribe(&session, &key, &callback, &heard, base).map(Done::Declared)
            }
            What::Repair(on) => repair(&session, &on).map(|s| Done::Repaired(on.to_string(), s)),
        })
        .await
        .unwrap_or_else(|e| Err(zenoh(e)))
    }

    /// An act's outcome, into its slot: the subscriber or the repair, or a
    /// failure counted, warned of at most once a horizon, and retried (a
    /// declaration at the next check, the slot holding none; a repair at
    /// the next time due).
    fn store(&self, slot: &mut Slot, done: Result<Done>) {
        let mut totals = lock(&self.totals);
        match done {
            Ok(Done::Declared(fresh)) => {
                slot.subscriber = Some(fresh);
                slot.counts.redeclared += 1;
                totals.redeclared += 1;
                tracing::info!(
                    key = %slot.key,
                    silence = ?slot.silence(),
                    "declared a silent subscription again (core §8.5)"
                );
            }
            Ok(Done::Repaired(on, fresh)) => {
                tracing::info!(
                    key = %slot.key,
                    repair = %on,
                    "declared a repair subscription, the subscription declared again and still \
                     silent (core §8.5)"
                );
                slot.repairs.push((on, fresh));
                slot.counts.repaired += 1;
                totals.repaired += 1;
            }
            Err(e) => {
                // A repair that failed is held by none, so the next time
                // due tries the same one again.
                slot.counts.failed += 1;
                totals.failed += 1;
                let now = since(self.base);
                let horizon = slot.horizon.or_else(|| lock(&self.shortest).map(|s| s / 2));
                if slot
                    .warned
                    .is_none_or(|w| horizon.is_none_or(|h| now.saturating_sub(w) >= nanos(h)))
                {
                    slot.warned = Some(now);
                    tracing::warn!(
                        key = %slot.key,
                        "declaring a silent subscription again failed, retried and warned of \
                         once a horizon ({horizon:?}): {e} (core §8.5)"
                    );
                } else {
                    tracing::debug!(key = %slot.key, "declaring a silent subscription again: {e}");
                }
            }
        }
    }
}

/// What an act declared.
enum Done {
    Declared(Subscriber<()>),
    Repaired(String, Subscriber<()>),
}

impl Done {
    fn subscriber(self) -> Subscriber<()> {
        match self {
            Done::Declared(s) | Done::Repaired(_, s) => s,
        }
    }
}

#[cfg(test)]
mod tests {
    use tracing_subscriber::layer::SubscriberExt as _;

    use super::*;

    /// Counts the warnings this module writes.
    #[derive(Clone, Default)]
    struct Warnings(Arc<AtomicU64>);

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Warnings {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let m = event.metadata();
            if *m.level() == tracing::Level::WARN && m.target() == "zenkey::redeclare" {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    impl Warnings {
        fn count(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    fn config(mode: &str) -> zenoh::Config {
        let mut c = zenoh::Config::default();
        c.insert_json5("mode", &format!("{mode:?}")).unwrap();
        c.insert_json5("scouting/multicast/enabled", "false")
            .unwrap();
        c.insert_json5("scouting/gossip/enabled", "false").unwrap();
        c
    }

    /// A router on a loopback port, and a client of it.
    async fn pair() -> (zenoh::Session, zenoh::Session) {
        let mut r = config("router");
        r.insert_json5("listen/endpoints", "[\"tcp/127.0.0.1:0\"]")
            .unwrap();
        let router = zenoh::open(r).await.unwrap();
        let ep = router
            .info()
            .locators()
            .await
            .into_iter()
            .map(|l| l.to_string())
            .find(|l| l.starts_with("tcp/127.0.0.1:"))
            .unwrap();
        let mut c = config("client");
        c.insert_json5("connect/endpoints", &format!("[{ep:?}]"))
            .unwrap();
        (router, zenoh::open(c).await.unwrap())
    }

    /// A declaration again that fails (#1047) is counted, and retried at
    /// every check while the subscription holds no subscriber; it is warned
    /// of once a horizon, not at every check. Once one succeeds, it is
    /// counted as declared again, and the subscription hears.
    ///
    /// The checks are made here, one by one, with the redeclaring task
    /// stopped, on the test's own thread, whose warnings are counted.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn a_failed_declaration_again_warns_once_a_horizon_and_is_retried() {
        let warnings = Warnings::default();
        let _guard =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(warnings.clone()));
        let (router, client) = pair().await;
        let r = Redeclaring::start(&client).await.unwrap();
        r.task.abort();
        let heard = Arc::new(AtomicU64::new(0));
        let h = Arc::clone(&heard);
        let key = OwnedKeyExpr::new("zk2/v/beacon/beacon.v1/stream/level").unwrap();
        let horizon = Duration::from_millis(300);
        r.declare(
            key.clone(),
            Some(horizon),
            Arc::new(move |_| {
                h.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .await
        .unwrap();
        // Two horizons of silence: the next check declares it again, and
        // every declaration fails from now on.
        tokio::time::sleep(2 * horizon).await;
        r.inner.refuse.store(u32::MAX, Ordering::SeqCst);
        for _ in 0..3 {
            r.inner.check().await;
        }
        assert_eq!(
            (r.failed(), r.redeclared()),
            (3, 0),
            "retried at each check"
        );
        assert_eq!(warnings.count(), 1, "warned once in a horizon");
        // A horizon on, the next failure warns again.
        tokio::time::sleep(horizon).await;
        r.inner.check().await;
        assert_eq!((r.failed(), warnings.count()), (4, 2));
        // It succeeds: declared again, and heard.
        r.inner.refuse.store(0, Ordering::SeqCst);
        r.inner.check().await;
        assert_eq!((r.failed(), r.redeclared(), warnings.count()), (4, 1, 2));
        assert_eq!(r.keys(), [key.to_string()]);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while heard.load(Ordering::SeqCst) == 0 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the subscription declared again hears"
            );
            router.put(&key, "1").await.unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// The run of a subscription that stays silent (core §8.5, #1185):
    /// declared again the first time it is due, then a repair each later
    /// time, `$*` one character further left in its last chunk, up to
    /// three, then declared again as at first. A subscription with no
    /// horizon is due beside it, and is repaired the same way. A sample
    /// heard ends the run, and the next silence declares it again first;
    /// the repairs stay, three in its life, so a second whole run declares
    /// it again each time and no repair beside it. A repair delivers
    /// nothing: the subscription's own subscriber delivers each sample
    /// once.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn a_silent_subscription_is_declared_again_then_repaired_up_to_three_times() {
        let (router, client) = pair().await;
        let r = Redeclaring::start(&client).await.unwrap();
        r.task.abort();
        let heard = Arc::new(AtomicU64::new(0));
        let h = Arc::clone(&heard);
        let key = OwnedKeyExpr::new("zk2/v/beacon/beacon.v1/stream/level").unwrap();
        let event = OwnedKeyExpr::new("zk2/v/beacon/beacon.v1/events/trip/*").unwrap();
        let horizon = Duration::from_millis(200);
        let level = r
            .declare(
                key.clone(),
                Some(horizon),
                Arc::new(move |_| {
                    h.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .await
            .unwrap();
        let trip = r.declare(event, None, Arc::new(|_| {})).await.unwrap();
        let silence = 2 * horizon + Duration::from_millis(50);
        tokio::time::sleep(silence).await;
        let step = || async {
            tokio::time::sleep(silence).await;
            r.inner.check().await;
            (r.counts(&[level]), r.counts(&[trip]))
        };
        r.inner.check().await;
        let one = |redeclared, repaired| Counts {
            redeclared,
            repaired,
            failed: 0,
        };
        assert_eq!(
            (r.counts(&[level]), r.counts(&[trip])),
            (one(1, 0), one(1, 0)),
            "declared again first, the event's beside it"
        );
        assert_eq!(step().await, (one(1, 1), one(1, 1)), "then a repair");
        assert_eq!(step().await, (one(1, 2), one(1, 2)));
        assert_eq!(step().await, (one(1, 3), one(1, 3)));
        assert_eq!(
            step().await,
            (one(2, 3), one(2, 3)),
            "past three, declared again"
        );
        assert_eq!(
            r.repair_keys(),
            [
                "zk2/v/beacon/beacon.v1/stream/level$*",
                "zk2/v/beacon/beacon.v1/stream/leve$*l",
                "zk2/v/beacon/beacon.v1/stream/lev$*el",
                "zk2/v/beacon/beacon.v1/events/trip$*/*",
                "zk2/v/beacon/beacon.v1/events/tri$*p/*",
                "zk2/v/beacon/beacon.v1/events/tr$*ip/*",
            ]
        );
        assert_eq!((r.redeclared(), r.repaired(), r.failed()), (4, 6, 0));
        // A sample is delivered once, through the subscription's own
        // subscriber; one the repairs alone match is dropped.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while heard.load(Ordering::SeqCst) == 0 {
            assert!(tokio::time::Instant::now() < deadline, "it hears");
            router.put(&key, "1").await.unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        let before = heard.load(Ordering::SeqCst);
        router.put(&key, "2").await.unwrap();
        router
            .put("zk2/v/beacon/beacon.v1/stream/levels", "3")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(heard.load(Ordering::SeqCst), before + 1, "each sample once");
        // Heard: the next silence starts the run again, declared again
        // first; and a second whole run declares no repair beside it, the
        // three being held (#1185's review): declared again each time.
        for redeclared in 3..=7 {
            assert_eq!(
                step().await.0,
                one(redeclared, 3),
                "a second run: declared again, no repair"
            );
        }
        assert_eq!(r.repair_keys().len(), 6, "no repair added");
        assert_eq!(r.repaired(), 6);
        assert!(r.undeclare(level).await.unwrap());
        assert_eq!(r.repair_keys().len(), 3, "its repairs undeclared with it");
        r.forget(&[trip]);
        assert!(r.keys().is_empty());
        assert_eq!(r.repaired(), 6, "the totals keep what was undeclared");
    }
}
