//! `zenctl field` (#223; zk2's since #612, FJ8b) — per-path statistics plus
//! the field-granular findings, one family.
//!
//! The honesty load here is the module's whole reason: the path table is
//! bounded and its cost is a [`Note`] in every format (O6); samples with no
//! document are counted apart from absence, a key no contract resolved is
//! judged for nothing that needs its type rather than read as clean, and
//! `field-stuck` is said to be not asked — its freshness is a profile's
//! (#613) — never silently absent (O4).

use zenkey_fleet::report::FieldReport;

use crate::render::{
    BoundCost, BoundKind, Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_without,
};

impl Render for FieldReport {
    const FAMILY: &'static str = "field";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        // Everything except the rows and findings — the coverage and bound
        // claims must survive a truncated pipe.
        envelope_without(self, &["rows", "findings"])
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
            detail.push(match r.declared {
                Some(true) => "declared".to_string(),
                Some(false) => "NOT declared by its type".to_string(),
                // No declared surface: unjudgeable, which is not undeclared.
                None => "—".to_string(),
            });
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
                let mark = crate::render::style::mark(f.severity);
                grid.row([
                    Cell::styled(mark, crate::render::style::severity(f.severity)),
                    Cell::text(format!(
                        "{}: {} — {}",
                        f.check.as_str(),
                        f.subject,
                        f.evidence
                    )),
                ]);
            }
            t.grid(grid);
        }
    }

    fn bounds(&self) -> Vec<BoundCost> {
        // The examples behind the refused count (`paths_dropped_examples`)
        // and the bound itself (`max_paths`) ride the document; the note
        // carries the cost and its reading.
        vec![
            BoundCost::new(
                BoundKind::Missed,
                self.dropped,
                "sample(s) dropped while behind — every per-path count covers \
                 only what was seen",
            ),
            BoundCost::new(
                BoundKind::Refused,
                self.paths_dropped,
                "path observation(s) refused at the path-table bound; stats \
                 cover the tracked set",
            ),
        ]
    }

    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: vec![self.selector.clone()],
            window_s: Some(self.window_s),
        })
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        let mut coverage = format!(
            "watched {} for {:.0}s: {} sample(s) on {} key(s), {} path(s) tracked",
            self.selector, self.window_s, self.samples, self.keys_seen, self.paths
        );
        if self.undocumented > 0 {
            coverage.push_str(&format!(
                "; {} sample(s) carried no document (a raw type, bytes that do not \
                 decode, a deletion) — fields are unobservable for them, which is not \
                 absence",
                self.undocumented
            ));
        }
        notes.push(Note::coverage(coverage).cite("tooling guide O5"));
        if self.unresolved > 0 {
            notes.push(
                Note::coverage(format!(
                    "{} sample(s) on keys no contract resolved were observed \
                     structurally: field-new is unjudgeable for them, which is not \
                     the same as clean",
                    self.unresolved
                ))
                .cite("tooling guide O4"),
            );
        }
        notes.push(
            Note::coverage(
                "field-stuck was not asked: it judges against a declared freshness, \
                 the freshness.v1 profile's (#613)",
            )
            .cite("tooling guide O4"),
        );
        if self.findings.is_empty() {
            notes.push(Note::summary(format!(
                "no findings over this {:.0}s window — which scopes the claim: a \
                 longer window can still say what a short one cannot.",
                self.window_s
            )));
        } else {
            notes.push(Note::summary(format!(
                "{} finding(s) across field-vanished / field-new.",
                self.findings.len()
            )));
        }
        notes
    }
}
