//! Payload diff (issue #203): what moved between two values of one key,
//! which is *the* question for a `state` document. It needs no bus. v1's
//! registry diff, the served slice against a checkout's, left with the v1
//! registry (#612, FJ9).

use serde_json::json;
use zenkey_fleet::model::diff::diff;
use zenkey_fleet::{Change, byte_diff};

#[test]
fn a_structural_diff_names_the_path_that_moved() {
    let old = json!({"disk": {"used": 10, "free": 90}, "host": "a"});
    let new = json!({"disk": {"used": 20, "free": 90}, "host": "a"});
    let d = diff(&old, &new, 20);

    assert_eq!(d.changes.len(), 1, "only one field moved: {:?}", d.changes);
    match &d.changes[0] {
        Change::Changed { path, old, new } => {
            assert_eq!(path, "disk.used", "addressed by dotted path, not index");
            assert_eq!((old, new), (&json!(10), &json!(20)));
        }
        other => panic!("expected a change, got {other:?}"),
    }
    assert_eq!(d.truncated, 0);
    assert!(diff(&old, &old, 20).is_empty(), "no change is not a change");
}

#[test]
fn added_and_removed_are_not_the_same_as_changed() {
    let old = json!({"a": 1, "gone": true});
    let new = json!({"a": 1, "fresh": "x"});
    let d = diff(&old, &new, 20);

    let mut paths: Vec<(&str, &'static str)> = d
        .changes
        .iter()
        .map(|c| {
            (
                c.path(),
                match c {
                    Change::Added { .. } => "added",
                    Change::Removed { .. } => "removed",
                    Change::Changed { .. } => "changed",
                },
            )
        })
        .collect();
    paths.sort();
    assert_eq!(paths, [("fresh", "added"), ("gone", "removed")]);
}

/// O6 applied to a diff: a bounded view reports what it dropped. A diff that
/// quietly stops at the cap reads as "and nothing else changed".
#[test]
fn a_capped_diff_counts_what_it_did_not_list() {
    let old = json!({"a": 0, "b": 0, "c": 0, "d": 0, "e": 0});
    let new = json!({"a": 1, "b": 1, "c": 1, "d": 1, "e": 1});
    let d = diff(&old, &new, 2);
    assert_eq!(d.changes.len(), 2, "the cap holds");
    assert_eq!(d.truncated, 3, "and the rest are counted, not dropped");
    assert!(!d.is_empty());
}

#[test]
fn a_byte_diff_says_it_is_a_byte_diff() {
    let d = byte_diff(b"hello world", b"hello there");
    assert_eq!(d.common_prefix, 6, "`hello ` is shared");
    assert_eq!((d.old_len, d.new_len), (11, 11));
    assert!(!d.is_empty());
    let (old_range, new_range) = d.ranges();
    assert_eq!(old_range.start, 6);
    assert_eq!(new_range.start, 6);

    assert!(
        byte_diff(b"same", b"same").is_empty(),
        "identical bytes are not a difference"
    );
    // A payload that only grew: the whole of the old is a common prefix.
    let grew = byte_diff(b"ab", b"abcd");
    assert_eq!(grew.common_prefix, 2);
    assert_eq!((grew.old_len, grew.new_len), (2, 4));
}
