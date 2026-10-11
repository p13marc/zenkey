//! Subject slots (#181, #257): one tick fanned out to every slot, pins
//! that keep out of the reveal paths, and section messages routed by slot.

use super::*;

/// One sample on one key, for the slot fan-out tests (#257).
fn sample_on(key: &str, payload: &[u8]) -> Arc<zenkey_fleet::SampleView> {
    Arc::new(zenkey_fleet::SampleView {
        key: key.to_string(),
        payload: zenoh::bytes::ZBytes::from(payload.to_vec()),
        encoding: "application/json".to_string(),
        kind: zenoh::sample::SampleKind::Put,
        timestamp: None,
        stamped_by: None,
        attachment: None,
        priority: zenoh::qos::Priority::DEFAULT,
        congestion_control: zenoh::qos::CongestionControl::DEFAULT,
        reliability: zenoh::qos::Reliability::DEFAULT,
        express: false,
        source: None,
        received: std::time::Instant::now(),
    })
}

/// #257's acceptance, whole: two panes show two different keys, each with its
/// own live chart and history count; the tick feeds both **without a second
/// subscription** — the samples ride the one monitor stream, and the fan-out
/// is the slot loop in `apply_tick`; closing the pinned one drops exactly its
/// recorder and nothing of the follow slot's.
#[test]
fn two_panes_two_keys_each_with_its_own_recorder_fed_by_one_tick() {
    use crate::message::WorkspaceMsg;
    use crate::prefs::{DockRole, LayoutPreset};

    let pinned_key = "v1/h-3fa9c2d41b7e/state/sysinfo/health";
    let follow_key = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu";

    let mut app = booted();
    let _ = app.update(Message::Workspace(WorkspaceMsg::LayoutPreset(
        LayoutPreset::Explore,
    )));

    // Look at one key, and tear the Inspector off: the tear-off IS the pin.
    let _ = app.update(Message::Subject(SubjectMsg::Select(Subject::Key(
        pinned_key.to_string(),
    ))));
    let _ = app.update(Message::Workspace(WorkspaceMsg::TearOff(
        DockRole::Inspector,
    )));
    assert_eq!(app.sub.len(), 2, "the tear-off minted a slot");
    let window = app
        .work
        .windows
        .window_of(DockRole::Inspector)
        .expect("the pin has a window");
    let slot = app.work.windows.slot_of(window).expect("and a binding");
    assert!(slot.is_pin(), "a torn Inspector with a subject is a pin");
    assert!(
        !app.chrome
            .prefs
            .layout
            .torn
            .iter()
            .any(|t| t.role == DockRole::Inspector),
        "a pin is session-only: persisting the identity without its evidence \
         would be the identity-only freeze #257 rejects, one restart over"
    );

    // The selection moves on; the pin does not.
    let _ = app.update(Message::Subject(SubjectMsg::Select(Subject::Key(
        follow_key.to_string(),
    ))));
    assert_eq!(app.sub.follow.current.key(), Some(follow_key));
    assert_eq!(
        app.sub.slot(slot).unwrap().current.key(),
        Some(pinned_key),
        "the pinned slot still holds what was pinned"
    );

    // One tick, samples for both keys, no watch declared by either slot:
    // the fan-out is the slot loop, never a second subscription.
    assert!(app.obs.my_watches.is_empty());
    let mut t = tick(2, 0, 0, &Arc::from([]));
    t.samples = vec![
        sample_on(pinned_key, br#"{"ok":1}"#),
        sample_on(follow_key, br#"{"pct":40}"#),
        sample_on(pinned_key, br#"{"ok":2}"#),
    ];
    tick_into(&mut app, &t);
    assert!(app.obs.my_watches.is_empty(), "the tick declared nothing");

    let pinned = app.sub.slot(slot).unwrap();
    assert_eq!(
        pinned.history.as_ref().unwrap().ring.len(),
        2,
        "the pinned recorder holds its key's two samples"
    );
    assert!(
        pinned.series.is_some(),
        "and its chart is alive — rebuilt by the same tick"
    );
    let follow = &app.sub.follow;
    assert_eq!(
        follow.history.as_ref().unwrap().ring.len(),
        1,
        "the follow recorder holds only its own key's sample"
    );
    assert!(follow.series.is_some());

    // Closing the pinned window unpins: exactly its recorder drops, and the
    // follow slot keeps everything.
    let _ = app.update(Message::Workspace(WorkspaceMsg::WindowClosed(window)));
    assert_eq!(app.sub.len(), 1, "the slot went with its window");
    assert_eq!(
        app.sub.follow.history.as_ref().unwrap().ring.len(),
        1,
        "nothing of the follow slot's was dropped"
    );
    assert!(
        app.work.docks.is_open(DockRole::Inspector),
        "the role comes home to the grid (#186)"
    );
}

/// While an Inspector window is pinned it is not the Inspector's home
/// (#257): the reveal paths restore the *docked* Inspector — pointing the
/// selection at a window that holds a different subject would show the wrong
/// thing — and the pin stands beside it, stating what it is.
#[test]
fn a_pinned_inspector_does_not_capture_the_reveal_paths() {
    use crate::message::{RightPane, WorkspaceMsg};
    use crate::prefs::{DockRole, LayoutPreset};
    use iced_test::simulator;

    let key = "v1/h-3fa9c2d41b7e/state/sysinfo/health";
    let mut app = booted();
    let main = app.work.windows.main.expect("booted");
    let _ = app.update(Message::Workspace(WorkspaceMsg::LayoutPreset(
        LayoutPreset::Explore,
    )));
    let _ = app.update(Message::Subject(SubjectMsg::Select(Subject::Key(
        key.to_string(),
    ))));
    let _ = app.update(Message::Workspace(WorkspaceMsg::TearOff(
        DockRole::Inspector,
    )));
    let window = app.work.windows.window_of(DockRole::Inspector).unwrap();
    assert!(
        !app.work.docks.is_open(DockRole::Inspector),
        "the dock left the grid with the tear-off"
    );

    // The reveal restores the docked Inspector rather than focusing the pin
    // — the #186 redirect reads on the *claim*, and a pin claims another
    // subject.
    let _ = app.update(Message::Workspace(WorkspaceMsg::PaneSelected(
        RightPane::Inspector,
    )));
    assert!(
        app.work.docks.is_open(DockRole::Inspector),
        "the selection's Inspector is the docked one"
    );
    assert_eq!(
        app.work.windows.window_of(DockRole::Inspector),
        Some(window),
        "and the pinned window stands beside it"
    );

    // The pinned window says what it is, on the surface the ⇱ produced —
    // where a pin's failure (nothing pinned) would show too.
    {
        let mut pinned_ui = simulator::<Message, _, _>(app.view(window));
        assert!(
            pinned_ui
                .find(format!(
                    "pinned to {key} — the selection drives the docked Inspector, \
                     not this window; closing it unpins and drops this recording"
                ))
                .is_ok(),
            "a pinned Inspector states what it is pinned to"
        );
        let mut main_ui = simulator::<Message, _, _>(app.view(main));
        assert!(main_ui.find("locator").is_ok(), "the main window renders");
    }

    // Tearing off with nothing selected is not a pin: there is nothing to
    // pin, so the window follows the selection and says so.
    let _ = app.update(Message::Workspace(WorkspaceMsg::WindowClosed(window)));
    let _ = app.update(Message::Subject(SubjectMsg::Select(Subject::None)));
    let _ = app.update(Message::Workspace(WorkspaceMsg::TearOff(
        DockRole::Inspector,
    )));
    let window = app.work.windows.window_of(DockRole::Inspector).unwrap();
    assert_eq!(
        app.work.windows.slot_of(window),
        Some(crate::message::SlotId::FOLLOW)
    );
    let mut follow_ui = simulator::<Message, _, _>(app.view(window));
    assert!(
        follow_ui
            .find(
                "follows the selection — pins are made by tearing off the \
                 Inspector while a subject is selected"
            )
            .is_ok(),
        "a follow-bound window says the selection drives it"
    );
}

/// The slotted section messages come home (#257): a pinned Inspector's
/// Detail, History, Fields and Why speak their own slot, so two Inspectors
/// over two slots never write into each other's state — and a message for a
/// dropped slot lands nowhere rather than in a stranger.
#[test]
fn a_section_message_routes_to_its_slot_and_misses_a_dropped_one() {
    use crate::message::{PaneMsg, WorkspaceMsg};
    use crate::prefs::{DockRole, LayoutPreset};

    let key = "v1/h-3fa9c2d41b7e/state/sysinfo/health";
    let mut app = booted();
    let _ = app.update(Message::Workspace(WorkspaceMsg::LayoutPreset(
        LayoutPreset::Explore,
    )));
    let _ = app.update(Message::Subject(SubjectMsg::Select(Subject::Key(
        key.to_string(),
    ))));
    let _ = app.update(Message::Workspace(WorkspaceMsg::TearOff(
        DockRole::Inspector,
    )));
    let window = app.work.windows.window_of(DockRole::Inspector).unwrap();
    let slot = app.work.windows.slot_of(window).unwrap();

    // A leaf choice in the pinned window moves the pinned slot, not the
    // follow slot.
    let _ = app.update(Message::Pane(PaneMsg::Detail(
        slot,
        crate::view::detail::DetailMsg::LeafSelected("cpu.pct".into()),
    )));
    assert_eq!(
        app.sub.slot(slot).unwrap().series_leaf.as_deref(),
        Some("cpu.pct")
    );
    assert_eq!(app.sub.follow.series_leaf, None);

    // And after the slot is dropped, the same message lands nowhere: the
    // only surface that could display the result is gone with the window.
    let _ = app.update(Message::Workspace(WorkspaceMsg::WindowClosed(window)));
    let _ = app.update(Message::Pane(PaneMsg::Detail(
        slot,
        crate::view::detail::DetailMsg::LeafSelected("mem.used".into()),
    )));
    assert_eq!(
        app.sub.follow.series_leaf, None,
        "a dropped slot's message must not land in another slot"
    );
}
