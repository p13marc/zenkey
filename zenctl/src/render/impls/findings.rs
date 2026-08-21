//! The families whose rows are *findings*: doctor, registry diff, schema dump.
//!
//! What they share is the thing this seam is for — an empty result means
//! something, and it never means "nothing is wrong". `doctor` with no findings
//! has to say what it checked; a diff with no producers has to say it looked
//! at both sides; a producer that serves no `describe` has undescribed shapes,
//! which is not the same as having none.

use zenkey_fleet::report::{DoctorReport, DoctorSeverity, RegistryDiff, SchemaDump};

use crate::render::{Cell, Grid, Note, Render, Row, Table};

impl Render for DoctorReport {
    const FAMILY: &'static str = "doctor";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        // Everything except the findings themselves, so an empty findings list
        // stays legible: what was checked, not just what was found.
        let mut e = match serde_json::to_value(self).expect("a report serializes") {
            serde_json::Value::Object(m) => m,
            _ => unreachable!("a report is an object"),
        };
        e.remove("findings");
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for f in &self.findings {
            out(Row::of("finding", f));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut grid = Grid::unheaded(2);
        for s in &self.synced {
            grid.row([Cell::text("✓"), Cell::text(format!("{s}: in sync"))]);
        }
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
                Cell::text(mark),
                Cell::text(format!(
                    "{}: {} — {}{citation}",
                    f.check, f.subject, f.evidence
                )),
            ]);
        }
        t.grid(grid);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        // The listen phase's scope statement (#161): what was watched, for how
        // long, and what the bounded observer missed.
        if let Some(obs) = &self.observation {
            let mut text = format!(
                "listened {:.0}s over {} scope(s): {} sample(s) on {} key(s), {} dropped",
                obs.window_s,
                obs.scopes.len(),
                obs.samples,
                obs.keys_seen,
                obs.dropped
            );
            if obs.dropped > 0 {
                text.push_str(" — findings cover only what was seen");
            }
            if obs.synthetic_marked > 0 {
                text.push_str(&format!(
                    "; {} sample(s) carried the synthetic marker",
                    obs.synthetic_marked
                ));
            }
            notes.push(Note::bound(text));
        }
        notes.push(Note::coverage(format!(
            "{} introspect repl(y|ies) from {} live producer(s); {} producer(s) serve \
             describe, {} do not; {} router(s){}",
            self.introspect_answered,
            self.live_producers,
            self.describe_served,
            self.describe_missing,
            self.routers,
            self.router_version
                .as_deref()
                .map(|v| format!(" (version {v})"))
                .unwrap_or_default(),
        )));
        let errors = self.count(DoctorSeverity::Error);
        let warnings = self.count(DoctorSeverity::Warning);
        notes.push(if errors == 0 && warnings == 0 {
            Note::summary("no findings — the fleet agrees with this build.")
        } else {
            Note::summary(format!(
                "{} finding(s): {errors} error(s), {warnings} warning(s), {} info.",
                self.findings.len(),
                self.count(DoctorSeverity::Info)
            ))
        });
        notes
    }
}

impl Render for RegistryDiff {
    const FAMILY: &'static str = "registry-diff";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = serde_json::Map::new();
        e.insert("producers".into(), self.producers.len().into());
        e.insert("disagreeing".into(), self.disagreeing().into());
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for p in &self.producers {
            out(Row::of("producer", p));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut grid = Grid::unheaded(3).max(1, 16);
        for p in &self.producers {
            // A version one side has and the other does not is `—`: the
            // question was put to both, and one of them has no answer to give.
            let versions = match (&p.served_version, &p.local_version) {
                (Some(s), Some(l)) if s == l => format!("registry {s}"),
                (Some(s), Some(l)) => format!("served {s} · local {l}"),
                (Some(s), None) => format!("served {s} · local —"),
                (None, Some(l)) => format!("served — · local {l}"),
                (None, None) => String::new(),
            };
            if p.findings.is_empty() {
                grid.row([
                    Cell::text(" "),
                    Cell::text(&p.producer),
                    Cell::text(format!("{versions}   agree")),
                ]);
            } else {
                grid.row([
                    Cell::text("✗"),
                    Cell::text(&p.producer),
                    Cell::text(versions),
                ]);
                grid.detail(p.findings.iter().map(|f| format!("      {f}")));
            }
        }
        t.grid(grid);
    }

    fn notes(&self) -> Vec<Note> {
        if self.producers.is_empty() {
            return vec![Note::silence(
                "no producers on either side — the bus served no introspect slice and \
                 the registry directory declares none, which is two silences rather \
                 than one agreement",
            )];
        }
        // A disagreement is a finding, not a verdict: the count is reported
        // and the exit code is not touched.
        vec![
            Note::summary(format!(
                "{} producer(s), {} disagreeing (a disagreement is a finding).",
                self.producers.len(),
                self.disagreeing()
            ))
            .cite("RFC 08 §6"),
        ]
    }
}

impl Render for SchemaDump {
    const FAMILY: &'static str = "schema-dump";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = serde_json::Map::new();
        e.insert("producer".into(), self.producer.clone().into());
        e.insert("served".into(), self.served.into());
        if let Some(app) = &self.app {
            e.insert("app".into(), app.clone().into());
        }
        if !self.missing.is_empty() {
            e.insert("missing".into(), serde_json::json!(self.missing));
        }
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for ty in &self.types {
            out(Row::of("type", ty));
        }
    }

    fn table(&self, t: &mut Table) {
        if !self.served {
            return;
        }
        t.line(format!(
            "producer  {}{}",
            self.producer,
            self.app
                .as_deref()
                .map(|a| format!("   (app {a})"))
                .unwrap_or_default()
        ));
        if !self.types.is_empty() {
            t.blank()
                .line(format!("{} type(s):", self.types.len()))
                .blank();
            let mut grid = Grid::unheaded(3).max(0, 24);
            for ty in &self.types {
                grid.row([
                    Cell::text(format!("  {}", ty.type_name)),
                    Cell::text(&ty.kind),
                    Cell::text(&ty.hash),
                ]);
            }
            t.grid(grid);
        }
        for ty in &self.types {
            if let Some(doc) = &ty.document {
                t.blank().line(format!("{}:", ty.type_name));
                t.line(serde_json::to_string_pretty(doc).unwrap_or_default());
            }
        }
    }

    fn notes(&self) -> Vec<Note> {
        if !self.served {
            // §7 is a SHOULD. "Serves no describe" is a fact about the
            // producer, not a verdict about its payloads.
            return vec![
                Note::coverage(format!(
                    "{} serves no `describe` — its payload shapes are undescribed, \
                     which is not the same as having none",
                    self.producer
                ))
                .cite("RFC 08 §7"),
            ];
        }
        let mut notes = Vec::new();
        if self.types.is_empty() {
            notes.push(Note::coverage("the served set declares no matching types"));
        }
        if !self.missing.is_empty() {
            // §7's totality clause, checked where the user is already looking
            // rather than only in `doctor`.
            notes.push(
                Note::coverage(format!(
                    "⚠ the registry references {} type(s) this set does not cover: {} \
                     — the set MUST cover every referenced name",
                    self.missing.len(),
                    self.missing.join(", ")
                ))
                .cite("RFC 08 §7"),
            );
        }
        notes
    }
}
