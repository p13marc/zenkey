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

/// A `json:DiagnosticsRequest` the contract's schema accepts.
const DIAGNOSTICS: &[u8] = br#"{"namespace":"default","interface":"eth0"}"#;

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
        elect: None,
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
    assert_eq!(during.instances, std::slice::from_ref(&instance));
    let refused = check_address(&during, &address, false).unwrap_err();
    assert!(refused.is_unaskable(), "{refused}");
    assert!(refused.to_string().contains("--i-know"), "{refused}");
    check_address(&during, &address, true).expect("the operator means it");

    let report = run.await.unwrap().unwrap();
    assert_eq!(report.instance, instance);
    assert!(report.sent > 5, "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");
}

/// #744: a `--rate` above an event's declared rate (`tc.netem.v1`'s
/// `events/applied`, `low`) runs, and holds the event to its rate: the plan
/// says so on the entry, and the run ends with nothing failed and nothing
/// the runtime's budget refused, the streams and states at the rate asked.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gen_holds_an_event_to_its_declared_rate_and_says_so() {
    let (_router, ep) = router(None).await;
    let owners = client(&ep).await;
    let spec = GenSpec {
        address: addr("host-b/tc"),
        revisions: vec![revision("tcgui/tc.netem.v1")],
        members: vec![],
        bindings: BTreeMap::new(),
        rate_hz: Some(10.0),
        pattern: GenPattern::Steady,
        duration: Duration::from_secs(3),
        seed: 7,
        tool: "mock_owner test".into(),
        elect: None,
    };
    let plan = gen_plan(&spec).unwrap();
    let applied = plan
        .entries
        .iter()
        .find(|e| e.resource == "events/applied")
        .expect("the event's entry");
    assert_eq!(applied.rate_hz, Some(1.0 / 60.0));
    assert!(
        applied
            .note
            .as_deref()
            .is_some_and(|n| n.contains("above its declared rate low")
                && n.contains("it keeps its declared rate")),
        "{applied:?}"
    );
    let report = run_gen(&owners, &spec, &plan, |_| {}).await.unwrap();
    assert_eq!((report.failed, report.refused), (0, 0), "{report:?}");
    assert!(
        report.sent > 10,
        "the streams and states at 10 Hz: {report:?}"
    );
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
            elect: None,
        },
    )
    .await
    .unwrap();
    let d = descriptor(&tool, "host-a/tc").await;
    assert_eq!(d.instance, served.instance());

    let client = zenkey::Client::new(&tool, netif.shared_contract(), &["host-a/tc"])
        .unwrap()
        .with_timeout(Duration::from_secs(5))
        .with_metadata(zenkey::CallMetadata::new("ops", "req-1"));
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
            MockAnswer::Refuse(zenkey::OpError::busy("a scan is running")),
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
                    elect: None,
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
    // A request its type accepts: one it refuses is never sent (#671).
    let request = zenkey_fleet::encode_request(&netif, &plan, DIAGNOSTICS).unwrap();
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
    let request = zenkey_fleet::encode_request(&netif, &silent_plan, DIAGNOSTICS).unwrap();
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

/// `redundancy.v1` (#841, RC): `gen` of a governed contract
/// (`zs.catalog.v1`, ZenSight's singleton) is a candidate. Alone it is
/// elected and serves; a second beside it stands by, holding no token and
/// putting nothing, and the address reads as one incumbent and a standby.
/// When the first's run ends, the standby takes over and serves (§2.8).
/// Each run's report says its role, the standby's writes held back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gen_of_a_governed_contract_elects_and_a_second_stands_by() {
    let (_router, ep) = router(None).await;
    let tool = client(&ep).await;
    let catalog = revision("zensight/zs.catalog.v1");
    let address = addr("lab/catalog");
    let candidate = |seed: u64, secs: u64| {
        let ep = ep.clone();
        let catalog = Arc::clone(&catalog);
        let address = address.clone();
        async move {
            let owner = client(&ep).await;
            let startup = zenkey::redundancy::RedundancyStartup::new(client(&ep).await)
                .with_watch(client(&ep).await);
            let spec = GenSpec {
                address,
                revisions: vec![catalog],
                members: vec![],
                bindings: BTreeMap::new(),
                rate_hz: Some(10.0),
                pattern: GenPattern::Steady,
                duration: Duration::from_secs(secs),
                seed,
                tool: "mock_owner test".into(),
                elect: Some(startup),
            };
            let plan = gen_plan(&spec).unwrap();
            let (up, came_up) = tokio::sync::oneshot::channel();
            let run = tokio::spawn(async move {
                run_gen(&owner, &spec, &plan, |i| {
                    let _ = up.send(i.to_owned());
                })
                .await
            });
            (came_up.await.expect("the mock came up"), run)
        }
    };
    let read = || async {
        zenkey_fleet::run_redundancy(
            &tool,
            "",
            Some(address.clone()),
            zenkey_fleet::RedundancySpec::new(T),
            &zenkey_fleet::bus::contracts::BundleStore::new(T),
        )
        .await
    };
    let eventually = |what: &'static str, ok: fn(&zenkey_fleet::report::RedundancyRow) -> bool| {
        let read = &read;
        async move {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
            loop {
                let r = read().await;
                if let Some(row) = r.interfaces.first()
                    && ok(row)
                {
                    return r;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "never {what}: {r:#?}"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    };
    let (first, run_a) = candidate(1, 6).await;
    let r = eventually("one incumbent", |row| {
        row.one_incumbent.reason == "one" && row.holders.len() == 1
    })
    .await;
    assert_eq!(r.interfaces[0].holders, vec![first.clone()], "{r:#?}");
    let (second, run_b) = candidate(2, 14).await;
    let r = eventually("a standby beside it", |row| {
        row.standbys.count == 1 && row.one_incumbent.reason == "one"
    })
    .await;
    let row = &r.interfaces[0];
    assert_eq!(row.holders, vec![first.clone()], "{r:#?}");
    let roles: BTreeMap<&str, zenkey_fleet::report::RedundancyRole> = row
        .instances
        .iter()
        .map(|i| (i.instance.as_str(), i.role))
        .collect();
    assert_eq!(
        roles,
        BTreeMap::from([
            (
                first.as_str(),
                zenkey_fleet::report::RedundancyRole::Incumbent
            ),
            (
                second.as_str(),
                zenkey_fleet::report::RedundancyRole::Standby
            ),
        ])
    );
    assert_eq!(
        zenkey_fleet::judgement_exit_code(&r.judgement()),
        0,
        "{r:#?}"
    );
    let a = run_a.await.unwrap().expect("the incumbent's run");
    assert_eq!(a.roles["zs.catalog.v1"], "serving", "{a:?}");
    assert!(a.sent > 0 && a.failed == 0 && a.standing_by == 0, "{a:?}");
    // Its clean stop withdraws its tokens at once: the standby takes over.
    let r = eventually("the standby taking over", |row| {
        row.one_incumbent.reason == "one" && row.standbys.count == 0
    })
    .await;
    assert_eq!(r.interfaces[0].holders, vec![second.clone()], "{r:#?}");
    let b = run_b.await.unwrap().expect("the standby's run");
    assert_eq!(b.roles["zs.catalog.v1"], "serving", "{b:?}");
    assert!(
        b.sent > 0 && b.failed == 0 && b.standing_by > 0,
        "held back while it stood by, then sent: {b:?}"
    );
}

/// `redundancy.v1` (#841, RC): `serve` of a governed contract elects too. A
/// second beside a serving one stands by: a call finds it nowhere (§2.4),
/// and it serves once the first is gone.
///
/// Each incumbent is called once the tool reads its interface token (#900).
/// Its own role reads `serving` as soon as its session has declared the
/// token, before the router has registered what that session declared, and
/// a call the router routes before then meets no queryable: silence. The
/// router registers one session's declarations in their order, the
/// queryables first and the token last (§2.6), so once the tool reads the
/// token through it, the call finds the queryable.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn serve_of_a_governed_contract_stands_by_beside_a_serving_one() {
    /// Whether the tool reads `instance`'s interface token of `iface` at the
    /// address, through the router.
    async fn held(
        tool: &zenoh::Session,
        iface: &zenkey_model::grammar::IfaceId,
        instance: &str,
    ) -> bool {
        let Ok(r) = zenkey::presence::liveliness_read(tool, "zk2/lab/ledger/@zk/**", T).await
        else {
            return false;
        };
        r.keys.iter().any(|k| {
            matches!(
                zenkey_model::grammar::parse(k),
                Ok(zenkey_model::grammar::ZkKey::Alive { iface: i, instance: id, .. })
                    if i == *iface && id.as_str() == instance
            )
        })
    }
    async fn until(
        s: &zenkey_fleet::Served,
        iface: &zenkey_model::grammar::IfaceId,
        role: zenkey::redundancy::Role,
    ) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        while s.role(iface) != Some(role) {
            assert!(tokio::time::Instant::now() < deadline, "never {role:?}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    let (_router, ep) = router(None).await;
    let tool = client(&ep).await;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../zenkey/tests/contracts/ledger.v1.toml");
    let l = zenkey_model::contract::load_path(&path);
    let ledger = Arc::new(Revision::from_contract(
        l.contract.unwrap_or_else(|| panic!("{}", l.report)),
        ContractSource::File,
    ));
    let iface = ledger.iface().clone();
    let up = |answer: &'static str| {
        let ep = ep.clone();
        let ledger = Arc::clone(&ledger);
        async move {
            let owner = client(&ep).await;
            let startup = zenkey::redundancy::RedundancyStartup::new(client(&ep).await)
                .with_watch(client(&ep).await);
            serve_operation(
                &owner,
                ServeSpec {
                    address: addr("lab/ledger"),
                    revision: ledger,
                    operation: "@op/append".into(),
                    answer: MockAnswer::Reply {
                        bytes: answer.as_bytes().to_vec(),
                        summary: None,
                    },
                    bindings: BTreeMap::new(),
                    tool: "mock_owner test".into(),
                    elect: Some(startup),
                },
            )
            .await
            .unwrap()
        }
    };
    let first = up("first").await;
    until(&first, &iface, zenkey::redundancy::Role::Serving).await;
    let first_id = first.instance();
    util::zk2::eventually("the first's token at the tool", || {
        held(&tool, &iface, &first_id)
    })
    .await;
    let second = up("second").await;
    // A live incumbent wins (§2.5 step 2): it stands by once it has read.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(second.role(&iface), Some(zenkey::redundancy::Role::Standby));
    let call = zenkey::Client::new(&tool, ledger.shared_contract(), &["lab/ledger"])
        .unwrap()
        .with_timeout(Duration::from_secs(5));
    let answered = |o: &zenkey::Outcome, from: &str| -> Vec<u8> {
        o.answer()
            .unwrap_or_else(|| panic!("a value from the {from}: {o:?}"))
            .payload()
            .to_bytes()
            .to_vec()
    };
    for _ in 0..3 {
        let o = call
            .call(
                &addr("lab/ledger"),
                "@op/append",
                &Bindings::new(),
                b"x".to_vec(),
            )
            .await
            .unwrap();
        assert_eq!(
            answered(&o, "first"),
            b"first",
            "a call finds no standby (§2.4)"
        );
    }
    assert_eq!(second.calls(), 0);
    first.close().await.unwrap();
    until(&second, &iface, zenkey::redundancy::Role::Serving).await;
    // Its queryable is declared before its token (§2.6), so a call made once
    // the tool reads its token finds it; its own role, read above, says
    // nothing of the router (#900).
    let second_id = second.instance();
    util::zk2::eventually("the second's token at the tool", || {
        held(&tool, &iface, &second_id)
    })
    .await;
    let o = call
        .call(
            &addr("lab/ledger"),
            "@op/append",
            &Bindings::new(),
            b"x".to_vec(),
        )
        .await
        .unwrap();
    assert_eq!(answered(&o, "second"), b"second");
    // A standby's close undeclares what it never declared, cleanly.
    let third = up("third").await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(third.role(&iface), Some(zenkey::redundancy::Role::Standby));
    third.close().await.expect("a standby closes");
    second.close().await.unwrap();
}

/// Review on #862 (#841, RD): `gen`'s members of a governed interface
/// follow its standing. Under `external`, the mock's lock lost, it steps
/// down and its member token goes (§2.12); the lock taken again, it takes
/// over, and the member's token is held again, declared once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gen_declares_a_governed_member_again_after_a_step_down() {
    use zenkey::redundancy::{Lock, ManualLock, RedundancyStartup};
    let (_router, ep) = router(None).await;
    let tool = client(&ep).await;
    let l = zenkey_model::contract::load_str(
        "[interface]\nname = \"tank\"\nmajor = 1\nminor = 0\nuses = [\"redundancy.v1\"]\n\
         [defaults]\nannotations = { \"redundancy.election\" = \"external\" }\n\
         [resources.\"level/{tank}\"]\nkind = \"state\"\ntype = { raw = \"text/plain\" }\n\
         params = { tank = \"string\" }\ncardinality = 4\nepoch = \"tank\"\n",
        std::path::Path::new("."),
        None,
    );
    let tank = Arc::new(Revision::from_contract(
        l.contract.unwrap_or_else(|| panic!("{}", l.report)),
        ContractSource::File,
    ));
    let lock = Arc::new(ManualLock::new(true));
    let mut startup = RedundancyStartup::new(client(&ep).await)
        .with_watch(client(&ep).await)
        .with_lock(Arc::clone(&lock) as Arc<dyn Lock>);
    startup.lock_period = Duration::from_millis(200);
    let spec = GenSpec {
        address: addr("lab/tank"),
        revisions: vec![tank],
        members: vec![MemberArg::parse("level/{tank}=t1").unwrap()],
        bindings: BTreeMap::new(),
        rate_hz: Some(5.0),
        pattern: GenPattern::Steady,
        duration: Duration::from_secs(15),
        seed: 3,
        tool: "mock_owner test".into(),
        elect: Some(startup),
    };
    let plan = gen_plan(&spec).unwrap();
    let owner = client(&ep).await;
    let run = tokio::spawn(async move { run_gen(&owner, &spec, &plan, |_| {}).await });
    // The member tokens of `t1` a complete read of the address shows.
    let members = || async {
        let r = zenkey::presence::liveliness_read(&tool, "zk2/lab/tank/@zk/**", T)
            .await
            .ok()?;
        r.complete.then(|| {
            r.keys
                .iter()
                .filter(|k| {
                    matches!(
                        zenkey_model::grammar::parse(k),
                        Ok(zenkey_model::grammar::ZkKey::Member { member, .. }) if member == "t1"
                    )
                })
                .count()
        })
    };
    util::zk2::eventually("t1's member token while it serves", || async {
        members().await == Some(1)
    })
    .await;
    lock.set(false);
    util::zk2::eventually("t1's member token gone at the step-down", || async {
        members().await == Some(0)
    })
    .await;
    lock.set(true);
    util::zk2::eventually("t1's member token again after the takeover", || async {
        members().await == Some(1)
    })
    .await;
    let report = run.await.unwrap().expect("the mock's run");
    assert_eq!(report.roles["tank.v1"], "serving", "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");
}
