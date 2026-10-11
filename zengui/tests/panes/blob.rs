//! The blob browser's honesty surfaces (#68): the pane must never report a
//! probe it did not run, must never offer a wildcard fetch, and must report a
//! root disagreement rather than resolving it.

use super::*;
use std::sync::Arc;
use zengui::blob::{BlobState, Probe};
use zengui::view::blob::section;
use zenkey_fleet_v1::report::{
    BlobAvailability, BlobHolder, BlobList, BlobListSource, BlobManifest, BlobProbeReport,
    BlobTierRow,
};

const ID: &str = "01jqz3demo0001";

fn holder(origin: &str, root: &str) -> BlobHolder {
    BlobHolder {
        origin: origin.into(),
        key: format!("v1/{origin}/@blob/artifact/{ID}"),
        availability: Some(BlobAvailability {
            chunk_count: 4,
            have: 4,
            complete: true,
        }),
        note: None,
        manifest: Some(BlobManifest {
            id: ID.into(),
            filename: Some("demo-bundle.bin".into()),
            total_len: 262_144,
            chunk_size: 65_536,
            chunk_count: 4,
            root: root.into(),
            created_ms: 0,
        }),
        unreadable: None,
        error: None,
    }
}

fn probed(state: &mut BlobState, holders: Vec<BlobHolder>, roots: Vec<String>) {
    state.probe = Probe::Done(Arc::new(BlobProbeReport {
        target: format!("artifact/{ID}"),
        tier: "artifact".into(),
        asked: vec![format!("v1/*/@blob/artifact/{ID}/have")],
        not_probed: None,
        answered: holders.len(),
        holders,
        roots,
        declared_by: vec![],
        slices_considered: 0,
    }));
}

/// O4, twice over: an unloaded registry is not "nobody serves blobs", and
/// an unrun probe is not "nobody holds it".
#[test]
fn the_blob_pane_never_reports_what_it_did_not_ask() {
    let state = BlobState::default();
    let mut ui = simulator::<Message, _, _>(section(&state, false, sp()));
    assert!(ui.find("no registry loaded").is_ok());
    assert!(ui.find("no probe yet").is_ok());
    assert!(
        ui.find("nothing has been asked — this is \"not asked\", not \"nobody holds it\"")
            .is_ok(),
        "the never-probed state must say why it is empty"
    );
    assert!(
        ui.find("no origin answered").is_err(),
        "an unrun probe must not render as a silent one"
    );
}

/// The tier table is a capability claim, and says so — and an unasked
/// roster reads differently from an empty one.
#[test]
fn the_tier_matrix_labels_declaration_not_possession() {
    let state = BlobState {
        list: Some(BlobList {
            tiers: vec![BlobTierRow {
                producer: "netring".into(),
                registry_version: "1.1".into(),
                tier: "artifact".into(),
                known_tier: true,
                endpoints: vec!["manifest".into(), "have".into()],
                algo: None,
                reference: Some("BlobReference".into()),
                encoding: None,
                since: None,
                description: None,
                origins: zenkey_fleet_v1::report::Asked::NotAsked,
            }],
            source: BlobListSource::RegistryDirs,
            slices_considered: 3,
            slices_without_blob: 2,
        }),
        ..Default::default()
    };
    let mut ui = simulator::<Message, _, _>(section(&state, true, sp()));
    assert!(ui.find("netring").is_ok());
    assert!(
        ui.find(
            "a declaration is a capability, never possession — probe below to ask who \
             actually holds one"
        )
        .is_ok()
    );
    assert!(
        ui.find("origins  — (roster not asked)").is_ok(),
        "an unasked roster must not read as 'no origin serves this'"
    );
}

/// #68's acceptance: the pane cannot express a wildcard-origin fetch. It
/// has no origin input at all, and until one holder is chosen the fetch
/// control produces no message.
#[test]
fn the_blob_pane_names_one_origin_and_offers_no_wildcard() {
    let mut state = BlobState::default();
    state.set_target(ID.into());
    state.dest_input = "/tmp/demo.bin".into();
    state.allow_unpinned = true;
    probed(
        &mut state,
        vec![
            holder("h-aaaaaaaaaaaa", "ab12"),
            holder("h-bbbbbbbbbbbb", "ab12"),
        ],
        vec!["ab12".into()],
    );

    let mut ui = simulator::<Message, _, _>(section(&state, false, sp()));
    assert!(ui.find("h-aaaaaaaaaaaa").is_ok());
    assert!(ui.find("h-bbbbbbbbbbbb").is_ok());
    assert!(
        ui.find("choose one holder — a fetch names exactly one origin (RFC 07 §2.5)")
            .is_ok(),
        "an unchosen holder must say why the fetch is not ready"
    );
    // Pressed with nothing selected, the control emits nothing.
    let _ = ui.click("fetch");
    assert!(
        ui.into_messages().next().is_none(),
        "a fetch with no chosen origin must not be dispatchable"
    );

    // Chosen: the button names the single origin it will talk to.
    state.holder = Some(1);
    let mut ui = simulator::<Message, _, _>(section(&state, false, sp()));
    assert!(
        ui.find("fetch from h-bbbbbbbbbbbb").is_ok(),
        "the control names the one origin, so 'from where?' is never guessed"
    );
}

/// RFC 07 §2.1: the id is a name and the root is the anchor, so two roots
/// under one id are a finding — the pane reports both and picks neither.
#[test]
fn the_blob_pane_flags_disagreeing_roots() {
    let mut state = BlobState::default();
    state.set_target(ID.into());
    probed(
        &mut state,
        vec![
            holder("h-aaaaaaaaaaaa", "ab12"),
            holder("h-cccccccccccc", "cd34"),
        ],
        vec!["ab12".into(), "cd34".into()],
    );
    let mut ui = simulator::<Message, _, _>(section(&state, false, sp()));
    assert!(ui.find("2 distinct content roots under one id").is_ok());
    assert!(
        ui.find(
            "the id is a name; the root is the anchor (RFC 07 §2.1). This is a finding, \
             not a tie-break — pin the root you mean and the fetch will refuse anything \
             else."
        )
        .is_ok()
    );
}

/// An unasked probe must not render as an empty holder list. Since RFC 07
/// v1.17 every tier has a probe endpoint, so the case left for this render
/// path is a store algorithm the reference client does not speak — the
/// fixture models that one.
#[test]
fn a_tier_two_probe_says_it_was_not_run() {
    let hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let mut state = BlobState::default();
    state.set_target(format!("store/sha256/{hash}"));
    state.probe = Probe::Done(Arc::new(BlobProbeReport {
        target: format!("store/sha256/{hash}"),
        tier: "store".into(),
        asked: vec![],
        not_probed: Some("the reference client speaks `blake3` only".into()),
        holders: vec![],
        answered: 0,
        roots: vec![],
        declared_by: vec!["logs".into()],
        slices_considered: 2,
    }));
    let mut ui = simulator::<Message, _, _>(section(&state, false, sp()));
    assert!(ui.find("not probed").is_ok());
    assert!(
        ui.find("no origin answered").is_err(),
        "an unasked probe must not read as a silent one"
    );
    assert!(
        ui.find(
            "declared by logs — a registry claim, not a statement that any of them holds \
             this object"
        )
        .is_ok()
    );
}
