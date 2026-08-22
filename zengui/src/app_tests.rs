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
use crate::state::tree::shape_held;
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
    app.work.bench.publish_form.body = "{\"celsius\": 21.5}".into();
    app.work.bench.call_form.params = "origin=h-3fa9c2d41b7e".into();
    app.work.bench.context_form.connect = "tcp/10.0.0.1:7447".into();
    app.tree.tree_search = "sysinfo".into();
    app.sub.current = Subject::Key("v1/h-3fa9c2d41b7e/state/sysinfo/health".into());
    app.chrome.prefs.zoom = 1.25;

    update::deployment::forget(&mut app.dep, &mut app.obs, &mut app.tree, &mut app.work);

    assert_eq!(app.work.bench.publish_form.body, "{\"celsius\": 21.5}");
    assert_eq!(app.work.bench.call_form.params, "origin=h-3fa9c2d41b7e");
    assert_eq!(app.work.bench.context_form.connect, "tcp/10.0.0.1:7447");
    assert_eq!(app.tree.tree_search, "sysinfo");
    assert_eq!(
        app.sub.current.key(),
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
    let stats = zenkey_fleet::stats::StatsTable::new();
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
        totals: (0, 0, 0.0),
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
    for (label, next) in [
        ("a new key", (8usize, 0u64, 0u64)),
        ("an eviction", (7, 1, 0)),
        ("an unwatch", (7, 0, 1)),
    ] {
        let mut app = test_app();
        tick_into(&mut app, &tick(7, 0, 0, &watched));
        let before = app.tree.shape_rebuilt;
        tick_into(&mut app, &tick(next.0, next.1, next.2, &watched));
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
    let stats = zenkey_fleet::stats::StatsTable::new();
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
    assert!(shape_held((1, 2, 3), (1, 2, 3), &a, &a));
    assert!(!shape_held((1, 2, 3), (2, 2, 3), &a, &a));
    assert!(!shape_held((1, 2, 3), (1, 3, 3), &a, &a));
    assert!(!shape_held((1, 2, 3), (1, 2, 4), &a, &a));
    assert!(
        !shape_held((1, 2, 3), (1, 2, 3), &a, &b),
        "equal contents, different Arc: the watch set was rebuilt, and \
             only a rebuild reassigns it"
    );
    // A replay seeking backwards moves the counters *down*, which is why
    // the comparison is equality and not a `>`.
    assert!(!shape_held((9, 9, 9), (1, 2, 3), &a, &a));
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
    assert_eq!(app.sub.current.path(), Some(path));
    assert!(
        app.sub.fetched.is_none(),
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
    assert_eq!(app.sub.current.key(), Some(key));
    assert!(
        app.sub.history.is_some(),
        "a key subject records history (#63)"
    );

    let _ = app.update(Message::Subject(SubjectMsg::Select(Subject::Origin(
        "h-3fa9c2d41b7e".into(),
    ))));
    assert_eq!(app.sub.current.origin(), Some("h-3fa9c2d41b7e"));
    assert_eq!(app.sub.current.key(), None);
    assert!(
        app.sub.history.is_none(),
        "the recorder followed the key that is no longer the subject — a \
         recorder outliving its subject is what made deselecting cost \
         something"
    );
    assert!(app.sub.selected_latency.is_none());
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

    assert_eq!(app.sub.current.key(), Some(symbolic));
    assert!(app.sub.history.is_none(), "nothing can be recorded for it");
    assert!(app.sub.fetched.is_none());
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
    app.sub.decoded = None;
    app.work.right_pane = RightPane::Echo;

    let late = Arc::new(FetchOutcome::None {
        attempted: ["storage", "cache", "window"],
    });
    let _ = app.update(Message::Subject(SubjectMsg::ValueFetched(
        stale.to_string(),
        Ok(late),
    )));

    assert_eq!(
        app.work.right_pane,
        RightPane::Echo,
        "a superseded answer must not steal the pane"
    );
    assert!(
        app.sub.fetched.is_some(),
        "the answer is real evidence and is kept — the view decides it is \
         about something else"
    );
    assert_eq!(
        app.sub.fetched.as_ref().map(|(k, _)| k.as_str()),
        Some(stale)
    );

    // And the view says which of the three states it is in.
    let data = |sub: &crate::state::SubjectState| match sub.fetched.as_ref() {
        None => "not asked",
        Some((k, _)) if Some(k.as_str()) == sub.current.key() => "landed",
        Some(_) => "superseded",
    };
    assert_eq!(data(&app.sub), "superseded");
}
