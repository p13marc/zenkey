//! `check conform` (#222) against a real bus — the generator is the suite's
//! oracle. A conforming mock (`serve_describe` for the RFC 08 halves, a
//! clean `run_gen` for the traffic, `BringUp` responders for the procedures
//! with `alive` last) conforms; each injected fault fails exactly the one
//! assertion it names; and the procedure rows of RFC 13 §3 land where the
//! RFC puts them.
//! Ports are ephemeral (`util::peer_pair`), so two test runs at once
//! cannot collide.

use std::time::Duration;

use zenkey_fleet::bus::producer::{BringUp, LiveProducer, ReservedError};
use zenkey_fleet::model::registry::SliceSource;
use zenkey_fleet::report::{AssertionState, ConformReport, ConformVerdict, Fault};
use zenkey_fleet::tape::generate::{GenPattern, GenSpec, build_plan, run_gen, serve_describe};
use zenkey_fleet::{ConformSpec, Fleet, SliceSet, run_conform};

mod util;
use util::{SETTLE, peer_pair};

const ORIGIN: &str = "h-fefefefefefe";
const ALIVE: &str = "v1/h-fefefefefefe/state/demo/alive";

/// The traffic half: one state subject, declared QoS and a served schema.
const TRAFFIC: &str = r#"
[registry]
version = "1.0"
app = "t"
convention = 1
[producer]
name = "demo"
[[subject]]
path = "health"
class = "state"
type = "Health"
qos = "transition"
[[procedure]]
path = "status"
kind = "read"
[[procedure]]
path = "dns"
kind = "read"
when = ["config:collect.dns"]
gate_note = "enable collect.dns"
[[procedure]]
path = "reset"
kind = "write"
"#;

/// The procedure half: one of each RFC 13 §3 row.
const PROCEDURES: &str = r#"
[registry]
version = "1.0"
app = "t"
convention = 1
[producer]
name = "demo"
[[subject]]
path = "health"
class = "state"
type = "Health"
[[procedure]]
path = "status"
kind = "read"
[[procedure]]
path = "args"
kind = "read"
[[procedure]]
path = "dns"
kind = "read"
when = ["config:collect.dns"]
[[procedure]]
path = "probe"
kind = "read"
[[procedure]]
path = "missing"
kind = "read"
[[procedure]]
path = "rf0/rssi"
kind = "read"
when = ["capability:rssi"]
[[procedure]]
path = "sat0/rssi"
kind = "read"
when = ["capability:rssi"]
[[procedure]]
path = "reset"
kind = "write"
"#;

const SET: &str = r#"{"schema_version":1,"app":"t","types":{
    "Health":{"kind":"json-schema","hash":"","schema":{
        "type":"object","required":["ok"],
        "properties":{"ok":{"type":"boolean"}}}}}}"#;

/// A slice set with its verbatim TOML, which `serve_describe` serves.
fn slices(toml: &str) -> (tempfile::TempDir, SliceSet) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("demo.toml"), toml).expect("write slice");
    let set = SliceSet::from_dirs(&[dir.path().to_path_buf()]).expect("from_dirs");
    (dir, set)
}

/// How each procedure the fixture declares answers.
#[derive(Clone, Copy)]
enum Answer {
    Value,
    Err(ReservedError),
}

/// Bring the producer up in the RFC 04 §5 order — every procedure queryable
/// first, `alive` last — and drive each responder with its fixed answer.
async fn bring_up(
    session: &zenoh::Session,
    procedures: &[(&str, Answer)],
) -> (LiveProducer, Vec<tokio::task::JoinHandle<()>>) {
    let mut up = BringUp::new(session);
    for (path, _) in procedures {
        up.serve(&format!("v1/{ORIGIN}/@rpc/demo/{path}"))
            .await
            .expect("declare procedure");
    }
    let mut live = up.alive(ALIVE).await.expect("alive last");
    let mut tasks = Vec::new();
    for (responder, (_, answer)) in std::mem::take(&mut live.responders)
        .into_iter()
        .zip(procedures.iter().copied())
    {
        tasks.push(tokio::spawn(async move {
            while let Some(q) = responder.next().await {
                let _ = match answer {
                    Answer::Value => {
                        responder
                            .reply(&q, b"{\"ok\":true}".to_vec(), Some("application/json"))
                            .await
                    }
                    Answer::Err(e) => responder.reply_err(&q, e, "fixture").await,
                };
            }
        }));
    }
    (live, tasks)
}

/// The roster half of "alive ⇒ callable": the token is visible before the
/// suite reads the roster, so its calls race nothing.
async fn wait_alive(session: &zenoh::Session) {
    tokio::time::timeout(SETTLE, async {
        loop {
            if let Ok(replies) = session
                .liveliness()
                .get("v1/*/state/*/alive")
                .timeout(Duration::from_millis(300))
                .await
            {
                while let Ok(reply) = replies.recv_async().await {
                    if reply
                        .into_result()
                        .is_ok_and(|s| s.key_expr().as_str() == ALIVE)
                    {
                        return;
                    }
                }
            }
        }
    })
    .await
    .expect("the alive token should be visible");
}

fn spec(listen: Option<f64>, origin: Option<&str>) -> ConformSpec {
    ConformSpec {
        producer: "demo".into(),
        origin: origin.map(str::to_string),
        listen: listen.map(Duration::from_secs_f64),
        deep: false,
        timeout: Duration::from_millis(500),
        source: SliceSource::Dirs,
    }
}

fn state_of<'r>(report: &'r ConformReport, id: &str) -> &'r AssertionState {
    &report
        .assertions
        .iter()
        .find(|a| a.id == id)
        .unwrap_or_else(|| panic!("no assertion {id}: {:#?}", report.assertions))
        .state
}

fn not_met(report: &ConformReport) -> Vec<&str> {
    report
        .assertions
        .iter()
        .filter(|a| a.state.is_not_met())
        .map(|a| a.id.as_str())
        .collect()
}

/// The traffic fixture, generating `faults` (none = conforming) for the
/// whole run, judged over a two-second window.
async fn traffic_run(faults: Vec<Fault>) -> ConformReport {
    let (observer, producer) = peer_pair().await;
    let (_dir, set) = slices(TRAFFIC);
    let schemas = zenkey::schema::SchemaSet::parse(SET).expect("set");
    let fleet = Fleet::new(&producer, "");

    let _mock = serve_describe(&fleet, ORIGIN, &set, Some(&schemas), Some("demo"))
        .await
        .expect("serve describe");
    let (_live, _tasks) = bring_up(
        &producer,
        &[
            ("status", Answer::Value),
            ("dns", Answer::Err(ReservedError::Gated)),
            ("reset", Answer::Value),
        ],
    )
    .await;
    wait_alive(&observer).await;

    let gen_spec = GenSpec {
        origin: ORIGIN.into(),
        producer: Some("demo".into()),
        subject: None,
        vars: vec![],
        rate_hz: Some(5.0),
        pattern: GenPattern::Steady,
        duration: Duration::from_secs(30),
        seed: 7,
        tool: "zenctl gen".into(),
        faults,
    };
    let store = zenkey_fleet::SchemaStore::new("", Duration::from_millis(200));
    let plan = build_plan(None, &store, &set, "", Some(&schemas), &gen_spec)
        .await
        .expect("plan");
    let generating = {
        let producer = producer.clone();
        tokio::spawn(async move {
            let _ = run_gen(&Fleet::new(&producer, ""), &plan, &gen_spec).await;
        })
    };

    let report = run_conform(&Fleet::new(&observer, ""), &set, &spec(Some(2.0), None))
        .await
        .expect("run_conform");
    generating.abort();
    report
}

/// The oracle's clean half: the conforming mock conforms — every assertion
/// met, the gated `when` procedure exempt and saying so, the write never
/// called and met by its served declaration.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_conforming_mock_conforms() {
    let report = traffic_run(vec![]).await;
    assert_eq!(report.verdict, ConformVerdict::Conforms, "{report:#?}");
    assert_eq!(report.origins_asked, [ORIGIN]);
    assert!(report.assertions.iter().all(|a| a.state.is_met()));

    let dns = report
        .assertions
        .iter()
        .find(|a| a.id == "procedure/dns")
        .expect("dns");
    assert_eq!(dns.exempt.as_deref(), Some("when: config:collect.dns"));
    let reset = report
        .assertions
        .iter()
        .find(|a| a.id == "procedure/reset")
        .expect("reset");
    assert!(
        reset.evidence.contains("never called"),
        "{}",
        reset.evidence
    );
    for id in [
        "procedure/introspect",
        "procedure/status",
        "slice-sync",
        "describe-totality",
        "schema-drift/Health",
        "observed/health",
        "qos-observed-mismatch/health",
        "payload/health",
        "unregistered-traffic",
    ] {
        assert!(state_of(&report, id).is_met(), "{id}: {report:#?}");
    }
    let junit = report.junit();
    assert!(!junit.contains("<failure"), "{junit}");
}

/// Each fault the generator injects fails exactly the one assertion that
/// names it — the oracle's other half (#163's deltas, judged).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_qos_fails_only_the_qos_assertion() {
    let report = traffic_run(vec![Fault::WrongQos]).await;
    assert_eq!(
        not_met(&report),
        ["qos-observed-mismatch/health"],
        "{report:#?}"
    );
    assert_eq!(report.verdict, ConformVerdict::Violates);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unregistered_key_fails_only_the_unregistered_assertion() {
    let report = traffic_run(vec![Fault::UnregisteredKey]).await;
    assert_eq!(
        not_met(&report),
        ["unregistered-traffic/health/unregistered"],
        "{report:#?}"
    );
    // The declared key never rode: unknowable, never failed — a window
    // proves presence, never absence (RFC 13 §3).
    assert!(state_of(&report, "observed/health").is_unknowable());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_type_fails_only_the_payload_assertion() {
    let report = traffic_run(vec![Fault::WrongType]).await;
    assert_eq!(not_met(&report), ["payload/health"], "{report:#?}");
}

/// The procedure rows, one fixture: a value and `invalid-args` are met;
/// `gated` from a `when` procedure is exempt, from a plain one not met;
/// a declared procedure nobody answers on a rostered origin is not met;
/// and a `gated` reply the device's own capabilities contradict is not met,
/// while the device that does not claim it stays exempt (RFC 04 §5, v1.41).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_procedure_rows_land_where_rfc_13_puts_them() {
    let (observer, producer) = peer_pair().await;
    let (_dir, set) = slices(PROCEDURES);
    let schemas = zenkey::schema::SchemaSet::parse(SET).expect("set");
    let _mock = serve_describe(
        &Fleet::new(&producer, ""),
        ORIGIN,
        &set,
        Some(&schemas),
        Some("demo"),
    )
    .await
    .expect("serve describe");

    // The registration document: rf0 claims rssi, sat0 does not.
    let _sensor = producer
        .declare_queryable(format!("v1/{ORIGIN}/state/demo/sensor"))
        .callback(|query| {
            tokio::spawn(async move {
                // On its own concrete key: the suite asks with a `*`
                // producer, and a reply on the query's wildcard is refused.
                let doc = br#"{"capabilities":{"rf0":["sdu","rssi"],"sat0":["sdu"]}}"#;
                let _ = query
                    .reply(format!("v1/{ORIGIN}/state/demo/sensor"), doc.to_vec())
                    .await;
            });
        })
        .await
        .expect("sensor queryable");

    let (_live, _tasks) = bring_up(
        &producer,
        &[
            ("status", Answer::Value),
            ("args", Answer::Err(ReservedError::InvalidArgs)),
            ("dns", Answer::Err(ReservedError::Gated)),
            ("probe", Answer::Err(ReservedError::Gated)),
            ("rf0/rssi", Answer::Err(ReservedError::Gated)),
            ("sat0/rssi", Answer::Err(ReservedError::Gated)),
        ],
    )
    .await;
    wait_alive(&observer).await;

    let report = run_conform(&Fleet::new(&observer, ""), &set, &spec(None, None))
        .await
        .expect("run_conform");

    assert!(state_of(&report, "procedure/status").is_met());
    assert!(
        state_of(&report, "procedure/args").is_met(),
        "invalid-args is a reply"
    );
    let exempt = |id: &str| {
        report
            .assertions
            .iter()
            .find(|a| a.id == id)
            .and_then(|a| a.exempt.clone())
    };
    assert!(state_of(&report, "procedure/dns").is_met());
    assert_eq!(
        exempt("procedure/dns").as_deref(),
        Some("when: config:collect.dns")
    );
    assert!(state_of(&report, "procedure/sat0/rssi").is_met());
    assert_eq!(
        exempt("procedure/sat0/rssi").as_deref(),
        Some("when: capability:rssi")
    );
    assert!(
        state_of(&report, "procedure/reset").is_met(),
        "a write is never called"
    );

    assert_eq!(
        not_met(&report),
        ["procedure/probe", "procedure/missing", "procedure/rf0/rssi"],
        "{report:#?}"
    );
    assert_eq!(report.verdict, ConformVerdict::Violates);
    // No window: the subject half is not asked, and the report says so.
    assert!(report.observation.is_none());
    assert!(
        report.not_asked.iter().any(|n| n.starts_with("observed/*")),
        "{:?}",
        report.not_asked
    );
}

/// An `--origin` the roster does not show: its silence is attributable to
/// nothing, so every run-time assertion is unknowable and the suite is
/// unproven — never violated (RFC 13 §2).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unrostered_origin_is_unknowable_never_failed() {
    let (observer, _producer) = peer_pair().await;
    let (_dir, set) = slices(PROCEDURES);

    let report = run_conform(
        &Fleet::new(&observer, ""),
        &set,
        &spec(None, Some("h-abababababab")),
    )
    .await
    .expect("run_conform");
    assert_eq!(report.origins_asked, ["h-abababababab"]);
    assert!(state_of(&report, "procedure/introspect").is_unknowable());
    assert!(state_of(&report, "procedure/status").is_unknowable());
    assert!(not_met(&report).is_empty(), "{report:#?}");
    assert_eq!(report.verdict, ConformVerdict::Unproven);
    let junit = report.junit();
    assert!(junit.contains("<skipped"), "{junit}");
    assert!(!junit.contains("<failure"), "{junit}");
}

/// A producer no loaded slice declares is refused before anything is
/// asked — the CLI's exit 2.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_undeclared_producer_is_refused() {
    let (observer, _producer) = peer_pair().await;
    let (_dir, set) = slices(PROCEDURES);
    let mut s = spec(None, None);
    s.producer = "nobody".into();
    let err = run_conform(&Fleet::new(&observer, ""), &set, &s)
        .await
        .expect_err("refused");
    assert!(err.is_unaskable(), "{err}");
}
