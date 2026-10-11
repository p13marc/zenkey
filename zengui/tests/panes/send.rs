//! The send pane: call mode, publish mode, and both over one form.

use super::*;

/// The Send pane's call mode scaffolds from slices and labels the fanout
/// refusal (issue #60, merged by #184): the visual layer of the three-layer
/// guard.
#[test]
fn the_call_mode_labels_forbidden_fanout() {
    use zengui::view::send::{SendForm, SendMode, pane};
    use zenkey_v1::slice::{ProcedureDecl, RegistrySlice};

    let mut trigger = ProcedureDecl::new("capture/trigger");
    trigger.kind = Some(zenkey_v1::ProcedureKind::Write.into());
    trigger.reply = Some("Ack".into());
    trigger.fanout = Some(zenkey_v1::Fanout::Forbidden.into());
    trigger.idempotent = Some(false);
    let mut slice = RegistrySlice::new("1.0", "t", "netring");
    slice.procedures = vec![trigger];
    let slices = SliceSet::from_slices(vec![slice]);

    let form = SendForm {
        mode: SendMode::Call,
        producer: Some("netring".into()),
        procedure: Some("capture/trigger".into()),
        target: "*".into(),
        ..SendForm::default()
    };
    let roster = zengui::nodes::NodeRoster::default();
    let mut ui = simulator::<Message, _, _>(pane(&form, Some(&slices), &roster, sp()));
    assert!(
        ui.find("fanout = \"forbidden\" — a fleet (*) target is refused (RFC 05 §2.1)")
            .is_ok(),
        "the refusal must be visible before any send"
    );
    // #234: the `@rpc` surface the pane picked from is finally shown — the
    // call key `service_info` spells, `{origin}` standing for the
    // publishing identity (#211).
    assert!(
        ui.find("→ v1/{origin}/@rpc/netring/capture/trigger")
            .is_ok(),
        "the declared call key is on screen, not left to be reconstructed"
    );
    // The declared shape as fields since #562 (was one joined line):
    // each value under its eyebrow, "—" where the registry is silent.
    for (label, value) in [
        ("FANOUT", "forbidden"),
        ("IDEMPOTENT", "false"),
        ("REPLY", "Ack"),
    ] {
        assert!(ui.find(label).is_ok(), "{label}");
        assert!(ui.find(value).is_ok(), "{label} {value}");
    }

    // Without a registry the pane says "not asked", not empty dropdowns.
    let empty = SendForm {
        mode: SendMode::Call,
        ..SendForm::default()
    };
    let mut ui = simulator::<Message, _, _>(pane(&empty, None, &roster, sp()));
    assert!(ui.find("No registry loaded").is_ok());
}

/// Silence gets a name (#60's first acceptance): an origin the roster says is
/// alive and running the producer, that did not reply, is listed. RFC 05 §3.1
/// says "no reply" is not one condition — this is the join that says which.
#[test]
fn the_call_mode_names_the_origins_that_did_not_answer() {
    use zengui::view::send::{SendForm, SendMode, pane};
    use zenkey_fleet_v1::report::{CallAnswer, CallReport};

    let mut roster = zengui::nodes::NodeRoster::default();
    roster.seed(&BTreeMap::from([
        ("h-aaaaaaaaaaaa".to_string(), vec!["netring".to_string()]),
        ("h-bbbbbbbbbbbb".to_string(), vec!["netring".to_string()]),
        // A third node that does not run the producer must NOT be blamed.
        ("h-cccccccccccc".to_string(), vec!["sysinfo".to_string()]),
    ]));

    let form = SendForm {
        mode: SendMode::Call,
        producer: Some("netring".into()),
        procedure: Some("introspect".into()),
        target: "*".into(),
        outcome: Some(Ok(CallReport {
            key: "v1/*/@rpc/netring/introspect".into(),
            timeout_s: 5.0,
            answers: vec![CallAnswer {
                origin: "h-aaaaaaaaaaaa".into(),
                outcome: zenkey_fleet_v1::report::CallOutcome::Ok {
                    value: None,
                    text: Some("slice".into()),
                },
                attachment: Some(serde_json::json!({"who": "me"})),
                attachment_bytes: Some(12),
            }],
        })),
        ..SendForm::default()
    };
    let mut introspect = zenkey_v1::slice::ProcedureDecl::new("introspect");
    introspect.kind = Some(zenkey_v1::ProcedureKind::Read.into());
    introspect.reply = Some("RegistrySlice".into());
    introspect.idempotent = Some(true);
    let mut netring = zenkey_v1::slice::RegistrySlice::new("1.0", "t", "netring");
    netring.procedures = vec![introspect];
    let slices = SliceSet::from_slices(vec![netring]);
    let mut ui = simulator::<Message, _, _>(pane(&form, Some(&slices), &roster, sp()));
    assert!(
        ui.find("did not answer, though alive: h-bbbbbbbbbbbb")
            .is_ok(),
        "the alive non-replier must be named, not folded into silence"
    );
}

/// Request-form scaffolding (§6.4 item 3): a declared request type whose
/// schema has not been fetched reads as "not asked", never as "takes
/// nothing" (O4) — and once fetched, the fields are on screen.
#[test]
fn the_call_mode_distinguishes_an_unasked_schema_from_an_empty_one() {
    use zengui::view::send::{SchemaField, SendForm, SendMode, pane};
    use zenkey_v1::slice::{ProcedureDecl, RegistrySlice};

    let mut start = ProcedureDecl::new("capture/start");
    start.kind = Some(zenkey_v1::ProcedureKind::Write.into());
    start.reply = Some("Ack".into());
    start.request = Some("CaptureSpec".into());
    start.idempotent = Some(false);
    let mut slice = RegistrySlice::new("1.0", "t", "netring");
    slice.procedures = vec![start];
    let slices = SliceSet::from_slices(vec![slice]);
    let roster = zengui::nodes::NodeRoster::default();

    let unasked = SendForm {
        mode: SendMode::Call,
        producer: Some("netring".into()),
        procedure: Some("capture/start".into()),
        ..SendForm::default()
    };
    {
        let mut ui = simulator::<Message, _, _>(pane(&unasked, Some(&slices), &roster, sp()));
        assert!(
            ui.find("CaptureSpec: schema not asked yet — pick the procedure again once connected to scaffold from it")
                .is_ok(),
            "an unfetched schema must not read as a request with no fields"
        );
    }

    let asked = SendForm {
        request_fields: Some(vec![
            SchemaField {
                name: "iface".into(),
                type_name: Some("string".into()),
                required: true,
            },
            SchemaField {
                name: "seconds".into(),
                type_name: Some("integer".into()),
                required: false,
            },
        ]),
        ..unasked
    };
    {
        let mut ui = simulator::<Message, _, _>(pane(&asked, Some(&slices), &roster, sp()));
        assert!(
            ui.find("CaptureSpec fields: iface: string*, seconds: integer")
                .is_ok(),
            "the served schema's fields scaffold the form"
        );
    }
    // …and the scaffold is a real body, with the declared shapes.
    let body: serde_json::Value =
        serde_json::from_str(&asked.scaffold().expect("scaffold")).expect("valid JSON");
    assert_eq!(body["iface"], serde_json::json!(""));
    assert_eq!(body["seconds"], serde_json::json!(0));
}

/// The publish mode's three provenances must never look alike (#60/#97):
/// encoded, sent as typed, and sent raw are different facts about what is on
/// the wire, and a user who cannot tell them apart cannot trust any of them.
#[test]
fn the_publish_mode_says_how_the_body_reached_the_wire() {
    use zengui::view::send::{SendForm, pane};
    use zenkey_fleet_v1::BodySource;

    let slices = slices();
    let roster = zengui::nodes::NodeRoster::default();
    let base = SendForm {
        key: "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used".into(),
        body: "{\"value\": 1}".into(),
        encoding_used: Some("application/protobuf".into()),
        source: Some(BodySource::Encoded {
            type_name: "TelemetryPoint".into(),
        }),
        ..SendForm::default()
    };
    {
        let mut ui = simulator::<Message, _, _>(pane(&base, Some(&slices), &roster, sp()));
        assert!(
            ui.find("encoded as TelemetryPoint → application/protobuf")
                .is_ok()
        );
    }

    let as_typed = SendForm {
        source: Some(BodySource::AsTyped),
        encoding_used: None,
        note: Some("sysinfo serves no schema for TelemetryPoint".into()),
        ..base.clone()
    };
    {
        let mut ui = simulator::<Message, _, _>(pane(&as_typed, Some(&slices), &roster, sp()));
        assert!(ui.find("sent as typed → (no encoding set)").is_ok());
        assert!(
            ui.find("sysinfo serves no schema for TelemetryPoint")
                .is_ok(),
            "the engine's note is rendered, not swallowed"
        );
    }

    let raw = SendForm {
        source: Some(BodySource::Raw),
        raw: true,
        ..base.clone()
    };
    let mut ui = simulator::<Message, _, _>(pane(&raw, Some(&slices), &roster, sp()));
    assert!(ui.find("sent raw — bytes verbatim, not encoded").is_ok());
}

/// The publish mode's honesty surfaces: the closed QoS vocabulary, the O4
/// matching badge, and the O6 bounded log.
#[test]
fn the_publish_mode_bounds_its_log_and_never_invents_a_matcher() {
    use zengui::view::send::{LOG_LINES, SendForm, pane};

    let slices = slices();
    let roster = zengui::nodes::NodeRoster::default();
    let mut form = SendForm {
        key: "demo/foreign/key".into(),
        armed: true,
        // Armed, but the status could not be asked: "not asked" (O4).
        matching: None,
        ..SendForm::default()
    };
    {
        let mut ui = simulator::<Message, _, _>(pane(&form, Some(&slices), &roster, sp()));
        assert!(ui.find("matching: not asked").is_ok());
    }
    // The five profiles, and only those (RFC 04 §3's closed vocabulary).
    assert_eq!(zenkey_v1::qos::QosProfile::ALL.len(), 5);
    for name in ["sampled", "refreshed", "transition", "alert", "frame"] {
        assert!(
            zenkey_v1::qos::QosProfile::ALL
                .iter()
                .any(|p| p.name() == name),
            "{name} must stay in the vocabulary the picker renders"
        );
    }

    form.matching = Some(false);
    {
        let mut ui = simulator::<Message, _, _>(pane(&form, Some(&slices), &roster, sp()));
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
        form.log(true, format!("sent {i} bytes"));
    }
    {
        let mut ui = simulator::<Message, _, _>(pane(&form, Some(&slices), &roster, sp()));
        assert!(
            ui.find(format!(
                "send log — {LOG_LINES} shown, 3 dropped (bounded at {LOG_LINES})"
            ))
            .is_ok(),
            "a trimmed log must say how much it trimmed"
        );
    }

    // With no registry the pane says "not asked", never "unregistered".
    let mut ui = simulator::<Message, _, _>(pane(&form, None, &roster, sp()));
    assert!(
        ui.find(
            "no registry loaded — the body cannot be schema-checked, and that is \
             \"not asked\", not \"unregistered\" (O4)"
        )
        .is_ok()
    );
}

/// #184's acceptance: one pane, one form, both modes. The mode strip is on
/// screen whichever mode is showing, and switching is a message the strip
/// itself emits — not a second pane wearing a toggle.
#[test]
fn the_send_pane_offers_both_modes_over_one_form() {
    use zengui::message::PaneMsg;
    use zengui::view::send::{SendForm, SendMode, SendMsg, pane};

    let slices = slices();
    let roster = zengui::nodes::NodeRoster::default();
    let mut form = SendForm::default();
    {
        let mut ui = simulator::<Message, _, _>(pane(&form, Some(&slices), &roster, sp()));
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
    let mut ui = simulator::<Message, _, _>(pane(&form, Some(&slices), &roster, sp()));
    assert!(ui.find("put / publish").is_ok(), "…and the way back too");
}
