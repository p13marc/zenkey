//! `config.v2`: a configuration declared in its contract, read back in one
//! packed record (`spec/profiles/config/v2.md`, text 0.1, #985 G1).
//!
//! The profile's session-free half. `config.v2` keeps every rule of
//! `config.v1` and replaces its wire, so this module reads the declaration,
//! derives the types, writes and reads the record and the requests, and
//! runs [`super`]'s validators and [`super::Owner`] unchanged under them:
//!
//! ```text
//! Groups::read(annotation, packed files) ─▶ Groups │ [Invalid{entry, reason}]       §2.2
//! Derived::of(&Groups)                   ─▶ Record, Set<T> per settable group, Control, Error   §3
//! Interface::of(&Contract)               ─▶ the groups and their derived types
//! lint(&Resources), lint_contract(c)     ─▶ [Finding{at, reason}], in §2.2's order
//! Interface::write_record / read_record  ─▶ the record's bytes │ State │ RecordRefusal   §2.3
//! Interface::decode_set / decode_control ─▶ the request, a query's payload (core §7.5)
//! validate_set / validate_control        ─▶ () │ Refusal, config.v1's rows over the record's facts §2.5
//! check_desired                          ─▶ () │ DesiredRefused, by declared names        §2.18
//! deadline, apply_at                     ─▶ instants rounded up to the second           §2.8
//! Owner (over config::Owner)             ─▶ puts, then the reply, then events, each stamped §2.3–§2.17
//! ```
//!
//! A [`Mint`] stamps what the owner writes: the run it writes under, each
//! seq, a new run when the seq or the token counter is spent (core §4.3),
//! and each run's first token count. The runtime implements it over the
//! service's own minter, since the seq is the instance's, shared by every
//! state key and event it writes.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json, json};

use super::{
    Act, Call, Class, Code, ConfigView, ControlOp, ControlRequest, DesiredConfig, DesiredOutcome,
    DesiredReport, DeviceResult, Edit, Field, GroupDecl, Kind as V1Kind, Outcome, ParamDecl,
    PersistPlan, REDACTED, Refusal, Schema, SetReply, SetRequest, Source, StartError, Time, Value,
};
use crate::authoring::{Fanout, Kind, Replies, Serving};
use crate::contract::{Body as ContractBody, Contract, Rate};
use crate::envelope::{Cause, Code as ErrorCode, ErrorForm};
use crate::grammar::KindToken;
use crate::packed::{self, FieldKind, Item, Layout};
use crate::schema::{ArtifactData, SchemaKind, TypeId};
use crate::wire::{self, Envelope, Request, Run, Seq, Stamp, Tag};

/// The profile's id.
pub const PROFILE: &str = "config.v2";
/// The annotation that declares the groups (§4).
pub const GROUPS: &str = "config.groups";
/// The state that carries the record, `state/config` (§2.3).
pub const CONFIG: &str = "config";
/// The operations' templates beside `set/<group>` (§3).
pub const CONTROLS: [&str; 4] = [super::CONFIRM, super::CANCEL, super::EXTEND, super::PERSIST];
/// The change event's template (§2.17).
pub const CHANGE: &str = "change";
/// A marker of the declaration, and of `Record` (§2.2, §3).
pub const MARKER: &str = packed::MARKER;
/// The most groups a declaration holds, and the most parameters a group.
pub const MAX_GROUPS: usize = 16;
/// The most parameters a group type holds (§2.2).
pub const MAX_PARAMETERS: usize = 16;
/// The names a parameter may not take: the derived types use them (§2.2).
pub const RESERVED_NAMES: [&str; 8] = [
    "confirm_s",
    "token",
    "expected_revision",
    "idempotency_key",
    "dry_run",
    "writable",
    "pending",
    "last",
];
/// The largest token, 48 bits (§2.6).
pub const TOKEN_MAX: u64 = (1 << 48) - 1;
/// The largest token count within one run (§2.6).
pub const COUNTER_MAX: u32 = 65_535;
/// The standard schema's first revision, by artifact id (§3).
pub const STANDARD_SCHEMA: &str =
    "sha256:0dadb0afae6b405abf958f23a2ff8d61063580028afb6980fdfc208d53a730f1";
/// The change event's type in the standard schema (§2.17).
pub const EVENT_TYPE: &str = "ConfigChangeEvent";
/// The change event's rate and retention (§3).
pub const EVENT_RATE: Rate = Rate::Burst(600);
/// Seven days, in seconds.
pub const EVENT_RETENTION_S: u64 = 7 * 86_400;
/// The derived types' names beside `Set<T>`.
pub const RECORD: &str = "Record";
/// `Control`, the control operations' request.
pub const CONTROL: &str = "Control";
/// `Error`, every operation's `app` detail.
pub const ERROR: &str = "Error";
/// `Error`'s one code at this version (§2.5).
pub const DEVICE_REFUSED: &str = "device_refused";
/// The sources a parameter's `__source` field names, in code order (§4).
pub const SOURCES: [&str; 4] = ["default", "file", "overlay", "runtime"];
/// The desired outcomes, in code order (§4).
pub const OUTCOMES: [&str; 3] = ["converged", "refused", "failed"];
/// The desired refusal codes, in code order (§4).
pub const DESIRED_CODES: [&str; 8] = [
    "reach",
    "contract",
    "sensitive",
    "not_found",
    "invalid_request",
    "unavailable",
    "busy",
    "device_refused",
];

/// Whether `s` is a group's or a parameter's name:
/// `[a-z][a-z0-9]*(_[a-z0-9]+)*` (§2.2).
#[must_use]
pub fn is_name(s: &str) -> bool {
    let b = s.as_bytes();
    if !b.first().is_some_and(u8::is_ascii_lowercase) {
        return false;
    }
    let mut underscore = false;
    for &c in &b[1..] {
        if c == b'_' {
            if underscore {
                return false;
            }
            underscore = true;
        } else if c.is_ascii_lowercase() || c.is_ascii_digit() {
            underscore = false;
        } else {
            return false;
        }
    }
    !underscore
}

// --- the declaration (§2.2) -----------------------------------------------------

/// Why a declaration is invalid (§2.2): the annotation whole, then each
/// item's checks in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reason {
    /// Not an array, no group, more than 16, or a marker first, last or
    /// after a marker.
    GroupsValue,
    /// An item that is neither a table nor a marker, or a table holding a
    /// member outside the five.
    GroupValue,
    /// `name` missing, not of the form, or an earlier group's.
    GroupName,
    /// `class` not `hot`, `reach` or `contract`.
    GroupClass,
    /// `type` not a packed type a group may name, or an earlier group's.
    GroupType,
    /// `units` not a table of `int` parameters' non-empty units.
    GroupUnits,
    /// `sensitive` not an array of parameter names, each once.
    GroupSensitive,
}

impl Reason {
    /// The fixtures' token.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Self::GroupsValue => "groups_value",
            Self::GroupValue => "group_value",
            Self::GroupName => "group_name",
            Self::GroupClass => "group_class",
            Self::GroupType => "group_type",
            Self::GroupUnits => "group_units",
            Self::GroupSensitive => "group_sensitive",
        }
    }
}

/// One reason a declaration is invalid, at its item's index in the array
/// (markers counted), or `None` for the annotation whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Invalid {
    pub entry: Option<usize>,
    pub reason: Reason,
}

/// A parameter's kind at this version (§2.2): config.v1's `integer` with
/// its bounds, or `bool`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamKind {
    Int { minimum: i64, maximum: i64 },
    Bool,
}

/// One parameter: a field of its group's type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parameter {
    pub name: String,
    pub kind: ParamKind,
    /// Its width in bits (core §7.6).
    pub width: u32,
    /// Its unit, from the entry's `units`.
    pub unit: Option<String>,
    /// Write-only (§2.15).
    pub sensitive: bool,
    /// Its field's `description`, for a person.
    pub description: Option<String>,
}

impl Parameter {
    /// Its field in a derived type, optional or not.
    fn field(&self, name: String, optional: bool) -> FieldDecl {
        match self.kind {
            ParamKind::Int { minimum, maximum } => FieldDecl::int(name, minimum, maximum, optional),
            ParamKind::Bool => FieldDecl::boolean(name, optional),
        }
    }

    /// Reads a JSON value as this parameter's: `true` or `false` for a
    /// `bool`, an integer within the bounds for an `int`.
    fn value_of(&self, v: &Json) -> Option<Value> {
        match self.kind {
            ParamKind::Bool => v.as_bool().map(Value::Bool),
            ParamKind::Int { minimum, maximum } => v
                .as_i64()
                .filter(|i| (minimum..=maximum).contains(i))
                .map(Value::Integer),
        }
    }
}

/// One group, as its entry and its type declare it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub class: Class,
    /// Its type's name in its packed file, without a stem.
    pub type_name: String,
    /// Its type's `description`, for a person.
    pub description: Option<String>,
    /// In its type's field order.
    pub parameters: Vec<Parameter>,
    /// Whether a marker comes before it: a later revision's group, every
    /// field of which is optional in `Record` (§3).
    pub appended: bool,
}

impl Group {
    /// A `hot` or `reach` group, which has a `set` (§0).
    #[must_use]
    pub fn settable(&self) -> bool {
        matches!(self.class, Class::Hot | Class::Reach)
    }

    /// The parameter by name, with its index in the group.
    #[must_use]
    pub fn parameter(&self, name: &str) -> Option<(usize, &Parameter)> {
        self.parameters
            .iter()
            .enumerate()
            .find(|(_, p)| p.name == name)
    }

    /// Its `set`'s request type's name, `Set<T>`.
    #[must_use]
    pub fn set_type(&self) -> String {
        format!("Set{}", self.type_name)
    }
}

/// An item of the declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Group(Group),
    /// `"..."`: the groups after it are a later revision's.
    Marker,
}

/// A declaration read: its groups and markers, in the owner's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Groups {
    pub entries: Vec<Entry>,
}

/// A packed reference resolved: its file's stem, the type's name, and its
/// declaration.
struct Resolved<'a> {
    stem: &'a str,
    name: &'a str,
    decl: &'a Json,
    doc: &'a Json,
}

/// Resolves `packed:<Name>` or `packed:<stem>#<Name>` among the listed
/// packed files, `(stem, document)`, as core §9.4 does: one file of that
/// stem, or the one file that defines the name.
fn resolve<'a>(reference: &'a str, files: &[(&'a str, &'a Json)]) -> Option<Resolved<'a>> {
    let rest = reference.strip_prefix("packed:")?;
    let (stem, name) = match rest.split_once('#') {
        Some((s, n)) => (Some(s), n),
        None => (None, rest),
    };
    let found: Vec<Resolved<'a>> = files
        .iter()
        .filter(|(s, _)| stem.is_none_or(|x| x == *s))
        .filter_map(|(s, doc)| {
            doc.get("types")?.get(name).map(|decl| Resolved {
                stem: s,
                name,
                decl,
                doc,
            })
        })
        .collect();
    match <[_; 1]>::try_from(found) {
        Ok([r]) => Some(r),
        Err(_) => None,
    }
}

/// A group type's parameters, or `None` when the type is not one a group
/// may name (§2.2): it must read as a packed type, and hold int and bool
/// fields alone, at most 16, none optional, none with `reserved`, each
/// named in a group name's form and none of [`RESERVED_NAMES`].
fn group_type(r: &Resolved<'_>) -> Option<Vec<Parameter>> {
    let fields = r.decl.get("fields")?.as_array()?;
    if fields.len() > MAX_PARAMETERS {
        return None;
    }
    for f in fields {
        let f = f.as_object()?;
        let name = f.get("name")?.as_str()?;
        if f.get("optional").and_then(Json::as_bool) == Some(true)
            || f.contains_key("reserved")
            || !matches!(f.get("type").and_then(Json::as_str), Some("int" | "bool"))
            || !is_name(name)
            || RESERVED_NAMES.contains(&name)
        {
            return None;
        }
    }
    let layout = packed::layout_in(r.doc, r.name).ok()?;
    layout
        .items
        .iter()
        .zip(fields)
        .map(|(it, raw)| {
            let Item::Field(f) = it else {
                return None;
            };
            let kind = match f.kind {
                FieldKind::Int {
                    minimum, maximum, ..
                } => ParamKind::Int { minimum, maximum },
                FieldKind::Bool => ParamKind::Bool,
                _ => return None,
            };
            Some(Parameter {
                name: f.name.clone(),
                kind,
                width: f.width,
                unit: None,
                sensitive: false,
                description: raw
                    .get("description")
                    .and_then(Json::as_str)
                    .map(str::to_owned),
            })
        })
        .collect()
}

const ENTRY_MEMBERS: [&str; 5] = ["name", "class", "type", "units", "sensitive"];

impl Groups {
    /// Reads `config.groups`, the annotation's value as TOML reads it into
    /// JSON, against the contract's packed files, each `(stem, document)`
    /// (§2.2): the groups, or every reason it is invalid, in the text's
    /// order.
    ///
    /// # Errors
    ///
    /// The reasons, the annotation whole's alone when it is wrong.
    pub fn read(annotation: &Json, files: &[(&str, &Json)]) -> Result<Self, Vec<Invalid>> {
        let whole = || {
            vec![Invalid {
                entry: None,
                reason: Reason::GroupsValue,
            }]
        };
        let Some(items) = annotation.as_array() else {
            return Err(whole());
        };
        let is_marker = |v: &Json| v.as_str() == Some(MARKER);
        let groups = items.iter().filter(|v| !is_marker(v)).count();
        let misplaced = items.first().is_some_and(is_marker)
            || items.last().is_some_and(is_marker)
            || items
                .windows(2)
                .any(|w| is_marker(&w[0]) && is_marker(&w[1]));
        if groups == 0 || groups > MAX_GROUPS || misplaced {
            return Err(whole());
        }
        let mut reasons = Vec::new();
        let mut entries = Vec::new();
        let mut names: Vec<&str> = Vec::new();
        let mut types: Vec<(&str, &str)> = Vec::new();
        let mut appended = false;
        for (i, item) in items.iter().enumerate() {
            let mut fail = |reason| {
                reasons.push(Invalid {
                    entry: Some(i),
                    reason,
                });
            };
            if is_marker(item) {
                appended = true;
                entries.push(Entry::Marker);
                continue;
            }
            let Some(m) = item.as_object() else {
                fail(Reason::GroupValue);
                continue;
            };
            if m.keys().any(|k| !ENTRY_MEMBERS.contains(&k.as_str())) {
                fail(Reason::GroupValue);
            }
            let name = m.get("name").and_then(Json::as_str);
            let name_ok = name.is_some_and(|n| is_name(n) && !names.contains(&n));
            if !name_ok {
                fail(Reason::GroupName);
            }
            if let Some(n) = name {
                names.push(n);
            }
            let class = match m.get("class").and_then(Json::as_str) {
                Some("hot") => Some(Class::Hot),
                Some("reach") => Some(Class::Reach),
                Some("contract") => Some(Class::Contract),
                _ => None,
            };
            if class.is_none() {
                fail(Reason::GroupClass);
            }
            let resolved = m
                .get("type")
                .and_then(Json::as_str)
                .and_then(|t| resolve(t, files));
            let earlier = resolved
                .as_ref()
                .is_some_and(|r| types.contains(&(r.stem, r.name)));
            let mut params = resolved.as_ref().filter(|_| !earlier).and_then(group_type);
            if let Some(r) = &resolved {
                types.push((r.stem, r.name));
            }
            if params.is_none() {
                fail(Reason::GroupType);
            }
            if let Some(units) = m.get("units") {
                let ok = match (units.as_object(), &params) {
                    (None, _) => false,
                    (Some(_), None) => true,
                    (Some(u), Some(ps)) => u.iter().all(|(k, v)| {
                        ps.iter()
                            .any(|p| p.name == *k && matches!(p.kind, ParamKind::Int { .. }))
                            && v.as_str().is_some_and(|s| !s.is_empty())
                    }),
                };
                if ok {
                    if let (Some(u), Some(ps)) = (units.as_object(), params.as_mut()) {
                        for p in ps.iter_mut() {
                            p.unit = u.get(&p.name).and_then(Json::as_str).map(str::to_owned);
                        }
                    }
                } else {
                    fail(Reason::GroupUnits);
                }
            }
            if let Some(sensitive) = m.get("sensitive") {
                let ok = match (sensitive.as_array(), &params) {
                    (None, _) => false,
                    (Some(_), None) => true,
                    (Some(s), Some(ps)) => {
                        let mut seen = BTreeSet::new();
                        s.iter().all(|v| {
                            v.as_str().is_some_and(|n| {
                                ps.iter().any(|p| p.name == n) && seen.insert(n.to_owned())
                            })
                        })
                    }
                };
                if ok {
                    if let (Some(s), Some(ps)) = (sensitive.as_array(), params.as_mut()) {
                        for p in ps.iter_mut() {
                            p.sensitive = s.iter().any(|v| v.as_str() == Some(p.name.as_str()));
                        }
                    }
                } else {
                    fail(Reason::GroupSensitive);
                }
            }
            if let (Some(n), Some(c), Some(ps), Some(r)) = (name, class, params, &resolved) {
                entries.push(Entry::Group(Group {
                    name: n.to_owned(),
                    class: c,
                    type_name: r.name.to_owned(),
                    description: r
                        .decl
                        .get("description")
                        .and_then(Json::as_str)
                        .map(str::to_owned),
                    parameters: ps,
                    appended,
                }));
            }
        }
        if reasons.is_empty() {
            Ok(Self { entries })
        } else {
            Err(reasons)
        }
    }

    /// The groups, markers left out, in the declaration's order: a
    /// group's index here is the one the record's desired report names.
    pub fn iter(&self) -> impl Iterator<Item = &Group> {
        self.entries.iter().filter_map(|e| match e {
            Entry::Group(g) => Some(g),
            Entry::Marker => None,
        })
    }

    /// The group by name, with its index.
    #[must_use]
    pub fn group(&self, name: &str) -> Option<(usize, &Group)> {
        self.iter().enumerate().find(|(_, g)| g.name == name)
    }

    /// The settable group by name: the one `set/<name>` reaches.
    #[must_use]
    pub fn settable(&self, name: &str) -> Option<&Group> {
        self.group(name).map(|(_, g)| g).filter(|g| g.settable())
    }

    /// config.v1's schema of these groups, each `writable` where the
    /// startup allowlist holds its name: what `config.v1`'s validators and
    /// [`super::Owner`] run under, unchanged.
    #[must_use]
    pub fn schema(&self, writable: &BTreeSet<String>) -> Schema {
        Schema {
            groups: self
                .iter()
                .map(|g| GroupDecl {
                    name: g.name.clone(),
                    class: g.class,
                    description: g.description.clone().unwrap_or_default(),
                    writable: g.settable() && writable.contains(&g.name),
                    parameters: g
                        .parameters
                        .iter()
                        .map(|p| ParamDecl {
                            name: p.name.clone(),
                            kind: match p.kind {
                                ParamKind::Int { minimum, maximum } => V1Kind::Integer {
                                    min: Some(minimum),
                                    max: Some(maximum),
                                    unit: p.unit.clone(),
                                },
                                ParamKind::Bool => V1Kind::Bool,
                            },
                            description: p.description.clone().unwrap_or_default(),
                            sensitive: p.sensitive,
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    /// The declaration as `derive.json` spells its groups: each group's
    /// name, class, type, whether settable, and its parameters; a marker
    /// where it stands.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::Array(
            self.entries
                .iter()
                .map(|e| match e {
                    Entry::Marker => json!(MARKER),
                    Entry::Group(g) => json!({
                        "name": g.name,
                        "class": g.class.as_str(),
                        "type": g.type_name,
                        "settable": g.settable(),
                        "parameters": g.parameters.iter().map(|p| {
                            let (kind, minimum, maximum) = match p.kind {
                                ParamKind::Int { minimum, maximum } => ("int", json!(minimum), json!(maximum)),
                                ParamKind::Bool => ("bool", Json::Null, Json::Null),
                            };
                            json!({
                                "name": p.name,
                                "kind": kind,
                                "minimum": minimum,
                                "maximum": maximum,
                                "width": p.width,
                                "unit": p.unit,
                                "sensitive": p.sensitive,
                            })
                        }).collect::<Vec<_>>(),
                    }),
                })
                .collect(),
        )
    }
}

// --- the derived types (§3) -------------------------------------------------------

/// One field of a derived type, as `zk2 config derive` writes it: its
/// members in this order, and no description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct FieldDecl {
    name: String,
    #[serde(rename = "type")]
    ty: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    minimum: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    maximum: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    values: Option<Vec<&'static str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    escape: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    optional: Option<bool>,
}

impl FieldDecl {
    fn new(name: String, ty: &'static str, optional: bool) -> Self {
        Self {
            name,
            ty,
            minimum: None,
            maximum: None,
            values: None,
            escape: None,
            width: None,
            optional: optional.then_some(true),
        }
    }

    fn int(name: impl Into<String>, minimum: i64, maximum: i64, optional: bool) -> Self {
        Self {
            minimum: Some(minimum),
            maximum: Some(maximum),
            ..Self::new(name.into(), "int", optional)
        }
    }

    fn boolean(name: impl Into<String>, optional: bool) -> Self {
        Self::new(name.into(), "bool", optional)
    }

    fn time(name: &str, optional: bool) -> Self {
        Self::new(name.into(), "time", optional)
    }

    fn enumeration(
        name: impl Into<String>,
        values: &[&'static str],
        escape: bool,
        optional: bool,
    ) -> Self {
        Self {
            values: Some(values.to_vec()),
            escape: escape.then_some(true),
            ..Self::new(name.into(), "enum", optional)
        }
    }
}

/// An item of a derived type's `fields`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FieldItem {
    Field(FieldDecl),
    Marker,
}

impl Serialize for FieldItem {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Field(f) => f.serialize(s),
            Self::Marker => s.serialize_str(MARKER),
        }
    }
}

/// A derived type's declaration: a description for a person, then its
/// fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct TypeDecl {
    description: String,
    fields: Vec<FieldItem>,
}

/// The derived types of a declaration, each with its layout (§3): what
/// `zk2 config derive` writes, and what the lint compares a contract's
/// types with.
#[derive(Debug, Clone)]
pub struct Derived {
    types: Vec<(String, TypeDecl)>,
    layouts: BTreeMap<String, Layout>,
    /// Each settable group's `Set<T>` type name, by group.
    sets: BTreeMap<String, String>,
}

/// `Record`'s header (§3), in order.
fn header() -> Vec<FieldItem> {
    let u32_max = i64::from(u32::MAX);
    let token_max = TOKEN_MAX as i64;
    [
        FieldDecl::int("instance", 0, u32_max, false),
        FieldDecl::int("revision", 0, u32_max, false),
        FieldDecl::int("floor_s", 1, i64::from(u16::MAX), false),
        FieldDecl::boolean("would_be", false),
        FieldDecl::int("pending_token", 0, token_max, true),
        FieldDecl::time("pending_apply_at", true),
        FieldDecl::time("pending_deadline", true),
        FieldDecl::int("last_token", 0, token_max, true),
        FieldDecl::enumeration("desired", &OUTCOMES, true, true),
        FieldDecl::int("desired_revision", 0, u32_max, true),
        FieldDecl::enumeration("desired_code", &DESIRED_CODES, true, true),
        FieldDecl::int("desired_group", 0, 15, true),
        FieldDecl::int("desired_parameter", 0, 15, true),
    ]
    .into_iter()
    .map(FieldItem::Field)
    .collect()
}

/// A group's fields in `Record`: its three bits when settable, then each
/// parameter's source, and, unless sensitive, its `differs` bit and value.
fn group_fields(g: &Group) -> Vec<FieldItem> {
    let opt = g.appended;
    let mut out = Vec::new();
    if g.settable() {
        for bit in ["writable", "pending", "last"] {
            out.push(FieldDecl::boolean(format!("{}__{bit}", g.name), opt));
        }
    }
    for p in &g.parameters {
        let at = format!("{}__{}", g.name, p.name);
        out.push(FieldDecl::enumeration(
            format!("{at}__source"),
            &SOURCES,
            false,
            true,
        ));
        if !p.sensitive {
            out.push(FieldDecl::boolean(format!("{at}__differs"), opt));
            out.push(p.field(at, true));
        }
    }
    out.into_iter().map(FieldItem::Field).collect()
}

fn layout_of(name: &str, decl: &TypeDecl) -> Layout {
    let doc = json!({ "types": { name: decl } });
    packed::layout_in(&doc, name).expect("a derived type reads as a packed type")
}

impl Derived {
    /// The derivation (§3): `Record`, then `Set<T>` for each settable group
    /// in the groups' order, then `Control` and `Error`.
    #[must_use]
    pub fn of(groups: &Groups) -> Self {
        let mut record = header();
        for e in &groups.entries {
            match e {
                Entry::Group(g) => record.extend(group_fields(g)),
                Entry::Marker => record.push(FieldItem::Marker),
            }
        }
        record.push(FieldItem::Marker);
        let mut types = vec![(
            RECORD.to_owned(),
            TypeDecl {
                description:
                    "config.v2's record, derived (v2.md 2.3, 3): state/config, every reply"
                        .to_owned(),
                fields: record,
            },
        )];
        let mut sets = BTreeMap::new();
        for g in groups.iter().filter(|g| g.settable()) {
            let mut fields: Vec<FieldItem> = g
                .parameters
                .iter()
                .map(|p| FieldItem::Field(p.field(p.name.clone(), true)))
                .collect();
            fields.extend(
                [
                    FieldDecl::int("confirm_s", 1, i64::from(u16::MAX), true),
                    FieldDecl::int("token", 0, TOKEN_MAX as i64, true),
                    FieldDecl::int("expected_revision", 0, i64::from(u32::MAX), true),
                    FieldDecl::int("idempotency_key", 0, i64::from(u32::MAX), true),
                    FieldDecl::boolean("dry_run", false),
                ]
                .into_iter()
                .map(FieldItem::Field),
            );
            let name = g.set_type();
            sets.insert(g.name.clone(), name.clone());
            types.push((
                name,
                TypeDecl {
                    description: format!(
                        "set/{}'s request, derived from {} (v2.md 2.5, 3)",
                        g.name, g.type_name
                    ),
                    fields,
                },
            ));
        }
        types.push((
            CONTROL.to_owned(),
            TypeDecl {
                description:
                    "confirm, cancel, extend and persist's request, derived (v2.md 2.10, 3)"
                        .to_owned(),
                fields: vec![
                    FieldItem::Field(FieldDecl::int("token", 0, TOKEN_MAX as i64, false)),
                    FieldItem::Field(FieldDecl::int("confirm_s", 1, i64::from(u16::MAX), true)),
                ],
            },
        ));
        types.push((
            ERROR.to_owned(),
            TypeDecl {
                description: "Every operation's error detail, derived (v2.md 2.5, 3)".to_owned(),
                fields: vec![FieldItem::Field(FieldDecl {
                    width: Some(4),
                    ..FieldDecl::enumeration("code", &[DEVICE_REFUSED], true, false)
                })],
            },
        ));
        let layouts = types
            .iter()
            .map(|(n, d)| (n.clone(), layout_of(n, d)))
            .collect();
        Self {
            types,
            layouts,
            sets,
        }
    }

    /// The derived types' names, in derivation order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.types.iter().map(|(n, _)| n.as_str())
    }

    /// A derived type's declaration as JSON, its description included.
    #[must_use]
    pub fn declaration(&self, name: &str) -> Option<Json> {
        self.types
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, d)| serde_json::to_value(d).expect("a declaration is JSON"))
    }

    /// A derived type's layout.
    #[must_use]
    pub fn layout(&self, name: &str) -> Option<&Layout> {
        self.layouts.get(name)
    }

    /// `Record`'s layout.
    #[must_use]
    pub fn record(&self) -> &Layout {
        &self.layouts[RECORD]
    }

    /// `Control`'s layout.
    #[must_use]
    pub fn control(&self) -> &Layout {
        &self.layouts[CONTROL]
    }

    /// `Error`'s layout.
    #[must_use]
    pub fn error(&self) -> &Layout {
        &self.layouts[ERROR]
    }

    /// A settable group's `Set<T>`: its name and layout.
    #[must_use]
    pub fn set(&self, group: &str) -> Option<(&str, &Layout)> {
        let name = self.sets.get(group)?;
        Some((name, &self.layouts[name]))
    }

    /// The derived types as a packed file's text, `{"types": {...}}`, in
    /// derivation order, each field's members in a fixed order and indented
    /// by two spaces: what `zk2 config derive` prints.
    #[must_use]
    pub fn to_text(&self) -> String {
        struct Types<'a>(&'a [(String, TypeDecl)]);
        impl Serialize for Types<'_> {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                use serde::ser::SerializeMap as _;
                let mut m = s.serialize_map(Some(self.0.len()))?;
                for (n, d) in self.0 {
                    m.serialize_entry(n, d)?;
                }
                m.end()
            }
        }
        #[derive(Serialize)]
        struct Doc<'a> {
            types: Types<'a>,
        }
        serde_json::to_string_pretty(&Doc {
            types: Types(&self.types),
        })
        .expect("a declaration is JSON")
            + "\n"
    }
}

/// A config interface's declaration and derived types: everything a
/// writer and a reader of its wire need from its contract.
#[derive(Debug, Clone)]
pub struct Interface {
    pub groups: Groups,
    pub derived: Derived,
}

/// Why a contract has no config interface to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unread {
    /// No state `config` with `config.groups`.
    Missing,
    /// The declaration is invalid.
    Invalid(Vec<Invalid>),
}

impl fmt::Display for Unread {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => write!(f, "no state `{CONFIG}` declaring `{GROUPS}`"),
            Self::Invalid(r) => {
                write!(f, "`{GROUPS}` is invalid:")?;
                for i in r {
                    match i.entry {
                        Some(e) => write!(f, " [{e}] {}", i.reason.token())?,
                        None => write!(f, " {}", i.reason.token())?,
                    }
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for Unread {}

/// A contract's packed files, `(stem, document)`.
fn packed_files(c: &Contract) -> Vec<(&str, &Json)> {
    c.artifacts
        .values()
        .filter(|a| a.kind == SchemaKind::Packed)
        .filter_map(|a| match &a.data {
            ArtifactData::Json(doc) => Some((a.name.as_str(), doc)),
            ArtifactData::Protobuf(_) => None,
        })
        .collect()
}

impl Interface {
    /// Reads a declaration against its packed files, and derives its types.
    ///
    /// # Errors
    ///
    /// [`Groups::read`]'s reasons.
    pub fn read(annotation: &Json, files: &[(&str, &Json)]) -> Result<Self, Vec<Invalid>> {
        let groups = Groups::read(annotation, files)?;
        let derived = Derived::of(&groups);
        Ok(Self { groups, derived })
    }

    /// A contract's config interface: its state `config`'s `config.groups`.
    /// A reader MUST NOT read a record by a contract whose lint has a
    /// finding (§2.2); [`lint_contract`] says.
    ///
    /// # Errors
    ///
    /// [`Unread`].
    pub fn of(contract: &Contract) -> Result<Self, Unread> {
        let ann = contract
            .resource(KindToken::State, CONFIG)
            .and_then(|r| r.annotations.get(GROUPS))
            .ok_or(Unread::Missing)?;
        Self::read(ann, &packed_files(contract)).map_err(Unread::Invalid)
    }

    /// config.v1's schema of the groups (§2.1, [`Groups::schema`]).
    #[must_use]
    pub fn schema(&self, writable: &BTreeSet<String>) -> Schema {
        self.groups.schema(writable)
    }
}

// --- the record (§2.3) ---------------------------------------------------------------

/// The pending change as the record carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    pub token: u64,
    /// Its groups, in the contract's order.
    pub groups: Vec<String>,
    /// While a reach group waits: when the last to join applies, in
    /// milliseconds since the Unix epoch, a whole second.
    pub apply_at: Option<i64>,
    /// When its undo runs, in milliseconds since the Unix epoch.
    pub deadline: i64,
    /// Whether every group of it is applied: no `apply_at`.
    pub applied: bool,
}

/// The last change as the record carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Last {
    pub token: u64,
    /// Its groups, in the contract's order.
    pub groups: Vec<String>,
}

/// A desired refusal's code (§4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesiredCode {
    Reach,
    Contract,
    Sensitive,
    NotFound,
    InvalidRequest,
    Unavailable,
    Busy,
    DeviceRefused,
}

impl DesiredCode {
    /// Its name, as [`DESIRED_CODES`] spells it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        DESIRED_CODES[self as usize]
    }

    fn from_name(s: &str) -> Option<Self> {
        use DesiredCode as D;
        [
            D::Reach,
            D::Contract,
            D::Sensitive,
            D::NotFound,
            D::InvalidRequest,
            D::Unavailable,
            D::Busy,
            D::DeviceRefused,
        ]
        .into_iter()
        .find(|c| c.as_str() == s)
    }
}

impl From<super::DesiredCode> for DesiredCode {
    fn from(c: super::DesiredCode) -> Self {
        use super::DesiredCode as V;
        match c {
            V::Reach => Self::Reach,
            V::Contract => Self::Contract,
            V::Sensitive => Self::Sensitive,
            V::NotFound => Self::NotFound,
            V::InvalidRequest => Self::InvalidRequest,
            V::Unavailable => Self::Unavailable,
            V::Busy => Self::Busy,
            V::DeviceRefused => Self::DeviceRefused,
        }
    }
}

/// Why a desired document was refused or failed, by declared names: a
/// group or a parameter the contract does not declare is `None`, since
/// the record carries indices (§2.18).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesiredRefused {
    /// `None`: a code this revision does not name, the escape.
    pub code: Option<DesiredCode>,
    pub group: Option<String>,
    pub parameter: Option<String>,
}

/// The desired report as the record carries it (§2.18).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Desired {
    /// `None`: an outcome this revision does not name, the escape.
    pub outcome: Option<DesiredOutcome>,
    pub observed_revision: u32,
    pub refusal: Option<DesiredRefused>,
}

/// One parameter as the record carries it (§2.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamState {
    /// A parameter that is not sensitive: its source and value, absent
    /// when unknown, and whether its startup value differs.
    Plain {
        source: Option<Source>,
        differs: bool,
        value: Option<Value>,
    },
    /// A sensitive parameter: its source alone (§2.15).
    Sensitive { source: Option<Source> },
}

/// One group as the record carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupState {
    /// A settable group's: whether the allowlist lets a `set` reach it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writable: Option<bool>,
    pub params: BTreeMap<String, ParamState>,
}

/// The record, as `config.v2` reads one (§2.3): the fixtures' `state`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// The instance number: the run the instance first wrote under.
    pub instance: u32,
    pub revision: u32,
    pub floor_s: u16,
    /// Set only in a dry run's reply (§2.9).
    pub would_be: bool,
    pub pending: Option<Pending>,
    pub last: Option<Last>,
    pub desired: Option<Desired>,
    /// By name. A group an earlier revision's writer did not write is
    /// absent.
    pub groups: BTreeMap<String, GroupState>,
}

/// Why a reader refused a record (§2.3): core §7.6's refusals, then
/// config.v2's, the first that applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordRefusal {
    Packed(packed::ReadRefusal),
    /// A group behind a marker written in part.
    Group,
    /// The pending fields not all present or all absent, or a pending
    /// change in no group.
    Pending,
    /// A last token and no group's last bit, or the reverse.
    Last,
    /// The desired fields do not hold together (§3).
    Desired,
    /// A value and its source not both present or both absent, or
    /// `differs` set with no value.
    Parameter,
}

impl RecordRefusal {
    /// The fixtures' token.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Self::Packed(r) => r.token(),
            Self::Group => "group",
            Self::Pending => "pending",
            Self::Last => "last",
            Self::Desired => "desired",
            Self::Parameter => "parameter",
        }
    }
}

impl fmt::Display for RecordRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.token())
    }
}

impl std::error::Error for RecordRefusal {}

fn source_name(s: Source) -> &'static str {
    s.as_str()
}

fn source_of(s: &str) -> Option<Source> {
    match s {
        "default" => Some(Source::Default),
        "file" => Some(Source::File),
        "overlay" => Some(Source::Overlay),
        "runtime" => Some(Source::Runtime),
        _ => None,
    }
}

fn outcome_of(s: &str) -> Option<DesiredOutcome> {
    match s {
        "converged" => Some(DesiredOutcome::Converged),
        "refused" => Some(DesiredOutcome::Refused),
        "failed" => Some(DesiredOutcome::Failed),
        _ => None,
    }
}

fn outcome_name(o: DesiredOutcome) -> &'static str {
    match o {
        DesiredOutcome::Converged => "converged",
        DesiredOutcome::Refused => "refused",
        DesiredOutcome::Failed => "failed",
    }
}

fn value_json(v: &Value) -> Json {
    serde_json::to_value(v).expect("a value is JSON")
}

impl Interface {
    /// A record's JSON reading (core §7.6): its fields by name, an absent
    /// optional field no member, a time in milliseconds since the Unix
    /// epoch.
    #[must_use]
    pub fn fields(&self, s: &State) -> Json {
        let mut m = Map::new();
        m.insert("instance".into(), s.instance.into());
        m.insert("revision".into(), s.revision.into());
        m.insert("floor_s".into(), s.floor_s.into());
        m.insert("would_be".into(), s.would_be.into());
        if let Some(p) = &s.pending {
            m.insert("pending_token".into(), p.token.into());
            if let Some(a) = p.apply_at {
                m.insert("pending_apply_at".into(), a.into());
            }
            m.insert("pending_deadline".into(), p.deadline.into());
        }
        if let Some(l) = &s.last {
            m.insert("last_token".into(), l.token.into());
        }
        if let Some(d) = &s.desired {
            m.insert(
                "desired".into(),
                d.outcome.map_or(Json::Null, |o| outcome_name(o).into()),
            );
            m.insert("desired_revision".into(), d.observed_revision.into());
            if let Some(r) = &d.refusal {
                m.insert(
                    "desired_code".into(),
                    r.code.map_or(Json::Null, |c| c.as_str().into()),
                );
                if let Some((gi, g)) = r.group.as_deref().and_then(|n| self.groups.group(n)) {
                    m.insert("desired_group".into(), gi.into());
                    if let Some((pi, _)) = r.parameter.as_deref().and_then(|n| g.parameter(n)) {
                        m.insert("desired_parameter".into(), pi.into());
                    }
                }
            }
        }
        for g in self.groups.iter() {
            let Some(gs) = s.groups.get(&g.name) else {
                continue;
            };
            if g.settable() {
                let inside =
                    |groups: Option<&Vec<String>>| groups.is_some_and(|v| v.contains(&g.name));
                m.insert(
                    format!("{}__writable", g.name),
                    gs.writable.unwrap_or(false).into(),
                );
                m.insert(
                    format!("{}__pending", g.name),
                    inside(s.pending.as_ref().map(|p| &p.groups)).into(),
                );
                m.insert(
                    format!("{}__last", g.name),
                    inside(s.last.as_ref().map(|l| &l.groups)).into(),
                );
            }
            for p in &g.parameters {
                let at = format!("{}__{}", g.name, p.name);
                let (source, differs, value) = match gs.params.get(&p.name) {
                    Some(ParamState::Plain {
                        source,
                        differs,
                        value,
                    }) => (*source, *differs, value.as_ref()),
                    Some(ParamState::Sensitive { source }) => (*source, false, None),
                    None => (None, false, None),
                };
                if let Some(src) = source {
                    m.insert(format!("{at}__source"), source_name(src).into());
                }
                if !p.sensitive {
                    m.insert(format!("{at}__differs"), differs.into());
                    if let Some(v) = value {
                        m.insert(at, value_json(v));
                    }
                }
            }
        }
        Json::Object(m)
    }

    /// Writes a record under the deployment epoch, in seconds (§2.3).
    ///
    /// # Errors
    ///
    /// The packed writer's refusal: a value outside its field, a time
    /// before the epoch or past its 32 bits.
    pub fn write_record(
        &self,
        s: &State,
        epoch: i64,
    ) -> Result<packed::Record, packed::WriteRefusal> {
        self.derived.record().write(&self.fields(s), Some(epoch))
    }

    /// Reads a record under its CH's tag and the deployment epoch, refusing
    /// it for the first of §2.3's checks that applies.
    ///
    /// # Errors
    ///
    /// The [`RecordRefusal`].
    pub fn read_record(&self, tag: u8, bytes: &[u8], epoch: i64) -> Result<State, RecordRefusal> {
        let fields = self
            .derived
            .record()
            .read(tag, bytes, Some(epoch))
            .map_err(RecordRefusal::Packed)?;
        self.state_of(&fields)
    }

    /// A record's JSON reading read as a [`State`], with §2.3's checks
    /// after core §7.6's: `group`, `pending`, `last`, `desired`, then
    /// `parameter`.
    ///
    /// # Errors
    ///
    /// The first [`RecordRefusal`] that applies.
    #[allow(clippy::too_many_lines)]
    pub fn state_of(&self, fields: &Json) -> Result<State, RecordRefusal> {
        let empty = Map::new();
        let f = fields.as_object().unwrap_or(&empty);
        let has = |k: &str| f.contains_key(k);
        let get_bool = |k: &str| f.get(k).and_then(Json::as_bool);
        let get_u64 = |k: &str| f.get(k).and_then(Json::as_u64);
        // Which groups an earlier revision's writer left out: none of their
        // fields present.
        let field_names = |g: &Group| {
            let mut v = Vec::new();
            if g.settable() {
                for bit in ["writable", "pending", "last"] {
                    v.push(format!("{}__{bit}", g.name));
                }
            }
            for p in &g.parameters {
                let at = format!("{}__{}", g.name, p.name);
                v.push(format!("{at}__source"));
                if !p.sensitive {
                    v.push(format!("{at}__differs"));
                    v.push(at);
                }
            }
            v
        };
        let present: Vec<&Group> = self
            .groups
            .iter()
            .filter(|g| !g.appended || field_names(g).iter().any(|n| has(n)))
            .collect();
        for g in &present {
            if !g.appended {
                continue;
            }
            let bits = ["writable", "pending", "last"].map(|b| has(&format!("{}__{b}", g.name)));
            if g.settable() && !bits.iter().all(|b| *b) {
                return Err(RecordRefusal::Group);
            }
            for p in g.parameters.iter().filter(|p| !p.sensitive) {
                let at = format!("{}__{}", g.name, p.name);
                if has(&at) && !has(&format!("{at}__differs")) {
                    return Err(RecordRefusal::Group);
                }
            }
        }
        let bit = |g: &Group, b: &str| get_bool(&format!("{}__{b}", g.name)) == Some(true);
        let pending_groups: Vec<String> = present
            .iter()
            .filter(|g| g.settable() && bit(g, "pending"))
            .map(|g| g.name.clone())
            .collect();
        let last_groups: Vec<String> = present
            .iter()
            .filter(|g| g.settable() && bit(g, "last"))
            .map(|g| g.name.clone())
            .collect();
        let token = get_u64("pending_token");
        let deadline = f.get("pending_deadline").and_then(Json::as_i64);
        let apply_at = f.get("pending_apply_at").and_then(Json::as_i64);
        if token.is_some() != deadline.is_some()
            || ((apply_at.is_some() || !pending_groups.is_empty()) && token.is_none())
            || (token.is_some() && pending_groups.is_empty())
        {
            return Err(RecordRefusal::Pending);
        }
        let last_token = get_u64("last_token");
        if last_token.is_some() != !last_groups.is_empty() {
            return Err(RecordRefusal::Last);
        }
        let desired = if has("desired") {
            let outcome = f.get("desired").and_then(Json::as_str).and_then(outcome_of);
            let escaped = f.get("desired").is_some_and(Json::is_null);
            let coded = has("desired_code");
            let code_wrong = match outcome {
                Some(DesiredOutcome::Converged) => coded,
                Some(_) => !coded,
                None => !escaped,
            };
            let group = get_u64("desired_group");
            let parameter = get_u64("desired_parameter");
            let g = group.and_then(|i| self.groups.iter().nth(usize::try_from(i).ok()?));
            let p = match (g, parameter) {
                (Some(g), Some(i)) => usize::try_from(i).ok().and_then(|i| g.parameters.get(i)),
                _ => None,
            };
            let Some(revision) = get_u64("desired_revision").and_then(|r| u32::try_from(r).ok())
            else {
                return Err(RecordRefusal::Desired);
            };
            if code_wrong
                || (group.is_some() && !coded)
                || (parameter.is_some() && group.is_none())
                || (group.is_some() && g.is_none())
                || (parameter.is_some() && p.is_none())
            {
                return Err(RecordRefusal::Desired);
            }
            Some(Desired {
                outcome,
                observed_revision: revision,
                refusal: coded.then(|| DesiredRefused {
                    code: f
                        .get("desired_code")
                        .and_then(Json::as_str)
                        .and_then(DesiredCode::from_name),
                    group: g.map(|g| g.name.clone()),
                    parameter: p.map(|p| p.name.clone()),
                }),
            })
        } else {
            if [
                "desired_revision",
                "desired_code",
                "desired_group",
                "desired_parameter",
            ]
            .iter()
            .any(|k| has(k))
            {
                return Err(RecordRefusal::Desired);
            }
            None
        };
        let mut groups = BTreeMap::new();
        for g in &present {
            let mut params = BTreeMap::new();
            for p in &g.parameters {
                let at = format!("{}__{}", g.name, p.name);
                let source = f
                    .get(&format!("{at}__source"))
                    .and_then(Json::as_str)
                    .and_then(source_of);
                let state = if p.sensitive {
                    ParamState::Sensitive { source }
                } else {
                    let value = f.get(&at).and_then(|v| p.value_of(v));
                    let differs = get_bool(&format!("{at}__differs")) == Some(true);
                    if value.is_some() != source.is_some() || (differs && value.is_none()) {
                        return Err(RecordRefusal::Parameter);
                    }
                    ParamState::Plain {
                        source,
                        differs,
                        value,
                    }
                };
                params.insert(p.name.clone(), state);
            }
            groups.insert(
                g.name.clone(),
                GroupState {
                    writable: g.settable().then(|| bit(g, "writable")),
                    params,
                },
            );
        }
        let int = |k: &str| get_u64(k).unwrap_or_default();
        Ok(State {
            instance: u32::try_from(int("instance")).unwrap_or(u32::MAX),
            revision: u32::try_from(int("revision")).unwrap_or(u32::MAX),
            floor_s: u16::try_from(int("floor_s")).unwrap_or(u16::MAX),
            would_be: get_bool("would_be") == Some(true),
            pending: token.map(|token| Pending {
                token,
                groups: pending_groups,
                apply_at,
                deadline: deadline.unwrap_or_default(),
                applied: apply_at.is_none(),
            }),
            last: last_token.map(|token| Last {
                token,
                groups: last_groups,
            }),
            desired,
            groups,
        })
    }
}

// --- requests (§2.5, §2.10) -----------------------------------------------------------

/// A `set`'s request, `Set<T>` read (§2.5).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Set {
    /// Some parameters of the group, by name.
    pub values: BTreeMap<String, Value>,
    pub confirm_s: Option<u16>,
    pub token: Option<u64>,
    pub expected_revision: Option<u32>,
    pub idempotency_key: Option<u32>,
    pub dry_run: bool,
}

/// A control operation's request, `Control` read (§2.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Control {
    pub token: u64,
    /// For `extend` alone.
    pub confirm_s: Option<u16>,
}

/// Why a request does not parse: `invalid_request`, first (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestRefusal {
    /// Core §7.5's refusals for a query.
    Form(wire::Refusal),
    /// Core §7.6's for the request's type, its tag first.
    Record(packed::ReadRefusal),
}

impl RequestRefusal {
    /// The refusal's token.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Self::Form(r) => r.token(),
            Self::Record(r) => r.token(),
        }
    }
}

impl fmt::Display for RequestRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the request does not parse: {}", self.token())
    }
}

impl std::error::Error for RequestRefusal {}

fn tag_of(layout: &Layout) -> Tag {
    Tag::new(layout.tag()).expect("a layout's tag is four bits")
}

/// A query's payload read as a request of `layout`'s type: its JSON
/// reading.
fn read_request(layout: &Layout, payload: &[u8]) -> Result<Json, RequestRefusal> {
    let req = wire::read_query(payload).map_err(RequestRefusal::Form)?;
    layout
        .read(req.tag.get(), req.record, None)
        .map_err(RequestRefusal::Record)
}

fn write_request(layout: &Layout, fields: &Json) -> Result<Vec<u8>, packed::WriteRefusal> {
    let rec = layout.write(fields, None)?;
    Ok(Request::new(tag_of(layout), &rec.bytes).write())
}

impl Set {
    /// Its JSON reading (core §7.6).
    #[must_use]
    pub fn to_json(&self) -> Json {
        let mut m: Map<String, Json> = self
            .values
            .iter()
            .map(|(k, v)| (k.clone(), value_json(v)))
            .collect();
        if let Some(c) = self.confirm_s {
            m.insert("confirm_s".into(), c.into());
        }
        if let Some(t) = self.token {
            m.insert("token".into(), t.into());
        }
        if let Some(e) = self.expected_revision {
            m.insert("expected_revision".into(), e.into());
        }
        if let Some(k) = self.idempotency_key {
            m.insert("idempotency_key".into(), k.into());
        }
        m.insert("dry_run".into(), self.dry_run.into());
        Json::Object(m)
    }

    fn of_json(group: &Group, f: &Json) -> Self {
        let num = |k: &str| f.get(k).and_then(Json::as_u64);
        Self {
            values: group
                .parameters
                .iter()
                .filter_map(|p| Some((p.name.clone(), p.value_of(f.get(&p.name)?)?)))
                .collect(),
            confirm_s: num("confirm_s").and_then(|v| u16::try_from(v).ok()),
            token: num("token"),
            expected_revision: num("expected_revision").and_then(|v| u32::try_from(v).ok()),
            idempotency_key: num("idempotency_key").and_then(|v| u32::try_from(v).ok()),
            dry_run: f.get("dry_run").and_then(Json::as_bool) == Some(true),
        }
    }

    /// config.v1's request of this one, its token by `token`: the values
    /// as JSON, and no idempotency key, which [`Owner`] holds itself.
    fn to_v1(&self, token: impl Fn(u64) -> String) -> SetRequest {
        SetRequest {
            values: self
                .values
                .iter()
                .map(|(k, v)| (k.clone(), value_json(v)))
                .collect(),
            confirm_s: self.confirm_s.map(u64::from),
            token: self.token.map(token),
            expected_revision: self.expected_revision.map(u64::from),
            idempotency_key: None,
            dry_run: self.dry_run,
        }
    }
}

impl Control {
    /// Its JSON reading (core §7.6).
    #[must_use]
    pub fn to_json(&self) -> Json {
        let mut m = Map::new();
        m.insert("token".into(), self.token.into());
        if let Some(c) = self.confirm_s {
            m.insert("confirm_s".into(), c.into());
        }
        Json::Object(m)
    }

    fn to_v1(self, token: impl Fn(u64) -> String) -> ControlRequest {
        ControlRequest {
            token: token(self.token),
            confirm_s: self.confirm_s.map(u64::from),
            answer: None,
        }
    }
}

impl Interface {
    /// A `set/<group>`'s query payload read as its `Set<T>`, with the
    /// request's JSON reading; `None` for a group with no `set`, which no
    /// queryable answers (§2.4).
    ///
    /// # Errors
    ///
    /// The [`RequestRefusal`].
    pub fn decode_set(
        &self,
        group: &str,
        payload: &[u8],
    ) -> Option<Result<(Set, Json), RequestRefusal>> {
        let g = self.groups.settable(group)?;
        let (_, layout) = self.derived.set(group)?;
        Some(read_request(layout, payload).map(|f| (Set::of_json(g, &f), f)))
    }

    /// A `set/<group>`'s query payload: CH, then `Set<T>`'s record.
    ///
    /// # Errors
    ///
    /// The packed writer's refusal; [`packed::WriteRefusal::Unknown`] for a
    /// group with no `set`.
    pub fn encode_set(&self, group: &str, s: &Set) -> Result<Vec<u8>, packed::WriteRefusal> {
        let (_, layout) = self
            .derived
            .set(group)
            .ok_or(packed::WriteRefusal::Unknown)?;
        write_request(layout, &s.to_json())
    }

    /// A control operation's query payload read as its `Control`.
    ///
    /// # Errors
    ///
    /// The [`RequestRefusal`].
    pub fn decode_control(&self, payload: &[u8]) -> Result<Control, RequestRefusal> {
        let f = read_request(self.derived.control(), payload)?;
        let num = |k: &str| f.get(k).and_then(Json::as_u64);
        Ok(Control {
            token: num("token").unwrap_or_default(),
            confirm_s: num("confirm_s").and_then(|v| u16::try_from(v).ok()),
        })
    }

    /// A control operation's query payload.
    ///
    /// # Errors
    ///
    /// The packed writer's refusal.
    pub fn encode_control(&self, c: &Control) -> Result<Vec<u8>, packed::WriteRefusal> {
        write_request(self.derived.control(), &c.to_json())
    }

    /// A `set`'s request as a log line may carry it (§2.15): its JSON
    /// reading, each sensitive parameter's value written [`REDACTED`].
    #[must_use]
    pub fn log_set(&self, group: &str, fields: &Json) -> Json {
        let mut out = fields.clone();
        if let (Some(g), Some(m)) = (self.groups.settable(group), out.as_object_mut()) {
            for p in g.parameters.iter().filter(|p| p.sensitive) {
                if let Some(v) = m.get_mut(&p.name) {
                    *v = Json::String(REDACTED.to_owned());
                }
            }
        }
        out
    }
}

// --- answers: the value and error forms (§2.5, §2.9) ----------------------------------

/// What an operation answered, as a caller reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The record, in core §7.5's value form, under the as-of mark.
    Value { stamp: Stamp, state: Box<State> },
    /// A refusal in the error form, under the as-of mark.
    Error {
        stamp: Stamp,
        code: ErrorCode,
        cause: Option<Cause>,
        /// `app`'s `Error` detail: `device_refused` at this version.
        app: bool,
        /// For a person: the device's words, with `device_refused` alone.
        message: Option<String>,
    },
}

/// A reply: what it answered, and its payload's bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub answer: Answer,
    pub payload: Vec<u8>,
}

impl Interface {
    /// The record's value form: a reply's payload under the as-of mark.
    ///
    /// # Errors
    ///
    /// [`Interface::write_record`]'s.
    pub fn value(
        &self,
        state: &State,
        stamp: Stamp,
        epoch: i64,
    ) -> Result<Reply, packed::WriteRefusal> {
        let rec = self.write_record(state, epoch)?;
        let payload = Envelope::value(tag_of(self.derived.record()), stamp, &rec.bytes).write();
        Ok(Reply {
            answer: Answer::Value {
                stamp,
                state: Box::new(state.clone()),
            },
            payload,
        })
    }

    /// A refusal's error form (§2.5): `Error`'s tag, the as-of mark, the
    /// code and `unavailable`'s cause `config`; with `app`, an `Error`
    /// detail and the device's words, where §2.15 lets them be carried.
    #[must_use]
    pub fn error(&self, r: &Refusal, stamp: Stamp) -> Reply {
        let code = match r.code {
            Code::InvalidRequest => ErrorCode::InvalidRequest,
            Code::NotFound => ErrorCode::NotFound,
            Code::Unavailable => ErrorCode::Unavailable,
            Code::Forbidden => ErrorCode::Forbidden,
            Code::Busy => ErrorCode::Busy,
            Code::App => ErrorCode::App,
        };
        let cause = (r.code == Code::Unavailable).then_some(Cause::Config);
        let app = r.app.is_some();
        let message = r
            .app
            .as_ref()
            .and_then(|a| a.detail.as_deref())
            .map(|m| crate::envelope::cut(m).to_owned());
        let detail = app.then(|| {
            self.derived
                .error()
                .write(&json!({ "code": DEVICE_REFUSED }), None)
                .expect("device_refused is Error's")
                .bytes
        });
        let form = ErrorForm {
            tag: tag_of(self.derived.error()),
            stamp,
            code,
            cause,
            message: message.as_deref(),
            detail: detail.as_deref(),
        };
        let payload = form
            .write()
            .expect("a cause on unavailable alone, a detail on app alone");
        Reply {
            answer: Answer::Error {
                stamp,
                code,
                cause,
                app,
                message,
            },
            payload,
        }
    }
}

// --- times (§2.8) ----------------------------------------------------------------------

/// The deadline of a window `max(confirm_s, floor_s)` counted from
/// `received`, rounded up to the next whole second (§2.8).
#[must_use]
pub fn deadline(received: Time, confirm_s: u16, floor_s: u16) -> Time {
    super::deadline(received, super::window(confirm_s.into(), floor_s.into())).ceil_second()
}

/// The `apply_at` an owner writes for the instant its device states,
/// rounded up to the next whole second (§2.8).
#[must_use]
pub fn apply_at(device: Time) -> Time {
    device.ceil_second()
}

// --- validation (§2.5, §2.10, §2.13, §2.18) ---------------------------------------------

/// What a validator reads of an owner's state, as a record shows it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Facts {
    pub revision: u32,
    pub pending: Option<PendingFacts>,
    pub last: Option<LastFacts>,
    /// Whether the owner has a persisted layer.
    pub persist: bool,
    /// Whether an act is in flight.
    pub in_flight: bool,
    /// The groups the startup allowlist lets a `set` reach.
    pub writable: BTreeSet<String>,
}

/// The pending change, as a validator reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingFacts {
    pub token: u64,
    pub applied: bool,
    pub deadline_passed: bool,
}

/// The last change, as `persist` reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LastFacts {
    pub token: u64,
    pub set_sensitive: bool,
}

fn unparsed(r: RequestRefusal) -> Refusal {
    Refusal::new(Code::InvalidRequest, None, r.to_string())
}

/// Runs `f` over config.v1's facts of `facts`, its tokens spelt in
/// decimal, as a request's are.
fn with_v1<T>(facts: &Facts, f: impl FnOnce(&super::Facts<'_>) -> T) -> T {
    let pending = facts.pending.map(|p| (p.token.to_string(), p));
    let last = facts.last.map(|l| (l.token.to_string(), l));
    let v1 = super::Facts {
        revision: facts.revision.into(),
        pending: pending.as_ref().map(|(t, p)| super::PendingFacts {
            token: t,
            applied: p.applied,
            deadline_passed: p.deadline_passed,
        }),
        last_change: last.as_ref().map(|(t, l)| super::LastFacts {
            token: t,
            set_sensitive: l.set_sensitive,
        }),
        persist: facts.persist,
        in_flight: facts.in_flight,
    };
    f(&v1)
}

/// Checks a `set/<group>`'s request as §2.5 orders it: the payload's parse
/// (`invalid_request`), then config.v1's rows without rows 3, 8 and 11.
/// Rows 2 and 12 to 14 need the owner's keys, rate, deadline and device:
/// [`Owner::set`] adds them. `None` for a group with no `set`.
///
/// # Errors
///
/// The first [`Refusal`] that applies.
pub fn validate_set(
    iface: &Interface,
    facts: &Facts,
    group: &str,
    payload: &[u8],
) -> Option<Result<(), Refusal>> {
    let parsed = iface.decode_set(group, payload)?;
    Some(parsed.map_err(unparsed).and_then(|(s, _)| {
        let schema = iface.schema(&facts.writable);
        let req = s.to_v1(|t| t.to_string());
        with_v1(facts, |f| super::validate_set(&schema, f, group, &req)).map(|_| ())
    }))
}

/// Checks a control operation's request as §2.10 and §2.13 order it:
/// `persist` without a persisted layer first, then the parse, then
/// config.v1's checks.
///
/// # Errors
///
/// The first [`Refusal`] that applies.
pub fn validate_control(
    iface: &Interface,
    op: ControlOp,
    facts: &Facts,
    payload: &[u8],
) -> Result<(), Refusal> {
    if op == ControlOp::Persist && !facts.persist {
        return with_v1(facts, |f| {
            super::validate_control(op, f, &ControlRequest::default())
        });
    }
    let c = iface.decode_control(payload).map_err(unparsed)?;
    with_v1(facts, |f| {
        super::validate_control(op, f, &c.to_v1(|t| t.to_string()))
    })
}

/// A desired refusal by declared names (§2.18): a group or a parameter the
/// contract does not declare named `None`.
fn declared(
    iface: &Interface,
    code: DesiredCode,
    group: &str,
    parameter: Option<&str>,
) -> DesiredRefused {
    let g = iface.groups.group(group).map(|(_, g)| g);
    DesiredRefused {
        code: Some(code),
        group: g.map(|g| g.name.clone()),
        parameter: g
            .zip(parameter)
            .and_then(|(g, p)| g.parameter(p))
            .map(|(_, p)| p.name.clone()),
    }
}

/// Checks a desired document (§2.18): its revision within 32 bits first,
/// then config.v1's order over the groups' schema.
///
/// # Errors
///
/// The [`DesiredRefused`].
pub fn check_desired(
    iface: &Interface,
    writable: &BTreeSet<String>,
    doc: &DesiredConfig,
) -> Result<(), DesiredRefused> {
    if doc.revision > u64::from(u32::MAX) {
        return Err(DesiredRefused {
            code: Some(DesiredCode::InvalidRequest),
            group: None,
            parameter: None,
        });
    }
    super::check_desired(&iface.schema(writable), doc)
        .map_err(|r| declared(iface, r.code.into(), &r.group, r.parameter.as_deref()))
}

// --- the change event (§2.17) ------------------------------------------------------------

/// One change as it ended, `ConfigChangeEvent` of `config.v2.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeEvent {
    pub service: String,
    /// The instance number.
    pub instance: u32,
    pub revision: u64,
    pub token: u64,
    pub outcome: Outcome,
    /// In the contract's order of groups, then of parameters.
    pub edits: Vec<Edit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_source: Option<String>,
}

// --- the owner (§2.3 to §2.17) ------------------------------------------------------------

/// What an owner writes under (core §4.3): the run, each seq, a new run
/// when the seq is spent or the owner asks, and each run's first token
/// count. The seq is the instance's, shared by every state key and event
/// it writes, so the runtime implements this over the service's minter.
pub trait Mint {
    /// The run written under now.
    fn run(&self) -> Run;
    /// The last seq minted under it, `None` when none was.
    fn last_seq(&self) -> Option<Seq>;
    /// Mints the next seq for a put or an event, taking a new run first
    /// when the seq is spent.
    fn stamp(&mut self) -> Stamp;
    /// Takes a new run now: the token counter is spent (§2.6), or the
    /// owner restarts.
    fn next_run(&mut self);
    /// The token counter's first value in the run written under now, the
    /// same each time it is asked within that run: drawn at random once
    /// per run (§2.6).
    fn counter_start(&mut self) -> u16;
}

/// The as-of mark (core §5.1): the run written under, and the last seq
/// minted under it, or 0.
fn mark(mint: &dyn Mint) -> Stamp {
    Stamp {
        run: mint.run(),
        seq: mint.last_seq().unwrap_or(Seq::new(0).expect("0 is a seq")),
    }
}

/// What a `config.v2` owner starts from.
#[derive(Debug, Clone)]
pub struct Start {
    /// The owner's address, `<system>/<service>`.
    pub service: String,
    /// The startup file's and the defaults' values, as config.v1's.
    pub startup: BTreeMap<String, BTreeMap<String, (Value, Source)>>,
    /// The persisted layer.
    pub overlay: BTreeMap<String, BTreeMap<String, Value>>,
    /// The floor, from 1 to 65,535 s (§2.8).
    pub floor_s: u16,
    /// When the device applies a reach `set` after receiving it, as
    /// config.v1's; the `apply_at` written is rounded up.
    pub apply_after: Duration,
    pub persist: bool,
    /// The startup allowlist: the settable groups a `set` may reach.
    pub writable: BTreeSet<String>,
    /// The change events already published, for the rate.
    pub events: Vec<Time>,
    /// The deployment epoch, in seconds (core §8.6): what times count from.
    pub epoch: i64,
}

/// A put of the record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Put {
    pub stamp: Stamp,
    pub state: State,
    /// The put's payload: CH, form 0, the stamp, the record.
    pub payload: Vec<u8>,
}

/// A change event to publish, under its stamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    pub stamp: Stamp,
    pub event: ChangeEvent,
}

/// What a step writes, in this order: the record's puts, the reply, then
/// the change events (§2.3, §2.9, §2.17).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Output {
    pub puts: Vec<Put>,
    /// `None` when no call is answered, or for a call to a group with no
    /// `set`, which no queryable answers.
    pub reply: Option<Reply>,
    pub events: Vec<Published>,
}

/// What the owner does next: write, or have the runtime act on the device
/// and report back with [`Owner::finish`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Done(Output),
    Act(Act),
}

/// What converging a desired document asks next (§2.18).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Converge {
    /// A change is pending, or an act is in flight.
    Wait,
    /// Refused whole: the report to carry.
    Refused(DesiredReport),
    /// One `set` without a window on this group.
    Set { group: String, request: Set },
    /// Every value the document names runs.
    Converged(DesiredReport),
}

#[derive(Debug, Clone)]
struct Held {
    key: u32,
    group: String,
    payload: Vec<u8>,
    reply: Reply,
    at: Time,
}

/// A `set` whose device act is in flight.
#[derive(Debug, Clone)]
struct InFlight {
    key: Option<(u32, String, Vec<u8>)>,
}

/// One `config.v2` owner, as a pure state machine over
/// [`super::Owner`], which runs config.v1's rules unchanged with its
/// times rounded up to the second ([`super::Owner::whole_seconds`]).
///
/// It adds the wire (§2.3 to §2.17): it puts its record whenever a field
/// of it changes, and only then; it answers every call with the record in
/// the value form, or a refusal in the error form, under its as-of mark;
/// it publishes each change's event after the reply; it mints 48-bit
/// tokens, a new run before the counter passes 65,535; and it replays an
/// idempotent retry's first answer byte for byte.
#[derive(Debug, Clone)]
pub struct Owner {
    iface: Interface,
    inner: super::Owner,
    epoch: i64,
    instance: u32,
    /// The run the token counter counts in, and its next count.
    token_run: Option<Run>,
    counter: u32,
    /// config.v1's tokens of the pending and last changes, and their
    /// config.v2 numbers.
    tokens: Vec<(String, u64)>,
    keys: Vec<Held>,
    flight: Option<InFlight>,
    last_put: Option<Vec<u8>>,
}

impl Owner {
    /// Starts an owner (§2.3): its instance number is the run `mint`
    /// writes under, and its first put is its record, before its tokens.
    ///
    /// # Errors
    ///
    /// config.v1's [`StartError`]s, and a floor of 0.
    pub fn start(
        iface: Interface,
        s: Start,
        mint: &mut dyn Mint,
    ) -> Result<(Self, Output), StartError> {
        let instance = mint.run().value();
        let inner = super::Owner::start(super::Start {
            service: s.service,
            instance: instance.to_string(),
            schema: iface.schema(&s.writable),
            startup: s.startup,
            overlay: s.overlay,
            floor_s: s.floor_s.into(),
            apply_after: s.apply_after,
            persist: s.persist,
            events: s.events,
        })?
        .whole_seconds();
        let mut owner = Self {
            iface,
            inner,
            epoch: s.epoch,
            instance,
            token_run: None,
            counter: 0,
            tokens: Vec::new(),
            keys: Vec::new(),
            flight: None,
            last_put: None,
        };
        let out = owner.settle(super::Done::default(), false, mint);
        Ok((owner, out))
    }

    /// The owner after a restart (§2.12): a new instance under the next
    /// run `mint` takes, its startup values and persisted layer, nothing
    /// pending, no key held.
    #[must_use]
    pub fn restart(&self, mint: &mut dyn Mint) -> (Self, Output) {
        mint.next_run();
        let instance = mint.run().value();
        let mut owner = Self {
            iface: self.iface.clone(),
            inner: self.inner.restart(instance.to_string()),
            epoch: self.epoch,
            instance,
            token_run: None,
            counter: 0,
            tokens: Vec::new(),
            keys: Vec::new(),
            flight: None,
            last_put: None,
        };
        let out = owner.settle(super::Done::default(), false, mint);
        (owner, out)
    }

    /// The config interface.
    #[must_use]
    pub fn interface(&self) -> &Interface {
        &self.iface
    }

    /// config.v1's owner this one runs: its startup values, its due
    /// instants and its published events.
    #[must_use]
    pub fn inner(&self) -> &super::Owner {
        &self.inner
    }

    /// The instance number.
    #[must_use]
    pub fn instance(&self) -> u32 {
        self.instance
    }

    /// The deployment epoch its times count from, in seconds.
    #[must_use]
    pub fn epoch(&self) -> i64 {
        self.epoch
    }

    /// Whether an act is in flight.
    #[must_use]
    pub fn in_flight(&self) -> bool {
        self.inner.in_flight()
    }

    /// The next instant [`Owner::due`] has something to do.
    #[must_use]
    pub fn next_due(&self) -> Option<Time> {
        self.inner.next_due()
    }

    /// The record as it stands.
    #[must_use]
    pub fn state(&self) -> State {
        self.render(&self.inner.view(), false)
    }

    fn token_of(&self, v1: &str) -> u64 {
        self.tokens
            .iter()
            .find(|(t, _)| t == v1)
            .map(|(_, n)| *n)
            .expect("every config.v1 token is numbered when it appears")
    }

    /// config.v1's spelling of a request's token: the change's, or one no
    /// change of config.v1 has, which matches none.
    fn v1_token(&self, token: u64) -> String {
        self.tokens
            .iter()
            .find(|(_, n)| *n == token)
            .map_or_else(|| format!("?{token}"), |(t, _)| t.clone())
    }

    /// Mints a token (§2.6): the run written under, then the counter, a
    /// new run before it would pass 65,535.
    fn mint_token(&mut self, mint: &mut dyn Mint) -> u64 {
        if self.token_run != Some(mint.run()) {
            self.token_run = Some(mint.run());
            self.counter = mint.counter_start().into();
        }
        if self.counter > COUNTER_MAX {
            mint.next_run();
            self.token_run = Some(mint.run());
            self.counter = mint.counter_start().into();
        }
        let token = (u64::from(mint.run().value()) << 16) | u64::from(self.counter);
        self.counter += 1;
        token
    }

    fn in_contract_order(&self, groups: &[String]) -> Vec<String> {
        self.iface
            .groups
            .iter()
            .filter(|g| groups.contains(&g.name))
            .map(|g| g.name.clone())
            .collect()
    }

    fn render(&self, view: &ConfigView, would_be: bool) -> State {
        let apply_at = self.inner.apply_at().map(Time::millis);
        State {
            instance: self.instance,
            revision: u32::try_from(view.revision).unwrap_or(u32::MAX),
            floor_s: u16::try_from(view.confirm_floor_s).unwrap_or(u16::MAX),
            would_be,
            pending: view.pending.as_ref().map(|p| Pending {
                token: self.token_of(&p.token),
                groups: self.in_contract_order(&p.groups),
                apply_at,
                deadline: self.inner.deadline().map_or(0, Time::millis),
                applied: apply_at.is_none(),
            }),
            last: view.last_change.as_ref().map(|l| Last {
                token: self.token_of(&l.token),
                groups: self.in_contract_order(&l.groups),
            }),
            desired: view.desired.as_ref().map(|d| Desired {
                outcome: Some(d.outcome),
                observed_revision: u32::try_from(d.observed_revision).unwrap_or(u32::MAX),
                refusal: d.refusal.as_ref().map(|r| {
                    declared(&self.iface, r.code.into(), &r.group, r.parameter.as_deref())
                }),
            }),
            groups: view
                .groups
                .iter()
                .map(|g| {
                    let settable = self.iface.groups.settable(&g.name).is_some();
                    let params = g
                        .parameters
                        .iter()
                        .map(|p| {
                            let s = if p.sensitive {
                                ParamState::Sensitive { source: p.source }
                            } else {
                                ParamState::Plain {
                                    source: p.source,
                                    differs: p.startup.is_some(),
                                    value: p.value.clone(),
                                }
                            };
                            (p.name.clone(), s)
                        })
                        .collect();
                    (
                        g.name.clone(),
                        GroupState {
                            writable: settable.then_some(g.writable),
                            params,
                        },
                    )
                })
                .collect(),
        }
    }

    fn event(&self, e: &super::ConfigChangeEvent) -> ChangeEvent {
        let order = |ed: &Edit| {
            self.iface.groups.group(&ed.group).map(|(gi, g)| {
                (
                    gi,
                    g.parameter(&ed.parameter).map_or(usize::MAX, |(pi, _)| pi),
                )
            })
        };
        let mut edits = e.edits.clone();
        edits.sort_by_key(|ed| order(ed).unwrap_or((usize::MAX, usize::MAX)));
        ChangeEvent {
            service: e.service.clone(),
            instance: self.instance,
            revision: e.revision,
            token: self.token_of(&e.token),
            outcome: e.outcome,
            edits,
            actor: e.actor.clone(),
            request_id: e.request_id.clone(),
            claimed_source: e.claimed_source.clone(),
        }
    }

    fn record_bytes(&self, state: &State) -> Vec<u8> {
        self.iface
            .write_record(state, self.epoch)
            .expect("an owner's record is its Record type's: its times within 32 bits of the epoch")
            .bytes
    }

    /// Ends a step config.v1 answered: numbers its new tokens, puts the
    /// record when a field of it changed, then the reply under the as-of
    /// mark, then the events, each under its own seq (§2.3, §2.9, §2.17).
    fn settle(&mut self, done: super::Done, dry_run: bool, mint: &mut dyn Mint) -> Output {
        let view = self.inner.view();
        let seen: Vec<String> = view
            .pending
            .iter()
            .map(|p| p.token.clone())
            .chain(view.last_change.iter().map(|l| l.token.clone()))
            .chain(done.events.iter().map(|e| e.token.clone()))
            .collect();
        for t in seen {
            if !self.tokens.iter().any(|(v, _)| *v == t) {
                let n = self.mint_token(mint);
                self.tokens.push((t, n));
            }
        }
        let events: Vec<ChangeEvent> = done.events.iter().map(|e| self.event(e)).collect();
        let mut out = Output::default();
        let state = self.render(&view, false);
        let bytes = self.record_bytes(&state);
        if self.last_put.as_deref() != Some(bytes.as_slice()) {
            let stamp = mint.stamp();
            let payload =
                Envelope::mutation(tag_of(self.iface.derived.record()), stamp, &bytes).write();
            out.puts.push(Put {
                stamp,
                state: state.clone(),
                payload,
            });
            self.last_put = Some(bytes);
        }
        let keep: Vec<String> = view
            .pending
            .iter()
            .map(|p| p.token.clone())
            .chain(view.last_change.iter().map(|l| l.token.clone()))
            .collect();
        self.tokens.retain(|(t, _)| keep.contains(t));
        if let Some(r) = done.reply {
            let stamp = mark(mint);
            out.reply = Some(match r {
                Ok(super::Reply::Set(SetReply::Applied(v))) if dry_run => {
                    let would_be = self.render(&v, true);
                    self.iface
                        .value(&would_be, stamp, self.epoch)
                        .expect("a would-be record is its Record type's")
                }
                Ok(_) => self
                    .iface
                    .value(&state, stamp, self.epoch)
                    .expect("the record was just written"),
                Err(e) => self.iface.error(&e, stamp),
            });
        }
        for event in events {
            out.events.push(Published {
                stamp: mint.stamp(),
                event,
            });
        }
        out
    }

    fn refuse(&self, r: &Refusal, mint: &dyn Mint) -> Output {
        Output {
            reply: Some(self.iface.error(r, mark(mint))),
            ..Output::default()
        }
    }

    fn step(&mut self, step: super::Step, dry_run: bool, mint: &mut dyn Mint) -> Step {
        match step {
            super::Step::Act(a) => Step::Act(a),
            super::Step::Done(d) => Step::Done(self.settle(d, dry_run, mint)),
        }
    }

    fn hold(&mut self, now: Time, key: Option<(u32, String, Vec<u8>)>, out: &Output) {
        if let (Some((key, group, payload)), Some(reply)) = (key, &out.reply)
            && !self.keys.iter().any(|h| h.key == key)
        {
            self.keys.push(Held {
                key,
                group,
                payload,
                reply: reply.clone(),
                at: now,
            });
        }
    }

    /// A call to `set/<group>` (§2.5, §2.9): the parse, an act in flight,
    /// a key held, then config.v1's rows and its owner; answered now, or an
    /// [`Act::Apply`] for a hot group, answered by [`Owner::finish`]. A
    /// group with no `set` reaches no queryable: nothing answers.
    pub fn set(
        &mut self,
        now: Time,
        group: &str,
        payload: &[u8],
        call: &Call,
        mint: &mut dyn Mint,
    ) -> Step {
        let Some(parsed) = self.iface.decode_set(group, payload) else {
            return Step::Done(Output::default());
        };
        let set = match parsed {
            Ok((s, _)) => s,
            Err(r) => return Step::Done(self.refuse(&unparsed(r), mint)),
        };
        if self.inner.in_flight() {
            return Step::Done(self.refuse(&super::in_flight(), mint));
        }
        self.keys.retain(|h| now.within(h.at, super::KEY_HELD));
        if let Some(k) = set.idempotency_key
            && let Some(h) = self.keys.iter().find(|h| h.key == k)
        {
            if h.group == group && h.payload == payload {
                return Step::Done(Output {
                    reply: Some(h.reply.clone()),
                    ..Output::default()
                });
            }
            return Step::Done(self.refuse(
                &Refusal::new(
                    Code::InvalidRequest,
                    Some(Field::member("idempotency_key")),
                    "this idempotency key was used for another request within 300 s",
                ),
                mint,
            ));
        }
        let key = set
            .idempotency_key
            .map(|k| (k, group.to_owned(), payload.to_vec()));
        let req = set.to_v1(|t| self.v1_token(t));
        match self.inner.set(now, group, &req, call) {
            super::Step::Act(a) => {
                self.flight = Some(InFlight { key });
                Step::Act(a)
            }
            super::Step::Done(d) => {
                let out = self.settle(d, set.dry_run, mint);
                self.hold(now, key, &out);
                Step::Done(out)
            }
        }
    }

    fn control(
        &mut self,
        op: ControlOp,
        now: Time,
        payload: &[u8],
        call: &Call,
        mint: &mut dyn Mint,
    ) -> Step {
        let c = match self.iface.decode_control(payload) {
            Ok(c) => c,
            Err(r) => return Step::Done(self.refuse(&unparsed(r), mint)),
        };
        let req = c.to_v1(|t| self.v1_token(t));
        let step = match op {
            ControlOp::Confirm => self.inner.confirm(now, &req, call),
            ControlOp::Cancel => self.inner.cancel(now, &req, call),
            ControlOp::Extend => self.inner.extend(now, &req, call),
            ControlOp::Persist => unreachable!("persist is Owner::persist"),
        };
        if matches!(step, super::Step::Act(_)) {
            self.flight = Some(InFlight { key: None });
        }
        self.step(step, false, mint)
    }

    /// `confirm` (§2.10): answered now.
    pub fn confirm(&mut self, now: Time, payload: &[u8], call: &Call, mint: &mut dyn Mint) -> Step {
        self.control(ControlOp::Confirm, now, payload, call, mint)
    }

    /// `cancel` (§2.10): an [`Act::Undo`] when something of the change is
    /// applied, else answered now.
    pub fn cancel(&mut self, now: Time, payload: &[u8], call: &Call, mint: &mut dyn Mint) -> Step {
        self.control(ControlOp::Cancel, now, payload, call, mint)
    }

    /// `extend` (§2.10): answered now, the deadline rounded up.
    pub fn extend(&mut self, now: Time, payload: &[u8], call: &Call, mint: &mut dyn Mint) -> Step {
        self.control(ControlOp::Extend, now, payload, call, mint)
    }

    /// Admits a `persist` (§2.13): without a persisted layer `unavailable`
    /// before the request is read, then the parse, then config.v1's
    /// checks. The runtime writes the plan's entries, then calls
    /// [`Owner::persisted`].
    ///
    /// # Errors
    ///
    /// The refusal's [`Output`].
    pub fn persist(
        &mut self,
        now: Time,
        payload: &[u8],
        mint: &mut dyn Mint,
    ) -> Result<PersistPlan, Output> {
        let req = if self.inner.facts(now).persist {
            match self.iface.decode_control(payload) {
                Ok(c) => c.to_v1(|t| self.v1_token(t)),
                Err(r) => return Err(self.refuse(&unparsed(r), mint)),
            }
        } else {
            ControlRequest::default()
        };
        self.inner
            .persist_plan(now, &req)
            .map_err(|r| self.refuse(&r, mint))
    }

    /// Commits a `persist` whose entries the runtime wrote: the record put
    /// when a source moved, then the reply.
    pub fn persisted(&mut self, plan: PersistPlan, mint: &mut dyn Mint) -> Output {
        let done = self.inner.persisted(plan);
        self.settle(done, false, mint)
    }

    /// The act owed at `now`, if any (§2.9, §2.11): the deadline's undo, or
    /// the earliest reach apply whose `apply_at` has come.
    pub fn due(&mut self, now: Time, mint: &mut dyn Mint) -> Option<Step> {
        let step = self.inner.due(now)?;
        Some(self.step(step, false, mint))
    }

    /// Reports the device's result for the act in flight, and answers what
    /// it owed.
    pub fn finish(&mut self, now: Time, result: DeviceResult, mint: &mut dyn Mint) -> Step {
        let flight = self.flight.take();
        match self.inner.finish(now, result) {
            super::Step::Act(a) => {
                self.flight = flight;
                Step::Act(a)
            }
            super::Step::Done(d) => {
                let out = self.settle(d, false, mint);
                self.hold(now, flight.and_then(|f| f.key), &out);
                Step::Done(out)
            }
        }
    }

    /// Sets the desired report the record carries (§2.18), putting the
    /// record when it changed.
    pub fn report_desired(&mut self, report: Option<DesiredReport>, mint: &mut dyn Mint) -> Output {
        self.inner.report_desired(report);
        self.settle(super::Done::default(), false, mint)
    }

    /// What converging `doc` asks next (§2.18): config.v1's, a revision
    /// above 32 bits refused whole, naming no group, before any group is
    /// read.
    #[must_use]
    pub fn converge(&self, doc: &DesiredConfig) -> Converge {
        let next = self.inner.converge(doc);
        if doc.revision > u64::from(u32::MAX) && next != super::Converge::Wait {
            return Converge::Refused(DesiredReport {
                observed_revision: doc.revision,
                outcome: DesiredOutcome::Refused,
                refusal: Some(super::DesiredRefusal {
                    code: super::DesiredCode::InvalidRequest,
                    group: String::new(),
                    parameter: None,
                }),
            });
        }
        match next {
            super::Converge::Wait => Converge::Wait,
            super::Converge::Refused(r) => Converge::Refused(r),
            super::Converge::Converged(r) => Converge::Converged(r),
            super::Converge::Set { group, request } => {
                let g = self.iface.groups.settable(&group);
                Converge::Set {
                    request: Set {
                        values: g.map_or_else(BTreeMap::new, |g| {
                            g.parameters
                                .iter()
                                .filter_map(|p| {
                                    Some((
                                        p.name.clone(),
                                        p.value_of(request.values.get(&p.name)?)?,
                                    ))
                                })
                                .collect()
                        }),
                        ..Set::default()
                    },
                    group,
                }
            }
        }
    }

    /// The change events of the last hour, for the next instance.
    #[must_use]
    pub fn published(&self, now: Time) -> Vec<Time> {
        self.inner.published(now)
    }
}

// --- the contract's lint (§2.2, §3) -------------------------------------------------------

/// A finding of the contract's lint (§2.2), an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lint {
    /// No resource `config` with `config.groups`.
    GroupsMissing,
    /// The declaration is invalid.
    GroupsInvalid,
    /// A resource §3 requires is absent.
    ResourceMissing,
    /// A resource is not of §3's kind, options or types.
    ResourceShape,
    /// A type is not its derivation.
    ConfigDerivation,
    /// `config` has no `freshness.ttl_s` above 0, or the contract does not
    /// use `freshness.v2`.
    Freshness,
    /// A `set` resource of no settable group.
    SetNotSettable,
    /// `config.groups` on a resource other than `config`.
    GroupsPlace,
}

impl Lint {
    /// The fixtures' token.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Self::GroupsMissing => "groups_missing",
            Self::GroupsInvalid => "groups_invalid",
            Self::ResourceMissing => "resource_missing",
            Self::ResourceShape => "resource_shape",
            Self::ConfigDerivation => "config_derivation",
            Self::Freshness => "freshness",
            Self::SetNotSettable => "set_not_settable",
            Self::GroupsPlace => "groups_place",
        }
    }
}

/// One finding, at a resource's template.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Finding {
    pub at: String,
    pub reason: Lint,
}

/// A type as the lint reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeRef {
    /// A packed type: its file's stem where known, and its name.
    Packed { stem: Option<String>, name: String },
    /// A JSON Schema type: its name and its artifact id.
    JsonSchema { name: String, schema: String },
    /// Any other type.
    Other,
}

/// A resource's body as the lint reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResBody {
    /// A stream, state or event.
    Data {
        type_: TypeRef,
        rate: Option<Rate>,
        retention_s: Option<u64>,
    },
    Operation {
        request: TypeRef,
        response: TypeRef,
        error: Option<TypeRef>,
        fanout: Fanout,
        idempotent: bool,
        serving: Serving,
        replies: Replies,
    },
}

/// One resource, resolved (core §9.3), as the lint reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Res {
    pub template: String,
    pub kind: Kind,
    /// The template's parameters.
    pub params: Vec<String>,
    pub optional: bool,
    pub annotations: BTreeMap<String, Json>,
    pub body: ResBody,
}

/// A contract as the lint reads it: its `uses`, its resources, and its
/// packed files, `(stem, document)`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Resources {
    pub uses: Vec<String>,
    pub resources: Vec<Res>,
    pub packed: Vec<(String, Json)>,
}

impl Resources {
    /// A loaded contract as the lint reads it.
    #[must_use]
    pub fn of(c: &Contract) -> Self {
        let tref = |t: &TypeId| match t {
            TypeId::Packed { name, schema } => TypeRef::Packed {
                stem: c.artifacts.get(schema).map(|a| a.name.clone()),
                name: name.clone(),
            },
            TypeId::JsonSchema { name, schema } => TypeRef::JsonSchema {
                name: name.clone(),
                schema: schema.clone(),
            },
            _ => TypeRef::Other,
        };
        Self {
            uses: c.uses.iter().map(ToString::to_string).collect(),
            resources: c
                .resources
                .iter()
                .map(|r| Res {
                    template: r.template.as_str().to_owned(),
                    kind: r.kind,
                    params: r.params.keys().cloned().collect(),
                    optional: r.optional,
                    annotations: r.annotations.clone(),
                    body: match &r.body {
                        ContractBody::Data(d) => ResBody::Data {
                            type_: tref(&d.type_),
                            rate: d.rate,
                            retention_s: d.retention_s,
                        },
                        ContractBody::Operation(o) => ResBody::Operation {
                            request: tref(&o.request),
                            response: tref(&o.response),
                            error: o.error.as_ref().map(tref),
                            fanout: o.fanout,
                            idempotent: o.idempotent,
                            serving: o.serving,
                            replies: o.replies,
                        },
                    },
                })
                .collect(),
            packed: packed_files(c)
                .into_iter()
                .map(|(s, d)| (s.to_owned(), d.clone()))
                .collect(),
        }
    }

    fn resource(&self, template: &str) -> Option<&Res> {
        self.resources.iter().find(|r| r.template == template)
    }

    fn files(&self) -> Vec<(&str, &Json)> {
        self.packed.iter().map(|(s, d)| (s.as_str(), d)).collect()
    }

    /// A packed type's layout: `None` for one no listed file defines once,
    /// or whose declaration does not read.
    fn layout(&self, t: &TypeRef) -> Option<Layout> {
        let TypeRef::Packed { stem, name } = t else {
            return None;
        };
        let reference = match stem {
            Some(s) => format!("packed:{s}#{name}"),
            None => format!("packed:{name}"),
        };
        let files = self.files();
        let r = resolve(&reference, &files)?;
        packed::layout_in(r.doc, r.name).ok()
    }
}

fn is_packed(t: &TypeRef) -> bool {
    matches!(t, TypeRef::Packed { .. })
}

/// The lint of a contract's resources (§2.2): `groups_missing` or
/// `groups_invalid` alone; else, for `config`, each `set/<group>` in the
/// groups' order, `confirm`, `cancel`, `extend`, `persist` and `change`,
/// `resource_missing`, else `resource_shape`, else `config_derivation`,
/// with `freshness` after `config`'s; then `set_not_settable` and
/// `groups_place`, each in the byte order of the templates.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn lint(r: &Resources) -> Vec<Finding> {
    let find = |at: &str, reason| Finding {
        at: at.to_owned(),
        reason,
    };
    let config = r.resource(CONFIG);
    let Some(ann) = config.and_then(|c| c.annotations.get(GROUPS)) else {
        return vec![find(CONFIG, Lint::GroupsMissing)];
    };
    let Ok(iface) = Interface::read(ann, &r.files()) else {
        return vec![find(CONFIG, Lint::GroupsInvalid)];
    };
    let d = &iface.derived;
    let same = |t: &TypeRef, want: &Layout| r.layout(t).is_some_and(|l| l.items == want.items);
    let mut out = Vec::new();
    // `config`.
    if let Some(c) = config {
        let shaped = c.kind == Kind::State
            && !c.optional
            && c.params.is_empty()
            && matches!(&c.body, ResBody::Data { type_, .. } if is_packed(type_));
        if !shaped {
            out.push(find(CONFIG, Lint::ResourceShape));
        } else if !matches!(&c.body, ResBody::Data { type_, .. } if same(type_, d.record())) {
            out.push(find(CONFIG, Lint::ConfigDerivation));
        }
        let ttl = c
            .annotations
            .get(crate::freshness::TTL_S)
            .and_then(Json::as_f64)
            .is_some_and(|t| t > 0.0);
        if !ttl || !r.uses.iter().any(|u| u == crate::freshness::PROFILE_V2) {
            out.push(find(CONFIG, Lint::Freshness));
        }
    }
    // The operations.
    let sets: Vec<(String, &Layout)> = iface
        .groups
        .iter()
        .filter(|g| g.settable())
        .filter_map(|g| Some((format!("{}{}", SET_PREFIX, g.name), d.set(&g.name)?.1)))
        .collect();
    let ops = sets
        .iter()
        .map(|(t, l)| (t.as_str(), *l))
        .chain(CONTROLS.iter().map(|t| (*t, d.control())));
    for (template, request) in ops {
        let Some(res) = r.resource(template) else {
            out.push(find(template, Lint::ResourceMissing));
            continue;
        };
        let ResBody::Operation {
            request: req,
            response,
            error,
            fanout,
            idempotent,
            serving,
            replies,
        } = &res.body
        else {
            out.push(find(template, Lint::ResourceShape));
            continue;
        };
        let shaped = res.kind == Kind::Operation
            && res.params.is_empty()
            && res.optional == (template == super::PERSIST)
            && *fanout == Fanout::Forbidden
            && !idempotent
            && *serving == Serving::Exclusive
            && *replies == Replies::One
            && is_packed(req)
            && is_packed(response)
            && error.as_ref().is_some_and(is_packed);
        if !shaped {
            out.push(find(template, Lint::ResourceShape));
        } else if !(same(req, request)
            && same(response, d.record())
            && error.as_ref().is_some_and(|e| same(e, d.error())))
        {
            out.push(find(template, Lint::ConfigDerivation));
        }
    }
    // `change`.
    match r.resource(CHANGE) {
        None => out.push(find(CHANGE, Lint::ResourceMissing)),
        Some(c) => {
            let shaped = c.kind == Kind::Event
                && !c.optional
                && c.params.is_empty()
                && matches!(&c.body, ResBody::Data { type_: TypeRef::JsonSchema { name, schema }, rate, retention_s }
                    if name == EVENT_TYPE
                        && schema == STANDARD_SCHEMA
                        && *rate == Some(EVENT_RATE)
                        && *retention_s == Some(EVENT_RETENTION_S));
            if !shaped {
                out.push(find(CHANGE, Lint::ResourceShape));
            }
        }
    }
    let mut templates: Vec<&str> = r.resources.iter().map(|x| x.template.as_str()).collect();
    templates.sort_unstable();
    templates.dedup();
    for t in &templates {
        let set = *t == "set" || t.starts_with(SET_PREFIX);
        if set && !sets.iter().any(|(s, _)| s == t) {
            out.push(find(t, Lint::SetNotSettable));
        }
    }
    for t in &templates {
        if *t != CONFIG
            && r.resources
                .iter()
                .any(|x| x.template == *t && x.annotations.contains_key(GROUPS))
        {
            out.push(find(t, Lint::GroupsPlace));
        }
    }
    out
}

/// The `set` operations' template prefix, `set/<group>`.
pub const SET_PREFIX: &str = "set/";

/// [`lint`] over a loaded contract that uses `config.v2`; none for any
/// other contract.
#[must_use]
pub fn lint_contract(c: &Contract) -> Vec<Finding> {
    if c.uses.iter().any(|u| u.to_string() == PROFILE) {
        lint(&Resources::of(c))
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_have_the_group_names_form() {
        for ok in ["duty", "fraction_ppm", "p0", "a1_b2"] {
            assert!(is_name(ok), "{ok}");
        }
        for bad in ["", "Duty", "tx__queue", "queue_", "_x", "1a", "a-b"] {
            assert!(!is_name(bad), "{bad}");
        }
    }

    #[test]
    fn times_round_up_to_the_second() {
        let t = |ms| Time::from_millis(ms);
        assert_eq!(t(1000).ceil_second(), t(1000));
        assert_eq!(t(1001).ceil_second(), t(2000));
        assert_eq!(t(-1).ceil_second(), t(0));
        assert_eq!(deadline(t(300), 5, 10), t(11_000));
        assert_eq!(apply_at(t(1300)), t(2000));
    }
}
