//! Packed types (core §7.6, #979 B2): a Rust struct per type of each
//! listed packed file, and its codec as straight-line code.
//!
//! **Why straight-line code.** Each record's layout is fixed when the
//! contract is: its fields' order, widths, presence bits, markers, unused
//! codes and reserved codes. So the encoder and decoder are written out a
//! statement per field, in the layout's order, each a call to one of
//! `zenkey::packed`'s helpers (the bits, exact fixed point, a time's code),
//! which are the model's own. A reader of the generated file sees the
//! record's wire, and a record costs no JSON value and no layout walked at
//! run time. The alternative, a table handed to the model's interpreted
//! codec through `serde_json::Value`, would make the vectors check the
//! model twice and the generated code not at all.
//!
//! The files a contract lists are modules under `__packed`, one per
//! artifact (contracts listing the same file share it), named after the
//! file's stem. In each: a struct per type, and a module per type holding
//! its fields' enums, its fixed-point declarations and its
//! `zenkey::packed::PackedRecord` impl.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use heck::{ToPascalCase, ToSnakeCase};
use serde_json::Value;
use zenkey_model::contract::Contract;
use zenkey_model::packed::{Field, FieldKind, Item, Layout, PackedDoc, RESERVED};
use zenkey_model::schema::{ArtifactData, SchemaKind};

use crate::Error;
use crate::names::ident;

/// One listed packed file and its module.
struct File {
    id: String,
    stem: String,
    module: String,
    doc: Value,
    /// Each type with a layout, in name order.
    types: Vec<(String, Layout)>,
}

/// Every packed file the contracts list, and which lists which.
#[derive(Default)]
pub struct PackedTypes {
    files: Vec<File>,
    /// Interface id → its files' indices.
    by_iface: BTreeMap<String, Vec<usize>>,
}

/// A Rust type name from a packed one: a type name is `[A-Z][A-Za-z0-9]*`
/// already, and `Self` is a keyword.
fn type_ident(name: &str) -> String {
    if name == "Self" {
        "Self_".to_owned()
    } else {
        name.to_owned()
    }
}

/// `names`, made unique in order: a repeat gets `_<n>` from 2.
fn unique(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut used = BTreeSet::new();
    names
        .into_iter()
        .map(|base| {
            let mut name = base.clone();
            let mut n = 2;
            while !used.insert(name.clone()) {
                name = format!("{base}_{n}");
                n += 1;
            }
            name
        })
        .collect()
}

/// A PascalCase identifier for an enum or a variant.
fn pascal(raw: &str) -> String {
    let p = raw.to_pascal_case();
    match p.as_str() {
        "" => "X".to_owned(),
        "Self" => "Self_".to_owned(),
        _ if p.starts_with(|c: char| c.is_ascii_digit()) => format!("X{p}"),
        _ => p,
    }
}

impl PackedTypes {
    pub fn gather(contracts: &[Contract]) -> Self {
        let mut out = Self::default();
        let mut by_id: BTreeMap<String, usize> = BTreeMap::new();
        for c in contracts {
            let mut mine = Vec::new();
            for (id, a) in &c.artifacts {
                let (SchemaKind::Packed, ArtifactData::Json(doc)) = (a.kind, &a.data) else {
                    continue;
                };
                let i = *by_id.entry(id.clone()).or_insert_with(|| {
                    let types = PackedDoc::read(doc)
                        .types
                        .into_iter()
                        .filter_map(|(n, l)| l.map(|l| (n, l)))
                        .collect();
                    out.files.push(File {
                        id: id.clone(),
                        stem: a.name.clone(),
                        module: String::new(),
                        doc: doc.clone(),
                        types,
                    });
                    out.files.len() - 1
                });
                mine.push(i);
            }
            if !mine.is_empty() {
                out.by_iface.insert(c.iface.to_string(), mine);
            }
        }
        let modules = unique(out.files.iter().map(|f| ident(&f.stem.to_snake_case())));
        for (f, m) in out.files.iter_mut().zip(modules) {
            f.module = m;
        }
        out
    }

    /// Each file's module, its stem, its artifact id and its description,
    /// for the module's doc.
    pub fn modules(&self) -> Vec<(&str, &str, &str, Option<&str>)> {
        self.files
            .iter()
            .map(|f| {
                (
                    f.module.as_str(),
                    f.stem.as_str(),
                    f.id.as_str(),
                    f.doc.get("description").and_then(Value::as_str),
                )
            })
            .collect()
    }

    /// The modules an interface's `types` re-exports.
    pub fn modules_of(&self, iface: &str) -> Vec<&str> {
        self.by_iface
            .get(iface)
            .into_iter()
            .flatten()
            .map(|i| self.files[*i].module.as_str())
            .collect()
    }

    /// The Rust type of `packed:<name>` in artifact `schema`, as seen from a
    /// module beside `__packed` (`root` is the path to their parent).
    pub fn rust_type(
        &self,
        iface: &str,
        name: &str,
        schema: &str,
        root: &str,
    ) -> Result<String, Error> {
        let f = self
            .files
            .iter()
            .find(|f| f.id == schema)
            .ok_or_else(|| Error::Codegen(format!("{iface}: no packed artifact {schema}")))?;
        if !f.types.iter().any(|(n, _)| n == name) {
            return Err(Error::Codegen(format!(
                "{iface}: {}.json has no types/{name} with a layout",
                f.stem
            )));
        }
        Ok(format!(
            "{root}::__packed::{}::{}",
            f.module,
            type_ident(name)
        ))
    }

    /// Generates every file's module into `dir`. Returns (module, file)
    /// pairs.
    pub fn compile(&self, dir: &Path) -> Result<Vec<(String, PathBuf)>, Error> {
        if self.files.is_empty() {
            return Ok(Vec::new());
        }
        std::fs::create_dir_all(dir).map_err(|e| Error::Io(dir.to_owned(), e))?;
        let mut out = Vec::new();
        for f in &self.files {
            let path = dir.join(format!("{}.rs", f.module));
            crate::write_if_changed(&path, f.emit().as_bytes())?;
            out.push((f.module.clone(), path));
        }
        Ok(out)
    }
}

/// A field's place in its record, and what its Rust value is.
struct Slot<'a> {
    f: &'a Field,
    ident: String,
    /// The field's index among the record's items: its fixed-point
    /// declaration's static is `FIXED_<i>`.
    index: usize,
    /// After the first marker, or optional: an `Option`.
    optional: bool,
    /// The enum's Rust name, for an `enum` field.
    enum_name: Option<String>,
    description: Option<String>,
}

fn reserved_of(k: &FieldKind) -> &[&'static str] {
    match k {
        FieldKind::Int { reserved, .. } | FieldKind::Fixed { reserved, .. } => reserved,
        _ => &[],
    }
}

impl Slot<'_> {
    /// The value's type, inside an `Option` or not: `module` is the type's
    /// module, as the struct names it.
    fn value_ty(&self, module: &str) -> String {
        let base = match &self.f.kind {
            FieldKind::Int { .. } | FieldKind::Time { .. } => "i64".to_owned(),
            FieldKind::Fixed { .. } => "::zenkey::packed::Decimal".to_owned(),
            FieldKind::Bool => "bool".to_owned(),
            FieldKind::Counter => "u64".to_owned(),
            FieldKind::Enum { escape, .. } => {
                let e = format!("{module}::{}", self.enum_name.as_deref().unwrap_or("E"));
                if *escape {
                    format!("::zenkey::packed::Open<{e}>")
                } else {
                    e
                }
            }
        };
        if reserved_of(&self.f.kind).is_empty() {
            base
        } else {
            format!("::zenkey::packed::Coded<{base}>")
        }
    }

    fn ty(&self, module: &str) -> String {
        if self.optional {
            format!("::std::option::Option<{}>", self.value_ty(module))
        } else {
            self.value_ty(module)
        }
    }

    /// What the field declares, in words, for its doc.
    fn declared(&self) -> String {
        let f = self.f;
        let what = match &f.kind {
            FieldKind::Int {
                minimum, maximum, ..
            } => format!("`int`, {minimum} to {maximum}"),
            FieldKind::Fixed {
                minimum,
                maximum,
                resolution,
                ..
            } => format!("`fixed`, {minimum} to {maximum} by {resolution}, exact"),
            FieldKind::Enum { values, escape } => format!(
                "`enum` of {}{}",
                values.len(),
                if *escape { ", with an escape" } else { "" }
            ),
            FieldKind::Bool => "`bool`".to_owned(),
            FieldKind::Counter => format!("`counter`, written modulo 2^{}", f.width),
            FieldKind::Time { ms } => format!(
                "`time`, milliseconds since the Unix epoch, written {} since the deployment epoch",
                if *ms {
                    "as seconds and milliseconds"
                } else {
                    "as whole seconds"
                }
            ),
        };
        let bits = if f.width == 1 { "bit" } else { "bits" };
        let mut s = format!("`{}`: {what}, {} {bits}", f.name, f.width);
        let r = reserved_of(&f.kind);
        if !r.is_empty() {
            let _ = write!(s, "; reserved {}", r.join(", "));
        }
        if f.optional {
            s.push_str("; optional");
        } else if self.optional {
            s.push_str("; behind an extension marker");
        }
        s
    }

    /// The code of `v`, a reference to the field's value (not its
    /// `Option`), as an expression that may `?`.
    fn code(&self, v: &str) -> String {
        let f = self.f;
        let value = |x: &str| -> String {
            match &f.kind {
                FieldKind::Int {
                    minimum, maximum, ..
                } => format!(
                    "::zenkey::packed::int_code(i128::from(*{x}), {minimum}_i64, {maximum}_i64)?"
                ),
                FieldKind::Fixed { .. } => format!("FIXED_{}.code({x})?", self.index),
                FieldKind::Enum { escape: true, .. } => format!(
                    "match {x} {{ ::zenkey::packed::Open::Known(e) => e.code(), ::zenkey::packed::Open::Escape => ::zenkey::packed::top_code({}) }}",
                    f.width
                ),
                FieldKind::Enum { .. } => format!("{x}.code()"),
                FieldKind::Bool => format!("u64::from(*{x})"),
                // Its low `width` bits: the count modulo 2^width.
                FieldKind::Counter => format!("*{x}"),
                FieldKind::Time { ms } => {
                    format!("::zenkey::packed::time_code(i128::from(*{x}), epoch, {ms})?")
                }
            }
        };
        let declared = reserved_of(&f.kind);
        if declared.is_empty() {
            return value(v);
        }
        let mut s = format!(
            "match {v} {{ ::zenkey::packed::Coded::Value(x) => {}",
            value("x")
        );
        for (name, variant) in RESERVED.iter().zip(["NotAvailable", "Error", "OutOfRange"]) {
            match zenkey_model::packed::reserved_code(f.width, declared, name) {
                Some(c) => {
                    let _ = write!(s, ", ::zenkey::packed::Coded::{variant} => {c}_u64");
                }
                None => {
                    let _ = write!(
                        s,
                        ", ::zenkey::packed::Coded::{variant} => return ::std::result::Result::Err(::zenkey::packed::WriteRefusal::Value)"
                    );
                }
            }
        }
        s.push_str(" }");
        s
    }

    /// The field's value read from its code `c`, as an expression that may
    /// `?`.
    fn read(&self) -> String {
        let f = self.f;
        let value = |c: &str| -> String {
            match &f.kind {
                FieldKind::Int {
                    minimum, maximum, ..
                } => format!("::zenkey::packed::int_i64({c}, {minimum}_i64, {maximum}_i64)?"),
                FieldKind::Fixed { .. } => format!("FIXED_{}.value({c})?", self.index),
                FieldKind::Enum { escape: true, .. } => format!(
                    "match {}::from_code({c}) {{ ::std::option::Option::Some(e) => ::zenkey::packed::Open::Known(e), ::std::option::Option::None => ::zenkey::packed::Open::Escape }}",
                    self.enum_name.as_deref().unwrap_or("E")
                ),
                FieldKind::Enum { .. } => format!(
                    "{}::from_code({c}).ok_or(::zenkey::packed::ReadRefusal::Code)?",
                    self.enum_name.as_deref().unwrap_or("E")
                ),
                FieldKind::Bool => format!("{c} == 1"),
                FieldKind::Counter => c.to_owned(),
                FieldKind::Time { ms } => format!("::zenkey::packed::time_ms({c}, epoch, {ms})?"),
            }
        };
        let declared = reserved_of(&f.kind);
        if declared.is_empty() {
            return format!("{{ let c = r.take({})?; {} }}", f.width, value("c"));
        }
        let mut s = format!("{{ let c = r.take({})?; match c {{ ", f.width);
        for (name, variant) in RESERVED.iter().zip(["NotAvailable", "Error", "OutOfRange"]) {
            if let Some(c) = zenkey_model::packed::reserved_code(f.width, declared, name) {
                let _ = write!(s, "{c}_u64 => ::zenkey::packed::Coded::{variant}, ");
            }
        }
        let _ = write!(
            s,
            "c => ::zenkey::packed::Coded::Value({}) }} }}",
            value("c")
        );
        s
    }

    /// The JSON reading of `v`, a reference to the field's value.
    fn json(&self, v: &str) -> String {
        let value = |x: &str| -> String {
            match &self.f.kind {
                FieldKind::Int { .. } | FieldKind::Counter | FieldKind::Time { .. } => {
                    format!("::zenkey::packed::Json::from(*{x})")
                }
                FieldKind::Fixed { .. } => {
                    format!("::zenkey::packed::Json::String({x}.to_string())")
                }
                FieldKind::Bool => format!("::zenkey::packed::Json::Bool(*{x})"),
                FieldKind::Enum { escape: true, .. } => format!(
                    "match {x} {{ ::zenkey::packed::Open::Known(e) => ::zenkey::packed::Json::from(e.name()), ::zenkey::packed::Open::Escape => ::zenkey::packed::Json::Null }}"
                ),
                FieldKind::Enum { .. } => format!("::zenkey::packed::Json::from({x}.name())"),
            }
        };
        if reserved_of(&self.f.kind).is_empty() {
            value(v)
        } else {
            format!(
                "match {v} {{ ::zenkey::packed::Coded::Value(x) => {}, c => ::zenkey::packed::Json::from(c.reserved().unwrap_or_default()) }}",
                value("x")
            )
        }
    }

    /// The field's value from its JSON member `v`, as an expression that
    /// may `?`.
    fn json_read(&self) -> String {
        let read = match &self.f.kind {
            FieldKind::Int { .. } | FieldKind::Time { .. } => {
                "::zenkey::packed::json_i64".to_owned()
            }
            FieldKind::Fixed { .. } => "::zenkey::packed::json_decimal".to_owned(),
            FieldKind::Bool => "::zenkey::packed::json_bool".to_owned(),
            FieldKind::Counter => "::zenkey::packed::json_u64".to_owned(),
            FieldKind::Enum { escape, .. } => {
                let e = self.enum_name.as_deref().unwrap_or("E");
                if *escape {
                    return format!("::zenkey::packed::json_open(v, {e}::from_name)?");
                }
                return format!("::zenkey::packed::json_enum(v, {e}::from_name)?");
            }
        };
        if reserved_of(&self.f.kind).is_empty() {
            format!("{read}(v)?")
        } else {
            format!("::zenkey::packed::json_coded(v, {read})?")
        }
    }
}

fn doc(indent: &str, text: &str) -> String {
    crate::emit::doc(indent, text)
}

impl File {
    fn emit(&self) -> String {
        let mut s = format!(
            "// @generated by zenkey-build from {}.json ({}): core §7.6's packed types. Do not edit.\n",
            self.stem, self.id
        );
        // A type's module never takes `error`: typify names its own module so
        // in every JSON Schema module, and an interface's `types` re-exports
        // both (config.v2's `Error` beside its standard schema, #985 G2).
        let modules = unique(
            std::iter::once("error".to_owned())
                .chain(self.types.iter().map(|(n, _)| ident(&n.to_snake_case()))),
        );
        for ((name, layout), module) in self.types.iter().zip(modules.into_iter().skip(1)) {
            let record = &self.doc["types"][name.as_str()];
            emit_type(&mut s, name, &module, layout, record);
        }
        s
    }
}

#[allow(clippy::too_many_lines)]
fn emit_type(s: &mut String, name: &str, module: &str, layout: &Layout, record: &Value) {
    let rust = type_ident(name);
    let items = record["fields"].as_array().cloned().unwrap_or_default();
    // Each field's slot, in order.
    let mut seen_marker = false;
    let enum_names = unique(layout.items.iter().filter_map(|it| match it {
        Item::Field(f) if matches!(f.kind, FieldKind::Enum { .. }) => Some(pascal(&f.name)),
        _ => None,
    }));
    let mut enums = enum_names.into_iter();
    let idents = unique(layout.fields().map(|f| ident(&f.name)));
    let mut idents = idents.into_iter();
    let mut slots: Vec<Option<Slot<'_>>> = Vec::new();
    for (i, it) in layout.items.iter().enumerate() {
        match it {
            Item::Marker => {
                seen_marker = true;
                slots.push(None);
            }
            Item::Field(f) => slots.push(Some(Slot {
                f,
                ident: idents.next().unwrap_or_default(),
                index: i,
                optional: f.optional || seen_marker,
                enum_name: matches!(f.kind, FieldKind::Enum { .. })
                    .then(|| enums.next().unwrap_or_default()),
                description: items
                    .get(i)
                    .and_then(|v| v.get("description"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })),
        }
    }
    let fields: Vec<&Slot<'_>> = slots.iter().flatten().collect();

    // The struct.
    let mut head = match record.get("description").and_then(Value::as_str) {
        Some(d) => format!("{d}\n\n"),
        None => String::new(),
    };
    let _ = write!(
        head,
        "`packed:{name}` (core §7.6): tag {}, {} to {} bits before the pad.",
        layout.tag(),
        layout.min_bits(),
        layout.max_bits()
    );
    s.push('\n');
    s.push_str(&doc("", &head));
    s.push_str("#[derive(Debug, Clone, PartialEq)]\n");
    let _ = writeln!(s, "pub struct {rust} {{");
    for f in &fields {
        let mut d = f.declared();
        if let Some(text) = &f.description {
            d = format!("{text}\n\n{d}");
        }
        s.push_str(&doc("    ", &d));
        let _ = writeln!(s, "    pub {}: {},", f.ident, f.ty(module));
    }
    s.push_str("}\n\n");

    // Its module: the enums, the fixed-point declarations, the codec.
    s.push_str(&doc(
        "",
        &format!("`{name}`'s fields' enums and its codec."),
    ));
    let _ = writeln!(s, "pub mod {module} {{");
    for f in &fields {
        let FieldKind::Enum { values, .. } = &f.f.kind else {
            continue;
        };
        let e = f.enum_name.as_deref().unwrap_or("E");
        let variants = unique(values.iter().map(|v| pascal(v)));
        s.push_str(&doc("    ", &format!("`{}`'s values, by code.", f.f.name)));
        s.push_str("    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]\n");
        let _ = writeln!(s, "    pub enum {e} {{");
        for (i, (v, raw)) in variants.iter().zip(values).enumerate() {
            s.push_str(&doc("        ", &format!("`{raw}`, code {i}")));
            let _ = writeln!(s, "        {v} = {i},");
        }
        s.push_str("    }\n\n");
        let _ = writeln!(s, "    impl {e} {{");
        let _ = writeln!(
            s,
            "        /// Every value, by code.\n        pub const ALL: [Self; {}] = [{}];",
            values.len(),
            variants
                .iter()
                .map(|v| format!("Self::{v}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        s.push_str("        /// Its code.\n        #[must_use]\n        pub fn code(self) -> u64 {\n            self as u64\n        }\n");
        s.push_str("        /// The value of a code, `None` past the values.\n        #[must_use]\n        pub fn from_code(code: u64) -> ::std::option::Option<Self> {\n            match code {\n");
        for (i, v) in variants.iter().enumerate() {
            let _ = writeln!(
                s,
                "                {i} => ::std::option::Option::Some(Self::{v}),"
            );
        }
        s.push_str("                _ => ::std::option::Option::None,\n            }\n        }\n");
        s.push_str("        /// Its name, as the JSON reading spells it.\n        #[must_use]\n        pub fn name(self) -> &'static str {\n            match self {\n");
        for (v, raw) in variants.iter().zip(values) {
            let _ = writeln!(s, "                Self::{v} => {raw:?},");
        }
        s.push_str("            }\n        }\n");
        s.push_str("        /// The value a name spells.\n        #[must_use]\n        pub fn from_name(name: &str) -> ::std::option::Option<Self> {\n            match name {\n");
        for (v, raw) in variants.iter().zip(values) {
            let _ = writeln!(
                s,
                "                {raw:?} => ::std::option::Option::Some(Self::{v}),"
            );
        }
        s.push_str("                _ => ::std::option::Option::None,\n            }\n        }\n    }\n\n");
    }
    for f in &fields {
        if let FieldKind::Fixed {
            minimum,
            resolution,
            values,
            ..
        } = &f.f.kind
        {
            let _ = writeln!(
                s,
                "    /// `{}`'s fixed point, its decimals read once. The build linted its\n    /// declaration (core §9.2's E038 to E045) and spelt them from the\n    /// decimals it read, so `Fixed::new` reads them back: it never panics.\n    static FIXED_{}: ::std::sync::LazyLock<::zenkey::packed::Fixed> =\n        ::std::sync::LazyLock::new(|| ::zenkey::packed::Fixed::new({:?}, {:?}, {values}));\n",
                f.f.name,
                f.index,
                minimum.to_string(),
                resolution.to_string()
            );
        }
    }

    let _ = writeln!(
        s,
        "    impl ::zenkey::packed::PackedRecord for super::{rust} {{\n        const NAME: &'static str = {name:?};\n        const TAG: u8 = {};\n        const MIN_BITS: u64 = {};\n        const MAX_BITS: u64 = {};\n",
        layout.tag(),
        layout.min_bits(),
        layout.max_bits()
    );

    // encode
    s.push_str("        fn encode(&self, epoch: ::std::option::Option<i64>) -> ::std::result::Result<::zenkey::packed::Record, ::zenkey::packed::WriteRefusal> {\n            let mut w = ::zenkey::packed::BitWriter::new();\n");
    emit_encode(s, &slots, 0, 3);
    s.push_str("            ::std::result::Result::Ok(w.finish())\n        }\n\n");

    // decode
    s.push_str("        fn decode(tag: u8, bytes: &[u8], epoch: ::std::option::Option<i64>) -> ::std::result::Result<Self, ::zenkey::packed::ReadRefusal> {\n            if tag != Self::TAG {\n                return ::std::result::Result::Err(::zenkey::packed::ReadRefusal::Tag);\n            }\n            let mut r = ::zenkey::packed::BitReader::new(bytes);\n");
    let first_marker = slots.iter().position(Option::is_none);
    let root = first_marker.unwrap_or(slots.len());
    for f in slots[..root].iter().flatten() {
        if f.f.optional {
            let _ = writeln!(
                s,
                "            let {} = if r.bit()? {{ ::std::option::Option::Some({}) }} else {{ ::std::option::Option::None }};",
                f.ident,
                f.read()
            );
        } else {
            let _ = writeln!(s, "            let {} = {};", f.ident, f.read());
        }
    }
    if let Some(m0) = first_marker {
        for f in slots[m0..].iter().flatten() {
            let _ = writeln!(
                s,
                "            let mut {} = ::std::option::Option::None;",
                f.ident
            );
        }
        s.push_str("            // A marker read as 1 that ends the layout: a later revision's bits follow.\n            let mut later = false;\n            // Whether a field is present since the last marker read as 1.\n            let mut present = false;\n            'record: {\n");
        let mut k = 0;
        let last = slots.len() - 1;
        for (i, slot) in slots.iter().enumerate().skip(m0) {
            match slot {
                None => {
                    let _ = writeln!(s, "                // extension marker {}", k + 1);
                    s.push_str("                if r.bit()? {\n");
                    if i == last {
                        s.push_str("                    r.later()?;\n                    later = true;\n                    break 'record;\n");
                    } else {
                        s.push_str("                    present = false;\n");
                    }
                    s.push_str("                } else {\n");
                    if k > 0 {
                        s.push_str("                    if !present {\n                        return ::std::result::Result::Err(::zenkey::packed::ReadRefusal::Extension);\n                    }\n");
                    }
                    s.push_str("                    break 'record;\n                }\n");
                    k += 1;
                }
                Some(f) => {
                    let _ = writeln!(
                        s,
                        "                if r.bit()? {{\n                    {} = ::std::option::Option::Some({});\n                    present = true;\n                }}",
                        f.ident,
                        f.read()
                    );
                }
            }
        }
        if slots.last().is_some_and(Option::is_some) {
            s.push_str("                if !present {\n                    return ::std::result::Result::Err(::zenkey::packed::ReadRefusal::Extension);\n                }\n");
        }
        s.push_str(
            "            }\n            if !later {\n                r.end()?;\n            }\n",
        );
    } else {
        s.push_str("            r.end()?;\n");
    }
    let _ = writeln!(
        s,
        "            ::std::result::Result::Ok(Self {{ {} }})\n        }}\n",
        fields
            .iter()
            .map(|f| f.ident.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    // to_json
    s.push_str("        fn to_json(&self) -> ::zenkey::packed::Json {\n            let mut m = ::zenkey::packed::Map::new();\n");
    for f in &fields {
        if f.optional {
            let _ = writeln!(
                s,
                "            if let ::std::option::Option::Some(v) = &self.{} {{\n                m.insert({:?}.to_owned(), {});\n            }}",
                f.ident,
                f.f.name,
                f.json("v")
            );
        } else {
            let _ = writeln!(
                s,
                "            {{\n                let v = &self.{};\n                m.insert({:?}.to_owned(), {});\n            }}",
                f.ident,
                f.f.name,
                f.json("v")
            );
        }
    }
    s.push_str("            ::zenkey::packed::Json::Object(m)\n        }\n\n");

    // from_json
    let _ = writeln!(
        s,
        "        fn from_json(v: &::zenkey::packed::Json) -> ::std::result::Result<Self, ::zenkey::packed::WriteRefusal> {{\n            let m = ::zenkey::packed::object(v)?;\n            ::zenkey::packed::known(m, &[{}])?;",
        fields
            .iter()
            .map(|f| format!("{:?}", f.f.name))
            .collect::<Vec<_>>()
            .join(", ")
    );
    for f in &fields {
        if f.optional {
            let _ = writeln!(
                s,
                "            let {} = match m.get({:?}) {{\n                ::std::option::Option::Some(v) => ::std::option::Option::Some({}),\n                ::std::option::Option::None => ::std::option::Option::None,\n            }};",
                f.ident,
                f.f.name,
                f.json_read()
            );
        } else {
            let _ = writeln!(
                s,
                "            let {} = {{\n                let v = ::zenkey::packed::required(m, {:?})?;\n                {}\n            }};",
                f.ident,
                f.f.name,
                f.json_read()
            );
        }
    }
    let _ = writeln!(
        s,
        "            ::std::result::Result::Ok(Self {{ {} }})\n        }}\n    }}\n}}",
        fields
            .iter()
            .map(|f| f.ident.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
}

/// The encoder's statements for the items from `from`, at `depth`
/// indentation: a field per statement, a marker an `if` over the rest.
fn emit_encode(s: &mut String, slots: &[Option<Slot<'_>>], from: usize, depth: usize) {
    let pad = "    ".repeat(depth);
    for (i, slot) in slots.iter().enumerate().skip(from) {
        match slot {
            Some(f) => {
                let width = f.f.width;
                if f.optional {
                    let _ = writeln!(
                        s,
                        "{pad}// {}\n{pad}match &self.{} {{\n{pad}    ::std::option::Option::None => w.bit(false),\n{pad}    ::std::option::Option::Some(v) => {{\n{pad}        let c = {};\n{pad}        w.bit(true);\n{pad}        w.push(c, {width});\n{pad}    }}\n{pad}}}",
                        f.f.name,
                        f.ident,
                        f.code("v")
                    );
                } else {
                    let _ = writeln!(
                        s,
                        "{pad}// {}\n{pad}{{\n{pad}    let v = &self.{};\n{pad}    let c = {};\n{pad}    w.push(c, {width});\n{pad}}}",
                        f.f.name,
                        f.ident,
                        f.code("v")
                    );
                }
            }
            None => {
                let after: Vec<String> = slots[i + 1..]
                    .iter()
                    .flatten()
                    .map(|f| format!("self.{}.is_some()", f.ident))
                    .collect();
                let _ = writeln!(
                    s,
                    "{pad}// An extension marker: 1 when a field after it is present."
                );
                if after.is_empty() {
                    let _ = writeln!(s, "{pad}w.bit(false);");
                    return;
                }
                let _ = writeln!(
                    s,
                    "{pad}let later = {};\n{pad}w.bit(later);\n{pad}if later {{",
                    after.join(" || ")
                );
                emit_encode(s, slots, i + 1, depth + 1);
                let _ = writeln!(s, "{pad}}}");
                return;
            }
        }
    }
}
