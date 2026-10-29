//! The owner's half of core §2.7 (#741): a templated resource's live
//! members kept within its bound, and an event member's occurrences within
//! its `rate` — a write that would break either refused, nothing published,
//! never dropped silently.
//!
//! | §2.7 | Here |
//! |---|---|
//! | the bound: the descriptor's lowered `cardinality`, else the contract's; no ceiling is no bound | [`Budget`], `zenkey_model::budget::bound` |
//! | a state member is live while its key has a value | admitted on a put, released on a delete |
//! | a stream member is live for an hour after its last publication | `zenkey_model::budget::STREAM_LIVENESS` |
//! | an event member is live for the retention after its last occurrence | the resource's `retention` |
//! | at most `n` occurrences of an event member within any period of its rate | each member's last `n` instants |
//! | a refusal is never silent | [`Refusal`], `crate::Error::Budget` |
//!
//! **What it costs.** Nothing is tracked for a resource with no bound and
//! no rate: no ceiling (4294967295), no parameters, a state or a stream.
//! Otherwise the members tracked are bounded: at most the bound live at
//! once, forgotten when they stop being live; an event's rate history adds
//! the members with an occurrence within the rate's period — at most the
//! bound per retention, so the bound × ⌈period ÷ retention⌉ — each holding
//! at most the rate's `n` instants. Expired members are swept when the map
//! reaches the bound, or doubles, so a write costs O(1) amortized.
//!
//! **Its clock** is the owner's monotonic clock at the write: when the
//! occurrence is published, which is what a reader measures (§2.7,
//! "Spans"), and not the instant an `EventWriter::put_at` ULID carries.
//!
//! The writers ([`crate::writer::Writer`], [`crate::writer::EventWriter`],
//! [`crate::state::StateWriter`]) consult the budget their service made
//! for the resource. A raw publisher (`ServiceBuilder::declare_publisher`)
//! is plain zenoh, and keeping §2.7 with it is its caller's.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use zenkey_model::authoring::Kind;
use zenkey_model::budget::{Bound, STREAM_LIVENESS};
use zenkey_model::contract::{Body, Rate, Resource};
use zenkey_model::grammar::IfaceId;

/// Whose a bound is (§2.7): the descriptor's lowered `cardinality`, or the
/// contract's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Descriptor,
    Contract,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Descriptor => "the descriptor's",
            Self::Contract => "the contract's",
        })
    }
}

/// What a refused write would have broken (§2.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Breach {
    /// A new member while `live` members are live, at the bound.
    Bound {
        bound: u64,
        source: Source,
        live: u64,
    },
    /// An occurrence of a member that already has `allowed` within the
    /// rate's period.
    Rate {
        rate: Rate,
        allowed: u32,
        period: Duration,
    },
}

/// A write §2.7 forbids, refused: nothing was published.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub struct Refusal {
    pub iface: IfaceId,
    /// The resource, `<kind token>/<template>`.
    pub resource: String,
    /// The member: its key, an event's without the ULID chunk.
    pub member: String,
    pub breach: Breach,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            iface,
            resource,
            member,
            breach,
        } = self;
        match breach {
            Breach::Bound {
                bound,
                source,
                live,
            } => write!(
                f,
                "{iface} {resource:?}: {member} would be a new live member while {live} are \
                 live, at its bound of {bound} ({source}): refused, nothing published (core \
                 §2.7)"
            ),
            Breach::Rate {
                rate,
                allowed,
                period,
            } => write!(
                f,
                "{iface} {resource:?}: an occurrence of {member} beyond its rate {rate}, which \
                 allows {allowed} within {}: refused, nothing published (core §2.7)",
                period_words(*period)
            ),
        }
    }
}

fn period_words(d: Duration) -> String {
    match d.as_secs() {
        60 => "a minute".to_owned(),
        3600 => "an hour".to_owned(),
        s => format!("{s} s"),
    }
}

/// What one member's writes left behind.
#[derive(Debug)]
struct Member {
    /// Its last write.
    last: Instant,
    /// An event's occurrences within the rate's period, oldest first, at
    /// most the rate's `n`.
    recent: VecDeque<Instant>,
}

/// The bookkeeping of one stream, state or event resource in one service.
#[derive(Debug)]
struct Members {
    map: HashMap<String, Member>,
    /// The size past which expired members are swept.
    sweep_at: usize,
}

/// One resource's budget in one service (§2.7): its bound, its liveness,
/// its rate, and the members its writes made live.
#[derive(Debug)]
pub struct Budget {
    iface: IfaceId,
    resource: String,
    kind: Kind,
    /// The contract's `cardinality`; `None` for a template without
    /// parameters.
    contract: Option<u64>,
    /// The descriptor's lowered `cardinality`, 0 for none.
    stated: AtomicU64,
    /// How long a member stays live after its last write: a stream's hour,
    /// an event's retention; `None` for a state, live until deleted.
    span: Option<Duration>,
    /// An event's rate: at most `n` occurrences of a member within the
    /// period.
    rate: Option<(Rate, u32, Duration)>,
    members: Mutex<Members>,
}

/// The fewest members kept before a sweep.
const SWEEP_FLOOR: usize = 64;

impl Budget {
    /// The budget of `r`, a resource of `iface`; `None` for an operation,
    /// whose members no owner holds.
    #[must_use]
    pub fn new(iface: &IfaceId, r: &Resource) -> Option<Self> {
        let Body::Data(d) = &r.body else {
            return None;
        };
        let span = match r.kind {
            Kind::Stream => Some(STREAM_LIVENESS),
            Kind::Event => d.retention_s.map(Duration::from_secs),
            Kind::State | Kind::Operation => None,
        };
        let rate = d.rate.map(|rate| {
            let (n, period) = rate.limit();
            (rate, n, period)
        });
        Some(Self {
            iface: iface.clone(),
            resource: crate::implementation::resource_name(r),
            kind: r.kind,
            contract: r.cardinality,
            stated: AtomicU64::new(0),
            span,
            rate,
            members: Mutex::new(Members {
                map: HashMap::new(),
                sweep_at: SWEEP_FLOOR,
            }),
        })
    }

    /// The resource, `<kind token>/<template>`.
    #[must_use]
    pub fn resource(&self) -> &str {
        &self.resource
    }

    /// The bound in force, and whose it is (§2.7): the descriptor's lowered
    /// value when one from 1 to the contract's is stated, else the
    /// contract's. No ceiling and no parameters are no bound.
    #[must_use]
    pub fn bound(&self) -> (Bound, Source) {
        let stated = match self.stated.load(Ordering::Relaxed) {
            0 => None,
            n => Some(n),
        };
        let source = match (stated, self.contract) {
            (Some(n), Some(c)) if (1..=c).contains(&n) => Source::Descriptor,
            _ => Source::Contract,
        };
        (zenkey_model::budget::bound(self.contract, [stated]), source)
    }

    /// Whether a member last written at `last` is live at `now`.
    fn is_live(&self, m: &Member, now: Instant) -> bool {
        self.span
            .is_none_or(|span| now.saturating_duration_since(m.last) < span)
    }

    /// Whether a member is worth keeping at `now`: live, or with an
    /// occurrence within the rate's period.
    fn is_kept(&self, m: &Member, now: Instant) -> bool {
        self.is_live(m, now)
            || self
                .rate
                .is_some_and(|(_, _, period)| now.saturating_duration_since(m.last) < period)
    }

    /// The members live at `now`.
    #[must_use]
    pub fn live(&self, now: Instant) -> u64 {
        let m = self.members.lock().expect("not poisoned");
        m.map.values().filter(|e| self.is_live(e, now)).count() as u64
    }

    /// Forgets every member no longer kept, and moves the next sweep.
    fn sweep(&self, m: &mut Members, now: Instant) {
        m.map.retain(|_, e| self.is_kept(e, now));
        m.sweep_at = (m.map.len() * 2).max(SWEEP_FLOOR);
    }

    /// A write on `member` at `now`: a state put, a stream publication, an
    /// event occurrence (§2.7). Refused when it would make a new member
    /// live while the live members are at the bound, or, for an event, put
    /// an occurrence beyond the member's rate; otherwise recorded.
    pub fn admit(&self, member: &str, now: Instant) -> Result<(), Refusal> {
        let (bound, source) = self.bound();
        let limit = bound.limit();
        if limit.is_none() && self.rate.is_none() {
            return Ok(());
        }
        let mut m = self.members.lock().expect("not poisoned");
        let live = m.map.get(member).is_some_and(|e| self.is_live(e, now));
        if !live && let Some(n) = limit {
            if m.map.len() as u64 >= n {
                self.sweep(&mut m, now);
            }
            let count = m.map.values().filter(|e| self.is_live(e, now)).count() as u64;
            if count >= n {
                return Err(self.refusal(
                    member,
                    Breach::Bound {
                        bound: n,
                        source,
                        live: count,
                    },
                ));
            }
        }
        if m.map.len() >= m.sweep_at {
            self.sweep(&mut m, now);
        }
        let e = m.map.entry(member.to_owned()).or_insert_with(|| Member {
            last: now,
            recent: VecDeque::new(),
        });
        if let Some((rate, allowed, period)) = self.rate {
            while e
                .recent
                .front()
                .is_some_and(|t| now.saturating_duration_since(*t) >= period)
            {
                e.recent.pop_front();
            }
            if e.recent.len() >= allowed as usize {
                return Err(self.refusal(
                    member,
                    Breach::Rate {
                        rate,
                        allowed,
                        period,
                    },
                ));
            }
            e.recent.push_back(now);
        }
        e.last = now;
        Ok(())
    }

    /// A state member deleted (§2.7: a deleted key is not live).
    pub fn release(&self, member: &str) {
        if self.kind == Kind::State {
            self.members
                .lock()
                .expect("not poisoned")
                .map
                .remove(member);
        }
    }

    /// The descriptor lowers the bound to `n` (§3.3). Refused, the bound
    /// unchanged, when more than `n` members are live at `now`: an owner
    /// MUST NOT hold more than its bound (§2.7).
    pub fn lower(&self, n: u64, now: Instant) -> Result<(), Refusal> {
        let before = self.stated.swap(n, Ordering::Relaxed);
        let (bound, source) = self.bound();
        let live = self.live(now);
        if let Some(b) = bound.limit()
            && live > b
        {
            self.stated.store(before, Ordering::Relaxed);
            return Err(self.refusal(
                "the lowered bound",
                Breach::Bound {
                    bound: b,
                    source,
                    live,
                },
            ));
        }
        Ok(())
    }

    fn refusal(&self, member: &str, breach: Breach) -> Refusal {
        Refusal {
            iface: self.iface.clone(),
            resource: self.resource.clone(),
            member: member.to_owned(),
            breach,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(text: &str) -> (IfaceId, Resource) {
        let l = zenkey_model::contract::load_str(
            &format!("[interface]\nname = \"b\"\nmajor = 1\n{text}"),
            std::path::Path::new("."),
            None,
        );
        let c = l.contract.unwrap_or_else(|| panic!("{}", l.report));
        (c.iface.clone(), c.resources[0].clone())
    }

    fn budget(text: &str) -> Budget {
        let (iface, r) = resource(text);
        Budget::new(&iface, &r).expect("a data resource")
    }

    const STATE: &str = "[resources.\"items/{item}\"]\nkind = \"state\"\n\
        type = { raw = \"text/plain\" }\nparams = { item = \"string\" }\ncardinality = 2\n";
    const STREAM: &str = "[resources.\"load/{cpu}\"]\nkind = \"stream\"\n\
        type = { raw = \"text/plain\" }\nparams = { cpu = \"string\" }\ncardinality = 2\n";
    const EVENT: &str = "[resources.\"alarms/{src}\"]\nkind = \"event\"\n\
        type = { raw = \"text/plain\" }\nparams = { src = \"string\" }\ncardinality = 2\n\
        rate = \"burst(2/h)\"\nretention = \"10s\"\n";

    fn s(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn a_state_member_is_live_until_deleted() {
        let b = budget(STATE);
        let t = Instant::now();
        b.admit("a", t).unwrap();
        b.admit("b", t).unwrap();
        let e = b.admit("c", t + s(7200)).unwrap_err();
        assert_eq!(
            e.breach,
            Breach::Bound {
                bound: 2,
                source: Source::Contract,
                live: 2
            }
        );
        assert!(
            e.to_string().contains("at its bound of 2 (the contract's)"),
            "{e}"
        );
        b.admit("a", t + s(7200))
            .expect("a live key adds no member");
        b.release("b");
        b.admit("c", t + s(7200)).expect("a delete frees one");
        assert_eq!(b.live(t), 2);
    }

    #[test]
    fn a_stream_member_is_forgotten_an_hour_after_its_last_publication() {
        let b = budget(STREAM);
        let t = Instant::now();
        b.admit("a", t).unwrap();
        b.admit("b", t + s(10)).unwrap();
        assert!(b.admit("c", t + s(3599)).is_err());
        // Exactly an hour after `a`: no longer live.
        b.admit("c", t + s(3600)).expect("a forgotten");
        assert!(b.admit("a", t + s(3601)).is_err(), "a is new again");
        b.release("b");
        assert!(b.admit("d", t + s(3601)).is_err(), "a stream has no delete");
    }

    #[test]
    fn an_event_keeps_its_population_and_each_members_rate() {
        let b = budget(EVENT);
        let t = Instant::now();
        b.admit("a", t).unwrap();
        b.admit("b", t).unwrap();
        assert!(matches!(
            b.admit("c", t + s(9)).unwrap_err().breach,
            Breach::Bound { .. }
        ));
        b.admit("c", t + s(10))
            .expect("a and b past their retention");
        b.admit("a", t + s(15))
            .expect("a new again, its second in the hour");
        // burst(2/h): `a`'s third within the hour, though its retention
        // lapsed between the first two.
        let e = b.admit("a", t + s(16)).unwrap_err();
        assert!(matches!(e.breach, Breach::Rate { .. }), "{e}");
        let e = b.admit("d", t + s(16)).unwrap_err();
        assert!(
            matches!(e.breach, Breach::Bound { live: 2, .. }),
            "c and a live: {e}"
        );
        let b = budget(EVENT);
        b.admit("a", t).unwrap();
        b.admit("a", t + s(1)).unwrap();
        let e = b.admit("a", t + s(2)).unwrap_err();
        assert!(matches!(e.breach, Breach::Rate { allowed: 2, .. }), "{e}");
        assert!(e.to_string().contains("beyond its rate burst(2/h)"), "{e}");
        b.admit("b", t + s(2)).expect("the rate bounds each member");
        b.admit("a", t + s(3600)).expect("an hour after the first");
    }

    #[test]
    fn the_descriptor_lowers_it_and_no_ceiling_costs_nothing() {
        let b = budget(STATE);
        let t = Instant::now();
        b.lower(1, t).unwrap();
        assert_eq!(b.bound(), (Bound::Of(1), Source::Descriptor));
        b.admit("a", t).unwrap();
        let e = b.admit("b", t).unwrap_err();
        assert!(
            e.to_string().contains("bound of 1 (the descriptor's)"),
            "{e}"
        );
        b.lower(2, t).unwrap();
        b.admit("b", t).unwrap();
        assert!(
            b.lower(1, t).is_err(),
            "two live, a lowering to one refused"
        );
        assert_eq!(b.bound().0, Bound::Of(2), "and the bound unchanged");
        let none = budget(&STATE.replace("cardinality = 2", "cardinality = 4294967295"));
        for i in 0..1000 {
            none.admit(&format!("m{i}"), t).unwrap();
        }
        assert_eq!(none.live(t), 0, "no ceiling tracks nothing");
        // `"unbounded"` is the same value (core §2.2, 0.57; #937).
        let named = budget(&STATE.replace("cardinality = 2", "cardinality = \"unbounded\""));
        for i in 0..1000 {
            named.admit(&format!("m{i}"), t).unwrap();
        }
        assert_eq!(named.bound().0, Bound::NoCeiling);
        assert_eq!(named.live(t), 0, "\"unbounded\" tracks nothing either");
    }

    #[test]
    fn the_bookkeeping_is_swept() {
        let b = budget(&STREAM.replace("cardinality = 2", "cardinality = 1000000"));
        let t = Instant::now();
        for i in 0..10_000u64 {
            // Each member live a second only past the hour of the one before.
            b.admit(&format!("m{i}"), t + s(3600 * i)).unwrap();
        }
        let len = b.members.lock().unwrap().map.len();
        assert!(len <= 2 * SWEEP_FLOOR, "{len} members kept");
    }
}
