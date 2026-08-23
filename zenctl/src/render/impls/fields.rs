//! `zenctl field` (#223) — per-path statistics plus the three field-granular
//! findings, one family.
//!
//! The honesty load here is the module's whole reason: the path table is
//! bounded and its cost is a [`Note`] in every format (RFC 09 §5.1 O6);
//! samples with no structural document are counted apart from absence, a
//! missing registry makes `field-stuck`/`field-new` unjudgeable rather than
//! silently clean (O4), and a stuck reading carries its own "observation,
//! not a verdict" caveat so nobody pages off a constant-by-design field.

use zenkey_fleet::report::{DoctorSeverity, FieldReport};

use crate::render::{Cell, Grid, Note, Render, Row, Table};

impl Render for FieldReport {
    const FAMILY: &'static str = "field";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        // Everything except the rows and findings — the coverage and bound
        // claims must survive a truncated pipe.
        let mut e = match serde_json::to_value(self).expect("a report serializes") {
            serde_json::Value::Object(m) => m,
            _ => unreachable!("a report is an object"),
        };
        e.remove("rows");
        e.remove("findings");
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for r in &self.rows {
            out(Row::of("path", r));
        }
        for f in &self.findings {
            out(Row::of("finding", f));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut grid = Grid::unheaded(3).right(0);
        for r in &self.rows {
            let mut detail = vec![r.kinds.join("+")];
            detail.push(match (r.changes, r.last_change_s) {
                (0, _) => "unchanged".to_string(),
                (n, Some(at)) => format!("{n} change(s), last at {at:.1}s"),
                (n, None) => format!("{n} change(s)"),
            });
            if let (Some(min), Some(max), Some(last)) = (r.min, r.max, r.last) {
                detail.push(format!("min {min} max {max} last {last}"));
            }
            match &r.values {
                Some(values) => detail.push(format!("values {{{}}}", values.join(", "))),
                // Absent = the domain outgrew the cap — stated, not blank.
                None => detail.push("values: not a small domain".to_string()),
            }
            grid.row([
                Cell::text(format!("{}/{}", r.seen, r.documents)),
                Cell::text(format!("{} · {}", r.key, r.path)),
                Cell::text(detail.join("  ")),
            ]);
        }
        t.grid(grid);
        if !self.findings.is_empty() {
            t.blank();
            let mut grid = Grid::unheaded(2);
            for f in &self.findings {
                let mark = match f.severity {
                    DoctorSeverity::Error => "✗",
                    DoctorSeverity::Warning => "⚠",
                    DoctorSeverity::Info => "·",
                };
                let citation = f
                    .citation
                    .as_deref()
                    .map(|c| format!("  [{c}]"))
                    .unwrap_or_default();
                grid.row([
                    Cell::styled(mark, crate::render::style::severity(f.severity)),
                    Cell::text(format!(
                        "{}: {} — {}{citation}",
                        f.check, f.subject, f.evidence
                    )),
                ]);
            }
            t.grid(grid);
        }
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        let mut coverage = format!(
            "watched {} for {:.0}s: {} sample(s) on {} key(s), {} path(s) tracked",
            self.selector, self.window_s, self.samples, self.keys_seen, self.paths
        );
        if self.undocumented > 0 {
            coverage.push_str(&format!(
                "; {} sample(s) carried no structural document — fields are \
                 unobservable for them, which is not absence",
                self.undocumented
            ));
        }
        notes.push(Note::coverage(coverage).cite("RFC 09 §5.1 O5"));
        if !self.registry_loaded {
            notes.push(
                Note::coverage(
                    "no registry loaded — declared ttl_s and type names are unknown, \
                     so field-stuck and field-new are unjudgeable, which is not the \
                     same as clean",
                )
                .cite("RFC 09 §5.1 O4"),
            );
        }
        if self.dropped > 0 {
            notes.push(Note::bound(format!(
                "{} sample(s) dropped while behind — every per-path count covers \
                 only what was seen",
                self.dropped
            )));
        }
        if self.paths_dropped > 0 {
            let examples = if self.paths_dropped_examples.is_empty() {
                String::new()
            } else {
                format!(" — e.g. {}", self.paths_dropped_examples.join(", "))
            };
            notes.push(Note::bound(format!(
                "path table full at {}: {} path observation(s) refused{examples}; \
                 stats cover the tracked set",
                self.max_paths, self.paths_dropped
            )));
        }
        if self.findings.is_empty() {
            notes.push(Note::summary(format!(
                "no findings over this {:.0}s window — which scopes the claim: a \
                 longer window can still say what a short one cannot.",
                self.window_s
            )));
        } else {
            notes.push(
                Note::caveat(
                    "a stuck reading is an observation with a stated window, not a \
                     verdict — a constant-by-design field always reads this way",
                )
                .cite("RFC 04 §1.2"),
            );
            notes.push(Note::summary(format!(
                "{} finding(s) across field-vanished / field-stuck / field-new.",
                self.findings.len()
            )));
        }
        notes
    }
}
