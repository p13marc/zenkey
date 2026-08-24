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

use zenkey_fleet::report::{CallAnswer, CallOutcome, CallReport, ProbeReport};

use crate::render::{Cell, Grid, Note, ObservedScope, Render, Row, Table};

/// One answer, as a person reads it: the value if it is JSON-shaped, the raw
/// text otherwise, and the reply's attachment when the wire carried one.
pub fn answer_text(a: &CallAnswer) -> String {
    let mut out = match &a.outcome {
        CallOutcome::Ok {
            value: Some(v),
            text: _,
        } => serde_json::to_string_pretty(v).unwrap_or_default(),
        CallOutcome::Ok {
            value: None,
            text: Some(t),
        } => t.clone(),
        CallOutcome::Ok {
            value: None,
            text: None,
        } => String::new(),
        CallOutcome::Err(e) => format!("✗ {} — {}", e.name, e.message),
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
        // R5: the wait is part of the coverage claim (GetReport is the
        // model) — the silence note names it, so the document must state it.
        e.insert("timeout_s".into(), self.timeout_s.into());
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
            // The outcome is an enum, so this match is total: the old
            // `{ok, error: Option}` shape could spell an error-less failure,
            // and this renderer silently dropped exactly that row.
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

    fn notes(&self) -> Vec<Note> {
        match self.answers.len() {
            // R5: the note used to say "the timeout" without stating it —
            // now it names the wait the report itself carries.
            0 => vec![Note::silence(format!(
                "no replies to {} within {}s. The origin may be down, the procedure \
                 unregistered, or the timeout too short — `zenctl node list` says \
                 who is up",
                self.key, self.timeout_s
            ))],
            n => vec![Note::summary(format!(
                "{n} repl{}",
                if n == 1 { "y" } else { "ies" }
            ))],
        }
    }

    /// The GET's coverage claim: the one key asked, and the wait the report
    /// carries (R5 / P1's `timeout_s`).
    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: vec![self.key.clone()],
            window_s: Some(self.timeout_s),
        })
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
        // Inherited from the delegated call (R5), like the silence note.
        e.insert("timeout_s".into(), self.call.timeout_s.into());
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

    /// Delegated, like the rendering: the probe's observation *is* the call.
    fn scope(&self) -> Option<ObservedScope> {
        self.call.scope()
    }
}
