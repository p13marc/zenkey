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

/// `alarms.v1` in the owner example (#800, AB): the standard contract
/// among the files is the runtime's to implement; the commands a black-box
/// runner drives raise, escalate, refresh, resolve and stop confirming, a
/// raise past the lowered bound is refused, and a restart on the same
/// `--alarms-record` takes the alarms over, resolving a rule it no longer
/// evaluates and deleting a phantom.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owner_example_implements_alarms() {
    use zenkey::model::alarms as am;

    let (_r1, ep) = router(None).await;
    let tool = client(&ep).await;
    let standard =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/profiles/alarms/alarms.v1.toml");
    let standard = standard.display().to_string();
    let dir = tempfile::tempdir().unwrap();
    let record = dir.path().join("alarms.json");
    let record_arg = record.display().to_string();
    let o = spawn_owner(&[
        "--connect",
        &ep,
        "--alarms-record",
        &record_arg,
        "--cardinality",
        "alarms.v1:state/alarms/{alert_key}=2",
        "ex/alarmed",
        &standard,
    ]);
    assert_eq!(line(&o.lines).await, format!("connected {ep}"));
    let ready = line(&o.lines).await;
    let instance = ready.rsplit('/').next().unwrap().parse().unwrap();
    let addr = "ex/alarmed".parse().unwrap();
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
        .find(|e| e.iface == "alarms.v1")
        .unwrap();
    assert_eq!(
        e.contract,
        format!("sha256:{}", zenkey::alarms::FINGERPRINT)
    );
    assert_eq!(e.cardinality.get(zenkey::alarms::ALARMS), Some(&2));
    assert_eq!(
        o.ask("alarm adoption").await,
        "alarm adoption source=nothing adopted=- retired=- resolved=- over_bound=-"
    );

    let a = r#"{"rule": "link_down", "severity": "warning", "summary": "down", "labels": {"peer": "r2", "port": "eth0"}}"#;
    let raised = o.ask(&format!("alarm raise {a}")).await;
    assert!(
        raised.starts_with("alarm raised a659f813308ad1da {"),
        "{raised}"
    );
    let key = "zk2/ex/alarmed/alarms.v1/state/alarms/a659f813308ad1da";
    let got = state(&tool, key).await;
    let doc = am::decode(&got[0]).unwrap();
    assert_eq!(
        (doc.state, doc.severity),
        (am::State::Firing, am::Severity::Warning)
    );
    let critical = a.replace("warning", "critical");
    assert!(
        o.ask(&format!("alarm raise {critical}"))
            .await
            .starts_with("alarm escalated a659f813308ad1da ")
    );
    let refreshed = critical.replace("\"down\"", "\"down, 3 retries\"");
    let r = o.ask(&format!("alarm raise {refreshed}")).await;
    assert!(r.starts_with("alarm refreshed a659f813308ad1da "), "{r}");
    let doc: am::Alarm = serde_json::from_str(r.splitn(4, ' ').nth(3).unwrap()).unwrap();
    assert!(doc.observed_at_ms.is_some_and(|o| o >= doc.timestamp));
    assert!(
        o.ask(&format!("alarm raise {refreshed}"))
            .await
            .starts_with("alarm unchanged a659f813308ad1da ")
    );
    // The JSON is the rest of the line as written: two spaces stay two.
    let spaced = critical.replace("\"down\"", "\"down  twice\"");
    let r = o.ask(&format!("alarm  raise   {spaced}")).await;
    assert!(r.starts_with("alarm refreshed a659f813308ad1da "), "{r}");
    let doc: am::Alarm = serde_json::from_str(r.splitn(4, ' ').nth(3).unwrap()).unwrap();
    assert_eq!(doc.summary, "down  twice");
    let got = am::decode(&state(&tool, key).await[0]).unwrap();
    assert_eq!(got.summary, "down  twice", "as put");
    let b = r#"{"rule": "disk_full", "severity": "warning", "summary": "full", "labels": {"mount": "/var"}}"#;
    let raised_b = o.ask(&format!("alarm raise {b}")).await;
    let b_key = raised_b.split(' ').nth(2).unwrap().to_owned();
    assert!(raised_b.starts_with("alarm raised "), "{raised_b}");
    let c = r#"{"rule": "link_down", "severity": "warning", "summary": "down", "labels": {"peer": "r3", "port": "eth1"}}"#;
    let refused = o.ask(&format!("alarm raise {c}")).await;
    assert!(
        refused.starts_with("refused alarm raise")
            && refused.contains("at its bound of 2 (the descriptor's)"),
        "{refused}"
    );
    assert_eq!(
        o.ask("alarm confirm a659f813308ad1da off").await,
        "alarm confirm a659f813308ad1da off"
    );
    assert_eq!(
        o.ask("alarm confirm 0000000000000000 off").await,
        "refused alarm confirm 0000000000000000 off: no alarm is held on 0000000000000000"
    );
    assert_eq!(o.ask("alarm confirm all on").await, "alarm confirm all on");
    let held = o.ask("alarm list").await;
    let held: serde_json::Value =
        serde_json::from_str(held.strip_prefix("alarm held ").unwrap()).unwrap();
    assert_eq!(held.as_object().unwrap().len(), 2);
    let resolved = o.ask(&format!("alarm resolve {b_key}")).await;
    assert!(
        resolved.starts_with(&format!("alarm resolved {b_key} {{"))
            && resolved.contains("\"resolved\""),
        "{resolved}"
    );
    assert_eq!(
        o.ask(&format!("alarm resolve {b_key}")).await,
        format!("alarm none {b_key}")
    );
    assert!(
        o.ask("alarm raise nope")
            .await
            .starts_with("refused alarm raise nope: not a condition")
    );
    assert!(
        o.ask("alarm nope")
            .await
            .starts_with("refused alarm nope: usage")
    );
    o.stop().await;

    // The clean close left A; a phantom written into the record; a restart
    // that evaluates only disk_full.
    let mut file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&record).unwrap()).unwrap();
    assert_eq!(
        file["alarms"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["a659f813308ad1da"]
    );
    file["alarms"]["0000000000000001"] = file["alarms"]["a659f813308ad1da"].clone();
    std::fs::write(&record, serde_json::to_vec(&file).unwrap()).unwrap();
    let o = spawn_owner(&[
        "--connect",
        &ep,
        "--alarms-record",
        &record_arg,
        "--alarms-rules",
        "disk_full",
        "ex/alarmed",
        &standard,
    ]);
    assert_eq!(line(&o.lines).await, format!("connected {ep}"));
    line(&o.lines).await;
    assert_eq!(
        o.ask("alarm adoption").await,
        "alarm adoption source=record adopted=- retired=0000000000000001:phantom \
         resolved=a659f813308ad1da over_bound=-"
    );
    assert_eq!(o.ask("alarm list").await, "alarm held {}");
    o.stop().await;

    // Without --alarms, an alarm command is refused.
    let o = spawn_owner(&["--connect", &ep, "ex/plain"]);
    assert_eq!(line(&o.lines).await, format!("connected {ep}"));
    line(&o.lines).await;
    assert!(
        o.ask("alarm list")
            .await
            .starts_with("refused alarm list: alarms.v1 is not implemented here")
    );
    o.stop().await;
}

/// `owner --help` (#800, AC; zk2py's F-133) documents the interface a live
/// runner drives: the command word `alarm` and each of its commands, the
/// condition's JSON, every answer line, the record file's format, and that
/// a short-horizon copy of the standard contract is accepted.
#[test]
fn the_help_documents_the_alarms_surface() {
    for piece in [
        "alarm raise <json>",
        "alarm resolve <alert_key>",
        "alarm confirm <alert_key>|all on|off",
        "alarm list",
        "alarm adoption",
        "\"rule\": <string>",
        "\"severity\": \"info\"|\"warning\"|\"critical\"",
        "\"labels\"?",
        "\"category\"?",
        "alarm raised|escalated|refreshed|unchanged <alert_key> <document>",
        "alarm resolved <alert_key> <document>, or alarm none <alert_key>",
        "alarm held <json>",
        "alarm adoption source=<record|archive|nothing>",
        "refused <line>: <why>",
        "{\"profile\": \"alarms.v1\", \"alarms\": {\"<alert key>\": <document>, ...}}",
        "short\n  horizon is accepted",
        "--alarms-record <file>",
        "--alarms-archive",
        "--alarms-rules",
    ] {
        assert!(owner::HELP.contains(piece), "--help names {piece:?}");
    }
}

/// `telemetry.v1` in the owner example (#819, TB): `--telemetry` implements
/// the scenarios' `meter.v1`, embedded, its fingerprint the test file's; a
/// put goes through the member's checked writer, which writes the epoch
/// and answers a refusal with its reason, nothing published; `raw` puts
/// what the writer refuses, as written; `new-epoch` moves the epoch.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owner_example_implements_telemetry() {
    let (_r1, ep) = router(None).await;
    let tool = client(&ep).await;
    let data = tool
        .declare_subscriber("zk2/ex/meter/meter.v1/stream/*")
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();
    let next = || async {
        let s = tokio::time::timeout(SETTLE, data.recv_async())
            .await
            .expect("a sample")
            .unwrap();
        let attachment = s.attachment().map(|a| a.to_bytes().into_owned());
        (
            s.key_expr().as_str().to_owned(),
            s.payload().to_bytes().into_owned(),
            attachment,
        )
    };
    let o = spawn_owner(&["--connect", &ep, "--telemetry", "ex/meter"]);
    assert_eq!(line(&o.lines).await, format!("connected {ep}"));
    let ready = line(&o.lines).await;
    let i1 = ready.rsplit('/').next().unwrap().to_owned();
    assert_eq!(
        o.ask("telemetry instance").await,
        format!("telemetry instance {i1}")
    );
    // The embedded meter.v1 is the test file's, fingerprint and all.
    // `ready` comes before the declarations reach R1: wait for the
    // descriptor, which follows the state queryables (§8.2).
    let (addr, instance) = ("ex/meter".parse().unwrap(), i1.parse().unwrap());
    let d = loop {
        match presence::descriptor(&tool, &addr, &instance, T)
            .await
            .unwrap()
        {
            presence::Found::Nothing => continue,
            found => break found.into_descriptor().unwrap(),
        }
    };
    let file = zenkey::Implementation::new(common::contract("meter.v1"));
    let e = d.interfaces.iter().find(|e| e.iface == "meter.v1").unwrap();
    assert_eq!(e.contract, format!("sha256:{}", file.fingerprint().hex()));
    tokio::time::sleep(Duration::from_millis(500)).await;

    let frames = "zk2/ex/meter/meter.v1/stream/frames";
    let counter = |ts: u64, n: u64| {
        format!(r#"{{"timestamp": {ts}, "value": {{"type": "counter", "value": {n}}}}}"#)
    };
    assert_eq!(
        o.ask(&format!("telemetry put frames {}", counter(1, 10)))
            .await,
        format!("telemetry put {frames} epoch={i1}")
    );
    let (key, payload, _) = next().await;
    assert_eq!(key, frames);
    let p: serde_json::Value = serde_json::from_slice(&payload).unwrap();
    assert_eq!(p["epoch"], serde_json::json!(i1), "the writer wrote it");

    // Refusals: a decrease, a tag that disagrees; nothing published.
    let down = o
        .ask(&format!("telemetry put stream/frames {}", counter(2, 7)))
        .await;
    assert!(
        down.starts_with(&format!(
            "refused telemetry put stream/frames {}: telemetry.v1 decreased epoch={i1}: ",
            counter(2, 7)
        )),
        "{down}"
    );
    let gauge = r#"{"timestamp": 3, "value": {"type": "gauge", "value": 3.5}}"#;
    let tag = o
        .ask(&format!("telemetry put meter.v1:frames {gauge}"))
        .await;
    assert!(
        tag.contains(": telemetry.v1 does_not tag_disagrees: "),
        "{tag}"
    );
    // Raw: the bytes as written, from the session.
    let raw = format!(
        r#"{{"timestamp": 2, "value": {{"type": "counter", "value": 7}}, "epoch": "{i1}"}}"#
    );
    assert_eq!(
        o.ask(&format!("telemetry raw frames {raw}")).await,
        format!("telemetry raw {frames}")
    );
    let (_, payload, _) = next().await;
    assert_eq!(
        payload,
        raw.as_bytes(),
        "the refused ones published nothing"
    );

    // A bare counter's epoch rides its attachment; an empty one is refused.
    let bytes = "zk2/ex/meter/meter.v1/stream/bytes";
    assert_eq!(
        o.ask("telemetry put bytes 100").await,
        format!("telemetry put {bytes} epoch={i1}")
    );
    let (_, payload, attachment) = next().await;
    assert_eq!(
        (payload, attachment),
        (b"100".to_vec(), Some(format!("\"{i1}\"").into_bytes()))
    );
    let empty = o.ask(r#"telemetry put bytes 60 """#).await;
    assert!(
        empty.contains(": telemetry.v1 does_not epoch_invalid: "),
        "{empty}"
    );
    let other = o.ask(r#"telemetry put bytes 60 "i9""#).await;
    assert!(
        other.contains(&format!(": telemetry.v1 epoch given=\"i9\" current={i1}: ")),
        "{other}"
    );
    assert_eq!(
        o.ask(r#"telemetry raw bytes 60 """#).await,
        format!("telemetry raw {bytes}")
    );
    let (_, payload, attachment) = next().await;
    assert_eq!(
        (payload, attachment),
        (b"60".to_vec(), Some(b"\"\"".to_vec()))
    );

    // A new epoch: the next Point carries it, and starts again.
    let moved = o.ask("telemetry new-epoch").await;
    let i2 = moved
        .strip_prefix("telemetry new-epoch ")
        .unwrap()
        .to_owned();
    assert_ne!(i1, i2);
    assert_eq!(
        o.ask("telemetry instance").await,
        format!("telemetry instance {i2}")
    );
    assert_eq!(
        o.ask(&format!("telemetry put frames {}", counter(5, 0)))
            .await,
        format!("telemetry put {frames} epoch={i2}")
    );
    let (_, payload, _) = next().await;
    let p: serde_json::Value = serde_json::from_slice(&payload).unwrap();
    assert_eq!(p["epoch"], serde_json::json!(i2));

    // The state counter (§9, text 0.2) puts through its state writer.
    assert_eq!(
        o.ask("telemetry put total 3").await,
        format!("telemetry put zk2/ex/meter/meter.v1/state/total epoch={i2}")
    );

    // What is refused here, before any writer.
    for (cmd, why) in [
        (
            "telemetry put nope 1",
            "no resource \"nope\" is implemented here",
        ),
        ("telemetry put frames {", "the payload is not JSON"),
        (
            "telemetry put frames 1 2 3",
            "the attachment is not one JSON text",
        ),
        ("telemetry nope", "usage: telemetry put"),
    ] {
        let a = o.ask(cmd).await;
        assert!(a.starts_with(&format!("refused {cmd}: {why}")), "{a}");
    }
    o.stop().await;

    // Without --telemetry, a resource that is no metric is named so.
    let contract = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/contracts/interop.v1.toml");
    let o = spawn_owner(&["ex/interop", &contract.display().to_string()]);
    line(&o.lines).await;
    line(&o.lines).await;
    let a = o.ask("telemetry put state/status 1").await;
    assert!(
        a.contains(
            "is not a metric resource (telemetry.v1 §2.1): its declaration is not_this_profiles"
        ),
        "{a}"
    );
    o.stop().await;
}

/// `owner --help` documents the `telemetry` surface (#819, TB).
#[test]
fn the_help_documents_the_telemetry_surface() {
    for piece in [
        "--telemetry",
        "telemetry put <resource> <json> [<attachment json>]",
        "telemetry raw <resource> <json> [<attachment json>]",
        "telemetry new-epoch",
        "telemetry instance",
        "telemetry put <key> epoch=<id>",
        "telemetry raw <key>",
        "telemetry new-epoch <id>",
        "telemetry instance <id>",
        "refused <line>: telemetry.v1 <why>: <words>",
        "does_not <problem>[,<problem>...]",
        "epoch given=<json> current=<id>, decreased epoch=<id>",
    ] {
        assert!(owner::HELP.contains(piece), "--help names {piece:?}");
    }
}

/// `desired.v1` in the owner example (#840, DC): `--desired-author` and
/// `--desired-target`, two owners of one router, the scenarios' contracts
/// embedded. The author puts a document only on a change, the target acts
/// on its drive and reports; the drive refuses, fails and holds on the
/// runner's word; a restart is the process's, from the author's record;
/// `--desired-unbound` is the kill switch.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owner_example_implements_desired() {
    use zenkey::model::desired::{Outcome, Report, Source};

    let (_r1, ep) = router(None).await;
    let dir = tempfile::tempdir().unwrap();
    let record = dir.path().join("author.json");
    let record_arg = record.display().to_string();
    let author_args = [
        "--connect",
        &ep,
        "--desired-record",
        &record_arg,
        "--desired-window",
        "10",
        "lab/author",
    ];
    let a = spawn_owner(&author_args);
    assert_eq!(line(&a.lines).await, format!("connected {ep}"));
    line(&a.lines).await;
    assert_eq!(
        a.ask("desired adoption").await,
        "desired adoption found=nothing reput=- redeleted=- expired=- failed=-"
    );
    let t = spawn_owner(&[
        "--connect",
        &ep,
        "--desired-target",
        "lab/author",
        "--desired-watch",
        "lab/worker",
    ]);
    assert_eq!(line(&t.lines).await, format!("connected {ep}"));
    line(&t.lines).await;
    let d = "zk2/lab/author/tasking.v1/state/tasks/lab/worker";
    let report = |answer: String| -> Report {
        serde_json::from_str(answer.strip_prefix("desired report ").expect(&answer)).unwrap()
    };
    assert_eq!(
        report(t.ask("desired report").await),
        Report::baseline(Some(d.to_owned()))
    );
    loop {
        let acts = t.ask("desired acts").await;
        if !acts.contains("seeds=0") {
            assert!(acts.starts_with("desired acts 0 "), "{acts}");
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;

    // A put: converged on it, its stamp the report's.
    let v1 = r#"{"revision": 1, "mode": "run", "rate_hz": 10}"#;
    let put = a.ask(&format!("desired put lab/worker {v1}")).await;
    let stamp = put
        .strip_prefix(&format!("desired put {d} "))
        .unwrap_or_else(|| panic!("{put}"))
        .to_owned();
    let until = async |what: &str, f: &dyn Fn(&Report) -> bool| -> Report {
        let deadline = tokio::time::Instant::now() + SETTLE;
        loop {
            let r = report(t.ask("desired report").await);
            if f(&r) {
                return r;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "never: {what}: {r:?}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    let r = until("converged", &|r| r.outcome == Some(Outcome::Converged)).await;
    assert_eq!(r.observed.unwrap().stamp.to_string(), stamp);
    assert_eq!(r.in_force, r.observed);
    assert_eq!(
        t.ask("desired running").await,
        r#"desired running {"mode":"run","rate_hz":10,"revision":1}"#
    );
    assert_eq!(
        a.ask(&format!("desired put lab/worker {v1}")).await,
        format!("desired unchanged {d}"),
        "the same content: nothing put (§2.11)"
    );

    // The drive refuses its next act, then fails one partway.
    assert_eq!(
        t.ask("desired refuse-next invalid /mode").await,
        "desired refuse-next invalid"
    );
    a.ask(r#"desired put lab/worker {"revision": 2, "mode": "halt", "rate_hz": 10}"#)
        .await;
    let r = until("refused", &|r| r.outcome == Some(Outcome::Refused)).await;
    let refusal = r.refusal.unwrap();
    assert_eq!(
        (refusal.code.as_str(), refusal.pointer.as_deref()),
        ("invalid", Some("/mode"))
    );
    assert_eq!(t.ask("desired fail-next").await, "desired fail-next");
    a.ask(r#"desired put lab/worker {"revision": 3, "mode": "halt", "rate_hz": 20}"#)
        .await;
    let r = until("failed", &|r| r.outcome == Some(Outcome::Failed)).await;
    assert_eq!(r.refusal.unwrap().code, "apply");
    assert_eq!(
        t.ask("desired running").await,
        r#"desired running {"mode":"run","rate_hz":20,"revision":1}"#
    );
    // The scenarios' own refusals: an endpoint is reach.
    a.ask(r#"desired put lab/worker {"revision": 4, "mode": "run", "rate_hz": 10, "endpoint": "tcp/10.0.0.9:7447"}"#)
        .await;
    let r = until("refused reach", &|r| {
        r.refusal.as_ref().is_some_and(|f| f.code == "reach")
    })
    .await;
    assert_eq!(r.source, Source::Desired);

    // A held act, released.
    assert_eq!(t.ask("desired hold").await, "desired hold");
    a.ask(r#"desired put lab/worker {"revision": 5, "mode": "run", "rate_hz": 30}"#)
        .await;
    loop {
        if t.ask("desired acts").await.ends_with("held=true") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(t.ask("desired release").await, "desired release");
    until("converged on v5", &|r| {
        r.outcome == Some(Outcome::Converged) && r.observed.is_some_and(|o| o.revision == Some(5))
    })
    .await;
    assert_eq!(t.ask("desired reread 2000").await, "desired reread 2000");
    assert_eq!(t.ask("desired reread off").await, "desired reread off");

    // The line put byte for byte (text 0.4, F-194): a number past a
    // double's precision keeps its exact value, and the target acts on it.
    let tool = client(&ep).await;
    let v6 = r#"{"revision": 6, "mode": "run", "rate_hz": 10, "nonce": 9007199254740993}"#;
    let put = a.ask(&format!("desired put lab/worker {v6}")).await;
    assert!(put.starts_with(&format!("desired put {d} ")), "{put}");
    assert_eq!(state(&tool, d).await, [v6.as_bytes().to_vec()]);
    until("converged on v6", &|r| {
        r.outcome == Some(Outcome::Converged) && r.observed.is_some_and(|o| o.revision == Some(6))
    })
    .await;
    assert!(
        a.ask(r#"desired put lab/worker {"revision": 7, "revision": 8}"#)
            .await
            .contains("not strict JSON"),
        "a duplicate member is not strict JSON, and nothing is put"
    );
    assert_eq!(state(&tool, d).await, [v6.as_bytes().to_vec()]);

    // What the last seed found, and the GETs that reached the slice (text
    // 0.4, F-197).
    let seeds = t.ask("desired seeds").await;
    assert!(
        seeds.starts_with("desired seeds ")
            && seeds.contains(" answered=")
            && seeds.contains(" author=")
            && seeds.contains(" archive=-"),
        "{seeds}"
    );
    let gets = a.ask("desired gets lab/worker").await;
    let n: u64 = gets
        .strip_prefix(&format!("desired gets {d} "))
        .unwrap_or_else(|| panic!("{gets}"))
        .parse()
        .unwrap();
    assert!(
        n >= 2,
        "the target's seeds and the tool's GETs reach D: {gets}"
    );
    assert_eq!(
        a.ask("desired gets lab/nobody").await,
        "desired gets zk2/lab/author/tasking.v1/state/tasks/lab/nobody 0"
    );

    // The delete: reverted.
    let del = a.ask("desired delete lab/worker").await;
    assert!(del.starts_with(&format!("desired deleted {d} ")), "{del}");
    until("reverted", &|r| r.outcome == Some(Outcome::Reverted)).await;
    assert_eq!(
        t.ask("desired running").await,
        r#"desired running {"mode":"idle","rate_hz":1,"revision":0}"#
    );
    assert_eq!(
        a.ask("desired delete lab/worker").await,
        format!("desired none {d}")
    );
    assert_eq!(a.ask("desired documents").await, "desired documents {}");
    a.ask(r#"desired put lab/spare {"revision": 1, "mode": "run", "rate_hz": 10}"#)
        .await;
    assert_eq!(
        a.ask("desired documents").await,
        r#"desired documents {"lab/spare":{"mode":"run","rate_hz":10,"revision":1}}"#
    );
    let e = a.ask("desired new-epoch").await;
    assert!(
        e.starts_with("desired new-epoch zk2/lab/author/@zk/instance/"),
        "{e}"
    );

    // Refusals.
    assert!(
        a.ask("desired report")
            .await
            .starts_with("refused desired report: this owner is no desired.v1 target")
    );
    assert!(
        t.ask("desired put lab/x {}")
            .await
            .starts_with("refused desired put lab/x {}: this owner authors no desired.v1 document")
    );
    assert!(
        a.ask("desired nope")
            .await
            .starts_with("refused desired nope: usage")
    );
    assert!(
        a.ask("desired put worker {}")
            .await
            .starts_with("refused desired put worker {}: \"worker\" names no target")
    );

    // A restart from the record: the documents put again, the delete
    // within its window made again.
    a.stop().await;
    let a = spawn_owner(&author_args);
    line(&a.lines).await;
    line(&a.lines).await;
    assert_eq!(
        a.ask("desired adoption").await,
        "desired adoption found=record reput=lab/spare redeleted=lab/worker expired=- failed=-"
    );
    a.stop().await;
    t.stop().await;

    // The kill switch.
    let u = spawn_owner(&["--connect", &ep, "--desired-unbound", "lab/other"]);
    line(&u.lines).await;
    line(&u.lines).await;
    assert_eq!(
        u.ask("desired report").await,
        r#"desired report {"source":"baseline"}"#
    );
    assert_eq!(
        u.ask("desired acts").await,
        "desired acts 0 seeds=0 held=false"
    );
    assert_eq!(u.ask("desired seeds").await, "desired seeds 0 none");
    u.stop().await;

    // Without --desired-*, a desired command is refused.
    let o = spawn_owner(&["--connect", &ep, "ex/plain"]);
    line(&o.lines).await;
    line(&o.lines).await;
    assert!(
        o.ask("desired report")
            .await
            .starts_with("refused desired report: desired.v1 is not implemented here")
    );
    o.stop().await;
    assert!(owner::Options::parse(&["--desired-hold".into(), "ex/x".into()]).is_err());
}

/// `owner --help` documents the `desired.v1` surface a live runner drives
/// (#840, DC): every option, command and answer.
#[test]
fn the_help_documents_the_desired_surface() {
    for piece in [
        "--desired-author",
        "--desired-record <file>",
        "--desired-window <s>",
        "--desired-target <sys>/<svc>",
        "--desired-unbound",
        "--desired-archive <sys>/<svc>",
        "--desired-watch",
        "--desired-reread-ms <ms>",
        "--desired-hold",
        "desired put <target> <json>",
        "desired delete <target>",
        "desired documents",
        "desired adoption",
        "desired new-epoch",
        "desired report",
        "desired running",
        "desired fail-next",
        "desired refuse-next <code> [<pointer>]",
        "desired hold",
        "desired release",
        "desired reread <ms>|off",
        "desired acts",
        "desired seeds",
        "desired gets <target>",
        "put byte for byte once it reads as",
        "desired put <key> <stamp>",
        "desired unchanged <key>",
        "desired deleted <key> <stamp>, or desired none <key>",
        "desired adoption found=<record|archive|nothing>",
        "desired acts <n> seeds=<n> held=<true|false>",
        "desired gets <key> <n>",
        "desired seeds <n> answered=<n> author=<present|absent|incomplete|not-read|not-made>",
        "archive=<-|answered|nothing> [because <why>]",
        "desired seeds 0 none before the first",
        "{\"revision\": 0, \"mode\": \"idle\", \"rate_hz\": 1}",
    ] {
        assert!(owner::HELP.contains(piece), "--help names {piece:?}");
    }
}

/// One JSON answer of a `redundancy` command.
async fn ask_json(o: &Running, cmd: &str) -> serde_json::Value {
    let answer = o.ask(cmd).await;
    serde_json::from_str(&answer).unwrap_or_else(|e| panic!("{cmd}: not JSON ({e}): {answer}"))
}

/// The paths of the scenarios' governed contracts.
fn governed(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/contracts")
        .join(format!("{name}.toml"))
        .display()
        .to_string()
}

/// `redundancy.v1` in the owner example (#841, RB): `--standby` claims, and
/// serves only once elected; a second owner stands by on the first, its id
/// lower or not; each instance answers `append` with its `--name`, counted,
/// and puts its name as `balance` when it takes over; killed (here its task
/// dropped, as a kernel closes the links), the standby takes over; a re-mint
/// keeps it serving under its new id (F-209).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owner_example_stands_by_and_takes_over() {
    let (_r1, ep) = router(None).await;
    let tool = client(&ep).await;
    let ledger = governed("ledger.v1");
    let start = |name: &'static str, id: &'static str| {
        spawn_owner(&[
            "--connect",
            &ep,
            "--standby",
            "--election",
            "claim",
            "--instance-id",
            id,
            "--name",
            name,
            "lab/ledger",
            &ledger,
        ])
    };
    let a = start("A", "000000000000000a");
    assert_eq!(line(&a.lines).await, format!("connected {ep}"));
    assert_eq!(
        line(&a.lines).await,
        "ready zk2/lab/ledger/@zk/instance/000000000000000a"
    );
    let waited = ask_json(&a, "redundancy wait ledger.v1 serving 10000").await;
    assert_eq!(waited["reached"], true, "{waited}");
    let b = start("B", "0000000000000009");
    line(&b.lines).await;
    line(&b.lines).await;
    // B's id is the lower: a live incumbent wins whatever its id (§2.5).
    let mut role = serde_json::Value::Null;
    for _ in 0..200 {
        role = ask_json(&b, "redundancy role").await;
        if role["interfaces"]["ledger.v1"]["why"] == "incumbent" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        role,
        serde_json::json!({
            "redundancy": "role",
            "instance": "0000000000000009",
            "interfaces": {"ledger.v1": {
                "role": "standby", "why": "incumbent", "instance": "000000000000000a"
            }},
        })
    );
    // A call finds no standby.
    let call = |s: &zenoh::Session| {
        let s = s.clone();
        async move {
            get(
                &s,
                "zk2/lab/ledger/ledger.v1/@op/append",
                b"x",
                QueryTarget::BestMatching,
                ConsolidationMode::None,
            )
            .await
            .into_iter()
            .filter_map(|r| r.into_result().ok())
            .map(|s| String::from_utf8_lossy(&s.payload().to_bytes()).into_owned())
            .collect::<Vec<_>>()
        }
    };
    assert_eq!(call(&tool).await, ["A"]);
    assert_eq!(ask_json(&a, "redundancy calls").await["calls"], 1);
    assert_eq!(ask_json(&b, "redundancy calls").await["calls"], 0);
    assert_eq!(
        state(&tool, "zk2/lab/ledger/ledger.v1/state/balance").await,
        [b"A".to_vec()],
        "A's name, put at its takeover"
    );
    let reads = ask_json(&b, "redundancy reads").await;
    assert!(
        reads["interfaces"]["ledger.v1"]["made"].as_u64().unwrap() >= 1,
        "{reads}"
    );
    assert_eq!(reads["interfaces"]["ledger.v1"]["polls"], 0);
    assert!(reads["sessions"]["reader"]["reads"].as_u64().unwrap() >= 1);
    assert_eq!(
        reads["sessions"]["watch"]["reads"], 0,
        "the watch makes no read"
    );
    assert!(
        a.ask("redundancy lock hold").await.starts_with("refused"),
        "no lock under claim"
    );

    // A crashes: B takes over, and answers.
    a.task.abort();
    let waited = ask_json(&b, "redundancy wait ledger.v1 serving 10000").await;
    assert_eq!(waited["reached"], true, "{waited}");
    assert_eq!(call(&tool).await, ["B"]);
    eventually("B's name, put at its takeover", || async {
        state(&tool, "zk2/lab/ledger/ledger.v1/state/balance").await == [b"B".to_vec()]
    })
    .await;

    // A re-mint in this session: still serving, under its new id.
    let minted = ask_json(&b, "redundancy new-epoch").await;
    let next = minted["instance"].as_str().unwrap().to_owned();
    assert_ne!(next, "0000000000000009");
    tokio::time::sleep(Duration::from_secs(3)).await;
    let role = ask_json(&b, "redundancy role").await;
    assert_eq!(role["instance"], next.as_str());
    assert_eq!(role["interfaces"]["ledger.v1"]["role"], "serving", "{role}");
    let held = presence::liveliness_read(&tool, "zk2/lab/ledger/@zk/alive/**", T)
        .await
        .unwrap();
    assert_eq!(held.keys.len(), 1, "{held:?}");
    assert!(held.keys[0].contains(&next), "the token under the new id");
    b.stop().await;
}

/// `--election external` with the commands' lock (#841, RB): an owner
/// without the lock stands by, marked, whatever its id; `redundancy lock
/// hold` takes over, `lose` steps down; `--refuse-without-lock` refuses
/// to start; a governed contract without `--standby`, and `--standby`
/// without one, refuse too.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owner_example_under_external() {
    let (_r1, ep) = router(None).await;
    let valve = governed("valve.v1");
    let start = |id: &'static str, extra: &[&str]| {
        let mut args = vec![
            "--connect",
            &ep,
            "--election",
            "external",
            "--instance-id",
            id,
        ];
        args.extend_from_slice(extra);
        args.extend_from_slice(&["lab/valve", &valve]);
        spawn_owner(&args)
    };
    let v1 = start("0000000000000002", &[]);
    line(&v1.lines).await;
    line(&v1.lines).await;
    let v2 = start("0000000000000001", &[]);
    line(&v2.lines).await;
    line(&v2.lines).await;
    for o in [&v1, &v2] {
        let w = ask_json(o, "redundancy wait valve.v1 standby 5000").await;
        assert_eq!(w["standing"]["why"], "no_lock", "{w}");
    }
    assert_eq!(ask_json(&v1, "redundancy lock hold").await["held"], true);
    let w = ask_json(&v1, "redundancy wait valve.v1 serving 10000").await;
    assert_eq!(w["reached"], true, "{w}");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        ask_json(&v2, "redundancy role").await["interfaces"]["valve.v1"]["role"],
        "standby",
        "no election by ids moves V2"
    );
    assert_eq!(ask_json(&v1, "redundancy lock lose").await["held"], false);
    let w = ask_json(&v1, "redundancy wait valve.v1 standby 10000").await;
    assert_eq!(w["reached"], true, "the lock lost: a step-down");
    assert_eq!(ask_json(&v2, "redundancy lock hold").await["held"], true);
    let w = ask_json(&v2, "redundancy wait valve.v1 serving 10000").await;
    assert_eq!(w["reached"], true, "{w}");

    // Refusals at the start.
    let refuse = |args: &[&str]| {
        let o = spawn_owner(args);
        async move { o.task.await.unwrap().unwrap_err() }
    };
    let why = refuse(&[
        "--connect",
        &ep,
        "--refuse-without-lock",
        "lab/valve",
        &valve,
    ])
    .await;
    assert!(why.contains("§2.11"), "{why}");
    let why = refuse(&["--connect", &ep, "lab/valve", &valve]).await;
    assert!(why.contains("ServiceBuilder::redundancy"), "{why}");
    let plain = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/contracts/interop.v1.toml")
        .display()
        .to_string();
    let why = refuse(&["--connect", &ep, "--standby", "ex/plain", &plain]).await;
    assert!(why.contains("no interface"), "{why}");
    let why = refuse(&["--connect", &ep, "--election", "claim", "lab/valve", &valve]).await;
    assert!(why.contains("declares external"), "{why}");
    v1.stop().await;
    v2.stop().await;
}

/// `owner --help` documents the `redundancy.v1` surface a live runner
/// drives (#841, RB): every option, command and answer.
#[test]
fn the_help_documents_the_redundancy_surface() {
    for piece in [
        "--standby",
        "--election claim|external",
        "--instance-id <id>",
        "--name <word>",
        "--retry-ms <ms>",
        "--grace-ms <ms>",
        "--poll-ms <ms>",
        "--lease-ms <ms>",
        "--lock-file <path>",
        "--refuse-without-lock",
        "--hold-heal",
        "--peer",
        "redundancy role",
        "redundancy reads",
        "redundancy lock hold|lose",
        "redundancy calls",
        "redundancy hold-heal on|off",
        "redundancy wait <iface> serving|standby <ms>",
        "redundancy new-epoch",
        "redundancy zid",
        "--wait-go",
        "\"poll_log\"",
        "{\"redundancy\": \"role\", \"instance\": <id>, \"interfaces\": {<iface>: <standing>}}",
        "{\"redundancy\": \"lock\", \"held\": <bool>}",
        "own_claim_missing, descriptor_unread, lower_candidate,",
        "\"fault\"?: <words>",
        "refused <line>: <why>",
    ] {
        assert!(owner::HELP.contains(piece), "--help names {piece:?}");
    }
}

/// desired.v1 §2.1, text 0.7, F-224 (`scenarios.md` §11): given copies of
/// `tasking.v1` with two desired resources and of `worker.v1` whose
/// requirement names both, `--desired-target` refuses to start with the
/// runtime's refusal, `not started: role "task": its binding is invalid
/// (two_documents) (desired.v1 §2.3)`, before any token.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owner_example_refuses_a_role_on_two_documents() {
    let (_r1, ep) = router(None).await;
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/contracts");
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(src.join("tasking.json"), dir.path().join("tasking.json")).unwrap();
    std::fs::copy(
        src.join("../../../spec/profiles/desired/desired.v1.json"),
        dir.path().join("desired.v1.json"),
    )
    .unwrap();
    let tasking = std::fs::read_to_string(src.join("tasking.v1.toml")).unwrap()
        + "\n[resources.\"plans/{system}/{service}\"]\nkind        = \"state\"\n\
           type        = \"json:Task\"\n\
           params      = { system = \"string\", service = \"string\" }\n\
           cardinality = 16\n\
           annotations = { \"desired.target_param\" = [\"system\", \"service\"], \
           \"freshness.ttl_s\" = 0 }\n";
    let worker = std::fs::read_to_string(src.join("worker.v1.toml"))
        .unwrap()
        .replace(
            "\"../../../spec/profiles/desired/desired.v1.json\"",
            "\"desired.v1.json\"",
        )
        .replace(
            "resources = [\"tasks/{system}/{service}\"]",
            "resources = [\"tasks/{system}/{service}\", \"plans/{system}/{service}\"]",
        );
    let (t, w) = (
        dir.path().join("tasking.v1.toml"),
        dir.path().join("worker.v1.toml"),
    );
    std::fs::write(&t, tasking).unwrap();
    std::fs::write(&w, worker).unwrap();
    let (t, w) = (t.display().to_string(), w.display().to_string());
    let o = spawn_owner(&[
        "--connect",
        &ep,
        "--desired-target",
        "lab/author",
        "lab/worker",
        &t,
        &w,
    ]);
    let why = o.task.await.unwrap().unwrap_err();
    assert_eq!(
        why,
        "not started: role \"task\": its binding is invalid (two_documents) (desired.v1 §2.3)"
    );
    let g = client(&ep).await;
    let read = presence::liveliness_read(&g, "zk2/lab/worker/@zk/**", T)
        .await
        .unwrap();
    assert!(
        read.complete && read.keys.is_empty(),
        "no token of lab/worker: {read:?}"
    );
}

/// F-214 (#841, RC): a contract listing `redundancy.v1` is written only
/// once the start has checked its declaration, so a copy of `ledger.v1`
/// whose election is `"vote"` stops the owner with the start's refusal,
/// `not started: … invalid: value`, as `scenarios.md` §11 step 2 expects,
/// and never with a standby's refused write.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owner_example_refuses_an_invalid_declaration_before_writing() {
    let (_r1, ep) = router(None).await;
    let dir = tempfile::tempdir().unwrap();
    let text = std::fs::read_to_string(governed("ledger.v1"))
        .unwrap()
        .replace(
            "\"redundancy.election\" = \"claim\"",
            "\"redundancy.election\" = \"vote\"",
        );
    let path = dir.path().join("ledger.v1.toml");
    std::fs::write(&path, text).unwrap();
    let path = path.display().to_string();
    for extra in [&["--standby"][..], &[][..]] {
        let mut args = vec!["--connect", &ep];
        args.extend_from_slice(extra);
        args.extend_from_slice(&["lab/ledger", &path]);
        let o = spawn_owner(&args);
        let why = o.task.await.unwrap().unwrap_err();
        assert!(why.starts_with("not started:"), "{why}");
        assert!(why.contains("invalid: value"), "{why}");
        assert!(
            !why.contains("stands by"),
            "never a standby's refused write: {why}"
        );
    }
}

/// F-216 and the round's stdin gaps (#841, RC): `--wait-go` opens every
/// session and starts at `go`; `redundancy zid` names the claim's, the
/// reader's and the watch's zids; without a watch session `redundancy
/// reads` lists what each poll read.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owner_example_waits_for_go_and_says_its_polls() {
    let (_r1, ep) = router(None).await;
    let tool = client(&ep).await;
    let ledger = governed("ledger.v1");
    let o = spawn_owner(&[
        "--connect",
        &ep,
        "--wait-go",
        "--poll-ms",
        "200",
        "--instance-id",
        "000000000000000a",
        "lab/ledger",
        &ledger,
    ]);
    assert_eq!(line(&o.lines).await, format!("connected {ep}"));
    assert_eq!(line(&o.lines).await, "waiting go");
    assert!(
        o.ask("redundancy role")
            .await
            .starts_with("refused redundancy role:")
    );
    let none = presence::liveliness_read(&tool, "zk2/lab/ledger/@zk/**", T)
        .await
        .unwrap();
    assert!(
        none.complete && none.keys.is_empty(),
        "nothing before go: {none:?}"
    );
    o.send.send("go".to_owned()).unwrap();
    assert_eq!(
        line(&o.lines).await,
        "ready zk2/lab/ledger/@zk/instance/000000000000000a"
    );
    let w = ask_json(&o, "redundancy wait ledger.v1 serving 10000").await;
    assert_eq!(w["reached"], true, "{w}");
    let zid = ask_json(&o, "redundancy zid").await;
    assert!(zid["zid"].is_string() && zid["reader"].is_string(), "{zid}");
    assert!(zid.get("watch").is_none(), "it polls: no watch session");
    assert_ne!(zid["zid"], zid["reader"]);
    tokio::time::sleep(Duration::from_millis(600)).await;
    let reads = ask_json(&o, "redundancy reads").await;
    let log = reads["interfaces"]["ledger.v1"]["poll_log"]
        .as_array()
        .unwrap();
    assert!(log.len() >= 2, "{reads}");
    let last = log.last().unwrap();
    assert_eq!(last["made"], true);
    assert_eq!(
        last["holders"],
        serde_json::json!(["000000000000000a"]),
        "{last}"
    );
    o.stop().await;
}

/// F-220 (#841, RD): `--wait-go` through the commands `main` reads, a pipe
/// standing in for standard input. A line sent before `go` is refused,
/// `go` starts the service, its instance token reaches the router, and the
/// pipe's close stops it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owner_example_starts_at_go_on_its_standard_input() {
    use std::io::Write;
    let (_r1, ep) = router(None).await;
    let tool = client(&ep).await;
    let ledger = governed("ledger.v1");
    let args: Vec<String> = [
        "--connect",
        ep.as_str(),
        "--wait-go",
        "--instance-id",
        "000000000000000a",
        "lab/ledger",
        ledger.as_str(),
    ]
    .iter()
    .map(|a| (*a).to_owned())
    .collect();
    let opts = owner::Options::parse(&args).unwrap();
    let (stdin, mut input) = std::io::pipe().unwrap();
    let (commands, closed) = owner::command_lines(move || std::io::BufReader::new(stdin));
    let (say, lines) = flume::unbounded::<String>();
    let task = tokio::spawn(async move {
        owner::run_with(
            opts,
            move |l| {
                let _ = say.send(l);
            },
            commands,
            closed,
        )
        .await
        .map_err(|e| e.to_string())
    });
    assert_eq!(line(&lines).await, format!("connected {ep}"));
    assert_eq!(line(&lines).await, "waiting go");
    writeln!(input, "redundancy role").unwrap();
    assert!(
        line(&lines)
            .await
            .starts_with("refused redundancy role: this owner starts at go"),
    );
    writeln!(input, "go").unwrap();
    assert_eq!(
        line(&lines).await,
        "ready zk2/lab/ledger/@zk/instance/000000000000000a"
    );
    eventually("its instance token at the router", || async {
        presence::liveliness_read(&tool, "zk2/lab/ledger/@zk/instance/*", T)
            .await
            .is_ok_and(|r| {
                r.keys
                    .iter()
                    .any(|k| k == "zk2/lab/ledger/@zk/instance/000000000000000a")
            })
    })
    .await;
    // Standard input's close ends the run.
    drop(input);
    tokio::time::timeout(SETTLE, task)
        .await
        .expect("the owner stops at its input's close")
        .unwrap()
        .unwrap();
}
