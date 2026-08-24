//! The node plane (RFC 04 §5): who is up, what they run, and which
//! deployment bases were seen at all.

use serde::Serialize;

/// One producer on one origin — row-shaped so a `--watch` loop can diff it
/// and a GUI can select it (`origin/producer` is the stable row identity).
#[derive(Debug, Clone, Serialize)]
pub struct NodeRow {
    pub origin: String,
    /// Live producer name (from the liveliness token; zero payload).
    pub producer: String,
    /// The producing app, when a registry slice joined (`--verbose`).
    /// `None` = not asked / no slice — never a default (O4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// Registry version from the joined slice, same provenance rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeList {
    pub nodes: Vec<NodeRow>,
    /// Whether a slice join was even attempted (`--verbose`) — keeps "asked,
    /// no slice served" distinguishable from "not asked" in rows whose
    /// `app`/`registry_version` are `None` (O4).
    pub slices_joined: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct BaseList {
    /// Discovered bases; `base` is a plain string, `""` for the empty base.
    pub bases: Vec<crate::DiscoveredBase>,
}

/// One producer's story on one node — the enrichment §6.3 promised
/// (issue #40). Every field is honest about its provenance: absent
/// introspection is `None`, never a default (RFC 09 §5.1 O4).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProducerInfo {
    pub name: String,
    /// A liveliness token stands (RFC 04 §5 — the only presence signal).
    pub alive: bool,
    /// From this origin's served introspect slice, when it answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry_version: Option<String>,
    pub subjects: usize,
    pub procedures: usize,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub blob_tiers: Vec<String>,
    /// Declared `@media` streams (RFC 08 §2/§6, v1.16 — the slice finally
    /// carries what §6 claimed through v1.7): discoverable off the bus, so
    /// a viewer can enumerate streams without a compiled-in registry.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub media: Vec<MediaStreamInfo>,
    /// Deprecated subjects this build still serves — RFC 08 §6's headline
    /// buy ("which hosts still serve a deprecated subject").
    pub deprecated_served: usize,
}

/// One declared media stream, as the slice states it (RFC 08 §2).
#[derive(Debug, Clone, serde::Serialize)]
pub struct MediaStreamInfo {
    /// The stream pattern after `@media/<producer>/`.
    pub path: String,
    /// The declared wire encoding (`image/jpeg`, `video/*`).
    pub encoding: String,
}

/// Freshness of one declared state subject on this node (RFC 04 §1.2).
#[derive(Debug, Clone, serde::Serialize)]
pub struct Freshness {
    pub producer: String,
    pub path: String,
    pub ttl_s: i64,
    /// Seconds since the newest matching sample's HLC stamp; `None` when no
    /// sample answered — which is "not seen", not "fresh" (O4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_s: Option<i64>,
    /// `age > ttl`, or no sample at all for a declared live subject.
    pub stale: bool,
}

/// One node, joined: liveliness × introspect × state freshness.
#[derive(Debug, Clone, serde::Serialize)]
pub struct NodeInfo {
    pub origin: String,
    pub producers: Vec<ProducerInfo>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub freshness: Vec<Freshness>,
}
