//! `expect` (#160) — one observation window, one verdict, exit-coded for CI.
//!
//! The shape every application test suite needs: "my service's resource
//! shows up, at roughly the right rate, with valid payloads" as a one-liner
//! whose exit code can be trusted. Trusted means three states, not two
//! (O4/O6, the `cutover`/`probe` discipline):
//!
//! - **0 / Met** — the expectation held.
//! - **1 / NotMet** — it did not, *and the observation was clean enough to
//!   say so*: either conclusive positive evidence (a sample where none may
//!   be, a nonconformant payload, a rate above the ceiling), or a shortfall
//!   observed with zero drops.
//! - **2 / Impaired** — the observation cannot carry the claim: the
//!   subscriber dropped samples under a claim that needs completeness
//!   (absence, a rate ceiling, or a shortfall that the dropped samples could
//!   have filled), a presence read that ended at its timeout, or the
//!   session/watch failed outright.
//!
//! **zk2** (#612, FJ8b). The expectation is written over a zk2 address and
//! resource, not a selector: the window is a subscription through the
//! runtime's `Consumer::for_tool` ([`crate::watch_resource`]) — R6's
//! discards and #671's unresolved samples counted apart from the lag, which
//! is what a drop means here — each sample decoded and checked against its
//! contract, its QoS against the resource's (§2.4). `present` asks presence
//! (§8.1) whether the address holds the interface's token. The judgement
//! core is unchanged.
//!
//! The subscriber is declared **before** the window opens — a window that
//! starts counting before anyone listens converts "not asked" into "no".
//!
//! Since #227 the three-state judgement is spelled in the closed condition
//! vocabulary ([`crate::judge::condition`]): the count and rate floors ride
//! [`crate::judge::condition::judge_shortfall`] (`rate-below`'s rule), the
//! rate ceiling rides [`crate::judge::condition::judge_excess`]
//! (`rate-above`'s), and `--absent` is `silent-for` over the whole window
//! ([`crate::judge::condition::judge_silence`]) — so `expect` and `zenctl
//! watchdog` cannot drift about what a drop means.

use std::collections::BTreeSet;
use std::time::Duration;

use zenkey_model::contract::Resource;
use zenkey_model::template::Bindings;
use zenoh::Session;

use crate::Result;
use crate::judge::common::FINDING_CAP;
use crate::judge::condition;
use crate::model::catalog::Revision;
use crate::model::examples::Examples;
use crate::model::target::Target;
use crate::report::{Conformance, ExpectPresence, ExpectReport, ExpectVerdict, WatchEvent};

/// What must hold within the window.
#[derive(Debug, Clone)]
pub struct ExpectSpec {
    /// The observation window.
    pub within: Duration,
    /// At least this many samples. Defaults to 1 when nothing else implies
    /// presence; ignored under [`absent`](Self::absent).
    pub count: Option<u64>,
    /// Samples per second over the full window, at least.
    pub rate_min: Option<f64>,
    /// Samples per second over the full window, at most.
    pub rate_max: Option<f64>,
    /// Every observed payload must conform to its declared type (§7.2,
    /// §7.3). A payload that *cannot* be checked (a raw type, a key no
    /// resource resolves) fails the assertion with its reason — asking for
    /// validity and getting "unknowable" is not met.
    pub valid_payload: bool,
    /// Every observed sample must ride its resource's declared QoS (§2.4).
    pub qos_declared: bool,
    /// Assert silence instead: no sample may arrive. The only absence claim
    /// this tool makes, and only because the verdict states its window and
    /// observer status, and any drop forces `Impaired` (O1/O4).
    pub absent: bool,
    /// The address must hold the interface's token (§8.1) within the
    /// window.
    pub present: bool,
}

impl Default for ExpectSpec {
    fn default() -> Self {
        ExpectSpec {
            within: Duration::from_secs(30),
            count: None,
            rate_min: None,
            rate_max: None,
            valid_payload: false,
            qos_declared: false,
            absent: false,
            present: false,
        }
    }
}

/// What an expectation is about: an address (or pattern), one revision of
/// an interface, and — unless only presence is asked — one resource with the
/// template values given.
pub struct ExpectAim<'a> {
    pub revision: &'a Revision,
    pub target: &'a Target,
    pub resource: Option<&'a Resource>,
    pub values: &'a Bindings,
}

/// How often a presence expectation re-reads the interface's tokens while
/// its window is open.
const PRESENCE_EVERY: Duration = Duration::from_millis(500);

/// Run one expectation window, on a session in the deployment's namespace.
/// `Err` means the observation never stood up (the subscription could not
/// be declared) — callers map it to the Impaired exit, never to "not met".
pub async fn run_expect(
    session: &Session,
    aim: ExpectAim<'_>,
    spec: &ExpectSpec,
) -> Result<ExpectReport> {
    let ExpectAim {
        revision,
        target,
        resource,
        values,
    } = aim;
    let mut watch = match resource {
        Some(r) => Some(crate::bus::consume::watch(session, revision, target, r, values).await?),
        None => None,
    };
    let opened = tokio::time::Instant::now();
    let deadline = opened + spec.within;

    // The existence floor: implied unless silence is the claim, or the
    // question is presence alone.
    let need = if spec.absent || watch.is_none() {
        0
    } else {
        spec.count.unwrap_or(1)
    };
    let no_rate_bounds = spec.rate_min.is_none() && spec.rate_max.is_none();
    let alive = format!("zk2/{}/@zk/alive/{}/**", target.address, revision.iface());

    let mut samples: u64 = 0;
    let mut keys: BTreeSet<String> = BTreeSet::new();
    let mut violations: Examples<String> = Examples::new(FINDING_CAP);
    let mut presence: Option<ExpectPresence> = None;
    let mut ended_early = false;

    let window_over = tokio::time::sleep_until(deadline);
    tokio::pin!(window_over);
    let mut presence_tick = tokio::time::interval(PRESENCE_EVERY);
    loop {
        // A presence read: until the token is seen, every half second.
        let seen = presence.as_ref().is_some_and(|p| !p.holders.is_empty());
        tokio::select! {
            next = async {
                match watch.as_mut() {
                    Some(w) => w.next().await,
                    None => std::future::pending().await,
                }
            } => {
                let Some(s) = next else { break };
                samples += 1;
                keys.insert(s.key.clone());
                if spec.absent {
                    violations.push(format!("{}: a sample where none may be", s.key));
                    continue;
                }
                if spec.valid_payload {
                    match &s.event {
                        WatchEvent::Put { conformance, .. } => match conformance {
                            Conformance::Valid => {}
                            Conformance::Invalid { violations: v } => {
                                violations.push(format!("{}: invalid — {}", s.key, v.join("; ")))
                            }
                            Conformance::Undecodable { declared, reason } => violations.push(
                                format!("{}: does not decode as {declared} — {reason}", s.key),
                            ),
                            Conformance::NotChecked { reason } => violations
                                .push(format!("{}: validity unknowable — {reason}", s.key)),
                        },
                        WatchEvent::Delete => {}
                    }
                }
                if spec.qos_declared
                    && let Some(m) = &s.qos_mismatch
                {
                    violations.push(format!("{}: {}", s.key, m.summary()));
                }
                let presence_met = !spec.present || presence.as_ref().is_some_and(|p| !p.holders.is_empty());
                if no_rate_bounds
                    && violations.total() == 0
                    && samples >= need
                    && need > 0
                    && presence_met
                {
                    ended_early = true;
                    break;
                }
            }
            _ = presence_tick.tick(), if spec.present && !seen => {
                let read = crate::bus::presence::liveliness_read(session, &alive, spec.within).await;
                presence = Some(match read {
                    Ok(r) => ExpectPresence {
                        selector: alive.clone(),
                        holders: r
                            .keys
                            .iter()
                            .filter_map(|k| match zenkey_model::grammar::parse(k) {
                                Ok(zenkey_model::grammar::ZkKey::Alive { addr, .. }) => {
                                    Some(addr.to_string())
                                }
                                _ => None,
                            })
                            .collect::<BTreeSet<_>>()
                            .into_iter()
                            .collect(),
                        complete: r.complete,
                        error: None,
                    },
                    Err(e) => ExpectPresence {
                        selector: alive.clone(),
                        holders: Vec::new(),
                        complete: false,
                        error: Some(crate::one_line(&e)),
                    },
                });
                let met = presence.as_ref().is_some_and(|p| !p.holders.is_empty());
                if met && (watch.is_none() || (no_rate_bounds && violations.total() == 0 && samples >= need && need > 0)) {
                    ended_early = true;
                    break;
                }
            }
            () = &mut window_over => break,
        }
    }
    let (dropped, discarded, unresolved, selectors) = match &watch {
        Some(w) => (
            w.lagged(),
            w.discarded(),
            w.unresolved(),
            w.selectors().to_vec(),
        ),
        None => (0, 0, 0, Vec::new()),
    };
    drop(watch);

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
    // excess firing, a complete presence read with no token — which stays
    // NotMet even under drops; the shortfalls ride `judge_shortfall`, whose
    // drops make them unobservable instead.
    let mut unmet: Vec<String> = Vec::new();
    let mut positive = false;
    let mut impaired = Vec::new();
    let violations_total = violations.total() as u64;
    if violations_total > 0 {
        positive = true;
        unmet.push(if spec.absent {
            format!("{violations_total} sample(s) observed where none may be")
        } else {
            format!("{violations_total} sample(s) violated a per-sample requirement")
        });
    }
    if spec.present {
        match &presence {
            Some(p) if !p.holders.is_empty() => {}
            Some(p) if p.complete && p.error.is_none() => {
                positive = true;
                unmet.push(format!(
                    "no token of {} for {} visible to this reader (a refused read is empty too)",
                    target.address,
                    revision.iface()
                ));
            }
            Some(p) => impaired.push(match &p.error {
                Some(e) => format!("the presence read could not be made: {e}"),
                None => {
                    "the presence read ended at its timeout: possibly incomplete (§8.1)".to_owned()
                }
            }),
            None => impaired.push("presence was not read within the window".to_owned()),
        }
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

    let verdict = if positive {
        ExpectVerdict::NotMet
    } else if !impaired.is_empty() {
        unmet.extend(impaired);
        ExpectVerdict::Impaired
    } else if unmet.is_empty() {
        // Every claim held on its face — but a met completeness claim under
        // drops is unobservable, never ok (O6).
        let met_states = [
            spec.absent.then(|| {
                condition::judge_silence(condition::SilenceEvidence {
                    sample_within: false,
                    span_observed: true,
                    drop_free: dropped == 0,
                })
            }),
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
    } else {
        let shortfall = condition::judge_shortfall(count_short || rate_short, dropped);
        if shortfall.is_unobservable() {
            unmet.push(format!(
                "{dropped} sample(s) dropped — the shortfall may not be real (O6)"
            ));
        }
        ExpectVerdict::from(shortfall)
    };

    Ok(ExpectReport {
        address: target.address.clone(),
        iface: revision.iface().to_string(),
        fingerprint: revision.fingerprint().to_string(),
        resource: resource.map(zk2::implementation::resource_name),
        selectors,
        window_s,
        ended_early,
        samples,
        keys_seen: keys.len(),
        dropped,
        discarded,
        unresolved,
        rate_hz,
        presence,
        violations: violations.into_vec(),
        violations_total,
        unmet,
        verdict,
    })
}
