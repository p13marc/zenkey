//! `config.v2`'s derived types through the generated code (#985 G2):
//! `rfcfg.v1`, the spec's test contract, declares its groups, and
//! `zk2 config derive` wrote `Record`, one `Set<T>` per settable group,
//! `Control` and `Error` into its packed file, which the build generates
//! like any other. Every record of `conformance/v2/records.json` and every
//! request of `validation.json` reads and writes back through them exactly
//! as the model's codec does, refusals included.

use std::path::Path;

use serde_json::Value;
use zenkey::model::config::v2::Interface;
use zenkey::model::contract::load_path;
use zenkey::packed::PackedRecord;
use zenkey_codegen_test::zk2::rfcfg_v1::types::*;

fn fixture(name: &str) -> Value {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../spec/profiles/config/conformance/v2")
        .join(name);
    serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
}

fn interface() -> Interface {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../spec/profiles/config/conformance/contracts/rfcfg.v1.toml");
    Interface::of(&load_path(&p).contract.expect("rfcfg.v1 loads")).unwrap()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// `bytes` read by the generated `T` and by the model's layout of the
/// type named `name`: the same JSON reading and the same bytes written
/// back, or the same refusal.
fn agree<T: PackedRecord>(i: &Interface, name: &str, tag: u8, bytes: &[u8], epoch: Option<i64>) {
    let layout = i
        .derived
        .layout(name)
        .unwrap_or_else(|| panic!("{name} is derived"));
    let model = layout.read(tag, bytes, epoch);
    match (T::decode(tag, bytes, epoch), model) {
        (Ok(t), Ok(json)) => {
            assert_eq!(t.to_json(), json, "{name}: the JSON reading");
            assert_eq!(
                t.encode(epoch).unwrap().bytes,
                bytes,
                "{name}: written back"
            );
        }
        (Err(a), Err(b)) => assert_eq!(a.token(), b.token(), "{name}: the refusal"),
        (a, b) => panic!(
            "{name}: generated {:?}, model {:?}",
            a.map(|t| t.to_json()),
            b
        ),
    }
}

/// Each type `derive.json`'s derivation names is generated, under its name.
#[test]
fn the_derived_types_are_generated() {
    let i = interface();
    let mut names: Vec<&str> = i.derived.names().collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "Control", "Error", "Record", "SetDuty", "SetLink", "SetQueue", "SetRadio", "SetSim"
        ]
    );
    // Each generated type's tag is its derivation's.
    let tag = |n: &str| i.derived.layout(n).unwrap().tag();
    assert_eq!(<Record as PackedRecord>::TAG, tag("Record"));
    assert_eq!(<SetRadio as PackedRecord>::TAG, tag("SetRadio"));
    assert_eq!(<Control as PackedRecord>::TAG, tag("Control"));
    assert_eq!(<Error as PackedRecord>::TAG, tag("Error"));
}

/// `records.json`: every write's bytes and every read's, through the
/// generated `Record`. A read config.v2 refuses past core §7.6 (`pending`,
/// `last`, …) is a packed record still, read alike.
#[test]
fn every_record_reads_and_writes_through_the_generated_record() {
    let i = interface();
    let f = fixture("records.json");
    let tag = i.derived.record().tag();
    for w in f["writes"].as_array().unwrap() {
        let bytes = unhex(w["expect"]["hex"].as_str().unwrap());
        let epoch = w["epoch"].as_i64();
        agree::<Record>(&i, "Record", tag, &bytes, epoch);
        let r = Record::decode(tag, &bytes, epoch).unwrap();
        assert_eq!(r.to_json(), w["expect"]["fields"], "{}", w["note"]);
    }
    for r in f["reads"].as_array().unwrap() {
        let bytes = unhex(r["hex"].as_str().unwrap());
        let t = u8::try_from(r["tag"].as_u64().unwrap()).unwrap();
        agree::<Record>(&i, "Record", t, &bytes, r["epoch"].as_i64());
    }
}

/// `validation.json`: every request's record, after CH, through its
/// operation's generated type.
#[test]
fn every_request_reads_through_its_generated_type() {
    let i = interface();
    let f = fixture("validation.json");
    for c in f["cases"].as_array().unwrap() {
        let Some(hex) = c["hex"].as_str() else {
            continue;
        };
        let payload = unhex(hex);
        let Some((ch, record)) = payload.split_first() else {
            continue;
        };
        let tag = ch & 0x0f;
        match (c["op"].as_str().unwrap(), c["group"].as_str()) {
            ("set", Some("duty")) => agree::<SetDuty>(&i, "SetDuty", tag, record, None),
            ("set", Some("radio")) => agree::<SetRadio>(&i, "SetRadio", tag, record, None),
            ("set", Some("link")) => agree::<SetLink>(&i, "SetLink", tag, record, None),
            ("set", Some("sim")) => agree::<SetSim>(&i, "SetSim", tag, record, None),
            ("set", Some("queue")) => agree::<SetQueue>(&i, "SetQueue", tag, record, None),
            ("set", _) => {}
            _ => agree::<Control>(&i, "Control", tag, record, None),
        }
    }
}
