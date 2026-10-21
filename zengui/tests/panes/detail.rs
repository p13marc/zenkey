//! The Inspector's value: the zk2 read and its source (#614, FL8a), decode
//! provenance, superseded against never asked, declared against observed
//! QoS, the subject it follows, numeric plotting (#64), and the declared
//! type (#234).

use super::*;

use std::sync::Arc;
use zengui::value::{ReadKind, Replies, Reply, ValueRead};
use zengui::view::detail::{DetailData, Fetched, section};

/// A Detail section's inputs with nothing asked, nothing observed and no
/// replay — each test sets what it pins.
fn data<'a>(key: &'a str) -> DetailData<'a> {
    DetailData {
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
        key,
        identity: None,
        fetched: Fetched::NotAsked,
        decoded: None,
        series: None,
        history_entries: None,
        observed: None,
        latency: None,
        replaying: false,
        archives: None,
        last_known: None,
    }
}

/// A raw GET's answer: one reply carrying `body`, unstamped.
fn raw(key: &str, body: &[u8], encoding: &str) -> ValueRead {
    ValueRead::Replies(Replies {
        kind: ReadKind::Raw,
        selector: key.to_string(),
        replies: vec![Reply {
            key: key.to_string(),
            payload: zenoh::bytes::ZBytes::from(body.to_vec()),
            encoding: encoding.to_string(),
            attachment: None,
            delete: false,
            stamp: None,
            provenance: None,
        }],
        errors: 0,
        elided: 0,
        retention: None,
    })
}

/// `x`, leaked so a simulator may outlive the block — a test-only cost,
/// bounded by the calls in each test.
fn leak<T>(x: T) -> &'static T {
    Box::leak(Box::new(x))
}

/// The detail pane (§6.4 item 5 + #66): the decoded side is tagged with HOW
/// it was produced — contract-decoded and sniffed must never look alike.
/// Since #614 (FL6a) the decode is the lens's: a zk2 key resolved to its
/// resource decodes as its declared type, and a key the lens stops short of
/// says at which rung.
#[test]
fn the_detail_pane_tags_decode_provenance() {
    let key = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used";
    let world = common::World::new();
    // The key's identity, as the window's identity cache holds it (#614,
    // FL7): a v1 key is foreign since the port.
    let identity = leak(world.resolve(key));
    let render =
        |key: &'static str, read: ValueRead, decoded: &'static zengui::value::DecodedValue| {
            let fetched = leak(Ok::<_, zengui::services::ServiceError>(Arc::new(read)));
            simulator::<Message, _, _>(section(DetailData {
                identity: Some(identity),
                fetched: Fetched::Landed(fetched),
                decoded: Some(decoded),
                ..data(key)
            }))
        };
    let decode = |key: &str, body: &[u8], encoding: &str| -> &'static zengui::value::DecodedValue {
        leak(zengui::value::DecodedValue::new(
            world.check(key, body, encoding),
            body,
        ))
    };

    // A v1 key is foreign to the lens: a raw GET's answer renders
    // structurally, with the rung it stopped at — never dressed as a decode.
    let body = br#"{"value":42.0}"#;
    let mut ui = render(
        key,
        raw(key, body, "application/json"),
        decode(key, body, "application/json"),
    );
    assert!(
        ui.find("not zk2").is_ok(),
        "the identity section renders: a v1 key is foreign since the port"
    );
    assert!(
        ui.find("contract-decoded <json:NetworkInterface>").is_err(),
        "a sniff never wears a decode's tag"
    );
    let sniffed = world.check(key, body, "application/json");
    let zenkey_fleet::report::Rendered::Structural { why, .. } = &sniffed.rendering.rendered else {
        panic!("a v1 key renders structurally")
    };
    assert!(
        ui.find(format!("structural — {}", why.words())).is_ok(),
        "where the ladder stopped is said"
    );
    assert!(ui.find("hex").is_ok(), "the hex side is present");
    // #563: the value's facts as chips — the read in words, never the
    // enum's `Debug` name.
    assert!(ui.find("via raw GET").is_ok());
    assert!(ui.find("application/json").is_ok());
    assert!(
        ui.find("unstamped").is_ok(),
        "no stamp rode it, and it says so"
    );

    // A zk2 key the lens resolves decodes as its declared type, and says
    // which resource of which revision it resolved to.
    let eth0 = br#"{"name":"eth0","index":2,"namespace":"default","is_up":true}"#;
    let mut ui = render(
        common::ETH0,
        raw(common::ETH0, eth0, "application/json"),
        decode(common::ETH0, eth0, "application/json"),
    );
    assert!(
        ui.find("contract-decoded <json:NetworkInterface>").is_ok(),
        "contract-decoded says so"
    );
    assert!(ui.find("valid").is_ok());

    // Bytes that are not their declared type: the undecodable pole, the
    // sniff beside it tagged as what it is.
    let bad = [0x0a, 0xff, 0xff, 0xff];
    let mut ui = render(
        common::NAV_ORIGIN,
        raw(common::NAV_ORIGIN, &bad, ""),
        decode(common::NAV_ORIGIN, &bad, ""),
    );
    assert!(
        ui.find("<nav.v2.Origin?> structural (the bytes did not decode as their declared type)")
            .is_ok(),
        "typed-but-undecoded must say so"
    );
}

/// #614 (FL8a): the value names the zk2 read that produced it — S4's owner
/// GET, its silence, an archive's last-known answer beside it and never as
/// it, a raw GET — and none of them is ever "no value".
#[test]
fn the_value_names_the_zk2_read_that_produced_it() {
    use zengui::view::detail::LastKnown;
    let world = common::World::new();
    let key = common::ETH0;
    let identity = leak(world.resolve(key));
    let body = br#"{"name":"eth0","index":2,"namespace":"default","is_up":true}"#;
    let archive: zenkey_model::grammar::Addr = "ground/archive".parse().expect("an address");

    // The owner answered: decoded through its contract, its stamp's clock
    // named, and its bytes in hex beside the decode — kept on the row
    // since #765.
    let answer = world.state_read(key, Some(body), None);
    let decoded = answer.decoded.clone().expect("a value row is checked");
    let fetched = leak(Ok::<_, zengui::services::ServiceError>(Arc::new(
        ValueRead::Owner(answer),
    )));
    let mut ui = simulator::<Message, _, _>(section(DetailData {
        identity: Some(identity),
        fetched: Fetched::Landed(fetched),
        decoded: Some(leak(decoded).as_ref()),
        archives: Some(vec![archive.clone()]),
        ..data(key)
    }));
    assert!(ui.find("via owner GET (S4)").is_ok());
    assert!(ui.find("contract-decoded <json:NetworkInterface>").is_ok());
    assert!(
        ui.find("stamp unattributable (O7)").is_ok(),
        "no descriptor of this world states meta.zid: unattributable, never foreign"
    );
    assert!(
        ui.find("stamp 2026-10-05T09:12:44.250000000Z · clock a1b2c3d4e5f60718")
            .is_ok()
    );
    assert!(
        ui.find(zengui::view::detail::hex_dump(body)).is_ok(),
        "the owner's reply, in hex"
    );
    assert!(
        ui.find("ask ground/archive").is_ok(),
        "last-known is a button, never ambient"
    );

    // The owner silent: silence, not "no value" (S6) — and the archives
    // stated, or that none is known.
    let silent = leak(Ok::<_, zengui::services::ServiceError>(Arc::new(
        ValueRead::Owner(world.state_read(key, None, None)),
    )));
    let mut ui = simulator::<Message, _, _>(section(DetailData {
        identity: Some(identity),
        fetched: Fetched::Landed(silent),
        ..data(key)
    }));
    assert!(
        ui.find(
            "silent — the owner did not answer S4's GET within 5s: not a verdict about the key \
             (S6). An archive may still hold its last-known value."
        )
        .is_ok()
    );
    assert!(
        ui.find("presence not read — no archive.v1 provider is known to ask")
            .is_ok()
    );
    assert!(ui.find("no value").is_err(), "silence is never 'no value'");

    // An archive's answer beside the silence: last-known, never current.
    let last_known = leak(LastKnown {
        key: key.to_string(),
        archive: archive.clone(),
        answer: Some(Ok(Arc::new(world.state_read(
            key,
            Some(body),
            Some("ground/archive"),
        )))),
    });
    let mut ui = simulator::<Message, _, _>(section(DetailData {
        identity: Some(identity),
        fetched: Fetched::Landed(silent),
        archives: Some(vec![archive.clone()]),
        last_known: Some(last_known),
        ..data(key)
    }));
    assert!(
        ui.find("last-known from ground/archive — not current (S5, S6)")
            .is_ok()
    );
    assert!(ui.find("confirmed by alignment (§4.4)").is_ok());
    assert!(
        ui.find("via owner GET (S4)").is_err(),
        "an archive's answer never wears the owner's"
    );

    // A foreign key's raw GET, and a raw GET answered by nobody.
    let foreign = leak(world.resolve(FOREIGN));
    let read = leak(Ok::<_, zengui::services::ServiceError>(Arc::new(raw(
        FOREIGN,
        b"hello",
        "text/plain",
    ))));
    let mut ui = simulator::<Message, _, _>(section(DetailData {
        identity: Some(foreign),
        fetched: Fetched::Landed(read),
        ..data(FOREIGN)
    }));
    assert!(ui.find("via raw GET").is_ok());
    let nobody = leak(Ok::<_, zengui::services::ServiceError>(Arc::new(
        ValueRead::Replies(Replies {
            kind: ReadKind::Raw,
            selector: FOREIGN.to_string(),
            replies: Vec::new(),
            errors: 1,
            elided: 0,
            retention: None,
        }),
    )));
    let mut ui = simulator::<Message, _, _>(section(DetailData {
        identity: Some(foreign),
        fetched: Fetched::Landed(nobody),
        ..data(FOREIGN)
    }));
    assert!(
        ui.find("raw GET: no reply — a non-verdict, not proof of absence (RFC 05 §3.1)")
            .is_ok()
    );
    assert!(
        ui.find("1 error reply — a refusal is not a value, and not silence")
            .is_ok()
    );

    // In flight: asking, never "not asked".
    let mut ui = simulator::<Message, _, _>(section(DetailData {
        identity: Some(identity),
        fetched: Fetched::Asking(ReadKind::Owner),
        ..data(key)
    }));
    assert!(ui.find("asking — owner GET (S4) …").is_ok());
}

/// #765: S4's read keeps the owner's reply bytes on its row, so the value
/// shows them in hex beside the decode, and bytes that did not decode as
/// their declared type are sniffed from them, as a GET's reply is, rather
/// than only counted. A row built with no bytes says so.
#[test]
fn an_owner_read_shows_its_bytes_in_hex() {
    use zengui::view::detail::hex_dump;
    use zenkey_fleet::report::{Provenance, StateReading, StateReport, StateRow, StateValue};
    let world = common::World::new();
    let key = common::ETH0;
    let identity = leak(world.resolve(key));
    let resolved = identity.resolved.as_ref().expect("a state key resolves");
    // S4's answer as the fleet hands it back: the row rendered through the
    // world's contract, carrying `bytes`.
    let owner_read = |body: &[u8], bytes: Option<&[u8]>| {
        let report = StateReport {
            reading: StateReading::Current,
            address: resolved.address.to_string(),
            iface: resolved.iface.to_string(),
            fingerprint: resolved.revision.fingerprint().to_string(),
            resource: resolved.resource_name(),
            values: resolved.values.clone(),
            selectors: vec![key.to_string()],
            archive: None,
            timeout_s: 5.0,
            rows: vec![StateRow {
                key: key.to_string(),
                value: StateValue::Value {
                    payload: Box::new(world.check(key, body, "application/json").rendering),
                },
                bytes: bytes.map(|b| b.to_vec().into()),
                timestamp: None,
                confirmed: None,
                identity: None,
            }],
        };
        zengui::value::StateAnswer::new(report, identity, |_| Provenance::Unattributable)
    };
    let draw = |answer: zengui::value::StateAnswer| {
        let decoded = answer.decoded.clone().expect("a value row is checked");
        let fetched = leak(Ok::<_, zengui::services::ServiceError>(Arc::new(
            ValueRead::Owner(answer),
        )));
        simulator::<Message, _, _>(section(DetailData {
            identity: Some(identity),
            fetched: Fetched::Landed(fetched),
            decoded: Some(leak(decoded).as_ref()),
            ..data(key)
        }))
    };

    let body = br#"{"name":"eth0","index":2,"namespace":"default","is_up":true}"#;
    let mut ui = draw(owner_read(body, Some(body)));
    assert!(ui.find("via owner GET (S4)").is_ok());
    assert!(ui.find("hex").is_ok(), "the hex side is present");
    assert!(ui.find(hex_dump(body)).is_ok(), "the owner's reply, in hex");
    assert!(ui.find("contract-decoded <json:NetworkInterface>").is_ok());

    // Not the declared type: the sniff of what arrived, beside its hex.
    let bad = b"not json";
    let mut ui = draw(owner_read(bad, Some(bad)));
    assert!(
        ui.find(
            "<json:NetworkInterface?> structural (the bytes did not decode as their declared \
             type)"
        )
        .is_ok()
    );
    assert!(
        ui.find(zenkey_fleet::structural(bad)).is_ok(),
        "sniffed from the row's bytes, not only counted"
    );
    assert!(ui.find(hex_dump(bad)).is_ok());

    // A row that kept no bytes: said, never a blank well.
    let mut ui = draw(owner_read(body, None));
    assert!(
        ui.find("no hex — this read handed back its rendering with no bytes beside it")
            .is_ok()
    );
    assert!(ui.find(hex_dump(body)).is_err());
}

/// #765: a raw GET answered with a tombstone — an owner's `reply_del`,
/// which `fleet_get` now keeps as a delete — reads as one: the retired chip
/// where a size would be, the tombstone's words, and no hex of an empty
/// payload.
#[test]
fn a_tombstoned_raw_reply_reads_as_deleted() {
    let world = common::World::new();
    let identity = leak(world.resolve(FOREIGN));
    let ValueRead::Replies(mut read) = raw(FOREIGN, b"", "zenoh/bytes") else {
        unreachable!("a raw GET's answer")
    };
    read.replies[0].delete = true;
    let fetched = leak(Ok::<_, zengui::services::ServiceError>(Arc::new(
        ValueRead::Replies(read),
    )));
    let mut ui = simulator::<Message, _, _>(section(DetailData {
        identity: Some(identity),
        fetched: Fetched::Landed(fetched),
        ..data(FOREIGN)
    }));
    assert!(ui.find("via raw GET").is_ok());
    assert!(ui.find("DELETE (tombstone)").is_ok());
    assert!(
        ui.find("a tombstone — retirement, not an empty value (RFC 04 §1.2)")
            .is_ok()
    );
    assert!(ui.find("0 B").is_err(), "never an empty value's size");
    assert!(ui.find("hex").is_err(), "no hex of an empty payload");
}

/// #614 (FL8a): what is not read on selection says why, in the plan's own
/// terms — an operation is a call, a stream without history keeps nothing,
/// an event is replayed by button within its retention — and a replay's
/// silence is the capture's, the bus never asked.
#[test]
fn what_is_not_read_says_why() {
    let world = common::World::new();
    let draw = |key: &'static str, replaying: bool| {
        let identity = leak(world.resolve(key));
        simulator::<Message, _, _>(section(DetailData {
            identity: Some(identity),
            replaying,
            ..data(key)
        }))
    };
    let mut ui = draw(common::POSITION, false);
    assert!(
        ui.find(
            "a stream that declares no history keeps no value to read (§2.5) — watch it to see \
             its samples"
        )
        .is_ok()
    );
    let mut ui = draw("zk2/robot1/nav/nav.v2/@op/set_origin", false);
    assert!(
        ui.find(
            "an operation's key holds no value — a GET on it is a call (spec §5), so selecting \
             it asks nothing"
        )
        .is_ok()
    );
    let mut ui = draw(common::ETH0, true);
    assert!(
        ui.find(
            "nothing recorded for this key up to the playhead — a replay asks the bus nothing, \
             so this is the capture's silence, not the bus's"
        )
        .is_ok()
    );
    assert!(
        ui.find("LAST-KNOWN (S5) — AN ARCHIVE'S, NEVER CURRENT")
            .is_err(),
        "a replay asks no archive"
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
    let key = "v1/h-3fa9c2d41b7e/state/sysinfo/health";

    const NOT_ASKED: &str = "no value read — selecting a concrete key reads it once, as its \
                             identity says (the owner's GET for state, a raw GET for a foreign \
                             key), nothing ambient";
    const SUPERSEDED: &str = "superseded — the last fetch answered a different \
                              subject, so nothing here describes this key. \
                              Re-select it to ask again.";

    let mut ui = simulator::<Message, _, _>(section(data(key)));
    assert!(
        ui.find(NOT_ASKED).is_ok(),
        "nothing was asked, and the pane says so"
    );
    assert!(ui.find(SUPERSEDED).is_err());

    let mut ui = simulator::<Message, _, _>(section(DetailData {
        fetched: Fetched::Superseded,
        ..data(key)
    }));
    assert!(
        ui.find(SUPERSEDED).is_ok(),
        "an answer that is real but about something else is its own state"
    );
    assert!(
        ui.find(NOT_ASKED).is_err(),
        "and must not be reported as an unasked question"
    );
}

/// #120 on zk2 (#614, FL8a): the declared QoS is the resource's (§2.4),
/// read off the key's identity, and a sample that rode other axes is named
/// axis by axis with both sides — the wire the fact, the contract the
/// claim. Reliability is the link's and is not compared.
#[test]
fn a_qos_mismatch_is_named_per_axis() {
    use zengui::history::HistoryEntry;
    let world = common::World::new();
    let identity = leak(world.resolve(common::ETH0));
    let declared =
        zengui::view::detail::declared_qos(Some(identity)).expect("a state declares QoS");
    let entry = |priority, qos_recorded| HistoryEntry {
        seq: 0,
        received: Instant::now(),
        timestamp: None,
        stamp: None,
        is_delete: false,
        len: 2,
        encoding: "application/json".into(),
        payload: zenoh::bytes::ZBytes::from("{}"),
        priority,
        congestion_control: zenoh::qos::CongestionControl::Drop,
        reliability: zenoh::qos::Reliability::BestEffort,
        express: false,
        qos_recorded,
        source: None,
        attachment: None,
        value: None,
        preview: "{}".into(),
    };
    let off = leak(entry(zenoh::qos::Priority::RealTime, true));
    let mut ui = simulator::<Message, _, _>(section(DetailData {
        identity: Some(identity),
        observed: Some(off),
        ..data(common::ETH0)
    }));
    assert!(
        ui.find(zengui::view::detail::axes_token(&declared)).is_ok(),
        "the claim"
    );
    assert!(
        ui.find("real_time/drop/best_effort").is_ok(),
        "the fact, as the recorder spells it"
    );
    assert!(
        ui.find("observed axes differ from the declared QoS")
            .is_ok()
    );
    assert!(
        ui.find("the wire is the fact, the contract the claim (§2.4)")
            .is_ok()
    );
    assert!(
        ui.find(format!(
            "priority: declared {}, observed real_time",
            declared.priority
        ))
        .is_ok(),
        "each axis that differs is named with both sides"
    );
    if declared.congestion != "drop" {
        assert!(
            ui.find(format!(
                "congestion: declared {}, observed drop",
                declared.congestion
            ))
            .is_ok()
        );
    }
    assert!(
        ui.find("reliability is the link's, as received — shown, not compared")
            .is_ok()
    );

    // A replayed row that recorded no axes draws no verdict at all.
    let unrecorded = leak(entry(zenoh::qos::Priority::RealTime, false));
    let mut ui = simulator::<Message, _, _>(section(DetailData {
        identity: Some(identity),
        observed: Some(unrecorded),
        ..data(common::ETH0)
    }));
    assert!(ui.find("QoS not recorded").is_ok());
    assert!(ui.find("observed axes match the declared QoS").is_err());
    assert!(
        ui.find("observed axes differ from the declared QoS")
            .is_err(),
        "no verdict from axes a row never recorded"
    );
}

/// The Inspector dispatches on the subject (#182). The plane sections it
/// once grew for v1's `@blob` and `@media` keys were deleted in #614 (FL11).
#[test]
fn the_inspector_follows_the_subject() {
    use zengui::message::Subject;
    use zengui::nodes::NodesState;
    use zengui::view::inspector::pane;

    let nodes = NodesState::default();

    // Nothing selected: the Inspector holds nothing of its own and says so.
    let none = Subject::None;
    let mut ui = simulator::<Message, _, _>(pane(inspector(&none, None, &nodes)));
    assert!(ui.find("Nothing selected").is_ok());
    assert!(ui.find("Detail").is_err(), "no subject, no sections");

    // A prefix is a subtree, and the pane says that rather than reporting
    // "no value fetched" about a key that was never a key (#85).
    let prefix = Subject::Prefix("zk2/robot1/nav".into());
    let mut ui = simulator::<Message, _, _>(pane(inspector(&prefix, None, &nodes)));
    assert!(ui.find("A subtree, not a key").is_ok());
    assert!(ui.find("History").is_err(), "a subtree records nothing");

    // An ordinary state key: detail and history.
    let key = Subject::Key(common::ETH0.into());
    let world = common::World::new();
    let identity = leak(world.resolve(common::ETH0));
    let mut ui = simulator::<Message, _, _>(pane(inspector(&key, Some(identity), &nodes)));
    assert!(ui.find("Detail").is_ok());
    assert!(
        ui.find("History").is_ok(),
        "the timeline is a section now, not another tab"
    );

    // An address subject (#614, FL9): presence and health, not a value —
    // and with no presence read, "not read", never "nobody there".
    let address = Subject::Address("robot1/nav".parse().expect("an address"));
    let mut ui = simulator::<Message, _, _>(pane(inspector(&address, None, &nodes)));
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
    let subject = Subject::Address("robot1/nav".parse().expect("an address"));
    let mut data = inspector(&subject, None, &nodes);
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
    use zengui::view::detail::SeriesData;

    let series = SeriesData {
        leaves: NumericLeaves::default(),
        leaf: None,
        value: Series::new(),
        rate: RateSampler::new().series().clone(),
        unit: zengui::metric::Unit::Unresolved,
        metric: None,
        caches: Default::default(),
    };
    let mut ui = simulator::<Message, _, _>(section(DetailData {
        series: Some(&series),
        history_entries: Some(3),
        ..data(FOREIGN)
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
    use zengui::view::detail::SeriesData;

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
        unit: zengui::metric::Unit::Known(
            "percent".to_string(),
            zenkey_model::telemetry::Source::Declared,
        ),
        metric: None,
        caches: Default::default(),
    };
    let mut ui = simulator::<Message, _, _>(section(DetailData {
        series: Some(&series),
        history_entries: Some(3),
        ..data(REGISTERED)
    }));
    assert!(ui.find("Series").is_ok());
    assert!(
        ui.find(
            "value [percent]: 42 now · 41 min · 42 max · 3 points · 1 drawn as gaps, \
             not interpolated"
        )
        .is_ok(),
        "the caption carries the numbers, the unit and the gap count"
    );
    assert!(
        ui.find("message rate (samples/s): 5.0/s now · 5.0/s min · 5.0/s max · 1 point")
            .is_ok(),
        "the message rate is named as such, in the shared rate wording"
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

    // A resource that is no telemetry.v1 metric declares no unit, and the
    // axis says why rather than guess one (#819, TD; it read "a zk2
    // contract declares none" until telemetry.v1 declared units).
    drop(ui);
    let unitless = SeriesData {
        unit: zengui::metric::Unit::NotAMetric,
        ..series
    };
    let mut ui = simulator::<Message, _, _>(section(DetailData {
        series: Some(&unitless),
        ..data(REGISTERED)
    }));
    assert!(
        ui.find(
            "no unit — this resource is no telemetry.v1 metric (§2.1), so nothing declares one \
             for its leaves, and the axis is not guessed"
        )
        .is_ok()
    );
}

// ── The declared type (#234; zk2's since #614, FL8a) ─────────────────────

fn inspector<'a>(
    subject: &'a zengui::message::Subject,
    identity: Option<&'a zenkey_fleet::Resolution>,
    nodes: &'a zengui::nodes::NodesState,
) -> zengui::view::inspector::InspectorData<'a> {
    zengui::view::inspector::InspectorData {
        slot: zengui::message::SlotId::FOLLOW,
        subject,
        identity,
        fetched: zengui::view::detail::Fetched::NotAsked,
        replaying: false,
        archives: None,
        last_known: None,
        decoded: None,
        series: None,
        history: None,
        history_scroll: Default::default(),
        watched: false,
        snapshot: None,
        compare_snapshot: false,
        latency: None,
        presence: None,
        nodes,
        fields: Box::leak(Box::default()),
        why: Box::leak(Box::default()),
        consumers: Box::leak(Box::default()),
        sp: sp(),
    }
}

/// The type-subject arm (#234), on zk2: a resolved key's declared type as
/// the contract names it — its kind, its name, the artifact it lives in —
/// and the members of the revision that carry it, from the revision's own
/// schema view.
#[test]
fn the_inspector_places_a_declared_type_in_its_revision() {
    use zengui::message::Subject;

    let world = common::World::new();
    let nodes = zengui::nodes::NodesState::default();
    let identity = leak(world.resolve(common::ETH0));
    let subject = Subject::Key(common::ETH0.into());

    let mut ui = simulator::<Message, _, _>(zengui::view::inspector::pane(inspector(
        &subject,
        Some(identity),
        &nodes,
    )));
    assert!(ui.find("Type").is_ok());
    assert!(ui.find("jsonschema").is_ok(), "the type's kind");
    let resolved = identity.resolved.as_ref().expect("resolved");
    let name = zengui::keyfacts::type_name(resolved);
    let view = resolved
        .revision
        .schema_view(None, false)
        .expect("the revision's schema view");
    let carriers = view.members.iter().filter(|m| m.type_.name == name).count();
    assert!(
        ui.find(format!(
            "{name} — carried by {} of tc.netif.v1's {} members",
            if carriers == 1 {
                "1 member".to_string()
            } else {
                format!("{carriers} members")
            },
            view.members.len()
        ))
        .is_ok(),
        "the type's place in its revision"
    );
    assert!(
        ui.find("state/interfaces/{ns}/{iface} · type").is_ok(),
        "each carrier, by resource and member"
    );

    // A foreign key has no declared type, and no section claims one.
    let foreign = leak(world.resolve(FOREIGN));
    let subject = Subject::Key(FOREIGN.into());
    let mut ui = simulator::<Message, _, _>(zengui::view::inspector::pane(inspector(
        &subject,
        Some(foreign),
        &nodes,
    )));
    assert!(ui.find("Type").is_err());
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
    let world = common::World::new();
    let render = |key: &'static str, body: &'static [u8], encoding: &str| {
        let fetched = leak(Ok::<_, zengui::services::ServiceError>(Arc::new(raw(
            key, body, encoding,
        ))));
        let decoded: &'static zengui::value::DecodedValue = leak(zengui::value::DecodedValue::new(
            world.check(key, body, encoding),
            body,
        ));
        (
            decoded,
            simulator::<Message, _, _>(section(DetailData {
                fetched: Fetched::Landed(fetched),
                decoded: Some(decoded),
                ..data(key)
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
    let world = common::World::new();
    let draw = |key: &'static str| {
        let identity = leak(world.resolve(key));
        simulator::<Message, _, _>(section(DetailData {
            identity: Some(identity),
            ..data(key)
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

/// #614 (FL8a): the status strip's fetched cell names the zk2 read — "owner
/// GET (S4)", "silent", "raw GET" — and an archive's answer beside it,
/// "last-known from <archive> — not current", never in its place.
#[test]
fn the_strip_names_the_zk2_read_and_last_known_beside_it() {
    use zengui::view::detail::LastKnown;
    use zengui::view::status::{self, Status};

    type Read = (
        String,
        Result<Arc<ValueRead>, zengui::services::ServiceError>,
    );
    fn shows(fetched: &Read, last_known: Option<&LastKnown>, text: &str) -> bool {
        let link = zengui::message::LinkState::Pumping;
        let watched: Vec<String> = vec![];
        let mut ui = simulator::<Message, _, _>(status::strip(Status {
            link: &link,
            namespace_label: "(bus root)",
            watched: &watched,
            skeleton: None,
            keys_unwatched: 0,
            fetched: Some(fetched),
            last_known,
            scope_label: "all",
            scope_coverage: "",
            keys: 0,
            keys_evicted: 0,
            identity: (0, 0),
            totals: Default::default(),
            contracts: Default::default(),
            presence: None,
            seeding: 0,
            seeded_watches: 0,
            seed_totals: Default::default(),
            unreachable: false,
            prefs_note: None,
            replaying: false,
            retention: None,
        }));
        ui.find(text).is_ok()
    }

    let world = common::World::new();
    let key = common::ETH0;
    let body = br#"{"name":"eth0","index":2,"namespace":"default","is_up":true}"#;
    let answered: Read = (
        key.to_string(),
        Ok(Arc::new(ValueRead::Owner(world.state_read(
            key,
            Some(body),
            None,
        )))),
    );
    assert!(shows(
        &answered,
        None,
        &format!("fetched {key}: owner GET (S4): 60 B")
    ));

    let silent: Read = (
        key.to_string(),
        Ok(Arc::new(ValueRead::Owner(
            world.state_read(key, None, None),
        ))),
    );
    let lk = LastKnown {
        key: key.to_string(),
        archive: "ground/archive".parse().expect("an address"),
        answer: Some(Ok(Arc::new(world.state_read(
            key,
            Some(body),
            Some("ground/archive"),
        )))),
    };
    assert!(shows(
        &silent,
        Some(&lk),
        &format!(
            "fetched {key}: silent — owner GET (S4) unanswered in 5s (not a verdict) · \
             last-known from ground/archive — not current"
        )
    ));

    let foreign: Read = (
        FOREIGN.to_string(),
        Ok(Arc::new(raw(FOREIGN, b"hello", "text/plain"))),
    );
    assert!(shows(
        &foreign,
        None,
        &format!("fetched {FOREIGN}: raw GET: 5 B")
    ));
}
