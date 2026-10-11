//! The config tool (#481).

use super::*;

/// #481: the Config tool draws only what the producer served — every class
/// in words, each value with its source and startup value, a pending change
/// and the last one verbatim — and what the producer did not say is drawn
/// as not said: a value not read back is *unknown*, a sensitive one
/// *write-only*, and a sensitive value a producer wrongly sent never
/// reaches the screen.
#[test]
fn the_config_tool_draws_the_served_schema_and_nothing_it_did_not_say() {
    use std::time::Instant;
    use zengui::configure::{ConfigForm, ConfigRead, ConfigTarget, Reply};
    use zengui::view::configure::{ConfigData, pane};

    let mut view = common::config_view();
    // A producer that breaks the rule: the view must not repeat it.
    view.groups[0].parameters[2].value =
        Some(zenkey_v1::config::ParamValue::Text("hunter2".into()));
    let target = ConfigTarget::new("h-3fa9c2d41b7e", "radio", "wlan0");
    let form = ConfigForm {
        target: target.clone(),
        read: Some(ConfigRead {
            reply: Reply::Document(Box::new(view)),
            target,
            at: Instant::now(),
        }),
        ..ConfigForm::default()
    };
    let mut ui = simulator::<Message, _, _>(pane(ConfigData {
        form: &form,
        producers: vec!["radio".into()],
        origins: vec!["h-3fa9c2d41b7e".into()],
        session: true,
        sp: sp(),
    }));
    for words in [
        "hot — applies at once",
        "reach — can cut the link",
        "contract — restart required",
        "integer 1..=10000 packets",
        "startup 1000",
        "overlay",
        "write-only",
        "unknown",
        "pending change",
        "chg-2",
        "on link — rolls back at 2026-10-05T12:00:00Z unless confirmed",
        "LAST CHANGE",
        "chg-1",
    ] {
        assert!(ui.find(words).is_ok(), "{words:?} must reach the screen");
    }
    assert!(
        ui.find("hunter2").is_err(),
        "a sensitive value is never drawn"
    );
}

/// #481's empty states: never asked is not an empty configuration, silence
/// is not a refusal, and a refusal carries the producer's words.
#[test]
fn the_config_tool_says_what_it_has_not_got() {
    use std::time::Instant;
    use zengui::configure::{ConfigForm, ConfigRead, ConfigTarget, Reply};

    fn data(form: &ConfigForm, session: bool) -> iced_test::Simulator<'_, Message> {
        simulator::<Message, _, _>(zengui::view::configure::pane(
            zengui::view::configure::ConfigData {
                form,
                producers: vec![],
                origins: vec![],
                session,
                sp: Spacing::default(),
            },
        ))
    }
    let form = ConfigForm::default();
    assert!(data(&form, false).find("Not connected").is_ok());
    assert!(data(&form, true).find("Not read yet").is_ok());

    let target = ConfigTarget::new("h-a1b2c3d4e5f6", "radio", "eth9");
    let landed = |reply| ConfigForm {
        target: target.clone(),
        read: Some(ConfigRead {
            reply,
            target: target.clone(),
            at: Instant::now(),
        }),
        ..ConfigForm::default()
    };
    let silent = landed(Reply::Silent);
    assert!(data(&silent, true).find("No reply").is_ok());
    let refused = landed(Reply::Refused {
        name: "error/not-found".into(),
        message: "no resource eth9".into(),
    });
    assert!(
        data(&refused, true)
            .find("error/not-found — no resource eth9")
            .is_ok()
    );
}

/// #481, editing: a draft that does not parse is its field's error, a
/// change the producer would refuse is refused here in the validator's own
/// words, and a contract group names the restart instead of offering a box.
/// The outcomes of a write each say what they are — an unknown outcome
/// above all, which must never read as a failure or a success.
#[test]
fn the_config_tool_refuses_here_what_the_producer_would_refuse() {
    use std::time::Instant;
    use zengui::configure::{ConfigForm, ConfigRead, ConfigTarget, GroupNote, Reply};
    use zengui::view::configure::{ConfigData, pane};

    let target = ConfigTarget::new("h-3fa9c2d41b7e", "radio", "wlan0");
    let mut form = ConfigForm {
        target: target.clone(),
        read: Some(ConfigRead {
            reply: Reply::Document(Box::new(common::config_view())),
            target,
            at: Instant::now(),
        }),
        ..ConfigForm::default()
    };
    let slot = |p: &str| ("queue".to_string(), p.to_string());
    let render = |form: &ConfigForm| {
        let mut ui = simulator::<Message, _, _>(pane(ConfigData {
            form,
            producers: vec![],
            origins: vec![],
            session: true,
            sp: sp(),
        }));
        let mut seen = Vec::new();
        for words in [
            "tx_queue_len: 0 packets is below the minimum of 1 packets",
            "expected an integer, got \"lots\"",
            "changed in the producer's startup configuration and applied by a restart \
             (RFC 05 §5.1) — read-only here",
            "NEW VALUE",
        ] {
            if ui.find(words).is_ok() {
                seen.push(words);
            }
        }
        seen
    };

    form.drafts.insert(slot("tx_queue_len"), "0".into());
    let seen = render(&form);
    assert!(seen.contains(&"tx_queue_len: 0 packets is below the minimum of 1 packets"));
    assert!(
        seen.iter()
            .any(|w| w.starts_with("changed in the producer's startup"))
    );
    assert!(
        seen.contains(&"NEW VALUE"),
        "a hot group is edited in place"
    );

    form.drafts.insert(slot("tx_queue_len"), "lots".into());
    assert!(render(&form).contains(&"expected an integer, got \"lots\""));

    // The notes, one sentence each.
    for (note, words) in [
        (
            GroupNote::Applied { revision: 5 },
            "applied — the document is at revision 5",
        ),
        (
            GroupNote::Unknown,
            "no reply within the timeout: whether it applied is unknown. Read back before \
             deciding — apply again sends the same change with the same key, so the \
             producer answers it once (RFC 05 §5.1)",
        ),
        (
            GroupNote::Refused("error/radio/device-refused — the device refused: busy".into()),
            "error/radio/device-refused — the device refused: busy",
        ),
    ] {
        form.drafts.clear();
        form.notes.insert("queue".into(), note);
        let mut ui = simulator::<Message, _, _>(pane(ConfigData {
            form: &form,
            producers: vec![],
            origins: vec![],
            session: true,
            sp: sp(),
        }));
        assert!(ui.find(words).is_ok(), "{words:?}");
    }
}

/// #481's confirmed commit, on screen: a pending change is armed, not a
/// verdict — its verbs beside it and, when this window armed it, a count
/// that names itself as this window's (the producer's deadline is the
/// truth). While it is pending, every apply says it joins. With nothing
/// pending, a reach group asks for its window and a person's yes.
#[test]
fn the_config_tool_drives_a_pending_change_and_asks_before_a_reach_one() {
    use std::time::{Duration, Instant};
    use zengui::configure::{Armed, ConfigForm, ConfigRead, ConfigTarget, Reply};
    use zengui::view::configure::{ConfigData, pane};

    let target = ConfigTarget::new("h-3fa9c2d41b7e", "radio", "wlan0");
    let view = common::config_view();
    let mut form = ConfigForm {
        target: target.clone(),
        read: Some(ConfigRead {
            reply: Reply::Document(Box::new(view.clone())),
            target: target.clone(),
            at: Instant::now(),
        }),
        armed: Some(Armed {
            token: "chg-2".into(),
            sent: Instant::now(),
            window: Duration::from_secs(60),
            reread: false,
        }),
        ..ConfigForm::default()
    };
    form.drafts
        .insert(("queue".into(), "fq".into()), "true".into());
    fn render(form: &ConfigForm) -> iced_test::Simulator<'_, Message> {
        simulator::<Message, _, _>(pane(ConfigData {
            form,
            producers: vec![],
            origins: vec![],
            session: true,
            sp: Spacing::default(),
        }))
    }
    let mut ui = render(&form);
    for words in [
        "confirm",
        "cancel — undo now",
        "extend",
        "apply — joins chg-2",
        "joins pending change chg-2 — its window covers this change too",
        "persist",
    ] {
        assert!(ui.find(words).is_ok(), "{words:?}");
    }
    assert!(
        ui.find("~60s left by this window's count, from the send — the producer's deadline above is the truth").is_ok()
            || ui.find("~59s left by this window's count, from the send — the producer's deadline above is the truth").is_ok(),
        "the count names itself as this window's"
    );

    drop(ui);

    // Nothing pending: the reach group asks for a window and a yes.
    let mut quiet = view;
    quiet.pending = None;
    form.read = Some(ConfigRead {
        reply: Reply::Document(Box::new(quiet)),
        target,
        at: Instant::now(),
    });
    form.armed = None;
    let mut ui = render(&form);
    assert!(ui.find("CONFIRM WINDOW (S)").is_ok());
    assert!(
        ui.find(
            "I understand: this can cut the link to h-3fa9c2d41b7e. It rolls back unless confirmed."
        )
        .is_ok()
    );
}

/// #481's entry points: a producer whose slice declares RFC 05 §5.1
/// procedures gets "configure" on its presence row — and only that one: a
/// link to a form nothing serves would be a promise the window cannot keep.
#[test]
fn a_configurable_producer_is_one_click_from_the_config_tool() {
    use zengui::nodes::NodeRoster;
    use zengui::view::nodes::{DetailState, NodesData, pane};

    let radio = zenkey_v1::slice::parse_slice(
        r#"
[registry]
version = "1.0"
app = "demo"
convention = 1

[producer]
name = "radio"

[[procedure]]
path = "config/{device}"
kind = "read"
reply = "ConfigView"
since = "1.0"
description = "the read-back"
"#,
    )
    .expect("a slice");
    let slices = SliceSet::from_slices(vec![radio]);
    let mut roster = NodeRoster::default();
    roster.apply_transitions(
        "",
        &[
            ("v1/h-3fa9c2d41b7e/state/radio/alive".to_string(), true),
            ("v1/h-3fa9c2d41b7e/state/sysinfo/alive".to_string(), true),
        ],
        Instant::now(),
    );
    let mut ui = simulator::<Message, _, _>(pane(NodesData {
        sp: sp(),
        roster: &roster,
        selected: None,
        detail: &DetailState::NotAsked,
        slices: Some(&slices),
    }));
    assert!(ui.find("configure").is_ok(), "radio declares config/…");
    let mut ui = simulator::<Message, _, _>(pane(NodesData {
        sp: sp(),
        roster: &roster,
        selected: None,
        detail: &DetailState::NotAsked,
        slices: None,
    }));
    assert!(
        ui.find("configure").is_err(),
        "no registry, no claim that anything is configurable"
    );
}
