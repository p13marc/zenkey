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
//! `REGISTRIES`, `registry_toml()`, and `is_registered_telemetry()`. When the
//! registry declares `[[blob]]` entries (RFC 08 §2, v1.8), an app-level
//! `blob` module is emitted as well — a deduped `Tier` enum over every
//! declared tier (blob keys carry no producer chunk, so the surface is
//! app-level and `Tier::declared_by()` names every declaring producer) with
//! typed per-tier key builders and the `*`-origin probe form.
//!
//! Lints (RFC 08 §5) are errors returned from [`Config::generate`] — a
//! violating registry file fails the *consumer's* build, where the TOML was
//! authored. Codegen is normative in both directions (RFC 08 §1): an
//! unregistered subject does not construct, and a metric name refines into a
//! typed subject with named variables instead of positional `split('/')`.
//!
//! Two ledgers ride beside the TOMLs, both checked here: the append-only
//! `deprecated.lock` (RFC 08 §3 — retirements never un-happen) and the
//! deliberately non-append-only `conditional.lock` (RFC 08 §6.1, v1.25 —
//! one `<producer>\t<path>\t<condition>` line per subject whose emission is
//! gated; a line naming no registry subject fails the build, and a
//! consumer's emitted-surface check reads the validated set through
//! [`Config::conditional_subjects`] to exempt exactly those).

// docs.rs builds on nightly with `--cfg docsrs` (see Cargo.toml), which is
// what lets each feature-gated item carry the feature that gates it. Inert
// everywhere else — a stable `cargo doc` never sets the cfg (#325).
#![cfg_attr(docsrs, feature(doc_cfg))]

mod emit;
#[cfg(feature = "export")]
#[cfg_attr(docsrs, doc(cfg(feature = "export")))]
pub mod export;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use zenkey::grammar::{is_valid_plain_chunk, is_valid_verbatim_chunk};
// One implementation of the registry pattern grammar (#320): the codegen
// names `zenkey`'s types rather than keeping a second copy of the rules.
use zenkey::pattern::{PatternChunk as Chunk, PatternError, SubjectPattern};
use zenkey::{Fanout, SliceToken, SubjectKind};

/// A codegen failure. Lint variants carry the registry file they were found
/// in — surface them with `unwrap()` in the build script so the message
/// reaches the build output.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("registry lint failed [{file}]: {message}")]
    Lint {
        file: String,
        message: String,
        /// What kind of lint failure this is — the thing
        /// [`Config::write_compat_lock`] decides forceability on.
        kind: LintKind,
    },
    /// `#[source]` on the inner error, so a caller can reach the
    /// `io::ErrorKind` and tell "no registry dir" from "unreadable registry
    /// dir" — the two the flattened form rendered identically (#317).
    #[error("registry dir {0:?}")]
    Io(PathBuf, #[source] std::io::Error),
    #[error(
        "OUT_DIR is not set and no out_file was given — call from a build script or set .out_file(..)"
    )]
    NoOutDir,
}

/// What kind of registry-lint failure an [`Error::Lint`] reports.
///
/// Exists because forceability used to be decided by string-matching the
/// error's own prose: `msg.contains("incompatible registry edit") ||
/// msg.contains("vanished")`. Rewording either message — both of them
/// multi-line paragraphs that read like something an editor would tidy —
/// silently changed what `--force` would and would not overwrite, with no
/// test that would notice. (The word "vanished" is already used as a *subject
/// path* in this crate's own conditional-ledger tests, which is how close
/// that coupling sits to an accidental match.) #318.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LintKind {
    /// A pinned entry changed shape — the edit RFC 08 §3.1 forbids under
    /// `compat = "backward"`. Overwritable with an explicit force.
    Incompatible,
    /// A pinned entry disappeared without a retirement. Same: overwritable
    /// with an explicit force.
    Vanished,
    /// The lock is behind the registry — entries added, retirements not yet
    /// recorded. Additive evolution is free; the snapshot just has to follow.
    ///
    /// Not a failure to force past: it is the case
    /// [`Config::write_compat_lock`] **exists to fix**, so that call proceeds
    /// through it without a force at all.
    Stale,
    /// A line of the lock file itself does not parse. Never forceable, and
    /// never proceeded past: overwriting a lock you could not read is how a
    /// corrupt file becomes a silent reset.
    BadLine,
    /// Everything else — a malformed registry entry, a ledger disagreement,
    /// an illegal field. **Never** forceable: forcing past one of these would
    /// write a lock for a registry that does not lint.
    Invalid,
    /// A file declares `draft = true` — an observation-derived, unreviewed
    /// registry (RFC 08 §6.1, v1.34) — and the build was not told to admit
    /// one. **Never** forceable and never resolved by writing a lock: the
    /// marker exists so a draft cannot become a fleet's `introspect` truth
    /// by being copied into place, and the only way past it is a review
    /// that removes it (or [`Config::allow_drafts`], for prototyping).
    Draft,
}

impl Error {
    /// Whether this failure means the question could not be *put* — the
    /// caller's input was refused before any check ran.
    ///
    /// A **lint failure is not** one of these: finding a lint violation is
    /// this crate answering the question it was asked, and `zenctl registry
    /// lint` exits 1 for it by design. A registry directory that does not
    /// exist is the opposite — nothing was linted, and reporting a finding
    /// would be a verdict on a question nobody could ask (#348).
    pub fn is_unaskable(&self) -> bool {
        match self {
            Error::Io(..) | Error::NoOutDir => true,
            Error::Lint { .. } => false,
        }
    }
}

impl LintKind {
    /// Whether an explicit force may overwrite past this failure.
    pub fn is_forceable(self) -> bool {
        matches!(self, LintKind::Incompatible | LintKind::Vanished)
    }

    /// Whether [`Config::write_compat_lock`] proceeds through this failure
    /// without being asked — true only for [`Stale`](LintKind::Stale), the
    /// drift it is called to resolve.
    pub fn is_resolved_by_writing(self) -> bool {
        matches!(self, LintKind::Stale)
    }
}

/// What `write_compat_lock` should do when the existing lock and the registry
/// disagree incompatibly.
///
/// A `bool` named `force` at a call site says nothing about which way round
/// it goes; these two names do (#318).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnIncompatible {
    /// Refuse the write and return the failure. The default.
    Refuse,
    /// Overwrite, and report every pin that was broken so the caller can
    /// print them — the escape hatch is legal, silent it is not.
    ForceAndReport,
}

/// A registry diagnostic that is **not** a failure.
///
/// Returned as a value rather than printed, so the same check reaches a
/// build script (as `cargo::warning=`) and a CLI (`zenctl registry lint`)
/// without either one deciding what the other sees (#319).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryWarning {
    /// The registry file the warning is about.
    pub file: String,
    pub message: String,
}

impl std::fmt::Display for RegistryWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.file, self.message)
    }
}

fn lint(file: &str, message: impl Into<String>) -> Error {
    lint_kind(file, message, LintKind::Invalid)
}

fn lint_kind(file: &str, message: impl Into<String>, kind: LintKind) -> Error {
    Error::Lint {
        file: file.to_string(),
        message: message.into(),
        kind,
    }
}

/// Read an optional **count** field (`ttl_s`, `cardinality`) — a duration in
/// seconds or a key population, neither of which has a negative value.
///
/// The sign check lives here rather than in the emitter because of what the
/// crate doc promises: a violating registry file fails the *consumer's* build,
/// where the TOML was authored. `as_integer()` alone yields an `i64`, which
/// the emitter interpolated verbatim into an `Option<u64>` accessor — so
/// `ttl_s = -60` linted clean and then failed as `Some(-60)` in `$OUT_DIR`, in
/// code the consumer never wrote and cannot fix (issue #313). The message
/// names the file, the entry's path and the field, so the reader is pointed at
/// the line responsible.
fn opt_count(
    file: &str,
    entry: &toml::Value,
    path: &str,
    field: &str,
) -> Result<Option<u64>, Error> {
    match entry.get(field).and_then(|v| v.as_integer()) {
        None => Ok(None),
        Some(n) => u64::try_from(n).map(Some).map_err(|_| {
            lint(
                file,
                format!("{path:?}: {field} must not be negative, got {n} (RFC 08 §5)"),
            )
        }),
    }
}

pub(crate) struct SubjectEntry {
    pub path: String,
    pub chunks: Vec<Chunk>,
    pub class: String,
    pub payload_type: String,
    pub unit: Option<String>,
    /// `kind = "counter|gauge|text|bool"` — what the leaf value *is*
    /// (RFC 08 §2, v1.32); the canonical token, already linted.
    pub kind: Option<String>,
    pub cardinality: Option<u64>,
    pub qos: String,
    pub ttl_s: Option<u64>,
    pub rate: Option<String>,
    pub variant: String,
    /// `common = "..."` — the RFC-defined framework state subject this entry
    /// declares itself as (drives `AnySubject::common_state()`).
    pub common: Option<String>,
    /// Optional declared payload encoding (RFC 08 §2, v1.5).
    pub encoding: Option<String>,
}

pub(crate) struct ProcedureEntry {
    pub path: String,
    /// Literal and `{var}` chunks (rest-vars are illegal in procedure paths).
    pub chunks: Vec<Chunk>,
    pub kind: String,
    pub request: Option<String>,
    pub reply: Option<String>,
    pub variant: String,
    /// RFC 08 §2 (v1.4 G2): default `Forbidden` for `kind = "write"`.
    pub fanout: Fanout,
    /// Whether a retried call is safe (RFC 05 §3).
    pub idempotent: bool,
    /// Key-population bound — RFC 08 §2 requires it on any `{var}`-bearing
    /// procedure path, the same budget rule as `[[subject]]` and `[[media]]`.
    #[allow(dead_code)] // linted here; served verbatim through introspect
    pub cardinality: Option<u64>,
    /// Optional declared payload encoding (RFC 08 §2, v1.5).
    pub encoding: Option<String>,
}

/// One `[[media]]` entry (RFC 08 §2; modeled since v1.5 — H2 delivers the
/// v1.3 builder-codegen promise).
#[allow(dead_code)] // consumed by the media-codegen commit (#10)
pub(crate) struct MediaEntry {
    pub path: String,
    pub chunks: Vec<Chunk>,
    pub encoding: String,
    pub attachment: String,
    pub cardinality: Option<u64>,
    pub variant: String,
}

/// One `[[blob]]` entry (RFC 08 §2, v1.8).
///
/// Unlike every other entry kind, a blob entry has **no `path`**: the three
/// key shapes are fixed by RFC 07 §2 and their variable chunks are content
/// addresses, not registry vocabulary. What a deployment actually varies is
/// which tiers and endpoints an origin serves, so that is the whole entry.
pub(crate) struct BlobEntry {
    /// `artifact` | `tree` | `store` — the reserved tier token (RFC 07 §2).
    pub tier: String,
    /// The RFC 07 §2.2 endpoints served; `artifact` only, empty elsewhere.
    pub endpoints: Vec<String>,
    /// The `<algo>` chunk; `store` only (RFC 07 §2.4).
    pub algo: Option<String>,
    /// Type conveying this blob's reference — the payload that must carry the
    /// content root (RFC 07 §2.1). Resolved against the shared type table.
    pub reference: Option<String>,
    /// Encoding of the blob *content*, never of the chunk framing (which is
    /// self-describing on the wire, RFC 07 §2.4).
    pub encoding: Option<String>,
    pub description: Option<String>,
}

/// The endpoints RFC 07 §2.2 reserves under `artifact/<id>/`. A closed set:
/// the plane defines them, so an unknown name is a build error rather than an
/// extension point.
pub(crate) const BLOB_ENDPOINTS: &[&str] = &["manifest", "slice", "have", "push", "fanout"];

/// The tier tokens RFC 07 §2 reserves at position 5 under `@blob`.
pub(crate) const BLOB_TIERS: &[&str] = &["artifact", "tree", "store"];

/// The `[budget]` table (RFC 08 §2, v1.32): what the producer may cost the
/// machine it runs on, in the units its health document's `self_stats`
/// reports (RFC 04 §1.2).
///
/// Linted here — non-negative bounds, a required and unique `name` per
/// `[[budget.tables]]` row — and then **not generated**: the block rides the
/// slice verbatim in `REGISTRY_TOML` and reaches `introspect` unchanged, which
/// is where an observer reads it (RFC 13 §3). It is the first per-producer
/// table that is not a subject, procedure, tier or stream, and the first that
/// yields no key and no builder — so no accessor is emitted for it, and the
/// lock does not pin it (a budget is a claim about cost, not about shape).
#[allow(dead_code)] // linted, carried verbatim in REGISTRY_TOML; nothing is generated from it
pub(crate) struct BudgetEntry {
    pub rss_mb: Option<u64>,
    pub tables: Vec<TableBudgetEntry>,
}

/// One `[[budget.tables]]` row (RFC 08 §2, v1.32).
#[allow(dead_code)] // see `BudgetEntry`
pub(crate) struct TableBudgetEntry {
    pub name: String,
    pub max_entries: Option<u64>,
    pub max_bytes: Option<u64>,
}

pub(crate) struct RegistryFile {
    /// Producer base name, or service name for `[service]` files.
    pub name: String,
    /// `Some("@catalog")`-style origin for services, `None` for producers.
    pub service_origin: Option<String>,
    pub toml_path: String,
    pub subjects: Vec<SubjectEntry>,
    pub procedures: Vec<ProcedureEntry>,
    pub media: Vec<MediaEntry>,
    pub blob: Vec<BlobEntry>,
    pub deprecated: Vec<Deprecated>,
    /// The file's compatibility level (RFC 08 §3.1): `backward` (default)
    /// pins its entries in `registry.lock`; `none` opts out, loudly.
    pub compat: Compat,
    /// The producer's declared cost (RFC 08 §2, v1.32), when it declares one.
    #[allow(dead_code)] // see `BudgetEntry`: linted, never generated from
    pub budget: Option<BudgetEntry>,
    /// `draft = true` in the header (RFC 08 §6.1, v1.34): observation-derived
    /// and unreviewed. Refused by [`Config::checked`] unless
    /// [`Config::allow_drafts`] admits it; its subjects may omit `since`.
    pub draft: bool,
}

/// One `[[deprecated]]` entry: what was retired, and which kind of thing it
/// was (RFC 08 §3, v1.26).
///
/// The `kind` is why this is a struct and not the bare path it used to be.
/// Retirement was checked for subjects only, so a procedure removed from a
/// `compat = "backward"` file failed `registry.lock` with *"pinned entry
/// vanished without retirement"* and had no sanctioned exit — the ledger
/// entry that was supposed to be the exit was never consulted for it
/// (#377).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Deprecated {
    pub kind: EntryKind,
    pub path: String,
}

/// Which declared surface a pin or a retirement is about (RFC 08 §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryKind {
    Subject,
    Procedure,
}

impl EntryKind {
    /// The token `registry.lock` and `deprecated.lock` spell it with.
    pub fn token(self) -> &'static str {
        match self {
            EntryKind::Subject => "subject",
            EntryKind::Procedure => "procedure",
        }
    }

    fn parse(s: &str) -> Option<EntryKind> {
        match s {
            "subject" => Some(EntryKind::Subject),
            "procedure" => Some(EntryKind::Procedure),
            _ => None,
        }
    }
}

/// A registry file's declared compatibility level (RFC 08 §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Compat {
    /// Existing subject paths keep their class and type, existing
    /// procedures their kind and request/reply shapes; additive evolution
    /// is free; retirement goes through `[[deprecated]]`. The default.
    Backward,
    /// Unchecked — the loud escape hatch for a registry still finding its
    /// shape. The build prints a warning per file.
    None,
}

/// The framework state subjects a `common = "..."` field may name — the
/// RFC 04 §1.4 table (v1.25, extended in v1.29/v1.30; the `@catalog` set
/// per RFC 06 §5/§5.5/§5.6, `errors` per RFC 11 §2's profile-extension
/// rule) — with the `zenkey::CommonState`
/// constructor and the **canonical subject pattern** the entry's `path`
/// MUST spell exactly (04 §1.4: "the spelling is the table's" — the token
/// is a claim that this entry *is* that framework subject).
///
/// The cross-producer subset of this vocabulary also lives in
/// `zenkey::CommonFamily` (#168), which drives `selector::common_family`.
/// The tables are kept separate — the constructor expressions are
/// codegen-specific, and the `@catalog` rows are one service's subjects,
/// not families across producers — but must agree where they overlap; a
/// test below pins that in both directions.
pub(crate) const COMMON_STATE: &[(&str, &str, &str)] = &[
    ("health", "Health", "health"),
    ("errors", "Errors", "errors"),
    ("sensor", "Sensor", "sensor"),
    ("alert", "Alert { alert_key }", "alert/{alert_key}"),
    ("evidence_self", "EvidenceSelf", "evidence/self"),
    (
        "evidence_device",
        "EvidenceDevice { device }",
        "evidence/device/{device}",
    ),
    (
        "evidence_names",
        "EvidenceNames { ip_slug }",
        "evidence/names/{ip_slug}",
    ),
    (
        "evidence_relation",
        "EvidenceRelation { relation_id }",
        "evidence/relation/{relation_id}",
    ),
    (
        "entity",
        "CatalogEntity { entity_id }",
        "entity/{entity_id}",
    ),
    ("alias", "CatalogAlias { old_id }", "alias/{old_id}"),
    ("pdns", "CatalogPdns { ip_slug }", "pdns/{ip_slug}"),
    (
        "incident",
        "CatalogIncident { incident_id }",
        "incident/{incident_id}",
    ),
    ("ack", "CatalogAck { alert_ref }", "ack/{alert_ref}"),
    ("silence", "CatalogSilence { id }", "silence/{id}"),
    ("edge", "CatalogEdge { edge_id }", "edge/{edge_id}"),
];

/// The rows of [`COMMON_STATE`] that are one *service's* subjects — the
/// `@catalog` set (RFC 04 §1.4; RFC 06 §5, §5.5 since v1.29, §5.6 since
/// v1.30) — claimable only by a `[service]` registry file, never by an
/// ordinary producer.
pub(crate) const COMMON_SERVICE_TOKENS: &[&str] = &[
    "entity", "alias", "pdns", "incident", "ack", "silence", "edge",
];

/// Builder for one codegen run. See the crate docs for the two-line consumer
/// integration.
#[derive(Debug)]
pub struct Config {
    registry_dir: PathBuf,
    out_file: Option<PathBuf>,
    zenkey_path: String,
    ledger: Option<PathBuf>,
    /// The RFC 08 §3.1 compatibility lock
    /// (default `<registry_dir>/registry.lock`).
    compat_lock: Option<PathBuf>,
    /// The RFC 08 §6.1 conditional-subject ledger
    /// (default `<registry_dir>/conditional.lock`).
    conditional: Option<PathBuf>,
    emit_rerun_if_changed: bool,
    /// Admit `draft = true` files (RFC 08 §6.1, v1.34) instead of refusing
    /// them — prototyping against an inferred registry. Off by default.
    allow_drafts: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

impl Config {
    #[must_use]
    pub fn new() -> Self {
        Config {
            registry_dir: PathBuf::from("registry"),
            out_file: None,
            zenkey_path: "::zenkey".to_string(),
            ledger: None,
            compat_lock: None,
            conditional: None,
            emit_rerun_if_changed: true,
            allow_drafts: false,
        }
    }

    /// The directory holding `*.toml` registry files (default `registry`,
    /// relative to the consuming crate's manifest).
    #[must_use]
    pub fn registry_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.registry_dir = dir.as_ref().to_path_buf();
        self
    }

    /// Where the generated module is written
    /// (default `$OUT_DIR/zenkey_registry.rs`).
    #[must_use]
    pub fn out_file(mut self, f: impl AsRef<Path>) -> Self {
        self.out_file = Some(f.as_ref().to_path_buf());
        self
    }

    /// The path the generated code uses to reach the `zenkey` crate
    /// (default `::zenkey`) — override for renamed-dependency setups.
    #[must_use]
    pub fn zenkey_path(mut self, p: &str) -> Self {
        self.zenkey_path = p.to_string();
        self
    }

    /// The append-only deprecation ledger
    /// (default `<registry_dir>/deprecated.lock`; a missing file is an empty
    /// ledger).
    #[must_use]
    pub fn ledger(mut self, f: impl AsRef<Path>) -> Self {
        self.ledger = Some(f.as_ref().to_path_buf());
        self
    }

    /// The compatibility lock (RFC 08 §3.1; default
    /// `<registry_dir>/registry.lock` — a missing file is an empty snapshot
    /// and fails as stale until regenerated).
    #[must_use]
    pub fn compat_lock(mut self, f: impl AsRef<Path>) -> Self {
        self.compat_lock = Some(f.as_ref().to_path_buf());
        self
    }

    /// The conditional-subject ledger (RFC 08 §6.1, v1.25; default
    /// `<registry_dir>/conditional.lock` — a missing file is an empty
    /// ledger: no subject is conditional).
    ///
    /// Unlike its sibling [`ledger`](Self::ledger), this file is **not**
    /// append-only: a line leaves when its gating condition does, and the
    /// subject re-enters the emitted-surface check by deletion.
    #[must_use]
    pub fn conditional_ledger(mut self, f: impl AsRef<Path>) -> Self {
        self.conditional = Some(f.as_ref().to_path_buf());
        self
    }

    /// Suppress the `cargo::rerun-if-changed` lines (default on) — for
    /// calling outside a build script.
    #[must_use]
    pub fn no_rerun_if_changed(mut self) -> Self {
        self.emit_rerun_if_changed = false;
        self
    }

    /// Admit registry files that declare `draft = true` (RFC 08 §6.1,
    /// v1.34) — the observation-derived, unreviewed drafts
    /// `zenctl registry infer` writes.
    ///
    /// Off by default, and the default is the point: a draft describes
    /// surfaces that *were* served by an author who cannot vouch that they
    /// *will* be, so a build MUST refuse it unless explicitly told to admit
    /// one ([`LintKind::Draft`], never forceable). With `allow_drafts(true)`
    /// a draft is admitted for prototyping — its subjects may omit `since`
    /// (a draft has no version stream) and every draft file comes back as a
    /// [`RegistryWarning`], so a build that admits one says so every time.
    /// Promotion is a review: drop the marker, assign `since`.
    #[must_use]
    pub fn allow_drafts(mut self, allow: bool) -> Self {
        self.allow_drafts = allow;
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
        let (generated, warnings) = self.generate_string_checked()?;
        // Emitting is the *build's* job. `checked` returns them as values so
        // `lint()` reports the same set without printing cargo directives
        // into a CLI's stdout (#319).
        for w in warnings {
            println!("cargo::warning={w}");
        }
        std::fs::write(&out, generated).map_err(|e| Error::Io(out, e))?;
        Ok(())
    }

    /// As [`generate`](Self::generate), returning the generated source
    /// instead of writing it.
    pub fn generate_string(&self) -> Result<String, Error> {
        self.generate_string_checked().map(|(source, _)| source)
    }

    /// As [`generate_string`](Self::generate_string), also returning the
    /// non-fatal diagnostics the build would emit as `cargo::warning=`.
    pub fn generate_string_checked(&self) -> Result<(String, Vec<RegistryWarning>), Error> {
        let (files, warnings) = self.checked()?;
        Ok((emit::emit(&files, &self.zenkey_path), warnings))
    }

    /// Every check `generate` runs, and **nothing else** (issue #50):
    /// `zenctl registry lint <dir>` gives an application the same diagnostic
    /// its `build.rs` would produce, without needing the application's build.
    ///
    /// Deliberately one `Err`, not a collected list: the point is fidelity to
    /// what a build says, and a build stops at the first lint. A lint that
    /// reported *more* than the build would be a different tool wearing the
    /// same name.
    ///
    /// It reported *less* until #319: the `compat = "none"` warning sat
    /// inside `if emit_rerun_if_changed`, and `zenctl registry lint` calls
    /// `.no_rerun_if_changed()` — correctly, for the cargo directives — so a
    /// registry that had opted out of RFC 08 §3.1 checking was never flagged
    /// by the tool whose whole job is to say what the build says. The flag
    /// governs cargo directives only now, and the warnings come back as
    /// values.
    pub fn lint(&self) -> Result<Vec<RegistryWarning>, Error> {
        self.checked().map(|(_, warnings)| warnings)
    }

    fn checked(&self) -> Result<(Vec<RegistryFile>, Vec<RegistryWarning>), Error> {
        if self.emit_rerun_if_changed {
            println!("cargo::rerun-if-changed={}", self.registry_dir.display());
            if let Some(l) = &self.ledger {
                println!("cargo::rerun-if-changed={}", l.display());
            }
            if let Some(c) = &self.conditional {
                println!("cargo::rerun-if-changed={}", c.display());
            }
        }
        let files = load_registry(&self.registry_dir)?;
        // A draft (RFC 08 §6.1, v1.34) is refused before any other check
        // runs: the marker is what keeps an observation-derived file from
        // becoming `introspect` truth by being copied into place, and it is
        // a refusal a human has to answer, not a comment a human is trusted
        // to notice.
        if !self.allow_drafts
            && let Some(f) = files.iter().find(|f| f.draft)
        {
            return Err(lint_kind(
                &f.name,
                "declares draft = true — an observation-derived, unreviewed registry \
                 (RFC 08 §6.1). Review it, drop the marker and add `since` to promote \
                 it; or build with Config::allow_drafts() to prototype against it",
                LintKind::Draft,
            ));
        }
        let ledger = self
            .ledger
            .clone()
            .unwrap_or_else(|| self.registry_dir.join("deprecated.lock"));
        check_deprecation_ledger(&ledger, &files)?;
        check_conditional_ledger(&self.conditional_path(), &files)?;
        check_type_table(&self.registry_dir, &files)?;
        // The compatibility lock (RFC 08 §3.1): opting out is legal and loud.
        // "Loud" is the caller's to arrange — this collects, it does not print.
        // An admitted draft is louder still, and its warning subsumes the
        // opt-out one (a draft is `compat = "none"` by construction).
        let warnings: Vec<RegistryWarning> = files
            .iter()
            .filter(|f| f.compat == Compat::None)
            .map(|f| RegistryWarning {
                file: f.name.clone(),
                message: if f.draft {
                    "draft — observation-derived and unreviewed (RFC 08 §6.1): admitted \
                     by allow_drafts, unpinned, every field a guess; promote it by review"
                        .to_string()
                } else {
                    "declares compat = \"none\" — its entries are unpinned and \
                     incompatible edits pass unchecked (RFC 08 §3.1)"
                        .to_string()
                },
            })
            .collect();
        check_compat_lock(&self.compat_lock_path(), &files)?;
        Ok((files, warnings))
    }

    fn compat_lock_path(&self) -> PathBuf {
        self.compat_lock
            .clone()
            .unwrap_or_else(|| self.registry_dir.join("registry.lock"))
    }

    fn conditional_path(&self) -> PathBuf {
        self.conditional
            .clone()
            .unwrap_or_else(|| self.registry_dir.join("conditional.lock"))
    }

    /// The conditional subjects of this registry, validated (RFC 08 §6.1,
    /// v1.25) — the exemption half of the ledger's two-direction check.
    ///
    /// A subject listed here is **exempt** from the emitted-surface check: a
    /// consumer's build- or test-time check of "the build contains code that
    /// can publish it" MUST NOT require the build's mappers to cover a
    /// ledgered subject unconditionally. The slice entry itself still exists
    /// and is served through `introspect` unmarked — the ledger *conditions*
    /// an entry, it does not replace one, and the RFC 08 §6 slice format
    /// deliberately carries no conditional field (the `feature`/`when`
    /// schema design stays deferred, zenkey #171).
    ///
    /// Returns the same [`Error`] the build would: a line naming no registry
    /// subject fails here exactly as it fails `generate()`.
    pub fn conditional_subjects(&self) -> Result<Vec<ConditionalSubject>, Error> {
        let files = load_registry(&self.registry_dir)?;
        check_conditional_ledger(&self.conditional_path(), &files)
    }

    /// Write (or update) the RFC 08 §3.1 compatibility lock — the
    /// regeneration half of the check [`generate`](Self::generate) and
    /// [`lint`](Self::lint) enforce. Additive and retirement drift writes
    /// cleanly; an **incompatible** rewrite is refused unless the caller
    /// passes [`OnIncompatible::ForceAndReport`], and a forced write reports
    /// every broken pin so the caller can print them — the escape hatch is
    /// legal, silent it is not.
    ///
    /// A failure that is *not* about a broken pin — a malformed lock line —
    /// is never forceable and is always returned. That was a bug until #318:
    /// the old string match classified it as "not incompatible", and neither
    /// arm of the `if` then fired, so the error was dropped on the floor and
    /// the corrupt lock was overwritten instead of reported. Matching on
    /// [`LintKind`] made the third case impossible to leave out.
    pub fn write_compat_lock(&self, on: OnIncompatible) -> Result<CompatLockUpdate, Error> {
        let files = load_registry(&self.registry_dir)?;
        let path = self.compat_lock_path();
        let created = !path.exists();
        let check = check_compat_lock(&path, &files);
        let mut forced = Vec::new();
        match check {
            Ok(()) => {}
            Err(e) => {
                let kind = match &e {
                    Error::Lint { kind, .. } => *kind,
                    _ => return Err(e),
                };
                if kind.is_resolved_by_writing() {
                    // Stale is the drift this call exists to resolve: the
                    // rewrite below *is* the fix, so it proceeds silently.
                } else if kind.is_forceable() && on == OnIncompatible::ForceAndReport {
                    forced.push(e.to_string());
                } else {
                    // Refused, or a failure no rewrite can resolve — either
                    // way the caller hears about it rather than getting a
                    // fresh lock over the top of it.
                    return Err(e);
                }
            }
        }
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        // Counted by pin (kind, producer, path), not by whole line: a line
        // that only gained its `kind` column (v1.32) is neither added nor
        // retired.
        let pin = |l: &str| -> String { l.splitn(4, '\t').take(3).collect::<Vec<_>>().join("\t") };
        let old: std::collections::BTreeSet<String> = existing
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .map(pin)
            .collect();
        let new_lines = compat_lock_lines(&files);
        let new: std::collections::BTreeSet<String> = new_lines.iter().map(|l| pin(l)).collect();
        let added = new.difference(&old).count();
        let retired = old.difference(&new).count();
        std::fs::write(&path, compat_lock_content(&files))
            .map_err(|e| Error::Io(path.clone(), e))?;
        Ok(CompatLockUpdate {
            path,
            created,
            added,
            retired,
            forced,
        })
    }
}

/// Parse a registry subject path, and apply the reservations a *local*
/// registry is held to.
///
/// The lexical rules are [`SubjectPattern::parse`]'s — one implementation,
/// which is what `zenkey/src/pattern.rs`'s module doc has claimed since v1.5
/// while this file kept a second hand-rolled copy of the same `{var}` /
/// `{var...}` grammar. The parity was a coincidence, not a guarantee (#320).
///
/// What stays here is what does *not* belong there: `alive` is reserved at
/// any position of any registered pattern (RFC 03 §3, v1.25 A5b), and
/// `SubjectPattern` also parses patterns served by a *foreign* fleet, where
/// this deployment's reservations do not apply.
fn parse_pattern(file: &str, path: &str) -> Result<Vec<Chunk>, Error> {
    let parsed = SubjectPattern::parse(path).map_err(|e| match e {
        PatternError::Empty => lint(file, "empty subject path"),
        PatternError::RestNotTrailing(_) => lint(
            file,
            format!("{path:?}: {{var...}} only in trailing position (RFC 08 §2)"),
        ),
        PatternError::BadVarName(v) => lint(file, format!("{path:?}: bad variable name {v:?}")),
        PatternError::BadChunk(c) => {
            lint(file, format!("{path:?}: chunk {c:?} violates RFC 03 §2"))
        }
    })?;
    for chunk in parsed.chunks() {
        if let Chunk::Literal(l) = chunk
            && l == "alive"
        {
            return Err(lint(
                file,
                format!("{path:?}: `alive` is a reserved liveliness leaf (RFC 03 §3)"),
            ));
        }
    }
    Ok(parsed.chunks().to_vec())
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

fn load_registry(dir: &Path) -> Result<Vec<RegistryFile>, Error> {
    let mut files = Vec::new();
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| Error::Io(dir.to_path_buf(), e))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .filter(|p| p.file_name().is_none_or(|n| n != "types.toml"))
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
        let compat = match header.get("compat").and_then(|v| v.as_str()) {
            None | Some("backward") => Compat::Backward,
            Some("none") => Compat::None,
            Some(other) => {
                return Err(lint(
                    &fname,
                    format!(
                        "[registry] compat must be \"backward\" (default) or \"none\" \
                         (RFC 08 §3.1), got {other:?}"
                    ),
                ));
            }
        };
        // `draft = true` (RFC 08 §6.1, v1.34): read here so the `since`
        // waiver below can see it; whether a draft is *admitted* is
        // `Config::checked`'s question. A draft MUST be `compat = "none"` —
        // pinning entries nobody has reviewed would lock guesses in.
        let draft = match header.get("draft") {
            None => false,
            Some(v) => v
                .as_bool()
                .ok_or_else(|| lint(&fname, "[registry] draft must be a boolean (RFC 08 §6.1)"))?,
        };
        if draft && compat != Compat::None {
            return Err(lint(
                &fname,
                "declares draft = true with compat pinned — a draft cannot be pinned; \
                 it MUST carry compat = \"none\" (RFC 08 §6.1)",
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

        // [budget] (RFC 08 §2, v1.32). Every bound is an optional count and
        // takes the same sign check as `ttl_s` and `cardinality` (#313: a
        // negative fails here, where the TOML was authored); each table row
        // must name itself, uniquely, or `self_stats.tables[]` has nothing
        // to match it against (RFC 04 §1.2). Unknown keys are tolerated, as
        // everywhere else in this file.
        let budget = match doc.get("budget") {
            None => None,
            Some(b) => {
                let rss_mb = opt_count(&fname, b, "[budget]", "rss_mb")?;
                let mut tables = Vec::new();
                let mut names_seen = std::collections::BTreeSet::new();
                for row in b
                    .get("tables")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
                {
                    let tname = row.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
                        lint(&fname, "[[budget.tables]] missing name (RFC 08 §2)")
                    })?;
                    if !names_seen.insert(tname) {
                        return Err(lint(
                            &fname,
                            format!(
                                "[[budget.tables]] name {tname:?} declared twice — a table \
                                 name is unique within the file (RFC 08 §2)"
                            ),
                        ));
                    }
                    let path = format!("[[budget.tables]] {tname}");
                    tables.push(TableBudgetEntry {
                        name: tname.to_string(),
                        max_entries: opt_count(&fname, row, &path, "max_entries")?,
                        max_bytes: opt_count(&fname, row, &path, "max_bytes")?,
                    });
                }
                Some(BudgetEntry { rss_mb, tables })
            }
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
            // RFC 08 §2 (v1.32): `kind` is a closed vocabulary — the token a
            // judge compares the wire against, so a typo here is a lint,
            // never a silent "unchecked".
            let kind = match entry.get("kind").and_then(|v| v.as_str()) {
                None => None,
                Some(k) => match <SubjectKind as SliceToken>::from_token(k) {
                    Some(known) => Some(known.token().to_string()),
                    None => {
                        return Err(lint(
                            &fname,
                            format!(
                                "{spath:?}: unknown kind {k:?} — one of {} (RFC 08 §2)",
                                SubjectKind::ALL
                                    .iter()
                                    .map(|k| format!("`{}`", k.token()))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                        ));
                    }
                },
            };
            let cardinality = opt_count(&fname, entry, spath, "cardinality")?;
            let has_var = chunks.iter().any(|c| !matches!(c, Chunk::Literal(_)));
            if has_var && cardinality.is_none() {
                return Err(lint(
                    &fname,
                    format!("{spath:?}: {{var}} pattern needs integer cardinality (RFC 08 §5)"),
                ));
            }
            let ttl_s = opt_count(&fname, entry, spath, "ttl_s")?;
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
            // A draft has no version stream, so `since` is waived for it —
            // and MUST be absent: a draft carrying one would be claiming a
            // lifecycle it does not have (RFC 08 §6.1). `description` is
            // still required; the draft emitter writes one that says
            // "inferred".
            if entry.get("description").and_then(|v| v.as_str()).is_none()
                || (!draft && entry.get("since").and_then(|v| v.as_str()).is_none())
            {
                return Err(lint(
                    &fname,
                    format!("{spath:?}: missing description/since"),
                ));
            }
            if draft && entry.get("since").is_some() {
                return Err(lint(
                    &fname,
                    format!(
                        "{spath:?}: a draft entry carries `since` — a draft has no version \
                         stream (RFC 08 §6.1); promote the file instead"
                    ),
                ));
            }
            // `common = "..."` (RFC 04/06): declares this entry as one of the
            // framework state subjects; drives AnySubject::common_state().
            let common = entry
                .get("common")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            if let Some(c) = &common {
                let Some((_, _, canonical)) = COMMON_STATE.iter().find(|(n, _, _)| n == c) else {
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
                // RFC 04 §1.4 (v1.25): the `@catalog` trio is one service's
                // state, not a family across producers — an ordinary
                // producer file cannot claim a service token.
                if COMMON_SERVICE_TOKENS.contains(&c.as_str()) && service_origin.is_none() {
                    return Err(lint(
                        &fname,
                        format!(
                            "{spath:?}: common = {c:?} is a service subject (RFC 04 §1.4, \
                             RFC 06 §5) — only a [service] registry may claim it"
                        ),
                    ));
                }
                // RFC 04 §1.4 (v1.25): "the spelling is the table's" — the
                // entry's path must be the token's canonical pattern, chunk
                // for chunk: same literals, same variable names.
                let canonical_chunks = parse_pattern(&fname, canonical)
                    .expect("COMMON_STATE canonical patterns parse");
                let matches = chunks.len() == canonical_chunks.len()
                    && chunks
                        .iter()
                        .zip(&canonical_chunks)
                        .all(|(a, b)| match (a, b) {
                            (Chunk::Literal(x), Chunk::Literal(y)) => x == y,
                            (Chunk::Var(x), Chunk::Var(y)) | (Chunk::Rest(x), Chunk::Rest(y)) => {
                                snake(x) == snake(y)
                            }
                            _ => false,
                        });
                if !matches {
                    return Err(lint(
                        &fname,
                        format!(
                            "{spath:?}: common = {c:?} claims the framework subject \
                             {canonical:?} and must spell it exactly (RFC 04 §1.4: the \
                             spelling is the table's)"
                        ),
                    ));
                }
            }
            // RFC 04 §3: the alert family (`state/*/alert/*`) rides the
            // `alert` profile — reliable, blocking, interactive-high,
            // **express**; alerts are the one family the express axis exists
            // for. The class default above is per *class* and cannot see the
            // family, so an undeclared alert subject would silently compile
            // to `refreshed` — a drop-eligible firing flank. An alert-family
            // subject therefore declares its profile, and of the five only
            // `alert` is alert-or-stronger (v1.23; RFC 08 §2/§5).
            let alert_family = common.as_deref() == Some("alert")
                || (class == "state"
                    && matches!(chunks.first(), Some(Chunk::Literal(l)) if l == "alert"));
            if alert_family {
                match entry.get("qos").and_then(|v| v.as_str()) {
                    Some("alert") => {}
                    Some(weaker) => {
                        return Err(lint(
                            &fname,
                            format!(
                                "{spath:?}: alert state must use the alert profile or \
                                 stronger, not {weaker:?} (RFC 04 §3)"
                            ),
                        ));
                    }
                    None => {
                        return Err(lint(
                            &fname,
                            format!(
                                "{spath:?}: alert state must not fall to the class \
                                 default — declare qos = \"alert\" (RFC 04 §3, v1.23)"
                            ),
                        ));
                    }
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
                kind,
                cardinality,
                qos,
                ttl_s,
                rate,
                common,
                encoding: entry
                    .get("encoding")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
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
            // Literal and `{var}` chunks: RFC 09 (v1.4, amendment G6) requires
            // the actuated resource as a path chunk, so parameterized write
            // procedures are legal. Rest-vars are not — a procedure names one
            // operation, never an open family.
            let chunks = parse_pattern(&fname, ppath)?;
            if chunks.iter().any(|c| matches!(c, Chunk::Rest(_))) {
                return Err(lint(
                    &fname,
                    format!(
                        "procedure {ppath:?}: {{var...}} rest-variables are not allowed in procedure paths"
                    ),
                ));
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
            // fanout (RFC 08 §2, v1.4 G2): default Forbidden for writes,
            // Allowed for read/long-running; an explicit value must be one of
            // the two. Parsed since v1.5 (#9) — the builder-level refusal.
            let fanout = match entry.get("fanout").and_then(|v| v.as_str()) {
                // One parse, in the pattern. This was a match guard calling
                // `from_token` and an arm body calling it again behind
                // `.expect("checked by the guard")` — the invariant written
                // out by hand across the guard/body boundary, where the
                // pattern can just carry it.
                Some(token) => match Fanout::from_token(token) {
                    Some(f) => f,
                    None => {
                        return Err(lint(
                            &fname,
                            format!(
                                "procedure {ppath:?}: unknown fanout {token:?} (allowed|forbidden)"
                            ),
                        ));
                    }
                },
                None => {
                    if kind == "write" {
                        Fanout::Forbidden
                    } else {
                        Fanout::Allowed
                    }
                }
            };
            let idempotent = entry
                .get("idempotent")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            // `reply` is required (RFC 08 §2's field table): errors ride
            // `reply_err`, but a *success* reply always has a declared type —
            // a procedure whose reply nobody can decode is not registered.
            let reply = entry.get("reply").and_then(|v| v.as_str());
            if reply.is_none() {
                return Err(lint(
                    &fname,
                    format!("procedure {ppath:?}: missing reply type (RFC 08 §2)"),
                ));
            }
            // `{var}`-bearing procedure paths carry the same key-population
            // budget as subjects and media (RFC 08 §2/§5): the expansions are
            // real keys, and the budget review needs the bound declared.
            let cardinality = opt_count(&fname, entry, ppath, "cardinality")?;
            let has_var = chunks.iter().any(|c| matches!(c, Chunk::Var(_)));
            if has_var && cardinality.is_none() {
                return Err(lint(
                    &fname,
                    format!(
                        "procedure {ppath:?}: {{var}} pattern needs integer cardinality (RFC 08 §2)"
                    ),
                ));
            }
            let refs: Vec<&str> = ppath.split('/').collect();
            procedures.push(ProcedureEntry {
                path: ppath.to_string(),
                variant: camel(&refs),
                chunks,
                kind: kind.to_string(),
                request: entry
                    .get("request")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                reply: reply.map(str::to_string),
                fanout,
                idempotent,
                cardinality,
                encoding: entry
                    .get("encoding")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            });
        }

        // [[media]] entries (RFC 08 §2): patterns validated; a `{var}`-bearing
        // media path MUST declare a `cardinality` (the highest-bandwidth plane
        // must bound its fan-out — the `{tier}` chunk multiplies it), and every
        // entry MUST name an `attachment` type. Modeled since v1.5 (H2) so
        // builders can be generated.
        let mut media_entries = Vec::new();
        if let Some(media) = doc.get("media").and_then(|v| v.as_array()) {
            for entry in media {
                let mpath = entry
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| lint(&fname, "[[media]] missing path"))?;
                let chunks = parse_pattern(&fname, mpath)?;
                if chunks.iter().any(|c| matches!(c, Chunk::Rest(_))) {
                    return Err(lint(
                        &fname,
                        format!("[[media]] {mpath:?}: {{var...}} rest-variables are not allowed"),
                    ));
                }
                let has_var = chunks.iter().any(|c| !matches!(c, Chunk::Literal(_)));
                let cardinality = opt_count(&fname, entry, mpath, "cardinality")?;
                if has_var && cardinality.is_none() {
                    return Err(lint(
                        &fname,
                        format!(
                            "[[media]] {mpath:?}: {{var}} pattern needs integer cardinality \
                             (RFC 08 §2)"
                        ),
                    ));
                }
                let attachment = entry
                    .get("attachment")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        lint(
                            &fname,
                            format!("[[media]] {mpath:?}: missing attachment type (RFC 08 §2)"),
                        )
                    })?;
                let encoding = entry
                    .get("encoding")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        lint(
                            &fname,
                            format!("[[media]] {mpath:?}: missing encoding (RFC 08 §2)"),
                        )
                    })?;
                let variant = match entry.get("variant").and_then(|v| v.as_str()) {
                    Some(v) if is_valid_variant(v) => v.to_string(),
                    Some(v) => {
                        return Err(lint(
                            &fname,
                            format!("[[media]] {mpath:?}: variant {v:?} is not CamelCase"),
                        ));
                    }
                    None => variant_name(&chunks),
                };
                media_entries.push(MediaEntry {
                    path: mpath.to_string(),
                    chunks,
                    encoding: encoding.to_string(),
                    attachment: attachment.to_string(),
                    cardinality,
                    variant,
                });
            }
        }
        // Media variant collisions (same rule as subjects).
        {
            let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
            for m in &media_entries {
                if let Some(other) = seen.insert(m.variant.as_str(), m.path.as_str()) {
                    return Err(lint(
                        &fname,
                        format!(
                            "media {other:?} and {:?} collide on variant {:?}",
                            m.path, m.variant
                        ),
                    ));
                }
            }
        }

        // [[blob]] entries (RFC 08 §2/§5, v1.8). Every vocabulary here is
        // closed by RFC 07 §2, so every lint is decidable and none is a
        // matter of taste. There is no `path` to validate: blob key shapes
        // are fixed by the chapter and their variable chunks are content
        // addresses, so what an entry declares is which tier and endpoints
        // this origin serves.
        let mut blob_entries: Vec<BlobEntry> = Vec::new();
        if let Some(blobs) = doc.get("blob").and_then(|v| v.as_array()) {
            for entry in blobs {
                if entry.get("path").is_some() {
                    return Err(lint(
                        &fname,
                        "[[blob]] takes no path — blob key shapes are fixed by RFC 07 §2 and \
                         their variable chunks are content addresses (RFC 08 §2)",
                    ));
                }
                if entry.get("cardinality").is_some() {
                    return Err(lint(
                        &fname,
                        "[[blob]] takes no cardinality — RFC 03 §3 already carves blob ids and \
                         tree roots out of the budget as unbounded families (RFC 08 §2)",
                    ));
                }
                let tier = entry
                    .get("tier")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| lint(&fname, "[[blob]] missing tier (RFC 08 §2)"))?;
                if !BLOB_TIERS.contains(&tier) {
                    return Err(lint(
                        &fname,
                        format!(
                            "[[blob]] tier {tier:?} is not a reserved tier token ({}) — RFC 07 §2",
                            BLOB_TIERS.join(" | ")
                        ),
                    ));
                }
                let is_artifact = tier == "artifact";
                let is_store = tier == "store";

                // `endpoints` present exactly on `artifact`: the Tier-2 keys
                // *are* the endpoint, so naming one there is a category error
                // rather than a harmless extra.
                let endpoints: Vec<String> = match entry.get("endpoints") {
                    Some(v) => {
                        if !is_artifact {
                            return Err(lint(
                                &fname,
                                format!(
                                    "[[blob]] tier {tier:?} takes no endpoints — the key is the \
                                     endpoint (RFC 07 §2.3/§2.4)"
                                ),
                            ));
                        }
                        let arr = v.as_array().ok_or_else(|| {
                            lint(&fname, "[[blob]] endpoints must be an array of names")
                        })?;
                        let mut names: Vec<String> = Vec::with_capacity(arr.len());
                        for e in arr {
                            let n = e.as_str().ok_or_else(|| {
                                lint(&fname, "[[blob]] endpoints must be an array of names")
                            })?;
                            if !BLOB_ENDPOINTS.contains(&n) {
                                return Err(lint(
                                    &fname,
                                    format!(
                                        "[[blob]] endpoint {n:?} is not reserved by RFC 07 §2.2 \
                                         ({})",
                                        BLOB_ENDPOINTS.join(", ")
                                    ),
                                ));
                            }
                            if names.iter().any(|k| k == n) {
                                return Err(lint(
                                    &fname,
                                    format!("[[blob]] endpoint {n:?} listed twice"),
                                ));
                            }
                            names.push(n.to_string());
                        }
                        names
                    }
                    None if is_artifact => {
                        return Err(lint(
                            &fname,
                            "[[blob]] tier \"artifact\" must declare its endpoints (RFC 07 §2.2)",
                        ));
                    }
                    None => Vec::new(),
                };

                let algo = match entry.get("algo").and_then(|v| v.as_str()) {
                    Some(_) if !is_store => {
                        return Err(lint(
                            &fname,
                            format!("[[blob]] tier {tier:?} takes no algo (RFC 07 §2.4)"),
                        ));
                    }
                    Some(a) if !is_valid_plain_chunk(a) => {
                        return Err(lint(
                            &fname,
                            format!("[[blob]] algo {a:?} violates RFC 03 §2"),
                        ));
                    }
                    Some(a) => Some(a.to_string()),
                    None if is_store => {
                        return Err(lint(
                            &fname,
                            "[[blob]] tier \"store\" must declare its hash algo (RFC 07 §2.4)",
                        ));
                    }
                    None => None,
                };

                // Required metadata, same rule as every other entry kind
                // (RFC 08 §2). Prose is per-declaration — each declarer says
                // why *it* serves the tier — but it must exist.
                if entry.get("description").and_then(|v| v.as_str()).is_none()
                    || entry.get("since").and_then(|v| v.as_str()).is_none()
                {
                    return Err(lint(
                        &fname,
                        format!("[[blob]] tier {tier:?}: missing description/since"),
                    ));
                }

                blob_entries.push(BlobEntry {
                    tier: tier.to_string(),
                    endpoints,
                    algo,
                    reference: entry
                        .get("reference")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    encoding: entry
                        .get("encoding")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    description: entry
                        .get("description")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                });
            }
        }

        // The same `(tier, algo)` twice in *one* file is a copy-paste error.
        // Cross-file repetition is legitimate — each producer declares the
        // tiers it serves — and is shape-checked app-wide after all files
        // are read.
        {
            let mut seen: BTreeSet<(&str, Option<&str>)> = BTreeSet::new();
            for b in &blob_entries {
                if !seen.insert((b.tier.as_str(), b.algo.as_deref())) {
                    return Err(lint(
                        &fname,
                        format!("[[blob]] tier {:?} declared twice in one file", b.tier),
                    ));
                }
            }
        }

        // H4 (RFC 08 §5, v1.5): in a service registry, a subject pattern
        // containing the variable `{host}` must lead with it — the G1
        // desired-state proxy rule (the target host is addressing, and
        // addressing lives where ACL prefix rules can reach it).
        if service_origin.is_some() {
            for s in &subjects {
                let host_pos = s
                    .chunks
                    .iter()
                    .position(|c| matches!(c, Chunk::Var(v) | Chunk::Rest(v) if v == "host"));
                if let Some(pos) = host_pos
                    && pos != 0
                {
                    return Err(lint(
                        &fname,
                        format!(
                            "service subject {:?}: {{host}} must be the FIRST chunk \
                             (RFC 08 §5 H4, 07 §3)",
                            s.path
                        ),
                    ));
                }
            }
        }

        let mut deprecated: Vec<Deprecated> = Vec::new();
        if let Some(arr) = doc.get("deprecated").and_then(|v| v.as_array()) {
            for entry in arr {
                let path = entry
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| lint(&fname, "[[deprecated]] missing path"))?
                    .to_string();
                // `kind` defaults to `subject`, so every registry written
                // before v1.26 means exactly what it did (RFC 08 §3).
                let kind = match entry.get("kind").and_then(|v| v.as_str()) {
                    None => EntryKind::Subject,
                    Some(k) => EntryKind::parse(k).ok_or_else(|| {
                        lint(
                            &fname,
                            format!(
                                "[[deprecated]] {path:?} has kind = {k:?} — it is \
                                 `subject` (the default) or `procedure` (RFC 08 §3)"
                            ),
                        )
                    })?,
                };
                deprecated.push(Deprecated { kind, path });
            }
        }
        // A deprecated path is never re-registered — per kind, because a
        // subject and a procedure of the same name are two declarations.
        for d in &deprecated {
            let live = match d.kind {
                EntryKind::Subject => subjects.iter().any(|s| s.path == d.path),
                EntryKind::Procedure => procedures.iter().any(|p| p.path == d.path),
            };
            if live {
                return Err(lint(
                    &fname,
                    format!(
                        "deprecated path {:?} re-registered as a live {} (RFC 08 §3)",
                        d.path,
                        d.kind.token()
                    ),
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
            media: media_entries,
            blob: blob_entries,
            deprecated,
            compat,
            budget,
            draft,
        });
    }

    // Blob shape agreement is **app-wide** — the one cross-file rule here,
    // and for a structural reason: a blob key carries no producer chunk
    // (RFC 07 §2), so every declaration of one tier names the same app-level
    // key family. Several producers MAY each declare a tier — the introspect
    // slice is per-producer truth (RFC 08 §6), and "does this producer serve
    // blobs?" must be answerable per producer — but the family has one
    // shape, so the declarations must agree on it: endpoints (as a set),
    // reference, encoding. `since` and `description` are per-declaration
    // prose and free to differ. Agreement is keyed per `(tier, algo)`
    // (RFC 08 §2, v1.17): `algo` is the store tier's migration
    // discriminator, so `store`/`blake3` and `store`/`sha256` are two
    // address families that agree internally, not a conflict.
    {
        fn sorted(v: &[String]) -> Vec<&str> {
            let mut s: Vec<&str> = v.iter().map(String::as_str).collect();
            s.sort_unstable();
            s
        }
        let mut canon: BTreeMap<(&str, Option<&str>), (&BlobEntry, &str)> = BTreeMap::new();
        for f in &files {
            for b in &f.blob {
                let family = (b.tier.as_str(), b.algo.as_deref());
                let Some(&(first, first_file)) = canon.get(&family) else {
                    canon.insert(family, (b, f.name.as_str()));
                    continue;
                };
                let field = if sorted(&first.endpoints) != sorted(&b.endpoints) {
                    Some("endpoints")
                } else if first.reference != b.reference {
                    Some("reference")
                } else if first.encoding != b.encoding {
                    Some("encoding")
                } else {
                    None
                };
                if let Some(field) = field {
                    return Err(lint(
                        "registry",
                        format!(
                            "blob tier {:?}: {:?} declares a different {field} than \
                             {first_file:?}; blob keys have no producer chunk, so every \
                             declarer serves one app-level key family and shapes must \
                             agree (RFC 08 §5)",
                            b.tier, f.name
                        ),
                    ));
                }
            }
        }
        // `algo` is deliberately not part of the shape: it is the store
        // tier's **migration discriminator** (RFC 08 §2/§5, v1.17). A second
        // concurrent algo is the sanctioned dual-algo window of RFC 07 §2.4,
        // and codegen emits one builder and one address type per declared
        // algo, so both address spaces are spellable — and unmixable — at
        // once. (From v1.8 to v1.16 this was a build diagnostic instead,
        // which is how one downstream hash migration became a flag day.)
    }
    Ok(files)
}

/// The append-only deprecation ledger (RFC 08 §3/§5): every line is
/// `<producer>\t<path>`. A ledger line without its TOML entry = someone
/// deleted a deprecation; a TOML deprecation missing from the ledger = the
/// ledger append was forgotten. Both fail the build.
/// The RFC 08 §5 type-table resolution lint (v1.5, H6): when
/// `registry/types.toml` exists, every `type`/`request`/`reply`/`attachment`
/// name — and, since v1.8, every `[[blob]]` `reference` — across the registry
/// set must resolve in it. Absent file = lint
/// inactive (activation-on-existence, so adoption is incremental).
fn check_type_table(dir: &Path, files: &[RegistryFile]) -> Result<(), Error> {
    let path = dir.join("types.toml");
    let Ok(src) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let fname = "types.toml";
    // `toml::from_str`, not `str::parse`: since toml 0.9, `Value: FromStr`
    // parses a single TOML *value*, and only `from_str` parses a document.
    let doc: toml::Value =
        toml::from_str(&src).map_err(|e| lint(fname, format!("does not parse: {e}")))?;
    let table = doc
        .get("types")
        .and_then(|v| v.as_table())
        .ok_or_else(|| lint(fname, "missing [types.*] table"))?;
    for (name, entry) in table {
        if entry.get("kind").and_then(|v| v.as_str()).is_none() {
            return Err(lint(fname, format!("[types.{name}] missing kind")));
        }
    }
    let mut missing = std::collections::BTreeSet::new();
    let mut check = |t: &str| {
        if !table.contains_key(t) {
            missing.insert(t.to_string());
        }
    };
    for f in files {
        for s in &f.subjects {
            check(&s.payload_type);
        }
        for p in &f.procedures {
            if let Some(t) = &p.request {
                check(t);
            }
            if let Some(t) = &p.reply {
                check(t);
            }
        }
        for m in &f.media {
            check(&m.attachment);
        }
        for b in &f.blob {
            if let Some(t) = &b.reference {
                check(t);
            }
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(lint(
            fname,
            format!(
                "registry type name(s) not in the type table (RFC 08 §5): {}",
                missing.into_iter().collect::<Vec<_>>().join(", ")
            ),
        ))
    }
}

/// One `registry.lock` line per pinned entry, sorted for determinism.
/// Files declaring `compat = "none"` contribute nothing — their entries are
/// deliberately unpinned (RFC 08 §3.1).
fn compat_lock_lines(files: &[RegistryFile]) -> Vec<String> {
    let mut lines = Vec::new();
    for f in files.iter().filter(|f| f.compat == Compat::Backward) {
        for s in &f.subjects {
            // `kind` (RFC 08 §2, v1.32) rides as an optional sixth column:
            // absent stays absent, so a registry that never declares one
            // produces the lock it always did.
            let mut line = format!(
                "subject\t{}\t{}\t{}\t{}",
                f.name, s.path, s.class, s.payload_type
            );
            if let Some(k) = &s.kind {
                line.push('\t');
                line.push_str(k);
            }
            lines.push(line);
        }
        for p in &f.procedures {
            lines.push(format!(
                "procedure\t{}\t{}\t{}\t{}\t{}",
                f.name,
                p.path,
                p.kind,
                p.request.as_deref().unwrap_or("-"),
                p.reply.as_deref().unwrap_or("-"),
            ));
        }
    }
    lines.sort();
    lines
}

/// The full `registry.lock` file content for the current registry.
fn compat_lock_content(files: &[RegistryFile]) -> String {
    let mut out = String::from(
        "# registry.lock — the RFC 08 §3.1 compatibility snapshot.\n\
         # Regenerate with `zenctl registry lock <dir>` after additive edits;\n\
         # an incompatible edit (changed type/class/kind/shape on an existing\n\
         # path) is refused — retire through [[deprecated]] and add a sibling\n\
         # instead (RFC 08 §3). A subject line's optional sixth column is its\n\
         # declared `kind` (RFC 08 §2, v1.32): adding one is additive,\n\
         # changing or removing one is incompatible.\n",
    );
    for l in compat_lock_lines(files) {
        out.push_str(&l);
        out.push('\n');
    }
    out
}

/// How a pinned line differs from the line the registry would write now,
/// once they are known to differ (RFC 08 §3.1).
#[derive(Debug, PartialEq, Eq)]
enum LockDrift {
    /// One of the shape columns moved — class, type, a procedure's kind or
    /// request/reply shape. Always incompatible.
    Shape,
    /// The pin had no `kind` and the registry now declares one (v1.32):
    /// additive, so merely stale.
    KindAdded,
    /// The pin had a `kind` and it changed or vanished (v1.32): incompatible,
    /// naming the column. `to` reads `-` for a removal.
    KindChanged { from: String, to: String },
}

/// The lock line's shape columns and, for a subject, its optional trailing
/// `kind` (RFC 08 §2, v1.32). A procedure line is all shape: its `kind` is
/// the read/write idiom, in a fixed column.
fn lock_line_columns(line: &str) -> (Vec<&str>, Option<&str>) {
    let cols: Vec<&str> = line.split('\t').collect();
    if cols.first() == Some(&"subject") && cols.len() > 5 {
        (cols[..5].to_vec(), Some(cols[5]))
    } else {
        (cols, None)
    }
}

fn lock_line_drift(pinned: &str, now: &str) -> LockDrift {
    let (old_shape, old_kind) = lock_line_columns(pinned);
    let (new_shape, new_kind) = lock_line_columns(now);
    if old_shape != new_shape {
        return LockDrift::Shape;
    }
    match (old_kind, new_kind) {
        (None, Some(_)) => LockDrift::KindAdded,
        (Some(from), to) => LockDrift::KindChanged {
            from: from.to_string(),
            to: to.unwrap_or("-").to_string(),
        },
        // Equal shape, no kind on either side, yet the lines differ — a
        // trailing column this build does not know. Refuse to guess.
        (None, None) => LockDrift::Shape,
    }
}

/// Check the compatibility lock (RFC 08 §3.1): every pinned entry must still
/// exist with identical shape, unless its subject was retired through
/// `[[deprecated]]`. Additive drift (entries the lock has not pinned yet, or
/// retired lines lingering) is stale, not incompatible — the error says to
/// regenerate. A missing lock is an empty snapshot: everything current reads
/// as unpinned, and the same regeneration message bootstraps it.
fn check_compat_lock(lock_path: &Path, files: &[RegistryFile]) -> Result<(), Error> {
    use std::collections::BTreeMap;
    let fname = "registry.lock";
    let existing = std::fs::read_to_string(lock_path).unwrap_or_default();
    // (kind, producer, path) → the full pinned line.
    let index = |lines: &[String]| -> Result<BTreeMap<(String, String, String), String>, Error> {
        let mut m = BTreeMap::new();
        for l in lines {
            let mut it = l.splitn(4, '\t');
            let (Some(kind), Some(producer), Some(path)) = (it.next(), it.next(), it.next()) else {
                return Err(lint_kind(
                    fname,
                    format!("bad lock line {l:?}"),
                    LintKind::BadLine,
                ));
            };
            m.insert(
                (kind.to_string(), producer.to_string(), path.to_string()),
                l.clone(),
            );
        }
        Ok(m)
    };
    let old_lines: Vec<String> = existing
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect();
    let old = index(&old_lines)?;
    let desired = index(&compat_lock_lines(files))?;

    let mut stale = Vec::new();
    for (key, line) in &old {
        match desired.get(key) {
            Some(new_line) if new_line == line => {}
            Some(new_line) => match lock_line_drift(line, new_line) {
                LockDrift::Shape => {
                    // Same path, different shape — the exact edit §3 forbids.
                    return Err(lint_kind(
                        fname,
                        format!(
                            "incompatible registry edit (RFC 08 §3.1, compat = \"backward\"):\n  \
                             pinned:  {line}\n  now:     {new_line}\n\
                             an existing path never changes shape — retire it through \
                             [[deprecated]] and add a sibling (`sockets` → `sockets2`, RFC 08 §3)"
                        ),
                        LintKind::Incompatible,
                    ));
                }
                LockDrift::KindAdded => {
                    // Additive metadata (RFC 08 §3.1, v1.32): the snapshot
                    // has to follow, nothing else changed.
                    stale.push(line.clone());
                }
                LockDrift::KindChanged { from, to } => {
                    return Err(lint_kind(
                        fname,
                        format!(
                            "incompatible registry edit (RFC 08 §3.1, compat = \"backward\"):\n  \
                             pinned:  {line}\n  now:     {new_line}\n\
                             the `kind` column changed ({from} → {to}) — a consumer that \
                             learned to rate() a counter is wrong the moment the same path \
                             is a gauge; retire it through [[deprecated]] and add a sibling \
                             (RFC 08 §2, §3)"
                        ),
                        LintKind::Incompatible,
                    ));
                }
            },
            None => {
                let (kind, producer, path) = key;
                // Both kinds retire (RFC 08 §3, v1.26): this used to read
                // `kind == "subject"`, which left a removed procedure with no
                // sanctioned exit at all — the `[[deprecated]]` entry that is
                // supposed to be the exit was never consulted for it (#377).
                let retired = EntryKind::parse(kind).is_some_and(|k| {
                    files.iter().any(|f| {
                        &f.name == producer
                            && f.deprecated.iter().any(|d| d.kind == k && &d.path == path)
                    })
                });
                let compat_off = files
                    .iter()
                    .any(|f| &f.name == producer && f.compat == Compat::None);
                if retired || compat_off {
                    // The sanctioned exits: [[deprecated]] (whose own ledger
                    // is append-only), or the file opting out loudly.
                    stale.push(line.clone());
                } else {
                    return Err(lint_kind(
                        fname,
                        format!(
                            "pinned entry vanished without retirement (RFC 08 §3.1):\n  {line}\n\
                             a {kind} is removed by deprecating it ([[deprecated]] + the \
                             deprecated.lock ledger), never by deletion — or the file \
                             declares compat = \"none\" and says so out loud"
                        ),
                        LintKind::Vanished,
                    ));
                }
            }
        }
    }
    let missing: Vec<&String> = desired
        .iter()
        .filter(|(k, _)| !old.contains_key(*k))
        .map(|(_, v)| v)
        .collect();
    if !missing.is_empty() || !stale.is_empty() {
        let mut sample: Vec<String> = missing.iter().take(10).map(|s| (*s).clone()).collect();
        if missing.len() > 10 {
            sample.push(format!("… and {} more", missing.len() - 10));
        }
        return Err(lint_kind(
            fname,
            format!(
                "stale lock: {} unpinned entr(y/ies), {} retired or metadata-only \
                 line(s) lingering — additive evolution is free but the snapshot must \
                 follow; run `zenctl registry lock <dir>` (RFC 08 §3.1){}{}",
                missing.len(),
                stale.len(),
                if sample.is_empty() { "" } else { "\n  " },
                sample.join("\n  ")
            ),
            LintKind::Stale,
        ));
    }
    Ok(())
}

/// What [`Config::write_compat_lock`] did.
#[derive(Debug, Clone)]
pub struct CompatLockUpdate {
    /// Where the lock was written.
    pub path: PathBuf,
    /// No lock existed before.
    pub created: bool,
    /// Entries newly pinned.
    pub added: usize,
    /// Previously pinned lines released (retired through `[[deprecated]]`
    /// or the file went `compat = "none"`).
    pub retired: usize,
    /// Pinned lines rewritten or dropped **incompatibly** — populated only
    /// under `force`, and the caller prints every one: a forced break is
    /// loud by contract (RFC 08 §3.1).
    pub forced: Vec<String>,
}

/// One `conditional.lock` line (RFC 08 §6.1, v1.25): a registered subject
/// whose emission is gated — by a compile-time feature, an operator switch,
/// or a host capability — recorded so the emitted-surface check can exempt
/// it instead of silently not asking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionalSubject {
    /// The producer whose registry file declares the subject.
    pub producer: String,
    /// The subject `path`, exactly as the registry entry spells it.
    pub path: String,
    /// The gating condition, as **free text** for the human deciding whether
    /// the gate still exists — deliberately not a machine-readable
    /// expression (the field-level `feature`/`when` design stays deferred,
    /// zenkey #171).
    pub condition: String,
}

/// The RFC 08 §6.1 (v1.25) conditional-subject ledger, checked in the
/// direction zenkey-build can observe: every line must name a registry
/// subject that still exists — "the entry was retired or renamed, and the
/// line must follow it or leave". A missing file is an empty ledger.
///
/// The other direction — the exemption from the emitted-surface check —
/// belongs to the consumer's own build- or test-time coverage check, which
/// reads the validated set via [`Config::conditional_subjects`]. Unlike
/// `deprecated.lock`, this ledger is **not** append-only: a line leaves when
/// its condition does, and the subject re-enters the emitted-surface check
/// by deletion.
fn check_conditional_ledger(
    path: &Path,
    files: &[RegistryFile],
) -> Result<Vec<ConditionalSubject>, Error> {
    let ledger = std::fs::read_to_string(path).unwrap_or_default();
    let mut entries = Vec::new();
    for line in ledger
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
    {
        // Producer and path name the registry entry exactly as a
        // `deprecated.lock` line does; the condition is free text (which may
        // itself contain tabs — everything after the second is the text).
        let mut fields = line.splitn(3, '\t');
        let (Some(producer), Some(spath), Some(condition)) =
            (fields.next(), fields.next(), fields.next())
        else {
            return Err(lint(
                "conditional.lock",
                format!(
                    "bad ledger line {line:?} — expected \
                     <producer>\\t<path>\\t<condition> (RFC 08 §6.1)"
                ),
            ));
        };
        if condition.trim().is_empty() {
            return Err(lint(
                "conditional.lock",
                format!(
                    "ledger line {line:?} has an empty condition — the gate is \
                     the point of the line (RFC 08 §6.1)"
                ),
            ));
        }
        let file = files.iter().find(|f| f.name == producer);
        let live = file.is_some_and(|f| f.subjects.iter().any(|s| s.path == spath));
        if !live {
            let retired = file.is_some_and(|f| {
                f.deprecated
                    .iter()
                    .any(|d| d.kind == EntryKind::Subject && d.path == spath)
            });
            return Err(lint(
                "conditional.lock",
                format!(
                    "ledger line {line:?} names no registry subject{} — the entry \
                     was retired or renamed, and the line must follow it or \
                     leave (RFC 08 §6.1)",
                    if retired {
                        " (it is retired through [[deprecated]])"
                    } else {
                        ""
                    }
                ),
            ));
        }
        entries.push(ConditionalSubject {
            producer: producer.to_string(),
            path: spath.to_string(),
            condition: condition.to_string(),
        });
    }
    Ok(entries)
}

/// One `deprecated.lock` line, in either spelling (RFC 08 §3).
///
/// `<kind>\t<producer>\t<path>` since v1.26; a two-field
/// `<producer>\t<path>` line is the older spelling and means `subject`, so
/// every ledger written before the amendment keeps validating and nothing
/// asks anyone to rewrite one.
fn parse_ledger_line(l: &str) -> Result<(EntryKind, &str, &str), Error> {
    let fields: Vec<&str> = l.split('\t').collect();
    match fields.as_slice() {
        [producer, path] => Ok((EntryKind::Subject, producer, path)),
        [kind, producer, path] => match EntryKind::parse(kind) {
            Some(k) => Ok((k, producer, path)),
            None => Err(lint(
                "deprecated.lock",
                format!(
                    "ledger line {l:?} starts with {kind:?} — a three-field line \
                     is `<kind>\\t<producer>\\t<path>`, kind `subject` or \
                     `procedure` (RFC 08 §3)"
                ),
            )),
        },
        _ => Err(lint("deprecated.lock", format!("bad ledger line {l:?}"))),
    }
}

fn check_deprecation_ledger(ledger_path: &Path, files: &[RegistryFile]) -> Result<(), Error> {
    let ledger = std::fs::read_to_string(ledger_path).unwrap_or_default();
    let mut ledger_entries: Vec<(EntryKind, &str, &str)> = Vec::new();
    for l in ledger
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
    {
        ledger_entries.push(parse_ledger_line(l)?);
    }
    for (kind, producer, path) in &ledger_entries {
        let present = files.iter().any(|f| {
            f.name == *producer
                && f.deprecated
                    .iter()
                    .any(|d| d.kind == *kind && d.path == *path)
        });
        if !present {
            return Err(lint(
                "deprecated.lock",
                format!(
                    "ledger entry {}{producer}\t{path} has no [[deprecated]] entry — deprecations are append-only, restore it (RFC 08 §3)",
                    match kind {
                        EntryKind::Subject => String::new(),
                        k => format!("{}\t", k.token()),
                    }
                ),
            ));
        }
    }
    for f in files {
        for d in &f.deprecated {
            let listed = ledger_entries
                .iter()
                .any(|(k, p, path)| *k == d.kind && *p == f.name && *path == d.path);
            if !listed {
                // The suggested line keeps the two-field spelling for a
                // subject: the common case asks for no new syntax.
                let line = match d.kind {
                    EntryKind::Subject => format!("{}\t{}", f.name, d.path),
                    k => format!("{}\t{}\t{}", k.token(), f.name, d.path),
                };
                return Err(lint(
                    "deprecated.lock",
                    format!(
                        "[[deprecated]] {} {:?} in {} is not in the ledger — append `{line}` to the ledger file",
                        d.kind.token(),
                        d.path,
                        f.name
                    ),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write `content` as `registry/<name>.toml` in a fresh temp dir and run
    /// the linter via `generate_string`.
    fn lint_one(content: &str) -> Result<String, Error> {
        let dir = std::env::temp_dir().join(format!(
            "zenkey-build-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("t.toml"), content).unwrap();
        // Bootstrap the §3.1 lock so lint tests exercise *their* lint, not
        // the missing-snapshot bootstrap (which has its own tests below). An
        // unloadable registry fails identically with or without this.
        let _ = Config::new()
            .registry_dir(&dir)
            .write_compat_lock(OnIncompatible::Refuse);
        let out = Config::new().registry_dir(&dir).generate_string();
        let _ = std::fs::remove_dir_all(&dir);
        out
    }

    const HEADER: &str = "[registry]\nversion = \"1.0\"\napp = \"t\"\nconvention = 1\n";

    /// #168 put the cross-producer family table in zenkey
    /// (`CommonFamily`, driving `selector::common_family`); this lint's own
    /// [`COMMON_STATE`] table must agree with it — same registry tokens,
    /// same canonical patterns (fixed chunks and the trailing population
    /// variable, RFC 04 §1.4) — wherever they overlap, or a subject the
    /// lint accepts would fall outside the selector the runtime builds.
    #[test]
    fn common_state_table_agrees_with_zenkey_common_family() {
        use zenkey::CommonFamily;
        for f in CommonFamily::ALL {
            let (_, _, canonical) = COMMON_STATE
                .iter()
                .find(|(n, _, _)| *n == f.token())
                .unwrap_or_else(|| panic!("COMMON_STATE has no row for {:?}", f.token()));
            let mut want: Vec<String> = f.prefix().iter().map(|c| (*c).to_string()).collect();
            if let Some(v) = f.var() {
                want.push(format!("{{{v}}}"));
            }
            assert_eq!(
                *canonical,
                want.join("/"),
                "canonical pattern for {:?}",
                f.token()
            );
        }
        // And the rows zenkey does *not* know are exactly the `@catalog`
        // service set — one service's subjects, not families across
        // producers — which is also, by construction, the service-only list
        // the lint enforces.
        let extra: Vec<&str> = COMMON_STATE
            .iter()
            .map(|(n, _, _)| *n)
            .filter(|n| CommonFamily::ALL.iter().all(|f| f.token() != *n))
            .collect();
        assert_eq!(
            extra,
            [
                "entity", "alias", "pdns", "incident", "ack", "silence", "edge"
            ]
        );
        assert_eq!(extra, COMMON_SERVICE_TOKENS);
    }

    #[test]
    fn fanout_rejects_unknown_values() {
        let toml = format!(
            "{HEADER}[producer]\nname = \"t\"\n\n[[procedure]]\npath = \"x/set\"\nkind = \"write\"\nreply = \"Ack\"\nfanout = \"sometimes\"\nsince = \"1.0\"\ndescription = \"d\"\n"
        );
        let err = lint_one(&toml).unwrap_err();
        assert!(err.to_string().contains("unknown fanout"), "{err}");
    }

    #[test]
    fn explicit_fanout_values_parse() {
        let toml = format!(
            "{HEADER}[producer]\nname = \"t\"\n\n[[procedure]]\npath = \"x/set\"\nkind = \"write\"\nreply = \"Ack\"\nfanout = \"allowed\"\nidempotent = true\nsince = \"1.0\"\ndescription = \"d\"\n"
        );
        lint_one(&toml).unwrap();
    }

    /// RFC 04 §3: the alert family rides the `alert` profile — the class
    /// default (`refreshed`) is drop-eligible on the one family the express
    /// axis exists for, so an alert-family subject declares its profile and
    /// nothing weaker passes.
    #[test]
    fn alert_state_declares_the_alert_profile() {
        let subject = |extra: &str| {
            format!(
                "{HEADER}[producer]\nname = \"t\"\n\n[[subject]]\npath = \"alert/{{alert_key}}\"\nclass = \"state\"\ntype = \"Alert\"\ncommon = \"alert\"\n{extra}ttl_s = 900\ncardinality = 64\nsince = \"1.0\"\ndescription = \"d\"\n"
            )
        };
        // Absent: the class default cannot see the family — refused with the
        // fix spelled out.
        let err = lint_one(&subject("")).unwrap_err();
        assert!(err.to_string().contains("qos = \"alert\""), "{err}");
        assert!(err.to_string().contains("RFC 04 §3"), "{err}");
        // Declared weaker: refused.
        let err = lint_one(&subject("qos = \"refreshed\"\n")).unwrap_err();
        assert!(err.to_string().contains("alert profile"), "{err}");
        // Declared right: passes.
        lint_one(&subject("qos = \"alert\"\n")).unwrap();

        // The family *shape* is linted even without `common = "alert"` — the
        // selector `state/*/alert/*` does not read the common marker either.
        let shaped = format!(
            "{HEADER}[producer]\nname = \"t\"\n\n[[subject]]\npath = \"alert/{{key}}\"\nclass = \"state\"\ntype = \"Alert\"\nttl_s = 900\ncardinality = 64\nsince = \"1.0\"\ndescription = \"d\"\n"
        );
        let err = lint_one(&shaped).unwrap_err();
        assert!(err.to_string().contains("RFC 04 §3"), "{err}");
    }

    /// RFC 04 §1.4 (v1.25): a `common` token is a claim that this entry *is*
    /// that framework subject, so the path must be the token's canonical
    /// pattern — literals included. Before v1.25 the lint checked only the
    /// variable names, and `common = "health"` on `path = "wellness"`
    /// passed.
    #[test]
    fn common_token_requires_the_canonical_spelling() {
        let subject = |path: &str, common: &str| {
            format!(
                "{HEADER}[producer]\nname = \"t\"\n\n[[subject]]\npath = \"{path}\"\nclass = \"state\"\ntype = \"T\"\ncommon = \"{common}\"\nttl_s = 900\ncardinality = 64\nsince = \"1.0\"\ndescription = \"d\"\n"
            )
        };
        // The audit's example: right variables (none), wrong literal.
        let err = lint_one(&subject("wellness", "health")).unwrap_err();
        assert!(err.to_string().contains("RFC 04 §1.4"), "{err}");
        assert!(err.to_string().contains("\"health\""), "{err}");
        // Same shape and variable names, wrong leading literal.
        let err = lint_one(&subject("alarm/{alert_key}", "alert")).unwrap_err();
        assert!(err.to_string().contains("alert/{alert_key}"), "{err}");
        // A wrong variable name is still refused, as before.
        let err = lint_one(&subject("alert/{key}", "alert")).unwrap_err();
        assert!(err.to_string().contains("RFC 04 §1.4"), "{err}");
        // The canonical spellings pass.
        lint_one(&subject("health", "health")).unwrap();
        lint_one(&subject("evidence/device/{device}", "evidence_device")).unwrap();
    }

    /// RFC 04 §1.4 (v1.25): `entity`/`alias`/`pdns` are the `@catalog`
    /// service's state (RFC 06 §5) — an ordinary producer file cannot claim
    /// them; a `[service]` file can. v1.29/v1.30 (#425) widened the set to
    /// `incident`/`ack`/`silence`/`edge`, under the same rule.
    #[test]
    fn service_tokens_are_service_only() {
        let err = lint_one(&format!(
            "{HEADER}[producer]\nname = \"t\"\n\n[[subject]]\npath = \"entity/{{entity_id}}\"\nclass = \"state\"\ntype = \"T\"\ncommon = \"entity\"\nttl_s = 900\ncardinality = 64\nsince = \"1.0\"\ndescription = \"d\"\n"
        ))
        .unwrap_err();
        assert!(err.to_string().contains("service subject"), "{err}");
        assert!(err.to_string().contains("[service]"), "{err}");

        // The passing form: the same entry under a [service] registry.
        lint_one(&format!(
            "{HEADER}[service]\nname = \"catalog\"\norigin = \"@catalog\"\ndescription = \"d\"\n\n[[subject]]\npath = \"pdns/{{ip_slug}}\"\nclass = \"state\"\ntype = \"T\"\ncommon = \"pdns\"\nttl_s = 900\ncardinality = 64\nsince = \"1.0\"\ndescription = \"d\"\n"
        ))
        .unwrap();

        // Every service token, both ways: refused on a producer, accepted
        // on the service at its canonical spelling.
        for (token, path) in [
            ("incident", "incident/{incident_id}"),
            ("ack", "ack/{alert_ref}"),
            ("silence", "silence/{id}"),
            ("edge", "edge/{edge_id}"),
        ] {
            let entry = format!(
                "[[subject]]\npath = \"{path}\"\nclass = \"state\"\ntype = \"T\"\ncommon = \"{token}\"\nttl_s = 900\ncardinality = 64\nsince = \"1.0\"\ndescription = \"d\"\n"
            );
            let err =
                lint_one(&format!("{HEADER}[producer]\nname = \"t\"\n\n{entry}")).unwrap_err();
            assert!(
                err.to_string().contains("service subject"),
                "{token}: {err}"
            );
            lint_one(&format!(
                "{HEADER}[service]\nname = \"catalog\"\norigin = \"@catalog\"\ndescription = \"d\"\n\n{entry}"
            ))
            .unwrap_or_else(|e| panic!("{token}: {e}"));
        }

        // …and the v1.30 producer-side family lints on an ordinary producer.
        lint_one(&format!(
            "{HEADER}[producer]\nname = \"t\"\n\n[[subject]]\npath = \"evidence/relation/{{relation_id}}\"\nclass = \"state\"\ntype = \"T\"\ncommon = \"evidence_relation\"\nttl_s = 900\ncardinality = 64\nsince = \"1.0\"\ndescription = \"d\"\n"
        ))
        .unwrap();
    }

    /// RFC 08 §2's field table marks `reply` required: errors ride
    /// `reply_err`, but a success reply always has a declared type.
    #[test]
    fn a_procedure_without_a_reply_type_is_refused() {
        let toml = format!(
            "{HEADER}[producer]\nname = \"t\"\n\n[[procedure]]\npath = \"x/set\"\nkind = \"write\"\nsince = \"1.0\"\ndescription = \"d\"\n"
        );
        let err = lint_one(&toml).unwrap_err();
        assert!(err.to_string().contains("missing reply"), "{err}");
        assert!(err.to_string().contains("RFC 08 §2"), "{err}");
    }

    /// RFC 08 §2: a `{var}`-bearing procedure path carries the same
    /// key-population budget as a subject or media pattern — its expansions
    /// are real keys, and the budget review needs the declared bound.
    #[test]
    fn a_var_procedure_needs_a_cardinality() {
        let base = format!("{HEADER}[producer]\nname = \"t\"\n\n");
        let err = lint_one(&format!(
            "{base}[[procedure]]\npath = \"port/{{port}}/drain\"\nkind = \"write\"\nreply = \"Ack\"\nsince = \"1.0\"\ndescription = \"d\"\n"
        ))
        .unwrap_err();
        assert!(err.to_string().contains("cardinality"), "{err}");
        assert!(err.to_string().contains("RFC 08 §2"), "{err}");
        // Declared, it parses; literal-only paths still need none.
        lint_one(&format!(
            "{base}[[procedure]]\npath = \"port/{{port}}/drain\"\nkind = \"write\"\nreply = \"Ack\"\ncardinality = 64\nsince = \"1.0\"\ndescription = \"d\"\n"
        ))
        .unwrap();
        lint_one(&format!(
            "{base}[[procedure]]\npath = \"x/set\"\nkind = \"write\"\nreply = \"Ack\"\nsince = \"1.0\"\ndescription = \"d\"\n"
        ))
        .unwrap();
    }

    /// Issue #313: a count field has no negative value, and the refusal
    /// belongs *here*, where the TOML was authored.
    ///
    /// `ttl_s = -60` used to lint clean and reach the emitter as an `i64`,
    /// which interpolated it verbatim into an `Option<u64>` accessor: the
    /// consumer's build then broke on `Some(-60)` in `$OUT_DIR`, in code they
    /// did not write, with nothing naming the line responsible — the exact
    /// opposite of what this crate's doc promises.
    #[test]
    fn a_negative_count_is_refused_where_the_toml_was_authored() {
        let cases = [
            (
                "ttl_s",
                format!(
                    "{HEADER}[producer]\nname = \"t\"\n\n[[subject]]\npath = \"health\"\nclass = \"state\"\ntype = \"Health\"\nttl_s = -60\nsince = \"1.0\"\ndescription = \"d\"\n"
                ),
                "health",
            ),
            (
                "cardinality",
                format!(
                    "{HEADER}[producer]\nname = \"t\"\n\n[[subject]]\npath = \"disk/{{mount}}/used\"\nclass = \"telemetry\"\ntype = \"F64\"\ncardinality = -5\nsince = \"1.0\"\ndescription = \"d\"\n"
                ),
                "disk/{mount}/used",
            ),
            (
                "cardinality",
                format!(
                    "{HEADER}[producer]\nname = \"t\"\n\n[[procedure]]\npath = \"port/{{port}}/drain\"\nkind = \"write\"\nreply = \"Ack\"\ncardinality = -1\nsince = \"1.0\"\ndescription = \"d\"\n"
                ),
                "port/{port}/drain",
            ),
            (
                "cardinality",
                format!(
                    "{HEADER}[producer]\nname = \"t\"\n\n[[media]]\npath = \"{{cam}}/video/h264/{{tier}}\"\nencoding = \"video/h264\"\nattachment = \"FrameMeta\"\ncardinality = -12\nsince = \"1.0\"\ndescription = \"d\"\n"
                ),
                "{cam}/video/h264/{tier}",
            ),
        ];
        for (field, toml, path) in cases {
            let err = lint_one(&toml).unwrap_err().to_string();
            // The file, the entry's path, and the field — all three, so the
            // reader is pointed at a line and not at a generated accessor.
            assert!(err.contains("t.toml"), "{err}");
            assert!(err.contains(path), "{err}");
            assert!(err.contains(field), "{err}");
            assert!(err.contains("must not be negative"), "{err}");
        }
    }

    /// The other half of #313: whatever the lint lets through, no generated
    /// source can carry a negative literal into a `u64` accessor. A count
    /// reaches the emitter as a `u64`, so this is now a property of the type
    /// rather than of the emitter's care.
    #[test]
    fn no_generated_source_holds_a_negative_count() {
        let out = lint_one(&format!(
            "{HEADER}[producer]\nname = \"t\"\n\n[[subject]]\npath = \"health\"\nclass = \"state\"\ntype = \"Health\"\ncommon = \"health\"\nttl_s = 900\nsince = \"1.0\"\ndescription = \"d\"\n\n[[subject]]\npath = \"disk/{{mount}}/used\"\nclass = \"telemetry\"\ntype = \"F64\"\ncardinality = 64\nsince = \"1.0\"\ndescription = \"d\"\n\n[[procedure]]\npath = \"port/{{port}}/drain\"\nkind = \"write\"\nreply = \"Ack\"\ncardinality = 64\nsince = \"1.0\"\ndescription = \"d\"\n\n[[media]]\npath = \"{{cam}}/video/h264/{{tier}}\"\nencoding = \"video/h264\"\nattachment = \"FrameMeta\"\ncardinality = 12\nsince = \"1.0\"\ndescription = \"d\"\n"
        ))
        .unwrap();
        assert!(!out.contains("Some(-"), "{out}");
        assert!(out.contains("=> Some(900),"), "{out}");
        assert!(out.contains("=> Some(64),"), "{out}");
    }

    /// `[budget]` (RFC 08 §2, v1.32) is linted where the TOML was authored
    /// and then carried verbatim: a negative bound fails the way `ttl_s`
    /// does (#313), a nameless or twice-named table row fails naming the
    /// section, and a well-formed one generates — with no accessor, because
    /// nothing is emitted from it.
    #[test]
    fn a_negative_or_duplicate_budget_fails_the_lint() {
        let body = "[producer]\nname = \"t\"\n\n[[subject]]\npath = \"health\"\nclass = \"state\"\ntype = \"Health\"\ncommon = \"health\"\nttl_s = 900\nsince = \"1.0\"\ndescription = \"d\"\n";
        for (toml, wants) in [
            (
                format!("{HEADER}{body}\n[budget]\nrss_mb = -64\n"),
                vec!["[budget]", "rss_mb", "must not be negative"],
            ),
            (
                format!(
                    "{HEADER}{body}\n[budget]\nrss_mb = 64\n\n[[budget.tables]]\nname = \"flows\"\nmax_entries = -1\n"
                ),
                vec!["flows", "max_entries", "must not be negative"],
            ),
            (
                format!(
                    "{HEADER}{body}\n[budget]\n\n[[budget.tables]]\nname = \"flows\"\nmax_bytes = -1\n"
                ),
                vec!["flows", "max_bytes", "must not be negative"],
            ),
            (
                format!("{HEADER}{body}\n[budget]\n\n[[budget.tables]]\nmax_entries = 4\n"),
                vec!["[[budget.tables]] missing name", "RFC 08 §2"],
            ),
            (
                format!(
                    "{HEADER}{body}\n[budget]\n\n[[budget.tables]]\nname = \"flows\"\n\n[[budget.tables]]\nname = \"flows\"\n"
                ),
                vec!["\"flows\" declared twice", "RFC 08 §2"],
            ),
        ] {
            let err = lint_one(&toml).unwrap_err().to_string();
            assert!(err.contains("t.toml"), "{err}");
            for want in wants {
                assert!(err.contains(want), "{err:?} lacks {want:?}");
            }
        }

        let out = lint_one(&format!(
            "{HEADER}{body}\n[budget]\nrss_mb = 64\n\n[[budget.tables]]\nname = \"flows\"\nmax_entries = 65536\nmax_bytes = 16777216\n\n[[budget.tables]]\nname = \"names\"\n"
        ))
        .unwrap();
        assert!(
            !out.contains("rss_mb") && !out.contains("fn budget"),
            "nothing is generated from [budget]; it rides REGISTRY_TOML verbatim:\n{out}"
        );
    }

    #[test]
    fn service_host_var_must_lead() {
        let toml = format!(
            "{HEADER}[service]\nname = \"desired\"\norigin = \"@desired\"\n\n[[subject]]\npath = \"config/{{host}}/x\"\nclass = \"state\"\ntype = \"Doc\"\nttl_s = 60\ncardinality = 100\nsince = \"1.0\"\ndescription = \"d\"\n"
        );
        let err = lint_one(&toml).unwrap_err();
        assert!(err.to_string().contains("{host}"), "{err}");
        // Leading {host} is the legal spelling (G1/H4).
        let ok = format!(
            "{HEADER}[service]\nname = \"desired\"\norigin = \"@desired\"\n\n[[subject]]\npath = \"{{host}}/config/x\"\nclass = \"state\"\ntype = \"Doc\"\nttl_s = 60\ncardinality = 100\nsince = \"1.0\"\ndescription = \"d\"\n"
        );
        lint_one(&ok).unwrap();
    }

    /// Issue #312: wrapping a builder's own output is not public API. The
    /// constructors are `pub(crate)` in zenkey and reachable only through
    /// `zenkey::__private`, which generated code names explicitly — so the
    /// old `Key::from_canonical` / `Selector::from_canonical` /
    /// `Chunk::from_valid` spellings must not survive anywhere in the
    /// emitted module. This is the compile-time half of the fix; the
    /// wildcard refusal itself is pinned in zenkey's `key` tests.
    #[test]
    fn generated_code_names_the_private_wrapping_path() {
        let out = lint_one(&format!(
            "{HEADER}[service]\nname = \"desired\"\norigin = \"@desired\"\n\n[[subject]]\npath = \"{{host}}/config/x\"\nclass = \"state\"\ntype = \"Doc\"\nttl_s = 60\ncardinality = 100\nsince = \"1.0\"\ndescription = \"d\"\n\n[[procedure]]\npath = \"port/{{port}}/drain\"\nkind = \"write\"\nreply = \"Ack\"\ncardinality = 64\nsince = \"1.0\"\ndescription = \"d\"\n"
        ))
        .unwrap();
        assert!(
            out.contains("use ::zenkey::__private::{key_from_canonical, selector_from_canonical};"),
            "the generated preamble must name the private path"
        );
        for gone in [
            "Key::from_canonical",
            "Selector::from_canonical",
            "Chunk::from_valid",
        ] {
            assert!(!out.contains(gone), "generated code still calls {gone}");
        }
        // A `{host}` var takes a typed HostId, whose `h-<12hex>` shape is a
        // legal plain chunk by construction — converted, never slugged and
        // never waved through by a `debug_assert`.
        assert!(out.contains("host: Chunk::from(host)"), "{out}");
    }

    #[test]
    fn type_table_lint_activates_on_existence() {
        let dir = std::env::temp_dir().join(format!(
            "zenkey-build-types-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let reg = format!(
            "{HEADER}[producer]\nname = \"t\"\n\n[[subject]]\npath = \"health\"\nclass = \"state\"\ntype = \"HealthSnapshot\"\nttl_s = 60\nsince = \"1.0\"\ndescription = \"d\"\n"
        );
        std::fs::write(dir.join("t.toml"), &reg).unwrap();
        Config::new()
            .registry_dir(&dir)
            .write_compat_lock(OnIncompatible::Refuse)
            .unwrap();
        // No types.toml: lint inactive.
        Config::new().registry_dir(&dir).generate_string().unwrap();
        // types.toml missing the referenced name: build fails.
        std::fs::write(
            dir.join("types.toml"),
            "[types.Other]\nkind = \"json-schema\"\n",
        )
        .unwrap();
        let err = Config::new()
            .registry_dir(&dir)
            .generate_string()
            .unwrap_err();
        assert!(err.to_string().contains("HealthSnapshot"), "{err}");
        // Resolving table: build passes.
        std::fs::write(
            dir.join("types.toml"),
            "[types.HealthSnapshot]\nkind = \"json-schema\"\n",
        )
        .unwrap();
        Config::new().registry_dir(&dir).generate_string().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn media_needs_encoding_and_no_rest() {
        let base = format!("{HEADER}[producer]\nname = \"cam\"\n\n");
        let missing_encoding = format!(
            "{base}[[media]]\npath = \"front/video/h264/low\"\nattachment = \"FrameMeta\"\nsince = \"1.0\"\n"
        );
        assert!(lint_one(&missing_encoding).is_err());
        let rest = format!(
            "{base}[[media]]\npath = \"front/{{rest...}}\"\nencoding = \"video/h264\"\nattachment = \"FrameMeta\"\nsince = \"1.0\"\n"
        );
        assert!(lint_one(&rest).is_err());
    }

    /// RFC 08 §5 (v1.8). Every blob vocabulary is closed by RFC 07 §2, so
    /// every one of these is decidable at build time — which is the argument
    /// for modelling the plane rather than leaving it to prose.
    ///
    /// Each case asserts the *rejection*; the accepting cases at the end are
    /// what keep the rejections from passing for the wrong reason (a lint
    /// that rejects everything is not a lint).
    #[test]
    fn blob_vocabularies_are_closed() {
        let base = format!("{HEADER}[producer]\nname = \"netring\"\n\n");
        // Every body carries description/since so each case fails only for
        // its targeted reason, not for missing metadata.
        let blob = |body: &str| {
            lint_one(&format!(
                "{base}[[blob]]\n{body}since = \"1.8\"\ndescription = \"x\"\n"
            ))
        };

        // The tier token is one of three (RFC 07 §2).
        assert!(blob("tier = \"snapshot\"\n").is_err());
        assert!(blob("").is_err(), "tier is required");

        // Endpoints: required on artifact, forbidden elsewhere, closed set.
        assert!(
            blob("tier = \"artifact\"\n").is_err(),
            "artifact must declare its endpoints"
        );
        assert!(
            blob("tier = \"artifact\"\nendpoints = [\"manifest\", \"chunks\"]\n").is_err(),
            "\"chunks\" is not an RFC 07 §2.2 endpoint"
        );
        assert!(
            blob("tier = \"tree\"\nendpoints = [\"manifest\"]\n").is_err(),
            "a Tier-2 key IS the endpoint"
        );
        assert!(blob("tier = \"artifact\"\nendpoints = [\"have\", \"have\"]\n").is_err());

        // algo: required on store, forbidden elsewhere.
        assert!(blob("tier = \"store\"\n").is_err());
        assert!(blob("tier = \"tree\"\nalgo = \"blake3\"\n").is_err());

        // The two absences that are load-bearing rather than lenient: a blob
        // entry has no path (the key shapes are fixed by the chapter) and no
        // cardinality (RFC 03 §3 carves content addresses out of the budget).
        assert!(blob("tier = \"tree\"\npath = \"{root}\"\n").is_err());
        assert!(blob("tier = \"tree\"\ncardinality = 1000\n").is_err());

        // description/since are required (RFC 08 §2) — per-declaration prose,
        // but it must exist.
        let bare = |body: &str| lint_one(&format!("{base}[[blob]]\n{body}"));
        assert!(
            bare("tier = \"tree\"\nsince = \"1.8\"\n").is_err(),
            "description is required"
        );
        assert!(
            bare("tier = \"tree\"\ndescription = \"snapshots\"\n").is_err(),
            "since is required"
        );

        // …and the shapes that must be accepted.
        assert!(
            blob("tier = \"tree\"\n").is_ok(),
            "a bare Tier-2 declaration is the minimal legal entry"
        );
        assert!(
            blob(
                "tier = \"artifact\"\nendpoints = [\"manifest\", \"slice\", \"have\", \"push\", \
                 \"fanout\"]\nreference = \"Delivery\"\nencoding = \"application/gzip\"\n"
            )
            .is_ok(),
            "the full artifact declaration must build"
        );
    }

    /// Build a registry from (file name, producer name, [[blob]] bodies) and
    /// return the generated source or the lint error.
    fn blob_registry(tag: &str, files: &[(&str, &str, &[&str])]) -> Result<String, Error> {
        let dir = std::env::temp_dir().join(format!(
            "zenkey-build-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (fname, producer, entries) in files {
            let mut src = format!("{HEADER}[producer]\nname = {producer:?}\n");
            for body in *entries {
                src.push_str(&format!("\n[[blob]]\n{body}"));
            }
            std::fs::write(dir.join(fname), src).unwrap();
        }
        let out = Config::new().registry_dir(&dir).generate_string();
        let _ = std::fs::remove_dir_all(&dir);
        out
    }

    /// A blob key carries no producer chunk, so every declaration of one tier
    /// names the *same* app-level key family. Several producers may each
    /// declare a tier — the introspect slice is per-producer truth — but the
    /// declarations must agree on the family's shape, and codegen dedups them
    /// into one surface that records every declarer.
    #[test]
    fn blob_tier_shapes_must_agree_across_the_app() {
        let tree = "tier = \"tree\"\nsince = \"1.8\"\ndescription = \"snapshots\"\n";

        // Same shape in two files: builds, one variant, every declarer named.
        let out = blob_registry(
            "blobagree",
            &[("a.toml", "netring", &[tree]), ("b.toml", "logs", &[tree])],
        )
        .unwrap();
        assert_eq!(
            out.matches("        Tree,").count(),
            1,
            "shape-identical declarations dedup to one variant"
        );
        assert_eq!(out.matches("pub fn tree_key").count(), 1);
        assert!(
            out.contains("&[\"netring\", \"logs\"]"),
            "declared_by lists every declarer in file order"
        );

        // Prose may differ per declaration — each declarer says why *it*
        // serves the tier.
        let other_prose = "tier = \"tree\"\nsince = \"1.9\"\ndescription = \"log bundles\"\n";
        assert!(
            blob_registry(
                "blobprose",
                &[
                    ("a.toml", "netring", &[tree]),
                    ("b.toml", "logs", &[other_prose]),
                ],
            )
            .is_ok()
        );

        // Shape fields may not: the family has one shape.
        let with_ref = "tier = \"tree\"\nreference = \"Delivery\"\nsince = \"1.8\"\n\
                        description = \"snapshots\"\n";
        let err = blob_registry(
            "blobref",
            &[
                ("a.toml", "netring", &[tree]),
                ("b.toml", "logs", &[with_ref]),
            ],
        )
        .unwrap_err();
        assert!(err.to_string().contains("reference"), "{err}");

        let with_enc = "tier = \"tree\"\nencoding = \"application/zstd\"\nsince = \"1.8\"\n\
                        description = \"snapshots\"\n";
        let err = blob_registry(
            "blobenc",
            &[
                ("a.toml", "netring", &[tree]),
                ("b.toml", "logs", &[with_enc]),
            ],
        )
        .unwrap_err();
        assert!(err.to_string().contains("encoding"), "{err}");
    }

    /// RFC 08 §2/§5 (v1.17): a second concurrent store algo is the
    /// sanctioned dual-algo migration window of RFC 07 §2.4 — codegen emits
    /// one builder and one address type per declared algo, so both address
    /// spaces are spellable, and unmixable by type.
    #[test]
    fn two_store_algos_emit_per_algo_builders() {
        let blake3 = "tier = \"store\"\nalgo = \"blake3\"\nsince = \"1.8\"\n\
                      description = \"chunks\"\n";
        let sha256 = "tier = \"store\"\nalgo = \"sha256\"\nsince = \"1.8\"\n\
                      description = \"chunks\"\n";
        let out = blob_registry(
            "blobalgo",
            &[
                ("a.toml", "netring", &[blake3]),
                ("b.toml", "logs", &[sha256]),
            ],
        )
        .unwrap();
        assert_eq!(
            out.matches("        Store,").count(),
            1,
            "one tier variant however many algos — the algo is a key chunk, \
             not a tier"
        );
        for needle in [
            "pub struct Blake3Addr(ContentHash)",
            "pub struct Sha256Addr(ContentHash)",
            "pub fn store_key_blake3(o: &impl HostOrigin, addr: &Blake3Addr)",
            "pub fn store_key_sha256(o: &impl HostOrigin, addr: &Sha256Addr)",
            "pub fn store_have_probe_blake3()",
            "pub fn store_have_probe_sha256()",
        ] {
            assert!(out.contains(needle), "missing {needle:?} in:\n{out}");
        }
        assert!(
            out.contains("&[\"blake3\", \"sha256\"]"),
            "algos() lists both address spaces in registry order"
        );

        // One algo everywhere is the steady state: one builder, one type.
        let out = blob_registry(
            "blobalgook",
            &[
                ("a.toml", "netring", &[blake3]),
                ("b.toml", "logs", &[blake3]),
            ],
        )
        .unwrap();
        assert!(out.contains("pub fn store_key_blake3"));
        assert!(!out.contains("sha256"));
    }

    /// Cross-file repetition is a producer declaring what it serves; the same
    /// tier twice in *one* file is a copy-paste error.
    #[test]
    fn a_blob_tier_declared_twice_in_one_file_is_rejected() {
        let tree = "tier = \"tree\"\nsince = \"1.8\"\ndescription = \"snapshots\"\n";
        let err =
            blob_registry("blobdupfile", &[("a.toml", "netring", &[tree, tree])]).unwrap_err();
        assert!(err.to_string().contains("twice in one file"), "{err}");
    }

    /// H3 (#78): a fresh directory in a named temp home, for the lock tests
    /// — each builds its own registry and drives the lock explicitly.
    fn lock_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "zenkey-build-lock-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const SUBJECT_V1: &str = "[[subject]]\npath = \"health\"\nclass = \"state\"\ntype = \"Health\"\ncommon = \"health\"\nttl_s = 900\nsince = \"1.0\"\ndescription = \"d\"\n";

    /// The acceptance case verbatim: changing a subject's `type` without
    /// touching the lock fails the build, citing the RFC subsection.
    #[test]
    fn a_changed_type_without_lock_churn_fails() {
        let dir = lock_dir("changed-type");
        let base = format!("{HEADER}[producer]\nname = \"t\"\n\n{SUBJECT_V1}");
        std::fs::write(dir.join("t.toml"), &base).unwrap();
        Config::new()
            .registry_dir(&dir)
            .write_compat_lock(OnIncompatible::Refuse)
            .unwrap();
        assert!(
            Config::new()
                .registry_dir(&dir)
                .no_rerun_if_changed()
                .lint()
                .is_ok()
        );

        std::fs::write(
            dir.join("t.toml"),
            base.replace("type = \"Health\"", "type = \"Health2\""),
        )
        .unwrap();
        let err = Config::new()
            .registry_dir(&dir)
            .no_rerun_if_changed()
            .lint()
            .unwrap_err()
            .to_string();
        assert!(err.contains("RFC 08 §3.1"), "{err}");
        assert!(err.contains("incompatible"), "{err}");
        // …and the regeneration tool refuses the same edit without force.
        let err = Config::new()
            .registry_dir(&dir)
            .write_compat_lock(OnIncompatible::Refuse)
            .unwrap_err()
            .to_string();
        assert!(err.contains("incompatible"), "{err}");
        // Forced, it writes — and reports the break, loudly.
        let update = Config::new()
            .registry_dir(&dir)
            .write_compat_lock(OnIncompatible::ForceAndReport)
            .unwrap();
        assert!(!update.forced.is_empty(), "a forced break is never silent");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC 08 §3.1 (v1.32): `kind` is an optional trailing column of a
    /// subject line. Declaring one on a pinned path is additive — stale,
    /// regenerate — and the regeneration counts it as neither added nor
    /// retired.
    #[test]
    fn adding_kind_to_a_pinned_subject_is_stale_not_incompatible() {
        let dir = lock_dir("kind-added");
        let counter = "[[subject]]\npath = \"rx/bytes_total\"\nclass = \"telemetry\"\ntype = \"TelemetryPoint\"\nunit = \"bytes\"\nsince = \"1.0\"\ndescription = \"d\"\n";
        let base = format!("{HEADER}[producer]\nname = \"t\"\n\n{counter}");
        std::fs::write(dir.join("t.toml"), &base).unwrap();
        Config::new()
            .registry_dir(&dir)
            .write_compat_lock(OnIncompatible::Refuse)
            .unwrap();
        let pinned = std::fs::read_to_string(dir.join("registry.lock")).unwrap();
        assert!(
            pinned.contains("subject\tt\trx/bytes_total\ttelemetry\tTelemetryPoint\n"),
            "a subject without a kind pins the five-column line it always did:\n{pinned}"
        );

        std::fs::write(
            dir.join("t.toml"),
            base.replace("unit = \"bytes\"", "unit = \"bytes\"\nkind = \"counter\""),
        )
        .unwrap();
        let err = Config::new()
            .registry_dir(&dir)
            .no_rerun_if_changed()
            .lint()
            .unwrap_err();
        assert!(
            matches!(
                err,
                Error::Lint {
                    kind: LintKind::Stale,
                    ..
                }
            ),
            "adding a kind is stale, never incompatible: {err}"
        );
        assert!(err.to_string().contains("zenctl registry lock"), "{err}");
        let update = Config::new()
            .registry_dir(&dir)
            .write_compat_lock(OnIncompatible::Refuse)
            .unwrap();
        assert_eq!(
            (update.added, update.retired),
            (0, 0),
            "the pin is the same pin"
        );
        assert!(update.forced.is_empty());
        let pinned = std::fs::read_to_string(dir.join("registry.lock")).unwrap();
        assert!(
            pinned.contains("subject\tt\trx/bytes_total\ttelemetry\tTelemetryPoint\tcounter\n"),
            "{pinned}"
        );
        assert!(
            Config::new()
                .registry_dir(&dir)
                .no_rerun_if_changed()
                .lint()
                .is_ok()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC 08 §3.1 (v1.32): a pinned `kind` never changes in place, and
    /// never disappears — a consumer that learned to `rate()` a counter is
    /// wrong the moment the same path is a gauge. Both fail as incompatible,
    /// naming the column.
    #[test]
    fn changing_or_removing_a_pinned_kind_is_incompatible() {
        let dir = lock_dir("kind-changed");
        let counter = "[[subject]]\npath = \"rx/bytes_total\"\nclass = \"telemetry\"\ntype = \"TelemetryPoint\"\nunit = \"bytes\"\nkind = \"counter\"\nsince = \"1.0\"\ndescription = \"d\"\n";
        let base = format!("{HEADER}[producer]\nname = \"t\"\n\n{counter}");
        std::fs::write(dir.join("t.toml"), &base).unwrap();
        Config::new()
            .registry_dir(&dir)
            .write_compat_lock(OnIncompatible::Refuse)
            .unwrap();

        for (edit, expect) in [("kind = \"gauge\"", "counter → gauge"), ("", "counter → -")] {
            std::fs::write(dir.join("t.toml"), base.replace("kind = \"counter\"", edit)).unwrap();
            let err = Config::new()
                .registry_dir(&dir)
                .no_rerun_if_changed()
                .lint()
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    Error::Lint {
                        kind: LintKind::Incompatible,
                        ..
                    }
                ),
                "{edit:?}: {err}"
            );
            let msg = err.to_string();
            assert!(msg.contains("`kind` column changed"), "{msg}");
            assert!(msg.contains(expect), "{msg}");
            assert!(msg.contains("RFC 08 §3.1"), "{msg}");
            // The regeneration tool refuses the same edit without force…
            let err = Config::new()
                .registry_dir(&dir)
                .write_compat_lock(OnIncompatible::Refuse)
                .unwrap_err()
                .to_string();
            assert!(err.contains("`kind` column changed"), "{err}");
        }
        // …and forced, it reports the break.
        let update = Config::new()
            .registry_dir(&dir)
            .write_compat_lock(OnIncompatible::ForceAndReport)
            .unwrap();
        assert!(!update.forced.is_empty(), "a forced break is never silent");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The `kind` vocabulary is closed (RFC 08 §2): a token outside it is a
    /// lint naming the four, not an unchecked subject.
    #[test]
    fn an_unknown_kind_token_is_a_lint() {
        let dir = lock_dir("kind-unknown");
        let bad = "[[subject]]\npath = \"rx/bytes_total\"\nclass = \"telemetry\"\ntype = \"TelemetryPoint\"\nkind = \"histogram\"\nsince = \"1.0\"\ndescription = \"d\"\n";
        std::fs::write(
            dir.join("t.toml"),
            format!("{HEADER}[producer]\nname = \"t\"\n\n{bad}"),
        )
        .unwrap();
        let err = Config::new()
            .registry_dir(&dir)
            .no_rerun_if_changed()
            .lint()
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown kind \"histogram\""), "{err}");
        for token in ["`counter`", "`gauge`", "`text`", "`bool`"] {
            assert!(err.contains(token), "{err}");
        }
        assert!(err.contains("RFC 08 §2"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Additive evolution is free: a new subject passes after regeneration,
    /// and a stale lock says "regenerate", never "incompatible".
    #[test]
    fn additive_changes_pass_with_regeneration_only() {
        let dir = lock_dir("additive");
        let base = format!("{HEADER}[producer]\nname = \"t\"\n\n{SUBJECT_V1}");
        std::fs::write(dir.join("t.toml"), &base).unwrap();
        Config::new()
            .registry_dir(&dir)
            .write_compat_lock(OnIncompatible::Refuse)
            .unwrap();

        let added = format!(
            "{base}\n[[subject]]\npath = \"errors\"\nclass = \"state\"\ntype = \"Errors\"\ncommon = \"errors\"\nttl_s = 900\nsince = \"1.1\"\ndescription = \"d\"\n"
        );
        std::fs::write(dir.join("t.toml"), &added).unwrap();
        let err = Config::new()
            .registry_dir(&dir)
            .no_rerun_if_changed()
            .lint()
            .unwrap_err()
            .to_string();
        assert!(err.contains("stale lock"), "{err}");
        assert!(!err.contains("incompatible"), "{err}");

        let update = Config::new()
            .registry_dir(&dir)
            .write_compat_lock(OnIncompatible::Refuse)
            .unwrap();
        assert_eq!(update.added, 1);
        assert!(update.forced.is_empty());
        assert!(
            Config::new()
                .registry_dir(&dir)
                .no_rerun_if_changed()
                .lint()
                .is_ok()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    const PROCEDURE_V1: &str = "[[procedure]]\npath = \"rotate\"\nkind = \"write\"\nrequest = \"RotateReq\"\nreply = \"Ack\"\nsince = \"1.0\"\ndescription = \"d\"\n";

    /// The gap #377 names: a procedure had **no** sanctioned exit. Removing
    /// one from a `compat = "backward"` file failed the lock, and the
    /// `[[deprecated]]` entry that is supposed to be the exit was not
    /// consulted for it.
    #[test]
    fn a_procedure_retires_through_the_same_ledger_a_subject_does() {
        let dir = lock_dir("retire-procedure");
        let base = format!("{HEADER}[producer]\nname = \"t\"\n\n{SUBJECT_V1}\n{PROCEDURE_V1}");
        std::fs::write(dir.join("t.toml"), &base).unwrap();
        Config::new()
            .registry_dir(&dir)
            .write_compat_lock(OnIncompatible::Refuse)
            .unwrap();

        let retired = format!(
            "{HEADER}[producer]\nname = \"t\"\n\n{SUBJECT_V1}\n[[deprecated]]\nkind = \"procedure\"\npath = \"rotate\"\ngone = \"1.1\"\n"
        );
        std::fs::write(dir.join("t.toml"), &retired).unwrap();
        std::fs::write(dir.join("deprecated.lock"), "procedure\tt\trotate\n").unwrap();
        let update = Config::new()
            .registry_dir(&dir)
            .write_compat_lock(OnIncompatible::Refuse)
            .unwrap();
        assert_eq!(update.retired, 1);
        assert!(update.forced.is_empty(), "retirement is not a break");
        assert!(
            Config::new()
                .registry_dir(&dir)
                .no_rerun_if_changed()
                .lint()
                .is_ok()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Kind is part of the identity: a `[[deprecated]]` subject says nothing
    /// about the procedure of the same name, whose pin stays.
    #[test]
    fn retiring_a_subject_does_not_release_a_procedure_of_the_same_name() {
        let dir = lock_dir("retire-wrong-kind");
        let dual_subject = "[[subject]]\npath = \"dual\"\nclass = \"state\"\ntype = \"D\"\nttl_s = 900\nsince = \"1.0\"\ndescription = \"d\"\n";
        let dual_procedure = "[[procedure]]\npath = \"dual\"\nkind = \"read\"\nrequest = \"Q\"\nreply = \"A\"\nsince = \"1.0\"\ndescription = \"d\"\n";
        let base = format!("{HEADER}[producer]\nname = \"t\"\n\n{dual_subject}\n{dual_procedure}");
        std::fs::write(dir.join("t.toml"), &base).unwrap();
        Config::new()
            .registry_dir(&dir)
            .write_compat_lock(OnIncompatible::Refuse)
            .unwrap();

        // Both leave the file; only the subject is retired.
        let wrong = format!(
            "{HEADER}[producer]\nname = \"t\"\n\n[[deprecated]]\npath = \"dual\"\ngone = \"1.1\"\n"
        );
        std::fs::write(dir.join("t.toml"), &wrong).unwrap();
        std::fs::write(dir.join("deprecated.lock"), "t\tdual\n").unwrap();
        let err = Config::new()
            .registry_dir(&dir)
            .no_rerun_if_changed()
            .lint()
            .expect_err("the procedure vanished with no retirement of its own");
        let err = err.to_string();
        assert!(err.contains("vanished without retirement"), "{err}");
        assert!(err.contains("procedure"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The two-field ledger line is the pre-v1.26 spelling and still means
    /// `subject` — no existing ledger is asked to change.
    #[test]
    fn the_old_two_field_ledger_line_still_reads_as_a_subject() {
        assert_eq!(
            parse_ledger_line("t\thealth").unwrap(),
            (EntryKind::Subject, "t", "health")
        );
        assert_eq!(
            parse_ledger_line("procedure\tt\trotate").unwrap(),
            (EntryKind::Procedure, "t", "rotate")
        );
        let err = parse_ledger_line("nonsense\tt\trotate").unwrap_err();
        assert!(err.to_string().contains("`procedure`"), "{err}");
    }

    /// Retirement through `[[deprecated]]` is the sanctioned exit: the pin is
    /// released on regeneration and nothing calls it incompatible.
    #[test]
    fn deprecation_releases_the_pin() {
        let dir = lock_dir("retire");
        let base = format!("{HEADER}[producer]\nname = \"t\"\n\n{SUBJECT_V1}");
        std::fs::write(dir.join("t.toml"), &base).unwrap();
        Config::new()
            .registry_dir(&dir)
            .write_compat_lock(OnIncompatible::Refuse)
            .unwrap();

        let retired = format!(
            "{HEADER}[producer]\nname = \"t\"\n\n[[deprecated]]\npath = \"health\"\ngone = \"1.1\"\n"
        );
        std::fs::write(dir.join("t.toml"), &retired).unwrap();
        std::fs::write(dir.join("deprecated.lock"), "t\thealth\n").unwrap();
        let update = Config::new()
            .registry_dir(&dir)
            .write_compat_lock(OnIncompatible::Refuse)
            .unwrap();
        assert_eq!(update.retired, 1);
        assert!(update.forced.is_empty(), "retirement is not a break");
        assert!(
            Config::new()
                .registry_dir(&dir)
                .no_rerun_if_changed()
                .lint()
                .is_ok()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Forceability is a property of the failure, not of its prose (#318).
    ///
    /// The old classifier was `msg.contains("incompatible registry edit") ||
    /// msg.contains("vanished")` over two multi-line paragraphs that read
    /// like something an editor would tidy. This asserts the four kinds
    /// directly, so rewording a message cannot move a failure between
    /// "forceable" and "refused" without this test failing first.
    #[test]
    fn forceability_is_the_kind_not_the_wording() {
        assert!(LintKind::Incompatible.is_forceable());
        assert!(LintKind::Vanished.is_forceable());
        // Stale is not *forced* past — it is what the write resolves.
        assert!(!LintKind::Stale.is_forceable());
        assert!(LintKind::Stale.is_resolved_by_writing());
        // And the two that a rewrite must never paper over.
        for k in [LintKind::BadLine, LintKind::Invalid] {
            assert!(!k.is_forceable(), "{k:?}");
            assert!(!k.is_resolved_by_writing(), "{k:?}");
        }
    }

    /// A lock file that does not parse is reported, not overwritten.
    ///
    /// This was the bug the string match hid: `bad lock line` matched neither
    /// sentinel, so `incompatible` was false, *neither* arm of the old `if`
    /// fired, the error was dropped, and the corrupt lock was silently
    /// replaced. A reset is not a repair (#318).
    #[test]
    fn a_corrupt_lock_is_reported_rather_than_reset() {
        let dir = lock_dir("corrupt");
        let base = format!("{HEADER}[producer]\nname = \"t\"\n\n{SUBJECT_V1}");
        std::fs::write(dir.join("t.toml"), &base).unwrap();
        std::fs::write(dir.join("registry.lock"), "this is not a lock line\n").unwrap();

        let err = Config::new()
            .registry_dir(&dir)
            .write_compat_lock(OnIncompatible::Refuse)
            .unwrap_err();
        assert!(matches!(
            err,
            Error::Lint {
                kind: LintKind::BadLine,
                ..
            }
        ));
        // Not forceable either: forcing is for broken pins, not for a file
        // this build could not read in the first place.
        assert!(
            Config::new()
                .registry_dir(&dir)
                .write_compat_lock(OnIncompatible::ForceAndReport)
                .is_err()
        );
        // And the file the caller could not read is still there to look at.
        assert_eq!(
            std::fs::read_to_string(dir.join("registry.lock")).unwrap(),
            "this is not a lock line\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A draft (RFC 08 §6.1, v1.34) is refused by default with its own
    /// kind, which nothing can force and no lock write resolves: the marker
    /// is the mechanism, so it must not be a warning a build prints and
    /// proceeds past.
    #[test]
    fn a_draft_is_refused_unless_admitted() {
        let dir = lock_dir("draft");
        let draft_header = "[registry]\nversion = \"0.1\"\napp = \"t\"\nconvention = 1\ncompat = \"none\"\ndraft = true\n";
        // No `since` — a draft has no version stream.
        let subject = "[[subject]]\npath = \"health\"\nclass = \"state\"\ntype = \"Health\"\nttl_s = 900\ndescription = \"inferred; unreviewed\"\n";
        std::fs::write(
            dir.join("t.toml"),
            format!("{draft_header}[producer]\nname = \"t\"\n\n{subject}"),
        )
        .unwrap();
        let err = Config::new()
            .registry_dir(&dir)
            .no_rerun_if_changed()
            .lint()
            .unwrap_err();
        match &err {
            Error::Lint { kind, message, .. } => {
                assert_eq!(*kind, LintKind::Draft);
                assert!(message.contains("draft = true"), "{message}");
                assert!(message.contains("allow_drafts"), "{message}");
            }
            other => panic!("expected a draft lint, got {other:?}"),
        }
        assert!(!LintKind::Draft.is_forceable());
        assert!(!LintKind::Draft.is_resolved_by_writing());
        // A lint failure is an answer, not a refusal to ask (#348).
        assert!(!err.is_unaskable());

        // Admitted: the missing `since` is waived, and the draft is a
        // warning every build sees.
        let warnings = Config::new()
            .registry_dir(&dir)
            .no_rerun_if_changed()
            .allow_drafts(true)
            .lint()
            .expect("an admitted draft lints");
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].file, "t");
        assert!(warnings[0].message.starts_with("draft"), "{warnings:?}");
        // …and it generates, so a consumer can prototype against it.
        let (code, _) = Config::new()
            .registry_dir(&dir)
            .no_rerun_if_changed()
            .allow_drafts(true)
            .generate_string_checked()
            .expect("an admitted draft generates");
        assert!(code.contains("Health"), "the draft's subject is generated");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A draft cannot be pinned, and cannot claim a lifecycle: `draft = true`
    /// with `compat = "backward"` or with a `since` is malformed
    /// (`Invalid`), admitted or not.
    #[test]
    fn a_draft_cannot_be_pinned_or_carry_since() {
        let pinned = lint_one(
            "[registry]\nversion = \"0.1\"\napp = \"t\"\nconvention = 1\ndraft = true\n[producer]\nname = \"t\"\n",
        )
        .unwrap_err();
        match pinned {
            Error::Lint { kind, message, .. } => {
                assert_eq!(kind, LintKind::Invalid);
                assert!(message.contains("cannot be pinned"), "{message}");
            }
            other => panic!("{other:?}"),
        }
        let with_since = lint_one(
            "[registry]\nversion = \"0.1\"\napp = \"t\"\nconvention = 1\ncompat = \"none\"\ndraft = true\n[producer]\nname = \"t\"\n\n[[subject]]\npath = \"health\"\nclass = \"state\"\ntype = \"Health\"\nttl_s = 900\nsince = \"1.0\"\ndescription = \"d\"\n",
        )
        .unwrap_err();
        match with_since {
            Error::Lint { kind, message, .. } => {
                assert_eq!(kind, LintKind::Invalid);
                assert!(message.contains("since"), "{message}");
            }
            other => panic!("{other:?}"),
        }
        let not_bool = lint_one(
            "[registry]\nversion = \"0.1\"\napp = \"t\"\nconvention = 1\ncompat = \"none\"\ndraft = \"yes\"\n[producer]\nname = \"t\"\n",
        )
        .unwrap_err()
        .to_string();
        assert!(not_bool.contains("boolean"), "{not_bool}");
    }

    /// `compat = "none"` unpins the file — the loud escape hatch: the same
    /// type change that fails under backward passes, and contributes no
    /// lock lines at all.
    #[test]
    fn compat_none_opts_out() {
        let dir = lock_dir("none");
        let header_none =
            "[registry]\nversion = \"1.0\"\napp = \"t\"\nconvention = 1\ncompat = \"none\"\n";
        let base = format!("{header_none}[producer]\nname = \"t\"\n\n{SUBJECT_V1}");
        std::fs::write(dir.join("t.toml"), &base).unwrap();
        Config::new()
            .registry_dir(&dir)
            .write_compat_lock(OnIncompatible::Refuse)
            .unwrap();
        std::fs::write(
            dir.join("t.toml"),
            base.replace("type = \"Health\"", "type = \"Health2\""),
        )
        .unwrap();
        // …and the opt-out is *reported*, through both doors, whether or not
        // cargo directives are being emitted (#319). `no_rerun_if_changed`
        // used to suppress this warning along with the directives, which is
        // exactly the door `zenctl registry lint` goes through.
        let quiet = Config::new()
            .registry_dir(&dir)
            .no_rerun_if_changed()
            .lint()
            .expect("compat = \"none\" is legal");
        assert_eq!(quiet.len(), 1, "{quiet:?}");
        assert_eq!(quiet[0].file, "t");
        assert!(quiet[0].message.contains("compat = \"none\""), "{quiet:?}");

        let loud = Config::new()
            .registry_dir(&dir)
            .lint()
            .expect("compat = \"none\" is legal");
        assert_eq!(loud, quiet, "the flag governs cargo directives, not checks");

        let err = lint_one(
            "[registry]\nversion = \"1.0\"\napp = \"t\"\nconvention = 1\ncompat = \"sometimes\"\n[producer]\nname = \"t\"\n",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("compat"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A one-producer registry dir with a `conditional.lock`, linted — the
    /// RFC 08 §6.1 (v1.25) harness, mirroring `lint_one`.
    fn lint_conditional(ledger: &str) -> Result<Vec<ConditionalSubject>, Error> {
        let dir = lock_dir("conditional");
        let toml = format!("{HEADER}[producer]\nname = \"t\"\n\n{SUBJECT_V1}");
        std::fs::write(dir.join("t.toml"), toml).unwrap();
        std::fs::write(dir.join("conditional.lock"), ledger).unwrap();
        Config::new()
            .registry_dir(&dir)
            .write_compat_lock(OnIncompatible::Refuse)
            .unwrap();
        let lint = Config::new()
            .registry_dir(&dir)
            .no_rerun_if_changed()
            .lint();
        let entries = lint.and_then(|_| {
            Config::new()
                .registry_dir(&dir)
                .no_rerun_if_changed()
                .conditional_subjects()
        });
        let _ = std::fs::remove_dir_all(&dir);
        entries
    }

    /// An absent ledger is an empty ledger: no subject is conditional
    /// (RFC 08 §6.1 — the file is optional, its absence is a statement).
    #[test]
    fn conditional_ledger_absent_is_empty() {
        let dir = lock_dir("conditional-absent");
        let toml = format!("{HEADER}[producer]\nname = \"t\"\n\n{SUBJECT_V1}");
        std::fs::write(dir.join("t.toml"), toml).unwrap();
        Config::new()
            .registry_dir(&dir)
            .write_compat_lock(OnIncompatible::Refuse)
            .unwrap();
        assert!(
            Config::new()
                .registry_dir(&dir)
                .no_rerun_if_changed()
                .lint()
                .is_ok()
        );
        assert_eq!(
            Config::new()
                .registry_dir(&dir)
                .no_rerun_if_changed()
                .conditional_subjects()
                .unwrap(),
            []
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The passing form: `#` comments, blank lines, one tab-separated line
    /// per conditional subject, condition as free text (tabs included — the
    /// third field runs to end of line).
    #[test]
    fn conditional_ledger_parses_and_conditions_a_live_entry() {
        let entries = lint_conditional(
            "# gated surfaces (RFC 08 §6.1)\n\nt\thealth\tfeature wireguard\tand a tab\n",
        )
        .unwrap();
        assert_eq!(
            entries,
            [ConditionalSubject {
                producer: "t".into(),
                path: "health".into(),
                condition: "feature wireguard\tand a tab".into(),
            }]
        );
    }

    /// Direction B of the two-direction check: a ledger line naming no
    /// registry subject fails the build with the line quoted — the entry was
    /// retired or renamed, and the line must follow it or leave.
    #[test]
    fn conditional_ledger_line_naming_no_subject_fails() {
        let err = lint_conditional("t\tvanished\tfeature ebpf\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("conditional.lock"), "{err}");
        assert!(err.contains("t\\tvanished\\tfeature ebpf"), "{err}");
        assert!(err.contains("names no registry subject"), "{err}");

        // A wrong producer fails the same way.
        let err = lint_conditional("u\thealth\tfeature ebpf\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("names no registry subject"), "{err}");
    }

    /// A malformed line (fewer than three tab-separated fields) and an empty
    /// condition are both refused — a gate with no condition is the decay the
    /// RFC's two-direction rule exists to prevent.
    #[test]
    fn conditional_ledger_refuses_malformed_lines() {
        let err = lint_conditional("t\thealth\n").unwrap_err().to_string();
        assert!(err.contains("bad ledger line"), "{err}");

        let err = lint_conditional("t\thealth\t \n").unwrap_err().to_string();
        assert!(err.contains("empty condition"), "{err}");
    }

    /// A subject that left the registry through `[[deprecated]]` does not
    /// keep its conditional line: the ledger is not append-only, and the
    /// error says the entry is retired so the fix is obvious.
    #[test]
    fn conditional_ledger_line_for_retired_subject_fails_naming_retirement() {
        let dir = lock_dir("conditional-retired");
        let toml = format!(
            "{HEADER}[producer]\nname = \"t\"\n\n{SUBJECT_V1}\n[[deprecated]]\npath = \"old\"\ngone = \"1.1\"\n"
        );
        std::fs::write(dir.join("t.toml"), toml).unwrap();
        std::fs::write(dir.join("deprecated.lock"), "t\told\n").unwrap();
        std::fs::write(dir.join("conditional.lock"), "t\told\tfeature ebpf\n").unwrap();
        Config::new()
            .registry_dir(&dir)
            .write_compat_lock(OnIncompatible::Refuse)
            .unwrap();
        let err = Config::new()
            .registry_dir(&dir)
            .no_rerun_if_changed()
            .lint()
            .unwrap_err()
            .to_string();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(err.contains("retired through [[deprecated]]"), "{err}");
    }
}
