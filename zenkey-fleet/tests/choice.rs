//! `spec/conformance/envelope/tags.json` (observer §7.3's CH's tag, §7.5's
//! revision of a key; #990 V2): which revision a tool reads a sample with
//! where no descriptor names the owner's, and what it shows of a record
//! whose type it does not hold. The fixture is evidence, written from the
//! text and checked by a second reading, never blessed: every case must
//! pass as written.
//!
//! A catalog is named by its id, mapped to a document or a file, and its
//! id is checked. `e3a529fe`, zenoh-modem's catalog of nine entries, is an
//! inline document since the fixture's answer to zk2py round 106, and
//! `29bf7d47` (modem.v3 3.4 at index 10, #991), `examples/zk2/zenoh-modem/
//! catalog.json` copied whole with its lineage, since round 107's: cases 31
//! to 35 read 3.4's `face/` events through it. A file whose id is not the
//! one named is read through its lineage (core §8.6): the predecessor the
//! lineage names, its entries the first it counts, is rebuilt and its id
//! checked.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use zenkey_fleet::model::choice::{Choice, Heard, Step, Target, choose, tag_of};
use zenkey_fleet::model::deployment::{Catalogs, DeploymentCatalog};
use zenkey_fleet::model::inband::Unheld;
use zenkey_fleet::{ContractSet, Revision};
use zenkey_model::grammar::ZkKey;
use zenkey_model::wire::Header;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// The catalog `id` names: a document, or a file read as itself or as the
/// predecessor its lineage names, its id checked either way.
fn catalog(id: &str, at: &Value) -> DeploymentCatalog {
    let doc: Value = match at {
        Value::String(path) => {
            let text = std::fs::read(root().join(path)).expect("the catalog file");
            let doc: Value = serde_json::from_slice(&text).expect("JSON");
            let whole = DeploymentCatalog::parse_bytes(&text).expect("a catalog");
            if whole.id().to_string() == id {
                return whole;
            }
            let entries = doc["lineage"]
                .as_array()
                .and_then(|l| l.iter().find(|e| e["id"] == id))
                .and_then(|e| e["entries"].as_u64())
                .unwrap_or_else(|| panic!("{path} is not {id} and names no {id} in its lineage"));
            let n = usize::try_from(entries).expect("a count");
            let mut prior = serde_json::json!({
                "format": doc["format"],
                "epoch": doc["epoch"],
                "entries": doc["entries"].as_array().expect("entries")[..n].to_vec(),
            });
            let lineage: Vec<Value> = doc["lineage"]
                .as_array()
                .expect("a lineage")
                .iter()
                .filter(|e| e["entries"].as_u64() < Some(entries))
                .cloned()
                .collect();
            if !lineage.is_empty() {
                prior["lineage"] = Value::Array(lineage);
            }
            prior
        }
        doc => doc.clone(),
    };
    let c = DeploymentCatalog::parse_bytes(&serde_json::to_vec(&doc).expect("bytes"))
        .unwrap_or_else(|e| panic!("catalog {id}: {e}"));
    assert_eq!(c.id().to_string(), id, "the catalog's id");
    c
}

#[test]
fn every_case_of_tags_json() {
    let text = std::fs::read_to_string(root().join("spec/conformance/envelope/tags.json"))
        .expect("tags.json");
    let doc: Value = serde_json::from_str(&text).expect("JSON");
    let history = doc["history"].as_str().expect("history");
    let iface_dir = root().join(history);
    let history_root = iface_dir.parent().expect("the history root");
    let (set, problems) = ContractSet::load_history(history_root);
    assert!(problems.is_empty(), "{problems:?}");
    let cases = doc["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 36, "the fixture's cases");
    let mut failures = Vec::new();
    for (i, case) in cases.iter().enumerate() {
        let note = case["note"].as_str().unwrap_or_default();
        let key = case["key"].as_str().expect("a key");
        let ch = Header::from_byte(
            u8::from_str_radix(
                case["ch"].as_str().expect("ch").trim_start_matches("0x"),
                16,
            )
            .expect("hex"),
        );
        let size = case["size"].as_u64().expect("size");
        let held_fps: Vec<&str> = case["held"]
            .as_array()
            .expect("held")
            .iter()
            .map(|f| f.as_str().expect("a fingerprint"))
            .collect();
        let catalogs = case["catalog"]
            .as_str()
            .map(|id| Catalogs::new(vec![catalog(id, &doc["catalogs"][id])]));
        let expect = &case["expect"];
        let shown = |shows: Unheld| match shows {
            Unheld::Packed(t) => serde_json::json!({"packed": {"tag": t.get(), "size": size}}),
            Unheld::Sniffed => Value::from("sniffed"),
            Unheld::Envelope => Value::from("envelope"),
        };
        // A reply's value: what the tool shows, the steps reading puts.
        if case["sample"] == "reply" {
            let got = serde_json::json!({"shows": shown(Unheld::of(ch.form, ch.tag))});
            if &got != expect {
                failures.push(format!("case {i} ({note}): expected {expect}, got {got}"));
            }
            continue;
        }
        let ZkKey::Data {
            iface,
            kind,
            resource,
            ..
        } = zenkey_model::grammar::parse(key).expect("a zk2 key")
        else {
            panic!("case {i}: a data key");
        };
        let heartbeat = zenkey_model::heartbeat::covered(key);
        let (iface, target) = match &heartbeat {
            Some((_, covered)) => (covered.clone(), Target::Heartbeat),
            None => (
                iface,
                Target::Data {
                    kind,
                    chunks: &resource,
                },
            ),
        };
        let held: Vec<Arc<Revision>> = set
            .of_iface(&iface)
            .filter(|r| held_fps.contains(&r.fingerprint().to_string().as_str()))
            .cloned()
            .collect();
        assert_eq!(held.len(), held_fps.len(), "case {i}: every revision held");
        let heard = Heard::Put {
            form: ch.form,
            tag: ch.tag,
        };
        let got = match choose(&held, target, catalogs.as_ref(), heard) {
            Choice::Chosen { revision, step } => {
                let mut got = serde_json::json!({
                    "revision": revision.fingerprint().to_string(),
                    "step": step.number(),
                    "by": if step == Step::Deployment { "deployment" } else { "tag" },
                });
                // The chosen revision's tag, as observer §7.3 checks it.
                if tag_of(&revision, target, heard) != Some(ch.tag) {
                    got["refused"] = Value::from("tag");
                }
                got
            }
            Choice::NoneLeft { step, .. } => serde_json::json!({
                "revision": null,
                "none": step.token(),
                "shows": shown(Unheld::of(ch.form, ch.tag)),
            }),
            Choice::Ambiguous(fps) => serde_json::json!({
                "revision": null,
                "ambiguous": fps.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "shows": shown(Unheld::of(ch.form, ch.tag)),
            }),
        };
        if &got != expect {
            failures.push(format!(
                "case {i} ({note}):\n  expected {expect}\n  got      {got}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A key's identity alone, and a delete, read no record: any revision
/// step 1 leaves reads the key, the catalog's latest where it orders them;
/// a snapshot's page is mapped by step 1 alone (observer §7.5).
#[test]
fn a_key_with_no_record_reads_with_any_revision_step_one_leaves() {
    let (set, problems) = ContractSet::load_history(&root().join("examples/zk2/.history"));
    assert!(problems.is_empty(), "{problems:?}");
    let iface: zenkey_model::grammar::IfaceId = "modem.v3".parse().unwrap();
    let held: Vec<Arc<Revision>> = set.of_iface(&iface).cloned().collect();
    assert_eq!(held.len(), 5);
    let chunks = vec!["oper".to_owned()];
    let target = Target::Data {
        kind: zenkey_model::grammar::KindToken::State,
        chunks: &chunks,
    };
    let modem = Catalogs::new(vec![
        Catalogs::load(&root().join("examples/zk2/zenoh-modem/catalog.json")).unwrap(),
    ]);
    match choose(&held, target, Some(&modem), Heard::Nothing) {
        Choice::Chosen {
            revision,
            step: Step::Any,
        } => assert_eq!(
            revision.fingerprint().to_string(),
            "sha256:04836e1785402629a03f27d3ff0f07158942460564facde5731f4467020467ca",
            "the catalog's latest, modem.v3 3.4"
        ),
        other => panic!("{other:?}"),
    }
    let page = Target::Collection {
        kind: zenkey_model::grammar::KindToken::State,
        coll: 0,
    };
    assert!(matches!(
        choose(&held, page, Some(&modem), Heard::Snapshot),
        Choice::Ambiguous(_)
    ));
    let latest: Vec<Arc<Revision>> = held
        .iter()
        .filter(|r| r.fingerprint().to_string().starts_with("sha256:0483"))
        .cloned()
        .collect();
    assert!(matches!(
        choose(&latest, page, Some(&modem), Heard::Snapshot),
        Choice::Chosen {
            step: Step::Deployment,
            ..
        }
    ));
}

/// Through the lens (observer §7.3, §7.5): a JSON state every revision of
/// modem.v3 declares alike reads with the catalog's latest, chosen by tag;
/// under a tag other than 0 none carries it, so it is a packed record,
/// never sniffed; with one revision held it is read with it and refused
/// `tag`; with none held, a packed record by its tag and size.
#[test]
fn the_lens_reads_a_record_with_the_revision_chtag_chooses() {
    use zenkey_fleet::Lens;
    use zenkey_fleet::model::render::Member;
    use zenkey_fleet::report::{Rendered, Unresolved};
    use zenkey_model::wire::{Envelope, Run, Seq, Stamp, Tag};

    let (set, problems) = ContractSet::load_history(&root().join("examples/zk2/.history"));
    assert!(problems.is_empty(), "{problems:?}");
    let modem = Catalogs::new(vec![
        Catalogs::load(&root().join("examples/zk2/zenoh-modem/catalog.json")).unwrap(),
    ]);
    let key = "zk2/h-3fa9c2d41b7e/rf0/modem.v3/state/oper";
    let stamp = Stamp {
        run: Run::U16(1),
        seq: Seq::new(1).unwrap(),
    };
    let put = |tag: u8| Envelope::mutation(Tag::new(tag).unwrap(), stamp, b"{}").write();
    let latest = "sha256:04836e1785402629a03f27d3ff0f07158942460564facde5731f4467020467ca";

    let lens = Lens::new("", None, &set).offline(&set).catalogs(&modem);
    let c = lens.check(key, Member::Type, Some("zenoh/bytes"), &put(0));
    let chosen = c.identity.chosen.expect("chosen by tag");
    assert_eq!(chosen.step, 3, "{c:?}");
    assert_eq!(
        c.rendering
            .resource
            .as_ref()
            .map(|r| r.fingerprint.as_str()),
        Some(latest)
    );
    assert!(
        matches!(c.rendering.rendered, Rendered::Value { .. }),
        "{c:?}"
    );

    let c = lens.check(key, Member::Type, Some("zenoh/bytes"), &put(5));
    assert!(
        matches!(&c.identity.unresolved, Some(Unresolved::NoneLeft { step, .. }) if step.token() == "tag"),
        "{c:?}"
    );
    assert!(
        matches!(&c.rendering.rendered, Rendered::Structural { text, value: None, .. }
            if text == "<packed record, tag 5, 2 bytes>"),
        "{c:?}"
    );

    let mut one = ContractSet::new();
    for r in set.iter() {
        if r.fingerprint().to_string() == latest {
            one.insert((**r).clone());
        }
    }
    let lens = Lens::new("", None, &one).offline(&one);
    let c = lens.check(key, Member::Type, Some("zenoh/bytes"), &put(5));
    assert!(c.identity.chosen.is_none(), "step 1, the deployment's word");
    assert!(
        matches!(&c.rendering.rendered, Rendered::Undecodable { reason, .. } if reason.contains("refused, tag")),
        "{c:?}"
    );

    let none = ContractSet::new();
    let lens = Lens::new("", None, &none).offline(&none);
    let packed = "zk2/h-3fa9c2d41b7e/rf0/modem.v3/state/packed/oper";
    let c = lens.check(packed, Member::Type, Some("zenoh/bytes"), &put(4));
    assert!(
        matches!(&c.rendering.rendered, Rendered::Structural { text, .. }
            if text == "<packed record, tag 4, 2 bytes>"),
        "{c:?}"
    );
    let c = lens.check(packed, Member::Type, Some("zenoh/bytes"), &put(0));
    assert!(
        matches!(
            &c.rendering.rendered,
            Rendered::Structural { value: Some(_), .. }
        ),
        "tag 0 is sniffed: {c:?}"
    );
}

/// Step 3 orders only by one index: two revisions at the same index, as
/// two unrelated catalogs given together can place them, are ambiguous,
/// never the last one seen (review-bot on #1088).
#[test]
fn two_revisions_at_one_index_are_ambiguous() {
    use zenkey_fleet::model::deployment::DeploymentCatalog;
    use zenkey_model::canonical::Fingerprint;
    let (set, problems) = ContractSet::load_history(&root().join("examples/zk2/.history"));
    assert!(problems.is_empty(), "{problems:?}");
    let iface: zenkey_model::grammar::IfaceId = "modem.v3".parse().unwrap();
    let pick = |prefix: &str| {
        set.of_iface(&iface)
            .find(|r| r.fingerprint().to_string().starts_with(prefix))
            .cloned()
            .expect("a revision")
    };
    let (x, y) = (pick("sha256:a584"), pick("sha256:71f5"));
    let at_three = |rev: &Revision, filler: char| {
        let mut c = DeploymentCatalog::new(1_767_225_600).unwrap();
        for n in 0..3 {
            let fp = Fingerprint::parse(&format!(
                "sha256:{}",
                filler.to_string().repeat(63) + &n.to_string()
            ))
            .unwrap();
            c.push(format!("filler{n}.v1").parse().unwrap(), fp)
                .unwrap();
        }
        c.push(rev.iface().clone(), rev.fingerprint().clone())
            .unwrap();
        c
    };
    let both = Catalogs::new(vec![at_three(&x, 'a'), at_three(&y, 'b')]);
    let chunks = vec!["oper".to_owned()];
    let target = Target::Data {
        kind: zenkey_model::grammar::KindToken::State,
        chunks: &chunks,
    };
    let heard = Heard::Put {
        form: zenkey_model::wire::Form::Mutation,
        tag: zenkey_model::wire::Tag::NONE,
    };
    match choose(&[x.clone(), y.clone()], target, Some(&both), heard) {
        Choice::Ambiguous(fps) => assert_eq!(fps.len(), 2, "{fps:?}"),
        other => panic!("two at index 3 chose one: {other:?}"),
    }
    // One catalog orders them: its latest.
    let one = Catalogs::new(vec![
        Catalogs::load(&root().join("examples/zk2/zenoh-modem/catalog.json")).unwrap(),
    ]);
    assert!(matches!(
        choose(&[x, y.clone()], target, Some(&one), heard),
        Choice::Chosen { step: Step::Latest, revision } if revision.fingerprint() == y.fingerprint()
    ));
}
