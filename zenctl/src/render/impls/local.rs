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
        }
        t.grid(g);
    }

    fn notes(&self) -> Vec<Note> {
        vec![
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
        ]
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
