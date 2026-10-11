//! The edits that re-shape a session: the selector editor (#187) and a
//! tuning apply (#188).

use super::*;

/// The key-expression editor's chain (#187), driven through `update`:
/// opening seeds the draft from the deployment's truth, a fork copies the
/// resolved selectors through `scope::selectors`, and an invalid draft is
/// refused with the validator's own words while the deployment stays
/// untouched.
#[test]
fn the_selector_editor_seeds_forks_and_refuses_invalid_drafts() {
    use crate::message::{ChromeMsg, PaneMsg};
    use crate::view::palette::{Overlay, PaletteMsg};
    use crate::view::scope_editor::ScopeMsg;

    let mut app = test_app();
    // Open: the draft is the deployment's current truth — Everything, so
    // read-only with no custom rows.
    let _ = app.update(Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(
        Overlay::Selectors,
    ))));
    assert!(!app.work.bench.scope_form.editing);
    assert!(app.work.bench.scope_form.rows.is_empty());

    // Fork: the resolved selectors of the current scope — for Everything,
    // the raw `**` sweep, never a hand-formatted string.
    let _ = app.update(Message::Pane(PaneMsg::Scope(ScopeMsg::Fork)));
    assert!(app.work.bench.scope_form.editing);
    assert_eq!(app.work.bench.scope_form.rows, ["**"]);

    // An invalid draft is refused where it is displayed; nothing moves.
    let _ = app.update(Message::Pane(PaneMsg::Scope(ScopeMsg::RowChanged(
        0,
        "demo/$*/x".into(),
    ))));
    let _ = app.update(Message::Pane(PaneMsg::Scope(ScopeMsg::Apply)));
    let status = app.work.bench.scope_form.status.clone().expect("a verdict");
    assert!(
        status.expect_err("must refuse").contains("RFC 03 §2"),
        "the refusal carries the validator's own words"
    );
    assert_eq!(
        app.dep.settings.scope,
        crate::scope::ScopePreset::Everything
    );
    assert!(
        app.dep.settings.selectors.is_empty(),
        "a refused draft must not half-write the scope"
    );

    // An empty draft is refused too — a custom scope needs at least one.
    let _ = app.update(Message::Pane(PaneMsg::Scope(ScopeMsg::RowRemoved(0))));
    let _ = app.update(Message::Pane(PaneMsg::Scope(ScopeMsg::Apply)));
    let status = app.work.bench.scope_form.status.clone().expect("a verdict");
    assert!(status.expect_err("must refuse").contains("at least one"));
}

/// The Settings apply (#188), driven through `update`: the echo ring
/// re-bounds live — no reconnect, no monitor restart — and **no resize ever
/// discards the counters that reported the old bound's cost**; the
/// reconnect-gated knobs change only the settings; and a contracts change
/// reloads them, dropping what was judged under the old ones (#614, FL6a).
#[test]
fn a_tuning_apply_resizes_live_keeps_the_counters_and_reloads_contracts() {
    use crate::message::DeploymentMsg;
    use crate::view::settings::Tuning;

    let mut app = test_app();
    let epoch = std::time::Instant::now();
    for v in traffic(epoch) {
        app.work.echo.echo.push(&v); // nine lines
    }
    app.work.echo.echo.record_lag(4);

    let tune = |app: &mut Zengui, t: Tuning| {
        let _ = app.update(Message::Deployment(DeploymentMsg::TuningApplied(t)));
    };
    let base = Tuning {
        echo_lines: 3,
        history_entries: 10,
        max_keys: 1000,
        timeout_secs: 5,
        eager: false,
        contracts: vec![],
    };

    // Shrink, live: the ring re-bounds in place, the trim is counted, and
    // nothing else about the session moved.
    tune(&mut app, base.clone());
    assert_eq!(app.dep.settings.echo_lines, 3);
    assert_eq!(app.work.echo.echo.len(), 3);
    assert_eq!(
        app.work.echo.echo.evicted(),
        6,
        "the shrink's trim is counted"
    );
    assert_eq!(app.work.echo.echo.lagged(), 4);
    let status = app
        .work
        .bench
        .settings_form
        .status
        .clone()
        .expect("a verdict");
    assert!(
        status
            .as_ref()
            .unwrap()
            .contains("echo ring 3 lines (live)")
    );

    // Raise: the acceptance invariant — a raised bound never discards the
    // counter that reported the old bound's cost.
    tune(
        &mut app,
        Tuning {
            echo_lines: 500,
            ..base.clone()
        },
    );
    assert_eq!(app.dep.settings.echo_lines, 500);
    assert_eq!(app.work.echo.echo.evicted(), 6, "raising must not clear it");
    assert_eq!(app.work.echo.echo.lagged(), 4);

    // …and the raise is remembered, on the settle timer like a drag —
    // never a disk write inside the handler.
    assert_eq!(app.chrome.prefs.echo_lines, Some(500));
    assert!(app.chrome.prefs_dirty, "the settle timer owes a write");

    // A reconnect-gated knob changes the settings and says so — nothing
    // else moves until the reconnect it is labelled with.
    tune(
        &mut app,
        Tuning {
            echo_lines: 500,
            max_keys: 9000,
            ..base.clone()
        },
    );
    assert_eq!(app.dep.settings.max_keys, 9000);
    let status = app
        .work
        .bench
        .settings_form
        .status
        .clone()
        .expect("a verdict");
    assert!(
        status
            .as_ref()
            .unwrap()
            .contains("max keys 9000 (on reconnect)"),
        "{status:?}"
    );

    // A contracts change reloads them (#614, FL6a): the settings and this
    // window's own memory take the new list — never the shared context —
    // and the load's landing drops every verdict judged under the old
    // contracts rather than layering new ones under it…
    let walkthrough =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2/walkthrough");
    app.work.verdicts.payloads.record(
        "zk2/robot1/nav/nav.v2/stream/position",
        zenkey_v1::schema::validate::Verdict::Valid,
    );
    tune(
        &mut app,
        Tuning {
            echo_lines: 500,
            max_keys: 9000,
            contracts: vec![walkthrough.clone()],
            ..base
        },
    );
    assert_eq!(
        app.dep.settings.contracts,
        std::slice::from_ref(&walkthrough)
    );
    assert_eq!(
        app.chrome.prefs.contracts,
        std::slice::from_ref(&walkthrough),
        "remembered by this window"
    );
    let status = app
        .work
        .bench
        .settings_form
        .status
        .clone()
        .expect("a verdict");
    assert!(status.unwrap().contains("contracts changed"));
    let _ = app.update(Message::Bus(crate::message::BusMsg::ContractsLoaded(
        Arc::new(crate::services::contracts::load_all(&[walkthrough])),
    )));
    assert_eq!(
        app.work.verdicts.payloads.keys_checked(),
        0,
        "the old contracts' verdicts are dropped, not layered under new ones"
    );
    assert!(
        app.dep
            .contracts
            .as_ref()
            .is_some_and(|c| !c.set.is_empty()),
        "the new contracts are held"
    );
    // …while the ring — session state, not fleet evidence — keeps both its
    // three retained lines and its loss counters.
    assert_eq!(app.work.echo.echo.len(), 3);
    assert_eq!(app.work.echo.echo.evicted(), 6);
    assert_eq!(app.work.echo.echo.lagged(), 4);
}
