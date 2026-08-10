//! Payload diff (issue #63): what changed between two consecutive values of
//! one key.
//!
//! For a keyspace-v2 `state` key this is *the* question — state is a LWW
//! document, so "what moved" is what a reader actually wants, and re-reading
//! two pretty-printed payloads side by side to find it is the thing explorers
//! are supposed to save you from.
//!
//! Two levels, and which one ran is never hidden:
//!
//! - [`diff`] over two structural values ([`crate::decode::structural_value`])
//!   — named fields, added/removed/changed;
//! - [`byte_diff`] when either side has no structural form (plain text, a
//!   protobuf frame, an opaque blob) — the honest fallback, reporting how much
//!   of the two byte strings is common rather than pretending to name fields.
//!
//! Deliberately no notion of `Put` vs `Delete`: a tombstone is not a value and
//! diffing it against one would be a category error. `SampleView::kind` is
//! exact, and the frontend words the retirement.

use serde_json::Value;

/// How deep a value is walked before the diff stops descending.
///
/// A bus carries whatever a foreign publisher sends, including deeply nested
/// or self-referential-looking documents; the recursion is bounded for the
/// same reason `zenkey`'s CDR resolver is. Past the bound, the subtree is
/// compared whole and reported as one change.
const MAX_DEPTH: usize = 32;

/// One field-level difference, addressed by a dotted path (`disk.used`,
/// `items.0.name`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// The path is present in the new value and absent from the old.
    Added { path: String, new: Value },
    /// The path is present in the old value and absent from the new.
    Removed { path: String, old: Value },
    /// The path is in both and its value moved.
    Changed {
        path: String,
        old: Value,
        new: Value,
    },
}

impl Change {
    pub fn path(&self) -> &str {
        match self {
            Change::Added { path, .. }
            | Change::Removed { path, .. }
            | Change::Changed { path, .. } => path,
        }
    }
}

/// The result of comparing two structural values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValueDiff {
    pub changes: Vec<Change>,
    /// Changes found past `max_changes` and therefore not listed.
    ///
    /// Counted rather than silently cut: a bounded view that reports what it
    /// dropped is the RFC 09 §5.1 O6 rule, and a diff that quietly stops at
    /// twenty entries reads as "and nothing else changed".
    pub truncated: usize,
}

impl ValueDiff {
    /// No change at all — distinct from "we did not look".
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.truncated == 0
    }
}

/// Structurally compare two values, listing at most `max_changes` differences
/// and counting the rest.
pub fn diff(old: &Value, new: &Value, max_changes: usize) -> ValueDiff {
    let mut out = ValueDiff::default();
    walk(String::new(), old, new, max_changes, 0, &mut out);
    out
}

/// Record a change, or count it as truncated once the budget is spent.
fn push(out: &mut ValueDiff, max_changes: usize, change: Change) {
    if out.changes.len() < max_changes {
        out.changes.push(change);
    } else {
        out.truncated += 1;
    }
}

fn join(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

fn walk(
    path: String,
    old: &Value,
    new: &Value,
    max_changes: usize,
    depth: usize,
    out: &mut ValueDiff,
) {
    if old == new {
        return;
    }
    if depth >= MAX_DEPTH {
        push(
            out,
            max_changes,
            Change::Changed {
                path,
                old: old.clone(),
                new: new.clone(),
            },
        );
        return;
    }
    match (old, new) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, av) in a {
                match b.get(k) {
                    Some(bv) => walk(join(&path, k), av, bv, max_changes, depth + 1, out),
                    None => push(
                        out,
                        max_changes,
                        Change::Removed {
                            path: join(&path, k),
                            old: av.clone(),
                        },
                    ),
                }
            }
            for (k, bv) in b {
                if !a.contains_key(k) {
                    push(
                        out,
                        max_changes,
                        Change::Added {
                            path: join(&path, k),
                            new: bv.clone(),
                        },
                    );
                }
            }
        }
        // Arrays are compared by index, not by identity: a bus payload's array
        // is a positional field list far more often than it is a set, and
        // guessing an element identity would invent moves that never happened.
        (Value::Array(a), Value::Array(b)) => {
            for (i, (av, bv)) in a.iter().zip(b.iter()).enumerate() {
                walk(
                    join(&path, &i.to_string()),
                    av,
                    bv,
                    max_changes,
                    depth + 1,
                    out,
                );
            }
            for (i, av) in a.iter().enumerate().skip(b.len()) {
                push(
                    out,
                    max_changes,
                    Change::Removed {
                        path: join(&path, &i.to_string()),
                        old: av.clone(),
                    },
                );
            }
            for (i, bv) in b.iter().enumerate().skip(a.len()) {
                push(
                    out,
                    max_changes,
                    Change::Added {
                        path: join(&path, &i.to_string()),
                        new: bv.clone(),
                    },
                );
            }
        }
        // Scalars, and any shape change (object → array, number → string):
        // one change at this path, carrying both sides.
        _ => push(
            out,
            max_changes,
            Change::Changed {
                path,
                old: old.clone(),
                new: new.clone(),
            },
        ),
    }
}

/// What a byte comparison can honestly say when neither side is structural.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ByteDiff {
    pub common_prefix: usize,
    pub common_suffix: usize,
    pub old_len: usize,
    pub new_len: usize,
}

impl ByteDiff {
    /// True when the two byte strings are identical.
    pub fn is_empty(&self) -> bool {
        self.old_len == self.new_len && self.common_prefix == self.old_len
    }

    /// The half-open byte range that differs on each side: `(old, new)`.
    ///
    /// Both start at `common_prefix`; both end where the common suffix begins.
    pub fn ranges(&self) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
        (
            self.common_prefix..self.old_len - self.common_suffix,
            self.common_prefix..self.new_len - self.common_suffix,
        )
    }
}

/// Compare two payloads as bytes — the fallback when no structural form exists.
pub fn byte_diff(old: &[u8], new: &[u8]) -> ByteDiff {
    let common_prefix = old
        .iter()
        .zip(new.iter())
        .take_while(|(a, b)| a == b)
        .count();
    // The suffix must not overlap the prefix, or a run of one repeated byte
    // would report more bytes in common than either payload has.
    let room = old.len().min(new.len()) - common_prefix;
    let common_suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(room)
        .take_while(|(a, b)| a == b)
        .count();
    ByteDiff {
        common_prefix,
        common_suffix,
        old_len: old.len(),
        new_len: new.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_changed_scalar_names_its_path_and_both_sides() {
        let d = diff(
            &json!({"value": 41.0, "unit": "percent"}),
            &json!({"value": 42.0, "unit": "percent"}),
            32,
        );
        assert_eq!(
            d.changes,
            [Change::Changed {
                path: "value".into(),
                old: json!(41.0),
                new: json!(42.0),
            }]
        );
        assert_eq!(d.truncated, 0);
    }

    #[test]
    fn identical_values_produce_nothing() {
        let d = diff(
            &json!({"a": [1, 2, {"b": true}]}),
            &json!({"a": [1, 2, {"b": true}]}),
            32,
        );
        assert!(d.is_empty(), "no change is not the same as not looked");
    }

    #[test]
    fn added_and_removed_fields_are_distinct_from_changed() {
        let d = diff(
            &json!({"a": 1, "gone": 2}),
            &json!({"a": 1, "fresh": 3}),
            32,
        );
        assert!(d.changes.contains(&Change::Removed {
            path: "gone".into(),
            old: json!(2)
        }));
        assert!(d.changes.contains(&Change::Added {
            path: "fresh".into(),
            new: json!(3)
        }));
        assert_eq!(d.changes.len(), 2);
    }

    #[test]
    fn nesting_produces_dotted_paths() {
        let d = diff(
            &json!({"disk": {"var-log": {"used": 1}}}),
            &json!({"disk": {"var-log": {"used": 2}}}),
            32,
        );
        assert_eq!(d.changes[0].path(), "disk.var-log.used");
    }

    /// Arrays are positional: index 1 moved, and nothing claims the elements
    /// were reordered.
    #[test]
    fn arrays_are_compared_by_index() {
        let d = diff(&json!({"xs": [1, 2, 3]}), &json!({"xs": [1, 9, 3]}), 32);
        assert_eq!(
            d.changes,
            [Change::Changed {
                path: "xs.1".into(),
                old: json!(2),
                new: json!(9),
            }]
        );
    }

    #[test]
    fn a_shorter_array_reports_the_tail_as_removed() {
        let d = diff(&json!([1, 2, 3]), &json!([1]), 32);
        assert_eq!(
            d.changes,
            [
                Change::Removed {
                    path: "1".into(),
                    old: json!(2)
                },
                Change::Removed {
                    path: "2".into(),
                    old: json!(3)
                },
            ]
        );
    }

    /// A type change is one change carrying both sides, not a remove plus an
    /// add — the field is still the same field.
    #[test]
    fn a_shape_change_is_a_single_change() {
        let d = diff(&json!({"a": {"b": 1}}), &json!({"a": [1]}), 32);
        assert_eq!(d.changes.len(), 1);
        assert_eq!(d.changes[0].path(), "a");
    }

    /// The bound is reported, never silently applied: a diff that stops at N
    /// without saying so reads as "and nothing else changed".
    #[test]
    fn changes_past_the_bound_are_counted_not_dropped_silently() {
        let old = json!({"a": 1, "b": 1, "c": 1, "d": 1, "e": 1});
        let new = json!({"a": 2, "b": 2, "c": 2, "d": 2, "e": 2});
        let d = diff(&old, &new, 2);
        assert_eq!(d.changes.len(), 2);
        assert_eq!(d.truncated, 3);
        assert!(!d.is_empty());
    }

    #[test]
    fn byte_diff_brackets_the_differing_run() {
        let d = byte_diff(b"hello world", b"hello there");
        assert_eq!(d.common_prefix, 6);
        assert_eq!(d.old_len, 11);
        assert_eq!(d.new_len, 11);
        let (old, new) = d.ranges();
        assert_eq!(&b"hello world"[old], b"world");
        assert_eq!(&b"hello there"[new], b"there");
        assert!(!d.is_empty());
    }

    #[test]
    fn identical_bytes_are_empty() {
        let d = byte_diff(b"same", b"same");
        assert!(d.is_empty());
        assert_eq!(d.common_prefix, 4);
    }

    /// A repeated byte must not let prefix and suffix double-count the same
    /// bytes and claim more in common than the payload has.
    #[test]
    fn prefix_and_suffix_never_overlap() {
        let d = byte_diff(b"aaaa", b"aaaaaa");
        assert_eq!(d.common_prefix, 4);
        assert_eq!(d.common_suffix, 0);
        assert!(d.common_prefix + d.common_suffix <= d.old_len);
    }
}
