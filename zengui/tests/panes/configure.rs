//! The Config tool on `config.v1` (#768, CD): the view by class, the
//! pending change on the owner's clock, confirm only once applied, a
//! sensitive value never on screen, a set refused in the form, and each
//! answer by its union.

use std::sync::Arc;

use zengui::configure::{Act, Answer, Answered, ConfigState, Draft, Masked, ReadLanding, ReadWhy};
use zengui::view::configure::{ConfigData, pane};
use zenkey_fleet::report::{
    ConfigActAnswer, ConfigActReport, ConfigRefusal, PresenceAttribution, SilenceView,
};
use zenkey_model::config::{ConfigError, ConfigView, ErrorCode, PendingReply};
use zenkey_model::grammar::Addr;

use super::common::{RF0, RF0_INSTANCE, World, config_now, config_read_at};
use super::*;

fn rf0() -> Addr {
    RF0.parse().expect("an address")
}

/// The tool pointed at `lab/rf0`, `view` read as the engine judges it, the
/// countdown's clock 13 s after the reading.
fn state(view: &ConfigView) -> ConfigState {
    let world = World::new();
    let mut s = ConfigState::default();
    s.target = Some(rf0());
    s.landed(ReadLanding {
        addr: rf0(),
        namespace: String::new(),
        why: ReadWhy::Asked,
        report: world.config_report(
            view,
            world.config_events(),
            Some(&format!("{RF0_INSTANCE}-2")),
        ),
        at: config_read_at(),
    });
    s.now = Some(config_now());
    s
}

fn render<'a>(s: &'a ConfigState, replaying: bool) -> iced_test::Simulator<'a, Message> {
    iced_test::Simulator::with_size(
        iced::Settings::default(),
        (1600.0, 3200.0),
        pane(ConfigData {
            state: s,
            subject: s.target.as_ref(),
            replaying,
            session: true,
            sp: sp(),
        }),
    )
}

/// The served schema, rendered (§2.2–§2.4, §2.16): groups by class in
/// their own words, a kind with its bounds and unit, a startup value that
/// differs from the running one, `writable` as the engine answered it, the
/// floor, and the desired report.
#[test]
fn the_view_renders_its_schema_by_class() {
    let s = state(&World::new().config_view(false));
    let mut ui = render(&s, false);
    for text in [
        "hot — takes effect at once",
        "reach — can cut the link",
        "contract — a restart's",
        "integer 0…1000000 ppm",
        "integer ≥ 0 packets",
        "startup 10000 — what a restart runs (§2.3)",
        "not writable — outside the startup allowlist (§2.16)",
        "not writable — set in the startup configuration, then a restart (§2.4)",
        "writable",
        "10 s — the smallest window",
        "desired: revision 4 acted on — converged",
        "runtime",
        "default",
    ] {
        assert!(ui.find(text).is_ok(), "{text:?} on screen");
    }
    // A writable group's form says the floor before anything is armed.
    assert!(
        ui.find(
            "required unless joining: the owner applies it at its apply_at and undoes it at the \
             deadline unless you confirm over the new link; a shorter window is raised to the \
             floor, 10 s (§2.4, §2.8)"
        )
        .is_ok()
    );
}

/// The pending change (§2.7–§2.9): its deadline as the owner's instant,
/// the countdown read against this host's clock and labelled so, and §5's
/// "is a change pending?" in the engine's words.
#[test]
fn a_pending_change_counts_down_on_the_owners_deadline() {
    let s = state(&World::new().config_view(true));
    let mut ui = render(&s, false);
    let token = format!("{RF0_INSTANCE}-3");
    assert!(ui.find(format!("pending change {token} on radio")).is_ok());
    assert!(
        ui.find(
            "deadline 2026-10-18T09:01:00.000Z — the owner's instant (§2.8): undone then \
             unless confirmed"
        )
        .is_ok()
    );
    assert!(
        ui.find(
            "47s left before the owner's deadline — its instant, read against this host's clock"
        )
        .is_ok(),
        "the countdown reads the tick's clock, never the wall"
    );
    assert!(ui.find("applied — confirm it over the new link").is_ok());
}

/// Confirm is offered only once the view shows the change applied: before
/// that the owner answers it busy (§2.9, §2.10 row 5). Cancel and extend
/// are offered either way.
#[test]
fn confirm_is_offered_only_when_applied() {
    let mut view = World::new().config_view(true);
    let applied = state(&view);
    let mut ui = render(&applied, false);
    assert!(ui.find("confirm").is_ok(), "applied: confirm offered");
    assert!(ui.find("cancel — undo now").is_ok());

    view.pending.as_mut().expect("pending").applied = false;
    let waiting = state(&view);
    let mut ui = render(&waiting, false);
    assert!(
        ui.find("confirm").is_err(),
        "not applied: no confirm to press"
    );
    assert!(
        ui.find(
            "not applied yet — a group waits for its apply_at; confirm is offered once the view \
             shows it applied (§2.9)"
        )
        .is_ok()
    );
    assert!(ui.find("cancel — undo now").is_ok());
}

/// A sensitive value is never on screen (§2.15): typed, its field shows
/// bullets; its row says write-only; its event's edit is redacted; and a
/// refusal that would have carried it is scrubbed.
#[test]
fn a_sensitive_value_is_never_rendered() {
    let mut s = state(&World::new().config_view(false));
    s.forms
        .entry("sim".into())
        .or_default()
        .drafts
        .insert("pin".into(), Draft::Secret(Masked::new("4821")));
    s.answered = Some(Answered {
        act: Act::Set {
            group: "sim".into(),
            dry_run: false,
        },
        answer: Answer::Landed(Arc::new(act_report(
            "set/sim",
            ConfigActAnswer::Refused {
                refusal: ConfigRefusal {
                    code: "app".into(),
                    message: "the device refused pin <redacted>".into(),
                    cause: None,
                    error: Some(ConfigError {
                        code: ErrorCode::DeviceRefused,
                        detail: None,
                    }),
                    detail_undecodable: None,
                },
            },
        ))),
    });
    let mut ui = render(&s, false);
    assert!(ui.find("4821").is_err(), "the value is nowhere");
    assert!(ui.find("••••").is_ok(), "the masked field shows bullets");
    assert!(ui.find("write-only").is_ok());
    assert!(ui.find("sim.pin (write-only)").is_ok(), "the event's edit");
    assert!(ui.find("the device refused pin <redacted>").is_ok());
}

/// A set refused in the form says so, and that nothing was sent.
#[test]
fn a_refused_set_says_nothing_was_sent() {
    let mut s = state(&World::new().config_view(true));
    s.answered = Some(Answered {
        act: Act::Set {
            group: "duty".into(),
            dry_run: false,
        },
        answer: Answer::RefusedHere("busy: a change is pending".into()),
    });
    let mut ui = render(&s, false);
    assert!(ui.find("refused here — nothing was sent").is_ok());
    assert!(ui.find("busy: a change is pending").is_ok());
    assert!(ui.find("set/duty").is_ok());
}

/// Each answer by its union (§3): a reach set's pending reply with the
/// owner's instants, a control's view, a refusal with its ConfigError, and
/// silence — a non-verdict, never a failure.
#[test]
fn each_answer_renders_by_its_union() {
    let view = World::new().config_view(true);
    let cases: Vec<(Act, ConfigActAnswer, &str)> = vec![
        (
            Act::Set {
                group: "radio".into(),
                dry_run: false,
            },
            ConfigActAnswer::Pending {
                pending: PendingReply {
                    token: format!("{RF0_INSTANCE}-3"),
                    apply_at: "2026-10-18T09:00:01.000Z".into(),
                    deadline: "2026-10-18T09:01:00.000Z".into(),
                },
            },
            "pending — change 00000000000000a1-3, answered before its apply (§2.9)",
        ),
        (
            Act::Confirm,
            ConfigActAnswer::View {
                view: Box::new(view.clone()),
            },
            "answered — the owner's view after the act (§2.10)",
        ),
        (
            Act::Set {
                group: "modulation".into(),
                dry_run: false,
            },
            ConfigActAnswer::Refused {
                refusal: ConfigRefusal {
                    code: "app".into(),
                    message: "group \"modulation\" is part of what this service was started \
                              against"
                        .into(),
                    cause: None,
                    error: Some(ConfigError {
                        code: ErrorCode::RestartRequired,
                        detail: None,
                    }),
                    detail_undecodable: None,
                },
            },
            "ConfigError restart-required",
        ),
        (
            Act::Set {
                group: "duty".into(),
                dry_run: true,
            },
            ConfigActAnswer::Applied {
                view: Box::new(view.clone()),
            },
            "dry run — the view as it would read; nothing was touched, minted or published \
             (§2.9)",
        ),
        (
            Act::Cancel,
            ConfigActAnswer::Silent {
                silence: SilenceView {
                    attempts: 1,
                    transport: None,
                    presence: PresenceAttribution::Present,
                },
            },
            "No answer",
        ),
    ];
    for (act, answer, want) in cases {
        let mut s = state(&view);
        let op = act.op().name();
        s.answered = Some(Answered {
            act,
            answer: Answer::Landed(Arc::new(act_report(&op, answer))),
        });
        let mut ui = render(&s, false);
        assert!(ui.find(want).is_ok(), "{op}: {want:?}");
        assert!(
            ui.find("refused here — nothing was sent").is_err(),
            "{op}: an answer that landed is not a refusal in the form"
        );
    }
}

/// Nothing picked is not an empty configuration, and an address selected
/// but not read says it was not asked.
#[test]
fn not_picked_and_not_read_say_which() {
    let s = ConfigState::default();
    let mut ui = iced_test::Simulator::with_size(
        iced::Settings::default(),
        (1600.0, 1200.0),
        pane(ConfigData {
            state: &s,
            subject: None,
            replaying: false,
            session: true,
            sp: sp(),
        }),
    );
    assert!(ui.find("No service picked").is_ok());
    let a = rf0();
    let mut ui = iced_test::Simulator::with_size(
        iced::Settings::default(),
        (1600.0, 1200.0),
        pane(ConfigData {
            state: &s,
            subject: Some(&a),
            replaying: false,
            session: true,
            sp: sp(),
        }),
    );
    assert!(ui.find("Not read").is_ok());
}

/// While the panes replay a capture, the tool says it acts on nothing.
#[test]
fn the_replay_lock_is_said_where_the_acts_are() {
    let s = state(&World::new().config_view(true));
    let mut ui = render(&s, true);
    assert!(ui.find(zengui::view::send::REPLAY_LOCK).is_ok());
}

fn act_report(operation: &str, answer: ConfigActAnswer) -> ConfigActReport {
    ConfigActReport {
        namespace: String::new(),
        service: RF0.into(),
        operation: operation.into(),
        key: format!("zk2/{RF0}/config.v1/@op/{operation}"),
        request: serde_json::json!({}),
        actor: Some("ops".into()),
        request_id: None,
        checked: None,
        across_face: false,
        timeout_s: 5.0,
        answer,
    }
}
