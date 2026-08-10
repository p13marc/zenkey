//! The node roster model (#61): per-producer presence per RFC 04 §5, where
//! the liveliness token *key* is the record.
//!
//! Presence semantics are GDBusProxy-style: a token retraction marks the
//! producer **suspect immediately** — never aged out — and a reappearance
//! recovers it. `@catalog` is tracked like any origin but surfaced by name,
//! because "catalog dead" and "no entities" must never look alike (D4).
//!
//! Laziness (the epic ground rule): everything here is fed by liveliness
//! (zero payload) plus the already-in-memory tree snapshot for *watched*
//! producers. The model itself never touches the bus.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use zenkey_fleet::KeyTreeSnapshot;

/// One producer's presence on one origin.
#[derive(Debug, Clone, Default)]
pub struct ProducerPresence {
    pub alive: bool,
    /// Set the instant a retraction was applied; cleared on reappearance.
    pub suspect_since: Option<Instant>,
    /// Age of the newest state sample under this producer — present only
    /// when the producer's state subtree is covered by an active watch
    /// (zero bus cost: read from the tick's tree snapshot). `None` =
    /// not watched or no sample = unknown, never "fresh" (O4).
    pub last_state_age: Option<Duration>,
    /// Whether any active watch selector covers this producer's state
    /// subtree — what makes `last_state_age`'s absence legible.
    pub watched: bool,
}

/// What the catalog line says. A dedicated type so the pane cannot fold
/// "no token observed" into an empty node list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogPresence {
    /// No `@catalog` token has been observed — not a verdict that no
    /// catalog exists (RFC 05 §3.1).
    NoTokenObserved,
    Alive,
    Suspect,
}

/// The roster: origin → producer → presence.
#[derive(Debug, Default)]
pub struct NodeRoster {
    nodes: BTreeMap<String, BTreeMap<String, ProducerPresence>>,
    /// Whether any presence source has reported yet — "not asked" is not
    /// "empty fleet" (O4).
    seeded: bool,
}

impl NodeRoster {
    /// Seed from a one-shot liveliness roster (`build_skeleton`'s GET):
    /// everything listed is alive; a re-seen origin recovers from suspect.
    pub fn seed(&mut self, roster: &BTreeMap<String, Vec<String>>) {
        self.seeded = true;
        for (origin, producers) in roster {
            let node = self.nodes.entry(origin.clone()).or_default();
            for producer in producers {
                let p = node.entry(producer.clone()).or_default();
                p.alive = true;
                p.suspect_since = None;
            }
        }
    }

    /// Apply one tick's liveliness transitions **in arrival order** — a
    /// down→up flap within one tick nets alive, up→down nets suspect.
    /// A retraction for a producer never seen still records a suspect row:
    /// the token existed, whatever we missed before.
    pub fn apply_transitions(&mut self, base: &str, transitions: &[(String, bool)], now: Instant) {
        for (key, up) in transitions {
            let Some(parsed) = zenkey::grammar::parse_full(base, key) else {
                continue;
            };
            self.seeded = true;
            let origin = parsed.origin.chunk().to_string();
            // `@catalog`'s token has no producer chunk — the service is the
            // producer (same fallback as the engine's roster()).
            let producer = parsed
                .producer
                .as_ref()
                .map(|p| p.chunk())
                .unwrap_or_else(|| origin.trim_start_matches('@').to_string());
            let presence = self
                .nodes
                .entry(origin)
                .or_default()
                .entry(producer)
                .or_default();
            if *up {
                presence.alive = true;
                presence.suspect_since = None;
            } else {
                presence.alive = false;
                presence.suspect_since = Some(now);
            }
        }
    }

    /// The watched-only freshness join: for each producer whose state
    /// subtree an active watch covers, read the newest sample age from the
    /// tick's tree snapshot. Purely in-memory.
    pub fn refresh(
        &mut self,
        tree: &KeyTreeSnapshot,
        base: &str,
        watched: &[String],
        now: Instant,
    ) {
        for (origin, producers) in &mut self.nodes {
            for (producer, presence) in producers.iter_mut() {
                let subtree = state_subtree(base, origin, producer);
                presence.watched = watched.iter().any(|w| intersects(w, &subtree));
                presence.last_state_age = if presence.watched {
                    let chunks: Vec<&str> = subtree.split('/').take_while(|c| *c != "**").collect();
                    tree.node(&chunks)
                        .and_then(|n| n.subtree_last_seen)
                        .map(|seen| now.saturating_duration_since(seen))
                } else {
                    None
                };
            }
        }
    }

    /// Base change / reconnect: back to "not asked".
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.seeded = false;
    }

    pub fn is_seeded(&self) -> bool {
        self.seeded
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Origins in order, catalog included (the pane surfaces it separately
    /// too, via [`catalog`](Self::catalog)).
    pub fn iter(&self) -> impl Iterator<Item = (&String, &BTreeMap<String, ProducerPresence>)> {
        self.nodes.iter()
    }

    pub fn get(&self, origin: &str) -> Option<&BTreeMap<String, ProducerPresence>> {
        self.nodes.get(origin)
    }

    /// Origin → live producers, in the shape `zenkey_fleet::roster` returns —
    /// for joins that need "who is actually up".
    ///
    /// `None` while unseeded, and that is the whole point of the method: a
    /// caller must be able to tell "no presence source has reported" from "the
    /// fleet is empty" (RFC 09 §5.1 O4), and a `BTreeMap` alone cannot say it.
    pub fn live_map(&self) -> Option<BTreeMap<String, Vec<String>>> {
        if !self.seeded {
            return None;
        }
        Some(
            self.nodes
                .iter()
                .map(|(origin, producers)| {
                    (
                        origin.clone(),
                        producers
                            .iter()
                            .filter(|(_, p)| p.alive)
                            .map(|(name, _)| name.clone())
                            .collect(),
                    )
                })
                .collect(),
        )
    }

    /// The catalog's own presence, by name — never folded into "no nodes".
    pub fn catalog(&self) -> CatalogPresence {
        match self
            .nodes
            .get("@catalog")
            .and_then(|producers| producers.values().next())
        {
            None => CatalogPresence::NoTokenObserved,
            Some(p) if p.alive => CatalogPresence::Alive,
            Some(_) => CatalogPresence::Suspect,
        }
    }
}

/// The display-path selector of one producer's state subtree — assembled
/// from grammar constants like `scope::catalog_subtree`, never `format!` of
/// user input (`origin`/`producer` come from parsed token keys).
fn state_subtree(base: &str, origin: &str, producer: &str) -> String {
    let tail = if origin.starts_with('@') {
        // A service origin has no producer chunk under state.
        format!("v1/{origin}/state/**")
    } else {
        format!("v1/{origin}/state/{producer}/**")
    };
    zenkey::grammar::with_base(base, tail)
}

fn intersects(a: &str, b: &str) -> bool {
    match (
        zenoh::key_expr::KeyExpr::new(a),
        zenoh::key_expr::KeyExpr::new(b),
    ) {
        (Ok(a), Ok(b)) => a.intersects(&b),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "v1/h-3fa9c2d41b7e/state/sysinfo/alive";
    const CATALOG_TOKEN: &str = "v1/@catalog/state/alive";

    fn now() -> Instant {
        Instant::now()
    }

    #[test]
    fn seed_marks_alive_and_reseed_recovers_suspect() {
        let mut roster = NodeRoster::default();
        assert!(!roster.is_seeded(), "not asked yet");

        let mut seed = BTreeMap::new();
        seed.insert("h-3fa9c2d41b7e".to_string(), vec!["sysinfo".to_string()]);
        roster.seed(&seed);
        assert!(roster.is_seeded());
        let p = &roster.get("h-3fa9c2d41b7e").unwrap()["sysinfo"];
        assert!(p.alive && p.suspect_since.is_none());

        roster.apply_transitions("", &[(TOKEN.to_string(), false)], now());
        let p = &roster.get("h-3fa9c2d41b7e").unwrap()["sysinfo"];
        assert!(
            !p.alive && p.suspect_since.is_some(),
            "retraction ⇒ suspect"
        );

        roster.seed(&seed);
        let p = &roster.get("h-3fa9c2d41b7e").unwrap()["sysinfo"];
        assert!(
            p.alive && p.suspect_since.is_none(),
            "reappearance recovers"
        );
    }

    /// Transitions apply in arrival order: what nets out inside one tick is
    /// the last word, not the first.
    #[test]
    fn flaps_within_one_tick_resolve_in_order() {
        let mut roster = NodeRoster::default();
        roster.apply_transitions(
            "",
            &[(TOKEN.to_string(), false), (TOKEN.to_string(), true)],
            now(),
        );
        assert!(roster.get("h-3fa9c2d41b7e").unwrap()["sysinfo"].alive);

        roster.apply_transitions(
            "",
            &[(TOKEN.to_string(), true), (TOKEN.to_string(), false)],
            now(),
        );
        let p = &roster.get("h-3fa9c2d41b7e").unwrap()["sysinfo"];
        assert!(!p.alive && p.suspect_since.is_some());
    }

    #[test]
    fn the_catalog_is_named_never_folded() {
        let mut roster = NodeRoster::default();
        assert_eq!(roster.catalog(), CatalogPresence::NoTokenObserved);

        roster.apply_transitions("", &[(CATALOG_TOKEN.to_string(), true)], now());
        assert_eq!(roster.catalog(), CatalogPresence::Alive);

        roster.apply_transitions("", &[(CATALOG_TOKEN.to_string(), false)], now());
        assert_eq!(roster.catalog(), CatalogPresence::Suspect);
    }

    /// The freshness join reaches only watched producers; everything else
    /// stays `None` — unknown, not fresh (O4).
    #[test]
    fn freshness_joins_watched_producers_only() {
        let mut roster = NodeRoster::default();
        roster.apply_transitions("", &[(TOKEN.to_string(), true)], now());

        let tree = KeyTreeSnapshot::default();
        roster.refresh(
            &tree,
            "",
            &["v1/h-3fa9c2d41b7e/state/**".to_string()],
            now(),
        );
        let p = &roster.get("h-3fa9c2d41b7e").unwrap()["sysinfo"];
        assert!(p.watched, "the selector covers this producer's subtree");
        assert!(p.last_state_age.is_none(), "no sample yet = unknown age");

        roster.refresh(&tree, "", &[], now());
        let p = &roster.get("h-3fa9c2d41b7e").unwrap()["sysinfo"];
        assert!(!p.watched);
    }
}
