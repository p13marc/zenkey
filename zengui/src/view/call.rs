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

use std::sync::Arc;

use crate::message::{Message, PaneMsg};
use crate::view::kit;
use crate::view::theme::colors;
use crate::view::tokens::{font, space};

/// One field of a served request schema, as the form scaffolds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaField {
    pub name: String,
    /// The declared type, when the schema names one.
    pub type_name: Option<String>,
    pub required: bool,
}

impl SchemaField {
    /// `name: type*` — the star marks required, so a scaffolded body's
    /// mandatory half is legible before the send refuses it.
    pub fn label(&self) -> String {
        format!(
            "{}: {}{}",
            self.name,
            self.type_name.as_deref().unwrap_or("?"),
            if self.required { "*" } else { "" }
        )
    }

    /// A placeholder value of the right shape, for the scaffolded body.
    fn placeholder(&self) -> serde_json::Value {
        match self.type_name.as_deref() {
            Some("integer") => serde_json::json!(0),
            Some("number") => serde_json::json!(0.0),
            Some("boolean") => serde_json::json!(false),
            Some("array") => serde_json::json!([]),
            Some("object") => serde_json::json!({}),
            _ => serde_json::json!(""),
        }
    }
}

/// Extract the top-level field list from a served schema. Only the
/// `json-schema` kind describes fields in a form this can read; every other
/// kind returns an empty list, which the pane renders as "nothing to
/// scaffold" rather than as "no fields".
pub fn schema_fields(schema: &zenkey::schema::TypeSchema) -> Vec<SchemaField> {
    let Some(doc) = schema.json_document() else {
        return Vec::new();
    };
    let Some(props) = doc.get("properties").and_then(|p| p.as_object()) else {
        return Vec::new();
    };
    let required: Vec<&str> = doc
        .get("required")
        .and_then(|r| r.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    props
        .iter()
        .map(|(name, spec)| SchemaField {
            name: name.clone(),
            type_name: spec
                .get("type")
                .and_then(|t| t.as_str())
                .map(str::to_string),
            required: required.contains(&name.as_str()),
        })
        .collect()
}

/// The pane's editable state (owned by the app).
#[derive(Debug, Clone, Default)]
pub struct CallForm {
    pub producer: Option<String>,
    pub procedure: Option<String>,
    pub target: String,
    pub params: String,
    pub body: String,
    /// Attachment riding beside the body, verbatim — never schema-encoded
    /// (#117's rule on the call side, #126). Empty = none.
    pub attachment: String,
    /// The selected procedure's request-schema fields. `None` = **not asked**
    /// (no schema fetched yet), which is not "the request has no fields".
    pub request_fields: Option<Vec<SchemaField>>,
    /// The last outcome: a report, or the refusal/error text.
    pub outcome: Option<Result<CallReport, String>>,
    pub in_flight: bool,
}

impl CallForm {
    /// A skeleton body from the scaffolded fields — every field present, each
    /// with a placeholder of its declared shape.
    pub fn scaffold(&self) -> Option<String> {
        let fields = self.request_fields.as_ref()?;
        if fields.is_empty() {
            return None;
        }
        let mut obj = serde_json::Map::new();
        for f in fields {
            obj.insert(f.name.clone(), f.placeholder());
        }
        serde_json::to_string_pretty(&serde_json::Value::Object(obj)).ok()
    }
}

/// Messages the pane emits (wrapped into the app's `Message::Pane(PaneMsg::Call)`).
#[derive(Debug, Clone)]
pub enum CallMsg {
    ProducerPicked(String),
    ProcedurePicked(String),
    TargetChanged(String),
    ParamsChanged(String),
    BodyChanged(String),
    AttachmentChanged(String),
    /// Fill the body editor from the served request schema.
    ScaffoldBody,
    Submit,
    /// The selected procedure's request schema arrived (or did not).
    RequestSchema(Option<Vec<SchemaField>>),
    /// The call finished — its report, or why it did not (#176).
    ///
    /// Here rather than at `Message`'s top level, where it lived beside
    /// `RequestSchema`, which is the identical shape and was already nested. A
    /// message lives where its failure is displayed, and this one's `Err` is
    /// written into `CallForm::outcome`.
    Done(Result<Arc<zenkey_fleet::report::CallReport>, String>),
}

/// Wrap one of this pane's messages for the app (#176).
///
/// One place the pane's name is spelled, rather than at every widget —
/// which is what the other seven panes already did, and what makes the
/// six-group regroup a one-line change here instead of 8.
fn msg(m: CallMsg) -> Message {
    Message::Pane(PaneMsg::Call(m))
}

/// Render the pane. `slices` scaffolds the pickers; without a registry the
/// pane says so instead of presenting empty dropdowns as knowledge (O4). The
/// `roster` is what turns silence into a sentence: RFC 05 §3.1 says "no
/// reply" is not one condition, and the only way to say *which* origins did
/// not answer is to join the answers against who is up.
pub fn pane<'a>(
    form: &'a CallForm,
    slices: Option<&'a SliceSet>,
    roster: &'a crate::nodes::NodeRoster,
) -> Element<'a, Message> {
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
        msg(CallMsg::ProducerPicked(p))
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
        msg(CallMsg::ProcedurePicked(p))
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
        // Request-form scaffolding (§6.4 item 3): the served schema's fields,
        // and a button that drops a skeleton body in the editor. A declared
        // request type whose schema has not been fetched says "not asked" —
        // it must not read as "this procedure takes nothing" (O4).
        if let Some(request) = &d.request {
            meta = match &form.request_fields {
                Some(fields) if !fields.is_empty() => meta.push(
                    row![
                        kit::muted(format!(
                            "{request} fields: {}",
                            fields
                                .iter()
                                .map(SchemaField::label)
                                .collect::<Vec<_>>()
                                .join(", ")
                        )),
                        button(kit::caption("scaffold body"))
                            .padding(2)
                            .on_press(msg(CallMsg::ScaffoldBody)),
                    ]
                    .spacing(space::SM)
                    .align_y(iced::Alignment::Center),
                ),
                Some(_) => meta.push(kit::muted(format!(
                    "{request}'s served schema declares no fields to scaffold"
                ))),
                None => meta.push(kit::muted(format!(
                    "{request}: schema not asked yet — pick the procedure again once \
                     connected to scaffold from it"
                ))),
            };
        }
        if fanout_forbidden {
            meta = meta.push(
                kit::body("fanout = \"forbidden\" — a fleet (*) target is refused (RFC 05 §2.1)")
                    .style(|theme: &iced::Theme| text::Style {
                        color: Some(colors(theme).danger()),
                    }),
            );
        }
    }

    let target = text_input("target: h-… | @service | *", &form.target)
        .on_input(|t| msg(CallMsg::TargetChanged(t)))
        .size(font::CAPTION);
    let params = text_input("params: k=v;k=v (selector)", &form.params)
        .on_input(|t| msg(CallMsg::ParamsChanged(t)))
        .size(font::CAPTION);
    let body = text_input("body: JSON (query payload)", &form.body)
        .on_input(|t| msg(CallMsg::BodyChanged(t)))
        .size(font::CAPTION);
    let attachment = text_input(
        "attachment: verbatim, beside the body (empty = none)",
        &form.attachment,
    )
    .on_input(|t| msg(CallMsg::AttachmentChanged(t)))
    .size(font::CAPTION);

    let ready = decl.is_some()
        && !form.target.is_empty()
        && !(fanout_forbidden && form.target == "*")
        && !form.in_flight;
    let mut submit = button(kit::caption(if form.in_flight {
        "calling…"
    } else {
        "call"
    }))
    .padding(4);
    if ready {
        submit = submit.on_press(msg(CallMsg::Submit));
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
        attachment,
        submit,
    ]
    .spacing(space::SM);

    if let Some(outcome) = &form.outcome {
        col = col.push(outcome_view(outcome, form, roster));
    }

    iced::widget::scrollable(col).height(Length::Fill).into()
}

/// Origins the roster says are up and running the asked producer, that did
/// not answer. This is the join RFC 05 §3.1 asks for: "no reply" is not one
/// condition, and naming the silent origins is the difference between a
/// non-verdict a user can act on and one they cannot.
fn non_repliers(
    report: &CallReport,
    form: &CallForm,
    roster: &crate::nodes::NodeRoster,
) -> Vec<String> {
    let Some(producer) = form.producer.as_deref() else {
        return Vec::new();
    };
    roster
        .iter()
        .filter(|(_, producers)| {
            producers
                .get(producer)
                .is_some_and(|presence| presence.alive)
        })
        .map(|(origin, _)| origin.clone())
        .filter(|origin| !report.answers.iter().any(|a| &a.origin == origin))
        .collect()
}

fn outcome_view<'a>(
    outcome: &'a Result<CallReport, String>,
    form: &'a CallForm,
    roster: &'a crate::nodes::NodeRoster,
) -> Element<'a, Message> {
    match outcome {
        Err(e) => kit::body(format!("refused / failed: {e}"))
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
                // A reply attachment is a wire fact, shown where the reply
                // is — present only when the wire carried one (#126).
                if let (Some(att), Some(n)) = (&a.attachment, a.attachment_bytes) {
                    col = col.push(kit::muted(format!("   attachment ({n} B): {att}")));
                }
            }
            let silent = non_repliers(report, form, roster);
            if !silent.is_empty() {
                col = col.push(kit::muted(format!(
                    "did not answer, though alive: {}",
                    silent.join(", ")
                )));
            }
            col.into()
        }
    }
}
