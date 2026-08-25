//! The window's own tests: what a tick does to it, and what a base change
//! does to it (#175).
//!
//! A child module of [`app`](super) rather than a `mod tests` inside it,
//! because these are the only tests that need the whole `Zengui` — and its six
//! fields are private, destructured in exactly two places. A `#[path]` child
//! keeps that true while keeping `app.rs` about the Elm loop.
//!
//! Everything narrower moved with its subject: the placement table to
//! `state/`, the watch-coverage rule to `view/panes.rs`.

use std::sync::Arc;

use super::{Zengui, test_app};
use crate::message::{BusTick, Message, Subject, SubjectMsg};
use crate::state::tree::{KeyCounts, shape_held};
use crate::update;

/// Feed one tick through the five sub-states `apply_tick` names.
fn tick_into(app: &mut Zengui, tick: &BusTick) {
    update::bus::apply_tick(
        &mut app.dep,
        &mut app.obs,
        &mut app.sub,
        &mut app.tree,
        &mut app.work,
        tick,
    );
}

/// #179's first acceptance, and the defect exactly: `forget_deployment`
/// cleared ten collections and not this one, so every node the user had
/// ever opened outlived the deployment it belonged to — keyed to paths
/// that no longer exist, where a stale one re-expands a coincidentally
/// matching new subtree.
#[test]
fn switching_base_forgets_every_node_that_was_open() {
    let mut app = test_app();
    for i in 0..10_000 {
        app.tree.expanded.open(format!("v1/h-{i:04}/state/sysinfo"));
    }
    assert_eq!(app.tree.expanded.len(), 10_000);
    update::deployment::forget(&mut app.dep, &mut app.obs, &mut app.tree, &mut app.work);
    assert!(app.tree.expanded.is_empty());
    assert!(
        app.work.verdicts.admin.expanded_raw.is_empty(),
        "`AdminState::clear` is `*self = default()`, so this was already \
             true — asserted here so it stays true if that changes"
    );
}

/// The other half of the old checklist, as behaviour rather than a list.
///
/// `KEPT` named 43 fields and asserted only that `forget_deployment` did
/// not mention them. That is weaker than it reads: a field can be listed
/// as kept and still be clobbered by the same message through some other
/// path. This asks the question the user asks — "is my half-written
/// publish body still there?" — of the three groups that hold typing.
#[test]
fn switching_base_keeps_what_the_user_typed() {
    let mut app = test_app();
    app.work.bench.send_form.body = "{\"celsius\": 21.5}".into();
    app.work.bench.send_form.params = "origin=h-3fa9c2d41b7e".into();
    app.work.bench.context_form.connect = "tcp/10.0.0.1:7447".into();
    app.tree.tree_search = "sysinfo".into();
    app.sub.follow.current = Subject::Key("v1/h-3fa9c2d41b7e/state/sysinfo/health".into());
    app.chrome.prefs.zoom = 1.25;

    update::deployment::forget(&mut app.dep, &mut app.obs, &mut app.tree, &mut app.work);

    assert_eq!(app.work.bench.send_form.body, "{\"celsius\": 21.5}");
    assert_eq!(app.work.bench.send_form.params, "origin=h-3fa9c2d41b7e");
    assert_eq!(app.work.bench.context_form.connect, "tcp/10.0.0.1:7447");
    assert_eq!(app.tree.tree_search, "sysinfo");
    assert_eq!(
        app.sub.follow.current.key(),
        Some("v1/h-3fa9c2d41b7e/state/sysinfo/health"),
        "a selection follows the user, not the fleet — the panes then say \
             honestly that they have not asked about it yet"
    );
    assert_eq!(app.chrome.prefs.zoom, 1.25);
}

/// The proof for a deleted line.
///
/// `forget_deployment` used to reset `merged_cache`, and that line was
/// dead: `merged`'s validity check compares the skeleton `Option`s, and
/// after a forget the cached `Some` cannot match the new `None`. If both
/// were already `None`, all three merge inputs are identical and reuse is
/// *correct*. Deleting a line needs the same evidence as adding one.
#[test]
fn a_forgotten_deployment_needs_no_merge_cache_reset() {
    let mut app = test_app();
    app.dep.skeleton = Some(Arc::new(zenkey_fleet::Skeleton::build(
        "",
        &zenkey_fleet::SliceSet::default(),
        &std::collections::BTreeMap::new(),
        None,
    )));
    let with_skeleton = app.tree.merged(&app.dep, &app.obs);

    update::deployment::forget(&mut app.dep, &mut app.obs, &mut app.tree, &mut app.work);
    assert!(
        app.tree.merged_cache.is_some(),
        "the cache is deliberately not cleared — this test exists to \
             notice if someone reinstates the reset"
    );
    let after = app.tree.merged(&app.dep, &app.obs);
    assert!(
        !Arc::ptr_eq(&with_skeleton, &after),
        "a forgotten skeleton must not be served from cache"
    );
}

/// A tick that changes nothing about the key set or the watch set.
fn tick(keys: usize, evicted: u64, unwatched: u64, watched: &Arc<[String]>) -> BusTick {
    let stats = zenkey_fleet::StatsTable::new();
    BusTick {
        tree: Arc::new(zenkey_fleet::KeyTreeSnapshot::build(&stats)),
        samples: Vec::new(),
        lagged: 0,
        coalesced: 0,
        nodes: Vec::new(),
        keys,
        keys_evicted: evicted,
        keys_unwatched: unwatched,
        watched: Arc::clone(watched),
        seeded: Vec::new(),
        totals: Default::default(),
    }
}

/// #177's whole claim, and the thing criterion cannot say because it
/// cannot count allocations: at steady state the tree is walked once.
#[test]
fn steady_state_ticks_reuse_the_tree_shape() {
    let mut app = test_app();
    let watched: Arc<[String]> = Arc::from(["v1/**".to_string()]);
    for _ in 0..100 {
        tick_into(&mut app, &tick(7, 0, 0, &watched));
    }
    assert_eq!(
        (app.tree.shape_rebuilt, app.tree.shape_reused),
        (1, 99),
        "one rebuild to establish the shape, then ninety-nine retargets"
    );
}

/// Each of the four things that can move the shape, alone.
///
/// The `keys` rung needs the other two beside it: `keys` is a *current*
/// count, so a key added and evicted within one 250 ms window cancels out
/// — which is exactly why the trigger is a triple and not a number.
#[test]
fn a_new_key_an_eviction_an_unwatch_or_a_new_watch_set_all_rebuild() {
    let watched: Arc<[String]> = Arc::from(["v1/**".to_string()]);
    for (label, keys, evicted, unwatched) in [
        ("a new key", 8usize, 0u64, 0u64),
        ("an eviction", 7, 1, 0),
        ("an unwatch", 7, 0, 1),
    ] {
        let mut app = test_app();
        tick_into(&mut app, &tick(7, 0, 0, &watched));
        let before = app.tree.shape_rebuilt;
        tick_into(&mut app, &tick(keys, evicted, unwatched, &watched));
        assert_eq!(
            app.tree.shape_rebuilt,
            before + 1,
            "{label} changes the tree and must rebuild it"
        );
    }

    // A different watch set with identical counters: `is_covered` decides
    // `NodeStatus`, so the badge would freeze without this rung.
    let mut app = test_app();
    tick_into(&mut app, &tick(7, 0, 0, &watched));
    let before = app.tree.shape_rebuilt;
    let other: Arc<[String]> = Arc::from(["v1/**".to_string()]);
    tick_into(&mut app, &tick(7, 0, 0, &other));
    assert_eq!(
        app.tree.shape_rebuilt,
        before + 1,
        "a new watch-set Arc rebuilds even when every counter matches"
    );
}

/// The hazard the watch rung covers, named so it cannot be optimised away.
///
/// `Message::Subject(SubjectMsg::WatchReleased)` does **not** reflatten — it returns
/// `Task::none()` and has always relied on the unconditional tick rebuild
/// to repaint node status. Under a conditional trigger that would be a live
/// bug, except that releasing a watch fires `WatchChanged`, which is the
/// only thing that reassigns `watched` in `link.rs`, which swaps the `Arc`.
#[test]
fn a_released_watch_still_repaints_the_tree() {
    let mut app = test_app();
    let two: Arc<[String]> = Arc::from(["v1/a/**".to_string(), "v1/b/**".to_string()]);
    tick_into(&mut app, &tick(7, 0, 0, &two));
    let before = app.tree.shape_rebuilt;
    // The release: same keys, same counters, one selector fewer.
    let one: Arc<[String]> = Arc::from(["v1/a/**".to_string()]);
    tick_into(&mut app, &tick(7, 0, 0, &one));
    assert_eq!(app.tree.shape_rebuilt, before + 1);
}

/// Expanding, collapsing, typing in the find box and switching pivot all
/// change only *how* the tree is presented — so the merge behind it is
/// repetition, and at 50,000 keys it is the larger half of `reflatten`
/// (24.97 ms against 22.08 ms).
#[test]
fn a_presentation_change_does_not_re_merge_the_tree() {
    let mut app = test_app();
    app.tree.reflatten(&app.dep, &app.obs);
    let first = Arc::clone(&app.tree.merged_cache.as_ref().expect("cached").merged);

    app.tree.expanded.open("v1");
    app.tree.reflatten(&app.dep, &app.obs);
    assert!(
        Arc::ptr_eq(
            &first,
            &app.tree.merged_cache.as_ref().expect("cached").merged
        ),
        "an expand changes no input to the merge"
    );

    // A new observed snapshot is a different tree, and must not be served
    // from the cache.
    let stats = zenkey_fleet::StatsTable::new();
    app.obs.observed = Arc::new(zenkey_fleet::KeyTreeSnapshot::build(&stats));
    app.tree.reflatten(&app.dep, &app.obs);
    assert!(
        !Arc::ptr_eq(
            &first,
            &app.tree.merged_cache.as_ref().expect("cached").merged
        ),
        "a new snapshot is new evidence, whatever it contains"
    );
}

/// The predicate on its own, since it is the whole trigger.
#[test]
fn the_shape_trigger_reads_every_rung() {
    let a: Arc<[String]> = Arc::from(["x".to_string()]);
    let b: Arc<[String]> = Arc::from(["x".to_string()]);
    assert!(shape_held(
        KeyCounts {
            keys: 1,
            evicted: 2,
            unwatched: 3
        },
        KeyCounts {
            keys: 1,
            evicted: 2,
            unwatched: 3
        },
        &a,
        &a
    ));
    assert!(!shape_held(
        KeyCounts {
            keys: 1,
            evicted: 2,
            unwatched: 3
        },
        KeyCounts {
            keys: 2,
            evicted: 2,
            unwatched: 3
        },
        &a,
        &a
    ));
    assert!(!shape_held(
        KeyCounts {
            keys: 1,
            evicted: 2,
            unwatched: 3
        },
        KeyCounts {
            keys: 1,
            evicted: 3,
            unwatched: 3
        },
        &a,
        &a
    ));
    assert!(!shape_held(
        KeyCounts {
            keys: 1,
            evicted: 2,
            unwatched: 3
        },
        KeyCounts {
            keys: 1,
            evicted: 2,
            unwatched: 4
        },
        &a,
        &a
    ));
    assert!(
        !shape_held(
            KeyCounts {
                keys: 1,
                evicted: 2,
                unwatched: 3
            },
            KeyCounts {
                keys: 1,
                evicted: 2,
                unwatched: 3
            },
            &a,
            &b
        ),
        "equal contents, different Arc: the watch set was rebuilt, and \
             only a rebuild reassigns it"
    );
    // A replay seeking backwards moves the counters *down*, which is why
    // the comparison is equality and not a `>`.
    assert!(!shape_held(
        KeyCounts {
            keys: 9,
            evicted: 9,
            unwatched: 9
        },
        KeyCounts {
            keys: 1,
            evicted: 2,
            unwatched: 3
        },
        &a,
        &a
    ));
}

/// The two messages the re-entrancy commit introduced, driven through
/// `update` rather than by calling their handlers (#175).
///
/// `ShowInTree` used to be one arm that opened every prefix and set
/// `selected` inline. It is now two messages, and the risk of splitting it is
/// that one half silently stops arriving. So this asserts the *outcome* the
/// arm used to produce, through the same entry point iced uses.
#[test]
fn revealing_a_subtree_opens_every_prefix_and_selects_without_fetching() {
    use crate::message::WorkspaceMsg;

    let mut app = test_app();
    let path = "v1/h-3fa9c2d41b7e/state/sysinfo";

    let _ = app.update(Message::Workspace(WorkspaceMsg::Reveal(path.to_string())));
    for prefix in [
        "v1",
        "v1/h-3fa9c2d41b7e",
        "v1/h-3fa9c2d41b7e/state",
        "v1/h-3fa9c2d41b7e/state/sysinfo",
    ] {
        assert!(
            app.tree.expanded.contains(prefix),
            "{prefix} should be open after revealing {path}"
        );
    }

    let _ = app.update(Message::Subject(SubjectMsg::Select(Subject::Prefix(
        path.to_string(),
    ))));
    assert_eq!(app.sub.follow.current.path(), Some(path));
    assert!(
        app.sub.follow.fetched.is_none(),
        "a subtree prefix is not a key: selecting one must not leave a fetch \
         behind, because no producer publishes it (#85)"
    );
}

/// One subject, not two (#181).
///
/// Before this, a key lived in `sub.selected` and an origin in
/// `verdicts.node_selected`, so both could be set at once and the window had
/// two answers to "what am I looking at?". The nodes pane showed one and the
/// inspector the other. Now the second assignment displaces the first, and
/// everything derived from the displaced one goes with it.
#[test]
fn pointing_at_an_origin_stops_pointing_at_a_key() {
    let mut app = test_app();
    let key = "v1/h-3fa9c2d41b7e/state/sysinfo/health";

    let _ = app.update(Message::Subject(SubjectMsg::Select(Subject::Key(
        key.to_string(),
    ))));
    assert_eq!(app.sub.follow.current.key(), Some(key));
    assert!(
        app.sub.follow.history.is_some(),
        "a key subject records history (#63)"
    );

    let _ = app.update(Message::Subject(SubjectMsg::Select(Subject::Origin(
        "h-3fa9c2d41b7e".into(),
    ))));
    assert_eq!(app.sub.follow.current.origin(), Some("h-3fa9c2d41b7e"));
    assert_eq!(app.sub.follow.current.key(), None);
    assert!(
        app.sub.follow.history.is_none(),
        "the recorder followed the key that is no longer the subject — a \
         recorder outliving its subject is what made deselecting cost \
         something"
    );
    assert!(app.sub.follow.selected_latency.is_none());
}

/// A symbolic skeleton path is a key by grammar and not by fact.
///
/// `Subject::Key` is the fetchable variant, and `{var}` leaves are the one
/// case where that is true of the type and false of the world: the registry
/// declares the shape, no producer publishes the literal. So the fetch is
/// refused and the recorder is not created — both by the same `contains('{')`
/// test, in one place now rather than two.
#[test]
fn a_symbolic_key_is_selected_but_never_fetched_or_recorded() {
    let mut app = test_app();
    let symbolic = "v1/h-3fa9c2d41b7e/state/tc/iface/{iface}";
    let _ = app.update(Message::Subject(SubjectMsg::Select(Subject::Key(
        symbolic.to_string(),
    ))));

    assert_eq!(app.sub.follow.current.key(), Some(symbolic));
    assert!(
        app.sub.follow.history.is_none(),
        "nothing can be recorded for it"
    );
    assert!(app.sub.follow.fetched.is_none());
}

/// A superseded fetch says so, instead of pretending nothing was asked
/// (#181).
///
/// Three things used to go wrong when a reply landed for a key the user had
/// already moved past. The pane flipped to Detail for the wrong subject —
/// acknowledged in place as "a focus nit". `decoded` was cleared
/// unconditionally, so a late reply for key A wiped key B's rendering while B
/// was on screen. And the view filtered the stale result out with an
/// `Option`, which made "superseded" indistinguishable from "not asked" in the
/// one pane whose whole job is saying what was and was not asked (O4).
#[test]
fn a_fetch_for_a_stale_subject_supersedes_rather_than_replaces() {
    use std::sync::Arc;

    use crate::message::RightPane;
    use zenkey_fleet::FetchOutcome;

    let mut app = test_app();
    let stale = "v1/h-3fa9c2d41b7e/state/sysinfo/health";
    let current = "v1/h-3fa9c2d41b7e/state/tc/qdisc";

    // Look at one key, then move on before its answer arrives.
    let _ = app.update(Message::Subject(SubjectMsg::Select(Subject::Key(
        stale.to_string(),
    ))));
    let _ = app.update(Message::Subject(SubjectMsg::Select(Subject::Key(
        current.to_string(),
    ))));
    app.sub.follow.decoded = None;
    app.work.right_pane = RightPane::Nodes;

    let late = Arc::new(FetchOutcome::None {
        attempted: ["storage", "cache", "window"],
    });
    let _ = app.update(Message::Subject(SubjectMsg::ValueFetched(
        stale.to_string(),
        Ok(late),
    )));

    assert_eq!(
        app.work.right_pane,
        RightPane::Nodes,
        "a superseded answer must not steal the pane"
    );
    assert!(
        app.sub.follow.fetched.is_some(),
        "the answer is real evidence and is kept — the view decides it is \
         about something else"
    );
    assert_eq!(
        app.sub.follow.fetched.as_ref().map(|(k, _)| k.as_str()),
        Some(stale)
    );

    // And the view says which of the three states it is in.
    let data = |sub: &crate::state::SubjectState| match sub.follow.fetched.as_ref() {
        None => "not asked",
        Some((k, _)) if Some(k.as_str()) == sub.follow.current.key() => "landed",
        Some(_) => "superseded",
    };
    assert_eq!(data(&app.sub), "superseded");
}

/// Synthetic traffic for the retained-window tests (#217): ~88 s of
/// watched keys with an LWW update, a tombstone, an attachment and one
/// key that is not this convention at all (O1 — a window curates nothing).
fn traffic(epoch: std::time::Instant) -> Vec<zenkey_fleet::SampleView> {
    use std::time::Duration;
    use zenoh::sample::SampleKind;
    let view =
        |t_s: u64, key: &str, payload: &[u8], encoding: &str, kind, attachment: Option<&[u8]>| {
            zenkey_fleet::SampleView {
                key: key.to_string(),
                payload: zenoh::bytes::ZBytes::from(payload.to_vec()),
                encoding: encoding.to_string(),
                kind,
                timestamp: None,
                stamped_by: None,
                attachment: attachment.map(|a| zenoh::bytes::ZBytes::from(a.to_vec())),
                priority: zenoh::qos::Priority::DEFAULT,
                congestion_control: zenoh::qos::CongestionControl::DEFAULT,
                reliability: zenoh::qos::Reliability::DEFAULT,
                express: false,
                source: None,
                received: epoch + Duration::from_secs(t_s),
            }
        };
    let put = SampleKind::Put;
    vec![
        view(0, "v1/h-0123456789ab/state/p/a", b"1", "", put, None),
        view(
            5,
            "v1/h-0123456789ab/state/p/b",
            br#"{"ok":true}"#,
            "application/json",
            put,
            None,
        ),
        view(
            20,
            "v1/h-0123456789ab/telemetry/x/m",
            b"12345678",
            "",
            put,
            None,
        ),
        view(30, "v1/h-0123456789ab/state/p/a", b"2", "", put, None),
        view(40, "not/this/convention", b"x", "", put, None),
        view(
            55,
            "v1/h-0123456789ab/state/p/b",
            b"",
            "",
            SampleKind::Delete,
            None,
        ),
        view(
            60,
            "v1/h-0123456789ab/telemetry/x/m",
            b"87654321",
            "",
            put,
            Some(b"meta"),
        ),
        view(75, "v1/h-0123456789ab/state/p/a", b"3", "", put, None),
        view(88, "v1/h-0123456789ab/state/p/c", b"zzz", "", put, None),
    ]
}

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
    update::pane::replay::enter_retained(
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
        ReplayMsg::Loaded("missing.zrec".to_string(), Err("no such file".to_string())),
    );
    assert!(app.work.replay.replay_loading.is_none());
    assert_eq!(
        app.work.replay.replay_open.as_deref(),
        Some("missing.zrec"),
        "the row comes back with the path that failed"
    );
    assert_eq!(app.work.replay.replay_note.as_deref(), Some("no such file"));
    assert!(app.work.replay.replay.is_none(), "no mode was entered");
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
    update::pane::replay::enter_retained(
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

/// The app with its windows open (#186): what `boot` makes, minus the link —
/// `open_windows` mints real `window::Id`s without a display, because
/// `window::open` only *returns* a task.
fn booted() -> Zengui {
    let mut app = test_app();
    let _ = app.open_windows();
    app
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
    update::pane::replay::enter_retained(
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
/// reconnect-gated knobs change only the settings; and a registry change
/// takes the same forget path a base change does.
#[test]
fn a_tuning_apply_resizes_live_keeps_the_counters_and_forgets_on_registry() {
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
        registry: vec![],
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

    // A registry change is a SliceSource change: it takes the same forget
    // path a base change does, so verdicts about the old slices are dropped…
    app.tree.expanded.open("v1/h-0123456789ab/state");
    app.dep.slice_source = crate::view::status::SliceSource::Bus { count: 3 };
    tune(
        &mut app,
        Tuning {
            echo_lines: 500,
            max_keys: 9000,
            registry: vec![std::path::PathBuf::from("/tmp/registry")],
            ..base
        },
    );
    assert_eq!(
        app.dep.settings.registry,
        [std::path::PathBuf::from("/tmp/registry")]
    );
    assert!(app.tree.expanded.is_empty(), "the forget path ran");
    assert_eq!(
        app.dep.slice_source,
        crate::view::status::SliceSource::None,
        "the old slices' verdicts are dropped, not layered under new ones"
    );
    // …while the ring — session state, not fleet evidence — keeps both its
    // three retained lines and its loss counters through the forget.
    assert_eq!(app.work.echo.echo.len(), 3);
    assert_eq!(app.work.echo.echo.evicted(), 6);
    assert_eq!(app.work.echo.echo.lagged(), 4);
}

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
