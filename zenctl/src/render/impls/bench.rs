//! `bench call` (#612, FJ8a): the one family that is a real grid — a header
//! and six numeric columns — and the one where "a truncated number is a
//! wrong number" is not an abstract rule.
//!
//! The populations stay apart in every medium: the latency rows are value
//! replies by the key they went on (O3); refusals, malformed envelopes, the
//! transport's errors, silent calls, R6's discards and panicked calls are
//! counted beside them and never averaged in (the tooling guide's O6), and
//! each token holder is tallied against the calls it sent no value in.

use zenkey_fleet::report::{BenchReport, CallMode, Latency};

use super::zk2::short_fp;
use crate::render::{Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_without};

fn latency_cells(l: &Latency) -> [Cell; 5] {
    [
        Cell::num(l.min_ms, 2),
        Cell::num(l.p50_ms, 2),
        Cell::num(l.p95_ms, 2),
        Cell::num(l.p99_ms, 2),
        Cell::num(l.max_ms, 2),
    ]
}

impl Render for BenchReport {
    const FAMILY: &'static str = "bench";

    /// The whole report, less the repliers, which are the rows. The
    /// non-answers stay on the envelope: averaging one into a latency
    /// figure is how a benchmark lies.
    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_without(self, &["repliers"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for r in &self.repliers {
            out(Row::of("replier", r));
        }
    }

    fn table(&self, t: &mut Table) {
        t.line(format!(
            "bench {} {}@{} {}  ({}, {}s per call)",
            self.address,
            self.iface,
            short_fp(&self.fingerprint),
            self.operation,
            match self.mode {
                CallMode::Concrete => "one address",
                CallMode::Fanout => "fan-out",
            },
            self.timeout_s
        ));
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
        if !self.repliers.is_empty() {
            t.blank();
            let mut grid = Grid::new([
                "replier", "replies", "min ms", "p50 ms", "p95 ms", "p99 ms", "max ms",
            ])
            .max(0, 24);
            for r in &self.repliers {
                let [a, b, c, d, e] = latency_cells(&r.latency);
                grid.row([Cell::text(&r.address), Cell::int(r.replies), a, b, c, d, e]);
            }
            t.grid(grid);
        }
        if self.refusals.count > 0 {
            let codes: Vec<String> = self
                .refusals
                .codes
                .iter()
                .map(|(c, n)| format!("{c} ×{n}"))
                .collect();
            t.line(format!(
                "refused {} time(s), unattributed: {}{}",
                self.refusals.count,
                codes.join(", "),
                self.refusals
                    .latency
                    .map(|l| format!(" (p50 {:.2} ms)", l.p50_ms))
                    .unwrap_or_default()
            ));
        }
        for h in self.presence.holders.iter().filter(|h| h.without_value > 0) {
            t.line(format!(
                "{} holds the interface's token and sent no value in {} call(s): refused or \
                 silent, which a caller cannot tell apart",
                h.address, h.without_value
            ));
        }
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = vec![
            Note::caveat(
                "every latency is this tool's round trip, query sent to reply received — never \
                 a stamp",
            )
            .cite("tooling guide O7"),
        ];
        if self.repliers.is_empty() {
            notes.push(
                Note::silence(
                    "no value from any replier — a non-verdict, not proof of absence; `zenctl \
                     service list` says who is up",
                )
                .cite("spec §5.1 O5"),
            );
        }
        let apart = [
            ("refusal(s)", self.refusals.count),
            ("malformed envelope(s)", self.malformed),
            ("transport error(s)", self.transport),
            ("silent call(s)", self.silent),
            ("discarded value(s) (R6)", self.discarded),
            ("call(s) that panicked in this tool", self.panicked),
        ];
        let nonzero: Vec<String> = apart
            .iter()
            .filter(|(_, n)| *n > 0)
            .map(|(what, n)| format!("{n} {what}"))
            .collect();
        if !nonzero.is_empty() {
            notes.push(Note::coverage(format!(
                "{} — counted apart from the latencies above, because averaging a non-answer \
                 into a latency figure is how a benchmark lies",
                nonzero.join(", ")
            )));
        }
        if !self.presence.complete {
            notes.push(
                Note::coverage(match &self.presence.error {
                    Some(e) => format!(
                        "the selection's presence could not be read ({e}): who sent no value \
                         is not known"
                    ),
                    None => "the selection's presence read may be incomplete: a holder that \
                             sent no value may be missing"
                        .into(),
                })
                .cite("spec §8.1"),
            );
        }
        notes
    }

    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: self.selectors.clone(),
            window_s: Some(self.elapsed_s),
        })
    }
}
