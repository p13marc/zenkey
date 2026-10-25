//! `desired.v1` (#840, DD): what a target's roles converged on, as a tool
//! reads them (`spec/profiles/desired/v1.md` §2.17, §5).
//!
//! **The profile's own vocabulary, then the judgement shape.** §5's four
//! questions each answer in the tokens `conclusions.json` states (`yes`,
//! `no`, `not_yet`, `unobservable`, `not_asked`, each with a reason), per
//! role ([`DesiredAnswers`], named as the fixture names them):
//! - "has the target converged on its author's current document?" —
//!   `converged`, no the finding;
//! - "is its last act refused or failed?" — `refused`, yes the finding;
//! - "is what it acted on last-known?" — `last_known`, informative;
//! - "is a desired document in force?" — `in_force`, informative.
//!
//! Beside them, what a window found ([`DesiredBehind`], text 0.4): a target
//! "not yet" on one version of its author's for at least the window, by the
//! readings or by a trusted clock, a finding that leaves §5's answer at
//! "not yet". [`DesiredRow::judgement`] and [`DesiredReport::judgement`]
//! are the run as the one [`Judgement`] a frontend exits through.
//!
//! **What was read is shown as read.** The target's report is the
//! document as it read ([`DesiredReportSeen`]), the author's version is
//! its stamp as the profile writes one (`<time>/<id>`, §2.4), and an
//! archive's last-known of a slice whose author is absent is
//! [`DesiredArchived`]: last-known, never current, never part of a verdict
//! (core S6).

use serde::Serialize;

use super::config::FreshnessToken;
use super::health::ClockGround;
use super::judgement::Judgement;
use super::state::Stamp;

/// A question of §5, answered: the fixture's answer tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesiredAnswerToken {
    Yes,
    No,
    NotYet,
    Unobservable,
    NotAsked,
}

impl DesiredAnswerToken {
    /// The wire token, exactly as it serializes.
    pub fn as_str(self) -> &'static str {
        match self {
            DesiredAnswerToken::Yes => "yes",
            DesiredAnswerToken::No => "no",
            DesiredAnswerToken::NotYet => "not_yet",
            DesiredAnswerToken::Unobservable => "unobservable",
            DesiredAnswerToken::NotAsked => "not_asked",
        }
    }
}

/// One of §5's questions answered for one role: the answer, the fixture's
/// reason, and the reason in words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DesiredQuestion {
    pub answer: DesiredAnswerToken,
    pub reason: String,
    pub says: String,
}

/// §5's four questions, named as `conclusions.json` names them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DesiredAnswers {
    /// "Has the target converged on its author's current document?"
    pub converged: DesiredQuestion,
    /// "Is its last act refused or failed?"
    pub refused: DesiredQuestion,
    /// "Is what it acted on last-known?"
    pub last_known: DesiredQuestion,
    /// "Is a desired document in force?"
    pub in_force: DesiredQuestion,
}

/// What the target's descriptor says of the role (§2.3, §2.17 step 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesiredListing {
    /// Listed with a binding.
    Bound,
    /// Listed with `"bindings": []`: the kill switch (§2.12).
    Unbound,
    /// Not listed, or listed on no desired resource.
    NotListed,
    /// The descriptor was not read.
    Unread,
}

/// How the target's report was read (§2.10).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DesiredReportSeen {
    /// The key it came on, base-relative.
    pub key: String,
    /// The report, when it reads, as the profile writes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<zenkey_model::desired::Report>,
    /// Why it does not read (§2.10): `json`, `schema`, `stamp` or
    /// `revision_range`, then the words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unreadable: Option<String>,
    /// Its consistency findings (§2.10), the fixture's tokens: a report
    /// that reads with one is malformed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<String>,
    /// Its put's stamp: the target's own clock (core S1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stamp: Option<Stamp>,
    /// Its age by `freshness.v1` (§2.17 step 4).
    pub freshness: DesiredFreshness,
}

/// A report's `freshness.v1` verdict, as the report spells it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DesiredFreshness {
    pub verdict: FreshnessToken,
    pub reason: String,
    pub says: String,
}

/// What a GET of the slice drew from its author (§2.17 step 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesiredCurrentState {
    /// No GET made.
    NotRead,
    /// It ended at its timeout or with an error reply, or a reply carried
    /// no stamp.
    Incomplete,
    /// It drew nothing for the key.
    Silent,
    /// The author's latest put.
    Put,
    /// Its latest delete, or `reply_del`.
    Delete,
}

/// The author, as read (§2.17 step 6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DesiredAuthorSeen {
    /// `<system>/<service>`.
    pub address: String,
    /// Its presence: `present`, `absent`, `incomplete` or `across_face`.
    pub presence: String,
    /// Its version of the slice by S4's GET.
    pub current: DesiredCurrentState,
    /// That version's stamp, as the profile writes one (§2.4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stamp: Option<String>,
    /// Why its presence or its version is not known, when it is not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// An archive's last-known of the slice, its author absent (core §4.4,
/// S6): last-known, never current, and never part of a verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DesiredArchived {
    /// The archive, `<system>/<service>`.
    pub archive: String,
    /// A tombstone.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stamp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    /// Whether its alignment confirmed it (core §4.4).
    pub confirmed: bool,
}

/// How a target was found behind for at least the window (text 0.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesiredBehindBy {
    /// "Not yet" on one version in readings at least the window apart.
    Window,
    /// The version stamped longer ago than the window and the HLC delta,
    /// by a clock trusted to the delta (`freshness.v1` §2.6).
    Clock,
}

/// A target "not yet" on one version of its author's for at least the
/// window: the finding §2.17 lets a tool make of a target behind for
/// longer than it expects (text 0.4).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DesiredBehind {
    /// The author's version, `<time>/<id>`.
    pub version: String,
    /// For at least this long, seconds.
    pub since_s: f64,
    pub by: DesiredBehindBy,
}

/// One target's role, as read, with §5's answers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DesiredRow {
    /// The target, `<system>/<service>`.
    pub target: String,
    /// The role; `*` when none could be read.
    pub role: String,
    pub listing: DesiredListing,
    /// The required interface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface: Option<String>,
    /// The desired resource, `<kind token>/<template>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    /// The slice's key (§2.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slice: Option<String>,
    /// Where the slice came from: `author_contract`, `offline_contract` or
    /// `requirement` (text 0.4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slice_from: Option<String>,
    /// The author's declaration's reasons, when it is invalid (§2.2): a
    /// contract's finding.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub declaration_invalid: Vec<String>,
    /// The binding's problems (§2.3).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub binding_invalid: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<DesiredReportSeen>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<DesiredAuthorSeen>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archived: Vec<DesiredArchived>,
    pub answers: DesiredAnswers,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behind: Option<DesiredBehind>,
}

impl DesiredRow {
    /// What it found, when anything: a refused or failed act, the author's
    /// current version refused or failed, or a target behind for at least
    /// the window.
    pub fn finding(&self) -> Option<String> {
        let a = &self.answers;
        if a.refused.answer == DesiredAnswerToken::Yes {
            return Some(format!("its last act was {}", a.refused.reason));
        }
        if a.converged.answer == DesiredAnswerToken::No {
            return Some(format!(
                "its author's current version was {}",
                a.converged.reason
            ));
        }
        self.behind.as_ref().map(|b| {
            format!(
                "behind {} for at least {:.1}s ({})",
                b.version,
                b.since_s,
                match b.by {
                    DesiredBehindBy::Window => "the window",
                    DesiredBehindBy::Clock => "a trusted clock",
                }
            )
        })
    }

    /// Whether §2.17's premises left every question not asked of it: the
    /// kill switch, or a target not of the role. Such a row is no target
    /// asked.
    pub fn not_a_question(&self) -> bool {
        self.answers.converged.answer == DesiredAnswerToken::NotAsked
            && matches!(
                self.answers.converged.reason.as_str(),
                "unbound" | "not_a_target"
            )
    }

    /// Whether another reading could not move its answer: answered yes or
    /// no, not asked, or a finding.
    pub fn settled(&self) -> bool {
        self.finding().is_some()
            || matches!(
                self.answers.converged.answer,
                DesiredAnswerToken::Yes | DesiredAnswerToken::No | DesiredAnswerToken::NotAsked
            )
    }

    /// The row as one [`Judgement`]: a finding is the yes; converged is
    /// the no; anything else no verdict, with why.
    pub fn judgement(&self) -> Judgement {
        if self.finding().is_some() {
            return Judgement::Established;
        }
        let c = &self.answers.converged;
        match c.answer {
            DesiredAnswerToken::Yes => Judgement::NotEstablished {
                reason: format!("{} {}: converged ({})", self.target, self.role, c.reason),
            },
            DesiredAnswerToken::NotAsked => Judgement::NotAsked,
            _ => Judgement::Unobservable {
                reason: format!(
                    "{} {}: converged {} ({}) — {}",
                    self.target,
                    self.role,
                    c.answer.as_str().replace('_', " "),
                    c.reason,
                    c.says
                ),
            },
        }
    }
}

/// Where the targets were looked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DesiredPresence {
    /// The liveliness selector, base-relative in the namespace.
    pub selector: String,
    /// Whether the read ended at the routers' final reply (core §8.1).
    pub complete: bool,
    /// The read's session was cut off while it ran (core §8.1, 0.38).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cut_off_during: bool,
    /// Services holding an instance token in it.
    pub services: usize,
}

/// `zenctl desired` (#840, DD): each target's roles against their authors'
/// documents, read as the profile's tool reads them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DesiredReport {
    /// The deployment's namespace; empty for the bus root.
    pub namespace: String,
    /// The target asked of, `<system>/<service>`; absent for every one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// The role asked of; absent for every one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Presence as read; absent across a face, or cut off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence: Option<DesiredPresence>,
    /// Across a constrained face: the deployment's word stood for the
    /// target's descriptor.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub across_face: bool,
    /// The window, seconds: the longest a target may be behind. Absent
    /// when none was given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_s: Option<f64>,
    /// How many readings were made.
    pub readings: u32,
    pub clock: ClockGround,
    /// The selectors put to the bus, base-relative: what was read, and no
    /// wider.
    pub asked: Vec<String>,
    pub rows: Vec<DesiredRow>,
    /// Set when the reading could not be made, or found no target: why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unobservable: Option<String>,
}

impl DesiredReport {
    /// Whether another reading could not move the run's answer.
    pub fn settled(&self) -> bool {
        self.unobservable.is_none()
            && !self.rows.is_empty()
            && self.rows.iter().all(DesiredRow::settled)
    }

    /// The run as one [`Judgement`]: any role's finding is the yes (exit
    /// 1); no (exit 0) when every role asked converged, its author's
    /// current version converged or reverted, none refused, and at least
    /// one was asked; otherwise no verdict (exit 2), with why. A role the
    /// kill switch disarms, or a target not of the role, is not asked.
    pub fn judgement(&self) -> Judgement {
        if self.rows.iter().any(|r| r.finding().is_some()) {
            return Judgement::Established;
        }
        if let Some(why) = &self.unobservable {
            return Judgement::Unobservable {
                reason: why.clone(),
            };
        }
        let asked: Vec<&DesiredRow> = self.rows.iter().filter(|r| !r.not_a_question()).collect();
        if asked.is_empty() {
            return Judgement::NotAsked;
        }
        if let Some(open) = asked
            .iter()
            .map(|r| r.judgement())
            .find(|j| j.conclusive().is_none())
        {
            return match open {
                Judgement::NotAsked => Judgement::Unobservable {
                    reason: asked
                        .iter()
                        .find(|r| r.judgement() == Judgement::NotAsked)
                        .map(|r| {
                            format!(
                                "{} {}: converged not asked ({}) — {}",
                                r.target,
                                r.role,
                                r.answers.converged.reason,
                                r.answers.converged.says
                            )
                        })
                        .unwrap_or_default(),
                },
                other => other,
            };
        }
        Judgement::NotEstablished {
            reason: format!(
                "{} role(s) converged on their authors' current documents, none refused",
                asked.len()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::judgement_exit_code;
    use serde_json::json;

    fn answer(a: DesiredAnswerToken, reason: &str) -> DesiredQuestion {
        DesiredQuestion {
            answer: a,
            reason: reason.into(),
            says: "s".into(),
        }
    }

    fn row(
        converged: (DesiredAnswerToken, &str),
        refused: (DesiredAnswerToken, &str),
    ) -> DesiredRow {
        DesiredRow {
            target: "lab/worker".into(),
            role: "task".into(),
            listing: DesiredListing::Bound,
            interface: Some("tasking.v1".into()),
            resource: Some("state/tasks/{system}/{service}".into()),
            slice: Some("zk2/lab/author/tasking.v1/state/tasks/lab/worker".into()),
            slice_from: Some("author_contract".into()),
            declaration_invalid: vec![],
            binding_invalid: vec![],
            report: None,
            author: None,
            archived: vec![],
            answers: DesiredAnswers {
                converged: answer(converged.0, converged.1),
                refused: answer(refused.0, refused.1),
                last_known: answer(DesiredAnswerToken::No, "current"),
                in_force: answer(DesiredAnswerToken::Yes, "desired"),
            },
            behind: None,
        }
    }

    fn report(rows: Vec<DesiredRow>) -> DesiredReport {
        DesiredReport {
            namespace: "acme".into(),
            target: Some("lab/worker".into()),
            role: None,
            presence: Some(DesiredPresence {
                selector: "zk2/lab/worker/@zk/**".into(),
                complete: true,
                cut_off_during: false,
                services: 1,
            }),
            across_face: false,
            window_s: None,
            readings: 1,
            clock: ClockGround::None,
            asked: vec!["zk2/lab/worker/@zk/**".into()],
            rows,
            unobservable: None,
        }
    }

    /// The report's wire shape: §5's answers in the fixture's tokens, the
    /// report as the profile writes it, the author's version as §2.4
    /// writes a stamp, an archive's document apart and last-known, and
    /// every optional field absent when empty.
    #[test]
    fn the_desired_report_pins_its_shape() {
        use zenkey_model::desired::{Report, Source};
        let mut r = row(
            (DesiredAnswerToken::NotYet, "behind"),
            (DesiredAnswerToken::No, "converged"),
        );
        r.report = Some(DesiredReportSeen {
            key: "zk2/lab/worker/worker.v1/state/desired/task".into(),
            document: Some(Report {
                source: Source::Baseline,
                ..Report::baseline(Some(
                    "zk2/lab/author/tasking.v1/state/tasks/lab/worker".into(),
                ))
            }),
            unreadable: None,
            findings: vec![],
            stamp: Some(Stamp {
                time: "2026-10-24T00:00:00.000000000Z".into(),
                clock: "ab12".into(),
            }),
            freshness: DesiredFreshness {
                verdict: FreshnessToken::NotAsked,
                reason: "no_horizon".into(),
                says: "s".into(),
            },
        });
        r.author = Some(DesiredAuthorSeen {
            address: "lab/author".into(),
            presence: "present".into(),
            current: DesiredCurrentState::Put,
            stamp: Some("200/a1".into()),
            why: None,
        });
        r.archived = vec![DesiredArchived {
            archive: "lab/archive".into(),
            deleted: false,
            stamp: Some("100/a1".into()),
            revision: Some(1),
            confirmed: true,
        }];
        r.behind = Some(DesiredBehind {
            version: "200/a1".into(),
            since_s: 5.0,
            by: DesiredBehindBy::Window,
        });
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(
            v,
            json!({
                "target": "lab/worker", "role": "task", "listing": "bound",
                "interface": "tasking.v1", "resource": "state/tasks/{system}/{service}",
                "slice": "zk2/lab/author/tasking.v1/state/tasks/lab/worker",
                "slice_from": "author_contract",
                "report": {
                    "key": "zk2/lab/worker/worker.v1/state/desired/task",
                    "document": {"key": "zk2/lab/author/tasking.v1/state/tasks/lab/worker",
                                 "source": "baseline"},
                    "stamp": {"time": "2026-10-24T00:00:00.000000000Z", "clock": "ab12"},
                    "freshness": {"verdict": "not_asked", "reason": "no_horizon", "says": "s"},
                },
                "author": {"address": "lab/author", "presence": "present", "current": "put",
                           "stamp": "200/a1"},
                "archived": [{"archive": "lab/archive", "stamp": "100/a1", "revision": 1,
                              "confirmed": true}],
                "answers": {
                    "converged": {"answer": "not_yet", "reason": "behind", "says": "s"},
                    "refused": {"answer": "no", "reason": "converged", "says": "s"},
                    "last_known": {"answer": "no", "reason": "current", "says": "s"},
                    "in_force": {"answer": "yes", "reason": "desired", "says": "s"},
                },
                "behind": {"version": "200/a1", "since_s": 5.0, "by": "window"},
            })
        );
        let v = serde_json::to_value(report(vec![])).unwrap();
        for absent in ["role", "across_face", "window_s", "unobservable"] {
            assert!(v.get(absent).is_none(), "{absent}: {v}");
        }
        assert_eq!(v["clock"], "none");
        assert_eq!(v["readings"], 1);
        for t in [
            DesiredAnswerToken::Yes,
            DesiredAnswerToken::No,
            DesiredAnswerToken::NotYet,
            DesiredAnswerToken::Unobservable,
            DesiredAnswerToken::NotAsked,
        ] {
            assert_eq!(serde_json::to_value(t).unwrap(), json!(t.as_str()));
        }
    }

    /// The run's judgement: a refusal, a refused current version, or a
    /// target behind past the window the 1; every role asked converged
    /// the 0; not yet, unobservable, the author absent, nothing asked, or
    /// a reading not made the 2.
    #[test]
    fn the_judgement_is_the_roles() {
        use DesiredAnswerToken::{No, NotAsked, NotYet, Unobservable, Yes};
        let exit = |r: &DesiredReport| judgement_exit_code(&r.judgement());
        let converged = row((Yes, "converged"), (No, "converged"));
        assert_eq!(exit(&report(vec![converged.clone()])), 0);
        assert_eq!(
            exit(&report(vec![row((No, "refused"), (Yes, "refused"))])),
            1
        );
        assert_eq!(
            exit(&report(vec![row(
                (NotAsked, "author_absent"),
                (Yes, "failed")
            )])),
            1,
            "a refusal stands, its author absent"
        );
        let behind = row((NotYet, "behind"), (No, "converged"));
        assert_eq!(
            exit(&report(vec![behind.clone()])),
            2,
            "not yet is no verdict"
        );
        let mut past = behind;
        past.behind = Some(DesiredBehind {
            version: "200/a1".into(),
            since_s: 5.0,
            by: DesiredBehindBy::Window,
        });
        assert_eq!(exit(&report(vec![past.clone()])), 1);
        assert!(past.settled());
        assert_eq!(
            exit(&report(vec![row(
                (NotAsked, "author_absent"),
                (No, "converged")
            )])),
            2
        );
        assert_eq!(
            exit(&report(vec![row(
                (Unobservable, "author_silent"),
                (No, "converged")
            )])),
            2
        );
        let unbound = row((NotAsked, "unbound"), (NotAsked, "unbound"));
        assert_eq!(exit(&report(vec![unbound.clone()])), 2, "nothing asked");
        assert!(unbound.not_a_question());
        assert_eq!(
            exit(&report(vec![unbound, converged.clone()])),
            0,
            "the kill switch is not asked; the other converged"
        );
        assert_eq!(exit(&report(vec![])), 2);
        let mut cut = report(vec![converged]);
        cut.unobservable = Some("the session reaches no other zenoh node".into());
        assert_eq!(exit(&cut), 2);
        assert!(!cut.settled());
    }
}
