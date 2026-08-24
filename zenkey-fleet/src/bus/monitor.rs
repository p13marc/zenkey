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

use crate::model::retain::{Retention, RetentionBudget, RetentionStats};
use crate::model::stats::StatsTable;
use crate::model::tree::KeyTreeSnapshot;

/// The publisher a sample came from, when its session attaches SourceInfo
/// — the same signal the gap counter reads, surfaced (#120). All `Copy`:
/// carrying it costs no allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleSource {
    /// The publishing session's Zenoh id.
    pub zid: zenoh::config::ZenohId,
    /// The entity id within that session.
    pub eid: u32,
    /// The per-entity sequence number — what the gap counter diffs.
    pub sn: u32,
}

/// Who stamped a sample's HLC (issue #213, RFC 09 §5.1 **O7**).
///
/// zenoh stamps at the **first node with timestamping enabled**, not
/// necessarily at the publisher: `timestamping.enabled` is mode-dependent and
/// documented as "whether data messages should be timestamped *if not
/// already*", so on a fleet configured `{ router: true }` an unstamped sample
/// picks up a **router's** clock on the way past. A tool that calls that
/// "the publisher's HLC" is reporting a different measurement than the one it
/// names, and every latency built on it inherits the mislabel.
///
/// The comparison is exact rather than heuristic: a `Timestamp`'s id *is* a
/// [`zenoh::time::TimestampId`] (= `uhlc::ID`), a `ZenohId` is a transparent
/// newtype over the same type, and zenoh provides the conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StampProvenance {
    /// The publishing session stamped it: this is the publisher's own clock.
    SelfStamped,
    /// Another node stamped it — commonly a router with timestamping enabled.
    /// The HLC is *that* node's clock, and a latency computed from it measures
    /// stamper → observer, not publisher → observer.
    Foreign { stamper: zenoh::time::TimestampId },
    /// Stamped, but the sample carried no `SourceInfo`, so there is nothing to
    /// compare the stamper against. Not "foreign" — unknown (O4).
    Unattributable { stamper: zenoh::time::TimestampId },
}

impl StampProvenance {
    /// The stamping node, when the sample said who it was.
    pub fn stamper(self) -> Option<zenoh::time::TimestampId> {
        match self {
            StampProvenance::SelfStamped => None,
            StampProvenance::Foreign { stamper } | StampProvenance::Unattributable { stamper } => {
                Some(stamper)
            }
        }
    }

    /// Judge one sample's stamp against the publisher it claims to come from.
    fn of(timestamp: &zenoh::time::Timestamp, source: Option<SampleSource>) -> StampProvenance {
        let stamper = *timestamp.get_id();
        match source {
            Some(src) if stamper == zenoh::time::TimestampId::from(src.zid) => {
                StampProvenance::SelfStamped
            }
            Some(_) => StampProvenance::Foreign { stamper },
            None => StampProvenance::Unattributable { stamper },
        }
    }
}

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
    /// HLC timestamp, when the sample carried one.
    ///
    /// Absent is common — nothing on the path had timestamping enabled — and
    /// absence must never be defaulted to an arrival time. This is *not*
    /// necessarily the publisher's clock: see [`SampleView::stamped_by`] for
    /// whose it is. [`SampleView::received`] is ours, and the two are never
    /// mixed (a consumer plotting a time axis states which one it plotted).
    pub timestamp: Option<zenoh::time::Timestamp>,
    /// Who stamped [`SampleView::timestamp`] — `None` exactly when it is.
    ///
    /// The stamper's identity rode on every sample all along and was thrown
    /// away, which is how "the publisher's HLC" survived as a description of
    /// a number that is often a router's (issue #213).
    pub stamped_by: Option<StampProvenance>,
    /// The sample's attachment, when it carried one — zenoh's refcounted
    /// buffer, like the payload, so retaining it is a refcount bump and the
    /// per-sample allocation floor stands (`docs/zero-copy.md` §4).
    ///
    /// `None` means the sample carried none: an attachment is a wire fact,
    /// not a decode, and what arrived is a fact to show (#117).
    pub attachment: Option<zenoh::bytes::ZBytes>,
    /// The wire's actual QoS axes (#120) — always present: zenoh stamps
    /// every sample with them, defaults included. A registry *declares* a
    /// profile; these are what actually rode, and the two can disagree —
    /// which is exactly what a frontend renders. All `Copy`.
    ///
    /// Deliberately absent: SHM-vs-raw buffer provenance. zenoh 1.9's
    /// public API does not expose it on a received sample, and chasing it
    /// through `zenoh::internal` is the dependency this crate refuses
    /// (nuze's decoder is the cautionary tale).
    pub priority: zenoh::qos::Priority,
    pub congestion_control: zenoh::qos::CongestionControl,
    pub reliability: zenoh::qos::Reliability,
    pub express: bool,
    /// The publishing entity, when SourceInfo rode the sample. `None` is
    /// "the publisher's session does not attach it" — common, and not a
    /// defect.
    pub source: Option<SampleSource>,
    /// Arrival, on **this observer's** monotonic clock — always available,
    /// never wall-clock, and never a claim about when the sample was produced.
    ///
    /// Stamped per sample rather than per batch so a consumer that coalesces
    /// (zengui ticks at 250 ms) can still space a 5 Hz key's samples truthfully
    /// instead of collapsing a tick's worth onto one instant.
    pub received: Instant,
}

impl SampleView {
    /// Build a view from a received sample.
    ///
    /// The one place this conversion lives. It was written out by hand twice —
    /// here and on the seed/GET-reply path — and the second copy is exactly
    /// the kind of site that silently misses a new field: adding stamp
    /// provenance to only one of them would have left seeded samples
    /// unattributed for no stated reason (issue #213).
    pub fn of(sample: &zenoh::sample::Sample) -> SampleView {
        let source = sample.source_info().map(|si| SampleSource {
            zid: si.source_id().zid(),
            eid: si.source_id().eid(),
            sn: si.source_sn(),
        });
        let timestamp = sample.timestamp().copied();
        SampleView {
            key: sample.key_expr().as_str().to_string(),
            payload: sample.payload().clone(),
            encoding: sample.encoding().to_string(),
            kind: sample.kind(),
            stamped_by: timestamp.as_ref().map(|t| StampProvenance::of(t, source)),
            timestamp,
            attachment: sample.attachment().cloned(),
            priority: sample.priority(),
            congestion_control: sample.congestion_control(),
            reliability: sample.reliability(),
            express: sample.express(),
            source,
            // Arrival, not production: a sample is *received* now, however old
            // the value it carries is. The HLC above is the only thing that
            // speaks for when it was produced, and it is often absent.
            received: Instant::now(),
        }
    }

    /// Whether the wire's actual axes match a declared profile (RFC 04 §3)
    /// — the declared-vs-observed comparison nobody else in the field can
    /// render, because nobody else holds a registry that declares QoS.
    pub fn qos_matches(&self, profile: zenkey::qos::QosProfile) -> bool {
        self.priority == profile.priority()
            && self.congestion_control == profile.congestion_control()
            && self.reliability == profile.reliability()
            && self.express == profile.express()
    }
}

/// What the monitor emits.
///
/// Deliberately **no matching variant** (#38/#80 adoption note): zenoh 1.9
/// has matching listeners on publishers and queriers only — a subscriber
/// cannot ask "does anyone publish what I watch", so the monitor's watches
/// have nothing honest to report here. Matching lives on the write facade's
/// [`crate::Publication`] and on [`crate::RepeatingQuery`], the two entities
/// this process declares that zenoh can answer for.
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
    /// A seeded watch's seed phase resolved (issue #92): both seed paths of
    /// [`Monitor::watch_seeded`] finished, with what each contributed.
    /// Everything on this watch after this event is live-only — "seeding"
    /// panes flip to "live" here, never on a guess.
    WatchSeeded {
        id: WatchId,
        coverage: crate::report::SeedCoverage,
    },
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
            max_keys: crate::model::bounded::DEFAULT_MAX_KEYS,
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
    /// The retained window (#217): recent samples off the same ingest path,
    /// bounded by bytes *and* age. Its own mutex, never held with the stats
    /// lock — the two bounds are different facts and different contention.
    retain: Mutex<Retention>,
}

impl MonitorCore {
    pub fn new(capacity: usize) -> Arc<MonitorCore> {
        MonitorCore::bounded(capacity, crate::model::bounded::DEFAULT_MAX_KEYS)
    }

    /// A core whose statistics table is bounded at `max_keys` distinct keys.
    pub fn bounded(capacity: usize, max_keys: usize) -> Arc<MonitorCore> {
        let (tx, _) = broadcast::channel(capacity.max(2));
        Arc::new(MonitorCore {
            tx,
            stats: Mutex::new(StatsTable::with_capacity(max_keys)),
            tree: ArcSwap::from_pointee(KeyTreeSnapshot::default()),
            dropped: AtomicU64::new(0),
            retain: Mutex::new(Retention::new(RetentionBudget::default())),
        })
    }

    /// Ingest one sample: stats update + retention + broadcast. Hot path —
    /// two short locks, no tree work (that happens on the tick).
    pub fn ingest(&self, view: SampleView, sn: Option<u32>) {
        self.ingest_at(
            Arc::new(view),
            sn,
            Instant::now(),
            std::time::SystemTime::now(),
        );
    }

    /// [`MonitorCore::ingest`] with **both** clocks injected (#217).
    ///
    /// Replay rebuilds feed this with the **capture clock** — the row's `t`
    /// offset from the load epoch, on both axes: `now` for the monotonic
    /// fold, `wall` for the skewed-latency subtraction — rather than the
    /// live clocks, which is what makes a rebuild deterministic down to the
    /// EWMA rates and the latency window: the same rows at the same instants
    /// fold to bit-identical statistics, however fast the rebuild loop runs.
    /// (Before `wall` was threaded, the latency read the *live* wall clock
    /// even under an injected `now`, so a rebuild folded arrival-time
    /// garbage — deep-review D2.)
    pub fn ingest_at(
        &self,
        view: Arc<SampleView>,
        sn: Option<u32>,
        now: Instant,
        wall: std::time::SystemTime,
    ) {
        {
            // Observed *skewed* latency (#119): our wall clock minus the
            // sample's HLC — both halves this crate deliberately never mixes
            // elsewhere, subtracted here on purpose and labeled as containing
            // clock skew. Unstamped samples pass None and are counted, not
            // defaulted (no latency ≠ zero latency).
            //
            // The class rides with the number (#213), because the HLC is not
            // always the publisher's: whichever node stamped it is the one
            // this measures from, and three populations that mean different
            // things must not land in one median.
            let latency = view.timestamp.as_ref().map(|t| {
                let stamped = t.get_time().to_system_time();
                let us = match wall.duration_since(stamped) {
                    Ok(d) => i64::try_from(d.as_micros()).unwrap_or(i64::MAX),
                    // The stamping node's clock is ahead of ours: negative,
                    // and shown as such — that *is* the skew evidence.
                    Err(e) => -i64::try_from(e.duration().as_micros()).unwrap_or(i64::MAX),
                };
                let class = match view.stamped_by {
                    Some(StampProvenance::SelfStamped) => {
                        crate::model::stats::StampClass::SelfStamped
                    }
                    Some(StampProvenance::Foreign { .. }) => {
                        crate::model::stats::StampClass::Foreign
                    }
                    _ => crate::model::stats::StampClass::Unattributable,
                };
                (us, class)
            });
            let stamper = view.stamped_by.and_then(StampProvenance::stamper);
            let mut stats = self.stats.lock().expect("stats lock");
            stats.record(&view.key, view.payload.len(), sn, now, latency, stamper);
        }
        // The retained window (#217): an Arc clone, off the same path the
        // broadcast rides — the ring can never disagree with what was
        // ingested, and retaining costs a refcount bump, not a copy.
        self.retain
            .lock()
            .expect("retain lock")
            .push(Arc::clone(&view), now);
        // Send errors mean "no receiver right now" — not a failure.
        let _ = self.tx.send(FleetEvent::Sample(view));
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
    /// (RFC 09 §5.1 O6 — see [`crate::model::stats::StatsTable::unwatched`]).
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

    /// The retained window, oldest first (#217): `Arc` clones of every
    /// sample still inside both retention budgets. This is what the GUI's
    /// retained scrub rebuilds panes from, and what "save window as `.zrec`"
    /// writes — the same rows either way.
    ///
    /// Covers only **watched** keys by construction: the ring sits on the
    /// ingest path, and nothing unwatched is ever ingested. A consumer that
    /// presents this window MUST say so (RFC 09 §5.1 O5 — a retained window
    /// over three watches is not a retained window over the bus).
    pub fn retained(&self) -> Vec<Arc<SampleView>> {
        self.retain
            .lock()
            .expect("retain lock")
            .snapshot(Instant::now())
    }

    /// The retained window's account of itself: budget in force, what it
    /// holds, and what each bound cost — `evicted` (byte budget) apart from
    /// `expired` (age), both apart from [`MonitorCore::dropped`],
    /// [`MonitorCore::keys_evicted`] and [`MonitorCore::keys_unwatched`]
    /// (RFC 09 §5.1 O6; v1.18 R1 forbids folding the kinds).
    pub fn retention(&self) -> RetentionStats {
        self.retain
            .lock()
            .expect("retain lock")
            .stats(Instant::now())
    }

    /// Change the retention budget in force; applied from the next push or
    /// read. The default ([`RetentionBudget::default`]) is 64 MiB / 2 min.
    pub fn set_retention_budget(&self, budget: RetentionBudget) {
        self.retain.lock().expect("retain lock").set_budget(budget);
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
    /// The seed task, while a seeded watch's seed phase is still running.
    /// Aborted on [`Monitor::unwatch`] so a released watch cannot keep
    /// ingesting seed replies. (A dropped *monitor* lets it run out — it is
    /// bounded by the seed timeout and feeds a core nobody reads.)
    seed_task: Option<tokio::task::JoinHandle<()>>,
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

impl std::fmt::Debug for Monitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Monitor").finish_non_exhaustive()
    }
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
                let view = SampleView::of(&sample);
                let sn = view.source.map(|s| s.sn);
                core.ingest(view, sn);
            })
            .await
            .map_err(|e| anyhow!("subscribe {selector}: {e}"))?;
        let id = WatchId(self.next_watch.fetch_add(1, Ordering::Relaxed));
        self.watches.lock().await.insert(
            id,
            WatchEntry {
                selector: selector.to_string(),
                subscriber,
                seed_task: None,
            },
        );
        let _ = self.core.tx.send(FleetEvent::WatchChanged);
        Ok(id)
    }

    /// Observe a selector **with a correct seed phase** (issue #92; the
    /// RFC 04 §3.2 discipline of [`crate::seed_subscribe`], run through this
    /// monitor's bounded broadcast):
    ///
    /// - the subscriber is declared first, then both seed paths run as
    ///   bounded GETs (`@adv` caches for live publishers, the selector
    ///   itself for router storages — the crashed-producer case);
    /// - one per-key LWW merge spans seed *and* live samples until the
    ///   boundary, so a transition in the seed window lands exactly once and
    ///   a stale seed cannot regress a key. Suppressions are counted in the
    ///   coverage, never silently absorbed (O6) — and they are *not* part of
    ///   `Dropped(n)`, which counts only broadcast lag;
    /// - [`FleetEvent::WatchSeeded`] fires once **both** paths resolve,
    ///   carrying this id and the [`crate::SeedCoverage`]. After it, the
    ///   merge is dropped and live samples flow untouched (the merge map is
    ///   a seed-phase structure, not a per-watch leak).
    pub async fn watch_seeded(
        &self,
        selector: &str,
        policy: crate::bus::seed::SeedPolicy,
    ) -> Result<WatchId> {
        use crate::bus::seed::{Merge, cache_selector, seed_get, view_of};
        use crate::report::SeedCoverage;

        // The merge gate: `Some` while seeding (both live callback and seed
        // replies pass `admit`), swapped to `None` at the boundary.
        let gate: Arc<arc_swap::ArcSwapOption<Merge>> =
            Arc::new(arc_swap::ArcSwapOption::from_pointee(Merge::new()));

        // 1) The subscriber, FIRST (RFC 04 §3.2).
        let core = Arc::clone(&self.core);
        let cb_gate = Arc::clone(&gate);
        let subscriber = self
            .session
            .declare_subscriber(selector)
            .callback(move |sample| {
                let sn = sample.source_info().map(|si| si.source_sn());
                let view = view_of(&sample);
                if let Some(merge) = cb_gate.load_full()
                    && !merge.admit(&view)
                {
                    return;
                }
                core.ingest(view, sn);
            })
            .await
            .map_err(|e| anyhow!("seeded subscribe {selector}: {e}"))?;

        // 2) The seed GETs, AFTER — registered as this watch's seed task so
        //    `unwatch` during the seed phase aborts it.
        let id = WatchId(self.next_watch.fetch_add(1, Ordering::Relaxed));
        let seed_task = {
            let session = self.session.clone();
            let core = Arc::clone(&self.core);
            let selector = selector.to_string();
            tokio::spawn(async move {
                let merge = gate
                    .load_full()
                    .expect("gate holds the merge while seeding");
                let history = async {
                    if policy.history {
                        let sel = cache_selector(&selector);
                        Some(
                            seed_get(&session, &sel, policy.timeout, &merge, |view| {
                                core.ingest(view, None);
                            })
                            .await,
                        )
                    } else {
                        None
                    }
                };
                let storage = async {
                    if policy.storage {
                        Some(
                            seed_get(&session, &selector, policy.timeout, &merge, |view| {
                                core.ingest(view, None);
                            })
                            .await,
                        )
                    } else {
                        None
                    }
                };
                let (history_replies, storage_replies) = tokio::join!(history, storage);
                let coverage = SeedCoverage {
                    history_replies,
                    storage_replies,
                    superseded: merge.superseded(),
                };
                gate.store(None);
                // Seeded keys should be visible on the very tick that
                // announces the boundary, not one tick later.
                core.tick();
                let _ = core.tx.send(FleetEvent::WatchSeeded { id, coverage });
            })
        };
        self.watches.lock().await.insert(
            id,
            WatchEntry {
                selector: selector.to_string(),
                subscriber,
                seed_task: Some(seed_task),
            },
        );
        let _ = self.core.tx.send(FleetEvent::WatchChanged);
        Ok(id)
    }

    /// Stop observing: undeclares the subscriber (awaited to completion — the
    /// teardown is acknowledged, not racing a drop), then retires statistics
    /// for keys no remaining watch covers. Retired keys are **counted**
    /// ([`crate::model::stats::StatsTable::unwatched`]): a shrinking key set must
    /// never read as a quieting bus (RFC 09 §5.1 O6).
    pub async fn unwatch(&self, id: WatchId) -> Result<()> {
        let mut entry = {
            let mut watches = self.watches.lock().await;
            watches
                .remove(&id)
                .ok_or_else(|| anyhow!("unknown watch id {id:?}"))?
        };
        // A released watch must not keep ingesting seed replies: the seed
        // task dies with the watch (its boundary event simply never fires —
        // the watch is gone, so there is nothing left to flip to "live").
        if let Some(task) = entry.seed_task.take() {
            task.abort();
        }
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
    ///
    /// The teardown is the [`Drop`] one: tasks aborted, subscribers left to
    /// undeclare in the background. Where the *acknowledgement* matters —
    /// tearing one monitor down to declare another over the same keys — use
    /// [`shutdown`](Self::shutdown) instead.
    pub fn stop(self) {
        drop(self);
    }

    /// Stop watching, **acknowledged**: every watch undeclares and is waited
    /// for before this returns.
    ///
    /// [`unwatch`](Self::unwatch) awaits `undeclare` on purpose — "the
    /// teardown is acknowledged, not racing a drop" — but the whole-monitor
    /// path had no such verb: [`Drop`] can only abort the tasks and let the
    /// subscribers undeclare on their own, in the background, which is the
    /// race that doc disavows. A frontend that re-scopes by rebuilding its
    /// monitor was therefore declaring the new subscribers while the old ones
    /// were still tearing down.
    ///
    /// Every watch is drained even if one fails to undeclare — a monitor half
    /// torn down is worse than one torn down noisily — and the failures are
    /// reported together. `Drop` still runs afterwards, aborting the
    /// liveliness and tick tasks, and remains the fallback for every path
    /// that does not come through here.
    ///
    /// Statistics are **not** retired the way `unwatch` retires them: that
    /// counter answers "the key set shrank because you stopped looking"
    /// (RFC 09 §5.1 O6) for a monitor that goes on running. This one is the
    /// end of the observation; the core goes with it unless a caller kept an
    /// `Arc`, and a re-scope's next monitor starts from a fresh one.
    pub async fn shutdown(self) -> Result<()> {
        let drained: Vec<WatchEntry> = {
            let mut watches = self.watches.lock().await;
            watches.drain().map(|(_, entry)| entry).collect()
        };
        let mut failed = Vec::new();
        for mut entry in drained {
            if let Some(task) = entry.seed_task.take() {
                task.abort();
            }
            if let Err(e) = entry.subscriber.undeclare().await {
                failed.push(format!("{}: {e}", entry.selector));
            }
        }
        drop(self);
        if failed.is_empty() {
            Ok(())
        } else {
            Err(anyhow!("undeclare {}", failed.join("; ")))
        }
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
            stamped_by: None,
            attachment: None,
            priority: zenoh::qos::Priority::DEFAULT,
            congestion_control: zenoh::qos::CongestionControl::DEFAULT,
            reliability: zenoh::qos::Reliability::DEFAULT,
            express: false,
            source: None,
            received: Instant::now(),
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

    /// Deep-review D2: `ingest_at` measures the skewed latency (#119) from
    /// the **injected** wall clock, never `SystemTime::now()` — so a replay
    /// rebuild that injects the capture clock on both axes folds the same
    /// latencies every time, exactly (the #217 bit-identical promise), and
    /// the number itself is `wall − HLC`, not `rebuild-time − HLC`.
    #[test]
    fn injected_wall_clock_drives_the_latency_fold_deterministically() {
        let stamp_epoch = Duration::from_secs(1_000_000);
        let ts = zenoh::time::Timestamp::new(
            zenoh::time::NTP64::from(stamp_epoch),
            zenoh::time::TimestampId::rand(),
        );
        let key = "zs/v1/h-a/telemetry/x/m";
        let stamped_view = || {
            let mut v = view(key, 4);
            v.timestamp = Some(ts);
            v.stamped_by = Some(StampProvenance::Unattributable {
                stamper: *ts.get_id(),
            });
            v
        };
        let now = Instant::now();
        // The injected wall clock says the sample arrived 5 ms after its
        // stamp — regardless of what the live wall clock reads (it is a
        // million seconds past this epoch already).
        let wall = ts.get_time().to_system_time() + Duration::from_millis(5);

        let fold = || {
            let core = MonitorCore::new(8);
            core.ingest_at(Arc::new(stamped_view()), None, now, wall);
            core.with_stats(|s| s.get(key).expect("recorded").latency())
                .expect("a stamped sample has a latency window")
        };
        let a = fold();
        let summary = a.unattributable.expect("unattributable population");
        assert_eq!(summary.samples, 1);
        assert_eq!(
            summary.median_us, 5_000,
            "the latency is wall − HLC, on the injected wall clock"
        );
        // A second rebuild with the same injected clocks folds identically —
        // a live `SystemTime::now()` read in between would not.
        assert_eq!(a, fold());
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

    /// The retained window rides the ingest path, not the broadcast: a
    /// receiver that lagged its way to `Dropped(n)` lost nothing from the
    /// ring, and the window still holds every ingested sample (#217).
    #[tokio::test]
    async fn the_ring_sees_what_a_lagging_receiver_missed() {
        let core = MonitorCore::new(2);
        let mut slow = core.events();
        for i in 0..10 {
            core.ingest(view(&format!("zs/v1/h-a/telemetry/x/m{i}"), 1), None);
        }
        let Some(StreamItem::Dropped(_)) = slow.recv().await else {
            panic!("the broadcast lagged");
        };
        let window = core.retained();
        assert_eq!(window.len(), 10, "the ring is upstream of the lag");
        assert_eq!(window[0].key, "zs/v1/h-a/telemetry/x/m0");
        assert_eq!(window[9].key, "zs/v1/h-a/telemetry/x/m9");
    }

    /// RFC 09 §5.1 **O6** / v1.18 **R1**: the eviction populations stay
    /// separate numbers (#217). Broadcast lag ("could not keep up"),
    /// stats-table eviction ("chose to forget under the key bound") and
    /// retention eviction ("chose to forget under the window's byte budget")
    /// are three different facts about the same session, and each ledger
    /// balances on its own.
    #[tokio::test]
    async fn the_eviction_populations_are_never_folded() {
        const SAMPLES: usize = 100;
        let core = MonitorCore::bounded(2, 8);
        core.set_retention_budget(crate::model::retain::RetentionBudget {
            max_bytes: 1100,
            max_age: Duration::from_secs(3600),
        });
        let mut slow = core.events();
        for i in 0..SAMPLES {
            core.ingest(view(&format!("zs/v1/h-a/telemetry/x/m{i}"), 64), None);
        }
        let Some(StreamItem::Dropped(lagged)) = slow.recv().await else {
            panic!("the broadcast lagged");
        };

        // Each population's own ledger balances — nothing crossed over.
        assert_eq!(core.dropped(), lagged, "lag counts only broadcast lag");
        let table_kept = core.with_stats(StatsTable::len);
        assert_eq!(
            table_kept as u64 + core.keys_evicted(),
            SAMPLES as u64,
            "every key is in the table or in its eviction count"
        );
        let r = core.retention();
        assert_eq!(
            r.retained as u64 + r.evicted,
            SAMPLES as u64,
            "every sample is in the ring or in its eviction count"
        );
        assert_eq!(r.expired, 0, "nothing aged out in this window");
        assert_eq!(core.keys_unwatched(), 0, "nothing was unwatched");

        // And they are genuinely different numbers, not one figure worn
        // three ways.
        assert_ne!(r.evicted, core.keys_evicted());
        assert_ne!(r.evicted, core.dropped());
    }
}
