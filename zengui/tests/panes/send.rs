//! The send pane: publish mode, call mode, and both over one form — zk2's
//! since #614 (FL10).

use super::*;

use std::sync::Arc;

use zengui::view::send::{RevisionChoice, SendForm, SendMode, pane, request_form};
use zenkey_fleet::report::{
    CallMode, EnvelopeView, OperationAnswer, OperationReport, PayloadRendering,
    PresenceAttribution, Rendered, SilenceView,
};

/// One revision of the window's world (`common::contracts`), by interface.
fn revision(iface: &str) -> Arc<zenkey_fleet::Revision> {
    let iface: zenkey_model::grammar::IfaceId = iface.parse().expect("an interface");
    common::contracts()
        .set
        .of_iface(&iface)
        .next()
        .expect("the contract is held")
        .clone()
}

/// The call form with `iface`'s `operation` picked, as the pane holds it
/// after the pickers spoke.
fn picked(iface: &str, operation: &str) -> SendForm {
    let rev = revision(iface);
    let choice = RevisionChoice(Arc::clone(&rev));
    SendForm {
        mode: SendMode::Call,
        revisions: vec![choice.clone()],
        operations: choice.operations(),
        revision: Some(choice),
        operation: Some(operation.into()),
        shape: Some(zengui::view::send::CallShape::of(&rev, operation).expect("declared")),
        request: Some(request_form(&rev, operation)),
        ..SendForm::default()
    }
}

/// A report of `nav.v2`'s `set_origin` at `robot1/nav`, answered `answer`.
fn report(answer: OperationAnswer) -> OperationReport {
    OperationReport {
        address: "robot1/nav".into(),
        iface: "nav.v2".into(),
        fingerprint: format!("sha256:{}", "ab".repeat(32)),
        operation: "@op/set_origin".into(),
        values: Default::default(),
        selectors: vec!["zk2/robot1/nav/nav.v2/@op/set_origin".into()],
        mode: CallMode::Concrete,
        timeout_s: 2.0,
        answer,
    }
}

/// P3, said before the send (#614, FL6a; FL10): a key a zk2 service owns is
/// refused on screen as it is typed, naming the owner and the rule — and a
/// wildcard is refused as the blast radius it is (#504). A foreign key
/// meets neither.
#[test]
fn the_publish_mode_names_the_p3_refusal_before_sending() {
    let owned = SendForm {
        key: "spray/zk2/robot1/nav/nav.v2/state/status".into(),
        ..SendForm::default()
    };
    let mut ui = simulator::<Message, _, _>(pane(&owned, false, sp()));
    assert!(
        ui.find(
            "refused: spray/zk2/robot1/nav/nav.v2/state/status is a key robot1/nav owns, and \
             only the owner writes its keys (P3, spec §6) — act on the service through its \
             operations"
        )
        .is_ok(),
        "the refusal is on screen before anything is sent"
    );

    let wildcard = SendForm {
        key: "demo/**".into(),
        ..SendForm::default()
    };
    let mut ui = simulator::<Message, _, _>(pane(&wildcard, false, sp()));
    let blast = zenkey_fleet::check_concrete("demo/**", zenkey_fleet::WriteAct::Put)
        .unwrap_err()
        .to_string();
    assert!(ui.find(blast.as_str()).is_ok(), "{blast}");

    let foreign = SendForm {
        key: "demo/example/foo".into(),
        ..SendForm::default()
    };
    let mut ui = simulator::<Message, _, _>(pane(&foreign, false, sp()));
    assert!(ui.find("P3, spec §6").is_err(), "a foreign key is no one's");
}

/// What a publish put on the wire, and the axes it will go out on: zenoh's
/// default said as such until the operator picks, the bytes as typed, the
/// encoding set or none — `zenctl pub`'s sentences.
#[test]
fn the_publish_mode_says_what_went_on_the_wire() {
    let mut form = SendForm {
        key: "demo/example/foo".into(),
        body: "hello".into(),
        ..SendForm::default()
    };
    {
        let mut ui = simulator::<Message, _, _>(pane(&form, false, sp()));
        assert!(
            ui.find("qos: data/drop/reliable (zenoh's default — no contract types a foreign key)")
                .is_ok()
        );
        assert!(
            ui.find(
                "bytes as typed — a foreign key has no contract, so nothing encodes or validates it"
            )
            .is_ok()
        );
    }
    form.qos.priority = zenoh::qos::Priority::RealTime;
    form.qos.express = true;
    form.qos_touched = true;
    form.sent = Some(zengui::view::send::Sent {
        bytes: 5,
        encoding: None,
        qos: form.qos.token(),
    });
    let mut ui = simulator::<Message, _, _>(pane(&form, false, sp()));
    assert!(ui.find("qos: real_time/drop/reliable+express").is_ok());
    assert!(
        ui.find("sent 5 bytes as typed → (no encoding set) · qos real_time/drop/reliable+express")
            .is_ok()
    );
}

/// The publish mode's honesty surfaces: the O4 matching badge and the O6
/// bounded log.
#[test]
fn the_publish_mode_bounds_its_log_and_never_invents_a_matcher() {
    use zengui::view::send::LOG_LINES;

    let mut form = SendForm {
        key: "demo/foreign/key".into(),
        armed: true,
        // Armed, but the status could not be asked: "not asked" (O4).
        matching: None,
        ..SendForm::default()
    };
    {
        let mut ui = simulator::<Message, _, _>(pane(&form, false, sp()));
        assert!(ui.find("matching: not asked").is_ok());
    }

    form.matching = Some(false);
    {
        let mut ui = simulator::<Message, _, _>(pane(&form, false, sp()));
        assert!(
            ui.find(
                "matching: no subscriber currently matches this publication — a routing \
                 fact about this publisher, not a fleet verdict (RFC 05 §3.1)"
            )
            .is_ok(),
            "a false matching badge must disclaim, exactly as the CLI does"
        );
    }

    // O6: the log is bounded, and it reports what the bound cost.
    for i in 0..(LOG_LINES + 3) {
        form.log(true, format!("published demo/foreign/key ({i} bytes)"));
    }
    let mut ui = simulator::<Message, _, _>(pane(&form, false, sp()));
    assert!(
        ui.find(format!(
            "send log — {LOG_LINES} shown, 3 dropped (bounded at {LOG_LINES})"
        ))
        .is_ok(),
        "a trimmed log must say how much it trimmed"
    );
}

/// #184's acceptance: one pane, one form, both modes. The mode strip is on
/// screen whichever mode is showing, and switching is a message the strip
/// itself emits — not a second pane wearing a toggle.
#[test]
fn the_send_pane_offers_both_modes_over_one_form() {
    use zengui::message::PaneMsg;
    use zengui::view::send::SendMsg;

    let mut form = SendForm::default();
    {
        let mut ui = simulator::<Message, _, _>(pane(&form, false, sp()));
        assert!(ui.find("put / publish").is_ok());
        assert!(
            ui.find("get / call").is_ok(),
            "the other mode is one click away, not another pane"
        );
        ui.click("get / call").expect("the mode tab is a button");
        let messages: Vec<String> = ui.into_messages().map(|m| format!("{m:?}")).collect();
        let want = format!(
            "{:?}",
            Message::Pane(PaneMsg::Send(SendMsg::ModeSelected(SendMode::Call)))
        );
        assert!(
            messages.contains(&want),
            "clicking the tab switches the mode, got: {messages:?}"
        );
    }
    form.mode = SendMode::Call;
    let mut ui = simulator::<Message, _, _>(pane(&form, false, sp()));
    assert!(ui.find("put / publish").is_ok(), "…and the way back too");
    // No revision in hand is "not asked", never an empty picker.
    assert!(ui.find("No operation in hand").is_ok());
}

/// The replay lock, on screen (#74; #614, FL10): while the panes show a
/// capture the form says nothing is written, and neither send nor call is
/// pressable.
#[test]
fn the_send_pane_says_replay_writes_nothing() {
    use zengui::view::send::REPLAY_LOCK;
    let form = SendForm {
        key: "demo/example/foo".into(),
        ..SendForm::default()
    };
    let mut ui = simulator::<Message, _, _>(pane(&form, true, sp()));
    assert!(ui.find(REPLAY_LOCK).is_ok());
    let _ = ui.click("send");
    assert!(
        ui.into_messages().next().is_none(),
        "the send button is not pressable in replay"
    );
}

/// The call mode reads an operation's declared shape from the revision in
/// hand, and labels a forbidden fan-out before any call (O2's visual half;
/// `plan_call` refuses it at the engine).
#[test]
fn the_call_mode_labels_forbidden_fanout() {
    let form = SendForm {
        target: "*/tc".into(),
        ..picked("tc.netif.v1", "@op/interfaces/{ns}/{iface}/set")
    };
    let mut ui = simulator::<Message, _, _>(pane(&form, false, sp()));
    assert!(
        ui.find(
            "fanout = \"forbidden\" — a `*` in the address, or a parameter not given, is \
             refused before anything is sent (spec §5.1 O2)"
        )
        .is_ok(),
        "the refusal must be visible before any call"
    );
    assert!(
        ui.find("→ zk2/{system}/{service}/tc.netif.v1/@op/interfaces/{ns}/{iface}/set")
            .is_ok(),
        "the key a call goes out on is on screen"
    );
    for (label, value) in [
        ("FANOUT", "forbidden"),
        ("REQUEST", "json:InterfaceControlRequest"),
        ("ERROR", "json:TcError"),
        ("REPLIES", "one"),
    ] {
        assert!(ui.find(label).is_ok(), "{label}");
        assert!(ui.find(value).is_ok(), "{label} {value}");
    }
    assert!(
        ui.find("name=value;… — ns, iface: each one not given is a wildcard")
            .is_ok(),
        "the template's parameters are named"
    );
}

/// The request form from a JSON Schema type (§7.2): the `$defs` entry's
/// properties, required starred, an enumeration scaffolded by its first
/// value — and the scaffold is a request the engine encodes.
#[test]
fn the_request_form_reads_a_json_schema_type() {
    let rev = revision("tc.netif.v1");
    let zengui::view::send::RequestForm::Fields { declared, fields } =
        request_form(&rev, "@op/diagnostics")
    else {
        panic!("a JSON Schema request has fields");
    };
    assert_eq!(declared, "json:DiagnosticsRequest");
    let labels: Vec<String> = fields.iter().map(|f| f.label()).collect();
    for want in [
        "namespace: string*",
        "interface: string*",
        "target: string",
        "timeout_ms: integer",
    ] {
        assert!(labels.contains(&want.to_string()), "{want} in {labels:?}");
    }

    let form = picked("tc.netif.v1", "@op/interfaces/{ns}/{iface}/set");
    let body: serde_json::Value =
        serde_json::from_str(&form.scaffold().expect("a scaffold")).expect("JSON");
    assert_eq!(body, serde_json::json!({"operation": "Enable"}));
    let plan = zenkey_fleet::plan_call(
        &rev,
        zenkey_fleet::ResolvedTarget::parse("host-a/tc").unwrap(),
        "@op/interfaces/{ns}/{iface}/set",
        [
            ("ns".to_string(), vec!["default".to_string()]),
            ("iface".to_string(), vec!["eth0".to_string()]),
        ]
        .into(),
    )
    .expect("one address, every parameter");
    zenkey_fleet::encode_request(&rev, &plan, form.scaffold().unwrap().as_bytes())
        .expect("the scaffold satisfies its own schema");

    let mut ui = simulator::<Message, _, _>(pane(&form, false, sp()));
    assert!(
        ui.find("json:InterfaceControlRequest fields: operation: enum*")
            .is_ok()
    );
    assert!(ui.find("scaffold body").is_ok());
}

/// The request form from a protobuf message (§7.2): its fields from the
/// bundle's descriptor set, a nested message scaffolded with its own fields
/// — and the scaffold encodes as the message.
#[test]
fn the_request_form_reads_a_protobuf_message() {
    let form = picked("nav.v2", "@op/set_origin");
    let Some(zengui::view::send::RequestForm::Fields { declared, fields }) = &form.request else {
        panic!("a protobuf request has fields");
    };
    assert_eq!(declared, "nav.v2.SetOriginRequest");
    assert_eq!(
        fields.iter().map(|f| f.label()).collect::<Vec<_>>(),
        ["origin: nav.v2.Origin"]
    );
    let body: serde_json::Value =
        serde_json::from_str(&form.scaffold().expect("a scaffold")).expect("JSON");
    assert_eq!(
        body,
        serde_json::json!({"origin": {"latitude_deg": 0.0, "longitude_deg": 0.0, "altitude_m": 0.0}})
    );
    let rev = revision("nav.v2");
    let plan = zenkey_fleet::plan_call(
        &rev,
        zenkey_fleet::ResolvedTarget::parse("robot1/nav").unwrap(),
        "@op/set_origin",
        Default::default(),
    )
    .unwrap();
    zenkey_fleet::encode_request(&rev, &plan, form.scaffold().unwrap().as_bytes())
        .expect("the scaffold is a SetOriginRequest in its JSON form");

    let mut ui = simulator::<Message, _, _>(pane(&form, false, sp()));
    assert!(
        ui.find("nav.v2.SetOriginRequest fields: origin: nav.v2.Origin")
            .is_ok()
    );
    for (label, value) in [
        ("RESPONSE", "nav.v2.Origin"),
        ("ERROR", "nav.v2.NavError"),
        ("IDEMPOTENT", "true"),
        ("TIMEOUT", "2000 ms"),
    ] {
        assert!(ui.find(label).is_ok() && ui.find(value).is_ok(), "{label}");
    }
}

/// O5 on screen: a value, an envelope and silence are three renderings, and
/// silence is a non-verdict — no failure chip, no error callout.
#[test]
fn the_call_mode_keeps_value_envelope_and_silence_apart() {
    let base = picked("nav.v2", "@op/set_origin");

    let value = SendForm {
        outcome: Some(Ok(report(OperationAnswer::Value {
            reply: PayloadRendering {
                key: "zk2/robot1/nav/nav.v2/@op/set_origin".into(),
                size: 27,
                resource: None,
                rendered: Rendered::Value {
                    declared: "nav.v2.Origin".into(),
                    value: serde_json::json!({"latitudeDeg": 47.6}),
                },
            },
        }))),
        ..base.clone()
    };
    {
        let mut ui = simulator::<Message, _, _>(pane(&value, false, sp()));
        assert!(ui.find("value").is_ok());
        assert!(ui.find("→ zk2/robot1/nav/nav.v2/@op/set_origin").is_ok());
        assert!(
            ui.find("nav.v2.Origin").is_ok(),
            "the declared type is named"
        );
        let decoded =
            serde_json::to_string_pretty(&serde_json::json!({"latitudeDeg": 47.6})).unwrap();
        assert!(
            ui.find(decoded.as_str()).is_ok(),
            "the reply decoded as its type"
        );
    }

    let refused = SendForm {
        outcome: Some(Ok(report(OperationAnswer::Refused {
            envelope: EnvelopeView {
                code: "unavailable".into(),
                message: "the reset line is not wired".into(),
                cause: Some("capability".into()),
                detail: None,
            },
        }))),
        ..base.clone()
    };
    {
        let mut ui = simulator::<Message, _, _>(pane(&refused, false, sp()));
        assert!(ui.find("refused: unavailable").is_ok());
        assert!(ui.find("cause: capability").is_ok());
        assert!(ui.find("No answer").is_err(), "an envelope is an answer");
    }

    let silent = SendForm {
        outcome: Some(Ok(report(OperationAnswer::Silent {
            silence: SilenceView {
                attempts: 1,
                transport: None,
                presence: PresenceAttribution::NoTokenVisible,
            },
        }))),
        ..base
    };
    let mut ui = simulator::<Message, _, _>(pane(&silent, false, sp()));
    assert!(ui.find("No answer").is_ok());
    assert!(
        ui.find(
            "no value and no envelope within 2s, after 1 attempt — a non-verdict, never an \
             error (O5). Presence: no token of the service is visible to this reader — what \
             this reader could see, not a verdict that it is gone."
        )
        .is_ok(),
        "silence is said as a non-verdict, with what presence could see"
    );
    assert!(ui.find("failed").is_err(), "silence is not an error");
    assert!(ui.find("value").is_err(), "silence is not a value");
}
