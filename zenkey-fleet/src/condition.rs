//! Conditions and the watchdog (#227) — transitions, not states.
//!
//! Three shipped features each hard-coded their own predicate over the
//! observation surface: `expect` (one window), `doctor --listen-for` (five
//! checks), `cutover` (silence). This module is the one **closed vocabulary**
//! they were each a spelling of: [`Condition`], evaluated to three states,
//! never two (RFC 09 §5.1 O4/O6) — `ok` / `firing` / **`unobservable`**. The
//! third state is the reason this exists: an alerting tool that cannot say
//! *"I could not tell"* is the one that pages at 3am for a dropped buffer. A
//! drop under a completeness claim yields `unobservable`, never `ok`.
//!
//! The vocabulary is deliberately closed — no expressions, no templating, no
//! rules engine. A new condition is a new variant, argued for the way a new
//! doctor check id is.
//!
//! The semantic core is three tiny rules — [`judge_shortfall`],
//! [`judge_excess`], [`judge_silence`] — shared with [`crate::expect`], so
//! the watchdog and the CI assertion cannot drift about what a drop means.

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use serde::Serialize;

use crate::report::DoctorReport;

/// One condition's evaluation state. Three, not two (RFC 09 §5.1 O4/O6):
/// `unobservable` is "I could not tell", which is neither "fine" nor "fire".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CondState {
    /// The condition conclusively does not hold.
    Ok,
    /// The condition conclusively holds.
    Firing,
    /// The observation cannot carry the claim: a drop under a completeness
    /// claim, a window shorter than the claim's span, or an ask that failed.
    Unobservable,
}

/// The closed condition vocabulary (#227), over the existing observation
/// surface. Each variant names what *firing* means; the drop rules are in
/// the judge functions this module documents.
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
    /// An observed payload on `selector` did not reach [`crate::Verdict::Valid`]
    /// (#159) — `Invalid` and `NotValidated` both count: asking for validity
    /// and getting "unknowable" is not valid. Scoped to what was observed
    /// and checked; the `ok` state claims "nothing checked failed", never
    /// "nothing invalid rode" — the drop count rides in the evidence.
    InvalidPayload { selector: String },
    /// An observed sample on `selector` did not ride its registry-declared
    /// QoS profile (RFC 04 §3). Same per-observed-sample scope as
    /// [`Condition::InvalidPayload`]; samples with no declared profile are
    /// unjudgeable and counted in the evidence, not the state.
    QosMismatch { selector: String },
    /// A doctor run reported at least one finding with this check id
    /// (the stable [`crate::CHECK_IDS`] vocabulary). A failed doctor run is
    /// unobservable for every doctor condition — never `ok`.
    DoctorCheck { check: String },
    /// The origin holds no `alive` token on the liveliness roster
    /// (RFC 04 §5). A roster that could not be asked is unobservable —
    /// silence is not a verdict (RFC 05 §3.1).
    OriginDown { origin: String },
    /// The observer itself dropped samples this window (RFC 09 §5.1 O6) —
    /// self-knowledge, so never unobservable.
    Dropped,
}

/// The rule grammar, spelled once for the parse error and the docs.
const VOCABULARY: &str = "rate-above <SEL> <HZ> | rate-below <SEL> <HZ> | \
     silent-for <SEL> <SECS> | invalid-payload <SEL> | qos-mismatch <SEL> | \
     doctor <CHECK-ID> | origin-down <ORIGIN> | dropped";

impl Condition {
    /// Parse one rule: whitespace-separated, kind first (Zenoh key
    /// expressions cannot contain whitespace, so the split is unambiguous).
    /// The vocabulary is closed; anything else is an error that spells it.
    pub fn parse(rule: &str) -> Result<Condition> {
        let hz = |s: &str, kind: &str| -> Result<f64> {
            let v: f64 = s
                .parse()
                .map_err(|_| anyhow::anyhow!("{kind}: {s:?} is not a number"))?;
            if !v.is_finite() || v < 0.0 {
                bail!("{kind}: the threshold must be a finite non-negative number");
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
                    bail!("silent-for: the span must be a positive number of seconds");
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
                if !crate::doctor::CHECK_IDS.contains(check) {
                    bail!(
                        "doctor: {check:?} is not a check id — the stable vocabulary is: {}",
                        crate::doctor::CHECK_IDS.join(", ")
                    );
                }
                Condition::DoctorCheck {
                    check: check.to_string(),
                }
            }
            ["origin-down", origin] => Condition::OriginDown {
                origin: origin.to_string(),
            },
            ["dropped"] => Condition::Dropped,
            _ => bail!(
                "not a rule: {rule:?} — the vocabulary is closed (no expressions, \
                 no templating): {VOCABULARY}"
            ),
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

    /// Judge one observation window. `None` for the conditions that are not
    /// window-scoped ([`Condition::DoctorCheck`], [`Condition::OriginDown`]).
    pub fn judge_window(&self, w: &Window) -> Option<Eval> {
        let synth = if w.synthetic > 0 {
            format!("; {} synthetic-marked (RFC 09 §5.3)", w.synthetic)
        } else {
            String::new()
        };
        let rate = if w.window_s > 0.0 {
            w.samples as f64 / w.window_s
        } else {
            0.0
        };
        Some(match self {
            Condition::RateAbove { hz, .. } => {
                let state = judge_excess(rate > *hz, w.dropped);
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
                let state = judge_shortfall(rate < *hz, w.dropped);
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
                let sample_within = w.last_sample_ago_s.map(|ago| ago < *for_s) == Some(true);
                let span_observed = w.observed_s >= *for_s;
                let drop_free = w.last_drop_ago_s.map(|ago| ago >= *for_s) != Some(false);
                let state = judge_silence(sample_within, span_observed, drop_free);
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
            Condition::InvalidPayload { .. } => Eval {
                state: if w.invalid > 0 {
                    CondState::Firing
                } else {
                    CondState::Ok
                },
                evidence: format!(
                    "{} of {} checked sample(s) did not reach Valid ({} observed, \
                     {} dropped{synth})",
                    w.invalid, w.checked, w.samples, w.dropped
                ),
            },
            Condition::QosMismatch { .. } => Eval {
                state: if w.qos_mismatched > 0 {
                    CondState::Firing
                } else {
                    CondState::Ok
                },
                evidence: format!(
                    "{} of {} judged sample(s) did not ride their declared profile \
                     ({} observed, {} with no declared profile to judge, \
                     {} dropped{synth})",
                    w.qos_mismatched,
                    w.qos_judged,
                    w.samples,
                    w.samples.saturating_sub(w.qos_judged),
                    w.dropped
                ),
            },
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
            Condition::DoctorCheck { .. } | Condition::OriginDown { .. } => return None,
        })
    }

    /// Judge a roster ask. `None` unless this is [`Condition::OriginDown`].
    /// `Err` is the ask failing, which is unobservable — silence is not a
    /// verdict (RFC 05 §3.1).
    pub fn judge_roster(
        &self,
        roster: Result<&BTreeMap<String, Vec<String>>, &str>,
    ) -> Option<Eval> {
        let Condition::OriginDown { origin } = self else {
            return None;
        };
        Some(match roster {
            Err(e) => Eval {
                state: CondState::Unobservable,
                evidence: format!("the roster could not be asked: {e}"),
            },
            Ok(r) => match r.get(origin) {
                Some(producers) => Eval {
                    state: CondState::Ok,
                    evidence: format!(
                        "{origin} holds an alive token ({} producer(s))",
                        producers.len()
                    ),
                },
                None => Eval {
                    state: CondState::Firing,
                    evidence: format!("{origin} holds no alive token (RFC 04 §5)"),
                },
            },
        })
    }

    /// Judge a doctor run. `None` unless this is [`Condition::DoctorCheck`].
    /// A failed run is unobservable for every doctor condition — never `ok`.
    pub fn judge_doctor(&self, outcome: Result<&DoctorReport, &str>) -> Option<Eval> {
        let Condition::DoctorCheck { check } = self else {
            return None;
        };
        Some(match outcome {
            Err(e) => Eval {
                state: CondState::Unobservable,
                evidence: format!("the doctor run failed: {e}"),
            },
            Ok(report) => {
                let mut hits = report.findings.iter().filter(|f| f.check == *check);
                match hits.next() {
                    Some(first) => Eval {
                        state: CondState::Firing,
                        evidence: format!(
                            "{} finding(s); first: {} — {}",
                            1 + hits.count(),
                            first.subject,
                            first.evidence
                        ),
                    },
                    None => Eval {
                        state: CondState::Ok,
                        evidence: format!("no {check} findings"),
                    },
                }
            }
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
            Condition::OriginDown { origin } => write!(f, "origin-down {origin}"),
            Condition::Dropped => write!(f, "dropped"),
        }
    }
}

// ─── the three-state rules (the vocabulary's semantic core) ─────────────────

/// The shortfall rule ([`Condition::RateBelow`]; `expect`'s count floor and
/// rate floor): too little was seen. Enough seen is conclusive `ok` even
/// under drops — a drop can only hide *more*. A shortfall with drops is
/// unobservable: the dropped samples could have filled it (RFC 09 §5.1 O6).
pub fn judge_shortfall(short: bool, dropped: u64) -> CondState {
    match (short, dropped) {
        (false, _) => CondState::Ok,
        (true, 0) => CondState::Firing,
        (true, _) => CondState::Unobservable,
    }
}

/// The excess rule ([`Condition::RateAbove`]; `expect`'s rate ceiling): too
/// much was seen. Firing is positive evidence, conclusive under drops. `ok`
/// is a completeness claim — it counts what did NOT happen — so under drops
/// it is unobservable, never `ok` (O6).
pub fn judge_excess(over: bool, dropped: u64) -> CondState {
    match (over, dropped) {
        (true, _) => CondState::Firing,
        (false, 0) => CondState::Ok,
        (false, _) => CondState::Unobservable,
    }
}

/// The silence rule ([`Condition::SilentFor`]; `expect --absent`): a sample
/// inside the span is conclusive `ok`; silence is provable only over a span
/// the observer actually watched (O4) drop-free (O6) — otherwise
/// unobservable, never `ok`.
pub fn judge_silence(sample_within: bool, span_observed: bool, drop_free: bool) -> CondState {
    if sample_within {
        CondState::Ok
    } else if span_observed && drop_free {
        CondState::Firing
    } else {
        CondState::Unobservable
    }
}

// ─── observations and evaluations ───────────────────────────────────────────

/// What one evaluation window observed on one condition's selector — the
/// facts, separated from the judgement so the judgement is pure.
#[derive(Debug, Clone, Copy, Default)]
pub struct Window {
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
    /// Samples whose payload did not reach `Valid`, among those checked.
    pub invalid: u64,
    /// Samples actually decode-checked (a budget bounds the cost).
    pub checked: u64,
    /// Samples that did not ride their declared QoS, among those judged.
    pub qos_mismatched: u64,
    /// Samples with a declared profile to judge against.
    pub qos_judged: u64,
    /// Samples carrying the RFC 09 §5.3 synthetic-traffic marker — generated
    /// traffic judged as real would be a self-inflicted page, so every
    /// evidence line carries the count.
    pub synthetic: u64,
}

/// One evaluation: the three-valued state, and the evidence for it.
#[derive(Debug, Clone, PartialEq)]
pub struct Eval {
    pub state: CondState,
    pub evidence: String,
}

/// One genuine state change — the only thing the watchdog ever emits.
#[derive(Debug, Clone, Serialize)]
pub struct Transition {
    /// The rule, in its canonical spelling ([`Condition`]'s `Display`).
    pub rule: String,
    /// `null` on the first evaluation: the baseline stated out loud, because
    /// inventing a prior state would answer a question nobody asked (O4).
    pub from: Option<CondState>,
    pub to: CondState,
    /// RFC 3339 wall clock.
    pub at: String,
    pub evidence: String,
}

/// One rule's transition detector: feed evaluations in, get a [`Transition`]
/// back **only** when the state genuinely changed. An unchanged tick returns
/// `None` — transitions, not states.
#[derive(Debug, Clone)]
pub struct RuleState {
    rule: String,
    state: Option<CondState>,
}

impl RuleState {
    pub fn new(rule: impl Into<String>) -> RuleState {
        RuleState {
            rule: rule.into(),
            state: None,
        }
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
            rule: self.rule.clone(),
            from,
            to: eval.state,
            at: at.into(),
            evidence: eval.evidence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant's canonical spelling parses back to itself, and a rule
    /// outside the vocabulary is an error that names the vocabulary — closed
    /// means closed.
    #[test]
    fn the_vocabulary_round_trips_and_is_closed() {
        let rules = [
            "rate-above v1/*/telemetry/** 5",
            "rate-below v1/h-aaaaaaaaaaaa/state/p/health 0.5",
            "silent-for v1/*/events/** 30",
            "invalid-payload v1/*/state/**",
            "qos-mismatch v1/*/telemetry/**",
            "doctor slice-sync",
            "origin-down h-aaaaaaaaaaaa",
            "dropped",
        ];
        for rule in rules {
            let parsed = Condition::parse(rule).expect(rule);
            assert_eq!(parsed.to_string(), rule, "canonical spelling round-trips");
        }
        let err = Condition::parse("if rate > 5 then page").unwrap_err();
        assert!(err.to_string().contains("closed"), "{err}");
        assert!(err.to_string().contains("rate-above"), "{err}");
        // A doctor rule outside the stable check-id vocabulary is refused at
        // parse, naming the vocabulary.
        let err = Condition::parse("doctor no-such-check").unwrap_err();
        assert!(err.to_string().contains("slice-sync"), "{err}");
    }

    /// The acceptance rule of #227: a drop under a completeness claim yields
    /// `unobservable`, **never** `ok` — across all three core judges.
    #[test]
    fn a_drop_under_a_completeness_claim_is_unobservable_never_ok() {
        // Excess: the "did not exceed" side counts what did not happen.
        assert_eq!(judge_excess(false, 1), CondState::Unobservable);
        assert_eq!(judge_excess(false, 0), CondState::Ok);
        // …while firing is positive evidence, conclusive under drops.
        assert_eq!(judge_excess(true, 7), CondState::Firing);
        // Shortfall: the drops could have carried the difference.
        assert_eq!(judge_shortfall(true, 1), CondState::Unobservable);
        assert_eq!(judge_shortfall(true, 0), CondState::Firing);
        // …while "enough seen" is conclusive: a drop only hides more.
        assert_eq!(judge_shortfall(false, 9), CondState::Ok);
        // Silence: unprovable over a dropped or unwatched span.
        assert_eq!(judge_silence(false, true, false), CondState::Unobservable);
        assert_eq!(judge_silence(false, false, true), CondState::Unobservable);
        assert_eq!(judge_silence(false, true, true), CondState::Firing);
        assert_eq!(judge_silence(true, true, false), CondState::Ok);
    }

    /// The window judges apply those rules: `rate-above` firing survives
    /// drops, its ok does not; a young watch cannot claim silence.
    #[test]
    fn window_judgement_applies_the_drop_rules() {
        let rule = Condition::parse("rate-above k/** 1").unwrap();
        let base = Window {
            window_s: 10.0,
            observed_s: 10.0,
            ..Window::default()
        };
        let over = Window {
            samples: 20,
            dropped: 5,
            ..base
        };
        assert_eq!(rule.judge_window(&over).unwrap().state, CondState::Firing);
        let under_dropped = Window {
            samples: 2,
            dropped: 5,
            ..base
        };
        assert_eq!(
            rule.judge_window(&under_dropped).unwrap().state,
            CondState::Unobservable
        );

        let rule = Condition::parse("silent-for k/** 30").unwrap();
        let young = Window {
            window_s: 5.0,
            observed_s: 5.0,
            ..Window::default()
        };
        let eval = rule.judge_window(&young).unwrap();
        assert_eq!(eval.state, CondState::Unobservable);
        assert!(eval.evidence.contains("watched only"), "{}", eval.evidence);
        let silent = Window {
            window_s: 5.0,
            observed_s: 60.0,
            ..Window::default()
        };
        assert_eq!(rule.judge_window(&silent).unwrap().state, CondState::Firing);
        let recently_dropped = Window {
            last_drop_ago_s: Some(10.0),
            ..silent
        };
        assert_eq!(
            rule.judge_window(&recently_dropped).unwrap().state,
            CondState::Unobservable
        );
        let spoken = Window {
            samples: 1,
            last_sample_ago_s: Some(3.0),
            ..silent
        };
        assert_eq!(rule.judge_window(&spoken).unwrap().state, CondState::Ok);
    }

    /// The synthetic-traffic marker count (RFC 09 §5.3, the #162 rider)
    /// rides every window evidence line when present.
    #[test]
    fn synthetic_marked_samples_are_said_out_loud() {
        let rule = Condition::parse("rate-above k/** 0.1").unwrap();
        let w = Window {
            window_s: 10.0,
            observed_s: 10.0,
            samples: 20,
            synthetic: 3,
            ..Window::default()
        };
        let eval = rule.judge_window(&w).unwrap();
        assert!(
            eval.evidence.contains("3 synthetic-marked"),
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
        let mut rs = RuleState::new("dropped");
        let first = rs.observe(eval(CondState::Ok), "t0").expect("baseline");
        assert_eq!(first.from, None, "the baseline comes from null (O4)");
        assert_eq!(first.to, CondState::Ok);
        assert!(rs.observe(eval(CondState::Ok), "t1").is_none());
        assert!(rs.observe(eval(CondState::Ok), "t2").is_none());
        let change = rs.observe(eval(CondState::Firing), "t3").expect("a change");
        assert_eq!(change.from, Some(CondState::Ok));
        assert_eq!(change.to, CondState::Firing);
        assert!(rs.observe(eval(CondState::Firing), "t4").is_none());
    }

    /// The ndjson shape of a transition is a wire contract for scripts:
    /// `{"rule","from","to","at","evidence"}`, states snake_case, `from`
    /// null on the baseline.
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
}
