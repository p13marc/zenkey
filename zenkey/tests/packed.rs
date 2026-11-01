//! Packed records through the runtime (core §7.5, §7.6; #979 B2): the
//! writer seals each record with its layout's tag in CH and admits it by its
//! layout, and the consumer admits each record it reads by the layout of the
//! type it holds, refusing another revision's by its tag or its length.
//!
//! `spec/conformance/packed/vectors.json` runs through both: every accepted
//! write is put by the owner's writer and read back by a tool's consumer,
//! byte for byte under its type's tag; every read is put as an envelope
//! carrying its case's tag, and the consumer delivers it or counts it
//! undecodable, `zenkey::inband::admit` naming the vector's refusal. The
//! contract is `tests/contracts/packed/packed.v1.toml`, a stream per type of
//! the vectors but `Wide64`, which no contract may list (E028).

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use common::{T, client, config, contract, eventually, router};
use serde_json::Value;
use zenkey::consumer::{Consumer, Delivery, Subscription};
use zenkey::model::contract::{Body, Contract};
use zenkey::model::grammar::IfaceId;
use zenkey::model::packed::Layout;
use zenkey::model::schema::TypeId;
use zenkey::model::template::Bindings;
use zenkey::model::wire::{Envelope, Run, Seq, Stamp, Tag};
use zenkey::{Implementation, ServiceBuilder};
use zenoh::bytes::Encoding;
use zenoh::sample::Sample;

const ADDR: &str = "lab/packed";

fn iface() -> IfaceId {
    "packed.v1".parse().unwrap()
}

fn vectors() -> Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../spec/conformance/packed/vectors.json");
    serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// Each packed stream of `c`: its type's name → (resource name, layout).
fn streams(c: &Contract) -> BTreeMap<String, (String, Layout)> {
    c.resources
        .iter()
        .filter(|r| r.token.as_str() == "stream")
        .filter_map(|r| {
            let Body::Data(d) = &r.body else { return None };
            let TypeId::Packed { name, .. } = &d.type_ else {
                return None;
            };
            let layout = c.packed_layout(&d.type_).unwrap().unwrap();
            Some((name.clone(), (format!("{}/{}", r.token, r.template), layout)))
        })
        .collect()
}

/// Exposes every resource of `c`: the contract requires them all (§8.2),
/// and the tests write some.
fn expose_all(b: &mut ServiceBuilder, c: &Contract) {
    for r in &c.resources {
        b.expose(&iface(), &format!("{}/{}", r.token, r.template))
            .unwrap();
    }
    b.serve_state(&iface()).unwrap();
}

fn tool(s: &zenoh::Session, c: &Contract) -> Consumer {
    Consumer::for_tool(s, Arc::new(c.clone()), &[ADDR], &BTreeMap::new()).unwrap()
}

/// A put's payload: `CH · run · seq · record`, CH carrying `tag`.
fn enveloped(tag: u8, seq: u32, record: &[u8]) -> Vec<u8> {
    Envelope {
        tag: Tag::new(tag).unwrap(),
        stamp: Stamp {
            run: Run::U32(0x00be_ef00),
            seq: Seq::new(seq).unwrap(),
        },
        record,
    }
    .write()
}

type Got = Arc<Mutex<Vec<Delivery>>>;

async fn subscribe(c: &Consumer, resource: &str, got: &Got) -> Subscription {
    let sink = Arc::clone(got);
    c.subscribe(resource, move |d| sink.lock().unwrap().push(d))
        .await
        .unwrap()
}

/// Every vector through the runtime: the accepted writes by the owner's
/// writers to a tool's consumer, the reads as envelopes to the same
/// consumer, and each read's refusal named by `zenkey::inband::admit`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_vector_through_the_runtime() {
    let (_r, ep) = router(None).await;
    let (owner, reader, raw) = (client(&ep).await, client(&ep).await, client(&ep).await);
    let c = contract("packed/packed.v1");
    let types = streams(&c);
    let mut b = ServiceBuilder::new(&owner, config(ADDR));
    b.implement(Implementation::new(c.clone())).unwrap();
    expose_all(&mut b, &c);
    let mut writers = BTreeMap::new();
    for (ty, (resource, _)) in &types {
        let w = b
            .declare_writer(&iface(), resource, &Bindings::new())
            .await
            .unwrap();
        writers.insert(ty.clone(), w);
    }
    let _svc = b.start().await.unwrap();

    let consumer = tool(&reader, &c);
    let mut subs = BTreeMap::new();
    let got: Got = Arc::default();
    for (ty, (resource, _)) in &types {
        subs.insert(ty.clone(), subscribe(&consumer, resource, &got).await);
    }
    let heard: Arc<Mutex<Vec<Sample>>> = Arc::default();
    let h = Arc::clone(&heard);
    let _raw_sub = raw
        .declare_subscriber(format!("zk2/{ADDR}/packed.v1/stream/**"))
        .callback(move |s| h.lock().unwrap().push(s))
        .await
        .unwrap();
    let doc = vectors();
    let sensor = &doc["writes"][0];
    let probe = unhex(sensor["expect"]["hex"].as_str().unwrap());
    eventually("the tool and the raw subscriber hear the owner", || async {
        writers["Sensor"].put(probe.clone()).await.unwrap();
        !got.lock().unwrap().is_empty() && !heard.lock().unwrap().is_empty()
    })
    .await;
    tokio::time::sleep(T / 4).await;
    got.lock().unwrap().clear();
    heard.lock().unwrap().clear();

    // The accepted writes: put by the owner's writer, CH carrying the
    // type's tag, read back byte for byte.
    let mut written = 0;
    for case in doc["writes"].as_array().unwrap() {
        let ty = case["type"].as_str().unwrap();
        let (Some(hex), Some((_, layout))) = (case["expect"]["hex"].as_str(), types.get(ty)) else {
            continue;
        };
        let note = case["note"].as_str().unwrap();
        let record = unhex(hex);
        writers[ty].put(record.clone()).await.unwrap();
        written += 1;
        eventually(note, || async { got.lock().unwrap().len() == written }).await;
        let d = got.lock().unwrap().last().cloned().unwrap();
        assert_eq!(&*d.sample.payload().to_bytes(), &record[..], "{note}");
        assert_eq!(d.tag.map(Tag::get), Some(layout.tag()), "{note}: CH's tag");
        let s = heard.lock().unwrap().last().cloned().unwrap();
        let on_wire = s.payload().to_bytes();
        assert_eq!(
            Envelope::parse(&on_wire).unwrap().tag.get(),
            layout.tag(),
            "{note}: on the wire"
        );
    }
    assert!(written > 30, "{written} writes");

    // The reads: an envelope carrying the case's tag. The consumer delivers
    // the record or counts it undecodable; `admit` names the refusal.
    let undecodable = |subs: &BTreeMap<String, Subscription>| -> u64 {
        subs.values().map(Subscription::undecodable).sum()
    };
    let mut refusals = std::collections::BTreeSet::new();
    let mut by_writer = 0;
    for (i, case) in doc["reads"].as_array().unwrap().iter().enumerate() {
        let ty = case["type"].as_str().unwrap();
        let Some((resource, layout)) = types.get(ty) else {
            continue;
        };
        let note = case["note"].as_str().unwrap();
        let tag = u8::try_from(case["tag"].as_u64().unwrap()).unwrap();
        let record = unhex(case["hex"].as_str().unwrap());
        let key = format!("zk2/{ADDR}/packed.v1/{resource}");
        let (delivered, refused) = (got.lock().unwrap().len(), undecodable(&subs));
        let n = heard.lock().unwrap().len();
        raw.put(&key, enveloped(tag, u32::try_from(i).unwrap(), &record))
            .encoding(Encoding::ZENOH_BYTES)
            .await
            .unwrap();
        eventually(note, || async { heard.lock().unwrap().len() > n }).await;
        let s = heard.lock().unwrap().last().cloned().unwrap();
        let admitted = zenkey::inband::admit(&zenkey::inband::read(s), Some(layout));
        let epoch = case.get("epoch").and_then(Value::as_i64);
        match (case["expect"].get("value"), case["expect"]["refused"].as_str()) {
            (Some(value), _) => {
                assert_eq!(admitted, Ok(()), "{note}");
                eventually(note, || async { got.lock().unwrap().len() > delivered }).await;
                let d = got.lock().unwrap().last().cloned().unwrap();
                let back = layout.read(tag, &d.sample.payload().to_bytes(), epoch);
                assert_eq!(back.as_ref(), Ok(value), "{note}: read back");
            }
            (None, Some(token)) => {
                assert_eq!(admitted.map_err(|e| e.token()), Err(token), "{note}");
                refusals.insert(token);
                eventually(note, || async { undecodable(&subs) > refused }).await;
                // The owner's writer refuses the same record under its own
                // tag, nothing published.
                if tag == layout.tag() {
                    let e = writers[ty].put(record.clone()).await.unwrap_err();
                    assert!(e.to_string().contains(&format!("{token}:")), "{note}: {e}");
                    by_writer += 1;
                }
            }
            _ => panic!("{note}: no expectation"),
        }
    }
    for token in ["tag", "length", "code", "extension", "pad"] {
        assert!(refusals.contains(token), "no read refused with {token}");
    }
    assert!(by_writer > 10, "{by_writer} refused by the writer");
    // Nothing the writer refused reached the bus.
    tokio::time::sleep(T / 4).await;
    assert_eq!(got.lock().unwrap().len() as u64 + undecodable(&subs), {
        let reads = doc["reads"].as_array().unwrap();
        (written + reads.iter().filter(|c| types.contains_key(c["type"].as_str().unwrap())).count()) as u64
    });
}

/// Another revision of the type, refused live by its tag and by its
/// length: a stream and a state member written as `Sensor`, read by tools
/// compiled against `SensorWide` (another tag) and `SensorCollide` (the
/// same tag, a bit wider), and by one compiled against `Sensor`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_wrong_revision_is_refused_by_tag_and_by_length() {
    let (_r, ep) = router(None).await;
    let (owner, reader) = (client(&ep).await, client(&ep).await);
    let c = contract("packed/packed.v1");
    let mut b = ServiceBuilder::new(&owner, config(ADDR));
    b.implement(Implementation::new(c.clone())).unwrap();
    expose_all(&mut b, &c);
    let stream = b
        .declare_writer(&iface(), "stream/sensor", &Bindings::new())
        .await
        .unwrap();
    let mut unit = Bindings::new();
    unit.insert("unit".into(), vec!["u1".into()]);
    let state = b
        .declare_state_writer(&iface(), "state/reading/{unit}", &unit)
        .await
        .unwrap();
    let _svc = b.start().await.unwrap();
    let record = unhex("4ceafbdcaa");
    state.put(record.clone()).await.unwrap();

    let revisions = [
        ("right", contract("packed/packed.v1")),
        ("wide", contract("packed-wide.v1")),
        ("collide", contract("packed-collide.v1")),
    ];
    let mut tools = Vec::new();
    for (name, rc) in &revisions {
        let consumer = tool(&reader, rc);
        let got: Got = Arc::default();
        let sub = subscribe(&consumer, "stream/sensor", &got).await;
        tools.push((*name, consumer, got, sub));
    }
    eventually("every tool hears the owner", || async {
        stream.put(record.clone()).await.unwrap();
        tools
            .iter()
            .all(|(_, _, got, sub)| !got.lock().unwrap().is_empty() || sub.undecodable() > 0)
    })
    .await;
    tokio::time::sleep(T / 4).await;
    for (name, _, got, sub) in &tools {
        let (delivered, refused) = (got.lock().unwrap().len(), sub.undecodable());
        match *name {
            "right" => assert!(delivered > 0 && refused == 0, "{name}: {delivered} {refused}"),
            _ => assert!(delivered == 0 && refused > 0, "{name}: {delivered} {refused}"),
        }
    }
    // S4's GET: the right tool's answer holds the record under its tag; the
    // others name the refusal.
    let key = format!("zk2/{ADDR}/packed.v1/state/reading/u1");
    for (name, consumer, _, _) in &tools {
        let a = consumer
            .get_answer("state/reading/{unit}", None, T)
            .await
            .unwrap();
        match *name {
            "right" => {
                let [zenkey::state::Current::Value { sample, .. }] = a.current.as_slice() else {
                    panic!("{name}: {:?}", a.current)
                };
                assert_eq!(&*sample.payload().to_bytes(), &record[..]);
                assert_eq!(a.tags.get(&key).map(|t| t.get()), Some(4));
            }
            "wide" => assert_eq!(a.undecodable.get(&key).map(|r| r.token()), Some("tag")),
            _ => assert_eq!(a.undecodable.get(&key).map(|r| r.token()), Some("length")),
        }
    }
}
