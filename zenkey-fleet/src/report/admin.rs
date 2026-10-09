//! The admin plane (RFC 09 §5.1): routers, storages, declared entities and
//! the mesh topology — everything read out of Zenoh's own `@/**` adminspace
//! rather than off the keyspace. v1's state coverage and origin attachments
//! left with the v1 grammar they joined against (#612, FJ9).

use serde::Serialize;

/// The storages the admin space answered for.
#[derive(Debug, Clone, Serialize)]
pub struct StorageList {
    pub storages: Vec<crate::StorageInfo>,
}

/// The routers the admin space answered for (#236).
///
/// Same argument as [`ScoutReport`](super::scout::ScoutReport): `[]` cannot distinguish a peer-only mesh
/// from an admin space that is disabled, and those are different facts about
/// the deployment. The selector that was actually asked rides with the answer.
#[derive(Debug, Clone, Serialize)]
pub struct RouterList {
    /// The admin selector put to the bus.
    pub asked: String,
    pub routers: Vec<crate::report::RouterInfo>,
}

/// A router (or peer) as the admin space reports it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RouterInfo {
    pub zid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub locators: Vec<String>,
    /// The full admin document, untrimmed — layouts vary by version.
    pub raw: serde_json::Value,
}

/// One configured storage, as the admin space reports it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StorageInfo {
    pub zid: String,
    pub name: String,
    /// The key expression the storage captures, when the layout exposes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_expr: Option<String>,
    /// The literal prefix stripped before the volume sees a key (RFC 09 §2 —
    /// zenoh requires a wildcard-free prefix here). Absent when the layout
    /// does not say, which is not the same as "none configured".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strip_prefix: Option<String>,
    /// The backing volume's id — `memory` is volatile and loses late-joiner
    /// seeds on a router restart, `fs`/`rocksdb` are the durable LWW stores
    /// (RFC 09 §2). Spelled either as a bare string or as `{ id: "fs", … }`
    /// depending on version; both are absorbed here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<String>,
    /// The full admin document, untrimmed — layouts vary by version.
    pub raw: serde_json::Value,
}

/// What kind of declared entity an admin reply describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Subscriber,
    Publisher,
    Queryable,
    Querier,
    Token,
}

/// One declared entity, as the admin space reports it: the reply key is
/// `@/<zid>/<whatami>/<kind>/<declared-keyexpr...>`, so the keyexpr every
/// session declared is readable **without subscribing to any data** — the
/// payload-free discovery leg of issue #84.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DeclaredEntity {
    pub kind: EntityKind,
    /// The declared key expression, verbatim.
    pub keyexpr: String,
    /// The node whose admin space answered.
    pub node_zid: String,
    /// The raw payload (`Sources { routers, peers, clients }`-shaped in
    /// zenoh 1.9) — kept as-is; layouts vary by version.
    pub sources: serde_json::Value,
}

/// The declared-entity sweep result.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DeclaredEntities {
    pub entities: Vec<DeclaredEntity>,
}

/// One link of the mesh, seen as undirected.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MeshLink {
    /// The lower zid of the pair — the ordering is arbitrary but stable, so a
    /// renderer can group without re-sorting.
    pub a: String,
    pub b: String,
    /// Both ends reported this link. Reciprocal reports are corroboration,
    /// not duplication, and the distinction is worth keeping: a link only one
    /// end mentions is weaker evidence than one both do.
    pub corroborated: bool,
    /// Endpoints as the **first** reporter described them. A second report's
    /// links are not merged: the two ends name the same link from opposite
    /// sides, and concatenating them would read as twice the links.
    pub links: Vec<String>,
}

/// One node of the mesh, as the topology join sees it (#118).
#[derive(Debug, Clone, serde::Serialize)]
pub struct TopologyNode {
    pub zid: String,
    /// `router` | `peer` | `client`, as the admin key (or a neighbour's
    /// session list) spells it.
    pub whatami: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Locators as the node's own root doc declares them. zenoh 1.10.0's
    /// root doc filters every loopback endpoint out of this list (upstream
    /// eclipse-zenoh/zenoh#2671, the loopback scouting fix:
    /// `get_locators()` → `get_locators_noloopback()`), so a loopback-only
    /// node declares `[]`; from 1.10.1 the filter covers only the loopback
    /// addresses an unspecified listener resolves to, and an explicit
    /// `127.0.0.1` listener is declared again.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub locators: Vec<String>,
    /// Endpoints corroborated from session links when the root doc
    /// declares no locators: addresses a live link actually used on this
    /// node's side (#155). Evidence of reachability, **not** a
    /// listen-endpoint claim — renderers label the provenance ("via
    /// session link") rather than folding these into `locators`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub locators_via_links: Vec<String>,
    /// `true` = this node's own admin space answered; `false` = only heard
    /// of via a neighbour's session list — "heard of, not queryable",
    /// rendered as such rather than omitted (the issue's honesty rule).
    pub answered: bool,
}

/// One reported link. Kept per-reporter — a renderer that wants an
/// undirected mesh dedups by unordered zid pair, and reciprocal reports
/// are corroboration, not duplication.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TopologyEdge {
    /// The zid whose admin doc reported this session.
    pub reporter: String,
    /// The far end's zid.
    pub peer: String,
    /// The far end's whatami, as the reporter says it.
    pub whatami: String,
    /// The session's region, verbatim as the reporter's admin doc states
    /// it (zenoh 1.10 session entries carry one, `"unknown"` included —
    /// the regions rework that landed over 1.9 "Longwang"). Absent on
    /// older fleets whose docs have no such field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// Link endpoints, `src -> dst`, protocol included.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
}

/// The mesh as the admin space answered it, joined with nothing invented
/// (#118): what answered, what was only mentioned, and who we are.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TopologyReport {
    pub nodes: Vec<TopologyNode>,
    pub edges: Vec<TopologyEdge>,
    /// The selector the sweep asked.
    pub asked: String,
    /// Root docs that answered. Zero is "the admin space did not answer" —
    /// a reading about reachability, never an empty mesh.
    pub answered: usize,
    /// This session's own zid — the "you are here" marker.
    pub self_zid: String,
}
