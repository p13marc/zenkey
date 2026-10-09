//! Report types this crate owns.
//!
//! `zenkey_fleet::report` is the contract shared with zengui and pinned by
//! `report_contract.rs`. These are not that: the name cache is *this tool's
//! own disk footprint*, no second frontend renders it, and putting it in the
//! shared surface would freeze a shape nobody else reads. The governing rule
//! catches the difference — a report belongs to the engine iff it is the
//! return value of a fleet function *and* a second frontend would plausibly
//! render it.

use serde::Serialize;

use crate::render::{Cell, Grid, Note, Render, Row, Table, envelope_of};

/// One namespace's names, as the completion cache holds them: what the
/// last presence read there saw.
#[derive(Debug, Clone, Serialize)]
pub struct CachedNamespace {
    /// The namespace; empty for the bus root.
    pub namespace: String,
    pub services: usize,
    pub ifaces: usize,
}

/// What `zenctl cache show` found on disk.
#[derive(Debug, Clone, Serialize)]
pub struct CacheReport {
    /// The directory itself — the point of the command is that a tool which
    /// leaves files on a user's disk can be asked where they are (#54).
    pub dir: String,
    /// The namespaces `namespace list` last saw.
    pub listed: Vec<String>,
    /// What each namespace's last presence read saw.
    pub seen: Vec<CachedNamespace>,
}

fn namespace_label(ns: &str) -> &str {
    if ns.is_empty() { "(empty)" } else { ns }
}

impl Render for CacheReport {
    const FAMILY: &'static str = "cache";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = serde_json::Map::new();
        e.insert("dir".into(), self.dir.clone().into());
        e.insert("listed".into(), self.listed.clone().into());
        e.insert("namespaces".into(), self.seen.len().into());
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for s in &self.seen {
            out(Row::of("namespace", s));
        }
    }

    fn table(&self, t: &mut Table) {
        t.line(&self.dir);
        let mut g = Grid::unheaded(2).max(0, 24);
        for s in &self.seen {
            g.row([
                Cell::text(format!("  {}", namespace_label(&s.namespace))),
                Cell::text(format!(
                    "{} service(s), {} interface(s)",
                    s.services, s.ifaces
                )),
            ]);
        }
        t.grid(g);
        if !self.listed.is_empty() {
            let names: Vec<&str> = self.listed.iter().map(|n| namespace_label(n)).collect();
            t.line(format!("namespaces listed: {}", names.join(", ")));
        }
    }

    fn notes(&self) -> Vec<Note> {
        if self.seen.is_empty() && self.listed.is_empty() {
            return vec![Note::coverage(
                "empty — completion falls back to the static command tree. Any presence \
                 read fills it (`service list`, `iface list`, `zenctl cache refresh`), and \
                 `namespace list` adds the namespaces",
            )];
        }
        vec![Note::coverage(format!(
            "{} namespace(s), from the last sighting — suggestions, not an inventory. \
             Nothing reads this except shell completion, and no command answers from it",
            self.seen.len()
        ))]
    }
}

/// A key-expression relation, answered (#198).
///
/// zenctl-local for the same reason as [`CacheReport`], and a sharper one: the
/// algebra is `zenoh-keyexpr`'s, not the fleet's. No second frontend renders a
/// key-relation verdict, and pinning `{op, a, b, answer}` into the shared
/// contract would freeze a shape nobody else reads.
/// Which relation `key includes` / `key intersects` asked about.
///
/// A closed set the compiler can check. As a `String` it was matched with a
/// `_` fallback in **two** places — here and `cmd/key.rs` — so a third
/// relation added tomorrow would compute *and* render as `intersects`,
/// silently and in two different files (#356).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyOp {
    /// Every key `b` names is one `a` names.
    Includes,
    /// `a` and `b` can name a key in common.
    Intersects,
}

impl KeyOp {
    /// The wire token, exactly as it serializes.
    pub fn as_str(self) -> &'static str {
        match self {
            KeyOp::Includes => "includes",
            KeyOp::Intersects => "intersects",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct KeyRelation {
    pub op: KeyOp,
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
        envelope_of(self)
    }

    fn rows(&self, _out: &mut dyn FnMut(Row)) {}

    fn table(&self, t: &mut Table) {
        let verb = match (self.op, self.answer) {
            (KeyOp::Includes, true) => format!("includes every key {} names", self.b),
            (KeyOp::Includes, false) => format!("does not include all of {}", self.b),
            (KeyOp::Intersects, true) => format!("and {} can name a common key", self.b),
            (KeyOp::Intersects, false) => format!("and {} share no key", self.b),
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
        envelope_of(self)
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
    pub timeout_s: f64,
    /// Replies that arrived and were not read, because the reply bound bit
    /// (#339, RFC 13 §3 O6). Absent — not `0` — when it did not: an unheld
    /// fact is absent from the row rather than null, and a bound that never
    /// bit hid nothing to disclose.
    #[serde(skip_serializing_if = "is_zero")]
    pub elided: u64,
    #[serde(skip)]
    pub answers: Vec<serde_json::Value>,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl Render for GetReport {
    const FAMILY: &'static str = "get";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = serde_json::Map::new();
        e.insert("selector".into(), self.selector.clone().into());
        e.insert("timeout_s".into(), self.timeout_s.into());
        e.insert("answers".into(), self.answers.len().into());
        if self.elided > 0 {
            e.insert("elided".into(), self.elided.into());
        }
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
        let mut notes = Vec::new();
        if self.answers.is_empty() {
            notes.push(Note::silence(format!(
                "no replies to {} within {}s. Nothing may hold it, its holders may \
                 be down, or the timeout was short — and the three are different",
                self.selector, self.timeout_s
            )));
        }
        if self.elided > 0 {
            notes.push(Note::coverage(format!(
                "{} further repl(y|ies) arrived and were not read — the reply \
                 bound bit, so this is a sample of the answers, not all of them",
                self.elided
            )));
        }
        notes
    }

    fn scope(&self) -> Option<crate::render::ObservedScope> {
        Some(crate::render::ObservedScope {
            asked: vec![self.selector.clone()],
            window_s: Some(self.timeout_s),
        })
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
    pub context: zenkey_explorer_config::StoredContext,
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

/// `zenctl cache refresh|clear` — what happened to this tool's own disk
/// footprint.
#[derive(Debug, Clone, Serialize)]
pub struct CacheAction {
    /// `refreshed` or `cleared`.
    pub action: &'static str,
    pub dir: String,
    /// Services the presence read saw, for `refresh`. Absent for `clear`,
    /// which counts nothing — not zero (RFC 09 §5.1 O4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub services: Option<usize>,
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
        vec![Note::summary(match (self.action, self.services) {
            ("refreshed", Some(n)) => format!("cached {n} service(s) to {}", self.dir),
            (_, _) if self.existed => format!("removed {}", self.dir),
            _ => format!("{} does not exist — nothing to clear", self.dir),
        })]
    }
}
