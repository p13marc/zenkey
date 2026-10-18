//! The interop owner example (`examples/owner.rs`, #610) as a client of a
//! separate router (#660): what `state.md §1` and `presence.md §1` ask of an
//! owner a black-box scenario watches through R1.
//!
//! The example is compiled into this suite and run in process, through the
//! same `run` (and `run_with`, for its commands) its `main` calls, so the
//! suite needs no built binary. Its `health.v1` (#721) is here too: the
//! knobs and commands a black-box runner drives.

mod common;

#[allow(dead_code)]
#[path = "../examples/owner.rs"]
mod owner;

use std::path::Path;
use std::time::Duration;

use common::{SETTLE, T, client, eventually, router};
use zenkey::model::envelope;
use zenkey::model::grammar::{ZkKey, parse};
use zenkey::presence;
use zenoh::query::{ConsolidationMode, QueryTarget, Reply};
use zenoh::sample::{Sample, SampleKind};

async fn line(lines: &flume::Receiver<String>) -> String {
    tokio::time::timeout(SETTLE, lines.recv_async())
        .await
        .expect("a line from the owner")
        .expect("the owner is running")
}

async fn get(
    s: &zenoh::Session,
    key: &str,
    payload: &[u8],
    target: QueryTarget,
    c: ConsolidationMode,
) -> Vec<Reply> {
    let rx = s
        .get(key)
        .payload(payload.to_vec())
        .target(target)
        .consolidation(c)
        .timeout(T)
        .with(flume::unbounded::<Reply>())
        .await
        .unwrap();
    let mut out = Vec::new();
    while let Ok(r) = rx.recv_async().await {
        out.push(r);
    }
    out
}

/// `--connect <R1>`: the owner is a client of R1, with the two output lines
/// (`connected`, `ready`). Its state value was put before its tokens, so a
/// GET made the moment the interface token appears finds it (presence.md
/// §1, F-68), stamped with the owner's own zid, not R1's, where an
/// unstamped put through R1 carries R1's (state.md §1, F-69). Its
/// operations echo, or refuse `app` without a detail (F-65), templated
/// ones included (#670).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owner_example_as_a_client_of_a_separate_router() {
    let (r1, ep) = router(None).await;
    let tool = client(&ep).await;
    let tokens = tool
        .liveliness()
        .declare_subscriber("zk2/ex/owner/@zk/**")
        .history(true)
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();

    let contract = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/contracts/interop.v1.toml");
    let args: Vec<String> = vec![
        "--connect".into(),
        ep.clone(),
        "ex/owner".into(),
        contract.display().to_string(),
    ];
    let opts = owner::Options::parse(&args).unwrap();
    assert!(owner::Options::parse(&["--listen".into()]).is_err());
    let (say, lines) = flume::unbounded::<String>();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let running = tokio::spawn(async move {
        owner::run(
            opts,
            move |l| {
                let _ = say.send(l);
            },
            async {
                let _ = stopped.await;
            },
        )
        .await
        .map_err(|e| e.to_string())
    });
    assert_eq!(line(&lines).await, format!("connected {ep}"));

    // The moment the interface token appears: the state, with All + Latest.
    let state_key = "zk2/ex/owner/interop.v1/state/status";
    let (instance, stamp) = loop {
        let s = tokio::time::timeout(SETTLE, tokens.recv_async())
            .await
            .expect("a token")
            .unwrap();
        if s.kind() != SampleKind::Put {
            continue;
        }
        if let ZkKey::Alive { instance, .. } = parse(s.key_expr().as_str()).unwrap() {
            let replies = get(
                &tool,
                state_key,
                b"",
                QueryTarget::All,
                ConsolidationMode::Latest,
            )
            .await;
            let values: Vec<_> = replies.iter().filter_map(|r| r.result().ok()).collect();
            assert_eq!(values.len(), 1, "the value it started with is there");
            assert_eq!(&*values[0].payload().to_bytes(), b"ok");
            break (instance, *values[0].timestamp().expect("stamped (S2)"));
        }
    };
    let ready = line(&lines).await;
    assert_eq!(ready, format!("ready zk2/ex/owner/@zk/instance/{instance}"));

    // S1, observed through R1: the stamp is the owner's, not the router's.
    let d = presence::descriptor(&tool, &"ex/owner".parse().unwrap(), &instance, T)
        .await
        .unwrap()
        .into_descriptor()
        .unwrap();
    let owner_zid = d.meta["zid"].as_str().unwrap().to_owned();
    assert_eq!(stamp.get_id().to_string(), owner_zid);
    assert_ne!(
        owner_zid,
        r1.zid().to_string(),
        "a client, not its own router"
    );
    // The control: an unstamped put through R1 carries R1's zid.
    let third = client(&ep).await;
    let control = tool
        .declare_subscriber("ex/control")
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();
    let got = loop {
        third.put("ex/control", "x").await.unwrap();
        if let Ok(Ok(s)) =
            tokio::time::timeout(Duration::from_millis(100), control.recv_async()).await
        {
            break s;
        }
    };
    assert_eq!(
        got.timestamp().expect("R1 stamps it").get_id().to_string(),
        r1.zid().to_string()
    );

    // The operations: an echo, and `app` without a detail.
    let echo = get(
        &tool,
        "zk2/ex/owner/interop.v1/@op/echo",
        b"ping",
        QueryTarget::BestMatching,
        ConsolidationMode::None,
    )
    .await;
    let value = echo.iter().find_map(|r| r.result().ok()).expect("a value");
    assert_eq!(&*value.payload().to_bytes(), b"ping");
    let typed = get(
        &tool,
        "zk2/ex/owner/interop.v1/@op/typed",
        br#"{"up": true}"#,
        QueryTarget::BestMatching,
        ConsolidationMode::None,
    )
    .await;
    let err = typed
        .iter()
        .find_map(|r| r.result().err())
        .expect("a refusal");
    assert_eq!(err.encoding().to_string(), envelope::JSON);
    let env = envelope::decode(envelope::JSON, &err.payload().to_bytes()).unwrap();
    assert_eq!((env.code.as_str(), env.detail), ("app", None));

    // A templated operation is served too (#670, §8.2 "Exposed"): a call
    // whose key binds the parameter is echoed on that member's key; a
    // fan-out that leaves it unbound names no member, so it is refused
    // `internal`, never silent (§5.1).
    let member = "zk2/ex/owner/interop.v1/@op/ports/p1/echo";
    let one = get(
        &tool,
        member,
        b"pong",
        QueryTarget::BestMatching,
        ConsolidationMode::None,
    )
    .await;
    let value = one.iter().find_map(|r| r.result().ok()).expect("a value");
    assert_eq!(value.key_expr().as_str(), member);
    assert_eq!(&*value.payload().to_bytes(), b"pong");
    let fan = get(
        &tool,
        "zk2/ex/owner/interop.v1/@op/ports/*/echo",
        b"pong",
        QueryTarget::All,
        ConsolidationMode::None,
    )
    .await;
    let err = fan.iter().find_map(|r| r.result().err()).expect("answered");
    let env = envelope::decode(&err.encoding().to_string(), &err.payload().to_bytes()).unwrap();
    assert_eq!(env.code, "internal", "{env:?}");

    stop.send(()).unwrap();
    running.await.unwrap().unwrap();
}

/// `@hostid.v1/<service>` (#719): the system is minted under
/// `--hostid-root` before the session opens (hostid.v1 §2.7), and a root
/// without an id stops the owner before it says anything (§2.6).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owner_example_with_a_minted_system() {
    let (_r1, ep) = router(None).await;
    let contract = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/contracts/interop.v1.toml");
    let root = tempfile::tempdir().unwrap();
    let args = |root: &Path| -> Vec<String> {
        vec![
            "--connect".into(),
            ep.clone(),
            "--hostid-root".into(),
            root.display().to_string(),
            "@hostid.v1/owner".into(),
            contract.display().to_string(),
        ]
    };

    // No id under the root, and a shared file that holds none: refused.
    std::fs::create_dir_all(root.path().join("var/lib/zk2")).unwrap();
    std::fs::write(root.path().join("var/lib/zk2/hostid"), "garbage\n").unwrap();
    let (say, lines) = flume::unbounded::<String>();
    let e = owner::run(
        owner::Options::parse(&args(root.path())).unwrap(),
        move |l| {
            let _ = say.send(l);
        },
        async {},
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(e.contains("/var/lib/zk2/hostid: refused"), "{e}");
    assert!(lines.try_recv().is_err(), "nothing said: no session opened");

    // M1 under the root: the owner comes up at its minted system.
    std::fs::create_dir_all(root.path().join("etc")).unwrap();
    std::fs::write(
        root.path().join("etc/machine-id"),
        "b642b4217b34b1e8d3bd915fc65c4452\n",
    )
    .unwrap();
    let (say, lines) = flume::unbounded::<String>();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let opts = owner::Options::parse(&args(root.path())).unwrap();
    let running = tokio::spawn(async move {
        owner::run(
            opts,
            move |l| {
                let _ = say.send(l);
            },
            async {
                let _ = stopped.await;
            },
        )
        .await
        .map_err(|e| e.to_string())
    });
    assert_eq!(line(&lines).await, format!("connected {ep}"));
    let ready = line(&lines).await;
    assert!(
        ready.starts_with("ready zk2/h-bbd1aa1db10b/owner/@zk/instance/"),
        "{ready}"
    );
    stop.send(()).unwrap();
    running.await.unwrap().unwrap();
}

/// A running owner example: its output lines, its command sender, its
/// stop, and its task.
struct Running {
    lines: flume::Receiver<String>,
    send: flume::Sender<String>,
    stop: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<Result<(), String>>,
}

/// Starts the owner example with `args`, through `run_with`.
fn spawn_owner(args: &[&str]) -> Running {
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    let opts = owner::Options::parse(&args).unwrap();
    let (say, lines) = flume::unbounded::<String>();
    let (send, commands) = flume::unbounded::<String>();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        owner::run_with(
            opts,
            move |l| {
                let _ = say.send(l);
            },
            commands,
            async {
                let _ = stopped.await;
            },
        )
        .await
        .map_err(|e| e.to_string())
    });
    Running {
        lines,
        send,
        stop,
        task,
    }
}

impl Running {
    /// The owner's answer to one command line.
    async fn ask(&self, cmd: &str) -> String {
        self.send.send(cmd.to_owned()).unwrap();
        line(&self.lines).await
    }

    async fn stop(self) {
        self.stop.send(()).unwrap();
        self.task.await.unwrap().unwrap();
    }
}

/// The payloads of a state GET (All + Latest).
async fn state(tool: &zenoh::Session, key: &str) -> Vec<Vec<u8>> {
    get(tool, key, b"", QueryTarget::All, ConsolidationMode::Latest)
        .await
        .into_iter()
        .filter_map(|r| r.into_result().ok())
        .map(|s| s.payload().to_bytes().into_owned())
        .collect()
}

/// `health.v1` in the owner example (#721): the standard contract among
/// the files is the runtime's to implement, the status and checks given on
/// the command line are put before the tokens, and the commands move them,
/// each answered with what is published.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owner_example_implements_health() {
    use zenkey::health::v1;
    use zenkey::prost::Message as _;

    let (_r1, ep) = router(None).await;
    let tool = client(&ep).await;
    let faults = tool
        .declare_subscriber("zk2/ex/healthy/health.v1/stream/faults")
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();
    let standard =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/profiles/health/health.v1.toml");
    let standard = standard.display().to_string();
    let o = spawn_owner(&[
        "--connect",
        &ep,
        "--health-status",
        "OK:serving",
        "--health-check",
        "disk=OK:70% used",
        "--tokenless",
        "health.v1",
        "ex/healthy",
        &standard,
    ]);
    assert_eq!(line(&o.lines).await, format!("connected {ep}"));
    let ready = line(&o.lines).await;
    let instance = ready.rsplit('/').next().unwrap().parse().unwrap();
    // `ready` comes before the declarations reach R1: wait for the
    // descriptor, which follows the state queryables (§8.2).
    let addr = "ex/healthy".parse().unwrap();
    let d = loop {
        match presence::descriptor(&tool, &addr, &instance, T)
            .await
            .unwrap()
        {
            presence::Found::Nothing => continue,
            found => break found.into_descriptor().unwrap(),
        }
    };
    let e = d
        .interfaces
        .iter()
        .find(|e| e.iface == "health.v1")
        .unwrap();
    assert_eq!(
        e.contract,
        format!("sha256:{}", zenkey::health::FINGERPRINT)
    );
    assert!(!e.token, "in the tokenless set");
    let status = state(&tool, "zk2/ex/healthy/health.v1/state/status").await;
    let s = v1::Status::decode(status[0].as_slice()).unwrap();
    assert_eq!((s.level, s.reason.as_str()), (1, "serving"));

    assert_eq!(
        o.ask("check disk FAILED full").await,
        "health status=FAILED declared=OK raised_by=disk"
    );
    let check = state(&tool, "zk2/ex/healthy/health.v1/state/checks/disk").await;
    assert_eq!(v1::Check::decode(check[0].as_slice()).unwrap().level, 3);
    assert_eq!(
        o.ask("fault ex_restart DEGRADED the poller restarted")
            .await,
        "health status=FAILED declared=OK raised_by=disk"
    );
    let f = tokio::time::timeout(SETTLE, faults.recv_async())
        .await
        .unwrap()
        .unwrap();
    let f = v1::Fault::decode(&*f.payload().to_bytes()).unwrap();
    assert_eq!(
        (f.code.as_str(), f.level, f.detail.as_str()),
        ("ex_restart", 2, "the poller restarted")
    );
    let refused = o.ask("fault clock_ahead FAILED x").await;
    assert!(
        refused.starts_with("refused fault clock_ahead FAILED x: clock_ahead is a code of"),
        "{refused}"
    );
    let refused = o.ask("fault Bad FAILED x").await;
    assert!(
        refused.starts_with("refused fault Bad FAILED x: fault code \"Bad\""),
        "{refused}"
    );
    assert!(o.ask("status UNSPECIFIED").await.starts_with("refused"));
    assert_eq!(
        o.ask("retire disk").await,
        "health status=OK declared=OK raised_by=-"
    );
    assert_eq!(
        o.ask("check probe UNSPECIFIED cannot reach it").await,
        "health status=OK declared=OK raised_by=-"
    );
    assert_eq!(
        o.ask("confirm off").await,
        "health status=OK declared=OK raised_by=-"
    );
    assert!(o.ask("nonsense").await.starts_with("refused nonsense"));
    o.stop().await;
}

/// `--clock-reference` and `--clock-offset-ms` (health.v1 §2.5): an owner
/// 2 s ahead publishes `clock_ahead` once its guard holds, and R1 re-stamps
/// it, the session's HLC being left off under a simulated offset; the
/// `clock-offset` command sets the clock right.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owner_example_reports_a_clock_ahead() {
    use zenkey::health::v1;
    use zenkey::prost::Message as _;

    let (r1, ep) = router(None).await;
    let tool = client(&ep).await;
    let faults = tool
        .declare_subscriber("zk2/ex/ahead/health.v1/stream/faults")
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();
    let o = spawn_owner(&[
        "--connect",
        &ep,
        "--health",
        "--clock-reference",
        "clock/heartbeat",
        "--clock-offset-ms",
        "2000",
        "ex/ahead",
    ]);
    assert_eq!(line(&o.lines).await, format!("connected {ep}"));
    let ready = line(&o.lines).await;
    assert!(
        ready.starts_with("ready zk2/ex/ahead/@zk/instance/"),
        "{ready}"
    );
    let fault = loop {
        tool.put("clock/heartbeat", "tick").await.unwrap();
        if let Ok(Ok(s)) =
            tokio::time::timeout(Duration::from_millis(100), faults.recv_async()).await
        {
            break s;
        }
    };
    let f = v1::Fault::decode(&*fault.payload().to_bytes()).unwrap();
    assert_eq!((f.code.as_str(), f.level), ("clock_ahead", 3));
    assert_eq!(
        fault.timestamp().unwrap().get_id().to_string(),
        r1.zid().to_string(),
        "re-stamped by R1"
    );
    assert_eq!(o.ask("clock-offset 0").await, "clock-offset 0");
    o.stop().await;
}

/// The knobs that reach each bound (core §2.7, #741; `budget.md` §4–§7):
/// `--cardinality` lowers a state's bound in the descriptor, and `fill`,
/// `put` and `delete` drive the runtime's refusals, each answered with the
/// key written or the refusal naming the bound and whose it is.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owner_example_reaches_each_bound() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("budget.v1.toml");
    std::fs::write(
        &file,
        "[interface]\nname = \"budget\"\nmajor = 1\n\
         [resources.\"items/{item}\"]\nkind = \"state\"\ntype = { raw = \"text/plain\" }\n\
         params = { item = \"string\" }\ncardinality = 3\n\
         [resources.\"load/{cpu}\"]\nkind = \"stream\"\ntype = { raw = \"text/plain\" }\n\
         params = { cpu = \"string\" }\ncardinality = 3\nreliability = \"reliable\"\n\
         congestion = \"block\"\n\
         [resources.\"alarms/{src}\"]\nkind = \"event\"\ntype = { raw = \"text/plain\" }\n\
         params = { src = \"string\" }\ncardinality = 2\nrate = \"burst(3/h)\"\n\
         retention = \"2s\"\n",
    )
    .unwrap();
    let file = file.display().to_string();
    let (_r1, ep) = router(None).await;
    let o = spawn_owner(&[
        "--connect",
        &ep,
        "--cardinality",
        "budget.v1:state/items/{item}=2",
        "ex/budget",
        &file,
    ]);
    assert_eq!(line(&o.lines).await, format!("connected {ep}"));
    assert!(line(&o.lines).await.starts_with("ready "));
    const KEY: &str = "zk2/ex/budget/budget.v1";
    // §4, the descriptor's bound of 2.
    let refused = o.ask("fill budget.v1 state/items/{item} 3").await;
    assert!(
        refused.starts_with("refused fill budget.v1 state/items/{item} 3: 2 put, then: ")
            && refused.contains("at its bound of 2 (the descriptor's)"),
        "{refused}"
    );
    assert_eq!(
        o.ask("put budget.v1 state/items/{item} m0").await,
        format!("put {KEY}/state/items/m0"),
        "a live key adds no member"
    );
    assert_eq!(
        o.ask("delete budget.v1 state/items/{item} m1").await,
        format!("deleted {KEY}/state/items/m1")
    );
    assert_eq!(
        o.ask("put budget.v1 state/items/{item} m2").await,
        format!("put {KEY}/state/items/m2"),
        "a delete frees one"
    );
    // §5, the contract's bound of 3.
    assert_eq!(
        o.ask("fill budget.v1 stream/load/{cpu} 3").await,
        "filled 3"
    );
    let refused = o.ask("put budget.v1 stream/load/{cpu} c9").await;
    assert!(
        refused.contains("at its bound of 3 (the contract's)"),
        "{refused}"
    );
    assert!(
        o.ask("delete budget.v1 stream/load/{cpu} m0")
            .await
            .contains("only a state is deleted")
    );
    // §6 and §7: burst(3/h) per source, two sources live at once.
    for _ in 0..3 {
        let put = o.ask("put budget.v1 events/alarms/{src} a").await;
        assert!(
            put.starts_with(&format!("put {KEY}/events/alarms/a/")),
            "{put}"
        );
    }
    let refused = o.ask("put budget.v1 events/alarms/{src} a").await;
    assert!(refused.contains("beyond its rate burst(3/h)"), "{refused}");
    assert!(
        o.ask("put budget.v1 events/alarms/{src} b")
            .await
            .starts_with("put ")
    );
    let refused = o.ask("put budget.v1 events/alarms/{src} c").await;
    assert!(
        refused.contains("at its bound of 2 (the contract's)"),
        "{refused}"
    );
    assert!(
        o.ask("put budget.v1 events/alarms/{src}")
            .await
            .contains("takes 1 value(s)")
    );
    o.stop().await;
}

/// `config.v1` in the owner example (#768, CB): the standard contract
/// among the files is the runtime's to implement, on `rf` and a contract
/// group; the commands a black-box runner drives make the device refuse,
/// restart the owner as a new instance, and give it a desired document.
/// `--config-no-persist` lists `persist` unavailable, cause `config`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owner_example_implements_config() {
    use zenkey::config_v1::{self, ConfigView};
    use zenkey::{CallMetadata, Client, Outcome};

    let (_r1, ep) = router(None).await;
    let tool = client(&ep).await;
    let standard =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/profiles/config/config.v1.toml");
    let standard = standard.display().to_string();
    let o = spawn_owner(&["--connect", &ep, "--config-floor", "5", "ex/rf", &standard]);
    assert_eq!(line(&o.lines).await, format!("connected {ep}"));
    let ready = line(&o.lines).await;
    let first: zenkey::model::grammar::InstanceId =
        ready.rsplit('/').next().unwrap().parse().unwrap();
    let addr: zenkey::model::grammar::Addr = "ex/rf".parse().unwrap();
    let descriptor = |instance: zenkey::model::grammar::InstanceId, addr| {
        let tool = tool.clone();
        async move {
            loop {
                match presence::descriptor(&tool, &addr, &instance, T)
                    .await
                    .unwrap()
                {
                    presence::Found::Nothing => continue,
                    found => break found.into_descriptor().unwrap(),
                }
            }
        }
    };
    let d = descriptor(first.clone(), addr.clone()).await;
    let e = d
        .interfaces
        .iter()
        .find(|e| e.iface == "config.v1")
        .unwrap();
    assert_eq!(e.contract, format!("sha256:{}", config_v1::FINGERPRINT));
    assert!(e.unavailable.is_empty(), "persist is served");
    let view = || async {
        let v = state(&tool, "zk2/ex/rf/config.v1/state/view").await;
        serde_json::from_slice::<ConfigView>(&v[0]).unwrap()
    };
    let v = view().await;
    let groups: Vec<(&str, &str)> = v
        .groups
        .iter()
        .map(|g| (g.name.as_str(), g.class.as_str()))
        .collect();
    assert_eq!(
        groups,
        [
            ("duty", "hot"),
            ("radio", "reach"),
            ("link", "reach"),
            ("sim", "hot"),
            ("queue", "hot"),
            ("modulation", "contract")
        ]
    );
    assert_eq!(v.confirm_floor_s, 5);

    let c = Client::new(
        &tool,
        config_v1::implementation().shared_contract(),
        &["ex/rf"],
    )
    .unwrap()
    .with_timeout(Duration::from_secs(2))
    .with_metadata(CallMetadata::new("ops", "r1"));
    let set = |group: &str, body: serde_json::Value| {
        let c = c.clone();
        let addr = addr.clone();
        let values: zenkey::model::template::Bindings =
            [("group".to_owned(), vec![group.to_owned()])].into();
        async move {
            c.call_value(&addr, config_v1::SET, &values, &body)
                .await
                .unwrap()
        }
    };
    let refusal = |o: &Outcome| o.refusal().cloned().expect("a refusal");

    // The contract group: restart-required.
    let r = set(
        "modulation",
        serde_json::json!({"values": {"spreading_factor": 10}}),
    )
    .await;
    let e = refusal(&r);
    assert_eq!(e.code, "app");
    assert_eq!(
        e.detail,
        Some(envelope::Detail::Value(
            serde_json::json!({"code": "restart-required"})
        ))
    );

    // The device told to refuse its next apply, in its words.
    let held = o.ask("config fail-apply no carrier").await;
    assert!(
        held.starts_with("config device held=") && held.ends_with("next=fail-apply"),
        "{held}"
    );
    let r = set(
        "duty",
        serde_json::json!({"values": {"fraction_ppm": 20000}}),
    )
    .await;
    let e = refusal(&r);
    assert_eq!(
        e.detail,
        Some(envelope::Detail::Value(
            serde_json::json!({"code": "device-refused", "detail": "no carrier"})
        ))
    );
    let r = set(
        "duty",
        serde_json::json!({"values": {"fraction_ppm": 20000}}),
    )
    .await;
    assert!(r.answer().is_some(), "{r:?}");
    let device = o.ask("config device").await;
    assert!(
        device.contains(r#""fraction_ppm":20000"#) && device.contains("applies=1"),
        "{device}"
    );

    // A restart: a new instance, from the startup values.
    let restarted = o.ask("config restart").await;
    let next: zenkey::model::grammar::InstanceId = restarted
        .strip_prefix("config restarted zk2/ex/rf/@zk/instance/")
        .unwrap_or_else(|| panic!("{restarted}"))
        .parse()
        .unwrap();
    assert_ne!(next, first);
    descriptor(next.clone(), addr.clone()).await;
    let v = view().await;
    assert_eq!((v.instance, v.revision), (next.to_string(), 0));
    let duty = &v.groups[0].parameters[0];
    assert_eq!(
        (duty.value.clone(), duty.source),
        (
            Some(zenkey::config_v1::Value::Integer(10000)),
            Some(zenkey::config_v1::Source::File)
        )
    );

    // A desired document, converged; the clock shifted.
    assert_eq!(
        o.ask(r#"config desired {"revision": 1, "values": {"duty": {"fraction_ppm": 30000}}}"#)
            .await,
        "config desired 1"
    );
    eventually("the document converged", || async {
        view().await.desired.is_some_and(|d| {
            d.observed_revision == 1
                && d.outcome == zenkey::model::config::DesiredOutcome::Converged
        })
    })
    .await;
    // F-114 (text 0.3): the binding outlives a restart, and the new
    // instance converges the same revision again: its converged value was
    // a runtime change the restart rolled back.
    let restarted = o.ask("config restart").await;
    let third: zenkey::model::grammar::InstanceId = restarted
        .strip_prefix("config restarted zk2/ex/rf/@zk/instance/")
        .unwrap_or_else(|| panic!("{restarted}"))
        .parse()
        .unwrap();
    eventually("the bound document converged again", || async {
        let v = view().await;
        v.instance == third.to_string()
            && v.revision == 1
            && v.groups[0].parameters[0].value == Some(zenkey::config_v1::Value::Integer(30000))
            && v.desired.is_some_and(|d| {
                d.observed_revision == 1
                    && d.outcome == zenkey::model::config::DesiredOutcome::Converged
            })
    })
    .await;
    assert_eq!(o.ask("config offset 1000").await, "config offset 1000");
    assert!(
        o.ask("config nope")
            .await
            .starts_with("refused config nope: usage")
    );
    o.stop().await;

    // Without a persisted layer: persist unavailable, cause config.
    let o = spawn_owner(&["--connect", &ep, "--config-no-persist", "ex/rf2"]);
    assert_eq!(line(&o.lines).await, format!("connected {ep}"));
    let ready = line(&o.lines).await;
    let instance = ready.rsplit('/').next().unwrap().parse().unwrap();
    let d = descriptor(instance, "ex/rf2".parse().unwrap()).await;
    let e = d
        .interfaces
        .iter()
        .find(|e| e.iface == "config.v1")
        .unwrap();
    let listed: Vec<&str> = e.unavailable.iter().map(|u| u.resource.as_str()).collect();
    assert_eq!(listed, ["@op/persist"]);
    o.stop().await;
}
