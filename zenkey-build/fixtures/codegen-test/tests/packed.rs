//! `spec/conformance/packed/vectors.json` through the generated code (core
//! §7.6, #979 B2): each type's tag and lengths, each write's bits, pad and
//! bytes or the writer's refusal, each read's JSON reading or the reader's
//! refusal, the first in §7.6's order. The generated encoder and decoder
//! are straight-line code over `zenkey::packed`; the vectors were written
//! by an independent reading of the text, so they check the generated code
//! and the model's interpreted codec alike.
//!
//! Every type of the vectors is generated but `Wide64`: its bounds lie past
//! the canonical domain, so no contract lists it (E028), and no generated
//! code meets it.

use std::path::Path;

use serde_json::{Value, json};
use zenkey::codec::{Codec, Packed};
use zenkey::model::wire::Tag;
use zenkey::packed::PackedRecord;
use zenkey_codegen_test::zk2::packed_v1::types::*;

fn vectors() -> Value {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../spec/conformance/packed/vectors.json");
    serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// The types no contract lists, which no generated code meets.
const NOT_GENERATED: &[&str] = &["Wide64"];

/// Runs `f` with the generated type named `ty`.
macro_rules! by_type {
    ($ty:expr, $f:ident, $($arg:expr),*) => {
        match $ty {
            "Sensor" => $f::<Sensor>($($arg),*),
            "Status" => $f::<Status>($($arg),*),
            "Reserved" => $f::<Reserved>($($arg),*),
            "Counters" => $f::<Counters>($($arg),*),
            "Ties" => $f::<Ties>($($arg),*),
            "Clock" => $f::<Clock>($($arg),*),
            "Mode" => $f::<Mode>($($arg),*),
            "Link" => $f::<Link>($($arg),*),
            "LinkV2" => $f::<LinkV2>($($arg),*),
            "Grow" => $f::<Grow>($($arg),*),
            "Chain" => $f::<Chain>($($arg),*),
            "SensorRenamed" => $f::<SensorRenamed>($($arg),*),
            "SensorWide" => $f::<SensorWide>($($arg),*),
            "SensorCollide" => $f::<SensorCollide>($($arg),*),
            other => panic!("no generated type {other}"),
        }
    };
}

fn layout<T: PackedRecord>(case: &Value) {
    let e = &case["expect"];
    assert_eq!(u64::from(T::TAG), e["tag"].as_u64().unwrap(), "tag of {}", T::NAME);
    assert_eq!(T::MIN_BITS, e["min_bits"].as_u64().unwrap(), "min of {}", T::NAME);
    assert_eq!(T::MAX_BITS, e["max_bits"].as_u64().unwrap(), "max of {}", T::NAME);
}

fn write<T: PackedRecord>(case: &Value) {
    let note = &case["note"];
    let epoch = case.get("epoch").and_then(Value::as_i64);
    let got = match T::from_json(&case["value"]).and_then(|v| v.encode(epoch)) {
        Ok(r) => {
            let want = case.get("reads_as").unwrap_or(&case["value"]);
            let back = T::decode(T::TAG, &r.bytes, epoch).unwrap_or_else(|e| panic!("{note}: {e}"));
            assert_eq!(&back.to_json(), want, "{note}: reads back");
            json!({"hex": hex(&r.bytes), "bits": r.bit_string(), "pad": r.pad()})
        }
        Err(e) => json!({"refused": e.token()}),
    };
    assert_eq!(case["expect"], got, "write: {note}");
}

fn read<T: PackedRecord>(case: &Value) -> Option<&'static str> {
    let note = &case["note"];
    let epoch = case.get("epoch").and_then(Value::as_i64);
    let tag = u8::try_from(case["tag"].as_u64().unwrap()).unwrap();
    let bytes = unhex(case["hex"].as_str().unwrap());
    let (got, refused) = match T::decode(tag, &bytes, epoch) {
        Ok(v) => (json!({"value": v.to_json()}), None),
        Err(e) => (json!({"refused": e.token()}), Some(e.token())),
    };
    assert_eq!(case["expect"], got, "read: {note}");
    refused
}

#[test]
fn every_vector_through_the_generated_code() {
    let doc = vectors();
    let generated = |case: &Value| !NOT_GENERATED.contains(&case["type"].as_str().unwrap());
    let mut counts = [0usize; 3];
    for case in doc["layouts"].as_array().unwrap().iter().filter(|c| generated(c)) {
        by_type!(case["type"].as_str().unwrap(), layout, case);
        counts[0] += 1;
    }
    for case in doc["writes"].as_array().unwrap().iter().filter(|c| generated(c)) {
        by_type!(case["type"].as_str().unwrap(), write, case);
        counts[1] += 1;
    }
    let mut refused = std::collections::BTreeSet::new();
    for case in doc["reads"].as_array().unwrap().iter().filter(|c| generated(c)) {
        refused.extend(by_type!(case["type"].as_str().unwrap(), read, case));
        counts[2] += 1;
    }
    for token in ["tag", "length", "code", "extension", "pad"] {
        assert!(refused.contains(token), "no read refuses with {token}");
    }
    assert!(counts.iter().all(|n| *n > 0), "{counts:?}");
}

/// The types the fixture contract lists are the vectors' schema's, Wide64
/// alone left out.
#[test]
fn the_fixtures_types_are_the_vectors() {
    let doc = vectors();
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../zenkey/tests/contracts/packed/vectors.json");
    let fixture: Value = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
    let mut want = doc["schema"]["types"].as_object().unwrap().clone();
    for t in NOT_GENERATED {
        assert!(want.remove(*t).is_some(), "{t}");
    }
    assert_eq!(fixture["types"], Value::Object(want));
}

/// The typed codec over a generated type: its record, read back by the tag
/// its envelope carries and refused under another; a fixed-point value is
/// exact, a reserved code typed, an escape typed.
#[test]
fn the_codec_carries_a_generated_type() {
    use zenkey::packed::{Coded, Decimal, Open};
    let s = Sensor {
        temperature: "21.5".parse().unwrap(),
        humidity: Decimal::from_f64(43.5).unwrap(),
        pressure: "1013.2".parse().unwrap(),
        battery: "3.70".parse().unwrap(),
    };
    let bytes = Packed::<Sensor>::encode(&s, None).unwrap();
    assert_eq!(hex(&bytes), "4ceafbdcaa");
    let tag = Packed::<Sensor>::tag().unwrap();
    assert_eq!(Packed::<Sensor>::decode_record(&bytes, None, Some(tag)).unwrap(), s);
    let other = Tag::new((tag.get() + 1) % 16).unwrap();
    let e = Packed::<Sensor>::decode_record(&bytes, None, Some(other)).unwrap_err();
    assert!(e.contains("tag"), "{e}");
    assert!(Packed::<Sensor>::decode_record(&bytes, None, None).is_err());
    // SensorCollide shares Sensor's tag: its length refuses the record.
    let e = Packed::<SensorCollide>::decode_record(&bytes, None, Some(tag)).unwrap_err();
    assert!(e.contains("length"), "{e}");

    let r = Reserved {
        level: Coded::NotAvailable,
        temperature: Coded::Value("0".parse().unwrap()),
        phase: reserved::Phase::C,
    };
    let rec = r.encode(None).unwrap();
    assert_eq!(Reserved::decode(Reserved::TAG, &rec.bytes, None).unwrap(), r);
    let m = Mode {
        mode: Open::Escape,
        wide: Open::Known(mode::Wide::Y),
    };
    let rec = m.encode(None).unwrap();
    assert_eq!(Mode::decode(Mode::TAG, &rec.bytes, None).unwrap(), m);
}
