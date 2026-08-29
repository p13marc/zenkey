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
use zenkey_fleet::Sipper as _;
use zenkey_fleet::declare_publication;
use zenkey_fleet::judge::condition::{Condition, WatchdogSpec, watchdog};
use zenkey_fleet::report::{CondState, Transition, WatchdogSummary};

mod util;
use util::peer_pair;

const KEY: &str = "v1/h-dddddddddddd/state/demo/health";

fn store_of() -> zenkey_fleet::model::decode::SchemaStore {
    zenkey_fleet::model::decode::SchemaStore::new("", Duration::from_millis(300))
}

/// Drain a whole run: every transition, then the summary (#397).
///
/// The three tests below used to hand `run_watchdog` an `emit` closure over
/// an `mpsc::Sender` and read the channel after the join. That was the shape
/// the callback forced — and the shape that had no way to report a failing
/// emission. Sipping is the same drive without the channel, and the summary
/// comes out of the `await` that also performs the monitor teardown.
async fn drain(
    session: &zenoh::Session,
    slices: &zenkey_fleet::SliceSet,
    spec: &WatchdogSpec,
) -> zenkey_fleet::Result<(Vec<Transition>, WatchdogSummary)> {
    let fleet = zenkey_fleet::Fleet::new(session, "");
    let store = store_of();
    let mut run = watchdog(&fleet, Some(slices), &store, spec).pin();
    let mut transitions = Vec::new();
    while let Some(t) = run.sip().await {
        transitions.push(t);
    }
    run.await.map(|summary| (transitions, summary))
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
            drain(&b, &slices, &spec).await
        }
    });

    // The watchdog's subscriber raises the badge; publish mid-run, between
    // the second and third tick, so the silence has fired before the sample
    // breaks it and re-accumulates after.
    assert!(
        tokio::time::timeout(util::SETTLE, matching.recv())
            .await
            .expect("matching within 5s")
            .expect("listener alive")
    );
    tokio::time::sleep(Duration::from_millis(1050)).await;
    publication.send(b"{}".to_vec(), None).await.expect("send");

    let (transitions, summary) = watchdog.await.expect("join").expect("run");
    assert_eq!(summary.ticks, 6);

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

    let spec = WatchdogSpec {
        rules: vec![Condition::parse("origin-down h-000000000000").expect("rule")],
        tick: Duration::from_millis(200),
        ticks: Some(3),
        timeout: Duration::from_millis(300),
    };
    let (transitions, summary) = drain(&b, &slices, &spec).await.expect("run");
    assert_eq!(summary.ticks, 3);
    assert_eq!(transitions.len(), 1, "{transitions:#?}");
    assert_eq!(transitions[0].to, CondState::Firing);
    assert!(
        transitions[0].evidence.contains("no alive token"),
        "{}",
        transitions[0].evidence
    );
}

/// #338: the sweep no longer gates the sampling it is judging.
///
/// An `origin-down` rule makes every tick ask the roster, and a roster sweep
/// on this fixture takes the better part of a second. That sweep used to run
/// *after* the drain loop broke, so for its whole duration nobody attended
/// the monitor's 1024-slot broadcast — and `dropped_tick` was reset
/// immediately afterwards, so the samples lost to it were billed to the
/// *following* window. In the one tool whose entire product is a per-window
/// verdict.
///
/// The bus here carries ~5 000 samples a second, so a sweep-shaped gap of
/// even a quarter-second overflows the broadcast several times over. With
/// the sweep running beside the drain, the `dropped` rule stays `ok` for the
/// whole run: one baseline line, no firing.
///
/// Measured while the fix was written: with the sweep after the drain, the
/// rule changed state three times in three ticks — every one of those drops
/// the observer's own, and every one billed to the window after the one
/// that lost them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sweep_does_not_stop_the_sampling_it_judges() {
    let (a, b) = peer_pair().await;
    // An  queryable that never answers, so the doctor sweep
    // really costs its timeout — a fleet that answers nothing at all ends
    // the query at once and would gate nothing.
    let stuck: std::sync::Arc<std::sync::Mutex<Vec<zenoh::query::Query>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let _introspect = a
        .declare_queryable("v1/h-dddddddddddd/@rpc/demo/introspect")
        .callback({
            let stuck = std::sync::Arc::clone(&stuck);
            move |q| stuck.lock().expect("stuck lock").push(q)
        })
        .await
        .expect("introspect queryable");

    let publication = declare_publication(&a, KEY, QosProfile::Transition, None)
        .await
        .expect("declare");
    let matching = publication.matching_events().await.expect("events");

    let watchdog = tokio::spawn({
        let b = b.clone();
        async move {
            let slices = zenkey_fleet::SliceSet::default();
            let spec = WatchdogSpec {
                rules: vec![
                    Condition::parse("dropped").expect("rule"),
                    // The watch: without a selector-bearing rule the
                    // watchdog subscribes to nothing and there is no drain
                    // to gate.
                    Condition::parse(&format!("qos-mismatch {KEY}")).expect("rule"),
                    // The sweep: a whole doctor run per tick.
                    Condition::parse("doctor slice-sync").expect("rule"),
                ],
                tick: Duration::from_millis(300),
                ticks: Some(3),
                timeout: Duration::from_millis(500),
            };
            drain(&b, &slices, &spec).await
        }
    });

    assert!(
        tokio::time::timeout(util::SETTLE, matching.recv())
            .await
            .expect("matching within 5s")
            .expect("listener alive")
    );
    let flood = tokio::spawn(async move {
        loop {
            for _ in 0..100 {
                if publication.send(b"{}".to_vec(), None).await.is_err() {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });

    let (transitions, summary) = watchdog.await.expect("join").expect("run");
    flood.abort();
    assert_eq!(summary.ticks, 3);
    let dropped: Vec<&Transition> = transitions.iter().filter(|t| t.rule == "dropped").collect();
    assert_eq!(
        dropped.len(),
        1,
        "the drop rule changed state, so the sweep cost the window samples: {transitions:#?}"
    );
    assert_eq!(
        dropped[0].to,
        CondState::Ok,
        "baseline clean, and it stayed clean: {}",
        dropped[0].evidence
    );
}

/// A consumer that gives up mid-run still gets the summary, and the monitor
/// is still torn down (#397).
///
/// This is the coverage #360 was assumed to have and did not. That bug was a
/// verb finishing clean having emitted nothing, because the engine's `emit`
/// could not fail and `zenctl` had to stash the first write error and answer
/// for it after the run. There was nothing to test, because there was no way
/// to *stop*: the callback ran to the end whatever the caller thought of it.
///
/// Now stopping is just not sipping. Awaiting the run drains and discards
/// what is left, performs the acknowledged teardown (#207/#336) and yields
/// the summary — so a caller whose write failed returns its own error from
/// where it happened and still leaves nothing half torn down.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_consumer_that_stops_sipping_still_gets_the_summary_and_the_teardown() {
    let (_a, b) = peer_pair().await;
    let slices = zenkey_fleet::SliceSet::default();
    let spec = WatchdogSpec {
        rules: vec![Condition::parse("origin-down h-000000000000").expect("rule")],
        tick: Duration::from_millis(100),
        ticks: Some(4),
        timeout: Duration::from_millis(200),
    };

    let fleet = zenkey_fleet::Fleet::new(&b, "");
    let store = store_of();
    let mut run = watchdog(&fleet, Some(&slices), &store, &spec).pin();

    // One transition, then the consumer decides it has had enough — the
    // shape of `zenctl watchdog` hitting a write error on its first line.
    let first = run.sip().await.expect("the baseline is said out loud");
    assert_eq!(first.to, CondState::Firing);

    let summary = tokio::time::timeout(util::SETTLE, run)
        .await
        .expect("the run must finish rather than block on an undrained consumer")
        .expect("run");
    assert_eq!(
        summary.ticks, 4,
        "the run completes its bound; giving up reading is not stopping it"
    );
    assert!(
        summary.transitions >= 1,
        "and the summary counts what it produced, read or not: {summary:?}"
    );
}
