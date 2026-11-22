//! `desired.v2`'s fixtures (`spec/profiles/desired/conformance/v2/`, #986's
//! J2), run against `zenkey_model::desired::v2` as written. Each
//! expectation was computed twice by two scripts of the text's drafter and
//! never blessed, and no runner here blesses one: a failure is the
//! model's, or a fixture or a sentence to raise with the bytes.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value, json};
use zenkey_model::authoring::Kind;
use zenkey_model::desired::v2::{
    self, Answered, Beat, Decision, Found, Index, ReportAt, ReportDeclaration, ReportResource,
    Section, Tables, Target, Watch, Word,
};
use zenkey_model::desired::{self, Mutation, Via};
use zenkey_model::freshness;
use zenkey_model::grammar::{Addr, IfaceId, KindToken};
use zenkey_model::heartbeat::Digest;
use zenkey_model::packed;
use zenkey_model::schema::TypeId;
use zenkey_model::template::Template;
use zenkey_model::wire::{Run, Seq, Stamp};

use super::{
    desired_declaration_json, desired_judged, desired_resource, desired_seen, jcs_id, profiles,
    read_json, repo_root, unhex,
};

fn dir() -> PathBuf {
    profiles().join("desired/conformance/v2")
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn version(v: &Value) -> Stamp {
    let text = v.as_str().expect("a version");
    let got = v2::parse_version(text).unwrap_or_else(|| panic!("{text:?} is no version"));
    assert_eq!(v2::version_text(got), text, "a version writes back");
    got
}

fn run(v: &Value) -> Run {
    Run::from_hex(v.as_str().expect("a run")).expect("4 or 8 hexadecimal digits")
}

fn seq(v: &Value) -> Seq {
    Seq::new(u32::try_from(v.as_u64().expect("a seq")).expect("a seq")).expect("a seq")
}

fn tag(v: &Value) -> u8 {
    u8::try_from(v.as_u64().expect("a tag")).expect("a tag")
}

fn findings_json(f: &[desired::Finding]) -> Value {
    json!(f.iter().map(|f| f.as_str()).collect::<Vec<_>>())
}

/// `v2/schema.json`: the standard packed schema's published ids, and the
/// `Report`'s tag (desired.v2 §2.10, §3). The model's own layout, which no
/// crate may read from `spec/`, is the file's.
#[test]
fn desired_v2_schema() {
    let doc = read_json(&dir().join("schema.json"));
    let file = read_json(
        &profiles()
            .join("desired")
            .join(doc["file"].as_str().expect("file")),
    );
    assert_eq!(
        json!(jcs_id(&file)),
        doc["current"],
        "the profile's file is the current revision"
    );
    let published: Vec<&str> = doc["published"]
        .as_array()
        .expect("published")
        .iter()
        .map(|p| p["id"].as_str().expect("id"))
        .collect();
    assert_eq!(published, v2::PUBLISHED, "desired::v2::PUBLISHED");
    assert_eq!(published.last(), doc["current"].as_str().as_ref());
    let types = doc["types"].as_array().expect("types");
    assert_eq!(types.len(), 1);
    assert_eq!(types[0]["name"], v2::REPORT);
    assert_eq!(types[0]["tag"], json!(v2::TAG));
    let of_file = packed::layout_in(&file, v2::REPORT).expect("the file's Report");
    assert_eq!(of_file.tag(), v2::TAG);
    assert_eq!(
        v2::layout(),
        &of_file,
        "the model's layout is the standard schema's, names included"
    );
    let root = repo_root();
    let cases = doc["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 3);
    for case in cases {
        let document = match case["path"].as_str() {
            Some(p) => read_json(&root.join(p)),
            None => case["document"].clone(),
        };
        let id = jcs_id(&document);
        let got = json!({"id": id, "published": v2::is_published(&id)});
        assert_eq!(case["expect"], got, "{}", case["note"]);
    }
}

fn tables(case: &Value) -> Option<Tables> {
    if case["pointers"].is_null() {
        assert!(case["details"].is_null());
        return None;
    }
    Some(Tables {
        pointers: serde_json::from_value(case["pointers"].clone()).expect("pointers"),
        details: serde_json::from_value(case["details"].clone()).expect("details"),
    })
}

fn index_json(i: Index) -> Value {
    match i {
        Index::At(i) => json!(i),
        Index::OutOfRange => json!(v2::OUT_OF_RANGE),
    }
}

fn index_of(v: &Value) -> Index {
    match v {
        Value::String(s) if s == v2::OUT_OF_RANGE => Index::OutOfRange,
        v => Index::At(u8::try_from(v.as_u64().expect("an index")).expect("an index")),
    }
}

fn word_json(w: Word<'_>) -> Value {
    match w {
        Word::Entry(e) => json!(e),
        Word::Number(_) => Value::Null,
        Word::OutOfRange => json!(v2::OUT_OF_RANGE),
    }
}

/// `v2/reports.json`: the packed `Report` written, read, refused and
/// found malformed, and a refusal's place and line as indices and as
/// words (desired.v2 §2.10, §3).
#[test]
fn desired_v2_reports() {
    let doc = read_json(&dir().join("reports.json"));
    let l = v2::layout();
    assert_eq!(
        doc["layout"],
        json!({
            "form": l.form(),
            "sha256": l.sha256_hex(),
            "tag": l.tag(),
            "min_bits": l.min_bits(),
            "max_bits": l.max_bits(),
        }),
        "the Report's layout"
    );
    let writes = doc["writes"].as_array().expect("writes");
    assert_eq!(writes.len(), 22);
    for case in writes {
        let what = format!("write: {}", case["note"]);
        let written = v2::Report::from_reading(&case["report"]).and_then(|r| {
            let fields = r.fields()?;
            let record = fields.record()?;
            assert_eq!(r.write().as_ref(), Ok(&record), "{what}");
            let mut got = json!({
                "fields": fields.to_json(),
                "bits": record.bit_string(),
                "pad": record.pad(),
                "hex": hex(&record.bytes),
            });
            if let Some(stamp) = case.get("stamp") {
                got["payload"] = json!(hex(&r.put(version(stamp))?));
            }
            // What a writer wrote, a reader reads back.
            let back = v2::read(v2::TAG, &record.bytes).expect("it reads back");
            assert_eq!(back.fields, fields, "{what}");
            Ok(got)
        });
        let got = written.unwrap_or_else(|e| json!({"refused": e.token()}));
        assert_eq!(case["expect"], got, "{what}");
    }
    let reads = doc["reads"].as_array().expect("reads");
    assert_eq!(reads.len(), 57);
    for case in reads {
        let what = format!("read: {}", case["note"]);
        let bytes = unhex(case["hex"].as_str().expect("hex"));
        let got = match v2::read(tag(&case["tag"]), &bytes) {
            Err(e) => json!({"refused": e.token()}),
            Ok(read) => {
                let mut got = json!({
                    "fields": read.fields.to_json(),
                    "findings": findings_json(&read.findings),
                });
                assert_eq!(read.report.is_some(), read.findings.is_empty(), "{what}");
                if let Some(r) = &read.report {
                    got["report"] = r.reading();
                    assert_eq!(v2::Report::from_reading(&r.reading()).as_ref(), Ok(r));
                }
                got
            }
        };
        assert_eq!(case["expect"], got, "{what}");
    }
    let places = doc["places"].as_array().expect("places");
    assert_eq!(places.len(), 12);
    for case in places {
        let t = tables(case).expect("a target holds its tables");
        let mut got = serde_json::Map::new();
        if let Some(place) = case["refusal"]["place"].as_str() {
            got.insert("pointer".into(), index_json(t.place(place)));
        }
        if let Some(line) = case["refusal"]["line"].as_str() {
            got.insert("detail".into(), index_json(t.line(line)));
        }
        assert_eq!(
            case["expect"],
            Value::Object(got),
            "place: {}",
            case["note"]
        );
    }
    let words = doc["words"].as_array().expect("words");
    assert_eq!(words.len(), 6);
    for case in words {
        let t = tables(case);
        let mut got = serde_json::Map::new();
        if let Some(p) = case["refusal"].get("pointer") {
            let table = t.as_ref().map(|t| t.pointers.as_slice());
            got.insert("pointer".into(), word_json(v2::word(table, index_of(p))));
        }
        if let Some(d) = case["refusal"].get("detail") {
            let table = t.as_ref().map(|t| t.details.as_slice());
            got.insert("detail".into(), word_json(v2::word(table, index_of(d))));
        }
        assert_eq!(case["expect"], Value::Object(got), "word: {}", case["note"]);
    }
}

fn via(v: &Value) -> Via {
    match v.as_str().expect("from") {
        "owner" => Via::Owner,
        "archive" => Via::Archive,
        other => panic!("from {other:?}"),
    }
}

fn mutation(v: &Value) -> Mutation {
    match v.as_str().expect("kind") {
        "put" => Mutation::Put,
        "delete" => Mutation::Delete,
        other => panic!("kind {other:?}"),
    }
}

fn refusal(v: &Value) -> v2::Refusal {
    let code = v["code"].as_str().expect("a code");
    let named = v2::Code::of(code);
    assert_ne!(
        named,
        v2::Code::Own,
        "{code:?} is one of the profile's five"
    );
    v2::Refusal {
        code: named,
        pointer: v.get("pointer").map(index_of),
        detail: v.get("detail").map(index_of),
    }
}

/// `v2/order.json`: one target's steps → what each does, and its report
/// after the last (desired.v2 §2.4, §2.6 to §2.10).
#[test]
fn desired_v2_order() {
    let doc = read_json(&dir().join("order.json"));
    let cases = doc["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 67);
    for case in cases {
        let what = format!("{}", case["note"]);
        let mut t = Target::new();
        let mut steps = Vec::new();
        for step in case["steps"].as_array().expect("steps") {
            let token = if let Some(s) = step.get("sample") {
                if s["kind"] == "no_entry" {
                    assert_eq!(s["from"], "owner", "{what}: a snapshot is its author's");
                    let snapshot = &s["snapshot"];
                    t.observe_no_entry(run(&snapshot["run"]), seq(&snapshot["asof"]))
                        .as_str()
                } else {
                    let sample = v2::Sample {
                        via: via(&s["from"]),
                        mutation: mutation(&s["kind"]),
                        version: (!s["version"].is_null()).then(|| version(&s["version"])),
                        revision: s.get("revision").map(|r| r.as_u64().expect("a revision")),
                        content: match (s.get("document"), s.get("bytes")) {
                            (Some(d), _) => Some(desired::Content::of_json(d)),
                            (None, Some(h)) => {
                                Some(desired::Content::of_bytes(&unhex(h.as_str().expect("hex"))))
                            }
                            (None, None) => None,
                        },
                    };
                    t.observe(sample).as_str()
                }
            } else if let Some(f) = step.get("finish") {
                let ended = if f == "done" {
                    v2::Ended::Done
                } else if f == "check" {
                    // §2.4's bound judges the running act's put.
                    let running = t.running().expect("a check finishes a running act");
                    assert_eq!(running.kind, desired::ActKind::Apply, "{what}");
                    running.bound().map_or(v2::Ended::Done, v2::Ended::Refused)
                } else if let Some(r) = f.get("refused") {
                    v2::Ended::Refused(refusal(r))
                } else if let Some(r) = f.get("failed") {
                    v2::Ended::Failed(refusal(r))
                } else {
                    panic!("finish {f}")
                };
                t.finish(ended).as_str()
            } else if step.get("call").is_some() {
                t.call();
                "called"
            } else if step.get("restart").is_some() {
                t = Target::new();
                "restarted"
            } else {
                panic!("step {step}")
            };
            steps.push(token);
            // A target's report is consistent after every step (§2.10).
            let report = t.report();
            assert!(report.check().is_empty(), "{what}: {report:?}");
        }
        let got = json!({"steps": steps, "report": t.report().reading()});
        assert_eq!(case["expect"], got, "{what}");
    }
}

fn section(v: &Value) -> Option<Section> {
    if v.is_null() {
        return None;
    }
    let digest = u32::from_str_radix(v["digest"].as_str().expect("digest"), 16).expect("hex");
    Some(Section {
        digest: Digest::from_u32(digest),
        live: v["live"].as_u64().expect("live"),
    })
}

fn section_json(s: Option<Section>) -> Value {
    s.map_or(
        Value::Null,
        |s| json!({"digest": s.digest.to_string(), "live": s.live}),
    )
}

/// One version of the slice given to the target as its author's own, its
/// act, if it starts one, ended at once: every version here is a document
/// of its own.
fn admit(t: &mut Target, from: Via, kind: Mutation, v: Stamp) {
    let step = t.observe(v2::Sample {
        via: from,
        mutation: kind,
        version: Some(v),
        revision: None,
        content: (kind == Mutation::Put)
            .then(|| desired::Content::of_bytes(v2::version_text(v).as_bytes())),
    });
    match step {
        v2::Step::Act(_) => assert_eq!(t.finish(v2::Ended::Done), v2::Step::Done),
        v2::Step::Adopted => {}
        other => panic!("{} is not admitted: {other:?}", v2::version_text(v)),
    }
}

/// `v2/watch.json`: the watch's decision for each heartbeat and each
/// answer, and the section the target expects after it (desired.v2 §2.5).
/// The target is the model's [`Target`], so what it holds, the stamp its
/// read carries and what is last-known are the state machine's.
#[test]
fn desired_v2_watch() {
    let doc = read_json(&dir().join("watch.json"));
    let cases = doc["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 25);
    for case in cases {
        let what = format!("{}", case["note"]);
        let key = case["key"].as_str().expect("key");
        let mut t = Target::new();
        if let Some(h) = case["held"].as_object() {
            let from = if h.get("last_known") == Some(&json!(true)) {
                Via::Archive
            } else {
                Via::Owner
            };
            admit(&mut t, from, mutation(&h["kind"]), version(&h["version"]));
        }
        let mut w = Watch::new(key, case["moves"].as_bool().expect("moves"));
        // The held stamp the outstanding read carried.
        let mut sent: Option<Option<Stamp>> = None;
        let (mut steps, mut expects) = (Vec::new(), Vec::new());
        for step in case["steps"].as_array().expect("steps") {
            let token = if let Some(h) = step.get("heartbeat") {
                let beat = match h.get("read").and_then(Value::as_str) {
                    Some("revision") => Beat::Revision,
                    Some("refused") => Beat::Refused,
                    Some(other) => panic!("read {other:?}"),
                    None => {
                        let s = section(&h["section"]);
                        if let Some(members) = h.get("members").and_then(Value::as_array) {
                            // The section is core §7.5's digest of these.
                            let live: Vec<(&str, Stamp)> = members
                                .iter()
                                .map(|m| (m["key"].as_str().expect("key"), version(&m["version"])))
                                .collect();
                            let digest = Digest::of(live.iter().copied());
                            assert_eq!(
                                s,
                                Some(Section {
                                    digest,
                                    live: live.len() as u64
                                }),
                                "{what}: the section is its members' digest and count"
                            );
                        }
                        Beat::Read {
                            stamp: Stamp {
                                run: run(&h["run"]),
                                seq: seq(&h["seq"]),
                            },
                            section: s,
                        }
                    }
                };
                let decision = w.heartbeat(beat, t.held());
                if let Decision::Read { held } = decision {
                    assert!(sent.is_none(), "{what}: one read at a time");
                    assert_eq!(held, t.held(), "{what}: the read carries the held stamp");
                    sent = Some(held);
                }
                assert_eq!(w.outstanding(), sent.is_some(), "{what}");
                decision.as_str()
            } else if let Some(h) = step.get("heard") {
                let old = t.holds();
                admit(
                    &mut t,
                    Via::Owner,
                    mutation(&h["kind"]),
                    version(&h["version"]),
                );
                w.heard(old, t.holds().expect("it holds what it admitted"))
                    .as_str()
            } else if let Some(a) = step.get("answer") {
                let carried = sent.take().expect("an answer ends an outstanding read");
                let answer = match a {
                    Value::String(s) if s == "unchanged" => {
                        // The held stamp the target sent: its author's word.
                        t.unchanged(carried.expect("an unchanged answer names a stamp sent"));
                        Answered::Unchanged
                    }
                    Value::String(s) if s == "timeout" || s == "held" => Answered::NoAnswer,
                    a => {
                        let whole = &a["whole"];
                        if whole["kind"] == "no_entry" {
                            // The fixture gives a snapshot with no entry no
                            // run: it is no verdict only with nothing held.
                            assert_eq!(t.held(), None, "{what}");
                        } else {
                            admit(
                                &mut t,
                                Via::Owner,
                                mutation(&whole["kind"]),
                                version(&whole["version"]),
                            );
                        }
                        Answered::Whole
                    }
                };
                w.answer(answer, t.held()).as_str()
            } else {
                panic!("step {step}")
            };
            steps.push(token);
            expects.push(section_json(w.expects()));
        }
        let got = json!({
            "steps": steps,
            "expects": expects,
            "last_known": t.report().last_known,
        });
        assert_eq!(case["expect"], got, "{what}");
    }
}

fn type_of(v: &Value) -> TypeId {
    let name = v["name"].as_str().expect("name").to_owned();
    let schema = v["artifact"].as_str().expect("artifact").to_owned();
    match v["kind"].as_str().expect("kind") {
        "packed" => TypeId::Packed { name, schema },
        "jsonschema" => TypeId::JsonSchema { name, schema },
        other => panic!("type kind {other:?}"),
    }
}

/// A resource as `declarations.json`'s `reports` writes one.
struct Res {
    kind: Kind,
    token: KindToken,
    template: Template,
    is_report: bool,
    annotations: BTreeMap<String, Value>,
}

impl Res {
    fn of(r: &Value) -> Self {
        let (kind, template, _, annotations) = desired_resource(r);
        let explicit = r.get("explicit") == Some(&json!(true));
        Self {
            kind,
            token: match (kind, explicit) {
                (Kind::State, true) => KindToken::ExplicitState,
                (Kind::State, false) => KindToken::State,
                (Kind::Stream, _) => KindToken::Stream,
                (Kind::Event, _) => KindToken::Events,
                (Kind::Operation, _) => KindToken::Op,
            },
            template,
            is_report: v2::is_report(&type_of(&r["type"])),
            annotations,
        }
    }

    fn view(&self) -> ReportResource<'_> {
        ReportResource {
            kind: self.kind,
            token: self.token,
            template: &self.template,
            is_report: self.is_report,
            annotations: &self.annotations,
        }
    }
}

fn names(v: &Value) -> Vec<&str> {
    v.as_array()
        .expect("names")
        .iter()
        .map(|n| n.as_str().expect("a name"))
        .collect()
}

fn reasons_json(why: &[v2::ReportInvalid]) -> Value {
    json!(why.iter().map(|w| w.as_str()).collect::<Vec<_>>())
}

/// `v2/declarations.json`: a desired resource's declaration with its
/// horizon, and a report resource's with its two tables (desired.v2 §2.2,
/// §2.10).
#[test]
fn desired_v2_declarations() {
    let doc = read_json(&dir().join("declarations.json"));
    let resources = doc["resources"].as_array().expect("resources");
    assert_eq!(resources.len(), 12);
    for case in resources {
        let (kind, template, params, ann) = desired_resource(&case["resource"]);
        let uses = names(&case["uses"]);
        let d = v2::declaration(kind, &template, &params, &ann, |p| uses.contains(&p));
        let what = format!("{}: {}", case["note"], case["resource"]);
        assert_eq!(case["expect"], desired_declaration_json(&d), "{what}");
    }
    let reports = doc["reports"].as_array().expect("reports");
    assert_eq!(reports.len(), 34);
    for case in reports {
        let requires = names(&case["requires"]);
        let res = Res::of(&case["resource"]);
        let d = v2::report_declaration(
            &res.view(),
            |n| requires.contains(&n),
            case["others"].as_array().expect("others"),
        );
        let got = match &d {
            ReportDeclaration::NotThisProfiles => json!({"declaration": d.as_str()}),
            ReportDeclaration::Invalid(why) => {
                json!({"declaration": d.as_str(), "reasons": reasons_json(why)})
            }
            ReportDeclaration::Declared { names, tables } => {
                let mut got = json!({
                    "declaration": d.as_str(),
                    "pointers": tables.pointers,
                    "details": tables.details,
                });
                match names {
                    v2::Names::Role(r) => got["role"] = json!(r),
                    v2::Names::Param(p) => got["param"] = json!(p),
                }
                got
            }
        };
        assert_eq!(case["expect"], got, "{}", case["note"]);
    }
}

/// `v2/slices.json`: which reply is a role's report, found by
/// `desired.report` (desired.v2 §2.10).
#[test]
fn desired_v2_slices() {
    let doc = read_json(&dir().join("slices.json"));
    let cases = doc["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 16);
    for case in cases {
        let what = format!("{}", case["note"]);
        let r = &case["reading"];
        let target: Addr = r["target"]
            .as_str()
            .expect("target")
            .parse()
            .expect("an address");
        let iface: IfaceId = r["iface"]
            .as_str()
            .expect("iface")
            .parse()
            .expect("an interface id");
        let requires = names(&r["requires"]);
        let held: Vec<Res> = r["resources"]
            .as_array()
            .expect("resources")
            .iter()
            .map(Res::of)
            .collect();
        let views: Vec<ReportResource<'_>> = held.iter().map(Res::view).collect();
        let at = v2::report_at(
            &target,
            &iface,
            r["role"].as_str().expect("role"),
            &views,
            |n| requires.contains(&n),
        );
        let replies = r["replies"].as_array().expect("replies");
        let bytes = at.key().and_then(|k| {
            let on: Vec<&Value> = replies.iter().filter(|x| x["key"] == k.as_str()).collect();
            assert!(
                on.len() <= 1,
                "{what}: one reply a key after a tool's merge"
            );
            on.first()
                .map(|x| (tag(&x["tag"]), unhex(x["hex"].as_str().expect("hex"))))
        });
        let found = at.found(bytes.as_ref().map(|(t, b)| (*t, b.as_slice())));
        let mut got = json!({
            "found": found.as_str(),
            "key": at.key().map(|k| k.as_str()),
            "reasons": match &at {
                ReportAt::Invalid(why) => reasons_json(why),
                _ => json!([]),
            },
        });
        match &found {
            Found::None(why) => assert_eq!(reasons_json(why), got["reasons"], "{what}"),
            Found::Unreadable(u) => got["why"] = json!(u.as_str()),
            Found::One(read) => got["findings"] = findings_json(&read.findings),
        }
        assert_eq!(case["expect"], got, "{what}");
    }
}

/// `v2/conclusions.json`: one reading of a target's role and its author's
/// document → §5's four questions, the versions compared by run and seq
/// (desired.v2 §2.17, §5). Three of the four judges are `desired.v1`'s,
/// over this major's report.
#[test]
fn desired_v2_conclusions() {
    let doc = read_json(&dir().join("conclusions.json"));
    let cases = doc["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 39);
    for case in cases {
        let t = &case["reading"]["target"];
        let a = &case["reading"]["author"];
        let report = match &t["report"] {
            Value::Null => None,
            r => Some(v2::read(
                tag(&r["tag"]),
                &unhex(r["hex"].as_str().expect("hex")),
            )),
        };
        let listing = |d: &Value| match d {
            Value::Null => None,
            d if d["lists"] == json!(false) => Some(desired::Listing::NotListed),
            d => Some(desired::Listing::Listed {
                bound: d["bound"] == json!(true),
            }),
        };
        let target = desired::TargetReading {
            presence: desired_seen(&t["presence"]),
            descriptor: listing(&t["descriptor"]),
            deployment: listing(&t["deployment"]),
            report: match &report {
                None => desired::ReportRead::NotRead,
                Some(Err(_)) => desired::ReportRead::Unreadable,
                Some(Ok(r)) => desired::ReportRead::Read(r),
            },
            freshness: match t["age"].as_str() {
                Some("fresh") => freshness::Verdict::Fresh,
                Some("stale") => freshness::Verdict::Stale,
                Some("unobservable") => freshness::Verdict::Unobservable,
                None => freshness::Verdict::NotAsked,
                Some(other) => panic!("age {other:?}"),
            },
        };
        let author = v2::AuthorReading {
            presence: desired_seen(&a["presence"]),
            current: match &a["get"] {
                Value::String(s) => match s.as_str() {
                    "not_made" => v2::Current::NotRead,
                    "timeout" => v2::Current::Incomplete,
                    "silent" => v2::Current::Silent,
                    "unstamped" => v2::Current::Unstamped,
                    other => panic!("get {other:?}"),
                },
                g => match (g.get("entry"), g.get("tombstone"), g.get("no_entry")) {
                    (Some(v), _, _) => v2::Current::Entry(version(v)),
                    (_, Some(v), _) => v2::Current::Tombstone(version(v)),
                    (_, _, Some(r)) => v2::Current::NoEntry(run(r)),
                    _ => panic!("get {g}"),
                },
            },
        };
        let got = json!({
            "converged": desired_judged(v2::converged(&target, &author)),
            "refused": desired_judged(desired::refused(&target)),
            "last_known": desired_judged(desired::last_known(&target)),
            "in_force": desired_judged(desired::in_force(&target)),
        });
        assert_eq!(case["expect"], got, "{}", case["note"]);
    }
}

/// The scenarios' contracts, as `scenarios-v2.md` gives them and the
/// runtime's tests keep them (`zenkey/tests/contracts/`).
fn scenario_contract(file: &str) -> zenkey_model::contract::Contract {
    let path = repo_root().join("zenkey/tests/contracts").join(file);
    let l = zenkey_model::contract::load_path(&path);
    assert!(l.report.0.is_empty(), "{file} has findings:\n{}", l.report);
    l.contract.expect("no error")
}

/// The scenarios' `tasking.v2` and `worker.v2` load with no finding at
/// all, W105 reading this major's vocabulary (core §10, desired.v2 §4),
/// and read as the text says: the author's desired resource declared and
/// answered by snapshot (§2.4, core 0.112), the target's report resource
/// declared with its role and its two tables, and found at the scenarios'
/// `Rp` (§2.10).
#[test]
fn desired_v2_scenario_contracts() {
    use zenkey_model::collection::per_key;
    let tasking = scenario_contract("tasking.v2.toml");
    let worker = scenario_contract("worker.v2.toml");
    for c in [&tasking, &worker] {
        assert!(v2::uses_v2(c));
        assert_eq!(v2::Major::of(c), Some(v2::Major::V2));
        assert_eq!(v2::lint_contract(c), []);
    }
    let tasks = &tasking.resources[0];
    assert!(matches!(
        v2::declaration_of(&tasking, tasks),
        desired::Declaration::Declared(ref t) if t.len() == 2
    ));
    assert!(!per_key(&tasking, tasks), "answered by snapshot");
    let v1 = scenario_contract("tasking.v1.toml");
    assert_eq!(v2::Major::of(&v1), Some(v2::Major::V1));
    assert!(
        per_key(&v1, &v1.resources[0]),
        "desired.v1's keeps its reply per key"
    );
    assert_eq!(v2::lint_contract(&v1), [], "no lint of another major's");

    let report = worker
        .resource(KindToken::State, "desired/task")
        .expect("the report resource");
    let ReportDeclaration::Declared { names, tables } = v2::report_declaration_of(&worker, report)
    else {
        panic!("the report resource is declared");
    };
    assert_eq!(names, v2::Names::Role("task".into()));
    assert_eq!(tables.place("/rate_hz"), Index::At(2));
    assert_eq!(tables.place("/endpoint/host"), Index::At(3));
    assert_eq!(tables.line("a revision above 4,294,967,295"), Index::At(2));
    let running = worker
        .resource(KindToken::State, "running")
        .expect("running");
    assert_eq!(
        v2::report_declaration_of(&worker, running),
        ReportDeclaration::NotThisProfiles
    );
    // Its layout, read from the contract's own artifact, is the model's.
    let zenkey_model::contract::Body::Data(data) = &report.body else {
        panic!("a state");
    };
    assert!(v2::is_report(&data.type_));
    assert_eq!(
        worker
            .packed_layout(&data.type_)
            .expect("a layout")
            .as_ref(),
        Some(v2::layout())
    );
    let target: Addr = "lab/worker".parse().expect("an address");
    let at = v2::report_at_of(&target, &worker, "task");
    assert_eq!(
        at.key().map(|k| k.as_str()),
        Some("zk2/lab/worker/worker.v2/state/desired/task")
    );
    assert_eq!(v2::report_at_of(&target, &worker, "plan"), ReportAt::None);
    // The baseline's report, the 3 bytes `000000`, found and read.
    let record = v2::Report::baseline().write().expect("a record");
    assert_eq!(hex(&record.bytes), "000000");
    assert!(matches!(
        at.found(Some((v2::TAG, &record.bytes))),
        Found::One(ref r) if r.report == Some(v2::Report::baseline())
    ));
}

/// A contract's own lints for this major (§2.2, §2.10, §4): a key of
/// `desired.v2`'s vocabulary under `desired.v1` is outside v1's (W105), a
/// key of neither is outside both, and an invalid declaration is a
/// finding of [`v2::lint_contract`], each reason by its token.
#[test]
fn desired_v2_lints() {
    let dir = repo_root().join("zenkey/tests/contracts");
    let load = |uses: &str, annotations: &str| {
        let text = format!(
            "[interface]\nname = \"lint\"\nmajor = 1\nminor = 0\nuses = [{uses}]\n\
             [schemas]\njsonschema = [\"tasking.json\"]\n\
             packed = [\"../../../spec/profiles/desired/desired.v2.json\"]\n\
             [resources.\"tasks/{{system}}\"]\nkind = \"state\"\ntype = \"json:Task\"\n\
             params = {{ system = \"string\" }}\ncardinality = 4\n\
             annotations = {{ {annotations} }}\n\
             [resources.report]\nkind = \"state\"\ntype = \"packed:Report\"\n\
             annotations = {{ \"desired.report\" = \"task\" }}\n\
             [requires.task]\ninterface = \"tasking.v2\"\noptional = true\n"
        );
        zenkey_model::contract::load_str(&text, &dir, None)
    };
    let codes = |l: &zenkey_model::contract::Loaded| -> Vec<String> {
        l.report.0.iter().map(|d| d.code.to_string()).collect()
    };
    let ok = load(
        "\"desired.v2\", \"freshness.v2\"",
        "\"desired.target_param\" = \"system\", \"freshness.ttl_s\" = 600",
    );
    assert_eq!(codes(&ok), [] as [&str; 0], "{}", ok.report);
    assert_eq!(v2::lint_contract(&ok.contract.expect("it loads")), []);
    // Under desired.v1, `desired.report` is outside the vocabulary.
    let v1 = load("\"desired.v1\"", "\"desired.target_param\" = \"system\"");
    assert_eq!(codes(&v1), ["W105"], "{}", v1.report);
    // `desired.target` is outside both majors' (§4).
    let retired = load(
        "\"desired.v2\"",
        "\"desired.target_param\" = \"system\", \"desired.target\" = \"x\"",
    );
    assert_eq!(codes(&retired), ["W105"], "{}", retired.report);
    // A horizon above 0 under freshness.v1, and a second resource naming
    // a role that is no requirement: each reason a finding, by the
    // fixture's tokens, in the order checked.
    let bad = load(
        "\"desired.v2\", \"freshness.v1\"",
        "\"desired.target_param\" = \"system\", \"freshness.ttl_s\" = 600, \"desired.report\" = \"nobody\"",
    );
    let found = v2::lint_contract(&bad.contract.expect("the core's lints pass"));
    let got: Vec<(&str, &str)> = found.iter().map(|f| (f.at.as_str(), f.token)).collect();
    assert_eq!(
        got,
        [
            ("state/tasks/{system}", "horizon_v1"),
            ("state/tasks/{system}", "not_a_role"),
            ("state/tasks/{system}", "param_not_role"),
            ("state/tasks/{system}", "not_report"),
        ]
    );
}
