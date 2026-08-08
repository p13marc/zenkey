//! The bus link: one tokio task owning a [`Monitor`], fanned into iced.
//!
//! # Where things live, and why
//!
//! The **session** lives in app state: base discovery and registry fetches are
//! one-shot tasks against it, and re-scoping a subscription must not churn the
//! session. The **monitor** lives inside the subscription: it must be created
//! in an async context and torn down when the scope changes, both of which
//! `Subscription::run_with` already does.
//!
//! That teardown is why `zenkey-fleet` grew `impl Drop for Monitor`. A
//! `JoinHandle` merely detaches on drop, so without it every re-scope would
//! leak a live subscriber for the lifetime of the session.
//!
//! # The three properties this bridge exists to preserve
//!
//! 1. **The tree is pulled, never accumulated.** On each stats tick the task
//!    does one `ArcSwap` load and ships the `Arc<KeyTreeSnapshot>`. Sample
//!    events never maintain tree state, so a 100 kHz bus and a 10 Hz bus cost
//!    the render loop exactly the same.
//! 2. **~4 messages per second regardless of sample rate.** Samples are
//!    coalesced into the tick. A per-sample `Message` would melt the Elm loop.
//! 3. **Backpressure terminates in a visible counter.** Sending blocks the
//!    task when iced is behind → the bounded broadcast lags → `Dropped(n)` →
//!    `lagged` on the next tick → the status strip. Memory never grows
//!    silently, and the loss is reported (RFC 05 §3.1).

use std::sync::Arc;
use std::time::Duration;

use iced::Subscription;
use zenkey_fleet::{FleetEvent, Monitor, MonitorSpec, StreamItem};

use crate::message::{BusTick, LinkState, Message};

/// How many samples one tick may carry to the UI. Beyond this they are counted
/// as `coalesced` rather than queued — an echo pane cannot render 10 k lines in
/// 250 ms, and an honest counter beats an unbounded `Vec`.
const BATCH_CAP: usize = 512;

/// What the link watches. `Hash` is the subscription's identity: change any
/// hashed field and iced tears the old stream down and starts a new one, which
/// is exactly how re-scoping works. The session is carried but *not* hashed —
/// a redraw must not restart a healthy stream.
#[derive(Clone)]
pub struct LinkKey {
    pub session: zenoh::Session,
    pub selectors: Vec<String>,
    pub liveliness: Vec<String>,
    /// Distinct keys the stats table may hold (RFC 09 §5.1 O6).
    pub max_keys: usize,
    /// Bumped to force a restart (a manual reconnect).
    pub epoch: u64,
}

impl std::hash::Hash for LinkKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.selectors.hash(state);
        self.liveliness.hash(state);
        self.max_keys.hash(state);
        self.epoch.hash(state);
    }
}

/// Subscribe to the bus.
///
/// Note `run_with`'s builder is a plain `fn` pointer, not a closure — nothing
/// can be captured, so everything the stream needs rides in [`LinkKey`].
pub fn subscribe(key: LinkKey) -> Subscription<Message> {
    Subscription::run_with(key, |key| stream(key.clone()))
}

fn stream(key: LinkKey) -> impl iced::futures::Stream<Item = Message> {
    async_stream::stream! {
        let selectors = key.selectors.clone();
        yield Message::Link(LinkState::Connecting);

        let spec = MonitorSpec {
            selectors: key.selectors,
            liveliness: key.liveliness,
            stats_tick: Duration::from_millis(250),
            capacity: 2048,
            max_keys: key.max_keys,
        };
        let monitor = match Monitor::start(&key.session, spec).await {
            Ok(m) => m,
            Err(e) => {
                yield Message::Link(LinkState::Failed(e.to_string()));
                return;
            }
        };
        yield Message::Link(LinkState::Watching { selectors });

        let mut events = monitor.events();
        let mut samples = Vec::with_capacity(BATCH_CAP);
        let mut nodes = Vec::new();
        let (mut lagged, mut coalesced) = (0u64, 0u64);

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
                StreamItem::Event(FleetEvent::StatsTick) => {
                    let (keys, totals, keys_evicted) = monitor
                        .core()
                        .with_stats(|s| (s.len(), s.totals(), s.evicted()));
                    yield Message::Tick(Arc::new(BusTick {
                        // One lock-free load. The tree is never rebuilt here.
                        tree: monitor.tree(),
                        samples: std::mem::take(&mut samples),
                        lagged: std::mem::take(&mut lagged),
                        coalesced: std::mem::take(&mut coalesced),
                        nodes: std::mem::take(&mut nodes),
                        keys,
                        keys_evicted,
                        totals,
                    }));
                }
            }
        }
        yield Message::Link(LinkState::Ended);
        // `monitor` drops here; its Drop aborts the subscriber tasks.
    }
}
