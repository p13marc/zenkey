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

use std::collections::BTreeMap;
use std::fmt;
use std::time::{Duration, SystemTime};

use serde_json::Value;

use crate::authoring::Kind;
use crate::contract::Resource;

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
