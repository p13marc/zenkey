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
        stats.record(k, 8, None, now);
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
    keys.iter()
        .map(|k| {
            let mut f = KeyFacts::project("", k);
            if with_slices {
                f.resolve(&slices);
            }
            (k.to_string(), f)
        })
        .collect()
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
    let mut ui = simulator::<Message, _, _>(tree::pane(
        &flat,
        &facts,
        None,
        &watches,
        &watches,
        tree::Pivot::Chunks,
        "",
        0.0,
        600.0,
    ));

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
    let mut ui = simulator::<Message, _, _>(tree::pane(
        &flat,
        &facts,
        None,
        &watches,
        &watches,
        tree::Pivot::Chunks,
        "",
        0.0,
        600.0,
    ));

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
    let mut ui = simulator::<Message, _, _>(tree::pane(
        &flat,
        &facts,
        None,
        &watches,
        &watches,
        tree::Pivot::Chunks,
        "",
        0.0,
        600.0,
    ));

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
    let facts = FactsIndex::new();
    let watches = BTreeSet::new();
    let mut ui = simulator::<Message, _, _>(tree::pane(
        &flat,
        &facts,
        None,
        &watches,
        &watches,
        tree::Pivot::Chunks,
        "",
        0.0,
        600.0,
    ));

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
    use zengui::view::detail::{DetailData, pane};
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
            source: ValueSource::Storage,
        })));

    // Structural fallback with a declared type: the honest <T?> tag.
    let decoded = (
        Some("TelemetryPoint".to_string()),
        Rendering::Structural(r#"{"value":42.0}"#.to_string()),
    );
    let mut ui = simulator::<Message, _, _>(pane(DetailData {
        key,
        facts: Some(&facts),
        fetched: Some(&fetched),
        decoded: Some(&decoded),
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
    let mut ui = simulator::<Message, _, _>(pane(DetailData {
        key,
        facts: Some(&facts),
        fetched: Some(&none),
        decoded: None,
    }));
    assert!(
        ui.find("no value — asked get, @adv cache, subscribe window — a non-verdict, not proof of absence (RFC 05 §3.1)")
            .is_ok(),
        "silence stays attributed"
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
    use zengui::view::doctor::pane;
    use zenkey_fleet::report::{DoctorFinding, DoctorReport, DoctorSeverity};

    let state = DoctorState::default();
    let mut ui = simulator::<Message, _, _>(pane(&state, ""));
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
    };
    let mut state = DoctorState::default();
    state.finish(Ok(Arc::new(report.clone())));
    let mut ui = simulator::<Message, _, _>(pane(&state, ""));
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
    state.finish(Ok(Arc::new(second)));
    let mut ui = simulator::<Message, _, _>(pane(&state, ""));
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
        totals: (0, 0, 0.0),
        slices: &source,
        seeding: 0,
        seeded_watches: 0,
        seed_totals: (0, 0, 0),
        unreachable: false,
        prefs_note: Some("zengui.toml does not parse (bad) — using defaults"),
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
