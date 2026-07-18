//! Registry codegen for the keyspace-v2 convention (RFC 08).
//!
//! An application owns its subject vocabulary: `registry/*.toml` files checked
//! into the application's repository (RFC 08 §5). This crate is the build-time
//! half of that contract — call it from your build script:
//!
//! ```no_run
//! // build.rs
//! zenkey_build::Config::new()
//!     .registry_dir("registry")
//!     .generate()
//!     .unwrap();
//! ```
//!
//! ```ignore
//! // src/registry.rs
//! include!(concat!(env!("OUT_DIR"), "/zenkey_registry.rs"));
//! ```
//!
//! The generated module contains, per producer/service: a `Subject` enum with
//! typed constructors and a precedence-ordered parser, a `ProcedureId` enum
//! with `@rpc` key builders, and the raw registry slice served by
//! `introspect` (RFC 08 §6); plus the cross-producer `AnySubject` dispatch,
//! `REGISTRIES`, `registry_toml()`, and `is_registered_telemetry()`.
//!
//! Lints (RFC 08 §5) are errors returned from [`Config::generate`] — a
//! violating registry file fails the *consumer's* build, where the TOML was
//! authored. Codegen is normative in both directions (RFC 08 §1): an
//! unregistered subject does not construct, and a metric name refines into a
//! typed subject with named variables instead of positional `split('/')`.

mod emit;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use zenkey::grammar::{is_valid_plain_chunk, is_valid_verbatim_chunk};

/// A codegen failure. Lint variants carry the registry file they were found
/// in — surface them with `unwrap()` in the build script so the message
/// reaches the build output.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("registry lint failed [{file}]: {message}")]
    Lint { file: String, message: String },
    #[error("registry dir {0:?}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error(
        "OUT_DIR is not set and no out_file was given — call from a build script or set .out_file(..)"
    )]
    NoOutDir,
}

fn lint(file: &str, message: impl Into<String>) -> Error {
    Error::Lint {
        file: file.to_string(),
        message: message.into(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Chunk {
    Literal(String),
    Var(String),
    Rest(String),
}

pub(crate) struct SubjectEntry {
    pub path: String,
    pub chunks: Vec<Chunk>,
    pub class: String,
    pub payload_type: String,
    pub unit: Option<String>,
    pub cardinality: Option<i64>,
    pub qos: String,
    pub ttl_s: Option<i64>,
    pub rate: Option<String>,
    pub variant: String,
    /// `common = "..."` — the RFC-defined framework state subject this entry
    /// declares itself as (drives `AnySubject::common_state()`).
    pub common: Option<String>,
}

pub(crate) struct ProcedureEntry {
    pub path: String,
    pub chunks: Vec<String>,
    pub kind: String,
    pub request: Option<String>,
    pub reply: Option<String>,
    pub variant: String,
}

pub(crate) struct RegistryFile {
    /// Producer base name, or service name for `[service]` files.
    pub name: String,
    /// `Some("@catalog")`-style origin for services, `None` for producers.
    pub service_origin: Option<String>,
    pub toml_path: String,
    pub subjects: Vec<SubjectEntry>,
    pub procedures: Vec<ProcedureEntry>,
    pub deprecated: Vec<String>,
}

/// The RFC-defined framework state subjects a `common = "..."` field may name
/// (RFC 04 §1.2/§5, RFC 06 §4/§5), with the `zenkey::CommonState` constructor
/// and the variable names the subject pattern must bind.
pub(crate) const COMMON_STATE: &[(&str, &str, &[&str])] = &[
    ("health", "Health", &[]),
    ("errors", "Errors", &[]),
    ("sensor", "Sensor", &[]),
    ("alert", "Alert { alert_key }", &["alert_key"]),
    ("evidence_self", "EvidenceSelf", &[]),
    ("evidence_device", "EvidenceDevice { device }", &["device"]),
    ("evidence_names", "EvidenceNames { ip_slug }", &["ip_slug"]),
    ("entity", "CatalogEntity { entity_id }", &["entity_id"]),
    ("alias", "CatalogAlias { old_id }", &["old_id"]),
    ("pdns", "CatalogPdns { ip_slug }", &["ip_slug"]),
];

/// Builder for one codegen run. See the crate docs for the two-line consumer
/// integration.
#[derive(Debug)]
pub struct Config {
    registry_dir: PathBuf,
    out_file: Option<PathBuf>,
    zenkey_path: String,
    ledger: Option<PathBuf>,
    emit_rerun_if_changed: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

impl Config {
    pub fn new() -> Self {
        Config {
            registry_dir: PathBuf::from("registry"),
            out_file: None,
            zenkey_path: "::zenkey".to_string(),
            ledger: None,
            emit_rerun_if_changed: true,
        }
    }

    /// The directory holding `*.toml` registry files (default `registry`,
    /// relative to the consuming crate's manifest).
    pub fn registry_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.registry_dir = dir.as_ref().to_path_buf();
        self
    }

    /// Where the generated module is written
    /// (default `$OUT_DIR/zenkey_registry.rs`).
    pub fn out_file(mut self, f: impl AsRef<Path>) -> Self {
        self.out_file = Some(f.as_ref().to_path_buf());
        self
    }

    /// The path the generated code uses to reach the `zenkey` crate
    /// (default `::zenkey`) — override for renamed-dependency setups.
    pub fn zenkey_path(mut self, p: &str) -> Self {
        self.zenkey_path = p.to_string();
        self
    }

    /// The append-only deprecation ledger
    /// (default `<registry_dir>/deprecated.lock`; a missing file is an empty
    /// ledger).
    pub fn ledger(mut self, f: impl AsRef<Path>) -> Self {
        self.ledger = Some(f.as_ref().to_path_buf());
        self
    }

    /// Suppress the `cargo::rerun-if-changed` lines (default on) — for
    /// calling outside a build script.
    pub fn no_rerun_if_changed(mut self) -> Self {
        self.emit_rerun_if_changed = false;
        self
    }

    /// Lint the registry (RFC 08 §5), check the deprecation ledger
    /// (RFC 08 §3), and write the generated module.
    pub fn generate(self) -> Result<(), Error> {
        let out = match &self.out_file {
            Some(f) => f.clone(),
            None => Path::new(&std::env::var("OUT_DIR").map_err(|_| Error::NoOutDir)?)
                .join("zenkey_registry.rs"),
        };
        let generated = self.generate_string()?;
        std::fs::write(&out, generated).map_err(|e| Error::Io(out, e))?;
        Ok(())
    }

    /// As [`generate`](Self::generate), returning the generated source
    /// instead of writing it.
    pub fn generate_string(&self) -> Result<String, Error> {
        if self.emit_rerun_if_changed {
            println!("cargo::rerun-if-changed={}", self.registry_dir.display());
            if let Some(l) = &self.ledger {
                println!("cargo::rerun-if-changed={}", l.display());
            }
        }
        let files = load_registry(&self.registry_dir)?;
        let ledger = self
            .ledger
            .clone()
            .unwrap_or_else(|| self.registry_dir.join("deprecated.lock"));
        check_deprecation_ledger(&ledger, &files)?;
        Ok(emit::emit(&files, &self.zenkey_path))
    }
}

fn parse_pattern(file: &str, path: &str) -> Result<Vec<Chunk>, Error> {
    let mut chunks = Vec::new();
    let parts: Vec<&str> = path.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        if let Some(var) = part.strip_prefix('{').and_then(|p| p.strip_suffix("...}")) {
            if i != parts.len() - 1 {
                return Err(lint(
                    file,
                    format!("{path:?}: {{var...}} only in trailing position (RFC 08 §2)"),
                ));
            }
            if !is_valid_plain_chunk(var) {
                return Err(lint(
                    file,
                    format!("{path:?}: bad rest-variable name {var:?}"),
                ));
            }
            chunks.push(Chunk::Rest(var.to_string()));
        } else if let Some(var) = part.strip_prefix('{').and_then(|p| p.strip_suffix('}')) {
            if !is_valid_plain_chunk(var) {
                return Err(lint(file, format!("{path:?}: bad variable name {var:?}")));
            }
            chunks.push(Chunk::Var(var.to_string()));
        } else {
            if !is_valid_plain_chunk(part) {
                return Err(lint(
                    file,
                    format!("{path:?}: chunk {part:?} violates RFC 03 §2"),
                ));
            }
            if *part == "alive" {
                return Err(lint(
                    file,
                    format!("{path:?}: `alive` is a reserved liveliness leaf (RFC 03 §3)"),
                ));
            }
            chunks.push(Chunk::Literal(part.to_string()));
        }
    }
    if chunks.is_empty() {
        return Err(lint(file, "empty subject path"));
    }
    Ok(chunks)
}

pub(crate) fn camel(parts: &[&str]) -> String {
    let mut out = String::new();
    for part in parts {
        for seg in part.split(|c: char| !c.is_ascii_alphanumeric()) {
            let mut chars = seg.chars();
            if let Some(first) = chars.next() {
                out.push(first.to_ascii_uppercase());
                out.push_str(chars.as_str());
            }
        }
    }
    out
}

/// A hand-written `variant = "..."` override must be a plain CamelCase Rust
/// identifier — it lands verbatim in the generated enum.
fn is_valid_variant(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_uppercase()) && chars.all(|c| c.is_ascii_alphanumeric())
}

/// Variant name: CamelCase of the literal chunks; all-variable patterns use
/// the variable names instead.
fn variant_name(chunks: &[Chunk]) -> String {
    let literals: Vec<&str> = chunks
        .iter()
        .filter_map(|c| match c {
            Chunk::Literal(l) => Some(l.as_str()),
            _ => None,
        })
        .collect();
    if !literals.is_empty() {
        return camel(&literals);
    }
    let vars: Vec<&str> = chunks
        .iter()
        .map(|c| match c {
            Chunk::Literal(l) => l.as_str(),
            Chunk::Var(v) | Chunk::Rest(v) => v.as_str(),
        })
        .collect();
    camel(&vars)
}

pub(crate) fn snake(name: &str) -> String {
    name.replace(['-', '.'], "_")
}

pub(crate) fn producer_module(name: &str) -> String {
    snake(name)
}

/// Parse-precedence: at the first differing position, literal beats var
/// beats rest; shorter fixed arity ties break by pattern text (stable).
pub(crate) fn precedence_rank(c: &Chunk) -> u8 {
    match c {
        Chunk::Literal(_) => 0,
        Chunk::Var(_) => 1,
        Chunk::Rest(_) => 2,
    }
}

fn load_registry(dir: &Path) -> Result<Vec<RegistryFile>, Error> {
    let mut files = Vec::new();
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| Error::Io(dir.to_path_buf(), e))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    paths.sort();
    for path in paths {
        let fname = path.file_name().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(&path).map_err(|e| Error::Io(path.to_path_buf(), e))?;
        let doc: toml::Value =
            toml::from_str(&text).map_err(|e| lint(&fname, format!("TOML parse error: {e}")))?;

        // [registry] header (RFC 08 §2).
        let header = doc
            .get("registry")
            .ok_or_else(|| lint(&fname, "missing [registry] header"))?;
        for field in ["version", "app"] {
            if header.get(field).and_then(|v| v.as_str()).is_none() {
                return Err(lint(
                    &fname,
                    format!("[registry] missing string field {field:?}"),
                ));
            }
        }
        if header.get("convention").and_then(|v| v.as_integer()) != Some(1) {
            return Err(lint(
                &fname,
                "[registry] convention must be 1 for this crate",
            ));
        }

        let (name, service_origin) = if let Some(svc) = doc.get("service") {
            let name = svc
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| lint(&fname, "[service] missing name"))?;
            let origin = svc
                .get("origin")
                .and_then(|v| v.as_str())
                .ok_or_else(|| lint(&fname, "[service] missing origin"))?;
            if !is_valid_verbatim_chunk(origin) {
                return Err(lint(
                    &fname,
                    format!("[service] origin {origin:?} is not a verbatim chunk"),
                ));
            }
            (name.to_string(), Some(origin.to_string()))
        } else {
            let prod = doc
                .get("producer")
                .ok_or_else(|| lint(&fname, "missing [producer] or [service]"))?;
            let name = prod
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| lint(&fname, "[producer] missing name"))?;
            if !is_valid_plain_chunk(name) {
                return Err(lint(
                    &fname,
                    format!("producer name {name:?} violates RFC 03 §2"),
                ));
            }
            if name.rsplit_once('-').is_some_and(|(b, t)| {
                !b.is_empty() && t.bytes().all(|c| c.is_ascii_digit()) && !t.is_empty()
            }) {
                return Err(lint(
                    &fname,
                    format!("producer name {name:?} ends in -<int> (RFC 03 §1.5)"),
                ));
            }
            if ["artifact", "tree", "store"].contains(&name) {
                return Err(lint(
                    &fname,
                    format!("producer name {name:?} is a reserved blob tier token"),
                ));
            }
            (name.to_string(), None)
        };

        let empty = Vec::new();
        let subject_entries = doc
            .get("subject")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty);
        let mut subjects = Vec::new();
        for entry in subject_entries {
            let spath = entry
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| lint(&fname, "[[subject]] missing path"))?;
            let chunks = parse_pattern(&fname, spath)?;
            let class = entry
                .get("class")
                .and_then(|v| v.as_str())
                .ok_or_else(|| lint(&fname, format!("{spath:?}: missing class")))?;
            if !["telemetry", "state", "events"].contains(&class) {
                return Err(lint(&fname, format!("{spath:?}: unknown class {class:?}")));
            }
            let default_qos = match class {
                "telemetry" => "sampled",
                "state" => "refreshed",
                _ => "transition",
            };
            let qos = entry
                .get("qos")
                .and_then(|v| v.as_str())
                .unwrap_or(default_qos)
                .to_string();
            if !["sampled", "refreshed", "transition", "alert", "frame"].contains(&qos.as_str()) {
                return Err(lint(
                    &fname,
                    format!("{spath:?}: unknown qos profile {qos:?} (RFC 04 §3)"),
                ));
            }
            // RFC 08 §2: `type` is required — it is what binds one payload type
            // to every expansion of the pattern (P5), and what lets a consumer
            // decode a wildcard result set without sniffing.
            let payload_type = entry
                .get("type")
                .and_then(|v| v.as_str())
                .ok_or_else(|| lint(&fname, format!("{spath:?}: missing type (RFC 08 §2)")))?
                .to_string();
            let unit = entry
                .get("unit")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let cardinality = entry.get("cardinality").and_then(|v| v.as_integer());
            let has_var = chunks.iter().any(|c| !matches!(c, Chunk::Literal(_)));
            if has_var && cardinality.is_none() {
                return Err(lint(
                    &fname,
                    format!("{spath:?}: {{var}} pattern needs integer cardinality (RFC 08 §5)"),
                ));
            }
            let ttl_s = entry.get("ttl_s").and_then(|v| v.as_integer());
            if class == "state" && ttl_s.is_none() {
                return Err(lint(
                    &fname,
                    format!("{spath:?}: state subject needs ttl_s (RFC 08 §5)"),
                ));
            }
            let rate = entry
                .get("rate")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            if class == "events" {
                match rate.as_deref() {
                    Some("rare") | Some("low") => {}
                    Some(r) if r.starts_with("burst(") && r.ends_with("/h)") => {}
                    _ => {
                        return Err(lint(
                            &fname,
                            format!(
                                "{spath:?}: events subject needs rate rare|low|burst(n/h) (RFC 08 §5)"
                            ),
                        ));
                    }
                }
            }
            if entry.get("description").and_then(|v| v.as_str()).is_none()
                || entry.get("since").and_then(|v| v.as_str()).is_none()
            {
                return Err(lint(
                    &fname,
                    format!("{spath:?}: missing description/since"),
                ));
            }
            // `common = "..."` (RFC 04/06): declares this entry as one of the
            // framework state subjects; drives AnySubject::common_state().
            let common = entry
                .get("common")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            if let Some(c) = &common {
                let Some((_, _, want_vars)) = COMMON_STATE.iter().find(|(n, _, _)| n == c) else {
                    let known: Vec<&str> = COMMON_STATE.iter().map(|(n, _, _)| *n).collect();
                    return Err(lint(
                        &fname,
                        format!(
                            "{spath:?}: unknown common state {c:?} (known: {})",
                            known.join(", ")
                        ),
                    ));
                };
                if class != "state" {
                    return Err(lint(
                        &fname,
                        format!("{spath:?}: common = {c:?} is only valid on class = \"state\""),
                    ));
                }
                let have: Vec<String> = chunks
                    .iter()
                    .filter_map(|ch| match ch {
                        Chunk::Var(v) | Chunk::Rest(v) => Some(snake(v)),
                        Chunk::Literal(_) => None,
                    })
                    .collect();
                let want: Vec<String> = want_vars.iter().map(|v| v.to_string()).collect();
                if have != want {
                    return Err(lint(
                        &fname,
                        format!(
                            "{spath:?}: common = {c:?} needs pattern variables {want:?}, found {have:?}"
                        ),
                    ));
                }
            }
            // `variant` overrides the derived name. Two patterns with the same
            // literal chunks but different arity (`cpu/usage` vs
            // `cpu/{core}/usage`) derive the same name and would otherwise trip
            // the collision lint below with no way out.
            let variant = match entry.get("variant").and_then(|v| v.as_str()) {
                Some(v) if is_valid_variant(v) => v.to_string(),
                Some(v) => {
                    return Err(lint(
                        &fname,
                        format!("{spath:?}: variant {v:?} is not a CamelCase identifier"),
                    ));
                }
                None => variant_name(&chunks),
            };
            subjects.push(SubjectEntry {
                path: spath.to_string(),
                variant,
                chunks,
                class: class.to_string(),
                payload_type,
                unit,
                cardinality,
                qos,
                ttl_s,
                rate,
                common,
            });
        }

        // Variant-name and exact-path collisions.
        let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
        for s in &subjects {
            if let Some(other) = seen.insert(s.variant.as_str(), s.path.as_str()) {
                return Err(lint(
                    &fname,
                    format!(
                        "subjects {other:?} and {:?} collide on variant {:?}",
                        s.path, s.variant
                    ),
                ));
            }
        }
        let mut paths_seen = BTreeSet::new();
        for s in &subjects {
            if !paths_seen.insert(&s.path) {
                return Err(lint(&fname, format!("duplicate subject path {:?}", s.path)));
            }
        }

        let procedure_entries = doc
            .get("procedure")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty);
        let mut procedures = Vec::new();
        for entry in procedure_entries {
            let ppath = entry
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| lint(&fname, "[[procedure]] missing path"))?;
            let chunks: Vec<String> = ppath.split('/').map(str::to_string).collect();
            for c in &chunks {
                if !is_valid_plain_chunk(c) {
                    return Err(lint(
                        &fname,
                        format!(
                            "procedure {ppath:?}: chunk {c:?} violates RFC 03 §2 (procedure paths are literal; parameters ride the selector)"
                        ),
                    ));
                }
            }
            let kind = entry
                .get("kind")
                .and_then(|v| v.as_str())
                .ok_or_else(|| lint(&fname, format!("procedure {ppath:?}: missing kind")))?;
            if !["read", "write", "long-running"].contains(&kind) {
                return Err(lint(
                    &fname,
                    format!("procedure {ppath:?}: unknown kind {kind:?}"),
                ));
            }
            let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
            procedures.push(ProcedureEntry {
                path: ppath.to_string(),
                variant: camel(&refs),
                chunks,
                kind: kind.to_string(),
                request: entry
                    .get("request")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                reply: entry
                    .get("reply")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            });
        }

        // [[media]] entries (RFC 08 §2): patterns validated; a `{var}`-bearing
        // media path MUST declare a `cardinality` (the highest-bandwidth plane
        // must bound its fan-out — the `{tier}` chunk multiplies it), and every
        // entry MUST name an `attachment` type. No builder codegen yet; media
        // key builders are hand-written in `zenkey::V1Context`.
        if let Some(media) = doc.get("media").and_then(|v| v.as_array()) {
            for entry in media {
                let mpath = entry
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| lint(&fname, "[[media]] missing path"))?;
                let chunks = parse_pattern(&fname, mpath)?;
                let has_var = chunks.iter().any(|c| !matches!(c, Chunk::Literal(_)));
                let cardinality = entry.get("cardinality").and_then(|v| v.as_integer());
                if has_var && cardinality.is_none() {
                    return Err(lint(
                        &fname,
                        format!(
                            "[[media]] {mpath:?}: {{var}} pattern needs integer cardinality \
                             (RFC 08 §2)"
                        ),
                    ));
                }
                if entry.get("attachment").and_then(|v| v.as_str()).is_none() {
                    return Err(lint(
                        &fname,
                        format!("[[media]] {mpath:?}: missing attachment type (RFC 08 §2)"),
                    ));
                }
            }
        }

        let mut deprecated = Vec::new();
        if let Some(arr) = doc.get("deprecated").and_then(|v| v.as_array()) {
            for entry in arr {
                deprecated.push(
                    entry
                        .get("path")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| lint(&fname, "[[deprecated]] missing path"))?
                        .to_string(),
                );
            }
        }
        // Deprecated paths may never be re-registered as live subjects.
        for d in &deprecated {
            if subjects.iter().any(|s| &s.path == d) {
                return Err(lint(
                    &fname,
                    format!("deprecated path {d:?} re-registered as a live subject (RFC 08 §3)"),
                ));
            }
        }

        files.push(RegistryFile {
            name,
            service_origin,
            toml_path: path
                .canonicalize()
                .map_err(|e| Error::Io(path.clone(), e))?
                .to_string_lossy()
                .to_string(),
            subjects,
            procedures,
            deprecated,
        });
    }
    Ok(files)
}

/// The append-only deprecation ledger (RFC 08 §3/§5): every line is
/// `<producer>\t<path>`. A ledger line without its TOML entry = someone
/// deleted a deprecation; a TOML deprecation missing from the ledger = the
/// ledger append was forgotten. Both fail the build.
fn check_deprecation_ledger(ledger_path: &Path, files: &[RegistryFile]) -> Result<(), Error> {
    let ledger = std::fs::read_to_string(ledger_path).unwrap_or_default();
    let mut ledger_entries: Vec<(&str, &str)> = Vec::new();
    for l in ledger
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
    {
        ledger_entries.push(
            l.split_once('\t')
                .ok_or_else(|| lint("deprecated.lock", format!("bad ledger line {l:?}")))?,
        );
    }
    for (producer, path) in &ledger_entries {
        let present = files
            .iter()
            .any(|f| f.name == *producer && f.deprecated.iter().any(|d| d == path));
        if !present {
            return Err(lint(
                "deprecated.lock",
                format!(
                    "ledger entry {producer}\t{path} has no [[deprecated]] entry — deprecations are append-only, restore it (RFC 08 §3)"
                ),
            ));
        }
    }
    for f in files {
        for d in &f.deprecated {
            let listed = ledger_entries
                .iter()
                .any(|(p, path)| *p == f.name && path == d);
            if !listed {
                return Err(lint(
                    "deprecated.lock",
                    format!(
                        "[[deprecated]] {d:?} in {} is not in the ledger — append `{}\t{d}` to the ledger file",
                        f.name, f.name
                    ),
                ));
            }
        }
    }
    Ok(())
}
