//! Typed reports — the shared contract between the engine and every frontend.
//!
//! Moved from zenctl (issue #34; the redesign doc called this move "one
//! refactor unlocking scripting, tests, GUI parity"). A report struct is the
//! stable output shape: zenctl renders it as a table or serde JSON/NDJSON,
//! zengui renders it as widgets, and both stay in agreement because neither
//! owns it.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::facts::{KeyDescription, KeyShape, Registration};

#[derive(Debug, Clone, Serialize)]
pub struct TopicRow {
    pub producer: String,
    pub registry_version: String,
    pub class: String,
    pub path: String,
    pub type_name: String,
    /// Trailing `{var...}` family: the registry fixes the shape, not the
    /// members.
    pub open_ended: bool,
    /// Registry version the subject first appeared in, when declared.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// A retired subject (from the slice's `[[deprecated]]` ledger, RFC 08
    /// §6) — rendered only under `topic list --deprecated`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub deprecated: bool,
    /// When it was retired, if the ledger says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deprecated_since: Option<String>,
    /// The declared replacement subject, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaced_by: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TopicList {
    pub subjects: Vec<TopicRow>,
}

/// One key, described as far as the RFC 09 §5.1 ladder reached.
///
/// Redesigned in issue #34 from an all-or-nothing struct (whose builder
/// hard-errored on any key that was not a registered v1 data subject — an O1
/// violation) into a **partial** report: every key yields one, and `verdict`
/// says how far it got. Fields below the ladder's failure point are absent,
/// never defaulted.
#[derive(Debug, Clone, Serialize)]
pub struct TopicInfo {
    pub key: String,
    /// The ladder verdict, machine-stable (see [`TopicVerdict`]).
    pub verdict: TopicVerdict,
    /// Human-readable elaboration of the verdict (why, and what would answer
    /// it) — rendered, never parsed.
    pub note: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub producer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub variables: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qos: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_s: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cardinality: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Where the ladder stopped. Serialized snake_case; stable for scripts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TopicVerdict {
    /// Parses, refines, declared — the full story is present.
    Registered,
    /// Parses as a v1 data key; the producer's slice does not declare it.
    Unregistered,
    /// Parses; no loaded slice covers this producer (or service origin).
    NoSliceForProducer,
    /// Parses, but onto a verbatim plane — there is no `[[subject]]` surface
    /// to consult (RFC 03 §1.4).
    NotADataClass,
    /// A legal Zenoh key that is not this convention's (O1: a fact).
    NotV1,
    /// Sits under a different deployment base than the one configured.
    NotUnderBase,
    /// Parses as a data key, but no registry has been loaded — "not asked"
    /// is not "answered no" (O4).
    RegistryNotLoaded,
}

impl TopicInfo {
    /// Render a [`KeyDescription`] into the report shape.
    pub fn from_description(d: &KeyDescription) -> TopicInfo {
        let mut info = TopicInfo {
            key: d.key.clone(),
            verdict: TopicVerdict::NotV1,
            note: String::new(),
            origin: None,
            producer: None,
            class: None,
            subject: None,
            variables: BTreeMap::new(),
            payload_type: None,
            unit: None,
            qos: None,
            ttl_s: None,
            rate: None,
            cardinality: None,
            encoding: None,
            since: None,
            description: None,
        };
        match &d.facts.shape {
            KeyShape::NotUnderBase => {
                info.verdict = TopicVerdict::NotUnderBase;
                info.note = "under a different deployment base than the configured one \
                             (RFC 03 §1.1); `zenctl base list` discovers the bases in use"
                    .into();
                return info;
            }
            KeyShape::Unparsed { reason } => {
                info.verdict = TopicVerdict::NotV1;
                info.note = format!(
                    "not a keyspace-v2 key — a fact, not an error (RFC 09 §5.1 O1): {reason}"
                );
                return info;
            }
            KeyShape::V1(v) => {
                info.origin = Some(v.origin.clone());
                info.class = Some(v.class.clone());
                info.producer = v.producer.clone();
            }
        }
        match &d.facts.registration {
            Registration::Registered(s) => {
                info.verdict = TopicVerdict::Registered;
                info.subject = Some(s.path.clone());
                info.variables = s.vars.iter().cloned().collect();
                info.payload_type = Some(s.type_name.clone());
                info.unit = s.unit.clone();
                info.qos = s.qos.clone();
                info.encoding = s.encoding.clone();
                info.ttl_s = s.ttl_s;
            }
            Registration::Unregistered => {
                info.verdict = TopicVerdict::Unregistered;
                info.note = "parses as a v1 data key, but the producer's slice does not \
                             declare this subject — for a conforming producer, a subject \
                             that is not registered does not exist (RFC 08)"
                    .into();
            }
            Registration::NoSliceForProducer => {
                info.verdict = TopicVerdict::NoSliceForProducer;
                info.note = "no loaded registry slice covers this producer — `--registry \
                             <dir>` supplies slices offline; on-bus they come from \
                             introspect (RFC 08 §6)"
                    .into();
            }
            Registration::Unknown => {
                info.verdict = TopicVerdict::RegistryNotLoaded;
                info.note = "no registry loaded — \"not asked\" is not \"answered no\" \
                             (RFC 09 §5.1 O4)"
                    .into();
            }
            Registration::NotApplicable => {
                info.verdict = TopicVerdict::NotADataClass;
                info.note = "a verbatim plane, not a data class — there is no [[subject]] \
                             surface to describe (RFC 03 §1.4)"
                    .into();
            }
        }
        info
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceRow {
    pub producer: String,
    pub registry_version: String,
    pub kind: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceList {
    pub procedures: Vec<ServiceRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InterfaceTypeRow {
    pub name: String,
    pub carriers: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct InterfaceList {
    pub types: Vec<InterfaceTypeRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CarrierRow {
    pub producer: String,
    pub class: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InterfaceShow {
    pub type_name: String,
    pub carriers: Vec<CarrierRow>,
    /// What each producer serving this type name says its schema is
    /// (issue #51). Empty = nothing asked or nothing served; two rows with
    /// different hashes *is* the RFC 08 §7 drift finding, visible right here
    /// rather than only in `doctor`.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub schemas: Vec<SchemaRow>,
}

/// One type's schema entry as one producer serves it (issue #51).
#[derive(Debug, Clone, Serialize)]
pub struct SchemaRow {
    pub producer: String,
    pub type_name: String,
    pub kind: String,
    pub hash: String,
    /// The schema document, when the caller asked for the full form.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<serde_json::Value>,
}

/// One origin's reply-latency distribution in a benchmark (issue #52).
/// Timed **per reply**, so a fast origin in a fan-out is not charged the
/// slowest origin's round trip.
#[derive(Debug, Clone, Serialize)]
pub struct OriginLatency {
    pub origin: String,
    pub replies: usize,
    pub min_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

/// `zenctl bench rpc` (issue #52).
#[derive(Debug, Clone, Serialize)]
pub struct BenchReport {
    pub key: String,
    pub requested: usize,
    pub completed: usize,
    pub concurrency: usize,
    /// Error replies (RFC 05 §3) plus calls the GET itself failed.
    pub errors: usize,
    /// Calls that drew **zero** replies — counted apart from errors, because
    /// silence is not a failure and averaging it away would hide it
    /// (RFC 05 §3.1).
    pub silent: usize,
    pub elapsed_s: f64,
    pub calls_per_s: f64,
    pub origins: Vec<OriginLatency>,
}

/// One producer, as the bus serves it versus as the checkout declares it
/// (issue #50). A `None` version means "not present on that side", which is a
/// fact with two very different explanations — the findings say which.
#[derive(Debug, Clone, Serialize)]
pub struct ProducerDiff {
    pub producer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub served_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_version: Option<String>,
    /// RFC 08 §6 findings, rendered. Empty = the two agree.
    pub findings: Vec<String>,
}

/// `zenctl registry diff` (issue #50).
#[derive(Debug, Clone, Serialize)]
pub struct RegistryDiff {
    pub producers: Vec<ProducerDiff>,
}

impl RegistryDiff {
    /// Producers whose two sides disagree.
    pub fn disagreeing(&self) -> usize {
        self.producers
            .iter()
            .filter(|p| !p.findings.is_empty())
            .count()
    }
}

/// One producer's served `describe` reply, rendered (issue #51).
///
/// `served = false` is the honest degradation RFC 08 §7 leaves room for —
/// `describe` is a SHOULD, so a producer that serves none has said nothing
/// about its types, which is not the same as having no types.
#[derive(Debug, Clone, Serialize)]
pub struct SchemaDump {
    pub producer: String,
    pub served: bool,
    /// The declaring app, as the served set names it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    pub types: Vec<SchemaRow>,
    /// Registry-declared type names this producer's set does **not** cover —
    /// RFC 08 §7's totality clause, checked where the user is already looking.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub missing: Vec<String>,
}

/// One producer on one origin — row-shaped so a `--watch` loop can diff it
/// and a GUI can select it (`origin/producer` is the stable row identity).
#[derive(Debug, Clone, Serialize)]
pub struct NodeRow {
    pub origin: String,
    /// Live producer name (from the liveliness token; zero payload).
    pub producer: String,
    /// The producing app, when a registry slice joined (`--verbose`).
    /// `None` = not asked / no slice — never a default (O4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// Registry version from the joined slice, same provenance rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeList {
    pub nodes: Vec<NodeRow>,
    /// Whether a slice join was even attempted (`--verbose`) — keeps "asked,
    /// no slice served" distinguishable from "not asked" in rows whose
    /// `app`/`registry_version` are `None` (O4).
    pub slices_joined: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct BaseList {
    /// Discovered bases; `base` is a plain string, `""` for the empty base.
    pub bases: Vec<crate::DiscoveredBase>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StorageList {
    pub storages: Vec<crate::StorageInfo>,
    pub coverage: Vec<crate::CoverageRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallError {
    pub name: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallAnswer {
    pub origin: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    /// Raw text when the value is not JSON-shaped (TOML introspect replies…).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The reply's attachment, projected (JSON if it parses, UTF-8 text if
    /// it decodes, else a size tag) — never schema-decoded, an attachment is
    /// outside the registry's vocabulary (#117, #126). **Present only when
    /// the wire carried one** — absent, never null-when-unknown (O4); both
    /// fields are additive, so scripts on the old shape keep parsing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<serde_json::Value>,
    /// Its true size, regardless of how the projection reads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CallError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallReport {
    pub key: String,
    pub answers: Vec<CallAnswer>,
}

impl CallReport {
    /// The process exit code discipline (issue #12): 0 = at least one answer
    /// and no error replies; 1 = at least one error reply; 2 = zero replies
    /// (silence stays a distinct non-verdict — RFC 05 §3.1).
    pub fn exit_code(&self) -> i32 {
        if self.answers.is_empty() {
            2
        } else if self.answers.iter().any(|a| !a.ok) {
            1
        } else {
            0
        }
    }
}

/// One key's measured traffic over a `topic hz`/`topic bw` window.
#[derive(Debug, Clone, Serialize)]
pub struct RateRow {
    pub key: String,
    pub count: u64,
    pub bytes: u64,
    /// Source-sequence gaps (zero also means "publishers attach no
    /// SourceInfo" — an observation, not proof of losslessness).
    pub sn_gaps: u64,
    /// Observed **skewed** latency over the window (#119) — absent when no
    /// sample was HLC-stamped, which is not zero latency.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency: Option<crate::stats::LatencySummary>,
    /// Samples that carried no HLC — the other half of the observation.
    pub unstamped: u64,
}

/// The `topic hz` / `topic bw` report (issue #46) — measured counts plus the
/// O6 bound honesty: a bounded [`StatsTable`](crate::stats::StatsTable) that retired
/// keys must say so, or the totals silently claim more coverage than they
/// have.
#[derive(Debug, Clone, Serialize)]
pub struct RateReport {
    pub selector: String,
    pub window_s: u64,
    /// Rows are present only for a `--per-key` run, sorted by count
    /// descending.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<RateRow>,
    pub total_count: u64,
    pub total_bytes: u64,
    /// Concrete keys retained by the stats table over the window.
    pub keys: usize,
    /// Keys retired to stay within the table bound (RFC 09 §5.1 O6) — the
    /// totals cover the retained set only.
    pub evicted: u64,
    /// The bound the table ran under.
    pub max_keys: usize,
    /// Total source-sequence gaps (`None` = `--loss` was not asked).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sn_gaps: Option<u64>,
}

/// How bad a doctor finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorSeverity {
    /// A contract violation — the fleet disagrees with the RFCs or with
    /// itself.
    Error,
    /// Suspicious but explainable — judgement is degraded, not wrong.
    Warning,
    /// Worth knowing; not a defect.
    Info,
}

/// One machine-readable doctor finding (issue #46): what check fired, on
/// what, with the evidence and the normative citation — the shape the GUI
/// doctor panel renders as-is.
#[derive(Debug, Clone, Serialize)]
pub struct DoctorFinding {
    pub severity: DoctorSeverity,
    /// Stable check id (kebab-case), e.g. `slice-sync`, `introspect-coverage`,
    /// `schema-drift`, `stale-state`.
    pub check: String,
    /// What the finding is about (producer, key, or mesh-level subject).
    pub subject: String,
    /// The observed evidence, human-readable.
    pub evidence: String,
    /// The RFC section that makes this a finding (`None` when the check is
    /// operational judgement rather than a normative clause).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation: Option<String>,
}

/// The full doctor run: findings plus the coverage summary that makes an
/// empty findings list legible (what was checked, not just what was found —
/// RFC 05 §3.1: silence needs attribution).
#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub findings: Vec<DoctorFinding>,
    /// Producer slices confirmed in sync with the local registry
    /// (`origin/producer`), when `--registry` was given.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub synced: Vec<String>,
    /// Introspect replies received across the fleet.
    pub introspect_answered: usize,
    /// Producers on the liveliness roster.
    pub live_producers: usize,
    /// Producers serving an RFC 08 §7 `describe`.
    pub describe_served: usize,
    /// Producers serving no `describe` (a SHOULD, not a MUST).
    pub describe_missing: usize,
    /// Routers that answered the admin sweep.
    pub routers: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub router_version: Option<String>,
    /// Whether the `--deep` freshness/storage checks ran.
    pub deep: bool,
}

impl DoctorReport {
    pub fn count(&self, severity: DoctorSeverity) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == severity)
            .count()
    }
}

// ─── the @blob plane (RFC 07 §2, issues #58/#68) ────────────────────────────
//
// These are deliberately **not** feature-gated, and deliberately carry no
// `zblob` type. `zenkey-fleet`'s blob *transport* is optional (the `blob`
// feature); its blob *output shape* is not, because a report is a contract:
// `zenctl blob probe --format json` must serialize the same document whether
// or not the binary was built with the transport, and a frontend must be able
// to render a probe it deserialized from somewhere else entirely.

/// Where a [`BlobList`]'s rows came from — the O5 provenance line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlobListSource {
    /// Introspect slices served by live producers (RFC 08 §6).
    Bus,
    /// `--registry <dir>` TOMLs.
    RegistryDirs,
    /// Both, unioned.
    Union,
}

/// One producer's declaration that it serves one `@blob` tier.
#[derive(Debug, Clone, Serialize)]
pub struct BlobTierRow {
    pub producer: String,
    pub registry_version: String,
    /// The tier token **as declared**. A token RFC 07 §2 does not reserve is
    /// carried verbatim and flagged by `known_tier` rather than dropped: a
    /// declaration we do not understand is a fact about the fleet (RFC 09
    /// §5.1 O1), not noise.
    pub tier: String,
    /// Whether `tier` is one of the three reserved tokens.
    pub known_tier: bool,
    /// Declared endpoints (`artifact` only, RFC 07 §2.2).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub endpoints: Vec<String>,
    /// Content-hash algorithm (`store` only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub algo: Option<String>,
    /// The type whose payload carries the content root (RFC 07 §2.1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// The blob *content*'s encoding, when declared.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Origins whose liveliness roster names this producer.
    ///
    /// `None` means the roster was never asked — an offline `--registry` read
    /// learns nothing about who is up, and rendering that as "no origin serves
    /// this tier" would report a verdict nobody obtained (RFC 09 §5.1 O4).
    /// Even when present it is a *capability* claim: a producer that declares
    /// a tier is saying it serves the endpoints, never that it holds any
    /// particular blob. Only a probe answers that.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origins: Option<Vec<String>>,
}

/// Which producers declare which `@blob` tiers (RFC 07 §2.7 / 08 §2).
#[derive(Debug, Clone, Serialize)]
pub struct BlobList {
    pub tiers: Vec<BlobTierRow>,
    pub source: BlobListSource,
    /// How many slices were read. Without it an empty `tiers` reads as "nobody
    /// serves blobs" when it may mean "nothing was asked" (RFC 09 §5.1 O6).
    pub slices_considered: usize,
    /// How many of those declared no `@blob` tier at all.
    pub slices_without_blob: usize,
}

/// A holder's chunk availability for one artifact (RFC 07 §2.5's `have`).
#[derive(Debug, Clone, Serialize)]
pub struct BlobAvailability {
    pub chunk_count: u32,
    /// Chunks this holder can serve right now.
    pub have: u32,
    pub complete: bool,
}

/// A holder's manifest for one artifact (RFC 07 §2.2's `manifest`).
#[derive(Debug, Clone, Serialize)]
pub struct BlobManifest {
    pub id: String,
    /// Advisory only. It is never joined to any path — a remote party does not
    /// choose where bytes land.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    pub total_len: u64,
    pub chunk_size: u32,
    pub chunk_count: u32,
    /// The content root, hex — RFC 07 §2.1's integrity anchor.
    pub root: String,
    pub created_ms: i64,
}

/// One origin that answered a probe, and what it said.
#[derive(Debug, Clone, Serialize)]
pub struct BlobHolder {
    /// From the reply's **own** key. `"?"` only when that key neither parsed
    /// under the base nor had an origin in position 1.
    pub origin: String,
    /// The concrete key this origin answered on — the only fetchable form
    /// (RFC 07 §2.5: probe wide, fetch one).
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub availability: Option<BlobAvailability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<BlobManifest>,
    /// It answered, and we could not read it: the encoding it declared and
    /// why. Answering unreadably is not not answering (RFC 09 §5.1 O4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unreadable: Option<String>,
    /// An RFC 05 §3 error envelope.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CallError>,
}

/// Who holds an artifact, and at which root (RFC 07 §2.5).
#[derive(Debug, Clone, Serialize)]
pub struct BlobProbeReport {
    /// The target as spelled back: `artifact/<id>`, `tree/<hex>`, …
    pub target: String,
    pub tier: String,
    /// The selectors actually asked. A probe's coverage claim is exactly this
    /// list and no wider (RFC 09 §5.1 O5).
    pub asked: Vec<String>,
    /// Why nothing was asked, when nothing was — a tier with no probe
    /// endpoint, chiefly. Renders instead of a holder list; an unasked probe
    /// must never read as "no holders".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_probed: Option<String>,
    pub holders: Vec<BlobHolder>,
    pub answered: usize,
    /// Distinct content roots across holders.
    ///
    /// More than one is a **finding, not a tie-break**: the id is a name and
    /// RFC 07 §2.1's root is what disambiguates it, so a caller facing two
    /// roots must pin one rather than trust whoever answered first.
    pub roots: Vec<String>,
    /// Producers whose slice declares this tier — a capability claim, carried
    /// so a silent probe stays legible (RFC 05 §3.1: silence is not a verdict,
    /// and "nobody declares this" and "the declarers are down" are different
    /// silences).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub declared_by: Vec<String>,
}

/// A fetch's progress, as the caller may render it.
///
/// Engine-owned rather than a re-export of the reference client's progress
/// type: that one is `#[non_exhaustive]`, and a GUI message enum cannot carry
/// a non-exhaustive payload without a wildcard arm in every match — which is
/// how a new variant becomes invisible instead of a compile error.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum BlobProgress {
    Started {
        total_len: u64,
        chunk_count: u32,
    },
    /// A partial download resumed from its persisted chunk bitfield.
    Resumed {
        received: u32,
        total: u32,
    },
    Chunk {
        index: u32,
        received: u32,
        total: u32,
        bytes_received: u64,
    },
    Verifying,
    Completed {
        path: String,
    },
    Cancelled {
        received: u32,
        total: u32,
    },
    Failed {
        error: String,
    },
}

/// What one fetch from one origin cost and proved (RFC 07 §2.1, §2.5, §2.6).
#[derive(Debug, Clone, Serialize)]
pub struct BlobFetchReport {
    pub origin: String,
    /// The one concrete key fetched from.
    pub key: String,
    pub dest: String,
    pub bytes: u64,
    pub chunks: u32,
    /// Chunks a previous attempt had already banked.
    pub chunks_resumed: u32,
    /// Replies verification rejected **before disk** (RFC 07 §2.1).
    pub rejected: u32,
    pub retries: u32,
    pub elapsed_ms: u64,
    pub root: String,
    /// `false` = trust-on-first-use, which the caller had to ask for out loud.
    /// RFC 07 §2.1 requires a reference to carry the root; an operator typing
    /// an id by hand has no reference, so the report says which it was.
    pub root_pinned: bool,
    /// The priority the GETs actually rode at (RFC 07 §2.6) — reported rather
    /// than assumed, and filled from the same constant the client is built
    /// with, so the sentence cannot drift from the behaviour.
    pub priority: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::describe_key;
    use crate::registry::SliceSet;

    #[test]
    fn call_exit_codes() {
        let mut r = CallReport {
            key: "k".into(),
            answers: vec![],
        };
        assert_eq!(r.exit_code(), 2, "silence is its own exit code");
        r.answers.push(CallAnswer {
            origin: "h-1".into(),
            ok: true,
            value: None,
            text: Some("x".into()),
            attachment: None,
            attachment_bytes: None,
            error: None,
        });
        assert_eq!(r.exit_code(), 0);
        r.answers.push(CallAnswer {
            origin: "h-2".into(),
            ok: false,
            value: None,
            text: None,
            attachment: None,
            attachment_bytes: None,
            error: Some(CallError {
                name: "error/busy".into(),
                message: "later".into(),
            }),
        });
        assert_eq!(r.exit_code(), 1, "any refusal fails the invocation");
    }

    /// O1/O2 end to end: every kind of key yields a TopicInfo, and the
    /// verdicts are distinct.
    #[test]
    fn topic_info_is_partial_never_absent() {
        let cases = [
            ("demo/example/foo", TopicVerdict::NotV1),
            (
                "v1/h-3fa9c2d41b7e/@rpc/sysinfo/introspect",
                TopicVerdict::NotADataClass,
            ),
            (
                "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu",
                TopicVerdict::RegistryNotLoaded,
            ),
        ];
        for (key, want) in cases {
            let info = TopicInfo::from_description(&describe_key("", key, None));
            assert_eq!(info.verdict, want, "{key}");
            assert!(!info.note.is_empty(), "{key} must explain itself");
        }
        let info = TopicInfo::from_description(&describe_key(
            "zensight",
            "other/v1/h-3fa9c2d41b7e/state/x/y",
            None,
        ));
        assert_eq!(info.verdict, TopicVerdict::NotUnderBase);
        // Partial means partial: nothing below the failure point is invented.
        assert!(info.origin.is_none() && info.payload_type.is_none());
        // Loaded-and-empty is a different fact from not-loaded (O4).
        let empty = SliceSet::default();
        let info = TopicInfo::from_description(&describe_key(
            "",
            "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu",
            Some(&empty),
        ));
        assert_eq!(info.verdict, TopicVerdict::NoSliceForProducer);
        // The ladder reached the parse rung, so structural facts ARE present…
        assert_eq!(info.origin.as_deref(), Some("h-3fa9c2d41b7e"));
        // …but no registry facts were invented.
        assert!(info.payload_type.is_none());
    }

    /// The serialized DoctorReport is a wire contract: `zenctl doctor
    /// --format json` scripts and the GUI panel both consume this exact
    /// shape. Field renames/removals break users — this golden pin makes
    /// that a deliberate act.
    #[test]
    fn doctor_report_json_shape_is_pinned() {
        let report = DoctorReport {
            findings: vec![DoctorFinding {
                severity: DoctorSeverity::Error,
                check: "slice-sync".into(),
                subject: "h-3fa9c2d41b7e/sysinfo".into(),
                evidence: "registry version differs: served 1.0, local 2.0".into(),
                citation: Some("RFC 08 §6".into()),
            }],
            synced: vec!["h-3fa9c2d41b7e/other (registry 1.0)".into()],
            introspect_answered: 2,
            live_producers: 3,
            describe_served: 1,
            describe_missing: 1,
            routers: 1,
            router_version: Some("1.9.0".into()),
            deep: false,
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "findings": [{
                    "severity": "error",
                    "check": "slice-sync",
                    "subject": "h-3fa9c2d41b7e/sysinfo",
                    "evidence": "registry version differs: served 1.0, local 2.0",
                    "citation": "RFC 08 §6",
                }],
                "synced": ["h-3fa9c2d41b7e/other (registry 1.0)"],
                "introspect_answered": 2,
                "live_producers": 3,
                "describe_served": 1,
                "describe_missing": 1,
                "routers": 1,
                "router_version": "1.9.0",
                "deep": false,
            })
        );
    }
}

/// The RFC 09 §6 cutover-acceptance verdict (issue #59). Three states, not
/// two: "the old family is quiet on a fleet that is provably speaking" and
/// "everything is quiet" are different facts, and only the first is
/// evidence a migration finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CutoverVerdict {
    /// Old root silent, new plane carrying traffic — both halves held.
    Pass,
    /// The retired family still speaks — the migration is not done.
    OldStillSpeaks,
    /// The old root was silent but so was the new plane: a non-verdict
    /// (RFC 05 §3.1) — a dead fleet passes the silence half for free.
    Unproven,
}

/// The `zenctl cutover` report (issue #59; RFC 09 §6 half one).
#[derive(Debug, Clone, Serialize)]
pub struct CutoverReport {
    pub old_root: String,
    /// The stated meaning of "new plane": keys under this prefix. Stated,
    /// not inferred — the version chunk is plain, so key algebra cannot
    /// separate old from new (RFC 09 §6's note).
    pub new_prefix: String,
    pub window_s: u64,
    /// Samples heard on the old root — every one is a failure fact.
    pub old_samples: u64,
    pub old_keys_seen: usize,
    /// Up to a cap of offending keys, with per-key counts.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub old_examples: Vec<String>,
    /// Samples on the new plane over the window.
    pub new_samples: u64,
    /// Samples that were neither: outside `<base>/v1/` and not the old
    /// root. Leaks by this check's stated definition.
    pub leak_samples: u64,
    pub leaked_keys_seen: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub leak_examples: Vec<String>,
    /// Samples the bounded observer missed (O6): non-zero weakens the
    /// silence claim and the report says so.
    pub dropped: u64,
    pub verdict: CutoverVerdict,
}

/// The `zenctl probe` report (issue #59; RFC 09 §6 half two): how the
/// identity resolved, and what the origin-scoped concrete-key call said.
#[derive(Debug, Clone, Serialize)]
pub struct ProbeReport {
    /// What the operator typed (an origin id or a human label).
    pub input: String,
    /// The origin actually called.
    pub origin: String,
    /// `direct`, or `bridge:<key>` naming the self-certifying health
    /// document that resolved it (RFC 06 §6.2).
    pub via: String,
    pub call: CallReport,
}
