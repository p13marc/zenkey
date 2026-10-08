//! Schema artifacts and type resolution (r3 §3.9, r3.3 D5–D7).
//!
//! - **Protobuf:** every listed `.proto` file is compiled on its own with
//!   `protox` into a `FileDescriptorSet` that includes its imports, without
//!   source info. That set is the file's artifact. A message type resolves
//!   to the listed file that defines it. The well-known types
//!   (`google.protobuf.*`) are always available; one that no listed file
//!   defines is compiled on demand and becomes an artifact of its own.
//! - **JSON Schema:** every listed file is one artifact, hashed over its
//!   JCS form. `json:<Name>` names a `$defs` entry that is unique across
//!   the listed files; `json:<stem>#<Name>` names one in the file whose stem
//!   is `<stem>`. A cross-file `$ref` may point only at listed files, and
//!   in a bundle it resolves by the file stem of its path, so stems are
//!   unique per contract.
//! - **Raw:** a media type (`image/jpeg`) or family (`video/*`), with no
//!   artifact.
//!
//! An artifact's id is `sha256:<hex>` of its bytes (protobuf) or of its JCS
//! form (JSON Schema). A type's identity is (kind, name, artifact id).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

use base64::Engine as _;
use prost_reflect::DescriptorPool;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::authoring::{RawType, SchemasSection, TypeRef};
use crate::diag::{Diagnostic, Report};

/// A schema artifact's kind, by MCAP's spellings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SchemaKind {
    Protobuf,
    JsonSchema,
}

impl SchemaKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Protobuf => "protobuf",
            Self::JsonSchema => "jsonschema",
        }
    }
}

/// One schema artifact, as a bundle carries it.
#[derive(Debug, Clone, PartialEq)]
pub struct Artifact {
    pub kind: SchemaKind,
    /// The protobuf file name (include-relative, as imports spell it) or
    /// the JSON Schema file stem.
    pub name: String,
    pub data: ArtifactData,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ArtifactData {
    /// An encoded `FileDescriptorSet`.
    Protobuf(Vec<u8>),
    /// A JSON Schema document.
    Json(Value),
}

impl Artifact {
    /// `sha256:<hex>` over the artifact's bytes, or its JCS form.
    #[must_use]
    pub fn id(&self) -> String {
        match &self.data {
            ArtifactData::Protobuf(b) => sha256_id(b),
            ArtifactData::Json(v) => sha256_id(&jcs(v)),
        }
    }

    /// The `data` member of the artifact's bundle entry: base64 of a
    /// descriptor set, or the JSON Schema document itself.
    #[must_use]
    pub fn data_value(&self) -> Value {
        match &self.data {
            ArtifactData::Protobuf(b) => {
                Value::String(base64::engine::general_purpose::STANDARD.encode(b))
            }
            ArtifactData::Json(v) => v.clone(),
        }
    }
}

/// A resolved type identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum TypeId {
    Protobuf {
        name: String,
        schema: String,
    },
    JsonSchema {
        name: String,
        schema: String,
    },
    Raw {
        media_type: String,
        media_param: Option<String>,
    },
}

impl TypeId {
    #[must_use]
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Protobuf { .. } => "protobuf",
            Self::JsonSchema { .. } => "jsonschema",
            Self::Raw { .. } => "raw",
        }
    }
    #[must_use]
    pub fn is_json(&self) -> bool {
        matches!(self, Self::JsonSchema { .. })
    }
}

struct ProtoFile {
    /// The include-relative name (`nav/v2/nav.proto`).
    name: String,
    pool: DescriptorPool,
    artifact: String,
}

struct JsonFile {
    stem: String,
    doc: Value,
    artifact: String,
}

/// The compiled schemas of one contract.
pub struct SchemaSet {
    artifacts: BTreeMap<String, Artifact>,
    proto: Vec<ProtoFile>,
    json: Vec<JsonFile>,
    /// Set when a listed file failed to load, so unresolved references of
    /// that kind are not reported twice.
    proto_failed: bool,
    json_failed: bool,
}

/// The well-known types, by the file that defines them.
const WELL_KNOWN: &[(&str, &[&str])] = &[
    ("google/protobuf/any.proto", &["Any"]),
    ("google/protobuf/duration.proto", &["Duration"]),
    ("google/protobuf/empty.proto", &["Empty"]),
    ("google/protobuf/field_mask.proto", &["FieldMask"]),
    (
        "google/protobuf/struct.proto",
        &["Struct", "Value", "ListValue"],
    ),
    ("google/protobuf/timestamp.proto", &["Timestamp"]),
    (
        "google/protobuf/wrappers.proto",
        &[
            "DoubleValue",
            "FloatValue",
            "Int64Value",
            "UInt64Value",
            "Int32Value",
            "UInt32Value",
            "BoolValue",
            "StringValue",
            "BytesValue",
        ],
    ),
];

impl SchemaSet {
    /// Loads and compiles every listed schema file, relative to `dir`.
    pub fn load(dir: &Path, sec: &SchemasSection, report: &mut Report) -> Self {
        let mut set = Self {
            artifacts: BTreeMap::new(),
            proto: Vec::new(),
            json: Vec::new(),
            proto_failed: false,
            json_failed: false,
        };
        set.load_proto(dir, sec, report);
        set.load_json(dir, sec, report);
        set
    }

    fn load_proto(&mut self, dir: &Path, sec: &SchemasSection, report: &mut Report) {
        if sec.protobuf.is_empty() {
            return;
        }
        let includes: Vec<PathBuf> = match &sec.proto_include {
            Some(v) => v.iter().map(|p| dir.join(p)).collect(),
            None if dir.join("proto").is_dir() => vec![dir.join("proto")],
            None => vec![dir.to_path_buf()],
        };
        for f in &sec.protobuf {
            let at = format!("schemas.protobuf {f:?}");
            let compiled = protox::Compiler::new(&includes).and_then(|mut c| {
                c.include_imports(true).include_source_info(false);
                c.open_file(dir.join(f))?;
                Ok(c)
            });
            let c = match compiled {
                Ok(c) => c,
                Err(e) => {
                    report.push(Diagnostic::error(
                        "E029",
                        at,
                        format!("does not compile: {e}"),
                    ));
                    self.proto_failed = true;
                    continue;
                }
            };
            let Some(name) = c
                .files()
                .find(|m| !m.is_import())
                .map(|m| m.name().to_owned())
            else {
                report.push(Diagnostic::error("E029", at, "compiled to no file"));
                self.proto_failed = true;
                continue;
            };
            let art = Artifact {
                kind: SchemaKind::Protobuf,
                name: name.clone(),
                data: ArtifactData::Protobuf(c.encode_file_descriptor_set()),
            };
            let id = art.id();
            self.artifacts.insert(id.clone(), art);
            self.proto.push(ProtoFile {
                name,
                pool: c.descriptor_pool(),
                artifact: id,
            });
        }
    }

    fn load_json(&mut self, dir: &Path, sec: &SchemasSection, report: &mut Report) {
        let mut paths = Vec::new();
        for f in &sec.jsonschema {
            let at = format!("schemas.jsonschema {f:?}");
            let path = dir.join(f);
            let doc = match std::fs::read_to_string(&path) {
                Ok(text) => match crate::strict::parse_json(&text) {
                    Ok(v) => v,
                    Err(e) => {
                        report.push(Diagnostic::error(
                            "E029",
                            at,
                            format!("is not valid JSON: {e}"),
                        ));
                        self.json_failed = true;
                        continue;
                    }
                },
                Err(e) => {
                    report.push(Diagnostic::error(
                        "E029",
                        at,
                        format!("cannot be read: {e}"),
                    ));
                    self.json_failed = true;
                    continue;
                }
            };
            if let Some(n) = unsafe_integer(&doc) {
                report.push(Diagnostic::error(
                    "E028",
                    at.clone(),
                    format!("{n} is outside ±(2^53−1), so the artifact has no portable id"),
                ));
            }
            let mut refused = std::collections::BTreeSet::new();
            refused_keywords(&doc, &mut refused);
            for k in refused {
                report.push(Diagnostic::error(
                    "E037",
                    at.clone(),
                    format!("keyword {k:?} is outside the zk2 JSON Schema subset"),
                ));
            }
            let stem = file_stem(f);
            if self.json.iter().any(|j| j.stem == stem) {
                report.push(Diagnostic::error(
                    "E024",
                    at,
                    format!(
                        "a second listed file has the stem {stem:?}; stems are unique per contract"
                    ),
                ));
                continue;
            }
            let art = Artifact {
                kind: SchemaKind::JsonSchema,
                name: stem.clone(),
                data: ArtifactData::Json(doc.clone()),
            };
            let id = art.id();
            self.artifacts.insert(id.clone(), art);
            self.json.push(JsonFile {
                stem,
                doc,
                artifact: id,
            });
            paths.push((f.clone(), normalize(&path)));
        }
        // Cross-file `$ref`s: listed files only, existing definitions only.
        for (i, (f, path)) in paths.iter().enumerate() {
            let mut refs = Vec::new();
            collect_refs(&self.json[i].doc, &mut refs);
            for r in refs {
                let at = format!("schemas.jsonschema {f:?}");
                let (file_part, pointer) = r.split_once('#').unwrap_or((r.as_str(), ""));
                let target = if file_part.is_empty() {
                    Some(i)
                } else if file_part.contains(':') {
                    None
                } else {
                    let p = normalize(&path.parent().unwrap_or(Path::new("")).join(file_part));
                    paths.iter().position(|(_, q)| *q == p)
                };
                let Some(t) = target else {
                    report.push(Diagnostic::error(
                        "E032",
                        at,
                        format!("$ref {r:?} points outside the listed files"),
                    ));
                    continue;
                };
                if !pointer.is_empty() && self.json[t].doc.pointer(pointer).is_none() {
                    report.push(Diagnostic::error(
                        "E032",
                        at,
                        format!("$ref {r:?} points at a missing definition"),
                    ));
                }
            }
        }
    }

    /// Resolves a type reference, reporting E023/E024 at `at`.
    pub fn resolve(&mut self, r: &TypeRef, at: &str, report: &mut Report) -> Option<TypeId> {
        match r {
            TypeRef::Raw(raw) => resolve_raw(raw, at, report),
            TypeRef::Named(s) => {
                if let Some(rest) = s.strip_prefix("json:") {
                    self.resolve_json(s, rest, at, report)
                } else {
                    self.resolve_proto(s, at, report)
                }
            }
        }
    }

    fn resolve_json(
        &self,
        full: &str,
        rest: &str,
        at: &str,
        report: &mut Report,
    ) -> Option<TypeId> {
        let (stem, name) = match rest.split_once('#') {
            Some((stem, name)) => (Some(stem), name),
            None => (None, rest),
        };
        let has = |j: &JsonFile| j.doc.get("$defs").and_then(|d| d.get(name)).is_some();
        let found: Vec<&JsonFile> = self
            .json
            .iter()
            .filter(|j| stem.is_none_or(|s| j.stem == s) && has(j))
            .collect();
        match found.as_slice() {
            [j] => Some(TypeId::JsonSchema {
                name: name.to_owned(),
                schema: j.artifact.clone(),
            }),
            [] => {
                if !self.json_failed {
                    report.push(Diagnostic::error(
                        "E023",
                        at,
                        format!("{full:?}: no listed JSON Schema file defines $defs/{name}"),
                    ));
                }
                None
            }
            many => {
                let stems: Vec<&str> = many.iter().map(|j| j.stem.as_str()).collect();
                report.push(Diagnostic::error(
                    "E024",
                    at,
                    format!("{full:?} is defined in {stems:?}; qualify it as json:<stem>#{name}"),
                ));
                None
            }
        }
    }

    fn resolve_proto(&mut self, name: &str, at: &str, report: &mut Report) -> Option<TypeId> {
        let found: Vec<&ProtoFile> = self
            .proto
            .iter()
            .filter(|p| {
                p.pool
                    .get_message_by_name(name)
                    .is_some_and(|m| m.parent_file().name() == p.name)
            })
            .collect();
        match found.as_slice() {
            [p] => {
                return Some(TypeId::Protobuf {
                    name: name.to_owned(),
                    schema: p.artifact.clone(),
                });
            }
            [] => {}
            _ => {
                report.push(Diagnostic::error(
                    "E023",
                    at,
                    format!("{name:?} is defined by more than one listed .proto file"),
                ));
                return None;
            }
        }
        if let Some(short) = name.strip_prefix("google.protobuf.")
            && let Some((file, _)) = WELL_KNOWN.iter().find(|(_, ms)| ms.contains(&short))
        {
            return self.well_known(file, name);
        }
        if !self.proto_failed {
            let hint = if self
                .proto
                .iter()
                .any(|p| p.pool.get_message_by_name(name).is_some())
            {
                " (it is defined by an imported file: list that file)"
            } else {
                ""
            };
            report.push(Diagnostic::error(
                "E023",
                at,
                format!("{name:?}: no listed .proto file defines this message{hint}"),
            ));
        }
        None
    }

    fn well_known(&mut self, file: &str, name: &str) -> Option<TypeId> {
        if let Some(p) = self.proto.iter().find(|p| p.name == file) {
            return Some(TypeId::Protobuf {
                name: name.to_owned(),
                schema: p.artifact.clone(),
            });
        }
        let mut c = protox::Compiler::new::<_, &Path>([]).ok()?;
        c.include_imports(true).include_source_info(false);
        c.open_file(file).ok()?;
        let art = Artifact {
            kind: SchemaKind::Protobuf,
            name: file.to_owned(),
            data: ArtifactData::Protobuf(c.encode_file_descriptor_set()),
        };
        let id = art.id();
        self.artifacts.insert(id.clone(), art);
        self.proto.push(ProtoFile {
            name: file.to_owned(),
            pool: c.descriptor_pool(),
            artifact: id.clone(),
        });
        Some(TypeId::Protobuf {
            name: name.to_owned(),
            schema: id,
        })
    }

    /// The top-level field names of a message or object type, for the D21
    /// lint. `None` for raw types, or when the shape is not plain.
    #[must_use]
    pub fn field_names(&self, t: &TypeId) -> Option<Vec<String>> {
        match t {
            TypeId::Protobuf { name, schema } => {
                let p = self.proto.iter().find(|p| &p.artifact == schema)?;
                let m = p.pool.get_message_by_name(name)?;
                Some(m.fields().map(|f| f.name().to_owned()).collect())
            }
            TypeId::JsonSchema { name, schema } => {
                let j = self.json.iter().find(|j| &j.artifact == schema)?;
                let props = j
                    .doc
                    .get("$defs")?
                    .get(name)?
                    .get("properties")?
                    .as_object()?;
                Some(props.keys().cloned().collect())
            }
            TypeId::Raw { .. } => None,
        }
    }

    /// Every artifact, by id.
    #[must_use]
    pub fn artifacts(&self) -> &BTreeMap<String, Artifact> {
        &self.artifacts
    }
}

fn resolve_raw(raw: &RawType, at: &str, report: &mut Report) -> Option<TypeId> {
    if !is_media_type(&raw.raw) {
        report.push(Diagnostic::error(
            "E023",
            at,
            format!(
                "{:?} is not a lowercase media type <type>/<subtype> or family <type>/*",
                raw.raw
            ),
        ));
        return None;
    }
    Some(TypeId::Raw {
        media_type: raw.raw.clone(),
        media_param: raw.media_param.clone(),
    })
}

fn is_media_type(s: &str) -> bool {
    let tok = |t: &str| {
        let mut cs = t.chars();
        cs.next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            && cs.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "!#$&-^_.+".contains(c))
    };
    match s.split_once('/') {
        Some((t, sub)) => tok(t) && (sub == "*" || tok(sub)),
        None => false,
    }
}

fn file_stem(f: &str) -> String {
    let base = f.rsplit('/').next().unwrap_or(f);
    base.strip_suffix(".json").unwrap_or(base).to_owned()
}

/// Lexically normalizes a path (`.` and `..` folded), without touching the
/// filesystem.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

fn collect_refs(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                match (k.as_str(), x) {
                    ("$ref", Value::String(s)) => out.push(s.clone()),
                    _ => collect_refs(x, out),
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect_refs(x, out)),
        _ => {}
    }
}

/// RFC 8785 (JCS) bytes of a JSON value.
///
/// # Panics
/// Never for a `serde_json::Value`, which always serializes.
#[must_use]
pub fn jcs(v: &Value) -> Vec<u8> {
    serde_json_canonicalizer::to_vec(v).expect("a serde_json::Value always serializes")
}

/// `sha256:<lowercase hex>` of some bytes.
#[must_use]
pub fn sha256_id(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(71);
    s.push_str("sha256:");
    for b in digest {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// The first integer outside ±(2^53−1) in a JSON document, if any: JCS
/// implementations disagree beyond it (Python's `rfc8785` raises).
fn unsafe_integer(v: &Value) -> Option<String> {
    const MAX: u64 = (1 << 53) - 1;
    match v {
        Value::Number(n) => {
            let out = match (n.as_u64(), n.as_i64()) {
                (Some(u), _) => u > MAX,
                (None, Some(i)) => i.unsigned_abs() > MAX,
                _ => false,
            };
            out.then(|| n.to_string())
        }
        Value::Array(a) => a.iter().find_map(unsafe_integer),
        Value::Object(m) => m.values().find_map(unsafe_integer),
        _ => None,
    }
}

/// The zk2 JSON Schema subset (spec `core.md` §7.3): the keywords a schema
/// may use. Annotations are carried and ignored.
const SUBSET: &[&str] = &[
    "type",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "prefixItems",
    "enum",
    "const",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "minLength",
    "maxLength",
    "minItems",
    "maxItems",
    "$ref",
    "oneOf",
    "anyOf",
];
const ANNOTATIONS: &[&str] = &[
    "$schema",
    "$id",
    "$defs",
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

/// Walks the schema positions of a JSON Schema document (never property
/// names, which are data) and collects every keyword outside the subset.
fn refused_keywords(schema: &Value, out: &mut std::collections::BTreeSet<String>) {
    let Value::Object(m) = schema else { return };
    for (k, v) in m {
        if !SUBSET.contains(&k.as_str()) && !ANNOTATIONS.contains(&k.as_str()) {
            out.insert(k.clone());
            continue;
        }
        match (k.as_str(), v) {
            ("properties" | "$defs", Value::Object(subs)) => {
                for sub in subs.values() {
                    refused_keywords(sub, out);
                }
            }
            ("prefixItems" | "oneOf" | "anyOf", Value::Array(subs)) => {
                for sub in subs {
                    refused_keywords(sub, out);
                }
            }
            ("items" | "additionalProperties", sub) => refused_keywords(sub, out),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_types() {
        for ok in [
            "image/jpeg",
            "video/*",
            "application/vnd.foo+cbor",
            "audio/l16",
        ] {
            assert!(is_media_type(ok), "{ok}");
        }
        for bad in ["image", "Image/jpeg", "*/*", "image/", "image/jpeg; q=1"] {
            assert!(!is_media_type(bad), "{bad}");
        }
    }

    #[test]
    fn well_known_types_compile_on_demand() {
        let mut report = Report::default();
        let mut set = SchemaSet::load(Path::new("."), &SchemasSection::default(), &mut report);
        let t = set
            .resolve(
                &TypeRef::Named("google.protobuf.Empty".into()),
                "x",
                &mut report,
            )
            .unwrap();
        assert!(report.0.is_empty(), "{report}");
        let TypeId::Protobuf { schema, .. } = t else {
            panic!()
        };
        assert_eq!(set.artifacts()[&schema].name, "google/protobuf/empty.proto");
        assert!(
            set.resolve(
                &TypeRef::Named("google.protobuf.Nope".into()),
                "x",
                &mut report
            )
            .is_none()
        );
        assert_eq!(report.codes(), ["E023"]);
    }

    #[test]
    fn normalize_folds_dots() {
        assert_eq!(
            normalize(Path::new("a/./b/../c.json")),
            PathBuf::from("a/c.json")
        );
        assert_eq!(normalize(Path::new("../x")), PathBuf::from("../x"));
    }
}
