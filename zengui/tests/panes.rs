//! Pane rendering, driven headlessly by `iced_test`.
//!
//! Views are free `fn …(&State, …) -> Element<'_, Message>` over plain state
//! precisely so they can be rendered standalone here — no window, no bus. What
//! these pin is that the *honesty* surfaces actually reach the screen: the
//! tri-state registration badge, the positional overlay, and the empty states
//! that explain themselves rather than implying a verdict.
//!
//! `iced_test` selects widgets by the text they contain, so `find("x")` is
//! literally "is this on screen".

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Instant;

use iced_test::simulator;
use zengui::keyfacts::KeyFacts;
use zengui::message::Message;
use zengui::view::tree::{self, FactsIndex};
use zenkey_fleet::stats::StatsTable;
use zenkey_fleet::{KeyTreeSnapshot, SliceSet};

const REGISTERED: &str = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used";
const UNREGISTERED: &str = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/no/such/thing";
const FOREIGN: &str = "demo/example/foo";

fn snapshot(keys: &[&str]) -> zenkey_fleet::skeleton::MergedNode {
    let mut stats = StatsTable::new();
    let now = Instant::now();
    for k in keys {
        stats.record(k, 8, None, now);
    }
    let observed = KeyTreeSnapshot::build(&stats);
    // Pane tests watch everything: rows read Observed, as the bootstrap did.
    let skel = zenkey_fleet::Skeleton::build("", &SliceSet::default(), &BTreeMap::new(), None);
    zenkey_fleet::skeleton::merge(&skel, &observed, &["**".to_string()])
}

fn slices() -> SliceSet {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixture-tests/registry");
    SliceSet::from_dirs(&[dir]).expect("fixture registry")
}

/// Expand every prefix of every key, so leaves are visible.
fn expand_all(keys: &[&str]) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for k in keys {
        let mut acc = String::new();
        for chunk in k.split('/') {
            acc = if acc.is_empty() {
                chunk.to_string()
            } else {
                format!("{acc}/{chunk}")
            };
            set.insert(acc.clone());
        }
    }
    set
}

fn index(keys: &[&str], with_slices: bool) -> FactsIndex {
    let slices = slices();
    keys.iter()
        .map(|k| {
            let mut f = KeyFacts::project("", k);
            if with_slices {
                f.resolve(&slices);
            }
            (k.to_string(), f)
        })
        .collect()
}

fn render(keys: &[&str], with_slices: bool) -> (tree::Flattened, FactsIndex) {
    let snap = snapshot(keys);
    let flat = tree::flatten(&snap, "", &expand_all(keys), 500);
    (flat, index(keys, with_slices))
}

/// The registration tri-state must actually reach the screen, and its states
/// must read differently (RFC 09 §5.1 O4).
#[test]
fn the_tree_renders_the_registration_state() {
    let keys = [REGISTERED, UNREGISTERED, FOREIGN];
    let (flat, facts) = render(&keys, true);
    let watches = BTreeSet::new();
    let mut ui = simulator::<Message, _, _>(tree::pane(&flat, &facts, None, &watches));

    assert!(
        ui.find("registered").is_ok(),
        "a registered subject should be badged"
    );
    assert!(
        ui.find("unregistered").is_ok(),
        "an unregistered subject should be badged differently"
    );
    // The registry-declared type name is the payoff of the whole overlay.
    assert!(
        ui.find("TelemetryPoint").is_ok(),
        "the declared type should be shown"
    );
    // The positional labels for the conforming subtree.
    for role in ["version", "origin", "class", "producer", "subject"] {
        assert!(ui.find(role).is_ok(), "missing role label {role:?}");
    }
}

/// Before a registry is loaded, nothing may be badged either way — that would
/// report a verdict never obtained (RFC 09 §5.1 O4).
#[test]
fn an_unresolved_tree_claims_neither_way() {
    let keys = [REGISTERED];
    let (flat, facts) = render(&keys, false);
    let watches = BTreeSet::new();
    let mut ui = simulator::<Message, _, _>(tree::pane(&flat, &facts, None, &watches));

    assert!(
        ui.find("unregistered").is_err(),
        "a key we never checked must not read as unregistered"
    );
    assert!(ui.find("registered").is_err(), "…nor as registered");
    // It is still a fully labelled conforming key — only the registry verdict
    // is withheld.
    assert!(ui.find("producer").is_ok());
}

/// A foreign key is rendered, and rendered *plainly* — no invented labels.
#[test]
fn foreign_keys_render_without_convention_labels() {
    let keys = [FOREIGN];
    let (flat, facts) = render(&keys, true);
    let watches = BTreeSet::new();
    let mut ui = simulator::<Message, _, _>(tree::pane(&flat, &facts, None, &watches));

    for role in ["version", "origin", "class", "producer", "subject"] {
        assert!(
            ui.find(role).is_err(),
            "a foreign key must not be labelled {role:?}"
        );
    }
    // …and no registry verdict is invented for it either.
    assert!(ui.find("unregistered").is_err());
}

/// The empty tree must explain itself rather than implying the bus is quiet
/// (RFC 05 §3.1).
#[test]
fn the_empty_tree_explains_itself() {
    let flat = tree::Flattened {
        rows: Vec::new(),
        truncated: 0,
    };
    let facts = FactsIndex::new();
    let watches = BTreeSet::new();
    let mut ui = simulator::<Message, _, _>(tree::pane(&flat, &facts, None, &watches));

    assert!(ui.find("Nothing observed yet").is_ok());
    // Selectors match a widget's whole text, so this is the full disclaimer.
    assert!(
        ui.find(
            "An empty tree is not a verdict about the bus (RFC 05 §3.1) — \
             it may simply mean nothing has published on the current scope."
        )
        .is_ok(),
        "the empty state must disclaim, not just name the emptiness"
    );
}
