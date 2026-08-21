//! `topic hz` / `topic bw` — the family with view flags, and the reason two
//! of the three were never view flags at all.
//!
//! `output::rate` took `bandwidth`, `loss` and `latency`. Under the trait a
//! report renders itself and takes no options, so each had to be accounted
//! for:
//!
//! * **`loss`** was already encoded correctly. `RateReport.sn_gaps` is an
//!   `Option`, and its doc says `None` = `--loss` was not asked — RFC 09 §5.1
//!   O4 in one field. The table shows the clause iff the field is there.
//! * **`latency`** was a bug, and its own issue (#238): the report carried a
//!   full latency distribution whether or not anyone asked, while the caveat
//!   naming *which clock* it came from printed only under `--latency`. Fixed
//!   at the source, in `cmd/rate.rs`, so the field is absent when unasked.
//! * **`bandwidth`** is the one real view choice — B/s or Hz over facts both
//!   present — and it gets a wrapper rather than an associated `Opts` type
//!   that 23 other families would have to spell as `()`.

use zenkey_fleet::report::RateReport;

use crate::render::{Cell, Grid, Note, Render, Row, Table};

/// A rate report plus the one thing about it that is a *view*.
///
/// `Serialize` forwards to the report, so the wire shape is the report's and
/// the flag exists only for the table.
pub struct RateView<'a> {
    pub report: &'a RateReport,
    pub bandwidth: bool,
}

impl serde::Serialize for RateView<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.report.serialize(s)
    }
}

impl Render for RateView<'_> {
    const FAMILY: &'static str = "rate";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        // Derived from the struct, so `sn_gaps` keeps its `skip_serializing_if`
        // — the hand-built trailing envelope wrote it unconditionally and
        // nulled it when `--loss` was not asked, which is the O4 inversion
        // #232's fourth item names.
        let mut e = match serde_json::to_value(self.report).expect("a report serializes") {
            serde_json::Value::Object(m) => m,
            _ => unreachable!("a report is an object"),
        };
        e.remove("rows");
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for r in &self.report.rows {
            out(Row::of("key", r));
        }
    }

    fn table(&self, t: &mut Table) {
        let secs = self.report.window_s as f64;
        let mut g = Grid::unheaded(2).right(0);
        for row in &self.report.rows {
            if self.bandwidth {
                g.row([
                    Cell::text(format!("{:.1} B/s", row.bytes as f64 / secs)),
                    Cell::text(&row.key),
                ]);
                continue;
            }
            let mut tail = row.key.clone();
            if self.report.sn_gaps.is_some() {
                tail.push_str(&format!("  ({} sn gap(s))", row.sn_gaps));
            }
            match &row.latency {
                // One clause per population, never one median across them: a
                // publisher-stamped sample and a router-stamped one measure
                // from different clocks (#213).
                Some(l) => {
                    for (label, s) in l.populations() {
                        tail.push_str(&format!(
                            "  lat[{label}] med {} p95 {} (min {} max {}, {})",
                            human_us(s.median_us),
                            human_us(s.p95_us),
                            human_us(s.min_us),
                            human_us(s.max_us),
                            s.samples,
                        ));
                    }
                    tail.push_str(&format!("  ({} unstamped)", row.unstamped));
                }
                None if row.unstamped > 0 => tail.push_str(&format!(
                    "  lat — ({} unstamped: no HLC, no latency — not zero)",
                    row.unstamped
                )),
                None => {}
            }
            g.row([
                Cell::text(format!("{:.2} Hz", row.count as f64 / secs)),
                Cell::text(tail),
            ]);
        }
        t.grid(g);
        t.line(if self.bandwidth {
            format!(
                "total: {:.1} B/s over {} key(s) ({} bytes / {}s)",
                self.report.total_bytes as f64 / secs,
                self.report.keys,
                self.report.total_bytes,
                self.report.window_s
            )
        } else {
            format!(
                "total: {:.2} Hz over {} key(s) ({} samples / {}s)",
                self.report.total_count as f64 / secs,
                self.report.keys,
                self.report.total_count,
                self.report.window_s
            )
        });
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        // The caveat is part of the measurement (#119) and names *which*
        // clock (#213). Worded by the engine so the GUI cannot describe the
        // same number differently (RFC 09 §5.1 O7).
        if let Some(l) = self.report.rows.iter().find_map(|r| r.latency.as_ref()) {
            notes.push(Note::caveat(format!("latency = {}", l.caveat())).cite("RFC 09 §5.1 O7"));
        }
        if let Some(gaps) = self.report.sn_gaps {
            notes.push(Note::coverage(format!(
                "{gaps} source-sn gap(s) — zero also means \"publishers attach no \
                 SourceInfo\", which is an observation rather than proof of \
                 losslessness"
            )));
        }
        if self.report.evicted > 0 {
            // The table is bounded; a shrunken key set must say so.
            notes.push(Note::bound(format!(
                "{} key(s) retired to stay within the {}-key bound — totals cover the \
                 retained set",
                self.report.evicted, self.report.max_keys
            )));
        }
        notes
    }
}

/// Microseconds, humanised with the sign kept — a negative latency is the skew
/// evidence (#119), not an error.
fn human_us(us: i64) -> String {
    let sign = if us < 0 { "-" } else { "" };
    let a = us.unsigned_abs();
    if a < 1_000 {
        format!("{sign}{a}µs")
    } else if a < 1_000_000 {
        format!("{sign}{:.1}ms", a as f64 / 1_000.0)
    } else {
        format!("{sign}{:.2}s", a as f64 / 1_000_000.0)
    }
}
