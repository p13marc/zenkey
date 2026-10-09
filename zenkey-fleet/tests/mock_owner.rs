//! The mock owner on a live bus (#612, FJ8a): `gen` and `serve` bring up a
//! real zk2 service, read back the way a consumer and a tool read any
//! other, and `bench call` times what it answers.
//!
//! One router and client sessions, the zk2 suites' convention: the mock on
//! one session, the tool on another, nothing named by port.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

mod util;
use util::zk2::{T, addr, client, example, router};

use zenkey_fleet::bus::presence::{Scope, service_listing};
use zenkey_fleet::model::catalog::Revision;
use zenkey_fleet::report::{
    Asked, ContractSource, DescriptorAnswer, Rendered, ServedAnswer, StateValue, WatchEvent,
};
use zenkey_fleet::{
    BenchSpec, GenPattern, GenSpec, MemberArg, MockAnswer, ResolvedTarget, ServeSpec, StateRead,
    address_presence, check_address, gen_plan, run_gen, serve_operation,
};
use zenkey_model::authoring::Kind;
use zenkey_model::template::Bindings;

fn revision(path: &str) -> Arc<Revision> {
    Arc::new(Revision::from_contract(example(path), ContractSource::File))
}

/// The descriptor `address` serves, once presence lists it served.
async fn descriptor(tool: &zenoh::Session, address: &str) -> zenkey_model::descriptor::Descriptor {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(l) = service_listing(tool, &Scope::all(), T).await
            && let Some(d) = l
                .services
                .iter()
                .filter(|s| s.address == address)
                .flat_map(|s| s.instances.iter())
                .find_map(|i| match &i.descriptor {
                    Asked::Asked(DescriptorAnswer::Served { descriptor }) => {
                        Some((**descriptor).clone())
                    }
                    _ => None,
                })
        {
            return d;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "never: {address} listed with its descriptor"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// `gen` brings up an owner presence lists, with the synthetic marker in its
/// descriptor; a consumer's subscription decodes every sample through the
/// contract and discards none (R6); the owner's state GET answers with its
/// own stamp (S1); and a second mock at the address is refused until the
/// operator says otherwise.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gen_brings_up_a_visible_owner_publishing_through_its_contract() {
    let (_router, ep) = router(None).await;
    let owners = client(&ep).await;
    let tool = client(&ep).await;
    let netif = revision("tcgui/tc.netif.v1");
    let address = addr("host-a/tc");
    let before = address_presence(&tool, &address, T).await.unwrap();
    assert!(before.instances.is_empty() && before.complete, "{before:?}");
    check_address(&before, &address, false).expect("nothing runs there");

    let spec = GenSpec {
        address: address.clone(),
        revisions: vec![Arc::clone(&netif)],
        members: vec![MemberArg::parse("bandwidth/{ns}/{iface}=default/eth0").unwrap()],
        bindings: BTreeMap::new(),
        rate_hz: Some(20.0),
        pattern: GenPattern::Steady,
        duration: Duration::from_secs(4),
        seed: 7,
        tool: "mock_owner test".into(),
    };
    let plan = gen_plan(&spec).unwrap();
    let (up, came_up) = tokio::sync::oneshot::channel();
    let run = tokio::spawn({
        let spec = spec.clone();
        async move {
            run_gen(&owners, &spec, &plan, |i| {
                let _ = up.send(i.to_owned());
            })
            .await
        }
    });
    let instance = came_up.await.expect("the mock came up");

    let d = descriptor(&tool, "host-a/tc").await;
    assert_eq!(d.instance, instance);
    assert_eq!(d.meta["synthetic"]["synthetic"], true, "{:?}", d.meta);
    assert_eq!(d.meta["synthetic"]["seed"], 7);
    let zid = d.meta["zid"].as_str().expect("the owner's zid").to_owned();

    let target = ResolvedTarget::parse("host-a/tc").unwrap();
    let bw = netif
        .resource("bandwidth/{ns}/{iface}", &[Kind::Stream])
        .unwrap();
    let mut watch = zenkey_fleet::watch_resource(&tool, &netif, &target, bw, &Bindings::new())
        .await
        .unwrap();
    for _ in 0..5 {
        let s = tokio::time::timeout(Duration::from_secs(10), watch.next())
            .await
            .expect("a sample within the window")
            .expect("the subscription is up");
        assert_eq!(
            s.key,
            "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0"
        );
        let WatchEvent::Put { payload, .. } = s.event else {
            panic!("a put")
        };
        assert!(
            matches!(&payload.rendered, Rendered::Value { declared, .. } if declared == "json:BandwidthUpdate"),
            "{payload:?}"
        );
    }
    assert_eq!(watch.discarded(), 0, "no sample on a wildcard key (R6)");

    let ns = netif.resource("namespaces", &[Kind::State]).unwrap();
    let state = zenkey_fleet::get_state(
        &tool,
        StateRead {
            revision: &netif,
            owner: &address,
            resource: ns,
            values: &Bindings::new(),
            timeout: T,
        },
    )
    .await
    .unwrap();
    let row = state.rows.first().expect("the owner answers");
    assert!(matches!(row.value, StateValue::Value { .. }), "{row:?}");
    let stamp = row.timestamp.as_ref().expect("stamped (S1)");
    assert_eq!(
        u128::from_str_radix(&stamp.clock, 16).unwrap(),
        u128::from_str_radix(&zid, 16).unwrap(),
        "the owner's own clock (O7)"
    );

    let during = address_presence(&tool, &address, T).await.unwrap();
    assert_eq!(during.instances, [instance.clone()]);
    let refused = check_address(&during, &address, false).unwrap_err();
    assert!(refused.is_unaskable(), "{refused}");
    assert!(refused.to_string().contains("--i-know"), "{refused}");
    check_address(&during, &address, true).expect("the operator means it");

    let report = run.await.unwrap().unwrap();
    assert_eq!(report.instance, instance);
    assert!(report.sent > 5, "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");
}

/// `serve` answers one operation with its fixed reply and logs each call
/// with its request decoded and its claimed metadata (O7); every other
/// operation is answered too — an optional one `unavailable`, a required
/// one `internal` — never silent (O3).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn serve_answers_logs_and_leaves_no_operation_silent() {
    let (_router, ep) = router(None).await;
    let owners = client(&ep).await;
    let tool = client(&ep).await;
    let netif = revision("tcgui/tc.netif.v1");
    let diag = netif.resource("diagnostics", &[Kind::Operation]).unwrap();
    let reply = zenkey_fleet::encode_response(&netif, diag, br#"{"ok": true}"#).unwrap();
    let mut served = serve_operation(
        &owners,
        ServeSpec {
            address: addr("host-a/tc"),
            revision: Arc::clone(&netif),
            operation: "@op/diagnostics".into(),
            answer: MockAnswer::Reply {
                bytes: reply,
                summary: None,
            },
            bindings: BTreeMap::new(),
            tool: "mock_owner test".into(),
        },
    )
    .await
    .unwrap();
    let d = descriptor(&tool, "host-a/tc").await;
    assert_eq!(d.instance, served.instance());

    let client = zk2::Client::new(&tool, netif.shared_contract(), &["host-a/tc"])
        .unwrap()
        .with_timeout(Duration::from_secs(5))
        .with_metadata(zk2::CallMetadata::new("ops", "req-1"));
    let outcome = client
        .call_value(
            &addr("host-a/tc"),
            "@op/diagnostics",
            &Bindings::new(),
            &serde_json::json!({}),
        )
        .await
        .unwrap();
    let value: serde_json::Value = outcome.answer().expect("a value").value().unwrap();
    assert_eq!(value, serde_json::json!({"ok": true}));

    let call = tokio::time::timeout(Duration::from_secs(10), served.next())
        .await
        .expect("the call is logged")
        .expect("the log is open");
    assert_eq!(call.n, 1);
    assert_eq!(call.operation, "@op/diagnostics");
    assert_eq!(call.key, "zk2/host-a/tc/tc.netif.v1/@op/diagnostics");
    assert!(call.concrete);
    assert!(
        matches!(&call.request.rendered, Rendered::Value { declared, .. } if declared == "json:DiagnosticsRequest"),
        "{:?}",
        call.request
    );
    let meta = call.metadata.expect("the claimed metadata");
    assert_eq!(
        (meta.actor.as_deref(), meta.request_id.as_deref()),
        (Some("ops"), Some("req-1"))
    );
    assert_eq!(call.answer, ServedAnswer::Reply { summary: false });

    // The required `set` is not this mock's: answered `internal`, never
    // silence.
    let values: Bindings = [
        ("ns".to_owned(), vec!["default".to_owned()]),
        ("iface".to_owned(), vec!["eth0".to_owned()]),
    ]
    .into();
    let other = client
        .call_value(
            &addr("host-a/tc"),
            "@op/interfaces/{ns}/{iface}/set",
            &values,
            &serde_json::json!({"up": true}),
        )
        .await
        .unwrap();
    let env = other.refusal().expect("an envelope, never silence");
    assert_eq!(env.code, "internal");
    assert!(
        env.message.contains("serves @op/diagnostics alone"),
        "{env:?}"
    );
    assert_eq!(served.calls(), 1, "the other operation is not the log's");
    served.close().await.unwrap();
}

/// `bench call` attributes each reply by its key, keeps a refusal apart
/// from the values, and counts a holder that never sent a value.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bench_attributes_replies_by_key_and_keeps_refusals_apart() {
    let (_router, ep) = router(None).await;
    let owners = client(&ep).await;
    let tool = client(&ep).await;
    let netif = revision("tcgui/tc.netif.v1");
    let diag = netif.resource("diagnostics", &[Kind::Operation]).unwrap();
    let reply = zenkey_fleet::encode_response(&netif, diag, br#"{"ok": true}"#).unwrap();
    let mut mocks = Vec::new();
    for (host, answer) in [
        (
            "h1",
            MockAnswer::Reply {
                bytes: reply.clone(),
                summary: None,
            },
        ),
        (
            "h2",
            MockAnswer::Reply {
                bytes: reply.clone(),
                summary: None,
            },
        ),
        (
            "h3",
            MockAnswer::Refuse(zk2::OpError::busy("a scan is running")),
        ),
    ] {
        mocks.push(
            serve_operation(
                &owners,
                ServeSpec {
                    address: addr(&format!("{host}/tc")),
                    revision: Arc::clone(&netif),
                    operation: "@op/diagnostics".into(),
                    answer,
                    bindings: BTreeMap::new(),
                    tool: "mock_owner test".into(),
                },
            )
            .await
            .unwrap(),
        );
    }
    for host in ["h1", "h2", "h3"] {
        descriptor(&tool, &format!("{host}/tc")).await;
    }
    let target = ResolvedTarget::parse("*/tc").unwrap();
    let plan = zenkey_fleet::plan_call(&netif, target, "diagnostics", Bindings::new()).unwrap();
    let request = zenkey_fleet::encode_request(&netif, &plan, b"{}").unwrap();
    let report = zenkey_fleet::run_bench(
        &tool,
        BenchSpec {
            revision: &netif,
            plan: &plan,
            request,
            calls: 5,
            concurrency: 2,
            timeout: Duration::from_secs(2),
            force: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(report.completed, 5, "{report:?}");
    let keys: Vec<&str> = report.repliers.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(keys.len(), 2, "{report:?}");
    for host in ["h1", "h2"] {
        let r = report
            .repliers
            .iter()
            .find(|r| r.address == format!("{host}/tc"))
            .expect("attributed by its key");
        assert_eq!(r.key, format!("zk2/{host}/tc/tc.netif.v1/@op/diagnostics"));
        assert_eq!(r.replies, 5);
    }
    assert_eq!(report.refusals.count, 5);
    assert_eq!(report.refusals.codes["busy"], 5);
    assert_eq!(report.silent, 0);
    let h3 = report
        .presence
        .holders
        .iter()
        .find(|h| h.address == "h3/tc")
        .expect("h3 holds the token");
    assert_eq!(h3.without_value, 5, "refused or silent, never a value");
    assert_eq!(report.exit_code(), 1, "an envelope among the values");

    // An operation that is not idempotent is refused before any call.
    let set = zenkey_fleet::plan_call(
        &netif,
        ResolvedTarget::parse("h1/tc").unwrap(),
        "interfaces/{ns}/{iface}/set",
        [
            ("ns".to_owned(), vec!["default".to_owned()]),
            ("iface".to_owned(), vec!["eth0".to_owned()]),
        ]
        .into(),
    )
    .unwrap();
    let e = zenkey_fleet::run_bench(
        &tool,
        BenchSpec {
            revision: &netif,
            plan: &set,
            request: b"{}".to_vec(),
            calls: 1,
            concurrency: 1,
            timeout: T,
            force: false,
        },
    )
    .await
    .unwrap_err();
    assert!(e.to_string().contains("not idempotent"), "{e}");

    // Silence is counted apart: nothing serves `diagnostics` at h9.
    let silent_plan = zenkey_fleet::plan_call(
        &netif,
        ResolvedTarget::parse("h9/tc").unwrap(),
        "diagnostics",
        Bindings::new(),
    )
    .unwrap();
    let request = zenkey_fleet::encode_request(&netif, &silent_plan, b"{}").unwrap();
    let silent = zenkey_fleet::run_bench(
        &tool,
        BenchSpec {
            revision: &netif,
            plan: &silent_plan,
            request,
            calls: 2,
            concurrency: 1,
            timeout: Duration::from_millis(300),
            force: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(silent.silent, 2, "{silent:?}");
    assert!(silent.repliers.is_empty());
    assert_eq!(silent.exit_code(), 2);
    for m in mocks {
        m.close().await.unwrap();
    }
}
