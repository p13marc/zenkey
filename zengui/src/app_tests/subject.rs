//! The subject: revealing, pointing, symbolic keys, and a stale fetch.

use super::*;

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
fn pointing_at_an_address_stops_pointing_at_a_key() {
    let mut app = test_app();
    let key = "zk2/robot1/nav/nav.v2/state/status";

    let _ = app.update(Message::Subject(SubjectMsg::Select(Subject::Key(
        key.to_string(),
    ))));
    assert_eq!(app.sub.follow.current.key(), Some(key));
    assert!(
        app.sub.follow.history.is_some(),
        "a key subject records history (#63)"
    );

    let nav: zenkey_model::grammar::Addr = "robot1/nav".parse().expect("an address");
    let _ = app.update(Message::Subject(SubjectMsg::Select(Subject::Address(
        nav.clone(),
    ))));
    assert_eq!(app.sub.follow.current.address(), Some(&nav));
    assert_eq!(app.sub.follow.current.key(), None);
    assert!(
        app.sub.follow.history.is_none(),
        "the recorder followed the key that is no longer the subject — a \
         recorder outliving its subject is what made deselecting cost \
         something"
    );
    assert!(app.sub.follow.selected_latency.is_none());
}

/// A health reading lands only on the address the window points at, in the
/// namespace it shows (#614, FL9; #109) — and with no session in the
/// namespace, selecting an address asks nothing, so nothing is "reading".
#[test]
fn a_health_reading_lands_only_on_the_selected_address_in_this_namespace() {
    use crate::nodes::HealthState;
    use crate::view::nodes::NodesMsg;
    use zenkey_model::grammar::Addr;

    let report = |addr: &Addr| {
        std::sync::Arc::new(zenkey_fleet::judge_health(
            &zenkey_fleet::HealthObservation {
                namespace: String::new(),
                target: zenkey_fleet::HealthTarget::One(addr.clone()),
                spec: zenkey_fleet::HealthSpec {
                    timeout: std::time::Duration::from_secs(1),
                    window: None,
                    grace: std::time::Duration::from_secs(1),
                    clocks_synced: false,
                    face: None,
                },
                presence: None,
                first: Err("no session".into()),
                second: Err("no session".into()),
                window: None,
                archived: None,
            },
        ))
    };
    let nav: Addr = "robot1/nav".parse().expect("an address");
    let tc: Addr = "host-a/tc".parse().expect("an address");
    let mut app = test_app();
    let _ = app.update(Message::Subject(SubjectMsg::Select(Subject::Address(
        nav.clone(),
    ))));
    assert!(
        matches!(app.work.verdicts.nodes.health, HealthState::NotAsked),
        "no session in the namespace: nothing was asked, and nothing says it was"
    );
    let land = |app: &mut crate::app::Zengui, addr: &Addr, ns: &str| {
        let _ = app.update(Message::Pane(crate::message::PaneMsg::Nodes(
            NodesMsg::HealthRead(addr.clone(), ns.to_string(), report(addr)),
        )));
    };
    land(&mut app, &tc, "");
    assert!(
        matches!(app.work.verdicts.nodes.health, HealthState::NotAsked),
        "another address's reading is not this one's"
    );
    land(&mut app, &nav, "elsewhere");
    assert!(
        matches!(app.work.verdicts.nodes.health, HealthState::NotAsked),
        "a reading in another namespace is about another deployment"
    );
    land(&mut app, &nav, "");
    assert!(matches!(
        &app.work.verdicts.nodes.health,
        HealthState::Read(a, _) if a == &nav
    ));
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
    use zenkey_fleet_v1::FetchOutcome;

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
