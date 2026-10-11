//! The config tool (#481) against a live producer serving the RFC 05 §5.1
//! double.

use super::*;

/// The RFC 05 §5.1 test double. zengui is its only includer since #612's
/// FJ9 took `config` out of zenctl and the in-tree engine; the file's own
/// header says why it moved here from zenkey-fleet (#614, FL5).
#[path = "../../tests/util/config_server.rs"]
mod config_server;

/// A live producer serving the double's fixture as `radio` on `origin`;
/// the client session to reach it, and the guards that keep it serving.
async fn config_producer(
    origin: &str,
) -> (
    zenoh::Session,
    config_server::ConfigServer,
    Vec<tokio::task::JoinHandle<()>>,
    zenkey_fleet_v1::bus::producer::LiveProducer,
    zenoh::Session,
) {
    use zenkey_fleet_v1::bus::producer::BringUp;
    let server = zenkey_fleet_v1::bus::session::open(&[], &["tcp/127.0.0.1:0".to_string()], false)
        .await
        .expect("the producer's session");
    let endpoint = server
        .info()
        .locators()
        .await
        .into_iter()
        .map(|l| l.to_string())
        .find(|l| l.starts_with("tcp/127.0.0.1:"))
        .expect("a loopback listener");
    let client = zenkey_fleet_v1::bus::session::open(&[endpoint], &[], false)
        .await
        .expect("the explorer's session");
    let double = config_server::ConfigServer::fixture("radio");
    let mut up = BringUp::new(&server);
    double
        .declare(&mut up, &format!("v1/{origin}/@rpc/radio"))
        .await
        .expect("declare the config procedures");
    let mut live = up
        .alive(&format!("v1/{origin}/state/radio/alive"))
        .await
        .expect("alive");
    let served = std::mem::take(&mut live.responders)
        .into_iter()
        .map(|r| double.spawn(r))
        .collect();
    (client, double, served, live, server)
}

/// Read the double's resource through the tool until it answers — a query
/// sent before the peers meet is silence, not an answer.
async fn config_open(app: &mut Zengui, origin: &str) {
    use crate::configure::ConfigTarget;
    use crate::view::configure::ConfigMsg;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let task = app.update(Message::Pane(crate::message::PaneMsg::Config(
            ConfigMsg::Open(ConfigTarget::new(origin, "radio", config_server::RESOURCE)),
        )));
        drive(app, task).await;
        if app.work.bench.config_form.view().is_some() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the double never answered"
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

/// #481 against the double, nothing faked: read → a hot change typed against
/// the served kind and judged by the producer's own validator → sent, and
/// the read-back it answered is the document. Out of bounds is refused here,
/// in the validator's words, and nothing is sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_config_tool_applies_a_hot_change_and_refuses_an_unsound_one_unsent() {
    use crate::configure::GroupNote;
    use crate::view::configure::ConfigMsg;
    use zenkey_v1::config::{ParamValue, ValueSource};

    const ORIGIN: &str = "h-c0f1c0f1c0f1";
    let (client, double, _served, _live, _server) = config_producer(ORIGIN).await;
    let mut app = test_app();
    app.dep.raw = Some(client);
    config_open(&mut app, ORIGIN).await;
    assert_eq!(app.work.bench.config_form.view().unwrap().revision, 1);

    let config = |m| Message::Pane(crate::message::PaneMsg::Config(m));
    let draft = |text: &str| {
        config(ConfigMsg::Draft {
            group: "queue".into(),
            param: "tx_queue_len".into(),
            text: text.into(),
        })
    };

    // Below the declared minimum: the validator refuses, the button is the
    // floor's, and the producer hears nothing.
    let _ = app.update(draft("0"));
    let task = app.update(config(ConfigMsg::Apply("queue".into())));
    drive(&mut app, task).await;
    assert!(
        double.received().iter().all(|p| !p.ends_with("/set")),
        "an unsound change is never sent: {:?}",
        double.received()
    );

    // In bounds: sent, applied, read back.
    let _ = app.update(draft("2000"));
    let task = app.update(config(ConfigMsg::Apply("queue".into())));
    drive(&mut app, task).await;
    let form = &app.work.bench.config_form;
    assert_eq!(
        form.notes.get("queue"),
        Some(&GroupNote::Applied { revision: 2 })
    );
    assert!(form.drafts.is_empty(), "a spent draft is cleared");
    let view = form.view().expect("the hot set's reply is the document");
    let q = &view.group("queue").unwrap().parameters[0];
    assert_eq!(
        (q.value.clone(), q.source),
        (Some(ParamValue::Integer(2000)), Some(ValueSource::Runtime))
    );
    assert!(
        view.last_change.is_some(),
        "a windowless change is the last change (RFC v1.50)"
    );
    assert_eq!(
        double.value("queue", "tx_queue_len"),
        Some(ParamValue::Integer(2000))
    );
}

/// #481's confirmed commit against the double: a reach change leaves only
/// with a window and a person's yes, is answered with its token before it
/// applies, and is armed from the send; a hot change made while it is
/// pending joins it (RFC v1.50); confirm makes both the last change, and
/// persist writes that to the producer's persisted layer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_config_tool_arms_joins_confirms_and_persists_a_reach_change() {
    use crate::configure::Verb;
    use crate::message::PaneMsg;
    use crate::view::configure::ConfigMsg;
    use zenkey_v1::config::ValueSource;

    const ORIGIN: &str = "h-c0f1c0f1c0f2";
    let (client, double, _served, _live, _server) = config_producer(ORIGIN).await;
    let mut app = test_app();
    app.dep.raw = Some(client);
    config_open(&mut app, ORIGIN).await;

    let config = |m| Message::Pane(PaneMsg::Config(m));
    let sets = |double: &config_server::ConfigServer| {
        double
            .received()
            .iter()
            .filter(|p| p.ends_with("/set"))
            .count()
    };
    let _ = app.update(config(ConfigMsg::Draft {
        group: "link".into(),
        param: "ssid".into(),
        text: "field".into(),
    }));

    // No window, then no yes: nothing leaves either time.
    let task = app.update(config(ConfigMsg::Apply("link".into())));
    drive(&mut app, task).await;
    let _ = app.update(config(ConfigMsg::WindowChanged("60".into())));
    let task = app.update(config(ConfigMsg::Apply("link".into())));
    drive(&mut app, task).await;
    assert_eq!(sets(&double), 0, "a reach change needs a window and a yes");

    // With both: answered with its token, armed, read back.
    let _ = app.update(config(ConfigMsg::ConsentToggled(true)));
    let task = app.update(config(ConfigMsg::Apply("link".into())));
    drive(&mut app, task).await;
    let form = &app.work.bench.config_form;
    let token = form
        .armed
        .as_ref()
        .expect("armed from the send")
        .token
        .clone();
    assert!(!form.consent, "the yes was for that change");
    let pending = form.view().unwrap().pending.clone().expect("pending");
    assert_eq!(
        (pending.token.as_str(), pending.groups.as_slice()),
        (token.as_str(), &["link".to_string()][..])
    );

    // A hot change now joins it.
    let _ = app.update(config(ConfigMsg::Draft {
        group: "queue".into(),
        param: "fq".into(),
        text: "true".into(),
    }));
    let task = app.update(config(ConfigMsg::Apply("queue".into())));
    drive(&mut app, task).await;
    let view = app.work.bench.config_form.view().unwrap();
    assert_eq!(
        view.pending.as_ref().unwrap().groups,
        ["link", "queue"],
        "one change, two groups"
    );

    // Confirm: nothing pending, both groups the last change, disarmed.
    let task = app.update(config(ConfigMsg::Control(Verb::Confirm, token.clone())));
    drive(&mut app, task).await;
    let form = &app.work.bench.config_form;
    let view = form.view().unwrap();
    assert!(view.pending.is_none());
    let last = view.last_change.as_ref().expect("the confirmed change");
    assert_eq!(
        (last.token.as_str(), last.groups.as_slice()),
        (
            token.as_str(),
            &["link".to_string(), "queue".to_string()][..]
        )
    );
    assert!(form.armed.is_none());

    // Persist it.
    let task = app.update(config(ConfigMsg::Control(Verb::Persist, token)));
    drive(&mut app, task).await;
    let view = app.work.bench.config_form.view().unwrap();
    let ssid = &view.group("link").unwrap().parameters[0];
    assert_eq!(ssid.source, Some(ValueSource::Overlay));
}
