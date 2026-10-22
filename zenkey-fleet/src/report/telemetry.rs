//! `telemetry.v1` (#819, TC): what a reader reads of metric members
//! (`spec/profiles/telemetry/v1.md` §5).
//!
//! **The profile's own tokens.** Each of §5's five questions answers in the
//! tokens its fixtures state, so a script reads `decreased` where the text
//! says decreased:
//! - "is this resource's declaration valid?" — [`TelemetryDeclaration`]
//!   (§2.2, §2.4): `declared` or `invalid`, with the reasons;
//! - "what is this member's kind, and its unit?" — [`TelemetryMember`]'s
//!   `kind` and `unit` (§2.5), each with where it came from; a unit absent
//!   is **unknown**, never dimensionless;
//! - "does this sample conform?" — [`TelemetrySamples`] (§2.6), counted by
//!   answer, every reason of the samples that did not;
//! - "did this counter or histogram go backwards within one epoch?" — each
//!   [`TelemetryPair`]'s `continuity` (§2.10), with how its epoch was had
//!   (`established` in-band or by presence, or `assumed`), and over a
//!   member's pairs [`TelemetryContinuity`]'s `answer` (§5, 0.4): a member
//!   that formed no pair, or only pairs whose epoch was assumed, could not
//!   tell;
//! - "what is this counter's rate over the pair?" — its `rate` and
//!   `rate_unit`, or `no_rate` with why (§2.11).
//!
//! [`TelemetryReport::judgement`] is the run as the one [`Judgement`] a
//! frontend exits through: a sample that does not conform, or a decrease
//! within an established epoch, is the finding; a counter or histogram
//! whose continuity could not be told leaves the run without a verdict;
//! everything else is informative. Computed by `crate::judge::telemetry`,
//! pinned here.
//!
//! **The pairs a row lists are a tail.** A member keeps its latest 16 pairs
//! (`crate::model::telemetry::PAIRS_KEPT`) beside a tally of all of them:
//! [`TelemetryContinuity`]'s `pairs` counts every one, and `pairs_listed`
//! the ones [`TelemetryMember::pairs`] holds, so a script tells a whole
//! list from a cut one (zk2py round 47's F-155).

use std::collections::BTreeMap;

use serde::Serialize;

use super::judgement::Judgement;

/// §5's first answer, as the fixtures spell it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeclarationToken {
    Declared,
    Invalid,
}

/// "Is this resource's declaration valid?" (§2.2, §2.4): what the reader
/// read of it before any sample.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TelemetryDeclaration {
    pub answer: DeclarationToken,
    /// An invalid declaration's reasons, in §2.2's order: the fixtures'
    /// tokens.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
    /// The carrier: `point` or `bare`. Absent when invalid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carrier: Option<String>,
    /// The declared kind; absent for a Point that says its own, or when
    /// invalid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buckets: Option<Vec<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic: Option<String>,
    /// Where an in-band epoch rides: `point` or `attachment`; absent when
    /// none does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch_in: Option<String>,
    /// The answer in words.
    pub says: String,
}

/// One metric resource of one owner in scope.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TelemetryResource {
    /// `<system>/<service>`.
    pub address: String,
    pub iface: String,
    /// The revision it was read through; absent when it came from the
    /// contracts in hand with no descriptor naming it (across a face).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fingerprint: String,
    /// `<kind token>/<template>`.
    pub resource: String,
    /// The key expression its members were subscribed under, base-relative.
    pub selector: String,
    pub declaration: TelemetryDeclaration,
    /// Members heard.
    pub members: usize,
}

/// "Does this sample conform?" (§2.6), over every sample of one member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TelemetrySamples {
    /// Values received; a delete is none.
    pub received: u64,
    pub conforms: u64,
    /// The finding.
    pub does_not: u64,
    /// Unobservable: the declaration invalid, or the sample undecodable.
    pub unobservable: u64,
    /// Why the last unobservable sample was (`declaration_invalid`,
    /// `undecodable`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unobservable_reason: Option<String>,
    /// State samples the late joiner's merge did not keep (§2.10, 0.2):
    /// read, and no sample of any pair.
    #[serde(default, skip_serializing_if = "super::asked::u64_is_zero")]
    pub not_kept: u64,
    /// Deletes received: no value.
    #[serde(default, skip_serializing_if = "super::asked::u64_is_zero")]
    pub deletes: u64,
    /// Each reason of §2.6, by how many samples gave it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub problems: BTreeMap<String, u64>,
    /// The reasons of the latest sample that did not conform, in §2.6's
    /// order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub latest_does_not: Vec<String>,
}

/// Every pair of one member, counted by answer (§2.10) and by how its epoch
/// was had (§2.9), and the member's own answer over them (§5, 0.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TelemetryContinuity {
    /// "Did this counter or histogram go backwards within one epoch?" over
    /// every pair of the member in the window, as `answer` (and `reason`):
    /// `established`, a decrease within an established epoch (the
    /// finding); `not_established`, an established pair and none
    /// decreased; `unobservable`, could not tell — no pair formed, or every
    /// pair's epoch assumed or the pair incomparable; `not_asked`, neither
    /// a counter nor a histogram.
    #[serde(flatten)]
    pub answer: Judgement,
    /// Every pair formed in the window.
    pub pairs: u64,
    /// The pairs [`TelemetryMember::pairs`] lists: the latest, at most 16.
    /// Fewer than `pairs` when the list is a tail.
    pub pairs_listed: u64,
    pub same_epoch: u64,
    pub reset: u64,
    /// The finding: a decrease within an established epoch.
    pub decreased: u64,
    /// Incomparable: the kinds, or a histogram's bounds or length, differ.
    pub unobservable: u64,
    /// Neither sample a counter or a histogram.
    pub not_asked: u64,
    /// Established in-band.
    pub in_band: u64,
    /// Established by presence: §2.9's residual risk applies.
    pub by_presence: u64,
    /// Assumed: neither in-band nor presence evidence.
    pub assumed: u64,
}

/// A pair's continuity, as the fixtures spell it (§2.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinuityToken {
    SameEpoch,
    Reset,
    Decreased,
    Unobservable,
    NotAsked,
}

/// How a pair's epoch was had (§2.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EpochToken {
    Established,
    Assumed,
}

/// What presence gave a pair (§2.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PairPresenceToken {
    /// What the reader saw over the pair.
    Observed,
    /// A loss between the two arrivals: nothing established by presence.
    SpansLoss,
    /// No presence at either arrival.
    None,
}

/// One pair of two consecutive conforming samples of a member (§2.10,
/// §2.11).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TelemetryPair {
    /// The two values, as their kinds carry them.
    pub earlier: serde_json::Value,
    pub later: serde_json::Value,
    pub continuity: ContinuityToken,
    /// Absent when the epoch was not asked of the pair.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<EpochToken>,
    /// `in_band` or `presence`, when established.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    pub presence: PairPresenceToken,
    /// Per second, by §2.11's formula.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<f64>,
    /// The counter's unit followed by `/s`; absent when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_unit: Option<String>,
    /// Why there is no rate: `kind`, `reset`, `decreased` or `elapsed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_rate: Option<String>,
    /// The time base: `timestamp`, `stamp` or `arrival` (§2.11).
    pub base: String,
    /// The elapsed time over that base, seconds.
    pub elapsed_s: f64,
}

/// A counter's latest rate (§2.11).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TelemetryRate {
    pub per_s: f64,
    /// The counter's unit followed by `/s`; absent when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

/// One metric member, as the reader read it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TelemetryMember {
    /// The concrete key, base-relative: the series (§2.12).
    pub key: String,
    pub address: String,
    pub iface: String,
    pub resource: String,
    /// "What is this member's kind?" (§2.5); absent when unobservable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// `declared` or `point`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind_from: Option<String>,
    /// "…and its unit?" (§2.5); absent when **unknown**, never
    /// dimensionless.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_from: Option<String>,
    pub samples: TelemetrySamples,
    /// The latest conforming value, as its kind carries it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<serde_json::Value>,
    pub continuity: TelemetryContinuity,
    /// The latest pairs, at most `crate::model::telemetry::PAIRS_KEPT` (16),
    /// oldest first: `continuity.pairs_listed` of `continuity.pairs`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pairs: Vec<TelemetryPair>,
    /// The latest rate derived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<TelemetryRate>,
}

/// The presence this reader read (§2.9): the seed and its basis at the end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TelemetryPresence {
    /// The liveliness selector seeded, base-relative.
    pub selector: String,
    /// Whether the last seed ran to its final reply (spec §8.1).
    pub complete: bool,
    /// Services in scope holding an instance token, by the seed.
    pub services: usize,
    /// What presence stood on at the window's end: `complete`,
    /// `incomplete`, `lost` or `unseeded`.
    pub basis: String,
}

/// Every count of a reading, summed over its members.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct TelemetryTotals {
    pub resources: usize,
    /// Resources whose declaration is invalid.
    pub invalid: usize,
    pub members: usize,
    pub samples: u64,
    pub conforms: u64,
    pub does_not: u64,
    pub unobservable: u64,
    pub pairs: u64,
    pub decreased: u64,
    /// Pairs of counters or histograms whose epoch was established, in-band
    /// or by presence.
    pub established: u64,
    /// …and assumed.
    pub assumed: u64,
    /// Counter or histogram members whose continuity could not be told: no
    /// pair formed, or only pairs whose epoch was assumed or that were
    /// incomparable (§5, 0.4).
    pub unanswered: u64,
}

/// `zenctl telemetry` (#819, TC): the metric members a scope reaches, read
/// as the profile's reader reads them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TelemetryReport {
    /// The deployment's namespace; empty for the bus root.
    pub namespace: String,
    /// The selector asked, base-relative. Absent: every metric resource
    /// the descriptors in scope declare.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    /// Presence as read; absent across a face, where it does not cross.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence: Option<TelemetryPresence>,
    /// Read across a constrained face, on the deployment's word: no
    /// presence, and the contracts in hand (§2.14).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub across_face: bool,
    /// How long the subscription listened, seconds.
    pub window_s: f64,
    /// The selectors put to the bus, base-relative in the namespace: what
    /// was read, and no wider (tooling guide O5).
    pub asked: Vec<String>,
    pub totals: TelemetryTotals,
    pub resources: Vec<TelemetryResource>,
    pub members: Vec<TelemetryMember>,
    /// Deliveries this reader lost, and how often its receive path stalled:
    /// each leaves every pair across it without presence.
    #[serde(default, skip_serializing_if = "super::asked::u64_is_zero")]
    pub lost: u64,
    /// Members refused at the reader's bound.
    #[serde(default, skip_serializing_if = "super::asked::u64_is_zero")]
    pub refused: u64,
    /// Samples on a key that resolved to no metric member, and on a
    /// wildcard key (R6): not read.
    #[serde(default, skip_serializing_if = "super::asked::u64_is_zero")]
    pub unresolved: u64,
    /// What in scope could not be read — a revision not in hand, a presence
    /// read or a GET that could not be made — one line each.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<String>,
    /// Set when the reading as a whole answered nothing, with why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unobservable: Option<String>,
}

impl TelemetryReport {
    /// The run as one [`Judgement`]: a sample that does not conform, or a
    /// decrease within an established epoch, is the finding (exit 1),
    /// whatever else is known. Otherwise no verdict (exit 2) when a question
    /// asked went unanswered — the reading could not be made, no metric
    /// resource is in scope, no sample of one arrived, none could be read,
    /// or a counter or histogram read formed no pair, or only pairs whose
    /// epoch was assumed, so no decrease of it could have been found (§5,
    /// 0.4); and clean (exit 0) when every question asked was answered
    /// without one.
    pub fn judgement(&self) -> Judgement {
        let t = &self.totals;
        if t.does_not > 0 || t.decreased > 0 {
            return Judgement::Established;
        }
        if let Some(why) = &self.unobservable {
            return Judgement::Unobservable {
                reason: why.clone(),
            };
        }
        if t.resources == 0 {
            return Judgement::Unobservable {
                reason: format!(
                    "no metric resource in scope in {}: a reading over an empty scope answered \
                     nothing",
                    if self.namespace.is_empty() {
                        "the bus root".to_owned()
                    } else {
                        format!("namespace {:?}", self.namespace)
                    }
                ),
            };
        }
        if t.conforms == 0 && t.unobservable == 0 {
            return Judgement::Unobservable {
                reason: format!(
                    "no sample of the {} metric resource(s) in scope arrived in {:.1}s: silence is \
                     never a verdict",
                    t.resources, self.window_s
                ),
            };
        }
        if t.conforms == 0 {
            return Judgement::Unobservable {
                reason: format!(
                    "none of the {} sample(s) received could be read: each was unobservable (an \
                     invalid declaration, or bytes that do not decode)",
                    t.unobservable
                ),
            };
        }
        if t.unanswered > 0 {
            let first = self
                .members
                .iter()
                .find_map(|m| match &m.continuity.answer {
                    Judgement::Unobservable { reason } => Some(format!("; {}: {reason}", m.key)),
                    _ => None,
                });
            return Judgement::Unobservable {
                reason: format!(
                    "{} counter or histogram member(s) read could not tell whether they went \
                     backwards within one epoch: no pair formed, or only pairs whose epoch was \
                     assumed, so no decrease could have been found (telemetry.v1 §5){}",
                    t.unanswered,
                    first.unwrap_or_default()
                ),
            };
        }
        Judgement::NotEstablished {
            reason: format!(
                "{} sample(s) of {} member(s) conform; {} pair(s), none decreased within an \
                 established epoch ({} established, {} assumed){}",
                t.conforms,
                t.members,
                t.pairs,
                t.established,
                t.assumed,
                if t.established == 0 {
                    "; no counter or histogram was read, so no continuity was asked"
                } else {
                    "; every counter or histogram read formed an established pair"
                }
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::judgement_exit_code;
    use serde_json::json;

    /// A counter member whose continuity answered `answer`.
    fn member(answer: Judgement) -> TelemetryMember {
        TelemetryMember {
            key: "zk2/lab/meter/meter.v1/stream/drops".into(),
            address: "lab/meter".into(),
            iface: "meter.v1".into(),
            resource: "stream/drops".into(),
            kind: Some("counter".into()),
            kind_from: Some("declared".into()),
            unit: None,
            unit_from: None,
            samples: TelemetrySamples {
                received: 1,
                conforms: 1,
                does_not: 0,
                unobservable: 0,
                unobservable_reason: None,
                not_kept: 0,
                deletes: 0,
                problems: BTreeMap::new(),
                latest_does_not: vec![],
            },
            latest: None,
            continuity: TelemetryContinuity {
                answer,
                pairs: 0,
                pairs_listed: 0,
                same_epoch: 0,
                reset: 0,
                decreased: 0,
                unobservable: 0,
                not_asked: 0,
                in_band: 0,
                by_presence: 0,
                assumed: 0,
            },
            pairs: vec![],
            rate: None,
        }
    }

    fn report() -> TelemetryReport {
        TelemetryReport {
            namespace: "acme".into(),
            selector: None,
            presence: Some(TelemetryPresence {
                selector: "zk2/*/*/@zk/**".into(),
                complete: true,
                services: 1,
                basis: "complete".into(),
            }),
            across_face: false,
            window_s: 5.0,
            asked: vec!["zk2/lab/meter/meter.v1/stream/drops".into()],
            totals: TelemetryTotals {
                resources: 1,
                members: 1,
                samples: 3,
                conforms: 3,
                pairs: 2,
                established: 2,
                ..TelemetryTotals::default()
            },
            resources: vec![],
            members: vec![],
            lost: 0,
            refused: 0,
            unresolved: 0,
            skipped: vec![],
            unobservable: None,
        }
    }

    /// The report's wire shape: §5's answers in the fixtures' tokens, a unit
    /// absent when unknown, and every optional field absent when empty.
    #[test]
    fn the_telemetry_report_pins_its_shape() {
        let mut r = report();
        r.resources = vec![TelemetryResource {
            address: "lab/meter".into(),
            iface: "meter.v1".into(),
            fingerprint: "sha256:ab".into(),
            resource: "stream/drops".into(),
            selector: "zk2/lab/meter/meter.v1/stream/drops".into(),
            declaration: TelemetryDeclaration {
                answer: DeclarationToken::Declared,
                reasons: vec![],
                carrier: Some("bare".into()),
                kind: Some("counter".into()),
                unit: None,
                buckets: None,
                semantic: None,
                epoch_in: None,
                says: "s".into(),
            },
            members: 1,
        }];
        r.members = vec![TelemetryMember {
            key: "zk2/lab/meter/meter.v1/stream/drops".into(),
            address: "lab/meter".into(),
            iface: "meter.v1".into(),
            resource: "stream/drops".into(),
            kind: Some("counter".into()),
            kind_from: Some("declared".into()),
            unit: None,
            unit_from: None,
            samples: TelemetrySamples {
                received: 3,
                conforms: 3,
                does_not: 0,
                unobservable: 0,
                unobservable_reason: None,
                not_kept: 0,
                deletes: 0,
                problems: BTreeMap::new(),
                latest_does_not: vec![],
            },
            latest: Some(json!(9)),
            continuity: TelemetryContinuity {
                answer: Judgement::NotEstablished { reason: "r".into() },
                pairs: 2,
                pairs_listed: 1,
                same_epoch: 1,
                reset: 1,
                decreased: 0,
                unobservable: 0,
                not_asked: 0,
                in_band: 0,
                by_presence: 2,
                assumed: 0,
            },
            pairs: vec![TelemetryPair {
                earlier: json!(5),
                later: json!(9),
                continuity: ContinuityToken::SameEpoch,
                epoch: Some(EpochToken::Established),
                via: Some("presence".into()),
                presence: PairPresenceToken::Observed,
                rate: Some(4.0),
                rate_unit: None,
                no_rate: None,
                base: "arrival".into(),
                elapsed_s: 1.0,
            }],
            rate: Some(TelemetryRate {
                per_s: 4.0,
                unit: None,
            }),
        }];
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(
            v["resources"][0]["declaration"],
            json!({"answer": "declared", "carrier": "bare", "kind": "counter", "says": "s"})
        );
        assert_eq!(
            v["members"][0],
            json!({
                "key": "zk2/lab/meter/meter.v1/stream/drops",
                "address": "lab/meter", "iface": "meter.v1", "resource": "stream/drops",
                "kind": "counter", "kind_from": "declared",
                "samples": {"received": 3, "conforms": 3, "does_not": 0, "unobservable": 0},
                "latest": 9,
                "continuity": {"answer": "not_established", "reason": "r",
                               "pairs": 2, "pairs_listed": 1,
                               "same_epoch": 1, "reset": 1, "decreased": 0,
                               "unobservable": 0, "not_asked": 0, "in_band": 0,
                               "by_presence": 2, "assumed": 0},
                "pairs": [{"earlier": 5, "later": 9, "continuity": "same_epoch",
                           "epoch": "established", "via": "presence", "presence": "observed",
                           "rate": 4.0, "base": "arrival", "elapsed_s": 1.0}],
                "rate": {"per_s": 4.0},
            })
        );
        for absent in [
            "selector",
            "across_face",
            "lost",
            "refused",
            "unresolved",
            "skipped",
            "unobservable",
        ] {
            assert!(v.get(absent).is_none(), "{absent}: {v}");
        }
        for (t, s) in [
            (ContinuityToken::SameEpoch, "same_epoch"),
            (ContinuityToken::Reset, "reset"),
            (ContinuityToken::Decreased, "decreased"),
            (ContinuityToken::Unobservable, "unobservable"),
            (ContinuityToken::NotAsked, "not_asked"),
        ] {
            assert_eq!(serde_json::to_value(t).unwrap(), json!(s));
        }
        assert_eq!(
            serde_json::to_value(PairPresenceToken::SpansLoss).unwrap(),
            json!("spans_loss")
        );
        // The member's own answer, in the judgement's four tokens, and a
        // whole list (`pairs_listed` equal to `pairs`) said as plainly as a
        // cut one.
        let mut c = r.members[0].continuity.clone();
        for (answer, want) in [
            (Judgement::Established, json!({"answer": "established"})),
            (Judgement::NotAsked, json!({"answer": "not_asked"})),
            (
                Judgement::Unobservable {
                    reason: "no pair formed".into(),
                },
                json!({"answer": "unobservable", "reason": "no pair formed"}),
            ),
        ] {
            c.answer = answer;
            let v = serde_json::to_value(&c).unwrap();
            for (k, x) in want.as_object().unwrap() {
                assert_eq!(&v[k], x, "{v}");
            }
            assert_eq!(
                v.get("reason").is_some(),
                want.get("reason").is_some(),
                "{v}"
            );
        }
        assert_eq!(v["totals"]["unanswered"], 0, "always said: {v}");
    }

    /// Only a sample that does not conform, or a decrease within an
    /// established epoch, is the 1; nothing answered is the 2; the rest the
    /// 0.
    #[test]
    fn the_judgement_is_the_findings_or_no_verdict() {
        let exit = |r: &TelemetryReport| judgement_exit_code(&r.judgement());
        assert_eq!(exit(&report()), 0);
        let mut r = report();
        r.totals.decreased = 1;
        assert_eq!(exit(&r), 1);
        let mut r = report();
        r.totals.does_not = 1;
        r.unobservable = Some("x".into());
        assert_eq!(exit(&r), 1, "a finding stands whatever else");
        let mut r = report();
        r.totals.resources = 0;
        assert_eq!(exit(&r), 2, "no metric resource in scope");
        let mut r = report();
        r.totals.conforms = 0;
        assert_eq!(exit(&r), 2, "nothing arrived");
        let mut r = report();
        r.totals.established = 0;
        r.totals.assumed = 2;
        r.totals.unanswered = 1;
        assert_eq!(exit(&r), 2, "presence unobservable throughout");
        r.totals.established = 1;
        assert_eq!(
            exit(&r),
            2,
            "another member's established pair answers nothing of this one"
        );
        r.totals.unanswered = 0;
        assert_eq!(exit(&r), 0, "every member with an established pair");
        // F-152: one sample of a counter and no pair could not tell, and
        // says which member.
        let mut r = report();
        r.totals.pairs = 0;
        r.totals.established = 0;
        r.totals.unanswered = 1;
        r.members = vec![member(Judgement::Unobservable {
            reason: "no pair formed".into(),
        })];
        let j = r.judgement();
        assert_eq!(judgement_exit_code(&j), 2, "{j:?}");
        assert!(
            matches!(&j, Judgement::Unobservable { reason }
                if reason.contains("zk2/lab/meter/meter.v1/stream/drops: no pair formed")),
            "{j:?}"
        );
        r.totals.does_not = 1;
        assert_eq!(exit(&r), 1, "a finding still wins");
    }
}
