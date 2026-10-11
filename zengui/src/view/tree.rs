//! The key tree.
//!
//! The snapshot it walks is grammar-blind — `KeyTreeSnapshot` groups on a plain
//! `split('/')` — so an arbitrary bus renders as an ordinary chunk tree with
//! counts and rates. On top of that this module lays the *positional* overlay:
//! when a subtree turns out to be zk2's, its chunks are labelled by their
//! positions in zk2's grammar (#614, FL7) — `zk2`, then system, service,
//! interface, kind token and resource chunks, or `@zk` and the control
//! chunks below it.
//!
//! The overlay is resolved **relative to the namespace, never by absolute
//! index** — a multi-chunk namespace like `acme/fleet-a` is legal — and every
//! position is checked with the grammar's own types (`zenkey_model::grammar`:
//! a name, an interface chunk, a kind token), the ones its `parse` uses. If a
//! chunk is not what its position must be — the namespace's own chunk, `zk2`,
//! a valid name — the whole subtree is left unlabelled rather than
//! mislabelled. A `v1/…` key is foreign since the port, and wears no role.
//!
//! Tree v2 (issue #65) adds three things on top:
//!
//! - **Pivots**: the same entries re-grouped by system, by address, by
//!   interface or by kind (#614, FL7; v1's were origin, producer and class).
//!   The re-keying happens here, on the flattened entries — the hot
//!   `TreeNode` stays contract-blind (the §15 amendment's ruling).
//! - **Find-in-tree**: a fuzzy filter over real key paths. A filtered tree is
//!   a partial view, so it says what it hides ("showing N of M") — O5's
//!   spirit applied to a local filter.
//! - **Virtualized rendering**: rows have a fixed height and only the
//!   scrolled-into window is built each frame, so the row cap is a memory
//!   bound (still reported when hit), not a display truncation.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Instant;

use iced::widget::{Column, column, row, text};
use iced::{Element, Length};
use zenkey_fleet::KeyTreeSnapshot;
use zenkey_fleet::{DeclRef, MergedNode, NodeStats, NodeStatus};
use zenkey_model::grammar::{CONTROL, GRAMMAR, IfaceId, KindToken, Name};

use crate::message::{Message, Subject, SubjectMsg, WorkspaceMsg};
use crate::patharena::{ChunkId, PathArena, PathId};
use crate::view::kit::{self, human_bytes, human_rate};
use crate::view::theme::{Tone, colors};
use crate::view::tokens::{Spacing, face, font};

/// Fixed row height — what makes the scroll window arithmetic exact. The
/// *comfortable* baseline: the pane renders at `sp.row(ROW_HEIGHT)` (#192),
/// so density scales the rows and the window arithmetic together.
pub const ROW_HEIGHT: f32 = 24.0;

/// What a chunk means, when the subtree is zk2's (#614, FL7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The grammar's own first chunk, `zk2`.
    Grammar,
    System,
    Service,
    /// `<system>/<service>` as one group — the address pivot's first level.
    Address,
    /// The interface chunk, `<name>.v<major>`.
    Iface,
    /// The kind token: `stream`, `@stream`, `state`, `@state`, `events`,
    /// `@op`.
    Kind,
    /// A resource chunk below the kind token: the template's.
    Resource,
    /// `@zk` and everything under it: tokens, descriptors, bundles.
    Control,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Role::Grammar => "grammar",
            Role::System => "system",
            Role::Service => "service",
            Role::Address => "address",
            Role::Iface => "iface",
            Role::Kind => "kind",
            Role::Resource => "resource",
            Role::Control => "control",
        }
    }
}

/// The role of every chunk of `path`, under `namespace` — the tree's own
/// overlay, for the surfaces that show one path (the location trail).
/// `None` at every chunk of a path that is not zk2's under the namespace.
pub fn roles_of(path: &str, namespace: &str) -> Vec<Option<Role>> {
    let base: Vec<&str> = namespace.split('/').filter(|c| !c.is_empty()).collect();
    let mut expect = start_expect(&base);
    path.split('/')
        .map(|chunk| {
            let (role, next) = classify(chunk, expect, &base);
            expect = next;
            role
        })
        .collect()
}

/// How the tree groups its entries (issue #65; zk2's positions since #614,
/// FL7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pivot {
    /// Raw chunk order — the wire as it is.
    #[default]
    Chunks,
    /// System first: `system / service / iface / kind / resource…`, every
    /// level one contiguous wire subtree.
    System,
    /// Address first: each `system/service` one group, the services of a
    /// deployment side by side.
    Address,
    /// Interface first: every address providing it under it — a group
    /// that spans addresses, so it offers no one-click watch.
    Iface,
    /// Kind first (`stream`, `@state`, …), then interface, then address.
    Kind,
}

impl Pivot {
    pub const ALL: [Pivot; 5] = [
        Pivot::Chunks,
        Pivot::System,
        Pivot::Address,
        Pivot::Iface,
        Pivot::Kind,
    ];

    /// Short stable token: expansion paths are namespaced per pivot, so a
    /// path opened under one pivot never accidentally opens a group that
    /// happens to share its spelling under another.
    pub fn key(self) -> &'static str {
        match self {
            Pivot::Chunks => "chunks",
            Pivot::System => "system",
            Pivot::Address => "address",
            Pivot::Iface => "iface",
            Pivot::Kind => "kind",
        }
    }
}

impl std::fmt::Display for Pivot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Pivot::Chunks => "chunks",
            Pivot::System => "by system",
            Pivot::Address => "by address",
            Pivot::Iface => "by iface",
            Pivot::Kind => "by kind",
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
/// `zenkey_fleet::merge_skeleton` computes `status` from *set
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
///
/// ## Ids, not strings (#251)
///
/// Every path-shaped field is an id into [`Flattened::arena`]. A shape used to
/// own four `String`s — and in the non-pivot walk `path` and `target` were
/// byte-identical duplicates, so a 50,000-row flatten allocated 150,000–
/// 200,000 strings to feed a window that draws forty. Display strings are
/// materialised in [`Flattened::row`] and in the press builders, never here.
///
/// Ids are minted in walk order, so two shapes from the *same walk over the
/// same tree* still compare equal across `Flattened`s — what the cap-prefix
/// test relies on. An id means nothing against any other arena.
#[derive(Debug, Clone, PartialEq)]
pub struct RowShape {
    pub depth: usize,
    /// Display chunk — a symbolic skeleton position renders as `{var}`.
    pub chunk: ChunkId,
    /// The display-path prefix this row stands for (pivot paths in pivot
    /// mode — an expansion key, not necessarily a wire path).
    pub path: PathId,
    /// The *real* display path select/watch should act on. `None` on pivot
    /// group rows, whose synthetic grouping has no contiguous wire subtree.
    /// In the non-pivot walk this is `path` itself — one copied `u32`, where
    /// it used to be the row's second allocation of the same string.
    pub target: Option<PathId>,
    pub has_children: bool,
    pub expanded: bool,
    /// The declared/observed state (issue #85's typed acceptance criterion).
    pub status: NodeStatus,
    /// Traffic landed on exactly this key. Structural despite being derived
    /// from a count — see the type docs.
    pub is_leaf: bool,
    /// `None` for any chunk outside a recognised zk2 subtree.
    pub role: Option<Role>,
    /// The contract-declared payload type, when the skeleton knows one.
    /// Interned like a chunk: type names repeat across every row of a
    /// revision.
    pub decl_type: Option<ChunkId>,
    /// A declared operation's leaf (`…/@op/<name>`), or a row under an `@op`
    /// chunk (#614, FL6b): a queryable carries no samples, and a watch's
    /// seed would GET it — a GET on an operation's key is a call (spec §5)
    /// — so the row offers no watch control. Structural: it comes off the
    /// skeleton's `DeclRef` and the path's own chunks.
    pub operation: bool,
}

/// Where a drawn row's numbers come from (#177).
#[derive(Debug, Clone)]
enum Numbers {
    /// This tick's snapshot, held by `Arc` so the frame's read cannot be
    /// swapped out from under it by the engine's `ArcSwap`. A drawn row looks
    /// itself up by wire path — the arena's parent chain, root-first, since
    /// #251 — O(depth) for the ~40 rows in the window, and nothing at all for
    /// the 49,960 nobody can see.
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
    /// `None` for any chunk outside a recognised zk2 subtree.
    pub role: Option<Role>,
    /// The contract-declared payload type, when the skeleton knows one.
    pub decl_type: Option<String>,
    /// A declared operation's leaf: no watch control (see
    /// [`RowShape::operation`]).
    pub operation: bool,
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
    /// What every id in `rows` resolves against (#251). Rebuilt with the
    /// rows; an id from one flatten means nothing to another's arena.
    pub arena: PathArena,
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
            arena: PathArena::new(),
            truncated: 0,
            shown_keys: 0,
            total_keys: 0,
            filtered: false,
            numbers: Numbers::Frozen(Vec::new()),
            paths: PathKind::Wire,
            now: Instant::now(),
        }
    }

    /// One drawn row: shape joined to whatever numbers are current (#177),
    /// with the display strings materialised from the arena (#251) — the one
    /// per-frame place a path becomes a `String` again, ~40 times.
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
                let chunks = self.arena.chunks_of(shape.path);
                tree.node(&chunks).map(NodeStats::from_tree)
            }
        };
        TreeRow {
            depth: shape.depth,
            chunk: self.arena.chunk_str(shape.chunk).to_string(),
            path: self.arena.display(shape.path),
            target: shape.target.map(|t| self.arena.display(t)),
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
            decl_type: shape.decl_type.map(|t| self.arena.chunk_str(t).to_string()),
            operation: shape.operation,
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
/// `base` is the deployment namespace: its chunks must lead a key for the
/// key to be read as zk2's (#614, FL7). `expanded` holds the paths the user
/// has opened; everything else collapses.
pub fn flatten(
    merged: &MergedNode,
    base: &str,
    expanded: &BTreeSet<String>,
    max_rows: usize,
    now: Instant,
) -> Flattened {
    let base = namespace_chunks(base);
    let mut rows = Vec::new();
    let mut stats = Vec::new();
    let mut arena = PathArena::new();
    let mut probe = String::new();
    let mut truncated = 0;
    let mut total_keys = 0;
    walk(
        merged,
        &mut Ctx {
            base: &base,
            expanded,
            max_rows,
            rows: &mut rows,
            stats: &mut stats,
            arena: &mut arena,
            probe: &mut probe,
            truncated: &mut truncated,
            total_keys: &mut total_keys,
        },
        None,
        0,
        start_expect(&base),
    );
    Flattened {
        rows,
        arena,
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

/// The namespace's chunks, the empty namespace none.
fn namespace_chunks(base: &str) -> Vec<&str> {
    base.split('/').filter(|c| !c.is_empty()).collect()
}

fn start_expect(base: &[&str]) -> Expect {
    if base.is_empty() {
        Expect::Grammar
    } else {
        Expect::Base(0)
    }
}

struct Ctx<'a> {
    /// The namespace's chunks, which must lead a zk2 key (#614, FL7).
    base: &'a [&'a str],
    expanded: &'a BTreeSet<String>,
    max_rows: usize,
    rows: &'a mut Vec<RowShape>,
    /// Index-parallel to `rows` — the numbers as of this walk, for a caller
    /// that never retargets (#177).
    stats: &'a mut Vec<Option<NodeStats>>,
    /// Where the walk interns its paths (#251).
    arena: &'a mut PathArena,
    /// The current display path, one buffer for the whole descent — the
    /// `search_mark` trick. `expanded` is a set of `String`s that outlives
    /// every arena, so its lookups still need a spelled path; this is the
    /// only string the walk maintains, and it never allocates per node.
    probe: &'a mut String,
    truncated: &'a mut usize,
    total_keys: &'a mut usize,
}

/// What the *next* chunk down this branch is expected to be.
///
/// A state machine rather than arithmetic on depth, because depth alone cannot
/// distinguish "we have not reached the system position yet" from "this
/// branch is not zk2's at all" — and conflating them labels a foreign chunk
/// `system`, which is worse than saying nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// The namespace's chunk at this index comes next.
    Base(usize),
    /// `zk2`.
    Grammar,
    /// A system name, or `@zk` (the bundles, `zk2/@zk/contract/…`).
    System,
    Service,
    /// An interface chunk, or `@zk` (an address's tokens and descriptor).
    Iface,
    Kind,
    Resource,
    /// Under `@zk`: every chunk is a control chunk.
    Control,
    /// This branch is not zk2's. Nothing below it is labelled.
    Foreign,
}

fn walk(
    node: &MergedNode,
    ctx: &mut Ctx<'_>,
    parent: Option<PathId>,
    depth: usize,
    expect: Expect,
) {
    for (chunk, child) in &node.children {
        let probe_len = ctx.probe.len();
        if probe_len > 0 {
            ctx.probe.push('/');
        }
        ctx.probe.push_str(chunk);
        let (role, next_expect) = classify(chunk, expect, ctx.base);
        if is_entry(child) {
            *ctx.total_keys += 1;
        }

        let mut walked = false;
        if ctx.rows.len() >= ctx.max_rows {
            *ctx.truncated += 1;
        } else {
            let expanded = ctx.expanded.contains(ctx.probe.as_str());
            let stats = child.stats;
            let chunk_id = ctx.arena.chunk(chunk);
            let path = ctx.arena.node(parent, chunk_id);
            ctx.rows.push(RowShape {
                depth,
                chunk: chunk_id,
                // One id, twice: a wire row's target *is* its path (#251).
                path,
                target: Some(path),
                has_children: !child.children.is_empty(),
                expanded,
                status: child.status,
                is_leaf: stats.map(|s| s.count > 0).unwrap_or(false),
                role,
                decl_type: child.decl.as_ref().map(|d| ctx.arena.chunk(&d.type_name)),
                operation: is_operation(child.decl.as_ref(), ctx.arena, path),
            });
            ctx.stats.push(stats);
            if expanded {
                walk(child, ctx, Some(path), depth + 1, next_expect);
                walked = true;
            }
        }
        // Even truncated/collapsed branches count their entries, or the
        // "N of M" denominator would understate the bus.
        if !walked && !child.children.is_empty() {
            count_entries(child, ctx.total_keys);
        }
        ctx.probe.truncate(probe_len);
    }
}

/// Whether a row stands for an operation (#614, FL6b): a skeleton leaf
/// that declares one (`zenkey_fleet::DeclRef::kind`), or any row whose real
/// path crosses the `@op` chunk — verbatim (core §1.3), so nothing else
/// lives under it. A frontend offers no watch on either.
fn is_operation(decl: Option<&DeclRef>, arena: &PathArena, real: PathId) -> bool {
    decl.is_some_and(|d| d.kind == zenkey_model::authoring::Kind::Operation)
        || arena.contains_chunk(real, zenkey_model::grammar::KindToken::Op.as_str())
}

fn count_entries(node: &MergedNode, total: &mut usize) {
    for child in node.children.values() {
        if is_entry(child) {
            *total += 1;
        }
        count_entries(child, total);
    }
}

/// The role of one chunk, and what to expect below it (#614, FL7).
///
/// Each position is checked with the grammar's own types — what
/// `zenkey_model::grammar::parse` checks a whole key with — chunk by chunk,
/// because the tree labels interior nodes, which are no keys. A skeleton's
/// symbolic `{system}` and `{service}` (no presence read) hold their
/// positions.
fn classify(chunk: &str, expect: Expect, base: &[&str]) -> (Option<Role>, Expect) {
    let named = |what| chunk.starts_with('{') || Name::new(what, chunk).is_ok();
    match expect {
        // The grammar says nothing about how a deployment spells its own
        // namespace, so its chunks carry no role — but they must be its
        // chunks: another deployment's zk2 keys are not this one's.
        Expect::Base(i) if base.get(i) == Some(&chunk) => (
            None,
            if i + 1 < base.len() {
                Expect::Base(i + 1)
            } else {
                Expect::Grammar
            },
        ),
        Expect::Base(_) => (None, Expect::Foreign),
        // Not a zk2 subtree (a `v1/…` key among them, foreign since the
        // port): the whole branch stays unlabelled rather than guessed at —
        // mislabelling is worse than not labelling.
        Expect::Grammar if chunk == GRAMMAR => (Some(Role::Grammar), Expect::System),
        Expect::Grammar => (None, Expect::Foreign),
        Expect::System if chunk == CONTROL => (Some(Role::Control), Expect::Control),
        Expect::System if named("system") => (Some(Role::System), Expect::Service),
        Expect::Service if named("service") => (Some(Role::Service), Expect::Iface),
        Expect::Iface if chunk == CONTROL => (Some(Role::Control), Expect::Control),
        Expect::Iface if chunk.parse::<IfaceId>().is_ok() => (Some(Role::Iface), Expect::Kind),
        Expect::Kind if chunk.parse::<KindToken>().is_ok() => (Some(Role::Kind), Expect::Resource),
        Expect::Resource => (Some(Role::Resource), Expect::Resource),
        Expect::Control => (Some(Role::Control), Expect::Control),
        Expect::System | Expect::Service | Expect::Iface | Expect::Kind | Expect::Foreign => {
            (None, Expect::Foreign)
        }
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
    let base = namespace_chunks(base);
    let mut keep: Vec<Mark> = Vec::new();
    let mut shown = 0usize;
    let mut total = 0usize;
    let mut probe = String::new();
    search_mark(
        merged,
        &mut probe,
        (start_expect(&base), &base),
        query,
        &mut keep,
        &mut shown,
        &mut total,
    );

    let mut rows = Vec::new();
    let mut stats = Vec::new();
    let mut arena = PathArena::new();
    let mut truncated = 0usize;
    let mut at = 0usize;
    search_emit(
        merged,
        None,
        0,
        (start_expect(&base), &base),
        &keep,
        &mut at,
        &mut Emit {
            max_rows,
            rows: &mut rows,
            stats: &mut stats,
            arena: &mut arena,
            truncated: &mut truncated,
        },
    );
    Flattened {
        rows,
        arena,
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
    /// Where an emitting pass interns its paths (#251).
    arena: &'a mut PathArena,
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
    (expect, base): (Expect, &[&str]),
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
        let (_, next_expect) = classify(chunk, expect, base);
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
        let below = search_mark(child, probe, (next_expect, base), query, keep, shown, total);
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
    parent: Option<PathId>,
    depth: usize,
    (expect, base): (Expect, &[&str]),
    keep: &[Mark],
    at: &mut usize,
    ctx: &mut Emit<'_>,
) {
    for (chunk, child) in &node.children {
        let (role, next_expect) = classify(chunk, expect, base);
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
        let chunk_id = ctx.arena.chunk(chunk);
        let path = ctx.arena.node(parent, chunk_id);
        if ctx.rows.len() >= ctx.max_rows {
            *ctx.truncated += 1;
        } else {
            ctx.rows.push(RowShape {
                depth,
                chunk: chunk_id,
                path,
                target: Some(path),
                has_children: !child.children.is_empty(),
                // A search shows the chain to every match, so an ancestor of a
                // match is open by definition.
                expanded: true,
                status: child.status,
                is_leaf: child.stats.map(|s| s.count > 0).unwrap_or(false),
                role,
                decl_type: child.decl.as_ref().map(|d| ctx.arena.chunk(&d.type_name)),
                operation: is_operation(child.decl.as_ref(), ctx.arena, path),
            });
            ctx.stats.push(child.stats);
        }
        search_emit(
            child,
            Some(path),
            depth + 1,
            (next_expect, base),
            keep,
            at,
            ctx,
        );
    }
}

// ---------------------------------------------------------------------------
// Pivots
// ---------------------------------------------------------------------------

/// One concrete entry, with its zk2 coordinates extracted (#614, FL7).
///
/// All ids into the pivot's arena (#251): the walk that collects 50,000 of
/// these used to spell a `String` path, up to three coordinate `String`s and
/// a `Vec<String>` tail for every one.
struct PivotEntry {
    real_path: PathId,
    /// Whether the path is zk2's under the namespace: its `zk2` chunk was
    /// read where the grammar puts it. Every other entry is foreign.
    zk2: bool,
    /// System, service, interface, kind token — in the real path's order,
    /// which is what a group's wire target is reconstructed from.
    coords: [Option<ChunkId>; 4],
    /// Everything after the coordinates: resource chunks, or `@zk` and the
    /// control chunks below it.
    tail: Vec<(ChunkId, Role)>,
    status: NodeStatus,
    stats: Option<NodeStats>,
    decl: Option<DeclRef>,
}

/// Where a coordinate sits in [`PivotEntry::coords`], by role.
fn coord_of(role: Role) -> Option<usize> {
    match role {
        Role::System => Some(0),
        Role::Service => Some(1),
        Role::Iface => Some(2),
        Role::Kind => Some(3),
        Role::Grammar | Role::Address | Role::Resource | Role::Control => None,
    }
}

/// The walk's position: what the next chunk is expected to be, and the
/// namespace's chunks it is checked against.
type At<'a> = (Expect, &'a [&'a str]);

fn collect_entries(
    node: &MergedNode,
    parent: Option<PathId>,
    (expect, base): At<'_>,
    (zk2, coords): (bool, [Option<ChunkId>; 4]),
    tail: &[(ChunkId, Role)],
    arena: &mut PathArena,
    out: &mut Vec<PivotEntry>,
) {
    for (chunk, child) in &node.children {
        let chunk_id = arena.chunk(chunk);
        let child_path = arena.node(parent, chunk_id);
        let (role, next_expect) = classify(chunk, expect, base);
        let (mut zk2, mut coords) = (zk2, coords);
        // The per-node cost this walk was indicted for (#251): a `Vec`
        // allocation plus a deep clone of every `String` in it, at every
        // child, before knowing whether the chunk contributes. Ids make the
        // copy a memcpy of `u32`s.
        let mut next_tail = tail.to_vec();
        match role {
            Some(Role::Grammar) => zk2 = true,
            Some(r) => match coord_of(r) {
                Some(i) => coords[i] = Some(chunk_id),
                None => next_tail.push((chunk_id, r)),
            },
            // A namespace chunk, or a foreign one: a foreign entry is drawn
            // by its real path, and a namespace chunk is in every target.
            None => {}
        }
        if is_entry(child) {
            out.push(PivotEntry {
                real_path: child_path,
                zk2,
                coords,
                tail: next_tail.clone(),
                status: child.status,
                stats: child.stats,
                decl: child.decl.clone(),
            });
        }
        collect_entries(
            child,
            Some(child_path),
            (next_expect, base),
            (zk2, coords),
            &next_tail,
            arena,
            out,
        );
    }
}

/// The synthetic grouping tree a pivot builds from entries.
///
/// Keyed by [`ChunkId`] (#251): re-keying an entry is `entry(id)` on a `u32`
/// where it was `entry(chunk.clone())` per chunk per level. The id order is
/// intern order, not name order, so [`flatten_pnode`] sorts each level by
/// spelling as it walks — a small per-level sort against the wholesale
/// re-keying this tree exists for.
#[derive(Default)]
struct PNode {
    children: std::collections::BTreeMap<ChunkId, PNode>,
    role: Option<Role>,
    /// Set when an entry terminates here: (real path, own stats, decl).
    leaf: Option<(PathId, Option<NodeStats>, Option<DeclRef>)>,
    /// The real wire-path prefix this group stands for, when its constraint
    /// set is exactly contiguous (issue #93) — what watch/select act on.
    target: Option<PathId>,
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

/// Group the tree's entries by system / address / interface / kind (issue
/// #65; zk2's positions since #614, FL7).
///
/// The re-key per entry:
/// - **System**: `system / service / iface / kind / resource…` — every level
///   one contiguous wire subtree;
/// - **Address**: `system/service / iface / kind / resource…` — the address
///   one group, so a deployment's services sit side by side;
/// - **Iface**: `iface / address / kind / resource…` — every provider of an
///   interface under it;
/// - **Kind**: `kind / iface / address / resource…`.
///
/// A coordinate a key does not have is skipped (a control key has no
/// interface), and its `@zk` chunks ride in the tail. Foreign keys — a
/// `v1/…` key among them, foreign since the port, and another namespace's
/// zk2 keys — group under `(foreign)` with their raw path below: present,
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
    let base = namespace_chunks(base);
    let mut arena = PathArena::new();
    let mut entries = Vec::new();
    collect_entries(
        merged,
        None,
        (start_expect(&base), &base),
        (false, [None; 4]),
        &[],
        &mut arena,
        &mut entries,
    );
    let total_keys = entries.len();
    let filtered = !query.is_empty();
    if filtered {
        // The filter needs a spelled path; one buffer serves every entry.
        let mut probe = String::new();
        entries.retain(|e| {
            probe.clear();
            arena.write_display(e.real_path, &mut probe);
            fuzzy_match(&probe, query)
        });
    }
    let shown_keys = entries.len();

    // Chunks before the system position in a zk2 real path (the namespace +
    // the `zk2` chunk) — what group targets are reconstructed relative to.
    let skip = base.len() + 1;
    let mut root = PNode::default();
    for e in &entries {
        let chunks = pivot_chunks(e, pivot, skip, &mut arena);
        let mut node = &mut root;
        for (chunk, role, target) in &chunks {
            node = node.children.entry(*chunk).or_default();
            node.role = *role;
            if node.target.is_none() {
                node.target = *target;
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
        node.leaf = Some((e.real_path, e.stats, e.decl.clone()));
    }

    let mut rows = Vec::new();
    let mut stats = Vec::new();
    let mut probe = String::new();
    let mut truncated = 0usize;
    flatten_pnode(
        &root,
        &mut PivotCtx {
            expanded,
            auto_expand: filtered,
            pivot_key: pivot.key(),
            probe: &mut probe,
            emit: Emit {
                max_rows,
                rows: &mut rows,
                stats: &mut stats,
                arena: &mut arena,
                truncated: &mut truncated,
            },
        },
        None,
        0,
    );
    Flattened {
        rows,
        arena,
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
/// (issue #93): the coordinates fixed so far must be a prefix of `system /
/// service / iface / kind`, and tail chunks require every coordinate the
/// entry has. Groups that genuinely span a wildcard (an interface across
/// addresses, a kind across interfaces) get `None` — offering a watch there
/// would hide a fleet-wide cost behind one click.
fn pivot_chunks(
    e: &PivotEntry,
    pivot: Pivot,
    skip: usize,
    arena: &mut PathArena,
) -> Vec<(ChunkId, Option<Role>, Option<PathId>)> {
    if !e.zk2 {
        // Foreign keys keep their raw order, so every level below the
        // "(foreign)" group is a real prefix — the ancestor chain of the
        // real path, which the arena already holds (#251).
        let mut out: Vec<(ChunkId, Option<Role>, Option<PathId>)> =
            vec![(arena.chunk("(foreign)"), None, None)];
        out.extend((1..=arena.len_of(e.real_path)).map(|i| {
            let at = arena.ancestor_at(e.real_path, i);
            (arena.chunk_of(at), None, Some(at))
        }));
        return out;
    }

    // (chunk, role, the coordinates it fixes as bits in real order) first,
    // targets derived below from the constraint chain.
    let [system, service, iface, kind] = e.coords;
    let one = |c: Option<ChunkId>, role: Role, bit: u8| c.map(|c| (c, role, bit));
    let address = match (system, service) {
        (Some(sys), Some(svc)) => {
            let spelled = format!("{}/{}", arena.chunk_str(sys), arena.chunk_str(svc));
            Some((arena.chunk(&spelled), Role::Address, 0b0011))
        }
        (Some(sys), None) => Some((sys, Role::System, 0b0001)),
        _ => None,
    };
    let levels: Vec<(ChunkId, Role, u8)> = match pivot {
        Pivot::Chunks => unreachable!("chunk order uses flatten()"),
        Pivot::System => vec![
            one(system, Role::System, 0b0001),
            one(service, Role::Service, 0b0010),
            one(iface, Role::Iface, 0b0100),
            one(kind, Role::Kind, 0b1000),
        ],
        Pivot::Address => vec![
            address,
            one(iface, Role::Iface, 0b0100),
            one(kind, Role::Kind, 0b1000),
        ],
        Pivot::Iface => vec![
            one(iface, Role::Iface, 0b0100),
            address,
            one(kind, Role::Kind, 0b1000),
        ],
        Pivot::Kind => vec![
            one(kind, Role::Kind, 0b1000),
            one(iface, Role::Iface, 0b0100),
            address,
        ],
    }
    .into_iter()
    .flatten()
    .collect();

    let entry_mask = levels.iter().fold(0u8, |m, (_, _, bit)| m | bit);
    let mut out: Vec<(ChunkId, Option<Role>, Option<PathId>)> =
        Vec::with_capacity(levels.len() + e.tail.len());
    let mut mask = 0u8;
    // A mask fixes a prefix of the real order when it is all ones from the
    // bottom bit up: `0b0011` does, `0b0100` (an interface across every
    // address) does not.
    let prefix = |m: u8| m & m.wrapping_add(1) == 0;
    for (chunk, role, bit) in levels {
        mask |= bit;
        let present = mask.count_ones() as usize;
        // The prefix `real[..n].join("/")` used to allocate; `ancestor_at`
        // clamps to the path's length exactly as the slice did (#251).
        let target = prefix(mask).then(|| arena.ancestor_at(e.real_path, skip + present));
        out.push((chunk, Some(role), target));
    }
    let present = mask.count_ones() as usize;
    for (i, (chunk, role)) in e.tail.iter().enumerate() {
        let target = (mask == entry_mask && prefix(mask))
            .then(|| arena.ancestor_at(e.real_path, skip + present + i + 1));
        out.push((*chunk, Some(*role), target));
    }
    if out.is_empty() {
        // An entry that *is* the grammar's chunk, with nothing below it.
        out.push((arena.chunk("(…)"), None, None));
    }
    out
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
    /// The current synthetic display path, one buffer for the descent —
    /// `expanded` is a string set, so its lookups need a spelling (#251).
    probe: &'a mut String,
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
fn flatten_pnode(node: &PNode, ctx: &mut PivotCtx<'_>, parent: Option<PathId>, depth: usize) {
    // `PNode.children` is keyed by intern order, not spelling (#251); the
    // display order is restored here, level by level, on interned `&str`s.
    let mut kids: Vec<(&ChunkId, &PNode)> = node.children.iter().collect();
    kids.sort_unstable_by_key(|(id, _)| ctx.emit.arena.chunk_str(**id));
    for (chunk, child) in kids {
        let chunk = *chunk;
        let probe_len = ctx.probe.len();
        // The synthetic first level is one chunk spelling `pivot:<key>:<top>`,
        // so its expansion path never collides with another pivot's — or with
        // a wire path (issue #93's namespacing, kept verbatim).
        let path_chunk = if probe_len == 0 {
            ctx.probe.push_str("pivot:");
            ctx.probe.push_str(ctx.pivot_key);
            ctx.probe.push(':');
            ctx.probe.push_str(ctx.emit.arena.chunk_str(chunk));
            let synthetic = &ctx.probe[..];
            ctx.emit.arena.chunk(synthetic)
        } else {
            ctx.probe.push('/');
            ctx.probe.push_str(ctx.emit.arena.chunk_str(chunk));
            chunk
        };
        let child_path = ctx.emit.arena.node(parent, path_chunk);
        let is_open = ctx.auto_expand || ctx.expanded.contains(ctx.probe.as_str());
        let own = child.leaf.as_ref().and_then(|(_, s, _)| *s);
        if ctx.emit.rows.len() >= ctx.emit.max_rows {
            *ctx.emit.truncated += 1;
            // Still descend: a collapsed count would understate the tree, and
            // the rows below are counted the same way.
            if is_open {
                flatten_pnode(child, ctx, Some(child_path), depth + 1);
            }
            ctx.probe.truncate(probe_len);
            continue;
        }
        ctx.emit.rows.push(RowShape {
            depth,
            chunk,
            path: child_path,
            // A concrete entry acts on its real path; a group acts on its
            // contiguous wire prefix when it has one (issue #93).
            target: child.leaf.as_ref().map(|(p, _, _)| *p).or(child.target),
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
                .and_then(|(_, _, d)| d.as_ref())
                .map(|d| ctx.emit.arena.chunk(&d.type_name)),
            operation: is_operation(
                child.leaf.as_ref().and_then(|(_, _, d)| d.as_ref()),
                ctx.emit.arena,
                child
                    .leaf
                    .as_ref()
                    .map(|(p, _, _)| *p)
                    .or(child.target)
                    .unwrap_or(child_path),
            ),
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
            flatten_pnode(child, ctx, Some(child_path), depth + 1);
        }
        ctx.probe.truncate(probe_len);
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Identity lookup by wire key (#614, FL7): the engine's bounded,
/// generation-tagged identity cache rather than a plain map (issue #107's
/// rule, kept from v1's facts cache): a `HashMap` here grew one entry per
/// distinct key *ever* observed, shadowing a key table that is bounded and
/// counts its evictions. Aliased so every pane and row signature below reads
/// the same, and so `identities.get(key)` stays a pure `&self` read from the
/// render path. The badge itself is [`crate::keyfacts::badged`]'s.
pub type IdentityIndex = crate::keyfacts::IdentityCache;

/// The tree's two watch sets — same type, opposite meanings (#250).
///
/// They were adjacent `&BTreeSet<String>` parameters at **three** call layers —
/// [`pane`], `tree_view` and `row_view`, which is where both are actually
/// read: one picks "seeding…" over "quiet", the other the open eye over the
/// struck one. Transposing
/// them compiled, and the tree then drew every watched subtree as still
/// seeding and every seeding one as settled.
///
/// Named fields make that a type error at all three. Two flat fields on
/// [`TreeData`] would have fixed only the outermost — `row_view` takes six
/// arguments, so clippy never flagged it and it is invisible to #250's own
/// acceptance grep.
#[derive(Clone, Copy)]
pub struct Watches<'a> {
    /// Subtrees this app watches: the eye toggle on each row (open = a watch
    /// this app holds, struck = none, #533).
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
///
/// `Copy` since #557: the rows are built inside `responsive`, whose closure
/// may run once per layout, so it holds the struct rather than consuming it.
#[derive(Clone, Copy)]
pub struct TreeData<'a> {
    /// The flattened rows and their numbers, as of the last rebuild (#177).
    pub flat: &'a Flattened,
    /// How the tree groups its entries (issue #65).
    pub pivot: Pivot,
    /// The find-in-tree query; empty = no filter.
    pub search: &'a str,
    /// The virtual window: where the list is scrolled to and how much of it
    /// is on screen (#360).
    pub viewport: crate::view::kit::Viewport,
    /// Per-key zk2 identities, for the rung badges (#614, FL7).
    pub identities: &'a IdentityIndex,
    /// The deployment namespace: which keys are this deployment's, for the
    /// badges' foreign-key rule ([`crate::keyfacts::badged`]).
    pub namespace: &'a str,
    /// Per-key payload-conformance verdicts (#164) — the cache the tick's
    /// bounded validation batches fill, looked up here, never computed here.
    pub verdicts: &'a crate::verdict::VerdictCache,
    /// Per-subtree budget badges (#221; zk2's since #614, FL7): `None` =
    /// the judgement has not run (no presence read yet), which draws no
    /// badge — never a pass.
    pub budgets: Option<&'a crate::budget::BudgetBadges>,
    /// The pair this struct exists for.
    pub watches: Watches<'a>,
    /// The selected wire key, if any.
    pub selected: Option<&'a str>,
    /// The dock's resolved spacing grid (#192). The Locator defaults to
    /// Compact — a tree wants rows — so this is usually the tight grid.
    pub sp: Spacing,
}

/// What clicking the row body does (issue #93): concrete entries select
/// (detail fetch acts on `target`); groups toggle. Pure, so it is testable
/// without a renderer.
/// Takes the **shape**, not the row: what a click means must not depend on a
/// number that moved this tick (#177). `is_leaf` is structural for exactly
/// that reason — see [`RowShape`].
///
/// The `arena` is the shape's own (`Flattened::arena`): the message carries a
/// spelled path, materialised here — once per click, never per row (#251).
pub fn row_press(r: &RowShape, arena: &PathArena) -> Message {
    match (r.target, r.is_leaf || !r.has_children) {
        (Some(t), true) => Message::Subject(SubjectMsg::Select(Subject::Key(arena.display(t)))),
        _ => Message::Workspace(WorkspaceMsg::ToggleNode(arena.display(r.path))),
    }
}

/// What clicking the expand marker does — its own affordance, so a concrete
/// key that is also a prefix of deeper keys stays selectable (issue #93).
pub fn marker_press(r: &RowShape, arena: &PathArena) -> Option<Message> {
    r.has_children
        .then(|| Message::Workspace(WorkspaceMsg::ToggleNode(arena.display(r.path))))
}

/// Whether a row sits at or under one of the still-seeding watch paths.
fn under_seeding(row_path: &str, seeding: &BTreeSet<String>) -> bool {
    seeding
        .iter()
        .any(|p| row_path == p || row_path.starts_with(&format!("{p}/")))
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
            return kit::empty(
                kit::EmptyKind::Filtered,
                "No keys match",
                "The filter hides every key — showing 0 of the tree's entries. \
                 Clear it to see them again.",
            );
        }
        return kit::empty(
            kit::EmptyKind::Silent,
            "Nothing observed yet",
            "An empty tree is not a verdict about the bus (RFC 05 §3.1) — \
             it may simply mean nothing has published on the current scope.",
        );
    }

    // How much of each row fits is a question about the pane's width
    // (#557), so the rows are built where that width is known — the app
    // bar's idiom (#536).
    iced::widget::responsive(move |size| rows_view(d, size.width)).into()
}

/// The windowed rows at one pane width (#557).
fn rows_view<'a>(d: TreeData<'a>, width: f32) -> Element<'a, Message> {
    let flat = d.flat;
    // The rows end where the embedded scrollbar begins (below).
    let width = width - SCROLLBAR - d.sp.xs;
    // The density-scaled row height (#192): the window arithmetic and the
    // fixed row containers must agree on it, or the scrollbar lies.
    let row_h = d.sp.row(ROW_HEIGHT, crate::view::tokens::CAPTION_LINE);
    let (first, last) = kit::window(flat.rows.len(), d.viewport, row_h);
    let mut col = Column::new();
    if first > 0 {
        col = col.push(iced::widget::Space::new().height(Length::Fixed(first as f32 * row_h)));
    }
    // The one place a `TreeRow` is built now: ~40 per frame, joined against
    // this tick's numbers, rather than 50,000 per tick of which the window
    // draws forty (#177).
    for i in first..last {
        let shape = &flat.rows[i];
        let r = flat.row(i);
        col = col.push(
            iced::widget::container(row_view(
                shape,
                r,
                RowContext {
                    arena: &flat.arena,
                    identities: d.identities,
                    namespace: d.namespace,
                    verdicts: d.verdicts,
                    budgets: d.budgets,
                    selected: d.selected,
                    watches: d.watches,
                    sp: d.sp,
                    width,
                },
            ))
            .height(Length::Fixed(row_h))
            // A row is one line however narrow the pane (#557): whatever
            // does not fit is cut here, never painted over the next row —
            // echo's rows have held the same line since #538.
            .clip(true),
        );
    }
    if last < flat.rows.len() {
        col = col.push(
            iced::widget::Space::new()
                .height(Length::Fixed((flat.rows.len() - last) as f32 * row_h)),
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
        // Embedded, not floating (#557): iced's rail is opaque, and a
        // floating one sat on the right-hand end of every row — the rate
        // and the badges, the half the tiers exist to keep.
        .spacing(d.sp.xs)
        .on_scroll(|viewport| Message::Workspace(WorkspaceMsg::TreeScrolled(viewport.into())))
        .into()
}

/// iced's default scrollbar width — what an embedded one takes from the rows.
const SCROLLBAR: f32 = 10.0;

/// Two halves of one row, and the split is the point (#177).
///
/// `shape` decides **interaction** — what a click means — and comes from the
/// cache, so it cannot change under the cursor because a number moved. `r`
/// carries the numbers, joined against this tick's snapshot.
///
/// Neither is borrowed by the returned `Element`: every use below is a clone or
/// a `Copy` field, and the one that looks like a borrow (`identities.get`)
/// borrows `identities`. That is what lets `tree_view` hand these in as
/// per-frame temporaries.
/// What every row reads besides its own shape and numbers — the lookup
/// tables and the selection, one `Copy` bundle so `row_view` stays under the
/// argument lint the same way [`TreeData`] keeps `tree_view` there (#250's
/// precedent: adjacent same-typed parameters are how a transposition
/// compiles).
#[derive(Clone, Copy)]
struct RowContext<'a> {
    arena: &'a PathArena,
    /// Per-key zk2 identities (#614, FL7) — looked up, never resolved here.
    identities: &'a IdentityIndex,
    namespace: &'a str,
    /// Per-key payload verdicts (#164) — looked up, never computed here.
    verdicts: &'a crate::verdict::VerdictCache,
    /// Per-subtree budget badges (#221, FL7); `None` = not judged yet.
    budgets: Option<&'a crate::budget::BudgetBadges>,
    selected: Option<&'a str>,
    watches: Watches<'a>,
    /// The dock's resolved spacing grid (#192) — rows spend it, they never
    /// resolve it.
    sp: Spacing,
    /// The pane's width, from `responsive` (#557) — what decides the row's
    /// [`RowTier`].
    width: f32,
}

/// How much of a row the pane has room for (#557).
///
/// A row has two halves. The **left** — the name, its age, its role, its
/// status — fills what is left and is clipped at its edge, so a long name
/// is cut rather than wrapped. The **right** — the numbers and the verdict
/// badges — is laid out first at its natural width, so it is never the part
/// that disappears; which is exactly why it must shrink with the pane, or a
/// narrow Locator would be all numbers and no names. The tiers are what it
/// gives up, lowest priority first: the declared type, the bytes and the
/// verdict's parenthetical, then the role word and the sample count. The
/// identity and verdict badges are the last to go — they are findings, and
/// a resolved rung, the expected answer, yields to the verdict (#614, FL7)
/// — and below the narrowest tier `fit` drops them whole and counts them
/// (`+N`) rather than let the right half take the name past `NAME_FLOOR`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RowTier {
    /// Name, age, status | rate, identity, verdict.
    Narrow,
    /// Adds the role word and the sample count.
    Medium,
    /// Today's whole row: adds the declared type, the type name and the
    /// bytes.
    Wide,
}

impl RowTier {
    /// The body width at and above which a row is [`RowTier::Wide`].
    pub const WIDE: f32 = 560.0;
    /// … and [`RowTier::Medium`].
    pub const MEDIUM: f32 = 360.0;

    /// The tier for a row whose body — the pane minus the watch toggle, the
    /// expand marker and the indent — is `body` pixels wide.
    pub fn for_body(body: f32) -> Self {
        if body >= Self::WIDE {
            RowTier::Wide
        } else if body >= Self::MEDIUM {
            RowTier::Medium
        } else {
            RowTier::Narrow
        }
    }
}

/// What a row spends outside its two halves, on the comfortable grid: the
/// watch toggle and the expand marker (each a 12px glyph in XS padding), the
/// three gaps between them, the row button's padding and the gap between the
/// halves. Compact spends less, so it errs toward room.
const ROW_GUTTER: f32 = 68.0;

/// What the right half always leaves the name (#557): a dozen characters of
/// a mono caption.
const NAME_FLOOR: f32 = 96.0;

/// A caption character's advance, rounded up (#557): Inter at 12px measures
/// about 5.8px a character on these words, JetBrains Mono 7.2px. Estimates,
/// and deliberately generous ones — overestimating drops a cell that would
/// have fitted (and counts it), underestimating would clip one mid-word.
const SANS_W: f32 = 6.4;
const MONO_W: f32 = 7.2;

/// A plain caption cell's estimated width.
fn text_width(s: &str) -> f32 {
    s.chars().count() as f32 * SANS_W
}

/// A glyph badge's estimated width: its word, its glyph, the gap between
/// them and the chip's horizontal padding.
fn badge_width(word: &str) -> f32 {
    text_width(word) + SANS_W + 3.0 * crate::view::tokens::space::XS
}

/// A data chip's estimated width: its mono text and the chip's padding.
fn chip_width(s: &str) -> f32 {
    s.chars().count() as f32 * MONO_W + 2.0 * crate::view::tokens::space::XS
}

/// One cell of a row's right half (#557).
struct RightCell<'a> {
    /// Lower keeps longer: the numbers, then a rung that stopped short (a
    /// finding), then the verdict, then a resolved rung (the expected
    /// answer, #614, FL7), then the type name.
    rank: u8,
    /// Estimated, see [`SANS_W`].
    width: f32,
    el: Element<'a, Message>,
}

/// The right half at `budget` pixels (#557): every cell if they fit, else
/// the cells that fit **in priority order**, in display order, followed by a
/// `+N` chip for the ones that did not. A badge that does not fit is left out
/// whole and counted — never cut into a sliver, and never silently gone,
/// because a missing badge would read as a question nobody asked. The first
/// cell by priority is always kept; past that, the first cell that does not
/// fit ends the run, so a small low-priority cell never jumps the queue.
///
/// Returns the row and its estimated width — more than `budget` only when
/// the first cell and the count alone overrun it.
fn fit<'a>(
    cells: Vec<RightCell<'a>>,
    budget: f32,
    sp: Spacing,
) -> (iced::widget::Row<'a, Message>, f32) {
    let gap = sp.xs;
    let total: f32 =
        cells.iter().map(|c| c.width).sum::<f32>() + gap * cells.len().saturating_sub(1) as f32;
    let mut keep = vec![true; cells.len()];
    if total > budget {
        let mut order: Vec<usize> = (0..cells.len()).collect();
        order.sort_by_key(|&i| cells[i].rank);
        // Room for the count itself, at its widest likely spelling.
        let room = budget - chip_width("+9") - gap;
        let mut used = 0.0;
        let mut fits = true;
        for (n, &i) in order.iter().enumerate() {
            let w = cells[i].width + if n > 0 { gap } else { 0.0 };
            if n == 0 || (fits && used + w <= room) {
                used += w;
            } else {
                fits = false;
                keep[i] = false;
            }
        }
    }
    let dropped = keep.iter().filter(|k| !**k).count();
    let mut need = 0.0;
    let mut row = iced::widget::Row::new()
        .spacing(sp.xs)
        .align_y(iced::Alignment::Center);
    for (cell, kept) in cells.into_iter().zip(keep) {
        if kept {
            need += cell.width + if need > 0.0 { gap } else { 0.0 };
            row = row.push(cell.el);
        }
    }
    if dropped > 0 {
        let count = format!("+{dropped}");
        need += chip_width(&count) + if need > 0.0 { gap } else { 0.0 };
        row = row.push(kit::data_chip(count));
    }
    (row, need)
}

/// The pane width under which the indent halves (#566).
const NARROW_PANE: f32 = 360.0;

/// Pixels of indent per depth level.
const INDENT: f32 = 14.0;

fn row_view<'a>(shape: &RowShape, r: TreeRow, cx: RowContext<'a>) -> Element<'a, Message> {
    let RowContext {
        arena,
        identities,
        namespace,
        verdicts,
        budgets,
        selected,
        watches,
        sp,
        width,
    } = cx;
    // A narrow pane spends less on depth (#566): at 14px a level, a
    // seven-deep leaf in a 200px Locator had no room left for its name.
    let per_level = if width < NARROW_PANE {
        INDENT / 2.0
    } else {
        INDENT
    };
    let indent_w = r.depth as f32 * per_level;
    let body_w = width - indent_w - ROW_GUTTER;
    let tier = RowTier::for_body(body_w);
    let indent = iced::widget::Space::new().width(Length::Fixed(indent_w));

    // The expand marker is its own affordance (issue #93): a concrete key
    // that is also a prefix of deeper keys keeps body-click = select.
    let marker: Element<'a, Message> = match marker_press(shape, arena) {
        Some(msg) => kit::link(kit::icon_caption(if r.expanded {
            kit::Icon::ChevronDown
        } else {
            kit::Icon::ChevronRight
        }))
        // Horizontal only: a fixed-height row has no vertical air to
        // spend, at either density.
        .padding([0.0, sp.xs])
        .on_press(msg)
        .into(),
        None => iced::widget::Space::new().width(Length::Fixed(18.0)).into(),
    };

    let is_selected = r.target.is_some() && selected == r.target.as_deref();
    // Read off `r` before the name takes its chunk.
    let status = status_word(&r, watches);
    // What the name and its age actually need, up to the floor — a short
    // name leaves the right half the rest (#557).
    let mono = |s: &str| s.chars().count() as f32 * MONO_W;
    let name_need =
        (mono(&r.chunk) + r.age_s.map_or(0.0, |a| sp.xs + mono(&kit::age_word(a)))).min(NAME_FLOOR);
    let counts = if r.is_leaf {
        Some(leaf_counts(&r, tier))
    } else if !r.expanded && r.subtree_count > 0 {
        Some(subtree_counts(&r, tier))
    } else {
        None
    };
    let name = kit::cell(r.chunk)
        .font(face::MONO)
        .style(move |theme: &iced::Theme| text::Style {
            color: Some(if is_selected {
                colors(theme).primary()
            } else {
                colors(theme).text()
            }),
        });

    let mut line = row![name].spacing(sp.xs).align_y(iced::Alignment::Center);
    // The right half's cells, in display order, each with its priority and
    // its estimated width — `fit` decides which of them the row has room for.
    let mut right: Vec<RightCell<'a>> = Vec::new();

    // Freshness (issue #65), as a word since #538: "now", "12s", "4m" on a
    // single-hue ramp. The dot it replaced said freshness by colour alone —
    // green, then amber, verdict hues for something that is no verdict —
    // and its `●` was the Registered badge's glyph in this same row.
    if let Some(age) = r.age_s {
        line = line.push(kit::freshness(age));
    }

    if let Some(role) = r.role.filter(|_| tier >= RowTier::Medium) {
        line = line.push(kit::cell(role.label()));
    }

    // The declared/observed state (issue #85): "declared" must never read
    // like "quiet", and "unwatched" like neither — three words, and none of
    // them a verdict (#543). Unwatching is the operator's choice, so it is
    // the outline of a question not asked; the other three are commentary.
    // None borrows a registration glyph: the old "unwatched" wore the
    // Unregistered ○, in a row that could carry the real one.
    if let Some((tone, word)) = status {
        line = line.push(kit::status_chip(tone, word));
    }
    let declared = r.decl_type.is_some();
    if let Some(ty) = r.decl_type.filter(|_| tier == RowTier::Wide) {
        line = line.push(kit::cell(ty));
    }

    // The population budget (#221; core §2.7 since #614, FL7), on the
    // family's subtree row: the core judge's answer with its numbers or its
    // reason — "unobservable" says why, and is never a pass. `budgets` is
    // `None` until the first judgement lands: no badge, no claim.
    if let Some(b) = budgets.and_then(|b| b.get(r.target.as_deref().unwrap_or(&r.path))) {
        line = line.push(kit::badge_budget(b.tone(), b.label()));
    }

    // A collapsed node reports its whole subtree; an expanded leaf reports
    // itself. Showing only leaf traffic on a collapsed node would understate
    // it by exactly the part the user cannot see.
    if let Some(counts) = counts {
        right.push(RightCell {
            rank: 0,
            width: text_width(&counts),
            el: kit::cell(counts).into(),
        });
    }
    if r.is_leaf {
        if let Some((t, res)) = r
            .target
            .as_deref()
            .and_then(|t| identities.get(t).map(|res| (t, res)))
        {
            // The rung the key's identity reached (#614, FL7) — none for a
            // foreign key, which answers a question nobody asked of it.
            if let Some(rung) = crate::keyfacts::badged(t, namespace, &res.identity) {
                let word = rung.label();
                right.push(RightCell {
                    // A resolved key is the expected answer: with no room,
                    // it gives way to the verdict, which may be a finding.
                    rank: if rung == crate::keyfacts::Rung::Resolved {
                        3
                    } else {
                        1
                    },
                    width: badge_width(word),
                    el: kit::tone_badge(rung.tone(), word),
                });
            }
            // The type the contract gives a resolved key — unless the
            // skeleton already drew it on the left.
            if let Some(ty) = res
                .resolved
                .as_ref()
                .filter(|_| tier == RowTier::Wide && !declared)
                .map(crate::keyfacts::type_name)
            {
                right.push(RightCell {
                    rank: 4,
                    width: text_width(&ty),
                    el: kit::cell(ty).into(),
                });
            }
        }
        // The payload verdict of the most recently checked sample (#164) —
        // a cache lookup, never a decode. A leaf with no entry draws no
        // badge: either no sample has been checked yet or none carried a
        // payload to check, and a badge would claim a check nobody ran —
        // the checked states, `NotValidated` included, all render.
        if let Some(entry) = r.target.as_deref().and_then(|t| verdicts.get(t)) {
            let word = if tier == RowTier::Wide {
                crate::verdict::label(&entry.verdict)
            } else {
                crate::verdict::short_label(&entry.verdict).to_string()
            };
            right.push(RightCell {
                rank: 2,
                width: badge_width(&word),
                el: kit::badge_verdict(crate::verdict::tone(&entry.verdict), word),
            });
        }
    }

    // The left half fills and clips; the right half is laid out first, at
    // its natural width (#557, [`RowTier`]) — but never past the name's
    // floor: `fit` keeps whole cells by priority and counts the rest, and
    // the clip is only the backstop for an estimate that ran short, so the
    // row always says *which* key the numbers belong to.
    let budget = (body_w - name_need).max(0.0);
    let (right, need) = fit(right, budget, sp);
    let line = row![
        iced::widget::container(line).width(Length::Fill).clip(true),
        // The first cell and the count are kept whatever the budget, so the
        // half may need more than it was given: then the name gives way.
        iced::widget::container(right)
            .max_width(budget.max(need))
            .clip(true)
    ]
    .spacing(sp.sm)
    .align_y(iced::Alignment::Center);

    // Observation is opt-in, per subtree (issue #85). The toggle reflects
    // *this app's* watches, not global coverage — and only rows standing for
    // a real wire subtree offer it (pivot groups are synthetic), and none
    // standing for a declared operation (#614, FL6b): a queryable carries no
    // samples, so a watch on it would observe nothing and say "quiet".
    let watch: Element<'a, Message> = match r.target {
        Some(t) if !r.operation => {
            // Two shapes, not one shape in two colours (#193): the open eye
            // is a watch this app holds, the struck one is none — and the
            // struck one no longer shares a glyph with the Unregistered
            // badge in the same row, as the old `○` did (#533).
            let watch_icon = if watches.mine.contains(&t) {
                kit::icon_caption(kit::Icon::Watched)
            } else {
                kit::icon_caption(kit::Icon::Unwatched).style(|theme: &iced::Theme| {
                    iced::widget::text::Style {
                        color: Some(crate::view::theme::colors(theme).text_dim()),
                    }
                })
            };
            kit::link(watch_icon)
                .padding([0.0, sp.xs])
                .on_press(Message::Subject(SubjectMsg::WatchToggled(t)))
                .into()
        }
        _ => iced::widget::Space::new().width(Length::Fixed(18.0)).into(),
    };

    // The hover wash is what makes a 40k-row list trackable with the mouse
    // (#193); selection additionally paints the stronger shade.
    let body = kit::row_button(line, is_selected)
        .padding([0.0, sp.xs])
        .on_press(row_press(shape, arena));
    row![watch, indent, marker, body]
        .spacing(sp.xs)
        .align_y(iced::Alignment::Center)
        .into()
}

/// A row's declared/observed status as a word chip (#85, #543) — `None` for
/// an observed key, whose numbers say it all.
fn status_word(r: &TreeRow, watches: Watches<'_>) -> Option<(Tone, &'static str)> {
    match r.status {
        NodeStatus::DeclaredOnly(_) => Some((Tone::Info, "declared")),
        NodeStatus::WatchedQuiet(_) => {
            // While the watch's seed phase is still running, "quiet" is not
            // yet an observation — the seed may still deliver (issue #92).
            let check = r.target.as_deref().unwrap_or(&r.path);
            Some(if under_seeding(check, watches.seeding) {
                (Tone::Info, "seeding…")
            } else {
                (Tone::Info, "quiet")
            })
        }
        NodeStatus::Unwatched(_) => Some((Tone::Neutral, "unwatched")),
        NodeStatus::Observed(_) => None,
    }
}

/// A leaf's own numbers at a tier (#557): the rate always, then the count,
/// then the bytes.
fn leaf_counts(r: &TreeRow, tier: RowTier) -> String {
    match tier {
        RowTier::Narrow => human_rate(r.rate_hz),
        RowTier::Medium => format!("{} · {}", r.count, human_rate(r.rate_hz)),
        RowTier::Wide => format!(
            "{} · {} · {}",
            r.count,
            human_bytes(r.bytes),
            human_rate(r.rate_hz)
        ),
    }
}

/// A collapsed node's subtree numbers at a tier — the same priority as
/// [`leaf_counts`], with the key count where the sample count is.
fn subtree_counts(r: &TreeRow, tier: RowTier) -> String {
    match tier {
        RowTier::Narrow => human_rate(r.subtree_rate_hz),
        RowTier::Medium => format!(
            "{} · {}",
            kit::plural(r.subtree_keys, "key"),
            human_rate(r.subtree_rate_hz)
        ),
        RowTier::Wide => format!(
            "{} · {} · {} · {}",
            kit::plural(r.subtree_keys, "key"),
            r.subtree_count,
            human_bytes(r.subtree_bytes),
            human_rate(r.subtree_rate_hz)
        ),
    }
}

/// A standalone pane, for tests and for the `view/mod` composition.
pub fn pane<'a>(d: TreeData<'a>) -> Element<'a, Message> {
    let flat = d.flat;
    let pivot_picker = kit::picker(Pivot::ALL.to_vec(), Some(d.pivot), |v| {
        Message::Workspace(WorkspaceMsg::PivotSelected(v))
    })
    .text_size(font::CAPTION);
    let find = kit::search("find keys…", d.search)
        .size(font::CAPTION)
        .on_input(|v| Message::Workspace(WorkspaceMsg::TreeSearchChanged(v)));
    let mut header = row![pivot_picker, find]
        .spacing(d.sp.sm)
        .align_y(iced::Alignment::Center);
    if flat.filtered {
        // A filtered tree is a partial view; say what it hides (O5's spirit).
        header = header.push(kit::muted(format!(
            "showing {} of {}",
            kit::plural(flat.shown_keys, "key"),
            flat.total_keys
        )));
    }
    let sp = d.sp;
    column![kit::section_header("Keys", None), header, tree_view(d)]
        .spacing(sp.sm)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey_fleet::StatsTable;

    /// `robot1/nav`'s position stream, and keys beside it in the same and
    /// another service, for the pivots.
    const POSITION: &str = "zk2/robot1/nav/nav.v2/stream/position";
    const STATUS: &str = "zk2/robot1/nav/nav.v2/state/status";
    const ROBOT2: &str = "zk2/robot2/nav/nav.v2/stream/position";
    const ETH0: &str = "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0";
    const V1: &str = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu";

    fn snapshot(keys: &[&str]) -> MergedNode {
        let mut stats = StatsTable::new();
        let now = Instant::now();
        for k in keys {
            stats.record(k, 8, None, now, None, None);
        }
        let observed = zenkey_fleet::KeyTreeSnapshot::build(&stats);
        // Tests watch everything: every observed node reads Observed.
        let skel = zenkey_fleet::Skeleton::build(
            &zenkey_fleet::Lens::new("", None, &zenkey_fleet::ContractSet::new()),
            None,
        );
        zenkey_fleet::merge_skeleton(&skel, &observed, &["**".to_string()])
    }

    fn expand(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|s| s.to_string()).collect()
    }

    /// Every prefix of every key, so leaves are visible.
    fn expand_all(keys: &[&str]) -> BTreeSet<String> {
        let mut set = BTreeSet::new();
        for k in keys {
            let mut acc = String::new();
            for chunk in k.split('/') {
                if !acc.is_empty() {
                    acc.push('/');
                }
                acc.push_str(chunk);
                set.insert(acc.clone());
            }
        }
        set
    }

    fn flat_now(
        snap: &MergedNode,
        base: &str,
        expanded: &BTreeSet<String>,
        max: usize,
    ) -> Flattened {
        flatten(snap, base, expanded, max, Instant::now())
    }

    fn pivot(snap: &MergedNode, base: &str, p: Pivot, query: &str) -> Flattened {
        pivot_flatten(
            snap,
            base,
            p,
            &BTreeSet::new(),
            query,
            10_000,
            Instant::now(),
        )
    }

    // Shapes carry ids (#251); tests that reason about spellings resolve
    // them against the flatten's own arena through these four.
    fn path_of(flat: &Flattened, r: &RowShape) -> String {
        flat.arena.display(r.path)
    }

    fn chunk_of<'a>(flat: &'a Flattened, r: &RowShape) -> &'a str {
        flat.arena.chunk_str(r.chunk)
    }

    fn target_of(flat: &Flattened, r: &RowShape) -> Option<String> {
        r.target.map(|t| flat.arena.display(t))
    }

    fn find_path<'a>(flat: &'a Flattened, path: &str) -> Option<&'a RowShape> {
        flat.rows.iter().find(|r| path_of(flat, r) == path)
    }

    fn role_of(flat: &Flattened, path: &str) -> Option<Role> {
        find_path(flat, path).and_then(|r| r.role)
    }

    fn row_by_chunk<'a>(flat: &'a Flattened, chunk: &str) -> &'a RowShape {
        flat.rows
            .iter()
            .find(|r| chunk_of(flat, r) == chunk)
            .unwrap_or_else(|| panic!("no row {chunk}"))
    }

    fn tops(flat: &Flattened) -> Vec<&str> {
        flat.rows
            .iter()
            .filter(|r| r.depth == 0)
            .map(|r| chunk_of(flat, r))
            .collect()
    }

    #[test]
    fn collapsed_by_default_and_expands_on_demand() {
        let snap = snapshot(&[POSITION]);
        let flat = flat_now(&snap, "", &BTreeSet::new(), 100);
        assert_eq!(flat.rows.len(), 1, "only the root chunk shows");
        assert_eq!(chunk_of(&flat, &flat.rows[0]), "zk2");
        assert!(flat.rows[0].has_children);

        let flat = flat_now(&snap, "", &expand(&["zk2"]), 100);
        assert_eq!(flat.rows.len(), 2);
        assert_eq!(chunk_of(&flat, &flat.rows[1]), "robot1");
    }

    /// #614 (FL7): zk2's positions label a zk2 key — `zk2`, system,
    /// service, interface, kind token, resource — resolved relative to the
    /// namespace: the same key under a one-chunk and a two-chunk namespace
    /// labels identically, and the namespace's own chunks wear no role.
    #[test]
    fn zk2_roles_are_resolved_relative_to_the_namespace() {
        for (base, prefix) in [
            ("", ""),
            ("spray", "spray/"),
            ("acme/fleet-a", "acme/fleet-a/"),
        ] {
            let full = format!("{prefix}{ETH0}");
            let snap = snapshot(&[&full]);
            let flat = flat_now(&snap, base, &expand_all(&[&full]), 100);
            let at = |rel: &str| role_of(&flat, &format!("{prefix}{rel}"));
            assert_eq!(at("zk2"), Some(Role::Grammar), "base {base:?}");
            assert_eq!(at("zk2/host-a"), Some(Role::System));
            assert_eq!(at("zk2/host-a/tc"), Some(Role::Service));
            assert_eq!(at("zk2/host-a/tc/tc.netif.v1"), Some(Role::Iface));
            assert_eq!(at("zk2/host-a/tc/tc.netif.v1/state"), Some(Role::Kind));
            for rel in [
                "zk2/host-a/tc/tc.netif.v1/state/interfaces",
                "zk2/host-a/tc/tc.netif.v1/state/interfaces/default",
                ETH0,
            ] {
                assert_eq!(at(rel), Some(Role::Resource), "{rel}");
            }
            if let Some(first) = base.split('/').next().filter(|c| !c.is_empty()) {
                assert_eq!(role_of(&flat, first), None, "a namespace chunk has no role");
            }
        }
    }

    /// A zk2 key under another namespace is not this deployment's, and is
    /// not labelled by position (O3): another deployment's keys are foreign
    /// here, however zk2 they look.
    #[test]
    fn another_namespaces_zk2_keys_are_left_unlabelled() {
        let key = format!("elsewhere/{POSITION}");
        let snap = snapshot(&[&key]);
        let flat = flat_now(&snap, "spray", &expand_all(&[&key]), 100);
        for r in &flat.rows {
            assert_eq!(r.role, None, "{} must not be labelled", path_of(&flat, r));
        }
    }

    /// `@zk` and everything under it are control chunks, at the address's
    /// position and at the grammar's (the bundles); a verbatim kind token
    /// is a kind like any other.
    #[test]
    fn control_chunks_and_verbatim_kinds() {
        let keys = [
            "zk2/robot1/nav/@zk/instance/0000000000000001",
            "zk2/@zk/contract/nav.v2/abc",
            "zk2/robot1/camera/camera.v1/@stream/image",
        ];
        let snap = snapshot(&keys);
        let flat = flat_now(&snap, "", &expand_all(&keys), 100);
        assert_eq!(role_of(&flat, "zk2/robot1/nav/@zk"), Some(Role::Control));
        assert_eq!(
            role_of(&flat, "zk2/robot1/nav/@zk/instance"),
            Some(Role::Control)
        );
        assert_eq!(role_of(&flat, "zk2/@zk"), Some(Role::Control));
        assert_eq!(
            role_of(&flat, "zk2/@zk/contract/nav.v2"),
            Some(Role::Control)
        );
        assert_eq!(
            role_of(&flat, "zk2/robot1/camera/camera.v1/@stream"),
            Some(Role::Kind)
        );
    }

    /// A chunk that breaks its position — an interface chunk that is no
    /// `<name>.v<major>`, a kind that is no kind token — leaves the subtree
    /// unlabelled from there down, never guessed at.
    #[test]
    fn a_chunk_that_breaks_its_position_stops_the_labels() {
        let keys = [
            "zk2/robot1/nav/NotAnIface/stream/x",
            "zk2/robot1/nav/nav.v2/telemetry/x",
        ];
        let snap = snapshot(&keys);
        let flat = flat_now(&snap, "", &expand_all(&keys), 100);
        assert_eq!(role_of(&flat, "zk2/robot1/nav"), Some(Role::Service));
        assert_eq!(role_of(&flat, "zk2/robot1/nav/NotAnIface"), None);
        assert_eq!(role_of(&flat, "zk2/robot1/nav/NotAnIface/stream"), None);
        assert_eq!(role_of(&flat, "zk2/robot1/nav/nav.v2"), Some(Role::Iface));
        assert_eq!(role_of(&flat, "zk2/robot1/nav/nav.v2/telemetry"), None);
        assert_eq!(role_of(&flat, "zk2/robot1/nav/nav.v2/telemetry/x"), None);
    }

    /// The location trail reads the same overlay, chunk by chunk.
    #[test]
    fn roles_of_one_path() {
        assert_eq!(
            roles_of(&format!("spray/{POSITION}"), "spray"),
            [
                None,
                Some(Role::Grammar),
                Some(Role::System),
                Some(Role::Service),
                Some(Role::Iface),
                Some(Role::Kind),
                Some(Role::Resource),
            ]
        );
        assert!(
            roles_of(V1, "").iter().all(Option::is_none),
            "v1 is foreign"
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
            assert_eq!(r.role, None, "{} must not be labelled", path_of(&flat, r));
        }
        // …and it still carries its traffic.
        let i = flat
            .rows
            .iter()
            .position(|r| path_of(&flat, r) == "demo/example/foo")
            .unwrap();
        assert!(flat.rows[i].is_leaf);
        assert_eq!(flat.row(i).count, 1);
    }

    /// #614 (FL7): v1 keys are foreign since the port — no v1 overlay: a
    /// `v1/…` subtree is labelled by no position at all.
    #[test]
    fn a_v1_subtree_is_left_unlabelled() {
        let snap = snapshot(&[V1]);
        let flat = flat_now(&snap, "", &expand_all(&[V1]), 100);
        assert!(flat.rows.len() > 3);
        for r in &flat.rows {
            assert_eq!(r.role, None, "{} must not be labelled", path_of(&flat, r));
        }
    }

    /// A collapsed node must carry its subtree's totals, or the number the
    /// user reads understates the traffic by exactly the invisible part.
    #[test]
    fn collapsed_rows_report_subtree_totals() {
        let snap = snapshot(&[POSITION, STATUS, ETH0]);
        let flat = flat_now(&snap, "", &BTreeSet::new(), 100);
        assert_eq!(chunk_of(&flat, &flat.rows[0]), "zk2");
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

    // --- Tree v2 (issue #65) -------------------------------------------

    #[test]
    fn fuzzy_match_is_ordered_subsequence_case_insensitive() {
        assert!(fuzzy_match(POSITION, "position"));
        assert!(fuzzy_match(POSITION, "NAVpos"));
        assert!(fuzzy_match("anything", ""));
        assert!(!fuzzy_match("zk2/robot1/nav", "position"));
        assert!(!fuzzy_match("abc", "acb"));
    }

    /// Search hides nothing silently: the filtered view carries both counts,
    /// and matching keys render with their full ancestor chain expanded.
    #[test]
    fn search_shows_matches_with_ancestors_and_counts_the_rest() {
        let snap = snapshot(&[POSITION, STATUS, ETH0]);
        let flat = search_flatten(&snap, "", "position", 1000, Instant::now());
        assert!(flat.filtered);
        assert_eq!(flat.shown_keys, 1);
        assert_eq!(flat.total_keys, 3);
        let paths: Vec<String> = flat.rows.iter().map(|r| path_of(&flat, r)).collect();
        assert_eq!(
            paths,
            [
                "zk2",
                "zk2/robot1",
                "zk2/robot1/nav",
                "zk2/robot1/nav/nav.v2",
                "zk2/robot1/nav/nav.v2/stream",
                POSITION,
            ],
            "the match renders under its expanded ancestors, nothing else"
        );
        // …labelled as the chunk-order walk labels them.
        assert_eq!(role_of(&flat, "zk2/robot1/nav/nav.v2"), Some(Role::Iface));
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
        let snap = snapshot(&[POSITION, STATUS, ETH0]);
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
            .map(|i| format!("zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/k{i}"))
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
        let snap = snapshot(&[POSITION]);
        assert!(!search_flatten(&snap, "", "", 100, Instant::now()).filtered);
        assert!(search_flatten(&snap, "", "position", 100, Instant::now()).filtered);
    }

    /// #249: a capped pivot reports the rows it did not build, and still
    /// counts the entries beneath them.
    #[test]
    fn a_capped_pivot_reports_the_rows_it_did_not_build() {
        let keys: Vec<String> = (0..50)
            .map(|i| format!("zk2/h{i}/tc/tc.netif.v1/state/namespaces"))
            .collect();
        let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        let snap = snapshot(&refs);
        let capped = pivot_flatten(
            &snap,
            "",
            Pivot::Address,
            &expand(&[]),
            "",
            5,
            Instant::now(),
        );
        assert_eq!(capped.rows.len(), 5);
        assert!(capped.truncated > 0);
        assert_eq!(
            capped.total_keys, 50,
            "the denominator is the bus, not the cap"
        );
    }

    /// #614 (FL7): pivot by address — each `system/service` one group, a
    /// deployment's services side by side; foreign keys, a v1 key among
    /// them, under "(foreign)".
    #[test]
    fn address_pivot_groups_services_and_foreigners() {
        let snap = snapshot(&[POSITION, STATUS, ROBOT2, ETH0, V1, "demo/example/foo"]);
        let flat = pivot(&snap, "", Pivot::Address, "");
        assert_eq!(
            tops(&flat),
            ["(foreign)", "host-a/tc", "robot1/nav", "robot2/nav"]
        );
        let i = flat
            .rows
            .iter()
            .position(|r| chunk_of(&flat, r) == "robot1/nav")
            .unwrap();
        let nav = flat.row(i);
        assert_eq!(nav.role, Some(Role::Address));
        assert_eq!(nav.subtree_count, 2, "both of robot1/nav's keys");
        assert_eq!(
            target_of(&flat, &flat.rows[i]).as_deref(),
            Some("zk2/robot1/nav"),
            "an address is one contiguous wire subtree"
        );
        assert_eq!(flat.total_keys, 6);
        assert!(
            row_by_chunk(&flat, "(foreign)").target.is_none(),
            "the foreign group has no wire subtree"
        );
    }

    /// #614 (FL7): pivot by system — every level one contiguous wire
    /// subtree, and the selector built from a system group is exactly that
    /// system's subtree.
    #[test]
    fn system_pivot_groups_target_their_real_prefix() {
        let snap = snapshot(&[POSITION, ROBOT2, ETH0]);
        let flat = pivot(&snap, "", Pivot::System, "position");
        assert_eq!(tops(&flat), ["robot1", "robot2"]);
        let system = row_by_chunk(&flat, "robot1");
        assert_eq!(system.role, Some(Role::System));
        let target = target_of(&flat, system);
        assert_eq!(target.as_deref(), Some("zk2/robot1"));
        assert_eq!(
            crate::scope::subtree_selector(target.as_deref().unwrap()),
            "zk2/robot1/**",
            "watching the group declares exactly the system's subtree"
        );
        let service = flat
            .rows
            .iter()
            .find(|r| chunk_of(&flat, r) == "nav" && r.role == Some(Role::Service))
            .expect("a service group");
        assert!(target_of(&flat, service).is_some_and(|t| t.ends_with("/nav")));
    }

    /// #614 (FL7): pivot by interface — every provider of `nav.v2` under
    /// it. The interface group spans addresses, so it offers no one-click
    /// watch; the address under it closes the chain.
    #[test]
    fn iface_pivot_spans_addresses_and_stays_untargeted() {
        let snap = snapshot(&[POSITION, ROBOT2, ETH0]);
        let flat = pivot(&snap, "", Pivot::Iface, "position");
        assert_eq!(tops(&flat), ["nav.v2"]);
        let group = row_by_chunk(&flat, "nav.v2");
        assert_eq!(group.role, Some(Role::Iface));
        assert!(
            group.target.is_none(),
            "spans addresses — no one-click watch"
        );
        let address = row_by_chunk(&flat, "robot2/nav");
        assert_eq!(
            target_of(&flat, address).as_deref(),
            Some("zk2/robot2/nav/nav.v2"),
            "address under interface is contiguous"
        );
        let leaf = row_by_chunk(&flat, "position");
        assert!(
            target_of(&flat, leaf).is_some_and(|t| t == POSITION || t == ROBOT2),
            "a leaf acts on its real key"
        );
    }

    /// #614 (FL7): pivot by kind — a kind group spans every interface, an
    /// interface under it every address; only the address closes the chain.
    #[test]
    fn kind_pivot_groups_by_token_and_targets_only_the_closed_chain() {
        let snap = snapshot(&[POSITION, STATUS, ETH0]);
        let flat = pivot(&snap, "", Pivot::Kind, "");
        assert_eq!(tops(&flat), ["state", "stream"]);
        assert!(row_by_chunk(&flat, "state").target.is_none());
        let all = pivot_flatten(
            &snap,
            "",
            Pivot::Kind,
            &BTreeSet::new(),
            "eth0",
            10_000,
            Instant::now(),
        );
        assert!(
            row_by_chunk(&all, "tc.netif.v1").target.is_none(),
            "spans addresses"
        );
        assert_eq!(
            target_of(&all, row_by_chunk(&all, "host-a/tc")).as_deref(),
            Some("zk2/host-a/tc/tc.netif.v1/state"),
            "kind, interface and address fixed: one subtree"
        );
        assert_eq!(
            target_of(&all, row_by_chunk(&all, "eth0")).as_deref(),
            Some(ETH0)
        );
    }

    /// Pivot leaves keep the real wire path as their action target, so
    /// select/watch built from a pivot row acts on the real subtree.
    #[test]
    fn pivot_leaves_target_their_real_path() {
        let snap = snapshot(&[ETH0]);
        let flat = pivot(&snap, "", Pivot::Address, "eth0");
        let leaf = row_by_chunk(&flat, "eth0");
        assert_eq!(target_of(&flat, leaf).as_deref(), Some(ETH0));
        assert_eq!(leaf.role, Some(Role::Resource));
        assert_eq!((flat.shown_keys, flat.total_keys), (1, 1));
    }

    /// A key that is both a concrete key and a prefix of deeper keys is its
    /// own entry — pivoting must not lose its traffic to its children.
    #[test]
    fn a_prefix_key_with_children_is_its_own_entry() {
        let snap = snapshot(&[
            "zk2/host-a/tc/tc.netif.v1/state/iface",
            "zk2/host-a/tc/tc.netif.v1/state/iface/eth0",
        ]);
        let flat = pivot(&snap, "", Pivot::System, "");
        assert_eq!(flat.total_keys, 2, "prefix key and deep key both count");
    }

    /// Control keys pivot too: their `@zk` chunks ride in the tail, under
    /// whatever coordinates they have, every level a real prefix.
    #[test]
    fn control_keys_pivot_under_their_address() {
        let instance = "zk2/robot1/nav/@zk/instance/0000000000000001";
        let snap = snapshot(&[instance, POSITION]);
        let flat = pivot(&snap, "", Pivot::Address, "instance");
        let zk = row_by_chunk(&flat, "@zk");
        assert_eq!(zk.role, Some(Role::Control));
        assert_eq!(target_of(&flat, zk).as_deref(), Some("zk2/robot1/nav/@zk"));
        assert_eq!(
            target_of(&flat, row_by_chunk(&flat, "0000000000000001")).as_deref(),
            Some(instance)
        );
    }

    /// Issue #93: a namespaced deployment keeps the namespace in the target
    /// — the selector still spells the wire, never a bare grammar path.
    #[test]
    fn pivot_targets_carry_the_namespace() {
        let key = format!("spray/{POSITION}");
        let snap = snapshot(&[&key]);
        let flat = pivot(&snap, "spray", Pivot::Address, "position");
        assert_eq!(
            target_of(&flat, row_by_chunk(&flat, "robot1/nav")).as_deref(),
            Some("spray/zk2/robot1/nav")
        );
        assert_eq!(
            target_of(&flat, row_by_chunk(&flat, "position")).as_deref(),
            Some(key.as_str())
        );
        // …and under another namespace the same key is foreign.
        let other = pivot(&snap, "acme", Pivot::Address, "");
        assert_eq!(tops(&other), ["(foreign)"]);
    }

    /// Issue #93: the press split — a concrete key that is also a prefix of
    /// deeper keys stays selectable; its marker (alone) expands.
    #[test]
    fn a_prefix_key_row_selects_and_its_marker_expands() {
        let prefix_key = "zk2/host-a/tc/tc.netif.v1/state/iface";
        let snap = snapshot(&[prefix_key, "zk2/host-a/tc/tc.netif.v1/state/iface/eth0"]);
        let set = expand_all(&["zk2/host-a/tc/tc.netif.v1/state"]);
        let flat = flat_now(&snap, "", &set, 100);
        let prefix = find_path(&flat, prefix_key).expect("prefix row");
        assert!(prefix.is_leaf && prefix.has_children);
        match row_press(prefix, &flat.arena) {
            Message::Subject(SubjectMsg::Select(Subject::Key(k))) => assert_eq!(k, prefix_key),
            other => panic!("body must select, got {other:?}"),
        }
        assert!(
            matches!(
                marker_press(prefix, &flat.arena),
                Some(Message::Workspace(WorkspaceMsg::ToggleNode(_)))
            ),
            "the marker is the expand affordance"
        );
        // A plain group row still toggles on body click.
        let group = find_path(&flat, "zk2/host-a/tc/tc.netif.v1/state").expect("group row");
        assert!(matches!(
            row_press(group, &flat.arena),
            Message::Workspace(WorkspaceMsg::ToggleNode(_))
        ));
    }

    /// Issue #93: expansion paths are namespaced per pivot, so switching
    /// pivots cannot resurrect an expansion that meant something else.
    #[test]
    fn pivot_expansion_paths_are_namespaced() {
        let snap = snapshot(&[POSITION]);
        for p in [Pivot::System, Pivot::Address, Pivot::Iface, Pivot::Kind] {
            let flat = pivot(&snap, "", p, "");
            assert!(
                path_of(&flat, &flat.rows[0]).starts_with(&format!("pivot:{}:", p.key())),
                "{p}"
            );
        }
    }

    /// The soak numbers behind issue #65's acceptance: flatten, search and
    /// pivot over a 50k-key tree. Run with
    /// `cargo test -p zengui --release -- --ignored soak_flatten --nocapture`.
    #[test]
    #[ignore = "soak bench — run explicitly, read the printed numbers"]
    fn soak_flatten_50k_keys() {
        let keys: Vec<String> = (0..50_000)
            .map(|i| {
                format!(
                    "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/g{}/k{i}",
                    i / 100
                )
            })
            .collect();
        let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        let snap = snapshot(&refs);
        // Everything expanded — the worst case the virtual window renders.
        let all = expand_all(&refs);
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
        let piv = pivot_flatten(&snap, "", Pivot::Address, &all, "", 60_000, now);
        println!(
            "pivot (address):   {} rows in {:?}",
            piv.rows.len(),
            t.elapsed()
        );
        assert_eq!(piv.total_keys, 50_000);

        // The frame cost is the window, not the row count.
        let (first, last) = kit::window(
            flat.rows.len(),
            kit::Viewport::scrolled(25_000.0 * ROW_HEIGHT, 900.0),
            ROW_HEIGHT,
        );
        println!(
            "window at mid-scroll: rows {first}..{last} of {}",
            flat.rows.len()
        );
        assert!(last - first < 100);
    }

    /// The scroll window is exact row arithmetic with overscan, clamped.
    #[test]
    fn the_window_is_bounded_and_covers_the_viewport() {
        let (first, last) = kit::window(50_000, kit::Viewport::default(), ROW_HEIGHT);
        assert_eq!(first, 0);
        assert!(
            (26..=60).contains(&last),
            "viewport rows + overscan: {last}"
        );

        let (first, last) = kit::window(
            50_000,
            kit::Viewport::scrolled(25_000.0 * ROW_HEIGHT, 600.0),
            ROW_HEIGHT,
        );
        assert!((25_000 - kit::OVERSCAN..=25_000).contains(&first));
        assert!(last > 25_000);

        let (first, last) =
            kit::window(10, kit::Viewport::scrolled(1_000_000.0, 600.0), ROW_HEIGHT);
        assert_eq!((first, last), (10, 10), "clamped past the end");
    }
}
