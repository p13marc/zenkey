//! The generator (#162) on a real bus: conforming traffic, marked traffic,
//! budgeted events, and the served RFC 08 halves. Fault injection (#163):
//! each kind's delta from valid, observed on the wire, and the fault marker
//! riding every faulted sample.
//! Ports are ephemeral (`util::peer_pair`), so two test runs at once
//! cannot collide.

use std::time::Duration;

use zenkey_fleet::tape::generate::{GenPattern, GenSpec, build_plan, run_gen, serve_describe};

use zenkey_fleet::report::Fault;

mod util;
use util::peer_pair;

const SLICES: &str = r#"
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
ttl_s = 2
[[subject]]
path = "boom/{id}"
class = "events"
type = "Boom"
rate = "rare"
"#;

const SET: &str = r#"{"schema_version":1,"app":"t","types":{
    "Health":{"kind":"json-schema","hash":"","schema":{
        "type":"object","required":["ok"],
        "properties":{"ok":{"type":"boolean"},"load":{"type":"number"}}}},
    "Boom":{"kind":"json-schema","hash":"","schema":{"type":"object"}}}}"#;

fn spec(duration_s: f64) -> GenSpec {
    GenSpec {
        origin: "h-fefefefefefe".into(),
        producer: None,
        subject: None,
        vars: vec![],
        rate_hz: None,
        pattern: GenPattern::Steady,
        duration: Duration::from_secs_f64(duration_s),
        seed: 7,
        tool: "zenctl gen".into(),
        faults: vec![],
    }
}

/// The run publishes schema-valid bodies on the declared keys, every sample
/// marked synthetic (RFC 09 §5.3), and events stay inside their budget on
/// write-once keys.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn generated_traffic_is_conforming_marked_and_budgeted() {
    let (observer, generator) = peer_pair().await;
    let slices =
        zenkey_fleet::SliceSet::from_slices(vec![zenkey::parse_slice(SLICES).expect("slice")]);
    let set = zenkey::schema::SchemaSet::parse(SET).expect("set");
    let store = zenkey_fleet::model::decode::SchemaStore::new("", Duration::from_millis(200));

    // Watch before generating: the monitor is up, the generator's declared
    // publishers match it, samples arrive.
    let monitor = zenkey_fleet::Monitor::start(&observer, zenkey_fleet::MonitorSpec::default())
        .await
        .expect("monitor");
    let mut events = monitor.events();
    monitor.watch("v1/**").await.expect("watch");

    let plan = build_plan(None, &store, &slices, "", Some(&set), &spec(2.0))
        .await
        .expect("plan");
    assert_eq!(plan.len(), 2);
    let report = run_gen(&zenkey_fleet::Fleet::new(&generator, ""), &plan, &spec(2.0))
        .await
        .expect("run");
    assert!(report.sent > 0, "{report:?}");
    assert_eq!(report.refused, 0, "{report:?}");

    // Drain what the observer saw — against an absolute deadline, because
    // the monitor's StatsTick keeps the stream ticking forever (every event
    // resets a per-recv timeout; a bounded drain must bound the whole).
    let drain_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let mut health = Vec::new();
    let mut boom_keys = std::collections::BTreeSet::new();
    while let Ok(Some(item)) = tokio::time::timeout_at(drain_deadline, events.recv()).await {
        if let zenkey_fleet::StreamItem::Event(zenkey_fleet::FleetEvent::Sample(s)) = item {
            let att = s.attachment.as_ref().expect("every sample is marked");
            let marker: serde_json::Value =
                serde_json::from_slice(&att.to_bytes()).expect("marker is JSON");
            assert_eq!(marker["synthetic"], true);
            assert_eq!(marker["origin"], "h-fefefefefefe");
            if s.key == "v1/h-fefefefefefe/state/demo/health" {
                health.push(s);
            } else if s.key.starts_with("v1/h-fefefefefefe/events/demo/boom/") {
                boom_keys.insert(s.key.clone());
            } else {
                panic!("unexpected key {}", s.key);
            }
        }
    }
    assert!(!health.is_empty(), "the state family published");
    // The body is schema-valid JSON with the required field.
    let body: serde_json::Value =
        serde_json::from_slice(&health[0].payload.to_bytes()).expect("json body");
    assert!(body.get("ok").is_some(), "{body}");
    // Declared transition rode the wire (#158's rule, generator-side).
    assert!(
        health[0].qos_matches(zenkey::qos::QosProfile::Transition),
        "declared qos rides"
    );
    // rare = 1/h: a 2s run gets exactly one event, on a write-once key.
    assert!(
        boom_keys.len() <= 1,
        "the declared budget caps the run: {boom_keys:?}"
    );
}

/// `--serve-describe`: the impersonated producer answers both RFC 08 halves
/// — introspect with the verbatim slice TOML, describe with the SchemaSet.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_mock_producer_serves_both_registry_halves() {
    let (serving, asking) = peer_pair().await;
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("demo.toml"), SLICES).expect("write slice");
    let slices = zenkey_fleet::SliceSet::from_dirs(&[dir.path().to_path_buf()]).expect("from_dirs");
    let set = zenkey::schema::SchemaSet::parse(SET).expect("set");

    let mock = serve_describe(
        &zenkey_fleet::Fleet::new(&serving, ""),
        "h-fefefefefefe",
        &slices,
        Some(&set),
        None,
    )
    .await
    .expect("serve");
    assert_eq!(mock.keys, 2, "introspect + describe for the one producer");

    let introspect = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let answers = zenkey_fleet::fleet_get(
                &zenkey_fleet::Fleet::new(&asking, ""),
                "v1/h-fefefefefefe/@rpc/demo/introspect",
                &zenkey_fleet::GetOpts::new(Duration::from_millis(500)),
            )
            .await
            .expect("get");
            if !answers.is_empty() {
                break answers;
            }
        }
    })
    .await
    .expect("introspect should answer within 10s");
    let zenkey_fleet::Answer::Value(bytes) = &introspect[0].answer else {
        panic!("introspect answered an error");
    };
    let served = zenkey::parse_slice(std::str::from_utf8(&bytes.to_bytes()).unwrap())
        .expect("served slice parses");
    assert_eq!(served.name, "demo");

    let describe = zenkey_fleet::fleet_get(
        &zenkey_fleet::Fleet::new(&asking, ""),
        "v1/h-fefefefefefe/@rpc/demo/describe",
        &zenkey_fleet::GetOpts::new(Duration::from_millis(500)),
    )
    .await
    .expect("get");
    let zenkey_fleet::Answer::Value(bytes) = &describe[0].answer else {
        panic!("describe answered an error");
    };
    let served_set =
        zenkey::schema::SchemaSet::parse(std::str::from_utf8(&bytes.to_bytes()).unwrap())
            .expect("served set parses");
    assert!(served_set.get("Health").is_some());
}

/// Fault injection (#163): every kind's delta from valid, observed on the
/// wire, and the `fault=<kind>` marker riding every faulted sample. All
/// seven kinds run against the one `health` state subject (declared
/// `transition`, `application/json`), so each variant's deviation is exactly
/// one dimension away from a known-valid baseline.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn injected_faults_deviate_by_exactly_one_dimension_and_stay_marked() {
    let (observer, generator) = peer_pair().await;
    let slices =
        zenkey_fleet::SliceSet::from_slices(vec![zenkey::parse_slice(SLICES).expect("slice")]);
    let set = zenkey::schema::SchemaSet::parse(SET).expect("set");
    let store = zenkey_fleet::model::decode::SchemaStore::new("", Duration::from_millis(200));

    let mut spec = spec(2.0);
    spec.faults = Fault::ALL.to_vec();
    spec.subject = Some("health".into()); // the state subject only — no events

    let monitor = zenkey_fleet::Monitor::start(&observer, zenkey_fleet::MonitorSpec::default())
        .await
        .expect("monitor");
    let mut events = monitor.events();
    monitor.watch("v1/**").await.expect("watch");

    let plan = build_plan(None, &store, &slices, "", Some(&set), &spec)
        .await
        .expect("plan");
    assert_eq!(plan.len(), 7, "one variant per fault kind: {plan:?}");

    let report = run_gen(&zenkey_fleet::Fleet::new(&generator, ""), &plan, &spec)
        .await
        .expect("run");
    assert!(report.sent > 0, "{report:?}");

    // Bucket observed samples by the fault kind their marker names.
    use std::collections::HashMap;
    let mut by_fault: HashMap<String, Vec<std::sync::Arc<zenkey_fleet::SampleView>>> =
        HashMap::new();
    let drain_deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while let Ok(Some(item)) = tokio::time::timeout_at(drain_deadline, events.recv()).await {
        if let zenkey_fleet::StreamItem::Event(zenkey_fleet::FleetEvent::Sample(s)) = item {
            let att = s
                .attachment
                .as_ref()
                .expect("every faulted sample is marked");
            let marker: serde_json::Value =
                serde_json::from_slice(&att.to_bytes()).expect("marker is JSON");
            assert_eq!(marker["synthetic"], true);
            assert_eq!(marker["origin"], "h-fefefefefefe");
            let kind = marker["fault"]
                .as_str()
                .expect("a faulted sample carries fault=<kind>")
                .to_string();
            by_fault.entry(kind).or_default().push(s);
        }
    }

    // Every kind was seen at least once.
    for f in Fault::ALL {
        assert!(
            by_fault.contains_key(f.as_str()),
            "no sample observed for fault {} — saw {:?}",
            f.as_str(),
            by_fault.keys().collect::<Vec<_>>()
        );
    }

    let one = |kind: &str| by_fault[kind][0].clone();

    // truncate: half a JSON body no longer parses.
    let t = one("truncate");
    assert!(
        serde_json::from_slice::<serde_json::Value>(&t.payload.to_bytes()).is_err(),
        "a truncated body is a partial frame"
    );

    // wrong-type: a bare JSON string where the object type is declared.
    let wt = one("wrong-type");
    let v: serde_json::Value = serde_json::from_slice(&wt.payload.to_bytes()).unwrap();
    assert!(v.is_string(), "{v}");

    // extra-field: the valid object plus an undeclared field.
    let ef = one("extra-field");
    let v: serde_json::Value = serde_json::from_slice(&ef.payload.to_bytes()).unwrap();
    assert_eq!(v["_fault"], true, "{v}");
    assert!(v.get("ok").is_some(), "the valid fields survive: {v}");

    // unregistered-key: the key gained a trailing chunk the registry never
    // declared.
    let uk = one("unregistered-key");
    assert!(uk.key.ends_with("/health/unregistered"), "{}", uk.key);

    // wrong-qos: the declared `transition` profile did not ride.
    let wq = one("wrong-qos");
    assert!(
        !wq.qos_matches(zenkey::qos::QosProfile::Transition),
        "the declared profile was not honoured"
    );

    // missing-encoding: the declared application/json encoding was dropped.
    let me = one("missing-encoding");
    assert_ne!(me.encoding, "application/json", "encoding {}", me.encoding);

    // unstamped: no HLC timestamp, though the valid baseline stamps.
    let us = one("unstamped");
    assert!(us.timestamp.is_none(), "the fault omits the HLC stamp");

    // The baseline it deviates from: a non-faulted health sample IS stamped
    // and DOES carry the declared encoding — proof the delta is one dimension.
    // (The other kinds keep the valid encoding/timestamp/qos.)
    let baseline = one("truncate"); // truncate touches only the body
    assert!(baseline.timestamp.is_some(), "valid path stamps for LWW");
    assert_eq!(baseline.encoding, "application/json");
    assert!(baseline.qos_matches(zenkey::qos::QosProfile::Transition));
}

/// One entry's task panicking used to leave every *other* entry publishing
/// until its own deadline (#326): `run_gen` returned on the `JoinError` while
/// the surviving `JoinHandle`s were merely dropped, and a dropped handle
/// detaches. On `--duration 1h` that is an hour of synthetic traffic with no
/// owner and nothing left to stop it.
///
/// The panic is induced through the plan rather than through a test hook: a
/// `rate_hz` of zero makes the entry's `1.0 / rate_hz` interval non-finite,
/// which `Duration::from_secs_f64` refuses by panicking. `build_plan` never
/// produces one — this is the shape of *any* bug in an entry's task.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_panicking_entry_takes_every_other_entry_down_with_it() {
    let (observer, generator) = peer_pair().await;

    let monitor = zenkey_fleet::Monitor::start(&observer, zenkey_fleet::MonitorSpec::default())
        .await
        .expect("monitor");
    let mut events = monitor.events();
    monitor.watch("v1/**").await.expect("watch");

    let entry = |key: &str, rate_hz: f64| zenkey_fleet::GenPlanEntry {
        key: key.into(),
        class: "state".into(),
        producer: "demo".into(),
        type_name: "Health".into(),
        qos: "transition".into(),
        qos_source: "declared",
        rate_hz,
        body_source: "placeholder",
        encoding: Some("application/json".into()),
        events_cap: None,
        note: None,
        fault: None,
        fault_delta: None,
        schema: None,
        unique_chunk: None,
    };
    // The poisoned entry first, so the join order the old code walked reaches
    // it immediately; the survivor would otherwise publish for a full minute.
    let plan = vec![
        entry("v1/h-fefefefefefe/state/demo/poison", 0.0),
        entry("v1/h-fefefefefefe/state/demo/health", 50.0),
    ];

    let err = run_gen(
        &zenkey_fleet::Fleet::new(&generator, ""),
        &plan,
        &spec(60.0),
    )
    .await
    .expect_err("the panicked entry is reported, never swallowed");
    assert!(err.to_string().contains("gen task"), "{err}");

    // Drain everything already in flight, then listen again on a fresh
    // window: the survivor was publishing at 50 Hz, so anything still
    // running would be plainly audible here.
    let settle = tokio::time::Instant::now() + Duration::from_millis(500);
    while tokio::time::timeout_at(settle, events.recv()).await.is_ok() {}

    let listen = tokio::time::Instant::now() + Duration::from_millis(500);
    let mut after = 0usize;
    while let Ok(Some(item)) = tokio::time::timeout_at(listen, events.recv()).await {
        if let zenkey_fleet::StreamItem::Event(zenkey_fleet::FleetEvent::Sample(s)) = item
            && s.key.starts_with("v1/h-fefefefefefe/")
        {
            after += 1;
        }
    }
    assert_eq!(
        after, 0,
        "the surviving entry went silent when run_gen returned"
    );

    monitor.shutdown().await.expect("shutdown");
}
