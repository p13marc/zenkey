//! Protobuf types: prost, compiled from the bundles' own descriptor sets
//! (`prost_build::Config::compile_fds`), so the types and the bundle cannot
//! disagree. No `protoc`.

use std::collections::BTreeMap;
use std::path::Path;

use prost::Message as _;
use prost_types::{DescriptorProto, FileDescriptorProto, FileDescriptorSet};
use zenkey_model::contract::Contract;
use zenkey_model::schema::{ArtifactData, SchemaKind};

use crate::Error;
use heck::{ToSnakeCase, ToUpperCamelCase};

/// Every protobuf file the contracts carry, by name, and which artifact
/// lists which files.
#[derive(Default)]
pub struct Protos {
    files: BTreeMap<String, FileDescriptorProto>,
    /// Artifact id → the package of its own (non-import) file.
    packages: BTreeMap<String, String>,
}

/// The well-known types prost maps to Rust types of its own.
fn well_known(name: &str) -> Option<String> {
    let short = name.strip_prefix("google.protobuf.")?;
    Some(match short {
        "Empty" => "()".to_owned(),
        "BoolValue" => "bool".to_owned(),
        "BytesValue" => "::std::vec::Vec<u8>".to_owned(),
        "DoubleValue" => "f64".to_owned(),
        "FloatValue" => "f32".to_owned(),
        "Int32Value" => "i32".to_owned(),
        "Int64Value" => "i64".to_owned(),
        "StringValue" => "::std::string::String".to_owned(),
        "UInt32Value" => "u32".to_owned(),
        "UInt64Value" => "u64".to_owned(),
        other => format!("::zenkey::prost_types::{other}"),
    })
}

impl Protos {
    /// Gathers the protobuf artifacts of `contracts`. Two artifacts carrying
    /// a file of one name with different content cannot share a module.
    pub fn gather(contracts: &[Contract]) -> Result<Self, Error> {
        let mut out = Self::default();
        for c in contracts {
            for (id, a) in &c.artifacts {
                let (SchemaKind::Protobuf, ArtifactData::Protobuf(bytes)) = (a.kind, &a.data)
                else {
                    continue;
                };
                let fds = FileDescriptorSet::decode(bytes.as_slice()).map_err(|e| {
                    Error::Codegen(format!(
                        "{}: artifact {id} is not a FileDescriptorSet: {e}",
                        c.iface
                    ))
                })?;
                for f in fds.file {
                    let name = f.name().to_owned();
                    if f.name() == a.name {
                        out.packages.insert(id.clone(), f.package().to_owned());
                    }
                    match out.files.get(&name) {
                        Some(prev) if *prev != f => {
                            return Err(Error::Codegen(format!(
                                "{}: two contracts carry {name:?} with different content; one module cannot hold both",
                                c.iface
                            )));
                        }
                        Some(_) => {}
                        None => {
                            out.files.insert(name, f);
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    /// Whether there is anything to compile (the well-known types are
    /// prost's own).
    pub fn is_empty(&self) -> bool {
        self.files
            .values()
            .all(|f| f.package() == "google.protobuf")
    }

    /// Compiles every file into `dir`, with `_includes.rs` as the module
    /// tree; prost is named through `::zenkey::prost`.
    pub fn compile(&self, dir: &Path) -> Result<(), Error> {
        std::fs::create_dir_all(dir).map_err(|e| Error::Io(dir.to_owned(), e))?;
        let fds = FileDescriptorSet {
            file: self
                .files
                .values()
                .filter(|f| f.package() != "google.protobuf")
                .cloned()
                .collect(),
        };
        prost_build::Config::new()
            .out_dir(dir)
            .include_file("_includes.rs")
            .prost_path("::zenkey::prost")
            .prost_types_path("::zenkey::prost_types")
            .compile_fds(fds)
            .map_err(|e| Error::Codegen(format!("prost-build: {e}")))
    }

    /// The module path of a package, below the `__proto` root:
    /// `nav.v2` → `nav::v2`.
    pub fn package_path(package: &str) -> String {
        prost_build::Module::from_protobuf_package_name(package)
            .parts()
            .collect::<Vec<_>>()
            .join("::")
    }

    /// The packages an interface's own protobuf artifacts define.
    pub fn packages_of(&self, c: &Contract) -> Vec<String> {
        let mut v: Vec<String> = c
            .artifacts
            .keys()
            .filter_map(|id| self.packages.get(id))
            .filter(|p| !p.is_empty() && *p != "google.protobuf")
            .cloned()
            .collect();
        v.sort();
        v.dedup();
        v
    }

    /// The Rust type of message `name`, relative to a module beside
    /// `__proto` (`super::__proto::nav::v2::Origin`), or a well-known
    /// type's own.
    pub fn rust_type(&self, name: &str, root: &str) -> Result<String, Error> {
        if let Some(t) = well_known(name) {
            return Ok(t);
        }
        for f in self.files.values() {
            let pkg = f.package();
            let rest = if pkg.is_empty() {
                Some(name)
            } else {
                name.strip_prefix(pkg).and_then(|r| r.strip_prefix('.'))
            };
            let Some(rest) = rest else { continue };
            let parts: Vec<&str> = rest.split('.').collect();
            if find(&f.message_type, &parts) {
                let mut path = vec![root.to_owned()];
                if !pkg.is_empty() {
                    path.push(Self::package_path(pkg));
                }
                for parent in &parts[..parts.len() - 1] {
                    path.push(prost_ident(&parent.to_snake_case()));
                }
                path.push(prost_ident(&parts[parts.len() - 1].to_upper_camel_case()));
                return Ok(path.join("::"));
            }
        }
        Err(Error::Codegen(format!("no compiled file defines {name:?}")))
    }
}

/// prost-build's identifier rule (`ident::sanitize_identifier`): a keyword
/// is a raw identifier, or takes `_` where it cannot be one.
fn prost_ident(s: &str) -> String {
    match s {
        "as" | "break" | "const" | "continue" | "else" | "enum" | "false" | "fn" | "for" | "if"
        | "impl" | "in" | "let" | "loop" | "match" | "mod" | "move" | "mut" | "pub" | "ref"
        | "return" | "static" | "struct" | "trait" | "true" | "type" | "unsafe" | "use"
        | "where" | "while" | "dyn" | "abstract" | "become" | "box" | "do" | "final" | "macro"
        | "override" | "priv" | "typeof" | "unsized" | "virtual" | "yield" | "async" | "await"
        | "try" | "gen" => format!("r#{s}"),
        "_" | "super" | "self" | "Self" | "extern" | "crate" => format!("{s}_"),
        s if s.starts_with(|c: char| c.is_numeric()) => format!("_{s}"),
        _ => s.to_owned(),
    }
}

fn find(messages: &[DescriptorProto], parts: &[&str]) -> bool {
    let Some((first, rest)) = parts.split_first() else {
        return false;
    };
    messages
        .iter()
        .any(|m| m.name() == *first && (rest.is_empty() || find(&m.nested_type, rest)))
}
