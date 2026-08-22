//! `zenctl gen` (#162) — the registry-driven pattern generator.
//!
//! The opposite artifact of the spray demo: spray is deliberately hardcoded
//! adversarial weirdness; `gen` reads a registry and produces **conforming**
//! traffic — every declared subject of a producer, schema-synthesized
//! payloads ([`crate::synth`]), declared QoS, class-conscious rates. It is a
//! mock producer for testing consumers, not a load cannon.
//!
//! Everything rides the existing seams: keys assemble from the declared
//! patterns, bodies encode through [`SchemaStore::encode`] (the same
//! validating ladder `topic pub` writes through), publications are declared
//! (P7), and every sample carries the RFC 09 §5.3 synthetic marker (v1.19) —
//! someone's `doctor --listen-for` must be able to tell this traffic from real.

use std::time::Duration;

use anyhow::{Result, anyhow};
use serde::Serialize;
use zenkey::grammar::with_base;
use zenkey::pattern::{PatternChunk, SubjectPattern};
use zenkey::qos::QosProfile;
use zenkey::schema::{SchemaSet, TypeSchema};
use zenoh::Session;

use crate::decode::SchemaStore;
use crate::registry::SliceSet;
use crate::synth::Synth;

/// The RFC 09 §5.3 marker (v1.19): every synthetic sample's attachment.
pub fn synthetic_marker(tool: &str, origin: &str, fault: Option<&str>) -> Vec<u8> {
    let mut obj = serde_json::json!({
        "synthetic": true,
        "tool": tool,
        "origin": origin,
    });
    if let Some(kind) = fault {
        obj["fault"] = kind.into();
    }
    serde_json::to_vec(&obj).expect("the marker serializes")
}

/// A single deliberate deviation from a known-valid synthesized sample (#163).
///
/// Fault injection is a *mode of the generator*, not a sibling tool: it reuses
/// the whole registry walk, synthesis, scheduling, and guard machinery, then
/// perturbs one dimension of the output **after synthesis** — so the delta
/// from valid is always known, printable ([`Fault::delta`]), and stamped into
/// the marker (`"fault": "<kind>"`, RFC 09 §5.3). The point is
/// consumer-robustness testing: a consumer that crashes on a truncated payload
/// fails RFC 09 §5.1 O1's spirit — a non-conforming sample is a fact to
/// report, not an error to die on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Fault {
    /// Cut the encoded payload to half its bytes — a partial frame the
    /// decoder meets mid-value.
    Truncate,
    /// Replace the body with a JSON value of the wrong shape for the declared
    /// type (a bare string where a structured type is declared).
    WrongType,
    /// Add an undeclared field to the (JSON) body — the extra key a strict
    /// schema must reject or a lenient one must ignore, never choke on.
    ExtraField,
    /// Publish on a key that matches no registered subject (a trailing chunk
    /// the registry never declared).
    UnregisteredKey,
    /// Publish under a QoS profile other than the subject's declared one
    /// (RFC 04 §3) — the observed-vs-declared mismatch a doctor listen flags.
    WrongQos,
    /// Publish with no wire `Encoding` set, though the subject declares one —
    /// a consumer keyed on the encoding meets a blank.
    MissingEncoding,
    /// Publish a state sample carrying no HLC timestamp: LWW cannot order it
    /// (RFC 04 §4), and freshness is unjudgeable.
    Unstamped,
}

impl Fault {
    /// The kebab-case kind name — the CLI token and the marker's `"fault"`
    /// value.
    pub fn as_str(self) -> &'static str {
        match self {
            Fault::Truncate => "truncate",
            Fault::WrongType => "wrong-type",
            Fault::ExtraField => "extra-field",
            Fault::UnregisteredKey => "unregistered-key",
            Fault::WrongQos => "wrong-qos",
            Fault::MissingEncoding => "missing-encoding",
            Fault::Unstamped => "unstamped",
        }
    }

    /// Every kind, for a CLI error message and the round-trip test.
    pub const ALL: [Fault; 7] = [
        Fault::Truncate,
        Fault::WrongType,
        Fault::ExtraField,
        Fault::UnregisteredKey,
        Fault::WrongQos,
        Fault::MissingEncoding,
        Fault::Unstamped,
    ];

    /// Parse one kind, naming the vocabulary on a miss (spray's decline
    /// precedent: an unknown kind is refused, never silently ignored).
    pub fn parse(s: &str) -> Result<Fault> {
        Fault::ALL
            .into_iter()
            .find(|f| f.as_str() == s)
            .ok_or_else(|| {
                let known = Fault::ALL.map(Fault::as_str).join(", ");
                anyhow!("unknown fault kind {s:?} — known kinds: {known}")
            })
    }

    /// Perturb the wire key: only [`Fault::UnregisteredKey`] moves it (a
    /// trailing chunk the registry never declared). Every other kind leaves
    /// the declared key untouched and perturbs a different dimension.
    fn perturb_key(self, key: &str) -> String {
        match self {
            Fault::UnregisteredKey => format!("{key}/unregistered"),
            _ => key.to_string(),
        }
    }

    /// Perturb the QoS profile: only [`Fault::WrongQos`] swaps it, to a
    /// profile deliberately unlike the declared one.
    fn perturb_qos(self, declared: QosProfile) -> QosProfile {
        match self {
            Fault::WrongQos if declared == QosProfile::Sampled => QosProfile::Transition,
            Fault::WrongQos => QosProfile::Sampled,
            _ => declared,
        }
    }

    /// Whether this fault drops the declared wire encoding.
    fn drops_encoding(self) -> bool {
        matches!(self, Fault::MissingEncoding)
    }

    /// Whether this fault omits the HLC timestamp the valid path stamps.
    fn drops_timestamp(self) -> bool {
        matches!(self, Fault::Unstamped)
    }

    /// Perturb the encoded body bytes, post-synthesis and post-encode — so
    /// the deviation bypasses the validating encoder that produced the valid
    /// bytes (that is the whole point: near-valid traffic that violates on
    /// the wire). Key/QoS/encoding/timestamp faults leave the body alone.
    fn perturb_body(self, bytes: Vec<u8>) -> Vec<u8> {
        match self {
            Fault::Truncate => {
                let n = bytes.len() / 2;
                let mut out = bytes;
                out.truncate(n);
                out
            }
            Fault::WrongType => {
                // A bare JSON string where a structured type is declared —
                // built directly, never through the schema-validating encoder.
                serde_json::to_vec(&serde_json::Value::String("fault:wrong-type".into()))
                    .expect("a string serializes")
            }
            Fault::ExtraField => match serde_json::from_slice::<serde_json::Value>(&bytes) {
                Ok(serde_json::Value::Object(mut m)) => {
                    m.insert("_fault".into(), serde_json::Value::Bool(true));
                    serde_json::to_vec(&serde_json::Value::Object(m)).expect("object serializes")
                }
                Ok(other) => {
                    // Not an object: wrap it so the extra key still rides.
                    let wrapped = serde_json::json!({ "_orig": other, "_fault": true });
                    serde_json::to_vec(&wrapped).expect("object serializes")
                }
                Err(_) => {
                    // Non-JSON body (cdr/protobuf): append the marker bytes —
                    // still an undeclared trailer the decoder must survive.
                    let mut out = bytes;
                    out.extend_from_slice(b"_fault");
                    out
                }
            },
            _ => bytes,
        }
    }

    /// The printable per-key delta from valid — what the plan states before
    /// anything touches the bus (honesty: the tool says what it will do).
    /// `valid` is the entry as synthesized, before this fault's perturbation.
    fn delta(self, valid: &GenPlanEntry) -> String {
        match self {
            Fault::Truncate => {
                "payload truncated to half its encoded bytes — a partial frame".into()
            }
            Fault::WrongType => format!(
                "body replaced with a JSON string where {} is declared",
                valid.type_name
            ),
            Fault::ExtraField => "an undeclared `_fault` field added to the body".into(),
            Fault::UnregisteredKey => format!(
                "key → {} (an unregistered subject; RFC 09 §5.1 O1: a fact to report)",
                self.perturb_key(&valid.key)
            ),
            Fault::WrongQos => format!(
                "qos {} → {} (declared profile not honoured, RFC 04 §3)",
                valid.qos,
                self.perturb_qos(QosProfile::from_name(&valid.qos).unwrap_or(QosProfile::Sampled))
                    .name()
            ),
            Fault::MissingEncoding => match &valid.encoding {
                Some(e) => format!("wire encoding {e} omitted"),
                None => "no wire encoding set (none was declared either)".into(),
            },
            Fault::Unstamped => {
                "no HLC timestamp — state LWW cannot order it (RFC 04 §4)".into()
            }
        }
    }
}

/// The send-timing shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenPattern {
    /// Fixed interval.
    Steady,
    /// Interval jittered ±30%, seeded — reproducible irregularity.
    Jitter,
    /// The per-second budget sent at once, then a pause.
    Burst,
    /// Rate climbs linearly from ~0 to the full rate over the duration.
    Ramp,
}

/// What to generate.
#[derive(Debug, Clone)]
pub struct GenSpec {
    /// The origin chunk the generated keys claim (`h-…`). Stated, printed,
    /// and stamped into the marker — impersonation is the feature, and the
    /// marker is what keeps it honest.
    pub origin: String,
    /// Only this producer's subjects (else: every host producer in the set).
    pub producer: Option<String>,
    /// Only subjects whose declared path contains this.
    pub subject: Option<String>,
    /// `{var}` values by name; unnamed vars get deterministic synthetic
    /// values (stated in the plan).
    pub vars: Vec<(String, String)>,
    /// Override every entry's rate (Hz). `None` = the registry-driven
    /// defaults: telemetry 1 Hz, state ttl/2 refresh, events inside their
    /// declared budget.
    pub rate_hz: Option<f64>,
    pub pattern: GenPattern,
    pub duration: Duration,
    /// Drives synthesis and jitter — same seed, same run.
    pub seed: u64,
    /// The tool name stamped into the marker.
    pub tool: String,
    /// Fault kinds to inject (#163). Empty = conforming traffic. Non-empty
    /// expands the plan to one variant per (subject × fault), each carrying a
    /// single `fault=<kind>` marker and a printable delta — double-guarded at
    /// the CLI edge (`--i-know` plus an explicit endpoint/`--base`).
    pub faults: Vec<Fault>,
}

/// One subject the run will publish, fully resolved — the plan is printed
/// before anything touches the bus (the replay dry-run precedent).
#[derive(Debug, Clone, Serialize)]
pub struct GenPlanEntry {
    pub key: String,
    pub class: String,
    pub producer: String,
    pub type_name: String,
    pub qos: String,
    /// `declared` or `default` — where the profile came from (#158's rule:
    /// a generator that picks QoS silently is the write-side O4 mistake).
    pub qos_source: &'static str,
    pub rate_hz: f64,
    /// `describe` / `schema-set` / `placeholder` — where the body shape
    /// came from.
    pub body_source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    /// For `events`: the hard cap on sends within the run (the declared
    /// budget, RFC 04 §1.3) — a generator must not out-shout the registry
    /// it claims to follow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events_cap: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The fault this entry injects (#163), `None` for a conforming entry —
    /// stamped into every one of its samples' markers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fault: Option<Fault>,
    /// The printable delta from valid this fault introduces (#163) — stated
    /// in the plan before a byte moves.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fault_delta: Option<String>,
    #[serde(skip)]
    pub schema: Option<TypeSchema>,
    /// Absolute chunk index (into the full wire key) of the per-send unique
    /// id — set for events, whose keys MUST be write-once (RFC 04 §1.3).
    #[serde(skip)]
    pub unique_chunk: Option<usize>,
}

/// What one run did.
#[derive(Debug, Clone, Serialize)]
pub struct GenReport {
    pub duration_s: f64,
    pub entries: usize,
    pub sent: u64,
    /// Bodies the schema refused to encode — counted, never silently
    /// skipped (these are a bug in the synthesizer or the schema, and
    /// either way the operator hears about it).
    pub refused: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub first_errors: Vec<String>,
}

/// A deterministic chunk-safe value for an unnamed `{var}` (lowercase
/// alphanumerics only, RFC 03 §2's charset).
fn synthetic_var(name: &str) -> String {
    let clean: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect();
    if clean.is_empty() {
        "v1".into()
    } else {
        format!("{clean}1")
    }
}

/// Resolve the run's plan against the slices: which keys, which shapes,
/// which rates. Schema ladder per type: the producer's live `describe`
/// (when a session is given) > the offline `--schema-set` document > a
/// placeholder `{}` body with a stated note.
pub async fn build_plan(
    session: Option<&Session>,
    store: &SchemaStore,
    slices: &SliceSet,
    base: &str,
    schema_set: Option<&SchemaSet>,
    spec: &GenSpec,
) -> Result<Vec<GenPlanEntry>> {
    let mut plan = Vec::new();
    for slice in slices.slices() {
        if slice.service_origin.is_some() {
            // Impersonating a service origin (@catalog) would collide with
            // the real service's single-writer claim (RFC 06 §5.3) — out of
            // scope, stated rather than silently skipped.
            continue;
        }
        if let Some(p) = &spec.producer
            && &slice.name != p
        {
            continue;
        }
        for subject in &slice.subjects {
            if let Some(filter) = &spec.subject
                && !subject.path.contains(filter.as_str())
            {
                continue;
            }
            let pattern = SubjectPattern::parse(&subject.path)
                .map_err(|e| anyhow!("{}/{}: {e}", slice.name, subject.path))?;
            let mut tail: Vec<String> = Vec::new();
            let mut synthetic_vars: Vec<String> = Vec::new();
            let mut unique_tail_idx = None;
            for chunk in pattern.chunks() {
                match chunk {
                    PatternChunk::Literal(l) => tail.push(l.clone()),
                    PatternChunk::Var(name) | PatternChunk::Rest(name) => {
                        let value = spec
                            .vars
                            .iter()
                            .find(|(k, _)| k == name)
                            .map(|(_, v)| v.clone())
                            .unwrap_or_else(|| {
                                synthetic_vars.push(name.clone());
                                synthetic_var(name)
                            });
                        if subject.class == "events" {
                            // The last variable is the per-send unique id
                            // (events keys are write-once, RFC 04 §1.3).
                            unique_tail_idx = Some(tail.len());
                        }
                        tail.push(value);
                    }
                }
            }
            let key = with_base(
                base,
                format!(
                    "v1/{}/{}/{}/{}",
                    spec.origin,
                    subject.class,
                    slice.name,
                    tail.join("/")
                ),
            );
            // The unique chunk's index in the FULL key: base chunks +
            // v1/origin/class/producer (4) + its index in the tail.
            let base_chunks = if base.is_empty() {
                0
            } else {
                base.split('/').count()
            };
            let unique_chunk = unique_tail_idx.map(|i| base_chunks + 4 + i);

            let (qos, qos_source) = match subject.qos.as_deref().and_then(QosProfile::from_name) {
                Some(q) => (q, "declared"),
                None => (QosProfile::Sampled, "default"),
            };

            // Rate: override > class default. Events are additionally
            // capped at their declared budget for the run.
            let mut events_cap = None;
            let mut note: Option<String> = None;
            let rate_hz = match subject.class.as_str() {
                "events" => {
                    let cap_h = subject
                        .rate
                        .as_deref()
                        .and_then(crate::doctor::rate_cap_per_hour)
                        .unwrap_or(1);
                    let cap_run = ((f64::from(u32::try_from(cap_h.min(3600)).unwrap_or(3600))
                        * spec.duration.as_secs_f64())
                        / 3600.0)
                        .floor()
                        .max(1.0) as u64;
                    events_cap = Some(cap_run.min(cap_h));
                    // Spread the budget over the run.
                    (events_cap.unwrap_or(1) as f64 / spec.duration.as_secs_f64()).min(1.0)
                }
                "state" => match subject.ttl_s {
                    // Refresh at ttl/2 (RFC 04 §1.2).
                    Some(ttl) if ttl > 0 => 2.0 / ttl as f64,
                    _ => 0.5,
                },
                _ => 1.0,
            };
            let rate_hz = spec.rate_hz.unwrap_or(rate_hz).clamp(0.001, 1000.0);

            // The schema ladder.
            let mut body_source = "placeholder";
            let mut schema = None;
            if let Some(session) = session
                && let Some(s) = store
                    .schema_for(session, &slice.name, &subject.type_name)
                    .await
            {
                schema = Some(s);
                body_source = "describe";
            }
            if schema.is_none()
                && let Some(set) = schema_set
                && let Some(s) = set.get(&subject.type_name)
            {
                schema = Some(s.clone());
                body_source = "schema-set";
            }
            if schema.is_none() {
                note = Some(format!(
                    "no schema for {} — sending a placeholder {{}} body, labelled",
                    subject.type_name
                ));
            }
            if !synthetic_vars.is_empty() {
                let vars = synthetic_vars.join(", ");
                note = Some(match note.take() {
                    Some(n) => format!("{n}; synthetic values for {{{vars}}}"),
                    None => format!("synthetic values for {{{vars}}} (override with --var)"),
                });
            }
            let encoding =
                crate::body::encode_encoding(None, subject.encoding.as_deref(), schema.as_ref());

            let valid = GenPlanEntry {
                key,
                class: subject.class.clone(),
                producer: slice.name.clone(),
                type_name: subject.type_name.clone(),
                qos: qos.name().to_string(),
                qos_source,
                rate_hz,
                body_source,
                encoding,
                events_cap,
                note,
                fault: None,
                fault_delta: None,
                schema,
                unique_chunk,
            };

            if spec.faults.is_empty() {
                plan.push(valid);
                continue;
            }
            // One variant per fault kind: the delta is computed against the
            // valid entry, then the static perturbations (key/QoS/encoding)
            // are baked into the variant's fields — the body/timestamp faults
            // ride at send time off `fault` (see `run_gen`). Every variant
            // carries a single `fault=<kind>` marker.
            for &fault in &spec.faults {
                let mut variant = valid.clone();
                variant.fault_delta = Some(fault.delta(&valid));
                variant.key = fault.perturb_key(&valid.key);
                variant.qos = fault
                    .perturb_qos(QosProfile::from_name(&valid.qos).unwrap_or(QosProfile::Sampled))
                    .name()
                    .to_string();
                if fault.drops_encoding() {
                    variant.encoding = None;
                }
                variant.fault = Some(fault);
                plan.push(variant);
            }
        }
    }
    Ok(plan)
}

/// The serving halves of a mock producer, alive while held: each declared
/// responder is *driven* by its own task (a [`crate::MockResponder`] is
/// pull-based — a responder nobody drives answers nobody). Dropping this
/// aborts the drivers, which undeclares their queryables.
#[derive(Debug)]
pub struct MockProducer {
    /// How many `@rpc` keys are being answered.
    pub keys: usize,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for MockProducer {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

/// Serve the RFC 08 halves for the impersonated producers (`--serve-describe`):
/// `introspect` answers with the slice's verbatim TOML, `describe` with the
/// schema-set document — a consumer under test can fetch shapes from this
/// mock exactly as it would from the real producer.
pub async fn serve_describe(
    session: &Session,
    base: &str,
    origin: &str,
    slices: &SliceSet,
    schema_set: Option<&SchemaSet>,
    producer: Option<&str>,
) -> Result<MockProducer> {
    let mut tasks = Vec::new();
    let mut keys = 0usize;
    let mut serve = |key: String, body: Vec<u8>, encoding: &'static str| {
        let session = session.clone();
        keys += 1;
        tasks.push(tokio::spawn(async move {
            let Ok(responder) =
                crate::serve::declare_responder(&session, &key, body, Some(encoding), false).await
            else {
                return;
            };
            // Drive it: every incoming query gets the static answer.
            while responder.next().await.is_some() {}
        }));
    };
    for (slice, raw) in slices.entries() {
        if slice.service_origin.is_some() {
            continue;
        }
        if let Some(p) = producer
            && slice.name != p
        {
            continue;
        }
        if raw.is_empty() {
            continue; // a bus-built set has no verbatim TOML to serve
        }
        let introspect = with_base(base, format!("v1/{origin}/@rpc/{}/introspect", slice.name));
        serve(introspect, raw.as_bytes().to_vec(), "text/plain");
        if let Some(set) = schema_set {
            let describe = with_base(base, format!("v1/{origin}/@rpc/{}/describe", slice.name));
            serve(describe, set.to_json().into_bytes(), "application/json");
        }
    }
    Ok(MockProducer { keys, tasks })
}

/// Run the plan: every entry publishes on its own schedule until the
/// duration elapses. Bodies synthesize per tick and encode through the
/// store's validating ladder; a refused body is counted and reported.
pub async fn run_gen(
    session: &Session,
    store: &SchemaStore,
    plan: &[GenPlanEntry],
    spec: &GenSpec,
) -> Result<GenReport> {
    let _ = store; // encode rides a per-task registry; the store fetched schemas at plan time
    let synth = Synth::new(spec.seed);
    let deadline = tokio::time::Instant::now() + spec.duration;
    let total_s = spec.duration.as_secs_f64();

    let mut tasks = Vec::new();
    for (i, entry) in plan.iter().enumerate() {
        let entry = entry.clone();
        let session = session.clone();
        // Per-entry marker: a faulted sample additionally carries
        // `fault=<kind>` (RFC 09 §5.3), so a capture or doctor listen can
        // attribute exactly which deviation it saw.
        let marker = synthetic_marker(&spec.tool, &spec.origin, entry.fault.map(Fault::as_str));
        let store_encoding = entry.encoding.clone();
        let pattern = spec.pattern;
        let seed = spec.seed;
        tasks.push(tokio::spawn(async move {
            let registry = zenkey::schema::decode::DecoderRegistry::new();
            let started = tokio::time::Instant::now();
            let mut sent = 0u64;
            let mut refused = 0u64;
            let mut first_errors: Vec<String> = Vec::new();
            let record_err = |e: String, refused: &mut u64, errs: &mut Vec<String>| {
                *refused += 1;
                if errs.len() < 3 {
                    errs.push(e);
                }
            };
            // A long-lived publication for repeated keys; events declare
            // per send on their unique key.
            let publication = if entry.unique_chunk.is_none() {
                match crate::write::declare_publication(
                    &session,
                    &entry.key,
                    QosProfile::from_name(&entry.qos).unwrap_or(QosProfile::Sampled),
                    entry.encoding.as_deref(),
                )
                .await
                {
                    Ok(p) => Some(p),
                    Err(e) => {
                        return (0, 1, vec![format!("{}: declare: {e}", entry.key)]);
                    }
                }
            } else {
                None
            };

            let base_interval = Duration::from_secs_f64(1.0 / entry.rate_hz);
            let mut tick: u64 = 0;
            loop {
                if let Some(cap) = entry.events_cap
                    && sent >= cap
                {
                    // The declared budget is spent; the entry idles out the
                    // rest of the run rather than out-shouting the registry.
                    tokio::time::sleep_until(deadline).await;
                    break;
                }
                // Body: synthesize + encode, or the labelled placeholder.
                let bytes = match &entry.schema {
                    Some(schema) => match synth.instance(schema, tick) {
                        Some(value) => {
                            let wire = zenkey::schema::WireEncoding::from_encoding_str(
                                store_encoding.as_deref().unwrap_or("application/json"),
                            );
                            match registry.encode(schema, &value, &wire) {
                                Ok(b) => b,
                                Err(e) => {
                                    record_err(
                                        format!("{}: encode: {e}", entry.key),
                                        &mut refused,
                                        &mut first_errors,
                                    );
                                    tick += 1;
                                    continue;
                                }
                            }
                        }
                        None => b"{}".to_vec(),
                    },
                    None => b"{}".to_vec(),
                };
                // The fault (if any) perturbs the valid bytes post-encode, so
                // the deviation bypasses the validating encoder that made them
                // (#163). Key/QoS/encoding faults were already baked into the
                // entry at plan time; here ride the body and timestamp faults.
                let bytes = match entry.fault {
                    Some(f) => f.perturb_body(bytes),
                    None => bytes,
                };
                // Valid samples carry an HLC timestamp (state LWW, RFC 04 §4);
                // the `unstamped` fault omits it, the one deviation a doctor
                // freshness check can then catch.
                let stamp = if entry.fault.map(Fault::drops_timestamp).unwrap_or(false) {
                    None
                } else {
                    Some(session.new_timestamp())
                };
                let outcome = match &publication {
                    Some(p) => p.send_stamped(bytes, Some(marker.clone()), stamp).await,
                    None => {
                        // Events: a fresh write-once key per send.
                        let key = unique_key(&entry, seed, sent);
                        match crate::write::declare_publication(
                            &session,
                            &key,
                            QosProfile::from_name(&entry.qos).unwrap_or(QosProfile::Sampled),
                            entry.encoding.as_deref(),
                        )
                        .await
                        {
                            Ok(p) => {
                                let r = p.send_stamped(bytes, Some(marker.clone()), stamp).await;
                                let _ = p.undeclare().await;
                                r
                            }
                            Err(e) => Err(e),
                        }
                    }
                };
                match outcome {
                    Ok(()) => sent += 1,
                    Err(e) => record_err(
                        format!("{}: send: {e}", entry.key),
                        &mut refused,
                        &mut first_errors,
                    ),
                }
                tick += 1;

                // Pattern-shaped pacing, all deterministic.
                let interval = match pattern {
                    GenPattern::Steady => base_interval,
                    GenPattern::Jitter => {
                        let f = 0.7 + 0.6 * halton(seed ^ (i as u64) ^ tick);
                        base_interval.mul_f64(f)
                    }
                    GenPattern::Burst => {
                        let per_burst = entry.rate_hz.ceil().max(1.0) as u64;
                        if tick.is_multiple_of(per_burst) {
                            Duration::from_secs(1)
                        } else {
                            Duration::ZERO
                        }
                    }
                    GenPattern::Ramp => {
                        let progress = (started.elapsed().as_secs_f64() / total_s).clamp(0.05, 1.0);
                        base_interval.div_f64(progress)
                    }
                };
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {}
                    _ = tokio::time::sleep_until(deadline) => break,
                }
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
            }
            if let Some(p) = publication {
                let _ = p.undeclare().await;
            }
            (sent, refused, first_errors)
        }));
    }

    let mut sent = 0u64;
    let mut refused = 0u64;
    let mut first_errors = Vec::new();
    for t in tasks {
        let (s, r, errs) = t.await.map_err(|e| anyhow!("gen task: {e}"))?;
        sent += s;
        refused += r;
        for e in errs {
            if first_errors.len() < 5 {
                first_errors.push(e);
            }
        }
    }
    Ok(GenReport {
        duration_s: spec.duration.as_secs_f64(),
        entries: plan.len(),
        sent,
        refused,
        first_errors,
    })
}

/// Events keys are write-once: rebuild the key with the unique chunk set to
/// a fresh, deterministic, chunk-safe id.
fn unique_key(entry: &GenPlanEntry, seed: u64, n: u64) -> String {
    let Some(idx) = entry.unique_chunk else {
        return entry.key.clone();
    };
    let id = format!("{:012x}{:04x}", seed & 0xffff_ffff_ffff, n & 0xffff);
    entry
        .key
        .split('/')
        .enumerate()
        .map(|(i, c)| if i == idx { id.as_str() } else { c })
        .collect::<Vec<_>>()
        .join("/")
}

/// A low-discrepancy pseudo-random in [0,1) — deterministic, no RNG dep.
fn halton(n: u64) -> f64 {
    let mut f = 1.0;
    let mut r = 0.0;
    let mut i = n.wrapping_mul(2654435761) % 4096 + 1;
    while i > 0 {
        f /= 2.0;
        r += f * (i % 2) as f64;
        i /= 2;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLICES: &str = r#"
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
ttl_s = 30
[[subject]]
path = "cpu/{core}/usage"
class = "telemetry"
type = "Point"
[[subject]]
path = "boom/{id}"
class = "events"
type = "Boom"
rate = "rare"
"#;

    fn spec() -> GenSpec {
        GenSpec {
            origin: "h-abababababab".into(),
            producer: None,
            subject: None,
            vars: vec![("core".into(), "cpu0".into())],
            rate_hz: None,
            pattern: GenPattern::Steady,
            duration: Duration::from_secs(10),
            seed: 42,
            tool: "zenctl gen".into(),
            faults: vec![],
        }
    }

    async fn plan_for(base: &str) -> Vec<GenPlanEntry> {
        let slices =
            SliceSet::from_slices(vec![zenkey::parse_slice(SLICES).expect("fixture parses")]);
        let store = SchemaStore::new(base, Duration::from_millis(100));
        let set = SchemaSet::parse(
            r#"{"schema_version":1,"app":"t","types":{
                "Health":{"kind":"json-schema","hash":"","schema":{"type":"object",
                    "properties":{"ok":{"type":"boolean"}}}}}}"#,
        )
        .expect("set parses");
        build_plan(None, &store, &slices, base, Some(&set), &spec())
            .await
            .expect("plan builds")
    }

    /// The plan is the registry, resolved: declared QoS with its source,
    /// class-driven rates (state ttl/2, events inside their budget), the
    /// schema ladder's rung named per entry, vars filled as given or
    /// synthesized with a note.
    #[tokio::test]
    async fn the_plan_resolves_declared_qos_rates_and_the_schema_ladder() {
        let plan = plan_for("").await;
        assert_eq!(plan.len(), 3);

        let health = &plan[0];
        assert_eq!(health.key, "v1/h-abababababab/state/demo/health");
        assert_eq!(
            (health.qos.as_str(), health.qos_source),
            ("transition", "declared")
        );
        assert!(
            (health.rate_hz - 2.0 / 30.0).abs() < 1e-9,
            "{}",
            health.rate_hz
        );
        assert_eq!(health.body_source, "schema-set");
        assert!(health.note.is_none());

        let cpu = &plan[1];
        assert_eq!(cpu.key, "v1/h-abababababab/telemetry/demo/cpu/cpu0/usage");
        assert_eq!((cpu.qos.as_str(), cpu.qos_source), ("sampled", "default"));
        assert_eq!(cpu.rate_hz, 1.0);
        assert_eq!(cpu.body_source, "placeholder");
        assert!(
            cpu.note.as_deref().unwrap_or("").contains("no schema"),
            "{:?}",
            cpu.note
        );

        let boom = &plan[2];
        assert_eq!(boom.class, "events");
        assert_eq!(boom.events_cap, Some(1), "rare = 1/h caps a 10s run at 1");
        assert!(boom.unique_chunk.is_some(), "events keys are write-once");
        assert!(
            boom.note.as_deref().unwrap_or("").contains("{id}"),
            "the synthesized var is stated: {:?}",
            boom.note
        );
    }

    /// The unique chunk lands where the `{id}` was, under any base depth.
    #[tokio::test]
    async fn events_keys_get_a_fresh_id_where_the_var_was() {
        for base in ["", "acme", "acme/fleet-a"] {
            let plan = plan_for(base).await;
            let boom = plan.iter().find(|e| e.class == "events").unwrap();
            let k1 = unique_key(boom, 42, 0);
            let k2 = unique_key(boom, 42, 1);
            assert_ne!(k1, k2, "each send gets its own key ({base:?})");
            let tail1: Vec<&str> = k1.split('/').collect();
            let tail2: Vec<&str> = k2.split('/').collect();
            assert_eq!(tail1.len(), tail2.len());
            let diffs: Vec<usize> = (0..tail1.len()).filter(|&i| tail1[i] != tail2[i]).collect();
            assert_eq!(diffs.len(), 1, "only the id chunk moves ({base:?})");
            assert!(
                k1.ends_with(tail1[diffs[0]]),
                "the id is the declared {{id}} position ({base:?}): {k1}"
            );
        }
    }

    /// The marker is exactly the RFC 09 §5.3 shape #161's detector reads.
    #[test]
    fn the_marker_round_trips_through_the_doctors_detector() {
        let m = synthetic_marker("zenctl gen", "h-abababababab", None);
        let v: serde_json::Value = serde_json::from_slice(&m).unwrap();
        assert_eq!(v["synthetic"], true);
        assert_eq!(v["tool"], "zenctl gen");
        assert_eq!(v["origin"], "h-abababababab");
        assert!(v.get("fault").is_none(), "no fault key unless injecting");
        let f = synthetic_marker("zenctl gen", "h-abababababab", Some("truncate"));
        let v: serde_json::Value = serde_json::from_slice(&f).unwrap();
        assert_eq!(v["fault"], "truncate");
    }

    /// Every kind's CLI token round-trips, and an unknown kind is refused with
    /// the vocabulary named (spray's decline precedent, applied to a flag).
    #[test]
    fn fault_kinds_parse_and_an_unknown_is_refused() {
        for f in Fault::ALL {
            assert_eq!(Fault::parse(f.as_str()).unwrap(), f);
        }
        let err = Fault::parse("scramble").unwrap_err().to_string();
        assert!(err.contains("unknown fault kind"), "{err}");
        assert!(err.contains("truncate"), "the vocabulary is named: {err}");
    }

    /// With faults requested the plan expands to one variant per (subject ×
    /// fault); each states its printable delta and bakes the static
    /// perturbations (key/QoS/encoding) into its fields, leaving the body and
    /// timestamp faults for send time.
    #[tokio::test]
    async fn faults_expand_the_plan_one_variant_per_kind_with_a_stated_delta() {
        let slices =
            SliceSet::from_slices(vec![zenkey::parse_slice(SLICES).expect("fixture parses")]);
        let store = SchemaStore::new("", Duration::from_millis(100));
        let mut spec = spec();
        spec.faults = Fault::ALL.to_vec();
        let plan = build_plan(None, &store, &slices, "", None, &spec)
            .await
            .expect("plan builds");
        // Three subjects × seven faults.
        assert_eq!(plan.len(), 3 * 7);
        assert!(
            plan.iter().all(|e| e.fault.is_some() && e.fault_delta.is_some()),
            "every faulted entry names its kind and delta"
        );

        // The `health` state subject, one variant per kind — the static
        // perturbations are visible in the fields.
        let health: Vec<&GenPlanEntry> = plan
            .iter()
            .filter(|e| e.key.starts_with("v1/h-abababababab/state/demo/health"))
            .collect();
        assert_eq!(health.len(), 7);

        let unregistered = health
            .iter()
            .find(|e| e.fault == Some(Fault::UnregisteredKey))
            .unwrap();
        assert_eq!(unregistered.key, "v1/h-abababababab/state/demo/health/unregistered");

        let wrong_qos = health
            .iter()
            .find(|e| e.fault == Some(Fault::WrongQos))
            .unwrap();
        assert_ne!(wrong_qos.qos, "transition", "the declared profile is not honoured");

        let missing_enc = health
            .iter()
            .find(|e| e.fault == Some(Fault::MissingEncoding))
            .unwrap();
        assert!(missing_enc.encoding.is_none(), "the wire encoding is dropped");

        // The body/timestamp faults leave the entry's fields at the valid
        // resolution — they ride at send time.
        let truncate = health
            .iter()
            .find(|e| e.fault == Some(Fault::Truncate))
            .unwrap();
        assert_eq!(truncate.qos, "transition");
        assert!(truncate.key.ends_with("/health"));
    }

    /// The post-encode body perturbations produce exactly the deviation each
    /// kind names — and never route through the validating encoder (that is
    /// why they can violate the schema at all).
    #[test]
    fn body_faults_perturb_the_encoded_bytes() {
        let valid = br#"{"ok":true,"load":3}"#.to_vec();

        let truncated = Fault::Truncate.perturb_body(valid.clone());
        assert_eq!(truncated.len(), valid.len() / 2, "half the bytes survive");

        let wrong = Fault::WrongType.perturb_body(valid.clone());
        let v: serde_json::Value = serde_json::from_slice(&wrong).unwrap();
        assert!(v.is_string(), "a bare string where an object was declared");

        let extra = Fault::ExtraField.perturb_body(valid.clone());
        let v: serde_json::Value = serde_json::from_slice(&extra).unwrap();
        assert_eq!(v["_fault"], true, "the undeclared field rides");
        assert_eq!(v["ok"], true, "the valid fields survive alongside it");
    }
}
