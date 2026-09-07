//! The four families that end something and report what happened: `record`,
//! `replay`, `check cutover`, `check expect`.
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

use zenkey_fleet::report::{CutoverReport, CutoverVerdict, ExpectReport, ExpectVerdict};
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
                "{} malformed row(s), {} refused delete row(s) — counted, not silently \
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

impl Render for CutoverReport {
    const FAMILY: &'static str = "cutover";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_of(self)
    }

    fn rows(&self, _out: &mut dyn FnMut(Row)) {}

    fn table(&self, t: &mut Table) {
        t.line(format!(
            "old root {}: {} sample(s) on {} key(s) over {}s",
            self.old_root, self.old_samples, self.old_keys_seen, self.window_s
        ));
        let mut g = Grid::unheaded(2);
        for k in &self.old_examples {
            g.row([Cell::text("  ✗"), Cell::text(k)]);
        }
        t.grid(g);
        t.line(format!(
            "new plane {}**: {} sample(s)",
            self.new_prefix, self.new_samples
        ));
        if self.leak_samples > 0 {
            t.line(format!(
                "leaks (outside {} and not the old root): {} sample(s) on {} key(s)",
                self.new_prefix, self.leak_samples, self.leaked_keys_seen
            ));
            let mut g = Grid::unheaded(2);
            for k in &self.leak_examples {
                g.row([Cell::text("  !"), Cell::text(k)]);
            }
            t.grid(g);
        }
        // The word is the carrier; the colour repeats it (#200).
        let (word, style) = match self.verdict {
            CutoverVerdict::Pass => ("PASS", crate::render::style::PASS),
            CutoverVerdict::OldStillSpeaks => ("FAIL", crate::render::style::ERROR),
            // Dim, not yellow: "unproven" is the absence of a verdict rather
            // than a milder failure.
            CutoverVerdict::Unproven => ("UNPROVEN", crate::render::style::UNPROVEN),
        };
        t.line_styled(word, style);
    }

    fn bounds(&self) -> Vec<BoundCost> {
        vec![BoundCost::new(
            BoundKind::Missed,
            self.dropped,
            "sample(s) dropped while behind — the silence claim covers only \
             what was seen",
        )]
    }

    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: vec![self.old_root.clone(), format!("{}**", self.new_prefix)],
            window_s: Some(self.window_s),
        })
    }

    fn notes(&self) -> Vec<Note> {
        // The verdict word is on stdout beside the evidence; the sentence that
        // says what it *means* is a note, so a script gets it too.
        vec![match self.verdict {
            CutoverVerdict::Pass => Note::coverage(
                "the retired family is silent while the new plane carries traffic — \
                 both halves",
            )
            .cite("RFC 09 §6"),
            CutoverVerdict::OldStillSpeaks => Note::coverage(
                "the retired family still speaks; a migration you can assert the \
                 absence of is a migration you can finish",
            )
            .cite("RFC 09 §6"),
            CutoverVerdict::Unproven => Note::silence(
                "the old root was silent but so was the new plane: a dead fleet \
                 passes the silence half for free. Bring the fleet up and run it again",
            ),
        }]
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
            "{}: {} sample(s) on {} key(s) over {:.1}s{}{}",
            self.selector,
            self.samples,
            self.keys_seen,
            self.window_s,
            if self.ended_early { " (met early)" } else { "" },
            match self.rate_hz {
                Some(r) => format!(", {r:.2} Hz over the full window"),
                None => String::new(),
            }
        ));
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
        Some(ObservedScope {
            asked: vec![self.selector.clone()],
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
