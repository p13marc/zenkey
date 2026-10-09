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

use serde::{Deserialize, Serialize};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
///
/// `Deserialize` too, since v1.34: a version-2 `.zrec` interleaves the
/// transition that fired a trigger capture as a `{"trigger": …}` record
/// (RFC 13 §4.1), and a reader hands it back as this same shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize)]
pub struct WatchdogSummary {
    pub ticks: u64,
    pub transitions: u64,
    /// The rules whose last evaluation was `firing`, in their canonical
    /// spelling (#511) — what a bounded run *ended* on, which the
    /// transition stream only says to a reader who replays all of it.
    pub firing: Vec<String>,
    /// The rules whose last evaluation was `unobservable`, or that were
    /// never evaluated at all — likewise canonical (#511).
    pub unobservable: Vec<String>,
}

impl WatchdogSummary {
    /// How the run ended, as one RFC 13 §1.2 [`Judgement`] (#511): any rule
    /// firing is `Established` — the finding; otherwise any rule
    /// unobservable is `Unobservable`, since a rule nobody could judge has
    /// not said the fleet is fine; otherwise every rule ended `ok`, which is
    /// `NotEstablished`. The polarity is [`CondState`]'s: a condition names
    /// what *firing* means.
    pub fn judgement(&self) -> Judgement {
        if !self.firing.is_empty() {
            return Judgement::Established;
        }
        if !self.unobservable.is_empty() {
            return Judgement::Unobservable {
                reason: format!(
                    "{} rule(s) ended unobservable: {}",
                    self.unobservable.len(),
                    self.unobservable.join("; ")
                ),
            };
        }
        Judgement::NotEstablished {
            reason: "every rule ended ok".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::judgement_exit_code;

    fn summary(firing: &[&str], unobservable: &[&str]) -> WatchdogSummary {
        WatchdogSummary {
            ticks: 2,
            transitions: 3,
            firing: firing.iter().map(|r| r.to_string()).collect(),
            unobservable: unobservable.iter().map(|r| r.to_string()).collect(),
        }
    }

    /// The summary's wire shape, pinned now that it carries the end states
    /// (#511) — appended after the counters it always had.
    #[test]
    fn watchdog_summary_json_shape_is_pinned() {
        assert_eq!(
            serde_json::to_value(summary(&["rate-above v1/** 5"], &["dropped"])).unwrap(),
            serde_json::json!({
                "ticks": 2,
                "transitions": 3,
                "firing": ["rate-above v1/** 5"],
                "unobservable": ["dropped"],
            })
        );
    }

    /// #511: firing wins (1), then unobservable (2), then clean (0) — the
    /// order a cron job reading `$?` needs.
    #[test]
    fn a_bounded_run_ends_firing_then_unobservable_then_clean() {
        let exit = |s: &WatchdogSummary| judgement_exit_code(&s.judgement());
        assert_eq!(exit(&summary(&["a"], &["b"])), 1);
        assert_eq!(exit(&summary(&[], &["b"])), 2);
        assert_eq!(exit(&summary(&[], &[])), 0);
        assert_eq!(
            summary(&[], &["b", "c"]).judgement(),
            Judgement::Unobservable {
                reason: "2 rule(s) ended unobservable: b; c".into()
            }
        );
    }
}
