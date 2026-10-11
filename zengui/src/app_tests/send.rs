//! The Send pane through `update` (#614, FL6a; zk2's since FL10): P3 holds,
//! a forbidden fan-out is refused before anything is sent, and an answer
//! that is silence lands as an answer.

use super::*;
use crate::message::PaneMsg;
use crate::view::send::{RevisionChoice, SendMode, SendMsg};

fn send(app: &mut Zengui, msg: SendMsg) -> iced::Task<Message> {
    app.update(Message::Pane(PaneMsg::Send(msg)))
}

/// A window holding the walkthrough and tcgui contracts, as `--contracts`
/// would load them — off the update thread in the window; here, the landing.
fn with_contracts() -> Zengui {
    let mut app = test_app();
    let examples = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2");
    let loaded = crate::services::contracts::load_all(&[
        examples.join("walkthrough"),
        examples.join("tcgui"),
    ]);
    let _ = app.update(Message::Bus(crate::message::BusMsg::ContractsLoaded(
        Arc::new(loaded),
    )));
    app
}

/// The interface choice the call mode offers for `iface`.
fn choice(app: &Zengui, iface: &str) -> RevisionChoice {
    app.work
        .bench
        .send_form
        .revisions
        .iter()
        .find(|c| c.0.iface().to_string() == iface)
        .unwrap_or_else(|| panic!("{iface} is offered"))
        .clone()
}

/// A publish or a tombstone on a key a zk2 service owns is refused in the
/// form — said in its log and as its error — and nothing is put in flight,
/// session or none: a refusal is not a send that failed.
#[test]
fn the_send_pane_refuses_a_key_a_zk2_service_owns() {
    for msg in [SendMsg::Send, SendMsg::Retire] {
        let mut app = test_app();
        app.work.bench.send_form.key = "spray/zk2/robot1/nav/nav.v2/state/status".into();
        app.work.bench.send_form.retire_i_know = true;
        let task = send(&mut app, msg.clone());
        assert_eq!(task.units(), 0, "{msg:?}: nothing was started");
        let form = &app.work.bench.send_form;
        let error = form.error.as_ref().expect("a refusal").to_string();
        assert!(error.contains("robot1/nav owns"), "{msg:?}: {error}");
        assert!(error.contains("P3"), "{msg:?}: {error}");
        assert_eq!(form.log[0].text, error, "{msg:?}: the log says it too");
        assert!(!form.in_flight && !form.armed, "{msg:?}: nothing moved");
    }
}

/// The call mode offers the revisions in hand that declare an operation —
/// the contracts loaded and whatever the bus served — and picking one lists
/// its operations; one operation alone is picked for the user.
#[test]
fn the_call_mode_offers_the_revisions_in_hand() {
    let mut app = with_contracts();
    let _ = send(&mut app, SendMsg::ModeSelected(SendMode::Call));
    let offered: Vec<String> = app
        .work
        .bench
        .send_form
        .revisions
        .iter()
        .map(|c| c.0.iface().to_string())
        .collect();
    for iface in ["nav.v2", "tc.netif.v1", "thruster.v1"] {
        assert!(
            offered.contains(&iface.to_string()),
            "{iface} in {offered:?}"
        );
    }
    assert!(
        !offered.contains(&"camera.v1".to_string()),
        "a revision with no operation has nothing to call"
    );
    let nav = choice(&app, "nav.v2");
    let _ = send(&mut app, SendMsg::RevisionPicked(nav));
    let form = &app.work.bench.send_form;
    let mut operations = form.operations.clone();
    operations.sort();
    assert_eq!(operations, ["@op/reset", "@op/set_origin"]);
    assert!(form.operation.is_none(), "two operations: the user picks");
    assert!(
        !form.presence_read,
        "no presence read stands behind the providers"
    );
}

/// O2 before anything is sent: a fan-out to an operation that forbids one —
/// a `*` in the address, or a parameter not given — is refused by
/// `plan_call` in the form, with the engine's words; no task starts, so
/// nothing reaches the bus, and the refusal is a failure line in the log.
#[test]
fn a_forbidden_fan_out_is_refused_before_anything_is_sent() {
    for (target, params, why) in [
        ("*/tc", "ns=default;iface=eth0", "has a `*`"),
        ("host-a/tc", "", "--param ns=…"),
    ] {
        let mut app = with_contracts();
        let _ = send(&mut app, SendMsg::ModeSelected(SendMode::Call));
        let tc = choice(&app, "tc.netif.v1");
        let _ = send(&mut app, SendMsg::RevisionPicked(tc));
        let _ = send(
            &mut app,
            SendMsg::OperationPicked("@op/interfaces/{ns}/{iface}/set".into()),
        );
        let _ = send(&mut app, SendMsg::TargetChanged(target.into()));
        let _ = send(&mut app, SendMsg::ParamsChanged(params.into()));
        let _ = send(
            &mut app,
            SendMsg::BodyChanged(r#"{"operation":"Enable"}"#.into()),
        );
        let task = send(&mut app, SendMsg::Submit);
        assert_eq!(task.units(), 0, "{target}: no call was started");
        let form = &app.work.bench.send_form;
        assert!(!form.in_flight, "{target}: nothing in flight");
        let Some(Err(e)) = &form.outcome else {
            panic!("{target}: a refusal, got {:?}", form.outcome);
        };
        let e = e.to_string();
        assert!(e.contains("fanout = \"forbidden\""), "{target}: {e}");
        assert!(e.contains(why), "{target}: {e}");
        assert!(e.contains("nothing was sent"), "{target}: {e}");
        assert!(!form.log[0].ok, "a refusal is a line that did not happen");
    }

    // The same call to one address with every parameter given plans and
    // encodes — and stops only for want of a session in this headless app.
    let mut app = with_contracts();
    let _ = send(&mut app, SendMsg::ModeSelected(SendMode::Call));
    let tc = choice(&app, "tc.netif.v1");
    let _ = send(&mut app, SendMsg::RevisionPicked(tc));
    let _ = send(
        &mut app,
        SendMsg::OperationPicked("@op/interfaces/{ns}/{iface}/set".into()),
    );
    let _ = send(&mut app, SendMsg::TargetChanged("host-a/tc".into()));
    let _ = send(
        &mut app,
        SendMsg::ParamsChanged("ns=default;iface=eth0".into()),
    );
    let _ = send(&mut app, SendMsg::ScaffoldBody);
    let _ = send(&mut app, SendMsg::Submit);
    let Some(Err(e)) = &app.work.bench.send_form.outcome else {
        panic!("no session, no call");
    };
    assert!(
        e.to_string()
            .contains("no session in the deployment's namespace"),
        "{e}"
    );
}

/// Silence is an answer, not a failure (O5): it lands in the outcome and as
/// a line that happened, saying it is a non-verdict — and an envelope lands
/// the same way, as the refusal it is.
#[test]
fn a_silent_call_lands_as_an_answer_not_a_failure() {
    use zenkey_fleet::report::{
        CallMode, EnvelopeView, OperationAnswer, OperationReport, PresenceAttribution, SilenceView,
    };
    let report = |answer| OperationReport {
        address: "robot9/nav".into(),
        iface: "nav.v2".into(),
        fingerprint: format!("sha256:{}", "ab".repeat(32)),
        operation: "@op/set_origin".into(),
        values: Default::default(),
        selectors: vec!["zk2/robot9/nav/nav.v2/@op/set_origin".into()],
        mode: CallMode::Concrete,
        timeout_s: 2.0,
        answer,
    };
    let mut app = test_app();
    app.work.bench.send_form.in_flight = true;
    let _ = send(
        &mut app,
        SendMsg::Done(Ok(Arc::new(report(OperationAnswer::Silent {
            silence: SilenceView {
                attempts: 1,
                transport: Some("zenoh/string: Timeout".into()),
                presence: PresenceAttribution::NoTokenVisible,
            },
        })))),
    );
    let form = &app.work.bench.send_form;
    assert!(!form.in_flight);
    assert!(
        matches!(&form.outcome, Some(Ok(_))),
        "an answer, not an error"
    );
    assert!(form.log[0].ok, "silence is not a failure line");
    assert_eq!(
        form.log[0].text,
        "called nav.v2 @op/set_origin at robot9/nav → silent within 2s — a non-verdict"
    );

    let _ = send(
        &mut app,
        SendMsg::Done(Ok(Arc::new(report(OperationAnswer::Refused {
            envelope: EnvelopeView {
                code: "unavailable".into(),
                message: "no".into(),
                cause: Some("capability".into()),
                detail: None,
            },
        })))),
    );
    let form = &app.work.bench.send_form;
    assert!(form.log[0].ok);
    assert_eq!(
        form.log[0].text,
        "called nav.v2 @op/set_origin at robot9/nav → refused: unavailable (capability)"
    );
}
