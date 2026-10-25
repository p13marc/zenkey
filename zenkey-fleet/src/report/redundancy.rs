//! `redundancy.v1` (#841, RC): who serves a governed interface, as
//! `zenctl redundancy` reads it (`spec/profiles/redundancy/v1.md` §2.15,
//! §5).
//!
//! **The profile's own vocabulary, then the judgement shape.** Each row is
//! one address and governed interface, read twice a grace apart, and
//! carries §5's four questions in the tokens `conformance/conclusions.json`
//! states, so a script reads `split` where the text says split:
//! - "is the address served by exactly one incumbent?" — `one_incumbent`,
//!   whose *no* (`split`, or `vacant` in both reads) is the finding;
//! - "is there a vacancy?" — `vacancy`, whose *yes* `lasting` is the
//!   finding, and `vacant` a takeover due;
//! - "does any instance stand by, and how many?" — `standbys`, informative,
//!   with its count, exact or a lower bound;
//! - "is this instance a standby, or a twin?" — each instance's `twin`,
//!   whose *yes* is the finding, and the [`RedundancyRole`] it reads as.
//!
//! [`RedundancyReport::judgement`] folds them into the one [`Judgement`] a
//! frontend exits through. Every answer is `zenkey_model::redundancy`'s,
//! never re-derived (`crate::judge::redundancy`).

use serde::Serialize;

use super::judgement::Judgement;

/// One of §5's answers, in the fixture's tokens (§2.15): `yes`, `no`,
/// `unobservable`, `not_asked`, `not_this_profiles`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RedundancyAnswerToken {
    Yes,
    No,
    Unobservable,
    NotAsked,
    NotThisProfiles,
}

impl RedundancyAnswerToken {
    /// The wire token, exactly as it serializes.
    pub fn as_str(self) -> &'static str {
        match self {
            RedundancyAnswerToken::Yes => "yes",
            RedundancyAnswerToken::No => "no",
            RedundancyAnswerToken::Unobservable => "unobservable",
            RedundancyAnswerToken::NotAsked => "not_asked",
            RedundancyAnswerToken::NotThisProfiles => "not_this_profiles",
        }
    }
}

/// One question answered: the answer, its reason in `conclusions.json`'s
/// tokens, and the reason in words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RedundancyAnswer {
    pub answer: RedundancyAnswerToken,
    pub reason: String,
    pub says: String,
}

/// "Does any instance stand by, and how many?" (§2.15): the answer, and
/// the count, exact or a lower bound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RedundancyStandbys {
    pub answer: RedundancyAnswerToken,
    pub reason: String,
    pub says: String,
    pub count: usize,
    pub exact: bool,
}

/// What an instance reads as, from its fourth question (§2.15): the
/// incumbent (`no`, `incumbent`), a standby (`no`, `standby`), a twin (any
/// `yes`), not a candidate (`not_asked`, `not_a_candidate` or `absent`), or
/// unknown (unobservable, or not this profile's).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RedundancyRole {
    Incumbent,
    Standby,
    Twin,
    NotACandidate,
    Unknown,
}

impl RedundancyRole {
    /// The wire token, exactly as it serializes.
    pub fn as_str(self) -> &'static str {
        match self {
            RedundancyRole::Incumbent => "incumbent",
            RedundancyRole::Standby => "standby",
            RedundancyRole::Twin => "twin",
            RedundancyRole::NotACandidate => "not_a_candidate",
            RedundancyRole::Unknown => "unknown",
        }
    }
}

/// What a read showed of one instance (§2.15): its token of the interface,
/// and its descriptor's listing of it (`unread`, `not_listed`, `standby`,
/// `serving`), as `elections.json` writes them; absent where it was not in
/// that read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RedundancySeen {
    pub token: bool,
    pub listing: String,
}

/// One instance of the address, as the two reads showed it, and its
/// fourth question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RedundancyInstance {
    pub instance: String,
    /// Its session's zid, as its descriptor states `meta.zid`, when read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<RedundancySeen>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub second: Option<RedundancySeen>,
    pub twin: RedundancyAnswer,
    pub role: RedundancyRole,
}

/// One address and governed interface (§2.15).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RedundancyRow {
    pub address: String,
    pub iface: String,
    /// The governance its candidates' revisions give (§2.2): `claim`,
    /// `external`, `not_governed`, `no_contract` or `disagree`.
    pub governance: String,
    /// Why, in words: the revisions read, or the one missing.
    pub governance_says: String,
    /// The holders of the interface's token in the second read, lowest
    /// first.
    pub holders: Vec<String>,
    pub one_incumbent: RedundancyAnswer,
    pub vacancy: RedundancyAnswer,
    pub standbys: RedundancyStandbys,
    pub instances: Vec<RedundancyInstance>,
}

impl RedundancyRow {
    /// Whether the row holds a finding (§5's "Finding on"): two holders in
    /// both reads (`split`), a vacancy in both (`vacant`, `lasting`), or an
    /// instance that is a twin.
    pub fn is_finding(&self) -> bool {
        self.one_incumbent.answer == RedundancyAnswerToken::No
            || (self.vacancy.answer == RedundancyAnswerToken::Yes
                && self.vacancy.reason == "lasting")
            || self
                .instances
                .iter()
                .any(|i| i.twin.answer == RedundancyAnswerToken::Yes)
    }

    /// Whether the row is clean: exactly one incumbent, and every instance
    /// accounted for (the incumbent, a standby, or not a candidate).
    pub fn is_clean(&self) -> bool {
        self.one_incumbent.answer == RedundancyAnswerToken::Yes
            && self.instances.iter().all(|i| {
                matches!(
                    i.role,
                    RedundancyRole::Incumbent
                        | RedundancyRole::Standby
                        | RedundancyRole::NotACandidate
                )
            })
    }
}

/// One of the two reads (§2.15): complete or not, and why not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RedundancyRead {
    pub complete: bool,
    /// Why it is not complete, or why it was not made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The instances it found in scope.
    pub instances: usize,
}

/// `zenctl redundancy` (#841, RC): every governed interface in one
/// namespace, or one address's, read as §2.15's tool reads it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RedundancyReport {
    /// The deployment's namespace; empty for the bus root.
    pub namespace: String,
    /// The one address asked, `<system>/<service>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    /// The one interface asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iface: Option<String>,
    /// How far apart the two reads were taken, seconds (core §6).
    pub apart_s: f64,
    /// Across a constrained face, on the deployment's word: every answer is
    /// `across_face`, and nothing is read (§2.13).
    pub across_face: bool,
    /// The selectors put to the bus, base-relative in the namespace.
    pub asked: Vec<String>,
    /// The two reads, the first first; empty when none was made.
    pub reads: Vec<RedundancyRead>,
    pub interfaces: Vec<RedundancyRow>,
    /// Set when nothing could be read or nothing in scope is governed, with
    /// why. The run is then no verdict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unobservable: Option<String>,
}

impl RedundancyReport {
    /// The run as one [`Judgement`]: a split, a vacancy in both reads, or a
    /// twin is the finding (exit 1); every governed interface asked served
    /// by exactly one incumbent, every instance accounted for, is clean
    /// (exit 0); anything else is no verdict (exit 2): an answer
    /// unobservable, an interface not this profile's, nothing governed in
    /// scope, or a read that could not be made.
    pub fn judgement(&self) -> Judgement {
        if self.interfaces.iter().any(RedundancyRow::is_finding) {
            return Judgement::Established;
        }
        if let Some(why) = &self.unobservable {
            return Judgement::Unobservable {
                reason: why.clone(),
            };
        }
        let governed: Vec<&RedundancyRow> = self
            .interfaces
            .iter()
            .filter(|r| r.one_incumbent.answer != RedundancyAnswerToken::NotThisProfiles)
            .collect();
        if governed.is_empty() {
            return Judgement::Unobservable {
                reason: if self.interfaces.is_empty() {
                    "no interface in scope to ask".to_owned()
                } else {
                    format!(
                        "none of the {} interface(s) asked is governed by redundancy.v1: not \
                         this profile's",
                        self.interfaces.len()
                    )
                },
            };
        }
        let unseen: Vec<String> = governed
            .iter()
            .filter(|r| !r.is_clean())
            .map(|r| {
                format!(
                    "{} {} ({}: {})",
                    r.address, r.iface, r.one_incumbent.reason, r.one_incumbent.says
                )
            })
            .collect();
        if !unseen.is_empty() {
            return Judgement::Unobservable {
                reason: format!(
                    "no finding, and {} governed interface(s) not established: {}",
                    unseen.len(),
                    unseen.join("; ")
                ),
            };
        }
        Judgement::NotEstablished {
            reason: format!(
                "{} governed interface(s), each served by exactly one incumbent, every instance \
                 the incumbent, a standby or no candidate",
                governed.len()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::judgement_exit_code;
    use serde_json::json;

    fn answer(a: RedundancyAnswerToken, reason: &str) -> RedundancyAnswer {
        RedundancyAnswer {
            answer: a,
            reason: reason.into(),
            says: "s".into(),
        }
    }

    fn row(
        one: (RedundancyAnswerToken, &str),
        twin: (RedundancyAnswerToken, &str, RedundancyRole),
    ) -> RedundancyRow {
        RedundancyRow {
            address: "lab/ledger".into(),
            iface: "ledger.v1".into(),
            governance: "claim".into(),
            governance_says: "s".into(),
            holders: vec!["000000000000000a".into()],
            one_incumbent: answer(one.0, one.1),
            vacancy: answer(RedundancyAnswerToken::No, "held"),
            standbys: RedundancyStandbys {
                answer: RedundancyAnswerToken::Yes,
                reason: "standing_by".into(),
                says: "s".into(),
                count: 1,
                exact: true,
            },
            instances: vec![RedundancyInstance {
                instance: "000000000000000b".into(),
                zid: None,
                first: Some(RedundancySeen {
                    token: false,
                    listing: "standby".into(),
                }),
                second: None,
                twin: answer(twin.0, twin.1),
                role: twin.2,
            }],
        }
    }

    fn report(rows: Vec<RedundancyRow>) -> RedundancyReport {
        RedundancyReport {
            namespace: String::new(),
            service: None,
            iface: None,
            apart_s: 2.0,
            across_face: false,
            asked: vec![],
            reads: vec![],
            interfaces: rows,
            unobservable: None,
        }
    }

    /// The exit contract: a split or a twin is 1, one incumbent with every
    /// instance accounted for 0, the rest 2.
    #[test]
    fn the_exit_contract() {
        use RedundancyAnswerToken as A;
        let clean = row((A::Yes, "one"), (A::No, "standby", RedundancyRole::Standby));
        assert_eq!(
            judgement_exit_code(&report(vec![clean.clone()]).judgement()),
            0
        );
        let split = row((A::No, "split"), (A::Yes, "split", RedundancyRole::Twin));
        assert_eq!(
            judgement_exit_code(&report(vec![clean.clone(), split]).judgement()),
            1
        );
        let twin = row((A::Yes, "one"), (A::Yes, "unmarked", RedundancyRole::Twin));
        assert_eq!(judgement_exit_code(&report(vec![twin]).judgement()), 1);
        let vacant = row(
            (A::No, "vacant"),
            (A::No, "standby", RedundancyRole::Standby),
        );
        assert_eq!(judgement_exit_code(&report(vec![vacant]).judgement()), 1);
        let unseen = row(
            (A::Unobservable, "presence_incomplete"),
            (
                A::Unobservable,
                "presence_incomplete",
                RedundancyRole::Unknown,
            ),
        );
        assert_eq!(
            judgement_exit_code(&report(vec![clean, unseen]).judgement()),
            2
        );
        let foreign = row(
            (A::NotThisProfiles, "not_governed"),
            (A::NotThisProfiles, "not_governed", RedundancyRole::Unknown),
        );
        assert_eq!(judgement_exit_code(&report(vec![foreign]).judgement()), 2);
        assert_eq!(judgement_exit_code(&report(vec![]).judgement()), 2);
    }

    /// The pinned shape: tokens as the fixture spells them.
    #[test]
    fn the_shape_is_pinned() {
        use RedundancyAnswerToken as A;
        let r = report(vec![row(
            (A::Yes, "one"),
            (A::No, "standby", RedundancyRole::Standby),
        )]);
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            json!({
                "namespace": "",
                "apart_s": 2.0,
                "across_face": false,
                "asked": [],
                "reads": [],
                "interfaces": [{
                    "address": "lab/ledger",
                    "iface": "ledger.v1",
                    "governance": "claim",
                    "governance_says": "s",
                    "holders": ["000000000000000a"],
                    "one_incumbent": {"answer": "yes", "reason": "one", "says": "s"},
                    "vacancy": {"answer": "no", "reason": "held", "says": "s"},
                    "standbys": {"answer": "yes", "reason": "standing_by", "says": "s",
                                 "count": 1, "exact": true},
                    "instances": [{
                        "instance": "000000000000000b",
                        "first": {"token": false, "listing": "standby"},
                        "twin": {"answer": "no", "reason": "standby", "says": "s"},
                        "role": "standby"
                    }]
                }]
            })
        );
    }
}
