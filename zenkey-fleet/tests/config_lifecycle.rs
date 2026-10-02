//! The configuration write lifecycle (RFC 05 §5.1) through the engine's own
//! `call`, against a producer that serves it (#500).
//!
//! `call` had been driven only by attachments and traces; no test had ever
//! sent a change, read it back, confirmed it, or watched one lapse — and no
//! producer in the repo could have answered. The producer here is the
//! config test double (`util/config_server.rs`, shared with zenctl's live
//! suite), declared through `BringUp` like any producer.
//!
//! Ports are ephemeral (`util::peer_pair`), so two test runs at once
//! cannot collide.

use std::time::{Duration, Instant};

use zenkey::config::{ConfigChange, ConfigView, ControlRequest, ParamValue, ValueSource};
use zenkey_fleet::bus::producer::BringUp;
use zenkey_fleet::report::{CallOutcome, CallReport};
use zenkey_fleet::{CallSpec, CallTarget, Fleet};

mod util;
use util::{SETTLE, peer_pair};

#[path = "util/config_server.rs"]
mod config_server;
use config_server::{ConfigServer, RESOURCE};

const ORIGIN: &str = "h-c0f1c0f1c0f1";
const PRODUCER: &str = "radio";

/// One call, through the engine's typed builders — the path `zenctl config`
/// takes.
async fn call(
    session: &zenoh::Session,
    target: &str,
    procedure: &str,
    body: Option<Vec<u8>>,
) -> CallReport {
    let target = CallTarget::parse(target).expect("target");
    zenkey_fleet::call(
        &Fleet::new(session, ""),
        CallSpec {
            target: &target,
            producer: PRODUCER,
            procedure,
            params: &[],
            body,
            attachment: None,
            timeout: Duration::from_secs(5),
            slices: None,
            force: false,
        },
    )
    .await
    .expect("call")
}

/// The one answer's value; anything else is a failure naming what came back.
fn value(report: &CallReport) -> serde_json::Value {
    assert_eq!(
        report.answers.len(),
        1,
        "one origin, one answer: {report:?}"
    );
    match &report.answers[0].outcome {
        CallOutcome::Ok { value: Some(v), .. } => v.clone(),
        other => panic!("expected a value reply, got {other:?}"),
    }
}

fn view(report: &CallReport) -> ConfigView {
    serde_json::from_value(value(report)).expect("a read-back document")
}

/// The one answer's error name.
fn refusal(report: &CallReport) -> String {
    assert_eq!(
        report.answers.len(),
        1,
        "one origin, one answer: {report:?}"
    );
    match &report.answers[0].outcome {
        CallOutcome::Err(e) => e.name.clone(),
        other => panic!("expected an error reply, got {other:?}"),
    }
}

fn param(view: &ConfigView, group: &str, name: &str) -> (Option<ParamValue>, Option<ValueSource>) {
    let p = view
        .group(group)
        .and_then(|g| g.parameters.iter().find(|p| p.spec.name == name))
        .expect("declared parameter");
    (p.value.clone(), p.source)
}

fn change(name: &str, v: ParamValue, confirm_s: Option<u64>) -> Vec<u8> {
    let mut c = ConfigChange::of([(name, v)]);
    c.confirm_s = confirm_s;
    serde_json::to_vec(&c).expect("change")
}

fn control(token: &str, confirm_s: Option<u64>) -> Vec<u8> {
    let mut r = ControlRequest::of(token);
    r.confirm_s = confirm_s;
    serde_json::to_vec(&r).expect("control")
}

/// get → set (pending) → confirm → persist; set → cancel; extend; busy; a
/// lapse; the contract refusal; and the server-side fan-out refusal — the
/// whole §5.1 lifecycle, each step asserted on the reply the engine projects.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_change_is_read_set_confirmed_persisted_cancelled_extended_and_lapses() {
    let (server, client) = peer_pair().await;
    let double = ConfigServer::fixture(PRODUCER);
    let mut up = BringUp::new(&server);
    double
        .declare(&mut up, &format!("v1/{ORIGIN}/@rpc/{PRODUCER}"))
        .await
        .expect("declare the config procedures");
    let mut live = up
        .alive(&format!("v1/{ORIGIN}/state/{PRODUCER}/alive"))
        .await
        .expect("alive");
    let _served: Vec<_> = std::mem::take(&mut live.responders)
        .into_iter()
        .map(|r| double.spawn(r))
        .collect();

    let read = format!("config/{RESOURCE}");
    // Routability first: a query sent before the peers meet is silence,
    // not an answer.
    let deadline = Instant::now() + SETTLE;
    let first = loop {
        let report = call(&client, ORIGIN, &read, None).await;
        if !report.answers.is_empty() {
            break view(&report);
        }
        assert!(
            Instant::now() < deadline,
            "the double never became routable"
        );
    };
    assert_eq!(first.resource, RESOURCE);
    assert_eq!(first.revision, 1);
    assert!(first.pending.is_none());
    assert_eq!(
        param(&first, "queue", "tx_queue_len"),
        (Some(ParamValue::Integer(1000)), Some(ValueSource::File))
    );

    // set → pending: applied at once, a token and a deadline armed.
    let set = format!("config/{RESOURCE}/queue/set");
    let v = view(
        &call(
            &client,
            ORIGIN,
            &set,
            Some(change("tx_queue_len", ParamValue::Integer(2000), Some(60))),
        )
        .await,
    );
    let pending = v.pending.clone().expect("a confirmed change is pending");
    assert_eq!(pending.groups, ["queue"]);
    assert!(pending.deadline.is_some(), "the deadline is spelled");
    assert_eq!(
        param(&v, "queue", "tx_queue_len"),
        (Some(ParamValue::Integer(2000)), Some(ValueSource::Runtime))
    );
    assert_eq!(v.revision, 2);

    // One pending change per resource: a second writer is busy.
    let busy = call(
        &client,
        ORIGIN,
        &set,
        Some(change("fq", ParamValue::Bool(true), None)),
    )
    .await;
    assert_eq!(refusal(&busy), "error/busy");
    assert_eq!(busy.exit_code(), 1);

    // extend moves the deadline.
    let extended = view(
        &call(
            &client,
            ORIGIN,
            &format!("config/{RESOURCE}/extend"),
            Some(control(&pending.token, Some(3600))),
        )
        .await,
    );
    let moved = extended.pending.expect("still pending").deadline;
    assert!(
        moved > pending.deadline,
        "extend moves the deadline: {:?} → {moved:?}",
        pending.deadline
    );

    // confirm makes it permanent; persist writes it to the persisted layer.
    let confirmed = view(
        &call(
            &client,
            ORIGIN,
            &format!("config/{RESOURCE}/confirm"),
            Some(control(&pending.token, None)),
        )
        .await,
    );
    assert!(confirmed.pending.is_none());
    assert_eq!(
        param(&confirmed, "queue", "tx_queue_len").0,
        Some(ParamValue::Integer(2000))
    );
    let persisted = view(
        &call(
            &client,
            ORIGIN,
            &format!("config/{RESOURCE}/persist"),
            Some(control(&pending.token, None)),
        )
        .await,
    );
    assert_eq!(
        param(&persisted, "queue", "tx_queue_len"),
        (Some(ParamValue::Integer(2000)), Some(ValueSource::Overlay))
    );

    // An unknown token is not-found, never a silent success.
    let stray = call(
        &client,
        ORIGIN,
        &format!("config/{RESOURCE}/confirm"),
        Some(control("chg-404", None)),
    )
    .await;
    assert_eq!(refusal(&stray), "error/not-found");

    // set → cancel reverts.
    let v = view(
        &call(
            &client,
            ORIGIN,
            &set,
            Some(change("fq", ParamValue::Bool(true), Some(60))),
        )
        .await,
    );
    let token = v.pending.expect("pending").token;
    let cancelled = view(
        &call(
            &client,
            ORIGIN,
            &format!("config/{RESOURCE}/cancel"),
            Some(control(&token, None)),
        )
        .await,
    );
    assert!(cancelled.pending.is_none());
    assert_eq!(
        param(&cancelled, "queue", "fq"),
        (Some(ParamValue::Bool(false)), Some(ValueSource::File))
    );

    // An unconfirmed change lapses at its deadline: rolled back, no token.
    let v = view(
        &call(
            &client,
            ORIGIN,
            &set,
            Some(change("tx_queue_len", ParamValue::Integer(3000), Some(1))),
        )
        .await,
    );
    assert!(v.pending.is_some());
    let deadline = Instant::now() + SETTLE;
    let lapsed = loop {
        let v = view(&call(&client, ORIGIN, &read, None).await);
        if v.pending.is_none() {
            break v;
        }
        assert!(Instant::now() < deadline, "the change never lapsed");
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert_eq!(
        param(&lapsed, "queue", "tx_queue_len"),
        (Some(ParamValue::Integer(2000)), Some(ValueSource::Overlay)),
        "the lapse restores the value and the source the change replaced"
    );

    // A contract group is the producer's own refusal, with the restart named.
    let contract = call(
        &client,
        ORIGIN,
        &format!("config/{RESOURCE}/transport/set"),
        Some(change("mtu", ParamValue::Integer(9000), None)),
    )
    .await;
    assert_eq!(
        refusal(&contract),
        format!("error/{PRODUCER}/restart-required")
    );

    // A reach change answers {token, apply_at}, before the read-back.
    let reach = value(
        &call(
            &client,
            ORIGIN,
            &format!("config/{RESOURCE}/link/set"),
            Some(change("ssid", ParamValue::Text("field".into()), Some(60))),
        )
        .await,
    );
    assert!(
        reach["token"].is_string() && reach["apply_at"].is_string(),
        "{reach}"
    );

    // The server's own guard (RFC 05 §2.1, v1.38): a broadcast write is
    // refused at the producer whatever the caller's side did or did not
    // check — here it checked nothing, because no slices were loaded.
    let fanned = call(
        &client,
        "*",
        &set,
        Some(change("fq", ParamValue::Bool(true), None)),
    )
    .await;
    assert_eq!(refusal(&fanned), "error/fanout-forbidden");
}
