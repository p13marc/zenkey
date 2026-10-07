//! The conformance fixtures this crate seeds (`spec/conformance/`, #608).
//!
//! Each fixture holds its inputs and the expected outputs. This test checks
//! them; `ZK2_BLESS=1 cargo test -p zenkey-model --test conformance`
//! rewrites the expected outputs from this implementation (then read the
//! diff: a bless is a claim that every changed expectation is right).
//! `spec/contract.schema.json` is checked, and blessed, the same way.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use zenkey_model::authoring::ContractFile;
use zenkey_model::bundle::Bundle;
use zenkey_model::canonical::{Fingerprint, canonical_bytes};
use zenkey_model::contract::load_str;
use zenkey_model::grammar::{ZkKey, parse};
use zenkey_model::slug::{chunk_slug, chunk_unslug};
use zenkey_model::template::{Template, resolve};

fn spec() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec")
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

fn key_json(k: &ZkKey) -> Value {
    match k {
        ZkKey::Data {
            addr,
            iface,
            kind,
            resource,
        } => {
            json!({"form": "data", "system": addr.system.as_str(), "service": addr.service.as_str(),
                    "iface": iface.to_string(), "token": kind.as_str(), "resource": resource})
        }
        ZkKey::Instance { addr, instance } => {
            json!({"form": "instance", "system": addr.system.as_str(),
                    "service": addr.service.as_str(), "instance": instance.as_str()})
        }
        ZkKey::Alive {
            addr,
            iface,
            instance,
            fp,
        } => {
            json!({"form": "alive", "system": addr.system.as_str(), "service": addr.service.as_str(),
                    "iface": iface.to_string(), "instance": instance.as_str(), "fp16": fp.as_str()})
        }
        ZkKey::Member {
            addr,
            iface,
            member,
            epoch,
        } => {
            json!({"form": "member", "system": addr.system.as_str(), "service": addr.service.as_str(),
                    "iface": iface.to_string(), "member": member, "epoch": epoch.as_str()})
        }
        ZkKey::Contract { iface, fingerprint } => {
            json!({"form": "contract", "iface": iface.to_string(), "sha256": fingerprint.as_str()})
        }
    }
}

#[test]
fn keys() {
    let path = spec().join("conformance/keys.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().unwrap() {
        let key = case["key"].as_str().unwrap().to_owned();
        let got = match parse(&key) {
            Ok(k) => {
                assert_eq!(
                    k.to_key().unwrap().as_str(),
                    key,
                    "{key} builds back exactly"
                );
                key_json(&k)
            }
            Err(_) => Value::Null,
        };
        expect(case, got, &key);
    }
    if bless() {
        write_json(&path, &doc);
    }
}

#[test]
fn slugs() {
    let path = spec().join("conformance/slugs.json");
    let mut doc = read_json(&path);
    for case in doc["slug"].as_array_mut().unwrap() {
        let value = case["value"].as_str().unwrap().to_owned();
        let slug = chunk_slug(&value);
        assert_eq!(
            chunk_unslug(&slug).as_deref(),
            Some(value.as_str()),
            "{value:?} round-trips"
        );
        expect(case, json!(slug), &value);
    }
    for case in doc["unslug"].as_array_mut().unwrap() {
        let chunk = case["chunk"].as_str().unwrap().to_owned();
        expect(case, json!(chunk_unslug(&chunk)), &chunk);
    }
    if bless() {
        write_json(&path, &doc);
    }
}

#[test]
fn templates() {
    let path = spec().join("conformance/templates.json");
    let mut doc = read_json(&path);
    for case in doc["cases"].as_array_mut().unwrap() {
        let tpls: Vec<Template> = case["templates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| Template::parse(t.as_str().unwrap()).unwrap())
            .collect();
        let chunks: Vec<String> = serde_json::from_value(case["chunks"].clone()).unwrap();
        let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
        let got = match resolve(&tpls, &refs) {
            Some((i, b)) => json!({"winner": tpls[i].as_str(), "bindings": b}),
            None => Value::Null,
        };
        expect(case, got, &format!("{chunks:?}"));
    }
    if bless() {
        write_json(&path, &doc);
    }
}

/// Every `contracts/*.toml`: the sorted diagnostic codes, the fingerprint,
/// and the canonical bytes (`<stem>.canonical.json`) of a valid one.
#[test]
fn contracts() {
    let dir = spec().join("conformance/contracts");
    let expect_path = dir.join("expect.json");
    let mut want = read_json(&expect_path);
    let mut got = serde_json::Map::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    let mut fps: BTreeMap<String, String> = BTreeMap::new();
    for p in &files {
        let stem = p.file_stem().unwrap().to_str().unwrap().to_owned();
        let l = load_str(&std::fs::read_to_string(p).unwrap(), &dir, None);
        let fp = l.contract.as_ref().map(|c| Fingerprint::of(c).to_string());
        let canon_path = dir.join(format!("{stem}.canonical.json"));
        if let Some(c) = &l.contract {
            let bytes = canonical_bytes(c);
            if bless() {
                std::fs::write(&canon_path, &bytes).unwrap();
            } else {
                assert_eq!(
                    std::fs::read(&canon_path).ok(),
                    Some(bytes),
                    "{stem}: canonical bytes"
                );
            }
        }
        if let Some(f) = &fp {
            fps.insert(stem.clone(), f.clone());
        }
        let entry = json!({"codes": l.report.codes(), "fingerprint": fp});
        if !bless() {
            assert_eq!(want["cases"][&stem], entry, "{stem}:\n{}", l.report);
        }
        got.insert(stem, entry);
    }
    // The D2 pair: defaulted and spelled-out fingerprint alike.
    assert_eq!(
        fps.get("ok-defaults-defaulted"),
        fps.get("ok-defaults-spelled")
    );
    assert!(fps.contains_key("ok-defaults-defaulted"));
    if bless() {
        want["cases"] = Value::Object(got);
        write_json(&expect_path, &want);
    } else {
        let n = want["cases"].as_object().unwrap().len();
        assert_eq!(n, files.len(), "expect.json lists exactly the .toml files");
    }
}

/// Every `bundles/*.bundle.json` verifies, or is refused with its tag.
#[test]
fn bundles() {
    let dir = spec().join("conformance/bundles");
    let expect_path = dir.join("expect.json");
    if bless() {
        bless_bundles(&dir);
    }
    let want = read_json(&expect_path);
    let cases = want["cases"].as_object().unwrap();
    let mut seen = 0;
    for (name, w) in cases {
        let bytes = std::fs::read(dir.join(name)).unwrap();
        let got = match Bundle::verify(&bytes) {
            Ok(b) => json!({"ok": true, "fingerprint": b.fingerprint().to_string()}),
            Err(e) => json!({"ok": false, "error": e.tag()}),
        };
        assert_eq!(&got, w, "{name}");
        seen += 1;
    }
    let files = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".bundle.json"))
        .count();
    assert_eq!(seen, files, "expect.json lists exactly the bundle files");
}

/// Builds the bundle seeds: one valid bundle, and one refusal per tag.
fn bless_bundles(dir: &Path) {
    let src = std::fs::read_to_string(dir.join("seed.toml")).unwrap();
    let l = load_str(&src, dir, None);
    let c = l.contract.unwrap_or_else(|| panic!("{}", l.report));
    let b = Bundle::build(&c);
    let valid = b.to_value();
    let jcs = |v: &Value| serde_json_canonicalizer::to_vec(v).unwrap();
    let mut out: Vec<(&str, Vec<u8>, Value)> = Vec::new();
    let fp = b.fingerprint().to_string();
    out.push((
        "valid.bundle.json",
        b.to_bytes(),
        json!({"ok": true, "fingerprint": fp}),
    ));

    let ids: Vec<String> = b.schemas.keys().cloned().collect();
    let json_id = ids
        .iter()
        .find(|id| b.schemas[*id]["kind"] == "jsonschema")
        .unwrap()
        .clone();
    let proto_id = ids
        .iter()
        .find(|id| b.schemas[*id]["kind"] == "protobuf")
        .unwrap()
        .clone();

    let mut v = valid.clone();
    v["schemas"][&json_id]["data"]["$defs"]["Injected"] = json!({"type": "string"});
    out.push((
        "schema-hash.bundle.json",
        jcs(&v),
        json!({"ok": false, "error": "schema_hash"}),
    ));

    let mut v = valid.clone();
    v["schemas"][&proto_id]["data"] = json!("AAAA");
    out.push((
        "schema-hash-protobuf.bundle.json",
        jcs(&v),
        json!({"ok": false, "error": "schema_hash"}),
    ));

    let mut v = valid.clone();
    v["schemas"].as_object_mut().unwrap().remove(&json_id);
    out.push((
        "missing-schema.bundle.json",
        jcs(&v),
        json!({"ok": false, "error": "missing_schema"}),
    ));

    let mut v = valid.clone();
    let extra = json!({"kind": "jsonschema", "data": {"$defs": {}}});
    let extra_id = zenkey_model::schema::sha256_id(&jcs(&extra["data"]));
    v["schemas"][&extra_id] = extra;
    out.push((
        "unlisted-schema.bundle.json",
        jcs(&v),
        json!({"ok": false, "error": "unlisted_schema"}),
    ));

    let mut v = valid.clone();
    v["schemas"][&json_id]["kind"] = json!("protobuf");
    out.push((
        "schema-kind.bundle.json",
        jcs(&v),
        json!({"ok": false, "error": "schema_kind"}),
    ));

    let mut v = valid.clone();
    v["contract"]["format"] = json!("zk2-contract/draft-0");
    out.push((
        "format.bundle.json",
        jcs(&v),
        json!({"ok": false, "error": "format"}),
    ));

    let mut v = valid.clone();
    v["contract"]["resources"][0]["annotations"]["freshness.ttl_s"] =
        json!(9_007_199_254_740_992_u64);
    out.push((
        "restrictions.bundle.json",
        jcs(&v),
        json!({"ok": false, "error": "restrictions"}),
    ));

    let text = String::from_utf8(b.to_bytes()).unwrap();
    let dup = text.replacen("{\"contract\":", "{\"extras\":{},\"contract\":", 1);
    out.push((
        "duplicate-key.bundle.json",
        dup.into_bytes(),
        json!({"ok": false, "error": "json"}),
    ));

    let mut v = valid.clone();
    v["extras"]["sha256:".to_owned() + &"0".repeat(64)] =
        json!({"media_type": "application/json", "data": {}});
    out.push((
        "extra-hash.bundle.json",
        jcs(&v),
        json!({"ok": false, "error": "extra_hash"}),
    ));

    let mut cases = serde_json::Map::new();
    for (name, bytes, want) in out {
        std::fs::write(dir.join(name), bytes).unwrap();
        cases.insert(name.to_owned(), want);
    }
    let mut doc = read_json(&dir.join("expect.json"));
    doc["cases"] = Value::Object(cases);
    write_json(&dir.join("expect.json"), &doc);
}

/// `spec/contract.schema.json` is generated from the authoring types.
#[test]
fn contract_schema_is_current() {
    let path = spec().join("contract.schema.json");
    let schema = schemars::schema_for!(ContractFile);
    let text = serde_json::to_string_pretty(&schema).unwrap() + "\n";
    if bless() {
        std::fs::write(&path, &text).unwrap();
    } else {
        let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            on_disk == text,
            "spec/contract.schema.json is stale: ZK2_BLESS=1 cargo test -p zenkey-model --test conformance"
        );
    }
}
