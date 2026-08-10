//! End-to-end checks against a real bus. Ignored by default — they need the
//! `spray` example publishing. No router: `spray` listens and these connect
//! straight to it.
//!
//! ```text
//! just spray                 # or: cargo run -p zengui --example spray -- -l tcp/127.0.0.1:7449
//! just test-live             # or: cargo test -p zengui --test live_bus -- --ignored --nocapture
//! ```
//!
//! These exercise everything under the widgets: scope selectors, the monitor,
//! the tree snapshot, the key projection and the registry overlay. The Iced
//! layer is covered by unit tests; this is the part that can only be trusted
//! against real traffic.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use zengui::keyfacts::{KeyFacts, KeyShape, Registration};
use zengui::scope::{self, ScopePreset};
use zengui::view::tree;
use zenkey_fleet::{Monitor, MonitorSpec, SliceSet};

/// Where the traffic is. `just gui-demo` uses 7449; override with
/// `ZENGUI_TEST_ENDPOINT` to point these at an existing bus instead.
fn endpoint() -> String {
    std::env::var("ZENGUI_TEST_ENDPOINT").unwrap_or_else(|_| "tcp/127.0.0.1:7449".to_string())
}
const SETTLE: Duration = Duration::from_secs(3);

async fn watch(selectors: Vec<String>) -> (zenkey_fleet::KeyTreeSnapshot, Vec<String>) {
    let session = zenkey_fleet::session::open(&[endpoint()], &[], false)
        .await
        .expect("open session");
    let monitor = Monitor::start(
        &session,
        MonitorSpec {
            selectors,
            liveliness: scope::liveliness_any_base(),
            ..Default::default()
        },
    )
    .await
    .expect("start monitor");

    tokio::time::sleep(SETTLE).await;

    let tree = monitor.tree();
    let keys = monitor
        .core()
        .with_stats(|s| s.iter().map(|(k, _)| k.to_string()).collect::<Vec<_>>());
    ((*tree).clone(), keys)
}

fn slices() -> SliceSet {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixture-tests/registry");
    SliceSet::from_dirs(&[dir]).expect("fixture registry")
}

/// The raw scope must show foreign traffic *and* conforming traffic, and must
/// not show anything on a verbatim plane (RFC 03 §4 D2).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a live bus; see the module docs"]
async fn the_raw_scope_sees_plain_keys_but_no_verbatim_planes() {
    let selectors = ScopePreset::Everything.selectors("", &[]);
    let (_tree, keys) = watch(selectors).await;
    assert!(!keys.is_empty(), "no traffic — is `spray` running?");
    println!("observed {} keys:", keys.len());
    for k in &keys {
        println!("  {k}");
    }

    // Foreign keys are first-class.
    assert!(keys.iter().any(|k| k == "demo/example/foo"));
    assert!(keys.iter().any(|k| k == "two/chunks"));
    // Conforming keys arrive too.
    assert!(keys.iter().any(|k| k.contains("/telemetry/sysinfo/disk/")));
    // Another deployment's traffic is visible — that is the point of running
    // un-namespaced (RFC 09 §5).
    assert!(keys.iter().any(|k| k.starts_with("someotherbase/")));

    // …and the verbatim planes are not reachable by `**`.
    for k in &keys {
        assert!(
            !k.contains("/@media/"),
            "`**` must not reach @media (RFC 03 §4 D2): {k}"
        );
        assert!(
            !k.contains("/@catalog/"),
            "`**` must not reach @catalog: {k}"
        );
    }
}

/// The Deployment scope's explicit `@catalog` selector is the only reason
/// service-origin traffic is visible at all (D4).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a live bus; see the module docs"]
async fn the_deployment_scope_reaches_the_catalog() {
    let (_tree, keys) = watch(ScopePreset::Deployment.selectors("", &[])).await;
    assert!(
        keys.iter().any(|k| k.contains("/@catalog/")),
        "the @catalog selector should have delivered entity state; got {keys:#?}"
    );
}

/// The full overlay, against real traffic: every arm of the classification
/// ladder should be exercised by one of `spray`'s keys.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a live bus; see the module docs"]
async fn the_overlay_classifies_real_traffic_correctly() {
    let (_tree, keys) = watch(ScopePreset::Everything.selectors("", &[])).await;
    let slices = slices();
    let facts = |k: &str| {
        let mut f = KeyFacts::project("", k);
        f.resolve(&slices);
        f
    };

    let find = |needle: &str| {
        keys.iter()
            .find(|k| k.contains(needle))
            .unwrap_or_else(|| panic!("no key containing {needle:?} in {keys:#?}"))
            .clone()
    };

    // Registered.
    let registered = facts(&find("/telemetry/sysinfo/disk/"));
    assert!(
        matches!(registered.registration, Registration::Registered(_)),
        "expected Registered, got {:?}",
        registered.registration
    );
    assert_eq!(registered.type_name(), Some("TelemetryPoint"));

    // Conforming but not in the registry.
    let unregistered = facts(&find("/not/a/real/subject"));
    assert_eq!(unregistered.registration, Registration::Unregistered);

    // Plain foreign traffic: unparsed, and with no registry verdict.
    let foreign = facts(&find("demo/example/foo"));
    assert!(matches!(foreign.shape, KeyShape::Unparsed { .. }));
    assert_eq!(foreign.registration, Registration::NotApplicable);

    // A v2 key is not this convention.
    assert!(matches!(
        facts(&find("v2/h-")).shape,
        KeyShape::Unparsed { .. }
    ));

    // With the empty base everything is "under the base"; with a named base,
    // the same key is definitively elsewhere.
    let other = find("someotherbase/");
    assert!(matches!(
        KeyFacts::project("", &other).shape,
        KeyShape::V1(_) | KeyShape::Unparsed { .. }
    ));
    assert_eq!(
        KeyFacts::project("zensight", &other).shape,
        KeyShape::NotUnderBase
    );
}

/// RFC 09 §5.1 O6, against real traffic: a bounded observer stays bounded and
/// says what the bound cost. `spray` publishes 9 keys the raw scope can see, so
/// a bound of 3 must engage.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a live bus; see the module docs"]
async fn a_bounded_observer_reports_what_it_retired() {
    let session = zenkey_fleet::session::open(&[endpoint()], &[], false)
        .await
        .expect("open session");
    let monitor = Monitor::start(
        &session,
        MonitorSpec {
            selectors: ScopePreset::Everything.selectors("", &[]),
            max_keys: 3,
            ..Default::default()
        },
    )
    .await
    .expect("start monitor");

    tokio::time::sleep(SETTLE).await;

    let (len, evicted) = monitor.core().with_stats(|s| (s.len(), s.evicted()));
    println!("bounded at 3: tracking {len} keys, retired {evicted}");
    assert!(len <= 3, "the bound must hold, got {len}");
    assert!(
        evicted > 0,
        "spray publishes more than 3 keys — eviction should have engaged"
    );
    // The count the UI shows must disclose the loss rather than just shrink.
    let label = zengui::view::status::keys_text(len, evicted);
    assert!(label.contains("retired"), "{label}");
}

/// The tree must render foreign traffic as ordinary nodes with real counts.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a live bus; see the module docs"]
async fn the_tree_carries_foreign_traffic() {
    let (snapshot, _keys) = watch(ScopePreset::Everything.selectors("", &[])).await;
    let expanded: BTreeSet<String> = ["demo".to_string(), "demo/example".to_string()]
        .into_iter()
        .collect();
    let skel = zenkey_fleet::Skeleton::build(
        "",
        &zenkey_fleet::SliceSet::default(),
        &std::collections::BTreeMap::new(),
        None,
    );
    let merged = zenkey_fleet::skeleton::merge(&skel, &snapshot, &["**".to_string()]);
    let flat = tree::flatten(&merged, "", &expanded, 500, std::time::Instant::now());

    let demo = flat
        .rows
        .iter()
        .find(|r| r.path == "demo")
        .expect("a `demo` node");
    assert!(demo.subtree_count > 0, "demo subtree should carry traffic");
    assert!(demo.subtree_keys >= 3, "demo has several keys");
    assert_eq!(demo.role, None, "foreign chunks are not labelled");

    let leaf = flat
        .rows
        .iter()
        .find(|r| r.path == "demo/example/foo")
        .expect("the foo leaf");
    assert!(leaf.is_leaf);
    assert!(leaf.count > 0);
    println!(
        "demo/example/foo: {} samples, {} bytes, {:.1} Hz",
        leaf.count, leaf.bytes, leaf.rate_hz
    );
}

/// Suspect-on-retraction (#61), self-contained (NOT ignored): a second
/// in-process peer declares a conforming liveliness token; undeclaring it
/// must flip the roster to suspect on the very next NodeDown event — no ttl
/// aging, no polling — and redeclaring recovers it. Port 7476.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_retracted_token_flips_the_roster_to_suspect_immediately() {
    use std::time::Instant;
    use zenkey_fleet::{FleetEvent, StreamItem};

    let producer_side =
        zenkey_fleet::session::open(&[], &["tcp/127.0.0.1:7476".to_string()], false)
            .await
            .expect("producer session");
    let observer_side =
        zenkey_fleet::session::open(&["tcp/127.0.0.1:7476".to_string()], &[], false)
            .await
            .expect("observer session");

    let monitor = Monitor::start(
        &observer_side,
        MonitorSpec {
            selectors: vec![], // lazy: liveliness only
            liveliness: scope::liveliness_selectors(""),
            ..Default::default()
        },
    )
    .await
    .expect("start monitor");
    let mut events = monitor.events();

    let mut roster = zengui::nodes::NodeRoster::default();
    const TOKEN: &str = "v1/h-3fa9c2d41b7e/state/sysinfo/alive";

    // Bounded wait for the matching liveliness event, applied in order.
    async fn apply_next(
        roster: &mut zengui::nodes::NodeRoster,
        events: &mut zenkey_fleet::EventStream,
        want_up: bool,
    ) {
        loop {
            let item = tokio::time::timeout(Duration::from_secs(5), events.recv())
                .await
                .expect("liveliness event within 5s")
                .expect("monitor alive");
            if let StreamItem::Event(ev) = item {
                match ev {
                    FleetEvent::NodeUp(key) if want_up => {
                        roster.apply_transitions("", &[(key, true)], Instant::now());
                        break;
                    }
                    FleetEvent::NodeDown(key) if !want_up => {
                        roster.apply_transitions("", &[(key, false)], Instant::now());
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    let token = producer_side
        .liveliness()
        .declare_token(TOKEN)
        .await
        .expect("token");
    apply_next(&mut roster, &mut events, true).await;
    let p = &roster.get("h-3fa9c2d41b7e").expect("origin seen")["sysinfo"];
    assert!(p.alive && p.suspect_since.is_none());

    token.undeclare().await.expect("undeclare");
    apply_next(&mut roster, &mut events, false).await;
    let p = &roster.get("h-3fa9c2d41b7e").unwrap()["sysinfo"];
    assert!(
        !p.alive && p.suspect_since.is_some(),
        "retraction must mark suspect on the event itself, never aged out"
    );

    let _token = producer_side
        .liveliness()
        .declare_token(TOKEN)
        .await
        .expect("redeclare");
    apply_next(&mut roster, &mut events, true).await;
    let p = &roster.get("h-3fa9c2d41b7e").unwrap()["sysinfo"];
    assert!(
        p.alive && p.suspect_since.is_none(),
        "reappearance recovers the row"
    );
}

/// The history/plotting path end to end (#63, #64), against real traffic.
///
/// What only a live bus can prove: that samples reach a recorder through the
/// monitor with a usable arrival stamp, that a real tombstone survives the
/// engine as a `Delete` rather than an empty put, and that consecutive
/// payloads actually differ enough to diff and to plot. `spray` retires
/// `state/…/health` every twentieth sample, so this waits for one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a live bus; see the module docs"]
async fn history_records_a_real_tombstone_and_a_plottable_series() {
    use zengui::history::HistoryRecorder;
    use zengui::series::value_series;
    use zenkey_fleet::{FleetEvent, StreamItem};

    let session = zenkey_fleet::session::open(&[endpoint()], &[], false)
        .await
        .expect("open session");
    let monitor = Monitor::start(
        &session,
        MonitorSpec {
            selectors: vec![
                "v1/*/state/sysinfo/health".to_string(),
                "v1/*/telemetry/sysinfo/disk/**".to_string(),
            ],
            ..Default::default()
        },
    )
    .await
    .expect("start monitor");

    let mut events = monitor.events();
    let mut health = HistoryRecorder::new("", 512);
    let mut disk = HistoryRecorder::new("", 512);
    // The keys carry spray's host id; learn them from the first sample rather
    // than hard-coding what the fixture happens to use today.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while tokio::time::Instant::now() < deadline {
        let Ok(Some(item)) = tokio::time::timeout_at(deadline, events.recv()).await else {
            break;
        };
        if let StreamItem::Event(FleetEvent::Sample(view)) = item {
            let target = if view.key.contains("/state/") {
                &mut health
            } else {
                &mut disk
            };
            if target.key.is_empty() {
                target.key = view.key.clone();
            }
            target.observe(&view);
        }
        if health.ring.iter().any(|e| e.is_delete) && disk.ring.len() > 3 {
            break;
        }
    }

    // A real delete, through the engine, marked as retirement.
    let tombstones = health.ring.iter().filter(|e| e.is_delete).count();
    assert!(
        tombstones > 0,
        "no tombstone in {} health samples — is this `spray` new enough to retire the key?",
        health.ring.len()
    );
    // And it is not merely an empty put: the puts around it carry documents.
    assert!(
        health
            .ring
            .iter()
            .any(|e| !e.is_delete && e.value.is_some()),
        "the health key should also carry ordinary values"
    );

    // Arrival stamps are monotonic and distinct enough to space a series.
    let stamps: Vec<_> = disk.ring.iter().map(|e| e.received).collect();
    assert!(
        stamps.windows(2).all(|w| w[0] >= w[1]),
        "history is newest-first, so arrival must be non-increasing across it"
    );

    // The plotted series moves — a fixture that republished one payload would
    // give a flat line and prove nothing.
    let series = value_series(&disk.ring, "value");
    assert!(series.has_data(), "no numeric `value` on the disk subject");
    let (lo, hi) = series.bounds().expect("bounds over a non-empty series");
    assert!(
        hi > lo,
        "the demo value must actually move ({lo}..{hi}) or the sparkline demo shows nothing"
    );
    println!(
        "health: {} samples, {tombstones} tombstones · disk: {} points, {lo}..{hi}",
        health.ring.len(),
        series.len(),
    );
}
