//! `doctor` — diff what the fleet *serves* against local registry files, and
//! check the fleet against the RFC contracts it claims to follow.
//!
//! Since issue #46 every finding is a typed
//! [`DoctorFinding`](crate::report::DoctorFinding) — severity, stable check
//! id, subject, evidence, RFC citation — collected into one
//! [`DoctorReport`](crate::report::DoctorReport) and rendered as table or
//! JSON. The machine-readable shape is the one the GUI doctor panel reuses.

use anyhow::{Result, anyhow};
use zenkey::RegistrySlice;
use zenkey_fleet as bus;

use crate::report::{DoctorFinding, DoctorReport, DoctorSeverity};
use crate::{BusArgs, offline, output};

/// Every type name a slice references (subjects, procedure request/reply, and
/// since v1.8 blob `reference`s) — the RFC 08 §7 totality bound's right-hand
/// side. This list must track §7 exactly: a name §7 requires and this omits is
/// a coverage gap `doctor` would report as clean.
fn referenced_type_names(slice: &RegistrySlice) -> Vec<String> {
    let mut names: Vec<String> = slice
        .subjects
        .iter()
        .map(|s| s.type_name.clone())
        .filter(|t| !t.is_empty())
        .chain(slice.procedures.iter().filter_map(|p| p.request.clone()))
        .chain(slice.procedures.iter().filter_map(|p| p.reply.clone()))
        .chain(slice.blob.iter().filter_map(|b| b.reference.clone()))
        .collect();
    names.sort_unstable();
    names.dedup();
    names
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

pub async fn run(deep: bool, args: &BusArgs) -> Result<()> {
    let session = args.session().await?;
    let roster = bus::roster(&session, args.base(), args.timeout()).await?;

    let mut findings: Vec<DoctorFinding> = Vec::new();
    let mut synced: Vec<String> = Vec::new();
    let mut answered = 0usize;

    if args.registry.is_empty() {
        eprintln!(
            "note: no --registry <dir> given — skipping the served-vs-declared diff; only the \
             roster-vs-introspect check runs."
        );
    }
    let locals = offline::load_slices(&args.registry)?;

    for local in &locals {
        // A service origin's verbatim `@` chunk is structurally unmatchable by
        // the `*` of a fleet selector (property D4), so it takes its own key.
        // That is the grammar working, not an exception to it.
        let key = match &local.service_origin {
            Some(origin) => {
                let o = zenkey::ServiceOrigin::new(origin).map_err(|e| {
                    anyhow!("bad service origin in local slice {}: {e}", local.name)
                })?;
                args.wire(zenkey::selector::service_rpc(&o, &["introspect"]))?
            }
            None => args.wire(zenkey::selector::fleet_rpc(&local.name, &["introspect"]))?,
        };

        let answers = bus::fleet_get(&session, args.base(), &key, None, args.timeout()).await?;

        for answer in &answers {
            let bus::Answer::Value(bytes) = &answer.answer else {
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

    // With no local registry the only introspect coverage we can count is the
    // fleet-wide wildcard.
    if locals.is_empty() {
        let pairs = bus::fleet_registry(&session, args.base(), args.timeout()).await?;
        answered = pairs.len();
    }

    // The roster is what makes silence legible (RFC 05 §3.1): a producer that
    // holds an `alive` token but did not answer `introspect` is a bug, because
    // producers MUST declare their @rpc queryables *before* their token —
    // "alive ⇒ callable" (RFC 04 §5).
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

    // --- Admin reachability (issue #14) -------------------------------
    let routers = zenkey_fleet::routers(&session, args.timeout())
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

    // --- Schema conformance (RFC 08 §7, issue #14) --------------------
    // Which slices to judge: the locals when given, else what the fleet
    // serves.
    let schema_slices: Vec<RegistrySlice> = if locals.is_empty() {
        bus::fleet_registry(&session, args.base(), args.timeout())
            .await?
            .into_iter()
            .map(|(_, s)| s)
            .collect()
    } else {
        locals.clone()
    };
    let mut described: Vec<(String, zenkey::schema::SchemaSet)> = Vec::new();
    let mut undescribed = 0usize;
    for slice in &schema_slices {
        let key = match &slice.service_origin {
            Some(origin) => {
                let o = zenkey::ServiceOrigin::new(origin)
                    .map_err(|e| anyhow!("bad service origin in slice {}: {e}", slice.name))?;
                args.wire(zenkey::selector::service_rpc(&o, &["describe"]))?
            }
            None => args.wire(zenkey::selector::fleet_rpc(&slice.name, &["describe"]))?,
        };
        let answers = bus::fleet_get(&session, args.base(), &key, None, args.timeout()).await?;
        let set = answers.into_iter().find_map(|a| match a.answer {
            bus::Answer::Value(bytes) => {
                let cow = bytes.to_bytes();
                std::str::from_utf8(&cow)
                    .ok()
                    .and_then(|t| zenkey::schema::SchemaSet::parse(t).ok())
            }
            bus::Answer::Error { .. } => None,
        });
        match set {
            Some(set) => {
                let names = referenced_type_names(slice);
                let refs: Vec<&str> = names.iter().map(String::as_str).collect();
                if let Err(e) = set.verify_covers(&refs) {
                    findings.push(finding(
                        DoctorSeverity::Error,
                        "describe-totality",
                        slice.name.clone(),
                        format!("describe is not total: {e}"),
                        Some("RFC 08 §7"),
                    ));
                }
                described.push((slice.name.clone(), set));
            }
            None => undescribed += 1,
        }
    }
    for drift in zenkey_fleet::schema_drift(&described) {
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

    // --- --deep: freshness + storage coverage -------------------------
    if deep {
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
                    Some(origin) => {
                        args.wire(format!("v1/{origin}/state/{}", pattern.selector_tail()))?
                    }
                    None => args.wire(format!(
                        "v1/*/state/{}/{}",
                        slice.name,
                        pattern.selector_tail()
                    ))?,
                };
                for sample in bus::state_snapshot(&session, &selector, args.timeout()).await? {
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
        let storages = zenkey_fleet::storages(&session, args.timeout())
            .await
            .unwrap_or_default();
        let coverage = zenkey_fleet::state_coverage(
            &zenkey_fleet::SliceSet::from_slices(schema_slices.clone()),
            args.base(),
            &storages,
        );
        let uncovered: Vec<&zenkey_fleet::CoverageRow> = coverage
            .iter()
            .filter(|r| r.coverage == zenkey_fleet::Coverage::Uncovered)
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

    let report = DoctorReport {
        findings,
        synced,
        introspect_answered: answered,
        live_producers: live,
        describe_served: described.len(),
        describe_missing: undescribed,
        routers: routers.len(),
        router_version,
        deep,
    };
    output::doctor(&report, args.format)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn referenced_type_names_are_total_and_deduped() {
        let slice = zenkey::parse_slice(
            r#"
            [registry]
            version = "1.0"
            app = "t"
            convention = 1
            [producer]
            name = "tc"
            [[subject]]
            path = "health"
            class = "state"
            type = "Health"
            [[procedure]]
            path = "x/set"
            kind = "write"
            request = "Cmd"
            reply = "Health"
            "#,
        )
        .unwrap();
        assert_eq!(referenced_type_names(&slice), vec!["Cmd", "Health"]);
    }
}
