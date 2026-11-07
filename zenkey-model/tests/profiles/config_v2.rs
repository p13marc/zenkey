//! `config.v2`'s fixtures (`spec/profiles/config/conformance/v2/`, #985
//! G1), run against `zenkey_model::config::v2`. Each was computed by an
//! independent reading of the text, never blessed: a failure here is the
//! model's, or a fixture to raise with the text it contradicts.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use zenkey_model::authoring::{Fanout, Kind, Replies, Serving};
use zenkey_model::config::v2::{
    self, Answer, Interface, Mint, Output, Reply, Res, ResBody, Resources, Start, State, Step,
    TypeRef,
};
use zenkey_model::config::{self, Call, ControlOp, DesiredConfig, Time};
use zenkey_model::contract::{Contract, Rate, load_path};
use zenkey_model::packed;
use zenkey_model::wire::{Envelope, Run, Seq, Stamp, Tag};

use super::{Layer, device_result, profiles, read_json, unhex};

fn dir() -> PathBuf {
    profiles().join("config/conformance")
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The test contract, `rfcfg.v1`, loaded and linted.
fn rfcfg() -> Contract {
    let l = load_path(&dir().join("contracts/rfcfg.v1.toml"));
    l.contract
        .unwrap_or_else(|| panic!("rfcfg.v1 does not load:\n{}", l.report))
}

fn rfcfg_interface() -> Interface {
    Interface::of(&rfcfg()).expect("rfcfg.v1 declares its groups")
}

fn layout_json(l: &packed::Layout) -> Value {
    json!({
        "tag": l.tag(),
        "sha256": l.sha256_hex(),
        "min_bits": l.min_bits(),
        "max_bits": l.max_bits(),
    })
}

fn stamp_of(v: &Value) -> Stamp {
    let run = u32::try_from(v["run"].as_u64().expect("run")).expect("a run");
    Stamp {
        run: match v["run_width"].as_u64() {
            Some(32) => Run::U32(run),
            _ => Run::U16(u16::try_from(run).expect("a 16-bit run")),
        },
        seq: Seq::new(u32::try_from(v["seq"].as_u64().expect("seq")).expect("a seq"))
            .expect("a seq"),
    }
}

// --- derive.json ------------------------------------------------------------------

/// A type reference as `derive.json` resolves one: `packed:[<stem>#]<Name>`,
/// a JSON Schema type `{kind, name, schema}`, or any other.
fn type_ref(v: &Value) -> Option<TypeRef> {
    if v.is_null() {
        return None;
    }
    if let Some(s) = v.as_str() {
        return Some(match s.strip_prefix("packed:") {
            Some(rest) => match rest.split_once('#') {
                Some((stem, name)) => TypeRef::Packed {
                    stem: Some(stem.to_owned()),
                    name: name.to_owned(),
                },
                None => TypeRef::Packed {
                    stem: None,
                    name: rest.to_owned(),
                },
            },
            None => TypeRef::Other,
        });
    }
    Some(match v["kind"].as_str() {
        Some("jsonschema") => TypeRef::JsonSchema {
            name: v["name"].as_str().expect("name").to_owned(),
            schema: v["schema"].as_str().expect("schema").to_owned(),
        },
        Some("packed") => TypeRef::Packed {
            stem: None,
            name: v["name"].as_str().expect("name").to_owned(),
        },
        _ => TypeRef::Other,
    })
}

fn retention_s(s: &str) -> u64 {
    let (n, unit) = s.split_at(s.len() - 1);
    let n: u64 = n.parse().expect("a retention");
    n * match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        "w" => 604_800,
        other => panic!("unit {other:?}"),
    }
}

/// A resource as `derive.json`'s `contracts_base` resolves one.
fn resource(template: &str, v: &Value) -> Res {
    let kind = match v["kind"].as_str().expect("kind") {
        "stream" => Kind::Stream,
        "state" => Kind::State,
        "event" => Kind::Event,
        "operation" => Kind::Operation,
        other => panic!("kind {other:?}"),
    };
    let body = if kind == Kind::Operation {
        ResBody::Operation {
            request: type_ref(&v["request"]).expect("a request"),
            response: type_ref(&v["response"]).expect("a response"),
            error: type_ref(&v["error"]),
            fanout: serde_json::from_value::<Fanout>(v["fanout"].clone()).expect("fanout"),
            idempotent: v["idempotent"].as_bool().expect("idempotent"),
            serving: serde_json::from_value::<Serving>(v["serving"].clone()).expect("serving"),
            replies: serde_json::from_value::<Replies>(v["replies"].clone()).expect("replies"),
        }
    } else {
        ResBody::Data {
            type_: type_ref(&v["type"]).expect("a type"),
            rate: v["rate"].as_str().map(|r| Rate::parse(r).expect("a rate")),
            retention_s: v["retention"].as_str().map(retention_s),
        }
    };
    Res {
        template: template.to_owned(),
        kind,
        params: serde_json::from_value(v["params"].clone()).expect("params"),
        optional: v["optional"].as_bool().expect("optional"),
        annotations: serde_json::from_value(v["annotations"].clone()).expect("annotations"),
        body,
    }
}

/// A `contracts` case's resources: the test contract resolved, the case's
/// `uses`, `resources` and `types` in place of its own.
fn case_resources(doc: &Value, case: &Value) -> Resources {
    let mut resources: BTreeMap<String, Value> =
        serde_json::from_value(doc["contracts_base"]["resources"].clone()).expect("resources");
    if let Some(m) = case["resources"].as_object() {
        for (t, r) in m {
            if r.is_null() {
                resources.remove(t);
            } else {
                resources.insert(t.clone(), r.clone());
            }
        }
    }
    let mut packed_doc = doc["contracts_packed"].clone();
    if let Some(m) = case["types"].as_object() {
        for (n, t) in m {
            packed_doc["types"][n] = t.clone();
        }
    }
    let uses = if case["uses"].is_null() {
        &doc["contracts_base"]["uses"]
    } else {
        &case["uses"]
    };
    Resources {
        uses: serde_json::from_value(uses.clone()).expect("uses"),
        resources: resources.iter().map(|(t, r)| resource(t, r)).collect(),
        packed: vec![("rfcfg".to_owned(), packed_doc)],
    }
}

fn derived_json(i: &Interface) -> (Value, Value) {
    let mut fields = serde_json::Map::new();
    let mut layouts = serde_json::Map::new();
    for n in i.derived.names() {
        let d = i.derived.declaration(n).expect("a derived type");
        fields.insert(n.to_owned(), json!({ "fields": d["fields"] }));
        layouts.insert(
            n.to_owned(),
            layout_json(i.derived.layout(n).expect("a layout")),
        );
    }
    (Value::Object(fields), Value::Object(layouts))
}

/// `config/conformance/v2/derive.json`: `config.groups` read and its types
/// derived, or the reasons it is invalid; and the contract's lint
/// (config.v2 §2.2, §3).
#[test]
fn config_v2_derive() {
    let doc = read_json(&dir().join("v2/derive.json"));
    let files = [("schema", &doc["schema"])];
    for case in doc["cases"].as_array().expect("cases") {
        let note = case["note"].as_str().expect("note");
        let got = match Interface::read(&case["groups"], &files) {
            Ok(i) => {
                let (derived, layouts) = derived_json(&i);
                json!({
                    "declaration": "declared",
                    "groups": i.groups.to_json(),
                    "derived": derived,
                    "layouts": layouts,
                })
            }
            Err(reasons) => json!({
                "declaration": "invalid",
                "reasons": reasons.iter().map(|r| json!({
                    "entry": r.entry,
                    "reason": r.reason.token(),
                })).collect::<Vec<_>>(),
            }),
        };
        assert_eq!(case["expect"], got, "{note}");
    }
    // The test contract's groups, and its standard schema by artifact id.
    let c = rfcfg();
    let i = Interface::of(&c).expect("rfcfg.v1's groups");
    assert_eq!(
        doc["contract_groups"],
        i.groups.to_json(),
        "contract_groups"
    );
    let id = doc["standard_schema"]["id"].as_str().expect("id");
    assert_eq!(id, v2::STANDARD_SCHEMA);
    let event = c
        .resource(zenkey_model::grammar::KindToken::Events, v2::CHANGE)
        .expect("events/change");
    match &event.body {
        zenkey_model::contract::Body::Data(d) => assert_eq!(
            d.type_,
            zenkey_model::schema::TypeId::JsonSchema {
                name: v2::EVENT_TYPE.to_owned(),
                schema: id.to_owned()
            },
            "the standard schema's id is the sha256 of its JCS bytes"
        ),
        zenkey_model::contract::Body::Operation(_) => panic!("change is an event"),
    }
    for case in doc["contracts"].as_array().expect("contracts") {
        let note = case["note"].as_str().expect("note");
        let resources = match case["contract"].as_str() {
            Some(p) => {
                let l = load_path(&dir().join(p));
                Resources::of(
                    &l.contract
                        .unwrap_or_else(|| panic!("{note}: {p} does not load:\n{}", l.report)),
                )
            }
            None => case_resources(&doc, case),
        };
        let got: Vec<Value> = v2::lint(&resources)
            .iter()
            .map(|f| json!({"at": f.at, "reason": f.reason.token()}))
            .collect();
        assert_eq!(case["expect"]["findings"], json!(got), "{note}");
    }
}

/// `zk2 config derive`'s output is the test contract's packed file's eight
/// derived types, descriptions included.
#[test]
fn config_v2_derived_types_are_rfcfgs() {
    let i = rfcfg_interface();
    let packed_doc = read_json(&dir().join("contracts/rfcfg.json"));
    let text: Value = serde_json::from_str(&i.derived.to_text()).expect("JSON");
    let names: Vec<&str> = i.derived.names().collect();
    assert_eq!(
        names,
        [
            "Record", "SetDuty", "SetRadio", "SetLink", "SetSim", "SetQueue", "Control", "Error"
        ]
    );
    for n in names {
        assert_eq!(text["types"][n], packed_doc["types"][n], "{n}");
    }
    assert_eq!(
        text["types"].as_object().map(serde_json::Map::len),
        Some(8),
        "the derived types alone"
    );
    assert!(v2::lint_contract(&rfcfg()).is_empty());
}

// --- records.json -------------------------------------------------------------------

/// A revision's interface from `derive.json`'s spelling of its groups: the
/// annotation and a packed file of the group types it names.
fn interface_of_groups(groups: &Value) -> Interface {
    let mut annotation = Vec::new();
    let mut types = serde_json::Map::new();
    for g in groups.as_array().expect("groups") {
        if g == "..." {
            annotation.push(json!("..."));
            continue;
        }
        let mut entry = json!({
            "name": g["name"],
            "class": g["class"],
            "type": format!("packed:{}", g["type"].as_str().expect("type")),
        });
        let mut units = serde_json::Map::new();
        let mut sensitive = Vec::new();
        let mut fields = Vec::new();
        for p in g["parameters"].as_array().expect("parameters") {
            let mut f = json!({"name": p["name"], "type": p["kind"]});
            if p["kind"] == "int" {
                f["minimum"] = p["minimum"].clone();
                f["maximum"] = p["maximum"].clone();
            }
            fields.push(f);
            if let Some(u) = p["unit"].as_str() {
                units.insert(p["name"].as_str().expect("name").into(), json!(u));
            }
            if p["sensitive"] == true {
                sensitive.push(p["name"].clone());
            }
        }
        if !units.is_empty() {
            entry["units"] = Value::Object(units);
        }
        if !sensitive.is_empty() {
            entry["sensitive"] = json!(sensitive);
        }
        types.insert(
            g["type"].as_str().expect("type").into(),
            json!({ "fields": fields }),
        );
        annotation.push(entry);
    }
    let doc = json!({ "types": types });
    let i = Interface::read(&Value::Array(annotation), &[("growth", &doc)])
        .expect("a growth revision's groups");
    assert_eq!(&i.groups.to_json(), groups, "a growth revision read back");
    i
}

fn read_json_state(i: &Interface, tag: u8, bytes: &[u8], epoch: i64) -> Value {
    match i.read_record(tag, bytes, epoch) {
        Ok(s) => json!({ "state": s }),
        Err(r) => json!({ "refused": r.token() }),
    }
}

/// `config/conformance/v2/records.json`: an owner's state written as a
/// record, its put and its reply; records read back or refused; and two
/// revisions read by each other (config.v2 §2.3, §3).
#[test]
fn config_v2_records() {
    let doc = read_json(&dir().join("v2/records.json"));
    let i = rfcfg_interface();
    assert_eq!(doc["layout"], layout_json(i.derived.record()), "layout");
    let tag = Tag::new(i.derived.record().tag()).expect("a tag");
    for w in doc["writes"].as_array().expect("writes") {
        let note = w["note"].as_str().expect("note");
        let epoch = w["epoch"].as_i64().expect("epoch");
        let state: State = serde_json::from_value(w["state"].clone()).expect("a state");
        let rec = i.write_record(&state, epoch).expect("written");
        let stamp = stamp_of(&w["stamp"]);
        let got = json!({
            "fields": i.fields(&state),
            "bits": rec.bit_string(),
            "hex": hex(&rec.bytes),
            "bytes": rec.bytes.len(),
            "mutation": hex(&Envelope::mutation(tag, stamp, &rec.bytes).write()),
            "value": hex(&Envelope::value(tag, stamp, &rec.bytes).write()),
        });
        assert_eq!(w["expect"], got, "{note}");
        assert_eq!(
            i.read_record(tag.get(), &rec.bytes, epoch).as_ref(),
            Ok(&state),
            "{note}: read back"
        );
    }
    for r in doc["reads"].as_array().expect("reads") {
        let note = r["note"].as_str().expect("note");
        let got = read_json_state(
            &i,
            u8::try_from(r["tag"].as_u64().expect("tag")).expect("a tag"),
            &unhex(r["hex"].as_str().expect("hex")),
            r["epoch"].as_i64().expect("epoch"),
        );
        assert_eq!(r["expect"], got, "{note}");
    }
    let g = &doc["growth"];
    let revisions: BTreeMap<&str, Interface> = g["groups"]
        .as_object()
        .expect("groups")
        .iter()
        .map(|(n, gs)| (n.as_str(), interface_of_groups(gs)))
        .collect();
    let tags: BTreeSet<u8> = revisions
        .values()
        .map(|i| i.derived.record().tag())
        .collect();
    assert_eq!(tags.len(), 1, "growth behind the marker keeps Record's tag");
    let tag = *tags.first().expect("a tag");
    for r in g["records"]
        .as_array()
        .expect("records")
        .iter()
        .chain(g["reads"].as_array().expect("reads"))
    {
        let reader = &revisions[r["read_by"].as_str().expect("read_by")];
        let got = read_json_state(reader, tag, &unhex(r["hex"].as_str().expect("hex")), 0);
        assert_eq!(r["expect"], got, "{r}");
    }
}

// --- validation.json and window.json --------------------------------------------------

fn facts_of(s: &Value) -> v2::Facts {
    v2::Facts {
        revision: u32::try_from(s["revision"].as_u64().expect("revision")).expect("32 bits"),
        pending: (!s["pending"].is_null()).then(|| v2::PendingFacts {
            token: s["pending"]["token"].as_u64().expect("token"),
            applied: s["pending"]["applied"].as_bool().expect("applied"),
            deadline_passed: s["pending"]["deadline_passed"]
                .as_bool()
                .expect("deadline_passed"),
        }),
        last: (!s["last"].is_null()).then(|| v2::LastFacts {
            token: s["last"]["token"].as_u64().expect("token"),
            set_sensitive: s["last"]["set_sensitive"].as_bool().expect("set_sensitive"),
        }),
        persist: s["persist"].as_bool().expect("persist"),
        in_flight: s["in_flight"].as_bool().expect("in_flight"),
        writable: serde_json::from_value(s["writable"].clone()).expect("writable"),
    }
}

fn control_op(op: &str) -> ControlOp {
    match op {
        "confirm" => ControlOp::Confirm,
        "cancel" => ControlOp::Cancel,
        "extend" => ControlOp::Extend,
        "persist" => ControlOp::Persist,
        other => panic!("op {other:?}"),
    }
}

/// `config/conformance/v2/validation.json`: one request against an owner's
/// state, accepted or refused in the error form; and desired documents
/// refused whole (config.v2 §2.5, §2.10, §2.13, §2.18).
#[test]
fn config_v2_validation() {
    let doc = read_json(&dir().join("v2/validation.json"));
    let i = rfcfg_interface();
    let stamp = stamp_of(&doc["stamp"]);
    assert_eq!(
        doc["error_tag"],
        json!(i.derived.error().tag()),
        "Error's tag"
    );
    for case in doc["cases"].as_array().expect("cases") {
        let note = case["note"].as_str().expect("note");
        let payload = unhex(case["hex"].as_str().expect("hex"));
        let facts = facts_of(&case["state"]);
        let op = case["op"].as_str().expect("op");
        // The request's JSON reading, where it parses, both ways.
        if !case["fields"].is_null() {
            if op == "set" {
                let g = case["group"].as_str().expect("group");
                let (s, fields) = i
                    .decode_set(g, &payload)
                    .expect("a settable group")
                    .expect("parses");
                assert_eq!(case["fields"], fields, "{note}: the request read");
                assert_eq!(s.to_json(), fields, "{note}: Set's reading");
                assert_eq!(
                    i.encode_set(g, &s).as_deref(),
                    Ok(payload.as_slice()),
                    "{note}"
                );
            } else {
                let c = i.decode_control(&payload).expect("parses");
                assert_eq!(case["fields"], c.to_json(), "{note}: the request read");
                assert_eq!(
                    i.encode_control(&c).as_deref(),
                    Ok(payload.as_slice()),
                    "{note}"
                );
            }
        }
        let result = if op == "set" {
            v2::validate_set(&i, &facts, case["group"].as_str().expect("group"), &payload)
                .expect("a settable group")
        } else {
            v2::validate_control(&i, control_op(op), &facts, &payload)
        };
        let got = match super::under_encoding(case, result) {
            Ok(()) => json!({"accepted": true}),
            Err(r) => json!({
                "refused": {
                    "code": r.code.as_str(),
                    "cause": r.cause(),
                    "app": r.app.as_ref().map(|_| v2::DEVICE_REFUSED),
                },
                "reply_err": hex(&i.error(&r, stamp).payload),
            }),
        };
        assert_eq!(case["expect"], got, "{note}");
    }
    for case in doc["desired"].as_array().expect("desired") {
        let note = case["note"].as_str().expect("note");
        let d: DesiredConfig =
            serde_json::from_value(case["document"].clone()).expect("a document");
        let writable: BTreeSet<String> =
            serde_json::from_value(case["writable"].clone()).expect("writable");
        let got = match v2::check_desired(&i, &writable, &d) {
            Ok(()) => json!({"accepted": true}),
            Err(r) => json!({"refused": {
                "code": r.code.map(v2::DesiredCode::as_str),
                "group": r.group,
                "parameter": r.parameter,
            }}),
        };
        assert_eq!(case["expect"], got, "{note}");
    }
}

/// `config/conformance/v2/window.json`: the window, the deadline and
/// `apply_at`, rounded up to whole seconds, and their codes (config.v2
/// §2.8).
#[test]
fn config_v2_window() {
    let doc = read_json(&dir().join("v2/window.json"));
    for case in doc["cases"].as_array().expect("cases") {
        let note = case["note"].as_str().expect("note");
        let n = |k: &str| case[k].as_i64().unwrap_or_else(|| panic!("{note}: {k}"));
        let cs = u16::try_from(n("confirm_s")).expect("confirm_s");
        let fs = u16::try_from(n("floor_s")).expect("floor_s");
        let epoch = n("epoch");
        let code =
            |t: Time| packed::time_code(t.millis().into(), Some(epoch), false).expect("a time");
        let deadline = v2::deadline(Time::from_millis(n("received")), cs, fs);
        let mut got = json!({
            "window_s": config::window(cs.into(), fs.into()),
            "deadline": deadline.millis(),
            "deadline_code": code(deadline),
        });
        if let Some(d) = case["device_apply_at"].as_i64() {
            let a = v2::apply_at(Time::from_millis(d));
            got["apply_at"] = json!(a.millis());
            got["apply_at_code"] = json!(code(a));
        }
        assert_eq!(case["expect"], got, "{note}");
    }
}

// --- lifecycle.json and redaction.json ------------------------------------------------

/// The runs a fixture's owner takes, in order, each with its token
/// counter's start, and the first run's first seq.
struct Runs {
    runs: Vec<(Run, u16)>,
    at: usize,
    next: u32,
    last: Option<u32>,
}

impl Runs {
    fn of(setup: &Value) -> Self {
        let counters = setup["counters"].as_array().expect("counters");
        let runs = setup["runs"]
            .as_array()
            .expect("runs")
            .iter()
            .enumerate()
            .map(|(k, r)| {
                let n = u32::try_from(r["run"].as_u64().expect("run")).expect("a run");
                let run = match r["width"].as_u64() {
                    Some(32) => Run::U32(n),
                    _ => Run::U16(u16::try_from(n).expect("16 bits")),
                };
                let c = counters
                    .get(k)
                    .and_then(Value::as_u64)
                    .map_or(0, |c| u16::try_from(c).expect("a counter"));
                (run, c)
            })
            .collect();
        Self {
            runs,
            at: 0,
            next: setup["seq_start"]
                .as_u64()
                .map_or(0, |s| u32::try_from(s).expect("a seq")),
            last: None,
        }
    }
}

impl Mint for Runs {
    fn run(&self) -> Run {
        self.runs[self.at].0
    }

    fn last_seq(&self) -> Option<Seq> {
        self.last.and_then(Seq::new)
    }

    fn stamp(&mut self) -> Stamp {
        if self.next > Seq::MAX.get() {
            self.next_run();
        }
        let seq = self.next;
        self.next += 1;
        self.last = Some(seq);
        Stamp {
            run: self.run(),
            seq: Seq::new(seq).expect("a seq"),
        }
    }

    fn next_run(&mut self) {
        self.at += 1;
        assert!(self.at < self.runs.len(), "the fixture lists no run more");
        self.next = 0;
        self.last = None;
    }

    fn counter_start(&mut self) -> u16 {
        self.runs[self.at].1
    }
}

fn owner_of(setup: &Value, runs: &mut Runs) -> (v2::Owner, Output) {
    let startup: Layer<(config::Value, config::Source)> =
        serde_json::from_value::<Layer<serde_json::Map<String, Value>>>(setup["startup"].clone())
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
    v2::Owner::start(
        rfcfg_interface(),
        Start {
            service: setup["service"].as_str().expect("service").into(),
            startup,
            overlay: serde_json::from_value(setup["overlay"].clone()).expect("overlay"),
            floor_s: u16::try_from(setup["floor_s"].as_u64().expect("floor_s")).expect("16 bits"),
            apply_after: Duration::from_millis(
                setup["apply_after_ms"].as_u64().expect("apply_after_ms"),
            ),
            persist: setup["persist"].as_bool().expect("persist"),
            writable: serde_json::from_value(setup["writable"].clone()).expect("writable"),
            events: Vec::new(),
            epoch: setup["epoch"].as_i64().expect("epoch"),
        },
        runs,
    )
    .expect("a valid setup")
}

/// What one step wrote.
#[derive(Default)]
struct Wrote {
    acts: Vec<&'static str>,
    out: Output,
}

impl Wrote {
    fn take(&mut self, out: Output) {
        self.out.puts.extend(out.puts);
        if out.reply.is_some() {
            self.out.reply = out.reply;
        }
        self.out.events.extend(out.events);
    }

    fn json(&self) -> Value {
        json!({
            "acts": self.acts,
            "puts": self.out.puts.iter().map(|p| json!({
                "run": p.stamp.run.value(),
                "seq": p.stamp.seq.get(),
                "record": p.state,
                "hex": hex(&p.payload),
            })).collect::<Vec<_>>(),
            "reply": self.out.reply.as_ref().map_or(Value::Null, reply_json),
            "events": self.out.events.iter().map(|e| json!({
                "run": e.stamp.run.value(),
                "seq": e.stamp.seq.get(),
                "event": e.event,
            })).collect::<Vec<_>>(),
        })
    }
}

fn reply_json(r: &Reply) -> Value {
    match &r.answer {
        Answer::Value { stamp, state } => json!({"value": {
            "run": stamp.run.value(),
            "seq": stamp.seq.get(),
            "record": state,
            "hex": hex(&r.payload),
        }}),
        Answer::Error {
            stamp,
            code,
            cause,
            app,
            message,
        } => json!({"error": {
            "run": stamp.run.value(),
            "seq": stamp.seq.get(),
            "code": code.as_str(),
            "cause": cause.map(|c| c.as_str()),
            "app": app.then_some(v2::DEVICE_REFUSED),
            "message": message,
            "hex": hex(&r.payload),
        }}),
    }
}

/// Runs a step to its answer, each act with the next device result.
fn drive(
    owner: &mut v2::Owner,
    now: Time,
    mut step: Step,
    device: &mut VecDeque<Value>,
    runs: &mut Runs,
    wrote: &mut Wrote,
) {
    loop {
        match step {
            Step::Done(out) => {
                wrote.take(out);
                return;
            }
            Step::Act(a) => {
                wrote.acts.push(match a {
                    config::Act::Apply { .. } => "apply",
                    config::Act::Undo { .. } => "undo",
                });
                let r = device.pop_front().unwrap_or(json!("ok"));
                step = owner.finish(now, device_result(&r), runs);
            }
        }
    }
}

/// Runs one lifecycle step.
fn run_step(owner: &mut v2::Owner, step: &Value, runs: &mut Runs) -> Wrote {
    let now = Time::from_millis(step["t"].as_i64().expect("t"));
    let mut device: VecDeque<Value> = step["device"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into();
    let call = Call {
        actor: step["call"]["actor"].as_str().map(str::to_owned),
        request_id: step["call"]["request_id"].as_str().map(str::to_owned),
    };
    let payload = step["hex"].as_str().map(unhex).unwrap_or_default();
    let mut wrote = Wrote::default();
    match step["do"].as_str().expect("do") {
        "set" => {
            let s = owner.set(
                now,
                step["group"].as_str().expect("group"),
                &payload,
                &call,
                runs,
            );
            drive(owner, now, s, &mut device, runs, &mut wrote);
        }
        "confirm" => {
            let s = owner.confirm(now, &payload, &call, runs);
            drive(owner, now, s, &mut device, runs, &mut wrote);
        }
        "cancel" => {
            let s = owner.cancel(now, &payload, &call, runs);
            drive(owner, now, s, &mut device, runs, &mut wrote);
        }
        "extend" => {
            let s = owner.extend(now, &payload, &call, runs);
            drive(owner, now, s, &mut device, runs, &mut wrote);
        }
        "persist" => match owner.persist(now, &payload, runs) {
            Ok(plan) => {
                let out = owner.persisted(plan, runs);
                wrote.take(out);
            }
            Err(out) => wrote.take(out),
        },
        "due" => {
            while let Some(s) = owner.due(now, runs) {
                drive(owner, now, s, &mut device, runs, &mut wrote);
                if owner.in_flight() {
                    break;
                }
            }
        }
        "restart" => {
            let (next, out) = owner.restart(runs);
            *owner = next;
            wrote.take(out);
        }
        other => panic!("do {other:?}"),
    }
    wrote
}

fn start_json(out: Output) -> Value {
    Wrote {
        acts: Vec::new(),
        out,
    }
    .json()
}

/// `config/conformance/v2/lifecycle.json`: one owner's puts, replies and
/// events, step by step (config.v2 §2.3, §2.5 to §2.14, §2.17).
#[test]
fn config_v2_lifecycle() {
    let doc = read_json(&dir().join("v2/lifecycle.json"));
    for case in doc["cases"].as_array().expect("cases") {
        let note = case["note"].as_str().expect("note");
        let mut runs = Runs::of(&case["setup"]);
        let (mut owner, start) = owner_of(&case["setup"], &mut runs);
        assert_eq!(case["start"], start_json(start), "{note}: the start");
        for (k, step) in case["steps"].as_array().expect("steps").iter().enumerate() {
            let got = run_step(&mut owner, step, &mut runs).json();
            assert_eq!(
                step["expect"], got,
                "{note}, step {k} ({} at t={})",
                step["do"], step["t"]
            );
        }
    }
}

/// `config/conformance/v2/redaction.json`: what carries a sensitive value
/// (config.v2 §2.15).
#[test]
fn config_v2_redaction() {
    let doc = read_json(&dir().join("v2/redaction.json"));
    let i = rfcfg_interface();
    for case in doc["cases"].as_array().expect("cases") {
        let note = case["note"].as_str().expect("note");
        let mut runs = Runs::of(&case["setup"]);
        let (mut owner, _) = owner_of(&case["setup"], &mut runs);
        for (k, step) in case["before"]
            .as_array()
            .expect("before")
            .iter()
            .enumerate()
        {
            let got = run_step(&mut owner, step, &mut runs).json();
            assert_eq!(step["expect"], got, "{note}, before {k}");
        }
        let payload = unhex(case["hex"].as_str().expect("hex"));
        let now = Time::from_millis(0);
        let mut wrote = Wrote::default();
        let log = match case["op"].as_str().expect("op") {
            "set" => {
                let mut device: VecDeque<Value> = case["device_words"]
                    .as_str()
                    .map(|w| json!({ "refused": w }))
                    .into_iter()
                    .collect();
                let s = owner.set(now, "sim", &payload, &Call::default(), &mut runs);
                drive(&mut owner, now, s, &mut device, &mut runs, &mut wrote);
                match i.decode_set("sim", &payload).expect("sim is settable") {
                    Ok((_, fields)) => i.log_set("sim", &fields),
                    Err(_) => Value::Null,
                }
            }
            "persist" => {
                match owner.persist(now, &payload, &mut runs) {
                    Ok(plan) => {
                        let out = owner.persisted(plan, &mut runs);
                        wrote.take(out);
                    }
                    Err(out) => wrote.take(out),
                }
                i.decode_control(&payload)
                    .map_or(Value::Null, |c| c.to_json())
            }
            other => panic!("op {other:?}"),
        };
        let state = match wrote.out.reply.as_ref().map(|r| &r.answer) {
            Some(Answer::Value { state, .. }) => (**state).clone(),
            _ => owner.state(),
        };
        let sim: serde_json::Map<String, Value> = i
            .fields(&state)
            .as_object()
            .expect("fields")
            .iter()
            .filter(|(k, _)| k.starts_with("sim__"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let got = json!({
            "sim": sim,
            "reply": wrote.out.reply.as_ref().map_or(Value::Null, reply_json),
            "edits": wrote.out.events.first().map_or(Value::Null, |e| json!(e.event.edits)),
            "log": log,
        });
        assert_eq!(case["expect"], got, "{note}");
    }
    // No bit of the record depends on a sensitive value.
    let setup = &doc["cases"][0]["setup"];
    let mut puts = Vec::new();
    for pair in doc["same_bytes"].as_array().expect("same_bytes") {
        let mut runs = Runs::of(setup);
        let (mut owner, _) = owner_of(setup, &mut runs);
        let payload = i
            .encode_set("sim", &set_of(&i, &pair["fields"]))
            .expect("a SetSim");
        let mut wrote = Wrote::default();
        let s = owner.set(
            Time::from_millis(0),
            "sim",
            &payload,
            &Call::default(),
            &mut runs,
        );
        drive(
            &mut owner,
            Time::from_millis(0),
            s,
            &mut VecDeque::new(),
            &mut runs,
            &mut wrote,
        );
        let put = hex(&wrote.out.puts.first().expect("a put").payload);
        assert_eq!(pair["hex"], json!(put), "same_bytes {}", pair["fields"]);
        puts.push(put);
    }
    assert!(puts.windows(2).all(|w| w[0] == w[1]), "equal byte for byte");
}

/// A `SetSim` from its JSON reading.
fn set_of(i: &Interface, fields: &Value) -> v2::Set {
    let payload = {
        let layout = i.derived.set("sim").expect("SetSim").1;
        let rec = layout.write(fields, None).expect("a SetSim");
        zenkey_model::wire::Request::new(Tag::new(layout.tag()).expect("a tag"), &rec.bytes).write()
    };
    i.decode_set("sim", &payload)
        .expect("sim is settable")
        .expect("parses")
        .0
}
