//! Two snapshots compared (RFC 13 §4.4; #219): by key, facet by facet,
//! bounded, with both spans carried through untouched.
//!
//! Pure: two [`Snapshot`]s in hand become one [`SnapshotDiff`]. The value
//! facet goes through the same two-level comparison the History pane runs
//! ([`crate::model::diff`]) — structural where both sides have a structural
//! form, bytes otherwise, and which one ran is never hidden. The other
//! facets (verdict, registration, holder) are compared whole and reported
//! only where they differ, so a consumer that reads a `KeyChange` reads
//! exactly the facets that moved and nothing that did not.
//!
//! Origin alignment across deployments (`origin_map`, `unmapped`,
//! `by_subject`) is chunk DD's; this comparison keys on the wire key
//! verbatim and leaves those fields *not asked*.

use std::collections::BTreeMap;

use crate::model::diff::{byte_diff, diff as value_diff};
use crate::report::{Asked, KeyChange, Snapshot, SnapshotDiff, SnapshotRow};

/// The two bounds a diff runs under (RFC 09 §5.1 O6 — a bound that hides
/// data says so).
#[derive(Debug, Clone, Copy)]
pub struct DiffOpts {
    /// Field-level changes listed per key before the rest are counted
    /// (`ValueDiff::truncated`).
    pub max_changes: usize,
    /// Differing keys listed — added, removed and changed together — before
    /// the rest are counted (`SnapshotDiff::truncated`).
    pub max_keys: usize,
}

impl Default for DiffOpts {
    fn default() -> Self {
        DiffOpts {
            max_changes: 20,
            max_keys: crate::model::bounded::DEFAULT_MAX_KEYS,
        }
    }
}

/// Compare `a` against `b`, key by key.
pub fn diff_snapshots(a: &Snapshot, b: &Snapshot, opts: DiffOpts) -> SnapshotDiff {
    fn by_key(s: &Snapshot) -> BTreeMap<&str, &SnapshotRow> {
        s.rows.iter().map(|r| (r.key.as_str(), r)).collect()
    }
    let (ra, rb) = (by_key(a), by_key(b));

    let mut out = SnapshotDiff {
        a: a.header.clone(),
        b: b.header.clone(),
        added: Vec::new(),
        removed: Vec::new(),
        changed: Vec::new(),
        unchanged: 0,
        truncated: 0,
        origin_map: Asked::NotAsked,
        unmapped: Vec::new(),
        by_subject: Asked::NotAsked,
    };
    let mut listed = 0usize;
    // One budget across the three lists: a bound per list would let the
    // total quietly triple.
    let mut admit = |out: &mut SnapshotDiff| -> bool {
        if listed < opts.max_keys {
            listed += 1;
            true
        } else {
            out.truncated += 1;
            false
        }
    };

    for (key, row_a) in &ra {
        match rb.get(key) {
            None => {
                if admit(&mut out) {
                    out.removed.push((*key).to_string());
                }
            }
            Some(row_b) => match key_change(row_a, row_b, opts.max_changes) {
                None => out.unchanged += 1,
                Some(change) => {
                    if admit(&mut out) {
                        out.changed.push(change);
                    }
                }
            },
        }
    }
    for key in rb.keys() {
        if !ra.contains_key(key) && admit(&mut out) {
            out.added.push((*key).to_string());
        }
    }
    out
}

/// The structural form of a row's payload, when it has one.
///
/// Under `decode` this is the same sniff the explorers render with
/// ([`crate::structural_value`] — JSON, then CBOR, then text); without it a
/// plain JSON parse, so a library consumer that only diffs files still gets
/// field-level changes on the common case.
fn structural(row: &SnapshotRow) -> Option<serde_json::Value> {
    let bytes = payload(row)?;
    #[cfg(feature = "decode")]
    {
        crate::model::decode::structural_value(&bytes)
    }
    #[cfg(not(feature = "decode"))]
    {
        serde_json::from_slice(&bytes).ok()
    }
}

fn payload(row: &SnapshotRow) -> Option<Vec<u8>> {
    row.payload()
}

/// What moved between two rows of one key — `None` when nothing did.
fn key_change(a: &SnapshotRow, b: &SnapshotRow, max_changes: usize) -> Option<KeyChange> {
    let mut change = KeyChange {
        key: a.key.clone(),
        value: None,
        bytes: None,
        verdict: None,
        registration: None,
        holder: None,
        timestamp: (a.timestamp.clone(), b.timestamp.clone()),
    };
    let mut moved = false;

    // The value facet. A tombstone against a value (or vice versa) is a
    // change of kind, reported as a byte diff of nothing against something
    // rather than as field changes that never existed.
    if a.delete != b.delete || a.bytes != b.bytes {
        moved = true;
        match (structural(a), structural(b)) {
            (Some(va), Some(vb)) => {
                let d = value_diff(&va, &vb, max_changes);
                if d.is_empty() {
                    // Byte-different, structurally identical (whitespace, key
                    // order): the byte view is the only one that can show it.
                    change.bytes = Some(byte_diff(
                        &payload(a).unwrap_or_default(),
                        &payload(b).unwrap_or_default(),
                    ));
                } else {
                    change.value = Some(d);
                }
            }
            _ => {
                change.bytes = Some(byte_diff(
                    &payload(a).unwrap_or_default(),
                    &payload(b).unwrap_or_default(),
                ));
            }
        }
    }
    if a.verdict != b.verdict {
        moved = true;
        change.verdict = Some((a.verdict.clone(), b.verdict.clone()));
    }
    if a.registration != b.registration {
        moved = true;
        change.registration = Some((a.registration, b.registration));
    }
    if a.holder != b.holder {
        moved = true;
        change.holder = Some((a.holder.clone(), b.holder.clone()));
    }
    // A stamp that moved with nothing else moving is a re-publish of the
    // same value: a fact worth a row, because "the value did not change"
    // and "nobody published" are different claims about a fleet.
    if a.timestamp != b.timestamp {
        moved = true;
    }
    moved.then_some(change)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{AnsweredBy, Holder, RegistrationWire, VerdictWire, ZsnapHeader};

    fn header() -> ZsnapHeader {
        ZsnapHeader {
            zsnap: 1,
            selectors: vec!["v1/**".into()],
            base: String::new(),
            collected_at: "2026-09-06T00:00:00Z".into(),
            collection_span_s: 0.5,
            asked: 1,
            answered: 0,
            elided: 0,
            errors: 0,
            superseded: 0,
            roster: Asked::NotAsked,
        }
    }

    fn row(key: &str, body: &[u8]) -> SnapshotRow {
        use base64::Engine as _;
        SnapshotRow {
            key: key.into(),
            delete: false,
            bytes: Some(base64::engine::general_purpose::STANDARD.encode(body)),
            encoding: Some("application/json".into()),
            timestamp: None,
            stamper: None,
            source: None,
            source_zid: None,
            registration: RegistrationWire::RegistryNotLoaded,
            verdict: VerdictWire::NotValidated {
                reason: "no_registry".into(),
            },
            holder: Holder::Unattributed {
                reason: "roster not asked".into(),
            },
        }
    }

    fn snap(rows: Vec<SnapshotRow>) -> Snapshot {
        Snapshot {
            header: header(),
            rows,
        }
    }

    #[test]
    fn a_self_diff_is_empty() {
        let s = snap(vec![
            row("v1/h-aaaaaaaaaaaa/state/p/a", b"{\"x\":1}"),
            row("v1/h-aaaaaaaaaaaa/state/p/b", b"text"),
        ]);
        let d = diff_snapshots(&s, &s, DiffOpts::default());
        assert!(!d.differs());
        assert_eq!(d.unchanged, 2);
        assert!(d.changed.is_empty() && d.added.is_empty() && d.removed.is_empty());
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
            origin: "h-aaaaaaaaaaaa".into(),
            answered_by: AnsweredBy::Stamper,
        };
        let a = snap(vec![row("k", b"{}")]);
        let b = snap(vec![live]);
        let d = diff_snapshots(&a, &b, DiffOpts::default());
        let c = &d.changed[0];
        assert!(c.holder.is_some());
        assert!(c.value.is_none() && c.bytes.is_none());
        assert!(c.verdict.is_none() && c.registration.is_none());
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
                max_changes: 20,
                max_keys: 3,
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

    /// A delete on one side is a change of kind, reported through the byte
    /// view rather than as invented field changes.
    #[test]
    fn a_tombstone_against_a_value_is_a_byte_change() {
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
