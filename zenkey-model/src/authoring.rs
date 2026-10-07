//! The authoring format, draft 1 (`examples/zk2/README.md`, r3.3): the
//! TOML shape a contract file has, before any validation or defaulting.
//!
//! `spec/contract.schema.json` is generated from these types (schemars) and
//! committed. A test fails when the two drift.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One contract file: one interface major.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContractFile {
    pub interface: InterfaceSection,
    #[serde(default)]
    pub defaults: Option<DefaultsSection>,
    #[serde(default)]
    pub schemas: SchemasSection,
    #[serde(default)]
    pub resources: BTreeMap<String, ResourceSpec>,
    #[serde(default)]
    pub requires: BTreeMap<String, Requirement>,
}

/// `[interface]`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InterfaceSection {
    /// `[a-z][a-z0-9_]*` segments joined by `.`; never ending in `.v<int>`.
    pub name: String,
    pub major: u32,
    /// Informative; CI keeps it monotonic; not in the fingerprint.
    #[serde(default)]
    pub minor: Option<u32>,
    /// Documentation; not in the fingerprint.
    #[serde(default)]
    pub summary: Option<String>,
    /// Profiles whose annotations the contract uses (`freshness.v1`, …).
    #[serde(default)]
    pub uses: Vec<String>,
}

/// `[schemas]`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchemasSection {
    /// `.proto` files, relative to the contract's directory. proto2/proto3.
    #[serde(default)]
    pub protobuf: Vec<String>,
    /// Include roots for protobuf imports, relative to the contract's
    /// directory. Default: `["proto"]` when that directory exists, else `["."]`.
    #[serde(default)]
    pub proto_include: Option<Vec<String>>,
    /// JSON Schema 2020-12 files (the zk2 subset), definitions under `$defs`.
    #[serde(default)]
    pub jsonschema: Vec<String>,
}

/// The fields `[defaults]` may set (r3.3 D2).
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DefaultsBlock {
    #[serde(default)]
    pub annotations: BTreeMap<String, Value>,
    pub encoding: Option<Encoding>,
    pub attachment_encoding: Option<Encoding>,
    pub reliability: Option<Reliability>,
    pub congestion: Option<Congestion>,
    pub priority: Option<Priority>,
    pub express: Option<bool>,
    pub history: Option<History>,
    pub idempotent: Option<bool>,
    pub fanout: Option<Fanout>,
    pub serving: Option<Serving>,
    pub replies: Option<Replies>,
    pub timeout_ms: Option<u64>,
}

/// `[defaults]` and `[defaults.<kind>]`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DefaultsSection {
    #[serde(default)]
    pub annotations: BTreeMap<String, Value>,
    pub encoding: Option<Encoding>,
    pub attachment_encoding: Option<Encoding>,
    pub reliability: Option<Reliability>,
    pub congestion: Option<Congestion>,
    pub priority: Option<Priority>,
    pub express: Option<bool>,
    pub history: Option<History>,
    pub idempotent: Option<bool>,
    pub fanout: Option<Fanout>,
    pub serving: Option<Serving>,
    pub replies: Option<Replies>,
    pub timeout_ms: Option<u64>,
    pub stream: Option<DefaultsBlock>,
    pub state: Option<DefaultsBlock>,
    pub event: Option<DefaultsBlock>,
    pub operation: Option<DefaultsBlock>,
}

impl DefaultsSection {
    /// The base block (the fields set directly under `[defaults]`).
    #[must_use]
    pub fn base(&self) -> DefaultsBlock {
        DefaultsBlock {
            annotations: self.annotations.clone(),
            encoding: self.encoding,
            attachment_encoding: self.attachment_encoding,
            reliability: self.reliability,
            congestion: self.congestion,
            priority: self.priority,
            express: self.express,
            history: self.history.clone(),
            idempotent: self.idempotent,
            fanout: self.fanout,
            serving: self.serving,
            replies: self.replies,
            timeout_ms: self.timeout_ms,
        }
    }
    #[must_use]
    pub fn for_kind(&self, kind: Kind) -> Option<&DefaultsBlock> {
        match kind {
            Kind::Stream => self.stream.as_ref(),
            Kind::State => self.state.as_ref(),
            Kind::Event => self.event.as_ref(),
            Kind::Operation => self.operation.as_ref(),
        }
    }
}

/// `[resources."<template>"]`: every field any kind may carry. Which ones
/// a kind accepts is checked by the lints (E014).
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceSpec {
    pub kind: Kind,
    #[serde(default)]
    pub doc: Option<String>,
    #[serde(default)]
    pub params: BTreeMap<String, ParamType>,
    #[serde(default)]
    pub cardinality: Option<u64>,
    #[serde(default)]
    pub epoch: Option<String>,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub gate: Option<Gate>,
    #[serde(default)]
    pub deprecated: Option<Deprecated>,
    #[serde(default)]
    pub annotations: BTreeMap<String, Value>,
    // stream, state, event
    #[serde(default, rename = "type")]
    pub type_: Option<TypeRef>,
    #[serde(default)]
    pub attachment: Option<TypeRef>,
    #[serde(default)]
    pub encoding: Option<Encoding>,
    #[serde(default)]
    pub attachment_encoding: Option<Encoding>,
    #[serde(default)]
    pub explicit: Option<bool>,
    #[serde(default)]
    pub reliability: Option<Reliability>,
    #[serde(default)]
    pub congestion: Option<Congestion>,
    #[serde(default)]
    pub priority: Option<Priority>,
    #[serde(default)]
    pub express: Option<bool>,
    #[serde(default)]
    pub history: Option<History>,
    #[serde(default)]
    pub rate: Option<String>,
    #[serde(default)]
    pub retention: Option<String>,
    // operation
    #[serde(default)]
    pub request: Option<TypeRef>,
    #[serde(default)]
    pub response: Option<TypeRef>,
    #[serde(default)]
    pub error: Option<TypeRef>,
    #[serde(default)]
    pub idempotent: Option<bool>,
    #[serde(default)]
    pub fanout: Option<Fanout>,
    #[serde(default)]
    pub serving: Option<Serving>,
    #[serde(default)]
    pub replies: Option<Replies>,
    #[serde(default)]
    pub summary: Option<TypeRef>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// `[requires.<role>]`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Requirement {
    /// The required interface, `<name>.v<major>`.
    pub interface: String,
    /// What is consumed; default: everything.
    #[serde(default)]
    pub resources: Option<Vec<String>>,
    #[serde(default)]
    pub cardinality: Option<RequireCardinality>,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub doc: Option<String>,
    #[serde(default)]
    pub annotations: BTreeMap<String, Value>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Stream,
    State,
    Event,
    Operation,
}

impl Kind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stream => "stream",
            Self::State => "state",
            Self::Event => "event",
            Self::Operation => "operation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ParamType {
    String,
    Uint,
    Path,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    Json,
    Cbor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Reliability {
    BestEffort,
    Reliable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Congestion {
    Drop,
    Block,
}

/// Zenoh's seven priorities, by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    RealTime,
    InteractiveHigh,
    InteractiveLow,
    DataHigh,
    Data,
    DataLow,
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Fanout {
    Forbidden,
    Allowed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Serving {
    Exclusive,
    Replicated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Replies {
    One,
    Many,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RequireCardinality {
    One,
    Many,
}

/// A type reference: `"pkg.Message"`, `"json:Name"`, `"json:stem#Name"`,
/// or `{ raw = "<media type>", media_param = "<param>" }`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum TypeRef {
    Named(String),
    Raw(RawType),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RawType {
    pub raw: String,
    #[serde(default)]
    pub media_param: Option<String>,
}

/// `gate = "capability:rssi"` or a list of them (AND).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum Gate {
    One(String),
    All(Vec<String>),
}

impl Gate {
    #[must_use]
    pub fn items(&self) -> Vec<&str> {
        match self {
            Self::One(s) => vec![s.as_str()],
            Self::All(v) => v.iter().map(String::as_str).collect(),
        }
    }
}

/// `history = true` or `{ depth = <n>, miss_detection_ms = <ms> }`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum History {
    Flag(bool),
    Params(HistoryParams),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HistoryParams {
    pub depth: u32,
    #[serde(default)]
    pub miss_detection_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Deprecated {
    pub since: u32,
    #[serde(default)]
    pub replaced_by: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}
