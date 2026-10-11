//! The link: what loads when the router is dead or an endpoint malformed,
//! and what a namespace switch reopens and forgets (#614, FL6a).

use super::*;

/// #196 in the GUI, after #501 made the session a client and #614 (FL6a)
/// made the declared world zk2's contracts: a router that does not answer
/// fails the open — the link says so — and `--contracts` load anyway. What
/// is on disk stays answerable when the transport is the half that failed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dead_router_still_loads_contracts() {
    use crate::message::LinkState;

    let app = boot_against("tcp/127.0.0.1:1").await;
    assert!(
        matches!(app.obs.link, LinkState::Failed(_)),
        "the link says the session never opened"
    );
    assert!(app.dep.raw.is_none() && app.dep.session.is_none());
    let Some(contracts) = app.dep.contracts.as_ref() else {
        panic!("the contracts answer without a session")
    };
    assert!(!contracts.set.is_empty(), "the walkthrough loaded");
    assert!(contracts.problems.is_empty(), "{:?}", contracts.problems);
    assert_eq!(
        app.dep.contract_counts.offline,
        contracts.set.len(),
        "the store is seeded with what loaded, and says so"
    );
}

/// The other half of the fork: an endpoint that does not parse is the user's
/// own error, and the contracts are not loaded past it — answering anyway
/// would hide the typo behind a window that looks configured.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_malformed_endpoint_loads_nothing_past_it() {
    use crate::message::LinkState;

    let app = boot_against("127.0.0.1:1").await;
    assert!(matches!(app.obs.link, LinkState::Failed(_)));
    assert!(
        app.dep.contracts.is_none(),
        "a config failure is not answered past"
    );
}

/// A namespace switch (decided on #585) reopens the session **in** the
/// namespace and forgets what the old one held — its contract store, the
/// verdicts judged through it, the tree's open paths — while the session in
/// no namespace and the contracts (the user's own input) stay. The picker
/// remembers the pick, and a landing for a namespace the window has since
/// left is dropped rather than read.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_namespace_switch_reopens_and_forgets() {
    use crate::message::{BusMsg, DeploymentMsg};

    // A real pair of sessions, peers listening on nothing in particular: a
    // namespace switch must not
    // need a router to be observable, only sessions to reopen.
    let open = |ns: &str| {
        let ns = ns.to_string();
        async move {
            zenkey_fleet::open_in_namespace(
                &ns,
                None,
                &[],
                &["tcp/127.0.0.1:0".into()],
                Some(false),
            )
            .await
            .expect("a listening session opens")
        }
    };
    let raw = zenkey_fleet::open_reporting(None, &[], &["tcp/127.0.0.1:0".into()], Some(false))
        .await
        .expect("a raw session opens");
    let mut app = test_app();
    let _ = app.update(Message::Bus(BusMsg::SessionsOpened(Ok(
        crate::services::link::Sessions {
            raw: raw.clone(),
            session: open("").await,
            namespace: String::new(),
        },
    ))));
    let walkthrough =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2/walkthrough");
    let _ = app.update(Message::Bus(BusMsg::ContractsLoaded(Arc::new(
        crate::services::contracts::load_all(&[walkthrough]),
    ))));
    app.work.verdicts.payloads.record(
        "zk2/robot1/nav/nav.v2/stream/position",
        zenkey_fleet::report::Conformance::Valid,
    );
    app.tree.expanded.open("zk2/robot1");
    let store_before = app.dep.store.clone();
    // What the presence link told the old namespace: a projection, and the
    // lens's catalog.
    let epoch = app.obs.presence_epoch;
    let _ = app.update(Message::Bus(BusMsg::Presence(epoch, presence_update())));
    assert!(app.dep.presence.is_some() && app.dep.catalog.is_some());

    let _ = app.update(Message::Deployment(DeploymentMsg::NamespaceSelected(
        "spray".into(),
    )));
    assert!(
        app.dep.presence.is_none() && app.dep.catalog.is_none() && app.dep.lens_feed.is_none(),
        "presence and the lens described the namespace the window left"
    );
    assert!(app.obs.presence_monitor.is_none());
    assert_ne!(
        app.obs.presence_epoch, epoch,
        "the old link's epoch is retired"
    );
    let _ = app.update(Message::Bus(BusMsg::Presence(epoch, presence_update())));
    assert!(
        app.dep.presence.is_none(),
        "a landing from the old link is dropped"
    );
    assert_eq!(app.dep.namespace(), "spray");
    assert_eq!(app.chrome.prefs.namespace.as_deref(), Some("spray"));
    assert!(
        app.dep.session.is_none(),
        "the old namespace's session is gone"
    );
    assert!(app.dep.raw.is_some(), "the session in no namespace stays");
    assert!(
        app.dep.contracts.is_some(),
        "the contracts are the user's input, and stay"
    );
    assert_eq!(
        app.work.verdicts.payloads.keys_checked(),
        0,
        "verdicts judged through the old namespace are forgotten"
    );
    assert!(
        app.tree.expanded.is_empty(),
        "the old tree's open paths too"
    );
    // A late landing for the namespace the window left is dropped, not
    // installed as the session of the one it is on.
    let _ = app.update(Message::Bus(BusMsg::NamespacedOpened(
        String::new(),
        Ok(open("").await),
    )));
    assert!(app.dep.session.is_none(), "a stale landing is not read");
    // The reopened session lands, with a fresh store seeded from the same
    // contracts — never the old one, which held the old deployment's
    // retrievals.
    let _ = app.update(Message::Bus(BusMsg::NamespacedOpened(
        "spray".into(),
        Ok(open("spray").await),
    )));
    assert!(app.dep.session.is_some());
    let store_after = app.dep.store.clone();
    assert_eq!(
        app.dep.contract_counts.offline,
        app.dep.contracts.as_ref().unwrap().set.len()
    );
    assert_eq!(store_after.held().len(), store_before.held().len());
    assert_eq!(store_after.retrievals(), 0, "nothing carried over");
}

/// A presence landing over an empty deployment: a complete seed of nothing,
/// and a catalog of it — enough to be told apart from "not read".
fn presence_update() -> Arc<crate::presence_link::PresenceUpdate> {
    let observed = zenkey_fleet::ObservedPresence::from_keys("zk2/*/*/@zk/**", &[], true);
    let mut presence = zenkey_fleet::LivePresence::new(zenkey_fleet::PresenceScope::all());
    let moved = presence.seed(&observed, std::time::SystemTime::now());
    Arc::new(crate::presence_link::PresenceUpdate {
        presence: Arc::new(presence),
        moved,
        catalog: Some(Arc::new(zenkey_fleet::Catalog::new(&observed))),
        generation: 7,
        seed: zenkey_fleet::SeedOutcome::Complete,
    })
}

/// The lens moving (#614, FL6a) keeps every badge and makes every one due:
/// a presence landing with a new generation adopts its catalog, and the
/// verdict judged under the old one stays on screen as the last true
/// reading while the next tick's batch rechecks it. A landing carrying the
/// generation already held moves nothing.
#[test]
fn a_lens_move_adopts_the_catalog_and_makes_every_verdict_due() {
    use crate::message::BusMsg;

    let mut app = test_app();
    let key = "zk2/robot1/nav/nav.v2/stream/position";
    app.work
        .verdicts
        .payloads
        .record(key, zenkey_fleet::report::Conformance::Valid);
    assert!(!app.work.verdicts.payloads.should_check(key));

    let update = presence_update();
    let _ = app.update(Message::Bus(BusMsg::Presence(
        app.obs.presence_epoch,
        Arc::clone(&update),
    )));
    assert_eq!(app.dep.lens_generation, 7);
    assert!(
        app.dep
            .catalog
            .as_ref()
            .is_some_and(|c| Arc::ptr_eq(c, update.catalog.as_ref().unwrap())),
        "the landing's catalog is the one the lens resolves through"
    );
    assert!(
        app.work.verdicts.payloads.get(key).is_some(),
        "the badge stays: the last true reading"
    );
    assert!(
        app.work.verdicts.payloads.should_check(key),
        "and is due under the new lens"
    );

    app.work
        .verdicts
        .payloads
        .record(key, zenkey_fleet::report::Conformance::Valid);
    let _ = app.update(Message::Bus(BusMsg::Presence(
        app.obs.presence_epoch,
        update,
    )));
    assert!(
        !app.work.verdicts.payloads.should_check(key),
        "the same lens again moves nothing"
    );
}
