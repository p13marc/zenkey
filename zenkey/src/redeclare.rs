//! Declaring a subscription again after silence (core §8.5, `link.v2`
//! §2.21; #988 L4).
//!
//! zenoh 1.10.1 never repairs a declaration its medium lost while the
//! transport stays up: a subscriber whose declaration a radio dropped is
//! never heard of again by its router, and nothing at either end says so
//! (core Appendix B). A reader across a constrained face whose medium
//! repairs no lost frame (`arq = false`) therefore declares a subscription
//! again when it has heard nothing on it for two of its horizons
//! ([`zenkey_model::link::SILENT_HORIZONS`]) while its session's transport
//! stayed up.
//!
//! - **Its horizon** is that of what it hears
//!   ([`zenkey_model::link::redeclare_horizon`],
//!   [`zenkey_model::link::redeclare_heartbeat_horizon`]): a subscription to what
//!   has none is declared once, as any other.
//! - **The silence** counts from the later of the subscription's
//!   declaration, the last sample it heard (any sample, delivered or
//!   refused: the declaration stands), and its session's last transport
//!   event. A transport that opens again declares the session's
//!   subscriptions anew, so a silence across one says nothing of a lost
//!   declaration.
//! - **Break, then make.** The old subscriber is undeclared first, then the
//!   same key expression declared anew with the same callback: zenoh 1.10.1
//!   joins a subscriber on the key of one the session holds to it, and
//!   declares nothing (`register_subscriber`). A sample put in the instant
//!   between is missed; the subscription has heard nothing for two
//!   horizons.
//! - **What it repairs** is a lost `DeclareSubscriber`. A lost
//!   `DeclareKeyExpr` it cannot: the session's runtime declares a prefix
//!   toward its router once a transport and keeps the mapping, so the new
//!   subscriber rides the scope the router never learned (core Appendix
//!   B). Only a transport that opens again repairs that, or link ARQ
//!   keeps it.
//! - **Not through a session cut off**: it holds no link
//!   ([`crate::presence::cut_off`]), so a declaration goes nowhere, and the
//!   transport that opens again declares everything anyway.
//! - **A declaration that fails** leaves the subscription with no
//!   subscriber, declared again at the next check. Each failure is counted
//!   ([`Redeclaring::failed`]) and warned of, at most once a horizon for
//!   each subscription, so a watch that stays dead says so (#1047).
//! - **No lock across the network** (#1047): the checks take the due
//!   subscriptions' subscribers under the lock, undeclare and declare with
//!   it released, on the blocking pool, and take it again to store what
//!   they declared. zenoh 1.10.1 declares in place, so a declaration that
//!   waits for room behind a slow link would otherwise hold a runtime
//!   worker, and the runtime's timers with it. An
//!   [`Redeclaring::undeclare`] meanwhile returns at once, and the
//!   subscriber declared for the slot it removed is undeclared, not
//!   stored.
//!
//! [`Redeclaring`] is the one place it happens: the runtime's
//! [`crate::consumer::Consumer::across`] subscriptions and the fleet's
//! monitor watches both declare through it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
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

/// One subscription that is declared again after silence.
struct Slot {
    id: SlotId,
    key: OwnedKeyExpr,
    /// What it hears: a failed declaration warns at most once in one.
    horizon: Duration,
    /// Two horizons: the silence that declares it again.
    silence: Duration,
    callback: Callback,
    /// When it last heard a sample, or was declared: ns since `base`.
    heard: Arc<AtomicU64>,
    /// `None` between an undeclaration and a declaration that failed, and
    /// while a check declares it again: the next check declares it again.
    subscriber: Option<Subscriber<()>>,
    /// When a failed declaration last warned: ns since `base`.
    warned: Option<u64>,
}

impl Slot {
    /// Whether a check declares it again: it holds no subscriber, or has
    /// heard nothing for its silence since the later of its last sample and
    /// the transport's last move.
    fn due(&self, now: u64, moved: u64) -> bool {
        if self.subscriber.is_none() {
            return true;
        }
        let quiet = self.heard.load(Ordering::SeqCst).max(moved);
        now.saturating_sub(quiet) >= nanos(self.silence)
    }
}

/// A due slot's declaration in flight, made with the lock released.
struct Redeclare {
    id: SlotId,
    key: OwnedKeyExpr,
    callback: Callback,
    /// Its old subscriber, undeclared first.
    old: Option<Subscriber<()>>,
}

struct Inner {
    session: zenoh::Session,
    base: Instant,
    slots: tokio::sync::Mutex<Vec<Slot>>,
    /// The session's last transport event, ns since `base`.
    moved: Arc<AtomicU64>,
    redeclared: AtomicU64,
    /// Declarations again that failed, each retried at the next check.
    failed: AtomicU64,
    next: AtomicU64,
    /// The shortest silence of any slot declared, for the tick.
    shortest: Mutex<Option<Duration>>,
    _events: zenoh::session::TransportEventsListener<()>,
    /// How many declarations to refuse, the tests' failure.
    #[cfg(test)]
    refuse: std::sync::atomic::AtomicU32,
}

/// The subscriptions of one reader across a face that are declared again
/// after silence (see the module doc). Dropping it stops the checks and
/// undeclares every subscriber it holds.
pub struct Redeclaring {
    inner: Arc<Inner>,
    task: tokio::task::JoinHandle<()>,
}

impl std::fmt::Debug for Redeclaring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Redeclaring")
            .field("redeclared", &self.redeclared())
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
            slots: tokio::sync::Mutex::new(Vec::new()),
            moved,
            redeclared: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            next: AtomicU64::new(0),
            shortest: Mutex::new(None),
            _events: events,
            #[cfg(test)]
            refuse: std::sync::atomic::AtomicU32::new(0),
        });
        let weak = Arc::downgrade(&inner);
        let task = tokio::spawn(async move {
            loop {
                let Some(inner) = weak.upgrade() else { return };
                let tick = inner
                    .shortest
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .map_or(TICK_MAX, |s| (s / 10).clamp(TICK_MIN, TICK_MAX));
                inner.check().await;
                drop(inner);
                tokio::time::sleep(tick).await;
            }
        });
        Ok(Redeclaring { inner, task })
    }

    /// Declares a subscriber to `key` with `callback`, declared again
    /// whenever it hears nothing for two `horizon`s while the session's
    /// transport stays up.
    pub async fn declare(
        &self,
        key: OwnedKeyExpr,
        horizon: Duration,
        callback: Callback,
    ) -> Result<SlotId> {
        let silence = zenkey_model::link::FarSide::silence(horizon);
        let heard = Arc::new(AtomicU64::new(since(self.inner.base)));
        let subscriber = self.inner.subscribe(&key, &callback, &heard)?;
        let id = SlotId(self.inner.next.fetch_add(1, Ordering::Relaxed));
        {
            let mut shortest = self
                .inner
                .shortest
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *shortest = Some(shortest.map_or(silence, |s| s.min(silence)));
        }
        self.inner.slots.lock().await.push(Slot {
            id,
            key,
            horizon,
            silence,
            callback,
            heard,
            subscriber: Some(subscriber),
            warned: None,
        });
        Ok(id)
    }

    /// Undeclares one slot's subscriber, acknowledged. `false` when it
    /// holds none of that id. A check declaring the slot again meanwhile
    /// holds its subscriber, so this returns at once, and the check
    /// undeclares what it declared.
    pub async fn undeclare(&self, id: SlotId) -> Result<bool> {
        let slot = {
            let mut slots = self.inner.slots.lock().await;
            let Some(i) = slots.iter().position(|s| s.id == id) else {
                return Ok(false);
            };
            slots.remove(i)
        };
        if let Some(s) = slot.subscriber {
            s.undeclare().await.map_err(zenoh)?;
        }
        Ok(true)
    }

    /// How many times a silent subscription was declared again.
    #[must_use]
    pub fn redeclared(&self) -> u64 {
        self.inner.redeclared.load(Ordering::Relaxed)
    }

    /// How many times declaring a subscription again failed: each is
    /// retried at the next check, and warned of at most once a horizon.
    #[must_use]
    pub fn failed(&self) -> u64 {
        self.inner.failed.load(Ordering::Relaxed)
    }

    /// The key expressions it holds a subscriber to, in declaration order.
    pub async fn keys(&self) -> Vec<String> {
        self.inner
            .slots
            .lock()
            .await
            .iter()
            .map(|s| s.key.to_string())
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

    /// The tests' failure: a declaration refused while `refuse` counts.
    fn refused(&self) -> Result<()> {
        #[cfg(test)]
        if self
            .refuse
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(zenoh("refused for the test"));
        }
        Ok(())
    }

    /// Declares again each slot silent for its two horizons since the
    /// later of its last sample and the transport's last move, unless the
    /// session is cut off. Nothing is awaited with the lock held, and the
    /// network calls run on the blocking pool (#1047): zenoh 1.10.1 makes
    /// them in place, and one waiting for room behind a slow link would
    /// hold a runtime worker, and with it the runtime's timers, for as long.
    async fn check(&self) {
        let now = since(self.base);
        let moved = self.moved.load(Ordering::SeqCst);
        let any = self.slots.lock().await.iter().any(|s| s.due(now, moved));
        if !any || crate::presence::cut_off(&self.session).await {
            return;
        }
        // The due slots' subscribers, taken under the lock: a slot a check
        // holds has none, so `undeclare` meanwhile awaits nothing.
        let due: Vec<Redeclare> = {
            let moved = self.moved.load(Ordering::SeqCst);
            let mut slots = self.slots.lock().await;
            slots
                .iter_mut()
                .filter(|s| s.due(now, moved))
                .map(|s| Redeclare {
                    id: s.id,
                    key: s.key.clone(),
                    callback: Arc::clone(&s.callback),
                    old: s.subscriber.take(),
                })
                .collect()
        };
        for slot in due {
            let heard = Arc::new(AtomicU64::new(since(self.base)));
            let declared = match self.refused() {
                Err(e) => {
                    drop(slot.old);
                    Err(e)
                }
                Ok(()) => {
                    let (session, key, callback, h, base) = (
                        self.session.clone(),
                        slot.key.clone(),
                        Arc::clone(&slot.callback),
                        Arc::clone(&heard),
                        self.base,
                    );
                    let old = slot.old;
                    tokio::task::spawn_blocking(move || {
                        // Break, then make (the module doc): the old
                        // subscriber, and with it the session's declaration
                        // of its key expression, go first.
                        if let Some(old) = old
                            && let Err(e) = old.undeclare().wait()
                        {
                            tracing::debug!(key = %key, "undeclaring the silent subscriber: {e}");
                        }
                        subscribe(&session, &key, &callback, &h, base)
                    })
                    .await
                    .unwrap_or_else(|e| Err(zenoh(e)))
                }
            };
            let unwatched = {
                let mut slots = self.slots.lock().await;
                match slots.iter_mut().find(|s| s.id == slot.id) {
                    // Undeclared meanwhile: what was declared goes.
                    None => declared.ok(),
                    Some(s) => {
                        self.store(s, declared, heard);
                        None
                    }
                }
            };
            if let Some(fresh) = unwatched {
                let key = slot.key;
                tokio::task::spawn_blocking(move || {
                    if let Err(e) = fresh.undeclare().wait() {
                        tracing::debug!(key = %key, "undeclaring an unwatched subscriber: {e}");
                    }
                });
            }
        }
    }

    /// A declaration again's outcome, into its slot: the subscriber, or a
    /// failure counted, warned of at most once a horizon, and retried at
    /// the next check, the slot holding none.
    fn store(&self, slot: &mut Slot, declared: Result<Subscriber<()>>, heard: Arc<AtomicU64>) {
        match declared {
            Ok(fresh) => {
                slot.subscriber = Some(fresh);
                slot.heard = heard;
                self.redeclared.fetch_add(1, Ordering::Relaxed);
                tracing::info!(
                    key = %slot.key,
                    silence = ?slot.silence,
                    "declared a silent subscription again (core §8.5)"
                );
            }
            Err(e) => {
                self.failed.fetch_add(1, Ordering::Relaxed);
                let now = since(self.base);
                if slot
                    .warned
                    .is_none_or(|w| now.saturating_sub(w) >= nanos(slot.horizon))
                {
                    slot.warned = Some(now);
                    tracing::warn!(
                        key = %slot.key,
                        "declaring a silent subscription again failed, retried at every check \
                         and warned of once a horizon ({:?}): {e} (core §8.5)",
                        slot.horizon
                    );
                } else {
                    tracing::debug!(key = %slot.key, "declaring a silent subscription again: {e}");
                }
            }
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
            horizon,
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
        assert_eq!(r.keys().await, [key.to_string()]);
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
}
