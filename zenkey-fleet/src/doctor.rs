//! The doctor checks as engine functions (#55): every finding both frontends
//! render comes from here — `zenctl doctor` orchestrates and renders, the
//! zengui doctor panel calls the same [`run_doctor`] and renders the same
//! [`DoctorReport`]. A check that lives in one frontend is a check the other
//! frontend's user never sees (RFC 08 §6.1's argument, applied to ourselves).
//!
//! Check ids are **stable API**: scripts key on them (`--format json`), the
//! GUI keys deltas on them. New checks add ids; nothing renames one. The full
//! set is pinned in [`CHECK_IDS`].

use std::time::Duration;

use anyhow::{Result, anyhow};
use zenkey::RegistrySlice;
use zenkey::grammar::with_base;
use zenoh::Session;

use crate::query::{Answer, RepeatingRegistry, fleet_get, state_snapshot};
use crate::report::{DoctorFinding, DoctorReport, DoctorSeverity};

/// Every check id `run_doctor` can emit — the stable vocabulary, never
/// renamed (see the module doc).
pub const CHECK_IDS: [&str; 16] = [
    "slice-parse",
    "slice-sync",
    "introspect-coverage",
    "admin-unreachable",
    "router-version-skew",
    "describe-totality",
    "schema-drift",
    "describe-missing",
    "stale-state",
    "unstamped-state",
    "storage-coverage",
    // The `--listen` passive phase (#161) — traffic judged as it rides.
    "payload-undecodable",
    "payload-invalid",
    "qos-observed-mismatch",
    "unregistered-traffic",
    "rate-over-declared",
];

/// What a doctor run should cost.
#[derive(Debug, Clone)]
pub struct DoctorSpec {
    /// Run the deep checks too (per-family state snapshots for freshness,
    /// storage-coverage join) — real query load, opt-in.
    pub deep: bool,
    /// At most this many state samples drained per family in the deep
    /// checks (`--sample N`) — bounds the sweep's cost, not just its output.
    /// `None` = unbounded.
    pub sample: Option<usize>,
    /// Per-query timeout.
    pub timeout: Duration,
    /// Listen passively to the data planes for this long after the GET
    /// fan-in (`--listen`, #161) and judge what rides: decode/validity,
    /// declared-vs-observed QoS, unregistered traffic, over-rate events.
    /// `None` = the phase does not run and the report carries no
    /// observation section.
    pub listen: Option<Duration>,
}

fn finding(
    severity: DoctorSeverity,
    check: &str,
    subject: impl Into<String>,
    evidence: impl Into<String>,
    citation: Option<&str>,
) -> DoctorFinding {
    DoctorFinding {
        severity,
        check: check.to_string(),
        subject: subject.into(),
        evidence: evidence.into(),
        citation: citation.map(str::to_string),
    }
}

/// The introspect key for a slice — a service origin's verbatim `@` chunk is
/// structurally unmatchable by a fleet selector's `*` (property D4), so it
/// takes its own key. That is the grammar working, not an exception to it.
fn rpc_key(base: &str, slice: &RegistrySlice, procedure: &str) -> Result<String> {
    Ok(match &slice.service_origin {
        Some(origin) => {
            let o = zenkey::ServiceOrigin::new(origin)
                .map_err(|e| anyhow!("bad service origin in slice {}: {e}", slice.name))?;
            with_base(base, zenkey::selector::service_rpc(&o, &[procedure]))
        }
        None => with_base(base, zenkey::selector::fleet_rpc(&slice.name, &[procedure])),
    })
}

/// Run every check against the live fleet and report typed findings.
///
/// `locals` are the caller's registry slices (loaded from `--registry` dirs
/// or GUI settings); empty means the served-vs-declared diff is skipped and
/// only bus-derived checks run — the caller states that degradation to its
/// user (O4: "not asked" must not render as "in sync").
pub async fn run_doctor(
    session: &Session,
    base: &str,
    locals: &[RegistrySlice],
    spec: &DoctorSpec,
) -> Result<DoctorReport> {
    let roster = crate::roster(session, base, spec.timeout).await?;

    let mut findings: Vec<DoctorFinding> = Vec::new();
    let mut synced: Vec<String> = Vec::new();
    let mut answered = 0usize;

    // --- served-vs-declared diff (RFC 08 §6) --------------------------
    for local in locals {
        let key = rpc_key(base, local, "introspect")?;
        let answers = fleet_get(session, base, &key, None, spec.timeout).await?;
        for answer in &answers {
            let Answer::Value(bytes) = &answer.answer else {
                continue;
            };
            answered += 1;
            let served_toml = bytes.to_bytes();
            let served_toml = String::from_utf8_lossy(&served_toml);
            let served = match zenkey::parse_slice(&served_toml) {
                Ok(s) => s,
                Err(e) => {
                    findings.push(finding(
                        DoctorSeverity::Error,
                        "slice-parse",
                        format!("{}/{}", answer.origin, local.name),
                        format!("served slice does not parse: {e}"),
                        Some("RFC 08 §6"),
                    ));
                    continue;
                }
            };
            let diff = zenkey::slice::diff(&served, local);
            if diff.is_empty() {
                synced.push(format!(
                    "{}/{} (registry {})",
                    answer.origin, local.name, served.version
                ));
            } else {
                for f in &diff {
                    findings.push(finding(
                        DoctorSeverity::Error,
                        "slice-sync",
                        format!("{}/{}", answer.origin, local.name),
                        f.summary(),
                        Some("RFC 08 §6"),
                    ));
                }
            }
        }
    }

    // One declared registry sweep (#37) serves both fallbacks below —
    // doctor used to fan the identical wildcard GETs twice per run.
    let sweep = if locals.is_empty() {
        let repeating = RepeatingRegistry::declare(session, base, spec.timeout).await?;
        let slices: Vec<RegistrySlice> = repeating
            .fetch()
            .await?
            .into_iter()
            .map(|(s, _)| s)
            .collect();
        repeating.undeclare().await?;
        Some(slices)
    } else {
        None
    };

    // With no local registry the only introspect coverage we can count is
    // the fleet-wide wildcard.
    if let Some(slices) = &sweep {
        answered = slices.len();
    }

    // The roster is what makes silence legible (RFC 05 §3.1): a producer
    // that holds an `alive` token but did not answer `introspect` is a bug,
    // because producers MUST declare their @rpc queryables *before* their
    // token — "alive ⇒ callable" (RFC 04 §5).
    let live: usize = roster.values().map(Vec::len).sum();
    if answered < live {
        findings.push(finding(
            DoctorSeverity::Error,
            "introspect-coverage",
            "fleet",
            format!(
                "{} live producer(s) did not answer introspect — alive ⇒ callable, \
                 so this is a finding, not a boot race",
                live - answered
            ),
            Some("RFC 04 §5"),
        ));
    }

    // --- admin reachability ------------------------------------------
    let routers = crate::routers(session, spec.timeout)
        .await
        .unwrap_or_default();
    let mut router_version = None;
    if routers.is_empty() {
        findings.push(finding(
            DoctorSeverity::Info,
            "admin-unreachable",
            "mesh",
            "no routers answered @/*/router (peer-only mesh, or the admin space is \
             disabled) — storage/version checks skipped",
            None,
        ));
    } else {
        let versions: std::collections::BTreeSet<&str> = routers
            .iter()
            .filter_map(|r| r.version.as_deref())
            .collect();
        if versions.len() > 1 {
            findings.push(finding(
                DoctorSeverity::Error,
                "router-version-skew",
                "mesh",
                format!("router version skew across the mesh: {versions:?}"),
                None,
            ));
        } else {
            router_version = versions.iter().next().map(|v| v.to_string());
        }
    }

    // --- schema conformance (RFC 08 §7) ------------------------------
    // Which slices to judge: the locals when given, else what the fleet
    // serves (the sweep above).
    let schema_slices: Vec<RegistrySlice> = match sweep {
        Some(slices) => slices,
        None => locals.to_vec(),
    };
    let mut described: Vec<(String, zenkey::schema::SchemaSet)> = Vec::new();
    let mut undescribed = 0usize;
    for slice in &schema_slices {
        let key = rpc_key(base, slice, "describe")?;
        let answers = fleet_get(session, base, &key, None, spec.timeout).await?;
        let set = answers.into_iter().find_map(|a| match a.answer {
            Answer::Value(bytes) => {
                let cow = bytes.to_bytes();
                std::str::from_utf8(&cow)
                    .ok()
                    .and_then(|t| zenkey::schema::SchemaSet::parse(t).ok())
            }
            Answer::Error { .. } => None,
        });
        match set {
            Some(set) => described.push((slice.name.clone(), set)),
            None => undescribed += 1,
        }
    }
    // Totality through the one engine implementation (`totality_gaps`) —
    // doctor used to carry a parallel referenced-names path.
    let slice_set = crate::registry::SliceSet::from_slices(schema_slices.clone());
    for gap in crate::decode::totality_gaps(&described, &slice_set) {
        findings.push(finding(
            DoctorSeverity::Error,
            "describe-totality",
            gap.producer.clone(),
            format!(
                "describe is not total — missing: {}",
                gap.missing.join(", ")
            ),
            Some("RFC 08 §7"),
        ));
    }
    for drift in crate::decode::schema_drift(&described) {
        let servers: Vec<String> = drift
            .servers
            .iter()
            .map(|(p, h)| format!("{p} ({h})"))
            .collect();
        findings.push(finding(
            DoctorSeverity::Error,
            "schema-drift",
            drift.type_name.clone(),
            format!("served with different schemas by {}", servers.join(", ")),
            Some("RFC 08 §7"),
        ));
    }
    if undescribed > 0 {
        findings.push(finding(
            DoctorSeverity::Info,
            "describe-missing",
            "fleet",
            format!(
                "{undescribed} producer(s) serve no describe (a SHOULD; generic tools \
                 render their payloads structurally)"
            ),
            Some("RFC 08 §7"),
        ));
    }

    // --- deep: freshness + storage coverage --------------------------
    if spec.deep {
        let now = std::time::SystemTime::now();
        let mut unstamped = 0usize;
        for slice in &schema_slices {
            for subject in &slice.subjects {
                let (Some(ttl), "state") = (subject.ttl_s, subject.class.as_str()) else {
                    continue;
                };
                let Ok(pattern) = zenkey::pattern::SubjectPattern::parse(&subject.path) else {
                    continue;
                };
                let selector = match &slice.service_origin {
                    Some(origin) => with_base(
                        base,
                        format!("v1/{origin}/state/{}", pattern.selector_tail()),
                    ),
                    None => with_base(
                        base,
                        format!("v1/*/state/{}/{}", slice.name, pattern.selector_tail()),
                    ),
                };
                let samples = state_snapshot(session, &selector, spec.timeout, spec.sample).await?;
                let (family_findings, family_unstamped) = judge_state_samples(&samples, ttl, now);
                findings.extend(family_findings);
                unstamped += family_unstamped;
            }
        }
        if unstamped > 0 {
            findings.push(finding(
                DoctorSeverity::Warning,
                "unstamped-state",
                "fleet",
                format!(
                    "{unstamped} state sample(s) carry no HLC timestamp — the deployment \
                     lacks timestamping, which LWW requires; freshness is unjudgeable \
                     for them"
                ),
                Some("RFC 04 §4"),
            ));
        }
        let storages = crate::storages(session, spec.timeout)
            .await
            .unwrap_or_default();
        let coverage = crate::state_coverage(&slice_set, base, &storages);
        let uncovered: Vec<&crate::CoverageRow> = coverage
            .iter()
            .filter(|r| r.coverage == crate::Coverage::Uncovered)
            .collect();
        if !uncovered.is_empty() {
            findings.push(finding(
                DoctorSeverity::Info,
                "storage-coverage",
                "fleet",
                format!(
                    "{} state famil(y|ies) have no storage coverage (volatile seeding \
                     may ride the advanced-pub/sub cache): {}",
                    uncovered.len(),
                    uncovered
                        .iter()
                        .map(|r| format!("{}/{}", r.producer, r.path))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                Some("RFC 04 §3.5"),
            ));
        }
    }

    // --- listen: judge what actually rides (#161) --------------------
    let observation = match spec.listen {
        Some(window) => {
            let store = crate::decode::SchemaStore::new(base, spec.timeout);
            let (listen_findings, summary) =
                observe_traffic(session, base, &slice_set, &store, window).await?;
            findings.extend(listen_findings);
            Some(summary)
        }
        None => None,
    };

    Ok(DoctorReport {
        findings,
        synced,
        introspect_answered: answered,
        live_producers: live,
        describe_served: described.len(),
        describe_missing: undescribed,
        routers: routers.len(),
        router_version,
        deep: spec.deep,
        observation,
    })
}

/// How many decode attempts each key gets during the listen window — the
/// budget that keeps a hot bus from turning the doctor into a load test.
const DECODE_BUDGET: u8 = 2;

/// How many per-key findings each listen check emits before summarising.
const FINDING_CAP: usize = 20;

/// The declared events rate class as an hourly cap (RFC 04 §1.3):
/// `rare` ≤ 1/h, `low` ≤ 1/min, `burst(n/h)` a declared cap.
pub(crate) fn rate_cap_per_hour(rate: &str) -> Option<u64> {
    match rate {
        "rare" => Some(1),
        "low" => Some(60),
        other => other
            .strip_prefix("burst(")?
            .strip_suffix("/h)")?
            .parse()
            .ok(),
    }
}

/// Does an attachment carry the RFC 09 §5.2 synthetic-traffic marker
/// (`{"synthetic": true, …}`, #162)? Generated traffic judged as real would
/// be a self-inflicted finding, so the observation counts it separately.
fn is_synthetic_marker(attachment: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(attachment)
        .ok()
        .and_then(|v| v.get("synthetic").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

/// The passive listening phase: watch the data planes for `window`, judge
/// each sample through the ladders that already exist — the Registration
/// ladder, `qos_matches`, `decode_sample` — and aggregate per key so a hot
/// key is one finding with a count, not a finding per sample.
async fn observe_traffic(
    session: &Session,
    base: &str,
    slices: &crate::registry::SliceSet,
    store: &crate::decode::SchemaStore,
    window: Duration,
) -> Result<(Vec<DoctorFinding>, crate::report::ObservationSummary)> {
    use std::collections::BTreeMap;

    // Scope statement (O5): the three data classes for host origins, plus
    // each declared service origin's three — `*` never matches an `@` chunk
    // (D4), so the service planes must be named to be seen.
    let mut scopes = Vec::new();
    for class in ["telemetry", "state", "events"] {
        scopes.push(with_base(base, format!("v1/*/{class}/**")));
    }
    for slice in slices.slices() {
        if let Some(origin) = &slice.service_origin {
            for class in ["telemetry", "state", "events"] {
                scopes.push(with_base(base, format!("v1/{origin}/{class}/**")));
            }
        }
    }

    let monitor = crate::Monitor::start(session, crate::MonitorSpec::default()).await?;
    let mut events = monitor.events();
    for scope in &scopes {
        monitor.watch(scope).await?;
    }
    let deadline = tokio::time::Instant::now() + window;

    let mut samples: u64 = 0;
    let mut dropped: u64 = 0;
    let mut synthetic: u64 = 0;
    let mut facts_cache: BTreeMap<String, crate::facts::KeyFacts> = BTreeMap::new();
    let mut decode_budget: BTreeMap<String, u8> = BTreeMap::new();
    // Per-key aggregates: key → count (+ what was wrong, first occurrence).
    let mut unregistered: BTreeMap<String, u64> = BTreeMap::new();
    let mut qos_bad: BTreeMap<String, (String, u64, u64)> = BTreeMap::new();
    let mut undecodable: BTreeMap<String, (String, u64)> = BTreeMap::new();
    let mut invalid: BTreeMap<String, (String, u64)> = BTreeMap::new();
    // Per-family event counts: (family subject, declared rate) → count.
    let mut event_counts: BTreeMap<(String, String), u64> = BTreeMap::new();

    loop {
        let item = tokio::select! {
            item = events.recv() => item,
            _ = tokio::time::sleep_until(deadline) => break,
        };
        match item {
            Some(crate::StreamItem::Event(crate::FleetEvent::Sample(s))) => {
                samples += 1;
                if let Some(att) = &s.attachment
                    && is_synthetic_marker(&att.to_bytes())
                {
                    synthetic += 1;
                }
                let facts = facts_cache.entry(s.key.clone()).or_insert_with(|| {
                    let mut f = crate::facts::KeyFacts::project(base, &s.key);
                    f.resolve(slices);
                    f
                });
                match &facts.registration {
                    crate::facts::Registration::Unregistered => {
                        *unregistered.entry(s.key.clone()).or_default() += 1;
                    }
                    crate::facts::Registration::Registered(sf) => {
                        if let (Some(profile), Some(declared)) = (sf.declared_qos(), &sf.qos) {
                            let entry = qos_bad
                                .entry(s.key.clone())
                                .or_insert_with(|| (declared.clone(), 0, 0));
                            entry.2 += 1;
                            if !s.qos_matches(profile) {
                                entry.1 += 1;
                            }
                        }
                        if let (Some(rate), crate::facts::KeyShape::V1(v)) =
                            (&sf.rate, &facts.shape)
                            && v.class == "events"
                        {
                            let family = match &v.producer {
                                Some(p) => format!("{p}/{}", sf.path),
                                None => format!("{}/{}", v.origin, sf.path),
                            };
                            *event_counts.entry((family, rate.clone())).or_default() += 1;
                        }
                        let budget = decode_budget.entry(s.key.clone()).or_default();
                        if *budget < DECODE_BUDGET {
                            *budget += 1;
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
                                crate::Verdict::NotValidated(
                                    zenkey::schema::validate::NotValidated::Undecodable,
                                ) => {
                                    let e = undecodable.entry(s.key.clone()).or_insert_with(|| {
                                        (
                                            d.decode_error
                                                .unwrap_or_else(|| "does not decode".into()),
                                            0,
                                        )
                                    });
                                    e.1 += 1;
                                }
                                crate::Verdict::Invalid(errors) => {
                                    let e = invalid
                                        .entry(s.key.clone())
                                        .or_insert_with(|| (errors.join("; "), 0));
                                    e.1 += 1;
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
            Some(crate::StreamItem::Dropped(n)) => dropped += n,
            Some(_) => continue,
            None => break,
        }
    }
    let keys_seen = facts_cache.len();
    monitor.stop();

    let window_s = window.as_secs_f64();
    let mut findings = Vec::new();
    let capped = |findings: &mut Vec<DoctorFinding>, check: &str, total: usize, shown: usize| {
        if total > shown {
            findings.push(finding(
                DoctorSeverity::Info,
                check,
                "fleet",
                format!("… and {} more key(s) with the same finding", total - shown),
                None,
            ));
        }
    };

    for (key, (error, n)) in undecodable.iter().take(FINDING_CAP) {
        findings.push(finding(
            DoctorSeverity::Error,
            "payload-undecodable",
            key.clone(),
            format!("payload does not decode as its declared type: {error} ({n} sample(s) tried)"),
            Some("RFC 08 §7"),
        ));
    }
    capped(
        &mut findings,
        "payload-undecodable",
        undecodable.len(),
        FINDING_CAP,
    );
    for (key, (violations, n)) in invalid.iter().take(FINDING_CAP) {
        findings.push(finding(
            DoctorSeverity::Error,
            "payload-invalid",
            key.clone(),
            format!("payload violates the served schema: {violations} ({n} sample(s) tried)"),
            Some("RFC 08 §7"),
        ));
    }
    capped(&mut findings, "payload-invalid", invalid.len(), FINDING_CAP);
    for (key, (declared, bad, total)) in qos_bad.iter().take(FINDING_CAP) {
        if *bad > 0 {
            findings.push(finding(
                DoctorSeverity::Warning,
                "qos-observed-mismatch",
                key.clone(),
                format!(
                    "{bad} of {total} sample(s) did not ride the declared {declared} — this \
                     is what actually rode: an interceptor MAY rewrite QoS, so it is a \
                     deviation, not proof of the publisher"
                ),
                Some("RFC 04 §3"),
            ));
        }
    }
    capped(
        &mut findings,
        "qos-observed-mismatch",
        qos_bad.values().filter(|(_, bad, _)| *bad > 0).count(),
        FINDING_CAP.min(qos_bad.len()),
    );
    for (key, n) in unregistered.iter().take(FINDING_CAP) {
        findings.push(finding(
            DoctorSeverity::Warning,
            "unregistered-traffic",
            key.clone(),
            format!(
                "{n} sample(s) on a subject the producer's slice does not declare — \
                 for a conforming producer, a subject that is not registered does not exist"
            ),
            Some("RFC 08 §2"),
        ));
    }
    capped(
        &mut findings,
        "unregistered-traffic",
        unregistered.len(),
        FINDING_CAP,
    );
    // Over-rate only, and only when provable: within any window no longer
    // than an hour, exceeding the hourly cap is conclusive. Absence or
    // under-rate in a bounded window is never a finding (O1/O4).
    if window <= Duration::from_secs(3600) {
        for ((family, rate), count) in &event_counts {
            let Some(cap) = rate_cap_per_hour(rate) else {
                continue;
            };
            if *count > cap {
                findings.push(finding(
                    DoctorSeverity::Warning,
                    "rate-over-declared",
                    family.clone(),
                    format!(
                        "{count} event(s) in {window_s:.0}s exceeds the declared \
                         `{rate}` cap ({cap}/h)"
                    ),
                    Some("RFC 04 §1.3"),
                ));
            }
        }
    }

    Ok((
        findings,
        crate::report::ObservationSummary {
            window_s,
            scopes,
            samples,
            keys_seen,
            dropped,
            synthetic_marked: synthetic,
        },
    ))
}

/// Judge one state family's samples against its declared ttl — pure, so the
/// freshness math is testable without a bus. Returns the stale findings and
/// the count of unstamped samples (aggregated by the caller into the one
/// `unstamped-state` finding).
fn judge_state_samples(
    samples: &[crate::StateSample],
    ttl: i64,
    now: std::time::SystemTime,
) -> (Vec<DoctorFinding>, usize) {
    let mut findings = Vec::new();
    let mut unstamped = 0usize;
    for sample in samples {
        match sample.timestamp {
            Some(ts) => {
                let stamped = ts.get_time().to_system_time();
                if let Ok(age) = now.duration_since(stamped)
                    && age.as_secs() as i64 > ttl
                {
                    findings.push(finding(
                        DoctorSeverity::Error,
                        "stale-state",
                        sample.key.clone(),
                        format!(
                            "{}s old against ttl {ttl}s (refresh <= ttl/2)",
                            age.as_secs()
                        ),
                        Some("RFC 04 §1.2"),
                    ));
                }
            }
            None => unstamped += 1,
        }
    }
    (findings, unstamped)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The id vocabulary is API: additions append, nothing renames. If this
    /// test fails you are renaming a shipped check id — don't.
    #[test]
    fn check_ids_are_stable() {
        assert_eq!(
            CHECK_IDS,
            [
                "slice-parse",
                "slice-sync",
                "introspect-coverage",
                "admin-unreachable",
                "router-version-skew",
                "describe-totality",
                "schema-drift",
                "describe-missing",
                "stale-state",
                "unstamped-state",
                "storage-coverage",
                "payload-undecodable",
                "payload-invalid",
                "qos-observed-mismatch",
                "unregistered-traffic",
                "rate-over-declared",
            ]
        );
    }

    /// RFC 04 §1.3's closed vocabulary, as hourly caps — and everything
    /// else says "cannot judge", never a guessed budget.
    #[test]
    fn rate_caps_follow_the_declared_classes() {
        assert_eq!(rate_cap_per_hour("rare"), Some(1));
        assert_eq!(rate_cap_per_hour("low"), Some(60));
        assert_eq!(rate_cap_per_hour("burst(100/h)"), Some(100));
        assert_eq!(rate_cap_per_hour("burst(100)"), None);
        assert_eq!(rate_cap_per_hour("often"), None);
    }

    /// #162's marker as #161 reads it: a JSON object with `"synthetic": true`.
    /// Anything else — other attachments, non-JSON bytes — is real traffic.
    #[test]
    fn the_synthetic_marker_is_recognised_and_nothing_else_is() {
        assert!(is_synthetic_marker(
            br#"{"synthetic":true,"tool":"zenctl gen"}"#
        ));
        assert!(!is_synthetic_marker(br#"{"synthetic":false}"#));
        assert!(!is_synthetic_marker(br#"{"tool":"zenctl gen"}"#));
        assert!(!is_synthetic_marker(b"meta"));
        assert!(!is_synthetic_marker(b""));
    }

    #[test]
    fn freshness_judgement_is_pure_and_ttl_bound() {
        let now = std::time::SystemTime::now();
        let fresh_ts = zenoh::time::Timestamp::new(
            zenoh::time::NTP64::from(now.duration_since(std::time::UNIX_EPOCH).unwrap()),
            zenoh::time::TimestampId::rand(),
        );
        let stale_ts = zenoh::time::Timestamp::new(
            zenoh::time::NTP64::from(
                now.duration_since(std::time::UNIX_EPOCH).unwrap() - Duration::from_secs(120),
            ),
            zenoh::time::TimestampId::rand(),
        );
        let samples = vec![
            crate::StateSample {
                key: "b/v1/h-aaaaaaaaaaaa/state/p/health".into(),
                timestamp: Some(fresh_ts),
                payload_len: 2,
            },
            crate::StateSample {
                key: "b/v1/h-bbbbbbbbbbbb/state/p/health".into(),
                timestamp: Some(stale_ts),
                payload_len: 2,
            },
            crate::StateSample {
                key: "b/v1/h-cccccccccccc/state/p/health".into(),
                timestamp: None,
                payload_len: 2,
            },
        ];
        let (findings, unstamped) = judge_state_samples(&samples, 30, now);
        assert_eq!(
            findings.len(),
            1,
            "only the stale stamped sample is a finding"
        );
        assert_eq!(findings[0].check, "stale-state");
        assert!(findings[0].subject.contains("h-bbbbbbbbbbbb"));
        assert_eq!(unstamped, 1, "the unstamped sample is counted, not judged");
    }
}
