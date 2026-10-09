//! The raw observers over a live zk2 deployment (#612, FJ8b): the lens a
//! wire key resolves through, and every projection built on it — rate's
//! groups, field's declared paths, the timeline's lanes and clocks, the
//! snapshot's holders, `check expect`, `check probe`, and the watchdog's
//! `invalid-payload` and `qos-mismatch` rules firing and clearing.
//!
//! One router and client sessions, as the runtime's scenario suites run
//! them: `host-a/tc` implements the tcgui pilot's `tc.netif.v1` on the
//! owners' session (its descriptor states its session zid as `meta.zid`),
//! and the tool reads on sessions of its own. The router stamps what
//! arrives unstamped (zenoh's default for a router), which is what lets the
//! timeline tell the owner's clock from another's.

use std::time::Duration;

use serde_json::json;
use zenkey_fleet::model::render::Member;
use zenkey_fleet::model::stats::{RateAsk, rate_report};
use zenkey_fleet::report::{
    Conformance, ExpectVerdict, Holder, Judgement, KeyGroup, LaneId, Rendered, StamperWire,
    TimelineSource, Unresolved,
};
use zenkey_fleet::{
    BundleStore, Catalog, ContractSet, ExpectAim, ExpectSpec, FleetEvent, Lens, Monitor,
    MonitorSpec, Order, ResolvedTarget, Revision, SnapshotSpec, StreamItem, TimelineRow, Window,
    run_expect, run_probe, take_snapshot, timeline,
};
use zenkey_model::authoring::Kind;
use zenkey_model::template::Bindings;
use zk2::{Implementation, Service, ServiceBuilder};

mod util;
use util::zk2::{T, client, config, eventually, example, iface, router};

const ETH0: &str = "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0";
const BW0: &str = "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0";

/// Polls `f` until it yields a value, failing after [`util::SETTLE`].
async fn until<T, F, Fut>(what: &str, mut f: F) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    loop {
        if let Some(t) = f().await {
            return t;
        }
        assert!(tokio::time::Instant::now() < deadline, "never: {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn member(name: &str) -> Bindings {
    [
        ("ns".to_owned(), vec!["default".to_owned()]),
        ("iface".to_owned(), vec![name.to_owned()]),
    ]
    .into()
}

/// The owner: `host-a/tc` implementing `tc.netif.v1`, every resource
/// exposed, its state served.
async fn owner(session: &zenoh::Session) -> Service {
    let netif = iface("tc.netif.v1");
    let mut b = ServiceBuilder::new(session, config("host-a/tc"));
    b.implement(Implementation::new(example("tcgui/tc.netif.v1")))
        .expect("implement");
    for r in &example("tcgui/tc.netif.v1").resources {
        let _ = b.expose(&netif, &zk2::implementation::resource_name(r));
    }
    b.serve_state(&netif).expect("serve state");
    b.start().await.expect("the owner")
}

/// A deployment and a tool: the router, the owner's service, the tool's
/// session, and the lens read once the owner is visible.
struct Bed {
    _router: zenoh::Session,
    owners: zenoh::Session,
    service: Service,
    tool: zenoh::Session,
    store: BundleStore,
}

async fn bed() -> Bed {
    let (router, ep) = router(None).await;
    let owners = client(&ep).await;
    let service = owner(&owners).await;
    let tool = client(&ep).await;
    let store = BundleStore::new(T);
    Bed {
        _router: router,
        owners,
        service,
        tool,
        store,
    }
}

impl Bed {
    /// The lens's presence read, once the owner's descriptor and contract
    /// are in hand.
    async fn catalog(&self) -> Catalog {
        until("the owner is visible and its contract held", || async {
            let c = zenkey_fleet::read_lens(&self.tool, &self.store, T)
                .await
                .expect("a presence read");
            let ok = Lens::new("", Some(&c), &self.store)
                .resolve(ETH0)
                .resolved
                .is_some();
            ok.then_some(c)
        })
        .await
    }

    fn revision(&self) -> std::sync::Arc<Revision> {
        let rev = Revision::from_contract(
            example("tcgui/tc.netif.v1"),
            zenkey_fleet::report::ContractSource::File,
        );
        std::sync::Arc::new(rev)
    }
}

/// Every rung keeps its own spelling over a live deployment: the owner's
/// key resolves to its resource and decodes, a value that fails its type is
/// named as such, an address presence does not show stops at the provider,
/// and a foreign key is rendered structurally (O1, O2; spec §7.2).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lens_resolves_a_live_deployment_rung_by_rung() {
    let bed = bed().await;
    let catalog = bed.catalog().await;
    let lens = Lens::new("", Some(&catalog), &bed.store);

    let ok = lens.check(
        ETH0,
        Member::Type,
        Some("application/json"),
        br#"{"name":"eth0","index":2,"namespace":"default","is_up":true}"#,
    );
    assert_eq!(ok.conformance, Conformance::Valid);
    assert_eq!(
        ok.identity.group,
        KeyGroup::Resource {
            address: "host-a/tc".into(),
            iface: "tc.netif.v1".into(),
            token: "state".into(),
            resource: Some("state/interfaces/{ns}/{iface}".into()),
        }
    );
    assert!(matches!(ok.rendering.rendered, Rendered::Value { .. }));

    let bad = lens.check(
        ETH0,
        Member::Type,
        Some("application/json"),
        br#"{"name":"eth0","index":2,"namespace":"default","is_up":"yes"}"#,
    );
    assert!(
        matches!(&bad.conformance, Conformance::Invalid { violations } if violations.iter().any(|v| v.contains("/is_up"))),
        "{:?}",
        bad.conformance
    );
    let junk = lens.check(ETH0, Member::Type, Some("application/json"), b"{nope");
    assert!(matches!(junk.conformance, Conformance::Undecodable { .. }));

    let elsewhere = lens.identity("zk2/host-z/tc/tc.netif.v1/state/namespaces");
    assert_eq!(elsewhere.unresolved, Some(Unresolved::NoProvider));
    let foreign = lens.check("rt/chatter", Member::Type, None, b"hello");
    assert!(matches!(
        foreign.rendering.rendered,
        Rendered::Structural {
            why: Unresolved::NotZk2 { .. },
            ..
        }
    ));
    drop(bed.service);
}

/// `rate`'s projection: the window grouped by address and resource, the
/// members of one templated resource in one group, a foreign key in a
/// group of its own — every retained key in exactly one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rate_groups_keys_by_address_and_resource() {
    let bed = bed().await;
    let catalog = bed.catalog().await;
    let netif = iface("tc.netif.v1");
    let w0 = bed
        .service
        .writer(&netif, "stream/bandwidth/{ns}/{iface}", &member("eth0"))
        .await
        .expect("writer");
    let w1 = bed
        .service
        .writer(&netif, "stream/bandwidth/{ns}/{iface}", &member("eth1"))
        .await
        .expect("writer");
    let monitor = Monitor::start(
        &bed.tool,
        MonitorSpec {
            selectors: vec!["zk2/**".into(), "rt/**".into()],
            ..Default::default()
        },
    )
    .await
    .expect("monitor");
    eventually("the writers match", || async {
        w0.matching().await.unwrap_or(false)
    })
    .await;
    for _ in 0..5 {
        w0.put_value(&json!({"stats": {}})).await.expect("put");
        w1.put_value(&json!({"stats": {}})).await.expect("put");
        bed.owners.put("rt/chatter", "x").await.expect("put");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    eventually("every sample counted", || async {
        monitor.core().with_stats(|s| s.totals().0) >= 15
    })
    .await;
    let lens = Lens::new("", Some(&catalog), &bed.store);
    let report = monitor.core().with_stats(|stats| {
        rate_report(
            stats,
            &RateAsk {
                selector: "zk2/**".into(),
                window_s: 1.0,
                per_key: true,
                loss: false,
                latency: false,
            },
            &lens,
        )
    });
    monitor.stop();
    let stream = report
        .groups
        .iter()
        .find(|g| {
            g.group
                == KeyGroup::Resource {
                    address: "host-a/tc".into(),
                    iface: "tc.netif.v1".into(),
                    token: "stream".into(),
                    resource: Some("stream/bandwidth/{ns}/{iface}".into()),
                }
        })
        .unwrap_or_else(|| panic!("{:?}", report.groups));
    assert_eq!((stream.keys, stream.count), (2, 10));
    let foreign = report
        .groups
        .iter()
        .find(|g| g.group == KeyGroup::NotZk2)
        .expect("the foreign key is a group of its own");
    assert_eq!(foreign.count, 5);
    let summed: usize = report.groups.iter().map(|g| g.keys).sum();
    assert_eq!(
        summed, report.keys,
        "every retained key in exactly one group"
    );
    assert_eq!(report.rows.len(), report.keys);
}

/// `field`'s judgement over decoded payloads: a path the declared type
/// declares is marked so, one it never declares is `field-new`, and the
/// type comes from the bundle the owner serves.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn field_judges_paths_against_the_declared_type() {
    let bed = bed().await;
    let catalog = bed.catalog().await;
    let netif = iface("tc.netif.v1");
    let writer = bed
        .service
        .writer(&netif, "state/interfaces/{ns}/{iface}", &member("eth0"))
        .await
        .expect("writer");
    let lens = Lens::new("", Some(&catalog), &bed.store);
    let spec = zenkey_fleet::FieldSpec {
        selector: "zk2/host-a/tc/tc.netif.v1/state/**".into(),
        window: Duration::from_millis(1500),
        max_paths: 64,
    };
    let run = zenkey_fleet::run_field(&bed.tool, &lens, &spec);
    let publish = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        for i in 0..5 {
            let _ = writer
                .put_value(&json!({"name": "eth0", "mtu_hint": i, "is_up": true}))
                .await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };
    let (report, ()) = tokio::join!(run, publish);
    let report = report.expect("a field run");
    assert!(report.samples >= 5, "{report:?}");
    let row = |path: &str| {
        report
            .rows
            .iter()
            .find(|r| r.path == path)
            .unwrap_or_else(|| panic!("{path}: {:?}", report.rows))
    };
    assert_eq!(row("name").declared, Some(true));
    assert_eq!(row("mtu_hint").declared, Some(false));
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.check == zenkey_fleet::report::FieldCheck::New
                && f.subject.ends_with("· mtu_hint")),
        "{:?}",
        report.findings
    );
    assert!(
        report
            .findings
            .iter()
            .all(|f| !f.subject.ends_with("· name")),
        "a declared path is not drift"
    );
}

/// The timeline's lanes are zk2 resources, and its clocks are named: a
/// state put is stamped by its owner (S1), whose descriptor states the zid,
/// while a stream sample arrives unstamped and the router stamps it —
/// another clock. Two stampers, so the HLC axis claims no happened-before.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_timeline_names_the_owners_clock_and_the_routers() {
    let mut bed = bed().await;
    let catalog = bed.catalog().await;
    let netif = iface("tc.netif.v1");
    // The owner's state writer stamps every put (S1); the stream writer
    // does not, and the router does.
    let state = bed
        .service
        .state_writer(&netif, "state/interfaces/{ns}/{iface}", &member("eth0"))
        .await
        .expect("state writer");
    let stream = bed
        .service
        .writer(&netif, "stream/bandwidth/{ns}/{iface}", &member("eth0"))
        .await
        .expect("stream writer");
    let lens = Lens::new("", Some(&catalog), &bed.store);
    let monitor = Monitor::start(&bed.tool, MonitorSpec::default())
        .await
        .expect("monitor");
    let mut events = monitor.events();
    let monitor = monitor.watching(["zk2/**"]).await.expect("watch");
    let epoch = std::time::Instant::now();
    eventually("the writers match", || async {
        state.writer().matching().await.unwrap_or(false) && stream.matching().await.unwrap_or(false)
    })
    .await;
    for _ in 0..3 {
        state
            .put_value(&json!({"name": "eth0", "index": 2, "namespace": "default", "is_up": true}))
            .await
            .expect("state put");
        stream
            .put_value(&json!({"stats": {}}))
            .await
            .expect("stream put");
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    let mut rows = Vec::new();
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    while rows.len() < 6 && tokio::time::Instant::now() < deadline {
        if let Ok(Some(StreamItem::Event(FleetEvent::Sample(s)))) =
            tokio::time::timeout(Duration::from_millis(500), events.recv()).await
        {
            rows.push(TimelineRow::from_view(&s, epoch, &lens));
        }
    }
    monitor.shutdown().await.expect("teardown");
    assert_eq!(rows.len(), 6, "{rows:?}");
    let window = Window {
        rows,
        breaks: Vec::new(),
        scopes: vec!["zk2/**".into()],
        window_s: Some(1.0),
        source: TimelineSource::Live,
        keys_evicted: 0,
        lens: lens.scope(),
    };
    let report = timeline(&window, Order::Hlc);
    let lane = |resource: &str| {
        report
            .lanes
            .iter()
            .find(
                |l| matches!(&l.lane, LaneId::Resource { resource: Some(r), .. } if r == resource),
            )
            .unwrap_or_else(|| panic!("{resource}: {:?}", report.lanes))
    };
    let state_lane = lane("state/interfaces/{ns}/{iface}");
    assert_eq!(state_lane.provenance.owner, 3, "{state_lane:?}");
    let stream_lane = lane("stream/bandwidth/{ns}/{iface}");
    assert_eq!(
        stream_lane.provenance.other, 3,
        "the router's clock, not the owner's: {stream_lane:?}"
    );
    assert!(matches!(
        report.axis,
        zenkey_fleet::report::AxisLabel::Hlc {
            claim: zenkey_fleet::report::HlcClaim::SkewedWallClock { .. }
        }
    ));
}

/// A snapshot is the owners' state (S4): each row resolved to its
/// resource, its payload checked against its type, its stamp the owner's
/// and its holder live, answered by the owner.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_snapshot_takes_the_owners_state_with_its_holders() {
    let mut bed = bed().await;
    let netif = iface("tc.netif.v1");
    let eth0 = bed
        .service
        .state_writer(&netif, "state/interfaces/{ns}/{iface}", &member("eth0"))
        .await
        .expect("state writer");
    let eth1 = bed
        .service
        .state_writer(&netif, "state/interfaces/{ns}/{iface}", &member("eth1"))
        .await
        .expect("state writer");
    eth0.put_value(&json!({"name": "eth0", "index": 2, "namespace": "default", "is_up": true}))
        .await
        .expect("put");
    eth1.put(br#"{"name":"eth1","index":3,"namespace":"default","is_up":"maybe"}"#.to_vec())
        .await
        .expect("put");
    let catalog = bed.catalog().await;
    let lens = Lens::new("", Some(&catalog), &bed.store);
    let spec = SnapshotSpec {
        selectors: vec!["zk2/*/*/*/state/**".into()],
        timeout: T,
        max_replies: 1000,
    };
    let snapshot = until("both keys answer", || async {
        let t = take_snapshot(&bed.tool, &lens, &spec).await.expect("take");
        (t.snapshot.rows.len() >= 2).then_some(t)
    })
    .await
    .snapshot;
    let row = |suffix: &str| {
        snapshot
            .rows
            .iter()
            .find(|r| r.key.ends_with(suffix))
            .unwrap_or_else(|| panic!("{suffix}: {:?}", snapshot.rows))
    };
    let good = row("/default/eth0");
    assert_eq!(good.conformance, Conformance::Valid);
    assert!(
        matches!(good.stamper, Some(StamperWire::Owner { .. })),
        "{good:?}"
    );
    assert!(
        matches!(&good.holder, Holder::Live { address, .. } if address == "host-a/tc"),
        "{good:?}"
    );
    let bad = row("/default/eth1");
    assert!(bad.conformance.is_violation(), "{bad:?}");
    let report = zenkey_fleet::snapshot_report(&snapshot, None, vec![]);
    assert_eq!(report.nonconforming, 1);
    assert_eq!(
        report.header.presence.as_ref().map(|p| p.complete),
        Some(true),
        "the presence read behind the holders rides the header"
    );
}

/// `check expect` over a zk2 address and resource: samples that decode and
/// validate meet `--valid-payload`; one that fails its type does not; the
/// owner present meets `--present`, an address no token names does not —
/// worded as what this reader could see.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expect_judges_samples_values_and_presence() {
    let bed = bed().await;
    let _ = bed.catalog().await;
    let rev = bed.revision();
    let netif = iface("tc.netif.v1");
    let writer = bed
        .service
        .writer(&netif, "stream/bandwidth/{ns}/{iface}", &member("eth0"))
        .await
        .expect("writer");
    let target = ResolvedTarget::parse("host-a/tc").expect("target");
    let r = rev
        .resource("bandwidth/{ns}/{iface}", &[Kind::Stream])
        .expect("a stream");
    let values = Bindings::new();
    let publish = |body: serde_json::Value| {
        let writer = &writer;
        async move {
            for _ in 0..30 {
                let _ = writer.put_value(&body).await;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    };
    let spec = ExpectSpec {
        within: Duration::from_secs(3),
        count: Some(2),
        valid_payload: true,
        qos_declared: true,
        present: true,
        ..ExpectSpec::default()
    };
    let aim = || ExpectAim {
        revision: &rev,
        target: &target,
        resource: Some(r),
        values: &values,
    };
    let (met, ()) = tokio::join!(
        run_expect(&bed.tool, aim(), &spec),
        publish(json!({"stats": {"rx_bytes": 1}}))
    );
    let met = met.expect("a window");
    assert_eq!(met.verdict, ExpectVerdict::Met, "{met:?}");
    assert!(
        met.presence
            .as_ref()
            .is_some_and(|p| p.holders == ["host-a/tc"]),
        "{met:?}"
    );

    let (bad, ()) = tokio::join!(
        run_expect(&bed.tool, aim(), &spec),
        publish(json!({"no_stats": true}))
    );
    let bad = bad.expect("a window");
    assert_eq!(bad.verdict, ExpectVerdict::NotMet, "{bad:?}");
    assert!(
        bad.violations.iter().any(|v| v.contains("invalid")),
        "{bad:?}"
    );

    let nobody = ResolvedTarget::parse("host-z/tc").expect("target");
    let absent = run_expect(
        &bed.tool,
        ExpectAim {
            revision: &rev,
            target: &nobody,
            resource: None,
            values: &values,
        },
        &ExpectSpec {
            within: Duration::from_millis(800),
            present: true,
            ..ExpectSpec::default()
        },
    )
    .await
    .expect("a window");
    assert_eq!(absent.verdict, ExpectVerdict::NotMet, "{absent:?}");
    assert!(
        absent
            .unmet
            .iter()
            .any(|u| u.contains("visible to this reader")),
        "{absent:?}"
    );
}

/// `check probe` reads as a consumer does: the owner's current state
/// arrives at once (S4) and conforms; an address nobody holds gives
/// nothing, and the silence is attributed — no token visible to this
/// reader, the finding.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_probe_hears_values_or_attributes_its_silence() {
    let mut bed = bed().await;
    let netif = iface("tc.netif.v1");
    let eth0 = bed
        .service
        .state_writer(&netif, "state/interfaces/{ns}/{iface}", &member("eth0"))
        .await
        .expect("state writer");
    eth0.put_value(&json!({"name": "eth0", "index": 2, "namespace": "default", "is_up": true}))
        .await
        .expect("put");
    let _ = bed.catalog().await;
    let rev = bed.revision();
    let r = rev
        .resource("interfaces/{ns}/{iface}", &[Kind::State])
        .expect("state");
    let values = Bindings::new();
    let target = ResolvedTarget::parse("host-a/tc").expect("target");
    let heard = until("the probe hears the owner", || async {
        let p = run_probe(
            &bed.tool,
            ExpectAim {
                revision: &rev,
                target: &target,
                resource: Some(r),
                values: &values,
            },
            Duration::from_millis(800),
            T,
        )
        .await
        .expect("a probe");
        matches!(p.verdict, Judgement::NotEstablished { .. }).then_some(p)
    })
    .await;
    assert!(
        heard.current.as_ref().is_some_and(|c| c.conforming >= 1),
        "{heard:?}"
    );

    let nobody = ResolvedTarget::parse("host-z/tc").expect("target");
    let silent = run_probe(
        &bed.tool,
        ExpectAim {
            revision: &rev,
            target: &nobody,
            resource: Some(r),
            values: &values,
        },
        Duration::from_millis(500),
        T,
    )
    .await
    .expect("a probe");
    assert_eq!(silent.verdict, Judgement::Established, "{silent:?}");
    let p = silent
        .presence
        .expect("silence is attributed through presence");
    assert!(p.holders.is_empty() && p.complete, "{p:?}");
}

/// The watchdog's `invalid-payload` and `qos-mismatch` in their zk2
/// meaning: good values on the owner's key judge `ok`; a value that fails
/// its type fires the first, a put at another priority than the resource
/// declares fires the second; back to good, both clear.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invalid_payload_and_qos_mismatch_fire_and_clear() {
    use zenkey_fleet::Sipper as _;
    use zenkey_fleet::judge::condition::{Condition, WatchdogSpec, watchdog};
    use zenkey_fleet::report::CondState;

    let bed = bed().await;
    let _ = bed.catalog().await;
    let netif = iface("tc.netif.v1");
    let writer = bed
        .service
        .writer(&netif, "stream/bandwidth/{ns}/{iface}", &member("eth0"))
        .await
        .expect("writer");
    let bus = zenkey_fleet::DoctorBus {
        session: bed.tool.clone(),
        raw: bed.tool.clone(),
        namespace: String::new(),
    };
    let spec = WatchdogSpec {
        rules: vec![
            Condition::parse(&format!("invalid-payload {BW0}")).expect("rule"),
            Condition::parse(&format!("qos-mismatch {BW0}")).expect("rule"),
        ],
        tick: Duration::from_millis(400),
        ticks: Some(12),
        timeout: T,
    };
    let contracts = ContractSet::new();
    let store = bed.store.clone();
    let owners = bed.owners.clone();
    let publish = async move {
        // Phases of four ticks each: good, bad, good.
        for phase in 0..3 {
            let until = tokio::time::Instant::now() + Duration::from_millis(1600);
            while tokio::time::Instant::now() < until {
                if phase == 1 {
                    let _ = owners
                        .put(BW0, r#"{"not_stats":1}"#)
                        .priority(zenoh::qos::Priority::RealTime)
                        .await;
                } else {
                    let _ = writer.put_value(&json!({"stats": {}})).await;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    };
    let run = async {
        let mut run = watchdog(&bus, &store, &contracts, &spec).pin();
        let mut out = Vec::new();
        while let Some(t) = run.sip().await {
            out.push(t);
        }
        (out, run.await)
    };
    let ((transitions, summary), ()) = tokio::join!(run, publish);
    summary.expect("the run");
    for rule in ["invalid-payload", "qos-mismatch"] {
        let states: Vec<CondState> = transitions
            .iter()
            .filter(|t| t.rule.starts_with(rule))
            .map(|t| t.to)
            .collect();
        assert!(
            states.contains(&CondState::Firing),
            "{rule} fired: {transitions:#?}"
        );
        let fired = states
            .iter()
            .position(|s| *s == CondState::Firing)
            .expect("fired");
        assert!(
            states[fired..].contains(&CondState::Ok),
            "{rule} cleared after firing: {transitions:#?}"
        );
    }
    let qos = transitions
        .iter()
        .find(|t| t.rule.starts_with("qos-mismatch") && t.to == CondState::Firing)
        .expect("qos fired");
    assert!(
        qos.evidence
            .contains("priority declared data, observed real_time"),
        "{}",
        qos.evidence
    );
    let invalid = transitions
        .iter()
        .find(|t| t.rule.starts_with("invalid-payload") && t.to == CondState::Firing)
        .expect("invalid fired");
    assert!(invalid.evidence.contains("stats"), "{}", invalid.evidence);
}
