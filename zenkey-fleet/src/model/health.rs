//! `health.v1` (#721, PF) and `health.v2` (#986's Q2): one reading of one
//! service, built from what the bus answered (`spec/profiles/health/v1.md`
//! §0, §2.11; `v2.md` §2.11). Each service is read by one major, the one
//! its descriptor lists, `health.v2` where it lists both ([`major_of`]);
//! the steps, the verdicts and the reasons are one set for both (v2
//! §2.12).
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
//! readings: only when each one shows it; under `health.v2`, also from one
//! reading that is one instant ([`agreement_over_v2`], v2 §2.2). The verdicts themselves are
//! `zenkey_model::health`'s, called through [`ServiceReading`]; which of them
//! is wrong is the judges' to say ([`crate::judge::health`],
//! [`crate::judge::doctor`], [`crate::judge::conform`]).

use std::collections::BTreeMap;
use std::time::SystemTime;

use zenkey_model::freshness::{
    self, ClockMeasure, ClockTrust, DEFAULT_DELTA, Horizon, Observation, Reply, StampAge,
};
use zenkey_model::grammar::Addr;
use zenkey_model::health::v2 as hv2;
use zenkey_model::health::{
    self as hm, Agreement, Answer, Judged, Level, Listing, Major, Presence, Read, Reason, Verdict,
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
    /// `addr`'s part of a reading's `health.v1` GET.
    pub fn of(get: Option<&'a HealthGet>, addr: &Addr) -> Got<'a> {
        Got::of_major(get, addr, Major::V1)
    }

    /// `addr`'s part of a reading's GET of `major`.
    pub fn of_major(get: Option<&'a HealthGet>, addr: &Addr, major: Major) -> Got<'a> {
        match get {
            None => Got::NotMade,
            Some(g) => match g.services_of(major).get(addr) {
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
    /// The status's horizon: the standard contract's 60 s
    /// ([`hm::status_horizon`]), or across a face the revision the
    /// deployment's word names ([`ServiceReading::within`]).
    pub horizon: Horizon,
}

impl ServiceReading {
    /// §2.11's verdict, the status's freshness judged at the reading's
    /// horizon.
    pub fn judged(&self) -> Judged {
        hm::judge_status(
            self.presence,
            freshness::judge_all(&self.horizon, &self.observations),
            self.level,
            self.checks.as_deref(),
        )
    }

    /// This reading, its status judged at `horizon`, a revision's other
    /// than the standard's.
    #[must_use]
    pub fn within(mut self, horizon: Horizon) -> ServiceReading {
        self.horizon = horizon;
        self
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
        horizon: hm::status_horizon(),
    }
}

/// §5's second question over every reading, the first first: *no* only when
/// each reading says no — a break of §2.2 is a finding about the owner from
/// two readings a grace apart, since one reading can pair values never
/// current together; *no* in the last reading alone is unobservable;
/// otherwise the last reading's answer.
pub fn agreement_over(readings: &[Agreement]) -> HealthAnswer {
    agreement_over_in(readings, "health.v1")
}

/// [`agreement_over`], its finding citing `profile`'s §2.2: both majors
/// state the rule there (review-bot on #1168: a v2 answer named it by
/// rewriting v1's words, which a rewording would have left citing v1).
fn agreement_over_in(readings: &[Agreement], profile: &str) -> HealthAnswer {
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
                     owner breaks {profile} §2.2",
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

/// [`agreement_over`] under `health.v2` (v2 §2.2): a break the last reading
/// shows is reported from it alone where that reading is one instant
/// (`one_instant[i]` for `readings[i]`, from
/// [`hv2::Reading::is_one_instant`]); from any other reading, as v1 §2.2
/// says, only when each reading shows it.
pub fn agreement_over_v2(readings: &[Agreement], one_instant: &[bool]) -> HealthAnswer {
    match (readings.last(), one_instant.last()) {
        (Some(last), Some(true)) if last.answer == Answer::No => answer(
            HealthAnswerToken::No,
            Reason::Inconsistent.as_str(),
            "a current check is worse than its fresh status in one reading that is one instant, \
             its two snapshots under one run and one asof with no entry above it: its owner \
             breaks health.v2 §2.2",
        ),
        _ => agreement_over_in(readings, "health.v2"),
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
    level_read_of(r, Major::V1)
}

/// A level as a record of `major` carried it: under `health.v2` its
/// `unknown` is spelled so, v1's `UNSPECIFIED` (v2 §0, §2.6).
pub fn level_read_of(r: Read, major: Major) -> LevelRead {
    match (r, major) {
        (Read::Level(l), _) => LevelRead::Token(l.as_str()),
        (Read::Unspecified, Major::V1) => LevelRead::Token("unspecified"),
        (Read::Unspecified, Major::V2) => LevelRead::Token(hv2::UNKNOWN),
        (Read::Undecodable, _) => LevelRead::Token("undecodable"),
        (Read::Unlisted(n), _) => LevelRead::Unlisted(n),
    }
}

/// The major `addr`'s served descriptors list (v2 §2.11's step 2,
/// §2.12): `health.v2` where any lists it, `health.v1` where one lists
/// that and none `health.v2`, `None` where none lists either or none was
/// read.
pub fn major_of(o: &Observed, addr: &Addr) -> Option<Major> {
    let mut found = None;
    for ((a, _), read) in o.descriptors.iter().flatten() {
        if a != addr {
            continue;
        }
        if let Some(d) = read.descriptor() {
            match Major::of(d.interfaces.iter().map(|e| e.iface.as_str())) {
                Some(Major::V2) => return Some(Major::V2),
                Some(Major::V1) => found = Some(Major::V1),
                None => {}
            }
        }
    }
    found
}

/// What `addr`'s descriptors say of `health.v1` or `health.v2` (§2.7, v2
/// §2.7): listed by any served descriptor, with the token flag of the
/// major read ([`major_of`]); else unread when an instance's descriptor
/// did not read, or descriptors were not asked; else not listed.
pub fn listing(o: &Observed, addr: &Addr) -> Listing {
    let Some(descriptors) = &o.descriptors else {
        return Listing::Unread;
    };
    if let Some(major) = major_of(o, addr) {
        let token = descriptors
            .iter()
            .filter(|((a, _), _)| a == addr)
            .filter_map(|(_, read)| read.descriptor())
            .find_map(|d| d.interfaces.iter().find(|e| e.iface == major.iface()))
            .is_none_or(|e| e.token);
        return Listing::Listed { token };
    }
    let unread = descriptors
        .iter()
        .any(|((a, _), read)| a == addr && read.descriptor().is_none());
    if unread {
        Listing::Unread
    } else {
        Listing::NotListed
    }
}

/// An instance's tables (v2 §2.10) from the revisions its descriptors name
/// that `held` holds: never a revision they do not name.
pub fn instance_tables<'a>(
    o: &Observed,
    addr: &Addr,
    held: impl Fn(&str, &str) -> Option<&'a zenkey_model::contract::Contract>,
) -> hv2::InstanceTables {
    let tables: Vec<hv2::Tables> = o
        .descriptors
        .iter()
        .flatten()
        .filter(|((a, _), _)| a == addr)
        .filter_map(|(_, read)| read.descriptor())
        .flat_map(|d| d.interfaces.iter())
        .filter_map(|e| held(&e.iface, &e.contract))
        .map(hv2::Tables::of)
        .collect();
    hv2::Tables::of_instance(&tables)
}

/// A `health.v2` word as the report shows it (v2 §2.10, §5): its number,
/// how this reader read it, and its name where a table in hand names it.
pub fn word_seen(
    kind: hv2::Kind,
    word: hv2::Word,
    tables: Option<&hv2::InstanceTables>,
) -> crate::report::WordSeen {
    let own = tables.and_then(|t| t.own(kind));
    let (read, name) = match hv2::read_word(kind, word, own) {
        hv2::Named::None => ("none", None),
        hv2::Named::Profile(n) => ("profile", Some(n.to_owned())),
        hv2::Named::Service(n) => ("service", Some(n.to_owned())),
        hv2::Named::Number(_) => ("number", None),
        hv2::Named::OutOfRange => ("out_of_range", None),
    };
    crate::report::WordSeen {
        number: word.code(),
        read,
        name,
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
