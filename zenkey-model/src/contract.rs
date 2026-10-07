//! The contract model: an authoring file, resolved and linted.
//!
//! - **Defaults are expanded first (r3.3 D2):** a field comes from the
//!   resource, else `[defaults.<kind>]`, else `[defaults]`, else the
//!   built-in default of r3 §3.3. A default applies only where its field is
//!   legal, so `[defaults] priority = …` reaches streams and operations
//!   alike while `[defaults] history = true` skips an `@stream`.
//! - **Types are resolved** against the contract's [`SchemaSet`].
//! - **Every lint runs** ([`crate::diag::CODES`]), and the model exists only
//!   when no error was found.
//!
//! Set-level checks (two files, one interface; a requirement naming a
//! resource its interface lacks) are [`check_set`].

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::str::FromStr;

use serde_json::Value;

use crate::authoring::{
    self as a, ContractFile, DefaultsBlock, DefaultsSection, Encoding, Kind, ParamType,
    ResourceSpec, TypeRef,
};
use crate::chunk::is_ident;
use crate::diag::{Diagnostic, Report};
use crate::grammar::{IfaceId, KindToken};
use crate::schema::{Artifact, SchemaSet, TypeId};
use crate::template::{Segment, Template};

pub use crate::authoring::{
    Congestion, Fanout, HistoryParams, Priority, Reliability, Replies, RequireCardinality, Serving,
};

/// A valid contract, every default expanded.
#[derive(Debug, Clone)]
pub struct Contract {
    pub iface: IfaceId,
    /// Informative; not in the fingerprint.
    pub minor: Option<u32>,
    /// Documentation; not in the fingerprint.
    pub summary: Option<String>,
    /// The profiles used, sorted.
    pub uses: Vec<IfaceId>,
    /// Sorted by (kind token, template).
    pub resources: Vec<Resource>,
    pub requires: BTreeMap<String, Requirement>,
    /// Every schema artifact the contract lists or references, by id.
    pub artifacts: BTreeMap<String, Artifact>,
}

impl Contract {
    /// The resource declared under `token` with this exact template.
    #[must_use]
    pub fn resource(&self, token: KindToken, template: &str) -> Option<&Resource> {
        self.resources
            .iter()
            .find(|r| r.token == token && r.template.as_str() == template)
    }
}

/// One resource, fully explicit.
#[derive(Debug, Clone)]
pub struct Resource {
    pub template: Template,
    pub kind: Kind,
    pub token: KindToken,
    /// Documentation; not in the fingerprint.
    pub doc: Option<String>,
    pub params: BTreeMap<String, ParamType>,
    pub cardinality: Option<u64>,
    pub epoch: Option<String>,
    pub optional: bool,
    /// Sorted, deduplicated (a gate list is an AND).
    pub gate: Vec<String>,
    pub deprecated: Option<a::Deprecated>,
    /// `[defaults]`, then `[defaults.<kind>]`, then the resource's own.
    pub annotations: BTreeMap<String, Value>,
    pub body: Body,
}

#[derive(Debug, Clone)]
pub enum Body {
    Data(Data),
    Operation(Operation),
}

/// A stream, state or event resource.
#[derive(Debug, Clone)]
pub struct Data {
    pub type_: TypeId,
    pub attachment: Option<TypeId>,
    /// Set exactly when the type is a JSON Schema type.
    pub encoding: Option<Encoding>,
    /// Set exactly when the attachment is a JSON Schema type.
    pub attachment_encoding: Option<Encoding>,
    pub reliability: Reliability,
    pub congestion: Congestion,
    pub priority: Priority,
    pub express: bool,
    pub history: Option<HistoryParams>,
    /// Events only.
    pub rate: Option<Rate>,
    /// Events only: how far back a replay GET may reach.
    pub retention_s: Option<u64>,
}

/// An operation.
#[derive(Debug, Clone)]
pub struct Operation {
    pub request: TypeId,
    pub response: TypeId,
    pub error: Option<TypeId>,
    pub summary: Option<TypeId>,
    /// Set exactly when one of the operation's types is a JSON Schema type.
    pub encoding: Option<Encoding>,
    pub idempotent: bool,
    pub fanout: Fanout,
    pub serving: Serving,
    pub replies: Replies,
    pub timeout_ms: Option<u64>,
    /// A recommendation: replies inherit the caller's QoS.
    pub priority: Priority,
}

/// An event's declared rate (r3.3 D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rate {
    /// At most one per hour.
    Rare,
    /// At most one per minute.
    Low,
    /// At most n per hour.
    Burst(u32),
}

impl Rate {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "rare" => Some(Self::Rare),
            "low" => Some(Self::Low),
            _ => {
                let n = s.strip_prefix("burst(")?.strip_suffix("/h)")?;
                if n.is_empty()
                    || !n.bytes().all(|b| b.is_ascii_digit())
                    || (n.len() > 1 && n.starts_with('0'))
                {
                    return None;
                }
                n.parse().ok().filter(|n| *n > 0).map(Self::Burst)
            }
        }
    }
}

impl std::fmt::Display for Rate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rare => f.write_str("rare"),
            Self::Low => f.write_str("low"),
            Self::Burst(n) => write!(f, "burst({n}/h)"),
        }
    }
}

/// `"7d"` → seconds. Units: `s`, `m`, `h`, `d`, `w`.
fn parse_duration_s(s: &str) -> Option<u64> {
    let unit = s.chars().last()?;
    let n = &s[..s.len() - 1];
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mul = match unit {
        's' => 1,
        'm' => 60,
        'h' => 3600,
        'd' => 86_400,
        'w' => 604_800,
        _ => return None,
    };
    n.parse::<u64>().ok()?.checked_mul(mul).filter(|v| *v > 0)
}

/// A requirement, resolved.
#[derive(Debug, Clone)]
pub struct Requirement {
    pub interface: IfaceId,
    /// Sorted; `None` is everything.
    pub resources: Option<Vec<String>>,
    pub cardinality: RequireCardinality,
    pub optional: bool,
    /// Documentation; not in the fingerprint.
    pub doc: Option<String>,
    pub annotations: BTreeMap<String, Value>,
}

/// The outcome of loading one contract file.
#[derive(Debug)]
pub struct Loaded {
    /// `Some` exactly when the report has no error.
    pub contract: Option<Contract>,
    pub report: Report,
}

/// Loads, resolves and lints one contract file. Schema paths are relative
/// to the file's directory.
#[must_use]
pub fn load_path(path: &Path) -> Loaded {
    let mut report = Report::default();
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            report.push(Diagnostic::error(
                "E000",
                "file",
                format!("cannot be read: {e}"),
            ));
            return Loaded {
                contract: None,
                report,
            };
        }
    };
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().and_then(|n| n.to_str());
    load_str(&text, dir, name)
}

/// Loads, resolves and lints contract text. `dir` anchors schema paths;
/// `file_name`, when given, is checked against `<name>.v<major>.toml`.
#[must_use]
pub fn load_str(text: &str, dir: &Path, file_name: Option<&str>) -> Loaded {
    let mut report = Report::default();
    let file: ContractFile = match toml::from_str(text) {
        Ok(f) => f,
        Err(e) => {
            let msg = e.to_string().trim().replace('\n', " ");
            report.push(Diagnostic::error("E000", "file", msg));
            return Loaded {
                contract: None,
                report,
            };
        }
    };
    let contract = resolve(&file, dir, file_name, &mut report);
    let contract = if report.has_errors() { None } else { contract };
    if let Some(c) = &contract {
        crate::canonical::check_restrictions(&crate::canonical::canonical(c), &mut report);
    }
    let contract = if report.has_errors() { None } else { contract };
    Loaded { contract, report }
}

struct Ctx<'a> {
    report: &'a mut Report,
    schemas: SchemaSet,
    defaults: DefaultsSection,
    base: DefaultsBlock,
    profiles: BTreeSet<String>,
    minor: Option<u32>,
}

fn resolve(
    file: &ContractFile,
    dir: &Path,
    file_name: Option<&str>,
    report: &mut Report,
) -> Option<Contract> {
    let i = &file.interface;
    let iface = match IfaceId::new(&i.name, i.major) {
        Ok(id) => Some(id),
        Err(e) => {
            report.push(Diagnostic::error("E001", "interface", e.to_string()));
            None
        }
    };
    if i.minor.is_none() {
        report.push(Diagnostic::warning(
            "W104",
            "interface",
            "`minor` is missing",
        ));
    }
    if let (Some(id), Some(f)) = (&iface, file_name) {
        let want = format!("{id}.toml");
        if f != want {
            report.push(Diagnostic::warning(
                "W107",
                "file",
                format!("the file is named {f:?}; the convention is {want:?}"),
            ));
        }
    }
    let mut uses = BTreeSet::new();
    for u in &i.uses {
        match IfaceId::from_str(u) {
            Ok(p) => {
                uses.insert(p);
            }
            Err(_) => report.push(Diagnostic::error(
                "E002",
                "interface.uses",
                format!("{u:?} is not a profile id <name>.v<major>"),
            )),
        }
    }
    let profiles = uses.iter().map(|p| p.name().to_owned()).collect();
    let schemas = SchemaSet::load(dir, &file.schemas, report);
    let defaults = file.defaults.clone().unwrap_or_default();
    let mut cx = Ctx {
        report,
        schemas,
        base: defaults.base(),
        defaults,
        profiles,
        minor: i.minor,
    };
    check_defaults(&mut cx);

    let mut resources = Vec::new();
    for (raw, spec) in &file.resources {
        if let Some(r) = resolve_resource(raw, spec, &mut cx) {
            resources.push(r);
        }
    }
    resources.sort_by(|x: &Resource, y: &Resource| {
        (x.token.as_str(), x.template.as_str()).cmp(&(y.token.as_str(), y.template.as_str()))
    });
    check_shapes(file, cx.report);
    check_overlaps(&resources, cx.report);
    for (raw, spec) in &file.resources {
        if let Some(d) = &spec.deprecated
            && let Some(to) = &d.replaced_by
            && (to == raw || !file.resources.contains_key(to))
        {
            cx.report.push(Diagnostic::error(
                "E031",
                at_res(raw),
                format!(
                    "`deprecated.replaced_by` {to:?} names no other template of this interface"
                ),
            ));
        }
    }

    let mut requires = BTreeMap::new();
    for (role, req) in &file.requires {
        if let Some(r) = resolve_requirement(role, req, &mut cx) {
            requires.insert(role.clone(), r);
        }
    }

    Some(Contract {
        iface: iface?,
        minor: i.minor,
        summary: i.summary.clone(),
        uses: uses.into_iter().collect(),
        resources,
        requires,
        artifacts: cx.schemas.artifacts().clone(),
    })
}

/// The key's kind chunk for a resource (r3.3 D3).
#[must_use]
pub fn token_of(kind: Kind, explicit: bool) -> KindToken {
    match (kind, explicit) {
        (Kind::Stream, false) => KindToken::Stream,
        (Kind::Stream, true) => KindToken::ExplicitStream,
        (Kind::State, false) => KindToken::State,
        (Kind::State, true) => KindToken::ExplicitState,
        (Kind::Event, _) => KindToken::Events,
        (Kind::Operation, _) => KindToken::Op,
    }
}

fn at_res(raw: &str) -> String {
    format!("resources.{raw:?}")
}

const DATA_FIELDS: &[&str] = &[
    "type",
    "attachment",
    "encoding",
    "attachment_encoding",
    "reliability",
    "congestion",
    "priority",
    "express",
];
const OP_FIELDS: &[&str] = &[
    "request",
    "response",
    "error",
    "summary",
    "encoding",
    "idempotent",
    "fanout",
    "serving",
    "replies",
    "timeout_ms",
    "priority",
];

/// `None` when `field` is legal for the kind; otherwise the code to report.
fn illegal(kind: Kind, explicit: bool, field: &str) -> Option<&'static str> {
    let ok = match field {
        "history" => {
            return (!(matches!(kind, Kind::Stream | Kind::State) && !explicit)).then_some("E019");
        }
        "explicit" => matches!(kind, Kind::Stream | Kind::State),
        "rate" | "retention" => kind == Kind::Event,
        f if kind == Kind::Operation => OP_FIELDS.contains(&f),
        f => DATA_FIELDS.contains(&f),
    };
    (!ok).then_some("E014")
}

fn block_fields(b: &DefaultsBlock) -> Vec<&'static str> {
    let mut v = Vec::new();
    let mut p = |set: bool, n: &'static str| {
        if set {
            v.push(n);
        }
    };
    p(b.encoding.is_some(), "encoding");
    p(b.attachment_encoding.is_some(), "attachment_encoding");
    p(b.reliability.is_some(), "reliability");
    p(b.congestion.is_some(), "congestion");
    p(b.priority.is_some(), "priority");
    p(b.express.is_some(), "express");
    p(b.history.is_some(), "history");
    p(b.idempotent.is_some(), "idempotent");
    p(b.fanout.is_some(), "fanout");
    p(b.serving.is_some(), "serving");
    p(b.replies.is_some(), "replies");
    p(b.timeout_ms.is_some(), "timeout_ms");
    v
}

fn spec_fields(s: &ResourceSpec) -> Vec<&'static str> {
    let mut v = Vec::new();
    let mut p = |set: bool, n: &'static str| {
        if set {
            v.push(n);
        }
    };
    p(s.type_.is_some(), "type");
    p(s.attachment.is_some(), "attachment");
    p(s.encoding.is_some(), "encoding");
    p(s.attachment_encoding.is_some(), "attachment_encoding");
    p(s.explicit.is_some(), "explicit");
    p(s.reliability.is_some(), "reliability");
    p(s.congestion.is_some(), "congestion");
    p(s.priority.is_some(), "priority");
    p(s.express.is_some(), "express");
    p(s.history.is_some(), "history");
    p(s.rate.is_some(), "rate");
    p(s.retention.is_some(), "retention");
    p(s.request.is_some(), "request");
    p(s.response.is_some(), "response");
    p(s.error.is_some(), "error");
    p(s.idempotent.is_some(), "idempotent");
    p(s.fanout.is_some(), "fanout");
    p(s.serving.is_some(), "serving");
    p(s.replies.is_some(), "replies");
    p(s.summary.is_some(), "summary");
    p(s.timeout_ms.is_some(), "timeout_ms");
    v
}

fn check_defaults(cx: &mut Ctx<'_>) {
    check_annotations(&cx.base.annotations, "defaults", &cx.profiles, cx.report);
    for kind in [Kind::Stream, Kind::State, Kind::Event, Kind::Operation] {
        let Some(b) = cx.defaults.for_kind(kind) else {
            continue;
        };
        let at = format!("defaults.{}", kind.as_str());
        for f in block_fields(b) {
            if let Some(code) = illegal(kind, false, f) {
                cx.report.push(Diagnostic::error(
                    code,
                    at.clone(),
                    format!("`{f}` is not a field of kind = {:?}", kind.as_str()),
                ));
            }
        }
        check_annotations(&b.annotations, &at, &cx.profiles, cx.report);
        check_history(b.history.as_ref(), &at, cx.report);
    }
}

fn check_history(h: Option<&a::History>, at: &str, report: &mut Report) {
    if let Some(a::History::Params(p)) = h
        && p.depth == 0
    {
        report.push(Diagnostic::error(
            "E034",
            at,
            "`history.depth` must be at least 1",
        ));
    }
}

/// The interim annotation vocabularies of draft 1 (r3.3 D19), until each
/// profile ships its own table (#613).
const INTERIM_VOCABULARIES: &[(&str, &[&str])] = &[
    ("freshness", &["ttl_s"]),
    ("timing", &["period_ms", "deadline_ms", "lifespan_ms"]),
    ("telemetry", &["unit", "kind", "buckets", "semantic"]),
    ("link", &["exposure", "downsample_ms"]),
    ("arbitration", &["policy"]),
    ("desired", &["target_param", "target"]),
    ("alarms", &["severity_default"]),
    (
        "media",
        &[
            "tiers",
            "tier_param",
            "frame_clock",
            "control",
            "receiver_report",
        ],
    ),
    ("views", &["document"]),
    ("redundancy", &["election"]),
];

fn check_annotations(
    map: &BTreeMap<String, Value>,
    at: &str,
    profiles: &BTreeSet<String>,
    report: &mut Report,
) {
    for k in map.keys() {
        let Some((p, key)) = k.rsplit_once('.') else {
            report.push(Diagnostic::error(
                "E020",
                at,
                format!("annotation {k:?} is not <profile>.<key>"),
            ));
            continue;
        };
        if IfaceId::new(p, 1).is_err() || !is_ident(key) {
            report.push(Diagnostic::error(
                "E020",
                at,
                format!("annotation {k:?} is not <profile>.<key>"),
            ));
        } else if !profiles.contains(p) {
            report.push(Diagnostic::error(
                "E020",
                at,
                format!("annotation {k:?}: profile {p:?} is not listed in `uses`"),
            ));
        } else if let Some((_, keys)) = INTERIM_VOCABULARIES.iter().find(|(n, _)| *n == p)
            && !keys.contains(&key)
        {
            report.push(Diagnostic::warning(
                "W105",
                at,
                format!("annotation {k:?} is not in {p}'s interim vocabulary {keys:?}"),
            ));
        }
    }
}

fn check_gate(g: &str) -> bool {
    let Some((kind, name)) = g.split_once(':') else {
        return false;
    };
    let mut cs = name.chars();
    matches!(kind, "build" | "config" | "capability")
        && cs
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && cs.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_.-".contains(c))
}

/// resource > `[defaults.<kind>]` > `[defaults]`.
fn pick<T: Clone>(
    own: Option<&T>,
    kind: Option<&DefaultsBlock>,
    base: &DefaultsBlock,
    f: impl Fn(&DefaultsBlock) -> Option<&T>,
) -> Option<T> {
    own.or_else(|| kind.and_then(&f))
        .or_else(|| f(base))
        .cloned()
}

#[allow(clippy::too_many_lines)]
fn resolve_resource(raw: &str, spec: &ResourceSpec, cx: &mut Ctx<'_>) -> Option<Resource> {
    let at = at_res(raw);
    let template = match Template::parse(raw) {
        Ok(t) => t,
        Err(e) => {
            cx.report.push(Diagnostic::error("E010", at, e.to_string()));
            return None;
        }
    };
    let kind = spec.kind;
    let explicit = spec.explicit.unwrap_or(false);
    let token = token_of(kind, explicit);
    let errors_before = cx.report.errors().count();
    for f in spec_fields(spec) {
        if let Some(code) = illegal(kind, explicit, f) {
            let why = if code == "E019" {
                "`history` is for plain (non-explicit) stream and state resources only".to_owned()
            } else {
                format!("`{f}` is not a field of kind = {:?}", kind.as_str())
            };
            cx.report.push(Diagnostic::error(code, at.clone(), why));
        }
    }

    // Parameters.
    let tparams: Vec<(&str, bool)> = template.params().collect();
    for (name, rest) in &tparams {
        match spec.params.get(*name) {
            None => cx.report.push(Diagnostic::error(
                "E011",
                at.clone(),
                format!("template parameter {name:?} has no entry in `params`"),
            )),
            Some(t) => {
                if (*t == ParamType::Path) != *rest {
                    cx.report.push(Diagnostic::error(
                        "E012",
                        at.clone(),
                        format!("parameter {name:?}: `path` is the type of a rest parameter {{x...}}, and only of one"),
                    ));
                }
            }
        }
    }
    for name in spec.params.keys() {
        if !tparams.iter().any(|(n, _)| n == name) {
            cx.report.push(Diagnostic::error(
                "E011",
                at.clone(),
                format!("`params` declares {name:?}, which the template does not have"),
            ));
        }
    }
    match (template.has_params(), spec.cardinality) {
        (true, None) => cx.report.push(Diagnostic::error(
            "E013",
            at.clone(),
            "a templated resource declares its `cardinality`",
        )),
        (true, Some(0)) => cx.report.push(Diagnostic::error(
            "E013",
            at.clone(),
            "`cardinality` is at least 1",
        )),
        (false, Some(_)) => cx.report.push(Diagnostic::error(
            "E014",
            at.clone(),
            "`cardinality` on a template without parameters",
        )),
        _ => {}
    }
    let single = |n: &str| {
        template
            .segments()
            .iter()
            .any(|s| matches!(s, Segment::Param(p) if p == n))
    };
    if let Some(e) = &spec.epoch
        && !single(e)
    {
        cx.report.push(Diagnostic::error(
            "E022",
            at.clone(),
            format!("`epoch` {e:?} is not a single-chunk parameter of the template"),
        ));
    }

    // Gates.
    let mut gate: Vec<String> = spec
        .gate
        .as_ref()
        .map(|g| g.items().into_iter().map(str::to_owned).collect())
        .unwrap_or_default();
    gate.sort();
    gate.dedup();
    if !gate.is_empty() && !spec.optional {
        cx.report.push(Diagnostic::error(
            "E016",
            at.clone(),
            "`gate` without `optional = true`",
        ));
    }
    for g in &gate {
        if !check_gate(g) {
            let hint = if g.starts_with("feature:") {
                " (draft 0's `feature:` is `build:` in draft 1)"
            } else {
                ""
            };
            cx.report.push(Diagnostic::error(
                "E017",
                at.clone(),
                format!(
                    "gate {g:?} is not build:|config:|capability: plus [a-z0-9][a-z0-9_.-]*{hint}"
                ),
            ));
        }
    }
    if let Some(d) = &spec.deprecated
        && let Some(m) = cx.minor
        && d.since > m
    {
        cx.report.push(Diagnostic::error(
            "E031",
            at.clone(),
            format!(
                "`deprecated.since` = {} is later than the interface minor {m}",
                d.since
            ),
        ));
    }

    // Annotations: defaults, then the kind's defaults, then the resource's own.
    check_annotations(&spec.annotations, &at, &cx.profiles, cx.report);
    let kd = cx.defaults.for_kind(kind).cloned();
    let kd = kd.as_ref();
    let mut annotations = cx.base.annotations.clone();
    if let Some(k) = kd {
        annotations.extend(k.annotations.clone());
    }
    annotations.extend(spec.annotations.clone());

    let body = if kind == Kind::Operation {
        resolve_op(&at, spec, kd, &tparams, cx)
    } else {
        resolve_data(&at, spec, kind, explicit, kd, &template, cx)
    };
    if cx.report.errors().count() > errors_before {
        return None;
    }
    Some(Resource {
        template,
        kind,
        token,
        doc: spec.doc.clone(),
        params: spec.params.clone(),
        cardinality: spec.cardinality,
        epoch: spec.epoch.clone(),
        optional: spec.optional,
        gate,
        deprecated: spec.deprecated.clone(),
        annotations,
        body: body?,
    })
}

fn resolve_type(
    r: Option<&TypeRef>,
    what: &str,
    at: &str,
    template: Option<&Template>,
    cx: &mut Ctx<'_>,
) -> Option<TypeId> {
    let t = cx.schemas.resolve(r?, &format!("{at} {what}"), cx.report)?;
    if let (
        TypeId::Raw {
            media_param: Some(p),
            ..
        },
        Some(tpl),
    ) = (&t, template)
    {
        let ok = tpl
            .segments()
            .iter()
            .any(|s| matches!(s, Segment::Param(n) if n == p));
        if !ok {
            cx.report.push(Diagnostic::error(
                "E025",
                at,
                format!(
                    "{what}: `media_param` {p:?} is not a single-chunk parameter of the template"
                ),
            ));
        }
    }
    Some(t)
}

fn resolve_data(
    at: &str,
    spec: &ResourceSpec,
    kind: Kind,
    explicit: bool,
    kd: Option<&DefaultsBlock>,
    template: &Template,
    cx: &mut Ctx<'_>,
) -> Option<Body> {
    if spec.type_.is_none() {
        cx.report.push(Diagnostic::error(
            "E015",
            at,
            format!("kind = {:?} requires `type`", kind.as_str()),
        ));
    }
    let type_ = resolve_type(spec.type_.as_ref(), "type", at, Some(template), cx);
    let attachment = resolve_type(
        spec.attachment.as_ref(),
        "attachment",
        at,
        Some(template),
        cx,
    );
    let base = cx.base.clone();
    let encoding = match &type_ {
        Some(t) if t.is_json() => Some(
            pick(spec.encoding.as_ref(), kd, &base, |b| b.encoding.as_ref())
                .unwrap_or(Encoding::Json),
        ),
        _ => {
            if spec.encoding.is_some() {
                cx.report.push(Diagnostic::warning(
                    "W103",
                    at,
                    "`encoding` on a protobuf or raw type is ignored",
                ));
            }
            None
        }
    };
    let attachment_encoding = match &attachment {
        Some(t) if t.is_json() => Some(
            pick(spec.attachment_encoding.as_ref(), kd, &base, |b| {
                b.attachment_encoding.as_ref()
            })
            .unwrap_or(Encoding::Json),
        ),
        _ => {
            if spec.attachment_encoding.is_some() {
                cx.report.push(Diagnostic::warning(
                    "W103",
                    at,
                    "`attachment_encoding` without a JSON Schema attachment is ignored",
                ));
            }
            None
        }
    };
    let (rel, cong, prio) = match (kind, explicit) {
        (Kind::Stream, false) => (Reliability::BestEffort, Congestion::Drop, Priority::Data),
        (Kind::Stream, true) => (Reliability::BestEffort, Congestion::Drop, Priority::DataLow),
        _ => (Reliability::Reliable, Congestion::Block, Priority::Data),
    };
    let history = if illegal(kind, explicit, "history").is_none() {
        let h = pick(spec.history.as_ref(), kd, &base, |b| b.history.as_ref());
        check_history(spec.history.as_ref(), at, cx.report);
        match h {
            None | Some(a::History::Flag(false)) => None,
            Some(a::History::Flag(true)) => Some(HistoryParams {
                depth: 1,
                miss_detection_ms: None,
            }),
            Some(a::History::Params(p)) => Some(p),
        }
    } else {
        None
    };
    let (mut rate, mut retention_s) = (None, None);
    if kind == Kind::Event {
        match &spec.rate {
            None => cx.report.push(Diagnostic::error(
                "E015",
                at,
                "kind = \"event\" requires `rate`",
            )),
            Some(s) => {
                rate = Rate::parse(s);
                if rate.is_none() {
                    cx.report.push(Diagnostic::error(
                        "E026",
                        at,
                        format!("`rate` {s:?} is not \"rare\", \"low\" or \"burst(<n>/h)\""),
                    ));
                }
            }
        }
        match &spec.retention {
            None => cx.report.push(Diagnostic::error(
                "E015",
                at,
                "kind = \"event\" requires `retention`",
            )),
            Some(s) => {
                retention_s = parse_duration_s(s);
                if retention_s.is_none() {
                    cx.report.push(Diagnostic::error(
                        "E026",
                        at,
                        format!("`retention` {s:?} is not a positive duration <n>s|m|h|d|w"),
                    ));
                }
            }
        }
    }
    Some(Body::Data(Data {
        type_: type_?,
        attachment,
        encoding,
        attachment_encoding,
        reliability: pick(spec.reliability.as_ref(), kd, &base, |b| {
            b.reliability.as_ref()
        })
        .unwrap_or(rel),
        congestion: pick(spec.congestion.as_ref(), kd, &base, |b| {
            b.congestion.as_ref()
        })
        .unwrap_or(cong),
        priority: pick(spec.priority.as_ref(), kd, &base, |b| b.priority.as_ref()).unwrap_or(prio),
        express: pick(spec.express.as_ref(), kd, &base, |b| b.express.as_ref()).unwrap_or(false),
        history,
        rate,
        retention_s,
    }))
}

fn resolve_op(
    at: &str,
    spec: &ResourceSpec,
    kd: Option<&DefaultsBlock>,
    tparams: &[(&str, bool)],
    cx: &mut Ctx<'_>,
) -> Option<Body> {
    for (f, set) in [
        ("request", spec.request.is_some()),
        ("response", spec.response.is_some()),
    ] {
        if !set {
            cx.report.push(Diagnostic::error(
                "E015",
                at,
                format!("kind = \"operation\" requires `{f}`"),
            ));
        }
    }
    let request = resolve_type(spec.request.as_ref(), "request", at, None, cx);
    let response = resolve_type(spec.response.as_ref(), "response", at, None, cx);
    let error = resolve_type(spec.error.as_ref(), "error", at, None, cx);
    let summary = resolve_type(spec.summary.as_ref(), "summary", at, None, cx);
    let base = cx.base.clone();
    let replies =
        pick(spec.replies.as_ref(), kd, &base, |b| b.replies.as_ref()).unwrap_or(Replies::One);
    if spec.summary.is_some() && replies != Replies::Many {
        cx.report.push(Diagnostic::error(
            "E033",
            at,
            "`summary` without `replies = \"many\"`",
        ));
    }
    let idempotent = pick(spec.idempotent.as_ref(), kd, &base, |b| {
        b.idempotent.as_ref()
    })
    .unwrap_or(false);
    let serving = pick(spec.serving.as_ref(), kd, &base, |b| b.serving.as_ref())
        .unwrap_or(Serving::Exclusive);
    if serving == Serving::Replicated && !idempotent {
        cx.report.push(Diagnostic::error(
            "E018",
            at,
            "`serving = \"replicated\"` requires `idempotent = true`",
        ));
    }
    if let Some(req) = &request
        && let Some(fields) = cx.schemas.field_names(req)
    {
        let rep: Vec<&str> = tparams
            .iter()
            .map(|(n, _)| *n)
            .filter(|n| fields.iter().any(|f| f == n))
            .collect();
        if !rep.is_empty() {
            cx.report.push(Diagnostic::warning(
                "W102",
                at,
                format!("the request type repeats template parameter(s) {rep:?} (D21)"),
            ));
        }
    }
    let any_json = [&request, &response, &error, &summary]
        .into_iter()
        .any(|t| t.as_ref().is_some_and(TypeId::is_json));
    let encoding = if any_json {
        Some(
            pick(spec.encoding.as_ref(), kd, &base, |b| b.encoding.as_ref())
                .unwrap_or(Encoding::Json),
        )
    } else {
        if spec.encoding.is_some() {
            cx.report.push(Diagnostic::warning(
                "W103",
                at,
                "`encoding` on an operation without JSON Schema types is ignored",
            ));
        }
        None
    };
    Some(Body::Operation(Operation {
        request: request?,
        response: response?,
        error,
        summary,
        encoding,
        idempotent,
        fanout: pick(spec.fanout.as_ref(), kd, &base, |b| b.fanout.as_ref())
            .unwrap_or(Fanout::Forbidden),
        serving,
        replies,
        timeout_ms: pick(spec.timeout_ms.as_ref(), kd, &base, |b| {
            b.timeout_ms.as_ref()
        }),
        priority: pick(spec.priority.as_ref(), kd, &base, |b| b.priority.as_ref())
            .unwrap_or(Priority::InteractiveHigh),
    }))
}

/// The type a resource is read as, for the overlap lint (W101).
fn read_type(r: &Resource) -> (Option<&TypeId>, Option<&TypeId>) {
    match &r.body {
        Body::Data(d) => (Some(&d.type_), None),
        Body::Operation(o) => (Some(&o.request), Some(&o.response)),
    }
}

/// D1: one shape per kind token. Checked over every template that parses,
/// whatever else is wrong with its resource, so the codes reported do not
/// depend on the order lints run in.
fn check_shapes(file: &ContractFile, report: &mut Report) {
    let mut shapes: BTreeMap<(KindToken, String), &str> = BTreeMap::new();
    for (raw, spec) in &file.resources {
        let Ok(t) = Template::parse(raw) else {
            continue;
        };
        let token = token_of(spec.kind, spec.explicit.unwrap_or(false));
        let k = (token, t.shape());
        if let Some(first) = shapes.get(&k) {
            report.push(Diagnostic::error(
                "E021",
                at_res(raw),
                format!(
                    "same shape {:?} as {first:?} under the `{}` token",
                    t.shape(),
                    token.as_str()
                ),
            ));
        } else {
            shapes.insert(k, raw);
        }
    }
}

/// D1: overlapping templates under one token with different types are
/// flagged (W101). Only resolved resources have a type to compare.
fn check_overlaps(resources: &[Resource], report: &mut Report) {
    for (i, x) in resources.iter().enumerate() {
        for y in &resources[i + 1..] {
            if x.token == y.token
                && x.template.overlaps(&y.template)
                && read_type(x) != read_type(y)
            {
                report.push(Diagnostic::warning(
                    "W101",
                    at_res(y.template.as_str()),
                    format!(
                        "overlaps {:?} under `{}` with a different type; a key resolves most-literal-first",
                        x.template.as_str(),
                        x.token.as_str()
                    ),
                ));
            }
        }
    }
}

fn resolve_requirement(role: &str, req: &a::Requirement, cx: &mut Ctx<'_>) -> Option<Requirement> {
    let at = format!("requires.{role}");
    let before = cx.report.errors().count();
    if !is_ident(role) {
        cx.report.push(Diagnostic::error(
            "E030",
            at.clone(),
            format!("role {role:?} is not [a-z][a-z0-9_]*"),
        ));
    }
    let iface = IfaceId::from_str(&req.interface);
    if iface.is_err() {
        cx.report.push(Diagnostic::error(
            "E030",
            at.clone(),
            format!("interface {:?} is not <name>.v<major>", req.interface),
        ));
    }
    let resources = req.resources.as_ref().map(|v| {
        let mut v = v.clone();
        v.sort();
        v.dedup();
        v
    });
    for r in resources.iter().flatten() {
        if Template::parse(r).is_err() {
            cx.report.push(Diagnostic::error(
                "E030",
                at.clone(),
                format!("resource {r:?} is not a template"),
            ));
        }
    }
    if resources.as_ref().is_some_and(Vec::is_empty) {
        cx.report.push(Diagnostic::error(
            "E030",
            at.clone(),
            "`resources = []` consumes nothing; omit it to consume everything",
        ));
    }
    check_annotations(&req.annotations, &at, &cx.profiles, cx.report);
    if cx.report.errors().count() > before {
        return None;
    }
    Some(Requirement {
        interface: iface.ok()?,
        resources,
        cardinality: req.cardinality.unwrap_or(RequireCardinality::One),
        optional: req.optional,
        doc: req.doc.clone(),
        annotations: req.annotations.clone(),
    })
}

/// Checks across a set of contracts: one file per interface id (E036), and
/// every requirement naming resources its interface declares, when that
/// interface is in the set (E035).
#[must_use]
pub fn check_set(contracts: &[&Contract]) -> Report {
    let mut report = Report::default();
    let mut by_id: BTreeMap<String, &Contract> = BTreeMap::new();
    for c in contracts {
        let id = c.iface.to_string();
        if by_id.insert(id.clone(), c).is_some() {
            report.push(Diagnostic::error(
                "E036",
                "interface",
                format!("{id} is declared twice"),
            ));
        }
    }
    for c in contracts {
        for (role, req) in &c.requires {
            let Some(target) = by_id.get(&req.interface.to_string()) else {
                continue;
            };
            for r in req.resources.iter().flatten() {
                if !target.resources.iter().any(|x| x.template.as_str() == r) {
                    report.push(Diagnostic::error(
                        "E035",
                        format!("{}: requires.{role}", c.iface),
                        format!("{} declares no resource {r:?}", req.interface),
                    ));
                }
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(src: &str) -> Vec<&'static str> {
        load_str(src, Path::new("."), None).report.codes()
    }

    const HEAD: &str = "[interface]\nname = \"t\"\nmajor = 1\nminor = 0\n";

    #[test]
    fn rates_and_durations() {
        assert_eq!(Rate::parse("burst(12/h)"), Some(Rate::Burst(12)));
        for bad in ["burst(0/h)", "burst(012/h)", "burst(/h)", "often"] {
            assert_eq!(Rate::parse(bad), None, "{bad}");
        }
        assert_eq!(parse_duration_s("7d"), Some(604_800));
        assert_eq!(parse_duration_s("0s"), None);
        assert_eq!(parse_duration_s("7"), None);
    }

    #[test]
    fn defaults_expand_before_resolution() {
        let src = format!(
            "{HEAD}[defaults.stream]\npriority = \"data_low\"\n\
             [resources.\"a\"]\nkind = \"stream\"\ntype = {{ raw = \"image/jpeg\" }}\n\
             [resources.\"b\"]\nkind = \"stream\"\ntype = {{ raw = \"image/jpeg\" }}\npriority = \"data_high\"\n"
        );
        let l = load_str(&src, Path::new("."), None);
        let c = l.contract.expect("valid");
        let prio = |t: &str| match &c.resource(KindToken::Stream, t).unwrap().body {
            Body::Data(d) => d.priority,
            Body::Operation(_) => unreachable!(),
        };
        assert_eq!(prio("a"), Priority::DataLow);
        assert_eq!(prio("b"), Priority::DataHigh);
    }

    #[test]
    fn kind_tokens() {
        let src = format!(
            "{HEAD}[resources.\"a\"]\nkind = \"state\"\nexplicit = true\ntype = {{ raw = \"text/plain\" }}\n\
             [resources.\"b\"]\nkind = \"event\"\nrate = \"low\"\nretention = \"7d\"\ntype = {{ raw = \"text/plain\" }}\n"
        );
        let c = load_str(&src, Path::new("."), None)
            .contract
            .expect("valid");
        assert!(c.resource(KindToken::ExplicitState, "a").is_some());
        assert!(c.resource(KindToken::Events, "b").is_some());
    }

    #[test]
    fn lints_have_stable_codes() {
        let raw = "type = { raw = \"text/plain\" }";
        assert_eq!(
            codes(&format!(
                "{HEAD}[resources.\"a/{{x}}\"]\nkind = \"stream\"\n{raw}\n"
            )),
            ["E011", "E013"]
        );
        assert_eq!(
            codes(&format!(
                "{HEAD}[resources.\"a\"]\nkind = \"stream\"\n{raw}\ngate = \"feature:x\"\n"
            )),
            ["E016", "E017"]
        );
        assert_eq!(
            codes(&format!(
                "{HEAD}[resources.\"a\"]\nkind = \"stream\"\nexplicit = true\nhistory = true\n{raw}\n"
            )),
            ["E019"]
        );
        assert_eq!(
            codes(&format!(
                "{HEAD}[resources.\"a\"]\nkind = \"event\"\n{raw}\n"
            )),
            ["E015", "E015"]
        );
        assert_eq!(
            codes(&format!(
                "{HEAD}[resources.\"a/{{x}}\"]\nkind = \"stream\"\nparams = {{ x = \"string\" }}\ncardinality = 4\n{raw}\n\
                 [resources.\"a/{{y}}\"]\nkind = \"stream\"\nparams = {{ y = \"string\" }}\ncardinality = 4\n{raw}\n"
            )),
            ["E021"]
        );
        assert_eq!(
            codes(&format!(
                "{HEAD}[resources.\"a\"]\nkind = \"stream\"\n{raw}\nannotations = {{ \"freshness.ttl_s\" = 1 }}\n"
            )),
            ["E020"]
        );
        assert_eq!(
            codes(&format!(
                "{HEAD}[resources.\"a\"]\nkind = \"stream\"\ntype = \"json:Nope\"\n"
            )),
            ["E023"]
        );
        assert_eq!(codes("[interface]\nname = \"t\"\nmajor = 1\n"), ["W104"]);
        assert_eq!(codes("[interface]\nname = \"t\"\n"), ["E000"]);
    }

    #[test]
    fn same_shape_under_different_tokens_is_fine() {
        let raw = "type = { raw = \"text/plain\" }";
        let src = format!(
            "{HEAD}[resources.\"a/{{x}}\"]\nkind = \"stream\"\nparams = {{ x = \"string\" }}\ncardinality = 2\n{raw}\n\
             [resources.\"a/{{y}}\"]\nkind = \"state\"\nparams = {{ y = \"string\" }}\ncardinality = 2\n{raw}\n"
        );
        assert!(codes(&src).is_empty());
    }
}
