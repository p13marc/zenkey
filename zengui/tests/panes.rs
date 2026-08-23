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
use zengui::view::tokens::Spacing;
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

/// The comfortable grid — panes render the same claims at either density
/// (#192), so the honesty tests pin them at the default.
fn sp() -> Spacing {
    Spacing::default()
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

/// #193's acceptance: the five `Registration` states render five *distinct*
/// glyph + word pairs. The glyph comes from the tone type — no call site can
/// omit it or give two states the same one — so colour is structurally never
/// the only carrier.
#[test]
fn the_five_registration_states_render_five_distinct_glyph_word_pairs() {
    use zengui::keyfacts::{Registration, SubjectFacts};
    use zengui::view::tree::{registration_label, tone};

    let states = [
        Registration::Registered(Box::new(SubjectFacts {
            path: "disk/{mount}/used".into(),
            type_name: "TelemetryPoint".into(),
            vars: vec![],
            unit: None,
            qos: None,
            encoding: None,
            ttl_s: None,
            rate: None,
            cardinality: None,
            since: None,
            description: None,
        })),
        Registration::Unregistered,
        Registration::NoSliceForProducer,
        Registration::Unknown,
        Registration::NotApplicable,
    ];

    let pairs: Vec<(&str, &str)> = states
        .iter()
        .map(|r| (tone(r).glyph(), registration_label(r)))
        .collect();
    for (i, a) in pairs.iter().enumerate() {
        for b in &pairs[i + 1..] {
            assert_ne!(
                a, b,
                "two registration states render the same glyph + word pair"
            );
        }
    }

    // …and each pair actually reaches the screen, through the same
    // `tone_badge` every pane uses.
    let badges = iced::widget::Column::with_children(
        states
            .iter()
            .map(|r| zengui::view::kit::tone_badge::<Message>(tone(r), registration_label(r))),
    );
    let mut ui = simulator::<Message, _, _>(badges);
    for &(glyph, word) in &pairs {
        assert!(
            ui.find(glyph).is_ok(),
            "glyph {glyph:?} must reach the screen"
        );
        assert!(ui.find(word).is_ok(), "word {word:?} must reach the screen");
    }
}

/// The registration tri-state must actually reach the screen, and its states
/// must read differently (RFC 09 §5.1 O4).
#[test]
fn the_tree_renders_the_registration_state() {
    let keys = [REGISTERED, UNREGISTERED, FOREIGN];
    let (flat, facts) = render(&keys, true);
    let watches = BTreeSet::new();
    let verdicts = zengui::verdict::VerdictCache::default();
    let mut ui = simulator::<Message, _, _>(tree::pane(tree::TreeData {
        sp: sp(),
        flat: &flat,
        pivot: tree::Pivot::Chunks,
        search: "",
        scroll_y: 0.0,
        viewport_h: 600.0,
        facts: &facts,
        verdicts: &verdicts,
        budgets: None,
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

/// #192's acceptance, from the render side: density moves the grid and the
/// row heights, never the words or their sizes. The same pane at Comfortable
/// and Compact makes the same claims — and the type side is pinned
/// structurally in `view/tokens.rs` (`Spacing` has no font field; every
/// `.size(` is gated onto `font::`, whose constants take no density).
#[test]
fn density_changes_the_grid_never_the_claims() {
    use zengui::prefs::Density;

    let keys = [REGISTERED, UNREGISTERED, FOREIGN];
    let (flat, facts) = render(&keys, true);
    let watches = BTreeSet::new();
    for density in Density::ALL {
        let verdicts = zengui::verdict::VerdictCache::default();
        let mut ui = simulator::<Message, _, _>(tree::pane(tree::TreeData {
            sp: Spacing::of(density),
            flat: &flat,
            pivot: tree::Pivot::Chunks,
            search: "",
            scroll_y: 0.0,
            viewport_h: 600.0,
            facts: &facts,
            verdicts: &verdicts,
            budgets: None,
            watches: tree::Watches {
                mine: &watches,
                seeding: &watches,
            },
            selected: None,
        }));
        for claim in ["registered", "unregistered", "TelemetryPoint"] {
            assert!(
                ui.find(claim).is_ok(),
                "{claim:?} must survive {} density",
                density.label()
            );
        }
    }
}

/// Before a registry is loaded, nothing may be badged either way — that would
/// report a verdict never obtained (RFC 09 §5.1 O4).
#[test]
fn an_unresolved_tree_claims_neither_way() {
    let keys = [REGISTERED];
    let (flat, facts) = render(&keys, false);
    let watches = BTreeSet::new();
    let verdicts = zengui::verdict::VerdictCache::default();
    let mut ui = simulator::<Message, _, _>(tree::pane(tree::TreeData {
        sp: sp(),
        flat: &flat,
        pivot: tree::Pivot::Chunks,
        search: "",
        scroll_y: 0.0,
        viewport_h: 600.0,
        facts: &facts,
        verdicts: &verdicts,
        budgets: None,
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
    let verdicts = zengui::verdict::VerdictCache::default();
    let mut ui = simulator::<Message, _, _>(tree::pane(tree::TreeData {
        sp: sp(),
        flat: &flat,
        pivot: tree::Pivot::Chunks,
        search: "",
        scroll_y: 0.0,
        viewport_h: 600.0,
        facts: &facts,
        verdicts: &verdicts,
        budgets: None,
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
    let verdicts = zengui::verdict::VerdictCache::default();
    let mut ui = simulator::<Message, _, _>(tree::pane(tree::TreeData {
        sp: sp(),
        flat: &flat,
        pivot: tree::Pivot::Chunks,
        search: "",
        scroll_y: 0.0,
        viewport_h: 600.0,
        facts: &facts,
        verdicts: &verdicts,
        budgets: None,
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

/// The Send pane's call mode scaffolds from slices and labels the fanout
/// refusal (issue #60, merged by #184): the visual layer of the three-layer
/// guard.
#[test]
fn the_call_mode_labels_forbidden_fanout() {
    use zengui::view::send::{SendForm, SendMode, pane};
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

    let form = SendForm {
        mode: SendMode::Call,
        producer: Some("netring".into()),
        procedure: Some("capture/trigger".into()),
        target: "*".into(),
        ..SendForm::default()
    };
    let roster = zengui::nodes::NodeRoster::default();
    let mut ui = simulator::<Message, _, _>(pane(&form, Some(&slices), &roster, sp()));
    assert!(
        ui.find("fanout = \"forbidden\" — a fleet (*) target is refused (RFC 05 §2.1)")
            .is_ok(),
        "the refusal must be visible before any send"
    );
    // #234: the `@rpc` surface the pane picked from is finally shown — the
    // call key `service_info` spells, `{origin}` standing for the
    // publishing identity (#211).
    assert!(
        ui.find("→ v1/{origin}/@rpc/netring/capture/trigger")
            .is_ok(),
        "the declared call key is on screen, not left to be reconstructed"
    );
    assert!(
        ui.find("fanout forbidden · idempotent false · encoding — · since —")
            .is_ok(),
        "the declared shape rides the surface"
    );

    // Without a registry the pane says "not asked", not empty dropdowns.
    let empty = SendForm {
        mode: SendMode::Call,
        ..SendForm::default()
    };
    let mut ui = simulator::<Message, _, _>(pane(&empty, None, &roster, sp()));
    assert!(ui.find("No registry loaded").is_ok());
}

/// Silence gets a name (#60's first acceptance): an origin the roster says is
/// alive and running the producer, that did not reply, is listed. RFC 05 §3.1
/// says "no reply" is not one condition — this is the join that says which.
#[test]
fn the_call_mode_names_the_origins_that_did_not_answer() {
    use zengui::view::send::{SendForm, SendMode, pane};
    use zenkey_fleet::report::{CallAnswer, CallReport};

    let mut roster = zengui::nodes::NodeRoster::default();
    roster.seed(&BTreeMap::from([
        ("h-aaaaaaaaaaaa".to_string(), vec!["netring".to_string()]),
        ("h-bbbbbbbbbbbb".to_string(), vec!["netring".to_string()]),
        // A third node that does not run the producer must NOT be blamed.
        ("h-cccccccccccc".to_string(), vec!["sysinfo".to_string()]),
    ]));

    let form = SendForm {
        mode: SendMode::Call,
        producer: Some("netring".into()),
        procedure: Some("introspect".into()),
        target: "*".into(),
        outcome: Some(Ok(CallReport {
            key: "v1/*/@rpc/netring/introspect".into(),
            timeout_s: 5,
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
        ..SendForm::default()
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
    let mut ui = simulator::<Message, _, _>(pane(&form, Some(&slices), &roster, sp()));
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
fn the_call_mode_distinguishes_an_unasked_schema_from_an_empty_one() {
    use zengui::view::send::{SchemaField, SendForm, SendMode, pane};
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

    let unasked = SendForm {
        mode: SendMode::Call,
        producer: Some("netring".into()),
        procedure: Some("capture/start".into()),
        ..SendForm::default()
    };
    {
        let mut ui = simulator::<Message, _, _>(pane(&unasked, Some(&slices), &roster, sp()));
        assert!(
            ui.find("CaptureSpec: schema not asked yet — pick the procedure again once connected to scaffold from it")
                .is_ok(),
            "an unfetched schema must not read as a request with no fields"
        );
    }

    let asked = SendForm {
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
        let mut ui = simulator::<Message, _, _>(pane(&asked, Some(&slices), &roster, sp()));
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

/// The publish mode's three provenances must never look alike (#60/#97):
/// encoded, sent as typed, and sent raw are different facts about what is on
/// the wire, and a user who cannot tell them apart cannot trust any of them.
#[test]
fn the_publish_mode_says_how_the_body_reached_the_wire() {
    use zengui::view::send::{SendForm, pane};
    use zenkey_fleet::BodySource;

    let slices = slices();
    let roster = zengui::nodes::NodeRoster::default();
    let base = SendForm {
        key: "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used".into(),
        body: "{\"value\": 1}".into(),
        encoding_used: Some("application/protobuf".into()),
        source: Some(BodySource::Encoded {
            type_name: "TelemetryPoint".into(),
        }),
        ..SendForm::default()
    };
    {
        let mut ui = simulator::<Message, _, _>(pane(&base, Some(&slices), &roster, sp()));
        assert!(
            ui.find("encoded as TelemetryPoint → application/protobuf")
                .is_ok()
        );
    }

    let as_typed = SendForm {
        source: Some(BodySource::AsTyped),
        encoding_used: None,
        note: Some("sysinfo serves no schema for TelemetryPoint".into()),
        ..base.clone()
    };
    {
        let mut ui = simulator::<Message, _, _>(pane(&as_typed, Some(&slices), &roster, sp()));
        assert!(ui.find("sent as typed → (no encoding set)").is_ok());
        assert!(
            ui.find("sysinfo serves no schema for TelemetryPoint")
                .is_ok(),
            "the engine's note is rendered, not swallowed"
        );
    }

    let raw = SendForm {
        source: Some(BodySource::Raw),
        raw: true,
        ..base.clone()
    };
    let mut ui = simulator::<Message, _, _>(pane(&raw, Some(&slices), &roster, sp()));
    assert!(ui.find("sent raw — bytes verbatim, not encoded").is_ok());
}

/// The publish mode's honesty surfaces: the closed QoS vocabulary, the O4
/// matching badge, and the O6 bounded log.
#[test]
fn the_publish_mode_bounds_its_log_and_never_invents_a_matcher() {
    use zengui::view::send::{LOG_LINES, SendForm, pane};

    let slices = slices();
    let roster = zengui::nodes::NodeRoster::default();
    let mut form = SendForm {
        key: "demo/foreign/key".into(),
        armed: true,
        // Armed, but the status could not be asked: "not asked" (O4).
        matching: None,
        ..SendForm::default()
    };
    {
        let mut ui = simulator::<Message, _, _>(pane(&form, Some(&slices), &roster, sp()));
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
        let mut ui = simulator::<Message, _, _>(pane(&form, Some(&slices), &roster, sp()));
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
        let mut ui = simulator::<Message, _, _>(pane(&form, Some(&slices), &roster, sp()));
        assert!(
            ui.find(format!(
                "send log — {LOG_LINES} shown, 3 dropped (bounded at {LOG_LINES})"
            ))
            .is_ok(),
            "a trimmed log must say how much it trimmed"
        );
    }

    // With no registry the pane says "not asked", never "unregistered".
    let mut ui = simulator::<Message, _, _>(pane(&form, None, &roster, sp()));
    assert!(
        ui.find(
            "no registry loaded — the body cannot be schema-checked, and that is \
             \"not asked\", not \"unregistered\" (O4)"
        )
        .is_ok()
    );
}

/// #184's acceptance: one pane, one form, both modes. The mode strip is on
/// screen whichever mode is showing, and switching is a message the strip
/// itself emits — not a second pane wearing a toggle.
#[test]
fn the_send_pane_offers_both_modes_over_one_form() {
    use zengui::message::PaneMsg;
    use zengui::view::send::{SendForm, SendMode, SendMsg, pane};

    let slices = slices();
    let roster = zengui::nodes::NodeRoster::default();
    let mut form = SendForm::default();
    {
        let mut ui = simulator::<Message, _, _>(pane(&form, Some(&slices), &roster, sp()));
        assert!(ui.find("put / publish").is_ok());
        assert!(
            ui.find("get / call").is_ok(),
            "the other mode is one click away, not another pane"
        );
        ui.click("get / call").expect("the mode tab is a button");
        let messages: Vec<String> = ui.into_messages().map(|m| format!("{m:?}")).collect();
        let want = format!(
            "{:?}",
            Message::Pane(PaneMsg::Send(SendMsg::ModeSelected(SendMode::Call)))
        );
        assert!(
            messages.contains(&want),
            "clicking the tab switches the mode, got: {messages:?}"
        );
    }
    form.mode = SendMode::Call;
    let mut ui = simulator::<Message, _, _>(pane(&form, Some(&slices), &roster, sp()));
    assert!(ui.find("put / publish").is_ok(), "…and the way back too");
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

    // Structural fallback with a declared type: the honest <T?> tag — and
    // the verdict rides the sample now (#164): an undecodable payload under
    // a present schema is `NotValidated(Undecodable)`, worded as itself.
    let decoded = zenkey_fleet::decode::DecodedSample {
        type_name: Some("TelemetryPoint".to_string()),
        rendering: Rendering::Structural(r#"{"value":42.0}"#.to_string()),
        verdict: zenkey::schema::validate::Verdict::NotValidated(
            zenkey::schema::validate::NotValidated::Undecodable,
        ),
        decode_error: Some("wrong wire kind".to_string()),
    };
    let mut ui = simulator::<Message, _, _>(section(DetailData {
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
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
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
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
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
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
    let observed = KeyTreeSnapshot::default();

    let facts_for = |key: &str| {
        let mut f = KeyFacts::project("", key);
        f.resolve(&slices);
        f
    };
    #[allow(clippy::too_many_arguments)]
    fn data<'a>(
        subject: &'a Subject,
        facts: Option<&'a KeyFacts>,
        blob: &'a BlobState,
        media: &'a MediaState,
        slices: &'a SliceSet,
        roster: &'a NodeRoster,
        node_detail: &'a DetailState,
        observed: &'a KeyTreeSnapshot,
    ) -> InspectorData<'a> {
        InspectorData {
            slot: zengui::message::SlotId::FOLLOW,
            sp: sp(),
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
            fields: Box::leak(Box::default()),
            why: Box::leak(Box::default()),
            base: "",
            observed,
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
        &observed,
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
        &observed,
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
        &observed,
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
        &observed,
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
        &observed,
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
        sp: sp(),
        roster: &roster,
        selected: None,
        detail: &DetailState::NotAsked,
        slices: None,
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
        sp: sp(),
        roster: &roster,
        selected: None,
        detail: &DetailState::NotAsked,
        slices: None,
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
        sp: sp(),
        roster: &roster,
        selected: Some("h-3fa9c2d41b7e"),
        detail: &detail,
        slices: None,
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
    let mut ui = simulator::<Message, _, _>(section(&state, "", sp()));
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
        synced: None,
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
    let mut ui = simulator::<Message, _, _>(section(&state, "", sp()));
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
    let mut ui = simulator::<Message, _, _>(section(&state, "", sp()));
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
        synced: None,
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
            field_paths_dropped: 0,
            facts_evicted: 0,
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
    let mut ui = simulator::<Message, _, _>(section(&state, "", sp()));
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
        retention: None,
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
    use zengui::view::contexts::ContextForm;
    use zengui::view::palette::{Overlay, PaletteState, overlay};
    use zengui::view::scope_editor::{ScopeEditorData, ScopeForm};

    let form = ContextForm {
        known: vec!["lab".to_string()],
        ..ContextForm::default()
    };
    let scope_form = ScopeForm::default();
    fn scope(form: &ScopeForm) -> ScopeEditorData<'_> {
        ScopeEditorData {
            scope: zengui::scope::ScopePreset::Everything,
            base: "",
            selectors: &[],
            form,
        }
    }
    let cfg = launch_settings();
    let settings_form = zengui::view::settings::SettingsForm::default();
    fn settings<'a>(
        settings: &'a zengui::config::Settings,
        form: &'a zengui::view::settings::SettingsForm,
    ) -> zengui::view::settings::SettingsData<'a> {
        zengui::view::settings::SettingsData {
            settings,
            form,
            theme: "dark",
            zoom: 1.0,
            echo: (0, 0, 0),
            history: None,
            keys: (0, 0),
        }
    }
    let keys = [
        "v1/h-3fa9c2d41b7e/state/sysinfo/health".to_string(),
        "demo/example/foo".to_string(),
    ];

    let mut state = PaletteState::default();
    state.open(Overlay::Commands);
    {
        let element = overlay(
            &state,
            &form,
            false,
            scope(&scope_form),
            settings(&cfg, &settings_form),
            keys.iter().map(String::as_str),
        )
        .expect("commands overlay");
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
        // Connect stopped being a pane too (#185): the overlay is offered
        // instead, and a tab that is gone must not still be spoken of.
        assert!(ui.find("connect — contexts and endpoints").is_ok());
        assert!(ui.find("go to connect pane").is_err());
    }

    // The list is bounded, so reaching an action past the first screenful is
    // what typing is for — which is also the fuzzy match's real workload.
    state.query = "ndjson".into();
    {
        let element = overlay(
            &state,
            &form,
            false,
            scope(&scope_form),
            settings(&cfg, &settings_form),
            keys.iter().map(String::as_str),
        )
        .expect("commands overlay");
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
        let element = overlay(
            &state,
            &form,
            false,
            scope(&scope_form),
            settings(&cfg, &settings_form),
            keys.iter().map(String::as_str),
        )
        .expect("keys overlay");
        let mut ui = simulator::<Message, _, _>(element);
        assert!(ui.find("v1/h-3fa9c2d41b7e/state/sysinfo/health").is_ok());
        assert!(
            ui.find("fuzzy over keys observed so far — nothing here is a guess (O4)")
                .is_ok()
        );
    }

    // The `?` overlay renders the shortcut map itself — modifier-less
    // chords included (#190) — so it cannot drift from what the app
    // dispatches.
    state.open(Overlay::Help);
    {
        let element = overlay(
            &state,
            &form,
            false,
            scope(&scope_form),
            settings(&cfg, &settings_form),
            keys.iter().map(String::as_str),
        )
        .expect("help overlay");
        let mut ui = simulator::<Message, _, _>(element);
        for binding in zengui::shortcuts::map() {
            assert!(
                ui.find(binding.keys).is_ok(),
                "the help overlay omits {:?} ({})",
                binding.keys,
                binding.what
            );
        }
        // …and nothing else: the hardcoded trailing lines that restated
        // `Ctrl P`, `Ctrl K` and `?` by hand are gone (#190) — the table's
        // own rows for Esc and `?` are what remains.
        assert!(
            ui.find("this list · Esc closes").is_err(),
            "the help must render exactly the table"
        );
        assert!(ui.find("Esc").is_ok(), "Esc is a row of the table now");
    }

    // The Connect overlay is the connection pane, floated (#185) — same
    // form state, same messages, a different surface.
    state.open(Overlay::Connect);
    {
        let element = overlay(
            &state,
            &form,
            false,
            scope(&scope_form),
            settings(&cfg, &settings_form),
            keys.iter().map(String::as_str),
        )
        .expect("connect overlay");
        let mut ui = simulator::<Message, _, _>(element);
        assert!(ui.find("Connection").is_ok(), "the pane renders inside it");
        assert!(
            ui.find("session setup — Esc closes").is_ok(),
            "a modal says how to leave it"
        );
    }

    // Closed means nothing renders.
    state.close();
    assert!(
        overlay(
            &state,
            &form,
            false,
            scope(&scope_form),
            settings(&cfg, &settings_form),
            keys.iter().map(String::as_str)
        )
        .is_none()
    );
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
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
        key: None,
        recorder: None,
        watched: false,
        scroll: (0.0, 600.0),
    }));
    assert!(ui.find("Nothing selected").is_ok());

    // Selected, but no watch covers it — the two must not read alike.
    let rec = recording(REGISTERED, 8, &[]);
    let mut ui = simulator::<Message, _, _>(section(HistoryData {
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
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
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
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
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
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
            slot: zengui::message::SlotId::FOLLOW,
            sp: sp(),
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
            slot: zengui::message::SlotId::FOLLOW,
            sp: sp(),
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
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
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
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
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
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
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
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
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
        let mut ui = simulator::<Message, _, _>(section(&state, "", sp()));
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
        let mut ui = simulator::<Message, _, _>(section(&state, "", sp()));
        assert!(
            ui.find("schema cache cleared 1 time — the next decode asks the bus again")
                .is_ok()
        );
    }
    state.schemas_forgotten = 3;
    {
        let mut ui = simulator::<Message, _, _>(section(&state, "", sp()));
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
            slices_considered: 0,
        }));
    }

    /// O4, twice over: an unloaded registry is not "nobody serves blobs", and
    /// an unrun probe is not "nobody holds it".
    #[test]
    fn the_blob_pane_never_reports_what_it_did_not_ask() {
        let state = BlobState::default();
        let mut ui = simulator::<Message, _, _>(section(&state, false, sp()));
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
        let mut ui = simulator::<Message, _, _>(section(&state, true, sp()));
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

        let mut ui = simulator::<Message, _, _>(section(&state, false, sp()));
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
        let mut ui = simulator::<Message, _, _>(section(&state, false, sp()));
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
        let mut ui = simulator::<Message, _, _>(section(&state, false, sp()));
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
            slices_considered: 2,
        }));
        let mut ui = simulator::<Message, _, _>(section(&state, false, sp()));
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
        retention: None,
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
        retention: None,
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
                            locators_via_links: vec![],
                            answered: true,
                        },
                        zenkey_fleet::TopologyNode {
                            zid: "z2".into(),
                            whatami: "peer".into(),
                            version: None,
                            locators: vec![],
                            locators_via_links: vec![],
                            answered: false,
                        },
                    ],
                    edges: vec![
                        zenkey_fleet::TopologyEdge {
                            reporter: "z1".into(),
                            peer: "z2".into(),
                            whatami: "peer".into(),
                            region: None,
                            links: vec![],
                        },
                        // The same link from the other end: corroboration,
                        // not duplication (`mesh_links`, #234).
                        zenkey_fleet::TopologyEdge {
                            reporter: "z2".into(),
                            peer: "z1".into(),
                            whatami: "router".into(),
                            region: None,
                            links: vec![],
                        },
                    ],
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
        let mut ui = simulator::<Message, _, _>(pane(&state, sp()));
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
        // #234: the engine's `mesh_links` dedup keeps the evidence grade the
        // hand-rolled one threw away, and the caption states it.
        assert!(
            ui.find(
                "1 of 1 link(s) corroborated by both ends — a link only one end mentions is weaker evidence (drawn thinner)"
            )
            .is_ok(),
            "reciprocal reports read as corroboration, not as two links"
        );
        assert!(
            ui.find("copy graphviz (dot)").is_ok(),
            "the render_dot export is offered where the mesh is (#234)"
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
        let mut ui = simulator::<Message, _, _>(pane(&empty, sp()));
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
        let mut ui = simulator::<Message, _, _>(pane(&state, sp()));
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
        let mut ui = simulator::<Message, _, _>(pane(&state, sp()));
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
        let mut ui = simulator::<Message, _, _>(pane(&state, sp()));
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
        let mut ui = simulator::<Message, _, _>(pane(&state, sp()));
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

    let mut ui = simulator::<Message, _, _>(zengui::view::replay::scrubber(&state, sp()));
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
        retention: None,
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
    let mut ui = simulator::<Message, _, _>(section(&state, None, sp()));
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
    let mut ui = simulator::<Message, _, _>(section(&state, Some(&slices), sp()));
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
    let mut ui = simulator::<Message, _, _>(section(&state, None, sp()));
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

// ── Location bar (#185) ──────────────────────────────────────────────────

/// The location bar names the empty base — the RFC v1.6 bus-root deployment
/// is a first-class value, not a missing one — and says when nothing is
/// selected rather than rendering a gap.
#[test]
fn the_location_bar_names_the_empty_base_and_the_empty_selection() {
    use zengui::config::BaseChoice;
    use zengui::message::Subject;
    use zengui::view::location::{self, LocationData};

    let options = [BaseChoice::bus_root(), BaseChoice::new("acme")];
    // The picker's closed face and its rows both draw `BaseChoice`'s
    // `Display`, and a `pick_list` draws that text itself — it is not a
    // `Text` widget, so `iced_test`'s text selector cannot see inside it.
    // The label is pinned at the seam the picker draws from instead; that
    // this is the picker's row type is what `location::breadcrumb`'s
    // signature enforces.
    assert_eq!(
        options[0].to_string(),
        "(empty — keys start at v1/)",
        "the empty base renders its label, never a blank row"
    );
    let mut ui = simulator::<Message, _, _>(location::breadcrumb(LocationData {
        context: None,
        base: "",
        base_options: &options,
        scope: zengui::scope::ScopePreset::Everything,
        observing: false,
        subject: &Subject::None,
    }));
    assert!(
        ui.find("no context").is_ok(),
        "an unnamed context is a state, not an omission"
    );
    assert!(
        ui.find("nothing selected — pick a key in the tree").is_ok(),
        "an empty key trail explains itself"
    );
}

/// Every chunk of the subject reaches the screen, and clicking an ancestor
/// selects that subtree — the acceptance the bar exists for (#185).
#[test]
fn the_location_bar_renders_the_trail_and_an_ancestor_click_selects_its_subtree() {
    use zengui::config::BaseChoice;
    use zengui::message::{Subject, SubjectMsg};
    use zengui::view::location::{self, LocationData};

    let subject = Subject::Key("v1/h-a/telemetry/sysinfo/disk".into());
    let options = [BaseChoice::bus_root()];
    let mut ui = simulator::<Message, _, _>(location::breadcrumb(LocationData {
        context: Some("lab"),
        base: "",
        base_options: &options,
        scope: zengui::scope::ScopePreset::Telemetry,
        observing: false,
        subject: &subject,
    }));
    for chunk in ["v1", "h-a", "telemetry", "sysinfo", "disk"] {
        assert!(
            ui.find(chunk).is_ok(),
            "chunk {chunk:?} should be on screen"
        );
    }
    assert!(
        ui.find("context: lab").is_ok(),
        "the active context is display as well as control"
    );

    ui.click("telemetry")
        .expect("an ancestor chunk is a button");
    let messages: Vec<String> = ui.into_messages().map(|m| format!("{m:?}")).collect();
    let want = format!(
        "{:?}",
        Message::Subject(SubjectMsg::Select(Subject::Prefix(
            "v1/h-a/telemetry".into()
        )))
    );
    assert!(
        messages.contains(&want),
        "clicking an ancestor selects that subtree, got: {messages:?}"
    );
}

// ── The key-expression editor (#187) ─────────────────────────────────────

/// On a preset the editor is read-only truth: the selectors actually
/// resolved by `scope::selectors` — the Deployment preset's explicit
/// @catalog line included (RFC 03 §4 D4) — each with what it cannot see,
/// under the D2 statement, with the fork as the only way to edit.
#[test]
fn the_selector_editor_shows_the_resolved_truth_and_its_blind_spots() {
    use zengui::scope::ScopePreset;
    use zengui::view::scope_editor::{self, ScopeEditorData, ScopeForm};

    let form = ScopeForm::default();
    let mut ui = simulator::<Message, _, _>(scope_editor::pane(ScopeEditorData {
        scope: ScopePreset::Deployment,
        base: "zensight",
        selectors: &[],
        form: &form,
    }));

    // The D2 rule is stated inline — the exact wording, pinned.
    assert!(
        ui.find(scope_editor::D2_RULE).is_ok(),
        "the D2 rule is on screen"
    );

    // The resolved selectors are the ones `scope::selectors` builds — the
    // catalog subtree spelled explicitly, never reachable by the fleet
    // wildcards (D4).
    for sel in ScopePreset::Deployment.selectors("zensight", &[]) {
        assert!(
            ui.find(sel.as_str()).is_ok(),
            "resolved selector {sel:?} not shown"
        );
    }
    assert!(
        ui.find("zensight/v1/@catalog/state/**").is_ok(),
        "the Deployment preset must keep naming the catalog subtree"
    );

    // Every selector names what it cannot see.
    assert!(
        ui.find(format!(
            "  cannot see: {}",
            zengui::scope::blind_spot("zensight/v1/*/telemetry/**")
        ))
        .is_ok(),
        "a fleet selector's blind spot is on screen"
    );
    assert!(ui.find("fork into custom and edit").is_ok());
    assert!(
        ui.find("apply — the scope becomes custom").is_err(),
        "a preset is read-only until forked"
    );
}

/// In custom mode the editor validates per keystroke: an invalid row shows
/// the validator's own words (`$*` names RFC 03 §2), a valid row shows its
/// blind spot, and the apply row is offered.
#[test]
fn the_selector_editor_validates_each_row_as_typed() {
    use zengui::scope::ScopePreset;
    use zengui::view::scope_editor::{self, ScopeEditorData, ScopeForm};

    let form = ScopeForm {
        editing: true,
        rows: vec!["demo/**".into(), "demo/$*/x".into()],
        status: None,
    };
    let active = ["demo/**".to_string()];
    let mut ui = simulator::<Message, _, _>(scope_editor::pane(ScopeEditorData {
        scope: ScopePreset::Custom,
        base: "",
        selectors: &active,
        form: &form,
    }));

    // The valid row carries its blind spot…
    assert!(
        ui.find(format!(
            "  cannot see: {}",
            zengui::scope::blind_spot("demo/**")
        ))
        .is_ok()
    );
    // …and the invalid row carries the validator's verdict, verbatim —
    // which is where RFC 03 §2 reaches the screen.
    let err = zengui::scope::validate_selector("demo/$*/x").unwrap_err();
    assert!(
        ui.find(format!("  {err}")).is_ok(),
        "the row's own error must be beside it"
    );
    assert!(ui.find("apply — the scope becomes custom").is_ok());
    assert!(ui.find("add selector").is_ok());
    assert!(
        ui.find("fork into custom and edit").is_err(),
        "already editing — nothing to fork"
    );
}

// ── The Settings overlay (#188) ──────────────────────────────────────────

/// A `Settings` for the overlay tests, spelled out — the same shape
/// `app.rs`'s `test_app` uses.
fn launch_settings() -> zengui::config::Settings {
    zengui::config::Settings {
        base: String::new(),
        connect: vec![],
        listen: vec![],
        scouting: None,
        zenoh_config: None,
        registry: vec![],
        timeout_secs: 5,
        scope: zengui::scope::ScopePreset::Everything,
        selectors: vec![],
        eager: false,
        echo_lines: 2000,
        history_entries: 200,
        max_keys: 50_000,
    }
}

/// The Settings overlay (#188): every knob it controls is labelled live or
/// on-reconnect, each bound states the cost of raising it — the key table in
/// the status strip's own words — and every `Settings` field it does not
/// control is documented as owned elsewhere.
#[test]
fn the_settings_overlay_labels_live_against_reconnect_and_states_each_cost() {
    use zengui::view::settings::{self, SettingsData, SettingsForm};

    let cfg = launch_settings();
    let mut form = SettingsForm::default();
    form.seed(&cfg);
    let mut ui = simulator::<Message, _, _>(settings::pane(SettingsData {
        settings: &cfg,
        form: &form,
        theme: "dark",
        zoom: 1.0,
        echo: (1200, 34, 5),
        history: Some((80, 3)),
        keys: (120, 7),
    }));

    // The groups, and the live-vs-reconnect split.
    assert!(ui.find("bounds — applied live").is_ok());
    assert!(ui.find("bounds — take effect on reconnect").is_ok());

    // Each bound's cost, in the voice the status strip already uses — the
    // key table literally through `status::keys_text`.
    assert!(
        ui.find(format!(
            "takes effect on reconnect — a larger key table is more memory. \
             now: {}",
            zengui::view::status::keys_text(120, 7),
        ))
        .is_ok(),
        "the key-table bound must state its cost in the strip's words"
    );
    assert!(
        ui.find(
            "applies live — a larger ring is more memory and a longer filter \
             scan. now: 1200 lines held (+34 evicted, 5 lagged)"
        )
        .is_ok(),
        "the echo bound must state memory and the filter scan"
    );
    assert!(
        ui.find(
            "applies live — history keeps whole payloads so it can diff them: \
             the costliest bound per entry. now: 80 entries (+3 evicted)"
        )
        .is_ok()
    );
    // The invariant, stated where the bounds are set.
    assert!(ui.find(settings::BOUND_INVARIANT).is_ok());

    // Eager is labelled, not silently inert.
    assert!(
        ui.find(
            "takes effect on reconnect — the live equivalent is the location \
             bar's observe-scope toggle (#85)"
        )
        .is_ok()
    );

    // The registry names its blast radius and its path.
    assert!(
        ui.find(
            "a registry change re-runs the slice union and can change every \
             registration badge in the tree — applying it takes the same \
             forget path a base change does: every verdict about the old \
             slices is dropped rather than left on screen (O4). To keep it \
             across launches, save it into a context (Connect)."
        )
        .is_ok()
    );

    // Every field the overlay does not control is documented as owned
    // elsewhere — base, session setup, scope.
    assert!(
        ui.find("base: (empty — keys start at v1/) — the location bar's base picker owns it")
            .is_ok()
    );
    assert!(
        ui.find(
            "connect (none), listen (none), scouting (unset — off unless a \
             config file says otherwise), zenoh config (none) — session \
             setup, owned by the Connect overlay (Ctrl+Shift+C); a change \
             there reopens the session"
        )
        .is_ok()
    );
    assert!(
        ui.find(
            "scope (everything, 0 custom selectors) — the location bar's \
             scope picker and its selectors editor own them"
        )
        .is_ok()
    );

    // The apply and the reconnect it labels toward are both offered.
    assert!(ui.find("apply").is_ok());
    assert!(ui.find("reconnect now").is_ok());
}

// ── The engine's projections, consumed (#234) ────────────────────────────

/// A two-subject slice where exactly one subject has traffic, plus a ledger
/// entry — the smallest fleet in which declared and observed diverge.
fn projection_fixture() -> (SliceSet, zengui::nodes::NodeRoster, KeyTreeSnapshot) {
    use zenkey::slice::{DeprecationDecl, RegistrySlice, SubjectDecl};

    let subject = |path: &str| SubjectDecl {
        path: path.into(),
        class: "state".into(),
        type_name: "Health".into(),
        common: None,
        since: None,
        description: None,
        qos: None,
        ttl_s: None,
        unit: None,
        rate: None,
        cardinality: None,
        encoding: None,
    };
    let slice = RegistrySlice {
        version: "1.0".into(),
        app: "demo".into(),
        convention: 1,
        name: "sysinfo".into(),
        service_origin: None,
        description: None,
        subjects: vec![subject("health"), subject("mode")],
        procedures: vec![],
        blob: vec![],
        media: vec![],
        deprecated: vec![DeprecationDecl {
            path: "old/health".into(),
            since: Some("0.9".into()),
            replaced_by: Some("health".into()),
        }],
    };
    let slices = SliceSet::from_slices(vec![slice]);

    let mut roster = zengui::nodes::NodeRoster::default();
    roster.apply_transitions(
        "",
        &[("v1/h-3fa9c2d41b7e/state/sysinfo/alive".to_string(), true)],
        Instant::now(),
    );

    let mut stats = StatsTable::new();
    stats.record(
        "v1/h-3fa9c2d41b7e/state/sysinfo/health",
        8,
        None,
        Instant::now(),
        None,
        None,
    );
    (slices, roster, KeyTreeSnapshot::build(&stats))
}

#[allow(clippy::too_many_arguments)]
fn projection_inspector<'a>(
    subject: &'a zengui::message::Subject,
    facts: Option<&'a KeyFacts>,
    slices: &'a SliceSet,
    roster: &'a zengui::nodes::NodeRoster,
    observed: &'a KeyTreeSnapshot,
    blob: &'a zengui::blob::BlobState,
    media: &'a zengui::view::media::MediaState,
    node_detail: &'a zengui::view::nodes::DetailState,
) -> zengui::view::inspector::InspectorData<'a> {
    zengui::view::inspector::InspectorData {
        slot: zengui::message::SlotId::FOLLOW,
        subject,
        facts,
        fetched: zengui::view::detail::Fetched::NotAsked,
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
        fields: Box::leak(Box::default()),
        why: Box::leak(Box::default()),
        base: "",
        observed,
        sp: sp(),
    }
}

/// #234's headline acceptance: a subject the registry declares and nothing
/// publishes is visible, and distinguishable from one that is publishing.
/// The rows are the engine's `topic_list` — declared, not observed — joined
/// against the tick's tree; the ledger rows ride along (RFC 08 §6), and the
/// presence line carries the `node_rows` slice join.
#[test]
fn the_inspector_marks_a_declared_but_unpublished_subject() {
    use zengui::message::Subject;

    let (slices, roster, observed) = projection_fixture();
    let blob = zengui::blob::BlobState::default();
    let media = zengui::view::media::MediaState::default();
    let node_detail = zengui::view::nodes::DetailState::NotAsked;
    let subject = Subject::Origin("h-3fa9c2d41b7e".into());

    let mut ui = simulator::<Message, _, _>(zengui::view::inspector::pane(projection_inspector(
        &subject,
        None,
        &slices,
        &roster,
        &observed,
        &blob,
        &media,
        &node_detail,
    )));
    assert!(ui.find("declared subjects").is_ok());
    assert!(
        ui.find("sysinfo: 2 declared subject(s) · 1 observed on this origin")
            .is_ok(),
        "declared and observed are counted apart — the registry is the difference"
    );
    assert!(
        ui.find("observed on this origin").is_ok(),
        "the publishing subject says so"
    );
    assert!(
        ui.find("declared — not observed by this session (only watched keys are seen)")
            .is_ok(),
        "the unpublished subject is visible AND worded to this window's coverage (O5), \
         never as proof nothing publishes"
    );
    assert!(
        ui.find("old/health DEPRECATED since 0.9 — replaced by health")
            .is_ok(),
        "the ledger rows ride along — RFC 08 §6's headline buy"
    );
    assert!(
        ui.find("app demo · registry v1.0 (declared)").is_ok(),
        "the presence row carries the node_rows slice join, labelled as declared"
    );
}

/// The type-subject arm (#234): a registered key's payload type is placed in
/// the registry vocabulary (`interface_list`) and its carriers listed
/// (`interface_show`) — who else speaks this type, without a slice scan of
/// the pane's own.
#[test]
fn the_inspector_places_a_registered_type_in_the_vocabulary() {
    use zengui::message::Subject;

    let (slices, roster, observed) = projection_fixture();
    let blob = zengui::blob::BlobState::default();
    let media = zengui::view::media::MediaState::default();
    let node_detail = zengui::view::nodes::DetailState::NotAsked;

    let key = "v1/h-3fa9c2d41b7e/state/sysinfo/health";
    let mut facts = KeyFacts::project("", key);
    facts.resolve(&slices);
    let subject = Subject::Key(key.into());

    let mut ui = simulator::<Message, _, _>(zengui::view::inspector::pane(projection_inspector(
        &subject,
        Some(&facts),
        &slices,
        &roster,
        &observed,
        &blob,
        &media,
        &node_detail,
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

/// #164: the decoded pane renders the verdict in three states, never a
/// boolean — and the two silences keep their own words: `NoRegistry`'s
/// "nobody looked" must not wear `NoSchema`'s "asked, and the type has none"
/// (#246; RFC 09 §5.1 O4).
#[test]
fn the_decoded_pane_renders_three_verdict_states_and_keeps_the_silences_apart() {
    use std::sync::Arc;
    use zengui::view::detail::{DetailData, Fetched, section};
    use zenkey::schema::validate::{NotValidated, Verdict};
    use zenkey_fleet::decode::{DecodedSample, Rendering};
    use zenkey_fleet::{FetchOutcome, FetchedValue, ValueSource};

    let key = "v1/h-3fa9c2d41b7e/state/sysinfo/health";
    let fetched: Result<Arc<FetchOutcome>, String> =
        Ok(Arc::new(FetchOutcome::Value(FetchedValue {
            key: key.to_string(),
            payload: zenoh::bytes::ZBytes::from(br#"{"status":"ok"}"#.to_vec()),
            encoding: "application/json".into(),
            timestamp: None,
            attachment: None,
            source: ValueSource::Storage,
        })));
    let render = |verdict: Verdict| {
        // Leaked so the simulator may outlive the block — a test-only cost,
        // bounded by the four calls below.
        let decoded: &'static DecodedSample = Box::leak(Box::new(DecodedSample {
            type_name: None,
            rendering: Rendering::Structural(r#"{"status":"ok"}"#.to_string()),
            verdict,
            decode_error: None,
        }));
        simulator::<Message, _, _>(section(DetailData {
            slot: zengui::message::SlotId::FOLLOW,
            key,
            facts: None,
            fetched: Fetched::Landed(&fetched),
            decoded: Some(decoded),
            series: None,
            history_entries: None,
            observed: None,
            latency: None,
            sp: sp(),
        }))
    };

    let mut ui = render(Verdict::Valid);
    assert!(ui.find("valid").is_ok(), "checked-and-passed says so");

    let mut ui = render(Verdict::Invalid(vec![
        "/status: not a number".into(),
        "/extra: unexpected".into(),
    ]));
    assert!(ui.find("invalid (2 violations)").is_ok());
    assert!(
        ui.find("violation: /status: not a number").is_ok(),
        "each violation is a sentence with its path"
    );

    // The two silences: distinct badge words AND the full reason sentence.
    let mut ui = render(Verdict::NotValidated(NotValidated::NoRegistry));
    assert!(ui.find("not validated (no registry)").is_ok());
    assert!(
        ui.find("no registry loaded, so no type was looked up")
            .is_ok(),
        "NoRegistry is 'nobody looked', spelled out"
    );
    let mut ui = render(Verdict::NotValidated(NotValidated::NoSchema));
    assert!(ui.find("not validated (no schema served)").is_ok());
    assert!(
        ui.find("no schema served for this type").is_ok(),
        "NoSchema is 'asked, and the type has none', spelled out"
    );
}

/// #164: echo rows badge the *cached* verdict — a lookup, never a decode —
/// and a key never checked renders the third state in words: "not validated
/// (not yet checked)", never a blank that could read as fine and never a
/// borrowed verdict.
#[test]
fn the_echo_rows_badge_cached_verdicts_and_admit_the_unchecked() {
    use std::time::Instant;
    use zengui::echo::EchoRing;
    use zengui::verdict::VerdictCache;
    use zengui::view::echo::{EchoView, section};
    use zenkey::schema::validate::Verdict;
    use zenkey_fleet::SampleView;

    let sample = |key: &str| SampleView {
        key: key.to_string(),
        payload: zenoh::bytes::ZBytes::from(br#"{"v":1}"#.to_vec()),
        encoding: "application/json".into(),
        kind: zenoh::sample::SampleKind::Put,
        timestamp: None,
        stamped_by: None,
        attachment: None,
        priority: zenoh::qos::Priority::Data,
        congestion_control: zenoh::qos::CongestionControl::Drop,
        reliability: zenoh::qos::Reliability::BestEffort,
        express: false,
        source: None,
        received: Instant::now(),
    };
    let mut ring = EchoRing::new(10);
    ring.push(&sample("v1/h-3fa9c2d41b7e/state/sysinfo/health"));
    ring.push(&sample("v1/h-3fa9c2d41b7e/state/sysinfo/unchecked"));

    let mut verdicts = VerdictCache::default();
    verdicts.record(
        "v1/h-3fa9c2d41b7e/state/sysinfo/health",
        Verdict::Invalid(vec!["/v: wrong type".into()]),
    );

    let view = EchoView::new();
    let mut ui = simulator::<Message, _, _>(iced::Element::from(iced::widget::container(section(
        &ring,
        &view,
        None,
        ring.next_seq(),
        (0.0, 600.0),
        &verdicts,
        sp(),
    ))));
    assert!(
        ui.find("invalid (1 violation)").is_ok(),
        "the checked key wears its cached verdict"
    );
    assert!(
        ui.find("not validated (not yet checked)").is_ok(),
        "an unchecked key says so — the third state, never a blank"
    );
    assert!(
        ui.find("verdicts: 1 key checked (each badge is the key's most recently checked sample)")
            .is_ok(),
        "the strip scopes what a badge claims"
    );
}

/// #221: the tree badges an over-budget `{var}` family on its subtree row
/// with the numbers; under-declared draws nothing; a `{path...}` family is
/// exempt and says so — never a silent pass.
#[test]
fn the_tree_badges_the_budget_join() {
    use zengui::budget;
    use zenkey_fleet::stats::StatsTable;

    // A registry that declares a tight budget (2) on the disk family and a
    // rest-variable family beside it.
    let subject = |path: &str, class: &str, cardinality: Option<i64>| zenkey::slice::SubjectDecl {
        path: path.into(),
        class: class.into(),
        type_name: "T".into(),
        common: None,
        since: None,
        description: None,
        qos: None,
        ttl_s: None,
        unit: None,
        rate: None,
        cardinality,
        encoding: None,
    };
    let tight = SliceSet::from_slices(vec![zenkey::slice::RegistrySlice {
        version: "1.0".into(),
        app: "t".into(),
        convention: 1,
        name: "sysinfo".into(),
        service_origin: None,
        description: None,
        subjects: vec![
            subject("disk/{mount}/used", "telemetry", Some(2)),
            subject("log/{path...}", "events", Some(1)),
        ],
        procedures: vec![],
        blob: vec![],
        media: vec![],
        deprecated: vec![],
    }]);

    let over = [
        "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/root/used",
        "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var/used",
        "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/tmp/used",
        "v1/h-3fa9c2d41b7e/events/sysinfo/log/var/log/syslog",
    ];
    let mut stats = StatsTable::new();
    let now = std::time::Instant::now();
    for k in &over {
        stats.record(k, 1, None, now, None, None);
    }
    let badges = budget::badges("", &tight, &KeyTreeSnapshot::build(&stats));

    let (flat, facts) = render(&over, true);
    let watches = BTreeSet::new();
    let verdicts = zengui::verdict::VerdictCache::default();
    let mut ui = simulator::<Message, _, _>(tree::pane(tree::TreeData {
        flat: &flat,
        pivot: tree::Pivot::Chunks,
        search: "",
        scroll_y: 0.0,
        viewport_h: 600.0,
        facts: &facts,
        verdicts: &verdicts,
        budgets: Some(&badges),
        watches: tree::Watches {
            mine: &watches,
            seeding: &watches,
        },
        selected: None,
        sp: sp(),
    }));
    assert!(
        ui.find("over budget: observed 3 of 2 declared").is_ok(),
        "the offending family's subtree carries the numbers"
    );
    assert!(
        ui.find("exempt: rest-variable").is_ok(),
        "a rest-variable family is exempt and says so, never a silent pass"
    );
}

/// #223: the Fields section states its window, its coverage, and every
/// bound's cost — and never-run is not "no fields" (O4).
#[test]
fn the_fields_section_states_its_window_and_its_bounds() {
    use std::sync::Arc;
    use zengui::view::fields::{FieldsState, section};
    use zenkey_fleet::report::{FieldReport, FieldRow};

    // Never run: the section explains its cost instead of claiming absence.
    let state = FieldsState::default();
    let mut ui = simulator::<Message, _, _>(iced::Element::from(iced::widget::container(section(
        &state,
        zengui::message::SlotId::FOLLOW,
        sp(),
    ))));
    assert!(
        ui.find(
            "no field observation yet — the button subscribes to exactly this \
             key for the window, then releases; nothing ambient"
        )
        .is_ok(),
        "never-run states the cost, not an empty table"
    );

    // A landed report: coverage, the path bound's refusals, and the
    // no-registry caveat all render.
    let report = Some(Ok(Arc::new(FieldReport {
        selector: "v1/h-a/telemetry/sysinfo/cpu".into(),
        window_s: 10.0,
        samples: 40,
        keys_seen: 1,
        dropped: 2,
        undocumented: 3,
        registry_loaded: false,
        paths: 512,
        max_paths: 512,
        paths_dropped: 7,
        paths_dropped_examples: vec!["v1/h-a/telemetry/sysinfo/cpu · f99".into()],
        facts_evicted: 0,
        rows: vec![FieldRow {
            key: "v1/h-a/telemetry/sysinfo/cpu".into(),
            path: "temperature_c".into(),
            seen: 40,
            documents: 40,
            kinds: vec!["number".into()],
            changes: 0,
            last_change_s: None,
            min: Some(21.5),
            max: Some(21.5),
            last: Some(21.5),
            values: None,
        }],
        findings: vec![],
    })));
    let state = FieldsState {
        report,
        ..FieldsState::default()
    };
    let mut ui = simulator::<Message, _, _>(iced::Element::from(iced::widget::container(section(
        &state,
        zengui::message::SlotId::FOLLOW,
        sp(),
    ))));
    assert!(
        ui.find(
            "watched v1/h-a/telemetry/sysinfo/cpu for 10s: 40 samples on \
             1 key · 2 dropped · 3 without a structural document"
        )
        .is_ok(),
        "the window and its coverage are stated (O5/O6)"
    );
    assert!(
        ui.find(
            "512 paths tracked (bound 512) · 7 refused for the bound — \
             e.g. v1/h-a/telemetry/sysinfo/cpu · f99"
        )
        .is_ok(),
        "the path table's bound reports and names what it refused"
    );
    assert!(
        ui.find(
            "no registry loaded — declared ttl_s and types are unknown, so \
             field-stuck and field-new are unjudgeable here (O4), not clean"
        )
        .is_ok(),
        "no registry reads as unjudgeable, never as clean"
    );
    assert!(
        ui.find(
            "temperature_c · seen 40/40 · number · unchanged in the window \
             · min 21.5 · max 21.5 · last 21.5"
        )
        .is_ok(),
        "an unchanged path is scoped to the window, not 'never'"
    );
}

/// #214: the Why section renders every rung, and a rung whose input was not
/// fetched reads "not asked" — never "no". The impaired verdict refuses to
/// claim health over questions it could not ask.
#[test]
fn the_why_section_renders_not_asked_and_never_no() {
    use std::sync::Arc;
    use zengui::view::why::{WhyState, section};
    use zenkey_fleet::why::{RungAnswer, WhyInputs, ladder};

    // Never run: the section states the frugal default's cost.
    let state = WhyState::default();
    let mut ui = simulator::<Message, _, _>(iced::Element::from(iced::widget::container(section(
        &state,
        zengui::message::SlotId::FOLLOW,
        sp(),
    ))));
    assert!(
        ui.find(
            "not asked yet — \"why?\" runs the silence ladder on this key: \
             control-plane sweeps and one bounded GET, no subscriber \
             (RFC v1.18 frugality)"
        )
        .is_ok(),
    );

    // A ladder over nothing fetched: rungs degrade to NotAsked, and the
    // report is impaired rather than healthy.
    let report = ladder(&WhyInputs {
        base: "",
        key: "v1/h-3fa9c2d41b7e/state/sysinfo/health",
        slices: None,
        roster: None,
        entities: None,
        admin_answered: None,
        storages: None,
        stored: None,
        wire: None,
    });
    assert!(
        report
            .rungs
            .iter()
            .any(|r| matches!(r.answer, RungAnswer::NotAsked)),
        "setup: some rung must be NotAsked"
    );
    let state = WhyState {
        report: Some(Ok(Arc::new(report))),
        ..WhyState::default()
    };
    let mut ui = simulator::<Message, _, _>(iced::Element::from(iced::widget::container(section(
        &state,
        zengui::message::SlotId::FOLLOW,
        sp(),
    ))));
    assert!(
        ui.find("not asked").is_ok(),
        "an unfetched input renders 'not asked', never 'no'"
    );
    assert!(
        ui.find("wire-heard").is_ok(),
        "every rung renders, the unasked ones included"
    );
    assert!(
        ui.find(
            "no cause established, and the observation was impaired — \
             \"healthy\" cannot be claimed over questions that could not be \
             asked"
        )
        .is_ok(),
        "impaired refuses to over-claim"
    );
}
