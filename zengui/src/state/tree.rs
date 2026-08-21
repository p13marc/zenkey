//! The key tree: its rows, how they are grouped, and what is open.
//!
//! `pivot`, `tree_search` and `tree_scroll` are the tree by nature and the
//! shell by writer — the toolbar's pivot picker and find box write them.
//! Resolved **by nature**: `update_workspace` takes `&mut TreeState`, which is
//! the honest statement that the shell owns the tree\'s chrome while the tree
//! owns its rows.
//!
//! `expanded` is the one field a deployment change must clear that is not in
//! this group\'s own gift (#179): a stale path re-expands a coincidentally
//! matching new subtree.

use std::sync::Arc;

use zenkey_fleet::{KeyTreeSnapshot, MergedNode, Skeleton, SliceSet};

use super::deployment::Deployment;
use super::observation::Observation;
use crate::view;

/// Cap on tree rows built per flatten. With virtualized rendering (issue
/// #65) this is a memory bound, not a display truncation — the view builds
/// only the scrolled-into window.
///
/// It is now true of all three flatten paths. `search_flatten` and
/// `pivot_flatten` used to apply it *after* the walk, so the sentence above
/// described one path of three and the "rows not built" the pane prints was
/// false in the other two (#249).
pub(crate) const MAX_ROWS: usize = 50_000;

/// A merge, and the three inputs it was made from (#177).
///
/// All three are `Arc`s the app replaces wholesale, so validity is three
/// pointer comparisons — no content hashing, and no chance of a key that
/// compares equal while the tree behind it moved.
pub(crate) struct MergedCache {
    pub(crate) skeleton: Option<Arc<Skeleton>>,
    pub(crate) observed: Arc<KeyTreeSnapshot>,
    pub(crate) watched: Arc<[String]>,
    pub(crate) merged: Arc<MergedNode>,
}

sub_state! {
    pub(crate) struct TreeState {
        pub(crate) flat: view::tree::Flattened,
        /// How the tree groups its entries (issue #65).
        pub(crate) pivot: view::tree::Pivot,
        /// The find-in-tree query; empty = no filter.
        pub(crate) tree_search: String,
        /// Scroll position + viewport height, driving the virtual window.
        pub(crate) tree_scroll: (f32, f32),
        pub(crate) expanded: crate::expansion::Expansion,
        /// The last merge, kept while its three inputs are unchanged (#177).
        ///
        /// `reflatten` is `merge` **plus** a flatten, and the merge is the larger
        /// half — 24.97 ms against 22.08 ms at 50,000 keys. Expanding a node,
        /// collapsing one, typing in the find box and switching pivot all change
        /// only *how the tree is presented*: the skeleton, the observed snapshot
        /// and the watch set are identical, so the merge is pure repetition. Now it
        /// is paid once and the interaction costs a flatten.
        ///
        /// Legitimate precisely because of the shape/statistics split: `merge`'s
        /// `stats` field is read only for `is_entry`, which is structural for the
        /// reason `RowShape` documents. A cached merge cannot show stale numbers,
        /// because the numbers no longer come through it.
        ///
        /// The trade, stated: one merged tree is retained between reflattens rather
        /// than built and dropped each time. Peak is unchanged — today's transient
        /// peak is the same figure — but it is not returned. If that ever matters,
        /// drop this field and pay a merge per expand again.
        pub(crate) merged_cache: Option<MergedCache>,
        /// Ticks that reused the cached tree shape, and ticks that rebuilt it
        /// (#177).
        ///
        /// Not surfaced anywhere, and deliberately: the honesty rule is that a
        /// *bound* reports what it cost, and this cache hides nothing — it is
        /// either exactly correct or it is a bug. So the guarantee belongs in a
        /// test, and criterion cannot express it because it cannot count
        /// allocations. `steady_state_ticks_reuse_the_tree_shape` reads these.
        pub(crate) shape_reused: u64,
        pub(crate) shape_rebuilt: u64,
    }
}

impl Default for TreeState {
    fn default() -> TreeState {
        TreeState {
            flat: view::tree::Flattened::empty(),
            pivot: view::tree::Pivot::default(),
            tree_search: String::new(),
            tree_scroll: (0.0, 600.0),
            expanded: crate::expansion::Expansion::new(),
            merged_cache: None,
            shape_reused: 0,
            shape_rebuilt: 0,
        }
    }
}

impl TreeState {
    /// The merged declared-∪-observed tree, from cache when its inputs have
    /// not moved (#177).
    pub(crate) fn merged(&mut self, dep: &Deployment, obs: &Observation) -> Arc<MergedNode> {
        let same_skeleton = |a: &Option<Arc<Skeleton>>| match (a, &dep.skeleton) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        let fresh = self.merged_cache.as_ref().is_some_and(|c| {
            same_skeleton(&c.skeleton)
                && Arc::ptr_eq(&c.observed, &obs.observed)
                && Arc::ptr_eq(&c.watched, &obs.watched)
        });
        if let Some(c) = &self.merged_cache
            && fresh
        {
            return Arc::clone(&c.merged);
        }
        // No registry has answered yet, so there is nothing declared to merge
        // against. This used to be rebuilt on *every* `reflatten` just to have
        // an empty tree to hand the merge; now it is built only when the merge
        // itself is.
        let empty_skeleton;
        let skeleton = match &dep.skeleton {
            Some(s) => s.as_ref(),
            None => {
                empty_skeleton = Skeleton::build(
                    dep.base(),
                    &SliceSet::default(),
                    &std::collections::BTreeMap::new(),
                    None,
                );
                &empty_skeleton
            }
        };
        let merged = Arc::new(zenkey_fleet::skeleton::merge(
            skeleton,
            &obs.observed,
            &obs.watched,
        ));
        self.merged_cache = Some(MergedCache {
            skeleton: dep.skeleton.clone(),
            observed: Arc::clone(&obs.observed),
            watched: Arc::clone(&obs.watched),
            merged: Arc::clone(&merged),
        });
        merged
    }

    pub(crate) fn reflatten(&mut self, dep: &Deployment, obs: &Observation) {
        let merged = self.merged(dep, obs);
        let now = std::time::Instant::now();
        // Pivot and filter re-key the flattened entries here — the hot tree
        // stays registry-blind (issue #65).
        self.flat = match (self.pivot, self.tree_search.is_empty()) {
            (view::tree::Pivot::Chunks, true) => {
                view::tree::flatten(&merged, dep.base(), &self.expanded, MAX_ROWS, now)
            }
            (view::tree::Pivot::Chunks, false) => {
                view::tree::search_flatten(&merged, dep.base(), &self.tree_search, MAX_ROWS, now)
            }
            (pivot, _) => view::tree::pivot_flatten(
                &merged,
                dep.base(),
                pivot,
                &self.expanded,
                &self.tree_search,
                MAX_ROWS,
                now,
            ),
        };
        // Wire shapes track the live snapshot from here on, so a tick that
        // changes only the numbers can point them at the new one instead of
        // walking the tree again (#177). Pivot shapes refuse, and say so.
        self.flat
            .retarget(std::sync::Arc::clone(&obs.observed), now);
    }
}

/// Whether the tree's *shape* can have survived this tick (#177).
///
/// Two rungs, and both are needed.
///
/// **The key-set triple.** Keys leave `StatsTable` by exactly two routes —
/// `retire_unwatched` bumps `unwatched`, `evict` bumps `evicted` — and enter by
/// `record`, which moves `len()`. So any change to the set moves at least one
/// member. The comparison is equality rather than `>`, so a replay seeking
/// backwards trips it too.
///
/// **The watch set's identity.** `is_covered` is what flips a node's
/// `NodeStatus` when a watch is released, so the triple alone would freeze the
/// unwatched badge. `link.rs` reassigns `watched` *only* on a `WatchChanged`
/// event and hands out `Arc::clone` every tick, so pointer identity is an exact
/// test that costs nothing.
///
/// That second rung also covers a hazard: `Message::Subject(SubjectMsg::WatchReleased)` does not
/// reflatten — it returns `Task::none()` and has always relied on the
/// unconditional tick rebuild. Releasing a watch fires `WatchChanged`, which
/// swaps the `Arc`, which lands here as `false`.
pub(crate) fn shape_held(
    prev: (usize, u64, u64),
    next: (usize, u64, u64),
    prev_watched: &std::sync::Arc<[String]>,
    next_watched: &std::sync::Arc<[String]>,
) -> bool {
    prev == next && std::sync::Arc::ptr_eq(prev_watched, next_watched)
}
