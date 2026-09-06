//! `RegistrySlice` — the reply type of the `introspect` procedure (RFC 08 §6).
//!
//! Every producer's registry file has declared `reply = "RegistrySlice"` since
//! the convention was ratified, and every sensor has answered `introspect` with
//! the raw registry TOML — but the type it named did not exist, so no consumer
//! could read the answer. This is that type.
//!
//! A slice is what one build *says* it serves. The point of having it is the
//! diff: compare a host's served slice against the slice this build compiled
//! in (the application's `zenkey-build`-generated `REGISTRIES` table) and a
//! disagreement is a
//! **finding** — a version skew, a subject the fleet serves that we cannot
//! name, or a subject we expect that nothing out there publishes. RFC 08 §6 is
//! explicit that this is a finding and not an ambiguity, which is why the
//! parser below is strict about the header and forgiving about nothing.

use std::fmt;

use crate::common_state::CommonFamily;
use crate::encoding::WireEncoding;
use crate::grammar::{BlobTier, Class};
use crate::origin::ServiceOrigin;
use crate::qos::QosProfile;

// ─── the closed vocabularies, and the tolerance around them ─────────────────
//
// Six columns of a slice draw on vocabularies this crate already defines as
// closed types — `class`, `qos`, `common`, `tier`, `kind`, `rate`, `fanout`
// and the service origin. Carrying them as `String` meant the accessors took
// `&str`, so `serves_blob_tier("artefact")` compiled and answered `false`:
// a typo and an honest absence were the same answer.
//
// The forward-compat posture that argued for `String` is not lost by typing
// them, because it was never an argument for `String` in the first place —
// [`WireEncoding`](crate::schema::WireEncoding) has demonstrated the shape
// since v1.5: known variants plus an `Other` arm, tolerant of a newer fleet
// and typed for everything this build knows. [`Declared`] is that shape,
// generic, so each vocabulary keeps its own enum and gains the tolerance
// once rather than seven times.

/// A closed registry vocabulary, as a slice column spells it.
///
/// Implemented by the token enums a slice carries. The two directions are
/// deliberately not symmetric: [`from_token`](SliceToken::from_token) is
/// fallible because a *foreign* slice may use a token this build has never
/// heard of, and [`token`](SliceToken::token) is not because a value that
/// exists was either recognised or carried verbatim.
pub trait SliceToken: Sized {
    /// The token as the registry TOML spells it, or `None` if this build
    /// does not know it.
    fn from_token(token: &str) -> Option<Self>;

    /// The canonical spelling — for a value parsed from a slice, byte-equal
    /// to what the slice carried.
    fn token(&self) -> &str;
}

/// One closed-vocabulary column, read from a possibly-foreign slice.
///
/// `Known` is the vocabulary this build compiles against; `Other` is a token
/// a newer (or wrong) fleet member declared, **carried verbatim rather than
/// dropped**. Carrying it is the point: RFC 08 §6 makes a disagreement
/// between two slices a *finding*, and a column silently normalised to
/// "unknown" cannot be diffed into one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Declared<T> {
    /// A token in this build's vocabulary.
    Known(T),
    /// A token this build does not know, kept as the slice spelled it.
    Other(String),
}

impl<T: SliceToken> Declared<T> {
    /// Read a column, recognising what this build knows and keeping the rest.
    pub fn parse(token: &str) -> Self {
        match T::from_token(token) {
            Some(known) => Declared::Known(known),
            None => Declared::Other(token.to_string()),
        }
    }

    /// The token as the slice spells it — round-trips through
    /// [`to_toml`] byte-for-byte, `Other` included.
    pub fn token(&self) -> &str {
        match self {
            Declared::Known(k) => k.token(),
            Declared::Other(s) => s,
        }
    }

    /// The recognised value, if this build knows the token.
    pub fn known(&self) -> Option<&T> {
        match self {
            Declared::Known(k) => Some(k),
            Declared::Other(_) => None,
        }
    }

    /// Whether this column is exactly the given known token. Prefer this to
    /// comparing [`token`](Declared::token) against a string literal — that
    /// is the typo hole this type closed.
    pub fn is(&self, other: &T) -> bool
    where
        T: PartialEq,
    {
        matches!(self, Declared::Known(k) if k == other)
    }
}

impl<T: SliceToken> From<T> for Declared<T> {
    fn from(value: T) -> Self {
        Declared::Known(value)
    }
}

impl<T: SliceToken> fmt::Display for Declared<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.token())
    }
}

/// Which half of RFC 05 §2's request/response split a procedure is
/// (`[[procedure]] kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProcedureKind {
    /// A side-effect-free query (RFC 05 §2).
    Read,
    /// A procedure that changes something (RFC 05 §2; `fanout` defaults to
    /// [`Fanout::Forbidden`] here, RFC 08 §2 G2).
    Write,
}

impl SliceToken for ProcedureKind {
    fn from_token(token: &str) -> Option<Self> {
        match token {
            "read" => Some(ProcedureKind::Read),
            "write" => Some(ProcedureKind::Write),
            _ => None,
        }
    }

    fn token(&self) -> &str {
        match self {
            ProcedureKind::Read => "read",
            ProcedureKind::Write => "write",
        }
    }
}

/// Whether a `*`-origin fan-out call may target a procedure (RFC 05 §2.1,
/// RFC 08 §2 G2) — the registry layer of the three-layer refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fanout {
    /// A `*`-origin fan-out call may target this procedure.
    Allowed,
    /// Fleet spellings are not generated for this procedure.
    Forbidden,
}

impl SliceToken for Fanout {
    fn from_token(token: &str) -> Option<Self> {
        match token {
            "allowed" => Some(Fanout::Allowed),
            "forbidden" => Some(Fanout::Forbidden),
            _ => None,
        }
    }

    fn token(&self) -> &str {
        match self {
            Fanout::Allowed => "allowed",
            Fanout::Forbidden => "forbidden",
        }
    }
}

/// The declared rate class of an `events` subject (RFC 04 §1.3, RFC 08 §5).
///
/// Not a [`Declared`] column, and the difference is the point: the vocabulary
/// is `rare | low | burst(n/h)`, and the third is **parameterized**. Wrapping
/// it in `Declared` would file every legitimate `burst(240/h)` under
/// `Other` — the arm that means "a token this build does not know" — and lose
/// the budget in the process. So this carries its own `Other`, the shape
/// [`WireEncoding`] has had since v1.5.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RateClass {
    /// `rare` — at most one an hour; silence is the normal state.
    Rare,
    /// `low` — at most sixty an hour.
    Low,
    /// `burst(n/h)` — an explicit per-hour budget.
    Burst(u64),
    /// A spelling this build does not know, carried verbatim.
    Other(String),
}

impl RateClass {
    /// Read a `rate` token. Unknown spellings are carried, not rejected —
    /// this reads foreign slices (RFC 08 §6).
    pub fn parse(token: &str) -> RateClass {
        match token {
            "rare" => RateClass::Rare,
            "low" => RateClass::Low,
            other => match other
                .strip_prefix("burst(")
                .and_then(|r| r.strip_suffix("/h)"))
                .and_then(|n| n.parse().ok())
            {
                Some(n) => RateClass::Burst(n),
                None => RateClass::Other(other.to_string()),
            },
        }
    }

    /// The declared ceiling in events per hour, where one is declared.
    ///
    /// `None` for a spelling this build cannot read — which is *not* the same
    /// as "no ceiling", and callers must not treat it as unlimited.
    pub fn cap_per_hour(&self) -> Option<u64> {
        match self {
            RateClass::Rare => Some(1),
            RateClass::Low => Some(60),
            RateClass::Burst(n) => Some(*n),
            RateClass::Other(_) => None,
        }
    }

    /// The token as a registry TOML spells it.
    pub fn token(&self) -> String {
        match self {
            RateClass::Rare => "rare".to_string(),
            RateClass::Low => "low".to_string(),
            RateClass::Burst(n) => format!("burst({n}/h)"),
            RateClass::Other(s) => s.clone(),
        }
    }
}

impl fmt::Display for RateClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.token())
    }
}

impl SliceToken for Class {
    fn from_token(token: &str) -> Option<Self> {
        Class::from_chunk(token)
    }

    fn token(&self) -> &str {
        self.chunk()
    }
}

impl SliceToken for QosProfile {
    fn from_token(token: &str) -> Option<Self> {
        QosProfile::from_name(token)
    }

    fn token(&self) -> &str {
        self.name()
    }
}

impl SliceToken for BlobTier {
    fn from_token(token: &str) -> Option<Self> {
        BlobTier::from_chunk(token)
    }

    fn token(&self) -> &str {
        self.chunk()
    }
}

impl SliceToken for CommonFamily {
    fn from_token(token: &str) -> Option<Self> {
        CommonFamily::ALL.into_iter().find(|f| f.token() == token)
    }

    fn token(&self) -> &str {
        CommonFamily::token(*self)
    }
}

impl SliceToken for ServiceOrigin {
    fn from_token(token: &str) -> Option<Self> {
        ServiceOrigin::new(token).ok()
    }

    fn token(&self) -> &str {
        self.as_str()
    }
}

/// What a subject's leaf value *is* (`[[subject]] kind`, RFC 08 §2, v1.32),
/// so a judge can say when it is not (RFC 13 §3 "Declared versus observed").
///
/// Absent from a declaration means *unchecked*: the judge answers *not
/// asked*, never *yes*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubjectKind {
    /// A non-negative number that never decreases within one origin's
    /// series, except across a producer restart — visible on the wire as the
    /// origin's `alive` token cycling (RFC 04 §5).
    Counter,
    /// Any number.
    Gauge,
    /// A string.
    Text,
    /// A boolean. A self-describing payload spells the tag `boolean`
    /// (RFC 11 §4).
    Bool,
}

impl SubjectKind {
    /// Every kind, in declaration order — the closed vocabulary a lint
    /// names when it refuses a token.
    pub const ALL: [SubjectKind; 4] = [
        SubjectKind::Counter,
        SubjectKind::Gauge,
        SubjectKind::Text,
        SubjectKind::Bool,
    ];

    /// The tag a self-describing `{"type": …, "value": …}` payload carries
    /// for this kind (RFC 08 §2): the registry token, except `boolean` for
    /// [`SubjectKind::Bool`].
    #[must_use]
    pub fn payload_tag(self) -> &'static str {
        match self {
            SubjectKind::Bool => "boolean",
            other => other.token_str(),
        }
    }

    /// The kind a payload tag names, if any — the inverse of
    /// [`payload_tag`](Self::payload_tag). A tag that is not one of the four
    /// is `None`: an unknown tag is not a disagreement.
    #[must_use]
    pub fn from_payload_tag(tag: &str) -> Option<Self> {
        SubjectKind::ALL
            .into_iter()
            .find(|k| k.payload_tag() == tag)
    }

    const fn token_str(self) -> &'static str {
        match self {
            SubjectKind::Counter => "counter",
            SubjectKind::Gauge => "gauge",
            SubjectKind::Text => "text",
            SubjectKind::Bool => "bool",
        }
    }
}

impl SliceToken for SubjectKind {
    fn from_token(token: &str) -> Option<Self> {
        SubjectKind::ALL
            .into_iter()
            .find(|k| k.token_str() == token)
    }

    fn token(&self) -> &str {
        self.token_str()
    }
}

/// One `[[subject]]` entry of a served registry slice.
/// `#[non_exhaustive]`: every version of this type so far has been the
/// previous one plus a field (`encoding` v1.5, `blob` v1.8, `media`
/// v1.16), and each of those was a breaking change for anyone
/// constructing one. It is a *parse result*, not a thing callers build
/// — [`parse_slice`] is the constructor — so the attribute costs the
/// intended use nothing and stops the next field being a break (#325).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SubjectDecl {
    /// The subject pattern, base-relative to `<class>/<producer>` — e.g.
    /// `disk/{mount}/used`.
    pub path: String,
    /// `telemetry` | `state` | `events` (RFC 04 §1).
    pub class: Declared<Class>,
    /// The payload type name, as the producer declares it.
    pub type_name: String,
    /// What the leaf value *is* — `counter | gauge | text | bool` (RFC 08 §2,
    /// v1.32), when declared. Absent means unchecked: a judge answers *not
    /// asked* (RFC 13 §3).
    pub kind: Option<Declared<SubjectKind>>,
    /// `common = "health|errors|sensor|…"` — which of the RFC 08 §5 framework
    /// state roles this subject declares itself as, when it declares one.
    ///
    /// Carried since #50: a producer serves its registry TOML verbatim, so
    /// this field has always ridden the wire; the slice type simply dropped it
    /// on the floor. That was invisible until `registry export --as toml` made
    /// the parse→emit path a round trip somebody might feed back into a build,
    /// where a lost `common` silently changes the generated
    /// `AnySubject::common_state()`.
    pub common: Option<Declared<CommonFamily>>,
    /// Registry version this subject first appeared in.
    pub since: Option<String>,
    pub description: Option<String>,
    /// The declared QoS profile (RFC 04 §3), when the slice carries one.
    pub qos: Option<Declared<QosProfile>>,
    /// State-subject freshness bound (RFC 04 §1.2), when declared.
    pub ttl_s: Option<i64>,
    /// The subject's unit (RFC 08 §4), when declared.
    pub unit: Option<String>,
    /// Events rate class (RFC 04 §1.3), when declared.
    pub rate: Option<RateClass>,
    /// The declared key-population bound, when declared.
    pub cardinality: Option<i64>,
    /// The declared payload encoding (`application/cbor`, …), when declared
    /// (RFC 08 §2, v1.5). Resolution: sample `Encoding` > this > sniff.
    /// `WireEncoding` carries its own `Other` arm, so it needs no
    /// [`Declared`] wrapper — it has been this shape since v1.5.
    pub encoding: Option<WireEncoding>,
}

/// One `[[procedure]]` entry of a served registry slice.
/// `#[non_exhaustive]`: every version of this type so far has been the
/// previous one plus a field (`encoding` v1.5, `blob` v1.8, `media`
/// v1.16), and each of those was a breaking change for anyone
/// constructing one. It is a *parse result*, not a thing callers build
/// — [`parse_slice`] is the constructor — so the attribute costs the
/// intended use nothing and stops the next field being a break (#325).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProcedureDecl {
    /// The procedure path, base-relative to the producer's `@rpc` root.
    pub path: String,
    /// `read` | `write` (RFC 05 §2).
    pub kind: Option<Declared<ProcedureKind>>,
    pub reply: Option<String>,
    /// The declared request type name, when declared.
    pub request: Option<String>,
    /// The declared payload encoding (RFC 08 §2, v1.5).
    pub encoding: Option<WireEncoding>,
    /// `"forbidden"` forbids `*`-origin fan-out calls (RFC 05 §2.1); absent
    /// means unconstrained. Surfaced in the slice since 0.5 so *dynamic*
    /// callers (explorers) can refuse what generated builders make
    /// unspellable — the registry layer of the three-layer refusal.
    pub fanout: Option<Declared<Fanout>>,
    /// Whether the procedure declares itself idempotent (RFC 08 §2).
    pub idempotent: Option<bool>,
    /// The declared key-population bound of a `{var}`-bearing path, when
    /// declared (RFC 08 §2 — required there, optional here: this type reads
    /// foreign slices, and the strict check belongs to that build's own
    /// zenkey-build).
    pub cardinality: Option<i64>,
    pub since: Option<String>,
    pub description: Option<String>,
}

/// One `[[blob]]` entry of a served registry slice (RFC 08 §2, v1.8).
///
/// Unlike the other declarations this one has no `path`: RFC 07 §2 fixes the
/// three blob key shapes, and their variable chunks are content addresses
/// rather than registry vocabulary. What a slice reveals — and the reason
/// modelling `@blob` was worth doing — is *which tiers an origin serves*, so
/// an explorer can answer "who holds blobs, and of which kind?" without
/// probing the bus for keys nobody may be serving.
///
/// Note the asymmetry, which was pre-existing rather than introduced here:
/// `[[media]]` had a registry field table since v1.3 and codegen since v1.5,
/// but did not appear in a slice until v1.16 ([`MediaDecl`]).
/// `#[non_exhaustive]`: every version of this type so far has been the
/// previous one plus a field (`encoding` v1.5, `blob` v1.8, `media`
/// v1.16), and each of those was a breaking change for anyone
/// constructing one. It is a *parse result*, not a thing callers build
/// — [`parse_slice`] is the constructor — so the attribute costs the
/// intended use nothing and stops the next field being a break (#325).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BlobDecl {
    /// `artifact` | `tree` | `store` (RFC 07 §2).
    pub tier: Declared<BlobTier>,
    /// The RFC 07 §2.2 endpoints served under `artifact/<id>/`; empty for the
    /// Tier-2 tiers, whose key *is* the endpoint.
    pub endpoints: Vec<String>,
    /// The `<algo>` chunk, on the `store` tier (RFC 07 §2.4).
    pub algo: Option<String>,
    /// The type conveying a reference to this blob — the payload that must
    /// carry the content root (RFC 07 §2.1).
    pub reference: Option<String>,
    /// The blob content's encoding, when declared.
    pub encoding: Option<WireEncoding>,
    pub since: Option<String>,
    pub description: Option<String>,
}

/// One `[[media]]` stream shape (RFC 08 §2; reaching the slice in v1.16).
///
/// Through v1.7 §6 *claimed* the introspect slice carried "media shapes" and
/// it never had; v1.8 corrected the claim and deferred the retrofit; this is
/// the retrofit. Same forward-compat posture as [`BlobDecl`]: optional
/// fields stay optional, unknown vocabulary is carried rather than refused —
/// this parser reads *foreign* slices, and the strict checks belong to that
/// build's own `zenkey-build`.
/// `#[non_exhaustive]`: every version of this type so far has been the
/// previous one plus a field (`encoding` v1.5, `blob` v1.8, `media`
/// v1.16), and each of those was a breaking change for anyone
/// constructing one. It is a *parse result*, not a thing callers build
/// — [`parse_slice`] is the constructor — so the attribute costs the
/// intended use nothing and stops the next field being a break (#325).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MediaDecl {
    /// Media sub-path after `@media/<producer>/` — the stream pattern
    /// (`{stream}/preview/jpeg`), same variable rules as a subject.
    pub path: String,
    /// The wire `Encoding` on every frame (`image/jpeg`, `video/*` — may be
    /// a family): the codec is declared here, never in a payload envelope
    /// (RFC 07 §1).
    pub encoding: WireEncoding,
    /// The per-frame sidecar type on the attachment (`FrameMeta`).
    pub attachment: Option<String>,
    /// Key-population bound, when the path carries variables.
    pub cardinality: Option<i64>,
    pub since: Option<String>,
    pub description: Option<String>,
}

/// The `[budget]` table of a served registry slice (RFC 08 §2, v1.32):
/// what this producer may **cost** the machine it runs on, in the units its
/// health document's `self_stats` reports (RFC 04 §1.2).
///
/// The first per-producer table on the slice that is not a subject,
/// procedure, tier or stream: it generates no key and no builder, and rides
/// the slice verbatim into `introspect` (RFC 08 §6) so an observer that can
/// read `self_stats` can say whether the claim holds (RFC 13 §3). Every
/// bound is optional here for the same reason every other optional column
/// is — this type reads *foreign* slices, and the strict lint belongs to
/// that build's own zenkey-build.
/// `#[non_exhaustive]`: every version of this type so far has been the
/// previous one plus a field (`encoding` v1.5, `blob` v1.8, `media`
/// v1.16), and each of those was a breaking change for anyone
/// constructing one. It is a *parse result*, not a thing callers build
/// — [`parse_slice`] is the constructor — so the attribute costs the
/// intended use nothing and stops the next field being a break (#325).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BudgetDecl {
    /// The whole process's resident set, in MiB (`rss_mb`).
    pub rss_mb: Option<i64>,
    /// Each bounded structure the producer keeps (`[[budget.tables]]`).
    pub tables: Vec<TableBudget>,
}

/// One `[[budget.tables]]` row (RFC 08 §2, v1.32): a named table and the
/// bounds it must stay within.
/// `#[non_exhaustive]`: every version of this type so far has been the
/// previous one plus a field (`encoding` v1.5, `blob` v1.8, `media`
/// v1.16), and each of those was a breaking change for anyone
/// constructing one. It is a *parse result*, not a thing callers build
/// — [`parse_slice`] is the constructor — so the attribute costs the
/// intended use nothing and stops the next field being a break (#325).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TableBudget {
    /// The table's name — what `self_stats.tables[].name` is matched on;
    /// required, and unique within the file.
    pub name: String,
    /// Upper bound on the table's occupancy, when declared.
    pub max_entries: Option<i64>,
    /// Upper bound on the table's size in bytes, when declared.
    pub max_bytes: Option<i64>,
}

/// One `[[deprecated]]` entry — RFC 08 §3's append-only retirement ledger.
/// `#[non_exhaustive]`: every version of this type so far has been the
/// previous one plus a field (`encoding` v1.5, `blob` v1.8, `media`
/// v1.16), and each of those was a breaking change for anyone
/// constructing one. It is a *parse result*, not a thing callers build
/// — [`parse_slice`] is the constructor — so the attribute costs the
/// intended use nothing and stops the next field being a break (#325).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DeprecationDecl {
    pub path: String,
    /// Which declared surface was retired (RFC 08 §3, v1.26).
    ///
    /// Absent in the TOML means [`DeprecatedKind::Subject`], which is what
    /// every entry written before the amendment was — and `to_toml` omits
    /// the field for that value, so a pre-v1.26 slice round-trips byte for
    /// byte.
    pub kind: DeprecatedKind,
    /// Registry version the retirement was recorded in.
    pub since: Option<String>,
    /// What replaced it, if anything.
    pub replaced_by: Option<String>,
}

/// Which declared surface a `[[deprecated]]` entry retires (RFC 08 §3, v1.26).
///
/// Retirement covered subjects only, which left a removed procedure with no
/// sanctioned exit at all: the `registry.lock` pin failed as *vanished
/// without retirement*, and the ledger entry meant to be the exit was never
/// consulted for it (#377).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeprecatedKind {
    #[default]
    Subject,
    Procedure,
}

impl DeprecatedKind {
    /// The token the registry TOML and both ledgers spell it with.
    pub fn as_str(self) -> &'static str {
        match self {
            DeprecatedKind::Subject => "subject",
            DeprecatedKind::Procedure => "procedure",
        }
    }
}

impl std::str::FromStr for DeprecatedKind {
    type Err = ();

    fn from_str(s: &str) -> Result<DeprecatedKind, ()> {
        match s {
            "subject" => Ok(DeprecatedKind::Subject),
            "procedure" => Ok(DeprecatedKind::Procedure),
            _ => Err(()),
        }
    }
}

/// What one build says it serves: the payload of an `introspect` reply.
/// `#[non_exhaustive]`: every version of this type so far has been the
/// previous one plus a field (`encoding` v1.5, `blob` v1.8, `media`
/// v1.16), and each of those was a breaking change for anyone
/// constructing one. It is a *parse result*, not a thing callers build
/// — [`parse_slice`] is the constructor — so the attribute costs the
/// intended use nothing and stops the next field being a break (#325).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RegistrySlice {
    /// `[registry] version` — the number a version-skew check compares.
    pub version: String,
    pub app: String,
    /// The convention major version (`1` for keyspace-v2 as ratified).
    pub convention: i64,
    /// The producer or service base name (`sysinfo`, `catalog`, …).
    pub name: String,
    /// `Some(origin)` for a service (`@catalog`); `None` for a host producer,
    /// whose origin is the host it runs on and therefore not in the slice.
    pub service_origin: Option<Declared<ServiceOrigin>>,
    pub description: Option<String>,
    pub subjects: Vec<SubjectDecl>,
    pub procedures: Vec<ProcedureDecl>,
    /// The `@blob` tiers this build serves (RFC 08 §2/§6, v1.8). Empty for
    /// every producer that serves no blobs, and for any slice written before
    /// v1.8 — the parser is forward- *and* backward-tolerant here, which is
    /// the same posture it takes to unknown keys.
    pub blob: Vec<BlobDecl>,
    /// The `@media` streams this build publishes (RFC 08 §2/§6, v1.16 —
    /// closing the asymmetry v1.8 recorded). Same tolerance as `blob`:
    /// empty for non-media producers and for every slice written earlier.
    pub media: Vec<MediaDecl>,
    pub deprecated: Vec<DeprecationDecl>,
    /// The producer's declared cost (RFC 08 §2, v1.32). `None` for every
    /// slice that declares none, and for every slice written earlier —
    /// which is what "not asked" reads from (RFC 13 §3).
    pub budget: Option<BudgetDecl>,
}

impl SubjectDecl {
    /// A subject declaration with its two required columns; the optional
    /// ones are `None` and assignable — the fields stay `pub`, and
    /// `#[non_exhaustive]` blocks only literal construction.
    #[must_use]
    pub fn new(path: impl Into<String>, class: impl Into<Declared<Class>>) -> Self {
        SubjectDecl {
            path: path.into(),
            class: class.into(),
            type_name: String::new(),
            kind: None,
            common: None,
            since: None,
            description: None,
            qos: None,
            ttl_s: None,
            unit: None,
            rate: None,
            cardinality: None,
            encoding: None,
        }
    }
}

impl ProcedureDecl {
    /// A procedure declaration with its one required column.
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        ProcedureDecl {
            path: path.into(),
            kind: None,
            reply: None,
            request: None,
            encoding: None,
            fanout: None,
            idempotent: None,
            cardinality: None,
            since: None,
            description: None,
        }
    }
}

impl BlobDecl {
    /// A blob-tier declaration with its one required column.
    #[must_use]
    pub fn new(tier: impl Into<Declared<BlobTier>>) -> Self {
        BlobDecl {
            tier: tier.into(),
            endpoints: Vec::new(),
            algo: None,
            reference: None,
            encoding: None,
            since: None,
            description: None,
        }
    }
}

impl MediaDecl {
    /// A media-stream declaration with its two required columns.
    #[must_use]
    pub fn new(path: impl Into<String>, encoding: WireEncoding) -> Self {
        MediaDecl {
            path: path.into(),
            encoding,
            attachment: None,
            cardinality: None,
            since: None,
            description: None,
        }
    }
}

impl BudgetDecl {
    /// An empty budget — no resident-set bound, no tables; both assignable.
    #[must_use]
    pub fn new() -> Self {
        BudgetDecl {
            rss_mb: None,
            tables: Vec::new(),
        }
    }
}

impl Default for BudgetDecl {
    fn default() -> Self {
        BudgetDecl::new()
    }
}

impl TableBudget {
    /// A table budget with its one required column; the bounds are `None`
    /// and assignable.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        TableBudget {
            name: name.into(),
            max_entries: None,
            max_bytes: None,
        }
    }
}

impl DeprecationDecl {
    /// A retirement-ledger entry with its one required column, retiring a
    /// subject. [`DeprecationDecl::of`] retires a procedure.
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        DeprecationDecl::of(DeprecatedKind::Subject, path)
    }

    /// The same, naming which surface is retired (RFC 08 §3, v1.26).
    #[must_use]
    pub fn of(kind: DeprecatedKind, path: impl Into<String>) -> Self {
        DeprecationDecl {
            path: path.into(),
            kind,
            since: None,
            replaced_by: None,
        }
    }
}

impl RegistrySlice {
    /// An empty slice with its required header. `convention` is the
    /// keyspace-v2 major version, `1`; the collections start empty and are
    /// assignable.
    ///
    /// [`parse_slice`] is how a slice normally arrives — this is for the
    /// callers that build one to compare against.
    #[must_use]
    pub fn new(
        version: impl Into<String>,
        app: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        RegistrySlice {
            version: version.into(),
            app: app.into(),
            convention: 1,
            name: name.into(),
            service_origin: None,
            description: None,
            subjects: Vec::new(),
            procedures: Vec::new(),
            blob: Vec::new(),
            media: Vec::new(),
            deprecated: Vec::new(),
            budget: None,
        }
    }

    /// Subjects of one class.
    ///
    /// Takes the vocabulary, not a `&str`: `subjects_in("telementry")` used
    /// to compile and answer "no subjects", which is the same answer an
    /// honestly empty class gives. `Class::State` is the ordinary spelling;
    /// a [`Declared`] answers the same question about a class token only a
    /// newer fleet member knows.
    pub fn subjects_in(
        &self,
        class: impl Into<Declared<Class>>,
    ) -> impl Iterator<Item = &SubjectDecl> {
        let class = class.into();
        self.subjects.iter().filter(move |s| s.class == class)
    }

    /// Does this slice serve a subject with exactly this pattern?
    pub fn serves_subject(&self, path: &str) -> bool {
        self.subjects.iter().any(|s| s.path == path)
    }

    /// Does this slice serve this procedure?
    pub fn serves_procedure(&self, path: &str) -> bool {
        self.procedures.iter().any(|p| p.path == path)
    }

    /// Does this slice serve this `@blob` tier (RFC 07 §2)?
    ///
    /// `BlobTier::Artifact` is the ordinary spelling; a [`Declared`] asks the
    /// same question about a tier token only a newer fleet member knows,
    /// which is what [`diff`] needs to call skew.
    pub fn serves_blob_tier(&self, tier: impl Into<Declared<BlobTier>>) -> bool {
        let tier = tier.into();
        self.blob.iter().any(|b| b.tier == tier)
    }

    /// Every `@blob` tier this slice declares, recognised or not.
    ///
    /// The `Other` tokens are the interesting ones: RFC 08 §6 makes a tier
    /// this build cannot name a *finding* about version skew, which is only
    /// possible because [`parse_slice`] carries it rather than dropping it.
    pub fn blob_tiers(&self) -> impl Iterator<Item = &Declared<BlobTier>> {
        self.blob.iter().map(|b| &b.tier)
    }

    /// Does this slice publish a `@media` stream with exactly this pattern
    /// (RFC 07 §1, in the slice since v1.16)?
    pub fn serves_media(&self, path: &str) -> bool {
        self.media.iter().any(|m| m.path == path)
    }
}

/// Why a slice would not parse.
///
/// An enum with a real `source()` since #317: the TOML variant used to be a
/// stringified `toml::de::Error` behind an empty `impl std::error::Error`, so
/// a caller who wanted the span, or just to tell a syntax error from a
/// missing header, had nothing to match on but the sentence.
#[derive(Debug, thiserror::Error)]
pub enum SliceError {
    /// The reply is not well-formed TOML. The `toml` error is the source.
    #[error("malformed registry slice: {0}")]
    Toml(#[from] toml::de::Error),
    /// The reply parses as TOML but is not a registry slice — a missing
    /// header, a required field absent. There is no underlying error here:
    /// this crate is the one saying so.
    #[error("malformed registry slice: {0}")]
    Shape(String),
}

/// Parse an `introspect` reply — the raw registry TOML a build serves.
///
/// Deliberately tolerant of *unknown* keys (a newer fleet member may declare
/// fields this build has never heard of, and refusing to read the rest of its
/// slice would turn a forward-compatible addition into an outage of the very
/// view that exists to spot skew) and intolerant of *missing* ones (a slice
/// without a version cannot be diffed, which is the whole point).
pub fn parse_slice(toml_src: &str) -> Result<RegistrySlice, SliceError> {
    let doc: toml::Value = toml::from_str(toml_src)?;

    let err = |m: &str| SliceError::Shape(m.to_string());
    let s = |v: Option<&toml::Value>| v.and_then(|v| v.as_str()).map(str::to_string);
    // A closed-vocabulary column: recognised where this build knows the token,
    // carried verbatim where it does not (RFC 08 §6 — skew is a finding, and a
    // column normalised to "unknown" cannot be diffed into one).
    fn tok<T: SliceToken>(v: Option<&toml::Value>) -> Option<Declared<T>> {
        v.and_then(|v| v.as_str()).map(Declared::parse)
    }
    fn enc(v: Option<&toml::Value>) -> Option<WireEncoding> {
        v.and_then(|v| v.as_str())
            .map(WireEncoding::from_encoding_str)
    }

    let header = doc
        .get("registry")
        .ok_or_else(|| err("missing [registry]"))?;
    let version = s(header.get("version")).ok_or_else(|| err("[registry] missing version"))?;
    let app = s(header.get("app")).ok_or_else(|| err("[registry] missing app"))?;
    let convention = header
        .get("convention")
        .and_then(|v| v.as_integer())
        .ok_or_else(|| err("[registry] missing convention"))?;

    let (name, service_origin, description) = if let Some(svc) = doc.get("service") {
        (
            s(svc.get("name")).ok_or_else(|| err("[service] missing name"))?,
            Some(tok(svc.get("origin")).ok_or_else(|| err("[service] missing origin"))?),
            s(svc.get("description")),
        )
    } else if let Some(prod) = doc.get("producer") {
        (
            s(prod.get("name")).ok_or_else(|| err("[producer] missing name"))?,
            None,
            s(prod.get("description")),
        )
    } else {
        return Err(err("missing [producer] or [service]"));
    };

    // `[budget]` (RFC 08 §2, v1.32): the same foreign-slice tolerance as
    // the arrays below — a row without a `name` cannot be matched against
    // `self_stats.tables[]` and is the one refusal; every bound is carried
    // when present. The strict lint (non-negative, unique names) is
    // zenkey-build's, on the build that authored the file.
    let budget = match doc.get("budget") {
        None => None,
        Some(b) => {
            let mut tables = Vec::new();
            for e in b
                .get("tables")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
            {
                tables.push(TableBudget {
                    name: s(e.get("name")).ok_or_else(|| err("[[budget.tables]] missing name"))?,
                    max_entries: e.get("max_entries").and_then(|v| v.as_integer()),
                    max_bytes: e.get("max_bytes").and_then(|v| v.as_integer()),
                });
            }
            Some(BudgetDecl {
                rss_mb: b.get("rss_mb").and_then(|v| v.as_integer()),
                tables,
            })
        }
    };

    let array = |key: &str| -> Vec<&toml::Value> {
        doc.get(key)
            .and_then(|v| v.as_array())
            .map(|a| a.iter().collect())
            .unwrap_or_default()
    };

    let mut subjects = Vec::new();
    for e in array("subject") {
        subjects.push(SubjectDecl {
            path: s(e.get("path")).ok_or_else(|| err("[[subject]] missing path"))?,
            class: tok(e.get("class")).ok_or_else(|| err("[[subject]] missing class"))?,
            type_name: s(e.get("type")).unwrap_or_default(),
            kind: tok(e.get("kind")),
            common: tok(e.get("common")),
            since: s(e.get("since")),
            description: s(e.get("description")),
            qos: tok(e.get("qos")),
            ttl_s: e.get("ttl_s").and_then(|v| v.as_integer()),
            unit: s(e.get("unit")),
            rate: e.get("rate").and_then(|v| v.as_str()).map(RateClass::parse),
            cardinality: e.get("cardinality").and_then(|v| v.as_integer()),
            encoding: enc(e.get("encoding")),
        });
    }

    let mut procedures = Vec::new();
    for e in array("procedure") {
        procedures.push(ProcedureDecl {
            path: s(e.get("path")).ok_or_else(|| err("[[procedure]] missing path"))?,
            kind: tok(e.get("kind")),
            reply: s(e.get("reply")),
            request: s(e.get("request")),
            fanout: tok(e.get("fanout")),
            idempotent: e.get("idempotent").and_then(|v| v.as_bool()),
            cardinality: e.get("cardinality").and_then(|v| v.as_integer()),
            encoding: enc(e.get("encoding")),
            since: s(e.get("since")),
            description: s(e.get("description")),
        });
    }

    // `[[blob]]` (RFC 08 §2, v1.8). Every field is optional except `tier`:
    // this parser reads *foreign* slices, where the strict vocabulary checks
    // belong to that build's own `zenkey-build`, not to ours — refusing to
    // read the rest of a slice over a tier token we do not recognise would
    // turn a forward-compatible addition into an outage of the view that
    // exists to spot exactly that skew.
    let mut blob = Vec::new();
    for e in array("blob") {
        blob.push(BlobDecl {
            tier: tok(e.get("tier")).ok_or_else(|| err("[[blob]] missing tier"))?,
            endpoints: e
                .get("endpoints")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            algo: s(e.get("algo")),
            reference: s(e.get("reference")),
            encoding: enc(e.get("encoding")),
            since: s(e.get("since")),
            description: s(e.get("description")),
        });
    }

    // `[[media]]` (RFC 08 §2, reaching the slice in v1.16): the same
    // foreign-slice tolerance as `[[blob]]` — `path` and `encoding` are the
    // two fields without which a stream cannot even be named; everything
    // else is carried when present.
    let mut media = Vec::new();
    for e in array("media") {
        media.push(MediaDecl {
            path: s(e.get("path")).ok_or_else(|| err("[[media]] missing path"))?,
            encoding: enc(e.get("encoding")).ok_or_else(|| err("[[media]] missing encoding"))?,
            attachment: s(e.get("attachment")),
            cardinality: e.get("cardinality").and_then(|v| v.as_integer()),
            since: s(e.get("since")),
            description: s(e.get("description")),
        });
    }

    let mut deprecated = Vec::new();
    for e in array("deprecated") {
        let path = s(e.get("path")).ok_or_else(|| err("[[deprecated]] missing path"))?;
        let kind = match s(e.get("kind")) {
            None => DeprecatedKind::Subject,
            Some(k) => k.parse().map_err(|()| {
                err(&format!(
                    "[[deprecated]] {path:?} has kind = {k:?} — it is `subject` \
                     (the default) or `procedure`"
                ))
            })?,
        };
        deprecated.push(DeprecationDecl {
            path,
            kind,
            since: s(e.get("since")),
            replaced_by: s(e.get("replaced_by")),
        });
    }

    Ok(RegistrySlice {
        version,
        app,
        convention,
        name,
        service_origin,
        description,
        subjects,
        procedures,
        blob,
        media,
        deprecated,
        budget,
    })
}

/// Render a slice back to registry TOML — the inverse of [`parse_slice`]
/// (issue #50: `zenctl registry export --as toml`).
///
/// Lossy in exactly one direction, and deliberately: [`parse_slice`] is
/// tolerant of unknown keys, and a field this build has never heard of cannot
/// be re-emitted because it was never carried. Exporting a *foreign* slice
/// therefore round-trips what this build can read, which is the honest bound
/// and is stated here rather than discovered later. Everything this build does
/// carry round-trips exactly — pinned as a test.
pub fn to_toml(slice: &RegistrySlice) -> String {
    // TOML basic-string escaping: the values here are registry vocabulary
    // (chunk-legal paths, type names) plus free-text descriptions, and a
    // description with a quote in it must not produce a file that no longer
    // parses.
    fn s(value: &str) -> String {
        let mut out = String::with_capacity(value.len() + 2);
        out.push('"');
        for c in value.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }
    fn opt(out: &mut String, key: &str, value: Option<&str>) {
        if let Some(v) = value {
            out.push_str(&format!("{key} = {}\n", s(v)));
        }
    }
    // A typed column renders as the token the slice carried: `Known` by its
    // canonical spelling, `Other` verbatim — which is what keeps the export a
    // round trip for foreign slices too (`Declared::token`).
    fn opt_tok<T: SliceToken>(out: &mut String, key: &str, value: Option<&Declared<T>>) {
        if let Some(v) = value {
            out.push_str(&format!("{key} = {}\n", s(v.token())));
        }
    }
    fn opt_enc(out: &mut String, key: &str, value: Option<&WireEncoding>) {
        if let Some(v) = value {
            out.push_str(&format!("{key} = {}\n", s(v.as_encoding_str())));
        }
    }
    fn opt_int(out: &mut String, key: &str, value: Option<i64>) {
        if let Some(v) = value {
            out.push_str(&format!("{key} = {v}\n"));
        }
    }

    let mut out = String::new();
    out.push_str("[registry]\n");
    out.push_str(&format!("version = {}\n", s(&slice.version)));
    out.push_str(&format!("app = {}\n", s(&slice.app)));
    out.push_str(&format!("convention = {}\n", slice.convention));

    match &slice.service_origin {
        Some(origin) => {
            out.push_str("\n[service]\n");
            out.push_str(&format!("name = {}\n", s(&slice.name)));
            out.push_str(&format!("origin = {}\n", s(origin.token())));
        }
        None => {
            out.push_str("\n[producer]\n");
            out.push_str(&format!("name = {}\n", s(&slice.name)));
        }
    }
    opt(&mut out, "description", slice.description.as_deref());

    // `[budget]` sits right after the header block, where the TOML grammar
    // needs a plain table to be — before the first `[[array]]` — and is
    // written only when carried, so a pre-v1.32 slice round-trips byte for
    // byte.
    if let Some(b) = &slice.budget {
        out.push_str("\n[budget]\n");
        opt_int(&mut out, "rss_mb", b.rss_mb);
        for t in &b.tables {
            out.push_str("\n[[budget.tables]]\n");
            out.push_str(&format!("name = {}\n", s(&t.name)));
            opt_int(&mut out, "max_entries", t.max_entries);
            opt_int(&mut out, "max_bytes", t.max_bytes);
        }
    }

    for d in &slice.subjects {
        out.push_str("\n[[subject]]\n");
        out.push_str(&format!("path = {}\n", s(&d.path)));
        out.push_str(&format!("class = {}\n", s(d.class.token())));
        if !d.type_name.is_empty() {
            out.push_str(&format!("type = {}\n", s(&d.type_name)));
        }
        // Emitted only when carried, so a pre-v1.32 slice round-trips byte
        // for byte.
        opt_tok(&mut out, "kind", d.kind.as_ref());
        opt_tok(&mut out, "common", d.common.as_ref());
        opt_tok(&mut out, "qos", d.qos.as_ref());
        opt_int(&mut out, "ttl_s", d.ttl_s);
        opt(&mut out, "unit", d.unit.as_deref());
        opt(
            &mut out,
            "rate",
            d.rate.as_ref().map(RateClass::token).as_deref(),
        );
        opt_int(&mut out, "cardinality", d.cardinality);
        opt_enc(&mut out, "encoding", d.encoding.as_ref());
        opt(&mut out, "since", d.since.as_deref());
        opt(&mut out, "description", d.description.as_deref());
    }

    for d in &slice.procedures {
        out.push_str("\n[[procedure]]\n");
        out.push_str(&format!("path = {}\n", s(&d.path)));
        opt_tok(&mut out, "kind", d.kind.as_ref());
        opt(&mut out, "request", d.request.as_deref());
        opt(&mut out, "reply", d.reply.as_deref());
        opt_enc(&mut out, "encoding", d.encoding.as_ref());
        opt_tok(&mut out, "fanout", d.fanout.as_ref());
        if let Some(i) = d.idempotent {
            out.push_str(&format!("idempotent = {i}\n"));
        }
        opt_int(&mut out, "cardinality", d.cardinality);
        opt(&mut out, "since", d.since.as_deref());
        opt(&mut out, "description", d.description.as_deref());
    }

    for d in &slice.blob {
        out.push_str("\n[[blob]]\n");
        out.push_str(&format!("tier = {}\n", s(d.tier.token())));
        if !d.endpoints.is_empty() {
            let items: Vec<String> = d.endpoints.iter().map(|e| s(e)).collect();
            out.push_str(&format!("endpoints = [{}]\n", items.join(", ")));
        }
        opt(&mut out, "algo", d.algo.as_deref());
        opt(&mut out, "reference", d.reference.as_deref());
        opt_enc(&mut out, "encoding", d.encoding.as_ref());
        opt(&mut out, "since", d.since.as_deref());
        opt(&mut out, "description", d.description.as_deref());
    }

    for d in &slice.media {
        out.push_str("\n[[media]]\n");
        out.push_str(&format!("path = {}\n", s(&d.path)));
        out.push_str(&format!("encoding = {}\n", s(d.encoding.as_encoding_str())));
        opt(&mut out, "attachment", d.attachment.as_deref());
        if let Some(c) = d.cardinality {
            out.push_str(&format!("cardinality = {c}\n"));
        }
        opt(&mut out, "since", d.since.as_deref());
        opt(&mut out, "description", d.description.as_deref());
    }

    for d in &slice.deprecated {
        out.push_str("\n[[deprecated]]\n");
        out.push_str(&format!("path = {}\n", s(&d.path)));
        // Omitted for the default, so a pre-v1.26 slice round-trips byte for
        // byte through this exporter.
        if d.kind != DeprecatedKind::Subject {
            out.push_str(&format!("kind = {}\n", s(d.kind.as_str())));
        }
        opt(&mut out, "since", d.since.as_deref());
        opt(&mut out, "replaced_by", d.replaced_by.as_deref());
    }

    out
}

/// A disagreement between a served slice and the slice this build compiled in.
///
/// RFC 08 §6: a disagreement is a finding. Each variant is one thing an
/// operator would otherwise have to SSH to a host to learn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SliceFinding {
    /// The served `[registry] version` differs from ours.
    VersionSkew {
        served: String,
        local: String,
    },
    /// The host serves a subject we do not know — it is newer than us.
    UnknownSubject {
        path: String,
        class: Declared<Class>,
    },
    /// We know a subject the host does not serve — it is older than us.
    MissingSubject {
        path: String,
        class: Declared<Class>,
    },
    /// Likewise for procedures.
    UnknownProcedure {
        path: String,
    },
    MissingProcedure {
        path: String,
    },
    /// The host serves a `@blob` tier we do not know (RFC 08 §2, v1.8).
    UnknownBlobTier {
        tier: Declared<BlobTier>,
    },
    /// We know a `@blob` tier the host does not serve.
    MissingBlobTier {
        tier: Declared<BlobTier>,
    },
    /// The host publishes a `@media` stream we do not know (RFC 08 §6;
    /// `[[media]]` reached the slice in v1.16, and reached this diff in
    /// #169 — until then media drift produced no finding at all).
    UnknownMediaStream {
        path: String,
    },
    /// We know a `@media` stream the host does not publish.
    MissingMediaStream {
        path: String,
    },
    /// The host still serves a subject its own ledger marks deprecated.
    ServesDeprecated {
        path: String,
        replaced_by: Option<String>,
    },
}

impl SliceFinding {
    /// One line, for a table cell.
    pub fn summary(&self) -> String {
        match self {
            Self::VersionSkew { served, local } => {
                format!("registry {served} (we compiled {local})")
            }
            Self::UnknownSubject { path, class } => format!("serves unknown {class} {path}"),
            Self::MissingSubject { path, class } => format!("does not serve {class} {path}"),
            Self::UnknownProcedure { path } => format!("serves unknown procedure {path}"),
            Self::MissingProcedure { path } => format!("does not serve procedure {path}"),
            Self::UnknownBlobTier { tier } => format!("serves unknown @blob tier {tier}"),
            Self::MissingBlobTier { tier } => format!("does not serve @blob tier {tier}"),
            Self::UnknownMediaStream { path } => format!("serves unknown @media stream {path}"),
            Self::MissingMediaStream { path } => format!("does not serve @media stream {path}"),
            Self::ServesDeprecated { path, replaced_by } => match replaced_by {
                Some(r) => format!("serves deprecated {path} (use {r})"),
                None => format!("serves deprecated {path}"),
            },
        }
    }
}

/// Diff a served slice against the slice this build compiled in.
///
/// Empty means the host agrees with us exactly, which is the answer the view
/// wants to be able to give in one glance.
pub fn diff(served: &RegistrySlice, local: &RegistrySlice) -> Vec<SliceFinding> {
    let mut out = Vec::new();
    if served.version != local.version {
        out.push(SliceFinding::VersionSkew {
            served: served.version.clone(),
            local: local.version.clone(),
        });
    }
    for s in &served.subjects {
        if !local.serves_subject(&s.path) {
            out.push(SliceFinding::UnknownSubject {
                path: s.path.clone(),
                class: s.class.clone(),
            });
        }
    }
    for s in &local.subjects {
        if !served.serves_subject(&s.path) {
            out.push(SliceFinding::MissingSubject {
                path: s.path.clone(),
                class: s.class.clone(),
            });
        }
    }
    for p in &served.procedures {
        if !local.serves_procedure(&p.path) {
            out.push(SliceFinding::UnknownProcedure {
                path: p.path.clone(),
            });
        }
    }
    for p in &local.procedures {
        if !served.serves_procedure(&p.path) {
            out.push(SliceFinding::MissingProcedure {
                path: p.path.clone(),
            });
        }
    }
    for b in &served.blob {
        if !local.serves_blob_tier(b.tier.clone()) {
            out.push(SliceFinding::UnknownBlobTier {
                tier: b.tier.clone(),
            });
        }
    }
    for b in &local.blob {
        if !served.serves_blob_tier(b.tier.clone()) {
            out.push(SliceFinding::MissingBlobTier {
                tier: b.tier.clone(),
            });
        }
    }
    for m in &served.media {
        if !local.serves_media(&m.path) {
            out.push(SliceFinding::UnknownMediaStream {
                path: m.path.clone(),
            });
        }
    }
    for m in &local.media {
        if !served.serves_media(&m.path) {
            out.push(SliceFinding::MissingMediaStream {
                path: m.path.clone(),
            });
        }
    }
    // A host that still serves what its own ledger retired. Quiet until the
    // first deprecation lands, and exactly the question that needs SSH today.
    for d in &served.deprecated {
        let still_served = match d.kind {
            DeprecatedKind::Subject => served.serves_subject(&d.path),
            DeprecatedKind::Procedure => served.serves_procedure(&d.path),
        };
        if still_served {
            out.push(SliceFinding::ServesDeprecated {
                path: d.path.clone(),
                replaced_by: d.replaced_by.clone(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Corpus-level coverage ("every compiled slice parses") lives in
    // zenkey-build's fixture tests — this crate no longer bundles a registry.

    /// `to_toml` is the inverse of `parse_slice` for everything this build
    /// carries: parse → emit → parse yields the identical slice. Without this,
    /// `zenctl registry export --as toml` would be a formatter nobody could
    /// trust to feed back in (#50's own acceptance).
    #[test]
    fn toml_export_round_trips_every_carried_field() {
        let source = r#"
            [registry]
            version = "2.1"
            app = "acme"
            convention = 1
            [producer]
            name = "netring"
            description = "flow capture"
            [budget]
            rss_mb = 64
            [[budget.tables]]
            name = "flows"
            max_entries = 65536
            max_bytes = 16777216
            [[budget.tables]]
            name = "names"
            max_entries = 16384
            [[subject]]
            path = "flows/{proto}/count"
            class = "telemetry"
            type = "TelemetryPoint"
            kind = "counter"
            qos = "sampled"
            unit = "packets"
            cardinality = 512
            encoding = "application/cbor"
            since = "1.0"
            description = "per-protocol flow counter"
            [[subject]]
            path = "health"
            class = "state"
            type = "Health"
            common = "health"
            ttl_s = 60
            rate = "burst"
            [[procedure]]
            path = "capture/trigger"
            kind = "write"
            request = "CaptureSpec"
            reply = "Ack"
            encoding = "application/json"
            fanout = "forbidden"
            idempotent = false
            since = "1.1"
            description = "start a capture"
            [[procedure]]
            path = "capture/{port}/drain"
            kind = "write"
            reply = "Ack"
            cardinality = 8
            since = "1.2"
            description = "drain one port's ring"
            [[blob]]
            tier = "artifact"
            endpoints = ["manifest", "slice", "have"]
            reference = "ArtifactRef"
            encoding = "application/octet-stream"
            since = "1.2"
            description = "captured pcaps"
            [[blob]]
            tier = "store"
            algo = "blake3"
            since = "1.2"
            [[media]]
            path = "{stream}/preview/jpeg"
            encoding = "image/jpeg"
            attachment = "FrameMeta"
            cardinality = 16
            since = "1.3"
            description = "preview rung"
            [[deprecated]]
            path = "flows/legacy"
            since = "2.0"
            replaced_by = "flows/{proto}/count"
            [[deprecated]]
            kind = "procedure"
            path = "flows/reset"
            since = "2.0"
            "#;
        let parsed = parse_slice(source).unwrap();
        let emitted = to_toml(&parsed);
        let back = parse_slice(&emitted)
            .unwrap_or_else(|e| panic!("exported TOML must re-parse: {e}\n---\n{emitted}"));
        assert_eq!(back, parsed, "exported TOML:\n{emitted}");
    }

    /// `[budget]` (RFC 08 §2, v1.32) is carried whole — the resident-set
    /// bound and every table row with its bounds — and a slice that declares
    /// none carries `None`, which is what "not asked" reads from (RFC 13
    /// §3); such a slice also exports byte for byte as it did before the
    /// amendment.
    #[test]
    fn a_budget_is_carried_and_its_absence_stays_unwritten() {
        let source = "[registry]\nversion = \"1.0\"\napp = \"t\"\nconvention = 1\n\n\
                      [producer]\nname = \"netring\"\n\n[budget]\nrss_mb = 64\n\n\
                      [[budget.tables]]\nname = \"flows\"\nmax_entries = 65536\n\
                      max_bytes = 16777216\n\n[[budget.tables]]\nname = \"names\"\n\
                      max_entries = 16384\n";
        let parsed = parse_slice(source).unwrap();
        let budget = parsed.budget.as_ref().expect("[budget] carried");
        assert_eq!(budget.rss_mb, Some(64));
        assert_eq!(budget.tables.len(), 2);
        assert_eq!(budget.tables[0].name, "flows");
        assert_eq!(budget.tables[0].max_entries, Some(65536));
        assert_eq!(budget.tables[0].max_bytes, Some(16_777_216));
        assert_eq!(budget.tables[1].name, "names");
        assert_eq!(budget.tables[1].max_bytes, None);
        assert_eq!(
            to_toml(&parsed),
            source,
            "the emitter is the parser's inverse"
        );

        let bare = "[registry]\nversion = \"1.0\"\napp = \"t\"\nconvention = 1\n\n\
                    [producer]\nname = \"netring\"\n";
        let parsed = parse_slice(bare).unwrap();
        assert_eq!(parsed.budget, None, "no [budget] is not an empty one");
        assert_eq!(to_toml(&parsed), bare);
    }

    /// A table row without a `name` cannot be matched against anything the
    /// health document says (RFC 04 §1.2), so it is the one refusal here;
    /// every bound is optional, as every foreign-slice column is.
    #[test]
    fn a_budget_table_without_a_name_is_refused() {
        let header = "[registry]\nversion = \"1.0\"\napp = \"t\"\nconvention = 1\n\
                      [producer]\nname = \"netring\"\n";
        let e = parse_slice(&format!("{header}[[budget.tables]]\nmax_entries = 4\n"))
            .expect_err("a nameless row");
        assert!(
            e.to_string().contains("[[budget.tables]] missing name"),
            "{e}"
        );
        let ok = parse_slice(&format!("{header}[budget]\n")).unwrap();
        assert_eq!(
            ok.budget,
            Some(BudgetDecl::new()),
            "an empty table is carried"
        );
    }

    /// Retirement names its kind (RFC 08 §3, v1.26), and the default is the
    /// only thing a pre-amendment slice could have meant.
    #[test]
    fn a_deprecation_without_a_kind_is_a_subject_and_stays_unwritten() {
        let source = r#"
            [registry]
            version = "1.0"
            app = "t"
            convention = 1
            [producer]
            name = "p"
            [[deprecated]]
            path = "old"
            [[deprecated]]
            kind = "procedure"
            path = "reset"
            "#;
        let parsed = parse_slice(source).unwrap();
        assert_eq!(parsed.deprecated[0].kind, DeprecatedKind::Subject);
        assert_eq!(parsed.deprecated[1].kind, DeprecatedKind::Procedure);
        let emitted = to_toml(&parsed);
        // The subject entry gains no field it did not have; the procedure
        // entry says which it is.
        assert!(
            emitted.contains("[[deprecated]]\npath = \"old\"\n"),
            "{emitted}"
        );
        assert!(
            emitted.contains("path = \"reset\"\nkind = \"procedure\"\n"),
            "{emitted}"
        );
    }

    /// An unknown `kind` is a refusal, not a silent subject.
    #[test]
    fn a_deprecation_with_an_unknown_kind_is_refused() {
        let source = r#"
            [registry]
            version = "1.0"
            app = "t"
            convention = 1
            [producer]
            name = "p"
            [[deprecated]]
            kind = "media"
            path = "old"
            "#;
        let e = parse_slice(source).expect_err("kind is a closed vocabulary");
        assert!(e.to_string().contains("`procedure`"), "{e}");
    }

    /// A service slice keeps its `[service] origin` through the round trip —
    /// the one field that decides which of the two header tables is emitted.
    #[test]
    fn toml_export_keeps_a_service_origin() {
        let parsed = parse_slice(
            r#"
            [registry]
            version = "1.0"
            app = "acme"
            convention = 1
            [service]
            name = "catalog"
            origin = "@catalog"
            "#,
        )
        .unwrap();
        let emitted = to_toml(&parsed);
        assert!(emitted.contains("[service]"), "{emitted}");
        assert_eq!(parse_slice(&emitted).unwrap(), parsed);
    }

    /// Free text is escaped, not trusted: a description carrying a quote must
    /// not produce a file that no longer parses.
    #[test]
    fn toml_export_escapes_free_text() {
        let mut parsed = parse_slice(
            r#"
            [registry]
            version = "1.0"
            app = "acme"
            convention = 1
            [producer]
            name = "p"
            "#,
        )
        .unwrap();
        parsed.description = Some("a \"quoted\" \\ back\nslash".into());
        let emitted = to_toml(&parsed);
        assert_eq!(parse_slice(&emitted).unwrap(), parsed, "{emitted}");
    }

    #[test]
    fn a_service_slice_carries_its_origin() {
        let slice = parse_slice(
            r#"
            [registry]
            version = "1.0"
            app = "acme"
            convention = 1
            [service]
            name = "catalog"
            origin = "@catalog"
            [[subject]]
            path = "entity/{entity_id}"
            class = "state"
            type = "Entity"
            [[procedure]]
            path = "introspect"
            kind = "read"
            "#,
        )
        .unwrap();
        assert_eq!(
            slice.service_origin.as_ref().map(Declared::token),
            Some("@catalog")
        );
        assert!(slice.serves_procedure("introspect"));
    }

    /// A parse failure carries the `toml` error, not a copy of its sentence
    /// (#317). The chain is what a caller needs: `SliceError` says *which
    /// producer's slice*, the source says *where in the file*.
    #[test]
    fn a_malformed_slice_keeps_the_toml_error_as_its_source() {
        use std::error::Error as _;

        let err = parse_slice("this is not = = toml").unwrap_err();
        assert!(matches!(err, SliceError::Toml(_)), "{err:?}");

        // The chain reaches the real error, and it is a `toml` one — the
        // whole point: a caller can downcast to it rather than grep the text.
        let source = err.source().expect("a toml error underneath");
        assert!(
            source.downcast_ref::<toml::de::Error>().is_some(),
            "source was {source:?}"
        );

        // A slice that *is* TOML but is not a slice has no underlying error,
        // and says so rather than inventing one.
        let shape = parse_slice("[registry]\nversion = \"1.0\"\n").unwrap_err();
        assert!(matches!(shape, SliceError::Shape(_)), "{shape:?}");
        assert!(shape.source().is_none());
        assert!(shape.to_string().contains("missing app"), "{shape}");
    }

    /// A closed-vocabulary column recognises what this build knows and keeps
    /// the rest — and the kept token round-trips through [`to_toml`]
    /// verbatim, which is what lets [`diff`] call an unknown tier skew rather
    /// than silently normalising it away.
    #[test]
    fn a_declared_column_keeps_what_it_does_not_recognise() {
        let src = r#"
            [registry]
            version = "1.0"
            app = "acme"
            convention = 1
            [producer]
            name = "netring"
            [[subject]]
            path = "a"
            class = "telemetry"
            qos = "sampled"
            [[subject]]
            path = "b"
            class = "metrics"
            qos = "urgent"
        "#;
        let slice = parse_slice(src).unwrap();

        assert_eq!(slice.subjects[0].class, Declared::Known(Class::Telemetry));
        assert_eq!(
            slice.subjects[0].qos,
            Some(Declared::Known(QosProfile::Sampled))
        );
        // Neither token is in this build's vocabulary; both are carried.
        assert_eq!(slice.subjects[1].class, Declared::Other("metrics".into()));
        assert_eq!(
            slice.subjects[1].qos,
            Some(Declared::Other("urgent".into()))
        );
        assert_eq!(slice.subjects[1].class.token(), "metrics");
        assert_eq!(slice.subjects[1].class.known(), None);

        // The typed accessor answers about the vocabulary, and a foreign token
        // is reachable through the same accessor rather than a second one.
        assert_eq!(slice.subjects_in(Class::Telemetry).count(), 1);
        assert_eq!(
            slice
                .subjects_in(Declared::Other("metrics".to_string()))
                .count(),
            1
        );

        // And the export is a round trip for the carried token too.
        assert_eq!(parse_slice(&to_toml(&slice)).unwrap(), slice);
    }

    /// `rate` is the one column that is not a plain closed set: `burst(n/h)`
    /// carries a budget, so it gets [`WireEncoding`]'s shape (its own `Other`)
    /// rather than [`Declared`]'s — filing a legitimate `burst(240/h)` under
    /// "a token this build does not know" would lose the number.
    #[test]
    fn a_rate_class_keeps_its_burst_budget() {
        assert_eq!(RateClass::parse("rare").cap_per_hour(), Some(1));
        assert_eq!(RateClass::parse("low").cap_per_hour(), Some(60));
        assert_eq!(RateClass::parse("burst(240/h)"), RateClass::Burst(240));
        assert_eq!(RateClass::parse("burst(240/h)").cap_per_hour(), Some(240));
        // The `/h` is the unit and it is required: `burst(240)` names no
        // period, so it is a token this build cannot read rather than a
        // budget of 240 per something.
        assert_eq!(RateClass::parse("burst(240)").cap_per_hour(), None);

        // An unreadable spelling is carried, and its cap is *unknown* — which
        // is not "unlimited", and callers must not read it as one.
        let odd = RateClass::parse("whenever");
        assert_eq!(odd, RateClass::Other("whenever".into()));
        assert_eq!(odd.cap_per_hour(), None);

        // Every form round-trips through its own token.
        for token in ["rare", "low", "burst(240/h)", "whenever"] {
            assert_eq!(RateClass::parse(token).token(), token);
        }
    }

    /// This parser reads *foreign* slices (RFC 08 §6), so its posture is the
    /// opposite of zenkey-build's: only `tier` is required, and an
    /// unrecognised tier token is kept rather than rejected — refusing to
    /// read the rest of a slice over a forward-compatible addition would turn
    /// skew into an outage of the view that exists to spot skew.
    #[test]
    fn blob_entries_parse_lax_with_only_tier_required() {
        let header = r#"
            [registry]
            version = "1.8"
            app = "acme"
            convention = 1
            [producer]
            name = "netring"
        "#;
        let slice = parse_slice(&format!(
            r#"{header}
            [[blob]]
            tier = "artifact"
            endpoints = ["manifest", "have"]
            reference = "Delivery"
            [[blob]]
            tier = "flux"
            "#
        ))
        .unwrap();
        assert!(slice.serves_blob_tier(BlobTier::Artifact));
        let decl = slice
            .blob
            .iter()
            .find(|b| b.tier.is(&BlobTier::Artifact))
            .unwrap();
        assert_eq!(decl.endpoints, ["manifest", "have"]);
        assert_eq!(decl.reference.as_deref(), Some("Delivery"));
        assert_eq!(decl.algo, None);
        // The unknown tier is kept, so a diff can surface it as skew.
        assert!(slice.serves_blob_tier(Declared::Other("flux".into())));

        // `tier` itself is the one hard requirement.
        assert!(parse_slice(&format!("{header}\n[[blob]]\nalgo = \"blake3\"\n")).is_err());

        // Pre-v1.8 slices simply carry no blob entries.
        let old = parse_slice(header).unwrap();
        assert!(old.blob.is_empty());
        assert!(!old.serves_blob_tier(BlobTier::Artifact));
    }

    /// Blob tier drift is a finding in both directions, straight from
    /// `diff()` — the corpus-level version lives in fixture-tests, but this
    /// crate publishes standalone and must pin it locally.
    #[test]
    fn blob_tier_drift_is_a_finding() {
        let with = |tiers: &[&str]| {
            let mut src = String::from(
                "[registry]\nversion = \"1.8\"\napp = \"acme\"\nconvention = 1\n\
                 [producer]\nname = \"netring\"\n",
            );
            for t in tiers {
                src.push_str(&format!("[[blob]]\ntier = {t:?}\n"));
            }
            parse_slice(&src).unwrap()
        };
        let served = with(&["artifact", "tree"]);
        let local = with(&["tree", "store"]);
        let findings = diff(&served, &local);
        assert!(findings.iter().any(
            |f| matches!(f, SliceFinding::UnknownBlobTier { tier } if tier.is(&BlobTier::Artifact))
        ));
        assert!(findings.iter().any(
            |f| matches!(f, SliceFinding::MissingBlobTier { tier } if tier.is(&BlobTier::Store))
        ));
        assert!(diff(&served, &served).is_empty());
    }

    /// Media drift is a finding in both directions (#169): `[[media]]` rode
    /// the slice since v1.16, but `diff()` compared only subjects,
    /// procedures, and blob tiers — a host serving an undeclared `@media`
    /// stream, or missing a declared one, was silent.
    #[test]
    fn media_stream_drift_is_a_finding() {
        let with = |paths: &[&str]| {
            let mut src = String::from(
                "[registry]\nversion = \"1.16\"\napp = \"acme\"\nconvention = 1\n\
                 [producer]\nname = \"netring\"\n",
            );
            for p in paths {
                src.push_str(&format!(
                    "[[media]]\npath = {p:?}\nencoding = \"image/jpeg\"\n"
                ));
            }
            parse_slice(&src).unwrap()
        };
        let served = with(&["{stream}/preview/jpeg", "{stream}/live/h264"]);
        let local = with(&["{stream}/live/h264", "{stream}/still/png"]);
        let findings = diff(&served, &local);
        assert!(findings.iter().any(|f| matches!(
            f,
            SliceFinding::UnknownMediaStream { path } if path == "{stream}/preview/jpeg"
        )));
        assert!(findings.iter().any(|f| matches!(
            f,
            SliceFinding::MissingMediaStream { path } if path == "{stream}/still/png"
        )));
        // The stream both sides declare is not drift.
        assert!(!findings.iter().any(|f| matches!(
            f,
            SliceFinding::UnknownMediaStream { path }
            | SliceFinding::MissingMediaStream { path } if path == "{stream}/live/h264"
        )));
        assert!(diff(&served, &served).is_empty());

        // Pre-v1.16 slices simply carry no media entries: absent on both
        // sides is agreement, and absent against a declaring build is drift.
        let old = with(&[]);
        assert!(diff(&old, &old).is_empty());
        assert!(
            diff(&old, &local)
                .iter()
                .all(|f| matches!(f, SliceFinding::MissingMediaStream { .. }))
        );
    }

    #[test]
    fn a_slice_identical_to_ours_is_no_finding() {
        let slice = parse_slice(
            r#"
            [registry]
            version = "1.0"
            app = "acme"
            convention = 1
            [producer]
            name = "sysinfo"
            [[subject]]
            path = "cpu/usage"
            class = "telemetry"
            type = "TelemetryPoint"
            [[procedure]]
            path = "introspect"
            kind = "read"
            "#,
        )
        .unwrap();
        assert!(diff(&slice, &slice).is_empty());
    }

    #[test]
    fn skew_and_drift_are_findings() {
        let local = parse_slice(
            r#"
            [registry]
            version = "1.1"
            app = "zensight"
            convention = 1
            [producer]
            name = "sysinfo"
            [[subject]]
            path = "cpu/usage"
            class = "telemetry"
            type = "TelemetryPoint"
            [[procedure]]
            path = "introspect"
            kind = "read"
            "#,
        )
        .unwrap();
        let served = parse_slice(
            r#"
            [registry]
            version = "1.2"
            app = "zensight"
            convention = 1
            [producer]
            name = "sysinfo"
            [[subject]]
            path = "cpu/temperature"
            class = "telemetry"
            type = "TelemetryPoint"
            [[procedure]]
            path = "introspect"
            kind = "read"
            "#,
        )
        .unwrap();

        let findings = diff(&served, &local);
        assert!(findings.iter().any(|f| matches!(
            f,
            SliceFinding::VersionSkew { served, local } if served == "1.2" && local == "1.1"
        )));
        assert!(findings.iter().any(
            |f| matches!(f, SliceFinding::UnknownSubject { path, .. } if path == "cpu/temperature")
        ));
        assert!(findings.iter().any(
            |f| matches!(f, SliceFinding::MissingSubject { path, .. } if path == "cpu/usage")
        ));
    }

    /// A field we have never heard of must not cost us the rest of the slice —
    /// otherwise the view that exists to spot a newer fleet member breaks on
    /// exactly the member it was built to find.
    #[test]
    fn unknown_fields_do_not_break_the_parse() {
        let slice = parse_slice(
            r#"
            [registry]
            version = "9.9"
            app = "zensight"
            convention = 1
            future_knob = true
            [producer]
            name = "sysinfo"
            [[subject]]
            path = "cpu/usage"
            class = "telemetry"
            type = "TelemetryPoint"
            unheard_of = "whatever"
            "#,
        )
        .unwrap();
        assert_eq!(slice.version, "9.9");
        assert!(slice.serves_subject("cpu/usage"));
    }

    #[test]
    fn a_slice_without_a_version_cannot_be_diffed_and_is_rejected() {
        let e = parse_slice(
            r#"
            [registry]
            app = "zensight"
            convention = 1
            [producer]
            name = "sysinfo"
            "#,
        )
        .unwrap_err();
        assert!(e.to_string().contains("version"));
    }
}
