//! Report types this crate owns.
//!
//! `zenkey_fleet::report` is the contract shared with zengui and pinned by
//! `report_contract.rs`. These are not that: the slice cache is *this tool's
//! own disk footprint*, no second frontend renders it, and putting it in the
//! shared surface would freeze a shape nobody else reads. The governing rule
//! catches the difference — a report belongs to the engine iff it is the
//! return value of a fleet function *and* a second frontend would plausibly
//! render it.

use serde::Serialize;

use crate::render::{Cell, Grid, Note, Render, Row, Table};

/// One producer's slice, as the completion cache holds it.
#[derive(Debug, Clone, Serialize)]
pub struct CachedSlice {
    pub producer: String,
    pub registry_version: String,
    pub subjects: usize,
    pub procedures: usize,
}

/// What `zenctl cache show` found on disk.
#[derive(Debug, Clone, Serialize)]
pub struct CacheReport {
    /// The directory itself — the point of the command is that a tool which
    /// leaves files on a user's disk can be asked where they are (#54).
    pub dir: String,
    pub slices: Vec<CachedSlice>,
}

impl Render for CacheReport {
    const FAMILY: &'static str = "cache";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = serde_json::Map::new();
        e.insert("dir".into(), self.dir.clone().into());
        e.insert("slices".into(), self.slices.len().into());
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for s in &self.slices {
            out(Row::of("slice", s));
        }
    }

    fn table(&self, t: &mut Table) {
        t.line(&self.dir);
        let mut g = Grid::unheaded(3).max(0, 16);
        for s in &self.slices {
            g.row([
                Cell::text(format!("  {}", s.producer)),
                Cell::text(format!("registry {}", s.registry_version)),
                Cell::text(format!(
                    "{} subject(s), {} procedure(s)",
                    s.subjects, s.procedures
                )),
            ]);
        }
        t.grid(g);
    }

    fn notes(&self) -> Vec<Note> {
        if self.slices.is_empty() {
            return vec![Note::coverage(
                "empty — completion falls back to the static command tree. Any command \
                 that loads slices fills it (or `zenctl cache refresh`)",
            )];
        }
        vec![Note::coverage(format!(
            "{} producer(s), from the last sighting — suggestions, not an inventory. \
             Nothing reads this except shell completion, and no command answers from it",
            self.slices.len()
        ))]
    }
}

/// A key-expression relation, answered (#198).
///
/// zenctl-local for the same reason as [`CacheReport`], and a sharper one: the
/// algebra is `zenoh-keyexpr`'s, not the fleet's. No second frontend renders a
/// key-relation verdict, and pinning `{op, a, b, answer}` into the shared
/// contract would freeze a shape nobody else reads.
#[derive(Debug, Clone, Serialize)]
pub struct KeyRelation {
    pub op: String,
    pub a: String,
    pub b: String,
    pub answer: bool,
    /// Why a "no" is the convention's doing rather than a typo — `**` never
    /// crosses an `@`-chunk (RFC 03 §4 D2), `*` never matches a verbatim
    /// service origin (D4).
    ///
    /// Absent when the answer needs no explanation, never null (O4): a "yes"
    /// has no note, and a "no" without a convention reason has none either.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Render for KeyRelation {
    const FAMILY: &'static str = "key-relation";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        match serde_json::to_value(self).expect("serializes") {
            serde_json::Value::Object(m) => m,
            _ => unreachable!("a report is an object"),
        }
    }

    fn rows(&self, _out: &mut dyn FnMut(Row)) {}

    fn table(&self, t: &mut Table) {
        let verb = match (self.op.as_str(), self.answer) {
            ("includes", true) => format!("includes every key {} names", self.b),
            ("includes", false) => format!("does not include all of {}", self.b),
            (_, true) => format!("and {} can name a common key", self.b),
            (_, false) => format!("and {} share no key", self.b),
        };
        t.line(format!(
            "{} — {} {verb}",
            if self.answer { "yes" } else { "no" },
            self.a
        ));
        // Printed here rather than returned from `notes()`: the explanation
        // is a *field of this report*, so surfacing it as a note too would
        // put the same sentence in the document twice.
        if let Some(n) = &self.note {
            t.line(n);
        }
    }
}

/// A key expression's canonical spelling.
#[derive(Debug, Clone, Serialize)]
pub struct KeyCanon {
    pub input: String,
    pub canon: String,
    pub changed: bool,
}

impl Render for KeyCanon {
    const FAMILY: &'static str = "key-canon";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        match serde_json::to_value(self).expect("serializes") {
            serde_json::Value::Object(m) => m,
            _ => unreachable!("a report is an object"),
        }
    }

    fn rows(&self, _out: &mut dyn FnMut(Row)) {}

    /// The canonical form **alone** when it changed, because this verb is a
    /// filter: `$(zenctl key canon "$x")` has to be the answer and nothing
    /// else. The "already canonical" sentence is safe because in that case
    /// the input *is* the answer.
    fn table(&self, t: &mut Table) {
        if self.changed {
            t.line(&self.canon);
        } else {
            t.line(format!("{} is already canonical", self.input));
        }
    }
}

/// One payload checked against one schema (#159).
///
/// zenctl-local: the *verdict vocabulary* is `zenkey`'s and shared, but this
/// document is a CLI invocation's answer — one payload, one schema, one exit
/// code — rather than an observation of a fleet. zengui validates inline and
/// renders the `Verdict` itself.
#[derive(Debug, Clone, Serialize)]
pub struct SchemaCheck {
    #[serde(rename = "type")]
    pub type_name: String,
    pub kind: String,
    /// `valid`, `invalid`, `undecodable`, or `not-validated: <reason>`.
    pub verdict: String,
    /// The failing constraints, or the decoder's notes. Absent when there is
    /// nothing to say, rather than an empty array meaning the same thing.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub detail: Vec<String>,
}

impl Render for SchemaCheck {
    const FAMILY: &'static str = "schema-check";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        match serde_json::to_value(self).expect("serializes") {
            serde_json::Value::Object(m) => m,
            _ => unreachable!("a report is an object"),
        }
    }

    fn rows(&self, _out: &mut dyn FnMut(Row)) {}

    fn table(&self, t: &mut Table) {
        t.line(format!(
            "{} ({}): {}",
            self.type_name, self.kind, self.verdict
        ));
        let mut g = Grid::unheaded(1);
        for line in &self.detail {
            g.row([Cell::text(format!("  {line}"))]);
        }
        t.grid(g);
    }
}

/// What `zenctl gen` is about to publish, before it publishes anything.
///
/// The plan is a *dry run made visible*, the same precedent `replay
/// --dry-run` set: synthetic traffic is still publishing, so the operator
/// sees the whole of it before a byte moves (RFC 09 §5.3).
pub struct GenPlan<'a> {
    pub origin: &'a str,
    pub duration_s: f64,
    pub entries: &'a [zenkey_fleet::generate::GenPlanEntry],
}

impl serde::Serialize for GenPlan<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct as _;
        let mut st = s.serialize_struct("GenPlan", 3)?;
        st.serialize_field("origin", self.origin)?;
        st.serialize_field("duration_s", &self.duration_s)?;
        st.serialize_field("entries", &self.entries.len())?;
        st.end()
    }
}

impl Render for GenPlan<'_> {
    const FAMILY: &'static str = "gen-plan";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        match serde_json::to_value(self).expect("serializes") {
            serde_json::Value::Object(m) => m,
            _ => unreachable!("a report is an object"),
        }
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for e in self.entries {
            out(Row::of("entry", e));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut g = Grid::unheaded(1);
        for e in self.entries {
            g.row([Cell::text(format!(
                "  {} [{}] {:.2} Hz, qos {} ({}), body {}{}",
                e.key,
                e.type_name,
                e.rate_hz,
                e.qos,
                e.qos_source,
                e.body_source,
                e.events_cap
                    .map(|c| format!(", capped at {c} event(s)"))
                    .unwrap_or_default(),
            ))]);
            // The fault this entry injects, stated before a byte moves — the
            // tool says exactly what it is about to do to the bus (#163).
            if let (Some(fault), Some(delta)) = (e.fault, &e.fault_delta) {
                g.row([Cell::text(format!("    ↳ FAULT[{}]: {delta}", fault.as_str()))]);
            }
        }
        t.grid(g);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = vec![
            Note::coverage(format!(
                "plan: {} subject(s) as {}, {}s",
                self.entries.len(),
                self.origin,
                self.duration_s
            )),
            // The marker is not decoration: someone else's `doctor
            // --listen-for` has to be able to tell this traffic from real.
            Note::coverage("every sample carries the marker {\"synthetic\":true}")
                .cite("RFC 09 §5.3"),
        ];
        // When faults are in play, name the kinds up front and re-state the
        // per-sample marker rule (a faulted sample additionally carries
        // fault=<kind>, RFC 09 §5.3).
        let mut kinds: Vec<&str> = self
            .entries
            .iter()
            .filter_map(|e| e.fault.map(|f| f.as_str()))
            .collect();
        kinds.sort_unstable();
        kinds.dedup();
        if !kinds.is_empty() {
            notes.push(
                Note::coverage(format!(
                    "injecting fault(s): {} — near-valid traffic for consumer-robustness \
                     testing; every faulted sample's marker carries fault=<kind>",
                    kinds.join(", ")
                ))
                .cite("RFC 09 §5.3"),
            );
        }
        notes
    }
}

impl Render for zenkey_fleet::generate::GenReport {
    const FAMILY: &'static str = "gen";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        match serde_json::to_value(self).expect("serializes") {
            serde_json::Value::Object(m) => m,
            _ => unreachable!("a report is an object"),
        }
    }

    fn rows(&self, _out: &mut dyn FnMut(Row)) {}

    fn table(&self, t: &mut Table) {
        t.line(format!(
            "sent {} sample(s) over {:.1}s across {} subject(s); {} refused by schema",
            self.sent, self.duration_s, self.entries, self.refused
        ));
        let mut g = Grid::unheaded(2);
        for e in &self.first_errors {
            g.row([Cell::text("  ✗"), Cell::text(e)]);
        }
        t.grid(g);
    }

    fn notes(&self) -> Vec<Note> {
        if self.refused == 0 {
            return Vec::new();
        }
        vec![Note::coverage(format!(
            "{} bod(y|ies) the schema refused to encode — counted, never silently \
             skipped: either the synthesizer or the schema is wrong, and the \
             operator hears about it either way",
            self.refused
        ))]
    }
}

/// One reply to a fan-in GET.
///
/// Rows are already-built JSON because the decode that produces them is
/// async and per-reply — a `Render` impl is synchronous by design, so the
/// decoding happens where the session is and the rendering happens here.
#[derive(Debug, Clone, Serialize)]
pub struct GetReport {
    /// The selector as asked. A GET's coverage claim is exactly this and no
    /// wider (RFC 09 §5.1 O5).
    pub selector: String,
    /// Seconds waited — the other half of the claim, and what makes a silent
    /// result legible.
    pub timeout_s: u64,
    #[serde(skip)]
    pub answers: Vec<serde_json::Value>,
}

impl Render for GetReport {
    const FAMILY: &'static str = "get";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = serde_json::Map::new();
        e.insert("selector".into(), self.selector.clone().into());
        e.insert("timeout_s".into(), self.timeout_s.into());
        e.insert("answers".into(), self.answers.len().into());
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for a in &self.answers {
            out(Row::of("answer", a));
        }
    }

    /// Nothing: `get`'s human rendering decodes payloads and prints them with
    /// their type tags, which happens on the async path beside the session.
    /// This report exists for the machine formats, and saying that by
    /// rendering an empty table is more honest than pretending otherwise.
    fn table(&self, _t: &mut Table) {}

    fn notes(&self) -> Vec<Note> {
        if self.answers.is_empty() {
            return vec![Note::silence(format!(
                "no replies to {} within {}s. Nobody is registered for it, nobody \
                 who is was up, or the timeout was short — and the three are \
                 different",
                self.selector, self.timeout_s
            ))];
        }
        Vec::new()
    }
}

/// One context as `zenctl context list` shows it.
#[derive(Debug, Clone, Serialize)]
pub struct ContextRow {
    pub name: String,
    /// The `*` in the table: this is the one commands resolve against.
    pub current: bool,
    /// Three states, and the reason this is an `Option<String>` rather than a
    /// `String`: a stored **empty** base (`""`, the legal bus-root deployment)
    /// must not read like an unset one. In the table they are `""` and `—`;
    /// in json they are `""` and absent (RFC 09 §5.1 O4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    pub connect: Vec<String>,
}

/// What `zenctl context list` found in the config file.
#[derive(Debug, Clone, Serialize)]
pub struct ContextList {
    pub path: String,
    pub contexts: Vec<ContextRow>,
}

impl Render for ContextList {
    const FAMILY: &'static str = "context-list";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = serde_json::Map::new();
        e.insert("path".into(), self.path.clone().into());
        e.insert("contexts".into(), self.contexts.len().into());
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for c in &self.contexts {
            out(Row::of("context", c));
        }
    }

    fn table(&self, t: &mut Table) {
        // The marker rides the name cell rather than owning a column of its
        // own: a one-character column still pads to the gutter, which turned
        // `* lab` into `*  lab`.
        let mut g = Grid::unheaded(2);
        for c in &self.contexts {
            g.row([
                Cell::text(format!("{} {}", if c.current { "*" } else { " " }, c.name)),
                Cell::text(format!(
                    "base={}  connect={:?}",
                    match c.base.as_deref() {
                        Some("") => "\"\"",
                        Some(b) => b,
                        None => "-",
                    },
                    c.connect
                )),
            ]);
        }
        t.grid(g);
    }

    fn notes(&self) -> Vec<Note> {
        if self.contexts.is_empty() {
            // The empty case is a state, not an error, and it says what to
            // type next — which is why it is a note rather than an early
            // `return` from `table()` (#199).
            return vec![Note::summary(
                "no contexts. create one:\n  zenctl context create lab \
                 --base zensight -c tcp/127.0.0.1:7447",
            )];
        }
        vec![Note::summary(format!(
            "{} context(s) in {}",
            self.contexts.len(),
            self.path
        ))]
    }
}

/// One context in full — `zenctl context show`.
///
/// The envelope carries `StoredContext`'s own fields **flat**, because the
/// question a CI job asks is `zenctl context show --format json | jq -r .base`
/// and a nested `.context.base` would be a shape invented for no reader (#242).
#[derive(Debug, Clone, Serialize)]
pub struct ContextShow {
    pub name: String,
    pub current: bool,
    #[serde(flatten)]
    pub context: zenkey_fleet::context_store::StoredContext,
}

impl Render for ContextShow {
    const FAMILY: &'static str = "context";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(m)) => m,
            _ => serde_json::Map::new(),
        }
    }

    fn rows(&self, _out: &mut dyn FnMut(Row)) {}

    fn table(&self, t: &mut Table) {
        // TOML, as it has always been: this is the file's own language, and a
        // person reading `context show` is usually about to edit it.
        match toml::to_string_pretty(&self.context) {
            Ok(text) => t.line(text.trim_end()),
            Err(e) => t.line(format!("(cannot render: {e})")),
        };
    }

    fn notes(&self) -> Vec<Note> {
        vec![Note::summary(format!(
            "context {:?}{}",
            self.name,
            if self.current { " (selected)" } else { "" }
        ))]
    }
}

/// A context verb that changed the file: create, update, select, remove.
///
/// Notes-only. There is no wire shape to invent — the answer is "it happened",
/// and the envelope's `action`/`name` pair is already the whole of it.
#[derive(Debug, Clone, Serialize)]
pub struct ContextAction {
    /// `created`, `updated`, `selected` or `removed`.
    pub action: &'static str,
    pub name: String,
    /// Whether this context is now the one commands resolve against.
    pub current: bool,
}

impl Render for ContextAction {
    const FAMILY: &'static str = "context-action";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(m)) => m,
            _ => serde_json::Map::new(),
        }
    }

    fn rows(&self, _out: &mut dyn FnMut(Row)) {}

    fn table(&self, _t: &mut Table) {}

    fn notes(&self) -> Vec<Note> {
        vec![Note::summary(format!(
            "{} context {:?}{}",
            self.action,
            self.name,
            if self.current && self.action != "removed" {
                " (selected)"
            } else {
                ""
            }
        ))]
    }
}

/// `registry lint` — the RFC 08 §5 lints, passed.
///
/// Notes-only, deliberately. This command's stated value is *the build's own
/// wording, verbatim*; a `{dir, passed, message}` struct would be a second
/// shape to keep in step with `zenkey_build`'s, over an exit code and a
/// sentence. A failure is still an `Err` carrying the build's text.
#[derive(Debug, Clone, Serialize)]
pub struct LintReport {
    pub dir: String,
}

impl Render for LintReport {
    const FAMILY: &'static str = "registry-lint";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = serde_json::Map::new();
        e.insert("dir".into(), self.dir.clone().into());
        e.insert("passed".into(), true.into());
        e
    }

    fn rows(&self, _out: &mut dyn FnMut(Row)) {}

    fn table(&self, _t: &mut Table) {}

    fn notes(&self) -> Vec<Note> {
        vec![Note::summary(format!(
            "{}: registry lints pass (RFC 08 §5).",
            self.dir
        ))]
    }
}

/// `registry lock` — what the RFC 08 §3.1 compatibility lock became.
#[derive(Debug, Clone, Serialize)]
pub struct LockReport {
    pub path: String,
    pub created: bool,
    pub added: usize,
    pub retired: usize,
    /// Every pin a `--force` broke, verbatim. Loud by contract: the escape
    /// hatch is legal, silent it is not.
    pub forced: Vec<String>,
}

impl Render for LockReport {
    const FAMILY: &'static str = "registry-lock";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(m)) => m,
            _ => serde_json::Map::new(),
        }
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for broken in &self.forced {
            out(Row::tagged(
                "forced-break",
                serde_json::json!({ "detail": broken }),
            ));
        }
    }

    fn table(&self, _t: &mut Table) {}

    fn notes(&self) -> Vec<Note> {
        let mut notes = vec![Note::summary(format!(
            "{}: {} ({} pinned entr{} added, {} released)",
            self.path,
            if self.created { "created" } else { "updated" },
            self.added,
            if self.added == 1 { "y" } else { "ies" },
            self.retired,
        ))];
        for broken in &self.forced {
            notes.push(Note::caveat(format!("FORCED BREAK:\n{broken}")).cite("RFC 08 §3.1"));
        }
        notes
    }
}

/// `zenctl cache refresh|clear` — what happened to this tool's own disk
/// footprint.
#[derive(Debug, Clone, Serialize)]
pub struct CacheAction {
    /// `refreshed` or `cleared`.
    pub action: &'static str,
    pub dir: String,
    /// Producers cached, for `refresh`. Absent for `clear`, which counts
    /// nothing — not zero (RFC 09 §5.1 O4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slices: Option<usize>,
    /// Whether there was anything there. `clear` on an absent directory is
    /// the desired end state, not a failure.
    pub existed: bool,
}

impl Render for CacheAction {
    const FAMILY: &'static str = "cache-action";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(m)) => m,
            _ => serde_json::Map::new(),
        }
    }

    fn rows(&self, _out: &mut dyn FnMut(Row)) {}

    fn table(&self, _t: &mut Table) {}

    fn notes(&self) -> Vec<Note> {
        vec![Note::summary(match (self.action, self.slices) {
            ("refreshed", Some(n)) => format!("cached {n} producer(s) to {}", self.dir),
            (_, _) if self.existed => format!("removed {}", self.dir),
            _ => format!("{} does not exist — nothing to clear", self.dir),
        })]
    }
}
