//! `timing.v1`: a stream's cadence, its deadline and its lifespan
//! (`spec/profiles/timing/v1.md`, #908).
//!
//! The profile's session-free half: the declaration read from a resolved
//! resource (§2.2, §2.3), a reader's deadline tracker for one provider and
//! resource (§2.5), the lifespan of each sample a reader judges (§2.6), the
//! period over a tool's window (§2.7), and §5's three questions.
//!
//! ```text
//! declaration(kind, annotations)        ─▶ declared {period, deadline, lifespan, below_period} │ invalid [reasons] │ ignored │ not this profile's
//! Tracker::observe(at, event)           ─▶ missed {due, by, owner} │ met │ ended {gone, forgotten} │ beyond_bound     §2.5
//!   events: start │ arrival(member, expired) │ owner(present, unconfirmed, absent) │ member_gone │ tick
//! LifespanReader::judge(stamp)          ─▶ live │ expired │ unobservable │ not asked │ not this profile's, + reason  §2.6
//! period_window(period, window, …)      ─▶ kept │ not_kept │ too_short │ not_heard                                 §2.7
//! Holding::observe(tracker, reports, at) ─▶ each member's held time: while heard, missed or waited for, to its first end §2.7
//! deadline_met / period_kept / lifespan_expired ─▶ yes │ no │ unobservable │ not_asked │ not_this_profiles        §5
//! roll_up / roll_up_first               ─▶ a resource's or a run's answer, and its first member's reason          §5
//! ```
//!
//! There is no I/O and no clock here. A reader takes each arrival's instant
//! on its monotonic clock and each sample's age on its wall clock, follows
//! its provider's presence, and hands them over as values: the runtime
//! drives [`Tracker`] from its subscription and a one-shot timer at
//! [`Tracker::next_due`], and a tool drives the same tracker over its
//! window. A sample's age is
//! [`freshness::StampAge`](crate::freshness::StampAge), and a clock that
//! falsified the deployment's word is found as `freshness.v1` §2.6 finds
//! one, with [`freshness::ClockMeasure`](crate::freshness::ClockMeasure).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::Duration;

use serde_json::Value;

use crate::authoring::Kind;
use crate::contract::Resource;
use crate::freshness::{ClockMeasure, StampAge};

/// The profile's id, as a contract lists it in `uses`.
pub const PROFILE: &str = "timing.v1";

/// The profile's name, as an annotation key's prefix carries it.
pub const NAME: &str = "timing";

/// `timing.period_ms` (§4).
pub const PERIOD_MS: &str = "timing.period_ms";
/// `timing.deadline_ms` (§4).
pub const DEADLINE_MS: &str = "timing.deadline_ms";
/// `timing.lifespan_ms` (§4).
pub const LIFESPAN_MS: &str = "timing.lifespan_ms";

/// The published vocabulary (§4): the keys under `timing.`, which W105
/// reads in place of core Appendix D's interim row (core 0.53), unchanged.
pub const VOCABULARY: &[&str] = &["period_ms", "deadline_ms", "lifespan_ms"];

/// The largest value, in milliseconds: 2^53−1, the canonical domain's
/// bound (§2.2, core §9.5).
pub const MAX_MS: u64 = (1 << 53) - 1;

/// How long after its last arrival a member that is not known, once
/// missed, is forgotten (§2.5): an hour, core §2.7's liveness span of a
/// stream member.
pub const FORGET_AFTER: Duration = Duration::from_secs(3600);

/// The floor of the detection bound (§2.5): 1 ms.
pub const BOUND_FLOOR: Duration = Duration::from_millis(1);

// --- §2.2, §2.3: the declaration ----------------------------------------------

/// Why a declaration is invalid (§2.2), in the order checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Invalid {
    PeriodValue,
    DeadlineValue,
    LifespanValue,
}

impl Invalid {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PeriodValue => "period_value",
            Self::DeadlineValue => "deadline_value",
            Self::LifespanValue => "lifespan_value",
        }
    }

    /// The reason in words, for a linter's or a tool's message.
    #[must_use]
    pub fn says(self) -> &'static str {
        match self {
            Self::PeriodValue => {
                "timing.period_ms is not a whole number of milliseconds from 1 to 2^53-1"
            }
            Self::DeadlineValue => {
                "timing.deadline_ms is not a whole number of milliseconds from 1 to 2^53-1"
            }
            Self::LifespanValue => {
                "timing.lifespan_ms is not a whole number of milliseconds from 1 to 2^53-1"
            }
        }
    }
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.says())
    }
}

/// A timed resource's values (§2.2), each `None` when its key is absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Declared {
    pub period: Option<Duration>,
    pub deadline: Option<Duration>,
    pub lifespan: Option<Duration>,
}

impl Declared {
    /// Whether a period and a deadline are both declared and the deadline
    /// is below the period: no deadline, which a linter warns of (§2.5).
    #[must_use]
    pub fn below_period(&self) -> bool {
        matches!((self.period, self.deadline), (Some(p), Some(d)) if d < p)
    }

    /// The deadline a reader judges (§2.5): the declared one, unless it is
    /// below the period.
    #[must_use]
    pub fn judged_deadline(&self) -> Option<Duration> {
        if self.below_period() {
            None
        } else {
            self.deadline
        }
    }
}

/// What a reader reads of a resolved resource or a requirement (§2.2,
/// §2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Declaration {
    /// No key of the vocabulary.
    NotThisProfiles,
    /// A key of the vocabulary, whatever its value, on a state, an event,
    /// an operation or a requirement: it means nothing there, and a linter
    /// warns of it (§2.3).
    Ignored,
    /// A stream whose values are not all valid: the reasons, in §2.2's
    /// order. Every question of §5 is then unobservable.
    Invalid(Vec<Invalid>),
    /// A timed resource (§0).
    Declared(Declared),
}

impl Declaration {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotThisProfiles => "not_this_profiles",
            Self::Ignored => "ignored",
            Self::Invalid(_) => "invalid",
            Self::Declared(_) => "declared",
        }
    }

    /// The values of a valid declaration.
    #[must_use]
    pub fn declared(&self) -> Option<&Declared> {
        match self {
            Self::Declared(d) => Some(d),
            _ => None,
        }
    }
}

/// A whole number of milliseconds from 1 to 2^53−1, or `None` (§2.2). An
/// integral float is that integer: `100.0` is 100, and `-0.0` is 0, which
/// is no value.
fn millis(v: &Value) -> Option<u64> {
    let n = v.as_number()?;
    if let Some(u) = n.as_u64() {
        return (1..=MAX_MS).contains(&u).then_some(u);
    }
    if n.is_i64() {
        return None;
    }
    let f = n.as_f64()?;
    let max = MAX_MS as f64;
    (f.is_finite() && f.fract() == 0.0 && f >= 1.0 && f <= max).then_some(f as u64)
}

fn any_key(annotations: &BTreeMap<String, Value>) -> bool {
    [PERIOD_MS, DEADLINE_MS, LIFESPAN_MS]
        .iter()
        .any(|k| annotations.contains_key(*k))
}

/// Reads the declaration of a resource of `kind` from its resolved
/// annotations (§2.2, §2.3): `[defaults]`, then `[defaults.<kind>]`, then
/// its own, as the contract model merges them (core §9.3). The kind is read
/// first, then each key alone. Another `timing.<key>` is W105's and plays
/// no part.
#[must_use]
pub fn declaration(kind: Kind, annotations: &BTreeMap<String, Value>) -> Declaration {
    if !any_key(annotations) {
        return Declaration::NotThisProfiles;
    }
    if kind != Kind::Stream {
        return Declaration::Ignored;
    }
    let mut why = Vec::new();
    let mut read = |key: &str, wrong: Invalid| -> Option<Duration> {
        let v = annotations.get(key)?;
        let ms = millis(v);
        if ms.is_none() {
            why.push(wrong);
        }
        ms.map(Duration::from_millis)
    };
    let declared = Declared {
        period: read(PERIOD_MS, Invalid::PeriodValue),
        deadline: read(DEADLINE_MS, Invalid::DeadlineValue),
        lifespan: read(LIFESPAN_MS, Invalid::LifespanValue),
    };
    if why.is_empty() {
        Declaration::Declared(declared)
    } else {
        Declaration::Invalid(why)
    }
}

/// [`declaration`] of a resource of a contract.
#[must_use]
pub fn declaration_of(r: &Resource) -> Declaration {
    declaration(r.kind, &r.annotations)
}

/// The declaration of a requirement (`[requires.<role>]`) from its own
/// annotations (§2.3): a key of the vocabulary there means nothing at this
/// version, and a linter warns of it.
#[must_use]
pub fn requirement_declaration(annotations: &BTreeMap<String, Value>) -> Declaration {
    if any_key(annotations) {
        Declaration::Ignored
    } else {
        Declaration::NotThisProfiles
    }
}

/// The tracker a reader judges a resource of `kind` and declaration `d`
/// by (§2.2, §2.5): `None` when it judges no deadline, the resource not a
/// stream, its declaration invalid, no deadline declared, or the deadline
/// below the period (text 0.2, F-259). `bound` and `known` are
/// [`Tracker::new`]'s.
#[must_use]
pub fn tracker_for<S: Into<String>>(
    kind: Kind,
    d: &Declaration,
    bound: Option<u64>,
    known: impl IntoIterator<Item = S>,
) -> Option<Tracker> {
    if kind != Kind::Stream {
        return None;
    }
    let deadline = d.declared()?.judged_deadline()?;
    Some(Tracker::new(deadline, bound, known))
}

/// The detection bound of a deadline (§2.5): max(deadline/50, 1 ms). A
/// reader reports a miss at most this long after it is due.
#[must_use]
pub fn detection_bound(deadline: Duration) -> Duration {
    (deadline / 50).max(BOUND_FLOOR)
}

// --- §2.5: the deadline -----------------------------------------------------------

/// An instant on the reader's monotonic clock (§0): the time since an
/// origin of the reader's choosing, which never steps backwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Mono(Duration);

impl Mono {
    /// The origin.
    pub const ZERO: Self = Self(Duration::ZERO);

    /// The instant `d` after the origin.
    #[must_use]
    pub const fn after(d: Duration) -> Self {
        Self(d)
    }

    /// The instant `ns` nanoseconds after the origin.
    #[must_use]
    pub const fn from_nanos(ns: u64) -> Self {
        Self(Duration::from_nanos(ns))
    }

    /// The time since the origin.
    #[must_use]
    pub const fn since_origin(self) -> Duration {
        self.0
    }

    /// This instant plus `d`, saturating.
    #[must_use]
    pub fn saturating_add(self, d: Duration) -> Self {
        Self(self.0.saturating_add(d))
    }

    /// The time from `earlier` to this instant, zero when `earlier` is
    /// later.
    #[must_use]
    pub fn saturating_since(self, earlier: Self) -> Duration {
        self.0.saturating_sub(earlier.0)
    }
}

/// The owner's token as the reader observes it (§0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OwnerState {
    /// No state given yet: the reader follows no presence, or has read
    /// none.
    #[default]
    Unseen,
    /// The token held as seen: shown by a read or heard declared, and no
    /// withdrawal of it heard since.
    Present,
    /// A withdrawal heard, and no read since (core §8.1, 0.37).
    Unconfirmed,
    /// A complete read showed no such token (core §8.1, 0.37).
    Absent,
}

impl OwnerState {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unseen => "unseen",
            Self::Present => "present",
            Self::Unconfirmed => "unconfirmed",
            Self::Absent => "absent",
        }
    }

    /// Parses the fixture's token.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "unseen" => Self::Unseen,
            "present" => Self::Present,
            "unconfirmed" => Self::Unconfirmed,
            "absent" => Self::Absent,
            _ => return None,
        })
    }
}

/// What a reader hands its tracker, at an instant (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event<'a> {
    /// The subscription declared: its start.
    Start,
    /// A sample of `member` (its resource chunks, or its whole key, as the
    /// reader names members) that counts as one by §2.5: a put on a
    /// concrete key. An `expired` one (§2.6) is no arrival.
    Arrival { member: &'a str, expired: bool },
    /// The owner's token as the reader now observes it.
    Owner(OwnerState),
    /// A member token of `member` seen (core §8.1): heard declared, heard
    /// withdrawn, or shown by a read. On a template that declares `epoch`,
    /// a known member is waited for from the first (§2.5, text 0.2,
    /// F-261).
    MemberSeen(&'a str),
    /// A complete read shows no member token of `member` (core §8.1). On a
    /// template that declares `epoch`, it ends a member only after a token
    /// of it was seen (text 0.2, F-261).
    MemberGone(&'a str),
    /// An instant the reader judges at: its timer.
    Tick,
}

/// Why a reader stopped judging a member (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EndReason {
    /// Its member tokens are gone.
    Gone,
    /// Not known, missed, and silent for [`FORGET_AFTER`].
    Forgotten,
}

impl EndReason {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gone => "gone",
            Self::Forgotten => "forgotten",
        }
    }
}

/// A report a tracker makes (§2.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    /// The member became missed. `due` is the instant from which the miss
    /// holds (its last arrival, or the start of its wait, plus the
    /// deadline); `by`, the latest instant a reader may report it, due plus
    /// the detection bound; `owner`, the owner's state at the report.
    Missed {
        at: Mono,
        member: String,
        due: Mono,
        by: Mono,
        owner: OwnerState,
    },
    /// The member's next arrival after a miss.
    Met { at: Mono, member: String },
    /// The reader stopped judging a member it had heard or reported
    /// missed.
    Ended {
        at: Mono,
        member: String,
        reason: EndReason,
    },
    /// A member first heard while the bound's members are held: not held,
    /// and not judged, past the owner's bound (core §2.7).
    BeyondBound { at: Mono, member: String },
}

impl Report {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Missed { .. } => "missed",
            Self::Met { .. } => "met",
            Self::Ended { .. } => "ended",
            Self::BeyondBound { .. } => "beyond_bound",
        }
    }

    /// The member it is about.
    #[must_use]
    pub fn member(&self) -> &str {
        match self {
            Self::Missed { member, .. }
            | Self::Met { member, .. }
            | Self::Ended { member, .. }
            | Self::BeyondBound { member, .. } => member,
        }
    }

    /// The instant it was made.
    #[must_use]
    pub fn at(&self) -> Mono {
        match self {
            Self::Missed { at, .. }
            | Self::Met { at, .. }
            | Self::Ended { at, .. }
            | Self::BeyondBound { at, .. } => *at,
        }
    }
}

/// Where a held member stands (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberState {
    /// A known member neither heard nor waited for: before the start, its
    /// owner not present, or after it ended (`after_end`), until its next
    /// arrival.
    Idle { after_end: bool },
    /// A known member never heard, its owner present: missed once more than
    /// the deadline has passed since `since`.
    Waiting { since: Mono },
    /// In time: its last arrival.
    Heard { last: Mono },
    /// Missed: its last arrival, `None` for a known member never heard.
    Missed { last: Option<Mono> },
}

#[derive(Debug, Clone, Copy)]
struct Held {
    known: bool,
    state: MemberState,
}

/// One reader's deadline tracker for one provider address and one resource
/// (§2.5): a pure state machine over (instant, event) inputs. At each
/// event it first judges every member it holds at that instant, then
/// applies the event; [`Tracker::next_due`] is when a miss is next due, for
/// a one-shot timer that meets the detection bound.
#[derive(Debug, Clone)]
pub struct Tracker {
    deadline: Duration,
    bound: Option<u64>,
    start: Option<Mono>,
    owner: OwnerState,
    owner_since: Option<Mono>,
    held: BTreeMap<String, Held>,
    /// The template declares `epoch` (core §2.3): its members' tokens, not
    /// the owner's presence, say when a known member exists (F-261).
    epoch: bool,
    /// The members a token of which was seen since they last ended, and
    /// the instant the first was.
    seen: BTreeMap<String, Mono>,
    /// The members reported past the bound, until one is held (F-260).
    beyond: BTreeSet<String>,
}

impl Tracker {
    /// A tracker for a resource whose judged deadline is `deadline`
    /// ([`Declared::judged_deadline`]) and whose bound is `bound` (core
    /// §2.7: the instance's stated `cardinality`, else the contract's; 1 for
    /// a template without parameters; `None` for no bound), holding the
    /// `known` members (§0) from the first.
    pub fn new<S: Into<String>>(
        deadline: Duration,
        bound: Option<u64>,
        known: impl IntoIterator<Item = S>,
    ) -> Self {
        let held = known
            .into_iter()
            .map(|m| {
                (
                    m.into(),
                    Held {
                        known: true,
                        state: MemberState::Idle { after_end: false },
                    },
                )
            })
            .collect();
        Self {
            deadline,
            bound,
            start: None,
            owner: OwnerState::Unseen,
            owner_since: None,
            held,
            epoch: false,
            seen: BTreeMap::new(),
            beyond: BTreeSet::new(),
        }
    }

    /// The tracker of a template that declares `epoch` (core §2.3, §8.1;
    /// §2.5, text 0.2, F-261): a known member is waited for from the
    /// instant a member token of it is first seen, never from its owner's
    /// presence, since a member exists from its owner's first declaration
    /// of it; and its member tokens gone end it only after one was seen.
    #[must_use]
    pub fn with_epoch(mut self) -> Self {
        self.epoch = true;
        self
    }

    /// The deadline judged.
    #[must_use]
    pub fn deadline(&self) -> Duration {
        self.deadline
    }

    /// The owner's state as last given.
    #[must_use]
    pub fn owner(&self) -> OwnerState {
        self.owner
    }

    /// The members held, in member order.
    pub fn held(&self) -> impl Iterator<Item = &str> {
        self.held.keys().map(String::as_str)
    }

    /// Where a held member stands.
    #[must_use]
    pub fn state(&self, member: &str) -> Option<MemberState> {
        self.held.get(member).map(|h| h.state)
    }

    /// The earliest instant at which a miss is due among the members heard
    /// or waited for: a miss holds at any instant after it. `None` when no
    /// member can become missed without an event.
    #[must_use]
    pub fn next_due(&self) -> Option<Mono> {
        self.held
            .values()
            .filter_map(|h| match h.state {
                MemberState::Heard { last } => Some(last),
                MemberState::Waiting { since } => Some(since),
                _ => None,
            })
            .min()
            .map(|t| t.saturating_add(self.deadline))
    }

    /// Judges every member held at `at`, then applies `event` (§2.5).
    /// Returns the reports, in order: the misses due and the members
    /// forgotten, in member order, then the event's own.
    pub fn observe(&mut self, at: Mono, event: Event<'_>) -> Vec<Report> {
        let mut out = self.evaluate(at);
        self.apply(at, event, &mut out);
        out
    }

    fn evaluate(&mut self, at: Mono) -> Vec<Report> {
        let (d, bound, owner) = (self.deadline, detection_bound(self.deadline), self.owner);
        let mut out = Vec::new();
        let mut forgotten = Vec::new();
        for (name, h) in &mut self.held {
            let from = match h.state {
                MemberState::Heard { last } => Some((last, Some(last))),
                MemberState::Waiting { since } => Some((since, None)),
                _ => None,
            };
            if let Some((from, last)) = from {
                let due = from.saturating_add(d);
                if at > due {
                    h.state = MemberState::Missed { last };
                    out.push(Report::Missed {
                        at,
                        member: name.clone(),
                        due,
                        by: due.saturating_add(bound),
                        owner,
                    });
                }
            }
            if !h.known
                && let MemberState::Missed { last: Some(last) } = h.state
                && at.saturating_since(last) >= FORGET_AFTER
            {
                forgotten.push(name.clone());
                out.push(Report::Ended {
                    at,
                    member: name.clone(),
                    reason: EndReason::Forgotten,
                });
            }
        }
        for name in forgotten {
            self.held.remove(&name);
        }
        out
    }

    fn wait_known(&mut self, since: Mono) {
        for h in self.held.values_mut() {
            if h.known && h.state == (MemberState::Idle { after_end: false }) {
                h.state = MemberState::Waiting { since };
            }
        }
    }

    fn apply(&mut self, at: Mono, event: Event<'_>, out: &mut Vec<Report>) {
        match event {
            Event::Start => {
                if self.start.is_some() {
                    return;
                }
                self.start = Some(at);
                if self.epoch {
                    for (name, h) in &mut self.held {
                        if let Some(seen) = self.seen.get(name)
                            && h.known
                            && matches!(h.state, MemberState::Idle { .. })
                        {
                            h.state = MemberState::Waiting {
                                since: (*seen).max(at),
                            };
                        }
                    }
                } else if self.owner == OwnerState::Present {
                    let since = self.owner_since.map_or(at, |s| s.max(at));
                    self.wait_known(since);
                }
            }
            Event::Arrival { member, expired } => {
                if self.start.is_none() || expired {
                    return;
                }
                if let Some(h) = self.held.get_mut(member) {
                    let was_missed = matches!(h.state, MemberState::Missed { .. });
                    h.state = MemberState::Heard { last: at };
                    if was_missed {
                        out.push(Report::Met {
                            at,
                            member: member.to_owned(),
                        });
                    }
                } else if self
                    .bound
                    .is_some_and(|b| u64::try_from(self.held.len()).unwrap_or(u64::MAX) >= b)
                {
                    // Once, at its first arrival past the bound (F-260).
                    if self.beyond.insert(member.to_owned()) {
                        out.push(Report::BeyondBound {
                            at,
                            member: member.to_owned(),
                        });
                    }
                } else {
                    self.beyond.remove(member);
                    self.held.insert(
                        member.to_owned(),
                        Held {
                            known: false,
                            state: MemberState::Heard { last: at },
                        },
                    );
                }
            }
            Event::Owner(state) => match (self.owner, state) {
                (OwnerState::Unseen | OwnerState::Absent, OwnerState::Present) => {
                    self.owner = OwnerState::Present;
                    self.owner_since = Some(at);
                    if let Some(start) = self.start
                        && !self.epoch
                    {
                        self.wait_known(at.max(start));
                    }
                }
                (OwnerState::Unconfirmed, OwnerState::Present)
                | (OwnerState::Present, OwnerState::Unconfirmed) => self.owner = state,
                (_, OwnerState::Absent) => {
                    self.owner = OwnerState::Absent;
                    self.owner_since = None;
                    if !self.epoch {
                        for h in self.held.values_mut() {
                            if matches!(h.state, MemberState::Waiting { .. }) {
                                h.state = MemberState::Idle { after_end: false };
                            }
                        }
                    }
                }
                _ => {}
            },
            Event::MemberSeen(member) => {
                if !self.epoch || self.seen.contains_key(member) {
                    return;
                }
                self.seen.insert(member.to_owned(), at);
                if self.start.is_some()
                    && let Some(h) = self.held.get_mut(member)
                    && h.known
                    && matches!(h.state, MemberState::Idle { .. })
                {
                    h.state = MemberState::Waiting { since: at };
                }
            }
            Event::MemberGone(member) => {
                // A read that shows none before any was seen ends nothing
                // (F-261).
                if self.epoch && self.seen.remove(member).is_none() {
                    return;
                }
                let Some(h) = self.held.get_mut(member) else {
                    return;
                };
                if matches!(
                    h.state,
                    MemberState::Heard { .. } | MemberState::Missed { .. }
                ) {
                    out.push(Report::Ended {
                        at,
                        member: member.to_owned(),
                        reason: EndReason::Gone,
                    });
                }
                if h.known {
                    h.state = MemberState::Idle { after_end: true };
                } else {
                    self.held.remove(member);
                }
            }
            Event::Tick => {}
        }
    }
}

// --- §2.6: the lifespan -----------------------------------------------------------

/// A zid's value from its text (core §3.3): 1 to 32 hexadecimal digits,
/// case and leading zeros carrying no meaning.
#[must_use]
pub fn zid_value(s: &str) -> Option<u128> {
    let ok = !s.is_empty() && s.len() <= 32 && s.bytes().all(|b| b.is_ascii_hexdigit());
    if ok {
        u128::from_str_radix(s, 16).ok()
    } else {
        None
    }
}

/// A sample's stamp as a lifespan reader sees it (§2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamped {
    /// The reader's wall clock at the arrival minus the stamp's time,
    /// negative when the stamp is ahead.
    pub age: StampAge,
    /// The stamp's id, a zid's value ([`zid_value`]).
    pub id: u128,
}

/// The answer of one sample's lifespan (§2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SampleVerdict {
    Live,
    Expired,
    Unobservable,
    NotAsked,
    NotThisProfiles,
}

impl SampleVerdict {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Expired => "expired",
            Self::Unobservable => "unobservable",
            Self::NotAsked => "not_asked",
            Self::NotThisProfiles => "not_this_profiles",
        }
    }

    /// Parses the fixture's token.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "live" => Self::Live,
            "expired" => Self::Expired,
            "unobservable" => Self::Unobservable,
            "not_asked" => Self::NotAsked,
            "not_this_profiles" => Self::NotThisProfiles,
            _ => return None,
        })
    }
}

/// Why an answer is what it is: the fixtures' reasons (§2.6, §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reason {
    /// Not a stream (§2.3).
    Kind,
    /// The declaration is invalid (§2.2).
    Invalid,
    /// No `timing.deadline_ms`.
    NoDeadline,
    /// No `timing.period_ms`.
    NoPeriod,
    /// No `timing.lifespan_ms`.
    NoLifespan,
    /// The deadline is below the period: no deadline (§2.5).
    BelowPeriod,
    /// Read across a constrained face (§2.8).
    AcrossFace,
    /// A delivery lost, or the reader's receive path stalled (§2.7).
    Lost,
    /// A miss while the owner was present.
    Missed,
    /// A miss only while the owner was not present; or the period not
    /// kept, the owner not present throughout.
    OwnerNotPresent,
    /// The member ended in the window.
    Ended,
    /// The member was never held in the window, or not heard.
    NotHeard,
    /// Held for the deadline or less, or a span under three periods.
    WindowTooShort,
    /// No miss, held long enough.
    Met,
    /// The period kept (§2.7).
    Kept,
    /// The period not kept.
    NotKept,
    /// A sample expired (§2.6).
    Expired,
    /// A sample live.
    Live,
    /// No ε stated: nothing discarded.
    NoBound,
    /// The sample carries no stamp.
    Unstamped,
    /// No `meta.zid` held of an instance of the provider's address.
    Unattributed,
    /// The stamp's id is no owner's `meta.zid`.
    ForeignStamp,
    /// The stamp's clock was seen ahead of the reader's by more than ε.
    ClockDisagrees,
    /// No sample of the member.
    NoSample,
}

impl Reason {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Kind => "kind",
            Self::Invalid => "invalid",
            Self::NoDeadline => "no_deadline",
            Self::NoPeriod => "no_period",
            Self::NoLifespan => "no_lifespan",
            Self::BelowPeriod => "below_period",
            Self::AcrossFace => "across_face",
            Self::Lost => "lost",
            Self::Missed => "missed",
            Self::OwnerNotPresent => "owner_not_present",
            Self::Ended => "ended",
            Self::NotHeard => "not_heard",
            Self::WindowTooShort => "window_too_short",
            Self::Met => "met",
            Self::Kept => "kept",
            Self::NotKept => "not_kept",
            Self::Expired => "expired",
            Self::Live => "live",
            Self::NoBound => "no_bound",
            Self::Unstamped => "unstamped",
            Self::Unattributed => "unattributed",
            Self::ForeignStamp => "foreign_stamp",
            Self::ClockDisagrees => "clock_disagrees",
            Self::NoSample => "no_sample",
        }
    }

    /// Parses the fixture's token.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        [
            Self::Kind,
            Self::Invalid,
            Self::NoDeadline,
            Self::NoPeriod,
            Self::NoLifespan,
            Self::BelowPeriod,
            Self::AcrossFace,
            Self::Lost,
            Self::Missed,
            Self::OwnerNotPresent,
            Self::Ended,
            Self::NotHeard,
            Self::WindowTooShort,
            Self::Met,
            Self::Kept,
            Self::NotKept,
            Self::Expired,
            Self::Live,
            Self::NoBound,
            Self::Unstamped,
            Self::Unattributed,
            Self::ForeignStamp,
            Self::ClockDisagrees,
            Self::NoSample,
        ]
        .into_iter()
        .find(|r| r.as_str() == s)
    }

    /// The reason in words, for a tool's message.
    #[must_use]
    pub fn says(self) -> &'static str {
        match self {
            Self::Kind => "only a stream has a period, a deadline or a lifespan",
            Self::Invalid => "its timing declaration is invalid",
            Self::NoDeadline => "no timing.deadline_ms",
            Self::NoPeriod => "no timing.period_ms",
            Self::NoLifespan => "no timing.lifespan_ms",
            Self::BelowPeriod => "its deadline is below its period, so it is no deadline",
            Self::AcrossFace => "read across a constrained face, where presence does not cross",
            Self::Lost => "this reader lost a delivery, or stalled, in the window",
            Self::Missed => "nothing arrived within its deadline while its owner was present",
            Self::OwnerNotPresent => "its owner was not present throughout",
            Self::Ended => "the member ended in the window",
            Self::NotHeard => "the member was not heard in the window",
            Self::WindowTooShort => "the window was too short to tell",
            Self::Met => "every arrival within its deadline",
            Self::Kept => "published at its period",
            Self::NotKept => "fewer samples arrived than its period promises",
            Self::Expired => "a sample arrived older than its lifespan",
            Self::Live => "every sample arrived within its lifespan",
            Self::NoBound => {
                "the deployment states no bound on its clocks, so nothing is discarded"
            }
            Self::Unstamped => "the sample carries no stamp",
            Self::Unattributed => {
                "no meta.zid is held for the provider, so a stamp is unattributable"
            }
            Self::ForeignStamp => "the stamp is not the owner's: a router's, or another session's",
            Self::ClockDisagrees => {
                "the stamp's clock was seen ahead of this reader's by more than the stated bound"
            }
            Self::NoSample => "no sample was read",
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One sample's lifespan judgement (§2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    pub verdict: SampleVerdict,
    pub reason: Reason,
}

impl Sample {
    const fn new(verdict: SampleVerdict, reason: Reason) -> Self {
        Self { verdict, reason }
    }

    /// Whether a consumer discards the sample (§2.6): an expired one only.
    #[must_use]
    pub fn discarded(&self) -> bool {
        self.verdict == SampleVerdict::Expired
    }
}

/// One reader's lifespan judgement over one reading of one provider's
/// resource (§2.6): the subscription's lifetime, or a tool's window. It
/// remembers, per stamping clock, the stamp furthest ahead, so that a clock
/// that falsified the deployment's word stays unobservable for the rest of
/// the reading.
#[derive(Debug, Clone)]
pub struct LifespanReader {
    premise: Result<Duration, Sample>,
    epsilon: Option<Duration>,
    owners: BTreeSet<u128>,
    clocks: BTreeMap<u128, ClockMeasure>,
}

impl LifespanReader {
    /// A reader of a resource of `kind` and declaration `d`, under the
    /// deployment's stated bound `epsilon` (`None` when none is stated,
    /// never the HLC's delta), attributing stamps to the `owners`' zids:
    /// the `meta.zid` of each instance of the provider's address it holds a
    /// descriptor of.
    pub fn new(
        kind: Kind,
        d: &Declaration,
        epsilon: Option<Duration>,
        owners: impl IntoIterator<Item = u128>,
    ) -> Self {
        let premise = if kind != Kind::Stream {
            Err(Sample::new(SampleVerdict::NotThisProfiles, Reason::Kind))
        } else {
            match d {
                Declaration::Invalid(_) => {
                    Err(Sample::new(SampleVerdict::Unobservable, Reason::Invalid))
                }
                Declaration::Declared(Declared {
                    lifespan: Some(l), ..
                }) => Ok(*l),
                _ => Err(Sample::new(SampleVerdict::NotAsked, Reason::NoLifespan)),
            }
        };
        Self {
            premise,
            epsilon,
            owners: owners.into_iter().collect(),
            clocks: BTreeMap::new(),
        }
    }

    /// The owners' zids now held, when the reader reads another descriptor.
    pub fn set_owners(&mut self, owners: impl IntoIterator<Item = u128>) {
        self.owners = owners.into_iter().collect();
    }

    /// Judges one sample at its arrival (§2.6), by the first step that
    /// holds: the kind, the declaration, the key, ε, the stamp, its
    /// attribution, its clock's agreement with ε, then the band.
    pub fn judge(&mut self, stamp: Option<Stamped>) -> Sample {
        use Reason as R;
        use SampleVerdict as V;
        let lifespan = match self.premise {
            Ok(l) => l,
            Err(s) => return s,
        };
        let Some(eps) = self.epsilon else {
            return Sample::new(V::Unobservable, R::NoBound);
        };
        let Some(stamp) = stamp else {
            return Sample::new(V::Unobservable, R::Unstamped);
        };
        if self.owners.is_empty() {
            return Sample::new(V::Unobservable, R::Unattributed);
        }
        if !self.owners.contains(&stamp.id) {
            return Sample::new(V::Unobservable, R::ForeignStamp);
        }
        let m = self
            .clocks
            .entry(stamp.id)
            .and_modify(|m| m.record(stamp.age))
            .or_insert_with(|| ClockMeasure::new(stamp.age));
        let e = eps.as_nanos() as i128;
        if m.most_ahead().nanos() < -e {
            return Sample::new(V::Unobservable, R::ClockDisagrees);
        }
        if stamp.age.nanos() - e > lifespan.as_nanos() as i128 {
            Sample::new(V::Expired, R::Expired)
        } else {
            Sample::new(V::Live, R::Live)
        }
    }
}

// --- §2.7: the period over a window -----------------------------------------------

/// What one member's arrivals over a window say of its period (§2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PeriodVerdict {
    /// Not known, and no arrival in the window: no member to ask about.
    NotHeard,
    /// A span under three periods.
    TooShort,
    /// (arrivals + 2) × period is at least the span.
    Kept,
    NotKept,
}

impl PeriodVerdict {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotHeard => "not_heard",
            Self::TooShort => "too_short",
            Self::Kept => "kept",
            Self::NotKept => "not_kept",
        }
    }

    /// Parses the fixture's token.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "not_heard" => Self::NotHeard,
            "too_short" => Self::TooShort,
            "kept" => Self::Kept,
            "not_kept" => Self::NotKept,
            _ => return None,
        })
    }
}

/// [`period_window`]'s answer: the verdict, the arrivals in the span, and
/// the span's length (`None` when not heard).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeriodReading {
    pub verdict: PeriodVerdict,
    pub arrivals: u64,
    pub span: Option<Duration>,
}

/// One member's period over a window from `from` to `to` (§2.7), on the
/// reader's monotonic clock: the span starts at `from` for a `known`
/// member, else at its first arrival within the window, and ends at
/// `ended` when the member ended within the window, else at `to`; its
/// arrivals are those within the span, both ends included.
#[must_use]
pub fn period_window(
    period: Duration,
    from: Mono,
    to: Mono,
    known: bool,
    ended: Option<Mono>,
    arrivals: &[Mono],
) -> PeriodReading {
    let start = if known {
        Some(from)
    } else {
        arrivals.iter().copied().find(|t| (from..=to).contains(t))
    };
    let Some(start) = start else {
        return PeriodReading {
            verdict: PeriodVerdict::NotHeard,
            arrivals: 0,
            span: None,
        };
    };
    let end = ended.unwrap_or(to);
    let span = end.saturating_since(start);
    let n = arrivals
        .iter()
        .filter(|t| (start..=end).contains(*t))
        .count() as u64;
    let (p, s) = (period.as_nanos(), span.as_nanos());
    let verdict = if s < 3 * p {
        PeriodVerdict::TooShort
    } else if (u128::from(n) + 2) * p >= s {
        PeriodVerdict::Kept
    } else {
        PeriodVerdict::NotKept
    };
    PeriodReading {
        verdict,
        arrivals: n,
        span: Some(span),
    }
}

// --- §5: what a tool may conclude ---------------------------------------------------

/// A question of §5, answered in the three states: a pole of each
/// established kind, and the unestablished kinds apart (core O5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Answer {
    Yes,
    No,
    Unobservable,
    NotAsked,
    NotThisProfiles,
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
            Self::NotThisProfiles => "not_this_profiles",
        }
    }
}

impl fmt::Display for Answer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An answer and its reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Judged {
    pub answer: Answer,
    pub reason: Reason,
}

impl Judged {
    const fn new(answer: Answer, reason: Reason) -> Self {
        Self { answer, reason }
    }
}

/// What a tool's window establishes of every member (§2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Window {
    /// Read through a constrained face (§2.8).
    pub across_face: bool,
    /// The tool knows it lost a delivery of the member, or its receive path
    /// stalled, in the window.
    pub lost: bool,
    /// The owner present (§0) at every instant of the window.
    pub owner_present: bool,
}

/// The tracker's account of one member over the window (§2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeadlineAccount {
    /// How long the member was held in the window.
    pub held: Duration,
    /// Misses reported while the owner was present.
    pub missed_present: u32,
    /// Misses reported while it was unconfirmed, absent or unseen.
    pub missed_not_present: u32,
    /// Whether the member ended in the window.
    pub ended: bool,
}

impl DeadlineAccount {
    /// Counts a miss the tracker reported.
    pub fn count(&mut self, r: &Report) {
        if let Report::Missed { owner, .. } = r {
            if *owner == OwnerState::Present {
                self.missed_present += 1;
            } else {
                self.missed_not_present += 1;
            }
        }
        if matches!(r, Report::Ended { .. }) {
            self.ended = true;
        }
    }
}

/// How long a tool held one member in its window, so far (§2.7): see
/// [`Holding`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HeldSpan {
    /// The time counted while the count stood stopped.
    counted: Duration,
    /// The instant the running count started, while it runs.
    from: Option<Mono>,
    /// Its first end in the window: the count stops there for good
    /// (F-280).
    ended: Option<Mono>,
}

impl HeldSpan {
    /// The time held up to `at`, a running count included.
    #[must_use]
    pub fn held_to(&self, at: Mono) -> Duration {
        self.counted + self.from.map_or(Duration::ZERO, |f| at.saturating_since(f))
    }

    /// Its first end in the window, when it ended.
    #[must_use]
    pub fn ended(&self) -> Option<Mono> {
        self.ended
    }

    /// Whether the count runs.
    #[must_use]
    pub fn counting(&self) -> bool {
        self.from.is_some()
    }

    fn run(&mut self, at: Mono) {
        if self.from.is_none() && self.ended.is_none() {
            self.from = Some(at);
        }
    }

    fn stop(&mut self, at: Mono) {
        if let Some(f) = self.from.take() {
            self.counted += at.saturating_since(f);
        }
    }
}

/// How long a tool held each member of one tracker in its window (§2.7;
/// text 0.5, F-278, F-280): the account [`DeadlineAccount::held`] reads.
///
/// Handed the tracker after each event it observed in the window, with the
/// reports it made, a member counts from the instant the tracker holds it
/// heard, missed or waited for, and stops at the instant it holds it
/// otherwise, or not at all:
/// - **a known member never heard counts only while it is waited for**
///   (§2.5): its owner's absence stops the count, and its presence again
///   starts it (F-278), so a window whose owner left before the miss was
///   due holds it for less than the deadline, and §5 answers
///   `window_too_short`, never `met`;
/// - **its first `ended` stops it for good** (F-280): a member heard again
///   after it ended is read over its first life, its held time and its span
///   ending at that first end.
///
/// The window runs from the tracker's start (§2.7: the window's start is
/// its subscription's start) to [`Holding::close`]: before its start a
/// tracker waits for nothing and takes no arrival, so nothing counts.
#[derive(Debug, Clone, Default)]
pub struct Holding {
    spans: BTreeMap<String, HeldSpan>,
    closed: bool,
}

impl Holding {
    /// No member held yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// After `tracker` observed an event at `at` and made `reports`: each
    /// member's count moved to where the tracker leaves it. Nothing after
    /// [`Holding::close`].
    pub fn observe(&mut self, tracker: &Tracker, reports: &[Report], at: Mono) {
        if self.closed {
            return;
        }
        for r in reports {
            if let Report::Ended { member, .. } = r {
                let s = self.spans.entry(member.clone()).or_default();
                s.stop(at);
                s.ended.get_or_insert(at);
            }
        }
        for name in tracker.held() {
            let counts = matches!(
                tracker.state(name),
                Some(
                    MemberState::Heard { .. }
                        | MemberState::Missed { .. }
                        | MemberState::Waiting { .. }
                )
            );
            if !counts && !self.spans.contains_key(name) {
                // Held and not counted: listed, at nothing.
                self.spans.insert(name.to_owned(), HeldSpan::default());
                continue;
            }
            let s = self.spans.entry(name.to_owned()).or_default();
            if counts {
                s.run(at);
            } else {
                s.stop(at);
            }
        }
        for (name, s) in &mut self.spans {
            if s.counting() && tracker.state(name).is_none() {
                s.stop(at);
            }
        }
    }

    /// The window's end at `at`: every count stops there, and nothing moves
    /// after.
    pub fn close(&mut self, at: Mono) {
        for s in self.spans.values_mut() {
            s.stop(at);
        }
        self.closed = true;
    }

    /// Every member the tracker held in the window, in member order (byte
    /// order of the keys), with its span.
    pub fn spans(&self) -> impl Iterator<Item = (&str, &HeldSpan)> {
        self.spans.iter().map(|(k, s)| (k.as_str(), s))
    }

    /// `member`'s span, when the tracker held it in the window.
    #[must_use]
    pub fn span(&self, member: &str) -> Option<&HeldSpan> {
        self.spans.get(member)
    }
}

/// The premises every question reads first: the kind, then the declaration.
fn premise(kind: Kind, d: &Declaration) -> Result<Declared, Judged> {
    if kind != Kind::Stream {
        return Err(Judged::new(Answer::NotThisProfiles, Reason::Kind));
    }
    match d {
        Declaration::Invalid(_) => Err(Judged::new(Answer::Unobservable, Reason::Invalid)),
        Declaration::Declared(x) => Ok(*x),
        Declaration::NotThisProfiles | Declaration::Ignored => Ok(Declared::default()),
    }
}

/// §5's first question: did this member meet its deadline over the window?
/// No is the finding.
#[must_use]
pub fn deadline_met(kind: Kind, d: &Declaration, w: &Window, a: &DeadlineAccount) -> Judged {
    use Answer as A;
    use Reason as R;
    let x = match premise(kind, d) {
        Ok(x) => x,
        Err(j) => return j,
    };
    let Some(deadline) = x.deadline else {
        return Judged::new(A::NotAsked, R::NoDeadline);
    };
    if x.below_period() {
        return Judged::new(A::Unobservable, R::BelowPeriod);
    }
    if w.across_face {
        return Judged::new(A::Unobservable, R::AcrossFace);
    }
    if a.missed_present > 0 {
        return if w.lost {
            Judged::new(A::Unobservable, R::Lost)
        } else {
            Judged::new(A::No, R::Missed)
        };
    }
    if a.missed_not_present > 0 {
        return Judged::new(A::Unobservable, R::OwnerNotPresent);
    }
    if a.ended {
        return Judged::new(A::NotAsked, R::Ended);
    }
    if a.held.is_zero() {
        return Judged::new(A::NotAsked, R::NotHeard);
    }
    if a.held <= deadline {
        return Judged::new(A::Unobservable, R::WindowTooShort);
    }
    Judged::new(A::Yes, R::Met)
}

/// §5's second question: did the owner keep this member's period over the
/// window? No is the finding.
#[must_use]
pub fn period_kept(kind: Kind, d: &Declaration, w: &Window, p: PeriodVerdict) -> Judged {
    use Answer as A;
    use Reason as R;
    let x = match premise(kind, d) {
        Ok(x) => x,
        Err(j) => return j,
    };
    if x.period.is_none() {
        return Judged::new(A::NotAsked, R::NoPeriod);
    }
    if w.across_face {
        return Judged::new(A::Unobservable, R::AcrossFace);
    }
    match p {
        PeriodVerdict::NotHeard => Judged::new(A::NotAsked, R::NotHeard),
        PeriodVerdict::TooShort => Judged::new(A::Unobservable, R::WindowTooShort),
        PeriodVerdict::Kept => Judged::new(A::Yes, R::Kept),
        PeriodVerdict::NotKept if w.lost => Judged::new(A::Unobservable, R::Lost),
        PeriodVerdict::NotKept if !w.owner_present => {
            Judged::new(A::Unobservable, R::OwnerNotPresent)
        }
        PeriodVerdict::NotKept => Judged::new(A::No, R::NotKept),
    }
}

/// §5's third question: did a sample of this member arrive expired? Yes is
/// the finding.
#[must_use]
pub fn lifespan_expired(kind: Kind, d: &Declaration, samples: &[Sample]) -> Judged {
    use Answer as A;
    use Reason as R;
    let x = match premise(kind, d) {
        Ok(x) => x,
        Err(j) => return j,
    };
    if x.lifespan.is_none() {
        return Judged::new(A::NotAsked, R::NoLifespan);
    }
    if samples.iter().any(|s| s.verdict == SampleVerdict::Expired) {
        return Judged::new(A::Yes, R::Expired);
    }
    if let Some(s) = samples
        .iter()
        .find(|s| s.verdict == SampleVerdict::Unobservable)
    {
        return Judged::new(A::Unobservable, s.reason);
    }
    if samples.iter().any(|s| s.verdict == SampleVerdict::Live) {
        return Judged::new(A::No, R::Live);
    }
    Judged::new(A::Unobservable, R::NoSample)
}

/// §5's three questions, each with the pole that is its finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Question {
    /// Did this member meet its deadline? No is the finding.
    Deadline,
    /// Did the owner keep its period? No is the finding.
    Period,
    /// Did a sample arrive expired? Yes is the finding.
    Lifespan,
}

impl Question {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deadline => "deadline",
            Self::Period => "period",
            Self::Lifespan => "lifespan",
        }
    }

    /// Parses the fixture's token.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "deadline" => Self::Deadline,
            "period" => Self::Period,
            "lifespan" => Self::Lifespan,
            _ => return None,
        })
    }

    /// The answer that is the finding.
    #[must_use]
    pub fn finding(self) -> Answer {
        match self {
            Self::Deadline | Self::Period => Answer::No,
            Self::Lifespan => Answer::Yes,
        }
    }
}

/// §5's roll-up of one question over a resource or a run, read by its
/// finding (text 0.2, F-264), a member not asked, or not this profile's,
/// set aside: the finding when any member's answer is the finding; the
/// other pole when every member's answer is that pole or set aside, and at
/// least one is that pole; when every member is set aside, and there is at
/// least one, not this profile's when every one is, else not asked (text
/// 0.4, F-277); unobservable otherwise, an unobservable member and none the
/// finding, or no member at all.
#[must_use]
pub fn roll_up(question: Question, answers: impl IntoIterator<Item = Answer>) -> Answer {
    let finding = question.finding();
    let other = if finding == Answer::Yes {
        Answer::No
    } else {
        Answer::Yes
    };
    let (mut poles, mut unestablished) = (0u64, false);
    let (mut aside, mut not_asked) = (0u64, false);
    for a in answers {
        if a == finding {
            return finding;
        }
        match a {
            Answer::NotAsked => {
                aside += 1;
                not_asked = true;
            }
            Answer::NotThisProfiles => aside += 1,
            a if a == other => poles += 1,
            _ => unestablished = true,
        }
    }
    if unestablished {
        Answer::Unobservable
    } else if poles > 0 {
        other
    } else if aside == 0 {
        Answer::Unobservable
    } else if not_asked {
        Answer::NotAsked
    } else {
        Answer::NotThisProfiles
    }
}

/// §5's roll-up with its reason (text 0.5, F-279): [`roll_up`]'s answer
/// over `members`, each its key, its answer and its reason, and the first
/// member giving that answer in byte order of the keys, with its reason,
/// whatever order they come in; `None` over no member. Over one member or
/// more the answer is always some member's: `unobservable` is the roll-up's
/// only when a member's is, or when there is none.
pub fn roll_up_first<K: AsRef<str>, R>(
    question: Question,
    members: impl IntoIterator<Item = (K, Answer, R)>,
) -> (Answer, Option<(K, R)>) {
    let members: Vec<(K, Answer, R)> = members.into_iter().collect();
    let answer = roll_up(question, members.iter().map(|(_, a, _)| *a));
    let first = members
        .into_iter()
        .filter(|(_, a, _)| *a == answer)
        .min_by(|(x, _, _), (y, _, _)| x.as_ref().as_bytes().cmp(y.as_ref().as_bytes()))
        .map(|(k, _, r)| (k, r));
    (answer, first)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ann(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect()
    }

    fn ms(n: u64) -> Mono {
        Mono::after(Duration::from_millis(n))
    }

    #[test]
    fn a_value_is_a_whole_number_of_milliseconds_above_zero() {
        let d = |v: Value| declaration(Kind::Stream, &ann(&[(DEADLINE_MS, v)]));
        assert_eq!(
            d(json!(100)).declared().and_then(|x| x.deadline),
            Some(Duration::from_millis(100))
        );
        assert_eq!(
            d(json!(100.0)).declared().and_then(|x| x.deadline),
            Some(Duration::from_millis(100))
        );
        for bad in [
            json!(0),
            json!(-0.0),
            json!(-1),
            json!(1.5),
            json!(MAX_MS + 1),
        ] {
            assert_eq!(d(bad), Declaration::Invalid(vec![Invalid::DeadlineValue]));
        }
        assert_eq!(
            declaration(Kind::State, &ann(&[(DEADLINE_MS, json!("x"))])),
            Declaration::Ignored,
            "the kind first"
        );
    }

    #[test]
    fn the_bound_has_a_millisecond_floor() {
        assert_eq!(
            detection_bound(Duration::from_millis(100)),
            Duration::from_millis(2)
        );
        assert_eq!(detection_bound(Duration::from_millis(30)), BOUND_FLOOR);
        assert_eq!(
            detection_bound(Duration::from_millis(75)),
            Duration::from_micros(1500)
        );
    }

    #[test]
    fn a_paused_commander_is_missed_once_then_met() {
        let mut t = Tracker::new(Duration::from_millis(100), Some(1), ["cmd"]);
        t.observe(ms(0), Event::Owner(OwnerState::Present));
        t.observe(ms(0), Event::Start);
        let arrive = |t: &mut Tracker, at| {
            t.observe(
                ms(at),
                Event::Arrival {
                    member: "cmd",
                    expired: false,
                },
            )
        };
        assert!(arrive(&mut t, 10).is_empty());
        assert_eq!(t.next_due(), Some(ms(110)));
        let r = t.observe(ms(111), Event::Tick);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].as_str(), "missed");
        assert!(t.observe(ms(500), Event::Tick).is_empty(), "edge-triggered");
        let r = arrive(&mut t, 600);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].as_str(), "met");
    }

    #[test]
    fn a_clock_ahead_beyond_epsilon_stays_unobservable() {
        let d = declaration(Kind::Stream, &ann(&[(LIFESPAN_MS, json!(100))]));
        let mut r = LifespanReader::new(Kind::Stream, &d, Some(Duration::from_millis(10)), [7]);
        let at = |s: f64| {
            Some(Stamped {
                age: StampAge::from_secs_f64(s),
                id: 7,
            })
        };
        assert_eq!(r.judge(at(0.05)).reason, Reason::Live);
        assert_eq!(r.judge(at(-0.05)).reason, Reason::ClockDisagrees);
        assert_eq!(r.judge(at(0.05)).reason, Reason::ClockDisagrees);
        assert_eq!(
            r.judge(Some(Stamped {
                age: StampAge::from_secs_f64(0.5),
                id: 8
            }))
            .reason,
            Reason::ForeignStamp
        );
    }

    #[test]
    fn the_roll_up_reads_each_questions_finding() {
        let d = Question::Deadline;
        assert_eq!(roll_up(d, [Answer::Yes, Answer::NotAsked]), Answer::Yes);
        assert_eq!(
            roll_up(d, [Answer::Yes, Answer::Unobservable]),
            Answer::Unobservable
        );
        assert_eq!(roll_up(d, [Answer::Unobservable, Answer::No]), Answer::No);
        assert_eq!(roll_up(d, []), Answer::Unobservable);
        let l = Question::Lifespan;
        assert_eq!(roll_up(l, [Answer::No, Answer::Yes]), Answer::Yes, "F-264");
        assert_eq!(roll_up(l, [Answer::No, Answer::No]), Answer::No);
        // Every member set aside (text 0.4, F-277): nobody put the question.
        assert_eq!(roll_up(l, [Answer::NotAsked]), Answer::NotAsked);
        assert_eq!(
            roll_up(l, [Answer::NotThisProfiles, Answer::NotAsked]),
            Answer::NotAsked
        );
        assert_eq!(
            roll_up(Question::Period, [Answer::NotThisProfiles]),
            Answer::NotThisProfiles
        );
        assert_eq!(
            roll_up(d, [Answer::NotAsked, Answer::Unobservable]),
            Answer::Unobservable,
            "an unobservable member is never set aside"
        );
    }

    /// Text 0.5, F-279: the first member giving the answer, in byte order
    /// of the keys, whatever order they come in; over no member, none.
    #[test]
    fn a_roll_ups_reason_is_its_first_members_in_byte_order() {
        let d = Question::Deadline;
        let (a, first) = roll_up_first(
            d,
            [
                ("axis/a", Answer::Unobservable, Reason::OwnerNotPresent),
                ("axis/B", Answer::Unobservable, Reason::WindowTooShort),
            ],
        );
        assert_eq!(
            (a, first),
            (
                Answer::Unobservable,
                Some(("axis/B", Reason::WindowTooShort))
            ),
            "B (0x42) before a (0x61): bytes, never a locale"
        );
        let (a, first) = roll_up_first(
            d,
            [
                ("axis/a", Answer::Unobservable, Reason::Lost),
                ("cmd", Answer::No, Reason::Missed),
            ],
        );
        assert_eq!(
            (a, first),
            (Answer::No, Some(("cmd", Reason::Missed))),
            "the first member giving the answer, not the first member"
        );
        let (a, first) = roll_up_first(
            Question::Lifespan,
            [
                ("a", Answer::NotThisProfiles, Reason::Kind),
                ("b", Answer::NotAsked, Reason::NoLifespan),
            ],
        );
        assert_eq!(
            (a, first),
            (Answer::NotAsked, Some(("b", Reason::NoLifespan)))
        );
        let (a, first) = roll_up_first(d, std::iter::empty::<(&str, Answer, Reason)>());
        assert_eq!((a, first), (Answer::Unobservable, None));
    }

    fn holding_of(t: &mut Tracker, h: &mut Holding, at: u64, e: Event<'_>) -> Vec<Report> {
        let r = t.observe(ms(at), e);
        h.observe(t, &r, ms(at));
        r
    }

    /// Text 0.5, F-278: a known member never heard is held only while it
    /// is waited for; its owner's absence stops the count, its presence
    /// again starts it.
    #[test]
    fn a_known_member_is_held_only_while_it_is_waited_for() {
        let mut t = Tracker::new(Duration::from_millis(500), Some(1), ["cmd"]);
        let mut h = Holding::new();
        holding_of(&mut t, &mut h, 0, Event::Owner(OwnerState::Present));
        holding_of(&mut t, &mut h, 0, Event::Start);
        holding_of(&mut t, &mut h, 150, Event::Owner(OwnerState::Unconfirmed));
        holding_of(&mut t, &mut h, 330, Event::Owner(OwnerState::Absent));
        let r = holding_of(&mut t, &mut h, 3000, Event::Tick);
        assert!(r.is_empty(), "no miss: the wait stopped before its due");
        h.close(ms(3000));
        assert_eq!(
            h.span("cmd").map(|s| s.held_to(ms(3000))),
            Some(Duration::from_millis(330)),
            "the unconfirmed while waits on (§2.5), the absence stops it"
        );
        // Present again: the count starts again.
        let mut t = Tracker::new(Duration::from_millis(500), Some(1), ["cmd"]);
        let mut h = Holding::new();
        holding_of(&mut t, &mut h, 0, Event::Owner(OwnerState::Present));
        holding_of(&mut t, &mut h, 0, Event::Start);
        holding_of(&mut t, &mut h, 330, Event::Owner(OwnerState::Absent));
        holding_of(&mut t, &mut h, 1000, Event::Owner(OwnerState::Present));
        let r = holding_of(&mut t, &mut h, 1501, Event::Tick);
        assert_eq!(r.len(), 1, "{r:?}");
        h.close(ms(2000));
        assert_eq!(
            h.span("cmd").map(|s| s.held_to(ms(2000))),
            Some(Duration::from_millis(1330))
        );
    }

    /// Text 0.5, F-280: a member that ends and is heard again is held over
    /// its first life alone.
    #[test]
    fn a_member_heard_again_is_held_over_its_first_life() {
        let mut t =
            Tracker::new(Duration::from_millis(500), Some(4), Vec::<String>::new()).with_epoch();
        let mut h = Holding::new();
        holding_of(&mut t, &mut h, 0, Event::Owner(OwnerState::Present));
        holding_of(&mut t, &mut h, 0, Event::Start);
        holding_of(&mut t, &mut h, 0, Event::MemberSeen("axis/x"));
        let arrival = Event::Arrival {
            member: "axis/x",
            expired: false,
        };
        for at in [100, 500, 900] {
            holding_of(&mut t, &mut h, at, arrival);
        }
        let r = holding_of(&mut t, &mut h, 1100, Event::MemberGone("axis/x"));
        assert_eq!(r.len(), 1, "ended, and no miss: {r:?}");
        holding_of(&mut t, &mut h, 2000, Event::MemberSeen("axis/x"));
        holding_of(&mut t, &mut h, 2000, arrival);
        holding_of(&mut t, &mut h, 4000, arrival);
        h.close(ms(5000));
        let s = h.span("axis/x").copied().expect("held");
        assert_eq!(s.ended(), Some(ms(1100)));
        assert_eq!(s.held_to(ms(5000)), Duration::from_millis(1000));
    }
}
