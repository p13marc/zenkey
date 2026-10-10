//! The population budget (core §2.7, 0.24; #735): a templated resource's
//! bound in one instance, what its members and its live members are, what
//! an event's `rate` bounds, and the pure judgement of a reading against
//! them, in four states and a reason.
//!
//! ```text
//! bound(contract, stated…)              ─▶ Untemplated │ NoCeiling │ Of(n)
//! population(kind, bound, retention, r) ─▶ exceeds │ within │ unobservable │ not_asked, + reason
//! rate(rate, occurrences)               ─▶ the same four, + reason
//! ```
//!
//! Each judgement answers "does this instance exceed its budget here?", so
//! its finding is the yes: [`Verdict::Exceeds`]. There is no I/O and no
//! clock here. A reader counts what it read (the keys an owner's GET
//! answered with a value and whether it ran to its end; the instants a
//! window heard each member, how long it listened, whether it lost a
//! delivery and whether the owner stayed present) and hands it over as
//! values; the fixtures in `spec/conformance/budget/` pin the rest.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use crate::authoring::Kind;
use crate::contract::{Body, Rate, Resource};

/// The no-ceiling cardinality, 2^32−1 (§2.2): no bound, never a population
/// to budget with.
pub const NO_CEILING: u64 = 4_294_967_295;

/// How long a stream member stays live after the owner last published on
/// it (§2.7): an hour.
pub const STREAM_LIVENESS: Duration = Duration::from_secs(3600);

/// A templated resource's bound in an instance (§2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    /// The template has no parameters, so no `cardinality`: one member by
    /// construction, and nothing to budget.
    Untemplated,
    /// The no-ceiling value, from the contract or a descriptor (§2.2).
    NoCeiling,
    /// A bound of `n` live members.
    Of(u64),
}

impl Bound {
    /// The fixture's form: an integer, `"no_ceiling"` or `"untemplated"`.
    #[must_use]
    pub fn token(self) -> String {
        match self {
            Self::Untemplated => "untemplated".to_owned(),
            Self::NoCeiling => "no_ceiling".to_owned(),
            Self::Of(n) => n.to_string(),
        }
    }

    /// The bound, when it is one.
    #[must_use]
    pub fn limit(self) -> Option<u64> {
        match self {
            Self::Of(n) => Some(n),
            _ => None,
        }
    }
}

impl fmt::Display for Bound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Untemplated => f.write_str("no bound (a template without parameters)"),
            Self::NoCeiling => f.write_str("no ceiling (4294967295)"),
            Self::Of(n) => write!(f, "{n}"),
        }
    }
}

/// The bound of a resource whose contract declares `contract`, across the
/// instances exposing it, each stating its own lowered value or none
/// (`stated`, one entry per instance; core §2.7, §3.3).
///
/// - A stated value from 1 to the contract's applies to its instance;
///   any other is D007's, and the contract's applies instead.
/// - Several instances (a re-mint's overlap) share one population, since a
///   key names no instance: the greatest of their bounds applies, so a
///   finding never rests on a bound one of them did not state.
/// - No instance: the contract's.
/// - 4294967295, whichever source gives it, is no ceiling.
#[must_use]
pub fn bound(contract: Option<u64>, stated: impl IntoIterator<Item = Option<u64>>) -> Bound {
    let Some(c) = contract else {
        return Bound::Untemplated;
    };
    let n = stated
        .into_iter()
        .map(|s| match s {
            Some(n) if (1..=c).contains(&n) => n,
            _ => c,
        })
        .max()
        .unwrap_or(c);
    if n == NO_CEILING {
        Bound::NoCeiling
    } else {
        Bound::Of(n)
    }
}

/// [`bound`] of a resource of a contract.
#[must_use]
pub fn bound_of(r: &Resource, stated: impl IntoIterator<Item = Option<u64>>) -> Bound {
    bound(r.cardinality, stated)
}

/// An event's retention, when the resource declares one.
#[must_use]
pub fn retention_of(r: &Resource) -> Option<Duration> {
    match &r.body {
        Body::Data(d) => d.retention_s.map(Duration::from_secs),
        Body::Operation(_) => None,
    }
}

/// An event's rate, when the resource declares one.
#[must_use]
pub fn rate_of(r: &Resource) -> Option<Rate> {
    match &r.body {
        Body::Data(d) => d.rate,
        Body::Operation(_) => None,
    }
}

impl Rate {
    /// What the rate allows per member (§2.7): at most `n` occurrences
    /// within any period this long.
    #[must_use]
    pub fn limit(self) -> (u32, Duration) {
        const HOUR: Duration = Duration::from_secs(3600);
        match self {
            Self::Rare => (1, HOUR),
            Self::Low => (1, Duration::from_secs(60)),
            Self::Burst(n) => (n, HOUR),
        }
    }
}

/// How long a member of a resource of `kind` stays live after its last
/// put or occurrence (§2.7): a stream's for an hour, an event's for its
/// retention. A state member is live while its key has a value, which a
/// window cannot see; an operation has no live member a reading counts.
#[must_use]
pub fn liveness(kind: Kind, retention: Option<Duration>) -> Option<Duration> {
    match kind {
        Kind::Stream => Some(STREAM_LIVENESS),
        Kind::Event => retention,
        Kind::State | Kind::Operation => None,
    }
}

/// What a reader read of a resource's members (§2.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    /// An owner's S4 GET over a state resource (core §4.2): how many keys
    /// it answered with a value (a `reply_del` is no member), and whether
    /// it ran to its final reply, with no error reply, a timeout included.
    Get { members: u64, complete: bool },
    /// A subscription window over a stream or an event.
    Window(Window),
}

/// A subscription window over a stream or an event, as a population
/// reading (§2.7). It is complete — it heard every member live at its end —
/// when it lasted at least one liveness span, lost no delivery, and the
/// owner was present throughout.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Window {
    /// The instants each member was heard, on one clock (the reader's
    /// receive clock), from any origin. An event's member is its key
    /// without the ULID chunk.
    pub heard: BTreeMap<String, Vec<Duration>>,
    /// How long the window listened.
    pub listened: Duration,
    /// Whether it delivered everything that arrived for it: false when the
    /// reader fell behind and lost deliveries.
    pub lossless: bool,
    /// Whether the owner was present, an instance token of its held, from
    /// the window's start to its end: false when it joined, left or
    /// re-minted while the window listened.
    pub present: bool,
}

/// The occurrences of an event a window heard, for its `rate` (§2.7).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Occurrences {
    /// How long the window listened.
    pub listened: Duration,
    /// Whether the window delivered everything that arrived for it: false
    /// when the reader fell behind and lost deliveries.
    pub complete: bool,
    /// Per member, the instants of its occurrences, measured between the
    /// occurrences' stamps of one clock, or on the reader's receive clock.
    pub members: BTreeMap<String, Vec<Duration>>,
}

/// The answer to "does this instance exceed its budget here?" (§2.7). The
/// finding is the yes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Verdict {
    /// The finding: more live members than the bound, or an event's
    /// occurrences beyond its rate.
    Exceeds,
    /// Clean: a reading that can show the budget kept shows it kept.
    Within,
    /// The question was put, and the reading could not answer it.
    Unobservable,
    /// Nobody put the question, or it has no premise here.
    NotAsked,
}

impl Verdict {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exceeds => "exceeds",
            Self::Within => "within",
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
    /// Exceeds: more members than the bound, in any reading.
    OverBound,
    /// Exceeds: `n + 1` occurrences of one member less than the period
    /// apart.
    RateExceeded,
    /// Within: a complete reading — a GET that ran to its final reply, or a
    /// window of a whole liveness span, lossless, the owner present
    /// throughout — counted at least one member, and no more than the
    /// bound.
    Complete,
    /// Within: a window of at least one period, nothing lost, an
    /// occurrence heard, none beyond the rate.
    RateKept,
    /// Unobservable: no member, or no occurrence, was read: an empty reply
    /// set is never a verdict (core O5).
    Empty,
    /// Unobservable: the GET ended at its timeout, or with an error reply.
    Incomplete,
    /// Unobservable: the window lost deliveries.
    Lossy,
    /// Unobservable: the window was shorter than the rate's period, or than
    /// the population's liveness span.
    WindowTooShort,
    /// Unobservable: the owner was not present throughout the window: it
    /// joined, left or re-minted while the window listened.
    OwnerAbsent,
    /// Unobservable: a reading that does not count this kind's members (a
    /// window of a state, a GET of a stream or an event).
    ReadingKind,
    /// Not asked: the no-ceiling bound.
    NoCeiling,
    /// Not asked: a template without parameters.
    Untemplated,
    /// Not asked: an operation, whose members no reading counts.
    Kind,
    /// Not asked: no `rate`, so not an event.
    NoRate,
}

impl Reason {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OverBound => "over_bound",
            Self::RateExceeded => "rate_exceeded",
            Self::Complete => "complete",
            Self::RateKept => "rate_kept",
            Self::Empty => "empty",
            Self::Incomplete => "incomplete",
            Self::Lossy => "lossy",
            Self::WindowTooShort => "window_too_short",
            Self::OwnerAbsent => "owner_absent",
            Self::ReadingKind => "reading_kind",
            Self::NoCeiling => "no_ceiling",
            Self::Untemplated => "untemplated",
            Self::Kind => "kind",
            Self::NoRate => "no_rate",
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A verdict, its reason, and what it counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Judged {
    pub verdict: Verdict,
    pub reason: Reason,
    /// The members counted against the bound (the most within one liveness
    /// period, for a window), or, for a rate, the most occurrences of one
    /// member within one period. 0 when nothing was counted.
    pub counted: u64,
}

impl Judged {
    fn new(verdict: Verdict, reason: Reason, counted: u64) -> Self {
        Self {
            verdict,
            reason,
            counted,
        }
    }
}

/// The most distinct members heard within any span shorter than `period`:
/// the members a window shows live at once (§2.7). Without a period, every
/// member heard.
#[must_use]
pub fn most_within(heard: &BTreeMap<String, Vec<Duration>>, period: Option<Duration>) -> u64 {
    let Some(period) = period else {
        return heard.values().filter(|v| !v.is_empty()).count() as u64;
    };
    let mut at: Vec<(Duration, &str)> = heard
        .iter()
        .flat_map(|(m, ts)| ts.iter().map(move |t| (*t, m.as_str())))
        .collect();
    at.sort();
    let mut inside: BTreeMap<&str, usize> = BTreeMap::new();
    let (mut best, mut end) = (0usize, 0usize);
    for start in 0..at.len() {
        while end < at.len() && at[end].0 < at[start].0 + period {
            *inside.entry(at[end].1).or_default() += 1;
            end += 1;
        }
        best = best.max(inside.len());
        let m = at[start].1;
        if let Some(n) = inside.get_mut(m) {
            *n -= 1;
            if *n == 0 {
                inside.remove(m);
            }
        }
    }
    best as u64
}

/// "Does this instance hold more live members than its bound?" (§2.7),
/// for a resource of `kind` with `bound` and, for an event, `retention`.
///
/// The order a reason is chosen in: not asked (an operation; no ceiling;
/// no parameters); a reading this kind does not count; more members than
/// the bound, in any reading (a lower bound already exceeds it); an
/// incomplete reading — a GET that did not run to its final reply, or a
/// window that lost a delivery, was shorter than one liveness span, or over
/// which the owner was not present throughout, in that order; no member at
/// all (O5); else within.
#[must_use]
pub fn population(
    kind: Kind,
    bound: Bound,
    retention: Option<Duration>,
    reading: &Reading,
) -> Judged {
    use Verdict::{Exceeds, NotAsked, Unobservable, Within};
    if kind == Kind::Operation {
        return Judged::new(NotAsked, Reason::Kind, 0);
    }
    let n = match bound {
        Bound::Untemplated => return Judged::new(NotAsked, Reason::Untemplated, 0),
        Bound::NoCeiling => return Judged::new(NotAsked, Reason::NoCeiling, 0),
        Bound::Of(n) => n,
    };
    let span = liveness(kind, retention);
    let (counted, short_of) = match (kind, reading) {
        (Kind::State, Reading::Get { members, complete }) => {
            (*members, (!complete).then_some(Reason::Incomplete))
        }
        (Kind::Stream | Kind::Event, Reading::Window(w)) => {
            let short = span.is_none_or(|span| w.listened < span);
            let why = if !w.lossless {
                Some(Reason::Lossy)
            } else if short {
                Some(Reason::WindowTooShort)
            } else if !w.present {
                Some(Reason::OwnerAbsent)
            } else {
                None
            };
            (most_within(&w.heard, span), why)
        }
        _ => return Judged::new(Unobservable, Reason::ReadingKind, 0),
    };
    if counted > n {
        return Judged::new(Exceeds, Reason::OverBound, counted);
    }
    if let Some(why) = short_of {
        return Judged::new(Unobservable, why, counted);
    }
    if counted == 0 {
        return Judged::new(Unobservable, Reason::Empty, 0);
    }
    Judged::new(Within, Reason::Complete, counted)
}

/// "Does an event publish beyond its rate?" (§2.7), per member: at most
/// `n` occurrences of one member within any period, so `n + 1` less than
/// the period apart are the finding, in a window of any length.
///
/// The order a reason is chosen in: not asked (no rate); beyond the rate;
/// deliveries lost; a window shorter than one period; no occurrence heard
/// (O5); else kept.
#[must_use]
pub fn rate(rate: Option<Rate>, o: &Occurrences) -> Judged {
    use Verdict::{Exceeds, NotAsked, Unobservable, Within};
    let Some(rate) = rate else {
        return Judged::new(NotAsked, Reason::NoRate, 0);
    };
    let (n, period) = rate.limit();
    let n = n as usize;
    let mut most = 0usize;
    for ts in o.members.values() {
        let mut ts = ts.clone();
        ts.sort();
        let mut end = 0usize;
        for start in 0..ts.len() {
            while end < ts.len() && ts[end] < ts[start] + period {
                end += 1;
            }
            most = most.max(end - start);
        }
    }
    if most > n {
        return Judged::new(Exceeds, Reason::RateExceeded, most as u64);
    }
    if !o.complete {
        return Judged::new(Unobservable, Reason::Lossy, most as u64);
    }
    if o.listened < period {
        return Judged::new(Unobservable, Reason::WindowTooShort, most as u64);
    }
    if most == 0 {
        return Judged::new(Unobservable, Reason::Empty, 0);
    }
    Judged::new(Within, Reason::RateKept, most as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(secs: f64) -> Duration {
        Duration::from_secs_f64(secs)
    }

    fn heard(of: &[(&str, &[f64])]) -> BTreeMap<String, Vec<Duration>> {
        of.iter()
            .map(|(m, ts)| ((*m).to_owned(), ts.iter().map(|t| s(*t)).collect()))
            .collect()
    }

    #[test]
    fn the_bound_is_the_greatest_valid_one_stated() {
        assert_eq!(bound(None, []), Bound::Untemplated);
        assert_eq!(bound(Some(64), []), Bound::Of(64));
        assert_eq!(bound(Some(64), [Some(32)]), Bound::Of(32));
        assert_eq!(bound(Some(64), [Some(16), Some(32)]), Bound::Of(32));
        assert_eq!(bound(Some(64), [Some(32), None]), Bound::Of(64));
        assert_eq!(bound(Some(64), [Some(128)]), Bound::Of(64), "D007's");
        assert_eq!(bound(Some(64), [Some(0)]), Bound::Of(64), "D007's");
        assert_eq!(bound(Some(NO_CEILING), []), Bound::NoCeiling);
        assert_eq!(bound(Some(NO_CEILING), [Some(500)]), Bound::Of(500));
        assert_eq!(
            bound(Some(NO_CEILING), [Some(NO_CEILING)]),
            Bound::NoCeiling
        );
    }

    #[test]
    fn a_window_counts_the_members_live_at_once() {
        let h = heard(&[("a", &[0.0]), ("b", &[10.0]), ("c", &[4000.0])]);
        assert_eq!(most_within(&h, Some(STREAM_LIVENESS)), 2);
        assert_eq!(most_within(&h, None), 3);
        // Exactly one period apart is not within one period.
        let h = heard(&[("a", &[0.0]), ("b", &[3600.0])]);
        assert_eq!(most_within(&h, Some(STREAM_LIVENESS)), 1);
        // A member heard twice counts once.
        let h = heard(&[("a", &[0.0, 1.0, 2.0]), ("b", &[1.5])]);
        assert_eq!(most_within(&h, Some(STREAM_LIVENESS)), 2);
        assert_eq!(most_within(&BTreeMap::new(), Some(STREAM_LIVENESS)), 0);
    }

    #[test]
    fn population_poles() {
        let get = |members, complete| Reading::Get { members, complete };
        let j = |kind, bound, r: &Reading| {
            let j = population(kind, bound, Some(s(60.0)), r);
            (j.verdict, j.reason)
        };
        use Reason as R;
        use Verdict as V;
        assert_eq!(
            j(Kind::State, Bound::Of(2), &get(3, false)),
            (V::Exceeds, R::OverBound)
        );
        assert_eq!(
            j(Kind::State, Bound::Of(2), &get(2, true)),
            (V::Within, R::Complete)
        );
        assert_eq!(
            j(Kind::State, Bound::Of(2), &get(2, false)),
            (V::Unobservable, R::Incomplete)
        );
        assert_eq!(
            j(Kind::State, Bound::Of(2), &get(0, true)),
            (V::Unobservable, R::Empty)
        );
        assert_eq!(
            j(Kind::State, Bound::NoCeiling, &get(9, true)),
            (V::NotAsked, R::NoCeiling)
        );
        assert_eq!(
            j(Kind::State, Bound::Of(2), &get(0, false)),
            (V::Unobservable, R::Incomplete)
        );
        let w = |listened: f64, lossless, present, of: &[(&str, &[f64])]| {
            Reading::Window(Window {
                heard: heard(of),
                listened: s(listened),
                lossless,
                present,
            })
        };
        let two: &[(&str, &[f64])] = &[("a", &[0.0]), ("b", &[1.0])];
        // Above the bound: the finding, from any window.
        assert_eq!(
            j(Kind::Stream, Bound::Of(1), &w(5.0, false, false, two)),
            (V::Exceeds, R::OverBound)
        );
        // Within it: complete only over an hour, lossless, owner present.
        assert_eq!(
            j(Kind::Stream, Bound::Of(2), &w(3600.0, true, true, two)),
            (V::Within, R::Complete)
        );
        assert_eq!(
            j(Kind::Stream, Bound::Of(2), &w(3599.0, true, true, two)),
            (V::Unobservable, R::WindowTooShort)
        );
        assert_eq!(
            j(Kind::Stream, Bound::Of(2), &w(3600.0, false, true, two)),
            (V::Unobservable, R::Lossy)
        );
        assert_eq!(
            j(Kind::Stream, Bound::Of(2), &w(3600.0, true, false, two)),
            (V::Unobservable, R::OwnerAbsent)
        );
        assert_eq!(
            j(Kind::Stream, Bound::Of(2), &w(3600.0, true, true, &[])),
            (V::Unobservable, R::Empty)
        );
        assert_eq!(
            j(Kind::State, Bound::Of(2), &w(3600.0, true, true, two)),
            (V::Unobservable, R::ReadingKind)
        );
        assert_eq!(
            j(Kind::Operation, Bound::Of(2), &w(3600.0, true, true, two)),
            (V::NotAsked, R::Kind)
        );
        // An event's members are live for its retention (60 s here).
        let apart: &[(&str, &[f64])] = &[("a", &[0.0]), ("b", &[61.0])];
        assert_eq!(
            j(Kind::Event, Bound::Of(1), &w(120.0, true, true, apart)),
            (V::Within, R::Complete)
        );
        assert_eq!(
            j(Kind::Event, Bound::Of(1), &w(59.0, true, true, apart)),
            (V::Unobservable, R::WindowTooShort)
        );
    }

    #[test]
    fn rate_poles() {
        let o = |listened, complete, of: &[(&str, &[f64])]| Occurrences {
            listened: s(listened),
            complete,
            members: heard(of),
        };
        let j = |r, o: &Occurrences| {
            let j = rate(r, o);
            (j.verdict, j.reason)
        };
        use Reason as R;
        use Verdict as V;
        let low = Some(Rate::Low);
        assert_eq!(
            j(low, &o(5.0, true, &[("a", &[0.0, 59.9])])),
            (V::Exceeds, R::RateExceeded)
        );
        assert_eq!(
            j(low, &o(120.0, true, &[("a", &[0.0, 60.0])])),
            (V::Within, R::RateKept)
        );
        assert_eq!(
            j(low, &o(30.0, true, &[("a", &[0.0])])),
            (V::Unobservable, R::WindowTooShort)
        );
        assert_eq!(
            j(low, &o(120.0, false, &[("a", &[0.0])])),
            (V::Unobservable, R::Lossy)
        );
        assert_eq!(j(low, &o(120.0, true, &[])), (V::Unobservable, R::Empty));
        // Per member: two members once each is no excess.
        assert_eq!(
            j(low, &o(120.0, true, &[("a", &[0.0]), ("b", &[1.0])])),
            (V::Within, R::RateKept)
        );
        let burst = Some(Rate::Burst(3));
        assert_eq!(
            j(
                burst,
                &o(3600.0, true, &[("a", &[0.0, 1200.0, 2400.0, 3599.0])])
            ),
            (V::Exceeds, R::RateExceeded)
        );
        assert_eq!(
            j(
                burst,
                &o(3600.0, true, &[("a", &[0.0, 1200.0, 2400.0, 3600.0])])
            ),
            (V::Within, R::RateKept)
        );
        assert_eq!(j(None, &o(120.0, true, &[])), (V::NotAsked, R::NoRate));
    }
}
