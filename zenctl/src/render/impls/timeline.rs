//! `timeline` (#216) — lanes, one axis, and no line between them.
//!
//! The table groups by lane with a heading that names the lane, the axis
//! and the stamper(s), then `pos · t · hlc · key` per row. `pos` is the
//! position in the *merged* ordering, so the fleet-wide sequence survives
//! the grouping; `t` rides on both axes, so a reorder on the HLC listing is
//! visible as a non-monotonic arrival column. Under `--order hlc` the table
//! cannot draw an unstamped row, because the report has none to give it —
//! the engine's `Placed<HlcAxis>` refused them — and the note says how
//! many, and where they are.

use zenkey_fleet::report::{
    AxisLabel, BreakKind, HlcClaim, LaneId, OrderLabel, SnLaneReport, TimelineEntry, TimelineReport,
};

use crate::render::{
    BoundCost, BoundKind, Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_without,
};

impl Render for TimelineReport {
    const FAMILY: &'static str = "timeline";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_without(self, &["rows"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for r in &self.rows {
            // The tag the row already carries (`#[serde(tag = "row")]`),
            // spelled here too so the stream convention is the same one.
            out(Row::of(
                match r {
                    TimelineEntry::Sample { .. } => "sample",
                    TimelineEntry::Break { .. } => "break",
                },
                r,
            ));
        }
    }

    fn table(&self, t: &mut Table) {
        let axis = match self.order_by {
            OrderLabel::Arrival => "arrival",
            OrderLabel::Hlc => "hlc",
        };
        let mut g = Grid::unheaded(4).right(0).right(1);
        for lane in &self.lanes {
            let stampers = if lane.stampers.is_empty() {
                String::from("no stamper")
            } else {
                let mut s = format!(
                    "stamper {}",
                    lane.stampers.iter().cloned().collect::<Vec<_>>().join(", ")
                );
                let p = &lane.provenance;
                let mut parts = Vec::new();
                if p.owner > 0 {
                    parts.push(format!("{} the owner's clock", p.owner));
                }
                if p.other > 0 {
                    parts.push(format!("{} another clock", p.other));
                }
                if p.unattributable > 0 {
                    parts.push(format!("{} unattributable", p.unattributable));
                }
                if !parts.is_empty() {
                    s.push_str(&format!(" ({})", parts.join(", ")));
                }
                s
            };
            g.group(format!("{} · {axis} · {stampers}", lane.lane.label()));
            for entry in &self.rows {
                if let TimelineEntry::Sample {
                    pos,
                    lane: l,
                    key,
                    t_us,
                    hlc,
                    ..
                } = entry
                    && l == &lane.lane
                {
                    g.row([
                        Cell::int(*pos as u64),
                        Cell::text(offset(*t_us)),
                        // Unstamped is *asked and absent*, not unasked: an
                        // empty cell, never `—`.
                        Cell::text(hlc.clone().unwrap_or_default()),
                        Cell::text(key),
                    ]);
                }
            }
        }
        let breaks: Vec<&TimelineEntry> = self
            .rows
            .iter()
            .filter(|e| matches!(e, TimelineEntry::Break { .. }))
            .collect();
        if !breaks.is_empty() {
            g.group(format!("breaks · {axis} positions"));
            for b in breaks {
                if let TimelineEntry::Break {
                    pos, kind, n, lane, ..
                } = b
                {
                    g.row([
                        Cell::int(*pos as u64),
                        Cell::text(""),
                        Cell::text(match kind {
                            BreakKind::Dropped => format!("dropped ×{n}"),
                            BreakKind::Coalesced => format!("coalesced ×{n}"),
                        }),
                        Cell::text(lane.as_ref().map(LaneId::label).unwrap_or_default()),
                    ]);
                }
            }
        }
        t.grid(g);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        // The axis claim is part of the measurement (RFC 09 §5.1 O7): which
        // clock, and what an order on it can mean.
        notes.push(
            Note::caveat(match &self.axis {
                AxisLabel::Arrival { clock } => format!(
                    "ordered by arrival — {clock}; a position says when this observer \
                     saw a sample, never when it was produced"
                ),
                AxisLabel::Hlc { claim } => match claim {
                    HlcClaim::HappensBefore { stamper } => format!(
                        "ordered by HLC — every stamped sample was stamped by {stamper}, \
                         so the order is that node's happened-before (its HLC is \
                         monotonic and updated by what it forwarded)"
                    ),
                    HlcClaim::SkewedWallClock { stampers } => format!(
                        "ordered by HLC across {} stampers ({}) — a comparison of \
                         wall clocks nothing this observer can see synchronised; not \
                         a happened-before",
                        stampers.len(),
                        stampers.iter().cloned().collect::<Vec<_>>().join(", ")
                    ),
                    HlcClaim::NoStampedSamples => String::from(
                        "ordered by HLC, and no sample in the window carried one — an \
                         empty axis claims nothing",
                    ),
                },
            })
            .cite("RFC 09 §5.1 O7"),
        );
        if self.unstamped_excluded > 0 {
            notes.push(Note::coverage(format!(
                "{} unstamped sample(s) are not on this axis — an unstamped sample has \
                 no HLC position and is never defaulted to its arrival time; see \
                 `--order arrival`",
                self.unstamped_excluded
            )));
        }
        if self.order_by == OrderLabel::Hlc && self.dropped > 0 {
            notes.push(Note::coverage(format!(
                "{} dropped sample(s) have no position on the HLC axis (a drop is \
                 something this observer suffered, on its own clock); see `--order \
                 arrival` for where they fell",
                self.dropped
            )));
        }
        if let SnLaneReport::Unavailable { reason } = &self.sn_lane {
            notes.push(
                Note::coverage(format!(
                    "the per-publisher sequence-number lane is unavailable: {reason}"
                ))
                .cite("RFC 09 §5.1 O4"),
            );
        }
        let excluded = zenkey_fleet::zrec_excluded(&self.scopes);
        if !excluded.is_empty() {
            notes.push(
                Note::coverage(format!(
                    "{} excluded from this window, not empty: no selector names them, \
                     and `*`/`**` never match a verbatim chunk",
                    excluded.join(", ")
                ))
                .cite("tooling guide O5"),
            );
        }
        // Whose clock a stamp is on needs the owner's session zid, which only
        // a presence read names (O7): without one, every stamp reads
        // unattributable, and that is not the same as foreign.
        if self.lens.presence.is_none() {
            notes.push(
                Note::coverage(
                    "no presence read stood behind these lanes: no owner's zid is \
                     named, so every stamp is unattributable (not foreign), and a key \
                     resolves past its address only through --contracts",
                )
                .cite("tooling guide O7"),
            );
        } else if self.lens.presence.as_ref().is_some_and(|p| !p.complete) {
            notes.push(
                Note::coverage(
                    "the presence read ended at its timeout: an owner it did not see \
                     leaves its stamps unattributable",
                )
                .cite("spec §8.1"),
            );
        }
        notes.push(Note::rendering(
            "deliberately no edges: a merged ordering shows when things were seen on \
             which clock, never that one caused another",
        ));
        notes
    }

    fn bounds(&self) -> Vec<BoundCost> {
        vec![
            BoundCost::new(
                BoundKind::Missed,
                self.dropped,
                "sample(s) dropped while behind — the ordering covers only what was seen",
            ),
            BoundCost::new(
                BoundKind::Coalesced,
                self.coalesced,
                "sample(s) coalesced by the consumer — positions are of the merged",
            ),
            BoundCost::new(
                BoundKind::Retired,
                self.keys_evicted,
                "key(s) retired at the stats-table bound during the window",
            ),
        ]
    }

    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: self.scopes.clone(),
            window_s: self.window_s,
        })
    }
}

/// An arrival offset, humanised: `+1.000ms`, `+2.500s`.
fn offset(us: u64) -> String {
    if us < 1_000_000 {
        format!("+{:.3}ms", us as f64 / 1_000.0)
    } else {
        format!("+{:.3}s", us as f64 / 1_000_000.0)
    }
}
