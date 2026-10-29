//! `timing.v1` (#908, MC): what a tool's window reads of timed streams
//! (`spec/profiles/timing/v1.md` §2.7, §5).
//!
//! **The profile's own tokens.** Each of §5's three questions answers in
//! the tokens `conclusions.json` states — `yes`, `no`, `unobservable`,
//! `not_asked`, `not_this_profiles`, each with its reason — so a script
//! reads `missed` where the text says missed:
//! - "did this member meet its deadline over the window?" — `deadline`, no
//!   the finding;
//! - "did the owner keep this member's period over the window?" — `period`,
//!   no the finding;
//! - "did a sample of this member arrive expired?" — `lifespan`, yes the
//!   finding.
//!
//! Each is asked of one member ([`TimingMember`]), and read over a resource
//! ([`TimingResource`]) and over the run ([`TimingReport::run`]) by its
//! finding (text 0.2, F-264), a rolled-up answer carrying how many members
//! gave each token. [`TimingReport::judgement`] is the run as the one
//! [`Judgement`] a frontend exits through. Computed by
//! `crate::judge::timing`, pinned here.

use std::collections::BTreeMap;

use serde::Serialize;

use super::judgement::Judgement;

/// One of §5's answers, as `conclusions.json` spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TimingAnswerToken {
    Yes,
    No,
    Unobservable,
    NotAsked,
    NotThisProfiles,
}

impl TimingAnswerToken {
    /// The token, as serialized.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No => "no",
            Self::Unobservable => "unobservable",
            Self::NotAsked => "not_asked",
            Self::NotThisProfiles => "not_this_profiles",
        }
    }
}

/// §5's answer to one question of one member, with its reason in the
/// fixture's token and in words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimingAnswer {
    pub answer: TimingAnswerToken,
    /// The reason's token (`met`, `missed`, `owner_not_present`, `kept`,
    /// `not_kept`, `live`, `expired`, `no_bound`, …).
    pub reason: String,
    /// The reason in words.
    pub says: String,
    /// Whether this answer is the question's finding.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub finding: bool,
}

/// One question read over a resource or a run, by its finding (text 0.2,
/// F-264): the finding when any member's answer is the finding; the other
/// pole when every member's answer is that pole or set aside, and at least
/// one is that pole; when every member is set aside, not this profile's
/// when every one is, else not asked (text 0.4, F-277); unobservable
/// otherwise. A member not asked, or not this profile's, is set aside.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimingRollUp {
    pub answer: TimingAnswerToken,
    /// The first member whose answer is the roll-up's, in byte order of
    /// the members' keys (text 0.5, F-279): its key. Absent over no member.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<String>,
    /// That member's reason: the roll-up's (§5, F-279).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// How many members gave each answer.
    pub members: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub finding: bool,
}

/// The three questions, rolled up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimingQuestions {
    pub deadline: TimingRollUp,
    pub period: TimingRollUp,
    pub lifespan: TimingRollUp,
}

/// A timed resource's declaration as the tool read it (§2.2, §2.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimingDeclaration {
    /// `declared` or `invalid`.
    pub declaration: String,
    /// An invalid declaration's reasons, in §2.2's order: the fixture's
    /// tokens.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifespan_ms: Option<u64>,
    /// The deadline is below the period: no deadline (§2.5).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub below_period: bool,
    /// The declaration in words.
    pub says: String,
}

/// The owner of a resource as this window saw it (v1.md §0, §2.7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimingOwner {
    /// Present at every instant of the window.
    pub present_throughout: bool,
    /// Its state at the window's end: `present`, `unconfirmed`, `absent`
    /// or `unseen`.
    pub at_end: String,
    /// The `meta.zid`s held of its instances, by value as lowercase hex.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zids: Vec<String>,
    /// Its interface is marked `"token": false`: its instance tokens are
    /// its owner's tokens (v1.md §0).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tokenless: bool,
}

/// One timed resource of one owner in scope.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TimingResource {
    /// `<system>/<service>`.
    pub address: String,
    pub iface: String,
    /// The revision it was read through; absent when it came from the
    /// contracts in hand with no descriptor naming it (across a face).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fingerprint: String,
    /// `<kind token>/<template>`.
    pub resource: String,
    /// Its members' key expression, base-relative.
    pub selector: String,
    pub declaration: TimingDeclaration,
    /// Core §2.7's bound; absent for none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bound: Option<u64>,
    pub owner: TimingOwner,
    /// Members held.
    pub members: usize,
    /// Members first heard past the bound (§2.5): not held, not judged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub beyond_bound: Vec<String>,
    pub questions: TimingQuestions,
}

/// One miss the tracker reported (§2.5), its instants in milliseconds
/// from the window's start.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TimingMiss {
    pub at_ms: f64,
    pub due_ms: f64,
    /// Due plus the detection bound: the latest a reader may report it.
    pub by_ms: f64,
    /// The owner's state at it: `present`, `unconfirmed`, `absent` or
    /// `unseen`.
    pub owner: String,
}

/// How a member ended in the window (§2.5).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TimingEnd {
    pub at_ms: f64,
    /// `gone` or `forgotten`.
    pub reason: String,
}

/// Every sample's lifespan (§2.6), counted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimingLifespans {
    pub judged: u64,
    pub live: u64,
    pub expired: u64,
    pub unobservable: u64,
    /// Each reason the unobservable ones gave, by how many.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reasons: BTreeMap<String, u64>,
}

/// One timed member, as the window read it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TimingMember {
    /// The concrete key, base-relative.
    pub key: String,
    pub address: String,
    pub iface: String,
    pub resource: String,
    /// A member the tool could name without hearing it (§0).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub known: bool,
    /// How long it was held in the window (§2.7): its longest unbroken
    /// stretch, never the sum of its stretches (text 0.5), milliseconds.
    pub held_ms: f64,
    /// Its arrivals in the window: put samples, not expired.
    pub arrivals: u64,
    /// Its period's span, milliseconds; absent when its resource declares
    /// no period, or it was not heard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span_ms: Option<f64>,
    /// Every miss reported, and how many while its owner was present.
    pub missed: u32,
    pub missed_present: u32,
    /// The misses, the first 16.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub misses: Vec<TimingMiss>,
    /// `met` reports: arrivals after a miss.
    #[serde(default, skip_serializing_if = "super::asked::u32_is_zero")]
    pub met: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<TimingEnd>,
    /// Deletes received: no arrival (§2.5).
    #[serde(default, skip_serializing_if = "super::asked::u64_is_zero")]
    pub deletes: u64,
    pub lifespans: TimingLifespans,
    pub deadline: TimingAnswer,
    pub period: TimingAnswer,
    pub lifespan: TimingAnswer,
}

/// What the window's start stood on (§2.7, text 0.2, F-263): the presence
/// read made after the watch session's round trip, and the descriptor of
/// every instance it showed. The window starts once both stand, or, when
/// one could not, when the reading gave up waiting, and says which.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimingOpening {
    /// The presence read: `complete`, `incomplete` (ran to its timeout),
    /// `cut_off` (its session cut off while it ran), `unsettled` (the
    /// watch session's round trip did not stand), `failed`, or
    /// `across_face` (none is read).
    pub presence: String,
    /// The liveliness selector read, base-relative; absent across a face.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub selector: String,
    /// Instances in scope the read showed.
    pub instances: usize,
    /// …whose descriptor answered.
    pub descriptors: usize,
    /// Each instance whose descriptor did not, and why.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unanswered: Vec<String>,
    /// Whether both stood: the window's start is §2.7's.
    pub stood: bool,
}

/// `zenctl timing` (#908, MC): the timed members a scope reaches, over one
/// window, read as §2.7's tool reads them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TimingReport {
    /// The deployment's namespace; empty for the bus root.
    pub namespace: String,
    /// The selector asked, base-relative. Absent: every timed resource the
    /// descriptors in scope declare.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    /// Read across a constrained face (§2.8): no presence, the contracts in
    /// hand, and the deadline and period unobservable.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub across_face: bool,
    /// ε, the deployment's stated clock bound (§2.6); absent: none stated,
    /// every lifespan `no_bound`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock_bound_ms: Option<u64>,
    /// What the window opened on; absent when it never opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opening: Option<TimingOpening>,
    /// How long the window ran, seconds.
    pub window_s: f64,
    /// The selectors put to the bus, base-relative.
    pub asked: Vec<String>,
    pub resources: Vec<TimingResource>,
    pub members: Vec<TimingMember>,
    /// The three questions over every member of the run.
    pub run: TimingQuestions,
    /// Deliveries this reader lost, and its stalls: a miss or a period not
    /// kept is then never the owner's (§2.7).
    #[serde(default, skip_serializing_if = "super::asked::u64_is_zero")]
    pub lost: u64,
    /// Members refused at the reader's bound.
    #[serde(default, skip_serializing_if = "super::asked::u64_is_zero")]
    pub refused: u64,
    /// Samples on a key no timed resource holds, or a wildcard key (R6).
    #[serde(default, skip_serializing_if = "super::asked::u64_is_zero")]
    pub unresolved: u64,
    /// What in scope could not be read, one line each.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<String>,
    /// Set when the reading as a whole answered nothing, with why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unobservable: Option<String>,
}

impl TimingReport {
    /// The run as one [`Judgement`]: any question's finding over the run
    /// (a deadline missed while the owner was present and nothing lost, a
    /// period not kept with the owner present throughout and nothing lost,
    /// a sample expired) is the finding, exit 1. Clean, exit 0, when every
    /// question asked of a member is answered on its other pole over the
    /// run, and at least one was asked. No verdict otherwise, exit 2.
    pub fn judgement(&self) -> Judgement {
        let q = [
            ("deadline", &self.run.deadline),
            ("period", &self.run.period),
            ("lifespan", &self.run.lifespan),
        ];
        let findings: Vec<String> = q
            .iter()
            .filter(|(_, r)| r.finding)
            .map(|(n, r)| {
                format!(
                    "{n} {} ({})",
                    r.answer.as_str(),
                    r.first.as_deref().unwrap_or("?")
                )
            })
            .collect();
        if !findings.is_empty() {
            return Judgement::Established;
        }
        if let Some(why) = &self.unobservable {
            return Judgement::Unobservable {
                reason: why.clone(),
            };
        }
        let ns = if self.namespace.is_empty() {
            "the bus root".to_owned()
        } else {
            format!("namespace {:?}", self.namespace)
        };
        if self.resources.is_empty() {
            return Judgement::Unobservable {
                reason: format!(
                    "no timed resource in scope in {ns}: a reading over an empty scope answered \
                     nothing"
                ),
            };
        }
        // No member held, or none asked any question (text 0.4, F-277: a
        // roll-up over members all set aside is not asked; over none at
        // all, unobservable): nothing was answered.
        let asked: Vec<&(&str, &TimingRollUp)> = q
            .iter()
            .filter(|(_, r)| {
                !matches!(
                    r.answer,
                    TimingAnswerToken::NotAsked | TimingAnswerToken::NotThisProfiles
                )
            })
            .collect();
        // The three roll-ups are over one member set (`judge::timing::
        // questions`), so a member held is held for every question.
        let held = q.iter().any(|(_, r)| !r.members.is_empty());
        if !held {
            return Judgement::Unobservable {
                reason: format!(
                    "no question was asked of any member in {:.1}s: no member of the {} timed \
                     resource(s) in scope was held",
                    self.window_s,
                    self.resources.len()
                ),
            };
        }
        if asked.is_empty() {
            return Judgement::Unobservable {
                reason: format!(
                    "no question was asked of any member in {:.1}s: every member held was set \
                     aside, not asked or not this profile's, for every question (timing.v1 §5, \
                     text 0.4)",
                    self.window_s
                ),
            };
        }
        if let Some((n, r)) = asked
            .iter()
            .find(|(_, r)| r.answer == TimingAnswerToken::Unobservable)
        {
            return Judgement::Unobservable {
                reason: format!(
                    "the {n} question could not be answered over the run{} (timing.v1 §5)",
                    match (&r.first, &r.reason) {
                        (Some(k), Some(why)) => format!(": {k} is unobservable, {why}"),
                        _ => String::new(),
                    }
                ),
            };
        }
        Judgement::NotEstablished {
            reason: format!(
                "every question asked of the {} member(s) answered on its other pole: {}",
                self.members.len(),
                asked
                    .iter()
                    .map(|(n, r)| format!("{n} {}", r.answer.as_str()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::judgement_exit_code;
    use serde_json::json;

    fn answer(answer: TimingAnswerToken, reason: &str, finding: bool) -> TimingAnswer {
        TimingAnswer {
            answer,
            reason: reason.into(),
            says: "words".into(),
            finding,
        }
    }

    fn roll(answer: TimingAnswerToken, reason: Option<&str>, finding: bool) -> TimingRollUp {
        TimingRollUp {
            answer,
            first: reason.map(|_| "zk2/lab/pace/pace.v1/stream/cmd".into()),
            reason: reason.map(Into::into),
            members: BTreeMap::from([(answer.as_str().to_owned(), 1)]),
            finding,
        }
    }

    fn report(run: TimingQuestions) -> TimingReport {
        TimingReport {
            namespace: "acme".into(),
            selector: None,
            across_face: false,
            clock_bound_ms: Some(100),
            opening: Some(TimingOpening {
                presence: "complete".into(),
                selector: "zk2/lab/pace/@zk/**".into(),
                instances: 1,
                descriptors: 1,
                unanswered: vec![],
                stood: true,
            }),
            window_s: 5.0,
            asked: vec!["zk2/lab/pace/pace.v1/stream/cmd".into()],
            resources: vec![TimingResource {
                address: "lab/pace".into(),
                iface: "pace.v1".into(),
                fingerprint: String::new(),
                resource: "stream/cmd".into(),
                selector: "zk2/lab/pace/pace.v1/stream/cmd".into(),
                declaration: TimingDeclaration {
                    declaration: "declared".into(),
                    reasons: vec![],
                    period_ms: Some(100),
                    deadline_ms: Some(500),
                    lifespan_ms: Some(300),
                    below_period: false,
                    says: "words".into(),
                },
                bound: Some(1),
                owner: TimingOwner {
                    present_throughout: true,
                    at_end: "present".into(),
                    zids: vec!["a1".into()],
                    tokenless: false,
                },
                members: 1,
                beyond_bound: vec![],
                questions: run.clone(),
            }],
            members: vec![TimingMember {
                key: "zk2/lab/pace/pace.v1/stream/cmd".into(),
                address: "lab/pace".into(),
                iface: "pace.v1".into(),
                resource: "stream/cmd".into(),
                known: true,
                held_ms: 5000.0,
                arrivals: 49,
                span_ms: Some(5000.0),
                missed: 0,
                missed_present: 0,
                misses: vec![],
                met: 0,
                ended: None,
                deletes: 0,
                lifespans: TimingLifespans {
                    judged: 49,
                    live: 49,
                    expired: 0,
                    unobservable: 0,
                    reasons: BTreeMap::new(),
                },
                deadline: answer(TimingAnswerToken::Yes, "met", false),
                period: answer(TimingAnswerToken::Yes, "kept", false),
                lifespan: answer(TimingAnswerToken::No, "live", false),
            }],
            run,
            lost: 0,
            refused: 0,
            unresolved: 0,
            skipped: vec![],
            unobservable: None,
        }
    }

    fn clean() -> TimingQuestions {
        TimingQuestions {
            deadline: roll(TimingAnswerToken::Yes, Some("met"), false),
            period: roll(TimingAnswerToken::Yes, Some("kept"), false),
            lifespan: roll(TimingAnswerToken::No, Some("live"), false),
        }
    }

    /// The report's wire shape: §5's answers in the fixtures' tokens, every
    /// optional field absent when empty.
    #[test]
    fn the_timing_report_pins_its_shape() {
        let r = report(clean());
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(
            v["members"][0],
            json!({
                "key": "zk2/lab/pace/pace.v1/stream/cmd",
                "address": "lab/pace",
                "iface": "pace.v1",
                "resource": "stream/cmd",
                "known": true,
                "held_ms": 5000.0,
                "arrivals": 49,
                "span_ms": 5000.0,
                "missed": 0,
                "missed_present": 0,
                "lifespans": {"judged": 49, "live": 49, "expired": 0, "unobservable": 0},
                "deadline": {"answer": "yes", "reason": "met", "says": "words"},
                "period": {"answer": "yes", "reason": "kept", "says": "words"},
                "lifespan": {"answer": "no", "reason": "live", "says": "words"},
            })
        );
        assert_eq!(
            v["resources"][0]["declaration"],
            json!({
                "declaration": "declared",
                "period_ms": 100,
                "deadline_ms": 500,
                "lifespan_ms": 300,
                "says": "words",
            })
        );
        assert_eq!(
            v["resources"][0]["owner"],
            json!({"present_throughout": true, "at_end": "present", "zids": ["a1"]})
        );
        assert_eq!(
            v["run"]["deadline"],
            json!({
                "answer": "yes",
                "first": "zk2/lab/pace/pace.v1/stream/cmd",
                "reason": "met",
                "members": {"yes": 1},
            })
        );
        assert_eq!(
            v["opening"],
            json!({
                "presence": "complete",
                "selector": "zk2/lab/pace/@zk/**",
                "instances": 1,
                "descriptors": 1,
                "stood": true,
            })
        );
        let keys: std::collections::BTreeSet<&str> =
            v.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "asked",
                "clock_bound_ms",
                "members",
                "namespace",
                "opening",
                "resources",
                "run",
                "window_s",
            ]
            .into(),
            "every optional field absent when empty"
        );
        let miss = TimingMiss {
            at_ms: 2401.5,
            due_ms: 2400.0,
            by_ms: 2410.0,
            owner: "present".into(),
        };
        assert_eq!(
            serde_json::to_value(miss).unwrap(),
            json!({"at_ms": 2401.5, "due_ms": 2400.0, "by_ms": 2410.0, "owner": "present"})
        );
    }

    /// The run's exit: a finding of any question is 1; every question asked
    /// on its other pole is 0; an unobservable one, or nothing asked, is 2.
    #[test]
    fn the_run_exits_by_its_findings_then_its_poles() {
        assert_eq!(judgement_exit_code(&report(clean()).judgement()), 0);
        let mut missed = clean();
        missed.deadline = roll(TimingAnswerToken::No, Some("missed"), true);
        assert_eq!(judgement_exit_code(&report(missed).judgement()), 1);
        let mut expired = clean();
        expired.lifespan = roll(TimingAnswerToken::Yes, Some("expired"), true);
        expired.period = roll(TimingAnswerToken::Unobservable, Some("lost"), false);
        assert_eq!(
            judgement_exit_code(&report(expired).judgement()),
            1,
            "a finding stands beside an unobservable question"
        );
        let mut unbound = clean();
        unbound.lifespan = roll(TimingAnswerToken::Unobservable, Some("no_bound"), false);
        let j = report(unbound).judgement();
        assert_eq!(judgement_exit_code(&j), 2);
        assert!(
            matches!(&j, Judgement::Unobservable { reason } if reason.contains("no_bound")),
            "{j:?}"
        );
        let mut not_asked = clean();
        not_asked.lifespan = roll(TimingAnswerToken::NotAsked, None, false);
        assert_eq!(
            judgement_exit_code(&report(not_asked).judgement()),
            0,
            "a question nobody put is set aside"
        );
        let nothing = TimingQuestions {
            deadline: roll(TimingAnswerToken::NotAsked, None, false),
            period: roll(TimingAnswerToken::NotAsked, None, false),
            lifespan: roll(TimingAnswerToken::NotAsked, None, false),
        };
        assert_eq!(judgement_exit_code(&report(nothing).judgement()), 2);
        let mut empty = report(clean());
        empty.resources.clear();
        assert_eq!(judgement_exit_code(&empty.judgement()), 2);
    }
}
