//! Packed records read by a tool (core §7.6, observer §7.3; #979 B2): an
//! owner writes a packed stream and state member with the runtime, and the
//! fleet shows each as its fields, through S4's GET (`get state`), a
//! subscription (`watch`) and the lens's rendering of the raw sample
//! (`echo`), each decoded by the tag its envelope carried. Another
//! revision's record is undecodable with the layout's refusal named, and
//! `tape::synth` writes packed records the layout reads back.

mod util;

use std::path::Path;

use util::zk2::{T, addr, client, config, iface, router};
use zenkey::{Implementation, ServiceBuilder};
use zenkey_fleet::model::catalog::Revision;
use zenkey_fleet::model::render::{Member, render_with};
use zenkey_fleet::report::{ContractSource, Rendered, StateValue, WatchEvent};
use zenkey_fleet::{ResolvedTarget, StateRead, Synth};
use zenkey_model::contract::{Contract, load_path};
use zenkey_model::template::Bindings;
use zenkey_model::wire::{Envelope, Run, Seq, Stamp, Tag};

fn contract(name: &str) -> Contract {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../zenkey/tests/contracts/{name}.toml"));
    let l = load_path(&p);
    l.contract.unwrap_or_else(|| panic!("{name}: {}", l.report))
}

const RECORD: [u8; 5] = [0x4c, 0xea, 0xfb, 0xdc, 0xaa];

fn sensor() -> serde_json::Value {
    serde_json::json!({"temperature": "21.5", "humidity": "43.5", "pressure": "1013.2", "battery": "3.7"})
}

fn value_of(r: &Rendered) -> &serde_json::Value {
    match r {
        Rendered::Value {
            declared,
            value,
            epoch_assumed,
        } => {
            assert_eq!(declared, "packed:Sensor");
            assert_eq!(*epoch_assumed, None, "Sensor holds no time field");
            value
        }
        other => panic!("not a value: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tool_shows_a_packed_record_as_its_fields() {
    let (_r, ep) = router(None).await;
    let (owners, tool) = (client(&ep).await, client(&ep).await);
    let c = contract("packed/packed.v1");
    let packed = iface("packed.v1");
    let mut b = ServiceBuilder::new(&owners, config("lab/packed"));
    b.implement(Implementation::new(c.clone())).unwrap();
    for r in &c.resources {
        b.expose(&packed, &zenkey::implementation::resource_name(r))
            .unwrap();
    }
    b.serve_state(&packed).unwrap();
    let stream = b
        .declare_writer(&packed, "stream/sensor", &Bindings::new())
        .await
        .unwrap();
    let unit: Bindings = [("unit".to_owned(), vec!["u1".to_owned()])].into();
    let state = b
        .declare_state_writer(&packed, "state/reading/{unit}", &unit)
        .await
        .unwrap();
    let _svc = b.start().await.unwrap();
    state.put(RECORD.to_vec()).await.unwrap();

    let revision = Revision::from_contract(c.clone(), ContractSource::File);
    let owner = addr("lab/packed");

    // S4's GET, as `zenctl get state` reads it.
    let resource = revision
        .resource(
            "state/reading/{unit}",
            &[zenkey_model::authoring::Kind::State],
        )
        .unwrap();
    let read = || StateRead {
        revision: &revision,
        owner: &owner,
        resource,
        values: &unit,
        timeout: T,
    };
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    let report = loop {
        let report = zenkey_fleet::get_state(&tool, read()).await.unwrap();
        if !report.rows.is_empty() {
            break report;
        }
        assert!(tokio::time::Instant::now() < deadline, "never: S4's answer");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    let StateValue::Value { payload } = &report.rows[0].value else {
        panic!("{report:?}")
    };
    assert_eq!(value_of(&payload.rendered), &sensor());

    // A subscription, as `zenctl watch` reads it.
    let stream_r = revision
        .resource("stream/sensor", &[zenkey_model::authoring::Kind::Stream])
        .unwrap();
    let target = ResolvedTarget::parse("lab/packed").unwrap();
    let mut watch =
        zenkey_fleet::watch_resource(&tool, &revision, &target, stream_r, &Bindings::new())
            .await
            .unwrap();
    let raw = tool
        .declare_subscriber("zk2/lab/packed/packed.v1/stream/sensor")
        .await
        .unwrap();
    let sample = loop {
        stream.put(RECORD.to_vec()).await.unwrap();
        if let Ok(Some(s)) =
            tokio::time::timeout(std::time::Duration::from_millis(200), watch.next()).await
        {
            break s;
        }
        assert!(tokio::time::Instant::now() < deadline, "never: the watch");
    };
    let WatchEvent::Put {
        payload,
        conformance,
        ..
    } = &sample.event
    else {
        panic!("{sample:?}")
    };
    assert_eq!(value_of(&payload.rendered), &sensor());
    assert!(
        matches!(conformance, zenkey_fleet::report::Conformance::Valid),
        "{conformance:?}"
    );
    stream.put(RECORD.to_vec()).await.unwrap();

    // The raw sample, as `echo` renders it: the envelope read first, its
    // record decoded by CH's tag.
    let s = raw.recv_async().await.unwrap();
    let key = s.key_expr().as_str().to_owned();
    let bytes = s.payload().to_bytes();
    let shown = render_with(&revision, &key, Member::Type, Some("zenoh/bytes"), &bytes);
    assert_eq!(value_of(&shown.rendered), &sensor());

    // Another revision's record: undecodable, the layout's refusal named.
    let wide = Revision::from_contract(contract("packed-wide.v1"), ContractSource::File);
    match render_with(&wide, &key, Member::Type, Some("zenoh/bytes"), &bytes).rendered {
        Rendered::Undecodable { declared, reason } => {
            assert_eq!(declared, "packed:SensorWide");
            assert!(reason.contains("tag:"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
    let collide = Revision::from_contract(contract("packed-collide.v1"), ContractSource::File);
    match render_with(&collide, &key, Member::Type, Some("zenoh/bytes"), &bytes).rendered {
        Rendered::Undecodable { reason, .. } => assert!(reason.contains("length:"), "{reason}"),
        other => panic!("{other:?}"),
    }
    // A record whose envelope carries another tag, through the right
    // revision: refused by its tag.
    let forged = Envelope {
        form: zenkey_model::wire::Form::Mutation,
        tag: Tag::new(5).unwrap(),
        stamp: Stamp {
            run: Run::U16(1),
            seq: Seq::new(1).unwrap(),
        },
        record: &RECORD,
    }
    .write();
    match render_with(&revision, &key, Member::Type, Some("zenoh/bytes"), &forged).rendered {
        Rendered::Undecodable { reason, .. } => assert!(reason.contains("tag:"), "{reason}"),
        other => panic!("{other:?}"),
    }
}

/// `zenctl gen`'s synthesizer writes a packed record per (seed, tick),
/// deterministic, which the layout reads back as the value it reports.
#[test]
fn synth_writes_packed_records() {
    let c = contract("packed/packed.v1");
    let revision = Revision::from_contract(c.clone(), ContractSource::File);
    let synth = Synth::new(7);
    for r in &c.resources {
        let zenkey_model::contract::Body::Data(d) = &r.body else {
            continue;
        };
        let layout = c.packed_layout(&d.type_).unwrap().unwrap();
        for tick in 0..4 {
            let a = synth.sample(&revision, r, Member::Type, tick).unwrap();
            let b = synth.sample(&revision, r, Member::Type, tick).unwrap();
            assert_eq!(a.bytes, b.bytes, "{}: deterministic", r.template);
            let back = layout
                .read(
                    layout.tag(),
                    &a.bytes,
                    Some(zenkey_model::packed::DEFAULT_EPOCH),
                )
                .unwrap();
            assert_eq!(Some(back), a.value, "{}", r.template);
        }
    }
}

/// A packed record's `time` fields, read under the epoch a tool assumes
/// while nothing carries the deployment's (core §7.6, #980): the
/// rendering says which, so a reader sees the values rest on it.
#[test]
fn a_time_field_is_shown_under_the_epoch_assumed() {
    let c = contract("packed/packed.v1");
    let layout = |name: &str| {
        let r = c
            .resources
            .iter()
            .find(|r| r.template.as_str() == name)
            .unwrap();
        let zenkey_model::contract::Body::Data(d) = &r.body else {
            unreachable!()
        };
        c.packed_layout(&d.type_).unwrap().unwrap()
    };
    let clock = layout("clock");
    let record = clock
        .write(
            &serde_json::json!({"at": 1_700_000_000_000_i64, "at_ms": 1_700_000_000_000_i64, "valid": true}),
            Some(zenkey_model::packed::DEFAULT_EPOCH),
        )
        .unwrap();
    let bytes = Envelope {
        form: zenkey_model::wire::Form::Mutation,
        tag: Tag::new(clock.tag()).unwrap(),
        stamp: Stamp {
            run: Run::U16(1),
            seq: Seq::new(1).unwrap(),
        },
        record: &record.bytes,
    }
    .write();
    let revision = Revision::from_contract(c.clone(), ContractSource::File);
    let key = "zk2/lab/packed/packed.v1/stream/clock";
    match render_with(&revision, key, Member::Type, Some("zenoh/bytes"), &bytes).rendered {
        Rendered::Value {
            declared,
            value,
            epoch_assumed,
        } => {
            assert_eq!(declared, "packed:Clock");
            assert_eq!(value["at"], 1_700_000_000_000_i64);
            assert_eq!(epoch_assumed, Some(zenkey_model::packed::DEFAULT_EPOCH));
            let json = serde_json::to_value(Rendered::Value {
                declared,
                value,
                epoch_assumed,
            })
            .unwrap();
            assert_eq!(json["epoch_assumed"], 0, "{json}");
        }
        other => panic!("{other:?}"),
    }
}
