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
    // `"unbounded"` and the number it names fingerprint alike (§2.2, 0.57).
    assert_eq!(
        fps.get("ok-cardinality-unbounded"),
        fps.get("ok-cardinality-no-ceiling")
    );
    assert!(fps.contains_key("ok-cardinality-unbounded"));
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
        let verified = match w.get("expect_fingerprint").and_then(Value::as_str) {
            Some(fp) => Bundle::verify_expecting(&bytes, &Fingerprint::parse(fp).unwrap()),
            None => Bundle::verify(&bytes),
        };
        let mut got = match verified {
            Ok(b) => json!({"ok": true, "fingerprint": b.fingerprint().to_string()}),
            Err(e) => json!({"ok": false, "error": e.tag()}),
        };
        if let Some(fp) = w.get("expect_fingerprint") {
            got["expect_fingerprint"] = fp.clone();
        }
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

    let mut v = valid.clone();
    let doc = json!({"views": []});
    let doc_id = zenkey_model::schema::sha256_id(&jcs(&doc));
    v["extras"][&doc_id] = json!({"media_type": "application/json", "data": doc});
    out.push((
        "extras-unreferenced.bundle.json",
        jcs(&v),
        json!({"ok": false, "error": "extras"}),
    ));

    out.push((
        "not-utf8.bundle.json",
        vec![0xff, 0xfe],
        json!({"ok": false, "error": "shape"}),
    ));

    let mut v = valid.clone();
    v["signature"] = json!({});
    out.push((
        "unknown-member.bundle.json",
        jcs(&v),
        json!({"ok": false, "error": "shape"}),
    ));

    let mut v = valid.clone();
    v.as_object_mut().unwrap().remove("contract");
    out.push((
        "no-contract.bundle.json",
        jcs(&v),
        json!({"ok": false, "error": "shape"}),
    ));

    let mut v = valid.clone();
    v["contract"]["schemas"] = json!({});
    out.push((
        "contract-schemas-not-a-list.bundle.json",
        jcs(&v),
        json!({"ok": false, "error": "shape"}),
    ));

    let other = format!("sha256:{}", "0".repeat(64));
    out.push((
        "fingerprint.bundle.json",
        b.to_bytes(),
        json!({"ok": false, "error": "fingerprint", "expect_fingerprint": other}),
    ));

    // Step 7: absent `schemas` or `extras` means empty.
    let mut v = valid.clone();
    v.as_object_mut().unwrap().remove("extras");
    out.push((
        "no-extras.bundle.json",
        jcs(&v),
        json!({"ok": true, "fingerprint": fp}),
    ));

    // Step 9: base64 is RFC 4648 §4, padded.
    let data = valid["schemas"][&proto_id]["data"].as_str().unwrap();
    assert!(
        data.ends_with('='),
        "the seed's descriptor set needs padding"
    );
    let mut v = valid.clone();
    v["schemas"][&proto_id]["data"] = json!(data.trim_end_matches('='));
    out.push((
        "base64-unpadded.bundle.json",
        jcs(&v),
        json!({"ok": false, "error": "shape"}),
    ));

    // Step 9: an entry holds `kind` and `data`, nothing else; one that is not
    // an object has no `kind`.
    let mut v = valid.clone();
    v["schemas"][&json_id]["signature"] = json!("unverified");
    out.push((
        "schema-entry-member.bundle.json",
        jcs(&v),
        json!({"ok": false, "error": "shape"}),
    ));
    let mut v = valid.clone();
    v["schemas"][&json_id] = json!([]);
    out.push((
        "schema-entry-not-object.bundle.json",
        jcs(&v),
        json!({"ok": false, "error": "schema_kind"}),
    ));

    // Step 9: a JSON Schema document holding an integer beyond 2^53−1 has
    // no id, even under an id hashed over its exact digits.
    let mut big = valid["schemas"][&json_id]["data"].clone();
    big["$defs"]["Big"] = json!({"maximum": 9_007_199_254_740_993_u64});
    let exact_id = zenkey_model::schema::sha256_id(&serde_json::to_vec(&big).unwrap());
    let text = String::from_utf8(jcs(&valid))
        .unwrap()
        .replace(&json_id, &exact_id);
    let mut v: Value = serde_json::from_str(&text).unwrap();
    v["schemas"][&exact_id]["data"] = big;
    out.push((
        "schema-data-big-integer.bundle.json",
        jcs(&v),
        json!({"ok": false, "error": "schema_hash"}),
    ));

    // Verification checks hashes, not content (§9.6). What 0.5's rule built
    // from `contracts/e024-identical` — the shared id listed once, under
    // the later name `same-b` — beside the seed's artifacts: the `$ref`
    // `same-a.json#/$defs/Point` names a stem no artifact has. It verifies;
    // no 0.6 builder writes it (E024), and the classifier reads that `$ref`
    // as `schema_unreadable` (§9.8).
    let point = json!({"$defs": {"Point": {"type": "object", "properties": {
        "x": {"type": "number"}, "y": {"type": "number"}}}}});
    let refs = json!({"$defs": {"Track": {"type": "object", "properties": {
        "at": {"$ref": "same-a.json#/$defs/Point"}}}}});
    let mut v = valid.clone();
    let mut listed = v["contract"]["schemas"].as_array().unwrap().clone();
    for (name, doc) in [("same-b", &point), ("refs-same", &refs)] {
        let id = zenkey_model::schema::sha256_id(&jcs(doc));
        listed.push(json!({"id": id, "kind": "jsonschema", "name": name}));
        v["schemas"][&id] = json!({"kind": "jsonschema", "data": doc});
    }
    listed.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    v["contract"]["schemas"] = Value::Array(listed);
    let bytes = jcs(&v);
    let fp_refs = Bundle::verify(&bytes).unwrap().fingerprint().to_string();
    out.push((
        "ref-names-no-artifact.bundle.json",
        bytes,
        json!({"ok": true, "fingerprint": fp_refs}),
    ));

    // Step 10: an extra holds `media_type` and `data`, nothing else.
    let mut v = valid.clone();
    v["extras"]["sha256:".to_owned() + &"0".repeat(64)] =
        json!({"media_type": "application/json", "data": {}, "signature": "x"});
    out.push((
        "extra-entry-member.bundle.json",
        jcs(&v),
        json!({"ok": false, "error": "shape"}),
    ));

    // Step 9 (core §9.6, 0.75): a packed artifact is carried and hashed as
    // a JSON Schema one, by its JCS bytes. Built from `contracts/ok-packed`.
    let packed = zenkey_model::contract::load_path(&dir.join("../contracts/ok-packed.toml"));
    let packed = Bundle::build(
        &packed
            .contract
            .unwrap_or_else(|| panic!("{}", packed.report)),
    );
    out.push((
        "packed.bundle.json",
        packed.to_bytes(),
        json!({"ok": true, "fingerprint": packed.fingerprint().to_string()}),
    ));
    let mut v = packed.to_value();
    let packed_id = packed
        .schemas
        .keys()
        .find(|id| packed.schemas[*id]["kind"] == "packed")
        .unwrap()
        .clone();
    v["schemas"][&packed_id]["data"]["types"]["Injected"] = json!({"fields": []});
    out.push((
        "schema-hash-packed.bundle.json",
        jcs(&v),
        json!({"ok": false, "error": "schema_hash"}),
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

/// `spec/conformance/descriptors/`: each document → the sorted `D…` codes
/// `descriptor::check` reports against `contracts/nav.v2.toml`.
#[test]
fn descriptors() {
    let dir = spec().join("conformance/descriptors");
    let loaded = zenkey_model::contract::load_path(&dir.join("contracts/nav.v2.toml"));
    let contract = loaded
        .contract
        .unwrap_or_else(|| panic!("the fixture contract does not load:\n{}", loaded.report));
    let path = dir.join("expect.json");
    let mut doc = read_json(&path);
    let names: Vec<String> = doc["cases"]
        .as_object()
        .expect("cases")
        .keys()
        .cloned()
        .collect();
    for name in names {
        let text = std::fs::read_to_string(dir.join(format!("{name}.json"))).unwrap();
        let (_, report) = zenkey_model::descriptor::check(&text, &[&contract]);
        let got = json!({ "codes": report.codes() });
        if bless() {
            doc["cases"][&name] = got;
        } else {
            assert_eq!(doc["cases"][&name], got, "descriptor {name}:\n{report}");
        }
    }
    if bless() {
        write_json(&path, &doc);
    }
}

/// `spec/descriptor.schema.json` is generated from the descriptor types.
#[test]
fn descriptor_schema_is_current() {
    let path = spec().join("descriptor.schema.json");
    let schema = schemars::schema_for!(zenkey_model::descriptor::Descriptor);
    let text = serde_json::to_string_pretty(&schema).unwrap() + "\n";
    if bless() {
        std::fs::write(&path, &text).unwrap();
    } else {
        assert_eq!(
            std::fs::read_to_string(&path).unwrap_or_default(),
            text,
            "spec/descriptor.schema.json is stale: ZK2_BLESS=1 cargo test -p zenkey-model --test conformance"
        );
    }
}

/// `spec/conformance/sets/`: a directory of contracts → the codes of the
/// set checks over all of them (E035, E036). Written by hand, checked.
#[test]
fn sets() {
    let dir = spec().join("conformance/sets");
    let doc = read_json(&dir.join("expect.json"));
    for (name, want) in doc["sets"].as_object().expect("sets") {
        let mut files: Vec<_> = std::fs::read_dir(dir.join(name))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .collect();
        files.sort();
        let loaded: Vec<_> = files
            .iter()
            .map(|p| {
                let l = zenkey_model::contract::load_path(p);
                l.contract
                    .unwrap_or_else(|| panic!("{}: does not load:\n{}", p.display(), l.report))
            })
            .collect();
        let refs: Vec<_> = loaded.iter().collect();
        let report = zenkey_model::contract::check_set(&refs);
        assert_eq!(
            want,
            &json!({ "codes": report.codes() }),
            "set {name}:\n{report}"
        );
    }
}

/// `spec/conformance/history/`: a history root → its problems as
/// `[at, tag]`. Written by hand, checked.
#[test]
fn history() {
    let dir = spec().join("conformance/history");
    let doc = read_json(&dir.join("expect.json"));
    for (name, want) in doc["cases"].as_object().expect("cases") {
        let got: Vec<Value> = zenkey_model::history::check_tagged(&dir.join(name))
            .into_iter()
            .map(|p| json!([p.at, p.tag]))
            .collect();
        assert_eq!(want, &Value::Array(got), "history {name}");
    }
}

/// `spec/conformance/compat/`: the compatibility matrix (#607), evaluated
/// by the classifier (#618). Each case's class, warnings and, for
/// protobuf, `same_revision` must be what `expect.json` says; `invalid`
/// means the new revision does not load (E037).
#[test]
fn compat() {
    use zenkey_model::compat::{Revision, check_history, compare, same_revision};
    let dir = spec().join("conformance/compat");
    let doc = read_json(&dir.join("expect.json"));
    // A one-resource contract over a schema file, loaded from `at`.
    let load = |at: &Path, schema: &str, ty: &str| {
        let kind = if schema.ends_with(".proto") {
            "protobuf"
        } else if ty.starts_with("packed:") {
            "packed"
        } else {
            "jsonschema"
        };
        let text = format!(
            "[interface]\nname = \"m\"\nmajor = 1\nminor = 0\n[schemas]\n{kind} = [\"{schema}\"]\n\
             [resources.s]\nkind = \"state\"\ntype = \"{ty}\"\n"
        );
        load_str(&text, at, None)
    };
    let rev = |l: zenkey_model::contract::Loaded, what: &str| {
        Revision::of(
            &l.contract
                .unwrap_or_else(|| panic!("{what} does not load:\n{}", l.report)),
        )
    };
    let mut seen = 0;
    for (name, case) in doc["cases"].as_object().expect("cases") {
        let root = dir.join(name);
        let want_class = case["class"].as_str().unwrap();
        let (old, new) = if let Some(rest) = name.strip_prefix("payload/") {
            let ty = case["type"].as_str().unwrap();
            let (lo, ln) = if rest.starts_with("protobuf/") {
                (
                    load(&root.join("old"), "m.proto", ty),
                    load(&root.join("new"), "m.proto", ty),
                )
            } else {
                (load(&root, "old.json", ty), load(&root, "new.json", ty))
            };
            if want_class == "invalid" {
                let errors: Vec<_> = ln.report.errors().map(|d| d.code).collect();
                assert_eq!(errors, ["E037"], "{name}: the new revision must not load");
                seen += 1;
                continue;
            }
            (rev(lo, name), rev(ln, name))
        } else if name.starts_with("contract/") || name.starts_with("frame/") {
            let l = |f: &str| zenkey_model::contract::load_path(&root.join(f));
            (rev(l("old.toml"), name), rev(l("new.toml"), name))
        } else if name.starts_with("transitive/") {
            let ty = case["type"].as_str().unwrap();
            let v = |n: &str| {
                if root.join(n).is_dir() {
                    rev(load(&root.join(n), "m.proto", ty), name)
                } else {
                    rev(load(&root, &format!("{n}.json"), ty), name)
                }
            };
            let (v1, v2, v3) = (v("v1"), v("v2"), v("v3"));
            for (against, r) in [("v1", &v1), ("v2", &v2)] {
                let got = compare(r, &v3).class().as_str();
                assert_eq!(
                    case["against"][against], got,
                    "{name}: v3 against {against}"
                );
            }
            let verdict = check_history(&[v1, v2], &v3);
            assert_eq!(
                want_class,
                verdict.class().as_str(),
                "{name}: {:#?}",
                verdict.findings
            );
            seen += 1;
            continue;
        } else {
            panic!("{name}: unknown case family");
        };
        let verdict = compare(&old, &new);
        assert_eq!(
            want_class,
            verdict.class().as_str(),
            "{name}: {:#?}",
            verdict.findings
        );
        let warnings: Vec<Value> = verdict
            .warning_rules()
            .into_iter()
            .map(Value::from)
            .collect();
        assert_eq!(case["warnings"], Value::Array(warnings), "{name}: warnings");
        if let Some(same) = case.get("same_revision") {
            assert_eq!(
                same,
                &Value::Bool(same_revision(&old, &new)),
                "{name}: same_revision"
            );
        }
        seen += 1;
    }
    assert_eq!(seen, doc["cases"].as_object().unwrap().len());
}

/// Seconds as a fixture writes them.
fn secs(v: &Value) -> std::time::Duration {
    std::time::Duration::from_secs_f64(v.as_f64().unwrap_or_else(|| panic!("seconds: {v}")))
}

/// Instants per member, as `budget/` writes them.
fn instants(v: &Value) -> BTreeMap<String, Vec<std::time::Duration>> {
    v.as_object()
        .expect("members")
        .iter()
        .map(|(m, ts)| {
            (
                m.clone(),
                ts.as_array().expect("instants").iter().map(secs).collect(),
            )
        })
        .collect()
}

/// A bound as `budget/` writes it.
fn bound_read(v: &Value) -> zenkey_model::budget::Bound {
    use zenkey_model::budget::Bound;
    match v {
        Value::String(s) if s == "no_ceiling" => Bound::NoCeiling,
        Value::String(s) if s == "untemplated" => Bound::Untemplated,
        v => Bound::Of(v.as_u64().unwrap_or_else(|| panic!("bound {v}"))),
    }
}

/// `spec/conformance/budget/bounds.json` (core §2.7, 0.24): the contract's
/// `cardinality` and each exposing instance's stated one → the bound.
/// Written by hand from the text, checked, never blessed.
#[test]
fn budget_bounds() {
    let doc = read_json(&spec().join("conformance/budget/bounds.json"));
    for case in doc["cases"].as_array().expect("cases") {
        let stated: Vec<Option<u64>> = case["stated"]
            .as_array()
            .expect("stated")
            .iter()
            .map(Value::as_u64)
            .collect();
        let b = zenkey_model::budget::bound(case["contract"].as_u64(), stated);
        let got = match b {
            zenkey_model::budget::Bound::Of(n) => json!({"bound": n}),
            other => json!({"bound": other.token()}),
        };
        assert_eq!(case["expect"], got, "{}", case["note"]);
    }
}

/// `spec/conformance/budget/population.json` (core §2.7, 0.24): a kind, a
/// bound and one reading → the verdict and its reason class. Written by
/// hand from the text, checked, never blessed.
#[test]
fn budget_populations() {
    use zenkey_model::authoring::Kind;
    use zenkey_model::budget::{Reading, Window, population};
    let doc = read_json(&spec().join("conformance/budget/population.json"));
    for case in doc["cases"].as_array().expect("cases") {
        let kind = match case["kind"].as_str().expect("kind") {
            "stream" => Kind::Stream,
            "state" => Kind::State,
            "event" => Kind::Event,
            "operation" => Kind::Operation,
            other => panic!("kind {other:?}"),
        };
        let r = &case["reading"];
        let reading = match r["via"].as_str().expect("via") {
            "get" => Reading::Get {
                members: r["members"].as_u64().expect("members"),
                complete: r["complete"].as_bool().expect("complete"),
            },
            "window" => Reading::Window(Window {
                heard: instants(&r["heard"]),
                listened: secs(&r["listened_s"]),
                lossless: r["lossless"].as_bool().expect("lossless"),
                present: r["present"].as_bool().expect("present"),
            }),
            other => panic!("via {other:?}"),
        };
        let retention = (!case["retention_s"].is_null()).then(|| secs(&case["retention_s"]));
        let j = population(kind, bound_read(&case["bound"]), retention, &reading);
        assert_eq!(
            case["expect"],
            json!({"verdict": j.verdict.as_str(), "reason": j.reason.as_str()}),
            "{}",
            case["note"]
        );
    }
}

/// `spec/conformance/budget/rate.json` (core §2.7, 0.24; 0.25): an event's
/// rate and the occurrences a window heard, with the window's premises →
/// the verdict and its reason class. Written by hand from the text,
/// checked, never blessed.
#[test]
fn budget_rates() {
    use zenkey_model::budget::{Occurrences, rate};
    use zenkey_model::contract::Rate;
    let doc = read_json(&spec().join("conformance/budget/rate.json"));
    for case in doc["cases"].as_array().expect("cases") {
        let r = case["rate"]
            .as_str()
            .map(|s| Rate::parse(s).unwrap_or_else(|| panic!("rate {s:?}")));
        let o = &case["reading"];
        let occurrences = Occurrences {
            listened: secs(&o["listened_s"]),
            lossless: o["lossless"].as_bool().expect("lossless"),
            present: o["present"].as_bool().expect("present"),
            members: instants(&o["members"]),
        };
        let j = rate(r, &occurrences);
        assert_eq!(
            case["expect"],
            json!({"verdict": j.verdict.as_str(), "reason": j.reason.as_str()}),
            "{}",
            case["note"]
        );
    }
}

/// `spec/conformance/budget/lossless.json` (core §2.7, 0.25, 0.27): a
/// resource's declared QoS, the reader's own loss, what it saw of its own
/// receive path, and miss detection → whether a window lost no delivery,
/// and why; and the stall bound from the queue's `wait_before_close`, the
/// lease and its keep-alive count. Written by hand from the text, checked,
/// never blessed.
#[test]
fn budget_lossless() {
    use std::time::Duration;
    use zenkey_model::authoring::{Congestion, Reliability};
    use zenkey_model::budget::{Delivery, MissDetection, ReaderStall, lossless, stall_bound};
    let doc = read_json(&spec().join("conformance/budget/lossless.json"));
    let ms = |v: &serde_json::Value, what: &str| {
        Duration::from_millis(v.as_u64().unwrap_or_else(|| panic!("{what}")))
    };
    let bounds = doc["stall_bounds"].as_array().expect("stall_bounds");
    assert!(!bounds.is_empty());
    for case in bounds {
        let keep_alive = case["keep_alive"].as_u64().expect("keep_alive");
        let got = stall_bound(
            ms(&case["wait_before_close_ms"], "wait_before_close_ms"),
            ms(&case["lease_ms"], "lease_ms"),
            u32::try_from(keep_alive).expect("a u32"),
        );
        assert_eq!(got, ms(&case["expect_ms"], "expect_ms"), "{}", case["note"]);
    }
    let cases = doc["cases"].as_array().expect("cases");
    assert!(!cases.is_empty());
    for case in cases {
        let q = &case["qos"];
        let m = &case["miss_detection"];
        let st = &case["reader_stall"];
        let d = Delivery {
            reliability: match q["reliability"].as_str().expect("reliability") {
                "reliable" => Reliability::Reliable,
                "best_effort" => Reliability::BestEffort,
                other => panic!("reliability {other:?}"),
            },
            congestion: match q["congestion"].as_str().expect("congestion") {
                "block" => Congestion::Block,
                "drop" => Congestion::Drop,
                other => panic!("congestion {other:?}"),
            },
            reader_lost: case["reader_lost"].as_u64().expect("reader_lost"),
            stall: ReaderStall {
                gap: ms(&st["gap_ms"], "gap_ms"),
                bound: ms(&st["bound_ms"], "bound_ms"),
                transport_closed: st["transport_closed"].as_bool().expect("transport_closed"),
            },
            miss_detection: MissDetection {
                declared: m["declared"].as_bool().expect("declared"),
                used: m["used"].as_bool().expect("used"),
                misses: m["misses"].as_u64().expect("misses"),
            },
        };
        let why = lossless(&d);
        assert_eq!(
            case["expect"],
            json!({"lossless": why.is_lossless(), "reason": why.as_str()}),
            "{}",
            case["note"]
        );
    }
}

/// `spec/conformance/seed/merge.json` (core §4.2, 0.26; 0.27, #752): how
/// two stamps compare — time, then the id's bytes as zenoh compares them —
/// and which of one key's arrivals a late joiner keeps, unstamped ones
/// ranked below every stamp. Written by hand from the text, checked, never
/// blessed.
#[test]
fn seed_merge() {
    use std::cmp::Ordering;
    use zenkey_model::seed::{Stamp, merge};
    let doc = read_json(&spec().join("conformance/seed/merge.json"));
    let stamp = |v: &Value| -> Option<Stamp> {
        if v.is_null() {
            return None;
        }
        let time = v["time"].as_u64().expect("time");
        let id = v["id"].as_str().expect("id");
        Some(Stamp::parse(time, id).unwrap_or_else(|| panic!("{id:?} is an id")))
    };
    let order = doc["order"].as_array().expect("order");
    assert!(!order.is_empty());
    for case in order {
        let (a, b) = (stamp(&case["a"]), stamp(&case["b"]));
        let got = match a.expect("a").cmp(&b.expect("b")) {
            Ordering::Less => "less",
            Ordering::Equal => "equal",
            Ordering::Greater => "greater",
        };
        assert_eq!(case["expect"], json!(got), "{}", case["note"]);
    }
    let cases = doc["cases"].as_array().expect("cases");
    assert!(!cases.is_empty());
    for case in cases {
        let arrivals: Vec<Option<Stamp>> = case["arrivals"]
            .as_array()
            .expect("arrivals")
            .iter()
            .map(|a| {
                assert!(
                    matches!(a["kind"].as_str(), Some("put" | "delete")),
                    "{}",
                    case["note"]
                );
                stamp(&a["stamp"])
            })
            .collect();
        assert_eq!(
            case["expect"],
            json!({"kept": merge(&arrivals)}),
            "{}",
            case["note"]
        );
    }
}

/// `spec/conformance/seed/inband.json` (core §4.2, §4.3, 0.73; #976): a
/// key's kind and two in-band stamps → their order, plain for state and
/// events and serial for a stream; one key's arrivals, each with an in-band
/// stamp, a Zenoh timestamp, both or neither → which a tool's merge keeps
/// and, for a state key, which a consumer accepts. Written by hand from the
/// text, checked, never blessed.
#[test]
fn seed_inband() {
    use zenkey_model::grammar::KindToken;
    use zenkey_model::seed::{
        Held, Mark, Order, Stamp, Versus, admits_mark_in, consumer_accepts, versus_in,
    };
    use zenkey_model::wire::{self, Run, Seq};
    let doc = read_json(&spec().join("conformance/seed/inband.json"));
    let inband = |v: &Value| -> Option<wire::Stamp> {
        if v.is_null() {
            return None;
        }
        let run = v["run"].as_u64().expect("run");
        let run = match v["width"].as_u64() {
            Some(16) => Run::U16(u16::try_from(run).expect("a 16-bit run")),
            Some(32) => Run::U32(u32::try_from(run).expect("a 32-bit run")),
            w => panic!("width {w:?}"),
        };
        let seq = u32::try_from(v["seq"].as_u64().expect("seq")).expect("a seq");
        Some(wire::Stamp {
            run,
            seq: Seq::new(seq).unwrap_or_else(|| panic!("{seq} is a seq")),
        })
    };
    let zenoh = |v: Option<&Value>| -> Option<Stamp> {
        let v = v?;
        let time = v["time"].as_u64().expect("time");
        let id = v["id"].as_str().expect("id");
        Some(Stamp::parse(time, id).unwrap_or_else(|| panic!("{id:?} is an id")))
    };
    let order_of = |case: &Value| -> (Order, KindToken) {
        let kind: KindToken = case["kind"]
            .as_str()
            .expect("kind")
            .parse()
            .expect("a kind token");
        assert_ne!(
            kind,
            KindToken::Op,
            "{}: an operation carries no envelope",
            case["note"]
        );
        (Order::of_kind(kind), kind)
    };
    let pairs = doc["order"].as_array().expect("order");
    assert!(!pairs.is_empty());
    let mut seen = std::collections::BTreeSet::new();
    for case in pairs {
        let (order, kind) = order_of(case);
        let a = Mark::InBand(inband(&case["a"]).expect("a"));
        let b = Mark::InBand(inband(&case["b"]).expect("b"));
        let got = match versus_in(order, a, b) {
            Versus::Newer => "less",
            Versus::Same => "equal",
            Versus::Older => "greater",
            Versus::Unordered => "unordered",
        };
        assert_eq!(case["expect"], json!(got), "{}", case["note"]);
        seen.insert((kind.as_str(), got));
    }
    for want in [
        ("state", "unordered"),
        ("stream", "unordered"),
        ("stream", "less"),
        ("state", "less"),
    ] {
        assert!(seen.contains(&want), "the order pairs cover {want:?}");
    }
    let cases = doc["cases"].as_array().expect("cases");
    assert!(!cases.is_empty());
    for case in cases {
        let note = &case["note"];
        let (order, kind) = order_of(case);
        let marks: Vec<Option<Mark>> = case["arrivals"]
            .as_array()
            .expect("arrivals")
            .iter()
            .map(|a| {
                assert!(
                    matches!(a["sample"].as_str(), Some("put" | "delete")),
                    "{note}"
                );
                Mark::of(inband(&a["stamp"]), zenoh(a.get("zenoh")))
            })
            .collect();
        let mut held = Held::Nothing;
        let tool: Vec<bool> = marks
            .iter()
            .map(|m| {
                let kept = admits_mark_in(order, held, *m);
                if kept {
                    held = held.after_mark(*m);
                }
                kept
            })
            .collect();
        assert_eq!(
            case["expect"]["tool_kept"],
            json!(tool),
            "{note}: a tool's merge"
        );
        let state = matches!(kind, KindToken::State | KindToken::ExplicitState);
        let every_marked = marks.iter().all(Option::is_some);
        let want = &case["expect"]["consumer_accepts"];
        if state && every_marked {
            let mut last: Option<Mark> = None;
            let consumer: Vec<bool> = marks
                .iter()
                .map(|m| {
                    let m = m.expect("marked");
                    let ok = consumer_accepts(last, m);
                    if ok {
                        last = Some(m);
                    }
                    ok
                })
                .collect();
            assert_eq!(*want, json!(consumer), "{note}: a consumer");
        } else {
            assert!(
                want.is_null(),
                "{note}: a consumer's rule orders a state key's stamped values"
            );
        }
    }
}

/// Bytes from lowercase hexadecimal, two digits a byte.
fn unhex(h: &str) -> Vec<u8> {
    assert!(h.len().is_multiple_of(2), "{h:?}: odd length");
    (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or_else(|e| panic!("{h:?}: {e}")))
        .collect()
}

/// `spec/conformance/envelope/carries.json` (core §7.5, observer §7.3,
/// 0.14; #976): a key as a session hears it and the sample's Encoding →
/// whether the sample carries a zk2c envelope, by `wire::carries`. Written
/// by hand from the texts, checked, never blessed.
#[test]
fn envelope_carries() {
    let doc = read_json(&spec().join("conformance/envelope/carries.json"));
    let cases = doc["cases"].as_array().expect("cases");
    assert!(!cases.is_empty());
    let mut both = [false, false];
    for case in cases {
        let note = &case["note"];
        let key = case["key"].as_str().expect("key");
        let encoding = case["encoding"].as_str().expect("encoding");
        let expect = case["expect"].as_bool().expect("expect");
        both[usize::from(expect)] = true;
        assert_eq!(
            zenkey_model::wire::carries(key, encoding),
            expect,
            "{note}: {key} under {encoding}"
        );
    }
    assert_eq!(both, [true, true], "cases on both sides");
    // A key and a put's payload under `zenoh/bytes` → how the put reads
    // (core §7.5): plain, its form, or the refusal, `key_kind` among them.
    let reads = doc["reads"].as_array().expect("reads");
    assert!(!reads.is_empty());
    let mut seen = std::collections::BTreeSet::new();
    for case in reads {
        use zenkey_model::wire::{ENCODING, Reading, read_put};
        let note = &case["note"];
        let key = case["key"].as_str().expect("key");
        let bytes = unhex(case["hex"].as_str().expect("hex"));
        let got = match read_put(key, ENCODING, &bytes) {
            Reading::Plain => json!({"reads": "plain"}),
            Reading::Enveloped(e)
            | Reading::Framed(e)
            | Reading::Heartbeat(e)
            | Reading::Snapshot(e) => {
                assert_eq!(e.write(), bytes, "{note}: writes back");
                json!({"reads": e.form.as_str()})
            }
            Reading::Refused(r) => json!({"refused": r.token()}),
        };
        seen.insert(got.to_string());
        assert_eq!(case["expect"], got, "{note}: {key}");
    }
    assert!(
        seen.contains(r#"{"refused":"key_kind"}"#),
        "a key_kind case"
    );
    assert!(
        seen.contains(r#"{"reads":"snapshot"}"#),
        "a snapshot read on a state key"
    );
}

/// Bytes in lowercase hexadecimal, two digits a byte.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// An error form's stamp, tag and error, as `errors/forms.json`'s `writes`
/// and `reads` spell them.
fn error_form_json(e: &zenkey_model::envelope::ErrorForm<'_>) -> Value {
    json!({
        "run_width": if e.stamp.run.is_wide() { 32 } else { 16 },
        "run": e.stamp.run.value(),
        "seq": e.stamp.seq.get(),
        "tag": e.tag.get(),
        "code": e.code.as_str(),
        "cause": e.cause.map(|c| c.as_str()),
        "message": e.message,
        "detail": e.detail.map(hex),
    })
}

/// `spec/conformance/envelope/vectors.json` (core §7.5, 0.72; #976; a
/// call's forms, 0.92; a snapshot's envelope, 0.97): CH's three fields from one byte, and an envelope's
/// or a form's bytes with its sample → the parse or the refusal's token,
/// the first in §7.5's order. Every header and every accepted case writes
/// back to its bytes. Written by hand from the text, checked, never
/// blessed.
#[test]
fn envelope_vectors() {
    use zenkey_model::wire::{
        DeleteStamp, Envelope, Header, Stamp, read_query, read_reply, read_reply_err,
    };
    let doc = read_json(&spec().join("conformance/envelope/vectors.json"));
    let width = |h: Header| if h.wide { 32 } else { 16 };
    let parsed = |h: Header, s: Stamp, record: Value| {
        json!({
            "form": h.form.as_str(),
            "run_width": width(h),
            "run": s.run.value(),
            "seq": s.seq.get(),
            "tag": h.tag.get(),
            "record": record,
        })
    };
    let headers = doc["headers"].as_array().expect("headers");
    assert!(!headers.is_empty());
    for case in headers {
        let note = &case["note"];
        let byte = unhex(case["byte"].as_str().expect("byte"));
        assert_eq!(byte.len(), 1, "{note}");
        let h = Header::from_byte(byte[0]);
        assert_eq!(
            case["expect"],
            json!({"form": h.form.as_str(), "run_width": width(h), "tag": h.tag.get()}),
            "{note}"
        );
        assert_eq!(h.byte(), byte[0], "{note}: writes back");
    }
    let cases = doc["cases"].as_array().expect("cases");
    assert!(!cases.is_empty());
    let mut samples = std::collections::BTreeSet::new();
    for case in cases {
        let note = &case["note"];
        let bytes = unhex(case["hex"].as_str().expect("hex"));
        let sample = case["sample"].as_str();
        samples.insert(sample.unwrap_or_default().to_owned());
        let got = match sample {
            Some("put") => match Envelope::parse(&bytes) {
                Ok(e) => {
                    assert_eq!(e.write(), bytes, "{note}: writes back");
                    parsed(e.header(), e.stamp, json!(hex(e.record)))
                }
                Err(r) => json!({"refused": r.token()}),
            },
            Some("delete") => match DeleteStamp::parse(&bytes) {
                Ok(d) => {
                    assert_eq!(d.write(), bytes, "{note}: writes back");
                    parsed(d.header(), d.stamp, Value::Null)
                }
                Err(r) => json!({"refused": r.token()}),
            },
            Some("query") => match read_query(&bytes) {
                Ok(q) => {
                    assert_eq!(q.write(), bytes, "{note}: writes back");
                    json!({
                        "form": q.header().form.as_str(),
                        "run_width": null,
                        "run": null,
                        "seq": null,
                        "tag": q.tag.get(),
                        "record": hex(q.record),
                    })
                }
                Err(r) => json!({"refused": r.token()}),
            },
            Some("reply") => match read_reply(&bytes) {
                Ok(v) => {
                    assert_eq!(v.write(), bytes, "{note}: writes back");
                    parsed(v.header(), v.stamp, json!(hex(v.record)))
                }
                Err(r) => json!({"refused": r.token()}),
            },
            Some("reply_err") => match read_reply_err(&bytes) {
                Ok(e) => {
                    assert_eq!(e.write().as_ref(), Ok(&bytes), "{note}: writes back");
                    let mut p = parsed(e.header(), e.stamp, Value::Null);
                    p["error"] = json!({
                        "code": e.code.as_str(),
                        "cause": e.cause.map(|c| c.as_str()),
                        "message": e.message,
                        "detail": e.detail.map(hex),
                    });
                    p
                }
                Err(r) => json!({"refused": r.token()}),
            },
            other => panic!("{note}: sample {other:?}"),
        };
        assert_eq!(case["expect"], got, "{note}");
    }
    assert_eq!(
        samples.into_iter().collect::<Vec<_>>(),
        ["delete", "put", "query", "reply", "reply_err"],
        "every sample has cases"
    );
}

/// `spec/conformance/errors/forms.json` (core 0.92 §7.5, §5.2 "The error
/// form"; #980's C4a): every error byte read alone → its fields or the
/// refusal (`bytes`); an error and its stamp → the reply error's payload,
/// its message cut, or the writer's refusal (`writes`); a reply error's
/// `Encoding` and payload → the error it carries, or the refusal (`reads`).
/// Written by an independent reading of the text, checked, never blessed.
#[test]
fn error_forms() {
    use zenkey_model::envelope::{Cause, Code, ErrorByte, ErrorForm};
    use zenkey_model::wire::{Run, Seq, Stamp, Tag, read_reply_err};
    let doc = read_json(&spec().join("conformance/errors/forms.json"));
    assert_eq!(
        doc["codes"],
        json!(Code::ALL.map(Code::as_str)),
        "§5.2's codes, in order"
    );
    let mut causes = vec![Value::Null];
    causes.extend(Cause::ALL.map(|c| json!(c.as_str())));
    assert_eq!(doc["causes"], Value::Array(causes), "none, then the causes");

    let bytes = doc["bytes"].as_array().expect("bytes");
    assert_eq!(bytes.len(), 256, "every byte");
    for (i, case) in bytes.iter().enumerate() {
        let byte = unhex(case["byte"].as_str().expect("byte"));
        assert_eq!(byte, [u8::try_from(i).unwrap()], "in order");
        let got = match ErrorByte::read(byte[0]) {
            Ok(b) => {
                assert_eq!(b.byte(), byte[0], "{i:02x}: writes back");
                json!({
                    "code": b.code.as_str(),
                    "cause": b.cause.map(|c| c.as_str()),
                    "detail": b.detail,
                    "message": b.message,
                })
            }
            Err(r) => json!({"refused": r.token()}),
        };
        assert_eq!(case["expect"], got, "byte {i:02x}");
    }

    let writes = doc["writes"].as_array().expect("writes");
    assert!(!writes.is_empty());
    for case in writes {
        let note = &case["note"];
        let v = &case["value"];
        let run = v["run"].as_u64().expect("run");
        let run = match v["run_width"].as_u64() {
            Some(16) => Run::U16(u16::try_from(run).expect("a 16-bit run")),
            Some(32) => Run::U32(u32::try_from(run).expect("a 32-bit run")),
            other => panic!("{note}: run_width {other:?}"),
        };
        let seq = Seq::new(u32::try_from(v["seq"].as_u64().expect("seq")).unwrap()).expect("seq");
        let tag = Tag::new(u8::try_from(v["tag"].as_u64().expect("tag")).unwrap()).expect("tag");
        let code = Code::from_name(v["code"].as_str().expect("code")).expect("a code");
        let cause = v["cause"]
            .as_str()
            .map(|c| Cause::from_name(c).expect("a cause"));
        let detail = v["detail"].as_str().map(unhex);
        let form = ErrorForm {
            tag,
            stamp: Stamp { run, seq },
            code,
            cause,
            message: v["message"].as_str(),
            detail: detail.as_deref(),
        };
        let got = match form.write() {
            Ok(b) => {
                // Every accepted write parses back to its value, with the
                // message as cut.
                let back = read_reply_err(&b).unwrap_or_else(|r| panic!("{note}: {r}"));
                let mut want = v.clone();
                if let Some(m) = case["expect"]
                    .get("reads_as")
                    .and_then(|r| r.get("message"))
                {
                    want["message"] = m.clone();
                }
                assert_eq!(error_form_json(&back), want, "{note}: reads back");
                let mut got = json!({"hex": hex(&b)});
                if back.message != form.message {
                    got["reads_as"] = json!({"message": back.message});
                }
                got
            }
            Err(r) => json!({"refused": r.token()}),
        };
        assert_eq!(case["expect"], got, "{note}");
    }

    let reads = doc["reads"].as_array().expect("reads");
    assert!(!reads.is_empty());
    for case in reads {
        let note = &case["note"];
        let encoding = case["encoding"].as_str().expect("encoding");
        let bytes = unhex(case["hex"].as_str().expect("hex"));
        let got = match ErrorForm::read(encoding, &bytes) {
            Ok(e) => {
                assert_eq!(e.write().as_ref(), Ok(&bytes), "{note}: writes back");
                json!({"error": error_form_json(&e)})
            }
            Err(r) => json!({"refused": r.token()}),
        };
        assert_eq!(case["expect"], got, "{note}");
    }
}

/// A stamp as the fixtures spell it: `run_width`, `run` and `seq`.
fn stamp_of(v: &Value, note: &Value) -> zenkey_model::wire::Stamp {
    use zenkey_model::wire::{Run, Seq, Stamp};
    let run = v["run"].as_u64().expect("run");
    let run = match v["run_width"].as_u64() {
        Some(16) => Run::U16(u16::try_from(run).expect("a 16-bit run")),
        Some(32) => Run::U32(u32::try_from(run).expect("a 32-bit run")),
        other => panic!("{note}: run_width {other:?}"),
    };
    let seq = Seq::new(u32::try_from(v["seq"].as_u64().expect("seq")).unwrap()).expect("seq");
    Stamp { run, seq }
}

/// `spec/conformance/envelope/calls.json` (core 0.94 §5.1, §5.2; #985's
/// C4c): the operations of `contracts/ok-packed-operation.toml`, each
/// form's CH by its operation's types' tags (`types`, `forms`); a record
/// and a stamp → a request's, a value's, a summary's or an error's bytes,
/// or the writer's refusal (`writes`); a query's, a reply's or a reply
/// error's bytes → its stamp, its tag and its record read as its type under
/// CH's tag, or the first refusal (`reads`), through
/// `zenkey_model::forms`. Written by an independent reading of the text,
/// checked, never blessed.
#[test]
fn call_forms() {
    use zenkey_model::contract::{Body, Operation};
    use zenkey_model::envelope::{Cause, Code};
    use zenkey_model::forms::{FormType, Forms};
    use zenkey_model::wire::{Stamp, Tag, read_reply, read_reply_err};
    let doc = read_json(&spec().join("conformance/envelope/calls.json"));
    let path = spec()
        .join("conformance")
        .join(doc["contract"].as_str().expect("contract"));
    let l = zenkey_model::contract::load_path(&path);
    let contract = l.contract.unwrap_or_else(|| panic!("{}", l.report));
    let epoch = doc["epoch"].as_i64().expect("epoch");
    let op = |name: &str| -> Operation {
        let r = contract
            .resources
            .iter()
            .find(|r| r.template.as_str() == name)
            .unwrap_or_else(|| panic!("no operation {name}"));
        match &r.body {
            Body::Operation(o) => o.clone(),
            Body::Data(_) => panic!("{name} is no operation"),
        }
    };
    let forms = |name: &str| Forms::of(&contract, &op(name)).expect("its forms");
    let wide = |s: Stamp| -> Value { json!(if s.run.is_wide() { 32 } else { 16 }) };
    let mark = Stamp {
        run: zenkey_model::wire::Run::U16(1),
        seq: zenkey_model::wire::Seq::new(7).unwrap(),
    };

    // Each type's tag and layout, and each form's CH with a 16-bit run.
    let types = doc["types"].as_object().expect("types");
    for case in doc["forms"].as_array().expect("forms") {
        let name = case["operation"].as_str().expect("operation");
        let f = forms(name);
        let typed = |role: &str| -> Option<&FormType> {
            match role {
                "request" => Some(&f.request),
                "response" => Some(&f.response),
                "summary" => f.summary.as_ref(),
                "error" => f.error.as_ref(),
                other => panic!("{name}: role {other}"),
            }
        };
        for (role, ty) in case["types"].as_object().expect("types") {
            let t = typed(role).unwrap_or_else(|| panic!("{name} declares no {role}"));
            let ty = ty.as_str().expect("a type");
            match types.get(ty) {
                Some(want) => {
                    let layout = t
                        .layout()
                        .unwrap_or_else(|| panic!("{name} {role}: packed"));
                    assert_eq!(json!(layout.sha256_hex()), want["sha256"], "{name} {role}");
                    assert_eq!(json!(t.tag().get()), want["tag"], "{name} {role}");
                }
                None => assert_eq!((t.tag(), t.layout()), (Tag::NONE, None), "{name} {role}"),
            }
        }
        let mut got = serde_json::Map::new();
        got.insert("request".into(), json!(hex(&f.request(&[]))));
        got.insert("value".into(), json!(hex(&f.value(mark, &[])[..1])));
        if let Some(s) = f.summary(mark, &[]) {
            got.insert("summary".into(), json!(hex(&s[..1])));
        }
        let e = f.error(mark, Code::Busy, None, None, None).expect("busy");
        got.insert("error".into(), json!(hex(&e[..1])));
        assert_eq!(case["expect"], Value::Object(got), "{name}: CH");
    }

    // A record from a case: a packed value written by its layout, or a
    // JSON Schema record's bytes.
    let record = |t: &FormType, case: &Value| -> Result<Vec<u8>, &'static str> {
        match (&case["value"], case["record"].as_str()) {
            (Value::Null, Some(h)) => Ok(unhex(h)),
            (v, None) => t
                .layout()
                .expect("a packed value")
                .write(v, Some(epoch))
                .map(|r| r.bytes)
                .map_err(|r| r.token()),
            _ => panic!("a value or a record"),
        }
    };
    let writes = doc["writes"].as_array().expect("writes");
    assert!(!writes.is_empty());
    for case in writes {
        let note = &case["note"];
        let f = forms(case["operation"].as_str().expect("operation"));
        let got = match case["form"].as_str().expect("form") {
            "request" => record(&f.request, case).map(|r| f.request(&r)),
            "value" => {
                let s = stamp_of(&case["stamp"], note);
                record(&f.response, case).map(|r| f.value(s, &r))
            }
            "summary" => {
                let s = stamp_of(&case["stamp"], note);
                let t = f.summary.clone().expect("a summary");
                record(&t, case).map(|r| f.summary(s, &r).expect("declared"))
            }
            "error" => {
                let s = stamp_of(&case["stamp"], note);
                let e = &case["error"];
                let code = Code::from_name(e["code"].as_str().expect("code")).expect("a code");
                let cause = e["cause"]
                    .as_str()
                    .map(|c| Cause::from_name(c).expect("a cause"));
                let detail = match &e["detail"] {
                    Value::Null => Ok(None),
                    Value::String(h) => Ok(Some(unhex(h))),
                    v => f
                        .error
                        .as_ref()
                        .and_then(FormType::layout)
                        .expect("a packed error type")
                        .write(v, Some(epoch))
                        .map(|r| Some(r.bytes))
                        .map_err(|r| r.token()),
                };
                detail.and_then(|d| {
                    f.error(s, code, cause, e["message"].as_str(), d.as_deref())
                        .map_err(|r| r.token())
                })
            }
            other => panic!("{note}: form {other}"),
        };
        let got = match got {
            Ok(b) => json!({"hex": hex(&b)}),
            Err(token) => json!({"refused": token}),
        };
        assert_eq!(case["expect"], got, "{note}");
    }

    // A record read as its type under CH's tag: a packed one by its
    // layout, a JSON Schema one on the contract's encoding, JSON here.
    let value = |t: &FormType, tag: Tag, record: &[u8]| -> Result<Value, &'static str> {
        match t.layout() {
            Some(l) => l
                .read(tag.get(), record, Some(epoch))
                .map_err(|r| r.token()),
            None => Ok(serde_json::from_slice(record).expect("a JSON record")),
        }
    };
    let or_refused = |v: Result<Value, &'static str>| v.unwrap_or_else(|t| json!({"refused": t}));
    let reads = doc["reads"].as_array().expect("reads");
    assert!(!reads.is_empty());
    for case in reads {
        let note = &case["note"];
        let f = forms(case["operation"].as_str().expect("operation"));
        let bytes = unhex(case["hex"].as_str().expect("hex"));
        let got = match case["sample"].as_str().expect("sample") {
            "query" => match f.read_request(&bytes) {
                Ok(r) => {
                    assert_eq!(f.request(r.record), bytes, "{note}: writes back");
                    json!({
                        "run_width": null, "run": null, "seq": null,
                        "tag": r.tag.get(),
                        "value": or_refused(value(&f.request, r.tag, r.record)),
                    })
                }
                Err(r) => json!({"refused": r.token()}),
            },
            "reply" => {
                let summary = case["summary"].as_bool().unwrap_or(false);
                let t = if summary {
                    f.summary.clone().expect("a summary")
                } else {
                    f.response.clone()
                };
                match read_reply(&bytes) {
                    Ok(e) => match value(&t, e.tag, e.record) {
                        Err(token) => json!({"refused": token}),
                        Ok(v) => json!({
                            "run_width": wide(e.stamp),
                            "run": e.stamp.run.value(),
                            "seq": e.stamp.seq.get(),
                            "tag": e.tag.get(),
                            "value": v,
                        }),
                    },
                    Err(r) => json!({"refused": r.token()}),
                }
            }
            "reply_err" => match read_reply_err(&bytes) {
                Ok(e) => {
                    let detail = e.detail.map(|d| {
                        let t = f.error.as_ref().expect("an error type");
                        or_refused(value(t, e.tag, d))
                    });
                    json!({
                        "run_width": wide(e.stamp),
                        "run": e.stamp.run.value(),
                        "seq": e.stamp.seq.get(),
                        "tag": e.tag.get(),
                        "error": {
                            "code": e.code.as_str(),
                            "cause": e.cause.map(|c| c.as_str()),
                            "message": e.message,
                            "detail": detail,
                        },
                    })
                }
                Err(r) => json!({"refused": r.token()}),
            },
            other => panic!("{note}: sample {other}"),
        };
        assert_eq!(case["expect"], got, "{note}");
    }
}

/// The heartbeat fixtures' revisions, from `layouts.json`'s authoring
/// texts, as layouts by name.
fn heartbeat_layouts() -> BTreeMap<String, zenkey_model::heartbeat::Layout> {
    let doc = read_json(&spec().join("conformance/heartbeat/layouts.json"));
    doc["contracts"]
        .as_object()
        .expect("contracts")
        .iter()
        .map(|(name, text)| {
            let loaded = load_str(text.as_str().expect("text"), &spec(), None);
            let c = loaded
                .contract
                .unwrap_or_else(|| panic!("{name}: {:?}", loaded.report));
            (name.clone(), zenkey_model::heartbeat::Layout::of(&c))
        })
        .collect()
}

/// `spec/conformance/heartbeat/layouts.json` (core §7.5, 0.81; #978 F1): a
/// revision → its fingerprint, the heartbeat's tag, and its collections in
/// map order with their cardinalities, live-count widths and horizons.
/// Written by an independent reading of the text, checked, never blessed.
#[test]
fn heartbeat_layout_vectors() {
    let doc = read_json(&spec().join("conformance/heartbeat/layouts.json"));
    let layouts = heartbeat_layouts();
    let cases = doc["layouts"].as_array().expect("layouts");
    assert_eq!(cases.len(), layouts.len(), "a layout per contract");
    for case in cases {
        let name = case["contract"].as_str().expect("contract");
        let l = &layouts[name];
        let got = json!({
            "contract": name,
            "fingerprint": l.fingerprint.to_string(),
            "tag": l.tag.get(),
            "collections": l.collections.iter().map(|c| json!({
                "resource": c.resource(),
                "cardinality": c.cardinality,
                "live_bits": c.live_bits(),
                "ttl_s": c.ttl.as_secs(),
            })).collect::<Vec<_>>(),
        });
        assert_eq!(case, &got, "{name}");
    }
}

/// A heartbeat value as `bodies.json` writes it: the time, and each
/// section by its collection's resource.
fn heartbeat_value(
    hb: &zenkey_model::heartbeat::Heartbeat,
    l: &zenkey_model::heartbeat::Layout,
) -> Value {
    use zenkey_model::heartbeat::{Members, Time};
    let time = match hb.time {
        Time::Unknown => Value::Null,
        Time::At { secs, ms } => json!({"s": secs, "ms": ms}),
    };
    let mut sections = serde_json::Map::new();
    for (c, s) in l.collections.iter().zip(&hb.sections) {
        let Some(s) = s else { continue };
        let v = match &s.members {
            Members::One { vouched } => json!({"digest": s.digest.to_string(), "vouched": vouched}),
            m => json!({
                "digest": s.digest.to_string(),
                "live": m.live(),
                "vouched": m.vouched(),
                "form": m.form(),
            }),
        };
        sections.insert(c.resource(), v);
    }
    json!({"time": time, "sections": sections})
}

/// `bodies.json`'s value read back into a heartbeat: the writer's choice
/// of form where the value names none ([`Members::templated`]).
fn heartbeat_of(
    v: &Value,
    l: &zenkey_model::heartbeat::Layout,
) -> zenkey_model::heartbeat::Heartbeat {
    use zenkey_model::heartbeat::{Digest, Heartbeat, Members, Section, Time};
    let time = match &v["time"] {
        Value::Null => Time::Unknown,
        t => Time::At {
            secs: t["s"].as_u64().expect("s") as u32,
            ms: t["ms"].as_u64().expect("ms") as u16,
        },
    };
    let sections = l
        .collections
        .iter()
        .map(|c| {
            let s = v["sections"].get(c.resource())?;
            let digest = Digest::from_u32(
                u32::from_str_radix(s["digest"].as_str().expect("digest"), 16).expect("hex"),
            );
            let members = if c.is_templated() {
                let live = s["live"].as_u64().expect("live");
                let ranks: Vec<u64> = s["vouched"]
                    .as_array()
                    .expect("vouched")
                    .iter()
                    .map(|r| r.as_u64().expect("rank"))
                    .collect();
                match s.get("form").and_then(Value::as_str) {
                    None => Members::templated(live, ranks),
                    Some("list") => Members::List { live, ranks },
                    Some("bitmap") => {
                        Members::Bitmap((0..live).map(|i| ranks.contains(&i)).collect())
                    }
                    Some(other) => panic!("form {other}"),
                }
            } else {
                Members::One {
                    vouched: s["vouched"].as_bool().expect("vouched"),
                }
            };
            Some(Section { digest, members })
        })
        .collect();
    Heartbeat { time, sections }
}

/// `spec/conformance/heartbeat/bodies.json` (core §7.5, 0.81; #978 F1): a
/// layout and a value → the body's bits, pad and bytes, the list written
/// only where it is shorter; a layout and bytes → the value, or the
/// refusal, the first in §7.5's order. Every body read writes back to its
/// bytes. Written by an independent reading of the text, checked, never
/// blessed.
#[test]
fn heartbeat_body_vectors() {
    use zenkey_model::heartbeat::Heartbeat;
    let doc = read_json(&spec().join("conformance/heartbeat/bodies.json"));
    let layouts = heartbeat_layouts();
    let hex = |b: &[u8]| -> String { b.iter().map(|x| format!("{x:02x}")).collect() };
    let writes = doc["writes"].as_array().expect("writes");
    assert!(!writes.is_empty());
    for case in writes {
        let note = &case["note"];
        let l = &layouts[case["layout"].as_str().expect("layout")];
        let r = heartbeat_of(&case["value"], l).record(l).expect("writes");
        let got = json!({"bits": r.bit_string(), "pad": r.pad(), "hex": hex(&r.bytes)});
        assert_eq!(case["expect"], got, "{note}");
    }
    let reads = doc["reads"].as_array().expect("reads");
    assert!(!reads.is_empty());
    let mut refused = std::collections::BTreeSet::new();
    for case in reads {
        let note = &case["note"];
        let l = &layouts[case["layout"].as_str().expect("layout")];
        let bytes = unhex(case["hex"].as_str().expect("hex"));
        let got = match Heartbeat::parse(&bytes, l) {
            Ok(hb) => {
                assert_eq!(
                    hb.write(l).expect("writes back"),
                    bytes,
                    "{note}: writes back"
                );
                heartbeat_value(&hb, l)
            }
            Err(r) => {
                refused.insert(r.token());
                json!({"refused": r.token()})
            }
        };
        assert_eq!(case["expect"], got, "{note}");
    }
    assert_eq!(
        refused.into_iter().collect::<Vec<_>>(),
        ["code", "pad", "short", "trailing"],
        "every refusal pinned"
    );
}

/// `spec/conformance/heartbeat/digests.json` (core §7.5, 0.81; #978 F1): a
/// collection's live members → each one's value and their digest. Written
/// by an independent reading of the text, checked, never blessed.
#[test]
fn heartbeat_digest_vectors() {
    use zenkey_model::heartbeat::Digest;
    use zenkey_model::wire::{Run, Seq, Stamp};
    let doc = read_json(&spec().join("conformance/heartbeat/digests.json"));
    let cases = doc["cases"].as_array().expect("cases");
    assert!(!cases.is_empty());
    for case in cases {
        let note = &case["note"];
        let members: Vec<(String, Stamp)> = case["members"]
            .as_array()
            .expect("members")
            .iter()
            .map(|m| {
                let run = m["run"].as_u64().expect("run");
                let run = match m["run_width"].as_u64() {
                    Some(16) => Run::U16(run as u16),
                    Some(32) => Run::U32(run as u32),
                    w => panic!("{note}: width {w:?}"),
                };
                let seq = Seq::new(m["seq"].as_u64().expect("seq") as u32).expect("a seq");
                (
                    m["key"].as_str().expect("key").to_owned(),
                    Stamp { run, seq },
                )
            })
            .collect();
        let values: Vec<String> = members
            .iter()
            .map(|(k, s)| format!("{:08x}", Digest::member(k, *s)))
            .collect();
        let digest = Digest::of(members.iter().map(|(k, s)| (k.as_str(), *s)));
        assert_eq!(
            case["expect"],
            json!({"values": values, "digest": digest.to_string()}),
            "{note}"
        );
    }
}

/// An instant in milliseconds since the Unix epoch in RFC 3339, UTC, to
/// the millisecond: the civil date by days from 1970-01-01 (H. Hinnant's
/// `civil_from_days`), so `times.json`'s `utc` is checked against its
/// `at` and nothing else.
fn utc_of(ms: i128) -> String {
    let (days, ms) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i128::from(m <= 2);
    let (h, min, s, ms) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{min:02}:{s:02}.{ms:03}Z")
}

/// `spec/conformance/heartbeat/times.json` (core §7.5, §7.6, §8.6;
/// `freshness.v2` §2.4; #1080): a deployment epoch and an instant → the
/// time `heartbeat::Time::from_unix_ms` gives, the unknown time where the
/// instant has no code (before the epoch, or 2^32 s or more after it) or
/// the clock guard holds, and `bare.v1`'s body; an epoch and bytes → the
/// instant `Time::unix_ms` reads, or the refusal. No epoch given is E = 0,
/// as an owner with no catalog counts from the Unix epoch. Every known
/// write reads back to its `at`. Written by an independent reading of the
/// text, checked by a second, never blessed.
#[test]
fn heartbeat_time_vectors() {
    use zenkey_model::catalog::EPOCH_WITHOUT_CATALOG;
    use zenkey_model::heartbeat::{Heartbeat, Time};
    let doc = read_json(&spec().join("conformance/heartbeat/times.json"));
    let layouts = heartbeat_layouts();
    let hex = |b: &[u8]| -> String { b.iter().map(|x| format!("{x:02x}")).collect() };
    let epoch_of = |case: &Value| match &case["epoch"] {
        Value::Null => EPOCH_WITHOUT_CATALOG,
        e => e.as_i64().expect("epoch"),
    };
    let time_json = |t: Time| match t {
        Time::Unknown => Value::Null,
        Time::At { secs, ms } => json!({"s": secs, "ms": ms}),
    };
    let writes = doc["writes"].as_array().expect("writes");
    assert!(!writes.is_empty());
    for case in writes {
        let note = &case["note"];
        let l = &layouts[case["layout"].as_str().expect("layout")];
        let epoch = epoch_of(case);
        let time = match &case["at"] {
            Value::Null => Time::Unknown,
            at => {
                let at = i128::from(at.as_i64().expect("at"));
                assert_eq!(case["utc"], json!(utc_of(at)), "{note}: utc is at");
                let t = Time::from_unix_ms(at, epoch).unwrap_or(Time::Unknown);
                if t != Time::Unknown {
                    assert_eq!(t.unix_ms(epoch), Some(at), "{note}: reads back");
                }
                t
            }
        };
        let r = Heartbeat {
            time,
            sections: vec![None; l.collections.len()],
        }
        .record(l)
        .expect("writes");
        let got = json!({
            "time": time_json(time),
            "bits": r.bit_string(),
            "pad": r.pad(),
            "hex": hex(&r.bytes),
        });
        assert_eq!(case["expect"], got, "{note}");
    }
    let reads = doc["reads"].as_array().expect("reads");
    assert!(!reads.is_empty());
    for case in reads {
        let note = &case["note"];
        let l = &layouts[case["layout"].as_str().expect("layout")];
        let epoch = epoch_of(case);
        let bytes = unhex(case["hex"].as_str().expect("hex"));
        let got = match Heartbeat::parse(&bytes, l) {
            Ok(hb) => match hb.time.unix_ms(epoch) {
                None => json!({"at": null}),
                Some(at) => json!({"at": i64::try_from(at).expect("at"), "utc": utc_of(at)}),
            },
            Err(r) => json!({"refused": r.token()}),
        };
        assert_eq!(case["expect"], got, "{note}");
    }
}

/// `spec/conformance/packed/vectors.json` (core §7.6, 0.75; #979 B1): each
/// type's layout form, its sha256 and tag, each item's width and start, and
/// the shortest and longest record; a value → its bits, pad and bytes, or
/// the writer's refusal; a tag and bytes → the JSON reading, or the
/// reader's refusal, the first in §7.6's order. Every accepted write reads
/// back. Written by an independent reading of the text, checked, never
/// blessed.
#[test]
fn packed_vectors() {
    use zenkey_model::packed::PackedDoc;
    let doc = read_json(&spec().join("conformance/packed/vectors.json"));
    let read = PackedDoc::read(&doc["schema"]);
    assert!(read.findings.is_empty(), "{:?}", read.findings);
    let layout = |name: &Value| {
        read.types[name.as_str().expect("type")]
            .as_ref()
            .unwrap_or_else(|| panic!("{name}: no layout"))
    };
    let hex = |b: &[u8]| -> String { b.iter().map(|x| format!("{x:02x}")).collect() };
    let epoch = |case: &Value| case.get("epoch").and_then(Value::as_i64);
    let layouts = doc["layouts"].as_array().expect("layouts");
    assert_eq!(layouts.len(), read.types.len(), "a layout case per type");
    for case in layouts {
        let l = layout(&case["type"]);
        let items: Vec<Value> = l
            .slots()
            .into_iter()
            .map(|s| match s.name {
                Some(name) => json!({"name": name, "width": s.width,
                                     "optional": s.optional, "offset": s.offset}),
                None => json!({"marker": true, "offset": s.offset}),
            })
            .collect();
        let got = json!({
            "form": l.form(),
            "sha256": l.sha256_hex(),
            "tag": l.tag(),
            "items": items,
            "min_bits": l.min_bits(),
            "max_bits": l.max_bits(),
        });
        assert_eq!(case["expect"], got, "layout of {}", case["type"]);
    }
    let writes = doc["writes"].as_array().expect("writes");
    assert!(!writes.is_empty());
    for case in writes {
        let note = &case["note"];
        let l = layout(&case["type"]);
        let got = match l.write(&case["value"], epoch(case)) {
            Ok(r) => {
                if case.get("reads_back") != Some(&Value::Bool(false)) {
                    let want = case.get("reads_as").unwrap_or(&case["value"]);
                    assert_eq!(
                        l.read(l.tag(), &r.bytes, epoch(case)).as_ref(),
                        Ok(want),
                        "{note}: reads back"
                    );
                }
                json!({"hex": hex(&r.bytes), "bits": r.bit_string(), "pad": r.pad()})
            }
            Err(e) => json!({"refused": e.token()}),
        };
        assert_eq!(case["expect"], got, "write: {note}");
    }
    let reads = doc["reads"].as_array().expect("reads");
    assert!(!reads.is_empty());
    let mut refused = std::collections::BTreeSet::new();
    for case in reads {
        let note = &case["note"];
        let l = layout(&case["type"]);
        let tag = u8::try_from(case["tag"].as_u64().expect("tag")).expect("a 4-bit tag");
        let bytes = unhex(case["hex"].as_str().expect("hex"));
        let got = match l.read(tag, &bytes, epoch(case)) {
            Ok(v) => json!({"value": v}),
            Err(e) => {
                refused.insert(e.token());
                json!({"refused": e.token()})
            }
        };
        assert_eq!(case["expect"], got, "read: {note}");
    }
    // Every refusal of §7.6's reader is exercised, a wrong revision's by its
    // tag and by its length among them.
    for token in ["tag", "length", "code", "extension", "pad"] {
        assert!(refused.contains(token), "no read case refuses with {token}");
    }
}

/// `spec/conformance/frame/layouts.json` (core §7.7, 0.83; #981 K1): a
/// record type of the shared packed document and a `frame` table → the
/// frame form, its sha256 and tag, the count's and the offset's widths,
/// and the span. Written by an independent reading of the text, checked,
/// never blessed.
#[test]
fn frame_layouts() {
    let doc = read_json(&spec().join("conformance/frame/layouts.json"));
    for (name, l) in frame_fixture_layouts() {
        let case = doc["layouts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap()
            .clone();
        let got = json!({
            "form": l.form(),
            "sha256": l.sha256_hex(),
            "tag": l.tag().get(),
            "count_bits": l.count_bits(),
            "offset_bits": l.offset_bits(),
            "span_ms": l.span_ms(),
        });
        assert_eq!(case["expect"], got, "layout {name}");
    }
}

/// Every layout of `frame/layouts.json`, by name, through the model:
/// the packed document read, each table resolved as a contract's is.
fn frame_fixture_layouts() -> BTreeMap<String, zenkey_model::frame::Layout> {
    use zenkey_model::authoring::FrameSpec;
    use zenkey_model::frame::{FrameParams, Layout};
    use zenkey_model::packed::PackedDoc;
    let doc = read_json(&spec().join("conformance/frame/layouts.json"));
    let read = PackedDoc::read(&doc["schema"]);
    assert!(read.findings.is_empty(), "{:?}", read.findings);
    let mut out = BTreeMap::new();
    for case in doc["layouts"].as_array().expect("layouts") {
        let name = case["name"].as_str().expect("name").to_owned();
        let record = read.types[case["type"].as_str().expect("type")]
            .as_ref()
            .unwrap_or_else(|| panic!("{name}: no record layout"));
        let spec: FrameSpec = serde_json::from_value(case["frame"].clone()).expect("a frame table");
        let params = FrameParams::resolve(&spec).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let l = Layout::of(record, &params).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(out.insert(name, l).is_none(), "a layout's name once");
    }
    assert!(!out.is_empty());
    out
}

/// `spec/conformance/frame/bodies.json` (core §7.7, 0.83; #981 K1): a
/// layout and a frame's value → the body's bits, pad and bytes, or the
/// writer's refusal, each accepted write read back; a layout, a tag and a
/// body → the value, or the reader's refusal, the first in §7.7's order.
/// Every accepted body writes back to its bytes. Written by an independent
/// reading of the text, checked, never blessed.
#[test]
fn frame_bodies() {
    use zenkey_model::frame::Frame;
    use zenkey_model::packed::DEFAULT_EPOCH;
    use zenkey_model::wire::Tag;
    let layouts = frame_fixture_layouts();
    let doc = read_json(&spec().join("conformance/frame/bodies.json"));
    let hex = |b: &[u8]| -> String { b.iter().map(|x| format!("{x:02x}")).collect() };
    let epoch = Some(DEFAULT_EPOCH);
    let mut refused = std::collections::BTreeSet::new();
    let writes = doc["writes"].as_array().expect("writes");
    assert!(!writes.is_empty());
    for case in writes {
        let note = &case["note"];
        let l = &layouts[case["layout"].as_str().expect("layout")];
        let got = match Frame::from_json(&case["value"], l, epoch).and_then(|f| f.write(l)) {
            Ok(r) => {
                let back = Frame::parse(l.tag(), &r.bytes, l)
                    .unwrap_or_else(|e| panic!("{note}: reads back: {e}"));
                let want = case.get("reads_as").unwrap_or(&case["value"]);
                assert_eq!(
                    &back.read_json(l, epoch).unwrap(),
                    want,
                    "{note}: reads back"
                );
                assert_eq!(back.write(l).unwrap().bytes, r.bytes, "{note}: writes back");
                json!({"hex": hex(&r.bytes), "bits": r.bit_string(), "pad": r.pad()})
            }
            Err(e) => {
                refused.insert(e.token());
                json!({"refused": e.token()})
            }
        };
        assert_eq!(case["expect"], got, "write: {note}");
    }
    for token in ["records", "missing", "unknown", "value", "range", "order"] {
        assert!(
            refused.contains(token),
            "no write case refuses with {token}"
        );
    }
    let reads = doc["reads"].as_array().expect("reads");
    assert!(!reads.is_empty());
    let mut refused = std::collections::BTreeSet::new();
    for case in reads {
        let note = &case["note"];
        let l = &layouts[case["layout"].as_str().expect("layout")];
        let tag = Tag::new(u8::try_from(case["tag"].as_u64().expect("tag")).unwrap())
            .expect("a 4-bit tag");
        let bytes = unhex(case["hex"].as_str().expect("hex"));
        let got = match Frame::parse(tag, &bytes, l) {
            Ok(f) => {
                assert_eq!(f.write(l).unwrap().bytes, bytes, "{note}: writes back");
                f.read_json(l, epoch).unwrap()
            }
            Err(e) => {
                refused.insert(e.token());
                json!({"refused": e.token()})
            }
        };
        assert_eq!(case["expect"], got, "read: {note}");
    }
    for token in ["tag", "length", "code", "order", "pad"] {
        assert!(refused.contains(token), "no read case refuses with {token}");
    }
}

/// `spec/conformance/frame/apply.json` (core §7.7's "The reader's
/// fields"; #981 K2): one key's frames, each its run, seq and body, read
/// by one `frame::Reader` that held nothing before the first → each frame
/// as it reads it: what it follows, each record's seq and time, and every
/// field's state. Written by an independent reading of the text, checked,
/// never blessed.
#[test]
fn frame_apply() {
    use zenkey_model::frame::{Frame, Reader};
    use zenkey_model::packed::DEFAULT_EPOCH;
    use zenkey_model::wire::{Run, Seq, Stamp};
    let layouts = frame_fixture_layouts();
    let doc = read_json(&spec().join("conformance/frame/apply.json"));
    let epoch = Some(DEFAULT_EPOCH);
    let cases = doc["cases"].as_array().expect("cases");
    assert!(!cases.is_empty());
    let mut follows = std::collections::BTreeSet::new();
    let mut states = std::collections::BTreeSet::new();
    for case in cases {
        let note = &case["note"];
        let l = &layouts[case["layout"].as_str().expect("layout")];
        let mut reader = Reader::new();
        for (i, f) in case["frames"]
            .as_array()
            .expect("frames")
            .iter()
            .enumerate()
        {
            let run = Run::from_hex(f["run"].as_str().expect("run")).expect("a run");
            let seq =
                Seq::new(u32::try_from(f["seq"].as_u64().expect("seq")).unwrap()).expect("a seq");
            let body = unhex(f["hex"].as_str().expect("hex"));
            let frame = Frame::parse(l.tag(), &body, l)
                .unwrap_or_else(|e| panic!("{note}, frame {i}: {e}"));
            assert_eq!(
                frame.read_json(l, epoch).unwrap(),
                f["value"],
                "{note}, frame {i}: its value"
            );
            let got = reader
                .apply(l, Stamp { run, seq }, &frame)
                .reading(l, epoch)
                .unwrap();
            assert_eq!(got, f["expect"], "{note}, frame {i}");
            follows.insert(got["follows"].as_str().unwrap().to_owned());
            for r in got["records"].as_array().unwrap() {
                for v in r["fields"].as_object().unwrap().values() {
                    let token = v["state"].as_str().unwrap();
                    states.insert(match v.get("why") {
                        Some(w) => format!("{token}:{}", w.as_str().unwrap()),
                        None => token.to_owned(),
                    });
                }
            }
        }
    }
    for token in ["first", "continues", "gap", "rewind", "run"] {
        assert!(follows.contains(token), "no frame follows {token}");
    }
    for token in [
        "known",
        "within",
        "absent",
        "unknown:first",
        "unknown:run",
        "unknown:gap",
        "unknown:rewind",
        "unknown:deadband",
        "unknown:no_value",
    ] {
        assert!(states.contains(token), "no field reads {token}");
    }
}

/// A frame resource's bundle reads back into the contract model and
/// builds to its own bytes (core §9.6, §9.5), its `frame` member
/// included; and its layout is read from the contract (core §7.7).
#[test]
fn frame_bundle_round_trips() {
    use zenkey_model::contract::{Contract, load_path};
    use zenkey_model::frame::Layout;
    let l = load_path(&spec().join("conformance/contracts/ok-frame.toml"));
    let c = l
        .contract
        .unwrap_or_else(|| panic!("ok-frame:\n{}", l.report));
    let bytes = Bundle::build(&c).to_bytes();
    let b = Bundle::verify(&bytes).unwrap();
    let back = Contract::from_bundle(&b).unwrap();
    assert_eq!(Bundle::build(&back).to_bytes(), bytes);
    let frames: Vec<_> = back
        .resources
        .iter()
        .filter_map(|r| Layout::of_resource(&back, r).unwrap())
        .collect();
    assert_eq!(frames.len(), 4, "ok-frame declares four frame resources");
    let link = c
        .resources
        .iter()
        .find(|r| r.template.as_str() == "frames/link")
        .unwrap();
    let link = Layout::of_resource(&c, link).unwrap().unwrap();
    // The layouts fixture's `link`, from a contract.
    assert_eq!(link.form(), frame_fixture_layouts()["link"].form());
}

/// A packed bundle reads back into the contract model and builds to its
/// own bytes, as every bundle does (core §9.6, §9.5; #979 B1).
#[test]
fn packed_bundle_round_trips() {
    use zenkey_model::contract::Contract;
    let bytes = std::fs::read(spec().join("conformance/bundles/packed.bundle.json")).unwrap();
    let b = Bundle::verify(&bytes).unwrap();
    let c = Contract::from_bundle(&b).unwrap();
    assert_eq!(Bundle::build(&c).to_bytes(), bytes);
    let packed: Vec<_> = c
        .artifacts
        .values()
        .filter(|a| a.kind == zenkey_model::schema::SchemaKind::Packed)
        .collect();
    assert_eq!(packed.len(), 2, "ok-packed lists two packed files");
}

/// A tool's decoder reads a packed record through the bundle's layout,
/// under the tag its envelope carried, and refuses one by name (core
/// §7.6, observer §7.3; #979 B2). A record that came in no envelope has
/// no tag, and is undecodable.
#[test]
fn packed_records_decode_from_the_bundle() {
    use zenkey_model::contract::Contract;
    use zenkey_model::decode::{Context, Rendered, decode, decode_in, type_of};
    use zenkey_model::wire::Tag;
    let bytes = std::fs::read(spec().join("conformance/bundles/packed.bundle.json")).unwrap();
    let b = Bundle::verify(&bytes).unwrap();
    let c = Contract::from_bundle(&b).unwrap();
    let r = c
        .resources
        .iter()
        .find(|r| r.template.as_str() == "reading")
        .unwrap();
    let zenkey_model::contract::Body::Data(d) = &r.body else {
        panic!("a stream")
    };
    let layout = c.packed_layout(&d.type_).unwrap().expect("a packed type");
    let value =
        json!({"temperature": "21.5", "humidity": "43.5", "pressure": "1013.2", "battery": "3.7"});
    let record = layout.write(&value, None).unwrap().bytes;
    let ty = type_of(&b, "stream", "reading", "type").unwrap();
    let tag = Tag::new(layout.tag()).unwrap();
    assert_eq!(
        decode_in(&b, ty, None, &record, Context::tagged(tag)),
        Rendered::Value(value)
    );
    let undecodable = |r: Rendered| match r {
        Rendered::Undecodable {
            declared, reason, ..
        } => (declared, reason),
        other => panic!("{other:?}"),
    };
    let (declared, reason) = undecodable(decode(&b, ty, None, &record));
    assert_eq!(declared, "packed:Sensor");
    assert!(reason.contains("no tag"), "{reason}");
    let other = Tag::new((layout.tag() + 1) % 16).unwrap();
    let (_, reason) = undecodable(decode_in(&b, ty, None, &record, Context::tagged(other)));
    assert!(reason.contains("tag:"), "{reason}");
    let (_, reason) = undecodable(decode_in(&b, ty, None, &record[..4], Context::tagged(tag)));
    assert!(reason.contains("length:"), "{reason}");
}

/// `spec/conformance/catalog/documents.json` (core §8.6; #980 C1a): a
/// catalog's text → its id, or the reader's refusal, the first in §8.6's
/// order. Every accepted catalog writes back to JCS bytes of the same id,
/// and reads back from them. Written by hand from the text, checked, never
/// blessed.
#[test]
fn catalog_documents() {
    use zenkey_model::catalog::Catalog;
    let doc = read_json(&spec().join("conformance/catalog/documents.json"));
    let cases = doc["cases"].as_array().expect("cases");
    let mut refused = std::collections::BTreeSet::new();
    let mut accepted = 0;
    for case in cases {
        let note = &case["note"];
        let text = case["text"].as_str().expect("text");
        let got = match Catalog::parse(text) {
            Ok(c) => {
                accepted += 1;
                let again = Catalog::parse(std::str::from_utf8(&c.to_jcs()).unwrap())
                    .unwrap_or_else(|e| panic!("{note}: its JCS bytes refused: {e}"));
                assert_eq!(again, c, "{note}: reads back from its JCS bytes");
                json!({"id": c.id().hex()})
            }
            Err(e) => {
                refused.insert(e.token());
                json!({"refused": e.token()})
            }
        };
        assert_eq!(case["expect"], got, "{note}");
    }
    assert!(accepted > 0, "no accepted case");
    for token in [
        "json",
        "format",
        "member",
        "epoch",
        "entries",
        "index",
        "iface",
        "contract",
        "duplicate",
        "lineage",
    ] {
        assert!(refused.contains(token), "no case refuses with {token}");
    }
}

/// `spec/conformance/catalog/ids.json` (core §8.6): an accepted catalog's
/// text → its JCS bytes, their sha256 and its id. Written by hand from the
/// text, checked, never blessed.
#[test]
fn catalog_ids() {
    use zenkey_model::catalog::{Catalog, CatalogId};
    let doc = read_json(&spec().join("conformance/catalog/ids.json"));
    let cases = doc["cases"].as_array().expect("cases");
    assert!(!cases.is_empty());
    for case in cases {
        let note = &case["note"];
        let c = Catalog::parse(case["text"].as_str().expect("text"))
            .unwrap_or_else(|e| panic!("{note}: {e}"));
        let bytes = c.to_jcs();
        let sha = zenkey_model::schema::sha256_id(&bytes);
        assert_eq!(
            case["expect"],
            json!({
                "jcs": String::from_utf8(bytes.clone()).unwrap(),
                "sha256": &sha["sha256:".len()..],
                "id": c.id().hex(),
            }),
            "{note}"
        );
        assert_eq!(CatalogId::of_jcs(&bytes), c.id(), "{note}");
    }
}

/// `spec/conformance/catalog/succession.json` (core §8.6): pairs of named
/// catalogs → whether the second succeeds the first, or the checker's
/// refusal; and the catalogs a reader holds, the id an owner names and an
/// index → the epoch and the entry, or the reader's refusal. Written by
/// hand from the text, checked, never blessed.
#[test]
fn catalog_succession() {
    use zenkey_model::catalog::{Catalog, CatalogId, epoch_of, through};
    let doc = read_json(&spec().join("conformance/catalog/succession.json"));
    let catalogs: BTreeMap<String, Catalog> = doc["catalogs"]
        .as_object()
        .expect("catalogs")
        .iter()
        .map(|(name, v)| {
            let c =
                Catalog::parse(&v.to_string()).unwrap_or_else(|e| panic!("catalog {name}: {e}"));
            (name.clone(), c)
        })
        .collect();
    let named = |n: &Value| &catalogs[n.as_str().expect("a catalog's name")];
    let mut refused = std::collections::BTreeSet::new();
    for case in doc["successions"].as_array().expect("successions") {
        let note = &case["note"];
        let got = match named(&case["next"]).succeeds(named(&case["previous"])) {
            Ok(()) => json!("succeeds"),
            Err(s) => {
                refused.insert(s.token());
                json!({"refused": s.token()})
            }
        };
        assert_eq!(case["expect"], got, "succession: {note}");
    }
    for token in ["epoch_moved", "reassigned", "removed", "lineage"] {
        assert!(
            refused.contains(token),
            "no succession refused with {token}"
        );
    }
    let mut read_refused = std::collections::BTreeSet::new();
    for case in doc["reads"].as_array().expect("reads") {
        let note = &case["note"];
        let held: Vec<Catalog> = case["held"]
            .as_array()
            .expect("held")
            .iter()
            .map(|n| named(n).clone())
            .collect();
        let id: Option<CatalogId> = case["catalog"]
            .as_str()
            .map(|s| s.parse().unwrap_or_else(|e| panic!("{note}: {e}")));
        let index = case["index"]
            .as_u64()
            .map(|i| u16::try_from(i).expect("an index"));
        let got = match (id, index) {
            (None, Some(_)) => Err(zenkey_model::catalog::ReadRefusal::UnknownCatalog),
            (_, None) => epoch_of(&held, id).map(|epoch| json!({"epoch": epoch})),
            (Some(id), Some(i)) => through(&held, id).and_then(|t| {
                t.entry(i).map(|e| {
                    json!({
                        "epoch": t.epoch(),
                        "iface": e.iface.to_string(),
                        "contract": e.contract.to_string(),
                    })
                })
            }),
        };
        let got = got.unwrap_or_else(|r| {
            read_refused.insert(r.token());
            json!({"refused": r.token()})
        });
        assert_eq!(case["expect"], got, "read: {note}");
    }
    for token in ["unknown_catalog", "unknown_index"] {
        assert!(read_refused.contains(token), "no read refused with {token}");
    }
}

/// `spec/catalog.schema.json` is generated from the catalog's document type.
#[test]
fn catalog_schema_is_current() {
    let path = spec().join("catalog.schema.json");
    let schema = schemars::schema_for!(zenkey_model::catalog::Document);
    let text = serde_json::to_string_pretty(&schema).unwrap() + "\n";
    if bless() {
        std::fs::write(&path, &text).unwrap();
    } else {
        assert_eq!(
            std::fs::read_to_string(&path).unwrap_or_default(),
            text,
            "spec/catalog.schema.json is stale: ZK2_BLESS=1 cargo test -p zenkey-model --test conformance"
        );
    }
}

/// A snapshot's time as `snapshot/bodies.json` spells it: `{"s", "ms"}`, or
/// `null` for the unknown time.
fn snapshot_time(v: &Value) -> zenkey_model::collection::Time {
    use zenkey_model::collection::Time;
    if v.is_null() {
        return Time::Unknown;
    }
    Time::Known {
        s: u32::try_from(v["s"].as_u64().expect("s")).expect("t32"),
        ms: u16::try_from(v["ms"].as_u64().expect("ms")).expect("ms16"),
    }
}

fn snapshot_time_json(t: zenkey_model::collection::Time) -> Value {
    use zenkey_model::collection::Time;
    match t {
        Time::Unknown => Value::Null,
        Time::Known { s, ms } => json!({"s": s, "ms": ms}),
    }
}

/// A run from `run_width` and `run`; `None` where both are `null`.
fn snapshot_run(v: &Value) -> Option<zenkey_model::wire::Run> {
    use zenkey_model::wire::Run;
    let run = v["run"].as_u64()?;
    Some(match v["run_width"].as_u64() {
        Some(16) => Run::U16(u16::try_from(run).expect("a 16-bit run")),
        Some(32) => Run::U32(u32::try_from(run).expect("a 32-bit run")),
        other => panic!("run_width {other:?}"),
    })
}

fn snapshot_run_json(run: Option<zenkey_model::wire::Run>) -> (Value, Value) {
    match run {
        None => (Value::Null, Value::Null),
        Some(r) => (json!(if r.is_wide() { 32 } else { 16 }), json!(r.value())),
    }
}

fn snapshot_seq(v: &Value) -> zenkey_model::wire::Seq {
    zenkey_model::wire::Seq::new(u32::try_from(v.as_u64().expect("a seq")).unwrap()).expect("seq")
}

/// A snapshot's entries, `live` and `tombs`, as the fixture spells them.
fn snapshot_entries(
    v: &Value,
) -> (
    Vec<zenkey_model::collection::Entry>,
    Vec<zenkey_model::collection::Tombstone>,
) {
    use zenkey_model::collection::{Confirmation, Entry, Tombstone};
    let live = v["live"]
        .as_array()
        .expect("live")
        .iter()
        .map(|e| Entry {
            id: e["id"].as_str().expect("id").to_owned(),
            run: snapshot_run(e),
            seq: snapshot_seq(&e["seq"]),
            confirmation: match e["confirmation"].as_str() {
                Some("none") => Confirmation::None,
                Some("time") => Confirmation::AtTime,
                Some("conf") => Confirmation::At(snapshot_time(&e["conf"])),
                other => panic!("confirmation {other:?}"),
            },
            record: unhex(e["record"].as_str().expect("record")),
        })
        .collect();
    let tombs = v["tombs"]
        .as_array()
        .expect("tombs")
        .iter()
        .map(|t| Tombstone {
            id: t["id"].as_str().expect("id").to_owned(),
            run: snapshot_run(t),
            seq: snapshot_seq(&t["seq"]),
        })
        .collect();
    (live, tombs)
}

fn snapshot_page(v: &Value) -> zenkey_model::collection::Page {
    let (live, tombs) = snapshot_entries(v);
    zenkey_model::collection::Page {
        tag: zenkey_model::wire::Tag::new(u8::try_from(v["tag"].as_u64().expect("tag")).unwrap())
            .expect("tag"),
        run: snapshot_run(v).expect("the header's run"),
        asof: snapshot_seq(&v["asof"]),
        time: snapshot_time(&v["time"]),
        index: u32::try_from(v["index"].as_u64().expect("index")).expect("index"),
        more: v["more"].as_bool().expect("more"),
        coll: u32::try_from(v["coll"].as_u64().expect("coll")).expect("coll"),
        live,
        tombs,
    }
}

fn snapshot_of(v: &Value) -> zenkey_model::collection::Snapshot {
    let mut page = v.clone();
    page["index"] = json!(0);
    page["more"] = json!(false);
    let p = snapshot_page(&page);
    zenkey_model::collection::Snapshot {
        tag: p.tag,
        run: p.run,
        asof: p.asof,
        time: p.time,
        coll: p.coll,
        live: p.live,
        tombs: p.tombs,
    }
}

/// A page as `snapshot/bodies.json`'s `reads` spell it.
fn snapshot_page_json(p: &zenkey_model::collection::Page) -> Value {
    use zenkey_model::collection::Confirmation;
    let (run_width, run) = snapshot_run_json(Some(p.run));
    let live: Vec<Value> = p
        .live
        .iter()
        .map(|e| {
            let (run_width, run) = snapshot_run_json(e.run);
            let mut v = json!({
                "id": e.id,
                "run_width": run_width,
                "run": run,
                "seq": e.seq.get(),
                "record": hex(&e.record),
            });
            match e.confirmation {
                Confirmation::None => v["confirmation"] = json!("none"),
                Confirmation::AtTime => v["confirmation"] = json!("time"),
                Confirmation::At(t) => {
                    v["confirmation"] = json!("conf");
                    v["conf"] = snapshot_time_json(t);
                }
            }
            v
        })
        .collect();
    let tombs: Vec<Value> = p
        .tombs
        .iter()
        .map(|t| {
            let (run_width, run) = snapshot_run_json(t.run);
            json!({"id": t.id, "run_width": run_width, "run": run, "seq": t.seq.get()})
        })
        .collect();
    json!({
        "tag": p.tag.get(),
        "run_width": run_width,
        "run": run,
        "asof": p.asof.get(),
        "time": snapshot_time_json(p.time),
        "index": p.index,
        "more": p.more,
        "coll": p.coll,
        "live": live,
        "tombs": tombs,
    })
}

/// `spec/conformance/snapshot/bodies.json` (core §7.5, 0.97; #977's E1): a
/// page → its bytes and their size, or the writer's refusal (`writes`); a
/// page's bytes → the page, or the reading's first refusal (`reads`); a
/// snapshot and a page size → its pages' bytes and sizes (`pages`). Every
/// page read writes back to its bytes, every page written reads back to
/// its value, and every set of pages to its snapshot. Written by an
/// independent reading of the text, checked, never blessed.
#[test]
fn snapshot_bodies() {
    use zenkey_model::collection::{Page, PageRefusal, Snapshot};
    let doc = read_json(&spec().join("conformance/snapshot/bodies.json"));

    let writes = doc["writes"].as_array().expect("writes");
    assert!(!writes.is_empty());
    for case in writes {
        let note = &case["note"];
        let page = snapshot_page(&case["page"]);
        let got = match page.write() {
            Ok(bytes) => {
                assert_eq!(page.size(), bytes.len(), "{note}: its size");
                assert_eq!(Page::parse(&bytes), Ok(page.clone()), "{note}: reads back");
                json!({"hex": hex(&bytes), "size": bytes.len()})
            }
            Err(r) => json!({"refused": r.token()}),
        };
        assert_eq!(case["expect"], got, "{note}");
    }

    let reads = doc["reads"].as_array().expect("reads");
    assert!(!reads.is_empty());
    let mut tokens = std::collections::BTreeSet::new();
    for case in reads {
        let note = &case["note"];
        let bytes = unhex(case["hex"].as_str().expect("hex"));
        let got = match Page::parse(&bytes) {
            Ok(p) => {
                assert_eq!(p.write().as_ref(), Ok(&bytes), "{note}: writes back");
                snapshot_page_json(&p)
            }
            Err(PageRefusal::Body(r)) => {
                tokens.insert(r.token());
                json!({"refused": r.token()})
            }
            Err(r) => panic!("{note}: every case's envelope is a snapshot's: {r}"),
        };
        assert_eq!(case["expect"], got, "{note}");
    }
    assert_eq!(
        tokens.into_iter().collect::<Vec<_>>(),
        [
            "flags", "id", "leb128", "order", "run", "short", "time", "trailing"
        ],
        "every refusal"
    );

    let pages = doc["pages"].as_array().expect("pages");
    assert!(!pages.is_empty());
    for case in pages {
        let note = &case["note"];
        let snapshot = snapshot_of(&case["snapshot"]);
        let size = usize::try_from(case["page_bytes"].as_u64().expect("page_bytes")).unwrap();
        let got = match snapshot.paginate(size) {
            Ok(cut) => {
                let bytes: Vec<Vec<u8>> = cut
                    .iter()
                    .map(|p| p.write().unwrap_or_else(|r| panic!("{note}: {r}")))
                    .collect();
                let read: Vec<Page> = bytes
                    .iter()
                    .map(|b| Page::parse(b).unwrap_or_else(|r| panic!("{note}: {r}")))
                    .collect();
                assert_eq!(read, cut, "{note}: each page reads back");
                assert_eq!(
                    Snapshot::complete(&read).as_ref(),
                    Some(&snapshot),
                    "{note}: the pages read back to the snapshot"
                );
                json!({
                    "pages": bytes.iter().map(|b| hex(b)).collect::<Vec<_>>(),
                    "sizes": bytes.iter().map(Vec::len).collect::<Vec<_>>(),
                })
            }
            Err(r) => json!({"refused": r.token()}),
        };
        assert_eq!(case["expect"], got, "{note}");
    }
}

/// `spec/conformance/seed/snapshot.json` (core §4.2): a reader's
/// collections, what it holds when it asks a GET, and every arrival after
/// it — the GET's pages and per-key replies, and live samples — → what it
/// holds, and each
/// snapshot the GET brought, its pages, completeness and the keys its
/// omission rule dropped. The merge is a tool's
/// (`zenkey_model::seed::admits_mark`), a page's entries are its members'
/// per-key replies (`collection::Page::members`), the pages are held by
/// `collection::hear`, and the omission is `seed::apply_snapshot`, applied
/// once, when a snapshot turns complete. A case that gives `revisions` maps
/// coll by the one its `descriptor` names (core §4.2, F-497). Written by an
/// independent reading of the text, checked by a second, never blessed.
#[test]
fn seed_snapshot() {
    use zenkey_model::collection::{Heard, Page, Snapshot, hear, snapshot_prefix};
    use zenkey_model::seed::{HeldMember, Mark, Versus, apply_snapshot, versus};
    use zenkey_model::wire::{self, Run, Seq};

    /// One key's held sample: a put's record or a delete, its stamp, and
    /// whether it was heard after the GET was asked.
    #[derive(Debug, Clone)]
    struct HeldSample {
        record: Option<Vec<u8>>,
        stamp: wire::Stamp,
        heard_after: bool,
    }

    let doc = read_json(&spec().join("conformance/seed/snapshot.json"));
    let stamp = |v: &Value| -> wire::Stamp {
        let run = v["run"].as_u64().expect("run");
        let run = match v["width"].as_u64() {
            Some(16) => Run::U16(u16::try_from(run).expect("a 16-bit run")),
            Some(32) => Run::U32(u32::try_from(run).expect("a 32-bit run")),
            w => panic!("width {w:?}"),
        };
        let seq = u32::try_from(v["seq"].as_u64().expect("seq")).expect("a seq");
        wire::Stamp {
            run,
            seq: Seq::new(seq).expect("a seq"),
        }
    };
    let stamp_json = |s: wire::Stamp| json!({"width": if s.run.is_wide() { 32 } else { 16 }, "run": s.run.value(), "seq": s.seq.get()});
    let sample = |v: &Value| -> HeldSample {
        let record = match v["sample"].as_str() {
            Some("put") => Some(unhex(v["record"].as_str().expect("record"))),
            Some("delete") => None,
            other => panic!("sample {other:?}"),
        };
        HeldSample {
            record,
            stamp: stamp(&v["stamp"]),
            heard_after: false,
        }
    };
    let cases = doc["cases"].as_array().expect("cases");
    assert!(cases.len() >= 40, "the fixture's cases");
    let (mut omitted_some, mut incomplete_some, mut paged) = (false, false, false);
    for case in cases {
        let note = &case["note"];
        // Each collection the reader's contract gives it: its index, its
        // snapshot key, the key its template's chunks start under, and the
        // template. Where the case gives revisions, the reader maps coll by
        // the one the owner's descriptor names alone (F-497).
        let given = match case.get("revisions") {
            Some(revs) => {
                let named = case["descriptor"].as_str().expect("descriptor");
                &revs[named]
            }
            None => &case["collections"],
        };
        let collections: Vec<(u32, String, String, Template)> = given
            .as_array()
            .expect("collections")
            .iter()
            .map(|c| {
                let template = Template::parse(c["template"].as_str().expect("template"))
                    .unwrap_or_else(|e| panic!("{note}: {e}"));
                let key = c["snapshot_key"].as_str().expect("snapshot_key").to_owned();
                let n = snapshot_prefix(&template).expect("a snapshot key").len();
                let chunks: Vec<&str> = key.split('/').collect();
                let root = chunks[..chunks.len() - n].join("/");
                let coll = u32::try_from(c["coll"].as_u64().expect("coll")).expect("coll");
                (coll, key, root, template)
            })
            .collect();
        let mut view: BTreeMap<String, HeldSample> = case["view"]
            .as_array()
            .expect("view")
            .iter()
            .map(|v| (v["key"].as_str().expect("key").to_owned(), sample(v)))
            .collect();
        // Every arrival is heard after the GET was asked: kept when newer,
        // or once when it is the mutation held.
        let merge = |view: &mut BTreeMap<String, HeldSample>, key: &str, mut got: HeldSample| {
            got.heard_after = true;
            let held = view
                .get(key)
                .map(|h| versus(Mark::InBand(h.stamp), Mark::InBand(got.stamp)));
            match held {
                None | Some(Versus::Newer | Versus::Unordered) => {
                    view.insert(key.to_owned(), got);
                }
                Some(Versus::Same) => view.get_mut(key).expect("held").heard_after = true,
                Some(Versus::Older) => {}
            }
        };
        let mut heard: Vec<Heard> = Vec::new();
        let mut pages: Vec<Vec<Page>> = Vec::new();
        let mut omitted: Vec<Vec<String>> = Vec::new();
        for a in case["arrivals"].as_array().expect("arrivals") {
            let key = a["key"].as_str().expect("key");
            let via = a["via"].as_str().expect("via");
            assert!(matches!(via, "get" | "subscription"), "{note}: via {via}");
            if a.get("page").is_none() {
                merge(&mut view, key, sample(a));
                continue;
            }
            let bytes = unhex(a["hex"].as_str().expect("hex"));
            let page = Page::parse(&bytes).unwrap_or_else(|r| panic!("{note}: {r}"));
            assert_eq!(snapshot_page_json(&page), a["page"], "{note}: the page");
            assert_eq!(page.write().as_deref(), Ok(&bytes[..]), "{note}");
            // A snapshot heard on a subscription is no owner's answer.
            if via == "subscription" {
                continue;
            }
            for m in page.members(key) {
                let got = HeldSample {
                    record: m.record.clone(),
                    stamp: m.stamp,
                    heard_after: true,
                };
                merge(&mut view, &m.key, got);
            }
            let (at, completed) = hear(&mut heard, key, &page);
            if at == pages.len() {
                pages.push(Vec::new());
                omitted.push(Vec::new());
            }
            // A page heard twice is held once.
            if !pages[at].iter().any(|p| p.index == page.index) {
                pages[at].push(page.clone());
            }
            if !completed {
                continue;
            }
            let h = &heard[at];
            let snapshot = Snapshot::complete(&pages[at])
                .unwrap_or_else(|| panic!("{note}: complete, as `hear` says"));
            let Some((_, _, root, template)) = collections.iter().find(|(coll, sk, _, _)| {
                *coll == h.coll && (key == sk || key.starts_with(&format!("{sk}/")))
            }) else {
                // Without the contract, coll maps to no template.
                continue;
            };
            let of_collection = |k: &str| {
                k.strip_prefix(root.as_str())
                    .and_then(|rest| rest.strip_prefix('/'))
                    .is_some_and(|rest| {
                        let chunks: Vec<&str> = rest.split('/').collect();
                        template.matches(&chunks).is_some()
                    })
            };
            let held: Vec<HeldMember<'_>> = view
                .iter()
                .map(|(k, s)| HeldMember {
                    key: k,
                    mark: Some(Mark::InBand(s.stamp)),
                    heard_after: s.heard_after,
                })
                .collect();
            let drop: Vec<String> = apply_snapshot(&snapshot, key, of_collection, held)
                .into_iter()
                .map(str::to_owned)
                .collect();
            for k in &drop {
                view.remove(k);
            }
            omitted[at].extend(drop);
        }
        let got_view: Vec<Value> = view
            .iter()
            .map(|(k, s)| match &s.record {
                Some(r) => json!({"key": k, "sample": "put", "stamp": stamp_json(s.stamp), "record": hex(r)}),
                None => json!({"key": k, "sample": "delete", "stamp": stamp_json(s.stamp)}),
            })
            .collect();
        let got_snapshots: Vec<Value> = heard
            .iter()
            .zip(&omitted)
            .map(|(h, o)| {
                let mut o = o.clone();
                o.sort();
                json!({
                    "key": h.key,
                    "coll": h.coll,
                    "run": {"width": if h.run.is_wide() { 32 } else { 16 }, "run": h.run.value()},
                    "asof": h.asof.get(),
                    "pages": h.pages.iter().collect::<Vec<_>>(),
                    "complete": h.complete(),
                    "omitted": o,
                })
            })
            .collect();
        omitted_some |= omitted.iter().any(|o| !o.is_empty());
        incomplete_some |= heard.iter().any(|h| !h.complete());
        paged |= heard.iter().any(|h| h.pages.len() > 1);
        assert_eq!(
            json!({"view": got_view, "snapshots": got_snapshots}),
            case["expect"],
            "{note}"
        );
    }
    assert!(
        omitted_some && incomplete_some && paged,
        "the cases cover omissions, losses and pages"
    );
}
