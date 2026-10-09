//! The snapshot plane (RFC 13 §4.4, v1.34; #219; zk2's since #612, FJ8b):
//! the `.zsnap` header, the row a snapshot is made of, what taking one
//! reports, and what comparing two says.
//!
//! A zk2 snapshot is the owners' current state kept on disk: one state GET
//! per selector, target `All` and consolidation `Latest` (spec §4.2 S4),
//! each row keyed by its zk2 address and resource, its payload checked
//! against the declared type, its stamp attributed to its owner, and its
//! holder established from presence. The one obligation a capture does not
//! carry is stated in the header and repeated by every renderer: **it was
//! collected *over* a span, never *at* an instant**
//! ([`ZsnapHeader::collection_span_s`]).
//!
//! Like [`ZrecHeader`](super::ZrecHeader), every shape here is read as well
//! as written — a `.zsnap` outlives the build that wrote it, and a diff
//! reads two of them back — so the module derives `Deserialize`. The row
//! spellings mirror [`SampleRow`](super::SampleRow)'s where the two carry
//! the same fact (`key`, `identity`, `delete`, `bytes`, `encoding`,
//! `timestamp`).

use serde::{Deserialize, Serialize};

use super::asked::u64_is_zero;
use super::diff::{ByteDiff, ValueDiff};
use super::judgement::Judgement;
use super::observe::{Conformance, KeyIdentity, LensPresence};

/// The first line of a `.zsnap` file (RFC 13 §4.4): what was asked, in
/// which namespace, when, and — the fact a capture does not need — over
/// what span. Recorded keys are full wire keys and are never re-derived
/// from the namespace (O3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZsnapHeader {
    /// Format version ([`ZSNAP_VERSION`](crate::tape::snapshot::ZSNAP_VERSION)):
    /// 2 since #612 (FJ8b), zk2's rows.
    pub zsnap: u32,
    /// The full wire selectors GET, one each: the state projection of what
    /// was asked (spec §1.3 — only `state` and `@state` answer a GET). A
    /// wildcard never matches a verbatim chunk, so `@state` is there only
    /// when a selector names it (O5).
    pub selectors: Vec<String>,
    /// The deployment namespace the operator stated (may be empty: the
    /// bus-root deployment).
    pub base: String,
    /// Collection start, RFC 3339 wall clock.
    pub collected_at: String,
    /// How long the collection took, first GET issued to last reply drained,
    /// the presence read included. **The number every rendering states**: a
    /// fan-in GET is collected over this, not at
    /// [`collected_at`](Self::collected_at).
    pub collection_span_s: f64,
    /// GETs issued — one per selector, whether or not it answered.
    pub asked: u64,
    /// Value and deletion replies received across every GET, before they
    /// were folded per key — so `answered - superseded` is the row count.
    pub answered: u64,
    /// Replies that arrived and were not kept past the observer's bound
    /// (O6). Absent at zero.
    #[serde(default, skip_serializing_if = "u64_is_zero")]
    pub elided: u64,
    /// Error replies: a refusal is not a value and not silence. Absent at
    /// zero.
    #[serde(default, skip_serializing_if = "u64_is_zero")]
    pub errors: u64,
    /// Replies on a non-concrete key, discarded by rule (R6). Not losses.
    /// Absent at zero.
    #[serde(default, skip_serializing_if = "u64_is_zero")]
    pub discarded: u64,
    /// Answers for one key that lost to a newer one from another selector's
    /// GET. Absent at zero.
    #[serde(default, skip_serializing_if = "u64_is_zero")]
    pub superseded: u64,
    /// The presence read holders and owners' clocks were attributed with,
    /// when one was made. Absent when not (O4) — and then every row's
    /// holder is [`Holder::Unattributed`] and every stamp unattributable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence: Option<LensPresence>,
}

/// One key, as the snapshot could establish it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotRow {
    /// Full wire key, verbatim — zk2's or not (O1).
    pub key: String,
    /// How far the key resolved: its address and resource, or the rung it
    /// stopped at (O2).
    pub identity: KeyIdentity,
    /// A deletion answered within the owner's window (`reply_del`, S3):
    /// a retirement, never an empty value. Always written.
    pub delete: bool,
    /// Base64 of the exact wire payload — lossless. Absent on a delete row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<String>,
    /// The reply's declared encoding, verbatim, when it carried one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    /// The value's stamp, when one rode it — whose clock,
    /// [`stamper`](Self::stamper) says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// Whose clock stamped it (O7): the owner's, another's, or
    /// unattributable. `None` exactly when [`timestamp`](Self::timestamp)
    /// is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stamper: Option<StamperWire>,
    /// The zenoh id of the session that *answered*, where the reply named
    /// its replier (`source_zid`, RFC 13 §4.4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_zid: Option<String>,
    /// The payload against its declared type: valid, invalid, undecodable,
    /// or not checked with why (§7.2, §7.3).
    pub conformance: Conformance,
    /// Who holds this value: evidence, not inference.
    pub holder: Holder,
}

impl SnapshotRow {
    /// The exact wire payload, decoded from `bytes`. `None` on a delete row,
    /// on a row that carries no `bytes`, or on base64 that does not decode.
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

/// Whose clock stamped a value (the tooling guide's O7; spec §3.3 0.10), on
/// the wire, tagged `kind`. The stamp's id rides in every variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StamperWire {
    /// The session zid the owner's descriptor states as `meta.zid`,
    /// compared by value: the owner's own clock (S1).
    Owner { id: String },
    /// The owner's zid is known, and this is another: a router's, commonly.
    Other { id: String },
    /// Nothing names the owner's zid: unknown, never foreign (O4).
    Unattributable { id: String },
}

/// Who holds a value: **evidence, not inference**. Tagged `kind`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Holder {
    /// The key's address held an instance token in the presence read —
    /// alive *at collection*, not fresh.
    Live {
        address: String,
        answered_by: AnsweredBy,
    },
    /// A value answered, and a complete presence read shows no instance of
    /// the address: an owner gone, or a store answering on its keys, which
    /// S4 forbids.
    NoInstance { address: String },
    /// The presence read was not made or did not complete, or the key names
    /// no address (O1, O4).
    Unattributed { reason: String },
}

/// Whether the replier was the owner (spec §4.2: the owner alone answers
/// for its state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnsweredBy {
    /// The reply came from the session the owner's descriptor names.
    Owner,
    /// Both identities are known and differ.
    Other,
    /// One side or the other is unknown: no replier id, or no `meta.zid`.
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
    /// Rows whose holder is [`Holder::NoInstance`].
    pub no_instance: u64,
    /// Rows whose holder is [`Holder::Unattributed`].
    pub unattributed: u64,
    /// Rows whose payload failed its declared type (invalid or
    /// undecodable).
    pub nonconforming: u64,
    /// Selectors whose GET could not be issued, or that reach no state key
    /// at all — asked, and the question never reached the bus. Named so the
    /// file says which of its selectors it does not cover (O5).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub incomplete: Vec<String>,
}

/// What two snapshots disagree about (RFC 13 §4.4), compared **by zk2 key**:
/// each row's key relative to its own file's namespace, so two deployments'
/// snapshots line up on the keys their services publish. Both spans are
/// stated, and the facets are kept apart.
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
    /// Keys in both whose value, conformance, holder or stamp moved — each
    /// facet reported on its own.
    pub changed: Vec<KeyChange>,
    /// Keys in both that are identical on every facet.
    pub unchanged: u64,
    /// Differing keys past the listing bound — counted, never dropped (O6).
    /// Absent at zero.
    #[serde(default, skip_serializing_if = "u64_is_zero")]
    pub truncated: u64,
}

impl SnapshotDiff {
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
    /// parsed files is always asked.
    pub fn to_judgement(&self) -> Judgement {
        if self.differs() {
            Judgement::Established
        } else {
            Judgement::NotEstablished {
                reason: "the two snapshots are identical on every facet".into(),
            }
        }
    }
}

/// One key present in both snapshots that moved on at least one facet. A
/// facet pair is present only when that facet differs; `timestamp` is
/// always carried because the two stamps are what date the change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyChange {
    /// The zk2 key, relative to each file's namespace (a key outside its
    /// file's namespace keeps its wire spelling).
    pub key: String,
    /// The structural diff, when both sides carried a structural value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<ValueDiff>,
    /// The byte comparison, when at least one side did not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<ByteDiff>,
    /// `(a, b)` when the conformance moved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conformance: Option<(Conformance, Conformance)>,
    /// `(a, b)` when the holder moved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder: Option<(Holder, Holder)>,
    /// `(a, b)` stamps, each absent where the value carried none.
    pub timestamp: (Option<String>, Option<String>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::KeyGroup;
    use serde_json::json;

    fn header() -> ZsnapHeader {
        ZsnapHeader {
            zsnap: 2,
            selectors: vec!["acme/zk2/*/*/*/state/**".into()],
            base: "acme".into(),
            collected_at: "2026-10-09T00:00:00Z".into(),
            collection_span_s: 1.25,
            asked: 1,
            answered: 2,
            elided: 0,
            errors: 0,
            discarded: 0,
            superseded: 0,
            presence: None,
        }
    }

    /// The header's O4/O6 spellings: a counter at zero is absent, a
    /// presence read not made is absent — and both read back to the value
    /// that wrote them.
    #[test]
    fn a_header_omits_zero_counters_and_an_unread_presence() {
        let h = header();
        let v = serde_json::to_value(&h).unwrap();
        assert_eq!(
            v,
            json!({
                "zsnap": 2,
                "selectors": ["acme/zk2/*/*/*/state/**"],
                "base": "acme",
                "collected_at": "2026-10-09T00:00:00Z",
                "collection_span_s": 1.25,
                "asked": 1,
                "answered": 2,
            })
        );
        let back: ZsnapHeader = serde_json::from_value(v).unwrap();
        assert_eq!(back, h);

        let asked = ZsnapHeader {
            elided: 3,
            discarded: 1,
            presence: Some(LensPresence {
                selector: "zk2/*/*/@zk/**".into(),
                complete: true,
                services: 2,
            }),
            ..header()
        };
        let v = serde_json::to_value(&asked).unwrap();
        assert_eq!(v["elided"], 3);
        assert_eq!(v["discarded"], 1);
        assert_eq!(v["presence"]["services"], 2);
        assert!(v.get("errors").is_none(), "still zero, still absent");
        let back: ZsnapHeader = serde_json::from_value(v).unwrap();
        assert_eq!(back, asked);
    }

    /// The tag spellings of the tagged vocabularies, pinned once.
    #[test]
    fn the_tagged_vocabularies_spell_snake_case_kinds() {
        assert_eq!(
            serde_json::to_value(StamperWire::Other { id: "ab12".into() }).unwrap(),
            json!({"kind": "other", "id": "ab12"})
        );
        assert_eq!(
            serde_json::to_value(StamperWire::Owner { id: "a1".into() }).unwrap(),
            json!({"kind": "owner", "id": "a1"})
        );
        assert_eq!(
            serde_json::to_value(Holder::Live {
                address: "host-a/tc".into(),
                answered_by: AnsweredBy::Owner,
            })
            .unwrap(),
            json!({"kind": "live", "address": "host-a/tc", "answered_by": "owner"})
        );
        assert_eq!(
            serde_json::to_value(Holder::NoInstance {
                address: "host-a/tc".into(),
            })
            .unwrap(),
            json!({"kind": "no_instance", "address": "host-a/tc"})
        );
        assert_eq!(
            serde_json::to_value(Holder::Unattributed {
                reason: "presence not read".into()
            })
            .unwrap(),
            json!({"kind": "unattributed", "reason": "presence not read"})
        );
    }

    /// A row omits what it does not hold and never nulls it (O4); a delete
    /// row has no `bytes` at all, and its conformance says it was not
    /// checked.
    #[test]
    fn a_delete_row_carries_no_payload_and_no_null() {
        let row = SnapshotRow {
            key: "acme/zk2/host-a/tc/tc.netif.v1/state/namespaces".into(),
            identity: KeyIdentity {
                group: KeyGroup::Resource {
                    address: "host-a/tc".into(),
                    iface: "tc.netif.v1".into(),
                    token: "state".into(),
                    resource: Some("state/namespaces".into()),
                },
                values: Default::default(),
                unresolved: None,
            },
            delete: true,
            bytes: None,
            encoding: None,
            timestamp: None,
            stamper: None,
            source_zid: None,
            conformance: Conformance::NotChecked {
                reason: "a deletion".into(),
            },
            holder: Holder::Unattributed {
                reason: "presence not read".into(),
            },
        };
        let v = serde_json::to_value(&row).unwrap();
        assert_eq!(
            v,
            json!({
                "key": "acme/zk2/host-a/tc/tc.netif.v1/state/namespaces",
                "identity": {
                    "is": "resource",
                    "address": "host-a/tc",
                    "iface": "tc.netif.v1",
                    "token": "state",
                    "resource": "state/namespaces",
                },
                "delete": true,
                "conformance": {"state": "not_checked", "reason": "a deletion"},
                "holder": {"kind": "unattributed", "reason": "presence not read"},
            })
        );
        assert_eq!(serde_json::from_value::<SnapshotRow>(v).unwrap(), row);
    }

    /// A diff of two identical files is clean, and a zero truncation is
    /// absent.
    #[test]
    fn an_identical_diff_is_clean() {
        let d = SnapshotDiff {
            a: header(),
            b: header(),
            added: vec![],
            removed: vec![],
            changed: vec![],
            unchanged: 4,
            truncated: 0,
        };
        let v = serde_json::to_value(&d).unwrap();
        assert!(v.get("truncated").is_none());
        assert_eq!(v["unchanged"], 4);
        assert!(!d.differs());
        assert_eq!(crate::report::judgement_exit_code(&d.to_judgement()), 0);
    }
}
