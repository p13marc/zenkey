//! The live suite: the real `zenctl` binary against a real bus (#499, #500).
//!
//! `tests/cli.rs` pins what zenctl says with no bus; the render snapshots pin
//! how a report draws; zenkey-fleet's suites pin the engine. None of them ran
//! what zenctl adds on top — the argument → `GetOpts` mapping, the guards in
//! `cmd/*.rs`, the rendering of *real* replies, and every 0 and 1 a verdict
//! verb can exit with. Every case here does: an in-process publisher of
//! foreign keys on an ephemeral port (`live/harness.rs`), the binary run as
//! a script would run it, and its stdout, stderr and exit code asserted
//! against the contract in `zenctl/src/exit.rs`. The zk2 cases — services,
//! contracts, owners — are `tests/live_zk2.rs`.
//!
//! Later chunks of epic #498 land their regressions here.

#[path = "live/harness.rs"]
mod harness;

use std::time::{Duration, Instant};

use harness::{Bus, CPU, HEALTH, HEALTH_KEY, Run, SETTLE};
use serde_json::{Value, json};
use zenoh::sample::SampleKind;

/// The publisher's two keys, as the wire spells them.
fn cpu(bus: &Bus) -> String {
    bus.key(CPU)
}

fn health(bus: &Bus) -> String {
    bus.key(HEALTH_KEY)
}

/// Assert an exit code, printing the whole run when it is not the one.
#[track_caller]
fn exits(run: &Run, code: i32) {
    assert_eq!(run.code, code, "wrong exit code\n{run}");
}

// ── reads ───────────────────────────────────────────────────────────────
//
// v1's listings — `node list`, `topic list` — left with their nouns at FJ4
// (#612); zk2's `service`, `iface`, `graph` and `namespace` are pinned
// against live services in `tests/live_zk2.rs`.

/// `get` answers 0 with the stored value, resolved through the lens — a
/// foreign key in the namespace is `not_zk2`, rendered structurally and
/// never refused; a key nobody answers is silence, and silence is 2 —
/// never an empty 0 (RFC 05 §3.1).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_is_0_with_a_value_and_2_on_silence() {
    let bus = Bus::up().await;
    let key = health(&bus);
    let run = bus
        .until(&bus.ns(&["get", &key, "--format", "ndjson"]), |r| {
            r.code == 0
        })
        .await;
    exits(&run, 0);
    let answers = run.rows("answer");
    assert_eq!(answers.len(), 1, "{run}");
    assert_eq!(answers[0]["key"], json!(key));
    assert_eq!(answers[0]["identity"]["is"], json!("not_zk2"), "{run}");
    assert!(
        answers[0].get("origin").is_none(),
        "v1's origin left at FJ9"
    );
    assert_eq!(
        answers[0]["value"],
        serde_json::from_str::<Value>(HEALTH).unwrap()
    );

    // The bus answers (above); this key is simply unserved.
    let nothing = bus.key("plant/line-1/nothing");
    let run = bus
        .zenctl(&bus.ns(&["get", &nothing, "--timeout", "1", "--format", "ndjson"]))
        .await;
    exits(&run, 2);
    assert_eq!(run.ndjson()[0]["answers"], json!(0), "{run}");
    assert!(run.rows("answer").is_empty(), "{run}");
    assert!(run.stderr.contains("no replies"), "{run}");
}

/// `rate --for` measures the window it was given and counts what rode it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rate_counts_a_window() {
    let bus = Bus::up().await;
    let key = cpu(&bus);
    let run = bus
        .until(
            &bus.ns(&["rate", &key, "--for", "2", "--format", "json"]),
            |r| r.code == 0 && r.json()["total_count"].as_u64().is_some_and(|n| n > 0),
        )
        .await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["window_s"], json!(2.0), "{run}");
    assert_eq!(doc["keys"], json!(1), "{run}");
    assert!(doc["total_count"].as_u64().unwrap() > 0, "{run}");
}

// ── acts ────────────────────────────────────────────────────────────────

type Sub = zenoh::pubsub::Subscriber<zenoh::handlers::FifoChannelHandler<zenoh::sample::Sample>>;

/// Run an act until the subscriber hears what it sent. The act is rerun
/// only while nothing matching has arrived, because a fresh session's first
/// publication can leave before its route to the subscriber exists.
async fn act_until_heard(
    bus: &Bus,
    args: &[&str],
    sub: &Sub,
    matches: impl Fn(&zenoh::sample::Sample) -> bool,
) -> (Run, zenoh::sample::Sample) {
    let deadline = Instant::now() + SETTLE;
    loop {
        let run = bus.zenctl(args).await;
        let heard = tokio::time::timeout(Duration::from_secs(2), async {
            while let Ok(s) = sub.recv_async().await {
                if matches(&s) {
                    return Some(s);
                }
            }
            None
        })
        .await
        .ok()
        .flatten();
        if let Some(sample) = heard {
            return (run, sample);
        }
        assert!(
            Instant::now() < deadline,
            "the subscriber never heard it\n{run}"
        );
    }
}

/// `pub` goes out on a declared publisher, the bytes as typed, and a
/// subscriber receives it. stdout stays empty (#242).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pub_is_received_by_a_subscriber() {
    let bus = Bus::up().await;
    let key = health(&bus);
    let sub = bus.subscribe(&key).await;
    let body = r#"{"status":"degraded"}"#;
    let (run, sample) = act_until_heard(&bus, &["pub", &key, body], &sub, |s| {
        s.kind() == SampleKind::Put && s.payload().to_bytes().as_ref() == body.as_bytes()
    })
    .await;
    exits(&run, 0);
    assert!(run.stdout.is_empty(), "pub has no document (#242)\n{run}");
    assert_eq!(sample.key_expr().as_str(), key);
}

/// Everything `sub` hears within `window` whose payload is `body`.
async fn heard_body(sub: &Sub, body: &str, window: Duration) -> usize {
    let mut n = 0;
    let _ = tokio::time::timeout(window, async {
        while let Ok(s) = sub.recv_async().await {
            if s.payload().to_bytes().as_ref() == body.as_bytes() {
                n += 1;
            }
        }
    })
    .await;
    n
}

/// A wildcard put is refused — 2, before a session opens, whatever else the
/// flags say — and a subscriber on everything under it hears none of it
/// (#504).
/// The same subscriber hears a concrete `pub` first, so its silence is
/// about the refusal and not about a route that never formed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pub_refuses_a_wildcard_and_nothing_is_delivered() {
    let bus = Bus::up().await;
    let everything = bus.key("plant/**");
    let sub = bus.subscribe(&everything).await;
    let control = r#"{"status":"control"}"#;
    let (run, _) = act_until_heard(&bus, &["pub", &health(&bus), control], &sub, |s| {
        s.payload().to_bytes().as_ref() == control.as_bytes()
    })
    .await;
    exits(&run, 0);

    let wild = r#"{"status":"wild"}"#;
    for args in [
        vec!["pub", &everything, wild],
        vec!["pub", &everything, wild, "--qos", "data/block/reliable"],
    ] {
        let run = bus.zenctl(&args).await;
        exits(&run, 2);
        assert!(
            run.stderr.contains("is a wildcard") && run.stderr.contains("Not overridable"),
            "{run}"
        );
    }
    assert_eq!(
        heard_body(&sub, wild, Duration::from_secs(2)).await,
        0,
        "a refused put was delivered"
    );
}

/// A `pub --from ndjson` put row on a wildcard is refused and counted — the
/// pipe's 1 — while the concrete row beside it is published (#504).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pub_from_ndjson_refuses_a_wildcard_row() {
    let bus = Bus::up().await;
    let everything = bus.key("plant/**");
    let sub = bus.subscribe(&everything).await;
    let rows = format!(
        "{}\n{}\n",
        json!({ "key": everything, "value": "wild-row" }),
        json!({ "key": health(&bus), "value": "concrete-row" }),
    );
    let deadline = Instant::now() + SETTLE;
    let mut wild = 0;
    loop {
        let run = bus
            .zenctl_with_stdin(&["pub", "--from", "ndjson"], &rows)
            .await;
        exits(&run, 1);
        assert!(run.stderr.contains("1 refused row(s)"), "{run}");
        assert!(run.stderr.contains("is a wildcard"), "{run}");
        let mut concrete = false;
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            while let Ok(s) = sub.recv_async().await {
                match s.payload().to_bytes().as_ref() {
                    b"wild-row" => wild += 1,
                    b"concrete-row" => concrete = true,
                    _ => {}
                }
            }
        })
        .await;
        if concrete {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the concrete row never arrived\n{run}"
        );
    }
    assert_eq!(wild, 0, "a refused row was delivered");
}

// ── verdicts ────────────────────────────────────────────────────────────

/// `watchdog --count` exits on how its rules ended (#511): a `rate-above`
/// under the 20 Hz telemetry is still firing at the last tick — 1; one far
/// above it ends ok — 0; a silence claim longer than the run could watch
/// ends unobservable — 2.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watchdog_count_exits_on_how_its_rules_ended() {
    let bus = Bus::up().await;
    let key = cpu(&bus);
    let bounded = |rule: String| {
        vec![
            "watchdog".to_string(),
            "--rule".into(),
            rule,
            "--every".into(),
            "1".into(),
            "--count".into(),
            "2".into(),
            "--namespace".into(),
            bus.base.clone(),
        ]
    };

    let firing = bounded(format!("rate-above {key} 5"));
    let firing: Vec<&str> = firing.iter().map(String::as_str).collect();
    let run = bus.until(&firing, |r| r.code == 1).await;
    exits(&run, 1);
    assert_eq!(
        run.rows("transition").last().map(|t| t["to"].clone()),
        Some(json!("firing")),
        "{run}"
    );
    assert!(run.stderr.contains("1 rule(s) ended firing"), "{run}");

    let ok = bounded(format!("rate-above {key} 1000"));
    let ok: Vec<&str> = ok.iter().map(String::as_str).collect();
    let run = bus.until(&ok, |r| r.code == 0).await;
    exits(&run, 0);

    let quiet = bus.key("plant/line-1/nothere");
    let blind = bounded(format!("silent-for {quiet} 600"));
    let blind: Vec<&str> = blind.iter().map(String::as_str).collect();
    let run = bus.zenctl(&blind).await;
    exits(&run, 2);
    assert!(run.stderr.contains("ended unobservable"), "{run}");
}
