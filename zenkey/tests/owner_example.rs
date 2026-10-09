//! The interop owner example (`examples/owner.rs`, #610) as a client of a
//! separate router (#660): what `state.md §1` and `presence.md §1` ask of an
//! owner a black-box scenario watches through R1.
//!
//! The example is compiled into this suite and run in process, through the
//! same `run` its `main` calls, so the suite needs no built binary.

mod common;

#[allow(dead_code)]
#[path = "../examples/owner.rs"]
mod owner;

use std::path::Path;
use std::time::Duration;

use common::{SETTLE, T, client, router};
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
