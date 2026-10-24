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

use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use serde_json::{Value, json};
use zenkey_model::alarms;
use zenkey_model::authoring::Kind;
use zenkey_model::bundle::Bundle;
use zenkey_model::config::{
    self, Call, ConfigStatus, ControlOp, ControlReply, ControlRequest, DesiredConfig, DeviceResult,
    Facts, GroupDecl, LastFacts, Owner, PendingFacts, Refusal, Schema, SetReply, SetRequest, Start,
    Step, Time,
};
use zenkey_model::desired;
use zenkey_model::freshness::{self, ClockTrust, Horizon, Last, Observation, Reply, StampAge};
use zenkey_model::grammar::KindToken;
use zenkey_model::health::{self, Judged, Level, Listing, Presence, Read, Reading};
use zenkey_model::hostid;
use zenkey_model::link;
use zenkey_model::slug::chunk_slug;
use zenkey_model::telemetry::{self, Datum, Declaration, MetricKind, TypeView};
use zenkey_model::template::Template;

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
    ("config", "window.json"),
    ("config", "validation.json"),
    ("config", "desired.json"),
    ("config", "redaction.json"),
    ("config", "lifecycle.json"),
    ("link", "exposure.json"),
    ("link", "plan.json"),
    ("link", "lints.json"),
    ("alarms", "keys.json"),
    ("alarms", "documents.json"),
    ("alarms", "transitions.json"),
    ("alarms", "conclusions.json"),
    ("telemetry", "schema.json"),
    ("telemetry", "declarations.json"),
    ("telemetry", "samples.json"),
    ("telemetry", "continuity.json"),
    ("desired", "schema.json"),
    ("desired", "declarations.json"),
    ("desired", "bindings.json"),
    ("desired", "order.json"),
    ("desired", "reports.json"),
    ("desired", "conclusions.json"),
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
    for standard in ["health/health.v1.toml", "alarms/alarms.v1.toml"] {
        assert!(
            files.iter().any(|p| p.ends_with(standard)),
            "{standard}, a standard contract: {files:?}"
        );
    }
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

// --- config.v1 (#768, CA) -------------------------------------------------------

/// `config.v1`'s standard contract as a bundle: every wire document the
/// model produces below is checked against its own type (core §7.3).
fn config_bundle() -> Bundle {
    let l = zenkey_model::contract::load_path(&profiles().join("config/config.v1.toml"));
    Bundle::build(&l.contract.unwrap_or_else(|| panic!("{}", l.report)))
}

/// Checks `value` against the contract's type of a resource's member.
fn conforms(b: &Bundle, token: &str, template: &str, member: &str, value: &Value, what: &str) {
    let ty = zenkey_model::decode::type_of(b, token, template, member)
        .unwrap_or_else(|| panic!("{token}/{template}: no {member} type"));
    if let Err(v) = zenkey_model::validate::validate(b, ty, value) {
        panic!("{what}: {token}/{template} {member} does not satisfy its type: {v:?}\n{value:#}");
    }
}

fn config_schema(doc: &Value, name: &Value) -> Schema {
    serde_json::from_value(doc["schemas"][name.as_str().expect("schema name")].clone())
        .expect("a schema")
}

fn refusal_json(r: &Refusal) -> Value {
    let mut o =
        json!({"code": r.code.as_str(), "field": r.field.as_ref().map(config::Field::as_str)});
    if let Some(c) = r.cause() {
        o["cause"] = json!(c);
    }
    if let Some(a) = &r.app {
        o["app"] = json!(a.code.as_str());
    }
    json!({ "refused": o })
}

/// `config/conformance/window.json`: `confirm_s`, the floor and the instant
/// received → the window and the deadline (config.v1 §2.8).
#[test]
fn config_window() {
    let path = profiles().join("config/conformance/window.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let confirm = case["confirm_s"].as_u64().expect("confirm_s");
        let floor = case["floor_s"].as_u64().expect("floor_s");
        let received = Time::parse(case["received"].as_str().expect("received"))
            .unwrap_or_else(|| panic!("{}: received is RFC 3339", case["note"]));
        let w = config::window(confirm, floor);
        let d = config::deadline(received, w);
        assert_eq!(
            Time::parse(&d.rfc3339()),
            Some(d),
            "the owner's spelling reads back"
        );
        let what = format!("{}", case["note"]);
        expect(case, json!({"window_s": w, "deadline": d.rfc3339()}), &what);
    }
    if bless() {
        write_json(&path, &doc);
    }
}

/// `config/conformance/validation.json`: one request against a schema and
/// the owner's state → accepted or the refusal (config.v1 §2.5, §2.10,
/// §2.13).
#[test]
fn config_validation() {
    let path = profiles().join("config/conformance/validation.json");
    let mut doc = read_json(&path);
    let schemas = doc.clone();
    for case in doc["cases"].as_array_mut().expect("cases") {
        let schema = config_schema(&schemas, &case["schema"]);
        let st = &case["state"];
        let pending_token = st["pending"]["token"].as_str().map(str::to_owned);
        let last_token = st["last_change"]["token"].as_str().map(str::to_owned);
        let got = {
            let facts = Facts {
                revision: st["revision"].as_u64().expect("revision"),
                pending: pending_token.as_deref().map(|token| PendingFacts {
                    token,
                    applied: st["pending"]["applied"].as_bool().expect("applied"),
                    deadline_passed: st["pending"]["deadline_passed"]
                        .as_bool()
                        .expect("deadline_passed"),
                }),
                last_change: last_token.as_deref().map(|token| LastFacts {
                    token,
                    set_sensitive: st["last_change"]["set_sensitive"]
                        .as_bool()
                        .expect("set_sensitive"),
                }),
                persist: st["persist"].as_bool().expect("persist"),
                in_flight: st["in_flight"].as_bool().expect("in_flight"),
            };
            let op = case["op"].as_str().expect("op");
            // The body as it arrived: its own type is checked first, as
            // the decode does (text 0.3, F-115).
            let r = if op == "set" {
                let group = case["group"].as_str().expect("group");
                config::validate_set_json(&schema, &facts, group, &case["request"]).map(|_| ())
            } else {
                let op = match op {
                    "confirm" => ControlOp::Confirm,
                    "cancel" => ControlOp::Cancel,
                    "extend" => ControlOp::Extend,
                    "persist" => ControlOp::Persist,
                    other => panic!("op {other:?}"),
                };
                config::validate_control_json(op, &facts, &case["request"])
            };
            match r {
                Ok(()) => json!({"accepted": true}),
                Err(r) => refusal_json(&r),
            }
        };
        let what = format!("{}", case["note"]);
        expect(case, got, &what);
    }
    if bless() {
        write_json(&path, &doc);
    }
}

/// `config/conformance/desired.json`: a desired document against a schema →
/// accepted, or refused whole with the first problem (config.v1 §2.18).
#[test]
fn config_desired() {
    let path = profiles().join("config/conformance/desired.json");
    let mut doc = read_json(&path);
    let schemas = doc.clone();
    for case in doc["cases"].as_array_mut().expect("cases") {
        let schema = config_schema(&schemas, &case["schema"]);
        let d: DesiredConfig =
            serde_json::from_value(case["document"].clone()).expect("a DesiredConfig");
        let got = match config::check_desired(&schema, &d) {
            Ok(()) => json!({"accepted": true}),
            Err(r) => json!({ "refused": r }),
        };
        let what = format!("{}", case["note"]);
        expect(case, got, &what);
    }
    if bless() {
        write_json(&path, &doc);
    }
}

type Layer<T> = BTreeMap<String, BTreeMap<String, T>>;

/// Runs a step to its answer: performs each act with the next device
/// result, leaving it in flight on `hold`.
fn drive(owner: &mut Owner, now: Time, mut step: Step, queue: &mut VecDeque<Value>, out: &mut Ran) {
    loop {
        match step {
            Step::Done(d) => {
                if d.reply.is_some() {
                    out.reply = d.reply;
                }
                out.events.extend(d.events);
                return;
            }
            Step::Act(a) => {
                out.acts.push(match a {
                    config::Act::Apply { .. } => "apply",
                    config::Act::Undo { .. } => "undo",
                });
                let r = queue.pop_front().unwrap_or(json!("ok"));
                if r == json!("hold") {
                    return;
                }
                step = owner.finish(now, device_result(&r));
            }
        }
    }
}

/// What one step produced.
#[derive(Default)]
struct Ran {
    acts: Vec<&'static str>,
    events: Vec<config::ConfigChangeEvent>,
    reply: Option<Result<config::Reply, Refusal>>,
}

fn device_result(r: &Value) -> DeviceResult {
    if r == &json!("ok") {
        return DeviceResult::Ok;
    }
    if let Some(w) = r["refused"].as_str() {
        return DeviceResult::Refused(w.to_owned());
    }
    if let Some(w) = r["failed"].as_str() {
        return DeviceResult::Failed {
            words: w.to_owned(),
            readback: serde_json::from_value(r["readback"].clone()).expect("a readback"),
        };
    }
    panic!("device result {r}")
}

/// `config/conformance/redaction.json`: what the view, the status, the
/// event, an envelope and a log line carry of a sensitive value
/// (config.v1 §2.15).
#[test]
fn config_redaction() {
    let path = profiles().join("config/conformance/redaction.json");
    let bundle = config_bundle();
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let what = format!("{}", case["note"]);
        let group: GroupDecl = serde_json::from_value(case["group"].clone()).expect("a group");
        let gname = group.name.clone();
        let schema = Schema {
            groups: vec![group],
        };
        let mut startup: Layer<(config::Value, config::Source)> = BTreeMap::new();
        for (p, h) in case["running"].as_object().expect("running") {
            let v = serde_json::from_value(h["value"].clone()).expect("a value");
            let s = serde_json::from_value(h["source"].clone()).expect("a source");
            assert_eq!(
                case["startup"][p], h["value"],
                "{what}: the cases run from their startup values"
            );
            startup
                .entry(gname.clone())
                .or_default()
                .insert(p.clone(), (v, s));
        }
        let fresh = || {
            Owner::start(Start {
                service: "lab/svc".into(),
                instance: "i1".into(),
                schema: schema.clone(),
                startup: startup.clone(),
                overlay: BTreeMap::new(),
                floor_s: 10,
                apply_after: Duration::from_secs(1),
                persist: true,
                events: Vec::new(),
            })
            .expect("a valid start")
        };
        let req: SetRequest =
            serde_json::from_value(case["request"].clone()).expect("a SetRequest");
        let t0 = Time::from_millis(0);
        let mut owner = fresh();
        let mut ran = Ran::default();
        let step = owner.set(t0, &gname, &req, &Call::default());
        drive(&mut owner, t0, step, &mut VecDeque::new(), &mut ran);
        let got = match ran.reply.expect("a set is answered") {
            Err(r) => {
                let repeats = req.values.values().any(|v| {
                    r.message.contains(&v.to_string())
                        || v.as_str().is_some_and(|s| r.message.contains(s))
                });
                json!({"refused": {
                    "code": r.code.as_str(),
                    "field": r.field.as_ref().map(config::Field::as_str),
                    "message_repeats_value": repeats,
                }})
            }
            Ok(config::Reply::Set(SetReply::Applied(view))) => {
                let vj = serde_json::to_value(&*view).unwrap();
                conforms(&bundle, "state", "view", "type", &vj, &what);
                let g = view
                    .groups
                    .iter()
                    .find(|g| g.name == gname)
                    .expect("the group");
                let device_detail = match case["device_words"].as_str() {
                    None => Value::Null,
                    Some(words) => {
                        let mut o = fresh();
                        let s = o.set(t0, &gname, &req, &Call::default());
                        assert!(matches!(s, Step::Act(_)), "{what}: the device is asked");
                        match o.finish(t0, DeviceResult::Refused(words.into())) {
                            Step::Done(config::Done {
                                reply: Some(Err(r)),
                                ..
                            }) => json!(r.app.and_then(|a| a.detail)),
                            other => panic!("{what}: {other:?}"),
                        }
                    }
                };
                json!({
                    "view": g.parameters,
                    "status": ConfigStatus::of(&view).values.get(&gname),
                    "edits": ran.events.first().map(|e| &e.edits),
                    "log": config::log_request(&schema, &gname, &req),
                    "device_detail": device_detail,
                })
            }
            Ok(other) => panic!("{what}: {other:?}"),
        };
        expect(case, got, &what);
    }
    if bless() {
        write_json(&path, &doc);
    }
}

fn reply_json(r: &Option<Result<config::Reply, Refusal>>) -> Value {
    match r {
        None => Value::Null,
        Some(Err(r)) => refusal_json(r),
        Some(Ok(config::Reply::Set(SetReply::Applied(v)))) => {
            json!({"applied": ConfigStatus::of(v)})
        }
        Some(Ok(config::Reply::Set(SetReply::Pending(p)))) => json!({ "pending": p }),
        Some(Ok(config::Reply::Control(ControlReply::View(v)))) => {
            json!({"view": ConfigStatus::of(v)})
        }
        Some(Ok(config::Reply::Control(ControlReply::Status(s)))) => json!({ "status": s }),
    }
}

fn config_state(owner: &Owner) -> Value {
    let view = owner.view();
    let mut params = serde_json::Map::new();
    for g in &view.groups {
        let mut ps = serde_json::Map::new();
        for p in &g.parameters {
            let mut o = serde_json::Map::new();
            if let Some(s) = p.source {
                o.insert("source".into(), json!(s));
            }
            if let Some(s) = &p.startup {
                o.insert("startup".into(), json!(s));
            }
            ps.insert(p.name.clone(), Value::Object(o));
        }
        params.insert(g.name.clone(), Value::Object(ps));
    }
    let writable: Vec<&str> = view
        .groups
        .iter()
        .filter(|g| g.writable)
        .map(|g| g.name.as_str())
        .collect();
    json!({"status": owner.status(), "params": params, "writable": writable})
}

fn lifecycle_owner(schemas: &Value, su: &Value) -> Owner {
    let startup: Layer<(config::Value, config::Source)> =
        serde_json::from_value::<Layer<serde_json::Map<String, Value>>>(su["startup"].clone())
            .expect("startup")
            .into_iter()
            .map(|(g, ps)| {
                let ps = ps
                    .into_iter()
                    .map(|(p, h)| {
                        let v = serde_json::from_value(h["value"].clone()).expect("a value");
                        let s = serde_json::from_value(h["source"].clone()).expect("a source");
                        (p, (v, s))
                    })
                    .collect();
                (g, ps)
            })
            .collect();
    let mut events = Vec::new();
    for e in su["events_at"].as_array().expect("events_at") {
        let t = Time::from_secs_f64(e["t"].as_f64().expect("t"));
        for _ in 0..e["count"].as_u64().expect("count") {
            events.push(t);
        }
    }
    Owner::start(Start {
        service: su["service"].as_str().expect("service").into(),
        instance: su["instance"].as_str().expect("instance").into(),
        schema: config_schema(schemas, &su["schema"]),
        startup,
        overlay: serde_json::from_value(su["overlay"].clone()).expect("overlay"),
        floor_s: su["floor_s"].as_u64().expect("floor_s"),
        apply_after: Duration::from_millis(su["apply_after_ms"].as_u64().expect("apply_after_ms")),
        persist: su["persist"].as_bool().expect("persist"),
        events,
    })
    .expect("a valid setup")
}

/// Runs one lifecycle step on `owner`.
fn lifecycle_step(owner: &mut Owner, step: &Value, now: Time) -> Ran {
    let mut queue: VecDeque<Value> = step["device"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into();
    let call = Call {
        actor: step["call"]["actor"].as_str().map(str::to_owned),
        request_id: step["call"]["request_id"].as_str().map(str::to_owned),
    };
    let control = || -> ControlRequest {
        serde_json::from_value(step["request"].clone()).expect("a ControlRequest")
    };
    let mut ran = Ran::default();
    match step["do"].as_str().expect("do") {
        "set" => {
            let req: SetRequest =
                serde_json::from_value(step["request"].clone()).expect("a SetRequest");
            let s = owner.set(now, step["group"].as_str().expect("group"), &req, &call);
            drive(owner, now, s, &mut queue, &mut ran);
        }
        "confirm" => {
            let s = owner.confirm(now, &control(), &call);
            drive(owner, now, s, &mut queue, &mut ran);
        }
        "cancel" => {
            let s = owner.cancel(now, &control(), &call);
            drive(owner, now, s, &mut queue, &mut ran);
        }
        "extend" => {
            let s = owner.extend(now, &control(), &call);
            drive(owner, now, s, &mut queue, &mut ran);
        }
        "persist" => match owner.persist_plan(now, &control()) {
            Ok(plan) => ran.reply = owner.persisted(plan).reply,
            Err(r) => ran.reply = Some(Err(r)),
        },
        "due" => {
            while let Some(s) = owner.due(now) {
                drive(owner, now, s, &mut queue, &mut ran);
                if owner.in_flight() {
                    break;
                }
            }
        }
        "complete" => {
            let r = queue.pop_front().unwrap_or(json!("ok"));
            let s = owner.finish(now, device_result(&r));
            drive(owner, now, s, &mut queue, &mut ran);
        }
        "restart" => {
            *owner = owner.restart(step["instance"].as_str().expect("instance"));
        }
        other => panic!("do {other:?}"),
    }
    ran
}

/// `config/conformance/lifecycle.json`: sequences of calls, device results
/// and instants on one owner → its replies, acts, events and state
/// (config.v1 §2.3, §2.6–§2.14, §2.17, §2.19).
#[test]
fn config_lifecycle() {
    let path = profiles().join("config/conformance/lifecycle.json");
    let bundle = config_bundle();
    let mut doc = read_json(&path);
    let schemas = doc.clone();
    for case in doc["cases"].as_array_mut().expect("cases") {
        let note = case["note"].as_str().expect("note").to_owned();
        let mut owner = lifecycle_owner(&schemas, &case["setup"]);
        let start = config_state(&owner);
        if bless() {
            case["start"] = start;
        } else {
            assert_eq!(case["start"], start, "{note}: the state before any step");
        }
        for (i, step) in case["steps"]
            .as_array_mut()
            .expect("steps")
            .iter_mut()
            .enumerate()
        {
            let what = format!("{note}, step {i} ({} at t={})", step["do"], step["t"]);
            let now = Time::from_secs_f64(step["t"].as_f64().expect("t"));
            let ran = lifecycle_step(&mut owner, step, now);
            // Every document the owner puts or sends is its contract's type.
            let view = serde_json::to_value(owner.view()).unwrap();
            conforms(&bundle, "state", "view", "type", &view, &what);
            let status = serde_json::to_value(owner.status()).unwrap();
            conforms(&bundle, "state", "status", "type", &status, &what);
            for e in &ran.events {
                let e = serde_json::to_value(e).unwrap();
                conforms(&bundle, "events", "change", "type", &e, &what);
            }
            match &ran.reply {
                Some(Ok(r @ config::Reply::Set(_))) => {
                    conforms(
                        &bundle,
                        "@op",
                        "set/{group}",
                        "response",
                        &r.to_json(),
                        &what,
                    );
                }
                Some(Ok(r @ config::Reply::Control(_))) => {
                    conforms(&bundle, "@op", "confirm", "response", &r.to_json(), &what);
                }
                Some(Err(Refusal { app: Some(a), .. })) => {
                    let a = serde_json::to_value(a).unwrap();
                    conforms(&bundle, "@op", "set/{group}", "error", &a, &what);
                }
                _ => {}
            }
            let got = json!({
                "reply": reply_json(&ran.reply),
                "acts": ran.acts,
                "events": ran.events,
                "state": config_state(&owner),
            });
            expect(step, got, &what);
        }
    }
    if bless() {
        write_json(&path, &doc);
    }
}

/// The two types no fixture puts on the wire, checked against the contract:
/// a view and a status carrying a desired report (config.v1 §2.18), and a
/// `DesiredConfig`, which no resource of `config.v1` carries.
#[test]
fn config_desired_types_are_the_contracts() {
    let bundle = config_bundle();
    let doc = read_json(&profiles().join("config/conformance/lifecycle.json"));
    let mut owner = lifecycle_owner(&doc, &doc["cases"][0]["setup"]);
    owner.report_desired(Some(config::DesiredReport {
        observed_revision: 2,
        outcome: config::DesiredOutcome::Refused,
        refusal: Some(config::DesiredRefusal {
            code: config::DesiredCode::Sensitive,
            group: "sim".into(),
            parameter: Some("pin".into()),
        }),
    }));
    let view = serde_json::to_value(owner.view()).unwrap();
    conforms(&bundle, "state", "view", "type", &view, "a desired report");
    assert_eq!(
        view["desired"],
        json!({"observed_revision": 2, "outcome": "refused",
               "refusal": {"code": "sensitive", "group": "sim", "parameter": "pin"}})
    );
    let status = serde_json::to_value(owner.status()).unwrap();
    conforms(
        &bundle,
        "state",
        "status",
        "type",
        &status,
        "a desired report",
    );
    let mut ty = zenkey_model::decode::type_of(&bundle, "state", "view", "type")
        .expect("the view's type")
        .clone();
    ty["name"] = json!("DesiredConfig");
    let d = config::DesiredConfig {
        revision: 7,
        values: [(
            "duty".to_owned(),
            [("fraction_ppm".to_owned(), json!(20000))].into(),
        )]
        .into(),
    };
    let dj = serde_json::to_value(&d).unwrap();
    assert!(
        zenkey_model::validate::validate(&bundle, &ty, &dj).is_ok(),
        "{dj}"
    );
    assert!(config::check_desired(owner.schema(), &d).is_ok());
}

// --- link.v1 (#769, LA) ---------------------------------------------------------

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn annotations(v: &Value) -> BTreeMap<String, Value> {
    v.as_object()
        .expect("annotations")
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn link_token(v: &Value) -> KindToken {
    v.as_str()
        .expect("token")
        .parse()
        .unwrap_or_else(|e| panic!("token {v}: {e}"))
}

fn findings_json(f: &[link::Finding]) -> Value {
    serde_json::to_value(f).unwrap()
}

/// One revision's resources, as `link/conformance/` writes them.
fn link_resources(rs: &Value) -> Vec<link::Res> {
    rs.as_array()
        .expect("resources")
        .iter()
        .map(|r| link::Res {
            template: Template::parse(r["template"].as_str().expect("template"))
                .expect("a template"),
            token: link_token(&r["token"]),
            annotations: annotations(&r["annotations"]),
        })
        .collect()
}

/// A case's deployment, as `link/conformance/` writes it: `implements`, and
/// the interfaces inline or loaded from `contracts`. An interface inline is
/// one revision's resources, or `{revisions: [[...], ...]}` (0.3, F-119);
/// two contracts of one interface id are two revisions of it.
fn link_deployment(case: &Value) -> Option<link::Deployment> {
    let implements = case.get("implements")?;
    let mut d = link::Deployment {
        implements: serde_json::from_value(implements.clone()).expect("implements"),
        ..link::Deployment::default()
    };
    for p in case
        .get("contracts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let path = repo_root().join(p.as_str().expect("a path"));
        let l = zenkey_model::contract::load_path(&path);
        let c = l
            .contract
            .unwrap_or_else(|| panic!("{}: {}", path.display(), l.report));
        d.add_contract(&c);
    }
    for (iface, rs) in case
        .get("interfaces")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        match rs.get("revisions") {
            Some(revs) => {
                for rev in revs.as_array().expect("revisions") {
                    d.add_revision(iface.clone(), link_resources(rev));
                }
            }
            None => d.add_revision(iface.clone(), link_resources(rs)),
        }
    }
    Some(d)
}

/// `link/conformance/exposure.json`: one resource and the face's `expose` →
/// its exposure and whether its pushes cross (link.v1 §2.2, §2.7).
#[test]
fn link_exposure() {
    let path = profiles().join("link/conformance/exposure.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let token = link_token(&case["token"]);
        let template =
            Template::parse(case["template"].as_str().expect("template")).expect("a template");
        let e = link::exposure(link::kind_of(token), &annotations(&case["annotations"]));
        let expose = match case["expose"].as_str().expect("expose") {
            "link" => link::Expose::Link,
            "none" => link::Expose::None,
            other => panic!("expose {other:?}"),
        };
        let c = link::crossing(token, template.has_params(), &e, expose);
        let what = format!("{}", case["note"]);
        expect(
            case,
            json!({"exposure": e.as_str(), "pushes": c.pushes.as_str(), "reason": c.reason.as_str()}),
            &what,
        );
    }
    if bless() {
        write_json(&path, &doc);
    }
}

/// A plan as a router's block, which [`link::check_block`] must pass: every
/// plan has the whole `@zk` deny and concrete rules that do not intersect
/// (link.v1 §2.6, §2.8), and a generator writes it enabled (§2.15, 0.7).
fn plan_as_block(p: &link::FacePlan) -> link::Block {
    link::Block {
        namespace: String::new(),
        enabled: Some(true),
        deny: p
            .deny
            .iter()
            .map(|d| link::BlockDeny {
                flows: d.flows.iter().map(|s| (*s).to_owned()).collect(),
                messages: d.messages.iter().map(|s| (*s).to_owned()).collect(),
                key_exprs: d.key_exprs.clone(),
            })
            .collect(),
        downsampling: p
            .downsampling
            .iter()
            .flat_map(|d| &d.rules)
            .map(|r| link::BlockRule {
                key_expr: r.key_expr.clone(),
                freq: r.freq,
            })
            .collect(),
        subjects: None,
    }
}

/// `link/conformance/plan.json`: a face policy and a deployment → each
/// face's plan, or the findings that refuse it (link.v1 §2.3, §2.5–§2.12).
#[test]
fn link_plan() {
    let path = profiles().join("link/conformance/plan.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let what = format!("{}", case["note"]);
        let parsed = link::parse(case["policy"].as_str().expect("policy"));
        let deployment = link_deployment(case).expect("implements");
        let got = match parsed.policy {
            None => json!({"refused": findings_json(&parsed.findings)}),
            Some(policy) => match link::plan(&policy, &deployment) {
                Err(f) => json!({"refused": findings_json(&f)}),
                Ok(plans) => {
                    for p in &plans {
                        for d in &p.deny {
                            for k in &d.key_exprs {
                                assert!(
                                    zenoh_keyexpr::keyexpr::new(k.as_str()).is_ok(),
                                    "{what}: {k} is a canonical key expression"
                                );
                            }
                        }
                        let found = link::check_block(&p.name, &plan_as_block(p));
                        assert!(
                            found.is_empty(),
                            "{what}: the plan fails its own checks: {found:?}"
                        );
                    }
                    json!({"faces": serde_json::to_value(&plans).unwrap()})
                }
            },
        };
        expect(case, got, &what);
    }
    if bless() {
        write_json(&path, &doc);
    }
}

/// A case's router block, as `lints.json` writes it.
fn link_block(b: &Value) -> (String, link::Block) {
    let strings = |v: &Value| -> Vec<String> {
        v.as_array()
            .expect("strings")
            .iter()
            .map(|s| s.as_str().expect("a string").to_owned())
            .collect()
    };
    let block = link::Block {
        namespace: b["namespace"].as_str().expect("namespace").to_owned(),
        // Optional (0.7, F-124): `false` is the finding, and absent was not
        // read.
        enabled: b.get("enabled").map(|e| e.as_bool().expect("enabled")),
        deny: b["deny"]
            .as_array()
            .expect("deny")
            .iter()
            .map(|d| link::BlockDeny {
                flows: strings(&d["flows"]),
                messages: strings(&d["messages"]),
                key_exprs: strings(&d["key_exprs"]),
            })
            .collect(),
        downsampling: b["downsampling"]
            .as_array()
            .expect("downsampling")
            .iter()
            .map(|r| link::BlockRule {
                key_expr: r["key_expr"].as_str().expect("key_expr").to_owned(),
                freq: r["freq"].as_f64().expect("freq"),
            })
            .collect(),
        // `default_permission`, `subjects` and `policies`, all three or none
        // (0.3; `policies` since 0.4, F-120).
        subjects: match (
            b.get("default_permission"),
            b.get("subjects"),
            b.get("policies"),
        ) {
            (None, None, None) => None,
            (Some(p), Some(subjects), Some(policies)) => Some(link::BlockSubjects {
                posture: match p.as_str().expect("default_permission") {
                    "deny" => link::Posture::Deny,
                    "allow" => link::Posture::Allow,
                    other => panic!("default_permission {other:?}"),
                },
                subjects: subjects
                    .as_array()
                    .expect("subjects")
                    .iter()
                    .map(|s| {
                        let list = |k: &str| s.get(k).map(&strings).unwrap_or_default();
                        link::BlockSubject {
                            id: s["id"].as_str().expect("a subject's id").to_owned(),
                            usernames: list("usernames"),
                            cert_common_names: list("cert_common_names"),
                            link_protocols: s.get("link_protocols").map(&strings),
                            interfaces: list("interfaces"),
                            zids: list("zids"),
                        }
                    })
                    .collect(),
                policies: policies
                    .as_array()
                    .expect("policies")
                    .iter()
                    .map(|p| link::BlockPolicy {
                        subjects: strings(&p["subjects"]),
                    })
                    .collect(),
            }),
            _ => panic!("block: default_permission, subjects and policies go together"),
        },
    };
    (b["face"].as_str().expect("block.face").to_owned(), block)
}

/// `link/conformance/lints.json`: a face policy, and what comes with it →
/// every finding (link.v1 §2.1, §2.3, §2.11, §2.15).
#[test]
fn link_lints() {
    let path = profiles().join("link/conformance/lints.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let what = format!("{}", case["note"]);
        let deployment = link_deployment(case);
        let windows: Option<BTreeMap<String, u64>> = case
            .get("windows")
            .map(|w| serde_json::from_value(w.clone()).expect("windows"));
        let block = case.get("block").map(link_block);
        let found = link::lint(&link::LintInput {
            text: case["policy"].as_str().expect("policy"),
            deployment: deployment.as_ref(),
            windows: windows.as_ref(),
            block: block.as_ref().map(|(f, b)| (f.as_str(), b)),
        });
        expect(case, findings_json(&found), &what);
    }
    if bless() {
        write_json(&path, &doc);
    }
}

// --- alarms.v1 (#800, AA) -------------------------------------------------------

/// `alarms.v1`'s standard contract as a bundle: every document of
/// `documents.json` is checked against its own type too (core §7.3).
fn alarms_bundle() -> Bundle {
    let l = zenkey_model::contract::load_path(&profiles().join("alarms/alarms.v1.toml"));
    Bundle::build(&l.contract.unwrap_or_else(|| panic!("{}", l.report)))
}

fn labels(v: &Value) -> Vec<(&str, &str)> {
    v.as_object()
        .expect("labels")
        .iter()
        .map(|(n, v)| (n.as_str(), v.as_str().expect("a string value")))
        .collect()
}

/// `alarms/conformance/keys.json`: a rule and its labels → the alert key a
/// reader derives, its input and whether it is framed; a label name →
/// whether it is host-scoped; a reference → its key, and back (alarms.v1
/// §2.2, §2.3, §2.12).
#[test]
fn alarms_keys() {
    let path = profiles().join("alarms/conformance/keys.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let what = format!("{}: {}", case["op"], case["note"]);
        let got = match case["op"].as_str().expect("op") {
            "derive" => {
                let rule = case["rule"].as_str().expect("rule");
                let labels = labels(&case["labels"]);
                let d = alarms::derive(rule, labels.iter().copied());
                let strict = alarms::alert_key(rule, labels.iter().copied());
                // On framed input the strict recipe and the reader's agree.
                assert_eq!(strict.is_ok(), d.framed, "{what}");
                if let Ok(k) = &strict {
                    assert_eq!(*k, d.key, "{what}");
                }
                assert!(
                    d.key.len() == 16
                        && d.key
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                    "{what}: {}",
                    d.key
                );
                assert_eq!(chunk_slug(&d.key), d.key, "{what}: its own slug");
                json!({"key": d.key, "input": d.input, "framed": d.framed})
            }
            "host_scoped" => json!(alarms::is_host_scoped(case["name"].as_str().expect("name"))),
            "ref" => {
                let r: alarms::AlarmRef = serde_json::from_value(case["ref"].clone()).expect("ref");
                match r.key() {
                    Ok(k) => {
                        assert_eq!(
                            alarms::AlarmRef::from_key(k.as_str()).as_ref(),
                            Some(&r),
                            "{what}: back"
                        );
                        json!(k.as_str())
                    }
                    Err(_) => Value::Null,
                }
            }
            "parse" => json!(alarms::AlarmRef::from_key(
                case["key"].as_str().expect("key")
            )),
            other => panic!("op {other:?}"),
        };
        expect(case, got, &what);
    }
    if bless() {
        write_json(&path, &doc);
    }
}

/// `alarms/conformance/documents.json`: a payload → whether it reads, and
/// the owner's errors and warnings (alarms.v1 §2.4, §2.3). The model's
/// reading of the schema agrees with the schema's own validation. A case
/// with `text` gives the payload as its text, read as strict JSON first
/// (text 0.2, F-126).
#[test]
fn alarms_documents() {
    let path = profiles().join("alarms/conformance/documents.json");
    let b = alarms_bundle();
    let ty = zenkey_model::decode::type_of(&b, "state", alarms::ALARMS, "type").expect("its type");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let what = format!("{}", case["note"]);
        let read = if let Some(text) = case.get("text") {
            alarms::decode(text.as_str().expect("text").as_bytes())
        } else {
            let v = &case["document"];
            let read = alarms::Alarm::from_json(v);
            let schema_ok = zenkey_model::validate::validate(&b, ty, v).is_ok();
            assert_eq!(
                schema_ok,
                !matches!(read, Err(alarms::Invalid::Schema(_))),
                "{what}: the model reads the schema as validate does: {read:?}"
            );
            read
        };
        let got = match &read {
            Err(e) => json!({"readable": false, "errors": [e.as_str()], "warnings": []}),
            Ok(a) => {
                let p = a.check();
                let of = |g| {
                    p.iter()
                        .filter(|x| x.gravity() == g)
                        .map(alarms::Problem::as_str)
                        .collect::<Vec<_>>()
                };
                // A readable document serializes back into its type.
                let back = serde_json::to_value(a).expect("serializes");
                assert!(
                    zenkey_model::validate::validate(&b, ty, &back).is_ok(),
                    "{what}: {back}"
                );
                json!({
                    "readable": true,
                    "errors": of(alarms::Gravity::Error),
                    "warnings": of(alarms::Gravity::Warning),
                })
            }
        };
        expect(case, got, &what);
    }
    if bless() {
        write_json(&path, &doc);
    }
}

/// `alarms/conformance/transitions.json`: one key's samples in the owner's
/// stamp order → the transition each makes (alarms.v1 §2.6).
#[test]
fn alarms_transitions() {
    let path = profiles().join("alarms/conformance/transitions.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let what = format!("{}", case["note"]);
        let mut track = alarms::Track::default();
        let got: Vec<Value> = case["samples"]
            .as_array()
            .expect("samples")
            .iter()
            .map(|s| {
                let t = if s == "delete" {
                    track.apply(alarms::Sample::Delete)
                } else {
                    match alarms::Alarm::from_json(&s["put"]) {
                        Ok(a) => track.apply(alarms::Sample::Put(&a)),
                        Err(_) => track.apply(alarms::Sample::Unreadable),
                    }
                };
                json!(t.as_str())
            })
            .collect();
        expect(case, json!(got), &what);
    }
    if bless() {
        write_json(&path, &doc);
    }
}

/// A case's presence, descriptor and face, as `alarms/conformance/` writes
/// them.
fn alarms_presence(case: &Value) -> alarms::Presence {
    match case["presence"].as_str().expect("presence") {
        "present" => alarms::Presence::Present(match &case["descriptor"] {
            Value::Null => alarms::Listing::Unread,
            d if d["lists"] == json!(false) => alarms::Listing::NotListed,
            d => alarms::Listing::Listed {
                token: d["token"].as_bool().expect("descriptor.token"),
            },
        }),
        "absent" => alarms::Presence::Absent,
        "incomplete" => alarms::Presence::Incomplete,
        "across_face" => alarms::Presence::AcrossFace {
            reachable: case["face"]["reachable"].as_bool().expect("face.reachable"),
        },
        other => panic!("presence {other:?}"),
    }
}

fn alarms_judged(j: alarms::Judged) -> Value {
    json!({"answer": j.answer.as_str(), "reason": j.reason.as_str()})
}

fn alarms_answer(v: &Value) -> alarms::Answer {
    match v.as_str().expect("answer") {
        "yes" => alarms::Answer::Yes,
        "no" => alarms::Answer::No,
        "unobservable" => alarms::Answer::Unobservable,
        "not_asked" => alarms::Answer::NotAsked,
        other => panic!("answer {other:?}"),
    }
}

/// One alarm of a scope, as `any_firing` reads it: an answer and a
/// severity.
type AlarmRead = (alarms::Judged, Option<alarms::Severity>);

fn alarms_severity(v: &Value) -> Option<alarms::Severity> {
    v.as_str()
        .map(|s| alarms::Severity::parse(s).unwrap_or_else(|| panic!("severity {s:?}")))
}

/// `alarms/conformance/conclusions.json`: §5's questions, from one reading,
/// an owner's GET and window, a scope, or a key and its payload (alarms.v1
/// §2.14, §2.15, §5).
#[test]
fn alarms_conclusions() {
    let path = profiles().join("alarms/conformance/conclusions.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let what = format!("{}: {}", case["question"], case["note"]);
        let got = match case["question"].as_str().expect("question") {
            "reading" => {
                let observations: Vec<Observation> = case["observations"]
                    .as_array()
                    .expect("observations")
                    .iter()
                    .map(observation)
                    .collect();
                let latest = match &case["latest"] {
                    Value::Null => None,
                    l => Some(match l.as_str().expect("latest") {
                        "firing" => alarms::Latest::Firing,
                        "resolved" => alarms::Latest::Resolved,
                        "deleted" => alarms::Latest::Deleted,
                        "undecodable" => alarms::Latest::Undecodable,
                        other => panic!("latest {other:?}"),
                    }),
                };
                let r = alarms::Reading {
                    presence: alarms_presence(case),
                    observations: &observations,
                    latest,
                    previous_instance: case["previous_instance"]
                        .as_bool()
                        .expect("previous_instance"),
                    held_whole: case["held_whole"].as_bool().expect("held_whole"),
                    left_out: case["left_out"].as_bool().expect("left_out"),
                };
                json!({
                    "firing": alarms_judged(alarms::firing(&r)),
                    "stale_firing": alarms_judged(alarms::stale_firing(&r)),
                })
            }
            "owner_set" => {
                let seed = match &case["seed"] {
                    Value::Null => None,
                    s if s["complete"] == json!(true) => Some(alarms::Seed::Complete {
                        members: usize::try_from(s["members"].as_u64().expect("members"))
                            .expect("fits"),
                    }),
                    _ => Some(alarms::Seed::Incomplete),
                };
                let window = match &case["window"] {
                    Value::Null => None,
                    w => Some(alarms::Window {
                        listened: secs(&w["listened_s"]),
                        lossless: w["lossless"].as_bool().expect("lossless"),
                        present_throughout: w["present_throughout"]
                            .as_bool()
                            .expect("present_throughout"),
                    }),
                };
                let s = alarms::owner_set(alarms_presence(case), seed, window);
                json!({"verdict": s.verdict.as_str(), "reason": s.reason.as_str()})
            }
            "any_firing" => {
                let at_least = alarms_severity(&case["at_least"]).expect("at_least");
                let owners: Vec<(alarms::SetJudged, Vec<AlarmRead>)> = case["owners"]
                    .as_array()
                    .expect("owners")
                    .iter()
                    .map(|o| {
                        let verdict = match o["set"]["verdict"].as_str().expect("verdict") {
                            "established" => alarms::SetVerdict::Established,
                            "unobservable" => alarms::SetVerdict::Unobservable,
                            "not_asked" => alarms::SetVerdict::NotAsked,
                            other => panic!("verdict {other:?}"),
                        };
                        // The set's reason plays no part in the answer.
                        let set = alarms::SetJudged {
                            verdict,
                            reason: alarms::Reason::Answered,
                        };
                        let alarms_read = o["alarms"]
                            .as_array()
                            .expect("alarms")
                            .iter()
                            .map(|a| {
                                (
                                    alarms::Judged {
                                        answer: alarms_answer(&a["answer"]),
                                        // Nor does an alarm's.
                                        reason: alarms::Reason::Firing,
                                    },
                                    alarms_severity(&a["severity"]),
                                )
                            })
                            .collect();
                        (set, alarms_read)
                    })
                    .collect();
                let scope: Vec<alarms::OwnerAlarms<'_>> = owners
                    .iter()
                    .map(|(set, a)| alarms::OwnerAlarms {
                        set: *set,
                        alarms: a,
                    })
                    .collect();
                let read = if case["scope_whole"].as_bool().expect("scope_whole") {
                    alarms::ScopeRead::Whole
                } else {
                    alarms::ScopeRead::NotWhole
                };
                let a = alarms::any_firing(at_least, &scope, read);
                json!({
                    "answer": a.answer.as_str(),
                    "reason": a.reason.as_str(),
                    "firing": a.firing,
                    "owners": {
                        "established": a.established,
                        "unobservable": a.unobservable,
                        "not_asked": a.not_asked,
                    },
                })
            }
            "rederives" => {
                let key = case["alert_key"].as_str().expect("alert_key");
                let read = (case["payload"] != "delete")
                    .then(|| alarms::Alarm::from_json(&case["payload"]));
                let payload = match &read {
                    None => alarms::Payload::Deleted,
                    Some(Ok(a)) => alarms::Payload::Alarm(a),
                    Some(Err(_)) => alarms::Payload::Unreadable,
                };
                alarms_judged(alarms::rederives(key, payload))
            }
            other => panic!("question {other:?}"),
        };
        expect(case, got, &what);
    }
    if bless() {
        write_json(&path, &doc);
    }
}

// --- telemetry.v1 (#819, TA) ------------------------------------------------------

/// The JCS id of a JSON document (core §9.4).
fn jcs_id(v: &Value) -> String {
    zenkey_model::schema::sha256_id(&zenkey_model::schema::jcs(v))
}

/// `telemetry/conformance/schema.json`: the standard schema's published ids
/// (telemetry.v1 §2.3, §3). The profile's file is the current revision, the
/// published list is the one `telemetry::PUBLISHED` holds, every type is
/// defined where it says, and each case's document has the id it says.
#[test]
fn telemetry_schema() {
    let path = profiles().join("telemetry/conformance/schema.json");
    let mut doc = read_json(&path);
    let file = read_json(
        &profiles()
            .join("telemetry")
            .join(doc["file"].as_str().expect("file")),
    );
    assert_eq!(
        json!(jcs_id(&file)),
        doc["current"],
        "the profile's file is the current revision; after an intended change, publish its id in v1.md §3, telemetry::PUBLISHED and this fixture"
    );
    let published: Vec<&str> = doc["published"]
        .as_array()
        .expect("published")
        .iter()
        .map(|p| p["id"].as_str().expect("id"))
        .collect();
    assert_eq!(published, telemetry::PUBLISHED, "telemetry::PUBLISHED");
    assert_eq!(
        published.last(),
        doc["current"].as_str().as_ref(),
        "the current id is the latest"
    );
    let defs = file["$defs"].as_object().expect("$defs");
    let types: Vec<&str> = doc["types"]
        .as_array()
        .expect("types")
        .iter()
        .map(|t| t["name"].as_str().expect("name"))
        .collect();
    assert_eq!(
        types,
        [
            telemetry::POINT,
            telemetry::VALUE,
            telemetry::HISTOGRAM,
            telemetry::EPOCH
        ]
    );
    for t in &types {
        assert!(defs.contains_key(*t), "{t} is defined in the current file");
    }
    let root = repo_root();
    for case in doc["cases"].as_array_mut().expect("cases") {
        let document = if let Some(p) = case["path"].as_str() {
            read_json(&root.join(p))
        } else if let Some(p) = case["bundle"].as_str() {
            let bytes = std::fs::read(root.join(p)).unwrap_or_else(|e| panic!("{p}: {e}"));
            let b = Bundle::verify(&bytes).unwrap_or_else(|e| panic!("{p}: {e:?}"));
            let id = b.contract["schemas"]
                .as_array()
                .expect("schemas")
                .iter()
                .find(|s| s["name"] == telemetry::SCHEMA_STEM)
                .and_then(|s| s["id"].as_str())
                .unwrap_or_else(|| panic!("{p}: no artifact named telemetry.v1"))
                .to_owned();
            b.schemas[&id]["data"].clone()
        } else {
            case["document"].clone()
        };
        let id = jcs_id(&document);
        let got = json!({"id": id, "published": telemetry::is_published(&id)});
        let what = format!("{}", case["note"]);
        expect(case, got, &what);
    }
    if bless() {
        write_json(&path, &doc);
    }
}

/// A type as `telemetry/conformance/` writes it.
fn telemetry_type(v: &Value) -> Option<TypeView<'_>> {
    match v["kind"].as_str()? {
        "jsonschema" => Some(TypeView::Json {
            name: v["name"].as_str().expect("name"),
            schema: v["schema"].as_str().expect("schema"),
            root: v.get("root").filter(|r| !r.is_null()),
        }),
        "raw" => Some(TypeView::Raw),
        "protobuf" => Some(TypeView::Protobuf),
        other => panic!("type kind {other:?}"),
    }
}

/// A resolved resource as `telemetry/conformance/` writes it → its
/// declaration: its annotations given, or merged from its layers (core
/// §9.3).
fn telemetry_declaration(r: &Value) -> Declaration {
    let kind = match r["kind"].as_str().expect("kind") {
        "stream" => Kind::Stream,
        "state" => Kind::State,
        "event" => Kind::Event,
        "operation" => Kind::Operation,
        other => panic!("kind {other:?}"),
    };
    let annotations = match r.get("layers") {
        Some(l) => {
            let mut m = BTreeMap::new();
            for layer in ["defaults", "kind_defaults", "resource"] {
                m.extend(annotations(&l[layer]));
            }
            m
        }
        None => annotations(&r["annotations"]),
    };
    let ty = telemetry_type(&r["type"]);
    let att = telemetry_type(&r["attachment"]);
    telemetry::declaration(kind, &annotations, ty.as_ref(), att.as_ref())
}

fn telemetry_declaration_json(d: &Declaration) -> Value {
    match d {
        Declaration::Declared(x) => json!({
            "declaration": d.as_str(),
            "carrier": x.carrier.as_str(),
            "kind": x.kind.map(MetricKind::as_str),
            "unit": x.unit,
            "buckets": x.buckets,
            "semantic": x.semantic.map(telemetry::Semantic::as_str),
            "epoch": x.epoch.map(telemetry::EpochIn::as_str),
        }),
        Declaration::Invalid(r) => json!({
            "declaration": d.as_str(),
            "reasons": r.iter().map(|r| r.as_str()).collect::<Vec<_>>(),
        }),
        Declaration::Ignored | Declaration::NotThisProfiles => json!({"declaration": d.as_str()}),
    }
}

/// `telemetry/conformance/declarations.json`: a resolved resource → its
/// declaration (telemetry.v1 §2.1, §2.2, §2.4).
#[test]
fn telemetry_declarations() {
    let path = profiles().join("telemetry/conformance/declarations.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let got = telemetry_declaration_json(&telemetry_declaration(&case["resource"]));
        let what = format!("{}: {}", case["note"], case["resource"]);
        expect(case, got, &what);
    }
    if bless() {
        write_json(&path, &doc);
    }
}

/// Bytes from their hex.
fn unhex(h: &str) -> Vec<u8> {
    assert!(h.len().is_multiple_of(2), "hex of whole bytes: {h}");
    (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&h[i..i + 2], 16).expect("hex"))
        .collect()
}

/// A data item as `telemetry/conformance/` writes it: JSON, with
/// `{"$int"}`, `{"$float"}` and `{"$bytes"}` for what JSON cannot hold.
fn telemetry_datum(v: &Value) -> Datum {
    if let Some(m) = v.as_object()
        && m.len() == 1
    {
        let (k, x) = m.iter().next().expect("one member");
        let text = x.as_str();
        match (k.as_str(), text) {
            ("$int", Some(t)) => return Datum::Int(t.parse().expect("an integer within i128")),
            ("$float", Some("NaN")) => return Datum::Float(f64::NAN),
            ("$float", Some("Infinity")) => return Datum::Float(f64::INFINITY),
            ("$float", Some("-Infinity")) => return Datum::Float(f64::NEG_INFINITY),
            ("$bytes", Some(t)) => {
                use base64::Engine as _;
                return Datum::Bytes(
                    base64::engine::general_purpose::STANDARD
                        .decode(t)
                        .expect("base64"),
                );
            }
            _ => {}
        }
    }
    match v {
        Value::Array(a) => Datum::Array(a.iter().map(telemetry_datum).collect()),
        Value::Object(m) => Datum::Map(
            m.iter()
                .map(|(k, v)| (k.clone(), telemetry_datum(v)))
                .collect(),
        ),
        other => Datum::from_json(other),
    }
}

/// `telemetry/conformance/samples.json`: a resolved resource and one
/// sample → its conformance, and the member's kind and unit as it gives
/// them (telemetry.v1 §2.5, §2.6).
#[test]
fn telemetry_samples() {
    let path = profiles().join("telemetry/conformance/samples.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let decl = telemetry_declaration(&case["resource"]);
        let sample = &case["sample"];
        // The bytes as they came (0.2): JSON text, read strictly; CBOR in
        // hex; an attachment's bytes as text, read as JSON. A payload that
        // does not decode is undecodable; an attachment that does not is
        // its bytes, no string.
        let value = match (sample.get("json_text"), sample.get("cbor_hex")) {
            (Some(t), _) => Datum::from_json_bytes(t.as_str().expect("text").as_bytes()),
            (_, Some(h)) => Datum::from_cbor_bytes(&unhex(h.as_str().expect("hex"))),
            _ => sample.get("value").map(telemetry_datum),
        };
        let attachment = match sample.get("attachment_text") {
            Some(t) => Some(Datum::attachment(
                t.as_str().expect("text").as_bytes(),
                None,
            )),
            None => sample.get("attachment").map(telemetry_datum),
        };
        let payload = match &value {
            Some(value) if sample["undecodable"] != json!(true) => telemetry::Payload::Decoded {
                value,
                attachment: attachment.as_ref(),
            },
            _ => telemetry::Payload::Undecodable,
        };
        let c = telemetry::check(&decl, &payload);
        let reasons: Vec<&str> = match &c.conformance {
            telemetry::Conformance::Conforms => Vec::new(),
            telemetry::Conformance::DoesNot(p) => p.iter().map(|p| p.as_str()).collect(),
            telemetry::Conformance::Unobservable(u) | telemetry::Conformance::NotAsked(u) => {
                vec![u.as_str()]
            }
        };
        if let telemetry::Conformance::Conforms = c.conformance {
            assert!(
                c.reading.is_some(),
                "{}: a conforming sample is read",
                case["note"]
            );
        }
        let got = json!({
            "conformance": c.conformance.as_str(),
            "reasons": reasons,
            "kind": c.kind.map(|(k, _)| k.as_str()),
            "kind_from": c.kind.map(|(_, f)| f.as_str()),
            "unit": c.unit.as_ref().map(|(u, _)| u.clone()),
            "unit_from": c.unit.as_ref().map(|(_, f)| f.as_str()),
        });
        let what = format!("{}: {}", case["note"], case["sample"]);
        expect(case, got, &what);
    }
    if bless() {
        write_json(&path, &doc);
    }
}

/// A sample's value as `continuity.json` writes it, by its kind.
fn telemetry_measure(kind: &str, v: &Value) -> telemetry::Measure {
    use telemetry::Measure as M;
    let f = |v: &Value| v.as_f64().expect("a number");
    match MetricKind::parse(kind).expect("kind") {
        MetricKind::Counter => M::Counter(v.as_u64().expect("a counter")),
        MetricKind::Gauge => M::Gauge(f(v)),
        MetricKind::Text => M::Text(v.as_str().expect("text").to_owned()),
        MetricKind::Boolean => M::Boolean(v.as_bool().expect("a boolean")),
        MetricKind::Binary => M::Binary(Vec::new()),
        MetricKind::Histogram => M::Histogram(telemetry::Histogram {
            bounds: v["buckets"]
                .as_array()
                .expect("buckets")
                .iter()
                .map(f)
                .collect(),
            counts: v["counts"]
                .as_array()
                .expect("counts")
                .iter()
                .map(|c| c.as_u64().expect("a count"))
                .collect(),
            count: v["count"].as_u64().expect("count"),
            sum: f(&v["sum"]),
        }),
    }
}

/// `telemetry/conformance/continuity.json`: two conforming samples of one
/// member, their unit and the presence seen → the pair's continuity, how
/// its epoch was had, and its rate (telemetry.v1 §2.9–§2.11).
#[test]
fn telemetry_continuity() {
    let path = profiles().join("telemetry/conformance/continuity.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let measures: Vec<telemetry::Measure> = ["earlier", "later"]
            .iter()
            .map(|w| {
                telemetry_measure(case[*w]["kind"].as_str().expect("kind"), &case[*w]["value"])
            })
            .collect();
        let sampled = |w: &str, m| {
            let s = &case[w];
            let int = |v: &Value| -> Option<i128> {
                v.as_i64()
                    .map(i128::from)
                    .or_else(|| v.as_u64().map(i128::from))
            };
            // A stamp as zenoh carries it, its NTP64 value (0.2), or
            // already in nanoseconds.
            let stamp = s["stamp_ntp64"]
                .as_u64()
                .map(telemetry::stamp_ns)
                .or_else(|| int(&s["stamp_ns"]));
            telemetry::Sampled {
                value: m,
                epoch: s["epoch"].as_str(),
                timestamp_ms: s["timestamp_ms"].as_i64(),
                stamp_ns: stamp,
                arrival_ns: int(&s["arrival_ns"]).expect("arrival_ns"),
            }
        };
        let (a, b) = (
            sampled("earlier", &measures[0]),
            sampled("later", &measures[1]),
        );
        let presence = case["presence"].as_object().map(|p| telemetry::Presence {
            alone_throughout: p["alone_throughout"].as_bool().expect("alone_throughout"),
            earlier: p["earlier"].as_str(),
            later: p["later"].as_str(),
        });
        let p = telemetry::pair(&a, &b, case["unit"].as_str(), presence.as_ref());
        let (epoch, via) = match p.basis {
            None => (None, None),
            Some(telemetry::Basis::Assumed) => (Some("assumed"), None),
            Some(telemetry::Basis::Established(v)) => (Some("established"), Some(v.as_str())),
        };
        let got = json!({
            "continuity": p.continuity.as_str(),
            "epoch": epoch,
            "via": via,
            "rate": p.rate.as_ref().ok().map(|r| r.per_s),
            "rate_unit": p.rate.as_ref().ok().and_then(|r| r.unit.clone()),
            "no_rate": p.rate.as_ref().err().map(|n| n.as_str()),
        });
        let what = format!("{}", case["note"]);
        expect(case, got, &what);
    }
    if bless() {
        write_json(&path, &doc);
    }
}

// --- desired.v1 ----------------------------------------------------------------

/// `desired/conformance/schema.json`: the standard schema's published ids
/// (desired.v1 §2.10, §3), as `telemetry_schema` reads telemetry.v1's.
#[test]
fn desired_schema() {
    let path = profiles().join("desired/conformance/schema.json");
    let mut doc = read_json(&path);
    let file = read_json(
        &profiles()
            .join("desired")
            .join(doc["file"].as_str().expect("file")),
    );
    assert_eq!(
        json!(jcs_id(&file)),
        doc["current"],
        "the profile's file is the current revision; after an intended change, publish its id in v1.md §3, desired::PUBLISHED and this fixture"
    );
    let published: Vec<&str> = doc["published"]
        .as_array()
        .expect("published")
        .iter()
        .map(|p| p["id"].as_str().expect("id"))
        .collect();
    assert_eq!(published, desired::PUBLISHED, "desired::PUBLISHED");
    assert_eq!(
        published.last(),
        doc["current"].as_str().as_ref(),
        "the current id is the latest"
    );
    let defs = file["$defs"].as_object().expect("$defs");
    let types: Vec<&str> = doc["types"]
        .as_array()
        .expect("types")
        .iter()
        .map(|t| t["name"].as_str().expect("name"))
        .collect();
    assert_eq!(types, [desired::REPORT, desired::VERSION, desired::REFUSAL]);
    for t in &types {
        assert!(defs.contains_key(*t), "{t} is defined in the current file");
    }
    let root = repo_root();
    for case in doc["cases"].as_array_mut().expect("cases") {
        let document = match case["path"].as_str() {
            Some(p) => read_json(&root.join(p)),
            None => case["document"].clone(),
        };
        let id = jcs_id(&document);
        let got = json!({"id": id, "published": desired::is_published(&id)});
        let what = format!("{}", case["note"]);
        expect(case, got, &what);
    }
    if bless() {
        write_json(&path, &doc);
    }
}

/// A resource as `desired/conformance/` writes it: its kind, template and
/// resolved annotations.
fn desired_resource(r: &Value) -> (Kind, Template, BTreeMap<String, Value>) {
    let kind = match r["kind"].as_str().expect("kind") {
        "stream" => Kind::Stream,
        "state" => Kind::State,
        "event" => Kind::Event,
        "operation" => Kind::Operation,
        other => panic!("kind {other:?}"),
    };
    let template = Template::parse(r["template"].as_str().expect("template")).expect("a template");
    (kind, template, annotations(&r["annotations"]))
}

fn desired_declaration_json(d: &desired::Declaration) -> Value {
    match d {
        desired::Declaration::Declared(t) => json!({
            "declaration": d.as_str(),
            "targets": t
                .iter()
                .map(|t| json!({"param": t.param, "binds": t.binds.as_str()}))
                .collect::<Vec<_>>(),
        }),
        desired::Declaration::Invalid(r) => json!({
            "declaration": d.as_str(),
            "reasons": r.iter().map(|r| r.as_str()).collect::<Vec<_>>(),
        }),
        desired::Declaration::NotThisProfiles => json!({"declaration": d.as_str()}),
    }
}

/// `desired/conformance/declarations.json`: a resolved resource → its
/// declaration (desired.v1 §2.2).
#[test]
fn desired_declarations() {
    let path = profiles().join("desired/conformance/declarations.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let (kind, template, ann) = desired_resource(&case["resource"]);
        let got = desired_declaration_json(&desired::declaration(kind, &template, &ann));
        let what = format!("{}: {}", case["note"], case["resource"]);
        expect(case, got, &what);
    }
    if bless() {
        write_json(&path, &doc);
    }
}

/// `desired/conformance/bindings.json`: a desired resource, its author's
/// interface, the target's address and its requirement → the binding
/// (desired.v1 §2.3).
#[test]
fn desired_bindings() {
    use zenkey_model::grammar::{Addr, IfaceId};
    let path = profiles().join("desired/conformance/bindings.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let (kind, template, ann) = desired_resource(&case["resource"]);
        let decl = desired::declaration(kind, &template, &ann);
        let targets = decl.targets().expect("a declared resource");
        let iface: IfaceId = case["iface"]
            .as_str()
            .expect("iface")
            .parse()
            .expect("an interface id");
        let token = if case["resource"]["explicit"] == json!(true) {
            KindToken::ExplicitState
        } else {
            KindToken::State
        };
        let own: Addr = case["target"]
            .as_str()
            .expect("target")
            .parse()
            .expect("an address");
        let req = &case["requirement"];
        let bindings: Vec<String> = req["bindings"]
            .as_array()
            .expect("bindings")
            .iter()
            .map(|b| b.as_str().expect("a binding").to_owned())
            .collect();
        let params: BTreeMap<String, String> = req["params"]
            .as_object()
            .expect("params")
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.as_str().expect("a parameter binding").to_owned(),
                )
            })
            .collect();
        let slice = desired::Slice {
            iface: &iface,
            token,
            template: &template,
            targets,
        };
        let requirement = desired::Requirement {
            bindings: &bindings,
            params: &params,
            optional: req["optional"] == json!(true),
        };
        let b = desired::binding(&slice, &requirement, &own);
        let got = match &b {
            desired::Binding::Unbound => json!({"binding": b.as_str()}),
            desired::Binding::Invalid(p) => json!({
                "binding": b.as_str(),
                "reasons": p.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
            }),
            desired::Binding::Bound { key } => json!({"binding": b.as_str(), "key": key.as_str()}),
        };
        let what = format!("{}", case["note"]);
        expect(case, got, &what);
    }
    if bless() {
        write_json(&path, &doc);
    }
}

fn desired_stamp(v: &Value) -> Option<zenkey_model::seed::Stamp> {
    if v.is_null() {
        return None;
    }
    Some(
        zenkey_model::seed::Stamp::parse(
            v["time"].as_u64().expect("time"),
            v["id"].as_str().expect("id"),
        )
        .expect("a stamp"),
    )
}

fn desired_refusal(v: &Value) -> desired::Refusal {
    serde_json::from_value(v.clone()).expect("a refusal")
}

/// `desired/conformance/order.json`: one target's steps → what each does,
/// and its report after the last (desired.v1 §2.6 to §2.10).
#[test]
fn desired_order() {
    let path = profiles().join("desired/conformance/order.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let key = case["key"].as_str().map(str::to_owned);
        let mut t = desired::Target::new(key.clone());
        let mut steps = Vec::new();
        for step in case["steps"].as_array().expect("steps") {
            let token = if let Some(s) = step.get("sample") {
                let sample = desired::Sample {
                    via: match s["from"].as_str().expect("from") {
                        "owner" => desired::Via::Owner,
                        "archive" => desired::Via::Archive,
                        other => panic!("from {other:?}"),
                    },
                    mutation: match s["kind"].as_str().expect("kind") {
                        "put" => desired::Mutation::Put,
                        "delete" => desired::Mutation::Delete,
                        other => panic!("kind {other:?}"),
                    },
                    stamp: desired_stamp(&s["stamp"]),
                    revision: s.get("revision").and_then(Value::as_u64),
                    content: match (s.get("document"), s.get("bytes").and_then(Value::as_str)) {
                        (Some(d), _) => Some(desired::Content::of_json(d)),
                        (None, Some(h)) => Some(desired::Content::of_bytes(&unhex(h))),
                        (None, None) => None,
                    },
                };
                t.observe(sample).as_str()
            } else if let Some(f) = step.get("finish") {
                let ended = if f == "done" {
                    desired::Ended::Done
                } else if let Some(r) = f.get("refused") {
                    desired::Ended::Refused(desired_refusal(r))
                } else if let Some(r) = f.get("failed") {
                    desired::Ended::Failed(desired_refusal(r))
                } else {
                    panic!("finish {f}")
                };
                t.finish(ended).as_str()
            } else if step.get("call").is_some() {
                t.call();
                "called"
            } else if step.get("restart").is_some() {
                t = desired::Target::new(key.clone());
                "restarted"
            } else {
                panic!("step {step}")
            };
            steps.push(token);
        }
        let got = json!({"steps": steps, "report": t.report().to_json()});
        let what = format!("{}", case["note"]);
        expect(case, got, &what);
    }
    if bless() {
        write_json(&path, &doc);
    }
}

/// A report as a reader reads one: from its text, or from its decoded
/// JSON.
fn desired_report(case: &Value) -> Result<desired::Report, desired::Unreadable> {
    match case.get("text").and_then(Value::as_str) {
        Some(t) => desired::decode(t.as_bytes()),
        None => desired::Report::from_json(&case["document"]),
    }
}

/// `desired/conformance/reports.json`: a report → whether it reads, and
/// its findings (desired.v1 §2.4, §2.10).
#[test]
fn desired_reports() {
    let path = profiles().join("desired/conformance/reports.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let got = match desired_report(case) {
            Ok(r) => json!({
                "readable": true,
                "errors": r.check().iter().map(|f| f.as_str()).collect::<Vec<_>>(),
            }),
            Err(u) => json!({"readable": false, "errors": [u.as_str()]}),
        };
        let what = format!("{}", case["note"]);
        expect(case, got, &what);
    }
    if bless() {
        write_json(&path, &doc);
    }
}

fn desired_seen(v: &Value) -> desired::Seen {
    match v.as_str().expect("presence") {
        "present" => desired::Seen::Present,
        "absent" => desired::Seen::Absent,
        "incomplete" => desired::Seen::Incomplete,
        "across_face" => desired::Seen::AcrossFace,
        other => panic!("presence {other:?}"),
    }
}

fn desired_judged(j: desired::Judged) -> Value {
    json!({"answer": j.answer.as_str(), "reason": j.reason.as_str()})
}

/// `desired/conformance/conclusions.json`: one reading of a target's role
/// and its author's document → §5's four questions (desired.v1 §2.17).
#[test]
fn desired_conclusions() {
    let path = profiles().join("desired/conformance/conclusions.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().expect("cases") {
        let t = &case["reading"]["target"];
        let a = &case["reading"]["author"];
        let report = match &t["report"] {
            Value::Null => None,
            Value::String(s) if s == "undecodable" => Some(Err(())),
            r => Some(desired::Report::from_json(&r["document"]).map_err(|_| ())),
        };
        let target = desired::TargetReading {
            presence: desired_seen(&t["presence"]),
            descriptor: match &t["descriptor"] {
                Value::Null => None,
                d if d["lists"] == json!(false) => Some(desired::Listing::NotListed),
                d => Some(desired::Listing::Listed {
                    bound: d["bound"] == json!(true),
                }),
            },
            report: match &report {
                None => desired::ReportRead::NotRead,
                Some(Err(())) => desired::ReportRead::Unreadable,
                Some(Ok(r)) => desired::ReportRead::Read(r),
            },
            freshness: match t["report_freshness"].as_str().expect("report_freshness") {
                "fresh" => freshness::Verdict::Fresh,
                "stale" => freshness::Verdict::Stale,
                "unobservable" => freshness::Verdict::Unobservable,
                "not_asked" => freshness::Verdict::NotAsked,
                other => panic!("report_freshness {other:?}"),
            },
        };
        let author = desired::AuthorReading {
            presence: desired_seen(&a["presence"]),
            current: match &a["current"] {
                Value::Null => desired::Current::NotRead,
                Value::String(s) if s == "incomplete" => desired::Current::Incomplete,
                Value::String(s) if s == "silent" => desired::Current::Silent,
                c => match (c.get("put"), c.get("delete")) {
                    (Some(s), _) => desired::Current::Put(desired_stamp(s).expect("a stamp")),
                    (_, Some(s)) => desired::Current::Delete(desired_stamp(s).expect("a stamp")),
                    _ => panic!("current {c}"),
                },
            },
        };
        let got = json!({
            "converged": desired_judged(desired::converged(&target, &author)),
            "refused": desired_judged(desired::refused(&target)),
            "last_known": desired_judged(desired::last_known(&target)),
            "in_force": desired_judged(desired::in_force(&target)),
        });
        let what = format!("{}", case["note"]);
        expect(case, got, &what);
    }
    if bless() {
        write_json(&path, &doc);
    }
}
