//! The Send pane (#184): publish and call, one form — the GUI face of the
//! write facade (`zenctl pub` / `zenctl call`), on zk2 since #614 (FL10).
//!
//! Publish and Call carried the same body editor and result region and
//! differed in what they write and whether anything answers. Two panes with
//! two message enums drifted apart; this is one pane with a mode toggle —
//! `put / publish` vs `get / call` — over one form. The call's answer stays
//! a distinct renderer, because a value, an envelope and silence are three
//! genuinely different results from a matching badge.
//!
//! **Publish writes a foreign key, as bytes.** A key under
//! `zk2/<system>/<service>/` is its owner's alone to write (P3, spec §6), so
//! the pane refuses one before anything moves — and says so as it is typed,
//! in `zenctl pub`'s words. Every other key is foreign, and no contract
//! types it: the body goes out as typed (text, or hex for raw bytes), with
//! the wire `Encoding` the form names or none, on the QoS axes the form
//! picks or zenoh's defaults (`WireQos`), and the pane says which (the
//! write-side O4). v1's encode ladder — the registry's type, the served
//! schema, "send raw" to opt out of it — left with the v1 registry.
//!
//! **Publishing can be sustained.** A repeat interval drives a real stream,
//! and the publication's #38 matching badge says whether anyone is consuming
//! it — a routing fact about *this* publisher, never a fleet verdict
//! (observer §2). Every send is logged, the log is bounded, and when the
//! bound bites the pane reports what it dropped (tooling guide O6). A tombstone
//! is an operator act on a key no contract describes, behind `--i-know`
//! (`zenkey_fleet::check_retire`).
//!
//! **Calls go through a contract.** An operation of a revision in hand —
//! loaded with `--contracts` or retrieved from the bus — at one address:
//! [`ResolvedTarget::parse`](zenkey_fleet::ResolvedTarget::parse), then
//! [`plan_call`](zenkey_fleet::plan_call), which refuses a fan-out to an
//! operation that forbids one before anything is sent (spec §5.1 O2), then
//! [`encode_request`](zenkey_fleet::encode_request) and the runtime's client
//! ([`call_operation`](zenkey_fleet::call_operation)). The request form is
//! read from the revision's own schema (`Revision::schema_view`): a JSON
//! Schema type's properties or a protobuf message's fields. The answer keeps
//! a value, an envelope and silence apart (O5), and silence is a non-verdict
//! — never an error.

use std::collections::VecDeque;
use std::sync::Arc;

use iced::widget::{Column, column, row};
use iced::{Element, Length};
use zenkey_fleet::report::{
    CallMode, EnvelopeView, OperationAnswer, OperationReport, PayloadRendering,
    PresenceAttribution, Rendered, RepliesView, TypeView,
};
use zenkey_fleet::{Revision, WireQos};

use crate::message::{Message, PaneMsg};
use crate::view::kit;
use crate::view::theme::Tone;
use crate::view::tokens::{Spacing, face, font};

/// How many send-log lines the pane keeps. Bounded on purpose: a 5 Hz stream
/// left running overnight is 150k lines, and an explorer that grows without
/// bound is the O6 bug this project already fixed once for the key tree.
pub const LOG_LINES: usize = 200;

/// What the form says while the panes replay a capture (#74; #614, FL10):
/// the window writes nothing to the live bus until the replay is left.
pub const REPLAY_LOCK: &str = "replay mode — the panes show a capture, and this window \
                               writes nothing to the live bus until you leave it";

/// Which half of the write facade the form is driving (#184).
///
/// A mode, not a pane: the body and the log survive the toggle, because "I
/// published this and now want to call about it" is one session, not two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SendMode {
    #[default]
    Publish,
    Call,
}

impl SendMode {
    pub const ALL: [SendMode; 2] = [SendMode::Publish, SendMode::Call];

    pub fn label(self) -> &'static str {
        match self {
            SendMode::Publish => "put / publish",
            SendMode::Call => "get / call",
        }
    }
}

/// One line of the in-pane send log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub text: String,
    /// Failures render differently — a send that errored must not read like
    /// one that landed.
    pub ok: bool,
}

/// Why the pane will not write `key`, when a zk2 service owns it (P3, spec
/// §6): a key under `zk2/<system>/<service>/` is its owner's alone to write,
/// wherever its namespace — the prefix before the `zk2` chunk is not read as
/// one (the tooling guide's O3) — and no checkbox moves the refusal. `zenctl
/// pub` refuses with the same words.
pub fn owned_refusal(key: &str) -> Option<String> {
    zenkey_fleet::owned_key(key).map(|o| {
        format!(
            "refused: {key} is a key {} owns, and only the owner writes its keys \
             (P3, spec §6) — act on the service through its operations",
            o.owner
        )
    })
}

/// The engine's spelling of one QoS axis value: the word the `qos_axes`
/// token gives it, so the picker, the log and a `.zrec` row cannot describe
/// one publication differently (#353).
fn axis_word(token: String, axis: usize) -> String {
    token
        .split(['/', '+'])
        .nth(axis)
        .unwrap_or_default()
        .to_string()
}

/// A priority, for the picker (`pick_list` needs `Display`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriorityChoice(pub zenoh::qos::Priority);

impl std::fmt::Display for PriorityChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let d = WireQos::DEFAULT;
        f.write_str(&axis_word(
            zenkey_fleet::report::qos_axes_token(self.0, d.congestion, d.reliability, false),
            0,
        ))
    }
}

/// Every priority zenoh carries, highest first.
pub const PRIORITIES: [PriorityChoice; 7] = {
    use zenoh::qos::Priority as P;
    [
        PriorityChoice(P::RealTime),
        PriorityChoice(P::InteractiveHigh),
        PriorityChoice(P::InteractiveLow),
        PriorityChoice(P::DataHigh),
        PriorityChoice(P::Data),
        PriorityChoice(P::DataLow),
        PriorityChoice(P::Background),
    ]
};

/// A congestion control, for the picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CongestionChoice(pub zenoh::qos::CongestionControl);

impl std::fmt::Display for CongestionChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let d = WireQos::DEFAULT;
        f.write_str(&axis_word(
            zenkey_fleet::report::qos_axes_token(d.priority, self.0, d.reliability, false),
            1,
        ))
    }
}

/// The two congestion controls a publisher declares.
pub const CONGESTIONS: [CongestionChoice; 2] = [
    CongestionChoice(zenoh::qos::CongestionControl::Drop),
    CongestionChoice(zenoh::qos::CongestionControl::Block),
];

/// A reliability, for the picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReliabilityChoice(pub zenoh::qos::Reliability);

impl std::fmt::Display for ReliabilityChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let d = WireQos::DEFAULT;
        f.write_str(&axis_word(
            zenkey_fleet::report::qos_axes_token(d.priority, d.congestion, self.0, false),
            2,
        ))
    }
}

/// The two reliabilities.
pub const RELIABILITIES: [ReliabilityChoice; 2] = [
    ReliabilityChoice(zenoh::qos::Reliability::BestEffort),
    ReliabilityChoice(zenoh::qos::Reliability::Reliable),
];

/// What the last publish put on the wire — the honesty line under the form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub bytes: usize,
    /// The wire `Encoding` set, when the form named one.
    pub encoding: Option<String>,
    /// The axes the publication was declared with, as the `qos_axes` token.
    pub qos: String,
}

/// One revision in hand, as the call mode's interface picker offers it.
///
/// Equal by interface and fingerprint: a bundle is content-addressed, so
/// one revision held twice is one choice.
#[derive(Debug, Clone)]
pub struct RevisionChoice(pub Arc<Revision>);

impl PartialEq for RevisionChoice {
    fn eq(&self, other: &RevisionChoice) -> bool {
        self.0.iface() == other.0.iface() && self.0.fingerprint() == other.0.fingerprint()
    }
}

impl std::fmt::Display for RevisionChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use zenkey_fleet::report::ContractSource;
        let source = match self.0.source() {
            ContractSource::Bus => "retrieved",
            ContractSource::History => "history",
            ContractSource::File => "offline",
            ContractSource::Bundle => "bundle",
        };
        write!(
            f,
            "{} @ {} ({source})",
            self.0.iface(),
            self.0.fingerprint().hex().fp16()
        )
    }
}

impl RevisionChoice {
    /// Every operation the revision declares, by name (`@op/<template>`).
    pub fn operations(&self) -> Vec<String> {
        self.0
            .contract()
            .resources
            .iter()
            .filter(|r| r.kind == zenkey_model::authoring::Kind::Operation)
            .map(resource_name)
            .collect()
    }
}

/// A resource's name, `<kind token>/<template>` — the runtime's spelling
/// (`zenkey::implementation::resource_name`), which is what `plan_call` and
/// `Revision::resource` take.
fn resource_name(r: &zenkey_model::contract::Resource) -> String {
    format!("{}/{}", r.token, r.template)
}

/// A type as the authoring format names it (spec §7.2): the message name
/// for protobuf, `json:<name>` for a JSON Schema type, the media type for a
/// raw one.
fn declared(t: &TypeView) -> String {
    match t.kind.as_str() {
        "jsonschema" => format!("json:{}", t.name),
        _ => t.name.clone(),
    }
}

/// The picked operation's declared shape, read once from the revision when
/// it is picked rather than per frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallShape {
    /// `@op/<template>`.
    pub name: String,
    /// The key a call goes out on, base-relative, the address symbolic.
    pub key: String,
    /// The template's parameters, in order.
    pub params: Vec<String>,
    /// The params box's example, from those parameters (`ns=…;iface=…`);
    /// empty when the template has none, so no example suggests one.
    pub params_example: String,
    pub request: String,
    pub response: String,
    pub error: Option<String>,
    /// `fanout = "allowed"`: a `*` or an unbound parameter may fan out.
    pub fanout_allowed: bool,
    pub idempotent: bool,
    /// `replies = "many"`.
    pub many: bool,
    /// The contract's recommended timeout, which the call waits by.
    pub timeout_ms: Option<u64>,
    pub doc: Option<String>,
}

impl CallShape {
    /// The shape of `operation` in `revision`, or the revision's own
    /// sentence saying it declares no such operation.
    pub fn of(revision: &Revision, operation: &str) -> Result<CallShape, String> {
        use zenkey_model::authoring::Kind;
        use zenkey_model::contract::{Body, Fanout, Replies};
        let r = revision.resource(operation, &[Kind::Operation])?;
        let Body::Operation(op) = &r.body else {
            return Err(format!("{operation} is not an operation"));
        };
        let view = |t| declared(&zenkey_fleet::type_view(t));
        let params: Vec<String> = r.template.params().map(|(n, _)| n.to_owned()).collect();
        Ok(CallShape {
            name: resource_name(r),
            key: format!(
                "zk2/{{system}}/{{service}}/{}/{}",
                revision.iface(),
                resource_name(r)
            ),
            params_example: params
                .iter()
                .map(|p| format!("{p}=…"))
                .collect::<Vec<_>>()
                .join(";"),
            params,
            request: view(&op.request),
            response: view(&op.response),
            error: op.error.as_ref().map(view),
            fanout_allowed: op.fanout == Fanout::Allowed,
            idempotent: op.idempotent,
            many: op.replies == Replies::Many,
            timeout_ms: op.timeout_ms,
            doc: r.doc.clone(),
        })
    }
}

/// One field of an operation's request type, as the call mode scaffolds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaField {
    pub name: String,
    /// The declared type, when the schema names one.
    pub type_name: Option<String>,
    pub required: bool,
    /// A value of the field's shape, for the scaffolded body: what the
    /// request's JSON form accepts (a protobuf enum by its first value's
    /// name, a nested message with its own fields).
    pub placeholder: serde_json::Value,
}

impl SchemaField {
    /// `name: type*` — the star marks required, so a scaffolded body's
    /// mandatory half is legible before the send refuses it.
    pub fn label(&self) -> String {
        format!(
            "{}: {}{}",
            self.name,
            self.type_name.as_deref().unwrap_or("?"),
            if self.required { "*" } else { "" }
        )
    }
}

/// What the request form is built from (§7.2): the request type's fields,
/// or why there are none to scaffold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestForm {
    /// A JSON Schema type's properties, or a protobuf message's fields.
    Fields {
        declared: String,
        fields: Vec<SchemaField>,
    },
    /// A raw request type: the body goes out as typed, its media type said.
    Raw { media_type: String },
    /// The schema in the bundle could not be read into fields: why. Said,
    /// never rendered as "takes nothing".
    Unreadable { declared: String, why: String },
}

/// The request form of `operation` in `revision`, read from the bundle's own
/// schema (`Revision::schema_view`): a JSON Schema type's `properties` (its
/// `$defs` entry), or a protobuf message's fields from the descriptor set,
/// nested messages scaffolded with their own fields.
pub fn request_form(revision: &Revision, operation: &str) -> RequestForm {
    let view = match revision.schema_view(Some(operation), true) {
        Ok(v) => v,
        Err(why) => {
            return RequestForm::Unreadable {
                declared: operation.to_owned(),
                why,
            };
        }
    };
    let Some(request) = view.members.iter().find(|m| m.member == "request") else {
        return RequestForm::Unreadable {
            declared: operation.to_owned(),
            why: "the operation names no request type".into(),
        };
    };
    let t = &request.type_;
    let name = declared(t);
    if t.kind == "raw" {
        return RequestForm::Raw {
            media_type: t.name.clone(),
        };
    }
    let document = t
        .schema
        .as_ref()
        .and_then(|id| view.artifacts.iter().find(|a| &a.id == id))
        .and_then(|a| a.document.as_option());
    let Some(document) = document else {
        return RequestForm::Unreadable {
            declared: name,
            why: "the bundle carries no artifact for it".into(),
        };
    };
    let fields = match t.kind.as_str() {
        "jsonschema" => json_schema_fields(document, &t.name),
        "protobuf" => protobuf_fields(document, &t.name, 0),
        other => Err(format!("a {other} type is not one this pane reads")),
    };
    match fields {
        Ok(fields) => RequestForm::Fields {
            declared: name,
            fields,
        },
        Err(why) => RequestForm::Unreadable {
            declared: name,
            why,
        },
    }
}

/// A JSON Schema type's top-level properties, from its `$defs` entry.
fn json_schema_fields(doc: &serde_json::Value, name: &str) -> Result<Vec<SchemaField>, String> {
    let def = doc
        .pointer(&format!(
            "/$defs/{}",
            name.replace('~', "~0").replace('/', "~1")
        ))
        .ok_or_else(|| format!("its artifact defines no {name}"))?;
    let Some(props) = def.get("properties").and_then(|p| p.as_object()) else {
        return Ok(Vec::new());
    };
    let required: Vec<&str> = def
        .get("required")
        .and_then(|r| r.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    Ok(props
        .iter()
        .map(|(field, spec)| {
            let type_name = json_type(spec);
            SchemaField {
                name: field.clone(),
                placeholder: json_placeholder(spec, type_name.as_deref()),
                type_name,
                required: required.contains(&field.as_str()),
            }
        })
        .collect())
}

/// A property's type, as the schema names it: its `type`, the first
/// non-null of an `anyOf`, `enum` for an enumeration, `$ref` for a
/// reference.
fn json_type(spec: &serde_json::Value) -> Option<String> {
    if let Some(t) = spec.get("type").and_then(|t| t.as_str()) {
        return Some(t.to_owned());
    }
    if spec.get("enum").is_some() {
        return Some("enum".into());
    }
    if let Some(r) = spec.get("$ref").and_then(|r| r.as_str()) {
        return Some(r.rsplit('/').next().unwrap_or(r).to_owned());
    }
    spec.get("anyOf")
        .and_then(|a| a.as_array())
        .and_then(|a| {
            a.iter()
                .filter_map(|s| s.get("type").and_then(|t| t.as_str()))
                .find(|t| *t != "null")
        })
        .map(str::to_owned)
}

fn json_placeholder(spec: &serde_json::Value, type_name: Option<&str>) -> serde_json::Value {
    if let Some(first) = spec
        .get("enum")
        .and_then(|e| e.as_array())
        .and_then(|e| e.first())
    {
        return first.clone();
    }
    if let Some(c) = spec.get("const") {
        return c.clone();
    }
    match type_name {
        Some("integer") => serde_json::json!(0),
        Some("number") => serde_json::json!(0.0),
        Some("boolean") => serde_json::json!(false),
        Some("array") => serde_json::json!([]),
        Some("string") => serde_json::json!(""),
        _ if spec.get("$ref").is_some() || type_name == Some("object") => serde_json::json!({}),
        _ => serde_json::json!(""),
    }
}

/// How deep a protobuf scaffold follows nested messages: a recursive type
/// must not scaffold forever.
const NESTING: usize = 3;

/// A protobuf message's fields, from `schema_view`'s reading of the
/// descriptor set (its files' messages and enums, flattened by full name).
fn protobuf_fields(
    doc: &serde_json::Value,
    message: &str,
    depth: usize,
) -> Result<Vec<SchemaField>, String> {
    let found = proto_entries(doc).find(|m| m["name"] == message && m.get("fields").is_some());
    let Some(found) = found else {
        return Err(format!("its descriptor set defines no {message}"));
    };
    Ok(found["fields"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|f| {
            let ty = f["type"].as_str().unwrap_or_default();
            let label = f["label"].as_str();
            let repeated = label == Some("repeated");
            SchemaField {
                name: f["name"].as_str().unwrap_or_default().to_owned(),
                type_name: Some(if repeated {
                    format!("repeated {ty}")
                } else {
                    ty.to_owned()
                }),
                required: label == Some("required"),
                placeholder: if repeated {
                    serde_json::json!([])
                } else {
                    proto_placeholder(doc, ty, depth)
                },
            }
        })
        .collect())
}

/// Every message and enum the document lists, across its files.
fn proto_entries(doc: &serde_json::Value) -> impl Iterator<Item = &serde_json::Value> {
    doc["files"].as_array().into_iter().flatten().flat_map(|f| {
        f["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(f["enums"].as_array().into_iter().flatten())
    })
}

fn proto_placeholder(doc: &serde_json::Value, ty: &str, depth: usize) -> serde_json::Value {
    match ty {
        "double" | "float" => serde_json::json!(0.0),
        "int32" | "int64" | "uint32" | "uint64" | "sint32" | "sint64" | "fixed32" | "fixed64"
        | "sfixed32" | "sfixed64" => serde_json::json!(0),
        "bool" => serde_json::json!(false),
        "string" | "bytes" => serde_json::json!(""),
        t if t.starts_with("map<") => serde_json::json!({}),
        t => {
            // An enum, by its first value's name (proto3's JSON form takes
            // the name); a message, with its own fields, to a bound.
            if let Some(first) = proto_entries(doc)
                .find(|e| e["name"] == t && e.get("values").is_some())
                .and_then(|e| e["values"].as_array()?.first()?["name"].as_str())
            {
                return serde_json::json!(first);
            }
            let mut obj = serde_json::Map::new();
            if depth < NESTING
                && let Ok(fields) = protobuf_fields(doc, t, depth + 1)
            {
                for f in fields {
                    obj.insert(f.name, f.placeholder);
                }
            }
            serde_json::Value::Object(obj)
        }
    }
}

/// The pane's editable state (owned by the app): one form, both modes (#184).
///
/// `body` is shared between the modes on purpose — it is the same editor.
/// The publish target (`key`, a full wire key) and the call target
/// (`target`, an address) are *not* shared, because they are different
/// questions and folding them would make the mode toggle silently retarget
/// a send.
#[derive(Debug, Clone)]
pub struct SendForm {
    pub mode: SendMode,

    // ── shared between the modes ────────────────────────────────────────
    /// The body: a publish's bytes as typed (or as hex), a call's request in
    /// its JSON form (or as typed, for a raw request type).
    pub body: String,
    pub in_flight: bool,
    /// Newest first, bounded by [`LOG_LINES`].
    pub log: VecDeque<LogLine>,
    /// How many lines the bound has discarded (O6).
    pub dropped: u64,

    // ── publish mode ────────────────────────────────────────────────────
    pub key: String,
    /// The body is hex — two digits a byte, whitespace ignored — rather
    /// than text: raw bytes, typed.
    pub hex: bool,
    /// The axes the publication is declared with.
    pub qos: WireQos,
    /// The user picked an axis by hand: the pane stops saying "zenoh's
    /// default" (#158's rule, kept: a default must never read as a choice).
    pub qos_touched: bool,
    /// The wire `Encoding`; empty sets none.
    pub encoding: String,
    /// Attachment text riding beside the payload; empty = none. Shipped
    /// verbatim (#117). A zk2 call carries none.
    pub attachment: String,
    /// Keep publishing on an interval once armed.
    pub repeat: bool,
    pub interval: String,
    /// What the last publish put on the wire.
    pub sent: Option<Sent>,
    /// A publication is armed and repeating.
    pub armed: bool,
    /// Matching status of the armed publication (#38). `None` = **not asked**,
    /// which is not "nobody listens" (O4).
    pub matching: Option<bool>,
    /// The last refusal, when a send did not happen at all.
    pub error: Option<crate::services::ServiceError>,
    /// `--i-know`: a tombstone on a key no contract describes is an operator
    /// act. `zenkey_fleet::check_retire` stays the judge — this only arms it.
    pub retire_i_know: bool,

    // ── call mode ───────────────────────────────────────────────────────
    /// The revisions in hand that declare an operation, refreshed when the
    /// mode is entered and when the picker opens.
    pub revisions: Vec<RevisionChoice>,
    pub revision: Option<RevisionChoice>,
    /// The picked revision's operations.
    pub operations: Vec<String>,
    pub operation: Option<String>,
    /// The picked operation's declared shape.
    pub shape: Option<CallShape>,
    /// Its request form. `None` = **not asked** (no operation picked), which
    /// is not "the request has no fields".
    pub request: Option<RequestForm>,
    /// The addresses presence says provide the picked interface; empty
    /// before a presence read, which the pane says.
    pub providers: Vec<String>,
    /// Whether a presence read stood behind `providers`.
    pub presence_read: bool,
    /// `<system>/<service>`, either position `*`.
    pub target: String,
    /// `name=value;…` — the template's values.
    pub params: String,
    /// The last call: its report, or why it was refused or failed.
    pub outcome: Option<Result<OperationReport, crate::services::ServiceError>>,
}

impl SendForm {
    /// Append a line, dropping the oldest when the bound bites — and counting
    /// what it dropped, because a silently-trimmed log claims a completeness
    /// it does not have (O6).
    pub fn log(&mut self, ok: bool, text: impl Into<String>) {
        self.log.push_front(LogLine {
            text: text.into(),
            ok,
        });
        while self.log.len() > LOG_LINES {
            self.log.pop_back();
            self.dropped += 1;
        }
    }

    /// The repeat interval in seconds, floored at 100 ms so a typo cannot
    /// turn the pane into a load generator.
    pub fn interval_secs(&self) -> f64 {
        self.interval.trim().parse::<f64>().unwrap_or(1.0).max(0.1)
    }

    /// The publish body's bytes: the text as typed, or the hex read as
    /// bytes — refused, before anything moves, when it is not hex.
    pub fn body_bytes(&self) -> Result<Vec<u8>, String> {
        if !self.hex {
            return Ok(self.body.clone().into_bytes());
        }
        let digits: Vec<u8> = self
            .body
            .bytes()
            .filter(|b| !b.is_ascii_whitespace())
            .collect();
        if !digits.len().is_multiple_of(2) {
            return Err(format!(
                "the body is not hex: {} digits, and a byte is two",
                digits.len()
            ));
        }
        digits
            .chunks(2)
            .map(|pair| {
                let s = std::str::from_utf8(pair).unwrap_or("");
                u8::from_str_radix(s, 16)
                    .map_err(|_| format!("the body is not hex: {s:?} is no byte"))
            })
            .collect()
    }

    /// A body from the request form's fields — every field present, each
    /// with a value of its declared shape.
    pub fn scaffold(&self) -> Option<String> {
        let Some(RequestForm::Fields { fields, .. }) = &self.request else {
            return None;
        };
        if fields.is_empty() {
            return None;
        }
        let mut obj = serde_json::Map::new();
        for f in fields {
            obj.insert(f.name.clone(), f.placeholder.clone());
        }
        serde_json::to_string_pretty(&serde_json::Value::Object(obj)).ok()
    }
}

impl Default for SendForm {
    fn default() -> Self {
        SendForm {
            mode: SendMode::Publish,
            body: String::new(),
            in_flight: false,
            log: VecDeque::new(),
            dropped: 0,
            key: String::new(),
            hex: false,
            qos: WireQos::DEFAULT,
            qos_touched: false,
            encoding: String::new(),
            attachment: String::new(),
            repeat: false,
            interval: "1.0".into(),
            sent: None,
            armed: false,
            matching: None,
            error: None,
            retire_i_know: false,
            revisions: Vec::new(),
            revision: None,
            operations: Vec::new(),
            operation: None,
            shape: None,
            request: None,
            providers: Vec::new(),
            presence_read: false,
            target: String::new(),
            params: String::new(),
            outcome: None,
        }
    }
}

/// Messages the pane emits (wrapped into the app's `Message::Pane(PaneMsg::Send)`).
///
/// One enum where there were two (#184): `PublishMsg` and `CallMsg` merged
/// under the rule that placed each of them — a message lives where its
/// failure is displayed, and every failure below is written into
/// [`SendForm`].
#[derive(Debug, Clone)]
pub enum SendMsg {
    /// The put/publish vs get/call toggle.
    ModeSelected(SendMode),

    // ── shared edits ────────────────────────────────────────────────────
    BodyChanged(String),

    // ── publish mode ────────────────────────────────────────────────────
    KeyChanged(String),
    AttachmentChanged(String),
    /// The body is hex, or text.
    HexToggled(bool),
    PriorityPicked(PriorityChoice),
    CongestionPicked(CongestionChoice),
    ReliabilityPicked(ReliabilityChoice),
    ExpressToggled(bool),
    EncodingChanged(String),
    RepeatToggled(bool),
    IntervalChanged(String),
    /// Declare and send once (arming the repeat if it is on).
    Send,
    /// Undeclare the armed publication.
    Stop,
    /// Retire the key with a tombstone (core §4.2, #115).
    Retire,
    /// The `--i-know` confirmation a tombstone needs.
    RetireIKnowToggled(bool),
    /// A declare→send round finished: what went out, the declared
    /// publication (kept when repeating), and its matching status.
    Ready(Result<Arc<crate::message::PublishOutcome>, crate::services::ServiceError>),
    /// One repeat tick fired.
    Tick,
    /// A repeat send landed (or did not).
    Sent(Result<usize, crate::services::ServiceError>),
    /// The armed publication was undeclared.
    Stopped(Result<(), crate::services::ServiceError>),
    /// A retire round finished (#115): the tombstone shipped (with the
    /// publication's matching fact), or it did not.
    Retired(Result<Option<bool>, crate::services::ServiceError>),

    // ── call mode ───────────────────────────────────────────────────────
    /// The interface picker opened: its revisions and the providers are
    /// read again, so a revision retrieved since is offered.
    RevisionsOpened,
    RevisionPicked(RevisionChoice),
    OperationPicked(String),
    TargetChanged(String),
    ParamsChanged(String),
    /// Fill the body editor from the request form.
    ScaffoldBody,
    Submit,
    /// The call finished — its report, or why it did not (#176). Its `Err`
    /// is written into [`SendForm::outcome`].
    Done(Result<Arc<OperationReport>, crate::services::ServiceError>),
}

fn msg(m: SendMsg) -> Message {
    Message::Pane(PaneMsg::Send(m))
}

/// Render the pane: the mode strip, then the mode's form. `replaying` is the
/// replay lock (#74): while the panes show a capture, nothing here writes.
pub fn pane(form: &SendForm, replaying: bool, sp: Spacing) -> Element<'_, Message> {
    let modes = kit::segmented(
        SendMode::ALL
            .into_iter()
            .map(|m| kit::Segment {
                value: m,
                label: m.label().to_string(),
                icon: None,
                count: None,
                tip: None,
            })
            .collect(),
        Some(form.mode),
        |m| msg(SendMsg::ModeSelected(m)),
    );
    let mut col = column![kit::section_header("Send", None), modes].spacing(sp.sm);
    if replaying {
        col = col.push(kit::callout(Tone::Mode, kit::caption(REPLAY_LOCK)));
    }

    col = match form.mode {
        SendMode::Publish => publish_body(col, form, replaying, sp),
        SendMode::Call => call_body(col, form, replaying, sp),
    };

    iced::widget::scrollable(col).height(Length::Fill).into()
}

// ── put / publish ───────────────────────────────────────────────────────

fn publish_body<'a>(
    mut col: Column<'a, Message>,
    form: &'a SendForm,
    replaying: bool,
    sp: Spacing,
) -> Column<'a, Message> {
    // Labelled fields (#562, after #559): the label survives typing; the
    // placeholder is only an example.
    let key = kit::form_field(
        "KEY",
        kit::input("demo/example/foo", &form.key)
            .on_input(|t| msg(SendMsg::KeyChanged(t)))
            .font(face::MONO)
            .size(font::CAPTION),
        Some(
            "a full wire key, namespace included — written as bytes on the session in no \
             namespace; a key a zk2 service owns is refused (P3)"
                .into(),
        ),
    );

    // The refusals the send would meet, said as the key is typed: P3 first,
    // then the wildcard (#504) — both before anything moves, neither moved
    // by any checkbox.
    let typed = form.key.trim();
    let mut checks = Column::new().spacing(sp.xs);
    if let Some(why) = owned_refusal(typed) {
        checks = checks.push(kit::callout(Tone::Negative, kit::caption(why)));
    } else if !typed.is_empty()
        && let Err(e) = zenkey_fleet::check_concrete(typed, zenkey_fleet::WriteAct::Put)
    {
        checks = checks.push(kit::callout(Tone::Negative, kit::caption(e.to_string())));
    }

    let hex = kit::check(form.hex)
        .label("hex")
        .on_toggle(|b| msg(SendMsg::HexToggled(b)))
        .text_size(font::CAPTION);
    let body = kit::form_field(
        "BODY",
        row![
            kit::input(
                if form.hex {
                    "7b 22 76 22 3a 31 7d"
                } else {
                    "hello"
                },
                &form.body
            )
            .on_input(|t| msg(SendMsg::BodyChanged(t)))
            .font(face::MONO)
            .size(font::CAPTION),
            hex,
        ]
        .spacing(sp.sm)
        .align_y(iced::Alignment::Center),
        Some(if form.hex {
            "raw bytes, two hex digits each (spaces ignored)".into()
        } else {
            "bytes as typed — a foreign key has no contract, so nothing encodes or \
             validates it"
                .into()
        }),
    );

    let qos = row![
        kit::picker(
            &PRIORITIES[..],
            Some(PriorityChoice(form.qos.priority)),
            |p| { msg(SendMsg::PriorityPicked(p)) }
        )
        .text_size(font::CAPTION),
        kit::picker(
            &CONGESTIONS[..],
            Some(CongestionChoice(form.qos.congestion)),
            |c| msg(SendMsg::CongestionPicked(c))
        )
        .text_size(font::CAPTION),
        kit::picker(
            &RELIABILITIES[..],
            Some(ReliabilityChoice(form.qos.reliability)),
            |r| msg(SendMsg::ReliabilityPicked(r))
        )
        .text_size(font::CAPTION),
        kit::check(form.qos.express)
            .label("express")
            .on_toggle(|b| msg(SendMsg::ExpressToggled(b)))
            .text_size(font::CAPTION),
    ]
    .spacing(sp.sm)
    .align_y(iced::Alignment::Center);
    // Say when the axes are zenoh's own default, so a default never reads
    // as the operator's choice — `zenctl pub`'s sentence.
    let qos_help = if !form.qos_touched && form.qos == WireQos::DEFAULT {
        format!(
            "qos: {} (zenoh's default — no contract types a foreign key)",
            WireQos::DEFAULT.token()
        )
    } else {
        format!("qos: {}", form.qos.token())
    };
    let qos = kit::form_field("QOS AXES", qos, Some(qos_help));

    let encoding = kit::form_field(
        "ENCODING",
        kit::input("text/plain", &form.encoding)
            .on_input(|t| msg(SendMsg::EncodingChanged(t)))
            .font(face::MONO)
            .size(font::CAPTION),
        Some("optional — the wire Encoding; empty sets none".into()),
    );

    let attachment = kit::form_field(
        "ATTACHMENT",
        kit::input("trace-id=4bf92f35", &form.attachment)
            .on_input(|t| msg(SendMsg::AttachmentChanged(t)))
            .font(face::MONO)
            .size(font::CAPTION),
        Some("optional — ships verbatim beside the body".into()),
    );

    let repeat = kit::check(form.repeat)
        .label("repeat")
        .on_toggle(|b| msg(SendMsg::RepeatToggled(b)))
        .text_size(font::CAPTION);
    let interval = row![
        kit::eyebrow("INTERVAL (S)"),
        kit::input("1", &form.interval)
            .on_input(|t| msg(SendMsg::IntervalChanged(t)))
            .size(font::CAPTION)
            .width(Length::Fixed(90.0)),
    ]
    .spacing(sp.xs)
    .align_y(iced::Alignment::Center);

    let ready = !form.key.trim().is_empty() && !form.in_flight && !replaying;
    let mut send = kit::primary(kit::caption(if form.in_flight {
        "sending…"
    } else if form.armed {
        "re-send"
    } else {
        "send"
    }))
    .padding(sp.xs);
    if ready {
        send = send.on_press(msg(SendMsg::Send));
    }
    let mut controls = row![send].spacing(sp.sm);
    if form.armed {
        controls = controls.push(
            kit::secondary(kit::caption("stop"))
                .padding(sp.xs)
                .on_press(msg(SendMsg::Stop)),
        );
    }
    // Retire (#115): a tombstone, not an empty put — and on a key no
    // contract describes, always the operator act behind `--i-know`.
    let mut retire = kit::danger(kit::caption("retire")).padding(sp.xs);
    if ready && form.retire_i_know {
        retire = retire.on_press(msg(SendMsg::Retire));
    }
    controls = controls.push(retire);
    let i_know = kit::check(form.retire_i_know)
        .label(
            "--i-know: a tombstone on a key no contract describes — nothing says a delete \
             is its writer's to send",
        )
        .on_toggle(|b| msg(SendMsg::RetireIKnowToggled(b)))
        .text_size(font::CAPTION);

    col = col.push(key);
    col = col.push(checks);
    col = col.push(body);
    col = col.push(qos);
    col = col.push(row![encoding, attachment].spacing(sp.sm));
    col = col.push(
        row![repeat, interval]
            .spacing(sp.sm)
            .align_y(iced::Alignment::Center),
    );
    col = col.push(controls);
    if !form.key.trim().is_empty() {
        col = col.push(i_know);
    }

    if let Some(e) = &form.error {
        col = col.push(kit::error(e.to_string()));
    }
    if let Some(line) = provenance(form) {
        col = col.push(line);
    }
    if form.armed {
        // A routing fact about this publisher (#38): commentary when asked,
        // the outline of a question not asked when not.
        let tone = if form.matching.is_some() {
            Tone::Info
        } else {
            Tone::Neutral
        };
        col = col.push(kit::status_chip(tone, matching_sentence(form.matching)));
    }
    col.push(log_view(form, sp))
}

/// What the last publish put on the wire: the bytes as typed, the encoding
/// set or none, and the axes — three facts, no verdict among them.
fn provenance(form: &SendForm) -> Option<Element<'_, Message>> {
    let sent = form.sent.as_ref()?;
    Some(kit::status_chip(
        Tone::Info,
        format!(
            "sent {} as typed → {} · qos {}",
            kit::plural(sent.bytes, "byte"),
            sent.encoding.as_deref().unwrap_or("(no encoding set)"),
            sent.qos
        ),
    ))
}

/// The #38 badge, worded exactly as the CLI words it: a routing fact about
/// *this* publisher. `false` is never a claim about the fleet, and `None` is
/// never rendered as `false`.
fn matching_sentence(matching: Option<bool>) -> String {
    match matching {
        Some(true) => "matching: a subscriber currently matches this publication".into(),
        Some(false) => "matching: no subscriber currently matches this publication — a \
                        routing fact about this publisher, not a fleet verdict (observer §2)"
            .into(),
        None => "matching: not asked".into(),
    }
}

/// The send log, on its own — the Activity dock's Publish stream (#183).
///
/// Split out of the pane so verifying a publish does not mean leaving the
/// form. The form asks a question; the log is what came back, and those are
/// two different regions of the workspace.
///
/// In a scrollable of its own: the dock is a fixed height, and a log bounded
/// at [`LOG_LINES`] is longer than it — without one its rows shared the
/// dock's height between them, and the ones past it were clipped (#614,
/// FL10; the first scene with lines in it showed both).
pub fn log_section(form: &SendForm, sp: Spacing) -> Element<'_, Message> {
    iced::widget::scrollable(log_view(form, sp))
        .height(Length::Fill)
        .into()
}

fn log_view(form: &SendForm, sp: Spacing) -> Element<'_, Message> {
    let mut col = Column::new().spacing(sp.xs);
    if form.log.is_empty() {
        return kit::empty(
            kit::EmptyKind::Empty,
            "No sends yet",
            format!(
                "Every publish, retire and call this window sends lands here, newest first — \
                 bounded at {LOG_LINES}, and the drops are counted."
            ),
        );
    }
    col = col.push(kit::muted(if form.dropped > 0 {
        format!(
            "send log — {} shown, {} dropped (bounded at {LOG_LINES})",
            form.log.len(),
            form.dropped
        )
    } else {
        format!("send log — {}", kit::plural(form.log.len(), "entry"))
    }));
    // Rows with a leading edge (#562, echo's anatomy): a failed send wears
    // danger's edge and says so in words — the colour never alone.
    for line in &form.log {
        let entry = kit::caption(line.text.as_str()).font(face::MONO);
        col = col.push(if line.ok {
            kit::edge_row(kit::Edge::Put, entry)
        } else {
            kit::edge_row(
                kit::Edge::Failed,
                row![kit::status_chip(Tone::Negative, "failed"), entry]
                    .spacing(sp.sm)
                    .align_y(iced::Alignment::Center),
            )
        });
    }
    col.into()
}

// ── get / call ──────────────────────────────────────────────────────────

fn call_body<'a>(
    mut col: Column<'a, Message>,
    form: &'a SendForm,
    replaying: bool,
    sp: Spacing,
) -> Column<'a, Message> {
    if form.revisions.is_empty() {
        return col.push(kit::empty(
            kit::EmptyKind::NotAsked,
            "No operation in hand",
            "Call mode plans against a contract revision held — loaded with --contracts or \
             retrieved from the bus — and none in hand declares an operation yet. \"Not \
             asked\" is not \"nothing serves\" (observer §1).",
        ));
    }

    let revision_pick = kit::picker(&form.revisions[..], form.revision.clone(), |r| {
        msg(SendMsg::RevisionPicked(r))
    })
    .on_open(msg(SendMsg::RevisionsOpened))
    .placeholder("interface")
    .text_size(font::CAPTION);
    let operation_pick = kit::picker(&form.operations[..], form.operation.clone(), |o| {
        msg(SendMsg::OperationPicked(o))
    })
    .placeholder("operation")
    .text_size(font::CAPTION);
    col = col.push(
        row![revision_pick, operation_pick]
            .spacing(sp.sm)
            .align_y(iced::Alignment::Center),
    );

    if let Some(shape) = &form.shape {
        col = col.push(shape_view(shape, form, sp));
    }

    let mut target = Column::new().spacing(sp.xs);
    target = target.push(kit::form_field(
        "ADDRESS",
        kit::input("robot1/nav", &form.target)
            .on_input(|t| msg(SendMsg::TargetChanged(t)))
            .font(face::MONO)
            .size(font::CAPTION),
        Some("<system>/<service>; a `*` position fans out — only to fanout = \"allowed\"".into()),
    ));
    if form.revision.is_some() {
        target = target.push(if form.providers.is_empty() {
            kit::muted(if form.presence_read {
                "presence shows no provider of this interface — type the address"
            } else {
                "no presence read — type the address"
            })
        } else {
            let mut providers = row![kit::eyebrow("IN PRESENCE")]
                .spacing(sp.xs)
                .align_y(iced::Alignment::Center);
            for p in &form.providers {
                providers = providers.push(
                    kit::ghost(kit::caption(p.as_str()).font(face::MONO))
                        .padding([0.0, sp.xs])
                        .on_press(msg(SendMsg::TargetChanged(p.clone()))),
                );
            }
            providers.wrap().into()
        });
    }
    col = col.push(target);

    let params_help = match &form.shape {
        Some(s) if !s.params.is_empty() => format!(
            "name=value;… — {}: each one not given is a wildcard",
            s.params.join(", ")
        ),
        Some(_) => "this operation's template has no parameters".into(),
        None => "name=value;… — the template's values".into(),
    };
    col = col.push(kit::form_field(
        "PARAMS",
        kit::input(
            form.shape
                .as_ref()
                .map_or("name=value;…", |s| s.params_example.as_str()),
            &form.params,
        )
        .on_input(|t| msg(SendMsg::ParamsChanged(t)))
        .font(face::MONO)
        .size(font::CAPTION),
        Some(params_help),
    ));
    let body_help = match &form.request {
        Some(RequestForm::Raw { media_type }) => {
            format!("{media_type} — a raw request: the bytes as typed")
        }
        Some(RequestForm::Fields { declared, .. } | RequestForm::Unreadable { declared, .. }) => {
            format!("{declared}, in its JSON form — encoded by the engine; empty sends {{}}")
        }
        None => "the request, in its JSON form".into(),
    };
    col = col.push(kit::form_field(
        "REQUEST",
        kit::input("{\"origin\": {\"latitude_deg\": 47.6}}", &form.body)
            .on_input(|t| msg(SendMsg::BodyChanged(t)))
            .font(face::MONO)
            .size(font::CAPTION),
        Some(body_help),
    ));

    let ready =
        form.shape.is_some() && !form.target.trim().is_empty() && !form.in_flight && !replaying;
    let mut submit = kit::primary(kit::caption(if form.in_flight {
        "calling…"
    } else {
        "call"
    }))
    .padding(sp.xs);
    if ready {
        submit = submit.on_press(msg(SendMsg::Submit));
    }
    col = col.push(submit);

    if let Some(outcome) = &form.outcome {
        col = col.push(outcome_view(outcome, sp));
    }
    col
}

/// The picked operation as its contract declares it: the key it goes out
/// on, its types, its fan-out and its request form.
fn shape_view<'a>(shape: &'a CallShape, form: &'a SendForm, sp: Spacing) -> Element<'a, Message> {
    let mut meta = Column::new().spacing(sp.xs);
    meta = meta.push(kit::mono(format!("→ {}", shape.key)));
    // The declared shape as fields (#562): an eyebrow over each value, "—"
    // where the contract declares nothing — absent, never guessed.
    let field = |label: &'static str, value: String| {
        kit::field(label, kit::caption(value).font(face::MONO))
    };
    meta = meta.push(kit::fields(vec![
        field("REQUEST", shape.request.clone()),
        field("RESPONSE", shape.response.clone()),
        field("ERROR", shape.error.clone().unwrap_or_else(|| "—".into())),
        field(
            "FANOUT",
            if shape.fanout_allowed {
                "allowed"
            } else {
                "forbidden"
            }
            .into(),
        ),
        field("IDEMPOTENT", shape.idempotent.to_string()),
        field("REPLIES", if shape.many { "many" } else { "one" }.into()),
        field(
            "TIMEOUT",
            shape
                .timeout_ms
                .map_or_else(|| "—".into(), |ms| format!("{ms} ms")),
        ),
    ]));
    if let Some(doc) = &shape.doc {
        meta = meta.push(kit::muted(doc.clone()));
    }
    // The request form (§7.2), from the revision's own schema: its fields
    // and a button that drops a body of their shapes in the editor.
    meta = match &form.request {
        Some(RequestForm::Fields { declared, fields }) if !fields.is_empty() => meta.push(
            row![
                kit::muted(format!(
                    "{declared} fields: {}",
                    fields
                        .iter()
                        .map(SchemaField::label)
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                kit::secondary(kit::caption("scaffold body"))
                    .padding([0.0, sp.xs])
                    .on_press(msg(SendMsg::ScaffoldBody)),
            ]
            .spacing(sp.sm)
            .align_y(iced::Alignment::Center),
        ),
        Some(RequestForm::Fields { declared, .. }) => {
            meta.push(kit::muted(format!("{declared} declares no fields")))
        }
        Some(RequestForm::Raw { media_type }) => meta.push(kit::muted(format!(
            "a raw request ({media_type}): no fields — the bytes as typed"
        ))),
        Some(RequestForm::Unreadable { declared, why }) => meta.push(kit::muted(format!(
            "{declared}'s fields could not be read: {why} — type its JSON form"
        ))),
        None => meta,
    };
    if !shape.fanout_allowed {
        // A rule the call will meet, said before it: a caution, not an error
        // — nothing has been refused yet.
        meta = meta.push(kit::callout(
            Tone::Caution,
            kit::caption(
                "fanout = \"forbidden\" — a `*` in the address, or a parameter not given, is \
                 refused before anything is sent (spec §5.1 O2)",
            ),
        ));
    }
    meta.into()
}

/// What presence said of a service that sent no answer (O5, §8.1), as a
/// sentence a reader can act on — what this reader could see, never a
/// verdict that the service is gone.
pub fn attribution_sentence(a: PresenceAttribution) -> &'static str {
    match a {
        PresenceAttribution::Present => {
            "it holds the interface's token: up, and did not answer — access control \
             refused the call, the server is frozen, or the reply was lost"
        }
        PresenceAttribution::InstanceOnly => {
            "it holds an instance token and not this interface's: a standby, or an \
             interface in its tokenless set"
        }
        PresenceAttribution::NoTokenVisible => {
            "no token of the service is visible to this reader — what this reader could \
             see, not a verdict that it is gone"
        }
        PresenceAttribution::Unknown => {
            "the presence read was possibly incomplete and saw no interface token"
        }
        PresenceAttribution::Unobservable => "presence cannot be observed from here (R7)",
    }
}

/// One line for the send log: what a call got back, in O5's four words —
/// a value, a refusal, a malformed envelope, silence — or a fan-out's tally.
pub fn call_summary(report: &OperationReport) -> String {
    let head = format!(
        "called {} {} at {}",
        report.iface, report.operation, report.address
    );
    match &report.answer {
        OperationAnswer::Value { reply } => {
            format!("{head} → value ({})", kit::plural(reply.size, "byte"))
        }
        OperationAnswer::Refused { envelope } => format!(
            "{head} → refused: {}{}",
            envelope.code,
            envelope
                .cause
                .as_ref()
                .map(|c| format!(" ({c})"))
                .unwrap_or_default()
        ),
        OperationAnswer::Malformed { malformed } => {
            format!("{head} → a malformed envelope ({})", malformed.encoding)
        }
        OperationAnswer::Silent { .. } => format!(
            "{head} → silent within {}s — a non-verdict",
            report.timeout_s
        ),
        OperationAnswer::Replies { replies } => format!(
            "{head} → {} from {}{}",
            kit::plural(
                replies.repliers.iter().map(|r| r.replies.len()).sum(),
                "reply"
            ),
            kit::plural(replies.repliers.len(), "replier"),
            if replies.refusals.is_empty() {
                String::new()
            } else {
                format!(", {}", kit::plural(replies.refusals.len(), "refusal"))
            }
        ),
    }
}

/// A rendered payload as text: the declared type's decode, or what it is
/// instead — opaque, undecodable, or structural with the rung it stopped at.
/// `lead` is a chip drawn before the type's, on its row: what the reply was
/// ("value"), when the caller says it.
fn rendering<'a>(
    p: &PayloadRendering,
    lead: Option<Element<'a, Message>>,
    sp: Spacing,
) -> Element<'a, Message> {
    let (chip, body): (Element<'a, Message>, Option<String>) = match &p.rendered {
        Rendered::Value { declared, value } => (
            kit::data_chip(declared.clone()),
            Some(serde_json::to_string_pretty(value).unwrap_or_default()),
        ),
        Rendered::Opaque { media_type } => (
            kit::data_chip(media_type.clone()),
            Some(format!(
                "{} — a raw type: not this tool's to read",
                kit::plural(p.size, "byte")
            )),
        ),
        Rendered::Undecodable { declared, reason } => (
            kit::status_chip(Tone::Caution, format!("undecodable as {declared}")),
            Some(reason.clone()),
        ),
        Rendered::Structural { text, .. } => (
            kit::status_chip(Tone::Neutral, "structural"),
            Some(text.clone()),
        ),
    };
    let chips = match lead {
        Some(lead) => row![lead, chip]
            .spacing(sp.sm)
            .align_y(iced::Alignment::Center),
        None => row![chip],
    };
    let mut col = column![chips].spacing(sp.xs);
    if let Some(body) = body {
        col = col.push(kit::inset(kit::caption(body).font(face::MONO)).width(Length::Fill));
    }
    col.into()
}

fn envelope_card<'a>(env: &EnvelopeView, sp: Spacing) -> Element<'a, Message> {
    let mut col = column![
        row![
            kit::status_chip(Tone::Negative, format!("refused: {}", env.code)),
            kit::caption(env.message.clone()),
        ]
        .spacing(sp.sm)
        .align_y(iced::Alignment::Center)
    ]
    .spacing(sp.xs);
    if let Some(cause) = &env.cause {
        col = col.push(kit::muted(format!("cause: {cause}")));
    }
    if let Some(detail) = &env.detail {
        col = col.push(kit::muted(format!(
            "detail: {}",
            serde_json::to_string(detail).unwrap_or_default()
        )));
    }
    kit::card(col)
}

fn replies_view<'a>(r: &RepliesView, timeout_s: f64, sp: Spacing) -> Element<'a, Message> {
    let mut col = Column::new().spacing(sp.sm);
    if r.repliers.is_empty() && r.refusals.is_empty() && r.malformed.is_empty() {
        col = col.push(kit::empty(
            kit::EmptyKind::Silent,
            "No replies",
            format!(
                "no value and no envelope within {timeout_s}s — a non-verdict, not proof of \
                 absence (O5)"
            ),
        ));
    }
    for replier in &r.repliers {
        let mut card = column![
            row![
                kit::caption(replier.address.clone()).font(face::MONO),
                kit::status_chip(Tone::Positive, kit::plural(replier.replies.len(), "value")),
            ]
            .spacing(sp.sm)
            .align_y(iced::Alignment::Center)
        ]
        .spacing(sp.xs);
        if replier.possibly_partial == Some(true) {
            card = card.push(kit::status_chip(
                Tone::Caution,
                "possibly partial — no single summary closed it (O6)",
            ));
        }
        for reply in replier.replies.iter().chain(&replier.summaries) {
            card = card.push(rendering(reply, None, sp));
        }
        col = col.push(kit::card(card));
    }
    for env in &r.refusals {
        col = col.push(envelope_card(env, sp));
    }
    if !r.refusals.is_empty() {
        col = col.push(kit::muted(
            "refusals are unattributed: an error reply carries no key (spec §5.1)",
        ));
    }
    for m in &r.malformed {
        col = col.push(kit::status_chip(
            Tone::Caution,
            format!("malformed envelope ({}): {}", m.encoding, m.error),
        ));
    }
    if r.discarded > 0 {
        col = col.push(kit::muted(format!(
            "{} discarded: on a key not concrete, or not this operation's (R6)",
            kit::plural(r.discarded as usize, "reply")
        )));
    }
    if !r.presence.unheard.is_empty() {
        col = col.push(kit::callout(
            Tone::Info,
            kit::caption(format!(
                "holding the interface's token, and no value: {} — each refused or was \
                 silent, which no caller can tell apart",
                r.presence.unheard.join(", ")
            )),
        ));
    }
    if !r.presence.complete {
        col = col.push(kit::muted(
            "the presence read was possibly incomplete: a holder it missed is not listed",
        ));
    }
    col.into()
}

/// The call's answer — a value, an envelope, a malformed envelope, silence,
/// or a fan-out's replies — each its own rendering (O5).
fn outcome_view<'a>(
    outcome: &'a Result<OperationReport, crate::services::ServiceError>,
    sp: Spacing,
) -> Element<'a, Message> {
    let report = match outcome {
        Err(e) => return kit::error(e.to_string()),
        Ok(r) => r,
    };
    let mut col = Column::new().spacing(sp.sm);
    col = col.push(kit::mono(format!("→ {}", report.selectors.join(" + "))));
    col = col.push(kit::muted(format!(
        "{} @ {} · {} · waited up to {}s",
        report.iface,
        report
            .fingerprint
            .strip_prefix("sha256:")
            .map_or(report.fingerprint.as_str(), |h| &h[..h.len().min(16)]),
        match report.mode {
            CallMode::Concrete => "one address",
            CallMode::Fanout => "a fan-out",
        },
        report.timeout_s
    )));
    col = col.push(match &report.answer {
        OperationAnswer::Value { reply } => kit::card(rendering(
            reply,
            Some(kit::status_chip(Tone::Positive, "value")),
            sp,
        )),
        OperationAnswer::Refused { envelope } => envelope_card(envelope, sp),
        OperationAnswer::Malformed { malformed } => kit::card(
            column![
                kit::status_chip(Tone::Caution, "malformed envelope — a broken server"),
                kit::muted(format!("{}: {}", malformed.encoding, malformed.error)),
            ]
            .spacing(sp.xs),
        ),
        // Silence is not an error (O5): no failure chip, no danger edge —
        // a non-verdict, with what presence said of the service.
        OperationAnswer::Silent { silence } => kit::empty(
            kit::EmptyKind::Silent,
            "No answer",
            format!(
                "no value and no envelope within {}s, after {} — a non-verdict, never an \
                 error (O5). Presence: {}.{}",
                report.timeout_s,
                kit::plural(silence.attempts as usize, "attempt"),
                attribution_sentence(silence.presence),
                silence
                    .transport
                    .as_ref()
                    .map(|t| format!(" Transport: {t}."))
                    .unwrap_or_default()
            ),
        ),
        OperationAnswer::Replies { replies } => replies_view(replies, report.timeout_s, sp),
    });
    col.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_is_bounded_and_counts_what_it_dropped() {
        let mut form = SendForm::default();
        for i in 0..(LOG_LINES + 5) {
            form.log(true, format!("send {i}"));
        }
        assert_eq!(form.log.len(), LOG_LINES);
        assert_eq!(form.dropped, 5);
        // Newest first: the last send is the head.
        assert_eq!(form.log[0].text, format!("send {}", LOG_LINES + 4));
    }

    #[test]
    fn the_interval_floors_a_typo_instead_of_spinning() {
        let mut form = SendForm::default();
        assert_eq!(form.interval_secs(), 1.0);
        form.interval = "0".into();
        assert_eq!(form.interval_secs(), 0.1);
        form.interval = "not a number".into();
        assert_eq!(form.interval_secs(), 1.0);
        form.interval = "2.5".into();
        assert_eq!(form.interval_secs(), 2.5);
    }

    /// The body is text as typed, or hex read as bytes — and hex that is not
    /// hex is refused with its reason, before anything moves.
    #[test]
    fn a_body_is_text_or_hex_bytes() {
        let mut form = SendForm {
            body: "hi".into(),
            ..SendForm::default()
        };
        assert_eq!(form.body_bytes().unwrap(), b"hi");
        form.hex = true;
        form.body = "7b 22 76 22\n3a 31 7d".into();
        assert_eq!(form.body_bytes().unwrap(), br#"{"v":1}"#);
        form.body = "7b2".into();
        assert!(form.body_bytes().unwrap_err().contains("3 digits"));
        form.body = "zz".into();
        assert!(form.body_bytes().unwrap_err().contains("\"zz\""));
    }

    /// The axis pickers speak the engine's token: each value's word is the
    /// one `qos_axes` writes, so the picker, the log and a `.zrec` row agree.
    #[test]
    fn the_axis_pickers_spell_the_qos_axes_token() {
        let words: Vec<String> = PRIORITIES.iter().map(ToString::to_string).collect();
        assert_eq!(
            words,
            [
                "real_time",
                "interactive_high",
                "interactive_low",
                "data_high",
                "data",
                "data_low",
                "background"
            ]
        );
        let c: Vec<String> = CONGESTIONS.iter().map(ToString::to_string).collect();
        assert_eq!(c, ["drop", "block"]);
        let r: Vec<String> = RELIABILITIES.iter().map(ToString::to_string).collect();
        assert_eq!(r, ["best_effort", "reliable"]);
    }

    /// P3's words, as `zenctl pub` says them: the owner and the rule, under
    /// any namespace — and a foreign key, a v1 key or a bundle's key passes.
    #[test]
    fn an_owned_key_is_refused_by_name() {
        let why = owned_refusal("spray/zk2/robot1/nav/nav.v2/stream/position").unwrap();
        assert!(why.contains("robot1/nav owns"), "{why}");
        assert!(why.contains("P3, spec §6"), "{why}");
        for foreign in [
            "demo/example/foo",
            "v1/h-3fa9c2d41b7e/state/sysinfo/health",
            "zk2/robot1",
        ] {
            assert!(owned_refusal(foreign).is_none(), "{foreign}");
        }
    }
}
