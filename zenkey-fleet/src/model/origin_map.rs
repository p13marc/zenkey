//! Origin alignment across two deployments (#220, RFC 13 §4.4): which host
//! in snapshot `b` is which host in snapshot `a`, and on what evidence.
//!
//! "It works in staging" is unfalsifiable on a bus until two fleets can be
//! compared subject by subject — and RFC 03 §1.1 makes that possible,
//! because publishing identity sits at one fixed base-relative position.
//! Two fleets whose hosts are entirely different `h-…` values can be aligned
//! origin to origin, and then `sysinfo/cpu/usage` on the one is
//! `sysinfo/cpu/usage` on the other.
//!
//! The one way this could be a bad idea is being clever about it: a wrong
//! pairing produces confident nonsense. So the rules are few, ordered, and
//! never guess:
//!
//! 1. **Explicit** — the operator said `--map a=b`. Both ends must exist in
//!    their snapshot; a pairing that names an unknown origin is an error at
//!    the edge, not a silent no-op.
//! 2. **Label** — the `source` label the identity bridge carries
//!    (RFC 06 §6.2: `state/<producer>/health` and `state/<producer>/sensor`
//!    carry `host_id` beside `source`), when it is **verified** (`host_id`
//!    is the origin the document sits under) and **unique** among the
//!    still-unpaired origins on *both* sides.
//! 3. **Producer set** — the set of producer names an origin publishes
//!    under, when it is unique among the still-unpaired origins on both
//!    sides. Two identical hosts share a fingerprint and stay unpaired
//!    until the operator maps them; that is the point.
//!
//! Anything left is [`Unmapped`] with a reason that names the count it
//! failed on — "label `pve` claimed by 2 origins in b", "producer set
//! {sysinfo} matches 3 origins in a", "no health/sensor row: label not
//! asked" — because "I cannot map these" is both the honest answer and the
//! useful one. The plan lists them; the diff refuses over them
//! ([`diff_normalized`](crate::model::snapshot_diff::diff_normalized)).
//!
//! Pure: two [`Snapshot`]s in hand become two profile lists become one
//! [`MapPlan`]. Only host origins are profiled — a service origin
//! (`@catalog`) is the same chunk in every deployment and compares
//! verbatim.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use zenkey::origin::HostId;

use crate::model::facts::{KeyFacts, KeyShape, OriginKind};
use crate::model::snapshot_diff::structural_of;
use crate::report::{Asked, MapEvidence, OriginPair, Side, Snapshot, SnapshotRow, Unmapped};

/// The `source` label an origin's identity-bridge documents carry, and
/// whether they certify it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    /// The `source` field, verbatim.
    pub source: String,
    /// Every document carrying this label also carried a `host_id` equal to
    /// the origin it sits under — the pair RFC 06 §6.2 calls self-certifying.
    /// An unverified label never pairs anything.
    pub verified: bool,
}

/// One host origin as a snapshot shows it: what it publishes, and what it
/// calls itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginProfile {
    /// The origin chunk, verbatim.
    pub origin: String,
    /// Producer base names (`sysinfo`, not `sysinfo-2`) seen under this
    /// origin on any class or plane.
    pub producers: BTreeSet<String>,
    /// The label, three ways (RFC 09 §5.1 O4): `NotAsked` when the snapshot
    /// holds no `health`/`sensor` row for this origin — labels ride
    /// `state/*/health`, and a snapshot that did not include it never asked;
    /// `Asked(None)` when it holds one that carries no usable `source` (or
    /// several that disagree); `Asked(Some)` when it does.
    pub label: Asked<Option<Label>>,
}

impl OriginProfile {
    /// The verified label, if there is one — the only kind that pairs.
    fn verified_label(&self) -> Option<&str> {
        match &self.label {
            Asked::Asked(Some(l)) if l.verified => Some(&l.source),
            _ => None,
        }
    }
}

/// Whether a row is one of the two identity-bridge documents
/// (RFC 06 §6.2): `state/<producer>/health` or `state/<producer>/sensor`,
/// under a host origin.
fn is_bridge_document(facts: &KeyFacts) -> bool {
    let KeyShape::V1(f) = &facts.shape else {
        return false;
    };
    f.origin_kind == OriginKind::Host
        && f.class == "state"
        && f.producer.is_some()
        && matches!(f.subject.as_slice(), [s] if s == "health" || s == "sensor")
}

/// What one bridge document claims: its `source` and `host_id`, each when
/// present as a string.
fn bridge_claim(row: &SnapshotRow) -> Option<(Option<String>, Option<String>)> {
    let doc = structural_of(row)?;
    let field = |name: &str| doc.get(name).and_then(|v| v.as_str()).map(str::to_string);
    Some((field("source"), field("host_id")))
}

/// Profile every host origin a snapshot holds, in origin order.
pub fn origin_profiles(snapshot: &Snapshot) -> Vec<OriginProfile> {
    struct Acc {
        producers: BTreeSet<String>,
        /// `(source, host_id)` per bridge document seen.
        claims: Vec<(Option<String>, Option<String>)>,
        bridge_rows: usize,
    }
    let base = snapshot.header.base.as_str();
    let mut acc: BTreeMap<String, Acc> = BTreeMap::new();
    for row in &snapshot.rows {
        let facts = KeyFacts::project(base, &row.key);
        let KeyShape::V1(f) = &facts.shape else {
            continue;
        };
        if f.origin_kind != OriginKind::Host {
            continue;
        }
        let entry = acc.entry(f.origin.clone()).or_insert_with(|| Acc {
            producers: BTreeSet::new(),
            claims: Vec::new(),
            bridge_rows: 0,
        });
        if let Some(p) = &f.producer {
            entry.producers.insert(p.clone());
        }
        // A tombstoned health document is a retirement, not a claim.
        if is_bridge_document(&facts) && !row.delete {
            entry.bridge_rows += 1;
            if let Some(claim) = bridge_claim(row) {
                entry.claims.push(claim);
            }
        }
    }
    acc.into_iter()
        .map(|(origin, a)| {
            let label = if a.bridge_rows == 0 {
                Asked::NotAsked
            } else {
                let sources: BTreeSet<&str> =
                    a.claims.iter().filter_map(|(s, _)| s.as_deref()).collect();
                match sources.into_iter().collect::<Vec<_>>().as_slice() {
                    // One label, every document agreeing: verified when each
                    // of them also names this origin as its `host_id`.
                    [source] => {
                        let verified = a
                            .claims
                            .iter()
                            .filter(|(s, _)| s.as_deref() == Some(source))
                            .all(|(_, h)| h.as_deref() == Some(origin.as_str()));
                        Asked::Asked(Some(Label {
                            source: (*source).to_string(),
                            verified,
                        }))
                    }
                    // No `source` at all, or documents that disagree: not a
                    // label this alignment will use.
                    _ => Asked::Asked(None),
                }
            };
            OriginProfile {
                origin,
                producers: a.producers,
                label,
            }
        })
        .collect()
}

/// What the alignment decided: the pairs it can stand behind, and every
/// origin it could not pair, with the reason.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MapPlan {
    pub pairs: Vec<OriginPair>,
    pub unmapped: Vec<Unmapped>,
}

impl MapPlan {
    /// Every origin on both sides is paired.
    pub fn is_complete(&self) -> bool {
        self.unmapped.is_empty()
    }

    /// `b`'s origin → `a`'s, for the rewrite.
    pub fn b_to_a(&self) -> BTreeMap<&str, &str> {
        self.pairs
            .iter()
            .map(|p| (p.b.as_str(), p.a.as_str()))
            .collect()
    }
}

/// An explicit pairing the plan refuses — an error at the edge, because
/// silently ignoring a `--map` would compare the wrong keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapError {
    /// The named origin is not a host origin in that snapshot.
    UnknownOrigin { origin: String, side: Side },
    /// The same origin was named by two explicit pairings.
    PairedTwice { origin: String, side: Side },
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MapError::UnknownOrigin { origin, side } => write!(
                f,
                "--map names {origin}, which is not a host origin in {}",
                side_name(*side)
            ),
            MapError::PairedTwice { origin, side } => write!(
                f,
                "--map names {origin} (in {}) twice; an origin pairs once",
                side_name(*side)
            ),
        }
    }
}

impl std::error::Error for MapError {}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::A => "a",
        Side::B => "b",
    }
}

/// The set spelled the way the reasons spell it: `{logs, sysinfo}`.
fn producer_set(p: &OriginProfile) -> String {
    format!(
        "{{{}}}",
        p.producers.iter().cloned().collect::<Vec<_>>().join(", ")
    )
}

/// Plan the alignment: explicit pairs, then verified unique labels, then
/// unique producer sets; the rest listed with reasons. Deterministic —
/// explicit pairs in the order given, everything else in origin order.
pub fn plan_map(
    a: &[OriginProfile],
    b: &[OriginProfile],
    explicit: &[(HostId, HostId)],
) -> Result<MapPlan, MapError> {
    let (ia, ib) = (index(a), index(b));
    let mut used_a: BTreeSet<&str> = BTreeSet::new();
    let mut used_b: BTreeSet<&str> = BTreeSet::new();
    let mut pairs = Vec::new();

    // 1. Explicit — validated whole before anything is paired.
    for (x, y) in explicit {
        let (x, y) = (x.as_str(), y.as_str());
        if !ia.contains_key(x) {
            return Err(MapError::UnknownOrigin {
                origin: x.to_string(),
                side: Side::A,
            });
        }
        if !ib.contains_key(y) {
            return Err(MapError::UnknownOrigin {
                origin: y.to_string(),
                side: Side::B,
            });
        }
        if !used_a.insert(x) {
            return Err(MapError::PairedTwice {
                origin: x.to_string(),
                side: Side::A,
            });
        }
        if !used_b.insert(y) {
            return Err(MapError::PairedTwice {
                origin: y.to_string(),
                side: Side::B,
            });
        }
        pairs.push(OriginPair {
            a: x.to_string(),
            b: y.to_string(),
            evidence: MapEvidence::Explicit,
        });
    }

    // 2. Labels — verified, and unique among the unpaired on both sides.
    // The counts are taken once, before the pass: a label unique on both
    // sides has no other claimant, so pairing it changes no other count.
    {
        let (fa, fb) = (free(a, &used_a), free(b, &used_b));
        let (la, lb) = (by_label(&fa), by_label(&fb));
        for p in &fa {
            let Some(label) = p.verified_label() else {
                continue;
            };
            if la[label].len() != 1 {
                continue;
            }
            let Some([q]) = lb.get(label).map(Vec::as_slice) else {
                continue;
            };
            used_a.insert(&p.origin);
            used_b.insert(&q.origin);
            pairs.push(OriginPair {
                a: p.origin.clone(),
                b: q.origin.clone(),
                evidence: MapEvidence::Label {
                    source: label.to_string(),
                },
            });
        }
    }

    // 3. Producer sets — unique among the unpaired on both sides.
    {
        let (fa, fb) = (free(a, &used_a), free(b, &used_b));
        let (sa, sb) = (by_set(&fa), by_set(&fb));
        for p in &fa {
            if sa[&p.producers].len() != 1 {
                continue;
            }
            let Some([q]) = sb.get(&p.producers).map(Vec::as_slice) else {
                continue;
            };
            used_a.insert(&p.origin);
            used_b.insert(&q.origin);
            pairs.push(OriginPair {
                a: p.origin.clone(),
                b: q.origin.clone(),
                evidence: MapEvidence::ProducerSet,
            });
        }
    }

    // The rest, each with the count it failed on — recomputed over what is
    // still unpaired, which is the pool the operator's next `--map` draws
    // from.
    let (fa, fb) = (free(a, &used_a), free(b, &used_b));
    let mut unmapped = Vec::new();
    for p in &fa {
        unmapped.push(Unmapped {
            origin: p.origin.clone(),
            side: Side::A,
            reason: why_unpaired(p, &fa, &fb, Side::A),
        });
    }
    for p in &fb {
        unmapped.push(Unmapped {
            origin: p.origin.clone(),
            side: Side::B,
            reason: why_unpaired(p, &fb, &fa, Side::B),
        });
    }
    Ok(MapPlan { pairs, unmapped })
}

fn index(side: &[OriginProfile]) -> BTreeMap<&str, &OriginProfile> {
    side.iter().map(|p| (p.origin.as_str(), p)).collect()
}

/// The origins on one side not yet paired.
fn free<'p>(side: &'p [OriginProfile], used: &BTreeSet<&str>) -> Vec<&'p OriginProfile> {
    side.iter()
        .filter(|p| !used.contains(p.origin.as_str()))
        .collect()
}

/// Verified label → its claimants.
fn by_label<'p>(side: &[&'p OriginProfile]) -> BTreeMap<&'p str, Vec<&'p OriginProfile>> {
    let mut m: BTreeMap<&str, Vec<&OriginProfile>> = BTreeMap::new();
    for p in side {
        if let Some(l) = p.verified_label() {
            m.entry(l).or_default().push(p);
        }
    }
    m
}

/// Producer set → the origins carrying it.
fn by_set<'p>(
    side: &[&'p OriginProfile],
) -> BTreeMap<&'p BTreeSet<String>, Vec<&'p OriginProfile>> {
    let mut m: BTreeMap<&BTreeSet<String>, Vec<&OriginProfile>> = BTreeMap::new();
    for p in side {
        m.entry(&p.producers).or_default().push(p);
    }
    m
}

/// The reason `p` (on `side`, among `this` side's unpaired) did not pair
/// with any of `other`'s unpaired: the label's fate, then the producer
/// set's.
fn why_unpaired(
    p: &OriginProfile,
    this: &[&OriginProfile],
    other: &[&OriginProfile],
    side: Side,
) -> String {
    let (here, there) = match side {
        Side::A => ("a", "b"),
        Side::B => ("b", "a"),
    };
    let label = match &p.label {
        Asked::NotAsked => "no health/sensor row: label not asked".to_string(),
        Asked::Asked(None) => "health/sensor row carries no usable `source`".to_string(),
        Asked::Asked(Some(l)) if !l.verified => format!(
            "label `{}` not verified: `host_id` absent or not this origin",
            l.source
        ),
        Asked::Asked(Some(l)) => {
            let claims = |side: &[&OriginProfile]| {
                side.iter()
                    .filter(|q| q.verified_label() == Some(l.source.as_str()))
                    .count()
            };
            let (n_here, n_there) = (claims(this), claims(other));
            if n_here > 1 {
                format!("label `{}` claimed by {n_here} origins in {here}", l.source)
            } else if n_there == 0 {
                let unverified = other.iter().any(|q| {
                    matches!(&q.label, Asked::Asked(Some(m)) if m.source == l.source && !m.verified)
                });
                if unverified {
                    format!(
                        "label `{}` claimed in {there} only by an unverified document",
                        l.source
                    )
                } else {
                    format!("label `{}` claimed by no origin in {there}", l.source)
                }
            } else {
                format!(
                    "label `{}` claimed by {n_there} origins in {there}",
                    l.source
                )
            }
        }
    };
    let matches =
        |side: &[&OriginProfile]| side.iter().filter(|q| q.producers == p.producers).count();
    let (n_here, n_there) = (matches(this), matches(other));
    let set = producer_set(p);
    let producers = if n_there == 0 {
        format!("producer set {set} matches no origin in {there}")
    } else if n_here > 1 {
        format!("producer set {set} shared by {n_here} origins in {here}")
    } else {
        format!("producer set {set} matches {n_there} origins in {there}")
    };
    format!("{label}; {producers}")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::report::{AnsweredBy, Holder, RegistrationWire, VerdictWire, ZsnapHeader};

    fn header(base: &str) -> ZsnapHeader {
        ZsnapHeader {
            zsnap: 1,
            selectors: vec![zenkey::grammar::with_base(base, "v1/**")],
            base: base.into(),
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

    pub(crate) fn row(key: &str, body: &str) -> SnapshotRow {
        use base64::Engine as _;
        let origin = key
            .split('/')
            .skip_while(|c| *c != "v1")
            .nth(1)
            .unwrap_or("")
            .to_string();
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
            holder: Holder::Live {
                origin,
                answered_by: AnsweredBy::Stamper,
            },
        }
    }

    /// A host with a verified health label and the given extra producers.
    pub(crate) fn host(base: &str, origin: &str, label: &str, extra: &[&str]) -> Vec<SnapshotRow> {
        let k = |rel: &str| zenkey::grammar::with_base(base, format!("v1/{origin}/{rel}"));
        let mut rows = vec![
            row(
                &k("state/sysinfo/health"),
                &format!(r#"{{"host_id":"{origin}","source":"{label}","status":"ok"}}"#),
            ),
            row(&k("telemetry/sysinfo/disk/root/used"), r#"{"value":41.0}"#),
        ];
        for p in extra {
            rows.push(row(&k(&format!("state/{p}/rotated")), r#"{"n":3}"#));
        }
        rows
    }

    pub(crate) fn snap(base: &str, rows: Vec<SnapshotRow>) -> Snapshot {
        let mut rows = rows;
        rows.sort_by(|x, y| x.key.cmp(&y.key));
        Snapshot {
            header: header(base),
            rows,
        }
    }

    const A1: &str = "h-aaaaaaaaaaa1";
    const A2: &str = "h-aaaaaaaaaaa2";
    const B1: &str = "h-bbbbbbbbbbb1";
    const B2: &str = "h-bbbbbbbbbbb2";

    fn hid(s: &str) -> HostId {
        HostId::parse(s).unwrap()
    }

    /// The same fleet, every origin re-minted: every host pairs on its
    /// verified label, and nothing is left over.
    #[test]
    fn a_renamed_fleet_pairs_every_origin_on_its_label() {
        let a = snap(
            "acme",
            [
                host("acme", A1, "web", &[]),
                host("acme", A2, "db", &["logs"]),
            ]
            .concat(),
        );
        let b = snap(
            "acme",
            [
                host("acme", B1, "web", &[]),
                host("acme", B2, "db", &["logs"]),
            ]
            .concat(),
        );
        let (pa, pb) = (origin_profiles(&a), origin_profiles(&b));
        assert_eq!(pa.len(), 2);
        assert_eq!(
            pa[1].producers,
            ["logs", "sysinfo"].into_iter().map(String::from).collect()
        );
        assert_eq!(
            pa[0].label,
            Asked::Asked(Some(Label {
                source: "web".into(),
                verified: true
            }))
        );
        let plan = plan_map(&pa, &pb, &[]).unwrap();
        assert!(plan.is_complete());
        assert_eq!(
            plan.pairs,
            vec![
                OriginPair {
                    a: A1.into(),
                    b: B1.into(),
                    evidence: MapEvidence::Label {
                        source: "web".into()
                    }
                },
                OriginPair {
                    a: A2.into(),
                    b: B2.into(),
                    evidence: MapEvidence::Label {
                        source: "db".into()
                    }
                },
            ]
        );
    }

    /// Two origins on one side claiming one label: both stay unpaired, the
    /// reason names the label and the count — and, the producer sets being
    /// identical too, nothing falls through to a guess.
    #[test]
    fn an_ambiguous_label_leaves_both_claimants_unpaired_and_says_why() {
        let a = snap("acme", host("acme", A1, "node", &[]));
        let b = snap(
            "acme",
            [host("acme", B1, "node", &[]), host("acme", B2, "node", &[])].concat(),
        );
        let plan = plan_map(&origin_profiles(&a), &origin_profiles(&b), &[]).unwrap();
        assert!(plan.pairs.is_empty());
        let reasons: Vec<(&str, Side, &str)> = plan
            .unmapped
            .iter()
            .map(|u| (u.origin.as_str(), u.side, u.reason.as_str()))
            .collect();
        assert_eq!(
            reasons,
            vec![
                (
                    A1,
                    Side::A,
                    "label `node` claimed by 2 origins in b; producer set {sysinfo} matches 2 origins in b"
                ),
                (
                    B1,
                    Side::B,
                    "label `node` claimed by 2 origins in b; producer set {sysinfo} shared by 2 origins in b"
                ),
                (
                    B2,
                    Side::B,
                    "label `node` claimed by 2 origins in b; producer set {sysinfo} shared by 2 origins in b"
                ),
            ]
        );
    }

    /// `--map` is decided first: a pairing the operator stated stands even
    /// where the labels would have paired differently, and the label pass
    /// then works the pool that is left.
    #[test]
    fn an_explicit_pair_beats_a_conflicting_label() {
        let a = snap(
            "acme",
            [
                host("acme", A1, "web", &[]),
                host("acme", A2, "db", &["logs"]),
            ]
            .concat(),
        );
        let b = snap(
            "acme",
            [
                host("acme", B1, "web", &[]),
                host("acme", B2, "db", &["logs"]),
            ]
            .concat(),
        );
        let plan = plan_map(
            &origin_profiles(&a),
            &origin_profiles(&b),
            &[(hid(A1), hid(B2))],
        )
        .unwrap();
        assert_eq!(plan.pairs[0].evidence, MapEvidence::Explicit);
        assert_eq!(
            (plan.pairs[0].a.as_str(), plan.pairs[0].b.as_str()),
            (A1, B2)
        );
        // A2 ("db", {logs, sysinfo}) against the one left, B1 ("web",
        // {sysinfo}): neither label nor set agrees, so it is listed, not
        // forced.
        assert_eq!(plan.pairs.len(), 1);
        assert_eq!(plan.unmapped.len(), 2);
        assert_eq!(
            plan.unmapped[0].reason,
            "label `db` claimed by no origin in b; producer set {logs, sysinfo} matches no origin in b"
        );
    }

    /// An origin only one side has is listed under that side (RFC 13 §4.4:
    /// listed, never dropped), and the count of unpaired equals the input's
    /// unmatched set exactly.
    #[test]
    fn an_origin_only_in_b_is_unmapped_on_side_b() {
        let a = snap("acme", host("acme", A1, "web", &[]));
        let b = snap(
            "acme",
            [
                host("acme", B1, "web", &[]),
                host("acme", B2, "db", &["logs"]),
            ]
            .concat(),
        );
        let plan = plan_map(&origin_profiles(&a), &origin_profiles(&b), &[]).unwrap();
        assert_eq!(plan.pairs.len(), 1);
        assert_eq!(
            plan.unmapped,
            vec![Unmapped {
                origin: B2.into(),
                side: Side::B,
                reason: "label `db` claimed by no origin in a; producer set {logs, sysinfo} matches no origin in a".into(),
            }]
        );
    }

    /// No health rows at all: the label is *not asked*, and distinct
    /// producer sets still pair — on that evidence, and named as such.
    #[test]
    fn an_unlabelled_fleet_pairs_on_producer_sets_and_says_the_label_was_not_asked() {
        let strip = |rows: Vec<SnapshotRow>| -> Vec<SnapshotRow> {
            rows.into_iter()
                .filter(|r| !r.key.ends_with("/health"))
                .collect()
        };
        let a = snap(
            "acme",
            strip(
                [
                    host("acme", A1, "web", &[]),
                    host("acme", A2, "db", &["logs"]),
                ]
                .concat(),
            ),
        );
        let b = snap(
            "acme",
            strip(
                [
                    host("acme", B1, "web", &[]),
                    host("acme", B2, "db", &["logs"]),
                ]
                .concat(),
            ),
        );
        let pa = origin_profiles(&a);
        assert_eq!(pa[0].label, Asked::NotAsked);
        let plan = plan_map(&pa, &origin_profiles(&b), &[]).unwrap();
        assert!(plan.is_complete());
        assert!(
            plan.pairs
                .iter()
                .all(|p| p.evidence == MapEvidence::ProducerSet)
        );
        assert_eq!(plan.b_to_a()[B2], A2);

        // Identical producer sets and no labels: the honest answer names
        // both facts.
        let a = snap(
            "acme",
            strip([host("acme", A1, "x", &[]), host("acme", A2, "y", &[])].concat()),
        );
        let plan = plan_map(&origin_profiles(&a), &origin_profiles(&b), &[]).unwrap();
        assert_eq!(
            plan.unmapped[0].reason,
            "no health/sensor row: label not asked; producer set {sysinfo} shared by 2 origins in a"
        );
    }

    /// A label whose `host_id` is not the origin it sits under is a claim
    /// the document does not certify (RFC 06 §6.2): it never pairs.
    #[test]
    fn an_unverified_label_does_not_pair() {
        let mut a_rows = host("acme", A1, "web", &[]);
        a_rows[0] = row(
            &format!("acme/v1/{A1}/state/sysinfo/health"),
            r#"{"host_id":"h-000000000000","source":"web"}"#,
        );
        let a = snap("acme", a_rows);
        let b = snap(
            "acme",
            [host("acme", B1, "web", &[]), host("acme", B2, "web", &[])].concat(),
        );
        let pa = origin_profiles(&a);
        assert_eq!(
            pa[0].label,
            Asked::Asked(Some(Label {
                source: "web".into(),
                verified: false
            }))
        );
        let plan = plan_map(&pa, &origin_profiles(&b), &[]).unwrap();
        assert!(plan.pairs.is_empty());
        assert!(
            plan.unmapped[0]
                .reason
                .starts_with("label `web` not verified")
        );
        assert!(
            plan.unmapped[1]
                .reason
                .starts_with("label `web` claimed by 2 origins in b"),
            "{}",
            plan.unmapped[1].reason
        );
    }

    /// `--map` naming an origin the snapshot does not hold, or the same
    /// origin twice, is refused whole — before anything is paired.
    #[test]
    fn an_explicit_pair_must_name_origins_both_snapshots_hold() {
        let a = snap("acme", host("acme", A1, "web", &[]));
        let b = snap("acme", host("acme", B1, "web", &[]));
        let (pa, pb) = (origin_profiles(&a), origin_profiles(&b));
        assert_eq!(
            plan_map(&pa, &pb, &[(hid(A2), hid(B1))]),
            Err(MapError::UnknownOrigin {
                origin: A2.into(),
                side: Side::A
            })
        );
        assert_eq!(
            plan_map(&pa, &pb, &[(hid(A1), hid(B2))])
                .unwrap_err()
                .to_string(),
            "--map names h-bbbbbbbbbbb2, which is not a host origin in b"
        );
        let twice = snap(
            "acme",
            [host("acme", A1, "web", &[]), host("acme", A2, "db", &[])].concat(),
        );
        assert_eq!(
            plan_map(
                &origin_profiles(&twice),
                &pb,
                &[(hid(A1), hid(B1)), (hid(A2), hid(B1))]
            ),
            Err(MapError::PairedTwice {
                origin: B1.into(),
                side: Side::B
            })
        );
    }

    /// Service origins are not profiled: `@catalog` is `@catalog` in every
    /// deployment and compares verbatim.
    #[test]
    fn a_service_origin_is_not_profiled() {
        let mut rows = host("acme", A1, "web", &[]);
        rows.push(row("acme/v1/@catalog/state/entity/x", "{}"));
        let p = origin_profiles(&snap("acme", rows));
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].origin, A1);
    }
}
