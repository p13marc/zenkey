//! zk2 codegen (#611, `docs/zk2/codegen.md`): typed servers, consumers and
//! clients from zk2 contracts, on the `zenkey` 0.20 runtime.
//!
//! Call it from a build script:
//!
//! ```no_run
//! // build.rs
//! zenkey_build::Config::new()
//!     .contracts_dir("contracts")              // *.toml, the authoring format (spec §9.1)
//!     .history_dir("contracts/.history")       // optional: the compatibility gate (§9.7, §9.8)
//!     .json_type("json:Status", "crate::model::Status") // optional name hints (G23)
//!     .generate()
//!     .unwrap();
//! ```
//!
//! ```ignore
//! // src/zk2.rs
//! include!(concat!(env!("OUT_DIR"), "/zk2.rs"));
//! ```
//!
//! [`Config::generate`] fails the consumer's build, where the contract was
//! authored:
//! - on **every lint error** of `zenkey-model` (stable codes; warnings go to
//!   `cargo:warning`), and the set's checks (E035, E036);
//! - **with a history**, on a candidate that is `breaking` against any
//!   published revision (`zenkey_model::compat::check_history`, the rules
//!   of `zk2 contract compat --history`); `review` is a `cargo:warning`
//!   naming each finding.
//!
//! It emits `cargo:rerun-if-changed` for every contract, every schema file
//! and include root it read, and the history.
//!
//! **What is generated**, one module per interface (`thruster.v1` →
//! `thruster_v1`): `IFACE`, `FINGERPRINT`, `BUNDLE` (built once here, by
//! `zenkey-model`'s bundle builder, and embedded; never rebuilt at run
//! time), `implementation()`, `contract()`, the `types`, the `resource`
//! names, and, as the contract has them, the `Handlers` and `Api` traits,
//! `Server`, `Consumer`, `Client` and `Fleet`. Protobuf types are prost's,
//! compiled from the bundle's own descriptor sets (no `protoc`); JSON
//! Schema types are typify's, or the Rust type a hint names
//! ([`check_schema`] keeps such a type and its committed schema in step).
//!
//! **Contract crates** ([`Config::contract_crate`]): every runtime item
//! behind `#[cfg(feature = "zenoh")]`, so the crate's default build has no
//! zenoh dependency (`zenkey` with `default-features = false`).
//!
//! The generated code names `zenkey` (and through it `prost`); JSON Schema
//! types also need `serde` and `serde_json` in the consumer's dependencies.

mod emit;
mod json;
mod names;
mod proto;
mod schema_check;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use zenkey_model::bundle::Bundle;
use zenkey_model::compat::{Class, Revision, check_history};
use zenkey_model::contract::{Contract, check_set, load_path};

pub use schema_check::check_schema;

/// A codegen failure. Each fails the consumer's build with its message.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A contract does not lint: every error, by file.
    #[error("zk2 contract lint failed [{file}]:\n{report}")]
    Lint { file: String, report: String },
    /// The set's checks (E035, E036) failed.
    #[error("zk2 contract set:\n{0}")]
    Set(String),
    /// The history directory is not a valid history (layout, integrity).
    #[error("zk2 history {dir}:\n{problems}")]
    History { dir: String, problems: String },
    /// A candidate is breaking against a published revision (spec §9.8).
    #[error(
        "zk2 compatibility: {iface} is breaking against its history (spec §9.8):\n{findings}\na breaking change is a new major"
    )]
    Breaking { iface: String, findings: String },
    #[error("{0:?}: {1}")]
    Io(PathBuf, #[source] std::io::Error),
    #[error(
        "OUT_DIR is not set and no out_file was given: call from a build script or set .out_file(..)"
    )]
    NoOutDir,
    /// The configuration is not usable (a hint that is not a type
    /// reference, no contracts directory).
    #[error("zenkey-build configuration: {0}")]
    Config(String),
    /// prost-build or typify refused, or a type cannot be named.
    #[error("zk2 codegen: {0}")]
    Codegen(String),
}

/// What [`Config::generate`] did.
#[derive(Debug, Clone, Default)]
pub struct Generated {
    /// The file to `include!`.
    pub out_file: PathBuf,
    /// The interfaces generated, in order.
    pub interfaces: Vec<String>,
    /// Lint warnings, compatibility `review` findings and warnings, unused
    /// hints: each also printed as `cargo:warning` under a build script.
    pub warnings: Vec<String>,
}

/// The codegen configuration: where the contracts are, and what to emit.
#[derive(Debug, Clone, Default)]
pub struct Config {
    contracts_dirs: Vec<PathBuf>,
    history_dir: Option<PathBuf>,
    json_types: BTreeMap<String, String>,
    contract_crate: bool,
    out_file: Option<PathBuf>,
}

impl Config {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A directory of contract files (`*.toml`, the authoring format;
    /// `*.bindings.toml` deployment files are skipped). Not recursive; call
    /// it once per directory. Relative paths are the build script's (the
    /// package root).
    #[must_use]
    pub fn contracts_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.contracts_dirs.push(dir.into());
        self
    }

    /// The published revisions (`<dir>/<iface>/<hex>.bundle.json`, r3
    /// §3.11): every contract is checked against its interface's history,
    /// FULL_TRANSITIVE (spec §9.8). A directory that does not exist is an
    /// empty history.
    #[must_use]
    pub fn history_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.history_dir = Some(dir.into());
        self
    }

    /// A name hint (G23): the JSON Schema type reference `json:<Name>` (or
    /// `json:<stem>#<Name>`) is the existing Rust type `rust_path`, which must
    /// be `Serialize + DeserializeOwned`. Nothing is generated for it; the
    /// types that reference it name it.
    #[must_use]
    pub fn json_type(mut self, reference: &str, rust_path: &str) -> Self {
        self.json_types
            .insert(reference.to_owned(), rust_path.to_owned());
        self
    }

    /// Contract-crate mode: every runtime item behind
    /// `#[cfg(feature = "zenoh")]`. The crate depends on `zenkey` with
    /// `default-features = false` and forwards a `zenoh` feature to
    /// `zenkey/zenoh`.
    #[must_use]
    pub fn contract_crate(mut self, on: bool) -> Self {
        self.contract_crate = on;
        self
    }

    /// Where the generated file goes: `$OUT_DIR/zk2.rs` unless set. The
    /// embedded bundles and the generated types go beside it, in `zk2.d/`.
    #[must_use]
    pub fn out_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.out_file = Some(path.into());
        self
    }

    /// Lints, checks compatibility, builds the bundles and generates the
    /// code. Under a build script, prints `cargo:rerun-if-changed` and
    /// `cargo:warning` lines.
    ///
    /// # Errors
    /// See [`Error`]: every one fails the build that called this.
    pub fn generate(&self) -> Result<Generated, Error> {
        let cargo = std::env::var_os("OUT_DIR").is_some();
        let out_file = match &self.out_file {
            Some(p) => p.clone(),
            None => {
                PathBuf::from(std::env::var_os("OUT_DIR").ok_or(Error::NoOutDir)?).join("zk2.rs")
            }
        };
        let out_file = absolute(&out_file)?;
        let mut generated = Generated {
            out_file: out_file.clone(),
            ..Generated::default()
        };
        let mut rerun: Vec<PathBuf> = Vec::new();

        let loaded = self.load(&mut generated.warnings, &mut rerun)?;
        let contracts: Vec<Contract> = loaded.iter().map(|(_, c)| c.clone()).collect();
        let set = check_set(&contracts.iter().collect::<Vec<_>>());
        if set.has_errors() {
            return Err(Error::Set(set.to_string()));
        }
        if let Some(h) = &self.history_dir {
            rerun.push(h.clone());
            compat(h, &contracts, &mut generated.warnings)?;
        }

        let work = out_file
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("zk2.d");
        std::fs::create_dir_all(&work).map_err(|e| Error::Io(work.clone(), e))?;

        let hints = self
            .json_types
            .iter()
            .map(|(k, v)| json::Hint::parse(k, v))
            .collect::<Result<Vec<_>, _>>()?;
        let protos = proto::Protos::gather(&contracts)?;
        let proto_dir = work.join("proto");
        if !protos.is_empty() {
            protos.compile(&proto_dir)?;
        }
        let jsons = json::JsonTypes::gather(&contracts, hints);
        for h in jsons.unused_hints() {
            generated.warnings.push(format!(
                "json_type {:?} names no definition of any contract's JSON Schema files",
                h.key
            ));
        }
        let json_files = jsons.compile(&work.join("json"))?;

        let mut ifaces = Vec::new();
        for (file, c) in &loaded {
            let b = Bundle::build(c);
            let path = work.join(format!("{}.bundle.json", c.iface));
            write_if_changed(&path, &b.to_bytes())?;
            ifaces.push(emit::Iface {
                contract: c,
                source: file.clone(),
                fingerprint: b.fingerprint().to_string(),
                bundle_path: path,
            });
            generated.interfaces.push(c.iface.to_string());
        }
        let code = emit::emit(&emit::Unit {
            ifaces: &ifaces,
            protos: &protos,
            proto_includes: (!protos.is_empty()).then(|| proto_dir.join("_includes.rs")),
            jsons: &jsons,
            json_files: &json_files,
            contract_crate: self.contract_crate,
        })?;
        write_if_changed(&out_file, code.as_bytes())?;

        if cargo {
            for p in &rerun {
                println!("cargo:rerun-if-changed={}", p.display());
            }
            for w in &generated.warnings {
                println!("cargo:warning={}", w.replace('\n', " "));
            }
        }
        Ok(generated)
    }

    /// Loads and lints every contract file, in directory then name order.
    fn load(
        &self,
        warnings: &mut Vec<String>,
        rerun: &mut Vec<PathBuf>,
    ) -> Result<Vec<(PathBuf, Contract)>, Error> {
        if self.contracts_dirs.is_empty() {
            return Err(Error::Config("no contracts_dir".into()));
        }
        let mut out = Vec::new();
        for dir in &self.contracts_dirs {
            rerun.push(dir.clone());
            let entries = std::fs::read_dir(dir).map_err(|e| Error::Io(dir.clone(), e))?;
            let mut files: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.ends_with(".toml") && !n.ends_with(".bindings.toml"))
                })
                .collect();
            files.sort();
            for f in files {
                rerun.push(f.clone());
                rerun.extend(schema_inputs(&f));
                let l = load_path(&f);
                let shown = f.display().to_string();
                for w in l.report.warnings() {
                    warnings.push(format!("{shown}: {w}"));
                }
                match l.contract {
                    Some(c) => out.push((f, c)),
                    None => {
                        let errors: Vec<String> =
                            l.report.errors().map(ToString::to_string).collect();
                        return Err(Error::Lint {
                            file: shown,
                            report: errors.join("\n"),
                        });
                    }
                }
            }
        }
        Ok(out)
    }
}

/// The schema files and include roots a contract file names, for
/// `rerun-if-changed`: read from its authoring form, whatever the lints say
/// of it (an import outside the listed files is under an include root).
fn schema_inputs(file: &Path) -> Vec<PathBuf> {
    let dir = file.parent().unwrap_or(Path::new("."));
    let Ok(text) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    let Ok(f) = toml::from_str::<zenkey_model::authoring::ContractFile>(&text) else {
        return Vec::new();
    };
    let s = f.schemas;
    let includes = match s.proto_include {
        Some(v) => v,
        None if !s.protobuf.is_empty() && dir.join("proto").is_dir() => vec!["proto".to_owned()],
        None => Vec::new(),
    };
    s.protobuf
        .into_iter()
        .chain(includes)
        .chain(s.jsonschema)
        .map(|p| dir.join(p))
        .collect()
}

/// The compatibility gate (spec §9.8): breaking fails, review warns.
fn compat(root: &Path, contracts: &[Contract], warnings: &mut Vec<String>) -> Result<(), Error> {
    if !root.exists() {
        return Ok(());
    }
    let problems = zenkey_model::history::check(root);
    if !problems.is_empty() {
        return Err(Error::History {
            dir: root.display().to_string(),
            problems: problems.join("\n"),
        });
    }
    for c in contracts {
        let dir = root.join(c.iface.to_string());
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        files.sort();
        let mut history = Vec::new();
        for f in files {
            let bytes = std::fs::read(&f).map_err(|e| Error::Io(f.clone(), e))?;
            let b = Bundle::verify(&bytes).map_err(|e| Error::History {
                dir: root.display().to_string(),
                problems: format!("{}: {e}", f.display()),
            })?;
            history.push(Revision::of_bundle(&b));
        }
        let v = check_history(&history, &Revision::of(c));
        let line = |f: &zenkey_model::compat::Finding| {
            format!(
                "{}: {} {} at {}: {}",
                c.iface,
                f.class.as_str(),
                f.rule,
                f.at,
                f.detail
            )
        };
        if v.class() == Class::Breaking {
            return Err(Error::Breaking {
                iface: c.iface.to_string(),
                findings: v.findings.iter().map(line).collect::<Vec<_>>().join("\n"),
            });
        }
        warnings.extend(v.findings.iter().map(line));
        warnings.extend(
            v.warnings
                .iter()
                .map(|w| format!("{}: warning {} at {}: {}", c.iface, w.rule, w.at, w.detail)),
        );
    }
    Ok(())
}

fn absolute(p: &Path) -> Result<PathBuf, Error> {
    std::path::absolute(p).map_err(|e| Error::Io(p.to_owned(), e))
}

/// Writes a file unless it already holds these bytes, so an unchanged
/// generation does not rebuild its dependents.
pub(crate) fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    if std::fs::read(path).is_ok_and(|b| b == bytes) {
        return Ok(());
    }
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| Error::Io(d.to_owned(), e))?;
    }
    std::fs::write(path, bytes).map_err(|e| Error::Io(path.to_owned(), e))
}
