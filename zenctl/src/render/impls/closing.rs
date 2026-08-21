//! The four families that end something and report what happened: `record`,
//! `replay`, `cutover`, `expect`.
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

use crate::render::{Cell, Grid, Note, Render, Row, Table};

/// The struct as its own envelope.
fn envelope_of<T: serde::Serialize>(v: &T) -> serde_json::Map<String, serde_json::Value> {
    match serde_json::to_value(v).expect("a report serializes") {
        serde_json::Value::Object(m) => m,
        _ => unreachable!("a report is an object"),
    }
}

impl Render for RecordReport {
    const FAMILY: &'static str = "record";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_of(self)
    }

    fn rows(&self, _out: &mut dyn FnMut(Row)) {}

    fn table(&self, t: &mut Table) {
        t.line(format!(
            "recorded {} sample(s) in {:.1}s{}",
            self.samples,
            self.duration_ms as f64 / 1000.0,
            self.out
                .as_deref()
                .map(|o| format!(" to {o}"))
                .unwrap_or_default(),
        ));
    }

    fn notes(&self) -> Vec<Note> {
        if self.dropped == 0 {
            return Vec::new();
        }
        vec![Note::bound(format!(
            "{} sample(s) dropped while behind — recorded as in-file drop records \
             where the gaps happened; the capture is a partial view and says so",
            self.dropped
        ))]
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
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if self.capture_dropped > 0 {
            notes.push(
                Note::bound(format!(
                    "the capture dropped {} sample(s) at record time — this replay is \
                     a partial view of a partial view",
                    self.capture_dropped
                ))
                .cite("RFC 09 §5.2"),
            );
        }
        if self.malformed > 0 || self.refused > 0 {
            notes.push(Note::coverage(format!(
                "{} malformed row(s), {} refused delete row(s) — counted, not silently \
                 skipped",
                self.malformed, self.refused
            )));
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

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if self.dropped > 0 {
            notes.push(Note::bound(format!(
                "{} sample(s) dropped while behind — the silence claim covers only \
                 what was seen",
                self.dropped
            )));
        }
        // The verdict word is on stdout beside the evidence; the sentence that
        // says what it *means* is a note, so a script gets it too.
        notes.push(match self.verdict {
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
        });
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

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if self.dropped > 0 {
            notes.push(Note::bound(format!(
                "{} sample(s) dropped while behind — counted into the verdict",
                self.dropped
            )));
        }
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
