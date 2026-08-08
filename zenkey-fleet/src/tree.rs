//! The live key tree (issue #15): an immutable snapshot of everything the
//! monitor has seen, grouped by key chunks, with per-node statistics.
//!
//! Snapshots are rebuilt on the monitor's stats tick and published through
//! an `ArcSwap` — render loops *pull* the latest snapshot at their own pace
//! and never contend with the per-sample hot path (a hot bus cannot melt a
//! zengui redraw).

use std::collections::BTreeMap;
use std::time::Instant;

use crate::stats::{KeyStats, StatsTable};

/// Fold one key's stats into every node on its path (the node itself included).
fn accumulate(node: &mut TreeNode, s: &KeyStats) {
    node.subtree_count += s.count;
    node.subtree_bytes += s.bytes;
    node.subtree_rate_hz += s.rate_hz;
    node.subtree_keys += 1;
    node.subtree_last_seen = node.subtree_last_seen.max(Some(s.last_seen));
}

/// One node of the snapshot: a key chunk, its subtree, and — when a sample
/// has landed exactly here — its stats.
///
/// The `subtree_*` fields are what a **collapsed** node shows: a UI that can
/// only report the traffic of keys it happens to have expanded is reporting a
/// number the user will misread as the total.
#[derive(Debug, Clone, Default)]
pub struct TreeNode {
    pub children: BTreeMap<String, TreeNode>,
    /// Samples observed at exactly this key (leaf traffic).
    pub count: u64,
    pub bytes: u64,
    pub rate_hz: f64,
    /// When a sample last landed exactly here.
    pub last_seen: Option<Instant>,
    /// Aggregates over the whole subtree (this node included).
    pub subtree_count: u64,
    pub subtree_bytes: u64,
    pub subtree_rate_hz: f64,
    /// Most recent sample anywhere in the subtree.
    pub subtree_last_seen: Option<Instant>,
    /// Distinct keys that have carried traffic in this subtree.
    pub subtree_keys: usize,
}

/// An immutable point-in-time view of the observed keyspace.
#[derive(Debug, Clone, Default)]
pub struct KeyTreeSnapshot {
    pub root: TreeNode,
    pub keys: usize,
}

impl KeyTreeSnapshot {
    /// Build from the stats table (called on the stats tick, off the
    /// per-sample path).
    pub fn build(stats: &StatsTable) -> KeyTreeSnapshot {
        let mut root = TreeNode::default();
        for (key, s) in stats.iter() {
            let mut node = &mut root;
            accumulate(node, s);
            for chunk in key.split('/') {
                node = node.children.entry(chunk.to_string()).or_default();
                accumulate(node, s);
            }
            node.count = s.count;
            node.bytes = s.bytes;
            node.rate_hz = s.rate_hz;
            node.last_seen = Some(s.last_seen);
        }
        KeyTreeSnapshot {
            root,
            keys: stats.len(),
        }
    }

    /// Walk to a node by its chunk path.
    pub fn node(&self, path: &[&str]) -> Option<&TreeNode> {
        let mut node = &self.root;
        for chunk in path {
            node = node.children.get(*chunk)?;
        }
        Some(node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn builds_grouped_counts() {
        let mut stats = StatsTable::new();
        let now = Instant::now();
        stats.record("zs/v1/h-a/telemetry/x/m1", 4, None, now);
        stats.record("zs/v1/h-a/telemetry/x/m1", 4, None, now);
        stats.record("zs/v1/h-a/telemetry/x/m2", 4, None, now);
        stats.record("zs/v1/h-b/state/x/health", 4, None, now);

        let snap = KeyTreeSnapshot::build(&stats);
        assert_eq!(snap.keys, 3);
        assert_eq!(snap.root.subtree_count, 4);
        let telemetry = snap.node(&["zs", "v1", "h-a", "telemetry", "x"]).unwrap();
        assert_eq!(telemetry.subtree_count, 3);
        let m1 = snap
            .node(&["zs", "v1", "h-a", "telemetry", "x", "m1"])
            .unwrap();
        assert_eq!(m1.count, 2);
        assert_eq!(m1.bytes, 8);
        assert!(snap.node(&["zs", "v1", "h-c"]).is_none());
    }

    /// A collapsed node must be able to report its subtree's traffic — bytes
    /// and distinct keys, not only the sample count.
    #[test]
    fn collapsed_nodes_aggregate_their_subtree() {
        let mut stats = StatsTable::new();
        let now = Instant::now();
        stats.record("zs/v1/h-a/telemetry/x/m1", 4, None, now);
        stats.record("zs/v1/h-a/telemetry/x/m1", 4, None, now);
        stats.record("zs/v1/h-a/telemetry/x/m2", 10, None, now);
        stats.record("zs/v1/h-b/state/x/health", 7, None, now);

        let snap = KeyTreeSnapshot::build(&stats);
        let root = &snap.root;
        assert_eq!(root.subtree_count, 4);
        assert_eq!(root.subtree_bytes, 4 + 4 + 10 + 7);
        assert_eq!(root.subtree_keys, 3, "three distinct keys carried traffic");
        assert!(root.subtree_last_seen.is_some());
        // The root itself is not a leaf: no sample landed exactly there.
        assert_eq!(root.count, 0);
        assert_eq!(root.last_seen, None);

        let x = snap.node(&["zs", "v1", "h-a", "telemetry", "x"]).unwrap();
        assert_eq!(x.subtree_count, 3);
        assert_eq!(x.subtree_bytes, 18);
        assert_eq!(x.subtree_keys, 2);

        let m1 = snap
            .node(&["zs", "v1", "h-a", "telemetry", "x", "m1"])
            .unwrap();
        assert_eq!(m1.last_seen, Some(now));
        assert_eq!(m1.subtree_keys, 1, "a leaf counts only itself");
    }
}
