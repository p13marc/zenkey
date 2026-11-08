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
use util::zk2::{T, addr, client, config, eventually, example, iface, router};

/// An owner answering in core §7.5's forms (#980's C4b), the runtime's
/// suites' and zenctl's too.
#[path = "../../zenkey/tests/common/forms_owner.rs"]
mod forms_owner;

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
            &["walkthrough/camera.v1", "profiles/health/health.v1"],
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
    // Both use freshness.v2 since #978's F5: the runtime implements its
    // standard contract beside them, its heartbeats' interface.
    let fv2 = zenkey::heartbeat::implementation();
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
                "freshness.v2",
                Some(fp16(&fv2).as_str()),
                Some(fv2.fingerprint().to_string().as_str()),
                false
            ),
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

    // Presence, descriptors, bundles: the tool's whole knowledge. Four
    // interfaces and `freshness.v2`, which tc.netif.v1 and zs.parallax.v1
    // use since #978's F5, its standard contract implemented beside them.
    eventually("every owner described", || async {
        observe(&tool, &Scope::all(), T)
            .await
            .is_ok_and(|o| Catalog::new(&o).wanted().len() == 5)
    })
    .await;
    let catalog = Catalog::new(&observe(&tool, &Scope::all(), T).await.expect("observe"));
    assert_eq!(catalog.wanted().len(), 5);
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
        Rendered::Value {
            declared, value, ..
        } => {
            assert_eq!(declared, "camera.v1.FrameMeta");
            assert_eq!(value["width"], 640);
            assert_eq!(value["height"], 480);
        }
        other => panic!("{other:?}"),
    }

    // Protobuf, under its zk2c envelope (core §4.3, §7.5): the sample's
    // Encoding is `zenoh/bytes` (§7.2's exception), and its record is read
    // on the contract's (observer §7.3).
    let (key, enc, payload, _) = sample("stream/objects");
    assert_eq!(enc, "zenoh/bytes");
    assert!(
        zenkey_fleet::model::inband::payload(&key, Some(&enc), &payload)
            .stamp()
            .is_some(),
        "stamped in band"
    );
    let r = render(&catalog, &store, &key, Member::Type, Some(&enc), &payload);
    match &r.rendered {
        Rendered::Value {
            declared, value, ..
        } => {
            assert_eq!(declared, "detections.v1.Objects");
            assert!(value.as_object().is_some_and(|o| o.len() == 1), "{value}");
        }
        other => panic!("{other:?}"),
    }
    // A record that is not its declared type: the type, and the reason.
    let garbage = zenkey_model::wire::Envelope {
        form: zenkey_model::wire::Form::Mutation,
        tag: zenkey_model::wire::Tag::NONE,
        stamp: zenkey_model::wire::Stamp {
            run: zenkey_model::wire::Run::U16(0),
            seq: zenkey_model::wire::Seq::new(1).expect("a seq"),
        },
        record: &[0xff, 0xff, 0xff],
    }
    .write();
    let r = render(&catalog, &store, &key, Member::Type, Some(&enc), &garbage);
    assert!(
        matches!(&r.rendered, Rendered::Undecodable { declared, reason }
            if declared == "detections.v1.Objects" && !reason.is_empty()),
        "{:?}",
        r.rendered
    );

    // JSON, under its envelope too.
    let (key, enc, payload, _) = sample("bandwidth/default/eth0");
    assert_eq!(enc, "zenoh/bytes");
    let r = render(&catalog, &store, &key, Member::Type, Some(&enc), &payload);
    assert_eq!(
        r.rendered,
        Rendered::Value {
            declared: "json:BandwidthUpdate".into(),
            value: serde_json::json!({"rx_bps": 1, "tx_bps": 2}),
            epoch_assumed: None,
            dated: None,
            bounds: None,
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
            epoch_assumed: None,
            dated: None,
            bounds: None,
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

/// #765: a state read keeps the reply's bytes beside its rendering, the
/// owner's (S4) and the archive's (S5) alike, and a raw GET within the
/// owner's tombstone window reads its `reply_del` as a delete (core §4,
/// S3), never as a put with an empty payload.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_state_read_keeps_its_bytes_and_a_raw_get_reads_a_tombstone() {
    use zenkey::archive::{Archive, ArchiveConfig, Recorded};
    use zenkey_fleet::model::catalog::Revision;
    use zenkey_fleet::report::{StateReport, StateValue};
    use zenkey_fleet::{Answer, FleetAnswer, GetOpts, StateRead};
    use zenoh::sample::SampleKind;

    let (_router, ep) = router(None).await;
    let (owners, tool, archiving) = (client(&ep).await, client(&ep).await, client(&ep).await);
    let contract = example("tcgui/tc.netif.v1");
    let netif = iface("tc.netif.v1");
    let origin = "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0";
    let archive = Archive::start(
        &archiving,
        ArchiveConfig {
            service: config("ground/archive"),
            records: vec![Recorded {
                owner: addr("host-a/tc"),
                selector: "zk2/host-a/tc/tc.netif.v1/state/interfaces/*/*".to_owned(),
                implementation: Implementation::new(contract.clone()),
            }],
            peers: Vec::new(),
            unconfirmed_horizon: None,
            watch: None,
        },
    )
    .await
    .expect("an archive");
    let mut b = ServiceBuilder::new(&owners, config("host-a/tc"));
    b.implement(Implementation::new(contract.clone()))
        .expect("implement");
    for r in &contract.resources {
        let _ = b.expose(&netif, &zenkey::implementation::resource_name(r));
    }
    b.serve_state(&netif).expect("serve state");
    let mut owner = b.start().await.expect("the owner");
    let member: Bindings = [
        ("ns".to_owned(), vec!["default".to_owned()]),
        ("iface".to_owned(), vec!["eth0".to_owned()]),
    ]
    .into();
    let writer = owner
        .state_writer(&netif, "state/interfaces/{ns}/{iface}", &member)
        .await
        .expect("a state writer");
    let body = br#"{"name":"eth0","index":2,"namespace":"default","is_up":true}"#;
    writer.put(body.to_vec()).await.expect("a put");

    let revision = Revision::from_contract(contract, ContractSource::File);
    let resource = revision
        .resource(
            "state/interfaces/{ns}/{iface}",
            &[zenkey_model::authoring::Kind::State],
        )
        .expect("the resource");
    let owner_addr = addr("host-a/tc");
    let read = || StateRead {
        revision: &revision,
        owner: &owner_addr,
        resource,
        values: &member,
        timeout: T,
    };
    let bytes_of = |report: &StateReport| {
        report
            .rows
            .first()
            .and_then(|row| row.bytes.as_ref())
            .map(|b| b.to_bytes().into_owned())
    };
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    let settle = |what: &str| {
        assert!(tokio::time::Instant::now() < deadline, "never: {what}");
        tokio::time::sleep(std::time::Duration::from_millis(50))
    };

    // S4: the owner's reply, its bytes kept beside the rendering.
    let current = loop {
        let report = zenkey_fleet::get_state(&tool, read())
            .await
            .expect("a state read");
        if !report.rows.is_empty() {
            break report;
        }
        settle("the owner answers S4's GET").await;
    };
    assert!(
        matches!(&current.rows[0].value, StateValue::Value { .. }),
        "{current:?}"
    );
    assert_eq!(
        bytes_of(&current).as_deref(),
        Some(&body[..]),
        "the owner's reply, as it arrived"
    );

    // S5: the archive's record, its bytes kept the same way.
    eventually("the archive records it, confirmed", || async {
        archive.confirmed(origin) == Some(true)
    })
    .await;
    let last = zenkey_fleet::last_known_state(&tool, read(), &addr("ground/archive"))
        .await
        .expect("a last-known read");
    assert_eq!(bytes_of(&last).as_deref(), Some(&body[..]), "{last:?}");

    // The tombstone: a raw GET within the window, a member read, is answered
    // by the member's snapshot (core 0.100, #977's E3), whose tombstone reads
    // as a delete, its kind kept, never a value.
    writer.delete().await.expect("a delete");
    let answers = loop {
        let answers: Vec<FleetAnswer> = zenkey_fleet::fleet_get(&tool, origin, &GetOpts::new(T))
            .await
            .expect("a GET")
            .iter()
            .flat_map(|a| match a.entries() {
                Some(Ok(entries)) => entries,
                _ => vec![a.clone()],
            })
            .collect();
        if answers.iter().any(FleetAnswer::is_delete) {
            break answers;
        }
        settle("the owner answers with its tombstone").await;
    };
    assert_eq!(answers.len(), 1, "one owner, one entry: {answers:?}");
    let tombstone = &answers[0];
    assert_eq!(tombstone.key, origin);
    assert_eq!(tombstone.kind, Some(SampleKind::Delete));
    // A snapshot carries no Zenoh timestamp, and a tombstone no
    // confirmation (core §7.5); its in-band stamp is its delete's.
    assert_eq!(tombstone.timestamp, None, "a snapshot's tombstone");
    assert!(tombstone.inband().is_some(), "the delete's in-band stamp");
    let _ = &owners;
    let Answer::Value(payload) = &tombstone.answer else {
        panic!("a delete is a sample, not an error reply: {tombstone:?}")
    };
    assert!(payload.is_empty(), "a delete carries nothing");

    // S4 reads the same tombstone as a deletion, with no bytes beside it.
    let deleted = zenkey_fleet::get_state(&tool, read())
        .await
        .expect("a state read");
    assert!(
        matches!(&deleted.rows[..], [row] if row.value == StateValue::Deleted && row.bytes.is_none()),
        "{deleted:?}"
    );
    drop(writer);
}

/// #985's C4b to C4d: `call_operation` reads an owner answering in §7.5's
/// forms by each reply's `Encoding` alone (core §5.1, §5.2; observer
/// §7.3, "A call's forms first"): a value form's record rendered as its
/// type with its as-of mark, an error form's code and message with its
/// detail rendered as the `error` type and its mark, a value that is no
/// form, under `zenoh/bytes` or another `Encoding`, undecodable with why,
/// and an earlier core's envelope and a router's `Timeout` silence with
/// their `Encoding` and bytes, never `busy`; and each many-reply form's
/// mark.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_is_read_from_its_form_alone() {
    use zenkey_fleet::report::{InbandStamp, OperationAnswer, OperationReport};
    use zenkey_fleet::{OperationCall, ResolvedTarget, call_operation, plan_call};

    let (_router, ep) = router(None).await;
    let owner = client(&ep).await;
    let tool = client(&ep).await;
    let _served = forms_owner::serve(&owner, "f1/tc").await;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../zenkey/tests/contracts/tc.v1.toml");
    let loaded = zenkey_model::contract::load_path(&path);
    let rev = zenkey_fleet::model::catalog::Revision::from_contract(
        loaded.contract.expect("tc.v1 loads"),
        ContractSource::File,
    );
    let call = |op: &'static str, member: Option<&'static str>, request: &'static [u8]| {
        let (rev, tool) = (&rev, &tool);
        async move {
            let values: Bindings = member
                .map(|m| [("if".to_owned(), vec![m.to_owned()])].into())
                .unwrap_or_default();
            let target = ResolvedTarget::parse("f1/tc").unwrap();
            let plan = plan_call(rev, target, op, values).unwrap();
            call_operation(
                tool,
                OperationCall {
                    revision: rev,
                    plan: &plan,
                    request: request.to_vec(),
                    epoch: zenkey_fleet::DeploymentEpoch::Assumed,
                    timeout: std::time::Duration::from_millis(500),
                    retries: 1,
                },
            )
            .await
            .unwrap()
        }
    };
    let set = |m| call("@op/interfaces/{if}/set", Some(m), br#"{"up":true}"#);
    let mark = InbandStamp {
        run: "0001".into(),
        seq: 7,
    };
    let json = |r: &OperationReport| serde_json::to_value(r).unwrap();

    // The value form: the record rendered as its type, the mark beside it.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
    let value = loop {
        let r = set("eth0").await;
        if matches!(r.answer, OperationAnswer::Value { .. }) {
            break r;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "never answered: {r:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    let OperationAnswer::Value { reply, mark: got } = &value.answer else {
        unreachable!()
    };
    assert_eq!(got.as_ref(), Some(&mark));
    assert!(
        matches!(&reply.rendered, Rendered::Value { declared, value, .. }
            if declared == "json:SetResponse" && value == &serde_json::json!({"up": true})),
        "{reply:?}"
    );
    assert_eq!(
        json(&value)["mark"],
        serde_json::json!({"run": "0001", "seq": 7})
    );

    // The error form: its code and message, its detail as the `error` type.
    let refused = json(&set("eth9").await);
    assert_eq!(refused["answer"], "refused");
    assert_eq!(
        refused["envelope"],
        serde_json::json!({
            "code": "app",
            "message": "the kernel refused",
            "detail": {"as": "value", "declared": "json:TcError", "value": {"kind": "kernel"}},
            "mark": {"run": "0001", "seq": 7},
        })
    );

    // An earlier core's envelope: the transport's, silence, never `busy`.
    // `set` is not idempotent: one attempt.
    let envelope = json(&set("eth1").await);
    assert_eq!(envelope["answer"], "silent", "{envelope}");
    assert_eq!(envelope["silence"]["attempts"], 1);
    assert_eq!(
        envelope["silence"]["transport"],
        r#"application/json: {"code":"busy","message":""}"#
    );

    // A value under `zenoh/bytes` that is no form: undecodable, never sniffed.
    let r = set("eth2").await;
    let OperationAnswer::Value { reply, mark: None } = &r.answer else {
        panic!("{r:?}")
    };
    assert!(
        matches!(&reply.rendered, Rendered::Undecodable { reason, .. }
            if reason == "zk2c envelope refused: sample_kind"),
        "{reply:?}"
    );

    // A value under another `Encoding`: no form, undecodable, never read as
    // the JSON it holds.
    let r = set("eth4").await;
    let OperationAnswer::Value { reply, mark: None } = &r.answer else {
        panic!("{r:?}")
    };
    assert!(
        matches!(&reply.rendered, Rendered::Undecodable { declared, reason }
            if declared == "json:SetResponse" && reason.starts_with("no call form under application/json")),
        "{reply:?}"
    );

    // A router's `Timeout`: silence, with its `Encoding` and its bytes.
    let silent = json(&call("@op/diagnostics", None, b"{}").await);
    assert_eq!(silent["answer"], "silent");
    assert_eq!(silent["silence"]["attempts"], 2);
    assert_eq!(
        silent["silence"]["transport"],
        "zenoh/bytes: Timeout (no error form: sample_kind)"
    );

    // Many replies: each value's and the summary's mark, in their order.
    let listing = json(&call("@op/listing", None, b"{}").await);
    let p = &listing["replies"]["repliers"][0];
    assert_eq!(p["replies"][1]["value"], serde_json::json!({"name": "b"}));
    assert_eq!(p["summaries"][0]["value"]["scanned"], 2);
    assert_eq!(p["marks"], serde_json::json!([mark, mark, mark]));
    assert_eq!(p["possibly_partial"], false);
}

/// Review-bot on #1086: a state GET's reader merges its replies per key by
/// their in-band stamps (core S4), never keeping the last to arrive. Two
/// repliers of one alarm key under one run, the newer (seq 4) first and
/// the older (seq 2) 300 ms after it: the alarms reader keeps seq 4, one
/// reply for the key.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_state_gets_replies_merge_per_key_whatever_arrives_last() {
    use zenkey_model::wire::{Envelope, Form, Run, Seq, Stamp, Tag};
    let (_router, ep) = router(None).await;
    let (first, second, tool) = (client(&ep).await, client(&ep).await, client(&ep).await);
    let key = "zk2/lab/al/alarms.v1/state/alarms/a659f813308ad1da";
    let stamp = |seq| Stamp {
        run: Run::U16(5),
        seq: Seq::new(seq).expect("a seq"),
    };
    let put = |seq| {
        Envelope {
            form: Form::Mutation,
            tag: Tag::NONE,
            stamp: stamp(seq),
            record: b"{}",
        }
        .write()
    };
    let mut held = Vec::new();
    for (s, seq, wait) in [(&first, 4, 0), (&second, 2, 300)] {
        let bytes = put(seq);
        held.push(
            s.declare_queryable("zk2/lab/al/alarms.v1/state/**")
                .callback(move |q| {
                    let bytes = bytes.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
                        q.reply(key, bytes)
                            .encoding(zenoh::bytes::Encoding::ZENOH_BYTES)
                            .await
                            .expect("a reply");
                    });
                })
                .await
                .expect("a queryable"),
        );
    }
    let selector = "zk2/lab/al/alarms.v1/state/alarms/*";
    eventually("both repliers are reached", || async {
        let rx = tool
            .get(selector)
            .consolidation(zenoh::query::ConsolidationMode::None)
            .timeout(std::time::Duration::from_secs(2))
            .await
            .expect("a GET");
        let mut n = 0;
        while let Ok(r) = rx.recv_async().await {
            n += usize::from(r.result().is_ok());
        }
        n == 2
    })
    .await;
    let r = zenkey_fleet::get_alarms(
        &tool,
        selector,
        std::time::Duration::from_secs(2),
        &zenkey_fleet::EntryEpochs::default(),
    )
    .await
    .expect("a GET");
    assert_eq!(r.replies.len(), 1, "one reply a key: {:?}", r.replies);
    assert_eq!(
        r.envelopes.get(key),
        Some(&Ok(stamp(4))),
        "the newer, not the last"
    );
}

/// Core §4.2 at 0.101 (zk2py rounds 105 and 106): whether an alarm is per
/// key is read from its owner's revision under the `reply_del`'s run, never
/// from the reader's alone. An owner on `alarms.v1` 1.0, which uses
/// `freshness.v1`, answers its alarms per key, and its descriptor names
/// 1.0: the alarms reader, which holds 1.1, retrieves 1.0 and merges the
/// `reply_del` of a resolved alarm as a deletion with its stamp. By 1.1
/// alone it would be a stray.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_owner_on_revision_1_0_answers_its_alarm_per_key() {
    let (_router, ep) = router(None).await;
    let (owners, tool) = (client(&ep).await, client(&ep).await);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../spec/profiles/.history/alarms.v1/\
         95579151aa231d2ba1224eae828083bdfc7f4f9403a80c831c1915decca9e8b6.bundle.json",
    );
    let earlier = Implementation::from_bundle(&std::fs::read(path).expect("the bundle"))
        .expect("alarms.v1 1.0");
    let mut b = ServiceBuilder::new(&owners, config("lab/al"));
    let a = b
        .alarms_with(earlier.clone(), zenkey::alarms::Startup::default())
        .await
        .expect("an alarms owner on 1.0");
    let svc = b.start().await.expect("started");
    let raised = a
        .raise(zenkey::alarms::Condition::new(
            "link_down",
            zenkey_model::alarms::Severity::Warning,
            "link down",
        ))
        .await
        .expect("raised");
    let key = format!("zk2/lab/al/alarms.v1/state/alarms/{}", raised.alert_key);
    assert!(
        !zenkey_model::collection::stray_reply_del(earlier.contract(), &key),
        "by 1.0, per key"
    );
    assert!(
        zenkey_model::collection::stray_reply_del(
            zenkey::alarms::implementation().contract(),
            &key
        ),
        "by 1.1, answered by snapshot"
    );
    a.resolve(&raised.alert_key)
        .await
        .expect("resolved")
        .expect("held");
    let r = eventually_some_reply(&tool, &key).await;
    let reply = r.replies.iter().find(|s| s.key == key).expect("its reply");
    assert!(reply.delete, "a deletion, no stray: {:?}", r.replies);
    assert!(
        matches!(r.envelopes.get(&key), Some(Ok(s)) if s.run == svc.minter().run()),
        "under its stamp: {:?}",
        r.envelopes.get(&key)
    );
    svc.close().await.expect("closed");
}

/// The alarms reader's GET of `lab/al`, once it answers `key`.
async fn eventually_some_reply(tool: &zenoh::Session, key: &str) -> zenkey_fleet::AlarmSeedRead {
    for _ in 0..50 {
        let r = zenkey_fleet::get_alarms(
            tool,
            "zk2/lab/al/alarms.v1/state/alarms/*",
            std::time::Duration::from_secs(2),
            &zenkey_fleet::EntryEpochs::default(),
        )
        .await
        .expect("a GET");
        if r.replies.iter().any(|s| s.key == key && s.delete) {
            return r;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("the owner never answered {key} with its deletion")
}

/// #977's E4: an alarms reader reads a snapshot's tombstone as a deletion
/// with its stamp, and a `reply_del` on an alarm, which its owner answers
/// by snapshot, as no owner's answer: a reply undecodable, `form`, never a
/// deletion.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tombstone_is_a_deletion_and_a_stray_reply_del_is_undecodable() {
    use zenkey_model::collection::{Snapshot, Time, Tombstone};
    use zenkey_model::wire::{DeleteStamp, Run, Seq, Stamp, Tag};
    let (_router, ep) = router(None).await;
    let (owner, tool) = (client(&ep).await, client(&ep).await);
    let tomb = "zk2/lab/al/alarms.v1/state/alarms/0000000000000001";
    let stray = "zk2/lab/al/alarms.v1/state/alarms/0000000000000002";
    let seq = |n| Seq::new(n).expect("a seq");
    let page = Snapshot {
        tag: Tag::NONE,
        run: Run::U16(5),
        asof: seq(3),
        time: Time::Unknown,
        coll: 0,
        live: Vec::new(),
        tombs: vec![Tombstone {
            id: String::new(),
            run: None,
            seq: seq(3),
        }],
    }
    .paginate(1024)
    .expect("a page")[0]
        .write()
        .expect("written");
    let attachment = DeleteStamp {
        stamp: Stamp {
            run: Run::U16(5),
            seq: seq(4),
        },
    }
    .write();
    let _q = owner
        .declare_queryable("zk2/lab/al/alarms.v1/state/**")
        .callback(move |q| {
            let (page, attachment) = (page.clone(), attachment.clone());
            tokio::spawn(async move {
                q.reply(tomb, page)
                    .encoding(zenoh::bytes::Encoding::ZENOH_BYTES)
                    .await
                    .expect("a page");
                q.reply_del(stray)
                    .attachment(attachment)
                    .await
                    .expect("a reply_del");
            });
        })
        .await
        .expect("a queryable");
    eventually("the owner answers", || async {
        let rx = tool
            .get("zk2/lab/al/alarms.v1/state/alarms/*")
            .consolidation(zenoh::query::ConsolidationMode::None)
            .timeout(std::time::Duration::from_secs(2))
            .await
            .expect("a GET");
        let mut n = 0;
        while let Ok(r) = rx.recv_async().await {
            n += usize::from(r.result().is_ok());
        }
        n == 2
    })
    .await;
    let r = zenkey_fleet::get_alarms(
        &tool,
        "zk2/lab/al/alarms.v1/state/alarms/*",
        std::time::Duration::from_secs(2),
        &zenkey_fleet::EntryEpochs::default(),
    )
    .await
    .expect("a GET");
    let by = |k: &str| r.replies.iter().find(|s| s.key == k).expect("a reply");
    assert!(by(tomb).delete, "a tombstone is a deletion");
    assert_eq!(
        r.envelopes.get(tomb),
        Some(&Ok(Stamp {
            run: Run::U16(5),
            seq: seq(3)
        }))
    );
    assert!(!by(stray).delete, "a stray reply_del is no deletion");
    assert_eq!(
        r.envelopes.get(stray).map(|e| e.map_err(|r| r.token())),
        Some(Err("form"))
    );
}
