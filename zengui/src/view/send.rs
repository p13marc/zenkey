//! The Send pane (#184): publish and call, one form — the GUI face of the
//! write facade (`zenctl topic pub` / `zenctl service call`).
//!
//! Publish (517 lines) and Call (369) carried the same target key, body
//! editor, schema validation, QoS vocabulary and result region, and differed
//! in exactly two things: fan-out policy, and whether replies come back. Two
//! panes with two message enums drifted apart; this is one pane with a mode
//! toggle — `put / publish` vs `get / call` — over one form. The per-origin
//! outcome list with its silent-origin join stays a distinct renderer,
//! because a fleet of replies is a genuinely different result from a
//! matching badge.
//!
//! Everything both modes promised, they still promise:
//!
//! **The body is encoded by the engine, never here.** The editor holds JSON;
//! `zenkey_fleet::prepare_publish` (#97) turns it into the bytes that ship,
//! against the producer's served schema, labelled with the declared encoding.
//! No GUI-side codec logic exists or may be added — a missing codec is a
//! zenkey-fleet issue (`docs/redesign-2026-07.md` §15). What the pane *does*
//! own is saying which of the three outcomes happened: encoded, sent as
//! typed, or sent raw (the write-side O4).
//!
//! **Publishing can be sustained.** A repeat interval drives a real stream,
//! and the publication's #38 matching badge says whether anyone is consuming
//! it — a routing fact about *this* publisher, never a fleet verdict
//! (RFC 05 §3.1). Every send is logged, the log is bounded, and when the
//! bound bites the pane reports what it dropped (RFC 09 §5.1 O6).
//!
//! **Calls go through the engine.** The target parses through
//! [`CallTarget`](zenkey_fleet::CallTarget) (a hostname fails with the
//! RFC 06 §6 pointer, exactly like the CLI), and a fleet call to a
//! forbidden-fanout procedure is refused by the engine's registry guard —
//! this pane adds the visual layer: the fleet option is *labelled* refused as
//! soon as the selected procedure declares it.

use std::collections::VecDeque;

use iced::widget::{Column, column, row, text};
use iced::{Element, Length};
use zenkey::qos::QosProfile;
use zenkey_fleet::report::CallReport;
use zenkey_fleet::{BodySource, KeyFacts, SliceSet};

use std::sync::Arc;

use crate::message::{Message, PaneMsg};
use crate::view::kit;
use crate::view::theme::colors;
use crate::view::tokens::{Spacing, font};

/// How many send-log lines the pane keeps. Bounded on purpose: a 5 Hz stream
/// left running overnight is 150k lines, and an explorer that grows without
/// bound is the O6 bug this project already fixed once for the key tree.
pub const LOG_LINES: usize = 200;

/// Which half of the write facade the form is driving (#184).
///
/// A mode, not a pane: the body, the attachment and the log survive the
/// toggle, because "I published this and now want to call about it" is one
/// session, not two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SendMode {
    #[default]
    Publish,
    Call,
}

impl SendMode {
    pub const ALL: [SendMode; 2] = [SendMode::Publish, SendMode::Call];

    pub fn label(self) -> &'static str {
        match self {
            SendMode::Publish => "put / publish",
            SendMode::Call => "get / call",
        }
    }
}

/// One line of the in-pane send log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub text: String,
    /// Failures render differently — a send that errored must not read like
    /// one that landed.
    pub ok: bool,
}

/// A QoS profile wrapped for the picker (`pick_list` needs `Display`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QosChoice(pub QosProfile);

impl std::fmt::Display for QosChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.name())
    }
}

/// Every profile, as picker options. Derived from the closed enum
/// (`QosProfile::ALL`), so the picker cannot drift from RFC 04 §3.
///
/// Built once for the process rather than once per frame (#178): the list is
/// five constants and the pane redraws at frame rate. `LazyLock` rather than a
/// `const` array so it still *derives* from `ALL` — a hand-written second list
/// is exactly the drift the doc comment above promises cannot happen.
pub fn qos_choices() -> &'static [QosChoice] {
    static CHOICES: std::sync::LazyLock<[QosChoice; QosProfile::ALL.len()]> =
        std::sync::LazyLock::new(|| QosProfile::ALL.map(QosChoice));
    &*CHOICES
}

/// The declared profile behind a classified key, when there is one (#158).
/// Drives the picker default until the user takes the picker over.
pub fn declared_qos(facts: Option<&KeyFacts>) -> Option<QosProfile> {
    use zenkey_fleet::facts::Registration;
    match facts.map(|f| &f.registration) {
        Some(Registration::Registered(s)) => s.declared_qos(),
        _ => None,
    }
}

/// One field of a served request schema, as the call mode scaffolds it.
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

/// The pane's editable state (owned by the app): one form, both modes (#184).
///
/// `body` and `attachment` are shared between the modes on purpose — they are
/// the same editor. The publish target (`key`, a full wire key) and the call
/// target (`target`, an origin / service token / `*`) are *not* shared,
/// because they are different questions and folding them would make the mode
/// toggle silently retarget a send.
#[derive(Debug, Clone)]
pub struct SendForm {
    pub mode: SendMode,

    // ── shared between the modes ────────────────────────────────────────
    /// JSON body; encoded for the wire by the engine (#97) on publish,
    /// shipped as the query payload on call.
    pub body: String,
    /// Attachment text riding beside the payload; empty = none. Shipped
    /// verbatim — never schema-encoded, the registry's vocabulary ends at
    /// the payload (#117, #126).
    pub attachment: String,
    pub in_flight: bool,

    // ── publish mode ────────────────────────────────────────────────────
    pub key: String,
    pub qos: QosChoice,
    /// The user picked a profile by hand — stop following the declared one
    /// (#158). A default that fought the operator would be worse than none.
    pub qos_touched: bool,
    /// Optional `Encoding` override; empty means "let the registry and the
    /// schema kind decide" (the engine's ladder).
    pub encoding: String,
    /// Send the bytes verbatim — the explicit escape hatch, mirroring
    /// `zenctl topic pub --raw`.
    pub raw: bool,
    /// Keep publishing on an interval once armed.
    pub repeat: bool,
    pub interval: String,
    /// Live classification of the composed key (the same ladder the tree
    /// uses). `None` before anything is typed.
    pub facts: Option<KeyFacts>,
    /// How the last prepared body was produced — the honesty line.
    pub source: Option<BodySource>,
    /// The engine's note about that preparation, rendered verbatim.
    pub note: Option<String>,
    /// The encoding actually set on the wire, once resolved.
    pub encoding_used: Option<String>,
    /// A publication is armed and repeating.
    pub armed: bool,
    /// Matching status of the armed publication (#38). `None` = **not asked**,
    /// which is not "nobody listens" (O4).
    pub matching: Option<bool>,
    /// Newest first, bounded by [`LOG_LINES`].
    pub log: VecDeque<LogLine>,
    /// How many lines the bound has discarded (O6).
    pub dropped: u64,
    /// The last refusal, when a send did not happen at all.
    pub error: Option<String>,
    /// The v1.12 confirmation: retiring a key that is not state-shaped is an
    /// operator cleanup, priced with the same `--i-know` vocabulary as the
    /// CLI. The engine's `check_retire` stays the judge — this only arms it.
    pub retire_i_know: bool,

    // ── call mode ───────────────────────────────────────────────────────
    pub producer: Option<String>,
    pub procedure: Option<String>,
    pub target: String,
    pub params: String,
    /// The selected procedure's request-schema fields. `None` = **not asked**
    /// (no schema fetched yet), which is not "the request has no fields".
    pub request_fields: Option<Vec<SchemaField>>,
    /// The last call outcome: a report, or the refusal/error text.
    pub outcome: Option<Result<CallReport, String>>,
}

impl SendForm {
    /// Append a line, dropping the oldest when the bound bites — and counting
    /// what it dropped, because a silently-trimmed log claims a completeness
    /// it does not have (O6).
    pub fn log(&mut self, ok: bool, text: impl Into<String>) {
        self.log.push_front(LogLine {
            text: text.into(),
            ok,
        });
        while self.log.len() > LOG_LINES {
            self.log.pop_back();
            self.dropped += 1;
        }
    }

    /// The repeat interval in seconds, floored at 100 ms so a typo cannot
    /// turn the pane into a load generator.
    pub fn interval_secs(&self) -> f64 {
        self.interval.trim().parse::<f64>().unwrap_or(1.0).max(0.1)
    }

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

impl Default for SendForm {
    fn default() -> Self {
        SendForm {
            mode: SendMode::Publish,
            body: String::new(),
            attachment: String::new(),
            in_flight: false,
            key: String::new(),
            qos: QosChoice(QosProfile::Sampled),
            qos_touched: false,
            encoding: String::new(),
            raw: false,
            repeat: false,
            interval: "1.0".into(),
            facts: None,
            source: None,
            note: None,
            encoding_used: None,
            armed: false,
            matching: None,
            log: VecDeque::new(),
            dropped: 0,
            error: None,
            retire_i_know: false,
            producer: None,
            procedure: None,
            target: String::new(),
            params: String::new(),
            request_fields: None,
            outcome: None,
        }
    }
}

/// Messages the pane emits (wrapped into the app's `Message::Pane(PaneMsg::Send)`).
///
/// One enum where there were two (#184): `PublishMsg` and `CallMsg` merged
/// under the rule that placed each of them — a message lives where its
/// failure is displayed, and every failure below is written into
/// [`SendForm`].
#[derive(Debug, Clone)]
pub enum SendMsg {
    /// The put/publish vs get/call toggle.
    ModeSelected(SendMode),

    // ── shared edits ────────────────────────────────────────────────────
    BodyChanged(String),
    AttachmentChanged(String),

    // ── publish mode ────────────────────────────────────────────────────
    KeyChanged(String),
    QosPicked(QosChoice),
    EncodingChanged(String),
    RawToggled(bool),
    RepeatToggled(bool),
    IntervalChanged(String),
    /// Prepare, declare, and send once (arming the repeat if it is on).
    Send,
    /// Undeclare the armed publication.
    Stop,
    /// Retire the key with a tombstone (RFC 04 §1.2, #115).
    Retire,
    /// The v1.12 operator confirmation for an off-state retire.
    RetireIKnowToggled(bool),
    /// A prepare→declare→send round finished: the prepared body's provenance,
    /// the declared publication (kept when repeating), and its matching status.
    Ready(Result<Arc<crate::message::PublishOutcome>, String>),
    /// One repeat tick fired.
    Tick,
    /// A repeat send landed (or did not).
    Sent(Result<usize, String>),
    /// The armed publication was undeclared.
    Stopped(Result<(), String>),
    /// A retire round finished (#115): the tombstone shipped (with the
    /// publication's matching fact), or it did not.
    Retired(Result<Option<bool>, String>),

    // ── call mode ───────────────────────────────────────────────────────
    ProducerPicked(String),
    ProcedurePicked(String),
    TargetChanged(String),
    ParamsChanged(String),
    /// Fill the body editor from the served request schema.
    ScaffoldBody,
    Submit,
    /// The selected procedure's request schema arrived (or did not).
    RequestSchema(Option<Vec<SchemaField>>),
    /// The call finished — its report, or why it did not (#176). Its `Err`
    /// is written into [`SendForm::outcome`].
    Done(Result<Arc<zenkey_fleet::report::CallReport>, String>),
}

/// Whether retiring this key is the v1.12 operator act — i.e. anything but a
/// state key, where retirement is the class's own semantics. Display logic
/// only: the engine's `check_retire` is the judge at submit.
pub fn retire_needs_i_know(facts: Option<&KeyFacts>) -> bool {
    use zenkey_fleet::facts::{ClassKind, KeyShape};
    match facts.map(|f| &f.shape) {
        Some(KeyShape::V1(v)) => v.class_kind != ClassKind::State,
        _ => true,
    }
}

fn msg(m: SendMsg) -> Message {
    Message::Pane(PaneMsg::Send(m))
}

/// Render the pane: the mode strip, then the mode's form. `slices` scaffolds
/// the call pickers and classifies the publish key; the `roster` is what
/// turns a call's silence into a sentence — RFC 05 §3.1 says "no reply" is
/// not one condition, and the only way to say *which* origins did not answer
/// is to join the answers against who is up.
pub fn pane<'a>(
    form: &'a SendForm,
    slices: Option<&'a SliceSet>,
    roster: &'a crate::nodes::NodeRoster,
    sp: Spacing,
) -> Element<'a, Message> {
    let mut modes = row![].spacing(sp.xs);
    for m in SendMode::ALL {
        modes = modes.push(kit::tab(
            m.label(),
            form.mode == m,
            msg(SendMsg::ModeSelected(m)),
        ));
    }
    let mut col = column![kit::section_header("Send", None), modes].spacing(sp.sm);

    col = match form.mode {
        SendMode::Publish => publish_body(col, form, slices.is_some(), sp),
        SendMode::Call => call_body(col, form, slices, roster, sp),
    };

    iced::widget::scrollable(col).height(Length::Fill).into()
}

// ── put / publish ───────────────────────────────────────────────────────

fn publish_body<'a>(
    mut col: Column<'a, Message>,
    form: &'a SendForm,
    slices_loaded: bool,
    sp: Spacing,
) -> Column<'a, Message> {
    let key = kit::input("key: full wire key to publish on", &form.key)
        .on_input(|t| msg(SendMsg::KeyChanged(t)))
        .size(font::CAPTION);

    // KeyFacts feedback as you type — the same classification ladder the tree
    // renders, so a key that will not refine says so *before* the send.
    let mut facts_col = Column::new().spacing(sp.xs);
    match (&form.facts, form.key.trim().is_empty()) {
        (_, true) => {
            facts_col = facts_col.push(kit::muted(
                "a full wire key, base included — this session is un-namespaced (RFC 09 §5)",
            ));
        }
        (Some(f), false) => {
            facts_col = facts_col.push(crate::view::detail::facts_section(f, sp));
        }
        (None, false) => {}
    }
    if !slices_loaded && !form.key.trim().is_empty() {
        facts_col = facts_col.push(kit::muted(
            "no registry loaded — the body cannot be schema-checked, and that is \
             \"not asked\", not \"unregistered\" (O4)",
        ));
    }

    let body = kit::input(
        "body: JSON (encoded for the wire by the engine)",
        &form.body,
    )
    .on_input(|t| msg(SendMsg::BodyChanged(t)))
    .size(font::CAPTION);

    let qos = kit::picker(qos_choices(), Some(form.qos), |q| {
        msg(SendMsg::QosPicked(q))
    })
    .placeholder("qos")
    .text_size(font::CAPTION);
    // #158: say when the picker is following the registry, so a declared
    // default never reads as the operator's choice.
    let qos_source: Option<Element<'a, Message>> = (!form.qos_touched
        && declared_qos(form.facts.as_ref()) == Some(form.qos.0))
    .then(|| kit::muted(format!("qos {} (declared)", form.qos.0.name())));

    let encoding = kit::input("encoding override (optional)", &form.encoding)
        .on_input(|t| msg(SendMsg::EncodingChanged(t)))
        .size(font::CAPTION);

    let attachment = kit::input(
        "attachment (optional — ships verbatim, never schema-encoded)",
        &form.attachment,
    )
    .on_input(|t| msg(SendMsg::AttachmentChanged(t)))
    .size(font::CAPTION);

    let raw = kit::check(form.raw)
        .label("send raw")
        .on_toggle(|b| msg(SendMsg::RawToggled(b)))
        .text_size(font::CAPTION);
    let repeat = kit::check(form.repeat)
        .label("repeat")
        .on_toggle(|b| msg(SendMsg::RepeatToggled(b)))
        .text_size(font::CAPTION);
    let interval = kit::input("interval (s)", &form.interval)
        .on_input(|t| msg(SendMsg::IntervalChanged(t)))
        .size(font::CAPTION)
        .width(Length::Fixed(90.0));

    let ready = !form.key.trim().is_empty() && !form.in_flight;
    let mut send = kit::action(kit::caption(if form.in_flight {
        "sending…"
    } else if form.armed {
        "re-send"
    } else {
        "send"
    }))
    .padding(sp.xs);
    if ready {
        send = send.on_press(msg(SendMsg::Send));
    }
    let mut controls = row![send].spacing(sp.sm);
    if form.armed {
        controls = controls.push(
            kit::action(kit::caption("stop"))
                .padding(sp.xs)
                .on_press(msg(SendMsg::Stop)),
        );
    }
    // Retire (#115): a tombstone, not an empty put. Off the state class it
    // is the v1.12 operator act and stays disabled until confirmed.
    let needs_i_know = retire_needs_i_know(form.facts.as_ref());
    let mut retire = kit::action(kit::caption("retire")).padding(sp.xs);
    if ready && (!needs_i_know || form.retire_i_know) {
        retire = retire.on_press(msg(SendMsg::Retire));
    }
    controls = controls.push(retire);
    let i_know_row: Option<Element<'a, Message>> = (needs_i_know
        && !form.key.trim().is_empty())
    .then(|| {
        kit::check(form.retire_i_know)
            .label("--i-know: not state-shaped — retiring is an operator cleanup (RFC 04 §1.2, v1.12)")
            .on_toggle(|b| msg(SendMsg::RetireIKnowToggled(b)))
            .text_size(font::CAPTION)
            .into()
    });

    col = col.push(key);
    col = col.push(facts_col);
    col = col.push(body);
    col = col.push(row![qos, encoding].spacing(sp.sm));
    if let Some(line) = qos_source {
        col = col.push(line);
    }
    col = col.push(attachment);
    col = col.push(
        row![raw, repeat, interval]
            .spacing(sp.sm)
            .align_y(iced::Alignment::Center),
    );
    col = col.push(controls);
    if let Some(row) = i_know_row {
        col = col.push(row);
    }

    if let Some(e) = &form.error {
        col = col.push(
            kit::body(format!("refused: {e}")).style(|theme: &iced::Theme| text::Style {
                color: Some(colors(theme).danger()),
            }),
        );
    }
    if let Some(line) = provenance(form) {
        col = col.push(line);
    }
    if let Some(note) = &form.note {
        col = col.push(kit::muted(note.clone()));
    }
    if form.armed {
        col = col.push(kit::muted(matching_sentence(form.matching)));
    }
    col.push(log_view(form, sp))
}

/// How the last body reached the wire. Encoded and as-typed must never look
/// alike — that is the whole reason `BodySource` leaves the engine.
fn provenance(form: &SendForm) -> Option<Element<'_, Message>> {
    let source = form.source.as_ref()?;
    let encoding = form.encoding_used.as_deref().unwrap_or("(no encoding set)");
    Some(match source {
        BodySource::Encoded { type_name } => {
            kit::muted(format!("encoded as {type_name} → {encoding}"))
        }
        BodySource::AsTyped => kit::muted(format!("sent as typed → {encoding}")),
        BodySource::Raw => kit::muted("sent raw — bytes verbatim, not encoded".to_string()),
    })
}

/// The #38 badge, worded exactly as the CLI words it: a routing fact about
/// *this* publisher. `false` is never a claim about the fleet, and `None` is
/// never rendered as `false`.
fn matching_sentence(matching: Option<bool>) -> String {
    match matching {
        Some(true) => "matching: a subscriber currently matches this publication".into(),
        Some(false) => "matching: no subscriber currently matches this publication — a \
                        routing fact about this publisher, not a fleet verdict (RFC 05 §3.1)"
            .into(),
        None => "matching: not asked".into(),
    }
}

/// The send log, on its own — the Activity dock's Publish stream (#183).
///
/// Split out of the pane so verifying a publish does not mean leaving the
/// form. The form asks a question; the log is what came back, and those are
/// two different regions of the workspace.
pub fn log_section(form: &SendForm, sp: Spacing) -> Element<'_, Message> {
    log_view(form, sp)
}

fn log_view(form: &SendForm, sp: Spacing) -> Element<'_, Message> {
    let mut col = Column::new().spacing(sp.xs);
    if form.log.is_empty() {
        return col.push(kit::muted("no sends yet")).into();
    }
    col = col.push(kit::muted(if form.dropped > 0 {
        format!(
            "send log — {} shown, {} dropped (bounded at {LOG_LINES})",
            form.log.len(),
            form.dropped
        )
    } else {
        format!("send log — {}", kit::plural(form.log.len(), "entry"))
    }));
    for line in &form.log {
        let entry = kit::body(line.text.as_str()).font(iced::Font::MONOSPACE);
        col = col.push(if line.ok {
            entry
        } else {
            entry.style(|theme: &iced::Theme| text::Style {
                color: Some(colors(theme).danger()),
            })
        });
    }
    col.into()
}

// ── get / call ──────────────────────────────────────────────────────────

fn call_body<'a>(
    mut col: Column<'a, Message>,
    form: &'a SendForm,
    slices: Option<&'a SliceSet>,
    roster: &'a crate::nodes::NodeRoster,
    sp: Spacing,
) -> Column<'a, Message> {
    let Some(slices) = slices else {
        return col.push(kit::empty_state(
            "No registry loaded",
            "Call mode scaffolds its form from registry slices — none are \
             loaded yet. \"Not asked\" is not \"nothing serves\" (RFC 09 §5.1 O4).",
        ));
    };

    // The fleet's `@rpc` vocabulary and one producer's surface both come from
    // the engine's projections (#234): `service_list` for who declares
    // procedures at all, `service_info` for the keys, shapes and fanout
    // verdicts — zenctl `service info`'s report (#211), consumed rather than
    // re-derived from the raw slices.
    let mut producers: Vec<String> = slices
        .service_list(None)
        .procedures
        .into_iter()
        .map(|p| p.producer)
        .collect();
    producers.dedup();
    if producers.is_empty() {
        return col.push(kit::empty_state(
            "No procedures declared",
            "The loaded slices declare no [[procedure]] entries.",
        ));
    }

    let producer_pick = kit::picker(producers, form.producer.clone(), |p| {
        msg(SendMsg::ProducerPicked(p))
    })
    .placeholder("producer")
    .text_size(font::CAPTION);

    let info = form
        .producer
        .as_deref()
        .map(|p| slices.service_info(p, None));
    let surface = info.as_ref().and_then(|r| r.as_ref().ok());
    let procedures: Vec<String> = surface
        .map(|i| i.procedures.iter().map(|p| p.path.clone()).collect())
        .unwrap_or_default();
    let procedure_pick = kit::picker(procedures, form.procedure.clone(), |p| {
        msg(SendMsg::ProcedurePicked(p))
    })
    .placeholder("procedure")
    .text_size(font::CAPTION);

    // The declared shape of the selected procedure, and the fanout verdict.
    let decl = surface
        .zip(form.procedure.as_deref())
        .and_then(|(i, want)| i.procedures.iter().find(|p| p.path == want));
    let fanout_forbidden = decl
        .map(|d| d.fanout.as_deref() == Some("forbidden"))
        .unwrap_or(false);

    let mut meta = Column::new().spacing(sp.xs);
    if let Some(Err(e)) = info.as_ref() {
        // Unreachable while the picker feeds from the same slices, but a
        // projection that answers with an error is rendered, not swallowed.
        meta = meta.push(
            kit::body(e.to_string()).style(|theme: &iced::Theme| text::Style {
                color: Some(colors(theme).danger()),
            }),
        );
    }
    if let Some(d) = decl {
        // The key a caller would use — with `{origin}` standing for the
        // publishing identity, and no producer chunk for a service origin
        // (RFC 06 §5). The surface the Call pane picked from but never
        // showed (#234).
        meta = meta.push(kit::mono(format!("→ {}", d.key)));
        meta = meta.push(kit::muted(format!(
            "kind {} · request {} · reply {}",
            d.kind,
            d.request.as_deref().unwrap_or("—"),
            d.reply.as_deref().unwrap_or("—"),
        )));
        meta = meta.push(kit::muted(format!(
            "fanout {} · idempotent {} · encoding {} · since {}",
            d.fanout.as_deref().unwrap_or("—"),
            d.idempotent
                .map(|b| b.to_string())
                .as_deref()
                .unwrap_or("—"),
            d.encoding.as_deref().unwrap_or("—"),
            d.since.as_deref().unwrap_or("—"),
        )));
        if let Some(desc) = &d.description {
            meta = meta.push(kit::muted(desc.clone()));
        }
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
                        kit::action(kit::caption("scaffold body"))
                            .padding([0.0, sp.xs])
                            .on_press(msg(SendMsg::ScaffoldBody)),
                    ]
                    .spacing(sp.sm)
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

    let target = kit::input("target: h-… | @service | *", &form.target)
        .on_input(|t| msg(SendMsg::TargetChanged(t)))
        .size(font::CAPTION);
    let params = kit::input("params: k=v;k=v (selector)", &form.params)
        .on_input(|t| msg(SendMsg::ParamsChanged(t)))
        .size(font::CAPTION);
    let body = kit::input("body: JSON (query payload)", &form.body)
        .on_input(|t| msg(SendMsg::BodyChanged(t)))
        .size(font::CAPTION);
    let attachment = kit::input(
        "attachment: verbatim, beside the body (empty = none)",
        &form.attachment,
    )
    .on_input(|t| msg(SendMsg::AttachmentChanged(t)))
    .size(font::CAPTION);

    let ready = decl.is_some()
        && !form.target.is_empty()
        && !(fanout_forbidden && form.target == "*")
        && !form.in_flight;
    let mut submit = kit::action(kit::caption(if form.in_flight {
        "calling…"
    } else {
        "call"
    }))
    .padding(sp.xs);
    if ready {
        submit = submit.on_press(msg(SendMsg::Submit));
    }

    col = col.push(
        row![producer_pick, procedure_pick]
            .spacing(sp.sm)
            .align_y(iced::Alignment::Center),
    );
    col = col.push(meta);
    col = col.push(target);
    col = col.push(params);
    col = col.push(body);
    col = col.push(attachment);
    col = col.push(submit);

    if let Some(outcome) = &form.outcome {
        col = col.push(outcome_view(outcome, form, roster, sp));
    }
    col
}

/// Origins the roster says are up and running the asked producer, that did
/// not answer. This is the join RFC 05 §3.1 asks for: "no reply" is not one
/// condition, and naming the silent origins is the difference between a
/// non-verdict a user can act on and one they cannot.
fn non_repliers(
    report: &CallReport,
    form: &SendForm,
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

/// The per-origin outcome list — a genuinely different rendering of a
/// genuinely different result, kept whole through the #184 merge.
fn outcome_view<'a>(
    outcome: &'a Result<CallReport, String>,
    form: &'a SendForm,
    roster: &'a crate::nodes::NodeRoster,
    sp: Spacing,
) -> Element<'a, Message> {
    match outcome {
        Err(e) => kit::body(format!("refused / failed: {e}"))
            .style(|theme: &iced::Theme| text::Style {
                color: Some(colors(theme).danger()),
            })
            .into(),
        Ok(report) => {
            let mut col = Column::new().spacing(sp.xs);
            col = col.push(kit::mono(format!("→ {}", report.key)));
            if report.answers.is_empty() {
                // Exit-code 2's meaning, rendered: silence is not a verdict —
                // and the wait it is read against is stated, not alluded to
                // (R5: the report carries it now).
                col = col.push(kit::muted(format!(
                    "no replies within {}s — a non-verdict, not proof of \
                     absence (RFC 05 §3.1); the roster says who should have answered",
                    report.timeout_s
                )));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_is_bounded_and_counts_what_it_dropped() {
        let mut form = SendForm::default();
        for i in 0..(LOG_LINES + 5) {
            form.log(true, format!("send {i}"));
        }
        assert_eq!(form.log.len(), LOG_LINES);
        assert_eq!(form.dropped, 5);
        // Newest first: the last send is the head.
        assert_eq!(form.log[0].text, format!("send {}", LOG_LINES + 4));
    }

    #[test]
    fn the_interval_floors_a_typo_instead_of_spinning() {
        let mut form = SendForm::default();
        assert_eq!(form.interval_secs(), 1.0);
        form.interval = "0".into();
        assert_eq!(form.interval_secs(), 0.1);
        form.interval = "not a number".into();
        assert_eq!(form.interval_secs(), 1.0);
        form.interval = "2.5".into();
        assert_eq!(form.interval_secs(), 2.5);
    }

    /// The retire gate mirrors the engine guard's happy path (#115): state
    /// keys retire freely — the class is written in the key — and everything
    /// else, including a key nobody classified, needs the confirmation.
    #[test]
    fn only_a_state_key_retires_without_the_i_know() {
        let facts = |k: &str| KeyFacts::project("", k);
        let state = facts("v1/h-3fa9c2d41b7e/state/sysinfo/health");
        assert!(!retire_needs_i_know(Some(&state)));
        let telemetry = facts("v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu/usage");
        assert!(retire_needs_i_know(Some(&telemetry)));
        let plane = facts("v1/h-3fa9c2d41b7e/@rpc/sysinfo/introspect");
        assert!(retire_needs_i_know(Some(&plane)));
        let foreign = facts("some/foreign/key");
        assert!(retire_needs_i_know(Some(&foreign)));
        // Not classified at all is not "state" (O4).
        assert!(retire_needs_i_know(None));
    }

    /// #158: the declared profile is read off the classification the pane
    /// already holds — and only the `Registered` rung ever yields one.
    #[test]
    fn declared_qos_reads_only_the_registered_rung() {
        use zenkey::slice::{RegistrySlice, SubjectDecl};
        let slice = RegistrySlice {
            version: "1.0".into(),
            app: "test".into(),
            convention: 1,
            name: "sysinfo".into(),
            service_origin: None,
            description: None,
            subjects: vec![SubjectDecl {
                path: "health".into(),
                class: "state".into(),
                type_name: "Health".into(),
                common: None,
                since: None,
                description: None,
                qos: Some("transition".into()),
                ttl_s: None,
                unit: None,
                rate: None,
                cardinality: None,
                encoding: None,
            }],
            procedures: vec![],
            blob: vec![],
            media: vec![],
            deprecated: vec![],
        };
        let slices = zenkey_fleet::SliceSet::from_slices(vec![slice]);
        let facts = |key: &str| zenkey_fleet::describe_key("", key, Some(&slices)).facts;
        let registered = facts("v1/h-3fa9c2d41b7e/state/sysinfo/health");
        assert_eq!(
            declared_qos(Some(&registered)),
            Some(QosProfile::Transition)
        );
        let unregistered = facts("v1/h-3fa9c2d41b7e/state/sysinfo/other");
        assert_eq!(declared_qos(Some(&unregistered)), None);
        // Not asked is not "declared nothing" — but there is no profile to
        // follow either way.
        assert_eq!(declared_qos(None), None);
    }

    /// The picker is the enum, not a copy of it.
    #[test]
    fn the_qos_picker_offers_exactly_the_closed_vocabulary() {
        let names: Vec<&str> = qos_choices().iter().map(|c| c.0.name()).collect();
        assert_eq!(
            names,
            ["sampled", "refreshed", "transition", "alert", "frame"]
        );
    }
}
