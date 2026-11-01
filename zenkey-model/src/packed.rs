//! Packed records (core §7.6, 0.75; #979 B1): a type kind whose record is
//! bit fields, each as wide as the codes its declaration names.
//!
//! - **The schema file** ([`PackedDoc::read`]) is strict JSON: `types`, each
//!   a record of `fields`, every field `int`, `fixed`, `enum`, `bool`,
//!   `counter` or `time`, optionally optional, and `"..."` the extension
//!   marker. Its lints are E038 to E045 (core §9.2), in cascade 7's order.
//! - **The layout** ([`Layout`]) is each item's width, the record's
//!   shortest and longest length, its layout form (the root's wire, without
//!   names) and the tag, the low four bits of the form's sha256.
//! - **The codec** ([`Layout::write`], [`Layout::read`]) is interpreted
//!   over `serde_json::Value`, as core §7.6's JSON reading spells a record.
//!   It is the source of truth that generated code (B2) matches, checked
//!   against `spec/conformance/packed/vectors.json`.
//!
//! Fixed point is computed in exact decimal arithmetic over big integers,
//! never in binary floating point: a JSON number is read by its shortest
//! decimal text, as JSON writes it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use num_bigint::{BigInt, Sign};
use num_integer::Integer as _;
use num_traits::{One as _, Signed as _, ToPrimitive as _, Zero as _};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// The extension marker, as `fields` writes it.
pub const MARKER: &str = "...";

/// The six field types of core §7.6, by their `type` spelling.
pub const FIELD_TYPES: &[&str] = &["int", "fixed", "enum", "bool", "counter", "time"];

/// The reserved codes, in the order they take the top codes of a width.
pub const RESERVED: &[&str] = &["not_available", "error", "out_of_range"];

/// The most digits a writer's decimal may carry, either side of the point,
/// once its exponent is applied. A model limit, not the text's: a value
/// past it is refused as `value`.
const INPUT_DIGITS: u32 = 1024;

// --- Exact decimals ---------------------------------------------------------

/// An exact decimal: `m / 10^e`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decimal {
    m: BigInt,
    e: u32,
}

impl Decimal {
    /// A declaration's decimal string, `-?(0|[1-9][0-9]*)(\.[0-9]+)?`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let body = s.strip_prefix('-').unwrap_or(s);
        let (int, frac) = match body.split_once('.') {
            Some((i, f)) => (i, Some(f)),
            None => (body, None),
        };
        let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
        if !digits(int) || (int.len() > 1 && int.starts_with('0')) {
            return None;
        }
        if frac.is_some_and(|f| !digits(f)) {
            return None;
        }
        Self::from_parts(s.starts_with('-'), int, frac.unwrap_or(""), 0)
    }

    /// A writer's decimal: a JSON number's text (RFC 8259 §6, core §7.6
    /// "What a writer takes"), the declaration grammar with an exponent
    /// allowed: no `+`, no leading zero, no bare point.
    #[must_use]
    pub fn parse_input(s: &str) -> Option<Self> {
        let neg = s.starts_with('-');
        let body = s.strip_prefix('-').unwrap_or(s);
        let (mant, exp) = match body.find(['e', 'E']) {
            Some(i) => (&body[..i], Some(&body[i + 1..])),
            None => (body, None),
        };
        let (int, frac) = match mant.split_once('.') {
            Some((i, f)) => (i, f),
            None => (mant, ""),
        };
        let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
        if !digits(int) || (int.len() > 1 && int.starts_with('0')) {
            return None;
        }
        if mant.contains('.') && !digits(frac) {
            return None;
        }
        let exp: i64 = match exp {
            None => 0,
            Some(x) => {
                let d = x.strip_prefix(['+', '-']).unwrap_or(x);
                if !digits(d) || d.len() > 9 {
                    return None;
                }
                x.parse().ok()?
            }
        };
        Self::from_parts(neg, int, frac, exp)
    }

    fn from_parts(neg: bool, int: &str, frac: &str, exp: i64) -> Option<Self> {
        let text = format!("{int}{frac}");
        let mut m: BigInt = text.parse().ok()?;
        let scale = i64::try_from(frac.len()).ok()? - exp;
        let width = i64::try_from(int.len()).ok()? + exp;
        if scale > i64::from(INPUT_DIGITS) || width > i64::from(INPUT_DIGITS) {
            return None;
        }
        let e = if scale < 0 {
            m *= pow10(u32::try_from(-scale).ok()?);
            0
        } else {
            u32::try_from(scale).ok()?
        };
        if neg {
            m = -m;
        }
        Some(Self { m, e }.normalized())
    }

    fn normalized(mut self) -> Self {
        let ten = BigInt::from(10);
        while self.e > 0 && (&self.m % &ten).is_zero() {
            self.m /= &ten;
            self.e -= 1;
        }
        self
    }

    /// `m` at the scale `e` (which is at least this decimal's).
    fn at(&self, e: u32) -> BigInt {
        &self.m * pow10(e - self.e)
    }

    fn sub(&self, o: &Self) -> Self {
        let e = self.e.max(o.e);
        Self {
            m: self.at(e) - o.at(e),
            e,
        }
        .normalized()
    }

    fn add(&self, o: &Self) -> Self {
        let e = self.e.max(o.e);
        Self {
            m: self.at(e) + o.at(e),
            e,
        }
        .normalized()
    }

    fn mul_int(&self, k: &BigInt) -> Self {
        Self {
            m: &self.m * k,
            e: self.e,
        }
        .normalized()
    }

    fn is_positive(&self) -> bool {
        self.m.is_positive()
    }

    /// `self / o`, when it is an integer.
    fn div_exact(&self, o: &Self) -> Option<BigInt> {
        let e = self.e.max(o.e);
        let (q, r) = self.at(e).div_mod_floor(&o.at(e));
        r.is_zero().then_some(q)
    }

    /// `self / o` rounded to the nearest integer, a tie to the even one.
    fn div_round_even(&self, o: &Self) -> BigInt {
        let e = self.e.max(o.e);
        let d = o.at(e);
        let (q, r) = self.at(e).div_mod_floor(&d);
        let twice: BigInt = &r * BigInt::from(2u8);
        let up = match twice.cmp(&d) {
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Equal => q.is_odd(),
        };
        if up { q + BigInt::one() } else { q }
    }
}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for Decimal {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        let e = self.e.max(o.e);
        self.at(e).cmp(&o.at(e))
    }
}

impl std::fmt::Display for Decimal {
    /// The shortest form (core §7.6): no zero at the end of a fraction, no
    /// point without a digit after it, `0` for zero.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.clone().normalized();
        if n.m.is_zero() {
            return f.write_str("0");
        }
        let digits = n.m.abs().to_string();
        let e = n.e as usize;
        let sign = if n.m.sign() == Sign::Minus { "-" } else { "" };
        if e == 0 {
            return write!(f, "{sign}{digits}");
        }
        let padded = format!("{digits:0>width$}", width = e + 1);
        let (int, frac) = padded.split_at(padded.len() - e);
        write!(f, "{sign}{int}.{frac}")
    }
}

impl std::str::FromStr for Decimal {
    type Err = WriteRefusal;

    /// A writer's decimal ([`Decimal::parse_input`]): `value` when it does
    /// not read.
    fn from_str(s: &str) -> Result<Self, WriteRefusal> {
        Self::parse_input(s).ok_or(WriteRefusal::Value)
    }
}

impl From<i64> for Decimal {
    fn from(v: i64) -> Self {
        Self {
            m: BigInt::from(v),
            e: 0,
        }
    }
}

impl Decimal {
    /// A binary float, read by its shortest round-trip text, as a JSON
    /// writer spells it (so `0.15` is the decimal 0.15); `None` for a NaN
    /// or an infinity.
    #[must_use]
    pub fn from_f64(x: f64) -> Option<Self> {
        if !x.is_finite() {
            return None;
        }
        Self::parse_input(&format!("{x}"))
    }

    /// The nearest binary float: for display and arithmetic outside the
    /// codec, never for a code.
    #[must_use]
    pub fn to_f64(&self) -> f64 {
        self.to_string().parse().unwrap_or(f64::NAN)
    }
}

fn pow10(n: u32) -> BigInt {
    num_traits::pow(BigInt::from(10), n as usize)
}

/// The fewest bits that hold `codes` codes, `⌈log₂ codes⌉`.
fn bits_for(codes: &BigInt) -> u64 {
    if codes <= &BigInt::one() {
        0
    } else {
        (codes - 1u32).bits()
    }
}

// --- The schema file ---------------------------------------------------------

/// One lint finding in a packed file (core §9.2's E038 to E045).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub code: &'static str,
    /// Where in the file: `""` for its top level, `types.<Name>`, or
    /// `types.<Name>.fields[<i>]`.
    pub at: String,
    pub message: String,
}

/// A field's type, with what its declaration fixes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldKind {
    Int {
        minimum: i64,
        maximum: i64,
        reserved: Vec<&'static str>,
    },
    Fixed {
        minimum: Decimal,
        maximum: Decimal,
        resolution: Decimal,
        reserved: Vec<&'static str>,
        /// `(maximum − minimum) / resolution + 1`.
        values: u128,
    },
    Enum {
        values: Vec<String>,
        escape: bool,
    },
    Bool,
    Counter,
    Time {
        ms: bool,
    },
}

impl FieldKind {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Int { .. } => "int",
            Self::Fixed { .. } => "fixed",
            Self::Enum { .. } => "enum",
            Self::Bool => "bool",
            Self::Counter => "counter",
            Self::Time { .. } => "time",
        }
    }
}

/// One field of a layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub kind: FieldKind,
    /// From 1 to 64.
    pub width: u32,
    pub optional: bool,
}

/// An item of a record's `fields`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Field(Field),
    /// The extension marker, one bit.
    Marker,
}

/// A packed file read and linted: every type its `types` names with a type
/// name, and the layout of each that has no finding.
#[derive(Debug, Clone, Default)]
pub struct PackedDoc {
    /// `None` for a type whose record has a finding.
    pub types: BTreeMap<String, Option<Layout>>,
    pub findings: Vec<Finding>,
}

fn is_type_name(s: &str) -> bool {
    let mut b = s.bytes();
    b.next().is_some_and(|c| c.is_ascii_uppercase()) && b.all(|c| c.is_ascii_alphanumeric())
}

fn is_field_name(s: &str) -> bool {
    let mut b = s.bytes();
    b.next().is_some_and(|c| c.is_ascii_lowercase())
        && b.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

fn is_value_name(s: &str) -> bool {
    let mut b = s.bytes();
    b.next().is_some_and(|c| c.is_ascii_alphabetic())
        && b.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

/// The members a field type takes beside `name`, `type`, `optional` and
/// `description`, and those it needs.
fn members(ty: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match ty {
        "int" => (&["minimum", "maximum", "reserved"], &["minimum", "maximum"]),
        "fixed" => (
            &["minimum", "maximum", "resolution", "reserved"],
            &["minimum", "maximum", "resolution"],
        ),
        "enum" => (&["values", "escape", "width"], &["values"]),
        "counter" => (&["width"], &["width"]),
        "time" => (&["ms"], &[]),
        _ => (&[], &[]),
    }
}

/// The first shape problem of a field object, for its one E038.
fn field_shape(m: &Map<String, Value>, ty: &str) -> Option<String> {
    let (takes, needs) = members(ty);
    for k in m.keys() {
        if !["name", "type", "optional", "description"].contains(&k.as_str())
            && !takes.contains(&k.as_str())
        {
            return Some(format!("a `{ty}` field takes no member {k:?}"));
        }
    }
    for k in std::iter::once(&"name").chain(needs) {
        if !m.contains_key(*k) {
            return Some(format!("a `{ty}` field needs `{k}`"));
        }
    }
    match m.get("name") {
        Some(Value::String(n)) if is_field_name(n) => {}
        Some(v) => return Some(format!("`name` {v} is not [a-z][a-z0-9_]*")),
        None => {}
    }
    let is_bool = |k: &str| m.get(k).is_none_or(Value::is_boolean);
    let is_int = |k: &str| m.get(k).is_none_or(|v| v.as_i64().is_some());
    let is_str = |k: &str| m.get(k).is_none_or(Value::is_string);
    if !is_bool("optional") {
        return Some("`optional` is not a boolean".into());
    }
    if !is_str("description") {
        return Some("`description` is not a string".into());
    }
    let list_of = |k: &str, ok: &dyn Fn(&str) -> bool| match m.get(k) {
        None => true,
        Some(Value::Array(a)) => a.iter().all(|v| v.as_str().is_some_and(ok)),
        Some(_) => false,
    };
    let problem = match ty {
        "int" if !is_int("minimum") || !is_int("maximum") => {
            Some("`minimum` and `maximum` are integers")
        }
        "fixed" if !is_str("minimum") || !is_str("maximum") || !is_str("resolution") => {
            Some("`minimum`, `maximum` and `resolution` are decimal strings")
        }
        "int" | "fixed" if !list_of("reserved", &|s| RESERVED.contains(&s)) => {
            Some("`reserved` lists not_available, error and out_of_range")
        }
        "enum" if !list_of("values", &is_value_name) => {
            Some("`values` is a list of names [A-Za-z][A-Za-z0-9_]*")
        }
        "enum" if !is_bool("escape") => Some("`escape` is not a boolean"),
        "enum" | "counter" if !is_int("width") => Some("`width` is not an integer"),
        "time" if !is_bool("ms") => Some("`ms` is not a boolean"),
        _ => None,
    };
    problem.map(str::to_owned)
}

/// Repeats in a list of names: each one after its first.
fn repeats<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let mut seen = BTreeSet::new();
    names.into_iter().filter(|n| !seen.insert(*n)).collect()
}

fn strings(v: Option<&Value>) -> Vec<&str> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn reserved_of(m: &Map<String, Value>) -> Vec<&'static str> {
    let listed = strings(m.get("reserved"));
    RESERVED
        .iter()
        .copied()
        .filter(|r| listed.contains(r))
        .collect()
}

/// A field's E039, E040 or E041, the first met in cascade 7's order, or
/// its layout.
fn field_of(m: &Map<String, Value>, ty: &str) -> Result<Field, (&'static str, String)> {
    let name = m["name"].as_str().unwrap_or_default().to_owned();
    let optional = m.get("optional").and_then(Value::as_bool).unwrap_or(false);
    let wide = |codes: &BigInt| -> Result<u32, (&'static str, String)> {
        let w = bits_for(codes);
        if (1..=64).contains(&w) {
            Ok(u32::try_from(w).unwrap_or(64))
        } else {
            Err((
                "E041",
                format!("{codes} codes need {w} bits; a field is 1 to 64"),
            ))
        }
    };
    let (kind, width) = match ty {
        "int" => {
            let (lo, hi) = (
                m["minimum"].as_i64().unwrap_or(0),
                m["maximum"].as_i64().unwrap_or(0),
            );
            if lo > hi {
                return Err(("E039", format!("`minimum` {lo} is above `maximum` {hi}")));
            }
            let reserved = reserved_of(m);
            let codes = BigInt::from(hi) - BigInt::from(lo) + BigInt::from(1 + reserved.len());
            let width = wide(&codes)?;
            (
                FieldKind::Int {
                    minimum: lo,
                    maximum: hi,
                    reserved,
                },
                width,
            )
        }
        "fixed" => {
            let read = |k: &str| {
                let s = m[k].as_str().unwrap_or_default();
                Decimal::parse(s).ok_or_else(|| ("E040", format!("`{k}` {s:?} is not a decimal")))
            };
            let (lo, hi, res) = (read("minimum")?, read("maximum")?, read("resolution")?);
            if lo > hi {
                return Err(("E039", format!("`minimum` {lo} is above `maximum` {hi}")));
            }
            if !res.is_positive() {
                return Err(("E040", format!("`resolution` {res} is not above 0")));
            }
            let Some(steps) = hi.sub(&lo).div_exact(&res) else {
                return Err((
                    "E040",
                    format!("`resolution` {res} does not divide {hi} − {lo}"),
                ));
            };
            let reserved = reserved_of(m);
            let values: BigInt = steps + BigInt::one();
            let width = wide(&(&values + BigInt::from(reserved.len())))?;
            (
                FieldKind::Fixed {
                    minimum: lo,
                    maximum: hi,
                    resolution: res,
                    reserved,
                    values: values.to_u128().unwrap_or(u128::MAX),
                },
                width,
            )
        }
        "enum" => {
            let values: Vec<String> = strings(m.get("values"))
                .into_iter()
                .map(str::to_owned)
                .collect();
            if values.is_empty() {
                return Err(("E039", "an enum with no value".into()));
            }
            let escape = m.get("escape").and_then(Value::as_bool).unwrap_or(false);
            let need = bits_for(&BigInt::from(values.len() + usize::from(escape)));
            let width = match m.get("width").and_then(Value::as_i64) {
                Some(w) if !(1..=64).contains(&w) || u64::try_from(w).unwrap_or(0) < need => {
                    return Err((
                        "E041",
                        format!("`width` {w}: the enum needs {need} bits, and a field is 1 to 64"),
                    ));
                }
                Some(w) => u32::try_from(w).unwrap_or(64),
                None => wide(&BigInt::from(values.len() + usize::from(escape)))?,
            };
            (FieldKind::Enum { values, escape }, width)
        }
        "bool" => (FieldKind::Bool, 1),
        "counter" => {
            let w = m["width"].as_i64().unwrap_or(0);
            if !(1..=64).contains(&w) {
                return Err(("E041", format!("a counter's `width` {w} is not 1 to 64")));
            }
            (FieldKind::Counter, u32::try_from(w).unwrap_or(64))
        }
        "time" => {
            let ms = m.get("ms").and_then(Value::as_bool).unwrap_or(false);
            (FieldKind::Time { ms }, if ms { 42 } else { 32 })
        }
        _ => unreachable!("checked by E043"),
    };
    Ok(Field {
        name,
        kind,
        width,
        optional,
    })
}

impl PackedDoc {
    /// Reads and lints a packed document (core §7.6, §9.2's E038 to E045 and
    /// cascade 7).
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn read(doc: &Value) -> Self {
        let mut findings = Vec::new();
        let mut find = |code: &'static str, at: String, message: String| {
            findings.push(Finding { code, at, message });
        };
        let Value::Object(top) = doc else {
            find(
                "E038",
                String::new(),
                "the top level is not an object".into(),
            );
            return Self {
                types: BTreeMap::new(),
                findings,
            };
        };
        let mut types: BTreeMap<String, Option<Layout>> = BTreeMap::new();
        let problem = top
            .keys()
            .find(|k| *k != "types" && *k != "description")
            .map(|k| format!("the top level takes no member {k:?}"))
            .or_else(|| {
                (!top.get("description").is_none_or(Value::is_string))
                    .then(|| "`description` is not a string".to_owned())
            })
            .or_else(|| match top.get("types") {
                Some(Value::Object(_)) => None,
                Some(_) => Some("`types` is not an object".into()),
                None => Some("there is no `types`".into()),
            });
        if let Some(p) = problem {
            find("E038", String::new(), p);
        }
        let Some(Value::Object(defs)) = top.get("types") else {
            return Self { types, findings };
        };
        for (name, record) in defs {
            let at = format!("types.{name}");
            if !is_type_name(name) {
                find(
                    "E038",
                    at,
                    format!("{name:?} is not a type name [A-Z][A-Za-z0-9]*"),
                );
                continue;
            }
            let mut clean = true;
            let Value::Object(rec) = record else {
                find("E038", at, "a record is an object".into());
                types.insert(name.clone(), None);
                continue;
            };
            let problem = rec
                .keys()
                .find(|k| *k != "fields" && *k != "description")
                .map(|k| format!("a record takes no member {k:?}"))
                .or_else(|| {
                    (!rec.get("description").is_none_or(Value::is_string))
                        .then(|| "`description` is not a string".to_owned())
                })
                .or_else(|| match rec.get("fields") {
                    Some(Value::Array(_)) => None,
                    Some(_) => Some("`fields` is not a list".into()),
                    None => Some("a record needs `fields`".into()),
                });
            if let Some(p) = problem {
                find("E038", at.clone(), p);
                clean = false;
            }
            let Some(Value::Array(list)) = rec.get("fields") else {
                types.insert(name.clone(), None);
                continue;
            };
            // Each item: a marker, a field read whole, or a field with a
            // finding (which takes no part in E042 and E045).
            let mut items: Vec<Option<Item>> = Vec::new();
            let mut sound: Vec<(usize, &str, bool)> = Vec::new();
            let mut objects = 0;
            for (i, item) in list.iter().enumerate() {
                let fat = format!("{at}.fields[{i}]");
                if item.as_str() == Some(MARKER) {
                    items.push(Some(Item::Marker));
                    continue;
                }
                let Value::Object(f) = item else {
                    find(
                        "E038",
                        fat,
                        "an item is a field object or the marker \"...\"".into(),
                    );
                    items.push(None);
                    clean = false;
                    continue;
                };
                objects += 1;
                let ty = match f.get("type") {
                    Some(Value::String(t)) if FIELD_TYPES.contains(&t.as_str()) => t.as_str(),
                    Some(Value::String(t)) => {
                        find(
                            "E043",
                            fat,
                            format!("`type` {t:?} names no field type of core §7.6"),
                        );
                        items.push(None);
                        clean = false;
                        continue;
                    }
                    _ => {
                        find("E038", fat, "a field needs `type`, a string".into());
                        items.push(None);
                        clean = false;
                        continue;
                    }
                };
                if let Some(p) = field_shape(f, ty) {
                    find("E038", fat, p);
                    items.push(None);
                    clean = false;
                    continue;
                }
                let name = f["name"].as_str().unwrap_or_default();
                let optional = f.get("optional").and_then(Value::as_bool).unwrap_or(false);
                sound.push((i, name, optional));
                for key in ["values", "reserved"] {
                    for r in repeats(strings(f.get(key))) {
                        find("E042", fat.clone(), format!("{r:?} is repeated in `{key}`"));
                        clean = false;
                    }
                }
                match field_of(f, ty) {
                    Ok(field) => items.push(Some(Item::Field(field))),
                    Err((code, message)) => {
                        find(code, fat, message);
                        items.push(None);
                        clean = false;
                    }
                }
            }
            for r in repeats(sound.iter().map(|(_, n, _)| *n)) {
                find(
                    "E042",
                    at.clone(),
                    format!("the field name {r:?} is repeated"),
                );
                clean = false;
            }
            let first_marker = list.iter().position(|v| v.as_str() == Some(MARKER));
            for (i, item) in list.iter().enumerate() {
                if item.as_str() == Some(MARKER) && i > 0 && list[i - 1].as_str() == Some(MARKER) {
                    find(
                        "E045",
                        format!("{at}.fields[{i}]"),
                        "a marker right after another".into(),
                    );
                    clean = false;
                }
            }
            if let Some(m0) = first_marker {
                for (i, n, optional) in &sound {
                    if *i > m0 && !optional {
                        find(
                            "E045",
                            format!("{at}.fields[{i}]"),
                            format!("{n:?} is after a marker, and is not optional"),
                        );
                        clean = false;
                    }
                }
            }
            if objects == 0 {
                find("E039", at.clone(), "a record with no field".into());
                clean = false;
            }
            let layout = if clean {
                items
                    .into_iter()
                    .collect::<Option<Vec<Item>>>()
                    .map(Layout::new)
            } else {
                None
            };
            types.insert(name.clone(), layout);
        }
        Self { types, findings }
    }
}

// --- The layout ---------------------------------------------------------------

/// A type's layout: its items, its layout form and its tag (core §7.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub items: Vec<Item>,
    form: Value,
    sha256: [u8; 32],
}

/// An item's width and start, as `describe` reports them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    /// `None` for a marker.
    pub name: Option<String>,
    pub width: u32,
    pub optional: bool,
    /// The bit where the item starts, `None` once an optional field or a
    /// marker before it makes it vary.
    pub offset: Option<u64>,
}

impl Field {
    fn form(&self) -> Value {
        let mut f = Map::new();
        f.insert("type".into(), self.kind.as_str().into());
        f.insert("width".into(), self.width.into());
        f.insert("optional".into(), self.optional.into());
        match &self.kind {
            FieldKind::Int {
                minimum,
                maximum,
                reserved,
            } => {
                f.insert("minimum".into(), (*minimum).into());
                f.insert("maximum".into(), (*maximum).into());
                f.insert("reserved".into(), reserved.clone().into());
            }
            FieldKind::Fixed {
                minimum,
                maximum,
                resolution,
                reserved,
                ..
            } => {
                f.insert("minimum".into(), minimum.to_string().into());
                f.insert("maximum".into(), maximum.to_string().into());
                f.insert("resolution".into(), resolution.to_string().into());
                f.insert("reserved".into(), reserved.clone().into());
            }
            FieldKind::Enum { values, escape } => {
                f.insert("escape".into(), (*escape).into());
                if !escape {
                    f.insert("values".into(), values.len().into());
                }
            }
            FieldKind::Time { ms } => {
                f.insert("ms".into(), (*ms).into());
            }
            FieldKind::Bool | FieldKind::Counter => {}
        }
        Value::Object(f)
    }

    fn top(&self) -> u64 {
        top_code(self.width)
    }

    fn reserved_code(&self, i: usize) -> u64 {
        self.top() - i as u64
    }
}

/// The all-ones code of a width from 1 to 64: the first reserved code, an
/// enum's escape.
#[must_use]
pub const fn top_code(width: u32) -> u64 {
    if width >= 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    }
}

/// The reserved code `name` takes in a field of `width` whose `reserved`
/// lists `declared` (core §7.6: the top codes, from all ones down, in
/// [`RESERVED`]'s order), `None` when it does not declare it.
#[must_use]
pub fn reserved_code(width: u32, declared: &[&str], name: &str) -> Option<u64> {
    RESERVED
        .iter()
        .filter(|r| declared.contains(r))
        .position(|r| *r == name)
        .map(|i| top_code(width) - i as u64)
}

/// The deployment epoch the reference reads and writes `time` fields
/// under while nothing carries one (core §7.6: "This version carries it
/// nowhere"; #980's catalog will): 0, the Unix epoch, so a `time` field's
/// 32 bits are Unix seconds, to 2106.
pub const DEFAULT_EPOCH: i64 = 0;

/// Why a writer refused a value (core §7.6, the vectors' tokens).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteRefusal {
    /// A field that is not optional is not given.
    Missing,
    /// A member the type does not have.
    Unknown,
    /// A value of the wrong JSON type, a name not declared, or a decimal
    /// that does not read.
    Value,
    /// A value that has no code.
    Range,
    /// A time field, and no deployment epoch.
    Epoch,
}

impl WriteRefusal {
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Unknown => "unknown",
            Self::Value => "value",
            Self::Range => "range",
            Self::Epoch => "epoch",
        }
    }

    /// The refusal in words, its token first.
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            Self::Missing => "missing: a field that is not optional is not given",
            Self::Unknown => "unknown: a member the type does not have",
            Self::Value => "value: the wrong JSON type, a name not declared, or no decimal",
            Self::Range => "range: a value that has no code",
            Self::Epoch => "epoch: a time field, and no deployment epoch",
        }
    }
}

impl std::fmt::Display for WriteRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason())
    }
}

impl std::error::Error for WriteRefusal {}

/// Why a reader refused a record (core §7.6's order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadRefusal {
    /// CH's tag is not the type's.
    Tag,
    /// The record ends before a bit its layout needs, or holds a byte after
    /// its pad.
    Length,
    /// A field's code is unused.
    Code,
    /// A marker read as 1 is followed by no present field.
    Extension,
    /// A bit of the pad is 1.
    Pad,
    /// A time field, and no deployment epoch: the reader's input, not the
    /// record's fault.
    Epoch,
}

impl ReadRefusal {
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Self::Tag => "tag",
            Self::Length => "length",
            Self::Code => "code",
            Self::Extension => "extension",
            Self::Pad => "pad",
            Self::Epoch => "epoch",
        }
    }

    /// The refusal in words, its token first.
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            Self::Tag => "tag: the envelope's tag is not the type's",
            Self::Length => "length: the record is not as long as the layout reads it",
            Self::Code => "code: a field's code is unused",
            Self::Extension => "extension: a marker read as 1, and no field present after it",
            Self::Pad => "pad: a bit of the pad is 1",
            Self::Epoch => "epoch: a time field, and no deployment epoch",
        }
    }
}

impl std::fmt::Display for ReadRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason())
    }
}

impl std::error::Error for ReadRefusal {}

/// A record as a writer wrote it: its bytes, and how many of their bits
/// are the record's (the rest is the pad).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub bytes: Vec<u8>,
    pub bits: u64,
}

impl Record {
    /// The pad's zero bits, fewer than 8.
    #[must_use]
    pub fn pad(&self) -> u64 {
        self.bytes.len() as u64 * 8 - self.bits
    }

    /// The record's bits before the pad, as `0` and `1`.
    #[must_use]
    pub fn bit_string(&self) -> String {
        (0..self.bits)
            .map(|i| {
                let b = (self.bytes[(i / 8) as usize] >> (7 - i % 8)) & 1;
                if b == 1 { '1' } else { '0' }
            })
            .collect()
    }
}

/// A record being written, most significant bit first, with no padding
/// between fields (core §7.6's layout). [`BitWriter::finish`] adds the pad.
#[derive(Debug, Clone, Default)]
pub struct BitWriter {
    bytes: Vec<u8>,
    len: u64,
}

impl BitWriter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Writes the low `width` bits of `v`, the most significant first: a
    /// code in its width (so a counter's count modulo 2^`width`).
    pub fn push(&mut self, v: u64, width: u32) {
        for i in (0..width).rev() {
            if self.len.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if (v >> i) & 1 == 1 {
                let last = self.bytes.len() - 1;
                self.bytes[last] |= 0x80 >> (self.len % 8);
            }
            self.len += 1;
        }
    }

    /// Writes one bit: a presence bit, or an extension marker.
    pub fn bit(&mut self, set: bool) {
        self.push(u64::from(set), 1);
    }

    /// The record: its bits, then the zero pad up to a whole byte.
    #[must_use]
    pub fn finish(self) -> Record {
        Record {
            bytes: self.bytes,
            bits: self.len,
        }
    }
}

/// A record being read under its layout, with core §7.6's length and pad
/// checks.
#[derive(Debug, Clone)]
pub struct BitReader<'a> {
    bytes: &'a [u8],
    pos: u64,
}

impl<'a> BitReader<'a> {
    #[must_use]
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn total(&self) -> u64 {
        self.bytes.len() as u64 * 8
    }

    /// The next `width` bits, as an unsigned integer: `length` when the
    /// record ends before them.
    ///
    /// # Errors
    /// [`ReadRefusal::Length`].
    pub fn take(&mut self, width: u32) -> Result<u64, ReadRefusal> {
        if self.pos + u64::from(width) > self.total() {
            return Err(ReadRefusal::Length);
        }
        let mut v = 0u64;
        for _ in 0..width {
            let b = (self.bytes[(self.pos / 8) as usize] >> (7 - self.pos % 8)) & 1;
            v = (v << 1) | u64::from(b);
            self.pos += 1;
        }
        Ok(v)
    }

    /// The next bit: a presence bit, or an extension marker.
    ///
    /// # Errors
    /// [`ReadRefusal::Length`].
    pub fn bit(&mut self) -> Result<bool, ReadRefusal> {
        self.take(1).map(|b| b == 1)
    }

    /// After a marker read as 1 that ends the reader's layout: the rest is
    /// a later revision's, and holds at least one bit.
    ///
    /// # Errors
    /// [`ReadRefusal::Length`].
    pub fn later(&self) -> Result<(), ReadRefusal> {
        if self.total() <= self.pos {
            Err(ReadRefusal::Length)
        } else {
            Ok(())
        }
    }

    /// The record's end: no byte after its pad (`length`), then every pad
    /// bit 0 (`pad`).
    ///
    /// # Errors
    /// [`ReadRefusal::Length`], then [`ReadRefusal::Pad`].
    pub fn end(mut self) -> Result<(), ReadRefusal> {
        if self.bytes.len() as u64 != self.pos.div_ceil(8) {
            return Err(ReadRefusal::Length);
        }
        while self.pos < self.total() {
            if self.take(1)? == 1 {
                return Err(ReadRefusal::Pad);
            }
        }
        Ok(())
    }
}

/// A JSON number or a decimal string, as a writer's decimal (`value` when
/// it does not read).
///
/// # Errors
/// [`WriteRefusal::Value`].
pub fn json_decimal(v: &Value) -> Result<Decimal, WriteRefusal> {
    match v {
        Value::String(s) => Decimal::parse_input(s).ok_or(WriteRefusal::Value),
        // serde_json writes a float in its shortest round-trip text (ryu),
        // and an integer as its digits.
        Value::Number(n) => Decimal::parse_input(&n.to_string()).ok_or(WriteRefusal::Value),
        _ => Err(WriteRefusal::Value),
    }
}

/// A JSON integer. A float that is integral and beyond 64 bits is how a
/// JSON reader without big integers holds integer text past them: such a
/// value is out of range (`range`), never of the wrong type (`value`).
///
/// # Errors
/// [`WriteRefusal::Value`] or [`WriteRefusal::Range`].
pub fn json_integer(v: &Value) -> Result<i128, WriteRefusal> {
    match v {
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i128::from(i))
            } else if let Some(u) = n.as_u64() {
                Ok(i128::from(u))
            } else {
                // Every float this large is an integer.
                if n.as_f64().unwrap_or(0.0).abs() >= 2f64.powi(63) {
                    Err(WriteRefusal::Range)
                } else {
                    Err(WriteRefusal::Value)
                }
            }
        }
        _ => Err(WriteRefusal::Value),
    }
}

/// An `int` field's code for `x`: `x − minimum` (`range` outside
/// `minimum` to `maximum`).
///
/// # Errors
/// [`WriteRefusal::Range`].
pub fn int_code(x: i128, minimum: i64, maximum: i64) -> Result<u64, WriteRefusal> {
    if x < i128::from(minimum) || x > i128::from(maximum) {
        return Err(WriteRefusal::Range);
    }
    u64::try_from(x - i128::from(minimum)).map_err(|_| WriteRefusal::Range)
}

/// An `int` field's value for a code that is no reserved one: `minimum +
/// code` (`code` past `maximum`).
///
/// # Errors
/// [`ReadRefusal::Code`].
pub fn int_value(code: u64, minimum: i64, maximum: i64) -> Result<i128, ReadRefusal> {
    let span = i128::from(maximum) - i128::from(minimum);
    if i128::from(code) > span {
        return Err(ReadRefusal::Code);
    }
    Ok(i128::from(minimum) + i128::from(code))
}

/// A `fixed` field's code for `x`: round((x − `minimum`) / `resolution`),
/// a tie to the even code, in exact decimal arithmetic (`range` when it is
/// not one of the field's `values` codes).
///
/// # Errors
/// [`WriteRefusal::Range`].
pub fn fixed_code(
    x: &Decimal,
    minimum: &Decimal,
    resolution: &Decimal,
    values: u128,
) -> Result<u64, WriteRefusal> {
    match x.sub(minimum).div_round_even(resolution).to_u128() {
        Some(c) if c < values => u64::try_from(c).map_err(|_| WriteRefusal::Range),
        _ => Err(WriteRefusal::Range),
    }
}

/// A `fixed` field's value for a code that is no reserved one: `minimum +
/// code × resolution` (`code` from `values` up).
///
/// # Errors
/// [`ReadRefusal::Code`].
pub fn fixed_value(
    code: u64,
    minimum: &Decimal,
    resolution: &Decimal,
    values: u128,
) -> Result<Decimal, ReadRefusal> {
    if u128::from(code) >= values {
        return Err(ReadRefusal::Code);
    }
    Ok(minimum.add(&resolution.mul_int(&BigInt::from(code))))
}

/// A `fixed` field's declaration, read once: what generated code keeps in
/// a static, so its decimals are parsed once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fixed {
    pub minimum: Decimal,
    pub resolution: Decimal,
    /// `(maximum − minimum) / resolution + 1`.
    pub values: u128,
}

impl Fixed {
    /// # Panics
    /// When `minimum` or `resolution` is not a decimal string: a
    /// declaration the lints passed never is.
    #[must_use]
    pub fn new(minimum: &str, resolution: &str, values: u128) -> Self {
        Self {
            minimum: Decimal::parse(minimum).expect("a linted decimal"),
            resolution: Decimal::parse(resolution).expect("a linted decimal"),
            values,
        }
    }

    /// [`fixed_code`].
    ///
    /// # Errors
    /// [`WriteRefusal::Range`].
    pub fn code(&self, x: &Decimal) -> Result<u64, WriteRefusal> {
        fixed_code(x, &self.minimum, &self.resolution, self.values)
    }

    /// [`fixed_value`].
    ///
    /// # Errors
    /// [`ReadRefusal::Code`].
    pub fn value(&self, code: u64) -> Result<Decimal, ReadRefusal> {
        fixed_value(code, &self.minimum, &self.resolution, self.values)
    }
}

/// A `time` field's code for an instant `t`, in milliseconds since the
/// Unix epoch, under the deployment epoch in seconds: its seconds since
/// the epoch, then with `ms` its milliseconds in 10 bits (`range` outside
/// the epoch's 2^32 seconds, `epoch` with none).
///
/// # Errors
/// [`WriteRefusal::Epoch`] or [`WriteRefusal::Range`].
pub fn time_code(t: i128, epoch: Option<i64>, ms: bool) -> Result<u64, WriteRefusal> {
    let e = epoch.ok_or(WriteRefusal::Epoch)?;
    let delta = t - 1000 * i128::from(e);
    let (s, m) = (delta.div_euclid(1000), delta.rem_euclid(1000));
    let s = u32::try_from(s).map_err(|_| WriteRefusal::Range)?;
    let m = u64::try_from(m).unwrap_or(0);
    Ok(if ms {
        (u64::from(s) << 10) | m
    } else {
        u64::from(s)
    })
}

/// A `time` field's instant for a code, in milliseconds since the Unix
/// epoch (`code` for a millisecond code from 1000 up, then `epoch` with
/// none).
///
/// # Errors
/// [`ReadRefusal::Code`], then [`ReadRefusal::Epoch`].
pub fn time_value(code: u64, epoch: Option<i64>, ms: bool) -> Result<i128, ReadRefusal> {
    let (s, m) = if ms {
        (code >> 10, code & 0x3ff)
    } else {
        (code, 0)
    };
    if m >= 1000 {
        return Err(ReadRefusal::Code);
    }
    let e = epoch.ok_or(ReadRefusal::Epoch)?;
    Ok(1000 * i128::from(e) + 1000 * i128::from(s) + i128::from(m))
}

impl Layout {
    fn new(items: Vec<Item>) -> Self {
        let mut form = Vec::new();
        for it in &items {
            match it {
                Item::Marker => {
                    form.push(Value::from(MARKER));
                    break;
                }
                Item::Field(f) => form.push(f.form()),
            }
        }
        let form = Value::Array(form);
        let sha256 = Sha256::digest(crate::schema::jcs(&form)).into();
        Self {
            items,
            form,
            sha256,
        }
    }

    /// The layout form: the root's wire, without names (core §7.6).
    #[must_use]
    pub fn form(&self) -> &Value {
        &self.form
    }

    /// The sha256 of the layout form's JCS bytes, in lowercase hex.
    #[must_use]
    pub fn sha256_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for b in self.sha256 {
            let _ = write!(s, "{b:02x}");
        }
        s
    }

    /// The tag CH carries: the low four bits of the hash's last byte.
    #[must_use]
    pub fn tag(&self) -> u8 {
        self.sha256[31] & 0x0f
    }

    /// Each item's width and start.
    #[must_use]
    pub fn slots(&self) -> Vec<Slot> {
        let (mut pos, mut fixed) = (0u64, true);
        let mut out = Vec::new();
        for it in &self.items {
            let offset = fixed.then_some(pos);
            match it {
                Item::Marker => {
                    out.push(Slot {
                        name: None,
                        width: 1,
                        optional: false,
                        offset,
                    });
                    fixed = false;
                }
                Item::Field(f) => {
                    out.push(Slot {
                        name: Some(f.name.clone()),
                        width: f.width,
                        optional: f.optional,
                        offset,
                    });
                    if f.optional {
                        fixed = false;
                    } else {
                        pos += u64::from(f.width);
                    }
                }
            }
        }
        out
    }

    /// The shortest record this revision writes, before the pad: every
    /// optional field absent and the first marker 0.
    #[must_use]
    pub fn min_bits(&self) -> u64 {
        let mut n = 0;
        for it in &self.items {
            match it {
                Item::Marker => return n + 1,
                Item::Field(f) if f.optional => n += 1,
                Item::Field(f) => n += u64::from(f.width),
            }
        }
        n
    }

    /// The longest: every field present, every marker before one 1, and
    /// the last marker 0.
    #[must_use]
    pub fn max_bits(&self) -> u64 {
        self.items
            .iter()
            .map(|it| match it {
                Item::Marker => 1,
                Item::Field(f) => u64::from(f.width) + u64::from(f.optional),
            })
            .sum()
    }

    /// Its fields, the extension markers left out.
    pub fn fields(&self) -> impl Iterator<Item = &Field> {
        self.items.iter().filter_map(|it| match it {
            Item::Field(f) => Some(f),
            Item::Marker => None,
        })
    }

    /// Writes a value, the JSON object core §7.6's reading spells, with the
    /// deployment epoch in seconds where a time field needs one.
    ///
    /// # Errors
    /// The writer's refusal: [`WriteRefusal`].
    pub fn write(&self, value: &Value, epoch: Option<i64>) -> Result<Record, WriteRefusal> {
        let Value::Object(obj) = value else {
            return Err(WriteRefusal::Value);
        };
        if obj
            .keys()
            .any(|k| !self.fields().any(|f| f.name == k.as_str()))
        {
            return Err(WriteRefusal::Unknown);
        }
        let mut bits = BitWriter::new();
        for (i, it) in self.items.iter().enumerate() {
            match it {
                Item::Marker => {
                    let later = self.items[i + 1..].iter().any(|x| match x {
                        Item::Field(f) => obj.contains_key(&f.name),
                        Item::Marker => false,
                    });
                    bits.bit(later);
                    if !later {
                        break;
                    }
                }
                Item::Field(f) => match obj.get(&f.name) {
                    None if f.optional => bits.bit(false),
                    None => return Err(WriteRefusal::Missing),
                    Some(v) => {
                        let code = f.code(v, epoch)?;
                        if f.optional {
                            bits.bit(true);
                        }
                        bits.push(code, f.width);
                    }
                },
            }
        }
        Ok(bits.finish())
    }

    /// Reads a record under the tag its envelope's CH carries, refusing it
    /// for the first of core §7.6's checks that applies.
    ///
    /// # Errors
    /// The reader's refusal: [`ReadRefusal`].
    pub fn read(&self, tag: u8, bytes: &[u8], epoch: Option<i64>) -> Result<Value, ReadRefusal> {
        let mut out = Map::new();
        self.walk(tag, bytes, |f, code| {
            out.insert(f.name.clone(), f.value(code, epoch)?);
            Ok(())
        })?;
        Ok(Value::Object(out))
    }

    /// [`Layout::read`]'s checks, with no value made and no epoch needed: a
    /// writer's or a reader's admission of a record (#979 B2's runtime).
    ///
    /// # Errors
    /// The reader's refusal, never [`ReadRefusal::Epoch`].
    pub fn check(&self, tag: u8, bytes: &[u8]) -> Result<(), ReadRefusal> {
        self.walk(tag, bytes, |f, code| f.check_code(code))
    }

    /// Core §7.6's reading, in its order, each present field's code handed
    /// to `each` where it is read.
    fn walk(
        &self,
        tag: u8,
        bytes: &[u8],
        mut each: impl FnMut(&Field, u64) -> Result<(), ReadRefusal>,
    ) -> Result<(), ReadRefusal> {
        if tag != self.tag() {
            return Err(ReadRefusal::Tag);
        }
        let mut r = BitReader::new(bytes);
        let mut set_marker = false;
        let mut present_after = false;
        let last = self.items.len().saturating_sub(1);
        for (i, it) in self.items.iter().enumerate() {
            match it {
                Item::Marker => {
                    if !r.bit()? {
                        break;
                    }
                    if i == last {
                        // A later revision's bits: at least one of them.
                        return r.later();
                    }
                    set_marker = true;
                    present_after = false;
                }
                Item::Field(f) => {
                    if f.optional && !r.bit()? {
                        continue;
                    }
                    let code = r.take(f.width)?;
                    each(f, code)?;
                    present_after = true;
                }
            }
        }
        if set_marker && !present_after {
            return Err(ReadRefusal::Extension);
        }
        r.end()
    }
}

impl Field {
    fn code(&self, v: &Value, epoch: Option<i64>) -> Result<u64, WriteRefusal> {
        let reserved = |list: &[&'static str], s: &str| {
            reserved_code(self.width, list, s).ok_or(WriteRefusal::Value)
        };
        match &self.kind {
            FieldKind::Int {
                minimum,
                maximum,
                reserved: res,
            } => {
                if let Value::String(s) = v
                    && RESERVED.contains(&s.as_str())
                {
                    return reserved(res, s);
                }
                int_code(json_integer(v)?, *minimum, *maximum)
            }
            FieldKind::Fixed {
                minimum,
                resolution,
                reserved: res,
                values,
                ..
            } => {
                if let Value::String(s) = v
                    && RESERVED.contains(&s.as_str())
                {
                    return reserved(res, s);
                }
                fixed_code(&json_decimal(v)?, minimum, resolution, *values)
            }
            FieldKind::Enum { values, escape } => match v {
                Value::Null if *escape => Ok(self.top()),
                Value::String(s) => values
                    .iter()
                    .position(|x| x == s)
                    .map(|i| i as u64)
                    .ok_or(WriteRefusal::Value),
                _ => Err(WriteRefusal::Value),
            },
            FieldKind::Bool => v.as_bool().map(u64::from).ok_or(WriteRefusal::Value),
            FieldKind::Counter => {
                let c = json_integer(v)?;
                u64::try_from(c).map_err(|_| WriteRefusal::Range)
            }
            FieldKind::Time { ms } => time_code(json_integer(v)?, epoch, *ms),
        }
    }

    /// The reserved code's name, when `code` is one this field declares.
    fn reserved_name(&self, code: u64) -> Option<&'static str> {
        let list = match &self.kind {
            FieldKind::Int { reserved, .. } | FieldKind::Fixed { reserved, .. } => reserved,
            _ => return None,
        };
        list.iter()
            .enumerate()
            .find(|(i, _)| code == self.reserved_code(*i))
            .map(|(_, r)| *r)
    }

    fn value(&self, code: u64, epoch: Option<i64>) -> Result<Value, ReadRefusal> {
        if let Some(r) = self.reserved_name(code) {
            return Ok(Value::from(r));
        }
        match &self.kind {
            FieldKind::Int {
                minimum, maximum, ..
            } => {
                let v = int_value(code, *minimum, *maximum)?;
                Ok(i64::try_from(v).map_or(Value::Null, Value::from))
            }
            FieldKind::Fixed {
                minimum,
                resolution,
                values,
                ..
            } => fixed_value(code, minimum, resolution, *values)
                .map(|x| Value::String(x.to_string())),
            FieldKind::Enum { values, escape } => match usize::try_from(code) {
                Ok(i) if i < values.len() => Ok(Value::String(values[i].clone())),
                _ if *escape => Ok(Value::Null),
                _ => Err(ReadRefusal::Code),
            },
            FieldKind::Bool => Ok(Value::Bool(code == 1)),
            FieldKind::Counter => Ok(Value::from(code)),
            FieldKind::Time { ms } => {
                let t = time_value(code, epoch, *ms)?;
                Ok(i64::try_from(t).map_or(Value::Null, Value::from))
            }
        }
    }

    /// Whether `code` is one this field reads: [`Field::value`]'s `code`
    /// check alone.
    fn check_code(&self, code: u64) -> Result<(), ReadRefusal> {
        if self.reserved_name(code).is_some() {
            return Ok(());
        }
        let used = match &self.kind {
            FieldKind::Int {
                minimum, maximum, ..
            } => int_value(code, *minimum, *maximum).is_ok(),
            FieldKind::Fixed { values, .. } => u128::from(code) < *values,
            FieldKind::Enum { values, escape } => *escape || code < values.len() as u64,
            FieldKind::Bool | FieldKind::Counter => true,
            FieldKind::Time { ms } => !*ms || code & 0x3ff < 1000,
        };
        if used { Ok(()) } else { Err(ReadRefusal::Code) }
    }
}

/// One type's layout in a packed document (an artifact's data): `Err`
/// names why there is none, a type the document does not define or one
/// whose declaration has a finding.
///
/// # Errors
/// The reason, in words.
pub fn layout_in(doc: &Value, name: &str) -> Result<Layout, String> {
    let read = PackedDoc::read(doc);
    match read.types.get(name) {
        Some(Some(layout)) => Ok(layout.clone()),
        Some(None) => Err(format!(
            "packed:{name}'s declaration does not read ({})",
            read.findings
                .iter()
                .map(|f| f.code)
                .collect::<Vec<_>>()
                .join(", ")
        )),
        None => Err(format!("the packed document defines no types/{name}")),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn codes(doc: &Value) -> Vec<&'static str> {
        let mut c: Vec<_> = PackedDoc::read(doc)
            .findings
            .iter()
            .map(|f| f.code)
            .collect();
        c.sort_unstable();
        c
    }

    #[test]
    fn decimals_read_exactly_and_write_shortest() {
        for (s, want) in [
            ("0", "0"),
            ("-0", "0"),
            ("2.00", "2"),
            ("-40", "-40"),
            ("0.10", "0.1"),
            ("1013.2", "1013.2"),
        ] {
            assert_eq!(Decimal::parse(s).unwrap().to_string(), want, "{s}");
        }
        for bad in ["", "01", "1.", ".5", "+1", "1e3", "1,5", "--1"] {
            assert!(Decimal::parse(bad).is_none(), "{bad}");
        }
        assert_eq!(Decimal::parse_input("5e-1").unwrap().to_string(), "0.5");
        assert_eq!(Decimal::parse_input("1.5E+2").unwrap().to_string(), "150");
        assert_eq!(
            Decimal::parse_input("1e21").unwrap().to_string(),
            "1000000000000000000000"
        );
        assert!(Decimal::parse_input("1e999999").is_none());
    }

    #[test]
    fn ties_go_to_the_even_code() {
        let half = Decimal::parse("0.5").unwrap();
        let r = |s: &str| {
            Decimal::parse_input(s)
                .unwrap()
                .div_round_even(&half)
                .to_string()
        };
        assert_eq!(r("0.25"), "0");
        assert_eq!(r("0.75"), "2");
        assert_eq!(r("-0.25"), "0");
        assert_eq!(r("-0.75"), "-2");
        assert_eq!(r("0.2500000000000000000000000001"), "1");
    }

    #[test]
    fn the_report_sensor_record_is_40_bits() {
        let doc = json!({"types": {"Sensor": {"fields": [
            {"name": "temperature", "type": "fixed", "minimum": "-40", "maximum": "85", "resolution": "0.1"},
            {"name": "humidity", "type": "fixed", "minimum": "0", "maximum": "100", "resolution": "0.5"},
            {"name": "pressure", "type": "fixed", "minimum": "300", "maximum": "1100", "resolution": "0.1"},
            {"name": "battery", "type": "fixed", "minimum": "2.00", "maximum": "4.20", "resolution": "0.01"}
        ]}}});
        let p = PackedDoc::read(&doc);
        assert!(p.findings.is_empty(), "{:?}", p.findings);
        let l = p.types["Sensor"].clone().unwrap();
        assert_eq!((l.min_bits(), l.max_bits()), (40, 40));
        let rec = l
            .write(
                &json!({"temperature": "21.5", "humidity": "43.5", "pressure": "1013.2", "battery": "3.70"}),
                None,
            )
            .unwrap();
        assert_eq!(rec.bytes, [0x4c, 0xea, 0xfb, 0xdc, 0xaa]);
        assert_eq!(rec.pad(), 0);
        let back = l.read(l.tag(), &rec.bytes, None).unwrap();
        assert_eq!(back["battery"], "3.7");
        assert_eq!(l.read(l.tag() ^ 1, &rec.bytes, None), Err(ReadRefusal::Tag));
        assert_eq!(
            l.read(l.tag(), &rec.bytes[..4], None),
            Err(ReadRefusal::Length)
        );
    }

    #[test]
    fn lints_follow_cascade_seven() {
        assert_eq!(codes(&json!([])), ["E038"]);
        assert_eq!(codes(&json!({"types": {}, "x": 1})), ["E038"]);
        assert_eq!(codes(&json!({"types": {"bad": {"fields": []}}})), ["E038"]);
        assert_eq!(codes(&json!({"types": {"A": {"fields": []}}})), ["E039"]);
        assert_eq!(
            codes(&json!({"types": {"A": {"fields": [{"name": "a", "type": "float"}]}}})),
            ["E043"]
        );
        assert_eq!(
            codes(
                &json!({"types": {"A": {"fields": [{"name": "a", "type": "int", "minimum": 1}]}}})
            ),
            ["E038"]
        );
        assert_eq!(
            codes(&json!({"types": {"A": {"fields": [
                {"name": "a", "type": "int", "minimum": 2, "maximum": 1},
                {"name": "b", "type": "fixed", "minimum": "0", "maximum": "1", "resolution": "0.3"},
                {"name": "c", "type": "fixed", "minimum": "x", "maximum": "-1", "resolution": "0"},
                {"name": "d", "type": "counter", "width": 65},
                {"name": "e", "type": "int", "minimum": 0, "maximum": 0},
                {"name": "a", "type": "bool"},
                {"name": "f", "type": "enum", "values": ["x", "x"]},
                "...", "...",
                {"name": "g", "type": "bool"}
            ]}}})),
            [
                "E039", "E040", "E040", "E041", "E041", "E042", "E042", "E045", "E045"
            ]
        );
    }

    #[test]
    fn appending_behind_a_marker_keeps_the_tag() {
        let v1 = json!({"types": {"L": {"fields": [{"name": "up", "type": "bool"}, "..."]}}});
        let v2 = json!({"types": {"L": {"fields": [
            {"name": "up", "type": "bool"}, "...",
            {"name": "q", "type": "int", "minimum": 0, "maximum": 100, "optional": true}, "..."
        ]}}});
        let a = PackedDoc::read(&v1).types["L"].clone().unwrap();
        let b = PackedDoc::read(&v2).types["L"].clone().unwrap();
        assert_eq!(a.tag(), b.tag());
        let new = b.write(&json!({"up": true, "q": 87}), None).unwrap();
        assert_eq!(
            a.read(a.tag(), &new.bytes, None).unwrap(),
            json!({"up": true})
        );
        let old = a.write(&json!({"up": false}), None).unwrap();
        assert_eq!(
            b.read(b.tag(), &old.bytes, None).unwrap(),
            json!({"up": false})
        );
    }
}
