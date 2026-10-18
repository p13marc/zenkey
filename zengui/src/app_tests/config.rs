//! The Config tool through `update` (#768, CD): a set the owner's own
//! validator refuses starts no task, every act is refused during a replay,
//! silence lands as an answer, and a landing about another deployment is
//! dropped.

use std::time::{Duration, SystemTime};

use super::*;
use crate::configure::{Act, ActLanding, Answer, Draft, ReadLanding, ReadWhy};
use crate::message::PaneMsg;
use crate::view::configure::ConfigMsg;
use zenkey_fleet::report::{ConfigActAnswer, ConfigActReport, PresenceAttribution, SilenceView};
use zenkey_model::config::{ConfigStatus, ConfigView};
use zenkey_model::grammar::Addr;

fn rf0() -> Addr {
    "lab/rf0".parse().expect("an address")
}

fn config(app: &mut Zengui, msg: ConfigMsg) -> iced::Task<Message> {
    app.update(Message::Pane(PaneMsg::Config(msg)))
}

/// `lab/rf0`'s view: `duty` hot, `radio` reach; with `pending`, a change
/// on `radio` pending and applied.
fn view(pending: bool) -> ConfigView {
    let mut v = serde_json::json!({
        "service": "lab/rf0", "instance": "i1", "revision": 2, "confirm_floor_s": 10,
        "groups": [
            {"name": "duty", "class": "hot", "description": "d", "writable": true,
             "parameters": [{"name": "fraction_ppm", "kind": "integer", "min": 0,
                             "max": 1000000, "description": "p", "value": 20000,
                             "source": "runtime"}]},
            {"name": "radio", "class": "reach", "description": "d", "writable": true,
             "parameters": [{"name": "frequency_hz", "kind": "integer",
                             "description": "p", "value": 868100000, "source": "file"}]}
        ]
    });
    if pending {
        v["pending"] = serde_json::json!({"token": "i1-3", "groups": ["radio"],
                                         "deadline": "2026-10-18T09:01:00.000Z",
                                         "applied": true});
    }
    serde_json::from_value(v).expect("a view")
}

/// A reading of `view`, judged by the engine as a GET's answer under the
/// operator's clock word.
fn report(view: &ConfigView) -> std::sync::Arc<zenkey_fleet::report::ConfigReport> {
    use zenkey_fleet::bus::config::ServiceDocs;
    use zenkey_fleet::bus::health::{Replied, Stamped};
    let at = SystemTime::now();
    let stamp = Some(Stamped {
        time: at,
        clock: "ab12".into(),
    });
    let docs = ServiceDocs {
        view: Some(Replied {
            value: Some(Ok(view.clone())),
            stamp: stamp.clone(),
        }),
        status: Some(Replied {
            value: Some(Ok(ConfigStatus::of(view))),
            stamp,
        }),
    };
    let mut spec = zenkey_fleet::ConfigSpec::new(Duration::from_secs(1));
    spec.clocks_synced = true;
    Arc::new(zenkey_fleet::judge_config(
        &zenkey_fleet::ConfigObservation {
            namespace: String::new(),
            address: rf0(),
            spec,
            presence: None,
            get: Ok(zenkey_fleet::ConfigGet {
                selector: zenkey_fleet::ConfigTarget::One(rf0()).state_selector(false),
                read_at: at,
                complete: true,
                services: [(rf0(), docs)].into(),
            }),
            window: None,
            replay: None,
        },
    ))
}

/// A window whose Config tool shows `lab/rf0`, `view` read.
fn reading(view: &ConfigView) -> Zengui {
    let mut app = test_app();
    app.work.verdicts.config.target = Some(rf0());
    let _ = config(
        &mut app,
        ConfigMsg::Read(ReadLanding {
            addr: rf0(),
            namespace: String::new(),
            why: ReadWhy::Asked,
            report: report(view),
            at: SystemTime::now(),
        }),
    );
    assert!(app.work.verdicts.config.view().is_some(), "the view landed");
    app
}

fn refusal(app: &Zengui) -> String {
    match app
        .work
        .verdicts
        .config
        .answered
        .as_ref()
        .map(|a| &a.answer)
    {
        Some(Answer::RefusedHere(why)) => why.clone(),
        other => panic!("a refusal in the form, got {other:?}"),
    }
}

/// The owner's own validator, in the form (§2.5): a hot set while a change
/// is pending and not joined is `busy`, and a reach set without a window is
/// `invalid_request` — each refused with no task at all, so nothing went on
/// the wire, and said as the act's answer.
#[test]
fn a_refused_set_starts_no_task() {
    let mut app = reading(&view(true));
    let _ = config(
        &mut app,
        ConfigMsg::Draft {
            group: "duty".into(),
            param: "fraction_ppm".into(),
            draft: Draft::Text("30000".into()),
        },
    );
    let task = config(&mut app, ConfigMsg::Apply("duty".into()));
    assert_eq!(task.units(), 0, "nothing was started");
    let why = refusal(&app);
    assert!(why.starts_with("busy:"), "{why}");
    assert!(why.contains("i1-3"), "the pending token named: {why}");
    assert!(app.work.verdicts.config.in_flight.is_none());

    let mut app = reading(&view(false));
    let _ = config(
        &mut app,
        ConfigMsg::Draft {
            group: "radio".into(),
            param: "frequency_hz".into(),
            draft: Draft::Text("869525000".into()),
        },
    );
    let task = config(&mut app, ConfigMsg::Apply("radio".into()));
    assert_eq!(task.units(), 0);
    let why = refusal(&app);
    assert!(why.starts_with("invalid_request:"), "{why}");
    assert!(why.contains("confirm_s"), "{why}");

    // With its window and without its consent, the reach change asks first
    // (§2.4): refused in the engine's words, nothing sent.
    let _ = config(
        &mut app,
        ConfigMsg::WindowChanged("radio".into(), "30".into()),
    );
    let task = config(&mut app, ConfigMsg::Apply("radio".into()));
    assert_eq!(task.units(), 0);
    let why = refusal(&app);
    assert!(why.contains("is reach"), "{why}");
    assert!(why.contains("tick its consent"), "{why}");

    // Consent given, it passes every check here — and stops only for want
    // of a session in this headless window.
    let _ = config(&mut app, ConfigMsg::ConsentToggled("radio".into(), true));
    let task = config(&mut app, ConfigMsg::Apply("radio".into()));
    assert_eq!(task.units(), 0);
    assert!(refusal(&app).contains("no session"), "{}", refusal(&app));
}

/// Confirm is refused in the form while the change waits for its apply_at
/// (§2.9): the owner would answer it busy.
#[test]
fn confirm_before_the_apply_is_refused_in_the_form() {
    let mut v = view(true);
    v.pending.as_mut().expect("pending").applied = false;
    let mut app = reading(&v);
    let task = config(&mut app, ConfigMsg::Confirm);
    assert_eq!(task.units(), 0);
    assert!(
        refusal(&app).contains("not applied yet"),
        "{}",
        refusal(&app)
    );
}

/// The replay lock (#74): while the panes show a capture, every act — a
/// set, a dry run, confirm, cancel, extend, persist — is refused, the form
/// says why, and no task starts.
#[test]
fn an_act_is_refused_during_a_replay() {
    let mut app = reading(&view(true));
    let core = zenkey_fleet::MonitorCore::new(64);
    for v in traffic(std::time::Instant::now()) {
        core.ingest(v, None);
    }
    let _ = update::pane::replay::enter_retained(
        &mut app.dep,
        &mut app.obs,
        &mut app.sub,
        &mut app.tree,
        &mut app.work,
        &core,
    );
    assert!(app.work.replay.replay.is_some(), "in replay mode");
    app.work.verdicts.config.extend_by = "30".into();
    for msg in [
        ConfigMsg::Apply("duty".into()),
        ConfigMsg::DryRun("duty".into()),
        ConfigMsg::Confirm,
        ConfigMsg::Cancel,
        ConfigMsg::Extend,
        ConfigMsg::Persist,
    ] {
        let task = config(&mut app, msg.clone());
        assert_eq!(task.units(), 0, "{msg:?}: nothing was started in replay");
        assert_eq!(
            refusal(&app),
            crate::view::send::REPLAY_LOCK,
            "{msg:?}: the lock, in its words"
        );
    }
}

fn act(answer: ConfigActAnswer) -> ConfigActReport {
    ConfigActReport {
        namespace: String::new(),
        service: "lab/rf0".into(),
        operation: "set/duty".into(),
        key: "zk2/lab/rf0/config.v1/@op/set/duty".into(),
        request: serde_json::json!({}),
        actor: None,
        request_id: None,
        checked: None,
        across_face: false,
        timeout_s: 2.0,
        answer,
    }
}

/// Silence is an answer, not a failure (O5): it lands as the act's answer,
/// the act is no longer in flight, and the form keeps its idempotency key —
/// a resend is a retry the owner answers once (§2.14). A value drops it.
#[test]
fn silence_lands_as_an_answer() {
    let mut app = reading(&view(false));
    let set = Act::Set {
        group: "duty".into(),
        dry_run: false,
    };
    let now = SystemTime::now();
    let cfg = &mut app.work.verdicts.config;
    let req = zenkey_model::config::SetRequest::default();
    let _ = cfg
        .forms
        .entry("duty".into())
        .or_default()
        .key_for(&req, now);
    cfg.in_flight = Some(crate::configure::Sent {
        act: set.clone(),
        at: now,
        window_s: None,
    });
    let _ = config(
        &mut app,
        ConfigMsg::Answered(ActLanding {
            addr: rf0(),
            namespace: String::new(),
            act: set.clone(),
            at: now,
            result: Ok(Arc::new(act(ConfigActAnswer::Silent {
                silence: SilenceView {
                    attempts: 1,
                    transport: None,
                    presence: PresenceAttribution::NoTokenVisible,
                },
            }))),
        }),
    );
    let cfg = &app.work.verdicts.config;
    assert!(cfg.in_flight.is_none());
    match cfg.answered.as_ref().map(|a| &a.answer) {
        Some(Answer::Landed(r)) => {
            assert!(matches!(r.answer, ConfigActAnswer::Silent { .. }));
        }
        other => panic!("silence lands as an answer, got {other:?}"),
    }
    assert!(cfg.forms["duty"].holds_key(), "a retry keeps the key");

    let _ = config(
        &mut app,
        ConfigMsg::Answered(ActLanding {
            addr: rf0(),
            namespace: String::new(),
            act: set,
            at: now,
            result: Ok(Arc::new(act(ConfigActAnswer::Applied {
                view: Box::new(view(false)),
            }))),
        }),
    );
    assert!(
        !app.work
            .verdicts
            .config
            .forms
            .get("duty")
            .is_some_and(|f| f.holds_key()),
        "an answer drops the key, and an applied change its drafts"
    );
}

/// A landing about another deployment — a namespace this window has left —
/// or another address describes nothing on screen, and is dropped (#109).
#[test]
fn a_landing_about_another_deployment_is_dropped() {
    let mut app = reading(&view(false));
    let before = app.work.verdicts.config.shown().map(|r| r.at);
    let _ = config(
        &mut app,
        ConfigMsg::Read(ReadLanding {
            addr: rf0(),
            namespace: "elsewhere".into(),
            why: ReadWhy::Asked,
            report: report(&view(true)),
            at: SystemTime::now() + Duration::from_secs(5),
        }),
    );
    assert_eq!(app.work.verdicts.config.shown().map(|r| r.at), before);
    assert!(app.work.verdicts.config.pending().is_none());
    let _ = config(
        &mut app,
        ConfigMsg::Answered(ActLanding {
            addr: "lab/rf1".parse().expect("an address"),
            namespace: String::new(),
            act: Act::Confirm,
            at: SystemTime::now(),
            result: Ok(Arc::new(act(ConfigActAnswer::View {
                view: Box::new(view(false)),
            }))),
        }),
    );
    assert!(app.work.verdicts.config.answered.is_none());
}

/// Selecting an address while the tool is the Workbench's points the tool
/// at it; a forget (a namespace switch) drops it all but the clock word.
#[test]
fn the_tool_follows_the_selection_and_a_forget_drops_it() {
    use crate::message::{RightPane, WorkspaceMsg};
    let mut app = test_app();
    let _ = app.update(Message::Workspace(WorkspaceMsg::PaneSelected(
        RightPane::Config,
    )));
    let _ = app.update(Message::Subject(SubjectMsg::Select(
        Subject::Address(rf0()),
    )));
    assert_eq!(app.work.verdicts.config.target, Some(rf0()));
    app.work.verdicts.config.clocks_synced = true;
    app.work.verdicts.forget();
    assert!(app.work.verdicts.config.target.is_none());
    assert!(
        app.work.verdicts.config.clocks_synced,
        "the operator's word"
    );
}
