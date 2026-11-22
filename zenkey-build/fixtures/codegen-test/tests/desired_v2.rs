//! `desired.v2`'s packed `Report` through the generated code (#986 J2):
//! `worker.v2`, the scenarios' target contract, lists the profile's
//! standard schema, and the build generates its one type like any other
//! packed type. Every record `conformance/v2/reports.json` writes and
//! reads goes through it exactly as the model's codec reads it, refusals
//! included; §2.10's consistency is the model's
//! (`zenkey::model::desired::v2::Fields::check`), over the same fields.

use std::path::Path;

use serde_json::Value;
use zenkey::model::desired::v2;
use zenkey::packed::PackedRecord;
use zenkey_codegen_test::zk2::worker_v2;
use zenkey_codegen_test::zk2::worker_v2::types::Report;

fn fixture() -> Value {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../spec/profiles/desired/conformance/v2/reports.json");
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

#[test]
fn the_generated_report_is_the_profiles() {
    let doc = fixture();
    assert_eq!(<Report as PackedRecord>::TAG, v2::TAG);
    assert_eq!(
        u64::from(<Report as PackedRecord>::TAG),
        doc["layout"]["tag"].as_u64().unwrap()
    );
    assert_eq!(<Report as PackedRecord>::MIN_BITS, v2::layout().min_bits());
    assert_eq!(<Report as PackedRecord>::MAX_BITS, v2::layout().max_bits());
    assert_eq!(worker_v2::resource::DESIRED_TASK, "state/desired/task");
    assert_eq!(worker_v2::role::TASK, "task");
    // The contract the module embeds finds the role's report where the
    // scenarios put it.
    let target = "lab/worker".parse().unwrap();
    let at = v2::report_at_of(&target, &worker_v2::contract(), worker_v2::role::TASK);
    assert_eq!(
        at.key().map(|k| k.as_str()),
        Some("zk2/lab/worker/worker.v2/state/desired/task")
    );
}

#[test]
fn every_record_through_the_generated_code() {
    let doc = fixture();
    let (mut written, mut read, mut refused) = (0, 0, 0);
    for case in doc["writes"].as_array().unwrap() {
        let note = &case["note"];
        // A writer's refusal is of the profile's reading, the model's to
        // give: a record's fields are what the generated type writes.
        let Some(fields) = case["expect"].get("fields") else {
            continue;
        };
        let report = Report::from_json(fields).unwrap_or_else(|e| panic!("{note}: {e}"));
        let record = report
            .encode(None)
            .unwrap_or_else(|e| panic!("{note}: {e}"));
        assert_eq!(hex(&record.bytes), case["expect"]["hex"], "write: {note}");
        assert_eq!(record.bit_string(), case["expect"]["bits"], "write: {note}");
        // The model's typed fields write the same bytes.
        let model = v2::Fields::of_json(fields).unwrap().record().unwrap();
        assert_eq!(record.bytes, model.bytes, "write: {note}");
        written += 1;
    }
    for case in doc["reads"].as_array().unwrap() {
        let note = &case["note"];
        let tag = u8::try_from(case["tag"].as_u64().unwrap()).unwrap();
        let bytes = unhex(case["hex"].as_str().unwrap());
        match (Report::decode(tag, &bytes, None), v2::read(tag, &bytes)) {
            (Ok(r), Ok(model)) => {
                assert_eq!(r.to_json(), case["expect"]["fields"], "read: {note}");
                assert_eq!(r.to_json(), model.fields.to_json(), "read: {note}");
                read += 1;
            }
            (Err(a), Err(b)) => {
                assert_eq!(a.token(), case["expect"]["refused"], "read: {note}");
                assert_eq!(a.token(), b.token(), "read: {note}");
                refused += 1;
            }
            (a, b) => panic!(
                "read: {note}: generated {:?}, model {b:?}",
                a.map(|r| r.to_json())
            ),
        }
    }
    assert_eq!((written, read, refused), (19, 49, 8));
}
