//! The presence link (#614, FL6a): zk2 presence and the lens, kept current
//! from a monitor of their own on the session **in** the namespace.
//!
//! zenwatch's pattern, carried over. The data pump ([`crate::link`]) runs
//! on the session in no namespace, and its bounded broadcast can lag on a
//! hot bus — a `Dropped(n)` there would knock a projection that shared it
//! to "lost" every time the echo pane fell behind. So presence has its own
//! monitor, watching nothing but the three token selectors
//! (`MonitorSpec::with_presence`), and this subscription drives it:
//!
//! 1. a [`PresenceFeed`] is opened — the event stream taken *before* the
//!    seed is read, so nothing between the two is missed — and its seed
//!    lands as the first [`PresenceUpdate`]. The monitor runs on a watch
//!    session of its own, and the feed reads through the session in the
//!    namespace, which holds no liveliness subscriber (#830, core §8.1,
//!    0.37): the feed refuses the monitor's own session;
//! 2. a [`LensFeed`] is opened on the same session and store, and handed to
//!    the app ([`LensHandle`]) for the nudges a decode makes;
//! 3. every batch the feed hands out is folded into the lens in order
//!    (`LensFeed::on_presence`, awaited here, so two batches can never
//!    apply out of order), and the projection, the moves and the catalog
//!    are yielded together — at most four times a second, however fast
//!    tokens move: a snapshot clones the projection, and a coalesced batch
//!    loses no move, it only carries several.
//!
//! The subscription's identity is an epoch, bumped whenever the session in
//! the namespace, its presence monitor or the contract store changes; a new
//! epoch is a new stream, and a landing from an old one is dropped by the
//! app.

use std::sync::Arc;
use std::time::Duration;

use iced::Subscription;
use zenkey_fleet::{
    BundleStore, Catalog, LensFeed, LivePresence, Monitor, PresenceFeed, PresenceScope,
    PresenceTransition, SeedOutcome,
};

use crate::message::{BusMsg, Message};

/// How often the projection may be yielded, at most: a snapshot clones it,
/// and the window ticks at 250 ms anyway.
pub const COALESCE: Duration = Duration::from_millis(250);

/// One presence landing: the projection as folded so far, what moved since
/// the last landing, and the catalog the lens holds after folding it.
#[derive(Debug, Clone)]
pub struct PresenceUpdate {
    /// The projection, snapshotted: each address up, down or unobservable,
    /// on the basis its seed and its events allow (§8.1).
    pub presence: Arc<LivePresence>,
    /// Every move since the last landing, in order; a seed's included.
    pub moved: Vec<PresenceTransition>,
    /// The lens's catalog after the moves were folded, and the generation
    /// it is at. `None` before the lens opened, or when its first read
    /// failed and presence has moved nothing since.
    pub catalog: Option<Arc<Catalog>>,
    pub generation: u64,
    /// What the feed's last seed found.
    pub seed: SeedOutcome,
}

/// The lens feed, as the app holds it: `LensFeed` carries no `Debug`, and a
/// message must.
#[derive(Clone)]
pub struct LensHandle(pub LensFeed);

impl std::fmt::Debug for LensHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LensHandle")
            .field("generation", &self.0.generation())
            .finish_non_exhaustive()
    }
}

/// The subscription's identity. Only `epoch` is hashed: a new epoch is a
/// new session, monitor or store, and restarts the stream; a redraw changes
/// nothing.
#[derive(Clone)]
pub struct PresenceKey {
    pub epoch: u64,
    /// The session in the deployment's namespace: the link's reads.
    pub session: zenoh::Session,
    /// The presence monitor, on a watch session of its own in that
    /// namespace (#830).
    pub monitor: Arc<Monitor>,
    /// The contract store the lens retrieves into.
    pub store: BundleStore,
    pub timeout: Duration,
}

impl std::hash::Hash for PresenceKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.epoch.hash(state);
    }
}

/// What woke the stream: a batch from the feed (`None` when its monitor's
/// broadcast closed), or the coalescing timer.
enum Wake {
    Batch(Option<Vec<PresenceTransition>>),
    Due,
}

/// Drive presence and the lens into iced. `run_with`'s builder is a plain
/// `fn`, so everything the stream needs rides in [`PresenceKey`].
pub fn subscribe(key: PresenceKey) -> Subscription<Message> {
    Subscription::run_with(key, |key| stream(key.clone()))
}

/// The link's stream itself: what [`subscribe`] runs, public so a live test
/// can drive it against a real deployment without a window.
pub fn stream(key: PresenceKey) -> impl iced::futures::Stream<Item = Message> {
    async_stream::stream! {
        let PresenceKey { epoch, session, monitor, store, timeout } = key;
        // The monitor runs on the namespaced session, so its keys are
        // base-relative and the projection's wire namespace is empty.
        let live = LivePresence::new(PresenceScope::all());
        let (mut feed, seeded) = match PresenceFeed::open(&session, &monitor, live, timeout).await
        {
            Ok(opened) => opened,
            Err(e) => {
                // A link wired with its reads beside its monitor (#830) reads
                // nothing: presence stays unread, and says why.
                let why = zenkey_fleet::one_line(&e);
                tracing::warn!("the presence link was not opened: {why}");
                yield Message::Bus(BusMsg::Presence(epoch, Arc::new(PresenceUpdate {
                    presence: Arc::new(LivePresence::new(PresenceScope::all())),
                    moved: Vec::new(),
                    catalog: None,
                    generation: 0,
                    seed: SeedOutcome::Failed(why),
                })));
                return;
            }
        };
        let update = |feed: &PresenceFeed, moved, lens: Option<&LensFeed>| {
            Arc::new(PresenceUpdate {
                presence: Arc::new(feed.presence().clone()),
                moved,
                catalog: lens.and_then(LensFeed::catalog),
                generation: lens.map_or(0, LensFeed::generation),
                seed: feed.last_seed().clone(),
            })
        };
        yield Message::Bus(BusMsg::Presence(epoch, update(&feed, seeded, None)));

        // The lens reads presence itself, describes every instance and
        // retrieves what their descriptors name; a read that could not be
        // made leaves it without a catalog, and every zk2 key then says
        // presence was not read (O4).
        let (lens, error) = LensFeed::open(&session, &store, timeout).await;
        if let Some(e) = error {
            tracing::warn!("the lens's presence read failed: {}", zenkey_fleet::one_line(&e));
        }
        yield Message::Bus(BusMsg::LensOpened(epoch, LensHandle(lens.clone())));
        yield Message::Bus(BusMsg::Presence(epoch, update(&feed, Vec::new(), Some(&lens))));

        let mut moved: Vec<PresenceTransition> = Vec::new();
        let mut due = tokio::time::Instant::now();
        loop {
            // Folded as they come; yielded at most once per `COALESCE`.
            // `PresenceFeed::next` is cancel-safe, so the timer winning
            // loses nothing the feed had folded.
            let wake = if moved.is_empty() {
                Wake::Batch(feed.next().await)
            } else {
                tokio::select! {
                    batch = feed.next() => Wake::Batch(batch),
                    () = tokio::time::sleep_until(due) => Wake::Due,
                }
            };
            let batch = match wake {
                Wake::Due => {
                    yield Message::Bus(BusMsg::Presence(
                        epoch,
                        update(&feed, std::mem::take(&mut moved), Some(&lens)),
                    ));
                    due = tokio::time::Instant::now() + COALESCE;
                    continue;
                }
                Wake::Batch(None) => break,
                Wake::Batch(Some(batch)) => batch,
            };
            // In order, and before the next batch is read: the lens folds
            // exactly what the projection folded.
            lens.on_presence(&batch).await;
            moved.extend(batch);
            if tokio::time::Instant::now() >= due {
                yield Message::Bus(BusMsg::Presence(
                    epoch,
                    update(&feed, std::mem::take(&mut moved), Some(&lens)),
                ));
                due = tokio::time::Instant::now() + COALESCE;
            }
        }
        // The monitor's broadcast closed: what was folded last still lands.
        if !moved.is_empty() {
            yield Message::Bus(BusMsg::Presence(epoch, update(&feed, moved, Some(&lens))));
        }
    }
}
