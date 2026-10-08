//! The compatibility classifier (spec `core.md` §9.8, #618).
//!
//! Revisions inside a major are checked FULL_TRANSITIVE: a candidate
//! against every earlier revision, in both directions. A change is
//! [`Class::Compatible`], [`Class::Review`] (a human accepts it) or
//! [`Class::Breaking`] (CI refuses it); the candidate's class is the worst.
//!
//! The classifier works on [`Revision`]s: a canonical contract plus its
//! schema artifacts. A revision comes from a loaded contract or from a
//! published bundle, so the `.history` is checked without re-reading
//! sources.
//!
//! - **Contract metadata** follows §9.8's table.
//! - **Protobuf payloads** use WIRE semantics with renumber detection
//!   (decided 2026-10-08): a type or cardinality change breaks, a rename is
//!   review, a field added is compatible, a field deleted without reserving
//!   its number is compatible with the warning `field_deleted_unreserved`.
//!   A proto2 `required` label added, removed or toggled breaks: a reader
//!   refuses a message that lacks it.
//! - **JSON Schema payloads** use the zk2 subset with tolerant readers;
//!   anything inside `oneOf`, `anyOf` or `prefixItems` that is not decided
//!   is review.
//! - **Retention** ([`same_revision`]): two revisions are the same when
//!   their canonical forms agree once schema ids are replaced by artifact
//!   names, and their artifacts agree once protobuf descriptor sets are
//!   normalized (source info and default `json_name` dropped).

use std::collections::{BTreeMap, BTreeSet};

use base64::Engine as _;
use prost::Message as _;
use prost_reflect::{
    Cardinality, DescriptorPool, EnumDescriptor, FieldDescriptor, Kind, MessageDescriptor,
};
use serde_json::{Map, Value};

use crate::bundle::Bundle;
use crate::canonical::canonical;
use crate::contract::Contract;
use crate::schema::{ArtifactData, jcs};

/// How a change is judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    Compatible,
    Review,
    Breaking,
}

impl Class {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compatible => "compatible",
            Self::Review => "review",
            Self::Breaking => "breaking",
        }
    }
}

/// One judged change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub class: Class,
    /// A stable rule name (`field_renamed`, `resource_removed`, …).
    pub rule: &'static str,
    /// Where: `resources."<template>"`, a field path, a role.
    pub at: String,
    pub detail: String,
}

/// The result of comparing a candidate with earlier revisions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verdict {
    /// Every review and breaking change.
    pub findings: Vec<Finding>,
    /// Warnings that do not change the class (`field_deleted_unreserved`).
    pub warnings: Vec<Finding>,
}

impl Verdict {
    /// The worst class found; compatible when there is none.
    #[must_use]
    pub fn class(&self) -> Class {
        self.findings
            .iter()
            .map(|f| f.class)
            .max()
            .unwrap_or(Class::Compatible)
    }
    /// The warning rules, sorted and deduplicated.
    #[must_use]
    pub fn warning_rules(&self) -> Vec<&'static str> {
        let s: BTreeSet<&'static str> = self.warnings.iter().map(|w| w.rule).collect();
        s.into_iter().collect()
    }
    fn push(&mut self, class: Class, rule: &'static str, at: &str, detail: impl Into<String>) {
        if class == Class::Compatible {
            return;
        }
        self.findings.push(Finding {
            class,
            rule,
            at: at.to_owned(),
            detail: detail.into(),
        });
    }
    fn warn(&mut self, rule: &'static str, at: &str, detail: impl Into<String>) {
        self.warnings.push(Finding {
            class: Class::Compatible,
            rule,
            at: at.to_owned(),
            detail: detail.into(),
        });
    }
    fn extend(&mut self, other: Verdict) {
        self.findings.extend(other.findings);
        self.warnings.extend(other.warnings);
    }
}

/// A schema artifact of a revision.
#[derive(Debug, Clone, PartialEq)]
pub enum Schema {
    /// An encoded `FileDescriptorSet`.
    Protobuf(Vec<u8>),
    /// A JSON Schema document.
    Json(Value),
}

/// A revision: its canonical contract and its schema artifacts by id.
#[derive(Debug, Clone, PartialEq)]
pub struct Revision {
    pub canonical: Value,
    pub schemas: BTreeMap<String, Schema>,
}

impl Revision {
    /// Of a loaded contract.
    #[must_use]
    pub fn of(c: &Contract) -> Self {
        let schemas = c
            .artifacts
            .iter()
            .map(|(id, a)| {
                let s = match &a.data {
                    ArtifactData::Protobuf(b) => Schema::Protobuf(b.clone()),
                    ArtifactData::Json(v) => Schema::Json(v.clone()),
                };
                (id.clone(), s)
            })
            .collect();
        Self {
            canonical: canonical(c),
            schemas,
        }
    }

    /// Of a verified bundle.
    #[must_use]
    pub fn of_bundle(b: &Bundle) -> Self {
        let schemas = b
            .schemas
            .iter()
            .filter_map(|(id, e)| {
                let data = e.get("data")?;
                let s = match e.get("kind")?.as_str()? {
                    "protobuf" => Schema::Protobuf(
                        base64::engine::general_purpose::STANDARD
                            .decode(data.as_str()?)
                            .ok()?,
                    ),
                    "jsonschema" => Schema::Json(data.clone()),
                    _ => return None,
                };
                Some((id.clone(), s))
            })
            .collect();
        Self {
            canonical: b.contract.clone(),
            schemas,
        }
    }

    fn resources(&self) -> BTreeMap<(String, String), &Value> {
        self.canonical["resources"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| {
                Some((
                    (
                        r["kind"].as_str()?.to_owned(),
                        r["template"].as_str()?.to_owned(),
                    ),
                    r,
                ))
            })
            .collect()
    }

    /// JSON Schema documents by file stem (their artifact name).
    fn json_by_stem(&self) -> BTreeMap<String, &Value> {
        let names: BTreeMap<&str, &str> = self.canonical["schemas"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| Some((s["id"].as_str()?, s["name"].as_str()?)))
            .collect();
        self.schemas
            .iter()
            .filter_map(|(id, s)| match s {
                Schema::Json(v) => Some((names.get(id.as_str())?.to_string(), v)),
                Schema::Protobuf(_) => None,
            })
            .collect()
    }
}

/// Classifies `new` against every revision of `history` (FULL_TRANSITIVE).
#[must_use]
pub fn check_history(history: &[Revision], new: &Revision) -> Verdict {
    let mut v = Verdict::default();
    for old in history {
        v.extend(compare(old, new));
    }
    v
}

/// Classifies the change from `old` to `new`, both directions.
#[must_use]
pub fn compare(old: &Revision, new: &Revision) -> Verdict {
    let mut v = Verdict::default();
    if old.canonical["interface"] != new.canonical["interface"] {
        v.push(
            Class::Breaking,
            "interface_changed",
            "interface",
            "the interface id differs",
        );
        return v;
    }
    if old.canonical["uses"] != new.canonical["uses"] {
        v.push(
            Class::Review,
            "uses_changed",
            "interface.uses",
            "the profiles used changed",
        );
    }
    let (ro, rn) = (old.resources(), new.resources());
    for (key, o) in &ro {
        let at = format!("resources.{:?}", key.1);
        match rn.get(key) {
            None => v.push(
                Class::Breaking,
                "resource_removed",
                &at,
                "removed; a deprecated resource stays until the next major",
            ),
            Some(n) => compare_resource(o, n, old, new, &at, &mut v),
        }
    }
    for (key, n) in &rn {
        if ro.contains_key(key) {
            continue;
        }
        let at = format!("resources.{:?}", key.1);
        if n["optional"] == Value::Bool(true) {
            // An optional resource added: old providers simply do not
            // expose it.
        } else {
            v.push(
                Class::Breaking,
                "required_resource_added",
                &at,
                "a required resource added: old providers lack it",
            );
        }
    }
    compare_requires(
        &old.canonical["requires"],
        &new.canonical["requires"],
        &mut v,
    );
    v
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

#[allow(clippy::too_many_lines)]
fn compare_resource(o: &Value, n: &Value, ro: &Revision, rn: &Revision, at: &str, v: &mut Verdict) {
    // Explicit-ness is in the token.
    match (s(&o["token"]), s(&n["token"])) {
        (a, b) if a == b => {}
        ("stream", "@stream") | ("state", "@state") => {
            v.push(
                Class::Breaking,
                "explicit_set",
                at,
                "leaves ambient selectors",
            );
        }
        ("@stream", "stream") | ("@state", "state") => {
            v.push(
                Class::Review,
                "explicit_cleared",
                at,
                "joins ambient selectors (link budgets)",
            );
        }
        _ => v.push(
            Class::Breaking,
            "token_changed",
            at,
            "the kind token changed",
        ),
    }
    if o["params"] != n["params"] {
        v.push(
            Class::Breaking,
            "params_changed",
            at,
            "a parameter's type changed",
        );
    }
    match (o["optional"].as_bool(), n["optional"].as_bool()) {
        (Some(true), Some(false)) => {
            v.push(
                Class::Breaking,
                "optional_to_required",
                at,
                "old implementers may not expose it",
            );
        }
        (Some(false), Some(true)) => {
            v.push(
                Class::Breaking,
                "required_to_optional",
                at,
                "old consumers rely on it",
            );
        }
        _ => {}
    }
    for (field, rule) in [
        ("cardinality", "cardinality_changed"),
        ("epoch", "epoch_changed"),
        ("gate", "gate_changed"),
        ("annotations", "annotations_changed"),
    ] {
        if o[field] != n[field] {
            v.push(Class::Review, rule, at, format!("`{field}` changed"));
        }
    }
    if o["deprecated"] != n["deprecated"] && !n["deprecated"].is_null() && o["deprecated"].is_null()
    {
        // Deprecating is compatible; anything else about it is review.
    } else if o["deprecated"] != n["deprecated"] {
        v.push(
            Class::Review,
            "deprecated_changed",
            at,
            "`deprecated` changed",
        );
    }
    if s(&o["kind"]) != s(&n["kind"]) {
        v.push(
            Class::Breaking,
            "kind_changed",
            at,
            "the resource kind changed",
        );
        return;
    }
    if o["encoding"] != n["encoding"] {
        v.push(
            Class::Breaking,
            "encoding_changed",
            at,
            "the payload encoding changed",
        );
    }
    if s(&o["kind"]) == "operation" {
        for (field, what) in [("request", Dir::Request), ("response", Dir::Response)] {
            compare_type(
                &o[field],
                &n[field],
                ro,
                rn,
                &format!("{at}.{field}"),
                what,
                v,
            );
        }
        for (field, rule) in [
            ("error", "error_type_changed"),
            ("summary", "summary_type_changed"),
        ] {
            match (o[field].is_null(), n[field].is_null()) {
                (true, true) => {}
                (false, false) => compare_type(
                    &o[field],
                    &n[field],
                    ro,
                    rn,
                    &format!("{at}.{field}"),
                    Dir::Response,
                    v,
                ),
                _ => v.push(
                    Class::Review,
                    rule,
                    at,
                    format!("`{field}` added or removed"),
                ),
            }
        }
        if o["idempotent"] == Value::Bool(true) && n["idempotent"] == Value::Bool(false) {
            v.push(
                Class::Breaking,
                "idempotent_cleared",
                at,
                "callers may retry it",
            );
        } else if o["idempotent"] != n["idempotent"] {
            v.push(
                Class::Review,
                "idempotent_set",
                at,
                "new callers may retry old servers",
            );
        }
        if s(&o["fanout"]) == "allowed" && s(&n["fanout"]) == "forbidden" {
            v.push(
                Class::Breaking,
                "fanout_forbidden",
                at,
                "fan-out calls are refused",
            );
        }
        if s(&o["replies"]) == "one" && s(&n["replies"]) == "many" {
            v.push(
                Class::Breaking,
                "replies_many",
                at,
                "callers' consolidation drops replies",
            );
        }
        for (field, rule) in [
            ("serving", "serving_changed"),
            ("timeout_ms", "timeout_changed"),
            ("priority", "priority_changed"),
        ] {
            if o[field] != n[field] {
                v.push(Class::Review, rule, at, format!("`{field}` changed"));
            }
        }
        return;
    }
    compare_type(
        &o["type"],
        &n["type"],
        ro,
        rn,
        &format!("{at}.type"),
        Dir::Data,
        v,
    );
    match (o["attachment"].is_null(), n["attachment"].is_null()) {
        (true, true) => {}
        (false, false) => compare_type(
            &o["attachment"],
            &n["attachment"],
            ro,
            rn,
            &format!("{at}.attachment"),
            Dir::Data,
            v,
        ),
        _ => v.push(
            Class::Review,
            "attachment_changed",
            at,
            "an attachment added or removed",
        ),
    }
    if o["attachment_encoding"] != n["attachment_encoding"]
        && !o["attachment"].is_null()
        && !n["attachment"].is_null()
    {
        v.push(
            Class::Breaking,
            "encoding_changed",
            at,
            "the attachment encoding changed",
        );
    }
    if s(&o["reliability"]) == "reliable" && s(&n["reliability"]) == "best_effort" {
        v.push(
            Class::Review,
            "reliability_lowered",
            at,
            "reliable → best_effort",
        );
    }
    if o["congestion"] != n["congestion"] {
        v.push(
            Class::Review,
            "congestion_changed",
            at,
            "drop ↔ block (it can stall the producer, or drop)",
        );
    }
    for (field, rule) in [
        ("priority", "priority_changed"),
        ("express", "express_toggled"),
        ("history", "history_changed"),
        ("rate", "rate_changed"),
        ("retention_s", "retention_changed"),
    ] {
        if o[field] != n[field] {
            v.push(Class::Review, rule, at, format!("`{field}` changed"));
        }
    }
}

fn compare_requires(o: &Value, n: &Value, v: &mut Verdict) {
    let empty = Map::new();
    let (o, n) = (
        o.as_object().unwrap_or(&empty),
        n.as_object().unwrap_or(&empty),
    );
    for (role, ro) in o {
        let at = format!("requires.{role}");
        let Some(rn) = n.get(role) else {
            // A role removed: deployments stop binding it.
            continue;
        };
        if ro["interface"] != rn["interface"] || ro["cardinality"] != rn["cardinality"] {
            v.push(
                Class::Breaking,
                "role_changed",
                &at,
                "the role's interface or cardinality changed",
            );
        }
        if ro["optional"] == Value::Bool(true) && rn["optional"] == Value::Bool(false) {
            v.push(
                Class::Breaking,
                "role_required",
                &at,
                "deployments must now bind it",
            );
        }
        if ro["resources"] != rn["resources"] || ro["annotations"] != rn["annotations"] {
            v.push(
                Class::Review,
                "role_resources_changed",
                &at,
                "what the role consumes changed",
            );
        }
    }
    for (role, rn) in n {
        if !o.contains_key(role) && rn["optional"] != Value::Bool(true) {
            v.push(
                Class::Breaking,
                "required_role_added",
                &format!("requires.{role}"),
                "deployments must bind it",
            );
        }
    }
}

/// Which side writes a payload. The classification is the same both ways
/// (FULL), so this only names the place in findings.
#[derive(Debug, Clone, Copy)]
enum Dir {
    Data,
    Request,
    Response,
}

fn compare_type(
    o: &Value,
    n: &Value,
    ro: &Revision,
    rn: &Revision,
    at: &str,
    _dir: Dir,
    v: &mut Verdict,
) {
    match (s(&o["kind"]), s(&n["kind"])) {
        ("raw", "raw") => {
            if o["media_type"] != n["media_type"] {
                v.push(
                    Class::Breaking,
                    "media_type_changed",
                    at,
                    "the raw media type changed",
                );
            } else if o["media_param"] != n["media_param"] {
                v.push(
                    Class::Review,
                    "media_param_changed",
                    at,
                    "`media_param` changed",
                );
            }
        }
        ("protobuf", "protobuf") => {
            let (Some(po), Some(pn)) = (pool(ro, &o["schema"]), pool(rn, &n["schema"])) else {
                v.push(
                    Class::Review,
                    "schema_unreadable",
                    at,
                    "a descriptor set does not decode",
                );
                return;
            };
            match (
                po.get_message_by_name(s(&o["name"])),
                pn.get_message_by_name(s(&n["name"])),
            ) {
                (Some(mo), Some(mn)) => {
                    let mut seen = BTreeSet::new();
                    compare_messages(&mo, &mn, at, &mut seen, v);
                }
                _ => v.push(
                    Class::Review,
                    "schema_unreadable",
                    at,
                    "a message is missing from its descriptor set",
                ),
            }
        }
        ("jsonschema", "jsonschema") => {
            let (Some(Schema::Json(doc_old)), Some(Schema::Json(doc_new))) = (
                ro.schemas.get(s(&o["schema"])),
                rn.schemas.get(s(&n["schema"])),
            ) else {
                v.push(
                    Class::Review,
                    "schema_unreadable",
                    at,
                    "a JSON Schema artifact is missing",
                );
                return;
            };
            let cx = JsonCx {
                old_docs: ro.json_by_stem(),
                new_docs: rn.json_by_stem(),
            };
            let so = doc_old["$defs"][s(&o["name"])].clone();
            let sn = doc_new["$defs"][s(&n["name"])].clone();
            let mut seen = BTreeSet::new();
            cx.compare(&so, doc_old, &sn, doc_new, at, &mut seen, v);
        }
        _ => v.push(
            Class::Breaking,
            "type_kind_changed",
            at,
            "the schema kind changed",
        ),
    }
}

fn pool(r: &Revision, id: &Value) -> Option<DescriptorPool> {
    match r.schemas.get(id.as_str()?)? {
        Schema::Protobuf(b) => DescriptorPool::decode(b.as_slice()).ok(),
        Schema::Json(_) => None,
    }
}

fn reserved(m: &MessageDescriptor, number: u32) -> bool {
    m.reserved_ranges().any(|r| r.contains(&number))
}

fn compare_messages(
    o: &MessageDescriptor,
    n: &MessageDescriptor,
    at: &str,
    seen: &mut BTreeSet<(String, String)>,
    v: &mut Verdict,
) {
    if !seen.insert((o.full_name().to_owned(), n.full_name().to_owned())) {
        return;
    }
    for fo in o.fields() {
        let fat = format!("{at}.{}", fo.name());
        match n.get_field(fo.number()) {
            Some(fnew) => {
                if fo.name() != fnew.name() {
                    v.push(
                        Class::Review,
                        "field_renamed",
                        &fat,
                        format!("now {:?}", fnew.name()),
                    );
                } else if fo.json_name() != fnew.json_name() {
                    v.push(
                        Class::Review,
                        "json_name_changed",
                        &fat,
                        format!("now {:?}", fnew.json_name()),
                    );
                }
                if fo.is_list() != fnew.is_list() || fo.is_map() != fnew.is_map() {
                    v.push(
                        Class::Breaking,
                        "cardinality_changed",
                        &fat,
                        "repeated, map or singular changed",
                    );
                    continue;
                }
                if is_required(&fo) != is_required(&fnew) {
                    v.push(
                        Class::Breaking,
                        "required_label_changed",
                        &fat,
                        "proto2 `required` toggled",
                    );
                }
                let oneof = |f: &prost_reflect::FieldDescriptor| {
                    f.containing_oneof()
                        .filter(|o| !o.is_synthetic())
                        .map(|o| o.name().to_owned())
                };
                if oneof(&fo) != oneof(&fnew) {
                    v.push(
                        Class::Breaking,
                        "oneof_changed",
                        &fat,
                        "moved into or out of a oneof",
                    );
                } else if fo.supports_presence() != fnew.supports_presence() && !fo.is_list() {
                    v.push(
                        Class::Review,
                        "presence_changed",
                        &fat,
                        "explicit presence toggled",
                    );
                }
                compare_kinds(&fo.kind(), &fnew.kind(), &fat, seen, v);
            }
            None => {
                if is_required(&fo) && n.get_field_by_name(fo.name()).is_none() {
                    v.push(
                        Class::Breaking,
                        "required_field_removed",
                        &fat,
                        "a proto2 `required` field deleted: old readers refuse what new writers send",
                    );
                }
                if let Some(moved) = n.get_field_by_name(fo.name()) {
                    v.push(
                        Class::Breaking,
                        "renumbered",
                        &fat,
                        format!(
                            "{} → {}: the data is dropped both ways",
                            fo.number(),
                            moved.number()
                        ),
                    );
                } else if !reserved(n, fo.number()) {
                    v.warn(
                        "field_deleted_unreserved",
                        &fat,
                        format!("field {} deleted without reserving its number", fo.number()),
                    );
                }
            }
        }
    }
    for fnew in n.fields() {
        if o.get_field(fnew.number()).is_some() || o.get_field_by_name(fnew.name()).is_some() {
            continue;
        }
        if is_required(&fnew) {
            v.push(
                Class::Breaking,
                "required_field_added",
                &format!("{at}.{}", fnew.name()),
                "a proto2 `required` field added: new readers refuse what old writers send",
            );
        }
        if reserved(o, fnew.number()) {
            v.push(
                Class::Breaking,
                "reserved_reused",
                &format!("{at}.{}", fnew.name()),
                format!("number {} was reserved", fnew.number()),
            );
        }
    }
}

fn is_required(f: &FieldDescriptor) -> bool {
    f.cardinality() == Cardinality::Required
}

fn compare_kinds(
    o: &Kind,
    n: &Kind,
    at: &str,
    seen: &mut BTreeSet<(String, String)>,
    v: &mut Verdict,
) {
    match (o, n) {
        (Kind::Message(mo), Kind::Message(mn)) => compare_messages(mo, mn, at, seen, v),
        (Kind::Enum(eo), Kind::Enum(en)) => compare_enums(eo, en, at, v),
        (a, b) if std::mem::discriminant(a) == std::mem::discriminant(b) => {}
        _ => v.push(
            Class::Breaking,
            "type_changed",
            at,
            format!("{o:?} → {n:?}"),
        ),
    }
}

fn compare_enums(o: &EnumDescriptor, n: &EnumDescriptor, at: &str, v: &mut Verdict) {
    for vo in o.values() {
        match n.get_value(vo.number()) {
            None => v.push(
                Class::Review,
                "enum_value_removed",
                at,
                format!("{} removed", vo.name()),
            ),
            Some(vn) if vn.name() != vo.name() => {
                v.push(
                    Class::Review,
                    "enum_value_renamed",
                    at,
                    format!("{} → {}", vo.name(), vn.name()),
                );
            }
            Some(_) => {}
        }
    }
    let closed = n.parent_file().syntax() == prost_reflect::Syntax::Proto2;
    if closed && n.values().any(|vn| o.get_value(vn.number()).is_none()) {
        v.push(
            Class::Review,
            "closed_enum_value_added",
            at,
            "a proto2 (closed) enum gained a value: old readers fall back to the default",
        );
    }
}

/// JSON Schema comparison context: every listed document of each side, by
/// stem, to follow `$ref`s across files.
struct JsonCx<'a> {
    old_docs: BTreeMap<String, &'a Value>,
    new_docs: BTreeMap<String, &'a Value>,
}

const ANNOTATIONS: &[&str] = &[
    "$schema",
    "$id",
    "$comment",
    "title",
    "description",
    "default",
    "examples",
    "format",
    "deprecated",
    "readOnly",
    "writeOnly",
];
const BOUNDS: &[&str] = &[
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "minLength",
    "maxLength",
    "minItems",
    "maxItems",
];

/// A schema without its annotations, at every schema position
/// ([`crate::schema::visit_schemas`]'s): what a change inside an undecided
/// keyword is compared on. Property names and data (`enum`, `const`) are
/// kept as written, so a property named `title` is not an annotation. An
/// array is a list of schemas (the value of `oneOf`, `anyOf`,
/// `prefixItems`).
fn strip(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(k, _)| !ANNOTATIONS.contains(&k.as_str()))
                .map(|(k, x)| {
                    let x = match (k.as_str(), x) {
                        ("properties" | "$defs", Value::Object(subs)) => {
                            Value::Object(subs.iter().map(|(n, s)| (n.clone(), strip(s))).collect())
                        }
                        (
                            "prefixItems" | "oneOf" | "anyOf" | "items" | "additionalProperties",
                            sub,
                        ) => strip(sub),
                        (_, x) => x.clone(),
                    };
                    (k.clone(), x)
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(strip).collect()),
        x => x.clone(),
    }
}

fn types(s: &Value) -> BTreeSet<String> {
    match &s["type"] {
        Value::String(t) => [t.clone()].into(),
        Value::Array(a) => a
            .iter()
            .filter_map(|t| t.as_str().map(str::to_owned))
            .collect(),
        _ => BTreeSet::new(),
    }
}

/// An `enum` as the set of its values: order carries no meaning.
fn values(e: Option<&Value>) -> Option<BTreeSet<Vec<u8>>> {
    e.map(|e| e.as_array().into_iter().flatten().map(jcs).collect())
}

fn required(s: &Value) -> BTreeSet<String> {
    s["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r.as_str().map(str::to_owned))
        .collect()
}

impl<'a> JsonCx<'a> {
    /// Follows `$ref` to its target, and the document the target lives in.
    /// A `$ref`'s file part names an artifact by the stem of its last path
    /// segment (spec §9.4): a bundle keeps no paths. Keywords beside a
    /// `$ref` (other than `$defs`) are added to its target, the outermost
    /// taking the place of the target's own, so a change beside a `$ref` is
    /// compared like any other.
    fn deref(&self, s: &Value, doc: &'a Value, old: bool) -> (Value, &'a Value) {
        let mut cur = s.clone();
        let mut doc = doc;
        let mut beside: Vec<Map<String, Value>> = Vec::new();
        for _ in 0..32 {
            let Some(r) = cur.get("$ref").and_then(Value::as_str).map(str::to_owned) else {
                break;
            };
            if let Value::Object(m) = &cur {
                let sib: Map<String, Value> = m
                    .iter()
                    .filter(|(k, _)| !matches!(k.as_str(), "$ref" | "$defs"))
                    .map(|(k, x)| (k.clone(), x.clone()))
                    .collect();
                if !sib.is_empty() {
                    beside.push(sib);
                }
            }
            let (file, ptr) = r.split_once('#').unwrap_or((r.as_str(), ""));
            if !file.is_empty() {
                let base = file.rsplit('/').next().unwrap_or(file);
                let stem = base.strip_suffix(".json").unwrap_or(base);
                let docs = if old { &self.old_docs } else { &self.new_docs };
                if let Some(d) = docs.get(stem) {
                    doc = d;
                }
            }
            cur = doc.pointer(ptr).cloned().unwrap_or(Value::Null);
        }
        if !beside.is_empty() && cur != Value::Bool(false) {
            let mut merged = match cur {
                Value::Object(m) => m,
                _ => Map::new(),
            };
            for sib in beside.into_iter().rev() {
                merged.extend(sib);
            }
            cur = Value::Object(merged);
        }
        (cur, doc)
    }

    #[allow(clippy::too_many_arguments)]
    fn compare(
        &self,
        so: &Value,
        doc_o: &'a Value,
        sn: &Value,
        doc_n: &'a Value,
        at: &str,
        seen: &mut BTreeSet<(String, String)>,
        v: &mut Verdict,
    ) {
        let key = (
            String::from_utf8(jcs(so)).unwrap_or_default(),
            String::from_utf8(jcs(sn)).unwrap_or_default(),
        );
        if !seen.insert(key) {
            return;
        }
        let (o, doc_o) = self.deref(so, doc_o, true);
        let (n, doc_n) = self.deref(sn, doc_n, false);
        let (o, n) = (&o, &n);
        // A boolean schema (`true` accepts anything, `false` nothing) is not
        // compared keyword by keyword: any change to or from one is review.
        if o.is_boolean() || n.is_boolean() {
            if o != n {
                v.push(
                    Class::Review,
                    "boolean_schema_changed",
                    at,
                    "a boolean schema changed",
                );
            }
            return;
        }
        if types(o) != types(n) {
            v.push(
                Class::Breaking,
                "type_changed",
                at,
                format!("{:?} → {:?}", types(o), types(n)),
            );
            return;
        }
        for k in BOUNDS {
            if o.get(*k) != n.get(*k) {
                v.push(
                    Class::Breaking,
                    "bound_changed",
                    at,
                    format!("`{k}` changed"),
                );
            }
        }
        if values(o.get("enum")) != values(n.get("enum")) {
            v.push(
                Class::Breaking,
                "enum_changed",
                at,
                "a value added or removed",
            );
        }
        if o.get("const") != n.get("const") {
            v.push(Class::Breaking, "const_changed", at, "`const` changed");
        }
        // Undecided keywords: review on any change, breaking on a oneOf
        // branch added (spike S7).
        for k in ["oneOf", "anyOf", "prefixItems"] {
            let (a, b) = (o.get(k).map(strip), n.get(k).map(strip));
            if a == b {
                continue;
            }
            let grown = k == "oneOf"
                && matches!((&a, &b), (Some(Value::Array(x)), Some(Value::Array(y))) if y.len() > x.len());
            if grown {
                v.push(
                    Class::Breaking,
                    "oneof_branch_added",
                    at,
                    "a `oneOf` branch added",
                );
            } else {
                v.push(
                    Class::Review,
                    "undecided_changed",
                    at,
                    format!("`{k}` changed"),
                );
            }
        }
        // Objects: tolerant readers, writers send only what they declare.
        let empty = Map::new();
        let po = o
            .get("properties")
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        let pn = n
            .get("properties")
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        let (ro, rn) = (required(o), required(n));
        for (name, so) in po {
            let pat = format!("{at}.{name}");
            match pn.get(name) {
                Some(sn) => {
                    if ro.contains(name) != rn.contains(name) {
                        v.push(
                            Class::Breaking,
                            "required_changed",
                            &pat,
                            "optional ↔ required",
                        );
                    }
                    self.compare(so, doc_o, sn, doc_n, &pat, seen, v);
                }
                None if ro.contains(name) => {
                    v.push(
                        Class::Breaking,
                        "required_removed",
                        &pat,
                        "a required property removed",
                    );
                }
                None => {}
            }
        }
        for name in pn.keys() {
            if !po.contains_key(name) && rn.contains(name) {
                v.push(
                    Class::Breaking,
                    "required_added",
                    &format!("{at}.{name}"),
                    "a required property added",
                );
            }
        }
        // A required name with no property on either side still binds:
        // the member must be present, whatever its value.
        for name in ro.symmetric_difference(&rn) {
            if po.contains_key(name) || pn.contains_key(name) {
                continue;
            }
            let rule = if rn.contains(name) {
                "required_added"
            } else {
                "required_removed"
            };
            v.push(
                Class::Breaking,
                rule,
                &format!("{at}.{name}"),
                "a required name without a property",
            );
        }
        // Members: a schema on both sides is compared; a boolean or absent
        // one on both is open or closed, which tolerant readers ignore; a
        // schema on one side only is review (a map closed, or array items
        // constrained).
        for k in ["items", "additionalProperties"] {
            match (o.get(k), n.get(k)) {
                (Some(a @ Value::Object(_)), Some(b @ Value::Object(_))) => {
                    self.compare(a, doc_o, b, doc_n, &format!("{at}.{k}"), seen, v);
                }
                (Some(Value::Object(_)), _) | (_, Some(Value::Object(_))) => {
                    v.push(
                        Class::Review,
                        "members_changed",
                        at,
                        format!("`{k}` gained or lost a schema"),
                    );
                }
                _ => {}
            }
        }
    }
}

/// The retention rule's identity check (spec §9.7): the same canonical form
/// once schema ids are replaced by artifact names, and the same artifacts
/// once protobuf descriptor sets are normalized.
#[must_use]
pub fn same_revision(a: &Revision, b: &Revision) -> bool {
    let named = |r: &Revision| -> Option<(Value, BTreeMap<String, Vec<u8>>)> {
        let mut by_name = BTreeMap::new();
        let mut ids = BTreeMap::new();
        for s in r.canonical["schemas"].as_array()? {
            let (id, name) = (s["id"].as_str()?, s["name"].as_str()?);
            let key = format!("{}:{name}", s["kind"].as_str()?);
            let bytes = match r.schemas.get(id)? {
                Schema::Protobuf(b) => normalize_fds(b)?,
                Schema::Json(v) => jcs(v),
            };
            by_name.insert(key.clone(), bytes);
            ids.insert(id.to_owned(), key);
        }
        let mut c = r.canonical.clone();
        c["schemas"] = Value::Null;
        replace_ids(&mut c, &ids);
        Some((c, by_name))
    };
    match (named(a), named(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

fn replace_ids(v: &mut Value, ids: &BTreeMap<String, String>) {
    match v {
        Value::String(s) => {
            if let Some(k) = ids.get(s.as_str()) {
                *s = k.clone();
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| replace_ids(x, ids)),
        Value::Object(m) => m.values_mut().for_each(|x| replace_ids(x, ids)),
        _ => {}
    }
}

/// A `FileDescriptorSet` with source info dropped and every `json_name`
/// equal to its default dropped, re-encoded.
fn normalize_fds(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut set = prost_types::FileDescriptorSet::decode(bytes).ok()?;
    for f in &mut set.file {
        f.source_code_info = None;
        for m in &mut f.message_type {
            normalize_message(m);
        }
    }
    Some(set.encode_to_vec())
}

fn normalize_message(m: &mut prost_types::DescriptorProto) {
    for f in &mut m.field {
        if f.json_name.as_deref() == Some(default_json_name(f.name()).as_str()) {
            f.json_name = None;
        }
    }
    for n in &mut m.nested_type {
        normalize_message(n);
    }
}

/// protoc's default JSON name: underscores removed, the next letter
/// uppercased.
fn default_json_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut up = false;
    for c in name.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::json;

    use super::*;
    use crate::contract::load_str;

    /// A one-type revision over a JSON Schema document, as the `compat/`
    /// fixtures wrap theirs.
    fn rev(dir: &Path, doc: &Value) -> Revision {
        std::fs::write(dir.join("t.json"), doc.to_string()).unwrap();
        let src = "[interface]\nname = \"m\"\nmajor = 1\nminor = 0\n[schemas]\njsonschema = [\"t.json\"]\n\
                   [resources.s]\nkind = \"state\"\ntype = \"json:T\"\n";
        let l = load_str(src, dir, None);
        Revision::of(&l.contract.unwrap_or_else(|| panic!("{}", l.report)))
    }

    fn class(old: &Value, new: &Value) -> &'static str {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("zk2-compat-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (o, n) = (rev(&dir, old), rev(&dir, new));
        std::fs::remove_dir_all(&dir).unwrap();
        compare(&o, &n).class().as_str()
    }

    fn doc(t: &Value) -> Value {
        json!({"$defs": {"T": t, "U": {"type": "integer"}}})
    }

    #[test]
    fn keywords_beside_a_ref_are_compared() {
        let a = doc(&json!({"type": "object", "properties": {"n": {"$ref": "#/$defs/U"}}}));
        let b = doc(
            &json!({"type": "object", "properties": {"n": {"$ref": "#/$defs/U", "maximum": 3}}}),
        );
        assert_eq!(class(&a, &b), "breaking");
        assert_eq!(class(&b, &b), "compatible");
        let inline =
            doc(&json!({"type": "object", "properties": {"n": {"type": "integer", "maximum": 3}}}));
        assert_eq!(class(&b, &inline), "compatible");
    }

    #[test]
    fn boolean_schemas_are_review() {
        let t = |p: Value| doc(&json!({"type": "object", "properties": {"p": p}}));
        assert_eq!(class(&t(json!(true)), &t(json!(false))), "review");
        assert_eq!(class(&t(json!(true)), &t(json!({}))), "review");
        assert_eq!(class(&t(json!(false)), &t(json!(false))), "compatible");
    }

    #[test]
    fn a_required_name_without_a_property_binds() {
        let t = |r: Value| doc(&json!({"type": "object", "required": r}));
        assert_eq!(class(&t(json!(["a"])), &t(json!([]))), "breaking");
        assert_eq!(class(&t(json!([])), &t(json!(["a"]))), "breaking");
        assert_eq!(class(&t(json!(["a"])), &t(json!(["a"]))), "compatible");
    }

    #[test]
    fn property_names_inside_oneof_are_not_annotations() {
        let t = |ty: &str| {
            doc(&json!({"oneOf": [{"type": "object", "properties": {"title": {"type": ty}}}]}))
        };
        assert_eq!(class(&t("string"), &t("integer")), "review");
        let d = |text: &str| doc(&json!({"oneOf": [{"type": "string", "description": text}]}));
        assert_eq!(class(&d("a"), &d("b")), "compatible");
    }
}
