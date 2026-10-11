//! End-to-end checks against the zk2 spray (#614, FL6b). Ignored by default:
//! each brings a deployment up and watches real traffic, which takes
//! seconds, not milliseconds.
//!
//! With no endpoint named, each test brings the spray up **in process**, as a
//! router on an ephemeral port (the example included by path, as
//! `tests/spray_smoke.rs` does), so nothing needs to be running and two runs
//! cannot collide. `ZENGUI_TEST_ENDPOINT` points them at a spray already
//! running instead, in the namespace `ZENGUI_TEST_NAMESPACE` (default
//! `spray`, the spray's own default):
//!
//! ```text
//! cargo test -p zengui --test live_bus -- --ignored --nocapture   # in process
//! just spray                  # or: cargo run -p zengui --example spray -- -l tcp/127.0.0.1:7449
//! just test-live              # ZENGUI_TEST_ENDPOINT=tcp/127.0.0.1:7449, the same tests
//! ```
//!
//! These exercise everything under the widgets: the scope selectors, zk2's
//! monitor, the tree snapshot and its merge, the lens over a presence read,
//! and the window's own data pump seeding a watch. The Iced layer is covered
//! by unit tests; this is the part that can only be trusted against real
//! traffic.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use zengui::scope::ScopePreset;
use zengui::view::tree;
use zenkey_fleet::{Monitor, MonitorSpec};

/// The example, whole. Its `main` and the flags only `main` reads are dead
/// here.
#[allow(dead_code)]
#[path = "../examples/spray.rs"]
mod spray;

/// How long a raw watch listens before its keys are read: the owners
/// publish a stream at 1 Hz, the raw traffic at 5 Hz.
const SETTLE: Duration = Duration::from_secs(4);

/// The bus a test runs against: a spray it brought up, or one already
/// running at `ZENGUI_TEST_ENDPOINT`.
struct Bus {
    endpoint: String,
    namespace: String,
    spray: Option<spray::Spray>,
}

impl Bus {
    async fn up() -> Bus {
        if let Ok(endpoint) = std::env::var("ZENGUI_TEST_ENDPOINT") {
            let namespace =
                std::env::var("ZENGUI_TEST_NAMESPACE").unwrap_or_else(|_| "spray".into());
            return Bus {
                endpoint,
                namespace,
                spray: None,
            };
        }
        let args = spray::Args {
            connect: vec![],
            listen: vec!["tcp/127.0.0.1:0".into()],
            namespace: "spray".into(),
            hz: 5.0,
            keys: 0,
            seed: 42,
            contracts: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/zk2")),
        };
        let up = spray::Spray::start(&args)
            .await
            .expect("the spray comes up");
        let endpoint = up
            .endpoints
            .iter()
            .find(|e| e.starts_with("tcp/127.0.0.1:"))
            .expect("a loopback listener")
            .clone();
        Bus {
            endpoint,
            namespace: args.namespace,
            spray: Some(up),
        }
    }

    /// A session in **no** namespace — the data pump's (#614, FL6a).
    async fn raw(&self) -> zenoh::Session {
        zenkey_fleet::open(std::slice::from_ref(&self.endpoint), &[], false)
            .await
            .expect("open session")
    }

    /// `rel`, a base-relative key, as the wire spells it in this bus's
    /// namespace.
    fn wire(&self, rel: &str) -> String {
        zenkey_fleet::with_namespace(&self.namespace, rel)
    }

    async fn down(self) {
        if let Some(spray) = self.spray {
            spray.stop().await;
        }
    }
}

/// Watch `selectors` on a session in no namespace for [`SETTLE`]; the tree
/// and every key the monitor tracked.
async fn observe(
    bus: &Bus,
    selectors: Vec<String>,
) -> (zenkey_fleet::KeyTreeSnapshot, Vec<String>) {
    let session = bus.raw().await;
    let monitor = Monitor::start(
        &session,
        MonitorSpec {
            selectors,
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
    let _ = monitor.shutdown().await;
    let _ = session.close().await;
    ((*tree).clone(), keys)
}

/// The raw scope (`**`, in no namespace) sees the deployment's ambient
/// streams and every foreign key, and nothing under a verbatim chunk — not
/// `robot1/camera`'s explicit `@stream` frames, which no `**` can cross
/// (core §1.3). The explicit preset, which names `@stream`, reaches them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the zk2 spray; see the module docs"]
async fn the_raw_scope_sees_streams_and_not_explicit_frames_which_a_named_selector_reaches() {
    let bus = Bus::up().await;
    let (_tree, keys) = observe(&bus, ScopePreset::Everything.selectors("", &[])).await;
    assert!(!keys.is_empty(), "no traffic — is the spray running?");
    println!("`**` observed {} keys:", keys.len());
    for k in &keys {
        println!("  {k}");
    }

    let position = bus.wire("zk2/robot1/nav/nav.v2/stream/position");
    assert!(
        keys.contains(&position),
        "an ambient zk2 stream rides `**`: {position}"
    );
    // Foreign keys are first-class, and so is another deployment's
    // traffic — that is the point of a pump in no namespace (RFC 09 §5).
    for foreign in ["demo/example/foo", "two/chunks"] {
        assert!(keys.iter().any(|k| k == foreign), "{foreign} is observed");
    }
    assert!(keys.iter().any(|k| k.starts_with("elsewhere/zk2/")));
    // …and nothing under a verbatim chunk.
    for k in &keys {
        assert!(
            !k.split('/').any(|c| c.starts_with('@')),
            "`**` must not cross a verbatim chunk (core §1.3): {k}"
        );
    }

    let image = bus.wire("zk2/robot1/camera/camera.v1/@stream/image");
    let (_tree, explicit) =
        observe(&bus, ScopePreset::Explicit.selectors(&bus.namespace, &[])).await;
    assert!(
        explicit.contains(&image),
        "the explicit preset names `@stream`, so the camera's frames reach it: {explicit:#?}"
    );
    bus.down().await;
}

/// RFC 09 §5.1 O6, against real traffic: a bounded observer stays bounded
/// and says what the bound cost. The spray publishes far more than 3 keys
/// the raw scope can see, so a bound of 3 must engage.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the zk2 spray; see the module docs"]
async fn a_bounded_observer_reports_what_it_retired() {
    let bus = Bus::up().await;
    let session = bus.raw().await;
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
        "the spray publishes more than 3 keys — eviction should have engaged"
    );
    // The count the UI shows must disclose the loss rather than just shrink.
    let label = zengui::view::status::keys_text(len, evicted);
    assert!(label.contains("retired"), "{label}");
    let _ = monitor.shutdown().await;
    let _ = session.close().await;
    bus.down().await;
}

/// The tree renders foreign traffic as ordinary nodes with real counts, and
/// a zk2 key beside it — neither labelled by v1's roles (#614: a zk2 key is
/// unlabelled until FL7, never mislabelled).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the zk2 spray; see the module docs"]
async fn the_tree_carries_foreign_traffic() {
    let bus = Bus::up().await;
    let (snapshot, _keys) = observe(&bus, ScopePreset::Everything.selectors("", &[])).await;
    let position = bus.wire("zk2/robot1/nav/nav.v2/stream/position");
    let mut expanded: BTreeSet<String> = ["demo".to_string(), "demo/example".to_string()]
        .into_iter()
        .collect();
    let mut prefix = String::new();
    for chunk in position.split('/') {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(chunk);
        expanded.insert(prefix.clone());
    }
    let none = zenkey_fleet::ContractSet::new();
    let skel = zenkey_fleet::Skeleton::build(&zenkey_fleet::Lens::new("", None, &none), None);
    let merged = zenkey_fleet::merge_skeleton(&skel, &snapshot, &["**".to_string()]);
    let flat = tree::flatten(&merged, "", &expanded, 500, std::time::Instant::now());

    // `rows` carries the shape; the numbers are joined by `row` (#177), and
    // paths are ids into the flatten's own arena (#251).
    let at = |path: &str| {
        flat.rows
            .iter()
            .position(|r| flat.arena.display(r.path) == path)
            .unwrap_or_else(|| panic!("no `{path}` node"))
    };
    let i = at("demo");
    assert_eq!(flat.rows[i].role, None, "foreign chunks are not labelled");
    let demo = flat.row(i);
    assert!(demo.subtree_count > 0, "demo subtree should carry traffic");
    assert!(demo.subtree_keys >= 3, "demo has several keys");

    let i = at("demo/example/foo");
    assert!(flat.rows[i].is_leaf);
    let leaf = flat.row(i);
    assert!(leaf.count > 0);
    println!(
        "demo/example/foo: {} samples, {} bytes, {:.1} Hz",
        leaf.count, leaf.bytes, leaf.rate_hz
    );

    let i = at(&position);
    assert!(flat.rows[i].is_leaf && flat.row(i).count > 0);
    assert_eq!(flat.rows[i].role, None, "a zk2 key wears no v1 role");
    bus.down().await;
}

/// The lens places the spray's real traffic at its rungs (#614, FL6a's lens
/// against the wire): an owned stream resolves to its resource, the ghost
/// nobody provides stops at "no provider", and a zk2 key in another
/// namespace is not this deployment's. Replaces v1's overlay check, which
/// read `spray_v1`'s v1 registry.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the zk2 spray; see the module docs"]
async fn the_lens_places_real_traffic_at_its_rungs() {
    use zenkey_fleet::report::Unresolved;

    let bus = Bus::up().await;
    let (_tree, keys) = observe(&bus, ScopePreset::Everything.selectors("", &[])).await;
    let session = zenkey_fleet::open_in_namespace(
        &bus.namespace,
        None,
        std::slice::from_ref(&bus.endpoint),
        &[],
        Some(false),
    )
    .await
    .expect("a session in the namespace");
    let store = zenkey_fleet::BundleStore::new(Duration::from_secs(2));
    let catalog = zenkey_fleet::read_lens(&session, &store, Duration::from_secs(2))
        .await
        .expect("a presence read");
    let lens = zenkey_fleet::Lens::new(&bus.namespace, Some(&catalog), &store);
    let find = |needle: &str| {
        keys.iter()
            .find(|k| k.ends_with(needle) || k.starts_with(needle))
            .unwrap_or_else(|| panic!("no key with {needle:?} in {keys:#?}"))
            .clone()
    };

    let position = bus.wire("zk2/robot1/nav/nav.v2/stream/position");
    let resolved = lens.resolve(&position);
    assert_eq!(
        resolved.identity.unresolved, None,
        "an owned stream resolves, with the revision retrieved from the bus"
    );

    let ghost = find("/robot1/ghost/nav.v2/stream/position");
    assert_eq!(
        lens.resolve(&ghost).identity.unresolved,
        Some(Unresolved::NoProvider),
        "{ghost}: nobody provides it"
    );

    let elsewhere = find("elsewhere/zk2/");
    assert!(
        matches!(
            lens.resolve(&elsewhere).identity.unresolved,
            Some(Unresolved::NotInNamespace { .. })
        ),
        "{elsewhere}: another deployment's"
    );
    let _ = session.close().await;
    bus.down().await;
}

/// The history/plotting path end to end (#63, #64), against real traffic.
///
/// What only a live bus can prove: that samples reach a recorder through
/// zk2's monitor with a usable arrival stamp, that a real tombstone survives
/// the engine as a `Delete` rather than an empty put, and that consecutive
/// payloads actually differ enough to diff and to plot. The spray retires
/// `demo/cycle` on the last tick of every twenty, so this waits for one; its
/// v1 key (foreign to zk2) carries a `value` that wanders.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the zk2 spray; see the module docs"]
async fn history_records_a_real_tombstone_and_a_plottable_series() {
    use zengui::history::HistoryRecorder;
    use zengui::series::value_series;
    use zenkey_fleet::{FleetEvent, StreamItem};

    const CYCLE: &str = "demo/cycle";
    const WANDER: &str = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used";

    let bus = Bus::up().await;
    let session = bus.raw().await;
    let monitor = Monitor::start(
        &session,
        MonitorSpec {
            selectors: vec![CYCLE.to_string(), WANDER.to_string()],
            ..Default::default()
        },
    )
    .await
    .expect("start monitor");

    let mut events = monitor.events();
    let mut cycle = HistoryRecorder::new(CYCLE, 512);
    let mut wander = HistoryRecorder::new(WANDER, 512);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while tokio::time::Instant::now() < deadline {
        let Ok(Some(item)) = tokio::time::timeout_at(deadline, events.recv()).await else {
            break;
        };
        if let StreamItem::Event(FleetEvent::Sample(view)) = item {
            cycle.observe(&view);
            wander.observe(&view);
        }
        if cycle.ring.iter().any(|e| e.is_delete) && wander.ring.len() > 3 {
            break;
        }
    }

    // A real delete, through the engine, marked as retirement.
    let tombstones = cycle.ring.iter().filter(|e| e.is_delete).count();
    assert!(
        tombstones > 0,
        "no tombstone in {} samples of {CYCLE}",
        cycle.ring.len()
    );
    // And it is not merely an empty put: the puts around it carry documents.
    assert!(
        cycle.ring.iter().any(|e| !e.is_delete && e.value.is_some()),
        "{CYCLE} should also carry ordinary values"
    );

    // Arrival stamps are monotonic and distinct enough to space a series.
    let stamps: Vec<_> = wander.ring.iter().map(|e| e.received).collect();
    assert!(
        stamps.windows(2).all(|w| w[0] >= w[1]),
        "history is newest-first, so arrival must be non-increasing across it"
    );

    // The plotted series moves — a fixture that republished one payload would
    // give a flat line and prove nothing.
    let series = value_series(&wander.ring, "value");
    assert!(series.has_data(), "no numeric `value` on {WANDER}");
    let (lo, hi) = series.bounds().expect("bounds over a non-empty series");
    assert!(
        hi > lo,
        "the demo value must actually move ({lo}..{hi}) or the sparkline demo shows nothing"
    );
    println!(
        "{CYCLE}: {} samples, {tombstones} tombstones · {WANDER}: {} points, {lo}..{hi}",
        cycle.ring.len(),
        series.len(),
    );
    let _ = monitor.shutdown().await;
    let _ = session.close().await;
    bus.down().await;
}

/// Run a task to quiescence as the runtime would, feeding every message it
/// outputs back through `update` (window actions dropped: no display here).
async fn drive(app: &mut zengui::app::Zengui, task: iced::Task<zengui::message::Message>) {
    use iced::futures::StreamExt as _;
    use iced_test::runtime::{Action, task::into_stream};
    let mut queue = vec![task];
    let mut steps = 0;
    while let Some(task) = queue.pop() {
        let Some(mut stream) = into_stream(task) else {
            continue;
        };
        while let Some(action) = stream.next().await {
            if let Action::Output(message) = action {
                steps += 1;
                assert!(steps < 200, "the tasks did not settle");
                queue.push(app.update(message));
            }
        }
    }
}

/// A late joiner's seed, through the window (#614, FL6b; core §4.2): the
/// window boots against the spray, a tree toggle watches `host-a/tc`, and
/// the window's own data pump delivers the seed boundary — whose state leg,
/// S4's owner GET, counted the owner's replies — and the strip says
/// "seeded 1 watch: h history · s state · k superseded".
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the zk2 spray; see the module docs"]
async fn a_late_joiners_seed_counts_its_state_leg() {
    use iced::futures::StreamExt as _;
    use zengui::message::{BusMsg, Message, SubjectMsg};

    // The boot re-reads the shared context file: a scratch one, never the
    // user's.
    let home = std::env::temp_dir().join(format!("zengui-live-seed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("scratch dir");
    // SAFETY: set before any store call; no other test here reads a store.
    unsafe { std::env::set_var("ZENKEY_EXPLORER_CONFIG_DIR", &home) };

    let bus = Bus::up().await;
    let settings = zengui::config::Settings {
        namespace: bus.namespace.clone(),
        connect: vec![bus.endpoint.clone()],
        listen: vec![],
        scouting: Some(false),
        zenoh_config: None,
        contracts: vec![],
        timeout_secs: 2,
        scope: ScopePreset::Everything,
        selectors: vec![],
        eager: false,
        echo_lines: 100,
        history_entries: 10,
        max_keys: 1000,
    };
    let (mut app, boot) =
        zengui::app::Zengui::with_prefs(settings, zengui::prefs::Prefs::default(), None);
    tokio::time::timeout(Duration::from_secs(20), drive(&mut app, boot))
        .await
        .expect("the boot settles");

    // The window's own pump, subscribed before the watch exists, so the
    // boundary cannot fall between them.
    let key = app.data_link().expect("a data pump after the boot");
    let mut link = Box::pin(zengui::link::stream(key));
    let first = link.next().await.expect("the pump runs");
    let _ = app.update(first);

    let subtree = bus.wire("zk2/host-a/tc");
    let watch = app.update(Message::Subject(SubjectMsg::WatchToggled(subtree.clone())));
    drive(&mut app, watch).await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let coverage = loop {
        let message = tokio::time::timeout_at(deadline, link.next())
            .await
            .expect("the seed resolves in time")
            .expect("the pump runs");
        let seeded = match &message {
            Message::Bus(BusMsg::Tick(tick)) => tick.seeded.first().map(|(_, c)| *c),
            _ => None,
        };
        let _ = app.update(message);
        if let Some(coverage) = seeded {
            break coverage;
        }
    };
    println!("seeded {subtree}: {coverage:?}");
    let state = coverage.state.get().expect("the state leg is asked");
    assert!(
        state.replies > 0,
        "{}'s owner answers S4's GET for its state",
        spray::SQUATTED.0
    );
    let history = coverage.history.get().map_or(0, |l| l.replies);

    let mut said = format!(
        "seeded 1 watch: {history} history · {} state · {} superseded",
        state.replies, coverage.superseded
    );
    let incomplete = [coverage.history.get(), coverage.state.get()]
        .into_iter()
        .flatten()
        .filter(|l| !l.complete)
        .count();
    if incomplete > 0 {
        said.push_str(&format!(
            " · {incomplete} leg{} possibly incomplete (a GET reached its timeout)",
            if incomplete == 1 { "" } else { "s" }
        ));
    }
    let mut ui = iced_test::simulator::<Message, _, _>(app.view(iced::window::Id::unique()));
    assert!(
        ui.find(said.as_str()).is_ok(),
        "the strip says what the seed's two legs brought: {said:?}"
    );

    bus.down().await;
    let _ = std::fs::remove_dir_all(&home);
}

/// v1's alive-token selectors — the fleet's alive tokens, and `@catalog`'s
/// by name (RFC 03 §4 D4). The nodes pane's roster reads v1 tokens until
/// FL9 moves it onto zk2 presence; the test below keeps its own copy.
fn liveliness_selectors(base: &str) -> Vec<String> {
    use zenkey_v1::grammar::with_base;
    use zenkey_v1::origin::ServiceOrigin;
    use zenkey_v1::selector::{self, Scope};
    vec![
        with_base(base, selector::all_liveliness(Scope::fleet())),
        with_base(base, selector::service_alive(&ServiceOrigin::catalog())),
    ]
}

/// Suspect-on-retraction (#61), self-contained (NOT ignored): a second
/// in-process peer declares a conforming v1 liveliness token; undeclaring it
/// must flip the roster to suspect on the very next NodeDown event — no ttl
/// aging, no polling — and redeclaring recovers it. zk2's monitor carries
/// the token events since FL6b; the roster itself is v1's until FL9. The
/// port is ephemeral, so two test runs at once cannot collide.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_retracted_token_flips_the_roster_to_suspect_immediately() {
    use std::time::Instant;
    use zenkey_fleet::{FleetEvent, StreamItem};

    // A port the OS has just said is free, released the instant its number is
    // known: a hard-coded one collides with a second `cargo test` run.
    let endpoint = {
        let held = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
        let port = held.local_addr().expect("local addr").port();
        format!("tcp/127.0.0.1:{port}")
    };
    let producer_side = zenkey_fleet::open(&[], std::slice::from_ref(&endpoint), false)
        .await
        .expect("producer session");
    let observer_side = zenkey_fleet::open(std::slice::from_ref(&endpoint), &[], false)
        .await
        .expect("observer session");

    let monitor = Monitor::start(
        &observer_side,
        MonitorSpec {
            selectors: vec![], // lazy: liveliness only
            liveliness: liveliness_selectors(""),
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

// ── the Send pane on zk2 (#614, FL10) ──────────────────────────────────────

/// A window booted against `bus`, holding `contracts` (as `--contracts`
/// loads them), its boot tasks run to quiescence. The shared context file it
/// re-reads is a scratch one, never the user's — set once per process; run
/// these with `--test-threads=1` beside the seed test, which sets its own.
async fn booted(bus: &Bus, contracts: Vec<PathBuf>) -> zengui::app::Zengui {
    static HOME: std::sync::Once = std::sync::Once::new();
    HOME.call_once(|| {
        let home = std::env::temp_dir().join(format!("zengui-live-fl10-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).expect("scratch dir");
        // SAFETY: set once, before this process's first window reads a store.
        unsafe { std::env::set_var("ZENKEY_EXPLORER_CONFIG_DIR", &home) };
    });
    let settings = zengui::config::Settings {
        namespace: bus.namespace.clone(),
        connect: vec![bus.endpoint.clone()],
        listen: vec![],
        scouting: Some(false),
        zenoh_config: None,
        contracts,
        timeout_secs: 2,
        scope: ScopePreset::Everything,
        selectors: vec![],
        // The scope observed from the start (`**`, in no namespace): what a
        // foreign publish needs to come back through the pump.
        eager: true,
        echo_lines: 500,
        history_entries: 10,
        max_keys: 5000,
    };
    let (mut app, boot) =
        zengui::app::Zengui::with_prefs(settings, zengui::prefs::Prefs::default(), None);
    tokio::time::timeout(Duration::from_secs(20), drive(&mut app, boot))
        .await
        .expect("the boot settles");
    app
}

/// The example contracts, as `--contracts` names them.
fn example_contracts() -> Vec<PathBuf> {
    let examples = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/zk2"));
    vec![examples.join("walkthrough"), examples.join("tcgui")]
}

/// The revision of `iface` the example contracts hold.
fn held(iface: &str) -> std::sync::Arc<zenkey_fleet::Revision> {
    let loaded = zengui::services::contracts::load_all(&example_contracts());
    loaded
        .set
        .of_iface(&iface.parse().expect("an interface"))
        .next()
        .expect("held")
        .clone()
}

fn send_msg(m: zengui::view::send::SendMsg) -> zengui::message::Message {
    zengui::message::Message::Pane(zengui::message::PaneMsg::Send(m))
}

/// Whether the window's main view shows `text`, exactly.
fn shows(app: &zengui::app::Zengui, text: &str) -> bool {
    let mut ui = iced_test::simulator::<zengui::message::Message, _, _>(
        app.view(iced::window::Id::unique()),
    );
    ui.find(text).is_ok()
}

/// A call through the window, against the spray (#614, FL10; spec §5.1):
/// `nav.v2`'s `set_origin` at `robot1/nav`, which the spray serves, answers
/// a value; at `robot3/nav`, where a mock owner (`zenctl serve`'s engine,
/// brought up here because the spray refuses nothing) refuses with `busy`,
/// an envelope; at `robot9/nav`, where nothing runs, silence — a non-verdict
/// with what presence could see. Three answers, three renderings (O5).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the zk2 spray; see the module docs"]
async fn a_call_keeps_value_envelope_and_silence_apart() {
    use zengui::message::{Message, RightPane, WorkspaceMsg};
    use zengui::view::send::{RevisionChoice, SendMode, SendMsg};

    let bus = Bus::up().await;
    let nav = held("nav.v2");

    // The refusing owner: one operation of nav.v2 at an address the spray
    // does not run, on a session in the namespace, as `zenctl serve` runs.
    let owner = zenkey_fleet::open_in_namespace(
        &bus.namespace,
        None,
        std::slice::from_ref(&bus.endpoint),
        &[],
        Some(false),
    )
    .await
    .expect("the mock owner's session");
    let served = zenkey_fleet::serve_operation(
        &owner,
        zenkey_fleet::ServeSpec {
            address: "robot3/nav".parse().unwrap(),
            revision: std::sync::Arc::clone(&nav),
            operation: "@op/set_origin".into(),
            answer: zenkey_fleet::MockAnswer::refusal("busy", "the origin is being moved", None)
                .unwrap(),
            bindings: Default::default(),
            tool: "zengui live test".into(),
        },
    )
    .await
    .expect("the mock owner comes up");

    let mut app = booted(&bus, example_contracts()).await;
    let _ = app.update(Message::Workspace(WorkspaceMsg::PaneSelected(
        RightPane::Send,
    )));
    let _ = app.update(send_msg(SendMsg::ModeSelected(SendMode::Call)));
    let _ = app.update(send_msg(SendMsg::RevisionPicked(RevisionChoice(nav))));
    let _ = app.update(send_msg(SendMsg::OperationPicked("@op/set_origin".into())));
    let _ = app.update(send_msg(SendMsg::BodyChanged(
        r#"{"origin": {"latitude_deg": 47.6, "longitude_deg": -122.3, "altitude_m": 56.0}}"#.into(),
    )));

    for (address, want) in [
        ("robot1/nav", "value"),
        ("robot3/nav", "refused: busy"),
        ("robot9/nav", "No answer"),
    ] {
        let _ = app.update(send_msg(SendMsg::TargetChanged(address.into())));
        let task = app.update(send_msg(SendMsg::Submit));
        tokio::time::timeout(Duration::from_secs(15), drive(&mut app, task))
            .await
            .expect("the call settles");
        let selector = format!("→ zk2/{address}/nav.v2/@op/set_origin");
        assert!(shows(&app, &selector), "{address}: the key it went out on");
        assert!(shows(&app, want), "{address}: answered {want:?}");
        // ("value" is the one word another surface could also show; the
        // other two are the outcome's alone.)
        for other in ["refused: busy", "No answer"]
            .into_iter()
            .filter(|o| *o != want)
        {
            assert!(!shows(&app, other), "{address}: not also {other:?}");
        }
        println!("{address}: {want}");
    }

    drop(served);
    let _ = owner.close().await;
    bus.down().await;
}

/// A refused fan-out sends nothing (#614, FL10; spec §5.1 O2): a queryable of
/// this test's own, on every `tc.netif.v1` interface-set key in the
/// namespace, counts the queries that reach it. The window's call to
/// `*/tc`, and to `host-a/tc` with its parameters left wildcards, are
/// refused in the form with no task at all — and the counter stays at zero,
/// though a GET of the test's own on those keys reaches it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the zk2 spray; see the module docs"]
async fn a_refused_fan_out_sends_nothing() {
    use std::sync::atomic::{AtomicU64, Ordering};
    use zengui::view::send::{RevisionChoice, SendMode, SendMsg};

    let bus = Bus::up().await;
    let raw = bus.raw().await;
    let heard = std::sync::Arc::new(AtomicU64::new(0));
    let counter = std::sync::Arc::clone(&heard);
    let _queryable = raw
        .declare_queryable(bus.wire("zk2/*/tc/tc.netif.v1/@op/interfaces/**"))
        .callback(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        })
        .await
        .expect("the counting queryable");

    let mut app = booted(&bus, example_contracts()).await;
    let _ = app.update(send_msg(SendMsg::ModeSelected(SendMode::Call)));
    let _ = app.update(send_msg(SendMsg::RevisionPicked(RevisionChoice(held(
        "tc.netif.v1",
    )))));
    let _ = app.update(send_msg(SendMsg::OperationPicked(
        "@op/interfaces/{ns}/{iface}/set".into(),
    )));
    let _ = app.update(send_msg(SendMsg::BodyChanged(
        r#"{"operation": "Enable"}"#.into(),
    )));
    for (address, params) in [("*/tc", "ns=default;iface=eth0"), ("host-a/tc", "")] {
        let _ = app.update(send_msg(SendMsg::TargetChanged(address.into())));
        let _ = app.update(send_msg(SendMsg::ParamsChanged(params.into())));
        let task = app.update(send_msg(SendMsg::Submit));
        assert_eq!(task.units(), 0, "{address}: no call was started");
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        heard.load(Ordering::SeqCst),
        0,
        "a refused fan-out reached no queryable"
    );

    // The counter does count: a GET of this test's own on one of the keys.
    let replies = raw
        .get(bus.wire("zk2/host-a/tc/tc.netif.v1/@op/interfaces/default/eth0/set"))
        .target(zenoh::query::QueryTarget::All)
        .timeout(Duration::from_secs(2))
        .await
        .expect("a GET");
    while replies.recv_async().await.is_ok() {}
    assert!(
        heard.load(Ordering::SeqCst) >= 1,
        "the counting queryable hears a query on these keys"
    );

    let _ = raw.close().await;
    bus.down().await;
}

/// A foreign publish reaches the echo (#614, FL10): the window publishes a
/// foreign key as bytes on zenoh's default axes, its own pump carries the
/// sample back, and the echo shows the key. A key a zk2 service owns, on
/// the same form, is refused (P3) and starts nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the zk2 spray; see the module docs"]
async fn a_foreign_publish_reaches_the_echo() {
    use iced::futures::StreamExt as _;
    use zengui::message::{ActivityTab, BusMsg, Message, WorkspaceMsg};
    use zengui::view::send::SendMsg;

    let bus = Bus::up().await;
    let mut app = booted(&bus, vec![]).await;
    let key = format!("demo/fl10/probe-{}", std::process::id());

    // P3 on the same form: refused, nothing started.
    let _ = app.update(send_msg(SendMsg::KeyChanged(
        bus.wire("zk2/robot1/nav/nav.v2/state/status"),
    )));
    let refused = app.update(send_msg(SendMsg::Send));
    assert_eq!(refused.units(), 0, "an owned key starts nothing");

    let link_key = app.data_link().expect("a data pump after the boot");
    let mut link = Box::pin(zengui::link::stream(link_key));
    let first = link.next().await.expect("the pump runs");
    let _ = app.update(first);

    let _ = app.update(send_msg(SendMsg::KeyChanged(key.clone())));
    let _ = app.update(send_msg(SendMsg::BodyChanged("hello from FL10".into())));
    let publish = app.update(send_msg(SendMsg::Send));
    tokio::time::timeout(Duration::from_secs(10), drive(&mut app, publish))
        .await
        .expect("the publish settles");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let message = tokio::time::timeout_at(deadline, link.next())
            .await
            .expect("the sample comes back in time")
            .expect("the pump runs");
        let seen = match &message {
            Message::Bus(BusMsg::Tick(tick)) => tick
                .samples
                .iter()
                .any(|s| s.key == key && s.payload.to_bytes().as_ref() == b"hello from FL10"),
            _ => false,
        };
        let _ = app.update(message);
        if seen {
            break;
        }
    }
    let _ = app.update(Message::Workspace(WorkspaceMsg::ActivityTab(
        ActivityTab::Echo,
    )));
    assert!(shows(&app, &key), "the echo shows the foreign key");
    let _ = app.update(Message::Workspace(WorkspaceMsg::ActivityTab(
        ActivityTab::Publish,
    )));
    assert!(
        shows(
            &app,
            &format!("published {key} (15 bytes) · qos data/drop/reliable")
        ),
        "the publish log names what went out"
    );

    bus.down().await;
}

/// A recorded window replays through the offline lens (#614, FL10b; O4): a
/// capture of the spray's traffic (`.zrec` v3, recorded as the window's own
/// recorder does) opened in a window whose presence link has read the live
/// deployment. While it replays, the tree merges against a skeleton with no
/// presence read — the strip says so where it said the live addresses — the
/// banner states the capture's exclusions and that it resolves offline.
/// Leaving gives the live skeleton back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the zk2 spray; see the module docs"]
async fn a_recorded_window_replays_through_the_offline_lens() {
    use iced::futures::StreamExt as _;
    use zengui::message::{ActivityTab, Message, WorkspaceMsg};
    use zengui::view::replay::ReplayMsg;

    let bus = Bus::up().await;

    // The capture: `**` in no namespace for a few seconds, through the
    // fleet's recorder, into a scratch file.
    let path = std::env::temp_dir().join(format!("zengui-fl10-{}.zrec", std::process::id()));
    {
        let session = bus.raw().await;
        let selectors = ScopePreset::Everything.selectors("", &[]);
        let monitor = Monitor::start(
            &session,
            MonitorSpec {
                selectors: selectors.clone(),
                ..Default::default()
            },
        )
        .await
        .expect("start monitor");
        let header = zenkey_fleet::ZrecHeader {
            zrec: zenkey_fleet::ZREC_VERSION,
            excluded: Some(zenkey_fleet::zrec_excluded(&selectors)),
            selectors,
            base: bus.namespace.clone(),
            captured_at: zenkey_fleet::rfc3339_now(),
            preamble: None,
            pre_roll: None,
        };
        let file = std::fs::File::create(&path).expect("a scratch file");
        let sink = zenkey_fleet::ZrecSink::spawn(std::io::BufWriter::new(file), &header)
            .await
            .expect("the sink");
        let mut events = monitor.events();
        zenkey_fleet::record(
            &mut events,
            &sink,
            zenkey_fleet::RecordBounds {
                max_samples: None,
                max_duration: Some(SETTLE),
            },
            |_, _| {},
        )
        .await
        .expect("the capture");
        let counts = sink.finish().await.expect("the trailer");
        assert!(counts.samples > 0, "the spray's traffic was captured");
        println!("captured {} samples to {}", counts.samples, path.display());
        let _ = monitor.shutdown().await;
        let _ = session.close().await;
    }

    // What the replay's skeleton will say: every revision held, no presence.
    let offline = zengui::view::status::skeleton_text(Some(
        &zengui::replay::ReplayLens::offline(
            &bus.namespace,
            Some(zengui::services::contracts::load_all(&example_contracts()).set),
        )
        .skeleton
        .coverage,
    ));
    assert!(offline.contains("no presence read"), "{offline}");

    // The window, with the live deployment read through its presence link.
    let mut app = booted(&bus, example_contracts()).await;
    let presence = app.presence_link().expect("a presence link after the boot");
    let mut presence = Box::pin(zengui::presence_link::stream(presence));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let message = tokio::time::timeout_at(deadline, presence.next())
            .await
            .expect("presence lands")
            .expect("the link runs");
        let landed = matches!(
            &message,
            Message::Bus(zengui::message::BusMsg::Presence(..))
        );
        let task = app.update(message);
        drive(&mut app, task).await;
        if landed {
            break;
        }
    }
    assert!(
        !shows(&app, &offline),
        "the live skeleton is placed by presence, not this: {offline}"
    );

    // Open the capture through the window's own load path.
    let replay = |m| Message::Workspace(WorkspaceMsg::Replay(m));
    let _ = app.update(replay(ReplayMsg::OpenToggled));
    let _ = app.update(replay(ReplayMsg::PathChanged(path.display().to_string())));
    let open = app.update(replay(ReplayMsg::Open));
    tokio::time::timeout(Duration::from_secs(20), drive(&mut app, open))
        .await
        .expect("the load and its offline check settle");
    let _ = app.update(Message::Workspace(WorkspaceMsg::ActivityTab(
        ActivityTab::Replay,
    )));
    assert!(
        shows(&app, &offline),
        "the strip states the replay's own skeleton: {offline}"
    );
    assert!(
        shows(
            &app,
            "excluded: @stream, @state, @op, @zk, @adv — no selector names them, so no sample \
             under them is in this file"
        ),
        "the banner states what the capture could not contain"
    );
    assert!(
        shows(
            &app,
            "resolved offline: 10 revisions held, no presence read — the live catalog \
             describes another world"
        ),
        "the banner says the replay resolves offline"
    );

    let _ = app.update(replay(ReplayMsg::Exit));
    assert!(
        !shows(&app, &offline),
        "leaving gives the tree back to the live skeleton"
    );

    let _ = std::fs::remove_file(&path);
    bus.down().await;
}
