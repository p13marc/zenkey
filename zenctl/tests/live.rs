//! The live suite: the real `zenctl` binary against a real bus (#499, #500).
//!
//! `tests/cli.rs` pins what zenctl says with no bus; the render snapshots pin
//! how a report draws; zenkey-fleet's suites pin the engine. None of them ran
//! what zenctl adds on top — `Bus::resolve`, the argument → `GetOpts`/
//! `CallSpec` mapping, the guards in `cmd/*.rs`, the rendering of *real*
//! replies, and every 0 and 1 a verdict verb can exit with. Every case here
//! does: an in-process producer on an ephemeral port (`live/harness.rs`),
//! the binary run as a script would run it, and its stdout, stderr and exit
//! code asserted against the contract in `zenctl/src/exit.rs`.
//!
//! The config cases (#500) run against the RFC 05 §5.1 test double in
//! `zenkey-fleet/tests/util/config_server.rs` — the same file the engine's
//! lifecycle test drives, included by path so the two suites cannot disagree
//! about what a configuration server answers.
//!
//! Later chunks of epic #498 land their regressions here.

#[path = "../../zenkey-fleet/tests/util/config_server.rs"]
mod config_server;
#[path = "live/harness.rs"]
mod harness;

use std::time::{Duration, Instant};

use harness::{Bus, Extras, HEALTH, HOST, PRODUCER, Run, SETTLE};
use serde_json::{Value, json};
use zenoh::sample::SampleKind;

/// The producer's two published keys, as the wire spells them.
fn cpu(bus: &Bus) -> String {
    bus.key(&format!("v1/{HOST}/telemetry/{PRODUCER}/cpu"))
}

fn health(bus: &Bus) -> String {
    bus.key(&format!("v1/{HOST}/state/{PRODUCER}/health"))
}

/// Assert an exit code, printing the whole run when it is not the one.
#[track_caller]
fn exits(run: &Run, code: i32) {
    assert_eq!(run.code, code, "wrong exit code\n{run}");
}

// ── listings ────────────────────────────────────────────────────────────

/// `node list` sees the producer by its `alive` token (RFC 04 §5).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn node_list_sees_the_producer() {
    let bus = Bus::up().await;
    let node = json!({ "row": "node", "origin": HOST, "producer": PRODUCER });
    let run = bus
        .until(&["node", "list", "--format", "ndjson"], |r| {
            r.code == 0 && r.ndjson().contains(&node)
        })
        .await;
    exits(&run, 0);
    assert_eq!(run.rows("node"), [node], "exactly the one producer\n{run}");
}

/// `topic list` reads the subjects off the served slice (RFC 08 §6) — no
/// `--registry` anywhere.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn topic_list_reads_the_served_slice() {
    let bus = Bus::up().await;
    let run = bus
        .until(&["topic", "list", "--format", "ndjson"], |r| {
            r.code == 0 && r.rows("subject").len() == 2
        })
        .await;
    exits(&run, 0);
    let subjects: Vec<(Value, Value, Value)> = run
        .rows("subject")
        .into_iter()
        .map(|r| {
            (
                r["path"].clone(),
                r["class"].clone(),
                r["type_name"].clone(),
            )
        })
        .collect();
    assert_eq!(
        subjects,
        [
            (json!("health"), json!("state"), json!("Health")),
            (json!("cpu"), json!("telemetry"), json!("Cpu")),
        ],
        "{run}"
    );
}

// ── reads ───────────────────────────────────────────────────────────────

/// `get` answers 0 with the stored value, decoded against the served schema;
/// a key nobody answers is silence, and silence is 2 — never an empty 0
/// (RFC 05 §3.1).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_is_0_with_a_value_and_2_on_silence() {
    let bus = Bus::up().await;
    let key = health(&bus);
    let run = bus
        .until(&["get", &key, "--format", "ndjson"], |r| r.code == 0)
        .await;
    exits(&run, 0);
    let answers = run.rows("answer");
    assert_eq!(answers.len(), 1, "{run}");
    assert_eq!(answers[0]["key"], json!(key));
    assert_eq!(answers[0]["origin"], json!(HOST));
    assert_eq!(answers[0]["type"], json!("Health"));
    assert_eq!(
        answers[0]["value"],
        serde_json::from_str::<Value>(HEALTH).unwrap()
    );

    // The bus answers (above); this key is simply unserved.
    let nothing = bus.key(&format!("v1/{HOST}/state/{PRODUCER}/nothing"));
    let run = bus
        .zenctl(&["get", &nothing, "--timeout", "1", "--format", "ndjson"])
        .await;
    exits(&run, 2);
    assert_eq!(run.ndjson()[0]["answers"], json!(0), "{run}");
    assert!(run.rows("answer").is_empty(), "{run}");
    assert!(run.stderr.contains("no replies"), "{run}");
}

/// `echo --count N` prints exactly N samples and exits 0.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn echo_stops_after_count() {
    let bus = Bus::up().await;
    let key = cpu(&bus);
    let run = bus
        .until(&["echo", &key, "--count", "3", "--format", "ndjson"], |r| {
            r.code == 0
        })
        .await;
    exits(&run, 0);
    let samples = run.ndjson();
    assert_eq!(samples.len(), 3, "{run}");
    for s in &samples {
        assert_eq!(s["key"], json!(key), "{run}");
        assert_eq!(s["type"], json!("Cpu"), "decoded against describe\n{run}");
        assert_eq!(s["delete"], json!(false), "{run}");
        assert!(s["value"]["value"].is_number(), "{run}");
    }
}

/// `rate --for` measures the window it was given and counts what rode it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rate_counts_a_window() {
    let bus = Bus::up().await;
    let key = cpu(&bus);
    let run = bus
        .until(&["rate", &key, "--for", "2", "--format", "json"], |r| {
            r.code == 0 && r.json()["total_count"].as_u64().is_some_and(|n| n > 0)
        })
        .await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(doc["window_s"], json!(2.0), "{run}");
    assert_eq!(doc["keys"], json!(1), "{run}");
    assert!(doc["total_count"].as_u64().unwrap() > 0, "{run}");
}

/// `service call` reaches a procedure the producer serves, typed by the
/// served slice, and prints its reply.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn service_call_reaches_a_served_procedure() {
    let bus = Bus::up().await;
    let run = bus
        .until(
            &[
                "service", "call", HOST, PRODUCER, "ping", "--format", "json",
            ],
            |r| r.code == 0,
        )
        .await;
    exits(&run, 0);
    let doc = run.json();
    assert_eq!(
        doc["key"],
        json!(bus.key(&format!("v1/{HOST}/@rpc/{PRODUCER}/ping")))
    );
    assert_eq!(
        doc["rows"],
        json!([{ "row": "answer", "origin": HOST, "ok": true, "value": { "pong": true } }]),
        "{run}"
    );
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

/// `pub` goes out on a declared publisher, encoded as the served slice
/// declares, and a subscriber receives it. stdout stays empty (#242).
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

/// `retire` sends a tombstone — a delete, not an empty put (RFC 04 §1.2).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retire_is_received_as_a_delete() {
    let bus = Bus::up().await;
    let key = bus.key(&format!("v1/{HOST}/state/{PRODUCER}/lease/a1"));
    let sub = bus.subscribe(&key).await;
    let (run, sample) = act_until_heard(&bus, &["retire", &key], &sub, |s| {
        s.kind() == SampleKind::Delete
    })
    .await;
    exits(&run, 0);
    assert!(
        run.stdout.is_empty(),
        "retire has no document (#242)\n{run}"
    );
    assert!(run.stderr.contains(&format!("retired {key}")), "{run}");
    assert_eq!(sample.key_expr().as_str(), key);
}

/// A wildcard tombstone is refused outright — 2, this tool refusing its
/// input, and `--i-know` does not move it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retire_refuses_a_wildcard() {
    let bus = Bus::up().await;
    let wild = bus.key(&format!("v1/{HOST}/state/{PRODUCER}/*"));
    for args in [vec!["retire", &wild], vec!["retire", &wild, "--i-know"]] {
        let run = bus.zenctl(&args).await;
        exits(&run, 2);
        assert!(run.stdout.is_empty(), "{run}");
        assert!(
            run.stderr.contains("is a wildcard") && run.stderr.contains("Not overridable"),
            "{run}"
        );
    }
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

/// A wildcard put is refused — 2, before a session opens, `--raw` or not —
/// and a subscriber on everything under the base hears none of it (#504).
/// The same subscriber hears a concrete `pub` first, so its silence is
/// about the refusal and not about a route that never formed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pub_refuses_a_wildcard_and_nothing_is_delivered() {
    let bus = Bus::up().await;
    let everything = bus.key("v1/**");
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
        vec!["pub", &everything, wild, "--raw"],
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
    let everything = bus.key("v1/**");
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

/// `check expect`: 0 when the expectation is met, 1 when a clean
/// observation does not meet it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn check_expect_is_0_when_met_and_1_when_not() {
    let bus = Bus::up().await;
    let key = cpu(&bus);

    let met = bus
        .until(
            &[
                "check",
                "expect",
                &key,
                "--for",
                "10",
                "--at-least",
                "3",
                "--format",
                "json",
            ],
            |r| r.code == 0,
        )
        .await;
    exits(&met, 0);
    assert_eq!(met.json()["verdict"], json!("met"), "{met}");
    assert_eq!(met.json()["samples"], json!(3), "{met}");

    // Twenty a second against a one-a-second ceiling: not met, on samples
    // that did arrive — the 1 is about the rate, not about silence.
    let unmet = bus
        .until(
            &[
                "check",
                "expect",
                &key,
                "--for",
                "2",
                "--rate-max",
                "1",
                "--format",
                "json",
            ],
            |r| r.code == 1 && r.json()["samples"].as_u64().is_some_and(|n| n > 2),
        )
        .await;
    exits(&unmet, 1);
    let doc = unmet.json();
    assert_eq!(doc["verdict"], json!("not_met"), "{unmet}");
    assert!(doc["unmet"].to_string().contains("ceiling"), "{unmet}");
}

/// `why` exits 1 when it establishes a cause: a subject the producer's own
/// served slice does not declare (RFC 08 §2).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn why_is_1_when_a_cause_is_established() {
    let bus = Bus::up().await;
    let key = bus.key(&format!("v1/{HOST}/telemetry/{PRODUCER}/nothere"));
    let run = bus
        .until(&["why", &key, "--format", "ndjson"], |r| {
            r.code == 1 && r.ndjson()[0]["causes"] == json!(["registry-declared"])
        })
        .await;
    exits(&run, 1);
    let head = &run.ndjson()[0];
    assert_eq!(head["verdict"], json!("explained"), "{run}");
    assert_eq!(head["causes"], json!(["registry-declared"]), "{run}");
    let alive = run
        .rows("rung")
        .into_iter()
        .find(|r| r["id"] == "origin-alive")
        .expect("the roster rung");
    assert_eq!(alive["answer"], json!("established"), "{run}");
}

/// `doctor --fail-on error` is 0 on a producer that keeps its contracts.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn doctor_fail_on_error_is_0_on_a_healthy_producer() {
    let bus = Bus::up().await;
    let run = bus
        .until(
            &["doctor", "--fail-on", "error", "--format", "ndjson"],
            |r| {
                let head = &r.ndjson()[0];
                r.code == 0 && head["live_producers"] == 1 && head["introspect_answered"] == 1
            },
        )
        .await;
    exits(&run, 0);
    let head = &run.ndjson()[0];
    assert_eq!(head["describe_served"], json!(1), "{run}");
    assert!(
        run.rows("finding").iter().all(|f| f["severity"] != "error"),
        "{run}"
    );
}

/// … and 1 when a producer holds `alive` and answers nothing — alive ⇒
/// callable (RFC 04 §5) broken, filed as `introspect-coverage`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn doctor_fail_on_error_is_1_on_a_mute_producer() {
    let bus = Bus::with(Extras { mute: true }).await;
    let run = bus
        .until(
            &["doctor", "--fail-on", "error", "--format", "ndjson"],
            |r| {
                let head = &r.ndjson()[0];
                r.code == 1 && head["live_producers"] == 2 && head["introspect_answered"] == 1
            },
        )
        .await;
    exits(&run, 1);
    let errors: Vec<Value> = run
        .rows("finding")
        .into_iter()
        .filter(|f| f["severity"] == "error")
        .collect();
    assert!(
        !errors.is_empty() && errors.iter().all(|f| f["check"] == "introspect-coverage"),
        "{run}"
    );
}

// ── config (RFC 05 §5.1, #500) ──────────────────────────────────────────

/// The `parameter` row for one name.
#[track_caller]
fn param(run: &Run, name: &str) -> Value {
    run.rows("parameter")
        .into_iter()
        .find(|r| r["name"] == name)
        .unwrap_or_else(|| panic!("no parameter {name:?}\n{run}"))
}

/// The `pending` row, if the document carries one.
fn pending(run: &Run) -> Option<Value> {
    run.rows("pending").into_iter().next()
}

const CONFIG_GET: [&str; 7] = [
    "config", "get", HOST, PRODUCER, "wlan0", "--format", "ndjson",
];

/// `config get`, rerun until it answers.
async fn config_get(bus: &Bus) -> Run {
    let run = bus.until(&CONFIG_GET, |r| r.code == 0).await;
    exits(&run, 0);
    run
}

/// A write, rerun while it meets silence — and only then, which is why every
/// `set` here carries an idempotency key (RFC 05 §5.1: a lost reply must not
/// become a doubled write).
async fn config_write(bus: &Bus, args: &[&str]) -> Run {
    let mut argv = vec!["config"];
    argv.extend_from_slice(args);
    argv.extend(["--format", "ndjson"]);
    bus.until(&argv, |r| r.code != 2).await
}

/// A `set` of `queue` with a confirm window; the run and the pending
/// change's token.
async fn set_pending(bus: &Bus, value: &str, window: &str, key: &str) -> (Run, String) {
    let run = config_write(
        bus,
        &[
            "set",
            HOST,
            PRODUCER,
            "wlan0",
            "queue",
            value,
            "--confirm",
            window,
            "--idempotency-key",
            key,
        ],
    )
    .await;
    exits(&run, 0);
    let token = pending(&run)
        .and_then(|p| p["token"].as_str().map(str::to_string))
        .unwrap_or_else(|| panic!("a confirmed change is pending\n{run}"));
    (run, token)
}

/// `config get`: the served schema beside every running value, its class
/// and its source.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_get_reads_the_schema_beside_every_value() {
    let bus = Bus::up().await;
    let run = config_get(&bus).await;
    assert_eq!(run.rows("parameter").len(), 4, "{run}");
    let q = param(&run, "tx_queue_len");
    assert_eq!(
        [
            &q["group"],
            &q["class"],
            &q["kind"],
            &q["value"],
            &q["source"]
        ],
        [
            &json!("queue"),
            &json!("hot"),
            &json!("integer"),
            &json!(1000),
            &json!("file")
        ],
        "{run}"
    );
    assert_eq!(param(&run, "ssid")["class"], json!("reach"), "{run}");
    assert_eq!(param(&run, "mtu")["class"], json!("contract"), "{run}");
    assert!(pending(&run).is_none(), "{run}");
}

/// `set` with `--confirm` applies at once and leaves a change pending;
/// `confirm` makes it permanent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_set_is_pending_and_confirm_applies_it() {
    let bus = Bus::up().await;
    config_get(&bus).await;
    let (set, token) = set_pending(&bus, "tx_queue_len=2000", "600", "live-confirm").await;
    let q = param(&set, "tx_queue_len");
    assert_eq!(
        [&q["value"], &q["source"]],
        [&json!(2000), &json!("runtime")]
    );
    assert_eq!(pending(&set).unwrap()["groups"], json!(["queue"]), "{set}");

    let run = config_write(&bus, &["confirm", HOST, PRODUCER, "wlan0", &token]).await;
    exits(&run, 0);
    let after = config_get(&bus).await;
    assert!(
        pending(&after).is_none(),
        "confirmed is not pending\n{after}"
    );
    assert_eq!(
        param(&after, "tx_queue_len")["value"],
        json!(2000),
        "{after}"
    );
}

/// `set` then `cancel`: the change is undone now.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_cancel_reverts_a_pending_change() {
    let bus = Bus::up().await;
    config_get(&bus).await;
    let (set, token) = set_pending(&bus, "fq=true", "600", "live-cancel").await;
    assert_eq!(param(&set, "fq")["value"], json!(true), "{set}");

    let run = config_write(&bus, &["cancel", HOST, PRODUCER, "wlan0", &token]).await;
    exits(&run, 0);
    let after = config_get(&bus).await;
    assert!(pending(&after).is_none(), "{after}");
    let fq = param(&after, "fq");
    assert_eq!(
        [&fq["value"], &fq["source"]],
        [&json!(false), &json!("file")]
    );
}

/// `extend` moves a pending change's deadline.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_extend_moves_the_deadline() {
    let bus = Bus::up().await;
    config_get(&bus).await;
    let (set, token) = set_pending(&bus, "tx_queue_len=1500", "60", "live-extend").await;
    let before = pending(&set).unwrap()["deadline"].clone();

    let run = config_write(
        &bus,
        &["extend", HOST, PRODUCER, "wlan0", &token, "--by", "3600"],
    )
    .await;
    exits(&run, 0);
    let after = config_get(&bus).await;
    let moved = pending(&after).expect("still pending")["deadline"].clone();
    // RFC 3339 in one zone and one precision orders as text.
    assert!(
        moved.as_str() > before.as_str(),
        "extend must move the deadline: {before} → {moved}\n{after}"
    );
}

/// An unconfirmed change lapses at its deadline: rolled back, nothing
/// pending.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_an_unconfirmed_change_lapses() {
    let bus = Bus::up().await;
    config_get(&bus).await;
    let (set, _) = set_pending(&bus, "tx_queue_len=4000", "1", "live-lapse").await;
    assert_eq!(param(&set, "tx_queue_len")["value"], json!(4000), "{set}");

    let lapsed = bus
        .until(&CONFIG_GET, |r| r.code == 0 && pending(r).is_none())
        .await;
    exits(&lapsed, 0);
    assert!(
        pending(&lapsed).is_none(),
        "the change never lapsed\n{lapsed}"
    );
    let q = param(&lapsed, "tx_queue_len");
    assert_eq!([&q["value"], &q["source"]], [&json!(1000), &json!("file")]);
}

/// `persist` writes a confirmed change into the persisted layer — the
/// read-back's source says so.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_persist_writes_the_persisted_layer() {
    let bus = Bus::up().await;
    config_get(&bus).await;
    let (_, token) = set_pending(&bus, "tx_queue_len=2500", "600", "live-persist").await;
    let confirmed = config_write(&bus, &["confirm", HOST, PRODUCER, "wlan0", &token]).await;
    exits(&confirmed, 0);
    let run = config_write(&bus, &["persist", HOST, PRODUCER, "wlan0", &token]).await;
    exits(&run, 0);
    let after = config_get(&bus).await;
    let q = param(&after, "tx_queue_len");
    assert_eq!(
        [&q["value"], &q["source"]],
        [&json!(2500), &json!("overlay")]
    );
}

/// How many times the double has received a `set` of one group.
fn sets_of(bus: &Bus, group: &str) -> usize {
    let suffix = format!("/{group}/set");
    bus.config
        .received()
        .iter()
        .filter(|p| p.ends_with(&suffix))
        .count()
}

/// A `config set` this tool must refuse before sending: rerun only while the
/// refusal is not the one named by `needle` — a run whose read-back met
/// silence has no class to refuse on, and says so instead — and assert that
/// the refusing run itself sent nothing. The count is taken around that one
/// run, so an earlier run that did reach the producer cannot be mistaken for
/// it.
async fn refused_unsent(bus: &Bus, group: &str, value: &str, needle: &str) -> Run {
    let args = [
        "config",
        "set",
        HOST,
        PRODUCER,
        "wlan0",
        group,
        value,
        "--confirm",
        "60",
    ];
    let deadline = Instant::now() + SETTLE;
    loop {
        let before = sets_of(bus, group);
        let run = bus.zenctl(&args).await;
        if run.code == 2 && run.stderr.contains(needle) {
            assert_eq!(sets_of(bus, group), before, "refused, yet sent\n{run}");
            return run;
        }
        assert!(
            Instant::now() < deadline,
            "never refused at the edge\n{run}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A reach `set` from a script (stdin not a terminal) without `--yes` is
/// refused here — 2, before anything is sent — and with `--yes` it goes out
/// and is answered `{token, apply_at}` (RFC 05 §5.1).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_a_reach_set_needs_yes_from_a_script() {
    let bus = Bus::up().await;
    config_get(&bus).await;
    let refused = refused_unsent(&bus, "link", "ssid=field", "is reach").await;
    assert!(refused.stdout.is_empty(), "{refused}");
    assert!(refused.stderr.contains("Pass --yes"), "{refused}");

    let run = config_write(
        &bus,
        &[
            "set",
            HOST,
            PRODUCER,
            "wlan0",
            "link",
            "ssid=field",
            "--confirm",
            "60",
            "--yes",
            "--idempotency-key",
            "live-reach",
        ],
    )
    .await;
    exits(&run, 0);
    let reply = &run.rows("answer")[0];
    assert!(
        reply["value"]["token"].is_string() && reply["value"]["apply_at"].is_string(),
        "a reach set answers {{token, apply_at}}, not the read-back\n{run}"
    );
}

/// A `contract` group is refused at the edge in the producer's own words
/// (`ConfigSchema::validate`) — 2, and nothing sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_a_contract_set_is_refused_before_it_is_sent() {
    let bus = Bus::up().await;
    config_get(&bus).await;
    let run = refused_unsent(&bus, "transport", "mtu=9000", "startup configuration").await;
    assert!(run.stderr.contains("restart"), "{run}");
}

