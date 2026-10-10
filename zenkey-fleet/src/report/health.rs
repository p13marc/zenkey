//! `health.v1` (#721, PF): how well each service serves, as `zenctl health`
//! reads it (`spec/profiles/health/v1.md`).
//!
//! **The profile's own vocabulary, then the judgement shape.** Each service
//! row carries §5's four questions in the tokens the profile's fixtures
//! state, so a script reads `healthy` where the text says healthy:
//! - "is this service healthy?" — [`HealthVerdict`] (`healthy`,
//!   `unhealthy`, `stale`, `unobservable`, `not_asked`) with its reason
//!   class and, for the two established verdicts, the [`HealthLevel`] it
//!   rests on (§2.11). Unhealthy and stale are findings; stale is never a
//!   level, and never `failed` or down (§2.4, §2.8);
//! - "does its status agree with its checks?" — a [`HealthAnswer`] whose
//!   *no* is the finding, and only when both readings say it (§2.2);
//! - "is its clock ahead?" — a [`HealthAnswer`] whose *yes* is the finding,
//!   answered from what the window's subscription heard (§2.5);
//! - "how healthy are these services?" — the [`HealthRollup`], the worst
//!   established level with every verdict counted apart (§2.2).
//!
//! [`HealthReport::judgement`] folds them into the one [`Judgement`] a
//! frontend exits through: the finding is the yes of "is a service
//! unhealthy, stale, breaking §2.2 or running its clock ahead?".
//!
//! **What was read is shown as read.** A status's level is the payload's
//! ([`LevelRead`]: a level, `unspecified`, a number the enum does not list,
//! `undecodable`), with its stamp and how it reached this reader. An
//! archive's status for an absent owner is [`LastKnownStatus`]: last-known,
//! never current, and never part of a verdict (core S6).

use serde::Serialize;

use super::judgement::Judgement;
use super::state::Stamp;

/// "Is this service healthy?" (§2.11, §5): the profile's verdict tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthVerdict {
    Healthy,
    /// A finding.
    Unhealthy,
    /// A finding of its own: the status was not confirmed within its
    /// horizon. Never a level (§2.4).
    Stale,
    Unobservable,
    NotAsked,
}

impl HealthVerdict {
    /// The wire token, exactly as it serializes.
    pub fn as_str(self) -> &'static str {
        match self {
            HealthVerdict::Healthy => "healthy",
            HealthVerdict::Unhealthy => "unhealthy",
            HealthVerdict::Stale => "stale",
            HealthVerdict::Unobservable => "unobservable",
            HealthVerdict::NotAsked => "not_asked",
        }
    }

    /// Whether the verdict is a finding (§5's polarity).
    pub fn is_finding(self) -> bool {
        matches!(self, HealthVerdict::Unhealthy | HealthVerdict::Stale)
    }
}

/// A level (§2.1), in the fixtures' tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthLevel {
    Ok,
    Degraded,
    Failed,
}

impl HealthLevel {
    /// The wire token, exactly as it serializes.
    pub fn as_str(self) -> &'static str {
        match self {
            HealthLevel::Ok => "ok",
            HealthLevel::Degraded => "degraded",
            HealthLevel::Failed => "failed",
        }
    }
}

/// A level as a payload carried it (§2.6): a level's token, `unspecified`,
/// `undecodable`, or the number of a level the enum does not list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum LevelRead {
    /// `ok`, `degraded`, `failed`, `unspecified` or `undecodable`.
    Token(&'static str),
    /// A number the enum does not list: unknown, never `ok`.
    Unlisted(i32),
}

/// A yes-or-no question of §5, answered in the three states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthAnswerToken {
    Yes,
    No,
    Unobservable,
    NotAsked,
}

/// One of §5's yes-or-no questions answered for one service: the answer,
/// its reason class, and the reason in words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HealthAnswer {
    pub answer: HealthAnswerToken,
    pub reason: String,
    pub says: String,
}

/// One reading's answer to "is this service healthy?" (§2.11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HealthReadingVerdict {
    pub verdict: HealthVerdict,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<HealthLevel>,
}

/// How a status reached this reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusVia {
    /// The owner's reply to the reading's GET (core S4), the checks beside
    /// it in the same answer (§2.4).
    Get,
    /// The window's subscription: the last put it heard.
    Subscription,
}

/// The status as the last reading read it, its level as the payload
/// carried it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatusSeen {
    pub level: LevelRead,
    /// Why the level is what it is, as the owner wrote it.
    pub reason: String,
    /// When the level last changed, on the owner's clock: shown, never
    /// aged by (§2.3).
    pub since_ns: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stamp: Option<Stamp>,
    pub via: StatusVia,
}

/// One current check the owner answered (§2.4): current only while the
/// status vouching for it is fresh.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckSeen {
    /// Its name: the `{check}` chunk of its key.
    pub check: String,
    pub level: LevelRead,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

/// An archive's status for an owner presence shows absent: last-known,
/// never current (core S6, `freshness.v1` §2.8). It plays no part in a
/// verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LastKnownStatus {
    /// The archive, `<system>/<service>`.
    pub archive: String,
    pub level: LevelRead,
    pub reason: String,
    pub since_ns: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stamp: Option<Stamp>,
    /// Whether the archive's alignment confirmed the value (core §4.4).
    pub confirmed: bool,
}

/// The last fault the window heard (§2.3, §2.10): an occurrence, shown as
/// written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FaultSeen {
    pub code: String,
    /// `profile`, `application` or `malformed` (§2.10).
    pub class: &'static str,
    pub level: LevelRead,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

/// One service's health, by its readings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HealthRow {
    /// `<system>/<service>`.
    pub address: String,
    /// "Is this service healthy?", by the last reading.
    pub verdict: HealthVerdict,
    /// Its reason class (§2.11): the fixtures' token.
    pub reason: String,
    /// For healthy and unhealthy, the level the verdict rests on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<HealthLevel>,
    /// The reason in words.
    pub says: String,
    /// Each reading's verdict, the first first.
    pub readings: Vec<HealthReadingVerdict>,
    /// "Does its status agree with its checks?" over the readings (§2.2).
    pub agrees: HealthAnswer,
    /// "Is its clock ahead?", from the window's subscription (§2.5).
    pub clock_ahead: HealthAnswer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<StatusSeen>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<CheckSeen>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_known: Option<LastKnownStatus>,
    /// Faults the window heard; absent at zero, and when no window ran.
    #[serde(default, skip_serializing_if = "super::asked::u64_is_zero")]
    pub faults: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fault: Option<FaultSeen>,
}

impl HealthRow {
    /// Whether the row holds a finding: unhealthy or stale, a break of §2.2
    /// seen in both readings, or a clock ahead.
    pub fn is_finding(&self) -> bool {
        self.verdict.is_finding()
            || self.agrees.answer == HealthAnswerToken::No
            || self.clock_ahead.answer == HealthAnswerToken::Yes
    }
}

/// A tool's roll-up of many services (§2.2): the worst level among the
/// established verdicts, `null` when none is, and every verdict counted.
/// Stale, unobservable and not asked are counted apart, never folded into a
/// level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct HealthRollup {
    pub worst: Option<HealthLevel>,
    pub healthy: usize,
    pub unhealthy: usize,
    pub stale: usize,
    pub unobservable: usize,
    pub not_asked: usize,
}

/// How this reader trusted its clock against the owners', to age a GET
/// reply's stamp (`freshness.v1` §2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockGround {
    /// The deployment's word, `--clocks-synced` (ground 1).
    DeploymentWord,
    /// Measured on the window's live puts, per stamping clock (ground 2).
    Measured,
    /// Neither: no window ran and no word was given, so a reply's age is
    /// unobservable.
    None,
}

/// Where the services were looked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HealthPresence {
    /// The liveliness selector, base-relative in the namespace.
    pub selector: String,
    /// Whether the read ended at the routers' final reply (core §8.1).
    pub complete: bool,
    /// Services holding an instance token in it.
    pub services: usize,
}

/// A service across a constrained face (§2.8): presence does not cross, and
/// the deployment's word stands for its descriptor (core R7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct HealthFace {
    /// Whether the face lets `state/status` cross.
    pub status_crosses: bool,
}

/// `zenctl health` (#721, PF): every service's health in one namespace, or
/// one service's.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HealthReport {
    /// The deployment's namespace; empty for the bus root.
    pub namespace: String,
    /// The one service asked, `<system>/<service>`; absent when every
    /// service presence shows was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    /// Presence as read; absent across a face, where it does not cross.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence: Option<HealthPresence>,
    /// The deployment's word for a service across a constrained face.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face: Option<HealthFace>,
    /// The window's length, seconds: how long a subscription to every
    /// status and to the faults listened. Absent when none did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_s: Option<f64>,
    /// How far apart the two readings were taken, seconds (§2.2).
    pub apart_s: f64,
    pub clock: ClockGround,
    /// The selectors put to the bus, base-relative in the namespace: what
    /// was read, and no wider (tooling guide O5).
    pub asked: Vec<String>,
    pub services: Vec<HealthRow>,
    pub rollup: HealthRollup,
    /// Set when nothing in scope could be read — no service visible, a
    /// presence read or a GET that could not be made — with why. The run is
    /// then no verdict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unobservable: Option<String>,
}

impl HealthReport {
    /// The run as one [`Judgement`]: a service unhealthy or stale, a break
    /// of §2.2 seen in both readings, or a clock ahead, is the finding (exit
    /// 1); without one, an unobservable service, an unobservable scope, or a
    /// run that asked no service is no verdict (exit 2); every service asked
    /// healthy is clean (exit 0). A service not asked counts for neither.
    pub fn judgement(&self) -> Judgement {
        let findings: Vec<&str> = self
            .services
            .iter()
            .filter(|r| r.is_finding())
            .map(|r| r.address.as_str())
            .collect();
        if !findings.is_empty() {
            return Judgement::Established;
        }
        if let Some(why) = &self.unobservable {
            return Judgement::Unobservable {
                reason: why.clone(),
            };
        }
        let asked: Vec<&HealthRow> = self
            .services
            .iter()
            .filter(|r| r.verdict != HealthVerdict::NotAsked)
            .collect();
        if asked.is_empty() {
            return Judgement::Unobservable {
                reason: if self.services.is_empty() {
                    "no service to ask".to_owned()
                } else {
                    format!(
                        "health was asked of none of the {} service(s) read: not asked is \
                         neither healthy nor a finding",
                        self.services.len()
                    )
                },
            };
        }
        let unseen: Vec<String> = asked
            .iter()
            .filter(|r| r.verdict == HealthVerdict::Unobservable)
            .map(|r| format!("{} ({})", r.address, r.reason))
            .collect();
        if !unseen.is_empty() {
            return Judgement::Unobservable {
                reason: format!(
                    "no finding, and {} service(s) could not be read: {}",
                    unseen.len(),
                    unseen.join(", ")
                ),
            };
        }
        Judgement::NotEstablished {
            reason: format!(
                "{} service(s) asked, every one healthy, its status fresh and agreeing with its \
                 checks",
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

    fn answer(answer: HealthAnswerToken, reason: &str) -> HealthAnswer {
        HealthAnswer {
            answer,
            reason: reason.into(),
            says: "s".into(),
        }
    }

    fn row(address: &str, verdict: HealthVerdict, reason: &str) -> HealthRow {
        let level = match verdict {
            HealthVerdict::Healthy => Some(HealthLevel::Ok),
            HealthVerdict::Unhealthy => Some(HealthLevel::Failed),
            _ => None,
        };
        HealthRow {
            address: address.into(),
            verdict,
            reason: reason.into(),
            level,
            says: "s".into(),
            readings: vec![HealthReadingVerdict {
                verdict,
                reason: reason.into(),
                level,
            }],
            agrees: answer(HealthAnswerToken::Yes, "ok"),
            clock_ahead: answer(HealthAnswerToken::NotAsked, "no_window"),
            status: None,
            checks: vec![],
            last_known: None,
            faults: 0,
            last_fault: None,
        }
    }

    fn report(services: Vec<HealthRow>) -> HealthReport {
        HealthReport {
            namespace: "acme".into(),
            service: None,
            presence: Some(HealthPresence {
                selector: "zk2/*/*/@zk/**".into(),
                complete: true,
                services: services.len(),
            }),
            face: None,
            window_s: None,
            apart_s: 2.0,
            clock: ClockGround::DeploymentWord,
            asked: vec!["zk2/*/*/health.v1/state/**".into()],
            services,
            rollup: HealthRollup::default(),
            unobservable: None,
        }
    }

    /// The report's wire shape: the profile's tokens on each row, a level
    /// as the payload carried it (a token or a number), the archive's
    /// status apart and last-known, and every optional field absent when
    /// empty — the roll-up's `worst` alone is `null`, as the fixture writes
    /// it.
    #[test]
    fn the_health_report_pins_its_shape() {
        let mut liar = row("lab/liar", HealthVerdict::Unhealthy, "inconsistent");
        liar.agrees = answer(HealthAnswerToken::No, "inconsistent");
        liar.status = Some(StatusSeen {
            level: LevelRead::Token("ok"),
            reason: "serving".into(),
            since_ns: 7,
            stamp: Some(Stamp {
                time: "2026-10-10T00:00:00.000000000Z".into(),
                clock: "ab12".into(),
            }),
            via: StatusVia::Get,
        });
        liar.checks = vec![
            CheckSeen {
                check: "disk".into(),
                level: LevelRead::Token("failed"),
                detail: "full".into(),
            },
            CheckSeen {
                check: "net".into(),
                level: LevelRead::Unlisted(9),
                detail: String::new(),
            },
        ];
        let mut gone = row("lab/svc", HealthVerdict::NotAsked, "absent");
        gone.agrees = answer(HealthAnswerToken::NotAsked, "absent");
        gone.last_known = Some(LastKnownStatus {
            archive: "lab/archive".into(),
            level: LevelRead::Token("degraded"),
            reason: "upstream lost".into(),
            since_ns: 5,
            stamp: None,
            confirmed: true,
        });
        let mut ahead = row("lab/ahead", HealthVerdict::Stale, "beyond_horizon");
        ahead.clock_ahead = answer(HealthAnswerToken::Yes, "clock_ahead");
        ahead.faults = 2;
        ahead.last_fault = Some(FaultSeen {
            code: "clock_ahead".into(),
            class: "profile",
            level: LevelRead::Token("failed"),
            detail: String::new(),
        });
        let r = HealthReport {
            window_s: Some(31.0),
            clock: ClockGround::Measured,
            rollup: HealthRollup {
                worst: Some(HealthLevel::Failed),
                unhealthy: 1,
                stale: 1,
                not_asked: 1,
                ..HealthRollup::default()
            },
            ..report(vec![liar, gone, ahead])
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["clock"], "measured");
        assert_eq!(v["window_s"], 31.0);
        assert_eq!(
            v["rollup"],
            json!({"worst": "failed", "healthy": 0, "unhealthy": 1, "stale": 1,
                   "unobservable": 0, "not_asked": 1})
        );
        assert!(v.get("face").is_none() && v.get("service").is_none());
        assert_eq!(
            v["services"][0],
            json!({
                "address": "lab/liar",
                "verdict": "unhealthy",
                "reason": "inconsistent",
                "level": "failed",
                "says": "s",
                "readings": [{"verdict": "unhealthy", "reason": "inconsistent", "level": "failed"}],
                "agrees": {"answer": "no", "reason": "inconsistent", "says": "s"},
                "clock_ahead": {"answer": "not_asked", "reason": "no_window", "says": "s"},
                "status": {
                    "level": "ok", "reason": "serving", "since_ns": 7,
                    "stamp": {"time": "2026-10-10T00:00:00.000000000Z", "clock": "ab12"},
                    "via": "get",
                },
                "checks": [
                    {"check": "disk", "level": "failed", "detail": "full"},
                    {"check": "net", "level": 9},
                ],
            })
        );
        assert_eq!(
            v["services"][1]["last_known"],
            json!({"archive": "lab/archive", "level": "degraded", "reason": "upstream lost",
                   "since_ns": 5, "confirmed": true})
        );
        assert!(
            v["services"][1].get("level").is_none(),
            "not asked rests on no level"
        );
        assert_eq!(v["services"][2]["faults"], 2);
        assert_eq!(
            v["services"][2]["last_fault"],
            json!({"code": "clock_ahead", "class": "profile", "level": "failed"})
        );
        let empty = HealthReport {
            presence: None,
            face: Some(HealthFace {
                status_crosses: false,
            }),
            service: Some("vehicle-01/nav".into()),
            ..report(vec![])
        };
        let v = serde_json::to_value(&empty).unwrap();
        assert_eq!(v["face"], json!({"status_crosses": false}));
        assert_eq!(v["rollup"]["worst"], serde_json::Value::Null);
        assert!(v.get("presence").is_none());
    }

    /// The run's judgement: a finding on any row is the 1, whatever else;
    /// without one, an unobservable row or scope is the 2, and so is a run
    /// that asked no service; every row asked healthy is the 0.
    #[test]
    fn the_judgement_reads_findings_then_the_unobservable() {
        let exit = |r: &HealthReport| judgement_exit_code(&r.judgement());
        let healthy = row("a/ok", HealthVerdict::Healthy, "ok");
        let unseen = row("a/u", HealthVerdict::Unobservable, "clock_untrusted");
        let skip = row("a/n", HealthVerdict::NotAsked, "not_listed");
        assert_eq!(exit(&report(vec![healthy.clone(), skip.clone()])), 0);
        assert_eq!(exit(&report(vec![healthy.clone(), unseen.clone()])), 2);
        assert_eq!(exit(&report(vec![skip.clone()])), 2, "nothing asked");
        assert_eq!(exit(&report(vec![])), 2, "no service");
        let stale = row("a/s", HealthVerdict::Stale, "beyond_horizon");
        assert_eq!(exit(&report(vec![stale, unseen.clone()])), 1);
        let mut liar = healthy.clone();
        liar.agrees = answer(HealthAnswerToken::No, "inconsistent");
        assert_eq!(exit(&report(vec![liar])), 1, "a break in both readings");
        let mut ahead = healthy.clone();
        ahead.clock_ahead = answer(HealthAnswerToken::Yes, "clock_ahead");
        assert_eq!(exit(&report(vec![ahead])), 1);
        let whole = HealthReport {
            unobservable: Some("no zk2 token visible".into()),
            ..report(vec![healthy])
        };
        assert_eq!(exit(&whole), 2);
    }
}
