//! `check conform` (#222): one producer's registry, executed as a
//! conformance suite against the live fleet.
//!
//! RFC 13 §3 ("a conformance suite over the registry", v1.35) states the
//! obligation this module discharges: per declared surface, one of three
//! states — met, not met, unknowable with its reason — and the third never
//! folded into the second. It also states the limit that keeps the suite
//! honest: **it is a projection of the observer's own checks, and can find
//! nothing the observer cannot.** So nothing here re-implements a check.
//! The run has two halves:
//!
//! 1. **Run time, procedures** (RFC 08 §6.1). Every origin the roster shows
//!    running the producer is *called*: `introspect`, and every
//!    `kind = "read"` procedure with a concrete path, with no parameters.
//!    A write or a `long-running` procedure — or one that does not say it
//!    is a read — is never called, because a judge does not act; it is met
//!    when the origin's served slice declares it (RFC 13 §3, v1.45). The reply is read through
//!    the RFC 05 §3 vocabulary ([`ReservedError::parse`]): a value is met —
//!    save an `introspect` value that is not a readable slice in the
//!    spelling it declares, which answered and so is not silence, and is
//!    not met (RFC 08 §6, v1.44; #491);
//!    `invalid-args` is met (the procedure answered, and wants what this
//!    suite does not invent); `unsupported`/`gated` from a `when` procedure
//!    is met and **exempt**, with the kind binding judged, and from any
//!    other is not met; every other refusal is a reply, and met.
//!    **Silence from a rostered origin is not met** — alive ⇒ callable
//!    (RFC 13 §2, the doctor's `introspect-coverage` for one producer) —
//!    while silence from an `--origin` the roster does not show is
//!    unknowable.
//! 2. **The observer's checks, projected.** One doctor run scoped to the
//!    producer (`run_doctor_inner`), whose findings map by
//!    [`CheckId`] onto assertions (`project_conform`, pure): slice sync,
//!    describe totality, schema drift, and — with a `--for` window — each
//!    declared subject's presence and what rode on it; with `--deep`,
//!    freshness and the declared budget. The field-intelligence, stamper,
//!    storage and admin checks are not the producer's contract and are
//!    skipped.
//!
//! **Capabilities (RFC 04 §5, v1.41; the rule is RFC 13 §3, v1.45).** A `gated` reply says a `config:` or
//! `capability:` predicate is false *here*. The producer's registration
//! document (`state/<producer>/sensor`) may say, per device, which
//! capabilities hold — and when it claims every `capability:` predicate of
//! a procedure whose `when` names no `config:` one, the procedure and the
//! device contradict each other, and the assertion is not met. Not served,
//! or served without the member, is *not asked* (RFC 04 §5), and the
//! exemption stands with that said in its evidence. The map is read as the
//! producer's statement; no predicate is evaluated (RFC 08 §6.1, v1.41).

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use zenkey::slice::{Predicate, PredicateKind, ProcedureKind};
use zenkey::{Declared, RegistrySlice};

use crate::bus::producer::ReservedError;
use crate::bus::query::{Answer, read_introspect};
use crate::bus::write::{CallSpec, CallTarget};
use crate::judge::doctor::{DoctorInternals, DoctorSpec, run_doctor_inner};
use crate::model::facts::{KeyFacts, KeyShape, Registration};
use crate::model::registry::{SliceSet, SliceSource};
use crate::report::{
    Assertion, AssertionState, CallOutcome, CheckId, ConformReport, ConformSource, ConformSummary,
    ConformVerdict, DoctorFinding, DoctorReport, DoctorSeverity,
};
use crate::{Error, Result};

/// What one conformance run asks.
#[derive(Debug, Clone)]
pub struct ConformSpec {
    /// The producer whose registry slice is the suite.
    pub producer: String,
    /// Call this origin only, instead of every origin the roster shows
    /// running the producer. Not on the roster, its silence is unknowable.
    pub origin: Option<String>,
    /// Listen to the data planes for this long (`--for`) and judge each
    /// declared subject's presence and what rode on it. `None` = the
    /// subject assertions are not asked, and the report says so.
    pub listen: Option<Duration>,
    /// Run the doctor's deep checks too — freshness against `ttl_s`, the
    /// declared `[budget]` — which cost the data plane.
    pub deep: bool,
    /// Per-query timeout: each probe's wait, and the doctor's.
    pub timeout: Duration,
    /// Where `slices` came from, stated on the report.
    pub source: SliceSource,
}

/// Run the suite: call the producer's procedures on every origin in scope,
/// run the doctor scoped to it, and fold both into one [`ConformReport`].
///
/// Refused (an [`Error::unaskable`], exit 2 at the CLI) when no loaded slice
/// declares the producer, or `--origin` is not an origin: there is no suite
/// to run, and a report would claim one ran.
pub async fn run_conform(
    fleet: &crate::Fleet<'_>,
    slices: &SliceSet,
    spec: &ConformSpec,
) -> Result<ConformReport> {
    let slice = slices.get(&spec.producer).ok_or_else(|| {
        Error::unaskable(
            format!("--producer {}", spec.producer),
            "no loaded registry slice declares it — the suite is the producer's own \
             registry, and there is none to run (RFC 08 §6)",
        )
    })?;
    if let Some(origin) = &spec.origin {
        CallTarget::parse(origin)?;
    }

    let roster = crate::bus::roster::roster(fleet, spec.timeout).await?;
    let instances = instances_of(&roster, slice, spec.origin.as_deref());

    let mut assertions = Vec::new();
    let mut caps: BTreeMap<String, Capabilities> = BTreeMap::new();

    // --- run time: introspect, then every declared procedure ------------
    let mut served: BTreeMap<String, Option<RegistrySlice>> = BTreeMap::new();
    let mut answers = Vec::new();
    for inst in &instances {
        let (probe, slice_read) = probe_introspect(fleet, inst, spec.timeout).await;
        served.insert(inst.origin.clone(), slice_read);
        answers.push((
            inst,
            judge_probe(
                None,
                None,
                &probe,
                &Capabilities::Unconsulted,
                inst,
                spec.timeout,
                slice,
            ),
        ));
    }
    assertions.push(fold(
        "procedure/introspect",
        "@rpc/introspect",
        answers,
        "RFC 08 §6",
    ));

    for decl in &slice.procedures {
        if decl.path == "introspect" {
            continue; // already asked, above
        }
        let id = format!("procedure/{}", decl.path);
        let subject = format!("@rpc/{}", decl.path);
        if decl.path.contains('{') {
            assertions.push(Assertion {
                id,
                subject,
                state: AssertionState::Unknowable {
                    reason: "a `{var}` path names no concrete key, and this suite does \
                             not invent one"
                        .into(),
                },
                evidence: "declared; not called".into(),
                citation: Some("RFC 08 §6.1".into()),
                exempt: None,
            });
            continue;
        }
        let is_read = decl
            .kind
            .as_ref()
            .and_then(Declared::known)
            .is_some_and(|k| *k == ProcedureKind::Read);
        let mut answers = Vec::new();
        for inst in &instances {
            let answer = if is_read {
                let probe = probe(fleet, inst, &decl.path, spec.timeout).await;
                let when = decl.when.as_deref();
                let device = device_of(&decl.path);
                // The registration document is fetched only when a `gated`
                // reply could contradict it, and once per origin.
                let wants_caps = when.is_some_and(could_contradict)
                    && probe
                        .iter()
                        .any(|p| matches!(p, Probe::Error { name, .. } if name == ReservedError::Gated.name()));
                if wants_caps && !caps.contains_key(&inst.origin) {
                    let c = fetch_capabilities(fleet, inst, spec.timeout).await;
                    caps.insert(inst.origin.clone(), c);
                }
                let c = caps
                    .get(&inst.origin)
                    .filter(|_| wants_caps)
                    .unwrap_or(&Capabilities::Unconsulted);
                judge_probe(when, device, &probe, c, inst, spec.timeout, slice)
            } else {
                judge_unprobed(decl, served.get(&inst.origin).and_then(Option::as_ref))
            };
            answers.push((inst, answer));
        }
        assertions.push(fold(&id, &subject, answers, "RFC 08 §6.1"));
    }

    // --- the observer's checks, scoped to this producer -----------------
    let locals = SliceSet::from_slices(vec![slice.clone()]);
    let (doctor, internals) = run_doctor_inner(
        fleet,
        Some(&locals),
        &DoctorSpec {
            deep: spec.deep,
            sample: None,
            timeout: spec.timeout,
            listen: spec.listen,
        },
    )
    .await?;

    // An unseen `when` subject whose device claims its capabilities is no
    // longer excused by its gate: fetch the claim, once per origin, only
    // when there is such a subject to judge.
    if doctor.observation.is_some() {
        let wants = slice.subjects.iter().any(|s| {
            s.when.as_deref().is_some_and(could_contradict)
                && !internals
                    .seen
                    .contains_key(&(slice.name.clone(), s.path.clone()))
        });
        if wants {
            for inst in &instances {
                if !caps.contains_key(&inst.origin) {
                    let c = fetch_capabilities(fleet, inst, spec.timeout).await;
                    caps.insert(inst.origin.clone(), c);
                }
            }
        }
    }

    assertions.extend(project_conform(
        slice,
        fleet.base(),
        &locals,
        &doctor,
        &internals,
        spec.origin.as_deref(),
        &caps,
    ));

    let mut not_asked = Vec::new();
    if doctor.observation.is_none() {
        not_asked.push(
            "observed/*: no --for window — which declared subjects ride was not asked, \
             and their QoS, payloads, kinds and rates with it"
                .to_string(),
        );
    }
    if !spec.deep {
        not_asked.push(
            "stale-state/*, budget: not asked without --deep (a state snapshot and a \
             health fetch cost the data plane)"
                .to_string(),
        );
    }
    not_asked.push(
        "the subject half at build time (RFC 08 §6.1): a producer's mappers are checked \
         against its registry in its own tests — a wire suite sees what rides, never \
         what a mapper could emit"
            .to_string(),
    );

    let summary = ConformSummary::of(&assertions);
    Ok(ConformReport {
        producer: slice.name.clone(),
        slice_source: source_of(spec.source),
        origins_asked: instances.iter().map(|i| i.origin.clone()).collect(),
        verdict: ConformVerdict::of(&summary),
        summary,
        assertions,
        observation: doctor.observation,
        deep: spec.deep,
        not_asked,
    })
}

fn source_of(s: SliceSource) -> ConformSource {
    match s {
        SliceSource::Bus => ConformSource::Bus,
        SliceSource::Dirs => ConformSource::Dirs,
        SliceSource::Union => ConformSource::Union,
    }
}

/// One running copy of the producer: where it is, the producer chunk its
/// keys carry (an instance suffix included, RFC 03 §1.5), and whether the
/// roster showed it alive — which is what makes its silence attributable.
#[derive(Debug, Clone)]
struct Instance {
    origin: String,
    chunk: String,
    rostered: bool,
}

/// The producer's instances in scope, read off the roster the way the
/// doctor's coverage check reads it: an instance suffix shares its base
/// slice, and a service origin's token names the service.
fn instances_of(
    roster: &BTreeMap<String, Vec<String>>,
    slice: &RegistrySlice,
    origin: Option<&str>,
) -> Vec<Instance> {
    let mut out = Vec::new();
    for (o, producers) in roster {
        if origin.is_some_and(|want| want != o) {
            continue;
        }
        for p in producers {
            let matches = match &slice.service_origin {
                Some(service) => service.token() == o,
                None => zenkey::grammar::Producer::parse_chunk(p)
                    .map(|p| p.name() == slice.name)
                    .unwrap_or(p == &slice.name),
            };
            if matches {
                out.push(Instance {
                    origin: o.clone(),
                    chunk: p.clone(),
                    rostered: true,
                });
            }
        }
    }
    if out.is_empty()
        && let Some(o) = origin
    {
        out.push(Instance {
            origin: o.to_string(),
            chunk: slice.name.clone(),
            rostered: false,
        });
    }
    out
}

/// What one call came back with.
#[derive(Debug, Clone)]
enum Probe {
    /// A value reply. What it says is not the procedure's contract — except
    /// `introspect`'s, which [`probe_introspect`] reads.
    Value,
    /// An `introspect` value reply that is not a readable slice (#491): it
    /// answered, so this is not silence, and it is not a slice either.
    Unreadable(crate::report::UnreadableSlice),
    /// An RFC 05 §3 error envelope.
    Error { name: String, message: String },
    /// The call itself failed before any answer could come back.
    Failed(String),
}

/// Call `introspect` on one instance, reading each value reply in the
/// spelling it declares (RFC 08 §6, v1.44) through the reader every
/// introspect consumer shares — so the suite, the roster and the sweep
/// cannot disagree about which reply was a slice (#491). This used to sniff
/// the text whatever the reply declared, which is the second-guessing §6
/// forbids. The first slice that read is the origin's served slice.
async fn probe_introspect(
    fleet: &crate::Fleet<'_>,
    inst: &Instance,
    timeout: Duration,
) -> (Vec<Probe>, Option<RegistrySlice>) {
    let target = match CallTarget::parse(&inst.origin) {
        Ok(t) => t,
        Err(e) => return (vec![Probe::Failed(crate::one_line(&e))], None),
    };
    let spec = CallSpec {
        target: &target,
        producer: &inst.chunk,
        procedure: "introspect",
        params: &[],
        body: None,
        attachment: None,
        timeout,
        slices: None,
        force: false,
    };
    let answers = match crate::bus::write::call_answers(fleet, spec).await {
        Ok((_, _, answers)) => answers,
        Err(e) => return (vec![Probe::Failed(crate::one_line(&e))], None),
    };
    let mut slice = None;
    let probes = answers
        .into_iter()
        .map(|a| match a.answer {
            Answer::Value(bytes) => {
                match read_introspect(a.encoding.as_deref(), &bytes.to_bytes()) {
                    Ok((s, ..)) => {
                        slice.get_or_insert(s);
                        Probe::Value
                    }
                    Err(u) => Probe::Unreadable(u),
                }
            }
            Answer::Error { name, message } => Probe::Error { name, message },
        })
        .collect();
    (probes, slice)
}

/// Call one procedure on one instance, with no parameters and no body.
/// Empty is silence.
async fn probe(
    fleet: &crate::Fleet<'_>,
    inst: &Instance,
    path: &str,
    timeout: Duration,
) -> Vec<Probe> {
    let target = match CallTarget::parse(&inst.origin) {
        Ok(t) => t,
        Err(e) => return vec![Probe::Failed(crate::one_line(&e))],
    };
    let spec = CallSpec {
        target: &target,
        producer: &inst.chunk,
        procedure: path,
        params: &[],
        body: None,
        attachment: None,
        timeout,
        slices: None,
        force: false,
    };
    match crate::bus::write::call(fleet, spec).await {
        Ok(report) => report
            .answers
            .into_iter()
            .map(|a| match a.outcome {
                CallOutcome::Ok { .. } => Probe::Value,
                CallOutcome::Err(e) => Probe::Error {
                    name: e.name,
                    message: e.message,
                },
            })
            .collect(),
        Err(e) => vec![Probe::Failed(crate::one_line(&e))],
    }
}

/// One origin's answer about one surface, before the fold.
#[derive(Debug, Clone)]
struct OriginAnswer {
    state: AssertionState,
    evidence: String,
    exempt: Option<String>,
}

impl OriginAnswer {
    fn met(evidence: impl Into<String>) -> OriginAnswer {
        OriginAnswer {
            state: AssertionState::Met,
            evidence: evidence.into(),
            exempt: None,
        }
    }

    fn not_met(evidence: impl Into<String>) -> OriginAnswer {
        OriginAnswer {
            state: AssertionState::NotMet,
            evidence: evidence.into(),
            exempt: None,
        }
    }

    fn unknowable(reason: impl Into<String>, evidence: impl Into<String>) -> OriginAnswer {
        OriginAnswer {
            state: AssertionState::Unknowable {
                reason: reason.into(),
            },
            evidence: evidence.into(),
            exempt: None,
        }
    }

    fn exempt(evidence: impl Into<String>, exempt: String) -> OriginAnswer {
        OriginAnswer {
            state: AssertionState::Met,
            evidence: evidence.into(),
            exempt: Some(exempt),
        }
    }
}

/// Fold one surface's per-origin answers into its assertion: the worst
/// state wins (not met > unknowable > met), the evidence names each origin,
/// and an exemption survives only on a met fold. No origin at all is
/// unknowable — silence from nobody is attributable to nothing (RFC 13 §2).
fn fold(
    id: &str,
    subject: &str,
    answers: Vec<(&Instance, OriginAnswer)>,
    citation: &str,
) -> Assertion {
    if answers.is_empty() {
        return Assertion {
            id: id.to_string(),
            subject: subject.to_string(),
            state: AssertionState::Unknowable {
                reason: "no origin of this producer is on the roster, and no --origin \
                         named one — there is nobody to call (RFC 13 §2)"
                    .into(),
            },
            evidence: "not called".into(),
            citation: Some(citation.to_string()),
            exempt: None,
        };
    }
    let mut state = AssertionState::Met;
    let mut exempt = None;
    let mut evidence = Vec::new();
    for (inst, a) in answers {
        state = state.worst(a.state);
        exempt = exempt.or(a.exempt);
        evidence.push(format!("{}: {}", inst.origin, a.evidence));
    }
    Assertion {
        id: id.to_string(),
        subject: subject.to_string(),
        exempt: exempt.filter(|_| state.is_met()),
        state,
        evidence: evidence.join("; "),
        citation: Some(citation.to_string()),
    }
}

/// The device a path sits under, when its first chunk is literal (RFC 06
/// §3: a device-tracking producer's subjects lead with the device).
fn device_of(path: &str) -> Option<&str> {
    path.split('/').next().filter(|c| !c.contains('{'))
}

/// Whether a `when` could be contradicted by a capability claim: a `gated`
/// reply says a `config:` or `capability:` predicate is false, so only an
/// entry whose non-feature predicates are all `capability:` — and whose
/// kinds this build knows — can be held to the registration document.
fn could_contradict(when: &[Predicate]) -> bool {
    when.iter().any(|p| p.kind.is(&PredicateKind::Capability))
        && when.iter().all(|p| {
            matches!(
                p.kind.known(),
                Some(PredicateKind::Capability | PredicateKind::Feature)
            )
        })
}

/// Judge one origin's replies to one read probe (`introspect` included,
/// with no `when`). Several replies from one origin fold like origins do.
fn judge_probe(
    when: Option<&[Predicate]>,
    device: Option<&str>,
    replies: &[Probe],
    caps: &Capabilities,
    inst: &Instance,
    timeout: Duration,
    slice: &RegistrySlice,
) -> OriginAnswer {
    if replies.is_empty() {
        return if inst.rostered {
            OriginAnswer::not_met(format!(
                "silent within {:.1}s, and the roster shows it alive — alive ⇒ callable, \
                 so this is a finding, not a boot race (RFC 13 §2)",
                timeout.as_secs_f64()
            ))
        } else {
            OriginAnswer::unknowable(
                "not on the roster — silence from an origin nobody showed alive is \
                 attributable to nothing (RFC 13 §2)",
                format!("silent within {:.1}s", timeout.as_secs_f64()),
            )
        };
    }
    let mut out: Option<OriginAnswer> = None;
    for reply in replies {
        let a = judge_reply(when, device, reply, caps, slice);
        out = Some(match out {
            None => a,
            Some(prev) => {
                let evidence = format!("{}, {}", prev.evidence, a.evidence);
                OriginAnswer {
                    exempt: prev.exempt.or(a.exempt),
                    state: prev.state.worst(a.state),
                    evidence,
                }
            }
        });
    }
    out.expect("at least one reply")
}

/// One reply, read through the RFC 05 §3 vocabulary. Pure.
fn judge_reply(
    when: Option<&[Predicate]>,
    device: Option<&str>,
    reply: &Probe,
    caps: &Capabilities,
    slice: &RegistrySlice,
) -> OriginAnswer {
    let (name, message) = match reply {
        Probe::Value => return OriginAnswer::met("a value reply"),
        // Not unknowable: this build reads both spellings, so what failed is
        // the reply — a declaration that is neither (§6's MUST), or a
        // document malformed in the one it declared. The doctor files the
        // same reply as `slice-parse`, and the suite finds only what the
        // observer does (RFC 13 §3).
        Probe::Unreadable(u) => {
            return OriginAnswer::not_met(format!(
                "{} — §6 requires a slice in a declared spelling (RFC 08 §6, v1.44)",
                u.sentence()
            ));
        }
        Probe::Failed(e) => {
            return OriginAnswer::unknowable(
                format!("the call could not be made: {e}"),
                "not answered",
            );
        }
        Probe::Error { name, message } => (name.as_str(), message.as_str()),
    };
    match ReservedError::parse(name) {
        Some(ReservedError::InvalidArgs) => OriginAnswer::met(format!(
            "{name} — the procedure answered, and wants arguments this suite does not \
             invent"
        )),
        Some(e @ (ReservedError::Unsupported | ReservedError::Gated)) => {
            judge_gate(e, message, when, device, caps)
        }
        Some(_) => OriginAnswer::met(format!("{name} — a refusal is a reply (RFC 05 §3)")),
        None => {
            let registered = slice
                .errors
                .iter()
                .any(|e| e.wire_name(&slice.name) == name);
            OriginAnswer::met(if registered {
                format!("{name} — a registered error (RFC 08 §2), and a refusal is a reply")
            } else {
                format!(
                    "{name} — a refusal is a reply; neither reserved nor registered by \
                     this slice (RFC 05 §3)"
                )
            })
        }
    }
}

/// `unsupported` or `gated`: the §6.1 conditional-surface rule, the kind
/// binding, and — for `gated` — the device's own capability claim.
fn judge_gate(
    error: ReservedError,
    message: &str,
    when: Option<&[Predicate]>,
    device: Option<&str>,
    caps: &Capabilities,
) -> OriginAnswer {
    let name = error.name();
    let Some(preds) = when else {
        return OriginAnswer::not_met(format!(
            "{name} ({message:?}) from a procedure not declared `when` — a conditional \
             surface declares its condition (RFC 08 §6.1, RFC 13 §3)"
        ));
    };
    let tokens: Vec<String> = preds.iter().map(Predicate::token).collect();
    let exempt = format!("when: {}", tokens.join(", "));
    let has = |k: PredicateKind| preds.iter().any(|p| p.kind.is(&k));
    let unknown = preds.iter().any(|p| p.kind.known().is_none());
    let bound = match error {
        ReservedError::Unsupported => has(PredicateKind::Feature),
        _ => has(PredicateKind::Config) || has(PredicateKind::Capability),
    };
    if !bound {
        if unknown {
            return OriginAnswer::exempt(
                format!(
                    "{name}; its kind binding is not asked — a predicate kind this build \
                     does not know (RFC 13 §3)"
                ),
                exempt,
            );
        }
        return OriginAnswer::not_met(format!(
            "{name} from a procedure whose `when` is [{}] — `unsupported` answers a \
             false `feature:` predicate, `gated` a false `config:` or `capability:` one \
             (RFC 08 §6.1, RFC 13 §3)",
            tokens.join(", ")
        ));
    }
    if error != ReservedError::Gated || !could_contradict(preds) {
        return OriginAnswer::exempt(format!("{name} — conditional, and said so"), exempt);
    }
    let needed: Vec<&str> = preds
        .iter()
        .filter(|p| p.kind.is(&PredicateKind::Capability))
        .map(|p| p.name.as_str())
        .collect();
    match caps {
        Capabilities::Unconsulted => {
            OriginAnswer::exempt(format!("{name} — conditional, and said so"), exempt)
        }
        Capabilities::Absent(why) => OriginAnswer::exempt(
            format!("{name} — conditional, and said so; capabilities {why}"),
            exempt,
        ),
        Capabilities::Served(map) => {
            let missing: Vec<&str> = needed
                .iter()
                .copied()
                .filter(|n| !claims(map, device, n))
                .collect();
            let whose = device.map_or_else(|| "the producer".to_string(), str::to_string);
            if missing.is_empty() {
                OriginAnswer::not_met(format!(
                    "{name}, but the registration document claims {} hold{} for {whose} — \
                     the device says it can, the procedure says it cannot (RFC 04 §5, \
                     RFC 08 §6.1)",
                    needed.join(", "),
                    if needed.len() == 1 { "s" } else { "" },
                ))
            } else {
                OriginAnswer::exempt(
                    format!(
                        "{name} — conditional, and the registration document agrees: it \
                         does not claim {} for {whose}",
                        missing.join(", ")
                    ),
                    exempt,
                )
            }
        }
    }
}

/// A procedure this suite does not call: met iff the origin's served slice
/// declares it (a judge does not act — RFC 05 §2's write is a side effect).
fn judge_unprobed(
    decl: &zenkey::slice::ProcedureDecl,
    served: Option<&RegistrySlice>,
) -> OriginAnswer {
    // RFC 08 §2's third idiom, `long-running`, is a token this build's
    // `ProcedureKind` does not carry; it starts a job, which is an act.
    let why = match decl.kind.as_ref() {
        Some(k) if k.is(&ProcedureKind::Write) => "a write acts, so it is never called",
        Some(k) if k.is(&ProcedureKind::Read) => "not called",
        Some(k) if k.token() == "long-running" => {
            "a long-running procedure starts a job, so it is never called"
        }
        _ => "its kind is not declared a read, so it is never called",
    };
    match served {
        Some(s) if s.serves_procedure(&decl.path) => {
            OriginAnswer::met(format!("the served slice declares it; {why}"))
        }
        Some(_) => OriginAnswer::not_met(format!(
            "the served slice does not declare it — introspect is the contract a caller \
             reads (RFC 08 §6); {why}"
        )),
        None => OriginAnswer::unknowable(
            "introspect served no parseable slice, so what this origin declares is unknown",
            why,
        ),
    }
}

/// What the producer's registration document says holds, per device
/// (RFC 04 §5, v1.41).
#[derive(Debug, Clone)]
enum Capabilities {
    /// Not fetched: no reply this run could be contradicted by it.
    Unconsulted,
    /// Asked, and not available — *not asked* on the producer's side
    /// (RFC 04 §5): the sentence says which way.
    Absent(String),
    /// The `capabilities` member: device chunk (or `"*"`) → names.
    Served(BTreeMap<String, BTreeSet<String>>),
}

/// Whether the map claims `name` for `device` — its own entry, or the
/// producer's device-less `"*"`. A value spelled `capability:<name>` is
/// read as `<name>`.
fn claims(map: &BTreeMap<String, BTreeSet<String>>, device: Option<&str>, name: &str) -> bool {
    [Some("*"), device]
        .into_iter()
        .flatten()
        .filter_map(|d| map.get(d))
        .flatten()
        .any(|n| n.strip_prefix("capability:").unwrap_or(n) == name)
}

/// Read one instance's `state/<producer>/sensor` and take its
/// `capabilities` member.
async fn fetch_capabilities(
    fleet: &crate::Fleet<'_>,
    inst: &Instance,
    timeout: Duration,
) -> Capabilities {
    let Ok(id) = zenkey::HostId::parse(&inst.origin) else {
        return Capabilities::Absent(
            "not read: a service origin's registration document is outside this suite".into(),
        );
    };
    let origin = zenkey::origin::RemoteOrigin::from_host(id);
    let selector = fleet.wire(zenkey::selector::common_family(
        zenkey::selector::Scope::origin(&origin),
        zenkey::CommonFamily::Sensor,
    ));
    let answers = match crate::bus::query::fleet_get(
        fleet,
        &selector,
        &crate::bus::query::GetOpts::new(timeout),
    )
    .await
    {
        Ok(a) => a,
        Err(e) => {
            return Capabilities::Absent(format!(
                "unavailable: the registration document could not be asked for ({})",
                crate::one_line(&e)
            ));
        }
    };
    let doc = answers.iter().find_map(|a| {
        let crate::bus::query::Answer::Value(bytes) = &a.answer else {
            return None;
        };
        let parsed = zenkey::grammar::parse_full(fleet.base(), &a.key)?;
        (parsed.producer().map(|p| p.chunk()).as_deref() == Some(inst.chunk.as_str()))
            .then(|| crate::model::decode::structural_value(&bytes.to_bytes()))
    });
    let doc = match doc {
        None => {
            return Capabilities::Absent(
                "not asked: no registration document answered (RFC 04 §5 — absence is \
                 not asked, never \"none hold\")"
                    .into(),
            );
        }
        Some(None) => {
            return Capabilities::Absent(
                "unreadable: the registration document does not decode".into(),
            );
        }
        Some(Some(doc)) => doc,
    };
    let Some(member) = doc.get("capabilities") else {
        return Capabilities::Absent(
            "not asked: the registration document carries no `capabilities` (RFC 04 §5)".into(),
        );
    };
    let Some(obj) = member.as_object() else {
        return Capabilities::Absent(
            "unreadable: `capabilities` is not a map of device to names".into(),
        );
    };
    let mut map = BTreeMap::new();
    for (device, names) in obj {
        let Some(names) = names.as_array() else {
            return Capabilities::Absent(format!(
                "unreadable: `capabilities.{device}` is not a list of names"
            ));
        };
        let set: BTreeSet<String> = names
            .iter()
            .filter_map(|n| n.as_str())
            .map(str::to_string)
            .collect();
        map.insert(device.clone(), set);
    }
    Capabilities::Served(map)
}

/// The listen-phase checks projected per declared subject, and whether the
/// finding marks the assertion not met or merely unknowable.
const LISTEN_CHECKS: [CheckId; 6] = [
    CheckId::QosObservedMismatch,
    CheckId::PayloadInvalid,
    CheckId::PayloadUndecodable,
    CheckId::KindMismatch,
    CheckId::RateOverDeclared,
    CheckId::CardinalityOverDeclared,
];

/// The doctor's findings for one producer, attributed: per declared path
/// for the per-key checks, per origin for the producer-level ones. Built
/// once, so the projection below reads a table rather than re-parsing keys
/// per assertion.
#[derive(Debug, Default)]
struct Attributed<'r> {
    /// `(check, declared path)` → findings.
    by_path: BTreeMap<(CheckId, String), Vec<&'r DoctorFinding>>,
    /// Unregistered keys under the producer: the tail after the producer
    /// chunk, and its class.
    unregistered: BTreeMap<String, (String, &'r DoctorFinding)>,
    /// `(check)` → producer-level findings, origin-filtered.
    producer: BTreeMap<CheckId, Vec<&'r DoctorFinding>>,
    /// Checks whose findings were capped — a `fleet` remainder note — so a
    /// clean answer about any one path is not provable.
    capped: BTreeSet<CheckId>,
}

fn attribute<'r>(
    slice: &RegistrySlice,
    base: &str,
    slices: &SliceSet,
    report: &'r DoctorReport,
    origin: Option<&str>,
) -> Attributed<'r> {
    let mut out = Attributed::default();
    let origin_ok = |o: &str| origin.is_none_or(|want| want == o);
    for f in &report.findings {
        match f.check {
            CheckId::SliceSync | CheckId::SliceParse | CheckId::BudgetExceeded => {
                // `origin/producer`, or the bare producer name.
                let (o, name) = match f.subject.split_once('/') {
                    Some((o, n)) => (Some(o), n),
                    None => (None, f.subject.as_str()),
                };
                if name == slice.name && o.is_none_or(origin_ok) {
                    out.producer.entry(f.check).or_default().push(f);
                }
            }
            CheckId::DescribeTotality if f.subject == slice.name => {
                out.producer.entry(f.check).or_default().push(f);
            }
            CheckId::SchemaDrift => {
                out.by_path
                    .entry((f.check, f.subject.clone()))
                    .or_default()
                    .push(f);
            }
            CheckId::RateOverDeclared => {
                let path = match &slice.service_origin {
                    Some(s) => f.subject.strip_prefix(&format!("{}/", s.token())),
                    None => f.subject.strip_prefix(&format!("{}/", slice.name)),
                };
                if let Some(path) = path {
                    out.by_path
                        .entry((f.check, path.to_string()))
                        .or_default()
                        .push(f);
                }
            }
            CheckId::CardinalityOverDeclared => {
                // Info: `producer/path` (the rest-var exemption). Warning:
                // `origin/producer/path`, or `origin/path` under a service.
                let path = if f.severity == DoctorSeverity::Info {
                    f.subject.strip_prefix(&format!("{}/", slice.name))
                } else {
                    f.subject.split_once('/').and_then(|(o, rest)| {
                        if !origin_ok(o) {
                            return None;
                        }
                        match &slice.service_origin {
                            Some(s) if s.token() == o => Some(rest),
                            Some(_) => None,
                            None => rest.strip_prefix(&format!("{}/", slice.name)),
                        }
                    })
                };
                if let Some(path) = path {
                    out.by_path
                        .entry((f.check, path.to_string()))
                        .or_default()
                        .push(f);
                } else if f.subject == "fleet" {
                    out.capped.insert(f.check);
                }
            }
            CheckId::QosObservedMismatch
            | CheckId::PayloadInvalid
            | CheckId::PayloadUndecodable
            | CheckId::KindMismatch
            | CheckId::UnregisteredTraffic
            | CheckId::StaleState => {
                if f.subject == "fleet" {
                    out.capped.insert(f.check);
                    continue;
                }
                let mut facts = KeyFacts::project(base, &f.subject);
                facts.resolve(slices);
                let KeyShape::V1(v) = &facts.shape else {
                    continue;
                };
                if crate::judge::common::producer_of(&facts, Some(slices)).as_deref()
                    != Some(slice.name.as_str())
                    || !origin_ok(&v.origin)
                {
                    continue;
                }
                match &facts.registration {
                    Registration::Registered(sf) => {
                        out.by_path
                            .entry((f.check, sf.path.clone()))
                            .or_default()
                            .push(f);
                    }
                    _ if f.check == CheckId::UnregisteredTraffic => {
                        out.unregistered
                            .insert(v.subject.join("/"), (v.class.clone(), f));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    out
}

/// Map one scoped doctor run onto assertions (#222). Pure: the report, the
/// internals and the capability claims in, assertions out — so every row
/// of the projection table is testable without a bus.
fn project_conform(
    slice: &RegistrySlice,
    base: &str,
    slices: &SliceSet,
    report: &DoctorReport,
    internals: &DoctorInternals,
    origin: Option<&str>,
    caps: &BTreeMap<String, Capabilities>,
) -> Vec<Assertion> {
    let a = attribute(slice, base, slices, report, origin);
    let origin_ok = |o: &str| origin.is_none_or(|want| want == o);
    let producer = slice.name.as_str();
    let mut out = Vec::new();
    let evidence_of = |fs: &[&DoctorFinding]| {
        fs.iter()
            .map(|f| format!("{}: {}", f.subject, f.evidence))
            .collect::<Vec<_>>()
            .join("; ")
    };
    let assertion =
        |id: String, subject: String, state, evidence: String, citation: &str| Assertion {
            id,
            subject,
            state,
            evidence,
            citation: Some(citation.to_string()),
            exempt: None,
        };

    // slice-sync: served versus the suite's registry, per origin that
    // answered introspect.
    let introspected: Vec<&str> = internals
        .introspected
        .iter()
        .filter(|(o, p)| p == producer && origin_ok(o))
        .map(|(o, _)| o.as_str())
        .collect();
    let sync = a.producer.get(&CheckId::SliceSync).into_iter().flatten();
    let parse = a.producer.get(&CheckId::SliceParse).into_iter().flatten();
    let bad: Vec<&DoctorFinding> = sync.chain(parse).copied().collect();
    out.push(if !bad.is_empty() {
        assertion(
            "slice-sync".into(),
            producer.into(),
            AssertionState::NotMet,
            evidence_of(&bad),
            "RFC 08 §6",
        )
    } else if introspected.is_empty() {
        assertion(
            "slice-sync".into(),
            producer.into(),
            AssertionState::Unknowable {
                reason: "no introspect answered, so there is no served slice to diff".into(),
            },
            "not diffed".into(),
            "RFC 08 §6",
        )
    } else {
        assertion(
            "slice-sync".into(),
            producer.into(),
            AssertionState::Met,
            format!(
                "the served slice agrees with the loaded registry on {}",
                introspected.join(", ")
            ),
            "RFC 08 §6",
        )
    });

    // describe-totality: every type the slice names has a served shape.
    let described = internals.described.contains(producer);
    let totality = a.producer.get(&CheckId::DescribeTotality);
    out.push(match (described, totality) {
        (_, Some(fs)) => assertion(
            "describe-totality".into(),
            producer.into(),
            AssertionState::NotMet,
            evidence_of(fs),
            "RFC 08 §7",
        ),
        (false, None) => assertion(
            "describe-totality".into(),
            producer.into(),
            AssertionState::Unknowable {
                reason: "the producer serves no describe (a SHOULD) — its payload shapes \
                         are not stated"
                    .into(),
            },
            "no describe answered".into(),
            "RFC 08 §7",
        ),
        (true, None) => assertion(
            "describe-totality".into(),
            producer.into(),
            AssertionState::Met,
            "describe covers every type the slice names".into(),
            "RFC 08 §7",
        ),
    });

    // schema-drift, per declared type: one identity across the hosts.
    let types: BTreeSet<&str> = slice
        .subjects
        .iter()
        .map(|s| s.type_name.as_str())
        .filter(|t| !t.is_empty())
        .collect();
    for ty in types {
        let id = format!("schema-drift/{ty}");
        let subject = format!("type/{ty}");
        let fs = a
            .by_path
            .get(&(CheckId::SchemaDrift, ty.to_string()))
            .map(Vec::as_slice)
            .unwrap_or_default();
        let (state, evidence) = if fs.iter().any(|f| f.severity == DoctorSeverity::Error) {
            (AssertionState::NotMet, evidence_of(fs))
        } else if let Some(f) = fs.first() {
            (
                AssertionState::Unknowable {
                    reason: "a host served no schema identity, so agreement cannot be \
                             judged"
                        .into(),
                },
                format!("{}: {}", f.subject, f.evidence),
            )
        } else if described {
            (
                AssertionState::Met,
                "one schema identity across the hosts that serve it".into(),
            )
        } else {
            (
                AssertionState::Unknowable {
                    reason: "no describe served, so there is no identity to compare".into(),
                },
                "not compared".into(),
            )
        };
        out.push(assertion(id, subject, state, evidence, "RFC 08 §7"));
    }

    // --- the listen window ----------------------------------------------
    if let Some(obs) = &report.observation {
        let drops = if obs.dropped > 0 {
            format!(
                " ({} sample(s) dropped in the window — counts are lower bounds, RFC 13 §3 \
                 O6)",
                obs.dropped
            )
        } else {
            String::new()
        };
        for s in &slice.subjects {
            let class = s.class.token();
            let subject = format!("{class}/{}", s.path);
            let seen = internals
                .seen
                .get(&(producer.to_string(), s.path.clone()))
                .map(|f| {
                    let origins: Vec<&str> = f
                        .origins
                        .iter()
                        .map(String::as_str)
                        .filter(|o| origin_ok(o))
                        .collect();
                    (origins, f.samples)
                })
                .filter(|(origins, _)| !origins.is_empty());
            let Some((origins, samples)) = seen else {
                out.push(unseen(s, &subject, obs.window_s, &drops, caps));
                continue;
            };
            out.push(assertion(
                format!("observed/{}", s.path),
                subject.clone(),
                AssertionState::Met,
                format!(
                    "{samples} sample(s) in {:.0}s from {}{drops}",
                    obs.window_s,
                    origins.join(", ")
                ),
                "RFC 13 §3",
            ));
            for check in LISTEN_CHECKS {
                if let Some(x) = listen_assertion(check, s, &subject, &a, described, obs.window_s) {
                    out.push(x);
                }
            }
        }
        // Traffic the slice does not declare, under this producer.
        if a.unregistered.is_empty() {
            out.push(assertion(
                "unregistered-traffic".into(),
                producer.into(),
                if a.capped.contains(&CheckId::UnregisteredTraffic) {
                    AssertionState::Unknowable {
                        reason: "the doctor capped its unregistered-traffic findings, so \
                                 none can be attributed"
                            .into(),
                    }
                } else {
                    AssertionState::Met
                },
                format!(
                    "no sample on a subject the slice does not declare in {:.0}s",
                    obs.window_s
                ),
                "RFC 08 §2",
            ));
        }
        for (tail, (class, f)) in &a.unregistered {
            out.push(assertion(
                format!("unregistered-traffic/{tail}"),
                format!("{class}/{tail}"),
                AssertionState::NotMet,
                format!("{}: {}", f.subject, f.evidence),
                "RFC 08 §2",
            ));
        }
    }

    // --- deep: freshness and the declared budget ------------------------
    if report.deep {
        for s in &slice.subjects {
            if s.ttl_s.is_none() || !s.class.is(&zenkey::Class::State) {
                continue;
            }
            let fs = a
                .by_path
                .get(&(CheckId::StaleState, s.path.clone()))
                .map(Vec::as_slice)
                .unwrap_or_default();
            let read = internals
                .fresh_read
                .get(&(producer.to_string(), s.path.clone()))
                .copied()
                .unwrap_or(0);
            let (state, evidence) = if !fs.is_empty() {
                (AssertionState::NotMet, evidence_of(fs))
            } else if a.capped.contains(&CheckId::StaleState) {
                (
                    AssertionState::Unknowable {
                        reason: "the doctor capped its stale-state findings".into(),
                    },
                    format!("{read} sample(s) read"),
                )
            } else if read > 0 {
                (
                    AssertionState::Met,
                    format!(
                        "{read} stored sample(s), each within ttl {}s",
                        s.ttl_s.unwrap_or(0)
                    ),
                )
            } else {
                (
                    AssertionState::Unknowable {
                        reason: "no stored state sample answered the snapshot".into(),
                    },
                    "nothing to judge".into(),
                )
            };
            out.push(assertion(
                format!("stale-state/{}", s.path),
                format!("state/{}", s.path),
                state,
                evidence,
                "RFC 04 §1.2",
            ));
        }
        if slice.budget.is_some() {
            let fs = a
                .producer
                .get(&CheckId::BudgetExceeded)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let (state, evidence) = if fs.iter().any(|f| f.severity == DoctorSeverity::Error) {
                (AssertionState::NotMet, evidence_of(fs))
            } else if !fs.is_empty() {
                (
                    AssertionState::Unknowable {
                        reason: "this producer does not say how big it is".into(),
                    },
                    evidence_of(fs),
                )
            } else {
                (
                    AssertionState::Met,
                    "self_stats within the declared [budget]".into(),
                )
            };
            out.push(assertion(
                "budget".into(),
                producer.into(),
                state,
                evidence,
                "RFC 08 §2",
            ));
        }
    }
    out
}

/// A declared subject the window did not see. Unknowable — a window proves
/// presence, never absence (RFC 13 §3) — unless it is conditional, when it
/// is exempt and says so; and not exempt after all when the registration
/// document claims every capability its `when` names.
fn unseen(
    s: &zenkey::slice::SubjectDecl,
    subject: &str,
    window_s: f64,
    drops: &str,
    caps: &BTreeMap<String, Capabilities>,
) -> Assertion {
    let id = format!("observed/{}", s.path);
    let evidence = format!("not seen in {window_s:.0}s{drops}");
    let citation = Some("RFC 13 §3".to_string());
    let absent = || AssertionState::Unknowable {
        reason: "a window proves presence, never absence".into(),
    };
    let Some(when) = s.when.as_deref() else {
        return Assertion {
            id,
            subject: subject.to_string(),
            state: absent(),
            evidence,
            citation,
            exempt: None,
        };
    };
    let tokens: Vec<String> = when.iter().map(Predicate::token).collect();
    let device = device_of(&s.path);
    let needed: Vec<&str> = when
        .iter()
        .filter(|p| p.kind.is(&PredicateKind::Capability))
        .map(|p| p.name.as_str())
        .collect();
    let claimed_on: Vec<&str> = if could_contradict(when) {
        caps.iter()
            .filter_map(|(o, c)| match c {
                Capabilities::Served(map) if needed.iter().all(|n| claims(map, device, n)) => {
                    Some(o.as_str())
                }
                _ => None,
            })
            .collect()
    } else {
        Vec::new()
    };
    if !claimed_on.is_empty() {
        return Assertion {
            id,
            subject: subject.to_string(),
            state: absent(),
            evidence: format!(
                "{evidence}; the registration document on {} claims {} — the gate does \
                 not excuse the silence",
                claimed_on.join(", "),
                needed.join(", ")
            ),
            citation,
            exempt: None,
        };
    }
    Assertion {
        id,
        subject: subject.to_string(),
        state: AssertionState::Met,
        evidence: format!("{evidence}; conditional — an unobserved `when` subject is exempt"),
        citation,
        exempt: Some(format!("when: {}", tokens.join(", "))),
    }
}

/// One listen check over one seen subject, when the check applies to it —
/// the declaration it holds the wire to is present.
fn listen_assertion(
    check: CheckId,
    s: &zenkey::slice::SubjectDecl,
    subject: &str,
    a: &Attributed<'_>,
    described: bool,
    window_s: f64,
) -> Option<Assertion> {
    let findings = |c: CheckId| {
        a.by_path
            .get(&(c, s.path.clone()))
            .map(Vec::as_slice)
            .unwrap_or_default()
    };
    let (id, clean, citation): (String, String, &str) = match check {
        CheckId::QosObservedMismatch => {
            let q = s.qos.as_ref().and_then(Declared::known)?;
            (
                format!("qos-observed-mismatch/{}", s.path),
                format!("every sample rode the declared {}", q.name()),
                "RFC 04 §3",
            )
        }
        // The two payload checks answer one question, "does the payload
        // conform?", so they share one assertion.
        CheckId::PayloadInvalid => (
            format!("payload/{}", s.path),
            "the samples judged decoded and validated against the served schema".into(),
            "RFC 08 §7",
        ),
        CheckId::PayloadUndecodable => return None,
        CheckId::KindMismatch => {
            let k = s.kind.as_ref().filter(|k| k.known().is_some())?;
            (
                format!("kind-mismatch/{}", s.path),
                format!("every judged sample is a `{}`", k.token()),
                "RFC 08 §2",
            )
        }
        CheckId::RateOverDeclared => {
            let rate = s
                .rate
                .as_ref()
                .filter(|_| s.class.is(&zenkey::Class::Events))?;
            (
                format!("rate-over-declared/{}", s.path),
                format!("within the declared `{}` cap", rate.token()),
                "RFC 04 §1.3",
            )
        }
        CheckId::CardinalityOverDeclared => {
            if !s.path.contains('{') {
                return None;
            }
            (
                format!("cardinality-over-declared/{}", s.path),
                "within the declared cardinality".into(),
                "RFC 08 §2",
            )
        }
        _ => return None,
    };
    let mut fs: Vec<&DoctorFinding> = findings(check).to_vec();
    if check == CheckId::PayloadInvalid {
        fs.extend(findings(CheckId::PayloadUndecodable));
    }
    let capped = a.capped.contains(&check)
        || (check == CheckId::PayloadInvalid && a.capped.contains(&CheckId::PayloadUndecodable));
    let evidence = |fs: &[&DoctorFinding]| {
        fs.iter()
            .map(|f| format!("{}: {}: {}", f.check, f.subject, f.evidence))
            .collect::<Vec<_>>()
            .join("; ")
    };
    let mut exempt = None;
    let (state, evidence) = if fs.iter().any(|f| {
        f.severity == DoctorSeverity::Error
            || (f.severity == DoctorSeverity::Warning && check != CheckId::KindMismatch)
    }) {
        (AssertionState::NotMet, evidence(&fs))
    } else if check == CheckId::CardinalityOverDeclared && !fs.is_empty() {
        // Info: the rest-variable exemption, stated by the doctor.
        exempt = Some("rest-variable".to_string());
        (AssertionState::Met, evidence(&fs))
    } else if !fs.is_empty() {
        (
            AssertionState::Unknowable {
                reason: "the payloads could not be decoded, so the declared kind is \
                         unobservable for them"
                    .into(),
            },
            evidence(&fs),
        )
    } else if capped {
        (
            AssertionState::Unknowable {
                reason: format!(
                    "the doctor capped its {check} findings, so a clean answer about one \
                     subject is not provable"
                ),
            },
            "not attributable".into(),
        )
    } else if check == CheckId::PayloadInvalid && !described {
        (
            AssertionState::Unknowable {
                reason: "the producer serves no describe, so payloads were not validated".into(),
            },
            "not validated".into(),
        )
    } else if check == CheckId::RateOverDeclared && window_s > 3600.0 {
        (
            AssertionState::Unknowable {
                reason: "over an hour, one window cannot prove an hourly cap exceeded".into(),
            },
            "not judged".into(),
        )
    } else {
        (AssertionState::Met, clean)
    };
    Some(Assertion {
        id,
        subject: subject.to_string(),
        state,
        evidence,
        citation: Some(citation.to_string()),
        exempt,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preds(tokens: &[&str]) -> Vec<Predicate> {
        tokens.iter().map(|t| Predicate::parse(t)).collect()
    }

    fn gated() -> Probe {
        Probe::Error {
            name: ReservedError::Gated.name().into(),
            message: "disabled here".into(),
        }
    }

    fn slice() -> RegistrySlice {
        zenkey::parse_slice(
            r#"
[registry]
version = "1.0"
app = "t"
convention = 1
[producer]
name = "demo"
[[error]]
name = "restart-required"
"#,
        )
        .expect("slice")
    }

    fn served(pairs: &[(&str, &[&str])]) -> Capabilities {
        Capabilities::Served(
            pairs
                .iter()
                .map(|(d, ns)| (d.to_string(), ns.iter().map(|n| n.to_string()).collect()))
                .collect(),
        )
    }

    /// RFC 13 §3's `when` rows, one per case: exempt with the predicates,
    /// a finding without `when`, and the kind binding judged both ways.
    #[test]
    fn the_gate_rows_of_rfc_13_section_3() {
        let s = slice();
        let unconsulted = Capabilities::Unconsulted;
        let when = preds(&["config:collect.dns"]);
        let a = judge_reply(Some(&when), None, &gated(), &unconsulted, &s);
        assert!(a.state.is_met());
        assert_eq!(a.exempt.as_deref(), Some("when: config:collect.dns"));

        let a = judge_reply(None, None, &gated(), &unconsulted, &s);
        assert!(a.state.is_not_met(), "{a:?}");
        assert!(a.evidence.contains("RFC 08 §6.1"));

        // `unsupported` with no `feature:` predicate: the binding is wrong.
        let unsupported = Probe::Error {
            name: ReservedError::Unsupported.name().into(),
            message: "".into(),
        };
        let a = judge_reply(Some(&when), None, &unsupported, &unconsulted, &s);
        assert!(a.state.is_not_met(), "{a:?}");
        // `gated` from a feature-only entry: wrong the other way.
        let feature = preds(&["feature:ebpf"]);
        let a = judge_reply(Some(&feature), None, &gated(), &unconsulted, &s);
        assert!(a.state.is_not_met(), "{a:?}");
        let a = judge_reply(Some(&feature), None, &unsupported, &unconsulted, &s);
        assert!(a.state.is_met() && a.exempt.is_some());
        // A kind this build does not know: conditional all the same, binding
        // not asked.
        let foreign = preds(&["hardware:gpu"]);
        let a = judge_reply(Some(&foreign), None, &gated(), &unconsulted, &s);
        assert!(a.state.is_met() && a.exempt.is_some(), "{a:?}");
        assert!(a.evidence.contains("not asked"));
    }

    /// v1.41: the device's claim against the procedure's refusal.
    #[test]
    fn a_gated_reply_is_held_to_the_capabilities_the_device_claims() {
        let s = slice();
        let when = preds(&["feature:modem", "capability:rssi"]);
        // rf0 claims rssi: the procedure says it cannot, the device says it can.
        let caps = served(&[("rf0", &["sdu", "rssi"]), ("sat0", &["sdu"])]);
        let a = judge_reply(Some(&when), Some("rf0"), &gated(), &caps, &s);
        assert!(a.state.is_not_met(), "{a:?}");
        // sat0 does not: the gate is honest.
        let a = judge_reply(Some(&when), Some("sat0"), &gated(), &caps, &s);
        assert!(a.state.is_met() && a.exempt.is_some(), "{a:?}");
        // The producer's own claim under `*` holds for every device.
        let caps = served(&[("*", &["capability:rssi"])]);
        let a = judge_reply(Some(&when), None, &gated(), &caps, &s);
        assert!(a.state.is_not_met(), "{a:?}");
        // Not served: the plain exemption, and the evidence says why.
        let absent = Capabilities::Absent("not asked: no registration document".into());
        let a = judge_reply(Some(&when), Some("rf0"), &gated(), &absent, &s);
        assert!(a.state.is_met() && a.exempt.is_some());
        assert!(a.evidence.contains("not asked"), "{}", a.evidence);
        // A `config:` predicate may be the false one: the claim cannot
        // contradict, so it is not consulted.
        assert!(!could_contradict(&preds(&["config:x", "capability:rssi"])));
        assert!(could_contradict(&when));
    }

    /// Every other reply is a reply: a value, `invalid-args`, a reserved
    /// refusal, a registered name — met; a failed call is unknowable.
    #[test]
    fn a_refusal_is_a_reply() {
        let s = slice();
        let c = Capabilities::Unconsulted;
        for reply in [
            Probe::Value,
            Probe::Error {
                name: "error/invalid-args".into(),
                message: "".into(),
            },
            Probe::Error {
                name: "error/busy".into(),
                message: "".into(),
            },
            Probe::Error {
                name: "error/fanout-forbidden".into(),
                message: "".into(),
            },
            Probe::Error {
                name: "error/demo/restart-required".into(),
                message: "".into(),
            },
        ] {
            let a = judge_reply(None, None, &reply, &c, &s);
            assert!(a.state.is_met(), "{reply:?} → {a:?}");
            assert!(a.exempt.is_none());
        }
        let a = judge_reply(
            None,
            None,
            &Probe::Error {
                name: "error/demo/restart-required".into(),
                message: "".into(),
            },
            &c,
            &s,
        );
        assert!(a.evidence.contains("registered"), "{}", a.evidence);
        let a = judge_reply(None, None, &Probe::Failed("boom".into()), &c, &s);
        assert!(a.state.is_unknowable());
    }

    /// An `introspect` that answered unreadably is neither silence nor a
    /// slice (#491): not met — the reply broke RFC 08 §6 — and the evidence
    /// names what it declared. Folded beside a readable origin, it is the
    /// worse of the two.
    #[test]
    fn an_unreadable_introspect_is_not_met_and_names_its_encoding() {
        let s = slice();
        let c = Capabilities::Unconsulted;
        let u = Probe::Unreadable(crate::report::UnreadableSlice::new(
            Some("application/json".into()),
            &"unreadable registry slice: the reply declares encoding \"application/json\"",
        ));
        let a = judge_reply(None, None, &u, &c, &s);
        assert!(a.state.is_not_met(), "{a:?}");
        assert!(
            a.evidence
                .starts_with("introspect answered, slice unreadable (`application/json`: "),
            "{}",
            a.evidence
        );
        let inst = Instance {
            origin: "h-3fa9c2d41b7e".into(),
            chunk: "demo".into(),
            rostered: true,
        };
        let t = Duration::from_millis(500);
        let both = judge_probe(None, None, &[Probe::Value, u], &c, &inst, t, &s);
        assert!(both.state.is_not_met(), "{both:?}");
    }

    /// Silence is judged against the roster (RFC 13 §2).
    #[test]
    fn silence_is_a_finding_only_where_the_roster_shows_alive() {
        let s = slice();
        let t = Duration::from_millis(500);
        let c = Capabilities::Unconsulted;
        let alive = Instance {
            origin: "h-3fa9c2d41b7e".into(),
            chunk: "demo".into(),
            rostered: true,
        };
        let stranger = Instance {
            rostered: false,
            ..alive.clone()
        };
        assert!(
            judge_probe(None, None, &[], &c, &alive, t, &s)
                .state
                .is_not_met()
        );
        assert!(
            judge_probe(None, None, &[], &c, &stranger, t, &s)
                .state
                .is_unknowable()
        );
        // And nobody at all is unknowable, not a pass.
        let a = fold(
            "procedure/introspect",
            "@rpc/introspect",
            vec![],
            "RFC 08 §6",
        );
        assert!(a.state.is_unknowable());
    }

    /// The roster read: instance suffixes share their slice, `--origin`
    /// narrows, and an unrostered `--origin` is kept — marked, so its
    /// silence is unknowable.
    #[test]
    fn instances_come_off_the_roster() {
        let s = slice();
        let roster: BTreeMap<String, Vec<String>> = [
            (
                "h-aaaaaaaaaaaa".to_string(),
                vec!["demo".to_string(), "other".to_string()],
            ),
            ("h-bbbbbbbbbbbb".to_string(), vec!["demo-2".to_string()]),
        ]
        .into();
        let all = instances_of(&roster, &s, None);
        assert_eq!(
            all.iter().map(|i| i.chunk.as_str()).collect::<Vec<_>>(),
            ["demo", "demo-2"]
        );
        let one = instances_of(&roster, &s, Some("h-bbbbbbbbbbbb"));
        assert_eq!(one.len(), 1);
        assert!(one[0].rostered);
        let stranger = instances_of(&roster, &s, Some("h-cccccccccccc"));
        assert_eq!(stranger.len(), 1);
        assert!(!stranger[0].rostered);
    }

    /// The listen projection's rows (RFC 13 §3): seen is met, with each
    /// check the declaration asks for; an unseen plain subject is
    /// unknowable; an unseen `when` subject is exempt — unless the device
    /// claims its capabilities; a rest-var family is exempt from its bound.
    #[test]
    fn the_listen_projection_rows() {
        let slice = zenkey::parse_slice(
            r#"
[registry]
version = "1.0"
app = "t"
convention = 1
[producer]
name = "demo"
[[subject]]
path = "health"
class = "state"
type = "Health"
qos = "transition"
[[subject]]
path = "peers/{id...}"
class = "state"
type = "Health"
[[subject]]
path = "boom"
class = "events"
type = "Health"
[[subject]]
path = "rf0/rssi"
class = "telemetry"
type = "Health"
when = ["capability:rssi"]
[[subject]]
path = "wifi"
class = "telemetry"
type = "Health"
when = ["config:wifi"]
"#,
        )
        .expect("slice");
        let set = SliceSet::from_slices(vec![slice.clone()]);
        let origin = "h-3fa9c2d41b7e";
        let report = DoctorReport {
            findings: vec![DoctorFinding {
                severity: DoctorSeverity::Info,
                check: CheckId::CardinalityOverDeclared,
                subject: "demo/peers/{id...}".into(),
                evidence: "exempt: rest-variable".into(),
                citation: None,
            }],
            synced: crate::report::Asked::Asked(vec![]),
            introspect_answered: 1,
            live_producers: 1,
            describe_served: 1,
            describe_missing: 0,
            routers: 0,
            router_version: None,
            deep: false,
            observation: Some(crate::report::ObservationSummary {
                window_s: 2.0,
                scopes: vec!["v1/*/state/**".into()],
                samples: 9,
                keys_seen: 2,
                dropped: 0,
                synthetic_marked: 0,
                field_paths_dropped: 0,
                facts_evicted: 0,
            }),
        };
        let mut internals = DoctorInternals::default();
        internals.described.insert("demo".into());
        internals
            .introspected
            .insert((origin.to_string(), "demo".to_string()));
        for path in ["health", "peers/{id...}"] {
            let seen = internals
                .seen
                .entry(("demo".to_string(), path.to_string()))
                .or_default();
            seen.origins.insert(origin.into());
            seen.samples = 4;
        }
        let caps: BTreeMap<String, Capabilities> =
            [(origin.to_string(), served(&[("rf0", &["rssi"])]))].into();

        let out = project_conform(&slice, "", &set, &report, &internals, None, &caps);
        let get = |id: &str| {
            out.iter()
                .find(|a| a.id == id)
                .unwrap_or_else(|| panic!("no {id}: {out:#?}"))
        };
        for id in [
            "slice-sync",
            "describe-totality",
            "schema-drift/Health",
            "observed/health",
            "qos-observed-mismatch/health",
            "payload/health",
            "observed/peers/{id...}",
            "unregistered-traffic",
        ] {
            assert!(get(id).state.is_met(), "{id}: {:?}", get(id));
            assert!(get(id).exempt.is_none(), "{id}");
        }
        let bound = get("cardinality-over-declared/peers/{id...}");
        assert!(bound.state.is_met());
        assert_eq!(bound.exempt.as_deref(), Some("rest-variable"));
        assert!(get("observed/boom").state.is_unknowable());
        assert!(
            get("observed/rf0/rssi").state.is_unknowable(),
            "the device claims rssi"
        );
        let wifi = get("observed/wifi");
        assert!(wifi.state.is_met());
        assert_eq!(wifi.exempt.as_deref(), Some("when: config:wifi"));
        assert!(!out.iter().any(|a| a.state.is_not_met()), "{out:#?}");
    }
}
