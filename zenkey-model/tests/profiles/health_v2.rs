//! `health.v2`'s fixtures (`spec/profiles/health/conformance/v2/`, #986's
//! Q1), run against `zenkey_model::health` and `zenkey_model::health::v2`
//! as written. Each expectation was computed twice by two scripts of the
//! text's drafter and never blessed, and no runner here blesses one: a
//! failure is the model's, or a fixture or a sentence to raise with the
//! bytes.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use zenkey_model::canonical::Fingerprint;
use zenkey_model::collection::{self, Confirmation, Page};
use zenkey_model::contract::{Contract, load_path, load_str};
use zenkey_model::freshness::{self, Applied, View};
use zenkey_model::health::v2::{
    self, Argument, Check, ClockHeard, Declared, Fault, Instance, Kind, Named, Packed, Reading,
    RecordType, Status, StatusEntry, Tables, Word,
};
use zenkey_model::health::{self, Listing, Major, Presence, Read};
use zenkey_model::heartbeat::{self, Digest, Heartbeat, Layout, Time};
use zenkey_model::packed::{self, ReadRefusal};
use zenkey_model::schema::TypeId;
use zenkey_model::timing::Mono;
use zenkey_model::wire::{Envelope, Run, Seq, Stamp};

use super::{jcs_id, profiles, read_json, repo_root, unhex};

fn dir() -> PathBuf {
    profiles().join("health/conformance/v2")
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A stamp as the fixtures write one: `<run>/<seq>`, the run in 4
/// lowercase hexadecimal digits for a 16-bit run and 8 for a 32-bit one,
/// the seq in decimal.
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

/// A case's deployment epoch in seconds: 0 where none is given, as an
/// owner with no catalog counts from the Unix epoch.
fn epoch(case: &Value) -> i64 {
    case.get("epoch").and_then(Value::as_i64).unwrap_or(0)
}

/// The standard contract, `health.v2.toml`, loaded with no finding.
fn standard() -> Contract {
    let l = load_path(&profiles().join("health/health.v2.toml"));
    assert!(l.report.0.is_empty(), "health.v2.toml:\n{}", l.report);
    l.contract.expect("no finding, so a contract")
}

/// `v2/schema.json`: the standard packed schema's published id, and its
/// three types' tags (health.v2 §3). The model's own declaration, which no
/// crate may read from `spec/`, has the file's layouts.
#[test]
fn health_v2_schema() {
    let doc = read_json(&dir().join("schema.json"));
    let file = read_json(
        &profiles()
            .join("health")
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
    assert_eq!(published, v2::PUBLISHED, "health::v2::PUBLISHED");
    assert_eq!(published.last(), doc["current"].as_str().as_ref());
    let types = doc["types"].as_array().expect("types");
    assert_eq!(
        types.iter().map(|t| &t["name"]).collect::<Vec<_>>(),
        RecordType::ALL.map(RecordType::name),
        "the schema's three types"
    );
    for (t, ty) in types.iter().zip(RecordType::ALL) {
        let of_file = packed::layout_in(&file, ty.name()).expect("the file's type");
        assert_eq!(t["tag"], json!(of_file.tag()), "{}'s tag", ty.name());
        assert_eq!(ty.tag(), of_file.tag(), "{}'s tag in the model", ty.name());
        assert_eq!(
            ty.layout(),
            &of_file,
            "the model's {} is the standard schema's, names included",
            ty.name()
        );
    }
    let root = repo_root();
    let cases = doc["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 4, "schema.json's cases");
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

/// health.v2 §3, the standard contract as the model's constants have it:
/// revision 2.0's fingerprint and its heartbeat's tag, its one collection
/// with a horizon, the two collections' indices in a snapshot, its three
/// resources typed by the published packed schema, and no `link.exposure`
/// (§2.8). §4: W105 reads the profile's table for a contract that lists
/// `health.v2`, and none for one that lists `health.v1`.
#[test]
fn health_v2_standard_contract() {
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
    assert_eq!(Tables::of(&c), Tables::default());
    let layout = Layout::of(&c);
    assert_eq!(layout.tag.get(), 1, "its heartbeat's tag");
    assert_eq!(
        layout
            .collections
            .iter()
            .map(|x| (x.resource(), x.ttl.as_secs(), x.class))
            .collect::<Vec<_>>(),
        [(
            "state/status".to_owned(),
            health::STATUS_TTL.as_secs(),
            v2::STATUS_CLASS
        )],
        "one collection: the status, at 60 s, of the host class"
    );
    for (template, coll, ty) in [
        (health::STATUS, v2::STATUS_COLL, RecordType::Status),
        (health::CHECKS, v2::CHECKS_COLL, RecordType::Check),
    ] {
        let r = c
            .resource(zenkey_model::grammar::KindToken::State, template)
            .unwrap_or_else(|| panic!("state/{template}"));
        assert_eq!(collection::coll_of(&c, r), Some(coll), "state/{template}");
        packed_type(&c, r, ty);
    }
    let faults = c
        .resource(zenkey_model::grammar::KindToken::Stream, health::FAULTS)
        .expect("stream/faults");
    packed_type(&c, faults, RecordType::Fault);
    assert_eq!(c.resources.len(), 3, "three resources");
    assert!(
        c.resources
            .iter()
            .all(|r| !r.annotations.contains_key("link.exposure")),
        "it exposes nothing `link` (§2.8)"
    );
    assert_eq!(
        c.resource(zenkey_model::grammar::KindToken::State, health::CHECKS)
            .and_then(|r| r.cardinality),
        Some(health::MAX_CHECKS)
    );

    // W105 (§4, core §10): any other `health.<key>` is outside the table
    // of a contract that lists health.v2. health.v1 publishes none.
    let warned = |uses: &str, key: &str| -> Vec<String> {
        let text = format!(
            "[interface]\nname = \"tc\"\nmajor = 1\nminor = 0\nuses = [\"{uses}\"]\n\n\
             [defaults]\nannotations = {{ \"health.{key}\" = [\"tc_a\"] }}\n\n\
             [resources.rates]\nkind = \"stream\"\ntype = {{ raw = \"text/plain\" }}\n"
        );
        let l = load_str(&text, &repo_root().join("spec"), None);
        assert!(l.contract.is_some(), "{uses}, {key}:\n{}", l.report);
        l.report.0.iter().map(|d| d.code.to_owned()).collect()
    };
    for key in v2::VOCABULARY {
        assert_eq!(warned("health.v2", key), Vec::<String>::new(), "{key}");
    }
    assert_eq!(warned("health.v2", "words"), ["W105"]);
    assert_eq!(
        warned("health.v1", "words"),
        Vec::<String>::new(),
        "health.v1 publishes no vocabulary, and draws no W105"
    );
}

/// `r`'s type is the packed type `ty` of the published standard schema.
fn packed_type(c: &Contract, r: &zenkey_model::contract::Resource, ty: RecordType) {
    let zenkey_model::contract::Body::Data(d) = &r.body else {
        panic!("{}: a data resource", r.template.as_str())
    };
    let TypeId::Packed { name, schema } = &d.type_ else {
        panic!("{}: a packed type, not {:?}", r.template.as_str(), d.type_)
    };
    assert_eq!(name, ty.name());
    assert!(v2::is_published(schema), "{schema}: the published schema");
    let l = c
        .packed_layout(&d.type_)
        .expect("reads")
        .expect("a packed type");
    assert_eq!(&l, ty.layout(), "{}", ty.name());
}

fn record_type(v: &Value) -> RecordType {
    let name = v.as_str().expect("type");
    RecordType::ALL
        .into_iter()
        .find(|t| t.name() == name)
        .unwrap_or_else(|| panic!("type {name}"))
}

/// One record through the model's typed reader and writer.
struct Through {
    /// Its JSON reading.
    value: Value,
    /// The bytes it writes back to.
    again: Vec<u8>,
    /// Its put under the stamp given, where one is.
    put: Option<Vec<u8>>,
}

fn through<R: Packed>(
    tag: u8,
    bytes: &[u8],
    epoch: i64,
    stamp: Option<Stamp>,
) -> Result<Through, ReadRefusal> {
    let r = R::read(tag, bytes, epoch)?;
    let again = r.write(epoch).expect("a record read writes back").bytes;
    let put = stamp.map(|s| r.put(s, epoch).expect("a record read is put"));
    Ok(Through {
        value: r.to_json(),
        again,
        put,
    })
}

fn typed(
    ty: RecordType,
    tag: u8,
    bytes: &[u8],
    epoch: i64,
    stamp: Option<Stamp>,
) -> Result<Through, ReadRefusal> {
    match ty {
        RecordType::Status => through::<Status>(tag, bytes, epoch, stamp),
        RecordType::Check => through::<Check>(tag, bytes, epoch, stamp),
        RecordType::Fault => through::<Fault>(tag, bytes, epoch, stamp),
    }
}

/// `v2/records.json`: the three types' layouts; a value → its record's
/// bits, pad and bytes and its put's payload, or the writer's refusal; a
/// tag and bytes → the value, or core §7.6's refusal; and a `clock_ahead`
/// fault's argument from the offset measured (health.v2 §2.3, §2.5, §2.6,
/// §3). Every accepted write goes through the model's typed records too:
/// read, written back to the same bytes, and put.
#[test]
fn health_v2_records() {
    let doc = read_json(&dir().join("records.json"));
    assert_eq!(doc["schema"], json!(format!("{}.json", v2::SCHEMA_STEM)));

    let layouts = doc["layouts"].as_array().expect("layouts");
    assert_eq!(layouts.len(), 3, "layouts");
    for case in layouts {
        let l = record_type(&case["type"]).layout();
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
    assert_eq!(writes.len(), 42, "writes");
    let (mut written, mut refused) = (0, std::collections::BTreeSet::new());
    for case in writes {
        let note = &case["note"];
        let ty = record_type(&case["type"]);
        let e = epoch(case);
        let at = case.get("stamp").map(stamp);
        let got = match ty.layout().write(&case["value"], Some(e)) {
            Err(r) => {
                assert!(at.is_none(), "{note}: a refused write is no put");
                refused.insert(r.token());
                json!({"refused": r.token()})
            }
            Ok(r) => {
                written += 1;
                // The typed record: read under the type's tag, to
                // `reads_as` where given, else to the value; written back
                // to the same bytes; put under the owner's stamp.
                let t = typed(ty, ty.tag(), &r.bytes, e, at).unwrap_or_else(|x| {
                    panic!("{note}: an accepted write reads back, not {}", x.token())
                });
                let reads_as = case.get("reads_as").unwrap_or(&case["value"]);
                assert_eq!(&t.value, reads_as, "{note}: reads back");
                assert_eq!(t.again, r.bytes, "{note}: the typed record's bytes");
                let mut got = json!({
                    "bits": r.bit_string(),
                    "pad": r.pad(),
                    "hex": hex(&r.bytes),
                });
                if let Some(put) = t.put {
                    got["payload"] = json!(hex(&put));
                }
                got
            }
        };
        assert_eq!(case["expect"], got, "write: {note}");
    }
    assert_eq!(written, 25, "accepted writes");
    assert_eq!(
        refused.into_iter().collect::<Vec<_>>(),
        ["missing", "range", "unknown", "value"],
        "every writer's refusal pinned"
    );

    let reads = doc["reads"].as_array().expect("reads");
    assert_eq!(reads.len(), 49, "reads");
    let mut refused = std::collections::BTreeSet::new();
    for case in reads {
        let note = &case["note"];
        let ty = record_type(&case["type"]);
        let tag = u8::try_from(case["tag"].as_u64().expect("tag")).expect("a tag");
        let bytes = unhex(case["hex"].as_str().expect("hex"));
        let e = epoch(case);
        let read = typed(ty, tag, &bytes, e, None);
        // The layout's own reading, which the typed one stands on.
        assert_eq!(
            ty.layout().read(tag, &bytes, Some(e)).ok(),
            read.as_ref().ok().map(|t| t.value.clone()),
            "{note}: the typed reading is the layout's"
        );
        // §2.6: a refused status or check is unknown, never ok and never
        // the level its bits would read.
        let level = |r: Result<Option<health::Level>, ReadRefusal>| v2::level_read(r);
        match ty {
            RecordType::Status => {
                let l = level(Status::read(tag, &bytes, e).map(|s| s.level));
                assert_eq!(l == Read::Undecodable, read.is_err(), "{note}");
            }
            RecordType::Check => {
                let l = level(Check::read(tag, &bytes, e).map(|c| c.level));
                assert_eq!(l == Read::Undecodable, read.is_err(), "{note}");
            }
            RecordType::Fault => {}
        }
        let got = match read {
            Ok(t) => {
                // A later text's bits behind the marker are not written
                // back: never more bytes than were read.
                assert!(t.again.len() <= bytes.len(), "{note}: writes back");
                json!({"value": t.value})
            }
            Err(r) => {
                refused.insert(r.token());
                json!({"refused": r.token()})
            }
        };
        assert_eq!(case["expect"], got, "read: {note}");
    }
    assert_eq!(
        refused.into_iter().collect::<Vec<_>>(),
        ["length", "pad", "tag"],
        "a record of these types is refused by its tag, its length or its pad, never a code"
    );

    let arguments = doc["arguments"].as_array().expect("arguments");
    assert_eq!(arguments.len(), 8, "arguments");
    for case in arguments {
        let offset = case["offset_ms"].as_u64().map(Duration::from_millis);
        assert_eq!(
            offset.is_none(),
            case["offset_ms"].is_null(),
            "an offset in ms or null"
        );
        let a = Argument::clock_ahead(offset);
        assert_eq!(
            case["expect"],
            json!({"argument": a.to_json()}),
            "{}",
            case["note"]
        );
        assert_eq!(Fault::clock_ahead(offset, None).argument, a);
    }
}

fn kind(v: &Value) -> Kind {
    let name = v.as_str().expect("kind");
    Kind::ALL
        .into_iter()
        .find(|k| k.table() == name)
        .unwrap_or_else(|| panic!("kind {name}"))
}

/// A table as the fixtures write one: a list of names, or `null` for
/// none.
fn table(v: &Value) -> Option<Vec<String>> {
    match v {
        Value::Null => None,
        v => Some(serde_json::from_value(v.clone()).expect("a list of names")),
    }
}

fn tables(v: &Value) -> Tables {
    Tables {
        reasons: table(&v["reasons"]),
        details: table(&v["details"]),
        codes: table(&v["codes"]),
    }
}

/// `v2/tables.json`: the profile's three tables; a contract's tables from
/// its resolved annotations; an instance's from the revisions its
/// descriptor names; a word written as a number and read back as a name;
/// and a table's growth (health.v2 §2.10, §4).
#[test]
fn health_v2_tables() {
    let doc = read_json(&dir().join("tables.json"));
    for k in Kind::ALL {
        assert_eq!(
            doc["profile"][k.table()],
            json!(k.profile()),
            "the profile's {}",
            k.table()
        );
        assert_eq!(k.key(), format!("health.{}", k.table()));
    }
    assert_eq!(
        v2::VOCABULARY,
        Kind::ALL.map(Kind::table),
        "the vocabulary is the three keys"
    );

    let declarations = doc["declarations"].as_array().expect("declarations");
    assert_eq!(declarations.len(), 23, "declarations");
    let mut invalid = std::collections::BTreeSet::new();
    for case in declarations {
        let uses_v2 = case["uses"]
            .as_array()
            .expect("uses")
            .iter()
            .any(|u| u == v2::PROFILE);
        let resources = case["resources"].as_object().expect("resources");
        let mut got = serde_json::Map::new();
        for k in Kind::ALL {
            let carried = resources.values().filter_map(|a| a.get(k.key()));
            let d = v2::declaration(uses_v2, carried);
            got.insert(
                k.table().to_owned(),
                match &d {
                    Declared::None => Value::Null,
                    Declared::Table(t) => json!(t),
                    Declared::Invalid(why) => {
                        invalid.insert(why.as_str());
                        json!({"invalid": why.as_str()})
                    }
                },
            );
            // An invalid declaration holds no table.
            assert_eq!(d.table().is_some(), matches!(d, Declared::Table(_)));
        }
        assert_eq!(case["expect"], Value::Object(got), "{}", case["note"]);
    }
    assert_eq!(
        invalid.into_iter().collect::<Vec<_>>(),
        ["disagrees", "not_used", "value"],
        "every reason pinned"
    );

    let instances = doc["instances"].as_array().expect("instances");
    assert_eq!(instances.len(), 10, "instances");
    for case in instances {
        let holds: BTreeMap<&str, Tables> = case["holds"]
            .as_object()
            .expect("holds")
            .iter()
            .map(|(name, t)| (name.as_str(), tables(t)))
            .collect();
        // The named revisions the reader holds: a revision it holds that
        // the descriptor does not name adds nothing.
        let named = case["names"]
            .as_array()
            .expect("names")
            .iter()
            .filter_map(|n| holds.get(n.as_str().expect("a revision")));
        let got = Tables::of_instance(named);
        let mut out = serde_json::Map::new();
        for k in Kind::ALL {
            out.insert(
                k.table().to_owned(),
                match got.get(k) {
                    Instance::None => Value::Null,
                    Instance::Table(t) => json!(t),
                    Instance::Differ => json!("differ"),
                },
            );
            assert_eq!(got.own(k), got.get(k).table());
        }
        assert_eq!(case["expect"], Value::Object(out), "{}", case["note"]);
    }

    let writes = doc["writes"].as_array().expect("writes");
    assert_eq!(writes.len(), 20, "writes");
    for case in writes {
        let own = table(&case["own"]);
        let w = v2::write_word(kind(&case["kind"]), case["word"].as_str(), own.as_deref());
        assert_eq!(
            case["word"].is_null(),
            case["word"].as_str().is_none(),
            "a word or null"
        );
        assert_eq!(case["expect"], w.to_json(), "write: {}", case["note"]);
    }

    let reads = doc["reads"].as_array().expect("reads");
    assert_eq!(reads.len(), 23, "reads");
    for case in reads {
        let own = table(&case["own"]);
        let word = Word::of_json(&case["number"]).expect("a number or out_of_range");
        let got = match v2::read_word(kind(&case["kind"]), word, own.as_deref()) {
            Named::None => json!({"none": true}),
            Named::Profile(name) => json!({"name": name, "of": "profile"}),
            Named::Service(name) => json!({"name": name, "of": "service"}),
            Named::Number(n) => json!({"number": n}),
            Named::OutOfRange => json!({"out_of_range": true}),
        };
        assert_eq!(case["expect"], got, "read: {}", case["note"]);
    }

    let growth = doc["growth"].as_array().expect("growth");
    assert_eq!(growth.len(), 8, "growth");
    for case in growth {
        let (old, new) = (
            table(&case["old"]).expect("old"),
            table(&case["new"]).expect("new"),
        );
        assert_eq!(
            case["expect"],
            json!(v2::growth(&old, &new).as_str()),
            "{}",
            case["note"]
        );
    }
}

/// A record read, as the fixtures write one: its fields, or `{refused}`.
fn record_json<R: Packed>(r: &Result<R, ReadRefusal>) -> Value {
    match r {
        Ok(r) => r.to_json(),
        Err(x) => json!({"refused": x.token()}),
    }
}

fn ms(t: collection::Time, epoch: i64) -> Value {
    t.unix_ms(epoch)
        .map_or(Value::Null, |t| json!(i64::try_from(t).expect("ms")))
}

fn entry_json<R: Packed>(e: &v2::Entry<R>, epoch: i64) -> Value {
    json!({
        "stamp": stamp_text(e.stamp),
        "confirmation": match e.confirmation {
            Confirmation::None => "none",
            Confirmation::AtTime => "time",
            Confirmation::At(_) => "conf",
        },
        "confirmed_at": e.confirmed_at.map_or(Value::Null, |t| ms(t, epoch)),
        "record": record_json(&e.record),
    })
}

fn heard_json(h: &v2::Heard, epoch: i64) -> Value {
    json!({
        "complete": h.complete,
        "run": h.run.hex(),
        "asof": h.asof.get(),
        "time": ms(h.time, epoch),
    })
}

/// `v2/snapshots.json`: a late joiner's reading of one owner's answer to
/// a GET over its `health.v2/state/**`, entry by entry, whether the
/// reading is one instant, and the break of §2.2 a tool may report from
/// it alone (health.v2 §2.2, §2.4).
#[test]
fn health_v2_snapshots() {
    let doc = read_json(&dir().join("snapshots.json"));
    let cases = doc["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 25, "snapshots.json's cases");
    let (mut instants, mut breaks) = (0, 0);
    for case in cases {
        let note = &case["note"];
        let e = epoch(case);
        let replies: Vec<(String, Vec<u8>)> = case["replies"]
            .as_array()
            .expect("replies")
            .iter()
            .map(|r| {
                (
                    r["key"].as_str().expect("key").to_owned(),
                    unhex(r["hex"].as_str().expect("hex")),
                )
            })
            .collect();
        // Every page is one core §7.5 accepts, of a collection of the
        // standard contract.
        for (key, payload) in &replies {
            let page = Page::parse(payload).unwrap_or_else(|x| panic!("{note}: {key}: {x:?}"));
            assert!(
                v2::Collection::of(key, page.coll).is_some(),
                "{note}: {key}, collection {}",
                page.coll
            );
        }
        let r = Reading::of(replies.iter().map(|(k, p)| (k.as_str(), p.as_slice())), e);
        assert!(!r.mixed, "{note}: one snapshot of each collection");
        let status = r.status.as_ref().map_or(Value::Null, |s| {
            let mut v = heard_json(&s.heard, e);
            v["entry"] = match &s.entry {
                StatusEntry::None => Value::Null,
                StatusEntry::Tombstone(at) => {
                    json!({"tombstone": true, "stamp": stamp_text(*at)})
                }
                StatusEntry::Live(entry) => entry_json(entry, e),
            };
            v
        });
        let checks = r.checks.as_ref().map_or(Value::Null, |c| {
            let mut v = heard_json(&c.heard, e);
            v["live"] = c
                .live
                .iter()
                .map(|entry| {
                    let mut v = entry_json(entry, e);
                    v["check"] = json!(entry.id);
                    v
                })
                .collect();
            v["retired"] = c
                .retired
                .iter()
                .map(|(check, at)| json!({"check": check, "stamp": stamp_text(*at)}))
                .collect();
            v
        });
        let got = json!({
            "status": status,
            "checks": checks,
            "instant": r.is_one_instant(),
            "status_better_than_a_check": r.status_better_than_a_check(),
        });
        assert_eq!(case["expect"], got, "{note}");
        instants += usize::from(r.is_one_instant());
        breaks += usize::from(r.status_better_than_a_check() == Some(true));
    }
    assert!(
        instants > 0 && instants < cases.len() && breaks > 0,
        "both answers of each question pinned: {instants} instants, {breaks} breaks"
    );
}

/// A case's descriptor, face and deployment's word, as
/// `health/conformance/v2/` writes them: the presence a reading is judged
/// under, and the major (§2.11's step 2, §2.12). Across a face no
/// descriptor crosses, and the deployment's word names the interface;
/// `clock.json` states none, its cases being this major's.
fn presence_and_major(case: &Value) -> (Presence, Option<Major>) {
    match case["presence"].as_str().expect("presence") {
        "present" => match &case["descriptor"] {
            Value::Null => (Presence::Present(Listing::Unread), None),
            d => {
                let interfaces: Vec<&str> = d["interfaces"]
                    .as_array()
                    .expect("descriptor.interfaces")
                    .iter()
                    .map(|i| i.as_str().expect("an interface id"))
                    .collect();
                let major = Major::of(interfaces.iter().copied());
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
        "across_face" => {
            let major = match case.get("deployment") {
                None => Major::V2,
                Some(d) => Major::named(d.as_str().expect("deployment")).expect("a major"),
            };
            let status_crosses = case["face"]["status_crosses"]
                .as_bool()
                .expect("face.status_crosses");
            (Presence::AcrossFace { status_crosses }, Some(major))
        }
        other => panic!("presence {other:?}"),
    }
}

fn major_json(m: Option<Major>) -> Value {
    m.map_or(Value::Null, |m| match m {
        Major::V1 => json!(1),
        Major::V2 => json!(2),
    })
}

/// `freshness.v2`'s verdict over the status's observations, as a case
/// states it judged.
fn freshness_judged(v: &Value) -> freshness::Judged {
    use freshness::{Reason as R, Verdict as V};
    let token = |v: &Value| v.as_str().expect("a token").to_owned();
    let verdict = [V::Fresh, V::Stale, V::Unobservable, V::NotAsked]
        .into_iter()
        .find(|x| x.as_str() == token(&v["verdict"]))
        .unwrap_or_else(|| panic!("freshness verdict {}", v["verdict"]));
    let reason = [
        R::WithinHorizon,
        R::NeverStale,
        R::BeyondHorizon,
        R::NoDelivery,
        R::NotAHorizon,
        R::ListenedTooShort,
        R::Incomplete,
        R::Silent,
        R::NoStamp,
        R::ClockUntrusted,
        R::ClockDisagrees,
        R::NearHorizon,
        R::NoObservation,
        R::NoHorizon,
        R::Kind,
        R::LastKnown,
        R::Deleted,
    ]
    .into_iter()
    .find(|x| x.as_str() == token(&v["reason"]))
    .unwrap_or_else(|| panic!("freshness reason {}", v["reason"]));
    freshness::Judged { verdict, reason }
}

/// `v2/judgements.json`: one reading of one service → the major it is
/// judged under and the answer to "is this service healthy?" (health.v2
/// §2.1, §2.4, §2.6 to §2.8, §2.11, §2.12). The status's freshness is
/// given as judged; its level and each check's are read from their
/// records, a refused one and one at `unknown` being unknown.
#[test]
fn health_v2_judgements() {
    const EPOCH: i64 = 1_767_225_600;
    let doc = read_json(&dir().join("judgements.json"));
    let cases = doc["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 61, "judgements.json's cases");
    let record = |v: &Value| {
        (
            u8::try_from(v["tag"].as_u64().expect("tag")).expect("a tag"),
            unhex(v["hex"].as_str().expect("hex")),
        )
    };
    let mut v1 = 0;
    for case in cases {
        let note = &case["note"];
        let (presence, major) = presence_and_major(case);
        if case["expect"] == json!({"major": 1}) {
            // health.v1's own judgements.json is the evidence.
            assert_eq!(major, Some(Major::V1), "{note}");
            v1 += 1;
            continue;
        }
        let (status, level) = match &case["status"] {
            // No status read: no observation of it.
            Value::Null => (freshness::judge_all(&health::status_horizon(), &[]), None),
            s => (
                freshness_judged(&s["freshness"]),
                match &s["record"] {
                    Value::Null => None,
                    r => {
                        let (tag, bytes) = record(r);
                        Some(v2::level_read(
                            Status::read(tag, &bytes, EPOCH).map(|s| s.level),
                        ))
                    }
                },
            ),
        };
        let checks: Option<Vec<Read>> = match &case["checks"] {
            Value::Null => None,
            v => Some(
                v.as_array()
                    .expect("checks")
                    .iter()
                    .map(|c| {
                        let (tag, bytes) = record(c);
                        v2::level_read(Check::read(tag, &bytes, EPOCH).map(|c| c.level))
                    })
                    .collect(),
            ),
        };
        let j = health::judge_status(presence, status, level, checks.as_deref());
        let got = json!({
            "major": major_json(major),
            "verdict": j.verdict.as_str(),
            "reason": j.reason.as_str(),
            "level": j.level.map_or(Value::Null, |l| json!(l.as_str())),
        });
        assert_eq!(case["expect"], got, "{note}");
    }
    assert_eq!(v1, 2, "the cases read by health.v1's text");
}

/// `v2/clock.json`: a put on the heartbeat key covering `health.v2` → the
/// time of a read heartbeat, or why it was not read; and what one reader
/// heard of one service → the answer to "is this service's clock ahead?"
/// (health.v2 §2.5, §5). A heartbeat is read by `freshness.v2` §2.5's
/// steps, through the reader's [`View`] of the standard contract.
#[test]
fn health_v2_clock() {
    let doc = read_json(&dir().join("clock.json"));
    let layout = Layout::of(&standard());

    // What the payloads were built from.
    let h = &doc["heartbeat"];
    let iface: zenkey_model::grammar::IfaceId = v2::IFACE.parse().expect("an interface");
    assert_eq!(
        h["key"],
        json!(heartbeat::key("lab", "svc", &iface)),
        "the heartbeat key covering health.v2"
    );
    assert_eq!(h["collection"], json!(layout.collections[0].resource()));
    let member = (h["member"].as_str().expect("member"), stamp(&h["stamp"]));
    assert_eq!(h["digest"], json!(Digest::of([member]).to_string()));

    let at = |n: u64| Mono::after(Duration::from_millis(n));
    let heartbeats = doc["heartbeats"].as_array().expect("heartbeats");
    assert_eq!(heartbeats.len(), 16, "heartbeats");
    let mut seen = std::collections::BTreeSet::new();
    for case in heartbeats {
        let note = &case["note"];
        assert_eq!(
            case["holds_tag"],
            json!(layout.tag.get()),
            "{note}: the reader holds revision 2.0"
        );
        let mut view: View<Mono> = View::new(layout.clone());
        assert!(view.follow_resource("state/status"));
        // The last heartbeat it read on that key, whatever it carried.
        if !case["last_read"].is_null() {
            let before = Heartbeat {
                time: Time::Unknown,
                sections: vec![None],
            }
            .envelope(&layout, stamp(&case["last_read"]))
            .expect("writes");
            let env = Envelope::parse(&before).expect("an envelope");
            let (tag, body) = heartbeat::body_of(&env).expect("form 6");
            assert!(
                matches!(
                    view.apply(tag, env.stamp, body, at(0)),
                    Applied::Read { .. }
                ),
                "{note}: the last read heartbeat"
            );
        }
        let payload = unhex(case["payload"].as_str().expect("payload"));
        let env = Envelope::parse(&payload).unwrap_or_else(|x| panic!("{note}: envelope {x}"));
        let (tag, body) = heartbeat::body_of(&env).expect("core §7.5's form 6");
        let (got, heard) = match view.apply(tag, env.stamp, body, at(1)) {
            Applied::Revision => (json!({"unread": "revision"}), ClockHeard::HeartbeatUnread),
            Applied::Refused(_) => (json!({"unread": "refused"}), ClockHeard::HeartbeatUnread),
            Applied::Older => (json!({"unread": "older"}), ClockHeard::HeartbeatUnread),
            // A read heartbeat, whatever its sections say.
            Applied::Read { .. } => {
                let time = view.owner_time().expect("a read heartbeat's time");
                let word = match time {
                    Time::At { .. } => "instant",
                    Time::Ahead => "ahead",
                    Time::Unknown => "unknown",
                };
                (json!({"read": word}), ClockHeard::Heartbeat(time))
            }
        };
        assert_eq!(case["expect"], got, "{note}");
        seen.insert(got.to_string());
        // One heartbeat alone answers, where it was read.
        let listed = Presence::Present(Listing::Listed { token: true });
        let alone = v2::clock_ahead(listed, &[heard]);
        let want = match heard {
            ClockHeard::Heartbeat(Time::Ahead) => "yes",
            ClockHeard::Heartbeat(Time::At { .. }) => "no",
            _ => "unobservable",
        };
        assert_eq!(alone.answer.as_str(), want, "{note}");
    }
    assert_eq!(
        seen.len(),
        6,
        "each reading and each unread pinned: {seen:?}"
    );

    let cases = doc["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 37, "clock.json's cases");
    let instant = Time::At { secs: 1, ms: 0 };
    let mut v1 = 0;
    for case in cases {
        let note = &case["note"];
        let (presence, major) = presence_and_major(case);
        let tokens: Vec<&str> = case["heard"]
            .as_array()
            .expect("heard")
            .iter()
            .map(|h| h.as_str().expect("a delivery"))
            .collect();
        if case["expect"] == json!({"major": 1}) {
            // health.v1's question, where a time that is no instant
            // answers nothing alone (v1 §2.5): the fixture expects nothing
            // more, and the model's v1 answer is checked beside it.
            assert_eq!(major, Some(Major::V1), "{note}");
            assert_eq!(tokens, ["heartbeat_ahead"], "{note}");
            let c = health::clock_ahead(presence, &[health::Heard::heartbeat(Time::Ahead)]);
            assert_eq!(
                (c.answer, c.reason),
                (
                    health::Answer::Unobservable,
                    health::ClockReason::TimeUnknown
                ),
                "{note}"
            );
            v1 += 1;
            continue;
        }
        let heard: Vec<ClockHeard> = tokens
            .iter()
            .map(|h| match *h {
                "heartbeat" => ClockHeard::Heartbeat(instant),
                "heartbeat_ahead" => ClockHeard::Heartbeat(Time::Ahead),
                "heartbeat_unknown" => ClockHeard::Heartbeat(Time::Unknown),
                "heartbeat_unread" => ClockHeard::HeartbeatUnread,
                "status_put" => ClockHeard::StatusPut,
                "status_deleted" => ClockHeard::StatusDeleted,
                "clock_ahead" => ClockHeard::ClockAhead,
                "fault" => ClockHeard::Fault,
                other => panic!("heard {other:?}"),
            })
            .collect();
        let c = v2::clock_ahead(presence, &heard);
        let got = json!({
            "major": major_json(major),
            "answer": c.answer.as_str(),
            "reason": c.reason.as_str(),
        });
        assert_eq!(case["expect"], got, "{note}");
    }
    assert_eq!(v1, 1, "the case that is health.v1's question");
}
