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
}

impl Default for MonitorSpec {
    fn default() -> Self {
        MonitorSpec {
            selectors: Vec::new(),
            liveliness: Vec::new(),
            stats_tick: Duration::from_millis(250),
            capacity: 1024,
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
        let (tx, _) = broadcast::channel(capacity.max(2));
        Arc::new(MonitorCore {
            tx,
            stats: Mutex::new(StatsTable::new()),
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

    /// Total events dropped across all lagging receivers so far.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
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

/// The wired monitor: subscribers + liveliness + tick task feeding a core.
pub struct Monitor {
    core: Arc<MonitorCore>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Monitor {
    /// Declare the spec's subscribers on `session` and start watching.
    pub async fn start(session: &Session, spec: MonitorSpec) -> Result<Monitor> {
        let core = MonitorCore::new(spec.capacity);
        let mut tasks = Vec::new();

        for selector in &spec.selectors {
            let subscriber = session
                .declare_subscriber(selector)
                .await
                .map_err(|e| anyhow!("subscribe {selector}: {e}"))?;
            let core = Arc::clone(&core);
            tasks.push(tokio::spawn(async move {
                while let Ok(sample) = subscriber.recv_async().await {
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
                }
            }));
        }

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

        Ok(Monitor { core, tasks })
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
