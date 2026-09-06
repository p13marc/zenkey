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
//! Two entry points, one comparison. [`diff_snapshots`] keys on the wire
//! key verbatim and leaves the alignment fields *not asked*.
//! [`diff_normalized`] (#220) takes a [`MapPlan`] — which host in `b` is
//! which host in `a` — rewrites `b` onto `a`'s origins and base, runs the
//! same comparison, and rolls it up per subject, so a fleet-wide drift
//! reads as one line ("`sysinfo/cpu/usage` differs on 3 of 12 origins")
//! rather than N. It **refuses over an incomplete plan**: an origin the
//! plan could not pair is listed, never dropped (RFC 13 §4.4), and no
//! comparison is made over it, because a diff that quietly compared the
//! rest would let "it works in staging" survive on the keys it skipped.

use std::collections::BTreeMap;

use crate::model::diff::{byte_diff, diff as value_diff};
use crate::model::facts::{KeyFacts, KeyShape, OriginKind};
use crate::model::origin_map::MapPlan;
use crate::report::{
    Asked, Holder, KeyChange, Snapshot, SnapshotDiff, SnapshotRow, SubjectDelta, ZsnapHeader,
};

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
    /// Whether a stamp that moved with nothing else moving is a change.
    /// True for two moments of one fleet — a re-publish of the same value
    /// is a fact worth a row. [`diff_normalized`] turns it off: two
    /// deployments never share a clock, so a stamp that differs alone says
    /// nothing about the fleets.
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

/// Compare `a` against `b`, key by key.
pub fn diff_snapshots(a: &Snapshot, b: &Snapshot, opts: DiffOpts) -> SnapshotDiff {
    bound(&a.header, &b.header, compare(a, b, opts), opts.max_keys)
}

/// Compare `a` against `b` with `b`'s origins read through `plan`, and
/// roll the result up per subject.
///
/// `b`'s rows are rewritten before the comparison — the origin chunk by
/// the plan's pairs, the base onto `a`'s stated base, the holder's origin
/// with the key's, and the identity bridge's `host_id` (RFC 06 §6.2) with
/// the origin it restates — and then compared exactly as [`diff_snapshots`]
/// would. A stamp that moved alone is not a change here
/// ([`DiffOpts::stamps_alone`]).
///
/// Over a plan with anything [`unmapped`](MapPlan::unmapped) the
/// comparison is **not made**: the report carries the pairs it had, every
/// unpaired origin with its reason, and `by_subject` not asked —
/// [`SnapshotDiff::refused`], the reserved non-verdict.
pub fn diff_normalized(a: &Snapshot, b: &Snapshot, plan: &MapPlan, opts: DiffOpts) -> SnapshotDiff {
    let mut out = SnapshotDiff {
        a: a.header.clone(),
        b: b.header.clone(),
        added: Vec::new(),
        removed: Vec::new(),
        changed: Vec::new(),
        unchanged: 0,
        truncated: 0,
        origin_map: Asked::Asked(plan.pairs.clone()),
        unmapped: plan.unmapped.clone(),
        by_subject: Asked::NotAsked,
    };
    if !plan.is_complete() {
        return out;
    }
    let opts = DiffOpts {
        stamps_alone: false,
        ..opts
    };
    let b_to_a = plan.b_to_a();
    let a_norm = Snapshot {
        header: a.header.clone(),
        rows: a
            .rows
            .iter()
            .map(|r| canonical_bridge(r, &a.header.base, None))
            .collect(),
    };
    let b_norm = Snapshot {
        header: b.header.clone(),
        rows: b
            .rows
            .iter()
            .map(|r| rewrite_row(r, &b.header.base, &a.header.base, &b_to_a))
            .collect(),
    };
    let outcomes = compare(&a_norm, &b_norm, opts);

    // The roll-up, over every outcome — the bound below applies to the
    // listing, not to the counts (O6).
    let mut subjects: BTreeMap<String, SubjectDelta> = BTreeMap::new();
    for (key, outcome) in &outcomes {
        let subject = subject_of(&a.header.base, key);
        let s = subjects
            .entry(subject.clone())
            .or_insert_with(|| SubjectDelta {
                subject,
                compared: 0,
                differing: 0,
                only_in_a: 0,
                only_in_b: 0,
                example: None,
            });
        match outcome {
            Outcome::Added => s.only_in_b += 1,
            Outcome::Removed => s.only_in_a += 1,
            Outcome::Unchanged => s.compared += 1,
            Outcome::Changed(c) => {
                s.compared += 1;
                s.differing += 1;
                if s.example.is_none() {
                    s.example = Some((**c).clone());
                }
            }
        }
    }
    let bounded = bound(&a.header, &b.header, outcomes, opts.max_keys);
    out.added = bounded.added;
    out.removed = bounded.removed;
    out.changed = bounded.changed;
    out.unchanged = bounded.unchanged;
    out.truncated = bounded.truncated;
    out.by_subject = Asked::Asked(subjects.into_values().collect());
    out
}

/// The subject a key rolls up under: for a host key, everything after the
/// origin (`state/sysinfo/health`); for a service origin, the origin stays
/// (`@catalog/state/entity/x` — two services with one subject tail are two
/// subjects); for a key that is not this convention's, the key verbatim.
fn subject_of(base: &str, key: &str) -> String {
    let facts = KeyFacts::project(base, key);
    let (KeyShape::V1(f), Some(relative)) = (&facts.shape, zenkey::grammar::strip_base(base, key))
    else {
        return key.to_string();
    };
    let mut chunks = relative.split('/');
    chunks.next(); // v1
    if f.origin_kind == OriginKind::Host {
        chunks.next(); // the origin
    }
    chunks.collect::<Vec<_>>().join("/")
}

/// `b`'s row read through the plan: the key's origin chunk renamed and the
/// key re-based, the holder's origin renamed with it, and the bridge
/// document's `host_id` renamed too.
fn rewrite_row(
    row: &SnapshotRow,
    from_base: &str,
    to_base: &str,
    b_to_a: &BTreeMap<&str, &str>,
) -> SnapshotRow {
    let facts = KeyFacts::project(from_base, &row.key);
    let (KeyShape::V1(f), Some(relative)) = (
        &facts.shape,
        zenkey::grammar::strip_base(from_base, &row.key),
    ) else {
        // Not under `b`'s base, or not a v1 key: nothing to rename, and
        // re-basing a key that names no base would invent one.
        return row.clone();
    };
    let mapped = if f.origin_kind == OriginKind::Host {
        b_to_a.get(f.origin.as_str()).copied()
    } else {
        None
    };
    let mut chunks: Vec<&str> = relative.split('/').collect();
    if let Some(to) = mapped
        && chunks.len() > 1
    {
        chunks[1] = to;
    }
    let mut out = row.clone();
    out.key = zenkey::grammar::with_base(to_base, chunks.join("/"));
    if let Some(to) = mapped {
        out.holder = match &row.holder {
            Holder::Live {
                origin,
                answered_by,
            } if origin == &f.origin => Holder::Live {
                origin: to.to_string(),
                answered_by: *answered_by,
            },
            Holder::StorageOnly { origin } if origin == &f.origin => Holder::StorageOnly {
                origin: to.to_string(),
            },
            other => other.clone(),
        };
    }
    canonical_bridge(&out, to_base, mapped.map(|to| (f.origin.as_str(), to)))
}

/// An identity-bridge document (RFC 06 §6.2) with its `host_id` read
/// through the rename, re-serialised canonically — on **both** sides, so
/// the two byte forms agree exactly when the two documents do. Only a
/// document whose `host_id` is the origin it sits under is touched (a
/// `host_id` that names some other host is a fact the diff must show), and
/// only under a normalised diff, where the origin is what is being mapped.
fn canonical_bridge(row: &SnapshotRow, base: &str, rename: Option<(&str, &str)>) -> SnapshotRow {
    use base64::Engine as _;
    let facts = KeyFacts::project(base, &row.key);
    let KeyShape::V1(f) = &facts.shape else {
        return row.clone();
    };
    let is_bridge = f.origin_kind == OriginKind::Host
        && f.class == "state"
        && matches!(f.subject.as_slice(), [s] if s == "health" || s == "sensor");
    if !is_bridge || row.delete {
        return row.clone();
    }
    let Some(serde_json::Value::Object(mut doc)) = structural_of(row) else {
        return row.clone();
    };
    let own = rename.map(|(from, _)| from).unwrap_or(f.origin.as_str());
    if doc.get("host_id").and_then(|v| v.as_str()) != Some(own) {
        return row.clone();
    }
    if let Some((_, to)) = rename {
        doc.insert("host_id".into(), serde_json::Value::String(to.to_string()));
    }
    let mut out = row.clone();
    let bytes = serde_json::to_vec(&serde_json::Value::Object(doc)).unwrap_or_default();
    out.bytes = Some(base64::engine::general_purpose::STANDARD.encode(bytes));
    out
}

/// Every key on either side, in `a`'s key order then `b`'s additions, with
/// what it did.
fn compare(a: &Snapshot, b: &Snapshot, opts: DiffOpts) -> Vec<(String, Outcome)> {
    fn by_key(s: &Snapshot) -> BTreeMap<&str, &SnapshotRow> {
        s.rows.iter().map(|r| (r.key.as_str(), r)).collect()
    }
    let (ra, rb) = (by_key(a), by_key(b));
    let mut out = Vec::with_capacity(ra.len() + rb.len());
    for (key, row_a) in &ra {
        let outcome = match rb.get(key) {
            None => Outcome::Removed,
            Some(row_b) => match key_change(row_a, row_b, opts) {
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
        origin_map: Asked::NotAsked,
        unmapped: Vec::new(),
        by_subject: Asked::NotAsked,
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

/// The structural form of a row's payload, when it has one.
///
/// Under `decode` this is the same sniff the explorers render with
/// ([`crate::structural_value`] — JSON, then CBOR, then text); without it a
/// plain JSON parse, so a library consumer that only diffs files still gets
/// field-level changes on the common case.
pub(crate) fn structural_of(row: &SnapshotRow) -> Option<serde_json::Value> {
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
fn key_change(a: &SnapshotRow, b: &SnapshotRow, opts: DiffOpts) -> Option<KeyChange> {
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
        match (structural_of(a), structural_of(b)) {
            (Some(va), Some(vb)) => {
                let d = value_diff(&va, &vb, opts.max_changes);
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
    // and "nobody published" are different claims about a fleet — unless
    // the two sides are two fleets, whose clocks never agreed to begin
    // with (`stamps_alone`).
    if opts.stamps_alone && a.timestamp != b.timestamp {
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

    // ─── normalised (#220) ───────────────────────────────────────────────

    use crate::model::origin_map::tests::{host, snap as fleet};
    use crate::model::origin_map::{origin_profiles, plan_map};
    use crate::report::MapEvidence;

    const A1: &str = "h-aaaaaaaaaaa1";
    const A2: &str = "h-aaaaaaaaaaa2";
    const B1: &str = "h-bbbbbbbbbbb1";
    const B2: &str = "h-bbbbbbbbbbb2";

    fn renamed(base_b: &str) -> (Snapshot, Snapshot) {
        let a = fleet(
            "prod",
            [
                host("prod", A1, "web", &[]),
                host("prod", A2, "db", &["logs"]),
            ]
            .concat(),
        );
        let mut b = fleet(
            base_b,
            [
                host(base_b, B1, "web", &[]),
                host(base_b, B2, "db", &["logs"]),
            ]
            .concat(),
        );
        // Two deployments, two clocks: every stamp differs.
        for r in &mut b.rows {
            r.timestamp = Some("7f3b2a1c00000009/ef56".into());
        }
        (a, b)
    }

    fn plan(a: &Snapshot, b: &Snapshot) -> MapPlan {
        plan_map(&origin_profiles(a), &origin_profiles(b), &[]).unwrap()
    }

    /// The acceptance case (#220): the same fleet with every origin
    /// re-minted — different base, different stamps, each health document
    /// naming its own `host_id` — diffs to zero once aligned, and every
    /// subject rolls up with nothing differing.
    #[test]
    fn a_renamed_fleet_diffs_to_zero_once_aligned() {
        let (a, b) = renamed("stg");
        let plain = diff_snapshots(&a, &b, DiffOpts::default());
        assert!(plain.differs(), "verbatim, nothing lines up");
        assert!(plain.origin_map.is_not_asked() && plain.by_subject.is_not_asked());

        let d = diff_normalized(&a, &b, &plan(&a, &b), DiffOpts::default());
        assert!(!d.differs(), "{d:?}");
        assert_eq!(d.unchanged, 5);
        assert!(d.unmapped.is_empty());
        let pairs = d.origin_map.as_option().unwrap();
        assert_eq!(pairs.len(), 2);
        assert!(matches!(pairs[0].evidence, MapEvidence::Label { .. }));
        let subjects = d.by_subject.as_option().unwrap();
        assert!(
            subjects
                .iter()
                .all(|s| s.differing == 0 && s.only_in_a == 0 && s.only_in_b == 0)
        );
        assert_eq!(
            subjects
                .iter()
                .map(|s| s.subject.as_str())
                .collect::<Vec<_>>(),
            [
                "state/logs/rotated",
                "state/sysinfo/health",
                "telemetry/sysinfo/disk/root/used"
            ]
        );
        assert_eq!(subjects[1].compared, 2, "one health document per origin");
        assert_eq!(crate::report::judgement_exit_code(&d.to_judgement()), 0);
    }

    /// One value moved on one host: the subject line says on how many of
    /// how many, and carries the example.
    #[test]
    fn a_changed_value_on_one_host_reads_as_one_of_n_on_its_subject() {
        let (a, mut b) = renamed("prod");
        let disk = b
            .rows
            .iter_mut()
            .find(|r| r.key == format!("prod/v1/{B2}/telemetry/sysinfo/disk/root/used"))
            .unwrap();
        disk.bytes = Some({
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode(r#"{"value":97.0}"#)
        });
        let d = diff_normalized(&a, &b, &plan(&a, &b), DiffOpts::default());
        assert!(d.differs());
        assert_eq!(d.changed.len(), 1);
        assert_eq!(
            d.changed[0].key,
            format!("prod/v1/{A2}/telemetry/sysinfo/disk/root/used"),
            "the changed key is spelled in a's origin"
        );
        let subjects = d.by_subject.as_option().unwrap();
        let disk = subjects
            .iter()
            .find(|s| s.subject == "telemetry/sysinfo/disk/root/used")
            .unwrap();
        assert_eq!((disk.compared, disk.differing), (2, 1));
        assert!(disk.example.as_ref().unwrap().value.is_some());
        assert_eq!(crate::report::judgement_exit_code(&d.to_judgement()), 1);
    }

    /// A key one side has and the other does not rolls up as only-in, and
    /// the holder is read through the rename like the key.
    #[test]
    fn a_key_only_one_side_holds_rolls_up_as_only_in() {
        let (a, mut b) = renamed("prod");
        b.rows.retain(|r| !r.key.ends_with("/logs/rotated"));
        let d = diff_normalized(&a, &b, &plan(&a, &b), DiffOpts::default());
        assert_eq!(d.removed, [format!("prod/v1/{A2}/state/logs/rotated")]);
        let logs = &d.by_subject.as_option().unwrap()[0];
        assert_eq!(logs.subject, "state/logs/rotated");
        assert_eq!((logs.compared, logs.only_in_a, logs.only_in_b), (0, 1, 0));
        assert!(
            d.changed.iter().all(|c| c.holder.is_none()),
            "a renamed holder is not a moved holder: {:?}",
            d.changed
        );
    }

    /// Over an incomplete plan the comparison is not made: the pairs it
    /// had and every unpaired origin ride the report, nothing is compared,
    /// and the judgement is the reserved non-verdict (RFC 13 §4.4).
    #[test]
    fn an_incomplete_plan_is_refused_not_compared_around() {
        let (a, mut b) = renamed("prod");
        b.rows
            .extend(host("prod", "h-bbbbbbbbbbb3", "db", &["logs"]));
        let plan = plan(&a, &b);
        assert_eq!(
            plan.unmapped.len(),
            3,
            "db is claimed twice in b, so a's db is unpaired too"
        );
        let d = diff_normalized(&a, &b, &plan, DiffOpts::default());
        assert!(d.refused());
        assert_eq!(d.unmapped.len(), plan.unmapped.len(), "never dropped");
        assert_eq!(
            d.origin_map.as_option().unwrap().len(),
            1,
            "web still paired"
        );
        assert!(d.by_subject.is_not_asked());
        assert!(d.added.is_empty() && d.removed.is_empty() && d.changed.is_empty());
        assert_eq!(d.unchanged, 0);
        assert!(!d.differs());
        assert_eq!(crate::report::judgement_exit_code(&d.to_judgement()), 2);
    }

    /// A `host_id` that names some other host is a fact, not the origin
    /// restated: it is left alone and the diff shows it.
    #[test]
    fn a_foreign_host_id_is_not_rewritten() {
        let (a, mut b) = renamed("prod");
        let health = b
            .rows
            .iter_mut()
            .find(|r| r.key == format!("prod/v1/{B1}/state/sysinfo/health"))
            .unwrap();
        health.bytes = Some({
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD
                .encode(r#"{"host_id":"h-000000000000","source":"web","status":"ok"}"#)
        });
        // The label is now unverified in b, so pair explicitly.
        let plan = plan_map(
            &origin_profiles(&a),
            &origin_profiles(&b),
            &[(
                zenkey::origin::HostId::parse(A1).unwrap(),
                zenkey::origin::HostId::parse(B1).unwrap(),
            )],
        )
        .unwrap();
        let d = diff_normalized(&a, &b, &plan, DiffOpts::default());
        let c = d
            .changed
            .iter()
            .find(|c| c.key.ends_with("/health"))
            .unwrap();
        let v = c.value.as_ref().unwrap();
        assert_eq!(v.changes[0].path(), "host_id");
    }

    #[test]
    fn a_subject_keeps_a_service_origin_and_a_foreign_key_verbatim() {
        assert_eq!(
            subject_of("acme", "acme/v1/h-aaaaaaaaaaa1/state/sysinfo/health"),
            "state/sysinfo/health"
        );
        assert_eq!(
            subject_of("acme", "acme/v1/@catalog/state/entity/x"),
            "@catalog/state/entity/x"
        );
        assert_eq!(subject_of("acme", "acme/plain/leak"), "acme/plain/leak");
        assert_eq!(
            subject_of("", "v1/h-aaaaaaaaaaa1/telemetry/sysinfo-2/cpu"),
            "telemetry/sysinfo-2/cpu"
        );
    }
}
