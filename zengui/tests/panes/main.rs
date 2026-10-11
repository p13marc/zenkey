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
//!
//! One test binary, a module per surface (#614, FL5): the shared fixtures
//! below, the tests beside the surface they pin. Separate binaries would each
//! link iced and wgpu.

#[path = "../common/mod.rs"]
mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use common::slices;

use iced_test::simulator;
use zengui::history::HistoryRecorder;
use zengui::keyfacts::KeyFacts;
use zengui::message::Message;
use zengui::view::tokens::Spacing;
use zengui::view::tree::{self, FactsIndex};
use zenkey_fleet::KeyTreeSnapshot;
use zenkey_fleet::StatsTable;
use zenkey_fleet_v1::SliceSet;

const REGISTERED: &str = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used";
const UNREGISTERED: &str = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/no/such/thing";
const FOREIGN: &str = "demo/example/foo";

fn snapshot(keys: &[&str]) -> zenkey_fleet::MergedNode {
    let mut stats = StatsTable::new();
    let now = Instant::now();
    for k in keys {
        stats.record(k, 8, None, now, None, None);
    }
    let observed = KeyTreeSnapshot::build(&stats);
    // Pane tests watch everything: rows read Observed, as the bootstrap did.
    let skel = zenkey_fleet::Skeleton::build(
        &zenkey_fleet::Lens::new("", None, &zenkey_fleet::ContractSet::new()),
        None,
    );
    zenkey_fleet::merge_skeleton(&skel, &observed, &["**".to_string()])
}

/// The comfortable grid — panes render the same claims at either density
/// (#192), so the honesty tests pin them at the default.
fn sp() -> Spacing {
    Spacing::default()
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
    let mut cache = FactsIndex::default();
    for k in keys {
        cache.ensure("", k, with_slices.then_some(&slices));
    }
    cache
}

fn render(keys: &[&str], with_slices: bool) -> (tree::Flattened, FactsIndex) {
    let snap = snapshot(keys);
    let flat = tree::flatten(&snap, "", &expand_all(keys), 500, std::time::Instant::now());
    (flat, index(keys, with_slices))
}

mod admin;
mod blob;
mod chrome;
mod config;
mod detail;
mod doctor;
mod echo;
mod history;
mod media;
mod nodes;
mod replay;
mod sections;
mod send;
mod traffic;
mod tree_pane;
