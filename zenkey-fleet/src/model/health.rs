//! `health.v1` (#721, PF): one reading of one service, built from what the
//! bus answered (`spec/profiles/health/v1.md` §0, §2.11).
//!
//! **Values in hand → a reading.** [`reading`] turns a GET's answer, the
//! window's subscription and an archive's word into the
//! [`zenkey_model::health::Reading`] the profile judges:
//!
//! - **presence** from the instance token, and the descriptor's listing of
//!   `health.v1` ([`listing`], [`presence_in`]) — never the interface token,
//!   which plays no part (§2.7); across a constrained face, the deployment's
//!   word (§2.8);
//! - **the status's observations**, judged by `freshness.v1` at the
//!   contract's 60 s: the window's subscription (§2.5) and the GET's reply,
//!   its stamp aged only against a clock trusted to the HLC delta ([`Trust`]:
//!   the deployment's word, or measured on the window's live puts, §2.6),
//!   and an archive's answer, last-known (§2.8);
//! - **the checks** the same GET answered, read together with the status
//!   (§2.4); a GET that answered nothing for the service read none.
//!
//! [`agreement_over`] is §2.2's rule for a tool reporting a break across
//! readings: only when each one shows it. The verdicts themselves are
//! `zenkey_model::health`'s, called through [`ServiceReading`]; which of them
//! is wrong is the judges' to say ([`crate::judge::health`],
//! [`crate::judge::doctor`], [`crate::judge::conform`]).

use std::collections::BTreeMap;
use std::time::SystemTime;

use zenkey_model::freshness::{
    ClockMeasure, ClockTrust, DEFAULT_DELTA, Observation, Reply, StampAge,
};
use zenkey_model::grammar::Addr;
use zenkey_model::health::{
    self as hm, Agreement, Answer, Judged, Level, Listing, Presence, Read, Reading, Reason, Verdict,
};

use crate::bus::health::{HealthGet, Replied, ServiceGot, StatusValue};
use crate::model::catalog::{Catalog, Observed};
use crate::report::{
    ClockGround, HealthAnswer, HealthAnswerToken, HealthLevel, HealthVerdict, LevelRead,
};

/// How a reader trusts its clock against a stamping clock, to age a GET
/// reply's stamp (`freshness.v1` §2.6).
#[derive(Debug, Clone, Copy)]
pub enum Trust<'a> {
    /// The deployment's word (ground 1): every clock, to the HLC delta.
    Word,
    /// Measured on live puts in this reading (ground 2), per clock.
    Measured(&'a BTreeMap<String, ClockMeasure>),
    /// No ground: a reply's age is unobservable.
    None,
}

impl Trust<'_> {
    /// The trust in `clock`, a stamp's id, compared by value.
    pub fn of(&self, clock: &str) -> ClockTrust {
        match self {
            Trust::Word => ClockTrust::Trusted {
                delta: DEFAULT_DELTA,
            },
            Trust::Measured(m) => {
                let want = crate::model::catalog::zid_value(clock);
                m.iter()
                    .find(|(c, _)| crate::model::catalog::zid_value(c) == want)
                    .map_or(ClockTrust::Untrusted, |(_, m)| m.trust(DEFAULT_DELTA))
            }
            Trust::None => ClockTrust::Untrusted,
        }
    }

    /// As the report spells it.
    pub fn ground(&self) -> ClockGround {
        match self {
            Trust::Word => ClockGround::DeploymentWord,
            Trust::Measured(_) => ClockGround::Measured,
            Trust::None => ClockGround::None,
        }
    }
}

/// What one reading's GET answered for one service.
#[derive(Debug, Clone, Copy)]
pub enum Got<'a> {
    /// No GET was made, or it could not be put on the bus.
    NotMade,
    /// The GET was made, and answered nothing for this service: silence.
    Silent { at: SystemTime },
    /// Its answer, judged at `at`, this host's clock as the GET ended.
    Answered { got: &'a ServiceGot, at: SystemTime },
}

impl<'a> Got<'a> {
    /// `addr`'s part of a GET.
    pub fn of(get: Option<&'a HealthGet>, addr: &Addr) -> Got<'a> {
        match get {
            None => Got::NotMade,
            Some(g) => match g.services.get(addr) {
                Some(got) => Got::Answered { got, at: g.read_at },
                None => Got::Silent { at: g.read_at },
            },
        }
    }
}

/// One reading of one service, as values (§0): what [`hm::judge`] takes.
#[derive(Debug, Clone, PartialEq)]
pub struct ServiceReading {
    pub presence: Presence,
    /// The status's observations: the window's subscription first, then the
    /// GET's reply, then an archive's answer.
    pub observations: Vec<Observation>,
    /// The level of the latest status value read.
    pub level: Option<Read>,
    /// The levels of the checks the owner answered with a value; `None`
    /// when the checks were not read.
    pub checks: Option<Vec<Read>>,
}

impl ServiceReading {
    /// §2.11's verdict.
    pub fn judged(&self) -> Judged {
        hm::judge(&Reading {
            presence: self.presence,
            status: &self.observations,
            level: self.level,
            checks: self.checks.as_deref(),
        })
    }

    /// §5's second question, for this reading alone.
    pub fn agreement(&self) -> Agreement {
        hm::agrees(self.judged(), self.checks.is_some())
    }
}

/// One reading of one service from what the bus answered: `subscribed` is
/// the window's observation of the status at this reading's instant and the
/// last status put it heard; `archived` that an archive answered for it.
pub fn reading(
    presence: Presence,
    got: Got<'_>,
    subscribed: Option<(Observation, Option<&Replied<StatusValue>>)>,
    trust: &Trust<'_>,
    archived: bool,
) -> ServiceReading {
    let mut observations = Vec::new();
    let mut level = None;
    if let Some((o, last)) = subscribed {
        observations.push(o);
        level = last.and_then(|r| r.value.as_ref()).map(|v| v.level);
    }
    let mut checks = None;
    match got {
        Got::NotMade => {}
        Got::Silent { .. } => observations.push(Observation::Got {
            reply: None,
            clock: ClockTrust::Untrusted,
        }),
        Got::Answered { got, at } => {
            observations.push(match &got.status {
                None => Observation::Got {
                    reply: None,
                    clock: ClockTrust::Untrusted,
                },
                Some(r) => Observation::Got {
                    reply: Some(match &r.value {
                        None => Reply::Delete,
                        Some(_) => Reply::Put {
                            stamp_age: r.stamp.as_ref().map(|s| StampAge::between(s.time, at)),
                        },
                    }),
                    clock: r
                        .stamp
                        .as_ref()
                        .map_or(ClockTrust::Untrusted, |s| trust.of(&s.clock)),
                },
            });
            // The status and its checks, read together (§2.4): the GET's
            // level stands over the window's, which may be older.
            if let Some(v) = got.status.as_ref().and_then(|r| r.value.as_ref()) {
                level = Some(v.level);
            }
            checks = Some(
                got.checks
                    .values()
                    .filter_map(|c| c.value.as_ref().map(|v| v.level))
                    .collect(),
            );
        }
    }
    if archived {
        observations.push(Observation::Archived);
    }
    ServiceReading {
        presence,
        observations,
        level,
        checks,
    }
}

/// §5's second question over every reading, the first first: *no* only when
/// each reading says no — a break of §2.2 is a finding about the owner from
/// two readings a grace apart, since one reading can pair values never
/// current together; *no* in the last reading alone is unobservable;
/// otherwise the last reading's answer.
pub fn agreement_over(readings: &[Agreement]) -> HealthAnswer {
    let Some(last) = readings.last() else {
        return answer(
            HealthAnswerToken::NotAsked,
            "no_reading",
            "no reading was taken",
        );
    };
    match last.answer {
        Answer::No if readings.len() >= 2 && readings.iter().all(|a| a.answer == Answer::No) => {
            answer(
                HealthAnswerToken::No,
                Reason::Inconsistent.as_str(),
                &format!(
                    "a current check is worse than its fresh status in each of {} readings: its \
                     owner breaks health.v1 §2.2",
                    readings.len()
                ),
            )
        }
        Answer::No => answer(
            HealthAnswerToken::Unobservable,
            "one_reading",
            "a current check is worse than its status in the last reading only: a break of \
             §2.2 is reported from two readings a grace apart",
        ),
        Answer::Yes => answer(
            HealthAnswerToken::Yes,
            last.reason.as_str(),
            "its status is fresh at a level, and no current check is worse",
        ),
        Answer::NotAsked => answer(
            HealthAnswerToken::NotAsked,
            last.reason.as_str(),
            last.reason.says(),
        ),
        Answer::Unobservable => answer(
            HealthAnswerToken::Unobservable,
            last.reason.as_str(),
            last.reason.says(),
        ),
    }
}

/// A §5 answer as the report spells it.
pub fn answer(answer: HealthAnswerToken, reason: &str, says: &str) -> HealthAnswer {
    HealthAnswer {
        answer,
        reason: reason.to_owned(),
        says: says.to_owned(),
    }
}

/// The answer's token.
pub fn token(a: Answer) -> HealthAnswerToken {
    match a {
        Answer::Yes => HealthAnswerToken::Yes,
        Answer::No => HealthAnswerToken::No,
        Answer::Unobservable => HealthAnswerToken::Unobservable,
        Answer::NotAsked => HealthAnswerToken::NotAsked,
    }
}

/// The verdict's token.
pub fn verdict_token(v: Verdict) -> HealthVerdict {
    match v {
        Verdict::Healthy => HealthVerdict::Healthy,
        Verdict::Unhealthy => HealthVerdict::Unhealthy,
        Verdict::Stale => HealthVerdict::Stale,
        Verdict::Unobservable => HealthVerdict::Unobservable,
        Verdict::NotAsked => HealthVerdict::NotAsked,
    }
}

/// The level's token.
pub fn level_token(l: Level) -> HealthLevel {
    match l {
        Level::Ok => HealthLevel::Ok,
        Level::Degraded => HealthLevel::Degraded,
        Level::Failed => HealthLevel::Failed,
    }
}

/// A level as a payload carried it (§2.6).
pub fn level_read(r: Read) -> LevelRead {
    match r {
        Read::Level(l) => LevelRead::Token(l.as_str()),
        Read::Unspecified => LevelRead::Token("unspecified"),
        Read::Undecodable => LevelRead::Token("undecodable"),
        Read::Unlisted(n) => LevelRead::Unlisted(n),
    }
}

/// What `addr`'s descriptors say of `health.v1` (§2.7): listed by any
/// served descriptor, with its token flag; else unread when an instance's
/// descriptor did not read, or descriptors were not asked; else not listed.
pub fn listing(o: &Observed, addr: &Addr) -> Listing {
    let Some(descriptors) = &o.descriptors else {
        return Listing::Unread;
    };
    let mut unread = false;
    for ((a, _), read) in descriptors {
        if a != addr {
            continue;
        }
        match read.descriptor() {
            Some(d) => {
                if let Some(e) = d.interfaces.iter().find(|e| e.iface == hm::IFACE) {
                    return Listing::Listed { token: e.token };
                }
            }
            None => unread = true,
        }
    }
    if unread {
        Listing::Unread
    } else {
        Listing::NotListed
    }
}

/// `addr`'s presence in one read (core §8.1): its instance token seen, with
/// what its descriptors say; else absent from a complete read, possibly
/// incomplete otherwise.
pub fn presence_in(o: &Observed, addr: &Addr) -> Presence {
    if Catalog::new(o).has_instance(addr) {
        Presence::Present(listing(o, addr))
    } else if o.complete {
        Presence::Absent
    } else {
        Presence::Incomplete
    }
}
