//! Replay and recording: the retained window, `.zrec` and snapshots, and
//! the locks that hold in every window.

use super::*;

/// Everything the panes fold from, as bytes.
///
/// What is in: the whole observed tree (counts, bytes, EWMA rates —
/// `to_bits`, so a rate matches or the dump does not), the O6 counters,
/// the totals, the coverage statement, and every echo line verbatim
/// (`Debug` includes its seq, preview, encoding, tombstone flag and
/// attachment preview).
///
/// What is out, and why: arrival `Instant`s. A `.zrec` reader and the
/// ring are two observers, and RFC 09 §5.2's two-clocks rule is exactly
/// that they share offsets (`t`, which *is* compared, through every rate
/// and every fold) but not absolute clocks — the panes render those only
/// as ages against the frame's own "now", which no two runs share either.
fn canonical(app: &Zengui) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    fn node(out: &mut String, path: &str, n: &zenkey_fleet::TreeNode) {
        writeln!(
            out,
            "{path} c={} b={} r={:016x} sc={} sb={} sr={:016x} sk={}",
            n.count,
            n.bytes,
            n.rate_hz.to_bits(),
            n.subtree_count,
            n.subtree_bytes,
            n.subtree_rate_hz.to_bits(),
            n.subtree_keys,
        )
        .expect("write to string");
        for (chunk, child) in &n.children {
            node(out, &format!("{path}/{chunk}"), child);
        }
    }
    node(&mut out, "", &app.obs.observed.root);
    writeln!(
        out,
        "keys={} evicted={} unwatched={} totals=({},{},{:016x}) watched={:?}",
        app.obs.keys,
        app.obs.keys_evicted,
        app.obs.keys_unwatched,
        app.obs.totals.samples,
        app.obs.totals.bytes,
        app.obs.totals.rate_hz.to_bits(),
        app.obs.watched,
    )
    .expect("write to string");
    for line in app.work.echo.echo.iter() {
        writeln!(out, "{line:?}").expect("write to string");
    }
    out
}

/// #217's acceptance: a headless sim scrubs the retained window back 60 s
/// and the panes are **byte-identical** to a `.zrec` replay of the same
/// traffic at the same playhead — the ring and the file are two routes to
/// one window, not two windows.
///
/// The file half really is the retained window's save path: the bytes go
/// through [`crate::services::record::write_window`], i.e. "save window as
/// `.zrec`", and both apps are driven through the same `update()` messages
/// iced would deliver.
#[test]
fn a_retained_scrub_matches_a_zrec_replay_byte_for_byte() {
    use crate::view::replay::ReplayMsg;
    use std::sync::Arc as StdArc;

    let epoch = std::time::Instant::now();
    let views = traffic(epoch);
    let watched: StdArc<[String]> = StdArc::from(["v1/**".to_string()]);

    // Route one: the live monitor's ring, entered through the toggle's seam.
    let core = zenkey_fleet::MonitorCore::new(1024);
    for v in &views {
        core.ingest(v.clone(), None);
    }
    let mut app_a = test_app();
    app_a.obs.watched = StdArc::clone(&watched);
    let _ = update::pane::replay::enter_retained(
        &mut app_a.dep,
        &mut app_a.obs,
        &mut app_a.sub,
        &mut app_a.tree,
        &mut app_a.work,
        &core,
    );

    // Route two: the same window saved as a `.zrec` and opened from disk.
    let rows: Vec<StdArc<zenkey_fleet::SampleView>> =
        views.iter().cloned().map(StdArc::new).collect();
    let mut bytes = Vec::new();
    let samples = crate::services::record::write_window(
        &rows,
        epoch,
        vec!["v1/**".to_string()],
        String::new(),
        &mut bytes,
    )
    .expect("the window writes");
    assert_eq!(samples, views.len() as u64);
    let mut app_b = test_app();
    let open = |app: &mut Zengui, m: ReplayMsg| {
        let _ = app.update(Message::Workspace(crate::message::WorkspaceMsg::Replay(m)));
    };
    open(&mut app_b, ReplayMsg::OpenToggled);
    open(&mut app_b, ReplayMsg::PathChanged("w.zrec".to_string()));
    open(&mut app_b, ReplayMsg::Open);
    // The parse runs as a task now (#255): headless, we assert the loading
    // claim `Open` raised — a load in flight is a state the UI must say
    // (O4) — then land its result through the same message the task sends.
    assert_eq!(
        app_b.work.replay.replay_loading.as_deref(),
        Some("w.zrec"),
        "an open in flight is an explicit loading state, not a blank tab"
    );
    let state = crate::replay::ReplayState::load("w.zrec", bytes.as_slice()).expect("parses");
    open(
        &mut app_b,
        ReplayMsg::Loaded(
            "w.zrec".to_string(),
            Ok(crate::replay::LoadedReplay::new(state)),
        ),
    );
    assert!(
        app_b.work.replay.replay_loading.is_none(),
        "the landing clears the loading claim"
    );

    // The two windows agree about their own extent before any scrubbing.
    let span_us = app_a.work.replay.replay.as_ref().expect("retained").span_us;
    assert_eq!(
        span_us,
        app_b.work.replay.replay.as_ref().expect("file").span_us,
        "one window, two routes, one span"
    );
    assert_eq!(span_us, 88_000_000);

    // Scrub both to the newest edge, then back 60 s — the gesture the
    // feature exists for — through the same messages iced would deliver.
    for app in [&mut app_a, &mut app_b] {
        open(app, ReplayMsg::Scrubbed(span_us));
        open(app, ReplayMsg::Scrubbed(span_us - 60_000_000));
    }

    // 28 s in: the tombstoned key is still alive, the late keys not yet
    // seen — the panes show *then*, not a filtered now.
    assert_eq!(app_a.obs.keys, 3);
    assert_eq!(canonical(&app_a), canonical(&app_b), "byte-identical panes");

    // And forward again lands on the same folds too — a rebuild, not a
    // one-shot coincidence.
    for app in [&mut app_a, &mut app_b] {
        open(app, ReplayMsg::Scrubbed(span_us));
    }
    assert_eq!(app_a.obs.keys, 5, "a, b, c, m and the foreign key");
    assert_eq!(canonical(&app_a), canonical(&app_b));
}

/// A failed `.zrec` open lands like any other async result (#255): the
/// loading claim clears, the open row comes back holding the path that
/// failed, and the note renders beside the box with the mistake in it —
/// the failure has a surface, it does not vanish into a blank tab.
#[test]
fn a_failed_zrec_open_restores_the_row_with_its_note() {
    use crate::view::replay::ReplayMsg;

    let mut app = test_app();
    let open = |app: &mut Zengui, m: ReplayMsg| {
        let _ = app.update(Message::Workspace(crate::message::WorkspaceMsg::Replay(m)));
    };
    open(&mut app, ReplayMsg::OpenToggled);
    open(&mut app, ReplayMsg::PathChanged("missing.zrec".to_string()));
    open(&mut app, ReplayMsg::Open);
    assert_eq!(
        app.work.replay.replay_loading.as_deref(),
        Some("missing.zrec")
    );
    assert!(
        app.work.replay.replay_open.is_none(),
        "the row yields to the loading claim while the parse runs"
    );
    // A second Open while one is in flight is refused, not raced.
    open(&mut app, ReplayMsg::Open);

    open(
        &mut app,
        ReplayMsg::Loaded("missing.zrec".to_string(), Err("no such file".into())),
    );
    assert!(app.work.replay.replay_loading.is_none());
    assert_eq!(
        app.work.replay.replay_open.as_deref(),
        Some("missing.zrec"),
        "the row comes back with the path that failed"
    );
    assert_eq!(
        app.work.replay.replay_note.as_ref().map(|e| e.to_string()),
        Some("no such file".to_string())
    );
    assert!(app.work.replay.replay.is_none(), "no mode was entered");
}

/// A loaded `.zsnap` (#219) enters no mode and feeds no pane: it sits
/// beside the live world for the history section to compare against, its
/// open row behaves like the `.zrec` one — loading claim, failure note — and
/// closing it drops every slot's compare toggle with it.
#[test]
fn a_loaded_snapshot_is_held_beside_the_live_world_not_instead_of_it() {
    use crate::view::replay::ReplayMsg;

    let mut app = test_app();
    let open = |app: &mut Zengui, m: ReplayMsg| {
        let _ = app.update(Message::Workspace(crate::message::WorkspaceMsg::Replay(m)));
    };
    open(&mut app, ReplayMsg::SnapshotOpenToggled);
    open(
        &mut app,
        ReplayMsg::SnapshotPathChanged("a.zsnap".to_string()),
    );
    open(&mut app, ReplayMsg::SnapshotOpen);
    assert_eq!(app.work.replay.snapshot_loading.as_deref(), Some("a.zsnap"));
    assert!(app.work.replay.snapshot_open.is_none());

    open(
        &mut app,
        ReplayMsg::SnapshotLoaded("a.zsnap".to_string(), Err("no such file".into())),
    );
    assert!(app.work.replay.snapshot_loading.is_none());
    assert_eq!(
        app.work.replay.snapshot_open.as_deref(),
        Some("a.zsnap"),
        "the row comes back with the path that failed"
    );
    assert!(app.work.replay.snapshot_note.is_some());

    let snapshot = std::sync::Arc::new(zenkey_fleet_v1::Snapshot {
        header: zenkey_fleet_v1::ZsnapHeader {
            zsnap: zenkey_fleet_v1::ZSNAP_VERSION,
            selectors: vec!["v1/**".into()],
            base: String::new(),
            collected_at: "2026-09-06T00:00:00Z".into(),
            collection_span_s: 0.4,
            asked: 1,
            answered: 0,
            elided: 0,
            errors: 0,
            superseded: 0,
            roster: zenkey_fleet_v1::report::Asked::NotAsked,
        },
        rows: Vec::new(),
    });
    open(
        &mut app,
        ReplayMsg::SnapshotLoaded("a.zsnap".to_string(), Ok(snapshot.clone())),
    );
    assert_eq!(app.work.replay.snapshot.as_ref(), Some(&snapshot));
    assert!(app.work.replay.snapshot_note.is_none());
    assert!(
        app.work.replay.replay.is_none(),
        "a snapshot is read beside the live world — no mode was entered"
    );

    // The compare toggle is per slot, and closing the file drops it.
    let _ = app.update(Message::Pane(crate::message::PaneMsg::History(
        crate::message::SlotId::FOLLOW,
        crate::view::history::HistoryMsg::CompareSnapshotToggled,
    )));
    assert!(app.sub.follow.compare_snapshot);
    open(&mut app, ReplayMsg::SnapshotClosed);
    assert!(app.work.replay.snapshot.is_none());
    assert!(!app.sub.follow.compare_snapshot);
}

/// The retained window keeps replay mode's whole posture (#217): while it
/// feeds the panes the live pump is not built, a capture cannot start —
/// there is nothing live to capture — and the toggle is the way back.
#[test]
fn the_retained_window_is_replay_mode_with_all_its_locks() {
    use crate::replay::ReplaySource;
    use crate::view::replay::ReplayMsg;

    let epoch = std::time::Instant::now();
    let core = zenkey_fleet::MonitorCore::new(64);
    for v in traffic(epoch) {
        core.ingest(v, None);
    }
    let mut app = test_app();
    let _ = update::pane::replay::enter_retained(
        &mut app.dep,
        &mut app.obs,
        &mut app.sub,
        &mut app.tree,
        &mut app.work,
        &core,
    );
    let state = app.work.replay.replay.as_ref().expect("in the window");
    assert!(matches!(state.source, ReplaySource::Retained { .. }));

    // `subscription()` gates the live pump on `replay.is_none()` — being
    // `Some` *is* the lock, the same one a `.zrec` holds (#74).
    let _ = app.update(Message::Workspace(crate::message::WorkspaceMsg::Replay(
        ReplayMsg::RecordToggled,
    )));
    assert!(
        app.work.replay.recording.is_none(),
        "a window is not live traffic: recording must refuse to start"
    );

    // The toggle is its own exit.
    let _ = app.update(Message::Workspace(crate::message::WorkspaceMsg::Replay(
        ReplayMsg::RetainedToggled,
    )));
    assert!(app.work.replay.replay.is_none(), "back to live");
}

/// The replay locks hold in every window, because they are one lock (#186,
/// #74): the pump gate and the record refusal read `work.replay` — state of
/// the *application* — and `subscription()` is called once per app, not per
/// window, so a torn-off window has no seam of its own to leak through.
/// Tearing a dock off must therefore change the subscription set not at all,
/// and the refusals must answer a torn window's messages exactly as the main
/// window's — messages carry no window id, which is the structural fact this
/// test pins.
#[test]
fn replay_forbids_publishing_and_recording_in_every_window() {
    use crate::message::WorkspaceMsg;
    use crate::prefs::{DockRole, LayoutPreset};
    use crate::view::replay::ReplayMsg;

    let epoch = std::time::Instant::now();
    let core = zenkey_fleet::MonitorCore::new(64);
    for v in traffic(epoch) {
        core.ingest(v, None);
    }
    let mut app = booted();
    let _ = app.update(Message::Workspace(WorkspaceMsg::LayoutPreset(
        LayoutPreset::Watch,
    )));
    let _ = update::pane::replay::enter_retained(
        &mut app.dep,
        &mut app.obs,
        &mut app.sub,
        &mut app.tree,
        &mut app.work,
        &core,
    );
    assert!(app.work.replay.replay.is_some(), "in replay mode");
    let units_docked = app.subscription().units();

    // Tear Echo's home off onto the second monitor, mid-replay.
    let _ = app.update(Message::Workspace(WorkspaceMsg::TearOff(
        DockRole::Activity,
    )));
    assert!(app.work.replay.replay.is_some(), "the mode survived");
    assert_eq!(
        app.subscription().units(),
        units_docked,
        "a torn-off window adds no subscription: the pump gate stays the \
         application's one gate, with nothing per-window to forget"
    );

    // The record toggle — reachable from the torn Activity window's replay
    // tab — is the same message from any window, and the same refusal.
    let _ = app.update(Message::Workspace(WorkspaceMsg::Replay(
        ReplayMsg::RecordToggled,
    )));
    assert!(
        app.work.replay.recording.is_none(),
        "replay has nothing live to capture, whichever window asks"
    );

    // Publishing is the same lock (#614, FL10): a publish, a tombstone and a
    // call from the Send pane — reachable from a torn Workbench too — are
    // refused, the form says why, and no task is started.
    use crate::message::PaneMsg;
    use crate::view::send::{REPLAY_LOCK, SendMsg};
    let _ = app.update(Message::Workspace(WorkspaceMsg::PaneSelected(
        crate::message::RightPane::Send,
    )));
    let _ = app.update(Message::Workspace(WorkspaceMsg::TearOff(
        DockRole::Workbench,
    )));
    app.work.bench.send_form.key = "demo/example/foo".into();
    app.work.bench.send_form.retire_i_know = true;
    for msg in [SendMsg::Send, SendMsg::Retire] {
        let task = app.update(Message::Pane(PaneMsg::Send(msg.clone())));
        assert_eq!(task.units(), 0, "{msg:?}: nothing was started in replay");
        let form = &app.work.bench.send_form;
        assert!(!form.in_flight, "{msg:?}");
        assert_eq!(
            form.error.as_ref().map(ToString::to_string),
            Some(format!("refused: {REPLAY_LOCK}")),
            "{msg:?}"
        );
    }
    let task = app.update(Message::Pane(PaneMsg::Send(SendMsg::Submit)));
    assert_eq!(task.units(), 0, "a call starts nothing in replay");
    // The pane says so in the torn Workbench window.
    let bench = app.work.windows.window_of(DockRole::Workbench).unwrap();
    {
        use iced_test::simulator;
        let mut ui = simulator::<Message, _, _>(app.view(bench));
        assert!(
            ui.find(REPLAY_LOCK).is_ok(),
            "the lock is said where the send is"
        );
    }

    // And the banner that says so renders in the torn window too: a replayed
    // Echo stream on the second monitor must not look live.
    use iced_test::simulator;
    let torn = app.work.windows.window_of(DockRole::Activity).unwrap();
    let mut ui = simulator::<Message, _, _>(app.view(torn));
    assert!(
        ui.find("RETAINED").is_ok() || ui.find("REPLAY").is_ok(),
        "the replay banner crosses windows"
    );
}

/// #544: a recording is a mode the operator chose — its status says so in
/// the mode's own tone, beside a *secondary* "stop recording" (danger's rank
/// is the irreversible wire act's), in the app bar and on the Replay tab.
#[test]
fn a_recording_is_a_status_not_an_alarm() {
    use crate::message::{ActivityTab, WorkspaceMsg};
    use iced_test::simulator;

    let mut app = test_app();
    let _ = app.update(Message::Workspace(WorkspaceMsg::ActivityTab(
        ActivityTab::Replay,
    )));
    let (stop, _stopped) = tokio::sync::oneshot::channel();
    app.work.replay.recording = Some(crate::state::workspace::RecordingHandle {
        stop,
        path: "capture.zrec".into(),
    });
    let mut ui = simulator::<Message, _, _>(app.view(iced::window::Id::unique()));
    assert!(ui.find("recording").is_ok(), "the status names the mode");
    assert!(ui.find("stop recording").is_ok());
    assert!(
        ui.find(
            "current watches to capture.zrec — the location bar's 'stop recording' finishes the file"
        )
        .is_ok(),
        "the Replay tab says where it is writing"
    );
}

/// Open `file` as a replay, through the landing the load task sends; the
/// task it returns is the first tick's offline check.
fn open_replay(app: &mut Zengui, file: &[u8]) -> iced::Task<Message> {
    use crate::view::replay::ReplayMsg;
    let state = crate::replay::ReplayState::load("t.zrec", file).expect("a capture");
    app.update(Message::Workspace(crate::message::WorkspaceMsg::Replay(
        ReplayMsg::Loaded("t.zrec".into(), Ok(crate::replay::LoadedReplay::new(state))),
    )))
}

/// Scrub the replay to `t_us`.
fn scrub(app: &mut Zengui, t_us: u64) -> iced::Task<Message> {
    app.update(Message::Workspace(crate::message::WorkspaceMsg::Replay(
        crate::view::replay::ReplayMsg::Scrubbed(t_us),
    )))
}

/// A version-1 or -2 row named a v1 QoS profile, which zk2 does not read
/// (#614, FL10b): the Inspector says "QoS not recorded" of it where FL6b
/// showed default axes, and draws no verdict — while a version-3 row's
/// recorded axes are shown as observed.
#[test]
fn a_v1_or_v2_file_reads_qos_not_recorded() {
    use iced_test::simulator;
    let select = |app: &mut Zengui, key: &str| {
        let _ = app.update(Message::Subject(SubjectMsg::Select(Subject::Key(
            key.into(),
        ))));
    };

    let v2 = [
        r#"{"zrec":2,"selectors":["demo/**"],"base":"","captured_at":"x"}"#,
        r#"{"key":"demo/a","t":0,"bytes":"MQ==","qos":"sampled"}"#,
        r#"{"key":"demo/a","t":500000,"bytes":"Mg==","qos":"sampled"}"#,
    ]
    .join("\n");
    let mut app = test_app();
    let _ = open_replay(&mut app, v2.as_bytes());
    // The recorder starts at the selection: the next row feeds it.
    select(&mut app, "demo/a");
    let _ = scrub(&mut app, 500_000);
    let entry = app
        .sub
        .follow
        .history
        .as_ref()
        .unwrap()
        .ring
        .newest()
        .cloned();
    assert!(
        entry.as_ref().is_some_and(|e| !e.qos_recorded),
        "the row recorded no axes"
    );
    let mut ui = simulator::<Message, _, _>(app.view(iced::window::Id::unique()));
    assert!(ui.find(crate::replay::QOS_NOT_RECORDED).is_ok());
    assert!(
        ui.find(zenkey_fleet::WireQos::DEFAULT.token().as_str())
            .is_err(),
        "the view's default axes are not shown as observed"
    );

    let v3 = [
        r#"{"zrec":3,"selectors":["demo/**"],"base":"","captured_at":"x","excluded":["@stream","@state","@op","@zk","@adv"]}"#,
        r#"{"key":"demo/a","t":0,"bytes":"MQ==","qos_axes":"real_time/block/reliable"}"#,
        r#"{"key":"demo/a","t":500000,"bytes":"Mg==","qos_axes":"real_time/block/reliable"}"#,
    ]
    .join("\n");
    let mut app = test_app();
    let _ = open_replay(&mut app, v3.as_bytes());
    select(&mut app, "demo/a");
    let _ = scrub(&mut app, 500_000);
    let mut ui = simulator::<Message, _, _>(app.view(iced::window::Id::unique()));
    assert!(ui.find("real_time/block/reliable").is_ok());
    assert!(ui.find(crate::replay::QOS_NOT_RECORDED).is_err());
}

/// A replay resolves offline, never through the live catalog (#614, FL10b;
/// O4). The window's catalog names `robot1/nav` as a provider of `nav.v2`
/// from its tokens alone — no descriptor names a revision — so the live lens
/// stops a `robot1/nav` key at `NoRevision`. A capture of that key replays
/// through the offline lens instead: the tree merges against a skeleton with
/// no presence read, the live verdict is dropped, the replayed tick is
/// checked through the one revision held offline and lands valid, a live
/// batch landing meanwhile is not recorded, and leaving gives the tree back
/// to the deployment's skeleton.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replay_resolves_offline_never_through_the_live_catalog() {
    use zenkey_fleet::report::Conformance;
    use zenkey_model::grammar::{Addr, IfaceId, InstanceId, alive_key, instance_key};

    let mut app = test_app();
    let walkthrough =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2/walkthrough");
    let loaded = crate::services::contracts::load_all(&[walkthrough]);
    let iface: IfaceId = "nav.v2".parse().unwrap();
    let revision = loaded.set.of_iface(&iface).next().unwrap().clone();
    let _ = app.update(Message::Bus(crate::message::BusMsg::ContractsLoaded(
        Arc::new(loaded),
    )));

    // The live world: robot1/nav's tokens, no descriptor.
    let addr: Addr = "robot1/nav".parse().unwrap();
    let instance = InstanceId::from_u64(1);
    let keys = vec![
        instance_key(&addr, &instance).unwrap().to_string(),
        alive_key(
            &addr,
            &iface,
            &instance,
            &revision.fingerprint().hex().fp16(),
        )
        .unwrap()
        .to_string(),
    ];
    let observed = zenkey_fleet::model::catalog::Observed::from_keys("zk2/*/*/@zk/**", &keys, true);
    app.dep.catalog = Some(Arc::new(zenkey_fleet::Catalog::new(&observed)));
    app.dep.refresh_skeleton();

    let key = "zk2/robot1/nav/nav.v2/stream/position";
    let resource = revision
        .contract()
        .resources
        .iter()
        .find(|r| r.template.as_str() == "position")
        .unwrap();
    let payload = zenkey_fleet::Synth::new(7)
        .sample(&revision, resource, zenkey_fleet::Member::Type, 0)
        .unwrap()
        .bytes;
    let live = app.dep.lens_inputs().lens().check(
        key,
        zenkey_fleet::Member::Type,
        Some("application/protobuf"),
        &payload,
    );
    assert_eq!(
        live.identity.unresolved,
        Some(zenkey_fleet::report::Unresolved::NoRevision),
        "the live lens stops at the catalog"
    );
    app.work
        .verdicts
        .payloads
        .record(key, live.conformance.clone());

    // The capture: that key's sample, saved as a version-3 file.
    let epoch = std::time::Instant::now();
    let view = Arc::new(zenkey_fleet::SampleView {
        key: key.into(),
        payload: zenoh::bytes::ZBytes::from(payload.clone()),
        encoding: "application/protobuf".into(),
        kind: zenoh::sample::SampleKind::Put,
        timestamp: None,
        stamped_by: None,
        attachment: None,
        priority: zenoh::qos::Priority::DEFAULT,
        congestion_control: zenoh::qos::CongestionControl::DEFAULT,
        reliability: zenoh::qos::Reliability::DEFAULT,
        express: false,
        source: None,
        received: epoch,
    });
    let mut file = Vec::new();
    crate::services::record::write_window(
        &[view],
        epoch,
        vec!["**".into()],
        String::new(),
        &mut file,
    )
    .unwrap();
    let check = open_replay(&mut app, &file);

    // The tree merges against the replay's own skeleton: no presence read.
    let skeleton = app.tree.replay_skeleton.clone().expect("a replay skeleton");
    assert!(skeleton.coverage.presence.is_none(), "no presence read");
    assert!(
        app.dep
            .skeleton
            .as_ref()
            .is_some_and(|s| s.coverage.presence.is_some()),
        "the live skeleton read presence"
    );
    assert!(app.tree.merged_from(Some(&skeleton)));
    assert!(
        app.work.verdicts.payloads.get(key).is_none(),
        "the live verdict describes another world, and went"
    );
    {
        use iced_test::simulator;
        let mut ui = simulator::<Message, _, _>(app.view(iced::window::Id::unique()));
        assert!(
            ui.find(crate::view::status::skeleton_text(Some(&skeleton.coverage)).as_str())
                .is_ok(),
            "the strip states the replay's skeleton"
        );
        let note = crate::view::replay::lens_note(app.work.replay.replay.as_ref().unwrap());
        assert!(
            note.starts_with("resolved offline: 7 revisions held"),
            "{note}"
        );
        assert!(
            ui.find(note.as_str()).is_ok(),
            "the banner says how the replay resolves"
        );
    }

    // The replayed tick is checked offline, and lands valid.
    drive(&mut app, check).await;
    assert_eq!(
        app.work.verdicts.payloads.get(key).map(|v| &v.verdict),
        Some(&Conformance::Valid),
        "resolved through the one revision held offline, never the catalog"
    );
    // A live batch landing meanwhile is the live world's: not recorded.
    let _ = app.update(Message::Bus(crate::message::BusMsg::VerdictsChecked(vec![
        (key.into(), live.conformance.clone()),
    ])));
    assert_eq!(
        app.work.verdicts.payloads.get(key).map(|v| &v.verdict),
        Some(&Conformance::Valid)
    );

    // Leaving hands the tree back to the deployment's skeleton, and the
    // capture's verdicts go.
    let _ = app.update(Message::Workspace(crate::message::WorkspaceMsg::Replay(
        crate::view::replay::ReplayMsg::Exit,
    )));
    assert!(app.tree.replay_skeleton.is_none());
    assert!(app.work.verdicts.payloads.get(key).is_none());
}
