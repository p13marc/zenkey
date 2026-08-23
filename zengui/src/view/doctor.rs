//! The doctor panel (#71): run-on-demand rendering of the engine's typed
//! findings — the same `DoctorReport` struct `zenctl doctor --format json`
//! emits, grouped by severity with RFC citations, with run-over-run deltas.
//!
//! Never ambient: a doctor run fans real queries across the fleet, so it
//! costs exactly one button press (the laziness ground rule).

use iced::widget::{column, row, scrollable, text};
use iced::{Element, Length};
use zenkey_fleet::report::{DoctorFinding, DoctorSeverity};

use crate::doctor::{DoctorState, finding_target};
use crate::message::{Message, PaneMsg};
use crate::view::kit;
use crate::view::theme::{SeverityTone, colors};
use crate::view::tokens::{Spacing, font};

/// The panel's interactions, nested per the `CallMsg` precedent.
#[derive(Debug, Clone)]
pub enum DoctorMsg {
    /// The run button — the only way a sweep starts.
    Run,
    DeepToggled(bool),
    /// The `--listen` window in seconds (#161); empty = off.
    ListenChanged(String),
    /// A run finished.
    Done(Result<crate::doctor::DoctorRun, String>),
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

/// Wrap one of this pane's messages for the app (#176).
///
/// One place the pane's name is spelled, rather than at every widget —
/// which is what the other seven panes already did, and what makes the
/// six-group regroup a one-line change here instead of 5.
fn msg(m: DoctorMsg) -> Message {
    Message::Pane(PaneMsg::Doctor(m))
}

/// The doctor's results — the Activity dock's Doctor stream (#183).
///
/// It stopped being a *place* and became an action: the run is a palette
/// command, and this is where its verdict lands. That is the honest shape,
/// because a doctor run is a thing you do to a session, not a view of the
/// subject.
pub fn section<'a>(state: &'a DoctorState, base: &'a str, sp: Spacing) -> Element<'a, Message> {
    let run_label = if state.in_flight {
        "running…"
    } else {
        "run doctor"
    };
    let mut run = kit::action(kit::caption(run_label)).padding(sp.xs);
    if !state.in_flight {
        run = run.on_press(msg(DoctorMsg::Run));
    }
    let deep = kit::check(state.deep)
        .label("deep: freshness + storage sweeps (adds query load)")
        .size(font::CAPTION)
        .text_size(font::CAPTION)
        .on_toggle(|b| msg(DoctorMsg::DeepToggled(b)));
    // The listen window (#161): off by default — a passive phase still holds
    // subscribers open, and ambient cost is the thing this panel refuses.
    let listen = kit::input("listen (s, empty = off)", &state.listen)
        .on_input(|t| msg(DoctorMsg::ListenChanged(t)))
        .size(font::CAPTION)
        .width(Length::Fixed(140.0));

    // The schema cache's escape hatch lives here because it is the same kind
    // of thing as the run button: an explicit, costed re-ask, never ambient.
    let reask = kit::action(kit::caption("re-ask schemas"))
        .padding(sp.xs)
        .on_press(msg(DoctorMsg::ReaskSchemas));

    let mut col = column![
        kit::section_header("doctor", None),
        row![run, deep, listen]
            .spacing(sp.md)
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
        .spacing(sp.md)
        .align_y(iced::Alignment::Center),
    ]
    .spacing(sp.sm);

    if let Some(e) = &state.error {
        col = col.push(kit::body(format!("doctor run failed: {e}")).style(
            |theme: &iced::Theme| text::Style {
                color: Some(colors(theme).danger()),
            },
        ));
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

    // R1: `synced: None` means the served-vs-declared diff never ran (no
    // registry loaded) — which must not read as "nothing out of sync"
    // (RFC 09 §5.1 O4).
    if report.synced.is_not_asked() {
        col = col.push(kit::muted(
            "no registry loaded — the served-vs-declared diff never ran; \
             \"not checked\" is not \"in sync\" (RFC 09 §5.1 O4)",
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

    // The listen phase's scope statement (#161): what was watched, for how
    // long, and what the bounded observer dropped (O5/O6).
    if let Some(obs) = &report.observation {
        col = col.push(kit::muted(format!(
            "listened {:.0}s over {}: {} on {}, {} dropped{}{}",
            obs.window_s,
            kit::plural(obs.scopes.len(), "scope"),
            kit::plural(obs.samples as usize, "sample"),
            kit::plural(obs.keys_seen, "key"),
            obs.dropped,
            if obs.dropped > 0 {
                " (findings cover only what was seen — O6)"
            } else {
                ""
            },
            if obs.synthetic_marked > 0 {
                format!(" · {} synthetic-marked", obs.synthetic_marked)
            } else {
                String::new()
            },
        )));
        if obs.facts_evicted > 0 {
            // The bounded facts cache's cost (#107): the key figures cover
            // the retained keys only (O6).
            col = col.push(kit::muted(format!(
                "{} retired at the facts-cache bound — key figures cover the \
                 retained keys only (O6)",
                kit::plural(obs.facts_evicted as usize, "key projection"),
            )));
        }
    }

    let mut list = column![].spacing(sp.sm);
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
            list = list.push(finding_row(state, f, i, base, sp));
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
    sp: Spacing,
) -> Element<'a, Message> {
    let is_new = state
        .delta
        .as_ref()
        .is_some_and(|d| d.new.contains(&(f.check.clone(), f.subject.clone())));
    let mut header = row![
        kit::badge_severity(tone(f.severity), &f.check),
        kit::mono(f.subject.clone()),
    ]
    .spacing(sp.sm)
    .align_y(iced::Alignment::Center);
    if is_new {
        header = header.push(kit::badge_severity(SeverityTone::Warning, "new"));
    }
    if let Some(c) = &f.citation {
        header = header.push(iced::widget::space::horizontal());
        header = header.push(kit::muted(c.clone()));
    }
    let mut body = column![header, kit::muted(f.evidence.clone())].spacing(sp.xs);
    if finding_target(f, base).is_some() {
        body = body.push(
            kit::action(kit::caption("go to subject"))
                .padding([0.0, sp.xs])
                .on_press(msg(DoctorMsg::FindingClicked(index))),
        );
    }
    kit::card(body)
}
