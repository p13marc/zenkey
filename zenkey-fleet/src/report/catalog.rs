//! The `@catalog` documents as a **consumer** reads them (#389): entity,
//! alias and edge — other people's wire shapes, `Deserialize`d here so a
//! key-agnostic tool (a notifier, an exporter, a second UI) can run the
//! RFC 06 §5.1 join and the §5.6 impact walk from the documents alone.
//!
//! These are deliberately *readers*, not the catalog's own types: every
//! field the walk does not need is optional or ignored, an unknown edge
//! kind is carried rather than refused (the vocabulary is closed by
//! amendment, RFC 06 §5.6 rule 3, and a consumer built before the amendment
//! must not fall over on it), and the spelling is the reference profile's
//! (RFC 11 §3.3) — pinned by the tests below against verbatim documents.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One `@catalog/state/entity/{entity_id}` document (RFC 06 §5.1, §6.4).
///
/// `origins[]` is the normative bridge (self-reported origins only);
/// `host_id` is the older single-origin form a consumer falls back to when
/// a catalog predates the field. Everything else rides in `rest`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityDoc {
    pub entity_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub origins: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(flatten)]
    pub rest: BTreeMap<String, serde_json::Value>,
}

impl EntityDoc {
    /// Every origin this entity merges: `origins[]` first, then the legacy
    /// `host_id` when it is not already among them (RFC 06 §5.1 step 3).
    pub fn member_origins(&self) -> Vec<&str> {
        let mut out: Vec<&str> = self.origins.iter().map(String::as_str).collect();
        if let Some(h) = &self.host_id
            && !out.contains(&h.as_str())
        {
            out.push(h);
        }
        out
    }
}

/// One `@catalog/state/alias/{old_id}` document: a merged entity's old id
/// re-pointed at the survivor (RFC 06 §5.1 step 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AliasDoc {
    pub old_id: String,
    pub entity_id: String,
    #[serde(flatten)]
    pub rest: BTreeMap<String, serde_json::Value>,
}

/// The closed edge-kind vocabulary (RFC 06 §5.6 rule 3, spelled by RFC 11
/// §3.3) — plus [`EdgeKind::Other`], which carries a token this build does
/// not know so the document is still readable. An unknown kind never
/// propagates: a consumer that guessed a causal direction for a kind it
/// cannot name would be inventing structure.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// A hypervisor node hosts a guest (containment).
    Hosts,
    /// A host runs a container (containment).
    Runs,
    /// `from` is the gateway for `to` (containment).
    GatewayOf,
    /// `from` is a vantage point checking `to` (containment).
    Probes,
    /// The two share a link-layer segment — symmetric, **inert**.
    L2Adjacent,
    /// A token outside this build's vocabulary, carried verbatim.
    #[serde(untagged)]
    Other(String),
}

impl EdgeKind {
    /// Whether failure propagates `from → to` over this kind: true for the
    /// four containment kinds and nothing else (RFC 11 §3.3's containment
    /// column; `l2_adjacent` is the reason the column exists).
    pub fn propagates(&self) -> bool {
        matches!(
            self,
            EdgeKind::Hosts | EdgeKind::Runs | EdgeKind::GatewayOf | EdgeKind::Probes
        )
    }

    /// The wire token.
    pub fn as_str(&self) -> &str {
        match self {
            EdgeKind::Hosts => "hosts",
            EdgeKind::Runs => "runs",
            EdgeKind::GatewayOf => "gateway_of",
            EdgeKind::Probes => "probes",
            EdgeKind::L2Adjacent => "l2_adjacent",
            EdgeKind::Other(s) => s,
        }
    }
}

/// One resolved end of an edge (RFC 06 §5.6): an entity, or an honest
/// `External` for something the fleet observed but runs no sensor on.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeEnd {
    Entity {
        entity_id: String,
    },
    External {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ip: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mac: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

impl EdgeEnd {
    /// The entity id, when this end resolved to one.
    pub fn entity_id(&self) -> Option<&str> {
        match self {
            EdgeEnd::Entity { entity_id } => Some(entity_id),
            EdgeEnd::External { .. } => None,
        }
    }
}

/// Who claimed an edge: one sensor on one origin.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EdgeObserver {
    pub sensor: String,
    pub origin: String,
}

/// One `@catalog/state/edge/{edge_id}` document (RFC 06 §5.6). The id is
/// opaque (rule 2): the ends are read from here, never from the key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdgeDoc {
    pub edge_id: String,
    pub kind: EdgeKind,
    pub from: EdgeEnd,
    pub to: EdgeEnd,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observers: Vec<EdgeObserver>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The RFC 11 §3.3 test vector, verbatim: the pve node hosts the guest.
    /// Externally tagged ends, snake_case kinds, optional fields absent.
    #[test]
    fn an_edge_document_reads_with_the_profiles_spelling() {
        let doc = r#"{
            "edge_id": "e-2879d4667f9d946d",
            "kind": "hosts",
            "from": {"entity": {"entity_id": "h-3fa9c2d41b7e"}},
            "to": {"entity": {"entity_id": "h-9d02aa17c44f"}},
            "attrs": {"vmid": "101"},
            "observers": [{"sensor": "pve", "origin": "h-3fa9c2d41b7e"}],
            "last_updated": 1700000000
        }"#;
        let e: EdgeDoc = serde_json::from_str(doc).unwrap();
        assert_eq!(e.edge_id, "e-2879d4667f9d946d");
        assert_eq!(e.kind, EdgeKind::Hosts);
        assert!(e.kind.propagates());
        assert_eq!(e.from.entity_id(), Some("h-3fa9c2d41b7e"));
        assert_eq!(e.to.entity_id(), Some("h-9d02aa17c44f"));
        assert_eq!(e.attrs.get("vmid").map(String::as_str), Some("101"));
        assert_eq!(e.observers[0].sensor, "pve");
        assert_eq!(e.last_updated, Some(1_700_000_000));
        // Round-trip: what it reads is what it writes.
        let back: EdgeDoc = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back, e);

        // An external end and a probe: the minimal document.
        let probe: EdgeDoc = serde_json::from_str(
            r#"{"edge_id":"e-1","kind":"probes","from":{"entity":{"entity_id":"h-1"}},
                "to":{"external":{"ip":"1.1.1.1"}}}"#,
        )
        .unwrap();
        assert_eq!(probe.to.entity_id(), None);
        assert!(probe.attrs.is_empty() && probe.observers.is_empty());
        assert_eq!(probe.last_updated, None);
    }

    /// Every token of the closed vocabulary, its containment column, and an
    /// unknown token carried rather than refused — and never propagating.
    #[test]
    fn edge_kinds_spell_the_vocabulary_and_only_containment_propagates() {
        for (token, propagates) in [
            ("hosts", true),
            ("runs", true),
            ("gateway_of", true),
            ("probes", true),
            ("l2_adjacent", false),
            ("teleports", false),
        ] {
            let k: EdgeKind = serde_json::from_value(serde_json::json!(token)).unwrap();
            assert_eq!(k.as_str(), token);
            assert_eq!(k.propagates(), propagates, "{token}");
            assert_eq!(serde_json::to_value(&k).unwrap(), serde_json::json!(token));
        }
        assert_eq!(
            serde_json::from_value::<EdgeKind>(serde_json::json!("teleports")).unwrap(),
            EdgeKind::Other("teleports".into())
        );
    }

    /// An entity with `origins[]`, one without (an older catalog: `host_id`
    /// is the bridge), and an alias — the fields the join needs, the rest
    /// kept.
    #[test]
    fn entity_and_alias_documents_read_the_join_fields() {
        let e: EntityDoc = serde_json::from_str(
            r#"{"entity_id":"h-9d02aa17c44f","host_id":"h-9d02aa17c44f",
                "origins":["h-9d02aa17c44f","h-0000aa17c44f"],"hostname":"db01",
                "ips":["10.0.0.5"],"last_seen":1700000000}"#,
        )
        .unwrap();
        assert_eq!(
            e.member_origins(),
            vec!["h-9d02aa17c44f", "h-0000aa17c44f"],
            "host_id already among the origins is not repeated"
        );
        assert_eq!(e.hostname.as_deref(), Some("db01"));
        assert_eq!(e.rest["ips"], serde_json::json!(["10.0.0.5"]));

        let old: EntityDoc =
            serde_json::from_str(r#"{"entity_id":"ent-1","host_id":"h-3fa9c2d41b7e"}"#).unwrap();
        assert_eq!(old.member_origins(), vec!["h-3fa9c2d41b7e"]);
        assert!(old.origins.is_empty());

        let a: AliasDoc = serde_json::from_str(
            r#"{"old_id":"ent-1","entity_id":"ent-2","last_updated":1700000000}"#,
        )
        .unwrap();
        assert_eq!(
            (a.old_id.as_str(), a.entity_id.as_str()),
            ("ent-1", "ent-2")
        );
    }
}
