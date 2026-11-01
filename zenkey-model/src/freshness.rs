//! `freshness.v1`: how long a value stays current
//! (`spec/profiles/freshness/v1.md`, #720).
//!
//! The profile's session-free half: the horizon read from a resource's
//! annotations (§2.1–§2.3), and the pure judgement of one observation of a
//! member (§2.5–§2.8), three states and a reason, with the rule that
//! combines several observations (§2.7).
//!
//! ```text
//! horizon(kind, annotations) ─▶ Undeclared │ Ignored │ Invalid │ Never │ Within(ttl)
//! judge(horizon, observation) ─▶ fresh │ stale │ unobservable │ not_asked, + reason
//! combine(judged…)            ─▶ fresh > stale > unobservable > not_asked
//! ```
//!
//! There is no I/O and no clock here. A reader measures what an observation
//! holds (a delivery's age on its monotonic clock, a reply's stamp age
//! against its wall clock, whether that clock is trusted to the delta) and
//! hands it over as values. The owner's re-puts (§2.4) are the runtime's.
//!
//! **A reader's view** ([`View`], #978's F2): the members of one owner's
//! collections for one interface as a reader heard them, and what that
//! owner's heartbeats (core §7.5, [`crate::heartbeat`]) confirmed. A
//! heartbeat whose section's digest is the view's confirms each member it
//! vouches for at its arrival, so [`View::observation`] ages a member from
//! the later of its last put and its last confirmation. No rule adopts the
//! heartbeat yet: the judgement is §2.5's, unchanged.

use std::collections::BTreeMap;
use std::fmt;
use std::time::{Duration, SystemTime};

use serde_json::Value;

use crate::authoring::Kind;
use crate::contract::Resource;
use crate::wire::Stamp;

/// The profile's id, as a contract lists it in `uses`.
pub const PROFILE: &str = "freshness.v1";

/// The profile's name, as an annotation key's prefix carries it.
pub const NAME: &str = "freshness";

/// The one annotation key (§4).
pub const TTL_S: &str = "freshness.ttl_s";

/// The published vocabulary (§4): the keys under `freshness.`, which W105
/// reads in place of core Appendix D's interim row (core 0.21).
pub const VOCABULARY: &[&str] = &["ttl_s"];

/// The largest horizon, in seconds: 2^53−1, the canonical domain's bound
/// (§2.1, core §9.5).
pub const MAX_TTL_S: u64 = (1 << 53) - 1;

/// The HLC's maximum delta unless the deployment configures another (core
/// §4.1): what a GET reader's trusted clock is trusted to (§2.6).
pub const DEFAULT_DELTA: Duration = Duration::from_millis(500);

/// A resource's staleness horizon, read from its resolved annotations
/// (§2.1–§2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Horizon {
    /// No `freshness.ttl_s`: freshness is not asked (§2.3).
    Undeclared,
    /// A horizon on an event or an operation, which have no freshness
    /// (§2.2): ignored.
    Ignored,
    /// A `freshness.ttl_s` that is not a horizon (§2.1), as JSON text: the
    /// resource's freshness is unobservable.
    Invalid(String),
    /// `0`: never stale (§2.3).
    Never,
    /// A horizon above 0.
    Within(Duration),
}

impl Horizon {
    /// The horizon, when it is one above 0.
    #[must_use]
    pub fn ttl(&self) -> Option<Duration> {
        match self {
            Self::Within(t) => Some(*t),
            _ => None,
        }
    }

    /// The owner's bound between two puts of a member (§2.4): ttl/2, when
    /// the horizon is above 0.
    #[must_use]
    pub fn refresh_bound(&self) -> Option<Duration> {
        self.ttl().map(|t| t / 2)
    }
}

impl fmt::Display for Horizon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Undeclared => f.write_str("no horizon"),
            Self::Ignored => f.write_str("a horizon this kind ignores"),
            Self::Invalid(v) => write!(f, "{v}, not a horizon"),
            Self::Never => f.write_str("ttl_s = 0, never stale"),
            Self::Within(t) => write!(f, "ttl_s = {}", t.as_secs()),
        }
    }
}

/// Reads the horizon of a resource of `kind` from its resolved annotations
/// (§2.1, §2.2): `[defaults]`, then `[defaults.<kind>]`, then its own, as
/// the contract model merges them (core §9.3).
#[must_use]
pub fn horizon(kind: Kind, annotations: &BTreeMap<String, Value>) -> Horizon {
    let Some(v) = annotations.get(TTL_S) else {
        return Horizon::Undeclared;
    };
    if matches!(kind, Kind::Event | Kind::Operation) {
        return Horizon::Ignored;
    }
    match seconds(v) {
        Some(0) => Horizon::Never,
        Some(n) => Horizon::Within(Duration::from_secs(n)),
        None => Horizon::Invalid(v.to_string()),
    }
}

/// [`horizon`] of a resource of a contract.
#[must_use]
pub fn horizon_of(r: &Resource) -> Horizon {
    horizon(r.kind, &r.annotations)
}

/// A whole number of seconds from 0 to 2^53−1, or `None` (§2.1). An
/// integral float is that integer, as the canonical form writes it: `60.0`
/// is 60, and `-0.0` is 0.
fn seconds(v: &Value) -> Option<u64> {
    let n = v.as_number()?;
    if let Some(u) = n.as_u64() {
        return (u <= MAX_TTL_S).then_some(u);
    }
    if n.is_i64() {
        return None;
    }
    let f = n.as_f64()?;
    // `-0.0 >= 0.0` holds: JCS writes it `0`.
    let max = MAX_TTL_S as f64;
    (f.is_finite() && f.fract() == 0.0 && f >= 0.0 && f <= max).then_some(f as u64)
}

/// A reply stamp's age at the instant a reader judges it: its clock minus
/// the stamp's time, negative when the stamp is ahead of the reader's clock
/// (§2.6). Nanoseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StampAge(i128);

impl StampAge {
    /// The age at `now` of a stamp whose time is `stamp`.
    #[must_use]
    pub fn between(stamp: SystemTime, now: SystemTime) -> Self {
        match now.duration_since(stamp) {
            Ok(d) => Self::behind(d),
            Err(e) => Self::ahead(e.duration()),
        }
    }

    /// A stamp `d` before the reader's clock.
    #[must_use]
    pub fn behind(d: Duration) -> Self {
        Self(d.as_nanos() as i128)
    }

    /// A stamp `d` ahead of the reader's clock.
    #[must_use]
    pub fn ahead(d: Duration) -> Self {
        Self(-(d.as_nanos() as i128))
    }

    /// Seconds, rounded to the nanosecond: how the fixtures write it.
    #[must_use]
    pub fn from_secs_f64(s: f64) -> Self {
        Self((s * 1e9).round() as i128)
    }

    #[must_use]
    pub fn as_secs_f64(self) -> f64 {
        self.0 as f64 / 1e9
    }

    /// Nanoseconds, negative when the stamp is ahead.
    #[must_use]
    pub fn nanos(self) -> i128 {
        self.0
    }
}

/// Whether a GET reader trusts its clock against the clock that stamped a
/// reply (§2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockTrust {
    /// The two agree within `delta`: on the deployment's word, or measured.
    Trusted { delta: Duration },
    /// No ground to trust them: a reply's age is unobservable.
    Untrusted,
}

/// What a reader measured of one stamping clock during a reading (§2.6,
/// ground 2, 0.2): the offset closest to its own clock, since transit only
/// adds to an offset, and the offset furthest ahead, since a stamp ahead of
/// the reader's clock by more than the delta proves the clocks disagree
/// whatever the transit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockMeasure {
    closest: StampAge,
    most_ahead: StampAge,
}

impl ClockMeasure {
    /// A first measurement.
    #[must_use]
    pub fn new(offset: StampAge) -> Self {
        Self {
            closest: offset,
            most_ahead: offset,
        }
    }

    /// Another live put's offset, in the same reading.
    pub fn record(&mut self, offset: StampAge) {
        if offset.nanos().unsigned_abs() < self.closest.nanos().unsigned_abs() {
            self.closest = offset;
        }
        if offset < self.most_ahead {
            self.most_ahead = offset;
        }
    }

    /// Two readers' measurements of one clock, as one reading's.
    #[must_use]
    pub fn merge(mut self, other: Self) -> Self {
        self.record(other.closest);
        self.record(other.most_ahead);
        self
    }

    /// The offset closest to the reader's clock.
    #[must_use]
    pub fn closest(&self) -> StampAge {
        self.closest
    }

    /// The offset furthest ahead (the most negative).
    #[must_use]
    pub fn most_ahead(&self) -> StampAge {
        self.most_ahead
    }

    /// §2.6, ground 2 (0.2): trusted when the closest offset is within
    /// `delta`, and no stamp was ahead by more than `delta`.
    #[must_use]
    pub fn trust(&self, delta: Duration) -> ClockTrust {
        let d = delta.as_nanos() as i128;
        if self.closest.nanos().unsigned_abs() <= delta.as_nanos() && self.most_ahead.nanos() >= -d
        {
            ClockTrust::Trusted { delta }
        } else {
            ClockTrust::Untrusted
        }
    }
}

impl ClockTrust {
    /// §2.6, ground 2 (0.2), over a reading's measurements of one clock:
    /// untrusted when there are none.
    #[must_use]
    pub fn from_offsets(offsets: impl IntoIterator<Item = StampAge>, delta: Duration) -> Self {
        let mut it = offsets.into_iter();
        let Some(first) = it.next() else {
            return Self::Untrusted;
        };
        let mut m = ClockMeasure::new(first);
        it.for_each(|o| m.record(o));
        m.trust(delta)
    }

    /// §2.6, ground 2: a live put whose stamp was `offset` from the reader's
    /// clock at receipt (the receipt minus the stamp) shows the clocks agree
    /// within `delta` when the offset is within it, either way.
    #[must_use]
    pub fn measured(offset: StampAge, delta: Duration) -> Self {
        if offset.0.unsigned_abs() <= delta.as_nanos() {
            Self::Trusted { delta }
        } else {
            Self::Untrusted
        }
    }
}

/// A subscription's last delivery of a member, and how long ago it arrived
/// on the subscriber's monotonic clock (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Last {
    /// A put (a change or a re-put), or a stream sample.
    Put(Duration),
    /// A delete: the member has no value.
    Delete(Duration),
}

/// The owner's reply for a member (§2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    /// A value, with its stamp's age at the instant of judgement, or `None`
    /// when the reply carries no stamp (which breaks core S2).
    Put { stamp_age: Option<StampAge> },
    /// A `reply_del`: the member is deleted.
    Delete,
}

/// One observation of one member, as a reader made it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observation {
    /// A subscription (§2.5): its last delivery of the member, how long it
    /// has listened since it was declared, and whether it lost deliveries
    /// (`complete` is false when it did).
    Subscribed {
        last: Option<Last>,
        listened: Duration,
        complete: bool,
    },
    /// A GET to the owner (§2.6): its reply for the member, or `None` when
    /// none came within the timeout, and the reader's trust in its clock
    /// against the reply's stamp.
    Got {
        reply: Option<Reply>,
        clock: ClockTrust,
    },
    /// An archive's answer: last-known, never current (§2.8).
    Archived,
}

/// The answer to "is this member's value fresh?" (§5). Stale is the
/// finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Verdict {
    Fresh,
    Stale,
    /// The question was put, and the observation could not answer it.
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
            Self::Fresh => "fresh",
            Self::Stale => "stale",
            Self::Unobservable => "unobservable",
            Self::NotAsked => "not_asked",
        }
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a verdict is what it is: the fixture's reason class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reason {
    /// Fresh: confirmed within the horizon (§2.5, §2.6).
    WithinHorizon,
    /// Fresh: ttl 0 (§2.3).
    NeverStale,
    /// Stale: the last confirmation is older than the horizon.
    BeyondHorizon,
    /// Stale: nothing delivered while listening longer than the horizon.
    NoDelivery,
    /// Unobservable: the `freshness.ttl_s` is not a horizon (§2.1).
    NotAHorizon,
    /// Unobservable: nothing delivered, and listened the horizon or less.
    ListenedTooShort,
    /// Unobservable: deliveries were lost, and none fresh arrived.
    Incomplete,
    /// Unobservable: the GET drew no reply for the member.
    Silent,
    /// Unobservable: the reply carries no stamp.
    NoStamp,
    /// Unobservable: the reader's clock is not trusted to the delta.
    ClockUntrusted,
    /// Unobservable: the stamp is ahead of the reader's clock beyond the
    /// delta.
    ClockDisagrees,
    /// Unobservable: the age is within the delta of the horizon.
    NearHorizon,
    /// Unobservable: no observation to judge.
    NoObservation,
    /// Not asked: no horizon (§2.3).
    NoHorizon,
    /// Not this profile's: an event or an operation (§2.2).
    Kind,
    /// Not asked: an archive's last-known value (§2.8).
    LastKnown,
    /// Not this profile's: the member is deleted.
    Deleted,
}

impl Reason {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WithinHorizon => "within_horizon",
            Self::NeverStale => "never_stale",
            Self::BeyondHorizon => "beyond_horizon",
            Self::NoDelivery => "no_delivery",
            Self::NotAHorizon => "not_a_horizon",
            Self::ListenedTooShort => "listened_too_short",
            Self::Incomplete => "incomplete",
            Self::Silent => "silent",
            Self::NoStamp => "no_stamp",
            Self::ClockUntrusted => "clock_untrusted",
            Self::ClockDisagrees => "clock_disagrees",
            Self::NearHorizon => "near_horizon",
            Self::NoObservation => "no_observation",
            Self::NoHorizon => "no_horizon",
            Self::Kind => "kind",
            Self::LastKnown => "last_known",
            Self::Deleted => "deleted",
        }
    }

    /// The reason in words, for a tool's message.
    #[must_use]
    pub fn says(self) -> &'static str {
        match self {
            Self::WithinHorizon => "confirmed within its horizon",
            Self::NeverStale => "ttl_s = 0, never stale",
            Self::BeyondHorizon => "not confirmed within its horizon",
            Self::NoDelivery => "nothing delivered for longer than its horizon",
            Self::NotAHorizon => "its freshness.ttl_s is not a horizon",
            Self::ListenedTooShort => {
                "nothing delivered, and not listened to for longer than its horizon"
            }
            Self::Incomplete => "deliveries were lost, and none within the horizon arrived",
            Self::Silent => "no value was read",
            Self::NoStamp => "the reply carries no stamp",
            Self::ClockUntrusted => {
                "this reader's clock is not trusted to the HLC delta against the stamping clock"
            }
            Self::ClockDisagrees => "the stamp is ahead of this reader's clock beyond the delta",
            Self::NearHorizon => "the age is within the delta of the horizon",
            Self::NoObservation => "nothing was observed",
            Self::NoHorizon => "no freshness.ttl_s, so freshness is not asked",
            Self::Kind => "an event or an operation has no freshness",
            Self::LastKnown => "an archive's last-known value is never current",
            Self::Deleted => "deleted, so no value",
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A verdict and its reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Judged {
    pub verdict: Verdict,
    pub reason: Reason,
}

impl Judged {
    const fn new(verdict: Verdict, reason: Reason) -> Self {
        Self { verdict, reason }
    }
}

/// Judges one observation of a member of a resource whose horizon is `h`,
/// by the order of §2.7: the horizon (§2.3, §2.2), an archive (§2.8), a
/// value that is no horizon (§2.1), then the observation (§2.5, §2.6).
#[must_use]
pub fn judge(h: &Horizon, o: &Observation) -> Judged {
    use Reason as R;
    use Verdict as V;
    match h {
        Horizon::Undeclared => return Judged::new(V::NotAsked, R::NoHorizon),
        Horizon::Ignored => return Judged::new(V::NotAsked, R::Kind),
        _ => {}
    }
    if matches!(o, Observation::Archived) {
        return Judged::new(V::NotAsked, R::LastKnown);
    }
    let ttl = match h {
        Horizon::Invalid(_) => return Judged::new(V::Unobservable, R::NotAHorizon),
        Horizon::Never => None,
        Horizon::Within(t) => Some(*t),
        Horizon::Undeclared | Horizon::Ignored => unreachable!("returned above"),
    };
    match (*o, ttl) {
        (Observation::Archived, _) => unreachable!("returned above"),
        // §2.5.
        (
            Observation::Subscribed {
                last: Some(Last::Delete(_)),
                ..
            },
            _,
        ) => Judged::new(V::NotAsked, R::Deleted),
        (Observation::Subscribed { .. }, None) => Judged::new(V::Fresh, R::NeverStale),
        (
            Observation::Subscribed {
                last,
                listened,
                complete,
            },
            Some(ttl),
        ) => match last {
            Some(Last::Put(a)) if a <= ttl => Judged::new(V::Fresh, R::WithinHorizon),
            _ if !complete => Judged::new(V::Unobservable, R::Incomplete),
            Some(_) => Judged::new(V::Stale, R::BeyondHorizon),
            None if listened > ttl => Judged::new(V::Stale, R::NoDelivery),
            None => Judged::new(V::Unobservable, R::ListenedTooShort),
        },
        // §2.6.
        (Observation::Got { reply: None, .. }, _) => Judged::new(V::Unobservable, R::Silent),
        (
            Observation::Got {
                reply: Some(Reply::Delete),
                ..
            },
            _,
        ) => Judged::new(V::NotAsked, R::Deleted),
        (Observation::Got { .. }, None) => Judged::new(V::Fresh, R::NeverStale),
        (
            Observation::Got {
                reply: Some(Reply::Put { stamp_age }),
                clock,
            },
            Some(ttl),
        ) => {
            let Some(age) = stamp_age else {
                return Judged::new(V::Unobservable, R::NoStamp);
            };
            let ClockTrust::Trusted { delta } = clock else {
                return Judged::new(V::Unobservable, R::ClockUntrusted);
            };
            let (a, t, d) = (age.0, ttl.as_nanos() as i128, delta.as_nanos() as i128);
            if a < -d {
                Judged::new(V::Unobservable, R::ClockDisagrees)
            } else if a + d <= t {
                Judged::new(V::Fresh, R::WithinHorizon)
            } else if a - d > t {
                Judged::new(V::Stale, R::BeyondHorizon)
            } else {
                Judged::new(V::Unobservable, R::NearHorizon)
            }
        }
    }
}

/// Combines the verdicts of several observations of one member (§2.7):
/// fresh when any is fresh, else stale when any is, else unobservable when
/// any is, else not asked. Among equal verdicts the first one's reason
/// stands. No observation at all is unobservable.
#[must_use]
pub fn combine(judged: impl IntoIterator<Item = Judged>) -> Judged {
    let rank = |v: Verdict| match v {
        Verdict::Fresh => 0,
        Verdict::Stale => 1,
        Verdict::Unobservable => 2,
        Verdict::NotAsked => 3,
    };
    judged
        .into_iter()
        .fold(None, |best: Option<Judged>, j| match best {
            Some(b) if rank(b.verdict) <= rank(j.verdict) => Some(b),
            _ => Some(j),
        })
        .unwrap_or(Judged::new(Verdict::Unobservable, Reason::NoObservation))
}

/// [`judge`] each observation, then [`combine`] them. With no observation,
/// §2.7's steps that need none still answer (0.2): no horizon is not
/// asked, an event or operation is not this profile's, a value that is no
/// horizon is unobservable; only a member with a horizon is unobservable
/// for want of an observation.
#[must_use]
pub fn judge_all(h: &Horizon, observations: &[Observation]) -> Judged {
    if observations.is_empty() {
        return match h {
            Horizon::Undeclared => Judged::new(Verdict::NotAsked, Reason::NoHorizon),
            Horizon::Ignored => Judged::new(Verdict::NotAsked, Reason::Kind),
            Horizon::Invalid(_) => Judged::new(Verdict::Unobservable, Reason::NotAHorizon),
            Horizon::Never | Horizon::Within(_) => {
                Judged::new(Verdict::Unobservable, Reason::NoObservation)
            }
        };
    }
    combine(observations.iter().map(|o| judge(h, o)))
}

// --- #978's F2: a reader's view, as heartbeats confirm it ----------------------

/// An instant on a reader's monotonic clock, as a [`View`] keeps it: a
/// delivery's or a heartbeat's arrival.
pub trait Arrival: Copy + Ord {
    /// The time from `earlier` to this instant, zero when `earlier` is
    /// later.
    fn since(self, earlier: Self) -> Duration;
}

impl Arrival for std::time::Instant {
    fn since(self, earlier: Self) -> Duration {
        self.saturating_duration_since(earlier)
    }
}

impl Arrival for crate::timing::Mono {
    fn since(self, earlier: Self) -> Duration {
        self.saturating_since(earlier)
    }
}

/// One member as a [`View`] holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Held<T> {
    collection: usize,
    /// The last mutation's in-band stamp; `None` for a put that carried
    /// none, which no digest can count, and for a delete.
    stamp: Option<Stamp>,
    deleted: bool,
    /// The last mutation's arrival.
    at: T,
    /// The last heartbeat that vouched for it, since its last mutation.
    confirmed: Option<T>,
}

/// What one section of a heartbeat did to the [`View`] (#978's design
/// §1.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SectionRead {
    /// The view does not follow the collection: nothing read.
    NotFollowed,
    /// Its map bit is 0: nothing confirmed there.
    Absent,
    /// The digest, or the live count, is not the view's: nothing
    /// confirmed in the collection.
    Diverged,
    /// The digest is the view's: each member vouched for is confirmed at
    /// the heartbeat's arrival, these keys in key order.
    Confirmed(Vec<String>),
}

impl SectionRead {
    /// `not_followed`, `absent`, `diverged` or `confirmed`.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotFollowed => "not_followed",
            Self::Absent => "absent",
            Self::Diverged => "diverged",
            Self::Confirmed(_) => "confirmed",
        }
    }
}

/// What a heartbeat did to the [`View`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    /// CH's tag is not the revision the view holds: nothing confirmed.
    Revision,
    /// The body was refused (core §7.5): nothing confirmed.
    Refused(crate::heartbeat::BodyRefusal),
    /// Its key's seq is not after the last heartbeat's, within one run:
    /// nothing confirmed, nothing counted.
    Older,
    /// Read, each collection's section in map order. `gap` says the key's
    /// seqs skipped since the last heartbeat: one or more were lost.
    Read {
        gap: bool,
        sections: Vec<SectionRead>,
    },
}

/// A reader's view of one owner's state for one interface, as its
/// deliveries, its reads and that owner's heartbeats make it (core §7.5's
/// heartbeat; #978's F2). No rule adopts the heartbeat yet, so nothing
/// here changes a [`judge`]: a heartbeat that confirms a member is a
/// confirmation of it, as a put is (§2.5), and [`View::observation`] ages
/// the member from the later of the two.
///
/// - **The members** are each collection's that the reader heard: a put
///   with its in-band stamp, a delete, or a read's answer
///   ([`View::replace`]). A put older than the one held, within one run,
///   is not taken.
/// - **A heartbeat** ([`View::apply`]) confirms, in each collection the
///   view follows ([`View::follow`]), each member vouched for, when the
///   section's digest is the view's and its live count is too. A reader
///   follows only a collection it hears whole: one whose template it
///   subscribes with no parameter bound.
/// - **A divergence** confirms nothing in its collection. After two in a
///   row the collection is due a read ([`View::reread_due`]), whose answer
///   replaces what the view holds.
/// - **A gap** in the heartbeat key's seqs is a heartbeat lost: the
///   view's members are incomplete until one arrives in sequence.
#[derive(Debug, Clone)]
pub struct View<T> {
    layout: crate::heartbeat::Layout,
    follows: Vec<bool>,
    members: BTreeMap<String, Held<T>>,
    diverged: Vec<u32>,
    last: Option<Stamp>,
    gap: bool,
    time: Option<crate::heartbeat::Time>,
    heartbeats: u64,
}

impl<T: Arrival> View<T> {
    /// An empty view of a revision's collections, following none.
    #[must_use]
    pub fn new(layout: crate::heartbeat::Layout) -> Self {
        let n = layout.collections.len();
        Self {
            layout,
            follows: vec![false; n],
            members: BTreeMap::new(),
            diverged: vec![0; n],
            last: None,
            gap: false,
            time: None,
            heartbeats: 0,
        }
    }

    /// The revision's layout.
    #[must_use]
    pub fn layout(&self) -> &crate::heartbeat::Layout {
        &self.layout
    }

    /// Follows the collection at `index`; `false` when there is none.
    pub fn follow(&mut self, index: usize) -> bool {
        match self.follows.get_mut(index) {
            Some(f) => {
                *f = true;
                true
            }
            None => false,
        }
    }

    /// Follows the collection named `<kind token>/<template>`; `false`
    /// when it is none.
    pub fn follow_resource(&mut self, resource: &str) -> bool {
        self.layout
            .index_of(resource)
            .is_some_and(|i| self.follow(i))
    }

    /// Whether it follows any collection.
    #[must_use]
    pub fn follows_any(&self) -> bool {
        self.follows.iter().any(|f| *f)
    }

    /// A put of `key` heard at `at`, with its in-band stamp (`None` when it
    /// carried none). A key of no collection is not held, and a put older
    /// than the one held within one run is not taken.
    pub fn put(&mut self, key: &str, stamp: Option<Stamp>, at: T) {
        self.mutation(key, stamp, false, at);
    }

    /// A delete of `key` heard at `at`, with its in-band stamp where its
    /// attachment carried one.
    pub fn delete(&mut self, key: &str, stamp: Option<Stamp>, at: T) {
        self.mutation(key, stamp, true, at);
    }

    fn mutation(&mut self, key: &str, stamp: Option<Stamp>, deleted: bool, at: T) {
        let Some(k) = crate::heartbeat::base_relative(key) else {
            return;
        };
        let Some(collection) = self.layout.collection_of(k) else {
            return;
        };
        if let (Some(h), Some(s)) = (self.members.get(k).and_then(|h| h.stamp), stamp)
            && h.run == s.run
            && s.seq <= h.seq
        {
            return;
        }
        self.members.insert(
            k.to_owned(),
            Held {
                collection,
                stamp,
                deleted,
                at,
                confirmed: None,
            },
        );
    }

    /// A complete read of the collection at `index` (S4's answer, or a
    /// snapshot once #977 lands), at `at`: its live members replace the
    /// view's, a member it left out leaving the view, and the collection
    /// is no longer due a read. A put heard after the read was asked is
    /// the caller's to apply again after it (core §4.2).
    pub fn replace<'k>(
        &mut self,
        index: usize,
        live: impl IntoIterator<Item = (&'k str, Option<Stamp>)>,
        at: T,
    ) {
        self.members.retain(|_, h| h.collection != index);
        for (key, stamp) in live {
            let Some(k) = crate::heartbeat::base_relative(key) else {
                continue;
            };
            if self.layout.collection_of(k) == Some(index) {
                self.members.insert(
                    k.to_owned(),
                    Held {
                        collection: index,
                        stamp,
                        deleted: false,
                        at,
                        confirmed: None,
                    },
                );
            }
        }
        if let Some(d) = self.diverged.get_mut(index) {
            *d = 0;
        }
    }

    /// The live members of a collection the view holds, in key order.
    fn live(&self, index: usize) -> impl Iterator<Item = (&String, &Held<T>)> {
        self.members
            .iter()
            .filter(move |(_, h)| h.collection == index && !h.deleted)
    }

    /// The view's digest of a collection: `None` while it holds a live
    /// member whose put carried no in-band stamp, which no digest counts.
    #[must_use]
    pub fn digest(&self, index: usize) -> Option<crate::heartbeat::Digest> {
        let mut d = crate::heartbeat::Digest::EMPTY;
        for (k, h) in self.live(index) {
            d.toggle(k, h.stamp?);
        }
        Some(d)
    }

    /// A heartbeat heard at `at`: CH's tag, its stamp and its body. See
    /// [`View`]'s doc for what it confirms.
    pub fn apply(&mut self, tag: crate::wire::Tag, stamp: Stamp, body: &[u8], at: T) -> Applied {
        use crate::heartbeat::{Heartbeat, Members};
        if tag != self.layout.tag {
            return Applied::Revision;
        }
        let hb = match Heartbeat::parse(body, &self.layout) {
            Ok(hb) => hb,
            Err(r) => return Applied::Refused(r),
        };
        let gap = match self.last {
            Some(l) if l.run == stamp.run => {
                match crate::seed::Order::Serial.seqs(l.seq, stamp.seq) {
                    Some(std::cmp::Ordering::Greater) => {
                        stamp.seq.get().wrapping_sub(l.seq.get()) & crate::wire::Seq::MAX.get() != 1
                    }
                    _ => return Applied::Older,
                }
            }
            _ => false,
        };
        self.last = Some(stamp);
        self.gap = gap;
        self.time = Some(hb.time);
        self.heartbeats += 1;
        let mut sections = Vec::with_capacity(hb.sections.len());
        for (i, section) in hb.sections.iter().enumerate() {
            if !self.follows[i] {
                sections.push(SectionRead::NotFollowed);
                continue;
            }
            let Some(s) = section else {
                self.diverged[i] = 0;
                sections.push(SectionRead::Absent);
                continue;
            };
            let keys: Vec<String> = self.live(i).map(|(k, _)| k.clone()).collect();
            let same_count = match &s.members {
                Members::One { .. } => keys.len() <= 1,
                m => m.live() == Some(keys.len() as u64),
            };
            if self.digest(i) != Some(s.digest) || !same_count {
                self.diverged[i] = self.diverged[i].saturating_add(1);
                sections.push(SectionRead::Diverged);
                continue;
            }
            self.diverged[i] = 0;
            let vouched: Vec<String> = s
                .members
                .vouched()
                .into_iter()
                .filter_map(|r| keys.get(usize::try_from(r).ok()?).cloned())
                .collect();
            for k in &vouched {
                if let Some(h) = self.members.get_mut(k) {
                    h.confirmed = Some(at);
                }
            }
            sections.push(SectionRead::Confirmed(vouched));
        }
        Applied::Read { gap, sections }
    }

    /// The last heartbeat that vouched for `key` since its last mutation.
    #[must_use]
    pub fn confirmed(&self, key: &str) -> Option<T> {
        let k = crate::heartbeat::base_relative(key)?;
        self.members.get(k).and_then(|h| h.confirmed)
    }

    /// The collections due a read: two heartbeats in a row diverged there.
    #[must_use]
    pub fn reread_due(&self) -> Vec<usize> {
        (0..self.diverged.len())
            .filter(|i| self.follows[*i] && self.diverged[*i] >= 2)
            .collect()
    }

    /// Whether the last heartbeat came after a gap in its key's seqs.
    #[must_use]
    pub fn gap(&self) -> bool {
        self.gap
    }

    /// The owner's time in the last heartbeat applied.
    #[must_use]
    pub fn owner_time(&self) -> Option<crate::heartbeat::Time> {
        self.time
    }

    /// The last heartbeat applied: its stamp.
    #[must_use]
    pub fn last_heartbeat(&self) -> Option<Stamp> {
        self.last
    }

    /// How many heartbeats it applied.
    #[must_use]
    pub fn heartbeats(&self) -> u64 {
        self.heartbeats
    }

    /// `key`'s last confirmation at `now` (§2.5): the later of its last
    /// mutation and the last heartbeat that vouched for it, as a [`Last`];
    /// `None` for a member the view does not hold.
    #[must_use]
    pub fn last(&self, key: &str, now: T) -> Option<Last> {
        let k = crate::heartbeat::base_relative(key)?;
        let h = self.members.get(k)?;
        Some(if h.deleted {
            Last::Delete(now.since(h.at))
        } else {
            Last::Put(now.since(h.confirmed.map_or(h.at, |c| c.max(h.at))))
        })
    }

    /// `key` as a subscription's observation at `now` (§2.5), the view
    /// standing in for its deliveries: its last confirmation, how long the
    /// subscription has `listened`, and complete when the subscription
    /// lost nothing (`complete`) and no heartbeat was lost.
    #[must_use]
    pub fn observation(
        &self,
        key: &str,
        now: T,
        listened: Duration,
        complete: bool,
    ) -> Observation {
        Observation::Subscribed {
            last: self.last(key, now),
            listened,
            complete: complete && !self.gap,
        }
    }
}

/// A subscription's last delivery of a member and, where a [`View`]
/// holds it, the last heartbeat that vouched for it: the later is its
/// last confirmation (§2.5). `delivered` is the subscription's own
/// [`Last`]; a delete stays a delete.
#[must_use]
pub fn confirmed_last(delivered: Option<Last>, heartbeat: Option<Duration>) -> Option<Last> {
    match (delivered, heartbeat) {
        (Some(Last::Put(a)), Some(h)) => Some(Last::Put(a.min(h))),
        (d, _) => d,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ann(v: Value) -> BTreeMap<String, Value> {
        BTreeMap::from([(TTL_S.to_owned(), v)])
    }

    #[test]
    fn the_horizon_is_a_whole_number_of_seconds() {
        let h = |v: Value| horizon(Kind::State, &ann(v));
        assert_eq!(h(json!(60)), Horizon::Within(Duration::from_secs(60)));
        assert_eq!(h(json!(60.0)), Horizon::Within(Duration::from_secs(60)));
        assert_eq!(h(json!(-0.0)), Horizon::Never);
        assert_eq!(h(json!(0)), Horizon::Never);
        assert!(matches!(h(json!(1.5)), Horizon::Invalid(_)));
        assert!(matches!(h(json!(-1)), Horizon::Invalid(_)));
        assert!(matches!(h(json!(MAX_TTL_S + 1)), Horizon::Invalid(_)));
        assert_eq!(
            horizon(Kind::Event, &ann(json!("x"))),
            Horizon::Ignored,
            "the kind first"
        );
        assert_eq!(horizon(Kind::State, &BTreeMap::new()), Horizon::Undeclared);
        assert_eq!(
            h(json!(61)).refresh_bound(),
            Some(Duration::from_millis(30_500))
        );
    }

    #[test]
    fn a_trusted_reply_is_judged_in_the_band() {
        let h = Horizon::Within(Duration::from_secs(2));
        let got = |s: f64| Observation::Got {
            reply: Some(Reply::Put {
                stamp_age: Some(StampAge::from_secs_f64(s)),
            }),
            clock: ClockTrust::Trusted {
                delta: DEFAULT_DELTA,
            },
        };
        let v = |s: f64| judge(&h, &got(s)).reason;
        assert_eq!(v(1.5), Reason::WithinHorizon);
        assert_eq!(v(1.6), Reason::NearHorizon);
        assert_eq!(v(2.5), Reason::NearHorizon);
        assert_eq!(v(2.6), Reason::BeyondHorizon);
        assert_eq!(v(-0.5), Reason::WithinHorizon);
        assert_eq!(v(-0.6), Reason::ClockDisagrees);
    }

    #[test]
    fn fresh_wins_then_stale_then_unobservable() {
        let j = |v, r| Judged::new(v, r);
        assert_eq!(
            combine([
                j(Verdict::Stale, Reason::NoDelivery),
                j(Verdict::Fresh, Reason::WithinHorizon)
            ])
            .verdict,
            Verdict::Fresh
        );
        assert_eq!(
            combine([
                j(Verdict::Unobservable, Reason::ClockUntrusted),
                j(Verdict::Stale, Reason::NoDelivery)
            ])
            .reason,
            Reason::NoDelivery
        );
        assert_eq!(combine([]).reason, Reason::NoObservation);
    }

    mod view {
        use super::super::*;
        use crate::contract::load_str;
        use crate::heartbeat::{Digest, Heartbeat, Layout, Members, Section, Time};
        use crate::timing::Mono;
        use crate::wire::{Run, Seq, Tag};

        const HB: &str = r#"
[interface]
name = "hb"
major = 1
minor = 0
uses = ["freshness.v1"]
[resources.link]
kind = "state"
type = { raw = "text/plain" }
annotations = { "freshness.ttl_s" = 2 }
[resources."peer/{peer}"]
kind = "state"
type = { raw = "text/plain" }
params = { peer = "string" }
cardinality = 5
annotations = { "freshness.ttl_s" = 2 }
"#;
        const LINK: &str = "zk2/lab/n1/hb.v1/state/link";
        const A: &str = "zk2/lab/n1/hb.v1/state/peer/a";
        const B: &str = "zk2/lab/n1/hb.v1/state/peer/b";

        fn layout() -> Layout {
            let c = load_str(HB, std::path::Path::new("."), None)
                .contract
                .unwrap();
            Layout::of(&c)
        }
        fn st(run: u16, seq: u32) -> Stamp {
            Stamp {
                run: Run::U16(run),
                seq: Seq::new(seq).unwrap(),
            }
        }
        fn ms(n: u64) -> Mono {
            Mono::after(Duration::from_millis(n))
        }
        /// A heartbeat over (link, peer/*), its sections given.
        fn body(l: &Layout, link: Option<Section>, peers: Option<Section>) -> Vec<u8> {
            Heartbeat {
                time: Time::Unknown,
                sections: vec![link, peers],
            }
            .write(l)
            .unwrap()
        }
        fn view() -> View<Mono> {
            let mut v = View::new(layout());
            assert!(v.follow_resource("state/link"));
            assert!(v.follow_resource("state/peer/{peer}"));
            assert!(!v.follow_resource("state/nothing"));
            v
        }

        #[test]
        fn a_heartbeat_whose_digest_is_the_views_confirms_what_it_vouches_for() {
            let l = layout();
            let mut v = view();
            v.put(LINK, Some(st(1, 1)), ms(0));
            v.put(A, Some(st(1, 2)), ms(0));
            v.put(B, Some(st(1, 3)), ms(0));
            let link = Section {
                digest: Digest::of([(LINK, st(1, 1))]),
                members: Members::One { vouched: true },
            };
            let peers = Section {
                digest: Digest::of([(A, st(1, 2)), (B, st(1, 3))]),
                members: Members::templated(2, [1]),
            };
            let got = v.apply(
                l.tag,
                st(9, 0),
                &body(&l, Some(link), Some(peers)),
                ms(1500),
            );
            assert_eq!(
                got,
                Applied::Read {
                    gap: false,
                    sections: vec![
                        SectionRead::Confirmed(vec![LINK.into()]),
                        SectionRead::Confirmed(vec![B.into()]),
                    ]
                }
            );
            assert_eq!(v.confirmed(B), Some(ms(1500)));
            assert_eq!(v.confirmed(A), None, "not vouched for");
            // At 3 s: the link put at 0 is 3 s old, confirmed 1.5 s ago.
            let h = Horizon::Within(Duration::from_secs(2));
            let j = |k: &str| {
                judge(
                    &h,
                    &v.observation(k, ms(3000), Duration::from_secs(3), true),
                )
            };
            assert_eq!(j(LINK).verdict, Verdict::Fresh);
            assert_eq!(j(B).verdict, Verdict::Fresh);
            assert_eq!(
                j(A).verdict,
                Verdict::Stale,
                "a cleared bit is no confirmation"
            );
            // A namespace in front reads as the base-relative key.
            assert_eq!(v.confirmed(&format!("prod/{LINK}")), Some(ms(1500)));
        }

        #[test]
        fn a_missed_change_diverges_and_two_in_a_row_are_due_a_read() {
            let l = layout();
            let mut v = view();
            v.put(A, Some(st(1, 2)), ms(0));
            // The owner holds a's seq 5 and b: the view missed both.
            let owner = Digest::of([(A, st(1, 5)), (B, st(1, 6))]);
            let sec = || Section {
                digest: owner,
                members: Members::templated(2, [0, 1]),
            };
            for (seq, due) in [(0, false), (1, true)] {
                let got = v.apply(l.tag, st(9, seq), &body(&l, None, Some(sec())), ms(1000));
                assert_eq!(
                    got,
                    Applied::Read {
                        gap: false,
                        sections: vec![SectionRead::Absent, SectionRead::Diverged]
                    }
                );
                assert_eq!(v.reread_due() == vec![1], due);
            }
            assert_eq!(v.confirmed(A), None);
            // The read's answer replaces the view, and the next heartbeat confirms.
            v.replace(1, [(A, Some(st(1, 5))), (B, Some(st(1, 6)))], ms(1100));
            assert!(v.reread_due().is_empty());
            let got = v.apply(l.tag, st(9, 2), &body(&l, None, Some(sec())), ms(1200));
            assert_eq!(
                got,
                Applied::Read {
                    gap: false,
                    sections: vec![
                        SectionRead::Absent,
                        SectionRead::Confirmed(vec![A.into(), B.into()])
                    ]
                }
            );
        }

        #[test]
        fn a_deleted_member_leaves_the_digest_and_a_stale_put_is_not_taken() {
            let mut v = view();
            v.put(A, Some(st(1, 4)), ms(0));
            v.put(A, Some(st(1, 3)), ms(10));
            assert_eq!(v.digest(1), Some(Digest::of([(A, st(1, 4))])));
            v.delete(A, Some(st(1, 5)), ms(20));
            assert_eq!(v.digest(1), Some(Digest::EMPTY));
            assert_eq!(
                v.last(A, ms(30)),
                Some(Last::Delete(Duration::from_millis(10)))
            );
            // Another run's put is taken whatever its seq.
            v.put(A, Some(st(2, 0)), ms(40));
            assert_eq!(v.digest(1), Some(Digest::of([(A, st(2, 0))])));
            // A put with no envelope leaves the collection with no digest.
            v.put(B, None, ms(50));
            assert_eq!(v.digest(1), None);
            // A key of no collection is not held.
            v.put("zk2/lab/n1/hb.v1/state/other", Some(st(1, 1)), ms(60));
            assert_eq!(v.last("zk2/lab/n1/hb.v1/state/other", ms(70)), None);
        }

        #[test]
        fn a_gap_is_a_heartbeat_lost_and_the_tag_and_the_order_guard() {
            let l = layout();
            let mut v = view();
            v.put(LINK, Some(st(1, 1)), ms(0));
            let link = || {
                Some(Section {
                    digest: Digest::of([(LINK, st(1, 1))]),
                    members: Members::One { vouched: true },
                })
            };
            let b = body(&l, link(), None);
            let other = Tag::new((l.tag.get() + 1) % 16).unwrap();
            assert_eq!(v.apply(other, st(9, 0), &b, ms(100)), Applied::Revision);
            assert!(matches!(
                v.apply(l.tag, st(9, 0), &b, ms(100)),
                Applied::Read { gap: false, .. }
            ));
            assert!(matches!(
                v.apply(l.tag, st(9, 3), &b, ms(200)),
                Applied::Read { gap: true, .. }
            ));
            assert!(v.gap());
            let o = v.observation(LINK, ms(5000), Duration::from_secs(5), true);
            assert_eq!(
                judge(&Horizon::Within(Duration::from_secs(2)), &o).reason,
                Reason::Incomplete
            );
            assert_eq!(v.apply(l.tag, st(9, 3), &b, ms(300)), Applied::Older);
            assert_eq!(v.apply(l.tag, st(9, 1), &b, ms(300)), Applied::Older);
            assert!(matches!(
                v.apply(l.tag, st(9, 4), &b, ms(400)),
                Applied::Read { gap: false, .. }
            ));
            assert!(!v.gap());
            // A new run of the key is no gap, and its seq starts anew.
            assert!(matches!(
                v.apply(l.tag, st(10, 0), &b, ms(500)),
                Applied::Read { gap: false, .. }
            ));
            // A body cut short is refused, and counts as no heartbeat.
            assert!(matches!(
                v.apply(l.tag, st(10, 1), &b[..2], ms(600)),
                Applied::Refused(_)
            ));
            assert_eq!(v.heartbeats(), 4);
        }
    }

    #[test]
    fn a_measured_offset_trusts_within_the_delta_either_way() {
        let d = DEFAULT_DELTA;
        assert!(matches!(
            ClockTrust::measured(StampAge::ahead(d), d),
            ClockTrust::Trusted { .. }
        ));
        assert_eq!(
            ClockTrust::measured(StampAge::behind(Duration::from_millis(501)), d),
            ClockTrust::Untrusted
        );
    }
}
