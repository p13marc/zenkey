//! Replay mode (issue #74): the panes fed from a `.zrec` file instead of
//! the link.
//!
//! The seam is the tick: everything downstream of
//! [`crate::message::BusTick`] already renders whatever it is handed, so
//! replay synthesizes ticks from a **session-less**
//! [`zenkey_fleet::MonitorCore`] — the same stats table, the same tree
//! builder, the same bounded counters as live — and the panes cannot tell
//! the difference. Nothing here can publish: there is no session in this
//! module at all, which is the RFC 09 §5.2 pane-replay posture made
//! structural.
//!
//! Two clocks ride a `.zrec` (§5.2's rule): the scrubber's axis is the
//! **capture clock** — each row's `t`, the recording observer's arrival
//! offsets — and the UI says so. Scrubbing backwards is a rebuild: LWW
//! folding is not invertible, so `scrub_to` re-ingests from the top into a
//! fresh core, which stays cheap because a capture is bounded by
//! construction.

use std::io::BufRead;
use std::sync::Arc;

use zenkey_fleet::{MonitorCore, SampleView, ZrecHeader, ZrecItem, ZrecReader};

use crate::message::BusTick;

/// How many samples one synthesized tick hands the panes — the same cap as
/// the live pump, for the same reason (an echo pane cannot render 10k
/// lines per tick; the overflow is counted as `coalesced`, not hidden).
const BATCH_CAP: usize = 512;

/// One loaded row: its capture-clock offset and the view the panes get.
pub struct ReplayRow {
    pub t_us: u64,
    pub view: Arc<SampleView>,
}

/// The whole replay: file metadata, the loaded rows, the transport state,
/// and the session-less core the tree is folded through.
pub struct ReplayState {
    /// Where the file came from — the banner names it.
    pub path: String,
    /// The capture's own account of itself (selectors, base, when).
    pub header: ZrecHeader,
    /// Every publishable row, in file order (which is capture order).
    pub rows: Vec<ReplayRow>,
    /// Samples the *capture* missed (summed from its drop records) — shown
    /// in the banner: a replay is a partial view and says so (O6).
    pub capture_dropped: u64,
    /// Rows that did not parse — counted, never skipped.
    pub malformed: u64,
    /// The playhead, on the capture clock, µs.
    pub position_us: u64,
    /// The capture's span (the last row's `t`).
    pub span_us: u64,
    pub playing: bool,
    pub speed: f64,
    /// Rows `[..cursor]` have been fed to the core for this playhead.
    cursor: usize,
    core: Arc<MonitorCore>,
}

/// A qos profile name back to wire axes, for display in the panes.
/// Rows that recorded no profile show the middle of the road — the panes
/// render what the file knows, and the file said nothing.
fn axes(
    qos: Option<&str>,
) -> (
    zenoh::qos::Priority,
    zenoh::qos::CongestionControl,
    zenoh::qos::Reliability,
    bool,
) {
    match qos.and_then(zenkey::qos::QosProfile::from_name) {
        Some(p) => (
            p.priority(),
            p.congestion_control(),
            p.reliability(),
            p.express(),
        ),
        None => (
            zenoh::qos::Priority::Data,
            zenoh::qos::CongestionControl::Drop,
            zenoh::qos::Reliability::BestEffort,
            false,
        ),
    }
}

impl ReplayState {
    /// Load a `.zrec` into memory. A capture is bounded by construction
    /// (RecordBounds or an operator's ctrl-c), so whole-file loading is the
    /// honest simple thing — and scrubbing needs random access anyway.
    pub fn load(path: &str, source: impl BufRead) -> Result<ReplayState, String> {
        let mut reader = ZrecReader::new(source).map_err(|e| e.to_string())?;
        let header = reader.header().clone();
        let mut rows = Vec::new();
        let mut capture_dropped = 0u64;
        let mut malformed = 0u64;
        let loaded_at = std::time::Instant::now();
        while let Some(item) = reader.next() {
            match item {
                Ok(ZrecItem::Sample { row, t_us, .. }) => {
                    let (priority, congestion_control, reliability, express) =
                        axes(row.qos.as_deref());
                    // A row without `t` (hand-piped ndjson) sits at the
                    // previous row's instant: no pacing claim invented.
                    let t_prev = rows.last().map_or(0, |r: &ReplayRow| r.t_us);
                    rows.push(ReplayRow {
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
                            priority,
                            congestion_control,
                            reliability,
                            express,
                            source: None,
                            received: loaded_at,
                        }),
                    });
                }
                Ok(ZrecItem::Dropped(n)) => capture_dropped += n,
                Err(_) => malformed += 1,
            }
        }
        let span_us = rows.last().map_or(0, |r| r.t_us);
        Ok(ReplayState {
            path: path.to_string(),
            header,
            rows,
            capture_dropped,
            malformed,
            position_us: 0,
            span_us,
            playing: false,
            speed: 1.0,
            cursor: 0,
            core: MonitorCore::new(1024),
        })
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
            self.core.ingest((*row.view).clone(), None);
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
        let totals = (
            tree.root.subtree_count,
            tree.root.subtree_bytes,
            tree.root.subtree_rate_hz,
        );
        Arc::new(BusTick {
            tree,
            samples,
            lagged: 0,
            coalesced,
            nodes: Vec::new(),
            keys,
            keys_evicted,
            keys_unwatched,
            // The coverage statement is the file's: what the capture asked
            // (O5) — not a claim about the live bus.
            watched: self.header.selectors.clone(),
            seeded: Vec::new(),
            totals,
        })
    }

    /// Seconds form of the playhead and span, for the scrubber's label.
    pub fn clock(&self) -> (f64, f64) {
        (self.position_us as f64 / 1e6, self.span_us as f64 / 1e6)
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
        assert_eq!(again.totals.0, back.totals.0);

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
