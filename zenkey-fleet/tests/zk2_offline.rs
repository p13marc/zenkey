//! Contracts in hand without a bus (#612, FJ3): an offline `ContractSet`
//! from the repository's own `examples/zk2/` — its `.history` root (§9.7)
//! and its authoring files — and a `BundleStore` pre-provisioned from one
//! (§8.5: bundles on a side with no holder).

use std::path::Path;

mod util;
use util::zk2::{T, example, examples, iface};

use zenkey::Implementation;
use zenkey_fleet::bus::contracts::BundleStore;
use zenkey_fleet::model::catalog::{ContractSet, Contracts};
use zenkey_fleet::model::render::{Member, render_with};
use zenkey_fleet::report::{ContractSource, Rendered};

/// The committed histories verify (`examples/zk2/.history`, and the
/// profiles' `spec/profiles/.history`), and hold the current revision of
/// every walkthrough and tcgui contract and of `health.v1`'s standard
/// contract, read back from its bundle alone.
#[test]
fn a_history_root_loads_every_revision() {
    let (mut set, problems) = ContractSet::load_history(&examples().join(".history"));
    assert!(problems.is_empty(), "{problems:?}");
    let (profiles, problems) = ContractSet::load_history(&util::zk2::profiles().join(".history"));
    assert!(problems.is_empty(), "{problems:?}");
    assert!(
        profiles.of_iface(&iface("health.v1")).next().is_some(),
        "health.v1 is published in the profiles' own root"
    );
    set.extend(profiles);
    for path in [
        "walkthrough/camera.v1",
        "walkthrough/detections.v1",
        "profiles/health/health.v1",
        "tcgui/tc.netif.v1",
        "tcgui/tc.netem.v1",
    ] {
        let imp = Implementation::new(example(path));
        let r = set
            .get(imp.iface(), imp.fingerprint())
            .unwrap_or_else(|| panic!("{path}: its current revision is in the history"));
        assert_eq!(r.source(), ContractSource::History);
        let view = r.view();
        assert!(view.minor.is_none(), "a bundle carries no minor");
        assert_eq!(
            view.resources.len(),
            imp.contract().resources.len(),
            "{path}"
        );
    }

    // Decoding needs nothing but the history: Objects { frame_sequence: 3 }.
    let det = iface("detections.v1");
    let r = set.of_iface(&det).next().expect("detections.v1 in history");
    let out = render_with(
        r,
        "zk2/vehicle-01/detector/detections.v1/stream/objects",
        Member::Type,
        Some("application/protobuf"),
        &[0x08, 3],
    );
    assert!(
        matches!(&out.rendered, Rendered::Value { declared, .. } if declared == "detections.v1.Objects"),
        "{:?}",
        out.rendered
    );
}

/// An authoring directory loads every contract in it, with the
/// documentation a bundle does not carry, and names the file that is not a
/// contract instead of skipping it.
#[test]
fn an_authoring_directory_loads_and_names_what_is_not_a_contract() {
    let (set, problems) = ContractSet::load_dir(&examples().join("tcgui"));
    let ifaces: Vec<String> = set.iter().map(|r| r.iface().to_string()).collect();
    assert_eq!(ifaces, ["tc.netem.v1", "tc.netif.v1", "tc.scenario.v1"]);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(
        problems[0].at.ends_with("frontend.bindings.toml"),
        "{problems:?}"
    );
    let netif = set.of_iface(&iface("tc.netif.v1")).next().expect("netif");
    assert_eq!(netif.source(), ContractSource::File);
    let view = netif.view();
    assert_eq!(view.minor, Some(0));
    assert!(view.summary.is_some());
    assert_eq!(
        netif.fingerprint(),
        Implementation::new(example("tcgui/tc.netif.v1")).fingerprint(),
        "the same revision a service built from the file would serve"
    );

    let (empty, problems) = ContractSet::load_dir(Path::new("/nonexistent/zk2"));
    assert!(empty.is_empty());
    assert_eq!(problems.len(), 1);
    let (empty, problems) = ContractSet::load_history(Path::new("/nonexistent/zk2"));
    assert!(empty.is_empty());
    assert_eq!(problems.len(), 1);
}

/// A store seeded from an offline set holds those revisions without a
/// retrieval, and still says "not asked" for the rest.
#[test]
fn a_seeded_store_holds_without_retrieving() {
    let (set, _) = ContractSet::load_history(&examples().join(".history"));
    let store = BundleStore::new(T);
    store.seed(&set);
    let imp = Implementation::new(example("walkthrough/camera.v1"));
    let held = store.state(imp.iface(), imp.fingerprint()).expect("seeded");
    assert_eq!(
        held.revision().expect("held").fingerprint(),
        imp.fingerprint()
    );
    assert_eq!(store.retrievals(), 0);
    assert_eq!(store.held().len(), set.len());
    let ghost = zenkey_model::canonical::Fingerprint::parse(&format!("sha256:{}", "0".repeat(64)))
        .expect("a fingerprint");
    assert!(store.state(imp.iface(), &ghost).is_none(), "not asked");
}
