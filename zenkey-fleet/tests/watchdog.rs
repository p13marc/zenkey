//! `run_watchdog` (#227): transitions, not states, live on a bus.
//!
//! The acceptance case: a run over a staged fixture emits exactly one
//! transition per genuine state change and **none** per unchanged tick.
//! Event-driven settle as in `tests/expect.rs`: the publisher's matching
//! badge proves the watchdog's subscriber is routable before the clock that
//! the assertions depend on starts mattering.
//! Ports are ephemeral (`util::peer_pair`), so two test runs at once
//! cannot collide.

use std::time::Duration;

use zenkey::qos::QosProfile;
use zenkey_fleet::condition::{CondState, Condition, Transition, WatchdogSpec, run_watchdog};
use zenkey_fleet::declare_publication;

mod util;
use util::peer_pair;

const KEY: &str = "v1/h-dddddddddddd/state/demo/health";

fn store_of() -> zenkey_fleet::model::decode::SchemaStore {
    zenkey_fleet::model::decode::SchemaStore::new("", Duration::from_millis(300))
}

/// The staged fixture: a `silent-for` rule over a key that is quiet, then
/// speaks once, then goes quiet again — plus a `dropped` rule that never
/// changes. Expected transitions, and nothing else:
///
/// 1. `silent-for`: baseline → `unobservable` (the watch is younger than the
///    claimed span — not asked is not answered, O4);
/// 2. `silent-for`: → `firing` once a drop-free 0.7s of silence was watched;
/// 3. `silent-for`: → `ok` when the sample rides;
/// 4. `silent-for`: → `firing` again once the silence re-accumulates;
/// 5. `dropped`: baseline → `ok`, exactly once — six ticks, one line.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_watchdog_emits_one_transition_per_genuine_change_and_none_per_tick() {
    let (a, b) = peer_pair().await;

    let publication = declare_publication(&a, KEY, QosProfile::Transition, None)
        .await
        .expect("declare");
    let matching = publication.matching_events().await.expect("events");

    let (tx, rx) = std::sync::mpsc::channel::<Transition>();
    let watchdog = tokio::spawn({
        let b = b.clone();
        async move {
            let slices = zenkey_fleet::SliceSet::default();
            let spec = WatchdogSpec {
                rules: vec![
                    Condition::parse(&format!("silent-for {KEY} 0.7")).expect("rule"),
                    Condition::parse("dropped").expect("rule"),
                ],
                tick: Duration::from_millis(500),
                ticks: Some(6),
                timeout: Duration::from_millis(300),
            };
            let mut emit = move |t: &Transition| {
                let _ = tx.send(t.clone());
            };
            run_watchdog(
                &zenkey_fleet::Fleet::new(&b, ""),
                Some(&slices),
                &store_of(),
                &spec,
                &mut emit,
            )
            .await
        }
    });

    // The watchdog's subscriber raises the badge; publish mid-run, between
    // the second and third tick, so the silence has fired before the sample
    // breaks it and re-accumulates after.
    assert!(
        tokio::time::timeout(Duration::from_secs(5), matching.recv())
            .await
            .expect("matching within 5s")
            .expect("listener alive")
    );
    tokio::time::sleep(Duration::from_millis(1050)).await;
    publication.send(b"{}".to_vec(), None).await.expect("send");

    let summary = watchdog.await.expect("join").expect("run");
    assert_eq!(summary.ticks, 6);

    let transitions: Vec<Transition> = rx.try_iter().collect();
    let silence: Vec<&Transition> = transitions
        .iter()
        .filter(|t| t.rule.starts_with("silent-for"))
        .collect();
    let states: Vec<CondState> = silence.iter().map(|t| t.to).collect();
    assert_eq!(
        states,
        [
            CondState::Unobservable,
            CondState::Firing,
            CondState::Ok,
            CondState::Firing,
        ],
        "one transition per genuine change: {transitions:#?}"
    );
    assert_eq!(silence[0].from, None, "the baseline comes from null");
    assert_eq!(silence[1].from, Some(CondState::Unobservable));

    // The unchanged rule spoke exactly once, at baseline — six ticks, one
    // line. This is the acceptance clause: no line per unchanged tick.
    let dropped: Vec<&Transition> = transitions.iter().filter(|t| t.rule == "dropped").collect();
    assert_eq!(dropped.len(), 1, "{transitions:#?}");
    assert_eq!(dropped[0].from, None);
    assert_eq!(dropped[0].to, CondState::Ok);
    assert_eq!(
        summary.transitions as usize,
        transitions.len(),
        "the summary counts what was emitted"
    );
    // The per-key facts cache is bounded (#107) and its cost is a summary
    // fact (O6): this fixture's key set fits, so the ledger reads zero.
    assert_eq!(summary.facts_evicted, 0);
}

/// `origin-down` judges the roster: an origin holding no alive token is a
/// baseline `firing` — and stays one line across the run.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn origin_down_fires_on_an_absent_origin_and_only_once() {
    let (_a, b) = peer_pair().await;
    let slices = zenkey_fleet::SliceSet::default();

    let (tx, rx) = std::sync::mpsc::channel::<Transition>();
    let spec = WatchdogSpec {
        rules: vec![Condition::parse("origin-down h-000000000000").expect("rule")],
        tick: Duration::from_millis(200),
        ticks: Some(3),
        timeout: Duration::from_millis(300),
    };
    let mut emit = move |t: &Transition| {
        let _ = tx.send(t.clone());
    };
    let summary = run_watchdog(
        &zenkey_fleet::Fleet::new(&b, ""),
        Some(&slices),
        &store_of(),
        &spec,
        &mut emit,
    )
    .await
    .expect("run");
    assert_eq!(summary.ticks, 3);
    let transitions: Vec<Transition> = rx.try_iter().collect();
    assert_eq!(transitions.len(), 1, "{transitions:#?}");
    assert_eq!(transitions[0].to, CondState::Firing);
    assert!(
        transitions[0].evidence.contains("no alive token"),
        "{}",
        transitions[0].evidence
    );
}
