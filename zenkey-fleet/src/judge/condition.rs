//! Conditions and the watchdog (#227) — transitions, not states.
//!
//! Three shipped features each hard-coded their own predicate over the
//! observation surface: `expect` (one window), `doctor --for` (five
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
//! [`judge_excess`], [`judge_silence`] — shared with [`crate::judge::expect`], so
//! the watchdog and the CI assertion cannot drift about what a drop means.
//! Since RFC 13 (v1.24; the material was RFC 09 §5.1 pre-v1.24) the rules
//! speak the four-pole [`Judgement`] core, and [`CondState`] is this
//! module's serde-stable **wire projection** of it — see its mapping doc.
//!
//! [`run_watchdog`] is the continuous observer over the vocabulary:
//! **foreground, explicitly launched, single-purpose, one process per
//! invocation, no shared state** — not the hidden, auto-started,
//! discovery-caching daemon the redesign ledger rejected
//! (`docs/redesign-2026-07.md` §6.1). It emits [`Transition`]s: one per
//! genuine state change, none per unchanged tick.

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Result, bail};

use crate::model::decode::SchemaStore;
use crate::model::registry::SliceSet;
use crate::report::DoctorReport;
use crate::report::{CondState, Judgement, Transition, WatchdogSummary};

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
    /// (the stable [`crate::judge::common::CHECK_IDS`] vocabulary). A failed doctor run is
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
                if !crate::judge::common::CHECK_IDS.contains(check) {
                    bail!(
                        "doctor: {check:?} is not a check id — the stable vocabulary is: {}",
                        crate::judge::common::CHECK_IDS.join(", ")
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
    pub fn judge_window(&self, w: &CondWindow) -> Option<Eval> {
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
                let sample_within = w.last_sample_ago_s.map(|ago| ago < *for_s) == Some(true);
                let span_observed = w.observed_s >= *for_s;
                let drop_free = w.last_drop_ago_s.map(|ago| ago >= *for_s) != Some(false);
                let state = CondState::from(judge_silence(sample_within, span_observed, drop_free));
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

// ─── the judgement rules (the vocabulary's semantic core) ───────────────────
//
// The three judges return the four-pole [`Judgement`] core (RFC 13, v1.24;
// RFC 09 §5.1 pre-v1.24). None of them ever answers `NotAsked` — a judge is
// only called when the question was put — but the pole exists in the currency
// so a caller that *skipped* a judge can say so in the same vocabulary. The
// watchdog projects each judgement onto [`CondState`] for the wire.

/// The shortfall rule ([`Condition::RateBelow`]; `expect`'s count floor and
/// rate floor): too little was seen. Enough seen is conclusively clean even
/// under drops — a drop can only hide *more*. A shortfall with drops is
/// unobservable: the dropped samples could have filled it (RFC 09 §5.1 O6).
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
pub fn judge_silence(sample_within: bool, span_observed: bool, drop_free: bool) -> Judgement {
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

// ─── observations and evaluations ───────────────────────────────────────────

/// What one evaluation window observed on one condition's selector — the
/// facts, separated from the judgement so the judgement is pure.
///
/// `CondWindow` and not `Window`: this type is re-exported at the crate root
/// beside `BudgetWindow` and `RecordBounds`, and a bare `Window` there reads
/// as *the* window of an engine that has several. Nothing serializes the
/// name (the type carries no `Serialize`), so the rename is Rust-side only.
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

/// Run-over-run delta over a doctor report: one [`RuleState`] per stable
/// check id ([`crate::judge::common::CHECK_IDS`]), fed by `doctor --transitions`. The
/// first run states the baseline (one transition per check id); every later run yields
/// only genuine changes. A failed run flips every check to `unobservable` —
/// a doctor that could not run has not said the fleet is healthy.
#[derive(Debug, Clone)]
pub struct DoctorWatch {
    checks: Vec<(Condition, RuleState)>,
}

impl DoctorWatch {
    pub fn new() -> DoctorWatch {
        DoctorWatch {
            checks: crate::judge::common::CHECK_IDS
                .iter()
                .map(|id| {
                    let condition = Condition::DoctorCheck {
                        check: id.to_string(),
                    };
                    let state = RuleState::new(condition.to_string());
                    (condition, state)
                })
                .collect(),
        }
    }

    /// Feed one doctor run (or its failure) and collect the transitions.
    pub fn observe(&mut self, outcome: Result<&DoctorReport, &str>, at: &str) -> Vec<Transition> {
        self.checks
            .iter_mut()
            .filter_map(|(condition, state)| {
                let eval = condition
                    .judge_doctor(outcome)
                    .expect("doctor conditions judge doctor runs");
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
    /// Per-ask timeout for the roster and doctor conditions.
    pub timeout: Duration,
}

/// How many decode attempts each key gets per tick under an
/// `invalid-payload` rule — the same budget the doctor listen phase runs,
/// for the same reason: a watchdog must not become a load test.
const DECODE_BUDGET: u8 = 2;

/// Watch the rules and emit one [`Transition`] per genuine change, none per
/// unchanged tick. The subscriber set is declared before the first window
/// opens (O4); every selector rule is judged per tick over the measured
/// window, doctor and roster rules by one ask per tick each.
pub async fn run_watchdog(
    fleet: &crate::Fleet<'_>,
    slices: Option<&SliceSet>,
    store: &SchemaStore,
    spec: &WatchdogSpec,
    emit: &mut (dyn FnMut(&Transition) + Send),
) -> Result<WatchdogSummary> {
    use crate::{FleetEvent, StreamItem};

    let (session, base) = (fleet.session(), fleet.base());

    #[derive(Default, Clone, Copy)]
    struct TickCounters {
        samples: u64,
        invalid: u64,
        checked: u64,
        qos_mismatched: u64,
        qos_judged: u64,
        synthetic: u64,
    }

    let mut states: Vec<RuleState> = spec
        .rules
        .iter()
        .map(|c| RuleState::new(c.to_string()))
        .collect();

    // Declared before the window opens — not-asked must never read as "no".
    let monitor = crate::Monitor::start(session, crate::MonitorSpec::default()).await?;
    let mut events = monitor.events();
    let mut watched: Vec<&str> = Vec::new();
    for rule in &spec.rules {
        if let Some(sel) = rule.selector()
            && !watched.contains(&sel)
        {
            monitor.watch(sel).await?;
            watched.push(sel);
        }
    }
    // Per-rule selector, compiled once for sample attribution.
    let keyexprs: Vec<Option<zenoh::key_expr::KeyExpr<'static>>> = spec
        .rules
        .iter()
        .map(|rule| {
            rule.selector()
                .map(|sel| {
                    zenoh::key_expr::KeyExpr::try_from(sel.to_string())
                        .map_err(|e| anyhow::anyhow!("{sel:?} is not a key expression: {e}"))
                })
                .transpose()
        })
        .collect::<Result<_>>()?;
    let wants_doctor = spec
        .rules
        .iter()
        .any(|r| matches!(r, Condition::DoctorCheck { .. }));
    let wants_roster = spec
        .rules
        .iter()
        .any(|r| matches!(r, Condition::OriginDown { .. }));

    let started = tokio::time::Instant::now();
    let mut counters: Vec<TickCounters> = vec![TickCounters::default(); spec.rules.len()];
    let mut last_sample: Vec<Option<tokio::time::Instant>> = vec![None; spec.rules.len()];
    let mut last_drop: Option<tokio::time::Instant> = None;
    let mut dropped_tick: u64 = 0;
    // Bounded (#107): the watchdog runs until stopped, so an unbounded
    // per-key map here is a leak on any bus with churning keys. Evictions
    // ride the summary (O6).
    let mut facts_cache = crate::model::facts::FactsCache::default();
    let mut decode_budget: BTreeMap<String, u8> = BTreeMap::new();

    let mut summary = WatchdogSummary {
        ticks: 0,
        transitions: 0,
        facts_evicted: 0,
    };
    let mut last_eval = started;
    let mut closed = false;
    loop {
        let deadline = last_eval + spec.tick;
        while !closed {
            let item = tokio::select! {
                item = events.recv() => item,
                _ = tokio::time::sleep_until(deadline) => break,
            };
            match item {
                Some(StreamItem::Event(FleetEvent::Sample(s))) => {
                    let Ok(key) = zenoh::key_expr::KeyExpr::try_from(s.key.as_str()) else {
                        continue;
                    };
                    let synthetic = s
                        .attachment
                        .as_ref()
                        .is_some_and(|a| crate::judge::common::is_synthetic_marker(&a.to_bytes()));
                    // Decode once per sample (budgeted per key per tick),
                    // shared by every invalid-payload rule the key matches.
                    let mut verdict: Option<crate::Verdict> = None;
                    for (i, rule) in spec.rules.iter().enumerate() {
                        let Some(sel) = &keyexprs[i] else { continue };
                        if !sel.intersects(&key) {
                            continue;
                        }
                        counters[i].samples += 1;
                        if synthetic {
                            counters[i].synthetic += 1;
                        }
                        last_sample[i] = Some(tokio::time::Instant::now());
                        match rule {
                            Condition::InvalidPayload { .. } => {
                                if verdict.is_none() {
                                    let budget = decode_budget.entry(s.key.clone()).or_default();
                                    if *budget < DECODE_BUDGET {
                                        *budget += 1;
                                        // An `invalid-payload` rule counts
                                        // every not-`Valid` verdict the same
                                        // way, so with no registry loaded
                                        // `NoRegistry` (#246) changes no
                                        // transition — only the reason the
                                        // sample was not validated.
                                        let d = crate::model::decode::decode_sample(
                                            fleet,
                                            store,
                                            slices,
                                            &s.key,
                                            Some(&s.encoding),
                                            &s.payload.to_bytes(),
                                        )
                                        .await;
                                        verdict = Some(d.verdict);
                                    }
                                }
                                if let Some(v) = &verdict {
                                    counters[i].checked += 1;
                                    if !matches!(v, crate::Verdict::Valid) {
                                        counters[i].invalid += 1;
                                    }
                                }
                            }
                            Condition::QosMismatch { .. } => {
                                facts_cache.ensure(base, &s.key, slices);
                                let facts = facts_cache.get(&s.key).expect("just ensured this key");
                                if let crate::model::facts::Registration::Registered(sf) =
                                    &facts.registration
                                    && let Some(profile) = sf.declared_qos()
                                {
                                    counters[i].qos_judged += 1;
                                    if !s.qos_matches(profile) {
                                        counters[i].qos_mismatched += 1;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Some(StreamItem::Dropped(n)) => {
                    dropped_tick += n;
                    last_drop = Some(tokio::time::Instant::now());
                }
                Some(_) => {}
                None => closed = true,
            }
        }

        // Evaluate the tick over the measured window, then say only what
        // changed.
        let now = tokio::time::Instant::now();
        let at = crate::tape::record::rfc3339_now();
        let doctor_outcome = if wants_doctor {
            Some(
                crate::judge::doctor::run_doctor(
                    fleet,
                    slices,
                    &crate::judge::doctor::DoctorSpec {
                        deep: false,
                        sample: None,
                        timeout: spec.timeout,
                        listen: None,
                    },
                )
                .await
                .map_err(|e| e.to_string()),
            )
        } else {
            None
        };
        let roster_outcome = if wants_roster {
            Some(
                crate::bus::roster::roster(fleet, spec.timeout)
                    .await
                    .map_err(|e| e.to_string()),
            )
        } else {
            None
        };
        for (i, rule) in spec.rules.iter().enumerate() {
            let eval = match rule {
                Condition::DoctorCheck { .. } => {
                    let outcome = doctor_outcome
                        .as_ref()
                        .expect("a doctor rule ran the doctor");
                    rule.judge_doctor(outcome.as_ref().map_err(String::as_str))
                }
                Condition::OriginDown { .. } => {
                    let outcome = roster_outcome
                        .as_ref()
                        .expect("an origin rule asked the roster");
                    rule.judge_roster(outcome.as_ref().map_err(String::as_str))
                }
                _ => rule.judge_window(&CondWindow {
                    window_s: (now - last_eval).as_secs_f64(),
                    observed_s: (now - started).as_secs_f64(),
                    samples: counters[i].samples,
                    dropped: dropped_tick,
                    last_sample_ago_s: last_sample[i].map(|t| (now - t).as_secs_f64()),
                    last_drop_ago_s: last_drop.map(|t| (now - t).as_secs_f64()),
                    invalid: counters[i].invalid,
                    checked: counters[i].checked,
                    qos_mismatched: counters[i].qos_mismatched,
                    qos_judged: counters[i].qos_judged,
                    synthetic: counters[i].synthetic,
                }),
            }
            .expect("every rule kind has a judge");
            if let Some(transition) = states[i].observe(eval, &at) {
                summary.transitions += 1;
                emit(&transition);
            }
        }
        counters.fill(TickCounters::default());
        dropped_tick = 0;
        decode_budget.clear();
        summary.ticks += 1;
        if closed || spec.ticks.is_some_and(|n| summary.ticks >= n) {
            break;
        }
        last_eval = now;
    }
    monitor.stop();
    summary.facts_evicted = facts_cache.evicted();
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{DoctorFinding, DoctorSeverity};

    fn report_with(checks: &[&str]) -> DoctorReport {
        DoctorReport {
            findings: checks
                .iter()
                .map(|c| DoctorFinding {
                    severity: DoctorSeverity::Error,
                    check: c.to_string(),
                    subject: "s".into(),
                    evidence: "e".into(),
                    citation: None,
                })
                .collect(),
            synced: crate::report::Asked::NotAsked,
            introspect_answered: 0,
            live_producers: 0,
            describe_served: 0,
            describe_missing: 0,
            routers: 0,
            router_version: None,
            deep: false,
            observation: None,
        }
    }

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
    /// `unobservable`, **never** `ok` — across all three core judges, now
    /// spoken in the [`Judgement`] core and projected onto [`CondState`]
    /// (RFC 13, v1.24).
    #[test]
    fn a_drop_under_a_completeness_claim_is_unobservable_never_ok() {
        let wire = CondState::from;
        // Excess: the "did not exceed" side counts what did not happen.
        assert!(judge_excess(false, 1).is_unobservable());
        assert_eq!(wire(judge_excess(false, 0)), CondState::Ok);
        // …while firing is positive evidence, conclusive under drops.
        assert_eq!(judge_excess(true, 7), Judgement::Established);
        // Shortfall: the drops could have carried the difference.
        assert!(judge_shortfall(true, 1).is_unobservable());
        assert_eq!(judge_shortfall(true, 0), Judgement::Established);
        // …while "enough seen" is conclusive: a drop only hides more.
        assert_eq!(wire(judge_shortfall(false, 9)), CondState::Ok);
        // Silence: unprovable over a dropped or unwatched span.
        assert!(judge_silence(false, true, false).is_unobservable());
        assert!(judge_silence(false, false, true).is_unobservable());
        assert_eq!(judge_silence(false, true, true), Judgement::Established);
        assert_eq!(wire(judge_silence(true, true, false)), CondState::Ok);
    }

    /// The wire projection's documented mapping, polarity note included:
    /// `NotEstablished` (established-clean) is `ok`, `Established` (the
    /// condition holds) is `firing`, and **both** unestablished poles land
    /// on `unobservable` — the wire cannot say more (RFC 13, v1.24).
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

    /// The synthetic-traffic marker count (RFC 09 §5.3, the #162 rider)
    /// rides every window evidence line when present.
    #[test]
    fn synthetic_marked_samples_are_said_out_loud() {
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

    /// `doctor --transitions`'s delta: the first run is a full baseline (every
    /// stable check id, once), an identical second run says nothing, a new
    /// finding transitions exactly its check — and a failed run flips every
    /// check to unobservable, never ok.
    #[test]
    fn doctor_watch_reports_deltas_not_states() {
        let mut watch = DoctorWatch::new();
        let clean = report_with(&[]);
        let baseline = watch.observe(Ok(&clean), "t0");
        assert_eq!(baseline.len(), crate::judge::common::CHECK_IDS.len());
        assert!(baseline.iter().all(|t| t.from.is_none()));
        assert!(baseline.iter().all(|t| t.to == CondState::Ok));

        assert!(
            watch.observe(Ok(&clean), "t1").is_empty(),
            "an unchanged run emits nothing"
        );

        let drifted = report_with(&["schema-drift", "schema-drift"]);
        let changes = watch.observe(Ok(&drifted), "t2");
        assert_eq!(changes.len(), 1, "only the changed check transitions");
        assert_eq!(changes[0].rule, "doctor schema-drift");
        assert_eq!(changes[0].to, CondState::Firing);
        assert!(changes[0].evidence.contains("2 finding(s)"));

        let failed = watch.observe(Err("session lost"), "t3");
        assert_eq!(
            failed.len(),
            crate::judge::common::CHECK_IDS.len(),
            "a failed run is unobservable for every check — never ok"
        );
        assert!(failed.iter().all(|t| t.to == CondState::Unobservable));
    }
}
