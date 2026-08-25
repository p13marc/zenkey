//! The `why` plane (#214): the ladder a silence is explained by.
//!
//! One [`Rung`] per question, each carrying a [`RungAnswer`] — which is the
//! judgement core itself, so "not asked" and "asked and unobservable" survive
//! all the way onto the wire instead of collapsing into a missing field.
//! [`WhyVerdict`] is the surface naming, and it is the documented case of
//! inverted polarity: `Explained` is the *finding*, and its CLI exits 0.

use serde::{Deserialize, Serialize};

use crate::judge::why::is_cause;

/// Every rung the `why` ladder can put, in ladder order.
///
/// The same promise as [`CheckId`](crate::report::CheckId), for the same
/// reason: scripts key on these ids and the GUI renders them, so new rungs
/// append and nothing renames one. It carries its own question, because the
/// question is *serialized beside the id* — they are one fact, and keeping
/// them apart is what let a `match` on the id go stale silently (#347).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RungId {
    ScopeReach,
    KeyParse,
    RegistryDeclared,
    OriginAlive,
    PublisherDeclared,
    StorageCoverage,
    StoredValue,
    SampleFreshness,
    AdminAnswered,
    WireHeard,
}

impl RungId {
    /// The ladder, in order. The `why` report emits exactly this, once each —
    /// a property that used to rest on a `debug_assert_eq!`, which only ran
    /// in debug builds.
    pub const ALL: [RungId; 10] = [
        RungId::ScopeReach,
        RungId::KeyParse,
        RungId::RegistryDeclared,
        RungId::OriginAlive,
        RungId::PublisherDeclared,
        RungId::StorageCoverage,
        RungId::StoredValue,
        RungId::SampleFreshness,
        RungId::AdminAnswered,
        RungId::WireHeard,
    ];

    /// The wire token, exactly as it serializes.
    pub fn as_str(self) -> &'static str {
        match self {
            RungId::ScopeReach => "scope-reach",
            RungId::KeyParse => "key-parse",
            RungId::RegistryDeclared => "registry-declared",
            RungId::OriginAlive => "origin-alive",
            RungId::PublisherDeclared => "publisher-declared",
            RungId::StorageCoverage => "storage-coverage",
            RungId::StoredValue => "stored-value",
            RungId::SampleFreshness => "sample-freshness",
            RungId::AdminAnswered => "admin-answered",
            RungId::WireHeard => "wire-heard",
        }
    }

    /// The question this rung puts, as prose — carried on the wire beside the
    /// id, which is why it lives on the type rather than in a `match` the
    /// compiler could not check.
    pub fn question(self) -> &'static str {
        match self {
            RungId::ScopeReach => "does a `**` explorer scope reach this key?",
            RungId::KeyParse => "does it parse as a v1 key under the base?",
            RungId::RegistryDeclared => "does a loaded registry slice declare it?",
            RungId::OriginAlive => "is the origin on the liveliness roster?",
            RungId::PublisherDeclared => "did any session declare a matching publisher?",
            RungId::StorageCoverage => "is a storage configured to capture it?",
            RungId::StoredValue => "does a stored value answer a bounded GET?",
            RungId::SampleFreshness => "is the last known sample within its declared ttl?",
            RungId::AdminAnswered => "is the admin space answering at all?",
            RungId::WireHeard => "did the key speak during a listen window?",
        }
    }

    /// Whether a **not-established** answer on this rung explains the
    /// silence.
    ///
    /// Policy, not rendering: both explorers and any script keying on the
    /// ndjson must agree on what exit 0 meant. The five that do are the ones
    /// whose failure *is* the reason nothing arrives. The five that do not,
    /// and why: `publisher-declared` because publishers declare lazily
    /// (RFC 08 §6.1), `storage-coverage` because uncovered volatile state is
    /// a legitimate deployment (RFC 04 §3.5), `stored-value` and
    /// `wire-heard` because an unanswered bounded ask is the very silence
    /// under investigation, and `admin-answered` because an absent admin
    /// space impairs the observation rather than explaining the key.
    pub fn is_cause_when_unestablished(self) -> bool {
        matches!(
            self,
            RungId::ScopeReach
                | RungId::KeyParse
                | RungId::RegistryDeclared
                | RungId::OriginAlive
                | RungId::SampleFreshness
        )
    }

    /// Read a rung id a caller supplied.
    pub fn parse(token: &str) -> Option<RungId> {
        RungId::ALL.into_iter().find(|r| r.as_str() == token)
    }
}

impl std::fmt::Display for RungId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One rung's answer — the [`Judgement`](crate::report::Judgement) core
/// (RFC 13, v1.24; RFC 09 §5.1 pre-v1.24), carried directly: since v1.24 the
/// ladder's three shipped states *are* three of the core's four poles, and
/// this alias is the fold. The serde tags are byte-identical to what #214
/// shipped (`established` / `not_established` + `reason` / `not_asked`).
///
/// A rung's judgement is over **its own question** (the rung's fact), not
/// over "is there a finding?" — which of its poles constitutes a finding is
/// per-rung policy, and [`is_cause`](crate::judge::why::is_cause) is where that policy lives. The rungs
/// currently never answer [`Unobservable`](crate::report::Judgement::Unobservable): an observation the
/// ladder could not obtain degrades the rung to `NotAsked` and rides
/// [`WhyReport::impairments`] instead.
///
/// A rung whose input was not fetched says
/// [`NotAsked`](crate::report::Judgement::NotAsked), never
/// `NotEstablished` (RFC 09 §5.1 O4).
pub type RungAnswer = crate::report::Judgement;

/// One rung of the ladder.
#[derive(Debug, Clone, Serialize)]
pub struct Rung {
    /// Which rung this is — stable, script-keyable.
    pub id: RungId,
    /// The question this rung puts, as prose. Serialized (a reader should not
    /// need this crate to know what was asked) and derived from `id`, so the
    /// two can no longer disagree.
    pub question: &'static str,
    #[serde(flatten)]
    pub answer: RungAnswer,
    /// What the answer rests on, one fact per line.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
}

/// The report's overall reading — what the CLI exits with (see the module
/// doc's exit table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WhyVerdict {
    /// An explanation of the silence was established (exit 0).
    Explained,
    /// No cause, and everything checked looks healthy (exit 1).
    Healthy,
    /// No cause, and the observation was impaired: an input this ladder
    /// wanted could not be obtained, so "healthy" cannot be claimed (exit 2).
    Impaired,
}

impl WhyVerdict {
    /// The [`Judgement`](crate::report::Judgement) mapping (RFC 13,
    /// v1.24), and it is **THE inverted one — read this before wiring exit
    /// codes**: `Explained` is *established-finding* (`Established`), because
    /// the thing `why` establishes is a cause — a finding about the fleet —
    /// even though this family's own historical CLI contract exits **0** for
    /// it (the module doc's table). The RFC 13 exit projection
    /// ([`crate::report::judgement_exit_code`]) therefore gives `why`'s
    /// three verdicts 1 / 0 / 2 in this order — the flip between the two
    /// contracts is carried **here, at the mapping**, never special-cased by
    /// a consumer downstream.
    ///
    /// | verdict | judgement | RFC 13 exit | historical `zenctl why` exit |
    /// |---|---|---|---|
    /// | `Explained` | `Established` (finding) | 1 | 0 |
    /// | `Healthy` | `NotEstablished` (clean) | 0 | 1 |
    /// | `Impaired` | `Unobservable` | 2 | 2 |
    pub fn to_judgement(self) -> crate::report::Judgement {
        use crate::report::Judgement;
        match self {
            WhyVerdict::Explained => Judgement::Established,
            WhyVerdict::Healthy => Judgement::NotEstablished {
                reason: "no cause established, and everything checked looks healthy".into(),
            },
            WhyVerdict::Impaired => Judgement::Unobservable {
                reason: "an input the ladder wanted could not be obtained — \"healthy\" \
                         cannot be claimed over questions it could not ask"
                    .into(),
            },
        }
    }
}

/// The inverse of [`WhyVerdict::to_judgement`], same (inverted) polarity:
/// an established finding is `Explained`, established-clean is `Healthy`,
/// and both unestablished poles fold to `Impaired` — a ladder nobody asked
/// is exactly a ladder that cannot claim health.
impl From<crate::report::Judgement> for WhyVerdict {
    fn from(j: crate::report::Judgement) -> WhyVerdict {
        use crate::report::Judgement;
        match j {
            Judgement::Established => WhyVerdict::Explained,
            Judgement::NotEstablished { .. } => WhyVerdict::Healthy,
            Judgement::NotAsked | Judgement::Unobservable { .. } => WhyVerdict::Impaired,
        }
    }
}

/// The ladder, assembled. One rung per [`RUNG_IDS`](crate::judge::common::RUNG_IDS) entry, in order, always —
/// a rung is never omitted, it degrades to `NotAsked`.
#[derive(Debug, Clone, Serialize)]
pub struct WhyReport {
    /// The key (or selector) as asked, verbatim.
    pub key: String,
    /// The base the ladder judged under. Empty is the bus-root deployment.
    pub base: String,
    pub rungs: Vec<Rung>,
    pub verdict: WhyVerdict,
    /// Inputs the ladder wanted and could not obtain — what makes a
    /// no-cause run [`WhyVerdict::Impaired`] rather than healthy.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub impairments: Vec<String>,
    /// The listen window that ran, seconds. Absent = not listened — which
    /// the `wire-heard` rung states rather than hiding (O4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listened_s: Option<f64>,
}

impl WhyReport {
    /// The rung ids whose answers established a cause — what exit 0 rests on.
    pub fn causes(&self) -> Vec<RungId> {
        self.rungs
            .iter()
            .filter(|r| is_cause(r.id, &r.answer))
            .map(|r| r.id)
            .collect()
    }
}

#[cfg(test)]
mod rung_id_tests {
    use super::*;

    /// The id vocabulary is API: additions append, nothing renames. If this
    /// test fails you are renaming a shipped rung id — don't (the
    /// [`CheckId`](crate::report::CheckId) discipline, applied here).
    #[test]
    fn rung_ids_are_stable() {
        assert_eq!(
            RungId::ALL.map(RungId::as_str),
            [
                "scope-reach",
                "key-parse",
                "registry-declared",
                "origin-alive",
                "publisher-declared",
                "storage-coverage",
                "stored-value",
                "sample-freshness",
                "admin-answered",
                "wire-heard",
            ]
        );
    }

    /// Every rung carries a question, and it is the type's — so a new rung
    /// cannot ship with the wrong prose, which is what a `match` on a
    /// `&'static str` allowed until #347 (its fallback was `unreachable!`).
    #[test]
    fn every_rung_id_has_a_question_and_round_trips() {
        for id in RungId::ALL {
            assert!(id.question().ends_with('?'), "{id}: {}", id.question());
            let json = serde_json::to_string(&id).unwrap();
            assert_eq!(json, format!("\"{}\"", id.as_str()));
            assert_eq!(serde_json::from_str::<RungId>(&json).unwrap(), id);
            assert_eq!(RungId::parse(id.as_str()), Some(id));
        }
        assert_eq!(RungId::parse("wire-herd"), None);
    }

    /// The cause poles, asserted as a set rather than by walking a report:
    /// this is the policy exit 0 rests on (RFC 13 §1.2).
    #[test]
    fn exactly_five_rungs_explain_a_silence() {
        let causes: Vec<&str> = RungId::ALL
            .into_iter()
            .filter(|r| r.is_cause_when_unestablished())
            .map(RungId::as_str)
            .collect();
        assert_eq!(
            causes,
            [
                "scope-reach",
                "key-parse",
                "registry-declared",
                "origin-alive",
                "sample-freshness"
            ]
        );
    }
}
