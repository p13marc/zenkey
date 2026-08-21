//! The declared keyspace, built without subscribing to anything (issue #84).
//!
//! A Zenoh subscription cannot be "metadata only" — once declared, payloads
//! flow. So a lazy explorer's tree starts from what costs (almost) nothing:
//!
//! - **registry slices** → the *declared* subject families per producer,
//!   with `{var}` positions kept symbolic;
//! - **the liveliness roster** → which origins run which producers
//!   (zero-payload by construction, RFC 04 §5);
//! - **admin-space declared entities** ([`crate::declared_entities`]) →
//!   keyexprs sessions actually declared, when an admin space answers at all
//!   (`adminspace.enabled` defaults to false — absence is "not available",
//!   never "nothing declared").
//!
//! [`merge`] then folds the skeleton with the *observed* tree
//! ([`crate::KeyTreeSnapshot`]) and the active watch set into one tree with a
//! typed per-node [`NodeStatus`] — the acceptance criterion of RFC 09 §5.1
//! O4/O5 applied to a tree: "declared, never seen" and "watched, quiet" are
//! different facts and must be different *types*, not rendering conventions.

use std::collections::BTreeMap;

use zenoh::key_expr::keyexpr;

use crate::registry::SliceSet;
use crate::tree::{KeyTreeSnapshot, TreeNode};

/// One skeleton chunk: concrete, or a declared variable kept symbolic.
///
/// The display form of a variable is `{name}` — `{` sorts after the
/// alphanumerics, so variables naturally list after their literal siblings in
/// a `BTreeMap<String, _>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkeletonChunk {
    Literal(String),
    /// `{var}` — exactly one chunk, member set unknown.
    Var(String),
    /// `{var...}` — the whole remaining tail (RFC 03 §1.4's rest-variable).
    Rest(String),
}

impl SkeletonChunk {
    fn parse(chunk: &str) -> SkeletonChunk {
        if let Some(inner) = chunk.strip_prefix('{').and_then(|c| c.strip_suffix("...}")) {
            SkeletonChunk::Rest(inner.to_string())
        } else if let Some(inner) = chunk.strip_prefix('{').and_then(|c| c.strip_suffix('}')) {
            SkeletonChunk::Var(inner.to_string())
        } else {
            SkeletonChunk::Literal(chunk.to_string())
        }
    }

    /// The display key (and `BTreeMap` key) for this chunk.
    pub fn display(&self) -> String {
        match self {
            SkeletonChunk::Literal(s) => s.clone(),
            SkeletonChunk::Var(v) => format!("{{{v}}}"),
            SkeletonChunk::Rest(v) => format!("{{{v}...}}"),
        }
    }

    /// The selector chunk this position contributes when testing watch
    /// coverage: a variable is any-one-chunk, a rest is any-tail.
    fn selector_chunk(&self) -> &str {
        match self {
            SkeletonChunk::Literal(s) => s,
            SkeletonChunk::Var(_) => "*",
            SkeletonChunk::Rest(_) => "**",
        }
    }
}

/// Why we believe a node exists. At least one flag is set on every skeleton
/// node (an all-false node would be a node nobody claimed).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Evidence {
    /// A registry-slice subject template covers it.
    pub declared: bool,
    /// The liveliness roster places this origin/producer here.
    pub alive: bool,
    /// An admin-space declared entity's keyexpr names it.
    pub admin: bool,
}

impl Evidence {
    fn merge(self, other: Evidence) -> Evidence {
        Evidence {
            declared: self.declared || other.declared,
            alive: self.alive || other.alive,
            admin: self.admin || other.admin,
        }
    }
}

/// The declaring registry entry behind a skeleton leaf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclRef {
    pub producer: String,
    /// The subject pattern as declared (`disk/{mount}/used`).
    pub path: String,
    pub type_name: String,
}

#[derive(Debug, Clone, Default)]
pub struct SkeletonNode {
    pub chunk: Option<SkeletonChunk>,
    pub children: BTreeMap<String, SkeletonNode>,
    pub evidence: Evidence,
    /// Set on template leaves.
    pub decl: Option<DeclRef>,
}

impl SkeletonNode {
    fn insert(&mut self, chunks: &[SkeletonChunk], evidence: Evidence, decl: Option<DeclRef>) {
        self.evidence = self.evidence.merge(evidence);
        let Some((first, rest)) = chunks.split_first() else {
            if decl.is_some() {
                self.decl = decl;
            }
            return;
        };
        let child = self
            .children
            .entry(first.display())
            .or_insert_with(|| SkeletonNode {
                chunk: Some(first.clone()),
                ..SkeletonNode::default()
            });
        child.insert(rest, evidence, decl);
    }
}

/// What fed the skeleton — the O5 coverage statement, typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkeletonCoverage {
    pub slices: usize,
    pub roster_origins: usize,
    /// `None` = the admin space was not asked or did not answer — which is
    /// *not* an observed zero (RFC 09 §5.1 O4).
    pub admin_entities: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct Skeleton {
    pub root: SkeletonNode,
    pub coverage: SkeletonCoverage,
}

impl Skeleton {
    /// Build the declared keyspace. Pure — every input was gathered by the
    /// caller, this only assembles.
    ///
    /// Expansion rules:
    /// - a **host** slice × every roster origin running that producer →
    ///   `<base>/v1/<origin>/<class>/<producer>/<pattern…>` (evidence:
    ///   declared+alive);
    /// - a host slice with **no** live origin → the same under a symbolic
    ///   `{origin}` variable (declared, not placed — still visible, O4);
    /// - a **service** slice → `<base>/v1/<origin>/<class>/<pattern…>` with
    ///   no producer chunk (RFC 03 §1.5);
    /// - admin entities with concrete (wildcard-free) keyexprs insert as
    ///   literal paths; wildcard-bearing ones are not expanded into fake
    ///   concrete nodes — they only contribute to the coverage count.
    pub fn build(
        base: &str,
        slices: &SliceSet,
        roster: &BTreeMap<String, Vec<String>>,
        admin: Option<&crate::DeclaredEntities>,
    ) -> Skeleton {
        let mut root = SkeletonNode::default();
        let base_chunks: Vec<SkeletonChunk> = if base.is_empty() {
            Vec::new()
        } else {
            base.split('/')
                .map(|c| SkeletonChunk::Literal(c.to_string()))
                .collect()
        };

        for slice in slices.slices() {
            for subject in &slice.subjects {
                let decl = DeclRef {
                    producer: slice.name.clone(),
                    path: subject.path.clone(),
                    type_name: subject.type_name.clone(),
                };
                let tail: Vec<SkeletonChunk> =
                    subject.path.split('/').map(SkeletonChunk::parse).collect();

                if let Some(origin) = &slice.service_origin {
                    // Service origin: no producer chunk (RFC 03 §1.5).
                    let mut path = base_chunks.clone();
                    path.push(SkeletonChunk::Literal("v1".into()));
                    path.push(SkeletonChunk::Literal(origin.clone()));
                    path.push(SkeletonChunk::Literal(subject.class.clone()));
                    path.extend(tail.clone());
                    root.insert(
                        &path,
                        Evidence {
                            declared: true,
                            ..Evidence::default()
                        },
                        Some(decl.clone()),
                    );
                    continue;
                }

                // Host producer: place under every live origin running it…
                let live: Vec<&String> = roster
                    .iter()
                    .filter(|(_, producers)| {
                        producers
                            .iter()
                            .any(|p| p == &slice.name || instance_base(p) == slice.name)
                    })
                    .map(|(origin, _)| origin)
                    .collect();
                if live.is_empty() {
                    // …or, unplaced, under a symbolic {origin}: declared but
                    // not currently served anywhere we can see.
                    let mut path = base_chunks.clone();
                    path.push(SkeletonChunk::Literal("v1".into()));
                    path.push(SkeletonChunk::Var("origin".into()));
                    path.push(SkeletonChunk::Literal(subject.class.clone()));
                    path.push(SkeletonChunk::Literal(slice.name.clone()));
                    path.extend(tail.clone());
                    root.insert(
                        &path,
                        Evidence {
                            declared: true,
                            ..Evidence::default()
                        },
                        Some(decl.clone()),
                    );
                } else {
                    for origin in live {
                        let mut path = base_chunks.clone();
                        path.push(SkeletonChunk::Literal("v1".into()));
                        path.push(SkeletonChunk::Literal(origin.clone()));
                        path.push(SkeletonChunk::Literal(subject.class.clone()));
                        path.push(SkeletonChunk::Literal(slice.name.clone()));
                        path.extend(tail.clone());
                        root.insert(
                            &path,
                            Evidence {
                                declared: true,
                                alive: true,
                                ..Evidence::default()
                            },
                            Some(decl.clone()),
                        );
                    }
                }
            }
        }

        let mut admin_count = None;
        if let Some(entities) = admin {
            admin_count = Some(entities.entities.len());
            for e in &entities.entities {
                if e.keyexpr.contains('*') {
                    continue; // never invent concrete nodes from wildcards
                }
                let path: Vec<SkeletonChunk> = e
                    .keyexpr
                    .split('/')
                    .map(|c| SkeletonChunk::Literal(c.to_string()))
                    .collect();
                root.insert(
                    &path,
                    Evidence {
                        admin: true,
                        ..Evidence::default()
                    },
                    None,
                );
            }
        }

        Skeleton {
            root,
            coverage: SkeletonCoverage {
                slices: slices.slices().len(),
                roster_origins: roster.len(),
                admin_entities: admin_count,
            },
        }
    }
}

/// `snmp-2` → `snmp` (the instance suffix, RFC 03 §1.5).
fn instance_base(producer: &str) -> &str {
    match producer.rsplit_once('-') {
        Some((name, suffix))
            if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) =>
        {
            name
        }
        _ => producer,
    }
}

/// The typed declared/observed state of one merged node.
///
/// Derived purely from (has traffic stats) × (a watch covers it):
///
/// |                | covered by a watch | not covered            |
/// |----------------|--------------------|------------------------|
/// | **has stats**  | `Observed`         | `Unwatched` (leftover) |
/// | **no stats**   | `WatchedQuiet`     | `DeclaredOnly`         |
///
/// `Unwatched` is transient by construction —
/// [`Monitor::unwatch`](crate::Monitor::unwatch) retires uncovered stats
/// immediately — but the
/// state exists so the interval between release and retirement never renders
/// as live observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
    /// Skeleton evidence only — never seen a payload here.
    DeclaredOnly(Evidence),
    /// A watch covers it; no traffic has arrived. "Quiet" is an observation.
    WatchedQuiet(Evidence),
    /// Traffic observed under an active watch.
    Observed(Evidence),
    /// Stats linger but no watch covers them any more.
    Unwatched(Evidence),
}

impl NodeStatus {
    pub fn evidence(self) -> Evidence {
        match self {
            NodeStatus::DeclaredOnly(e)
            | NodeStatus::WatchedQuiet(e)
            | NodeStatus::Observed(e)
            | NodeStatus::Unwatched(e) => e,
        }
    }
}

/// Owned copy of a [`TreeNode`]'s numbers (the merged tree must not borrow
/// the snapshot).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeStats {
    pub count: u64,
    pub bytes: u64,
    pub rate_hz: f64,
    pub subtree_count: u64,
    pub subtree_bytes: u64,
    pub subtree_rate_hz: f64,
    pub subtree_keys: usize,
    /// Newest sample anywhere in the subtree — the per-node freshness signal
    /// (issue #65; computed by the snapshot since the beginning, now carried
    /// through the merge instead of dropped).
    pub subtree_last_seen: Option<std::time::Instant>,
}

impl NodeStats {
    /// Project one snapshot node's numbers.
    ///
    /// `pub` because a frontend that caches the tree's *shape* has to join the
    /// numbers back on at draw time, and reimplementing this field-by-field in
    /// the frontend is how the two would drift (zengui #177). The standing rule
    /// applies — a missing type is a `zenkey-fleet` issue, not a frontend
    /// workaround (`docs/redesign-2026-07.md` §15).
    pub fn from_tree(node: &TreeNode) -> NodeStats {
        NodeStats {
            count: node.count,
            bytes: node.bytes,
            rate_hz: node.rate_hz,
            subtree_count: node.subtree_count,
            subtree_bytes: node.subtree_bytes,
            subtree_rate_hz: node.subtree_rate_hz,
            subtree_keys: node.subtree_keys,
            subtree_last_seen: node.subtree_last_seen,
        }
    }
}

/// One node of the merged declared∪observed tree.
#[derive(Debug, Clone)]
pub struct MergedNode {
    pub children: BTreeMap<String, MergedNode>,
    pub status: NodeStatus,
    pub stats: Option<NodeStats>,
    pub decl: Option<DeclRef>,
}

/// Fold the skeleton, the observed snapshot, and the active watch set into
/// one tree. Runs at tick cadence — the same order of work as a flatten.
pub fn merge(skeleton: &Skeleton, observed: &KeyTreeSnapshot, watched: &[String]) -> MergedNode {
    // Borrowed, not owned: `keyexpr::new(&str)` validates without allocating,
    // where `KeyExpr::new(String)` builds an `OwnedKeyExpr` (an `Arc<str>`
    // copy) per selector per tick (`docs/zero-copy.md`).
    let watched: Vec<&keyexpr> = watched
        .iter()
        .filter_map(|w| keyexpr::new(w.as_str()).ok())
        .collect();
    // One reusable buffer for the descent's prefixes, instead of a fresh
    // `String` per node per tick.
    let mut path = String::new();
    merge_nodes(
        Some(&skeleton.root),
        Some(&observed.root),
        &watched,
        &mut path,
    )
}

fn merge_nodes(
    skel: Option<&SkeletonNode>,
    obs: Option<&TreeNode>,
    watched: &[&keyexpr],
    path: &mut String,
) -> MergedNode {
    let evidence = skel.map(|s| s.evidence).unwrap_or_default();
    let stats = obs.map(NodeStats::from_tree);
    let covered = is_covered(path, watched);
    let status = match (stats.is_some(), covered) {
        (true, true) => NodeStatus::Observed(evidence),
        (true, false) => NodeStatus::Unwatched(evidence),
        (false, true) => NodeStatus::WatchedQuiet(evidence),
        (false, false) => NodeStatus::DeclaredOnly(evidence),
    };

    let mut names: Vec<&String> = Vec::new();
    if let Some(s) = skel {
        names.extend(s.children.keys());
    }
    if let Some(o) = obs {
        names.extend(o.children.keys());
    }
    names.sort();
    names.dedup();

    let mut children = BTreeMap::new();
    for name in names {
        let skel_child = skel.and_then(|s| s.children.get(name));
        let obs_child = obs.and_then(|o| o.children.get(name));
        // Coverage tests run on *selector* form: symbolic chunks widen.
        // `selector_chunk()` already returns `&str`, so nothing is owned here.
        let sel_chunk = skel_child
            .and_then(|c| c.chunk.as_ref())
            .map(|c| c.selector_chunk())
            .unwrap_or(name.as_str());
        let mark = path.len();
        if !path.is_empty() {
            path.push('/');
        }
        path.push_str(sel_chunk);
        let child = merge_nodes(skel_child, obs_child, watched, path);
        path.truncate(mark);
        children.insert(name.clone(), child);
    }

    MergedNode {
        children,
        status,
        stats,
        decl: skel.and_then(|s| s.decl.clone()),
    }
}

/// Does any active watch reach into this subtree?
///
/// Deliberately generous: a node counts as covered when a watch *intersects*
/// its subtree (`prefix/**`), so an ancestor of a watched subtree reads
/// "watched" rather than "declared only" — the honest reading of "some watch
/// reaches below here". The root is covered iff anything is watched.
fn is_covered(prefix: &str, watched: &[&keyexpr]) -> bool {
    if watched.is_empty() {
        return false;
    }
    if prefix.is_empty() {
        return true;
    }
    // A thread-local scratch buffer: this runs once per node per tick, and a
    // fresh `String` plus an `OwnedKeyExpr` each time was the heaviest thing
    // on the render path (`docs/zero-copy.md`).
    thread_local! {
        static SUBTREE: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    }
    SUBTREE.with(|buf| {
        let mut buf = buf.borrow_mut();
        buf.clear();
        buf.push_str(prefix);
        buf.push_str("/**");
        let Ok(node) = keyexpr::new(buf.as_str()) else {
            return false;
        };
        watched.iter().any(|w| w.intersects(node))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::StatsTable;
    use std::time::Instant;
    use zenkey::slice::{RegistrySlice, SubjectDecl};

    fn subject(path: &str, class: &str) -> SubjectDecl {
        SubjectDecl {
            path: path.into(),
            class: class.into(),
            type_name: "T".into(),
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

    fn host_slice(name: &str, subjects: Vec<SubjectDecl>) -> RegistrySlice {
        RegistrySlice {
            version: "1.0".into(),
            app: "test".into(),
            convention: 1,
            name: name.into(),
            service_origin: None,
            description: None,
            subjects,
            procedures: vec![],
            blob: vec![],
            media: vec![],
            deprecated: vec![],
        }
    }

    #[test]
    fn skeleton_places_declared_subjects_under_live_origins() {
        let slices = SliceSet::from_slices(vec![host_slice(
            "sysinfo",
            vec![subject("disk/{mount}/used", "telemetry")],
        )]);
        let mut roster = BTreeMap::new();
        roster.insert("h-3fa9c2d41b7e".to_string(), vec!["sysinfo".to_string()]);
        let skel = Skeleton::build("", &slices, &roster, None);

        let node = &skel.root.children["v1"].children["h-3fa9c2d41b7e"].children["telemetry"]
            .children["sysinfo"]
            .children["disk"]
            .children["{mount}"]
            .children["used"];
        assert!(node.evidence.declared && node.evidence.alive);
        assert_eq!(node.decl.as_ref().unwrap().type_name, "T");
        assert_eq!(
            skel.coverage.admin_entities, None,
            "admin not asked is None, not zero (O4)"
        );
    }

    /// A declared producer with no live origin is still visible — under a
    /// symbolic {origin}, never invented as a concrete host.
    #[test]
    fn skeleton_keeps_unplaced_producers_symbolic() {
        let slices = SliceSet::from_slices(vec![host_slice(
            "sysinfo",
            vec![subject("health", "state")],
        )]);
        let skel = Skeleton::build("", &slices, &BTreeMap::new(), None);
        let origin = &skel.root.children["v1"].children["{origin}"];
        assert!(origin.evidence.declared && !origin.evidence.alive);
        assert!(
            origin.children["state"].children["sysinfo"].children["health"]
                .evidence
                .declared
        );
    }

    /// A service slice omits the producer chunk (RFC 03 §1.5).
    #[test]
    fn skeleton_places_service_origins_without_a_producer_chunk() {
        let mut slice = host_slice("catalog", vec![subject("entity/{id}", "state")]);
        slice.service_origin = Some("@catalog".into());
        let slices = SliceSet::from_slices(vec![slice]);
        let skel = Skeleton::build("", &slices, &BTreeMap::new(), None);
        let state = &skel.root.children["v1"].children["@catalog"].children["state"];
        assert!(state.children["entity"].children["{id}"].evidence.declared);
    }

    /// Admin evidence inserts concrete keyexprs and never expands wildcards
    /// into fake concrete nodes.
    #[test]
    fn skeleton_admin_evidence_is_concrete_only() {
        let entities = crate::DeclaredEntities {
            entities: vec![
                crate::DeclaredEntity {
                    kind: crate::EntityKind::Publisher,
                    keyexpr: "v1/h-aabbccddeeff/telemetry/x/m".into(),
                    node_zid: "z".into(),
                    sources: serde_json::Value::Null,
                },
                crate::DeclaredEntity {
                    kind: crate::EntityKind::Subscriber,
                    keyexpr: "v1/*/state/**".into(),
                    node_zid: "z".into(),
                    sources: serde_json::Value::Null,
                },
            ],
        };
        let skel = Skeleton::build("", &SliceSet::default(), &BTreeMap::new(), Some(&entities));
        assert_eq!(skel.coverage.admin_entities, Some(2));
        let concrete = &skel.root.children["v1"].children["h-aabbccddeeff"];
        assert!(concrete.evidence.admin);
        // The wildcard subscriber created no children under v1 beyond the
        // concrete one.
        assert_eq!(skel.root.children["v1"].children.len(), 1);
    }

    /// The four NodeStatus values are types, produced by the documented
    /// (stats × coverage) table.
    #[test]
    fn merge_produces_all_four_statuses() {
        // Skeleton declares two leaves under one live origin.
        let slices = SliceSet::from_slices(vec![host_slice(
            "sysinfo",
            vec![subject("cpu", "telemetry"), subject("mem", "telemetry")],
        )]);
        let mut roster = BTreeMap::new();
        roster.insert("h-3fa9c2d41b7e".to_string(), vec!["sysinfo".to_string()]);
        let skel = Skeleton::build("", &slices, &roster, None);

        // Observed traffic: cpu (watched) and a foreign key (not watched).
        let mut stats = StatsTable::new();
        let now = Instant::now();
        stats.record(
            "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu",
            4,
            None,
            now,
            None,
            None,
        );
        stats.record("demo/foreign", 4, None, now, None, None);
        let observed = KeyTreeSnapshot::build(&stats);

        let watched = vec!["v1/h-3fa9c2d41b7e/telemetry/**".to_string()];
        let merged = merge(&skel, &observed, &watched);

        let sysinfo = &merged.children["v1"].children["h-3fa9c2d41b7e"].children["telemetry"]
            .children["sysinfo"];
        assert!(matches!(
            sysinfo.children["cpu"].status,
            NodeStatus::Observed(_)
        ));
        assert!(
            matches!(sysinfo.children["mem"].status, NodeStatus::WatchedQuiet(_)),
            "declared, covered, no traffic — quiet is an observation"
        );
        assert!(matches!(
            merged.children["demo"].children["foreign"].status,
            NodeStatus::Unwatched(_)
        ));

        // No watches at all: everything declared reads DeclaredOnly.
        let merged = merge(&skel, &KeyTreeSnapshot::default(), &[]);
        assert!(matches!(
            merged.children["v1"].status,
            NodeStatus::DeclaredOnly(_)
        ));
    }

    #[test]
    fn instance_suffixes_place_under_their_base_producer() {
        let slices = SliceSet::from_slices(vec![host_slice(
            "snmp",
            vec![subject("if/{iface}/in", "telemetry")],
        )]);
        let mut roster = BTreeMap::new();
        roster.insert("h-aabbccddeeff".to_string(), vec!["snmp-2".to_string()]);
        let skel = Skeleton::build("", &slices, &roster, None);
        assert!(
            skel.root.children["v1"].children["h-aabbccddeeff"].children["telemetry"].children
                ["snmp"]
                .evidence
                .alive
        );
    }
}
