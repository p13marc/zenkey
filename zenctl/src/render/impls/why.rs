//! The `why` ladder (#214): one line per rung, evidence indented, and the
//! three-state answer kept three-state all the way to the terminal — a rung
//! whose input was not fetched draws `?` and says why, never `✗` (RFC 09
//! §5.1 O4).

use zenkey_fleet::judge::why::is_cause;

use zenkey_fleet::report::{RungAnswer, WhyReport, WhyVerdict};

use crate::render::{Cell, Grid, Note, ObservedScope, Render, Row, Table};

impl Render for WhyReport {
    const FAMILY: &'static str = "why";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        // Everything except the rungs themselves — the verdict and the
        // impairments must survive a truncated pipe — plus the cause ids, so
        // a script need not re-derive the exit-0 policy.
        let mut e = match serde_json::to_value(self).expect("a report serializes") {
            serde_json::Value::Object(m) => m,
            _ => unreachable!("a report is an object"),
        };
        e.remove("rungs");
        e.insert("causes".into(), serde_json::json!(self.causes()));
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for r in &self.rungs {
            out(Row::of("rung", r));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut grid = Grid::unheaded(3);
        for r in &self.rungs {
            // The word carries the distinction; the mark and its colour only
            // repeat it (#200). `?` is dim rather than red: the absence of an
            // answer, not a milder failure.
            let mark = match &r.answer {
                RungAnswer::Established => Cell::styled("✓", crate::render::style::PASS),
                RungAnswer::NotEstablished { .. } if is_cause(r.id, &r.answer) => {
                    Cell::styled("✗", crate::render::style::ERROR)
                }
                RungAnswer::NotEstablished { .. } => Cell::text("·"),
                RungAnswer::NotAsked => Cell::styled("?", crate::render::style::UNPROVEN),
                // The ladder degrades an uncarriable observation to NotAsked
                // today; the pole exists in the core (RFC 13, v1.24) and a
                // future rung that emits it draws as the absence of a
                // verdict, never as a `✗`.
                RungAnswer::Unobservable { .. } => {
                    Cell::styled("!", crate::render::style::UNPROVEN)
                }
            };
            grid.row([mark, Cell::text(r.id.as_str()), Cell::text(r.question)]);
            if let RungAnswer::NotEstablished { reason } | RungAnswer::Unobservable { reason } =
                &r.answer
            {
                grid.detail([format!("      ↳ {reason}")]);
            }
            grid.detail(r.evidence.iter().map(|e| format!("      {e}")));
        }
        t.grid(grid);
        let (word, style) = match self.verdict {
            WhyVerdict::Explained => ("EXPLAINED", crate::render::style::PASS),
            // Dim, not green: the absence of an explanation, honestly held.
            WhyVerdict::Healthy => ("NO CAUSE ESTABLISHED", crate::render::style::UNPROVEN),
            WhyVerdict::Impaired => ("IMPAIRED", crate::render::style::ERROR),
        };
        t.line_styled(word, style);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = vec![Note::silence(
            "silence is never a verdict — this ladder itemises why the question \
             \"is it publishing?\" is unanswerable rather than answering it",
        )];
        for impairment in &self.impairments {
            notes.push(
                Note::coverage(format!("{impairment} — not asked is not answered no"))
                    .cite("RFC 09 §5.1 O4"),
            );
        }
        match self.listened_s {
            Some(s) => notes.push(Note::coverage(format!(
                "listened {s:.0}s on the asked key — the run's one data-plane cost"
            ))),
            None => notes.push(Note::next_step(
                "pass --for <SECS> to add the wire-heard rung — the only rung \
                 that costs the data plane",
            )),
        }
        let causes = self.causes();
        notes.push(match self.verdict {
            WhyVerdict::Explained => Note::summary(format!(
                "an explanation was established by: {} (exit 0).",
                causes
                    .iter()
                    .map(|c| c.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            WhyVerdict::Healthy => Note::summary(
                "no cause established and everything checked looks healthy (exit 1) \
                 — a key nothing has published yet looks exactly like this \
                 (publishers declare lazily, RFC 08 §6.1).",
            ),
            WhyVerdict::Impaired => Note::summary(
                "no cause established, and the observation was impaired (exit 2) — \
                 the unasked rungs above are why \"healthy\" cannot be claimed.",
            ),
        });
        notes
    }

    /// The asked key; the window is the opt-in listen (`None` = the run
    /// cost the control plane only, and there was no window).
    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: vec![self.key.clone()],
            window_s: self.listened_s,
        })
    }
}
