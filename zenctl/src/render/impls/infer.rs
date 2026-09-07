//! `registry infer` (#225): the draft as a document — one row per inferred
//! subject, the run's coverage and bounds in the envelope, and the caveats
//! a reviewer must read as notes, so they reach json and ndjson too.

use zenkey_fleet::report::InferReport;

use crate::render::{
    BoundCost, BoundKind, Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_without,
};

impl Render for InferReport {
    const FAMILY: &'static str = "registry-infer";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        // Everything except the producers — the coverage and bound claims
        // must survive a truncated pipe. Rows carry the subjects and types.
        envelope_without(self, &["producers"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        // Each row names its producer: a line cut out of the stream must
        // still say which file it lands in.
        fn with_producer<T: serde::Serialize>(
            kind: &'static str,
            value: &T,
            producer: &str,
        ) -> Row {
            let mut v = serde_json::to_value(value).expect("a report row serializes");
            if let serde_json::Value::Object(m) = &mut v {
                m.insert("producer".into(), producer.into());
            }
            Row::tagged(kind, v)
        }
        for p in &self.producers {
            for s in &p.subjects {
                out(with_producer("subject", s, &p.name));
            }
            for t in &p.types {
                out(with_producer("type", t, &p.name));
            }
        }
    }

    fn table(&self, t: &mut Table) {
        for p in &self.producers {
            t.line(format!(
                "{}.toml — {} subject(s), {} type(s)",
                p.name,
                p.subjects.len(),
                p.types.len()
            ));
            let mut grid = Grid::unheaded(4).right(3);
            for s in &p.subjects {
                let mut facts = vec![s.type_name.clone()];
                if let Some(u) = &s.unit {
                    facts.push(format!("unit {u}"));
                }
                if let Some(c) = s.cardinality {
                    facts.push(format!("cardinality {c}"));
                }
                if let Some(ttl) = s.ttl_s {
                    facts.push(format!("ttl_s {ttl}"));
                }
                if let Some(r) = &s.rate {
                    facts.push(format!("rate {r}"));
                }
                if let Some(e) = &s.encoding {
                    facts.push(e.clone());
                }
                grid.row([
                    Cell::text(s.class.clone()),
                    Cell::text(s.path.clone()),
                    Cell::text(facts.join(" · ")),
                    Cell::text(format!(
                        "{} key(s) / {} origin(s) / {} sample(s)",
                        s.keys, s.origins, s.samples
                    )),
                ]);
            }
            t.grid(grid);
            t.blank();
        }
    }

    fn bounds(&self) -> Vec<BoundCost> {
        vec![
            BoundCost::new(
                BoundKind::Missed,
                self.dropped,
                "sample(s) dropped while behind — every count is of what was seen",
            ),
            BoundCost::new(
                BoundKind::Refused,
                self.keys_refused,
                "key(s) refused at the key bound — the draft covers the retained set",
            ),
            BoundCost::new(
                BoundKind::Refused,
                self.paths_refused,
                "path observation(s) refused at the path-table bound — inferred schemas admit \
                 additional properties",
            ),
        ]
    }

    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: vec![self.source.clone()],
            window_s: self.window_s,
        })
    }

    fn notes(&self) -> Vec<Note> {
        let subjects: usize = self.producers.iter().map(|p| p.subjects.len()).sum();
        let mut notes = vec![Note::summary(format!(
            "drafted {subjects} subject(s) across {} producer(s) from {} sample(s) on {} key(s), \
             {} origin(s), over {:.1} s — every field a guess, marked draft = true (RFC 08 §6.1)",
            self.producers.len(),
            self.samples,
            self.keys_seen,
            self.origins,
            self.span_s
        ))];
        if self.unparsed_keys > 0 || self.off_plane_keys > 0 {
            notes.push(
                Note::caveat(format!(
                    "{} key(s) did not parse as v1 and {} sat on a verbatim plane — nothing to \
                     draft for them",
                    self.unparsed_keys, self.off_plane_keys
                ))
                .cite("RFC 03 §1.4"),
            );
        }
        if self.undocumented > 0 {
            notes.push(
                Note::caveat(format!(
                    "{} sample(s) carried no structural document — their fields are unobservable, \
                     not absent",
                    self.undocumented
                ))
                .cite("RFC 09 §5.1 O4"),
            );
        }
        if !self.hinted_by_registry {
            notes.push(
                Note::caveat("no registry hinted the inference: {var}s are named by position")
                    .cite("RFC 08 §6.1"),
            );
        }
        notes.extend(
            self.caveats
                .iter()
                .filter(|c| !c.starts_with("no registry hinted"))
                .map(|c| Note::caveat(c.clone()).cite("RFC 08 §6.1")),
        );
        notes
    }
}
