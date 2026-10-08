//! `spec/scenarios/types.md` §1–§3 (spec §2.4, §7.1–§7.2).

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use common::{T, client, config, eventually, imp, router};
use serde_json::json;
use zenkey::ServiceBuilder;
use zenkey::model::decode::{Rendered, decode, type_of};
use zenkey::model::grammar::{IfaceId, ZkKey, parse};
use zenkey::model::template::Bindings;
use zenkey::presence;
use zenkey::retrieval::{Retrieved, fetch_bundle};
use zenkey::writer::Writer;
use zenoh::qos::{CongestionControl, Priority};
use zenoh::sample::Sample;

fn sensor() -> IfaceId {
    "sensor.v1".parse().unwrap()
}

/// `sensor.v1.Pose { x: 1.5, y: -2, frame: "map" }`, hand-encoded.
fn pose() -> Vec<u8> {
    let mut b = vec![0x09];
    b.extend_from_slice(&1.5f64.to_le_bytes());
    b.push(0x11);
    b.extend_from_slice(&(-2.0f64).to_le_bytes());
    b.extend_from_slice(&[0x1a, 3, b'm', b'a', b'p']);
    b
}

/// The owner: one writer per resource of `sensor.v1`.
async fn owner(s: &zenoh::Session) -> (zenkey::Service, BTreeMap<&'static str, Writer>) {
    let mut b = ServiceBuilder::new(s, config("lab/sensor"));
    b.implement(imp("sensor.v1")).unwrap();
    let mut w = BTreeMap::new();
    for res in [
        "stream/pose",
        "state/status",
        "stream/frames",
        "stream/blobs",
        "stream/fast",
        "state/slow",
    ] {
        w.insert(
            res,
            b.declare_writer(&sensor(), res, &Bindings::new())
                .await
                .unwrap(),
        );
    }
    (b.start().await.unwrap(), w)
}

/// Subscribes to every sensor key and keeps the last sample per resource.
async fn watch(
    s: &zenoh::Session,
) -> (
    zenoh::pubsub::Subscriber<()>,
    Arc<Mutex<BTreeMap<String, Sample>>>,
) {
    let got: Arc<Mutex<BTreeMap<String, Sample>>> = Arc::default();
    let g = Arc::clone(&got);
    let sub = s
        .declare_subscriber("zk2/lab/sensor/sensor.v1/**")
        .callback(move |smp: Sample| {
            if let Ok(ZkKey::Data { kind, resource, .. }) = parse(smp.key_expr().as_str()) {
                g.lock()
                    .unwrap()
                    .insert(format!("{kind}/{}", resource.join("/")), smp);
            }
        })
        .await
        .unwrap();
    (sub, got)
}

async fn publish_all(w: &BTreeMap<&'static str, Writer>) {
    eventually("matched", || async {
        w["stream/pose"].matching().await.unwrap()
    })
    .await;
    w["stream/pose"].put(pose()).await.unwrap();
    w["state/status"]
        .put_value(&json!({"up": true, "load": 0.5}))
        .await
        .unwrap();
    w["stream/frames"]
        .put(vec![0xff, 0xd8, 0xff, 0xe0])
        .await
        .unwrap();
    w["stream/blobs"].put(vec![0u8; 42]).await.unwrap();
    w["stream/fast"].put("f").await.unwrap();
    w["state/slow"].put("s").await.unwrap();
}

/// §1: every sample carries its predefined `Encoding`, without a schema
/// suffix.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_encoding_on_every_sample() {
    let (_r1, ep) = router(None).await;
    let (_svc, w) = owner(&client(&ep).await).await;
    let tool = client(&ep).await;
    let (_sub, got) = watch(&tool).await;
    eventually("all six arrived", || async {
        if got.lock().unwrap().len() < 6 {
            publish_all(&w).await;
        }
        got.lock().unwrap().len() == 6
    })
    .await;
    let g = got.lock().unwrap();
    let enc = |r: &str| g[r].encoding().to_string();
    assert_eq!(enc("stream/pose"), "application/protobuf");
    assert_eq!(enc("state/status"), "application/cbor");
    assert_eq!(enc("stream/frames"), "image/jpeg");
}

/// §2: a tool with only the bus and the bundles it fetches decodes the
/// protobuf and JSON Schema values field by field, and renders the raw one
/// as its declared type and size.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_decoding_and_honest_rendering() {
    let (_r1, ep) = router(None).await;
    let (_svc, w) = owner(&client(&ep).await).await;
    let tool = client(&ep).await;

    // The bundle, from the bus alone.
    let deadline = tokio::time::Instant::now() + common::SETTLE;
    let bundle = 'found: loop {
        for t in presence::tokens(&tool, "zk2/lab/*/@zk/**", T)
            .await
            .unwrap()
        {
            if let ZkKey::Instance { addr, instance } = t {
                let d = presence::descriptor(&tool, &addr, &instance, T)
                    .await
                    .unwrap()
                    .into_descriptor()
                    .unwrap();
                let e = d
                    .interfaces
                    .iter()
                    .find(|e| e.iface == "sensor.v1")
                    .unwrap();
                let fp = zenkey::model::canonical::Fingerprint::parse(&e.contract).unwrap();
                if let Retrieved::Bundle(b, _) =
                    fetch_bundle(&tool, &sensor(), &fp, T).await.unwrap()
                {
                    break 'found *b;
                }
            }
        }
        assert!(tokio::time::Instant::now() < deadline, "no bundle");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };

    let (_sub, got) = watch(&tool).await;
    eventually("arrived", || async {
        publish_all(&w).await;
        got.lock().unwrap().len() == 6
    })
    .await;
    let g = got.lock().unwrap();
    let show = |r: &str| {
        let (token, template) = r.split_once('/').unwrap();
        let ty = type_of(&bundle, token, template, "type").unwrap();
        let s = &g[r];
        decode(
            &bundle,
            ty,
            Some(&s.encoding().to_string()),
            &s.payload().to_bytes(),
        )
    };
    assert_eq!(
        show("stream/pose"),
        Rendered::Value(json!({"x": 1.5, "y": -2.0, "frame": "map"}))
    );
    assert_eq!(
        show("state/status"),
        Rendered::Value(json!({"up": true, "load": 0.5}))
    );
    assert_eq!(
        show("stream/blobs"),
        Rendered::Opaque {
            media_type: "application/x-flatbuffers".to_owned(),
            size: 42
        }
    );
}

/// §3: each received sample's QoS is the declared one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_qos_applied() {
    let (_r1, ep) = router(None).await;
    let (_svc, w) = owner(&client(&ep).await).await;
    let tool = client(&ep).await;
    let (_sub, got) = watch(&tool).await;
    eventually("arrived", || async {
        publish_all(&w).await;
        let g = got.lock().unwrap();
        g.contains_key("stream/fast") && g.contains_key("state/slow")
    })
    .await;
    let g = got.lock().unwrap();
    let fast = &g["stream/fast"];
    assert_eq!(fast.priority(), Priority::DataHigh);
    assert!(fast.express());
    assert_eq!(
        fast.congestion_control(),
        CongestionControl::Drop,
        "a stream's default"
    );
    let slow = &g["state/slow"];
    assert_eq!(slow.priority(), Priority::Data);
    assert!(!slow.express());
    assert_eq!(
        slow.congestion_control(),
        CongestionControl::Block,
        "a state's default"
    );
}
