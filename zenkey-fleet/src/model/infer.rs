//! Registry inference (#225, RFC 08 §6.1 v1.34): a draft `registry/` from
//! the wire, marked as a draft.
//!
//! The registry is the adoption cliff — a fleet without one gets nothing
//! from this suite's best half, and hand-writing two hundred entries is why
//! it never gets one. Yet every input a `[[subject]]` wants is already in
//! this crate's hands: the grammar gives producer, class and subject; the
//! wire gives the encoding and the QoS that actually rode; a count over a
//! window gives a rate class and a refresh interval; the key population
//! gives observed expansions; the sample documents give a JSON Schema; RFC
//! 08 §4's suffix rule turns `_ms`/`_bytes`/`_percent` leaves into units.
//! This module joins them into an [`InferReport`] that
//! [`to_draft_toml`] writes out, one file per producer.
//!
//! Pure, the house pattern of [`crate::judge::field`]: nothing here takes a
//! session, so a `.zrec` replays through the same inference as live
//! traffic, and the tests feed synthetic rows.
//!
//! # What a draft is
//!
//! The inverse of the lie RFC 08 §6.1 forbids: it describes surfaces that
//! *were* served, by an author who cannot vouch that they *will* be. So the
//! file carries `draft = true` (a marker `zenkey-build` **refuses**, not a
//! comment a human is trusted to notice), `compat = "none"`, no `since`,
//! and a header saying every field is a guess. Everything the observation
//! could not establish is **absent**, never defaulted — the one exception
//! the emitter makes is stated on the entry as a comment (`rate = "rare"`
//! for an events family with at most one sample, which is the tightest
//! class consistent with the evidence, not a default). Observed QoS is a
//! **comment**, never a field: writing it as `qos = …` would launder a
//! current publisher bug into a contract. The one `qos` a draft writes is
//! the alert family's — `state` under a leading `alert` chunk — because RFC
//! 08 §5 (v1.23) *requires* `qos = "alert"` there: that is the family's
//! normative profile, not a reading of the wire, and the entry says so.
//!
//! # The literal-vs-`{var}` heuristic
//!
//! The registry convention's rule is semantic — *a leaf naming a distinct
//! measurement is a literal; a leaf that is a value of a dimension is a
//! `{var}`* — and observation cannot read meaning. What it can read is
//! structure and population, so the heuristic uses three kinds of evidence,
//! evaluated bottom-up on one chunk trie per (producer, class) that spans
//! every origin:
//!
//! * **E1 — interior siblings with one sub-structure.** Under one prefix,
//!   sibling chunks that each carry a subtree are one `{var}` position when
//!   their subtrees are shape-identical *after* recursive inference of the
//!   merged subtree, they share at least half their concrete tails (so
//!   `sda`/`sdb` merge and `system`/`tcp` do not), and every inferred tail
//!   is served by every member. `cpu0/usage`, `cpu1/usage` → `{v}/usage`.
//!   Siblings are only tried against each other when their tails reach the
//!   same depths, which is what bounds the pairwise work.
//! * **E2 — leaf siblings whose membership varies by origin.** Leaf chunks
//!   under one prefix are one `{var}` when some of them are published by
//!   only a subset of the origins that publish under the prefix: a
//!   population that differs from host to host is data, not vocabulary.
//!   Otherwise every leaf is a literal measurement.
//! * **E3 — write-once leaves under `events`.** RFC 04 §1.3 makes an events
//!   key write-once, so ≥ 2 leaf siblings each seen exactly once are one
//!   `{var}` even from a single origin.
//! * **Rest.** A prefix whose children are host-conditional at two depths
//!   — at least one leaf and at least two interior chunks, with unlike
//!   subtrees, each missing from some origin — with at least
//!   [`REST_MIN_PATHS`] distinct tails is a device-defined tree and becomes
//!   one trailing `{rest...}`.
//!
//! **The failure modes, named** (each is also a comment on the entry it
//! produces, so the reviewer sees it where it matters):
//!
//! 1. *One expansion cannot prove a dimension.* A `{var}` observed with one
//!    value, or a leaf dimension observed from one origin, is emitted as a
//!    literal. A single-origin window therefore drafts every leaf literal,
//!    and the run's caveat says so.
//! 2. *A literal pair with one shape looks like a dimension.* `cpu/usage`
//!    and `mem/usage` are indistinguishable from `{v}/usage`; E1 merges them
//!    and the entry lists the siblings it merged.
//! 3. *A closed vocabulary looks like measurements.* `tcp/{state}` whose
//!    eleven states every host publishes has no E2 signal and comes out as
//!    eleven literals. The convention chose `{var}` there by meaning, which
//!    is exactly what a review supplies.
//! 4. *A dimension whose values also carry value-specific subjects splits.*
//!    `cgroup/{resource}/pressure/{stat}` beside `cgroup/cpu/nr_throttled`
//!    and `cgroup/memory/current`: the members' subtrees differ, so E1
//!    keeps them literal and the shared `pressure/{stat}` family appears
//!    once per value.
//! 5. *A host-conditional measurement reads as a dimension.* A host with no
//!    swap never publishes `memory/swap_total`; E2 then reads the whole
//!    `memory/*` leaf set as `memory/{v}`. The entry lists the siblings.
//! 6. *A device-defined tree with too few host-varying paths stays
//!    literal*, and a producer with host-conditional subjects at two depths
//!    can become `{rest...}` — the rest rule's two sides.
//!
//! Var names come from a hint slice's pattern when one binds with the same
//! var positions, else `{v<depth>}`. Cardinality is the largest distinct
//! member count any one origin published at the position, rounded **up**
//! to a power of ten. `rate` (events only) is the busiest origin's count
//! over the span, classed `rare` ≤ 1/h, `low` ≤ 60/h, else `burst(n/h)`
//! with `n` rounded up to a power of ten. `ttl_s` (state) is twice the
//! longest median inter-arrival among the expansions, rounded up — a
//! *field* because the lint requires one, with a comment saying it is a
//! hint; absent, with the lint left to name it, when no expansion refreshed
//! inside the window. `unit` follows RFC 08 §4's suffix rule and nothing
//! else; `_total` becomes a `kind` guess instead. `encoding` is a field
//! only when every sample agreed.
//!
//! The schema inference is deliberately minimal — kinds per dotted path
//! from [`PathStats`], objects with `properties` and a `required` of what
//! every document carried, arrays without `items`, `integer` when every
//! number was integral — and neither this nor [`crate::model::jsonschema`]
//! should become a JSON Schema implementation.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use zenkey::pattern::{PatternChunk, SubjectPattern};
use zenkey::toml_quote;

use crate::judge::field::{FieldObservation, PathStats, ROOT_PATH};
use crate::model::facts::{KeyShape, OriginKind, describe_key};
use crate::model::registry::SliceSet;
use crate::report::{InferReport, InferredProducer, InferredSubject, InferredType};

/// How many distinct concrete tails a prefix needs before its
/// host-conditional children are read as a device-defined `{rest...}` tree.
pub const REST_MIN_PATHS: usize = 3;

/// The fraction of concrete tails two interior siblings must share before
/// E1 merges them — `sda`/`sdb` share everything, `system`/`tcp` nothing,
/// and the threshold is what keeps one shared leaf name from merging two
/// unrelated measurement groups.
const ANCHOR_MIN: f64 = 0.5;

/// How many sibling values an entry's `{var}` comment names.
const SIBLING_EXAMPLES: usize = 5;

/// Intervals kept per key for the median inter-arrival (bounded, O6).
const INTERVAL_CAP: usize = 64;

/// The default key bound — one trie node per distinct wire key.
pub const DEFAULT_MAX_KEYS: usize = 20_000;
/// The default path bound across every key's documents.
pub const DEFAULT_MAX_PATHS: usize = 8_192;

// ─── the observation ────────────────────────────────────────────────────────

/// Who a key's `[[subject]]` belongs to: a host producer, or a service
/// origin (whose registry file is a `[service]` file, RFC 03 §1.5).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Owner {
    Producer(String),
    Service(String),
}

impl Owner {
    fn file_name(&self) -> String {
        match self {
            Owner::Producer(p) => p.clone(),
            Owner::Service(o) => o.trim_start_matches('@').to_string(),
        }
    }
}

/// What one wire key did over the window.
#[derive(Debug, Clone)]
struct KeyObs {
    owner: Owner,
    class: String,
    origin: String,
    tail: Vec<String>,
    samples: u64,
    last_at_s: Option<f64>,
    /// Inter-arrival seconds, bounded — the refresh evidence.
    intervals: Vec<f64>,
    encodings: BTreeMap<String, u64>,
    qos: BTreeMap<String, u64>,
    /// The leaf value's kind evidence: numeric samples, whether any
    /// decreased, whether any was negative; text / bool counts.
    numeric: u64,
    decreased: bool,
    negative: bool,
    last_num: Option<f64>,
    text: u64,
    boolean: u64,
}

impl KeyObs {
    fn observe(
        &mut self,
        at_s: f64,
        encoding: Option<&str>,
        qos: Option<&str>,
        doc: Option<&Value>,
    ) {
        self.samples += 1;
        if let Some(last) = self.last_at_s
            && at_s >= last
            && self.intervals.len() < INTERVAL_CAP
        {
            self.intervals.push(at_s - last);
        }
        self.last_at_s = Some(at_s);
        if let Some(e) = encoding {
            *self.encodings.entry(e.to_string()).or_default() += 1;
        }
        if let Some(q) = qos {
            *self.qos.entry(q.to_string()).or_default() += 1;
        }
        if let Some(v) = doc.and_then(leaf_value) {
            match v {
                Value::Number(n) => {
                    if let Some(n) = n.as_f64() {
                        self.numeric += 1;
                        if n < 0.0 {
                            self.negative = true;
                        }
                        if self.last_num.is_some_and(|p| n < p) {
                            self.decreased = true;
                        }
                        self.last_num = Some(n);
                    }
                }
                Value::String(_) => self.text += 1,
                Value::Bool(_) => self.boolean += 1,
                _ => {}
            }
        }
    }
}

/// The leaf value a `kind` guess is about: the document itself when it is
/// a scalar, else its `value` field (the RFC 08 §2 self-describing value
/// object and the reference `TelemetryPoint` both spell it so).
fn leaf_value(doc: &Value) -> Option<&Value> {
    match doc {
        Value::Object(m) => m.get("value"),
        Value::Array(_) => None,
        scalar => Some(scalar),
    }
}

/// Bounded per-key observations over one window, fed one sample at a time.
///
/// Keys are projected through [`describe_key`] under `base`; a key that
/// does not parse, or sits on a verbatim plane, is counted and skipped —
/// there is no `[[subject]]` to draft for it.
#[derive(Debug, Clone)]
pub struct InferObservation {
    base: String,
    max_keys: usize,
    keys: BTreeMap<String, KeyObs>,
    keys_refused: u64,
    fields: FieldObservation,
    samples: u64,
    unparsed: u64,
    off_plane: u64,
    first_at_s: Option<f64>,
    last_at_s: Option<f64>,
}

impl InferObservation {
    pub fn new(base: &str, max_keys: usize, max_paths: usize) -> InferObservation {
        InferObservation {
            base: base.to_string(),
            max_keys: max_keys.max(1),
            keys: BTreeMap::new(),
            keys_refused: 0,
            fields: FieldObservation::new(max_paths),
            samples: 0,
            unparsed: 0,
            off_plane: 0,
            first_at_s: None,
            last_at_s: None,
        }
    }

    /// Feed one sample. `qos` is the label the caller resolved for the
    /// wire's axes (a profile name when one matches, else the axes spelled
    /// out); `doc` the structural document when the payload carried one
    /// ([`crate::model::decode::structural_value`]) — `None` counts it as
    /// undocumented rather than pretending its fields were absent.
    pub fn observe(
        &mut self,
        key: &str,
        at_s: f64,
        encoding: Option<&str>,
        qos: Option<&str>,
        doc: Option<&Value>,
    ) {
        self.samples += 1;
        self.first_at_s = Some(self.first_at_s.map_or(at_s, |f| f.min(at_s)));
        self.last_at_s = Some(self.last_at_s.map_or(at_s, |l| l.max(at_s)));
        if let Some(obs) = self.keys.get_mut(key) {
            obs.observe(at_s, encoding, qos, doc);
            self.fields.observe(key, at_s, doc);
            return;
        }
        let d = describe_key(&self.base, key, None);
        let KeyShape::V1(facts) = &d.facts.shape else {
            self.unparsed += 1;
            return;
        };
        if !facts.class_kind.is_data_class() {
            self.off_plane += 1;
            return;
        }
        let owner = match (facts.origin_kind, &facts.producer) {
            (OriginKind::Host, Some(p)) => Owner::Producer(p.clone()),
            (OriginKind::Service, _) => Owner::Service(facts.origin.clone()),
            (OriginKind::Host, None) => {
                self.unparsed += 1;
                return;
            }
        };
        if facts.subject.is_empty() {
            self.unparsed += 1;
            return;
        }
        if self.keys.len() >= self.max_keys {
            self.keys_refused += 1;
            return;
        }
        let mut obs = KeyObs {
            owner,
            class: facts.class.clone(),
            origin: facts.origin.clone(),
            tail: facts.subject.clone(),
            samples: 0,
            last_at_s: None,
            intervals: Vec::new(),
            encodings: BTreeMap::new(),
            qos: BTreeMap::new(),
            numeric: 0,
            decreased: false,
            negative: false,
            last_num: None,
            text: 0,
            boolean: 0,
        };
        obs.observe(at_s, encoding, qos, doc);
        self.keys.insert(key.to_string(), obs);
        self.fields.observe(key, at_s, doc);
    }

    /// A sample whose payload was too large to read (O6): counted, never
    /// read as an absent document.
    pub fn observe_unread(
        &mut self,
        key: &str,
        at_s: f64,
        encoding: Option<&str>,
        qos: Option<&str>,
    ) {
        self.observe(key, at_s, encoding, qos, None);
        if self.keys.contains_key(key) {
            self.fields.observe_unread(key);
        }
    }

    pub fn samples(&self) -> u64 {
        self.samples
    }

    pub fn keys_seen(&self) -> usize {
        self.keys.len()
    }

    pub fn keys_refused(&self) -> u64 {
        self.keys_refused
    }

    /// First to last sample, seconds (0 for fewer than two samples).
    pub fn span_s(&self) -> f64 {
        match (self.first_at_s, self.last_at_s) {
            (Some(f), Some(l)) => (l - f).max(0.0),
            _ => 0.0,
        }
    }

    fn origins(&self) -> BTreeSet<&str> {
        self.keys.values().map(|k| k.origin.as_str()).collect()
    }
}

// ─── the trie ───────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct Node {
    children: BTreeMap<String, Node>,
    /// Keys whose tail ends here.
    terminal: Vec<usize>,
    /// Origins publishing anything at or under this node.
    origins: BTreeSet<String>,
    /// Every concrete tail below this node, `""` for "ends here".
    tails: BTreeSet<String>,
}

impl Node {
    fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }

    fn insert(&mut self, tail: &[String], origin: &str, key: usize) {
        self.origins.insert(origin.to_string());
        self.tails.insert(tail.join("/"));
        match tail.split_first() {
            None => self.terminal.push(key),
            Some((head, rest)) => self
                .children
                .entry(head.clone())
                .or_default()
                .insert(rest, origin, key),
        }
    }

    /// The depths of this node's tails — the coarse shape two siblings must
    /// share before E1 even tries to merge them. Names are deliberately
    /// not part of it: a nested dimension's values differ by name at every
    /// depth, and the anchor test is what reads the names.
    fn depth_profile(&self) -> BTreeSet<usize> {
        self.tails
            .iter()
            .map(|t| {
                if t.is_empty() {
                    0
                } else {
                    t.matches('/').count() + 1
                }
            })
            .collect()
    }
}

/// A `{var}` position's population: per origin, the member chunks.
#[derive(Debug, Clone, Default)]
struct VarPop {
    members: BTreeMap<String, BTreeSet<String>>,
}

impl VarPop {
    fn add(&mut self, origin: &str, chunk: &str) {
        self.members
            .entry(origin.to_string())
            .or_default()
            .insert(chunk.to_string());
    }

    fn all(&self) -> BTreeSet<&str> {
        self.members
            .values()
            .flat_map(|s| s.iter().map(String::as_str))
            .collect()
    }

    fn max_per_origin(&self) -> usize {
        self.members.values().map(BTreeSet::len).max().unwrap_or(0)
    }
}

#[derive(Debug, Clone)]
enum Chunk {
    Literal(String),
    Var(VarPop),
    Rest(VarPop),
}

/// One inferred family: the pattern, the keys it covers, the reasons.
#[derive(Debug, Clone)]
struct Family {
    chunks: Vec<Chunk>,
    members: Vec<usize>,
    comments: Vec<String>,
}

impl Family {
    fn matches(&self, tail: &[&str]) -> bool {
        let has_rest = matches!(self.chunks.last(), Some(Chunk::Rest(_)));
        let fixed = self.chunks.len() - usize::from(has_rest);
        if has_rest {
            if tail.len() <= fixed {
                return false;
            }
        } else if tail.len() != fixed {
            return false;
        }
        self.chunks.iter().zip(tail).all(|(c, t)| match c {
            Chunk::Literal(l) => l == t,
            Chunk::Var(_) | Chunk::Rest(_) => true,
        })
    }
}

/// A merged view: the concrete nodes standing at one inferred position.
struct View<'a> {
    nodes: Vec<&'a Node>,
}

impl<'a> View<'a> {
    fn origins(&self) -> BTreeSet<&'a str> {
        self.nodes
            .iter()
            .flat_map(|n| n.origins.iter().map(String::as_str))
            .collect()
    }

    fn children(&self) -> BTreeMap<&'a str, Vec<&'a Node>> {
        let mut out: BTreeMap<&str, Vec<&Node>> = BTreeMap::new();
        for n in &self.nodes {
            for (name, child) in &n.children {
                out.entry(name.as_str()).or_default().push(child);
            }
        }
        out
    }

    fn terminal(&self) -> Vec<usize> {
        self.nodes
            .iter()
            .flat_map(|n| n.terminal.iter().copied())
            .collect()
    }

    fn tails(&self) -> BTreeSet<&'a str> {
        self.nodes
            .iter()
            .flat_map(|n| n.tails.iter().map(String::as_str))
            .collect()
    }
}

fn origins_of(nodes: &[&Node]) -> BTreeSet<String> {
    nodes
        .iter()
        .flat_map(|n| n.origins.iter().cloned())
        .collect()
}

/// The inference over one view, bottom-up. Returns the families found at
/// or below it, each with `prefix` prepended.
fn infer_view(view: &View<'_>, prefix: &[Chunk], keys: &[KeyObs]) -> Vec<Family> {
    let mut out = Vec::new();
    let terminal = view.terminal();
    if !terminal.is_empty() {
        out.push(Family {
            chunks: prefix.to_vec(),
            members: terminal,
            comments: Vec::new(),
        });
    }
    let children = view.children();
    if children.is_empty() {
        return out;
    }
    let view_origins: BTreeSet<String> = view.origins().into_iter().map(str::to_string).collect();
    let partial = |nodes: &[&Node]| origins_of(nodes) != view_origins;

    // ── Rest: a device-defined tree ───────────────────────────────────────
    let mut partial_leaves = 0usize;
    let mut partial_interiors: Vec<BTreeSet<&str>> = Vec::new();
    for nodes in children.values() {
        let leaf = nodes.iter().all(|n| n.is_leaf());
        if !partial(nodes) {
            continue;
        }
        if leaf {
            partial_leaves += 1;
        } else {
            partial_interiors.push(
                View {
                    nodes: nodes.clone(),
                }
                .tails(),
            );
        }
    }
    let unlike = partial_interiors.len() >= 2 && {
        let mut distinct: Vec<&BTreeSet<&str>> = Vec::new();
        for s in &partial_interiors {
            if !distinct.contains(&s) {
                distinct.push(s);
            }
        }
        distinct.len() >= 2
    };
    let tails = view.tails();
    if partial_leaves >= 1 && unlike && tails.len() >= REST_MIN_PATHS {
        let mut pop = VarPop::default();
        let mut members = Vec::new();
        for n in &view.nodes {
            collect_rest(n, "", &mut pop, &mut members);
        }
        let mut chunks = prefix.to_vec();
        chunks.push(Chunk::Rest(pop));
        out.push(Family {
            chunks,
            members,
            comments: vec![format!(
                "inferred {{rest...}}: {} distinct tails below this prefix, host-conditional at two \
                 depths — a device-defined tree (RFC 08 §2); if these are fixed subjects, \
                 register them one by one",
                tails.len()
            )],
        });
        return out;
    }

    // ── Leaves: E2 / E3 ───────────────────────────────────────────────────
    let leaves: Vec<(&str, &Vec<&Node>)> = children
        .iter()
        .filter(|(_, nodes)| nodes.iter().all(|n| n.is_leaf()))
        .map(|(name, nodes)| (*name, nodes))
        .collect();
    let leaf_class = leaves
        .first()
        .and_then(|(_, nodes)| nodes.first())
        .and_then(|n| n.terminal.first())
        .map(|&k| keys[k].class.as_str());
    let any_partial_leaf = leaves.iter().any(|(_, nodes)| partial(nodes));
    let write_once = leaf_class == Some("events")
        && leaves.len() >= 2
        && leaves.iter().all(|(_, nodes)| {
            nodes
                .iter()
                .flat_map(|n| n.terminal.iter())
                .all(|&k| keys[k].samples == 1)
        });
    if !leaves.is_empty() && (any_partial_leaf || write_once) {
        let mut pop = VarPop::default();
        let mut members = Vec::new();
        for (name, nodes) in &leaves {
            for n in nodes.iter() {
                for o in &n.origins {
                    pop.add(o, name);
                }
                members.extend(n.terminal.iter().copied());
            }
        }
        let names: Vec<&str> = leaves.iter().map(|(n, _)| *n).collect();
        let reason = if write_once {
            "each seen once under events — write-once keys (RFC 04 §1.3)"
        } else {
            "their membership varies by origin; a host-conditional measurement would look the same"
        };
        let mut chunks = prefix.to_vec();
        chunks.push(Chunk::Var(pop));
        out.push(Family {
            chunks,
            members,
            comments: vec![format!(
                "inferred {{var}} from {} leaf sibling(s): {} — {reason}; review",
                names.len(),
                examples(&names)
            )],
        });
    } else {
        for (name, nodes) in &leaves {
            let mut chunks = prefix.to_vec();
            chunks.push(Chunk::Literal((*name).to_string()));
            out.push(Family {
                chunks,
                members: nodes
                    .iter()
                    .flat_map(|n| n.terminal.iter().copied())
                    .collect(),
                comments: Vec::new(),
            });
        }
    }

    // ── Interiors: E1 ─────────────────────────────────────────────────────
    let interiors: Vec<(&str, &Vec<&Node>)> = children
        .iter()
        .filter(|(_, nodes)| !nodes.iter().all(|n| n.is_leaf()))
        .map(|(name, nodes)| (*name, nodes))
        .collect();
    // Greedy clustering: a sibling joins the first cluster it merges with
    // consistently; a merge is tried only between siblings of one coarse
    // shape, and accepted by the E1 test in `merge_accepted`.
    // One cluster: the depth profile its members share, and the members.
    type Cluster<'n> = (BTreeSet<usize>, Vec<(&'n str, &'n Vec<&'n Node>)>);
    let mut clusters: Vec<Cluster<'_>> = Vec::new();
    for (name, nodes) in interiors {
        let shape: BTreeSet<usize> = nodes.iter().flat_map(|n| n.depth_profile()).collect();
        let mut placed = false;
        for (cluster_shape, members) in clusters.iter_mut() {
            if *cluster_shape != shape {
                continue;
            }
            let mut trial: Vec<&Node> = members
                .iter()
                .flat_map(|(_, n)| n.iter().copied())
                .collect();
            trial.extend(nodes.iter().copied());
            let per_member: Vec<Vec<&Node>> = members
                .iter()
                .map(|(_, n)| (*n).clone())
                .chain(std::iter::once(nodes.clone()))
                .collect();
            if merge_accepted(&trial, &per_member, prefix, keys) {
                members.push((name, nodes));
                placed = true;
                break;
            }
        }
        if !placed {
            clusters.push((shape, vec![(name, nodes)]));
        }
    }
    for (_, members) in clusters {
        if members.len() >= 2 {
            let mut pop = VarPop::default();
            for (name, nodes) in &members {
                for n in nodes.iter() {
                    for o in &n.origins {
                        pop.add(o, name);
                    }
                }
            }
            let names: Vec<&str> = members.iter().map(|(n, _)| *n).collect();
            let leaf_only = members
                .iter()
                .all(|(_, nodes)| nodes.iter().all(|n| n.children.values().all(Node::is_leaf)));
            let comment = if leaf_only {
                format!(
                    "inferred {{var}} from siblings: {} — a literal pair with one shape looks \
                     the same; review",
                    examples(&names)
                )
            } else {
                format!(
                    "inferred {{var}} from siblings: {} ({} values on {} origin(s))",
                    examples(&names),
                    pop.all().len(),
                    pop.members.len()
                )
            };
            let mut chunks = prefix.to_vec();
            chunks.push(Chunk::Var(pop));
            let merged: Vec<&Node> = members
                .iter()
                .flat_map(|(_, n)| n.iter().copied())
                .collect();
            for mut f in infer_view(&View { nodes: merged }, &chunks, keys) {
                f.comments.insert(0, comment.clone());
                out.push(f);
            }
        } else {
            let (name, nodes) = &members[0];
            let mut chunks = prefix.to_vec();
            chunks.push(Chunk::Literal((*name).to_string()));
            out.extend(infer_view(
                &View {
                    nodes: (*nodes).clone(),
                },
                &chunks,
                keys,
            ));
        }
    }
    out
}

/// The E1 test: the merged subtree infers to patterns every member serves
/// in full, and the members share at least [`ANCHOR_MIN`] of their tails.
fn merge_accepted(
    trial: &[&Node],
    per_member: &[Vec<&Node>],
    prefix: &[Chunk],
    keys: &[KeyObs],
) -> bool {
    let tail_sets: Vec<BTreeSet<&str>> = per_member
        .iter()
        .map(|nodes| {
            View {
                nodes: nodes.clone(),
            }
            .tails()
        })
        .collect();
    let mut anchor = tail_sets[0].clone();
    for s in &tail_sets[1..] {
        anchor = anchor.intersection(s).copied().collect();
    }
    let smallest = tail_sets.iter().map(BTreeSet::len).min().unwrap_or(0);
    if smallest == 0 || (anchor.len() as f64) < ANCHOR_MIN * smallest as f64 {
        return false;
    }
    let mut var_prefix = prefix.to_vec();
    var_prefix.push(Chunk::Var(VarPop::default()));
    let families = infer_view(
        &View {
            nodes: trial.to_vec(),
        },
        &var_prefix,
        keys,
    );
    let depth = var_prefix.len();
    // Relative patterns below the var position.
    let relative: Vec<Family> = families
        .iter()
        .map(|f| Family {
            chunks: f.chunks[depth..].to_vec(),
            members: Vec::new(),
            comments: Vec::new(),
        })
        .collect();
    tail_sets.iter().all(|tails| {
        let split: Vec<Vec<&str>> = tails.iter().map(|t| split_tail(t)).collect();
        split
            .iter()
            .all(|tail| relative.iter().any(|f| f.matches(tail)))
            && relative
                .iter()
                .all(|f| split.iter().any(|tail| f.matches(tail)))
    })
}

fn split_tail(t: &str) -> Vec<&str> {
    if t.is_empty() {
        Vec::new()
    } else {
        t.split('/').collect()
    }
}

fn collect_rest(node: &Node, path: &str, pop: &mut VarPop, members: &mut Vec<usize>) {
    if !node.terminal.is_empty() && !path.is_empty() {
        for o in &node.origins {
            pop.add(o, path);
        }
        members.extend(node.terminal.iter().copied());
    }
    for (name, child) in &node.children {
        let p = if path.is_empty() {
            name.clone()
        } else {
            format!("{path}/{name}")
        };
        collect_rest(child, &p, pop, members);
    }
}

fn examples(names: &[&str]) -> String {
    let shown: Vec<&str> = names.iter().copied().take(SIBLING_EXAMPLES).collect();
    if names.len() > SIBLING_EXAMPLES {
        format!("{}, … ({} in all)", shown.join(", "), names.len())
    } else {
        shown.join(", ")
    }
}

// ─── the inference ──────────────────────────────────────────────────────────

/// Draft the registry from the observation. `hints` is a loaded slice set
/// when one was available: it names `{var}`s where a declared pattern binds
/// with the same positions, and resolves a service origin's name; the draft
/// comes out without one, and says so.
pub fn infer(
    obs: &InferObservation,
    source: &str,
    window_s: Option<f64>,
    hints: Option<&SliceSet>,
) -> InferReport {
    let keys: Vec<(&String, &KeyObs)> = obs.keys.iter().collect();
    let key_obs: Vec<KeyObs> = keys.iter().map(|(_, k)| (*k).clone()).collect();
    let key_names: Vec<&str> = keys.iter().map(|(k, _)| k.as_str()).collect();
    let span_s = obs.span_s();
    let origins = obs.origins().len();

    // One trie per (owner, class).
    let mut tries: BTreeMap<(Owner, String), Node> = BTreeMap::new();
    for (i, k) in key_obs.iter().enumerate() {
        tries
            .entry((k.owner.clone(), k.class.clone()))
            .or_default()
            .insert(&k.tail, &k.origin, i);
    }

    let mut producers: BTreeMap<Owner, InferredProducer> = BTreeMap::new();
    let mut type_shapes: BTreeMap<Owner, Vec<(String, Option<Value>, usize)>> = BTreeMap::new();
    let mut caveats = Vec::new();
    if origins <= 1 {
        caveats.push(
            "1 origin observed: a leaf dimension cannot be told from a set of measurements from one \
             host, so every leaf below is a literal (failure mode 1)"
                .to_string(),
        );
    }
    if hints.is_none() {
        caveats.push("no registry hinted the inference: {var}s are named by position".to_string());
    }

    for ((owner, class), root) in &tries {
        let families = infer_view(&View { nodes: vec![root] }, &[], &key_obs);
        let hint_slice = hints.and_then(|h| match owner {
            Owner::Producer(p) => h.get(p),
            Owner::Service(o) => h.by_service_origin(o),
        });
        let name = match (owner, hint_slice) {
            (Owner::Service(_), Some(s)) => s.name.clone(),
            _ => owner.file_name(),
        };
        let producer = producers
            .entry(owner.clone())
            .or_insert_with(|| InferredProducer {
                name: name.clone(),
                service_origin: match owner {
                    Owner::Service(o) => Some(o.clone()),
                    Owner::Producer(_) => None,
                },
                subjects: Vec::new(),
                types: Vec::new(),
            });
        let shapes = type_shapes.entry(owner.clone()).or_default();
        for fam in families {
            let subject = draft_subject(
                &fam, class, &key_obs, &key_names, obs, span_s, hint_slice, &name, shapes,
            );
            producer.subjects.push(subject);
        }
    }

    let mut out: Vec<InferredProducer> = Vec::new();
    for (owner, mut producer) in producers {
        let shapes = type_shapes.remove(&owner).unwrap_or_default();
        producer.types = shapes
            .into_iter()
            .map(|(name, schema, subjects)| InferredType {
                name,
                schema,
                subjects,
            })
            .collect();
        producer
            .subjects
            .sort_by(|a, b| (&a.class, &a.path).cmp(&(&b.class, &b.path)));
        assign_variants(&mut producer.subjects);
        out.push(producer);
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));

    InferReport {
        source: source.to_string(),
        window_s,
        span_s,
        samples: obs.samples,
        dropped: 0,
        keys_seen: obs.keys.len(),
        keys_refused: obs.keys_refused,
        origins,
        unparsed_keys: obs.unparsed,
        off_plane_keys: obs.off_plane,
        undocumented: obs.fields.undocumented(),
        unread: obs.fields.unread(),
        paths_refused: obs.fields.dropped_paths(),
        hinted_by_registry: hints.is_some(),
        caveats,
        producers: out,
    }
}

/// The generated-variant names `zenkey-build` would derive — the literal
/// chunks CamelCased, or every chunk when there is no literal — and an
/// override where two paths collide, so the draft lints as written.
fn assign_variants(subjects: &mut [InferredSubject]) {
    fn chunks(path: &str) -> Vec<(bool, String)> {
        path.split('/')
            .map(
                |c| match c.strip_prefix('{').and_then(|c| c.strip_suffix('}')) {
                    Some(v) => (true, v.trim_end_matches("...").to_string()),
                    None => (false, c.to_string()),
                },
            )
            .collect()
    }
    fn default_variant(path: &str) -> String {
        let chunks = chunks(path);
        let literals: String = chunks
            .iter()
            .filter(|(var, _)| !var)
            .map(|(_, c)| camel(c))
            .collect();
        if literals.is_empty() {
            chunks.iter().map(|(_, c)| camel(c)).collect()
        } else {
            literals
        }
    }
    let mut by_variant: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, s) in subjects.iter().enumerate() {
        by_variant
            .entry(default_variant(&s.path))
            .or_default()
            .push(i);
    }
    let mut taken: BTreeSet<String> = by_variant.keys().cloned().collect();
    for (_, members) in by_variant {
        if members.len() < 2 {
            continue;
        }
        // The literal-only path keeps the default; each var-bearing path is
        // named with its vars, then its class if that still collides.
        for &i in &members {
            let s = &subjects[i];
            if !s.path.contains('{') {
                continue;
            }
            let full: String = chunks(&s.path).iter().map(|(_, c)| camel(c)).collect();
            let name = if taken.contains(&full) {
                format!("{full}{}", camel(&s.class))
            } else {
                full
            };
            taken.insert(name.clone());
            subjects[i].variant = Some(name);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draft_subject(
    fam: &Family,
    class: &str,
    keys: &[KeyObs],
    key_names: &[&str],
    obs: &InferObservation,
    span_s: f64,
    hint: Option<&zenkey::RegistrySlice>,
    producer: &str,
    shapes: &mut Vec<(String, Option<Value>, usize)>,
) -> InferredSubject {
    let members: Vec<&KeyObs> = fam.members.iter().map(|&i| &keys[i]).collect();
    let mut comments = fam.comments.clone();

    // ── the path ──────────────────────────────────────────────────────────
    let var_names = hint_var_names(fam, class, hint, &members);
    let mut parts = Vec::new();
    let mut var_i = 0usize;
    for (depth, c) in fam.chunks.iter().enumerate() {
        match c {
            Chunk::Literal(l) => parts.push(l.clone()),
            Chunk::Var(_) | Chunk::Rest(_) => {
                let name = var_names
                    .as_ref()
                    .and_then(|v| v.get(var_i).cloned())
                    .unwrap_or_else(|| format!("v{depth}"));
                var_i += 1;
                parts.push(match c {
                    Chunk::Rest(_) => format!("{{{name}...}}"),
                    _ => format!("{{{name}}}"),
                });
            }
        }
    }
    let path = parts.join("/");

    // ── cardinality ───────────────────────────────────────────────────────
    let cardinality = fam
        .chunks
        .iter()
        .filter_map(|c| match c {
            Chunk::Var(p) | Chunk::Rest(p) => Some(p.max_per_origin()),
            Chunk::Literal(_) => None,
        })
        .max()
        .map(|n| round_up_pow10(n as u64) as i64);
    if fam
        .chunks
        .iter()
        .any(|c| matches!(c, Chunk::Var(_) | Chunk::Rest(_)))
    {
        comments.push(
            "cardinality: the largest distinct member count one origin published, rounded up to a \
             power of ten"
                .to_string(),
        );
    }

    // ── counts ────────────────────────────────────────────────────────────
    let samples: u64 = members.iter().map(|k| k.samples).sum();
    let origins: BTreeSet<&str> = members.iter().map(|k| k.origin.as_str()).collect();

    // ── rate (events) ─────────────────────────────────────────────────────
    let mut rate = None;
    if class == "events" {
        let mut per_origin: BTreeMap<&str, u64> = BTreeMap::new();
        for k in &members {
            *per_origin.entry(k.origin.as_str()).or_default() += k.samples;
        }
        let busiest = per_origin.values().copied().max().unwrap_or(0);
        if samples < 2 || span_s <= 0.0 {
            rate = Some("rare".to_string());
            comments.push(format!(
                "rate: 0–1 observed in {span_s:.1} s; rate class unestablished — `rare` is the \
                 tightest class consistent with that, not a measurement"
            ));
        } else {
            let per_hour = busiest as f64 * 3600.0 / span_s;
            rate = Some(if per_hour <= 1.0 {
                "rare".to_string()
            } else if per_hour <= 60.0 {
                "low".to_string()
            } else {
                format!("burst({}/h)", round_up_pow10(per_hour.ceil() as u64))
            });
            comments.push(format!(
                "rate: {busiest} event(s) from the busiest origin over {span_s:.1} s"
            ));
        }
    }

    // ── ttl_s (state) ─────────────────────────────────────────────────────
    let mut ttl_s = None;
    if class == "state" {
        let longest_median = members
            .iter()
            .filter_map(|k| median(&k.intervals))
            .fold(None, |acc: Option<f64>, m| {
                Some(acc.map_or(m, |a| a.max(m)))
            });
        match longest_median {
            Some(m) if m > 0.0 => {
                ttl_s = Some((2.0 * m).ceil() as i64);
                comments.push(format!(
                    "ttl_s: a hint — twice the longest median inter-arrival ({m:.1} s) among {} \
                     expansion(s)",
                    members.len()
                ));
            }
            _ => comments.push(format!(
                "ttl_s not established: no refresh observed within {span_s:.1} s; the lint requires \
                 one — set it from the producer's cadence"
            )),
        }
    }

    // ── unit / kind (RFC 08 §4) ───────────────────────────────────────────
    let mut unit = None;
    if let Some(Chunk::Literal(leaf)) = fam.chunks.last() {
        unit = unit_of(leaf).map(str::to_string);
        if leaf.ends_with("_total") {
            comments.push("kind guess: counter (the `_total` suffix, RFC 08 §4)".to_string());
        }
    }
    if let Some(guess) = kind_guess(&members) {
        comments.push(guess);
    }

    // ── encoding ──────────────────────────────────────────────────────────
    let mut encodings: BTreeMap<&str, u64> = BTreeMap::new();
    for k in &members {
        for (e, n) in &k.encodings {
            *encodings.entry(e.as_str()).or_default() += n;
        }
    }
    let encoding = match encodings.len() {
        1 => encodings.keys().next().map(|e| e.to_string()),
        0 => None,
        _ => {
            comments.push(format!(
                "encodings observed: {} — no single encoding to declare",
                encodings
                    .iter()
                    .map(|(e, n)| format!("{e} ×{n}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            None
        }
    };

    // ── qos: observed is a comment, never a field; the alert family's
    // normative profile is a field, because RFC 08 §5 (v1.23) requires the
    // declaration and the draft would not lint without it ────────────────
    let alert_family =
        class == "state" && matches!(fam.chunks.first(), Some(Chunk::Literal(l)) if l == "alert");
    let qos_field = alert_family.then(|| "alert".to_string());
    if alert_family {
        comments.push(
            "qos = \"alert\" is what RFC 08 §5 requires of the alert family (v1.23), not what was \
             observed — see the observed line below"
                .to_string(),
        );
    }
    let mut qos: BTreeMap<&str, u64> = BTreeMap::new();
    for k in &members {
        for (q, n) in &k.qos {
            *qos.entry(q.as_str()).or_default() += n;
        }
    }
    if !qos.is_empty() {
        let listed = qos
            .iter()
            .map(|(q, n)| {
                if qos.len() > 1 {
                    format!("{q} ×{n}")
                } else {
                    q.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        comments.push(format!(
            "qos observed: {listed} — what rode, not a declaration; a wrong profile on the wire \
             would be laundered into a contract by writing it here"
        ));
    }

    // ── schema / type ─────────────────────────────────────────────────────
    let (schema, undocumented, documents) = pooled_schema(fam, key_names, obs);
    if documents == 0 {
        comments.push(format!(
            "payload not structural in {undocumented} of {samples} samples; no schema inferred"
        ));
    } else if undocumented > 0 {
        comments.push(format!(
            "payload not structural in {undocumented} of {samples} samples; the schema covers the \
             {documents} that were"
        ));
    }
    let literals: String = fam
        .chunks
        .iter()
        .filter_map(|c| match c {
            Chunk::Literal(l) => Some(camel(l)),
            _ => None,
        })
        .collect();
    let type_name = {
        let canonical = schema
            .as_ref()
            .map(|s| serde_json::to_string(s).unwrap_or_default());
        match shapes.iter_mut().find(|(_, s, _)| {
            s.as_ref()
                .map(|v| serde_json::to_string(v).unwrap_or_default())
                == canonical
        }) {
            Some(existing) => {
                existing.2 += 1;
                existing.0.clone()
            }
            None => {
                let mut name = format!("{}{literals}", camel(producer));
                if schema.is_none() {
                    name.push_str("Opaque");
                }
                let mut candidate = name.clone();
                let mut n = 2;
                while shapes.iter().any(|(existing, _, _)| *existing == candidate) {
                    candidate = format!("{name}{n}");
                    n += 1;
                }
                shapes.push((candidate.clone(), schema, 1));
                candidate
            }
        }
    };

    InferredSubject {
        path,
        class: class.to_string(),
        type_name,
        variant: None,
        unit,
        qos: qos_field,
        ttl_s,
        rate,
        cardinality,
        encoding,
        comments,
        origins: origins.len(),
        keys: members.len(),
        samples,
    }
}

/// A hint slice's var names, when one of its declared patterns of this
/// class binds one member tail with the same var positions.
fn hint_var_names(
    fam: &Family,
    class: &str,
    hint: Option<&zenkey::RegistrySlice>,
    members: &[&KeyObs],
) -> Option<Vec<String>> {
    let hint = hint?;
    let sample = members.first()?;
    let tail: Vec<&str> = sample.tail.iter().map(String::as_str).collect();
    for d in &hint.subjects {
        if d.class.token() != class {
            continue;
        }
        let Ok(p) = SubjectPattern::parse(&d.path) else {
            continue;
        };
        if p.matches(&tail).is_none() || p.chunks().len() != fam.chunks.len() {
            continue;
        }
        let same_positions = p.chunks().iter().zip(&fam.chunks).all(|(h, c)| {
            matches!(
                (h, c),
                (PatternChunk::Literal(_), Chunk::Literal(_))
                    | (PatternChunk::Var(_), Chunk::Var(_))
                    | (PatternChunk::Rest(_), Chunk::Rest(_))
            )
        });
        if same_positions {
            return Some(
                p.chunks()
                    .iter()
                    .filter_map(|c| match c {
                        PatternChunk::Var(v) | PatternChunk::Rest(v) => Some(v.clone()),
                        PatternChunk::Literal(_) => None,
                    })
                    .collect(),
            );
        }
    }
    None
}

fn kind_guess(members: &[&KeyObs]) -> Option<String> {
    let numeric: u64 = members.iter().map(|k| k.numeric).sum();
    let text: u64 = members.iter().map(|k| k.text).sum();
    let boolean: u64 = members.iter().map(|k| k.boolean).sum();
    let total = numeric + text + boolean;
    if total == 0 {
        return None;
    }
    if numeric == total {
        let decreased = members.iter().any(|k| k.decreased);
        let negative = members.iter().any(|k| k.negative);
        let enough = members.iter().any(|k| k.numeric >= 2);
        return Some(if decreased || negative {
            format!(
                "kind guess: gauge ({numeric} numeric sample(s); a decrease or a negative was seen)"
            )
        } else if enough {
            format!(
                "kind guess: counter or gauge — never decreased over {numeric} numeric sample(s) on {} \
                 origin(s); a gauge that only rose looks the same",
                members
                    .iter()
                    .map(|k| k.origin.as_str())
                    .collect::<BTreeSet<_>>()
                    .len()
            )
        } else {
            format!("kind guess: numeric ({numeric} sample(s), too few to see a direction)")
        });
    }
    if text == total {
        return Some(format!("kind guess: text ({text} string sample(s))"));
    }
    if boolean == total {
        return Some(format!("kind guess: bool ({boolean} boolean sample(s))"));
    }
    Some(format!(
        "kind: mixed — {numeric} numeric, {text} text, {boolean} bool sample(s); no single kind"
    ))
}

/// RFC 08 §4's suffix rule — normative for the suffix, so these are the
/// only spellings; anything else stays absent.
fn unit_of(leaf: &str) -> Option<&'static str> {
    const SUFFIXES: [(&str, &str); 6] = [
        ("_ms", "ms"),
        ("_us", "us"),
        ("_s", "s"),
        ("_bytes", "bytes"),
        ("_percent", "percent"),
        ("_ratio", "ratio"),
    ];
    SUFFIXES
        .iter()
        .find(|(suffix, _)| leaf.ends_with(suffix))
        .map(|(_, unit)| *unit)
}

fn median(intervals: &[f64]) -> Option<f64> {
    if intervals.is_empty() {
        return None;
    }
    let mut v = intervals.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = v.len() / 2;
    Some(if v.len().is_multiple_of(2) {
        (v[mid - 1] + v[mid]) / 2.0
    } else {
        v[mid]
    })
}

fn round_up_pow10(n: u64) -> u64 {
    let mut p = 1u64;
    while p < n {
        p = p.saturating_mul(10);
    }
    p
}

fn camel(chunk: &str) -> String {
    chunk
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| {
            let mut cs = s.chars();
            match cs.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + cs.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

// ─── schema inference ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
struct PooledPath {
    seen: u64,
    kinds: BTreeMap<&'static str, u64>,
    all_integral: bool,
}

/// Pool the members' path statistics and build a schema. Returns the
/// schema (`None` when no member carried a document), the undocumented
/// sample count and the document count.
fn pooled_schema(
    fam: &Family,
    key_names: &[&str],
    obs: &InferObservation,
) -> (Option<Value>, u64, u64) {
    let mut paths: BTreeMap<String, PooledPath> = BTreeMap::new();
    let mut documents = 0u64;
    let mut undocumented = 0u64;
    let by_key: BTreeMap<&str, &crate::judge::field::KeyFields> = obs.fields.iter().collect();
    for &i in &fam.members {
        let Some(fields) = by_key.get(key_names[i]) else {
            continue;
        };
        documents += fields.documents;
        undocumented += fields.undocumented + fields.unread;
        for (path, stats) in &fields.paths {
            let p = paths.entry(path.clone()).or_insert_with(|| PooledPath {
                all_integral: true,
                ..PooledPath::default()
            });
            pool(p, stats);
        }
    }
    if documents == 0 {
        return (None, undocumented, 0);
    }
    let truncated = obs.fields.dropped_paths() > 0;
    (
        Some(schema_of(&paths, documents, truncated)),
        undocumented,
        documents,
    )
}

fn pool(p: &mut PooledPath, stats: &PathStats) {
    p.seen += stats.seen;
    for (k, n) in &stats.kinds {
        *p.kinds.entry(k).or_default() += n;
    }
    p.all_integral &= stats.all_integral;
}

#[derive(Debug, Default)]
struct SchemaNode {
    children: BTreeMap<String, SchemaNode>,
    leaf: Option<PooledPath>,
}

fn schema_of(paths: &BTreeMap<String, PooledPath>, documents: u64, truncated: bool) -> Value {
    if let Some(root) = paths.get(ROOT_PATH) {
        return scalar_schema(root);
    }
    let mut tree = SchemaNode::default();
    for (path, p) in paths {
        let mut node = &mut tree;
        for part in path.split('.') {
            node = node.children.entry(part.to_string()).or_default();
        }
        node.leaf = Some(p.clone());
    }
    let mut schema = object_schema(&tree, documents);
    if truncated && let Value::Object(m) = &mut schema {
        m.insert("additionalProperties".into(), Value::Bool(true));
    }
    schema
}

fn object_schema(node: &SchemaNode, presence: u64) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for (name, child) in &node.children {
        let schema = if child.children.is_empty() {
            child
                .leaf
                .as_ref()
                .map(scalar_schema)
                .unwrap_or(Value::Object(Default::default()))
        } else {
            // Present in every document iff some descendant leaf was.
            let child_presence = child
                .leaf
                .as_ref()
                .map_or(0, |l| l.seen)
                .max(max_seen(child));
            let mut s = object_schema(child, child_presence);
            if let Some(leaf) = &child.leaf
                && let Value::Object(m) = &mut s
                && leaf.kinds.keys().any(|k| *k != "object")
            {
                // Seen both as a scalar and as an object: say both.
                let mut types: Vec<&str> = leaf.kinds.keys().map(|k| json_type(k, leaf)).collect();
                types.push("object");
                types.sort_unstable();
                types.dedup();
                m.insert("type".into(), types.into_iter().map(Value::from).collect());
            }
            s
        };
        let seen = child
            .leaf
            .as_ref()
            .map_or(0, |l| l.seen)
            .max(max_seen(child));
        if presence > 0 && seen >= presence {
            required.push(Value::from(name.as_str()));
        }
        properties.insert(name.clone(), schema);
    }
    let mut m = serde_json::Map::new();
    m.insert("type".into(), Value::from("object"));
    m.insert("properties".into(), Value::Object(properties));
    if !required.is_empty() {
        m.insert("required".into(), Value::Array(required));
    }
    Value::Object(m)
}

fn max_seen(node: &SchemaNode) -> u64 {
    node.children
        .values()
        .map(|c| c.leaf.as_ref().map_or(0, |l| l.seen).max(max_seen(c)))
        .max()
        .unwrap_or(0)
}

fn json_type(kind: &str, p: &PooledPath) -> &'static str {
    match kind {
        "number" => {
            if p.all_integral {
                "integer"
            } else {
                "number"
            }
        }
        "bool" => "boolean",
        "string" => "string",
        "null" => "null",
        "array" => "array",
        _ => "object",
    }
}

fn scalar_schema(p: &PooledPath) -> Value {
    let mut types: Vec<&str> = p.kinds.keys().map(|k| json_type(k, p)).collect();
    types.sort_unstable();
    types.dedup();
    let mut m = serde_json::Map::new();
    match types.as_slice() {
        [one] => {
            m.insert("type".into(), Value::from(*one));
        }
        many => {
            m.insert(
                "type".into(),
                many.iter().map(|t| Value::from(*t)).collect(),
            );
        }
    }
    if types.contains(&"array") {
        m.insert(
            "$comment".into(),
            Value::from("items not inferred: arrays are leaves to the field walk"),
        );
    }
    Value::Object(m)
}

// ─── the draft emitter ──────────────────────────────────────────────────────

/// Where a draft came from — the header every file states.
#[derive(Debug, Clone)]
pub struct Provenance {
    /// `--app`, or `"unknown"`.
    pub app: String,
    /// The selector or capture.
    pub source: String,
    /// The span the observation covered, seconds.
    pub span_s: f64,
    /// RFC 3339, when the draft was written.
    pub at: String,
    pub keys: usize,
    pub samples: u64,
    pub dropped: u64,
}

/// One producer's draft file (RFC 08 §6.1). Its own emitter, not
/// [`zenkey::slice_to_toml`]: a slice cannot carry `draft = true` or the
/// per-entry comments, and those are the point.
pub fn to_draft_toml(producer: &InferredProducer, prov: &Provenance) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# DRAFT — inferred from {} over {:.1} s on {}, {} keys, {} samples, dropped {};\n\
         # observation-derived and unreviewed (RFC 08 §6.1). Every field below is a guess a\n\
         # human has not confirmed. `draft = true` makes zenkey-build refuse this file until\n\
         # a review removes the marker and assigns `since`; `registry lint --allow-drafts`\n\
         # checks it meanwhile.\n\n",
        prov.source, prov.span_s, prov.at, prov.keys, prov.samples, prov.dropped
    ));
    out.push_str("[registry]\nversion = \"0.1\"\n");
    out.push_str(&format!("app = {}\n", toml_quote(&prov.app)));
    out.push_str("convention = 1\ncompat = \"none\"\ndraft = true\n");
    match &producer.service_origin {
        Some(origin) => out.push_str(&format!(
            "\n[service]\nname = {}\norigin = {}\n",
            toml_quote(&producer.name),
            toml_quote(origin)
        )),
        None => out.push_str(&format!(
            "\n[producer]\nname = {}\n",
            toml_quote(&producer.name)
        )),
    }
    for s in &producer.subjects {
        out.push('\n');
        for c in &s.comments {
            out.push_str(&format!("# {c}\n"));
        }
        out.push_str(&format!(
            "# observed: {} key(s) on {} origin(s), {} sample(s)\n",
            s.keys, s.origins, s.samples
        ));
        out.push_str("[[subject]]\n");
        out.push_str(&format!("path = {}\n", toml_quote(&s.path)));
        out.push_str(&format!("class = {}\n", toml_quote(&s.class)));
        out.push_str(&format!("type = {}\n", toml_quote(&s.type_name)));
        if let Some(v) = &s.variant {
            out.push_str(&format!("variant = {}\n", toml_quote(v)));
        }
        if let Some(u) = &s.unit {
            out.push_str(&format!("unit = {}\n", toml_quote(u)));
        }
        if let Some(q) = &s.qos {
            out.push_str(&format!("qos = {}\n", toml_quote(q)));
        }
        if let Some(t) = s.ttl_s {
            out.push_str(&format!("ttl_s = {t}\n"));
        }
        if let Some(r) = &s.rate {
            out.push_str(&format!("rate = {}\n", toml_quote(r)));
        }
        if let Some(c) = s.cardinality {
            out.push_str(&format!("cardinality = {c}\n"));
        }
        if let Some(e) = &s.encoding {
            out.push_str(&format!("encoding = {}\n", toml_quote(e)));
        }
        out.push_str("description = \"inferred; unreviewed\"\n");
    }
    out
}

/// The `types.toml` for a draft directory: one row per inferred type,
/// `kind = "json-schema"` with the sidecar's path when a schema was
/// inferred, `kind = "unknown"` when no sample was structural. The
/// `schema` key is informational — the lint reads `kind` alone.
pub fn to_draft_types_toml(producers: &[InferredProducer], prov: &Provenance) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# DRAFT type table — inferred from {} on {} (RFC 08 §5, §6.1); every schema here is\n\
         # the shape of what was observed, not a declaration. Review beside the producer files.\n",
        prov.source, prov.at
    ));
    for p in producers {
        for t in &p.types {
            out.push('\n');
            out.push_str(&format!(
                "# {}: bound by {} subject(s)\n",
                p.name, t.subjects
            ));
            out.push_str(&format!("[types.{}]\n", t.name));
            match &t.schema {
                Some(_) => {
                    out.push_str("kind = \"json-schema\"\n");
                    out.push_str(&format!(
                        "schema = {}\n",
                        toml_quote(&format!("schemas/{}.json", t.name))
                    ));
                }
                None => out
                    .push_str("kind = \"unknown\"  # no structural sample; nothing to describe\n"),
            }
        }
    }
    out
}

/// The sidecar schema documents: `(relative path, contents)` per inferred
/// type that has one.
pub fn draft_schema_files(producers: &[InferredProducer]) -> Vec<(String, String)> {
    producers
        .iter()
        .flat_map(|p| p.types.iter())
        .filter_map(|t| {
            t.schema.as_ref().map(|s| {
                let mut doc = serde_json::Map::new();
                doc.insert(
                    "$schema".into(),
                    Value::from("https://json-schema.org/draft/2020-12/schema"),
                );
                doc.insert("title".into(), Value::from(t.name.as_str()));
                doc.insert(
                    "$comment".into(),
                    Value::from("DRAFT — inferred from observed samples (RFC 08 §6.1); unreviewed"),
                );
                if let Value::Object(m) = s {
                    for (k, v) in m {
                        doc.insert(k.clone(), v.clone());
                    }
                }
                (
                    format!("schemas/{}.json", t.name),
                    serde_json::to_string_pretty(&Value::Object(doc)).unwrap_or_default() + "\n",
                )
            })
        })
        .collect()
}

/// Every relative path a draft run would write — checked before anything
/// is written, so an occupied output directory refuses whole.
pub fn draft_file_names(report: &InferReport) -> Vec<String> {
    let mut names: Vec<String> = report
        .producers
        .iter()
        .map(|p| format!("{}.toml", p.name))
        .collect();
    names.push("types.toml".to_string());
    names.extend(
        draft_schema_files(&report.producers)
            .into_iter()
            .map(|(p, _)| p),
    );
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(
        obs: &mut InferObservation,
        origin: &str,
        class: &str,
        tail: &str,
        at: f64,
        doc: Option<&Value>,
    ) {
        obs.observe(
            &format!("v1/{origin}/{class}/demo/{tail}"),
            at,
            Some("application/json"),
            Some("sampled"),
            doc,
        );
    }

    const A: &str = "h-aaaaaaaaaaaa";
    const B: &str = "h-bbbbbbbbbbbb";

    fn paths(report: &InferReport) -> Vec<(String, String)> {
        report
            .producers
            .iter()
            .flat_map(|p| p.subjects.iter().map(|s| (s.class.clone(), s.path.clone())))
            .collect()
    }

    /// E1: interior siblings with one shape merge; E2: leaf siblings whose
    /// membership varies merge; leaf siblings every origin publishes stay
    /// literal.
    #[test]
    fn interior_and_leaf_evidence_split_the_three_cases() {
        let mut obs = InferObservation::new("", 100, 100);
        let n = serde_json::json!({"value": 1});
        for (i, o) in [A, B].iter().enumerate() {
            let core = format!("core{i}");
            feed(
                &mut obs,
                o,
                "telemetry",
                &format!("cpu/{core}/usage"),
                0.0,
                Some(&n),
            );
            feed(
                &mut obs,
                o,
                "telemetry",
                &format!("cpu/{core}/usage"),
                1.0,
                Some(&n),
            );
            feed(&mut obs, o, "telemetry", "cpu/usage", 0.0, Some(&n));
            feed(&mut obs, o, "telemetry", "memory/total", 0.0, Some(&n));
            feed(&mut obs, o, "telemetry", "memory/used", 0.0, Some(&n));
            feed(
                &mut obs,
                o,
                "telemetry",
                &format!("disk/mnt{i}/used_bytes"),
                0.0,
                Some(&n),
            );
        }
        let r = infer(&obs, "v1/**", Some(10.0), None);
        let mut got = paths(&r);
        got.sort();
        assert_eq!(
            got,
            vec![
                ("telemetry".into(), "cpu/usage".into()),
                ("telemetry".into(), "cpu/{v1}/usage".into()),
                ("telemetry".into(), "disk/{v1}/used_bytes".into()),
                ("telemetry".into(), "memory/total".into()),
                ("telemetry".into(), "memory/used".into()),
            ],
            "{r:#?}"
        );
        let disk = r.producers[0]
            .subjects
            .iter()
            .find(|s| s.path == "disk/{v1}/used_bytes")
            .unwrap();
        assert_eq!(disk.unit.as_deref(), Some("bytes"));
        // `cpu/usage` and `cpu/{v1}/usage` would collide on `CpuUsage`; the
        // var-bearing one is overridden so the draft lints.
        let core = r.producers[0]
            .subjects
            .iter()
            .find(|s| s.path == "cpu/{v1}/usage")
            .unwrap();
        assert_eq!(core.variant.as_deref(), Some("CpuV1Usage"));
        assert!(
            r.producers[0]
                .subjects
                .iter()
                .find(|s| s.path == "cpu/usage")
                .unwrap()
                .variant
                .is_none()
        );
        assert_eq!(disk.cardinality, Some(1));
        assert_eq!(disk.encoding.as_deref(), Some("application/json"));
        assert!(
            disk.comments
                .iter()
                .any(|c| c.starts_with("qos observed: sampled"))
        );
        assert!(disk.ttl_s.is_none() && disk.rate.is_none());
    }

    /// Failure mode 1, as a fact: one origin, one value — a literal.
    #[test]
    fn one_expansion_cannot_prove_a_dimension() {
        let mut obs = InferObservation::new("", 100, 100);
        feed(&mut obs, A, "telemetry", "cpu/core0/usage", 0.0, None);
        feed(&mut obs, A, "telemetry", "tcp/established", 0.0, None);
        feed(&mut obs, A, "telemetry", "tcp/listen", 0.0, None);
        let r = infer(&obs, "v1/**", None, None);
        let got = paths(&r);
        assert!(
            got.contains(&("telemetry".into(), "cpu/core0/usage".into())),
            "{got:?}"
        );
        assert!(
            got.contains(&("telemetry".into(), "tcp/listen".into())),
            "{got:?}"
        );
        assert!(
            r.caveats.iter().any(|c| c.contains("1 origin")),
            "{:?}",
            r.caveats
        );
    }

    /// Failure mode 2, as a fact: two literal measurements with one shape
    /// come out as a `{var}`, and the entry says which siblings it merged.
    #[test]
    fn a_literal_pair_with_one_shape_is_a_var_with_its_siblings_named() {
        let mut obs = InferObservation::new("", 100, 100);
        for o in [A, B] {
            feed(&mut obs, o, "telemetry", "cpu/usage", 0.0, None);
            feed(&mut obs, o, "telemetry", "mem/usage", 0.0, None);
        }
        let r = infer(&obs, "v1/**", None, None);
        let s = &r.producers[0].subjects;
        assert_eq!(s.len(), 1, "{s:#?}");
        assert_eq!(s[0].path, "{v0}/usage");
        assert!(
            s[0].comments
                .iter()
                .any(|c| c.contains("siblings: cpu, mem") && c.contains("literal pair")),
            "{:?}",
            s[0].comments
        );
    }

    /// E3 and the rate class: write-once events keys from one origin are
    /// one `{var}`, and the class follows the count over the span.
    #[test]
    fn events_keys_are_write_once_and_rated() {
        let mut obs = InferObservation::new("", 100, 100);
        for i in 0..5 {
            feed(
                &mut obs,
                A,
                "events",
                &format!("boom/id{i}"),
                i as f64 * 10.0,
                None,
            );
        }
        let r = infer(&obs, "v1/**", Some(40.0), None);
        let s = &r.producers[0].subjects;
        assert_eq!(s.len(), 1, "{s:#?}");
        assert_eq!(s[0].path, "boom/{v1}");
        // 5 in 40 s = 450/h → burst(1000/h).
        assert_eq!(s[0].rate.as_deref(), Some("burst(1000/h)"));
        assert_eq!(s[0].cardinality, Some(10));
    }

    /// State: ttl_s is twice the median inter-arrival, or absent with the
    /// reason when nothing refreshed.
    #[test]
    fn state_ttl_is_a_refresh_hint_or_absent() {
        let mut obs = InferObservation::new("", 100, 100);
        let ok = serde_json::json!({"ok": true, "load": 0.5});
        for t in [0.0, 5.0, 10.0, 15.0] {
            feed(&mut obs, A, "state", "health", t, Some(&ok));
        }
        feed(&mut obs, A, "state", "sensor", 0.0, Some(&ok));
        let r = infer(&obs, "v1/**", Some(20.0), None);
        let health = r.producers[0]
            .subjects
            .iter()
            .find(|s| s.path == "health")
            .unwrap();
        assert_eq!(health.ttl_s, Some(10));
        let sensor = r.producers[0]
            .subjects
            .iter()
            .find(|s| s.path == "sensor")
            .unwrap();
        assert_eq!(sensor.ttl_s, None);
        assert!(
            sensor
                .comments
                .iter()
                .any(|c| c.contains("ttl_s not established"))
        );
        // The schema: an object with both fields required, one boolean, one number.
        let t = r.producers[0]
            .types
            .iter()
            .find(|t| t.name == "DemoHealth")
            .unwrap();
        assert_eq!(
            t.schema,
            Some(serde_json::json!({
                "type": "object",
                "properties": {"ok": {"type": "boolean"}, "load": {"type": "number"}},
                "required": ["load", "ok"],
            }))
        );
        assert_eq!(t.subjects, 2, "sensor shares the shape and the type");
    }

    /// Integral numbers become `integer`; a field absent from some document
    /// is not required; kinds that vary list every type.
    #[test]
    fn the_schema_reads_integrality_presence_and_mixed_kinds() {
        let mut obs = InferObservation::new("", 100, 100);
        feed(
            &mut obs,
            A,
            "telemetry",
            "x",
            0.0,
            Some(&serde_json::json!({"n": 1, "m": "a", "opt": [1]})),
        );
        feed(
            &mut obs,
            A,
            "telemetry",
            "x",
            1.0,
            Some(&serde_json::json!({"n": 2, "m": 3})),
        );
        let r = infer(&obs, "v1/**", None, None);
        let t = &r.producers[0].types[0];
        assert_eq!(
            t.schema,
            Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "n": {"type": "integer"},
                    "m": {"type": ["integer", "string"]},
                    "opt": {"type": "array", "$comment": "items not inferred: arrays are leaves to the field walk"},
                },
                "required": ["m", "n"],
            }))
        );
    }

    /// The draft file: the marker, no `since`, every unestablished field
    /// absent, comments above the entry — and it parses as TOML.
    #[test]
    fn the_draft_carries_the_marker_and_omits_what_it_could_not_establish() {
        let mut obs = InferObservation::new("", 100, 100);
        for o in [A, B] {
            feed(&mut obs, o, "state", "health", 0.0, None);
        }
        let r = infer(&obs, "v1/**", Some(1.0), None);
        let prov = Provenance {
            app: "unknown".into(),
            source: "v1/**".into(),
            span_s: r.span_s,
            at: "2026-09-06T00:00:00Z".into(),
            keys: r.keys_seen,
            samples: r.samples,
            dropped: 0,
        };
        let toml = to_draft_toml(&r.producers[0], &prov);
        assert!(
            toml.starts_with("# DRAFT — inferred from v1/** over"),
            "{toml}"
        );
        assert!(toml.contains("draft = true\n"));
        assert!(toml.contains("compat = \"none\"\n"));
        assert!(
            !toml.contains("since ="),
            "a draft has no version stream:\n{toml}"
        );
        assert!(
            !toml.contains("ttl_s ="),
            "unestablished ttl is absent:\n{toml}"
        );
        assert!(
            !toml.contains("\nqos ="),
            "observed qos is never a field:\n{toml}"
        );
        // The alert family is the one exception, by RFC 08 §5's rule.
        let mut alert = InferObservation::new("", 10, 10);
        for o in [A, B] {
            feed(&mut alert, o, "state", "alert/disk_full", 0.0, None);
        }
        let r2 = infer(&alert, "v1/**", None, None);
        let a = &r2.producers[0].subjects[0];
        assert_eq!(a.qos.as_deref(), Some("alert"));
        assert!(
            a.comments.iter().any(|c| c.contains("RFC 08 §5 requires")),
            "{:?}",
            a.comments
        );
        assert!(to_draft_toml(&r2.producers[0], &prov).contains("\nqos = \"alert\"\n"));
        assert!(toml.contains("# ttl_s not established"), "{toml}");
        assert!(toml.contains("description = \"inferred; unreviewed\""));
        let doc: toml::Value = toml::from_str(&toml).expect("the draft parses");
        assert_eq!(doc["registry"]["draft"], toml::Value::Boolean(true));
        let types = to_draft_types_toml(&r.producers, &prov);
        let doc: toml::Value = toml::from_str(&types).expect("the type table parses");
        assert_eq!(
            doc["types"]["DemoHealthOpaque"]["kind"].as_str(),
            Some("unknown")
        );
        assert_eq!(
            draft_file_names(&r),
            vec!["demo.toml".to_string(), "types.toml".to_string()]
        );
    }

    /// A hint slice names the vars where its pattern binds with the same
    /// positions, and nowhere else.
    #[test]
    fn a_hint_names_the_vars_it_can_bind() {
        let slice = zenkey::parse_slice(
            "[registry]\nversion = \"1.0\"\napp = \"t\"\nconvention = 1\n[producer]\nname = \"demo\"\n\
             [[subject]]\npath = \"cpu/{core}/usage\"\nclass = \"telemetry\"\ntype = \"P\"\n",
        )
        .unwrap();
        let hints = SliceSet::from_slices(vec![slice]);
        let mut obs = InferObservation::new("", 100, 100);
        for (i, o) in [A, B].iter().enumerate() {
            feed(
                &mut obs,
                o,
                "telemetry",
                &format!("cpu/core{i}/usage"),
                0.0,
                None,
            );
            feed(
                &mut obs,
                o,
                "telemetry",
                &format!("disk/mnt{i}/used"),
                0.0,
                None,
            );
        }
        let r = infer(&obs, "v1/**", None, Some(&hints));
        let got = paths(&r);
        assert!(
            got.contains(&("telemetry".into(), "cpu/{core}/usage".into())),
            "{got:?}"
        );
        assert!(
            got.contains(&("telemetry".into(), "disk/{v1}/used".into())),
            "{got:?}"
        );
        assert!(r.hinted_by_registry);
    }

    #[test]
    fn helpers() {
        assert_eq!(round_up_pow10(0), 1);
        assert_eq!(round_up_pow10(1), 1);
        assert_eq!(round_up_pow10(2), 10);
        assert_eq!(round_up_pow10(10), 10);
        assert_eq!(round_up_pow10(11), 100);
        assert_eq!(camel("usage_percent"), "UsagePercent");
        assert_eq!(unit_of("rx_bytes"), Some("bytes"));
        assert_eq!(unit_of("p95_ms"), Some("ms"));
        assert_eq!(unit_of("usage"), None);
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[1.0, 3.0]), Some(2.0));
    }
}
