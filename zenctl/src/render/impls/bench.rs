//! `bench rpc`: the one family that is a real grid — a header and six numeric
//! columns — and the one where "a truncated number is a wrong number" is not
//! an abstract rule.

use zenkey_fleet::report::BenchReport;

use crate::render::{Cell, Grid, Note, ObservedScope, Render, Row, Table};

impl Render for BenchReport {
    const FAMILY: &'static str = "bench";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = serde_json::Map::new();
        e.insert("key".into(), self.key.clone().into());
        e.insert("requested".into(), self.requested.into());
        e.insert("completed".into(), self.completed.into());
        e.insert("concurrency".into(), self.concurrency.into());
        e.insert(
            "elapsed_s".into(),
            serde_json::Number::from_f64(self.elapsed_s)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
        );
        e.insert(
            "calls_per_s".into(),
            serde_json::Number::from_f64(self.calls_per_s)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
        );
        // Counted apart from the latency rows on purpose: averaging a
        // non-answer into a latency figure is how a benchmark lies.
        e.insert("errors".into(), self.errors.into());
        e.insert("silent".into(), self.silent.into());
        // A third population, apart from both (#329): a panicked call is news
        // about this tool, not about the fleet, and it belongs in neither of
        // the other two counters.
        e.insert("panicked".into(), self.panicked.into());
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for o in &self.origins {
            out(Row::of("origin", o));
        }
    }

    fn table(&self, t: &mut Table) {
        t.line(format!("→ {}", self.key));
        t.line(format!(
            "{} call(s), concurrency {}, {:.2}s — {:.1} calls/s",
            self.completed, self.concurrency, self.elapsed_s, self.calls_per_s
        ));
        if self.completed < self.requested {
            t.line(format!(
                "  {} of {} calls did not complete",
                self.requested - self.completed,
                self.requested
            ));
        }
        if self.panicked > 0 {
            t.line(format!(
                "  {} of those panicked in this tool — measured nothing",
                self.panicked
            ));
        }
        if self.origins.is_empty() {
            return;
        }
        t.blank();
        let mut grid = Grid::new([
            "origin", "replies", "min ms", "p50 ms", "p95 ms", "p99 ms", "max ms",
        ])
        .max(0, 16);
        for o in &self.origins {
            grid.row([
                Cell::text(&o.origin),
                Cell::int(o.replies as u64),
                Cell::num(o.min_ms, 2),
                Cell::num(o.p50_ms, 2),
                Cell::num(o.p95_ms, 2),
                Cell::num(o.p99_ms, 2),
                Cell::num(o.max_ms, 2),
            ]);
        }
        t.grid(grid);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if self.origins.is_empty() {
            notes.push(Note::silence(
                "no origin answered — a non-verdict, not proof of absence; \
                 `zenctl node list` says who is up",
            ));
        }
        if self.errors > 0 || self.silent > 0 {
            notes.push(Note::coverage(format!(
                "{} error repl(ies), {} call(s) drew no reply at all — counted apart \
                 from the latencies above, because averaging a non-answer into a \
                 latency figure is how a benchmark lies",
                self.errors, self.silent
            )));
        }
        if self.panicked > 0 {
            notes.push(Note::coverage(format!(
                "{} call(s) panicked inside this tool and are counted apart from \
                 both — a panicked call is not an error reply and not attributable \
                 silence, and folding it into either would report a bug here as a \
                 fact about the fleet",
                self.panicked
            )));
        }
        notes
    }

    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: vec![self.key.clone()],
            window_s: Some(self.elapsed_s),
        })
    }
}
