//! Frames read by a tool (core §7.7, observer §7.6; #981's K2): no owner
//! writes a frame yet (K3), so a raw client puts hand-written frames in
//! their envelopes on `linkframes.v1`'s keys, and the fleet's watch, as
//! `zenctl watch` reads it, shows each as its records: the runtime's reader
//! of the key, so a frame lost turns the next one's omitted fields
//! unknown, and a scheduled record reads scheduled with its jitter.

mod util;

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use util::zk2::{client, router};
use zenkey_fleet::ResolvedTarget;
use zenkey_fleet::model::catalog::Revision;
use zenkey_fleet::report::{Conformance, ContractSource, Rendered, WatchEvent};
use zenkey_model::contract::{Contract, load_path};
use zenkey_model::frame::{Frame, Layout};
use zenkey_model::template::Bindings;
use zenkey_model::wire::{Run, Seq, Stamp};

fn contract() -> Contract {
    let p =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../zenkey/tests/contracts/linkframes.v1.toml");
    let l = load_path(&p);
    l.contract
        .unwrap_or_else(|| panic!("linkframes.v1: {}", l.report))
}

fn layout(c: &Contract, template: &str) -> Layout {
    let r = c
        .resources
        .iter()
        .find(|r| r.template.as_str() == template)
        .unwrap();
    Layout::of_resource(c, r).unwrap().unwrap()
}

fn put(l: &Layout, seq: u32, v: &Value) -> Vec<u8> {
    Frame::from_json(v, l, None)
        .unwrap()
        .envelope(
            l,
            Stamp {
                run: Run::U16(5),
                seq: Seq::new(seq).unwrap(),
            },
        )
        .unwrap()
}

/// The watch's next sample's reading, valid, declared a frame.
async fn next_reading(watch: &mut zenkey_fleet::Watch) -> Value {
    let s = tokio::time::timeout(util::SETTLE, watch.next())
        .await
        .expect("a sample within the settle")
        .expect("the watch alive");
    let WatchEvent::Put {
        payload,
        conformance,
        ..
    } = &s.event
    else {
        panic!("{s:?}")
    };
    assert!(matches!(conformance, Conformance::Valid), "{conformance:?}");
    match &payload.rendered {
        Rendered::Value {
            declared, value, ..
        } => {
            assert!(declared.starts_with("zk2c frame of packed:"), "{declared}");
            value.clone()
        }
        other => panic!("not a value: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_watch_shows_each_frame_as_its_keys_reader_read_it() {
    let (_r, ep) = router(None).await;
    let (owner, tool) = (client(&ep).await, client(&ep).await);
    let c = contract();
    let revision = Revision::from_contract(c.clone(), ContractSource::File);
    let target = ResolvedTarget::parse("lab/modem").unwrap();
    let stream = |name: &str| {
        revision
            .resource(name, &[zenkey_model::authoring::Kind::Stream])
            .unwrap()
    };
    let mut link = zenkey_fleet::watch_resource(
        &tool,
        &revision,
        &target,
        stream("stream/frames/link"),
        &Bindings::new(),
    )
    .await
    .unwrap();
    let mut sample = zenkey_fleet::watch_resource(
        &tool,
        &revision,
        &target,
        stream("stream/frames/sample"),
        &Bindings::new(),
    )
    .await
    .unwrap();
    let p = owner
        .declare_publisher("zk2/lab/modem/linkframes.v1/stream/frames/link")
        .await
        .unwrap();
    let q = owner
        .declare_publisher("zk2/lab/modem/linkframes.v1/stream/frames/sample")
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    while !(p.matching_status().await.unwrap().matching()
        && q.matching_status().await.unwrap().matching())
    {
        assert!(tokio::time::Instant::now() < deadline, "never: matched");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let l = layout(&c, "frames/link");
    let full = json!({"time": {"s": 100, "ms": 0}, "records": [{"full": true,
        "values": {"dropped": 7, "queue_depth": 12, "duty_available": 1}}]});
    p.put(put(&l, 1, &full)).await.unwrap();
    let v = next_reading(&mut link).await;
    assert_eq!(v["follows"], "first");
    assert_eq!(
        v["records"][0]["fields"]["duty_wait_ms"],
        json!({"state": "absent"})
    );
    // 2 is lost: 3's omitted fields unknown, never their last value.
    let d = json!({"time": {"s": 220, "ms": 0}, "records": [{"full": false,
        "values": {"dropped": 9}}]});
    p.put(put(&l, 3, &d)).await.unwrap();
    let v = next_reading(&mut link).await;
    assert_eq!(
        (v["follows"].clone(), v["lost"].clone()),
        (json!("gap"), json!(1))
    );
    let f = &v["records"][0]["fields"];
    assert_eq!(f["dropped"], json!({"state": "known", "value": 9}));
    assert_eq!(f["queue_depth"], json!({"state": "unknown", "why": "gap"}));
    assert_eq!(
        f["duty_available"],
        json!({"state": "unknown", "why": "gap"})
    );

    let l = layout(&c, "frames/sample");
    let v3 = json!({"time": {"s": 7200, "ms": 0}, "records": [
        {"full": true, "values": {"temperature": "21.5", "rssi": -85, "link_up": true}},
        {"full": false, "at": "scheduled", "values": {"temperature": "22"}},
        {"full": false, "at": 20750, "values": {"rssi": -90}}]});
    q.put(put(&l, 30, &v3)).await.unwrap();
    let v = next_reading(&mut sample).await;
    let r = &v["records"];
    assert_eq!(
        (
            r[1]["seq"].clone(),
            r[1]["time"].clone(),
            r[1]["jitter_ms"].clone()
        ),
        (json!(31), json!("scheduled"), json!(500))
    );
    assert_eq!(r[1]["at"], json!({"s": 7210, "ms": 0}));
    assert_eq!(
        (r[2]["time"].clone(), r[2]["after_ms"].clone()),
        (json!("measured"), json!(20750))
    );
    let line = &zenkey_fleet::model::frame::reading_lines(&v).unwrap()[2];
    assert!(
        line.starts_with("#31 t=7210.000 scheduled ±500 ms: ")
            && line.contains("temperature \"22\"")
            && line.contains("mode absent"),
        "{line}"
    );
}
