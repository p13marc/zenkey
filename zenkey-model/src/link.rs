//! `link.v1`: what crosses a constrained face
//! (`spec/profiles/link/v1.md`, #769).
//!
//! The profile's session-free half: the exposure a contract gives a
//! resource (§2.2), the face policy a deployment writes and its lints
//! (§2.1), the plan a generator derives from both (§2.3–§2.12), every
//! revision of an interface read together, how a generator keeps every
//! other principal's subject off a face (§2.4), and the checks of a
//! router's block (§2.15).
//!
//! ```text
//! exposure(kind, annotations)       ─▶ host │ link │ invalid │ ignored
//! crossing(token, templated, …)     ─▶ downsampled │ whole │ denied │ none, + reason
//! parse(face policy TOML)           ─▶ the policy, or the findings that refuse it
//! plan(policy, deployment)          ─▶ per face: denies, downsampling, attachment,
//!                                      window floors, liveness evidence, warnings
//! narrowed(policy, is_far)          ─▶ the link protocols a principal's subject keeps
//! check_windows / check_block /
//!   check_subjects /
//!   check_narrowing                 ─▶ findings
//! ```
//!
//! There is no session here and no router: a generator writes the plan
//! into a router's configuration (`zenkey-fleet`'s `acl gen`), and a tool
//! reads a block somebody says the router runs. The deployment's services
//! and the contracts come in as values.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::Serialize;
use serde_json::Value;
use zenoh_keyexpr::keyexpr;

use crate::authoring::Kind;
use crate::chunk::is_plain_chunk;
use crate::contract::{Contract, Resource};
use crate::freshness::{self, Horizon};
use crate::grammar::KindToken;
use crate::template::{Segment, Template};

/// The profile's id, as a contract lists it in `uses`.
pub const PROFILE: &str = "link.v1";

/// The profile's name, as an annotation key's prefix carries it.
pub const NAME: &str = "link";

/// The one annotation key (§4).
pub const EXPOSURE: &str = "link.exposure";

/// The published vocabulary (§4): the keys under `link.`, which W105 reads
/// in place of core Appendix D's interim row (core 0.29).
pub const VOCABULARY: &[&str] = &["exposure"];

/// zenoh's transport lease, `transport.link.tx.lease`, when a face policy
/// gives none (§2.1).
pub const DEFAULT_LEASE_MS: u64 = 10_000;

/// The core's tombstone window when a deployment configures none (core S3).
pub const DEFAULT_WINDOW_S: u64 = 60;

/// zenoh 1.10.1's link protocol names (`InterceptorLink`, kebab-case), the
/// values `select.link_protocols` takes (§2.1).
pub const LINK_PROTOCOLS: &[&str] = &[
    "tcp",
    "udp",
    "tls",
    "quic",
    "serial",
    "unixpipe",
    "unixsock-stream",
    "vsock",
    "ws",
];

/// The nine access-control messages, in the order a plan lists them (§0).
pub const MESSAGES: [&str; 9] = [
    "put",
    "delete",
    "declare_subscriber",
    "query",
    "declare_queryable",
    "reply",
    "liveliness_token",
    "declare_liveliness_subscriber",
    "liveliness_query",
];

/// The push messages, which an exposure deny names (§0, §2.7).
pub const PUSH_MESSAGES: [&str; 3] = ["put", "delete", "declare_subscriber"];

/// Both flows, in the order a plan lists them (§0).
pub const FLOWS: [&str; 2] = ["egress", "ingress"];

/// The `@zk` deny's key expression, base-relative (§2.6).
pub const ZK_DENY: &str = "zk2/**/@zk/**";

/// The `@adv` deny's key expression, base-relative (§2.6).
pub const ADV_DENY: &str = "zk2/**/@adv/**";

/// The admin space's deny, never namespaced (§2.6).
pub const ADMIN_DENY: &str = "@/**";

/// The kind tokens whose pushes a plan denies, in its order (§2.7).
pub const PUSH_TOKENS: [KindToken; 5] = [
    KindToken::Stream,
    KindToken::ExplicitStream,
    KindToken::State,
    KindToken::ExplicitState,
    KindToken::Events,
];

const MAX_U32: u64 = u32::MAX as u64;
const MAX_BATCH: u64 = 65_535;
const MAX_REGION: usize = 32;

// ── Exposure (§2.2) ──────────────────────────────────────────────────────

/// A resource's exposure, read from its resolved annotations (§2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exposure {
    /// `"host"`, or no `link.exposure`: it never crosses a constrained face.
    Host,
    /// `"link"`: it may cross a face that exposes.
    Link,
    /// A value that is no exposure, as JSON text: read as `host`.
    Invalid(String),
    /// On an operation, whatever the value: a call crosses by grant.
    Ignored,
}

impl Exposure {
    /// The fixture's token (`exposure.json`).
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Link => "link",
            Self::Invalid(_) => "invalid",
            Self::Ignored => "ignored",
        }
    }
}

/// Reads the exposure of a resource of `kind` from its resolved annotations
/// (§2.2): `[defaults]`, then `[defaults.<kind>]`, then its own (core §9.3).
#[must_use]
pub fn exposure(kind: Kind, annotations: &BTreeMap<String, Value>) -> Exposure {
    if kind == Kind::Operation {
        return Exposure::Ignored;
    }
    match annotations.get(EXPOSURE) {
        None => Exposure::Host,
        Some(Value::String(s)) if s == "host" => Exposure::Host,
        Some(Value::String(s)) if s == "link" => Exposure::Link,
        Some(v) => Exposure::Invalid(v.to_string()),
    }
}

/// [`exposure`] of a resource of a contract.
#[must_use]
pub fn exposure_of(r: &Resource) -> Exposure {
    exposure(r.kind, &r.annotations)
}

/// What a face's policy lets cross (§2.1 `expose`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expose {
    /// Exposed resources cross.
    Link,
    /// No push crosses.
    None,
}

/// What happens to a resource's pushes on a face (§2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pushes {
    /// A stream that crosses, one put per key per interval (§2.8).
    Downsampled,
    /// A state or an event that crosses, every put and delete (§2.7, §2.9).
    Whole,
    /// Denied on the face.
    Denied,
    /// An operation, which has no pushes (§2.9).
    None,
}

impl Pushes {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Downsampled => "downsampled",
            Self::Whole => "whole",
            Self::Denied => "denied",
            Self::None => "none",
        }
    }
}

/// Why, the first of these that holds, in this order (`exposure.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// An operation: its calls cross by grant (§2.9).
    RequestsOnly,
    /// A value that is no exposure, read as `host` (§2.2).
    Invalid,
    /// `host`, or no annotation (§2.2).
    Host,
    /// The face exposes none (§2.1).
    FaceClosed,
    /// A stream whose template has a parameter: no rule names its members
    /// (§2.8).
    Templated,
    /// It crosses.
    Exposed,
}

impl Reason {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RequestsOnly => "requests_only",
            Self::Invalid => "invalid",
            Self::Host => "host",
            Self::FaceClosed => "face_closed",
            Self::Templated => "templated",
            Self::Exposed => "exposed",
        }
    }
}

/// Whether a resource's pushes cross a face, and why (§2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crossing {
    pub pushes: Pushes,
    pub reason: Reason,
}

impl Crossing {
    /// Whether its pushes cross (§0's "crossing").
    #[must_use]
    pub fn crosses(self) -> bool {
        matches!(self.pushes, Pushes::Downsampled | Pushes::Whole)
    }
}

/// Whether the pushes of a resource of `token` cross a face that exposes
/// `expose` (§2.7): `templated` when its template has a parameter.
#[must_use]
pub fn crossing(
    token: KindToken,
    templated: bool,
    exposure: &Exposure,
    expose: Expose,
) -> Crossing {
    let denied = |reason| Crossing {
        pushes: Pushes::Denied,
        reason,
    };
    if token == KindToken::Op {
        return Crossing {
            pushes: Pushes::None,
            reason: Reason::RequestsOnly,
        };
    }
    match exposure {
        Exposure::Ignored => Crossing {
            pushes: Pushes::None,
            reason: Reason::RequestsOnly,
        },
        Exposure::Invalid(_) => denied(Reason::Invalid),
        Exposure::Host => denied(Reason::Host),
        Exposure::Link if expose == Expose::None => denied(Reason::FaceClosed),
        Exposure::Link if is_stream(token) && templated => denied(Reason::Templated),
        Exposure::Link if is_stream(token) => Crossing {
            pushes: Pushes::Downsampled,
            reason: Reason::Exposed,
        },
        Exposure::Link => Crossing {
            pushes: Pushes::Whole,
            reason: Reason::Exposed,
        },
    }
}

fn is_stream(token: KindToken) -> bool {
    matches!(token, KindToken::Stream | KindToken::ExplicitStream)
}

fn is_state(token: KindToken) -> bool {
    matches!(token, KindToken::State | KindToken::ExplicitState)
}

/// The kind a token carries (core §1.3).
#[must_use]
pub fn kind_of(token: KindToken) -> Kind {
    match token {
        KindToken::Stream | KindToken::ExplicitStream => Kind::Stream,
        KindToken::State | KindToken::ExplicitState => Kind::State,
        KindToken::Events => Kind::Event,
        KindToken::Op => Kind::Operation,
    }
}

// ── Findings (§2.1, §2.3, §2.11, §2.15) ───────────────────────────────────

/// The closed vocabulary of findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Code {
    /// The file is not TOML 1.0 (§2.1).
    Toml,
    /// A key the format does not define (§2.1).
    UnknownField,
    /// A required key absent (§2.1).
    MissingField,
    /// A value of the wrong type or outside its range (§2.1).
    BadValue,
    /// `attach = "router"` (§2.5).
    RouterAttach,
    /// `select.interfaces` (§2.14).
    SelectInterfaces,
    /// A link protocol zenoh 1.10.1 does not name (§2.1).
    UnknownProtocol,
    /// Two faces with a common link protocol (§2.14).
    SharedProtocol,
    /// A batch over a third of the lease (§2.10).
    BatchOverLease,
    /// A listed service the deployment names no interfaces for (§2.3).
    ServiceUnknown,
    /// An interface whose contract is not given (§2.3).
    ContractUnknown,
    /// An exposed stream with a parameter, denied (§2.8).
    TemplatedExposure,
    /// An exposed resource a deny must include (§2.7).
    CarveOutOverlap,
    /// A crossing stream whose horizon is below twice the interval
    /// (§2.12).
    IntervalOverHorizon,
    /// Revisions of a listed interface that disagree on a resource's
    /// exposure, one `link` and another not: it does not cross (§2.3, 0.3).
    ExposureDisagrees,
    /// A listed owner's tombstone window below the outage (§2.11).
    WindowUnderOutage,
    /// A block whose denies leave a pair of `@zk` open (§2.15).
    ZkNotDenied,
    /// A downsampling rule that is not one concrete key (§2.15).
    WildcardDownsampling,
    /// Two downsampling rules that intersect (§2.15).
    IntersectingDownsampling,
    /// A subject on the face bound to another principal than the face's
    /// far one, whatever else it names (§2.4, §2.15, 0.3; widened in 0.4,
    /// F-120).
    NearSubjectOnFace,
    /// Under `allow`, a link protocol of the face that no face subject
    /// naming no interface and no zid selects (§2.4, §2.15, 0.3).
    FaceSubjectMissing,
    /// A face subject that also selects sessions off the face (§2.4,
    /// §2.15, 0.4, F-120).
    FaceSubjectBeyondFace,
    /// A face subject not listed in exactly the far principal's policies
    /// (§2.4, §2.15, 0.4, F-120).
    FaceSubjectMisplaced,
    /// Under `allow`, a name the block binds whose session over a link
    /// protocol it is not kept off matches no subject (§2.4, §2.15, 0.4,
    /// F-120).
    NarrowedTooFar,
}

impl Code {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Toml => "toml",
            Self::UnknownField => "unknown_field",
            Self::MissingField => "missing_field",
            Self::BadValue => "bad_value",
            Self::RouterAttach => "router_attach",
            Self::SelectInterfaces => "select_interfaces",
            Self::UnknownProtocol => "unknown_protocol",
            Self::SharedProtocol => "shared_protocol",
            Self::BatchOverLease => "batch_over_lease",
            Self::ServiceUnknown => "service_unknown",
            Self::ContractUnknown => "contract_unknown",
            Self::TemplatedExposure => "templated_exposure",
            Self::CarveOutOverlap => "carve_out_overlap",
            Self::IntervalOverHorizon => "interval_over_horizon",
            Self::ExposureDisagrees => "exposure_disagrees",
            Self::WindowUnderOutage => "window_under_outage",
            Self::ZkNotDenied => "zk_not_denied",
            Self::WildcardDownsampling => "wildcard_downsampling",
            Self::IntersectingDownsampling => "intersecting_downsampling",
            Self::NearSubjectOnFace => "near_subject_on_face",
            Self::FaceSubjectMissing => "face_subject_missing",
            Self::FaceSubjectBeyondFace => "face_subject_beyond_face",
            Self::FaceSubjectMisplaced => "face_subject_misplaced",
            Self::NarrowedTooFar => "narrowed_too_far",
        }
    }

    /// Its severity, fixed per code.
    #[must_use]
    pub fn severity(self) -> Severity {
        match self {
            Self::BatchOverLease
            | Self::TemplatedExposure
            | Self::CarveOutOverlap
            | Self::IntervalOverHorizon
            | Self::ExposureDisagrees => Severity::Warning,
            _ => Severity::Error,
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for Code {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

/// An error refuses the policy, or the plan; a warning does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

/// One finding: what, about which face, at which key (§2.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct Finding {
    pub code: Code,
    pub severity: Severity,
    /// The face's name, or `None` for the top level and the deployment.
    pub face: Option<String>,
    /// The key it is about (§2.1), the service, the interface, the
    /// resource's name, or the rule's key expression.
    pub at: String,
}

impl Finding {
    /// A finding of `code`, with its fixed severity.
    #[must_use]
    pub fn new(code: Code, face: Option<&str>, at: impl Into<String>) -> Self {
        Finding {
            code,
            severity: code.severity(),
            face: face.map(str::to_owned),
            at: at.into(),
        }
    }

    /// Whether it refuses the policy or the plan.
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        match &self.face {
            Some(face) => write!(f, "{sev} {} [face {face}] {}", self.code, self.at),
            None => write!(f, "{sev} {} {}", self.code, self.at),
        }
    }
}

/// Sorts findings by face (none first), `at`, then code, each by its bytes,
/// and drops the repeated ones: findings are a set (§2.1).
pub fn sort_findings(findings: &mut Vec<Finding>) {
    findings.sort_by(|a, b| {
        a.face
            .cmp(&b.face)
            .then_with(|| a.at.cmp(&b.at))
            .then_with(|| a.code.as_str().cmp(b.code.as_str()))
    });
    findings.dedup();
}

// ── The face policy (§2.1) ───────────────────────────────────────────────

/// How the far side attaches (§2.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attach {
    /// One far-side session, or a gateway session, as a client.
    Client,
    /// A far router in a south region, named by its `region_name`.
    SouthRegion { region: String },
}

/// One constrained face of a router (§2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Face {
    /// The far principal: a user name or a certificate CN (core §11.1).
    pub far: String,
    pub attach: Attach,
    /// `select.link_protocols`, in the policy's order.
    pub link_protocols: Vec<String>,
    pub expose: Expose,
    /// Exactly when `expose` is [`Expose::Link`].
    pub interval_s: Option<u64>,
    pub max_outage_s: u64,
    pub batch_bytes: u64,
    pub link_bps: u64,
    /// [`DEFAULT_LEASE_MS`] when the policy gives none.
    pub lease_ms: u64,
}

impl Face {
    /// Whether a batch takes at most a third of the lease to cross (§2.10):
    /// `batch_bytes` × 24,000 ≤ `lease_ms` × `link_bps`, in integers.
    #[must_use]
    pub fn batch_within_lease(&self) -> bool {
        within_lease(self.batch_bytes, self.link_bps, self.lease_ms)
    }

    /// The downsampling frequency, 1/`interval_s` Hz, the quotient as an
    /// IEEE 754 double (§2.8). `interval_s` is at most 2^32−1, which a
    /// double holds exactly.
    #[must_use]
    pub fn freq(&self) -> Option<f64> {
        self.interval_s
            .map(|n| 1.0 / f64::from(u32::try_from(n).unwrap_or(u32::MAX)))
    }
}

fn within_lease(batch_bytes: u64, link_bps: u64, lease_ms: u64) -> bool {
    u128::from(batch_bytes) * 24_000 <= u128::from(lease_ms) * u128::from(link_bps)
}

/// A router's face policy (§2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacePolicy {
    pub system: String,
    /// The listed services, on `system`.
    pub services: Vec<String>,
    /// The faces, by name.
    pub faces: BTreeMap<String, Face>,
}

impl FacePolicy {
    /// A listed service's address, `<system>/<service>`.
    #[must_use]
    pub fn address(&self, service: &str) -> String {
        format!("{}/{service}", self.system)
    }
}

/// What [`parse`] read: the policy when no error refused it, and every
/// finding, sorted.
#[derive(Debug, Clone)]
pub struct Parsed {
    pub policy: Option<FacePolicy>,
    pub findings: Vec<Finding>,
}

/// Reads a face policy, and lints it (§2.1).
#[must_use]
pub fn parse(text: &str) -> Parsed {
    let refuse = |f: Finding| Parsed {
        policy: None,
        findings: vec![f],
    };
    let table: toml::Table = match toml::from_str(text) {
        Ok(t) => t,
        Err(_) => return refuse(Finding::new(Code::Toml, None, "")),
    };
    if crate::toml10::first_beyond_toml10(text).is_some() {
        return refuse(Finding::new(Code::Toml, None, ""));
    }
    let mut out = Vec::new();
    for k in table.keys() {
        if !["system", "services", "face"].contains(&k.as_str()) {
            out.push(Finding::new(Code::UnknownField, None, k.clone()));
        }
    }
    let system = match table.get("system") {
        None => {
            out.push(Finding::new(Code::MissingField, None, "system"));
            None
        }
        Some(toml::Value::String(s)) if is_plain_chunk(s) => Some(s.clone()),
        Some(_) => {
            out.push(Finding::new(Code::BadValue, None, "system"));
            None
        }
    };
    let services = match table.get("services") {
        None => {
            out.push(Finding::new(Code::MissingField, None, "services"));
            None
        }
        Some(v) => match plain_list(v) {
            Some(l) => Some(l),
            None => {
                out.push(Finding::new(Code::BadValue, None, "services"));
                None
            }
        },
    };
    let mut faces = BTreeMap::new();
    match table.get("face") {
        None => out.push(Finding::new(Code::MissingField, None, "face")),
        Some(toml::Value::Table(t)) if t.is_empty() => {
            out.push(Finding::new(Code::BadValue, None, "face"));
        }
        Some(toml::Value::Table(t)) => {
            for (name, v) in t {
                if !is_plain_chunk(name) {
                    out.push(Finding::new(Code::BadValue, Some(name), ""));
                }
                match v {
                    toml::Value::Table(ft) => {
                        if let Some(face) = parse_face(name, ft, &mut out) {
                            faces.insert(name.clone(), face);
                        }
                    }
                    _ => out.push(Finding::new(Code::BadValue, Some(name), "")),
                }
            }
        }
        Some(_) => out.push(Finding::new(Code::BadValue, None, "face")),
    }
    shared_protocols(&table, &mut out);
    for (name, face) in &faces {
        if !face.batch_within_lease() {
            out.push(Finding::new(
                Code::BatchOverLease,
                Some(name),
                "batch_bytes",
            ));
        }
    }
    sort_findings(&mut out);
    let policy = if out.iter().any(Finding::is_error) {
        None
    } else {
        Some(FacePolicy {
            system: system.unwrap_or_default(),
            services: services.unwrap_or_default(),
            faces,
        })
    };
    Parsed {
        policy,
        findings: out,
    }
}

/// An array of plain chunks, none repeated.
fn plain_list(v: &toml::Value) -> Option<Vec<String>> {
    let a = v.as_array()?;
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(a.len());
    for x in a {
        let s = x.as_str().filter(|s| is_plain_chunk(s))?;
        if !seen.insert(s) {
            return None;
        }
        out.push(s.to_owned());
    }
    Some(out)
}

/// An integer of `v` within `lo..=hi`.
fn int_in(v: &toml::Value, lo: u64, hi: u64) -> Option<u64> {
    let i = v.as_integer()?;
    u64::try_from(i).ok().filter(|n| (lo..=hi).contains(n))
}

/// A link protocol list: non-empty strings, none repeated. `None` when it
/// is not one.
fn protocol_list(v: &toml::Value) -> Option<Vec<String>> {
    let a = v.as_array()?;
    if a.is_empty() {
        return None;
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(a.len());
    for x in a {
        let s = x.as_str()?;
        if !seen.insert(s) {
            return None;
        }
        out.push(s.to_owned());
    }
    Some(out)
}

const FACE_KEYS: &[&str] = &[
    "far",
    "attach",
    "region",
    "select",
    "class",
    "expose",
    "interval_s",
    "max_outage_s",
    "batch_bytes",
    "link_bps",
    "lease_ms",
];

fn parse_face(name: &str, t: &toml::Table, out: &mut Vec<Finding>) -> Option<Face> {
    let at = Some(name);
    let mut ok = true;
    let mut bad = |out: &mut Vec<Finding>, code: Code, key: &str| {
        out.push(Finding::new(code, at, key));
        ok = false;
    };
    for k in t.keys() {
        if !FACE_KEYS.contains(&k.as_str()) {
            bad(out, Code::UnknownField, k);
        }
    }
    let far = match t.get("far") {
        None => {
            bad(out, Code::MissingField, "far");
            None
        }
        Some(toml::Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(_) => {
            bad(out, Code::BadValue, "far");
            None
        }
    };
    // `attach`: Some(Some(south)) for a valid one, Some(None) when invalid.
    let attach = match t.get("attach") {
        None => {
            bad(out, Code::MissingField, "attach");
            None
        }
        Some(toml::Value::String(s)) if s == "client" => Some(false),
        Some(toml::Value::String(s)) if s == "south-region" => Some(true),
        Some(toml::Value::String(s)) if s == "router" => {
            bad(out, Code::RouterAttach, "attach");
            None
        }
        Some(_) => {
            bad(out, Code::BadValue, "attach");
            None
        }
    };
    let region = match t.get("region") {
        None => None,
        Some(toml::Value::String(s)) if !s.is_empty() && s.len() <= MAX_REGION => Some(s.clone()),
        Some(_) => {
            bad(out, Code::BadValue, "region");
            None
        }
    };
    match (attach, t.contains_key("region")) {
        (Some(true), false) => bad(out, Code::MissingField, "region"),
        (Some(false), true) => bad(out, Code::BadValue, "region"),
        _ => {}
    }
    let link_protocols = match t.get("select") {
        None => {
            bad(out, Code::MissingField, "select");
            None
        }
        Some(toml::Value::Table(s)) => {
            for k in s.keys() {
                if k == "interfaces" {
                    bad(out, Code::SelectInterfaces, "select.interfaces");
                } else if k != "link_protocols" {
                    bad(out, Code::UnknownField, &format!("select.{k}"));
                }
            }
            match s.get("link_protocols") {
                None => {
                    bad(out, Code::MissingField, "select.link_protocols");
                    None
                }
                Some(v) => match protocol_list(v) {
                    None => {
                        bad(out, Code::BadValue, "select.link_protocols");
                        None
                    }
                    Some(l) if l.iter().any(|p| !LINK_PROTOCOLS.contains(&p.as_str())) => {
                        bad(out, Code::UnknownProtocol, "select.link_protocols");
                        None
                    }
                    Some(l) => Some(l),
                },
            }
        }
        Some(_) => {
            bad(out, Code::BadValue, "select");
            None
        }
    };
    match t.get("class") {
        None => bad(out, Code::MissingField, "class"),
        Some(toml::Value::String(s)) if s == "constrained" => {}
        Some(_) => bad(out, Code::BadValue, "class"),
    }
    let expose = match t.get("expose") {
        None => {
            bad(out, Code::MissingField, "expose");
            None
        }
        Some(toml::Value::String(s)) if s == "link" => Some(Expose::Link),
        Some(toml::Value::String(s)) if s == "none" => Some(Expose::None),
        Some(_) => {
            bad(out, Code::BadValue, "expose");
            None
        }
    };
    let interval_s = match t.get("interval_s") {
        None => None,
        Some(v) => match int_in(v, 1, MAX_U32) {
            Some(n) => Some(n),
            None => {
                bad(out, Code::BadValue, "interval_s");
                None
            }
        },
    };
    match (expose, t.contains_key("interval_s")) {
        (Some(Expose::Link), false) => bad(out, Code::MissingField, "interval_s"),
        (Some(Expose::None), true) => bad(out, Code::BadValue, "interval_s"),
        _ => {}
    }
    let mut int = |key: &str, hi: u64, required: bool| match t.get(key) {
        None => {
            if required {
                bad(out, Code::MissingField, key);
            }
            None
        }
        Some(v) => {
            let n = int_in(v, 1, hi);
            if n.is_none() {
                bad(out, Code::BadValue, key);
            }
            n
        }
    };
    let max_outage_s = int("max_outage_s", MAX_U32, true);
    let batch_bytes = int("batch_bytes", MAX_BATCH, true);
    let link_bps = int("link_bps", MAX_U32, true);
    let lease_ms = int("lease_ms", MAX_U32, false);
    if !ok {
        return None;
    }
    Some(Face {
        far: far?,
        attach: if attach? {
            Attach::SouthRegion { region: region? }
        } else {
            Attach::Client
        },
        link_protocols: link_protocols?,
        expose: expose?,
        interval_s,
        max_outage_s: max_outage_s?,
        batch_bytes: batch_bytes?,
        link_bps: link_bps?,
        lease_ms: if t.contains_key("lease_ms") {
            lease_ms?
        } else {
            DEFAULT_LEASE_MS
        },
    })
}

/// `shared_protocol` (§2.14), over every face whose protocol list is one.
fn shared_protocols(table: &toml::Table, out: &mut Vec<Finding>) {
    let Some(toml::Value::Table(faces)) = table.get("face") else {
        return;
    };
    let lists: Vec<(&String, Vec<String>)> = faces
        .iter()
        .filter_map(|(name, v)| {
            let l = v.get("select")?.get("link_protocols")?;
            Some((name, protocol_list(l)?))
        })
        .collect();
    for (i, (name, l)) in lists.iter().enumerate() {
        let shared = lists
            .iter()
            .enumerate()
            .any(|(j, (_, m))| i != j && l.iter().any(|p| m.contains(p)));
        if shared {
            out.push(Finding::new(
                Code::SharedProtocol,
                Some(name),
                "select.link_protocols",
            ));
        }
    }
}

// ── The deployment ───────────────────────────────────────────────────────

/// One resource as the plan reads it: its template, its kind token, and
/// its resolved annotations.
#[derive(Debug, Clone)]
pub struct Res {
    pub template: Template,
    pub token: KindToken,
    pub annotations: BTreeMap<String, Value>,
}

impl From<&Resource> for Res {
    fn from(r: &Resource) -> Self {
        Res {
            template: r.template.clone(),
            token: r.token,
            annotations: r.annotations.clone(),
        }
    }
}

/// What the plan needs of the deployment (§2.3): the interfaces each listed
/// service implements, and the revisions given of each interface.
#[derive(Debug, Clone, Default)]
pub struct Deployment {
    /// A service's name → the interface ids it implements.
    pub implements: BTreeMap<String, Vec<String>>,
    /// An interface id → each revision given of it, as its resources. Two
    /// minors of one major are two revisions of one id: an annotation
    /// changed between them is a review (core §9.8), and two services of
    /// one deployment may run either (core §3.3). The plan reads them
    /// together, failing closed where they disagree (§2.3).
    pub interfaces: BTreeMap<String, Vec<Vec<Res>>>,
}

impl Deployment {
    /// Adds a contract's resources as a revision of its interface id.
    pub fn add_contract(&mut self, c: &Contract) {
        self.add_revision(
            c.iface.to_string(),
            c.resources.iter().map(Res::from).collect(),
        );
    }

    /// Adds one revision of `iface`, as its resources.
    pub fn add_revision(&mut self, iface: impl Into<String>, resources: Vec<Res>) {
        self.interfaces
            .entry(iface.into())
            .or_default()
            .push(resources);
    }
}

/// An interface's resources as the plan reads them across its revisions
/// (§2.3): each resource any revision declares, paired by kind token and
/// template, first revision first. A resource several revisions declare is
/// exposed only where every one of them reads `link` (§2.2 fails closed),
/// and has a horizon only where every one gives it a horizon above 0, the
/// least of them (§2.12). The set holds the indices of the resources the
/// revisions disagree on: one reads `link`, another does not.
fn merge(revisions: &[Vec<Res>]) -> (Vec<Res>, BTreeSet<usize>) {
    let mut out: Vec<Res> = Vec::new();
    let mut readings: Vec<Vec<&Res>> = Vec::new();
    for rev in revisions {
        for r in rev {
            match out
                .iter()
                .position(|x| x.token == r.token && x.template == r.template)
            {
                Some(i) => readings[i].push(r),
                None => {
                    out.push(r.clone());
                    readings.push(vec![r]);
                }
            }
        }
    }
    let mut disagree = BTreeSet::new();
    for (i, rs) in readings.iter().enumerate() {
        let links = rs
            .iter()
            .filter(|r| exposure(kind_of(r.token), &r.annotations) == Exposure::Link)
            .count();
        if links > 0 && links < rs.len() {
            disagree.insert(i);
            out[i]
                .annotations
                .insert(EXPOSURE.to_owned(), Value::String("host".to_owned()));
        }
        let ttls: Vec<Option<std::time::Duration>> = rs.iter().map(|r| horizon(r).ttl()).collect();
        if ttls.windows(2).any(|w| w[0] != w[1]) {
            match ttls.into_iter().collect::<Option<Vec<_>>>() {
                Some(all) => {
                    let least = all.into_iter().min().unwrap_or_default();
                    out[i]
                        .annotations
                        .insert(freshness::TTL_S.to_owned(), Value::from(least.as_secs()));
                }
                None => {
                    out[i].annotations.remove(freshness::TTL_S);
                }
            }
        }
    }
    (out, disagree)
}

// ── The plan (§2.3–§2.12) ────────────────────────────────────────────────

/// One deny (§2.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Deny {
    pub id: String,
    pub flows: Vec<&'static str>,
    pub messages: Vec<&'static str>,
    pub key_exprs: Vec<String>,
}

/// One downsampling rule (§2.8).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Rule {
    pub key_expr: String,
    pub freq: f64,
}

/// A face's downsampling block (§2.8).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Downsampling {
    pub id: String,
    pub link_protocols: Vec<String>,
    pub flows: Vec<&'static str>,
    pub messages: Vec<&'static str>,
    pub rules: Vec<Rule>,
}

/// One `gateway.south` filter (§2.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Filter {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub modes: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub region_names: Vec<String>,
}

/// One south subregion (§2.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct South {
    pub filters: Vec<Filter>,
}

/// The near router's `gateway` block (§2.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Gateway {
    pub south: Vec<South>,
}

/// The attachment, as the plan carries it (§2.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "shape")]
pub enum AttachPlan {
    #[serde(rename = "client")]
    Client,
    #[serde(rename = "south-region")]
    SouthRegion { region: String, gateway: Gateway },
}

/// A listed owner of state's tombstone window floor (§2.11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Window {
    pub service: String,
    pub min_s: u64,
}

/// A listed service's liveness evidence across the face (§2.12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Liveness {
    pub service: String,
    pub evidence: Vec<String>,
}

/// One face's plan (§2.3).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FacePlan {
    pub name: String,
    pub far: String,
    pub attach: AttachPlan,
    pub deny: Vec<Deny>,
    pub downsampling: Option<Downsampling>,
    pub windows: Vec<Window>,
    pub liveness: Vec<Liveness>,
    pub findings: Vec<Finding>,
}

/// A resource's name in a plan, `<iface>/<token>/<template>` (§0).
fn res_name(iface: &str, r: &Res) -> String {
    format!("{iface}/{}/{}", r.token.as_str(), r.template.as_str())
}

/// Derives each face's plan (§2.3), faces sorted by name; or the
/// deployment's errors, which refuse it.
pub fn plan(policy: &FacePolicy, deployment: &Deployment) -> Result<Vec<FacePlan>, Vec<Finding>> {
    let mut errors = Vec::new();
    let mut listed: BTreeSet<&str> = BTreeSet::new();
    for s in &policy.services {
        match deployment.implements.get(s) {
            None => errors.push(Finding::new(Code::ServiceUnknown, None, s.clone())),
            Some(ifaces) => {
                for i in ifaces {
                    if deployment
                        .interfaces
                        .get(i)
                        .is_some_and(|revs| !revs.is_empty())
                    {
                        listed.insert(i);
                    } else {
                        errors.push(Finding::new(Code::ContractUnknown, None, i.clone()));
                    }
                }
            }
        }
    }
    if !errors.is_empty() {
        sort_findings(&mut errors);
        return Err(errors);
    }
    let mut services: Vec<&String> = policy.services.iter().collect();
    services.sort_by_key(|s| policy.address(s));
    // Each listed interface read once across its revisions (§2.3).
    let view: View = listed
        .iter()
        .map(|i| ((*i).to_owned(), merge(&deployment.interfaces[*i])))
        .collect();
    Ok(policy
        .faces
        .iter()
        .map(|(name, face)| plan_face(policy, deployment, &view, &services, &listed, name, face))
        .collect())
}

/// Each listed interface's resources across its revisions, and the ones
/// they disagree on ([`merge`]).
type View = BTreeMap<String, (Vec<Res>, BTreeSet<usize>)>;

fn plan_face(
    policy: &FacePolicy,
    deployment: &Deployment,
    view: &View,
    services: &[&String],
    listed: &BTreeSet<&str>,
    name: &str,
    face: &Face,
) -> FacePlan {
    let mut findings = Vec::new();
    let resources_of = |iface: &str| -> &[Res] { &view[iface].0 };
    // Whether each resource of each listed interface crosses (§2.7).
    let crossing_of = |r: &Res| {
        crossing(
            r.token,
            r.template.has_params(),
            &exposure(kind_of(r.token), &r.annotations),
            face.expose,
        )
    };
    for iface in listed {
        let (resources, disagree) = &view[*iface];
        for (i, r) in resources.iter().enumerate() {
            if crossing_of(r).reason == Reason::Templated {
                findings.push(Finding::new(
                    Code::TemplatedExposure,
                    Some(name),
                    res_name(iface, r),
                ));
            }
            if face.expose == Expose::Link && disagree.contains(&i) {
                findings.push(Finding::new(
                    Code::ExposureDisagrees,
                    Some(name),
                    res_name(iface, r),
                ));
            }
        }
    }

    let mut deny = vec![
        plane("zk", ZK_DENY),
        plane("adv", ADV_DENY),
        plane("admin", ADMIN_DENY),
    ];
    // The name (§0) of every overlapped resource.
    let mut overlapped: BTreeSet<String> = BTreeSet::new();
    for token in PUSH_TOKENS {
        let any = listed.iter().any(|i| {
            resources_of(i)
                .iter()
                .any(|r| r.token == token && crossing_of(r).crosses())
        });
        if !any {
            deny.push(push_deny(
                token.as_str().to_owned(),
                vec![format!("zk2/*/*/*/{}/**", token.as_str())],
            ));
            continue;
        }
        for iface in listed {
            let items: Vec<Item<'_>> = resources_of(iface)
                .iter()
                .filter(|r| r.token == token)
                .map(|r| Item {
                    template: &r.template,
                    crosses: crossing_of(r).crosses(),
                })
                .collect();
            let root = format!("zk2/*/*/{iface}/{}", token.as_str());
            let keys = if items.iter().any(|i| i.crosses) {
                let (keys, exact) = carve(&root, &items);
                for c in items.iter().filter(|i| i.crosses) {
                    let hit = exact.iter().any(|&t| {
                        let t = items[t].template;
                        t.overlaps(c.template) && c.template.precedence(t) == Ordering::Greater
                    });
                    if hit {
                        let at = format!("{iface}/{}/{}", token.as_str(), c.template.as_str());
                        overlapped.insert(at.clone());
                        findings.push(Finding::new(Code::CarveOutOverlap, Some(name), at));
                    }
                }
                keys
            } else {
                vec![format!("{root}/**")]
            };
            if !keys.is_empty() {
                deny.push(push_deny(format!("{}:{iface}", token.as_str()), keys));
            }
        }
    }

    // The resources that cross, overlapped ones set aside.
    let crosses = |iface: &str, r: &Res| {
        crossing_of(r).crosses() && !overlapped.contains(&res_name(iface, r))
    };

    // Downsampling (§2.8), the interval against the horizon (§2.12).
    let mut rules = Vec::new();
    if let Some(freq) = face.freq() {
        for s in services {
            for iface in &deployment.implements[s.as_str()] {
                for r in resources_of(iface) {
                    if is_stream(r.token) && crosses(iface, r) {
                        rules.push(Rule {
                            key_expr: format!(
                                "zk2/{}/{iface}/{}/{}",
                                policy.address(s),
                                r.token.as_str(),
                                r.template.as_str()
                            ),
                            freq,
                        });
                    }
                }
            }
        }
        rules.sort_by(|a, b| a.key_expr.cmp(&b.key_expr));
        rules.dedup_by(|a, b| a.key_expr == b.key_expr);
        for iface in listed {
            for r in resources_of(iface) {
                if let (true, Some(ttl), Some(n)) = (
                    is_stream(r.token) && crosses(iface, r),
                    horizon(r).ttl(),
                    face.interval_s,
                ) && u128::from(n) * 2 > u128::from(ttl.as_secs())
                {
                    findings.push(Finding::new(
                        Code::IntervalOverHorizon,
                        Some(name),
                        res_name(iface, r),
                    ));
                }
            }
        }
    }
    let downsampling = (!rules.is_empty()).then(|| Downsampling {
        id: name.to_owned(),
        link_protocols: face.link_protocols.clone(),
        flows: vec!["egress"],
        messages: vec!["put"],
        rules,
    });

    // Windows (§2.11) and liveness (§2.12).
    let mut windows = Vec::new();
    let mut liveness = Vec::new();
    for s in services {
        let ifaces = &deployment.implements[s.as_str()];
        let resources = || {
            ifaces
                .iter()
                .flat_map(|i| resources_of(i).iter().map(move |r| (i, r)))
        };
        if resources().any(|(_, r)| is_state(r.token)) {
            windows.push(Window {
                service: policy.address(s),
                min_s: face.max_outage_s,
            });
        }
        let mut evidence: Vec<String> = resources()
            .filter(|(i, r)| crosses(i, r) && horizon(r).ttl().is_some())
            .map(|(i, r)| res_name(i, r))
            .collect();
        evidence.sort();
        evidence.dedup();
        liveness.push(Liveness {
            service: policy.address(s),
            evidence,
        });
    }

    sort_findings(&mut findings);
    FacePlan {
        name: name.to_owned(),
        far: face.far.clone(),
        attach: match &face.attach {
            Attach::Client => AttachPlan::Client,
            Attach::SouthRegion { region } => AttachPlan::SouthRegion {
                region: region.clone(),
                gateway: gateway(region),
            },
        },
        deny,
        downsampling,
        windows,
        liveness,
        findings,
    }
}

fn horizon(r: &Res) -> Horizon {
    freshness::horizon(kind_of(r.token), &r.annotations)
}

fn plane(id: &str, key: &str) -> Deny {
    Deny {
        id: id.to_owned(),
        flows: FLOWS.to_vec(),
        messages: MESSAGES.to_vec(),
        key_exprs: vec![key.to_owned()],
    }
}

fn push_deny(id: String, mut key_exprs: Vec<String>) -> Deny {
    key_exprs.sort();
    key_exprs.dedup();
    Deny {
        id,
        flows: FLOWS.to_vec(),
        messages: PUSH_MESSAGES.to_vec(),
        key_exprs,
    }
}

/// The near router's `gateway` block for a far router in `region` (§2.5):
/// its own peers and clients south first, then the region.
#[must_use]
pub fn gateway(region: &str) -> Gateway {
    Gateway {
        south: vec![
            South {
                filters: vec![Filter {
                    modes: vec!["peer".to_owned(), "client".to_owned()],
                    region_names: Vec::new(),
                }],
            },
            South {
                filters: vec![Filter {
                    modes: Vec::new(),
                    region_names: vec![region.to_owned()],
                }],
            },
        ],
    }
}

// ── The carve-out (§2.7) ─────────────────────────────────────────────────

struct Item<'a> {
    template: &'a Template,
    crosses: bool,
}

/// A segment's place in a pattern, and its group.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Seg {
    Lit(String),
    Param,
    Rest,
}

impl Seg {
    fn of(s: &Segment) -> Self {
        match s {
            Segment::Literal(l) => Seg::Lit(l.clone()),
            Segment::Param(_) => Seg::Param,
            Segment::Rest(_) => Seg::Rest,
        }
    }

    fn kind(&self) -> u8 {
        match self {
            Seg::Lit(_) => 0,
            Seg::Param => 1,
            Seg::Rest => 2,
        }
    }

    /// A segment's pattern (§0): `{p}` is `*`, `{p...}` is `*/**`.
    fn pattern(&self) -> &str {
        match self {
            Seg::Lit(l) => l,
            Seg::Param => "*",
            Seg::Rest => "*/**",
        }
    }
}

/// `root`, the prefix's pattern, and `suffix`, with a `**` written once
/// before a `/**` that follows it.
fn key(root: &str, prefix: &[Seg], suffix: &str) -> String {
    let mut k = root.to_owned();
    for s in prefix {
        k.push('/');
        k.push_str(s.pattern());
    }
    k.push_str(suffix);
    while k.contains("/**/**") {
        k = k.replace("/**/**", "/**");
    }
    k
}

/// The carve-out of one interface's templates of one token (§2.7): the
/// denies, and the indices of the templates denied by their own pattern in
/// step 3.3.
fn carve(root: &str, items: &[Item<'_>]) -> (Vec<String>, Vec<usize>) {
    let mut keys = Vec::new();
    let mut exact = Vec::new();
    let all: Vec<usize> = (0..items.len()).collect();
    carve_at(root, items, &[], &all, &mut keys, &mut exact);
    keys.sort();
    keys.dedup();
    (keys, exact)
}

fn carve_at(
    root: &str,
    items: &[Item<'_>],
    prefix: &[Seg],
    here: &[usize],
    keys: &mut Vec<String>,
    exact: &mut Vec<usize>,
) {
    let d = prefix.len();
    let crosses = |i: &usize| items[*i].crosses;
    // 1. Nothing crosses: the subtree.
    if !here.iter().any(crosses) {
        keys.push(key(root, prefix, "/**"));
        return;
    }
    // 2. Everything crosses.
    if here.iter().all(crosses) {
        return;
    }
    let segs = |i: usize| items[i].template.segments();
    let (equal, longer): (Vec<usize>, Vec<usize>) = here.iter().partition(|&&i| segs(i).len() == d);
    // 3.1 The template equal to the prefix.
    if equal.iter().any(|i| !crosses(i)) {
        keys.push(key(root, prefix, ""));
    }
    // 3.2 Nothing longer crosses.
    if !longer.iter().any(crosses) {
        keys.push(key(root, prefix, "/*/**"));
        return;
    }
    // 3.3 More than one kind of next segment: template by template.
    let kinds: BTreeSet<u8> = longer
        .iter()
        .map(|&i| Seg::of(&segs(i)[d]).kind())
        .collect();
    if kinds.len() > 1 {
        for &i in &longer {
            if !items[i].crosses {
                let pattern: Vec<Seg> = segs(i).iter().map(Seg::of).collect();
                keys.push(key(root, &pattern, ""));
                exact.push(i);
            }
        }
        return;
    }
    // 3.4 Group by the next segment.
    let mut groups: BTreeMap<Seg, Vec<usize>> = BTreeMap::new();
    for &i in &longer {
        groups.entry(Seg::of(&segs(i)[d])).or_default().push(i);
    }
    for (seg, group) in groups {
        let mut p = prefix.to_vec();
        p.push(seg);
        carve_at(root, items, &p, &group, keys, exact);
    }
}

// ── Windows (§2.11) ──────────────────────────────────────────────────────

/// Each listed owner of state whose configured window (`windows`, by
/// address; [`DEFAULT_WINDOW_S`] when absent) is below a face's outage
/// (§2.11).
#[must_use]
pub fn check_windows(
    policy: &FacePolicy,
    deployment: &Deployment,
    windows: &BTreeMap<String, u64>,
) -> Vec<Finding> {
    let mut out = Vec::new();
    for s in &policy.services {
        let Some(ifaces) = deployment.implements.get(s) else {
            continue;
        };
        let owns_state = ifaces.iter().any(|i| {
            deployment
                .interfaces
                .get(i)
                .is_some_and(|revs| revs.iter().flatten().any(|r| is_state(r.token)))
        });
        if !owns_state {
            continue;
        }
        let addr = policy.address(s);
        let w = windows.get(&addr).copied().unwrap_or(DEFAULT_WINDOW_S);
        for (name, face) in &policy.faces {
            if w < face.max_outage_s {
                out.push(Finding::new(
                    Code::WindowUnderOutage,
                    Some(name),
                    addr.clone(),
                ));
            }
        }
    }
    sort_findings(&mut out);
    out
}

// ── A router's block (§2.15) ─────────────────────────────────────────────

/// One deny of a router's block, as read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockDeny {
    pub flows: Vec<String>,
    pub messages: Vec<String>,
    pub key_exprs: Vec<String>,
}

/// One downsampling rule of a router's block, as read.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockRule {
    pub key_expr: String,
    pub freq: f64,
}

/// zenoh's `default_permission`, as a router's block runs it (core §11.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Posture {
    Deny,
    Allow,
}

/// One subject of a router's block, as read (§2.4, §2.15): zenoh 1.10.1's
/// `AclConfigSubjects`. A session matches it when it matches every property
/// the subject names, and one value of each.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockSubject {
    pub id: String,
    pub usernames: Vec<String>,
    pub cert_common_names: Vec<String>,
    /// `None` when the subject names no link protocol: it selects every one.
    /// An empty list, which zenoh 1.10.1 refuses to load (§2.15, 0.4), is
    /// read as `None`.
    pub link_protocols: Option<Vec<String>>,
    pub interfaces: Vec<String>,
    pub zids: Vec<String>,
}

impl BlockSubject {
    /// Whether it is bound to a principal: a user name or a CN.
    fn bound(&self) -> bool {
        !self.usernames.is_empty() || !self.cert_common_names.is_empty()
    }

    /// The link protocols it names; `None` for every one, an empty list
    /// included.
    fn protocols(&self) -> Option<&[String]> {
        self.link_protocols.as_deref().filter(|p| !p.is_empty())
    }

    /// Whether it selects the link protocol `p`.
    fn selects(&self, p: &str) -> bool {
        self.protocols().is_none_or(|l| l.iter().any(|x| x == p))
    }

    /// Whether it can match a session over `face` (§2.15): its link
    /// protocols are absent or name one of the face's. Interfaces and zids
    /// keep no session off a face: a zid is unauthenticated (core §11.3),
    /// and a face's link may report an interface.
    fn on_face(&self, face: &Face) -> bool {
        self.protocols()
            .is_none_or(|l| l.iter().any(|x| face.link_protocols.contains(x)))
    }

    /// Whether it is the far principal's (§2.15, 0.4): it names `far` alone,
    /// as its user names or as its CNs. zenoh matches one value of each
    /// property a subject names, so every session it matches presents
    /// `far`.
    fn far_alone(&self, far: &str) -> bool {
        let only = |v: &[String]| !v.is_empty() && v.iter().all(|x| x == far);
        only(&self.usernames) || only(&self.cert_common_names)
    }

    /// Whether it names the user name or CN `name`.
    fn names(&self, name: &str) -> bool {
        self.usernames
            .iter()
            .chain(&self.cert_common_names)
            .any(|x| x == name)
    }
}

/// One policy of a router's block, as read (§2.15, 0.4): the ids of the
/// subjects it lists. Its rules are not read here: which policies list a
/// subject is what the subject checks need.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockPolicy {
    pub subjects: Vec<String>,
}

/// A block's posture, every subject it defines and every policy's subject
/// list, when they were read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockSubjects {
    pub posture: Posture,
    pub subjects: Vec<BlockSubject>,
    pub policies: Vec<BlockPolicy>,
}

impl BlockSubjects {
    /// The indices of the policies that list the subject `id`.
    fn listing(&self, id: &str) -> BTreeSet<usize> {
        self.policies
            .iter()
            .enumerate()
            .filter(|(_, p)| p.subjects.iter().any(|s| s == id))
            .map(|(i, _)| i)
            .collect()
    }
}

/// What a router runs for one face, as somebody read it (§2.15).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Block {
    /// The deployment's namespace, empty for none (core §1.6).
    pub namespace: String,
    /// The denies of every policy that lists one subject that can match
    /// the far principal's sessions on the face (§2.15): a tool checks each
    /// such subject's denies alone, since any allow wins across subjects.
    pub deny: Vec<BlockDeny>,
    /// In the block's order.
    pub downsampling: Vec<BlockRule>,
    /// The block's posture, subjects and policies' subject lists, which
    /// [`check_subjects`] and [`check_narrowing`] read; `None` when they
    /// were not read.
    pub subjects: Option<BlockSubjects>,
}

/// Checks a router's block for the face `face` (§2.15).
#[must_use]
pub fn check_block(face: &str, block: &Block) -> Vec<Finding> {
    let mut out = Vec::new();
    let ns = block.namespace.trim_matches('/');
    let target = if ns.is_empty() {
        ZK_DENY.to_owned()
    } else {
        format!("{ns}/{ZK_DENY}")
    };
    if !zk_denied(&target, &block.deny) {
        out.push(Finding::new(Code::ZkNotDenied, Some(face), "@zk"));
    }
    let parsed: Vec<Option<&keyexpr>> = block
        .downsampling
        .iter()
        .map(|r| keyexpr::new(r.key_expr.as_str()).ok())
        .collect();
    for (j, r) in block.downsampling.iter().enumerate() {
        // `*`, `**` and `$*` each carry a `*` (zenoh-keyexpr's own test).
        if parsed[j].is_none_or(|k| k.as_str().contains('*')) {
            out.push(Finding::new(
                Code::WildcardDownsampling,
                Some(face),
                r.key_expr.clone(),
            ));
        }
        let hit = (0..j).any(|i| match (parsed[i], parsed[j]) {
            (Some(a), Some(b)) => a.intersects(b),
            _ => false,
        });
        if hit {
            out.push(Finding::new(
                Code::IntersectingDownsampling,
                Some(face),
                r.key_expr.clone(),
            ));
        }
    }
    sort_findings(&mut out);
    out
}

/// Whether every pair of the nine messages and both flows is denied on a
/// key expression that includes `target` (§2.15).
fn zk_denied(target: &str, deny: &[BlockDeny]) -> bool {
    let Ok(target) = keyexpr::new(target) else {
        return false;
    };
    FLOWS.iter().all(|flow| {
        MESSAGES.iter().all(|m| {
            deny.iter().any(|d| {
                d.flows.iter().any(|f| f == flow)
                    && d.messages.iter().any(|x| x == m)
                    && d.key_exprs
                        .iter()
                        .any(|k| keyexpr::new(k.as_str()).is_ok_and(|k| k.includes(target)))
            })
        })
    })
}

/// Checks a router's subjects against the face `name` they must keep to
/// its far principal (§2.4, §2.15):
/// - `near_subject_on_face`, each subject on the face bound by a user name
///   or a CN that is not the far principal's alone, whatever else it names
///   (0.4): zenoh ties no credential to a link, and matches one value of
///   each property, so a session over the face with another name the
///   subject binds matches it;
/// - `face_subject_beyond_face`, each face subject (on the face, bound by
///   no user name and no CN) whose link protocols are absent or name one
///   outside the face's: sessions off the face match it too (0.4);
/// - `face_subject_misplaced`, each face subject not listed in exactly the
///   policies that list a subject of the far principal's on the face, at
///   least one: it would grant every session over the face another
///   policy's rules, miss one of the far principal's denies, or, listed in
///   none, get the default (0.4);
/// - `face_subject_missing`, under `allow`, each link protocol of the face
///   that no face subject naming no interface and no zid names in its
///   `link_protocols`: a session over it that matches no other subject
///   gets the default, everything.
#[must_use]
pub fn check_subjects(name: &str, face: &Face, block: &BlockSubjects) -> Vec<Finding> {
    let mut out = Vec::new();
    let on_face: Vec<&BlockSubject> = block.subjects.iter().filter(|s| s.on_face(face)).collect();
    // The far principal's policies: those listing one of its subjects on
    // the face.
    let far_policies: BTreeSet<usize> = on_face
        .iter()
        .filter(|s| s.far_alone(&face.far))
        .flat_map(|s| block.listing(&s.id))
        .collect();
    for s in &on_face {
        let mut found = |code: Code| out.push(Finding::new(code, Some(name), s.id.clone()));
        if s.bound() {
            if !s.far_alone(&face.far) {
                found(Code::NearSubjectOnFace);
            }
            continue;
        }
        if s.protocols()
            .is_none_or(|l| l.iter().any(|x| !face.link_protocols.contains(x)))
        {
            found(Code::FaceSubjectBeyondFace);
        }
        let listed = block.listing(&s.id);
        if listed.is_empty() || listed != far_policies {
            found(Code::FaceSubjectMisplaced);
        }
    }
    if block.posture == Posture::Allow {
        for p in &face.link_protocols {
            let selected = block.subjects.iter().any(|s| {
                !s.bound()
                    && s.interfaces.is_empty()
                    && s.zids.is_empty()
                    && s.protocols().is_some_and(|l| l.contains(p))
            });
            if !selected {
                out.push(Finding::new(
                    Code::FaceSubjectMissing,
                    Some(name),
                    p.clone(),
                ));
            }
        }
    }
    sort_findings(&mut out);
    out
}

/// Checks a router's subjects against every face of `policy` at once
/// (§2.4, §2.15, 0.4): under `allow`, `narrowed_too_far` (no face, `at` the
/// name) for each user name or CN the block's subjects bind with a link
/// protocol of §2.1's list, outside every face whose far principal it is
/// not, that no subject naming no interface and no zid selects among those
/// naming that name or no user name and no CN. A session with that name
/// over that link protocol matches no subject, and is allowed everything
/// (core §11.3). Under `deny`, such a session gets nothing, and there is
/// nothing to report.
#[must_use]
pub fn check_narrowing(policy: &FacePolicy, block: &BlockSubjects) -> Vec<Finding> {
    let mut out = Vec::new();
    if block.posture != Posture::Allow {
        return out;
    }
    let names: BTreeSet<&str> = block
        .subjects
        .iter()
        .flat_map(|s| s.usernames.iter().chain(&s.cert_common_names))
        .map(String::as_str)
        .collect();
    for name in names {
        let kept: Vec<String> = narrowed(policy, |f| policy.faces[f].far == name)
            .unwrap_or_else(|| LINK_PROTOCOLS.iter().map(|p| (*p).to_owned()).collect());
        let bare = kept.iter().any(|p| {
            !block.subjects.iter().any(|s| {
                s.interfaces.is_empty()
                    && s.zids.is_empty()
                    && (!s.bound() || s.names(name))
                    && s.selects(p)
            })
        });
        if bare {
            out.push(Finding::new(Code::NarrowedTooFar, None, name));
        }
    }
    sort_findings(&mut out);
    out
}

/// The link protocols a subject bound to a principal is selected by, on the
/// router of `policy` (§2.4): every one of §2.1's closed list outside the
/// faces whose far principal it is not (`is_far`, by the face's name), in
/// the list's order. `None` when no face excludes any: the subject names
/// none. Empty when every protocol is excluded: the principal could
/// connect over none.
#[must_use]
pub fn narrowed(policy: &FacePolicy, is_far: impl Fn(&str) -> bool) -> Option<Vec<String>> {
    let excluded: BTreeSet<&str> = policy
        .faces
        .iter()
        .filter(|(name, _)| !is_far(name))
        .flat_map(|(_, f)| f.link_protocols.iter().map(String::as_str))
        .collect();
    (!excluded.is_empty()).then(|| {
        LINK_PROTOCOLS
            .iter()
            .filter(|p| !excluded.contains(*p))
            .map(|p| (*p).to_owned())
            .collect()
    })
}

// ── Lint (§2.1, §2.3, §2.11, §2.15) ──────────────────────────────────────

/// What a linter is given: the policy's text, and what else it has.
#[derive(Debug, Clone, Copy, Default)]
pub struct LintInput<'a> {
    pub text: &'a str,
    /// When given, the plan is derived, and its findings included.
    pub deployment: Option<&'a Deployment>,
    /// The windows the deployment configures, by address; needs the
    /// deployment.
    pub windows: Option<&'a BTreeMap<String, u64>>,
    /// A router's block, and the face it guards.
    pub block: Option<(&'a str, &'a Block)>,
}

/// Every finding of a face policy and of what comes with it, sorted. The
/// plan, the windows and the block are checked only when the policy has no
/// error.
#[must_use]
pub fn lint(input: &LintInput<'_>) -> Vec<Finding> {
    let parsed = parse(input.text);
    let mut out = parsed.findings;
    let Some(policy) = parsed.policy else {
        return out;
    };
    if let Some(d) = input.deployment {
        match plan(&policy, d) {
            Ok(plans) => out.extend(plans.into_iter().flat_map(|p| p.findings)),
            Err(f) => out.extend(f),
        }
        if let Some(w) = input.windows {
            out.extend(check_windows(&policy, d, w));
        }
    }
    if let Some((face, block)) = input.block {
        out.extend(check_block(face, block));
        if let Some(s) = &block.subjects {
            if let Some(f) = policy.faces.get(face) {
                out.extend(check_subjects(face, f, s));
            }
            out.extend(check_narrowing(&policy, s));
        }
    }
    sort_findings(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ke(s: &str) -> &keyexpr {
        keyexpr::new(s).unwrap()
    }

    /// The zenoh facts the plan rests on (§0, §2.6, §2.7).
    #[test]
    fn key_expression_facts() {
        // `**` matches zero chunks: a subtree deny includes its prefix, and
        // `zk2/**/@zk/**` the contract keys.
        assert!(ke("zk2/*/*/m.v1/stream/a/**").includes(ke("zk2/h/s/m.v1/stream/a")));
        assert!(ke(ZK_DENY).includes(ke("zk2/@zk/contract/m.v1/abc")));
        assert!(ke(ZK_DENY).includes(ke("zk2/h/s/@zk/instance/0123456789abcdef")));
        assert!(ke("**/@zk/**").includes(ke(ZK_DENY)));
        // A rest parameter takes one or more chunks: `*/**` leaves its prefix.
        assert!(!ke("zk2/*/*/m.v1/events/logs/*/**").includes(ke("zk2/h/s/m.v1/events/logs")));
        assert!(ke("zk2/*/*/m.v1/events/logs/*/**").includes(ke("zk2/h/s/m.v1/events/logs/a/b")));
        // `*` never matches a verbatim chunk, so a wholesale deny spells it.
        assert!(!ke("zk2/*/*/*/stream/**").includes(ke("zk2/h/s/m.v1/@stream/x")));
        assert!(ke("zk2/*/*/*/@stream/**").includes(ke("zk2/h/s/m.v1/@stream/x")));
        assert!(ke(ADV_DENY).includes(ke("zk2/h/s/m.v1/stream/x/@adv/pub/abc")));
    }

    #[test]
    fn keys_collapse_a_trailing_rest() {
        assert_eq!(
            key("r", &[Seg::Lit("x".into()), Seg::Rest], "/**"),
            "r/x/*/**"
        );
        assert_eq!(key("r", &[], "/**"), "r/**");
        assert_eq!(key("r", &[Seg::Lit("a".into())], "/*/**"), "r/a/*/**");
    }

    fn res(template: &str, annotations: Value) -> Res {
        Res {
            template: Template::parse(template).unwrap(),
            token: KindToken::Stream,
            annotations: serde_json::from_value(annotations).unwrap(),
        }
    }

    /// Two revisions read together (§2.3, 0.3): exposed only where every
    /// one that declares a resource exposes it, a horizon only where every
    /// one gives one, the least; a resource one revision adds is its own.
    #[test]
    fn revisions_are_read_together_failing_closed() {
        let a = vec![
            res(
                "x",
                serde_json::json!({"link.exposure": "link", "freshness.ttl_s": 10}),
            ),
            res(
                "y",
                serde_json::json!({"link.exposure": "link", "freshness.ttl_s": 10}),
            ),
            res("w", serde_json::json!({"link.exposure": "LINK"})),
        ];
        let b = vec![
            res("x", serde_json::json!({"freshness.ttl_s": 4})),
            res(
                "y",
                serde_json::json!({"link.exposure": "link", "freshness.ttl_s": 0}),
            ),
            res("w", serde_json::json!({})),
            res("z", serde_json::json!({"link.exposure": "link"})),
        ];
        let (m, disagree) = merge(&[a, b]);
        let read = |t: &str| {
            let r = m.iter().find(|r| r.template.as_str() == t).unwrap();
            (exposure_of_res(r), horizon(r).ttl().map(|d| d.as_secs()))
        };
        assert_eq!(read("x"), (Exposure::Host, Some(4)));
        assert_eq!(read("y"), (Exposure::Link, None));
        assert_eq!(read("z"), (Exposure::Link, None));
        // Two readings of host, one invalid: no disagreement to report.
        assert_eq!(disagree, BTreeSet::from([0]));
    }

    fn exposure_of_res(r: &Res) -> Exposure {
        match exposure(kind_of(r.token), &r.annotations) {
            Exposure::Invalid(_) => Exposure::Host,
            e => e,
        }
    }

    fn face(far: &str, protocols: &[&str]) -> Face {
        Face {
            far: far.to_owned(),
            attach: Attach::Client,
            link_protocols: protocols.iter().map(|p| (*p).to_owned()).collect(),
            expose: Expose::Link,
            interval_s: Some(1),
            max_outage_s: 1,
            batch_bytes: 1,
            link_bps: 1,
            lease_ms: DEFAULT_LEASE_MS,
        }
    }

    /// A subject keeps the closed list outside every face whose far
    /// principal it is not (§2.4, 0.3).
    #[test]
    fn a_subject_is_narrowed_off_the_faces_it_is_not_far_of() {
        let policy = |faces: Vec<(&str, Face)>| FacePolicy {
            system: "lab".into(),
            services: vec![],
            faces: faces.into_iter().map(|(n, f)| (n.to_owned(), f)).collect(),
        };
        let two = policy(vec![
            ("radio", face("ground", &["unixsock-stream"])),
            ("sat", face("sat", &["serial", "unixpipe"])),
        ]);
        let of = |p: &str| narrowed(&two, |name| two.faces[name].far == p);
        assert_eq!(
            of("local").unwrap(),
            ["tcp", "udp", "tls", "quic", "vsock", "ws"]
        );
        assert_eq!(
            of("ground").unwrap(),
            [
                "tcp",
                "udp",
                "tls",
                "quic",
                "unixsock-stream",
                "vsock",
                "ws"
            ]
        );
        let one = policy(vec![("radio", face("ground", &["unixsock-stream"]))]);
        assert_eq!(narrowed(&one, |_| true), None);
        let all = policy(vec![("x", face("x", LINK_PROTOCOLS))]);
        assert_eq!(narrowed(&all, |_| false), Some(Vec::new()));
    }

    /// An empty `link_protocols`, which zenoh 1.10.1 refuses to load, is
    /// read as absent: the subject selects every link protocol (§2.15,
    /// 0.4).
    #[test]
    fn an_empty_link_protocol_list_reads_as_absent() {
        let radio = face("ground", &["unixsock-stream"]);
        let block = |links: Option<Vec<String>>| BlockSubjects {
            posture: Posture::Deny,
            subjects: vec![BlockSubject {
                id: "local".into(),
                usernames: vec!["local".into()],
                link_protocols: links,
                ..BlockSubject::default()
            }],
            policies: vec![BlockPolicy {
                subjects: vec!["local".into()],
            }],
        };
        let found = |links| check_subjects("radio", &radio, &block(links));
        assert_eq!(found(Some(Vec::new())), found(None));
        assert_eq!(found(None).len(), 1);
    }

    #[test]
    fn the_batch_against_the_lease_is_integer_arithmetic() {
        assert!(within_lease(1000, 2400, 10_000));
        assert!(!within_lease(1024, 2400, 10_000));
        assert!(within_lease(65_535, MAX_U32, MAX_U32));
    }
}
