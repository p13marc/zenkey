//! The live monitor (issue #15): subscription multiplexing + liveliness
//! watching, fanned into a bounded broadcast of [`FleetEvent`]s, with the
//! key-tree snapshot published on a stats tick.
//!
//! The zengui contract, concretely:
//! - per-sample events feed **only** sample-shaped consumers (echo panes) —
//!   the channel is bounded, and overflow surfaces as an explicit
//!   [`StreamItem::Dropped`] count on the lagging receiver, never silently;
//! - tree/dashboard consumers redraw on [`FleetEvent::StatsTick`] by
//!   *pulling* the immutable [`KeyTreeSnapshot`] from an `ArcSwap` — a hot
//!   bus cannot melt a render loop.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use arc_swap::ArcSwap;
use tokio::sync::broadcast;
use zenoh::Session;
use zenoh::sample::SampleKind;

use crate::stats::StatsTable;
use crate::tree::KeyTreeSnapshot;

/// One observed sample, cheap to clone (the payload is zenoh's refcounted
/// buffer, not a copy — report §14's zero-copy discipline).
#[derive(Debug, Clone)]
pub struct SampleView {
    /// Full wire key, as received (this session is un-namespaced).
    pub key: String,
    pub payload: zenoh::bytes::ZBytes,
    /// The sample's declared encoding, verbatim.
    pub encoding: String,
    pub kind: SampleKind,
    /// HLC timestamp when the publisher's session stamps one.
    pub timestamp: Option<zenoh::time::Timestamp>,
}

/// What the monitor emits.
#[derive(Debug, Clone)]
pub enum FleetEvent {
    Sample(Arc<SampleView>),
    /// A liveliness token appeared (full wire key of the token).
    NodeUp(String),
    /// A liveliness token disappeared.
    NodeDown(String),
    /// The tree snapshot was rebuilt — pull it via [`Monitor::tree`].
    StatsTick,
    /// The watch set changed ([`Monitor::watch`]/[`Monitor::unwatch`]) —
    /// coverage labels should refresh; pull the set via [`Monitor::watched`].
    WatchChanged,
}

/// What to watch.
#[derive(Debug, Clone)]
pub struct MonitorSpec {
    /// Full wire selectors to subscribe to.
    pub selectors: Vec<String>,
    /// Also watch these liveliness selectors (with history: current tokens
    /// arrive on join — no separate seed GET).
    ///
    /// A list, not a single selector, because one selector cannot express the
    /// roster: `*` in the origin position never matches a verbatim service
    /// origin (RFC 03 §4 **D4**), so the fleet sweep
    /// `<base>/v1/*/state/*/alive` and `<base>/v1/@catalog/state/alive` are
    /// necessarily two entries. A dashboard that watches only the first
    /// renders "catalog dead" and "no entities" identically — the false
    /// verdict RFC 05 §3.1 forbids.
    pub liveliness: Vec<String>,
    /// Snapshot cadence.
    pub stats_tick: Duration,
    /// Broadcast capacity: bound it to what an echo pane can drain; lag is
    /// surfaced, never hidden.
    pub capacity: usize,
    /// How many distinct keys to keep statistics for. Least-recently-seen keys
    /// are dropped past this, and the drops are counted
    /// ([`MonitorCore::keys_evicted`]) — a long-running observer is bounded,
    /// and says so (RFC 09 §5.1).
    pub max_keys: usize,
}

impl Default for MonitorSpec {
    fn default() -> Self {
        MonitorSpec {
            selectors: Vec::new(),
            liveliness: Vec::new(),
            stats_tick: Duration::from_millis(250),
            capacity: 1024,
            max_keys: crate::stats::DEFAULT_MAX_KEYS,
        }
    }
}

/// The monitor's shareable core: ingest on one side, events + snapshots on
/// the other. Session wiring lives in [`Monitor`]; the core is pure and
/// deterministically testable.
pub struct MonitorCore {
    tx: broadcast::Sender<FleetEvent>,
    stats: Mutex<StatsTable>,
    tree: ArcSwap<KeyTreeSnapshot>,
    dropped: AtomicU64,
}

impl MonitorCore {
    pub fn new(capacity: usize) -> Arc<MonitorCore> {
        MonitorCore::bounded(capacity, crate::stats::DEFAULT_MAX_KEYS)
    }

    /// A core whose statistics table is bounded at `max_keys` distinct keys.
    pub fn bounded(capacity: usize, max_keys: usize) -> Arc<MonitorCore> {
        let (tx, _) = broadcast::channel(capacity.max(2));
        Arc::new(MonitorCore {
            tx,
            stats: Mutex::new(StatsTable::with_capacity(max_keys)),
            tree: ArcSwap::from_pointee(KeyTreeSnapshot::default()),
            dropped: AtomicU64::new(0),
        })
    }

    /// Ingest one sample: stats update + broadcast. Hot path — one lock, no
    /// tree work (that happens on the tick).
    pub fn ingest(&self, view: SampleView, sn: Option<u32>) {
        {
            let mut stats = self.stats.lock().expect("stats lock");
            stats.record(&view.key, view.payload.len(), sn, Instant::now());
        }
        // Send errors mean "no receiver right now" — not a failure.
        let _ = self.tx.send(FleetEvent::Sample(Arc::new(view)));
    }

    pub fn node_event(&self, key: String, up: bool) {
        let _ = self.tx.send(if up {
            FleetEvent::NodeUp(key)
        } else {
            FleetEvent::NodeDown(key)
        });
    }

    /// Rebuild the snapshot from the stats and announce it.
    pub fn tick(&self) {
        let snapshot = {
            let stats = self.stats.lock().expect("stats lock");
            KeyTreeSnapshot::build(&stats)
        };
        self.tree.store(Arc::new(snapshot));
        let _ = self.tx.send(FleetEvent::StatsTick);
    }

    /// The latest immutable snapshot (lock-free pull).
    pub fn tree(&self) -> Arc<KeyTreeSnapshot> {
        self.tree.load_full()
    }

    /// Read access to the raw stats (hz/bw commands).
    pub fn with_stats<R>(&self, f: impl FnOnce(&StatsTable) -> R) -> R {
        f(&self.stats.lock().expect("stats lock"))
    }

    /// Mutable access — watch retirement and tests.
    pub fn with_stats_mut<R>(&self, f: impl FnOnce(&mut StatsTable) -> R) -> R {
        f(&mut self.stats.lock().expect("stats lock"))
    }

    /// Keys retired from the table because their watch was released
    /// (RFC 09 §5.1 O6 — see [`crate::stats::StatsTable::unwatched`]).
    pub fn keys_unwatched(&self) -> u64 {
        self.with_stats(|s| s.unwatched())
    }

    /// Total events dropped across all lagging receivers so far.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Distinct keys dropped from the statistics table to stay within its
    /// bound. Non-zero means the key set on display is partial — report it
    /// rather than letting a shrinking tree read as a quieting bus
    /// (RFC 09 §5.1).
    pub fn keys_evicted(&self) -> u64 {
        self.with_stats(|s| s.evicted())
    }

    /// Subscribe to the event stream.
    pub fn events(self: &Arc<Self>) -> EventStream {
        EventStream {
            rx: self.tx.subscribe(),
            core: Arc::clone(self),
        }
    }
}

/// A receiver that surfaces lag as data: when this consumer falls behind the
/// bounded channel, the next `recv` yields the count of samples it missed —
/// dropped samples are never invisible (RFC 05 §3.1's honesty, applied to a
/// UI).
pub struct EventStream {
    rx: broadcast::Receiver<FleetEvent>,
    core: Arc<MonitorCore>,
}

/// An event, or how many this receiver just missed.
#[derive(Debug, Clone)]
pub enum StreamItem {
    Event(FleetEvent),
    Dropped(u64),
}

impl EventStream {
    /// `None` when the monitor stopped.
    pub async fn recv(&mut self) -> Option<StreamItem> {
        match self.rx.recv().await {
            Ok(ev) => Some(StreamItem::Event(ev)),
            Err(broadcast::error::RecvError::Lagged(n)) => {
                self.core.dropped.fetch_add(n, Ordering::Relaxed);
                Some(StreamItem::Dropped(n))
            }
            Err(broadcast::error::RecvError::Closed) => None,
        }
    }
}

/// Opaque handle naming one active watch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WatchId(u64);

struct WatchEntry {
    selector: String,
    subscriber: zenoh::pubsub::Subscriber<()>,
}

/// The wired monitor: a runtime-mutable watch set + liveliness + tick task
/// feeding a core.
///
/// **Lazy by construction** (issue #84): `start` with empty
/// `spec.selectors` declares *no data-plane subscribers at all* — only the
/// zero-payload liveliness watches and the tick. Data flows only for what
/// [`Monitor::watch`] was asked to observe, and [`Monitor::unwatch`]
/// provably undeclares (an explicit, awaited undeclaration — not a dropped
/// handle racing the network).
pub struct Monitor {
    core: Arc<MonitorCore>,
    session: Session,
    watches: tokio::sync::Mutex<std::collections::HashMap<WatchId, WatchEntry>>,
    next_watch: AtomicU64,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Monitor {
    /// Declare the spec's subscribers on `session` and start watching.
    /// `spec.selectors` are simply the *initial* watches — `[]` is the lazy
    /// start.
    pub async fn start(session: &Session, spec: MonitorSpec) -> Result<Monitor> {
        let core = MonitorCore::bounded(spec.capacity, spec.max_keys);
        let mut tasks = Vec::new();

        for liveliness_sel in &spec.liveliness {
            let subscriber = session
                .liveliness()
                .declare_subscriber(liveliness_sel)
                .history(true)
                .await
                .map_err(|e| anyhow!("liveliness subscribe {liveliness_sel}: {e}"))?;
            let core = Arc::clone(&core);
            tasks.push(tokio::spawn(async move {
                while let Ok(sample) = subscriber.recv_async().await {
                    let key = sample.key_expr().as_str().to_string();
                    core.node_event(key, sample.kind() == SampleKind::Put);
                }
            }));
        }

        {
            let core = Arc::clone(&core);
            let period = spec.stats_tick;
            tasks.push(tokio::spawn(async move {
                let mut interval = tokio::time::interval(period);
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    interval.tick().await;
                    core.tick();
                }
            }));
        }

        let monitor = Monitor {
            core,
            session: session.clone(),
            watches: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            next_watch: AtomicU64::new(0),
            tasks,
        };
        for selector in &spec.selectors {
            monitor.watch(selector).await?;
        }
        Ok(monitor)
    }

    /// Observe a selector: declares a callback subscriber feeding the core.
    ///
    /// The callback runs on zenoh's network thread and does exactly what the
    /// old per-selector task did — one stats lock, one bounded broadcast
    /// send — so a slow UI still cannot exert backpressure into the network
    /// layer beyond the channel's bound.
    pub async fn watch(&self, selector: &str) -> Result<WatchId> {
        let core = Arc::clone(&self.core);
        let subscriber = self
            .session
            .declare_subscriber(selector)
            .callback(move |sample| {
                let sn = sample.source_info().map(|si| si.source_sn());
                core.ingest(
                    SampleView {
                        key: sample.key_expr().as_str().to_string(),
                        payload: sample.payload().clone(),
                        encoding: sample.encoding().to_string(),
                        kind: sample.kind(),
                        timestamp: sample.timestamp().copied(),
                    },
                    sn,
                );
            })
            .await
            .map_err(|e| anyhow!("subscribe {selector}: {e}"))?;
        let id = WatchId(self.next_watch.fetch_add(1, Ordering::Relaxed));
        self.watches.lock().await.insert(
            id,
            WatchEntry {
                selector: selector.to_string(),
                subscriber,
            },
        );
        let _ = self.core.tx.send(FleetEvent::WatchChanged);
        Ok(id)
    }

    /// Stop observing: undeclares the subscriber (awaited to completion — the
    /// teardown is acknowledged, not racing a drop), then retires statistics
    /// for keys no remaining watch covers. Retired keys are **counted**
    /// ([`crate::stats::StatsTable::unwatched`]): a shrinking key set must
    /// never read as a quieting bus (RFC 09 §5.1 O6).
    pub async fn unwatch(&self, id: WatchId) -> Result<()> {
        let entry = {
            let mut watches = self.watches.lock().await;
            watches
                .remove(&id)
                .ok_or_else(|| anyhow!("unknown watch id {id:?}"))?
        };
        entry
            .subscriber
            .undeclare()
            .await
            .map_err(|e| anyhow!("undeclare {}: {e}", entry.selector))?;
        let kept: Vec<String> = {
            let watches = self.watches.lock().await;
            watches.values().map(|w| w.selector.clone()).collect()
        };
        self.core.with_stats_mut(|stats| {
            stats.retire_unwatched(&entry.selector, &kept);
        });
        self.core.tick();
        let _ = self.core.tx.send(FleetEvent::WatchChanged);
        Ok(())
    }

    /// The active watch set.
    pub async fn watched(&self) -> Vec<(WatchId, String)> {
        let watches = self.watches.lock().await;
        let mut v: Vec<(WatchId, String)> = watches
            .iter()
            .map(|(id, w)| (*id, w.selector.clone()))
            .collect();
        v.sort();
        v
    }

    pub fn core(&self) -> &Arc<MonitorCore> {
        &self.core
    }

    pub fn events(&self) -> EventStream {
        self.core.events()
    }

    pub fn tree(&self) -> Arc<KeyTreeSnapshot> {
        self.core.tree()
    }

    /// Stop watching. Equivalent to dropping the monitor — kept as an explicit
    /// verb for call sites that want to say so.
    pub fn stop(self) {
        drop(self);
    }
}

/// Dropping a monitor stops it: the ingest tasks are aborted and the
/// subscribers undeclare.
///
/// This is not a nicety. A `JoinHandle` merely *detaches* on drop, so without
/// this impl every monitor that goes out of scope leaks a live subscriber and
/// its ingest task for the lifetime of the session. `zenctl` never noticed —
/// it calls [`Monitor::stop`] once and exits — but a GUI re-scopes its
/// subscription whenever the user changes what they are watching, dropping and
/// rebuilding the monitor each time.
impl Drop for Monitor {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(key: &str, len: usize) -> SampleView {
        SampleView {
            key: key.to_string(),
            payload: zenoh::bytes::ZBytes::from(vec![0u8; len]),
            encoding: "zenoh/bytes".to_string(),
            kind: SampleKind::Put,
            timestamp: None,
        }
    }

    #[tokio::test]
    async fn events_flow_and_snapshots_rebuild_on_tick() {
        let core = MonitorCore::new(8);
        let mut events = core.events();
        core.ingest(view("zs/v1/h-a/telemetry/x/m", 4), None);
        core.tick();

        let Some(StreamItem::Event(FleetEvent::Sample(s))) = events.recv().await else {
            panic!("expected sample");
        };
        assert_eq!(s.key, "zs/v1/h-a/telemetry/x/m");
        assert_eq!(s.payload.len(), 4);
        let Some(StreamItem::Event(FleetEvent::StatsTick)) = events.recv().await else {
            panic!("expected tick");
        };
        let snap = core.tree();
        assert_eq!(snap.keys, 1);
        assert_eq!(snap.root.subtree_count, 1);
    }

    /// The bounded-channel honesty contract: a lagging receiver is told how
    /// many it missed — never a silent gap.
    #[tokio::test]
    async fn overflow_surfaces_as_dropped_counts() {
        let core = MonitorCore::new(2);
        let mut slow = core.events();
        for i in 0..10 {
            core.ingest(view(&format!("zs/v1/h-a/telemetry/x/m{i}"), 1), None);
        }
        let Some(StreamItem::Dropped(n)) = slow.recv().await else {
            panic!("expected a dropped count first");
        };
        assert!(n >= 8, "missed at least 8, reported {n}");
        assert_eq!(core.dropped(), n);
        // The stream then resumes with the retained tail.
        let Some(StreamItem::Event(FleetEvent::Sample(_))) = slow.recv().await else {
            panic!("expected a sample after the gap report");
        };
    }
}
