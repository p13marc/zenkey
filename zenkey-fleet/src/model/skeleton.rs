//! The declared keyspace of a zk2 deployment, built without subscribing to
//! anything (#614, FL4; v1's skeleton, issue #84, carried over).
//!
//! A Zenoh subscription cannot be "metadata only": once declared, payloads
//! flow. So a lazy explorer's tree starts from what costs nothing on the
//! data plane:
//!
//! - **presence and descriptors** ([`Catalog`], spec §8.1, §3.3) → which
//!   services run at which addresses, which interfaces each provides, and at
//!   which revision; a tokenless interface (U22) is found by its descriptor
//!   alone;
//! - **the revisions in hand** ([`Contracts`], an offline [`ContractSet`])
//!   → what each revision declares: its resources by kind token and
//!   template, with each `{p}` and `{p...}` kept symbolic;
//! - **admin-space declared entities** ([`crate::declared_entities`]) →
//!   key expressions sessions actually declared, when an admin space answers
//!   at all (`adminspace.enabled` defaults to false: absence is "not
//!   asked", never "nothing declared").
//!
//! Both come through one [`Lens`], so the skeleton places exactly what the
//! lens resolves with: the same namespace, the same presence read, the same
//! revisions. [`merge`] then folds the skeleton with the *observed* tree
//! ([`KeyTreeSnapshot`]) and the active watch set into one tree with a typed
//! per-node [`NodeStatus`] — the tooling guide's O4 and O5 applied to a
//! tree: "declared, never seen" and "watched, quiet" are different facts
//! and different *types*, not rendering conventions.
//!
//! **Where a resource is placed.** Each `(address, interface, revision)` a
//! descriptor names becomes `<namespace>/zk2/<system>/<service>/<iface>/
//! <token>/<template…>`; an event's leaf is one more chunk, the
//! occurrence's ULID, kept symbolic as `{occurrence}` (§2.6); an operation
//! is a declared leaf whose [`DeclRef::kind`] says so, since a frontend
//! offers no "watch" on a queryable. An interface a token names and no
//! descriptor does is placed by matching the token's fingerprint prefix
//! against the revisions held ([`DeclBasis::TokenPrefix`]); the lens does
//! not resolve keys that way, and [`DeclRef::basis`] says which way a leaf
//! got there. A revision not held places its address and interface and
//! nothing below them, counted in [`SkeletonCoverage::unheld`].
//!
//! **Without presence** (a replay, `--contracts` alone), every revision of
//! the offline set is placed under a symbolic `{system}/{service}`:
//! declared, not placed, as v1 kept `{origin}`.
//!
//! Session-free, like the rest of this layer: a `.zrec` replay builds the
//! skeleton from its offline contracts (no catalog) and merges the replay
//! [`crate::MonitorCore`]'s snapshot exactly as live does.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use zenkey_model::authoring::Kind;
use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::{Body, Resource};
use zenkey_model::grammar::{Addr, GRAMMAR, IfaceId, KindToken};
use zenkey_model::template::Segment;
use zenoh::key_expr::keyexpr;

use crate::model::catalog::{Catalog, ContractState, Revision, type_view};
use crate::model::lens::Lens;
use crate::model::tree::{KeyTreeSnapshot, TreeNode};
use crate::report::{DeclaredEntities, LensPresence};

/// One skeleton chunk: concrete, or a variable kept symbolic.
///
/// The display form of a variable is `{name}`: `{` sorts after the
/// alphanumerics and after `@`, so variables list after their literal
/// siblings in a `BTreeMap<String, _>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkeletonChunk {
    Literal(String),
    /// `{name}`: exactly one chunk, its values unknown. A template's
    /// `{param}`, an unplaced `{system}` or `{service}`, an event's
    /// `{occurrence}`.
    Var(String),
    /// `{name...}`: the rest of the key, one chunk or more (§2.2).
    Rest(String),
}

impl SkeletonChunk {
    /// The display key (and `BTreeMap` key) for this chunk.
    pub fn display(&self) -> String {
        match self {
            SkeletonChunk::Literal(s) => s.clone(),
            SkeletonChunk::Var(v) => format!("{{{v}}}"),
            SkeletonChunk::Rest(v) => format!("{{{v}...}}"),
        }
    }

    /// The selector chunk this position contributes when testing watch
    /// coverage: a variable is any one chunk, a rest is any tail.
    fn selector_chunk(&self) -> &str {
        match self {
            SkeletonChunk::Literal(s) => s,
            SkeletonChunk::Var(_) => "*",
            SkeletonChunk::Rest(_) => "**",
        }
    }

    fn literal(s: &str) -> SkeletonChunk {
        SkeletonChunk::Literal(s.to_owned())
    }
}

/// Why we believe a node exists. At least one flag is set on every skeleton
/// node: an all-false node would be a node nobody claimed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Evidence {
    /// A revision in hand declares it.
    pub declared: bool,
    /// Presence places it: an address a presence read saw, and what its
    /// descriptors or tokens name.
    pub present: bool,
    /// An admin-space declared entity's key expression names it.
    pub admin: bool,
}

impl Evidence {
    fn merge(self, other: Evidence) -> Evidence {
        Evidence {
            declared: self.declared || other.declared,
            present: self.present || other.present,
            admin: self.admin || other.admin,
        }
    }

    const DECLARED: Evidence = Evidence {
        declared: true,
        present: false,
        admin: false,
    };

    const PRESENT: Evidence = Evidence {
        declared: false,
        present: true,
        admin: false,
    };

    const PLACED: Evidence = Evidence {
        declared: true,
        present: true,
        admin: false,
    };
}

/// How a declared leaf got where it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclBasis {
    /// A descriptor of the address names the revision (§3.3): what the lens
    /// resolves keys through.
    Descriptor,
    /// No descriptor names it; an interface token's fingerprint prefix
    /// matched one revision held (§8.1). The lens resolves no key this way
    /// (it stops at "no revision"), so a frontend says so beside the leaf.
    TokenPrefix,
    /// No presence read: the revision is one of the offline set, under a
    /// symbolic `{system}/{service}`.
    Offline,
}

/// The declaring resource behind a skeleton leaf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclRef {
    /// The address it is placed under; `None` under a symbolic
    /// `{system}/{service}`.
    pub address: Option<Addr>,
    pub iface: IfaceId,
    /// The revision that declares it.
    pub fingerprint: Fingerprint,
    /// `<kind token>/<template>` (`state/plans/{vehicle}`, `@op/reset`).
    pub resource: String,
    /// Stream, state, event or operation: an operation is a queryable, and
    /// a frontend offers no "watch" on it.
    pub kind: Kind,
    /// The type of the value its key carries, as the report names types: a
    /// data resource's `type`, an operation's `response`.
    pub type_name: String,
    pub basis: DeclBasis,
}

#[derive(Debug, Clone, Default)]
pub struct SkeletonNode {
    pub chunk: Option<SkeletonChunk>,
    pub children: BTreeMap<String, SkeletonNode>,
    pub evidence: Evidence,
    /// Set on a resource's leaf. When two revisions place one leaf (a
    /// rolling upgrade), the first placed — fingerprint order — is kept, and
    /// [`SkeletonCoverage::ambiguous`] counts the interface.
    pub decl: Option<DeclRef>,
}

impl SkeletonNode {
    fn insert(&mut self, chunks: &[SkeletonChunk], evidence: Evidence, decl: Option<DeclRef>) {
        self.evidence = self.evidence.merge(evidence);
        let Some((first, rest)) = chunks.split_first() else {
            if self.decl.is_none() {
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

    /// The node at `path` (display chunks), when there is one.
    pub fn node(&self, path: &[&str]) -> Option<&SkeletonNode> {
        let mut node = self;
        for chunk in path {
            node = node.children.get(*chunk)?;
        }
        Some(node)
    }
}

/// What fed the skeleton — the O5 coverage statement, typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkeletonCoverage {
    /// Distinct revisions whose resources were placed.
    pub revisions: usize,
    /// Addresses placed from presence.
    pub addresses: usize,
    /// `(address, interface, revision)` triples presence names whose
    /// revision is not held (not asked, unavailable or unreadable), and
    /// interfaces a token names whose prefix matched nothing held: the
    /// address and interface are placed, nothing below them.
    pub unheld: usize,
    /// `(address, interface)` pairs whose descriptors name more than one
    /// revision: each one's resources are placed, a leaf both declare keeps
    /// the first, and the lens resolves keys there as ambiguous.
    pub ambiguous: usize,
    /// The presence read behind the skeleton; `None` without one.
    pub presence: Option<LensPresence>,
    /// `None`: the admin space was not asked or did not answer, which is
    /// *not* an observed zero (O4).
    pub admin_entities: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct Skeleton {
    pub root: SkeletonNode,
    pub coverage: SkeletonCoverage,
}

impl Skeleton {
    /// Build the declared keyspace from what `lens` resolves with: its
    /// namespace, its presence read (when there is one) and the revisions in
    /// hand. Pure — every input was gathered by the caller, this only
    /// assembles. See the module doc for where each resource is placed.
    ///
    /// Admin entities with concrete (wildcard-free) key expressions insert
    /// as literal paths; wildcard-bearing ones are never expanded into fake
    /// concrete nodes and only count.
    pub fn build(lens: &Lens<'_>, admin: Option<&DeclaredEntities>) -> Skeleton {
        let mut root = SkeletonNode::default();
        let base: Vec<SkeletonChunk> = lens
            .namespace()
            .split('/')
            .filter(|c| !c.is_empty())
            .chain(std::iter::once(GRAMMAR))
            .map(SkeletonChunk::literal)
            .collect();
        let mut coverage = SkeletonCoverage {
            revisions: 0,
            addresses: 0,
            unheld: 0,
            ambiguous: 0,
            presence: lens.scope().presence,
            admin_entities: None,
        };
        let mut placed: BTreeSet<(IfaceId, Fingerprint)> = BTreeSet::new();

        match lens.catalog() {
            Some(catalog) => {
                for addr in catalog.addresses() {
                    place_address(&mut root, &base, catalog, lens, addr, &mut coverage, |r| {
                        placed.insert((r.iface().clone(), r.fingerprint().clone()));
                    });
                }
            }
            None => {
                for revision in lens.offline_set().into_iter().flat_map(|s| s.iter()) {
                    let mut path = base.clone();
                    path.push(SkeletonChunk::Var("system".into()));
                    path.push(SkeletonChunk::Var("service".into()));
                    place_revision(
                        &mut root,
                        &path,
                        None,
                        revision,
                        DeclBasis::Offline,
                        Evidence::DECLARED,
                    );
                    placed.insert((revision.iface().clone(), revision.fingerprint().clone()));
                }
            }
        }
        coverage.revisions = placed.len();

        if let Some(entities) = admin {
            coverage.admin_entities = Some(entities.entities.len());
            for e in &entities.entities {
                if e.keyexpr.contains('*') {
                    continue; // never invent concrete nodes from wildcards
                }
                let path: Vec<SkeletonChunk> =
                    e.keyexpr.split('/').map(SkeletonChunk::literal).collect();
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

        Skeleton { root, coverage }
    }
}

/// One address of the presence read: its node, then each interface it
/// provides, then each revision's resources.
fn place_address(
    root: &mut SkeletonNode,
    base: &[SkeletonChunk],
    catalog: &Catalog,
    lens: &Lens<'_>,
    addr: &Addr,
    coverage: &mut SkeletonCoverage,
    mut placed: impl FnMut(&Arc<Revision>),
) {
    let mut at = base.to_vec();
    at.push(SkeletonChunk::literal(addr.system.as_str()));
    at.push(SkeletonChunk::literal(addr.service.as_str()));
    root.insert(&at, Evidence::PRESENT, None);
    coverage.addresses += 1;
    for iface in catalog.provided(addr) {
        let fps = catalog.revisions_of(addr, &iface);
        let mut iface_path = at.clone();
        iface_path.push(SkeletonChunk::literal(&iface.to_string()));
        let held: Vec<(Arc<Revision>, DeclBasis)> = if fps.is_empty() {
            // No descriptor names a revision: an interface token's prefix
            // matched against what is held, one revision or none.
            let prefixes = catalog.token_prefixes(addr, &iface);
            let mut matched: Vec<Arc<Revision>> = held_of(lens, &iface)
                .into_iter()
                .filter(|r| prefixes.contains(&r.fingerprint().hex().fp16()))
                .collect();
            matched.dedup_by(|a, b| a.fingerprint() == b.fingerprint());
            match matched.as_slice() {
                [one] => vec![(Arc::clone(one), DeclBasis::TokenPrefix)],
                _ => {
                    coverage.unheld += 1;
                    Vec::new()
                }
            }
        } else {
            if fps.len() > 1 {
                coverage.ambiguous += 1;
            }
            fps.iter()
                .filter_map(|fp| match lens.contracts().state(&iface, fp) {
                    Some(ContractState::Held(r)) => Some((r, DeclBasis::Descriptor)),
                    _ => {
                        coverage.unheld += 1;
                        None
                    }
                })
                .collect()
        };
        root.insert(&iface_path, Evidence::PRESENT, None);
        for (revision, basis) in held {
            place_revision(root, &at, Some(addr), &revision, basis, Evidence::PLACED);
            placed(&revision);
        }
    }
}

/// Every revision of `iface` in hand: the store's, then the offline set's.
fn held_of(lens: &Lens<'_>, iface: &IfaceId) -> Vec<Arc<Revision>> {
    let mut out = lens.contracts().held_of(iface);
    if let Some(set) = lens.offline_set() {
        out.extend(set.of_iface(iface).cloned());
    }
    out.sort_by(|a, b| a.fingerprint().cmp(b.fingerprint()));
    out
}

/// One revision's resources under `at` (`…/<system>/<service>`, literal or
/// symbolic).
fn place_revision(
    root: &mut SkeletonNode,
    at: &[SkeletonChunk],
    address: Option<&Addr>,
    revision: &Arc<Revision>,
    basis: DeclBasis,
    evidence: Evidence,
) {
    let iface = revision.iface();
    for resource in &revision.contract().resources {
        let mut path = at.to_vec();
        path.push(SkeletonChunk::literal(&iface.to_string()));
        path.push(SkeletonChunk::literal(resource.token.as_str()));
        path.extend(resource.template.segments().iter().map(|s| match s {
            Segment::Literal(l) => SkeletonChunk::Literal(l.clone()),
            Segment::Param(p) => SkeletonChunk::Var(p.clone()),
            Segment::Rest(p) => SkeletonChunk::Rest(p.clone()),
        }));
        if resource.token == KindToken::Events {
            path.push(SkeletonChunk::Var("occurrence".into()));
        }
        let decl = DeclRef {
            address: address.cloned(),
            iface: iface.clone(),
            fingerprint: revision.fingerprint().clone(),
            resource: zenkey::implementation::resource_name(resource),
            kind: resource.kind,
            type_name: type_name(resource),
            basis,
        };
        root.insert(&path, evidence, Some(decl));
    }
}

/// The type of the value a resource's key carries.
fn type_name(resource: &Resource) -> String {
    match &resource.body {
        Body::Data(d) => type_view(&d.type_).name,
        Body::Operation(o) => type_view(&o.response).name,
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
/// immediately — but the state exists so the interval between release and
/// retirement never renders as live observation.
///
/// Coverage is a key-expression fact: a watch on `zk2/**` covers no
/// `@stream`, `@state` or `@op` node, since `**` never crosses a verbatim
/// chunk (§1.3). Such a node reads `DeclaredOnly` under an ambient watch,
/// which is the truth about what that watch can see.
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
    /// (issue #65), carried through the merge instead of dropped.
    pub subtree_last_seen: Option<std::time::Instant>,
}

impl NodeStats {
    /// Project one snapshot node's numbers.
    ///
    /// `pub` because a frontend that caches the tree's *shape* has to join the
    /// numbers back on at draw time, and reimplementing this field-by-field in
    /// the frontend is how the two would drift (zengui #177).
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

/// Fold the skeleton, the observed snapshot, and the active watch set (wire
/// selectors, as [`crate::Monitor::watched`] lists them) into one tree.
/// Runs at tick cadence — the same order of work as a flatten.
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
    use crate::model::catalog::ContractSet;
    use crate::model::stats::StatsTable;
    use crate::report::ContractSource;
    use std::time::Instant;

    fn revision(toml: &str) -> Revision {
        let l = zenkey_model::contract::load_str(toml, std::path::Path::new("."), None);
        Revision::from_contract(
            l.contract
                .unwrap_or_else(|| panic!("does not load:\n{}", l.report)),
            ContractSource::File,
        )
    }

    const NAV: &str = r#"
[interface]
name = "nav"
major = 2

[resources.position]
kind = "stream"
type = { raw = "application/octet-stream" }

[resources."raw/{sensor}"]
kind = "stream"
explicit = true
type = { raw = "application/octet-stream" }
params = { sensor = "string" }
cardinality = 8

[resources."plans/{vehicle}"]
kind = "state"
type = { raw = "application/octet-stream" }
params = { vehicle = "string" }
cardinality = 8

[resources."applied/{plan}"]
kind = "event"
type = { raw = "application/octet-stream" }
params = { plan = "string" }
cardinality = 8
rate = "low"
retention = "1d"

[resources.reset]
kind = "operation"
request = { raw = "application/octet-stream" }
response = { raw = "text/plain" }
"#;

    fn offline() -> ContractSet {
        let mut set = ContractSet::new();
        set.insert(revision(NAV));
        set
    }

    /// Without presence, every offline revision sits under a symbolic
    /// `{system}/{service}`: declared, never placed, its params symbolic,
    /// an event's occurrence a leaf of its own and an operation a declared
    /// leaf that says it is one.
    #[test]
    fn offline_revisions_sit_under_a_symbolic_address() {
        let set = offline();
        let lens = Lens::new("prod", None, &set).offline(&set);
        let skel = Skeleton::build(&lens, None);
        let svc = skel
            .root
            .node(&["prod", "zk2", "{system}", "{service}", "nav.v2"])
            .expect("the interface under a symbolic address");
        assert!(svc.evidence.declared && !svc.evidence.present);
        let plans = svc
            .node(&["state", "plans", "{vehicle}"])
            .expect("a state template");
        let decl = plans.decl.as_ref().expect("a declared leaf");
        assert_eq!(decl.resource, "state/plans/{vehicle}");
        assert_eq!(decl.address, None);
        assert_eq!(decl.basis, DeclBasis::Offline);
        assert_eq!(decl.kind, Kind::State);
        let applied = svc
            .node(&["events", "applied", "{plan}", "{occurrence}"])
            .expect("an event's occurrence is a leaf");
        assert_eq!(applied.decl.as_ref().map(|d| d.kind), Some(Kind::Event));
        let reset = svc.node(&["@op", "reset"]).expect("an operation's leaf");
        let reset = reset.decl.as_ref().expect("declared");
        assert_eq!(reset.kind, Kind::Operation);
        assert_eq!(reset.type_name, "text/plain", "an operation's response");
        assert!(svc.node(&["@stream", "raw", "{sensor}"]).is_some());
        assert_eq!(skel.coverage.revisions, 1);
        assert_eq!(skel.coverage.addresses, 0);
        assert_eq!(skel.coverage.presence, None);
        assert_eq!(
            skel.coverage.admin_entities, None,
            "admin not asked is None, not zero (O4)"
        );
    }

    /// Admin evidence inserts concrete key expressions and never expands
    /// wildcards into fake concrete nodes.
    #[test]
    fn admin_evidence_is_concrete_only() {
        let entities = DeclaredEntities {
            entities: vec![
                crate::report::DeclaredEntity {
                    kind: crate::report::EntityKind::Publisher,
                    keyexpr: "zk2/h1/tc/tc.netif.v1/stream/x".into(),
                    node_zid: "z".into(),
                    sources: serde_json::Value::Null,
                },
                crate::report::DeclaredEntity {
                    kind: crate::report::EntityKind::Subscriber,
                    keyexpr: "zk2/*/*/*/state/**".into(),
                    node_zid: "z".into(),
                    sources: serde_json::Value::Null,
                },
            ],
        };
        let set = ContractSet::new();
        let lens = Lens::new("", None, &set);
        let skel = Skeleton::build(&lens, Some(&entities));
        assert_eq!(skel.coverage.admin_entities, Some(2));
        let concrete = skel.root.node(&["zk2", "h1"]).expect("the concrete one");
        assert!(concrete.evidence.admin && !concrete.evidence.declared);
        assert_eq!(skel.root.children["zk2"].children.len(), 1);
    }

    /// One descriptor naming `revisions`, each with its token flag.
    fn descriptor(
        address: &str,
        instance: u64,
        revisions: &[(&Revision, bool)],
    ) -> zenkey_model::descriptor::Descriptor {
        zenkey_model::descriptor::Descriptor {
            format: "zk2-descriptor/0.1".into(),
            service: address.into(),
            instance: format!("{instance:016x}"),
            interfaces: revisions
                .iter()
                .map(|(r, token)| zenkey_model::descriptor::InterfaceEntry {
                    iface: r.iface().to_string(),
                    contract: r.fingerprint().to_string(),
                    minor: 0,
                    token: *token,
                    unavailable: Vec::new(),
                    cardinality: BTreeMap::new(),
                })
                .collect(),
            capabilities: Vec::new(),
            requires: Vec::new(),
            profiles: Vec::new(),
            meta: BTreeMap::new(),
        }
    }

    /// A catalog of one instance at `address`, holding its instance token
    /// and the interface tokens `tokens` names, with `served` as its
    /// descriptor.
    fn catalog(
        address: &str,
        tokens: &[&Revision],
        served: zenkey_model::descriptor::Descriptor,
    ) -> Catalog {
        let addr: Addr = address.parse().expect("an address");
        let instance = zenkey_model::grammar::InstanceId::from_u64(1);
        let mut keys = vec![
            zenkey_model::grammar::instance_key(&addr, &instance)
                .expect("a key")
                .to_string(),
        ];
        for r in tokens {
            keys.push(
                zenkey_model::grammar::alive_key(
                    &addr,
                    r.iface(),
                    &instance,
                    &r.fingerprint().hex().fp16(),
                )
                .expect("a key")
                .to_string(),
            );
        }
        let mut observed =
            crate::model::catalog::Observed::from_keys("zk2/*/*/@zk/**", &keys, true);
        observed.descriptors = Some(BTreeMap::from([(
            (addr, instance),
            crate::model::catalog::DescriptorRead::Served(Box::new(served)),
        )]));
        Catalog::new(&observed)
    }

    /// The four statuses are types, produced by the documented
    /// (stats × coverage) table; and an ambient watch covers no verbatim
    /// node, so `@stream` reads declared only under `zk2/**`.
    #[test]
    fn merge_produces_all_four_statuses() {
        let set = offline();
        let nav = set.iter().next().expect("one revision");
        let catalog = catalog("r1/nav", &[nav], descriptor("r1/nav", 1, &[(nav, true)]));
        let lens = Lens::new("", Some(&catalog), &set);
        let skel = Skeleton::build(&lens, None);

        let mut stats = StatsTable::new();
        let now = Instant::now();
        stats.record(
            "zk2/r1/nav/nav.v2/stream/position",
            4,
            None,
            now,
            None,
            None,
        );
        stats.record("demo/foreign", 4, None, now, None, None);
        let observed = KeyTreeSnapshot::build(&stats);

        let watched = vec!["zk2/**".to_string()];
        let merged = merge(&skel, &observed, &watched);
        let nav = &merged.children["zk2"].children["r1"].children["nav"].children["nav.v2"];
        assert!(matches!(
            nav.children["stream"].children["position"].status,
            NodeStatus::Observed(_)
        ));
        assert!(
            matches!(
                nav.children["state"].children["plans"].status,
                NodeStatus::WatchedQuiet(_)
            ),
            "declared, covered, no traffic: quiet is an observation"
        );
        assert!(
            matches!(nav.children["@stream"].status, NodeStatus::DeclaredOnly(_)),
            "`**` never crosses a verbatim chunk (§1.3)"
        );
        assert!(matches!(
            merged.children["demo"].children["foreign"].status,
            NodeStatus::Unwatched(_)
        ));

        let merged = merge(&skel, &KeyTreeSnapshot::default(), &[]);
        assert!(matches!(
            merged.children["zk2"].status,
            NodeStatus::DeclaredOnly(_)
        ));
    }
}
