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
//!
//! [`Redeclaring`] is the one place it happens: the runtime's
//! [`crate::consumer::Consumer::across`] subscriptions and the fleet's
//! monitor watches both declare through it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
    /// Two horizons: the silence that declares it again.
    silence: Duration,
    callback: Callback,
    /// When it last heard a sample, or was declared: ns since `base`.
    heard: Arc<AtomicU64>,
    /// `None` between an undeclaration and a declaration that failed: the
    /// next check declares it again.
    subscriber: Option<Subscriber<()>>,
}

struct Inner {
    session: zenoh::Session,
    base: Instant,
    slots: tokio::sync::Mutex<Vec<Slot>>,
    /// The session's last transport event, ns since `base`.
    moved: Arc<AtomicU64>,
    redeclared: AtomicU64,
    next: AtomicU64,
    /// The shortest silence of any slot declared, for the tick.
    shortest: Mutex<Option<Duration>>,
    _events: zenoh::session::TransportEventsListener<()>,
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
            .finish_non_exhaustive()
    }
}

impl Drop for Redeclaring {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn since(base: Instant) -> u64 {
    u64::try_from(base.elapsed().as_nanos()).unwrap_or(u64::MAX)
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
            next: AtomicU64::new(0),
            shortest: Mutex::new(None),
            _events: events,
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
        let subscriber = self.inner.subscribe(&key, &callback, &heard).await?;
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
            silence,
            callback,
            heard,
            subscriber: Some(subscriber),
        });
        Ok(id)
    }

    /// Undeclares one slot's subscriber, acknowledged. `false` when it
    /// holds none of that id.
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

impl Inner {
    async fn subscribe(
        &self,
        key: &OwnedKeyExpr,
        callback: &Callback,
        heard: &Arc<AtomicU64>,
    ) -> Result<Subscriber<()>> {
        let (callback, heard, base) = (Arc::clone(callback), Arc::clone(heard), self.base);
        self.session
            .declare_subscriber(key.clone())
            .callback(move |sample: Sample| {
                heard.store(since(base), Ordering::SeqCst);
                callback(sample);
            })
            .await
            .map_err(zenoh)
    }

    /// Declares again each slot silent for its two horizons since the
    /// later of its last sample and the transport's last move, unless the
    /// session is cut off.
    async fn check(&self) {
        let now = since(self.base);
        let moved = self.moved.load(Ordering::SeqCst);
        let mut slots = self.slots.lock().await;
        let due: Vec<usize> = slots
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                if s.subscriber.is_none() {
                    return true;
                }
                let quiet = s.heard.load(Ordering::SeqCst).max(moved);
                let silence = u64::try_from(s.silence.as_nanos()).unwrap_or(u64::MAX);
                now.saturating_sub(quiet) >= silence
            })
            .map(|(i, _)| i)
            .collect();
        if due.is_empty() || crate::presence::cut_off(&self.session).await {
            return;
        }
        for i in due {
            let slot = &mut slots[i];
            // Break, then make (the module doc): the old subscriber, and
            // with it the session's declaration of its key expression, go
            // first.
            if let Some(old) = slot.subscriber.take()
                && let Err(e) = old.undeclare().await
            {
                tracing::debug!(key = %slot.key, "undeclaring the silent subscriber: {e}");
            }
            let heard = Arc::new(AtomicU64::new(since(self.base)));
            match self.subscribe(&slot.key, &slot.callback, &heard).await {
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
                    tracing::debug!(key = %slot.key, "declaring a silent subscription again: {e}");
                }
            }
        }
    }
}
