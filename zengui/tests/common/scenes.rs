//! The whole-window scenes the shots harness renders (#532).
//!
//! Each scene is a function of the theme alone, built through the public
//! message API, so the same list renders on `main` and on a branch — which
//! is what makes a before/after pair a comparison rather than two pictures.
//! Each names a *landmark*: a string that must be on screen, so the CI half
//! of the harness proves the scene still means what its name says.

use std::sync::Arc;

use zengui::app::Zengui;
use zengui::message::{
    ActivityTab, ChromeMsg, Message, PaneMsg, RightPane, Subject, SubjectMsg, WorkspaceMsg,
};
use zengui::prefs::{Density, LayoutPreset, Prefs, ThemeChoice};
use zengui::view::palette::{Overlay, PaletteMsg};

use super::{
    BYTES, DROPS, ETH0, FRAMES, HEALTH, LATENCY, POSITION, TX_BYTES, USED, World, send, window,
};

pub struct Scene {
    pub name: &'static str,
    /// Logical size; the PNG is twice this (`iced_test` renders at 2×).
    pub size: (f32, f32),
    pub build: fn(ThemeChoice) -> Zengui,
    /// Must be on screen, or the scene is not the scene it is named for.
    pub landmark: &'static str,
}

pub const DESKTOP: (f32, f32) = (1440.0, 900.0);

pub const ALL: &[Scene] = &[
    Scene {
        name: "empty",
        size: DESKTOP,
        build: empty,
        landmark: "nothing selected — pick a key in the tree",
    },
    Scene {
        name: "explore",
        size: DESKTOP,
        build: explore,
        landmark: "Detail",
    },
    Scene {
        name: "watch",
        size: DESKTOP,
        build: watch,
        landmark: "publish log",
    },
    Scene {
        name: "diagnose",
        size: DESKTOP,
        build: diagnose,
        // A zk2 check id (#614, FL9): the spray's phantom, found.
        landmark: "token-missing",
    },
    Scene {
        name: "workbench-send",
        size: DESKTOP,
        build: workbench_send,
        landmark: "put / publish",
    },
    Scene {
        name: "workbench-nodes",
        size: DESKTOP,
        build: workbench_nodes,
        // An address up, read from zk2 presence (#614, FL9): `robot1/nav`
        // selected, its instance and descriptor, and the health reading
        // its selection asked for.
        landmark: "robot1/nav: up",
    },
    Scene {
        name: "workbench-admin",
        size: DESKTOP,
        build: workbench_admin,
        landmark: "open the mesh tool",
    },
    Scene {
        name: "workbench-mesh",
        size: DESKTOP,
        build: workbench_mesh,
        landmark: "drag to pan · scroll to zoom · right-click resets",
    },
    // The Config tool on config.v1 (#768, CD): `lab/rf0` read, a reach
    // change on `radio` pending and applied, its deadline the owner's
    // instant and the countdown read against this host's clock.
    Scene {
        name: "workbench-config",
        size: DESKTOP,
        build: workbench_config,
        landmark: "pending change 00000000000000a1-3 on radio",
    },
    // A sensitive group (§2.15): the pin is write-only — no value in its
    // row, a masked field never prefilled, its event edit redacted.
    Scene {
        name: "workbench-config-sensitive",
        size: DESKTOP,
        build: workbench_config_sensitive,
        landmark: "write-only",
    },
    // A set refused in the form (§2.5): `duty` changed while the reach
    // change is pending and not joined — the owner's own `busy`, and
    // nothing sent.
    Scene {
        name: "workbench-config-refused",
        size: DESKTOP,
        build: workbench_config_refused,
        landmark: "refused here — nothing was sent",
    },
    Scene {
        name: "traffic",
        size: DESKTOP,
        build: traffic,
        landmark: "DROPPED",
    },
    // A mode, not an alarm (#544). (A recording needs a live monitor, which
    // no scene has — `app_tests` pins its status instead.)
    Scene {
        name: "replay",
        size: DESKTOP,
        build: replay,
        landmark: "REPLAY",
    },
    Scene {
        name: "palette",
        size: DESKTOP,
        build: palette,
        landmark: "run doctor",
    },
    // The palette ranked (#558): a typed query, each row naming its section.
    Scene {
        name: "palette-query",
        size: DESKTOP,
        build: palette_query,
        landmark: "zoom in",
    },
    Scene {
        name: "keys",
        size: DESKTOP,
        build: keys,
        landmark: "fuzzy over keys observed so far — nothing here is a guess (O4)",
    },
    // The contract skeleton in the tree (#614, FL6b): a templated chunk is
    // one only the declared keyspace can draw.
    Scene {
        name: "skeleton",
        size: DESKTOP,
        build: skeleton,
        landmark: "{iface}",
    },
    Scene {
        name: "help",
        size: DESKTOP,
        build: help,
        landmark: "Shortcuts",
    },
    Scene {
        name: "settings",
        size: DESKTOP,
        build: settings,
        landmark: "apply",
    },
    Scene {
        name: "connect",
        size: DESKTOP,
        build: connect,
        landmark: "isolated-verification preset",
    },
    Scene {
        name: "compact-watch",
        size: DESKTOP,
        build: compact_watch,
        landmark: "publish log",
    },
    Scene {
        name: "narrow",
        size: (1024.0, 700.0),
        build: explore,
        landmark: "Detail",
    },
    // A zk2 call (#614, FL10): `nav.v2`'s `set_origin` at `robot1/nav`, its
    // request form read from the contract, and the value it answered.
    Scene {
        name: "workbench-call",
        size: DESKTOP,
        build: workbench_call,
        landmark: "nav.v2.SetOriginRequest fields: origin: nav.v2.Origin",
    },
    // The publish log in the Watch layout's dock (#614, FL10): a P3
    // refusal, a foreign publish, a value and a silence, each its own line.
    Scene {
        name: "publish-log",
        size: DESKTOP,
        build: publish_log,
        landmark: "send log — 4 entries",
    },
    // A version-2 capture replayed (#614, FL10b): its rows named v1 QoS
    // profiles, which zk2 does not read — "QoS not recorded", in the banner
    // and in the Inspector, where FL6b showed default axes.
    Scene {
        name: "replay-v2",
        size: DESKTOP,
        build: replay_v2,
        landmark: "QoS not recorded — its rows name v1 profiles, which zk2 does not read",
    },
    // The owner silent and an archive's answer beside it (#614, FL8a):
    // last-known, by an explicit button, never shown as current (S5, S6).
    Scene {
        name: "inspector-last-known",
        size: DESKTOP,
        build: inspector_last_known,
        landmark: "last-known from ground/archive — not current (S5, S6)",
    },
    Scene {
        name: "inspector-qos-mismatch",
        size: DESKTOP,
        build: inspector_qos_mismatch,
        // The axis named with both sides (#614, FL12): the one scene whose
        // newest sample rode other axes than its contract declares.
        landmark: "congestion: declared block, observed drop",
    },
    // telemetry.v1 in the Inspector (#819, TD). A Point counter, unit `1`,
    // its epoch in the Point: the plotted leaf is `value.value`, its §2.11
    // rate in `1/s`, and the owner's restart — a new epoch — a reset with a
    // gap and no rate across it.
    Scene {
        name: "inspector-metric-counter",
        size: DESKTOP,
        build: metric_counter,
        landmark: "1 reset: 1 in-band (another epoch), 0 assumed (a decrease no epoch places)",
    },
    // A bare counter in `By` going backwards within its in-band epoch (the
    // Epoch attachment): `decreased`, the finding, never a reset.
    Scene {
        name: "inspector-metric-decreased",
        size: DESKTOP,
        build: metric_decreased,
        landmark: "31500 → 9000",
    },
    // A bare counter with no unit and no epoch: its unit unknown, never
    // dimensionless, and its drop a reset assumed — so no decrease could
    // have been found, which is said.
    Scene {
        name: "inspector-metric-unit-unknown",
        size: DESKTOP,
        build: metric_unit_unknown,
        landmark: "unknown (§2.5)",
    },
    // A Point histogram in `s`: its mean plotted, the one number of a
    // histogram in its member's unit; no rate of a histogram.
    Scene {
        name: "inspector-metric-histogram",
        size: DESKTOP,
        build: metric_histogram,
        landmark: "read by telemetry.v1 over the recorded history: the Point's value.value: its \
                   sum over its count, the one number of a histogram in its member's unit \
                   (§2.5, §2.6)",
    },
    // The histogram's newest Point tagged a gauge: it does not conform, and
    // the Inspector says why in the profile's words.
    Scene {
        name: "inspector-metric-tag-disagrees",
        size: DESKTOP,
        build: metric_tag_disagrees,
        landmark: "telemetry.v1 §2.6 tag_disagrees: the Point's tag differs from the declared kind",
    },
    // The same verdict in the Activity dock: echo pinned to the key, its
    // badge counting the violation and its strip naming it.
    Scene {
        name: "watch-metric-invalid",
        size: DESKTOP,
        build: watch_metric_invalid,
        landmark: "invalid · zk2/lab/meter/meter.v1/stream/latency: telemetry.v1 §2.6 \
                   tag_disagrees: the Point's tag differs from the declared kind",
    },
    // zenoh-modem's units (#819's "done when"): a bare JSON integer that is
    // a counter in `By` by its declaration alone, its rate in `By/s`.
    Scene {
        name: "inspector-modem-units",
        size: DESKTOP,
        build: modem_units,
        landmark: "By · declared",
    },
];

fn prefs(theme: ThemeChoice) -> Prefs {
    Prefs {
        theme,
        ..Prefs::default()
    }
}

/// A window on a live-looking bus: contracts loaded and presence read
/// (#614, FL6a), the pump running, a few seconds of traffic, the tree
/// opened down to the leaves.
fn live(prefs: Prefs) -> Zengui {
    live_world(prefs).0
}

/// [`live`], with the world still publishing, for a scene that adds a tick.
fn live_world(prefs: Prefs) -> (Zengui, World) {
    let mut app = window(prefs);
    let mut world = World::new();
    world.connect(&mut app);
    world.ticks(&mut app, 4);
    for key in [
        USED,
        HEALTH,
        "v1/h-3fa9c2d41b7e/telemetry/sysinfo/system/load",
        "v1/h-9c04d2e1a7f3/telemetry/sysinfo/no/such/thing",
        "demo/example/foo",
        POSITION,
        ETH0,
    ] {
        send(
            &mut app,
            Message::Workspace(WorkspaceMsg::Reveal(key.into())),
        );
    }
    // Selected before the rest of the traffic, so the history ring and the
    // series fill the way they would for a user who clicked early. A zk2
    // state key the lens resolves (#614, FL6a), JSON with numbers in it:
    // the Inspector has a contract decode to show and a leaf to plot.
    select(&mut app, &world, ETH0);
    world.ticks(&mut app, 30);
    world.verdicts(&mut app);
    (app, world)
}

/// The body `host-a/tc` answers for `eth0`: a valid `json:NetworkInterface`.
const ETH0_BODY: &[u8] = br#"{"name":"eth0","index":2,"namespace":"default","is_up":true,"is_oper_up":true,"has_tc_qdisc":false,"interface_type":"ether","addresses":["10.0.4.7/24"],"link_speed_mbps":1000}"#;

/// Point the workspace at a state key and answer the read it issues (#614,
/// FL8a): S4's owner GET, its row rendered through the contract and
/// checked, as the window's own task lands it.
fn select(app: &mut Zengui, world: &World, key: &str) {
    send(
        app,
        Message::Subject(SubjectMsg::Select(Subject::Key(key.into()))),
    );
    let answer = world.state_read(key, Some(ETH0_BODY), None);
    send(
        app,
        Message::Subject(SubjectMsg::ValueFetched(
            key.into(),
            Ok(Arc::new(zengui::value::ValueRead::Owner(answer))),
        )),
    );
}

/// A replay's recorded row for `key` checked through the replay's offline
/// lens, as the window's own task lands it (#614, FL8a).
fn recorded_check(app: &mut Zengui, replay: u64, key: &str, body: &[u8], encoding: &str) {
    let lens = zengui::replay::ReplayLens::offline("", Some(Arc::clone(&super::contracts().set)));
    let checked = lens.lens().check(
        key,
        zenkey_fleet::Member::Type,
        (!encoding.is_empty()).then_some(encoding),
        body,
    );
    send(
        app,
        Message::Subject(SubjectMsg::RecordedDecoded(
            replay,
            key.into(),
            Arc::new(zengui::value::DecodedValue::new(checked, body)),
        )),
    );
}

fn preset(app: &mut Zengui, p: LayoutPreset) {
    send(app, Message::Workspace(WorkspaceMsg::LayoutPreset(p)));
}

fn empty(theme: ThemeChoice) -> Zengui {
    window(prefs(theme))
}

fn explore(theme: ThemeChoice) -> Zengui {
    let mut app = live(prefs(theme));
    preset(&mut app, LayoutPreset::Explore);
    app
}

fn watch(theme: ThemeChoice) -> Zengui {
    watching(prefs(theme))
}

fn traffic(theme: ThemeChoice) -> Zengui {
    use zengui::prefs::DockRole;
    let mut app = watching(prefs(theme));
    send(
        &mut app,
        Message::Workspace(WorkspaceMsg::ActivityTab(ActivityTab::Traffic)),
    );
    // The tab given the window: the ranking sits under the tiles and the
    // chart, and a 40% dock would show only those.
    for role in [DockRole::Locator, DockRole::Inspector] {
        send(
            &mut app,
            Message::Workspace(WorkspaceMsg::DockToggled(role)),
        );
    }
    app
}

fn replay(theme: ThemeChoice) -> Zengui {
    use zengui::replay::{LoadedReplay, ReplayState};
    use zengui::view::replay::ReplayMsg;
    // A version-3 capture (#614, FL6b): what `**` excluded stated in the
    // header, each row's wire axes in `qos_axes`.
    let file = [
        r#"{"zrec":3,"selectors":["**"],"base":"","captured_at":"2026-10-05T03:00:00Z","excluded":["@stream","@state","@op","@zk","@adv"]}"#,
        r#"{"key":"zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0","t":0,"bytes":"eyJuYW1lIjoiZXRoMCIsImluZGV4IjoyLCJpc191cCI6dHJ1ZX0=","encoding":"application/json","qos_axes":"data/block/reliable"}"#,
        r#"{"key":"demo/example/foo","t":500000,"bytes":"aGVsbG8gIzA=","encoding":"text/plain","qos_axes":"data/drop/best_effort"}"#,
        r#"{"dropped":4}"#,
        r#"{"key":"zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0","t":2000000,"bytes":"eyJuYW1lIjoiZXRoMCIsImluZGV4IjoyLCJpc191cCI6ZmFsc2V9","encoding":"application/json","qos_axes":"data/block/reliable"}"#,
    ]
    .join("\n");
    let state = ReplayState::load("incident.zrec", file.as_bytes()).expect("a valid capture");
    let id = state.id();
    let mut app = watching(prefs(theme));
    send(
        &mut app,
        Message::Workspace(WorkspaceMsg::ActivityTab(ActivityTab::Replay)),
    );
    send(
        &mut app,
        Message::Workspace(WorkspaceMsg::Replay(ReplayMsg::Loaded(
            "incident.zrec".into(),
            Ok(LoadedReplay::new(state)),
        ))),
    );
    // The selected key's value is the capture's first row now (#614,
    // FL8a), checked through the replay's own lens — the bus asked nothing.
    recorded_check(
        &mut app,
        id,
        ETH0,
        br#"{"name":"eth0","index":2,"is_up":true}"#,
        "application/json",
    );
    app
}

/// The tree found down to `host-a/tc`'s `tc.netif.v1` (#614, FL6b): every
/// resource its contract declares, placed by presence before a byte of it
/// arrives — the observed interfaces beside the quiet resources nobody
/// published, with their templates symbolic, and the declared operation,
/// which offers no watch.
fn skeleton(theme: ThemeChoice) -> Zengui {
    let mut app = live(prefs(theme));
    send(
        &mut app,
        Message::Workspace(WorkspaceMsg::TreeSearchChanged("host-a/tc/tc.netif".into())),
    );
    app
}

fn compact_watch(theme: ThemeChoice) -> Zengui {
    watching(Prefs {
        density: Density::Compact,
        ..prefs(theme)
    })
}

/// The Watch layout with echo showing the whole stream rather than the
/// selected key alone, so the rows have every anatomy to draw.
fn watching(prefs: Prefs) -> Zengui {
    use zengui::view::echo::EchoMsg;
    let mut app = live(prefs);
    preset(&mut app, LayoutPreset::Watch);
    send(
        &mut app,
        Message::Workspace(WorkspaceMsg::ActivityTab(ActivityTab::Echo)),
    );
    send(
        &mut app,
        Message::Pane(PaneMsg::Echo(EchoMsg::FollowSubjectToggled)),
    );
    app
}

/// The Diagnose layout on a zk2 doctor run (#614, FL9): two findings — the
/// spray's phantom, an interface token with no instance token, and a router
/// storage answering on owners' state (S4) — every other check's verdict
/// in its own pole, the deep checks not asked.
fn diagnose(theme: ThemeChoice) -> Zengui {
    use zengui::view::doctor::DoctorMsg;
    use zenkey_fleet::report::{
        Asked, CheckId, CheckReport, DoctorFinding, DoctorHealth, DoctorPresence, DoctorReport,
        DoctorScope, DoctorSeverity,
    };

    let mut app = live(prefs(theme));
    preset(&mut app, LayoutPreset::Diagnose);
    let finding = |check, subject: &str, evidence: &str| DoctorFinding {
        severity: DoctorSeverity::Error,
        check,
        subject: subject.into(),
        evidence: evidence.into(),
    };
    let clean = |id: CheckId| -> &'static str {
        match id {
            CheckId::SplitBrain => "no interface held by two instances in both reads",
            CheckId::BindingUnsatisfied => "no descriptor requires a role",
            CheckId::ContractDrift => "one revision per interface",
            CheckId::ContractUnavailable => "2 revisions named, 2 held verified",
            CheckId::DescriptorInvalid => "3 descriptors pass the descriptor check",
            CheckId::PresenceOverBudget => "7 tokens in the presence domain, under 10000",
            CheckId::ArchiveUnaligned => "no archive.v1 provider in presence",
            CheckId::ShmMemlockLow => "RLIMIT_MEMLOCK is unlimited",
            CheckId::AdminUnreachable => "1 router answered the admin space",
            CheckId::RouterVersionSkew => "1 router, zenoh 1.10.1",
            CheckId::HostidDuplicate => "no instance lists hostid.v1",
            _ => "no present service lists health.v1 (§2.7)",
        }
    };
    let checks = CheckId::ALL
        .into_iter()
        .map(|id| match id {
            CheckId::TokenMissing => CheckReport::of(
                id,
                vec![finding(
                    id,
                    "robot9/phantom@00000000000000aa",
                    "holds an interface token for nav.v2 and no instance token, in both reads \
                     2 s apart",
                )],
                vec![],
                "",
            ),
            CheckId::StorageOnState => CheckReport::of(
                id,
                vec![finding(
                    id,
                    "state@z1",
                    "captures `**`, which answers on owners' state keys \
                     (`zk2/*/*/*/state/**`): the owner is authoritative for its state, and \
                     last-known state is an archive.v1's (§4.4)",
                )],
                vec![],
                "",
            ),
            id if id.reads_data() => CheckReport::not_asked(id),
            id => CheckReport::of(id, vec![], vec![], clean(id)),
        })
        .collect();
    let report = DoctorReport {
        scope: DoctorScope {
            namespace: String::new(),
            presence: Asked::Asked(DoctorPresence {
                selector: "zk2/*/*/@zk/**".into(),
                complete: true,
                grace_s: 2.0,
                services: 3,
                instances: 3,
                tokens: 7,
                undescribed: 0,
                revisions: 2,
                held: 2,
            }),
            routers: Asked::Asked(1),
            health: Asked::Asked(DoctorHealth {
                selector: "zk2/*/*/health.v1/state/**".into(),
                services: 0,
                clocks_synced: false,
            }),
            config: Asked::NotAsked,
        },
        checks,
        unobservable: None,
    };
    send(
        &mut app,
        Message::Pane(PaneMsg::Doctor(DoctorMsg::Done(
            zengui::doctor::DoctorRun {
                report: Arc::new(report),
                namespace: String::new(),
            },
        ))),
    );
    app
}

fn workbench(theme: ThemeChoice, pane: RightPane) -> Zengui {
    let mut app = live(prefs(theme));
    send(
        &mut app,
        Message::Workspace(WorkspaceMsg::PaneSelected(pane)),
    );
    app
}

/// The Send pane's publish form on a foreign key (#614, FL10): bytes as
/// typed, zenoh's default axes said as such, and the one-shot publish it
/// sent — what went on the wire, and its line in the log.
fn workbench_send(theme: ThemeChoice) -> Zengui {
    let mut app = workbench(theme, RightPane::Send);
    publish_foreign(&mut app);
    app
}

fn send_msg(app: &mut Zengui, msg: zengui::view::send::SendMsg) {
    send(app, Message::Pane(PaneMsg::Send(msg)));
}

/// Type a foreign publish and land the one-shot it sends, as the write task
/// would answer it.
fn publish_foreign(app: &mut Zengui) {
    use zengui::view::send::SendMsg;
    send_msg(app, SendMsg::KeyChanged("demo/example/foo".into()));
    send_msg(app, SendMsg::BodyChanged("hello from zengui".into()));
    send_msg(app, SendMsg::EncodingChanged("text/plain".into()));
    send_msg(
        app,
        SendMsg::Ready(Ok(Arc::new(zengui::message::PublishOutcome {
            key: "demo/example/foo".into(),
            bytes: Arc::new(b"hello from zengui".to_vec()),
            qos: zenkey_fleet::WireQos::DEFAULT,
            encoding: Some("text/plain".into()),
            publication: None,
            matching: Some(true),
            attachment: None,
        }))),
    );
}

/// `nav.v2`'s `set_origin`, as the call answered it.
fn set_origin_report(
    answer: zenkey_fleet::report::OperationAnswer,
) -> zenkey_fleet::report::OperationReport {
    zenkey_fleet::report::OperationReport {
        address: "robot1/nav".into(),
        iface: "nav.v2".into(),
        fingerprint: super::contracts()
            .set
            .of_iface(&"nav.v2".parse().expect("an interface"))
            .next()
            .expect("held")
            .fingerprint()
            .to_string(),
        operation: "@op/set_origin".into(),
        values: Default::default(),
        selectors: vec!["zk2/robot1/nav/nav.v2/@op/set_origin".into()],
        mode: zenkey_fleet::report::CallMode::Concrete,
        timeout_s: 2.0,
        answer,
    }
}

/// The value `set_origin` answers: the origin now in use.
fn origin_value() -> zenkey_fleet::report::OperationAnswer {
    use zenkey_fleet::report::{OperationAnswer, PayloadRendering, Rendered};
    OperationAnswer::Value {
        reply: PayloadRendering {
            key: "zk2/robot1/nav/nav.v2/@op/set_origin".into(),
            size: 27,
            resource: None,
            rendered: Rendered::Value {
                declared: "nav.v2.Origin".into(),
                value: serde_json::json!({
                    "latitudeDeg": 47.6062,
                    "longitudeDeg": -122.3321,
                    "altitudeM": 56.0
                }),
            },
        },
    }
}

/// The call mode on `nav.v2`'s `set_origin` (#614, FL10): the revision in
/// hand picked, the declared shape and request form read from it, the
/// address `robot1/nav`, and the value it answered.
fn workbench_call(theme: ThemeChoice) -> Zengui {
    use zengui::prefs::DockRole;
    use zengui::view::send::{RevisionChoice, SendMode, SendMsg};
    let mut app = workbench(theme, RightPane::Send);
    // The tool given the window, as the traffic scene gives its tab: the
    // answer sits under the form, and a third of the width would push it
    // below the fold.
    for role in [DockRole::Locator, DockRole::Inspector] {
        send(
            &mut app,
            Message::Workspace(WorkspaceMsg::DockToggled(role)),
        );
    }
    send_msg(&mut app, SendMsg::ModeSelected(SendMode::Call));
    let nav = super::contracts()
        .set
        .of_iface(&"nav.v2".parse().expect("an interface"))
        .next()
        .expect("held")
        .clone();
    send_msg(&mut app, SendMsg::RevisionPicked(RevisionChoice(nav)));
    send_msg(&mut app, SendMsg::OperationPicked("@op/set_origin".into()));
    send_msg(&mut app, SendMsg::TargetChanged("robot1/nav".into()));
    send_msg(
        &mut app,
        SendMsg::BodyChanged(
            r#"{"origin": {"latitude_deg": 47.6062, "longitude_deg": -122.3321, "altitude_m": 56.0}}"#
                .into(),
        ),
    );
    send_msg(
        &mut app,
        SendMsg::Done(Ok(Arc::new(set_origin_report(origin_value())))),
    );
    app
}

/// The Watch layout with the dock on the publish log (#614, FL10): a key a
/// zk2 service owns refused (P3), a foreign publish, a call's value and a
/// call's silence — a non-verdict, not a failure.
fn publish_log(theme: ThemeChoice) -> Zengui {
    use zengui::view::send::SendMsg;
    use zenkey_fleet::report::{OperationAnswer, PresenceAttribution, SilenceView};
    let mut app = watching(prefs(theme));
    send(
        &mut app,
        Message::Workspace(WorkspaceMsg::ActivityTab(ActivityTab::Publish)),
    );
    send_msg(
        &mut app,
        SendMsg::KeyChanged("zk2/robot1/nav/nav.v2/state/status".into()),
    );
    send_msg(&mut app, SendMsg::Send);
    publish_foreign(&mut app);
    send_msg(
        &mut app,
        SendMsg::Done(Ok(Arc::new(set_origin_report(origin_value())))),
    );
    let mut silent = set_origin_report(OperationAnswer::Silent {
        silence: SilenceView {
            attempts: 1,
            transport: Some("zenoh/string: Timeout".into()),
            presence: PresenceAttribution::NoTokenVisible,
        },
    });
    silent.address = "robot2/nav".into();
    silent.selectors = vec!["zk2/robot2/nav/nav.v2/@op/set_origin".into()];
    send_msg(&mut app, SendMsg::Done(Ok(Arc::new(silent))));
    app
}

/// The nodes tool on zk2 presence (#614, FL9): both addresses the world
/// holds up, `robot1/nav` selected — its instance, what its descriptor
/// says, and its `health.v1` as the reading its selection asks for would
/// answer: not asked, since its descriptor does not list the profile.
fn workbench_nodes(theme: ThemeChoice) -> Zengui {
    use zengui::view::nodes::NodesMsg;
    let mut app = workbench(theme, RightPane::Nodes);
    let nav: zenkey_model::grammar::Addr = "robot1/nav".parse().expect("an address");
    send(
        &mut app,
        Message::Subject(SubjectMsg::Select(Subject::Address(nav.clone()))),
    );
    send(
        &mut app,
        Message::Pane(PaneMsg::Nodes(NodesMsg::HealthRead(
            nav,
            String::new(),
            World::new().health("robot1/nav"),
        ))),
    );
    app
}

fn config_msg(app: &mut Zengui, msg: zengui::view::configure::ConfigMsg) {
    send(app, Message::Pane(PaneMsg::Config(msg)));
}

/// The Config tool given the window (#768, CD): presence with `lab/rf0`
/// beside the world, the tool picked, the address selected — which the
/// tool follows — and its reading landed as the engine judged it, with the
/// countdown's tick 13 s after it.
fn config_window(theme: ThemeChoice, view: zenkey_model::config::ConfigView) -> (Zengui, World) {
    use zengui::configure::{ReadLanding, ReadWhy};
    use zengui::prefs::DockRole;
    use zengui::view::configure::ConfigMsg;
    let (mut app, world) = live_world(prefs(theme));
    send(
        &mut app,
        Message::Bus(zengui::message::BusMsg::Presence(
            0,
            world.presence_update_with_config(),
        )),
    );
    // The tool first — picking it restores the Workbench — then the other
    // docks closed, so the tool has the window: its groups sit under the
    // pending change.
    send(
        &mut app,
        Message::Workspace(WorkspaceMsg::PaneSelected(RightPane::Config)),
    );
    for role in [DockRole::Locator, DockRole::Inspector] {
        send(
            &mut app,
            Message::Workspace(WorkspaceMsg::DockToggled(role)),
        );
    }
    let rf0: zenkey_model::grammar::Addr = super::RF0.parse().expect("an address");
    send(
        &mut app,
        Message::Subject(SubjectMsg::Select(Subject::Address(rf0.clone()))),
    );
    // The change asked about (§5): the pending one, else the last.
    let token = view
        .pending
        .as_ref()
        .map(|p| p.token.clone())
        .or_else(|| view.last_change.as_ref().map(|l| l.token.clone()));
    let report = world.config_report(&view, world.config_events(), token.as_deref());
    config_msg(
        &mut app,
        ConfigMsg::Read(ReadLanding {
            addr: rf0,
            namespace: String::new(),
            why: ReadWhy::Asked,
            report,
            at: super::config_read_at(),
        }),
    );
    config_msg(&mut app, ConfigMsg::Tick(super::config_now()));
    (app, world)
}

fn workbench_config(theme: ThemeChoice) -> Zengui {
    let view = World::new().config_view(true);
    config_window(theme, view).0
}

/// The sim group's pin typed into its masked field: four bullets on
/// screen, the value nowhere.
fn workbench_config_sensitive(theme: ThemeChoice) -> Zengui {
    use zengui::configure::{Draft, Masked};
    use zengui::view::configure::ConfigMsg;
    let view = World::new().config_view(false);
    let (mut app, _) = config_window(theme, view);
    config_msg(
        &mut app,
        ConfigMsg::Draft {
            group: "sim".into(),
            param: "pin".into(),
            draft: Draft::Secret(Masked::new("4821")),
        },
    );
    app
}

/// `duty` edited and applied while the reach change is pending, not
/// joined: the precheck refuses it in the owner's words, before any task.
fn workbench_config_refused(theme: ThemeChoice) -> Zengui {
    use zengui::configure::Draft;
    use zengui::view::configure::ConfigMsg;
    let view = World::new().config_view(true);
    let (mut app, _) = config_window(theme, view);
    config_msg(
        &mut app,
        ConfigMsg::Draft {
            group: "duty".into(),
            param: "fraction_ppm".into(),
            draft: Draft::Text("30000".into()),
        },
    );
    config_msg(&mut app, ConfigMsg::Apply("duty".into()));
    app
}

fn workbench_mesh(theme: ThemeChoice) -> Zengui {
    let mut app = workbench_admin(theme);
    send(
        &mut app,
        Message::Workspace(WorkspaceMsg::PaneSelected(RightPane::Mesh)),
    );
    app
}

/// The admin pane on zk2 (#614, FL9): one router, two storages judged
/// against S4 — one on owners' state (the finding), one beside it — and the
/// topology with the world's instances joined onto the router by the zid
/// their descriptors state (#705), one of them unattached.
fn workbench_admin(theme: ThemeChoice) -> Zengui {
    use zengui::admin::AdminSweep;
    use zengui::view::admin::AdminMsg;
    use zenkey_fleet::report::{
        Asked, AttachedTo, Attachment, InstanceAttachment, InstanceJoin, RouterInfo, StorageInfo,
        TopologyEdge, TopologyNode, TopologyReport,
    };

    let node = |zid: &str, whatami: &str, answered: bool| TopologyNode {
        zid: zid.into(),
        whatami: whatami.into(),
        version: answered.then(|| "1.10.1".into()),
        locators: if answered {
            vec![format!("tcp/10.0.4.{}:7447", zid.len())]
        } else {
            vec![]
        },
        locators_via_links: vec![],
        answered,
    };
    let edge = |reporter: &str, peer: &str, whatami: &str| TopologyEdge {
        reporter: reporter.into(),
        peer: peer.into(),
        whatami: whatami.into(),
        region: None,
        links: vec![],
    };
    let storage = |name: &str, key_expr: &str| StorageInfo {
        zid: "z1".into(),
        name: name.into(),
        key_expr: Some(key_expr.into()),
        strip_prefix: None,
        volume: Some("rocksdb".into()),
        raw: serde_json::json!({"volume": "rocksdb"}),
    };
    let instance = |address: &str, id: &str, attachment| InstanceAttachment {
        address: address.into(),
        instance: id.into(),
        zid: Some("ab12".into()),
        attachment,
    };
    let mut app = workbench(theme, RightPane::Admin);
    let storages = vec![
        storage("state", "**"),
        storage("events", "zk2/*/*/*/events/**"),
    ];
    let sweep = AdminSweep {
        routers: vec![RouterInfo {
            zid: "z1".into(),
            version: Some("1.10.1".into()),
            locators: vec!["tcp/10.0.4.2:7447".into()],
            raw: serde_json::json!({"zid": "z1", "version": "1.10.1"}),
        }],
        s4: zengui::admin::s4_rows("", &storages),
        storages,
        topology: TopologyReport {
            nodes: vec![
                node("z1", "router", true),
                node("z2", "peer", true),
                node("z3", "peer", false),
                node("z4", "client", false),
            ],
            edges: vec![
                edge("z1", "z2", "peer"),
                edge("z2", "z1", "router"),
                edge("z1", "z3", "peer"),
                edge("z1", "z4", "client"),
            ],
            asked: "@/*/*".into(),
            answered: 2,
            self_zid: "z4".into(),
            instances: Asked::Asked(InstanceJoin {
                namespace: String::new(),
                selector: "zk2/*/*/@zk/**".into(),
                complete: true,
                cut_off_during: false,
                verified: vec!["z1".into()],
                unverified: vec![],
                unobservable: None,
                instances: vec![
                    instance(
                        "robot1/nav",
                        "0000000000000001",
                        Attachment::Attached {
                            routers: vec![AttachedTo {
                                router: "z1".into(),
                                listed_as: "client".into(),
                            }],
                        },
                    ),
                    instance(
                        "host-a/tc",
                        "0000000000000002",
                        Attachment::Unattached {
                            reason: "no verified router lists ab12".into(),
                        },
                    ),
                ],
            }),
        },
        namespace: String::new(),
    };
    send(
        &mut app,
        Message::Pane(PaneMsg::Admin(AdminMsg::Done(Ok(Arc::new(sweep))))),
    );
    app
}

fn overlay(theme: ThemeChoice, which: Overlay) -> Zengui {
    let mut app = live(prefs(theme));
    send(
        &mut app,
        Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(which))),
    );
    app
}

fn palette(theme: ThemeChoice) -> Zengui {
    overlay(theme, Overlay::Commands)
}

fn palette_query(theme: ThemeChoice) -> Zengui {
    let mut app = overlay(theme, Overlay::Commands);
    send(
        &mut app,
        Message::Chrome(ChromeMsg::Palette(PaletteMsg::QueryChanged("zo".into()))),
    );
    app
}

fn keys(theme: ThemeChoice) -> Zengui {
    overlay(theme, Overlay::Keys)
}

fn help(theme: ThemeChoice) -> Zengui {
    overlay(theme, Overlay::Help)
}

fn settings(theme: ThemeChoice) -> Zengui {
    overlay(theme, Overlay::Settings)
}

fn connect(theme: ThemeChoice) -> Zengui {
    overlay(theme, Overlay::Connect)
}

/// A version-2 capture of v1 traffic, replayed (#614, FL10b): its rows name
/// v1 profiles (`qos`), which zk2 does not read, so the banner counts them
/// "QoS not recorded" and the Inspector says so of the selected key, where
/// FL6b showed default axes — and it states nothing about what was
/// excluded, because a version-2 file never did.
fn replay_v2(theme: ThemeChoice) -> Zengui {
    use zengui::replay::{LoadedReplay, ReplayState};
    use zengui::view::replay::ReplayMsg;
    const LOAD: &str = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/system/load";
    let file = [
        r#"{"zrec":2,"selectors":["v1/**"],"base":"","captured_at":"2026-08-12T03:00:00Z"}"#,
        r#"{"key":"v1/h-3fa9c2d41b7e/telemetry/sysinfo/system/load","t":0,"bytes":"eyJ2YWx1ZSI6MC40MiwidW5pdCI6ImxvYWQifQ==","encoding":"application/json","qos":"sampled"}"#,
        r#"{"key":"v1/h-3fa9c2d41b7e/state/sysinfo/health","t":250000,"bytes":"eyJzdGF0ZSI6Im9rIn0=","encoding":"application/json","qos":"transition"}"#,
        r#"{"key":"v1/h-3fa9c2d41b7e/telemetry/sysinfo/system/load","t":1000000,"bytes":"eyJ2YWx1ZSI6MC41NywidW5pdCI6ImxvYWQifQ==","encoding":"application/json","qos":"sampled"}"#,
        r#"{"key":"v1/h-3fa9c2d41b7e/telemetry/sysinfo/system/load","t":2000000,"bytes":"eyJ2YWx1ZSI6MC42MSwidW5pdCI6ImxvYWQifQ==","encoding":"application/json","qos":"sampled"}"#,
    ]
    .join("\n");
    let state = ReplayState::load("legacy.zrec", file.as_bytes()).expect("a valid capture");
    let id = state.id();
    let mut app = watching(prefs(theme));
    send(
        &mut app,
        Message::Workspace(WorkspaceMsg::ActivityTab(ActivityTab::Replay)),
    );
    send(
        &mut app,
        Message::Workspace(WorkspaceMsg::Replay(ReplayMsg::Loaded(
            "legacy.zrec".into(),
            Ok(LoadedReplay::new(state)),
        ))),
    );
    // Selected inside the replay, then played to its end: the recorder
    // keeps the rows that follow.
    send(
        &mut app,
        Message::Subject(SubjectMsg::Select(Subject::Key(LOAD.into()))),
    );
    send(
        &mut app,
        Message::Workspace(WorkspaceMsg::Replay(ReplayMsg::Scrubbed(2_000_000))),
    );
    // The value at the playhead is the capture's last row for the key
    // (#614, FL8a), checked offline — never a live read about another
    // world, so no "superseded" line.
    recorded_check(
        &mut app,
        id,
        LOAD,
        br#"{"value":0.61,"unit":"load"}"#,
        "application/json",
    );
    app
}

/// An owner silent on S4's GET, and an archive's last-known answer beside
/// it (#614, FL8a; S5, S6): `host-a/tc`'s `eth0` selected, presence showing
/// `ground/archive` providing `archive.v1`, and its answer read by the
/// button — labelled last-known, confirmed by alignment, never current.
fn inspector_last_known(theme: ThemeChoice) -> Zengui {
    use zengui::view::detail::DetailMsg;
    let mut app = live(prefs(theme));
    preset(&mut app, LayoutPreset::Explore);
    let world = World::new();
    send(
        &mut app,
        Message::Bus(zengui::message::BusMsg::Presence(
            0,
            world.presence_update_with_archive(),
        )),
    );
    send(
        &mut app,
        Message::Subject(SubjectMsg::ValueFetched(
            ETH0.into(),
            Ok(Arc::new(zengui::value::ValueRead::Owner(
                world.state_read(ETH0, None, None),
            ))),
        )),
    );
    let archive: zenkey_model::grammar::Addr = "ground/archive".parse().expect("an address");
    let detail = |m| Message::Pane(PaneMsg::Detail(zengui::message::SlotId::FOLLOW, m));
    // Asked as the button asks — no session answers here, so the scene
    // lands the archive's answer itself, as the task would have.
    send(&mut app, detail(DetailMsg::LastKnown(archive.clone())));
    send(
        &mut app,
        detail(DetailMsg::LastKnownRead(
            ETH0.into(),
            archive,
            Ok(Arc::new(world.state_read(
                ETH0,
                Some(ETH0_BODY),
                Some("ground/archive"),
            ))),
        )),
    );
    app
}

/// A stranger on an owned key (#614, FL12): `host-a/tc`'s `eth0` selected,
/// and its newest sample a raw put on zenoh's default axes after the
/// owner's. The Inspector names the axis that differs, with both sides:
/// the `block` tc.netif.v1's state declares against the `drop` the
/// stranger rode. Every other scene's zk2 samples ride their declared
/// axes, as an owner writing through the runtime does, so this is the one
/// picture of a mismatch.
fn inspector_qos_mismatch(theme: ThemeChoice) -> Zengui {
    let (mut app, mut world) = live_world(prefs(theme));
    preset(&mut app, LayoutPreset::Explore);
    world.tick_with_stranger(&mut app, ETH0);
    app
}

/// A window on the metered world (#819, TD): `lab/meter` and `site-a/rf0`
/// beside the rest, `key` revealed and selected before its traffic so its
/// history holds `ticks` of it, every newest payload checked, and the
/// Explore layout, which gives the Inspector the height its Series wants.
fn metered(prefs: Prefs, key: &str, ticks: u32) -> (Zengui, World) {
    let mut app = window(prefs);
    let mut world = World::metered();
    world.connect(&mut app);
    world.tick(&mut app);
    send(
        &mut app,
        Message::Workspace(WorkspaceMsg::Reveal(key.into())),
    );
    send(
        &mut app,
        Message::Subject(SubjectMsg::Select(Subject::Key(key.into()))),
    );
    world.ticks(&mut app, ticks);
    world.verdicts(&mut app);
    preset(&mut app, LayoutPreset::Explore);
    (app, world)
}

fn metric_counter(theme: ThemeChoice) -> Zengui {
    metered(prefs(theme), FRAMES, 29).0
}

fn metric_decreased(theme: ThemeChoice) -> Zengui {
    metered(prefs(theme), BYTES, 29).0
}

fn metric_unit_unknown(theme: ThemeChoice) -> Zengui {
    metered(prefs(theme), DROPS, 29).0
}

fn metric_histogram(theme: ThemeChoice) -> Zengui {
    metered(prefs(theme), LATENCY, 29).0
}

/// One more tick in which `latency` publishes a Point tagged a gauge — the
/// schema admits it, and telemetry.v1 does not (§2.6, check 4) — then the
/// newest payloads checked again, so the tree's badge carries the verdict
/// the Inspector explains.
fn tag_disagrees(prefs: Prefs) -> Zengui {
    let (mut app, mut world) = metered(prefs, LATENCY, 28);
    world.tick_with(
        &mut app,
        LATENCY,
        super::metric_point(
            29,
            "gauge",
            serde_json::json!(0.0125),
            Some("s"),
            Some("00000000000000a1"),
        ),
    );
    world.verdicts(&mut app);
    app
}

fn metric_tag_disagrees(theme: ThemeChoice) -> Zengui {
    tag_disagrees(prefs(theme))
}

/// [`tag_disagrees`] in the Watch layout, echo pinned to the key (its
/// default): the Activity dock's account of the verdict.
fn watch_metric_invalid(theme: ThemeChoice) -> Zengui {
    let mut app = tag_disagrees(prefs(theme));
    preset(&mut app, LayoutPreset::Watch);
    send(
        &mut app,
        Message::Workspace(WorkspaceMsg::ActivityTab(ActivityTab::Echo)),
    );
    app
}

fn modem_units(theme: ThemeChoice) -> Zengui {
    metered(prefs(theme), TX_BYTES, 29).0
}
