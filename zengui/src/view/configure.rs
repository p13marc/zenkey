//! The Config tool (#481): a producer's configuration, read from what it
//! serves (RFC 05 §5.1) — the schema beside every running value.
//!
//! It reads, and it changes a `hot` group: each parameter edited by its
//! served kind, typed and validated here by the producer's own validator
//! before anything is sent, sent guarded by the read-back's revision and
//! keyed for a retry, previewed as a dry run on request. Every claim on
//! screen is the producer's own: the
//! groups and their classes, each parameter's kind, value, source and
//! startup value, a pending change and the last one, verbatim. What the
//! producer did not say is drawn as not said — a value it has not read back
//! is *unknown*, a sensitive one is *write-only*, a class or a kind this
//! build does not know is named as unknown — never a guessed default (O4).

use std::sync::Arc;
use std::time::Instant;

use iced::widget::{Column, column, row};
use iced::{Element, Length};
use zenkey::config::{ConfigView, GroupView, ParamClass, ParamKind, ParamView};
use zenkey_fleet::report::CallReport;

use crate::configure::{
    ConfigAct, ConfigForm, ConfigRead, ConfigTarget, GroupNote, Reply, Slot, change_of,
    class_words, kind_text, plain,
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
    /// A parameter's draft: what the user typed, or the segment they lit.
    Draft {
        group: String,
        param: String,
        text: String,
    },
    /// Send a group's change.
    Apply(String),
    /// Send a group's change as a dry run.
    Preview(String),
    /// Drop a group's drafts and its last outcome.
    Discard(String),
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
        Some(read) => reply(read, f, sp),
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
fn reply<'a>(read: &'a ConfigRead, form: &'a ConfigForm, sp: Spacing) -> Element<'a, Message> {
    let t = &read.target;
    match &read.reply {
        Reply::Document(view) => document(view, read, form, sp),
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
    form: &'a ConfigForm,
    sp: Spacing,
) -> Element<'a, Message> {
    let stale = form.stale();
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
        col = col.push(group(view, g, form, stale, sp));
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

/// One group: its name, its class in words, its parameters — and, when
/// this tool may write it, an editor per parameter and the group's apply.
fn group<'a>(
    view: &'a ConfigView,
    g: &'a GroupView,
    form: &'a ConfigForm,
    stale: bool,
    sp: Spacing,
) -> Element<'a, Message> {
    // Reach groups are written with a rollback window (#481's confirmed
    // commit); a contract group and an unknown class never are.
    let editable = g.class == ParamClass::Hot && !stale && form.in_flight.is_none();
    let mut col = column![
        row![
            kit::emphasis(g.name.clone()).font(face::MONO),
            kit::status_chip(class_tone(g.class), class_words(g.class)),
            kit::muted(g.description.clone()),
        ]
        .spacing(sp.sm)
        .align_y(iced::Alignment::Center),
    ]
    .spacing(sp.sm);
    if g.class == ParamClass::Contract {
        col = col.push(kit::muted(
            "changed in the producer's startup configuration and applied by a restart \
             (RFC 05 §5.1) — read-only here",
        ));
    }
    col = col.push(header(g.class == ParamClass::Hot, sp));
    let errors = match change_of(view, &g.name, &form.drafts) {
        Ok(_) => Vec::new(),
        Err(errors) => errors,
    };
    for p in &g.parameters {
        let slot: Slot = (g.name.clone(), p.spec.name.clone());
        let error = errors
            .iter()
            .find(|(name, _)| *name == p.spec.name)
            .map(|(_, why)| why.clone());
        col = col.push(param(
            p,
            (g.class == ParamClass::Hot).then(|| Editor {
                group: &g.name,
                draft: form.drafts.get(&slot).map(String::as_str),
                enabled: editable,
                error,
            }),
            sp,
        ));
    }
    if g.class == ParamClass::Hot {
        col = col.push(footer(view, g, form, editable, sp));
    }
    kit::card(col)
}

/// A hot group's apply row: the producer's own refusal of the change as it
/// stands, the last outcome, and the buttons.
fn footer<'a>(
    view: &'a ConfigView,
    g: &'a GroupView,
    form: &'a ConfigForm,
    editable: bool,
    sp: Spacing,
) -> Element<'a, Message> {
    let mut col = Column::new().spacing(sp.sm);
    let change = change_of(view, &g.name, &form.drafts);
    let mut ready = false;
    if let Ok(mut change) = change
        && !change.values.is_empty()
    {
        change.expected_revision = Some(view.revision);
        match view.schema().validate(&g.name, &change) {
            // The refusal the producer would send, before it is sent.
            Err(e) => col = col.push(kit::error(e.to_string())),
            Ok(()) => ready = true,
        }
    }
    if let Some(note) = form.notes.get(&g.name) {
        col = col.push(note_view(note, sp));
    }
    let busy = match &form.in_flight {
        Some(ConfigAct::Set(group)) if *group == g.name => Some("applying…"),
        Some(ConfigAct::Preview(group)) if *group == g.name => Some("previewing…"),
        _ => None,
    };
    let mut apply = kit::primary(kit::caption("apply"));
    let mut preview = kit::secondary(kit::caption("preview"));
    let mut discard = kit::ghost(kit::caption("discard"));
    if editable && ready {
        apply = apply.on_press(msg(ConfigMsg::Apply(g.name.clone())));
        preview = preview.on_press(msg(ConfigMsg::Preview(g.name.clone())));
    }
    if editable && form.drafts.keys().any(|(group, _)| *group == g.name) {
        discard = discard.on_press(msg(ConfigMsg::Discard(g.name.clone())));
    }
    let mut buttons = row![apply, preview, discard]
        .spacing(sp.sm)
        .align_y(iced::Alignment::Center);
    if let Some(busy) = busy {
        buttons = buttons.push(kit::muted(busy));
    }
    col.push(buttons).into()
}

/// What the last write to a group came to.
fn note_view<'a>(note: &'a GroupNote, sp: Spacing) -> Element<'a, Message> {
    match note {
        GroupNote::Applied { revision } => kit::status_chip(
            Tone::Positive,
            format!("applied — the document is at revision {revision}"),
        ),
        GroupNote::Preview(edits) if edits.is_empty() => kit::callout(
            Tone::Info,
            kit::caption("preview: the producer would change nothing"),
        ),
        GroupNote::Preview(edits) => {
            let mut col = column![kit::eyebrow("PREVIEW — WOULD CHANGE")].spacing(sp.xs);
            for e in edits {
                col = col.push(kit::caption(if e.redacted {
                    format!("{}: (write-only)", e.parameter)
                } else {
                    format!(
                        "{}: {} → {}",
                        e.parameter,
                        e.old.as_ref().map_or("—".to_string(), ToString::to_string),
                        e.new.as_ref().map_or("—".to_string(), ToString::to_string)
                    )
                }));
            }
            kit::callout(Tone::Info, col)
        }
        GroupNote::Refused(why) => kit::error(why.clone()),
        GroupNote::Unknown => kit::callout(
            Tone::Caution,
            kit::caption(
                "no reply within the timeout: whether it applied is unknown. Read back before \
                 deciding — apply again sends the same change with the same key, so the \
                 producer answers it once (RFC 05 §5.1)",
            ),
        ),
        GroupNote::Moved => kit::callout(
            Tone::Info,
            kit::caption(
                "the document had moved since it was read — read again, your edits kept. \
                 Apply sends them against what is there now",
            ),
        ),
    }
}

fn header<'a>(editable: bool, sp: Spacing) -> Element<'a, Message> {
    let cell = |s: &'static str, n: u16| {
        Element::from(iced::widget::container(kit::eyebrow(s)).width(Length::FillPortion(n)))
    };
    let mut r = row![cell("PARAMETER", 3), cell("VALUE", 3), cell("SOURCE", 2),].spacing(sp.sm);
    if editable {
        r = r.push(cell("NEW VALUE", 4));
    }
    r.push(cell("DESCRIPTION", 4)).into()
}

/// What a parameter row needs to draw its editor.
struct Editor<'a> {
    group: &'a str,
    draft: Option<&'a str>,
    enabled: bool,
    /// The draft's own refusal, when it does not parse as the kind.
    error: Option<String>,
}

/// One parameter row. A sensitive value is write-only by rule; any other
/// absent value is unknown — the producer had none to show.
fn param<'a>(p: &'a ParamView, editor: Option<Editor<'a>>, sp: Spacing) -> Element<'a, Message> {
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
    let cell = |e: Element<'a, Message>, n: u16| {
        Element::from(iced::widget::container(e).width(Length::FillPortion(n)))
    };
    let mut r = row![
        cell(
            column![
                kit::caption(p.spec.name.clone()).font(face::MONO),
                kit::muted(kind_text(&p.spec.kind)),
            ]
            .into(),
            3
        ),
        cell(value, 3),
        cell(source, 2),
    ]
    .spacing(sp.sm)
    .align_y(iced::Alignment::Center);
    if let Some(e) = editor {
        r = r.push(cell(edit_control(p, e, sp), 4));
    }
    r.push(cell(kit::muted(p.spec.description.clone()), 4))
        .into()
}

/// The control a parameter is edited with, by its served kind: a segmented
/// false/true for a bool — which can show neither lit, the value unknown —
/// a box for an integer or a text, a secure box for a sensitive value that
/// is never pre-filled, and nothing for a kind this build does not know.
fn edit_control<'a>(p: &'a ParamView, e: Editor<'a>, sp: Spacing) -> Element<'a, Message> {
    let group = e.group.to_string();
    let param = p.spec.name.clone();
    let draft = move |text: String| {
        msg(ConfigMsg::Draft {
            group: group.clone(),
            param: param.clone(),
            text,
        })
    };
    let control: Element<'a, Message> = match &p.spec.kind {
        ParamKind::Bool => {
            let lit = e.draft.map(|d| d == "true").or(match p.value {
                Some(zenkey::config::ParamValue::Bool(b)) => Some(b),
                _ => None,
            });
            let segment = |v: bool| kit::Segment {
                value: v,
                label: v.to_string(),
                icon: None,
                count: None,
                tip: None,
            };
            if e.enabled {
                kit::segmented(vec![segment(false), segment(true)], lit, move |v: bool| {
                    draft(v.to_string())
                })
            } else {
                kit::muted(lit.map_or("—".to_string(), |b| b.to_string()))
            }
        }
        ParamKind::Integer { .. } | ParamKind::Text => {
            let current = if p.spec.sensitive {
                String::new()
            } else {
                p.value.as_ref().map(plain).unwrap_or_default()
            };
            let text = e.draft.map_or(current.clone(), str::to_string);
            let example = if p.spec.sensitive {
                "type to replace".to_string()
            } else {
                current
            };
            let mut input = kit::input(&example, &text)
                .secure(p.spec.sensitive)
                .font(face::MONO)
                .size(font::CAPTION);
            if e.enabled {
                input = input.on_input(draft);
            }
            input.into()
        }
        _ => kit::muted("read-only — a kind this build does not know"),
    };
    match e.error {
        None => control,
        Some(why) => column![control, kit::error(why)].spacing(sp.xs).into(),
    }
}
