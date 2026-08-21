//! The bus link: a pure event pump over an **app-owned** monitor.
//!
//! # The seam, since the lazy rework (#84/#85)
//!
//! The app owns `Arc<Monitor>`; this subscription only pumps its events into
//! iced. Watch/unwatch are ordinary `Task::perform` futures from `update()` —
//! nothing in the subscription's identity changes on a watch toggle, so the
//! stream **provably never restarts** for one, and the session is never
//! churned. (The bootstrap's stream owned the monitor and re-scoped by
//! restarting the world; the runtime watch set made that obsolete.)
//!
//! # The three properties this pump preserves
//!
//! 1. **The tree is pulled, never accumulated.** On each stats tick: one
//!    `ArcSwap` load, shipped as an `Arc`. Sample events never maintain tree
//!    state — a 100 kHz bus and a 10 Hz bus cost the render loop the same.
//! 2. **~4 messages per second regardless of sample rate.** Samples coalesce
//!    into the tick; a per-sample `Message` would melt the Elm loop.
//! 3. **Backpressure terminates in visible counters.** Slow UI → bounded
//!    broadcast lags → `Dropped(n)` → `lagged`; the batch cap → `coalesced`;
//!    released watches → `keys_unwatched`. Nothing disappears silently
//!    (RFC 09 §5.1 O6).

use std::sync::Arc;

use iced::Subscription;
use zenkey_fleet::{FleetEvent, Monitor, StreamItem};

use crate::message::{BusMsg, BusTick, LinkState, Message};

/// How many samples one tick may carry to the UI. Beyond this they are counted
/// as `coalesced` rather than queued — an echo pane cannot render 10 k lines in
/// 250 ms, and an honest counter beats an unbounded `Vec`.
const BATCH_CAP: usize = 512;

/// The subscription's identity. Only `epoch` is hashed: a new epoch means a
/// new monitor (reconnect, base change) and restarts the pump; a redraw or a
/// watch toggle changes nothing and provably keeps the stream.
#[derive(Clone)]
pub struct LinkKey {
    pub monitor: Arc<Monitor>,
    pub epoch: u64,
}

impl std::hash::Hash for LinkKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.epoch.hash(state);
    }
}

/// Pump the monitor's events into iced.
///
/// Note `run_with`'s builder is a plain `fn` pointer, not a closure — nothing
/// can be captured, so everything the stream needs rides in [`LinkKey`].
pub fn subscribe(key: LinkKey) -> Subscription<Message> {
    Subscription::run_with(key, |key| pump(key.clone()))
}

fn pump(key: LinkKey) -> impl iced::futures::Stream<Item = Message> {
    async_stream::stream! {
        let monitor = key.monitor;
        yield Message::Bus(BusMsg::Link(LinkState::Pumping));

        let mut events = monitor.events();
        let mut samples = Vec::with_capacity(BATCH_CAP);
        let mut nodes = Vec::new();
        let mut seeded = Vec::new();
        let (mut lagged, mut coalesced) = (0u64, 0u64);
        // Rebuilt only on a `WatchChanged`, and shared with every tick after
        // it: the ticks in between hand out a pointer (#178).
        let mut watched: std::sync::Arc<[String]> =
            monitor.watched().await.into_iter().map(|(_, s)| s).collect();

        while let Some(item) = events.recv().await {
            match item {
                // Lag as data, not as a silent gap.
                StreamItem::Dropped(n) => lagged += n,
                StreamItem::Event(FleetEvent::Sample(s)) => {
                    if samples.len() < BATCH_CAP {
                        samples.push(s);
                    } else {
                        coalesced += 1;
                    }
                }
                StreamItem::Event(FleetEvent::NodeUp(k)) => nodes.push((k, true)),
                StreamItem::Event(FleetEvent::NodeDown(k)) => nodes.push((k, false)),
                StreamItem::Event(FleetEvent::WatchChanged) => {
                    watched = monitor.watched().await.into_iter().map(|(_, s)| s).collect();
                }
                StreamItem::Event(FleetEvent::WatchSeeded { id, coverage }) => {
                    seeded.push((id, coverage));
                }
                StreamItem::Event(FleetEvent::StatsTick) => {
                    // One lock-free load, and *no stats lock at all*: the
                    // snapshot already carries the totals and the O6 counters
                    // (`KeyTreeSnapshot`), because `build` folded them while
                    // it held the lock. Locking here to re-fold 50k entries
                    // would contend with the ingest path four times a second
                    // for numbers we were already handed.
                    let tree = monitor.tree();
                    let keys = tree.keys;
                    let keys_evicted = tree.evicted;
                    let keys_unwatched = tree.unwatched;
                    let totals = (
                        tree.root.subtree_count,
                        tree.root.subtree_bytes,
                        tree.root.subtree_rate_hz,
                    );
                    yield Message::Bus(BusMsg::Tick(Arc::new(BusTick {
                        tree,
                        samples: std::mem::take(&mut samples),
                        lagged: std::mem::take(&mut lagged),
                        coalesced: std::mem::take(&mut coalesced),
                        nodes: std::mem::take(&mut nodes),
                        keys,
                        keys_evicted,
                        keys_unwatched,
                        watched: std::sync::Arc::clone(&watched),
                        seeded: std::mem::take(&mut seeded),
                        totals,
                    })));
                }
            }
        }
        yield Message::Bus(BusMsg::Link(LinkState::Ended));
    }
}
