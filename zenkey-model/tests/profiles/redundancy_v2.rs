//! `redundancy.v2`'s fixtures (`spec/profiles/redundancy/conformance/v2/`,
//! #987's N1), run against `zenkey_model::redundancy` as written. Each
//! expectation was computed twice by two scripts of the text's drafter and
//! never blessed, and no runner here blesses one: a failure is the
//! model's, or a fixture or a sentence to raise with the bytes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde_json::{Value, json};
use zenkey_model::chunk;
use zenkey_model::grammar::{Addr, IfaceId, InstanceId};
use zenkey_model::redundancy::{
    self, Declaration, Described, Fence, Governance, Holding, Major, Member, Outcome, Revision,
    Sample,
};
use zenkey_model::wire::{self, DeleteStamp, Envelope, Form, GetReply, Run, Seq, Stamp};

use super::{annotations, profiles, read_json, unhex};

fn dir() -> PathBuf {
    profiles().join("redundancy/conformance/v2")
}

fn id(v: &Value) -> InstanceId {
    v.as_str()
        .expect("an id")
        .parse()
        .expect("16 lowercase hex digits")
}

/// A run as the fixtures spell one: 4 or 8 lowercase hexadecimal digits,
/// which writes back to itself.
fn run(v: &Value) -> Run {
    let text = v.as_str().expect("a run");
    let r = Run::from_hex(text).expect("4 or 8 lowercase hexadecimal digits");
    assert_eq!(r.hex(), text, "a run writes back");
    r
}

/// A zid in hexadecimal, read by value (observer §4.4).
fn zid(v: &Value) -> u128 {
    u128::from_str_radix(v.as_str().expect("a zid"), 16).expect("hexadecimal")
}

/// The one address every case's follower follows.
fn addr() -> Addr {
    "lab/ledger".parse().expect("an address")
}

fn cases<'a>(doc: &'a Value, member: &str, n: usize) -> &'a [Value] {
    let cases = doc[member].as_array().expect(member);
    assert_eq!(cases.len(), n, "{member}: the README's count");
    cases
}

/// `v2/declarations.json`'s `cases`: a contract's `uses` and resources,
/// and the tokenless set → its declaration (v2 §2.2); and its `addresses`:
/// the revisions of an address's candidates → the governance a tool reads
/// (v2 §2.2, §2.15).
#[test]
fn redundancy_v2_declarations() {
    let doc = read_json(&dir().join("declarations.json"));
    for case in cases(&doc, "cases", 38) {
        let uses: Vec<IfaceId> = case["contract"]["uses"]
            .as_array()
            .expect("uses")
            .iter()
            .map(|u| u.as_str().expect("a profile").parse().expect("an id"))
            .collect();
        let resources: Vec<(BTreeMap<String, Value>, bool)> = case["contract"]["resources"]
            .as_array()
            .expect("resources")
            .iter()
            .map(|r| {
                let explicit = r.get("explicit").is_some_and(|e| e == &json!(true));
                (
                    annotations(&r["annotations"]),
                    r["kind"] == json!("stream") && explicit,
                )
            })
            .collect();
        let d = redundancy::v2::declaration(
            &uses,
            resources.iter().map(|(a, e)| (a, *e)),
            case["tokenless"] == json!(true),
        );
        let got = match &d {
            Declaration::Governed(e) => json!({"declaration": d.as_str(), "election": e.as_str()}),
            Declaration::Invalid(r) => json!({
                "declaration": d.as_str(),
                "reasons": r.iter().map(|r| r.as_str()).collect::<Vec<_>>(),
            }),
            Declaration::NotGoverned => json!({"declaration": d.as_str()}),
        };
        assert_eq!(case["expect"], got, "{}", case["note"]);
    }
    for case in cases(&doc, "addresses", 16) {
        let revisions = case["revisions"]
            .as_array()
            .expect("revisions")
            .iter()
            .map(|r| match r["declaration"].as_str().expect("declaration") {
                "governed" => Revision::Governed(
                    Major::from_number(
                        u32::try_from(r["major"].as_u64().expect("major")).expect("a major"),
                    )
                    .expect("1 or 2"),
                    match r["election"].as_str().expect("election") {
                        "claim" => redundancy::Election::Claim,
                        "external" => redundancy::Election::External,
                        other => panic!("election {other:?}"),
                    },
                ),
                "not_governed" => Revision::NotGoverned,
                "invalid" => Revision::Invalid,
                "unread" => Revision::Unread,
                other => panic!("declaration {other:?}"),
            });
        let got = json!({"governance": redundancy::governance(revisions).as_str()});
        assert_eq!(case["expect"], got, "{}", case["note"]);
    }
}

/// `v2/descriptors.json`: a descriptor, the governed interface's id and its
/// governance → its run, whether it lists the interface, the mark and the
/// findings (v2 §2.10, §2.14).
#[test]
fn redundancy_v2_descriptors() {
    let doc = read_json(&dir().join("descriptors.json"));
    for case in cases(&doc, "cases", 29) {
        let governance = Governance::parse(case["governance"].as_str().expect("governance"))
            .expect("a governance");
        let r = redundancy::read_descriptor(
            &case["descriptor"],
            case["iface"].as_str().expect("iface"),
            governance,
        );
        let got = json!({
            "findings": r.findings.iter().map(|f| f.as_str()).collect::<Vec<_>>(),
            "listed": r.listed,
            "mark": r.mark.map(redundancy::Mark::as_str),
            "run": r.run.map(Run::hex),
        });
        assert_eq!(case["expect"], got, "{}", case["note"]);
    }
}

/// A fence following `addr()`, whose holders are `holders` and whose
/// instances' descriptors stated what `instances` gives: for each, its
/// major, its runs, and its `meta.zid` where read.
fn follower(
    holders: &Value,
    instances: impl IntoIterator<Item = (InstanceId, Described)>,
) -> Fence<String> {
    let mut f = Fence::default();
    let holders: BTreeSet<InstanceId> = holders
        .as_array()
        .expect("holders")
        .iter()
        .map(id)
        .collect();
    f.hear(Holding::Read(addr(), f.ask(), true, holders), false);
    for (i, d) in instances {
        let out = f.describe(&addr(), i, d);
        assert_eq!(out, Outcome::default(), "nothing is held yet");
    }
    f
}

/// What a sample's form carries, read from its bytes as core §7.5 parses
/// that form (v2 §2.10's table), `None` for a sample with no envelope.
fn stamp_of(sample: &Value) -> Option<Stamp> {
    let form = sample["form"].as_str().expect("form");
    let bytes = || unhex(sample["bytes"].as_str().expect("bytes"));
    let put = |want: Form| {
        let b = bytes();
        let e = Envelope::parse(&b).unwrap_or_else(|r| panic!("{form}: {r}"));
        assert_eq!(e.form, want, "{form}");
        e.stamp
    };
    Some(match form {
        "none" => return None,
        "mutation" => put(Form::Mutation),
        "frame" => put(Form::Frame),
        "heartbeat" | "owner_heartbeat" => put(Form::Heartbeat),
        "snapshot" => {
            let b = bytes();
            match wire::read_get_reply(&b).unwrap_or_else(|r| panic!("{form}: {r}")) {
                GetReply::Page(e) => e.stamp,
                other => panic!("{form}: {other:?}"),
            }
        }
        "held_stamp" => match wire::read_get_reply(&bytes()).expect("a held stamp") {
            GetReply::Held(h) => h.stamp,
            other => panic!("{form}: {other:?}"),
        },
        "delete" => {
            DeleteStamp::parse(&bytes())
                .expect("a delete's attachment")
                .stamp
        }
        "value" => wire::read_reply(&bytes()).expect("a value").stamp,
        "error" => wire::read_reply_err(&bytes()).expect("an error").stamp,
        "event" => {
            let c = sample["chunk"].as_str().expect("an event's chunk");
            chunk::occurrence(c).expect("an occurrence chunk")
        }
        other => panic!("form {other:?}"),
    })
}

/// `v2/fences.json`'s `samples`: a follower's holders and what each
/// instance's descriptors stated, one sample, and whether a read asked
/// after it has ended → its run and what the rule decides (v2 §2.10,
/// §2.17).
#[test]
fn redundancy_v2_fence_samples() {
    let doc = read_json(&dir().join("fences.json"));
    for case in cases(&doc, "samples", 49) {
        let follower_of = &case["follower"];
        let mut instances = Vec::new();
        for (i, k) in follower_of["instances"].as_object().expect("instances") {
            let i: InstanceId = i.parse().expect("an id");
            let major = Major::from_number(
                u32::try_from(k["major"].as_u64().expect("major")).expect("a major"),
            )
            .expect("1 or 2");
            let zid = k.get("zid").map(zid);
            instances.push((
                i.clone(),
                Described {
                    major: Some(major),
                    run: None,
                    zid,
                },
            ));
            for r in k["runs"].as_array().expect("runs") {
                instances.push((
                    i.clone(),
                    Described {
                        major: Some(major),
                        run: Some(run(r)),
                        zid,
                    },
                ));
            }
        }
        let fence = follower(&follower_of["holders"], instances);
        let stamp = stamp_of(&case["sample"]);
        let sample = Sample {
            stamp,
            timestamp_id: case["sample"].get("stamp").map(zid),
        };
        let ended = case["read_ended"].as_bool().expect("read_ended");
        let fenced = fence.judge(&addr(), &sample, ended);
        let got = json!({
            "decision": fenced.decision(),
            "reason": fenced.reason(),
            "run": stamp.map(|s| s.run.hex()),
        });
        assert_eq!(case["expect"], got, "{}", case["note"]);
    }
}

/// A step's outcome as the fixture writes it, `offered` the sample an
/// offer was given, which `held` names when the fence holds it.
fn step_outcome(out: Outcome<String>, offered: Option<&str>) -> Value {
    let discarded: Vec<Value> = out
        .discarded
        .iter()
        .map(|(i, d)| json!({"id": i, "reason": d.as_str()}))
        .collect();
    let held: Vec<&str> = offered.filter(|_| out.held).into_iter().collect();
    json!({
        "accepted": out.accepted,
        "asks_read": out.asks_read,
        "discarded": discarded,
        "held": held,
        "superseded": out.superseded,
    })
}

/// `{instance, run}` as a descriptor of a `redundancy.v2` instance.
fn v2_descriptor(d: &Value) -> (InstanceId, Described) {
    (
        id(&d["instance"]),
        Described {
            major: Some(Major::V2),
            run: Some(run(&d["run"])),
            zid: None,
        },
    )
}

/// `v2/fences.json`'s `sequences`: a follower of `redundancy.v2`
/// incumbents, and its steps in arrival order → what each step decides,
/// holds, supersedes, and whether it asks a read (v2 §2.10).
#[test]
fn redundancy_v2_fence_sequences() {
    let doc = read_json(&dir().join("fences.json"));
    for case in cases(&doc, "sequences", 13) {
        let start = &case["start"];
        let mut instances = Vec::new();
        for (i, runs) in start["runs"].as_object().expect("runs") {
            for r in runs.as_array().expect("runs") {
                instances.push(v2_descriptor(&json!({"instance": i, "run": r})));
            }
        }
        let mut fence = follower(&start["holders"], instances);
        let mut got = Vec::new();
        for step in case["steps"].as_array().expect("steps") {
            let out = if let Some(s) = step.get("sample") {
                let stamp = (!s["run"].is_null()).then(|| Stamp {
                    run: run(&s["run"]),
                    seq: Seq::new(u32::try_from(s["seq"].as_u64().expect("seq")).expect("a seq"))
                        .expect("a seq"),
                });
                let member = match s["kind"].as_str().expect("kind") {
                    "state" => Member::Key(s["key"].as_str().expect("key").to_owned()),
                    "stream" | "event" | "heartbeat" => Member::Alone,
                    other => panic!("kind {other:?}"),
                };
                let item = s["id"].as_str().expect("id").to_owned();
                let out = fence.offer(
                    &addr(),
                    Sample {
                        stamp,
                        timestamp_id: None,
                    },
                    member,
                    item.clone(),
                );
                step_outcome(out, Some(&item))
            } else if let Some(t) = step.get("token") {
                let i = id(&t["instance"]);
                fence.hear(
                    if t["held"] == json!(true) {
                        Holding::Declared(addr(), i)
                    } else {
                        Holding::Withdrawn(addr(), i)
                    },
                    false,
                );
                step_outcome(Outcome::default(), None)
            } else if let Some(d) = step.get("descriptor") {
                let (i, d) = v2_descriptor(d);
                step_outcome(fence.describe(&addr(), i, d), None)
            } else if let Some(r) = step.get("read_ends") {
                let ds: Vec<(InstanceId, Described)> = r["descriptors"]
                    .as_array()
                    .expect("descriptors")
                    .iter()
                    .map(v2_descriptor)
                    .collect();
                step_outcome(fence.read_ended(&addr(), ds), None)
            } else {
                panic!("a step {step}");
            };
            got.push(out);
        }
        assert_eq!(case["expect"], json!(got), "{}", case["note"]);
    }
}

/// `v2/fences.json`'s `draws`: a candidate's own run and the runs it read
/// in other instances' descriptors of its address → whether it draws again
/// (v2 §2.10).
#[test]
fn redundancy_v2_fence_draws() {
    let doc = read_json(&dir().join("fences.json"));
    for case in cases(&doc, "draws", 8) {
        let got = redundancy::draw_again(
            run(&case["own"]),
            case["read"].as_array().expect("read").iter().map(run),
        );
        assert_eq!(
            case["expect"],
            json!({"draw_again": got}),
            "{}",
            case["note"]
        );
    }
}

/// A contract as a test writes one, loaded with no finding.
fn contract(uses: &str, defaults: &str, explicit: bool) -> zenkey_model::contract::Contract {
    let text = format!(
        "[interface]\nname = \"ledger\"\nmajor = 1\nminor = 1\nuses = [{uses}]\n\n\
         [defaults]\nannotations = {{ {defaults} }}\n\n\
         [resources.balance]\nkind = \"state\"\ntype = {{ raw = \"text/plain\" }}\n\n\
         [resources.feed]\nkind = \"stream\"\nexplicit = {explicit}\ntype = {{ raw = \"text/plain\" }}\n"
    );
    let l = zenkey_model::contract::load_str(&text, std::path::Path::new("."), None);
    assert!(l.report.errors().next().is_none(), "{}", l.report);
    l.contract.expect("a contract")
}

/// A reader of both majors (v2 §2.2, §2.17): `redundancy.v2`'s reading
/// where `uses` lists it, `redundancy.v1`'s otherwise, each governed by
/// either major; an address's candidates of both with one election are
/// one election; and a governed state of `redundancy.v2` is no per-key
/// resource (v2 §2.6), where `redundancy.v1`'s still is (core §4.2).
#[test]
fn either_major_is_read_and_v2_states_are_collections() {
    use zenkey_model::collection::per_key_in;
    use zenkey_model::grammar::KindToken;

    let v1 = contract(
        r#""redundancy.v1""#,
        r#""redundancy.election" = "claim""#,
        true,
    );
    let v2 = contract(
        r#""redundancy.v2""#,
        r#""redundancy.election" = "claim""#,
        false,
    );
    let v2_stream = contract(r#""redundancy.v2""#, "", true);
    let neither = contract(r#""freshness.v2""#, "", true);
    assert_eq!(Major::of(&v1.uses), Some(Major::V1));
    assert_eq!(Major::of(&v2.uses), Some(Major::V2));
    assert_eq!(Major::of(&neither.uses), None);
    for c in [&v1, &v2, &v2_stream] {
        assert!(redundancy::is_governed(c), "{}", c.iface);
    }
    assert!(!redundancy::is_governed(&neither));
    let claim = Declaration::Governed(redundancy::Election::Claim);
    assert_eq!(redundancy::either_declaration_of(&v1, false), claim);
    assert_eq!(redundancy::either_declaration_of(&v2, false), claim);
    assert_eq!(
        redundancy::either_declaration_of(&v2_stream, false),
        Declaration::Invalid(vec![redundancy::Invalid::ExplicitStream])
    );
    // v1's reading alone does not read this major.
    assert_eq!(
        redundancy::declaration_of(&v2, false),
        Declaration::NotGoverned
    );
    assert_eq!(
        redundancy::governance([Revision::of(&v1), Revision::of(&v2)]),
        Governance::Governed(redundancy::Election::Claim)
    );
    assert_eq!(Revision::of(&v2_stream), Revision::Invalid);
    assert_eq!(Revision::of(&neither), Revision::NotGoverned);
    let none = BTreeMap::new();
    assert!(per_key_in(KindToken::State, &none, |p| p == Major::V1.profile()));
    assert!(!per_key_in(KindToken::State, &none, |p| p == Major::V2.profile()));
    for r in &v2.resources {
        assert!(
            !zenkey_model::collection::per_key(&v2, r),
            "{}",
            r.template.as_str()
        );
    }
}

/// The linter's warnings (v2 §2.2): `explicit_stream` on each explicit
/// stream of a `redundancy.v2` contract, `value` on each resource of either
/// major whose election is outside the vocabulary, and nothing for a
/// contract governed by neither.
#[test]
fn the_linter_warns_of_explicit_stream_and_value() {
    let lines = |c: &zenkey_model::contract::Contract| -> Vec<String> {
        redundancy::lint_contract(c)
            .iter()
            .map(ToString::to_string)
            .collect()
    };
    assert_eq!(
        lines(&contract(r#""redundancy.v2""#, "", true)),
        [
            "warning[redundancy.v2 explicit_stream] @stream/feed: an explicit stream's samples \
          carry no envelope, so no run a follower can fence: a redundancy.v2 interface \
          declares none"
        ]
    );
    assert!(lines(&contract(r#""redundancy.v1""#, "", true)).is_empty());
    assert!(lines(&contract(r#""freshness.v2""#, "", true)).is_empty());
    let value = lines(&contract(
        r#""redundancy.v2""#,
        r#""redundancy.election" = "quorum""#,
        true,
    ));
    assert_eq!(value.len(), 3, "{value:?}");
    assert!(
        value
            .iter()
            .any(|l| l.starts_with("warning[redundancy.v2 value] state/balance: "))
    );
    assert!(
        value
            .iter()
            .any(|l| l.starts_with("warning[redundancy.v2 value] @stream/feed: "))
    );
    assert!(
        value
            .iter()
            .any(|l| l.starts_with("warning[redundancy.v2 explicit_stream] @stream/feed: "))
    );
}
