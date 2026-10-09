//! The fleet's zk2 core against live services (#612, FJ3): services brought
//! up with the runtime's own `ServiceBuilder` from the tcgui pilot's and the
//! walkthrough's contracts (`examples/zk2/`), read back the way zenctl will
//! read them — presence, descriptors, bundles — by a tool that was never
//! compiled against any of them.
//!
//! One router and client sessions, as the runtime's scenario suites do it:
//! the owners on one session, the tool on another, nothing named by port.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

mod util;
use util::zk2::{T, client, config, eventually, example, iface, router};

use zenkey::{Implementation, Service, ServiceBuilder};
use zenkey_fleet::bus::contracts::BundleStore;
use zenkey_fleet::bus::presence::{Scope, observe, service_listing};
use zenkey_fleet::model::catalog::{Catalog, ContractState, Contracts};
use zenkey_fleet::model::render::{Member, render, render_with};
use zenkey_fleet::report::{Asked, ContractAnswer, ContractSource, DescriptorAnswer, Rendered};
use zenkey_model::grammar::ZkKey;
use zenkey_model::template::Bindings;

/// Brings up `address` implementing every contract in `paths`, every
/// resource exposed (an operation exposed is answered `unavailable` until
/// served, O3; a resource gated on a capability not held stays absent).
async fn bring_up(session: &zenoh::Session, cfg: zenkey::ServiceConfig, paths: &[&str]) -> Service {
    let mut b = ServiceBuilder::new(session, cfg);
    for path in paths {
        let c = example(path);
        let id = c.iface.clone();
        let names: Vec<String> = c
            .resources
            .iter()
            .map(zenkey::implementation::resource_name)
            .collect();
        b.implement(Implementation::new(c)).expect("implement");
        for n in names {
            let _ = b.expose(&id, &n);
        }
    }
    b.start().await.expect("start")
}

/// The deployment every test below reads: the tcgui pilot (two hosts' tc
/// backends and the frontend that binds them) and the walkthrough's
/// perception pipeline (a camera with `health.v1` in its tokenless set, a
/// detector bound to it, a tracker that only consumes).
struct Deployment {
    _services: Vec<Service>,
}

async fn deployment(owners: &zenoh::Session) -> Deployment {
    let mut services = Vec::new();
    for host in ["host-a", "host-b"] {
        services.push(
            bring_up(
                owners,
                config(&format!("{host}/tc")),
                &["tcgui/tc.netif.v1", "tcgui/tc.netem.v1"],
            )
            .await,
        );
    }
    let mut b = ServiceBuilder::new(
        owners,
        config("ws-01/tcgui-frontend").bind("netif", &["*/tc"]),
    );
    b.require("netif", iface("tc.netif.v1"), false);
    services.push(b.start().await.expect("frontend"));

    services.push(
        bring_up(
            owners,
            config("vehicle-01/cam-front").tokenless(iface("health.v1")),
            &["walkthrough/camera.v1", "walkthrough/health.v1"],
        )
        .await,
    );
    services.push(
        bring_up(
            owners,
            config("vehicle-01/detector").bind("input", &["vehicle-01/cam-front"]),
            &["walkthrough/detections.v1"],
        )
        .await,
    );
    let mut b = ServiceBuilder::new(
        owners,
        config("vehicle-01/tracker").bind("sources", &["vehicle-01/*"]),
    );
    b.require("sources", iface("detections.v1"), false);
    services.push(b.start().await.expect("tracker"));
    Deployment {
        _services: services,
    }
}

const ADDRESSES: [&str; 6] = [
    "host-a/tc",
    "host-b/tc",
    "vehicle-01/cam-front",
    "vehicle-01/detector",
    "vehicle-01/tracker",
    "ws-01/tcgui-frontend",
];

/// A tool acts on presence, never on an owner's `start()` returning: waits
/// until every address is listed with its descriptor served.
async fn settled(tool: &zenoh::Session) {
    eventually("every service listed, every descriptor served", || async {
        let Ok(l) = service_listing(tool, &Scope::all(), T).await else {
            return false;
        };
        let seen: BTreeSet<&str> = l.services.iter().map(|s| s.address.as_str()).collect();
        seen == BTreeSet::from(ADDRESSES)
            && l.services.iter().all(|s| {
                s.instances
                    .iter()
                    .all(|i| matches!(i.descriptor, Asked::Asked(DescriptorAnswer::Served { .. })))
            })
    })
    .await;
}

/// `service list` from presence: every service, consumers included; each
/// interface carries its token's prefix and its descriptor's fingerprint,
/// which agree; the tokenless interface has a descriptor row and no token;
/// a system scope reads that system alone; and the read is complete.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn services_are_listed_from_tokens_and_descriptors() {
    let (_router, ep) = router(None).await;
    let owners = client(&ep).await;
    let tool = client(&ep).await;
    let _d = deployment(&owners).await;
    settled(&tool).await;

    let listing = service_listing(&tool, &Scope::all(), T)
        .await
        .expect("listing");
    assert!(listing.complete, "a GET that returned before its timeout");
    assert!(listing.unparsed.is_empty(), "{:?}", listing.unparsed);
    assert_eq!(listing.selector, "zk2/*/*/@zk/**");

    let netif = Implementation::new(example("tcgui/tc.netif.v1"));
    let netem = Implementation::new(example("tcgui/tc.netem.v1"));
    let host_a = listing
        .services
        .iter()
        .find(|s| s.address == "host-a/tc")
        .expect("host-a");
    assert_eq!(host_a.instances.len(), 1);
    let inst = &host_a.instances[0];
    assert!(inst.instance_token);
    let rows: Vec<(&str, Option<&str>, Option<&str>, bool)> = inst
        .interfaces
        .iter()
        .map(|r| {
            (
                r.iface.as_str(),
                r.token.as_deref(),
                r.contract.as_deref(),
                r.tokenless,
            )
        })
        .collect();
    let fp16 = |i: &Implementation| i.fingerprint().hex().fp16().to_string();
    assert_eq!(
        rows,
        [
            (
                "tc.netem.v1",
                Some(fp16(&netem).as_str()),
                Some(netem.fingerprint().to_string().as_str()),
                false
            ),
            (
                "tc.netif.v1",
                Some(fp16(&netif).as_str()),
                Some(netif.fingerprint().to_string().as_str()),
                false
            ),
        ]
    );

    let cam = listing
        .services
        .iter()
        .find(|s| s.address == "vehicle-01/cam-front")
        .expect("camera");
    let health = cam.instances[0]
        .interfaces
        .iter()
        .find(|r| r.iface == "health.v1")
        .expect("health.v1, from the descriptor alone");
    assert!(health.tokenless && health.token.is_none() && health.contract.is_some());

    let tracker = listing
        .services
        .iter()
        .find(|s| s.address == "vehicle-01/tracker")
        .expect("a pure consumer holds an instance token too (§8.1)");
    assert!(tracker.instances[0].instance_token);
    assert!(tracker.instances[0].interfaces.is_empty());

    let vehicle = service_listing(&tool, &Scope::system("vehicle-01").expect("a system"), T)
        .await
        .expect("listing");
    let seen: Vec<&str> = vehicle
        .services
        .iter()
        .map(|s| s.address.as_str())
        .collect();
    assert_eq!(
        seen,
        [
            "vehicle-01/cam-front",
            "vehicle-01/detector",
            "vehicle-01/tracker"
        ]
    );
}

/// The graph a tool draws is the graph the runtime computes from the same
/// descriptors and tokens (`zenkey::presence::edges`), with each edge's
/// interface joined from the consumer's descriptor.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_graph_equals_the_runtimes_edges() {
    let (_router, ep) = router(None).await;
    let owners = client(&ep).await;
    let tool = client(&ep).await;
    let _d = deployment(&owners).await;
    settled(&tool).await;

    let catalog = Catalog::new(&observe(&tool, &Scope::all(), T).await.expect("observe"));
    let graph = catalog.graph();
    assert!(graph.complete && graph.undescribed.is_empty());

    // The runtime's own reading, independently of the catalog.
    let tokens = zenkey::presence::tokens(&tool, "zk2/*/*/@zk/**", T)
        .await
        .expect("tokens");
    let mut descriptors = Vec::new();
    for k in &tokens {
        if let ZkKey::Instance { addr, instance } = k {
            descriptors.push(
                zenkey::presence::descriptor(&tool, addr, instance, T)
                    .await
                    .expect("descriptor GET")
                    .into_descriptor()
                    .expect("a descriptor"),
            );
        }
    }
    let runtime: BTreeSet<(String, String, String)> =
        zenkey::presence::edges(&descriptors, &tokens)
            .into_iter()
            .map(|e| (e.consumer, e.role, e.provider))
            .collect();
    let drawn: BTreeSet<(String, String, String)> = graph
        .edges
        .iter()
        .map(|e| (e.consumer.clone(), e.role.clone(), e.provider.clone()))
        .collect();
    assert_eq!(drawn, runtime);

    let with_iface: BTreeSet<(&str, &str, &str, &str)> = graph
        .edges
        .iter()
        .map(|e| {
            (
                e.consumer.as_str(),
                e.role.as_str(),
                e.interface.as_str(),
                e.provider.as_str(),
            )
        })
        .collect();
    assert_eq!(
        with_iface,
        BTreeSet::from([
            (
                "vehicle-01/detector",
                "input",
                "camera.v1",
                "vehicle-01/cam-front"
            ),
            (
                "vehicle-01/tracker",
                "sources",
                "detections.v1",
                "vehicle-01/detector"
            ),
            ("ws-01/tcgui-frontend", "netif", "tc.netif.v1", "host-a/tc"),
            ("ws-01/tcgui-frontend", "netif", "tc.netif.v1", "host-b/tc"),
        ])
    );
    let tracker = graph
        .nodes
        .iter()
        .find(|n| n.address == "vehicle-01/tracker")
        .expect("a pure consumer is a node");
    assert!(tracker.provides.is_empty());
    assert_eq!(tracker.requires["sources"].bindings, ["vehicle-01/*"]);
}

/// `iface show`: providers by token and, for the tokenless set, by
/// descriptor; the roles bound to the interface; each revision's contract
/// retrieved from the bus once, its exposure computed by the compact rule.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_interface_is_shown_with_its_contract_retrieved() {
    let (_router, ep) = router(None).await;
    let owners = client(&ep).await;
    let tool = client(&ep).await;
    let _d = deployment(&owners).await;
    settled(&tool).await;

    let catalog = Catalog::new(&observe(&tool, &Scope::all(), T).await.expect("observe"));
    let store = BundleStore::new(T);
    let wanted = catalog.wanted();
    let fetched = store.fetch_all(&tool, &wanted).await;
    assert!(
        fetched
            .iter()
            .all(|r| matches!(r, Ok(ContractState::Held(_)))),
        "every revision a descriptor names is held by its owner"
    );
    let retrieved = store.retrievals();
    assert_eq!(retrieved, wanted.len() as u64, "one retrieval per revision");

    let netif = catalog.iface(&iface("tc.netif.v1"), &store);
    let providers: Vec<&str> = netif.providers.iter().map(|p| p.address.as_str()).collect();
    assert_eq!(providers, ["host-a/tc", "host-b/tc"]);
    assert_eq!(netif.revisions.len(), 1, "both hosts at one revision");
    match &netif.revisions[0].contract {
        Asked::Asked(ContractAnswer::Held { source, contract }) => {
            assert_eq!(*source, ContractSource::Bus);
            assert_eq!(contract.iface, "tc.netif.v1");
            assert!(contract.minor.is_none(), "a bundle carries no minor");
        }
        other => panic!("{other:?}"),
    }
    let names: BTreeSet<String> = example("tcgui/tc.netif.v1")
        .resources
        .iter()
        .map(zenkey::implementation::resource_name)
        .collect();
    for p in &netif.providers {
        let Asked::Asked(exposes) = &p.exposes else {
            panic!("exposure is computed when the revision is held: {p:?}");
        };
        assert_eq!(exposes.iter().cloned().collect::<BTreeSet<_>>(), names);
    }
    assert_eq!(netif.consumers.len(), 1);
    assert_eq!(netif.consumers[0].address, "ws-01/tcgui-frontend");
    assert_eq!(netif.consumers[0].bindings, ["*/tc"]);

    let health = catalog.iface(&iface("health.v1"), &store);
    assert_eq!(health.providers.len(), 1);
    assert!(health.providers[0].tokenless && health.providers[0].token.is_none());

    // Asked again, the store answers from what it holds.
    let again = store.fetch_all(&tool, &wanted).await;
    assert!(again.iter().all(Result::is_ok));
    assert_eq!(store.retrievals(), retrieved, "nothing retrieved twice");
}

/// Bundles are retrieved once and cached: a counting holder sees one
/// query per revision however often, and however concurrently, the store
/// is asked; an unavailable revision is answered from the cache inside
/// its TTL; a holder serving the wrong bytes is refused, never accepted
/// (§8.4, §9.6).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bundles_are_retrieved_once_and_cached() {
    let (_router, ep) = router(None).await;
    let holder = client(&ep).await;
    let tool = client(&ep).await;

    let imp = Implementation::new(example("walkthrough/camera.v1"));
    let key = zenkey_model::grammar::contract_key(imp.iface(), imp.fingerprint().hex())
        .expect("a contract key");
    let asked = Arc::new(Mutex::new(0usize));
    let (count, bytes, reply_key) = (
        Arc::clone(&asked),
        Arc::clone(imp.bundle_bytes()),
        key.clone().into_keyexpr(),
    );
    let _holder = holder
        .declare_queryable(key.into_keyexpr())
        .complete(true)
        .callback(move |q| {
            use zenoh::Wait;
            *count.lock().expect("count") += 1;
            let _ = q.reply(reply_key.clone(), bytes.to_vec()).wait();
        })
        .await
        .expect("holder");

    let store = BundleStore::new(T);
    eventually("the holder is reachable", || async {
        matches!(
            store.fetch(&tool, imp.iface(), imp.fingerprint()).await,
            Ok(ContractState::Held(_))
        ) || {
            store.forget_absences();
            false
        }
    })
    .await;
    let after_first = *asked.lock().expect("count");
    let retrievals = store.retrievals();

    let mut tasks = Vec::new();
    for _ in 0..8 {
        let (store, tool, i, fp) = (
            store.clone(),
            tool.clone(),
            imp.iface().clone(),
            imp.fingerprint().clone(),
        );
        tasks.push(tokio::spawn(
            async move { store.contract(&tool, &i, &fp).await },
        ));
    }
    for t in tasks {
        let c = t.await.expect("task").expect("fetch").expect("a contract");
        assert_eq!(c.iface, *imp.iface());
    }
    assert_eq!(store.retrievals(), retrievals, "served from the cache");
    assert_eq!(
        *asked.lock().expect("count"),
        after_first,
        "no query reached the holder"
    );
    let held = store
        .state(imp.iface(), imp.fingerprint())
        .expect("asked about");
    assert_eq!(
        held.revision().expect("held").fingerprint(),
        imp.fingerprint()
    );

    // A revision nobody holds: unavailable, and cached within its TTL.
    let ghost = zenkey_model::canonical::Fingerprint::parse(&format!("sha256:{}", "0".repeat(64)))
        .expect("a fingerprint");
    let before = store.retrievals();
    for _ in 0..3 {
        match store
            .fetch(&tool, imp.iface(), &ghost)
            .await
            .expect("fetch")
        {
            ContractState::Unavailable { refused } => assert!(refused.is_empty(), "{refused:?}"),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(store.retrievals(), before + 1);

    // A holder serving bytes that are not the bundle: refused by tag.
    let liar_fp =
        zenkey_model::canonical::Fingerprint::parse(&format!("sha256:{}", "1".repeat(64)))
            .expect("a fingerprint");
    let liar_key = zenkey_model::grammar::contract_key(imp.iface(), liar_fp.hex())
        .expect("a contract key")
        .into_keyexpr();
    let reply_key = liar_key.clone();
    let real = Arc::clone(imp.bundle_bytes());
    let _liar = holder
        .declare_queryable(liar_key)
        .complete(true)
        .callback(move |q| {
            use zenoh::Wait;
            let _ = q.reply(reply_key.clone(), real.to_vec()).wait();
        })
        .await
        .expect("liar");
    eventually("the liar is refused", || async {
        store.forget_absences();
        match store.fetch(&tool, imp.iface(), &liar_fp).await {
            Ok(ContractState::Unavailable { refused }) => {
                !refused.is_empty() && refused.iter().all(|t| t == "fingerprint")
            }
            Ok(ContractState::Held(_)) => panic!("an unverified bundle was accepted"),
            _ => false,
        }
    })
    .await;
}

/// Each kind decodes or renders honestly (§7.2), on samples as they arrive
/// at a tool: protobuf through the bundle's descriptor set, JSON and CBOR
/// field by field, raw as its media type (a family's concrete subtype from
/// the sample's `Encoding`), and bytes that are not their declared type as
/// that type and the reason.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn each_kind_decodes_or_renders_honestly() {
    let (_router, ep) = router(None).await;
    let owners = client(&ep).await;
    let tool = client(&ep).await;
    let none = Bindings::new();
    let values = |pairs: &[(&str, &str)]| -> Bindings {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), vec![(*v).to_owned()]))
            .collect()
    };

    // The owners: a camera (raw image, protobuf attachment), a detector
    // (protobuf), a tc backend (JSON) and parallax (a raw family, CBOR
    // attachment).
    let camera = iface("camera.v1");
    let mut b = ServiceBuilder::new(&owners, config("vehicle-01/cam-front"));
    b.implement(Implementation::new(example("walkthrough/camera.v1")))
        .expect("camera");
    let frames = b
        .declare_writer(&camera, "@stream/image", &none)
        .await
        .expect("writer");
    b.expose(&camera, "state/info").expect("info");
    let _cam = b.start().await.expect("camera");

    let det = iface("detections.v1");
    let mut b = ServiceBuilder::new(
        &owners,
        config("vehicle-01/detector").bind("input", &["vehicle-01/cam-front"]),
    );
    b.implement(Implementation::new(example("walkthrough/detections.v1")))
        .expect("detections");
    let objects = b
        .declare_writer(&det, "stream/objects", &none)
        .await
        .expect("writer");
    let _det = b.start().await.expect("detector");

    let netif = iface("tc.netif.v1");
    let mut b = ServiceBuilder::new(&owners, config("host-a/tc"));
    let c = example("tcgui/tc.netif.v1");
    let names: Vec<String> = c
        .resources
        .iter()
        .map(zenkey::implementation::resource_name)
        .collect();
    b.implement(Implementation::new(c)).expect("netif");
    let bandwidth = b
        .declare_writer(
            &netif,
            "stream/bandwidth/{ns}/{iface}",
            &values(&[("ns", "default"), ("iface", "eth0")]),
        )
        .await
        .expect("writer");
    for n in names {
        let _ = b.expose(&netif, &n);
    }
    let _tc = b.start().await.expect("tc");

    let parallax = iface("zs.parallax.v1");
    let mut b = ServiceBuilder::new(&owners, config("h-3fa9c2d41b7e/parallax"));
    let c = example("zensight/zs.parallax.v1");
    let names: Vec<String> = c
        .resources
        .iter()
        .map(zenkey::implementation::resource_name)
        .collect();
    b.implement(Implementation::new(c)).expect("parallax");
    let video = b
        .declare_writer(
            &parallax,
            "@stream/streams/{stream}/video/{codec}/{tier}",
            &values(&[("stream", "cam0"), ("codec", "h264"), ("tier", "high")]),
        )
        .await
        .expect("writer");
    for n in names {
        let _ = b.expose(&parallax, &n);
    }
    let _parallax = b.start().await.expect("parallax");

    // The tool: one plain subscriber per key, recording what arrives.
    type Got = Arc<Mutex<Vec<(String, String, Vec<u8>, Option<Vec<u8>>)>>>;
    let got: Got = Arc::default();
    let mut subs = Vec::new();
    for key in [
        "zk2/vehicle-01/cam-front/camera.v1/@stream/image",
        "zk2/vehicle-01/detector/detections.v1/stream/objects",
        "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0",
        "zk2/h-3fa9c2d41b7e/parallax/zs.parallax.v1/@stream/streams/cam0/video/h264/high",
    ] {
        let g = Arc::clone(&got);
        subs.push(
            tool.declare_subscriber(key)
                .callback(move |s| {
                    g.lock().expect("got").push((
                        s.key_expr().as_str().to_owned(),
                        s.encoding().to_string(),
                        s.payload().to_bytes().into_owned(),
                        s.attachment().map(|a| a.to_bytes().into_owned()),
                    ));
                })
                .await
                .expect("subscriber"),
        );
    }

    // FrameMeta { captured_at_ns: 5, sequence: 7, width: 640, height: 480 }.
    let frame_meta = vec![0x08, 5, 0x10, 7, 0x18, 0x80, 0x05, 0x20, 0xe0, 0x03];
    // Objects { frame_sequence: 3 }.
    let detected = vec![0x08, 3];
    let mut video_meta = Vec::new();
    ciborium::into_writer(
        &serde_json::json!({"keyframe": true, "pts_ns": 40}),
        &mut video_meta,
    )
    .expect("cbor");
    eventually("a sample of every kind reached the tool", || async {
        frames
            .put_with(vec![0xff, 0xd8, 0xff], Some(frame_meta.clone()))
            .await
            .expect("frame");
        objects.put(detected.clone()).await.expect("objects");
        bandwidth
            .put_value(&serde_json::json!({"rx_bps": 1, "tx_bps": 2}))
            .await
            .expect("bandwidth");
        video
            .put_with(vec![0, 0, 0, 1, 0x65], Some(video_meta.clone()))
            .await
            .expect("video");
        let keys: BTreeSet<String> = got
            .lock()
            .expect("got")
            .iter()
            .map(|(k, ..)| k.clone())
            .collect();
        keys.len() == 4
    })
    .await;

    // Presence, descriptors, bundles: the tool's whole knowledge.
    eventually("every owner described", || async {
        observe(&tool, &Scope::all(), T)
            .await
            .is_ok_and(|o| Catalog::new(&o).wanted().len() == 4)
    })
    .await;
    let catalog = Catalog::new(&observe(&tool, &Scope::all(), T).await.expect("observe"));
    assert_eq!(catalog.wanted().len(), 4);
    let store = BundleStore::new(T);
    store.fetch_all(&tool, &catalog.wanted()).await;

    let samples = got.lock().expect("got").clone();
    let sample = |suffix: &str| {
        samples
            .iter()
            .find(|(k, ..)| k.ends_with(suffix))
            .cloned()
            .expect("a sample")
    };

    // Raw, with a protobuf attachment.
    let (key, enc, payload, attachment) = sample("@stream/image");
    assert_eq!(enc, "image/jpeg");
    let r = render(&catalog, &store, &key, Member::Type, Some(&enc), &payload);
    assert_eq!(
        r.rendered,
        Rendered::Opaque {
            media_type: "image/jpeg".into()
        }
    );
    assert_eq!(r.size, 3);
    let r = render(
        &catalog,
        &store,
        &key,
        Member::Attachment,
        None,
        &attachment.expect("FrameMeta rides along"),
    );
    match &r.rendered {
        Rendered::Value { declared, value } => {
            assert_eq!(declared, "camera.v1.FrameMeta");
            assert_eq!(value["width"], 640);
            assert_eq!(value["height"], 480);
        }
        other => panic!("{other:?}"),
    }

    // Protobuf.
    let (key, enc, payload, _) = sample("stream/objects");
    assert_eq!(enc, "application/protobuf");
    let r = render(&catalog, &store, &key, Member::Type, Some(&enc), &payload);
    match &r.rendered {
        Rendered::Value { declared, value } => {
            assert_eq!(declared, "detections.v1.Objects");
            assert!(value.as_object().is_some_and(|o| o.len() == 1), "{value}");
        }
        other => panic!("{other:?}"),
    }
    // Bytes that are not their declared type: the type, and the reason.
    let r = render(
        &catalog,
        &store,
        &key,
        Member::Type,
        Some(&enc),
        &[0xff, 0xff, 0xff],
    );
    assert!(
        matches!(&r.rendered, Rendered::Undecodable { declared, reason }
            if declared == "detections.v1.Objects" && !reason.is_empty()),
        "{:?}",
        r.rendered
    );

    // JSON.
    let (key, enc, payload, _) = sample("bandwidth/default/eth0");
    assert_eq!(enc, "application/json");
    let r = render(&catalog, &store, &key, Member::Type, Some(&enc), &payload);
    assert_eq!(
        r.rendered,
        Rendered::Value {
            declared: "json:BandwidthUpdate".into(),
            value: serde_json::json!({"rx_bps": 1, "tx_bps": 2}),
        }
    );
    let res = r.resource.expect("resolved");
    assert_eq!(res.resource, "stream/bandwidth/{ns}/{iface}");
    assert_eq!(res.values["iface"], ["eth0"]);

    // A raw family: the concrete subtype from the sample's `Encoding`; the
    // CBOR attachment by the contract's `attachment_encoding`.
    let (key, enc, payload, attachment) = sample("video/h264/high");
    assert_eq!(enc, "video/h264");
    let r = render(&catalog, &store, &key, Member::Type, Some(&enc), &payload);
    assert_eq!(
        r.rendered,
        Rendered::Opaque {
            media_type: "video/h264".into()
        }
    );
    let r = render(
        &catalog,
        &store,
        &key,
        Member::Attachment,
        None,
        &attachment.expect("FrameMeta rides along"),
    );
    assert_eq!(
        r.rendered,
        Rendered::Value {
            declared: "json:FrameMeta".into(),
            value: serde_json::json!({"keyframe": true, "pts_ns": 40}),
        }
    );

    // With the revision in hand directly, the same answer.
    let held = store
        .held()
        .into_iter()
        .find(|r| r.iface() == &det)
        .expect("detections.v1 held");
    let (key, enc, payload, _) = sample("stream/objects");
    assert_eq!(
        render_with(&held, &key, Member::Type, Some(&enc), &payload).rendered,
        render(&catalog, &store, &key, Member::Type, Some(&enc), &payload).rendered
    );

    // A provider presence does not show: the structural ladder, and why.
    let r = render(
        &catalog,
        &store,
        "zk2/vehicle-02/detector/detections.v1/stream/objects",
        Member::Type,
        Some("application/json"),
        br#"{"x":1}"#,
    );
    assert!(
        matches!(
            &r.rendered,
            Rendered::Structural {
                why: zenkey_fleet::report::Unresolved::NoProvider,
                value: Some(_),
                ..
            }
        ),
        "{:?}",
        r.rendered
    );
    drop(subs);
}
