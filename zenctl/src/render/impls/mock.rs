//! The mock owner (#612, FJ8a): `gen`'s plan before anything is brought up,
//! and its report after. `serve` is a stream — its `call` and `summary`
//! rows are written as they happen, by the verb.
//!
//! Two things the rendering keeps in view:
//!
//! * **the plan is a dry run made visible** — the replay precedent: a mock
//!   owner publishes, so every key, its type, its `Encoding`, its QoS and
//!   its rate are shown before a byte moves;
//! * **the marker is where zk2 can carry it** — in the descriptor's `meta`,
//!   not on every sample: an attachment is the contract's type (spec §2.3),
//!   and one writer per key (P3) makes the instance's marker a statement
//!   about every sample on its keys.

use zenkey_fleet::report::{GenPlan, GenReport, MemberSource, QosView};

use super::zk2::short_fp;
use crate::render::{Cell, Grid, Note, Render, Row, Table, envelope_without};

/// The spec's spelling of a QoS enum: its serde name.
fn spelled<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// A resource's QoS as `priority/congestion/reliability[+express]`, the
/// axes token's order.
fn qos_text(q: &QosView) -> String {
    format!(
        "{}/{}/{}{}",
        spelled(&q.priority),
        spelled(&q.congestion),
        spelled(&q.reliability),
        if q.express { "+express" } else { "" }
    )
}

impl Render for GenPlan {
    const FAMILY: &'static str = "gen-plan";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = envelope_without(self, &["entries"]);
        e.insert("entries".into(), self.entries.len().into());
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for e in &self.entries {
            out(Row::of("entry", e));
        }
    }

    fn table(&self, t: &mut Table) {
        t.line(format!(
            "plan: a mock owner at {} implementing {}, for {}s (seed {})",
            self.address,
            self.interfaces
                .iter()
                .map(|i| format!("{}@{}", i.iface, short_fp(&i.fingerprint)))
                .collect::<Vec<_>>()
                .join(", "),
            self.duration_s,
            self.seed
        ));
        let mut g = Grid::unheaded(1);
        for e in &self.entries {
            let how = match (&e.qos, e.rate_hz) {
                (Some(q), Some(hz)) => format!(
                    "{hz:.2} Hz, qos {}{}",
                    qos_text(q),
                    e.events_cap
                        .map(|c| format!(", at most {c} occurrence(s)"))
                        .unwrap_or_default()
                ),
                (Some(_), None) => "publishes nothing".to_owned(),
                (None, _) => "answers calls".to_owned(),
            };
            g.row([Cell::text(format!(
                "  {} [{}, {}] {how}{}",
                e.key,
                e.declared,
                e.encoding,
                if e.member_token { ", member token" } else { "" }
            ))]);
            if let Some(note) = &e.note {
                g.row([Cell::text(format!("    ↳ {note}"))]);
            }
        }
        t.grid(g);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = vec![
            Note::coverage(format!(
                "plan: {} entr(y|ies) at {}, every sample through the runtime's writers — the \
                 contract's QoS and Encoding, the owner's stamp on state, a fresh ULID per event",
                self.entries.len(),
                self.address
            ))
            .cite("spec §2.4, §4.2 S1, §2.6"),
            Note::coverage(format!(
                "the descriptor's meta carries the synthetic marker {}: one writer per key makes \
                 it a statement about every sample on these keys",
                self.marker
            ))
            .cite("spec §3.3, §6"),
        ];
        if self
            .entries
            .iter()
            .any(|e| e.members == MemberSource::Default)
        {
            notes.push(Note::coverage(
                "synthetic members stand in where --member named none: <param>-1, <param>-2",
            ));
        }
        notes
    }
}

impl Render for GenReport {
    const FAMILY: &'static str = "gen";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        crate::render::envelope_of(self)
    }

    fn rows(&self, _out: &mut dyn FnMut(Row)) {}

    fn table(&self, t: &mut Table) {
        t.line(format!(
            "{} as instance {}: sent {} sample(s) over {:.1}s across {} entr(y|ies), answered {} \
             call(s); {} not sent",
            self.address,
            self.instance,
            self.sent,
            self.duration_s,
            self.entries,
            self.calls,
            self.failed
        ));
        let mut g = Grid::unheaded(2);
        for e in &self.first_errors {
            g.row([Cell::text("  ✗"), Cell::text(e)]);
        }
        t.grid(g);
    }

    fn notes(&self) -> Vec<Note> {
        if self.failed == 0 {
            return Vec::new();
        }
        vec![Note::coverage(format!(
            "{} sample(s) not sent — a value the synthesizer could not make valid, or a put the \
             runtime refused — counted, never skipped, and never sent unchecked",
            self.failed
        ))]
    }
}
