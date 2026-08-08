//! The key tree.
//!
//! The snapshot it walks is grammar-blind — `KeyTreeSnapshot` groups on a plain
//! `split('/')` — so an arbitrary bus renders as an ordinary chunk tree with
//! counts and rates. On top of that this module lays the *positional* overlay:
//! when a subtree turns out to be keyspace-v2, its chunks are labelled origin /
//! class / producer / subject.
//!
//! The overlay is resolved **relative to the base, never by absolute index**
//! (RFC 03 §1.1) — a multi-chunk base like `acme/fleet-a` is legal — and the
//! producer-or-subject question at position 5 is decided by the origin chunk
//! alone (RFC 03 §1.5). If the chunk where `v1` should be is not `v1`, the
//! whole subtree is left unlabelled rather than mislabelled.
//!
//! Tree v2 (issue #65) adds three things on top:
//!
//! - **Pivots**: the same entries re-grouped by origin / producer / class.
//!   The re-keying happens here, on the flattened entries — the hot
//!   `TreeNode` stays registry-blind (the §15 amendment's ruling).
//! - **Find-in-tree**: a fuzzy filter over real key paths. A filtered tree is
//!   a partial view, so it says what it hides ("showing N of M") — O5's
//!   spirit applied to a local filter.
//! - **Virtualized rendering**: rows have a fixed height and only the
//!   scrolled-into window is built each frame, so the row cap is a memory
//!   bound (still reported when hit), not a display truncation.

use std::collections::BTreeSet;
use std::time::Instant;

use iced::widget::{Column, button, column, row, text};
use iced::{Element, Length};
use zenkey_fleet::skeleton::{DeclRef, MergedNode, NodeStats, NodeStatus};

use crate::keyfacts::{KeyFacts, Registration};
use crate::message::Message;
use crate::view::kit::{self, human_bytes, human_rate};
use crate::view::theme::{RegistrationTone, colors};
use crate::view::tokens::{font, space};

/// Fixed row height — what makes the scroll window arithmetic exact.
pub const ROW_HEIGHT: f32 = 24.0;
/// Rows rendered beyond each window edge, so scrolling never shows a gap.
const OVERSCAN: usize = 8;

/// What a chunk means, when the subtree is conforming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Version,
    Origin,
    Class,
    Producer,
    BlobTier,
    Subject,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Role::Version => "version",
            Role::Origin => "origin",
            Role::Class => "class",
            Role::Producer => "producer",
            Role::BlobTier => "tier",
            Role::Subject => "subject",
        }
    }
}

/// How the tree groups its entries (issue #65).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pivot {
    /// Raw chunk order — the wire as it is.
    #[default]
    Chunks,
    /// Origin first (service origins like `@catalog` are first-class groups).
    Origin,
    /// Producer first (service-origin keys group under their `@…` origin,
    /// which *is* the service; foreign keys under "(foreign)").
    Producer,
    /// Class/plane first.
    Class,
}

impl Pivot {
    pub const ALL: [Pivot; 4] = [Pivot::Chunks, Pivot::Origin, Pivot::Producer, Pivot::Class];
}

impl std::fmt::Display for Pivot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Pivot::Chunks => "chunks",
            Pivot::Origin => "by origin",
            Pivot::Producer => "by producer",
            Pivot::Class => "by class",
        })
    }
}

/// One rendered line of the tree.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeRow {
    pub depth: usize,
    /// Display chunk — a symbolic skeleton position renders as `{var}`.
    pub chunk: String,
    /// The display-path prefix this row stands for (pivot paths in pivot
    /// mode — an expansion key, not necessarily a wire path).
    pub path: String,
    /// The *real* display path select/watch should act on. `None` on pivot
    /// group rows, whose synthetic grouping has no contiguous wire subtree.
    pub target: Option<String>,
    pub has_children: bool,
    pub expanded: bool,
    /// The declared/observed state (issue #85's typed acceptance criterion).
    pub status: NodeStatus,
    /// Traffic landed on exactly this key.
    pub is_leaf: bool,
    pub count: u64,
    pub bytes: u64,
    pub rate_hz: f64,
    pub subtree_count: u64,
    pub subtree_bytes: u64,
    pub subtree_keys: usize,
    pub subtree_rate_hz: f64,
    /// Seconds since the newest sample in this subtree (the freshness dot);
    /// `None` when nothing was ever seen here.
    pub age_s: Option<f32>,
    /// `None` for any chunk outside a recognised v1 subtree.
    pub role: Option<Role>,
    /// The registry-declared payload type, when the skeleton knows one.
    pub decl_type: Option<String>,
}

/// The flattened tree, plus what was left out.
#[derive(Debug, Clone, PartialEq)]
pub struct Flattened {
    pub rows: Vec<TreeRow>,
    /// Rows beyond the cap. Reported, never silently dropped.
    pub truncated: usize,
    /// Concrete entries surviving the filter (== `total_keys` unfiltered).
    pub shown_keys: usize,
    /// All concrete entries the tree holds.
    pub total_keys: usize,
    /// Whether a find-in-tree filter is active.
    pub filtered: bool,
}

impl Flattened {
    pub fn empty() -> Flattened {
        Flattened {
            rows: Vec::new(),
            truncated: 0,
            shown_keys: 0,
            total_keys: 0,
            filtered: false,
        }
    }
}

/// Case-insensitive fuzzy match: every query char appears in order in the
/// haystack (substring matches trivially satisfy this).
pub fn fuzzy_match(haystack: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let mut qs = query.chars().flat_map(char::to_lowercase);
    let mut want = qs.next();
    for c in haystack.chars().flat_map(char::to_lowercase) {
        match want {
            Some(w) if w == c => want = qs.next(),
            Some(_) => {}
            None => break,
        }
    }
    want.is_none()
}

fn age_of(stats: Option<&NodeStats>, now: Instant) -> Option<f32> {
    stats
        .and_then(|s| s.subtree_last_seen)
        .map(|t| now.saturating_duration_since(t).as_secs_f32())
}

/// An "entry" is what pivots and filters count: a node that is a concrete
/// key with traffic, or a childless declared position.
fn is_entry(node: &MergedNode) -> bool {
    node.children.is_empty() || node.stats.map(|s| s.count > 0).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Chunk-order flatten (the default pivot)
// ---------------------------------------------------------------------------

/// Flatten a snapshot into display rows.
///
/// `base` is the active deployment base, used only to know how many leading
/// chunks to skip before looking for `v1`. `expanded` holds the paths the user
/// has opened; everything else collapses.
pub fn flatten(
    merged: &MergedNode,
    base: &str,
    expanded: &BTreeSet<String>,
    max_rows: usize,
    now: Instant,
) -> Flattened {
    let mut rows = Vec::new();
    let mut truncated = 0;
    let mut total_keys = 0;
    walk(
        merged,
        &mut Ctx {
            expanded,
            max_rows,
            rows: &mut rows,
            truncated: &mut truncated,
            total_keys: &mut total_keys,
            now,
        },
        String::new(),
        0,
        start_expect(base),
    );
    Flattened {
        rows,
        truncated,
        shown_keys: total_keys,
        total_keys,
        filtered: false,
    }
}

fn start_expect(base: &str) -> Expect {
    if base.is_empty() {
        Expect::Version
    } else {
        Expect::Base(base.split('/').count())
    }
}

struct Ctx<'a> {
    expanded: &'a BTreeSet<String>,
    max_rows: usize,
    rows: &'a mut Vec<TreeRow>,
    truncated: &'a mut usize,
    total_keys: &'a mut usize,
    now: Instant,
}

/// What the *next* chunk down this branch is expected to be.
///
/// A state machine rather than arithmetic on depth, because depth alone cannot
/// distinguish "we have not reached the origin position yet" from "this branch
/// is not conforming at all" — and conflating them labels a foreign chunk
/// `origin`, which is worse than saying nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// `n` base chunks still to consume before the convention starts.
    Base(usize),
    Version,
    Origin,
    /// `host` is what decides whether position 5 is a producer (RFC 03 §1.5).
    Class {
        host: bool,
    },
    Producer {
        host: bool,
    },
    BlobTier,
    Subject,
    /// This branch is not keyspace-v2. Nothing below it is labelled.
    Foreign,
}

fn walk(node: &MergedNode, ctx: &mut Ctx<'_>, path: String, depth: usize, expect: Expect) {
    for (chunk, child) in &node.children {
        let child_path = if path.is_empty() {
            chunk.clone()
        } else {
            format!("{path}/{chunk}")
        };
        let (role, next_expect) = classify(chunk, expect);
        if is_entry(child) {
            *ctx.total_keys += 1;
        }

        if ctx.rows.len() >= ctx.max_rows {
            *ctx.truncated += 1;
        } else {
            let expanded = ctx.expanded.contains(&child_path);
            let stats = child.stats;
            ctx.rows.push(TreeRow {
                depth,
                chunk: chunk.clone(),
                path: child_path.clone(),
                target: Some(child_path.clone()),
                has_children: !child.children.is_empty(),
                expanded,
                status: child.status,
                is_leaf: stats.map(|s| s.count > 0).unwrap_or(false),
                count: stats.map(|s| s.count).unwrap_or(0),
                bytes: stats.map(|s| s.bytes).unwrap_or(0),
                rate_hz: stats.map(|s| s.rate_hz).unwrap_or(0.0),
                subtree_count: stats.map(|s| s.subtree_count).unwrap_or(0),
                subtree_bytes: stats.map(|s| s.subtree_bytes).unwrap_or(0),
                subtree_keys: stats.map(|s| s.subtree_keys).unwrap_or(0),
                subtree_rate_hz: stats.map(|s| s.subtree_rate_hz).unwrap_or(0.0),
                age_s: age_of(stats.as_ref(), ctx.now),
                role,
                decl_type: child.decl.as_ref().map(|d| d.type_name.clone()),
            });
            if expanded {
                walk(child, ctx, child_path, depth + 1, next_expect);
                continue;
            }
        }
        // Even truncated/collapsed branches count their entries, or the
        // "N of M" denominator would understate the bus.
        if !child.children.is_empty() {
            count_entries(child, ctx.total_keys);
        }
    }
}

fn count_entries(node: &MergedNode, total: &mut usize) {
    for child in node.children.values() {
        if is_entry(child) {
            *total += 1;
        }
        count_entries(child, total);
    }
}

/// The role of one chunk, and what to expect below it.
fn classify(chunk: &str, expect: Expect) -> (Option<Role>, Expect) {
    match expect {
        // The convention says nothing about how a deployment spells its own
        // base, so base chunks carry no role.
        Expect::Base(n) if n > 1 => (None, Expect::Base(n - 1)),
        Expect::Base(_) => (None, Expect::Version),
        Expect::Version => {
            if chunk == zenkey::grammar::VERSION_CHUNK {
                (Some(Role::Version), Expect::Origin)
            } else {
                // Not a v1 subtree. Leave the whole branch unlabelled rather
                // than guess — mislabelling is worse than not labelling.
                (None, Expect::Foreign)
            }
        }
        Expect::Origin => {
            // RFC 03 §1.3 licenses tooling to use this shape, and §1.5 makes it
            // the sole discriminator for the producer position. A *symbolic*
            // origin (`{origin}`) exists only for host slices — the skeleton
            // never symbolizes a service origin (those are literal `@x`) — so
            // it classifies as host-like.
            let host = zenkey::grammar::is_valid_host_origin(chunk) || chunk.starts_with('{');
            (Some(Role::Origin), Expect::Class { host })
        }
        Expect::Class { host } => {
            // Under `@blob` position 5 is a tier token, not a producer.
            let next = if chunk == zenkey::grammar::PLANE_BLOB {
                Expect::BlobTier
            } else {
                Expect::Producer { host }
            };
            (Some(Role::Class), next)
        }
        // A service origin omits the producer chunk entirely; position 5 is
        // already subject (RFC 03 §1.5).
        Expect::Producer { host: true } => (Some(Role::Producer), Expect::Subject),
        Expect::Producer { host: false } => (Some(Role::Subject), Expect::Subject),
        Expect::BlobTier => (Some(Role::BlobTier), Expect::Subject),
        Expect::Subject => (Some(Role::Subject), Expect::Subject),
        Expect::Foreign => (None, Expect::Foreign),
    }
}

// ---------------------------------------------------------------------------
// Find-in-tree (chunk order + filter)
// ---------------------------------------------------------------------------

/// Flatten with a fuzzy filter over real key paths: matching entries and
/// their ancestor chain render (ancestors auto-expand); everything else is
/// hidden — and counted, so the view can say "showing N of M".
pub fn search_flatten(
    merged: &MergedNode,
    base: &str,
    query: &str,
    max_rows: usize,
    now: Instant,
) -> Flattened {
    let mut rows = Vec::new();
    let mut shown = 0usize;
    let mut total = 0usize;
    search_walk(
        merged,
        String::new(),
        0,
        start_expect(base),
        query,
        now,
        &mut rows,
        &mut shown,
        &mut total,
    );
    let truncated = rows.len().saturating_sub(max_rows);
    rows.truncate(max_rows);
    Flattened {
        rows,
        truncated,
        shown_keys: shown,
        total_keys: total,
        filtered: true,
    }
}

/// Returns whether this subtree contains any match (so ancestors render).
#[allow(clippy::too_many_arguments)]
fn search_walk(
    node: &MergedNode,
    path: String,
    depth: usize,
    expect: Expect,
    query: &str,
    now: Instant,
    rows: &mut Vec<TreeRow>,
    shown: &mut usize,
    total: &mut usize,
) -> bool {
    let mut any = false;
    for (chunk, child) in &node.children {
        let child_path = if path.is_empty() {
            chunk.clone()
        } else {
            format!("{path}/{chunk}")
        };
        let (role, next_expect) = classify(chunk, expect);
        let entry = is_entry(child);
        if entry {
            *total += 1;
        }
        let self_match = entry && fuzzy_match(&child_path, query);
        if self_match {
            *shown += 1;
        }

        // Tentatively emit the row, then recurse; drop it again if neither it
        // nor anything below matched.
        let at = rows.len();
        let stats = child.stats;
        rows.push(TreeRow {
            depth,
            chunk: chunk.clone(),
            path: child_path.clone(),
            target: Some(child_path.clone()),
            has_children: !child.children.is_empty(),
            expanded: true,
            status: child.status,
            is_leaf: stats.map(|s| s.count > 0).unwrap_or(false),
            count: stats.map(|s| s.count).unwrap_or(0),
            bytes: stats.map(|s| s.bytes).unwrap_or(0),
            rate_hz: stats.map(|s| s.rate_hz).unwrap_or(0.0),
            subtree_count: stats.map(|s| s.subtree_count).unwrap_or(0),
            subtree_bytes: stats.map(|s| s.subtree_bytes).unwrap_or(0),
            subtree_keys: stats.map(|s| s.subtree_keys).unwrap_or(0),
            subtree_rate_hz: stats.map(|s| s.subtree_rate_hz).unwrap_or(0.0),
            age_s: age_of(stats.as_ref(), now),
            role,
            decl_type: child.decl.as_ref().map(|d| d.type_name.clone()),
        });
        let below = search_walk(
            child,
            child_path,
            depth + 1,
            next_expect,
            query,
            now,
            rows,
            shown,
            total,
        );
        if self_match || below {
            any = true;
        } else {
            rows.truncate(at);
        }
    }
    any
}

// ---------------------------------------------------------------------------
// Pivots
// ---------------------------------------------------------------------------

/// One concrete entry, with its grammar coordinates extracted.
struct PivotEntry {
    real_path: String,
    origin: Option<String>,
    class: Option<String>,
    producer: Option<String>,
    /// Everything after the extracted coordinates (subject chunks, blob
    /// tiers…) — or the whole path for foreign keys.
    tail: Vec<String>,
    status: NodeStatus,
    stats: Option<NodeStats>,
    decl: Option<DeclRef>,
}

fn collect_entries(
    node: &MergedNode,
    path: &str,
    expect: Expect,
    coords: (Option<&str>, Option<&str>, Option<&str>),
    tail: &[String],
    out: &mut Vec<PivotEntry>,
) {
    for (chunk, child) in &node.children {
        let child_path = if path.is_empty() {
            chunk.clone()
        } else {
            format!("{path}/{chunk}")
        };
        let (role, next_expect) = classify(chunk, expect);
        let (mut origin, mut class, mut producer) = coords;
        let mut next_tail = tail.to_vec();
        match role {
            Some(Role::Origin) => origin = Some(chunk.as_str()),
            Some(Role::Class) => class = Some(chunk.as_str()),
            Some(Role::Producer) => producer = Some(chunk.as_str()),
            Some(Role::Version) => {}
            Some(Role::Subject) | Some(Role::BlobTier) => next_tail.push(chunk.clone()),
            None => next_tail.push(chunk.clone()),
        }
        if is_entry(child) {
            out.push(PivotEntry {
                real_path: child_path.clone(),
                origin: origin.map(str::to_string),
                class: class.map(str::to_string),
                producer: producer.map(str::to_string),
                tail: next_tail.clone(),
                status: child.status,
                stats: child.stats,
                decl: child.decl.clone(),
            });
        }
        collect_entries(
            child,
            &child_path,
            next_expect,
            (origin, class, producer),
            &next_tail,
            out,
        );
    }
}

/// The synthetic grouping tree a pivot builds from entries.
#[derive(Default)]
struct PNode {
    children: std::collections::BTreeMap<String, PNode>,
    role: Option<Role>,
    /// Set when an entry terminates here: (real path, own stats, decl).
    leaf: Option<(String, Option<NodeStats>, Option<DeclRef>)>,
    status: Option<NodeStatus>,
    agg_count: u64,
    agg_bytes: u64,
    agg_rate: f64,
    agg_keys: usize,
    agg_last: Option<Instant>,
}

/// Fold two node statuses: the stronger observation wins, evidence unions.
fn fold_status(a: Option<NodeStatus>, b: NodeStatus) -> NodeStatus {
    let Some(a) = a else { return b };
    let rank = |s: &NodeStatus| match s {
        NodeStatus::Observed(_) => 3,
        NodeStatus::Unwatched(_) => 2,
        NodeStatus::WatchedQuiet(_) => 1,
        NodeStatus::DeclaredOnly(_) => 0,
    };
    if rank(&b) > rank(&a) { b } else { a }
}

/// Group the tree's entries by origin / producer / class (issue #65).
///
/// The re-key per entry:
/// - **Origin**: `origin / class / [producer] / subject…`
/// - **Producer**: `producer / origin / class / subject…` — a service-origin
///   key has no producer chunk, so its origin (`@catalog`) is the group,
///   which *is* the service (RFC 03 §1.5);
/// - **Class**: `class / origin / [producer] / subject…`
///
/// Foreign keys group under `(foreign)` with their raw path below — present,
/// unlabelled, never silently dropped. An optional `query` filters entries
/// by real path (groups auto-expand while filtering).
#[allow(clippy::too_many_arguments)]
pub fn pivot_flatten(
    merged: &MergedNode,
    base: &str,
    pivot: Pivot,
    expanded: &BTreeSet<String>,
    query: &str,
    max_rows: usize,
    now: Instant,
) -> Flattened {
    let mut entries = Vec::new();
    collect_entries(
        merged,
        "",
        start_expect(base),
        (None, None, None),
        &[],
        &mut entries,
    );
    let total_keys = entries.len();
    let filtered = !query.is_empty();
    if filtered {
        entries.retain(|e| fuzzy_match(&e.real_path, query));
    }
    let shown_keys = entries.len();

    let mut root = PNode::default();
    for e in &entries {
        let chunks = pivot_chunks(e, pivot);
        let mut node = &mut root;
        for (chunk, role) in &chunks {
            node = node.children.entry(chunk.clone()).or_default();
            node.role = *role;
            node.status = Some(fold_status(node.status, e.status));
            if let Some(s) = &e.stats {
                node.agg_count += s.count;
                node.agg_bytes += s.bytes;
                node.agg_rate += s.rate_hz;
                node.agg_keys += usize::from(s.count > 0);
                node.agg_last = node.agg_last.max(s.subtree_last_seen);
            }
        }
        node.leaf = Some((e.real_path.clone(), e.stats, e.decl.clone()));
    }

    let mut rows = Vec::new();
    flatten_pnode(&root, expanded, filtered, now, String::new(), 0, &mut rows);
    let truncated = rows.len().saturating_sub(max_rows);
    rows.truncate(max_rows);
    Flattened {
        rows,
        truncated,
        shown_keys,
        total_keys,
        filtered,
    }
}

fn pivot_chunks(e: &PivotEntry, pivot: Pivot) -> Vec<(String, Option<Role>)> {
    let mut out: Vec<(String, Option<Role>)> = Vec::new();
    let origin = e.origin.as_deref();
    let class = e.class.as_deref();
    let producer = e.producer.as_deref();
    let foreign = origin.is_none() && class.is_none();
    if foreign {
        out.push(("(foreign)".to_string(), None));
        out.extend(e.tail.iter().map(|c| (c.clone(), None)));
        return out;
    }
    let push = |out: &mut Vec<(String, Option<Role>)>, v: Option<&str>, role: Role| {
        if let Some(v) = v {
            out.push((v.to_string(), Some(role)));
        }
    };
    match pivot {
        Pivot::Chunks => unreachable!("chunk order uses flatten()"),
        Pivot::Origin => {
            push(&mut out, origin, Role::Origin);
            push(&mut out, class, Role::Class);
            push(&mut out, producer, Role::Producer);
        }
        Pivot::Producer => {
            // The producer when there is one; the service origin otherwise —
            // for a service origin the service IS the producer (RFC 03 §1.5).
            match (producer, origin) {
                (Some(p), _) => out.push((p.to_string(), Some(Role::Producer))),
                (None, Some(o)) => out.push((o.to_string(), Some(Role::Origin))),
                (None, None) => out.push(("(no producer)".to_string(), None)),
            }
            if producer.is_some() {
                push(&mut out, origin, Role::Origin);
            }
            push(&mut out, class, Role::Class);
        }
        Pivot::Class => {
            push(&mut out, class, Role::Class);
            push(&mut out, origin, Role::Origin);
            push(&mut out, producer, Role::Producer);
        }
    }
    out.extend(e.tail.iter().map(|c| (c.clone(), Some(Role::Subject))));
    if out.is_empty() {
        // An entry that *is* one of the coordinates (e.g. a declared class
        // position with no subject below it yet).
        out.push(("(…)".to_string(), None));
    }
    out
}

fn flatten_pnode(
    node: &PNode,
    expanded: &BTreeSet<String>,
    auto_expand: bool,
    now: Instant,
    path: String,
    depth: usize,
    rows: &mut Vec<TreeRow>,
) {
    for (chunk, child) in &node.children {
        let child_path = if path.is_empty() {
            format!("pivot:{chunk}")
        } else {
            format!("{path}/{chunk}")
        };
        let is_open = auto_expand || expanded.contains(&child_path);
        let own = child.leaf.as_ref().and_then(|(_, s, _)| *s);
        rows.push(TreeRow {
            depth,
            chunk: chunk.clone(),
            path: child_path.clone(),
            target: child.leaf.as_ref().map(|(p, _, _)| p.clone()),
            has_children: !child.children.is_empty(),
            expanded: is_open,
            status: child
                .status
                .unwrap_or(NodeStatus::DeclaredOnly(Default::default())),
            is_leaf: own.map(|s| s.count > 0).unwrap_or(false),
            count: own.map(|s| s.count).unwrap_or(0),
            bytes: own.map(|s| s.bytes).unwrap_or(0),
            rate_hz: own.map(|s| s.rate_hz).unwrap_or(0.0),
            subtree_count: child.agg_count,
            subtree_bytes: child.agg_bytes,
            subtree_keys: child.agg_keys,
            subtree_rate_hz: child.agg_rate,
            age_s: child
                .agg_last
                .map(|t| now.saturating_duration_since(t).as_secs_f32()),
            role: child.role,
            decl_type: child
                .leaf
                .as_ref()
                .and_then(|(_, _, d)| d.as_ref().map(|d| d.type_name.clone())),
        });
        if is_open {
            flatten_pnode(
                child,
                expanded,
                auto_expand,
                now,
                child_path,
                depth + 1,
                rows,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Which tone a leaf's registration should read as.
pub fn tone(reg: &Registration) -> RegistrationTone {
    match reg {
        Registration::Registered(_) => RegistrationTone::Registered,
        Registration::Unregistered => RegistrationTone::Unregistered,
        Registration::NoSliceForProducer => RegistrationTone::NoSlice,
        Registration::Unknown => RegistrationTone::Unknown,
        Registration::NotApplicable => RegistrationTone::NotApplicable,
    }
}

/// The badge text for a registration state.
///
/// The tri-state is only useful if the words differ: "—" (not asked) must not
/// read like "unregistered" (asked, and the answer was no).
pub fn registration_label(reg: &Registration) -> &'static str {
    match reg {
        Registration::Registered(_) => "registered",
        Registration::Unregistered => "unregistered",
        Registration::NoSliceForProducer => "no slice",
        Registration::Unknown => "—",
        Registration::NotApplicable => "",
    }
}

/// Registration lookup by wire key. A plain map rather than a closure so the
/// pane borrows app state directly instead of a temporary.
pub type FactsIndex = std::collections::HashMap<String, KeyFacts>;

/// The visible row window for a scroll position: `(first, last)` indices
/// into the flattened rows, with overscan. Pure, so it is testable without
/// a renderer.
pub fn window(rows: usize, scroll_y: f32, viewport_h: f32) -> (usize, usize) {
    let first = (scroll_y / ROW_HEIGHT).floor() as usize;
    let visible = (viewport_h / ROW_HEIGHT).ceil() as usize + 1;
    let first = first.saturating_sub(OVERSCAN);
    let last = (first + visible + 2 * OVERSCAN).min(rows);
    (first.min(rows), last)
}

/// Render the tree pane.
pub fn tree_view<'a>(
    flat: &'a Flattened,
    facts: &'a FactsIndex,
    selected: Option<&'a str>,
    watched_paths: &'a BTreeSet<String>,
    scroll_y: f32,
    viewport_h: f32,
) -> Element<'a, Message> {
    if flat.rows.is_empty() {
        if flat.filtered && flat.total_keys > 0 {
            return kit::empty_state(
                "No keys match",
                "The filter hides every key — showing 0 of the tree's entries. \
                 Clear it to see them again.",
            );
        }
        return kit::empty_state(
            "Nothing observed yet",
            "An empty tree is not a verdict about the bus (RFC 05 §3.1) — \
             it may simply mean nothing has published on the current scope.",
        );
    }

    let (first, last) = window(flat.rows.len(), scroll_y, viewport_h);
    let mut col = Column::new();
    if first > 0 {
        col = col.push(iced::widget::Space::new().height(Length::Fixed(first as f32 * ROW_HEIGHT)));
    }
    for r in &flat.rows[first..last] {
        col = col.push(
            iced::widget::container(row_view(r, facts, selected, watched_paths))
                .height(Length::Fixed(ROW_HEIGHT)),
        );
    }
    if last < flat.rows.len() {
        col = col.push(
            iced::widget::Space::new()
                .height(Length::Fixed((flat.rows.len() - last) as f32 * ROW_HEIGHT)),
        );
    }
    if flat.truncated > 0 {
        col = col.push(kit::muted(format!(
            "… {} more rows not built (row cap)",
            flat.truncated
        )));
    }
    iced::widget::scrollable(col)
        .height(Length::Fill)
        .on_scroll(|viewport| {
            Message::TreeScrolled(viewport.absolute_offset().y, viewport.bounds().height)
        })
        .into()
}

fn row_view<'a>(
    r: &'a TreeRow,
    facts: &'a FactsIndex,
    selected: Option<&'a str>,
    watched_paths: &'a BTreeSet<String>,
) -> Element<'a, Message> {
    let indent = iced::widget::Space::new().width(Length::Fixed(r.depth as f32 * 14.0));

    let marker = if r.has_children {
        if r.expanded { "▾" } else { "▸" }
    } else {
        " "
    };

    let is_selected = r.target.is_some() && selected == r.target.as_deref();
    let name = text(format!("{marker} {}", r.chunk))
        .size(font::CAPTION)
        .font(iced::Font::MONOSPACE)
        .style(move |theme: &iced::Theme| text::Style {
            color: Some(if is_selected {
                colors(theme).primary()
            } else {
                colors(theme).text()
            }),
        });

    let mut line = row![indent, name]
        .spacing(space::XS)
        .align_y(iced::Alignment::Center);

    // Freshness dot (issue #65): green = seen just now, fades to dim.
    if let Some(age) = r.age_s {
        line = line.push(
            text("●")
                .size(font::CAPTION)
                .style(move |theme: &iced::Theme| text::Style {
                    color: Some(if age < 3.0 {
                        colors(theme).success()
                    } else if age < 30.0 {
                        colors(theme).warning()
                    } else {
                        colors(theme).text_dim()
                    }),
                }),
        );
    }

    if let Some(role) = r.role {
        line = line.push(kit::muted(role.label()));
    }

    // The declared/observed state (issue #85): "declared" must never read
    // like "quiet", and "unwatched" like neither.
    match r.status {
        NodeStatus::DeclaredOnly(_) => {
            line = line.push(kit::tone_badge(
                crate::view::theme::RegistrationTone::Unknown,
                "declared",
            ));
        }
        NodeStatus::WatchedQuiet(_) => {
            line = line.push(kit::muted("quiet"));
        }
        NodeStatus::Unwatched(_) => {
            line = line.push(kit::tone_badge(
                crate::view::theme::RegistrationTone::Unregistered,
                "unwatched",
            ));
        }
        NodeStatus::Observed(_) => {}
    }
    if let Some(ty) = &r.decl_type {
        line = line.push(kit::muted(ty.clone()));
    }

    line = line.push(iced::widget::space::horizontal());

    // A collapsed node reports its whole subtree; an expanded leaf reports
    // itself. Showing only leaf traffic on a collapsed node would understate
    // it by exactly the part the user cannot see.
    if r.is_leaf {
        line = line.push(kit::muted(format!(
            "{} · {} · {}",
            r.count,
            human_bytes(r.bytes),
            human_rate(r.rate_hz)
        )));
        if let Some(f) = r.target.as_deref().and_then(|t| facts.get(t)) {
            let label = registration_label(&f.registration);
            if !label.is_empty() {
                line = line.push(kit::tone_badge(tone(&f.registration), label));
            }
            if let Some(ty) = f.type_name() {
                line = line.push(kit::muted(ty.to_string()));
            }
        }
    } else if !r.expanded && r.subtree_count > 0 {
        line = line.push(kit::muted(format!(
            "{} · {} · {} · {}",
            kit::plural(r.subtree_keys, "key"),
            r.subtree_count,
            human_bytes(r.subtree_bytes),
            human_rate(r.subtree_rate_hz)
        )));
    }

    // Observation is opt-in, per subtree (issue #85). The toggle reflects
    // *this app's* watches, not global coverage — and only rows standing for
    // a real wire subtree offer it (pivot groups are synthetic).
    let watch: Element<'a, Message> = match &r.target {
        Some(t) => {
            let watch_label = if watched_paths.contains(t) {
                "◉"
            } else {
                "○"
            };
            button(text(watch_label).size(font::CAPTION))
                .padding(2)
                .style(button::text)
                .on_press(Message::WatchToggled(t.clone()))
                .into()
        }
        None => iced::widget::Space::new().width(Length::Fixed(18.0)).into(),
    };

    let press = if r.has_children {
        Message::ToggleNode(r.path.clone())
    } else {
        match &r.target {
            Some(t) => Message::SelectKey(Some(t.clone())),
            None => Message::ToggleNode(r.path.clone()),
        }
    };
    let body = button(line)
        .width(Length::Fill)
        .padding(2)
        .style(|_theme: &iced::Theme, _status| button::Style {
            background: None,
            text_color: iced::Color::WHITE,
            ..Default::default()
        })
        .on_press(press);
    row![watch, body]
        .spacing(space::XS)
        .align_y(iced::Alignment::Center)
        .into()
}

/// A standalone pane, for tests and for the `view/mod` composition.
#[allow(clippy::too_many_arguments)]
pub fn pane<'a>(
    flat: &'a Flattened,
    facts: &'a FactsIndex,
    selected: Option<&'a str>,
    watched_paths: &'a BTreeSet<String>,
    pivot: Pivot,
    search: &'a str,
    scroll_y: f32,
    viewport_h: f32,
) -> Element<'a, Message> {
    let pivot_picker =
        iced::widget::pick_list(Pivot::ALL.to_vec(), Some(pivot), Message::PivotSelected)
            .text_size(font::CAPTION);
    let find = iced::widget::text_input("find keys…", search)
        .size(font::CAPTION)
        .on_input(Message::TreeSearchChanged);
    let mut header = row![pivot_picker, find]
        .spacing(space::SM)
        .align_y(iced::Alignment::Center);
    if flat.filtered {
        // A filtered tree is a partial view; say what it hides (O5's spirit).
        header = header.push(kit::muted(format!(
            "showing {} of {}",
            kit::plural(flat.shown_keys, "key"),
            flat.total_keys
        )));
    }
    column![
        kit::section_header("Keys", None),
        header,
        tree_view(flat, facts, selected, watched_paths, scroll_y, viewport_h)
    ]
    .spacing(space::SM)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey_fleet::stats::StatsTable;

    fn snapshot(keys: &[&str]) -> MergedNode {
        let mut stats = StatsTable::new();
        let now = Instant::now();
        for k in keys {
            stats.record(k, 8, None, now);
        }
        let observed = zenkey_fleet::KeyTreeSnapshot::build(&stats);
        // Tests watch everything: every observed node reads Observed.
        let skel = zenkey_fleet::Skeleton::build(
            "",
            &zenkey_fleet::SliceSet::default(),
            &std::collections::BTreeMap::new(),
            None,
        );
        zenkey_fleet::skeleton::merge(&skel, &observed, &["**".to_string()])
    }

    fn expand(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|s| s.to_string()).collect()
    }

    fn flat_now(
        snap: &MergedNode,
        base: &str,
        expanded: &BTreeSet<String>,
        max: usize,
    ) -> Flattened {
        flatten(snap, base, expanded, max, Instant::now())
    }

    fn role_of(flat: &Flattened, path: &str) -> Option<Role> {
        flat.rows
            .iter()
            .find(|r| r.path == path)
            .and_then(|r| r.role)
    }

    #[test]
    fn collapsed_by_default_and_expands_on_demand() {
        let snap = snapshot(&["v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu/usage"]);
        let flat = flat_now(&snap, "", &BTreeSet::new(), 100);
        assert_eq!(flat.rows.len(), 1, "only the root chunk shows");
        assert_eq!(flat.rows[0].chunk, "v1");
        assert!(flat.rows[0].has_children);

        let flat = flat_now(&snap, "", &expand(&["v1"]), 100);
        assert_eq!(flat.rows.len(), 2);
        assert_eq!(flat.rows[1].chunk, "h-3fa9c2d41b7e");
    }

    /// RFC 03 §1.1: the overlay is resolved relative to the base. The same
    /// subject under a one-chunk and a two-chunk base must label identically.
    #[test]
    fn roles_are_resolved_relative_to_the_base() {
        let key = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu";
        for (base, prefix) in [
            ("", ""),
            ("zensight", "zensight/"),
            ("acme/fleet-a", "acme/fleet-a/"),
        ] {
            let full = format!("{prefix}{key}");
            let snap = snapshot(&[&full]);
            // Expand the whole path.
            let mut paths = Vec::new();
            let mut acc = String::new();
            for chunk in full.split('/') {
                acc = if acc.is_empty() {
                    chunk.to_string()
                } else {
                    format!("{acc}/{chunk}")
                };
                paths.push(acc.clone());
            }
            let set: BTreeSet<String> = paths.into_iter().collect();
            let flat = flat_now(&snap, base, &set, 100);

            assert_eq!(role_of(&flat, &format!("{prefix}v1")), Some(Role::Version));
            assert_eq!(
                role_of(&flat, &format!("{prefix}v1/h-3fa9c2d41b7e")),
                Some(Role::Origin)
            );
            assert_eq!(
                role_of(&flat, &format!("{prefix}v1/h-3fa9c2d41b7e/telemetry")),
                Some(Role::Class)
            );
            assert_eq!(
                role_of(
                    &flat,
                    &format!("{prefix}v1/h-3fa9c2d41b7e/telemetry/sysinfo")
                ),
                Some(Role::Producer),
                "base {base:?}"
            );
        }
    }

    /// RFC 03 §1.5: under a service origin position 5 is already subject.
    #[test]
    fn a_service_origin_has_no_producer_position() {
        let snap = snapshot(&["v1/@catalog/state/entity/x"]);
        let set = expand(&[
            "v1",
            "v1/@catalog",
            "v1/@catalog/state",
            "v1/@catalog/state/entity",
        ]);
        let flat = flat_now(&snap, "", &set, 100);
        assert_eq!(role_of(&flat, "v1/@catalog"), Some(Role::Origin));
        assert_eq!(role_of(&flat, "v1/@catalog/state"), Some(Role::Class));
        assert_eq!(
            role_of(&flat, "v1/@catalog/state/entity"),
            Some(Role::Subject),
            "`entity` is subject, not a producer"
        );
    }

    /// Under `@blob` position 5 is a tier token, not a producer (RFC 03 §1.5).
    #[test]
    fn a_blob_tier_is_not_labelled_a_producer() {
        let snap = snapshot(&["v1/h-3fa9c2d41b7e/@blob/store/sha256/abcdef01"]);
        let set = expand(&[
            "v1",
            "v1/h-3fa9c2d41b7e",
            "v1/h-3fa9c2d41b7e/@blob",
            "v1/h-3fa9c2d41b7e/@blob/store",
        ]);
        let flat = flat_now(&snap, "", &set, 100);
        assert_eq!(role_of(&flat, "v1/h-3fa9c2d41b7e/@blob"), Some(Role::Class));
        assert_eq!(
            role_of(&flat, "v1/h-3fa9c2d41b7e/@blob/store"),
            Some(Role::BlobTier)
        );
        assert_eq!(
            role_of(&flat, "v1/h-3fa9c2d41b7e/@blob/store/sha256"),
            Some(Role::Subject)
        );
    }

    /// A foreign key is a first-class citizen: it gets rows and stats, and is
    /// left unlabelled rather than mislabelled.
    #[test]
    fn foreign_keys_render_unlabelled_but_present() {
        let snap = snapshot(&["demo/example/foo"]);
        let set = expand(&["demo", "demo/example"]);
        let flat = flat_now(&snap, "", &set, 100);
        assert_eq!(flat.rows.len(), 3);
        for r in &flat.rows {
            assert_eq!(r.role, None, "{} must not be labelled", r.path);
        }
        // …and it still carries its traffic.
        let leaf = flat
            .rows
            .iter()
            .find(|r| r.path == "demo/example/foo")
            .unwrap();
        assert!(leaf.is_leaf);
        assert_eq!(leaf.count, 1);
    }

    /// A subtree whose version chunk is not `v1` must not be labelled by
    /// position — that would attach convention meanings to foreign chunks.
    #[test]
    fn a_non_v1_subtree_is_left_unlabelled() {
        let snap = snapshot(&["v2/h-3fa9c2d41b7e/telemetry/sysinfo/cpu"]);
        let set = expand(&["v2", "v2/h-3fa9c2d41b7e", "v2/h-3fa9c2d41b7e/telemetry"]);
        let flat = flat_now(&snap, "", &set, 100);
        for r in &flat.rows {
            assert_eq!(r.role, None, "{} must not be labelled", r.path);
        }
    }

    /// A collapsed node must carry its subtree's totals, or the number the
    /// user reads understates the traffic by exactly the invisible part.
    #[test]
    fn collapsed_rows_report_subtree_totals() {
        let snap = snapshot(&[
            "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu",
            "v1/h-3fa9c2d41b7e/telemetry/sysinfo/mem",
            "v1/h-3fa9c2d41b7e/state/sysinfo/health",
        ]);
        let flat = flat_now(&snap, "", &BTreeSet::new(), 100);
        let root = &flat.rows[0];
        assert_eq!(root.chunk, "v1");
        assert!(!root.expanded);
        assert_eq!(root.subtree_count, 3);
        assert_eq!(root.subtree_keys, 3);
        assert_eq!(root.subtree_bytes, 24);
        // The freshness signal is present: the subtree was just seen.
        assert!(root.age_s.is_some());
        assert!(root.age_s.unwrap() < 1.0);
        // Unfiltered: N of M agree, and the collapsed branch still counts.
        assert_eq!(flat.total_keys, 3);
        assert_eq!(flat.shown_keys, 3);
    }

    /// Truncation is reported, never silent — a capped list that looks
    /// complete is a lie about coverage.
    #[test]
    fn the_row_cap_reports_what_it_dropped() {
        let keys: Vec<String> = (0..50).map(|i| format!("demo/k{i}")).collect();
        let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        let snap = snapshot(&refs);
        let flat = flat_now(&snap, "", &expand(&["demo"]), 10);
        assert_eq!(flat.rows.len(), 10);
        assert_eq!(flat.truncated, 41, "1 root + 50 leaves, 10 shown");
        assert_eq!(flat.total_keys, 50, "the denominator ignores the cap");
    }

    /// The three "we know nothing" states must not read alike.
    #[test]
    fn registration_labels_distinguish_not_asked_from_not_registered() {
        assert_eq!(registration_label(&Registration::Unknown), "—");
        assert_eq!(
            registration_label(&Registration::Unregistered),
            "unregistered"
        );
        assert_eq!(
            registration_label(&Registration::NoSliceForProducer),
            "no slice"
        );
        assert_ne!(
            tone(&Registration::Unknown),
            tone(&Registration::Unregistered)
        );
    }

    // --- Tree v2 (issue #65) -------------------------------------------

    #[test]
    fn fuzzy_match_is_ordered_subsequence_case_insensitive() {
        assert!(fuzzy_match("v1/h-abc/telemetry/sysinfo/cpu", "cpu"));
        assert!(fuzzy_match("v1/h-abc/telemetry/sysinfo/cpu", "TELcpu"));
        assert!(fuzzy_match("anything", ""));
        assert!(!fuzzy_match("v1/h-abc/telemetry", "cpu"));
        assert!(!fuzzy_match("abc", "acb"));
    }

    /// Search hides nothing silently: the filtered view carries both counts,
    /// and matching keys render with their full ancestor chain expanded.
    #[test]
    fn search_shows_matches_with_ancestors_and_counts_the_rest() {
        let snap = snapshot(&[
            "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu",
            "v1/h-3fa9c2d41b7e/telemetry/sysinfo/mem",
            "v1/h-3fa9c2d41b7e/state/sysinfo/health",
        ]);
        let flat = search_flatten(&snap, "", "cpu", 1000, Instant::now());
        assert!(flat.filtered);
        assert_eq!(flat.shown_keys, 1);
        assert_eq!(flat.total_keys, 3);
        let paths: Vec<&str> = flat.rows.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "v1",
                "v1/h-3fa9c2d41b7e",
                "v1/h-3fa9c2d41b7e/telemetry",
                "v1/h-3fa9c2d41b7e/telemetry/sysinfo",
                "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu",
            ],
            "the match renders under its expanded ancestors, nothing else"
        );
        // A query matching nothing keeps the honest counts.
        let none = search_flatten(&snap, "", "zzz", 1000, Instant::now());
        assert_eq!(none.rows.len(), 0);
        assert_eq!((none.shown_keys, none.total_keys), (0, 3));
    }

    /// Pivot by producer: host keys group under the producer; a service
    /// origin (no producer chunk) groups under the `@…` origin, which is the
    /// service (RFC 03 §1.5); foreign keys under "(foreign)".
    #[test]
    fn producer_pivot_groups_hosts_services_and_foreigners() {
        let snap = snapshot(&[
            "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu",
            "v1/h-9fa9c2d41b70/telemetry/sysinfo/cpu",
            "v1/@catalog/state/entity/x",
            "demo/example/foo",
        ]);
        let flat = pivot_flatten(
            &snap,
            "",
            Pivot::Producer,
            &BTreeSet::new(),
            "",
            10_000,
            Instant::now(),
        );
        let tops: Vec<&str> = flat
            .rows
            .iter()
            .filter(|r| r.depth == 0)
            .map(|r| r.chunk.as_str())
            .collect();
        assert_eq!(tops, ["(foreign)", "@catalog", "sysinfo"]);
        // The sysinfo group aggregates both origins' traffic.
        let sysinfo = flat.rows.iter().find(|r| r.chunk == "sysinfo").unwrap();
        assert_eq!(sysinfo.subtree_count, 2);
        assert_eq!(sysinfo.subtree_keys, 2);
        assert!(
            sysinfo.target.is_none(),
            "a pivot group has no wire subtree"
        );
        assert_eq!(flat.total_keys, 4);
    }

    /// Pivot by origin: the service origin is a first-class group next to
    /// hosts (the issue's acceptance criterion).
    #[test]
    fn origin_pivot_promotes_service_origins() {
        let snap = snapshot(&[
            "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu",
            "v1/@catalog/state/entity/x",
        ]);
        let flat = pivot_flatten(
            &snap,
            "",
            Pivot::Origin,
            &BTreeSet::new(),
            "",
            10_000,
            Instant::now(),
        );
        let tops: Vec<&str> = flat
            .rows
            .iter()
            .filter(|r| r.depth == 0)
            .map(|r| r.chunk.as_str())
            .collect();
        assert_eq!(tops, ["@catalog", "h-3fa9c2d41b7e"]);
        assert_eq!(
            flat.rows
                .iter()
                .filter(|r| r.depth == 0)
                .map(|r| r.role)
                .collect::<Vec<_>>(),
            [Some(Role::Origin), Some(Role::Origin)]
        );
    }

    /// Pivot leaves keep the real wire path as their action target, so
    /// select/watch built from a pivot row acts on the real subtree.
    #[test]
    fn pivot_leaves_target_their_real_path() {
        let snap = snapshot(&["v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu"]);
        let flat = pivot_flatten(
            &snap,
            "",
            Pivot::Producer,
            &BTreeSet::new(),
            "cpu", // filter also auto-expands
            10_000,
            Instant::now(),
        );
        let leaf = flat.rows.iter().find(|r| r.chunk == "cpu").expect("leaf");
        assert_eq!(
            leaf.target.as_deref(),
            Some("v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu")
        );
        assert_eq!((flat.shown_keys, flat.total_keys), (1, 1));
    }

    /// A key that is both a concrete key and a prefix of deeper keys is its
    /// own entry — pivoting must not lose its traffic to its children.
    #[test]
    fn a_prefix_key_with_children_is_its_own_entry() {
        let snap = snapshot(&[
            "v1/h-3fa9c2d41b7e/state/tc/iface",
            "v1/h-3fa9c2d41b7e/state/tc/iface/eth0",
        ]);
        let flat = pivot_flatten(
            &snap,
            "",
            Pivot::Origin,
            &BTreeSet::new(),
            "",
            10_000,
            Instant::now(),
        );
        assert_eq!(flat.total_keys, 2, "prefix key and deep key both count");
    }

    /// The soak numbers behind issue #65's acceptance: flatten, search and
    /// pivot over a 50k-key tree. Run with
    /// `cargo test -p zengui --release -- --ignored soak_flatten --nocapture`.
    #[test]
    #[ignore = "soak bench — run explicitly, read the printed numbers"]
    fn soak_flatten_50k_keys() {
        let keys: Vec<String> = (0..50_000)
            .map(|i| format!("v1/h-3fa9c2d41b7e/telemetry/synth/g{}/k{i}", i / 100))
            .collect();
        let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        let snap = snapshot(&refs);
        // Everything expanded — the worst case the virtual window renders.
        let mut all = BTreeSet::new();
        for k in &refs {
            let mut acc = String::new();
            for chunk in k.split('/') {
                acc = if acc.is_empty() {
                    chunk.to_string()
                } else {
                    format!("{acc}/{chunk}")
                };
                all.insert(acc.clone());
            }
        }
        let now = Instant::now();

        let t = Instant::now();
        let flat = flatten(&snap, "", &all, 60_000, now);
        println!(
            "flatten (expanded): {} rows in {:?}",
            flat.rows.len(),
            t.elapsed()
        );
        assert_eq!(flat.total_keys, 50_000);

        let t = Instant::now();
        let found = search_flatten(&snap, "", "g250k", 60_000, now);
        println!(
            "search:            {} of {} in {:?}",
            found.shown_keys,
            found.total_keys,
            t.elapsed()
        );

        let t = Instant::now();
        let piv = pivot_flatten(&snap, "", Pivot::Producer, &all, "", 60_000, now);
        println!(
            "pivot (producer):  {} rows in {:?}",
            piv.rows.len(),
            t.elapsed()
        );
        assert_eq!(piv.total_keys, 50_000);

        // The frame cost is the window, not the row count.
        let (first, last) = window(flat.rows.len(), 25_000.0 * ROW_HEIGHT, 900.0);
        println!(
            "window at mid-scroll: rows {first}..{last} of {}",
            flat.rows.len()
        );
        assert!(last - first < 100);
    }

    /// The scroll window is exact row arithmetic with overscan, clamped.
    #[test]
    fn the_window_is_bounded_and_covers_the_viewport() {
        let (first, last) = window(50_000, 0.0, 600.0);
        assert_eq!(first, 0);
        assert!(
            (26..=60).contains(&last),
            "viewport rows + overscan: {last}"
        );

        let (first, last) = window(50_000, 25_000.0 * ROW_HEIGHT, 600.0);
        assert!((25_000 - OVERSCAN..=25_000).contains(&first));
        assert!(last > 25_000);

        let (first, last) = window(10, 1_000_000.0, 600.0);
        assert_eq!((first, last), (10, 10), "clamped past the end");
    }
}
