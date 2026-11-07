//! `spec/conformance/scope/faces.json` (observer §7.5, #990 V1): a tool's
//! scope across a constrained face, from the contracts in hand and, given a
//! face policy and its face, the face's plan. The fixture is evidence,
//! written from the text by an independent reading and never blessed: every
//! case must pass as written.

use std::path::{Path, PathBuf};

use serde_json::Value;
use zenkey_fleet::ContractSet;
use zenkey_fleet::model::face_scope::{Face, FaceScope, Reads, Refusal};
use zenkey_model::grammar::Addr;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap_or_else(|| panic!("a list: {v}"))
        .iter()
        .map(|s| s.as_str().expect("a string").to_owned())
        .collect()
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

#[test]
fn every_case_of_faces_json() {
    let text = std::fs::read_to_string(root().join("spec/conformance/scope/faces.json"))
        .expect("faces.json");
    let doc: Value = serde_json::from_str(&text).expect("JSON");
    let cases = doc["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 17, "the fixture's cases");
    let mut failures = Vec::new();
    for (i, case) in cases.iter().enumerate() {
        let note = case["note"].as_str().unwrap_or_default();
        let mut set = ContractSet::new();
        for p in strings(&case["contracts"]) {
            let (loaded, problems) = ContractSet::load_path(&root().join(&p));
            assert!(problems.is_empty(), "case {i}: {p}: {problems:?}");
            set.extend(loaded);
        }
        let across: Vec<Addr> = strings(&case["across"])
            .iter()
            .map(|a| a.parse().expect("an address"))
            .collect();
        let policy = case["policy"].as_str().map(|p| {
            let text = std::fs::read_to_string(root().join(p)).expect("the policy");
            zenkey_model::link::parse(&text)
                .policy
                .unwrap_or_else(|| panic!("case {i}: {p} is refused"))
        });
        let face = match (&policy, case["face"].as_str()) {
            (Some(policy), Some(name)) => Some(Face { policy, name }),
            (None, None) => None,
            _ => panic!("case {i}: a policy and a face come together"),
        };
        let reads = match case["reads"].as_str() {
            Some("samples") => Reads::Samples,
            Some("state") => Reads::State,
            r => panic!("case {i}: reads {r:?}"),
        };
        let expect = &case["expect"];
        let got: Value = match FaceScope::of(&across, &set, "", reads, face) {
            Err(Refusal::Plan(errors)) => {
                let mut codes: Vec<String> =
                    errors.iter().map(|e| e.code.as_str().to_owned()).collect();
                codes.sort();
                codes.dedup();
                serde_json::json!({ "plan_refused": codes })
            }
            Err(e) => serde_json::json!({ "error": e.to_string() }),
            Ok(scope) => match case["typed"].as_array() {
                None => serde_json::json!({ "selectors": sorted(scope.selectors().to_vec()) }),
                Some(_) => {
                    let n = scope
                        .narrow(&strings(&case["typed"]))
                        .expect("key expressions");
                    if n.unmet.is_empty() {
                        serde_json::json!({ "selectors": sorted(n.selectors) })
                    } else {
                        serde_json::json!({ "refused": n.unmet })
                    }
                }
            },
        };
        if &got != expect {
            failures.push(format!(
                "case {i} ({note}):\n  expected {expect}\n  got      {got}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
