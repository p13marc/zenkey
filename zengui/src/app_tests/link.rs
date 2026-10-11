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
        zenkey_v1::schema::validate::Verdict::Valid,
    );
    app.tree.expanded.open("zk2/robot1");
    let store_before = app.dep.store.clone().expect("a store");

    let _ = app.update(Message::Deployment(DeploymentMsg::NamespaceSelected(
        "spray".into(),
    )));
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
    let store_after = app.dep.store.clone().expect("a fresh store");
    assert_eq!(
        app.dep.contract_counts.offline,
        app.dep.contracts.as_ref().unwrap().set.len()
    );
    assert_eq!(store_after.held().len(), store_before.held().len());
    assert_eq!(store_after.retrievals(), 0, "nothing carried over");
}
