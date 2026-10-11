//! Replay mode (issue #74): the panes fed from a `.zrec` file — or, since
//! #217, from the monitor's **retained window** — instead of the link.
//!
//! The seam is the tick: everything downstream of
//! [`crate::message::BusTick`] already renders whatever it is handed, so
//! replay synthesizes ticks from a **session-less**
//! [`zenkey_fleet::MonitorCore`] — the same stats table, the same tree
//! builder, the same bounded counters as live — and the panes cannot tell
//! the difference. Nothing here can publish: there is no session in this
//! module at all, which is the RFC 09 §5.2 pane-replay posture made
//! structural. The retained window rides the same machinery over an
//! in-memory ring: [`ReplayState::from_retained`] instead of
//! [`ReplayState::load`], and everything after the constructor is shared.
//!
//! Two clocks ride a `.zrec` (§5.2's rule): the scrubber's axis is the
//! **capture clock** — each row's `t`, the recording observer's arrival
//! offsets — and the UI says so. Scrubbing backwards is a rebuild: LWW
//! folding is not invertible, so `scrub_to` re-ingests from the top into a
//! fresh core, which stays cheap because a capture is bounded by
//! construction (a file by [`zenkey_fleet::RecordBounds`], the ring by its
//! [`zenkey_fleet::RetentionBudget`]). The rebuild feeds
//! [`MonitorCore::ingest_at`] on the capture clock, so the fold — EWMA
//! rates included — is deterministic in the rows alone, which is what lets
//! a retained scrub and a file replay of the same traffic yield
//! byte-identical panes (#217's acceptance).
//!
//! **A replay resolves through an offline lens** (#614, FL10b; the tooling
//! guide's O4): the capture's namespace and the contracts held offline, and
//! no presence read. The live catalog describes the world the window is
//! connected to now, not the one the capture was taken in — so the tree's
//! declared half, the payload verdicts and an exported row's identity are
//! all read through [`ReplayLens`], never through the deployment's lens.
//!
//! **What a row did not record is not shown as observed.** A version-1 or
//! -2 `.zrec` row named a v1 QoS profile, which zk2 does not read; such a
//! row carries no wire axes, and the panes say "QoS not recorded" for it
//! rather than showing the defaults its view must carry.

use std::collections::HashSet;
use std::io::BufRead;
use std::sync::Arc;
use std::time::{Duration, Instant};

use zenkey_fleet::{
    BundleStore, ContractSet, MonitorCore, RetentionStats, SampleView, Skeleton, WireQos,
    ZrecHeader, ZrecItem, ZrecReader,
};

use crate::message::BusTick;

/// What a replay resolves keys through (#614, FL10b): its own namespace and
/// the contracts held offline, with **no** presence read — a lens over
/// another world than the live one (O4). Its skeleton is the tree's
/// declared half while the replay runs.
#[derive(Clone)]
pub struct ReplayLens {
    /// The capture's namespace (a file's header `base`), or the live one
    /// for the retained window, which is this deployment's own traffic.
    pub namespace: String,
    /// The revisions held offline (`--contracts`); empty when none are.
    pub contracts: Arc<ContractSet>,
    /// Never consulted: without a presence read the lens resolves each
    /// interface by the one revision held offline. A lens is built over a
    /// store all the same.
    store: BundleStore,
    /// The declared keyspace from this lens alone — every revision held
    /// under a symbolic `{system}/{service}`, nothing placed by presence.
    pub skeleton: Arc<Skeleton>,
}

impl std::fmt::Debug for ReplayLens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplayLens")
            .field("namespace", &self.namespace)
            .field("contracts", &self.contracts.len())
            .finish_non_exhaustive()
    }
}

impl ReplayLens {
    /// The offline lens over `namespace` and `contracts`, and its skeleton.
    pub fn offline(namespace: &str, contracts: Option<Arc<ContractSet>>) -> ReplayLens {
        let contracts: Arc<ContractSet> = contracts.unwrap_or_default();
        let store = BundleStore::new(Duration::from_secs(1));
        // The admin space is the live world's too: not an input here.
        let skeleton = Arc::new(Skeleton::build(
            &zenkey_fleet::Lens::new(namespace, None, &store)
                .offline(&contracts)
                .held(contracts.len()),
            None,
        ));
        ReplayLens {
            namespace: namespace.to_owned(),
            contracts,
            store,
            skeleton,
        }
    }

    /// The lens: catalog `None`, the offline contracts.
    pub fn lens(&self) -> zenkey_fleet::Lens<'_> {
        zenkey_fleet::Lens::new(&self.namespace, None, &self.store)
            .offline(&self.contracts)
            .held(self.contracts.len())
    }

    /// The same lens, owned, for a task: no catalog, no feed to nudge.
    pub fn inputs(&self) -> crate::services::value::LensInputs {
        crate::services::value::LensInputs {
            namespace: self.namespace.clone(),
            catalog: None,
            store: self.store.clone(),
            contracts: Some(Arc::clone(&self.contracts)),
            generation: 0,
            feed: None,
        }
    }
}

/// What the panes say of a replayed row that recorded no wire axes (#614,
/// FL10b): a version-1 or -2 `.zrec` named a v1 profile, which zk2 does
/// not read — so nothing was recorded a zk2 reader can show.
pub const QOS_NOT_RECORDED: &str = "QoS not recorded";

/// Mints each replay's id: a verdict batch carries it home, so a batch
/// checked for one replay lands in no other, and none after the replay ends.
static NEXT_REPLAY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// How many samples one synthesized tick hands the panes — the same cap as
/// the live pump, for the same reason (an echo pane cannot render 10k
/// lines per tick; the overflow is counted as `coalesced`, not hidden).
const BATCH_CAP: usize = 512;

/// One loaded row: its capture-clock offset and the view the panes get.
pub struct ReplayRow {
    pub t_us: u64,
    pub view: Arc<SampleView>,
    /// Whether the row recorded its wire axes (`.zrec` v3's `qos_axes`).
    /// When it did not, the view's axes are zenoh's defaults and are never
    /// shown as observed (#614, FL10b).
    pub qos_recorded: bool,
}

/// A parsed [`ReplayState`] in transit from the load task to its handler
/// (#255).
///
/// `Message` derives `Clone` (iced's widget callbacks require it), but a
/// replay state is a single-owner thing — cloning one would share its fold
/// core between two scrubbing owners. So the message carries this envelope
/// instead: `Clone`/`PartialEq` by handle, and [`take`](LoadedReplay::take)
/// hands the state itself out exactly once.
pub struct LoadedReplay(Arc<std::sync::Mutex<Option<ReplayState>>>);

impl LoadedReplay {
    pub fn new(state: ReplayState) -> LoadedReplay {
        LoadedReplay(Arc::new(std::sync::Mutex::new(Some(state))))
    }

    /// The state, the first time; `None` on any later ask.
    pub fn take(&self) -> Option<ReplayState> {
        self.0.lock().ok().and_then(|mut s| s.take())
    }
}

impl Clone for LoadedReplay {
    fn clone(&self) -> LoadedReplay {
        LoadedReplay(Arc::clone(&self.0))
    }
}

impl PartialEq for LoadedReplay {
    fn eq(&self, other: &LoadedReplay) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for LoadedReplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LoadedReplay(..)")
    }
}

/// Where the rows came from — the banner dispatches on this, because the
/// two sources owe the user different honesty (#217).
pub enum ReplaySource {
    /// A `.zrec` opened from disk (#74).
    File {
        /// Where the file came from — the banner names it.
        path: String,
        /// The capture's own account of itself (selectors, base, when).
        header: ZrecHeader,
    },
    /// The monitor's retained window, snapshotted on entry (#217).
    Retained {
        /// The ring's account of itself at the moment the window was taken:
        /// the budget the banner states, and what each bound cost —
        /// retention eviction stays its own number, apart from stats-table
        /// eviction and from `Dropped(n)` (RFC 09 §5.1 O6, v1.18 R1).
        taken: RetentionStats,
    },
}

/// The whole replay: its source, the loaded rows, the transport state,
/// and the session-less core the tree is folded through.
pub struct ReplayState {
    /// Where the rows came from, with that source's own metadata.
    pub source: ReplaySource,
    /// The coverage statement every tick carries (O5): the file's header
    /// selectors, or the watch set the ring was fed by. Built once rather
    /// than per scrub (#178).
    pub(crate) watched: Arc<[String]>,
    /// Every publishable row, in file order (which is capture order).
    pub rows: Vec<ReplayRow>,
    /// Samples the *capture* missed (summed from its drop records) — shown
    /// in the banner: a replay is a partial view and says so (O6).
    pub capture_dropped: u64,
    /// Rows that did not parse — counted, never skipped.
    pub malformed: u64,
    /// Version-2 preamble rows (RFC 13 §4.1; #218): state at capture
    /// start, fed to the fold first and counted apart from observed rows.
    /// A pane replay has no session, so seeding its own fold from them is
    /// exactly right — nothing is republished (§4.3's two replays).
    pub preamble_rows: u64,
    /// Version-2 trigger records, at their capture-clock position (the
    /// `t` of the last observed row before each): what fired, so the
    /// scrubber can mark where.
    pub triggers: Vec<(u64, zenkey_fleet::Transition)>,
    /// The playhead, on the capture clock, µs.
    pub position_us: u64,
    /// The capture's span (the last row's `t`).
    pub span_us: u64,
    pub playing: bool,
    pub speed: f64,
    /// Rows `[..cursor]` have been fed to the core for this playhead.
    cursor: usize,
    core: Arc<MonitorCore>,
    /// The capture clock's zero on this process's monotonic clock: the load
    /// instant for a file, the window's oldest arrival for a retained ring.
    /// The rebuild feeds `fold_epoch + t` into [`MonitorCore::ingest_at`],
    /// and "save window as `.zrec`" writes rows against it, so a saved
    /// window keeps the pacing the ring preserved (#217).
    pub(crate) fold_epoch: Instant,
    /// The same zero on the **wall** clock, fixed once at construction and
    /// fed as `ingest_at`'s wall axis (`wall_epoch + t`). Injecting both
    /// clocks is what keeps a rebuild deterministic through the latency
    /// fold too: reading the live wall clock mid-scrub would subtract a
    /// different "now" from each rebuild's HLCs (RFC 09 §5.2's two-clocks
    /// rule, applied to the second clock).
    wall_epoch: std::time::SystemTime,
    /// This replay's id ([`NEXT_REPLAY`]): what its verdict batches carry.
    pub(crate) id: u64,
    /// What this replay resolves keys through (#614, FL10b): offline, no
    /// presence read. Built with no contracts at load; the landing gives it
    /// the window's ([`ReplayState::resolve_offline`]).
    pub lens: ReplayLens,
    /// Rows that recorded no wire axes — a version-1 or -2 file's, whose v1
    /// profile names zk2 does not read: "QoS not recorded", counted.
    pub qos_unrecorded: u64,
    /// The views of those rows, by allocation: the replay holds every row's
    /// `Arc` for its whole life, so an address names one row exactly. What
    /// [`ReplayState::qos_recorded`] answers from.
    unrecorded: HashSet<usize>,
}

/// A row's recorded wire axes (`.zrec` v3's `qos_axes`, #614). A row that
/// recorded none — every version-1 and -2 row, whose v1 profile name zk2
/// does not read — carries zenoh's defaults in its view, because a view
/// must carry some; the row is marked, and the panes say "QoS not recorded"
/// for it (FL10b), never these.
fn axes(qos: Option<WireQos>) -> WireQos {
    qos.unwrap_or(WireQos::DEFAULT)
}

impl ReplayState {
    /// Load a `.zrec` into memory. A capture is bounded by construction
    /// (RecordBounds or an operator's ctrl-c), so whole-file loading is the
    /// honest simple thing — and scrubbing needs random access anyway.
    pub fn load(
        path: &str,
        source: impl BufRead,
    ) -> Result<ReplayState, crate::services::ServiceError> {
        let mut reader = ZrecReader::new(source).map_err(crate::services::ServiceError::of)?;
        let header = reader.header().clone();
        let mut rows = Vec::new();
        let mut capture_dropped = 0u64;
        let mut malformed = 0u64;
        let mut preamble_rows = 0u64;
        let mut triggers = Vec::new();
        let loaded_at = std::time::Instant::now();
        while let Some(item) = reader.next() {
            // A preamble row is a sample at `t: 0` to the fold — it is the
            // base the pre-roll's deltas are deltas *of* — and counted apart.
            let (row, t_us, preamble) = match item {
                Ok(ZrecItem::Sample { row, t_us, .. }) => (row, t_us, false),
                Ok(ZrecItem::Preamble { row, .. }) => (row, Some(0), true),
                Ok(ZrecItem::Dropped(n)) => {
                    capture_dropped += n;
                    continue;
                }
                Ok(ZrecItem::Trigger(t)) => {
                    triggers.push((rows.last().map_or(0, |r: &ReplayRow| r.t_us), *t));
                    continue;
                }
                Err(_) => {
                    malformed += 1;
                    continue;
                }
            };
            if preamble {
                preamble_rows += 1;
            }
            let qos_recorded = row.qos_axes.is_some();
            let q = axes(row.qos_axes);
            // A row without `t` (hand-piped ndjson) sits at the
            // previous row's instant: no pacing claim invented.
            let t_prev = rows.last().map_or(0, |r: &ReplayRow| r.t_us);
            rows.push(ReplayRow {
                qos_recorded,
                t_us: t_us.unwrap_or(t_prev),
                view: Arc::new(SampleView {
                    key: row.key,
                    payload: zenoh::bytes::ZBytes::from(row.payload),
                    encoding: row.encoding.unwrap_or_default(),
                    kind: if row.delete {
                        zenoh::sample::SampleKind::Delete
                    } else {
                        zenoh::sample::SampleKind::Put
                    },
                    // The capture-time HLC is informative text in the
                    // file; it is deliberately NOT resurrected as a
                    // live timestamp — the scrubber's axis is `t`. No
                    // timestamp, therefore no stamper (#213).
                    timestamp: None,
                    stamped_by: None,
                    attachment: row.attachment.map(zenoh::bytes::ZBytes::from),
                    priority: q.priority,
                    congestion_control: q.congestion,
                    reliability: q.reliability,
                    express: q.express,
                    source: None,
                    received: loaded_at,
                }),
            });
        }
        let span_us = rows.last().map_or(0, |r| r.t_us);
        let unrecorded: HashSet<usize> = rows
            .iter()
            .filter(|r| !r.qos_recorded)
            .map(|r| Arc::as_ptr(&r.view) as usize)
            .collect();
        Ok(ReplayState {
            id: NEXT_REPLAY.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            lens: ReplayLens::offline(&header.base, None),
            qos_unrecorded: unrecorded.len() as u64,
            unrecorded,
            watched: Arc::from(header.selectors.clone()),
            source: ReplaySource::File {
                path: path.to_string(),
                header,
            },
            rows,
            capture_dropped,
            malformed,
            preamble_rows,
            triggers,
            position_us: 0,
            span_us,
            playing: false,
            speed: 1.0,
            cursor: 0,
            core: MonitorCore::new(1024),
            fold_epoch: loaded_at,
            // Fixed at load: file rows carry no resurrectable HLC, so no
            // latency is ever computed from this — it is the epoch's wall
            // twin, kept for the injection contract.
            wall_epoch: std::time::SystemTime::now(),
        })
    }

    /// The retained window as a replay (#217): the monitor's ring, driven by
    /// the very machinery a `.zrec` gets — same rows type, same rebuild,
    /// same ticks — so the panes cannot tell a retained scrub from a file.
    ///
    /// The rows are `Arc` clones straight out of the ring (no copy, no
    /// re-parse); each row's `t` is its arrival offset from the window's
    /// oldest sample, which is the same subtraction "save window as `.zrec`"
    /// writes — the two views of the window agree by construction.
    ///
    /// `watched` is the coverage statement the ring was fed under (O5): the
    /// window covers those watches only, never the bus, and the banner says
    /// so. There is no `capture_dropped` here — the ring sits on the ingest
    /// path, upstream of the broadcast's lag; its own costs ride in
    /// `taken` and stay separate numbers (O6).
    ///
    /// The window arrives as the engine's shared `Arc<[_]>` snapshot (#331):
    /// borrowed here, because the rows it is turned into are `Arc` clones
    /// either way and taking it by value would only pin the whole slice for
    /// the life of the replay.
    pub fn from_retained(
        window: &[Arc<SampleView>],
        watched: Arc<[String]>,
        taken: RetentionStats,
    ) -> ReplayState {
        let epoch = window.first().map_or_else(Instant::now, |v| v.received);
        // The window's oldest arrival, mapped onto the wall clock once —
        // ring rows keep their HLCs, so the rebuild's latency fold subtracts
        // this stable epoch (+ each row's `t`), reproducing the arrival-time
        // measurement instead of re-reading a moving "now" per scrub.
        let wall_epoch = std::time::SystemTime::now() - epoch.elapsed();
        let rows: Vec<ReplayRow> = window
            .iter()
            .map(|view| ReplayRow {
                t_us: u64::try_from(view.received.saturating_duration_since(epoch).as_micros())
                    .unwrap_or(u64::MAX),
                view: Arc::clone(view),
                // Live traffic: zenoh stamps every sample with its axes.
                qos_recorded: true,
            })
            .collect();
        let span_us = rows.last().map_or(0, |r| r.t_us);
        ReplayState {
            id: NEXT_REPLAY.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            // The live namespace and no contracts until the landing gives
            // the window's (`resolve_offline`).
            lens: ReplayLens::offline("", None),
            qos_unrecorded: 0,
            unrecorded: HashSet::new(),
            watched,
            source: ReplaySource::Retained { taken },
            rows,
            capture_dropped: 0,
            malformed: 0,
            preamble_rows: 0,
            triggers: Vec::new(),
            position_us: 0,
            span_us,
            playing: false,
            speed: 1.0,
            cursor: 0,
            core: MonitorCore::new(1024),
            fold_epoch: epoch,
            wall_epoch,
        }
    }

    /// Advance the playhead by a wall-clock step (scaled by `speed`) and
    /// synthesize the tick for it. Reaching the end pauses — a replay does
    /// not loop on its own.
    pub fn advance(&mut self, wall: std::time::Duration) -> Arc<BusTick> {
        let step = (wall.as_micros() as f64 * self.speed) as u64;
        self.position_us = (self.position_us + step).min(self.span_us);
        if self.position_us >= self.span_us {
            self.playing = false;
        }
        self.feed_tick()
    }

    /// Jump the playhead to an absolute instant on the capture clock.
    ///
    /// Backwards means rebuild: the core's fold is last-writer-wins and not
    /// invertible, so the state as-of `t_us` is re-derived from the top —
    /// deterministic by construction (same rows, same order, same fold).
    pub fn scrub_to(&mut self, t_us: u64) -> Arc<BusTick> {
        let t_us = t_us.min(self.span_us);
        if t_us < self.position_us {
            self.core = MonitorCore::new(1024);
            self.cursor = 0;
        }
        self.position_us = t_us;
        self.feed_tick()
    }

    /// Feed every unfed row at or before the playhead into the core, then
    /// build the tick the panes consume.
    fn feed_tick(&mut self) -> Arc<BusTick> {
        let mut samples = Vec::new();
        let mut coalesced = 0u64;
        while self
            .rows
            .get(self.cursor)
            .is_some_and(|r| r.t_us <= self.position_us)
        {
            let row = &self.rows[self.cursor];
            // The capture clock on both axes, never the live clocks (#217):
            // the fold — EWMA rates and the latency window included — is
            // then a function of the rows and the two fixed epochs alone, so
            // the same window folds bit-identically however it arrived (ring
            // or file) and however fast this loop runs.
            self.core.ingest_at(
                Arc::clone(&row.view),
                None,
                self.fold_epoch + Duration::from_micros(row.t_us),
                self.wall_epoch + Duration::from_micros(row.t_us),
            );
            if samples.len() < BATCH_CAP {
                samples.push(Arc::clone(&row.view));
            } else {
                coalesced += 1;
            }
            self.cursor += 1;
        }
        self.core.tick();
        let tree = self.core.tree();
        let keys = tree.keys;
        let keys_evicted = tree.evicted;
        let keys_unwatched = tree.unwatched;
        let totals = crate::message::WatchedTotals {
            samples: tree.root.subtree_count,
            bytes: tree.root.subtree_bytes,
            rate_hz: tree.root.subtree_rate_hz,
        };
        Arc::new(BusTick {
            tree,
            samples,
            lagged: 0,
            coalesced,
            keys,
            keys_evicted,
            keys_unwatched,
            // The coverage statement is the file's: what the capture asked
            // (O5) — not a claim about the live bus.
            watched: Arc::clone(&self.watched),
            seeded: Vec::new(),
            totals,
        })
    }

    /// Seconds form of the playhead and span, for the scrubber's label.
    pub fn clock(&self) -> (f64, f64) {
        (self.position_us as f64 / 1e6, self.span_us as f64 / 1e6)
    }

    /// Resolve this replay offline (#614, FL10b): through the contracts the
    /// window holds, in the capture's namespace — a file's header `base`, or
    /// `live_namespace` for the retained window — and no presence read.
    pub fn resolve_offline(&mut self, live_namespace: &str, contracts: Option<Arc<ContractSet>>) {
        let namespace = match &self.source {
            ReplaySource::File { header, .. } => header.base.clone(),
            ReplaySource::Retained { .. } => live_namespace.to_owned(),
        };
        self.lens = ReplayLens::offline(&namespace, contracts);
    }

    /// Whether `view`'s row recorded its wire axes: `false` for a row a
    /// version-1 or -2 file wrote, whose axes the panes must not show as
    /// observed. Any view this replay did not load answers `true`.
    pub fn qos_recorded(&self, view: &Arc<SampleView>) -> bool {
        !self.unrecorded.contains(&(Arc::as_ptr(view) as usize))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> String {
        [
            r#"{"zrec":1,"selectors":["v1/**"],"base":"","captured_at":"2026-08-12T00:00:00Z"}"#,
            r#"{"key":"v1/h-0123456789ab/state/p/a","t":0,"bytes":"MQ=="}"#,
            r#"{"key":"v1/h-0123456789ab/state/p/b","t":500000,"bytes":"Mg=="}"#,
            r#"{"dropped":4}"#,
            r#"{"key":"v1/h-0123456789ab/state/p/a","t":1000000,"bytes":"Mw=="}"#,
            r#"{"key":"v1/h-0123456789ab/state/p/a","t":2000000,"delete":true}"#,
        ]
        .join("\n")
    }

    /// The file's account of itself survives loading: rows, span, the drop
    /// ledger — and nothing in this type can publish (no session exists).
    #[test]
    fn a_capture_loads_with_its_ledger() {
        let f = fixture();
        let state = ReplayState::load("f.zrec", f.as_bytes()).unwrap();
        assert_eq!(state.rows.len(), 4);
        assert_eq!(state.capture_dropped, 4);
        assert_eq!(state.malformed, 0);
        assert_eq!(state.span_us, 2_000_000);
        assert!(!state.playing);
    }

    /// Scrubbing is deterministic: the tree as-of an instant is the same
    /// whether reached forward, backward, or twice — a rebuild, not a diff.
    #[test]
    fn scrubbing_is_a_deterministic_rebuild() {
        let f = fixture();
        let mut state = ReplayState::load("f.zrec", f.as_bytes()).unwrap();

        let at_end = state.scrub_to(2_000_000);
        assert_eq!(at_end.keys, 2, "two distinct keys by the end");

        let back = state.scrub_to(600_000);
        assert_eq!(back.keys, 2, "a and b both exist at 0.6s");
        let (pos, span) = state.clock();
        assert!((pos - 0.6).abs() < 1e-9 && (span - 2.0).abs() < 1e-9);

        // Same instant again → same fold.
        let again = state.scrub_to(600_000);
        assert_eq!(again.keys, back.keys);
        assert_eq!(again.totals.samples, back.totals.samples);

        // Forward from here replays only the unfed tail.
        let fwd = state.scrub_to(1_500_000);
        assert_eq!(fwd.samples.len(), 1, "only the t=1.0s row is new");
    }

    /// Playing to the end pauses; it never loops on its own.
    #[test]
    fn the_end_of_the_file_pauses() {
        let f = fixture();
        let mut state = ReplayState::load("f.zrec", f.as_bytes()).unwrap();
        state.playing = true;
        state.speed = 4.0;
        let _ = state.advance(std::time::Duration::from_secs(1));
        assert!(!state.playing, "4s of capture in a 1s step at 4x = done");
        assert_eq!(state.position_us, state.span_us);
    }

    /// A version-1 or -2 row named a v1 profile, which zk2 does not read: it
    /// recorded no wire axes, and the replay says so of it by row — while a
    /// version-3 row's `qos_axes` ride its view (#614, FL10b).
    #[test]
    fn a_row_without_wire_axes_is_qos_not_recorded() {
        let v1 = fixture();
        let state = ReplayState::load("f.zrec", v1.as_bytes()).unwrap();
        assert_eq!(state.qos_unrecorded, 4, "no v1 row recorded wire axes");
        assert!(state.rows.iter().all(|r| !r.qos_recorded));
        assert!(state.rows.iter().all(|r| !state.qos_recorded(&r.view)));

        let v3 = [
            r#"{"zrec":3,"selectors":["**"],"base":"","captured_at":"x","excluded":["@stream"]}"#,
            r#"{"key":"demo/a","t":0,"bytes":"MQ==","qos_axes":"real_time/block/reliable+express"}"#,
            r#"{"key":"demo/b","t":1,"bytes":"Mg=="}"#,
        ]
        .join("\n");
        let state = ReplayState::load("g.zrec", v3.as_bytes()).unwrap();
        assert_eq!(state.qos_unrecorded, 1, "the hand-piped row recorded none");
        let a = &state.rows[0];
        assert!(a.qos_recorded && state.qos_recorded(&a.view));
        assert_eq!(a.view.priority, zenoh::qos::Priority::RealTime);
        assert!(a.view.express);
        assert!(!state.qos_recorded(&state.rows[1].view));
        // A view the replay never loaded is not one of its rows.
        let stranger = Arc::new((*a.view).clone());
        assert!(state.qos_recorded(&stranger));
    }

    /// A replay resolves offline (#614, FL10b): no presence read behind its
    /// lens or its skeleton, the capture's own namespace, the contracts the
    /// window holds — every revision under a symbolic `{system}/{service}`.
    #[test]
    fn a_replay_resolves_through_an_offline_lens() {
        let f = [
            r#"{"zrec":3,"selectors":["**"],"base":"spray","captured_at":"x","excluded":[]}"#,
            r#"{"key":"spray/zk2/robot1/nav/nav.v2/stream/position","t":0,"bytes":"","qos_axes":"data/drop/reliable"}"#,
        ]
        .join("\n");
        let mut state = ReplayState::load("h.zrec", f.as_bytes()).unwrap();
        assert!(state.lens.lens().catalog().is_none());
        assert_eq!(
            state.lens.skeleton.coverage.revisions, 0,
            "no contracts yet"
        );
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2/walkthrough");
        let (set, _) = ContractSet::load_path(&dir);
        let held = set.len();
        state.resolve_offline("live-namespace", Some(Arc::new(set)));
        assert_eq!(state.lens.namespace, "spray", "the capture's namespace");
        assert!(state.lens.lens().catalog().is_none());
        let coverage = &state.lens.skeleton.coverage;
        assert!(coverage.presence.is_none(), "no presence read");
        assert_eq!(coverage.revisions, held);
        assert_eq!(
            coverage.admin_entities, None,
            "the admin space is not asked"
        );
        let identity = state
            .lens
            .lens()
            .identity("spray/zk2/robot1/nav/nav.v2/stream/position");
        assert!(identity.unresolved.is_none(), "{identity:?}");
    }

    /// A malformed line is counted, and the rest of the file still loads.
    #[test]
    fn malformed_rows_are_counted_not_fatal() {
        let f = format!(
            "{}\nnot json\n{}",
            r#"{"zrec":1,"selectors":[],"base":"","captured_at":"x"}"#,
            r#"{"key":"v1/h-0123456789ab/state/p/a","t":0,"bytes":"MQ=="}"#
        );
        let state = ReplayState::load("f.zrec", f.as_bytes()).unwrap();
        assert_eq!(state.malformed, 1);
        assert_eq!(state.rows.len(), 1);
    }
}
