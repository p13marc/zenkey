//! The topic plane: what a key *is*, as a row and as a document.
//!
//! [`TopicInfo`] is the partial-never-absent one (RFC 09 §5.1 O1/O2): the
//! description ladder stops where the facts stop, and every rung below the
//! stop is absent rather than invented, with [`TopicVerdict`] naming which
//! rung was reached.

use crate::model::facts::{KeyDescription, KeyShape, Registration};
use serde::Serialize;
use std::collections::BTreeMap;
use zenkey::RateClass;

#[derive(Debug, Clone, Serialize)]
pub struct TopicRow {
    pub producer: String,
    pub registry_version: String,
    pub class: String,
    pub path: String,
    pub type_name: String,
    /// Trailing `{var...}` family: the registry fixes the shape, not the
    /// members.
    pub open_ended: bool,
    /// Registry version the subject first appeared in, when declared.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// A retired subject (from the slice's `[[deprecated]]` ledger, RFC 08
    /// §6) — rendered only under `topic list --deprecated`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub deprecated: bool,
    /// When it was retired, if the ledger says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deprecated_since: Option<String>,
    /// The declared replacement subject, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaced_by: Option<String>,
    /// The declared key-population bound (RFC 08 §2: mandatory on any
    /// `{var}` pattern; the budget review enforces). Additive (#221) — old
    /// consumers keep parsing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cardinality: Option<i64>,
    /// Declared-vs-observed key population — present only under
    /// `topic list --budget` (#221).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<BudgetCell>,
}

/// One `{var}` row's declared-vs-observed key population (#221).
///
/// Judged **per origin**: RFC 04 §1's table bounds cardinality per producer,
/// so one origin over the bound is conclusive and several origins' healthy
/// populations are never summed into a fake violation. `over` is the only
/// verdict this cell carries — observed *under* declared is not one (a
/// bounded window proves a lower bound, never the population, RFC 09 §5.1
/// O6), and a `{path...}` family is `exempt` and says so rather than
/// passing (the RFC 08 §6.1 v1.20 shape).
#[derive(Debug, Clone, Serialize)]
pub struct BudgetCell {
    /// The declared bound, when the subject declares one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declared: Option<i64>,
    /// Distinct concrete keys observed across all origins over the window.
    pub observed: usize,
    /// Origins that expanded this family.
    pub origins: usize,
    /// The origin with the most expansions — the one the bound judges.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worst_origin: Option<String>,
    /// That origin's distinct-key count (0 = family unobserved).
    pub worst_observed: usize,
    /// `Some("rest-variable")`: a `{path...}` family, unbounded by
    /// construction — exempt, and saying so.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exempt: Option<String>,
    /// The worst origin exceeds the declared bound — the finding.
    pub over: bool,
    /// Example expansions from the worst origin (capped).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
}

/// What a `--budget` column's numbers rest on (#221) — the O5/O6 coverage
/// statement: the window, the exact scopes watched, and the observer's
/// bound. Without it "observed 3" reads as "the population is 3", which a
/// bounded sweep never established.
#[derive(Debug, Clone, Serialize)]
pub struct BudgetWindow {
    pub window_s: f64,
    /// The selectors actually watched — coverage is a statement, not a vibe.
    pub scopes: Vec<String>,
    /// Distinct keys the bounded observer retained.
    pub keys: usize,
    /// Keys the observer retired to stay within its bound; non-zero makes
    /// every observed count a lower bound twice over.
    pub evicted: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TopicList {
    pub subjects: Vec<TopicRow>,
    /// The observation behind the rows' budget cells — present only under
    /// `topic list --budget` (#221).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<BudgetWindow>,
}

/// One key, described as far as the RFC 09 §5.1 ladder reached.
///
/// Redesigned in issue #34 from an all-or-nothing struct (whose builder
/// hard-errored on any key that was not a registered v1 data subject — an O1
/// violation) into a **partial** report: every key yields one, and `verdict`
/// says how far it got. Fields below the ladder's failure point are absent,
/// never defaulted.
#[derive(Debug, Clone, Serialize)]
pub struct TopicInfo {
    pub key: String,
    /// The ladder verdict, machine-stable (see [`TopicVerdict`]).
    pub verdict: TopicVerdict,
    /// Human-readable elaboration of the verdict (why, and what would answer
    /// it) — rendered, never parsed.
    pub note: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub producer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub variables: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// The declared `kind` token (RFC 08 §2, v1.32), when the registry
    /// declares one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qos: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_s: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cardinality: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Where the ladder stopped. Serialized snake_case; stable for scripts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TopicVerdict {
    /// Parses, refines, declared — the full story is present.
    Registered,
    /// Parses as a v1 data key; the producer's slice does not declare it.
    Unregistered,
    /// Parses; no loaded slice covers this producer (or service origin).
    NoSliceForProducer,
    /// Parses, but onto a verbatim plane — there is no `[[subject]]` surface
    /// to consult (RFC 03 §1.4).
    NotADataClass,
    /// A legal Zenoh key that is not this convention's (O1: a fact).
    NotV1,
    /// Sits under a different deployment base than the one configured.
    NotUnderBase,
    /// Parses as a data key, but no registry has been loaded — "not asked"
    /// is not "answered no" (O4).
    RegistryNotLoaded,
}

impl TopicInfo {
    /// Render a [`KeyDescription`] into the report shape.
    pub fn from_description(d: &KeyDescription) -> TopicInfo {
        let mut info = TopicInfo {
            key: d.key.clone(),
            verdict: TopicVerdict::NotV1,
            note: String::new(),
            origin: None,
            producer: None,
            class: None,
            subject: None,
            variables: BTreeMap::new(),
            payload_type: None,
            unit: None,
            kind: None,
            qos: None,
            ttl_s: None,
            rate: None,
            cardinality: None,
            encoding: None,
            since: None,
            description: None,
        };
        match &d.facts.shape {
            KeyShape::NotUnderBase => {
                info.verdict = TopicVerdict::NotUnderBase;
                info.note = "under a different deployment base than the configured one \
                             (RFC 03 §1.1); `zenctl base list` discovers the bases in use"
                    .into();
                return info;
            }
            KeyShape::Unparsed { reason } => {
                info.verdict = TopicVerdict::NotV1;
                info.note = format!(
                    "not a keyspace-v2 key — a fact, not an error (RFC 09 §5.1 O1): {reason}"
                );
                return info;
            }
            KeyShape::V1(v) => {
                info.origin = Some(v.origin.clone());
                info.class = Some(v.class.clone());
                info.producer = v.producer.clone();
            }
        }
        match &d.facts.registration {
            Registration::Registered(s) => {
                info.verdict = TopicVerdict::Registered;
                info.subject = Some(s.path.clone());
                info.variables = s.vars.iter().cloned().collect();
                info.payload_type = Some(s.type_name.clone());
                info.unit = s.unit.clone();
                info.kind = s.kind.as_ref().map(|k| k.token().to_string());
                info.qos = s.qos.as_ref().map(|q| q.token().to_string());
                info.encoding = s.encoding.as_ref().map(|e| e.as_encoding_str().to_string());
                info.ttl_s = s.ttl_s;
                // Declared since v1.0, dropped on this path until #221 — the
                // field existed and was never filled.
                info.cardinality = s.cardinality;
                // R2, the third recurrence of the same class (cardinality
                // pre-#221, then these): `rate` reached `SubjectFacts` and
                // died at this boundary; `since`/`description` never even
                // left the slice. The no-dead-field pin in
                // `report_contract.rs` now guards the whole struct.
                info.rate = s.rate.as_ref().map(RateClass::token);
                info.since = s.since.clone();
                info.description = s.description.clone();
            }
            Registration::Unregistered => {
                info.verdict = TopicVerdict::Unregistered;
                info.note = "parses as a v1 data key, but the producer's slice does not \
                             declare this subject — for a conforming producer, a subject \
                             that is not registered does not exist (RFC 08)"
                    .into();
            }
            Registration::NoSliceForProducer => {
                info.verdict = TopicVerdict::NoSliceForProducer;
                info.note = "no loaded registry slice covers this producer — `--registry \
                             <dir>` supplies slices offline; on-bus they come from \
                             introspect (RFC 08 §6)"
                    .into();
            }
            Registration::Unknown => {
                info.verdict = TopicVerdict::RegistryNotLoaded;
                info.note = "no registry loaded — \"not asked\" is not \"answered no\" \
                             (RFC 09 §5.1 O4)"
                    .into();
            }
            Registration::NotApplicable => {
                info.verdict = TopicVerdict::NotADataClass;
                info.note = "a verbatim plane, not a data class — there is no [[subject]] \
                             surface to describe (RFC 03 §1.4)"
                    .into();
            }
        }
        info
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::facts::describe_key;
    use crate::model::registry::SliceSet;

    /// O1/O2 end to end: every kind of key yields a TopicInfo, and the
    /// verdicts are distinct.
    #[test]
    fn topic_info_is_partial_never_absent() {
        let cases = [
            ("demo/example/foo", TopicVerdict::NotV1),
            (
                "v1/h-3fa9c2d41b7e/@rpc/sysinfo/introspect",
                TopicVerdict::NotADataClass,
            ),
            (
                "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu",
                TopicVerdict::RegistryNotLoaded,
            ),
        ];
        for (key, want) in cases {
            let info = TopicInfo::from_description(&describe_key("", key, None));
            assert_eq!(info.verdict, want, "{key}");
            assert!(!info.note.is_empty(), "{key} must explain itself");
        }
        let info = TopicInfo::from_description(&describe_key(
            "zensight",
            "other/v1/h-3fa9c2d41b7e/state/x/y",
            None,
        ));
        assert_eq!(info.verdict, TopicVerdict::NotUnderBase);
        // Partial means partial: nothing below the failure point is invented.
        assert!(info.origin.is_none() && info.payload_type.is_none());
        // Loaded-and-empty is a different fact from not-loaded (O4).
        let empty = SliceSet::default();
        let info = TopicInfo::from_description(&describe_key(
            "",
            "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu",
            Some(&empty),
        ));
        assert_eq!(info.verdict, TopicVerdict::NoSliceForProducer);
        // The ladder reached the parse rung, so structural facts ARE present…
        assert_eq!(info.origin.as_deref(), Some("h-3fa9c2d41b7e"));
        // …but no registry facts were invented.
        assert!(info.payload_type.is_none());
    }
}
