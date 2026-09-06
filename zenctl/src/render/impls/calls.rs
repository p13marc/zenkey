//! `service call`, `check probe` and `service call --trace`: one answer
//! rendering, used three times.
//!
//! `output::call` took the answer rendering as a closure, and both call sites
//! wrote their own. They drifted: `service call`'s appended the reply
//! attachment — a wire fact, shown where the reply is (#126) — and `check
//! probe`'s did not, so the same reply displayed differently depending on
//! which verb you reached for (#237).
//!
//! The closure was never a design point. It is a pure function of a
//! `CallAnswer`, so it is one here, and `ProbeReport` composes `CallReport`'s
//! rendering rather than re-entering the renderer with a hardcoded
//! `Format::Table` — which is what `table(&self, t: &mut Table)` taking a sink
//! is for.

use zenkey_fleet::report::{
    CallAnswer, CallOutcome, CallReport, HlcReference, PageSignal, ProbeReport, TraceRelation,
    TraceReport, TraceRow,
};

use crate::render::{
    BoundCost, BoundKind, Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_without,
};

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
                 unregistered, or the timeout too short — `zenctl node list` says \
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

    /// Delegated, and that is the fix for a second `check probe` defect: the old
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

// ── `service call --trace` (#215) ─────────────────────────────────────────

/// The relation, as a person reads it in the table.
fn relation_text(r: TraceRelation) -> &'static str {
    match r {
        TraceRelation::DeclaredChain => "declared-chain",
        TraceRelation::SameOriginUndeclared => "same-origin, not declared",
        TraceRelation::SameOriginRegistryNotLoaded => "same-origin, registry not loaded",
    }
}

/// `+12.345ms` — an arrival offset from `t0`.
fn arrival(ms: f64) -> String {
    format!("+{ms:.3}ms")
}

/// One lane's rows into the grid under its heading, a break line before any
/// row that carries one. The lanes share the shape, so they share the code.
fn lane_rows(g: &mut Grid, rows: &[TraceRow]) {
    for r in rows {
        if let Some(n) = r.break_before {
            g.row([
                Cell::text(""),
                Cell::text(""),
                Cell::text(format!("⋯ {n} sample(s) dropped while behind")),
                Cell::text(""),
            ]);
        }
        let hlc = match (&r.hlc_delta_ms, &r.stamped_by) {
            (Some(d), Some(by)) => Cell::text(format!("{d:+}ms ({by})")),
            // Stamped, but there is no reply HLC to measure against: the
            // question could not be asked of this row — not asked, never
            // an empty cell that would read as "unstamped".
            (None, Some(_)) => Cell::asked(None::<String>),
            // Unstamped: asked, and there is nothing — an empty cell.
            _ => Cell::text(""),
        };
        g.row([
            Cell::text(arrival(r.arrival_delta_ms)),
            hlc,
            Cell::text(&r.key),
            Cell::text(relation_text(r.relation)),
        ]);
    }
}

/// A trace row as an ndjson line, tagged `effect` and carrying its lane —
/// the lane is a fact about where the row sits in the document, and a line
/// cut out of the stream must still say which one.
fn effect_row(lane: &'static str, r: &TraceRow) -> Row {
    let mut v = serde_json::to_value(r).expect("a trace row serializes");
    if let serde_json::Value::Object(o) = &mut v {
        o.insert("lane".into(), lane.into());
    }
    Row::tagged("effect", v)
}

impl Render for TraceReport {
    const FAMILY: &'static str = "trace";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        // The call's own envelope facts ride first (a trace *is* a call),
        // then the trace's header — the lanes become rows.
        let mut e = self.call.envelope();
        for (k, v) in envelope_without(self, &["call", "attributed", "same_origin", "concurrent"]) {
            e.insert(k, v);
        }
        e.insert("attributed".into(), self.attributed.len().into());
        e.insert("same_origin".into(), self.same_origin.len().into());
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        self.call.rows(out);
        for r in &self.attributed {
            out(effect_row("attributed", r));
        }
        for r in &self.same_origin {
            out(effect_row("same_origin", r));
        }
        out(Row::of("concurrent", &self.concurrent));
    }

    fn table(&self, t: &mut Table) {
        // The reply block, exactly as `service call` draws it.
        self.call.table(t);
        // An empty lane draws no heading: the sentence that explains it is
        // a note, which is what reaches json and ndjson too.
        let mut g = Grid::unheaded(4).right(0).right(1);
        if !self.attributed.is_empty() {
            g.group(format!(
                "observed after the call · declared chain ({}) · Δarrival · ΔHLC (stamper) · key",
                self.idiom
            ));
            lane_rows(&mut g, &self.attributed);
        }
        if !self.same_origin.is_empty() {
            g.group(if self.registry_loaded {
                "observed after the call · same origin, not declared · Δarrival · ΔHLC (stamper) · key"
            } else {
                "observed after the call · same origin — registry not loaded, chain \
                 unjudgeable · Δarrival · ΔHLC (stamper) · key"
            });
            lane_rows(&mut g, &self.same_origin);
        }
        g.group("concurrent, not attributed");
        let c = &self.concurrent;
        let mut line = format!(
            "{} sample(s) on {} key(s) from other origins during the window",
            c.samples, c.keys
        );
        if !c.examples.is_empty() {
            line.push_str(&format!(" — e.g. {}", c.examples.join(", ")));
        }
        g.detail([line]);
        t.grid(g);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = self.call.notes();
        notes.push(Note::summary(format!(
            "{} in the declared chain, {} same-origin, {} concurrent within {}s of t0 (the \
             call returned at +{:.1}ms)",
            self.attributed.len(),
            self.same_origin.len(),
            self.concurrent.samples,
            self.window_s,
            self.call_returned_ms
        )));
        if self.attributed.is_empty() {
            notes.push(
                Note::coverage(
                    "nothing in the declared chain was observed within the window — not \
                     evidence the procedure had no effect: the window covers the data \
                     classes only, and a producer may publish after it or not at all",
                )
                .cite("RFC 09 §5.1 O4"),
            );
        }
        notes.push(Note::caveat(format!("chain rule: {}", self.chain_rule)).cite("RFC 05 §3"));
        if !self.registry_loaded {
            notes.push(
                Note::coverage(
                    "registry not loaded — chain unjudgeable: every same-origin sample is \
                     tagged so, and none can be `declared-chain`; load one with --registry \
                     or let introspect serve it",
                )
                .cite("RFC 09 §5.1 O4"),
            );
        }
        match (self.hlc_reference, &self.reply_hlc) {
            (HlcReference::Reply, Some(hlc)) => notes.push(
                Note::caveat(format!(
                    "ΔHLC is measured against the reply's HLC ({hlc}) — the stamping node's \
                     clock, which is not necessarily the responder's and is never this \
                     caller's (it mints none); a different stamper on a row is a different \
                     clock"
                ))
                .cite("RFC 09 §5.1 O7"),
            ),
            _ => notes.push(
                Note::coverage(
                    "the reply carried no HLC, so no ΔHLC is computed — the column is \
                     absent, never defaulted to arrival; Δarrival is this observer's clock",
                )
                .cite("RFC 09 §5.1 O4"),
            ),
        }
        notes.push(Note::coverage(self.excluded).cite("RFC 03 §4 D2"));
        notes.push(Note::rendering(
            "observed after the call, in arrival order — never caused: no edge is drawn \
             between the reply and any sample, and a relation is a statement about names \
             in the registry",
        ));
        notes
    }

    fn bounds(&self) -> Vec<BoundCost> {
        vec![
            BoundCost::new(
                BoundKind::Missed,
                self.dropped,
                "sample(s) dropped while behind on the origin's window — each is a break \
                 before the next row of every lane",
            ),
            BoundCost::new(
                BoundKind::Missed,
                self.concurrent.dropped,
                "sample(s) dropped on the fleet-wide window — the concurrent count is a \
                 lower bound",
            ),
            BoundCost::new(
                BoundKind::Retired,
                self.keys_evicted,
                "key(s) retired at the origin watch's stats-table bound during the window",
            ),
        ]
    }

    /// Both watches, over the window: the origin's subtree and the fleet's.
    /// The GET's own ask is inside `call`, and the window subsumes its wait.
    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: self.scopes.clone(),
            window_s: Some(self.window_s),
        })
    }
}
