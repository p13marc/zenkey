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
use zenkey_model::health::{self, Judged, Level, Listing, Presence, Read, Reading};
use zenkey_model::hostid;
use zenkey_model::slug::chunk_slug;

/// The fixtures this crate runs, as (profile directory, file name).
const KNOWN: &[(&str, &str)] = &[
    ("hostid", "vectors.json"),
    ("hostid", "shapes.json"),
    ("freshness", "horizons.json"),
    ("freshness", "judgements.json"),
    ("health", "judgements.json"),
    ("health", "rollups.json"),
    ("health", "codes.json"),
    ("freshness", "clock-trust.json"),
    ("health", "clock.json"),
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
        // `README.md` is the index; `.history/` holds the published standard
        // contracts (core §9.7), checked by
        // `every_standard_contract_is_published_and_compatible`.
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

/// Every profile's standard contract, `spec/profiles/<name>/<name>.v<N>.toml`
/// (`spec/profiles/README.md`), with its file named by its interface id.
fn standard_contracts() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in sorted_entries(&profiles()) {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        if !dir.is_dir() || name.starts_with('.') {
            continue;
        }
        for file in sorted_entries(&dir) {
            let f = file.file_name().unwrap().to_string_lossy().into_owned();
            if file.is_file() && f.starts_with(&format!("{name}.v")) && f.ends_with(".toml") {
                out.push(file);
            }
        }
    }
    out
}

/// Every standard contract loads with no finding at all, fingerprints and
/// bundles, is named by its interface id, and is published in
/// `spec/profiles/.history` (core §9.7), compatible with every earlier
/// revision of its interface there, as `tests/examples.rs` requires of
/// `examples/zk2/`. Every interface published there has its contract in the
/// tree. After an intended change, publish the new revision with
/// `zk2 contract bundle <file> --history spec/profiles/.history`.
#[test]
fn every_standard_contract_is_published_and_compatible() {
    use zenkey_model::bundle::Bundle;
    use zenkey_model::canonical::Fingerprint;
    use zenkey_model::compat::{Class, Revision, check_history, same_revision};
    use zenkey_model::contract::load_path;

    let root = profiles().join(".history");
    let problems = zenkey_model::history::check_tagged(&root);
    assert!(problems.is_empty(), "{problems:#?}");
    let files = standard_contracts();
    assert!(
        files.iter().any(|p| p.ends_with("health/health.v1.toml")),
        "health.v1's standard contract: {files:?}"
    );
    let mut ifaces = Vec::new();
    for p in &files {
        let rel = p.strip_prefix(profiles()).unwrap().display().to_string();
        let l = load_path(p);
        assert!(l.report.0.is_empty(), "{rel}:\n{}", l.report);
        let c = l.contract.expect("no finding, so a contract");
        assert_eq!(
            p.file_stem().unwrap().to_string_lossy(),
            c.iface.to_string(),
            "{rel}: a standard contract's file is named by its interface id"
        );
        let b = Bundle::build(&c);
        let v = Bundle::verify_expecting(&b.to_bytes(), &Fingerprint::of(&c));
        assert!(v.is_ok(), "{rel}: {:?}", v.err());
        let dir = root.join(c.iface.to_string());
        let mut revs = Vec::new();
        for e in std::fs::read_dir(&dir)
            .unwrap_or_else(|_| panic!("{rel}: no history at {}", dir.display()))
        {
            let bytes = std::fs::read(e.unwrap().path()).unwrap();
            revs.push(Revision::of_bundle(&Bundle::verify(&bytes).unwrap()));
        }
        let new = Revision::of(&c);
        assert!(
            revs.iter().any(|r| same_revision(r, &new)),
            "{rel}: the current revision is not published; run zk2 contract bundle spec/profiles/{rel} --history spec/profiles/.history"
        );
        let v = check_history(&revs, &new);
        assert_eq!(v.class(), Class::Compatible, "{rel}: {:#?}", v.findings);
        ifaces.push(c.iface.to_string());
    }
    for dir in sorted_entries(&root) {
        let iface = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            ifaces.contains(&iface),
            "spec/profiles/.history/{iface} has no standard contract in the tree"
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

/// A level as `health/conformance/` writes it: a token, or an integer the
/// enum does not list.
fn level_read(v: &Value) -> Read {
    match v {
        Value::String(s) => match s.as_str() {
            "ok" => Read::Level(Level::Ok),
            "degraded" => Read::Level(Level::Degraded),
            "failed" => Read::Level(Level::Failed),
            "unspecified" => Read::Unspecified,
            "undecodable" => Read::Undecodable,
            other => panic!("level {other:?}"),
        },
        Value::Number(n) => {
            let n = i32::try_from(n.as_i64().expect("an integer level")).expect("an i32");
            let r = Read::from_wire(n);
            assert!(
                matches!(r, Read::Unlisted(_)),
                "{n}: an integer level is one the enum does not list"
            );
            r
        }
        other => panic!("level {other}"),
    }
}

fn level_token(l: Option<Level>) -> Value {
    l.map_or(Value::Null, |l| json!(l.as_str()))
}

/// A case's presence, descriptor and face, as `health/conformance/` writes
/// them.
fn health_presence(case: &Value) -> Presence {
    match case["presence"].as_str().expect("presence") {
        "present" => Presence::Present(match &case["descriptor"] {
            Value::Null => Listing::Unread,
            d if d["lists"] == json!(false) => Listing::NotListed,
            d => Listing::Listed {
                token: d["token"].as_bool().expect("descriptor.token"),
            },
        }),
        "absent" => Presence::Absent,
        "incomplete" => Presence::Incomplete,
        "across_face" => Presence::AcrossFace {
            status_crosses: case["face"]["status_crosses"]
                .as_bool()
                .expect("face.status_crosses"),
        },
        other => panic!("presence {other:?}"),
    }
}

/// `health/conformance/clock.json`: what one subscriber heard of one service
/// in its window → the answer to "is this service's clock ahead?"
/// (health.v1 §5, §2.5).
#[test]
fn health_clock() {
    let path = profiles().join("health/conformance/clock.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let presence = health_presence(case);
        let heard: Vec<health::Heard> = case["heard"]
            .as_array()
            .expect("heard")
            .iter()
            .map(|h| match h.as_str().expect("a delivery") {
                "status" => health::Heard::Status,
                "status_deleted" => health::Heard::StatusDeleted,
                "clock_ahead" => health::Heard::ClockAhead,
                "fault" => health::Heard::Fault,
                other => panic!("heard {other:?}"),
            })
            .collect();
        let c = health::clock_ahead(presence, &heard);
        let what = format!("{}", case["note"]);
        expect(
            case,
            json!({"answer": c.answer.as_str(), "reason": c.reason.as_str()}),
            &what,
        );
    }
    if bless() {
        write_json(&path, &doc);
    }
}

/// `health/conformance/judgements.json`: one reading of one service → the
/// answer to "is this service healthy?" (health.v1 §2.11).
#[test]
fn health_judgements() {
    let path = profiles().join("health/conformance/judgements.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let presence = health_presence(case);
        let observations: Vec<Observation> = case["status"]["observations"]
            .as_array()
            .expect("status.observations")
            .iter()
            .map(observation)
            .collect();
        let level = match &case["status"]["level"] {
            Value::Null => None,
            v => Some(level_read(v)),
        };
        let checks: Option<Vec<Read>> = match &case["checks"] {
            Value::Null => None,
            v => Some(
                v.as_array()
                    .expect("checks")
                    .iter()
                    .map(|c| level_read(&c["level"]))
                    .collect(),
            ),
        };
        let j = health::judge(&Reading {
            presence,
            status: &observations,
            level,
            checks: checks.as_deref(),
        });
        let what = format!("{}", case["note"]);
        expect(
            case,
            json!({
                "verdict": j.verdict.as_str(),
                "reason": j.reason.as_str(),
                "level": level_token(j.level),
            }),
            &what,
        );
    }
    if bless() {
        write_json(&path, &doc);
    }
}

/// `health/conformance/rollups.json`: several judged readings → a tool's
/// roll-up (health.v1 §2.2, §5).
#[test]
fn health_rollups() {
    let path = profiles().join("health/conformance/rollups.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let judged: Vec<Judged> = case["judged"]
            .as_array()
            .expect("judged")
            .iter()
            .map(|j| {
                let verdict = match j["verdict"].as_str().expect("verdict") {
                    "healthy" => health::Verdict::Healthy,
                    "unhealthy" => health::Verdict::Unhealthy,
                    "stale" => health::Verdict::Stale,
                    "unobservable" => health::Verdict::Unobservable,
                    "not_asked" => health::Verdict::NotAsked,
                    other => panic!("verdict {other:?}"),
                };
                let level = match &j["level"] {
                    Value::Null => None,
                    v => level_read(v).level(),
                };
                // The reason plays no part in a roll-up.
                Judged {
                    verdict,
                    reason: health::Reason::Ok,
                    level,
                }
            })
            .collect();
        let r = health::rollup(judged);
        let what = format!("{}", case["note"]);
        expect(
            case,
            json!({
                "worst": level_token(r.worst),
                "counts": {
                    "healthy": r.healthy,
                    "unhealthy": r.unhealthy,
                    "stale": r.stale,
                    "unobservable": r.unobservable,
                    "not_asked": r.not_asked,
                },
            }),
            &what,
        );
    }
    if bless() {
        write_json(&path, &doc);
    }
}

/// `health/conformance/codes.json`: a fault's code → profile, application
/// or malformed (health.v1 §2.10).
#[test]
fn health_codes() {
    let path = profiles().join("health/conformance/codes.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let code = case["code"].as_str().expect("code").to_owned();
        expect(
            case,
            json!(health::code(&code).as_str()),
            &format!("{code:?}"),
        );
    }
    if bless() {
        write_json(&path, &doc);
    }
}

/// `freshness/conformance/clock-trust.json`: a reading's offsets of one
/// stamping clock and a delta → whether the reader trusts its clock
/// (freshness.v1 §2.6, ground 2, 0.2).
#[test]
fn freshness_clock_trust() {
    let path = profiles().join("freshness/conformance/clock-trust.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let offsets: Vec<StampAge> = case["offsets_s"]
            .as_array()
            .expect("offsets_s")
            .iter()
            .map(|o| StampAge::from_secs_f64(o.as_f64().expect("seconds")))
            .collect();
        let delta = std::time::Duration::from_secs_f64(case["delta_s"].as_f64().expect("delta_s"));
        let trusted = matches!(
            ClockTrust::from_offsets(offsets, delta),
            ClockTrust::Trusted { .. }
        );
        let what = format!("{}: {}", case["note"], case["offsets_s"]);
        expect(case, json!({ "trusted": trusted }), &what);
    }
    if bless() {
        write_json(&path, &doc);
    }
}
