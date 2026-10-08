//! JSON Schema types: serde types generated from the contracts' schemas
//! (`typify`), or existing Rust types named by a hint (G23, the Rust-first
//! path where the schema was produced by schemars).
//!
//! The JSON Schema files a contract lists form one *group*: their `$defs`
//! become one module, with cross-file `$ref`s resolved by file stem as in
//! a bundle (spec §7.3). Contracts listing the same files share the group,
//! so the tcgui pilot's three interfaces share one `TcError`. A definition
//! keeps its name unless two files of the group define it, when it is
//! qualified by its file's stem.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use heck::ToPascalCase;
use serde_json::{Map, Value};
use zenkey_model::contract::Contract;
use zenkey_model::schema::{ArtifactData, SchemaKind};

use crate::Error;
use crate::names::snake;

/// One group of JSON Schema artifacts and its module.
struct Group {
    module: String,
    /// (artifact id, stem, document), in id order.
    artifacts: Vec<(String, String, Value)>,
    /// (stem, definition) → the definition's key in the group.
    keys: BTreeMap<(String, String), String>,
}

/// typify's name for a definition key (its `sanitize(.., Pascal)`).
fn rust_name(key: &str) -> String {
    let cleaned: String = key
        .replace('\'', "")
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let out = cleaned.to_pascal_case();
    match out.chars().next() {
        Some(c) if c.is_alphabetic() => out,
        _ => format!("X{out}"),
    }
}

/// A name hint (`json:Name`, `json:stem#Name`), parsed.
#[derive(Debug, Clone)]
pub struct Hint {
    pub key: String,
    stem: Option<String>,
    name: String,
    pub path: String,
}

impl Hint {
    pub fn parse(key: &str, path: &str) -> Result<Self, Error> {
        let rest = key.strip_prefix("json:").ok_or_else(|| {
            Error::Config(format!(
                "json_type {key:?}: a hint names a JSON Schema type reference, json:<Name> or json:<stem>#<Name>"
            ))
        })?;
        let (stem, name) = match rest.split_once('#') {
            Some((s, n)) => (Some(s.to_owned()), n.to_owned()),
            None => (None, rest.to_owned()),
        };
        if name.is_empty() || path.trim().is_empty() {
            return Err(Error::Config(format!(
                "json_type {key:?} -> {path:?}: empty"
            )));
        }
        Ok(Self {
            key: key.to_owned(),
            stem,
            name,
            path: path.to_owned(),
        })
    }

    fn matches(&self, stem: &str, name: &str) -> bool {
        self.name == name && self.stem.as_deref().is_none_or(|s| s == stem)
    }
}

/// Every group, and which contract uses which.
#[derive(Default)]
pub struct JsonTypes {
    groups: Vec<Group>,
    /// Interface id → group index.
    by_iface: BTreeMap<String, usize>,
    hints: Vec<Hint>,
}

fn stem_of_ref_file(file: &str) -> String {
    let base = file.rsplit('/').next().unwrap_or(file);
    base.strip_suffix(".json").unwrap_or(base).to_owned()
}

/// Visits every schema object at a schema position (spec §7.3), mutably.
fn visit_mut(schema: &mut Value, f: &mut dyn FnMut(&mut Map<String, Value>)) {
    let Value::Object(m) = schema else { return };
    f(m);
    for (k, v) in m.iter_mut() {
        match (k.as_str(), v) {
            ("properties" | "$defs", Value::Object(subs)) => {
                subs.values_mut().for_each(|s| visit_mut(s, f));
            }
            ("prefixItems" | "oneOf" | "anyOf", Value::Array(subs)) => {
                subs.iter_mut().for_each(|s| visit_mut(s, f));
            }
            ("items" | "additionalProperties", sub) => visit_mut(sub, f),
            _ => {}
        }
    }
}

impl JsonTypes {
    pub fn gather(contracts: &[Contract], hints: Vec<Hint>) -> Self {
        let mut out = Self {
            hints,
            ..Self::default()
        };
        let mut by_ids: BTreeMap<Vec<String>, usize> = BTreeMap::new();
        for c in contracts {
            let arts: Vec<(String, String, Value)> = c
                .artifacts
                .iter()
                .filter_map(|(id, a)| match (a.kind, &a.data) {
                    (SchemaKind::JsonSchema, ArtifactData::Json(v)) => {
                        Some((id.clone(), a.name.clone(), v.clone()))
                    }
                    _ => None,
                })
                .collect();
            if arts.is_empty() {
                continue;
            }
            let ids: Vec<String> = arts.iter().map(|(id, _, _)| id.clone()).collect();
            let i = *by_ids.entry(ids).or_insert_with(|| {
                out.groups.push(Group::new(arts));
                out.groups.len() - 1
            });
            out.by_iface.insert(c.iface.to_string(), i);
        }
        // Module names: the stems, unique.
        let mut used = BTreeSet::new();
        for g in &mut out.groups {
            let stems: Vec<String> = g.artifacts.iter().map(|(_, s, _)| snake(s)).collect();
            let base = stems.join("_");
            let mut name = base.clone();
            let mut n = 2;
            while !used.insert(name.clone()) {
                name = format!("{base}_{n}");
                n += 1;
            }
            g.module = name;
        }
        out
    }

    /// The hints that name no definition of any group: a likely typo.
    pub fn unused_hints(&self) -> Vec<&Hint> {
        self.hints
            .iter()
            .filter(|h| {
                !self
                    .groups
                    .iter()
                    .any(|g| g.keys.keys().any(|(stem, name)| h.matches(stem, name)))
            })
            .collect()
    }

    fn hint(&self, stem: &str, name: &str) -> Option<&Hint> {
        // A stem-qualified hint wins over a bare one.
        self.hints
            .iter()
            .filter(|h| h.matches(stem, name))
            .max_by_key(|h| h.stem.is_some())
    }

    /// Generates every group's types into `dir`, one file per group.
    /// Returns (module, file) pairs.
    pub fn compile(&self, dir: &Path) -> Result<Vec<(String, std::path::PathBuf)>, Error> {
        std::fs::create_dir_all(dir).map_err(|e| Error::Io(dir.to_owned(), e))?;
        let mut out = Vec::new();
        for g in &self.groups {
            let mut settings = typify::TypeSpaceSettings::default();
            settings.with_derive("PartialEq".to_owned());
            for ((stem, name), key) in &g.keys {
                if let Some(h) = self.hint(stem, name) {
                    settings.with_replacement(rust_name(key), &h.path, std::iter::empty());
                }
            }
            let mut space = typify::TypeSpace::new(&settings);
            let defs = g.definitions()?;
            space
                .add_ref_types(defs)
                .map_err(|e| Error::Codegen(format!("typify, schemas {}: {e}", g.module)))?;
            let file = dir.join(format!("{}.rs", g.module));
            let text = format!(
                "// @generated by zenkey-build from {}. Do not edit.\n{}\n",
                g.artifacts
                    .iter()
                    .map(|(id, stem, _)| format!("{stem}.json ({id})"))
                    .collect::<Vec<_>>()
                    .join(", "),
                space.to_stream()
            );
            crate::write_if_changed(&file, text.as_bytes())?;
            out.push((g.module.clone(), file));
        }
        Ok(out)
    }

    /// The group module of an interface, if it lists JSON Schema files.
    pub fn module_of(&self, iface: &str) -> Option<&str> {
        self.by_iface
            .get(iface)
            .map(|i| self.groups[*i].module.as_str())
    }

    /// The hinted types of an interface's group: (Rust name, path).
    pub fn hinted_of(&self, iface: &str) -> Vec<(String, String)> {
        let Some(i) = self.by_iface.get(iface) else {
            return Vec::new();
        };
        let g = &self.groups[*i];
        g.keys
            .iter()
            .filter_map(|((stem, name), key)| {
                self.hint(stem, name)
                    .map(|h| (rust_name(key), h.path.clone()))
            })
            .collect()
    }

    /// The Rust type of `json:<name>` in artifact `schema`, as seen from a
    /// module beside `__json` (`root` is the path to their parent).
    pub fn rust_type(
        &self,
        iface: &str,
        name: &str,
        schema: &str,
        root: &str,
    ) -> Result<String, Error> {
        let g = self
            .by_iface
            .get(iface)
            .map(|i| &self.groups[*i])
            .ok_or_else(|| Error::Codegen(format!("{iface} lists no JSON Schema file")))?;
        let stem = g
            .artifacts
            .iter()
            .find(|(id, _, _)| id == schema)
            .map(|(_, s, _)| s.clone())
            .ok_or_else(|| Error::Codegen(format!("{iface}: no artifact {schema}")))?;
        if let Some(h) = self.hint(&stem, name) {
            return Ok(h.path.clone());
        }
        let key = g
            .keys
            .get(&(stem.clone(), name.to_owned()))
            .ok_or_else(|| Error::Codegen(format!("{iface}: {stem}.json has no $defs/{name}")))?;
        Ok(format!("{root}::__json::{}::{}", g.module, rust_name(key)))
    }
}

impl Group {
    fn new(artifacts: Vec<(String, String, Value)>) -> Self {
        let mut count: BTreeMap<String, usize> = BTreeMap::new();
        for (_, _, doc) in &artifacts {
            for name in doc
                .get("$defs")
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|m| m.keys())
            {
                *count.entry(name.clone()).or_default() += 1;
            }
        }
        let mut keys = BTreeMap::new();
        for (_, stem, doc) in &artifacts {
            for name in doc
                .get("$defs")
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|m| m.keys())
            {
                let key = if count[name] > 1 {
                    format!("{}{}", stem.to_pascal_case(), name)
                } else {
                    name.clone()
                };
                keys.insert((stem.clone(), name.clone()), key);
            }
        }
        Self {
            module: String::new(),
            artifacts,
            keys,
        }
    }

    /// Every definition of the group, keyed and with its `$ref`s rewritten
    /// to the group's keys; `format` (an annotation in the zk2 subset) is
    /// dropped, so a string stays a `String`.
    fn definitions(&self) -> Result<Vec<(String, schemars08::schema::Schema)>, Error> {
        let mut out = Vec::new();
        for (_, stem, doc) in &self.artifacts {
            let Some(defs) = doc.get("$defs").and_then(Value::as_object) else {
                continue;
            };
            for (name, schema) in defs {
                let mut schema = schema.clone();
                let mut bad = None;
                visit_mut(&mut schema, &mut |m| {
                    m.remove("format");
                    if let Some(Value::String(r)) = m.get_mut("$ref") {
                        let (file, pointer) = r.split_once('#').unwrap_or((r.as_str(), ""));
                        let target_stem = if file.is_empty() {
                            stem.clone()
                        } else {
                            stem_of_ref_file(file)
                        };
                        if let Some(def) = pointer.strip_prefix("/$defs/")
                            && !def.contains('/')
                        {
                            match self.keys.get(&(target_stem, def.to_owned())) {
                                Some(k) => *r = format!("#/$defs/{k}"),
                                None => bad = Some(r.clone()),
                            }
                        } else {
                            bad = Some(r.clone());
                        }
                    }
                });
                if let Some(r) = bad {
                    return Err(Error::Codegen(format!(
                        "{stem}.json $defs/{name}: $ref {r:?} names no definition of the contract's files"
                    )));
                }
                let key = self.keys[&(stem.clone(), name.clone())].clone();
                let parsed: schemars08::schema::Schema = serde_json::from_value(schema)
                    .map_err(|e| Error::Codegen(format!("{stem}.json $defs/{name}: {e}")))?;
                out.push((key, parsed));
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::{Hint, rust_name};

    #[test]
    fn names_and_hints() {
        assert_eq!(rust_name("TcConfigUpdate"), "TcConfigUpdate");
        assert_eq!(rust_name("tc-error"), "TcError");
        let h = Hint::parse("json:tc#TcError", "crate::TcError").unwrap();
        assert!(h.matches("tc", "TcError") && !h.matches("other", "TcError"));
        assert!(Hint::parse("TcError", "crate::TcError").is_err());
    }
}
