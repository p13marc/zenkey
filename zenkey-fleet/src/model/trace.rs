//! Relating a sample to a call (#215) — from values in hand, no session.
//!
//! [`crate::bus::write::call_traced`] holds the session, makes the call and
//! drains the window; everything that turns one observed sample into a
//! [`TraceRow`] lives here, so the same rules run in a unit test with no
//! bus — and so a future replay of a `.zrec` through a trace is a matter of
//! feeding rows, not of re-deriving the attribution.
//!
//! The rule is a **naming heuristic** and says so in the report
//! ([`crate::report::TRACE_CHAIN_RULE`]): RFC 05 §3's idiom puts the same
//! first chunk on the request procedure (`artifact/request`), the status
//! state (`state/<p>/artifact/<kind>`) and the audit event
//! (`events/<p>/artifact/<ulid>`), and that shared chunk is the only
//! declared link between them. A subject whose first chunk merely coincides
//! is tagged the same way. Nothing here claims a cause; a relation is a
//! statement about names in a registry.

use crate::model::facts::{KeyDescription, KeyShape, Registration};
use crate::model::timeline::{HlcStamp, TimelineRow};
use crate::report::{Provenance, TraceRelation, TraceRow};
use zenkey::slice::ProcedureDecl;

/// What a trace attributes against: the called origin, the producer named
/// on the call, and the procedure's first chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceTarget {
    /// The origin chunk (`h-…` or `@service`).
    pub origin: String,
    /// The producer named on the call. `None` for a service origin, whose
    /// keys carry no producer chunk (RFC 03 §1.5).
    pub producer: Option<String>,
    /// The first chunk of the procedure path — `artifact` for
    /// `artifact/request`.
    pub chain_chunk: String,
    /// Whether a registry was loaded at all. Without one the chain is
    /// unjudgeable, which is a third answer, not "undeclared" (O4).
    pub registry_loaded: bool,
}

impl TraceTarget {
    /// Which lane a described key falls in: `Some(relation)` for the called
    /// origin, `None` for any other origin (or a key that is not a v1 key
    /// under the base — those are somebody else's, counted alongside).
    pub fn relation_of(&self, desc: &KeyDescription) -> Option<TraceRelation> {
        let KeyShape::V1(facts) = &desc.facts.shape else {
            return None;
        };
        if facts.origin != self.origin {
            return None;
        }
        if !self.registry_loaded {
            return Some(TraceRelation::SameOriginRegistryNotLoaded);
        }
        let in_chain = matches!(desc.facts.registration, Registration::Registered(_))
            && facts.producer == self.producer
            && facts.subject.first().map(String::as_str) == Some(self.chain_chunk.as_str());
        Some(if in_chain {
            TraceRelation::DeclaredChain
        } else {
            TraceRelation::SameOriginUndeclared
        })
    }
}

/// The procedure's idiom label: its declared `kind` token verbatim —
/// `long-running`, `write`, `read` (RFC 08 §2) — or `undeclared`.
///
/// The token rather than [`zenkey::ProcedureKind`], because that enum knows
/// `read` and `write` only and `long-running` is exactly the one a trace is
/// for; the slice keeps an unknown token as spelled, so it is read from
/// there.
pub fn idiom_of(decl: Option<&ProcedureDecl>) -> String {
    decl.and_then(|d| d.kind.as_ref())
        .map(|k| k.token().to_string())
        .unwrap_or_else(|| "undeclared".to_string())
}

/// `sample − reference` in milliseconds, both NTP64 (seconds in the high 32
/// bits, a binary fraction in the low 32). Signed and truncated toward zero.
pub fn hlc_delta_ms(sample_ntp64: u64, reference_ntp64: u64) -> i64 {
    let diff = i128::from(sample_ntp64) - i128::from(reference_ntp64);
    // Truncation toward zero on the *signed* difference, so −0.4 ms reads
    // as 0 and not as −1: a sample stamped a hair before the reply is not
    // reported a full millisecond before it.
    i64::try_from(diff * 1000 / (1i128 << 32)).unwrap_or(if diff < 0 { i64::MIN } else { i64::MAX })
}

/// `self`, `foreign:<id>` or `unattributable:<id>` — the O7 provenance as
/// one column, the stamper named wherever it is not the publisher.
pub fn stamped_by(stamp: &HlcStamp) -> String {
    match stamp.provenance {
        Provenance::SelfStamped => "self".to_string(),
        Provenance::Foreign => format!("foreign:{}", stamp.stamper),
        Provenance::Unattributable => format!("unattributable:{}", stamp.stamper),
    }
}

/// One trace row from a timeline row (the clocks and provenance are the
/// timeline's, not a second vocabulary), the relation already decided, the
/// HLC Δ present exactly when both the sample and the reply were stamped.
pub fn trace_row(
    row: &TimelineRow,
    relation: TraceRelation,
    reply_ntp64: Option<u64>,
    break_before: Option<u64>,
) -> TraceRow {
    TraceRow {
        key: row.key.clone(),
        relation,
        arrival_delta_ms: row.t_us as f64 / 1_000.0,
        hlc: row.hlc.as_ref().map(HlcStamp::to_wire),
        hlc_delta_ms: match (&row.hlc, reply_ntp64) {
            (Some(h), Some(r)) => Some(hlc_delta_ms(h.ntp64, r)),
            _ => None,
        },
        stamped_by: row.hlc.as_ref().map(stamped_by),
        kind: row.kind,
        payload_bytes: row.payload_bytes,
        break_before,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::facts::describe_key;
    use crate::model::registry::SliceSet;
    use crate::report::{LaneId, RowKind};
    use zenkey::slice::{RegistrySlice, SubjectDecl};

    const ORIGIN: &str = "h-3fa9c2d41b7e";

    fn slices() -> SliceSet {
        let mut slice = RegistrySlice::new("1.0", "t", "demo");
        slice.subjects = vec![
            SubjectDecl::new("artifact/{kind}", zenkey::Class::State),
            SubjectDecl::new("artifact/{ulid}", zenkey::Class::Events),
        ];
        let mut proc_decl = ProcedureDecl::new("artifact/request");
        proc_decl.kind = Some(zenkey::Declared::Other("long-running".into()));
        slice.procedures = vec![proc_decl];
        let mut other = RegistrySlice::new("1.0", "t", "other");
        other.subjects = vec![SubjectDecl::new("artifact/copy", zenkey::Class::State)];
        SliceSet::from_slices(vec![slice, other])
    }

    fn target(registry_loaded: bool) -> TraceTarget {
        TraceTarget {
            origin: ORIGIN.into(),
            producer: Some("demo".into()),
            chain_chunk: "artifact".into(),
            registry_loaded,
        }
    }

    /// The four answers of the naming rule, and the fifth that is not an
    /// answer: another origin is nobody's lane.
    #[test]
    fn the_chain_is_the_registered_subject_sharing_the_first_chunk() {
        let s = slices();
        let rel = |key: &str| target(true).relation_of(&describe_key("", key, Some(&s)));
        assert_eq!(
            rel(&format!("v1/{ORIGIN}/state/demo/artifact/pcap")),
            Some(TraceRelation::DeclaredChain)
        );
        assert_eq!(
            rel(&format!("v1/{ORIGIN}/events/demo/artifact/01H")),
            Some(TraceRelation::DeclaredChain)
        );
        // Same origin, another producer — even one whose subject shares the
        // chunk: the chain is per called producer.
        assert_eq!(
            rel(&format!("v1/{ORIGIN}/state/other/artifact/copy")),
            Some(TraceRelation::SameOriginUndeclared)
        );
        // Same producer, unregistered subject.
        assert_eq!(
            rel(&format!("v1/{ORIGIN}/telemetry/demo/noise")),
            Some(TraceRelation::SameOriginUndeclared)
        );
        // Another origin: not attributed, not even "undeclared".
        assert_eq!(rel("v1/h-bbbbbbbbbbbb/state/demo/artifact/pcap"), None);
        // Not a v1 key at all: also nobody's.
        assert_eq!(rel("plain/key"), None);
    }

    /// No registry: every same-origin sample is *unjudgeable*, and the
    /// answer is that third state rather than "undeclared" (O4).
    #[test]
    fn without_a_registry_the_chain_is_unjudgeable_not_undeclared() {
        let rel = |key: &str| target(false).relation_of(&describe_key("", key, None));
        assert_eq!(
            rel(&format!("v1/{ORIGIN}/state/demo/artifact/pcap")),
            Some(TraceRelation::SameOriginRegistryNotLoaded)
        );
        assert_eq!(rel("v1/h-bbbbbbbbbbbb/state/demo/artifact/pcap"), None);
    }

    /// A service origin has no producer chunk (RFC 03 §1.5): the target
    /// says `None`, and a service key matches on the chunk alone.
    #[test]
    fn a_service_target_matches_keys_with_no_producer_chunk() {
        let mut slice = RegistrySlice::new("1.0", "t", "catalog");
        slice.service_origin = Some(zenkey::Declared::Known(
            zenkey::origin::ServiceOrigin::new("@catalog").unwrap(),
        ));
        slice.subjects = vec![SubjectDecl::new("entity/{id}", zenkey::Class::State)];
        let s = SliceSet::from_slices(vec![slice]);
        let t = TraceTarget {
            origin: "@catalog".into(),
            producer: None,
            chain_chunk: "entity".into(),
            registry_loaded: true,
        };
        assert_eq!(
            t.relation_of(&describe_key("", "v1/@catalog/state/entity/x", Some(&s))),
            Some(TraceRelation::DeclaredChain)
        );
    }

    #[test]
    fn the_idiom_is_the_declared_token_or_undeclared() {
        let s = slices();
        let decl = s.get("demo").unwrap().procedures.first();
        assert_eq!(idiom_of(decl), "long-running");
        assert_eq!(idiom_of(None), "undeclared");
        let mut write = ProcedureDecl::new("set");
        write.kind = Some(zenkey::Declared::Known(zenkey::ProcedureKind::Write));
        assert_eq!(idiom_of(Some(&write)), "write");
    }

    /// NTP64 arithmetic: one second is `1 << 32`; signed, truncated toward
    /// zero on both sides of the reply.
    #[test]
    fn hlc_delta_is_signed_milliseconds() {
        let one_s = 1u64 << 32;
        assert_eq!(hlc_delta_ms(one_s, 0), 1000);
        assert_eq!(hlc_delta_ms(0, one_s), -1000);
        // 0.4 ms either side rounds toward zero.
        let point_four_ms = one_s * 4 / 10_000;
        assert_eq!(hlc_delta_ms(point_four_ms, 0), 0);
        assert_eq!(hlc_delta_ms(0, point_four_ms), 0);
        assert_eq!(hlc_delta_ms(one_s + one_s / 2, one_s), 500);
    }

    /// The row carries the timeline's clocks: arrival always, the HLC pair
    /// only when both the sample and the reply were stamped.
    #[test]
    fn a_trace_row_takes_its_clocks_from_the_timeline_row() {
        let stamped = TimelineRow {
            key: format!("v1/{ORIGIN}/state/demo/artifact/pcap"),
            lane: LaneId::Origin {
                origin: ORIGIN.into(),
                producer: Some("demo".into()),
            },
            t_us: 12_345,
            hlc: Some(HlcStamp {
                ntp64: (1u64 << 32) * 2,
                stamper: "33".into(),
                provenance: Provenance::Foreign,
            }),
            source: None,
            sn: None,
            kind: RowKind::Put,
            payload_bytes: 7,
        };
        let row = trace_row(
            &stamped,
            TraceRelation::DeclaredChain,
            Some(1u64 << 32),
            Some(3),
        );
        assert_eq!(row.arrival_delta_ms, 12.345);
        assert_eq!(row.hlc.as_deref(), Some("8589934592/33"));
        assert_eq!(row.hlc_delta_ms, Some(1000));
        assert_eq!(row.stamped_by.as_deref(), Some("foreign:33"));
        assert_eq!(row.break_before, Some(3));

        // No reply HLC: the sample's own HLC still shows, the Δ does not.
        let row = trace_row(&stamped, TraceRelation::DeclaredChain, None, None);
        assert!(row.hlc.is_some());
        assert_eq!(row.hlc_delta_ms, None);

        let unstamped = TimelineRow {
            hlc: None,
            lane: LaneId::Unstamped,
            ..stamped
        };
        let row = trace_row(
            &unstamped,
            TraceRelation::SameOriginUndeclared,
            Some(1u64 << 32),
            None,
        );
        assert_eq!(
            (row.hlc, row.hlc_delta_ms, row.stamped_by),
            (None, None, None)
        );
    }
}
