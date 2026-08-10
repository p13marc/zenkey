//! The doctor panel (#71): run-on-demand rendering of the engine's typed
//! findings — the same `DoctorReport` struct `zenctl doctor --format json`
//! emits, grouped by severity with RFC citations, with run-over-run deltas.
//!
//! Never ambient: a doctor run fans real queries across the fleet, so it
//! costs exactly one button press (the laziness ground rule).

use iced::widget::{checkbox, column, row, scrollable, text};
use iced::{Element, Length};
use zenkey_fleet::report::{DoctorFinding, DoctorReport, DoctorSeverity};

use crate::doctor::{DoctorState, finding_target};
use crate::message::Message;
use crate::view::kit;
use crate::view::theme::{SeverityTone, colors};
use crate::view::tokens::{font, space};

/// The panel's interactions, nested per the `CallMsg` precedent.
#[derive(Debug, Clone)]
pub enum DoctorMsg {
    /// The run button — the only way a sweep starts.
    Run,
    DeepToggled(bool),
    /// A run finished.
    Done(Result<std::sync::Arc<DoctorReport>, String>),
    /// A finding row was clicked — navigate to its subject.
    FindingClicked(usize),
    /// Drop every cached `describe` so the next decode asks the bus again
    /// (issue #101). A positive entry never expires on its own, so a producer
    /// that changed its served set mid-session is otherwise read forever with
    /// the schemas it had at first contact.
    ReaskSchemas,
}

fn tone(severity: DoctorSeverity) -> SeverityTone {
    match severity {
        DoctorSeverity::Error => SeverityTone::Error,
        DoctorSeverity::Warning => SeverityTone::Warning,
        DoctorSeverity::Info => SeverityTone::Info,
    }
}

pub fn pane<'a>(state: &'a DoctorState, base: &'a str) -> Element<'a, Message> {
    let run_label = if state.in_flight {
        "running…"
    } else {
        "run doctor"
    };
    let mut run = iced::widget::button(text(run_label).size(font::CAPTION)).padding(4);
    if !state.in_flight {
        run = run.on_press(Message::Doctor(DoctorMsg::Run));
    }
    let deep = checkbox(state.deep)
        .label("deep: freshness + storage sweeps (adds query load)")
        .size(font::CAPTION)
        .text_size(font::CAPTION)
        .on_toggle(|b| Message::Doctor(DoctorMsg::DeepToggled(b)));

    // The schema cache's escape hatch lives here because it is the same kind
    // of thing as the run button: an explicit, costed re-ask, never ambient.
    let reask = iced::widget::button(text("re-ask schemas").size(font::CAPTION))
        .padding(4)
        .on_press(Message::Doctor(DoctorMsg::ReaskSchemas));

    let mut col = column![
        kit::section_header("doctor", None),
        row![run, deep]
            .spacing(space::MD)
            .align_y(iced::Alignment::Center),
        row![
            reask,
            kit::muted(match state.schemas_forgotten {
                0 => "schema cache: kept for the session; a producer that changes \
                      its served set is read with the schemas it had at first contact"
                    .to_string(),
                n => format!(
                    "schema cache cleared {} — the next decode asks the bus again",
                    kit::plural(n, "time")
                ),
            }),
        ]
        .spacing(space::MD)
        .align_y(iced::Alignment::Center),
    ]
    .spacing(space::SM);

    if let Some(e) = &state.error {
        col = col.push(
            text(format!("doctor run failed: {e}"))
                .size(font::CAPTION)
                .style(|theme: &iced::Theme| text::Style {
                    color: Some(colors(theme).danger()),
                }),
        );
    }

    let Some(report) = state.current.as_deref() else {
        // Never-run is not "0 findings" (O4).
        col = col.push(kit::empty_state(
            "no doctor run yet",
            "findings are produced on demand — a sweep queries the fleet",
        ));
        return col.into();
    };

    // The coverage summary is what makes an empty findings list legible
    // (RFC 05 §3.1: silence needs attribution).
    col = col.push(kit::muted(format!(
        "{} · checked {} introspect answer(s) against {} live producer(s) · \
         {} describe served, {} missing · {} router(s)",
        kit::plural(report.findings.len(), "finding"),
        report.introspect_answered,
        report.live_producers,
        report.describe_served,
        report.describe_missing,
        report.routers,
    )));

    if let Some(d) = &state.delta {
        col = col.push(kit::muted(format!(
            "vs previous run: {} new · {} fixed · {} unchanged",
            d.new.len(),
            d.fixed.len(),
            d.unchanged
        )));
    }

    let mut list = column![].spacing(space::SM);
    for severity in [
        DoctorSeverity::Error,
        DoctorSeverity::Warning,
        DoctorSeverity::Info,
    ] {
        let group: Vec<(usize, &DoctorFinding)> = report
            .findings
            .iter()
            .enumerate()
            .filter(|(_, f)| f.severity == severity)
            .collect();
        if group.is_empty() {
            continue;
        }
        list = list.push(kit::section_header(
            format!("{severity:?}").to_lowercase(),
            None,
        ));
        for (i, f) in group {
            list = list.push(finding_row(state, f, i, base));
        }
    }
    if let Some(d) = &state.delta
        && !d.fixed.is_empty()
    {
        list = list.push(kit::section_header("fixed since last run", None));
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

fn finding_row<'a>(
    state: &'a DoctorState,
    f: &'a DoctorFinding,
    index: usize,
    base: &str,
) -> Element<'a, Message> {
    let is_new = state
        .delta
        .as_ref()
        .is_some_and(|d| d.new.contains(&(f.check.clone(), f.subject.clone())));
    let mut header = row![
        kit::badge_severity(tone(f.severity), &f.check),
        kit::mono(f.subject.clone()),
    ]
    .spacing(space::SM)
    .align_y(iced::Alignment::Center);
    if is_new {
        header = header.push(kit::badge_severity(SeverityTone::Warning, "new"));
    }
    if let Some(c) = &f.citation {
        header = header.push(iced::widget::space::horizontal());
        header = header.push(kit::muted(c.clone()));
    }
    let mut body = column![header, kit::muted(f.evidence.clone())].spacing(space::XS);
    if finding_target(f, base).is_some() {
        body = body.push(
            iced::widget::button(text("go to subject").size(font::CAPTION))
                .padding(2)
                .on_press(Message::Doctor(DoctorMsg::FindingClicked(index))),
        );
    }
    kit::card(body)
}
