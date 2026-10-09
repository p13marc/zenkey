//! The admin plane (RFC 09 §5.1): routers, storages, declared entities and
//! the mesh topology — everything read out of Zenoh's own `@/**` adminspace
//! rather than off the keyspace. v1's state coverage and origin attachments
//! left with the v1 grammar they joined against (#612, FJ9); zk2's
//! instances join the topology by the session zid their descriptors state
//! (#705, [`InstanceJoin`]).

use serde::Serialize;

use super::asked::Asked;

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
    /// zk2's instances joined onto the routers (#705): absent when the join
    /// was not asked, never an empty list standing for it.
    #[serde(default, skip_serializing_if = "Asked::is_not_asked")]
    pub instances: Asked<InstanceJoin>,
}

/// zk2's instances joined onto the routers (#705, spec §3.3, §4.2).
///
/// Each instance is known by the session zid its descriptor states as
/// `meta.zid` (§3.3, a SHOULD since 0.10), and each router's admin document
/// lists its sessions (Appendix B). An instance is **attached** to the
/// routers whose document lists that zid, the two compared by value (0.11:
/// zenoh drops leading zeros). Only a **verified** router's list counts —
/// one whose own answer came from the router its key names, outward from
/// this session's (§4.2, 0.12–0.13), as the doctor verifies them — because
/// any session can answer under `@/<zid>/router`.
///
/// Every instance presence showed is reported, attached or not: an
/// instance no verified router lists is **unattached**, and one whose zid
/// could not be had is **unattributable**. None is omitted.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InstanceJoin {
    /// The deployment's namespace; empty for the bus root.
    pub namespace: String,
    /// The presence selector, base-relative in the namespace.
    pub selector: String,
    /// Whether the presence read ended at the routers' final reply. A read
    /// that ended at its timeout may have missed an instance (§8.1).
    pub complete: bool,
    /// The routers whose own answers were verified, by zid: the only
    /// session lists an instance is attached through.
    pub verified: Vec<String>,
    /// Answers that could not be shown to be a router's, one line each:
    /// what they list attaches nothing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unverified: Vec<String>,
    /// Set when the presence read could not be put on the bus: the join is
    /// then unobservable, and `instances` is empty for that reason alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unobservable: Option<String>,
    pub instances: Vec<InstanceAttachment>,
}

impl InstanceJoin {
    /// How many instances are attached, unattached and unattributable.
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut n = (0, 0, 0);
        for i in &self.instances {
            match i.attachment {
                Attachment::Attached { .. } => n.0 += 1,
                Attachment::Unattached { .. } => n.1 += 1,
                Attachment::Unattributable { .. } => n.2 += 1,
            }
        }
        n
    }
}

/// One instance, and where it is attached.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InstanceAttachment {
    /// `<system>/<service>`.
    pub address: String,
    pub instance: String,
    /// The descriptor's `meta.zid`, as written. Absent when the descriptor
    /// did not read or names none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zid: Option<String>,
    /// Tagged `attachment`.
    #[serde(flatten)]
    pub attachment: Attachment,
}

/// Where an instance's session is attached. Tagged `attachment`; each case
/// is distinct in every medium.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "attachment", rename_all = "snake_case")]
pub enum Attachment {
    /// A verified router's document lists the instance's zid among its
    /// sessions.
    Attached { routers: Vec<AttachedTo> },
    /// No verified router lists the zid: the reason says what was read.
    Unattached { reason: String },
    /// The zid could not be had: no descriptor read, or it names none.
    Unattributable { reason: String },
}

/// One router an instance is attached to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AttachedTo {
    /// The router's zid, as its admin key spells it.
    pub router: String,
    /// The session's `whatami` as the router lists it (`client`, `peer`),
    /// or `self` when the instance's session is the router itself.
    pub listed_as: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `admin graph`'s join is a wire contract (#705): each attachment is
    /// tagged apart — attached with its routers, unattached and
    /// unattributable each with its reason — the zid is absent when none
    /// could be had, and the join is absent from a topology that did not
    /// ask for it, never an empty list.
    #[test]
    fn the_instance_join_pins_its_shape() {
        let join = InstanceJoin {
            namespace: "acme".into(),
            selector: "zk2/*/*/@zk/**".into(),
            complete: true,
            verified: vec!["aabbccdd".into()],
            unverified: vec![],
            unobservable: None,
            instances: vec![
                InstanceAttachment {
                    address: "host-a/tc".into(),
                    instance: "3fa9c2d41b7e0012".into(),
                    zid: Some("ab12".into()),
                    attachment: Attachment::Attached {
                        routers: vec![AttachedTo {
                            router: "aabbccdd".into(),
                            listed_as: "client".into(),
                        }],
                    },
                },
                InstanceAttachment {
                    address: "host-b/tc".into(),
                    instance: "3fa9c2d41b7e0013".into(),
                    zid: Some("cd34".into()),
                    attachment: Attachment::Unattached { reason: "r".into() },
                },
                InstanceAttachment {
                    address: "host-c/tc".into(),
                    instance: "3fa9c2d41b7e0014".into(),
                    zid: None,
                    attachment: Attachment::Unattributable { reason: "u".into() },
                },
            ],
        };
        assert_eq!(join.counts(), (1, 1, 1));
        assert_eq!(
            serde_json::to_value(&join).unwrap(),
            json!({
                "namespace": "acme",
                "selector": "zk2/*/*/@zk/**",
                "complete": true,
                "verified": ["aabbccdd"],
                "instances": [
                    {"address": "host-a/tc", "instance": "3fa9c2d41b7e0012", "zid": "ab12",
                     "attachment": "attached",
                     "routers": [{"router": "aabbccdd", "listed_as": "client"}]},
                    {"address": "host-b/tc", "instance": "3fa9c2d41b7e0013", "zid": "cd34",
                     "attachment": "unattached", "reason": "r"},
                    {"address": "host-c/tc", "instance": "3fa9c2d41b7e0014",
                     "attachment": "unattributable", "reason": "u"},
                ],
            })
        );
        let topology = TopologyReport {
            nodes: vec![],
            edges: vec![],
            asked: "@/*/*".into(),
            answered: 0,
            self_zid: "ff".into(),
            instances: Asked::NotAsked,
        };
        assert!(
            serde_json::to_value(&topology)
                .unwrap()
                .get("instances")
                .is_none(),
            "not asked is absence"
        );
    }
}
