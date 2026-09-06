//! The doctor checks as engine functions (#55): every finding both frontends
//! render comes from here — `zenctl doctor` orchestrates and renders, the
//! zengui doctor panel calls the same [`run_doctor`] and renders the same
//! [`DoctorReport`]. A check that lives in one frontend is a check the other
//! frontend's user never sees (RFC 08 §6.1's argument, applied to ourselves).
//!
//! Check ids are **stable API**: scripts key on them (`--format json`), the
//! GUI keys deltas on them. New checks add ids; nothing renames one. The full
//! set is pinned in [`crate::report::CheckId`].

use std::time::Duration;

use crate::Result;
use zenkey::grammar::with_base;
use zenkey::{Declared, RegistrySlice};

use crate::bus::query::{Answer, GetOpts, RepeatingRegistry, fleet_get, state_snapshot};
use crate::judge::common::{FINDING_CAP, is_synthetic_marker};
use crate::model::examples::Examples;
use crate::report::{CheckId, DoctorFinding, DoctorReport, DoctorSeverity, DriftVerdict};

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
    /// fan-in (`--for`, #161) and judge what rides: decode/validity,
    /// declared-vs-observed QoS, unregistered traffic, over-rate events.
    /// `None` = the phase does not run and the report carries no
    /// observation section.
    pub listen: Option<Duration>,
}

fn finding(
    severity: DoctorSeverity,
    check: CheckId,
    subject: impl Into<String>,
    evidence: impl Into<String>,
    citation: Option<&str>,
) -> DoctorFinding {
    DoctorFinding {
        severity,
        check,
        subject: subject.into(),
        evidence: evidence.into(),
        citation: citation.map(str::to_string),
    }
}

/// Run every check against the live fleet and report typed findings.
///
/// `locals` is the caller's registry (loaded from `--registry` dirs or GUI
/// settings). `None` means none was loaded: the served-vs-declared diff is
/// skipped and only bus-derived checks run, and the report says so rather
/// than reading in sync (O4 — "not asked" must not render as "clean").
///
/// `Option<&SliceSet>` and not `&[RegistrySlice]`: this is the engine's
/// standing shape for "a registry, or honestly none" (`facts.rs` states it as
/// policy), an empty slice could not tell the two apart, and the set arrives
/// already indexed — doctor used to rebuild one from a clone of every slice
/// halfway through the run.
pub async fn run_doctor(
    fleet: &crate::Fleet<'_>,
    locals: Option<&crate::model::registry::SliceSet>,
    spec: &DoctorSpec,
) -> Result<DoctorReport> {
    let (session, base) = (fleet.session(), fleet.base());

    // A registry that declares nothing answers no question this run asks, so
    // it takes the same path as none at all — normalised once, here, rather
    // than at each of the four places that branch on it below.
    let locals = locals.filter(|set| !set.slices().is_empty());
    let roster = crate::bus::roster::roster(fleet, spec.timeout).await?;

    let mut findings: Vec<DoctorFinding> = Vec::new();
    let mut synced: Vec<String> = Vec::new();
    let mut answered = 0usize;

    // --- served-vs-declared diff (RFC 08 §6) --------------------------
    for local in locals.iter().flat_map(|set| set.slices()) {
        let key = crate::model::registry::rpc_key(base, local, "introspect")?;
        let answers = fleet_get(fleet, &key, &GetOpts::new(spec.timeout)).await?;
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
                        CheckId::SliceParse,
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
                        CheckId::SliceSync,
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
    let sweep = if locals.is_none() {
        let repeating = RepeatingRegistry::declare(fleet, spec.timeout).await?;
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
    // token — "alive ⇒ callable" (RFC 04 §5). Coverage is judged over the
    // producers that were actually *asked*: with `--registry` covering a
    // subset, a live producer outside the locals was never queried, and
    // "not asked" must not render as "did not answer" (RFC 09 §5.1 O4).
    let live: usize = roster.values().map(Vec::len).sum();
    findings.extend(judge_introspect_coverage(
        &roster,
        locals.map(crate::model::registry::SliceSet::slices),
        answered,
    ));

    // --- admin reachability ------------------------------------------
    let routers = crate::routers(session, spec.timeout)
        .await
        .unwrap_or_default();
    let mut router_version = None;
    if routers.is_empty() {
        findings.push(finding(
            DoctorSeverity::Info,
            CheckId::AdminUnreachable,
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
                CheckId::RouterVersionSkew,
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
    let slice_set: std::borrow::Cow<'_, crate::model::registry::SliceSet> = match sweep {
        Some(slices) => {
            std::borrow::Cow::Owned(crate::model::registry::SliceSet::from_slices(slices))
        }
        // The caller's set is already indexed; rebuilding it here reparsed
        // every subject pattern to arrive at the set we were handed.
        None => match locals {
            Some(set) => std::borrow::Cow::Borrowed(set),
            None => std::borrow::Cow::Owned(crate::model::registry::SliceSet::default()),
        },
    };
    // One sweep, kept whole (#410): every answer attributed to the host that
    // gave it, because `describe` fans in across every host running a
    // producer and keeping one of them was how a schema disagreement came
    // to name a producer and never a host (#398). The same helper serves
    // `interface show --schema`, so the two no longer each hold a copy of
    // "do these carriers agree".
    let describes = crate::bus::describe::describe_sweep(fleet, &slice_set, spec.timeout).await?;
    // One per producer, for the consumers whose question *is* the producer:
    // totality, the listen phase's store, the served count, and the field
    // table's declared-path join. Where several hosts answered this is the
    // first of them — arrival order, which is not a fact about the fleet, and
    // is why the drift check below reads the attributed list instead (#398).
    let described: Vec<(String, zenkey::schema::SchemaSet)> = describes.first_per_producer();
    let undescribed = describes.undescribed.len();
    // Totality through the one engine implementation (`totality_gaps`) —
    // doctor used to carry a parallel referenced-names path.
    for gap in crate::model::decode::totality_gaps(&described, &slice_set) {
        findings.push(finding(
            DoctorSeverity::Error,
            CheckId::DescribeTotality,
            gap.producer.clone(),
            format!(
                "describe is not total — missing: {}",
                gap.missing.join(", ")
            ),
            Some("RFC 08 §7"),
        ));
    }
    for drift in crate::model::decode::schema_drift(&describes.answers) {
        let servers: Vec<String> = drift
            .servers
            .iter()
            // `producer@origin`, because a type with two identities and no
            // host to go and look at is the finding you can do least with
            // (#398).
            .map(|s| match s.hash.as_option() {
                Some(h) => format!("{}@{} ({h})", s.producer, s.origin),
                None => format!("{}@{} (no identity served)", s.producer, s.origin),
            })
            .collect();
        // The two verdicts are not the same finding. A disagreement is a
        // defect; a producer that served no identity leaves the question
        // *unanswered*, and calling that an error would be the mirror of the
        // bug #370 fixed — reporting a verdict nobody's evidence supports.
        let (severity, evidence) = match drift.verdict {
            DriftVerdict::Disagree => (
                DoctorSeverity::Error,
                format!("served with different schemas by {}", servers.join(", ")),
            ),
            DriftVerdict::Unjudgeable => (
                DoctorSeverity::Warning,
                format!(
                    "agreement cannot be judged — {} served no schema identity: {} \
                     (RFC 09 §5.1 O4; the hash exists for exactly this, RFC 08 §7)",
                    drift
                        .servers
                        .iter()
                        .filter(|s| s.hash.is_not_asked())
                        .count(),
                    servers.join(", ")
                ),
            ),
        };
        findings.push(finding(
            severity,
            CheckId::SchemaDrift,
            drift.type_name.clone(),
            evidence,
            Some("RFC 08 §7"),
        ));
    }
    if undescribed > 0 {
        findings.push(finding(
            DoctorSeverity::Info,
            CheckId::DescribeMissing,
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
        for slice in slice_set.slices() {
            for subject in &slice.subjects {
                let (Some(ttl), true) = (subject.ttl_s, subject.class.is(&zenkey::Class::State))
                else {
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
                CheckId::UnstampedState,
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
                CheckId::StorageCoverage,
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
            let store = crate::model::decode::SchemaStore::new(base, spec.timeout);
            // The GET phase above already asked every producer for its
            // `describe` document. Hand those to the window's store rather
            // than letting it re-ask the fleet, mid-window, for what this
            // run is holding (RFC 08 §7; the store's frugality note).
            for (producer, set) in &described {
                store.insert(producer, set.clone());
            }
            // And sealed for the window (#337): the GET phase asked every
            // producer the registry names, so a miss inside the window is a
            // producer that served nothing — already counted as
            // `describe_missing`. Left unsealed, that miss is a `describe`
            // GET awaited inside the drain loop, re-asked every time its
            // backoff expires, with nobody attending the broadcast.
            let _sealed = store.seal();
            let (listen_findings, summary) =
                observe_traffic(fleet, &slice_set, &store, &described, window).await?;
            findings.extend(listen_findings);
            Some(summary)
        }
        None => None,
    };

    Ok(DoctorReport {
        findings,
        // `None` when no local registry was given: the served-vs-declared
        // diff never ran, and the report must say so rather than looking
        // like "ran, none in sync" (RFC 09 §5.1 O4, review finding R1).
        synced: locals.is_some().then_some(synced).into(),
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

/// The remainder wording every per-key listen check shares.
const SAME_FINDING: &str = "more key(s) with the same finding";

/// Spill a capped collector into `findings`, followed by the remainder note
/// when the cap bit.
///
/// Filter and judge **into** the collector, never around it (deep-review D4):
/// the `qos-observed-mismatch` cap used to bound the judged *keys*, so
/// violators past the first [`FINDING_CAP`] of them vanished and the
/// remainder note under-counted. [`Examples`] counts what it is offered, so
/// the note cannot disagree with the population it summarises.
fn emit_capped(
    findings: &mut Vec<DoctorFinding>,
    ex: Examples<DoctorFinding>,
    check: CheckId,
    tail: &str,
) {
    let more = ex.more(tail);

    findings.extend(ex.into_vec());

    if let Some(evidence) = more {
        findings.push(finding(
            DoctorSeverity::Info,
            check,
            "fleet",
            evidence,
            None,
        ));
    }
}

/// The declared events rate class as an hourly cap (RFC 04 §1.3):
/// `rare` ≤ 1/h, `low` ≤ 1/min, `burst(n/h)` a declared cap.
struct RateWindow {
    /// The declared hourly cap, from [`RateClass::cap_per_hour`]. `None` is
    /// a rate token this build cannot read — "cannot judge", never a
    /// guessed budget (RFC 09 §5.1 O4).
    cap: Option<u64>,
    /// Samples seen on the family during the window.
    seen: u64,
}

/// The passive listening phase: watch the data planes for `window`, judge
/// each sample through the ladders that already exist — the Registration
/// ladder, `qos_matches`, `decode_sample` — and aggregate per key so a hot
/// key is one finding with a count, not a finding per sample.
async fn observe_traffic(
    fleet: &crate::Fleet<'_>,
    slices: &crate::model::registry::SliceSet,
    store: &crate::model::decode::SchemaStore,
    described: &[(String, zenkey::schema::SchemaSet)],
    window: Duration,
) -> Result<(Vec<DoctorFinding>, crate::report::ObservationSummary)> {
    use std::collections::{BTreeMap, BTreeSet};

    let (session, base) = (fleet.session(), fleet.base());

    // Scope statement (O5): the three data classes for host origins, plus
    // each declared service origin's three — `*` never matches an `@` chunk
    // (D4), so the service planes must be named to be seen. Shared with the
    // `topic list --budget` observation (#221).
    let scopes = crate::judge::common::data_plane_scopes(base, slices);

    // The liveliness planes ride too (#422): a `counter` may reset across
    // its producer's restart, and RFC 08 §2 makes the `alive` token cycling
    // the one sanctioned reset — so the window has to see the cycle to
    // excuse the drop. Two selectors, never one: `*` cannot reach a verbatim
    // service origin (RFC 03 §4 D4), so `@catalog` and every service origin
    // the slices declare are named. Zero payload by construction (RFC 04 §5).
    let mut liveliness = vec![
        fleet.wire(zenkey::selector::all_liveliness(
            zenkey::selector::Scope::fleet(),
        )),
        fleet.wire(zenkey::selector::service_alive(
            &zenkey::ServiceOrigin::catalog(),
        )),
    ];
    for slice in slices.slices() {
        if let Some(origin) = slice.service_origin.as_ref().and_then(|o| o.known())
            && *origin != zenkey::ServiceOrigin::catalog()
        {
            liveliness.push(fleet.wire(zenkey::selector::service_alive(origin)));
        }
    }
    let monitor = crate::Monitor::start(
        session,
        crate::MonitorSpec {
            liveliness,
            ..Default::default()
        },
    )
    .await?;
    let mut events = monitor.events();
    // A scope that fails to declare tears the monitor down on the way out,
    // rather than leaving a `**` subscriber to `Drop` (#336).
    let monitor = monitor.watching(&scopes).await?;
    let started = tokio::time::Instant::now();
    let deadline = started + window;

    // Field intelligence (#223): per-dotted-path stats over the structural
    // value — sync and schema-free, so it rides every sample within the
    // decode budget's reach and beyond.
    let mut fields =
        crate::judge::field::FieldObservation::new(crate::judge::field::DEFAULT_MAX_PATHS);
    let mut samples: u64 = 0;
    let mut dropped: u64 = 0;
    let mut synthetic: u64 = 0;
    // Bounded (#107): one projection per distinct key, LRU past the bound,
    // evictions counted into the observation summary (O6).
    let mut facts_cache = crate::model::facts::FactsCache::default();
    let mut decode_budget: BTreeMap<String, u8> = BTreeMap::new();
    // Per-key aggregates: key → count (+ what was wrong, first occurrence).
    let mut unregistered: BTreeMap<String, u64> = BTreeMap::new();
    let mut qos_bad: BTreeMap<String, (String, u64, u64)> = BTreeMap::new();
    let mut undecodable: BTreeMap<String, (String, u64)> = BTreeMap::new();
    let mut invalid: BTreeMap<String, (String, u64)> = BTreeMap::new();
    // Per-family event counts: (family subject, declared rate) → what was
    // seen and what was declared.
    let mut event_counts: BTreeMap<(String, String), RateWindow> = BTreeMap::new();
    // Stamping nodes that are not the publisher (#213): zid → samples.
    let mut foreign_stampers: BTreeMap<String, u64> = BTreeMap::new();
    // Declared versus observed `kind` (#422): per key, with the producers
    // whose `alive` token is currently down — a `NodeUp` that follows one
    // is a cycle; a lone `NodeUp` (history replaying the tokens that are
    // simply alive at window start) is not.
    let mut kinds = crate::judge::kind::KindObservation::new();
    let mut alive_down: BTreeSet<(String, String)> = BTreeSet::new();

    // One timer for the whole window, not one per iteration (#346).
    // `sleep_until` builds a future and registers a timer each time it
    // is evaluated, and a `select!` in a loop evaluates it on every
    // pass — at 100k samples/s that is 100k registrations a second for
    // a deadline that never moves.
    let window_over = tokio::time::sleep_until(deadline);
    tokio::pin!(window_over);
    loop {
        let item = tokio::select! {
            item = events.recv() => item,
            () = &mut window_over => break,
        };
        match item {
            Some(crate::StreamItem::Event(crate::FleetEvent::Sample(s))) => {
                samples += 1;
                if let Some(att) = &s.attachment
                    && is_synthetic_marker(&att.to_bytes())
                {
                    synthetic += 1;
                }
                // Who stamped it (#213). A router doing the timestamping is
                // not a fault — it is a deployment choice — but it silently
                // changes what every latency in this suite measures, so it is
                // worth saying out loud once.
                if let Some(crate::StampProvenance::Foreign { stamper }) = s.stamped_by {
                    *foreign_stampers.entry(stamper.to_string()).or_default() += 1;
                }
                // A tombstone is a retirement, not a document (RFC 04 §1.2):
                // a Delete carries no payload to read, decode, or validate,
                // so the field and payload ladders skip it — judging the
                // empty body as a value manufactures `payload-undecodable`
                // out of a correct retirement (zensight#830). Everything
                // that is a wire fact about the publisher — QoS axes,
                // registration, stamping — still applies and stays judged.
                let is_put = s.kind == zenoh::sample::SampleKind::Put;
                // Same bound as `run_field`'s drain (#346): the parse is
                // per sample by design, so the payload size is what has to be
                // bounded, and the skip is counted rather than read as an
                // absent document.
                let bytes = s.payload.to_bytes();
                // The structural document, read once for the field ladder
                // and the kind judge alike; `None` for an oversized or
                // undecodable body, which each of them counts as unread.
                let mut doc = None;
                if is_put {
                    if bytes.len() > crate::model::decode::OBSERVE_LIMIT {
                        fields.observe_unread(&s.key);
                    } else {
                        doc = crate::model::decode::structural_value(&bytes);
                        fields.observe(&s.key, started.elapsed().as_secs_f64(), doc.as_ref());
                    }
                }
                facts_cache.ensure(base, &s.key, Some(slices));
                let facts = facts_cache.get(&s.key).expect("just ensured this key");
                match &facts.registration {
                    crate::model::facts::Registration::Unregistered => {
                        *unregistered.entry(s.key.clone()).or_default() += 1;
                    }
                    crate::model::facts::Registration::Registered(sf) => {
                        if let (Some(profile), Some(declared)) = (sf.declared_qos(), &sf.qos) {
                            let entry = qos_bad
                                .entry(s.key.clone())
                                .or_insert_with(|| (declared.token().to_string(), 0, 0));
                            entry.2 += 1;
                            if !s.qos_matches(profile) {
                                entry.1 += 1;
                            }
                        }
                        if let (Some(rate), crate::model::facts::KeyShape::V1(v)) =
                            (&sf.rate, &facts.shape)
                            && v.class == "events"
                        {
                            let family = match &v.producer {
                                Some(p) => format!("{p}/{}", sf.path),
                                None => format!("{}/{}", v.origin, sf.path),
                            };
                            // The cap is taken from the typed `RateClass`
                            // here, where it is in hand — this used to
                            // stringify the token and re-parse it below
                            // through a second copy of RFC 04 §1.3's
                            // mapping (#350's sweep).
                            event_counts
                                .entry((family, rate.token()))
                                .or_insert_with(|| RateWindow {
                                    cap: rate.cap_per_hour(),
                                    seen: 0,
                                })
                                .seen += 1;
                        }
                        // Declared `kind` (#422, RFC 08 §2): judged only where
                        // the entry declares one this build knows — absent
                        // or foreign is not asked (RFC 13 §3).
                        if let (Some(declared), crate::model::facts::KeyShape::V1(v)) =
                            (sf.kind.as_ref().and_then(|k| k.known()), &facts.shape)
                            && is_put
                        {
                            // The producer half of the identity spelled the
                            // way `token_identity` reads a liveliness token,
                            // so a cycle lands on the keys it restarted.
                            let producer = v
                                .producer
                                .clone()
                                .unwrap_or_else(|| v.origin.trim_start_matches('@').to_string());
                            kinds.observe(&s.key, &v.origin, &producer, *declared, doc.as_ref());
                        }
                        let budget = decode_budget.entry(s.key.clone()).or_default();
                        if is_put && *budget < DECODE_BUDGET {
                            *budget += 1;
                            // `Some`: the doctor's slice set comes from its
                            // own live introspect sweep, so the registry was
                            // always asked here — `NoRegistry` (#246) cannot
                            // arise, and like every not-validated reason
                            // other than `Undecodable` it would fall through
                            // the `_` arm below: not asked/not checkable is
                            // never a finding (RFC 09 §5.1 O4).
                            let d = crate::model::decode::decode_sample(
                                fleet,
                                store,
                                Some(slices),
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
            Some(crate::StreamItem::Event(crate::FleetEvent::NodeDown(key))) => {
                if let Some(id) = crate::bus::roster::token_identity(base, &key) {
                    alive_down.insert(id);
                }
            }
            Some(crate::StreamItem::Event(crate::FleetEvent::NodeUp(key))) => {
                // A cycle is down *then* up. History replays the tokens
                // alive at window start as bare `NodeUp`s; those excuse
                // nothing.
                if let Some((origin, producer)) = crate::bus::roster::token_identity(base, &key)
                    && alive_down.remove(&(origin.clone(), producer.clone()))
                {
                    kinds.alive_cycled(&origin, &producer);
                }
            }
            Some(_) => continue,
            None => break,
        }
    }
    let keys_seen = facts_cache.len();
    monitor.shutdown().await?;

    // Key-population budgets (#221): the window's distinct keys, grouped
    // into `{var}` families per origin, judged against each family's
    // declared `cardinality`.
    let budgets =
        crate::judge::budget::BudgetObservation::observe(base, slices, facts_cache.keys());

    let window_s = window.as_secs_f64();
    let mut findings = Vec::new();

    let mut ex = Examples::new(FINDING_CAP);
    for (key, (error, n)) in &undecodable {
        ex.push_with(|| {
            finding(
                DoctorSeverity::Error,
                CheckId::PayloadUndecodable,
                key.clone(),
                format!(
                    "payload does not decode as its declared type: {error} ({n} sample(s) tried)"
                ),
                Some("RFC 08 §7"),
            )
        });
    }
    emit_capped(&mut findings, ex, CheckId::PayloadUndecodable, SAME_FINDING);
    let mut ex = Examples::new(FINDING_CAP);
    for (key, (violations, n)) in &invalid {
        ex.push_with(|| {
            finding(
                DoctorSeverity::Error,
                CheckId::PayloadInvalid,
                key.clone(),
                format!("payload violates the served schema: {violations} ({n} sample(s) tried)"),
                Some("RFC 08 §7"),
            )
        });
    }
    emit_capped(&mut findings, ex, CheckId::PayloadInvalid, SAME_FINDING);
    findings.extend(judge_qos_observed(&qos_bad));
    if !foreign_stampers.is_empty() {
        let mut named: Vec<String> = foreign_stampers
            .iter()
            .map(|(zid, n)| format!("{zid} ({n} sample(s))"))
            .collect();
        named.sort();
        findings.push(finding(
            DoctorSeverity::Info,
            CheckId::TimestampStampedElsewhere,
            "fleet".to_string(),
            format!(
                "HLCs on this bus are stamped by {} node(s) that are not the publishing \
                 session — a deployment with router-side timestamping, which is legal and \
                 common. Latency measured from these stamps is stamper→observer, not \
                 publisher→observer: {}",
                foreign_stampers.len(),
                named.join(", ")
            ),
            Some("RFC 09 §5.1 O7"),
        ));
    }
    let mut ex = Examples::new(FINDING_CAP);
    for (key, n) in &unregistered {
        ex.push_with(|| {
            finding(
                DoctorSeverity::Warning,
                CheckId::UnregisteredTraffic,
                key.clone(),
                format!(
                    "{n} sample(s) on a subject the producer's slice does not declare — \
                     for a conforming producer, a subject that is not registered does not exist"
                ),
                Some("RFC 08 §2"),
            )
        });
    }
    emit_capped(
        &mut findings,
        ex,
        CheckId::UnregisteredTraffic,
        SAME_FINDING,
    );
    // Over-rate only, and only when provable: within any window no longer
    // than an hour, exceeding the hourly cap is conclusive. Absence or
    // under-rate in a bounded window is never a finding (O1/O4).
    if window <= Duration::from_secs(3600) {
        for ((family, rate), RateWindow { cap, seen: count }) in &event_counts {
            let Some(cap) = cap else {
                continue;
            };
            if count > cap {
                findings.push(finding(
                    DoctorSeverity::Warning,
                    CheckId::RateOverDeclared,
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

    findings.extend(judge_cardinality(slices, &budgets, window_s));
    findings.extend(crate::judge::kind::judge_kind(&kinds, window_s));

    // Field intelligence (#223): the three field-granular checks, judged
    // with what is known per key — declared `ttl_s`/type from the resolved
    // facts, declared paths from the describe sets the GET phase gathered.
    let field_ctx = field_context_from(slices, described, &facts_cache);
    findings.extend(crate::judge::field::judge_fields(
        &fields, window_s, &field_ctx,
    ));

    Ok((
        findings,
        crate::report::ObservationSummary {
            window_s,
            scopes,
            samples,
            keys_seen,
            dropped,
            synthetic_marked: synthetic,
            // The per-path table is bounded like every other table here, and
            // its cost is a wire fact (RFC 09 §5.1 O6).
            field_paths_dropped: fields.dropped_paths(),
            facts_evicted: facts_cache.evicted(),
        },
    ))
}

/// The per-key context the field judges need (#223), built from the listen
/// phase's resolved facts and the already-gathered describe sets — pure, so
/// the join is testable without a bus.
fn field_context_from(
    slices: &crate::model::registry::SliceSet,
    described: &[(String, zenkey::schema::SchemaSet)],
    facts: &crate::model::facts::FactsCache,
) -> std::collections::BTreeMap<String, crate::judge::field::KeyFieldContext> {
    use std::collections::BTreeMap;

    let mut declared_cache: BTreeMap<(String, String), Option<crate::judge::field::DeclaredPaths>> =
        BTreeMap::new();
    let mut ctx = BTreeMap::new();
    for (key, f) in facts.iter() {
        let mut c = crate::judge::field::KeyFieldContext::default();
        if let crate::model::facts::Registration::Registered(sf) = &f.registration {
            c.ttl_s = sf.ttl_s;
            c.type_name = Some(sf.type_name.clone());
            if let Some(producer) = crate::judge::common::producer_of(f, Some(slices))
                && !sf.type_name.is_empty()
            {
                let declared = declared_cache
                    .entry((producer.clone(), sf.type_name.clone()))
                    .or_insert_with(|| {
                        described
                            .iter()
                            .find(|(name, _)| *name == producer)
                            .and_then(|(_, set)| set.get(&sf.type_name))
                            .and_then(|schema| schema.json_document())
                            .and_then(crate::judge::field::DeclaredPaths::from_json_schema)
                    });
                c.declared = declared.clone();
            }
        }
        ctx.insert(key.to_string(), c);
    }
    ctx
}

/// The `qos-observed-mismatch` findings from the listen window's per-key
/// aggregates: `key → (declared profile, mismatched, judged)` — pure, so
/// the cap arithmetic is testable without a bus.
///
/// Filter **then** cap, [`judge_cardinality`]'s pattern (deep-review D4):
/// the map holds every judged key, most of them clean, so capping the map
/// *entries* first silently dropped violators past the first
/// [`FINDING_CAP`] keys and made the remainder note miscount. The cap
/// bounds the findings; the filter decides what a finding is.
fn judge_qos_observed(
    qos_bad: &std::collections::BTreeMap<String, (String, u64, u64)>,
) -> Vec<DoctorFinding> {
    let mut findings = Vec::new();

    let mut ex = Examples::new(FINDING_CAP);

    for (key, (declared, bad, total)) in qos_bad.iter().filter(|(_, (_, bad, _))| *bad > 0) {
        ex.push_with(|| {
            finding(
                DoctorSeverity::Warning,
                CheckId::QosObservedMismatch,
                key.clone(),
                format!(
                    "{bad} of {total} sample(s) did not ride the declared {declared} — this \
                     is what actually rode: an interceptor MAY rewrite QoS, so it is a \
                     deviation, not proof of the publisher"
                ),
                Some("RFC 04 §3"),
            )
        });
    }
    emit_capped(
        &mut findings,
        ex,
        CheckId::QosObservedMismatch,
        SAME_FINDING,
    );
    findings
}

/// Judge introspect coverage — "alive ⇒ callable" (RFC 04 §5) — against the
/// producers that were actually asked. Pure, so the O4 boundary is testable
/// without a bus.
///
/// `locals: Some` is the `--registry` run: only the producers the local
/// slices name were queried, so only those count toward coverage — a live
/// producer whose slice a *partial* registry does not carry was never asked,
/// and counting it as "did not answer" would be a false finding (RFC 09
/// §5.1 O4; deep-review D3). `None` is the wildcard sweep, where every
/// roster producer was in the fan-in. Either way the evidence states the
/// scope it checked.
///
/// Matching follows the roster's own conventions: an instance suffix shares
/// its base slice (`sysinfo-2` → `sysinfo`, RFC 03 §1.5), and a service
/// origin's token names the service as its producer (RFC 06 §5), matched by
/// the slice's declared origin or name.
fn judge_introspect_coverage(
    roster: &std::collections::BTreeMap<String, Vec<String>>,
    locals: Option<&[RegistrySlice]>,
    answered: usize,
) -> Option<DoctorFinding> {
    let live: usize = roster.values().map(Vec::len).sum();

    let (in_scope, scope) = match locals {
        None => (
            live,
            "scope: the whole roster (fleet-wide wildcard sweep)".to_string(),
        ),
        Some(locals) => {
            let named = |origin: &str, producer: &str| {
                let base_name = zenkey::grammar::Producer::parse_chunk(producer)
                    .map(|p| p.name().to_string())
                    .unwrap_or_else(|_| producer.to_string());
                locals.iter().any(|l| {
                    l.name == base_name
                        || l.service_origin.as_ref().map(Declared::token) == Some(origin)
                })
            };
            let in_scope: usize = roster
                .iter()
                .map(|(origin, producers)| producers.iter().filter(|p| named(origin, p)).count())
                .sum();
            let mut names: Vec<&str> = locals.iter().map(|l| l.name.as_str()).collect();
            names.sort_unstable();
            names.dedup();
            let not_asked = live - in_scope;
            (
                in_scope,
                format!(
                    "scope: the producer(s) the local registry names ({}); {} other \
                     live producer(s) were not asked and are not counted (O4)",
                    names.join(", "),
                    not_asked
                ),
            )
        }
    };
    (answered < in_scope).then(|| {
        finding(
            DoctorSeverity::Error,
            CheckId::IntrospectCoverage,
            "fleet",
            format!(
                "{} of {} live producer(s) in scope did not answer introspect — \
                 alive ⇒ callable, so this is a finding, not a boot race; {scope}",
                in_scope - answered,
                in_scope
            ),
            Some("RFC 04 §5"),
        )
    })
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
                        CheckId::StaleState,
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

/// Judge every declared `{var}` family's key population against its declared
/// `cardinality` (#221) — pure, so the acceptance case (declared 16, 40
/// observed) is testable without a bus.
///
/// The honesty rules, verbatim from the issue:
///
/// - Observed **over** declared is a finding (RFC 04 §1.2's budget is a
///   MUST); observed **under** declared is **not** — an idle host declares
///   nothing wrong, and a bounded window proves a lower bound, never the
///   population (RFC 09 §5.1 O4/O6). The window rides in the evidence.
/// - `{path...}` rest-variable families are unbounded by construction and
///   are **exempt and say so** — "exempt: rest-variable", never a silent
///   skip and never a pass (the RFC 08 §6.1 v1.20 shape for subject checks).
/// - Judged **per origin**: RFC 04 §1 bounds cardinality per producer, so
///   one origin over the bound is conclusive and two origins' healthy
///   populations are never summed into a fake violation.
fn judge_cardinality(
    slices: &crate::model::registry::SliceSet,
    observed: &crate::judge::budget::BudgetObservation,
    window_s: f64,
) -> Vec<DoctorFinding> {
    let mut findings = Vec::new();

    let mut over: Examples<DoctorFinding> = Examples::new(FINDING_CAP);

    for slice in slices.slices() {
        for s in &slice.subjects {
            if !s.path.contains('{') {
                continue; // a literal subject's population is 1 by construction
            }
            if s.path.contains("...") {
                let seen: usize = observed
                    .family(&slice.name, &s.path)
                    .map(|origins| origins.values().map(|keys| keys.len()).sum())
                    .unwrap_or(0);
                findings.push(finding(
                    DoctorSeverity::Info,
                    CheckId::CardinalityOverDeclared,
                    format!("{}/{}", slice.name, s.path),
                    format!(
                        "exempt: rest-variable — a `{{var...}}` family is unbounded by \
                         construction, so its declared cardinality is not a bound this \
                         check can pass or fail; {seen} distinct key(s) observed in \
                         {window_s:.0}s"
                    ),
                    Some("RFC 08 §6.1"),
                ));
                continue;
            }
            let Some(declared) = s.cardinality else {
                continue; // nothing declared, nothing to judge (the RFC 08 §5
                // lint that requires the field is the producer build's)
            };
            let Some(origins) = observed.family(&slice.name, &s.path) else {
                continue; // unobserved is not "within budget" — no verdict
            };
            for (origin, keys) in origins {
                if keys.len() as i64 <= declared {
                    continue; // under/at declared: not a finding (O4)
                }
                let examples = Examples::collect(
                    crate::judge::common::EXPANSION_CAP,
                    keys.iter().map(String::as_str),
                );
                let subject = if origin.starts_with('@') {
                    format!("{origin}/{}", s.path)
                } else {
                    format!("{origin}/{}/{}", slice.name, s.path)
                };
                over.push_with(|| {
                    finding(
                        DoctorSeverity::Warning,
                        CheckId::CardinalityOverDeclared,
                        subject,
                        format!(
                            "{} distinct key(s) observed in {window_s:.0}s exceed the \
                         declared cardinality {declared} — e.g. {}. A bounded window \
                         observes a lower bound: the population is at least this",
                            keys.len(),
                            examples.as_slice().join(", ")
                        ),
                        Some("RFC 04 §1.2"),
                    )
                });
            }
        }
    }
    emit_capped(
        &mut findings,
        over,
        CheckId::CardinalityOverDeclared,
        "more origin famil(y|ies) over their declared cardinality",
    );
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOUNDED: &str = r#"
        [registry]
        version = "1.0"
        app = "t"
        convention = 1
        [producer]
        name = "sysinfo"
        [[subject]]
        path = "disk/{mount}/used"
        class = "telemetry"
        type = "Point"
        cardinality = 16
    "#;

    /// The #221 acceptance case: declared 16, 40 observed expansions — the
    /// finding fires with the count, the declared bound, examples, and the
    /// window it rests on.
    #[test]
    fn cardinality_over_declared_fires_with_count_and_examples() {
        let slices = crate::model::registry::SliceSet::from_toml_for_tests(BOUNDED);
        let keys: Vec<String> = (0..40)
            .map(|i| format!("v1/h-aaaaaaaaaaaa/telemetry/sysinfo/disk/m{i:02}/used"))
            .collect();
        let obs = crate::judge::budget::BudgetObservation::observe(
            "",
            &slices,
            keys.iter().map(String::as_str),
        );
        let findings = judge_cardinality(&slices, &obs, 10.0);
        assert_eq!(findings.len(), 1, "{findings:?}");
        let f = &findings[0];
        assert_eq!(f.check, CheckId::CardinalityOverDeclared);
        assert_eq!(f.severity, DoctorSeverity::Warning);
        assert_eq!(f.subject, "h-aaaaaaaaaaaa/sysinfo/disk/{mount}/used");
        assert!(f.evidence.contains("40 distinct key(s)"), "{}", f.evidence);
        assert!(f.evidence.contains("cardinality 16"), "{}", f.evidence);
        assert!(f.evidence.contains("10s"), "the window is stated");
        assert!(
            f.evidence.contains("disk/m00/used"),
            "examples are named: {}",
            f.evidence
        );
    }

    /// Under (or at) the declared bound is **not** a finding: an idle host
    /// declares nothing wrong, and a bounded window proves a lower bound,
    /// never the population (O4/O6).
    #[test]
    fn cardinality_under_declared_is_not_a_finding() {
        let slices = crate::model::registry::SliceSet::from_toml_for_tests(BOUNDED);
        let keys = [
            "v1/h-aaaaaaaaaaaa/telemetry/sysinfo/disk/root/used",
            "v1/h-aaaaaaaaaaaa/telemetry/sysinfo/disk/var/used",
            // Two origins at 15 each must never be summed into a fake 30 > 16.
            "v1/h-bbbbbbbbbbbb/telemetry/sysinfo/disk/root/used",
        ];
        let obs = crate::judge::budget::BudgetObservation::observe("", &slices, keys);
        assert!(judge_cardinality(&slices, &obs, 5.0).is_empty());
    }

    /// The other #221 acceptance case: a `{path...}` family yields the
    /// exemption wording — "exempt: rest-variable" — and never a pass (nor an
    /// over-finding, however many members it grows).
    #[test]
    fn rest_variable_families_are_exempt_and_say_so() {
        let toml = r#"
            [registry]
            version = "1.0"
            app = "t"
            convention = 1
            [producer]
            name = "gnmi"
            [[subject]]
            path = "{device}/{path...}"
            class = "telemetry"
            type = "Point"
            cardinality = 2
        "#;
        let slices = crate::model::registry::SliceSet::from_toml_for_tests(toml);
        let keys: Vec<String> = (0..5)
            .map(|i| format!("v1/h-aaaaaaaaaaaa/telemetry/gnmi/sw1/if/eth{i}/rx"))
            .collect();
        let obs = crate::judge::budget::BudgetObservation::observe(
            "",
            &slices,
            keys.iter().map(String::as_str),
        );
        let findings = judge_cardinality(&slices, &obs, 5.0);
        assert_eq!(findings.len(), 1, "{findings:?}");
        let f = &findings[0];
        assert_eq!(f.severity, DoctorSeverity::Info, "an exemption, not a pass");
        assert!(
            f.evidence.starts_with("exempt: rest-variable"),
            "{}",
            f.evidence
        );
        assert!(f.evidence.contains("5 distinct key(s)"), "{}", f.evidence);
        // And with nothing observed the family still says so — exempt is a
        // property of the declaration, not of the traffic.
        let quiet = judge_cardinality(&slices, &Default::default(), 5.0);
        assert_eq!(quiet.len(), 1);
        assert!(quiet[0].evidence.starts_with("exempt: rest-variable"));
    }

    /// Deep-review D4: the qos-observed-mismatch cap bounds *violators*, not
    /// map entries. 30 judged keys where the first 5 (in map order) are
    /// clean and the remaining 25 violate: every violator is counted — the
    /// first 20 as findings, the other 5 in a remainder note that counts
    /// correctly. Capping before filtering used to drop the violators past
    /// the first 20 map entries and miscount the note.
    #[test]
    fn qos_mismatch_cap_bounds_violators_not_map_entries() {
        let mut qos_bad: std::collections::BTreeMap<String, (String, u64, u64)> =
            std::collections::BTreeMap::new();
        for i in 0..30u32 {
            // k00..k04 sort first and are clean; k05..k29 are violators.
            let bad = if i < 5 { 0 } else { 1 };
            qos_bad.insert(
                format!("v1/h-a/telemetry/x/k{i:02}"),
                ("tel".into(), bad, 10),
            );
        }
        let findings = judge_qos_observed(&qos_bad);
        let per_key: Vec<&DoctorFinding> = findings
            .iter()
            .filter(|f| f.severity == DoctorSeverity::Warning)
            .collect();
        assert_eq!(per_key.len(), FINDING_CAP, "the cap bounds the findings");
        assert!(
            per_key.iter().all(|f| f.evidence.starts_with("1 of 10")),
            "only violators become findings: {findings:#?}"
        );
        assert!(
            per_key.iter().any(|f| f.subject.ends_with("k24")),
            "violators past the first {FINDING_CAP} map entries (k20..k24) are \
             not dropped: {findings:#?}"
        );
        let note = findings
            .iter()
            .find(|f| f.severity == DoctorSeverity::Info)
            .expect("a remainder note");
        assert_eq!(
            note.evidence, "… and 5 more key(s) with the same finding",
            "the note counts violators (25 − 20), not map entries"
        );

        // At or under the cap: every violator is a finding, no note.
        let few: std::collections::BTreeMap<String, (String, u64, u64)> = qos_bad
            .iter()
            .take(10)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let findings = judge_qos_observed(&few);
        assert_eq!(findings.len(), 5, "{findings:#?}");
        assert!(
            findings
                .iter()
                .all(|f| f.severity == DoctorSeverity::Warning)
        );
    }

    fn roster_of(entries: &[(&str, &[&str])]) -> std::collections::BTreeMap<String, Vec<String>> {
        entries
            .iter()
            .map(|(origin, producers)| {
                (
                    origin.to_string(),
                    producers.iter().map(|p| p.to_string()).collect(),
                )
            })
            .collect()
    }

    fn slice_named(name: &str) -> RegistrySlice {
        zenkey::parse_slice(&format!(
            "[registry]\nversion = \"1.0\"\napp = \"t\"\nconvention = 1\n\
             [producer]\nname = \"{name}\"\n"
        ))
        .expect("fixture slice parses")
    }

    /// Deep-review D3: with `--registry` covering a subset of the fleet, a
    /// live producer the locals do not name was never asked — so it must not
    /// count as "did not answer" (O4). One local slice, answered by its one
    /// origin, beside an extra live producer: no finding.
    #[test]
    fn a_partial_registry_does_not_count_unasked_producers_against_coverage() {
        let roster = roster_of(&[("h-aaaaaaaaaaaa", &["sysinfo", "extra"])]);
        let locals = [slice_named("sysinfo")];
        assert_eq!(
            judge_introspect_coverage(&roster, Some(&locals), 1),
            None,
            "the un-asked producer is out of scope, not silent"
        );
    }

    /// …and when an in-scope producer really did not answer, the finding
    /// fires and its evidence states the scope it checked — including that
    /// the out-of-scope producer was not counted. An instance suffix shares
    /// its base slice (RFC 03 §1.5), so `sysinfo-2` is in scope too.
    #[test]
    fn introspect_coverage_evidence_states_its_scope() {
        let roster = roster_of(&[
            ("h-aaaaaaaaaaaa", &["sysinfo", "extra"]),
            ("h-bbbbbbbbbbbb", &["sysinfo-2"]),
        ]);
        let locals = [slice_named("sysinfo")];
        let f = judge_introspect_coverage(&roster, Some(&locals), 1).expect("a finding");
        assert_eq!(f.check, CheckId::IntrospectCoverage);
        assert!(f.evidence.contains("1 of 2"), "{}", f.evidence);
        assert!(
            f.evidence.contains("the local registry names (sysinfo)"),
            "{}",
            f.evidence
        );
        assert!(
            f.evidence
                .contains("1 other live producer(s) were not asked"),
            "{}",
            f.evidence
        );
    }

    /// A service origin's token names the service as its producer (RFC 06
    /// §5); a local slice matches it by declared origin.
    #[test]
    fn a_service_slice_scopes_its_origin_into_coverage() {
        let roster = roster_of(&[("@catalog", &["catalog"]), ("h-aaaaaaaaaaaa", &["extra"])]);
        let locals = [zenkey::parse_slice(
            "[registry]\nversion = \"1.0\"\napp = \"t\"\nconvention = 1\n\
             [service]\nname = \"catalog\"\norigin = \"@catalog\"\n",
        )
        .expect("service slice parses")];
        assert_eq!(judge_introspect_coverage(&roster, Some(&locals), 1), None);
        let f = judge_introspect_coverage(&roster, Some(&locals), 0).expect("a finding");
        assert!(f.evidence.contains("1 of 1"), "{}", f.evidence);
    }

    /// The wildcard sweep keeps the whole roster in scope, and says so.
    #[test]
    fn the_wildcard_sweep_judges_the_whole_roster() {
        let roster = roster_of(&[("h-aaaaaaaaaaaa", &["sysinfo", "extra"])]);
        let f = judge_introspect_coverage(&roster, None, 1).expect("a finding");
        assert!(f.evidence.contains("1 of 2"), "{}", f.evidence);
        assert!(f.evidence.contains("whole roster"), "{}", f.evidence);
        assert_eq!(judge_introspect_coverage(&roster, None, 2), None);
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
        assert_eq!(findings[0].check, CheckId::StaleState);
        assert!(findings[0].subject.contains("h-bbbbbbbbbbbb"));
        assert_eq!(unstamped, 1, "the unstamped sample is counted, not judged");
    }
}
