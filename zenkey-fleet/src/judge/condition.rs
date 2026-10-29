//! Conditions and the watchdog (#227) — transitions, not states.
//!
//! Three shipped features each hard-coded their own predicate over the
//! observation surface: `expect` (one window), `doctor --for` (five
//! checks), `cutover` (silence). This module is the one **closed vocabulary**
//! they were each a spelling of: [`Condition`], evaluated to three states,
//! never two (O4/O6) — `ok` / `firing` / **`unobservable`**. The third state
//! is the reason this exists: an alerting tool that cannot say *"I could not
//! tell"* is the one that pages at 3am for a dropped buffer. A drop under a
//! completeness claim yields `unobservable`, never `ok`.
//!
//! The vocabulary is deliberately closed — no expressions, no templating, no
//! rules engine. A new condition is a new variant, argued for the way a new
//! doctor check id is.
//!
//! **zk2** (#612, FJ8b). The selectors are wire keys, watched on a session
//! in no namespace, and every condition that needs to know what a key *is*
//! reads it through the raw observers' [`Lens`]: `invalid-payload` decodes
//! a payload through its contract and checks it against its declared type
//! (spec §7.2, §7.3); `qos-mismatch` compares the QoS a sample rode with
//! the one its resource declares (§2.4); `instance-gone` — v1's
//! `origin-down`, renamed for what it asks in zk2 — reads the live presence
//! projection per address ([`AddressWatch`], #747), the reading zenwatch's
//! rule of the same name makes, and folds it into the rule's one state;
//! `doctor` runs FJ6's doctor; `alert-firing` — v1's spelling, back on
//! `alarms.v1` (#800, AC) — reads the alarms its selector reaches through a
//! live reader in the namespace ([`AlarmFeed`]: subscribed, seeded by S4's
//! GET, re-read beside the drain each tick) and is §5's second question at
//! its floor: firing on *yes*, ok on *no*, and unobservable on every other
//! answer, never ok.
//!
//! The semantic core is three tiny rules — [`judge_shortfall`],
//! [`judge_excess`], [`judge_silence`] — shared with [`crate::judge::expect`], so
//! the watchdog and the CI assertion cannot drift about what a drop means.
//! The rules speak the four-pole [`Judgement`] core, and [`CondState`] is
//! this module's serde-stable **wire projection** of it — see its mapping
//! doc.
//!
//! [`watchdog`] is the continuous observer over the vocabulary:
//! **foreground, explicitly launched, single-purpose, one process per
//! invocation, no shared state** — not the hidden, auto-started,
//! discovery-caching daemon the redesign ledger rejected
//! (`docs/redesign-2026-07.md` §6.1). It emits [`Transition`]s: one per
//! genuine state change, none per unchanged tick.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use crate::{Error, Result};

use crate::bus::alarms::{AlarmFeed, AlarmRefresh, AlarmSpec, Refreshed};
use crate::bus::contracts::BundleStore;
use crate::bus::monitor::SampleView;
use crate::bus::presence::{PresenceFeed, Scope};
use crate::judge::doctor::DoctorBus;
use crate::model::alarms::{AlarmBook, AlarmScope};
use crate::model::catalog::{Catalog, ContractSet};
use crate::model::lens::{Lens, observed_qos, qos_mismatch};
use crate::model::presence_live::{LivePresence, PresenceBasis};
use crate::model::presence_watch::{AddressWatch, covering_scope};
use crate::model::render::Member;
use crate::model::target::Target;
use crate::report::{CheckId, Conformance, DoctorReport, PresenceTransition};
use crate::report::{CondState, Judgement, Transition, WatchdogSummary};
use sipper::{Straw, sipper};

/// The closed condition vocabulary (#227), over the observation surface.
/// Each variant names what *firing* means; the drop rules are in the judge
/// functions this module documents.
#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    /// Samples on `selector` rode above `hz` over the evaluation window.
    /// Firing is positive evidence, conclusive even under drops (a drop only
    /// hides more); `ok` under drops is unobservable — the true rate is
    /// higher than what was counted (O6).
    RateAbove { selector: String, hz: f64 },
    /// Samples on `selector` rode below `hz`. A shortfall under drops is
    /// unobservable — the dropped samples could have filled it (O6); enough
    /// observed is conclusive `ok` regardless.
    RateBelow { selector: String, hz: f64 },
    /// No sample matched `selector` for at least `for_s` seconds. Silence is
    /// a completeness claim — it counts what did NOT happen — so it is
    /// provable only over a drop-free span at least `for_s` long (O6), and
    /// only once the observer has watched that long (O4).
    SilentFor { selector: String, for_s: f64 },
    /// An observed payload on `selector` failed its declared type: bytes
    /// that do not decode as it, or a JSON Schema value that does not
    /// satisfy it (spec §7.2, §7.3). Firing on positive evidence; `ok`
    /// claims "nothing checked failed", never "nothing invalid rode" — the
    /// drop count rides the evidence. A window in which nothing could be
    /// checked (no sample, or no contract resolved one) is unobservable,
    /// with why.
    InvalidPayload { selector: String },
    /// An observed sample on `selector` rode a priority, congestion control
    /// or express other than its resource declares (spec §2.4: an owner
    /// MUST publish with it). Same per-observed-sample scope as
    /// [`Condition::InvalidPayload`]; a window with nothing judged is
    /// unobservable.
    QosMismatch { selector: String },
    /// A check of zk2's doctor established a finding (the stable
    /// [`crate::report::CheckId`] vocabulary, #612 FJ6). A check the run
    /// could not establish, and a failed run, are unobservable — never `ok`.
    DoctorCheck { check: CheckId },
    /// An address the pattern — `<system>/<service>`, either position `*`
    /// — watches holds no instance token visible to this reader, a complete
    /// read behind it (spec §8.1): zk2's spelling of v1's `origin-down`.
    /// Judged per address on the live presence projection
    /// ([`AddressWatch`], #747), as zenwatch's rule is: under a wildcard an
    /// address seen held that goes fires, even while others hold, and a
    /// pattern never matched fires nothing; an exact address is watched from
    /// the start, so one absent from a complete read fires. The rule's one
    /// state is firing while any address is gone, else unobservable while
    /// the projection cannot see (a read that ended at its timeout, events
    /// lost), else ok. A read access control refused is complete and empty,
    /// like absence, so the evidence says what this reader could see (0.8).
    InstanceGone { address: String },
    /// An alarm firing at or above `at_least` under `selector`: §5's second
    /// question of `alarms.v1` (§2.15; #800, AC), v1's `alert-firing` on
    /// zk2's profile. The selector is base-relative (`zk2/…`) and reaches
    /// alarm keys, read in the deployment's namespace by a live reader:
    /// subscribed, seeded by S4's GET and re-read each tick. *Yes* is
    /// firing, whatever else is known; *no* is ok, only when every owner in
    /// scope is established and none of its alarms that may be at or above
    /// the floor is unobservable; unobservable and not asked (no owner
    /// established, its owners absent or not listing `alarms.v1`) are
    /// unobservable — never ok. A loss makes the rule unobservable until a
    /// GET heals it.
    AlertFiring {
        selector: String,
        at_least: zenkey_model::alarms::Severity,
    },
    /// The observer itself dropped samples this window (O6) — self-knowledge,
    /// so never unobservable.
    Dropped,
}

/// The rule grammar, spelled once for the parse error and the docs.
const VOCABULARY: &str = "rate-above <SEL> <HZ> | rate-below <SEL> <HZ> | \
     silent-for <SEL> <SECS> | invalid-payload <SEL> | qos-mismatch <SEL> | \
     doctor <CHECK-ID> | instance-gone <SYSTEM/SERVICE> | \
     alert-firing <SEL> [info|warning|critical] | dropped";

impl Condition {
    /// Parse one rule: whitespace-separated, kind first (Zenoh key
    /// expressions cannot contain whitespace, so the split is unambiguous).
    /// The vocabulary is closed; anything else is an error that spells it.
    pub fn parse(rule: &str) -> Result<Condition> {
        let hz = |s: &str, kind: &str| -> Result<f64> {
            let v: f64 = s
                .parse()
                .map_err(|_| Error::unaskable(format!("{kind} {s:?}"), "is not a number"))?;
            if !v.is_finite() || v < 0.0 {
                return Err(Error::unaskable(
                    kind.to_string(),
                    "the threshold must be a finite non-negative number",
                ));
            }
            Ok(v)
        };
        let tokens: Vec<&str> = rule.split_whitespace().collect();
        Ok(match tokens.as_slice() {
            ["rate-above", sel, n] => Condition::RateAbove {
                selector: sel.to_string(),
                hz: hz(n, "rate-above")?,
            },
            ["rate-below", sel, n] => Condition::RateBelow {
                selector: sel.to_string(),
                hz: hz(n, "rate-below")?,
            },
            ["silent-for", sel, n] => {
                let for_s = hz(n, "silent-for")?;
                if for_s <= 0.0 {
                    return Err(Error::unaskable(
                        "silent-for",
                        "the span must be a positive number of seconds",
                    ));
                }
                Condition::SilentFor {
                    selector: sel.to_string(),
                    for_s,
                }
            }
            ["invalid-payload", sel] => Condition::InvalidPayload {
                selector: sel.to_string(),
            },
            ["qos-mismatch", sel] => Condition::QosMismatch {
                selector: sel.to_string(),
            },
            ["doctor", check] => {
                let Some(check) = CheckId::parse(check) else {
                    return Err(Error::unaskable(
                        format!("doctor {check:?}"),
                        format!(
                            "is not a check id — the stable vocabulary is: {}",
                            CheckId::ALL
                                .iter()
                                .map(|c| c.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    ));
                };
                Condition::DoctorCheck { check }
            }
            ["instance-gone", address] => {
                Target::parse(address)?;
                Condition::InstanceGone {
                    address: address.to_string(),
                }
            }
            ["dropped"] => Condition::Dropped,
            ["origin-down", ..] => {
                return Err(Error::unaskable(
                    format!("{rule:?}"),
                    "v1's origin-down read v1's liveliness roster; in zk2 it is \
                     instance-gone <SYSTEM/SERVICE>: an address with no instance token \
                     visible to this reader (spec §8.1)",
                ));
            }
            ["alert-firing", sel] | ["alert-firing", sel, _] => {
                AlarmScope::parse(sel).map_err(|why| Error::unaskable("alert-firing", why))?;
                let at_least = match tokens.get(2) {
                    None => zenkey_model::alarms::Severity::Warning,
                    Some(s) => zenkey_model::alarms::Severity::parse(s).ok_or_else(|| {
                        Error::unaskable(
                            format!("alert-firing {s:?}"),
                            "is not a severity: info, warning or critical (alarms.v1 §2.4)",
                        )
                    })?,
                };
                Condition::AlertFiring {
                    selector: sel.to_string(),
                    at_least,
                }
            }
            _ => {
                return Err(Error::unaskable(
                    format!("{rule:?}"),
                    format!(
                        "is not a rule — the vocabulary is closed (no \
                         expressions, no templating): {VOCABULARY}"
                    ),
                ));
            }
        })
    }

    /// The wire selector this condition observes, when it observes one.
    pub fn selector(&self) -> Option<&str> {
        match self {
            Condition::RateAbove { selector, .. }
            | Condition::RateBelow { selector, .. }
            | Condition::SilentFor { selector, .. }
            | Condition::InvalidPayload { selector }
            | Condition::QosMismatch { selector } => Some(selector),
            _ => None,
        }
    }

    /// Judge this condition against everything one tick observed.
    ///
    /// **The single entry point** (#352): this match *is* the partition,
    /// and each arm hands its judge exactly the evidence that judge needs.
    pub fn judge(&self, ev: &TickEvidence<'_>) -> Eval {
        match self {
            Condition::DoctorCheck { check } => judge_doctor_check(*check, ev.doctor),
            Condition::InstanceGone { .. } => judge_instance_gone(ev.addresses, ev.presence),
            Condition::AlertFiring { selector, at_least } => {
                judge_alert_firing(selector, *at_least, ev.alarms)
            }
            _ => self.judge_window_total(ev.window, ev.examples),
        }
    }

    /// Judge one observation window. `None` for the conditions that are not
    /// window-scoped ([`Condition::DoctorCheck`],
    /// [`Condition::InstanceGone`]).
    pub fn judge_window(&self, w: &CondWindow) -> Option<Eval> {
        self.judge_window_with(w, &WindowExamples::default())
    }

    /// [`judge_window`](Self::judge_window), with the window's examples:
    /// the first failure and the first sample that could not be checked,
    /// which the evidence names.
    pub fn judge_window_with(&self, w: &CondWindow, ex: &WindowExamples) -> Option<Eval> {
        let synth = if w.synthetic > 0 {
            format!(
                "; {} from a mock owner (its descriptor's meta.synthetic)",
                w.synthetic
            )
        } else {
            String::new()
        };
        let rate = if w.window_s > 0.0 {
            w.samples as f64 / w.window_s
        } else {
            0.0
        };
        let unchecked = |what: &str| match (&ex.unchecked, w.samples) {
            (_, 0) => format!("no sample on the selector this window: nothing to {what}"),
            (Some(why), n) => format!("{n} sample(s), none could be {what}ed: {why}"),
            (None, n) => format!("{n} sample(s), none {what}ed this window"),
        };
        Some(match self {
            Condition::RateAbove { hz, .. } => {
                let state = CondState::from(judge_excess(rate > *hz, w.dropped));
                let evidence = match state {
                    CondState::Unobservable => format!(
                        "{rate:.2} Hz observed but {} sample(s) dropped — the true rate \
                         is at least that, not exactly that (O6){synth}",
                        w.dropped
                    ),
                    _ => format!(
                        "{} sample(s) in {:.1}s = {rate:.2} Hz against the {hz:.2} Hz \
                         bound{synth}",
                        w.samples, w.window_s
                    ),
                };
                Eval { state, evidence }
            }
            Condition::RateBelow { hz, .. } => {
                let state = CondState::from(judge_shortfall(rate < *hz, w.dropped));
                let evidence = match state {
                    CondState::Unobservable => format!(
                        "{rate:.2} Hz observed with {} sample(s) dropped — the drops \
                         could have carried the difference (O6){synth}",
                        w.dropped
                    ),
                    _ => format!(
                        "{} sample(s) in {:.1}s = {rate:.2} Hz against the {hz:.2} Hz \
                         bound{synth}",
                        w.samples, w.window_s
                    ),
                };
                Eval { state, evidence }
            }
            Condition::SilentFor { for_s, .. } => {
                let ev = SilenceEvidence {
                    sample_within: w.last_sample_ago_s.map(|ago| ago < *for_s) == Some(true),
                    span_observed: w.observed_s >= *for_s,
                    drop_free: w.last_drop_ago_s.map(|ago| ago >= *for_s) != Some(false),
                };
                let SilenceEvidence { span_observed, .. } = ev;
                let state = CondState::from(judge_silence(ev));
                let evidence = match state {
                    CondState::Ok => format!(
                        "a sample rode {:.1}s ago, inside the {for_s:.1}s span{synth}",
                        w.last_sample_ago_s.unwrap_or(0.0)
                    ),
                    CondState::Firing => {
                        format!("no sample for {for_s:.1}s, on a drop-free observer{synth}")
                    }
                    CondState::Unobservable if !span_observed => format!(
                        "watched only {:.1}s of a {for_s:.1}s silence claim — not asked \
                         is not answered (O4){synth}",
                        w.observed_s
                    ),
                    CondState::Unobservable => format!(
                        "no sample seen, but the observer dropped inside the {for_s:.1}s \
                         span — silence is unprovable (O6){synth}"
                    ),
                };
                Eval { state, evidence }
            }
            Condition::InvalidPayload { .. } => {
                if w.invalid > 0 {
                    Eval {
                        state: CondState::Firing,
                        evidence: format!(
                            "{} of {} checked sample(s) failed their declared type; first: {} \
                             ({} observed, {} dropped{synth})",
                            w.invalid,
                            w.checked,
                            ex.failure.as_deref().unwrap_or("—"),
                            w.samples,
                            w.dropped
                        ),
                    }
                } else if w.checked > 0 {
                    Eval {
                        state: CondState::Ok,
                        evidence: format!(
                            "{} checked sample(s), none failed its declared type ({} observed, \
                             {} dropped{synth})",
                            w.checked, w.samples, w.dropped
                        ),
                    }
                } else {
                    Eval {
                        state: CondState::Unobservable,
                        evidence: format!("{}{synth}", unchecked("check")),
                    }
                }
            }
            Condition::QosMismatch { .. } => {
                if w.qos_mismatched > 0 {
                    Eval {
                        state: CondState::Firing,
                        evidence: format!(
                            "{} of {} judged sample(s) did not ride their declared QoS; first: \
                             {} ({} observed, {} dropped{synth})",
                            w.qos_mismatched,
                            w.qos_judged,
                            ex.failure.as_deref().unwrap_or("—"),
                            w.samples,
                            w.dropped
                        ),
                    }
                } else if w.qos_judged > 0 {
                    Eval {
                        state: CondState::Ok,
                        evidence: format!(
                            "{} judged sample(s) rode their declared priority, congestion \
                             control and express ({} observed, {} dropped{synth})",
                            w.qos_judged, w.samples, w.dropped
                        ),
                    }
                } else {
                    Eval {
                        state: CondState::Unobservable,
                        evidence: format!("{}{synth}", unchecked("judg")),
                    }
                }
            }
            Condition::Dropped => Eval {
                state: if w.dropped > 0 {
                    CondState::Firing
                } else {
                    CondState::Ok
                },
                evidence: format!(
                    "the observer dropped {} sample(s) in {:.1}s (O6){synth}",
                    w.dropped, w.window_s
                ),
            },
            Condition::DoctorCheck { .. }
            | Condition::InstanceGone { .. }
            | Condition::AlertFiring { .. } => return None,
        })
    }

    /// [`judge_window_with`](Self::judge_window_with) for the variants that
    /// *have* a window — total, because [`judge`](Self::judge) has already
    /// routed the other two elsewhere.
    fn judge_window_total(&self, w: &CondWindow, ex: &WindowExamples) -> Eval {
        self.judge_window_with(w, ex).unwrap_or_else(|| Eval {
            // Unreachable through `judge`; if some future variant reaches it,
            // "I have no window for this" is the honest answer, not a panic
            // in a watchdog that is supposed to keep running.
            state: CondState::Unobservable,
            evidence: "this rule is not judged against a sample window".into(),
        })
    }

    /// Judge a doctor run. `None` unless this is [`Condition::DoctorCheck`].
    ///
    /// The check's own verdict, projected (#612, FJ6): `Established` is
    /// firing, `NotEstablished` is ok, and both Unestablished poles are
    /// unobservable — a check the run could not establish, or did not ask,
    /// has not said it is clean. A failed run is unobservable for every
    /// doctor condition, and so is a run over an empty scope (#510).
    pub fn judge_doctor(&self, outcome: Result<&DoctorReport, &str>) -> Option<Eval> {
        let Condition::DoctorCheck { check } = self else {
            return None;
        };
        Some(match outcome {
            Err(e) => Eval {
                state: CondState::Unobservable,
                evidence: format!("the doctor run failed: {e}"),
            },
            Ok(DoctorReport {
                unobservable: Some(why),
                ..
            }) => Eval {
                state: CondState::Unobservable,
                evidence: format!("the doctor run judged nothing: {why}"),
            },
            Ok(report) => match report.check(*check).map(|c| (&c.verdict, &c.findings)) {
                Some((Judgement::Established, findings)) => Eval {
                    state: CondState::Firing,
                    evidence: match findings.first() {
                        Some(first) => format!(
                            "{} finding(s); first: {} — {}",
                            findings.len(),
                            first.subject,
                            first.evidence
                        ),
                        None => format!("{check} established a finding"),
                    },
                },
                Some((Judgement::NotEstablished { reason }, _)) => Eval {
                    state: CondState::Ok,
                    evidence: reason.clone(),
                },
                Some((Judgement::Unobservable { reason }, _)) => Eval {
                    state: CondState::Unobservable,
                    evidence: reason.clone(),
                },
                Some((Judgement::NotAsked, _)) | None => Eval {
                    state: CondState::Unobservable,
                    evidence: format!("the doctor run did not ask {check}"),
                },
            },
        })
    }
}

impl std::fmt::Display for Condition {
    /// The canonical rule spelling — [`Condition::parse`] round-trips it,
    /// and it is the `rule` field of every [`Transition`].
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Condition::RateAbove { selector, hz } => write!(f, "rate-above {selector} {hz}"),
            Condition::RateBelow { selector, hz } => write!(f, "rate-below {selector} {hz}"),
            Condition::SilentFor { selector, for_s } => {
                write!(f, "silent-for {selector} {for_s}")
            }
            Condition::InvalidPayload { selector } => write!(f, "invalid-payload {selector}"),
            Condition::QosMismatch { selector } => write!(f, "qos-mismatch {selector}"),
            Condition::DoctorCheck { check } => write!(f, "doctor {check}"),
            Condition::InstanceGone { address } => write!(f, "instance-gone {address}"),
            Condition::AlertFiring { selector, at_least } => {
                write!(f, "alert-firing {selector} {at_least}")
            }
            Condition::Dropped => write!(f, "dropped"),
        }
    }
}

// ─── the judgement rules (the vocabulary's semantic core) ───────────────────
//
// The three judges return the four-pole [`Judgement`] core (observer §1).
// None of them ever answers `NotAsked` — a judge is
// only called when the question was put — but the pole exists in the currency
// so a caller that *skipped* a judge can say so in the same vocabulary. The
// watchdog projects each judgement onto [`CondState`] for the wire.

/// The shortfall rule ([`Condition::RateBelow`]; `expect`'s count floor and
/// rate floor): too little was seen. Enough seen is conclusively clean even
/// under drops — a drop can only hide *more*. A shortfall with drops is
/// unobservable: the dropped samples could have filled it (tooling guide O6).
pub fn judge_shortfall(short: bool, dropped: u64) -> Judgement {
    match (short, dropped) {
        (false, _) => Judgement::NotEstablished {
            reason: "enough was seen — a drop only hides more".into(),
        },
        (true, 0) => Judgement::Established,
        (true, _) => Judgement::Unobservable {
            reason: format!("{dropped} dropped sample(s) could have filled the shortfall (O6)"),
        },
    }
}

/// The excess rule ([`Condition::RateAbove`]; `expect`'s rate ceiling): too
/// much was seen. An excess is positive evidence, conclusive under drops.
/// "No excess" is a completeness claim — it counts what did NOT happen — so
/// under drops it is unobservable, never clean (O6).
pub fn judge_excess(over: bool, dropped: u64) -> Judgement {
    match (over, dropped) {
        (true, _) => Judgement::Established,
        (false, 0) => Judgement::NotEstablished {
            reason: "no excess was counted, on a clean observation".into(),
        },
        (false, _) => Judgement::Unobservable {
            reason: format!(
                "{dropped} sample(s) dropped — \"did not exceed\" is a completeness \
                 claim (O6)"
            ),
        },
    }
}

/// The silence rule ([`Condition::SilentFor`]; `expect --absent`): a sample
/// inside the span conclusively breaks the silence; silence is provable only
/// over a span the observer actually watched (O4) drop-free (O6) — otherwise
/// unobservable, never clean.
/// What one silence claim rests on — three facts that are all `bool` and all
/// about the same span.
///
/// A struct rather than three positional parameters, because this feeds a
/// *judgement* and a transposition of two identically-typed booleans returns
/// a plausible wrong verdict with no compile error (#349).
/// `judge_shortfall`/`judge_excess` keep their positional `(bool, u64)` —
/// not transposable, so not a hazard.
#[derive(Debug, Clone, Copy)]
pub struct SilenceEvidence {
    /// A sample rode inside the claimed span — the conclusive break.
    pub sample_within: bool,
    /// The observer actually watched the whole span (O4). A span it did not
    /// watch is not a span it can call silent.
    pub span_observed: bool,
    /// The observer dropped nothing inside the span (O6). "Nothing arrived"
    /// under drops is a completeness claim the observation cannot carry.
    pub drop_free: bool,
}

pub fn judge_silence(ev: SilenceEvidence) -> Judgement {
    let SilenceEvidence {
        sample_within,
        span_observed,
        drop_free,
    } = ev;
    if sample_within {
        Judgement::NotEstablished {
            reason: "a sample rode inside the span".into(),
        }
    } else if span_observed && drop_free {
        Judgement::Established
    } else if !span_observed {
        Judgement::Unobservable {
            reason: "the observer has not watched the whole claimed span (O4)".into(),
        }
    } else {
        Judgement::Unobservable {
            reason: "the observer dropped inside the span — silence is unprovable (O6)".into(),
        }
    }
}

/// Everything one watchdog tick observed, in the shapes the conditions are
/// judged against.
///
/// `doctor` and `presence` are `Option` because a tick only runs those
/// asks if some rule wants them — and "not run this tick" is
/// *unobservable*, which is the honest reading (#352).
pub struct TickEvidence<'e> {
    pub window: &'e CondWindow,
    pub examples: &'e WindowExamples,
    pub doctor: Option<Result<&'e DoctorReport, &'e str>>,
    /// The live presence projection the `instance-gone` rules read (#747);
    /// `None` when none is running.
    pub presence: Option<&'e LivePresence>,
    /// This rule's own per-address reading of it, for an `instance-gone`
    /// rule: what it has seen held and gone.
    pub addresses: Option<&'e AddressWatch>,
    /// The alarms an `alert-firing` rule's selector reaches (#800, AC): the
    /// reader's book, or why there is none; `None` when not read this tick.
    pub alarms: Option<std::result::Result<&'e AlarmBook, &'e str>>,
}

/// Judge one doctor check against this tick's run — total, and total in the
/// "did not run" direction too.
pub fn judge_doctor_check(check: CheckId, outcome: Option<Result<&DoctorReport, &str>>) -> Eval {
    let Some(outcome) = outcome else {
        return Eval {
            state: CondState::Unobservable,
            evidence: "the doctor did not run this tick".into(),
        };
    };
    Condition::DoctorCheck { check }
        .judge_doctor(outcome)
        .expect("a DoctorCheck is judged by the doctor")
}

/// One tick's doctor run for the doctor rules (#612, FJ6), or why there is
/// none: zk2's doctor reads the deployment through a session in its
/// namespace, and a runner given none cannot ask it.
pub(crate) async fn tick_doctor(
    bus: Option<&DoctorBus>,
    store: &BundleStore,
    spec: &crate::judge::doctor::DoctorSpec,
) -> std::result::Result<DoctorReport, String> {
    match bus {
        Some(bus) => Ok(crate::judge::doctor::run_doctor(bus, store, spec).await),
        None => Err("no session in the deployment's namespace was given to the doctor".into()),
    }
}

/// What a presence reader can conclude under `basis`, of the addresses
/// under `pattern` (#747): only a complete read with nothing lost since
/// stands behind a gone, so any other basis is the observer unobservable,
/// and says why. The watchdog's `instance-gone` fold and zenwatch's
/// `observer` identity read it alike.
pub fn presence_observer(basis: PresenceBasis, pattern: &str) -> Eval {
    let (state, evidence) = match basis {
        PresenceBasis::Complete => (
            CondState::Ok,
            format!("a complete presence read stands: every address under {pattern} is observable"),
        ),
        PresenceBasis::Lost { dropped } => (
            CondState::Unobservable,
            format!(
                "the presence projection lost {dropped} liveliness event(s): every address \
                 under {pattern} is unobservable until a complete re-read — a token gone or \
                 back inside that span was not seen, and nothing here says it is fine (tooling \
                 guide O6)"
            ),
        ),
        PresenceBasis::Incomplete { cut_off_during } => (
            CondState::Unobservable,
            format!(
                "the presence read {}, possibly incomplete (spec §8.1): an address under \
                 {pattern} it did not show is unobservable, not gone; re-reading",
                crate::model::catalog::incomplete_reason(
                    false,
                    cut_off_during,
                    "ended at its timeout"
                )
                .unwrap_or_default()
            ),
        ),
        PresenceBasis::Unseeded => (
            CondState::Unobservable,
            format!("presence under {pattern} has not been read yet: nothing is established"),
        ),
        PresenceBasis::CutOff => (
            CondState::Unobservable,
            format!(
                "this reader's session reaches no router and no peer, so no presence read is \
                 made: every address under {pattern} is unobservable until a complete read \
                 (core §8.1)"
            ),
        ),
        PresenceBasis::Moved => (
            CondState::Unobservable,
            format!(
                "the watch session's transport moved, and its subscribers may not have heard \
                 what moved meanwhile: every address under {pattern} is unobservable until a \
                 complete re-read after its round trip (core §8.1)"
            ),
        ),
    };
    Eval { state, evidence }
}

/// Judge one `instance-gone` rule — total — from its per-address reading
/// of the live presence projection (#747, [`AddressWatch`]), the reading
/// zenwatch notifies from address by address, folded into the one state a
/// watchdog rule takes: **firing** while any address it watches is gone,
/// the evidence naming each; else **unobservable** while the projection
/// cannot see ([`presence_observer`]), or is not running; else **ok**.
///
/// An address gone stays firing across a loss until a complete re-read
/// says otherwise, and an address whose going was hidden in the loss is
/// found by that re-read and fires then. Under a wildcard, a pattern that
/// never matched is ok, not firing: nothing can be gone that was never
/// seen held. An exact address is watched from the start.
pub fn judge_instance_gone(watch: Option<&AddressWatch>, presence: Option<&LivePresence>) -> Eval {
    let Some(watch) = watch else {
        return Eval {
            state: CondState::Unobservable,
            evidence: "this rule's per-address reading of presence was not given".into(),
        };
    };
    let pattern = &watch.target().address;
    let gone: Vec<String> = watch.gone().map(ToString::to_string).collect();
    if !gone.is_empty() {
        return Eval {
            state: CondState::Firing,
            evidence: format!(
                "gone: {} — no instance token visible to this reader, a complete read behind \
                 it (spec §8.1; a refused read is empty too){}",
                gone.join(", "),
                if watch.is_exact() {
                    String::new()
                } else {
                    format!(
                        "; {} other address(es) under {pattern} hold",
                        watch.held().count()
                    )
                }
            ),
        };
    }
    let Some(presence) = presence else {
        return Eval {
            state: CondState::Unobservable,
            evidence: "the live presence projection is not running: no session in the \
                       deployment's namespace, or its monitor stopped"
                .into(),
        };
    };
    let sight = presence_observer(presence.basis(), pattern);
    if sight.state != CondState::Ok {
        return sight;
    }
    let held = watch.held().count();
    Eval {
        state: CondState::Ok,
        evidence: match (watch.is_exact(), held) {
            (true, _) => {
                format!("{pattern} holds an instance token, a complete read behind it (spec §8.1)")
            }
            (false, 0) => format!(
                "no address matches {pattern} yet: nothing can be gone that was never seen \
                 held, so the first one to appear is the first one watched"
            ),
            (false, n) => format!(
                "{n} address(es) under {pattern} hold an instance token, and none seen held has \
                 gone (spec §8.1)"
            ),
        },
    }
}

/// Judge one `alert-firing` rule — total — from its selector's reader
/// (#800, AC): §5's second question of `alarms.v1` at `at_least`, over the
/// owners in scope (§2.15), by `zenkey_model::alarms::any_firing`. **Firing**
/// on yes, with the count and the first named; **ok** on no; every other
/// answer **unobservable**, with its reason and each owner counted by its
/// verdict — not asked is never ok, nor is a reader that is not reading.
pub fn judge_alert_firing(
    selector: &str,
    at_least: zenkey_model::alarms::Severity,
    read: Option<std::result::Result<&AlarmBook, &str>>,
) -> Eval {
    use zenkey_model::alarms::Answer;
    let book = match read {
        None => {
            return Eval {
                state: CondState::Unobservable,
                evidence: format!("the alarms under {selector} were not read this tick"),
            };
        }
        Some(Err(why)) => {
            return Eval {
                state: CondState::Unobservable,
                evidence: format!("the alarms under {selector} could not be read: {why}"),
            };
        }
        Some(Ok(b)) => b,
    };
    let (now, wall) = (std::time::Instant::now(), SystemTime::now());
    let a = book.any_firing(at_least, now, wall);
    let counts = format!(
        "owners in scope: {} established, {} unobservable, {} not asked{}",
        a.established,
        a.unobservable,
        a.not_asked,
        if book.lost() > 0 {
            format!("; {} delivery(ies) lost or stalls so far", book.lost())
        } else {
            String::new()
        }
    );
    match a.answer {
        Answer::Yes => {
            let first = book
                .keys()
                .filter_map(|k| Some((k, book.reading(k, now, wall)?)))
                .find(|(_, r)| {
                    r.firing().answer == Answer::Yes && r.severity.is_some_and(|s| s >= at_least)
                })
                .map(|(k, r)| format!("; first: {k} at {}", r.severity.map_or("?", |s| s.as_str())))
                .unwrap_or_default();
            Eval {
                state: CondState::Firing,
                evidence: format!(
                    "{} alarm(s) firing at or above {at_least} under {selector}{first} ({counts}; alarms.v1 §2.15)",
                    a.firing
                ),
            }
        }
        Answer::No => Eval {
            state: CondState::Ok,
            evidence: format!(
                "no alarm firing at or above {at_least} under {selector}: every owner asked is known whole ({counts}; alarms.v1 §2.15)"
            ),
        },
        Answer::Unobservable => Eval {
            state: CondState::Unobservable,
            evidence: format!(
                "whether an alarm fires at or above {at_least} under {selector} is unobservable: {} ({}; {counts}) — never ok",
                a.reason.says(),
                a.reason.as_str()
            ),
        },
        Answer::NotAsked => Eval {
            state: CondState::Unobservable,
            evidence: format!(
                "no owner under {selector} was asked: {} ({}; {counts}) — absent, not listing \
                 alarms.v1, or not seen; not asked is never ok",
                a.reason.says(),
                a.reason.as_str()
            ),
        },
    }
}

/// The `alert-firing` rules' readers (#800, AC): one live [`AlarmFeed`] per
/// distinct selector, opened in the deployment's namespace before the first
/// window — subscribed, then seeded by S4's GET (core §4.2) — drained
/// beside the monitor, and re-read (presence, the GET) in each tick's
/// sweep, beside the drain, never inside it.
pub(crate) struct RuleAlarms {
    feeds: Vec<(String, std::result::Result<AlarmFeed, String>)>,
}

impl RuleAlarms {
    /// No reader: no rule reads alarms.
    pub(crate) fn none() -> RuleAlarms {
        RuleAlarms { feeds: Vec::new() }
    }

    /// One reader per selector `rules` read, reading on `session` and
    /// holding its instance tokens' subscription on `watch` (#830); with no
    /// session, each is unobservable, saying why.
    pub(crate) async fn open(
        session: Option<&zenoh::Session>,
        watch: Option<&crate::WatchSession>,
        rules: &RuleSet,
        timeout: Duration,
        clocks_synced: bool,
    ) -> RuleAlarms {
        let mut feeds = Vec::new();
        for selector in rules.alarm_selectors() {
            let feed = match (session, AlarmScope::parse(&selector)) {
                (None, _) => {
                    Err("no session in the deployment's namespace was given to read alarms".into())
                }
                (_, Err(why)) => Err(why),
                (Some(s), Ok(scope)) => {
                    let mut spec = AlarmSpec::new(timeout);
                    spec.clocks_synced = clocks_synced;
                    spec.archives = false;
                    AlarmFeed::open(s, watch, scope, spec, true)
                        .await
                        .map(|(f, _)| f)
                        .map_err(|e| crate::one_line(&e))
                }
            };
            feeds.push((selector, feed));
        }
        RuleAlarms { feeds }
    }

    /// Some reader listens: the drain owes it its deliveries.
    pub(crate) fn listening(&self) -> bool {
        self.feeds.iter().any(|(_, f)| f.is_ok())
    }

    /// The next delivery of any reader, folded into its book. A reader
    /// whose subscriptions closed reads unobservable from then on.
    pub(crate) async fn next(&mut self) {
        let mut ok: Vec<(usize, &mut AlarmFeed)> = self
            .feeds
            .iter_mut()
            .enumerate()
            .filter_map(|(i, (_, f))| f.as_mut().ok().map(|f| (i, f)))
            .collect();
        if ok.is_empty() {
            return std::future::pending().await;
        }
        let (closed, index) = {
            let nexts = ok
                .iter_mut()
                .map(|(i, f)| {
                    let i = *i;
                    Box::pin(async move { (i, f.next().await.is_none()) })
                })
                .collect::<Vec<_>>();
            let ((i, closed), _, _) = futures_util::future::select_all(nexts).await;
            (closed, i)
        };
        if closed {
            self.feeds[index].1 = Err("its subscriptions closed".into());
        }
    }

    /// Each reader's refresh, owned, to run beside the drain.
    pub(crate) fn plans(&self) -> Vec<(usize, AlarmRefresh)> {
        self.feeds
            .iter()
            .enumerate()
            .filter_map(|(i, (_, f))| f.as_ref().ok().map(|f| (i, f.refresh_plan())))
            .collect()
    }

    /// What the refreshes read, folded into their books.
    pub(crate) fn fold(&mut self, done: Vec<(usize, Refreshed)>) {
        for (i, r) in done {
            if let Some((_, Ok(f))) = self.feeds.get_mut(i) {
                f.fold(r);
            }
        }
    }

    /// The books, as a tick hands them to the rules.
    pub(crate) fn reads(&self) -> Vec<AlarmRead<'_>> {
        self.feeds
            .iter()
            .map(|(selector, f)| AlarmRead {
                selector,
                book: f.as_ref().map(AlarmFeed::book).map_err(String::as_str),
            })
            .collect()
    }
}

/// Every refresh in `plans`, together.
pub(crate) async fn refresh_alarms(plans: Vec<(usize, AlarmRefresh)>) -> Vec<(usize, Refreshed)> {
    futures_util::future::join_all(
        plans
            .into_iter()
            .map(|(i, p)| async move { (i, p.run().await) }),
    )
    .await
}

/// The `instance-gone` rules' presence (#747): the live projection over the
/// scope their patterns cover ([`RuleSet::presence_scope`]), kept current
/// by its own monitor on a watch session — in no namespace on the wire keys
/// (#875); a flood of samples on the raw session must not make presence
/// unobservable, and no presence read is made beside a liveliness
/// subscriber (#830, core §8.1, 0.37) — seeded through the session in the
/// namespace, and looked at once ([`RuleSet::first_look`]) before the
/// first window.
pub(crate) struct RulePresence {
    monitor: crate::Monitor,
    pub(crate) feed: PresenceFeed,
    /// Whether the feed still runs: its monitor's broadcast closing ends it,
    /// and the rules read unobservable from then on.
    pub(crate) running: bool,
}

impl RulePresence {
    /// Opens it for `rules` when one of them reads presence: its monitor on
    /// `watch`, its seeds through `session` (#830). With no watch session,
    /// or `session` itself as one, it is refused before anything is
    /// declared: [`crate::Error::NotAsked`].
    pub(crate) async fn open(
        session: &zenoh::Session,
        watch: Option<&crate::WatchSession>,
        rules: &mut RuleSet,
        timeout: Duration,
    ) -> Result<Option<RulePresence>> {
        if !rules.wants_presence() {
            return Ok(None);
        }
        let Some(watch) = watch.filter(|w| w.zid() != session.zid()) else {
            return Err(crate::Error::not_asked_for(
                "instance-gone's presence",
                crate::error::NotAsked::BesideSubscriber {
                    zid: session.zid().to_string(),
                },
            ));
        };
        let scope = rules.presence_scope();
        let spec = crate::MonitorSpec {
            // A stats tick would only wake the feed, which folds tokens.
            stats_tick: Duration::from_secs(3600),
            ..crate::MonitorSpec::default()
        }
        .with_presence(&scope);
        let monitor = crate::Monitor::start_watch(watch, spec).await?;
        let mut live = LivePresence::new(scope);
        rules.track_presence(&mut live, SystemTime::now());
        let (feed, _seeded) = match PresenceFeed::open(session, &monitor, live, timeout).await {
            Ok(opened) => opened,
            Err(e) => {
                if let Err(teardown) = monitor.shutdown().await {
                    tracing::warn!("after a refused presence feed: {teardown}");
                }
                return Err(e);
            }
        };
        rules.first_look(feed.presence());
        Ok(Some(RulePresence {
            monitor,
            feed,
            running: true,
        }))
    }

    /// The projection, while the feed runs.
    pub(crate) fn presence(&self) -> Option<&LivePresence> {
        self.running.then(|| self.feed.presence())
    }

    /// Its monitor's acknowledged teardown (#207/#336).
    pub(crate) async fn shutdown(self) -> Result<()> {
        self.monitor.shutdown().await
    }
}

// ─── observations and evaluations ───────────────────────────────────────────

/// What one evaluation window observed on one condition's selector — the
/// facts, separated from the judgement so the judgement is pure.
#[derive(Debug, Clone, Copy, Default)]
pub struct CondWindow {
    /// The span this window judges, seconds.
    pub window_s: f64,
    /// How long the observer has been watching in total — a claim about a
    /// span longer than this is unobservable (O4).
    pub observed_s: f64,
    /// Samples matching the selector within the window.
    pub samples: u64,
    /// Stream drops within the window — unattributable to any one selector,
    /// so they taint every completeness claim (O6).
    pub dropped: u64,
    /// Seconds since the last matching sample; `None` = none seen since the
    /// watch began.
    pub last_sample_ago_s: Option<f64>,
    /// Seconds since the last stream drop; `None` = the stream never dropped.
    pub last_drop_ago_s: Option<f64>,
    /// Samples whose payload failed its declared type, among those checked.
    pub invalid: u64,
    /// Samples actually checked against their declared type (a budget
    /// bounds the cost, and an unresolved key cannot be).
    pub checked: u64,
    /// Samples that did not ride their declared QoS, among those judged.
    pub qos_mismatched: u64,
    /// Samples with a declared QoS to judge against.
    pub qos_judged: u64,
    /// Samples from an address whose descriptor carries a mock owner's
    /// synthetic marker — generated traffic judged as real would be a
    /// self-inflicted page, so every evidence line carries the count.
    pub synthetic: u64,
}

/// What one window's evidence names beside its counts: the first failure,
/// and why a sample could not be checked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowExamples {
    /// The first sample that failed — its key and what failed.
    pub failure: Option<String>,
    /// Why the first sample that could not be checked was not.
    pub unchecked: Option<String>,
}

/// One evaluation: the three-valued state, and the evidence for it.
#[derive(Debug, Clone, PartialEq)]
pub struct Eval {
    pub state: CondState,
    pub evidence: String,
}

/// One rule's transition detector: feed evaluations in, get a [`Transition`]
/// back **only** when the state genuinely changed. An unchanged tick returns
/// `None` — transitions, not states.
#[derive(Debug, Clone)]
pub struct RuleState {
    /// The condition itself, not its `Display` (#352).
    rule: Condition,
    state: Option<CondState>,
}

impl RuleState {
    pub fn new(rule: Condition) -> RuleState {
        RuleState { rule, state: None }
    }

    /// The condition this state tracks.
    pub fn rule(&self) -> &Condition {
        &self.rule
    }

    /// The last observed state; `None` until the first evaluation.
    pub fn state(&self) -> Option<CondState> {
        self.state
    }

    /// Feed one evaluation. The first ever emits (from `null` — the baseline
    /// is said once); after that only a genuine change does.
    pub fn observe(&mut self, eval: Eval, at: impl Into<String>) -> Option<Transition> {
        if self.state == Some(eval.state) {
            return None;
        }
        let from = self.state;
        self.state = Some(eval.state);
        Some(Transition {
            rule: self.rule.to_string(),
            from,
            to: eval.state,
            at: at.into(),
            evidence: eval.evidence,
        })
    }
}

/// Run-over-run delta over a doctor report: one [`RuleState`] per stable
/// check id ([`CheckId`]), fed by `doctor --transitions`. The
/// first run states the baseline (one transition per check id); every later run yields
/// only genuine changes. A failed run flips every check to `unobservable` —
/// a doctor that could not run has not said the deployment is healthy.
#[derive(Debug, Clone)]
pub struct DoctorWatch {
    checks: Vec<RuleState>,
}

impl DoctorWatch {
    /// One state per check id there is.
    pub fn new() -> DoctorWatch {
        DoctorWatch::of(CheckId::ALL)
    }

    /// One state per check in `checks`: the ones a run asks.
    pub fn of(checks: impl IntoIterator<Item = CheckId>) -> DoctorWatch {
        DoctorWatch {
            checks: checks
                .into_iter()
                .map(|check| RuleState::new(Condition::DoctorCheck { check }))
                .collect(),
        }
    }

    /// Feed one doctor run (or its failure) and collect the transitions.
    pub fn observe(&mut self, outcome: Result<&DoctorReport, &str>, at: &str) -> Vec<Transition> {
        self.checks
            .iter_mut()
            .filter_map(|state| {
                let Condition::DoctorCheck { check } = *state.rule() else {
                    // Unconstructible: `new` builds only `DoctorCheck`s.
                    return None;
                };
                let eval = judge_doctor_check(check, Some(outcome));
                state.observe(eval, at)
            })
            .collect()
    }
}

impl Default for DoctorWatch {
    fn default() -> Self {
        DoctorWatch::new()
    }
}

// ─── the watchdog runner ────────────────────────────────────────────────────

/// What a watchdog run watches, and for how long.
#[derive(Debug, Clone)]
pub struct WatchdogSpec {
    /// The rules, evaluated every tick.
    pub rules: Vec<Condition>,
    /// Evaluation cadence. A tick that runs long (a doctor rule's fan-in)
    /// slides rather than backlogs; windows are measured, not nominal.
    pub tick: Duration,
    /// Stop after this many ticks; `None` = run until the caller stops it.
    pub ticks: Option<u64>,
    /// Per-ask timeout for the presence, contract and doctor asks.
    pub timeout: Duration,
    /// The operator's word that this host's clock and the owners' agree
    /// within the HLC delta, handed to a `doctor` rule's doctor as
    /// [`crate::judge::doctor::DoctorSpec::clocks_synced`]: the doctor
    /// listens to no status, so without it a `doctor health-*` rule reads a
    /// `health.v1` status's age as unobservable (#735, a PF follow-up).
    pub clocks_synced: bool,
}

/// How many payload checks each key gets per tick under an
/// `invalid-payload` rule: a watchdog must not become a load test.
const DECODE_BUDGET: u8 = 2;

/// What one tick counted on one rule's selector.
#[derive(Default, Clone)]
struct TickCounters {
    samples: u64,
    invalid: u64,
    checked: u64,
    qos_mismatched: u64,
    qos_judged: u64,
    synthetic: u64,
    examples: WindowExamples,
}

/// One rule's whole per-run state, together (#352).
struct RuleRuntime {
    rule: Condition,
    /// The rule's selector, compiled once for sample attribution.
    keyexpr: Option<zenoh::key_expr::KeyExpr<'static>>,
    counters: TickCounters,
    last_sample: Option<tokio::time::Instant>,
    /// An `instance-gone` rule's per-address reading of presence (#747).
    addresses: Option<AddressWatch>,
    state: RuleState,
}

/// What a tick judges beside its drain, as the rules see it: the doctor
/// run, the live presence projection the `instance-gone` rules read, and
/// the alarms the `alert-firing` rules read, each `None` (or empty) when no
/// rule wanted it or none could run — which is *unobservable* for the rules
/// that would have needed it (#352).
#[derive(Debug, Clone, Copy, Default)]
pub struct SweepOutcome<'e> {
    pub doctor: Option<Result<&'e DoctorReport, &'e str>>,
    pub presence: Option<&'e LivePresence>,
    /// One reader per `alert-firing` selector (#800, AC).
    pub alarms: &'e [AlarmRead<'e>],
}

/// The alarms one `alert-firing` selector reaches, as a tick hands them to
/// the rules: the reader's book, or why there is none.
#[derive(Debug, Clone, Copy)]
pub struct AlarmRead<'e> {
    pub selector: &'e str,
    pub book: std::result::Result<&'e AlarmBook, &'e str>,
}

/// A set of rules judged tick by tick over **one** event stream — the
/// watchdog's per-tick body, lifted out of [`watchdog`] so a second driver
/// can run it (#218).
///
/// The driver owns the stream, the drain loop, the sweep, the lens and the
/// presence projection; this owns everything the rules know: per-rule
/// counters, the last sample and drop instants, the per-tick check budget,
/// each `instance-gone` rule's per-address reading, and the transition
/// detectors. Feed it every sample
/// ([`observe_sample`](Self::observe_sample)) through the lens the driver
/// holds, every drop ([`observe_drop`](Self::observe_drop)), every batch of
/// presence moves ([`observe_presence`](Self::observe_presence)), then
/// [`evaluate`](Self::evaluate) once per tick and get back only what
/// changed.
///
/// **Why the seam exists.** A trigger capture (`zenctl record --on`) must
/// judge *the same event stream it records*: one subscription, one drop
/// ledger. With the body a value, the recorder drains one stream and hands
/// every item to both the ring and the rules.
///
/// Sample attribution is by key-expression intersection against each rule's
/// selector. Time is `tokio::time::Instant`, so a driver under paused time
/// judges exact windows.
pub struct RuleSet {
    rules: Vec<RuleRuntime>,
    /// The distinct selectors the rules observe, in first-seen order.
    watched: Vec<String>,
    started: tokio::time::Instant,
    last_eval: tokio::time::Instant,
    last_drop: Option<tokio::time::Instant>,
    dropped_tick: u64,
    budget: BTreeMap<String, u8>,
    ticks: u64,
    transitions: u64,
}

/// What one sample said to the rules that judge it — worked out once, the
/// first time a rule needs it.
#[derive(Default)]
struct Verdicts {
    checked: Option<Option<(Conformance, String)>>,
    qos: Option<std::result::Result<Option<String>, String>>,
}

impl RuleSet {
    /// Compile the rules. Fails on a selector that is not a key expression —
    /// before anything is declared, so the `?` has nothing to tear down
    /// (#336). The watch clock starts here: [`CondWindow::observed_s`] is
    /// measured from construction, so build the set right before the
    /// subscriptions are declared.
    pub fn new(rules: &[Condition]) -> Result<Self> {
        let compiled = rules
            .iter()
            .map(|rule| {
                Ok(RuleRuntime {
                    rule: rule.clone(),
                    keyexpr: rule
                        .selector()
                        .map(|sel| {
                            zenoh::key_expr::KeyExpr::try_from(sel.to_string())
                                .map_err(|e| Error::unaskable_from(format!("{sel:?}"), e))
                        })
                        .transpose()?,
                    counters: TickCounters::default(),
                    last_sample: None,
                    addresses: match rule {
                        Condition::InstanceGone { address } => {
                            Some(AddressWatch::new(Target::parse(address)?))
                        }
                        _ => None,
                    },
                    state: RuleState::new(rule.clone()),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut watched: Vec<String> = Vec::new();
        for rule in rules {
            if let Some(sel) = rule.selector()
                && !watched.iter().any(|s| s == sel)
            {
                watched.push(sel.to_string());
            }
        }
        let now = tokio::time::Instant::now();
        Ok(RuleSet {
            rules: compiled,
            watched,
            started: now,
            last_eval: now,
            last_drop: None,
            dropped_tick: 0,
            budget: BTreeMap::new(),
            ticks: 0,
            transitions: 0,
        })
    }

    /// The distinct selectors the rules observe — what the driver must
    /// subscribe to before the first window opens (O4).
    pub fn watched(&self) -> &[String] {
        &self.watched
    }

    /// Some rule judges a doctor run, so the driver owes one per tick.
    pub fn wants_doctor(&self) -> bool {
        self.rules
            .iter()
            .any(|r| matches!(r.rule, Condition::DoctorCheck { .. }))
    }

    /// The doctor a tick runs for the rules (#612, FJ6): only the checks
    /// they name, and `deep` when one names a check that reads the owners'
    /// data plane (`state-stamp-foreign`) — a rule asked for it — with
    /// `clocks_synced` as the operator gave it (#735) and every other
    /// setting the doctor's default.
    pub fn doctor_spec(
        &self,
        timeout: Duration,
        clocks_synced: bool,
    ) -> crate::judge::doctor::DoctorSpec {
        let checks: std::collections::BTreeSet<CheckId> = self
            .rules
            .iter()
            .filter_map(|r| match r.rule {
                Condition::DoctorCheck { check } => Some(check),
                _ => None,
            })
            .collect();
        let mut spec = crate::judge::doctor::DoctorSpec::new(timeout);
        spec.deep = checks.iter().any(|c| c.reads_data());
        spec.checks = checks;
        spec.clocks_synced = clocks_synced;
        spec
    }

    /// Some rule reads the live presence projection (`instance-gone`), so
    /// the driver owes one: tracked ([`track_presence`](Self::track_presence)),
    /// seeded and looked at once ([`first_look`](Self::first_look)) before
    /// the first window, then each batch of its moves
    /// ([`observe_presence`](Self::observe_presence)).
    pub fn wants_presence(&self) -> bool {
        self.rules.iter().any(|r| r.addresses.is_some())
    }

    /// Some rule reads `alarms.v1` (`alert-firing`, #800 AC), so the driver
    /// owes one live reader per distinct selector ([`RuleSet::alarm_selectors`]),
    /// opened before the first window and re-read beside the drain.
    pub fn wants_alarms(&self) -> bool {
        self.rules
            .iter()
            .any(|r| matches!(r.rule, Condition::AlertFiring { .. }))
    }

    /// The distinct selectors the `alert-firing` rules read, in rule order.
    pub fn alarm_selectors(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for r in &self.rules {
            if let Condition::AlertFiring { selector, .. } = &r.rule
                && !out.contains(selector)
            {
                out.push(selector.clone());
            }
        }
        out
    }

    /// The narrowest presence scope covering every `instance-gone` pattern:
    /// what the projection is read over.
    pub fn presence_scope(&self) -> Scope {
        covering_scope(
            self.rules
                .iter()
                .filter_map(|r| r.addresses.as_ref().map(AddressWatch::target)),
        )
    }

    /// Track every `instance-gone` pattern on `presence`, before it is
    /// seeded: an exact address is then watched from the start.
    pub fn track_presence(&self, presence: &mut LivePresence, at: SystemTime) {
        for watch in self.rules.iter().filter_map(|r| r.addresses.as_ref()) {
            watch.track(presence, at);
        }
    }

    /// The first look at the seeded projection, for every `instance-gone`
    /// rule ([`AddressWatch::first_look`]).
    pub fn first_look(&mut self, presence: &LivePresence) {
        for watch in self.rules.iter_mut().filter_map(|r| r.addresses.as_mut()) {
            watch.first_look(presence);
        }
    }

    /// Fold one batch of the projection's moves into every `instance-gone`
    /// rule's reading, as they arrive: the tick judges where each stands.
    pub fn observe_presence(&mut self, moved: &[PresenceTransition]) {
        for watch in self.rules.iter_mut().filter_map(|r| r.addresses.as_mut()) {
            watch.fold(moved);
        }
    }

    /// Some rule reads what a key *is* — its contract, its declared QoS —
    /// so the driver owes a lens: a presence read and the revisions it
    /// names, refreshed beside the drain.
    pub fn wants_lens(&self) -> bool {
        self.rules.iter().any(|r| {
            matches!(
                r.rule,
                Condition::InvalidPayload { .. } | Condition::QosMismatch { .. }
            )
        })
    }

    /// Count one observed sample against every rule its key matches,
    /// judging it through `lens` for the rules that need to: its payload
    /// against its declared type (budgeted per key per tick), its QoS
    /// against its resource's.
    pub fn observe_sample(&mut self, s: &SampleView, lens: &Lens<'_>) {
        let Ok(key) = zenoh::key_expr::KeyExpr::try_from(s.key.as_str()) else {
            return;
        };
        let now = tokio::time::Instant::now();
        let mut verdicts = Verdicts::default();
        let mut synthetic: Option<bool> = None;
        let mut budget = None;
        for rt in self.rules.iter_mut() {
            let Some(sel) = &rt.keyexpr else { continue };
            if !sel.intersects(&key) {
                continue;
            }
            rt.counters.samples += 1;
            let synth = *synthetic.get_or_insert_with(|| {
                lens.identity(&s.key)
                    .address()
                    .is_some_and(|a| lens.is_synthetic(a))
            });
            if synth {
                rt.counters.synthetic += 1;
            }
            rt.last_sample = Some(now);
            match &rt.rule {
                Condition::InvalidPayload { .. } => {
                    let allowed = *budget.get_or_insert_with(|| {
                        let b = self.budget.entry(s.key.clone()).or_default();
                        if *b < DECODE_BUDGET {
                            *b += 1;
                            true
                        } else {
                            false
                        }
                    });
                    if !allowed {
                        continue;
                    }
                    let checked = verdicts.checked.get_or_insert_with(|| check(s, lens));
                    match checked {
                        Some((c, line)) if c.is_checked() => {
                            rt.counters.checked += 1;
                            if c.is_violation() {
                                rt.counters.invalid += 1;
                                rt.counters
                                    .examples
                                    .failure
                                    .get_or_insert_with(|| line.clone());
                            }
                        }
                        Some((_, why)) => {
                            rt.counters
                                .examples
                                .unchecked
                                .get_or_insert_with(|| why.clone());
                        }
                        None => {
                            rt.counters
                                .examples
                                .unchecked
                                .get_or_insert_with(|| "a deletion carries no value".into());
                        }
                    }
                }
                Condition::QosMismatch { .. } => {
                    let qos = verdicts.qos.get_or_insert_with(|| qos(s, lens));
                    match qos {
                        Ok(mismatch) => {
                            rt.counters.qos_judged += 1;
                            if let Some(line) = mismatch {
                                rt.counters.qos_mismatched += 1;
                                rt.counters
                                    .examples
                                    .failure
                                    .get_or_insert_with(|| line.clone());
                            }
                        }
                        Err(why) => {
                            rt.counters
                                .examples
                                .unchecked
                                .get_or_insert_with(|| why.clone());
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// The stream dropped `n` samples here (O6): unattributable to any one
    /// selector, so it taints every completeness claim this tick.
    pub fn observe_drop(&mut self, n: u64) {
        self.dropped_tick += n;
        self.last_drop = Some(tokio::time::Instant::now());
    }

    /// Close the tick: judge every rule over the window measured since the
    /// last evaluation, reset the per-tick counts, and hand back only the
    /// genuine changes — none for an unchanged rule. `at` is the wall-clock
    /// stamp the transitions carry.
    pub fn evaluate(
        &mut self,
        now: tokio::time::Instant,
        at: &str,
        sweep: SweepOutcome<'_>,
    ) -> Vec<Transition> {
        let mut out = Vec::new();
        for rt in self.rules.iter_mut() {
            let window = CondWindow {
                window_s: (now - self.last_eval).as_secs_f64(),
                observed_s: (now - self.started).as_secs_f64(),
                samples: rt.counters.samples,
                dropped: self.dropped_tick,
                last_sample_ago_s: rt.last_sample.map(|t| (now - t).as_secs_f64()),
                last_drop_ago_s: self.last_drop.map(|t| (now - t).as_secs_f64()),
                invalid: rt.counters.invalid,
                checked: rt.counters.checked,
                qos_mismatched: rt.counters.qos_mismatched,
                qos_judged: rt.counters.qos_judged,
                synthetic: rt.counters.synthetic,
            };
            let alarms = match &rt.rule {
                Condition::AlertFiring { selector, .. } => sweep
                    .alarms
                    .iter()
                    .find(|a| a.selector == selector)
                    .map(|a| a.book),
                _ => None,
            };
            let eval = rt.rule.judge(&TickEvidence {
                window: &window,
                examples: &rt.counters.examples,
                doctor: sweep.doctor,
                presence: sweep.presence,
                addresses: rt.addresses.as_ref(),
                alarms,
            });
            if let Some(transition) = rt.state.observe(eval, at) {
                out.push(transition);
            }
        }
        for rt in self.rules.iter_mut() {
            rt.counters = TickCounters::default();
        }
        self.dropped_tick = 0;
        self.budget.clear();
        self.ticks += 1;
        self.transitions += out.len() as u64;
        self.last_eval = now;
        out
    }

    /// Ticks evaluated so far.
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    /// When the last tick closed (construction, before the first).
    pub fn last_eval(&self) -> tokio::time::Instant {
        self.last_eval
    }

    /// Transitions emitted so far.
    pub fn transitions(&self) -> u64 {
        self.transitions
    }

    /// Where each rule stands now, split the way [`WatchdogSummary`]
    /// carries it (#511): the rules last judged `firing`, then those last
    /// judged `unobservable` or never judged — canonical spellings, in rule
    /// order. A rule last judged `ok` is in neither.
    pub fn standing(&self) -> (Vec<String>, Vec<String>) {
        let (mut firing, mut unobservable) = (Vec::new(), Vec::new());
        for rt in &self.rules {
            match rt.state.state() {
                Some(CondState::Ok) => {}
                Some(CondState::Firing) => firing.push(rt.rule.to_string()),
                Some(CondState::Unobservable) | None => unobservable.push(rt.rule.to_string()),
            }
        }
        (firing, unobservable)
    }
}

/// One sample's payload against its declared type, through the lens: the
/// conformance, and one line naming the key and what failed — or why it
/// could not be checked. `None` for a deletion, which carries no value.
fn check(s: &SampleView, lens: &Lens<'_>) -> Option<(Conformance, String)> {
    if s.kind == zenoh::sample::SampleKind::Delete {
        return None;
    }
    let encoding = (!s.encoding.is_empty()).then_some(s.encoding.as_str());
    let attachment = s.attachment.as_ref().map(|a| a.to_bytes());
    let c = lens.check_sample(
        &s.key,
        Member::Type,
        encoding,
        &s.payload.to_bytes(),
        attachment.as_deref(),
    );
    let line = match &c.conformance {
        Conformance::Invalid { violations } => {
            format!("{}: {}", s.key, violations.join("; "))
        }
        Conformance::Undecodable { declared, reason } => {
            format!("{}: does not decode as {declared}: {reason}", s.key)
        }
        Conformance::NotChecked { reason } => reason.clone(),
        Conformance::Valid => String::new(),
    };
    Some((c.conformance, line))
}

/// One sample's QoS against its resource's (§2.4): `Ok(None)` when it rode
/// what was declared, `Ok(Some(line))` naming the axes that differ, `Err`
/// with why there was nothing to judge against.
fn qos(s: &SampleView, lens: &Lens<'_>) -> std::result::Result<Option<String>, String> {
    let declared = lens.declared_qos(&s.key).map_err(|why| why.words())?;
    let observed = observed_qos(s.priority, s.congestion_control, s.express);
    Ok(qos_mismatch(&declared, &observed).map(|m| format!("{}: {}", s.key, m.summary())))
}

/// Watch the rules and yield one [`Transition`] per genuine change, none per
/// unchanged tick. The subscriber set is declared before the first window
/// opens (O4); every selector rule is judged per tick over the measured
/// window, doctor rules by one run per tick, and `instance-gone` rules at
/// each tick from where their per-address reading of the live presence
/// projection stands (#747), its moves folded as they arrive.
///
/// The selectors are wire keys, watched on `bus.raw` (no namespace); the
/// lens, the presence projection (its own monitor, seeded before the first
/// window) and the doctor read through `bus.session`, in the namespace
/// (#612, FJ8b). Revisions in `offline` are never retrieved;
/// every other revision a descriptor names is retrieved into `store`.
///
/// A [`Straw`] rather than a `Stream` (#397), because a watchdog run is a
/// sequence **and** a final value: transitions while it runs, a
/// [`WatchdogSummary`] when it stops, and the acknowledged monitor teardown
/// (#207/#336) in between.
///
/// ```ignore
/// let mut run = watchdog(&bus, &store, &contracts, &spec).pin();
/// while let Some(transition) = run.sip().await {
///     writeln!(out, "{}", serde_json::to_string(&transition)?)?;
/// }
/// let summary = run.await?;
/// ```
pub fn watchdog<'a>(
    bus: &'a DoctorBus,
    store: &'a BundleStore,
    offline: &'a ContractSet,
    spec: &'a WatchdogSpec,
) -> impl Straw<WatchdogSummary, Transition, Error> + 'a {
    sipper(async move |mut sender: sipper::Sender<Transition>| {
        use crate::{FleetEvent, StreamItem};

        // Compiled *before* the monitor exists, so the `?` has nothing to tear
        // down (#336).
        let mut rules = RuleSet::new(&spec.rules)?;
        let (wants_doctor, wants_lens) = (rules.wants_doctor(), rules.wants_lens());
        let doctor_spec = rules.doctor_spec(spec.timeout, spec.clocks_synced);

        // The lens before the first window (#337's rule, kept): a check
        // inside the drain loop must never become a presence read or a
        // retrieval, because nothing attends the broadcast while one is in
        // flight. Each tick's sweep re-reads it beside the drain.
        let mut catalog: Option<Catalog> = if wants_lens {
            crate::bus::lens::read(&bus.session, store, spec.timeout)
                .await
                .ok()
        } else {
            None
        };

        // Presence before the first window too (#747): the `instance-gone`
        // rules read the live projection per address, seeded here and fed
        // beside the drain.
        let mut presence =
            RulePresence::open(&bus.session, bus.watch.as_ref(), &mut rules, spec.timeout).await?;

        // The `alert-firing` rules' readers too (#800, AC): subscribed and
        // seeded before the first window, drained beside it.
        let mut alarms = if rules.wants_alarms() {
            RuleAlarms::open(
                Some(&bus.session),
                bus.watch.as_ref(),
                &rules,
                spec.timeout,
                spec.clocks_synced,
            )
            .await
        } else {
            RuleAlarms::none()
        };

        // Declared before the window opens — not-asked must never read as "no".
        let monitor = crate::Monitor::start(&bus.raw, crate::MonitorSpec::default()).await?;
        let mut events = monitor.events();
        let monitor = monitor.watching(rules.watched()).await?;

        let mut closed = false;
        loop {
            let deadline = rules.last_eval() + spec.tick;
            // The tick's bus work runs **beside** the drain, not after it
            // (#338): a sweep that outlives the tick period widens this
            // window — `window_s` is measured, never assumed — so drops land
            // in the tick that incurred them.
            let plans = alarms.plans();
            let sweep = async {
                let doctor = if wants_doctor {
                    Some(tick_doctor(Some(bus), store, &doctor_spec).await)
                } else {
                    None
                };
                let lens = if wants_lens {
                    crate::bus::lens::read(&bus.session, store, spec.timeout)
                        .await
                        .ok()
                } else {
                    None
                };
                (doctor, lens, refresh_alarms(plans).await)
            };
            let mut sweep = std::pin::pin!(sweep);
            let mut swept = None;
            // One timer per tick, not one per drained sample (#346).
            let tick_over = tokio::time::sleep_until(deadline);
            tokio::pin!(tick_over);
            while !closed {
                let running = presence.as_ref().is_some_and(|p| p.running);
                let listening = alarms.listening();
                let item = tokio::select! {
                    item = events.recv() => item,
                    // An alarm reader's delivery folds into its book as it
                    // arrives (#800, AC).
                    () = alarms.next(), if listening => continue,
                    // Presence moves fold as they arrive, so each rule's
                    // per-address reading sees every one in order.
                    moved = async { presence.as_mut().expect("guarded").feed.next().await },
                        if running =>
                    {
                        match moved {
                            Some(moved) => rules.observe_presence(&moved),
                            None => {
                                if let Some(p) = presence.as_mut() {
                                    p.running = false;
                                }
                            }
                        }
                        continue;
                    }
                    // The tick cannot close before its own sweep has landed,
                    // and the drain keeps running until it does.
                    (doctor, lens, refreshed) = &mut sweep, if swept.is_none() => {
                        if let Some(c) = lens {
                            catalog = Some(c);
                        }
                        alarms.fold(refreshed);
                        swept = Some(doctor);
                        continue;
                    }
                    () = &mut tick_over, if swept.is_some() => break,
                };
                match item {
                    Some(StreamItem::Event(FleetEvent::Sample(s))) => {
                        let lens =
                            Lens::new(&bus.namespace, catalog.as_ref(), store).offline(offline);
                        rules.observe_sample(&s, &lens);
                    }
                    Some(StreamItem::Dropped(n)) => rules.observe_drop(n),
                    Some(_) => {}
                    None => closed = true,
                }
            }

            let doctor_outcome = match swept {
                Some(outcome) => outcome,
                None => {
                    let (doctor, lens, refreshed) = sweep.await;
                    if let Some(c) = lens {
                        catalog = Some(c);
                    }
                    alarms.fold(refreshed);
                    doctor
                }
            };
            let now = tokio::time::Instant::now();
            let at = crate::tape::record::rfc3339_now();
            let reads = alarms.reads();
            let transitions = rules.evaluate(
                now,
                &at,
                SweepOutcome {
                    doctor: doctor_outcome
                        .as_ref()
                        .map(|o| o.as_ref().map_err(String::as_str)),
                    presence: presence.as_ref().and_then(RulePresence::presence),
                    alarms: &reads,
                },
            );
            for transition in transitions {
                // Awaits, where the callback returned: the consumer's write
                // happens *here*, so its error returns from where it
                // happened (#360), and a slow consumer widens the next window
                // rather than stalling a drain (#338).
                sender.send(transition).await;
            }
            if closed || spec.ticks.is_some_and(|n| rules.ticks() >= n) {
                break;
            }
        }
        monitor.shutdown().await?;
        if let Some(p) = presence {
            p.shutdown().await?;
        }
        let (firing, unobservable) = rules.standing();
        Ok(WatchdogSummary {
            ticks: rules.ticks(),
            transitions: rules.transitions(),
            firing,
            unobservable,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{CheckReport, DoctorFinding, DoctorScope, DoctorSeverity};

    /// A zk2 doctor report whose checks are clean but for one finding per
    /// entry of `checks`, each under its own check.
    fn report_with(checks: &[CheckId]) -> DoctorReport {
        DoctorReport {
            scope: DoctorScope {
                namespace: String::new(),
                presence: crate::report::Asked::NotAsked,
                routers: crate::report::Asked::NotAsked,
                health: crate::report::Asked::NotAsked,
                config: crate::report::Asked::NotAsked,
            },
            checks: CheckId::ALL
                .into_iter()
                .map(|id| {
                    let findings = checks
                        .iter()
                        .filter(|c| **c == id)
                        .enumerate()
                        .map(|(n, c)| DoctorFinding {
                            severity: DoctorSeverity::Error,
                            check: *c,
                            subject: format!("s{n}"),
                            evidence: "e".into(),
                        })
                        .collect();
                    CheckReport::of(id, findings, vec![], "clean")
                })
                .collect(),
            unobservable: None,
        }
    }

    /// Every variant's canonical spelling parses back to itself, and a rule
    /// outside the vocabulary is an error that names the vocabulary — closed
    /// means closed. v1's two rules that read v1's planes are refused with
    /// where their question went.
    #[test]
    fn the_vocabulary_round_trips_and_is_closed() {
        let rules = [
            "rate-above prod/zk2/*/*/*/stream/** 5",
            "rate-below prod/zk2/host-a/tc/tc.netif.v1/state/namespaces 0.5",
            "silent-for prod/zk2/** 30",
            "invalid-payload prod/zk2/**",
            "qos-mismatch prod/zk2/*/tc/**",
            "doctor split-brain",
            "instance-gone host-a/tc",
            "instance-gone */tc",
            "alert-firing zk2/lab/*/alarms.v1/state/alarms/* warning",
            "alert-firing zk2/** critical",
            "dropped",
        ];
        for rule in rules {
            let parsed = Condition::parse(rule).expect(rule);
            assert_eq!(parsed.to_string(), rule, "canonical spelling round-trips");
        }
        let err = Condition::parse("origin-down h-aaaaaaaaaaaa").unwrap_err();
        assert!(err.to_string().contains("instance-gone"), "{err}");
        // v1's spelling, on alarms.v1 (#800, AC): the floor defaults to
        // warning, and a v1 selector is no alarm's.
        assert_eq!(
            Condition::parse("alert-firing zk2/**").unwrap().to_string(),
            "alert-firing zk2/** warning"
        );
        let err = Condition::parse("alert-firing v1/*/state/*/alert/*").unwrap_err();
        assert!(err.to_string().contains("base-relative"), "{err}");
        let err = Condition::parse("alert-firing zk2/** severe").unwrap_err();
        assert!(err.to_string().contains("not a severity"), "{err}");
        let err = Condition::parse("instance-gone Host A").unwrap_err();
        assert!(err.to_string().contains("not a rule"), "{err}");
        let err = Condition::parse("instance-gone host-a").unwrap_err();
        assert!(err.is_unaskable(), "{err}");
        let err = Condition::parse("if rate > 5 then page").unwrap_err();
        assert!(err.to_string().contains("closed"), "{err}");
        assert!(err.to_string().contains("rate-above"), "{err}");
        let err = Condition::parse("doctor no-such-check").unwrap_err();
        assert!(err.to_string().contains("split-brain"), "{err}");
    }

    /// The acceptance rule of #227: a drop under a completeness claim yields
    /// `unobservable`, **never** `ok` — across all three core judges.
    #[test]
    fn a_drop_under_a_completeness_claim_is_unobservable_never_ok() {
        let wire = CondState::from;
        assert!(judge_excess(false, 1).is_unobservable());
        assert_eq!(wire(judge_excess(false, 0)), CondState::Ok);
        assert_eq!(judge_excess(true, 7), Judgement::Established);
        assert!(judge_shortfall(true, 1).is_unobservable());
        assert_eq!(judge_shortfall(true, 0), Judgement::Established);
        assert_eq!(wire(judge_shortfall(false, 9)), CondState::Ok);
        let silence = |sample_within, span_observed, drop_free| {
            judge_silence(SilenceEvidence {
                sample_within,
                span_observed,
                drop_free,
            })
        };
        assert!(silence(false, true, false).is_unobservable());
        assert!(silence(false, false, true).is_unobservable());
        assert_eq!(silence(false, true, true), Judgement::Established);
        assert_eq!(wire(silence(true, true, false)), CondState::Ok);
    }

    /// The wire projection's documented mapping, polarity note included.
    #[test]
    fn cond_state_is_the_documented_projection_of_the_judgement_core() {
        assert_eq!(CondState::from(Judgement::Established), CondState::Firing);
        assert_eq!(
            CondState::from(Judgement::NotEstablished {
                reason: "clean".into()
            }),
            CondState::Ok
        );
        assert_eq!(
            CondState::from(Judgement::NotAsked),
            CondState::Unobservable
        );
        assert_eq!(
            CondState::from(Judgement::Unobservable {
                reason: "drops".into()
            }),
            CondState::Unobservable
        );
    }

    /// The window judges apply those rules: `rate-above` firing survives
    /// drops, its ok does not; a young watch cannot claim silence.
    #[test]
    fn window_judgement_applies_the_drop_rules() {
        let rule = Condition::parse("rate-above k/** 1").unwrap();
        let base = CondWindow {
            window_s: 10.0,
            observed_s: 10.0,
            ..CondWindow::default()
        };
        let over = CondWindow {
            samples: 20,
            dropped: 5,
            ..base
        };
        assert_eq!(rule.judge_window(&over).unwrap().state, CondState::Firing);
        let under_dropped = CondWindow {
            samples: 2,
            dropped: 5,
            ..base
        };
        assert_eq!(
            rule.judge_window(&under_dropped).unwrap().state,
            CondState::Unobservable
        );

        let rule = Condition::parse("silent-for k/** 30").unwrap();
        let young = CondWindow {
            window_s: 5.0,
            observed_s: 5.0,
            ..CondWindow::default()
        };
        let eval = rule.judge_window(&young).unwrap();
        assert_eq!(eval.state, CondState::Unobservable);
        assert!(eval.evidence.contains("watched only"), "{}", eval.evidence);
        let silent = CondWindow {
            window_s: 5.0,
            observed_s: 60.0,
            ..CondWindow::default()
        };
        assert_eq!(rule.judge_window(&silent).unwrap().state, CondState::Firing);
        let recently_dropped = CondWindow {
            last_drop_ago_s: Some(10.0),
            ..silent
        };
        assert_eq!(
            rule.judge_window(&recently_dropped).unwrap().state,
            CondState::Unobservable
        );
        let spoken = CondWindow {
            samples: 1,
            last_sample_ago_s: Some(3.0),
            ..silent
        };
        assert_eq!(rule.judge_window(&spoken).unwrap().state, CondState::Ok);
    }

    /// `invalid-payload` and `qos-mismatch` have three poles each: a
    /// failure checked is firing and names it; checks that all passed are
    /// ok; and a window with nothing checked is unobservable, with why —
    /// never ok (O4).
    #[test]
    fn a_payload_or_qos_rule_with_nothing_checked_is_unobservable() {
        let window = |samples, checked, invalid| CondWindow {
            window_s: 5.0,
            observed_s: 5.0,
            samples,
            checked,
            invalid,
            qos_judged: checked,
            qos_mismatched: invalid,
            ..CondWindow::default()
        };
        for rule in ["invalid-payload k/**", "qos-mismatch k/**"] {
            let rule = Condition::parse(rule).unwrap();
            let failing = WindowExamples {
                failure: Some("k/a: the first failure".into()),
                unchecked: None,
            };
            let fired = rule.judge_window_with(&window(4, 4, 1), &failing).unwrap();
            assert_eq!(fired.state, CondState::Firing);
            assert!(
                fired.evidence.contains("k/a: the first failure"),
                "{}",
                fired.evidence
            );
            let clean = rule
                .judge_window_with(&window(4, 4, 0), &WindowExamples::default())
                .unwrap();
            assert_eq!(clean.state, CondState::Ok, "{}", clean.evidence);
            let blind = WindowExamples {
                failure: None,
                unchecked: Some("no provider of this address in presence".into()),
            };
            let none = rule.judge_window_with(&window(4, 0, 0), &blind).unwrap();
            assert_eq!(none.state, CondState::Unobservable);
            assert!(none.evidence.contains("no provider"), "{}", none.evidence);
            let quiet = rule
                .judge_window_with(&window(0, 0, 0), &WindowExamples::default())
                .unwrap();
            assert_eq!(quiet.state, CondState::Unobservable);
            assert!(quiet.evidence.contains("no sample"), "{}", quiet.evidence);
        }
    }

    /// The projection's fixtures (#747): keys through the grammar's own
    /// builders, a seed, and a watch looked at once after it.
    mod presence {
        use super::super::*;
        use std::time::Duration;
        use zenkey_model::grammar::{Addr, InstanceId, instance_key};

        pub fn at(s: u64) -> SystemTime {
            SystemTime::UNIX_EPOCH + Duration::from_secs(s)
        }

        pub fn instance(a: &str, i: u64) -> String {
            let a: Addr = a.parse().expect("an address");
            instance_key(&a, &InstanceId::from_u64(i))
                .expect("a key")
                .as_str()
                .to_owned()
        }

        pub fn seed(keys: &[String], complete: bool) -> crate::model::catalog::Observed {
            crate::model::catalog::Observed::from_keys(Scope::all().selector(), keys, complete)
        }

        /// A projection seeded with `keys`, and a watch of `pattern` that
        /// tracked it first and looked once after.
        pub fn watched(
            pattern: &str,
            keys: &[String],
            complete: bool,
        ) -> (LivePresence, AddressWatch) {
            let mut p = LivePresence::new(Scope::all());
            let mut w = AddressWatch::new(Target::parse(pattern).expect("a pattern"));
            w.track(&mut p, at(0));
            p.seed(&seed(keys, complete), at(1));
            w.first_look(&p);
            (p, w)
        }
    }

    /// `instance-gone` on the live projection (#747), the poles of an exact
    /// address: held is ok; its going is firing once the complete read it
    /// owes shows none — heard withdrawn, what was last said stands (core
    /// §8.1, 0.37) — worded as what this reader could see and naming it;
    /// absent from the
    /// first complete read is firing from the start (the dead-man's
    /// switch); a read that ended at its timeout, and a `Dropped(n)`, are
    /// unobservable — never ok, never firing on their own; an evaluation
    /// given no reading, or no projection, is unobservable too.
    #[test]
    fn instance_gone_reads_the_projection_three_ways() {
        use presence::*;
        let judge = |w: &AddressWatch, p: &LivePresence| judge_instance_gone(Some(w), Some(p));

        let (mut p, mut w) = watched("host-a/tc", &[instance("host-a/tc", 1)], true);
        let e = judge(&w, &p);
        assert_eq!(e.state, CondState::Ok, "{}", e.evidence);
        assert!(
            e.evidence.contains("holds an instance token"),
            "{}",
            e.evidence
        );
        let t = p.token(&instance("host-a/tc", 1), false, at(2));
        w.fold(&t);
        assert_eq!(
            judge(&w, &p).state,
            CondState::Ok,
            "heard withdrawn: not gone until a read says so"
        );
        w.fold(&p.seed(&seed(&[], true), at(3)));
        let e = judge(&w, &p);
        assert_eq!(e.state, CondState::Firing);
        assert!(
            e.evidence.contains("gone: host-a/tc") && e.evidence.contains("visible to this reader"),
            "{}",
            e.evidence
        );

        let (p, w) = watched("host-a/tc", &[], true);
        assert_eq!(
            judge(&w, &p).state,
            CondState::Firing,
            "absent from the start"
        );

        let (p, w) = watched("host-a/tc", &[], false);
        let e = judge(&w, &p);
        assert_eq!(e.state, CondState::Unobservable);
        assert!(
            e.evidence.contains("ended at its timeout"),
            "{}",
            e.evidence
        );

        let (mut p, mut w) = watched("host-a/tc", &[instance("host-a/tc", 1)], true);
        let t = p.dropped(3, at(2));
        w.fold(&t);
        let e = judge(&w, &p);
        assert_eq!(e.state, CondState::Unobservable);
        assert!(e.evidence.contains("lost 3"), "{}", e.evidence);

        assert_eq!(
            judge_instance_gone(None, Some(&p)).state,
            CondState::Unobservable
        );
        let e = judge_instance_gone(Some(&w), None);
        assert_eq!(e.state, CondState::Unobservable);
        assert!(e.evidence.contains("not running"), "{}", e.evidence);
    }

    /// Under a wildcard (#747): one of two addresses gone fires, naming it,
    /// while the other holds; a pattern that never matched fires nothing,
    /// even under a complete read — nothing seen is nothing gone.
    #[test]
    fn a_wildcard_fires_per_address_and_never_on_nothing_seen() {
        use presence::*;
        let both = [instance("h1/tc", 1), instance("h2/tc", 2)];
        let (mut p, mut w) = watched("*/tc", &both, true);
        let e = judge_instance_gone(Some(&w), Some(&p));
        assert_eq!(e.state, CondState::Ok, "{}", e.evidence);
        assert!(
            e.evidence.contains("2 address(es) under */tc"),
            "{}",
            e.evidence
        );
        let t = p.token(&instance("h2/tc", 2), false, at(2));
        w.fold(&t);
        w.fold(&p.seed(&seed(&[instance("h1/tc", 1)], true), at(3)));
        let e = judge_instance_gone(Some(&w), Some(&p));
        assert_eq!(e.state, CondState::Firing);
        assert!(
            e.evidence.contains("gone: h2/tc") && e.evidence.contains("1 other address(es)"),
            "{}",
            e.evidence
        );

        let (p, w) = watched("*/tc", &[instance("h1/gui", 1)], true);
        let e = judge_instance_gone(Some(&w), Some(&p));
        assert_eq!(e.state, CondState::Ok);
        assert!(
            e.evidence.contains("no address matches */tc yet"),
            "{}",
            e.evidence
        );
    }

    /// The fold of the per-address states into the rule's one state:
    /// firing while any address is gone — across a loss, until a complete
    /// re-read says otherwise — else unobservable while the observer is,
    /// else ok.
    #[test]
    fn the_fold_is_firing_then_unobservable_then_ok() {
        use presence::*;
        let both = [instance("h1/tc", 1), instance("h2/tc", 2)];
        let (mut p, mut w) = watched("*/tc", &both, true);
        let state =
            |w: &AddressWatch, p: &LivePresence| judge_instance_gone(Some(w), Some(p)).state;
        w.fold(&p.token(&instance("h2/tc", 2), false, at(2)));
        w.fold(&p.seed(&seed(&[instance("h1/tc", 1)], true), at(2)));
        assert_eq!(state(&w, &p), CondState::Firing);
        w.fold(&p.dropped(5, at(3)));
        assert_eq!(
            state(&w, &p),
            CondState::Firing,
            "gone outranks unobservable"
        );
        // The re-read: h2/tc is back, so nothing is gone and the observer
        // sees again.
        let both = [instance("h1/tc", 1), instance("h2/tc", 3)];
        w.fold(&p.seed(&seed(&both, true), at(4)));
        assert_eq!(state(&w, &p), CondState::Ok);
        w.fold(&p.dropped(1, at(5)));
        assert_eq!(
            state(&w, &p),
            CondState::Unobservable,
            "nothing gone, nothing seen"
        );
    }

    /// The rule set carries one reading per `instance-gone` rule (#747):
    /// it says it wants presence, over the narrowest scope covering them,
    /// tracks each pattern before the seed, looks once after it, folds each
    /// batch of moves, and judges each rule from its own reading at the
    /// tick; what is gone stays gone on a tick given no projection.
    #[test]
    fn the_rule_set_judges_instance_gone_on_the_projection() {
        use presence::*;
        let mut rules = RuleSet::new(&[
            Condition::parse("instance-gone host-a/tc").expect("rule"),
            Condition::parse("instance-gone host-a/*").expect("rule"),
            Condition::parse("dropped").expect("rule"),
        ])
        .expect("compiles");
        assert!(rules.wants_presence());
        assert_eq!(rules.presence_scope().selector(), "zk2/host-a/*/@zk/**");
        let mut p = LivePresence::new(rules.presence_scope());
        rules.track_presence(&mut p, at(0));
        p.seed(&seed(&[instance("host-a/gui", 1)], true), at(1));
        rules.first_look(&p);
        let now = tokio::time::Instant::now();
        let to = |t: &[Transition], rule: &str| t.iter().find(|t| t.rule == rule).map(|t| t.to);
        let sweep = SweepOutcome {
            doctor: None,
            presence: Some(&p),
            ..SweepOutcome::default()
        };
        let t = rules.evaluate(now, "t0", sweep);
        assert_eq!(to(&t, "instance-gone host-a/tc"), Some(CondState::Firing));
        assert_eq!(to(&t, "instance-gone host-a/*"), Some(CondState::Ok));
        rules.observe_presence(&p.token(&instance("host-a/gui", 1), false, at(2)));
        // Heard withdrawn: a read is owed, and its answer decides.
        assert!(p.awaits_read());
        rules.observe_presence(&p.seed(&seed(&[], true), at(2)));
        let sweep = SweepOutcome {
            doctor: None,
            presence: Some(&p),
            ..SweepOutcome::default()
        };
        let t = rules.evaluate(now, "t1", sweep);
        assert_eq!(to(&t, "instance-gone host-a/*"), Some(CondState::Firing));
        assert_eq!(to(&t, "instance-gone host-a/tc"), None, "unchanged");
        let t = rules.evaluate(now, "t2", SweepOutcome::default());
        assert_eq!(
            to(&t, "instance-gone host-a/*"),
            None,
            "still firing: what is gone stays gone"
        );
        assert!(
            !RuleSet::new(&[Condition::Dropped])
                .expect("compiles")
                .wants_presence()
        );
    }

    /// A mock owner's samples (its descriptor's `meta.synthetic`) ride every
    /// window evidence line when present.
    #[test]
    fn synthetic_samples_are_said_out_loud() {
        let rule = Condition::parse("rate-above k/** 0.1").unwrap();
        let w = CondWindow {
            window_s: 10.0,
            observed_s: 10.0,
            samples: 20,
            synthetic: 3,
            ..CondWindow::default()
        };
        let eval = rule.judge_window(&w).unwrap();
        assert!(
            eval.evidence.contains("3 from a mock owner"),
            "{}",
            eval.evidence
        );
    }

    /// The transition machine: the first evaluation states the baseline
    /// (from `null`), an unchanged tick emits nothing, a genuine change
    /// emits exactly one line.
    #[test]
    fn transitions_fire_once_per_genuine_change_and_never_per_tick() {
        let eval = |state| Eval {
            state,
            evidence: "e".into(),
        };
        let mut rs = RuleState::new(Condition::Dropped);
        let first = rs.observe(eval(CondState::Ok), "t0").expect("baseline");
        assert_eq!(first.rule, "dropped", "the transition renders its rule");
        assert_eq!(first.from, None, "the baseline comes from null (O4)");
        assert_eq!(first.to, CondState::Ok);
        assert!(rs.observe(eval(CondState::Ok), "t1").is_none());
        assert!(rs.observe(eval(CondState::Ok), "t2").is_none());
        let change = rs.observe(eval(CondState::Firing), "t3").expect("a change");
        assert_eq!(change.from, Some(CondState::Ok));
        assert_eq!(change.to, CondState::Firing);
        assert!(rs.observe(eval(CondState::Firing), "t4").is_none());
    }

    /// The ndjson shape of a transition is a wire contract for scripts.
    #[test]
    fn transition_json_shape_is_pinned() {
        let t = Transition {
            rule: "silent-for k/** 30".into(),
            from: None,
            to: CondState::Unobservable,
            at: "2026-08-22T00:00:00Z".into(),
            evidence: "watched only 5.0s of a 30.0s silence claim".into(),
        };
        assert_eq!(
            serde_json::to_value(&t).unwrap(),
            serde_json::json!({
                "rule": "silent-for k/** 30",
                "from": null,
                "to": "unobservable",
                "at": "2026-08-22T00:00:00Z",
                "evidence": "watched only 5.0s of a 30.0s silence claim",
            })
        );
        let t = Transition {
            from: Some(CondState::Ok),
            to: CondState::Firing,
            ..t
        };
        let json = serde_json::to_value(&t).unwrap();
        assert_eq!(json["from"], "ok");
        assert_eq!(json["to"], "firing");
    }

    /// `doctor --transitions`'s delta.
    #[test]
    fn doctor_watch_reports_deltas_not_states() {
        let mut watch = DoctorWatch::new();
        let clean = report_with(&[]);
        let baseline = watch.observe(Ok(&clean), "t0");
        assert_eq!(baseline.len(), CheckId::ALL.len());
        assert!(baseline.iter().all(|t| t.from.is_none()));
        assert!(baseline.iter().all(|t| t.to == CondState::Ok));
        assert!(
            watch.observe(Ok(&clean), "t1").is_empty(),
            "an unchanged run emits nothing"
        );
        let drifted = report_with(&[CheckId::SplitBrain, CheckId::SplitBrain]);
        let changes = watch.observe(Ok(&drifted), "t2");
        assert_eq!(changes.len(), 1, "only the changed check transitions");
        assert_eq!(changes[0].rule, "doctor split-brain");
        assert_eq!(changes[0].to, CondState::Firing);
        assert!(changes[0].evidence.contains("2 finding(s)"));
        let failed = watch.observe(Err("session lost"), "t3");
        assert_eq!(failed.len(), CheckId::ALL.len());
        assert!(failed.iter().all(|t| t.to == CondState::Unobservable));
    }

    /// #510: a run that judged nothing is unobservable for every check.
    #[test]
    fn an_empty_scope_is_unobservable_for_every_check() {
        let mut watch = DoctorWatch::new();
        let empty = DoctorReport {
            unobservable: Some("nothing in scope".into()),
            ..report_with(&[CheckId::AdminUnreachable])
        };
        let baseline = watch.observe(Ok(&empty), "t0");
        assert_eq!(baseline.len(), CheckId::ALL.len());
        assert!(
            baseline.iter().all(|t| t.to == CondState::Unobservable
                && t.evidence.contains("judged nothing: nothing in scope")),
            "{baseline:?}"
        );
        let back = watch.observe(Ok(&report_with(&[])), "t1");
        assert_eq!(back.len(), CheckId::ALL.len());
        assert!(back.iter().all(|t| t.to == CondState::Ok));
    }

    /// The doctor a tick runs for its rules (#735): the checks they name,
    /// `deep` when one reads the owners' data plane, and the operator's word
    /// on the clocks handed through, never assumed.
    #[test]
    fn the_rules_doctor_takes_the_operators_word_on_the_clocks() {
        let rules = RuleSet::new(&[
            Condition::parse("doctor health-stale").expect("rule"),
            Condition::parse("doctor state-stamp-foreign").expect("rule"),
        ])
        .expect("compiles");
        let t = Duration::from_secs(1);
        let unsaid = rules.doctor_spec(t, false);
        assert!(!unsaid.clocks_synced);
        assert!(unsaid.deep);
        assert_eq!(
            unsaid.checks,
            [CheckId::StateStampForeign, CheckId::HealthStale]
                .into_iter()
                .collect()
        );
        assert!(rules.doctor_spec(t, true).clocks_synced);
        // population-over-bound reads the owners' data plane too (#735).
        let rules =
            RuleSet::new(&[Condition::parse("doctor population-over-bound").expect("rule")])
                .expect("compiles");
        assert!(rules.doctor_spec(t, false).deep);
    }

    /// `alert-firing` (#800, AC) is §5's second question at its floor: yes
    /// firing, no ok, everything else unobservable — an unread selector, a
    /// reader that could not open, an owner absent (not asked) — and a rule
    /// with no reading this tick is never ok.
    #[test]
    fn alert_firing_is_the_scopes_question_and_never_ok_when_unasked() {
        use crate::model::alarms::{AlarmSample, BookSpec, SeedRead};
        use crate::report::AlarmVia;
        use zenkey_model::alarms::Severity;
        use zenoh::time::{NTP64, Timestamp, TimestampId};
        let sel = "zk2/lab/*/alarms.v1/state/alarms/*";
        assert_eq!(
            judge_alert_firing(sel, Severity::Warning, None).state,
            CondState::Unobservable
        );
        assert_eq!(
            judge_alert_firing(sel, Severity::Warning, Some(Err("no session"))).state,
            CondState::Unobservable
        );
        let t0 = std::time::Instant::now();
        let mut spec = BookSpec::new(Duration::from_secs(1));
        spec.clocks_synced = true;
        let mut book = AlarmBook::new(AlarmScope::parse(sel).unwrap(), spec);
        let keys = vec!["zk2/lab/svc/@zk/instance/000000000000000a".to_owned()];
        let mut o = crate::model::catalog::Observed::from_keys("zk2/lab/*/@zk/**", &keys, true);
        let d: zenkey_model::descriptor::Descriptor = serde_json::from_value(serde_json::json!({
            "format": "zk2-descriptor/0.1", "service": "lab/svc",
            "instance": "000000000000000a",
            "interfaces": [{"iface": "alarms.v1",
                "contract": format!("sha256:{}", zenkey::alarms::FINGERPRINT),
                "minor": 0, "token": true}],
            "meta": {},
        }))
        .unwrap();
        o.descriptors = Some(
            [(
                (
                    "lab/svc".parse().unwrap(),
                    "000000000000000a".parse().unwrap(),
                ),
                crate::model::catalog::DescriptorRead::Served(Box::new(d)),
            )]
            .into(),
        );
        book.presence_read(&o, t0);
        let now = SystemTime::now();
        let stamp = Timestamp::new(
            NTP64::from(now.duration_since(SystemTime::UNIX_EPOCH).unwrap()),
            "a1".parse::<TimestampId>().unwrap(),
        );
        let doc = serde_json::json!({"timestamp": 1, "rule": "link_down", "severity": "warning",
            "state": "firing", "summary": "s", "labels": {"peer": "r2", "port": "eth0"}});
        book.seed(
            SeedRead {
                asked_at: t0,
                complete: true,
                replies: vec![AlarmSample {
                    key: "zk2/lab/svc/alarms.v1/state/alarms/a659f813308ad1da".into(),
                    delete: false,
                    payload: serde_json::to_vec(&doc).unwrap(),
                    stamp: Some(stamp),
                    at: t0,
                    wall: now,
                    via: AlarmVia::Get,
                }],
                discarded: 0,
            },
            t0,
            now,
        );
        let e = judge_alert_firing(sel, Severity::Warning, Some(Ok(&book)));
        assert_eq!(e.state, CondState::Firing, "{e:?}");
        assert!(e.evidence.contains("a659f813308ad1da"), "{e:?}");
        let e = judge_alert_firing(sel, Severity::Critical, Some(Ok(&book)));
        assert_eq!(e.state, CondState::Ok, "a warning is below critical: {e:?}");
        book.presence_read(
            &crate::model::catalog::Observed::from_keys("zk2/lab/*/@zk/**", &[], true),
            t0,
        );
        let e = judge_alert_firing(sel, Severity::Warning, Some(Ok(&book)));
        assert_eq!(
            e.state,
            CondState::Unobservable,
            "absent is not asked: {e:?}"
        );
        assert!(e.evidence.contains("never ok"), "{e:?}");
    }
}
