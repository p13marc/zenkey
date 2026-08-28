//! Field intelligence (#223): the stuck sensor that passes every check.
//!
//! Validation is per-sample and binary; per-key stats are about *arrival*.
//! Between them sits the failure mode neither can see: a key publishing at
//! exactly its declared rate, payload validating perfectly, whose
//! `temperature_c` has not moved in four hours because the sensor died. This
//! module makes a *field* — a dotted path inside a decoded structural value
//! ([`crate::model::decode::structural_value`]) — a first-class observed thing:
//! bounded per-path statistics over a window (presence, type stability,
//! last-change, change count, numeric min/max/last, small-domain distinct
//! values) yielding three finding kinds:
//!
//! - **`field-vanished`** — a path present in earlier samples, absent since,
//!   while later payloads still parse. If the schema declares it optional,
//!   validation reads `Valid` without it *by construction*. Vanished requires
//!   SEEN then absent: a path never observed is not a vanished path
//!   (RFC 09 §5.1 O4 — "not asked" never renders as "no").
//! - **`field-stuck`** — a numeric path unchanged across a span long relative
//!   to the subject's declared `ttl_s`, while the key kept publishing. An
//!   observation with a stated window, never a verdict: a constant-by-design
//!   field always reads this way, and with no declared `ttl_s` there is
//!   nothing to be long *relative to*, so nothing fires (O4).
//! - **`field-new`** — a path the served schema never declared:
//!   `schema-drift` at field granularity. Judged only where the served
//!   schema actually enumerates properties; a free-form subtree is
//!   unjudgeable, not new (O4).
//!
//! The path table is **bounded and reports what it dropped** (O6) — never a
//! silent truncation. The judges are pure functions over the observation,
//! the house pattern of [`crate::judge::condition`] (#227) and [`crate::judge::budget`]
//! (#221): testable without a bus. Surfaces: `zenctl field <selector>
//! [--for S]`, the doctor listen phase (#161) via the appended
//! [`crate::report::CheckId`], and — **deferred to a later zengui
//! window** — the Inspector field table with per-field sparklines through
//! the existing `series.rs`/`spark.rs` gap-drawing. This chunk ships the
//! engine and zenctl halves only.

use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::time::Duration;

use crate::Result;
use serde_json::Value;
use zenoh::Session;

use crate::judge::common::{FINDING_CAP, producer_of};
use crate::model::decode::SchemaStore;
use crate::model::examples::Examples;
use crate::model::jsonschema::{COMBINATORS, resolve_ref};
use crate::model::registry::SliceSet;
use crate::report::{CheckId, DoctorFinding, DoctorSeverity, FieldReport, FieldRow};

/// Default bound on the per-path table, across every key the window sees. A
/// high-cardinality document can blow a path table the way a `{var}` family
/// blows a key table (#221), so the cap and its cost are reported like every
/// other bound (RFC 09 §5.1 O6).
pub const DEFAULT_MAX_PATHS: usize = 512;

/// How many distinct values a path may show and still count as small-domain.
pub const DISTINCT_CAP: usize = 8;

/// A value longer than this cannot be a small-domain member — tracking a set
/// of megabyte blobs would move the memory bound into the values.
const DISTINCT_VALUE_CAP: usize = 64;

/// How long an unchanged span must be, relative to the declared `ttl_s`,
/// before `field-stuck` fires. One ttl unchanged is a slow sensor; three is
/// three consecutive refresh deadlines carrying the same number.
pub const STUCK_TTL_FACTOR: f64 = 3.0;

/// How many consecutive trailing document samples a seen path must be absent
/// from before `field-vanished` fires — one missing sample is jitter.
pub const VANISHED_MIN_ABSENT: u64 = 3;

/// A stuck path must have been observed at least this often — "the key kept
/// publishing" is part of the finding's meaning.
const STUCK_MIN_SEEN: u64 = 3;

/// Dropped-path examples carried by the bound report (O6 names, not just
/// counts — enough to recognise the document that exploded).
const DROPPED_EXAMPLE_CAP: usize = 5;

/// The dotted path of a document root that is not an object (a bare scalar
/// or array payload) — one field, named like `jq`'s root.
pub const ROOT_PATH: &str = "$";

/// How deep the declared-path walk may descend before it stops and calls the
/// rest of that subtree open. `$defs` nest, and a recursive type would
/// otherwise walk forever.
const SCHEMA_DEPTH_CAP: usize = 32;

/// How many subschema nodes one document's walk may visit. The depth cap
/// alone does not bound the *work*: a combinator tree fans out
/// multiplicatively, so N nested two-armed `oneOf`s are 2^N walks at a depth
/// of N. This is the bound that actually holds, and a served schema is a
/// stranger's document — the walk runs on whatever a producer replies with.
///
/// Deliberately not an O6-reported bound like the path table's: this one
/// bounds a *fetched artifact* walked once per (producer, type) and cached,
/// not a population of observations, and exhausting it degrades to
/// "unjudgeable" — which the report already carries — rather than to a
/// silently shorter answer.
const SCHEMA_NODE_BUDGET: usize = 10_000;

// ─── the observation ────────────────────────────────────────────────────────

/// Bounded per-(key, dotted-path) statistics over one window.
///
/// Fed structural values as they ride; judged afterwards by the pure
/// functions below. The bound is over the *total* path population across
/// keys, and every path refused for the bound is counted and exemplified —
/// a table that silently stops growing is indistinguishable from a document
/// that stopped changing (O6).
#[derive(Debug, Clone)]
pub struct FieldObservation {
    max_paths: usize,
    keys: BTreeMap<String, KeyFields>,
    paths: usize,
    /// Refused path observations: the count *and* the names, in one
    /// collector, so they cannot drift apart (O6).
    dropped: Examples<String>,
}

/// One key's document samples and the paths inside them.
#[derive(Debug, Clone, Default)]
pub struct KeyFields {
    /// Samples that carried a structural document (the population every
    /// presence ratio is against).
    pub documents: u64,
    /// Samples that carried none (plain text, opaque bytes) — fields are
    /// unobservable for them, which is stated, not folded into absence (O4).
    pub undocumented: u64,
    /// Samples whose payload was past [`crate::OBSERVE_LIMIT`] and therefore never
    /// read. **Not** `undocumented`: "we did not look" is not "there was
    /// nothing to see" (RFC 09 §5.1 O4).
    pub unread: u64,
    /// Per dotted path, the stats.
    pub paths: BTreeMap<String, PathStats>,
}

/// What one dotted path did across one key's window.
#[derive(Debug, Clone)]
pub struct PathStats {
    /// Document samples in which the path was present.
    pub seen: u64,
    /// Window-relative seconds of first / last presence.
    pub first_at_s: f64,
    pub last_at_s: f64,
    /// The key's document-sample index at last presence — what "absent since"
    /// is measured against.
    pub last_seen_sample: u64,
    /// JSON kind → occurrences (type stability: one entry is stable).
    pub kinds: BTreeMap<&'static str, u64>,
    /// Times the value differed from the previous observation of this path.
    pub changes: u64,
    /// Window-relative seconds of the last change; `None` = never changed.
    pub last_change_at_s: Option<f64>,
    /// Numeric min/max/last, when the path carried numbers.
    pub num_min: Option<f64>,
    pub num_max: Option<f64>,
    pub num_last: Option<f64>,
    /// Small-domain distinct values (canonical JSON), until the domain
    /// overflows [`DISTINCT_CAP`].
    pub distinct: BTreeSet<String>,
    /// The domain outgrew the cap (or carried values too large to track) —
    /// the set above is then cleared, not silently partial.
    pub distinct_overflow: bool,
    /// Fingerprint of the last observed value, for change detection.
    last_fingerprint: Option<u64>,
}

impl PathStats {
    fn new(at_s: f64, sample: u64) -> PathStats {
        PathStats {
            seen: 0,
            first_at_s: at_s,
            last_at_s: at_s,
            last_seen_sample: sample,
            kinds: BTreeMap::new(),
            changes: 0,
            last_change_at_s: None,
            num_min: None,
            num_max: None,
            num_last: None,
            distinct: BTreeSet::new(),
            distinct_overflow: false,
            last_fingerprint: None,
        }
    }

    fn observe(&mut self, at_s: f64, sample: u64, value: &Value) {
        self.seen += 1;
        self.last_at_s = at_s;
        self.last_seen_sample = sample;
        *self.kinds.entry(kind_of(value)).or_default() += 1;
        let canonical = serde_json::to_string(value).unwrap_or_default();
        let fingerprint = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            canonical.hash(&mut h);
            h.finish()
        };
        if let Some(prev) = self.last_fingerprint
            && prev != fingerprint
        {
            self.changes += 1;
            self.last_change_at_s = Some(at_s);
        }
        self.last_fingerprint = Some(fingerprint);
        if let Some(n) = value.as_f64() {
            self.num_min = Some(self.num_min.map_or(n, |m| m.min(n)));
            self.num_max = Some(self.num_max.map_or(n, |m| m.max(n)));
            self.num_last = Some(n);
        }
        if !self.distinct_overflow {
            if canonical.len() > DISTINCT_VALUE_CAP {
                self.distinct_overflow = true;
                self.distinct.clear();
            } else {
                self.distinct.insert(canonical);
                if self.distinct.len() > DISTINCT_CAP {
                    self.distinct_overflow = true;
                    self.distinct.clear();
                }
            }
        }
    }
}

impl FieldObservation {
    pub fn new(max_paths: usize) -> FieldObservation {
        FieldObservation {
            max_paths: max_paths.max(1),
            keys: BTreeMap::new(),
            paths: 0,
            dropped: Examples::new(DROPPED_EXAMPLE_CAP),
        }
    }

    /// Feed one sample. `doc` is the structural value when the payload
    /// carried one ([`crate::model::decode::structural_value`]); `None` counts the
    /// sample as undocumented rather than pretending its fields were absent.
    pub fn observe_unread(&mut self, key: &str) {
        self.keys.entry(key.to_string()).or_default().unread += 1;
    }

    /// Samples skipped because their payload was too large to read.
    pub fn unread(&self) -> u64 {
        self.keys.values().map(|k| k.unread).sum()
    }

    pub fn observe(&mut self, key: &str, at_s: f64, doc: Option<&Value>) {
        let entry = self.keys.entry(key.to_string()).or_default();
        let Some(doc) = doc else {
            entry.undocumented += 1;
            return;
        };
        entry.documents += 1;
        let sample = entry.documents;
        let mut leaves = Vec::new();
        flatten(doc, &mut leaves);
        for (path, value) in leaves {
            match entry.paths.get_mut(&path) {
                Some(stats) => stats.observe(at_s, sample, value),
                None if self.paths < self.max_paths => {
                    let mut stats = PathStats::new(at_s, sample);
                    stats.observe(at_s, sample, value);
                    entry.paths.insert(path, stats);
                    self.paths += 1;
                }
                // The bound: refused, counted, exemplified — never silent.
                None => self.dropped.push_with(|| format!("{key} · {path}")),
            }
        }
    }

    /// Per-key observations, for the judges and the report rows.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &KeyFields)> {
        self.keys.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn keys_seen(&self) -> usize {
        self.keys.len()
    }

    /// Distinct (key, path) pairs currently tracked.
    pub fn paths(&self) -> usize {
        self.paths
    }

    pub fn max_paths(&self) -> usize {
        self.max_paths
    }

    /// Path observations refused to stay within the bound (RFC 09 §5.1 O6).
    pub fn dropped_paths(&self) -> u64 {
        self.dropped.total() as u64
    }

    /// Up to a handful of `key · path` names among the refused (the cap is
    /// `DROPPED_EXAMPLE_CAP` — enough to recognise the document that
    /// exploded, without pasting the population).
    pub fn dropped_examples(&self) -> &[String] {
        self.dropped.as_slice()
    }

    /// Samples that carried no structural document, across every key.
    pub fn undocumented(&self) -> u64 {
        self.keys.values().map(|k| k.undocumented).sum()
    }
}

/// One observed value's JSON kind, for the type-stability count.
fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Flatten a structural document into dotted leaf paths. Objects recurse
/// (`a.b.c`); arrays and scalars are leaves — indexing into arrays would
/// mint a path per element and hand the cardinality problem a wildcard. A
/// non-object root is the single leaf [`ROOT_PATH`]; an empty object is its
/// own leaf (a present-but-empty subtree is presence, not absence).
pub fn flatten<'v>(doc: &'v Value, out: &mut Vec<(String, &'v Value)>) {
    fn walk<'v>(prefix: &str, v: &'v Value, out: &mut Vec<(String, &'v Value)>) {
        match v {
            Value::Object(map) if !map.is_empty() => {
                for (name, child) in map {
                    let path = if prefix.is_empty() {
                        name.clone()
                    } else {
                        format!("{prefix}.{name}")
                    };
                    walk(&path, child, out);
                }
            }
            leaf => out.push(if prefix.is_empty() {
                (ROOT_PATH.to_string(), leaf)
            } else {
                (prefix.to_string(), leaf)
            }),
        }
    }
    walk("", doc, out);
}

// ─── the declared-path surface (field-new's other half) ─────────────────────

/// The dotted paths a served JSON Schema declares, with the subtrees it
/// leaves free-form. `field-new` is judgeable only against this: a schema
/// kind this build cannot enumerate (protobuf, CDR), or a document that
/// enumerates nothing, yields `None` and no finding — unjudgeable is not
/// new (O4).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeclaredPaths {
    declared: BTreeSet<String>,
    /// Prefixes whose subschema enumerates nothing — anything below is
    /// unjudgeable rather than undeclared.
    open: BTreeSet<String>,
}

impl DeclaredPaths {
    /// Walk a JSON Schema document into the dotted paths it declares.
    ///
    /// Three shapes, because a `schemars`-derived document uses all three:
    /// `properties` descends; `oneOf`/`anyOf`/`allOf` branches are **unioned**
    /// — a path declared in *any* branch is declared — and local `$ref`
    /// pointers resolve against the document root (`#/$defs/…`,
    /// `#/definitions/…`).
    ///
    /// Both additions fix the same defect (#384). A tagged enum renders as a
    /// `oneOf` whose every branch declares the tag and content fields, and
    /// `schemars` hoists every nested named type into `$defs` and references
    /// it — so a walker that descends `properties` alone sees neither, and
    /// reports every field of every tagged enum and every nested struct as
    /// `field-new` on every sample. The check's own purpose goes with it: a
    /// warning that fires on conforming traffic cannot carry a real one.
    ///
    /// A `$ref` this walk cannot follow — external, missing, already on the
    /// current chain (a recursive type), or past a bound below — leaves its
    /// subtree **open**, never closed. "Could not follow" is unjudgeable, and
    /// rendering unjudgeable as "undeclared" is the O4 violation this check
    /// exists inside of.
    ///
    /// `None` when the document enumerates nothing, or when the root itself
    /// is unjudgeable: there is then no declared surface and `field-new` is
    /// unjudgeable for the whole type, which is not the same as clean (O4).
    pub fn from_json_schema(doc: &Value) -> Option<DeclaredPaths> {
        let mut out = DeclaredPaths::default();
        let mut walk = Walk {
            root: doc,
            visiting: BTreeSet::new(),
            budget: SCHEMA_NODE_BUDGET,
            root_open: false,
        };
        walk.node(doc, "", 0, &mut out);
        (!walk.root_open && !out.declared.is_empty()).then_some(out)
    }

    /// Whether the schema accounts for this path — declared, or under a
    /// free-form subtree it deliberately left open.
    pub fn accounts_for(&self, path: &str) -> bool {
        if path == ROOT_PATH || self.declared.contains(path) {
            return true;
        }
        // Any ancestor being open makes the path unjudgeable, not new.
        let mut prefix = String::new();
        for chunk in path.split('.') {
            if !prefix.is_empty() {
                prefix.push('.');
            }
            prefix.push_str(chunk);
            if self.open.contains(&prefix) {
                return true;
            }
        }
        false
    }
}

/// The state one document's walk carries: the root to resolve `$ref` against,
/// the pointers on the current chain (which is what stops a recursive type
/// from recursing forever), and the two bounds.
struct Walk<'d> {
    root: &'d Value,
    visiting: BTreeSet<String>,
    budget: usize,
    /// The root subtree itself turned out unjudgeable — there is no path to
    /// hang that on, so it collapses the whole document to `None`.
    root_open: bool,
}

impl Walk<'_> {
    /// Mark a subtree unjudgeable. At the root there is no path to mark, so
    /// the whole document goes.
    fn open(&mut self, prefix: &str, out: &mut DeclaredPaths) {
        if prefix.is_empty() {
            self.root_open = true;
        } else {
            out.open.insert(prefix.to_string());
        }
    }

    /// One subschema node. `prefix` is the dotted path it describes — empty
    /// at the root.
    fn node(&mut self, node: &Value, prefix: &str, depth: usize, out: &mut DeclaredPaths) {
        // Both bounds land on the same honest answer: stop, and say the rest
        // is unknown rather than absent (O4).
        if depth > SCHEMA_DEPTH_CAP || self.budget == 0 {
            self.open(prefix, out);
            return;
        }
        self.budget -= 1;

        // A boolean schema (`true`/`false`) enumerates nothing, and neither
        // does anything malformed. Not open: `false` accepts no instance and
        // `true` is not an object surface — neither claims a subtree.
        let Some(obj) = node.as_object() else {
            return;
        };

        // Whether this node said anything about its own shape. A node that
        // did not, and calls itself an object, is a free-form subtree.
        let mut described = false;

        if let Some(pointer) = obj.get("$ref").and_then(Value::as_str) {
            match resolve_ref(self.root, pointer) {
                Some(target) if !self.visiting.contains(pointer) => {
                    self.visiting.insert(pointer.to_string());
                    self.node(target, prefix, depth + 1, out);
                    self.visiting.remove(pointer);
                    described = true;
                }
                // Unresolvable, or already on this chain. Either way what is
                // below cannot be enumerated from here.
                _ => self.open(prefix, out),
            }
        }

        // Union, not intersection: `oneOf` is how a sum type reaches the
        // wire, and each branch declares the fields its own variant carries.
        // A field present in one branch is declared by the schema, and the
        // finer verdict — "declared only in some branches" — is a different
        // check from "never declared".
        for combinator in COMBINATORS {
            if let Some(arms) = obj.get(combinator).and_then(Value::as_array) {
                for arm in arms {
                    self.node(arm, prefix, depth + 1, out);
                }
                described = true;
            }
        }

        if let Some(props) = obj.get("properties").and_then(Value::as_object) {
            for (name, child) in props {
                let path = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}.{name}")
                };
                self.node(child, &path, depth + 1, out);
                out.declared.insert(path);
            }
            described = true;
        }

        // An object subschema with no enumerated properties is a free-form
        // subtree: everything under it is unjudgeable.
        if !described && obj.get("type").and_then(Value::as_str) == Some("object") {
            self.open(prefix, out);
        }
    }
}

// ─── the judges (pure — the #227/#221 house pattern) ────────────────────────

/// What the judges may know about one key beyond its stats. Everything is
/// optional, and every `None` suppresses the finding that needed it rather
/// than guessing (O4).
#[derive(Debug, Clone, Default)]
pub struct KeyFieldContext {
    /// The subject's declared `ttl_s`, when the key refined to a registered
    /// subject — what `field-stuck` is long *relative to*.
    pub ttl_s: Option<i64>,
    /// The registered type name, for the `field-new` evidence line.
    pub type_name: Option<String>,
    /// The served schema's declared paths, when derivable.
    pub declared: Option<DeclaredPaths>,
}

/// `field-vanished`: SEEN, then absent from at least [`VANISHED_MIN_ABSENT`]
/// consecutive trailing document samples. A path never seen is not vanished
/// — that would be rendering "not asked" as "no" (O4).
pub fn judge_vanished(stats: &PathStats, key_documents: u64) -> bool {
    stats.seen > 0 && key_documents.saturating_sub(stats.last_seen_sample) >= VANISHED_MIN_ABSENT
}

/// `field-stuck`: a purely numeric path, zero changes, observed at least
/// `STUCK_MIN_SEEN` times across a span of at least [`STUCK_TTL_FACTOR`] ×
/// the declared `ttl_s`. No declared ttl, no finding: there is nothing to be
/// long relative to (O4) — and the numeric requirement is what keeps a
/// constant-by-design hostname or enum out of the noise.
pub fn judge_stuck(stats: &PathStats, ttl_s: Option<i64>) -> bool {
    let Some(ttl) = ttl_s.filter(|t| *t > 0) else {
        return false;
    };
    stats.changes == 0
        && stats.seen >= STUCK_MIN_SEEN
        && stats.kinds.len() == 1
        && stats.kinds.contains_key("number")
        && (stats.last_at_s - stats.first_at_s) >= STUCK_TTL_FACTOR * ttl as f64
}

/// `field-new`: the served schema enumerates its properties and this path is
/// not among them (nor under a free-form subtree). With no declared surface
/// there is no finding — unjudgeable is not new (O4).
pub fn judge_new(path: &str, declared: Option<&DeclaredPaths>) -> bool {
    declared.is_some_and(|d| !d.accounts_for(path))
}

/// Judge a whole observation into doctor findings, capped per check the way
/// the doctor caps its listen findings. `ctx` supplies what is known per key;
/// a key it does not name gets the empty context (everything unjudgeable).
pub fn judge_fields(
    obs: &FieldObservation,
    window_s: f64,
    ctx: &BTreeMap<String, KeyFieldContext>,
) -> Vec<DoctorFinding> {
    let empty = KeyFieldContext::default();

    let mut vanished = Examples::new(FINDING_CAP);

    let mut stuck = Examples::new(FINDING_CAP);

    let mut new = Examples::new(FINDING_CAP);

    for (key, fields) in obs.iter() {
        let c = ctx.get(key).unwrap_or(&empty);
        for (path, stats) in &fields.paths {
            if judge_vanished(stats, fields.documents) {
                vanished.push_with(|| DoctorFinding {
                    severity: DoctorSeverity::Warning,
                    check: CheckId::FieldVanished,
                    subject: format!("{key} · {path}"),
                    evidence: format!(
                        "present in {} of {} document sample(s) in {window_s:.0}s, absent \
                         from the last {} — seen, then gone; a schema that declares it \
                         optional reads Valid without it by construction",
                        stats.seen,
                        fields.documents,
                        fields.documents - stats.last_seen_sample
                    ),
                    citation: None,
                });
            }
            if judge_stuck(stats, c.ttl_s) {
                let ttl = c.ttl_s.unwrap_or(0);
                stuck.push_with(|| DoctorFinding {
                    severity: DoctorSeverity::Warning,
                    check: CheckId::FieldStuck,
                    subject: format!("{key} · {path}"),
                    evidence: format!(
                        "value {} unchanged across {} sample(s) spanning {:.1}s — at least \
                         {STUCK_TTL_FACTOR:.0}× the declared ttl_s {ttl}s — while the key \
                         kept publishing. An observation over this {window_s:.0}s window, \
                         not a verdict: a constant-by-design field always reads this way",
                        stats
                            .num_last
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| "?".into()),
                        stats.seen,
                        stats.last_at_s - stats.first_at_s,
                    ),
                    citation: Some("RFC 04 §1.2".into()),
                });
            }
            if judge_new(path, c.declared.as_ref()) {
                new.push_with(|| DoctorFinding {
                    severity: DoctorSeverity::Warning,
                    check: CheckId::FieldNew,
                    subject: format!("{key} · {path}"),
                    evidence: format!(
                        "present in {} of {} document sample(s) but never declared by the \
                         served schema{} — schema drift at field granularity",
                        stats.seen,
                        fields.documents,
                        c.type_name
                            .as_deref()
                            .map(|t| format!(" for {t}"))
                            .unwrap_or_default()
                    ),
                    citation: Some("RFC 08 §7".into()),
                });
            }
        }
    }
    let mut findings = Vec::new();
    for (check, hits) in [
        (CheckId::FieldVanished, vanished),
        (CheckId::FieldStuck, stuck),
        (CheckId::FieldNew, new),
    ] {
        let more = hits.more("more path(s) with the same finding");
        findings.extend(hits.into_vec());
        if let Some(evidence) = more {
            findings.push(DoctorFinding {
                severity: DoctorSeverity::Info,
                check,
                subject: "fleet".into(),
                evidence,
                citation: None,
            });
        }
    }
    findings
}

// ─── the window runner (zenctl field) ───────────────────────────────────────

/// What a `zenctl field` run watches, and its bounds.
#[derive(Debug, Clone)]
pub struct FieldSpec {
    /// Full wire selector to watch (the session is un-namespaced, RFC 09 §5).
    pub selector: String,
    /// The observation window.
    pub window: Duration,
    /// The per-path table bound (RFC 09 §5.1 O6).
    pub max_paths: usize,
}

/// Watch one selector for the window and report per-path statistics plus the
/// three findings. The subscriber is declared **before** the window opens
/// (O4). `slices: None` means no registry was loaded: declared `ttl_s` and
/// type names are then unknown, `field-stuck` and `field-new` are
/// unjudgeable, and the report says so rather than reading clean (O4; #246).
pub async fn run_field(
    fleet: &crate::Fleet<'_>,
    slices: Option<&SliceSet>,
    store: &SchemaStore,
    spec: &FieldSpec,
) -> Result<FieldReport> {
    use crate::{FleetEvent, StreamItem};

    let (session, base) = (fleet.session(), fleet.base());

    let monitor = crate::Monitor::start(session, crate::MonitorSpec::default()).await?;

    let mut events = monitor.events();

    // Declared before the window opens: not-asked must never read as "no" —
    // and a declaration that fails takes the monitor down with it (#336).
    let monitor = monitor.watching([spec.selector.as_str()]).await?;
    let opened = tokio::time::Instant::now();
    let deadline = opened + spec.window;

    let mut obs = FieldObservation::new(spec.max_paths);
    let mut samples: u64 = 0;
    let mut dropped: u64 = 0;
    // Bounded (#107): one projection per distinct key, LRU past the bound,
    // evictions counted into the report (O6).
    let mut facts = crate::model::facts::FactsCache::default();

    // One timer for the whole window, not one per iteration (#346).
    // `sleep_until` builds a future and registers a timer each time it
    // is evaluated, and a `select!` in a loop evaluates it on every
    // pass — at 100k samples/s that is 100k registrations a second for
    // a deadline that never moves.
    let window_over = tokio::time::sleep_until(deadline);
    tokio::pin!(window_over);
    loop {
        let item = tokio::select! {
            item = events.recv() => item,
            () = &mut window_over => break,
        };
        match item {
            Some(StreamItem::Event(FleetEvent::Sample(s))) => {
                samples += 1;
                // Bounded, and the skip is counted rather than read as an
                // absent document (#346).
                let bytes = s.payload.to_bytes();
                if bytes.len() > crate::model::decode::OBSERVE_LIMIT {
                    obs.observe_unread(&s.key);
                } else {
                    let doc = crate::model::decode::structural_value(&bytes);
                    obs.observe(&s.key, opened.elapsed().as_secs_f64(), doc.as_ref());
                }
                facts.ensure(base, &s.key, slices);
            }
            Some(StreamItem::Dropped(n)) => dropped += n,
            Some(_) => continue,
            None => break,
        }
    }
    monitor.shutdown().await?;

    let window_s = spec.window.as_secs_f64();
    let ctx = field_context(session, store, slices, &facts).await;
    let findings = judge_fields(&obs, window_s, &ctx);

    let mut rows = Vec::new();
    for (key, fields) in obs.iter() {
        for (path, stats) in &fields.paths {
            rows.push(FieldRow {
                key: key.to_string(),
                path: path.clone(),
                seen: stats.seen,
                documents: fields.documents,
                kinds: stats.kinds.keys().map(|k| k.to_string()).collect(),
                changes: stats.changes,
                last_change_s: stats.last_change_at_s,
                min: stats.num_min,
                max: stats.num_max,
                last: stats.num_last,
                // `None` = the domain outgrew the cap; `Some` is total.
                values: (!stats.distinct_overflow)
                    .then(|| stats.distinct.iter().cloned().collect()),
            });
        }
    }

    Ok(FieldReport {
        selector: spec.selector.clone(),
        window_s,
        samples,
        keys_seen: obs.keys_seen(),
        dropped,
        undocumented: obs.undocumented(),
        unread: obs.unread(),
        registry_loaded: slices.is_some(),
        paths: obs.paths(),
        max_paths: obs.max_paths(),
        paths_dropped: obs.dropped_paths(),
        paths_dropped_examples: obs.dropped_examples().to_vec(),
        facts_evicted: facts.evicted(),
        rows,
        findings,
    })
}

/// Build the per-key judge context: declared `ttl_s`/type from the resolved
/// facts, declared paths from each producer's served schema (fetched through
/// the store's ordinary cache — one `describe` per producer, not per key).
pub(crate) async fn field_context(
    session: &Session,
    store: &SchemaStore,
    slices: Option<&SliceSet>,
    facts: &crate::model::facts::FactsCache,
) -> BTreeMap<String, KeyFieldContext> {
    let mut declared_cache: BTreeMap<(String, String), Option<DeclaredPaths>> = BTreeMap::new();

    let mut ctx = BTreeMap::new();

    for (key, f) in facts.iter() {
        let mut c = KeyFieldContext::default();
        if let crate::model::facts::Registration::Registered(sf) = &f.registration {
            c.ttl_s = sf.ttl_s;
            c.type_name = Some(sf.type_name.clone());
            if let Some(producer) = producer_of(f, slices)
                && !sf.type_name.is_empty()
            {
                let cache_key = (producer.clone(), sf.type_name.clone());
                if !declared_cache.contains_key(&cache_key) {
                    let declared = store
                        .schema_for(session, &producer, &sf.type_name)
                        .await
                        .and_then(|schema| {
                            schema
                                .json_document()
                                .and_then(DeclaredPaths::from_json_schema)
                        });
                    declared_cache.insert(cache_key.clone(), declared);
                }
                c.declared = declared_cache.get(&cache_key).cloned().flatten();
            }
        }
        ctx.insert(key.to_string(), c);
    }
    ctx
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn observe_docs(obs: &mut FieldObservation, key: &str, docs: &[(f64, Value)]) {
        for (at, doc) in docs {
            obs.observe(key, *at, Some(doc));
        }
    }

    /// Objects flatten to dotted leaves; arrays and scalars are leaves; a
    /// non-object root is the single `$` field.
    #[test]
    fn flattening_recurses_objects_and_stops_at_arrays() {
        let doc = json!({"a": {"b": 1, "c": [1, 2]}, "d": "x", "e": {}});
        let mut leaves = Vec::new();
        flatten(&doc, &mut leaves);
        let paths: Vec<&str> = leaves.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, ["a.b", "a.c", "d", "e"]);

        let scalar = json!(42.0);
        let mut leaves = Vec::new();
        flatten(&scalar, &mut leaves);
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].0, ROOT_PATH);
    }

    /// The acceptance bound: the path table refuses past its cap, counts
    /// every refusal, and names examples — never a silent truncation (O6).
    #[test]
    fn the_path_table_is_bounded_and_reports_what_it_dropped() {
        let mut obs = FieldObservation::new(4);
        let wide: serde_json::Map<String, Value> =
            (0..20).map(|i| (format!("f{i:02}"), json!(i))).collect();
        obs.observe("k", 0.0, Some(&Value::Object(wide)));
        assert_eq!(obs.paths(), 4, "the bound holds");
        assert_eq!(obs.dropped_paths(), 16, "every refusal is counted");
        assert!(
            obs.dropped_examples().iter().any(|e| e.contains("k · f04")),
            "refused paths are named: {:?}",
            obs.dropped_examples()
        );
        // A tracked path keeps updating even while the table is full.
        obs.observe("k", 1.0, Some(&json!({"f00": 9})));
        let (_, fields) = obs.iter().next().unwrap();
        assert_eq!(fields.paths["f00"].seen, 2);
    }

    /// Vanished needs SEEN then absent: a path never observed is not a
    /// vanished path — "not asked" never renders as "no" (O4).
    #[test]
    fn vanished_needs_seen_then_absent() {
        let mut obs = FieldObservation::new(64);
        let with = json!({"seq": 1, "opt": true});
        let without = json!({"seq": 2});
        observe_docs(
            &mut obs,
            "k",
            &[
                (0.0, with),
                (1.0, without.clone()),
                (2.0, without.clone()),
                (3.0, without.clone()),
                (4.0, without),
            ],
        );
        let (_, fields) = obs.iter().next().unwrap();
        assert!(judge_vanished(&fields.paths["opt"], fields.documents));
        assert!(
            !judge_vanished(&fields.paths["seq"], fields.documents),
            "a path present in the last sample has not vanished"
        );
        let findings = judge_fields(&obs, 5.0, &BTreeMap::new());
        let vanished: Vec<_> = findings
            .iter()
            .filter(|f| f.check == CheckId::FieldVanished)
            .collect();
        assert_eq!(vanished.len(), 1, "{findings:?}");
        assert!(vanished[0].subject.ends_with("· opt"));
        assert!(
            vanished[0].evidence.contains("1 of 5"),
            "presence is counted: {}",
            vanished[0].evidence
        );
        // One missing sample is jitter, not a vanish.
        let mut obs = FieldObservation::new(64);
        observe_docs(
            &mut obs,
            "k",
            &[
                (0.0, json!({"opt": 1})),
                (1.0, json!({})),
                (2.0, json!({"opt": 1})),
            ],
        );
        assert!(
            judge_fields(&obs, 3.0, &BTreeMap::new())
                .iter()
                .all(|f| f.check != CheckId::FieldVanished)
        );
    }

    /// Stuck is numeric, ttl-relative, and never a verdict without a declared
    /// ttl: a frozen number across ≥3×ttl fires, a changing one does not, a
    /// constant string (hostname) does not, and no ttl means nothing to be
    /// long relative to (O4).
    #[test]
    fn stuck_is_numeric_ttl_relative_and_suppressed_without_a_ttl() {
        let mut obs = FieldObservation::new(64);
        let docs: Vec<(f64, Value)> = (0..8)
            .map(|i| {
                (
                    i as f64,
                    json!({"temperature_c": 21.5, "seq": i, "host": "web-1"}),
                )
            })
            .collect();
        observe_docs(&mut obs, "k", &docs);
        let (_, fields) = obs.iter().next().unwrap();
        assert!(judge_stuck(&fields.paths["temperature_c"], Some(1)));
        assert!(
            !judge_stuck(&fields.paths["seq"], Some(1)),
            "a changing numeric is not stuck"
        );
        assert!(
            !judge_stuck(&fields.paths["host"], Some(1)),
            "a constant string is constant by design, not stuck"
        );
        assert!(
            !judge_stuck(&fields.paths["temperature_c"], None),
            "no declared ttl_s: nothing to be long relative to (O4)"
        );
        assert!(
            !judge_stuck(&fields.paths["temperature_c"], Some(10)),
            "a 7s span is not long relative to a 10s ttl"
        );

        let ctx: BTreeMap<String, KeyFieldContext> = [(
            "k".to_string(),
            KeyFieldContext {
                ttl_s: Some(1),
                ..KeyFieldContext::default()
            },
        )]
        .into();
        let findings = judge_fields(&obs, 8.0, &ctx);
        let stuck: Vec<_> = findings
            .iter()
            .filter(|f| f.check == CheckId::FieldStuck)
            .collect();
        assert_eq!(stuck.len(), 1, "{findings:?}");
        assert!(stuck[0].subject.ends_with("· temperature_c"));
        assert!(stuck[0].evidence.contains("21.5"), "{}", stuck[0].evidence);
        assert!(
            stuck[0].evidence.contains("not a verdict"),
            "stuck is an observation with a stated window: {}",
            stuck[0].evidence
        );
        assert!(
            stuck[0].evidence.contains("ttl_s 1s"),
            "the ttl it is relative to is stated: {}",
            stuck[0].evidence
        );
    }

    /// `field-new` is judged only against a schema that enumerates its
    /// properties: an undeclared path fires, a declared one does not, a
    /// free-form subtree is unjudgeable, and no schema means no finding (O4).
    #[test]
    fn new_is_judged_only_against_a_declaring_schema() {
        let declared = DeclaredPaths::from_json_schema(&json!({
            "type": "object",
            "properties": {
                "seq": {"type": "number"},
                "nested": {"type": "object", "properties": {"x": {"type": "number"}}},
                "freeform": {"type": "object"},
            },
        }))
        .expect("the schema enumerates properties");
        assert!(!judge_new("seq", Some(&declared)));
        assert!(!judge_new("nested.x", Some(&declared)));
        assert!(judge_new("extra", Some(&declared)));
        assert!(judge_new("nested.y", Some(&declared)));
        assert!(
            !judge_new("freeform.anything.at.all", Some(&declared)),
            "a free-form subtree is unjudgeable, not new"
        );
        assert!(!judge_new("extra", None), "no schema, no finding (O4)");
        assert_eq!(
            DeclaredPaths::from_json_schema(&json!({"type": "object"})),
            None,
            "a schema with no properties judges nothing"
        );

        let mut obs = FieldObservation::new(64);
        observe_docs(&mut obs, "k", &[(0.0, json!({"seq": 1, "extra": 2}))]);
        let ctx: BTreeMap<String, KeyFieldContext> = [(
            "k".to_string(),
            KeyFieldContext {
                type_name: Some("Health".into()),
                declared: Some(declared),
                ..KeyFieldContext::default()
            },
        )]
        .into();
        let findings = judge_fields(&obs, 1.0, &ctx);
        let new: Vec<_> = findings
            .iter()
            .filter(|f| f.check == CheckId::FieldNew)
            .collect();
        assert_eq!(new.len(), 1, "{findings:?}");
        assert!(new[0].subject.ends_with("· extra"));
        assert!(new[0].evidence.contains("Health"), "{}", new[0].evidence);
        assert_eq!(new[0].citation.as_deref(), Some("RFC 08 §7"));
    }

    /// The document `schemars` actually emits for an adjacently-tagged enum:
    /// a `oneOf` whose every branch declares both the tag and the content,
    /// reached through a `$ref` into `$defs`. Walking `properties` alone saw
    /// neither, so `value.type` and `value.value` — fields the schema
    /// *requires* — were reported as drift on every sample of every key
    /// (#384: 141 warnings in one 15s window, all of them this).
    #[test]
    fn a_tagged_enum_declares_its_variant_fields_rather_than_drifting() {
        let doc = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "TelemetryPoint",
            "type": "object",
            "properties": {
                "ts_ns": {"type": "integer", "format": "uint64"},
                "value": {"$ref": "#/$defs/TelemetryValue"},
            },
            "required": ["ts_ns", "value"],
            "$defs": {
                "TelemetryValue": {
                    "description": "Typed telemetry value.",
                    "oneOf": [
                        {"type": "object",
                         "properties": {"type": {"const": "counter", "type": "string"},
                                        "value": {"format": "uint64", "type": "integer"}},
                         "required": ["type", "value"]},
                        {"type": "object",
                         "properties": {"type": {"const": "gauge", "type": "string"},
                                        "value": {"format": "double", "type": "number"}},
                         "required": ["type", "value"]},
                        {"type": "object",
                         "properties": {"type": {"const": "text", "type": "string"},
                                        "value": {"type": "string"}},
                         "required": ["type", "value"]},
                    ],
                },
            },
        });
        let declared =
            DeclaredPaths::from_json_schema(&doc).expect("the schema enumerates properties");

        assert!(!judge_new("ts_ns", Some(&declared)));
        assert!(
            !judge_new("value.type", Some(&declared)),
            "the tag is declared by every branch"
        );
        assert!(
            !judge_new("value.value", Some(&declared)),
            "the content is declared by every branch"
        );
        // The union is not a blanket amnesty: the enum enumerated its fields,
        // so one it never declared is still drift.
        assert!(judge_new("value.unit", Some(&declared)));
        assert!(judge_new("extra", Some(&declared)));
    }

    /// A `$ref` is the normal shape for *any* nested struct, not only an
    /// enum — `schemars` hoists every named type into `$defs`. Unresolved,
    /// the whole subtree read as undeclared.
    #[test]
    fn a_ref_into_defs_resolves_and_its_fields_are_declared() {
        let declared = DeclaredPaths::from_json_schema(&json!({
            "type": "object",
            "properties": {"cpu": {"$ref": "#/$defs/Cpu"}},
            "$defs": {
                "Cpu": {
                    "type": "object",
                    "properties": {
                        "usage": {"type": "number"},
                        "core": {"$ref": "#/$defs/Core"},
                    },
                },
                "Core": {"type": "object", "properties": {"id": {"type": "integer"}}},
            },
        }))
        .expect("the schema enumerates properties");

        assert!(!judge_new("cpu.usage", Some(&declared)));
        assert!(
            !judge_new("cpu.core.id", Some(&declared)),
            "a $ref inside a $ref resolves too"
        );
        assert!(judge_new("cpu.missing", Some(&declared)));
    }

    /// A `$ref` this walk cannot follow leaves its subtree **open**, never
    /// closed. Rendering "could not follow" as "undeclared" is the O4
    /// violation the whole check sits inside.
    #[test]
    fn an_unfollowable_ref_opens_its_subtree_rather_than_condemning_it() {
        for pointer in [
            "#/$defs/Absent",                     // dangling, same document
            "https://example.invalid/Thing.json", // somebody else's document
        ] {
            let declared = DeclaredPaths::from_json_schema(&json!({
                "type": "object",
                "properties": {
                    "seq": {"type": "number"},
                    "opaque": {"$ref": pointer},
                },
            }))
            .expect("the schema still enumerates `seq`");
            assert!(!judge_new("seq", Some(&declared)));
            assert!(
                !judge_new("opaque.anything.at.all", Some(&declared)),
                "{pointer}: an unresolvable $ref is unjudgeable, not new"
            );
        }
    }

    /// A recursive type terminates, and the leg that would have looped is
    /// open rather than condemned.
    #[test]
    fn a_recursive_ref_terminates_and_opens_where_it_stops() {
        let declared = DeclaredPaths::from_json_schema(&json!({
            "$ref": "#/$defs/Node",
            "$defs": {
                "Node": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "parent": {"anyOf": [{"$ref": "#/$defs/Node"}, {"type": "null"}]},
                    },
                },
            },
        }))
        .expect("the schema enumerates properties");

        assert!(!judge_new("name", Some(&declared)));
        assert!(!judge_new("parent", Some(&declared)));
        assert!(
            !judge_new("parent.name", Some(&declared)),
            "the cycle stops at `parent`, and what it could not enumerate is open"
        );
    }

    /// `allOf` is how a flattened struct reaches the wire, and every arm's
    /// properties apply at once.
    #[test]
    fn all_of_arms_are_unioned() {
        let declared = DeclaredPaths::from_json_schema(&json!({
            "allOf": [
                {"type": "object", "properties": {"a": {"type": "number"}}},
                {"type": "object", "properties": {"b": {"type": "string"}}},
            ],
        }))
        .expect("the arms enumerate properties");

        assert!(!judge_new("a", Some(&declared)));
        assert!(!judge_new("b", Some(&declared)));
        assert!(judge_new("c", Some(&declared)));
    }

    /// A document that enumerates nothing stays unjudgeable, whichever shape
    /// the nothing takes — the O4 floor the walk must not lower while it
    /// learns to see more.
    #[test]
    fn a_document_that_enumerates_nothing_still_judges_nothing() {
        for doc in [
            json!({"type": "object"}),
            json!({}),
            json!({"$ref": "#/$defs/Absent"}),
            json!({"oneOf": [{"type": "string"}, {"type": "number"}]}),
            json!(true),
        ] {
            assert_eq!(
                DeclaredPaths::from_json_schema(&doc),
                None,
                "nothing enumerated, nothing judged: {doc}"
            );
        }
    }

    /// The walk runs on whatever a producer replies with, so it is bounded in
    /// work, not only in depth: nested two-armed combinators fan out
    /// multiplicatively, and thirty levels of them is a **small** document —
    /// thirty `$defs` entries — describing a billion-node walk. The depth cap
    /// alone would not stop it; the node budget does. Exhausting either
    /// degrades to open (O4), never to a partial surface reported as whole.
    #[test]
    fn a_fanning_combinator_tree_terminates_within_its_budget() {
        const LEVELS: usize = 30;
        let mut defs = serde_json::Map::new();
        for level in 0..LEVELS {
            // Both arms point at the next level: 2^30 walks, 30 entries.
            let next = json!({"$ref": format!("#/$defs/L{}", level + 1)});
            defs.insert(format!("L{level}"), json!({"anyOf": [next.clone(), next]}));
        }
        defs.insert(
            format!("L{LEVELS}"),
            json!({"type": "object", "properties": {"leaf": {"type": "number"}}}),
        );
        let doc = json!({
            "type": "object",
            "properties": {"seq": {"type": "number"}, "deep": {"$ref": "#/$defs/L0"}},
            "$defs": Value::Object(defs),
        });

        // Terminating at all is the assertion. Whatever it concluded about
        // the deep subtree, it must not have concluded a false "new" there,
        // and it must still have judged the shallow field it did reach.
        let declared = DeclaredPaths::from_json_schema(&doc).expect("`seq` is enumerated");
        assert!(!judge_new("seq", Some(&declared)));
        assert!(
            !judge_new("deep.leaf", Some(&declared)),
            "a subtree the budget could not finish is unjudgeable, not new"
        );
        assert!(
            judge_new("absent", Some(&declared)),
            "the check still works"
        );
    }

    /// The small-domain set is total or absent: past the cap it clears and
    /// flags, never silently partial.
    #[test]
    fn distinct_values_are_total_or_flagged_overflowed() {
        let mut obs = FieldObservation::new(8);
        for i in 0..3 {
            obs.observe("k", i as f64, Some(&json!({"mode": format!("m{}", i % 2)})));
        }
        let (_, fields) = obs.iter().next().unwrap();
        let stats = &fields.paths["mode"];
        assert!(!stats.distinct_overflow);
        assert_eq!(stats.distinct.len(), 2);

        let mut obs = FieldObservation::new(8);
        for i in 0..20 {
            obs.observe("k", i as f64, Some(&json!({"mode": i})));
        }
        let (_, fields) = obs.iter().next().unwrap();
        let stats = &fields.paths["mode"];
        assert!(stats.distinct_overflow, "20 values are not a small domain");
        assert!(
            stats.distinct.is_empty(),
            "an overflowed set is cleared, not silently partial"
        );
        // …and the numeric stats still carry the range.
        assert_eq!(stats.num_min, Some(0.0));
        assert_eq!(stats.num_max, Some(19.0));
        assert_eq!(stats.changes, 19);
    }

    /// An undocumented sample (text, opaque bytes) is counted apart — it must
    /// not read as "every field absent" (O4).
    #[test]
    fn undocumented_samples_do_not_fake_a_vanish() {
        let mut obs = FieldObservation::new(8);
        obs.observe("k", 0.0, Some(&json!({"opt": 1})));
        for i in 1..6 {
            obs.observe("k", i as f64, None);
        }
        let (_, fields) = obs.iter().next().unwrap();
        assert_eq!(fields.documents, 1);
        assert_eq!(fields.undocumented, 5);
        assert!(
            judge_fields(&obs, 6.0, &BTreeMap::new())
                .iter()
                .all(|f| f.check != CheckId::FieldVanished),
            "five undocumented samples are five unobservables, not a vanish"
        );
    }
}
