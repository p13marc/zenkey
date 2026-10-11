//! A tick and a base change, against the tree: what is forgotten, what is
//! kept, and when the merge runs again.

use super::*;

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
/// dead: `merged`'s validity check compares the skeletons by identity, and
/// after a forget the cached one cannot match the new one — `None` until
/// #614, a fresh skeleton built from the contracts since FL6b. If both were
/// already `None`, all three merge inputs are identical and reuse is
/// *correct*. Deleting a line needs the same evidence as adding one.
#[test]
fn a_forgotten_deployment_needs_no_merge_cache_reset() {
    let mut app = test_app();
    app.dep.skeleton = Some(Arc::new(zenkey_fleet::Skeleton::build(
        &zenkey_fleet::Lens::new("", None, &zenkey_fleet::ContractSet::new()),
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

/// #614 (FL6b): the skeleton is zk2's, built from the lens's own inputs, and
/// rebuilt when the lens moves — and only then. Ticks move nothing; a
/// presence landing at the generation already held moves nothing; the next
/// generation rebuilds it, and the tree's shape follows even when no key
/// moved (the skeleton rung of the tick's shape check).
#[test]
fn the_skeleton_rebuilds_on_a_lens_move_and_only_then() {
    use crate::message::BusMsg;
    use crate::presence_link::PresenceUpdate;

    let mut app = test_app();
    let watched: Arc<[String]> = Arc::from(["**".to_string()]);
    let walkthrough =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2/walkthrough");
    let loaded = Arc::new(crate::services::contracts::load_all(&[walkthrough]));
    let _ = app.update(Message::Bus(BusMsg::ContractsLoaded(loaded)));
    let first = app
        .dep
        .skeleton
        .clone()
        .expect("built when the contracts land");
    assert!(
        first
            .root
            .node(&["zk2", "{system}", "{service}", "nav.v2", "@op", "reset"])
            .is_some(),
        "no presence read: every revision held, under a symbolic address"
    );
    assert_eq!(first.coverage.presence, None);
    let same = |app: &Zengui| Arc::ptr_eq(&first, app.dep.skeleton.as_ref().expect("built"));

    for _ in 0..5 {
        tick_into(&mut app, &tick(7, 0, 0, &watched));
    }
    assert!(same(&app), "a tick is not a lens move");

    let landing = |generation| {
        Arc::new(PresenceUpdate {
            presence: Arc::new(zenkey_fleet::LivePresence::new(
                zenkey_fleet::PresenceScope::all(),
            )),
            moved: Vec::new(),
            catalog: None,
            generation,
            seed: zenkey_fleet::SeedOutcome::Complete,
        })
    };
    let epoch = app.obs.presence_epoch;
    let held = app.dep.lens_generation;
    let _ = app.update(Message::Bus(BusMsg::Presence(epoch, landing(held))));
    assert!(same(&app), "presence at the generation held is not a move");

    let _ = app.update(Message::Bus(BusMsg::Presence(epoch, landing(held + 1))));
    assert!(!same(&app), "the next generation rebuilds the skeleton");
    assert!(
        app.tree.merged_from(app.dep.skeleton.as_ref()),
        "and the landing redraws the tree from it"
    );
    let second = app.dep.skeleton.clone().expect("built");
    for _ in 0..5 {
        tick_into(&mut app, &tick(7, 0, 0, &watched));
    }
    assert!(Arc::ptr_eq(
        &second,
        app.dep.skeleton.as_ref().expect("built")
    ));

    // A lens the tick polls moved (a background re-read): the skeleton is
    // rebuilt with no key, eviction or watch moving — and the tick's shape
    // check sees it, rather than retargeting a shape merged from the old.
    app.dep.lens_generation += 1;
    assert!(app.dep.refresh_skeleton());
    assert!(!app.dep.refresh_skeleton(), "nothing moved since");
    let before = app.tree.shape_rebuilt;
    tick_into(&mut app, &tick(7, 0, 0, &watched));
    assert_eq!(
        app.tree.shape_rebuilt,
        before + 1,
        "a rebuilt skeleton rebuilds the shape"
    );
    assert!(app.tree.merged_from(app.dep.skeleton.as_ref()));
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

/// A tick whose tree holds `keys`, under one `**` watch.
fn tick_with(keys: &[&str], watched: &Arc<[String]>) -> BusTick {
    let mut stats = zenkey_fleet::StatsTable::new();
    let now = std::time::Instant::now();
    for k in keys {
        stats.record(k, 8, None, now, None, None);
    }
    BusTick {
        tree: Arc::new(zenkey_fleet::KeyTreeSnapshot::build(&stats)),
        samples: Vec::new(),
        lagged: 0,
        coalesced: 0,
        keys: keys.len(),
        keys_evicted: 0,
        keys_unwatched: 0,
        watched: Arc::clone(watched),
        seeded: Vec::new(),
        totals: Default::default(),
    }
}

const POSITION: &str = "zk2/robot1/nav/nav.v2/stream/position";
const ROBOT2: &str = "zk2/robot2/nav/nav.v2/stream/position";
const ETH0: &str = "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0";

/// The tree's top-level chunks as the window drew them.
fn tops(app: &Zengui) -> Vec<String> {
    let flat = &app.tree.flat;
    flat.rows
        .iter()
        .filter(|r| r.depth == 0)
        .map(|r| flat.arena.chunk_str(r.chunk).to_string())
        .collect()
}

/// #614 (FL7): the pivots regroup the window's own tree by zk2's
/// positions — by system, by address, by interface and by kind — each
/// expansion namespaced `pivot:<key>:`, and a v1 key, foreign since the
/// port, under "(foreign)" in every one.
#[test]
fn the_pivots_regroup_the_tree_by_zk2_positions() {
    use crate::message::WorkspaceMsg;
    use crate::view::tree::Pivot;

    let mut app = test_app();
    let watched: Arc<[String]> = Arc::from(["**".to_string()]);
    let v1 = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu";
    tick_into(
        &mut app,
        &tick_with(&[POSITION, ROBOT2, ETH0, v1], &watched),
    );
    assert_eq!(tops(&app), ["v1", "zk2"], "chunk order: the wire as it is");

    for (pivot, want) in [
        (
            Pivot::System,
            vec!["(foreign)", "host-a", "robot1", "robot2"],
        ),
        (
            Pivot::Address,
            vec!["(foreign)", "host-a/tc", "robot1/nav", "robot2/nav"],
        ),
        (Pivot::Iface, vec!["(foreign)", "nav.v2", "tc.netif.v1"]),
        (Pivot::Kind, vec!["(foreign)", "state", "stream"]),
    ] {
        let _ = app.update(Message::Workspace(WorkspaceMsg::PivotSelected(pivot)));
        assert_eq!(tops(&app), want, "{pivot}");
        let first = app.tree.flat.arena.display(app.tree.flat.rows[0].path);
        assert!(
            first.starts_with(&format!("pivot:{}:", pivot.key())),
            "{pivot}: {first}"
        );
    }
}

/// #614 (FL7): the Diagnose preset pivots the Locator by address — v1's
/// origin pivot, on zk2's positions — so "why is this service silent?"
/// opens on the services side by side.
#[test]
fn the_diagnose_preset_pivots_by_address() {
    use crate::message::WorkspaceMsg;
    use crate::prefs::LayoutPreset;
    use crate::view::tree::Pivot;

    let mut app = test_app();
    let watched: Arc<[String]> = Arc::from(["**".to_string()]);
    tick_into(&mut app, &tick_with(&[POSITION, ETH0], &watched));
    let _ = app.update(Message::Workspace(WorkspaceMsg::LayoutPreset(
        LayoutPreset::Diagnose,
    )));
    assert_eq!(app.tree.pivot, Pivot::Address);
    assert_eq!(tops(&app), ["host-a/tc", "robot1/nav"]);
}

/// The catalog a presence read of `robot1/nav` serving `nav.v2` makes, its
/// interface token's revision named by no descriptor.
fn undescribed_catalog() -> Arc<zenkey_fleet::Catalog> {
    use zenkey_model::grammar::{Addr, IfaceId, InstanceId, Sha256Hex, alive_key, instance_key};
    let addr: Addr = "robot1/nav".parse().expect("an address");
    let iface: IfaceId = "nav.v2".parse().expect("an interface");
    let instance = InstanceId::from_u64(1);
    let fp = Sha256Hex::new(&"ab".repeat(32))
        .expect("a fingerprint")
        .fp16();
    let keys = vec![
        instance_key(&addr, &instance).expect("a key").to_string(),
        alive_key(&addr, &iface, &instance, &fp)
            .expect("a key")
            .to_string(),
    ];
    let observed = zenkey_fleet::model::catalog::Observed::from_keys("zk2/*/*/@zk/**", &keys, true);
    Arc::new(zenkey_fleet::Catalog::new(&observed))
}

/// #614 (FL7): a lens move resolves every identity the cache holds again,
/// at once — not on each key's next sample, which a state published once
/// may never send. Before any presence read a zk2 key stops at "presence
/// not read"; once the presence link lands a catalog, the same key reads
/// where the read puts it, with no tick in between, and the cache counts
/// what it dropped as invalidated, apart from its bound.
#[test]
fn a_lens_move_resolves_every_identity_again() {
    use crate::message::BusMsg;
    use crate::presence_link::PresenceUpdate;
    use zenkey_fleet::report::Unresolved;

    let mut app = test_app();
    let ghost = "zk2/robot9/nav/nav.v2/stream/position";
    app.dep.ensure_identity(POSITION);
    app.dep.ensure_identity(ghost);
    let rung = |app: &Zengui, key: &str| {
        app.dep
            .identity
            .get(key)
            .and_then(|r| r.identity.unresolved.clone())
    };
    assert_eq!(rung(&app, POSITION), Some(Unresolved::PresenceNotRead));

    let landing = Arc::new(PresenceUpdate {
        presence: Arc::new(zenkey_fleet::LivePresence::new(
            zenkey_fleet::PresenceScope::all(),
        )),
        moved: Vec::new(),
        catalog: Some(undescribed_catalog()),
        generation: app.dep.lens_generation + 1,
        seed: zenkey_fleet::SeedOutcome::Complete,
    });
    let epoch = app.obs.presence_epoch;
    let _ = app.update(Message::Bus(BusMsg::Presence(epoch, landing)));
    assert_eq!(
        rung(&app, POSITION),
        Some(Unresolved::NoRevision),
        "a provider is present now, and no descriptor names its revision"
    );
    assert_eq!(rung(&app, ghost), Some(Unresolved::NoProvider));
    assert_eq!(app.dep.identity.invalidated(), 2, "both resolved again");
    assert_eq!(
        app.dep.identity.evicted(),
        0,
        "a lens move is not the bound"
    );
}

/// #614 (FL7): a selection resolves its key at once — a declared key nobody
/// has published has an identity the moment it is selected — and a
/// symbolic skeleton path, which is no key, resolves nothing.
#[test]
fn selecting_a_key_resolves_it_at_once() {
    let mut app = test_app();
    let declared = "zk2/host-a/tc/tc.netif.v1/state/namespaces";
    let _ = app.update(Message::Subject(SubjectMsg::Select(Subject::Key(
        declared.into(),
    ))));
    assert!(app.dep.identity.get(declared).is_some());
    let symbolic = "zk2/host-a/tc/tc.netif.v1/state/interfaces/{ns}/{iface}";
    let _ = app.update(Message::Subject(SubjectMsg::Select(Subject::Key(
        symbolic.into(),
    ))));
    assert!(app.dep.identity.get(symbolic).is_none());
}

/// #614 (FL7): the budget window's two observations ride the live tick —
/// when each watch was first seen, kept while the watch is, and the gap
/// between ticks.
#[test]
fn live_ticks_note_when_each_watch_began() {
    let mut app = test_app();
    let one: Arc<[String]> = Arc::from(["**".to_string()]);
    tick_into(&mut app, &tick_with(&[POSITION], &one));
    let began = *app.obs.watch_since.get("**").expect("seen");
    let two: Arc<[String]> = Arc::from(["**".to_string(), "zk2/robot1/**".to_string()]);
    tick_into(&mut app, &tick_with(&[POSITION], &two));
    assert_eq!(
        app.obs.watch_since.get("**"),
        Some(&began),
        "a watch kept keeps when it began"
    );
    assert!(app.obs.watch_since.contains_key("zk2/robot1/**"));
    let only: Arc<[String]> = Arc::from(["zk2/robot1/**".to_string()]);
    tick_into(&mut app, &tick_with(&[POSITION], &only));
    assert!(
        !app.obs.watch_since.contains_key("**"),
        "a released watch goes"
    );
    assert!(
        app.obs.tick_gap.longest(std::time::Instant::now()) < std::time::Duration::from_secs(5),
        "three ticks in a row are no stall"
    );
}

/// A key only the capture below carries: `robot2/nav`'s status, which the
/// live world never published.
const REPLAY_ONLY: &str = "zk2/robot2/nav/nav.v2/state/status";

/// A window whose live world stops `robot1/nav`'s keys at "no revision" —
/// its catalog names the address from its tokens alone, no descriptor —
/// with the walkthrough contracts held offline, the position stream's live
/// identity resolved, and a replay open over a capture of that stream and
/// of [`REPLAY_ONLY`].
fn replaying() -> Zengui {
    use crate::view::replay::ReplayMsg;

    let mut app = test_app();
    let walkthrough =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2/walkthrough");
    let loaded = Arc::new(crate::services::contracts::load_all(&[walkthrough]));
    let _ = app.update(Message::Bus(crate::message::BusMsg::ContractsLoaded(
        loaded,
    )));
    // The live lens moves (a catalog adopted) through the bus's own door,
    // as the presence link would move it.
    let landing = Arc::new(crate::presence_link::PresenceUpdate {
        presence: Arc::new(zenkey_fleet::LivePresence::new(
            zenkey_fleet::PresenceScope::all(),
        )),
        moved: Vec::new(),
        catalog: Some(undescribed_catalog()),
        generation: app.dep.lens_generation + 1,
        seed: zenkey_fleet::SeedOutcome::Complete,
    });
    let epoch = app.obs.presence_epoch;
    let _ = app.update(Message::Bus(crate::message::BusMsg::Presence(
        epoch, landing,
    )));
    app.dep.ensure_identity(POSITION);

    let at = std::time::Instant::now();
    let view = |key: &str| {
        Arc::new(zenkey_fleet::SampleView {
            key: key.into(),
            payload: zenoh::bytes::ZBytes::from(b"x".to_vec()),
            encoding: String::new(),
            kind: zenoh::sample::SampleKind::Put,
            timestamp: None,
            stamped_by: None,
            attachment: None,
            priority: zenoh::qos::Priority::DEFAULT,
            congestion_control: zenoh::qos::CongestionControl::DEFAULT,
            reliability: zenoh::qos::Reliability::DEFAULT,
            express: false,
            source: None,
            received: at,
        })
    };
    let mut file = Vec::new();
    crate::services::record::write_window(
        &[view(POSITION), view(REPLAY_ONLY)],
        at,
        vec!["**".into()],
        String::new(),
        &mut file,
    )
    .expect("a capture");
    let state = crate::replay::ReplayState::load("t.zrec", file.as_slice()).expect("parses");
    let _ = app.update(Message::Workspace(crate::message::WorkspaceMsg::Replay(
        ReplayMsg::Loaded("t.zrec".into(), Ok(crate::replay::LoadedReplay::new(state))),
    )));
    app
}

/// The rung a cache holds for `key`, when it holds one.
fn rung_in(cache: &zenkey_fleet::IdentityCache, key: &str) -> Option<crate::keyfacts::Rung> {
    cache
        .get(key)
        .map(|r| crate::keyfacts::Rung::of(&r.identity))
}

/// What the window draws for `key` in the Inspector, selected.
fn drawn<'a>(app: &'a mut Zengui, key: &str) -> iced_test::Simulator<'a, Message> {
    let _ = app.update(Message::Subject(SubjectMsg::Select(Subject::Key(
        key.into(),
    ))));
    iced_test::simulator::<Message, _, _>(app.view(iced::window::Id::unique()))
}

/// #614 (FL7): a key only the capture carries gets its rung from the
/// replay's offline lens, the moment its row is fed — in the replay's own
/// cache, which the jump-to list reads, and never in the live one.
#[test]
fn a_replay_only_key_gets_its_rung_from_the_offline_lens() {
    use crate::keyfacts::Rung;
    use crate::view::palette::{Overlay, PaletteMsg};

    let mut app = replaying();
    let (cache, namespace) = app.work.replay.identities().expect("replaying");
    assert_eq!(namespace, "", "the capture's base");
    assert_eq!(
        rung_in(cache, REPLAY_ONLY),
        Some(Rung::Resolved),
        "the one revision of nav.v2 held offline resolves it"
    );
    assert!(
        app.dep.identity.get(REPLAY_ONLY).is_none(),
        "the live cache never saw it, and still has not"
    );
    let _ = app.update(Message::Chrome(crate::message::ChromeMsg::Palette(
        PaletteMsg::Open(Overlay::Keys),
    )));
    let mut ui = iced_test::simulator::<Message, _, _>(app.view(iced::window::Id::unique()));
    assert!(
        ui.find(REPLAY_ONLY).is_ok(),
        "the jump-to list offers the capture's keys while it replays"
    );
}

/// #614 (FL7): a key whose live rung differs from its offline one shows the
/// offline one while the replay runs. Live, the catalog stops the position
/// stream at "no revision"; the replay resolves it through the revision
/// held offline, and the Inspector draws that.
#[test]
fn a_replay_shows_the_offline_rung_where_the_live_one_differs() {
    use crate::keyfacts::Rung;

    let mut app = replaying();
    assert_eq!(rung_in(&app.dep.identity, POSITION), Some(Rung::NoRevision));
    let (cache, _) = app.work.replay.identities().expect("replaying");
    assert_eq!(rung_in(cache, POSITION), Some(Rung::Resolved));
    let mut ui = drawn(&mut app, POSITION);
    assert!(ui.find("resolved").is_ok(), "the offline rung is drawn");
    assert!(
        ui.find("no revision").is_err(),
        "and the live one is not, anywhere in the window"
    );
}

/// #614 (FL7): leaving the replay drops its identities with it, and the
/// window reads the live cache again — the live rung back, the
/// capture-only key gone from the jump-to list.
#[test]
fn exiting_the_replay_restores_the_live_identities() {
    use crate::view::palette::{Overlay, PaletteMsg};

    let mut app = replaying();
    let _ = app.update(Message::Workspace(crate::message::WorkspaceMsg::Replay(
        crate::view::replay::ReplayMsg::Exit,
    )));
    assert!(app.work.replay.identities().is_none());
    let mut ui = drawn(&mut app, POSITION);
    assert!(ui.find("no revision").is_ok(), "the live rung is back");
    assert!(ui.find("resolved").is_err());
    drop(ui);
    let _ = app.update(Message::Chrome(crate::message::ChromeMsg::Palette(
        PaletteMsg::Open(Overlay::Keys),
    )));
    let mut ui = iced_test::simulator::<Message, _, _>(app.view(iced::window::Id::unique()));
    assert!(
        ui.find(REPLAY_ONLY).is_err(),
        "a key only the capture carried is no live observation"
    );
}
