//! The admin plane (RFC 09 §5.1): routers, storages, declared entities,
//! state coverage and the mesh topology — everything read out of Zenoh's own
//! `@/**` adminspace rather than off the keyspace.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct StorageList {
    pub storages: Vec<crate::StorageInfo>,
    pub coverage: Vec<crate::CoverageRow>,
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

/// How a declared state family relates to the configured storages.
///
/// `rename_all` is not decoration: without it this enum inherited Rust's
/// variant spelling and serialized `"Covered"` while every other vocabulary in
/// the report surface — `TopicVerdict`, `DoctorSeverity`, `CutoverVerdict`,
/// `ExpectVerdict` — was snake_case (#232). A consumer could not learn the
/// file's conventions from one document and apply them to the next.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "coverage", content = "storage", rename_all = "snake_case")]
pub enum Coverage {
    /// Some storage's key expression includes every key of the family.
    Covered(String),
    /// A storage overlaps the family but does not include all of it.
    Partial(String),
    /// No storage touches the family. For volatile (ttl'd) state this can be
    /// legitimate — advanced-pub/sub cache seeding (RFC 04 §3.5); storage is
    /// authoritative for durable data.
    Uncovered,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CoverageRow {
    pub producer: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_s: Option<i64>,
    #[serde(flatten)]
    pub coverage: Coverage,
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

/// One liveliness origin attached to the session that declared its token —
/// the #131 join, evidence-first: an attachment is made only from what the
/// admin space actually said, never guessed (a guessed attachment would be
/// the O4 failure on a picture).
#[derive(Debug, Clone, serde::Serialize)]
pub struct OriginAttachment {
    /// The origin the token names (`h-…` or `@service`).
    pub origin: String,
    /// The declaring session's zid, when the token's admin `sources` names
    /// exactly one. `None` = the sources were absent or ambiguous — the
    /// origin is then only *reported by* the answering admin space, and a
    /// renderer says so instead of drawing a line it cannot back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_zid: Option<String>,
    /// The admin space that reported the token: the origin's own session in
    /// a peer mesh serving its admin space, a router in a routed one.
    pub reporter_zid: String,
    /// The token key the evidence rode — the audit trail.
    pub token_key: String,
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
    /// Locators as the node's own root doc declares them. Since zenoh
    /// 1.10.0 the root doc filters loopback endpoints out of this list
    /// (upstream eclipse-zenoh/zenoh#2671, the loopback scouting fix:
    /// `get_locators()` → `get_locators_noloopback()`) — deliberate, so a
    /// loopback-only node honestly declares `[]` here.
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
