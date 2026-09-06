//! The snapshot plane (RFC 13 §4.4, v1.34; #219): the `.zsnap` header, the
//! row a snapshot is made of, what taking one reports, and what comparing
//! two says.
//!
//! A snapshot is a fan-in GET kept on disk — the sibling of a `.zrec`
//! capture — and the one obligation a capture does not carry is stated in
//! the header and repeated by every renderer: **it was collected *over* a
//! span, never *at* an instant** ([`ZsnapHeader::collection_span_s`]).
//!
//! Like [`ZrecHeader`](super::ZrecHeader), every shape here is read as well
//! as written — a `.zsnap` outlives the build that wrote it, and a diff reads
//! two of them back — so the whole module derives `Deserialize`. The row
//! spellings mirror [`SampleRow`](super::SampleRow)'s where the two carry the
//! same fact (`key`, `delete`, `bytes`, `encoding`, `timestamp`, `source`),
//! so a reader of one dialect reads the other.

use serde::{Deserialize, Serialize};

use super::asked::{Asked, u64_is_zero};
use super::diff::{ByteDiff, ValueDiff};
use super::judgement::Judgement;

/// The first line of a `.zsnap` file (RFC 13 §4.4): what was asked, under
/// which base, when, and — the fact a capture does not need — over what
/// span. The `base` is the operator's *stated* deployment base at collection
/// time; recorded keys are full wire keys and are never re-derived from it
/// (O3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZsnapHeader {
    /// Format version ([`ZSNAP_VERSION`](crate::tape::snapshot::ZSNAP_VERSION)).
    pub zsnap: u32,
    /// The full wire selectors the snapshot GET, one fan-in each. A `**`
    /// never crosses an `@`-chunk, so a `**` snapshot excludes the verbatim
    /// planes by construction (O5).
    pub selectors: Vec<String>,
    /// The deployment base the operator resolved at collection time (may be
    /// empty: the base-less bus-root deployment).
    pub base: String,
    /// Collection start, RFC 3339 wall clock.
    pub collected_at: String,
    /// How long the collection took, first GET issued to last reply drained,
    /// the roster ask included. **The number every rendering states**: a
    /// fan-in GET is collected over this, not at [`collected_at`](Self::collected_at).
    pub collection_span_s: f64,
    /// GETs issued — one per selector, whether or not it answered.
    pub asked: u64,
    /// Value replies received across every GET, *before* last-writer-wins
    /// folded them per key — so `answered - superseded` is the row count.
    pub answered: u64,
    /// Replies that arrived and were not kept past the observer's bound
    /// (O6). Absent at zero.
    #[serde(default, skip_serializing_if = "u64_is_zero")]
    pub elided: u64,
    /// Error replies (RFC 05 §3 envelopes) — a refusal is not a value and
    /// not silence. Absent at zero.
    #[serde(default, skip_serializing_if = "u64_is_zero")]
    pub errors: u64,
    /// Answers that lost last-writer-wins to a newer reply on the same key
    /// (RFC 04 §1.2). Absent at zero.
    #[serde(default, skip_serializing_if = "u64_is_zero")]
    pub superseded: u64,
    /// How many origins the liveliness roster reported, when it was asked.
    /// Absent when it was not (O4) — and then every row's holder is
    /// [`Holder::Unattributed`].
    #[serde(default, skip_serializing_if = "Asked::is_not_asked")]
    pub roster: Asked<usize>,
}

/// One key, as the snapshot could establish it (RFC 13 §4.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotRow {
    /// Full wire key, verbatim — conformant or not (O1).
    pub key: String,
    /// A tombstone answered: authoritative retirement, never an empty put
    /// (RFC 04 §1.2). Always written.
    pub delete: bool,
    /// Base64 of the exact wire payload — lossless. Absent on a delete row:
    /// the tombstone is the whole fact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<String>,
    /// The reply's declared encoding, verbatim, when it carried one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    /// The value's HLC, when one rode it — provenance, whose clock
    /// [`stamper`](Self::stamper) says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// O7's classification of the HLC: whose clock stamped it. `None`
    /// exactly when [`timestamp`](Self::timestamp) is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stamper: Option<StamperWire>,
    /// The publishing entity, `zid:eid#sn`, when `SourceInfo` rode the reply
    /// — the [`SampleRow`](super::SampleRow) spelling. Usually absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The zenoh id of the session that *answered*, where the reply named
    /// its replier (RFC 13 §4.4 `source_zid`). Not the same fact as
    /// [`source`](Self::source): a storage answers for a publisher it is not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_zid: Option<String>,
    /// O2's rung, including "registry not loaded" — which is not
    /// "unregistered" (O4).
    pub registration: RegistrationWire,
    /// The three-valued payload verdict, never a boolean (#159).
    pub verdict: VerdictWire,
    /// Who holds this value: evidence, not inference.
    pub holder: Holder,
}

impl SnapshotRow {
    /// The exact wire payload, decoded from `bytes`. `None` on a delete row
    /// (the tombstone is the whole fact), on a row that carries no `bytes`,
    /// or on base64 that does not decode — a file a hand edited.
    pub fn payload(&self) -> Option<Vec<u8>> {
        use base64::Engine as _;
        if self.delete {
            return None;
        }
        base64::engine::general_purpose::STANDARD
            .decode(self.bytes.as_deref()?)
            .ok()
    }
}

/// Who stamped a value's HLC (RFC 09 §5.1 O7), on the wire — the
/// [`StampProvenance`](crate::StampProvenance) vocabulary, tagged `kind`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StamperWire {
    /// The publishing session stamped it: the publisher's own clock.
    SelfStamped,
    /// Another node stamped it (commonly a router); `id` is that node's.
    Foreign { id: String },
    /// Stamped, and nothing to compare the stamper against — unknown, not
    /// foreign (O4).
    Unattributable { id: String },
}

/// Where the registry ladder stopped for a key (RFC 09 §5.1 O2) — the
/// [`TopicVerdict`](super::TopicVerdict) vocabulary, spelled identically so
/// a script that reads `topic info` reads a snapshot row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationWire {
    /// Parses, refines, declared.
    Registered,
    /// Parses as a v1 data key; the producer's slice does not declare it.
    Unregistered,
    /// Parses; no loaded slice covers this producer (or service origin).
    NoSliceForProducer,
    /// Parses, but onto a verbatim plane — no `[[subject]]` surface exists
    /// (RFC 03 §1.4).
    NotADataClass,
    /// A legal Zenoh key that is not this convention's (O1: a fact).
    NotV1,
    /// Sits under a different deployment base than the one stated.
    NotUnderBase,
    /// Parses as a data key, and no registry was loaded — "not asked" is
    /// not "answered no" (O4).
    RegistryNotLoaded,
}

/// The RFC 08 §7 conformance verdict on the wire, tagged `state` — the
/// [`Verdict`](zenkey::schema::validate::Verdict) three-state, never
/// collapsed to a boolean.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum VerdictWire {
    /// Checked and conformant.
    Valid,
    /// Checked and non-conformant, each violation one sentence.
    Invalid { violations: Vec<String> },
    /// Not checked, and why — a token (`no_schema`, `no_registry`,
    /// `feature_off`, `kind_unsupported`, `undecodable`, `bad_schema`, or
    /// `tombstone` for a delete row, which carries nothing to check).
    NotValidated { reason: String },
}

/// Who holds a value (RFC 13 §4.4): **evidence, not inference**. Tagged
/// `kind`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Holder {
    /// The key's origin held an `alive` token during the collection —
    /// alive *at collection*, not fresh (freshness stays RFC 04 §4's).
    Live {
        origin: String,
        answered_by: AnsweredBy,
    },
    /// A value answered and no token was held: a storage remembers it,
    /// nobody is saying it now.
    StorageOnly { origin: String },
    /// The roster was not asked, or the key names no origin (O1).
    Unattributed { reason: String },
}

/// Whether the replier was the stamping entity (RFC 13 §4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnsweredBy {
    /// The reply came from the session whose clock stamped the value.
    Stamper,
    /// Both identities are known and they differ — a storage or a cache
    /// answered for the publisher.
    Other,
    /// One side or the other is unknown: no replier id, or no stamp.
    Unknown,
}

/// A whole `.zsnap` in memory: the header and every row, in key order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub header: ZsnapHeader,
    pub rows: Vec<SnapshotRow>,
}

/// What taking a snapshot did — the report `zenctl snapshot` renders.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SnapshotReport {
    /// The header as written: a snapshot names its question (O4) and its
    /// span.
    pub header: ZsnapHeader,
    /// Where the file went, when it went to one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out: Option<String>,
    /// Rows whose holder is [`Holder::Live`].
    pub live: u64,
    /// Rows whose holder is [`Holder::StorageOnly`].
    pub storage_only: u64,
    /// Rows whose holder is [`Holder::Unattributed`].
    pub unattributed: u64,
    /// Selectors whose GET could not be issued at all — asked, and the
    /// question never reached the bus. Counted in `asked`, absent from
    /// `answered`, and named here so the file says which of its selectors
    /// it does not cover (O5).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub incomplete: Vec<String>,
}

/// What two snapshots disagree about (RFC 13 §4.4): both spans stated,
/// the facets kept apart, and an origin that could not be paired listed
/// rather than dropped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotDiff {
    /// The earlier side's header — its span is half of what a diff MUST
    /// state.
    pub a: ZsnapHeader,
    /// The later side's header.
    pub b: ZsnapHeader,
    /// Keys in `b` and not in `a`.
    pub added: Vec<String>,
    /// Keys in `a` and not in `b`.
    pub removed: Vec<String>,
    /// Keys in both whose value, verdict, registration, holder or stamp
    /// moved — each facet reported on its own.
    pub changed: Vec<KeyChange>,
    /// Keys in both that are identical on every facet.
    pub unchanged: u64,
    /// Differing keys past the listing bound — counted, never dropped (O6).
    /// Absent at zero.
    #[serde(default, skip_serializing_if = "u64_is_zero")]
    pub truncated: u64,
    /// The origin alignment across deployments, when one was asked for
    /// (#220). Absent when not.
    #[serde(default, skip_serializing_if = "Asked::is_not_asked")]
    pub origin_map: Asked<Vec<OriginPair>>,
    /// Origins the alignment could not pair — listed, never dropped
    /// (RFC 13 §4.4). Absent when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unmapped: Vec<Unmapped>,
    /// The per-subject roll-up, when asked for. Absent when not — and
    /// absent on a [`refused`](Self::refused) diff, where the alignment was
    /// asked and the comparison declined.
    #[serde(default, skip_serializing_if = "Asked::is_not_asked")]
    pub by_subject: Asked<Vec<SubjectDelta>>,
}

impl SnapshotDiff {
    /// The alignment was asked and left origins unpaired, so the
    /// comparison was **not made** (#220): the pairs and the unpaired ride
    /// the report, nothing is listed as added, removed or changed, and the
    /// judgement is the reserved non-verdict — a diff that compared around
    /// an origin it could not place would be confident nonsense.
    pub fn refused(&self) -> bool {
        self.origin_map.is_asked() && !self.unmapped.is_empty()
    }

    /// Whether anything at all differs — added, removed, changed, or
    /// differences past the bound.
    pub fn differs(&self) -> bool {
        !self.added.is_empty()
            || !self.removed.is_empty()
            || !self.changed.is_empty()
            || self.truncated > 0
    }

    /// The RFC 13 §1.2 projection: a difference is the finding
    /// (`Established`, exit 1), identity the clean answer. A diff over two
    /// parsed files is always asked; the one unestablished pole is a
    /// [`refused`](Self::refused) alignment, where the observation cannot
    /// carry the claim (`Unobservable`, exit 2).
    pub fn to_judgement(&self) -> Judgement {
        if self.refused() {
            Judgement::Unobservable {
                reason: format!(
                    "{} origin(s) could not be paired; the comparison was not made",
                    self.unmapped.len()
                ),
            }
        } else if self.differs() {
            Judgement::Established
        } else {
            Judgement::NotEstablished {
                reason: "the two snapshots are identical on every facet".into(),
            }
        }
    }
}

/// One key present in both snapshots that moved on at least one facet.
/// A facet pair is present only when that facet differs; `timestamp` is
/// always carried because the two stamps are what date the change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyChange {
    pub key: String,
    /// The structural diff, when both sides carried a structural value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<ValueDiff>,
    /// The byte comparison, when at least one side did not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<ByteDiff>,
    /// `(a, b)` when the verdict moved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<(VerdictWire, VerdictWire)>,
    /// `(a, b)` when the registration rung moved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registration: Option<(RegistrationWire, RegistrationWire)>,
    /// `(a, b)` when the holder moved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder: Option<(Holder, Holder)>,
    /// `(a, b)` stamps, each absent where the value carried none.
    pub timestamp: (Option<String>, Option<String>),
}

/// One origin in `a` aligned with one in `b`, and on what evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginPair {
    pub a: String,
    pub b: String,
    pub evidence: MapEvidence,
}

/// Why two origins were paired. Tagged `kind`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MapEvidence {
    /// The operator said so (`--map a=b`).
    Explicit,
    /// The `source` label both sides' identity-bridge documents carry
    /// (RFC 06 §6.2: `state/<producer>/health` or `…/sensor`, `host_id`
    /// beside `source`) agreed, verified on both sides and claimed by no
    /// other origin on either. `source` is the label itself.
    Label { source: String },
    /// The two origins serve the same producer set and nothing else does.
    ProducerSet,
}

/// An origin the alignment could not pair — listed, never dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unmapped {
    pub origin: String,
    pub side: Side,
    pub reason: String,
}

/// Which snapshot an unpaired origin belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    A,
    B,
}

/// The per-subject roll-up of a diff: one row per subject path across every
/// origin, so a fleet-wide drift reads as one line rather than N.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubjectDelta {
    pub subject: String,
    /// Keys compared under this subject (present on both sides).
    pub compared: u64,
    /// Of those, how many differ.
    pub differing: u64,
    pub only_in_a: u64,
    pub only_in_b: u64,
    /// One differing key, when there is one, for the human render.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<KeyChange>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn header() -> ZsnapHeader {
        ZsnapHeader {
            zsnap: 1,
            selectors: vec!["acme/v1/**".into()],
            base: "acme".into(),
            collected_at: "2026-09-06T00:00:00Z".into(),
            collection_span_s: 1.25,
            asked: 1,
            answered: 2,
            elided: 0,
            errors: 0,
            superseded: 0,
            roster: Asked::NotAsked,
        }
    }

    /// The header's O4/O6 spellings: a counter at zero is absent, a roster
    /// not asked is absent — and both read back to the value that wrote
    /// them.
    #[test]
    fn a_header_omits_zero_counters_and_an_unasked_roster() {
        let h = header();
        let v = serde_json::to_value(&h).unwrap();
        assert_eq!(
            v,
            json!({
                "zsnap": 1,
                "selectors": ["acme/v1/**"],
                "base": "acme",
                "collected_at": "2026-09-06T00:00:00Z",
                "collection_span_s": 1.25,
                "asked": 1,
                "answered": 2,
            })
        );
        let back: ZsnapHeader = serde_json::from_value(v).unwrap();
        assert_eq!(back, h);

        let asked = ZsnapHeader {
            elided: 3,
            superseded: 1,
            roster: Asked::Asked(2),
            ..header()
        };
        let v = serde_json::to_value(&asked).unwrap();
        assert_eq!(v["elided"], 3);
        assert_eq!(v["superseded"], 1);
        assert_eq!(v["roster"], 2);
        assert!(v.get("errors").is_none(), "still zero, still absent");
        let back: ZsnapHeader = serde_json::from_value(v).unwrap();
        assert_eq!(back, asked);
    }

    /// The tag spellings of the four tagged vocabularies, pinned once.
    #[test]
    fn the_tagged_vocabularies_spell_snake_case_kinds() {
        assert_eq!(
            serde_json::to_value(StamperWire::Foreign { id: "ab12".into() }).unwrap(),
            json!({"kind": "foreign", "id": "ab12"})
        );
        assert_eq!(
            serde_json::to_value(StamperWire::SelfStamped).unwrap(),
            json!({"kind": "self_stamped"})
        );
        assert_eq!(
            serde_json::to_value(VerdictWire::NotValidated {
                reason: "no_registry".into()
            })
            .unwrap(),
            json!({"state": "not_validated", "reason": "no_registry"})
        );
        assert_eq!(
            serde_json::to_value(VerdictWire::Invalid {
                violations: vec!["/status: not one of …".into()]
            })
            .unwrap(),
            json!({"state": "invalid", "violations": ["/status: not one of …"]})
        );
        assert_eq!(
            serde_json::to_value(Holder::Live {
                origin: "h-3fa9c2d41b7e".into(),
                answered_by: AnsweredBy::Stamper,
            })
            .unwrap(),
            json!({"kind": "live", "origin": "h-3fa9c2d41b7e", "answered_by": "stamper"})
        );
        assert_eq!(
            serde_json::to_value(Holder::Unattributed {
                reason: "roster not asked".into()
            })
            .unwrap(),
            json!({"kind": "unattributed", "reason": "roster not asked"})
        );
        assert_eq!(
            serde_json::to_value(RegistrationWire::RegistryNotLoaded).unwrap(),
            json!("registry_not_loaded")
        );
        assert_eq!(
            serde_json::to_value(MapEvidence::Label {
                source: "pve".into()
            })
            .unwrap(),
            json!({"kind": "label", "source": "pve"})
        );
        assert_eq!(serde_json::to_value(Side::A).unwrap(), json!("a"));
    }

    /// A row omits what it does not hold and never nulls it (O4); a delete
    /// row has no `bytes` at all.
    #[test]
    fn a_delete_row_carries_no_payload_and_no_null() {
        let row = SnapshotRow {
            key: "acme/v1/h-3fa9c2d41b7e/state/sysinfo/health".into(),
            delete: true,
            bytes: None,
            encoding: None,
            timestamp: None,
            stamper: None,
            source: None,
            source_zid: None,
            registration: RegistrationWire::Registered,
            verdict: VerdictWire::NotValidated {
                reason: "tombstone".into(),
            },
            holder: Holder::StorageOnly {
                origin: "h-3fa9c2d41b7e".into(),
            },
        };
        assert_eq!(
            serde_json::to_value(&row).unwrap(),
            json!({
                "key": "acme/v1/h-3fa9c2d41b7e/state/sysinfo/health",
                "delete": true,
                "registration": "registered",
                "verdict": {"state": "not_validated", "reason": "tombstone"},
                "holder": {"kind": "storage_only", "origin": "h-3fa9c2d41b7e"},
            })
        );
    }

    /// A diff with nothing asked beyond the two files carries neither the
    /// origin map nor the subject roll-up, and a zero truncation is absent.
    #[test]
    fn an_unasked_alignment_is_absent_from_a_diff() {
        let d = SnapshotDiff {
            a: header(),
            b: header(),
            added: vec![],
            removed: vec![],
            changed: vec![],
            unchanged: 4,
            truncated: 0,
            origin_map: Asked::NotAsked,
            unmapped: vec![],
            by_subject: Asked::NotAsked,
        };
        let v = serde_json::to_value(&d).unwrap();
        for absent in ["truncated", "origin_map", "unmapped", "by_subject"] {
            assert!(v.get(absent).is_none(), "{absent} should be absent: {v}");
        }
        assert_eq!(v["unchanged"], 4);
        assert!(!d.differs());
        assert_eq!(crate::report::judgement_exit_code(&d.to_judgement()), 0);
    }
}
