//! The Inspector's value: decode provenance, superseded against never asked,
//! the plane it follows, numeric plotting (#64), and the engine's
//! projections consumed (#234).

use super::*;

/// The detail pane (§6.4 item 5 + #66): the decoded side is tagged with HOW
/// it was produced — contract-decoded and sniffed must never look alike —
/// and the empty fetch renders the non-verdict sentence. Since #614 (FL6a)
/// the decode is the lens's: a zk2 key resolved to its resource decodes as
/// its declared type, and a key the lens stops short of says at which rung.
#[test]
fn the_detail_pane_tags_decode_provenance() {
    use std::sync::Arc;
    use zengui::view::detail::{DetailData, Fetched, section};
    use zenkey_fleet_v1::{FetchOutcome, FetchedValue, ValueSource};

    let key = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used";
    let world = common::World::new();
    // The key's identity, as the window's identity cache holds it (#614,
    // FL7): a v1 key is foreign since the port.
    let identity: &'static zenkey_fleet::Resolution = Box::leak(Box::new(world.resolve(key)));

    let fetched: Result<Arc<FetchOutcome>, zengui::services::ServiceError> =
        Ok(Arc::new(FetchOutcome::Value(FetchedValue {
            key: key.to_string(),
            payload: zenoh::bytes::ZBytes::from(br#"{"value":42.0}"#.to_vec()),
            encoding: "application/json".into(),
            timestamp: None,
            attachment: None,
            source: ValueSource::Storage,
        })));
    let render = |decoded: &'static zengui::value::DecodedValue| {
        simulator::<Message, _, _>(section(DetailData {
            slot: zengui::message::SlotId::FOLLOW,
            sp: sp(),
            key,
            identity: Some(identity),
            config: None,
            fetched: Fetched::Landed(Box::leak(Box::new(fetched.clone()))),
            decoded: Some(decoded),
            series: None,
            history_entries: None,
            observed: None,
            latency: None,
        }))
    };
    let decode = |key: &str, body: &[u8], encoding: &str| -> &'static zengui::value::DecodedValue {
        Box::leak(Box::new(zengui::value::DecodedValue::new(
            world.check(key, body, encoding),
            body,
        )))
    };

    // A v1 key is foreign to the lens: structural, with the rung it
    // stopped at — never dressed as a decode.
    let mut ui = render(decode(key, br#"{"value":42.0}"#, "application/json"));
    assert!(
        ui.find("not zk2").is_ok(),
        "the identity section renders: a v1 key is foreign since the port"
    );
    assert!(
        ui.find("contract-decoded <json:NetworkInterface>").is_err(),
        "a sniff never wears a decode's tag"
    );
    let sniffed = world.check(key, br#"{"value":42.0}"#, "application/json");
    let zenkey_fleet::report::Rendered::Structural { why, .. } = &sniffed.rendering.rendered else {
        panic!("a v1 key renders structurally")
    };
    assert!(
        ui.find(format!("structural — {}", why.words())).is_ok(),
        "where the ladder stopped is said"
    );

    // A zk2 key the lens resolves decodes as its declared type, and says
    // which resource of which revision it resolved to.
    let eth0 = br#"{"name":"eth0","index":2,"namespace":"default","is_up":true}"#;
    let mut ui = render(decode(common::ETH0, eth0, "application/json"));
    assert!(
        ui.find("contract-decoded <json:NetworkInterface>").is_ok(),
        "contract-decoded says so"
    );
    assert!(ui.find("valid").is_ok());

    // Bytes that are not their declared type: the undecodable pole, the
    // sniff beside it tagged as what it is.
    let mut ui = render(decode(common::NAV_ORIGIN, &[0x0a, 0xff, 0xff, 0xff], ""));
    assert!(
        ui.find("<nav.v2.Origin?> structural (the bytes did not decode as their declared type)")
            .is_ok(),
        "typed-but-undecoded must say so"
    );
    let mut ui = render(decode(key, br#"{"value":42.0}"#, "application/json"));
    assert!(ui.find("hex").is_ok(), "the hex side is present");
    // #563: the value's facts as chips — the ladder's rung in words, never
    // the enum's `Debug` name.
    assert!(ui.find("via storage").is_ok());
    assert!(ui.find("application/json").is_ok());
    assert!(
        ui.find("value: 14 bytes via Storage · encoding application/json")
            .is_err(),
        "the Debug-named line is gone"
    );

    // The attributed nothing.
    let none: Result<Arc<FetchOutcome>, zengui::services::ServiceError> =
        Ok(Arc::new(FetchOutcome::None {
            attempted: ["get", "@adv cache", "subscribe window"],
        }));
    let mut ui = simulator::<Message, _, _>(section(DetailData {
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
        key,
        identity: Some(identity),
        config: None,
        fetched: Fetched::Landed(&none),
        decoded: None,
        series: None,
        history_entries: None,
        observed: None,
        latency: None,
    }));
    assert!(
        ui.find("no value — asked get, @adv cache, subscribe window — a non-verdict, not proof of absence (RFC 05 §3.1)")
            .is_ok(),
        "silence stays attributed"
    );
}

/// The three fetch states are three sentences, and "superseded" is the one
/// that did not exist (#181).
///
/// It used to be folded into "not asked" by an `Option` in the view: a reply
/// that landed for a key the user had moved past was filtered out, and the
/// pane then told the user nothing had been asked about a key it had in fact
/// answered. That is an O4 violation in the one pane whose job is saying what
/// was and was not asked.
#[test]
fn the_detail_pane_distinguishes_superseded_from_never_asked() {
    use zengui::view::detail::{DetailData, Fetched, section};

    let key = "v1/h-3fa9c2d41b7e/state/sysinfo/health";
    let data = |fetched| DetailData {
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
        key,
        identity: None,
        config: None,
        fetched,
        decoded: None,
        series: None,
        history_entries: None,
        observed: None,
        latency: None,
    };

    const NOT_ASKED: &str = "no value fetched — selecting a concrete key \
                             fetches once (storage → cache → window), nothing \
                             ambient";
    const SUPERSEDED: &str = "superseded — the last fetch answered a different \
                              subject, so nothing here describes this key. \
                              Re-select it to ask again.";

    let mut ui = simulator::<Message, _, _>(section(data(Fetched::NotAsked)));
    assert!(
        ui.find(NOT_ASKED).is_ok(),
        "nothing was asked, and the pane says so"
    );
    assert!(ui.find(SUPERSEDED).is_err());

    let mut ui = simulator::<Message, _, _>(section(data(Fetched::Superseded)));
    assert!(
        ui.find(SUPERSEDED).is_ok(),
        "an answer that is real but about something else is its own state"
    );
    assert!(
        ui.find(NOT_ASKED).is_err(),
        "and must not be reported as an unasked question"
    );
}

/// The Inspector dispatches on the subject, and the plane sections come from
/// the classification seam (#182).
///
/// The load-bearing assertions are the *negative* ones. A `@blob` key must not
/// grow a media viewer and an ordinary state key must not grow either, because
/// the alternative — deciding from `key.contains("@blob")` — is the
/// string-shaped classifier this crate's key-agnostic-core claim forbids.
#[test]
fn the_inspector_follows_the_subject_and_its_plane() {
    use zengui::blob::BlobState;
    use zengui::message::Subject;
    use zengui::nodes::NodesState;
    use zengui::view::inspector::pane;
    use zengui::view::media::MediaState;

    let slices = slices();
    let blob = BlobState::default();
    let media = MediaState::default();
    let nodes = NodesState::default();

    let facts_for = |key: &str| {
        let mut f = KeyFacts::project("", key);
        f.resolve(&slices);
        f
    };

    // Nothing selected: the Inspector holds nothing of its own and says so.
    let none = Subject::None;
    let mut ui = simulator::<Message, _, _>(pane(projection_inspector(
        &none, None, &slices, &blob, &media, &nodes,
    )));
    assert!(ui.find("Nothing selected").is_ok());
    assert!(ui.find("Detail").is_err(), "no subject, no sections");

    // A prefix is a subtree, and the pane says that rather than reporting
    // "no value fetched" about a key that was never a key (#85).
    let prefix = Subject::Prefix("zk2/robot1/nav".into());
    let mut ui = simulator::<Message, _, _>(pane(projection_inspector(
        &prefix, None, &slices, &blob, &media, &nodes,
    )));
    assert!(ui.find("A subtree, not a key").is_ok());
    assert!(ui.find("History").is_err(), "a subtree records nothing");

    // An ordinary state key: detail and history, and neither plane section.
    let state_key = "v1/h-3fa9c2d41b7e/state/sysinfo/health";
    let state_facts = facts_for(state_key);
    let key = Subject::Key(state_key.into());
    let mut ui = simulator::<Message, _, _>(pane(projection_inspector(
        &key,
        Some(&state_facts),
        &slices,
        &blob,
        &media,
        &nodes,
    )));
    assert!(ui.find("Detail").is_ok());
    assert!(
        ui.find("History").is_ok(),
        "the timeline is a section now, not another tab"
    );
    assert!(ui.find("Blobs").is_err());
    assert!(ui.find("Media").is_err());

    // A `@blob` key grows the blob sections and only those.
    let blob_key = "v1/h-3fa9c2d41b7e/@blob/artifact/abc123";
    let blob_facts = facts_for(blob_key);
    let key = Subject::Key(blob_key.into());
    let mut ui = simulator::<Message, _, _>(pane(projection_inspector(
        &key,
        Some(&blob_facts),
        &slices,
        &blob,
        &media,
        &nodes,
    )));
    assert!(ui.find("Blobs").is_ok(), "the plane is read off ClassKind");
    assert!(
        ui.find("Media").is_err(),
        "one plane, not every plane the key is not on"
    );

    // An address subject (#614, FL9): presence and health, not a value —
    // and with no presence read, "not read", never "nobody there".
    let address = Subject::Address("robot1/nav".parse().expect("an address"));
    let mut ui = simulator::<Message, _, _>(pane(projection_inspector(
        &address, None, &slices, &blob, &media, &nodes,
    )));
    assert!(ui.find("robot1/nav").is_ok(), "the subject, restated");
    assert!(ui.find("presence not read").is_ok());
    assert!(
        ui.find("health not asked for this address — it is read on selection, never polled")
            .is_ok()
    );
    assert!(
        ui.find("Detail").is_err(),
        "an address has no value, no decode and no chart"
    );
}

/// An address subject in the Inspector (#614, FL9): what presence holds of
/// it, its instance as its descriptor describes it, and its health once
/// read — the nodes pane's sections, one wording.
#[test]
fn the_inspector_shows_an_address_from_presence_and_its_descriptor() {
    use zengui::message::Subject;
    use zengui::view::inspector::pane;

    let world = common::World::new();
    let update = world.presence_update();
    let mut nodes = zengui::nodes::NodesState::default();
    nodes.describe(Some(&world.catalog()));
    let slices = slices();
    let blob = zengui::blob::BlobState::default();
    let media = zengui::view::media::MediaState::default();
    let subject = Subject::Address("robot1/nav".parse().expect("an address"));
    let mut data = projection_inspector(&subject, None, &slices, &blob, &media, &nodes);
    data.presence = Some(&update);
    let mut ui = simulator::<Message, _, _>(pane(data));
    assert!(ui.find("robot1/nav: up").is_ok());
    assert!(ui.find("0000000000000001").is_ok(), "its instance, by id");
    assert!(
        ui.find("instance token held since just now").is_ok(),
        "since when, as the projection learned it"
    );
    assert!(
        ui.find("host — · process — · build — · zid —").is_ok(),
        "a descriptor naming no meta says so field by field"
    );
}

// ── Numeric plotting (#64) ───────────────────────────────────────────────

/// A payload with no numbers in it offers no chart at all — not an empty one,
/// and not an error. Nothing about a string payload is a problem.
#[test]
fn the_detail_pane_offers_no_chart_for_a_non_numeric_payload() {
    use zengui::series::{NumericLeaves, RateSampler, Series};
    use zengui::view::detail::{DetailData, Fetched, SeriesData, section};

    let series = SeriesData {
        leaves: NumericLeaves::default(),
        leaf: None,
        value: Series::new(),
        rate: RateSampler::new().series().clone(),
        unit: None,
        caches: Default::default(),
    };
    let mut ui = simulator::<Message, _, _>(section(DetailData {
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
        key: FOREIGN,
        identity: None,
        config: None,
        fetched: Fetched::NotAsked,
        decoded: None,
        series: Some(&series),
        history_entries: Some(3),
        observed: None,
        latency: None,
    }));
    assert!(
        ui.find("Series").is_err(),
        "a payload with nothing numeric in it must not grow a chart section"
    );
    // …but the recorded count is still there: history is not numeric. It used
    // to read "— open (Alt 8)", because the timeline was another tab; since
    // #182 it is the next section down, so the label is a count and not a
    // link.
    assert!(ui.find("history: 3 samples recorded").is_ok());
}

/// The chart's numbers live in its caption, which is what makes the plot
/// readable without colour — and gaps are named rather than smoothed away.
#[test]
fn the_detail_pane_labels_the_series_it_plots() {
    use zengui::series::{Series, numeric_leaves};
    use zengui::view::detail::{DetailData, Fetched, SeriesData, section};

    let mut value = Series::new();
    value.push(41.0);
    value.push_gap();
    value.push(42.0);
    let mut rate = Series::new();
    rate.push(5.0);

    let series = SeriesData {
        leaves: numeric_leaves(&serde_json::json!({"value": 42.0})),
        leaf: Some("value".to_string()),
        value,
        rate,
        unit: Some("percent".to_string()),
        caches: Default::default(),
    };
    let mut ui = simulator::<Message, _, _>(section(DetailData {
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
        key: REGISTERED,
        identity: None,
        config: None,
        fetched: Fetched::NotAsked,
        decoded: None,
        series: Some(&series),
        history_entries: Some(3),
        observed: None,
        latency: None,
    }));
    assert!(ui.find("Series").is_ok());
    assert!(
        ui.find(
            "value: 42percent now · 41percent min · 42percent max · 3 points · \
             1 drawn as gaps, not interpolated"
        )
        .is_ok(),
        "the caption carries the numbers, the unit and the gap count"
    );
    assert!(
        ui.find("rate: 5.0/s now · 5.0/s min · 5.0/s max · 1 point")
            .is_ok(),
        "the rate series uses the shared rate wording"
    );
    assert!(
        ui.find(
            "plotted from the recorded history's structural values — a schema decode \
             per sample would hit the bus on a render path, so a protobuf or CDR leaf \
             offers no chart"
        )
        .is_ok(),
        "the boundary of what can be plotted is stated, not left to be discovered"
    );
}

// ── The engine's projections, consumed (#234) ────────────────────────────

/// A two-subject slice plus a ledger entry — v1's declared world, which the
/// type section still reads until FL8 moves it.
fn projection_fixture() -> SliceSet {
    use zenkey_v1::slice::{DeprecationDecl, RegistrySlice, SubjectDecl};

    let subject = |path: &str| {
        let mut d = SubjectDecl::new(path, zenkey_v1::Class::State);
        d.type_name = "Health".into();
        d
    };
    let mut retired = DeprecationDecl::new("old/health");
    retired.since = Some("0.9".into());
    retired.replaced_by = Some("health".into());
    let mut slice = RegistrySlice::new("1.0", "demo", "sysinfo");
    slice.subjects = vec![subject("health"), subject("mode")];
    slice.deprecated = vec![retired];
    SliceSet::from_slices(vec![slice])
}

fn projection_inspector<'a>(
    subject: &'a zengui::message::Subject,
    facts: Option<&'a KeyFacts>,
    slices: &'a SliceSet,
    blob: &'a zengui::blob::BlobState,
    media: &'a zengui::view::media::MediaState,
    nodes: &'a zengui::nodes::NodesState,
) -> zengui::view::inspector::InspectorData<'a> {
    zengui::view::inspector::InspectorData {
        slot: zengui::message::SlotId::FOLLOW,
        subject,
        identity: None,
        v1: facts.cloned(),
        fetched: zengui::view::detail::Fetched::NotAsked,
        decoded: None,
        series: None,
        history: None,
        history_scroll: Default::default(),
        watched: false,
        snapshot: None,
        compare_snapshot: false,
        latency: None,
        blob,
        media,
        slices: Some(slices),
        presence: None,
        nodes,
        fields: Box::leak(Box::default()),
        why: Box::leak(Box::default()),
        consumers: Box::leak(Box::default()),
        sp: sp(),
    }
}

/// The type-subject arm (#234): a registered key's payload type is placed in
/// the registry vocabulary (`interface_list`) and its carriers listed
/// (`interface_show`) — who else speaks this type, without a slice scan of
/// the pane's own.
#[test]
fn the_inspector_places_a_registered_type_in_the_vocabulary() {
    use zengui::message::Subject;

    let slices = projection_fixture();
    let blob = zengui::blob::BlobState::default();
    let media = zengui::view::media::MediaState::default();
    let nodes = zengui::nodes::NodesState::default();

    let key = "v1/h-3fa9c2d41b7e/state/sysinfo/health";
    let mut facts = KeyFacts::project("", key);
    facts.resolve(&slices);
    let subject = Subject::Key(key.into());

    let mut ui = simulator::<Message, _, _>(zengui::view::inspector::pane(projection_inspector(
        &subject,
        Some(&facts),
        &slices,
        &blob,
        &media,
        &nodes,
    )));
    assert!(ui.find("Type").is_ok());
    assert!(
        ui.find("Health — one of 1 declared payload type(s), carried by 2 declaration(s)")
            .is_ok(),
        "the vocabulary placement is the engine's interface_list"
    );
    assert!(
        ui.find("sysinfo · state mode").is_ok(),
        "interface_show names the other carrier of the same type"
    );
}

/// #164, on zk2's four poles (#614, FL6a): the decoded pane renders the
/// conformance of a real check through the lens — valid, invalid with each
/// violation a sentence, undecodable with its decode error, and not checked
/// with the rung that stopped it — never a boolean, and never one pole in
/// another's words. The two silences keep theirs (#246; RFC 09 §5.1 O4):
/// "no provider of this address in presence" (asked, nobody serves it) is
/// not "not a zk2 data key" (nothing to ask).
#[test]
fn the_decoded_pane_keeps_the_four_conformance_poles_apart() {
    use std::sync::Arc;
    use zengui::view::detail::{DetailData, Fetched, section};
    use zenkey_fleet_v1::{FetchOutcome, FetchedValue, ValueSource};

    let world = common::World::new();
    let render = |key: &'static str, body: &'static [u8], encoding: &str| {
        let fetched: &'static Result<Arc<FetchOutcome>, zengui::services::ServiceError> =
            Box::leak(Box::new(Ok(Arc::new(FetchOutcome::Value(FetchedValue {
                key: key.to_string(),
                payload: zenoh::bytes::ZBytes::from(body.to_vec()),
                encoding: encoding.into(),
                timestamp: None,
                attachment: None,
                source: ValueSource::Storage,
            })))));
        // Leaked so the simulator may outlive the block — a test-only cost,
        // bounded by the calls below.
        let decoded: &'static zengui::value::DecodedValue = Box::leak(Box::new(
            zengui::value::DecodedValue::new(world.check(key, body, encoding), body),
        ));
        (
            decoded,
            simulator::<Message, _, _>(section(DetailData {
                slot: zengui::message::SlotId::FOLLOW,
                key,
                identity: None,
                config: None,
                fetched: Fetched::Landed(fetched),
                decoded: Some(decoded),
                series: None,
                history_entries: None,
                observed: None,
                latency: None,
                sp: sp(),
            })),
        )
    };

    let (_, mut ui) = render(
        common::ETH0,
        br#"{"name":"eth0","index":2,"namespace":"default","is_up":true}"#,
        "application/json",
    );
    assert!(ui.find("valid").is_ok(), "checked-and-passed says so");

    let (decoded, mut ui) = render(
        common::WLAN0,
        br#"{"name":"wlan0","index":-1,"namespace":"default","is_up":"yes"}"#,
        "application/json",
    );
    let zenkey_fleet::report::Conformance::Invalid { violations } = &decoded.checked.conformance
    else {
        panic!("the schema refuses wlan0's value")
    };
    assert!(
        ui.find(format!("invalid ({} violations)", violations.len()))
            .is_ok()
    );
    for v in violations {
        assert!(
            ui.find(format!("violation: {v}")).is_ok(),
            "each violation is a sentence with its path"
        );
    }
    assert!(ui.find("valid").is_err(), "invalid never reads as valid");

    let (decoded, mut ui) = render(common::NAV_ORIGIN, &[0x0a, 0xff, 0xff, 0xff], "");
    let zenkey_fleet::report::Conformance::Undecodable { reason, .. } =
        &decoded.checked.conformance
    else {
        panic!("those bytes are no nav.v2.Origin")
    };
    assert!(ui.find("undecodable (as nav.v2.Origin)").is_ok());
    assert!(ui.find(format!("decode error: {reason}")).is_ok());

    // The two silences: distinct badge words, each the engine's own.
    let (_, mut ui) = render("zk2/robot1/ghost/nav.v2/stream/position", &[0x08, 0x01], "");
    assert!(
        ui.find("not checked (no provider of this address in presence)")
            .is_ok(),
        "no provider is 'asked, and nobody serves it', spelled out"
    );
    let (decoded, mut ui) = render(
        "v1/h-3fa9c2d41b7e/state/sysinfo/health",
        br#"{"status":"ok"}"#,
        "application/json",
    );
    let label = zengui::verdict::label(&decoded.checked.conformance);
    assert!(
        label.starts_with("not checked (not a zk2 data key"),
        "{label}"
    );
    assert!(ui.find(label.as_str()).is_ok());
    assert!(
        ui.find("not checked (no provider of this address in presence)")
            .is_err(),
        "the two silences must not merge"
    );
}

/// #614 (FL7): the Inspector's facts line for a zk2 key is its identity,
/// not "not a keyspace-v2 key": the rung, the address, interface, kind and
/// resource it resolved to, the template's values, its type and revision.
/// A key the ladder stopped short of says where and why, and a foreign key
/// that it is no zk2 key — a fact, not an error (O1).
#[test]
fn the_detail_pane_states_a_keys_identity() {
    use zengui::view::detail::{DetailData, Fetched, section};

    let world = common::World::new();
    let draw = |key: &'static str| {
        let identity: &'static zenkey_fleet::Resolution = Box::leak(Box::new(world.resolve(key)));
        simulator::<Message, _, _>(section(DetailData {
            slot: zengui::message::SlotId::FOLLOW,
            sp: sp(),
            key,
            identity: Some(identity),
            config: None,
            fetched: Fetched::NotAsked,
            decoded: None,
            series: None,
            history_entries: None,
            observed: None,
            latency: None,
        }))
    };

    let mut ui = draw(common::ETH0);
    for said in [
        "resolved",
        "ADDRESS",
        "host-a/tc",
        "IFACE",
        "tc.netif.v1",
        "KIND",
        "state",
        "RESOURCE",
        "state/interfaces/{ns}/{iface}",
        "TYPE",
        "REVISION",
        "iface = eth0 · ns = default",
    ] {
        assert!(ui.find(said).is_ok(), "{said:?} must be on screen");
    }

    let mut ui = draw("zk2/robot9/nav/nav.v2/stream/position");
    assert!(ui.find("no provider").is_ok());
    assert!(ui.find("no provider of this address in presence").is_ok());
    assert!(ui.find("robot9/nav").is_ok(), "the address it names");

    let mut ui = draw(FOREIGN);
    assert!(ui.find("not zk2").is_ok());
    let why = world
        .resolve(FOREIGN)
        .identity
        .unresolved
        .expect("a foreign key stops at a rung")
        .words();
    assert!(why.starts_with("not a zk2 data key"), "{why}");
    assert!(
        ui.find(format!("{why} — a fact, not an error (O1)"))
            .is_ok(),
        "the rung's own words, and that it is no error"
    );
}
