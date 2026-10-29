//! `timing.v1` in zengui (#908, MC): a selected key's timed stream, its
//! declared period, deadline and lifespan, and whether it met its deadline
//! and kept its period over the slot's recorded history — **the window's
//! own view**, never a verdict about the deployment.
//!
//! **Session-free, and every verdict the model's.** [`read`] hands the
//! slot's history ring, oldest first, to the model's
//! [`Tracker`](zenkey_model::timing::Tracker), with the owner's state as
//! the presence link said it at the instants this window recorded
//! ([`OwnerTrack`]), then judges §5's deadline
//! and period questions with `deadline_met` and `period_kept` over
//! `period_window`. Nothing is computed here but what to hand the model.
//!
//! - **Arrivals.** Each ring entry's `received` is the instant the data
//!   pump's subscriber callback read (`zenkey_fleet::SampleView::of`), on
//!   this host's monotonic clock: what §2.5 judges a deadline on. A replayed
//!   row's or a snapshot's is not an arrival of the live member, and the
//!   slot says so: a replay reads unobservable here.
//! - **The window** is the ring's: from its oldest retained entry to now.
//!   The member is judged from its first arrival in it (§2.5's "any other
//!   member"): the ring does not know when the subscription started, so it
//!   never waits for a member it has not heard.
//! - **The owner** is the presence link's, recorded as it landed (at most
//!   four times a second), so a state's instant may be up to a landing late,
//!   and presence and samples arrive in no one order (§2.9 of `telemetry.v1`
//!   says why `crate::metric` reads none). A miss reads the owner's state at
//!   the last landing before it; a window that saw the owner other than
//!   present at any landing is not "present throughout".
//! - **Lost.** A tick the pump's broadcast lagged on (`BusTick::lagged`)
//!   while the slot recorded is a loss: a miss or a period not kept is then
//!   never the owner's (§2.7).
//! - **The lifespan** is not judged: it needs the deployment's ε, which
//!   zengui is not told, and every sample would read `no_bound` (§2.6).

use std::time::{Duration, Instant};

use zenkey_fleet::{Resolved, TimedResource};
use zenkey_model::authoring::Kind;
use zenkey_model::timing::{
    DeadlineAccount, Declaration, Declared, Event, Judged, Mono, OwnerState, PeriodReading, Report,
    Window, deadline_met, period_kept, period_window, tracker_for,
};

use crate::history::HistoryRing;

/// How many owner states a slot keeps: the latest, each a landing's.
pub const OWNER_STATES_KEPT: usize = 256;

/// How many misses a read lists, the latest; every one is counted.
pub const MISSES_KEPT: usize = 4;

/// The owner's state as the presence link said it, landing by landing, for
/// one slot's key.
#[derive(Debug, Clone, Default)]
pub struct OwnerTrack {
    /// `(instant it landed, state)`, oldest first, each a change.
    states: Vec<(Instant, OwnerState)>,
}

impl OwnerTrack {
    /// The state at a landing at `at`; kept only when it changed.
    pub fn record(&mut self, at: Instant, state: OwnerState) {
        if self.states.last().map(|(_, s)| *s) == Some(state) {
            return;
        }
        if self.states.len() >= OWNER_STATES_KEPT {
            self.states.remove(0);
        }
        self.states.push((at, state));
    }

    /// The state the last landing at or before `at` said: unseen before any.
    pub fn at(&self, at: Instant) -> OwnerState {
        self.states
            .iter()
            .rev()
            .find(|(t, _)| *t <= at)
            .map_or(OwnerState::Unseen, |(_, s)| *s)
    }

    /// The state now.
    pub fn now(&self) -> OwnerState {
        self.states.last().map_or(OwnerState::Unseen, |(_, s)| *s)
    }

    /// The changes after `from`, in order.
    fn after(&self, from: Instant) -> impl Iterator<Item = &(Instant, OwnerState)> {
        self.states.iter().filter(move |(t, _)| *t > from)
    }
}

/// The owner of `iface` at `addr` as a presence projection shows it (v1.md
/// §0): present while an instance holds the interface's token, absent when
/// none does and its basis is a complete read with no withdrawal awaiting
/// one, unconfirmed between, unseen with no projection.
pub fn owner_of(
    presence: Option<&zenkey_fleet::LivePresence>,
    addr: &zenkey_model::grammar::Addr,
    iface: &zenkey_model::grammar::IfaceId,
) -> OwnerState {
    let Some(p) = presence else {
        return OwnerState::Unseen;
    };
    let a = p.address(addr);
    if a.is_some_and(|a| {
        a.instances
            .values()
            .any(|i| i.interfaces.contains_key(iface))
    }) {
        return OwnerState::Present;
    }
    let whole = p.basis() == zenkey_fleet::model::presence_live::PresenceBasis::Complete;
    if whole && !a.is_some_and(|a| a.withdrawn) {
        OwnerState::Absent
    } else {
        OwnerState::Unconfirmed
    }
}

/// What the window's own view read of one timed member.
#[derive(Debug, Clone)]
pub struct TimingRead {
    pub declaration: Declaration,
    /// The revision that declared it, short.
    pub revision: String,
    /// Arrivals in the window: put samples.
    pub arrivals: usize,
    /// The window: the oldest retained entry's arrival to now.
    pub span: Duration,
    /// §5's deadline question; `None` when the read is not made (a replay).
    pub deadline: Option<Judged>,
    /// §5's period question.
    pub period: Option<Judged>,
    pub period_reading: Option<PeriodReading>,
    /// Misses reported, and the latest [`MISSES_KEPT`]: `(due, its
    /// lateness, the owner's state)`, the due in milliseconds from the
    /// window's start.
    pub misses: u32,
    pub latest_misses: Vec<(f64, f64, OwnerState)>,
    /// The owner's state now, as the presence link says it.
    pub owner_now: OwnerState,
    /// Why nothing is judged, when nothing is.
    pub unobservable: Option<&'static str>,
}

impl TimingRead {
    /// The declared values in words, with their source.
    pub fn declared(&self) -> Option<&Declared> {
        self.declaration.declared()
    }
}

/// The timed resource `resolved` names, when it is one.
pub fn timed(resolved: &Resolved) -> Option<TimedResource> {
    TimedResource::of(&resolved.address, &resolved.revision, resolved.index())
}

/// Reads `resolved`'s member over `ring` (see the module doc), at `now`.
/// `replayed` reads nothing: a replayed row's instant is no arrival.
pub fn read(
    resolved: &Resolved,
    ring: &HistoryRing,
    owner: &OwnerTrack,
    lost: bool,
    replayed: bool,
    now: Instant,
) -> Option<TimingRead> {
    let r = timed(resolved)?;
    let fp = resolved.revision.fingerprint().to_string();
    let revision = fp.split_once(':').map_or(fp.clone(), |(_, h)| {
        format!("sha256:{}", &h[..16.min(h.len())])
    });
    let mut out = TimingRead {
        declaration: r.declaration.clone(),
        revision,
        arrivals: 0,
        span: Duration::ZERO,
        deadline: None,
        period: None,
        period_reading: None,
        misses: 0,
        latest_misses: Vec::new(),
        owner_now: owner.now(),
        unobservable: None,
    };
    if replayed {
        out.unobservable = Some(
            "a replay's rows carry no arrival of the live member, so no deadline or period is \
             read here",
        );
        return Some(out);
    }
    // The ring iterates newest first: its arrivals, oldest first.
    let arrivals: Vec<Instant> = ring
        .iter()
        .rev()
        .filter(|e| !e.is_delete)
        .map(|e| e.received)
        .collect();
    out.arrivals = arrivals.len();
    let Some(&origin) = arrivals.first() else {
        out.unobservable =
            Some("nothing recorded yet: the member is judged from its first arrival");
        return Some(out);
    };
    let mono = |t: Instant| Mono::after(t.saturating_duration_since(origin));
    let end = mono(now.max(origin));
    out.span = end.since_origin();
    let window = Window {
        across_face: false,
        lost,
        owner_present: owner.at(origin) == OwnerState::Present
            && owner.after(origin).all(|(_, s)| *s == OwnerState::Present),
    };
    let key = resolved.key.as_str();
    let mut account = DeadlineAccount::default();
    if let Some(mut t) = tracker_for(Kind::Stream, &r.declaration, None, Vec::<String>::new()) {
        let mut reports: Vec<Report> = Vec::new();
        reports.extend(t.observe(Mono::ZERO, Event::Owner(owner.at(origin))));
        reports.extend(t.observe(Mono::ZERO, Event::Start));
        // Arrivals and the owner's landings in instant order.
        let mut owners = owner.after(origin).peekable();
        for a in &arrivals {
            while let Some((at, s)) = owners.next_if(|(at, _)| at <= a) {
                reports.extend(t.observe(mono(*at), Event::Owner(*s)));
            }
            reports.extend(t.observe(
                mono(*a),
                Event::Arrival {
                    member: key,
                    expired: false,
                },
            ));
        }
        for (at, s) in owners {
            if *at <= now {
                reports.extend(t.observe(mono(*at), Event::Owner(*s)));
            }
        }
        reports.extend(t.observe(end, Event::Tick));
        for rep in &reports {
            account.count(rep);
            if let Report::Missed {
                at, due, owner: o, ..
            } = rep
            {
                out.misses += 1;
                out.latest_misses.push((
                    due.since_origin().as_secs_f64() * 1e3,
                    at.saturating_since(*due).as_secs_f64() * 1e3,
                    *o,
                ));
            }
        }
        let keep = out.latest_misses.len().saturating_sub(MISSES_KEPT);
        out.latest_misses.drain(..keep);
    }
    account.held = out.span;
    out.deadline = Some(deadline_met(
        Kind::Stream,
        &r.declaration,
        &window,
        &account,
    ));
    let reading = r.period().map(|p| {
        period_window(
            p,
            Mono::ZERO,
            end,
            false,
            None,
            &arrivals.iter().map(|a| mono(*a)).collect::<Vec<_>>(),
        )
    });
    out.period = Some(period_kept(
        Kind::Stream,
        &r.declaration,
        &window,
        reading.map_or(zenkey_model::timing::PeriodVerdict::NotHeard, |p| p.verdict),
    ));
    out.period_reading = reading;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The owner track keeps changes only, and answers by landing.
    #[test]
    fn an_owner_track_answers_by_its_last_landing() {
        let t0 = Instant::now();
        let mut o = OwnerTrack::default();
        assert_eq!(o.at(t0), OwnerState::Unseen);
        o.record(t0, OwnerState::Present);
        o.record(t0 + Duration::from_millis(250), OwnerState::Present);
        o.record(t0 + Duration::from_millis(500), OwnerState::Unconfirmed);
        assert_eq!(
            o.states.len(),
            2,
            "a landing that changes nothing is not kept"
        );
        assert_eq!(o.at(t0 + Duration::from_millis(400)), OwnerState::Present);
        assert_eq!(o.now(), OwnerState::Unconfirmed);
    }
}
