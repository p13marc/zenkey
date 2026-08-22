//! Interned tree paths (#251).
//!
//! A flatten used to spell every row's path as an owned `String` — and in the
//! non-pivot walk, twice, because `path` and `target` were byte-identical. At
//! 50,000 rows that is 150,000–200,000 allocations to produce a shape the
//! window draws forty of; the pivot walk paid it three times over, every tick,
//! because pivots get no shape cache (#177).
//!
//! The arena replaces the strings with two kinds of `u32` id:
//!
//! - a [`ChunkId`] names one distinct chunk spelling (`v1`, `telemetry`,
//!   `k4071`), interned once per flatten however many rows repeat it;
//! - a [`PathId`] names one path as a parent link plus a chunk — the same
//!   shape as the tree it flattens, so a path is one arena node, not one
//!   heap string per row.
//!
//! Display strings are materialised only where a human reads one: the ~40
//! rows [`view::tree::Flattened::row`](crate::view::tree::Flattened::row)
//! builds per frame, and the message built by a click. `Numbers::Live`'s
//! by-path lookup walks the parent chain instead of splitting a string.
//!
//! **Push-only, per flatten.** Every flatten builds a fresh arena and visits
//! each tree node exactly once, so [`PathArena::node`] never needs a
//! `(parent, chunk)` dedup map — and ids are only meaningful against the
//! arena of the `Flattened` that minted them. That is also why the expansion
//! set does *not* ride these ids (`expansion.rs`): it outlives every arena,
//! and its subtree pruning is a string-range trick that ids cannot spell.
//!
//! `TreeRow<'a>` borrowing the snapshot was rejected in #251 itself: the
//! engine's `ArcSwap` replaces the snapshot every tick and `Element<'a>`
//! borrows `&self`, so rows must outlive the swap. Interning gives the same
//! win without the lifetime knot.

use std::collections::HashMap;

/// One distinct chunk spelling, interned by [`PathArena::chunk`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChunkId(u32);

/// One path: an index into the arena's parent-linked nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PathId(u32);

/// A path is its parent plus one chunk — the tree's own shape, kept.
#[derive(Debug, Clone, Copy)]
struct PathNode {
    parent: Option<PathId>,
    chunk: ChunkId,
}

/// The paths of one flatten, interned.
#[derive(Debug, Clone, Default)]
pub struct PathArena {
    nodes: Vec<PathNode>,
    /// Id → spelling. The interner's other half.
    chunks: Vec<Box<str>>,
    /// Spelling → id. Holds its own copy of the string: a self-borrowing
    /// map is not worth `unsafe`, and distinct chunks are the small set —
    /// rows repeat them, which is the whole point.
    index: HashMap<Box<str>, ChunkId>,
}

impl PathArena {
    pub fn new() -> PathArena {
        PathArena::default()
    }

    /// Intern one chunk spelling. Idempotent; allocates only on first sight.
    pub fn chunk(&mut self, s: &str) -> ChunkId {
        if let Some(&id) = self.index.get(s) {
            return id;
        }
        let id = ChunkId(u32::try_from(self.chunks.len()).expect("chunk count exceeds u32"));
        let owned: Box<str> = s.into();
        self.chunks.push(owned.clone());
        self.index.insert(owned, id);
        id
    }

    /// The spelling behind an id.
    pub fn chunk_str(&self, id: ChunkId) -> &str {
        &self.chunks[id.0 as usize]
    }

    /// Append one path node. Push-only — see the module docs for why no
    /// `(parent, chunk)` dedup is needed.
    pub fn node(&mut self, parent: Option<PathId>, chunk: ChunkId) -> PathId {
        let id = PathId(u32::try_from(self.nodes.len()).expect("path count exceeds u32"));
        self.nodes.push(PathNode { parent, chunk });
        id
    }

    /// The path's last chunk — what a row displays.
    pub fn chunk_of(&self, id: PathId) -> ChunkId {
        self.nodes[id.0 as usize].chunk
    }

    /// Chunk count of a path. Chains are key-deep (~7), so a walk beats a
    /// stored length no one else needs.
    pub fn len_of(&self, id: PathId) -> usize {
        let mut n = 1;
        let mut cur = self.nodes[id.0 as usize].parent;
        while let Some(p) = cur {
            n += 1;
            cur = self.nodes[p.0 as usize].parent;
        }
        n
    }

    /// The ancestor spelling the first `len` chunks of `id` — what
    /// `real[..len].join("/")` used to allocate. `len` is clamped to the
    /// path's own length, matching the old slice's `.min(real.len())`.
    pub fn ancestor_at(&self, id: PathId, len: usize) -> PathId {
        let mut depth = self.len_of(id);
        let mut cur = id;
        while depth > len.max(1) {
            cur = self.nodes[cur.0 as usize]
                .parent
                .expect("len_of counted this link");
            depth -= 1;
        }
        cur
    }

    /// Append the `/`-joined display path to `out`. Recursive so the
    /// root-first order costs no scratch buffer.
    pub fn write_display(&self, id: PathId, out: &mut String) {
        let node = self.nodes[id.0 as usize];
        if let Some(p) = node.parent {
            self.write_display(p, out);
            out.push('/');
        }
        out.push_str(self.chunk_str(node.chunk));
    }

    /// The display path, materialised. Per frame this runs ~40 times; per
    /// click, once — never per tree node.
    pub fn display(&self, id: PathId) -> String {
        let mut out = String::new();
        self.write_display(id, &mut out);
        out
    }

    /// Root-first chunk spellings — what `KeyTreeSnapshot::node` takes.
    /// Replaces the `split('/')` that re-derived them from a display string.
    pub fn chunks_of(&self, id: PathId) -> Vec<&str> {
        let mut out = Vec::new();
        let mut cur = Some(id);
        while let Some(c) = cur {
            let node = self.nodes[c.0 as usize];
            out.push(self.chunk_str(node.chunk));
            cur = node.parent;
        }
        out.reverse();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intern(arena: &mut PathArena, path: &str) -> PathId {
        let mut parent = None;
        for chunk in path.split('/') {
            let c = arena.chunk(chunk);
            parent = Some(arena.node(parent, c));
        }
        parent.expect("non-empty path")
    }

    #[test]
    fn a_path_round_trips_through_ids() {
        let mut arena = PathArena::new();
        let id = intern(&mut arena, "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu");
        assert_eq!(arena.display(id), "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu");
        assert_eq!(arena.chunk_str(arena.chunk_of(id)), "cpu");
        assert_eq!(arena.len_of(id), 5);
        assert_eq!(
            arena.chunks_of(id),
            ["v1", "h-3fa9c2d41b7e", "telemetry", "sysinfo", "cpu"]
        );
    }

    /// Interning is the point: a chunk repeated across rows is one string.
    #[test]
    fn a_repeated_chunk_is_one_id() {
        let mut arena = PathArena::new();
        let a = arena.chunk("telemetry");
        let b = arena.chunk("telemetry");
        assert_eq!(a, b);
        assert_eq!(arena.chunks.len(), 1);
    }

    /// `ancestor_at` replaces `real[..n].join("/")`, clamp included.
    #[test]
    fn ancestors_spell_the_old_prefix_joins() {
        let mut arena = PathArena::new();
        let id = intern(&mut arena, "zs/v1/h-3fa9c2d41b7e/telemetry");
        assert_eq!(
            arena.display(arena.ancestor_at(id, 3)),
            "zs/v1/h-3fa9c2d41b7e"
        );
        assert_eq!(arena.display(arena.ancestor_at(id, 1)), "zs");
        // Clamped, exactly as the slice's `.min(real.len())` was.
        assert_eq!(
            arena.display(arena.ancestor_at(id, 99)),
            "zs/v1/h-3fa9c2d41b7e/telemetry"
        );
    }

    /// A pivot's synthetic first level is one chunk containing colons; the
    /// display join must reproduce the expansion set's spelling exactly.
    #[test]
    fn a_synthetic_pivot_chunk_displays_verbatim() {
        let mut arena = PathArena::new();
        let root = arena.chunk("pivot:origin:@catalog");
        let state = arena.chunk("state");
        let top = arena.node(None, root);
        let deep = arena.node(Some(top), state);
        assert_eq!(arena.display(deep), "pivot:origin:@catalog/state");
    }
}
