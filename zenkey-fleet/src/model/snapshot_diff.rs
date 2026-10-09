//! Two snapshots compared (RFC 13 §4.4; #219; zk2's since #612, FJ8b): by
//! zk2 key, facet by facet, bounded, with both spans carried through
//! untouched.
//!
//! Pure: two [`Snapshot`]s in hand become one [`SnapshotDiff`]. A row's key
//! is read relative to its own file's namespace — `prod/zk2/host-a/tc/…`
//! and `staging/zk2/host-a/tc/…` are one key — so two deployments of the
//! same services line up on the keys those services publish, without an
//! alignment step: zk2's addresses are the identity v1's re-minted origins
//! never were (spec §1.5). A key outside its file's namespace keeps its wire
//! spelling, and only lines up with the same wire key on the other side.
//!
//! The value facet goes through the same two-level comparison the History
//! pane runs ([`crate::model::diff`]) — structural where both sides have a
//! structural form, bytes otherwise, and which one ran is never hidden. The
//! other facets (conformance, holder) are compared whole and reported only
//! where they differ.

use std::collections::BTreeMap;

use crate::model::diff::{byte_diff, diff as value_diff};
use crate::report::{KeyChange, Snapshot, SnapshotDiff, SnapshotRow, ZsnapHeader};

/// The two bounds a diff runs under (O6 — a bound that hides data says so).
#[derive(Debug, Clone, Copy)]
pub struct DiffOpts {
    /// Field-level changes listed per key before the rest are counted
    /// (`ValueDiff::truncated`).
    pub max_changes: usize,
    /// Differing keys listed — added, removed and changed together — before
    /// the rest are counted (`SnapshotDiff::truncated`).
    pub max_keys: usize,
    /// Whether a stamp that moved with nothing else moving is a change.
    /// True for two moments of one deployment — a re-put of the same value
    /// is a fact worth a row; false for two deployments, whose clocks never
    /// agreed to begin with, which [`diff_snapshots`] decides by the two
    /// namespaces.
    pub stamps_alone: bool,
}

impl Default for DiffOpts {
    fn default() -> Self {
        DiffOpts {
            max_changes: 20,
            max_keys: crate::model::bounded::DEFAULT_MAX_KEYS,
            stamps_alone: true,
        }
    }
}

/// What one key did between the two sides, before any bound is applied.
enum Outcome {
    Added,
    Removed,
    Changed(Box<KeyChange>),
    Unchanged,
}

/// Compare `a` against `b`, by zk2 key. Two files from two namespaces are
/// two deployments: a stamp that moved alone is not a change between them.
pub fn diff_snapshots(a: &Snapshot, b: &Snapshot, opts: DiffOpts) -> SnapshotDiff {
    let opts = DiffOpts {
        stamps_alone: opts.stamps_alone && a.header.base == b.header.base,
        ..opts
    };
    bound(&a.header, &b.header, compare(a, b, opts), opts.max_keys)
}

/// A row's key relative to its file's namespace, or its wire key when it
/// does not sit under it.
fn zk2_key<'r>(base: &str, row: &'r SnapshotRow) -> &'r str {
    zenkey::grammar::strip_base(base, &row.key).unwrap_or(&row.key)
}

/// Every key on either side, in `a`'s key order then `b`'s additions, with
/// what it did.
fn compare(a: &Snapshot, b: &Snapshot, opts: DiffOpts) -> Vec<(String, Outcome)> {
    fn by_key(s: &Snapshot) -> BTreeMap<&str, &SnapshotRow> {
        s.rows
            .iter()
            .map(|r| (zk2_key(&s.header.base, r), r))
            .collect()
    }
    let (ra, rb) = (by_key(a), by_key(b));
    let mut out = Vec::with_capacity(ra.len() + rb.len());
    for (key, row_a) in &ra {
        let outcome = match rb.get(key) {
            None => Outcome::Removed,
            Some(row_b) => match key_change(key, row_a, row_b, opts) {
                None => Outcome::Unchanged,
                Some(change) => Outcome::Changed(Box::new(change)),
            },
        };
        out.push(((*key).to_string(), outcome));
    }
    for key in rb.keys() {
        if !ra.contains_key(key) {
            out.push(((*key).to_string(), Outcome::Added));
        }
    }
    out
}

/// The listing, bounded: one budget across the three lists, because a
/// bound per list would let the total quietly triple; past it the
/// differing keys are counted, never dropped (O6).
fn bound(
    a: &ZsnapHeader,
    b: &ZsnapHeader,
    outcomes: Vec<(String, Outcome)>,
    max_keys: usize,
) -> SnapshotDiff {
    let mut out = SnapshotDiff {
        a: a.clone(),
        b: b.clone(),
        added: Vec::new(),
        removed: Vec::new(),
        changed: Vec::new(),
        unchanged: 0,
        truncated: 0,
    };
    let mut listed = 0usize;
    for (key, outcome) in outcomes {
        if matches!(outcome, Outcome::Unchanged) {
            out.unchanged += 1;
            continue;
        }
        if listed >= max_keys {
            out.truncated += 1;
            continue;
        }
        listed += 1;
        match outcome {
            Outcome::Added => out.added.push(key),
            Outcome::Removed => out.removed.push(key),
            Outcome::Changed(c) => out.changed.push(*c),
            Outcome::Unchanged => unreachable!("counted above"),
        }
    }
    out
}

/// The structural form of a row's payload, when it has one: the sniff the
/// explorers render with (JSON, then CBOR, then text).
pub(crate) fn structural_of(row: &SnapshotRow) -> Option<serde_json::Value> {
    crate::model::structural::structural_value(&row.payload()?)
}

/// What moved between two rows of one key — `None` when nothing did.
fn key_change(key: &str, a: &SnapshotRow, b: &SnapshotRow, opts: DiffOpts) -> Option<KeyChange> {
    let mut change = KeyChange {
        key: key.to_owned(),
        value: None,
        bytes: None,
        conformance: None,
        holder: None,
        timestamp: (a.timestamp.clone(), b.timestamp.clone()),
    };
    let mut moved = false;

    // The value facet. A deletion against a value (or vice versa) is a
    // change of kind, reported as a byte diff of nothing against something
    // rather than as field changes that never existed.
    if a.delete != b.delete || a.bytes != b.bytes {
        moved = true;
        match (structural_of(a), structural_of(b)) {
            (Some(va), Some(vb)) => {
                let d = value_diff(&va, &vb, opts.max_changes);
                if d.is_empty() {
                    // Byte-different, structurally identical (whitespace, key
                    // order): the byte view is the only one that can show it.
                    change.bytes = Some(byte_diff(
                        &a.payload().unwrap_or_default(),
                        &b.payload().unwrap_or_default(),
                    ));
                } else {
                    change.value = Some(d);
                }
            }
            _ => {
                change.bytes = Some(byte_diff(
                    &a.payload().unwrap_or_default(),
                    &b.payload().unwrap_or_default(),
                ));
            }
        }
    }
    if a.conformance != b.conformance {
        moved = true;
        change.conformance = Some((a.conformance.clone(), b.conformance.clone()));
    }
    // Two deployments' holders name two deployments' sessions: compared by
    // kind and address, which is what a holder means across them.
    if a.holder != b.holder {
        moved = true;
        change.holder = Some((a.holder.clone(), b.holder.clone()));
    }
    // A stamp that moved with nothing else moving is a re-put of the same
    // value: a fact worth a row, because "the value did not change" and
    // "nobody put it" are different claims — unless the two sides are two
    // deployments (`stamps_alone`).
    if opts.stamps_alone && a.timestamp != b.timestamp {
        moved = true;
    }
    moved.then_some(change)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{AnsweredBy, Conformance, Holder, KeyGroup, KeyIdentity};

    fn header(base: &str) -> ZsnapHeader {
        ZsnapHeader {
            zsnap: 2,
            selectors: vec![zenkey::grammar::with_base(base, "zk2/*/*/*/state/**")],
            base: base.into(),
            collected_at: "2026-10-09T00:00:00Z".into(),
            collection_span_s: 0.5,
            asked: 1,
            answered: 0,
            elided: 0,
            errors: 0,
            discarded: 0,
            superseded: 0,
            presence: None,
        }
    }

    fn row(key: &str, body: &[u8]) -> SnapshotRow {
        use base64::Engine as _;
        SnapshotRow {
            key: key.into(),
            identity: KeyIdentity {
                group: KeyGroup::NotZk2,
                values: Default::default(),
                unresolved: None,
            },
            delete: false,
            bytes: Some(base64::engine::general_purpose::STANDARD.encode(body)),
            encoding: Some("application/json".into()),
            timestamp: None,
            stamper: None,
            source_zid: None,
            conformance: Conformance::NotChecked {
                reason: "presence was not read".into(),
            },
            holder: Holder::Unattributed {
                reason: "presence was not read".into(),
            },
        }
    }

    fn snap(rows: Vec<SnapshotRow>) -> Snapshot {
        snap_in("", rows)
    }

    fn snap_in(base: &str, rows: Vec<SnapshotRow>) -> Snapshot {
        Snapshot {
            header: header(base),
            rows,
        }
    }

    #[test]
    fn a_self_diff_is_empty() {
        let s = snap(vec![
            row("zk2/lab/m/m.v1/state/a", b"{\"x\":1}"),
            row("zk2/lab/m/m.v1/state/b", b"text"),
        ]);
        let d = diff_snapshots(&s, &s, DiffOpts::default());
        assert!(!d.differs());
        assert_eq!(d.unchanged, 2);
        assert!(d.changed.is_empty() && d.added.is_empty() && d.removed.is_empty());
    }

    /// The zk2 key is the identity: one deployment in `prod`, the same in
    /// `staging`, line up key by key, and their clocks — two deployments,
    /// two clocks — are not a change.
    #[test]
    fn two_namespaces_line_up_on_their_zk2_keys() {
        let mut a_row = row("prod/zk2/lab/m/m.v1/state/a", b"{\"x\":1}");
        a_row.timestamp = Some("1/aa".into());
        let mut b_row = row("staging/zk2/lab/m/m.v1/state/a", b"{\"x\":1}");
        b_row.timestamp = Some("2/bb".into());
        let mut b_moved = row("staging/zk2/lab/m/m.v1/state/b", b"{\"x\":3}");
        b_moved.timestamp = Some("3/bb".into());
        let a = snap_in(
            "prod",
            vec![a_row, row("prod/zk2/lab/m/m.v1/state/b", b"{\"x\":2}")],
        );
        let b = snap_in("staging", vec![b_row, b_moved]);
        let d = diff_snapshots(&a, &b, DiffOpts::default());
        assert_eq!(d.unchanged, 1, "{d:?}");
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.changed[0].key, "zk2/lab/m/m.v1/state/b");
        assert!(d.added.is_empty() && d.removed.is_empty());
    }

    /// Structural where both sides parse; bytes where one does not — and
    /// which ran is visible in which field is present.
    #[test]
    fn a_non_json_payload_falls_back_to_bytes() {
        let a = snap(vec![
            row("k/json", b"{\"x\":1}"),
            row("k/text", b"hello world"),
        ]);
        let b = snap(vec![
            row("k/json", b"{\"x\":2}"),
            row("k/text", b"hello there"),
        ]);
        let d = diff_snapshots(&a, &b, DiffOpts::default());
        assert_eq!(d.changed.len(), 2);
        let json = d.changed.iter().find(|c| c.key == "k/json").unwrap();
        assert!(json.value.is_some() && json.bytes.is_none());
        assert_eq!(json.value.as_ref().unwrap().changes[0].path(), "x");
        let text = d.changed.iter().find(|c| c.key == "k/text").unwrap();
        assert!(text.bytes.is_some() && text.value.is_none());
        assert_eq!(text.bytes.unwrap().common_prefix, 6);
    }

    /// The facets stay apart: a holder that moved with the value unchanged
    /// is a holder change and nothing else.
    #[test]
    fn a_facet_pair_rides_only_when_that_facet_moved() {
        let mut live = row("k", b"{}");
        live.holder = Holder::Live {
            address: "lab/m".into(),
            answered_by: AnsweredBy::Owner,
        };
        let a = snap(vec![row("k", b"{}")]);
        let b = snap(vec![live]);
        let d = diff_snapshots(&a, &b, DiffOpts::default());
        let c = &d.changed[0];
        assert!(c.holder.is_some());
        assert!(c.value.is_none() && c.bytes.is_none());
        assert!(c.conformance.is_none());
    }

    #[test]
    fn added_and_removed_keys_are_listed_by_side() {
        let a = snap(vec![row("only/a", b"1"), row("both", b"1")]);
        let b = snap(vec![row("only/b", b"1"), row("both", b"1")]);
        let d = diff_snapshots(&a, &b, DiffOpts::default());
        assert_eq!(d.added, ["only/b"]);
        assert_eq!(d.removed, ["only/a"]);
        assert_eq!(d.unchanged, 1);
        assert!(d.differs());
    }

    /// Past `max_keys` the differing keys are counted, never dropped (O6),
    /// and the count is across all three lists.
    #[test]
    fn differing_keys_past_the_bound_are_counted() {
        let a = snap((0..5).map(|i| row(&format!("k/{i}"), b"1")).collect());
        let b = snap((3..8).map(|i| row(&format!("k/{i}"), b"2")).collect());
        let d = diff_snapshots(
            &a,
            &b,
            DiffOpts {
                max_keys: 3,
                ..DiffOpts::default()
            },
        );
        let listed = d.added.len() + d.removed.len() + d.changed.len();
        assert_eq!(listed, 3);
        assert_eq!(
            d.truncated, 5,
            "3 removed + 2 changed + 3 added = 8, 3 listed"
        );
        assert!(d.differs());
    }

    /// A deletion on one side is a change of kind, reported through the
    /// byte view rather than as invented field changes.
    #[test]
    fn a_deletion_against_a_value_is_a_byte_change() {
        let mut gone = row("k", b"{}");
        gone.delete = true;
        gone.bytes = None;
        let d = diff_snapshots(
            &snap(vec![row("k", b"{}")]),
            &snap(vec![gone]),
            DiffOpts::default(),
        );
        let c = &d.changed[0];
        assert!(c.bytes.is_some());
        assert_eq!(c.bytes.unwrap().new_len, 0);
    }
}
