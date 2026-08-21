//! Which tree nodes are open — and, as of #179, which are *not* remembered.
//!
//! The set was a bare `BTreeSet<String>` that only ever grew. Collapsing a node
//! removed that node and left every descendant behind, invisible and immortal:
//! open `v1/h-…/state/sysinfo`, collapse `v1`, and four entries stay for the
//! rest of the session. Pivot views multiply that by four, because the same
//! wire node is namespaced by string concatenation once per pivot
//! (`pivot:origin:…`, `view/tree.rs`).
//!
//! It also survived a deployment change. `App::forget_deployment` cleared ten
//! collections and not this one, so switching base left expansion state keyed
//! to paths that no longer exist — a leak, and a correctness smell: a stale
//! path re-expands a coincidentally matching new subtree.
//!
//! ## Collapsing takes the subtree with it
//!
//! That is a deliberate trade and worth stating, because it is a behaviour
//! change: collapse `v1` and reopen it, and the nodes that were open beneath it
//! are closed. The alternative is the set that grows forever, and it is the
//! same mechanism as the stale re-expansion above — a remembered descendant is
//! indistinguishable from a leaked one. #179 asks for a set that is
//! O(currently-open), and this is what makes it one.
//!
//! Interning the paths is **not** here: `HashSet<PathId>` over a `PathArena`
//! is #177's, and it is a much larger change. This is the part that needs none
//! of it.

use std::collections::BTreeSet;

/// The open nodes, by display path.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Expansion(BTreeSet<String>);

impl Expansion {
    pub fn new() -> Expansion {
        Expansion::default()
    }

    /// Open a node. Idempotent — the two auto-expand sites in `app.rs` call it
    /// on a prefix that may already be open.
    pub fn open(&mut self, path: impl Into<String>) {
        self.0.insert(path.into());
    }

    /// Open a closed node, or close an open one **with everything under it**.
    pub fn toggle(&mut self, path: &str) {
        if self.0.remove(path) {
            self.close_descendants(path);
        } else {
            self.0.insert(path.to_string());
        }
    }

    /// Everything, on a base change or a reconnect.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Drop every descendant of `path`.
    ///
    /// A range rather than a scan, and the bounds are exact rather than a
    /// `starts_with` filter. Descendants are precisely the keys in
    /// `["{path}/", "{path}0")`: `'0'` is the byte after `'/'`, so the
    /// half-open range holds every string beginning `"{path}/"` and nothing
    /// else.
    ///
    /// `take_while(starts_with)` would be wrong here, and subtly: a sibling
    /// like `a/b-x` sorts *between* `a/b` and `a/b/c`, because `'-'` is below
    /// `'/'` — and producer chunks contain `-` (`h-3fa9c2d41b7e`). The scan
    /// would stop at the sibling and leave the real descendants behind.
    fn close_descendants(&mut self, path: &str) {
        let doomed: Vec<String> = self
            .0
            .range(format!("{path}/")..format!("{path}0"))
            .cloned()
            .collect();
        for d in doomed {
            self.0.remove(&d);
        }
    }
}

impl std::ops::Deref for Expansion {
    type Target = BTreeSet<String>;
    fn deref(&self) -> &BTreeSet<String> {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The leak, stated as a test: what is open beneath a node closes with it,
    /// so the set is O(currently-open) rather than O(ever-opened).
    #[test]
    fn collapsing_a_node_takes_its_subtree_with_it() {
        let mut e = Expansion::new();
        for p in [
            "v1",
            "v1/h-3fa9c2d41b7e",
            "v1/h-3fa9c2d41b7e/state",
            "v1/h-3fa9c2d41b7e/state/sysinfo",
        ] {
            e.open(p);
        }
        assert_eq!(e.len(), 4);
        e.toggle("v1");
        assert!(e.is_empty(), "four entries, not one: {:?}", e.iter());
    }

    /// A sibling is not a descendant — and this is the case a `starts_with`
    /// scan gets wrong, because `'-'` sorts below `'/'` and producer chunks
    /// contain it.
    #[test]
    fn a_sibling_that_sorts_between_parent_and_child_survives() {
        let mut e = Expansion::new();
        for p in ["a/b", "a/b-x", "a/b-x/deep", "a/b/c", "a/b2"] {
            e.open(p);
        }
        e.toggle("a/b");
        let left: Vec<&String> = e.iter().collect();
        assert_eq!(
            left,
            ["a/b-x", "a/b-x/deep", "a/b2"],
            "only `a/b` and `a/b/c` are gone"
        );
    }

    /// Pivot rows namespace the first level by string concatenation and use
    /// `/` below it, so one rule covers both — which is the whole reason the
    /// range is written against `/` and not against the pivot prefix.
    #[test]
    fn a_pivot_subtree_collapses_like_any_other() {
        let mut e = Expansion::new();
        e.open("pivot:origin:h-3fa9c2d41b7e");
        e.open("pivot:origin:h-3fa9c2d41b7e/state");
        e.open("pivot:producer:sysinfo");
        e.toggle("pivot:origin:h-3fa9c2d41b7e");
        let left: Vec<&String> = e.iter().collect();
        assert_eq!(
            left,
            ["pivot:producer:sysinfo"],
            "the other pivot's copy of the same wire node is untouched"
        );
    }

    /// Toggling twice reopens the node and nothing else — the trade this type
    /// makes, spelled out so it cannot be reverted by accident.
    #[test]
    fn reopening_does_not_restore_what_was_beneath() {
        let mut e = Expansion::new();
        e.open("v1");
        e.open("v1/deep");
        e.toggle("v1");
        e.toggle("v1");
        let left: Vec<&String> = e.iter().collect();
        assert_eq!(left, ["v1"]);
    }

    /// Opening is idempotent: `app.rs` auto-expands a prefix that may already
    /// be open, on every seed and every reveal.
    #[test]
    fn opening_an_open_node_changes_nothing() {
        let mut e = Expansion::new();
        e.open("v1");
        e.open("v1");
        assert_eq!(e.len(), 1);
    }
}
