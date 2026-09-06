//! Impact attribution — a pure function of the edge set, the firing set
//! and the down set (#389, RFC 06 §5.6), and the origin → entity join it
//! needs (RFC 06 §5.1). No session, no clock, no private state: a
//! notifier runs it on its tick, a `.zrec` replay runs it on a recording.
//!
//! - **Only containment kinds propagate** ([`EdgeKind::propagates`]);
//!   `l2_adjacent` and any unknown kind are inert.
//! - **A root is a down entity with no down containment ancestor**; every
//!   other affected site the walk reaches is a symptom carrying its
//!   nearest root.
//! - **The walk is bounded** — a depth cap and a visited set, because the
//!   graph is built from claims by independent sensors and a cycle is a
//!   reachable input — and what the bound cost is reported, never
//!   absorbed (RFC 13 §3 O6).
//! - **The output is ordered**: roots and symptoms by entity id, so two
//!   consumers with the same inputs render the same thing.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::report::{AliasDoc, EdgeDoc, EntityDoc, ImpactReport, Root, Symptom};

/// What [`attribute`] reads.
#[derive(Debug, Clone, Copy)]
pub struct ImpactInputs<'a> {
    /// The catalog's resolved edges, as observed.
    pub edges: &'a [EdgeDoc],
    /// Entities with a firing alert (or any firing notice) on them.
    pub firing: &'a BTreeSet<String>,
    /// Entities the consumer judged down — every member origin's alive
    /// token gone (RFC 06 §5.6: downness is the consumer's decision).
    pub down: &'a BTreeSet<String>,
}

/// The largest depth cap a consumer should accept: the reference
/// implementation's, and what a notifier's config check enforces.
pub const MAX_DEPTH_CAP: usize = 4;

type Adjacency<'a> = BTreeMap<&'a str, BTreeSet<&'a str>>;

/// Roots and symptoms over the containment graph, walking at most
/// `depth_cap` edges from any entity.
pub fn attribute(inputs: &ImpactInputs<'_>, depth_cap: usize) -> ImpactReport {
    let mut children: Adjacency<'_> = BTreeMap::new();
    let mut parents: Adjacency<'_> = BTreeMap::new();
    for e in inputs.edges {
        if !e.kind.propagates() {
            continue;
        }
        let (Some(from), Some(to)) = (e.from.entity_id(), e.to.entity_id()) else {
            // An `External` end has no liveliness to lose and cannot be in
            // `down`: an edge touching one neither carries nor roots impact.
            continue;
        };
        if from == to {
            // A self-edge would make every entity its own ancestor.
            continue;
        }
        children.entry(from).or_default().insert(to);
        parents.entry(to).or_default().insert(from);
    }

    let mut walks_capped = 0u64;
    let mut cycles_seen = 0u64;

    // Roots: down, with no down ancestor within the cap.
    let roots: Vec<&str> = inputs
        .down
        .iter()
        .map(String::as_str)
        .filter(|id| !has_down_ancestor(id, &parents, inputs.down, depth_cap, &mut walks_capped))
        .collect();

    // Nearest root wins; ties by root id ascending (roots iterate sorted,
    // and a strictly nearer candidate is the only thing that replaces one).
    let mut best: BTreeMap<&str, (usize, &str)> = BTreeMap::new();
    let mut out_roots = Vec::with_capacity(roots.len());
    for root in &roots {
        let reach = descendants(
            root,
            &children,
            depth_cap,
            &mut walks_capped,
            &mut cycles_seen,
        );
        for (id, depth) in &reach {
            let cand = (*depth, *root);
            best.entry(id)
                .and_modify(|cur| {
                    if cand < *cur {
                        *cur = cand;
                    }
                })
                .or_insert(cand);
        }
        out_roots.push(Root {
            entity: (*root).to_string(),
            reached: reach.keys().map(|s| (*s).to_string()).collect(),
        });
    }

    // Symptoms: every firing or down site a root reaches that is not itself
    // a root. A down entity under a down host is as much a symptom as an
    // alert there — "vm-apps is gone", not nine messages about what it ran.
    let mut symptoms = Vec::new();
    for site in inputs.firing.iter().chain(inputs.down.iter()) {
        let id = site.as_str();
        if roots.contains(&id) {
            continue;
        }
        if let Some((_, root)) = best.get(id) {
            symptoms.push(Symptom {
                entity: id.to_string(),
                explained_by: (*root).to_string(),
            });
        }
    }
    symptoms.sort();
    symptoms.dedup();

    ImpactReport {
        roots: out_roots,
        symptoms,
        walks_capped,
        cycles_seen,
        depth_cap,
    }
}

/// Whether any containment ancestor within `depth_cap` is itself down.
fn has_down_ancestor(
    id: &str,
    parents: &Adjacency<'_>,
    down: &BTreeSet<String>,
    depth_cap: usize,
    walks_capped: &mut u64,
) -> bool {
    let mut seen: BTreeSet<&str> = BTreeSet::from([id]);
    let mut queue: VecDeque<(&str, usize)> = VecDeque::from([(id, 0usize)]);
    let mut capped = false;
    while let Some((cur, depth)) = queue.pop_front() {
        let Some(next) = parents.get(cur) else {
            continue;
        };
        if depth >= depth_cap {
            if next.iter().any(|p| !seen.contains(p)) {
                capped = true;
            }
            continue;
        }
        for p in next {
            if !seen.insert(p) {
                continue;
            }
            if down.contains(*p) {
                return true;
            }
            queue.push_back((p, depth + 1));
        }
    }
    if capped {
        *walks_capped += 1;
    }
    false
}

/// Every entity reachable downstream within `depth_cap`, with the shortest
/// depth at which it was reached. The root is excluded; an edge back to it
/// is a cycle, counted once per walk.
fn descendants<'a>(
    root: &'a str,
    children: &Adjacency<'a>,
    depth_cap: usize,
    walks_capped: &mut u64,
    cycles_seen: &mut u64,
) -> BTreeMap<&'a str, usize> {
    let mut out: BTreeMap<&str, usize> = BTreeMap::new();
    let mut seen: BTreeSet<&str> = BTreeSet::from([root]);
    let mut queue: VecDeque<(&str, usize)> = VecDeque::from([(root, 0usize)]);
    let (mut capped, mut cycled) = (false, false);
    while let Some((cur, depth)) = queue.pop_front() {
        let Some(next) = children.get(cur) else {
            continue;
        };
        if depth >= depth_cap {
            if next.iter().any(|c| !seen.contains(c)) {
                capped = true;
            }
            continue;
        }
        for c in next {
            if *c == root {
                cycled = true;
                continue;
            }
            if !seen.insert(c) {
                continue;
            }
            out.insert(c, depth + 1);
            queue.push_back((c, depth + 1));
        }
    }
    if capped {
        *walks_capped += 1;
    }
    if cycled {
        *cycles_seen += 1;
    }
    out
}

/// The RFC 06 §5.1 join, origin → entity: `origins[]` first, the legacy
/// `host_id` second, then one alias hop so a merged entity resolves to its
/// survivor. Deterministic under input order: the smallest matching entity
/// id wins when the documents disagree.
pub fn entity_of(origin: &str, entities: &[EntityDoc], aliases: &[AliasDoc]) -> Option<String> {
    let by_origins = entities
        .iter()
        .filter(|e| e.origins.iter().any(|o| o == origin))
        .map(|e| e.entity_id.as_str())
        .min();
    let by_host_id = || {
        entities
            .iter()
            .filter(|e| e.host_id.as_deref() == Some(origin))
            .map(|e| e.entity_id.as_str())
            .min()
    };
    let id = by_origins.or_else(by_host_id)?;
    let resolved = aliases
        .iter()
        .filter(|a| a.old_id == id)
        .map(|a| a.entity_id.as_str())
        .min()
        .unwrap_or(id);
    Some(resolved.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{EdgeEnd, EdgeKind};

    fn edge(kind: EdgeKind, from: &str, to: &str) -> EdgeDoc {
        EdgeDoc {
            edge_id: format!("e-{}-{from}-{to}", kind.as_str()),
            kind,
            from: EdgeEnd::Entity {
                entity_id: from.into(),
            },
            to: EdgeEnd::Entity {
                entity_id: to.into(),
            },
            attrs: BTreeMap::new(),
            observers: Vec::new(),
            last_updated: None,
        }
    }

    fn set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    /// `A hosts B runs C`, A down, an alert on C: A is the root, C a
    /// symptom explained by A — and B, merely reached, is not a symptom of
    /// anything because nothing fires there.
    #[test]
    fn a_chain_attributes_the_leaf_alert_to_the_down_host() {
        let edges = [
            edge(EdgeKind::Hosts, "A", "B"),
            edge(EdgeKind::Runs, "B", "C"),
        ];
        let r = attribute(
            &ImpactInputs {
                edges: &edges,
                firing: &set(&["C"]),
                down: &set(&["A"]),
            },
            MAX_DEPTH_CAP,
        );
        assert_eq!(
            r.roots,
            vec![Root {
                entity: "A".into(),
                reached: vec!["B".into(), "C".into()]
            }]
        );
        assert_eq!(
            r.symptoms,
            vec![Symptom {
                entity: "C".into(),
                explained_by: "A".into()
            }]
        );
        assert_eq!((r.walks_capped, r.cycles_seen, r.depth_cap), (0, 0, 4));

        // B down as well: B is a symptom of A, not a second root.
        let r = attribute(
            &ImpactInputs {
                edges: &edges,
                firing: &set(&["C"]),
                down: &set(&["A", "B"]),
            },
            MAX_DEPTH_CAP,
        );
        assert_eq!(r.roots.len(), 1);
        assert_eq!(
            r.symptoms
                .iter()
                .map(|s| s.entity.as_str())
                .collect::<Vec<_>>(),
            vec!["B", "C"]
        );
    }

    /// A symmetric kind never propagates: two neighbours, one down, the
    /// other's alert stays a cause of its own. Nor does an unknown kind, or
    /// an edge into an `External`.
    #[test]
    fn l2_adjacent_and_unknown_kinds_never_propagate() {
        let mut edges = vec![
            edge(EdgeKind::L2Adjacent, "A", "B"),
            edge(EdgeKind::Other("teleports".into()), "A", "C"),
        ];
        edges.push(EdgeDoc {
            to: EdgeEnd::External {
                ip: Some("1.1.1.1".into()),
                mac: None,
                name: None,
            },
            ..edge(EdgeKind::Probes, "A", "x")
        });
        let r = attribute(
            &ImpactInputs {
                edges: &edges,
                firing: &set(&["B", "C"]),
                down: &set(&["A"]),
            },
            MAX_DEPTH_CAP,
        );
        assert_eq!(r.roots[0].reached, Vec::<String>::new());
        assert!(r.symptoms.is_empty(), "{:?}", r.symptoms);
    }

    /// Two hosts each claiming to host the other: the walk terminates, and
    /// says it saw the cycle.
    #[test]
    fn a_cycle_terminates_and_is_counted_once() {
        let edges = [
            edge(EdgeKind::Hosts, "A", "B"),
            edge(EdgeKind::Hosts, "B", "A"),
        ];
        let r = attribute(
            &ImpactInputs {
                edges: &edges,
                firing: &set(&["B"]),
                down: &set(&["A"]),
            },
            MAX_DEPTH_CAP,
        );
        assert_eq!(r.cycles_seen, 1);
        assert_eq!(r.roots[0].entity, "A");
        assert_eq!(r.symptoms[0].explained_by, "A");
    }

    /// A chain five deep under the cap of four: the walk stops at four,
    /// reports the cut, and the fifth is not a symptom of anything.
    #[test]
    fn a_walk_past_the_depth_cap_is_cut_and_reported() {
        let edges = [
            edge(EdgeKind::Hosts, "A", "B"),
            edge(EdgeKind::Runs, "B", "C"),
            edge(EdgeKind::Runs, "C", "D"),
            edge(EdgeKind::Runs, "D", "E"),
            edge(EdgeKind::Runs, "E", "F"),
        ];
        let r = attribute(
            &ImpactInputs {
                edges: &edges,
                firing: &set(&["E", "F"]),
                down: &set(&["A"]),
            },
            4,
        );
        assert_eq!(r.walks_capped, 1);
        assert_eq!(r.roots[0].reached, vec!["B", "C", "D", "E"]);
        assert_eq!(
            r.symptoms
                .iter()
                .map(|s| s.entity.as_str())
                .collect::<Vec<_>>(),
            vec!["E"]
        );
    }

    /// The same inputs in any order render the same report; the nearest
    /// root wins, ties by id.
    #[test]
    fn the_output_is_deterministic_under_input_permutation() {
        let edges = [
            edge(EdgeKind::Hosts, "H2", "V"),
            edge(EdgeKind::Hosts, "H1", "V"),
            edge(EdgeKind::GatewayOf, "G", "H1"),
            edge(EdgeKind::Runs, "V", "C"),
        ];
        let inputs = |edges: &[EdgeDoc]| {
            attribute(
                &ImpactInputs {
                    edges,
                    firing: &set(&["C", "V"]),
                    down: &set(&["H2", "H1", "G"]),
                },
                MAX_DEPTH_CAP,
            )
        };
        let a = inputs(&edges);
        let mut reversed = edges.to_vec();
        reversed.reverse();
        let b = inputs(&reversed);
        assert_eq!(a, b);
        assert_eq!(
            a.roots
                .iter()
                .map(|r| r.entity.as_str())
                .collect::<Vec<_>>(),
            vec!["G", "H2"],
            "H1 is under G; roots are ordered"
        );
        let v = a.symptoms.iter().find(|s| s.entity == "V").unwrap();
        assert_eq!(v.explained_by, "H2", "H2 is one hop away, G is two");
    }

    /// `origins[]` first, `host_id` second, one alias hop — and nothing for
    /// an origin no document names.
    #[test]
    fn entity_of_joins_by_origins_then_host_id_then_alias() {
        let entities = [
            EntityDoc {
                entity_id: "ent-new".into(),
                origins: vec!["h-aaaaaaaaaaaa".into()],
                host_id: None,
                hostname: None,
                rest: BTreeMap::new(),
            },
            EntityDoc {
                entity_id: "ent-old".into(),
                origins: vec![],
                host_id: Some("h-bbbbbbbbbbbb".into()),
                hostname: None,
                rest: BTreeMap::new(),
            },
        ];
        let aliases = [AliasDoc {
            old_id: "ent-old".into(),
            entity_id: "ent-merged".into(),
            rest: BTreeMap::new(),
        }];
        assert_eq!(
            entity_of("h-aaaaaaaaaaaa", &entities, &aliases).as_deref(),
            Some("ent-new")
        );
        assert_eq!(
            entity_of("h-bbbbbbbbbbbb", &entities, &aliases).as_deref(),
            Some("ent-merged"),
            "host_id match, then the alias re-points it"
        );
        assert_eq!(entity_of("h-cccccccccccc", &entities, &aliases), None);
    }
}
