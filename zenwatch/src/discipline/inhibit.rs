//! Inhibition's inputs (#389, RFC 06 §5.6): the catalog's documents as
//! this daemon has seen them, and the two decisions the RFC leaves to the
//! consumer — which entity a notice is *about*, and which entities are
//! *down*.
//!
//! Three subscriptions, no application knowledge: `@catalog/state/edge/*`
//! (the resolved graph), `@catalog/state/entity/*` and
//! `@catalog/state/alias/*` (the origin → entity join, RFC 06 §5.1). The
//! shapes are the engine's readers ([`EdgeDoc`], [`EntityDoc`],
//! [`AliasDoc`]); the walk is the engine's [`zenkey_fleet::attribute`].
//! What lives here is only the bookkeeping: which key holds which document,
//! a tombstone forgets it, and a payload that does not parse is counted
//! rather than guessed at.

use std::collections::{BTreeMap, BTreeSet};

use zenkey_fleet::report::{AliasDoc, EdgeDoc, EntityDoc};
use zenoh::sample::SampleKind;

/// Which catalog family a key belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Entity,
    Alias,
    Edge,
}

/// The three selectors, base-relative — one per family, each seeded with
/// a GET on the same selector (storage-shaped: one reply per document).
pub const SELECTORS: [(&str, Family); 3] = [
    ("v1/@catalog/state/entity/*", Family::Entity),
    ("v1/@catalog/state/alias/*", Family::Alias),
    ("v1/@catalog/state/edge/*", Family::Edge),
];

/// Which family a wire key under `base` falls in, if any.
pub fn classify(base: &str, key: &str) -> Option<Family> {
    let parsed = zenkey::grammar::parse_full(base, key)?;
    if parsed.origin.chunk() != "@catalog" {
        return None;
    }
    // The catalog's state subjects sit directly under `state/`: the
    // service origin carries no producer chunk.
    let relative = zenkey::grammar::strip_base(base, key)?;
    let mut chunks = relative.split('/').skip(2); // v1, @catalog
    if chunks.next() != Some("state") {
        return None;
    }
    match chunks.next() {
        Some("entity") => Some(Family::Entity),
        Some("alias") => Some(Family::Alias),
        Some("edge") => Some(Family::Edge),
        _ => None,
    }
}

/// The catalog as observed: one document per key, forgotten on tombstone.
#[derive(Debug, Default)]
pub struct Catalog {
    entities: BTreeMap<String, EntityDoc>,
    aliases: BTreeMap<String, AliasDoc>,
    edges: BTreeMap<String, EdgeDoc>,
    /// Payloads that did not parse as their family's document — counted,
    /// because a catalog speaking a shape this build cannot read is a fact
    /// the health document should carry, not a silent hole in the graph.
    pub unparsed: u64,
}

impl Catalog {
    /// Apply one sample. Returns whether the graph or the join changed.
    pub fn apply(&mut self, family: Family, key: &str, kind: SampleKind, bytes: &[u8]) -> bool {
        if kind == SampleKind::Delete {
            return match family {
                Family::Entity => self.entities.remove(key).is_some(),
                Family::Alias => self.aliases.remove(key).is_some(),
                Family::Edge => self.edges.remove(key).is_some(),
            };
        }
        // JSON first, CBOR second: the reference catalog publishes either,
        // and the reader shapes are the same document.
        let Some(value) = zenkey_fleet::structural_value(bytes) else {
            self.unparsed += 1;
            return false;
        };
        let parsed = match family {
            Family::Entity => serde_json::from_value::<EntityDoc>(value)
                .map(|d| self.entities.insert(key.to_string(), d).is_none()),
            Family::Alias => serde_json::from_value::<AliasDoc>(value)
                .map(|d| self.aliases.insert(key.to_string(), d).is_none()),
            Family::Edge => serde_json::from_value::<EdgeDoc>(value)
                .map(|d| self.edges.insert(key.to_string(), d).is_none()),
        };
        match parsed {
            Ok(_) => true,
            Err(e) => {
                self.unparsed += 1;
                tracing::warn!(key, "catalog document did not parse: {e}");
                false
            }
        }
    }

    pub fn edges(&self) -> Vec<EdgeDoc> {
        self.edges.values().cloned().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.edges.is_empty() && self.entities.is_empty()
    }

    pub fn counts(&self) -> (usize, usize, usize) {
        (self.entities.len(), self.aliases.len(), self.edges.len())
    }

    /// The RFC 06 §5.1 join for one origin.
    pub fn entity_of(&self, origin: &str) -> Option<String> {
        let entities: Vec<EntityDoc> = self.entities.values().cloned().collect();
        let aliases: Vec<AliasDoc> = self.aliases.values().cloned().collect();
        zenkey_fleet::entity_of(origin, &entities, &aliases)
    }

    /// The consumer's downness decision: an entity is down when every
    /// member origin is in `gone` — origins whose alive tokens this daemon
    /// saw and saw retracted. An entity with no member origins is never
    /// down (silence is not evidence), and a member origin whose tokens
    /// were never seen keeps the entity up for the same reason.
    pub fn down_entities(&self, gone: &BTreeSet<String>) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for e in self.entities.values() {
            let members = e.member_origins();
            if !members.is_empty() && members.iter().all(|o| gone.contains(*o)) {
                let id = self.resolve_alias(&e.entity_id);
                out.insert(id);
            }
        }
        out
    }

    fn resolve_alias(&self, id: &str) -> String {
        self.aliases
            .values()
            .filter(|a| a.old_id == id)
            .map(|a| a.entity_id.clone())
            .min()
            .unwrap_or_else(|| id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_classify_by_family_under_the_base() {
        assert_eq!(
            classify("", "v1/@catalog/state/edge/e-2879d4667f9d946d"),
            Some(Family::Edge)
        );
        assert_eq!(
            classify("acme", "acme/v1/@catalog/state/entity/ent-1"),
            Some(Family::Entity)
        );
        assert_eq!(
            classify("", "v1/@catalog/state/alias/ent-0"),
            Some(Family::Alias)
        );
        assert_eq!(classify("", "v1/@catalog/state/incident/inc-1"), None);
        assert_eq!(classify("", "v1/h-3fa9c2d41b7e/state/pve/alert/x"), None);
        assert_eq!(
            classify("acme", "v1/@catalog/state/edge/e-1"),
            None,
            "wrong base"
        );
    }

    /// Documents in, a tombstone out, an unparsable payload counted; the
    /// join and the downness decision over what is left.
    #[test]
    fn the_catalog_joins_and_judges_down_from_documents_alone() {
        let mut c = Catalog::default();
        assert!(c.apply(
            Family::Entity,
            "v1/@catalog/state/entity/ent-a",
            SampleKind::Put,
            br#"{"entity_id":"ent-a","host_id":"h-aaaaaaaaaaaa","origins":["h-aaaaaaaaaaaa"]}"#,
        ));
        assert!(c.apply(
            Family::Entity,
            "v1/@catalog/state/entity/ent-b",
            SampleKind::Put,
            br#"{"entity_id":"ent-b","origins":["h-bbbbbbbbbbbb","h-cccccccccccc"]}"#,
        ));
        assert!(!c.apply(
            Family::Edge,
            "v1/@catalog/state/edge/e-bad",
            SampleKind::Put,
            b"\xff\xfe not a document",
        ));
        assert_eq!(c.unparsed, 1);
        assert_eq!(c.entity_of("h-aaaaaaaaaaaa").as_deref(), Some("ent-a"));
        assert_eq!(c.entity_of("h-cccccccccccc").as_deref(), Some("ent-b"));
        assert_eq!(c.entity_of("h-dddddddddddd"), None);

        let gone: BTreeSet<String> = ["h-aaaaaaaaaaaa", "h-bbbbbbbbbbbb"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            c.down_entities(&gone).into_iter().collect::<Vec<_>>(),
            vec!["ent-a".to_string()],
            "ent-b still has an origin whose token stands"
        );
        assert!(c.apply(
            Family::Entity,
            "v1/@catalog/state/entity/ent-a",
            SampleKind::Delete,
            b"",
        ));
        assert!(c.down_entities(&gone).is_empty());
    }
}
