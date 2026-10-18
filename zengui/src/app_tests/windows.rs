//! The workspace and its windows: docks, splitters, tear-offs and the
//! named layout (#183, #186).

use super::*;

/// #180's acceptance, both halves in one causal chain: dragging a splitter
/// changes the ratio, and the changed layout is what a restart rebuilds.
///
/// The drag arrives as the widget's own `ResizeEvent` — the same message
/// `on_resize` produces — and the "restart" is `DockGrid::from_layout` over
/// the prefs the drag wrote, which is exactly what `with_prefs` does at
/// launch.
#[test]
fn a_splitter_drag_survives_restart() {
    use crate::message::WorkspaceMsg;
    use crate::prefs::{LayoutNode, LayoutPreset};
    use iced::widget::pane_grid;

    let mut app = test_app();
    let _ = app.update(Message::Workspace(WorkspaceMsg::LayoutPreset(
        LayoutPreset::Watch,
    )));
    assert_eq!(app.chrome.prefs.layout, LayoutPreset::Watch.layout());

    // Drag the root splitter (Watch: the top row against the Activity dock).
    let split = *app
        .work
        .docks
        .grid
        .layout()
        .splits()
        .next()
        .expect("Watch has a split");
    let _ = app.update(Message::Workspace(WorkspaceMsg::PaneResized(
        pane_grid::ResizeEvent { split, ratio: 0.81 },
    )));

    // The drag reached the persisted model, unnamed it, and marked the prefs
    // for the settle timer — never one file write per pixel (#189's lesson).
    assert_eq!(
        app.chrome.prefs.layout.preset, None,
        "a dragged layout is no longer the preset it started as"
    );
    assert!(app.chrome.prefs_dirty, "the settle timer owes a write");
    let LayoutNode::Split { ratio, .. } = &app.chrome.prefs.layout.root else {
        panic!("the captured layout lost its root split");
    };
    assert!((ratio - 0.81).abs() < 1e-6, "the ratio landed: {ratio}");

    // Restart: a fresh grid over the prefs the drag wrote shows the same
    // workspace — id-for-id equality is impossible (the ids are widget
    // counters), tree equality is the point.
    let reborn = crate::state::workspace::DockGrid::from_layout(&app.chrome.prefs.layout.root);
    assert_eq!(reborn.capture(), app.work.docks.capture());
}

/// #180's other acceptance: the workspace renders as a grid, with two docks
/// side by side — headlessly, over the whole `view`.
#[test]
fn the_workspace_renders_two_docks_side_by_side() {
    use iced_test::simulator;

    let app = test_app();
    // The default layout is Explore: Locator | Inspector.
    assert_eq!(
        app.chrome.prefs.layout,
        crate::prefs::LayoutPreset::Explore.layout()
    );
    // Any id an un-booted app is asked about renders the main workspace —
    // the window-aware dispatch (#186) falls back rather than blanking.
    let mut ui = simulator::<Message, _, _>(app.view(iced::window::Id::unique()));
    // Each dock's title bar names its role — both on screen at once, which
    // eleven mutually-exclusive tabs could never do.
    assert!(ui.find("locator").is_ok(), "the locator dock renders");
    assert!(ui.find("inspector").is_ok(), "the inspector dock renders");
}

/// Selecting a pane stopped meaning "swap the one visible surface" (#180):
/// `Inspector` reveals its dock without touching the workbench's tool, and a
/// tool selection lands in the workbench dock — restoring it if it was
/// closed, because a selection that changes nothing visible is a lie.
#[test]
fn pane_selection_is_dock_reveal_not_a_tab_swap() {
    use crate::message::{RightPane, WorkspaceMsg};
    use crate::prefs::DockRole;

    let mut app = test_app();
    assert!(
        !app.work.docks.is_open(DockRole::Workbench),
        "Explore opens no workbench"
    );

    let _ = app.update(Message::Workspace(WorkspaceMsg::PaneSelected(
        RightPane::Nodes,
    )));
    assert!(
        app.work.docks.is_open(DockRole::Workbench),
        "selecting a tool opens the workbench"
    );
    assert_eq!(app.work.right_pane, RightPane::Nodes);
    assert_eq!(
        app.chrome.prefs.layout.preset, None,
        "opening a dock bends the layout"
    );

    // The Inspector is a dock, not a workbench tool: selecting it must not
    // overwrite which tool the workbench shows.
    let _ = app.update(Message::Workspace(WorkspaceMsg::PaneSelected(
        RightPane::Inspector,
    )));
    assert_eq!(
        app.work.right_pane,
        RightPane::Nodes,
        "the workbench keeps its tool"
    );
    assert!(app.work.docks.is_open(DockRole::Inspector));
}

/// Alt+A's message (#190), driven through `update`: focusing a closed dock
/// restores it first — a focus nothing renders is a control that does
/// nothing — and only the restore bends the layout. Moving the focus alone
/// owes the prefs no write.
#[test]
fn a_dock_focus_key_restores_and_focuses() {
    use crate::message::WorkspaceMsg;
    use crate::prefs::DockRole;

    let mut app = test_app();
    // The default layout is Explore, which opens no Activity dock.
    assert!(!app.work.docks.is_open(DockRole::Activity));

    let _ = app.update(Message::Workspace(WorkspaceMsg::FocusDock(
        DockRole::Activity,
    )));
    assert!(
        app.work.docks.is_open(DockRole::Activity),
        "focusing a closed dock restores it"
    );
    assert_eq!(
        app.work.docks.focus,
        app.work.docks.pane_of(DockRole::Activity)
    );
    assert_eq!(
        app.chrome.prefs.layout.preset, None,
        "the restore bends the layout"
    );

    // An open dock: the focus moves, and nothing is owed a write.
    app.chrome.prefs_dirty = false;
    let _ = app.update(Message::Workspace(WorkspaceMsg::FocusDock(
        DockRole::Locator,
    )));
    assert_eq!(
        app.work.docks.focus,
        app.work.docks.pane_of(DockRole::Locator)
    );
    assert!(!app.chrome.prefs_dirty, "focus alone owes no write");
}

/// The tear-off round trip (#186): tearing a dock off leaves the grid by the
/// #180 close machinery and lands in the window set *and* the named layout;
/// closing its window is the way back, and the layout follows both moves.
#[test]
fn a_torn_dock_leaves_the_grid_and_its_windows_close_restores_it() {
    use crate::message::WorkspaceMsg;
    use crate::prefs::{DockRole, LayoutPreset};

    let mut app = booted();
    let _ = app.update(Message::Workspace(WorkspaceMsg::LayoutPreset(
        LayoutPreset::Watch,
    )));
    assert!(app.work.docks.is_open(DockRole::Activity));

    let _ = app.update(Message::Workspace(WorkspaceMsg::TearOff(
        DockRole::Activity,
    )));
    assert!(
        !app.work.docks.is_open(DockRole::Activity),
        "a torn dock is not also in the grid"
    );
    let id = app
        .work
        .windows
        .window_of(DockRole::Activity)
        .expect("the dock has a window now");
    assert_eq!(
        app.chrome
            .prefs
            .layout
            .torn
            .iter()
            .map(|t| t.role)
            .collect::<Vec<_>>(),
        [DockRole::Activity],
        "the tear-off is part of the named layout, so a restart reopens it"
    );
    assert_eq!(app.chrome.prefs.layout.preset, None, "torn is not Watch");
    assert!(app.chrome.prefs_dirty, "the settle timer owes a write");

    let _ = app.update(Message::Workspace(WorkspaceMsg::WindowClosed(id)));
    assert!(
        app.work.docks.is_open(DockRole::Activity),
        "closing the window re-docks the role at its home edge"
    );
    assert!(app.work.windows.torn.is_empty());
    assert!(
        app.chrome.prefs.layout.torn.is_empty(),
        "the layout follows the re-dock"
    );
    assert!(!app.work.windows.exiting, "a torn close is not a shutdown");
}

/// The two refusals (#186): the Locator never tears off — it *is* the
/// navigation — and the last dock in the grid stays, because a main window
/// with zero regions renders nothing and can never be clicked back.
#[test]
fn the_locator_and_the_last_dock_refuse_to_tear_off() {
    use crate::message::WorkspaceMsg;
    use crate::prefs::{DockRole, LayoutNode, WorkspaceLayout};

    let mut app = booted();
    let _ = app.update(Message::Workspace(WorkspaceMsg::TearOff(DockRole::Locator)));
    assert!(app.work.windows.torn.is_empty(), "the locator stays home");
    assert!(app.work.docks.is_open(DockRole::Locator));

    // A grid down to one dock: tearing it off would leave the main window
    // empty and unrecoverable, so the #180 close refusal holds here too.
    let mut app = Zengui::with_prefs(
        crate::app::test_settings(),
        crate::prefs::Prefs {
            layout: WorkspaceLayout::custom(LayoutNode::Dock(DockRole::Inspector)),
            ..crate::prefs::Prefs::default()
        },
        None,
    )
    .0;
    let _ = app.open_windows();
    let _ = app.update(Message::Workspace(WorkspaceMsg::TearOff(
        DockRole::Inspector,
    )));
    assert!(app.work.windows.torn.is_empty(), "the last dock stays");
    assert!(app.work.docks.is_open(DockRole::Inspector));
}

/// A dock has one window (#186): a second tear-off focuses the window it
/// already has rather than minting a twin — two windows showing one dock
/// would be two claims about one region.
#[test]
fn a_second_tear_off_is_a_focus_not_a_twin() {
    use crate::message::WorkspaceMsg;
    use crate::prefs::{DockRole, LayoutPreset};

    let mut app = booted();
    let _ = app.update(Message::Workspace(WorkspaceMsg::LayoutPreset(
        LayoutPreset::Watch,
    )));
    let _ = app.update(Message::Workspace(WorkspaceMsg::TearOff(
        DockRole::Activity,
    )));
    let id = app.work.windows.window_of(DockRole::Activity);
    let _ = app.update(Message::Workspace(WorkspaceMsg::TearOff(
        DockRole::Activity,
    )));
    assert_eq!(app.work.windows.torn.len(), 1, "one dock, one window");
    assert_eq!(app.work.windows.window_of(DockRole::Activity), id);
}

/// A role has one home (#186): while a dock's window is torn off, every
/// #180 reveal path — the dock strip's toggle, Alt+A's focus, choosing a
/// stream, selecting a pane — points at that window instead of restoring
/// the role into the grid, which would render one region in two windows.
#[test]
fn a_torn_dock_cannot_be_restored_into_the_grid_beside_its_window() {
    use crate::message::{ActivityTab, RightPane, WorkspaceMsg};
    use crate::prefs::{DockRole, LayoutPreset};

    let mut app = booted();
    let _ = app.update(Message::Workspace(WorkspaceMsg::LayoutPreset(
        LayoutPreset::Watch,
    )));
    let _ = app.update(Message::Workspace(WorkspaceMsg::TearOff(
        DockRole::Activity,
    )));
    let _ = app.update(Message::Workspace(WorkspaceMsg::TearOff(
        DockRole::Workbench,
    )));

    for reveal in [
        Message::Workspace(WorkspaceMsg::DockToggled(DockRole::Activity)),
        Message::Workspace(WorkspaceMsg::FocusDock(DockRole::Activity)),
        Message::Workspace(WorkspaceMsg::ActivityTab(ActivityTab::Echo)),
        Message::Workspace(WorkspaceMsg::PaneSelected(RightPane::Nodes)),
    ] {
        let _ = app.update(reveal);
        assert!(
            !app.work.docks.is_open(DockRole::Activity)
                && !app.work.docks.is_open(DockRole::Workbench),
            "a reveal of a torn dock must focus its window, not re-grid it"
        );
    }
    assert_eq!(app.work.windows.torn.len(), 2, "both windows still stand");
    assert_eq!(
        app.work.right_pane,
        RightPane::Nodes,
        "the tool selection still lands; only the restore is redirected"
    );
}

/// The issue's acceptance, and the daemon's sharp edge (#186): tear off the
/// Activity dock (Echo's home), then close the main window — that is a
/// shutdown. `iced::daemon` does not stop when its windows close, so without
/// the explicit exit in `WindowClosed(Main)` the process would keep running
/// with no window to reach it: the zombie the issue names.
#[test]
fn closing_the_main_window_is_a_shutdown_not_a_zombie() {
    use crate::message::WorkspaceMsg;
    use crate::prefs::{DockRole, LayoutPreset};

    let mut app = booted();
    let main = app.work.windows.main.expect("booted");
    let _ = app.update(Message::Workspace(WorkspaceMsg::LayoutPreset(
        LayoutPreset::Watch,
    )));
    let _ = app.update(Message::Workspace(WorkspaceMsg::TearOff(
        DockRole::Activity,
    )));

    // A stranger's close first: it must not be mistaken for the main window.
    let _ = app.update(Message::Workspace(WorkspaceMsg::WindowClosed(
        iced::window::Id::unique(),
    )));
    assert!(!app.work.windows.exiting, "a stale close is not a shutdown");

    let _ = app.update(Message::Workspace(WorkspaceMsg::WindowClosed(main)));
    assert!(
        app.work.windows.exiting,
        "the main window's close is the application's exit"
    );
    assert_eq!(
        app.work.windows.torn.len(),
        1,
        "torn windows die with the process — no restore races the exit"
    );
}

/// Per-window geometry, persisted per window (#186): the main window's size
/// still lands in `prefs.window`, a torn-off dock's size and position land
/// in the named layout's entry for it — and a closed window's late resize
/// lands nowhere.
#[test]
fn window_geometry_is_remembered_per_window_in_the_named_layout() {
    use crate::message::{ChromeMsg, WorkspaceMsg};
    use crate::prefs::{DockRole, LayoutPreset};

    let mut app = booted();
    let main = app.work.windows.main.expect("booted");
    let _ = app.update(Message::Workspace(WorkspaceMsg::LayoutPreset(
        LayoutPreset::Watch,
    )));
    let _ = app.update(Message::Workspace(WorkspaceMsg::TearOff(
        DockRole::Activity,
    )));
    let torn = app.work.windows.window_of(DockRole::Activity).unwrap();

    let _ = app.update(Message::Chrome(ChromeMsg::WindowResized(
        torn, 900.0, 300.0,
    )));
    let _ = app.update(Message::Chrome(ChromeMsg::WindowMoved(torn, 1920.0, 24.0)));
    let _ = app.update(Message::Chrome(ChromeMsg::WindowResized(
        main, 1500.0, 1000.0,
    )));

    assert_eq!(app.chrome.prefs.window, Some((1500.0, 1000.0)));
    let entry = &app.chrome.prefs.layout.torn[0];
    assert_eq!(entry.role, DockRole::Activity);
    assert_eq!(entry.size, Some((900.0, 300.0)));
    assert_eq!(
        entry.position,
        Some((1920.0, 24.0)),
        "the second monitor is the point: the position rides the layout"
    );
    assert!(app.chrome.prefs_dirty, "the settle timer owes one write");

    // A late event from a window nobody knows: recorded nowhere, and above
    // all not over the main window's geometry.
    let _ = app.update(Message::Chrome(ChromeMsg::WindowResized(
        iced::window::Id::unique(),
        50.0,
        50.0,
    )));
    assert_eq!(app.chrome.prefs.window, Some((1500.0, 1000.0)));
}

/// A preset is fully docked (#186): applying one re-docks every torn window
/// — the grid already holds the role, so the window is closed and forgotten
/// first, and its close event then classifies as a stranger's rather than
/// restoring the dock a second time.
#[test]
fn a_layout_preset_redocks_every_torn_window() {
    use crate::message::WorkspaceMsg;
    use crate::prefs::{DockRole, LayoutPreset};

    let mut app = booted();
    let _ = app.update(Message::Workspace(WorkspaceMsg::LayoutPreset(
        LayoutPreset::Watch,
    )));
    let _ = app.update(Message::Workspace(WorkspaceMsg::TearOff(
        DockRole::Activity,
    )));
    let id = app.work.windows.window_of(DockRole::Activity).unwrap();

    let _ = app.update(Message::Workspace(WorkspaceMsg::LayoutPreset(
        LayoutPreset::Watch,
    )));
    assert!(app.work.windows.torn.is_empty());
    assert!(app.work.docks.is_open(DockRole::Activity));
    assert_eq!(app.chrome.prefs.layout, LayoutPreset::Watch.layout());

    // The closed window's event arrives after: everything it would do is
    // done, and the grid must not grow a second Activity.
    let _ = app.update(Message::Workspace(WorkspaceMsg::WindowClosed(id)));
    assert_eq!(app.chrome.prefs.layout, LayoutPreset::Watch.layout());
}

/// A restart reopens what the layout remembers (#186): `boot` opens the main
/// window plus one window per persisted torn dock, at its remembered
/// geometry — the other half of the round trip the tear-off test starts.
#[test]
fn boot_reopens_the_torn_windows_the_layout_remembers() {
    use crate::prefs::{DockRole, LayoutPreset, TornDock, WorkspaceLayout};

    let mut layout = WorkspaceLayout::custom(LayoutPreset::Explore.root());
    layout.torn = vec![TornDock {
        role: DockRole::Activity,
        size: Some((960.0, 380.0)),
        position: None,
    }];
    let mut app = Zengui::with_prefs(
        crate::app::test_settings(),
        crate::prefs::Prefs {
            layout,
            ..crate::prefs::Prefs::default()
        },
        None,
    )
    .0;
    let _ = app.open_windows();
    assert!(app.work.windows.main.is_some());
    assert_eq!(app.work.windows.torn.len(), 1);
    let t = &app.work.windows.torn[0];
    assert_eq!(t.role, DockRole::Activity);
    assert_eq!(t.size, Some((960.0, 380.0)), "the geometry came back");
    assert!(
        !app.work.docks.is_open(DockRole::Activity),
        "the torn dock is not also in the grid"
    );
}

/// Two windows, one state, rendered independently and headlessly (#186):
/// the main window shows the grid without the torn dock, the torn window
/// shows that dock alone — both through `view(id)`, both through the same
/// free pane functions `tests/panes.rs` renders.
#[test]
fn two_windows_render_independently() {
    use crate::message::WorkspaceMsg;
    use crate::prefs::{DockRole, LayoutPreset};
    use iced_test::simulator;

    let mut app = booted();
    let main = app.work.windows.main.expect("booted");
    let _ = app.update(Message::Workspace(WorkspaceMsg::LayoutPreset(
        LayoutPreset::Watch,
    )));
    let _ = app.update(Message::Workspace(WorkspaceMsg::TearOff(
        DockRole::Activity,
    )));
    let torn = app.work.windows.window_of(DockRole::Activity).unwrap();

    let mut main_ui = simulator::<Message, _, _>(app.view(main));
    assert!(main_ui.find("locator").is_ok(), "the grid renders");
    assert!(
        main_ui.find("publish log").is_err(),
        "the torn dock left the main window"
    );

    let mut torn_ui = simulator::<Message, _, _>(app.view(torn));
    assert!(
        torn_ui.find("publish log").is_ok(),
        "the activity dock renders alone in its window"
    );
    assert!(
        torn_ui.find("locator").is_err(),
        "the torn window holds its dock and nothing else"
    );
}

/// A layout saved while the Config tool was open still loads (#614, FL11a
/// deleted v1's tool; #768, CD brought one back on config.v1). The named
/// layout never named a tool: it names docks (#180), and which tool the
/// Workbench shows is session state, Send at every launch. So the file an
/// older build wrote with Config on screen comes back whole — the
/// Workbench torn into its own window at its geometry, every tool on its
/// strip, config.v1's among them — and a hand edit naming the tool is
/// ignored like the retired `split`, never a refused launch.
#[test]
fn a_layout_saved_with_the_config_tool_open_still_loads() {
    use crate::message::RightPane;
    use crate::prefs::{DockRole, Prefs};
    use iced_test::simulator;

    let dir = std::env::temp_dir().join("zengui-prefs-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config-tool-layout.toml");
    std::fs::write(
        &path,
        "theme = \"light\"\n\
         right_pane = \"config\"\n\
         [layout]\n\
         [layout.root.split]\naxis = \"vertical\"\nratio = 0.3\n\
         [layout.root.split.a]\ndock = \"locator\"\n\
         [layout.root.split.b]\ndock = \"inspector\"\n\
         [[layout.torn]]\nrole = \"workbench\"\nsize = [520.0, 700.0]\ntool = \"config\"\n",
    )
    .unwrap();
    let (prefs, note) = Prefs::load_from(&path);
    assert!(note.is_none(), "it loads, with no note: {note:?}");
    assert_eq!(prefs.layout.torn.len(), 1, "the torn Workbench is kept");
    assert_eq!(prefs.layout.torn[0].role, DockRole::Workbench);

    let mut app = Zengui::with_prefs(crate::app::test_settings(), prefs, None).0;
    let _ = app.open_windows();
    let bench = app
        .work
        .windows
        .window_of(DockRole::Workbench)
        .expect("the Workbench reopens in its own window");
    assert_eq!(
        app.work.windows.torn[0].size,
        Some((520.0, 700.0)),
        "at the geometry it was saved with"
    );
    assert_eq!(app.work.right_pane, RightPane::Send, "the launch tool");
    let mut ui = simulator::<Message, _, _>(app.view(bench));
    for tool in ["send", "nodes", "admin", "mesh", "config"] {
        assert!(ui.find(tool).is_ok(), "the {tool} tool is on the strip");
    }
}
