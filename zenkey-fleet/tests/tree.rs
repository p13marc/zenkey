//! The observed key tree (issue #203).
//!
//! `KeyTreeSnapshot` is the whole left pane of zengui and the grouping behind
//! `zenctl topic list`, and it had no test file. What matters here is not that
//! it counts, but that it counts **two different things and never conflates
//! them**: `count` is traffic that landed on exactly this key, `subtree_count`
//! is everything at or below it. The type's own doc says the confusion is "a
//! number the user will misread as the total"; this is what stops that being a
//! comment.
//!
//! No bus: building a snapshot is pure over a `StatsTable`.

use std::time::{Duration, Instant};

use zenkey_fleet::KeyTreeSnapshot;
use zenkey_fleet::stats::StatsTable;

fn table(keys: &[(&str, usize)]) -> StatsTable {
    let mut t = StatsTable::new();
    let now = Instant::now();
    for (i, (key, bytes)) in keys.iter().enumerate() {
        t.record(
            key,
            *bytes,
            None,
            now + Duration::from_millis(i as u64),
            None,
            None,
        );
    }
    t
}

#[test]
fn own_traffic_and_subtree_traffic_are_different_numbers() {
    // A key that is also a prefix of another: `…/disk` carries traffic itself
    // *and* has a child. The two counts must not merge.
    let snap = KeyTreeSnapshot::build(&table(&[
        ("v1/h-a/telemetry/sysinfo/disk", 10),
        ("v1/h-a/telemetry/sysinfo/disk/used", 20),
        ("v1/h-a/telemetry/sysinfo/disk/free", 30),
    ]));

    let disk = snap
        .node(&["v1", "h-a", "telemetry", "sysinfo", "disk"])
        .expect("the prefix is a node");
    assert_eq!(disk.count, 1, "one sample landed on exactly this key");
    assert_eq!(disk.bytes, 10);
    assert_eq!(
        disk.subtree_count, 3,
        "and three landed at or below it — a different fact"
    );
    assert_eq!(disk.subtree_bytes, 60);
    assert_eq!(
        disk.subtree_keys, 3,
        "distinct keys carrying traffic, this one included"
    );

    let used = snap
        .node(&["v1", "h-a", "telemetry", "sysinfo", "disk", "used"])
        .expect("the leaf is a node");
    assert_eq!((used.count, used.subtree_count), (1, 1));
    assert_eq!(
        used.subtree_keys, 1,
        "a leaf's subtree is itself, not zero and not its parent's"
    );

    // The root carries the table's fold, which is what lets a render loop
    // report totals without taking the ingest lock.
    assert_eq!(snap.root.subtree_count, 3);
    assert_eq!(snap.root.subtree_bytes, 60);
    assert_eq!(snap.keys, 3);
}

#[test]
fn the_grouping_is_stable_under_insertion_order() {
    let forward = KeyTreeSnapshot::build(&table(&[
        ("v1/h-a/telemetry/p/a", 1),
        ("v1/h-a/telemetry/p/b", 2),
        ("v1/h-b/telemetry/p/c", 4),
    ]));
    let backward = KeyTreeSnapshot::build(&table(&[
        ("v1/h-b/telemetry/p/c", 4),
        ("v1/h-a/telemetry/p/b", 2),
        ("v1/h-a/telemetry/p/a", 1),
    ]));

    let names = |s: &KeyTreeSnapshot, path: &[&str]| -> Vec<String> {
        s.node(path)
            .expect("node")
            .children
            .keys()
            .cloned()
            .collect()
    };
    assert_eq!(names(&forward, &["v1"]), names(&backward, &["v1"]));
    assert_eq!(
        names(&forward, &["v1", "h-a", "telemetry", "p"]),
        ["a", "b"],
        "children are ordered by key, not by arrival"
    );
    assert_eq!(forward.root.subtree_bytes, backward.root.subtree_bytes);
    assert_eq!(forward.keys, backward.keys);
}

#[test]
fn the_snapshot_carries_the_bounds_cost_with_it() {
    let mut t = StatsTable::with_capacity(2);
    let now = Instant::now();
    for i in 0..4 {
        t.record(
            &format!("v1/h-a/telemetry/p/k{i}"),
            8,
            None,
            now + Duration::from_millis(i),
            None,
            None,
        );
    }
    let snap = KeyTreeSnapshot::build(&t);
    assert_eq!(snap.keys, 2, "the table's bound holds");
    assert_eq!(
        snap.evicted, 2,
        "and the snapshot carries what it cost, so a render loop can say so \
         without reaching back into the table (O6)"
    );
}

#[test]
fn an_absent_path_is_none_rather_than_an_empty_node() {
    let snap = KeyTreeSnapshot::build(&table(&[("v1/h-a/telemetry/p/m", 4)]));
    assert!(snap.node(&["v1", "h-a", "telemetry", "p", "m"]).is_some());
    assert!(
        snap.node(&["v1", "h-a", "telemetry", "p", "nope"])
            .is_none(),
        "a path nothing has published is absent, not a zeroed node — the \
         difference between 'no traffic here' and 'no such key' (O4)"
    );
    assert!(snap.node(&["demo", "foreign"]).is_none());
}
