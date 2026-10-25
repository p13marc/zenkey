//! The subject: revealing, pointing, symbolic keys, a stale fetch, and the
//! read a selection makes — by resolution, never under `@op`, and never
//! during a replay (#614, FL8a).

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

/// `redundancy.v1`'s reading (#841, RC) lands under the health reading's
/// guard: only the selected address's, only in this namespace — and a
/// window with no session in the namespace asks nothing, and says so.
#[test]
fn a_redundancy_reading_lands_only_on_the_selected_address_in_this_namespace() {
    use crate::nodes::RedundancyState;
    use crate::view::nodes::NodesMsg;
    use zenkey_model::grammar::Addr;

    let report = |addr: &Addr| {
        std::sync::Arc::new(zenkey_fleet::judge_redundancy(
            "",
            Some(addr),
            &zenkey_fleet::RedundancySpec::new(std::time::Duration::from_secs(1)),
            &zenkey_fleet::RedundancyObservation {
                cut_off: Some("no session".into()),
                ..Default::default()
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
        matches!(
            app.work.verdicts.nodes.redundancy,
            RedundancyState::NotAsked
        ),
        "no session in the namespace: nothing was asked"
    );
    let land = |app: &mut crate::app::Zengui, addr: &Addr, ns: &str| {
        let _ = app.update(Message::Pane(crate::message::PaneMsg::Nodes(
            NodesMsg::RedundancyRead(addr.clone(), ns.to_string(), report(addr)),
        )));
    };
    land(&mut app, &tc, "");
    assert!(matches!(
        app.work.verdicts.nodes.redundancy,
        RedundancyState::NotAsked
    ));
    land(&mut app, &nav, "elsewhere");
    assert!(matches!(
        app.work.verdicts.nodes.redundancy,
        RedundancyState::NotAsked
    ));
    land(&mut app, &nav, "");
    let r = app
        .work
        .verdicts
        .nodes
        .redundancy
        .report(&nav)
        .expect("landed");
    assert!(r.unobservable.is_some(), "{r:?}");
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
    use crate::value::{ReadKind, Replies, ValueRead};

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

    let late = Arc::new(ValueRead::Replies(Replies {
        kind: ReadKind::Raw,
        selector: stale.to_string(),
        replies: Vec::new(),
        errors: 0,
        elided: 0,
        retention: None,
    }));
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

/// A window with both sessions open — peers listening on an ephemeral
/// loopback port, nothing else on the bus — and the walkthrough's contracts
/// held, so a `nav.v2` key resolves offline and a read has a session to go
/// out on (#614, FL8a). Its reads wait `timeout_s`.
async fn with_sessions(timeout_s: u64) -> Zengui {
    use crate::message::BusMsg;
    let raw = zenkey_fleet::open_reporting(None, &[], &["tcp/127.0.0.1:0".into()], Some(false))
        .await
        .expect("a raw session opens");
    let session =
        zenkey_fleet::open_in_namespace("", None, &[], &["tcp/127.0.0.1:0".into()], Some(false))
            .await
            .expect("a session in the namespace opens");
    let mut settings = crate::app::test_settings();
    settings.timeout_secs = timeout_s;
    let mut app = Zengui::with_prefs(settings, crate::prefs::Prefs::default(), None).0;
    let _ = app.update(Message::Bus(BusMsg::SessionsOpened(Ok(
        crate::services::link::Sessions {
            raw,
            session,
            namespace: String::new(),
        },
    ))));
    let walkthrough =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2/walkthrough");
    let _ = app.update(Message::Bus(BusMsg::ContractsLoaded(Arc::new(
        crate::services::contracts::load_all(&[walkthrough]),
    ))));
    app
}

/// #614 (FL8a): a state key's read is S4's owner GET — asked on the session
/// in the namespace, at the key the identity names — and an owner nobody
/// runs answers with silence: a read with no row, never "no value", and the
/// question no longer pending.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_state_keys_read_is_the_owner_get_and_an_absent_owner_is_silent() {
    use crate::value::{ReadKind, ValueRead};
    let mut app = with_sessions(1).await;
    let key = "zk2/robot1/nav/nav.v2/state/status";
    let read = app.update(Message::Subject(SubjectMsg::Select(Subject::Key(
        key.to_string(),
    ))));
    assert_eq!(
        app.sub.follow.asked,
        Some((key.to_string(), ReadKind::Owner)),
        "the question is the owner's GET"
    );
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        super::drive(&mut app, read),
    )
    .await
    .expect("the GET ends at its timeout");
    assert!(app.sub.follow.asked.is_none(), "answered, so not pending");
    let Some((k, Ok(read))) = app.sub.follow.fetched.as_ref() else {
        panic!("the read landed")
    };
    assert_eq!(k, key);
    let ValueRead::Owner(answer) = read.as_ref() else {
        panic!("a state key is read by its owner's GET")
    };
    assert_eq!(
        answer.report.selectors,
        [key],
        "addressed to the key itself"
    );
    assert!(answer.row().is_none(), "nobody answered: silence");
    assert!(
        crate::view::detail::read_summary(read).starts_with("silent — owner GET (S4)"),
        "and the strip says so"
    );
}

/// #614 (FL6b, FL8a): nothing is read under `@op` — a GET there is a call
/// (spec §5) — nor for a stream that declares no history, which keeps no
/// value to read; with both sessions open, neither selection asks a thing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_read_under_op_or_for_a_stream_without_history() {
    let mut app = with_sessions(1).await;
    for key in [
        "zk2/robot1/nav/nav.v2/@op/set_origin",
        "zk2/robot1/nav/nav.v2/stream/position",
    ] {
        let _ = app.update(Message::Subject(SubjectMsg::Select(Subject::Key(
            key.to_string(),
        ))));
        assert!(app.sub.follow.asked.is_none(), "{key}: nothing asked");
        assert!(app.sub.follow.fetched.is_none(), "{key}: nothing landed");
    }
    assert_eq!(
        crate::value::plan("zk2/robot1/nav/nav.v2/@op/set_origin", None),
        crate::value::ReadPlan::NotRead(crate::value::NotRead::Operation),
        "refused before the identity is even consulted"
    );
}

/// #614 (FL8a; FL10 found the opposite): a selection during a replay asks
/// the bus nothing, even with both sessions open — its value is the
/// capture's latest row for the key at the playhead, checked through the
/// replay's own lens and landing under the replay's id. A live read that
/// lands meanwhile is about another world and is dropped; leaving the
/// replay reads the key live again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_selection_during_a_replay_asks_the_bus_nothing() {
    use crate::replay::{LoadedReplay, ReplayState};
    use crate::value::{ReadKind, Replies, ValueRead};
    use crate::view::replay::ReplayMsg;

    let mut app = with_sessions(1).await;
    let key = "demo/example/foo";
    let file = [
        r#"{"zrec":3,"selectors":["**"],"base":"","captured_at":"2026-10-05T03:00:00Z","excluded":["@stream","@state","@op","@zk","@adv"]}"#,
        r#"{"key":"demo/example/foo","t":0,"bytes":"aGVsbG8gIzA=","encoding":"text/plain","qos_axes":"data/drop/best_effort"}"#,
        r#"{"key":"demo/example/foo","t":1000000,"bytes":"aGVsbG8gIzE=","encoding":"text/plain","qos_axes":"data/drop/best_effort"}"#,
    ]
    .join("\n");
    let state = ReplayState::load("capture.zrec", file.as_bytes()).expect("a capture");
    let id = state.id();
    let _ = app.update(Message::Workspace(crate::message::WorkspaceMsg::Replay(
        ReplayMsg::Loaded("capture.zrec".into(), Ok(LoadedReplay::new(state))),
    )));

    let check = app.update(Message::Subject(SubjectMsg::Select(Subject::Key(
        key.to_string(),
    ))));
    assert!(
        app.sub.follow.asked.is_none(),
        "no read went out, though both sessions are open"
    );
    let recorded_at = |app: &Zengui| match app.sub.follow.fetched.as_ref() {
        Some((_, Ok(read))) => match read.as_ref() {
            ValueRead::Recorded { t_us, reply, .. } => {
                Some((*t_us, reply.payload.to_bytes().to_vec()))
            }
            _ => None,
        },
        _ => None,
    };
    assert_eq!(
        recorded_at(&app),
        Some((0, b"hello #0".to_vec())),
        "the capture's row at the playhead"
    );
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        super::drive(&mut app, check),
    )
    .await
    .expect("the offline check settles");
    assert!(
        app.sub.follow.decoded.is_some(),
        "checked through the replay's lens, landing under its id ({id})"
    );

    // The playhead moves: the value is the capture's row there.
    let _ = app.update(Message::Workspace(crate::message::WorkspaceMsg::Replay(
        ReplayMsg::Scrubbed(1_000_000),
    )));
    assert_eq!(recorded_at(&app), Some((1_000_000, b"hello #1".to_vec())));

    // A live answer landing now is about the bus, and dropped.
    let live = Arc::new(ValueRead::Replies(Replies {
        kind: ReadKind::Raw,
        selector: key.to_string(),
        replies: Vec::new(),
        errors: 0,
        elided: 0,
        retention: None,
    }));
    let _ = app.update(Message::Subject(SubjectMsg::ValueFetched(
        key.to_string(),
        Ok(live),
    )));
    assert!(recorded_at(&app).is_some(), "the capture's value stays");
    // Nor does an archive get asked: a replay asks the bus nothing.
    let _ = app.update(Message::Pane(crate::message::PaneMsg::Detail(
        crate::message::SlotId::FOLLOW,
        crate::view::detail::DetailMsg::LastKnown("ground/archive".parse().expect("an address")),
    )));
    assert!(app.sub.follow.last_known.is_none());

    // Leaving reads the key live again: a raw GET for a foreign key.
    let _ = app.update(Message::Workspace(crate::message::WorkspaceMsg::Replay(
        ReplayMsg::Exit,
    )));
    assert!(
        recorded_at(&app).is_none(),
        "the file's value stays with the file"
    );
    assert_eq!(
        app.sub.follow.asked,
        Some((key.to_string(), ReadKind::Raw)),
        "and the live read is a raw GET, labelled raw"
    );

    // A key the capture never carried: silence during the replay, a live
    // read once it ends.
    let state = ReplayState::load("capture.zrec", file.as_bytes()).expect("a capture");
    let _ = app.update(Message::Workspace(crate::message::WorkspaceMsg::Replay(
        ReplayMsg::Loaded("capture.zrec".into(), Ok(LoadedReplay::new(state))),
    )));
    let quiet = "demo/example/bar";
    let _ = app.update(Message::Subject(SubjectMsg::Select(Subject::Key(
        quiet.to_string(),
    ))));
    assert!(app.sub.follow.fetched.is_none() && app.sub.follow.asked.is_none());
    let _ = app.update(Message::Workspace(crate::message::WorkspaceMsg::Replay(
        ReplayMsg::Exit,
    )));
    assert_eq!(
        app.sub.follow.asked,
        Some((quiet.to_string(), ReadKind::Raw)),
        "the capture's silence stays with the file too"
    );
}

/// #614 (FL8a; S5, S6): last-known is asked by the button, of the archive
/// named, and lands beside the owner's read — never in its place; an
/// archive nobody runs holds nothing, said as such. (During a replay the
/// button asks nothing: the replay test pins it.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_last_known_read_is_asked_by_button_and_never_during_a_replay() {
    use crate::message::PaneMsg;
    use crate::view::detail::DetailMsg;
    let mut app = with_sessions(1).await;
    let key = "zk2/robot1/nav/nav.v2/state/status";
    let read = app.update(Message::Subject(SubjectMsg::Select(Subject::Key(
        key.to_string(),
    ))));
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        super::drive(&mut app, read),
    )
    .await
    .expect("the owner GET settles");
    let archive: zenkey_model::grammar::Addr = "ground/archive".parse().expect("an address");
    let ask = app.update(Message::Pane(PaneMsg::Detail(
        crate::message::SlotId::FOLLOW,
        DetailMsg::LastKnown(archive.clone()),
    )));
    let asked = app.sub.follow.last_known.as_ref().expect("asked");
    assert_eq!((asked.key.as_str(), &asked.archive), (key, &archive));
    assert!(asked.answer.is_none(), "in flight");
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        super::drive(&mut app, ask),
    )
    .await
    .expect("the archive's GET settles");
    let Some(Ok(answer)) = app
        .sub
        .follow
        .last_known
        .as_ref()
        .and_then(|l| l.answer.as_ref())
    else {
        panic!("the archive's answer landed")
    };
    assert_eq!(
        answer.report.reading,
        zenkey_fleet::report::StateReading::LastKnown
    );
    assert_eq!(answer.report.archive.as_deref(), Some("ground/archive"));
    assert!(answer.row().is_none(), "no archive runs: it holds nothing");
    assert!(
        matches!(
            app.sub.follow.fetched.as_ref(),
            Some((_, Ok(r))) if matches!(r.as_ref(), crate::value::ValueRead::Owner(_))
        ),
        "the owner's read stays the value"
    );

    // A new subject forgets the archive's answer about the old one.
    let _ = app.update(Message::Subject(SubjectMsg::Select(Subject::Key(
        "zk2/robot1/nav/nav.v2/state/origin".to_string(),
    ))));
    assert!(app.sub.follow.last_known.is_none());
}
