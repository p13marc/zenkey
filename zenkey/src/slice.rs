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

/// One `[[subject]]` entry of a served registry slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectDecl {
    /// The subject pattern, base-relative to `<class>/<producer>` — e.g.
    /// `disk/{mount}/used`.
    pub path: String,
    /// `telemetry` | `state` | `events`.
    pub class: String,
    /// The payload type name, as the producer declares it.
    pub type_name: String,
    /// `common = "health|errors|sensor|…"` — which of the RFC 08 §5 framework
    /// state roles this subject declares itself as, when it declares one.
    ///
    /// Carried since #50: a producer serves its registry TOML verbatim, so
    /// this field has always ridden the wire; the slice type simply dropped it
    /// on the floor. That was invisible until `registry export --as toml` made
    /// the parse→emit path a round trip somebody might feed back into a build,
    /// where a lost `common` silently changes the generated
    /// `AnySubject::common_state()`.
    pub common: Option<String>,
    /// Registry version this subject first appeared in.
    pub since: Option<String>,
    pub description: Option<String>,
    /// The declared QoS profile name (RFC 04 §3), when the slice carries one.
    pub qos: Option<String>,
    /// State-subject freshness bound (RFC 04 §1.2), when declared.
    pub ttl_s: Option<i64>,
    /// The subject's unit (RFC 08 §4), when declared.
    pub unit: Option<String>,
    /// Events rate class (RFC 04 §1.3), when declared.
    pub rate: Option<String>,
    /// The declared key-population bound, when declared.
    pub cardinality: Option<i64>,
    /// The declared payload encoding (`application/cbor`, …), when declared
    /// (RFC 08 §2, v1.5). Resolution: sample `Encoding` > this > sniff.
    pub encoding: Option<String>,
}

/// One `[[procedure]]` entry of a served registry slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcedureDecl {
    /// The procedure path, base-relative to the producer's `@rpc` root.
    pub path: String,
    /// `read` | `write`.
    pub kind: String,
    pub reply: Option<String>,
    /// The declared request type name, when declared.
    pub request: Option<String>,
    /// The declared payload encoding (RFC 08 §2, v1.5).
    pub encoding: Option<String>,
    /// `"forbidden"` forbids `*`-origin fan-out calls (RFC 05 §2.1); absent
    /// means unconstrained. Surfaced in the slice since 0.5 so *dynamic*
    /// callers (explorers) can refuse what generated builders make
    /// unspellable — the registry layer of the three-layer refusal.
    pub fanout: Option<String>,
    /// Whether the procedure declares itself idempotent (RFC 08 §2).
    pub idempotent: Option<bool>,
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
/// Note the asymmetry, which is pre-existing rather than introduced here:
/// `[[media]]` has had a registry field table since v1.3 and codegen since
/// v1.5, but has never appeared in a slice. Retrofitting it is separate work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobDecl {
    /// `artifact` | `tree` | `store` (RFC 07 §2).
    pub tier: String,
    /// The RFC 07 §2.2 endpoints served under `artifact/<id>/`; empty for the
    /// Tier-2 tiers, whose key *is* the endpoint.
    pub endpoints: Vec<String>,
    /// The `<algo>` chunk, on the `store` tier (RFC 07 §2.4).
    pub algo: Option<String>,
    /// The type conveying a reference to this blob — the payload that must
    /// carry the content root (RFC 07 §2.1).
    pub reference: Option<String>,
    /// The blob content's encoding, when declared.
    pub encoding: Option<String>,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaDecl {
    /// Media sub-path after `@media/<producer>/` — the stream pattern
    /// (`{stream}/preview/jpeg`), same variable rules as a subject.
    pub path: String,
    /// The wire `Encoding` on every frame (`image/jpeg`, `video/*` — may be
    /// a family): the codec is declared here, never in a payload envelope
    /// (RFC 07 §1).
    pub encoding: String,
    /// The per-frame sidecar type on the attachment (`FrameMeta`).
    pub attachment: Option<String>,
    /// Key-population bound, when the path carries variables.
    pub cardinality: Option<i64>,
    pub since: Option<String>,
    pub description: Option<String>,
}

/// One `[[deprecated]]` entry — RFC 08 §3's append-only retirement ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeprecationDecl {
    pub path: String,
    /// Registry version the retirement was recorded in.
    pub since: Option<String>,
    /// What replaced it, if anything.
    pub replaced_by: Option<String>,
}

/// What one build says it serves: the payload of an `introspect` reply.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    pub service_origin: Option<String>,
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
}

impl RegistrySlice {
    /// Subjects of one class.
    pub fn subjects_in(&self, class: &str) -> impl Iterator<Item = &SubjectDecl> {
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
    pub fn serves_blob_tier(&self, tier: &str) -> bool {
        self.blob.iter().any(|b| b.tier == tier)
    }
}

/// Why a slice would not parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceError(String);

impl fmt::Display for SliceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "malformed registry slice: {}", self.0)
    }
}

impl std::error::Error for SliceError {}

/// Parse an `introspect` reply — the raw registry TOML a build serves.
///
/// Deliberately tolerant of *unknown* keys (a newer fleet member may declare
/// fields this build has never heard of, and refusing to read the rest of its
/// slice would turn a forward-compatible addition into an outage of the very
/// view that exists to spot skew) and intolerant of *missing* ones (a slice
/// without a version cannot be diffed, which is the whole point).
pub fn parse_slice(toml_src: &str) -> Result<RegistrySlice, SliceError> {
    let doc: toml::Value = toml::from_str(toml_src).map_err(|e| SliceError(e.to_string()))?;

    let err = |m: &str| SliceError(m.to_string());
    let s = |v: Option<&toml::Value>| v.and_then(|v| v.as_str()).map(str::to_string);

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
            Some(s(svc.get("origin")).ok_or_else(|| err("[service] missing origin"))?),
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
            class: s(e.get("class")).ok_or_else(|| err("[[subject]] missing class"))?,
            type_name: s(e.get("type")).unwrap_or_default(),
            common: s(e.get("common")),
            since: s(e.get("since")),
            description: s(e.get("description")),
            qos: s(e.get("qos")),
            ttl_s: e.get("ttl_s").and_then(|v| v.as_integer()),
            unit: s(e.get("unit")),
            rate: s(e.get("rate")),
            cardinality: e.get("cardinality").and_then(|v| v.as_integer()),
            encoding: s(e.get("encoding")),
        });
    }

    let mut procedures = Vec::new();
    for e in array("procedure") {
        procedures.push(ProcedureDecl {
            path: s(e.get("path")).ok_or_else(|| err("[[procedure]] missing path"))?,
            kind: s(e.get("kind")).unwrap_or_default(),
            reply: s(e.get("reply")),
            request: s(e.get("request")),
            fanout: s(e.get("fanout")),
            idempotent: e.get("idempotent").and_then(|v| v.as_bool()),
            encoding: s(e.get("encoding")),
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
            tier: s(e.get("tier")).ok_or_else(|| err("[[blob]] missing tier"))?,
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
            encoding: s(e.get("encoding")),
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
            encoding: s(e.get("encoding")).ok_or_else(|| err("[[media]] missing encoding"))?,
            attachment: s(e.get("attachment")),
            cardinality: e.get("cardinality").and_then(|v| v.as_integer()),
            since: s(e.get("since")),
            description: s(e.get("description")),
        });
    }

    let mut deprecated = Vec::new();
    for e in array("deprecated") {
        deprecated.push(DeprecationDecl {
            path: s(e.get("path")).ok_or_else(|| err("[[deprecated]] missing path"))?,
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
            out.push_str(&format!("origin = {}\n", s(origin)));
        }
        None => {
            out.push_str("\n[producer]\n");
            out.push_str(&format!("name = {}\n", s(&slice.name)));
        }
    }
    opt(&mut out, "description", slice.description.as_deref());

    for d in &slice.subjects {
        out.push_str("\n[[subject]]\n");
        out.push_str(&format!("path = {}\n", s(&d.path)));
        out.push_str(&format!("class = {}\n", s(&d.class)));
        if !d.type_name.is_empty() {
            out.push_str(&format!("type = {}\n", s(&d.type_name)));
        }
        opt(&mut out, "common", d.common.as_deref());
        opt(&mut out, "qos", d.qos.as_deref());
        opt_int(&mut out, "ttl_s", d.ttl_s);
        opt(&mut out, "unit", d.unit.as_deref());
        opt(&mut out, "rate", d.rate.as_deref());
        opt_int(&mut out, "cardinality", d.cardinality);
        opt(&mut out, "encoding", d.encoding.as_deref());
        opt(&mut out, "since", d.since.as_deref());
        opt(&mut out, "description", d.description.as_deref());
    }

    for d in &slice.procedures {
        out.push_str("\n[[procedure]]\n");
        out.push_str(&format!("path = {}\n", s(&d.path)));
        if !d.kind.is_empty() {
            out.push_str(&format!("kind = {}\n", s(&d.kind)));
        }
        opt(&mut out, "request", d.request.as_deref());
        opt(&mut out, "reply", d.reply.as_deref());
        opt(&mut out, "encoding", d.encoding.as_deref());
        opt(&mut out, "fanout", d.fanout.as_deref());
        if let Some(i) = d.idempotent {
            out.push_str(&format!("idempotent = {i}\n"));
        }
        opt(&mut out, "since", d.since.as_deref());
        opt(&mut out, "description", d.description.as_deref());
    }

    for d in &slice.blob {
        out.push_str("\n[[blob]]\n");
        out.push_str(&format!("tier = {}\n", s(&d.tier)));
        if !d.endpoints.is_empty() {
            let items: Vec<String> = d.endpoints.iter().map(|e| s(e)).collect();
            out.push_str(&format!("endpoints = [{}]\n", items.join(", ")));
        }
        opt(&mut out, "algo", d.algo.as_deref());
        opt(&mut out, "reference", d.reference.as_deref());
        opt(&mut out, "encoding", d.encoding.as_deref());
        opt(&mut out, "since", d.since.as_deref());
        opt(&mut out, "description", d.description.as_deref());
    }

    for d in &slice.media {
        out.push_str("\n[[media]]\n");
        out.push_str(&format!("path = {}\n", s(&d.path)));
        out.push_str(&format!("encoding = {}\n", s(&d.encoding)));
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
        class: String,
    },
    /// We know a subject the host does not serve — it is older than us.
    MissingSubject {
        path: String,
        class: String,
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
        tier: String,
    },
    /// We know a `@blob` tier the host does not serve.
    MissingBlobTier {
        tier: String,
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
        if !local.serves_blob_tier(&b.tier) {
            out.push(SliceFinding::UnknownBlobTier {
                tier: b.tier.clone(),
            });
        }
    }
    for b in &local.blob {
        if !served.serves_blob_tier(&b.tier) {
            out.push(SliceFinding::MissingBlobTier {
                tier: b.tier.clone(),
            });
        }
    }
    // A host that still serves what its own ledger retired. Quiet until the
    // first deprecation lands, and exactly the question that needs SSH today.
    for d in &served.deprecated {
        if served.serves_subject(&d.path) || served.serves_procedure(&d.path) {
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
            [[subject]]
            path = "flows/{proto}/count"
            class = "telemetry"
            type = "TelemetryPoint"
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
            "#;
        let parsed = parse_slice(source).unwrap();
        let emitted = to_toml(&parsed);
        let back = parse_slice(&emitted)
            .unwrap_or_else(|e| panic!("exported TOML must re-parse: {e}\n---\n{emitted}"));
        assert_eq!(back, parsed, "exported TOML:\n{emitted}");
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
        assert_eq!(slice.service_origin.as_deref(), Some("@catalog"));
        assert!(slice.serves_procedure("introspect"));
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
        assert!(slice.serves_blob_tier("artifact"));
        let decl = slice.blob.iter().find(|b| b.tier == "artifact").unwrap();
        assert_eq!(decl.endpoints, ["manifest", "have"]);
        assert_eq!(decl.reference.as_deref(), Some("Delivery"));
        assert_eq!(decl.algo, None);
        // The unknown tier is kept, so a diff can surface it as skew.
        assert!(slice.serves_blob_tier("flux"));

        // `tier` itself is the one hard requirement.
        assert!(parse_slice(&format!("{header}\n[[blob]]\nalgo = \"blake3\"\n")).is_err());

        // Pre-v1.8 slices simply carry no blob entries.
        let old = parse_slice(header).unwrap();
        assert!(old.blob.is_empty());
        assert!(!old.serves_blob_tier("artifact"));
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
        assert!(
            findings
                .iter()
                .any(|f| matches!(f, SliceFinding::UnknownBlobTier { tier } if tier == "artifact"))
        );
        assert!(
            findings
                .iter()
                .any(|f| matches!(f, SliceFinding::MissingBlobTier { tier } if tier == "store"))
        );
        assert!(diff(&served, &served).is_empty());
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
