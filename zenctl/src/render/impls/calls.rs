//! v1's `@rpc` replies — `config`'s read-back — and one answer rendering.
//! (`service call` and its `--trace` used it too, until FJ5 replaced them
//! with zk2's `call`, #612; and `check probe`, until FJ8b re-cut it as a
//! zk2 consumer.)
//!
//! `output::call` took the answer rendering as a closure, and both call sites
//! wrote their own. They drifted: `service call`'s appended the reply
//! attachment — a wire fact, shown where the reply is (#126) — and `check
//! probe`'s did not, so the same reply displayed differently depending on
//! which verb you reached for (#237).
//!
//! The closure was never a design point. It is a pure function of a
//! `CallAnswer`, so it is one here, and `check probe`'s report composed `CallReport`'s
//! rendering rather than re-entering the renderer with a hardcoded
//! `Format::Table` — which is what `table(&self, t: &mut Table)` taking a sink
//! is for.

use zenkey_fleet::report::{CallAnswer, CallOutcome, CallReport, PageSignal};

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
    // null-when-unknown (#117, #126). This clause is the one `check probe`
    // lost.
    if let (Some(att), Some(n)) = (&a.attachment, a.attachment_bytes) {
        out.push_str(&format!("\n  attachment ({n} B): {att}"));
    }
    // A bounded reply that stopped early says so on the line (#424): the
    // caller MUST NOT read a short page as the end, and a `partial: true`
    // buried in a pretty-printed object is exactly how it would.
    if let Some(p) = a.page_signal().filter(|p| p.partial) {
        out.push_str(&format!("\n  {}", stopped_early(&p)));
    }
    out
}

/// The RFC 05 §3.2 fields of a partial page, one line, the optional ones
/// only when the wire carried them.
fn stopped_early(p: &PageSignal) -> String {
    let mut line = format!(
        "stopped early (partial=true, next_cursor={}",
        p.next_cursor.as_deref().unwrap_or("null")
    );
    if let Some(n) = p.scanned {
        line.push_str(&format!(", scanned={n}"));
    }
    if let Some(c) = &p.covers_from {
        line.push_str(&format!(", covers_from={c}"));
    }
    line.push_str(") — RFC 05 §3.2");
    line
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
        let mut notes = match self.answers.len() {
            // R5: the note used to say "the timeout" without stating it —
            // now it names the wait the report itself carries.
            0 => vec![Note::silence(format!(
                "no replies to {} within {}s. The origin may be down, the procedure \
                 unregistered, or the timeout too short — `zenctl doctor` says \
                 who is up",
                self.key, self.timeout_s
            ))],
            n => vec![Note::summary(format!(
                "{n} repl{}",
                if n == 1 { "y" } else { "ies" }
            ))],
        };
        // `partial: true` with `next_cursor: null` is the contract violation
        // RFC 05 §3.2 says an observer MAY report. A caveat, not an exit
        // code: `call` is an act, and the reply *did* arrive (#424).
        notes.extend(
            self.answers
                .iter()
                .filter(|a| a.page_signal().is_some_and(|p| p.is_contract_violation()))
                .map(|a| {
                    Note::caveat(format!(
                        "{}: partial=true with next_cursor=null — the reply says it \
                         stopped early and offers no way to continue (a contract \
                         violation)",
                        a.origin
                    ))
                    .cite("RFC 05 §3.2")
                }),
        );
        notes
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
