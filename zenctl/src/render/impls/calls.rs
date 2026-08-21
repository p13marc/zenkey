//! `service call` and `probe`: one answer rendering, used twice.
//!
//! `output::call` took the answer rendering as a closure, and both call sites
//! wrote their own. They drifted: `service call`'s appended the reply
//! attachment — a wire fact, shown where the reply is (#126) — and `probe`'s
//! did not, so the same reply displayed differently depending on which verb
//! you reached for (#237).
//!
//! The closure was never a design point. It is a pure function of a
//! `CallAnswer`, so it is one here, and `ProbeReport` composes `CallReport`'s
//! rendering rather than re-entering the renderer with a hardcoded
//! `Format::Table` — which is what `table(&self, t: &mut Table)` taking a sink
//! is for.

use zenkey_fleet::report::{CallAnswer, CallReport, ProbeReport};

use crate::render::{Cell, Grid, Note, Render, Row, Table};

/// One answer, as a person reads it: the value if it is JSON-shaped, the raw
/// text otherwise, and the reply's attachment when the wire carried one.
pub fn answer_text(a: &CallAnswer) -> String {
    let mut out = match (&a.value, &a.text) {
        (Some(v), _) => serde_json::to_string_pretty(v).unwrap_or_default(),
        (None, Some(t)) => t.clone(),
        _ => String::new(),
    };
    // Present only when the wire carried one — absent, never
    // null-when-unknown (#117, #126). This clause is the one `probe` lost.
    if let (Some(att), Some(n)) = (&a.attachment, a.attachment_bytes) {
        out.push_str(&format!("\n  attachment ({n} B): {att}"));
    }
    out
}

impl Render for CallReport {
    const FAMILY: &'static str = "call";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = serde_json::Map::new();
        e.insert("key".into(), self.key.clone().into());
        e.insert("answers".into(), self.answers.len().into());
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for a in &self.answers {
            out(Row::of("answer", a));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut g = Grid::unheaded(1);
        for a in &self.answers {
            if a.ok {
                g.row([Cell::text(format!("{}:", a.origin))]);
                g.detail(answer_text(a).lines().map(str::to_string));
            } else if let Some(e) = &a.error {
                g.row([Cell::text(format!(
                    "{}: ✗ {} — {}",
                    a.origin, e.name, e.message
                ))]);
            }
        }
        t.grid(g);
    }

    fn notes(&self) -> Vec<Note> {
        match self.answers.len() {
            0 => vec![Note::silence(format!(
                "no replies to {}. The origin may be down, the procedure \
                 unregistered, or the timeout too short — `zenctl node list` says \
                 who is up",
                self.key
            ))],
            n => vec![Note::summary(format!(
                "{n} repl{}",
                if n == 1 { "y" } else { "ies" }
            ))],
        }
    }
}

impl Render for ProbeReport {
    const FAMILY: &'static str = "probe";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = serde_json::Map::new();
        e.insert("input".into(), self.input.clone().into());
        e.insert("origin".into(), self.origin.clone().into());
        e.insert("via".into(), self.via.clone().into());
        e.insert("key".into(), self.call.key.clone().into());
        e
    }

    /// Delegated, and that is the fix for a second `probe` defect: the old
    /// ndjson emitted the whole report as one compact line, so the answers
    /// were buried inside a nested object rather than being rows a consumer
    /// could iterate.
    fn rows(&self, out: &mut dyn FnMut(Row)) {
        self.call.rows(out);
    }

    fn table(&self, t: &mut Table) {
        t.line(format!(
            "probe {} → origin {} (via {})",
            self.input, self.origin, self.via
        ));
        self.call.table(t);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = self.call.notes();
        if self.call.answers.is_empty() {
            notes.push(
                Note::coverage(
                    "the origin resolved but did not answer — the probe reached a name, \
                     not a responder; absence of replies and absence of callers must \
                     not look alike",
                )
                .cite("RFC 09 §6"),
            );
        }
        notes
    }
}
