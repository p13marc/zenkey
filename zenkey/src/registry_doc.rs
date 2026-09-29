//! The registry file as a document, in either of its two spellings
//! (RFC 08 §5.1, v1.44; #374).
//!
//! RFC 08 §5.1 made the registry file writable in **KDL 2.0** as well as
//! TOML, and it did so as a second *spelling* of one document, not a second
//! schema: "a KDL registry file means exactly the TOML document the mapping
//! below produces from it". This module is that sentence as code. Each
//! spelling has a front-end that reads it into one format-neutral tree —
//! [`RawTable`] of [`RawValue`]s, shaped as the TOML document would be — and
//! everything downstream (the slice walk in [`crate::slice`], zenkey-build's
//! lints and codegen) reads the tree, so it cannot tell which spelling it was
//! handed and has no way to treat the two differently.
//!
//! What is *not* neutral lives here, once:
//!
//! - [`SliceFormat`] — which spelling, with its media type and extension.
//! - [`negotiate`] — the RFC 08 §6 dispatch on an `introspect` reply's
//!   declared `Encoding`, the whole of that table as one function.
//! - The KDL front-end's refusals — the only refusals the spelling adds
//!   (§5.1): a second argument, the identifying column spelled as a property
//!   as well, a list column spelled as a property, a repeated property, a
//!   number where a string column belongs, a type annotation, and any
//!   document that parses only as KDL 1.0.
//! - [`write_kdl`] — the tree back out as KDL, which is how the fixture
//!   corpus's KDL mirror is generated and how `to_kdl` renders a slice.

use std::fmt::Write as _;

use crate::encoding::WireEncoding;
use crate::slice::SliceError;

// ─── the tree ───────────────────────────────────────────────────────────────

/// One value of a registry document, in either spelling.
///
/// The variants are TOML's, because the RFC defines a KDL file by the TOML
/// document it maps to (§5.1). The accessors are named as `toml::Value`'s
/// are, so a walk written against one reads the other unchanged.
#[derive(Debug, Clone, PartialEq)]
pub enum RawValue {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    /// A TOML datetime, carried as its text. KDL has none; [`write_kdl`]
    /// spells it as a string, and no registry column is one.
    Datetime(String),
    /// A TOML array — of scalars, or (`[[…]]`) of tables.
    List(Vec<RawValue>),
    Table(RawTable),
}

/// A table of a registry document: its fields **in document order**, and
/// the comment block that stood before it.
///
/// Order is kept because a spelling converted from the other should read as
/// its author wrote it; it carries no meaning to any reader (§5.1 "Comments
/// and order"), so lookups ignore it and so does equality: two tables are
/// equal when they hold the same columns with equal values, in any order,
/// whatever comments stood before them — the document's meaning, which is
/// what "one document, two spellings" promises to preserve.
#[derive(Debug, Clone, Default)]
pub struct RawTable {
    /// `(column, value)` in document order; a column appears at most once.
    pub fields: Vec<(String, RawValue)>,
    /// Comment lines written before the table's node or header, without
    /// their comment marker. Neither front-end here fills it (comments carry
    /// no meaning, §5.1); a converter that keeps them does, and
    /// [`write_kdl`] writes them back as `//` lines.
    pub leading: Vec<String>,
}

impl PartialEq for RawTable {
    fn eq(&self, other: &Self) -> bool {
        self.fields.len() == other.fields.len() && self.iter().all(|(k, v)| other.get(k) == Some(v))
    }
}

impl RawTable {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        RawTable::default()
    }

    /// The value of one column, if present.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&RawValue> {
        self.fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    fn get_mut(&mut self, key: &str) -> Option<&mut RawValue> {
        self.fields
            .iter_mut()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    /// Whether the table carries this column.
    #[must_use]
    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Set a column: replaced in place when present, appended when not.
    pub fn insert(&mut self, key: impl Into<String>, value: RawValue) {
        let key = key.into();
        match self.get_mut(&key) {
            Some(slot) => *slot = value,
            None => self.fields.push((key, value)),
        }
    }

    /// The columns, in document order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &RawValue)> {
        self.fields.iter().map(|(k, v)| (k.as_str(), v))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

impl RawValue {
    /// A column of a table value; `None` for any other value.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&RawValue> {
        self.as_table().and_then(|t| t.get(key))
    }

    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            RawValue::Str(s) => Some(s),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            RawValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// A float — and only a float, as `toml::Value::as_float` is: an integer
    /// is not silently one, so a caller that accepts both says so.
    #[must_use]
    pub fn as_float(&self) -> Option<f64> {
        match self {
            RawValue::Float(f) => Some(*f),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            RawValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_array(&self) -> Option<&[RawValue]> {
        match self {
            RawValue::List(v) => Some(v),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_table(&self) -> Option<&RawTable> {
        match self {
            RawValue::Table(t) => Some(t),
            _ => None,
        }
    }

    /// The value's type, as a message names it.
    #[must_use]
    pub fn type_str(&self) -> &'static str {
        match self {
            RawValue::Str(_) => "string",
            RawValue::Int(_) => "integer",
            RawValue::Float(_) => "float",
            RawValue::Bool(_) => "boolean",
            RawValue::Datetime(_) => "datetime",
            RawValue::List(_) => "array",
            RawValue::Table(_) => "table",
        }
    }
}

// ─── the two spellings ──────────────────────────────────────────────────────

/// Which spelling a registry file is written in (RFC 08 §5.1).
///
/// The extension names it on disk (`<stem>.toml`, `<stem>.kdl`), and the
/// `introspect` reply's `Encoding` names it on the wire (RFC 08 §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SliceFormat {
    /// TOML — the pre-v1.44 wire, and what an undeclared reply is read as.
    Toml,
    /// KDL 2.0.0 (§5.1). Never KDL 1.0, which is refused.
    Kdl,
}

impl SliceFormat {
    /// Both spellings.
    pub const ALL: [SliceFormat; 2] = [SliceFormat::Toml, SliceFormat::Kdl];

    /// The media type an `introspect` reply MUST declare for a file in this
    /// spelling (RFC 08 §6, v1.44).
    #[must_use]
    pub const fn media_type(self) -> &'static str {
        match self {
            SliceFormat::Toml => "application/toml",
            SliceFormat::Kdl => "application/kdl",
        }
    }

    /// The file extension that names this spelling, without the dot.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            SliceFormat::Toml => "toml",
            SliceFormat::Kdl => "kdl",
        }
    }

    /// The spelling a file extension (without the dot) names, if either.
    #[must_use]
    pub fn from_extension(ext: &str) -> Option<SliceFormat> {
        SliceFormat::ALL.into_iter().find(|f| f.extension() == ext)
    }

    /// The spelling a declared media type names — exactly one of the two
    /// §6 names, parameters after a `;` ignored. `None` for everything
    /// else, `text/plain` included: that one is *undeclared*, which is
    /// [`negotiate`]'s business, not a spelling.
    #[must_use]
    pub fn from_media_type(media_type: &str) -> Option<SliceFormat> {
        let essence = essence(media_type);
        SliceFormat::ALL
            .into_iter()
            .find(|f| f.media_type() == essence)
    }

    /// The §6 sniff, for an undeclared reply only: the first byte that is
    /// neither whitespace nor inside a `#` comment line being `[` means TOML,
    /// and anything else is tried as KDL. A document with no such byte at
    /// all is TOML — the undeclared default, which the sniff only overrides
    /// on evidence.
    #[must_use]
    pub fn sniff(src: &str) -> SliceFormat {
        for line in src.lines() {
            let line = line.trim_start();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            return if line.starts_with('[') {
                SliceFormat::Toml
            } else {
                SliceFormat::Kdl
            };
        }
        SliceFormat::Toml
    }

    /// The [`WireEncoding`] a reply in this spelling carries.
    #[must_use]
    pub fn wire_encoding(self) -> WireEncoding {
        match self {
            SliceFormat::Toml => WireEncoding::Toml,
            SliceFormat::Kdl => WireEncoding::Kdl,
        }
    }
}

impl std::fmt::Display for SliceFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SliceFormat::Toml => "TOML",
            SliceFormat::Kdl => "KDL",
        })
    }
}

/// A media type without its parameters (`text/plain;charset=utf-8` is
/// `text/plain`, RFC 08 §6), trimmed.
fn essence(media_type: &str) -> &str {
    media_type
        .split_once(';')
        .map_or(media_type, |(e, _)| e)
        .trim()
}

/// Which spelling to read an `introspect` reply as — RFC 08 §6's table, as
/// one function.
///
/// - `application/toml` or `application/kdl` is the spelling, and is **never
///   second-guessed**: a sniff over a declaration would make the
///   declaration advisory.
/// - `text/plain`, `zenoh/bytes` (Zenoh's default for a replier that set
///   nothing) or no encoding at all is *undeclared* — the pre-v1.44 wire —
///   and only these are sniffed ([`SliceFormat::sniff`]), which rescues a
///   producer that broke the MUST and nothing else.
/// - Anything else is unreadable: [`SliceError::Encoding`], naming it. A
///   consumer reports that producer's slice as unreadable rather than
///   letting it pass for one that serves none (RFC 13 §3 O4).
///
/// Parameters after a `;` are not part of the declaration.
pub fn negotiate(declared: Option<&str>, src: &str) -> Result<SliceFormat, SliceError> {
    let Some(declared) = declared else {
        return Ok(SliceFormat::sniff(src));
    };
    if let Some(format) = SliceFormat::from_media_type(declared) {
        return Ok(format);
    }
    match essence(declared) {
        "" | "text/plain" | "zenoh/bytes" => Ok(SliceFormat::sniff(src)),
        _ => Err(SliceError::Encoding(declared.to_string())),
    }
}

/// Read a registry document in the given spelling into the neutral tree.
///
/// The tree is the TOML document the file means (§5.1) — for a KDL file,
/// the one the mapping produces — so whatever reads it next reads either
/// spelling identically.
pub fn parse_raw(src: &str, format: SliceFormat) -> Result<RawTable, SliceError> {
    match format {
        SliceFormat::Toml => toml_front::parse(src),
        SliceFormat::Kdl => kdl_front::parse(src),
    }
}

// ─── the TOML front-end ─────────────────────────────────────────────────────

mod toml_front {
    use super::{RawTable, RawValue};
    use crate::slice::SliceError;
    use toml::de::{DeTable, DeValue};

    /// Structural: the document as `toml` parses it, in document order.
    ///
    /// Order comes from the spans, never from the table's own iteration:
    /// that is sorted or insertion-ordered depending on whether *any* crate
    /// in the build enabled `toml`'s `preserve_order`, and a writer whose
    /// output moved with feature unification would not be a writer anyone
    /// could pin.
    pub(super) fn parse(src: &str) -> Result<RawTable, SliceError> {
        let doc = DeTable::parse(src)?;
        table(doc.get_ref())
    }

    fn table(t: &DeTable<'_>) -> Result<RawTable, SliceError> {
        let mut entries: Vec<_> = t.iter().collect();
        entries.sort_by_key(|(k, _)| k.span().start);
        let mut out = RawTable::new();
        for (k, v) in entries {
            out.fields
                .push((k.get_ref().to_string(), value(k.get_ref(), v.get_ref())?));
        }
        Ok(out)
    }

    fn value(key: &str, v: &DeValue<'_>) -> Result<RawValue, SliceError> {
        Ok(match v {
            DeValue::String(s) => RawValue::Str(s.to_string()),
            DeValue::Integer(i) => {
                RawValue::Int(i64::from_str_radix(i.as_str(), i.radix()).map_err(|_| {
                    SliceError::Shape(format!("{key} = {i} does not fit a 64-bit integer"))
                })?)
            }
            DeValue::Float(f) => RawValue::Float(
                f.as_str()
                    .parse::<f64>()
                    .map_err(|_| SliceError::Shape(format!("{key} = {f} is not a float")))?,
            ),
            DeValue::Boolean(b) => RawValue::Bool(*b),
            DeValue::Datetime(d) => RawValue::Datetime(d.to_string()),
            DeValue::Array(a) => RawValue::List(
                a.iter()
                    .map(|e| value(key, e.get_ref()))
                    .collect::<Result<_, _>>()?,
            ),
            DeValue::Table(t) => RawValue::Table(table(t)?),
        })
    }
}

// ─── the KDL front-end ──────────────────────────────────────────────────────

/// The §5.1 node table, as data: how each reserved node maps to a TOML
/// table.
struct NodeSpec {
    /// The node name, and the TOML key it maps to.
    name: &'static str,
    /// The identifying column the node's single argument spells, if any.
    ident: Option<&'static str>,
    /// `[[…]]` — one node per row — rather than a table at most once.
    many: bool,
    /// The list columns this node carries as child nodes (§5.1).
    lists: &'static [&'static str],
}

const TOP: &[NodeSpec] = &[
    NodeSpec {
        name: "registry",
        ident: None,
        many: false,
        lists: &[],
    },
    NodeSpec {
        name: "producer",
        ident: Some("name"),
        many: false,
        lists: &[],
    },
    NodeSpec {
        name: "service",
        ident: Some("name"),
        many: false,
        lists: &[],
    },
    NodeSpec {
        name: "budget",
        ident: None,
        many: false,
        lists: &[],
    },
    NodeSpec {
        name: "subject",
        ident: Some("path"),
        many: true,
        lists: &["when", "buckets"],
    },
    NodeSpec {
        name: "procedure",
        ident: Some("path"),
        many: true,
        lists: &["when"],
    },
    NodeSpec {
        name: "media",
        ident: Some("path"),
        many: true,
        lists: &[],
    },
    NodeSpec {
        name: "blob",
        ident: Some("tier"),
        many: true,
        lists: &["endpoints"],
    },
    NodeSpec {
        name: "error",
        ident: Some("name"),
        many: true,
        lists: &["procedures"],
    },
    NodeSpec {
        name: "deprecated",
        ident: Some("path"),
        many: true,
        lists: &[],
    },
];

/// `[[budget.tables]]`: one `table` child of `budget` per row.
const TABLE: NodeSpec = NodeSpec {
    name: "table",
    ident: Some("name"),
    many: true,
    lists: &[],
};

/// `[types.<Name>]`: one `type` node per name, the name its argument.
const TYPE: NodeSpec = NodeSpec {
    name: "type",
    ident: None,
    many: true,
    lists: &[],
};

/// The columns RFC 08 §2 types as strings, across every entry kind (and
/// the type table's). On a reserved node a non-string value for one is
/// refused rather than converted (§5.1) — a KDL `1.1` is a number, and
/// `1.10` is the same number and a different version.
const STRING_COLUMNS: &[&str] = &[
    "algo",
    "app",
    "attachment",
    "class",
    "common",
    "compat",
    "description",
    "encoding",
    "exposure",
    "fanout",
    "gate_note",
    "gone",
    "kind",
    "name",
    "origin",
    "path",
    "qos",
    "rate",
    "reference",
    "replaced_by",
    "reply",
    "request",
    "rust",
    "schema",
    "semantic",
    "since",
    "tier",
    "type",
    "unit",
    "variant",
    "version",
];

/// The version columns, whose refusal gets the sentence that says why.
const VERSION_COLUMNS: &[&str] = &["since", "gone", "version"];

/// The list columns whose elements are strings (`buckets`' are numbers).
const STRING_LISTS: &[&str] = &["when", "endpoints", "procedures"];

mod kdl_front {
    use super::{
        NodeSpec, RawTable, RawValue, STRING_COLUMNS, STRING_LISTS, TABLE, TOP, TYPE,
        VERSION_COLUMNS,
    };
    use crate::slice::SliceError;
    use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};

    /// KDL 2.0 only. `parse_v2`, not `parse`: the latter falls back to KDL
    /// 1.0 when a dependent enables kdl's `v1-fallback`, and §5.1 refuses a
    /// document that parses only as KDL 1.0 — whatever else is in the build.
    pub(super) fn parse(src: &str) -> Result<RawTable, SliceError> {
        let doc = KdlDocument::parse_v2(src)?;
        let cx = Cx { src };
        let mut out = RawTable::new();
        for node in doc.nodes() {
            cx.no_annotation_on_node(node)?;
            let name = node.name().value();
            if let Some(spec) = TOP.iter().find(|s| s.name == name) {
                let table = cx.entry(node, spec)?;
                if spec.many {
                    push_row(&mut out, name, table);
                } else if out.contains_key(name) {
                    return Err(cx.refuse(
                        node,
                        format!("a second `{name}` node — it appears at most once"),
                    ));
                } else {
                    out.fields.push((name.to_string(), RawValue::Table(table)));
                }
            } else if name == TYPE.name {
                let (type_name, table) = cx.type_node(node)?;
                if !matches!(out.get("types"), Some(RawValue::Table(_))) {
                    out.insert("types", RawValue::Table(RawTable::new()));
                }
                let Some(RawValue::Table(types)) = out.get_mut("types") else {
                    unreachable!("a `types` table was just ensured");
                };
                if types.contains_key(&type_name) {
                    return Err(cx.refuse(node, format!("type {type_name:?} is declared twice")));
                }
                types.fields.push((type_name, RawValue::Table(table)));
            } else {
                // An unknown node is an unknown table (§5.1): carried, never
                // refused — it may be a later amendment's entry kind, which a
                // consumer skips (§6).
                let value = cx.unknown(node)?;
                carry_unknown(&mut out, name, value);
            }
        }
        Ok(out)
    }

    /// Append one row to a `[[…]]` list, creating it on first sight.
    fn push_row(out: &mut RawTable, key: &str, row: RawTable) {
        match out.get_mut(key) {
            Some(RawValue::List(rows)) => rows.push(RawValue::Table(row)),
            _ => out
                .fields
                .push((key.to_string(), RawValue::List(vec![RawValue::Table(row)]))),
        }
    }

    /// Carry an unknown node or child: once is its value; again makes the
    /// column a list of each occurrence's value, in order.
    fn carry_unknown(out: &mut RawTable, key: &str, value: RawValue) {
        match out.get_mut(key) {
            None => out.fields.push((key.to_string(), value)),
            Some(slot) => {
                let first = std::mem::replace(slot, RawValue::List(Vec::new()));
                *slot = match first {
                    RawValue::List(mut rows) if rows.iter().all(|r| r.as_table().is_some()) => {
                        rows.push(value);
                        RawValue::List(rows)
                    }
                    other => RawValue::List(vec![other, value]),
                };
            }
        }
    }

    struct Cx<'s> {
        src: &'s str,
    }

    impl Cx<'_> {
        /// 1-based line of a byte offset, for the message.
        fn line(&self, offset: usize) -> usize {
            let end = offset.min(self.src.len());
            self.src.as_bytes()[..end]
                .iter()
                .filter(|&&b| b == b'\n')
                .count()
                + 1
        }

        fn refuse(&self, node: &KdlNode, why: String) -> SliceError {
            SliceError::Shape(format!(
                "line {}: `{}`{}: {why} (RFC 08 §5.1)",
                self.line(node.span().offset()),
                node.name().value(),
                first_arg(node)
                    .map(|a| format!(" {a:?}"))
                    .unwrap_or_default(),
            ))
        }

        fn no_annotation_on_node(&self, node: &KdlNode) -> Result<(), SliceError> {
            match node.ty() {
                Some(ty) => Err(self.refuse(
                    node,
                    format!(
                        "a type annotation `({})` on the node — §2's tables already make \
                         the one claim about a value's type, and a second can only \
                         repeat or contradict it",
                        ty.value()
                    ),
                )),
                None => Ok(()),
            }
        }

        fn no_annotation(&self, node: &KdlNode, e: &KdlEntry) -> Result<(), SliceError> {
            match e.ty() {
                Some(ty) => Err(self.refuse(
                    node,
                    format!(
                        "a type annotation `({})` on {} — §2's tables already make the \
                         one claim about a value's type, and a second can only repeat or \
                         contradict it",
                        ty.value(),
                        match e.name() {
                            Some(k) => format!("`{}`", k.value()),
                            None => "an argument".to_string(),
                        }
                    ),
                )),
                None => Ok(()),
            }
        }

        /// A scalar, as the TOML value it means. `None` is `#null` — absent.
        fn scalar(&self, node: &KdlNode, e: &KdlEntry) -> Result<Option<RawValue>, SliceError> {
            Ok(Some(match e.value() {
                KdlValue::String(s) => RawValue::Str(s.clone()),
                KdlValue::Integer(i) => RawValue::Int(i64::try_from(*i).map_err(|_| {
                    self.refuse(
                        node,
                        format!("{} does not fit a 64-bit integer, as TOML's do", repr(e)),
                    )
                })?),
                KdlValue::Float(f) => RawValue::Float(*f),
                KdlValue::Bool(b) => RawValue::Bool(*b),
                KdlValue::Null => return Ok(None),
            }))
        }

        /// A reserved node, as the table it maps to (§5.1).
        fn entry(&self, node: &KdlNode, spec: &NodeSpec) -> Result<RawTable, SliceError> {
            let mut out = RawTable::new();
            let mut seen: Vec<&str> = Vec::new();
            let mut args = 0usize;
            for e in node.entries() {
                self.no_annotation(node, e)?;
                let Some(key) = e.name() else {
                    args += 1;
                    let Some(ident) = spec.ident else {
                        return Err(self.refuse(
                            node,
                            format!(
                                "`{}` takes no argument — every column is a property",
                                spec.name
                            ),
                        ));
                    };
                    if args > 1 {
                        return Err(self.refuse(
                            node,
                            format!(
                                "a second argument {} — the one argument is the entry's \
                                 `{ident}`, and every other column is a property",
                                repr(e)
                            ),
                        ));
                    }
                    match e.value() {
                        KdlValue::String(s) => out
                            .fields
                            .push((ident.to_string(), RawValue::Str(s.clone()))),
                        _ => {
                            return Err(self.refuse(
                                node,
                                format!(
                                    "the argument {} is not a string — `{ident}` is one; \
                                     write it quoted",
                                    repr(e)
                                ),
                            ));
                        }
                    }
                    continue;
                };
                let key = key.value();
                if seen.contains(&key) {
                    return Err(self.refuse(
                        node,
                        format!(
                            "`{key}` is repeated — a repeated column is a copy-paste \
                             error, not an override, and is refused as TOML refuses a \
                             repeated key"
                        ),
                    ));
                }
                seen.push(key);
                if spec.ident == Some(key) {
                    return Err(self.refuse(
                        node,
                        format!(
                            "`{key}` spelled as a property — it is the node's argument, \
                             and one column has one spelling"
                        ),
                    ));
                }
                if spec.lists.contains(&key) {
                    return Err(self.refuse(
                        node,
                        format!(
                            "the list column `{key}` spelled as a property — it is a child \
                             node whose arguments are the elements: `{key} \"a\" \"b\"`"
                        ),
                    ));
                }
                let Some(value) = self.scalar(node, e)? else {
                    continue; // `#null`: the column is absent
                };
                if STRING_COLUMNS.contains(&key) && value.as_str().is_none() {
                    return Err(self.refuse(node, not_a_string(key, e)));
                }
                out.fields.push((key.to_string(), value));
            }
            if let Some(children) = node.children() {
                let mut child_seen: Vec<&str> = Vec::new();
                for child in children.nodes() {
                    self.no_annotation_on_node(child)?;
                    let cname = child.name().value();
                    if spec.name == "budget" && cname == TABLE.name {
                        let row = self.entry(child, &TABLE)?;
                        push_row(&mut out, "tables", row);
                        continue;
                    }
                    if spec.lists.contains(&cname) {
                        if child_seen.contains(&cname) {
                            return Err(self
                                .refuse(node, format!("the list column `{cname}` appears twice")));
                        }
                        child_seen.push(cname);
                        let list = self.list_child(node, child)?;
                        out.fields.push((cname.to_string(), list));
                        continue;
                    }
                    if out.contains_key(cname) && !child_seen.contains(&cname) {
                        return Err(self.refuse(
                            node,
                            format!("`{cname}` is both a property and a child node"),
                        ));
                    }
                    child_seen.push(cname);
                    let value = self.unknown(child)?;
                    carry_unknown(&mut out, cname, value);
                }
            }
            Ok(out)
        }

        /// A list column's child node: its arguments are the elements, in
        /// order, and it has no properties and no children (§5.1).
        fn list_child(&self, parent: &KdlNode, child: &KdlNode) -> Result<RawValue, SliceError> {
            let cname = child.name().value();
            if child.children().is_some() {
                return Err(self.refuse(
                    parent,
                    format!("the list column `{cname}` has children — its elements are arguments"),
                ));
            }
            let mut items = Vec::new();
            for e in child.entries() {
                self.no_annotation(child, e)?;
                if let Some(k) = e.name() {
                    return Err(self.refuse(
                        parent,
                        format!(
                            "the list column `{cname}` has a property `{}` — its elements \
                             are arguments",
                            k.value()
                        ),
                    ));
                }
                let Some(v) = self.scalar(child, e)? else {
                    return Err(self.refuse(
                        parent,
                        format!("`{cname}` has a #null element — an array has no null"),
                    ));
                };
                if STRING_LISTS.contains(&cname) && v.as_str().is_none() {
                    return Err(self.refuse(
                        parent,
                        format!(
                            "`{cname}` element {} is not a string — write it quoted",
                            repr(e)
                        ),
                    ));
                }
                items.push(v);
            }
            Ok(RawValue::List(items))
        }

        /// `type "Name" kind=…`: the name, and the `[types.Name]` table.
        fn type_node(&self, node: &KdlNode) -> Result<(String, RawTable), SliceError> {
            let mut name = None;
            let mut rest = node.clone();
            rest.entries_mut().retain(|e| {
                if e.name().is_none() && name.is_none() {
                    name = Some(e.clone());
                    false
                } else {
                    true
                }
            });
            let Some(name_entry) = name else {
                return Err(self.refuse(
                    node,
                    "a `type` node needs the type's name as its argument".to_string(),
                ));
            };
            self.no_annotation(node, &name_entry)?;
            let KdlValue::String(type_name) = name_entry.value() else {
                return Err(self.refuse(
                    node,
                    format!(
                        "the type name {} is not a string — write it quoted",
                        repr(&name_entry)
                    ),
                ));
            };
            if let Some(extra) = rest.entries().iter().find(|e| e.name().is_none()) {
                return Err(self.refuse(
                    node,
                    format!(
                        "a second argument {} — the one argument is the type's name, and \
                         every other column is a property",
                        repr(extra)
                    ),
                ));
            }
            Ok((type_name.clone(), self.entry(&rest, &TYPE)?))
        }

        /// An unknown node or child, carried as TOML carries an unknown
        /// table or key: arguments alone are a list; properties or a
        /// children block make a table. Its arguments, when it is a table,
        /// are not carried — they would be a later amendment's identifying
        /// column, and this build cannot know the column's name.
        fn unknown(&self, node: &KdlNode) -> Result<RawValue, SliceError> {
            let has_props = node.entries().iter().any(|e| e.name().is_some());
            if !has_props && node.children().is_none() {
                let mut items = Vec::new();
                for e in node.entries() {
                    self.no_annotation(node, e)?;
                    if let Some(v) = self.scalar(node, e)? {
                        items.push(v);
                    }
                }
                return Ok(RawValue::List(items));
            }
            let mut out = RawTable::new();
            let mut seen: Vec<&str> = Vec::new();
            for e in node.entries() {
                self.no_annotation(node, e)?;
                let Some(key) = e.name() else { continue };
                let key = key.value();
                if seen.contains(&key) {
                    return Err(self.refuse(
                        node,
                        format!("`{key}` is repeated — refused as TOML refuses a repeated key"),
                    ));
                }
                seen.push(key);
                if let Some(v) = self.scalar(node, e)? {
                    out.fields.push((key.to_string(), v));
                }
            }
            if let Some(children) = node.children() {
                for child in children.nodes() {
                    self.no_annotation_on_node(child)?;
                    let v = self.unknown(child)?;
                    carry_unknown(&mut out, child.name().value(), v);
                }
            }
            Ok(RawValue::Table(out))
        }
    }

    /// The node's first argument, for naming it in a message.
    fn first_arg(node: &KdlNode) -> Option<String> {
        node.entries()
            .iter()
            .find(|e| e.name().is_none())
            .and_then(|e| e.value().as_string().map(str::to_string))
    }

    /// The value as the file spelled it, where the parser kept that.
    fn repr(e: &KdlEntry) -> String {
        e.format()
            .map(|f| f.value_repr.clone())
            .filter(|r| !r.is_empty())
            .unwrap_or_else(|| e.value().to_string())
    }

    fn not_a_string(key: &str, e: &KdlEntry) -> String {
        let r = repr(e);
        let quoted = r.trim_start_matches('#');
        if VERSION_COLUMNS.contains(&key) {
            format!(
                "`{key}={r}` is a KDL number, and a version is a string: write \
                 `{key}=\"{quoted}\"` — `1.10` and `1.1` are one number but two \
                 MAJOR.MINOR versions (RFC 08 §3)"
            )
        } else {
            format!(
                "`{key}={r}` is not a string, and RFC 08 §2 types `{key}` as one: write \
                 `{key}=\"{quoted}\"` — a reader refuses rather than converts"
            )
        }
    }
}

// ─── the KDL writer ─────────────────────────────────────────────────────────

/// Columns whose values are a closed vocabulary, written bare by the §5.1
/// style (`class=telemetry`, `kind=write`) when the token is a bare
/// identifier. Everything else is quoted.
const BARE_COLUMNS: &[&str] = &[
    "class", "common", "compat", "exposure", "fanout", "kind", "qos", "semantic",
];

/// Past this width a node continues on the next line with `\` (§5.1 style).
const WRAP: usize = 80;

/// Render a registry document as KDL 2.0 (RFC 08 §5.1), in the RFC's style:
/// every argument and free-text value quoted, closed-vocabulary values bare,
/// a long node continued with `\`, one blank line between entries.
///
/// The inverse of [`parse_raw`] with [`SliceFormat::Kdl`]: `parse_raw` of
/// the output is the input, for every document a TOML file can hold that
/// has a KDL spelling. What has none — a top-level key outside any table, an
/// array of arrays, an array of tables other than `[[budget.tables]]` below
/// the top level — is a [`SliceError::Shape`], never a silent drop. No
/// registry file this RFC defines holds one.
pub fn write_kdl(doc: &RawTable) -> Result<String, SliceError> {
    let mut out = String::new();
    comments(&mut out, &doc.leading, "");
    let mut first = true;
    let mut last_was_type = false;
    let mut sep = |out: &mut String, is_type: bool| {
        if !first && !(is_type && last_was_type) {
            out.push('\n');
        }
        first = false;
        last_was_type = is_type;
    };
    for (key, value) in doc.iter() {
        if key == "types" {
            let Some(types) = value.as_table() else {
                return Err(no_spelling("`types` that is not a table"));
            };
            for (name, entry) in types.iter() {
                let Some(entry) = entry.as_table() else {
                    return Err(no_spelling(&format!(
                        "[types] {name:?} that is not a table"
                    )));
                };
                sep(&mut out, true);
                node(&mut out, "", "type", Some(name), entry, None, false)?;
            }
            continue;
        }
        let spec = TOP.iter().find(|s| s.name == key);
        match (spec, value) {
            (Some(spec), RawValue::List(rows)) if spec.many => {
                for row in rows {
                    let Some(row) = row.as_table() else {
                        return Err(no_spelling(&format!("[[{key}]] row that is not a table")));
                    };
                    sep(&mut out, false);
                    entry_node(&mut out, spec, row)?;
                }
            }
            (Some(spec), RawValue::Table(t)) if !spec.many => {
                sep(&mut out, false);
                entry_node(&mut out, spec, t)?;
            }
            (Some(_), other) => {
                return Err(no_spelling(&format!("`{key}` as a {}", other.type_str())));
            }
            (None, RawValue::Table(t)) => {
                sep(&mut out, false);
                node(&mut out, "", key, None, t, None, false)?;
            }
            (None, RawValue::List(rows))
                if !rows.is_empty() && rows.iter().all(|r| r.as_table().is_some()) =>
            {
                for row in rows.iter().filter_map(RawValue::as_table) {
                    sep(&mut out, false);
                    node(&mut out, "", key, None, row, None, false)?;
                }
            }
            (None, other) => {
                return Err(no_spelling(&format!(
                    "the top-level key `{key}` ({}) — KDL has no value outside a node",
                    other.type_str()
                )));
            }
        }
    }
    Ok(out)
}

fn no_spelling(what: &str) -> SliceError {
    SliceError::Shape(format!("{what} has no KDL spelling (RFC 08 §5.1)"))
}

fn comments(out: &mut String, lines: &[String], indent: &str) {
    for l in lines {
        if l.is_empty() {
            let _ = writeln!(out, "{indent}//");
        } else {
            let _ = writeln!(out, "{indent}// {l}");
        }
    }
}

fn entry_node(out: &mut String, spec: &NodeSpec, t: &RawTable) -> Result<(), SliceError> {
    let ident = match spec.ident {
        Some(col) => match t.get(col) {
            Some(RawValue::Str(s)) => Some(s.as_str()),
            Some(other) => {
                return Err(no_spelling(&format!(
                    "`{}` whose `{col}` is a {}",
                    spec.name,
                    other.type_str()
                )));
            }
            None => None,
        },
        None => None,
    };
    node(out, "", spec.name, ident, t, spec.ident, true)
}

/// One node: `name ["ident"] prop=value … [{ children }]`.
fn node(
    out: &mut String,
    indent: &str,
    name: &str,
    ident: Option<&str>,
    t: &RawTable,
    ident_col: Option<&str>,
    style_bare: bool,
) -> Result<(), SliceError> {
    comments(out, &t.leading, indent);
    let mut tokens: Vec<String> = Vec::new();
    if let Some(i) = ident {
        tokens.push(quote(i));
    }
    let mut children: Vec<(&str, &RawValue)> = Vec::new();
    for (k, v) in t.iter() {
        if Some(k) == ident_col {
            continue;
        }
        match v {
            RawValue::List(_) | RawValue::Table(_) => children.push((k, v)),
            scalar => {
                let bare = style_bare && BARE_COLUMNS.contains(&k);
                tokens.push(format!("{}={}", ident_token(k), scalar_token(scalar, bare)));
            }
        }
    }
    let mut line = format!("{indent}{}", ident_token(name));
    let cont = format!("{indent}    ");
    for tok in tokens {
        if line.trim_start().len() > name.len() && line.len() + 1 + tok.len() > WRAP {
            out.push_str(&line);
            out.push_str(" \\\n");
            line = format!("{cont}{tok}");
        } else {
            line.push(' ');
            line.push_str(&tok);
        }
    }
    out.push_str(&line);
    if children.is_empty() {
        out.push('\n');
        return Ok(());
    }
    out.push_str(" {\n");
    let inner = format!("{indent}    ");
    for (k, v) in children {
        match v {
            RawValue::List(items) if k == "tables" && name == "budget" => {
                for row in items {
                    let Some(row) = row.as_table() else {
                        return Err(no_spelling("a [[budget.tables]] row that is not a table"));
                    };
                    let ident = match row.get("name") {
                        Some(RawValue::Str(s)) => Some(s.as_str()),
                        Some(other) => {
                            return Err(no_spelling(&format!(
                                "a budget table whose `name` is a {}",
                                other.type_str()
                            )));
                        }
                        None => None,
                    };
                    node(out, &inner, "table", ident, row, Some("name"), true)?;
                }
            }
            RawValue::List(items)
                if !items.is_empty() && items.iter().all(|i| i.as_table().is_some()) =>
            {
                // An array of tables below the top level: one child per row,
                // as an unknown child repeated reads back.
                for row in items.iter().filter_map(RawValue::as_table) {
                    node(out, &inner, k, None, row, None, false)?;
                }
            }
            RawValue::List(items) => {
                let mut line = format!("{inner}{}", ident_token(k));
                for item in items {
                    let tok = match item {
                        RawValue::List(_) | RawValue::Table(_) => {
                            return Err(no_spelling(&format!(
                                "`{k}`, an array holding a {}",
                                item.type_str()
                            )));
                        }
                        scalar => scalar_token(scalar, false),
                    };
                    if line.len() + 1 + tok.len() > WRAP && line.trim_start() != k {
                        out.push_str(&line);
                        out.push_str(" \\\n");
                        line = format!("{inner}    {tok}");
                    } else {
                        line.push(' ');
                        line.push_str(&tok);
                    }
                }
                out.push_str(&line);
                out.push('\n');
            }
            RawValue::Table(sub) => {
                if sub.is_empty() {
                    let _ = writeln!(out, "{inner}{} {{}}", ident_token(k));
                } else {
                    node(out, &inner, k, None, sub, None, false)?;
                }
            }
            _ => unreachable!("only lists and tables are children"),
        }
    }
    let _ = writeln!(out, "{indent}}}");
    Ok(())
}

fn scalar_token(v: &RawValue, bare: bool) -> String {
    match v {
        RawValue::Str(s) if bare && is_bare_identifier(s) => s.clone(),
        RawValue::Str(s) | RawValue::Datetime(s) => quote(s),
        RawValue::Int(i) => i.to_string(),
        RawValue::Float(f) if f.is_nan() => "#nan".to_string(),
        RawValue::Float(f) if *f == f64::INFINITY => "#inf".to_string(),
        RawValue::Float(f) if *f == f64::NEG_INFINITY => "#-inf".to_string(),
        // `{:?}` always spells a float as one (`1.0`, `0.005`, `1e21`), so
        // it reads back as the same bits and never as an integer.
        RawValue::Float(f) => format!("{f:?}"),
        RawValue::Bool(b) => format!("#{b}"),
        RawValue::List(_) | RawValue::Table(_) => unreachable!("not a scalar"),
    }
}

/// A property key or node name: bare when KDL admits it, quoted otherwise.
fn ident_token(s: &str) -> String {
    if is_bare_identifier(s) {
        s.to_string()
    } else {
        quote(s)
    }
}

/// Conservative: ASCII letters, digits, `_`, `-`, `.`, led by a letter,
/// and none of the words KDL 2.0 reserves for its `#` keywords.
fn is_bare_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !matches!(s, "true" | "false" | "null" | "inf" | "nan")
}

/// A KDL 2.0 quoted string. Every character a quoted string may not hold
/// literally — the quote, the backslash, any newline, the controls and the
/// direction marks KDL disallows — is escaped.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if c.is_control()
                || matches!(
                    c,
                    '\u{85}'
                        | '\u{2028}'
                        | '\u{2029}'
                        | '\u{feff}'
                        | '\u{200e}'..='\u{200f}'
                        | '\u{202a}'..='\u{202e}'
                        | '\u{2066}'..='\u{2069}'
                ) =>
            {
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiation_follows_the_section_6_table() {
        let toml = "# c\n\n[registry]\n";
        let kdl = "// c\nregistry version=\"1.0\"\n";
        // Declared: the spelling, whatever the bytes look like.
        assert_eq!(
            negotiate(Some("application/toml"), kdl).unwrap(),
            SliceFormat::Toml
        );
        assert_eq!(
            negotiate(Some("application/kdl"), toml).unwrap(),
            SliceFormat::Kdl
        );
        assert_eq!(
            negotiate(Some("application/kdl;charset=utf-8"), toml).unwrap(),
            SliceFormat::Kdl
        );
        // Undeclared: sniffed.
        for undeclared in [
            None,
            Some("text/plain"),
            Some("text/plain;charset=utf-8"),
            Some("zenoh/bytes"),
        ] {
            assert_eq!(
                negotiate(undeclared, toml).unwrap(),
                SliceFormat::Toml,
                "{undeclared:?}"
            );
            assert_eq!(
                negotiate(undeclared, kdl).unwrap(),
                SliceFormat::Kdl,
                "{undeclared:?}"
            );
        }
        // Anything else: unreadable, and the error names what was declared.
        let err = negotiate(Some("application/json"), toml).unwrap_err();
        assert!(
            matches!(&err, SliceError::Encoding(e) if e == "application/json"),
            "{err:?}"
        );
        assert!(err.to_string().contains("application/json"), "{err}");
    }

    #[test]
    fn the_sniff_skips_blank_lines_and_hash_comments_only() {
        assert_eq!(
            SliceFormat::sniff("  \n# x\n  [registry]"),
            SliceFormat::Toml
        );
        assert_eq!(
            SliceFormat::sniff("registry version=\"1\""),
            SliceFormat::Kdl
        );
        // `//` is not a TOML comment: it is the first byte, and not `[`.
        assert_eq!(
            SliceFormat::sniff("// registry\n[registry]"),
            SliceFormat::Kdl
        );
        assert_eq!(SliceFormat::sniff(""), SliceFormat::Toml);
    }

    #[test]
    fn media_types_and_extensions_round_trip() {
        for f in SliceFormat::ALL {
            assert_eq!(SliceFormat::from_media_type(f.media_type()), Some(f));
            assert_eq!(SliceFormat::from_extension(f.extension()), Some(f));
            assert_eq!(
                WireEncoding::from_encoding_str(f.media_type()),
                f.wire_encoding()
            );
            assert_eq!(f.wire_encoding().as_encoding_str(), f.media_type());
        }
        assert_eq!(SliceFormat::from_media_type("text/plain"), None);
        assert_eq!(SliceFormat::from_extension("json"), None);
    }

    #[test]
    fn the_toml_front_end_keeps_document_order() {
        let t = parse_raw("[b]\nz = 1\na = 2\n[a]\n", SliceFormat::Toml).unwrap();
        let keys: Vec<&str> = t.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, ["b", "a"]);
        let inner: Vec<&str> = t
            .get("b")
            .unwrap()
            .as_table()
            .unwrap()
            .iter()
            .map(|(k, _)| k)
            .collect();
        assert_eq!(inner, ["z", "a"]);
    }

    #[test]
    fn strings_quote_what_kdl_cannot_hold_literally() {
        let src = "registry description=".to_string() + &quote("a \"q\" \\ \n\t\u{202e}x") + "\n";
        let t = parse_raw(&src, SliceFormat::Kdl).unwrap();
        assert_eq!(
            t.get("registry")
                .unwrap()
                .get("description")
                .unwrap()
                .as_str(),
            Some("a \"q\" \\ \n\t\u{202e}x")
        );
    }

    #[test]
    fn floats_integers_and_the_non_finite_round_trip() {
        let mut row = RawTable::new();
        row.insert("path", RawValue::Str("h".into()));
        row.insert(
            "buckets",
            RawValue::List(vec![
                RawValue::Float(0.0001),
                RawValue::Int(1),
                RawValue::Float(1e21),
                RawValue::Float(f64::INFINITY),
                RawValue::Float(f64::NEG_INFINITY),
            ]),
        );
        let mut doc = RawTable::new();
        doc.insert("subject", RawValue::List(vec![RawValue::Table(row)]));
        let kdl = write_kdl(&doc).unwrap();
        assert!(kdl.contains("buckets 0.0001 1 1e21 #inf #-inf"), "{kdl}");
        assert_eq!(parse_raw(&kdl, SliceFormat::Kdl).unwrap(), doc);
    }
}
