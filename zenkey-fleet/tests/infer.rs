//! Registry inference (#225, RFC 08 §6.1): the acceptance round-trip.
//!
//! Inferring from traffic a registry generated must reproduce that
//! registry's **shape** — the `(class, pattern)` set with `{var}`s in the
//! same positions (names may differ; a draft cannot know a name). Not the
//! metadata: units, rates and cardinalities are guesses the tests below pin
//! individually. Two corpora: the pure path over synthetic rows expanded
//! from the fixture registries, and the bus path over `zenctl gen` traffic.
//!
//! **Two origins, two values per var.** One expansion cannot prove a
//! dimension (the module's failure mode 1), and one `gen` run publishes one
//! value per var from one origin — so the bus test runs the generator twice,
//! with different `vars` and origins, into one observation. That is not a
//! test convenience; it is the evidence the heuristic asks for, and the test
//! documents it.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use zenkey::pattern::{PatternChunk, SubjectPattern};
use zenkey_fleet::model::infer::{
    InferObservation, Provenance, infer, to_draft_toml, to_draft_types_toml,
};
use zenkey_fleet::report::InferReport;

mod util;

const A: &str = "h-aaaaaaaaaaaa";
const B: &str = "h-bbbbbbbbbbbb";

fn fixture(name: &str) -> zenkey::RegistrySlice {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fixture-tests/registry")
        .join(name);
    zenkey::parse_slice(&std::fs::read_to_string(&path).expect("fixture registry"))
        .expect("fixture parses")
}

/// A pattern with its var names erased: `{mount}` → `{v}`, `{path...}` →
/// `{v...}` — the shape, which is what a draft can reproduce.
fn shape(pattern: &str) -> String {
    let p = SubjectPattern::parse(pattern).expect("a pattern");
    p.chunks()
        .iter()
        .map(|c| match c {
            PatternChunk::Literal(l) => l.clone(),
            PatternChunk::Var(_) => "{v}".to_string(),
            PatternChunk::Rest(_) => "{v...}".to_string(),
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Values a var takes on each origin: A publishes `_a`/`_b`, B `_b`/`_c` —
/// two values per origin, populations that overlap without coinciding.
fn var_values(var: &str, origin: &str) -> Vec<String> {
    let suffixes = if origin == A { ["a", "b"] } else { ["b", "c"] };
    suffixes.iter().map(|s| format!("{var}_{s}")).collect()
}

/// Tails a rest-var takes: mixed depth, host-conditional at every depth.
fn rest_values(origin: &str) -> Vec<String> {
    if origin == A {
        vec!["pa/x".into(), "pb".into(), "pe".into(), "pc/y/z".into()]
    } else {
        vec!["pb".into(), "pc/y/z".into(), "pf".into(), "pd/w".into()]
    }
}

/// Every concrete tail one origin publishes for a declared pattern.
fn expand(pattern: &str, origin: &str) -> Vec<String> {
    let p = SubjectPattern::parse(pattern).expect("a pattern");
    let mut tails: Vec<Vec<String>> = vec![Vec::new()];
    for c in p.chunks() {
        let choices: Vec<String> = match c {
            PatternChunk::Literal(l) => vec![l.clone()],
            PatternChunk::Var(v) => var_values(v, origin),
            PatternChunk::Rest(_) => rest_values(origin),
        };
        tails = tails
            .iter()
            .flat_map(|t| {
                choices.iter().map(move |c| {
                    let mut t = t.clone();
                    t.push(c.clone());
                    t
                })
            })
            .collect();
    }
    tails.into_iter().map(|t| t.join("/")).collect()
}

fn feed_registry(obs: &mut InferObservation, slice: &zenkey::RegistrySlice) {
    let doc = serde_json::json!({"type": "gauge", "value": 1.5});
    for d in &slice.subjects {
        let class = d.class.token().to_string();
        let samples = if class == "events" { 1 } else { 2 };
        for origin in [A, B] {
            for tail in expand(&d.path, origin) {
                let key = format!("v1/{origin}/{class}/{}/{tail}", slice.name);
                for i in 0..samples {
                    obs.observe(
                        &key,
                        i as f64 * 5.0,
                        Some("application/cbor"),
                        Some("sampled"),
                        Some(&doc),
                    );
                }
            }
        }
    }
}

fn declared_shapes(slice: &zenkey::RegistrySlice) -> BTreeSet<(String, String)> {
    slice
        .subjects
        .iter()
        .map(|d| (d.class.token().to_string(), shape(&d.path)))
        .collect()
}

fn inferred_shapes(report: &InferReport, producer: &str) -> BTreeSet<(String, String)> {
    report
        .producers
        .iter()
        .filter(|p| p.name == producer)
        .flat_map(|p| p.subjects.iter().map(|s| (s.class.clone(), shape(&s.path))))
        .collect()
}

fn assert_same_shape(declared: &BTreeSet<(String, String)>, inferred: &BTreeSet<(String, String)>) {
    let missing: Vec<_> = declared.difference(inferred).collect();
    let extra: Vec<_> = inferred.difference(declared).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "shape mismatch —\n  declared but not inferred: {missing:#?}\n  inferred but not declared: {extra:#?}"
    );
}

/// The reference registry, 130-odd subjects with every var shape the
/// convention uses — nested vars, a var beside a literal at the same
/// position, literal leaf clusters — round-trips as a shape.
#[test]
fn sysinfo_round_trips_as_a_shape() {
    let slice = fixture("sysinfo.toml");
    let mut obs = InferObservation::new("", 50_000, 50_000);
    feed_registry(&mut obs, &slice);
    let report = infer(&obs, "v1/**", Some(10.0), None);
    assert_same_shape(
        &declared_shapes(&slice),
        &inferred_shapes(&report, "sysinfo"),
    );
    // Nothing was defaulted: the state entries refreshed every 5 s, so
    // their ttl is a hint of 10; telemetry carries no ttl and no rate.
    for s in &report.producers[0].subjects {
        match s.class.as_str() {
            "state" => assert_eq!(s.ttl_s, Some(10), "{}", s.path),
            _ => assert!(s.ttl_s.is_none() && s.rate.is_none(), "{}", s.path),
        }
        assert_eq!(s.encoding.as_deref(), Some("application/cbor"));
        assert!(
            s.comments
                .iter()
                .any(|c| c.starts_with("qos observed: sampled")),
            "{:?}",
            s.comments
        );
        if s.path.contains('{') {
            assert!(s.cardinality.is_some(), "{}", s.path);
        } else {
            assert!(s.cardinality.is_none(), "{}", s.path);
        }
    }
    // The unit suffix rule, on a leaf that has one.
    let bytes = report.producers[0]
        .subjects
        .iter()
        .find(|s| s.path.ends_with("/rx_bytes"))
        .expect("network/{iface}/rx_bytes");
    assert_eq!(bytes.unit.as_deref(), Some("bytes"));
    let counter = report.producers[0]
        .subjects
        .iter()
        .find(|s| s.path == "memory/oom_kills_total")
        .expect("memory/oom_kills_total");
    assert!(counter.unit.is_none());
    assert!(
        counter
            .comments
            .iter()
            .any(|c| c.contains("kind guess: counter")),
        "{:?}",
        counter.comments
    );
}

/// A device-defined tree (`{device}/{path...}`) round-trips as a rest-var.
#[test]
fn gnmi_round_trips_with_its_rest_var() {
    let slice = fixture("gnmi.toml");
    let mut obs = InferObservation::new("", 10_000, 10_000);
    feed_registry(&mut obs, &slice);
    let report = infer(&obs, "v1/**", Some(10.0), None);
    assert_same_shape(&declared_shapes(&slice), &inferred_shapes(&report, "gnmi"));
    let rest = report.producers[0]
        .subjects
        .iter()
        .find(|s| s.path.ends_with("...}"))
        .expect("the rest family");
    assert!(
        rest.comments
            .iter()
            .any(|c| c.contains("device-defined tree")),
        "{:?}",
        rest.comments
    );
}

/// The emitted draft directory lints with `--allow-drafts` and is refused
/// without — the loop the issue asks `registry lint` to close.
#[test]
fn the_draft_directory_lints_only_when_drafts_are_admitted() {
    let slice = fixture("sysinfo.toml");
    let mut obs = InferObservation::new("", 50_000, 50_000);
    feed_registry(&mut obs, &slice);
    let report = infer(&obs, "v1/**", Some(10.0), None);
    let prov = Provenance {
        app: "unknown".into(),
        source: "v1/**".into(),
        span_s: report.span_s,
        at: "2026-09-06T00:00:00Z".into(),
        keys: report.keys_seen,
        samples: report.samples,
        dropped: 0,
    };
    let dir = std::env::temp_dir().join(format!("zenkey-infer-draft-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("schemas")).unwrap();
    for p in &report.producers {
        std::fs::write(
            dir.join(format!("{}.toml", p.name)),
            to_draft_toml(p, &prov),
        )
        .unwrap();
    }
    std::fs::write(
        dir.join("types.toml"),
        to_draft_types_toml(&report.producers, &prov),
    )
    .unwrap();
    for (path, body) in zenkey_fleet::model::infer::draft_schema_files(&report.producers) {
        std::fs::write(dir.join(path), body).unwrap();
    }
    let refused = zenkey_build::Config::new()
        .registry_dir(&dir)
        .no_rerun_if_changed()
        .lint()
        .unwrap_err();
    assert!(
        matches!(
            refused,
            zenkey_build::Error::Lint {
                kind: zenkey_build::LintKind::Draft,
                ..
            }
        ),
        "{refused}"
    );
    let warnings = zenkey_build::Config::new()
        .registry_dir(&dir)
        .no_rerun_if_changed()
        .allow_drafts(true)
        .lint()
        .expect("an admitted draft lints clean");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].message.starts_with("draft"), "{warnings:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The bus path: `zenctl gen`'s traffic, run twice (two origins, two var
/// values — one run cannot exhibit a dimension), inferred through the same
/// observation the live verb feeds, reproduces the generating registry's
/// shape.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gen_traffic_round_trips_as_a_shape() {
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
path = "cpu/{core}/usage_percent"
class = "telemetry"
type = "Point"
cardinality = 8
[[subject]]
path = "boom/{id}"
class = "events"
type = "Boom"
rate = "burst(100000/h)"
"#;
    const SET: &str = r#"{"schema_version":1,"app":"t","types":{
        "Health":{"kind":"json-schema","hash":"","schema":{
            "type":"object","required":["ok"],
            "properties":{"ok":{"type":"boolean"},"load":{"type":"number"}}}},
        "Point":{"kind":"json-schema","hash":"","schema":{"type":"object","properties":{"value":{"type":"number"}}}},
        "Boom":{"kind":"json-schema","hash":"","schema":{"type":"object"}}}}"#;

    let (observer, generator) = util::peer_pair().await;
    let slice = zenkey::parse_slice(SLICES).expect("slice");
    let slices = zenkey_fleet::SliceSet::from_slices(vec![slice.clone()]);
    let set = zenkey::schema::SchemaSet::parse(SET).expect("set");
    let store = zenkey_fleet::model::decode::SchemaStore::new("", Duration::from_millis(200));

    let monitor = zenkey_fleet::Monitor::start(&observer, zenkey_fleet::MonitorSpec::default())
        .await
        .expect("monitor");
    let mut events = monitor.events();
    monitor.watch("v1/**").await.expect("watch");
    let opened = std::time::Instant::now();

    let fleet = zenkey_fleet::Fleet::new(&generator, "");
    for (origin, core, seed) in [(A, "cpu0", 7u64), (B, "cpu1", 11)] {
        let spec = zenkey_fleet::GenSpec {
            origin: origin.into(),
            producer: None,
            subject: None,
            vars: vec![("core".into(), core.into())],
            rate_hz: None,
            pattern: zenkey_fleet::GenPattern::Steady,
            duration: Duration::from_secs_f64(1.5),
            seed,
            tool: "zenctl gen".into(),
            faults: vec![],
        };
        let plan = zenkey_fleet::build_plan(None, &store, &slices, "", Some(&set), &spec)
            .await
            .expect("plan");
        let report = zenkey_fleet::run_gen(&fleet, &plan, &spec)
            .await
            .expect("run");
        assert!(report.sent > 0 && report.refused == 0, "{report:?}");
    }

    let mut obs = InferObservation::new("", 1000, 1000);
    let drain_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while let Ok(Some(item)) = tokio::time::timeout_at(drain_deadline, events.recv()).await {
        if let zenkey_fleet::StreamItem::Event(zenkey_fleet::FleetEvent::Sample(s)) = item {
            let bytes = s.payload.to_bytes();
            let doc = zenkey_fleet::structural_value(&bytes);
            let qos = zenkey::QosProfile::ALL
                .iter()
                .find(|p| s.qos_matches(**p))
                .map(|p| p.name().to_string());
            obs.observe(
                &s.key,
                s.received.duration_since(opened).as_secs_f64(),
                Some(&s.encoding),
                qos.as_deref(),
                doc.as_ref(),
            );
        }
    }
    monitor.stop();

    let report = infer(&obs, "v1/**", Some(3.0), None);
    assert_same_shape(&declared_shapes(&slice), &inferred_shapes(&report, "demo"));
    let demo = &report.producers[0];
    let health = demo.subjects.iter().find(|s| s.path == "health").unwrap();
    // The generator refreshes state at ttl/2 = 1 s; the hint is twice the
    // median inter-arrival — 2 s, the declared ttl, give or take a tick.
    assert!(matches!(health.ttl_s, Some(2..=3)), "{health:?}");
    assert!(
        health
            .comments
            .iter()
            .any(|c| c.starts_with("qos observed: transition")),
        "the declared transition rode and is a comment, not a field: {:?}",
        health.comments
    );
    let boom = demo
        .subjects
        .iter()
        .find(|s| s.path.starts_with("boom/"))
        .unwrap();
    assert!(boom.rate.is_some(), "{boom:?}");
    let cpu = demo
        .subjects
        .iter()
        .find(|s| s.path.starts_with("cpu/"))
        .unwrap();
    assert_eq!(cpu.unit.as_deref(), Some("percent"));
    assert_eq!(
        cpu.cardinality,
        Some(1),
        "one core per origin, rounded up: {cpu:?}"
    );
    assert_eq!(cpu.encoding.as_deref(), Some("application/json"));
    assert_eq!(report.origins, 2);
}
