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
    ActivityTab, BusMsg, ChromeMsg, Message, PaneMsg, RightPane, Subject, SubjectMsg, WorkspaceMsg,
};
use zengui::prefs::{Density, LayoutPreset, Prefs, ThemeChoice};
use zengui::view::palette::{Overlay, PaletteMsg};

use super::{HEALTH, ORIGIN_A, ORIGIN_B, USED, World, send, window};

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
        landmark: "slice-sync",
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
        landmark: "sysinfo: alive",
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
    Scene {
        name: "palette",
        size: DESKTOP,
        build: palette,
        landmark: "run doctor",
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
];

fn prefs(theme: ThemeChoice) -> Prefs {
    Prefs {
        theme,
        ..Prefs::default()
    }
}

/// A window on a live-looking bus: registry loaded, the pump running, a
/// few seconds of traffic, the tree opened down to the leaves.
fn live(prefs: Prefs) -> Zengui {
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
    ] {
        send(
            &mut app,
            Message::Workspace(WorkspaceMsg::Reveal(key.into())),
        );
    }
    // Selected before the rest of the traffic, so the history ring and the
    // series fill the way they would for a user who clicked early.
    select(&mut app, USED);
    world.ticks(&mut app, 30);
    verdicts(&mut app);
    app
}

/// Point the workspace at a key and answer the fetch it issues.
fn select(app: &mut Zengui, key: &str) {
    use zenkey_fleet::model::decode::{DecodedSample, Rendering};
    use zenkey_fleet::{FetchOutcome, FetchedValue, ValueSource};

    send(
        app,
        Message::Subject(SubjectMsg::Select(Subject::Key(key.into()))),
    );
    let body = br#"{"value":64.20,"unit":"%","ts":"2026-10-05T09:12:44Z"}"#;
    send(
        app,
        Message::Subject(SubjectMsg::ValueFetched(
            key.into(),
            Ok(Arc::new(FetchOutcome::Value(FetchedValue {
                key: key.into(),
                payload: zenoh::bytes::ZBytes::from(body.to_vec()),
                encoding: "application/json".into(),
                timestamp: None,
                attachment: None,
                source: ValueSource::Storage,
            }))),
        )),
    );
    send(
        app,
        Message::Subject(SubjectMsg::ValueDecoded(
            key.into(),
            Arc::new(zengui::value::DecodedValue::new(DecodedSample {
                type_name: Some("TelemetryPoint".into()),
                rendering: Rendering::Structural(
                    "{\n  \"value\": 64.2,\n  \"unit\": \"%\",\n  \"ts\": \"2026-10-05T09:12:44Z\"\n}"
                        .into(),
                ),
                verdict: zenkey::schema::validate::Verdict::Valid,
                decode_error: None,
            })),
        )),
    );
}

/// What the bounded validation batch would have answered (#164): one key
/// of each verdict, so every badge state has a row.
fn verdicts(app: &mut Zengui) {
    use zenkey::schema::validate::{NotValidated, Verdict};
    send(
        app,
        Message::Bus(BusMsg::VerdictsChecked(vec![
            (USED.into(), Verdict::Valid),
            (
                "v1/h-3fa9c2d41b7e/telemetry/sysinfo/system/load".into(),
                Verdict::Valid,
            ),
            (
                "v1/h-9c04d2e1a7f3/telemetry/sysinfo/system/load".into(),
                Verdict::Invalid(vec!["/value: not a number".into()]),
            ),
            (
                "v1/h-9c04d2e1a7f3/telemetry/sysinfo/no/such/thing".into(),
                Verdict::NotValidated(NotValidated::NoSchema),
            ),
        ])),
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

fn diagnose(theme: ThemeChoice) -> Zengui {
    use zengui::view::doctor::DoctorMsg;
    use zenkey_fleet::report::{Asked, CheckId, DoctorFinding, DoctorReport, DoctorSeverity};

    let mut app = live(prefs(theme));
    preset(&mut app, LayoutPreset::Diagnose);
    let report = DoctorReport {
        findings: vec![
            DoctorFinding {
                severity: DoctorSeverity::Error,
                check: CheckId::SliceSync,
                subject: format!("{ORIGIN_B}/sysinfo"),
                evidence: "registry version differs: served 1.0, local 2.0".into(),
                citation: Some("RFC 08 §6".into()),
            },
            DoctorFinding {
                severity: DoctorSeverity::Warning,
                check: CheckId::DescribeMissing,
                subject: format!("{ORIGIN_A}/sysinfo"),
                evidence: "no describe document served for sysinfo".into(),
                citation: Some("RFC 08 §7".into()),
            },
            DoctorFinding {
                severity: DoctorSeverity::Info,
                check: CheckId::DescribeMissing,
                subject: "fleet".into(),
                evidence: "1 producer(s) serve no describe".into(),
                citation: Some("RFC 08 §7".into()),
            },
        ],
        synced: Asked::NotAsked,
        introspect_answered: 2,
        live_producers: 2,
        describe_served: 1,
        describe_missing: 1,
        routers: 1,
        router_version: Some("1.9.0".into()),
        deep: false,
        observation: None,
        unobservable: None,
    };
    send(
        &mut app,
        Message::Pane(PaneMsg::Doctor(DoctorMsg::Done(Ok(
            zengui::doctor::DoctorRun {
                report: Arc::new(report),
                base: String::new(),
            },
        )))),
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

fn workbench_send(theme: ThemeChoice) -> Zengui {
    workbench(theme, RightPane::Send)
}

fn workbench_nodes(theme: ThemeChoice) -> Zengui {
    workbench(theme, RightPane::Nodes)
}

fn workbench_mesh(theme: ThemeChoice) -> Zengui {
    let mut app = workbench_admin(theme);
    send(
        &mut app,
        Message::Workspace(WorkspaceMsg::PaneSelected(RightPane::Mesh)),
    );
    app
}

fn workbench_admin(theme: ThemeChoice) -> Zengui {
    use zengui::admin::AdminSweep;
    use zengui::view::admin::AdminMsg;
    use zenkey_fleet::report::StorageList;
    use zenkey_fleet::{
        Coverage, CoverageRow, OriginAttachment, RouterInfo, StorageInfo, TopologyEdge,
        TopologyNode, TopologyReport,
    };

    let node = |zid: &str, whatami: &str, answered: bool| TopologyNode {
        zid: zid.into(),
        whatami: whatami.into(),
        version: answered.then(|| "1.9.0".into()),
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
    let mut app = workbench(theme, RightPane::Admin);
    let sweep = AdminSweep {
        routers: vec![RouterInfo {
            zid: "z1".into(),
            version: Some("1.9.0".into()),
            locators: vec!["tcp/10.0.4.2:7447".into()],
            raw: serde_json::json!({"zid": "z1", "version": "1.9.0"}),
        }],
        storage: StorageList {
            storages: vec![StorageInfo {
                zid: "z1".into(),
                name: "state".into(),
                key_expr: Some("v1/*/state/**".into()),
                strip_prefix: None,
                volume: Some("rocksdb".into()),
                raw: serde_json::json!({"volume": "rocksdb"}),
            }],
            coverage: vec![
                CoverageRow {
                    producer: "sysinfo".into(),
                    path: "health".into(),
                    ttl_s: Some(60),
                    coverage: Coverage::Covered("state@z1".into()),
                },
                CoverageRow {
                    producer: "netring".into(),
                    path: "capture/{iface}/state".into(),
                    ttl_s: None,
                    coverage: Coverage::Partial("state@z1".into()),
                },
                CoverageRow {
                    producer: "snmp".into(),
                    path: "device/{host}/status".into(),
                    ttl_s: Some(30),
                    coverage: Coverage::Uncovered,
                },
            ],
        },
        declared: None,
        coverage_note: None,
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
        },
        origins: vec![
            OriginAttachment {
                origin: ORIGIN_A.into(),
                session_zid: Some("z2".into()),
                reporter_zid: "z1".into(),
                token_key: format!("v1/{ORIGIN_A}/state/sysinfo/alive"),
            },
            OriginAttachment {
                origin: ORIGIN_B.into(),
                session_zid: None,
                reporter_zid: "z1".into(),
                token_key: format!("v1/{ORIGIN_B}/state/sysinfo/alive"),
            },
        ],
        base: String::new(),
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

fn settings(theme: ThemeChoice) -> Zengui {
    overlay(theme, Overlay::Settings)
}

fn connect(theme: ThemeChoice) -> Zengui {
    overlay(theme, Overlay::Connect)
}
