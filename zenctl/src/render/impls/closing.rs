//! The families that end something and report what happened: `record`,
//! `replay`, `check expect`, `check probe`.
//!
//! They are the legitimate row-less impls. A capture is not a list of samples
//! — it is a file, and a count of what went into it; a verdict is not a list
//! of rows. Their envelope *is* the document, and saying that by writing an
//! empty `rows()` is a reviewable act, where the old code said it by falling
//! into the json arm alongside five families that had rows and lost them.
//!
//! What they share is the shape of their honesty: each one bounds an
//! observation, and each one has to say what the bound cost before it states
//! a verdict — O6 is the reason `Impaired` and `Unproven` exist at all.

use zenkey_fleet::report::{ExpectReport, ExpectVerdict};
use zenkey_fleet::{RecordReport, ReplayReport};

use crate::render::{
    BoundCost, BoundKind, Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_of,
};

impl Render for RecordReport {
    const FAMILY: &'static str = "record";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_of(self)
    }

    fn rows(&self, _out: &mut dyn FnMut(Row)) {}

    fn table(&self, t: &mut Table) {
        // A triggered run that never fired (#218): nothing was written, and
        // the line says so rather than "recorded 0 sample(s)".
        if self.pre_roll.is_none() && self.trigger.is_none() && self.out.is_none() {
            t.line(format!(
                "no rule fired in {:.1}s; nothing recorded",
                self.duration_ms as f64 / 1000.0
            ));
            return;
        }
        t.line(format!(
            "recorded {} sample(s) in {:.1}s{}",
            self.samples,
            self.duration_ms as f64 / 1000.0,
            self.out
                .as_deref()
                .map(|o| format!(" to {o}"))
                .unwrap_or_default(),
        ));
        if let Some(trigger) = &self.trigger {
            t.line(format!(
                "fired on {} at {} — {}",
                trigger.rule, trigger.at, trigger.evidence
            ));
        }
        if let Some(pre) = &self.pre_roll {
            t.line(format!(
                "pre-roll: {:.1}s of {:.1}s asked",
                pre.covered_s, pre.asked_s
            ));
        }
        if let Some(p) = &self.preamble {
            t.line(format!(
                "preamble: {} state row(s) ({}) over {:.2}s{}",
                p.count,
                match p.semantics {
                    zenkey_fleet::report::PreambleSemantics::AbsentFromWindow =>
                        "keys absent from the window",
                    zenkey_fleet::report::PreambleSemantics::Full => "full current state",
                },
                p.collected_over_s,
                if p.failed.is_empty() {
                    String::new()
                } else {
                    format!("; no state fetched for {}", p.failed.join(" + "))
                }
            ));
        }
    }

    fn bounds(&self) -> Vec<BoundCost> {
        let (evicted, expired) = self
            .pre_roll
            .as_ref()
            .map_or((0, 0), |p| (p.evicted, p.expired));
        vec![
            BoundCost::new(
                BoundKind::Missed,
                self.dropped,
                "sample(s) dropped while behind — recorded as in-file drop records \
                 where the gaps happened; the capture is a partial view and says so",
            ),
            // The ring's two eviction kinds, apart (v1.18 R1): the byte
            // budget biting narrows the pre-roll below what --pre claims;
            // ageing out is the window sliding as declared.
            BoundCost::new(
                BoundKind::Retired,
                evicted,
                "sample(s) evicted from the retained window by its byte budget — the \
                 pre-roll is narrower than --pre claims",
            ),
            BoundCost::new(
                BoundKind::Unwatched,
                expired,
                "sample(s) aged out of the retained window before the trigger — the \
                 pre-roll slid as declared",
            ),
            BoundCost::new(
                BoundKind::Refused,
                self.preamble.as_ref().map_or(0, |p| p.incomplete),
                "preamble reply(ies) not kept — error envelopes and replies past the \
                 bound; the state preamble is incomplete and says so",
            ),
        ]
    }

    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: self.header.selectors.clone(),
            // A triggered capture's window is what the ring covered plus the
            // post-roll — `duration_ms` is exactly that sum, never the
            // asked-for pre-roll.
            window_s: Some(self.duration_ms as f64 / 1000.0),
        })
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        // Version 3 (#612, FJ8a): what the selectors cannot reach, said —
        // an excluded verbatim chunk is not an empty one.
        if let Some(ex) = self.header.excluded.as_deref().filter(|e| !e.is_empty()) {
            notes.push(
                Note::coverage(format!(
                    "{} excluded: no watched selector names them, and `*`/`**` never match a \
                     verbatim chunk — the capture holds none of their traffic, which is not \
                     the same as there being none",
                    ex.join(", ")
                ))
                .cite("tooling guide O5"),
            );
        }
        if self.pre_roll.is_some() {
            notes.push(
                Note::coverage(
                    "the pre-roll covers only the watched selectors — the retained window is \
                     fed by the watch set, not the bus",
                )
                .cite("RFC 09 §5.1 O5"),
            );
        }
        if self.pre_roll.is_none() && self.trigger.is_none() && self.out.is_none() {
            notes.push(
                Note::silence(
                    "no rule fired within the wait — a rule not firing is not a finding, and \
                     silence is not a verdict",
                )
                .cite("RFC 05 §3.1"),
            );
        }
        notes
    }
}

impl Render for ReplayReport {
    const FAMILY: &'static str = "replay";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_of(self)
    }

    fn rows(&self, _out: &mut dyn FnMut(Row)) {}

    fn table(&self, t: &mut Table) {
        let verb = if self.dry_run {
            "would replay"
        } else {
            "replayed"
        };
        t.line(format!(
            "{verb} {} put(s) and {} tombstone(s) from {} (captured {})",
            self.published,
            self.tombstones,
            self.header.selectors.join(" + "),
            self.header.captured_at,
        ));
        if let Some(ns) = &self.namespace {
            t.line(format!(
                "into namespace {ns:?}, each key moved from the capture's base {:?}",
                self.header.base
            ));
        }
        if self.malformed > 0 || self.refused > 0 {
            let mut g = Grid::unheaded(1);
            for e in &self.first_errors {
                g.row([Cell::text(format!("  {e}"))]);
            }
            t.grid(g);
        }
        // The version-2 kinds, counted apart (RFC 13 §4.1): what the replay
        // did not publish and why, what it seeded, and what merely fired.
        if self.preamble_skipped > 0 {
            t.line(format!(
                "preamble rows skipped: {} (--seed-state to publish them)",
                self.preamble_skipped
            ));
        }
        if self.preamble_seeded > 0 {
            t.line(format!(
                "preamble rows {}seeded: {}",
                if self.dry_run { "would be " } else { "" },
                self.preamble_seeded
            ));
        }
        if self.triggers > 0 {
            t.line(format!(
                "trigger record(s) met: {} — markers, never published",
                self.triggers
            ));
        }
    }

    fn bounds(&self) -> Vec<BoundCost> {
        // Migrated from a hand-written note (which cited RFC 09 §5.2); the
        // auto-appended bound note carries the standard O6 citation.
        vec![BoundCost::new(
            BoundKind::Missed,
            self.capture_dropped,
            "sample(s) dropped at record time — this replay is a partial view \
             of a partial view",
        )]
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if self.malformed > 0 || self.refused > 0 {
            notes.push(Note::coverage(format!(
                "{} malformed row(s), {} refused row(s) — counted, not silently \
                 skipped",
                self.malformed, self.refused
            )));
        }
        if self.preamble_skipped > 0 {
            // Skipped is not lost: the row is in the file, and the reason it
            // stayed there is the section's whole argument.
            notes.push(
                Note::coverage(format!(
                    "{} preamble row(s) not published — state at capture start, \
                     re-stamped, would overwrite live state; --seed-state to mean it",
                    self.preamble_skipped
                ))
                .cite("RFC 13 §4.2"),
            );
        }
        notes
    }
}

impl Render for ExpectReport {
    const FAMILY: &'static str = "expect";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_of(self)
    }

    fn rows(&self, _out: &mut dyn FnMut(Row)) {}

    fn table(&self, t: &mut Table) {
        t.line(format!(
            "{} {} {}: {} sample(s) on {} key(s) over {:.1}s{}{}",
            self.address,
            self.iface,
            self.resource.as_deref().unwrap_or("(presence)"),
            self.samples,
            self.keys_seen,
            self.window_s,
            if self.ended_early { " (met early)" } else { "" },
            match self.rate_hz {
                Some(r) => format!(", {r:.2} Hz over the full window"),
                None => String::new(),
            }
        ));
        if let Some(p) = &self.presence {
            t.line(match (&p.error, p.holders.as_slice(), p.complete) {
                (Some(e), _, _) => format!("presence: not read — {e}"),
                (None, [], true) => "presence: no token visible to this reader".to_owned(),
                (None, [], false) => {
                    "presence: no token seen, and the read ended at its timeout".to_owned()
                }
                (None, holders, _) => format!("presence: held by {}", holders.join(", ")),
            });
        }
        if !self.violations.is_empty() {
            t.line(format!(
                "violations ({} shown of {}):",
                self.violations.len(),
                self.violations_total
            ));
            let mut g = Grid::unheaded(2);
            for v in &self.violations {
                g.row([Cell::text("  ✗"), Cell::text(v)]);
            }
            t.grid(g);
        }
        let (word, style) = match self.verdict {
            ExpectVerdict::Met => ("MET", crate::render::style::PASS),
            ExpectVerdict::NotMet => (
                "NOT MET — on a clean observation:",
                crate::render::style::ERROR,
            ),
            ExpectVerdict::Impaired => (
                "IMPAIRED — the observation cannot carry the claim:",
                crate::render::style::UNPROVEN,
            ),
        };
        t.line_styled(word, style);
        if !self.unmet.is_empty() {
            let mark = if matches!(self.verdict, ExpectVerdict::Impaired) {
                "  !"
            } else {
                "  ✗"
            };
            let mut g = Grid::unheaded(2);
            for u in &self.unmet {
                g.row([Cell::text(mark), Cell::text(u)]);
            }
            t.grid(g);
        }
    }

    fn bounds(&self) -> Vec<BoundCost> {
        vec![BoundCost::new(
            BoundKind::Missed,
            self.dropped,
            "sample(s) dropped while behind — counted into the verdict",
        )]
    }

    fn scope(&self) -> Option<ObservedScope> {
        let mut asked = self.selectors.clone();
        if let Some(p) = &self.presence {
            asked.push(p.selector.clone());
        }
        Some(ObservedScope {
            asked,
            window_s: Some(self.window_s),
        })
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if matches!(self.verdict, ExpectVerdict::Impaired) {
            // Impaired is not a third degree of failure: it is the absence of
            // a verdict, and the machine formats have to be able to tell.
            notes.push(
                Note::bound(
                    "the observation cannot carry the claim — this is not a verdict \
                     either way",
                )
                .cite("RFC 09 §5.1 O6"),
            );
        }
        notes
    }
}

/// `check probe` (#59; zk2's since #612, FJ8b): one resource read as a
/// consumer reads it. The verdict's word leads — a value arrived, nothing
/// usable did, or the silence could not be attributed — and the presence
/// read that attributes a silence is drawn only when it was made.
impl Render for zenkey_fleet::report::ProbeReport {
    const FAMILY: &'static str = "probe";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_of(self)
    }

    fn rows(&self, _out: &mut dyn FnMut(Row)) {}

    fn table(&self, t: &mut Table) {
        use zenkey_fleet::Judgement;
        t.line(format!(
            "probe {} {} {}: {} value(s) in {:.1}s ({} conforming, {} not){}",
            self.address,
            self.iface,
            self.resource,
            self.received,
            self.elapsed_s,
            self.conforming,
            self.nonconforming,
            match &self.current {
                Some(c) => match &c.error {
                    Some(e) => format!("; current state not read: {e}"),
                    None => format!(
                        "; current state: {} key(s), {} conforming",
                        c.answered, c.conforming
                    ),
                },
                None => String::new(),
            },
        ));
        if let Some(first) = &self.first {
            for line in crate::render::sample_lines(first) {
                t.line(format!("  {line}"));
            }
        }
        if let Some(p) = &self.presence {
            t.line(match (&p.error, p.holders.as_slice(), p.complete) {
                (Some(e), _, _) => format!("presence: not read — {e}"),
                (None, [], true) => "presence: no token visible to this reader".to_owned(),
                (None, [], false) => {
                    "presence: no token seen, and the read ended at its timeout".to_owned()
                }
                (None, holders, _) => {
                    format!("presence: held by {} — up, and silent", holders.join(", "))
                }
            });
        }
        let (word, style) = match &self.verdict {
            Judgement::NotEstablished { .. } => ("ARRIVED", crate::render::style::PASS),
            Judgement::Established => (
                "NOTHING USABLE ARRIVED — the finding",
                crate::render::style::ERROR,
            ),
            Judgement::Unobservable { .. } | Judgement::NotAsked => (
                "UNOBSERVABLE — the silence cannot be attributed",
                crate::render::style::UNPROVEN,
            ),
        };
        t.line_styled(word, style);
        if let Judgement::Unobservable { reason } = &self.verdict {
            t.line(format!("  ! {reason}"));
        }
    }

    fn bounds(&self) -> Vec<BoundCost> {
        vec![BoundCost::new(
            BoundKind::Missed,
            self.lagged,
            "value(s) arrived past this tool's buffer, not inspected",
        )]
    }

    fn scope(&self) -> Option<ObservedScope> {
        let mut asked = self.selectors.clone();
        if let Some(p) = &self.presence {
            asked.push(p.selector.clone());
        }
        Some(ObservedScope {
            asked,
            window_s: Some(self.window_s),
        })
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if self.discarded > 0 {
            notes.push(
                Note::coverage(format!(
                    "{} sample(s) on a wildcard key discarded by rule — not values",
                    self.discarded
                ))
                .cite("spec §3.2 R6"),
            );
        }
        if self.unresolved > 0 {
            notes.push(Note::coverage(format!(
                "{} sample(s) on a key that resolved to no member of the resource (#671)",
                self.unresolved
            )));
        }
        if self
            .presence
            .as_ref()
            .is_some_and(|p| p.holders.is_empty() && p.complete)
        {
            notes.push(
                Note::caveat(
                    "a read access control refused is complete and empty too: no token is \
                     what this reader could see",
                )
                .cite("spec §8.1"),
            );
        }
        notes
    }
}
