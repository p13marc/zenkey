//! The whole event vocabulary, in one file so the async/sync boundary is
//! readable at a glance.

use std::sync::Arc;

use zenkey_fleet::{DiscoveredBase, KeyTreeSnapshot, SampleView, SliceSet};

use crate::scope::ScopePreset;

/// Everything that can move the app.
///
/// `Clone` is required by iced's widget callbacks. Every payload here is
/// cheap to clone: `Session`, `Arc<…>` and `ZBytes` are all refcounted.
#[derive(Debug, Clone)]
pub enum Message {
    /// One coalesced batch from the bus link. See [`BusTick`].
    Tick(Arc<BusTick>),
    /// The link changed state.
    Link(LinkState),
    /// A session was opened (or could not be).
    SessionOpened(Result<zenoh::Session, String>),
    /// The base sweep finished. An empty list is *not* a verdict (RFC 05 §3.1).
    BasesDiscovered(Result<Vec<DiscoveredBase>, String>),
    /// Registry slices arrived, from the bus or from `--registry` dirs.
    SlicesLoaded(Result<Arc<SliceSet>, String>),

    BaseSelected(String),
    ScopeSelected(ScopePreset),
    ToggleNode(String),
    SelectKey(Option<String>),
    EchoFilterChanged(String),
    ClearEcho,
    Reconnect,
}

/// What the link is doing, so the UI never has to infer it from emptiness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkState {
    Connecting,
    /// Subscribed and receiving. Carries what it is actually watching, so the
    /// status strip can state coverage rather than imply it.
    Watching {
        selectors: Vec<String>,
    },
    /// The stream ended; `Subscription::run_with` will restart it.
    Ended,
    Failed(String),
}

/// One stats-tick's worth of bus activity, coalesced.
///
/// The monitor ticks at 250 ms, so this is ~4 messages/second **regardless of
/// sample rate** — the single most important perf property of the bridge. A
/// per-sample `Message` would melt the Elm loop on any real bus.
#[derive(Debug)]
pub struct BusTick {
    /// Pulled from the monitor's `ArcSwap` on the tick — never accumulated
    /// from samples. This is what makes a hot bus unable to melt the render
    /// loop (`zenkey-fleet/src/tree.rs`).
    pub tree: Arc<KeyTreeSnapshot>,
    /// Samples observed during this tick, capped.
    pub samples: Vec<Arc<SampleView>>,
    /// Samples the bounded broadcast dropped before we saw them.
    pub lagged: u64,
    /// Samples *we* dropped because the per-tick batch was full. Distinct from
    /// `lagged`: this is our own cap, and it is reported rather than hidden.
    pub coalesced: u64,
    /// Liveliness transitions: `(token key, is_up)`.
    pub nodes: Vec<(String, bool)>,
    /// Distinct keys the monitor has seen since it started.
    pub keys: usize,
    /// `(samples, bytes, rate_hz)` across everything watched.
    pub totals: (u64, u64, f64),
}
