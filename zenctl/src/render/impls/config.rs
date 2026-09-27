//! `config get` and a hot `config set`: the read-back document (RFC 05 §5.1)
//! drawn as what it is — a schema with values beside it — rather than as
//! the JSON it travelled as.
//!
//! A reply that is not a read-back (a reach `set`'s `{token, apply_at}`, a
//! dry run's report, an error envelope, or a producer that answers with
//! something else entirely) is kept and drawn the way `service call` draws
//! it, so nothing an origin said is dropped for not fitting the document.

use serde::Serialize;
use zenkey::config::{ConfigView, ParamKind, ParamView};
use zenkey_fleet::report::{CallAnswer, CallOutcome};

use super::calls::answer_text;
use crate::render::{Cell, Grid, Note, ObservedScope, Render, Row, Table};

/// One origin's read-back.
#[derive(Debug, Clone, Serialize)]
pub struct ConfigDocument {
    pub origin: String,
    #[serde(flatten)]
    pub view: ConfigView,
}

/// What a `config` verb got back: the documents, and the replies that were
/// not one.
#[derive(Debug, Clone, Serialize)]
pub struct ConfigReport {
    /// The key asked, as the wire spells it.
    pub key: String,
    /// Seconds the GET waited — the coverage claim's other half (R5).
    pub timeout_s: f64,
    pub documents: Vec<ConfigDocument>,
    /// Replies that are not a read-back, drawn as replies.
    pub other: Vec<CallAnswer>,
}

/// One parameter, flattened for the row stream: a script reads
/// `select(.row == "parameter")` and has the origin, the group and its
/// class on every line.
#[derive(Serialize)]
struct ParamRow<'a> {
    origin: &'a str,
    resource: &'a str,
    revision: u64,
    group: &'a str,
    class: &'static str,
    #[serde(flatten)]
    parameter: &'a ParamView,
}

impl Render for ConfigReport {
    const FAMILY: &'static str = "config";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = serde_json::Map::new();
        e.insert("key".into(), self.key.clone().into());
        e.insert("timeout_s".into(), self.timeout_s.into());
        e.insert("documents".into(), self.documents.len().into());
        e.insert("other".into(), self.other.len().into());
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for d in &self.documents {
            for g in &d.view.groups {
                for p in &g.parameters {
                    out(Row::of(
                        "parameter",
                        &ParamRow {
                            origin: &d.origin,
                            resource: &d.view.resource,
                            revision: d.view.revision,
                            group: &g.name,
                            class: g.class.token(),
                            parameter: p,
                        },
                    ));
                }
            }
            if let Some(p) = &d.view.pending {
                let mut row = serde_json::to_value(p).expect("a pending change serializes");
                if let serde_json::Value::Object(m) = &mut row {
                    m.insert("origin".into(), d.origin.clone().into());
                    m.insert("resource".into(), d.view.resource.clone().into());
                }
                out(Row::tagged("pending", row));
            }
        }
        for a in &self.other {
            out(Row::of("answer", a));
        }
    }

    fn table(&self, t: &mut Table) {
        for d in &self.documents {
            let mut head = format!(
                "{}  {}  revision {}",
                d.origin, d.view.resource, d.view.revision
            );
            if let Some(p) = &d.view.pending {
                head.push_str(&format!(
                    "  pending {} on {} until {}",
                    p.token,
                    p.groups.join(", "),
                    p.deadline.as_deref().unwrap_or("(no deadline stated)")
                ));
            }
            t.line(head);
            let mut g = Grid::new(["parameter", "value", "source", "kind", "description"]);
            for group in &d.view.groups {
                g.group(format!(
                    "{}  ({}) {}",
                    group.name,
                    group.class.token(),
                    group.description
                ));
                for p in &group.parameters {
                    // A sensitive value is absent by rule; any other absence
                    // is the producer having none to show — unknown, drawn
                    // as such, never a guessed default (O4).
                    let value = if p.spec.sensitive {
                        Some("(write-only)".to_string())
                    } else {
                        p.value.as_ref().map(|v| match &p.startup {
                            Some(startup) => format!("{v} (startup {startup})"),
                            None => v.to_string(),
                        })
                    };
                    g.row([
                        Cell::text(p.spec.name.clone()),
                        Cell::asked(value),
                        Cell::asked(p.source.map(|s| s.token().to_string())),
                        Cell::text(kind_text(&p.spec.kind)),
                        Cell::text(p.spec.description.clone()),
                    ]);
                }
            }
            t.grid(g);
        }
        if !self.other.is_empty() {
            let mut g = Grid::unheaded(1);
            for a in &self.other {
                match &a.outcome {
                    CallOutcome::Ok { .. } => {
                        g.row([Cell::text(format!("{}:", a.origin))]);
                        g.detail(answer_text(a).lines().map(str::to_string));
                    }
                    CallOutcome::Err(e) => {
                        g.row([Cell::text(format!(
                            "{}: ✗ {} — {}",
                            a.origin, e.name, e.message
                        ))]);
                    }
                }
            }
            t.grid(g);
        }
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if self.documents.is_empty() && self.other.is_empty() {
            notes.push(
                Note::silence(format!(
                    "no replies to {} within {}s. Nobody serves it, whoever does was down, \
                     or the timeout was short — and the three are different",
                    self.key, self.timeout_s
                ))
                .cite("RFC 05 §3.1"),
            );
        }
        if !self.other.is_empty() {
            notes.push(
                Note::caveat(format!(
                    "{} repl(y|ies) not shaped as a read-back document, shown as sent",
                    self.other.len()
                ))
                .cite("RFC 05 §5.1"),
            );
        }
        notes
    }

    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: vec![self.key.clone()],
            window_s: Some(self.timeout_s),
        })
    }
}

/// The kind as a person reads it: `bool`, `integer 0..=30 dBm`, `text`.
fn kind_text(kind: &ParamKind) -> String {
    match kind {
        ParamKind::Bool => "bool".to_string(),
        ParamKind::Integer { min, max, unit } => {
            let mut s = "integer".to_string();
            match (min, max) {
                (Some(lo), Some(hi)) => s.push_str(&format!(" {lo}..={hi}")),
                (Some(lo), None) => s.push_str(&format!(" {lo}..")),
                (None, Some(hi)) => s.push_str(&format!(" ..={hi}")),
                (None, None) => {}
            }
            if let Some(u) = unit {
                s.push(' ');
                s.push_str(u);
            }
            s
        }
        ParamKind::Text => "text".to_string(),
        // `#[non_exhaustive]`: a fourth kind arrives with the producer that
        // needs it, and this build draws it as what it does not know.
        _ => "(unknown kind)".to_string(),
    }
}
