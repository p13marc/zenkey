//! The profiles' fixtures (`spec/profiles/*/conformance/`, #719).
//!
//! Each fixture holds its inputs and the expected outputs, and this test
//! checks them. Every file under a profile's `conformance/` must be one
//! this crate knows how to run: an unknown one fails
//! [`every_profile_fixture_is_known`], never skipped, so a fixture cannot
//! land unrun (`spec/profiles/README.md`). A `README.md` there is prose.
//!
//! `ZK2_BLESS=1 cargo test -p zenkey-model --test profiles` rewrites the
//! expected outputs from this implementation. Then read the diff: a bless
//! is a claim that every changed expectation is right.

use std::path::{Path, PathBuf};

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Value, json};
use zenkey_model::authoring::Kind;
use zenkey_model::freshness::{self, ClockTrust, Horizon, Last, Observation, Reply, StampAge};
use zenkey_model::hostid;
use zenkey_model::slug::chunk_slug;

/// The fixtures this crate runs, as (profile directory, file name).
const KNOWN: &[(&str, &str)] = &[
    ("hostid", "vectors.json"),
    ("hostid", "shapes.json"),
    ("freshness", "horizons.json"),
    ("freshness", "judgements.json"),
];

fn profiles() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/profiles")
}

fn bless() -> bool {
    std::env::var_os("ZK2_BLESS").is_some()
}

fn read_json(p: &Path) -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display())),
    )
    .unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn write_json(p: &Path, v: &Value) {
    std::fs::write(p, serde_json::to_string_pretty(v).unwrap() + "\n").unwrap();
}

/// Checks `got` against the case's `expect`, or blesses it.
fn expect(case: &mut Value, got: Value, what: &str) {
    if bless() {
        case["expect"] = got;
    } else {
        assert_eq!(case["expect"], got, "{what}");
    }
}

fn sorted_entries(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .collect();
    v.sort();
    v
}

/// Every file under `spec/profiles/<profile>/conformance/` is in [`KNOWN`]
/// (or is its `README.md`), and every entry of [`KNOWN`] exists.
#[test]
fn every_profile_fixture_is_known() {
    let mut unknown = Vec::new();
    let mut seen = Vec::new();
    for dir in sorted_entries(&profiles()) {
        let profile = dir.file_name().unwrap().to_string_lossy().into_owned();
        // `README.md` is the index; `.history/` holds published contracts
        // (core §9.7), checked like `examples/zk2/.history` once one exists.
        if !dir.is_dir() || profile == ".history" {
            continue;
        }
        let conformance = dir.join("conformance");
        if !conformance.exists() {
            continue;
        }
        for file in sorted_entries(&conformance) {
            let name = file.file_name().unwrap().to_string_lossy().into_owned();
            if name == "README.md" && file.is_file() {
                continue;
            }
            if file.is_file() && KNOWN.contains(&(profile.as_str(), name.as_str())) {
                seen.push((profile.clone(), name));
            } else {
                unknown.push(file.display().to_string());
            }
        }
    }
    assert!(
        unknown.is_empty(),
        "profile fixtures this crate does not run (add a runner and list them in KNOWN): {unknown:#?}"
    );
    for (profile, name) in KNOWN {
        assert!(
            seen.iter().any(|(p, n)| p == profile && n == name),
            "{profile}/conformance/{name} is listed in KNOWN but missing"
        );
    }
}

/// `hostid/conformance/vectors.json`: a machine id and a salt → the minted
/// system, or `null` when the input is refused (hostid.v1 §2.1).
#[test]
fn hostid_vectors() {
    let path = profiles().join("hostid/conformance/vectors.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let input = case["input"].as_str().expect("input").to_owned();
        let salt = case["salt"].as_str().expect("salt").to_owned();
        let got = hostid::derive(&input, &salt);
        if let Some(s) = &got {
            assert!(hostid::is_minted_shape(s), "{input:?}: {s} is in the shape");
            assert_eq!(chunk_slug(s), *s, "{input:?}: {s} is its own slug");
        }
        if salt == hostid::SALT {
            assert_eq!(
                hostid::system(&input).as_ref().map(|n| n.as_str()),
                got.as_deref(),
                "{input:?}: system() derives with the one salt"
            );
        }
        expect(case, json!(got), &format!("{input:?} with {salt:?}"));
    }
    if bless() {
        write_json(&path, &doc);
    }
}

/// `hostid/conformance/shapes.json`: a string → whether it is in the minted
/// shape (hostid.v1 §2.11).
#[test]
fn hostid_shapes() {
    let path = profiles().join("hostid/conformance/shapes.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let value = case["value"].as_str().expect("value").to_owned();
        expect(
            case,
            json!(hostid::is_minted_shape(&value)),
            &format!("{value:?}"),
        );
    }
    if bless() {
        write_json(&path, &doc);
    }
}

/// A case's kind and merged annotations, as `freshness/conformance/` writes
/// them.
fn resource(case: &Value) -> (Kind, BTreeMap<String, Value>) {
    let kind = match case["kind"].as_str().expect("kind") {
        "stream" => Kind::Stream,
        "state" => Kind::State,
        "event" => Kind::Event,
        "operation" => Kind::Operation,
        other => panic!("kind {other:?}"),
    };
    let annotations = case["annotations"]
        .as_object()
        .expect("annotations")
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    (kind, annotations)
}

fn secs(v: &Value) -> Duration {
    Duration::from_secs_f64(v.as_f64().expect("seconds"))
}

/// `freshness/conformance/horizons.json`: a resource's kind and annotations
/// → its horizon (freshness.v1 §2.1–§2.3), with the owner's bound between
/// two puts (§2.4).
#[test]
fn freshness_horizons() {
    let path = profiles().join("freshness/conformance/horizons.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let (kind, annotations) = resource(case);
        let h = freshness::horizon(kind, &annotations);
        let got = match &h {
            Horizon::Undeclared => json!({"horizon": "none"}),
            Horizon::Ignored => json!({"horizon": "ignored"}),
            Horizon::Invalid(_) => json!({"horizon": "invalid"}),
            Horizon::Never => json!({"horizon": "never"}),
            Horizon::Within(t) => json!({
                "horizon": "within",
                "ttl_s": t.as_secs(),
                "refresh_ms": u64::try_from(h.refresh_bound().expect("a bound").as_millis())
                    .expect("fits"),
            }),
        };
        let what = format!("{} {}", kind.as_str(), case["annotations"]);
        expect(case, got, &what);
    }
    if bless() {
        write_json(&path, &doc);
    }
}

/// One observation, as `judgements.json` writes it.
fn observation(o: &Value) -> Observation {
    match o["via"].as_str().expect("via") {
        "archive" => Observation::Archived,
        "subscription" => Observation::Subscribed {
            last: match &o["last"] {
                Value::Null => None,
                l => {
                    let age = secs(&l["age_s"]);
                    match l["kind"].as_str().expect("last.kind") {
                        "put" => Some(Last::Put(age)),
                        "delete" => Some(Last::Delete(age)),
                        other => panic!("last.kind {other:?}"),
                    }
                }
            },
            listened: secs(&o["listened_s"]),
            complete: o["complete"].as_bool().expect("complete"),
        },
        "get" => {
            let clock = match o.get("clock") {
                Some(c) if c["trusted"] == json!(true) => ClockTrust::Trusted {
                    delta: secs(&c["delta_s"]),
                },
                _ => ClockTrust::Untrusted,
            };
            let reply = match &o["reply"] {
                Value::Null => None,
                r => Some(match r["kind"].as_str().expect("reply.kind") {
                    "delete" => Reply::Delete,
                    "put" => Reply::Put {
                        stamp_age: r["stamp_age_s"].as_f64().map(StampAge::from_secs_f64),
                    },
                    other => panic!("reply.kind {other:?}"),
                }),
            };
            Observation::Got { reply, clock }
        }
        other => panic!("via {other:?}"),
    }
}

/// `freshness/conformance/judgements.json`: a horizon and the observations
/// of one member → the combined verdict and its reason class
/// (freshness.v1 §2.3, §2.5–§2.8).
#[test]
fn freshness_judgements() {
    let path = profiles().join("freshness/conformance/judgements.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let (kind, annotations) = resource(case);
        let h = freshness::horizon(kind, &annotations);
        let observations: Vec<Observation> = case["observations"]
            .as_array()
            .expect("observations")
            .iter()
            .map(observation)
            .collect();
        let j = freshness::judge_all(&h, &observations);
        let what = format!("{}: {}", case["note"], case["observations"]);
        expect(
            case,
            json!({"verdict": j.verdict.as_str(), "reason": j.reason.as_str()}),
            &what,
        );
    }
    if bless() {
        write_json(&path, &doc);
    }
}
