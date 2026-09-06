//! What a payload comparison says (issue #63; on the wire since #219).
//!
//! The *algorithms* — [`diff`](crate::model::diff::diff) and
//! [`byte_diff`](crate::model::diff::byte_diff) — stay in
//! [`crate::model::diff`], because they compute from values in hand. The
//! *shapes* live here because a `.zsnap` diff (`zenctl snapshot diff
//! --format json`) carries them to a script: the placement rule in
//! [`crate::report`] has no exceptions, and a `Change` that reaches a pipe
//! is a contract however small it is.
//!
//! Deliberately no notion of `Put` vs `Delete`: a tombstone is not a value and
//! diffing it against one would be a category error. `SampleView::kind` is
//! exact, and the frontend words the retirement.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One field-level difference, addressed by a dotted path (`disk.used`,
/// `items.0.name`).
///
/// Tagged `op` on the wire — `added | removed | changed` — so a consumer
/// reads the kind before the fields, the way every other tagged row in this
/// crate is read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValueDiff {
    pub changes: Vec<Change>,
    /// Changes found past `max_changes` and therefore not listed.
    ///
    /// Counted rather than silently cut: a bounded view that reports what it
    /// dropped is the RFC 09 §5.1 O6 rule, and a diff that quietly stops at
    /// twenty entries reads as "and nothing else changed". Always written,
    /// even at zero — a diff's bound is part of what the diff *is*.
    pub truncated: usize,
}

impl ValueDiff {
    /// No change at all — distinct from "we did not look".
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.truncated == 0
    }
}

/// What a byte comparison can honestly say when neither side is structural.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The wire spelling of a change, pinned: `op` leads, snake_case, and
    /// the two sides ride under `old`/`new` exactly as the in-memory enum
    /// names them.
    #[test]
    fn a_change_is_tagged_by_op() {
        let d = ValueDiff {
            changes: vec![
                Change::Changed {
                    path: "value".into(),
                    old: json!(41),
                    new: json!(42),
                },
                Change::Added {
                    path: "fresh".into(),
                    new: json!(true),
                },
                Change::Removed {
                    path: "gone".into(),
                    old: json!(null),
                },
            ],
            truncated: 0,
        };
        assert_eq!(
            serde_json::to_value(&d).unwrap(),
            json!({
                "changes": [
                    {"op": "changed", "path": "value", "old": 41, "new": 42},
                    {"op": "added", "path": "fresh", "new": true},
                    {"op": "removed", "path": "gone", "old": null},
                ],
                "truncated": 0,
            })
        );
        let back: ValueDiff = serde_json::from_value(serde_json::to_value(&d).unwrap()).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn a_byte_diff_round_trips() {
        let d = ByteDiff {
            common_prefix: 6,
            common_suffix: 0,
            old_len: 11,
            new_len: 11,
        };
        assert_eq!(
            serde_json::to_value(d).unwrap(),
            json!({"common_prefix": 6, "common_suffix": 0, "old_len": 11, "new_len": 11})
        );
    }
}
