//! The design's walkthroughs on their own contracts (`examples/zk2/`), and
//! the tcgui pilot's frontend binding (#619's done-when).

mod common;

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use common::{T, client, config, eventually, example, router};
use zenkey::consumer::Delivery;
use zenkey::model::grammar::{IfaceId, ZkKey};
use zenkey::model::template::Bindings;
use zenkey::presence::{self, Edge};
use zenkey::{Implementation, Service, ServiceBuilder};

fn iface(s: &str) -> IfaceId {
    s.parse().unwrap()
}

/// A camera: `camera.v1`'s `@stream image` with its `FrameMeta`
/// attachment, and `state info`.
async fn camera(s: &zenoh::Session, address: &str) -> (Service, zenkey::writer::Writer) {
    let cam = iface("camera.v1");
    let mut b = ServiceBuilder::new(s, config(address));
    b.implement(Implementation::new(example("walkthrough/camera.v1")))
        .unwrap();
    let w = b
        .declare_writer(&cam, "@stream/image", &Bindings::new())
        .await
        .unwrap();
    b.expose(&cam, "state/info").unwrap();
    (b.start().await.unwrap(), w)
}

/// A detector: `detections.v1`, whose contract requires `camera.v1` as
/// `input`; the configuration binds it. Every image it receives becomes an
/// `objects` sample. The code is the same whatever the binding says.
async fn detector(
    s: &zenoh::Session,
    address: &str,
    input: &str,
) -> (
    Service,
    zenkey::consumer::Subscription,
    Arc<Mutex<Vec<String>>>,
) {
    let det = iface("detections.v1");
    let mut b = ServiceBuilder::new(s, config(address).bind("input", &[input]));
    b.implement(Implementation::new(example("walkthrough/detections.v1")))
        .unwrap();
    let out = Arc::new(
        b.declare_writer(&det, "stream/objects", &Bindings::new())
            .await
            .unwrap(),
    );
    let svc = b.start().await.unwrap();
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let (o, sn) = (Arc::clone(&out), Arc::clone(&seen));
    let handle = tokio::runtime::Handle::current();
    let sub = svc
        .consumer("input", Arc::new(example("walkthrough/camera.v1")))
        .unwrap()
        .subscribe("@stream/image", move |d: Delivery| {
            assert!(d.sample.attachment().is_some(), "FrameMeta rides along");
            sn.lock().unwrap().push(d.provider.to_string());
            let o = Arc::clone(&o);
            // An empty `detections.v1.Objects` is a valid message.
            handle.spawn(async move { o.put(Vec::<u8>::new()).await.unwrap() });
        })
        .await
        .unwrap();
    (svc, sub, seen)
}

/// Walkthrough §4.1: camera → detector → tracker over the walkthrough's
/// contracts; replay by rebinding the detector's input, with no code
/// change; and the graph, read from descriptors.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn w4_1_perception_pipeline() {
    let (_r1, ep) = router(None).await;
    let veh = client(&ep).await;
    let tool = client(&ep).await;

    let (_cam, frames) = camera(&veh, "vehicle-01/cam-front").await;
    let (_replay, replay_frames) = camera(&veh, "vehicle-01/replay-cam").await;
    let (_det, _dsub, det_seen) =
        detector(&veh, "vehicle-01/detector", "vehicle-01/cam-front").await;
    // Replay: the same detector code, its input rebound by configuration.
    let (_det2, _d2sub, det2_seen) =
        detector(&veh, "vehicle-01/detector-replay", "vehicle-01/replay-cam").await;

    let mut b = ServiceBuilder::new(
        &veh,
        config("vehicle-01/tracker").bind("sources", &["vehicle-01/*"]),
    );
    b.require("sources", iface("detections.v1"), false);
    let tracker = b.start().await.unwrap();
    let tracked: Arc<Mutex<BTreeSet<String>>> = Arc::default();
    let t = Arc::clone(&tracked);
    let _tsub = tracker
        .consumer("sources", Arc::new(example("walkthrough/detections.v1")))
        .unwrap()
        .subscribe("stream/objects", move |d: Delivery| {
            t.lock().unwrap().insert(d.provider.to_string());
        })
        .await
        .unwrap();

    // A camera frame: JPEG bytes, an (empty) FrameMeta attachment.
    eventually(
        "both detectors see their camera and the tracker sees both",
        || async {
            frames
                .put_with(vec![0xff, 0xd8], Some(Vec::<u8>::new()))
                .await
                .unwrap();
            replay_frames
                .put_with(vec![0xff, 0xd8], Some(Vec::<u8>::new()))
                .await
                .unwrap();
            tracked.lock().unwrap().len() == 2
        },
    )
    .await;
    assert!(
        det_seen
            .lock()
            .unwrap()
            .iter()
            .all(|p| p == "vehicle-01/cam-front")
    );
    assert!(
        det2_seen
            .lock()
            .unwrap()
            .iter()
            .all(|p| p == "vehicle-01/replay-cam")
    );

    // The graph: detector ← cam-front, tracker ← {detectors}, from
    // descriptors and tokens alone.
    let tokens = presence::tokens(&tool, "zk2/vehicle-01/*/@zk/**", T)
        .await
        .unwrap();
    let mut descriptors = Vec::new();
    for k in &tokens {
        if let ZkKey::Instance { addr, instance } = k {
            descriptors.push(
                presence::descriptor(&tool, addr, instance, T)
                    .await
                    .unwrap()
                    .into_descriptor()
                    .unwrap(),
            );
        }
    }
    let e = |c: &str, r: &str, p: &str| Edge {
        consumer: c.to_owned(),
        role: r.to_owned(),
        provider: p.to_owned(),
    };
    let drawn: BTreeSet<Edge> = presence::edges(&descriptors, &tokens).into_iter().collect();
    assert_eq!(
        drawn,
        BTreeSet::from([
            e("vehicle-01/detector", "input", "vehicle-01/cam-front"),
            e(
                "vehicle-01/detector-replay",
                "input",
                "vehicle-01/replay-cam"
            ),
            e("vehicle-01/tracker", "sources", "vehicle-01/detector"),
            e(
                "vehicle-01/tracker",
                "sources",
                "vehicle-01/detector-replay"
            ),
        ])
    );
}

/// The tcgui frontend (`examples/zk2/tcgui/frontend.bindings.toml`): a pure
/// consumer binding `tc.netif.v1` to `*/tc`, every system's tc service.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tcgui_frontend_binding() {
    let (_r1, ep) = router(None).await;
    let hosts = client(&ep).await;
    let ws = client(&ep).await;
    let netif = iface("tc.netif.v1");
    let bw = |ns: &str, i: &str| -> Bindings {
        [
            ("ns".to_owned(), vec![ns.to_owned()]),
            ("iface".to_owned(), vec![i.to_owned()]),
        ]
        .into()
    };
    let mut writers = Vec::new();
    let mut tcs = Vec::new();
    for host in ["host-a", "host-b"] {
        let c = example("tcgui/tc.netif.v1");
        let names: Vec<String> = c
            .resources
            .iter()
            .map(zenkey::implementation::resource_name)
            .collect();
        let mut b = ServiceBuilder::new(&hosts, config(&format!("{host}/tc")));
        b.implement(Implementation::new(c)).unwrap();
        writers.push(
            b.declare_writer(
                &netif,
                "stream/bandwidth/{ns}/{iface}",
                &bw("default", "eth0"),
            )
            .await
            .unwrap(),
        );
        for n in names {
            b.expose(&netif, &n).unwrap();
        }
        tcs.push(b.start().await.unwrap());
    }

    let mut b = ServiceBuilder::new(&ws, config("ws-01/tcgui-frontend").bind("netif", &["*/tc"]));
    b.require("netif", netif.clone(), false);
    let frontend = b.start().await.unwrap();
    let from: Arc<Mutex<BTreeSet<String>>> = Arc::default();
    let f = Arc::clone(&from);
    let _sub = frontend
        .consumer("netif", Arc::new(example("tcgui/tc.netif.v1")))
        .unwrap()
        .subscribe("stream/bandwidth/{ns}/{iface}", move |d: Delivery| {
            assert_eq!(d.values["iface"], ["eth0"]);
            f.lock().unwrap().insert(d.provider.to_string());
        })
        .await
        .unwrap();
    eventually("both hosts' tc services deliver", || async {
        for w in &writers {
            w.put_value(&serde_json::json!({"rx_bps": 1, "tx_bps": 2}))
                .await
                .ok();
        }
        from.lock().unwrap().len() == 2
    })
    .await;
    assert_eq!(
        *from.lock().unwrap(),
        BTreeSet::from(["host-a/tc".to_owned(), "host-b/tc".to_owned()])
    );
}
