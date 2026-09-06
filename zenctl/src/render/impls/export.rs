//! `export --once` (#228) — one fold of the metrics surface, as a report.
//!
//! The table is the series, grouped by producer, with the state beside
//! every value and an empty value cell — *asked and absent*, never `—` —
//! where the state says the series stopped. The observer's own bounds
//! ride as `bounds()`, one cost per O6 population, so the emit path writes
//! them in every format and never sums them; the unasked poles (registry,
//! doctor, payload verdicts) are coverage notes citing O4.

use zenkey_fleet::ExportSnapshot;
use zenkey_fleet::report::SeriesState;

use crate::render::{
    BoundCost, BoundKind, Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_without,
};

impl Render for ExportSnapshot {
    const FAMILY: &'static str = "export";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_without(self, &["series"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for s in &self.series {
            out(Row::of("series", s));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut g = Grid::new(["series", "labels", "value", "state", "last seen", "samples"])
            .right(2)
            .right(5);
        let mut producer: Option<&str> = None;
        for s in &self.series {
            if producer != Some(s.producer.as_str()) {
                g.group(format!("{}  ({})", s.producer, s.class));
                producer = Some(s.producer.as_str());
            }
            let mut labels: Vec<String> = vec![format!("origin={}", s.origin)];
            labels.extend(s.labels.iter().map(|(k, v)| format!("{k}={v}")));
            if let Some(f) = &s.field {
                labels.push(format!("field={f}"));
            }
            g.row([
                Cell::text(&s.name),
                Cell::text(labels.join(" ")),
                // A stopped series has no value to show: empty, not `—`,
                // because the question was asked and the answer is "it
                // stopped" (the state cell says how).
                match s.value {
                    Some(v) => Cell::num(v, 3),
                    None => Cell::text(""),
                },
                Cell::text(s.state.as_str()),
                Cell::int(s.last_seen_unix_s),
                Cell::int(s.samples),
            ]);
        }
        t.grid(g);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if !self.excluded.is_empty() {
            notes.push(
                Note::coverage(format!(
                    "a wildcard selector never crosses an `@`-chunk: {} are excluded from \
                     this surface, not empty",
                    self.excluded.join(", ")
                ))
                .cite("RFC 03 §4 D2"),
            );
        }
        if self.registry.is_not_asked() {
            notes.push(
                Note::coverage(
                    "no registry loaded — no key refines to a declared subject, so every key \
                     is counted unregistered and no subject series exists; that is a fact \
                     about this run, not about the fleet",
                )
                .cite("RFC 09 §5.1 O4"),
            );
        }
        if self.unregistered_keys > 0 {
            notes.push(
                Note::coverage(format!(
                    "{} distinct key(s) the registry does not declare are counted, never \
                     exported — the contract is the registry",
                    self.unregistered_keys
                ))
                .cite("RFC 09 §5.1 O4"),
            );
        }
        if self.contract.payload_valid == 0 && self.contract.payload_invalid == 0 {
            notes.push(
                Note::coverage(format!(
                    "payload verdicts not asked — {} sample(s) not validated; pass --validate",
                    self.contract.payload_not_validated
                ))
                .cite("RFC 09 §5.1 O4"),
            );
        } else if self.contract.payload_not_validated > 0 {
            notes.push(Note::caveat(format!(
                "{} sample(s) not validated (past the decode budget, or no schema) — a \
                 third population beside {} valid and {} invalid, never folded into a ratio",
                self.contract.payload_not_validated,
                self.contract.payload_valid,
                self.contract.payload_invalid
            )));
        }
        if self.doctor.is_not_asked() {
            notes.push(
                Note::coverage("doctor not asked — pass --doctor-every").cite("RFC 09 §5.1 O4"),
            );
        }
        let stopped = self
            .series
            .iter()
            .filter(|s| !s.state.exposes_value())
            .count();
        if stopped > 0 {
            notes.push(Note::caveat(format!(
                "{stopped} series stopped (evicted, origin_down or retired): each keeps its \
                 labels and state and exposes no value, so a scraper sees a named absence \
                 rather than a flat line"
            )));
        }
        if self.series.iter().any(|s| s.state == SeriesState::Quiet) {
            notes.push(Note::caveat(
                "quiet is judged only for `state` subjects against their declared ttl_s; \
                 telemetry declares no period and is never called quiet",
            ));
        }
        notes.push(
            Note::caveat(
                "last seen is this observer's wall clock at arrival, never the producer's",
            )
            .cite("RFC 09 §5.1 O7"),
        );
        notes.push(Note::rendering(
            "the Prometheus text is a pure function of this document; `--once --prom` \
             prints it, and the listener serves it on every scrape",
        ));
        notes
    }

    fn bounds(&self) -> Vec<BoundCost> {
        let o = &self.observer;
        let mut costs = vec![
            BoundCost::new(
                BoundKind::Missed,
                o.dropped,
                "sample(s) dropped while behind — every value is a lower bound while this moves",
            ),
            BoundCost::new(
                BoundKind::Retired,
                o.evicted_keys,
                "key(s) retired at the stats-table bound; their series read `evicted`",
            ),
            BoundCost::new(
                BoundKind::Retired,
                o.evicted_bytes,
                "retained sample(s) dropped at the byte budget",
            ),
            BoundCost::new(
                BoundKind::Retired,
                o.expired,
                "retained sample(s) aged out of the retention window",
            ),
            BoundCost::new(
                BoundKind::Unwatched,
                o.unwatched,
                "key(s) retired because their watch was released",
            ),
            BoundCost::new(
                BoundKind::Coalesced,
                o.coalesced,
                "sample(s) coalesced between scrapes — only the newest value per series is exposed",
            ),
        ];
        // One cost per suppression reason, never a sum across them: each is
        // a different fact about what the surface does not carry.
        for (reason, what) in [
            (
                "cardinality",
                "sample(s) refused a series past the declared `cardinality` budget",
            ),
            (
                "max_series",
                "sample(s) refused a series at the --max-series bound",
            ),
            (
                "fields",
                "field(s) refused a series past the per-subject field cap",
            ),
            (
                "undecodable",
                "sample(s) with no structural document, so no value to export",
            ),
            (
                "unparsed",
                "sample(s) on keys that are not v1 data keys, so no contract to derive from",
            ),
        ] {
            costs.push(BoundCost::new(
                BoundKind::Refused,
                self.suppressed.get(reason).copied().unwrap_or(0),
                what,
            ));
        }
        costs
    }

    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: self.scopes.clone(),
            window_s: Some(self.taken_at_unix_s.saturating_sub(self.started_at_unix_s) as f64),
        })
    }
}
