//! The live key tree (issue #15): an immutable snapshot of everything the
//! monitor has seen, grouped by key chunks, with per-node statistics.
//!
//! Snapshots are rebuilt on the monitor's stats tick and published through
//! an `ArcSwap` — render loops *pull* the latest snapshot at their own pace
//! and never contend with the per-sample hot path (a hot bus cannot melt a
//! zengui redraw).

use std::collections::BTreeMap;

use crate::stats::StatsTable;

/// One node of the snapshot: a key chunk, its subtree, and — when a sample
/// has landed exactly here — its stats.
#[derive(Debug, Clone, Default)]
pub struct TreeNode {
    pub children: BTreeMap<String, TreeNode>,
    /// Samples observed at exactly this key (leaf traffic).
    pub count: u64,
    pub bytes: u64,
    pub rate_hz: f64,
    /// Aggregate over the whole subtree (this node included).
    pub subtree_count: u64,
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
            node.subtree_count += s.count;
            for chunk in key.split('/') {
                node = node.children.entry(chunk.to_string()).or_default();
                node.subtree_count += s.count;
            }
            node.count = s.count;
            node.bytes = s.bytes;
            node.rate_hz = s.rate_hz;
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
}
