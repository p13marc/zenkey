//! The zk2 spray, smoked (#614, FL5): the demo traffic `just spray` and
//! `just gui-demo` run is a deployment a tool can read, and its deliberately
//! non-conforming traffic sits where the doctor sees it.
//!
//! In process, with no port named: the example is included by path and
//! brought up as a router on an ephemeral port, and a tool's two sessions
//! read it as `zenctl service list` and `zenctl doctor --deep
//! --clocks-synced` read it. `just smoke-spray` runs those two binaries
//! against the example binary itself.

use std::path::PathBuf;
use std::time::Duration;

use zenkey_fleet::report::CheckId;
use zenkey_fleet::{BundleStore, DoctorBus, DoctorSpec, PresenceScope};

/// The example, whole. Its `main` and the flags only `main` reads are dead
/// here.
#[allow(dead_code)]
#[path = "../examples/spray.rs"]
mod spray;

/// A panic whose unwind hangs ends in seconds, with stacks (#804).
#[path = "../../zenkey/tests/common/panic_watchdog.rs"]
mod panic_watchdog;

const NAMESPACE: &str = "spray";

/// How long a presence listing may take to show every address.
const SETTLE: Duration = Duration::from_secs(20);

async fn client(endpoint: &str, namespace: Option<&str>) -> zenoh::Session {
    let mut c = zenoh::Config::default();
    c.insert_json5("scouting/multicast/enabled", "false")
        .expect("config");
    c.insert_json5("mode", "\"client\"").expect("config");
    c.insert_json5("connect/endpoints", &format!("[\"{endpoint}\"]"))
        .expect("config");
    if let Some(ns) = namespace {
        c.insert_json5("namespace", &format!("\"{ns}\""))
            .expect("config");
    }
    zenoh::open(c).await.expect("a client session")
}

/// Every owner and `host-a/sysmon` is listed with its descriptor served,
/// beside the phantom; the doctor finds the phantom's missing instance token
/// (`token-missing`) and the squatter's foreign stamp on `host-a/tc`'s state
/// (`state-stamp-foreign`, asked under `deep`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_spray_is_a_deployment_and_the_doctor_finds_its_non_conformance() {
    panic_watchdog::install();
    let args = spray::Args {
        connect: vec![],
        listen: vec!["tcp/127.0.0.1:0".into()],
        namespace: NAMESPACE.into(),
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
    let tool = client(&endpoint, Some(NAMESPACE)).await;
    let raw = client(&endpoint, None).await;

    let mut want: Vec<String> = up.instances.keys().cloned().collect();
    want.push(spray::PHANTOM.to_owned());
    assert_eq!(want.len(), 11, "ten services and the phantom: {want:?}");
    let deadline = tokio::time::Instant::now() + SETTLE;
    loop {
        let listing =
            zenkey_fleet::service_listing(&tool, &PresenceScope::all(), Duration::from_secs(2))
                .await
                .expect("a presence read");
        let seen: Vec<&str> = listing
            .services
            .iter()
            .map(|s| s.address.as_str())
            .collect();
        if listing.complete && want.iter().all(|a| seen.contains(&a.as_str())) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "never listed together: {want:?}; last read {seen:?}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let mut spec = DoctorSpec::new(Duration::from_secs(2));
    spec.deep = true;
    spec.clocks_synced = true;
    let bus = DoctorBus {
        session: tool.clone(),
        raw: raw.clone(),
        namespace: NAMESPACE.into(),
        watch: None,
    };
    let report =
        zenkey_fleet::run_doctor(&bus, &BundleStore::new(Duration::from_secs(2)), &spec).await;
    let findings = |id: CheckId| -> Vec<String> {
        report
            .check(id)
            .unwrap_or_else(|| panic!("{id:?} reported"))
            .findings
            .iter()
            .map(|f| f.subject.clone())
            .collect()
    };
    let missing = findings(CheckId::TokenMissing);
    assert!(
        missing
            .iter()
            .any(|s| s.starts_with(&format!("{}@", spray::PHANTOM))),
        "the phantom holds an interface token and no instance token: {missing:?}"
    );
    let (owner, iface, _) = spray::SQUATTED;
    let foreign = findings(CheckId::StateStampForeign);
    assert!(
        foreign.contains(&format!("{owner} {iface}")),
        "the squatter answers {owner}'s {iface} state with another clock's stamp: {foreign:?}"
    );

    let _ = tool.close().await;
    let _ = raw.close().await;
    up.stop().await;
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

/// zengui's deployment on zk2, end to end against the spray (#614, FL6a):
/// the window boots for real — two sessions, the presence monitor on the
/// one in the namespace — then its own presence link (the stream its
/// subscription runs) seeds the projection and opens the lens, which
/// retrieves the spray's bundles into the window's store (§8.4). Selecting
/// a state key reads it by its owner's GET (S4; #614, FL8a), and the
/// Inspector shows the contract decode. No window, no endpoint named: the
/// spray on port 0.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_window_reads_the_spray_through_its_presence_link_and_lens() {
    use iced::futures::StreamExt as _;
    use zengui::message::{BusMsg, Message, Subject, SubjectMsg};

    panic_watchdog::install();
    // The boot re-reads the shared context file: a scratch one, never the
    // user's.
    let home = std::env::temp_dir().join("zengui-spray-live");
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("scratch dir");
    // SAFETY: set before any store call; the other test here reads no store.
    unsafe { std::env::set_var("ZENKEY_EXPLORER_CONFIG_DIR", &home) };

    let args = spray::Args {
        connect: vec![],
        listen: vec!["tcp/127.0.0.1:0".into()],
        namespace: NAMESPACE.into(),
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

    // The window, pointed at the spray's namespace with no contracts of its
    // own: every revision it decodes with is retrieved from the bus.
    let settings = zengui::config::Settings {
        namespace: NAMESPACE.into(),
        connect: vec![endpoint.clone()],
        listen: vec![],
        scouting: Some(false),
        zenoh_config: None,
        contracts: vec![],
        timeout_secs: 2,
        scope: zengui::scope::ScopePreset::Everything,
        selectors: vec![],
        eager: false,
        echo_lines: 100,
        history_entries: 10,
        max_keys: 1000,
    };
    let (mut app, boot) =
        zengui::app::Zengui::with_prefs(settings, zengui::prefs::Prefs::default(), None);
    tokio::time::timeout(SETTLE, drive(&mut app, boot))
        .await
        .expect("the boot settles");

    // The window's own presence link, run as its subscription would.
    let key = app
        .presence_link()
        .expect("two sessions and a presence monitor after the boot");
    let mut link = Box::pin(zengui::presence_link::stream(key));
    let nav: zenkey_model::grammar::Addr = "robot1/nav".parse().expect("an address");
    let deadline = tokio::time::Instant::now() + SETTLE;
    let mut lens_open = false;
    loop {
        let message = tokio::time::timeout_at(deadline, link.next())
            .await
            .expect("the link lands presence and the lens in time")
            .expect("the link runs");
        let resolves = matches!(
            &message,
            Message::Bus(BusMsg::Presence(_, u))
                if u.catalog.as_ref().is_some_and(|c| c.has_instance(&nav))
        );
        lens_open |= matches!(message, Message::Bus(BusMsg::LensOpened(..)));
        let _ = app.update(message);
        if resolves && lens_open {
            break;
        }
    }

    // The Inspector's chain on a state key (#614, FL8a): the selection's
    // own read — S4's owner GET on the session in the namespace — run as
    // the window's task, its row rendered through the revision the lens
    // retrieved from the spray's holders. A selection made before the lens
    // reached that revision reads nothing, and is made again.
    let key = format!("{NAMESPACE}/zk2/robot1/nav/nav.v2/state/status");
    let shows = |app: &zengui::app::Zengui, text: &str| {
        let mut ui = iced_test::simulator::<Message, _, _>(app.view(iced::window::Id::unique()));
        ui.find(text).is_ok()
    };
    let deadline = tokio::time::Instant::now() + SETTLE;
    while !shows(&app, "via owner GET (S4)") && tokio::time::Instant::now() < deadline {
        let read = app.update(Message::Subject(SubjectMsg::Select(Subject::Key(
            key.clone(),
        ))));
        tokio::time::timeout(SETTLE, drive(&mut app, read))
            .await
            .expect("the owner GET settles");
        if !shows(&app, "via owner GET (S4)") {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
    assert!(
        shows(&app, "via owner GET (S4)"),
        "a state key is read by its owner's GET"
    );
    assert!(
        shows(&app, "contract-decoded <nav.v2.Status>"),
        "the owner's reply renders through the revision retrieved from the \
         spray's holders"
    );
    assert!(shows(&app, "valid"), "and conforms to its declared type");

    // A metric of `lab/meter` (#819, TD): its kind and unit are its
    // contract's, retrieved from the spray's holders, and the Inspector
    // says them beside its type before a sample of it is recorded.
    let key = format!("{NAMESPACE}/zk2/{}/meter.v1/stream/bytes", spray::METER);
    let deadline = tokio::time::Instant::now() + SETTLE;
    while !shows(&app, "By · declared") && tokio::time::Instant::now() < deadline {
        let read = app.update(Message::Subject(SubjectMsg::Select(Subject::Key(
            key.clone(),
        ))));
        tokio::time::timeout(SETTLE, drive(&mut app, read))
            .await
            .expect("the selection settles");
        if !shows(&app, "By · declared") {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
    assert!(
        shows(&app, "By · declared"),
        "a bare counter's unit is its declaration's"
    );
    assert!(shows(&app, "counter · declared"));

    up.stop().await;
    let _ = std::fs::remove_dir_all(&home);
}
