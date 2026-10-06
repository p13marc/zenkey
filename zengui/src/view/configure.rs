//! The Config tool (#481): a producer's configuration, read from what it
//! serves (RFC 05 §5.1) — the schema beside every running value.
//!
//! The first cut reads. Every claim on screen is the producer's own: the
//! groups and their classes, each parameter's kind, value, source and
//! startup value, a pending change and the last one, verbatim. What the
//! producer did not say is drawn as not said — a value it has not read back
//! is *unknown*, a sensitive one is *write-only*, a class or a kind this
//! build does not know is named as unknown — never a guessed default (O4).

use std::sync::Arc;
use std::time::Instant;

use iced::widget::{Column, column, row};
use iced::{Element, Length};
use zenkey::config::{ConfigView, GroupView, ParamClass, ParamView};
use zenkey_fleet::report::CallReport;

use crate::configure::{
    ConfigAct, ConfigForm, ConfigRead, ConfigTarget, Reply, class_words, kind_text,
};
use crate::message::{Message, PaneMsg};
use crate::services::ServiceError;
use crate::view::kit;
use crate::view::theme::Tone;
use crate::view::tokens::{Spacing, face, font};

/// The tool's interactions.
#[derive(Debug, Clone)]
pub enum ConfigMsg {
    OriginChanged(String),
    ProducerChanged(String),
    ResourceChanged(String),
    /// Read the target's read-back.
    Read,
    /// Point the form at a resource and read it — the entry points' message
    /// (a node, an Inspector key), so arriving here is one act.
    Open(ConfigTarget),
    /// A call landed.
    Answered(ConfigAct, Result<Arc<CallReport>, ServiceError>),
}

fn msg(m: ConfigMsg) -> Message {
    Message::Pane(PaneMsg::Config(m))
}

/// Everything the tool reads, as plain data.
pub struct ConfigData<'a> {
    pub form: &'a ConfigForm,
    /// Producers whose slice declares `config/…` — the picker's offer.
    pub producers: Vec<String>,
    /// Origins the roster has seen running the named producer.
    pub origins: Vec<String>,
    /// Whether a session exists to ask through.
    pub session: bool,
    pub sp: Spacing,
}

pub fn pane<'a>(d: ConfigData<'a>) -> Element<'a, Message> {
    let sp = d.sp;
    let f = d.form;
    let mut col = column![kit::section_header("Configuration", None)].spacing(sp.md);
    col = col.push(target_row(&d));

    col = col.push(match &f.read {
        _ if f.in_flight == Some(ConfigAct::Read) => kit::muted(format!(
            "reading {} from {}…",
            f.target.read_path(),
            f.target.origin.trim()
        )),
        None if !d.session => kit::empty(
            kit::EmptyKind::NotAsked,
            "Not connected",
            "A configuration is read from its producer over the bus; connect first.",
        ),
        None => kit::empty(
            kit::EmptyKind::NotAsked,
            "Not read yet",
            "Name an origin, a producer and a resource, then read: the producer serves its \
             schema beside every value (RFC 05 §5.1). Nothing is asked until you do.",
        ),
        Some(read) => reply(read, f.stale(), sp),
    });
    iced::widget::scrollable(col)
        .height(Length::Fill)
        .spacing(sp.xs)
        .into()
}

/// ORIGIN · PRODUCER · RESOURCE, each typed or picked, and the read.
fn target_row<'a>(d: &ConfigData<'a>) -> Element<'a, Message> {
    let sp = d.sp;
    let t = &d.form.target;
    let typed = |example: &str, value: &'a str, on: fn(String) -> ConfigMsg| {
        kit::input(example, value)
            .on_input(move |s| msg(on(s)))
            .on_submit(msg(ConfigMsg::Read))
            .font(face::MONO)
            .size(font::CAPTION)
    };
    let with_picks =
        |input: Element<'a, Message>, options: Vec<String>, on: fn(String) -> ConfigMsg| {
            if options.is_empty() {
                return input;
            }
            row![
                input,
                kit::picker(options, None::<String>, move |s| msg(on(s)))
                    .placeholder("pick…")
                    .text_size(font::CAPTION)
            ]
            .spacing(sp.xs)
            .align_y(iced::Alignment::Center)
            .into()
        };
    let unaskable = t.unaskable();
    let mut read = kit::primary(kit::labelled(kit::Icon::Open, "read"));
    if unaskable.is_none() && d.session && d.form.in_flight.is_none() {
        read = read.on_press(msg(ConfigMsg::Read));
    }
    let mut col = column![
        row![
            kit::form_field(
                "ORIGIN",
                with_picks(
                    typed("h-3fa9c2d41b7e", &t.origin, ConfigMsg::OriginChanged).into(),
                    d.origins.clone(),
                    ConfigMsg::OriginChanged,
                ),
                None,
            ),
            kit::form_field(
                "PRODUCER",
                with_picks(
                    typed("radio", &t.producer, ConfigMsg::ProducerChanged).into(),
                    d.producers.clone(),
                    ConfigMsg::ProducerChanged,
                ),
                None,
            ),
            kit::form_field(
                "RESOURCE",
                typed("wlan0", &t.resource, ConfigMsg::ResourceChanged),
                None,
            ),
        ]
        .spacing(sp.sm),
        row![read].spacing(sp.sm).align_y(iced::Alignment::Center),
    ]
    .spacing(sp.sm);
    if let Some(why) = unaskable.filter(|_| !t.origin.is_empty() || !t.producer.is_empty()) {
        col = col.push(kit::muted(why));
    }
    col.into()
}

/// One landed read, drawn as what it is.
fn reply<'a>(read: &'a ConfigRead, stale: bool, sp: Spacing) -> Element<'a, Message> {
    let t = &read.target;
    match &read.reply {
        Reply::Document(view) => document(view, read, stale, sp),
        Reply::Refused { name, message } => kit::error(format!("{name} — {message}")),
        Reply::Silent => kit::empty(
            kit::EmptyKind::Silent,
            "No reply",
            format!(
                "{} did not answer {} within the timeout. Nobody serves it, whoever does was \
                 down, or the timeout was short — and the three are different (RFC 05 §3.1).",
                t.origin.trim(),
                t.read_path()
            ),
        ),
        Reply::Other(text) => column![
            kit::error("the reply is not a read-back document (RFC 05 §5.1) — shown as sent"),
            kit::inset(kit::caption(text.clone()).font(face::MONO)).width(Length::Fill),
        ]
        .spacing(sp.sm)
        .into(),
        Reply::Failed(why) => kit::error(why.clone()),
    }
}

/// The read-back: who answered and when, the change pending and the last
/// one, then a card per group.
fn document<'a>(
    view: &'a ConfigView,
    read: &'a ConfigRead,
    stale: bool,
    sp: Spacing,
) -> Element<'a, Message> {
    let age = Instant::now()
        .saturating_duration_since(read.at)
        .as_secs_f32();
    let mut col = Column::new().spacing(sp.md);
    col = col.push(kit::fields(vec![
        kit::field(
            "ORIGIN",
            kit::caption(read.target.origin.trim().to_string()).font(face::MONO),
        ),
        kit::field(
            "PRODUCER",
            kit::caption(read.target.producer.trim().to_string()).font(face::MONO),
        ),
        kit::field(
            "RESOURCE",
            kit::caption(view.resource.clone()).font(face::MONO),
        ),
        kit::field(
            "REVISION",
            kit::caption(view.revision.to_string()).font(face::MONO),
        ),
        kit::field(
            "READ",
            kit::caption(match kit::age_word(age).as_str() {
                "now" => "just now".to_string(),
                word => format!("{word} ago"),
            }),
        ),
    ]));
    if stale {
        col = col.push(kit::callout(
            Tone::Info,
            kit::caption("the target was edited since this was read — this document is about the one above; read again"),
        ));
    }
    if let Some(p) = &view.pending {
        // Armed, not a verdict: the mode's tone (#544), the producer's own
        // words for the deadline.
        col = col.push(kit::callout(
            Tone::Mode,
            column![
                row![
                    kit::status(Tone::Mode, "pending change"),
                    kit::data_chip(p.token.clone()),
                ]
                .spacing(sp.sm)
                .align_y(iced::Alignment::Center),
                kit::caption(format!(
                    "on {} — rolls back at {} unless confirmed",
                    p.groups.join(", "),
                    p.deadline.as_deref().unwrap_or("(no deadline stated)")
                )),
            ]
            .spacing(sp.xs),
        ));
    }
    if let Some(l) = &view.last_change {
        col = col.push(
            row![
                kit::eyebrow("LAST CHANGE"),
                kit::data_chip(l.token.clone()),
                kit::muted(format!("on {} — what persist takes", l.groups.join(", "))),
            ]
            .spacing(sp.sm)
            .align_y(iced::Alignment::Center),
        );
    }
    if view.groups.is_empty() {
        col = col.push(kit::empty(
            kit::EmptyKind::Empty,
            "No groups",
            "The producer answered and declares nothing configurable on this resource.",
        ));
    }
    for g in &view.groups {
        col = col.push(group(g, sp));
    }
    col.into()
}

fn class_tone(class: ParamClass) -> Tone {
    match class {
        ParamClass::Hot => Tone::Info,
        ParamClass::Reach => Tone::Caution,
        _ => Tone::Neutral,
    }
}

/// One group: its name, its class in words, and its parameters.
fn group<'a>(g: &'a GroupView, sp: Spacing) -> Element<'a, Message> {
    let mut col = column![
        row![
            kit::emphasis(g.name.clone()).font(face::MONO),
            kit::status_chip(class_tone(g.class), class_words(g.class)),
            kit::muted(g.description.clone()),
        ]
        .spacing(sp.sm)
        .align_y(iced::Alignment::Center),
        header(sp),
    ]
    .spacing(sp.sm);
    for p in &g.parameters {
        col = col.push(param(p, sp));
    }
    kit::card(col)
}

fn header<'a>(sp: Spacing) -> Element<'a, Message> {
    let cell = |s: &'static str, n: u16| {
        iced::widget::container(kit::eyebrow(s)).width(Length::FillPortion(n))
    };
    row![
        cell("PARAMETER", 3),
        cell("VALUE", 3),
        cell("SOURCE", 2),
        cell("KIND", 3),
        cell("DESCRIPTION", 5),
    ]
    .spacing(sp.sm)
    .into()
}

/// One parameter row. A sensitive value is write-only by rule; any other
/// absent value is unknown — the producer had none to show.
fn param<'a>(p: &'a ParamView, sp: Spacing) -> Element<'a, Message> {
    let value: Element<'a, Message> = if p.spec.sensitive {
        kit::status_chip(Tone::Neutral, "write-only")
    } else {
        match &p.value {
            None => kit::status_chip(Tone::Neutral, "unknown"),
            Some(v) => {
                let mut r = row![kit::caption(v.to_string()).font(face::MONO)]
                    .spacing(sp.xs)
                    .align_y(iced::Alignment::Center);
                if let Some(s) = &p.startup {
                    r = r.push(kit::muted(format!("startup {s}")));
                }
                r.into()
            }
        }
    };
    let source: Element<'a, Message> = match p.source {
        Some(s) => kit::data_chip(s.token()),
        None => kit::muted("—"),
    };
    let cell =
        |e: Element<'a, Message>, n: u16| iced::widget::container(e).width(Length::FillPortion(n));
    row![
        cell(kit::caption(p.spec.name.clone()).font(face::MONO).into(), 3),
        cell(value, 3),
        cell(source, 2),
        cell(kit::muted(kind_text(&p.spec.kind)), 3),
        cell(kit::muted(p.spec.description.clone()), 5),
    ]
    .spacing(sp.sm)
    .align_y(iced::Alignment::Center)
    .into()
}
