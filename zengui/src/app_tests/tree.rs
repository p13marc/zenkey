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
/// dead: `merged`'s validity check compares the skeleton `Option`s, and
/// after a forget the cached `Some` cannot match the new `None`. If both
/// were already `None`, all three merge inputs are identical and reuse is
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
