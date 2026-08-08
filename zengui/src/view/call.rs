//! The publish/call pane (§6.4 item 3, issue #60) — the GUI face of the
//! write facade.
//!
//! Everything writes through the engine: the QoS picker *is* the closed enum,
//! the target parses through [`CallTarget`](zenkey_fleet::CallTarget) (a
//! hostname fails with the RFC 06 §6 pointer, exactly like the CLI), and a
//! fleet call to a forbidden-fanout procedure is refused by the engine's
//! registry guard — this pane adds the visual layer: the fleet option is
//! *labelled* refused as soon as the selected procedure declares it.

use iced::widget::{Column, button, column, pick_list, row, text, text_input};
use iced::{Element, Length};
use zenkey_fleet::SliceSet;
use zenkey_fleet::report::CallReport;

use crate::message::Message;
use crate::view::kit;
use crate::view::theme::colors;
use crate::view::tokens::{font, space};

/// The pane's editable state (owned by the app).
#[derive(Debug, Clone, Default)]
pub struct CallForm {
    pub producer: Option<String>,
    pub procedure: Option<String>,
    pub target: String,
    pub params: String,
    pub body: String,
    /// The last outcome: a report, or the refusal/error text.
    pub outcome: Option<Result<CallReport, String>>,
    pub in_flight: bool,
}

/// Messages the pane emits (wrapped into the app's `Message::Call`).
#[derive(Debug, Clone)]
pub enum CallMsg {
    ProducerPicked(String),
    ProcedurePicked(String),
    TargetChanged(String),
    ParamsChanged(String),
    BodyChanged(String),
    Submit,
}

/// Render the pane. `slices` scaffolds the pickers; without a registry the
/// pane says so instead of presenting empty dropdowns as knowledge (O4).
pub fn pane<'a>(form: &'a CallForm, slices: Option<&'a SliceSet>) -> Element<'a, Message> {
    let Some(slices) = slices else {
        return kit::empty_state(
            "No registry loaded",
            "The call pane scaffolds its forms from registry slices — none are \
             loaded yet. \"Not asked\" is not \"nothing serves\" (RFC 09 §5.1 O4).",
        );
    };

    let producers: Vec<String> = slices
        .slices()
        .iter()
        .filter(|s| !s.procedures.is_empty())
        .map(|s| s.name.clone())
        .collect();
    if producers.is_empty() {
        return kit::empty_state(
            "No procedures declared",
            "The loaded slices declare no [[procedure]] entries.",
        );
    }

    let producer_pick = pick_list(producers, form.producer.clone(), |p| {
        Message::Call(CallMsg::ProducerPicked(p))
    })
    .placeholder("producer")
    .text_size(font::CAPTION);

    let procedures: Vec<String> = form
        .producer
        .as_deref()
        .and_then(|p| slices.get(p))
        .map(|s| s.procedures.iter().map(|p| p.path.clone()).collect())
        .unwrap_or_default();
    let procedure_pick = pick_list(procedures, form.procedure.clone(), |p| {
        Message::Call(CallMsg::ProcedurePicked(p))
    })
    .placeholder("procedure")
    .text_size(font::CAPTION);

    // The declared shape of the selected procedure, and the fanout verdict.
    let decl = form
        .producer
        .as_deref()
        .zip(form.procedure.as_deref())
        .and_then(|(prod, proc)| {
            slices
                .get(prod)
                .and_then(|s| s.procedures.iter().find(|p| p.path == proc))
        });
    let fanout_forbidden = decl
        .map(|d| d.fanout.as_deref() == Some("forbidden"))
        .unwrap_or(false);

    let mut meta = Column::new().spacing(2);
    if let Some(d) = decl {
        meta = meta.push(kit::muted(format!(
            "kind {} · request {} · reply {}",
            d.kind,
            d.request.as_deref().unwrap_or("—"),
            d.reply.as_deref().unwrap_or("—"),
        )));
        if fanout_forbidden {
            meta = meta.push(
                text("fanout = \"forbidden\" — a fleet (*) target is refused (RFC 05 §2.1)")
                    .size(font::CAPTION)
                    .style(|theme: &iced::Theme| text::Style {
                        color: Some(colors(theme).danger()),
                    }),
            );
        }
    }

    let target = text_input("target: h-… | @service | *", &form.target)
        .on_input(|t| Message::Call(CallMsg::TargetChanged(t)))
        .size(font::CAPTION);
    let params = text_input("params: k=v;k=v (selector)", &form.params)
        .on_input(|t| Message::Call(CallMsg::ParamsChanged(t)))
        .size(font::CAPTION);
    let body = text_input("body: JSON (query payload)", &form.body)
        .on_input(|t| Message::Call(CallMsg::BodyChanged(t)))
        .size(font::CAPTION);

    let ready = decl.is_some()
        && !form.target.is_empty()
        && !(fanout_forbidden && form.target == "*")
        && !form.in_flight;
    let mut submit =
        button(text(if form.in_flight { "calling…" } else { "call" }).size(font::CAPTION))
            .padding(4);
    if ready {
        submit = submit.on_press(Message::Call(CallMsg::Submit));
    }

    let mut col = column![
        kit::section_header("Call", None),
        row![producer_pick, procedure_pick]
            .spacing(space::SM)
            .align_y(iced::Alignment::Center),
        meta,
        target,
        params,
        body,
        submit,
    ]
    .spacing(space::SM);

    if let Some(outcome) = &form.outcome {
        col = col.push(outcome_view(outcome));
    }

    iced::widget::scrollable(col).height(Length::Fill).into()
}

fn outcome_view(outcome: &Result<CallReport, String>) -> Element<'_, Message> {
    match outcome {
        Err(e) => text(format!("refused / failed: {e}"))
            .size(font::CAPTION)
            .style(|theme: &iced::Theme| text::Style {
                color: Some(colors(theme).danger()),
            })
            .into(),
        Ok(report) => {
            let mut col = Column::new().spacing(2);
            col = col.push(kit::mono(format!("→ {}", report.key)));
            if report.answers.is_empty() {
                // Exit-code 2's meaning, rendered: silence is not a verdict.
                col = col.push(kit::muted(
                    "no replies within the timeout — a non-verdict, not proof of \
                     absence (RFC 05 §3.1); the roster says who should have answered",
                ));
            }
            for a in &report.answers {
                let line = match (&a.error, &a.value, &a.text) {
                    (Some(err), _, _) => format!("{}  ✗ {}: {}", a.origin, err.name, err.message),
                    (None, Some(v), _) => format!("{}  ✓ {}", a.origin, v),
                    (None, None, Some(t)) => {
                        format!("{}  ✓ {}", a.origin, t.lines().next().unwrap_or(""))
                    }
                    _ => format!("{}  ✓", a.origin),
                };
                col = col.push(kit::mono(line));
            }
            col.into()
        }
    }
}
