//! `tests/fixtures/zrec/modem-v4.cases.json` (the tooling guide's §5,
//! `.zrec` version 4, and its O8; #990 V3, V4): four modem.v3 rows under a
//! version-4 header, read by the fleet's reader. The fixture is evidence,
//! written by two independent readings and never blessed: every row and
//! every case must pass as written.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use zenkey_fleet::model::deployment::{Catalogs, DeploymentCatalog, Epoch, rfc3339_ms};
use zenkey_fleet::model::inband::Payload;
use zenkey_fleet::model::recorded::{Bundled, Recorded};
use zenkey_fleet::model::render::Member;
use zenkey_fleet::report::{Rendered, Unresolved};
use zenkey_fleet::tape::record::{read_header, reader_catalogs};
use zenkey_fleet::{ContractSet, Lens};
use zenkey_model::grammar::Addr;

const EPOCH_S: i64 = 1_767_225_600;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zrec")
}

/// RFC 6902's `add`, `remove` and `replace`, on JSON pointers.
fn patch(doc: &mut Value, ops: &[Value]) {
    for op in ops {
        let path = op["path"].as_str().expect("a path");
        let (parent, last) = path.rsplit_once('/').expect("a pointer");
        let target = doc.pointer_mut(parent).expect("the parent");
        match op["op"].as_str() {
            Some("remove") => match target {
                Value::Object(m) => {
                    m.remove(last).expect("the member");
                }
                Value::Array(a) => {
                    a.remove(last.parse::<usize>().expect("an index"));
                }
                _ => panic!("remove from a scalar"),
            },
            Some("replace" | "add") => match target {
                Value::Object(m) => {
                    m.insert(last.to_owned(), op["value"].clone());
                }
                Value::Array(a) => {
                    let i: usize = last.parse().expect("an index");
                    if op["op"] == "add" {
                        a.insert(i, op["value"].clone());
                    } else {
                        a[i] = op["value"].clone();
                    }
                }
                _ => panic!("replace in a scalar"),
            },
            other => panic!("op {other:?}"),
        }
    }
}

/// Every `dated` in a reading, rewritten by `f`: a frame record's, a
/// heartbeat's, and each field's of a packed record's map.
fn each_dated(v: &mut Value, f: &mut dyn FnMut(&mut Value, Option<&str>)) {
    match v {
        Value::Object(m) => {
            if let Some(d) = m.get_mut("dated") {
                if d.get("epoch").is_some() {
                    f(d, None);
                } else if let Some(fields) = d.as_object_mut() {
                    for (name, d) in fields.iter_mut() {
                        f(d, Some(name));
                    }
                }
            }
            for (k, x) in m.iter_mut() {
                if k != "dated" {
                    each_dated(x, f);
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| each_dated(x, f)),
        _ => {}
    }
}

/// A row's `decoded` as a variant of `as_recorded` reads it (the
/// fixture's description).
fn variant(decoded: &Value, name: &str) -> Value {
    let mut d = decoded.clone();
    match name {
        "as_recorded" => {}
        "as_recorded_undated" => each_dated(&mut d, &mut |x, _| {
            let m = x.as_object_mut().expect("a dated");
            m.remove("utc");
            m.insert("epoch".into(), "unknown_catalog".into());
        }),
        "as_recorded_epoch_deployment" => each_dated(&mut d, &mut |x, _| {
            x["epoch"] = "deployment".into();
        }),
        "as_recorded_epoch_assumed" | "as_recorded_epoch_zero" => {
            let zero = name.ends_with("zero");
            // A packed `time` field reads under 0.
            let fields: Vec<String> = d
                .get("dated")
                .and_then(Value::as_object)
                .filter(|m| {
                    m.values().all(|x| x.get("epoch").is_some()) && d.get("value").is_some()
                })
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default();
            for f in &fields {
                let v = d["value"][f].as_i64().expect("a time field");
                d["value"][f] = (v - EPOCH_S * 1000).into();
            }
            let value = d.get("value").cloned();
            each_dated(&mut d, &mut |x, field| {
                if !zero {
                    *x = json!({"epoch": "assumed"});
                    return;
                }
                let with_ms = x["utc"].as_str().is_some_and(|u| u.contains('.'));
                let ms = match field {
                    Some(f) => value.as_ref().and_then(|v| v[f].as_i64()).expect("a time"),
                    None => {
                        let utc = x["utc"].as_str().expect("a utc");
                        // The wire number from the epoch, back from its date.
                        parse_rfc3339_ms(utc) - EPOCH_S * 1000
                    }
                };
                *x = json!({
                    "utc": rfc3339_ms(i128::from(ms), with_ms).expect("a date"),
                    "epoch": "stated",
                });
            });
        }
        other => panic!("variant {other}"),
    }
    d
}

/// Milliseconds since the Unix epoch of an RFC 3339 UTC date, the
/// fixture's own spelling: `YYYY-MM-DDTHH:MM:SS(.mmm)?Z`.
fn parse_rfc3339_ms(s: &str) -> i64 {
    let n = |a: usize, b: usize| s[a..b].parse::<i64>().expect("digits");
    let (y, m, d) = (n(0, 4), n(5, 7), n(8, 10));
    let (h, mi, sec) = (n(11, 13), n(14, 16), n(17, 19));
    let ms = if s.as_bytes()[19] == b'.' {
        n(20, 23)
    } else {
        0
    };
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    ((days * 86_400 + h * 3600 + mi * 60 + sec) * 1000) + ms
}

/// What the reader made of one row: its envelope, its resolution and its
/// reading, in the fixture's spelling.
fn read_row(lens: &Lens<'_>, line: &Value) -> (Value, Value, Value, Option<Unresolved>, Rendered) {
    let key = line["key"].as_str().expect("a key");
    let encoding = line["encoding"].as_str();
    let bytes = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        line["bytes"].as_str().expect("bytes"),
    )
    .expect("base64");
    let envelope = match zenkey_fleet::model::inband::payload(key, encoding, &bytes) {
        Payload::Record { stamp, tag, .. } => {
            json!({"form": "mutation", "run": stamp.run.hex(), "seq": stamp.seq.get(), "tag": tag.get()})
        }
        Payload::Frame { stamp, tag, .. } => {
            json!({"form": "frame", "run": stamp.run.hex(), "seq": stamp.seq.get(), "tag": tag.get()})
        }
        Payload::Heartbeat { stamp, tag, .. } => {
            json!({"form": "heartbeat", "run": stamp.run.hex(), "seq": stamp.seq.get(), "tag": tag.get()})
        }
        other => panic!("{key}: {other:?}"),
    };
    let resolves = match lens.resolve(key).resolved {
        Some(r) => {
            let mut v = json!({
                "address": r.address.to_string(),
                "iface": r.iface.to_string(),
                "fingerprint": r.revision.fingerprint().to_string(),
                "resource": r.resource_name(),
            });
            if !r.values.is_empty() {
                v["values"] = serde_json::to_value(&r.values).expect("values");
            }
            v
        }
        None => Value::Null,
    };
    let checked = lens.check_sample(key, Member::Type, encoding, &bytes, None);
    let decoded = match &checked.rendering.rendered {
        Rendered::Value {
            declared,
            value,
            dated,
            bounds,
            ..
        } => match envelope["form"].as_str() {
            Some("frame") => {
                let full = frame_full(&bytes, lens, key);
                json!({"type": declared, "full": full, "reading": value})
            }
            Some("heartbeat") => {
                let mut v = value.clone();
                let m = v.as_object_mut().expect("a heartbeat's reading");
                m.remove("run");
                m.remove("seq");
                m.insert("type".into(), declared.clone().into());
                v
            }
            _ => {
                let mut v = json!({"type": declared, "value": value});
                if let Some(d) = dated {
                    v["dated"] = d.clone();
                }
                if let Some(b) = bounds {
                    v["bounds"] = b.clone();
                }
                v
            }
        },
        _ => Value::Null,
    };
    (
        envelope,
        resolves,
        decoded,
        checked.identity.unresolved,
        checked.rendering.rendered,
    )
}

/// Whether a frame's record 0 is full, read from its body with its
/// resource's layout.
fn frame_full(bytes: &[u8], lens: &Lens<'_>, key: &str) -> bool {
    let r = lens.resolve(key).resolved.expect("a frame resolves");
    let layout = zenkey_model::frame::Layout::of_resource(r.revision.contract(), r.resource())
        .expect("a layout")
        .expect("a frame resource");
    let Payload::Frame { body, tag, .. } =
        zenkey_fleet::model::inband::payload(key, Some("zenoh/bytes"), bytes)
    else {
        panic!("a frame");
    };
    let frame = zenkey_model::frame::Frame::parse(tag, body, &layout).expect("parses");
    frame.records.first().is_some_and(|r| r.full)
}

/// The epoch in force for an address, in the cases' spelling.
fn epoch_of(lens: &Lens<'_>, addr: &Addr) -> Value {
    let d = lens.dating_of(addr);
    let catalog = d.catalog.map(|c| c.hex());
    match d.epoch {
        Epoch::Stated(e) => match catalog {
            Some(c) => json!({"from": "stated", "catalog": c}),
            None => json!({"from": "stated", "epoch_s": e}),
        },
        Epoch::Deployment(_) => json!({"from": "deployment", "catalog": catalog}),
        Epoch::Assumed => json!({"from": "assumed"}),
        Epoch::Unknown(id) => json!({"from": "unknown_catalog", "catalog": id.hex()}),
        Epoch::Ambiguous => json!({"from": "ambiguous"}),
    }
}

/// A rung as the cases name it, from where the key's resolution stopped,
/// or the rendering's refusal.
fn rung(unresolved: Option<&Unresolved>, rendered: &Rendered) -> Value {
    match (unresolved, rendered) {
        (Some(Unresolved::ContractUnavailable { refused, .. }), _) => match refused.as_slice() {
            [] => json!({"rung": "unavailable"}),
            [one] => json!({"rung": "unavailable", "refused": one}),
            more => json!({"rung": "unavailable", "refused": more}),
        },
        (Some(Unresolved::ContractNotHeld { .. }), _) => json!({"rung": "not_held"}),
        (Some(Unresolved::NoProvider), _) => json!({"rung": "no_provider"}),
        (_, Rendered::Undecodable { reason, .. }) if reason.contains("unknown_catalog") => {
            json!({"rung": "undecodable", "refused": "unknown_catalog"})
        }
        (other, r) => json!({"rung": format!("{other:?} {r:?}")}),
    }
}

#[test]
fn every_row_and_case_of_modem_v4() {
    let doc: Value = serde_json::from_str(
        &std::fs::read_to_string(fixture_dir().join("modem-v4.cases.json")).expect("the cases"),
    )
    .expect("JSON");
    let file =
        std::fs::read_to_string(fixture_dir().join(doc["fixture"].as_str().expect("a file")))
            .expect("the fixture");
    let lines: Vec<&str> = file.lines().collect();
    let rows: Vec<Value> = lines[1..]
        .iter()
        .map(|l| serde_json::from_str(l).expect("a row"))
        .collect();
    let mut history = ContractSet::new();
    for h in ["examples/zk2/.history", "spec/profiles/.history"] {
        let (set, problems) = ContractSet::load_history(&root().join(h));
        assert!(problems.is_empty(), "{h}: {problems:?}");
        history.extend(set);
    }
    let expected_rows = doc["rows"].as_array().expect("rows");
    let cases = doc["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 17, "the fixture's cases");
    let mut failures = Vec::new();
    let mut check = |what: String, got: &Value, want: &Value| {
        if got != want {
            failures.push(format!("{what}:\n  expected {want}\n  got      {got}"));
        }
    };

    // The header as read: its catalog, revisions and verified bundles.
    let header = read_header(lines[0], &zenkey_fleet::tape::record::ZREC_READS).expect("reads");
    let recorded = Recorded::of(&header, ContractSet::new());
    let want = &doc["header"];
    check(
        "header.catalog".into(),
        &json!({"id": header.catalog.as_ref().map(|c| c.id.clone()), "epoch": header.catalog.as_ref().map(|c| c.epoch)}),
        &want["catalog"],
    );
    let revisions: Vec<Value> = header
        .revisions
        .iter()
        .flatten()
        .map(|r| serde_json::to_value(r).expect("a revision"))
        .collect();
    check(
        "header.revisions".into(),
        &Value::Array(revisions),
        &want["revisions"],
    );
    for (fp, b) in want["bundles"].as_object().expect("bundles") {
        let held = recorded
            .bundles()
            .iter()
            .find(|(f, _)| f.to_string() == *fp)
            .map(|(_, b)| b);
        let size = header
            .bundles
            .iter()
            .flatten()
            .find(|x| &x.fingerprint == fp)
            .map(|x| {
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &x.bytes)
                    .expect("base64")
                    .len()
            });
        check(
            format!("header bundle {fp}"),
            &json!({"verified": matches!(held, Some(Bundled::Verified(_))), "bytes": size}),
            b,
        );
    }

    // Each case: a patch on the header line, a reader, what it reads.
    for (i, case) in cases.iter().enumerate() {
        let name = case["name"].as_str().unwrap_or_default();
        let mut head: Value = serde_json::from_str(lines[0]).expect("the header");
        if let Some(ops) = case["patch"].as_array() {
            patch(&mut head, ops);
        }
        let reader = &case["reader"];
        let reads: Vec<u32> = reader["reads"]
            .as_array()
            .expect("reads")
            .iter()
            .map(|v| u32::try_from(v.as_u64().expect("a version")).expect("u32"))
            .collect();
        let expect = &case["expect"];
        let refused = |token: &str| json!({"refused": token});
        let header = match read_header(&head.to_string(), &reads) {
            Ok(h) => h,
            Err(r) => {
                check(format!("case {i} ({name})"), &refused(r.token()), expect);
                continue;
            }
        };
        let given = reader["catalog"].as_str().map(|id| {
            let c = DeploymentCatalog::parse_bytes(
                &serde_json::to_vec(&doc["catalogs"][id]).expect("bytes"),
            )
            .unwrap_or_else(|e| panic!("catalog {id}: {e}"));
            assert_eq!(c.id().hex(), id, "catalog {id}'s id");
            c
        });
        let catalogs: Catalogs = match reader_catalogs(&header, given.as_ref()) {
            Ok((c, _)) => c,
            Err(r) => {
                check(format!("case {i} ({name})"), &refused(r.token()), expect);
                continue;
            }
        };
        if expect.get("refused").is_some() {
            check(format!("case {i} ({name})"), &json!({"read": true}), expect);
            continue;
        }
        let mut contracts = ContractSet::new();
        for fp in reader["contracts"].as_array().into_iter().flatten() {
            let r = history
                .iter()
                .find(|r| r.fingerprint().to_string() == fp.as_str().expect("a fingerprint"))
                .unwrap_or_else(|| panic!("case {i}: {fp} in no history"));
            contracts.insert((**r).clone());
        }
        let recorded = Recorded::of(&header, contracts);
        let lens = Lens::new("", None, &recorded)
            .recorded(&recorded)
            .catalogs(&catalogs);
        if let Some(bundles) = expect["bundles"].as_object() {
            let got: serde_json::Map<String, Value> = recorded
                .bundles()
                .iter()
                .map(|(fp, b)| {
                    (
                        fp.to_string(),
                        match b {
                            Bundled::Verified(_) => json!("verified"),
                            Bundled::Refused(t) => json!({"refused": t}),
                            Bundled::Unreadable(r) => json!({"unreadable": r}),
                        },
                    )
                })
                .collect();
            check(
                format!("case {i} ({name}) bundles"),
                &Value::Object(got),
                &Value::Object(bundles.clone()),
            );
        }
        for (addr, want) in expect["epoch"].as_object().into_iter().flatten() {
            let addr: Addr = addr.parse().expect("an address");
            check(
                format!("case {i} ({name}) epoch of {addr}"),
                &epoch_of(&lens, &addr),
                want,
            );
        }
        for (n, row) in rows.iter().enumerate() {
            let line_no = (n + 2).to_string();
            let want = match &expect["rows"] {
                Value::String(v) => Value::String(v.clone()),
                Value::Object(m) => m.get(&line_no).cloned().unwrap_or(Value::Null),
                _ => Value::Null,
            };
            let (envelope, resolves, decoded, unresolved, rendered) = read_row(&lens, row);
            let at = format!("case {i} ({name}) line {line_no}");
            match want {
                Value::String(v) => {
                    let base = expected_rows
                        .iter()
                        .find(|r| r["line"].as_u64() == Some(n as u64 + 2))
                        .expect("the row");
                    check(format!("{at} key"), &row["key"], &base["key"]);
                    check(format!("{at} envelope"), &envelope, &base["envelope"]);
                    check(format!("{at} resolves"), &resolves, &base["resolves"]);
                    check(
                        format!("{at} decoded ({v})"),
                        &decoded,
                        &variant(&base["decoded"], &v),
                    );
                }
                Value::Object(_) => {
                    check(
                        format!("{at} rung"),
                        &rung(unresolved.as_ref(), &rendered),
                        &want,
                    );
                }
                _ => {}
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
