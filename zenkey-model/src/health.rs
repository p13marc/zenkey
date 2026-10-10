//! `health.v1`: how well a service serves
//! (`spec/profiles/health/v1.md`, #721).
//!
//! The profile's session-free half: the levels and their order (§2.1,
//! §2.6), the fault codes (§2.10), and the pure judgement of one reading of
//! one service (§2.11), built on [`freshness::judge_all`] for the status,
//! with a tool's roll-up of many (§2.2) and §5's other two questions.
//!
//! ```text
//! Read::from_wire(n)  ─▶ Level(OK < DEGRADED < FAILED) │ Unspecified │ Unlisted(n) │ Undecodable
//! judge(reading)      ─▶ healthy │ unhealthy │ stale │ unobservable │ not_asked, + reason, + level
//! rollup(judged…)     ─▶ the worst established level, every verdict counted
//! agrees(judged, read)─▶ §5 "does its status agree with its checks?": yes │ no │ unestablished
//! clock_ahead(p, heard)─▶ §5 "is its clock ahead?": yes │ no │ unestablished
//! code(s)             ─▶ profile │ application │ malformed
//! ```
//!
//! There is no I/O and no clock here. A reader observes presence, the
//! descriptor, the status (as [`freshness::Observation`]s, plus the level
//! of the latest value) and the checks, and hands them over as values. The
//! owner's puts, its re-puts and its `clock_ahead` fault are the runtime's.

use std::fmt;
use std::time::Duration;

use crate::freshness::{self, Horizon, Observation};

/// The profile's id, which is also its standard contract's interface id.
pub const PROFILE: &str = "health.v1";

/// The standard contract's interface id (§3).
pub const IFACE: &str = "health.v1";

/// The status's template (§3): `state/status`.
pub const STATUS: &str = "status";

/// The checks' template (§3): `state/checks/{check}`.
pub const CHECKS: &str = "checks/{check}";

/// The faults' template (§3): `stream/faults`.
pub const FAULTS: &str = "faults";

/// The status's horizon, the contract's `freshness.ttl_s` (§2.4).
pub const STATUS_TTL: Duration = Duration::from_secs(60);

/// The owner's bound between two confirmations of its status, ttl/2
/// (`freshness.v1` §2.4).
pub const STATUS_REFRESH: Duration = Duration::from_secs(30);

/// The contract's bound on an owner's checks (§2.3).
pub const MAX_CHECKS: u64 = 64;

/// The profile's code for a clock running ahead (§2.5, §2.10).
pub const CLOCK_AHEAD: &str = "clock_ahead";

/// The profile's table of fault codes (§2.10).
pub const CODES: &[&str] = &[CLOCK_AHEAD];

/// The status's horizon, as `freshness.v1` reads it.
#[must_use]
pub fn status_horizon() -> Horizon {
    Horizon::Within(STATUS_TTL)
}

/// A level (§2.1). The order is the declaration's: `Ok` < `Degraded` <
/// `Failed`, a later level being worse. `UNSPECIFIED` is not one (§2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Ok,
    Degraded,
    Failed,
}

impl Level {
    /// The enum's number on the wire (`health.v1.Level`).
    #[must_use]
    pub fn wire(self) -> i32 {
        match self {
            Self::Ok => 1,
            Self::Degraded => 2,
            Self::Failed => 3,
        }
    }

    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Degraded => "degraded",
            Self::Failed => "failed",
        }
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Ok => "OK",
            Self::Degraded => "DEGRADED",
            Self::Failed => "FAILED",
        })
    }
}

/// A level as a reader read it from a payload: a level, or unknown (§2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Read {
    /// `OK`, `DEGRADED` or `FAILED`.
    Level(Level),
    /// `LEVEL_UNSPECIFIED`, 0: unknown, never `OK`.
    Unspecified,
    /// A number the enum does not list, which proto3 keeps (its enums are
    /// open): unknown, never `OK`.
    Unlisted(i32),
    /// A payload that does not decode as the message: unknown.
    Undecodable,
}

impl Read {
    /// The enum's number as decoded.
    #[must_use]
    pub fn from_wire(n: i32) -> Self {
        match n {
            0 => Self::Unspecified,
            1 => Self::Level(Level::Ok),
            2 => Self::Level(Level::Degraded),
            3 => Self::Level(Level::Failed),
            n => Self::Unlisted(n),
        }
    }

    /// The level, when it is one.
    #[must_use]
    pub fn level(self) -> Option<Level> {
        match self {
            Self::Level(l) => Some(l),
            _ => None,
        }
    }
}

/// The worst of the levels among `checks`, with the index of the first
/// check at it; a check at an unknown level bounds nothing (§2.2).
#[must_use]
pub fn worst_check(checks: &[Read]) -> Option<(usize, Level)> {
    checks
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.level().map(|l| (i, l)))
        .fold(None, |best, (i, l)| match best {
            Some((_, b)) if b >= l => best,
            _ => Some((i, l)),
        })
}

/// §2.2: whether a status at `status` is no better than its worst current
/// check. A status may be worse.
#[must_use]
pub fn consistent(status: Level, checks: &[Read]) -> bool {
    worst_check(checks).is_none_or(|(_, w)| w <= status)
}

/// What the descriptor of a present service says (§2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Listing {
    /// Its `interfaces` list `health.v1`, with its `"token"`: `false` for an
    /// interface in the tokenless set. The token plays no part in a verdict.
    Listed { token: bool },
    /// Its `interfaces` do not list `health.v1`.
    NotListed,
    /// The descriptor could not be read.
    Unread,
}

/// A service's presence, as the reader saw it (core §8.1, §8.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// Its instance token was seen, with what its descriptor says.
    Present(Listing),
    /// A complete read held no instance token of it.
    Absent,
    /// Its token was not seen, in a read possibly incomplete or refused.
    Incomplete,
    /// Across a constrained face, where tokens and descriptors do not
    /// cross, and the deployment's word says the service implements
    /// `health.v1` (core R7): whether the face lets `state/status` cross.
    AcrossFace { status_crosses: bool },
}

/// One reading of one service (§0, §2.11).
#[derive(Debug, Clone, Copy)]
pub struct Reading<'a> {
    pub presence: Presence,
    /// The observations of `state/status`, judged as `freshness.v1` judges
    /// them, at [`STATUS_TTL`].
    pub status: &'a [Observation],
    /// The level of the latest status value read, or `None` when no value
    /// was read.
    pub level: Option<Read>,
    /// The levels of the checks the owner answered, or `None` when the
    /// checks were not read.
    pub checks: Option<&'a [Read]>,
}

/// The answer to "is this service healthy?" (§5). Unhealthy and stale are
/// findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Verdict {
    Healthy,
    Unhealthy,
    /// The status was not confirmed within its horizon: a finding of its
    /// own, never a level (§2.4).
    Stale,
    /// The question was put, and the reading could not answer it.
    Unobservable,
    /// Nobody put the question, or its premise does not hold (the reason
    /// says which).
    NotAsked,
}

impl Verdict {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Unhealthy => "unhealthy",
            Self::Stale => "stale",
            Self::Unobservable => "unobservable",
            Self::NotAsked => "not_asked",
        }
    }

    /// Whether the verdict is a finding (§5's polarity).
    #[must_use]
    pub fn is_finding(self) -> bool {
        matches!(self, Self::Unhealthy | Self::Stale)
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a verdict is what it is: the fixture's reason class (§2.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reason {
    /// Healthy: a fresh status at `OK`, no current check worse.
    Ok,
    /// Unhealthy: a fresh status at `DEGRADED`.
    Degraded,
    /// Unhealthy: a fresh status at `FAILED`.
    Failed,
    /// Unhealthy: a current check worse than the status, which breaks §2.2;
    /// read at the check's level.
    Inconsistent,
    /// Unhealthy: the status's level unknown, and a current check
    /// `DEGRADED` or `FAILED`.
    Check,
    /// Unobservable: the status's level is unknown (§2.6).
    UnknownLevel,
    /// Unobservable: the status's payload did not decode (§2.6).
    Undecodable,
    /// Unobservable: the owner deleted its status, which §2.3 forbids.
    Deleted,
    /// Unobservable: its token not seen, in a read possibly incomplete.
    PresenceIncomplete,
    /// Unobservable: the descriptor could not be read (§2.7).
    NoDescriptor,
    /// Unobservable: across a face that does not let the status cross
    /// (§2.8).
    FaceClosed,
    /// Unobservable: across a face that lets the status cross, nothing of
    /// it arrived, so no status is known to exist (§2.8, `freshness.v1`
    /// §2.5).
    NothingCrossed,
    /// Not asked: absent, presence's word (§2.1).
    Absent,
    /// Not asked: the descriptor does not list `health.v1` (§2.7).
    NotListed,
    /// Not asked: only an archive's status was read, last-known (core S6).
    LastKnown,
    /// Unobservable, for §5's second question only ([`agrees`]): the checks
    /// were not read, so whether one is worse than the status is not known.
    ChecksUnread,
    /// Stale or unobservable: the status's freshness, `freshness.v1`'s
    /// reason.
    Freshness(freshness::Reason),
}

impl Reason {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Degraded => "degraded",
            Self::Failed => "failed",
            Self::Inconsistent => "inconsistent",
            Self::Check => "check",
            Self::UnknownLevel => "unknown_level",
            Self::Undecodable => "undecodable",
            Self::Deleted => "deleted",
            Self::PresenceIncomplete => "presence_incomplete",
            Self::NoDescriptor => "no_descriptor",
            Self::FaceClosed => "face_closed",
            Self::NothingCrossed => "nothing_crossed",
            Self::Absent => "absent",
            Self::NotListed => "not_listed",
            Self::LastKnown => "last_known",
            Self::ChecksUnread => "checks_unread",
            Self::Freshness(r) => r.as_str(),
        }
    }

    /// The reason in words, for a tool's message.
    #[must_use]
    pub fn says(self) -> &'static str {
        match self {
            Self::Ok => "its status is OK and confirmed, and no check is worse",
            Self::Degraded => "its status is DEGRADED",
            Self::Failed => "its status is FAILED",
            Self::Inconsistent => "a check is worse than its status, which health.v1 §2.2 forbids",
            Self::Check => "its status's level is unknown, and a check is DEGRADED or FAILED",
            Self::UnknownLevel => "its status's level is unknown",
            Self::Undecodable => "its status does not decode",
            Self::Deleted => "its owner deleted its status",
            Self::PresenceIncomplete => {
                "its token was not seen, and the presence read may be incomplete or refused"
            }
            Self::NoDescriptor => "its descriptor could not be read",
            Self::FaceClosed => "the constrained face does not let its status cross",
            Self::NothingCrossed => {
                "nothing of its status has crossed the constrained face, so none is known to exist"
            }
            Self::Absent => "it is absent: presence's word, not a level",
            Self::NotListed => "its descriptor does not list health.v1",
            Self::LastKnown => "only an archive's last-known status was read, never current",
            Self::ChecksUnread => "its checks were not read",
            Self::Freshness(r) => r.says(),
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A verdict, its reason, and for healthy and unhealthy the level it rests
/// on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Judged {
    pub verdict: Verdict,
    pub reason: Reason,
    pub level: Option<Level>,
}

impl Judged {
    const fn new(verdict: Verdict, reason: Reason, level: Option<Level>) -> Self {
        Self {
            verdict,
            reason,
            level,
        }
    }

    const fn unobservable(reason: Reason) -> Self {
        Self::new(Verdict::Unobservable, reason, None)
    }

    const fn not_asked(reason: Reason) -> Self {
        Self::new(Verdict::NotAsked, reason, None)
    }

    const fn unhealthy(reason: Reason, level: Level) -> Self {
        Self::new(Verdict::Unhealthy, reason, Some(level))
    }
}

/// Judges one reading by §2.11's order: presence, the descriptor, the
/// status's freshness ([`freshness::judge_all`] at [`STATUS_TTL`]), its
/// level, then the checks it vouches for.
#[must_use]
pub fn judge(r: &Reading<'_>) -> Judged {
    use Reason as R;
    // 1. Presence; 2. the descriptor.
    match r.presence {
        Presence::Absent => return Judged::not_asked(R::Absent),
        Presence::Incomplete => return Judged::unobservable(R::PresenceIncomplete),
        Presence::AcrossFace {
            status_crosses: false,
        } => return Judged::unobservable(R::FaceClosed),
        Presence::Present(Listing::NotListed) => return Judged::not_asked(R::NotListed),
        Presence::Present(Listing::Unread) => return Judged::unobservable(R::NoDescriptor),
        Presence::AcrossFace {
            status_crosses: true,
        }
        | Presence::Present(Listing::Listed { .. }) => {}
    }
    // 3. The status's freshness.
    let f = freshness::judge_all(&status_horizon(), r.status);
    match f.verdict {
        freshness::Verdict::Fresh => {}
        // A present, listed owner holds a status (§2.3), so silence ages
        // it. Across a face nothing says one exists until something
        // crosses (`freshness.v1` §2.5, "a member it knows").
        freshness::Verdict::Stale
            if f.reason == freshness::Reason::NoDelivery
                && matches!(r.presence, Presence::AcrossFace { .. }) =>
        {
            return Judged::unobservable(R::NothingCrossed);
        }
        freshness::Verdict::Stale => {
            return Judged::new(Verdict::Stale, R::Freshness(f.reason), None);
        }
        freshness::Verdict::Unobservable => return Judged::unobservable(R::Freshness(f.reason)),
        freshness::Verdict::NotAsked => {
            return match f.reason {
                freshness::Reason::Deleted => Judged::unobservable(R::Deleted),
                freshness::Reason::LastKnown => Judged::not_asked(R::LastKnown),
                // No horizon, or a kind without one: not at the status's
                // fixed horizon, but kept apart rather than guessed.
                other => Judged::not_asked(R::Freshness(other)),
            };
        }
    }
    // 4. The status's level; 5. the checks, current with a fresh status.
    let worst = worst_check(r.checks.unwrap_or(&[])).map(|(_, l)| l);
    match r.level.and_then(Read::level) {
        Some(status) => match worst {
            Some(w) if w > status => Judged::unhealthy(R::Inconsistent, w),
            _ => match status {
                Level::Ok => Judged::new(Verdict::Healthy, R::Ok, Some(Level::Ok)),
                Level::Degraded => Judged::unhealthy(R::Degraded, Level::Degraded),
                Level::Failed => Judged::unhealthy(R::Failed, Level::Failed),
            },
        },
        None => match worst {
            Some(w) if w > Level::Ok => Judged::unhealthy(R::Check, w),
            _ => Judged::unobservable(match r.level {
                Some(Read::Unspecified | Read::Unlisted(_)) => R::UnknownLevel,
                // A fresh status always carries a value; a level not read
                // from it is a payload not read.
                _ => R::Undecodable,
            }),
        },
    }
}

/// A tool's roll-up of many services (§2.2, §5).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rollup {
    /// The worst level among the established verdicts, healthy and
    /// unhealthy; `None` when none is established.
    pub worst: Option<Level>,
    pub healthy: usize,
    pub unhealthy: usize,
    pub stale: usize,
    pub unobservable: usize,
    pub not_asked: usize,
}

/// Rolls up judged readings: the worst established level, with every
/// verdict counted, and stale, unobservable and not asked never folded into
/// a level (§2.2).
#[must_use]
pub fn rollup(judged: impl IntoIterator<Item = Judged>) -> Rollup {
    let mut r = Rollup::default();
    for j in judged {
        match j.verdict {
            Verdict::Healthy => r.healthy += 1,
            Verdict::Unhealthy => r.unhealthy += 1,
            Verdict::Stale => r.stale += 1,
            Verdict::Unobservable => r.unobservable += 1,
            Verdict::NotAsked => r.not_asked += 1,
        }
        if matches!(j.verdict, Verdict::Healthy | Verdict::Unhealthy) {
            r.worst = r.worst.max(j.level);
        }
    }
    r
}

/// A yes-or-no question of §5, answered in the three states: a pole of each
/// Established kind, and the two Unestablished kinds apart (core O5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Answer {
    Yes,
    No,
    /// The question was put, and the reading could not answer it.
    Unobservable,
    /// Nobody put the question, or its premise does not hold.
    NotAsked,
}

impl Answer {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No => "no",
            Self::Unobservable => "unobservable",
            Self::NotAsked => "not_asked",
        }
    }
}

impl fmt::Display for Answer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// §5's second question, "does its status agree with its checks?" (§2.2),
/// for one reading, and its reason in [`Reason`]'s vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Agreement {
    pub answer: Answer,
    pub reason: Reason,
}

/// §5's second question over one reading, from what [`judge`] concluded of
/// it and whether its checks were read: *yes* for a fresh status at a level
/// with no current check worse; *no* for a current check worse than that
/// status ([`Reason::Inconsistent`], the finding about the owner, which a
/// tool reports only from two readings a grace apart, §2.2); not asked as
/// the first question's not asked; unobservable when the status is not
/// fresh, its level is unknown, or the checks were not read.
#[must_use]
pub fn agrees(judged: Judged, checks_read: bool) -> Agreement {
    let a = |answer, reason| Agreement { answer, reason };
    match (judged.verdict, judged.reason) {
        (Verdict::NotAsked, r) => a(Answer::NotAsked, r),
        (_, Reason::Inconsistent) => a(Answer::No, Reason::Inconsistent),
        (
            Verdict::Healthy | Verdict::Unhealthy,
            r @ (Reason::Ok | Reason::Degraded | Reason::Failed),
        ) => {
            if checks_read {
                a(Answer::Yes, r)
            } else {
                a(Answer::Unobservable, Reason::ChecksUnread)
            }
        }
        // A status at an unknown level bounds nothing (§2.6), so whether a
        // check is worse than it has no answer.
        (Verdict::Unhealthy, Reason::Check) => a(Answer::Unobservable, Reason::UnknownLevel),
        (_, r) => a(Answer::Unobservable, r),
    }
}

/// One delivery a subscriber heard of one service during its reading, as
/// §5's last question reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Heard {
    /// A put of `state/status`, a change or a re-put: a confirmation.
    Status,
    /// A delete of `state/status`, which §2.3 forbids: no confirmation.
    StatusDeleted,
    /// A `faults` sample whose code is `clock_ahead` (§2.5, §2.10).
    ClockAhead,
    /// A `faults` sample with any other code, which says nothing of the
    /// clock.
    Fault,
}

/// Why §5's last question has its answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClockReason {
    /// Yes: a `clock_ahead` fault, and no confirmation of the status heard
    /// since.
    ClockAhead,
    /// No: the status confirmed after the last `clock_ahead` fault, or with
    /// none heard. Its guard did not hold that write.
    Confirmed,
    /// Unobservable: no `clock_ahead` fault heard, and the status not
    /// confirmed within the reader's window. Never "no" from the faults'
    /// silence.
    NothingHeard,
    /// Not asked: absent, presence's word (§2.1).
    Absent,
    /// Not asked: the descriptor does not list `health.v1` (§2.7).
    NotListed,
}

impl ClockReason {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClockAhead => "clock_ahead",
            Self::Confirmed => "confirmed",
            Self::NothingHeard => "nothing_heard",
            Self::Absent => "absent",
            Self::NotListed => "not_listed",
        }
    }

    /// The reason in words, for a tool's message.
    #[must_use]
    pub fn says(self) -> &'static str {
        match self {
            Self::ClockAhead => {
                "a clock_ahead fault was heard, and no confirmation of its status since"
            }
            Self::Confirmed => {
                "its status was confirmed after its last clock_ahead fault, or with none heard"
            }
            Self::NothingHeard => {
                "no clock_ahead fault was heard, and its status was not confirmed in the window"
            }
            Self::Absent => "it is absent: presence's word",
            Self::NotListed => "its descriptor does not list health.v1",
        }
    }
}

impl fmt::Display for ClockReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// §5's last question, "is this service's clock ahead?" (§2.5), and why.
/// Yes is the finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    pub answer: Answer,
    pub reason: ClockReason,
}

/// §5's last question from what one subscriber heard of one service in its
/// window, in arrival order: *yes* on a `clock_ahead` fault with no
/// confirmation of the status heard since; *no* when the status was
/// confirmed after the last one, or with none heard; unobservable when
/// neither was heard, since the faults' silence is their normal state (core
/// O5). Not asked as the first question's: absent, or not listing
/// `health.v1`. Whatever else presence says, a fault heard is the service's
/// own word, and answers. Nothing rests on a fault's stamp, nor its level
/// (§2.5).
#[must_use]
pub fn clock_ahead(presence: Presence, heard: &[Heard]) -> Clock {
    let c = |answer, reason| Clock { answer, reason };
    match presence {
        Presence::Absent => return c(Answer::NotAsked, ClockReason::Absent),
        Presence::Present(Listing::NotListed) => {
            return c(Answer::NotAsked, ClockReason::NotListed);
        }
        _ => {}
    }
    let fault = heard.iter().rposition(|h| *h == Heard::ClockAhead);
    let status = heard.iter().rposition(|h| *h == Heard::Status);
    match (fault, status) {
        (Some(f), Some(s)) if f > s => c(Answer::Yes, ClockReason::ClockAhead),
        (Some(_), None) => c(Answer::Yes, ClockReason::ClockAhead),
        (_, Some(_)) => c(Answer::No, ClockReason::Confirmed),
        (None, None) => c(Answer::Unobservable, ClockReason::NothingHeard),
    }
}

/// A fault's code, read by §2.10.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code<'a> {
    /// A code of the profile's table ([`CODES`]).
    Profile(&'static str),
    /// Any other code of the form `[a-z][a-z0-9_]*`: an application's.
    Application(&'a str),
    /// Not of that form: still an occurrence, shown verbatim.
    Malformed(&'a str),
}

impl Code<'_> {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Profile(_) => "profile",
            Self::Application(_) => "application",
            Self::Malformed(_) => "malformed",
        }
    }
}

/// Reads a fault's code (§2.10), exactly: no trimming, no case folding.
#[must_use]
pub fn code(s: &str) -> Code<'_> {
    let mut bytes = s.bytes();
    let well_formed = bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if !well_formed {
        return Code::Malformed(s);
    }
    match CODES.iter().find(|c| **c == s) {
        Some(c) => Code::Profile(c),
        None => Code::Application(s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::freshness::Last;

    fn fresh() -> [Observation; 1] {
        [Observation::Subscribed {
            last: Some(Last::Put(Duration::from_secs(10))),
            listened: Duration::from_secs(120),
            complete: true,
        }]
    }

    fn present() -> Presence {
        Presence::Present(Listing::Listed { token: false })
    }

    #[test]
    fn the_levels_are_ordered_and_unspecified_is_none_of_them() {
        assert!(Level::Ok < Level::Degraded && Level::Degraded < Level::Failed);
        for l in [Level::Ok, Level::Degraded, Level::Failed] {
            assert_eq!(Read::from_wire(l.wire()), Read::Level(l));
        }
        assert_eq!(Read::from_wire(0), Read::Unspecified);
        assert_eq!(Read::from_wire(4), Read::Unlisted(4));
        assert_eq!(Read::Unspecified.level(), None);
    }

    #[test]
    fn the_worst_check_is_the_first_at_the_worst_known_level() {
        let checks = [
            Read::Level(Level::Degraded),
            Read::Unlisted(9),
            Read::Level(Level::Failed),
            Read::Level(Level::Failed),
        ];
        assert_eq!(worst_check(&checks), Some((2, Level::Failed)));
        assert_eq!(worst_check(&[Read::Unspecified]), None);
        assert!(consistent(Level::Failed, &checks));
        assert!(!consistent(Level::Degraded, &checks));
        assert!(consistent(Level::Ok, &[]));
    }

    #[test]
    fn a_fresh_status_vouches_for_its_checks_and_a_stale_one_does_not() {
        let o = fresh();
        let checks = [Read::Level(Level::Failed)];
        let mut r = Reading {
            presence: present(),
            status: &o,
            level: Some(Read::Level(Level::Ok)),
            checks: Some(&checks),
        };
        assert_eq!(
            judge(&r),
            Judged::unhealthy(Reason::Inconsistent, Level::Failed)
        );
        let stale = [Observation::Subscribed {
            last: Some(Last::Put(Duration::from_secs(61))),
            listened: Duration::from_secs(120),
            complete: true,
        }];
        r.status = &stale;
        assert_eq!(judge(&r).verdict, Verdict::Stale);
    }

    #[test]
    fn a_rollup_never_folds_stale_into_a_level() {
        let stale = Judged::new(
            Verdict::Stale,
            Reason::Freshness(freshness::Reason::BeyondHorizon),
            None,
        );
        let r = rollup([
            stale,
            Judged::new(Verdict::Healthy, Reason::Ok, Some(Level::Ok)),
        ]);
        assert_eq!(r.worst, Some(Level::Ok));
        assert_eq!((r.healthy, r.stale), (1, 1));
        assert_eq!(rollup([stale]).worst, None);
    }

    #[test]
    fn the_status_agrees_with_its_checks_only_when_fresh_at_a_level_and_read() {
        let j = |verdict, reason, level| Judged::new(verdict, reason, level);
        let ok = j(Verdict::Healthy, Reason::Ok, Some(Level::Ok));
        assert_eq!(agrees(ok, true).answer, Answer::Yes);
        assert_eq!(
            agrees(ok, false),
            Agreement {
                answer: Answer::Unobservable,
                reason: Reason::ChecksUnread
            }
        );
        let liar = Judged::unhealthy(Reason::Inconsistent, Level::Failed);
        assert_eq!(agrees(liar, true).answer, Answer::No);
        let frank = Judged::unhealthy(Reason::Failed, Level::Failed);
        assert_eq!(agrees(frank, true).answer, Answer::Yes, "worse is allowed");
        let unknown = Judged::unhealthy(Reason::Check, Level::Failed);
        assert_eq!(agrees(unknown, true).reason, Reason::UnknownLevel);
        let stale = j(
            Verdict::Stale,
            Reason::Freshness(freshness::Reason::BeyondHorizon),
            None,
        );
        assert_eq!(agrees(stale, true).answer, Answer::Unobservable);
        let absent = Judged::not_asked(Reason::Absent);
        assert_eq!(agrees(absent, true).answer, Answer::NotAsked);
    }

    #[test]
    fn the_clock_is_ahead_on_a_fault_no_confirmation_followed() {
        use Heard::{ClockAhead, Fault, Status, StatusDeleted};
        let p = present();
        let a = |heard: &[Heard]| clock_ahead(p, heard).answer;
        assert_eq!(a(&[ClockAhead]), Answer::Yes);
        assert_eq!(a(&[Status, ClockAhead]), Answer::Yes);
        assert_eq!(a(&[ClockAhead, Status]), Answer::No);
        assert_eq!(a(&[ClockAhead, StatusDeleted]), Answer::Yes);
        assert_eq!(a(&[Status]), Answer::No);
        assert_eq!(a(&[Fault]), Answer::Unobservable, "never from silence");
        assert_eq!(a(&[]), Answer::Unobservable);
        assert_eq!(
            clock_ahead(Presence::Absent, &[ClockAhead]).answer,
            Answer::NotAsked
        );
    }

    #[test]
    fn codes_are_lowercase_ascii_and_the_table_is_the_profiles() {
        assert_eq!(code("clock_ahead"), Code::Profile(CLOCK_AHEAD));
        assert_eq!(code("tc_netns_gone"), Code::Application("tc_netns_gone"));
        assert_eq!(code(""), Code::Malformed(""));
        assert_eq!(code("_x"), Code::Malformed("_x"));
        assert_eq!(code("Clock_ahead").as_str(), "malformed");
    }
}
