//! `telemetry.v1`: what a metric is, and how a reader reads one
//! (`spec/profiles/telemetry/v1.md`, #819).
//!
//! The profile's session-free half: the declaration read from a resolved
//! resource (§2.2, §2.4), one sample's conformance and the member's kind and
//! unit (§2.5, §2.6), and the continuity and rate of a pair of samples
//! (§2.9–§2.11).
//!
//! ```text
//! declaration(kind, annotations, type, attachment) ─▶ declared │ invalid [reasons] │ ignored │ not this profile's
//! check(declaration, payload)                      ─▶ conforms │ does_not [reasons] │ unobservable │ not_asked, + kind, unit, reading
//! pair(earlier, later, presence)                   ─▶ same_epoch │ reset │ decreased │ unobservable │ not_asked, + epoch, rate
//! ```
//!
//! There is no I/O and no clock here. A reader decodes a payload into a
//! [`Datum`] (JSON's data model, or CBOR's with its byte strings and its
//! non-finite floats), measures what it saw of presence and when each
//! sample arrived, and hands them over as values. The owner's in-band
//! epoch is the runtime's.
//!
//! [`wire`] is the standard schema's types as an owner puts them: the
//! Point, its tagged value and the histogram, with serde (#819, TB).

use std::collections::BTreeMap;
use std::fmt;

use serde_json::Value;

use crate::authoring::Kind;
use crate::contract::{Body, Contract, Resource};
use crate::schema::{Artifact, ArtifactData, TypeId};

pub mod wire;

/// The profile's id, as a contract lists it in `uses`.
pub const PROFILE: &str = "telemetry.v1";

/// The profile's name, as an annotation key's prefix carries it.
pub const NAME: &str = "telemetry";

/// `telemetry.kind` (§4).
pub const KIND: &str = "telemetry.kind";
/// `telemetry.unit` (§4).
pub const UNIT: &str = "telemetry.unit";
/// `telemetry.buckets` (§4).
pub const BUCKETS: &str = "telemetry.buckets";
/// `telemetry.semantic` (§4).
pub const SEMANTIC: &str = "telemetry.semantic";

/// The published vocabulary (§4): the keys under `telemetry.`, which W105
/// reads in place of core Appendix D's interim row (core 0.35).
pub const VOCABULARY: &[&str] = &["kind", "unit", "buckets", "semantic"];

/// The standard schema's stem (§3): `telemetry.v1.json`.
pub const SCHEMA_STEM: &str = "telemetry.v1";
/// The Point's type name (§2.3).
pub const POINT: &str = "telemetry.v1.Point";
/// The tagged value's type name (§2.3).
pub const VALUE: &str = "telemetry.v1.Value";
/// The histogram's type name (§2.3, §2.8).
pub const HISTOGRAM: &str = "telemetry.v1.Histogram";
/// The in-band epoch's type name (§2.3, §2.9).
pub const EPOCH: &str = "telemetry.v1.Epoch";

/// The standard schema's artifact ids the profile publishes (§3), oldest
/// first: ZenSight's file before the profile, then text 0.1's. A type of
/// one of these artifacts is the profile's (§2.3).
pub const PUBLISHED: &[&str] = &[
    "sha256:3d6276dd302873ca5938013a44d460267971123fd25500f189e30be718f9dc1e",
    "sha256:715ce537b2150beb3863f0e5cd356281357f7ec56d93625ee64e14f0045629e9",
];

/// The longest unit and the longest in-band epoch, in characters (§2.2,
/// §2.6).
pub const MAX_UNIT: usize = 64;
pub const MAX_EPOCH: usize = 64;

/// Whether `id` is one of the standard schema's published artifact ids.
#[must_use]
pub fn is_published(id: &str) -> bool {
    PUBLISHED.contains(&id)
}

/// A metric's kind (§2.2), one spelling on the annotation and on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetricKind {
    Counter,
    Gauge,
    Histogram,
    Text,
    Boolean,
    Binary,
}

impl MetricKind {
    pub const ALL: [Self; 6] = [
        Self::Counter,
        Self::Gauge,
        Self::Histogram,
        Self::Text,
        Self::Boolean,
        Self::Binary,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Counter => "counter",
            Self::Gauge => "gauge",
            Self::Histogram => "histogram",
            Self::Text => "text",
            Self::Boolean => "boolean",
            Self::Binary => "binary",
        }
    }

    /// The kind spelled `s`, compared byte for byte.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// Counters and histograms have an epoch, a continuity and, for a
    /// counter, a rate (§2.7–§2.11).
    #[must_use]
    pub fn is_cumulative(self) -> bool {
        matches!(self, Self::Counter | Self::Histogram)
    }
}

impl fmt::Display for MetricKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A presentation hint (§2.2), never a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Semantic {
    Temperature,
    Power,
    Bytes,
    Duration,
    Ratio,
    Count,
    Identity,
    State,
}

impl Semantic {
    pub const ALL: [Self; 8] = [
        Self::Temperature,
        Self::Power,
        Self::Bytes,
        Self::Duration,
        Self::Ratio,
        Self::Count,
        Self::Identity,
        Self::State,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Temperature => "temperature",
            Self::Power => "power",
            Self::Bytes => "bytes",
            Self::Duration => "duration",
            Self::Ratio => "ratio",
            Self::Count => "count",
            Self::Identity => "identity",
            Self::State => "state",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// A unit (§2.2): 1 to 64 characters, each printable ASCII from 0x21 to
/// 0x7E. Never parsed or converted.
#[must_use]
pub fn is_unit(s: &str) -> bool {
    (1..=MAX_UNIT).contains(&s.len()) && s.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

/// An in-band epoch (§2.6, §2.9): a string of 1 to 64 characters, counted
/// as Unicode code points, as JSON Schema's `minLength`/`maxLength` count.
#[must_use]
pub fn is_epoch(s: &str) -> bool {
    (1..=MAX_EPOCH).contains(&s.chars().count())
}

// ---------------------------------------------------------------------------
// The declaration (§2.2, §2.4)
// ---------------------------------------------------------------------------

/// A resource's type, as a reader of the declaration sees it (§2.4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TypeView<'a> {
    /// A JSON Schema type: its name, its artifact's id, and its root
    /// schema, `$ref`s followed (core §9.8), when the reader resolved it.
    Json {
        name: &'a str,
        schema: &'a str,
        root: Option<&'a Value>,
    },
    /// A raw type, of any media type.
    Raw,
    /// A protobuf message.
    Protobuf,
}

impl TypeView<'_> {
    /// Whether this is the profile's type `name`, by its artifact's id
    /// (§2.3).
    #[must_use]
    pub fn is_profiles(&self, type_name: &str) -> bool {
        matches!(self, Self::Json { name, schema, .. } if *name == type_name && is_published(schema))
    }
}

/// What a declaration's samples are (§2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Carrier {
    /// The profile's Point, which says its own kind.
    Point,
    /// A bare value of the declared kind.
    Bare,
}

impl Carrier {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Point => "point",
            Self::Bare => "bare",
        }
    }
}

/// Where a member's in-band epoch rides (§2.4, §2.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpochIn {
    /// The Point's `epoch`.
    Point,
    /// The sample's attachment, typed `telemetry.v1.Epoch`.
    Attachment,
}

impl EpochIn {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Point => "point",
            Self::Attachment => "attachment",
        }
    }
}

/// A valid declaration (§2.2).
#[derive(Debug, Clone, PartialEq)]
pub struct Declared {
    pub carrier: Carrier,
    /// The declared kind; `None` for a Point that says its own.
    pub kind: Option<MetricKind>,
    pub unit: Option<String>,
    /// The bounds, as doubles (§2.2), exactly when the kind is `histogram`.
    pub buckets: Option<Vec<f64>>,
    pub semantic: Option<Semantic>,
    /// Where an in-band epoch rides, if anywhere.
    pub epoch: Option<EpochIn>,
}

/// Why a declaration is invalid (§2.2), in the order a reader checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Invalid {
    KindValue,
    UnitValue,
    BucketsValue,
    SemanticValue,
    KindMissing,
    BucketsWithoutHistogram,
    HistogramWithoutBuckets,
    NotACarrier,
}

impl Invalid {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::KindValue => "kind_value",
            Self::UnitValue => "unit_value",
            Self::BucketsValue => "buckets_value",
            Self::SemanticValue => "semantic_value",
            Self::KindMissing => "kind_missing",
            Self::BucketsWithoutHistogram => "buckets_without_histogram",
            Self::HistogramWithoutBuckets => "histogram_without_buckets",
            Self::NotACarrier => "not_a_carrier",
        }
    }

    /// The reason in words, for a linter's or a tool's message.
    #[must_use]
    pub fn says(self) -> &'static str {
        match self {
            Self::KindValue => "telemetry.kind is not one of the six kinds",
            Self::UnitValue => {
                "telemetry.unit is not 1 to 64 printable ASCII characters (0x21 to 0x7E)"
            }
            Self::BucketsValue => {
                "telemetry.buckets is not a non-empty array of finite numbers, strictly ascending"
            }
            Self::SemanticValue => "telemetry.semantic is not one of the eight semantics",
            Self::KindMissing => "no telemetry.kind, and the type is not the profile's Point",
            Self::BucketsWithoutHistogram => {
                "telemetry.buckets on a resource whose declared kind is not histogram"
            }
            Self::HistogramWithoutBuckets => "a histogram without telemetry.buckets",
            Self::NotACarrier => "the type cannot carry the declared kind",
        }
    }
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a reader reads of a resolved resource (§2.2, §2.4).
#[derive(Debug, Clone, PartialEq)]
pub enum Declaration {
    Declared(Declared),
    /// The reasons, in §2.2's order.
    Invalid(Vec<Invalid>),
    /// An event or an operation that writes a key of the vocabulary: it
    /// means nothing there (§2.1, §2.2).
    Ignored,
    /// Not a metric resource: no key of the vocabulary, and not the Point,
    /// or an event or an operation that writes none.
    NotThisProfiles,
}

impl Declaration {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Declared(_) => "declared",
            Self::Invalid(_) => "invalid",
            Self::Ignored => "ignored",
            Self::NotThisProfiles => "not_this_profiles",
        }
    }

    #[must_use]
    pub fn declared(&self) -> Option<&Declared> {
        match self {
            Self::Declared(d) => Some(d),
            _ => None,
        }
    }
}

/// One key's value, as §2.2 reads it.
enum Read<T> {
    Absent,
    Valid(T),
    Wrong,
}

impl<T> Read<T> {
    fn of(v: Option<&Value>, f: impl FnOnce(&Value) -> Option<T>) -> Self {
        match v {
            None => Self::Absent,
            Some(v) => f(v).map_or(Self::Wrong, Self::Valid),
        }
    }
    fn is_wrong(&self) -> bool {
        matches!(self, Self::Wrong)
    }
    fn valid(&self) -> Option<&T> {
        match self {
            Self::Valid(t) => Some(t),
            _ => None,
        }
    }
}

/// `telemetry.buckets`' value (§2.2): a non-empty array of finite numbers,
/// strictly ascending as doubles.
fn read_buckets(v: &Value) -> Option<Vec<f64>> {
    let a = v.as_array()?;
    let b: Vec<f64> = a.iter().map(Value::as_f64).collect::<Option<_>>()?;
    (!b.is_empty() && b.iter().all(|x| x.is_finite()) && ascending(&b)).then_some(b)
}

fn ascending(b: &[f64]) -> bool {
    b.windows(2).all(|w| w[0] < w[1])
}

/// A root schema's `type` as a set of names (core §7.3); empty when it has
/// none, or one that is not a name or a list of names.
fn type_names(root: &Value) -> Vec<&str> {
    let mut v: Vec<&str> = match root.get("type") {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(a)) => {
            let names: Option<Vec<&str>> = a.iter().map(Value::as_str).collect();
            names.unwrap_or_default()
        }
        _ => Vec::new(),
    };
    v.sort_unstable();
    v.dedup();
    v
}

/// Whether a type carries `kind` (§2.4).
#[must_use]
pub fn carries(ty: &TypeView<'_>, kind: MetricKind) -> bool {
    if ty.is_profiles(POINT) {
        return true;
    }
    if ty.is_profiles(HISTOGRAM) {
        return kind == MetricKind::Histogram;
    }
    match ty {
        TypeView::Raw => kind == MetricKind::Binary,
        TypeView::Protobuf | TypeView::Json { root: None, .. } => false,
        TypeView::Json {
            root: Some(root), ..
        } => {
            let t = type_names(root);
            match kind {
                MetricKind::Counter => t == ["integer"],
                MetricKind::Gauge => {
                    !t.is_empty() && t.iter().all(|n| matches!(*n, "integer" | "number"))
                }
                MetricKind::Boolean => t == ["boolean"],
                MetricKind::Text => t == ["string"],
                MetricKind::Histogram | MetricKind::Binary => false,
            }
        }
    }
}

/// Reads the declaration of a resource of `kind` from its resolved
/// annotations (core §9.3), its type and its attachment's type (§2.2,
/// §2.4). `ty` is `None` for an operation, which has no payload type.
#[must_use]
pub fn declaration(
    kind: Kind,
    annotations: &BTreeMap<String, Value>,
    ty: Option<&TypeView<'_>>,
    attachment: Option<&TypeView<'_>>,
) -> Declaration {
    let any_key = [KIND, UNIT, BUCKETS, SEMANTIC]
        .iter()
        .any(|k| annotations.contains_key(*k));
    if matches!(kind, Kind::Event | Kind::Operation) {
        return if any_key {
            Declaration::Ignored
        } else {
            Declaration::NotThisProfiles
        };
    }
    let point = ty.is_some_and(|t| t.is_profiles(POINT));
    if !any_key && !point {
        return Declaration::NotThisProfiles;
    }
    let k = Read::of(annotations.get(KIND), |v| {
        v.as_str().and_then(MetricKind::parse)
    });
    let u = Read::of(annotations.get(UNIT), |v| {
        v.as_str().filter(|s| is_unit(s)).map(str::to_owned)
    });
    let b = Read::of(annotations.get(BUCKETS), read_buckets);
    let s = Read::of(annotations.get(SEMANTIC), |v| {
        v.as_str().and_then(Semantic::parse)
    });
    let mut why = Vec::new();
    for (wrong, r) in [
        (k.is_wrong(), Invalid::KindValue),
        (u.is_wrong(), Invalid::UnitValue),
        (b.is_wrong(), Invalid::BucketsValue),
        (s.is_wrong(), Invalid::SemanticValue),
    ] {
        if wrong {
            why.push(r);
        }
    }
    if matches!(k, Read::Absent) && !point {
        why.push(Invalid::KindMissing);
    }
    let histogram = matches!(k, Read::Valid(MetricKind::Histogram));
    if matches!(b, Read::Valid(_)) && !k.is_wrong() && !histogram {
        why.push(Invalid::BucketsWithoutHistogram);
    }
    if histogram && matches!(b, Read::Absent) {
        why.push(Invalid::HistogramWithoutBuckets);
    }
    if let Some(declared) = k.valid()
        && !ty.is_some_and(|t| carries(t, *declared))
    {
        why.push(Invalid::NotACarrier);
    }
    if !why.is_empty() {
        return Declaration::Invalid(why);
    }
    let kind = k.valid().copied();
    let epoch = if point {
        Some(EpochIn::Point)
    } else if kind.is_some_and(MetricKind::is_cumulative)
        && attachment.is_some_and(|a| a.is_profiles(EPOCH))
    {
        Some(EpochIn::Attachment)
    } else {
        None
    };
    Declaration::Declared(Declared {
        carrier: if point { Carrier::Point } else { Carrier::Bare },
        kind,
        unit: u.valid().cloned(),
        buckets: b.valid().cloned(),
        semantic: s.valid().copied(),
        epoch,
    })
}

/// [`declaration`] of a resource of a contract: its type's root schema
/// resolved from the contract's artifacts ([`root_schema`]).
#[must_use]
pub fn declaration_of(c: &Contract, r: &Resource) -> Declaration {
    let Body::Data(d) = &r.body else {
        return declaration(r.kind, &r.annotations, None, None);
    };
    let root = match &d.type_ {
        TypeId::JsonSchema { name, schema } => root_schema(&c.artifacts, schema, name),
        _ => None,
    };
    let ty = view(&d.type_, root.as_ref());
    let attachment = d.attachment.as_ref().map(|a| view(a, None));
    declaration(r.kind, &r.annotations, Some(&ty), attachment.as_ref())
}

fn view<'a>(t: &'a TypeId, root: Option<&'a Value>) -> TypeView<'a> {
    match t {
        TypeId::JsonSchema { name, schema } => TypeView::Json { name, schema, root },
        TypeId::Raw { .. } => TypeView::Raw,
        TypeId::Protobuf { .. } => TypeView::Protobuf,
    }
}

/// The root schema of the JSON Schema type `name` in the artifact `schema`
/// (§2.4): `$defs/<name>`, its `$ref`s followed across the artifacts by
/// stem, as core §9.8 follows them, a keyword beside a `$ref` added to its
/// target and an outer one taking the place of the target's own. `None`
/// when a `$ref` resolves to nothing, or the chain runs past 64 steps.
#[must_use]
pub fn root_schema(
    artifacts: &BTreeMap<String, Artifact>,
    schema: &str,
    name: &str,
) -> Option<Value> {
    fn json(a: &Artifact) -> Option<&Value> {
        match &a.data {
            ArtifactData::Json(v) => Some(v),
            ArtifactData::Protobuf(_) => None,
        }
    }
    let mut doc = json(artifacts.get(schema)?)?;
    let mut s = doc.get("$defs")?.get(name)?.clone();
    for _ in 0..64 {
        let Some(Value::String(r)) = s.get("$ref").cloned() else {
            return Some(s);
        };
        let (file, pointer) = r.split_once('#').unwrap_or((r.as_str(), ""));
        if !file.is_empty() {
            let base = file.rsplit('/').next().unwrap_or(file);
            let stem = base.strip_suffix(".json").unwrap_or(base);
            doc = artifacts
                .values()
                .filter(|a| a.name == stem)
                .find_map(json)?;
        }
        let target = if pointer.is_empty() {
            doc
        } else {
            doc.pointer(pointer)?
        };
        let mut merged = target.as_object()?.clone();
        for (k, v) in s.as_object()? {
            if k != "$ref" && k != "$defs" {
                merged.insert(k.clone(), v.clone());
            }
        }
        if target.get("$ref").is_none() {
            merged.remove("$ref");
        } else {
            merged.insert("$ref".to_owned(), target["$ref"].clone());
        }
        s = Value::Object(merged);
    }
    None
}

// ---------------------------------------------------------------------------
// A decoded value (§2.6)
// ---------------------------------------------------------------------------

/// A sample's decoded data item (§2.6): JSON's data model, plus what CBOR
/// adds, byte strings and non-finite floats. Integers are exact.
#[derive(Debug, Clone, PartialEq)]
pub enum Datum {
    Null,
    Bool(bool),
    /// An integer: JSON text with no fraction and no exponent, CBOR's major
    /// types 0 and 1.
    Int(i128),
    /// A float: JSON text with a fraction or an exponent, a CBOR float.
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
    Array(Vec<Datum>),
    Map(BTreeMap<String, Datum>),
}

impl Datum {
    /// A JSON value as a datum: an integer JSON number exactly, any other
    /// as its double.
    #[must_use]
    pub fn from_json(v: &Value) -> Self {
        match v {
            Value::Null => Self::Null,
            Value::Bool(b) => Self::Bool(*b),
            Value::Number(n) => {
                if let Some(u) = n.as_u64() {
                    Self::Int(i128::from(u))
                } else if let Some(i) = n.as_i64() {
                    Self::Int(i128::from(i))
                } else {
                    Self::Float(n.as_f64().unwrap_or(f64::NAN))
                }
            }
            Value::String(s) => Self::Text(s.clone()),
            Value::Array(a) => Self::Array(a.iter().map(Self::from_json).collect()),
            Value::Object(m) => Self::Map(
                m.iter()
                    .map(|(k, v)| (k.clone(), Self::from_json(v)))
                    .collect(),
            ),
        }
    }

    /// A CBOR data item as a datum, keeping byte strings and non-finite
    /// floats (§2.6); a tag reads as its content. `None` for a map key that
    /// is not text, or a value with no datum.
    #[must_use]
    pub fn from_cbor(v: &ciborium::Value) -> Option<Self> {
        use ciborium::Value as C;
        Some(match v {
            C::Null => Self::Null,
            C::Bool(b) => Self::Bool(*b),
            C::Integer(i) => Self::Int((*i).into()),
            C::Float(f) => Self::Float(*f),
            C::Text(s) => Self::Text(s.clone()),
            C::Bytes(b) => Self::Bytes(b.clone()),
            C::Array(a) => Self::Array(a.iter().map(Self::from_cbor).collect::<Option<_>>()?),
            C::Map(m) => {
                let mut out = BTreeMap::new();
                for (k, v) in m {
                    let C::Text(k) = k else { return None };
                    out.insert(k.clone(), Self::from_cbor(v)?);
                }
                Self::Map(out)
            }
            C::Tag(_, inner) => Self::from_cbor(inner)?,
            _ => return None,
        })
    }

    /// One JSON text, as a datum (§2.6): `None` when the bytes are not one.
    #[must_use]
    pub fn from_json_bytes(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice::<Value>(bytes)
            .ok()
            .map(|v| Self::from_json(&v))
    }

    /// One CBOR data item and nothing after it, as a datum.
    #[must_use]
    pub fn from_cbor_bytes(bytes: &[u8]) -> Option<Self> {
        let mut r = bytes;
        let v: ciborium::Value = ciborium::from_reader(&mut r).ok()?;
        if !r.is_empty() {
            return None;
        }
        Self::from_cbor(&v)
    }

    fn get(&self, k: &str) -> Option<&Self> {
        match self {
            Self::Map(m) => m.get(k),
            _ => None,
        }
    }

    /// A number, as a double (an integer to its nearest).
    fn number(&self) -> Option<f64> {
        match self {
            Self::Int(i) => Some(*i as f64),
            Self::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// An integer, as JSON Schema's `integer` reads one: an integer, or an
    /// integral finite float, the integer it equals. `None` for any other
    /// value. An integral float beyond ±2^127 saturates to `i128`'s bound,
    /// which is beyond every range §2.6 asks of an integer, as its value is.
    fn integer(&self) -> Option<i128> {
        match self {
            Self::Int(i) => Some(*i),
            // `as` saturates a float to the integer's bound.
            Self::Float(f) if f.is_finite() && f.fract() == 0.0 => Some(*f as i128),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// A sample's conformance (§2.5, §2.6)
// ---------------------------------------------------------------------------

/// A sample's payload, as the reader received it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Payload<'a> {
    /// The decoded data item, and the decoded attachment, if any.
    Decoded {
        value: &'a Datum,
        attachment: Option<&'a Datum>,
    },
    /// Bytes the core could not decode (§2.6).
    Undecodable,
}

/// Why a sample does not conform (§2.6), in the order a reader checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Problem {
    NotAPoint,
    EpochInvalid,
    UnitInvalid,
    TagDisagrees,
    UnitDisagrees,
    WrongType,
    OutOfRange,
    NotFinite,
    BoundsInvalid,
    BoundsDiffer,
    CountsLength,
    CountSum,
}

impl Problem {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotAPoint => "not_a_point",
            Self::EpochInvalid => "epoch_invalid",
            Self::UnitInvalid => "unit_invalid",
            Self::TagDisagrees => "tag_disagrees",
            Self::UnitDisagrees => "unit_disagrees",
            Self::WrongType => "wrong_type",
            Self::OutOfRange => "out_of_range",
            Self::NotFinite => "not_finite",
            Self::BoundsInvalid => "bounds_invalid",
            Self::BoundsDiffer => "bounds_differ",
            Self::CountsLength => "counts_length",
            Self::CountSum => "count_sum",
        }
    }

    /// The reason in words, for a tool's message.
    #[must_use]
    pub fn says(self) -> &'static str {
        match self {
            Self::NotAPoint => "the payload is not a telemetry.v1.Point",
            Self::EpochInvalid => "the in-band epoch is not a string of 1 to 64 characters",
            Self::UnitInvalid => "the Point's unit is not a unit",
            Self::TagDisagrees => "the Point's tag differs from the declared kind",
            Self::UnitDisagrees => "the Point's unit differs from the declared unit",
            Self::WrongType => "the value is not of the type its kind needs",
            Self::OutOfRange => "a count outside 0 to 2^64-1",
            Self::NotFinite => "a number that is NaN or infinite",
            Self::BoundsInvalid => "the histogram's bounds are empty or not strictly ascending",
            Self::BoundsDiffer => "the histogram's bounds differ from the declared buckets",
            Self::CountsLength => "counts is not one longer than buckets",
            Self::CountSum => "count is not the sum of counts",
        }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a question about a sample was put and could not be answered, or
/// was not asked (§2.6, §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unasked {
    /// Unobservable: the declaration is invalid (§2.2).
    DeclarationInvalid,
    /// Unobservable: the core could not decode the sample.
    Undecodable,
    /// Not asked: not a metric resource.
    NotThisProfiles,
    /// Not asked: an event or an operation.
    Ignored,
}

impl Unasked {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DeclarationInvalid => "declaration_invalid",
            Self::Undecodable => "undecodable",
            Self::NotThisProfiles => "not_this_profiles",
            Self::Ignored => "ignored",
        }
    }
}

/// The answer to "does this sample conform?" (§2.6). `DoesNot` is the
/// finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conformance {
    Conforms,
    /// The reasons, in §2.6's order.
    DoesNot(Vec<Problem>),
    Unobservable(Unasked),
    NotAsked(Unasked),
}

impl Conformance {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Conforms => "conforms",
            Self::DoesNot(_) => "does_not",
            Self::Unobservable(_) => "unobservable",
            Self::NotAsked(_) => "not_asked",
        }
    }
}

/// Where a member's kind or unit came from (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The resource's annotation.
    Declared,
    /// The Point: its tag, or its `unit`.
    Point,
}

impl Source {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Declared => "declared",
            Self::Point => "point",
        }
    }
}

/// A histogram's value (§2.8).
#[derive(Debug, Clone, PartialEq)]
pub struct Histogram {
    pub bounds: Vec<f64>,
    pub counts: Vec<u64>,
    pub count: u64,
    pub sum: f64,
}

/// A conforming sample's value, by its kind.
#[derive(Debug, Clone, PartialEq)]
pub enum Measure {
    Counter(u64),
    Gauge(f64),
    Histogram(Histogram),
    Text(String),
    Boolean(bool),
    Binary(Vec<u8>),
}

impl Measure {
    #[must_use]
    pub fn kind(&self) -> MetricKind {
        match self {
            Self::Counter(_) => MetricKind::Counter,
            Self::Gauge(_) => MetricKind::Gauge,
            Self::Histogram(_) => MetricKind::Histogram,
            Self::Text(_) => MetricKind::Text,
            Self::Boolean(_) => MetricKind::Boolean,
            Self::Binary(_) => MetricKind::Binary,
        }
    }
}

/// What a conforming sample says: its value, its measurement time (a
/// Point's `timestamp`) and its in-band epoch.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub value: Measure,
    pub timestamp_ms: Option<i64>,
    pub epoch: Option<String>,
}

/// One sample, read (§2.5, §2.6).
#[derive(Debug, Clone, PartialEq)]
pub struct Checked {
    pub conformance: Conformance,
    /// The member's kind as this sample gives it, or `None`: unobservable
    /// or not this profile's (§2.5).
    pub kind: Option<(MetricKind, Source)>,
    /// The member's unit as this sample gives it, or `None`: unknown,
    /// never dimensionless (§2.5).
    pub unit: Option<(String, Source)>,
    /// The sample's value, when it conforms.
    pub reading: Option<Reading>,
}

/// A Point, its shape checked (§2.6, check 1).
struct PointView<'a> {
    timestamp: i64,
    tag: MetricKind,
    value: &'a Datum,
    unit: Option<&'a str>,
    epoch: Option<&'a Datum>,
}

fn point_view(d: &Datum) -> Option<PointView<'_>> {
    let Datum::Map(_) = d else { return None };
    let timestamp = i64::try_from(d.get("timestamp")?.integer()?).ok()?;
    let v = d.get("value")?;
    let Datum::Map(_) = v else { return None };
    let Some(Datum::Text(tag)) = v.get("type") else {
        return None;
    };
    let tag = MetricKind::parse(tag)?;
    let value = v.get("value")?;
    match d.get("labels") {
        None => {}
        Some(Datum::Map(m)) if m.values().all(|l| matches!(l, Datum::Text(_))) => {}
        Some(_) => return None,
    }
    let unit = match d.get("unit") {
        None => None,
        Some(Datum::Text(u)) => Some(u.as_str()),
        Some(_) => return None,
    };
    Some(PointView {
        timestamp,
        tag,
        value,
        unit,
        epoch: d.get("epoch"),
    })
}

/// A count (§2.6, check 7): `Ok` in 0 to 2^64−1, `Err` out of range.
fn count(i: i128) -> Result<u64, ()> {
    u64::try_from(i).map_err(|_| ())
}

/// The value checks (§2.6, checks 6 to 12) of `v` as a `kind`, a Point's
/// value or a bare one, against the declared bounds of a histogram.
fn check_value(
    kind: MetricKind,
    v: &Datum,
    point: bool,
    declared: Option<&[f64]>,
    out: &mut Vec<Problem>,
) -> Option<Measure> {
    match kind {
        MetricKind::Counter => {
            let Some(i) = v.integer() else {
                out.push(Problem::WrongType);
                return None;
            };
            match count(i) {
                Ok(n) => Some(Measure::Counter(n)),
                Err(()) => {
                    out.push(Problem::OutOfRange);
                    None
                }
            }
        }
        MetricKind::Gauge => {
            let Some(f) = v.number() else {
                out.push(Problem::WrongType);
                return None;
            };
            if f.is_finite() {
                Some(Measure::Gauge(f))
            } else {
                out.push(Problem::NotFinite);
                None
            }
        }
        MetricKind::Boolean => match v {
            Datum::Bool(b) => Some(Measure::Boolean(*b)),
            _ => {
                out.push(Problem::WrongType);
                None
            }
        },
        MetricKind::Text => match v {
            Datum::Text(s) => Some(Measure::Text(s.clone())),
            _ => {
                out.push(Problem::WrongType);
                None
            }
        },
        MetricKind::Binary => {
            let bytes = match (point, v) {
                (false, Datum::Bytes(b)) => Some(b.clone()),
                (true, Datum::Array(a)) => a
                    .iter()
                    .map(|x| x.integer().and_then(|i| u8::try_from(i).ok()))
                    .collect(),
                _ => None,
            };
            if bytes.is_none() {
                out.push(Problem::WrongType);
            }
            bytes.map(Measure::Binary)
        }
        MetricKind::Histogram => check_histogram(v, declared, out),
    }
}

fn check_histogram(v: &Datum, declared: Option<&[f64]>, out: &mut Vec<Problem>) -> Option<Measure> {
    let shape = (|| {
        let Some(Datum::Array(b)) = v.get("buckets") else {
            return None;
        };
        let Some(Datum::Array(c)) = v.get("counts") else {
            return None;
        };
        let bounds: Vec<f64> = b.iter().map(Datum::number).collect::<Option<_>>()?;
        let counts: Vec<i128> = c.iter().map(Datum::integer).collect::<Option<_>>()?;
        let count = v.get("count")?.integer()?;
        let sum = v.get("sum")?.number()?;
        Some((bounds, counts, count, sum))
    })();
    let Some((bounds, counts, total, sum)) = shape else {
        out.push(Problem::WrongType);
        return None;
    };
    let before = out.len();
    let in_range: Option<(Vec<u64>, u64)> = (|| {
        let c: Vec<u64> = counts
            .iter()
            .map(|i| count(*i).ok())
            .collect::<Option<_>>()?;
        Some((c, count(total).ok()?))
    })();
    if in_range.is_none() {
        out.push(Problem::OutOfRange);
    }
    let finite = bounds.iter().all(|b| b.is_finite());
    if !finite || !sum.is_finite() {
        out.push(Problem::NotFinite);
    }
    let valid_bounds = finite && !bounds.is_empty() && ascending(&bounds);
    if finite && !valid_bounds {
        out.push(Problem::BoundsInvalid);
    }
    if valid_bounds
        && let Some(d) = declared
        && (d.len() != bounds.len() || d.iter().zip(&bounds).any(|(a, b)| a != b))
    {
        out.push(Problem::BoundsDiffer);
    }
    if counts.len() != bounds.len() + 1 {
        out.push(Problem::CountsLength);
    }
    if let Some((c, n)) = &in_range
        && c.iter().map(|x| u128::from(*x)).sum::<u128>() != u128::from(*n)
    {
        out.push(Problem::CountSum);
    }
    if out.len() > before {
        return None;
    }
    let (counts, count) = in_range?;
    Some(Measure::Histogram(Histogram {
        bounds,
        counts,
        count,
        sum,
    }))
}

/// Reads one sample of a member whose resource's declaration is `decl`
/// (§2.5, §2.6): its conformance, the member's kind and unit as the sample
/// gives them, and its value when it conforms.
#[must_use]
pub fn check(decl: &Declaration, payload: &Payload<'_>) -> Checked {
    let none = |c| Checked {
        conformance: c,
        kind: None,
        unit: None,
        reading: None,
    };
    let d = match decl {
        Declaration::NotThisProfiles => {
            return none(Conformance::NotAsked(Unasked::NotThisProfiles));
        }
        Declaration::Ignored => return none(Conformance::NotAsked(Unasked::Ignored)),
        Declaration::Invalid(_) => {
            return none(Conformance::Unobservable(Unasked::DeclarationInvalid));
        }
        Declaration::Declared(d) => d,
    };
    let mut out = Checked {
        conformance: Conformance::Conforms,
        kind: d.kind.map(|k| (k, Source::Declared)),
        unit: d.unit.clone().map(|u| (u, Source::Declared)),
        reading: None,
    };
    let Payload::Decoded { value, attachment } = *payload else {
        out.conformance = Conformance::Unobservable(Unasked::Undecodable);
        return out;
    };
    let mut problems = Vec::new();
    let epoch_of = |e: &Datum, problems: &mut Vec<Problem>| match e {
        Datum::Text(s) if is_epoch(s) => Some(s.clone()),
        _ => {
            problems.push(Problem::EpochInvalid);
            None
        }
    };
    let (measure, timestamp_ms, epoch, point) = match d.carrier {
        Carrier::Point => {
            let Some(p) = point_view(value) else {
                out.conformance = Conformance::DoesNot(vec![Problem::NotAPoint]);
                return out;
            };
            let epoch = p.epoch.and_then(|e| epoch_of(e, &mut problems));
            let unit = p.unit.filter(|u| is_unit(u));
            if p.unit.is_some() && unit.is_none() {
                problems.push(Problem::UnitInvalid);
            }
            if d.kind.is_some_and(|k| k != p.tag) {
                problems.push(Problem::TagDisagrees);
            }
            if let (Some(du), Some(u)) = (&d.unit, unit)
                && du != u
            {
                problems.push(Problem::UnitDisagrees);
            }
            let m = check_value(p.tag, p.value, true, d.buckets.as_deref(), &mut problems);
            (m, Some(p.timestamp), epoch, Some((p.tag, unit)))
        }
        Carrier::Bare => {
            let kind = d.kind.expect("a valid bare declaration declares its kind");
            let epoch = match (d.epoch, attachment) {
                (Some(EpochIn::Attachment), Some(a)) => epoch_of(a, &mut problems),
                _ => None,
            };
            let m = check_value(kind, value, false, d.buckets.as_deref(), &mut problems);
            (m, None, epoch, None)
        }
    };
    if !problems.is_empty() {
        problems.sort_unstable();
        problems.dedup();
        out.conformance = Conformance::DoesNot(problems);
        return out;
    }
    if let Some((tag, unit)) = point {
        out.kind.get_or_insert((tag, Source::Point));
        if out.unit.is_none() {
            out.unit = unit.map(|u| (u.to_owned(), Source::Point));
        }
    }
    out.reading = measure.map(|value| Reading {
        value,
        timestamp_ms,
        epoch,
    });
    out
}

// ---------------------------------------------------------------------------
// A pair: its continuity and its rate (§2.9–§2.11)
// ---------------------------------------------------------------------------

/// One sample of a pair, as the reader holds it (§2.10, §2.11).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sampled<'a> {
    /// The conforming sample's value; its kind is the sample's.
    pub value: &'a Measure,
    /// Its in-band epoch (§2.9), if it carries one.
    pub epoch: Option<&'a str>,
    /// The Point's `timestamp`, for a Point.
    pub timestamp_ms: Option<i64>,
    /// The owner's stamp, in nanoseconds since the Unix epoch, for state.
    pub stamp_ns: Option<i128>,
    /// When the reader received it, in nanoseconds on its monotonic clock.
    pub arrival_ns: i128,
}

/// What the reader observed of the owning service's instance tokens, or of
/// a member's tokens for a template that declares `epoch` (§2.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Presence<'a> {
    /// One instance (or member epoch) alone, without a loss on the presence
    /// view, from before the earlier arrival to after the later one's.
    pub alone_throughout: bool,
    /// The only instance present at the earlier arrival; `None` when there
    /// was none, several, or the reader cannot tell.
    pub earlier: Option<&'a str>,
    /// The only instance present at the later arrival, likewise.
    pub later: Option<&'a str>,
}

/// How an epoch was established (§2.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    InBand,
    Presence,
}

impl Via {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InBand => "in_band",
            Self::Presence => "presence",
        }
    }
}

/// The epoch evidence for a pair (§2.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Epochs {
    Same(Via),
    Different(Via),
    Unobservable,
}

/// §2.9: in-band when both carry an epoch, else presence, else
/// unobservable.
#[must_use]
pub fn epochs(
    earlier: Option<&str>,
    later: Option<&str>,
    presence: Option<&Presence<'_>>,
) -> Epochs {
    if let (Some(a), Some(b)) = (earlier, later) {
        return if a == b {
            Epochs::Same(Via::InBand)
        } else {
            Epochs::Different(Via::InBand)
        };
    }
    match presence {
        Some(p) if p.alone_throughout => Epochs::Same(Via::Presence),
        Some(Presence {
            earlier: Some(a),
            later: Some(b),
            ..
        }) if a != b => Epochs::Different(Via::Presence),
        _ => Epochs::Unobservable,
    }
}

/// The answer to a pair's continuity (§2.10). `Decreased` is the finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Continuity {
    SameEpoch,
    Reset,
    Decreased,
    /// The two samples cannot be compared (`incomparable`).
    Unobservable,
    /// Neither sample is a counter or a histogram (`kind`).
    NotAsked,
}

impl Continuity {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SameEpoch => "same_epoch",
            Self::Reset => "reset",
            Self::Decreased => "decreased",
            Self::Unobservable => "unobservable",
            Self::NotAsked => "not_asked",
        }
    }
}

/// Whether a pair's epoch was established, and how, or assumed (§2.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    Established(Via),
    Assumed,
}

/// Why a pair has no rate (§2.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoRate {
    /// Not asked: the pair is not of counters.
    Kind,
    Reset,
    Decreased,
    Incomparable,
    /// The elapsed time is 0 or less.
    Elapsed,
}

impl NoRate {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Kind => "kind",
            Self::Reset => "reset",
            Self::Decreased => "decreased",
            Self::Incomparable => "incomparable",
            Self::Elapsed => "elapsed",
        }
    }
}

/// A counter's rate over a pair (§2.11).
#[derive(Debug, Clone, PartialEq)]
pub struct Rate {
    /// Per second, by §2.11's formula.
    pub per_s: f64,
    /// The counter's unit followed by `/s`; `None` when it is unknown.
    pub unit: Option<String>,
}

/// A pair, judged (§2.10, §2.11).
#[derive(Debug, Clone, PartialEq)]
pub struct Pair {
    pub continuity: Continuity,
    /// `None` when the epoch was not asked of the pair.
    pub basis: Option<Basis>,
    pub rate: Result<Rate, NoRate>,
}

/// Whether `later` decreased from `earlier` (§2.10); `None` when the two
/// cannot be compared.
fn decreased(earlier: &Measure, later: &Measure) -> Option<bool> {
    match (earlier, later) {
        (Measure::Counter(a), Measure::Counter(b)) => Some(b < a),
        (Measure::Histogram(a), Measure::Histogram(b)) => {
            let same_bounds = a.bounds.len() == b.bounds.len()
                && a.bounds.iter().zip(&b.bounds).all(|(x, y)| x == y);
            if !same_bounds || a.counts.len() != b.counts.len() {
                return None;
            }
            Some(b.count < a.count || a.counts.iter().zip(&b.counts).any(|(x, y)| y < x))
        }
        _ => None,
    }
}

/// The elapsed time between two samples, in nanoseconds, on the first time
/// base both have (§2.11).
fn elapsed_ns(a: &Sampled<'_>, b: &Sampled<'_>) -> i128 {
    if let (Some(x), Some(y)) = (a.timestamp_ms, b.timestamp_ms) {
        (i128::from(y) - i128::from(x)) * 1_000_000
    } else if let (Some(x), Some(y)) = (a.stamp_ns, b.stamp_ns) {
        y - x
    } else {
        b.arrival_ns - a.arrival_ns
    }
}

/// §2.11's rate: `double(d) / (double(e) / 1e9)`, rounding to nearest.
#[must_use]
pub fn rate_per_s(d: u64, elapsed_ns: i128) -> f64 {
    d as f64 / (elapsed_ns as f64 / 1e9)
}

/// Judges a pair of conforming samples of one member, in arrival order
/// (§2.9–§2.11). `unit` is the counter's unit (§2.5), for the rate's.
#[must_use]
pub fn pair(
    earlier: &Sampled<'_>,
    later: &Sampled<'_>,
    unit: Option<&str>,
    presence: Option<&Presence<'_>>,
) -> Pair {
    let (ka, kb) = (earlier.value.kind(), later.value.kind());
    if !ka.is_cumulative() && !kb.is_cumulative() {
        return Pair {
            continuity: Continuity::NotAsked,
            basis: None,
            rate: Err(NoRate::Kind),
        };
    }
    let counters = ka == MetricKind::Counter && kb == MetricKind::Counter;
    let no_rate = |why: NoRate| -> Result<Rate, NoRate> {
        if counters {
            Err(why)
        } else {
            Err(NoRate::Kind)
        }
    };
    let Some(down) = (ka == kb)
        .then(|| decreased(earlier.value, later.value))
        .flatten()
    else {
        return Pair {
            continuity: Continuity::Unobservable,
            basis: None,
            rate: no_rate(NoRate::Incomparable),
        };
    };
    let (continuity, basis) = match epochs(earlier.epoch, later.epoch, presence) {
        Epochs::Same(via) if down => (Continuity::Decreased, Basis::Established(via)),
        Epochs::Same(via) => (Continuity::SameEpoch, Basis::Established(via)),
        Epochs::Different(via) => (Continuity::Reset, Basis::Established(via)),
        Epochs::Unobservable if down => (Continuity::Reset, Basis::Assumed),
        Epochs::Unobservable => (Continuity::SameEpoch, Basis::Assumed),
    };
    let rate = match (continuity, earlier.value, later.value) {
        (Continuity::Reset, ..) => no_rate(NoRate::Reset),
        (Continuity::Decreased, ..) => no_rate(NoRate::Decreased),
        (Continuity::SameEpoch, Measure::Counter(a), Measure::Counter(b)) => {
            let e = elapsed_ns(earlier, later);
            if e <= 0 {
                Err(NoRate::Elapsed)
            } else {
                Ok(Rate {
                    per_s: rate_per_s(b - a, e),
                    unit: unit.map(|u| format!("{u}/s")),
                })
            }
        }
        _ => Err(NoRate::Kind),
    };
    Pair {
        continuity,
        basis: Some(basis),
        rate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ann(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect()
    }

    fn point() -> TypeView<'static> {
        TypeView::Json {
            name: POINT,
            schema: PUBLISHED[0],
            root: None,
        }
    }

    #[test]
    fn a_kind_on_a_counter_type_is_declared() {
        let root = json!({"type": "integer", "minimum": 0});
        let ty = TypeView::Json {
            name: "Counter",
            schema: "sha256:00",
            root: Some(&root),
        };
        let d = declaration(
            Kind::Stream,
            &ann(&[(KIND, json!("counter")), (UNIT, json!("By"))]),
            Some(&ty),
            None,
        );
        let Declaration::Declared(d) = d else {
            panic!("{d:?}")
        };
        assert_eq!(d.kind, Some(MetricKind::Counter));
        assert_eq!(d.unit.as_deref(), Some("By"));
        assert_eq!(d.carrier, Carrier::Bare);
    }

    #[test]
    fn a_point_needs_no_kind_and_is_known_by_its_id() {
        let d = declaration(Kind::Stream, &BTreeMap::new(), Some(&point()), None);
        assert_eq!(d.declared().map(|d| d.carrier), Some(Carrier::Point));
        let other = TypeView::Json {
            name: POINT,
            schema: "sha256:00",
            root: None,
        };
        assert_eq!(
            declaration(Kind::Stream, &BTreeMap::new(), Some(&other), None),
            Declaration::NotThisProfiles
        );
    }

    #[test]
    fn a_counter_is_read_exactly() {
        let d = declaration(
            Kind::Stream,
            &ann(&[(KIND, json!("counter"))]),
            Some(&TypeView::Json {
                name: "C",
                schema: "x",
                root: Some(&json!({"type": "integer"})),
            }),
            None,
        );
        let at = |v: Value| {
            let datum = Datum::from_json(&v);
            check(
                &d,
                &Payload::Decoded {
                    value: &datum,
                    attachment: None,
                },
            )
            .conformance
        };
        assert_eq!(at(json!(u64::MAX)), Conformance::Conforms);
        assert_eq!(
            at(json!(18_446_744_073_709_551_616.0)),
            Conformance::DoesNot(vec![Problem::OutOfRange])
        );
        assert_eq!(at(json!(5.0)), Conformance::Conforms);
        assert_eq!(
            at(json!(5.5)),
            Conformance::DoesNot(vec![Problem::WrongType])
        );
    }

    #[test]
    fn a_decrease_is_a_reset_unless_one_epoch_is_established() {
        let (a, b) = (Measure::Counter(10), Measure::Counter(3));
        let s = |v, e| Sampled {
            value: v,
            epoch: e,
            timestamp_ms: None,
            stamp_ns: None,
            arrival_ns: 0,
        };
        let p = pair(&s(&a, None), &s(&b, None), None, None);
        assert_eq!(
            (p.continuity, p.basis),
            (Continuity::Reset, Some(Basis::Assumed))
        );
        let p = pair(&s(&a, Some("e")), &s(&b, Some("e")), None, None);
        assert_eq!(p.continuity, Continuity::Decreased);
        assert_eq!(p.rate, Err(NoRate::Decreased));
    }

    #[test]
    fn the_rate_is_the_formula() {
        let (a, b) = (Measure::Counter(0), Measure::Counter(1_000));
        let s = |v, t| Sampled {
            value: v,
            epoch: None,
            timestamp_ms: Some(t),
            stamp_ns: None,
            arrival_ns: 0,
        };
        let p = pair(&s(&a, 0), &s(&b, 2_000), Some("By"), None);
        assert_eq!(
            p.rate,
            Ok(Rate {
                per_s: 500.0,
                unit: Some("By/s".into())
            })
        );
    }

    #[test]
    fn a_root_follows_its_refs_across_files_with_outer_keywords_winning() {
        let art = |name: &str, doc: Value| Artifact {
            kind: crate::schema::SchemaKind::JsonSchema,
            name: name.to_owned(),
            data: ArtifactData::Json(doc),
        };
        let a = art(
            "a",
            json!({"$defs": {
                "Count": {"$ref": "#/$defs/Inner", "description": "outer"},
                "Inner": {"$ref": "b.json#/$defs/Base", "minimum": 1},
                "Loop": {"$ref": "#/$defs/Loop"},
            }}),
        );
        let b = art(
            "b",
            json!({"$defs": {"Base": {"type": "integer", "minimum": 0}}}),
        );
        let arts = BTreeMap::from([("sha256:a".to_owned(), a), ("sha256:b".to_owned(), b)]);
        assert_eq!(
            root_schema(&arts, "sha256:a", "Count"),
            Some(json!({"type": "integer", "minimum": 1, "description": "outer"}))
        );
        assert_eq!(root_schema(&arts, "sha256:a", "Loop"), None, "a loop ends");
        assert_eq!(root_schema(&arts, "sha256:a", "Missing"), None);
    }

    #[test]
    fn units_and_epochs_have_their_lengths() {
        assert!(is_unit("dBm") && is_unit("bit/s") && !is_unit("") && !is_unit("m s"));
        assert!(!is_unit(&"x".repeat(65)) && is_unit(&"x".repeat(64)));
        assert!(is_epoch(&"é".repeat(64)) && !is_epoch(&"é".repeat(65)) && !is_epoch(""));
    }
}
