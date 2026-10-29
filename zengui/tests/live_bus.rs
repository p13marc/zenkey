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
            meter: PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../zenkey/tests/contracts/meter.v1.toml"
            )),
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
    // traffic — that is the point of a pump in no namespace (tooling guide §8.1).
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

/// The tooling guide's O6, against real traffic: a bounded observer stays bounded
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

/// The tree renders foreign traffic as ordinary nodes with real counts,
/// unlabelled, and a zk2 key beside it labelled by zk2's positions under
/// the deployment's namespace (#614, FL7).
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
    let flat = tree::flatten(
        &merged,
        &bus.namespace,
        &expanded,
        500,
        std::time::Instant::now(),
    );

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
    assert_eq!(
        flat.rows[i].role,
        Some(tree::Role::Resource),
        "a zk2 key wears zk2's roles"
    );
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
        admin_declared: false,
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

/// A contract with a templated stream that is reliable and blocking — the
/// only kind of family whose window can be lossless (core §2.7, 0.25): what
/// no example the spray serves declares.
const PROBE: &str = r#"
[interface]
name = "probe"
major = 1

[resources."m/{n}"]
kind = "stream"
type = { raw = "application/octet-stream" }
params = { n = "string" }
cardinality = 8
reliability = "reliable"
congestion = "block"
"#;

/// The tree's identity rungs and a budget badge on real traffic, through
/// the window (#614, FL7). Beside the spray, a probe owner, `lab/probe`,
/// publishes two members of a reliable, blocking templated stream. The
/// window boots, its own presence link reads the deployment, the scope is
/// observed, and its own pump runs until a budget judgement lands. Then the
/// tree, filtered to one key at a time, shows the rung each reached: the
/// owned position stream "resolved", the ghost "no provider", the other
/// namespace's key "not in namespace" — and the probe's family, a lossless
/// window with its owner present throughout, "budget unobservable (window
/// too short)": the only premise a window of seconds cannot meet is an hour
/// of listening.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the zk2 spray; see the module docs"]
async fn the_tree_shows_rungs_and_a_budget_badge_on_real_traffic() {
    use iced::futures::StreamExt as _;
    use zengui::message::{BusMsg, DeploymentMsg, Message, WorkspaceMsg};

    let home = std::env::temp_dir().join(format!("zengui-live-rungs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("scratch dir");
    // SAFETY: set before any store call; no other test here reads a store.
    unsafe { std::env::set_var("ZENKEY_EXPLORER_CONFIG_DIR", &home) };

    let bus = Bus::up().await;

    // The probe owner, through the engine `zenctl gen` runs, on a session
    // of its own in the deployment's namespace.
    let loaded = zenkey_model::contract::load_str(PROBE, std::path::Path::new("."), None);
    let contract = loaded
        .contract
        .unwrap_or_else(|| panic!("does not load:\n{}", loaded.report));
    let revision = std::sync::Arc::new(zenkey_fleet::Revision::from_contract(
        contract,
        zenkey_fleet::report::ContractSource::File,
    ));
    let owner = zenkey_fleet::open_in_namespace(
        &bus.namespace,
        None,
        std::slice::from_ref(&bus.endpoint),
        &[],
        Some(false),
    )
    .await
    .expect("the probe's session");
    let spec = zenkey_fleet::GenSpec {
        address: "lab/probe".parse().expect("an address"),
        revisions: vec![revision],
        members: vec![zenkey_fleet::MemberArg::parse("m/{n}=a,b").expect("members")],
        bindings: Default::default(),
        rate_hz: Some(4.0),
        pattern: zenkey_fleet::GenPattern::Steady,
        duration: Duration::from_secs(600),
        seed: 7,
        tool: "zengui live test".into(),
        elect: None,
    };
    let plan = zenkey_fleet::gen_plan(&spec).expect("the probe plans");
    let (up_tx, up_rx) = tokio::sync::oneshot::channel();
    let probe = {
        let owner = owner.clone();
        tokio::spawn(async move {
            let mut up = Some(up_tx);
            let _ = zenkey_fleet::run_gen(&owner, &spec, &plan, |instance| {
                if let Some(tx) = up.take() {
                    let _ = tx.send(instance.to_owned());
                }
            })
            .await;
        })
    };
    tokio::time::timeout(Duration::from_secs(20), up_rx)
        .await
        .expect("the probe comes up")
        .expect("its instance");

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
        admin_declared: false,
        echo_lines: 100,
        history_entries: 10,
        max_keys: 1000,
    };
    let (mut app, boot) =
        zengui::app::Zengui::with_prefs(settings, zengui::prefs::Prefs::default(), None);
    tokio::time::timeout(Duration::from_secs(20), drive(&mut app, boot))
        .await
        .expect("the boot settles");

    // Both of the window's links, run as its subscriptions would.
    let presence = app.presence_link().expect("a presence link after the boot");
    let mut presence = Box::pin(zengui::presence_link::stream(presence));
    let data = app.data_link().expect("a data pump after the boot");
    let mut data = Box::pin(zengui::link::stream(data));
    let first = data.next().await.expect("the pump runs");
    let _ = app.update(first);
    let observe = app.update(Message::Deployment(DeploymentMsg::ScopeWatchToggled));
    drive(&mut app, observe).await;

    // Forty ticks, ten seconds: the budget judgement runs on the first
    // and every sixteenth after it, as a task the tick returns — run here
    // as the runtime would, with any validation beside it.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut ticks = 0u32;
    while ticks < 40 {
        let message = tokio::time::timeout_at(deadline, async {
            tokio::select! {
                m = presence.next() => m,
                m = data.next() => m,
            }
        })
        .await
        .expect("the links run")
        .expect("a link runs");
        ticks += u32::from(matches!(message, Message::Bus(BusMsg::Tick(_))));
        let task = app.update(message);
        drive(&mut app, task).await;
    }

    // One key at a time, filtered, in a window wide enough for every badge.
    let mut said = |query: &str, want: &str| {
        let _ = app.update(Message::Workspace(WorkspaceMsg::TreeSearchChanged(
            query.to_owned(),
        )));
        let mut ui = iced_test::Simulator::with_size(
            iced::Settings::default(),
            (4000.0, 2400.0),
            app.view(iced::window::Id::unique()),
        );
        assert!(ui.find(want).is_ok(), "{query:?}: {want:?} on screen");
    };
    said(
        &bus.wire("zk2/robot1/nav/nav.v2/stream/position"),
        "resolved",
    );
    said("robot1/ghost/nav.v2/stream/position", "no provider");
    said("elsewhere/zk2/robot9", "not in namespace");
    said(
        &bus.wire("zk2/lab/probe/probe.v1/stream/m/a"),
        "budget unobservable (window too short)",
    );

    probe.abort();
    let _ = owner.close().await;
    bus.down().await;
    let _ = std::fs::remove_dir_all(&home);
}

// ── the Send pane on zk2 (#614, FL10) ──────────────────────────────────────

/// A window booted against `bus`, holding `contracts` (as `--contracts`
/// loads them), its boot tasks run to quiescence. The shared context file it
/// re-reads is a scratch one, never the user's — set once per process; run
/// these with `--test-threads=1` beside the seed test, which sets its own.
async fn booted(bus: &Bus, contracts: Vec<PathBuf>) -> zengui::app::Zengui {
    booted_at(&bus.endpoint, &bus.namespace, contracts).await
}

/// [`booted`], against any endpoint, in `namespace`.
async fn booted_at(
    endpoint: &str,
    namespace: &str,
    contracts: Vec<PathBuf>,
) -> zengui::app::Zengui {
    static HOME: std::sync::Once = std::sync::Once::new();
    HOME.call_once(|| {
        let home = std::env::temp_dir().join(format!("zengui-live-fl10-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).expect("scratch dir");
        // SAFETY: set once, before this process's first window reads a store.
        unsafe { std::env::set_var("ZENKEY_EXPLORER_CONFIG_DIR", &home) };
    });
    let settings = zengui::config::Settings {
        namespace: namespace.to_owned(),
        connect: vec![endpoint.to_owned()],
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
        admin_declared: false,
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
            elect: None,
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

// ── the Workbench on zk2 (#614, FL9) ───────────────────────────────────────

/// A port the OS has just said is free, released the instant its number is
/// known: a hard-coded one collides with a second `cargo test` run.
fn ephemeral() -> String {
    let held = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let port = held.local_addr().expect("local addr").port();
    format!("tcp/127.0.0.1:{port}")
}

/// A service implementing `health.v1` alone at `addr`, its status OK — the
/// spray's `host-a/sysmon`, in miniature.
async fn health_owner(
    session: &zenoh::Session,
    addr: &zenkey_model::grammar::Addr,
) -> zenkey::Service {
    let mut b = zenkey::ServiceBuilder::new(session, zenkey::ServiceConfig::new(addr.clone()));
    let health = b.health().await.expect("health.v1");
    health
        .set_status(zenkey::health::Level::Ok, "nominal")
        .await
        .expect("a status");
    b.start().await.expect("the service starts")
}

/// The window's presence link (its subscription's own stream) over a
/// presence monitor on `watch`, a watch session of its own in no namespace
/// on the deployment's wire keys, reading through `session`, as the window
/// starts one (#830, #875).
async fn presence_stream(
    session: &zenoh::Session,
    watch: &zenkey_fleet::WatchSession,
) -> impl iced::futures::Stream<Item = zengui::message::Message> {
    let spec = MonitorSpec {
        stats_tick: Duration::from_secs(3600),
        ..MonitorSpec::default()
    }
    .with_presence(&zenkey_fleet::PresenceScope::all());
    let monitor = Monitor::start_watch(watch, spec)
        .await
        .expect("presence monitor");
    zengui::presence_link::stream(zengui::presence_link::PresenceKey {
        epoch: 1,
        session: session.clone(),
        monitor: std::sync::Arc::new(monitor),
        store: zenkey_fleet::BundleStore::new(Duration::from_secs(2)),
        timeout: Duration::from_secs(2),
    })
}

/// The next presence landing for which `want` holds, within ten seconds.
async fn landing<S>(
    link: &mut std::pin::Pin<Box<S>>,
    want: impl Fn(&zenkey_fleet::LivePresence) -> bool,
) -> std::sync::Arc<zengui::presence_link::PresenceUpdate>
where
    S: iced::futures::Stream<Item = zengui::message::Message>,
{
    use iced::futures::StreamExt as _;
    use zengui::message::{BusMsg, Message};
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let message = tokio::time::timeout_at(deadline, link.next())
            .await
            .expect("the landing arrives in time")
            .expect("the link runs");
        if let Message::Bus(BusMsg::Presence(_, update)) = message
            && want(&update.presence)
        {
            return update;
        }
    }
}

/// The nodes pane over one presence landing, `addr` selected.
fn nodes_pane_shows(
    update: &zengui::presence_link::PresenceUpdate,
    addr: &zenkey_model::grammar::Addr,
    text: &str,
) -> bool {
    let nodes = zengui::nodes::NodesState::default();
    let mut ui = iced_test::Simulator::with_size(
        iced::Settings::default(),
        (1600.0, 1200.0),
        zengui::view::nodes::pane(zengui::view::nodes::NodesData {
            presence: Some(update),
            nodes: &nodes,
            selected: Some(addr),
            sp: zengui::view::tokens::Spacing::default(),
        }),
    );
    ui.find(text).is_ok()
}

/// An owner going away reads **down**, and coming back reads **up** with a
/// new instance (#614, FL9; core §8.1) — through the window's own presence
/// link, on its own monitor, and drawn by the nodes pane. Self-contained
/// (NOT ignored): two in-process sessions on an ephemeral port, the zk2
/// successor of v1's retracted-token check. Down is said only because the
/// link's seed was complete; the instance that comes back is another
/// instance, and the pane names it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_owner_going_away_reads_down_and_coming_back_reads_up_with_a_new_instance() {
    use zenkey_fleet::AddressStatus;

    let endpoint = ephemeral();
    let owners = zenkey_fleet::open_in_namespace(
        "acme",
        None,
        &[],
        std::slice::from_ref(&endpoint),
        Some(false),
    )
    .await
    .expect("the owners' session");
    let observer = zenkey_fleet::open_in_namespace(
        "acme",
        None,
        std::slice::from_ref(&endpoint),
        &[],
        Some(false),
    )
    .await
    .expect("the window's session in the namespace");
    // The presence monitor's watch session (#830): in no namespace, on the
    // wire keys `acme/zk2/…` (#875), as the window opens it. The owner's
    // token is declared before it, and its withdrawal is still heard.
    let watch = zenkey_fleet::open_watch(
        "acme",
        None,
        std::slice::from_ref(&endpoint),
        &[],
        Some(false),
    )
    .await
    .expect("the window's watch session");
    let svc: zenkey_model::grammar::Addr = "lab/svc".parse().expect("an address");

    let first = health_owner(&owners, &svc).await;
    let first_id = first.instance().to_string();
    let mut link = Box::pin(presence_stream(&observer, &watch).await);
    let up = landing(&mut link, |p| p.status(&svc) == AddressStatus::Up).await;
    assert!(nodes_pane_shows(&up, &svc, "lab/svc: up"));
    assert!(
        nodes_pane_shows(&up, &svc, &first_id),
        "its instance, named"
    );

    first.close().await.expect("the owner closes");
    let down = landing(&mut link, |p| p.status(&svc) == AddressStatus::Down).await;
    assert!(
        nodes_pane_shows(&down, &svc, "lab/svc: down"),
        "a complete seed behind it: no instance token is down"
    );

    let second = health_owner(&owners, &svc).await;
    let second_id = second.instance().to_string();
    assert_ne!(first_id, second_id, "a new run is a new instance");
    let back = landing(&mut link, |p| {
        p.address(&svc)
            .is_some_and(|a| a.instance_tokens().any(|i| i.as_str() == second_id))
    })
    .await;
    assert!(nodes_pane_shows(&back, &svc, "lab/svc: up"));
    assert!(
        nodes_pane_shows(&back, &svc, &second_id),
        "up again, with the new instance"
    );
    assert!(
        !nodes_pane_shows(&back, &svc, &first_id),
        "and not the old one"
    );

    let _ = second.close().await;
    let _ = observer.close().await;
    let _ = owners.close().await;
}

/// Health on selection, through the window (#614, FL9; D11): the window
/// boots against the spray, its presence link reads `host-a/sysmon`, and
/// selecting the address reads its `health.v1` — "age unobservable" with no
/// word for the clock (there is no ambient poll, and no window to measure
/// one), then a level once the operator's word is given.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the zk2 spray; see the module docs"]
async fn health_is_read_on_selection_from_the_sprays_sysmon() {
    use zengui::message::{Message, PaneMsg, RightPane, Subject, SubjectMsg, WorkspaceMsg};
    use zengui::view::nodes::NodesMsg;

    let bus = Bus::up().await;
    let mut app = booted(&bus, vec![]).await;
    let sysmon: zenkey_model::grammar::Addr = spray::SYSMON.parse().expect("an address");
    let key = app.presence_link().expect("a presence link after the boot");
    let mut link = Box::pin(zengui::presence_link::stream(key));
    let update = landing(&mut link, |p| {
        p.status(&sysmon) == zenkey_fleet::AddressStatus::Up
    })
    .await;
    let _ = app.update(Message::Bus(zengui::message::BusMsg::Presence(
        // The window's own epoch: the link was built from it.
        app.presence_link().expect("the link").epoch,
        update,
    )));
    let _ = app.update(Message::Workspace(WorkspaceMsg::PaneSelected(
        RightPane::Nodes,
    )));

    let read = app.update(Message::Subject(SubjectMsg::Select(Subject::Address(
        sysmon.clone(),
    ))));
    tokio::time::timeout(Duration::from_secs(20), drive(&mut app, read))
        .await
        .expect("the reading lands");
    assert!(
        shows(
            &app,
            "age unobservable — this reader's clock is not trusted to the HLC delta against \
             the stamping clock; tick \"clocks synced\" if they agree"
        ),
        "no word and no window: the reply's age is unobservable"
    );

    let read = app.update(Message::Pane(PaneMsg::Nodes(NodesMsg::ClocksSynced(true))));
    tokio::time::timeout(Duration::from_secs(20), drive(&mut app, read))
        .await
        .expect("the second reading lands");
    assert!(
        shows(&app, "clock: the deployment's word (clocks synced)"),
        "read again under the word"
    );
    let said = [
        "healthy (ok) — its status is OK and confirmed, and no check is worse",
        "unhealthy (degraded) — its status is DEGRADED",
    ]
    .into_iter()
    .find(|w| shows(&app, w));
    println!("host-a/sysmon read: {said:?}");
    assert!(
        said.is_some(),
        "sysmon cycles OK → DEGRADED → OK: either level, never unobservable"
    );

    bus.down().await;
}

/// Whether the window, drawn tall enough for a long dock, shows `text`.
fn shows_tall(app: &zengui::app::Zengui, text: &str) -> bool {
    let mut ui = iced_test::Simulator::with_size(
        iced::Settings::default(),
        (2400.0, 4000.0),
        app.view(iced::window::Id::unique()),
    );
    ui.find(text).is_ok()
}

// ── the Config tool on config.v1 (#768, CD) ─────────────────────────────────

/// A device that holds what it is told: every apply and every undo
/// succeeds (`config.v1` §2.9, §2.11).
struct HeldDevice;

impl zenkey::config_v1::Device for HeldDevice {
    fn apply<'a>(
        &'a self,
        _group: &'a str,
        _values: &'a std::collections::BTreeMap<String, zenkey::config_v1::Value>,
    ) -> zenkey::call::BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    fn undo<'a>(
        &'a self,
        _restore: &'a [zenkey::config_v1::Restore],
    ) -> zenkey::call::BoxFuture<'a, Result<(), zenkey::config_v1::UndoFailed>> {
        Box::pin(async { Ok(()) })
    }
}

/// `lab/rf0`'s startup, the profile's `rf` in miniature: `duty` hot and
/// `radio` reach, both writable, from its file; a 2 s floor, and its device
/// applying a reach change 300 ms after receiving it; no persisted layer.
fn rf0_startup() -> zenkey::config_v1::Startup {
    use zenkey::config_v1::{Class, GroupDecl, Kind, ParamDecl, Schema, Startup, Value};
    let group = |name: &str, class, param: &str, min, max, unit: &str| GroupDecl {
        name: name.to_owned(),
        class,
        description: format!("the {name} group"),
        writable: true,
        parameters: vec![ParamDecl {
            name: param.to_owned(),
            kind: Kind::Integer {
                min: Some(min),
                max: Some(max),
                unit: Some(unit.to_owned()),
            },
            description: format!("the {param}"),
            sensitive: false,
        }],
    };
    let one = |g: &str, p: &str, v: i64| {
        (
            g.to_owned(),
            std::collections::BTreeMap::from([(p.to_owned(), Value::Integer(v))]),
        )
    };
    Startup {
        schema: Schema {
            groups: vec![
                group("duty", Class::Hot, "fraction_ppm", 0, 1_000_000, "ppm"),
                group(
                    "radio",
                    Class::Reach,
                    "frequency_hz",
                    150_000_000,
                    960_000_000,
                    "Hz",
                ),
            ],
        },
        defaults: Default::default(),
        file: [
            one("duty", "fraction_ppm", 10_000),
            one("radio", "frequency_hz", 868_100_000),
        ]
        .into(),
        persisted: None,
        floor_s: 2,
        apply_after: Duration::from_millis(300),
        events: Vec::new(),
    }
}

/// A reach change through the Config tool, against an in-process
/// `zenkey::config_v1` owner (#768, CD; `config.v1` §2.4, §2.9–§2.11).
/// Self-contained (NOT ignored): a router on port 0, the owner and the
/// window its clients, in namespace `acme`.
///
/// The window reads `lab/rf0`, sets `radio` with a window and its consent,
/// and is answered pending; its reading listens until the `apply_at` and
/// shows the change applied; confirm is answered with the view, and the
/// view read back runs the new frequency, the change its last. A second
/// reach change is left unconfirmed: the tool's tick listens across its
/// deadline, and the undo is seen — the frequency back, nothing pending,
/// and the change's `rolled-back` event heard.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reach_change_is_confirmed_and_another_rolls_back_at_its_deadline() {
    use zengui::configure::Draft;
    use zengui::message::{Message, PaneMsg, RightPane, Subject, SubjectMsg, WorkspaceMsg};
    use zengui::view::configure::ConfigMsg;

    // R1: a router on an ephemeral port, timestamping on (a router's
    // default), and its loopback endpoint.
    let mut c = zenoh::Config::default();
    c.insert_json5("mode", "\"router\"").unwrap();
    c.insert_json5("listen/endpoints", "[\"tcp/127.0.0.1:0\"]")
        .unwrap();
    c.insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    c.insert_json5("scouting/gossip/enabled", "false").unwrap();
    let router = zenoh::open(c).await.expect("the router");
    let endpoint = router
        .info()
        .locators()
        .await
        .into_iter()
        .map(|l| l.to_string())
        .find(|l| l.starts_with("tcp/127.0.0.1:"))
        .expect("a loopback listener");

    // The owner, a client of R1 in the namespace.
    let owners = zenkey_fleet::open_in_namespace(
        "acme",
        None,
        std::slice::from_ref(&endpoint),
        &[],
        Some(false),
    )
    .await
    .expect("the owner's session");
    let rf0: zenkey_model::grammar::Addr = "lab/rf0".parse().expect("an address");
    let mut b = zenkey::ServiceBuilder::new(&owners, zenkey::ServiceConfig::new(rf0.clone()));
    b.config_v1(rf0_startup(), std::sync::Arc::new(HeldDevice))
        .await
        .expect("config.v1");
    let service = b.start().await.expect("the owner starts");
    let i = service.instance().to_string();

    let mut app = booted_at(&endpoint, "acme", vec![]).await;
    let config = |m: ConfigMsg| Message::Pane(PaneMsg::Config(m));
    let step = |app: &mut zengui::app::Zengui, m: Message| app.update(m);
    async fn settle(app: &mut zengui::app::Zengui, task: iced::Task<Message>) {
        tokio::time::timeout(Duration::from_secs(30), drive(app, task))
            .await
            .expect("the tool's tasks settle");
    }

    // The tool, the address, and the operator's word on the clocks: each
    // reading's stamps are aged, so §5 answers from a fresh view.
    let t = step(
        &mut app,
        Message::Workspace(WorkspaceMsg::PaneSelected(RightPane::Config)),
    );
    settle(&mut app, t).await;
    let t = step(
        &mut app,
        Message::Subject(SubjectMsg::Select(Subject::Address(rf0.clone()))),
    );
    settle(&mut app, t).await;
    let t = step(&mut app, config(ConfigMsg::ClocksSynced(true)));
    settle(&mut app, t).await;
    assert!(shows_tall(&app, "reach — can cut the link"), "read");
    assert!(shows_tall(&app, "868100000"), "the startup frequency");

    // A reach change with a window and its consent: pending, then applied.
    let t = step(
        &mut app,
        config(ConfigMsg::Draft {
            group: "radio".into(),
            param: "frequency_hz".into(),
            draft: Draft::Text("869525000".into()),
        }),
    );
    settle(&mut app, t).await;
    let _ = step(
        &mut app,
        config(ConfigMsg::WindowChanged("radio".into(), "20".into())),
    );
    let _ = step(
        &mut app,
        config(ConfigMsg::ConsentToggled("radio".into(), true)),
    );
    let t = step(&mut app, config(ConfigMsg::Apply("radio".into())));
    settle(&mut app, t).await;
    let first = format!("{i}-1");
    assert!(
        shows_tall(
            &app,
            &format!("pending — change {first}, answered before its apply (§2.9)")
        ),
        "a reach set is answered before its apply"
    );
    assert!(
        shows_tall(&app, &format!("pending change {first} on radio")),
        "the reading after it shows the change"
    );
    assert!(
        shows_tall(&app, "applied — confirm it over the new link"),
        "listened until its apply_at: applied"
    );

    // Confirm: the owner's view after it, then the view read back.
    let t = step(&mut app, config(ConfigMsg::Confirm));
    settle(&mut app, t).await;
    assert!(shows_tall(
        &app,
        "answered — the owner's view after the act (§2.10)"
    ));
    assert!(shows_tall(&app, "869525000"), "the new frequency runs");
    assert!(
        !shows_tall(&app, &format!("pending change {first} on radio")),
        "nothing pending once confirmed"
    );
    assert!(
        shows_tall(&app, &first),
        "the last change is the confirmed one"
    );
    assert_eq!(
        service
            .config_v1()
            .expect("config.v1")
            .view()
            .await
            .last_change
            .map(|l| l.token),
        Some(first.clone()),
        "and the owner agrees"
    );

    // A second reach change, never confirmed: the tick listens across its
    // deadline, and the undo is seen.
    let t = step(
        &mut app,
        config(ConfigMsg::Draft {
            group: "radio".into(),
            param: "frequency_hz".into(),
            draft: Draft::Text("870000000".into()),
        }),
    );
    settle(&mut app, t).await;
    let _ = step(
        &mut app,
        config(ConfigMsg::WindowChanged("radio".into(), "6".into())),
    );
    let _ = step(
        &mut app,
        config(ConfigMsg::ConsentToggled("radio".into(), true)),
    );
    let t = step(&mut app, config(ConfigMsg::Apply("radio".into())));
    settle(&mut app, t).await;
    let second = format!("{i}-2");
    assert!(shows_tall(
        &app,
        &format!("pending change {second} on radio")
    ));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while shows_tall(&app, &format!("pending change {second} on radio")) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the deadline's rollback is never seen"
        );
        let t = step(
            &mut app,
            config(ConfigMsg::Tick(std::time::SystemTime::now())),
        );
        settle(&mut app, t).await;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(shows_tall(&app, "rolled-back"), "its change event, heard");
    assert!(shows_tall(&app, &second), "named by its token");
    assert!(
        shows_tall(&app, "869525000"),
        "the frequency before it, back"
    );
    assert!(
        !shows_tall(&app, "870000000"),
        "the unconfirmed frequency is gone"
    );

    let _ = service.close().await;
    let _ = owners.close().await;
    let _ = router.close().await;
}

/// The doctor, through the window (#614, FL9): booted against the spray,
/// deep and with the clock's word, a run finds what `tests/spray_smoke.rs`
/// finds through the engine — the phantom's interface token with no
/// instance token (`token-missing`) and the squatter's foreign stamp on
/// `host-a/tc`'s state (`state-stamp-foreign`) — and draws both as
/// findings, each check's verdict beside them. Run again until the spray's
/// presence has settled.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the zk2 spray; see the module docs"]
async fn the_windows_doctor_finds_the_sprays_phantom_and_squatter() {
    use zengui::message::{ActivityTab, Message, PaneMsg, WorkspaceMsg};
    use zengui::prefs::LayoutPreset;
    use zengui::view::doctor::DoctorMsg;

    let bus = Bus::up().await;
    let mut app = booted(&bus, vec![]).await;
    let _ = app.update(Message::Workspace(WorkspaceMsg::LayoutPreset(
        LayoutPreset::Diagnose,
    )));
    let _ = app.update(Message::Workspace(WorkspaceMsg::ActivityTab(
        ActivityTab::Doctor,
    )));
    let _ = app.update(Message::Pane(PaneMsg::Doctor(DoctorMsg::DeepToggled(true))));
    let _ = app.update(Message::Pane(PaneMsg::Doctor(DoctorMsg::ClocksSynced(
        true,
    ))));

    let phantom = format!("{}@5ca1ab1e00000001", spray::PHANTOM);
    let (owner, iface, _) = spray::SQUATTED;
    let squatter = format!("{owner} {iface}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    loop {
        let run = app.update(Message::Pane(PaneMsg::Doctor(DoctorMsg::Run)));
        tokio::time::timeout(Duration::from_secs(60), drive(&mut app, run))
            .await
            .expect("the run lands");
        if shows_tall(&app, &phantom) && shows_tall(&app, &squatter) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the doctor never found both: {phantom:?}, {squatter:?}"
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    assert!(shows_tall(&app, "token-missing: finding"));
    assert!(shows_tall(&app, "state-stamp-foreign: finding"));
    assert!(
        shows_tall(&app, "run: a finding at or above warning"),
        "the run as one judgement"
    );

    bus.down().await;
}

// ── the Inspector on zk2 (#614, FL8) ───────────────────────────────────────

/// A session **in** the bus's namespace, as a resolved verb opens one.
async fn in_namespace(bus: &Bus) -> zenoh::Session {
    zenkey_fleet::open_in_namespace(
        &bus.namespace,
        None,
        std::slice::from_ref(&bus.endpoint),
        &[],
        Some(false),
    )
    .await
    .expect("a session in the namespace")
}

/// zk2's why, through the window (#614, FL8b; #702): an owned key the spray
/// answers reads healthy all the way down, and the ghost — a key under an
/// address no instance holds — is explained at presence. Each through the
/// Inspector's own run, held against the engine's reading of the same key.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the zk2 spray; see the module docs"]
async fn why_explains_an_owned_key_and_a_ghost_through_the_window() {
    use zengui::message::{Message, PaneMsg, SlotId, Subject, SubjectMsg};
    use zengui::view::why::WhyMsg;
    use zenkey_fleet::{WhySpec, WhyTarget};

    let bus = Bus::up().await;
    let mut app = booted(&bus, example_contracts()).await;
    let session = in_namespace(&bus).await;
    let store = zenkey_fleet::BundleStore::new(Duration::from_secs(2));
    store.seed(&zengui::services::contracts::load_all(&example_contracts()).set);
    let spec = WhySpec {
        timeout: Duration::from_secs(2),
        window: zengui::services::value::WHY_WINDOW,
    };

    for (rel, healthy) in [
        ("zk2/robot1/nav/nav.v2/state/status", true),
        ("zk2/robot1/ghost/nav.v2/stream/position", false),
    ] {
        let key = bus.wire(rel);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let engine = zenkey_fleet::run_why(
                &session,
                &store,
                &bus.namespace,
                WhyTarget::Key(key.clone()),
                spec,
            )
            .await;
            let expected = format!(
                "why {} — {}",
                engine.target,
                zengui::view::why::headline(&engine)
            );
            let _ = app.update(Message::Subject(SubjectMsg::Select(Subject::Key(
                key.clone(),
            ))));
            let run = app.update(Message::Pane(PaneMsg::Why(SlotId::FOLLOW, WhyMsg::Run)));
            tokio::time::timeout(Duration::from_secs(30), drive(&mut app, run))
                .await
                .expect("the ladder lands");
            let settled = if healthy {
                matches!(
                    engine.verdict,
                    zenkey_fleet::report::Judgement::NotEstablished { .. }
                )
            } else {
                engine.stopped_at == Some(zenkey_fleet::report::RungId::Presence)
            };
            if settled && shows_tall(&app, &expected) {
                println!("{expected}");
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{rel}: the window never read {expected:?}"
            );
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    let _ = session.close().await;
    bus.down().await;
}

/// Last-known from an archive, through the window (#614, FL8a; S5, S6).
/// The spray runs no archive, so the test brings one up beside it —
/// `ground/archive`, recording `host-a/tc`'s interfaces — and once it holds
/// a key, the Inspector's button reads it: labelled last-known, never
/// current, confirmed by alignment, beside the owner's own answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the zk2 spray; see the module docs"]
async fn last_known_reads_an_archive_through_the_window_and_never_as_current() {
    use zengui::message::{Message, PaneMsg, SlotId, Subject, SubjectMsg};
    use zengui::view::detail::DetailMsg;
    use zenkey::archive::{Archive, ArchiveConfig, Recorded};

    let bus = Bus::up().await;
    let archiving = in_namespace(&bus).await;
    let archive = Archive::start(
        &archiving,
        ArchiveConfig {
            service: zenkey::ServiceConfig::new("ground/archive".parse().expect("an address")),
            records: vec![Recorded {
                owner: "host-a/tc".parse().expect("an address"),
                selector: "zk2/host-a/tc/tc.netif.v1/state/interfaces/*/*".to_owned(),
                implementation: zenkey::Implementation::new(held("tc.netif.v1").contract().clone()),
            }],
            peers: Vec::new(),
            unconfirmed_horizon: None,
            watch: None,
        },
    )
    .await
    .expect("an archive");

    // A member the owner published, as the wire spells it.
    let raw = bus.raw().await;
    let sub = raw
        .declare_subscriber(bus.wire("zk2/host-a/tc/tc.netif.v1/state/interfaces/*/*"))
        .await
        .expect("a subscriber");
    let sample = tokio::time::timeout(Duration::from_secs(20), sub.recv_async())
        .await
        .expect("host-a/tc publishes an interface")
        .expect("a sample");
    let wire = sample.key_expr().as_str().to_owned();
    let origin = zenkey_fleet::strip_namespace(&bus.namespace, &wire)
        .expect("in the namespace")
        .to_owned();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while archive.confirmed(&origin).is_none() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        archive.confirmed(&origin).is_some(),
        "the archive holds {origin}"
    );

    let mut app = booted(&bus, example_contracts()).await;
    let read = app.update(Message::Subject(SubjectMsg::Select(Subject::Key(
        wire.clone(),
    ))));
    tokio::time::timeout(Duration::from_secs(20), drive(&mut app, read))
        .await
        .expect("the owner GET lands");
    assert!(
        shows_tall(&app, "via owner GET (S4)"),
        "the owner's own answer"
    );
    let ask = app.update(Message::Pane(PaneMsg::Detail(
        SlotId::FOLLOW,
        DetailMsg::LastKnown("ground/archive".parse().expect("an address")),
    )));
    tokio::time::timeout(Duration::from_secs(20), drive(&mut app, ask))
        .await
        .expect("the archive's answer lands");
    assert!(shows_tall(
        &app,
        "last-known from ground/archive — not current (S5, S6)"
    ));
    assert!(
        shows_tall(&app, "confirmed by alignment (§4.4)")
            || shows_tall(&app, "not confirmed by alignment (§4.4)"),
        "alignment stated either way"
    );
    assert!(
        shows_tall(&app, "via owner GET (S4)"),
        "beside the owner's answer, never in its place"
    );

    drop(sub);
    let _ = raw.close().await;
    drop(archive);
    let _ = archiving.close().await;
    bus.down().await;
}
