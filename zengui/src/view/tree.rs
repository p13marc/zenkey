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
use std::sync::Arc;
use std::time::Instant;

use iced::widget::{Column, button, column, row, text};
use iced::{Element, Length};
use zenkey_fleet::KeyTreeSnapshot;
use zenkey_fleet::skeleton::{DeclRef, MergedNode, NodeStats, NodeStatus};

use crate::keyfacts::Registration;
use crate::message::{Message, Subject, SubjectMsg, WorkspaceMsg};
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

    /// Short stable token: expansion paths are namespaced per pivot, so a
    /// path opened under one pivot never accidentally opens a group that
    /// happens to share its spelling under another.
    pub fn key(self) -> &'static str {
        match self {
            Pivot::Chunks => "chunks",
            Pivot::Origin => "origin",
            Pivot::Producer => "producer",
            Pivot::Class => "class",
        }
    }
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

/// The half of a row that changes only when the tree's *structure* does
/// (#177).
///
/// ## Why these ten fields and not the other eight
///
/// The claim this whole cache rests on is that a row's **shape does not depend
/// on any live number**, and it is checkable rather than plausible.
/// `zenkey_fleet::skeleton::merge_nodes` computes `status` from *set
/// membership* — does an observed node exist, is it covered by a watch — never
/// from a magnitude; and the child-name set it walks is the union of the
/// skeleton's names and the observed ones. Depth, chunk, path, target, role and
/// the declared type all come from names.
///
/// [`TreeRow::is_leaf`] is the one that looks live and is not, and it earns its
/// place here twice over. It is `count > 0`, but a `StatsTable` entry exists
/// only once a sample has arrived and an interior node takes `count = 0` from
/// `TreeNode::default()` — so a count going 0→1 *is* a new key, which moves
/// `tick.keys`, which invalidates this cache. And `row_press` branches on it to
/// choose select-versus-toggle: recomputing it per frame would change what a
/// click means under the cursor.
///
/// The same argument covers `is_entry`, and therefore `shown_keys` and
/// `total_keys`.
#[derive(Debug, Clone, PartialEq)]
pub struct RowShape {
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
    /// Traffic landed on exactly this key. Structural despite being derived
    /// from a count — see the type docs.
    pub is_leaf: bool,
    /// `None` for any chunk outside a recognised v1 subtree.
    pub role: Option<Role>,
    /// The registry-declared payload type, when the skeleton knows one.
    pub decl_type: Option<String>,
}

/// Where a drawn row's numbers come from (#177).
#[derive(Debug, Clone)]
enum Numbers {
    /// This tick's snapshot, held by `Arc` so the frame's read cannot be
    /// swapped out from under it by the engine's `ArcSwap`. A drawn row looks
    /// itself up by wire path — O(depth) for the ~40 rows in the window, and
    /// nothing at all for the 49,960 nobody can see.
    Live(Arc<KeyTreeSnapshot>),
    /// Numbers taken when the shape was built, index-parallel to `rows`. What
    /// a bare `flatten` returns, since it has no snapshot to point at.
    Frozen(Vec<Option<NodeStats>>),
}

/// Whether [`RowShape::path`] is a wire path a snapshot can answer for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathKind {
    Wire,
    /// A pivot grouping. Its aggregates are over a synthetic set that is not a
    /// contiguous wire subtree, so no snapshot can be asked about it — which is
    /// why [`Flattened::retarget`] refuses these rather than showing last
    /// tick's figures as if they were now.
    Pivot,
}

/// One rendered line of the tree — a per-frame temporary of ~40 since #177,
/// not a per-tick allocation of 50,000.
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
///
/// Since #177 `rows` carries only the *shape*; the numbers come from
/// [`Flattened::row`], joined at draw time against whichever source
/// [`Flattened::retarget`] last pointed this set at.
///
/// No `PartialEq`: `KeyTreeSnapshot` has none, and nothing in the workspace
/// compares two of these. A hand-written impl over `rows` and the four counters
/// would have to explain why it ignores the numbers source, which is a sentence
/// with no reader.
#[derive(Debug, Clone)]
pub struct Flattened {
    pub rows: Vec<RowShape>,
    /// Rows beyond the cap. Reported, never silently dropped.
    pub truncated: usize,
    /// Concrete entries surviving the filter (== `total_keys` unfiltered).
    pub shown_keys: usize,
    /// All concrete entries the tree holds.
    pub total_keys: usize,
    /// Whether a find-in-tree filter is active.
    pub filtered: bool,
    numbers: Numbers,
    paths: PathKind,
    /// What `age_s` is measured against. Refreshed by [`Self::retarget`], i.e.
    /// once per tick — deliberately not per frame. Making the freshness dot
    /// update at 60 Hz would be a behaviour change smuggled in under a
    /// performance commit, and it would make the dot untestable.
    now: Instant,
}

impl Flattened {
    pub fn empty() -> Flattened {
        Flattened {
            rows: Vec::new(),
            truncated: 0,
            shown_keys: 0,
            total_keys: 0,
            filtered: false,
            numbers: Numbers::Frozen(Vec::new()),
            paths: PathKind::Wire,
            now: Instant::now(),
        }
    }

    /// One drawn row: shape joined to whatever numbers are current (#177).
    ///
    /// Panics on an out-of-range index, like the slice indexing it replaced.
    pub fn row(&self, i: usize) -> TreeRow {
        let shape = &self.rows[i];
        let stats = match &self.numbers {
            Numbers::Frozen(v) => v[i],
            // A skeleton-only symbolic chunk keys as `{mount}` and can never
            // match an observed child, so it reads `None` — which is exactly
            // what the merge gave it.
            Numbers::Live(tree) => {
                let chunks: Vec<&str> = shape.path.split('/').collect();
                tree.node(&chunks).map(NodeStats::from_tree)
            }
        };
        TreeRow {
            depth: shape.depth,
            chunk: shape.chunk.clone(),
            path: shape.path.clone(),
            target: shape.target.clone(),
            has_children: shape.has_children,
            expanded: shape.expanded,
            status: shape.status,
            is_leaf: shape.is_leaf,
            count: stats.map(|s| s.count).unwrap_or(0),
            bytes: stats.map(|s| s.bytes).unwrap_or(0),
            rate_hz: stats.map(|s| s.rate_hz).unwrap_or(0.0),
            subtree_count: stats.map(|s| s.subtree_count).unwrap_or(0),
            subtree_bytes: stats.map(|s| s.subtree_bytes).unwrap_or(0),
            subtree_keys: stats.map(|s| s.subtree_keys).unwrap_or(0),
            subtree_rate_hz: stats.map(|s| s.subtree_rate_hz).unwrap_or(0.0),
            age_s: age_of(stats.as_ref(), self.now),
            role: shape.role,
            decl_type: shape.decl_type.clone(),
        }
    }

    /// Point this shape at a new tick's numbers, in O(1) (#177).
    ///
    /// `false` when the rows carry pivot aggregates: no snapshot can answer for
    /// a synthetic grouping, and showing last tick's figures as if they were now
    /// would be the kind of quiet lie this codebase files as a defect. The
    /// caller rebuilds instead.
    pub fn retarget(&mut self, tree: Arc<KeyTreeSnapshot>, now: Instant) -> bool {
        if self.paths == PathKind::Pivot {
            return false;
        }
        self.numbers = Numbers::Live(tree);
        self.now = now;
        true
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
    let mut stats = Vec::new();
    let mut truncated = 0;
    let mut total_keys = 0;
    walk(
        merged,
        &mut Ctx {
            expanded,
            max_rows,
            rows: &mut rows,
            stats: &mut stats,
            truncated: &mut truncated,
            total_keys: &mut total_keys,
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
        // Frozen, because a bare `flatten` has no snapshot to point at. The
        // app calls `retarget` immediately afterwards, which flips wire shapes
        // to live; a test that does not gets the numbers as of the walk, which
        // is what it got before #177.
        numbers: Numbers::Frozen(stats),
        paths: PathKind::Wire,
        now,
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
    rows: &'a mut Vec<RowShape>,
    /// Index-parallel to `rows` — the numbers as of this walk, for a caller
    /// that never retargets (#177).
    stats: &'a mut Vec<Option<NodeStats>>,
    truncated: &'a mut usize,
    total_keys: &'a mut usize,
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
            ctx.rows.push(RowShape {
                depth,
                chunk: chunk.clone(),
                path: child_path.clone(),
                target: Some(child_path.clone()),
                has_children: !child.children.is_empty(),
                expanded,
                status: child.status,
                is_leaf: stats.map(|s| s.count > 0).unwrap_or(false),
                role,
                decl_type: child.decl.as_ref().map(|d| d.type_name.clone()),
            });
            ctx.stats.push(stats);
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
    // Two passes, and the reason is #249: retention is decided *bottom-up* —
    // a node renders because something beneath it matched — so at push time
    // the first pass does not yet know whether a row counts against the cap.
    // The old single pass resolved that by building every row and dropping
    // most of them again, which on a 40k-key tree is ~160,000 `String`
    // allocations per keystroke, and left the cap to a `truncate` afterwards,
    // so the memory bound `app.rs` documents did not hold here at all.
    //
    // Pass one decides and allocates nothing but the verdicts: one reusable
    // path buffer for the descent — the trick `skeleton::merge` already uses,
    // and that `docs/zero-copy.md` §2 names — and one `bool` per node in
    // pre-order. Pass two walks the identical order and builds only what it
    // keeps, honouring the cap *during* the walk exactly as `flatten` does.
    let mut keep: Vec<Mark> = Vec::new();
    let mut shown = 0usize;
    let mut total = 0usize;
    let mut probe = String::new();
    search_mark(
        merged,
        &mut probe,
        start_expect(base),
        query,
        &mut keep,
        &mut shown,
        &mut total,
    );

    let mut rows = Vec::new();
    let mut stats = Vec::new();
    let mut truncated = 0usize;
    let mut at = 0usize;
    search_emit(
        merged,
        String::new(),
        0,
        start_expect(base),
        &keep,
        &mut at,
        &mut Emit {
            max_rows,
            rows: &mut rows,
            stats: &mut stats,
            truncated: &mut truncated,
        },
    );
    Flattened {
        rows,
        truncated,
        shown_keys: shown,
        total_keys: total,
        // Computed, not asserted. `search_flatten` used to hardcode `true`,
        // and `filtered` feeds an empty-state sentence and a coverage line —
        // an honesty field set by assumption is the thing this crate does not
        // do (RFC 09 §5.1 O4).
        filtered: !query.is_empty(),
        numbers: Numbers::Frozen(stats),
        paths: PathKind::Wire,
        now,
    }
}

/// One node's verdict from pass one, in pre-order.
#[derive(Debug, Clone, Copy, Default)]
struct Mark {
    /// This node or something beneath it matched, so it renders.
    keep: bool,
    /// Pre-order slots this node's subtree occupies. A dropped branch is
    /// stepped over with this rather than re-walked — which on the common
    /// no-match query is the whole tree.
    slots: usize,
}

/// What pass two writes into. Deliberately *not* [`Ctx`]: that carries
/// `expanded` and `total_keys`, which a search decides in pass one and must not
/// be able to touch again here.
struct Emit<'a> {
    max_rows: usize,
    rows: &'a mut Vec<RowShape>,
    stats: &'a mut Vec<Option<NodeStats>>,
    truncated: &'a mut usize,
}

/// Pass one: which nodes are retained, in pre-order, allocating nothing.
///
/// Returns whether this subtree contains any match, so ancestors render.
/// `probe` is one buffer reused for the whole descent rather than a `String`
/// per node — the path is needed for `fuzzy_match` and for nothing else here.
fn search_mark(
    node: &MergedNode,
    probe: &mut String,
    expect: Expect,
    query: &str,
    keep: &mut Vec<Mark>,
    shown: &mut usize,
    total: &mut usize,
) -> bool {
    let mut any = false;
    for (chunk, child) in &node.children {
        let base_len = probe.len();
        if base_len > 0 {
            probe.push('/');
        }
        probe.push_str(chunk);
        let (_, next_expect) = classify(chunk, expect);
        let entry = is_entry(child);
        if entry {
            *total += 1;
        }
        let self_match = entry && fuzzy_match(probe, query);
        if self_match {
            *shown += 1;
        }
        // The verdict is not known until the recursion returns, so reserve
        // this node's slot at its pre-order index and fill it after — along
        // with how many slots its subtree took, so pass two can step over a
        // dropped branch in O(1) instead of walking it to find out.
        let at = keep.len();
        keep.push(Mark::default());
        let below = search_mark(child, probe, next_expect, query, keep, shown, total);
        keep[at] = Mark {
            keep: self_match || below,
            slots: keep.len() - at - 1,
        };
        any |= keep[at].keep;
        probe.truncate(base_len);
    }
    any
}

/// Pass two: build the retained rows, bounded by the cap during the walk.
///
/// Walks the identical pre-order as [`search_mark`] — `BTreeMap` iteration is
/// deterministic — so one counter keeps the two in lockstep.
fn search_emit(
    node: &MergedNode,
    path: String,
    depth: usize,
    expect: Expect,
    keep: &[Mark],
    at: &mut usize,
    ctx: &mut Emit<'_>,
) {
    for (chunk, child) in &node.children {
        let (role, next_expect) = classify(chunk, expect);
        let mark = keep[*at];
        *at += 1;
        if !mark.keep {
            // Nothing below a dropped node is kept, and pass one already
            // counted how many slots to step over — so a dropped branch costs
            // one addition rather than a walk to find its size. On the common
            // no-match query that branch is the whole tree.
            *at += mark.slots;
            continue;
        }
        let child_path = if path.is_empty() {
            chunk.clone()
        } else {
            format!("{path}/{chunk}")
        };
        if ctx.rows.len() >= ctx.max_rows {
            *ctx.truncated += 1;
        } else {
            ctx.rows.push(RowShape {
                depth,
                chunk: chunk.clone(),
                path: child_path.clone(),
                target: Some(child_path.clone()),
                has_children: !child.children.is_empty(),
                // A search shows the chain to every match, so an ancestor of a
                // match is open by definition.
                expanded: true,
                status: child.status,
                is_leaf: child.stats.map(|s| s.count > 0).unwrap_or(false),
                role,
                decl_type: child.decl.as_ref().map(|d| d.type_name.clone()),
            });
            ctx.stats.push(child.stats);
        }
        search_emit(child, child_path, depth + 1, next_expect, keep, at, ctx);
    }
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
    /// The real wire-path prefix this group stands for, when its constraint
    /// set is exactly contiguous (issue #93) — what watch/select act on.
    target: Option<String>,
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

    // Chunks before the origin position in a conforming real path (base +
    // the `v1` chunk) — what group targets are reconstructed relative to.
    let skip = if base.is_empty() {
        1
    } else {
        base.split('/').count() + 1
    };
    let mut root = PNode::default();
    for e in &entries {
        let chunks = pivot_chunks(e, pivot, skip);
        let mut node = &mut root;
        for (chunk, role, target) in &chunks {
            node = node.children.entry(chunk.clone()).or_default();
            node.role = *role;
            if node.target.is_none() {
                node.target = target.clone();
            }
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
    let mut stats = Vec::new();
    let mut truncated = 0usize;
    flatten_pnode(
        &root,
        &mut PivotCtx {
            expanded,
            auto_expand: filtered,
            pivot_key: pivot.key(),
            emit: Emit {
                max_rows,
                rows: &mut rows,
                stats: &mut stats,
                truncated: &mut truncated,
            },
        },
        String::new(),
        0,
    );
    Flattened {
        rows,
        truncated,
        shown_keys,
        total_keys,
        filtered,
        numbers: Numbers::Frozen(stats),
        // Always frozen: a pivot group's numbers are aggregates over a set the
        // wire has no path for, so `retarget` refuses them (#177).
        paths: PathKind::Pivot,
        now,
    }
}

/// The re-keyed chunk list for one entry: `(display chunk, role, target)`.
///
/// The target is the **real wire-path prefix** the pivot node stands for,
/// present exactly when the node's constraint set is contiguous on the wire
/// (issue #93): coordinates accumulated so far must form a prefix of
/// `origin / class / producer`, and subject chunks require every coordinate
/// the entry has. Groups that genuinely span a wildcard (a producer group
/// across origins, a class group across origins) get `None` — offering a
/// watch there would hide a fleet-wide cost behind one click.
fn pivot_chunks(
    e: &PivotEntry,
    pivot: Pivot,
    skip: usize,
) -> Vec<(String, Option<Role>, Option<String>)> {
    let real: Vec<&str> = e.real_path.split('/').collect();
    let origin = e.origin.as_deref();
    let class = e.class.as_deref();
    let producer = e.producer.as_deref();
    let foreign = origin.is_none() && class.is_none();
    if foreign {
        // Foreign keys keep their raw order, so every level below the
        // "(foreign)" group is a real prefix.
        let mut out: Vec<(String, Option<Role>, Option<String>)> =
            vec![("(foreign)".to_string(), None, None)];
        out.extend(e.tail.iter().enumerate().map(|(i, c)| {
            (
                c.clone(),
                None,
                Some(real[..(i + 1).min(real.len())].join("/")),
            )
        }));
        return out;
    }

    // (chunk, role) first, targets derived below from the constraint chain.
    let mut chunks: Vec<(String, Option<Role>)> = Vec::new();
    let push = |out: &mut Vec<(String, Option<Role>)>, v: Option<&str>, role: Role| {
        if let Some(v) = v {
            out.push((v.to_string(), Some(role)));
        }
    };
    match pivot {
        Pivot::Chunks => unreachable!("chunk order uses flatten()"),
        Pivot::Origin => {
            push(&mut chunks, origin, Role::Origin);
            push(&mut chunks, class, Role::Class);
            push(&mut chunks, producer, Role::Producer);
        }
        Pivot::Producer => {
            // The producer when there is one; the service origin otherwise —
            // for a service origin the service IS the producer (RFC 03 §1.5).
            match (producer, origin) {
                (Some(p), _) => chunks.push((p.to_string(), Some(Role::Producer))),
                (None, Some(o)) => chunks.push((o.to_string(), Some(Role::Origin))),
                (None, None) => chunks.push(("(no producer)".to_string(), None)),
            }
            if producer.is_some() {
                push(&mut chunks, origin, Role::Origin);
            }
            push(&mut chunks, class, Role::Class);
        }
        Pivot::Class => {
            push(&mut chunks, class, Role::Class);
            push(&mut chunks, origin, Role::Origin);
            push(&mut chunks, producer, Role::Producer);
        }
    }
    chunks.extend(e.tail.iter().map(|c| (c.clone(), Some(Role::Subject))));
    if chunks.is_empty() {
        // An entry that *is* one of the coordinates (e.g. a declared class
        // position with no subject below it yet).
        chunks.push(("(…)".to_string(), None));
    }

    // Constraint chain → target per level.
    let coords_in_entry = usize::from(origin.is_some())
        + usize::from(class.is_some())
        + usize::from(producer.is_some());
    let (mut has_o, mut has_c, mut has_p, mut tail_len) = (false, false, false, 0usize);
    chunks
        .into_iter()
        .map(|(chunk, role)| {
            match role {
                Some(Role::Origin) => has_o = true,
                Some(Role::Class) => has_c = true,
                Some(Role::Producer) => has_p = true,
                Some(Role::Subject) | Some(Role::BlobTier) => tail_len += 1,
                Some(Role::Version) | None => tail_len += 1,
            }
            let present = usize::from(has_o) + usize::from(has_c) + usize::from(has_p);
            let contiguous = if tail_len > 0 {
                present == coords_in_entry
            } else {
                has_o && (!has_c || has_o) && (!has_p || has_c)
            };
            let target = (contiguous && present > 0)
                .then(|| real[..(skip + present + tail_len).min(real.len())].join("/"));
            (chunk, role, target)
        })
        .collect()
}

/// The pivot walk's invariants, and where it writes (#250).
///
/// The shape [`Ctx`] gives [`walk`]: what does not change during the descent
/// travels in the context, so only what does — node, path, depth — stays a
/// parameter. `flatten_pnode` carried `expanded`, `auto_expand` and `pivot_key`
/// loose instead, which is the whole reason it carried clippy's
/// argument-count allow.
///
/// **This retires a count, not a defect.** Unlike `pane`'s two adjacent
/// `&BTreeSet<String>` watch sets, `flatten_pnode`'s eight parameters were six
/// distinct types with nothing transposable among them.
///
/// It embeds [`Emit`] rather than restating four sink fields, so "what a
/// row-emitting pass writes into" keeps one spelling — and so `search_emit`
/// still cannot reach `expanded`, which is what `Emit` exists to withhold.
/// Extending `Emit` with `expanded` instead would have handed `search_emit`
/// exactly the field its own doc comment refuses it.
struct PivotCtx<'a> {
    /// Paths the user has opened.
    expanded: &'a BTreeSet<String>,
    /// A filtered pivot opens everything, so the matches are visible.
    auto_expand: bool,
    /// The active pivot's key, which prefixes every synthetic path.
    pivot_key: &'a str,
    emit: Emit<'a>,
}

/// The cap is enforced **during** the walk, as `flatten` does it (#249). It
/// used to be a `truncate` afterwards, so `truncated` reported rows that had
/// been built and dropped while the string beside it said "not built".
///
/// What this does *not* bound, and should not: `collect_entries` yields one
/// entry per concrete key, and that population is bounded by the key table,
/// which the status strip already reports as `keys_evicted`. Two bounds, two
/// counters, two sentences.
fn flatten_pnode(node: &PNode, ctx: &mut PivotCtx<'_>, path: String, depth: usize) {
    for (chunk, child) in &node.children {
        let child_path = if path.is_empty() {
            format!("pivot:{}:{chunk}", ctx.pivot_key)
        } else {
            format!("{path}/{chunk}")
        };
        let is_open = ctx.auto_expand || ctx.expanded.contains(&child_path);
        let own = child.leaf.as_ref().and_then(|(_, s, _)| *s);
        if ctx.emit.rows.len() >= ctx.emit.max_rows {
            *ctx.emit.truncated += 1;
            // Still descend: a collapsed count would understate the tree, and
            // the rows below are counted the same way.
            if is_open {
                flatten_pnode(child, ctx, child_path, depth + 1);
            }
            continue;
        }
        ctx.emit.rows.push(RowShape {
            depth,
            chunk: chunk.clone(),
            path: child_path.clone(),
            // A concrete entry acts on its real path; a group acts on its
            // contiguous wire prefix when it has one (issue #93).
            target: child
                .leaf
                .as_ref()
                .map(|(p, _, _)| p.clone())
                .or_else(|| child.target.clone()),
            has_children: !child.children.is_empty(),
            expanded: is_open,
            status: child
                .status
                .unwrap_or(NodeStatus::DeclaredOnly(Default::default())),
            is_leaf: own.map(|s| s.count > 0).unwrap_or(false),
            role: child.role,
            decl_type: child
                .leaf
                .as_ref()
                .and_then(|(_, _, d)| d.as_ref().map(|d| d.type_name.clone())),
        });
        // The row's own numbers are the group's *aggregates*, which is why
        // these can never be retargeted: `agg_*` sums a synthetic membership,
        // and no wire path names it.
        ctx.emit.stats.push(Some(NodeStats {
            count: own.map(|s| s.count).unwrap_or(0),
            bytes: own.map(|s| s.bytes).unwrap_or(0),
            rate_hz: own.map(|s| s.rate_hz).unwrap_or(0.0),
            subtree_count: child.agg_count,
            subtree_bytes: child.agg_bytes,
            subtree_rate_hz: child.agg_rate,
            subtree_keys: child.agg_keys,
            subtree_last_seen: child.agg_last,
        }));
        if is_open {
            flatten_pnode(child, ctx, child_path, depth + 1);
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

/// Registration lookup by wire key. The engine's bounded projection cache
/// rather than a plain map (issue #107): a `HashMap` here grew one entry per
/// distinct key *ever* observed, shadowing a key table that has been bounded
/// and counting its evictions since the bootstrap. Aliased so every pane and
/// row signature below reads the same, and so `facts.get(key)` stays a pure
/// `&self` read from the render path.
pub type FactsIndex = zenkey_fleet::FactsCache;

/// The tree's two watch sets — same type, opposite meanings (#250).
///
/// They were adjacent `&BTreeSet<String>` parameters at **three** call layers —
/// [`pane`], `tree_view` and `row_view`, which is where both are actually
/// read: one picks "seeding…" over "quiet", the other ◉ over ○. Transposing
/// them compiled, and the tree then drew every watched subtree as still
/// seeding and every seeding one as settled.
///
/// Named fields make that a type error at all three. Two flat fields on
/// [`TreeData`] would have fixed only the outermost — `row_view` takes six
/// arguments, so clippy never flagged it and it is invisible to #250's own
/// acceptance grep.
#[derive(Clone, Copy)]
pub struct Watches<'a> {
    /// Subtrees this app watches: the ◉/○ toggle on each row.
    pub mine: &'a BTreeSet<String>,
    /// Watches whose seed phase has not resolved yet (issue #92) — the
    /// "seeding…" badge, which must never read as "quiet".
    pub seeding: &'a BTreeSet<String>,
}

/// What the app hands the tree pane (#250, on `DetailData`'s precedent).
///
/// Ordered by **where each field comes from**, not by where it is drawn: the
/// tree's own display state, then what has been observed about it, then what
/// the user chose. That is a defensible order on its own terms, and #175 —
/// which splits `Zengui` along those same seams — is the corroboration: each
/// run below becomes one sub-state's prefix, so the call site stays one
/// literal.
///
/// A builder was rejected. The test call sites want a literal, and a literal
/// missing a field is a compile error where a builder missing a `.with_…` is a
/// silent default — and these are honesty inputs, where a default is a claim
/// nobody made.
pub struct TreeData<'a> {
    /// The flattened rows and their numbers, as of the last rebuild (#177).
    pub flat: &'a Flattened,
    /// How the tree groups its entries (issue #65).
    pub pivot: Pivot,
    /// The find-in-tree query; empty = no filter.
    pub search: &'a str,
    /// Scroll offset, and…
    pub scroll_y: f32,
    /// …the viewport height: together, the virtual window.
    pub viewport_h: f32,
    /// Per-key projections, for the row badges.
    pub facts: &'a FactsIndex,
    /// The pair this struct exists for.
    pub watches: Watches<'a>,
    /// The selected wire key, if any.
    pub selected: Option<&'a str>,
}

/// What clicking the row body does (issue #93): concrete entries select
/// (detail fetch acts on `target`); groups toggle. Pure, so it is testable
/// without a renderer.
/// Takes the **shape**, not the row: what a click means must not depend on a
/// number that moved this tick (#177). `is_leaf` is structural for exactly
/// that reason — see [`RowShape`].
pub fn row_press(r: &RowShape) -> Message {
    match (&r.target, r.is_leaf || !r.has_children) {
        (Some(t), true) => Message::Subject(SubjectMsg::Select(Subject::Key(t.clone()))),
        _ => Message::Workspace(WorkspaceMsg::ToggleNode(r.path.clone())),
    }
}

/// What clicking the expand marker does — its own affordance, so a concrete
/// key that is also a prefix of deeper keys stays selectable (issue #93).
pub fn marker_press(r: &RowShape) -> Option<Message> {
    r.has_children
        .then(|| Message::Workspace(WorkspaceMsg::ToggleNode(r.path.clone())))
}

/// Whether a row sits at or under one of the still-seeding watch paths.
fn under_seeding(row_path: &str, seeding: &BTreeSet<String>) -> bool {
    seeding
        .iter()
        .any(|p| row_path == p || row_path.starts_with(&format!("{p}/")))
}

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

/// Render the rows.
///
/// Takes the same [`TreeData`] as [`pane`], which formally hands it `pivot` and
/// `search` that it does not use. That is the trade: the two functions overlap
/// on five arguments, and one struct is what stops them drifting apart — this
/// one sat at exactly seven arguments, one below the lint, so the next field
/// the tree pane needs would have re-earned the allow that chunk AL deleted.
fn tree_view<'a>(d: TreeData<'a>) -> Element<'a, Message> {
    let flat = d.flat;
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

    let (first, last) = window(flat.rows.len(), d.scroll_y, d.viewport_h);
    let mut col = Column::new();
    if first > 0 {
        col = col.push(iced::widget::Space::new().height(Length::Fixed(first as f32 * ROW_HEIGHT)));
    }
    // The one place a `TreeRow` is built now: ~40 per frame, joined against
    // this tick's numbers, rather than 50,000 per tick of which the window
    // draws forty (#177).
    for i in first..last {
        let shape = &flat.rows[i];
        let r = flat.row(i);
        col = col.push(
            iced::widget::container(row_view(shape, &r, d.facts, d.selected, d.watches))
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
            Message::Workspace(WorkspaceMsg::TreeScrolled(
                viewport.absolute_offset().y,
                viewport.bounds().height,
            ))
        })
        .into()
}

/// Two halves of one row, and the split is the point (#177).
///
/// `shape` decides **interaction** — what a click means — and comes from the
/// cache, so it cannot change under the cursor because a number moved. `r`
/// carries the numbers, joined against this tick's snapshot.
///
/// Neither is borrowed by the returned `Element`: every use below is a clone or
/// a `Copy` field, and the one that looks like a borrow (`facts.get`) borrows
/// `facts`. That is what lets `tree_view` hand these in as per-frame
/// temporaries.
fn row_view<'a>(
    shape: &RowShape,
    r: &TreeRow,
    facts: &'a FactsIndex,
    selected: Option<&'a str>,
    watches: Watches<'a>,
) -> Element<'a, Message> {
    let indent = iced::widget::Space::new().width(Length::Fixed(r.depth as f32 * 14.0));

    // The expand marker is its own affordance (issue #93): a concrete key
    // that is also a prefix of deeper keys keeps body-click = select.
    let marker: Element<'a, Message> = match marker_press(shape) {
        Some(msg) => button(text(if r.expanded { "▾" } else { "▸" }).size(font::CAPTION))
            .padding(2)
            .style(button::text)
            .on_press(msg)
            .into(),
        None => iced::widget::Space::new().width(Length::Fixed(18.0)).into(),
    };

    let is_selected = r.target.is_some() && selected == r.target.as_deref();
    let name = text(r.chunk.clone())
        .size(font::CAPTION)
        .font(iced::Font::MONOSPACE)
        .style(move |theme: &iced::Theme| text::Style {
            color: Some(if is_selected {
                colors(theme).primary()
            } else {
                colors(theme).text()
            }),
        });

    let mut line = row![name]
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
            // While the watch's seed phase is still running, "quiet" is not
            // yet an observation — the seed may still deliver (issue #92).
            let check = r.target.as_deref().unwrap_or(&r.path);
            if under_seeding(check, watches.seeding) {
                line = line.push(kit::muted("seeding…"));
            } else {
                line = line.push(kit::muted("quiet"));
            }
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
            let watch_label = if watches.mine.contains(t) {
                "◉"
            } else {
                "○"
            };
            button(text(watch_label).size(font::CAPTION))
                .padding(2)
                .style(button::text)
                .on_press(Message::Subject(SubjectMsg::WatchToggled(t.clone())))
                .into()
        }
        None => iced::widget::Space::new().width(Length::Fixed(18.0)).into(),
    };

    let body = button(line)
        .width(Length::Fill)
        .padding(2)
        .style(|theme: &iced::Theme, _status| button::Style {
            background: None,
            text_color: colors(theme).text(),
            ..Default::default()
        })
        .on_press(row_press(shape));
    row![watch, indent, marker, body]
        .spacing(space::XS)
        .align_y(iced::Alignment::Center)
        .into()
}

/// A standalone pane, for tests and for the `view/mod` composition.
pub fn pane<'a>(d: TreeData<'a>) -> Element<'a, Message> {
    let flat = d.flat;
    let pivot_picker = iced::widget::pick_list(Pivot::ALL.to_vec(), Some(d.pivot), |v| {
        Message::Workspace(WorkspaceMsg::PivotSelected(v))
    })
    .text_size(font::CAPTION);
    let find = iced::widget::text_input("find keys…", d.search)
        .size(font::CAPTION)
        .on_input(|v| Message::Workspace(WorkspaceMsg::TreeSearchChanged(v)));
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
    column![kit::section_header("Keys", None), header, tree_view(d)]
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
            stats.record(k, 8, None, now, None, None);
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
        let i = flat
            .rows
            .iter()
            .position(|r| r.path == "demo/example/foo")
            .unwrap();
        assert!(flat.rows[i].is_leaf);
        assert_eq!(flat.row(i).count, 1);
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
        assert_eq!(flat.rows[0].chunk, "v1");
        assert!(!flat.rows[0].expanded);
        let root = flat.row(0);
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

    /// #249: a search that matches nothing builds nothing.
    ///
    /// It used to build a complete row — four owned `String`s — for every node
    /// in the tree and then drop them again, because retention is decided
    /// bottom-up. On a 40k-key bus that was ~160,000 allocations per keystroke,
    /// for a result of zero rows. The two-pass walk decides first.
    #[test]
    fn a_search_that_matches_nothing_builds_no_rows() {
        let snap = snapshot(&[
            "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu",
            "v1/h-3fa9c2d41b7e/telemetry/sysinfo/mem",
            "v1/h-3fa9c2d41b7e/state/sysinfo/health",
        ]);
        // A cap of zero: under the old code the walk built every row before
        // the post-hoc truncate could look, so this could only pass by never
        // building one.
        let none = search_flatten(&snap, "", "zzzznope", 0, Instant::now());
        assert!(none.rows.is_empty());
        assert_eq!(
            none.truncated, 0,
            "nothing was retained, so nothing went unbuilt — a cap is not a \
             filter and must not report one as the other"
        );
        assert_eq!((none.shown_keys, none.total_keys), (0, 3));
    }

    /// #249: the cap bounds a search that matches everything.
    ///
    /// This is the direction that was genuinely unbounded — `rows` grew to node
    /// count and the cap was applied afterwards, so the memory bound
    /// `app.rs`'s `MAX_ROWS` documents did not hold in this path at all.
    #[test]
    fn the_row_cap_bounds_a_search_that_matches_everything() {
        let keys: Vec<String> = (0..200)
            .map(|i| format!("v1/h-3fa9c2d41b7e/telemetry/sysinfo/k{i}"))
            .collect();
        let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        let snap = snapshot(&refs);
        let capped = search_flatten(&snap, "", "k", 10, Instant::now());
        assert_eq!(capped.rows.len(), 10, "the cap is a bound during the walk");
        assert!(
            capped.truncated > 0,
            "and what it cost is reported, never silently dropped"
        );
        // The denominator ignores the cap: it is a fact about the bus, not
        // about how many rows this view chose to build.
        assert_eq!(capped.total_keys, 200);
        assert_eq!(capped.shown_keys, 200);

        // Uncapped, the same query yields the same counts — so the cap changed
        // what was *built*, not what was *counted*.
        let full = search_flatten(&snap, "", "k", 10_000, Instant::now());
        assert_eq!(
            (full.shown_keys, full.total_keys),
            (capped.shown_keys, capped.total_keys)
        );
        assert_eq!(full.truncated, 0);
        assert_eq!(
            full.rows[..10],
            capped.rows[..],
            "the cap takes a prefix of the same walk, not a different one"
        );
    }

    /// #249: `filtered` is computed rather than asserted.
    ///
    /// `search_flatten` hardcoded `true`, and `filtered` decides which
    /// empty-state sentence the pane shows and whether it prints a coverage
    /// line — an honesty field set by assumption is the thing this crate does
    /// not do.
    #[test]
    fn an_empty_query_is_not_a_filter() {
        let snap = snapshot(&["v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu"]);
        assert!(!search_flatten(&snap, "", "", 100, Instant::now()).filtered);
        assert!(search_flatten(&snap, "", "cpu", 100, Instant::now()).filtered);
    }

    /// #249: a capped pivot reports the rows it did not build, and still
    /// counts the entries beneath them.
    #[test]
    fn a_capped_pivot_reports_the_rows_it_did_not_build() {
        let keys: Vec<String> = (0..50)
            .map(|i| format!("v1/h-3fa9c2d41b7e/telemetry/p{i}/leaf"))
            .collect();
        let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        let snap = snapshot(&refs);
        let all = expand(&[]);
        let capped = pivot_flatten(&snap, "", Pivot::Producer, &all, "", 5, Instant::now());
        assert_eq!(capped.rows.len(), 5);
        assert!(capped.truncated > 0);
        assert_eq!(
            capped.total_keys, 50,
            "the denominator is the bus, not the cap"
        );
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
        let i = flat.rows.iter().position(|r| r.chunk == "sysinfo").unwrap();
        let sysinfo = flat.row(i);
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

    /// Issue #93: origin-pivot groups stand for exactly contiguous wire
    /// subtrees, so they carry real targets — and the selector built from
    /// one is exactly the subtree selector (the acceptance criterion).
    #[test]
    fn origin_pivot_groups_target_their_real_prefix() {
        let snap = snapshot(&[
            "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu",
            "v1/h-3fa9c2d41b7e/state/sysinfo/health",
            "v1/@catalog/state/entity/x",
        ]);
        let flat = pivot_flatten(
            &snap,
            "",
            Pivot::Origin,
            &BTreeSet::new(),
            "x", // filter irrelevant to targets; auto-expands everything
            10_000,
            Instant::now(),
        );
        let by_chunk = |c: &str| {
            flat.rows
                .iter()
                .find(|r| r.chunk == c)
                .unwrap_or_else(|| panic!("row {c}"))
        };
        assert_eq!(by_chunk("@catalog").target.as_deref(), Some("v1/@catalog"));
        let host = pivot_flatten(
            &snap,
            "",
            Pivot::Origin,
            &BTreeSet::new(),
            "cpu",
            10_000,
            Instant::now(),
        );
        let origin = host
            .rows
            .iter()
            .find(|r| r.chunk == "h-3fa9c2d41b7e")
            .expect("origin group");
        assert_eq!(origin.target.as_deref(), Some("v1/h-3fa9c2d41b7e"));
        assert_eq!(
            crate::scope::subtree_selector(origin.target.as_deref().unwrap()),
            "v1/h-3fa9c2d41b7e/**",
            "watching the group declares exactly the origin subtree"
        );
        let class = host
            .rows
            .iter()
            .find(|r| r.chunk == "telemetry")
            .expect("class group");
        assert_eq!(class.target.as_deref(), Some("v1/h-3fa9c2d41b7e/telemetry"));
    }

    /// Issue #93: a base-relative deployment keeps the base in the target —
    /// the selector still spells the wire, never a bare convention path.
    #[test]
    fn pivot_targets_carry_the_base() {
        let snap = snapshot(&["zs/v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu"]);
        let flat = pivot_flatten(
            &snap,
            "zs",
            Pivot::Origin,
            &BTreeSet::new(),
            "cpu",
            10_000,
            Instant::now(),
        );
        let origin = flat
            .rows
            .iter()
            .find(|r| r.chunk == "h-3fa9c2d41b7e")
            .expect("origin group");
        assert_eq!(origin.target.as_deref(), Some("zs/v1/h-3fa9c2d41b7e"));
    }

    /// Issue #93: groups that genuinely span a wildcard stay untargeted —
    /// a producer group across origins is a fleet-wide watch, not one click.
    #[test]
    fn wildcard_spanning_groups_stay_untargeted() {
        let snap = snapshot(&[
            "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu",
            "v1/h-9fa9c2d41b70/telemetry/sysinfo/cpu",
        ]);
        let prod = pivot_flatten(
            &snap,
            "",
            Pivot::Producer,
            &BTreeSet::new(),
            "cpu",
            10_000,
            Instant::now(),
        );
        let group = prod.rows.iter().find(|r| r.chunk == "sysinfo").unwrap();
        assert!(group.target.is_none(), "spans origins — no one-click watch");
        // …but one level down the constraint chain closes: origin under
        // producer means v1/<origin>/<class>/<producer> is NOT contiguous
        // (class is still free), so it stays None too…
        let origin = prod
            .rows
            .iter()
            .find(|r| r.chunk == "h-3fa9c2d41b7e")
            .unwrap();
        assert!(origin.target.is_none(), "class position still wildcarded");
        // …and the class level (origin+class+producer all fixed) targets
        // the full contiguous prefix — producer chunk included, because the
        // node means "this producer's telemetry", not "all telemetry".
        let class = prod
            .rows
            .iter()
            .filter(|r| r.chunk == "telemetry")
            .find(|r| r.target.is_some())
            .expect("a targeted class row");
        assert_eq!(
            class.target.as_deref(),
            Some("v1/h-3fa9c2d41b7e/telemetry/sysinfo")
        );

        let by_class = pivot_flatten(
            &snap,
            "",
            Pivot::Class,
            &BTreeSet::new(),
            "cpu",
            10_000,
            Instant::now(),
        );
        let top = by_class.rows.iter().find(|r| r.depth == 0).unwrap();
        assert_eq!(top.chunk, "telemetry");
        assert!(top.target.is_none(), "spans origins");
    }

    /// Issue #93: the press split — a concrete key that is also a prefix of
    /// deeper keys stays selectable; its marker (alone) expands.
    #[test]
    fn a_prefix_key_row_selects_and_its_marker_expands() {
        let snap = snapshot(&[
            "v1/h-3fa9c2d41b7e/state/tc/iface",
            "v1/h-3fa9c2d41b7e/state/tc/iface/eth0",
        ]);
        let set = expand(&[
            "v1",
            "v1/h-3fa9c2d41b7e",
            "v1/h-3fa9c2d41b7e/state",
            "v1/h-3fa9c2d41b7e/state/tc",
        ]);
        let flat = flat_now(&snap, "", &set, 100);
        let prefix = flat
            .rows
            .iter()
            .find(|r| r.path == "v1/h-3fa9c2d41b7e/state/tc/iface")
            .expect("prefix row");
        assert!(prefix.is_leaf && prefix.has_children);
        match row_press(prefix) {
            Message::Subject(SubjectMsg::Select(Subject::Key(k))) => {
                assert_eq!(k, "v1/h-3fa9c2d41b7e/state/tc/iface")
            }
            other => panic!("body must select, got {other:?}"),
        }
        assert!(
            matches!(
                marker_press(prefix),
                Some(Message::Workspace(WorkspaceMsg::ToggleNode(_)))
            ),
            "the marker is the expand affordance"
        );
        // A plain group row still toggles on body click.
        let group = flat
            .rows
            .iter()
            .find(|r| r.path == "v1/h-3fa9c2d41b7e/state")
            .expect("group row");
        assert!(matches!(
            row_press(group),
            Message::Workspace(WorkspaceMsg::ToggleNode(_))
        ));
    }

    /// Issue #93: expansion paths are namespaced per pivot, so switching
    /// pivots cannot resurrect an expansion that meant something else.
    #[test]
    fn pivot_expansion_paths_are_namespaced() {
        let snap = snapshot(&["v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu"]);
        let by_origin = pivot_flatten(
            &snap,
            "",
            Pivot::Origin,
            &BTreeSet::new(),
            "",
            10_000,
            Instant::now(),
        );
        let by_producer = pivot_flatten(
            &snap,
            "",
            Pivot::Producer,
            &BTreeSet::new(),
            "",
            10_000,
            Instant::now(),
        );
        assert!(by_origin.rows[0].path.starts_with("pivot:origin:"));
        assert!(by_producer.rows[0].path.starts_with("pivot:producer:"));
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
