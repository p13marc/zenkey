//! `expect` (#160) — one observation window, one verdict, exit-coded for CI.
//!
//! The shape every application test suite needs: "my producer's traffic shows
//! up, at roughly the right rate, with valid payloads" as a one-liner whose
//! exit code can be trusted. Trusted means three states, not two
//! (RFC 09 §5.1 O4/O6, the `cutover`/`probe` discipline):
//!
//! - **0 / Met** — the expectation held.
//! - **1 / NotMet** — it did not, *and the observation was clean enough to
//!   say so*: either conclusive positive evidence (a sample where none may
//!   be, a nonconformant payload, a rate above the ceiling), or a shortfall
//!   observed with zero drops.
//! - **2 / Impaired** — the observation cannot carry the claim: the
//!   subscriber dropped samples under a claim that needs completeness
//!   (absence, a rate ceiling, or a shortfall that the dropped samples could
//!   have filled), or the session/watch failed outright.
//!
//! The subscriber is declared **before** the window opens — a window that
//! starts counting before anyone listens converts "not asked" into "no".
//!
//! Since #227 the three-state judgement is spelled in the closed condition
//! vocabulary ([`crate::condition`]): the count and rate floors ride
//! [`crate::condition::judge_shortfall`]
//! (`rate-below`'s rule), the rate ceiling rides
//! [`crate::condition::judge_excess`]
//! (`rate-above`'s), and `--absent` is `silent-for` over the whole window
//! ([`crate::condition::judge_silence`]) — so
//! `expect` and `zenctl watchdog` cannot drift about what a drop means.

use std::collections::BTreeSet;
use std::time::Duration;

use anyhow::Result;
use zenoh::Session;

use crate::condition;
use crate::decode::SchemaStore;
use crate::registry::SliceSet;
use crate::report::{ExpectReport, ExpectVerdict};
use crate::{FleetEvent, Monitor, MonitorSpec, StreamItem, Verdict};

/// How many violation examples the report names (totals are exact).
const EXAMPLE_CAP: usize = 20;

/// Which QoS each observed sample must have ridden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QosCheck {
    /// The subject's declared profile (RFC 04 §3, via the registry). A sample
    /// whose key has no declared profile fails the check with that reason —
    /// the assertion was "rides as declared", and nothing is declared.
    Declared,
    /// One named profile for everything observed.
    Profile(zenkey::qos::QosProfile),
}

/// What must hold within the window.
#[derive(Debug, Clone)]
pub struct ExpectSpec {
    /// Full wire selector to watch (the session is un-namespaced, RFC 09 §5).
    pub selector: String,
    /// The observation window.
    pub within: Duration,
    /// At least this many samples. Defaults to 1 when nothing else implies
    /// presence; ignored under [`absent`](Self::absent).
    pub count: Option<u64>,
    /// Samples per second over the full window, at least.
    pub rate_min: Option<f64>,
    /// Samples per second over the full window, at most.
    pub rate_max: Option<f64>,
    /// Every observed payload must reach [`Verdict::Valid`] (#159). A payload
    /// that *cannot* be checked (no schema served) fails the assertion with
    /// its reason — asking for validity and getting "unknowable" is not met.
    pub valid_payload: bool,
    pub qos: Option<QosCheck>,
    /// Assert silence instead: no sample may match. The only absence claim
    /// this tool makes, and only because the verdict states its window and
    /// observer status, and any drop forces `Impaired` (O1/O4).
    pub absent: bool,
}

impl Default for ExpectSpec {
    fn default() -> Self {
        ExpectSpec {
            selector: String::new(),
            within: Duration::from_secs(30),
            count: None,
            rate_min: None,
            rate_max: None,
            valid_payload: false,
            qos: None,
            absent: false,
        }
    }
}

/// Run one expectation window. `Err` means the observation never stood up
/// (session/watch failure) — callers map it to the Impaired exit, never to
/// "not met".
///
/// `slices: None` means no registry was loaded: the decode pipeline then
/// reports [`Verdict::NotValidated`]([`NoRegistry`](zenkey::schema::validate::NotValidated::NoRegistry))
/// per sample, which `--valid` treats exactly like every other `NotValidated`
/// reason — the assertion was validity, and "unknowable" is not met
/// (RFC 09 §5.1 O4; #246). It is never a pass or a fail on its own.
pub async fn run_expect(
    session: &Session,
    base: &str,
    slices: Option<&SliceSet>,
    store: &SchemaStore,
    spec: &ExpectSpec,
) -> Result<ExpectReport> {
    let monitor = Monitor::start(session, MonitorSpec::default()).await?;
    let mut events = monitor.events();
    // Declared before the window opens: not-asked must never read as "no".
    monitor.watch(&spec.selector).await?;
    let opened = tokio::time::Instant::now();
    let deadline = opened + spec.within;

    // The existence floor: presence is implied unless silence is the claim.
    let need = if spec.absent {
        0
    } else {
        spec.count.unwrap_or(1)
    };
    let no_rate_bounds = spec.rate_min.is_none() && spec.rate_max.is_none();

    let mut samples: u64 = 0;
    let mut keys: BTreeSet<String> = BTreeSet::new();
    let mut dropped: u64 = 0;
    let mut violations: Vec<String> = Vec::new();
    let mut violations_total: u64 = 0;
    let mut ended_early = false;

    let violate = |list: &mut Vec<String>, total: &mut u64, line: String| {
        *total += 1;
        if list.len() < EXAMPLE_CAP {
            list.push(line);
        }
    };

    loop {
        let item = tokio::select! {
            item = events.recv() => item,
            _ = tokio::time::sleep_until(deadline) => break,
        };
        match item {
            Some(StreamItem::Event(FleetEvent::Sample(s))) => {
                samples += 1;
                keys.insert(s.key.clone());
                if spec.absent {
                    violate(
                        &mut violations,
                        &mut violations_total,
                        format!("{}: a sample where none may be", s.key),
                    );
                    // Conclusive — but keep draining so the report counts
                    // the full extent of the failure within the window.
                    continue;
                }
                if spec.valid_payload {
                    let d = crate::decode::decode_sample(
                        store,
                        session,
                        slices,
                        base,
                        &s.key,
                        Some(&s.encoding),
                        &s.payload.to_bytes(),
                    )
                    .await;
                    match d.verdict {
                        Verdict::Valid => {}
                        Verdict::Invalid(errors) => violate(
                            &mut violations,
                            &mut violations_total,
                            format!("{}: invalid — {}", s.key, errors.join("; ")),
                        ),
                        // Every not-validated reason — `NoRegistry`
                        // included — rides the same arm: the user asserted
                        // validity, and "unknowable" is not met. The reason
                        // string keeps the two silences apart (#246).
                        Verdict::NotValidated(reason) => violate(
                            &mut violations,
                            &mut violations_total,
                            format!("{}: validity unknowable — {reason}", s.key),
                        ),
                    }
                }
                if let Some(check) = spec.qos {
                    let against = match check {
                        QosCheck::Profile(p) => Some(p),
                        QosCheck::Declared => {
                            match crate::facts::describe_key(base, &s.key, slices)
                                .facts
                                .registration
                            {
                                crate::facts::Registration::Registered(f) => f.declared_qos(),
                                _ => None,
                            }
                        }
                    };
                    match against {
                        Some(p) if s.qos_matches(p) => {}
                        Some(p) => violate(
                            &mut violations,
                            &mut violations_total,
                            format!("{}: did not ride {} on the wire", s.key, p.name()),
                        ),
                        None => violate(
                            &mut violations,
                            &mut violations_total,
                            format!("{}: no declared profile to ride", s.key),
                        ),
                    }
                }
                // Early success: the count is in, nothing else can invalidate
                // it (rate bounds need the full window), nothing has.
                if no_rate_bounds && violations_total == 0 && samples >= need && need > 0 {
                    ended_early = true;
                    break;
                }
            }
            Some(StreamItem::Dropped(n)) => dropped += n,
            Some(_) => continue,
            None => break,
        }
    }
    monitor.stop();

    let window = if ended_early {
        opened.elapsed()
    } else {
        spec.within
    };
    let window_s = window.as_secs_f64();
    let rate_hz = (spec.rate_min.is_some() || spec.rate_max.is_some())
        .then(|| samples as f64 / spec.within.as_secs_f64());

    // Judge, in the #227 condition vocabulary. `positive` marks conclusive
    // evidence — a per-sample violation, a sample where none may be, an
    // excess firing — which stays NotMet even under drops; the shortfalls
    // ride `judge_shortfall`, whose drops make them unobservable instead.
    let mut unmet: Vec<String> = Vec::new();
    let mut positive = false;
    if violations_total > 0 {
        positive = true;
        unmet.push(if spec.absent {
            format!("{violations_total} sample(s) observed where none may be")
        } else {
            format!("{violations_total} sample(s) violated a per-sample requirement")
        });
    }
    let count_short = !spec.absent && samples < need;
    if count_short {
        unmet.push(format!("{samples} sample(s) observed, {need} required"));
    }
    let mut rate_short = false;
    if let (Some(min), Some(r)) = (spec.rate_min, rate_hz)
        && r < min
    {
        rate_short = true;
        unmet.push(format!("rate {r:.2} Hz below the {min:.2} Hz floor"));
    }
    if let (Some(max), Some(r)) = (spec.rate_max, rate_hz)
        && r > max
    {
        positive = true;
        unmet.push(format!("rate {r:.2} Hz above the {max:.2} Hz ceiling"));
    }

    let verdict = if unmet.is_empty() {
        // Every claim held on its face — but a met completeness claim under
        // drops is unobservable, never ok (O6): `--absent` is `silent-for`
        // over the whole window, the ceiling is `rate-above` asserted quiet.
        let met_states = [
            spec.absent
                .then(|| condition::judge_silence(false, true, dropped == 0)),
            spec.rate_max
                .map(|_| condition::judge_excess(false, dropped)),
        ];
        if met_states
            .into_iter()
            .flatten()
            .any(|j| j.is_unobservable())
        {
            unmet.push(format!(
                "{dropped} sample(s) dropped while the claim needs completeness (O6)"
            ));
            ExpectVerdict::Impaired
        } else {
            ExpectVerdict::Met
        }
    } else if positive {
        ExpectVerdict::NotMet
    } else {
        // A pure shortfall (no positive evidence): the judge's answer folds
        // through the documented RFC 13 mapping — an established shortfall
        // is `NotMet`, an unobservable one `Impaired` — rather than being
        // hand-mapped here.
        let shortfall = condition::judge_shortfall(count_short || rate_short, dropped);
        if shortfall.is_unobservable() {
            unmet.push(format!(
                "{dropped} sample(s) dropped — the shortfall may not be real (O6)"
            ));
        }
        ExpectVerdict::from(shortfall)
    };

    Ok(ExpectReport {
        selector: spec.selector.clone(),
        window_s,
        ended_early,
        samples,
        keys_seen: keys.len(),
        dropped,
        rate_hz,
        violations,
        violations_total,
        unmet,
        verdict,
    })
}
