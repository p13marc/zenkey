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

/// Core §7.5, "Another form on a heartbeat's key": the lens refuses a
/// put of another form on a class key, and on a heartbeat's key, `form`,
/// before its tag or its body is read. A mutation and a frame under tag 0
/// whose bytes after the seq are an owner heartbeat's body are never read
/// as one, nor shown as a record of the key's raw type; a put with no
/// envelope is refused the same way. The parse's own refusal stands, and
/// the owner heartbeat itself is read. Rendering through one revision in
/// hand, `freshness.v2`'s, refuses alike.
#[test]
fn another_form_on_a_heartbeats_key_is_refused_form_by_the_lens() {
    use zenkey_model::heartbeat::Heartbeat;
    let set = contracts(&IFACES);
    let held = Catalogs::new(vec![catalog()]);
    let lens = Lens::new("", None, &set).offline(&set).catalogs(&held);
    // CH's form bits, the top three: 0 a mutation, 5 a frame.
    let as_form = |payload: &[u8], form: u8| -> Vec<u8> {
        let mut bytes = payload.to_vec();
        bytes[0] = (bytes[0] & 0x1f) | (form << 5);
        bytes
    };
    let owner_beat = put(vec![group(0, "alpha.v1", 1)]);
    assert_eq!(owner_beat[0], 0xc0, "form 6, a 16-bit run, tag 0");
    let alpha = Layout::of(revision(&set, "alpha.v1").contract());
    let beat = Heartbeat {
        time: Time::Unknown,
        sections: vec![None; alpha.collections.len()],
    }
    .envelope(&alpha, stamp())
    .unwrap();
    let class_key = owner_key("lab", "trio", Class::Host);
    let own_key = zenkey_model::heartbeat::key("lab", "trio", &alpha.iface);
    let standard = Revision::from_bundle(
        zenkey_model::bundle::Bundle::verify(zenkey::heartbeat::BUNDLE).unwrap(),
        zenkey_fleet::report::ContractSource::Bundle,
    )
    .unwrap();
    for (key, declared, whose, payload) in [
        (
            &class_key,
            "zk2c owner heartbeat (host)",
            "an owner heartbeat's",
            &owner_beat,
        ),
        (
            &own_key,
            "zk2c heartbeat (alpha.v1)",
            "a heartbeat's",
            &beat,
        ),
    ] {
        for (what, bytes, encoding) in [
            ("a mutation", as_form(payload, 0), "zenoh/bytes"),
            ("a frame", as_form(payload, 5), "zenoh/bytes"),
            (
                "a put with no envelope",
                payload.clone(),
                "application/octet-stream",
            ),
        ] {
            let want =
                format!("refused: form, {what} on {whose} key, which carries form 6 and no other");
            let c = lens.check(key, Member::Type, Some(encoding), &bytes);
            assert_eq!(
                c.rendering.rendered,
                Rendered::Undecodable {
                    declared: declared.to_owned(),
                    reason: want.clone()
                },
                "{key}: {what}"
            );
            assert_eq!(
                c.conformance,
                Conformance::Undecodable {
                    declared: declared.to_owned(),
                    reason: want.clone()
                },
                "{key}: {what}"
            );
            // Through one revision in hand, the standard contract's,
            // whose resource of the key is raw bytes.
            let r = zenkey_fleet::model::render::render_with(
                &standard,
                key,
                Member::Type,
                Some(encoding),
                &bytes,
            );
            assert_eq!(
                r.rendered,
                Rendered::Undecodable {
                    declared: declared.to_owned(),
                    reason: want
                },
                "{key}: {what}, one revision in hand"
            );
        }
        // The parse's refusal stands before `form`: a put cut short.
        let c = lens.check(key, Member::Type, Some("zenoh/bytes"), &payload[..2]);
        assert!(
            matches!(&c.conformance, Conformance::Undecodable { reason, .. } if reason == "zk2c envelope refused: short"),
            "{key}: {:?}",
            c.conformance
        );
        // Form 6 itself passes the gate on its key: the owner heartbeat
        // is read, and the heartbeat is its covered revision's to read,
        // never refused `form`.
        let c = lens.check(key, Member::Type, Some("zenoh/bytes"), payload);
        if key == &class_key {
            assert_eq!(c.conformance, Conformance::Valid, "{:?}", c.rendering);
        }
        assert!(
            !format!("{:?}", c.rendering.rendered).contains("refused: form"),
            "{key}: {:?}",
            c.rendering
        );
    }
    // Any other key is untouched: a mutation there is its resource's.
    assert_eq!(
        zenkey_fleet::model::inband::no_heartbeat(
            "zk2/lab/trio/alpha.v1/state/status",
            Some("zenoh/bytes"),
            &zenkey_fleet::model::inband::payload(
                "zk2/lab/trio/alpha.v1/state/status",
                Some("zenoh/bytes"),
                &as_form(&owner_beat, 0)
            )
        ),
        None
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

// --- `.zrec`: a capture that holds owner heartbeats (#1110's H2c) -------------

/// The capture zenctl's trycmd reads (`echo --from`, `timeline --from`,
/// `replay --dry-run`): four rows on `lab/trio`'s two class keys.
const ZREC: &str = "zenctl/tests/cmd/fixtures/owner-heartbeat.zrec";
/// The catalog its header names, as `--catalog` takes it.
const ZREC_CATALOG: &str = "zenctl/tests/cmd/fixtures/owner-heartbeat.catalog.json";

/// The capture's rows: the key's class, the put's seq and its bytes.
fn zrec_rows() -> Vec<(Class, u32, Vec<u8>)> {
    let at = |seq: u32, groups: Vec<Group>| {
        OwnerHeartbeat {
            time: Time::Unknown,
            groups,
        }
        .envelope(Stamp {
            run: Run::U16(7),
            seq: Seq::new(seq).unwrap(),
        })
        .unwrap()
    };
    // charlie.v1's `reach`, its `link` collection, on the `link` key.
    let all = contracts(&IFACES);
    let layout = Layout::of(revision(&all, "charlie.v1").contract());
    let reach: Vec<Option<Section>> = layout
        .collections
        .iter()
        .map(|c| {
            (c.resource() == "state/reach").then_some(Section {
                digest: Digest::from_u32(0x1c),
                members: Members::One { vouched: true },
            })
        })
        .collect();
    let mut refused = at(6, vec![group(0, "alpha.v1", 0x0a)]);
    refused[0] |= 0x05;
    vec![
        (
            Class::Host,
            3,
            at(
                3,
                vec![
                    group(0, "alpha.v1", 0x0a),
                    group(1, "bravo.v1", 0x0b),
                    group(2, "charlie.v1", 0x0c),
                ],
            ),
        ),
        (
            Class::Host,
            5,
            at(
                5,
                vec![group(0, "alpha.v1", 0x0a), group(900, "bravo.v1", 0x0b)],
            ),
        ),
        (
            Class::Link,
            0,
            at(0, vec![Group::of(2, &layout, &reach).unwrap()]),
        ),
        (Class::Host, 6, refused),
    ]
}

/// The capture's text: a version 4 header naming the catalog and the
/// revisions `lab/trio` runs, carrying no bundle, then the rows.
fn zrec_text() -> String {
    use base64::Engine as _;
    let all = contracts(&IFACES);
    let cat = catalog();
    let mut revisions: Vec<Value> = IFACES
        .iter()
        .map(|name| {
            json!({
                "address": "lab/trio",
                "iface": name,
                "fingerprint": revision(&all, name).fingerprint().to_string(),
                "named": cat.id().hex(),
            })
        })
        .collect();
    revisions.push(json!({
        "address": "lab/trio",
        "iface": "freshness.v2",
        "fingerprint": format!("sha256:{}", zenkey::heartbeat::FINGERPRINT),
        "named": cat.id().hex(),
    }));
    let header = json!({
        "zrec": 4,
        "selectors": ["zk2/lab/trio/**"],
        "base": "",
        "captured_at": "2026-11-21T00:00:00Z",
        "excluded": ["@stream", "@state", "@op", "@zk", "@adv"],
        "catalog": {"id": cat.id().hex(), "epoch": cat.epoch(), "lineage": []},
        "revisions": revisions,
    });
    let mut out = format!("{header}\n");
    for (i, (class, _, bytes)) in zrec_rows().iter().enumerate() {
        let row = json!({
            "key": owner_key("lab", "trio", *class),
            "t": 1_000_000 * (i as u64 + 1),
            "encoding": "zenoh/bytes",
            "qos_axes": "data/drop/reliable",
            "delete": false,
            "bytes": base64::engine::general_purpose::STANDARD.encode(bytes),
        });
        out.push_str(&format!("{row}\n"));
    }
    out
}

/// The fixture zenctl's snapshots read is what the model's writer writes:
/// regenerate it with `OWNER_HEARTBEAT_BLESS=1`, never by hand.
#[test]
fn the_zrec_fixture_is_what_the_writer_writes() {
    let (zrec, cat) = (root().join(ZREC), root().join(ZREC_CATALOG));
    let mut jcs = catalog().to_jcs();
    jcs.push(b'\n');
    if std::env::var_os("OWNER_HEARTBEAT_BLESS").is_some() {
        std::fs::write(&zrec, zrec_text()).unwrap();
        std::fs::write(&cat, &jcs).unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(&zrec).expect("the capture"),
        zrec_text()
    );
    assert_eq!(std::fs::read(&cat).expect("the catalog"), jcs);
}

/// A capture that holds owner heartbeats is read offline through the
/// lens, the header in presence's place (the tooling guide's §5). Its
/// header keeps the catalog's id, epoch and lineage, and no entry: given
/// that catalog (`--catalog`), each group is read with the revision its
/// index names; given none, each is shown by its envelope,
/// `unknown_catalog`. A put refused whole is undecodable either way.
#[test]
fn a_capture_of_owner_heartbeats_reads_through_the_headers_catalog() {
    use zenkey_fleet::model::recorded::Recorded;
    use zenkey_fleet::tape::record::{ZrecItem, ZrecReader, reader_catalogs};
    let text = zrec_text();
    let mut reader = ZrecReader::new(std::io::Cursor::new(text.into_bytes())).expect("version 4");
    let header = reader.header().clone();
    let mut rows = Vec::new();
    while let Some(item) = reader.next() {
        match item.expect("a row") {
            ZrecItem::Sample { row, .. } => rows.push(row),
            other => panic!("only samples: {other:?}"),
        }
    }
    assert_eq!(rows.len(), 4);
    let read = |given: Option<&Catalog>| -> Vec<Result<Value, String>> {
        let (catalogs, _) = reader_catalogs(&header, given).expect("the header's catalog");
        let recorded = Recorded::of(&header, contracts(&IFACES));
        let lens = Lens::new("", None, &recorded)
            .recorded(&recorded)
            .catalogs(&catalogs);
        rows.iter()
            .map(|row| {
                let c = lens.check(
                    &row.key,
                    Member::Type,
                    row.encoding.as_deref(),
                    &row.payload,
                );
                match c.rendering.rendered {
                    Rendered::Value { value, .. } => Ok(value),
                    Rendered::Undecodable { reason, .. } => Err(reason),
                    other => panic!("never sniffed: {other:?}"),
                }
            })
            .collect()
    };
    let cat = catalog();
    let given = read(Some(&cat));
    let host = given[0].as_ref().unwrap();
    assert_eq!((&host["class"], &host["seq"]), (&json!("host"), &json!(3)));
    assert!(
        groups(host)
            .iter()
            .all(|(_, covers, read)| covers.is_some() && read.is_ok()),
        "three groups read: {host}"
    );
    assert_eq!(
        groups(given[1].as_ref().unwrap())[1],
        (900, None, Err("unknown_index".into()))
    );
    let link = given[2].as_ref().unwrap();
    assert_eq!(link["class"], json!("link"));
    assert_eq!(
        groups(link),
        [(
            2,
            Some("charlie.v1".into()),
            Ok(json!({"state/reach": {"digest": "0000001c", "vouched": true}}))
        )]
    );
    assert_eq!(given[3], Err("the owner heartbeat is refused: tag".into()));
    // The header alone holds no entry to read an index by.
    let alone = read(None);
    for v in alone[..3].iter().map(|v| v.as_ref().unwrap()) {
        assert!(
            groups(v)
                .iter()
                .all(|(_, covers, read)| covers.is_none()
                    && read == &Err("unknown_catalog".to_owned())),
            "{v}"
        );
    }
    assert!(alone[3].is_err());
}
