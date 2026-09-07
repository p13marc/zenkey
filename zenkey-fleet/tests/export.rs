//! The exporter's ledger on a live bus (#228, RFC 13 §3): a producer
//! publishes, its series is live; it retires, and the series goes
//! `origin_down` by name rather than flatlining. The HTTP surface is the
//! frontend's; what is proven here is the fold — the same one `zenctl
//! export` serves on every scrape.
//!
//! Ports are ephemeral (`util::peer_pair`), so two test runs at once cannot
//! collide.

use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

use zenkey::qos::QosProfile;
use zenkey_fleet::report::SeriesState;
use zenkey_fleet::{
    BringUp, ExportLedger, FleetEvent, FoldInputs, Observed, PayloadVerdict, RosterWatch,
    StreamItem, describe_key, structural_value,
};

mod util;
use util::peer_pair;

const ORIGIN: &str = "h-dddddddddddd";
const KEY: &str = "v1/h-dddddddddddd/telemetry/demo/cpu/usage";
const ALIVE: &str = "v1/h-dddddddddddd/state/demo/alive";

fn slices() -> zenkey_fleet::SliceSet {
    zenkey_fleet::SliceSet::from_slices(vec![
        zenkey::parse_slice(
            r#"
[registry]
version = "1.0"
app = "t"
convention = 1
[producer]
name = "demo"
[[subject]]
path = "cpu/usage"
class = "telemetry"
type = "TelemetryPoint"
unit = "percent"
kind = "gauge"
"#,
        )
        .expect("fixture parses"),
    ])
}

/// The roster's departures since the last look: what the frontend feeds
/// the fold as `down`.
fn departed(
    before: &std::collections::BTreeMap<String, Vec<String>>,
    after: &std::collections::BTreeMap<String, Vec<String>>,
) -> BTreeSet<(String, String)> {
    let mut gone = BTreeSet::new();
    for (origin, producers) in before {
        for p in producers {
            if !after.get(origin).is_some_and(|ps| ps.contains(p)) {
                gone.insert((origin.clone(), p.clone()));
            }
        }
    }
    gone
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_retired_producer_turns_its_series_origin_down_rather_than_flat() {
    let (a, b) = peer_pair().await;
    let slices = slices();

    // The observer: a monitor over the data plane, a roster over presence.
    let monitor = zenkey_fleet::Monitor::start(&b, zenkey_fleet::MonitorSpec::default())
        .await
        .expect("monitor");
    let mut events = monitor.events();
    monitor.watch("v1/**").await.expect("watch");
    let fleet = zenkey_fleet::Fleet::new(&b, "");
    let mut roster = RosterWatch::start(&fleet, Duration::from_millis(500))
        .await
        .expect("roster");

    // The producer: alive, then publishing.
    let producer = BringUp::new(&a).alive(ALIVE).await.expect("alive");
    let publication = zenkey_fleet::declare_publication(&a, KEY, QosProfile::Sampled, None)
        .await
        .expect("declare");
    let matching = publication.matching_events().await.expect("events");
    assert!(
        tokio::time::timeout(util::SETTLE, matching.recv())
            .await
            .expect("matching")
            .expect("listener alive")
    );
    // Presence has to be seen before it can be seen leaving.
    let mut seen = roster.roster().clone();
    while !seen
        .get(ORIGIN)
        .is_some_and(|ps| ps.iter().any(|p| p == "demo"))
    {
        tokio::time::timeout(util::SETTLE, roster.next_change())
            .await
            .expect("presence within the net")
            .expect("roster alive");
        seen = roster.roster().clone();
    }
    publication
        .send(b"{\"type\":\"gauge\",\"value\":12.5}".to_vec(), None)
        .await
        .expect("put");

    let mut ledger = ExportLedger::new(64, vec!["v1/**".into()], Some(1), SystemTime::now());
    let view = loop {
        match tokio::time::timeout(util::SETTLE, events.recv())
            .await
            .expect("sample within the net")
        {
            Some(StreamItem::Event(FleetEvent::Sample(v))) if v.key == KEY => break v,
            Some(_) => continue,
            None => panic!("monitor closed"),
        }
    };
    let bytes = view.payload.to_bytes();
    let doc = structural_value(&bytes);
    let facts = describe_key("", &view.key, Some(&slices)).facts;
    ledger.ingest(
        &Observed {
            key: &view.key,
            delete: false,
            doc: doc.as_ref(),
            qos_matches: None,
            verdict: PayloadVerdict::NotValidated,
            wall_unix_s: 1,
        },
        &facts,
    );

    let fold = |ledger: &mut ExportLedger, down: &BTreeSet<(String, String)>| {
        let core = monitor.core();
        core.with_stats(|stats| {
            ledger.fold(&FoldInputs {
                stats,
                retention: core.retention(),
                dropped: core.dropped(),
                down,
                doctor: None,
                now: SystemTime::now(),
            })
        })
    };
    let live = fold(&mut ledger, &BTreeSet::new());
    let row = &live.series[0];
    assert_eq!(row.name, "zenkey_subject_demo_cpu_usage_percent");
    assert_eq!(row.state, SeriesState::Live);
    assert_eq!(row.value, Some(12.5));
    assert_eq!(live.scopes, ["v1/**"]);
    assert!(
        live.excluded.iter().any(|e| e == "service origins"),
        "a wildcard scope names what it cannot reach: {:?}",
        live.excluded
    );

    // The producer retires: the token goes first (RFC 04 §5), the roster
    // sees it leave, and the fold names the departure.
    producer.retire().await.expect("retire");
    let mut down = BTreeSet::new();
    while !down.contains(&(ORIGIN.to_string(), "demo".to_string())) {
        tokio::time::timeout(util::SETTLE, roster.next_change())
            .await
            .expect("departure within the net")
            .expect("roster alive");
        down = departed(&seen, roster.roster());
    }
    let stale = fold(&mut ledger, &down);
    let row = &stale.series[0];
    assert_eq!(row.state, SeriesState::OriginDown);
    assert_eq!(
        row.value, None,
        "no value line — absence is named, not silent"
    );
    assert_eq!(row.origin, ORIGIN, "the labels survive the departure");
    let text = zenkey_fleet::exposition(&stale);
    assert!(text.contains("state=\"origin_down\"} 1"), "{text}");
    assert!(
        !text.contains("zenkey_subject_demo_cpu_usage_percent{"),
        "{text}"
    );

    // Two scrapes with nothing between them are the same bytes.
    let again = zenkey_fleet::exposition(&fold(&mut ledger, &down));
    assert_eq!(text, again);

    roster.stop().await.expect("roster stop");
    monitor.shutdown().await.expect("monitor shutdown");
}
