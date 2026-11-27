//! `alarms.v2`'s fixtures (`spec/profiles/alarms/conformance/v2/`, #984's
//! S1), run against `zenkey_model::alarms` and `zenkey_model::alarms::v2`
//! as written. Each expectation was computed twice by two scripts of the
//! text's drafter and never blessed, and no runner here blesses one: a
//! failure is the model's, or a fixture or a sentence to raise with the
//! bytes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use zenkey_model::alarms::v2::{
    self, Act, Applied, Dating, Declared, Held, Instance, Left, Naming, Raise, Read, Reading,
    Record, Resolve, Rule, Sample, Seed, Seen, Since, Slots, SnapshotRead, SnapshotRun, Track,
    Words,
};
use zenkey_model::alarms::{
    self, Answer, Judged, Listing, Major, OwnerAlarms, Presence, Reason, ScopeRead, SetJudged,
    SetVerdict, Severity, State,
};
use zenkey_model::canonical::Fingerprint;
use zenkey_model::collection::{self, Confirmation, Page};
use zenkey_model::contract::{Body, Contract, load_path, load_str};
use zenkey_model::freshness::{self, Applied as Heartbeated, SectionRead, View};
use zenkey_model::grammar::KindToken;
use zenkey_model::heartbeat::{self, Heartbeat, Layout};
use zenkey_model::packed::{self, Decimal};
use zenkey_model::schema::TypeId;
use zenkey_model::timing::Mono;
use zenkey_model::wire::{Envelope, Run, Seq, Stamp};

use super::{jcs_id, profiles, read_json, repo_root, unhex};

fn dir() -> PathBuf {
    profiles().join("alarms/conformance/v2")
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn run(v: &Value) -> Run {
    Run::from_hex(v.as_str().expect("a run")).expect("4 or 8 hexadecimal digits")
}

/// A stamp as the fixtures write one: `<run>/<seq>`.
fn stamp(v: &Value) -> Stamp {
    let text = v.as_str().expect("a stamp");
    let (run, seq) = text.split_once('/').expect("<run>/<seq>");
    let s = Stamp {
        run: Run::from_hex(run).expect("4 or 8 hexadecimal digits"),
        seq: Seq::new(seq.parse().expect("a seq")).expect("a seq"),
    };
    assert_eq!(stamp_text(s), text, "a stamp writes back");
    s
}

fn stamp_text(s: Stamp) -> String {
    format!("{}/{}", s.run.hex(), s.seq.get())
}

fn epoch(case: &Value) -> i64 {
    case.get("epoch").and_then(Value::as_i64).unwrap_or(0)
}

fn u8_of(v: &Value) -> u8 {
    u8::try_from(v.as_u64().expect("a number")).expect("below 256")
}

/// An alert key as the fixtures write one: 16 hexadecimal digits, or null.
fn alert_key(v: &Value) -> Option<u64> {
    match v {
        Value::Null => None,
        v => Some(alarms::key_integer(v.as_str().expect("an alert key")).expect("16 hex digits")),
    }
}

fn pairs(v: &Value) -> Vec<(&str, &str)> {
    v.as_object()
        .expect("labels")
        .iter()
        .map(|(n, v)| (n.as_str(), v.as_str().expect("a label's text")))
        .collect()
}

/// The standard contract, `alarms.v2.toml`, loaded with no finding.
fn standard() -> Contract {
    let l = load_path(&profiles().join("alarms/alarms.v2.toml"));
    assert!(l.report.0.is_empty(), "alarms.v2.toml:\n{}", l.report);
    l.contract.expect("no finding, so a contract")
}

/// A record's fields as the fixtures give them where the state, the
/// severity and the time play no part: `rule`, `labels`, `argument`, `key`.
fn record_of_fields(v: &Value) -> Record {
    let mut whole = v.clone();
    let m = whole.as_object_mut().expect("a record's fields");
    m.entry("state").or_insert(json!("firing"));
    m.entry("severity").or_insert(json!("warning"));
    m.entry("argument").or_insert(json!(v2::NOT_AVAILABLE));
    Record::of_json(&whole).unwrap_or_else(|| panic!("a record's fields: {v}"))
}

/// The lists `tables.json` names.
fn named_tables() -> BTreeMap<String, Vec<Rule>> {
    let doc = read_json(&dir().join("tables.json"));
    doc["tables"]
        .as_object()
        .expect("tables")
        .iter()
        .map(|(name, list)| {
            let table = v2::rules(list).unwrap_or_else(|| panic!("table {name}"));
            assert_eq!(&v2::table_json(&table), list, "{name} writes back");
            (name.clone(), table)
        })
        .collect()
}

/// An instance's table as a case names it: a name of `tables`, null for
/// none, or "differ", which is none too.
fn table_named<'t>(tables: &'t BTreeMap<String, Vec<Rule>>, v: &Value) -> Option<&'t [Rule]> {
    match v.as_str() {
        None => {
            assert!(v.is_null(), "a table's name, null or differ: {v}");
            None
        }
        Some("differ") => None,
        Some(name) => Some(tables[name].as_slice()),
    }
}

/// `v2/schema.json`: the two standard schemas' published ids, and the
/// packed type's tag (alarms.v2 §3). The model's own declaration, which no
/// crate may read from `spec/`, has the file's layout.
#[test]
fn alarms_v2_schema() {
    let doc = read_json(&dir().join("schema.json"));
    let files = doc["files"].as_array().expect("files");
    assert_eq!(files.len(), 2, "the packed file and the JSON Schema");
    for f in files {
        let file = read_json(
            &profiles()
                .join("alarms")
                .join(f["file"].as_str().expect("file")),
        );
        assert_eq!(
            json!(jcs_id(&file)),
            f["current"],
            "{} is current",
            f["file"]
        );
    }
    let published: Vec<&str> = doc["published"]
        .as_array()
        .expect("published")
        .iter()
        .map(|p| p["id"].as_str().expect("id"))
        .collect();
    assert_eq!(published, v2::PUBLISHED, "alarms::v2::PUBLISHED");
    assert_eq!(
        published,
        files
            .iter()
            .map(|f| f["current"].as_str().expect("current"))
            .collect::<Vec<_>>(),
        "each file's current revision is the one published"
    );
    let packed_file = read_json(&profiles().join(format!("alarms/{}.json", v2::SCHEMA_STEM)));
    let types = doc["types"].as_array().expect("types");
    assert_eq!(
        types.iter().map(|t| &t["name"]).collect::<Vec<_>>(),
        [v2::TYPE, "alarms.v2.Words"]
    );
    let of_file = packed::layout_in(&packed_file, v2::TYPE).expect("the file's type");
    assert_eq!(types[0]["tag"], json!(of_file.tag()), "the Alarm's tag");
    assert_eq!(v2::TAG, of_file.tag(), "the Alarm's tag in the model");
    assert_eq!(
        v2::layout(),
        &of_file,
        "the model's Alarm is the standard schema's, names included"
    );
    let root = repo_root();
    let cases = doc["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 6, "schema.json's cases");
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

/// alarms.v2 §3, the standard contract as the model's constants have it:
/// revision 2.0's fingerprint and its heartbeat's tag, its one collection
/// with a horizon, the two collections' indices in a snapshot, its two
/// resources' types, its QoS and no `link.exposure` (§2.11). §4: W105 reads
/// the profile's table for a contract that lists `alarms.v2`, and
/// `alarms.v1`'s stays empty; a table's finding is a linter's error.
#[test]
fn alarms_v2_standard_contract() {
    let c = standard();
    assert_eq!(c.iface.to_string(), v2::IFACE);
    assert_eq!(
        Fingerprint::of(&c).to_string(),
        format!("sha256:{}", v2::FINGERPRINT),
        "revision 2.0, as v2.md §3 states it"
    );
    assert_eq!(
        c.uses.iter().map(ToString::to_string).collect::<Vec<_>>(),
        ["freshness.v2"]
    );
    assert!(!v2::uses_v2(&c), "the standard contract writes no table");
    assert_eq!(v2::declaration_of(&c), Declared::None);
    let layout = Layout::of(&c);
    assert_eq!(layout.tag.get(), 7, "its heartbeat's tag");
    assert_eq!(
        layout
            .collections
            .iter()
            .map(|x| (x.resource(), x.ttl, x.class, x.cardinality, x.live_bits()))
            .collect::<Vec<_>>(),
        [(
            format!("state/{}", v2::ALARMS),
            alarms::TTL,
            v2::CLASS,
            Some(u64::from(v2::MAX_SLOTS)),
            Some(9)
        )],
        "one collection: the alarms, at 900 s, of the host class, a live count of 9 bits"
    );
    let records = c
        .resource(KindToken::State, v2::ALARMS)
        .expect("state/alarms/{slot}");
    let words = c
        .resource(KindToken::State, v2::WORDS)
        .expect("state/words/{slot}");
    assert_eq!(c.resources.len(), 2, "two resources");
    assert_eq!(collection::coll_of(&c, records), Some(v2::ALARMS_COLL));
    assert_eq!(collection::coll_of(&c, words), Some(v2::WORDS_COLL));
    for r in [records, words] {
        assert_eq!(r.cardinality, Some(u64::from(v2::MAX_SLOTS)));
        assert!(!r.annotations.contains_key("link.exposure"), "§2.11");
    }
    let Body::Data(d) = &records.body else {
        panic!("a data resource")
    };
    let TypeId::Packed { name, schema } = &d.type_ else {
        panic!("a packed type, not {:?}", d.type_)
    };
    assert_eq!(name, v2::TYPE);
    assert!(v2::is_published(schema), "{schema}: the published schema");
    let l = c
        .packed_layout(&d.type_)
        .expect("reads")
        .expect("a packed type");
    assert_eq!(&l, v2::layout());
    assert!(d.express, "every put on alarms/{{slot}} is express");
    let Body::Data(w) = &words.body else {
        panic!("a data resource")
    };
    assert!(
        matches!(&w.type_, TypeId::JsonSchema { name, schema }
            if name == "alarms.v2.Words" && v2::is_published(schema)),
        "{:?}",
        w.type_
    );
    assert!(!w.express, "the words are not express");

    // W105 (§4, core §10), and the table's lint.
    let contract = |uses: &str, key: &str, value: &str| {
        let text = format!(
            "[interface]\nname = \"tc\"\nmajor = 1\nminor = 0\nuses = [\"{uses}\"]\n\n\
             [defaults]\nannotations = {{ \"alarms.{key}\" = {value} }}\n\n\
             [resources.rates]\nkind = \"stream\"\ntype = {{ raw = \"text/plain\" }}\n\n\
             [resources.\"links/{{link}}\"]\nkind = \"state\"\ncardinality = 4\n\
             params = {{ link = \"uint\" }}\ntype = {{ raw = \"text/plain\" }}\n"
        );
        let l = load_str(&text, &repo_root().join("spec"), None);
        let codes: Vec<String> = l.report.0.iter().map(|d| d.code.to_owned()).collect();
        (
            l.contract
                .unwrap_or_else(|| panic!("{uses}, {key}:\n{}", l.report)),
            codes,
        )
    };
    let table = "[ { name = \"hot\", summary = \"Temperature {argument} C\", \
                 argument = { minimum = \"-40\", resolution = \"0.1\" } }, \
                 { name = \"link_down\", summary = \"Link to {peer} is down\", \
                 labels = [ { name = \"peer\", values = [\"r1\", \"r2\"] } ] } ]";
    let (ok, codes) = contract("alarms.v2", "rules", table);
    assert_eq!(codes, Vec::<String>::new(), "rules is alarms.v2's key");
    assert_eq!(v2::lint_contract(&ok), [], "a table that reads");
    let held = v2::table_of(&ok).expect("the contract's table");
    assert_eq!(held.len(), 2);
    assert_eq!(held[1].labels[0].name, "peer");
    assert_eq!(
        contract("alarms.v2", "severity_default", "\"warning\"").1,
        ["W105"]
    );
    assert_eq!(
        contract("alarms.v1", "rules", table).1,
        ["W105"],
        "alarms.v1's table stays empty"
    );
    let (v1, _) = contract("alarms.v1", "rules", table);
    assert_eq!(v2::lint_contract(&v1), [], "W105's, not this lint's");
    assert_eq!(
        v2::declaration_of(&v1),
        Declared::Invalid(v2::Invalid::NotUsed),
        "and it holds no table"
    );
    let (bad, codes) = contract(
        "alarms.v2",
        "rules",
        "[ { name = \"door\", summary = \"Door {door} open\" } ]",
    );
    assert_eq!(codes, Vec::<String>::new(), "the core draws no finding");
    let findings = v2::lint_contract(&bad);
    assert_eq!(findings.len(), 1, "one finding for the table");
    assert_eq!(
        findings[0].to_string(),
        format!(
            "error[alarms.v2 placeholder] state/links/{{link}}: alarms.rules: {}",
            v2::Invalid::Placeholder.says()
        )
    );
    assert_eq!(
        v2::table_of(&bad),
        None,
        "an invalid declaration holds none"
    );
}

/// `v2/records.json`: the `Alarm`'s layout; a value → its record's bits,
/// pad and bytes and its put's payload, or the writer's refusal; a tag and
/// bytes → the value, or why it is undecodable (alarms.v2 §2.4, §3). Every
/// accepted write goes through the model's typed record too: read, written
/// back to the same bytes, and put.
#[test]
fn alarms_v2_records() {
    let doc = read_json(&dir().join("records.json"));
    let file = read_json(&profiles().join(format!("alarms/{}.json", v2::SCHEMA_STEM)));
    assert_eq!(doc["schema"], file, "the profile's packed document");

    let layouts = doc["layouts"].as_array().expect("layouts");
    assert_eq!(layouts.len(), 1, "layouts");
    for case in layouts {
        assert_eq!(case["type"], json!(v2::TYPE));
        let l = v2::layout();
        let got = json!({
            "form": l.form(),
            "sha256": l.sha256_hex(),
            "tag": l.tag(),
            "min_bits": l.min_bits(),
            "max_bits": l.max_bits(),
        });
        assert_eq!(case["expect"], got, "layout {}", case["type"]);
    }

    let writes = doc["writes"].as_array().expect("writes");
    assert_eq!(writes.len(), 35, "writes");
    let (mut written, mut refused) = (0, BTreeSet::new());
    for case in writes {
        let note = &case["note"];
        let e = epoch(case);
        let at = case.get("stamp").map(stamp);
        let got = match v2::write_value(&case["value"], e) {
            Err(r) => {
                assert!(at.is_none(), "{note}: a refused write is no put");
                refused.insert(r.token());
                // The typed record refuses what the value's writer does,
                // where the value is one a record holds.
                if let Some(typed) = Record::of_json(&case["value"])
                    && matches!(r, v2::Refused::Core(packed::WriteRefusal::Range))
                {
                    assert_eq!(typed.write(e), Err(r), "{note}: the typed record");
                }
                json!({"refused": r.token()})
            }
            Ok(r) => {
                written += 1;
                let typed = Record::read(v2::TAG, &r.bytes, e).unwrap_or_else(|x| {
                    panic!("{note}: an accepted write reads back, not {}", x.token())
                });
                let reads_as = case.get("reads_as").unwrap_or(&case["value"]);
                assert_eq!(&typed.to_json(), reads_as, "{note}: reads back");
                assert_eq!(
                    typed.write(e).expect("a record read writes back").bytes,
                    r.bytes,
                    "{note}: the typed record's bytes"
                );
                let mut got = json!({
                    "bits": r.bit_string(),
                    "pad": r.pad(),
                    "hex": hex(&r.bytes),
                });
                if let Some(s) = at {
                    got["payload"] = json!(hex(&typed.put(s, e).expect("a record read is put")));
                }
                got
            }
        };
        assert_eq!(case["expect"], got, "write: {note}");
    }
    assert_eq!(written, 19, "accepted writes");
    assert_eq!(
        refused.into_iter().collect::<Vec<_>>(),
        [
            "by_words_fields",
            "key_missing",
            "key_with_rule",
            "missing",
            "range",
            "unknown",
            "value"
        ],
        "every writer's refusal pinned"
    );

    let reads = doc["reads"].as_array().expect("reads");
    assert_eq!(reads.len(), 42, "reads");
    let mut undecodable = BTreeSet::new();
    for case in reads {
        let note = &case["note"];
        let bytes = unhex(case["hex"].as_str().expect("hex"));
        let got = match Record::read(u8_of(&case["tag"]), &bytes, epoch(case)) {
            Ok(r) => json!({"value": r.to_json()}),
            Err(u) => {
                undecodable.insert(u.token());
                json!({"undecodable": u.token()})
            }
        };
        assert_eq!(case["expect"], got, "read: {note}");
    }
    assert_eq!(
        undecodable.into_iter().collect::<Vec<_>>(),
        ["key_missing", "key_with_rule", "length", "pad", "tag"],
        "every reason a record is undecodable pinned"
    );
}

/// `v2/tables.json`: a contract's table from its resolved annotations,
/// with each finding; an instance's from the revisions its descriptor
/// names; an alarm written by its table or by words; a record read back
/// into words, or unnamed; and a table's growth (alarms.v2 §2.4, §4).
#[test]
fn alarms_v2_tables() {
    let doc = read_json(&dir().join("tables.json"));
    let tables = named_tables();

    let declarations = doc["declarations"].as_array().expect("declarations");
    assert_eq!(declarations.len(), 49, "declarations");
    let mut findings = BTreeSet::new();
    for case in declarations {
        let note = &case["note"];
        let uses = case["uses"]
            .as_array()
            .expect("uses")
            .iter()
            .any(|u| u == v2::PROFILE);
        let carried = case["resources"]
            .as_object()
            .expect("resources")
            .values()
            .filter_map(|a| a.get(v2::RULES));
        let got = match v2::declaration(uses, carried) {
            Declared::None => Value::Null,
            Declared::Table(t) => v2::table_json(&t),
            Declared::Invalid(why) => {
                findings.insert(why.as_str());
                json!({"invalid": why.as_str()})
            }
        };
        assert_eq!(case["expect"], got, "declaration: {note}");
    }
    assert_eq!(
        findings.into_iter().collect::<Vec<_>>(),
        [
            "disagrees",
            "host_label",
            "label",
            "labels_width",
            "not_used",
            "placeholder",
            "rule_name",
            "value"
        ],
        "each of §4's eight findings pinned"
    );

    let instances = doc["instances"].as_array().expect("instances");
    assert_eq!(instances.len(), 8, "instances");
    for case in instances {
        let note = &case["note"];
        let holds = case["holds"].as_object().expect("holds");
        let held: BTreeMap<&str, Option<Vec<Rule>>> = holds
            .iter()
            .map(|(rev, t)| {
                let table = match t {
                    Value::Null => None,
                    t => Some(v2::rules(t).expect("a table held")),
                };
                (rev.as_str(), table)
            })
            .collect();
        // Only the revisions the descriptor names, and of those the ones
        // the reader holds.
        let named = case["names"].as_array().expect("names").iter().map(|n| {
            held.get(n.as_str().expect("a revision"))
                .and_then(|t| t.as_deref())
        });
        let got = match v2::instance_table(named) {
            Instance::None => Value::Null,
            Instance::Table(t) => v2::table_json(&t),
            Instance::Differ => json!("differ"),
        };
        assert_eq!(case["expect"], got, "instance: {note}");
    }

    let writes = doc["writes"].as_array().expect("writes");
    assert_eq!(writes.len(), 32, "writes");
    for case in writes {
        let note = &case["note"];
        let reading = case["argument"]
            .as_str()
            .map(|s| Decimal::parse_input(s).expect("a decimal"));
        let fields = v2::write_alarm(
            table_named(&tables, &case["table"]),
            case["rule"].as_str().expect("rule"),
            pairs(&case["labels"]),
            reading.as_ref(),
        );
        assert_eq!(case["expect"], fields.to_json(), "write: {note}");
        // What it writes is a record an owner may write.
        Record::new(State::Firing, Severity::Warning, fields, None)
            .write(0)
            .unwrap_or_else(|r| panic!("{note}: refused {}", r.token()));
    }

    let reads = doc["reads"].as_array().expect("reads");
    assert_eq!(reads.len(), 27, "reads");
    let mut unnamed = BTreeSet::new();
    for case in reads {
        let note = &case["note"];
        let record = record_of_fields(&case["record"]);
        let got = match record.name(table_named(&tables, &case["table"])) {
            Naming::Named(n) => json!({"named": {
                "rule": n.rule.name,
                "labels": n.labels.iter().cloned().collect::<BTreeMap<_, _>>(),
                "alert_key": alarms::key_hex(n.alert_key),
                "summary": n.summary,
                "category": n.rule.category,
                "argument": n.argument.as_ref().map(ToString::to_string),
            }}),
            Naming::Unnamed(why) => {
                unnamed.insert(why.as_str());
                json!({"unnamed": why.as_str()})
            }
            Naming::ByWords { alert_key } => json!({"by_words": alarms::key_hex(alert_key)}),
        };
        assert_eq!(case["expect"], got, "read: {note}");
    }
    assert_eq!(
        unnamed.into_iter().collect::<Vec<_>>(),
        ["labels", "no_table", "rule"],
        "each reason an alarm is unnamed pinned"
    );

    let growth = doc["growth"].as_array().expect("growth");
    assert_eq!(growth.len(), 8, "growth");
    for case in growth {
        let of = |v: &Value| &tables[v.as_str().expect("a table's name")];
        let got = v2::growth(of(&case["old"]), of(&case["new"]));
        assert_eq!(
            case["expect"],
            json!(got.as_str()),
            "growth: {}",
            case["note"]
        );
    }
}

/// `v2/keys.json`: the alert key of an alarm's words, as digits and as the
/// integer a record carries; the one a table's rule derives; a key read as
/// a slot; and the collection a reference is read from (alarms.v2 §2.2,
/// §2.12, §4).
#[test]
fn alarms_v2_keys() {
    let doc = read_json(&dir().join("keys.json"));
    let tables = named_tables();
    let cases = doc["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 40, "keys.json's cases");
    let mut ops = BTreeMap::new();
    let slot_json = |k: Option<v2::SlotKey>| {
        k.map_or(
            Value::Null,
            |k| json!({"system": k.system, "service": k.service, "slot": k.slot}),
        )
    };
    for case in cases {
        let note = &case["note"];
        let op = case["op"].as_str().expect("op");
        *ops.entry(op).or_insert(0) += 1;
        let got = match op {
            "derive" => {
                let d =
                    alarms::derive(case["rule"].as_str().expect("rule"), pairs(&case["labels"]));
                assert_eq!(alarms::key_integer(&d.key), Some(d.integer()), "{note}");
                assert_eq!(alarms::key_hex(d.integer()), d.key, "{note}");
                json!({"key": d.key, "integer": d.integer(), "input": d.input})
            }
            "table" => record_of_fields(&case["record"])
                .name(Some(&tables["T1"]))
                .alert_key()
                .map_or(Value::Null, |k| json!(alarms::key_hex(k))),
            "slot" => {
                let key = case["key"].as_str().expect("key");
                let got = v2::slot_of(key);
                if let Some(k) = &got {
                    assert_eq!(
                        v2::alarm_key(&k.system, &k.service, k.slot)
                            .expect("a key")
                            .as_str(),
                        key,
                        "{note}: the key builds back"
                    );
                }
                slot_json(got)
            }
            "words" => {
                let key = case["key"].as_str().expect("key");
                let got = v2::words_slot_of(key);
                if let Some(k) = &got {
                    assert_eq!(
                        v2::words_key(&k.system, &k.service, k.slot)
                            .expect("a key")
                            .as_str(),
                        key,
                        "{note}: the key builds back"
                    );
                }
                slot_json(got)
            }
            "collection" => v2::collection_read(
                case["system"].as_str().expect("system"),
                case["service"].as_str().expect("service"),
            )
            .map_or(
                Value::Null,
                |c| json!({"selector": c.selector, "snapshot_key": c.snapshot_key}),
            ),
            other => panic!("op {other:?}"),
        };
        assert_eq!(case["expect"], got, "{op}: {note}");
    }
    assert_eq!(
        ops.into_iter().collect::<Vec<_>>(),
        [
            ("collection", 5),
            ("derive", 9),
            ("slot", 15),
            ("table", 8),
            ("words", 3)
        ]
    );
}

fn state_of(v: &Value) -> State {
    match v.as_str().expect("state") {
        "firing" => State::Firing,
        "resolved" => State::Resolved,
        other => panic!("state {other:?}"),
    }
}

/// `v2/slots.json`: the slot each identity takes, event by event; the
/// bound's refusal; a restart's adoption; and which of two slots a reader
/// takes for one identity (alarms.v2 §2.2, §2.6, §2.8, §2.9).
#[test]
fn alarms_v2_slots() {
    let doc = read_json(&dir().join("slots.json"));
    let cases = doc["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 15, "slots.json's cases");
    let (mut events, mut seen) = (0, BTreeSet::new());
    for case in cases {
        let note = &case["note"];
        let bound = u16::try_from(case["bound"].as_u64().expect("bound")).expect("a bound");
        let mut slots: Slots<String> = Slots::new(bound);
        for (i, event) in case["events"]
            .as_array()
            .expect("events")
            .iter()
            .enumerate()
        {
            events += 1;
            let id = |v: &Value| v.as_str().expect("an identity").to_owned();
            let got = if let Some(v) = event.get("raise") {
                match slots.raise(&id(v)) {
                    Raise::Put(slot) => json!({"put": slot, "state": "firing"}),
                    Raise::Firing => json!({"none": "firing"}),
                    Raise::Refused => json!({"refused": "bound"}),
                }
            } else if let Some(v) = event.get("resolve") {
                match slots.resolve(&id(v)) {
                    Resolve::Put(slot) => json!({"put": slot, "state": "resolved"}),
                    Resolve::NotFiring => json!({"none": "not_firing"}),
                }
            } else {
                let restart = &event["restart"];
                let left: Vec<(u8, Left<String>)> = match &restart["record"] {
                    Value::Null => Vec::new(),
                    record => record
                        .as_object()
                        .expect("record")
                        .iter()
                        .map(|(slot, e)| {
                            let flag = |m: &str| e[m].as_bool().expect("a flag");
                            (
                                slot.parse().expect("a slot"),
                                Left {
                                    id: id(&e["id"]),
                                    state: state_of(&e["state"]),
                                    reads: flag("reads"),
                                    derives: flag("derives"),
                                    evaluated: flag("evaluated"),
                                },
                            )
                        })
                        .collect(),
                };
                let bound =
                    u16::try_from(restart["bound"].as_u64().expect("bound")).expect("a bound");
                let (new, did) = Slots::restart(left, bound);
                slots = new;
                did.values().for_each(|a| {
                    seen.insert(a.as_str());
                });
                Value::Object(
                    did.iter()
                        .map(|(slot, a)| (slot.to_string(), json!(a.as_str())))
                        .collect(),
                )
            };
            for token in ["none", "refused"] {
                if let Some(t) = got.get(token) {
                    seen.insert(match t.as_str() {
                        Some("firing") => "none_firing",
                        Some("not_firing") => "none_not_firing",
                        _ => "refused",
                    });
                }
            }
            assert_eq!(event["expect"], got, "{note}: event {i}");
            if let Some(holds) = event.get("holds") {
                let held = Value::Object(
                    slots
                        .held()
                        .map(|(slot, id, state)| {
                            (slot.to_string(), json!({"id": id, "state": state.as_str()}))
                        })
                        .collect(),
                );
                assert_eq!(holds, &held, "{note}: held after event {i}");
            }
        }
    }
    assert!(events > 259, "the case of 259 events among them: {events}");
    assert_eq!(
        seen.into_iter().collect::<Vec<_>>(),
        [
            "adopted",
            "free",
            "left",
            "none_firing",
            "none_not_firing",
            "refused",
            "resolved"
        ],
        "each outcome pinned"
    );

    let views = doc["views"].as_array().expect("views");
    assert_eq!(views.len(), 8, "views");
    for case in views {
        let note = &case["note"];
        let held: Vec<Seen> = case["held"]
            .as_array()
            .expect("held")
            .iter()
            .map(|h| Seen {
                slot: u8_of(&h["slot"]),
                stamp: Stamp {
                    run: run(&h["run"]),
                    seq: Seq::new(u32::try_from(h["seq"].as_u64().expect("seq")).expect("a seq"))
                        .expect("a seq"),
                },
                alert_key: alert_key(&h["alert_key"]),
            })
            .collect();
        let taken = v2::taken(&held, run(&case["present_run"]));
        let got = json!({
            "takes": taken
                .takes
                .iter()
                .map(|(k, slot)| (alarms::key_hex(*k), json!(slot)))
                .collect::<serde_json::Map<_, _>>(),
            "double": taken.double.iter().map(|k| alarms::key_hex(*k)).collect::<Vec<_>>(),
        });
        assert_eq!(case["expect"], got, "view: {note}");
        // §5's fourth question, over an owner whose alarms are established.
        assert_eq!(
            v2::one_slot_each(SetVerdict::Established, &taken),
            if taken.double.is_empty() {
                Answer::Yes
            } else {
                Answer::No
            },
            "{note}"
        );
        assert_eq!(
            v2::one_slot_each(SetVerdict::Unobservable, &taken),
            Answer::Unobservable
        );
    }
}

/// A slot's record as `words.json` gives it: null, "undecodable", or the
/// fields that matter there.
fn slot_record(v: &Value) -> Option<Result<Record, ()>> {
    match v {
        Value::Null => None,
        Value::String(s) => {
            assert_eq!(s, "undecodable");
            Some(Err(()))
        }
        fields => Some(Ok(record_of_fields(fields))),
    }
}

/// A slot's words as `words.json` gives them: a document, decoded, or
/// `{text}`, a payload's text.
fn slot_words(v: &Value) -> Option<Result<Words, v2::WordsInvalid>> {
    match v {
        Value::Null => None,
        Value::Object(m) if m.len() == 1 && m.contains_key("text") => {
            Some(Words::decode(m["text"].as_str().expect("text").as_bytes()))
        }
        document => {
            let direct = Words::from_json(document);
            // A document as its payload: the same reading.
            let bytes = serde_json::to_vec(document).expect("a document");
            assert_eq!(Words::decode(&bytes), direct, "{document}");
            Some(direct)
        }
    }
}

/// `v2/words.json`: whether a reader takes a slot's words as its alarm's,
/// and why not; and what a tool shows as words and records arrive
/// (alarms.v2 §2.4, §5).
#[test]
fn alarms_v2_words() {
    let doc = read_json(&dir().join("words.json"));
    let takes = doc["takes"].as_array().expect("takes");
    assert_eq!(takes.len(), 24, "takes");
    let mut seen = BTreeSet::new();
    for case in takes {
        let note = &case["note"];
        let record = slot_record(&case["record"]);
        let words = slot_words(&case["words"]);
        let taken = v2::words_taken(record.as_ref(), words.as_ref());
        let got = match taken {
            Ok(how) => json!({"taken": true, "how": how.as_str()}),
            Err(why) => json!({"taken": false, "why": why.as_str()}),
        };
        assert_eq!(case["expect"], got, "takes: {note}");
        // §5's third question over the same slot.
        let j = v2::words_derive(taken);
        seen.insert((j.answer.as_str(), j.reason.as_str()));
        if let Some(Ok(w)) = &words {
            assert_eq!(Words::from_json(&w.to_json()).as_ref(), Ok(w), "{note}");
        }
    }
    assert_eq!(
        seen.into_iter().collect::<Vec<_>>(),
        [
            ("no", "mismatch"),
            ("not_asked", "no_record"),
            ("not_asked", "no_words"),
            ("not_asked", "table"),
            ("unobservable", "undecodable"),
            ("unobservable", "unreadable"),
            ("yes", "sanitized"),
            ("yes", "strict"),
        ],
        "§5's third row, each answer and reason pinned"
    );

    let shows = doc["shows"].as_array().expect("shows");
    assert_eq!(shows.len(), 6, "shows");
    for case in shows {
        let note = &case["note"];
        // What the reader holds on the slot: its record, and the words
        // last put on `words/<slot>`.
        let mut record: Option<Result<Record, ()>> = None;
        let mut words: Option<(Value, Result<Words, v2::WordsInvalid>)> = None;
        for (i, step) in case["steps"].as_array().expect("steps").iter().enumerate() {
            if let Some(w) = step.get("words") {
                words = Some((w.clone(), Words::from_json(w)));
            } else {
                record = slot_record(&step["record"]);
            }
            let got = match &record {
                None => Value::Null,
                Some(r) => {
                    let taken = v2::words_taken(Some(r), words.as_ref().map(|(_, w)| w));
                    json!({
                        "alert_key": r.as_ref().ok().and_then(|r| r.key).map(alarms::key_hex),
                        "words": match (taken, &words) {
                            (Ok(_), Some((document, _))) => document.clone(),
                            _ => Value::Null,
                        },
                    })
                }
            };
            assert_eq!(step["expect"], got, "{note}: step {i}");
        }
    }
}

/// `v2/transitions.json`: one slot's samples read by identity, `rebound`,
/// `replaced` and `dropped` among them, and what a notifier reports
/// (alarms.v2 §2.6, §2.16).
#[test]
fn alarms_v2_transitions() {
    let doc = read_json(&dir().join("transitions.json"));
    let cases = doc["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 35, "transitions.json's cases");
    let (mut reads, mut displaced) = (BTreeSet::new(), BTreeSet::new());
    for case in cases {
        let note = &case["note"];
        let mut track = Track::default();
        for (i, s) in case["samples"]
            .as_array()
            .expect("samples")
            .iter()
            .enumerate()
        {
            let sample = match &s["sample"] {
                Value::String(d) => {
                    assert_eq!(d, "delete");
                    Sample::Delete
                }
                put => {
                    let put = &put["put"];
                    match &put["record"] {
                        Value::String(u) => {
                            assert_eq!(u, "undecodable");
                            Sample::Unreadable
                        }
                        record => Sample::Put(Held {
                            record: Record::of_json(record)
                                .unwrap_or_else(|| panic!("{note}: a record, {record}")),
                            run: run(&put["run"]),
                            alert_key: alert_key(&put["alert_key"]),
                        }),
                    }
                }
            };
            let Applied {
                displaced: was,
                reads: read,
                reports,
            } = track.apply(sample);
            reads.insert(read.as_str());
            if let Some((d, _)) = &was {
                displaced.insert(d.as_str());
            }
            let mut got = json!({
                "held": was.as_ref().map(|(d, _)| d.as_str()),
                "reads": read.as_str(),
            });
            // A case that gives no `reports` is of an alarm a notifier
            // reports with no alert key, or of a severity not known.
            if s["expect"].get("reports").is_some() {
                got["reports"] = reports
                    .iter()
                    .map(|r| {
                        json!({
                            "report": r.change.as_str(),
                            "alert_key": r.alert_key.map(alarms::key_hex),
                        })
                    })
                    .collect();
            }
            assert_eq!(s["expect"], got, "{note}: sample {i}");
        }
    }
    assert_eq!(
        reads.into_iter().collect::<Vec<_>>(),
        [
            "confirmed",
            "escalated",
            "raised",
            "refreshed",
            "resolved",
            "retired",
            "unreadable"
        ],
        "each reading pinned"
    );
    assert_eq!(
        displaced.into_iter().collect::<Vec<_>>(),
        ["dropped", "rebound", "replaced"]
    );
}

fn judged_json(j: Judged) -> Value {
    json!({"answer": j.answer.as_str(), "reason": j.reason.as_str()})
}

fn listed() -> Presence {
    Presence::Present(Listing::Listed { token: true })
}

/// `v2/confirmations.json`: a heartbeat's bytes read over a reader's view,
/// through `freshness.v2`'s [`View`] of the standard contract: the live
/// count beside the firing count, the slots confirmed by rank, and each
/// slot's answer past the horizon; and a snapshot's page read entry by
/// entry (alarms.v2 §2.7, §3).
#[test]
fn alarms_v2_confirmations() {
    let doc = read_json(&dir().join("confirmations.json"));
    let layout = Layout::of(&standard());
    let resource = format!("state/{}", v2::ALARMS);
    let key = |slot: u8| v2::alarm_key("lab", "svc", slot).expect("a key");
    let at = |secs: u64| Mono::after(Duration::from_secs(secs));
    let past = 905;

    let heartbeats = doc["heartbeats"].as_array().expect("heartbeats");
    assert_eq!(heartbeats.len(), 16, "heartbeats");
    let mut seen = BTreeSet::new();
    for case in heartbeats {
        let note = &case["note"];
        assert_eq!(
            case["holds_tag"],
            json!(layout.tag.get()),
            "{note}: the reader holds revision 2.0"
        );
        let mut view: View<Mono> = View::new(layout.clone());
        assert!(view.follow_resource(&resource));
        let held: Vec<(u8, State)> = case["view"]
            .as_array()
            .expect("view")
            .iter()
            .map(|m| {
                let slot = u8_of(&m["slot"]);
                view.put(key(slot).as_str(), Some(stamp(&m["stamp"])), at(0));
                (slot, state_of(&m["state"]))
            })
            .collect();
        let built = &case["built_from"];

        let payload = unhex(case["payload"].as_str().expect("payload"));
        let env = Envelope::parse(&payload).unwrap_or_else(|x| panic!("{note}: envelope {x}"));
        assert_eq!(
            stamp_text(env.stamp),
            built["stamp"].as_str().expect("stamp")
        );
        let (tag, body) = heartbeat::body_of(&env).expect("core §7.5's form 6");
        assert_eq!(json!(tag.get()), built["tag"], "{note}");
        let mut got = match view.apply(tag, env.stamp, body, at(past)) {
            Heartbeated::Revision => json!({"unread": "revision"}),
            Heartbeated::Refused(_) => json!({"unread": "refused"}),
            Heartbeated::Older => json!({"unread": "older"}),
            Heartbeated::Read { sections, .. } => {
                let parsed = Heartbeat::parse(body, &layout).expect("a read heartbeat");
                let live = parsed.sections[0].as_ref().and_then(|s| s.members.live());
                let confirmed = match &sections[0] {
                    SectionRead::Confirmed(keys) => {
                        let mut slots: Vec<u8> = keys
                            .iter()
                            .map(|k| {
                                // The view's keys are base-relative, as a
                                // session reads them.
                                v2::slot_of(k)
                                    .unwrap_or_else(|| panic!("{note}: a slot's key, {k}"))
                                    .slot
                            })
                            .collect();
                        slots.sort_unstable();
                        slots
                    }
                    _ => Vec::new(),
                };
                json!({"read": {
                    "live": live,
                    "section": sections[0].as_str(),
                    "confirmed": confirmed,
                }})
            }
        };
        seen.insert(
            got.get("read")
                .map_or(got["unread"].clone(), |r| r["section"].clone())
                .as_str()
                .expect("a token")
                .to_owned(),
        );
        // Neither a count nor a bit is a state: the record says it.
        got["firing"] = json!(held.iter().filter(|(_, s)| *s == State::Firing).count());
        got["judged"] = Value::Object(
            held.iter()
                .map(|(slot, state)| {
                    let observation = view.observation(
                        key(*slot).as_str(),
                        at(past),
                        Duration::from_secs(past),
                        true,
                    );
                    let r = Reading {
                        presence: listed(),
                        freshness: Some(freshness::judge(&alarms::horizon(), &observation)),
                        latest: Some(match state {
                            State::Firing => v2::Latest::Firing,
                            State::Resolved => v2::Latest::Resolved,
                        }),
                        lost: false,
                        previous_instance: false,
                        held_whole: false,
                        left_out: false,
                    };
                    (slot.to_string(), judged_json(v2::firing(&r)))
                })
                .collect(),
        );
        assert_eq!(case["expect"], got, "heartbeat: {note}");
    }
    assert_eq!(
        seen.into_iter().collect::<Vec<_>>(),
        ["absent", "confirmed", "diverged", "refused", "revision"],
        "each reading of a heartbeat pinned"
    );

    let snapshots = doc["snapshots"].as_array().expect("snapshots");
    assert_eq!(snapshots.len(), 7, "snapshots");
    let read = v2::collection_read("lab", "svc").expect("names");
    for case in snapshots {
        let note = &case["note"];
        let e = epoch(case);
        let bytes = unhex(case["hex"].as_str().expect("hex"));
        let page = Page::parse(&bytes).unwrap_or_else(|x| panic!("{note}: page {x:?}"));
        let seed = Seed::of([(read.snapshot_key.as_str(), bytes.as_slice())], e);
        assert!(!seed.mixed && seed.words.is_none(), "{note}");
        let s = seed.alarms.as_ref().expect("the alarms' snapshot");
        assert!(s.heard.complete, "{note}: one page, and no more");
        assert_eq!(s.heard.tag.get(), v2::TAG, "{note}: the page's tag");
        assert!(s.retired.is_empty(), "{note}: no rule has an owner delete");
        let ms = |t: collection::Time| t.unix_ms(e).map(|t| i64::try_from(t).expect("ms"));
        let got = json!({
            "size": bytes.len(),
            "run": s.heard.run.hex(),
            "asof": s.heard.asof.get(),
            "coll": page.coll,
            "time": ms(s.heard.time),
            "entries": s.live.iter().map(|entry| json!({
                "slot": entry.slot,
                "stamp": stamp_text(entry.stamp),
                "confirmation": match entry.confirmation {
                    Confirmation::None => "none",
                    Confirmation::AtTime => "time",
                    Confirmation::At(_) => "conf",
                },
                "confirmed_at": entry.confirmed_at.and_then(ms),
                "record": entry.record.as_ref().map_or_else(
                    |u| json!({"undecodable": u.token()}),
                    Record::to_json,
                ),
            })).collect::<Vec<_>>(),
        });
        assert_eq!(case["expect"], got, "snapshot: {note}");
        // One complete snapshot establishes the owner's alarms, with an
        // entry or with none (§2.15).
        let stated = [s.heard.run];
        let set = v2::owner_set(listed(), Some(Read::Complete(seed.snapshot(&stated))), None);
        assert_eq!(
            (set.verdict, set.reason),
            (SetVerdict::Established, Reason::Snapshot),
            "{note}"
        );
    }
}

/// `v2/acks.json`: the `at` each act writes, with its one-second step and
/// its absence; whether an acknowledgement applies; and an `alarms.v1`
/// timestamp adopted (alarms.v2 §2.5).
#[test]
fn alarms_v2_acks() {
    let doc = read_json(&dir().join("acks.json"));
    let secs = |v: &Value| v.as_i64().map(|ms| ms.div_euclid(1000));
    let ms = |at: Option<i64>| at.map(|s| s * 1000);

    let writes = doc["writes"].as_array().expect("writes");
    assert_eq!(writes.len(), 9, "writes");
    for case in writes {
        let note = &case["note"];
        let e = epoch(case);
        // An adopted alarm's `at` is the one its record holds, and the
        // greatest its owner knows it wrote.
        let known = case.get("known").and_then(secs);
        let mut dating = Dating::known(known);
        let mut held = known;
        for (i, act) in case["acts"].as_array().expect("acts").iter().enumerate() {
            let what = match act["act"].as_str().expect("act") {
                "raise" => Act::Raise,
                "escalate" => Act::Escalate,
                "refresh" => Act::Refresh,
                "resolve" => Act::Resolve,
                other => panic!("act {other:?}"),
            };
            let clock = i128::from(act["clock_ms"].as_i64().expect("clock_ms"));
            held = dating.at(what, held, clock, e);
            assert_eq!(act["expect"], json!(ms(held)), "{note}: act {i}");
            // What it writes has a code.
            if let Some(at) = held {
                assert_eq!(v2::time_at(i128::from(at) * 1000, e), Some(at), "{note}");
            }
        }
    }

    let applies = doc["applies"].as_array().expect("applies");
    assert_eq!(applies.len(), 8, "applies");
    for case in applies {
        let acked = case["acked_at"].as_i64().map(i128::from);
        let got = v2::acknowledged(acked, secs(&case["at"]));
        assert_eq!(case["expect"], json!(got), "applies: {}", case["note"]);
    }

    let moves = doc["moves"].as_array().expect("moves");
    assert_eq!(moves.len(), 4, "moves");
    for case in moves {
        let timestamp = case["timestamp"].as_u64().expect("timestamp");
        let at = v2::adopted_at(timestamp, epoch(case));
        let got = json!({
            "at": ms(at),
            "acknowledgement_applies": v2::acknowledged(Some(i128::from(timestamp)), at),
        });
        assert_eq!(case["expect"], got, "moves: {}", case["note"]);
    }
}

/// A case's presence, descriptor and face, as `conclusions.json` writes
/// them, and the major the reading is judged under.
fn presence_and_major(case: &Value) -> (Presence, Option<Major>) {
    match case["presence"].as_str().expect("presence") {
        "present" => match &case["descriptor"] {
            Value::Null => (Presence::Present(Listing::Unread), None),
            d => {
                let major = major_of(d);
                let listing = match major {
                    Some(_) => Listing::Listed {
                        token: d["token"].as_bool().expect("descriptor.token"),
                    },
                    None => Listing::NotListed,
                };
                (Presence::Present(listing), major)
            }
        },
        "absent" => (Presence::Absent, None),
        "incomplete" => (Presence::Incomplete, None),
        // The deployment's word names alarms.v2.
        "across_face" => (
            Presence::AcrossFace {
                reachable: case["face"]["reachable"].as_bool().expect("face.reachable"),
            },
            Major::named(v2::IFACE),
        ),
        other => panic!("presence {other:?}"),
    }
}

fn major_of(descriptor: &Value) -> Option<Major> {
    Major::of(
        descriptor["interfaces"]
            .as_array()
            .expect("descriptor.interfaces")
            .iter()
            .map(|i| i.as_str().expect("an interface id")),
    )
}

fn major_json(m: Option<Major>) -> Value {
    m.map_or(Value::Null, |m| match m {
        Major::V1 => json!(1),
        Major::V2 => json!(2),
    })
}

/// `freshness.v2`'s verdict over a slot's observations, as a case states
/// it judged, by `freshness`'s own tokens (its `judgements.json`).
fn freshness_judged(v: &Value) -> Option<freshness::Judged> {
    use freshness::{Reason as R, Verdict as V};
    if v.is_null() {
        return None;
    }
    let token = |v: &Value| v.as_str().expect("a token").to_owned();
    let verdict = [V::Fresh, V::Stale, V::Unobservable, V::NotAsked]
        .into_iter()
        .find(|x| x.as_str() == token(&v["verdict"]))
        .unwrap_or_else(|| panic!("freshness verdict {}", v["verdict"]));
    let reason = [
        R::WithinHorizon,
        R::BeyondHorizon,
        R::NoDelivery,
        R::ListenedTooShort,
        R::Silent,
        R::NoStamp,
        R::ClockUntrusted,
        R::ClockDisagrees,
        R::NearHorizon,
        R::NoObservation,
        R::LastKnown,
    ]
    .into_iter()
    .find(|x| x.as_str() == token(&v["reason"]))
    .unwrap_or_else(|| panic!("freshness reason {}", v["reason"]));
    Some(freshness::Judged { verdict, reason })
}

/// One alarm as "is anything firing?" reads it: the first question's
/// answer, and its record's severity.
type Asked = (Judged, Option<Severity>);

fn set_verdict(v: &Value) -> SetVerdict {
    [
        SetVerdict::Established,
        SetVerdict::Unobservable,
        SetVerdict::NotAsked,
    ]
    .into_iter()
    .find(|x| x.as_str() == v.as_str().expect("a verdict"))
    .unwrap_or_else(|| panic!("verdict {v}"))
}

/// `v2/conclusions.json`: the major a descriptor is read by; "is this
/// alarm firing?" and "is it past its horizon?"; whether an owner's alarms
/// are established, by one snapshot; and "is anything firing?" (alarms.v2
/// §2.1, §2.11, §2.13 to §2.15, §5). A case whose descriptor lists
/// `alarms.v1` alone expects nothing more: `alarms.v1`'s own fixture is
/// the evidence.
#[test]
fn alarms_v2_conclusions() {
    let doc = read_json(&dir().join("conclusions.json"));

    let majors = doc["majors"].as_array().expect("majors");
    assert_eq!(majors.len(), 6, "majors");
    for case in majors {
        let got = major_json(major_of(&case["descriptor"]));
        assert_eq!(case["expect"], got, "major: {}", case["note"]);
    }

    let readings = doc["readings"].as_array().expect("readings");
    assert_eq!(readings.len(), 42, "readings");
    let (mut v1, mut reasons) = (0, BTreeSet::new());
    for case in readings {
        let note = &case["note"];
        let (presence, major) = presence_and_major(case);
        if case["expect"] == json!({"major": 1}) {
            assert_eq!(major, Some(Major::V1), "{note}");
            v1 += 1;
            continue;
        }
        let flag = |m: &str| case[m].as_bool().unwrap_or_else(|| panic!("{note}: {m}"));
        // An unnamed alarm is answered as any other, by its slot.
        let _named = flag("named");
        let r = Reading {
            presence,
            freshness: freshness_judged(&case["freshness"]),
            latest: match case["latest"].as_str() {
                None => None,
                Some("firing") => Some(v2::Latest::Firing),
                Some("resolved") => Some(v2::Latest::Resolved),
                Some("rebound") => Some(v2::Latest::Rebound),
                Some("deleted") => Some(v2::Latest::Deleted),
                Some("undecodable") => Some(v2::Latest::Undecodable),
                Some(other) => panic!("latest {other:?}"),
            },
            lost: flag("lost"),
            previous_instance: flag("previous_instance"),
            held_whole: flag("held_whole"),
            left_out: flag("left_out"),
        };
        let (firing, stale) = (v2::firing(&r), v2::stale_firing(&r));
        reasons.insert(firing.reason.as_str());
        let got = json!({
            "major": major_json(major),
            "firing": judged_json(firing),
            "stale_firing": judged_json(stale),
        });
        assert_eq!(case["expect"], got, "reading: {note}");
    }
    assert_eq!(v1, 1, "the reading that is alarms.v1's");
    assert!(reasons.contains("rebound") && reasons.contains("not_held"));

    let sets = doc["owner_sets"].as_array().expect("owner_sets");
    assert_eq!(sets.len(), 29, "owner_sets");
    let (mut v1, mut reasons) = (0, BTreeSet::new());
    for case in sets {
        let note = &case["note"];
        let (presence, major) = presence_and_major(case);
        if case["expect"] == json!({"major": 1}) {
            assert_eq!(major, Some(Major::V1), "{note}");
            v1 += 1;
            continue;
        }
        let read = match &case["read"] {
            Value::Null => None,
            r if r["complete"] == false => Some(Read::Incomplete),
            r => Some(Read::Complete(match &r["snapshot"] {
                Value::Null => None,
                s => Some(SnapshotRead {
                    complete: s["complete"].as_bool().expect("snapshot.complete"),
                    run: match s["run"].as_str().expect("snapshot.run") {
                        "stated" => SnapshotRun::Stated,
                        "other" => SnapshotRun::Other,
                        "unstated" => SnapshotRun::Unstated,
                        other => panic!("run {other:?}"),
                    },
                }),
            })),
        };
        let since = match &case["since"] {
            Value::Null => None,
            s => Some(Since {
                lossless: s["lossless"].as_bool().expect("since.lossless"),
                present_throughout: s["present_throughout"]
                    .as_bool()
                    .expect("since.present_throughout"),
            }),
        };
        let set = v2::owner_set(presence, read, since);
        reasons.insert(set.reason.as_str());
        let got = json!({
            "major": major_json(major),
            "verdict": set.verdict.as_str(),
            "reason": set.reason.as_str(),
        });
        assert_eq!(case["expect"], got, "owner_set: {note}");
    }
    assert_eq!(v1, 1, "the owner that is alarms.v1's");
    for reason in [
        "no_seed",
        "seed_incomplete",
        "window_lost",
        "owner_gap",
        "snapshot_incomplete",
        "snapshot_run",
        "snapshot",
    ] {
        assert!(reasons.contains(reason), "§2.15's {reason} pinned");
    }

    let any = doc["any_firing"].as_array().expect("any_firing");
    assert_eq!(any.len(), 18, "any_firing");
    for case in any {
        let note = &case["note"];
        let severity = |v: &Value| v.as_str().map(|s| Severity::parse(s).expect("a severity"));
        let owners: Vec<(SetJudged, Vec<Asked>)> = case["owners"]
            .as_array()
            .expect("owners")
            .iter()
            .map(|o| {
                let set = SetJudged {
                    verdict: set_verdict(&o["set"]["verdict"]),
                    // The reason plays no part in the second question.
                    reason: Reason::Snapshot,
                };
                let alarms = o["alarms"]
                    .as_array()
                    .expect("alarms")
                    .iter()
                    .map(|a| {
                        let answer = [
                            Answer::Yes,
                            Answer::No,
                            Answer::Unobservable,
                            Answer::NotAsked,
                        ]
                        .into_iter()
                        .find(|x| x.as_str() == a["answer"].as_str().expect("answer"))
                        .expect("an answer");
                        (
                            Judged {
                                answer,
                                reason: Reason::Firing,
                            },
                            severity(&a["severity"]),
                        )
                    })
                    .collect();
                (set, alarms)
            })
            .collect();
        let owners: Vec<OwnerAlarms<'_>> = owners
            .iter()
            .map(|(set, alarms)| OwnerAlarms { set: *set, alarms })
            .collect();
        let scope = if case["scope_whole"].as_bool().expect("scope_whole") {
            ScopeRead::Whole
        } else {
            ScopeRead::NotWhole
        };
        let a = alarms::any_firing(
            severity(&case["at_least"]).expect("at_least"),
            &owners,
            scope,
        );
        let got = json!({
            "answer": a.answer.as_str(),
            "reason": a.reason.as_str(),
            "firing": a.firing,
            "owners": {
                "established": a.established,
                "unobservable": a.unobservable,
                "not_asked": a.not_asked,
            },
        });
        assert_eq!(case["expect"], got, "any_firing: {note}");
    }
}
