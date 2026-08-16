//! The generator (#162) on a real bus: conforming traffic, marked traffic,
//! budgeted events, and the served RFC 08 halves.
//! Ports 7537-7538 (disjoint from every other test binary).

use std::time::Duration;

use zenkey_fleet::generate::{GenPattern, GenSpec, build_plan, run_gen, serve_describe};

async fn peer_pair(port: u16) -> (zenoh::Session, zenoh::Session) {
    let listen = zenkey_fleet::session::open(&[], &[format!("tcp/127.0.0.1:{port}")], false)
        .await
        .expect("listener session");
    let connect = zenkey_fleet::session::open(&[format!("tcp/127.0.0.1:{port}")], &[], false)
        .await
        .expect("connector session");
    (listen, connect)
}

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
    }
}

/// The run publishes schema-valid bodies on the declared keys, every sample
/// marked synthetic (RFC 09 §5.3), and events stay inside their budget on
/// write-once keys.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn generated_traffic_is_conforming_marked_and_budgeted() {
    let (observer, generator) = peer_pair(7537).await;
    let slices =
        zenkey_fleet::SliceSet::from_slices(vec![zenkey::parse_slice(SLICES).expect("slice")]);
    let set = zenkey::schema::SchemaSet::parse(SET).expect("set");
    let store = zenkey_fleet::decode::SchemaStore::new("", Duration::from_millis(200));

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
    let report = run_gen(&generator, &store, &plan, &spec(2.0))
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
    let (serving, asking) = peer_pair(7538).await;
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("demo.toml"), SLICES).expect("write slice");
    let slices = zenkey_fleet::SliceSet::from_dirs(&[dir.path().to_path_buf()]).expect("from_dirs");
    let set = zenkey::schema::SchemaSet::parse(SET).expect("set");

    let mock = serve_describe(&serving, "", "h-fefefefefefe", &slices, Some(&set), None)
        .await
        .expect("serve");
    assert_eq!(mock.keys, 2, "introspect + describe for the one producer");

    let introspect = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let answers = zenkey_fleet::fleet_get(
                &asking,
                "",
                "v1/h-fefefefefefe/@rpc/demo/introspect",
                None,
                Duration::from_millis(500),
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
        &asking,
        "",
        "v1/h-fefefefefefe/@rpc/demo/describe",
        None,
        Duration::from_millis(500),
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
