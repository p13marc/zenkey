//! The watchdog plane: a rule's state over a window, every transition of it,
//! and the summary a long run collapses to.
//!
//! [`Transition`] is the shape a watchdog emits per change rather than per
//! tick, which is what makes an hours-long run readable — and
//! [`WatchdogSummary`] carries what the run could *not* see, because a
//! watchdog that dropped samples has not been quiet, it has been blind
//! (RFC 09 §5.1 O6).
//!
//! `CondWindow` — the raw observation a [`CondState`] is judged from — is
//! deliberately *not* here: it carries no `Serialize`, so it is
//! [`crate::judge::condition`]'s own working value, not a contract.

use serde::Serialize;

use super::judgement::Judgement;

/// One condition's evaluation state — the watchdog's serde-stable **wire
/// projection** of the [`Judgement`] core (RFC 13, v1.24; RFC 09 §5.1
/// pre-v1.24). Three states, not two: `unobservable` is "I could not tell",
/// which is neither "fine" nor "fire".
///
/// The mapping (see [`From<Judgement>`](#impl-From<Judgement>-for-CondState)),
/// with the **polarity note spelled out**: a [`Condition`](crate::judge::condition::Condition) names what
/// *firing* means, so `CondState::Ok` means **the condition does not hold**
/// — it is `Established(no)` / [`Judgement::NotEstablished`], not a bare
/// "fine". `Firing` is `Established(yes)`; both `NotAsked` and
/// `Unobservable` project to `unobservable`, because this wire vocabulary
/// predates the NotAsked pole and the watchdog evaluates every declared rule
/// every tick — it never leaves one unasked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CondState {
    /// The condition conclusively does not hold ([`Judgement::NotEstablished`]
    /// — note the polarity: `ok` is the *established-clean* pole).
    Ok,
    /// The condition conclusively holds ([`Judgement::Established`]).
    Firing,
    /// The observation cannot carry the claim: a drop under a completeness
    /// claim, a window shorter than the claim's span, or an ask that failed
    /// ([`Judgement::Unobservable`]; a hypothetical [`Judgement::NotAsked`]
    /// also lands here — the wire cannot say more).
    Unobservable,
}

/// The documented wire projection (RFC 13, v1.24): `Established` → `firing`,
/// `NotEstablished` → `ok` (the polarity note on [`CondState`]), both
/// unestablished poles → `unobservable`.
impl From<&Judgement> for CondState {
    fn from(j: &Judgement) -> CondState {
        match j {
            Judgement::Established => CondState::Firing,
            Judgement::NotEstablished { .. } => CondState::Ok,
            Judgement::NotAsked | Judgement::Unobservable { .. } => CondState::Unobservable,
        }
    }
}

impl From<Judgement> for CondState {
    fn from(j: Judgement) -> CondState {
        CondState::from(&j)
    }
}

/// One genuine state change — the only thing the watchdog ever emits.
#[derive(Debug, Clone, Serialize)]
pub struct Transition {
    /// The rule, in its canonical spelling ([`Condition`](crate::judge::condition::Condition)'s `Display`).
    pub rule: String,
    /// `null` on the first evaluation: the baseline stated out loud, because
    /// inventing a prior state would answer a question nobody asked (O4).
    pub from: Option<CondState>,
    pub to: CondState,
    /// RFC 3339 wall clock.
    pub at: String,
    pub evidence: String,
}

/// What a bounded watchdog run cost and said.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct WatchdogSummary {
    pub ticks: u64,
    pub transitions: u64,
    /// Key projections the bounded facts cache retired to stay within its
    /// bound (RFC 09 §5.1 O6). The watchdog is the run-forever mode, so its
    /// per-key cache is a [`crate::model::facts::FactsCache`], not a map that grows
    /// one entry per distinct key ever seen — and a bound must count what it
    /// cost. An evicted key re-observed is re-projected identically (the
    /// projection is a pure function of key and slice set), so evictions
    /// cost recompute, never a changed verdict.
    pub facts_evicted: u64,
}
