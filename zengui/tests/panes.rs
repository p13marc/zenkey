//! Pane rendering, driven headlessly by `iced_test`.
//!
//! Views are free `fn …(&State, …) -> Element<'_, Message>` over plain state
//! precisely so they can be rendered standalone here — no window, no bus. What
//! these pin is that the *honesty* surfaces actually reach the screen: the
//! tri-state registration badge, the positional overlay, and the empty states
//! that explain themselves rather than implying a verdict.
//!
//! `iced_test` selects widgets by the text they contain, so `find("x")` is
//! literally "is this on screen".

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Instant;

use iced_test::simulator;
use zengui::history::HistoryRecorder;
use zengui::keyfacts::KeyFacts;
use zengui::message::Message;
use zengui::view::tree::{self, FactsIndex};
use zenkey_fleet::stats::StatsTable;
use zenkey_fleet::{KeyTreeSnapshot, SliceSet};

const REGISTERED: &str = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used";
const UNREGISTERED: &str = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/no/such/thing";
const FOREIGN: &str = "demo/example/foo";

fn snapshot(keys: &[&str]) -> zenkey_fleet::skeleton::MergedNode {
    let mut stats = StatsTable::new();
    let now = Instant::now();
    for k in keys {
        stats.record(k, 8, None, now, None, None);
    }
    let observed = KeyTreeSnapshot::build(&stats);
    // Pane tests watch everything: rows read Observed, as the bootstrap did.
    let skel = zenkey_fleet::Skeleton::build("", &SliceSet::default(), &BTreeMap::new(), None);
    zenkey_fleet::skeleton::merge(&skel, &observed, &["**".to_string()])
}

fn slices() -> SliceSet {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixture-tests/registry");
    SliceSet::from_dirs(&[dir]).expect("fixture registry")
}

/// Expand every prefix of every key, so leaves are visible.
fn expand_all(keys: &[&str]) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for k in keys {
        let mut acc = String::new();
        for chunk in k.split('/') {
            acc = if acc.is_empty() {
                chunk.to_string()
            } else {
                format!("{acc}/{chunk}")
            };
            set.insert(acc.clone());
        }
    }
    set
}

fn index(keys: &[&str], with_slices: bool) -> FactsIndex {
    let slices = slices();
    let mut cache = FactsIndex::default();
    for k in keys {
        cache.ensure("", k, with_slices.then_some(&slices));
    }
    cache
}

fn render(keys: &[&str], with_slices: bool) -> (tree::Flattened, FactsIndex) {
    let snap = snapshot(keys);
    let flat = tree::flatten(&snap, "", &expand_all(keys), 500, std::time::Instant::now());
    (flat, index(keys, with_slices))
}

/// The registration tri-state must actually reach the screen, and its states
/// must read differently (RFC 09 §5.1 O4).
#[test]
fn the_tree_renders_the_registration_state() {
    let keys = [REGISTERED, UNREGISTERED, FOREIGN];
    let (flat, facts) = render(&keys, true);
    let watches = BTreeSet::new();
    let mut ui = simulator::<Message, _, _>(tree::pane(tree::TreeData {
        flat: &flat,
        pivot: tree::Pivot::Chunks,
        search: "",
        scroll_y: 0.0,
        viewport_h: 600.0,
        facts: &facts,
        watches: tree::Watches {
            mine: &watches,
            seeding: &watches,
        },
        selected: None,
    }));

    assert!(
        ui.find("registered").is_ok(),
        "a registered subject should be badged"
    );
    assert!(
        ui.find("unregistered").is_ok(),
        "an unregistered subject should be badged differently"
    );
    // The registry-declared type name is the payoff of the whole overlay.
    assert!(
        ui.find("TelemetryPoint").is_ok(),
        "the declared type should be shown"
    );
    // The positional labels for the conforming subtree.
    for role in ["version", "origin", "class", "producer", "subject"] {
        assert!(ui.find(role).is_ok(), "missing role label {role:?}");
    }
}

/// Before a registry is loaded, nothing may be badged either way — that would
/// report a verdict never obtained (RFC 09 §5.1 O4).
#[test]
fn an_unresolved_tree_claims_neither_way() {
    let keys = [REGISTERED];
    let (flat, facts) = render(&keys, false);
    let watches = BTreeSet::new();
    let mut ui = simulator::<Message, _, _>(tree::pane(tree::TreeData {
        flat: &flat,
        pivot: tree::Pivot::Chunks,
        search: "",
        scroll_y: 0.0,
        viewport_h: 600.0,
        facts: &facts,
        watches: tree::Watches {
            mine: &watches,
            seeding: &watches,
        },
        selected: None,
    }));

    assert!(
        ui.find("unregistered").is_err(),
        "a key we never checked must not read as unregistered"
    );
    assert!(ui.find("registered").is_err(), "…nor as registered");
    // It is still a fully labelled conforming key — only the registry verdict
    // is withheld.
    assert!(ui.find("producer").is_ok());
}

/// A foreign key is rendered, and rendered *plainly* — no invented labels.
#[test]
fn foreign_keys_render_without_convention_labels() {
    let keys = [FOREIGN];
    let (flat, facts) = render(&keys, true);
    let watches = BTreeSet::new();
    let mut ui = simulator::<Message, _, _>(tree::pane(tree::TreeData {
        flat: &flat,
        pivot: tree::Pivot::Chunks,
        search: "",
        scroll_y: 0.0,
        viewport_h: 600.0,
        facts: &facts,
        watches: tree::Watches {
            mine: &watches,
            seeding: &watches,
        },
        selected: None,
    }));

    for role in ["version", "origin", "class", "producer", "subject"] {
        assert!(
            ui.find(role).is_err(),
            "a foreign key must not be labelled {role:?}"
        );
    }
    // …and no registry verdict is invented for it either.
    assert!(ui.find("unregistered").is_err());
}

/// The empty tree must explain itself rather than implying the bus is quiet
/// (RFC 05 §3.1).
#[test]
fn the_empty_tree_explains_itself() {
    let flat = tree::Flattened::empty();
    let facts = FactsIndex::default();
    let watches = BTreeSet::new();
    let mut ui = simulator::<Message, _, _>(tree::pane(tree::TreeData {
        flat: &flat,
        pivot: tree::Pivot::Chunks,
        search: "",
        scroll_y: 0.0,
        viewport_h: 600.0,
        facts: &facts,
        watches: tree::Watches {
            mine: &watches,
            seeding: &watches,
        },
        selected: None,
    }));

    assert!(ui.find("Nothing observed yet").is_ok());
    // Selectors match a widget's whole text, so this is the full disclaimer.
    assert!(
        ui.find(
            "An empty tree is not a verdict about the bus (RFC 05 §3.1) — \
             it may simply mean nothing has published on the current scope."
        )
        .is_ok(),
        "the empty state must disclaim, not just name the emptiness"
    );
}

/// The call pane scaffolds from slices and labels the fanout refusal
/// (issue #60): the visual layer of the three-layer guard.
#[test]
fn the_call_pane_labels_forbidden_fanout() {
    use zengui::view::call::{CallForm, pane};
    use zenkey::slice::{ProcedureDecl, RegistrySlice};

    let slice = RegistrySlice {
        version: "1.0".into(),
        app: "t".into(),
        convention: 1,
        name: "netring".into(),
        service_origin: None,
        description: None,
        subjects: vec![],
        procedures: vec![ProcedureDecl {
            path: "capture/trigger".into(),
            kind: "write".into(),
            reply: Some("Ack".into()),
            request: None,
            encoding: None,
            fanout: Some("forbidden".into()),
            idempotent: Some(false),
            since: None,
            description: None,
        }],
        blob: vec![],
        media: vec![],
        deprecated: vec![],
    };
    let slices = SliceSet::from_slices(vec![slice]);

    let form = CallForm {
        producer: Some("netring".into()),
        procedure: Some("capture/trigger".into()),
        target: "*".into(),
        ..CallForm::default()
    };
    let roster = zengui::nodes::NodeRoster::default();
    let mut ui = simulator::<Message, _, _>(pane(&form, Some(&slices), &roster));
    assert!(
        ui.find("fanout = \"forbidden\" — a fleet (*) target is refused (RFC 05 §2.1)")
            .is_ok(),
        "the refusal must be visible before any send"
    );

    // Without a registry the pane says "not asked", not empty dropdowns.
    let empty = CallForm::default();
    let mut ui = simulator::<Message, _, _>(pane(&empty, None, &roster));
    assert!(ui.find("No registry loaded").is_ok());
}

/// Silence gets a name (#60's first acceptance): an origin the roster says is
/// alive and running the producer, that did not reply, is listed. RFC 05 §3.1
/// says "no reply" is not one condition — this is the join that says which.
#[test]
fn the_call_pane_names_the_origins_that_did_not_answer() {
    use zengui::view::call::{CallForm, pane};
    use zenkey_fleet::report::{CallAnswer, CallReport};

    let mut roster = zengui::nodes::NodeRoster::default();
    roster.seed(&BTreeMap::from([
        ("h-aaaaaaaaaaaa".to_string(), vec!["netring".to_string()]),
        ("h-bbbbbbbbbbbb".to_string(), vec!["netring".to_string()]),
        // A third node that does not run the producer must NOT be blamed.
        ("h-cccccccccccc".to_string(), vec!["sysinfo".to_string()]),
    ]));

    let form = CallForm {
        producer: Some("netring".into()),
        procedure: Some("introspect".into()),
        target: "*".into(),
        outcome: Some(Ok(CallReport {
            key: "v1/*/@rpc/netring/introspect".into(),
            answers: vec![CallAnswer {
                origin: "h-aaaaaaaaaaaa".into(),
                ok: true,
                value: None,
                text: Some("slice".into()),
                attachment: Some(serde_json::json!({"who": "me"})),
                attachment_bytes: Some(12),
                error: None,
            }],
        })),
        ..CallForm::default()
    };
    let slices = SliceSet::from_slices(vec![zenkey::slice::RegistrySlice {
        version: "1.0".into(),
        app: "t".into(),
        convention: 1,
        name: "netring".into(),
        service_origin: None,
        description: None,
        subjects: vec![],
        procedures: vec![zenkey::slice::ProcedureDecl {
            path: "introspect".into(),
            kind: "read".into(),
            reply: Some("RegistrySlice".into()),
            request: None,
            encoding: None,
            fanout: None,
            idempotent: Some(true),
            since: None,
            description: None,
        }],
        blob: vec![],
        media: vec![],
        deprecated: vec![],
    }]);
    let mut ui = simulator::<Message, _, _>(pane(&form, Some(&slices), &roster));
    assert!(
        ui.find("did not answer, though alive: h-bbbbbbbbbbbb")
            .is_ok(),
        "the alive non-replier must be named, not folded into silence"
    );
}

/// Request-form scaffolding (§6.4 item 3): a declared request type whose
/// schema has not been fetched reads as "not asked", never as "takes
/// nothing" (O4) — and once fetched, the fields are on screen.
#[test]
fn the_call_pane_distinguishes_an_unasked_schema_from_an_empty_one() {
    use zengui::view::call::{CallForm, SchemaField, pane};
    use zenkey::slice::{ProcedureDecl, RegistrySlice};

    let slice = RegistrySlice {
        version: "1.0".into(),
        app: "t".into(),
        convention: 1,
        name: "netring".into(),
        service_origin: None,
        description: None,
        subjects: vec![],
        procedures: vec![ProcedureDecl {
            path: "capture/start".into(),
            kind: "write".into(),
            reply: Some("Ack".into()),
            request: Some("CaptureSpec".into()),
            encoding: None,
            fanout: None,
            idempotent: Some(false),
            since: None,
            description: None,
        }],
        blob: vec![],
        media: vec![],
        deprecated: vec![],
    };
    let slices = SliceSet::from_slices(vec![slice]);
    let roster = zengui::nodes::NodeRoster::default();

    let unasked = CallForm {
        producer: Some("netring".into()),
        procedure: Some("capture/start".into()),
        ..CallForm::default()
    };
    {
        let mut ui = simulator::<Message, _, _>(pane(&unasked, Some(&slices), &roster));
        assert!(
            ui.find("CaptureSpec: schema not asked yet — pick the procedure again once connected to scaffold from it")
                .is_ok(),
            "an unfetched schema must not read as a request with no fields"
        );
    }

    let asked = CallForm {
        request_fields: Some(vec![
            SchemaField {
                name: "iface".into(),
                type_name: Some("string".into()),
                required: true,
            },
            SchemaField {
                name: "seconds".into(),
                type_name: Some("integer".into()),
                required: false,
            },
        ]),
        ..unasked
    };
    {
        let mut ui = simulator::<Message, _, _>(pane(&asked, Some(&slices), &roster));
        assert!(
            ui.find("CaptureSpec fields: iface: string*, seconds: integer")
                .is_ok(),
            "the served schema's fields scaffold the form"
        );
    }
    // …and the scaffold is a real body, with the declared shapes.
    let body: serde_json::Value =
        serde_json::from_str(&asked.scaffold().expect("scaffold")).expect("valid JSON");
    assert_eq!(body["iface"], serde_json::json!(""));
    assert_eq!(body["seconds"], serde_json::json!(0));
}

/// The publish pane's three provenances must never look alike (#60/#97):
/// encoded, sent as typed, and sent raw are different facts about what is on
/// the wire, and a user who cannot tell them apart cannot trust any of them.
#[test]
fn the_publish_pane_says_how_the_body_reached_the_wire() {
    use zengui::view::publish::{PublishForm, pane};
    use zenkey_fleet::BodySource;

    let base = PublishForm {
        key: "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used".into(),
        body: "{\"value\": 1}".into(),
        encoding_used: Some("application/protobuf".into()),
        source: Some(BodySource::Encoded {
            type_name: "TelemetryPoint".into(),
        }),
        ..PublishForm::default()
    };
    {
        let mut ui = simulator::<Message, _, _>(pane(&base, true));
        assert!(
            ui.find("encoded as TelemetryPoint → application/protobuf")
                .is_ok()
        );
    }

    let as_typed = PublishForm {
        source: Some(BodySource::AsTyped),
        encoding_used: None,
        note: Some("sysinfo serves no schema for TelemetryPoint".into()),
        ..base.clone()
    };
    {
        let mut ui = simulator::<Message, _, _>(pane(&as_typed, true));
        assert!(ui.find("sent as typed → (no encoding set)").is_ok());
        assert!(
            ui.find("sysinfo serves no schema for TelemetryPoint")
                .is_ok(),
            "the engine's note is rendered, not swallowed"
        );
    }

    let raw = PublishForm {
        source: Some(BodySource::Raw),
        raw: true,
        ..base.clone()
    };
    let mut ui = simulator::<Message, _, _>(pane(&raw, true));
    assert!(ui.find("sent raw — bytes verbatim, not encoded").is_ok());
}

/// The publish pane's honesty surfaces: the closed QoS vocabulary, the O4
/// matching badge, and the O6 bounded log.
#[test]
fn the_publish_pane_bounds_its_log_and_never_invents_a_matcher() {
    use zengui::view::publish::{LOG_LINES, PublishForm, pane};

    let mut form = PublishForm {
        key: "demo/foreign/key".into(),
        armed: true,
        // Armed, but the status could not be asked: "not asked" (O4).
        matching: None,
        ..PublishForm::default()
    };
    {
        let mut ui = simulator::<Message, _, _>(pane(&form, true));
        assert!(ui.find("matching: not asked").is_ok());
    }
    // The five profiles, and only those (RFC 04 §3's closed vocabulary).
    assert_eq!(zenkey::qos::QosProfile::ALL.len(), 5);
    for name in ["sampled", "refreshed", "transition", "alert", "frame"] {
        assert!(
            zenkey::qos::QosProfile::ALL
                .iter()
                .any(|p| p.name() == name),
            "{name} must stay in the vocabulary the picker renders"
        );
    }

    form.matching = Some(false);
    {
        let mut ui = simulator::<Message, _, _>(pane(&form, true));
        assert!(
            ui.find(
                "matching: no subscriber currently matches this publication — a routing \
                 fact about this publisher, not a fleet verdict (RFC 05 §3.1)"
            )
            .is_ok(),
            "a false matching badge must disclaim, exactly as the CLI does"
        );
    }

    // O6: the log is bounded, and it reports what the bound cost.
    for i in 0..(LOG_LINES + 3) {
        form.log(true, format!("sent {i} bytes"));
    }
    {
        let mut ui = simulator::<Message, _, _>(pane(&form, true));
        assert!(
            ui.find(format!(
                "send log — {LOG_LINES} shown, 3 dropped (bounded at {LOG_LINES})"
            ))
            .is_ok(),
            "a trimmed log must say how much it trimmed"
        );
    }

    // With no registry the pane says "not asked", never "unregistered".
    let mut ui = simulator::<Message, _, _>(pane(&form, false));
    assert!(
        ui.find(
            "no registry loaded — the body cannot be schema-checked, and that is \
             \"not asked\", not \"unregistered\" (O4)"
        )
        .is_ok()
    );
}

/// The detail pane (§6.4 item 5 + #66): the decoded side is tagged with HOW
/// it was produced — schema-decoded and sniffed must never look alike — and
/// the empty fetch renders the non-verdict sentence.
#[test]
fn the_detail_pane_tags_decode_provenance() {
    use std::sync::Arc;
    use zengui::view::detail::{DetailData, Fetched, section};
    use zenkey_fleet::decode::Rendering;
    use zenkey_fleet::{FetchOutcome, FetchedValue, ValueSource};

    let slices = slices();
    let key = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used";
    let mut facts = KeyFacts::project("", key);
    facts.resolve(&slices);

    let fetched: Result<Arc<FetchOutcome>, String> =
        Ok(Arc::new(FetchOutcome::Value(FetchedValue {
            key: key.to_string(),
            payload: zenoh::bytes::ZBytes::from(br#"{"value":42.0}"#.to_vec()),
            encoding: "application/json".into(),
            timestamp: None,
            attachment: None,
            source: ValueSource::Storage,
        })));

    // Structural fallback with a declared type: the honest <T?> tag.
    let decoded = (
        Some("TelemetryPoint".to_string()),
        Rendering::Structural(r#"{"value":42.0}"#.to_string()),
    );
    let mut ui = simulator::<Message, _, _>(section(DetailData {
        key,
        facts: Some(&facts),
        fetched: Fetched::Landed(&fetched),
        decoded: Some(&decoded),
        series: None,
        history_entries: None,
        observed: None,
        latency: None,
    }));
    assert!(ui.find("registered").is_ok(), "the facts section renders");
    assert!(
        ui.find("<TelemetryPoint?> structural (schema did not decode)")
            .is_ok(),
        "typed-but-undecoded must say so"
    );
    assert!(ui.find("hex").is_ok(), "the hex side is present");

    // The attributed nothing.
    let none: Result<Arc<FetchOutcome>, String> = Ok(Arc::new(FetchOutcome::None {
        attempted: ["get", "@adv cache", "subscribe window"],
    }));
    let mut ui = simulator::<Message, _, _>(section(DetailData {
        key,
        facts: Some(&facts),
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
        key,
        facts: None,
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
    use zengui::nodes::NodeRoster;
    use zengui::view::detail::Fetched;
    use zengui::view::inspector::{InspectorData, pane};
    use zengui::view::media::MediaState;
    use zengui::view::nodes::DetailState;
    use zenkey_fleet::facts::KeyFacts;

    let slices = slices();
    let blob = BlobState::default();
    let media = MediaState::default();
    let roster = NodeRoster::default();
    let node_detail = DetailState::NotAsked;

    let facts_for = |key: &str| {
        let mut f = KeyFacts::project("", key);
        f.resolve(&slices);
        f
    };
    fn data<'a>(
        subject: &'a Subject,
        facts: Option<&'a KeyFacts>,
        blob: &'a BlobState,
        media: &'a MediaState,
        slices: &'a SliceSet,
        roster: &'a NodeRoster,
        node_detail: &'a DetailState,
    ) -> InspectorData<'a> {
        InspectorData {
            subject,
            facts,
            fetched: Fetched::NotAsked,
            decoded: None,
            series: None,
            history: None,
            history_scroll: (0.0, 600.0),
            watched: false,
            latency: None,
            blob,
            media,
            slices: Some(slices),
            roster,
            node_detail,
        }
    }

    // Nothing selected: the Inspector holds nothing of its own and says so.
    let none = Subject::None;
    let mut ui = simulator::<Message, _, _>(pane(data(
        &none,
        None,
        &blob,
        &media,
        &slices,
        &roster,
        &node_detail,
    )));
    assert!(ui.find("Nothing selected").is_ok());
    assert!(ui.find("Detail").is_err(), "no subject, no sections");

    // A prefix is a subtree, and the pane says that rather than reporting
    // "no value fetched" about a key that was never a key (#85).
    let prefix = Subject::Prefix("v1/h-3fa9c2d41b7e".into());
    let mut ui = simulator::<Message, _, _>(pane(data(
        &prefix,
        None,
        &blob,
        &media,
        &slices,
        &roster,
        &node_detail,
    )));
    assert!(ui.find("A subtree, not a key").is_ok());
    assert!(ui.find("History").is_err(), "a subtree records nothing");

    // An ordinary state key: detail and history, and neither plane section.
    let state_key = "v1/h-3fa9c2d41b7e/state/sysinfo/health";
    let state_facts = facts_for(state_key);
    let key = Subject::Key(state_key.into());
    let mut ui = simulator::<Message, _, _>(pane(data(
        &key,
        Some(&state_facts),
        &blob,
        &media,
        &slices,
        &roster,
        &node_detail,
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
    let mut ui = simulator::<Message, _, _>(pane(data(
        &key,
        Some(&blob_facts),
        &blob,
        &media,
        &slices,
        &roster,
        &node_detail,
    )));
    assert!(ui.find("Blobs").is_ok(), "the plane is read off ClassKind");
    assert!(
        ui.find("Media").is_err(),
        "one plane, not every plane the key is not on"
    );

    // An origin subject: presence, not a value.
    let origin = Subject::Origin("h-3fa9c2d41b7e".into());
    let mut ui = simulator::<Message, _, _>(pane(data(
        &origin,
        None,
        &blob,
        &media,
        &slices,
        &roster,
        &node_detail,
    )));
    assert!(
        ui.find("no liveliness token observed for this origin")
            .is_ok()
    );
    assert!(
        ui.find("Detail").is_err(),
        "an origin has no value, no decode and no chart"
    );
}

/// The nodes pane (#61): a retracted token reads suspect, the catalog is
/// named, and freshness never invents a verdict.
#[test]
fn the_nodes_pane_marks_retraction_suspect_and_names_the_catalog() {
    use zengui::nodes::NodeRoster;
    use zengui::view::nodes::{DetailState, NodesData, pane};

    let mut roster = NodeRoster::default();
    roster.apply_transitions(
        "",
        &[
            ("v1/h-3fa9c2d41b7e/state/sysinfo/alive".to_string(), true),
            ("v1/h-aaaaaaaaaaaa/state/other/alive".to_string(), true),
            ("v1/h-aaaaaaaaaaaa/state/other/alive".to_string(), false),
        ],
        Instant::now(),
    );

    let mut ui = simulator::<Message, _, _>(pane(NodesData {
        roster: &roster,
        selected: None,
        detail: &DetailState::NotAsked,
    }));
    assert!(
        ui.find("sysinfo: alive").is_ok(),
        "the live row reads alive"
    );
    assert!(
        ui.find("other: suspect since 0s (token retracted)").is_ok(),
        "retraction reads suspect immediately, never aged out"
    );
    assert!(
        ui.find("catalog: no token observed — not proof none exists (RFC 05 §3.1)")
            .is_ok(),
        "the catalog is asked for by name, never folded into 'no nodes'"
    );
    assert!(
        ui.find("not watched — freshness unknown").is_ok(),
        "an unwatched producer's freshness is unknown, not fresh (O4)"
    );
}

/// The nodes pane's O4 ladder before any presence source reports.
#[test]
fn the_nodes_pane_distinguishes_not_asked_from_empty() {
    use zengui::nodes::NodeRoster;
    use zengui::view::nodes::{DetailState, NodesData, pane};

    let roster = NodeRoster::default();
    let mut ui = simulator::<Message, _, _>(pane(NodesData {
        roster: &roster,
        selected: None,
        detail: &DetailState::NotAsked,
    }));
    assert!(
        ui.find("no presence asked yet").is_ok(),
        "'not asked' must not render as an empty fleet"
    );
}

/// The node detail's freshness honesty: an unanswered sample is stale, and
/// zero ttl declarations is unknown, never fresh.
#[test]
fn the_node_detail_reports_freshness_honestly() {
    use std::sync::Arc;
    use zengui::nodes::NodeRoster;
    use zengui::view::nodes::{DetailState, NodesData, pane};
    use zenkey_fleet::{Freshness, NodeInfo, ProducerInfo};

    let mut roster = NodeRoster::default();
    roster.apply_transitions(
        "",
        &[("v1/h-3fa9c2d41b7e/state/sysinfo/alive".to_string(), true)],
        Instant::now(),
    );
    let info = NodeInfo {
        origin: "h-3fa9c2d41b7e".to_string(),
        producers: vec![ProducerInfo {
            name: "sysinfo".to_string(),
            alive: true,
            app: Some("demo".to_string()),
            registry_version: Some("1.0".to_string()),
            subjects: 2,
            procedures: 1,
            blob_tiers: vec![],
            media: vec![],
            deprecated_served: 0,
        }],
        freshness: vec![Freshness {
            producer: "sysinfo".to_string(),
            path: "health".to_string(),
            ttl_s: 30,
            age_s: None,
            stale: true,
        }],
    };
    let detail = DetailState::Loaded("h-3fa9c2d41b7e".to_string(), Ok(Arc::new(info)));
    let mut ui = simulator::<Message, _, _>(pane(NodesData {
        roster: &roster,
        selected: Some("h-3fa9c2d41b7e"),
        detail: &detail,
    }));
    assert!(
        ui.find("sysinfo/health  no sample answered — stale  (ttl 30s)  STALE")
            .is_ok(),
        "an unanswered ttl'd subject reads stale with its evidence"
    );
}

/// The doctor panel (#71): never-run is not "0 findings", findings render
/// with their check ids and RFC citations, and re-runs show deltas — all
/// over the exact struct `zenctl doctor --format json` serializes.
#[test]
fn the_doctor_pane_never_invents_a_verdict() {
    use std::sync::Arc;
    use zengui::doctor::DoctorState;
    use zengui::view::doctor::section;
    use zenkey_fleet::report::{DoctorFinding, DoctorReport, DoctorSeverity};

    let state = DoctorState::default();
    let mut ui = simulator::<Message, _, _>(section(&state, ""));
    assert!(
        ui.find("no doctor run yet").is_ok(),
        "never-run must not read as a clean fleet (O4)"
    );

    let report = DoctorReport {
        findings: vec![DoctorFinding {
            severity: DoctorSeverity::Error,
            check: "slice-sync".into(),
            subject: "h-3fa9c2d41b7e/sysinfo".into(),
            evidence: "registry version differs: served 1.0, local 2.0".into(),
            citation: Some("RFC 08 §6".into()),
        }],
        synced: vec![],
        introspect_answered: 1,
        live_producers: 1,
        describe_served: 0,
        describe_missing: 1,
        routers: 0,
        router_version: None,
        deep: false,
        observation: None,
    };
    let mut state = DoctorState::default();
    state.finish(
        Ok(zengui::doctor::DoctorRun {
            report: Arc::new(report.clone()),
            base: String::new(),
        }),
        "",
    );
    let mut ui = simulator::<Message, _, _>(section(&state, ""));
    assert!(ui.find("slice-sync").is_ok(), "the stable check id renders");
    assert!(ui.find("RFC 08 §6").is_ok(), "the citation renders");
    assert!(
        ui.find("registry version differs: served 1.0, local 2.0")
            .is_ok(),
        "the evidence renders"
    );
    assert!(
        ui.find("go to subject").is_ok(),
        "an origin/producer subject offers navigation"
    );

    drop(ui);

    // Second run: the finding is fixed, a new one appears — the delta strip
    // says exactly that.
    let second = DoctorReport {
        findings: vec![DoctorFinding {
            severity: DoctorSeverity::Info,
            check: "describe-missing".into(),
            subject: "fleet".into(),
            evidence: "1 producer(s) serve no describe".into(),
            citation: Some("RFC 08 §7".into()),
        }],
        ..report
    };
    state.finish(
        Ok(zengui::doctor::DoctorRun {
            report: Arc::new(second),
            base: String::new(),
        }),
        "",
    );
    let mut ui = simulator::<Message, _, _>(section(&state, ""));
    assert!(
        ui.find("vs previous run: 1 new · 1 fixed · 0 unchanged")
            .is_ok(),
        "the delta strip renders"
    );
    assert!(
        ui.find("fixed since last run").is_ok(),
        "fixed findings stay visible, dimmed"
    );
}

/// #161: the listen phase's observation section renders its scope statement
/// — window, scopes, drops, synthetic count — and a report without one shows
/// nothing (absent, never zeroed).
#[test]
fn the_doctor_pane_states_what_the_listen_phase_observed() {
    use std::sync::Arc;
    use zengui::doctor::DoctorState;
    use zengui::view::doctor::section;
    use zenkey_fleet::report::{DoctorReport, ObservationSummary};

    let report = DoctorReport {
        findings: vec![],
        synced: vec![],
        introspect_answered: 1,
        live_producers: 1,
        describe_served: 0,
        describe_missing: 1,
        routers: 0,
        router_version: None,
        deep: false,
        observation: Some(ObservationSummary {
            window_s: 10.0,
            scopes: vec!["v1/*/state/**".into(), "v1/*/telemetry/**".into()],
            samples: 42,
            keys_seen: 7,
            dropped: 3,
            synthetic_marked: 2,
        }),
    };
    let mut state = DoctorState::default();
    state.finish(
        Ok(zengui::doctor::DoctorRun {
            report: Arc::new(report),
            base: String::new(),
        }),
        "",
    );
    let mut ui = simulator::<Message, _, _>(section(&state, ""));
    assert!(
        ui.find(
            "listened 10s over 2 scopes: 42 samples on 7 keys, 3 dropped \
                 (findings cover only what was seen — O6) · 2 synthetic-marked"
        )
        .is_ok(),
        "the observation section states window, scope, drops, and synthetic count"
    );
}

/// Persisted preferences (#73) reach the window: the theme button says what
/// you *get*, the zoom reads back as a percentage, and a preferences file that
/// could not be read explains itself in the strip rather than looking like a
/// reset.
#[test]
fn preferences_are_visible_and_a_broken_file_says_so() {
    use zengui::prefs::{Prefs, ThemeChoice};
    use zengui::view::status::{self, Status};

    // The theme choice drives the actual iced theme, not just a label.
    assert_eq!(ThemeChoice::Light.theme(), iced::Theme::Light);
    assert_eq!(ThemeChoice::Dark.theme(), iced::Theme::Dark);

    let mut prefs = Prefs::default();
    prefs.zoom_in();
    assert!(prefs.zoom > 1.0);

    let link = zengui::message::LinkState::Pumping;
    let watched: Vec<String> = vec![];
    let source = status::SliceSource::None;
    let mut ui = simulator::<Message, _, _>(status::strip(Status {
        link: &link,
        base_label: "acme",
        watched: &watched,
        skeleton: None,
        keys_unwatched: 0,
        fetched: None,
        scope_label: "all",
        keys: 0,
        keys_evicted: 0,
        facts_cached: 0,
        facts_evicted: 0,
        totals: (0, 0, 0.0),
        slices: &source,
        seeding: 0,
        seeded_watches: 0,
        seed_totals: (0, 0, 0),
        unreachable: false,
        prefs_note: Some("zengui.toml does not parse (bad) — using defaults"),
        replaying: false,
    }));
    assert!(
        ui.find("preferences: zengui.toml does not parse (bad) — using defaults")
            .is_ok(),
        "an unreadable prefs file must not look like a reset"
    );
}

/// The connection pane (#67) explains scouting rather than labelling it, and
/// carries RFC 09 §0.1's actual semantics: two independent switches, and the
/// reading of an empty result under an isolated session.
#[test]
fn the_connect_pane_states_what_scouting_means() {
    use zengui::view::contexts::{ContextForm, pane};

    let form = ContextForm {
        known: vec!["lab".into(), "prod".into()],
        active: Some("lab".into()),
        ..ContextForm::default()
    };
    let mut ui = simulator::<Message, _, _>(pane(&form, false));
    assert!(
        ui.find(
            "RFC 09 §0.1: multicast scouting and gossip are independent. This toggle is \
             the multicast half only — with it off, a peer still learns about others \
             through gossip over an established link."
        )
        .is_ok(),
        "the two-switch distinction must be on screen, not just in the RFC"
    );
    assert!(
        ui.find(
            "Contexts are shared with zenctl — one file, two explorers \
             (~/.config/zenkey-explorer/config.toml)."
        )
        .is_ok(),
        "the shared store is the feature; say so"
    );

    // A session that reaches nothing is called out where the fix is.
    let mut ui = simulator::<Message, _, _>(pane(&form, true));
    assert!(
        ui.find("this session has no endpoints and multicast scouting is off — it reaches nothing")
            .is_ok()
    );
}

/// The command palette (#75): every action is on screen and every one of
/// them emits a message the UI path also emits — the property that keeps a
/// palette from becoming a second implementation of the app.
#[test]
fn the_palette_offers_the_apps_own_actions_and_the_help_lists_the_real_map() {
    use zengui::view::palette::{Overlay, PaletteState, overlay};

    let contexts = vec!["lab".to_string()];
    let keys = [
        "v1/h-3fa9c2d41b7e/state/sysinfo/health".to_string(),
        "demo/example/foo".to_string(),
    ];

    let mut state = PaletteState::default();
    state.open(Overlay::Commands);
    {
        let element =
            overlay(&state, &contexts, keys.iter().map(String::as_str)).expect("commands overlay");
        let mut ui = simulator::<Message, _, _>(element);
        // The doctor stopped being a place and became an action (#183): its run
        // is a palette command and its verdict lands in the Activity dock.
        assert!(ui.find("run doctor").is_ok());
        assert!(
            ui.find("go to doctor pane").is_err(),
            "a pane that no longer exists must not still be offered"
        );
        assert!(ui.find("activity: doctor").is_ok());
        assert!(ui.find("context: lab").is_ok(), "contexts are offered");
    }

    // The list is bounded, so reaching an action past the first screenful is
    // what typing is for — which is also the fuzzy match's real workload.
    state.query = "ndjson".into();
    {
        let element =
            overlay(&state, &contexts, keys.iter().map(String::as_str)).expect("commands overlay");
        let mut ui = simulator::<Message, _, _>(element);
        assert!(ui.find("export echo as ndjson").is_ok());
        assert!(
            ui.find("go to doctor pane").is_err(),
            "a query narrows; it does not merely reorder"
        );
    }
    state.query.clear();

    // Jump-to-key offers only observed keys, and says so — it must not read
    // as an inventory of the keyspace (O4).
    state.open(Overlay::Keys);
    {
        let element =
            overlay(&state, &contexts, keys.iter().map(String::as_str)).expect("keys overlay");
        let mut ui = simulator::<Message, _, _>(element);
        assert!(ui.find("v1/h-3fa9c2d41b7e/state/sysinfo/health").is_ok());
        assert!(
            ui.find("fuzzy over keys observed so far — nothing here is a guess (O4)")
                .is_ok()
        );
    }

    // The `?` overlay renders the shortcut map itself, so it cannot drift
    // from what `resolve` dispatches.
    state.open(Overlay::Help);
    {
        let element =
            overlay(&state, &contexts, keys.iter().map(String::as_str)).expect("help overlay");
        let mut ui = simulator::<Message, _, _>(element);
        for binding in zengui::shortcuts::map() {
            assert!(
                ui.find(binding.keys).is_ok(),
                "the help overlay omits {:?} ({})",
                binding.keys,
                binding.what
            );
        }
    }

    // Closed means nothing renders.
    state.close();
    assert!(overlay(&state, &contexts, keys.iter().map(String::as_str)).is_none());
}

// ── History pane (#63) ───────────────────────────────────────────────────

/// Build a recorder over a scripted sample stream.
fn recording(key: &str, max_entries: usize, samples: &[(&[u8], bool)]) -> HistoryRecorder {
    let mut rec = HistoryRecorder::new(key, max_entries);
    for (payload, is_delete) in samples {
        rec.observe(&zenkey_fleet::SampleView {
            key: key.to_string(),
            payload: zenoh::bytes::ZBytes::from(payload.to_vec()),
            encoding: "application/json".to_string(),
            kind: if *is_delete {
                zenoh::sample::SampleKind::Delete
            } else {
                zenoh::sample::SampleKind::Put
            },
            timestamp: None,
            stamped_by: None,
            attachment: None,
            priority: zenoh::qos::Priority::DEFAULT,
            congestion_control: zenoh::qos::CongestionControl::DEFAULT,
            reliability: zenoh::qos::Reliability::DEFAULT,
            express: false,
            source: None,
            received: Instant::now(),
        });
    }
    rec
}

/// The pane's first job is saying whether anything is being recorded at all.
/// "Nothing yet" for a key nobody watches is a verdict never obtained (O4).
#[test]
fn the_history_pane_says_why_it_is_empty() {
    use zengui::view::history::{HistoryData, section};

    // Nothing selected.
    let mut ui = simulator::<Message, _, _>(section(HistoryData {
        key: None,
        recorder: None,
        watched: false,
        scroll: (0.0, 600.0),
    }));
    assert!(ui.find("Nothing selected").is_ok());

    // Selected, but no watch covers it — the two must not read alike.
    let rec = recording(REGISTERED, 8, &[]);
    let mut ui = simulator::<Message, _, _>(section(HistoryData {
        key: Some(REGISTERED),
        recorder: Some(&rec),
        watched: false,
        scroll: (0.0, 600.0),
    }));
    assert!(
        ui.find("Not watched — nothing is being recorded").is_ok(),
        "an unwatched key must not render as a quiet one"
    );
    assert!(
        ui.find("watch this key").is_ok(),
        "and the pane offers the watch rather than only complaining"
    );

    // Watched and genuinely quiet: a different sentence, and not a verdict.
    let mut ui = simulator::<Message, _, _>(section(HistoryData {
        key: Some(REGISTERED),
        recorder: Some(&rec),
        watched: true,
        scroll: (0.0, 600.0),
    }));
    assert!(ui.find("No samples yet").is_ok());
    assert!(
        ui.find(
            "The watch is active and nothing has arrived on this key since it \
             was selected. That is not a statement about the bus (RFC 05 §3.1)."
        )
        .is_ok(),
        "silence stays a non-verdict"
    );
}

/// The diff names the field that moved, and says which clock stamped the row.
#[test]
fn the_history_pane_diffs_consecutive_payloads() {
    use zengui::view::history::{HistoryData, section};

    let rec = recording(
        REGISTERED,
        8,
        &[
            (br#"{"value":41.0,"unit":"percent"}"#, false),
            (br#"{"value":42.0,"unit":"percent","inodes":1188}"#, false),
        ],
    );
    let mut ui = simulator::<Message, _, _>(section(HistoryData {
        key: Some(REGISTERED),
        recorder: Some(&rec),
        watched: true,
        scroll: (0.0, 600.0),
    }));
    assert!(
        ui.find("~ value  41.0 → 42.0").is_ok(),
        "the changed field is named with both sides"
    );
    assert!(
        ui.find("+ inodes  1188").is_ok(),
        "an added field is not a change"
    );
    assert!(
        ui.find(
            "times are the publisher's HLC where one was stamped and this observer's \
             arrival otherwise — every row says which"
        )
        .is_ok(),
        "the clock is labelled, never implied"
    );
}

/// A tombstone is authoritative retirement (RFC 04 §1.2), and the put that
/// follows one starts a new value rather than "changing" the deleted one.
#[test]
fn the_history_pane_marks_a_tombstone_as_retirement() {
    use zengui::view::history::{HistoryData, section};

    let script: &[(&[u8], bool)] = &[
        (br#"{"status":"ok"}"#, false),
        (b"", true),
        (br#"{"status":"ok"}"#, false),
    ];

    // Focused on the delete itself.
    let mut rec = recording(REGISTERED, 8, script);
    rec.selected = Some(1);
    {
        let mut ui = simulator::<Message, _, _>(section(HistoryData {
            key: Some(REGISTERED),
            recorder: Some(&rec),
            watched: true,
            scroll: (0.0, 600.0),
        }));
        assert!(ui.find("▸ t-1").is_ok(), "the focused row is marked");
        assert!(
            ui.find("DELETE (tombstone)").is_ok(),
            "a delete must be visibly distinct from an empty put"
        );
        assert!(
            ui.find("retired — an authoritative delete (RFC 04 §1.2), not an empty value")
                .is_ok()
        );
    }

    // Focused on the put after it: a new value, not a change.
    rec.selected = Some(2);
    {
        let mut ui = simulator::<Message, _, _>(section(HistoryData {
            key: Some(REGISTERED),
            recorder: Some(&rec),
            watched: true,
            scroll: (0.0, 600.0),
        }));
        assert!(
            ui.find("new value after retirement — not a change to the previous value")
                .is_ok()
        );
    }
}

/// A payload with no structural form degrades to a byte comparison that says
/// so, instead of inventing field names for bytes it cannot read.
#[test]
fn the_history_pane_falls_back_to_bytes_and_admits_it() {
    use zengui::view::history::{HistoryData, section};

    let rec = recording(
        FOREIGN,
        8,
        &[
            (b"just a plain string", false),
            (b"just a plain STRING", false),
        ],
    );
    let mut ui = simulator::<Message, _, _>(section(HistoryData {
        key: Some(FOREIGN),
        recorder: Some(&rec),
        watched: true,
        scroll: (0.0, 600.0),
    }));
    assert!(
        ui.find("neither sample has a structural form — compared as bytes, not as fields")
            .is_ok()
    );
}

/// The bound is on screen, not implied by a short list (RFC 09 §5.1 O6).
#[test]
fn the_history_pane_counts_what_it_evicted() {
    use zengui::view::history::{HistoryData, section};

    let script: Vec<(Vec<u8>, bool)> = (0..10)
        .map(|i| (format!(r#"{{"value":{i}}}"#).into_bytes(), false))
        .collect();
    let borrowed: Vec<(&[u8], bool)> = script.iter().map(|(p, d)| (p.as_slice(), *d)).collect();
    let rec = recording(REGISTERED, 3, &borrowed);

    let mut ui = simulator::<Message, _, _>(section(HistoryData {
        key: Some(REGISTERED),
        recorder: Some(&rec),
        watched: true,
        scroll: (0.0, 600.0),
    }));
    assert!(
        ui.find("recording since selection · 3 retained · 7 evicted (ring full)")
            .is_ok(),
        "what the bound cost is a fact on screen"
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
        key: FOREIGN,
        facts: None,
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
        key: REGISTERED,
        facts: None,
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

/// The schema cache's escape hatch (#101) is visible, and says which state it
/// is in — a button whose effect you cannot observe is one you cannot trust.
#[test]
fn the_doctor_pane_offers_the_schema_re_ask_and_reports_it() {
    use zengui::doctor::DoctorState;
    use zengui::view::doctor::section;

    let mut state = DoctorState::default();
    {
        let mut ui = simulator::<Message, _, _>(section(&state, ""));
        assert!(ui.find("re-ask schemas").is_ok());
        assert!(
            ui.find(
                "schema cache: kept for the session; a producer that changes its \
                 served set is read with the schemas it had at first contact"
            )
            .is_ok(),
            "the standing behaviour is stated, not left to be discovered"
        );
    }

    state.schemas_forgotten = 1;
    {
        let mut ui = simulator::<Message, _, _>(section(&state, ""));
        assert!(
            ui.find("schema cache cleared 1 time — the next decode asks the bus again")
                .is_ok()
        );
    }
    state.schemas_forgotten = 3;
    {
        let mut ui = simulator::<Message, _, _>(section(&state, ""));
        assert!(
            ui.find("schema cache cleared 3 times — the next decode asks the bus again")
                .is_ok()
        );
    }
}

/// The blob browser's honesty surfaces (#68): the pane must never report a
/// probe it did not run, must never offer a wildcard fetch, and must report a
/// root disagreement rather than resolving it.
mod blob {
    use super::*;
    use std::sync::Arc;
    use zengui::blob::{BlobState, Probe};
    use zengui::view::blob::section;
    use zenkey_fleet::report::{
        BlobAvailability, BlobHolder, BlobList, BlobListSource, BlobManifest, BlobProbeReport,
        BlobTierRow,
    };

    const ID: &str = "01jqz3demo0001";

    fn holder(origin: &str, root: &str) -> BlobHolder {
        BlobHolder {
            origin: origin.into(),
            key: format!("v1/{origin}/@blob/artifact/{ID}"),
            availability: Some(BlobAvailability {
                chunk_count: 4,
                have: 4,
                complete: true,
            }),
            note: None,
            manifest: Some(BlobManifest {
                id: ID.into(),
                filename: Some("demo-bundle.bin".into()),
                total_len: 262_144,
                chunk_size: 65_536,
                chunk_count: 4,
                root: root.into(),
                created_ms: 0,
            }),
            unreadable: None,
            error: None,
        }
    }

    fn probed(state: &mut BlobState, holders: Vec<BlobHolder>, roots: Vec<String>) {
        state.probe = Probe::Done(Arc::new(BlobProbeReport {
            target: format!("artifact/{ID}"),
            tier: "artifact".into(),
            asked: vec![format!("v1/*/@blob/artifact/{ID}/have")],
            not_probed: None,
            answered: holders.len(),
            holders,
            roots,
            declared_by: vec![],
        }));
    }

    /// O4, twice over: an unloaded registry is not "nobody serves blobs", and
    /// an unrun probe is not "nobody holds it".
    #[test]
    fn the_blob_pane_never_reports_what_it_did_not_ask() {
        let state = BlobState::default();
        let mut ui = simulator::<Message, _, _>(section(&state, false));
        assert!(ui.find("no registry loaded").is_ok());
        assert!(ui.find("no probe yet").is_ok());
        assert!(
            ui.find("nothing has been asked — this is \"not asked\", not \"nobody holds it\"")
                .is_ok(),
            "the never-probed state must say why it is empty"
        );
        assert!(
            ui.find("no origin answered").is_err(),
            "an unrun probe must not render as a silent one"
        );
    }

    /// The tier table is a capability claim, and says so — and an unasked
    /// roster reads differently from an empty one.
    #[test]
    fn the_tier_matrix_labels_declaration_not_possession() {
        let state = BlobState {
            list: Some(BlobList {
                tiers: vec![BlobTierRow {
                    producer: "netring".into(),
                    registry_version: "1.1".into(),
                    tier: "artifact".into(),
                    known_tier: true,
                    endpoints: vec!["manifest".into(), "have".into()],
                    algo: None,
                    reference: Some("BlobReference".into()),
                    encoding: None,
                    since: None,
                    description: None,
                    origins: None,
                }],
                source: BlobListSource::RegistryDirs,
                slices_considered: 3,
                slices_without_blob: 2,
            }),
            ..Default::default()
        };
        let mut ui = simulator::<Message, _, _>(section(&state, true));
        assert!(ui.find("netring").is_ok());
        assert!(
            ui.find(
                "a declaration is a capability, never possession — probe below to ask who \
                 actually holds one"
            )
            .is_ok()
        );
        assert!(
            ui.find("origins  — (roster not asked)").is_ok(),
            "an unasked roster must not read as 'no origin serves this'"
        );
    }

    /// #68's acceptance: the pane cannot express a wildcard-origin fetch. It
    /// has no origin input at all, and until one holder is chosen the fetch
    /// control produces no message.
    #[test]
    fn the_blob_pane_names_one_origin_and_offers_no_wildcard() {
        let mut state = BlobState::default();
        state.set_target(ID.into());
        state.dest_input = "/tmp/demo.bin".into();
        state.allow_unpinned = true;
        probed(
            &mut state,
            vec![
                holder("h-aaaaaaaaaaaa", "ab12"),
                holder("h-bbbbbbbbbbbb", "ab12"),
            ],
            vec!["ab12".into()],
        );

        let mut ui = simulator::<Message, _, _>(section(&state, false));
        assert!(ui.find("h-aaaaaaaaaaaa").is_ok());
        assert!(ui.find("h-bbbbbbbbbbbb").is_ok());
        assert!(
            ui.find("choose one holder — a fetch names exactly one origin (RFC 07 §2.5)")
                .is_ok(),
            "an unchosen holder must say why the fetch is not ready"
        );
        // Pressed with nothing selected, the control emits nothing.
        let _ = ui.click("fetch");
        assert!(
            ui.into_messages().next().is_none(),
            "a fetch with no chosen origin must not be dispatchable"
        );

        // Chosen: the button names the single origin it will talk to.
        state.holder = Some(1);
        let mut ui = simulator::<Message, _, _>(section(&state, false));
        assert!(
            ui.find("fetch from h-bbbbbbbbbbbb").is_ok(),
            "the control names the one origin, so 'from where?' is never guessed"
        );
    }

    /// RFC 07 §2.1: the id is a name and the root is the anchor, so two roots
    /// under one id are a finding — the pane reports both and picks neither.
    #[test]
    fn the_blob_pane_flags_disagreeing_roots() {
        let mut state = BlobState::default();
        state.set_target(ID.into());
        probed(
            &mut state,
            vec![
                holder("h-aaaaaaaaaaaa", "ab12"),
                holder("h-cccccccccccc", "cd34"),
            ],
            vec!["ab12".into(), "cd34".into()],
        );
        let mut ui = simulator::<Message, _, _>(section(&state, false));
        assert!(ui.find("2 distinct content roots under one id").is_ok());
        assert!(
            ui.find(
                "the id is a name; the root is the anchor (RFC 07 §2.1). This is a finding, \
                 not a tie-break — pin the root you mean and the fetch will refuse anything \
                 else."
            )
            .is_ok()
        );
    }

    /// An unasked probe must not render as an empty holder list. Since RFC 07
    /// v1.17 every tier has a probe endpoint, so the case left for this render
    /// path is a store algorithm the reference client does not speak — the
    /// fixture models that one.
    #[test]
    fn a_tier_two_probe_says_it_was_not_run() {
        let hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let mut state = BlobState::default();
        state.set_target(format!("store/sha256/{hash}"));
        state.probe = Probe::Done(Arc::new(BlobProbeReport {
            target: format!("store/sha256/{hash}"),
            tier: "store".into(),
            asked: vec![],
            not_probed: Some("the reference client speaks `blake3` only".into()),
            holders: vec![],
            answered: 0,
            roots: vec![],
            declared_by: vec!["logs".into()],
        }));
        let mut ui = simulator::<Message, _, _>(section(&state, false));
        assert!(ui.find("not probed").is_ok());
        assert!(
            ui.find("no origin answered").is_err(),
            "an unasked probe must not read as a silent one"
        );
        assert!(
            ui.find(
                "declared by logs — a registry claim, not a statement that any of them holds \
                 this object"
            )
            .is_ok()
        );
    }
}

/// #107's "the bound is not a silent one": the projection cache's retirement
/// count reaches the strip, and reads as its own number rather than being
/// folded into the key count. Two bounds, two populations, two sentences (O6).
#[test]
fn the_projection_cache_discloses_its_bound() {
    use zengui::prefs::Prefs;
    use zengui::view::status::{self, Status};

    let _ = Prefs::default();
    let link = zengui::message::LinkState::Pumping;
    let watched: Vec<String> = vec![];
    let source = status::SliceSource::None;
    let mut ui = simulator::<Message, _, _>(status::strip(Status {
        link: &link,
        base_label: "acme",
        watched: &watched,
        skeleton: None,
        keys_unwatched: 0,
        fetched: None,
        scope_label: "all",
        keys: 120,
        keys_evicted: 7,
        facts_cached: 118,
        facts_evicted: 312,
        totals: (0, 0, 0.0),
        slices: &source,
        seeding: 0,
        seeded_watches: 0,
        seed_totals: (0, 0, 0),
        unreachable: false,
        prefs_note: None,
        replaying: false,
    }));
    assert!(
        ui.find(status::facts_text(118, 312).as_str()).is_ok(),
        "the cache's bound must be visible, not merely enforced"
    );
    assert!(
        ui.find(status::keys_text(120, 7).as_str()).is_ok(),
        "and the key table's own count is still its own"
    );
    assert_ne!(
        status::facts_text(118, 312),
        status::keys_text(118, 312),
        "the two bounds must not read as one number"
    );
}

/// A quiet cache has nothing to disclose: the key count already states the
/// population, so an untripped bound must not add a line.
#[test]
fn an_untripped_cache_bound_says_nothing() {
    use zengui::view::status::{self, Status};

    let link = zengui::message::LinkState::Pumping;
    let watched: Vec<String> = vec![];
    let source = status::SliceSource::None;
    let mut ui = simulator::<Message, _, _>(status::strip(Status {
        link: &link,
        base_label: "acme",
        watched: &watched,
        skeleton: None,
        keys_unwatched: 0,
        fetched: None,
        scope_label: "all",
        keys: 12,
        keys_evicted: 0,
        facts_cached: 12,
        facts_evicted: 0,
        totals: (0, 0, 0.0),
        slices: &source,
        seeding: 0,
        seeded_watches: 0,
        seed_totals: (0, 0, 0),
        unreachable: false,
        prefs_note: None,
        replaying: false,
    }));
    assert!(ui.find(status::facts_text(12, 0).as_str()).is_err());
}

/// The admin & storage panel's honesty surfaces (#70). The pane exists to draw
/// three distinctions that all look like "nothing here" unless something makes
/// them look different: not swept, swept-and-unreachable, and swept-and-empty.
mod admin {
    use super::*;
    use std::sync::Arc;
    use zengui::admin::{AdminState, AdminSweep};
    use zengui::view::admin::pane;
    use zenkey_fleet::report::StorageList;
    use zenkey_fleet::{Coverage, CoverageRow, DeclaredEntities, RouterInfo, StorageInfo};

    fn sweep(
        routers: Vec<RouterInfo>,
        storages: Vec<StorageInfo>,
        coverage: Vec<CoverageRow>,
        declared: Option<DeclaredEntities>,
        note: Option<&str>,
    ) -> AdminState {
        let mut state = AdminState::default();
        state.finish(
            Ok(Arc::new(AdminSweep {
                routers,
                storage: StorageList { storages, coverage },
                declared,
                coverage_note: note.map(str::to_string),
                topology: zenkey_fleet::TopologyReport {
                    nodes: vec![],
                    edges: vec![],
                    asked: "@/*/*".into(),
                    answered: 0,
                    self_zid: String::new(),
                },
                origins: vec![],
                base: String::new(),
            })),
            "",
        );
        state
    }

    /// #118: the topology caption is the testable surface (the canvas is
    /// find-opaque) — it names the heard-of nodes, the storage hosts, and
    /// this session, and an unanswered admin space stays a reachability
    /// reading, never an empty mesh.
    #[test]
    fn the_topology_caption_carries_the_overlay_facts() {
        let mut state = AdminState::default();
        state.finish(
            Ok(Arc::new(AdminSweep {
                routers: vec![],
                storage: StorageList {
                    storages: vec![storage("s1")],
                    coverage: vec![],
                },
                declared: None,
                coverage_note: None,
                topology: zenkey_fleet::TopologyReport {
                    nodes: vec![
                        zenkey_fleet::TopologyNode {
                            zid: "z1".into(),
                            whatami: "router".into(),
                            version: Some("1.9.0".into()),
                            locators: vec!["tcp/10.0.0.1:7447".into()],
                            answered: true,
                        },
                        zenkey_fleet::TopologyNode {
                            zid: "z2".into(),
                            whatami: "peer".into(),
                            version: None,
                            locators: vec![],
                            answered: false,
                        },
                    ],
                    edges: vec![zenkey_fleet::TopologyEdge {
                        reporter: "z1".into(),
                        peer: "z2".into(),
                        whatami: "peer".into(),
                        links: vec![],
                    }],
                    asked: "@/*/*".into(),
                    answered: 1,
                    self_zid: "z2".into(),
                },
                origins: vec![
                    zenkey_fleet::OriginAttachment {
                        origin: "h-cccccccccccc".into(),
                        session_zid: Some("z2".into()),
                        reporter_zid: "z1".into(),
                        token_key: "v1/h-cccccccccccc/state/demo/alive".into(),
                    },
                    zenkey_fleet::OriginAttachment {
                        origin: "h-dddddddddddd".into(),
                        session_zid: None,
                        reporter_zid: "z1".into(),
                        token_key: "v1/h-dddddddddddd/state/demo/alive".into(),
                    },
                    zenkey_fleet::OriginAttachment {
                        origin: "h-eeeeeeeeeeee".into(),
                        session_zid: Some("z-not-drawn".into()),
                        reporter_zid: "z9".into(),
                        token_key: "v1/h-eeeeeeeeeeee/state/demo/alive".into(),
                    },
                ],
                base: String::new(),
            })),
            "",
        );
        let mut ui = simulator::<Message, _, _>(pane(&state));
        // #131: the origin join's evidence distinction reaches the caption —
        // attached-by-declaration vs reported-only vs outside the mesh.
        assert!(
            ui.find(
                "origins: 1 attached by declaration · 1 reported-only (dotted) · 1 naming a session outside the drawn mesh (listed, not drawn)"
            )
            .is_ok(),
            "the three evidence grades are counted out loud"
        );
        assert!(
            ui.find("drag to pan · scroll to zoom · right-click resets")
                .is_ok(),
            "the viewport controls are discoverable"
        );
        // iced_test's find matches a widget's WHOLE text.
        assert!(
            ui.find("z1  router  ⌂ storage").is_ok(),
            "the answered node rolls, with the storage overlay marking its host"
        );
        assert!(
            ui.find("z2  peer  (heard of)  ← you").is_ok(),
            "heard-of and you-are-here ride the caption"
        );

        // And the unanswered mesh stays a reading.
        let empty = sweep(vec![], vec![], vec![], None, None);
        let mut ui = simulator::<Message, _, _>(pane(&empty));
        assert!(
            ui.find(
                "no admin space answered @/*/* — adminspace.enabled defaults off; this is a \
             reading about reachability, never an empty mesh (RFC 05 §3.1)"
            )
            .is_ok(),
            "zero answers is reachability, not topology"
        );
    }

    fn storage(name: &str) -> StorageInfo {
        StorageInfo {
            zid: "z1".into(),
            name: name.into(),
            key_expr: Some("acme/v1/*/state/**".into()),
            strip_prefix: Some("acme/v1".into()),
            volume: Some("rocksdb".into()),
            raw: serde_json::json!({"volume": "rocksdb"}),
        }
    }

    fn row(producer: &str, path: &str, coverage: Coverage) -> CoverageRow {
        CoverageRow {
            producer: producer.into(),
            path: path.into(),
            ttl_s: Some(60),
            coverage,
        }
    }

    /// O4: never swept must not read as "no routers".
    #[test]
    fn the_admin_pane_never_reports_a_sweep_it_did_not_run() {
        let state = AdminState::default();
        let mut ui = simulator::<Message, _, _>(pane(&state));
        assert!(ui.find("admin space not swept yet").is_ok());
        assert!(
            ui.find(
                "nothing has been asked — this is \"not asked\", not \"no routers\" \
                 (RFC 09 §5.1 O4)"
            )
            .is_ok()
        );
        assert!(
            ui.find(
                "no routers answered @/*/router — a peer-only mesh, or the admin space \
                     is disabled."
            )
            .is_err(),
            "an unrun sweep must not render the swept-and-empty sentence"
        );
    }

    /// #70's second acceptance criterion: a router-less bus shows the honest
    /// empty states, not an error — and above all not a coverage verdict.
    #[test]
    fn a_router_less_bus_says_so_four_ways() {
        let state = sweep(
            vec![],
            vec![],
            vec![],
            None,
            Some(
                "no registry slices resolved, so the declared state families are unknown — \
                 this is \"not asked\", not \"uncovered\" (RFC 09 §5.1 O4). Pass --registry, \
                 or wait for the bus registry (RFC 08 §6).",
            ),
        );
        let mut ui = simulator::<Message, _, _>(pane(&state));
        // Verbatim from the CLI, so the two tools cannot disagree.
        assert!(
            ui.find(
                "no routers answered @/*/router — a peer-only mesh, or the admin space is \
                 disabled."
            )
            .is_ok()
        );
        assert!(
            ui.find(
                "no storages found in the admin space — a peer-only mesh, a router without \
                 the storage manager, or the admin space is disabled."
            )
            .is_ok()
        );
        assert!(ui.find("coverage not judged").is_ok());
        assert!(
            ui.find(
                "declared entities: n/a — nothing answered the admin sweep. zenoh's \
                 adminspace.enabled defaults to false and a peer mesh has none, so this is \
                 \"not available\", never \"nothing declared\" (RFC 09 §5.1 O4)."
            )
            .is_ok()
        );
        // The negative that matters: an unjudged table must never render as an
        // uncovered one.
        assert!(
            ui.find("uncovered").is_err(),
            "coverage that was never judged must not read as uncovered"
        );
    }

    /// A populated sweep: the three coverage verdicts read differently, both
    /// RFC notes are present, and the new storage fields are on screen.
    #[test]
    fn a_populated_sweep_renders_every_verdict_distinctly() {
        let state = sweep(
            vec![RouterInfo {
                zid: "z1".into(),
                version: Some("1.9.0".into()),
                locators: vec!["tcp/127.0.0.1:7447".into()],
                raw: serde_json::Value::Null,
            }],
            vec![storage("latest")],
            vec![
                row("tc", "health", Coverage::Covered("latest@z1".into())),
                row("tc", "config/{iface}", Coverage::Partial("one@z1".into())),
                row("netring", "capture", Coverage::Uncovered),
            ],
            Some(DeclaredEntities::default()),
            None,
        );
        let mut ui = simulator::<Message, _, _>(pane(&state));
        assert!(ui.find("covered by latest@z1").is_ok());
        assert!(ui.find("PARTIAL via one@z1").is_ok());
        assert!(ui.find("uncovered").is_ok());
        // The consequence RFC 09 §2 names, which is the visual #70 asked for.
        assert!(
            ui.find(
                "no storage captures an uncovered family — while its producer is down, a \
                 late joiner GETs nothing: the `latest` storage is the fleet-wide \
                 late-joiner seed (RFC 09 §2)"
            )
            .is_ok()
        );
        // And the counterweight, verbatim from the CLI.
        assert!(
            ui.find(
                "note: an uncovered ttl'd family is not automatically a defect — \
                 volatile-state seeding may ride the advanced-pub/sub cache (RFC 04 §3.5); \
                 storage is authoritative for durable data."
            )
            .is_ok()
        );
        assert!(
            ui.find("strip acme/v1  ·  volume rocksdb").is_ok(),
            "the fields #70 asked for reach the screen"
        );
        assert!(
            ui.find("ttl 60s").is_ok(),
            "the note is unusable without it"
        );
    }

    /// The O4 pair, pinned: "answered and declared none" must not render as
    /// "nothing answered".
    #[test]
    fn an_empty_admin_space_reads_differently_from_an_absent_one() {
        let state = sweep(
            vec![],
            vec![],
            vec![],
            Some(DeclaredEntities::default()),
            None,
        );
        let mut ui = simulator::<Message, _, _>(pane(&state));
        assert!(
            ui.find("declared entities: 0 — the admin space answered and declared none")
                .is_ok()
        );
        assert!(
            ui.find(
                "declared entities: n/a — nothing answered the admin sweep. zenoh's \
                 adminspace.enabled defaults to false and a peer mesh has none, so this is \
                 \"not available\", never \"nothing declared\" (RFC 09 §5.1 O4)."
            )
            .is_err(),
            "an answered sweep must not carry the unreachable sentence"
        );
    }
}

/// The REPLAY banner (issue #74) is unmistakable and carries the capture's
/// own account of itself: the file, its selectors and base, when it was
/// captured, the drop ledger (O6 applied to a file), the transport — and
/// the scrubber's axis names its clock (RFC 09 §5.2's two-clock rule).
#[test]
fn the_replay_banner_says_everything() {
    use zengui::replay::ReplayState;

    let file = [
        r#"{"zrec":1,"selectors":["v1/**"],"base":"acme","captured_at":"2026-08-12T03:00:00Z"}"#,
        r#"{"key":"v1/h-0123456789ab/state/p/a","t":0,"bytes":"MQ=="}"#,
        r#"{"dropped":4}"#,
        r#"{"key":"v1/h-0123456789ab/state/p/b","t":2000000,"bytes":"Mg=="}"#,
    ]
    .join("\n");
    let mut state = ReplayState::load("incident.zrec", file.as_bytes()).expect("load");
    let _ = state.scrub_to(600_000);

    let mut ui = simulator::<Message, _, _>(zengui::view::replay::banner(&state));
    assert!(ui.find("REPLAY").is_ok(), "the mode must be unmistakable");
    assert!(
        ui.find(
            "incident.zrec — v1/** under base \"acme\", captured 2026-08-12T03:00:00Z · 2 row(s)"
        )
        .is_ok(),
        "the capture's own account: file, selectors, base, when, rows"
    );
    assert!(
        ui.find("capture dropped 4 sample(s) — partial view")
            .is_ok(),
        "the drop ledger must reach the banner (O6)"
    );
    assert!(ui.find("exit replay").is_ok());
    assert!(ui.find("live link off").is_ok());
    // The transport moved to the Activity dock's Replay tab (#183). The
    // banner stayed put, and it stayed put for a reason — a mode indicator
    // behind a tab is one that can lie about what the panes are showing.
    assert!(
        ui.find("0.6s / 2.0s (capture clock t)").is_err(),
        "the scrubber is a stream control, not part of the mode banner"
    );

    let mut ui = simulator::<Message, _, _>(zengui::view::replay::scrubber(&state));
    assert!(
        ui.find("0.6s / 2.0s (capture clock t)").is_ok(),
        "the scrubber axis names which clock it plots"
    );
    assert!(ui.find("play").is_ok(), "paused replay offers play");
}

/// While replaying, the status strip must not describe the link it is not
/// pumping (#74): the replay statement wins over any link state.
#[test]
fn the_strip_reports_replay_over_the_link() {
    use zengui::view::status::{self, Status};

    let link = zengui::message::LinkState::Pumping;
    let watched: Vec<String> = vec!["v1/**".to_string()];
    let source = status::SliceSource::None;
    let mut ui = simulator::<Message, _, _>(status::strip(Status {
        link: &link,
        base_label: "acme",
        watched: &watched,
        skeleton: None,
        keys_unwatched: 0,
        fetched: None,
        scope_label: "all",
        keys: 3,
        keys_evicted: 0,
        facts_cached: 3,
        facts_evicted: 0,
        totals: (0, 0, 0.0),
        slices: &source,
        seeding: 0,
        seeded_watches: 0,
        seed_totals: (0, 0, 0),
        unreachable: false,
        prefs_note: None,
        replaying: true,
    }));
    assert!(ui.find("REPLAY — live link off").is_ok());
    assert!(
        ui.find("observing 1 watch").is_err(),
        "a link nobody is pumping must not read as observing"
    );
}

/// The media pane (issue #69) discovers declared streams off the loaded
/// slices (#77), and is honest twice over: a rung nobody can decode is
/// listed as metadata only, and no registry means "not asked" (O4).
#[test]
fn the_media_pane_lists_declared_streams_honestly() {
    use zengui::view::media::{MediaState, section};

    let state = MediaState::default();

    // Not asked ≠ nothing declared.
    let mut ui = simulator::<Message, _, _>(section(&state, None));
    assert!(
        ui.find(
            "no registry loaded — declared streams unknown (not asked, O4); \
             the bus serves them via introspect (RFC 08 §6, v1.16)"
        )
        .is_ok()
    );

    let slice = zenkey::slice::parse_slice(
        r#"
[registry]
version = "1.3"
app = "spray"
convention = 1
[producer]
name = "parallax"
[[media]]
path = "{stream}/preview/png"
encoding = "image/png"
[[media]]
path = "{stream}/video/{codec}/{tier}"
encoding = "video/*"
"#,
    )
    .unwrap();
    let slices = SliceSet::from_slices(vec![slice]);
    let mut ui = simulator::<Message, _, _>(section(&state, Some(&slices)));
    assert!(ui.find("parallax declares:").is_ok());
    assert!(
        ui.find("  {stream}/preview/png (image/png)").is_ok(),
        "the decodable rung is offered"
    );
    assert!(
        ui.find(
            "  {stream}/video/{codec}/{tier} (video/*) — metadata only, no \
             decode story yet"
        )
        .is_ok(),
        "the video rung is listed, never pretended"
    );
}

/// A viewing measures its own facts client-side and says which clock; a
/// frame in a codec we cannot decode is reported, not rendered.
#[test]
fn a_media_viewing_reports_what_it_cannot_render() {
    use zengui::view::media::{MediaState, Viewing, section};

    let mut viewing = Viewing::new("v1/h-0123456789ab/@media/parallax/cam0/video/h264/hi".into());
    let sample = zenkey_fleet::SampleView {
        key: viewing.key.clone(),
        payload: zenoh::bytes::ZBytes::from(vec![0u8; 4096]),
        encoding: "video/h264".into(),
        kind: zenoh::sample::SampleKind::Put,
        timestamp: None,
        stamped_by: None,
        attachment: Some(zenoh::bytes::ZBytes::from(
            br#"{"seq":7,"keyframe":true}"#.to_vec(),
        )),
        priority: zenoh::qos::Priority::InteractiveHigh,
        congestion_control: zenoh::qos::CongestionControl::Drop,
        reliability: zenoh::qos::Reliability::BestEffort,
        express: true,
        source: None,
        received: Instant::now(),
    };
    viewing.on_frame(&sample);
    assert_eq!(viewing.frames, 1);
    assert!(viewing.fps().is_none(), "one frame is not a rate");

    let state = MediaState {
        viewing: Some(viewing),
        ..MediaState::default()
    };
    let mut ui = simulator::<Message, _, _>(section(&state, None));
    assert!(
        ui.find(
            "frame arrived: 4096 B as video/h264 — no decode story for this \
             codec; shown as metadata only (RFC 07 §1: the codec is the \
             wire Encoding, and pretending to render it would be a lie)"
        )
        .is_ok()
    );
    assert!(
        ui.find(r#"frame meta (attachment): {"keyframe":true,"seq":7}"#)
            .is_ok(),
        "the attachment metadata reaches the screen"
    );
}
