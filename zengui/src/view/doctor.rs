//! The doctor panel (#71; zk2's since #614, FL9): run-on-demand rendering
//! of the engine's zk2 `DoctorReport` — the same struct `zenctl doctor
//! --format json` emits: one verdict per check in the judgement shape, the
//! findings grouped by severity, the subjects a check could not decide, and
//! the run-over-run delta.
//!
//! Never ambient: a doctor run fans real queries across the deployment, so
//! it costs exactly one button press (the laziness ground rule).

use iced::widget::{column, row, scrollable};
use iced::{Element, Length};
use zenkey_fleet::report::{
    Asked, CheckReport, DoctorFinding, DoctorReport, DoctorSeverity, Judgement,
};

use crate::doctor::{DoctorState, finding_target};
use crate::message::{Message, PaneMsg};
use crate::view::kit;
use crate::view::theme::{JudgementTone, SeverityTone};
use crate::view::tokens::{Spacing, font};

/// How many undecided subjects the list draws before it stops and says so
/// (RFC 09 §5.1 O6): a deployment with a hundred silent descriptors would
/// otherwise push every finding below the fold.
const UNJUDGED_ROWS: usize = 12;

/// The panel's interactions, nested per the `CallMsg` precedent.
#[derive(Debug, Clone)]
pub enum DoctorMsg {
    /// The run button — the only way a run starts.
    Run,
    DeepToggled(bool),
    /// The operator's word for the clock (D11): the `health-*` checks can
    /// judge a status's age only with it.
    ClocksSynced(bool),
    /// A run finished: it always lands a report.
    Done(crate::doctor::DoctorRun),
    /// A finding row was clicked — navigate to its subject. The index is
    /// into the report's findings, in check order.
    FindingClicked(usize),
    /// Read presence and the contracts again — v1's "re-ask schemas" on
    /// zk2's inputs: the lens's catalog, and every revision no holder
    /// served (#101's escape hatch).
    Reread,
}

/// Wrap one of this pane's messages for the app (#176).
fn msg(m: DoctorMsg) -> Message {
    Message::Pane(PaneMsg::Doctor(m))
}

/// The doctor's results — the Activity dock's Doctor stream (#183).
///
/// It stopped being a *place* and became an action: the run is a palette
/// command, and this is where its verdict lands. That is the honest shape,
/// because a doctor run is a thing you do to a session, not a view of the
/// subject.
pub fn section<'a>(
    state: &'a DoctorState,
    namespace: &'a str,
    sp: Spacing,
) -> Element<'a, Message> {
    let run_label = if state.in_flight {
        "running…"
    } else {
        "run doctor"
    };
    let mut run = kit::primary(kit::caption(run_label)).padding(sp.xs);
    if !state.in_flight {
        run = run.on_press(msg(DoctorMsg::Run));
    }
    let deep = kit::check(state.deep)
        .label("deep: GET every owner's state")
        .size(font::CAPTION)
        .text_size(font::CAPTION)
        .on_toggle(|b| msg(DoctorMsg::DeepToggled(b)));
    let clocks = kit::check(state.clocks_synced)
        .label("clocks synced")
        .size(font::CAPTION)
        .text_size(font::CAPTION)
        .on_toggle(|b| msg(DoctorMsg::ClocksSynced(b)));

    // The re-read lives here because it is the same kind of thing as the
    // run button: an explicit, costed re-ask, never ambient.
    let reread = kit::secondary(kit::caption("re-read presence and contracts"))
        .padding(sp.xs)
        .on_press(msg(DoctorMsg::Reread));

    let mut col = column![
        kit::section_header("Doctor", None),
        row![run, deep, clocks]
            .spacing(sp.md)
            .align_y(iced::Alignment::Center),
        row![
            reread,
            kit::muted(match state.rereads {
                0 => "the presence link follows both; this reads them again now".to_string(),
                n => format!(
                    "presence and the contracts re-read {} — every revision no holder served \
                     is asked again",
                    kit::plural(n, "time")
                ),
            }),
        ]
        .spacing(sp.md)
        .align_y(iced::Alignment::Center),
    ]
    .spacing(sp.sm);

    if let Some(e) = &state.error {
        col = col.push(kit::error(format!("doctor run could not start: {e}")));
    }

    let Some(report) = state.current.as_deref() else {
        // Never-run is not "0 findings" (O4).
        col = col.push(kit::empty(
            kit::EmptyKind::NotAsked,
            "no doctor run yet",
            "findings are produced on demand — a run reads presence, descriptors, \
             contracts and the admin space",
        ));
        return col.into();
    };

    // The scope is what makes an empty findings list legible (RFC 05 §3.1:
    // silence needs attribution).
    col = col.push(kit::muted(scope_line(report)));
    // The run as one judgement, under the floor `zenctl doctor` defaults to.
    col = col.push(judgement_line(report));
    // An empty scope judged nothing — its empty findings list is not a
    // healthy deployment, and the panel says so in the engine's words.
    if let Some(why) = &report.unobservable {
        col = col.push(kit::callout(
            crate::view::theme::Tone::Neutral,
            kit::caption(format!("unobservable — {why}")),
        ));
    }
    if let Some(d) = &state.delta {
        col = col.push(kit::muted(format!(
            "vs previous run: {} new · {} fixed · {} unchanged",
            d.new.len(),
            d.fixed.len(),
            d.unchanged
        )));
    }

    // The findings first — what fired is what a run is read for — then
    // every check's verdict, then what no check could decide.
    let mut list = column![].spacing(sp.sm);
    let findings: Vec<(usize, &DoctorFinding)> = report.findings().enumerate().collect();
    for severity in [
        DoctorSeverity::Error,
        DoctorSeverity::Warning,
        DoctorSeverity::Info,
    ] {
        let group: Vec<&(usize, &DoctorFinding)> = findings
            .iter()
            .filter(|(_, f)| f.severity == severity)
            .collect();
        if group.is_empty() {
            continue;
        }
        // Sentence case like every section header (#539), spelled here —
        // never the enum's `Debug` name — and counted (#565).
        let word = match severity {
            DoctorSeverity::Error => "Error",
            DoctorSeverity::Warning => "Warning",
            DoctorSeverity::Info => "Info",
        };
        list = list.push(kit::section_header(
            word,
            Some(kit::count_pill(group.len().to_string())),
        ));
        for (i, f) in group {
            list = list.push(finding_row(state, f, *i, namespace, sp));
        }
    }

    list = list.push(kit::section_header(
        "Checks",
        Some(kit::count_pill(report.checks.len().to_string())),
    ));
    for check in &report.checks {
        list = list.push(check_row(check, sp));
    }

    // What a check could not decide, and why: never folded into clean.
    let unjudged: Vec<(&CheckReport, &zenkey_fleet::report::Unjudged)> = report
        .checks
        .iter()
        .flat_map(|c| c.unjudged.iter().map(move |u| (c, u)))
        .collect();
    if !unjudged.is_empty() {
        list = list.push(kit::section_header(
            "Not judged",
            Some(kit::count_pill(unjudged.len().to_string())),
        ));
        for (c, u) in unjudged.iter().take(UNJUDGED_ROWS) {
            list = list.push(kit::muted(format!(
                "{} · {} — {}",
                c.check, u.subject, u.reason
            )));
        }
        if unjudged.len() > UNJUDGED_ROWS {
            list = list.push(kit::muted(format!(
                "+{} more not shown (display bound)",
                unjudged.len() - UNJUDGED_ROWS
            )));
        }
    }

    if let Some(d) = &state.delta
        && !d.fixed.is_empty()
    {
        list = list.push(kit::section_header("Fixed since last run", None));
        for f in &d.fixed {
            list = list.push(kit::muted(format!(
                "{} · {} — {}",
                f.check, f.subject, f.evidence
            )));
        }
    }
    col = col.push(scrollable(list).height(Length::Fill));
    col.into()
}

/// What the run read: the namespace, presence, the revisions the
/// descriptors name, the routers and the health checks' ground — each
/// "not asked" when no check that needs it was.
fn scope_line(report: &DoctorReport) -> String {
    let s = &report.scope;
    let ns = if s.namespace.is_empty() {
        "the bus root".to_string()
    } else {
        format!("namespace {:?}", s.namespace)
    };
    let presence = match &s.presence {
        Asked::NotAsked => "presence not asked".to_string(),
        Asked::Asked(p) => format!(
            "{} · {} · {} · {} of {} held{}",
            kit::plural(p.services, "service"),
            kit::plural(p.instances, "instance"),
            kit::plural(p.tokens, "token"),
            p.held,
            kit::plural(p.revisions, "revision"),
            if p.complete {
                ""
            } else {
                " · presence possibly incomplete (§8.1)"
            },
        ),
    };
    let routers = match &s.routers {
        Asked::NotAsked => "admin space not asked".to_string(),
        Asked::Asked(n) => kit::plural(*n, "router"),
    };
    let health = match &s.health {
        Asked::NotAsked => String::new(),
        Asked::Asked(h) => format!(
            " · health.v1: {} ({})",
            kit::plural(h.services, "service"),
            if h.clocks_synced {
                "clocks synced"
            } else {
                "no clock word: a status's age is unobservable"
            }
        ),
    };
    format!("{ns} · {presence} · {routers}{health}")
}

/// The run as one judgement under the warning floor — what `zenctl doctor`
/// exits through by default: a finding, clean, or no verdict with why.
fn judgement_line<'a>(report: &DoctorReport) -> Element<'a, Message> {
    let j = report.judgement(DoctorSeverity::Warning);
    let tone = JudgementTone::of(&j);
    let said = match &j {
        Judgement::Established => "a finding at or above warning".to_string(),
        Judgement::NotEstablished { reason } => format!("clean — {reason}"),
        Judgement::Unobservable { reason } => format!("no verdict — {reason}"),
        Judgement::NotAsked => "no check was asked".to_string(),
    };
    kit::badge_judgement(tone, format!("run: {said}"))
}

/// One check's verdict in the judgement shape (tooling guide §1): its id,
/// the pole it stands at, the section it enforces, and the reason a clean
/// or an unobservable answer carries — never "no" for "not asked".
fn check_row<'a>(c: &CheckReport, sp: Spacing) -> Element<'a, Message> {
    let tone = JudgementTone::of(&c.verdict);
    let mut r = row![
        kit::badge_judgement(tone, format!("{}: {}", c.check.as_str(), tone.word())),
        kit::data_chip(c.section),
    ]
    .spacing(sp.sm)
    .align_y(iced::Alignment::Center);
    match &c.verdict {
        Judgement::NotEstablished { reason } | Judgement::Unobservable { reason } => {
            r = r.push(kit::muted(reason.clone()));
        }
        Judgement::Established => {
            r = r.push(kit::muted(kit::plural(c.findings.len(), "finding")));
        }
        Judgement::NotAsked => {}
    }
    r.into()
}

fn finding_row<'a>(
    state: &'a DoctorState,
    f: &'a DoctorFinding,
    index: usize,
    namespace: &str,
    sp: Spacing,
) -> Element<'a, Message> {
    let is_new = state.delta.as_ref().is_some_and(|d| d.is_new(f));
    let mut header = row![
        kit::badge_severity(SeverityTone::of(f.severity), f.check.as_str()),
        kit::mono(f.subject.clone()),
    ]
    .spacing(sp.sm)
    .align_y(iced::Alignment::Center);
    // "new" is news about the run, not a second warning (#565): a chip in
    // commentary's tone, beside the finding's own severity badge.
    if is_new {
        header = header.push(kit::status_chip(crate::view::theme::Tone::Info, "new"));
    }
    header = header.push(iced::widget::space::horizontal());
    header = header.push(kit::data_chip(f.check.section()));
    let mut body = column![header, kit::muted(f.evidence.clone())].spacing(sp.xs);
    if finding_target(f, namespace).is_some() {
        body = body.push(
            kit::icon_button(kit::Icon::GoTo, Some("go to subject"))
                .on_press(msg(DoctorMsg::FindingClicked(index))),
        );
    }
    // An error's card wears danger's edge; the badge still says which.
    let edge = if f.severity == DoctorSeverity::Error {
        kit::Edge::Failed
    } else {
        kit::Edge::Put
    };
    kit::card(kit::edge_row(
        edge,
        iced::widget::container(body).padding(iced::Padding::ZERO.left(sp.sm)),
    ))
}
