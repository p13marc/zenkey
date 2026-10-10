//! The contract skeleton over the repository's own contracts (#614, FL4):
//! `examples/zk2/walkthrough` and `examples/zk2/tcgui`, beside the profiles'
//! `health.v1`, with a catalog built by hand from tokens and descriptors
//! carrying their real fingerprints — the ones `examples/zk2/.history`
//! publishes. No bus: the skeleton, its merge and the identity cache are
//! session-free, which is what lets a `.zrec` replay draw the same tree.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use zenkey_fleet::model::catalog::{DescriptorRead, Observed};
use zenkey_fleet::model::stats::StatsTable;
use zenkey_fleet::report::Unresolved;
use zenkey_fleet::{
    Catalog, ContractSet, DeclBasis, IdentityCache, KeyTreeSnapshot, Lens, NodeStatus, Revision,
    Skeleton, merge_skeleton,
};
use zenkey_model::authoring::Kind;
use zenkey_model::descriptor::{Descriptor, InterfaceEntry};
use zenkey_model::grammar::{Addr, IfaceId, InstanceId, alive_key, instance_key};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// The walkthrough's and the tcgui pilot's contracts, and `health.v1`.
fn contracts() -> ContractSet {
    let mut set = ContractSet::new();
    for dir in [
        "examples/zk2/walkthrough",
        "examples/zk2/tcgui",
        "spec/profiles/health",
    ] {
        let (more, problems) = ContractSet::load_path(&root().join(dir));
        // The tcgui directory holds its frontend's bindings beside the
        // contracts, reported and skipped.
        for p in &problems {
            assert!(p.at.ends_with(".bindings.toml"), "{dir}: {p:?}");
        }
        set.extend(more);
    }
    set
}

fn iface(s: &str) -> IfaceId {
    s.parse().expect("an interface")
}

fn addr(s: &str) -> Addr {
    s.parse().expect("an address")
}

/// The one revision of `name` held.
fn revision(set: &ContractSet, name: &str) -> Arc<Revision> {
    let revs: Vec<_> = set.of_iface(&iface(name)).collect();
    assert_eq!(revs.len(), 1, "{name}");
    Arc::clone(revs[0])
}

/// A fingerprint no revision has: a descriptor naming a revision nobody
/// holds.
const UNHELD: &str = "sha256:00000000000000000000000000000000000000000000000000000000000000aa";

/// One instance as a test bed states it: the interface tokens it holds
/// (by revision), and what its descriptor names — `(iface, fingerprint,
/// token)` — or `None` for a descriptor GET that went unanswered.
struct Inst<'a> {
    address: &'a str,
    id: u64,
    tokens: Vec<&'a Revision>,
    descriptor: Option<Vec<(String, String, bool)>>,
}

fn named(r: &Revision, token: bool) -> (String, String, bool) {
    (r.iface().to_string(), r.fingerprint().to_string(), token)
}

fn catalog(instances: &[Inst<'_>]) -> Catalog {
    let mut keys = Vec::new();
    let mut descriptors = BTreeMap::new();
    for i in instances {
        let a = addr(i.address);
        let id = InstanceId::from_u64(i.id);
        keys.push(instance_key(&a, &id).expect("a key").to_string());
        for r in &i.tokens {
            keys.push(
                alive_key(&a, r.iface(), &id, &r.fingerprint().hex().fp16())
                    .expect("a key")
                    .to_string(),
            );
        }
        let read = match &i.descriptor {
            None => DescriptorRead::Silent,
            Some(entries) => DescriptorRead::Served(Box::new(Descriptor {
                format: "zk2-descriptor/0.1".into(),
                service: i.address.into(),
                instance: id.to_string(),
                interfaces: entries
                    .iter()
                    .map(|(iface, contract, token)| InterfaceEntry {
                        iface: iface.clone(),
                        contract: contract.clone(),
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
            })),
        };
        descriptors.insert((a, id), read);
    }
    let mut observed = Observed::from_keys("zk2/*/*/@zk/**", &keys, true);
    observed.descriptors = Some(descriptors);
    Catalog::new(&observed)
}

/// The deployment the tests draw:
/// - `robot1/nav` serves `nav.v2` with its token, and `health.v1` from its
///   tokenless set (U22): its descriptor alone names it;
/// - `robot1/camera` serves `camera.v1`, whose image is an `@stream`;
/// - `host-a/tc` serves `tc.netem.v1`, and names a `tc.netif.v1` revision
///   nobody holds;
/// - `host-b/tc` holds a `tc.netif.v1` token, and its descriptor GET went
///   unanswered;
/// - `robot2/nav` has two instances naming two `nav.v2` revisions, one of
///   them not held (a rolling upgrade);
/// - `fleet/gui` is a pure consumer: an instance token, no interface.
fn deployment(set: &ContractSet) -> Catalog {
    let nav = revision(set, "nav.v2");
    let health = revision(set, "health.v1");
    let camera = revision(set, "camera.v1");
    let netem = revision(set, "tc.netem.v1");
    let netif = revision(set, "tc.netif.v1");
    catalog(&[
        Inst {
            address: "robot1/nav",
            id: 1,
            tokens: vec![&nav],
            descriptor: Some(vec![named(&nav, true), named(&health, false)]),
        },
        Inst {
            address: "robot1/camera",
            id: 2,
            tokens: vec![&camera],
            descriptor: Some(vec![named(&camera, true)]),
        },
        Inst {
            address: "host-a/tc",
            id: 3,
            tokens: vec![&netem],
            descriptor: Some(vec![
                named(&netem, true),
                ("tc.netif.v1".into(), UNHELD.into(), true),
            ]),
        },
        Inst {
            address: "host-b/tc",
            id: 4,
            tokens: vec![&netif],
            descriptor: None,
        },
        Inst {
            address: "robot2/nav",
            id: 5,
            tokens: vec![&nav],
            descriptor: Some(vec![named(&nav, true)]),
        },
        Inst {
            address: "robot2/nav",
            id: 6,
            tokens: Vec::new(),
            descriptor: Some(vec![("nav.v2".into(), UNHELD.into(), true)]),
        },
        Inst {
            address: "fleet/gui",
            id: 7,
            tokens: Vec::new(),
            descriptor: Some(Vec::new()),
        },
    ])
}

/// The fingerprints are the published ones (`examples/zk2/.history`), not
/// ones this test made up.
#[test]
fn the_fingerprints_are_the_published_ones() {
    let set = contracts();
    for name in [
        "nav.v2",
        "camera.v1",
        "tc.netem.v1",
        "tc.netif.v1",
        "health.v1",
    ] {
        let r = revision(&set, name);
        let published = root()
            .join("examples/zk2/.history")
            .join(name)
            .join(format!("{}.bundle.json", r.fingerprint().hex()));
        assert!(published.is_file(), "{}", published.display());
    }
}

/// Every resource of a revision a descriptor names is placed under its
/// address with its params symbolic, the U22 interface from the descriptor
/// alone, an `@stream` under its verbatim token, an event's occurrence as a
/// leaf of its own and an operation as a leaf that says it is one; a
/// revision not held places its interface and nothing below it; a token
/// whose descriptor went unanswered places its interface through the
/// prefix, and says so; a pure consumer is an address and nothing else.
#[test]
fn a_deployment_is_placed_from_presence_and_the_revisions_held() {
    let set = contracts();
    let catalog = deployment(&set);
    let lens = Lens::new("site", Some(&catalog), &set);
    let skel = Skeleton::build(&lens, None);
    let at = |path: &str| {
        let chunks: Vec<&str> = path.split('/').collect();
        skel.root
            .node(&chunks)
            .unwrap_or_else(|| panic!("no node at {path}"))
    };

    // nav.v2 by its descriptor, under its address.
    let position = at("site/zk2/robot1/nav/nav.v2/stream/position");
    assert!(position.evidence.declared && position.evidence.present);
    let decl = position.decl.as_ref().expect("a declared leaf");
    assert_eq!(decl.address, Some(addr("robot1/nav")));
    assert_eq!(decl.resource, "stream/position");
    assert_eq!(decl.kind, Kind::Stream);
    assert_eq!(decl.type_name, "nav.v2.Position");
    assert_eq!(decl.basis, DeclBasis::Descriptor);
    let set_origin = at("site/zk2/robot1/nav/nav.v2/@op/set_origin");
    let decl = set_origin.decl.as_ref().expect("declared");
    assert_eq!(decl.kind, Kind::Operation, "no watch on a queryable");
    assert_eq!(decl.type_name, "nav.v2.Origin", "the response's type");

    // health.v1 from the tokenless set: its descriptor alone names it.
    let checks = at("site/zk2/robot1/nav/health.v1/state/checks/{check}");
    assert_eq!(
        checks.decl.as_ref().map(|d| d.basis),
        Some(DeclBasis::Descriptor)
    );

    // camera.v1's explicit stream, under its verbatim token.
    let image = at("site/zk2/robot1/camera/camera.v1/@stream/image");
    assert_eq!(image.decl.as_ref().map(|d| d.kind), Some(Kind::Stream));

    // tc.netem.v1's event: the occurrence is the leaf.
    let applied = at("site/zk2/host-a/tc/tc.netem.v1/events/applied/{occurrence}");
    assert_eq!(applied.decl.as_ref().map(|d| d.kind), Some(Kind::Event));
    assert!(
        at("site/zk2/host-a/tc/tc.netem.v1/state/config/{ns}/{iface}")
            .decl
            .is_some()
    );

    // A revision nobody holds: the interface, and nothing below it.
    let unheld = at("site/zk2/host-a/tc/tc.netif.v1");
    assert!(unheld.children.is_empty());
    assert!(unheld.evidence.present && !unheld.evidence.declared);

    // A token whose descriptor went unanswered, placed by its prefix —
    // which the lens does not resolve through, and the leaf says so.
    let by_token = at("site/zk2/host-b/tc/tc.netif.v1/state/interfaces/{ns}/{iface}");
    assert_eq!(
        by_token.decl.as_ref().map(|d| d.basis),
        Some(DeclBasis::TokenPrefix)
    );
    let identity = lens.identity("site/zk2/host-b/tc/tc.netif.v1/state/interfaces/ns0/eth0");
    assert_eq!(identity.unresolved, Some(Unresolved::NoRevision));

    // A rolling upgrade: both revisions named, the held one placed.
    assert!(at("site/zk2/robot2/nav/nav.v2/state/status").decl.is_some());

    // A pure consumer: an address and nothing else.
    let gui = at("site/zk2/fleet/gui");
    assert!(gui.children.is_empty() && gui.evidence.present);

    let c = &skel.coverage;
    assert_eq!(c.addresses, 6);
    assert_eq!(
        c.revisions, 5,
        "nav.v2, health.v1, camera.v1, tc.netem.v1, tc.netif.v1"
    );
    assert_eq!(c.unheld, 2, "host-a's tc.netif.v1, robot2's second nav.v2");
    assert_eq!(c.ambiguous, 1, "robot2/nav names two revisions");
    let presence = c.presence.as_ref().expect("a presence read");
    assert!(presence.complete);
    assert_eq!(presence.services, 6);
    assert_eq!(c.admin_entities, None);
}

/// The skeleton places what the lens resolves: a key under a declared leaf
/// resolves to the leaf's resource, through the identity cache.
#[test]
fn the_lens_resolves_what_the_skeleton_places() {
    let set = contracts();
    let catalog = deployment(&set);
    let lens = Lens::new("site", Some(&catalog), &set).at_generation(1);
    let skel = Skeleton::build(&lens, None);
    let mut cache = IdentityCache::default();
    for (wire, leaf) in [
        (
            "site/zk2/robot1/nav/nav.v2/state/status",
            "site/zk2/robot1/nav/nav.v2/state/status",
        ),
        (
            "site/zk2/host-a/tc/tc.netem.v1/state/config/ns0/eth0",
            "site/zk2/host-a/tc/tc.netem.v1/state/config/{ns}/{iface}",
        ),
        (
            "site/zk2/host-a/tc/tc.netem.v1/events/applied/01hzy3d1s5k8b2c4e6g8j0m2p4",
            "site/zk2/host-a/tc/tc.netem.v1/events/applied/{occurrence}",
        ),
    ] {
        let chunks: Vec<&str> = leaf.split('/').collect();
        let decl = skel.root.node(&chunks).and_then(|n| n.decl.clone());
        let decl = decl.unwrap_or_else(|| panic!("no declared leaf at {leaf}"));
        let resolved = cache
            .ensure(&lens, wire)
            .resolved
            .clone()
            .unwrap_or_else(|| panic!("{wire} does not resolve"));
        assert_eq!(resolved.resource_name(), decl.resource, "{wire}");
        assert_eq!(resolved.revision.fingerprint(), &decl.fingerprint);
    }
    assert_eq!(cache.len(), 3);
    assert_eq!(cache.generation(), 1);
}

/// Without presence (a replay, `--contracts` alone), every revision held
/// offline sits under a symbolic `{system}/{service}`: declared, not
/// placed.
#[test]
fn without_presence_every_revision_is_declared_not_placed() {
    let set = contracts();
    let lens = Lens::new("", None, &set).offline(&set);
    let skel = Skeleton::build(&lens, None);
    let unplaced = skel
        .root
        .node(&["zk2", "{system}", "{service}"])
        .expect("a symbolic address");
    assert!(unplaced.evidence.declared && !unplaced.evidence.present);
    assert_eq!(unplaced.children.len(), set.len(), "one interface each");
    let plans = unplaced
        .node(&["mission_plan.v1", "state", "plans", "{vehicle}"])
        .and_then(|n| n.decl.as_ref())
        .expect("declared");
    assert_eq!(plans.basis, DeclBasis::Offline);
    assert_eq!(plans.address, None);
    assert_eq!(skel.coverage.revisions, set.len());
    assert_eq!(skel.coverage.addresses, 0);
    assert_eq!(skel.coverage.presence, None);
}

/// The merge's four statuses over the deployment: observed under a watch,
/// quiet under one, declared only where no watch reaches — an `@stream`
/// under an ambient watch, since `**` never crosses a verbatim chunk — and
/// a leftover with no watch at all.
#[test]
fn the_merge_tells_observed_quiet_declared_and_leftover_apart() {
    let set = contracts();
    let catalog = deployment(&set);
    let lens = Lens::new("site", Some(&catalog), &set);
    let skel = Skeleton::build(&lens, None);

    let mut stats = StatsTable::new();
    let now = Instant::now();
    for key in [
        "site/zk2/robot1/nav/nav.v2/stream/position",
        "site/zk2/host-a/tc/tc.netem.v1/state/config/ns0/eth0",
        "elsewhere/demo",
    ] {
        stats.record(key, 8, None, now, None, None);
    }
    let observed = KeyTreeSnapshot::build(&stats);
    let merged = merge_skeleton(&skel, &observed, &["site/zk2/**".to_string()]);
    let node = |path: &str| {
        let mut n = &merged;
        for chunk in path.split('/') {
            n = n
                .children
                .get(chunk)
                .unwrap_or_else(|| panic!("no merged node {chunk} in {path}"));
        }
        n
    };
    assert!(matches!(
        node("site/zk2/robot1/nav/nav.v2/stream/position").status,
        NodeStatus::Observed(e) if e.declared && e.present
    ));
    assert!(matches!(
        node("site/zk2/robot1/nav/nav.v2/state/status").status,
        NodeStatus::WatchedQuiet(_)
    ));
    // The observed `ns0/eth0` sits beside the symbolic `{ns}/{iface}`: the
    // tree shows both, the declared leaf quiet.
    assert!(matches!(
        node("site/zk2/host-a/tc/tc.netem.v1/state/config/ns0/eth0").status,
        NodeStatus::Observed(_)
    ));
    assert!(matches!(
        node("site/zk2/robot1/camera/camera.v1/@stream/image").status,
        NodeStatus::DeclaredOnly(_)
    ));
    assert!(matches!(
        node("elsewhere/demo").status,
        NodeStatus::Unwatched(e) if e == zenkey_fleet::Evidence::default()
    ));
    let decl = node("site/zk2/robot1/nav/nav.v2/@op/set_origin")
        .decl
        .as_ref()
        .expect("the merge carries the declaration");
    assert_eq!(decl.kind, Kind::Operation);
}
