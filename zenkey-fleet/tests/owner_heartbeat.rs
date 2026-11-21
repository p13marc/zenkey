//! A tool reads an owner heartbeat (core §7.5's second layout of form 6;
//! observer 0.34 §7.3, §7.5; #1110's H2b): its groups by index, tag and
//! size with no contract, and each group's sections with the revision its
//! index names in the catalog the tool was given, never one its key would
//! choose. Session-free: the lens offline, over the runtime's
//! `tests/contracts/trio/` and a catalog listing them at indices 0, 1, 2.
//! No owner sends one yet, so the bytes come from the model's writer.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use zenkey_fleet::model::deployment::Catalogs;
use zenkey_fleet::model::face_scope::{Face, FaceScope, Reads};
use zenkey_fleet::model::lens::Lens;
use zenkey_fleet::model::render::Member;
use zenkey_fleet::report::{Conformance, Rendered};
use zenkey_fleet::{ContractSet, Revision};
use zenkey_model::catalog::Catalog;
use zenkey_model::heartbeat::{
    Class, Digest, Group, Layout, Members, OwnerHeartbeat, Section, Time, owner_key,
};
use zenkey_model::wire::{Run, Seq, Stamp, Tag};

const IFACES: [&str; 3] = ["alpha.v1", "bravo.v1", "charlie.v1"];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// The contracts in hand: the three, or those named.
fn contracts(names: &[&str]) -> ContractSet {
    let mut set = ContractSet::new();
    for name in names {
        let path = root().join(format!("zenkey/tests/contracts/trio/{name}.toml"));
        let (loaded, problems) = ContractSet::load_path(&path);
        assert!(problems.is_empty(), "{name}: {problems:?}");
        set.extend(loaded);
    }
    set
}

fn revision(set: &ContractSet, name: &str) -> std::sync::Arc<Revision> {
    set.iter()
        .find(|r| r.iface().to_string() == name)
        .cloned()
        .unwrap_or_else(|| panic!("{name} in hand"))
}

/// The deployment's catalog: the three interfaces at indices 0, 1, 2.
fn catalog() -> Catalog {
    let all = contracts(&IFACES);
    let mut c = Catalog::new(0).unwrap();
    for name in IFACES {
        let r = revision(&all, name);
        c.push(r.iface().clone(), r.fingerprint().clone()).unwrap();
    }
    c
}

/// A group covering `name` at `index`: its `state/status`, one member
/// vouched for under `digest`, every other map bit 0.
fn group(index: u16, name: &str, digest: u32) -> Group {
    let all = contracts(&IFACES);
    let layout = Layout::of(revision(&all, name).contract());
    let sections: Vec<Option<Section>> = layout
        .collections
        .iter()
        .map(|c| {
            (c.resource() == "state/status").then_some(Section {
                digest: Digest::from_u32(digest),
                members: Members::One { vouched: true },
            })
        })
        .collect();
    Group::of(index, &layout, &sections).unwrap()
}

fn stamp() -> Stamp {
    Stamp {
        run: Run::U16(7),
        seq: Seq::new(3).unwrap(),
    }
}

fn put(groups: Vec<Group>) -> Vec<u8> {
    OwnerHeartbeat {
        time: Time::Unknown,
        groups,
    }
    .envelope(stamp())
    .unwrap()
}

/// The owner heartbeat's reading through `lens`: its value, or the reason
/// it is undecodable.
fn read(lens: &Lens<'_>, bytes: &[u8]) -> Result<Value, String> {
    let key = owner_key("lab", "trio", Class::Host);
    let c = lens.check(&key, Member::Type, Some("zenoh/bytes"), bytes);
    match c.rendering.rendered {
        Rendered::Value {
            declared, value, ..
        } => {
            assert_eq!(declared, "zk2c owner heartbeat (host)");
            assert_eq!(c.conformance, Conformance::Valid);
            Ok(value)
        }
        Rendered::Undecodable { declared, reason } => {
            assert_eq!(declared, "zk2c owner heartbeat (host)");
            Err(reason)
        }
        other => panic!("an owner heartbeat is never sniffed: {other:?}"),
    }
}

/// What a reading says of each group: its index, the interface its index
/// names where the catalog read it, and `sections` or why it is unread.
fn groups(v: &Value) -> Vec<(u64, Option<String>, Result<Value, String>)> {
    v["groups"]
        .as_array()
        .expect("groups")
        .iter()
        .map(|g| {
            (
                g["index"].as_u64().unwrap(),
                g["covers"].as_str().map(str::to_owned),
                match g["unread"].as_str() {
                    Some(why) => Err(why.to_owned()),
                    None => Ok(g["sections"].clone()),
                },
            )
        })
        .collect()
}

/// Given the catalog and the three revisions, one owner heartbeat reads as
/// three groups, each under the revision its index names: the class, the
/// key's run and seq, and each group's index, tag, size and sections.
#[test]
fn three_groups_are_read_by_their_indices() {
    let set = contracts(&IFACES);
    let held = Catalogs::new(vec![catalog()]);
    let lens = Lens::new("", None, &set).offline(&set).catalogs(&held);
    let all = [
        group(0, "alpha.v1", 0x0a),
        group(1, "bravo.v1", 0x0b),
        group(2, "charlie.v1", 0x0c),
    ];
    let v = read(&lens, &put(all.to_vec())).unwrap();
    assert_eq!(
        (&v["class"], &v["run"], &v["seq"], &v["time"]),
        (&json!("host"), &json!("0007"), &json!(3), &Value::Null)
    );
    let section = |d: &str| json!({"state/status": {"digest": d, "vouched": true}});
    assert_eq!(
        groups(&v),
        [
            (0, Some("alpha.v1".into()), Ok(section("0000000a"))),
            (1, Some("bravo.v1".into()), Ok(section("0000000b"))),
            (2, Some("charlie.v1".into()), Ok(section("0000000c"))),
        ]
    );
    for (g, shown) in all.iter().zip(v["groups"].as_array().unwrap()) {
        assert_eq!(shown["tag"], json!(g.tag.get()), "the group's own tag");
        assert_eq!(shown["size"], json!(g.size()), "its sections' bits");
        assert_eq!(
            shown["covered"].as_str().map(|s| s.len()),
            Some("sha256:".len() + 64),
            "the revision its index names"
        );
    }
}

/// A group the tool cannot read is shown by its index, tag and size, and
/// the put's other groups are read all the same (observer §7.5): an index
/// the catalog does not hold, a revision the tool does not hold, and a
/// tag that is not the named revision's.
#[test]
fn a_group_it_cannot_read_is_shown_by_its_envelope() {
    // bravo.v1 is in the catalog, and not in hand.
    let set = contracts(&["alpha.v1", "charlie.v1"]);
    let held = Catalogs::new(vec![catalog()]);
    let lens = Lens::new("", None, &set).offline(&set).catalogs(&held);
    // charlie.v1's group written under alpha.v1's index's neighbour's tag:
    // a tag that is not the revision's its index names.
    let mut wrong = group(2, "charlie.v1", 0x0c);
    wrong.tag = Tag::new((wrong.tag.get() + 1) % 16).unwrap();
    let v = read(
        &lens,
        &put(vec![
            group(0, "alpha.v1", 0x0a),
            group(1, "bravo.v1", 0x0b),
            wrong.clone(),
            group(900, "alpha.v1", 0x0d),
        ]),
    )
    .unwrap();
    let got = groups(&v);
    assert!(got[0].2.is_ok(), "the known group is read: {:?}", got[0]);
    assert_eq!(
        got[1..],
        [
            (1, Some("bravo.v1".into()), Err("not_held".into())),
            (2, Some("charlie.v1".into()), Err("tag".into())),
            (900, None, Err("unknown_index".into())),
        ]
    );
    let shown = &v["groups"][3];
    assert_eq!(
        (&shown["index"], &shown["size"], shown.get("sections")),
        (&json!(900), &json!(group(900, "alpha.v1", 0).size()), None),
        "by its envelope: index, tag and size"
    );
    assert_eq!(v["groups"][2]["tag"], json!(wrong.tag.get()));
}

/// Given no catalog a tool reads no group (core §8.6): each is shown by
/// its envelope, `unknown_catalog`, whatever revisions are in hand. A
/// tool needs no contract to show the groups.
#[test]
fn given_no_catalog_every_group_is_shown_by_its_envelope() {
    let set = contracts(&IFACES);
    let lens = Lens::new("", None, &set).offline(&set);
    let v = read(
        &lens,
        &put(vec![group(0, "alpha.v1", 1), group(2, "charlie.v1", 2)]),
    )
    .unwrap();
    assert_eq!(
        groups(&v),
        [
            (0, None, Err("unknown_catalog".into())),
            (2, None, Err("unknown_catalog".into())),
        ]
    );
    let none = ContractSet::new();
    let bare = Lens::new("", None, &none).offline(&none);
    let v = read(&bare, &put(vec![group(1, "bravo.v1", 1)])).unwrap();
    assert_eq!(groups(&v), [(1, None, Err("unknown_catalog".into()))]);
}

/// An owner heartbeat core §7.5 refuses whole is undecodable, its refusal
/// named, and never read as a heartbeat of some interface: the key says
/// which layout a put carries.
#[test]
fn a_refused_owner_heartbeat_is_undecodable() {
    let set = contracts(&IFACES);
    let held = Catalogs::new(vec![catalog()]);
    let lens = Lens::new("", None, &set).offline(&set).catalogs(&held);
    let mut bytes = put(vec![group(0, "alpha.v1", 1)]);
    // CH's tag is 0 for an owner heartbeat: any other is refused `tag`.
    bytes[0] |= 0x05;
    assert_eq!(
        read(&lens, &bytes),
        Err("the owner heartbeat is refused: tag".into())
    );
    let mut cut = put(vec![group(0, "alpha.v1", 1)]);
    cut.truncate(cut.len() - 2);
    assert_eq!(
        read(&lens, &cut),
        Err("the owner heartbeat is refused: short".into())
    );
}

/// The `host` class's key is in no face's scope (observer 0.34 §7.5,
/// `link.v2` §2.9), and the `link` class's is only where the plan lets it
/// cross; with no face policy a tool given a catalog has both, and one
/// given none has neither.
#[test]
fn the_host_key_is_in_no_faces_scope() {
    let (set, problems) = ContractSet::load_path(
        &root().join("spec/profiles/link/conformance/contracts/probe.v1.toml"),
    );
    assert!(problems.is_empty(), "{problems:?}");
    // health.v1 uses `freshness.v2` and its status is of the `host` class;
    // probe.v1's one collection is of the `link` class.
    let (health, problems) =
        ContractSet::load_path(&root().join("spec/profiles/health/health.v1.toml"));
    assert!(problems.is_empty(), "{problems:?}");
    let mut trio = health;
    trio.extend(set.clone());
    let across = ["lab/radio-a".parse().unwrap()];
    let owner = |scope: &FaceScope| -> Vec<String> {
        scope
            .selectors()
            .iter()
            .filter(|s| s.contains("/heartbeats/"))
            .cloned()
            .collect()
    };
    let host = owner_key("lab", "radio-a", Class::Host);
    let link = owner_key("lab", "radio-a", Class::Link);

    // No face: each class a collection in hand has.
    let open = FaceScope::of_given(&across, &trio, "", Reads::Samples, None, true).unwrap();
    assert_eq!(owner(&open), [link.clone(), host.clone()]);
    let none = FaceScope::of_given(&across, &trio, "", Reads::Samples, None, false).unwrap();
    assert!(owner(&none).is_empty(), "given no catalog, no such scope");

    // Across the RF medium's face: the `link` key alone, and the `host`
    // key given is refused, never dropped.
    let text = std::fs::read_to_string(root().join("examples/zk2/link/media/rf.face.toml"))
        .expect("the policy");
    let policy = zenkey_model::link::parse(&text).policy.expect("a policy");
    let face = Face {
        policy: &policy,
        name: "radio",
    };
    let faced = FaceScope::of_given(&across, &set, "", Reads::Samples, Some(face), true).unwrap();
    assert_eq!(owner(&faced), [link]);
    let narrowed = faced.narrow(std::slice::from_ref(&host)).unwrap();
    assert_eq!(narrowed.unmet, [host]);
    // A state reader's GET takes no heartbeat.
    let state = FaceScope::of_given(&across, &set, "", Reads::State, Some(face), true).unwrap();
    assert!(owner(&state).is_empty());
}
