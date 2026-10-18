//! The Config tool (#768, CD): one service's `config.v1`, in the Workbench
//! (`spec/profiles/config/v1.md`, text 0.3).
//!
//! It shows what the owner serves and what the engine concluded from it:
//! the view's groups by class, each parameter with its value, source,
//! startup value where it differs, bounds and unit, and whether the group is
//! writable (§2.2–§2.4, §2.16); the floor (§2.8); the pending change with its
//! deadline as the owner's instant and a countdown read against this host's
//! clock, labelled so (§2.8, §2.9); the last change (§2.6), the change
//! events heard (§2.17) and the desired report (§2.18); and §5's answers in
//! their three states, the engine's words.
//!
//! It acts through the engine: a form per writable group, typed by the
//! served schema, refused in the form by the owner's own validator before
//! anything is sent, a reach change behind its consent with its window
//! shown before sending (raised to the floor), and a dry run; then
//! confirm — offered only once the view shows the change applied — cancel,
//! extend and persist. Each answer renders by its union (§3): applied or
//! pending, view or status, the envelope with its `ConfigError`, or
//! silence, which is a non-verdict and never a failure line.
//!
//! **A sensitive value is never on screen** (§2.15): its row says
//! `write-only`, its field is masked and never prefilled, and nothing the
//! owner returns carries it (the engine strips and scrubs it).

use iced::widget::{Column, column, row};
use iced::{Element, Length};
use zenkey_fleet::model::config as mc;
use zenkey_fleet::report::{
    ClockGround, ConfigActAnswer, ConfigActReport, ConfigAnswer, ConfigAnswerToken, ConfigDocument,
    ConfigReport,
};
use zenkey_model::config::{
    Class, ConfigChangeEvent, ConfigView, DesiredOutcome, Edit, GroupView, Kind, Outcome,
    Parameter, SetRequest, Value,
};
use zenkey_model::grammar::Addr;

use crate::configure::{
    self, Act, ActLanding, Answer, Answered, ConfigState, Draft, Masked, ReadLanding, ReadWhy,
};
use crate::message::{Message, PaneMsg};
use crate::view::kit;
use crate::view::send::{REPLAY_LOCK, attribution_sentence};
use crate::view::theme::Tone;
use crate::view::tokens::{Spacing, face, font};

/// The tool's interactions.
#[derive(Debug, Clone)]
pub enum ConfigMsg {
    /// The entry points' message — a node's card, the Inspector's address
    /// view: point the window at the address, show the tool, read it.
    Open(Addr),
    /// Read the service again.
    ReadAgain,
    /// The operator's word on the clocks (`freshness.v1` §2.6): read again
    /// under it.
    ClocksSynced(bool),
    /// The countdown's second — subscribed only while a change is pending
    /// on screen. Carries this host's clock: a view never reads one.
    Tick(std::time::SystemTime),
    /// A parameter's draft.
    Draft {
        group: String,
        param: String,
        draft: Draft,
    },
    /// Drop a group's drafts and guards.
    Discard(String),
    /// A group's window field, seconds.
    WindowChanged(String, String),
    /// A group's "join the pending change".
    JoinToggled(String, bool),
    /// A group's yes to a change that can cut the link.
    ConsentToggled(String, bool),
    /// The extend form's window, seconds.
    ExtendChanged(String),
    /// Send a group's change.
    Apply(String),
    /// Send a group's change as a dry run.
    DryRun(String),
    Confirm,
    Cancel,
    Extend,
    Persist,
    /// A reading landed.
    Read(ReadLanding),
    /// An act landed.
    Answered(ActLanding),
}

fn msg(m: ConfigMsg) -> Message {
    Message::Pane(PaneMsg::Config(m))
}

/// Everything the tool renders.
pub struct ConfigData<'a> {
    pub state: &'a ConfigState,
    /// The address the workspace points at, when it points at one.
    pub subject: Option<&'a Addr>,
    /// The replay lock (#74): while the panes show a capture, nothing here
    /// acts on the live bus.
    pub replaying: bool,
    /// Whether a session in the deployment's namespace is open to ask
    /// through.
    pub session: bool,
    pub sp: Spacing,
}

pub fn pane(d: ConfigData<'_>) -> Element<'_, Message> {
    let sp = d.sp;
    let mut col = column![kit::section_header("Configuration", None)].spacing(sp.md);
    if d.replaying {
        col = col.push(kit::callout(Tone::Mode, kit::caption(REPLAY_LOCK)));
    }
    let Some(addr) = d.state.target.as_ref() else {
        col = col.push(match d.subject {
            Some(a) => kit::empty_with(
                kit::EmptyKind::NotAsked,
                "Not read",
                format!(
                    "{a} is selected and its config.v1 was not asked for: reading it is an S4 GET \
                     of its view and its status (§2.3), made when you ask."
                ),
                kit::primary(kit::labelled(kit::Icon::Config, "read"))
                    .on_press(msg(ConfigMsg::Open(a.clone()))),
            ),
            None => kit::empty(
                kit::EmptyKind::Unselected,
                "No service picked",
                "Select an address whose descriptor lists config.v1: the nodes pane and the \
                 Inspector's address view offer \"configure\" on each one. The tool reads what \
                 the owner serves — its schema beside every value — and nothing else.",
            ),
        });
        return iced::widget::scrollable(col).height(Length::Fill).into();
    };
    col = col.push(header(addr, &d));
    if let Some(sent) = &d.state.in_flight {
        col = col.push(kit::muted(format!(
            "sending {} to {addr} — one act at a time, and its answer lands here",
            sent.act.label()
        )));
    }
    if let Some(a) = &d.state.answered {
        col = col.push(answer_card(a, sp));
    }
    col = match d.state.shown() {
        Some(r) => reading(col, &r.report, &d),
        None => col.push(match d.state.asking {
            Some(why) => kit::muted(asking_words(addr, why)),
            None if !d.session => kit::empty(
                kit::EmptyKind::NotAsked,
                "Not connected",
                "a configuration is read from its owner over the bus, on the session in the \
                 deployment's namespace — none is open yet",
            ),
            None => kit::empty(
                kit::EmptyKind::NotAsked,
                "Not read",
                "nothing was asked of this service yet — \"read again\" asks",
            ),
        }),
    };
    iced::widget::scrollable(col)
        .height(Length::Fill)
        .spacing(sp.xs)
        .into()
}

/// What a reading in flight is doing, in words.
fn asking_words(addr: &Addr, why: ReadWhy) -> String {
    match why {
        ReadWhy::Asked | ReadWhy::AfterAct => format!(
            "reading {addr}'s config.v1 — its descriptor, then its view and status by S4 GET \
             (core §4.2)"
        ),
        ReadWhy::UntilApplied => format!(
            "listening to {addr}'s view until the change's apply_at, then the GET: the apply's \
             put says it applied (§2.9)"
        ),
        ReadWhy::AtDeadline => format!(
            "listening to {addr} across the deadline, then the GET: the undo's put and the \
             change event say how it ended (§2.11, §2.17)"
        ),
    }
}

/// The service, the reading's facts and its controls.
fn header<'a>(addr: &'a Addr, d: &ConfigData<'a>) -> Element<'a, Message> {
    let sp = d.sp;
    let s = d.state;
    let clocks = kit::check(s.clocks_synced)
        .label("clocks synced — take my word that this host's clock agrees with the owner's")
        .size(font::CAPTION)
        .text_size(font::CAPTION)
        .on_toggle(|b| msg(ConfigMsg::ClocksSynced(b)));
    let mut again = kit::secondary(kit::caption("read again")).padding([0.0, sp.xs]);
    if s.asking.is_none() && d.session {
        again = again.on_press(msg(ConfigMsg::ReadAgain));
    }
    let mut col = column![
        row![
            kit::emphasis(addr.to_string()).font(face::MONO),
            iced::widget::space::horizontal(),
            clocks,
            again,
        ]
        .spacing(sp.sm)
        .align_y(iced::Alignment::Center)
    ]
    .spacing(sp.sm);
    let mut fields = Vec::new();
    if let Some(v) = s.view() {
        fields.push(kit::field("INSTANCE", kit::mono(v.instance.clone())));
        fields.push(kit::field("REVISION", kit::mono(v.revision.to_string())));
        fields.push(kit::field(
            "FLOOR",
            kit::mono(format!("{} s — the smallest window", v.confirm_floor_s)),
        ));
    } else if let Some(st) = s.status() {
        fields.push(kit::field("INSTANCE", kit::mono(st.instance.clone())));
        fields.push(kit::field("REVISION", kit::mono(st.revision.to_string())));
        fields.push(kit::field(
            "FLOOR",
            kit::mono(format!("{} s — the smallest window", st.confirm_floor_s)),
        ));
    }
    if let Some(r) = s.shown() {
        fields.push(kit::field(
            "READ",
            kit::mono(format!("{} (this host)", configure::instant(r.at))),
        ));
    }
    if !fields.is_empty() {
        col = col.push(kit::fields(fields));
    }
    if let (Some(why), Some(_)) = (s.asking, s.shown()) {
        col = col.push(kit::muted(asking_words(addr, why)));
    }
    col.into()
}

/// One of §5's answers in the profile's words: its three ways of neither
/// kept apart, never folded into a no (core O5).
fn token_words(a: ConfigAnswerToken) -> &'static str {
    match a {
        ConfigAnswerToken::Yes => "yes",
        ConfigAnswerToken::No => "no",
        ConfigAnswerToken::NotYet => "not yet",
        ConfigAnswerToken::Unobservable => "unobservable",
        ConfigAnswerToken::NotAsked => "not asked",
    }
}

fn answer_line(question: &str, a: &ConfigAnswer) -> String {
    format!("{question} {} — {}", token_words(a.answer), a.says)
}

/// One document's freshness, as `freshness.v1` judged it, and how to give
/// an unjudgeable age a ground.
fn freshness_line<T>(name: &str, doc: &ConfigDocument<T>, clock: ClockGround) -> String {
    let f = &doc.freshness;
    let via = match doc.via {
        Some(zenkey_fleet::report::ConfigVia::Get) => " (the owner's reply, S4)",
        Some(zenkey_fleet::report::ConfigVia::Subscription) => " (heard live)",
        None => "",
    };
    let hint = if f.reason == "clock_untrusted" && clock == ClockGround::None {
        "; tick \"clocks synced\" if they agree"
    } else {
        ""
    };
    format!("{name}: {}{via} — {}{hint}", f.verdict.as_str(), f.says)
}

/// The reading: its premise, the documents' freshness, the pending change,
/// the last one, the groups, §5, the desired report and the events.
fn reading<'a>(
    mut col: Column<'a, Message>,
    report: &'a ConfigReport,
    d: &ConfigData<'a>,
) -> Column<'a, Message> {
    let sp = d.sp;
    if let Some(premise) = zenkey_fleet::judge::config::premise(report.premise) {
        col = col.push(kit::callout(
            Tone::Neutral,
            kit::caption(answer_line("config.v1 here:", &premise)),
        ));
    }
    if let Some(why) = &report.unobservable {
        col = col.push(kit::callout(
            Tone::Neutral,
            kit::caption(format!("unobservable — {why}")),
        ));
    }
    let mut fresh = column![].spacing(sp.xs);
    if let Some(v) = report.view.as_option() {
        fresh = fresh.push(kit::muted(freshness_line("view", v, report.clock)));
        if let Some(u) = &v.undecodable {
            fresh = fresh.push(kit::status_chip(
                Tone::Caution,
                format!("the view's bytes are no ConfigView — {u}"),
            ));
        }
    }
    fresh = fresh.push(kit::muted(freshness_line(
        "status",
        &report.status,
        report.clock,
    )));
    col = col.push(fresh);

    let Some(view) = d.state.view() else {
        // The schema is the view's to serve (§2.2): without one nothing can
        // be typed, so nothing is offered.
        return col.push(kit::empty(
            kit::EmptyKind::Silent,
            "No view",
            "no ConfigView was read, so there is no schema to type a change by and no value \
             to show — the status alone carries values and no classes (§2.19)",
        ));
    };
    if let Some(card) = pending_card(view, d) {
        col = col.push(card);
    }
    if let Some(l) = &view.last_change {
        col = col.push(last_change_row(l, d));
    }
    // The groups first — what an operator came to read and change — then
    // what the reading concludes, the desired report and the events.
    for class in configure::CLASS_ORDER {
        for g in view.groups.iter().filter(|g| g.class == class) {
            col = col.push(group_card(g, view, report, d));
        }
    }
    col = col.push(conclusions(report, sp));
    col = col.push(desired_line(view, sp));
    col.push(events_section(d.state, sp))
}

/// "Is a change pending?" made visible: the change, its deadline as the
/// owner's instant, the countdown read against this host's clock, whether
/// it applied, and what can be done to it.
fn pending_card<'a>(view: &'a ConfigView, d: &ConfigData<'a>) -> Option<Element<'a, Message>> {
    let sp = d.sp;
    let s = d.state;
    let p = view.pending.as_ref()?;
    let mut inner = column![
        row![kit::status(
            Tone::Mode,
            format!("pending change {} on {}", p.token, p.groups.join(", "))
        ),]
        .spacing(sp.sm)
        .align_y(iced::Alignment::Center),
        kit::caption(format!(
            "deadline {} — the owner's instant (§2.8): undone then unless confirmed",
            p.deadline
        )),
    ]
    .spacing(sp.xs);
    inner = inner.push(kit::muted(match s.deadline_left_ms() {
        Some(ms) if ms > 0 => format!(
            "{}s left before the owner's deadline — its instant, read against this host's \
             clock",
            (ms + 999) / 1000
        ),
        Some(_) => "the owner's deadline has passed by this host's clock — the read-back says \
                    whether it was undone"
            .to_owned(),
        None => {
            "the deadline is no RFC 3339 instant, or no clock reading yet: no countdown".to_owned()
        }
    }));
    if let Some(ms) = s.margin_left_ms() {
        inner = inner.push(kit::muted(if ms > 0 {
            format!(
                "this window's own margin: {}s — the window asked, raised to the floor, less \
                 the reply's transit, counted from its arrival (§2.8)",
                (ms + 999) / 1000
            )
        } else {
            "this window's own margin is spent (§2.8)".to_owned()
        }));
    }
    if let Some(at) = s
        .armed
        .as_ref()
        .filter(|a| a.token == p.token)
        .and_then(|a| a.apply_at.as_ref())
    {
        inner = inner.push(kit::muted(format!(
            "applies at {at} — the owner's instant, from its reply (§2.9)"
        )));
    }
    inner = inner.push(if p.applied {
        kit::status_chip(Tone::Info, "applied — confirm it over the new link")
    } else {
        kit::status_chip(
            Tone::Neutral,
            "not applied yet — a group waits for its apply_at; confirm is offered once the \
             view shows it applied (§2.9)",
        )
    });
    // No session is the handler's to refuse, in words, as Send's call is:
    // a control that greys out without saying why says nothing.
    let free = s.in_flight.is_none() && !d.replaying;
    let mut controls = row![].spacing(sp.sm).align_y(iced::Alignment::Center);
    // Confirm is offered only when every group applied: before then the
    // owner answers it busy (§2.9, §2.10 row 5).
    if p.applied {
        let mut confirm = kit::primary(kit::caption("confirm"));
        if free {
            confirm = confirm.on_press(msg(ConfigMsg::Confirm));
        }
        controls = controls.push(confirm);
    }
    let mut extend = kit::secondary(kit::caption("extend"));
    if free && !s.extend_by.trim().is_empty() {
        extend = extend.on_press(msg(ConfigMsg::Extend));
    }
    let mut cancel = kit::danger(kit::caption("cancel — undo now"));
    if free {
        cancel = cancel.on_press(msg(ConfigMsg::Cancel));
    }
    controls = controls
        .push(
            kit::input(&view.confirm_floor_s.to_string(), &s.extend_by)
                .on_input(|t| msg(ConfigMsg::ExtendChanged(t)))
                .font(face::MONO)
                .size(font::CAPTION)
                .width(Length::Fixed(72.0)),
        )
        .push(kit::muted("s"))
        .push(extend)
        .push(cancel);
    inner = inner.push(controls);
    inner = inner.push(kit::muted(
        "extend moves the deadline to its arrival plus the window, raised to the floor; \
         joining never moves it (§2.7, §2.10)",
    ));
    Some(kit::callout(Tone::Mode, inner))
}

/// The last change: what `persist` takes (§2.6, §2.13).
fn last_change_row<'a>(
    l: &'a zenkey_model::config::LastChange,
    d: &ConfigData<'a>,
) -> Element<'a, Message> {
    let sp = d.sp;
    let mut persist = kit::secondary(kit::caption("persist"));
    // The pending change's token is busy to persist (§2.13 row 4); the last
    // change's is what persist takes, whatever is pending beside it.
    if d.state.in_flight.is_none() && !d.replaying {
        persist = persist.on_press(msg(ConfigMsg::Persist));
    }
    row![
        kit::eyebrow("LAST CHANGE"),
        kit::data_chip(l.token.clone()),
        kit::muted(format!(
            "on {} — persist writes it to the owner's persisted layer (§2.13)",
            l.groups.join(", ")
        )),
        persist,
    ]
    .spacing(sp.sm)
    .align_y(iced::Alignment::Center)
    .wrap()
    .into()
}

/// §5's answers about changes and the desired document, in the engine's
/// words. Writability and survival are drawn on their group and parameter.
fn conclusions<'a>(report: &'a ConfigReport, sp: Spacing) -> Element<'a, Message> {
    let mut col = column![kit::eyebrow("WHAT THIS READING CONCLUDES (§5)")].spacing(sp.xs);
    col = col.push(kit::muted(answer_line(
        "is a change pending?",
        &report.pending,
    )));
    if let Some(c) = &report.change {
        col = col.push(kit::muted(answer_line(
            &format!("did change {} land?", c.token),
            &c.landed,
        )));
        col = col.push(kit::muted(answer_line(
            "was its undo clean?",
            &c.undo_clean,
        )));
    }
    col = col.push(kit::muted(answer_line(
        "has it converged to its desired document?",
        &report.converged.answer,
    )));
    col.into()
}

/// The desired report, as the owner states it (§2.18).
fn desired_line<'a>(view: &'a ConfigView, sp: Spacing) -> Element<'a, Message> {
    let line = match &view.desired {
        None => "desired: no report — the owner converges no desired document, or a new \
                 instance has acted on none yet (§2.18)"
            .to_owned(),
        Some(r) => {
            let outcome = match r.outcome {
                DesiredOutcome::Converged => "converged".to_owned(),
                DesiredOutcome::Refused | DesiredOutcome::Failed => {
                    let word = if r.outcome == DesiredOutcome::Refused {
                        "refused"
                    } else {
                        "failed"
                    };
                    match &r.refusal {
                        Some(f) => format!(
                            "{word}: {} on {}{}",
                            f.code.as_str(),
                            f.group,
                            f.parameter
                                .as_ref()
                                .map(|p| format!(".{p}"))
                                .unwrap_or_default()
                        ),
                        None => word.to_owned(),
                    }
                }
            };
            format!(
                "desired: revision {} acted on — {outcome}",
                r.observed_revision
            )
        }
    };
    row![kit::muted(line)].spacing(sp.sm).into()
}

fn class_tone(class: Class) -> Tone {
    match class {
        Class::Hot => Tone::Info,
        Class::Reach => Tone::Caution,
        Class::Contract => Tone::Neutral,
    }
}

fn class_words(class: Class) -> &'static str {
    match class {
        Class::Hot => "hot — takes effect at once",
        Class::Reach => "reach — can cut the link",
        Class::Contract => "contract — a restart's",
    }
}

/// "Is this group writable?" as the engine answered it, short.
fn writable_words(report: &ConfigReport, group: &str) -> (Tone, String) {
    let Some(a) = report.writable.iter().find(|w| w.group == group) else {
        return (Tone::Neutral, "writable: not asked".to_owned());
    };
    match (a.answer.answer, a.answer.reason.as_str()) {
        (ConfigAnswerToken::Yes, _) => (Tone::Positive, "writable".to_owned()),
        (ConfigAnswerToken::No, "outside_allowlist") => (
            Tone::Neutral,
            "not writable — outside the startup allowlist (§2.16)".to_owned(),
        ),
        (ConfigAnswerToken::No, _) => (
            Tone::Neutral,
            "not writable — set in the startup configuration, then a restart (§2.4)".to_owned(),
        ),
        (t, _) => (Tone::Neutral, format!("writable: {}", token_words(t))),
    }
}

/// One group: its class, whether it is writable, its parameters — and,
/// when the view says a `set` may reach it, its form.
fn group_card<'a>(
    g: &'a GroupView,
    view: &'a ConfigView,
    report: &'a ConfigReport,
    d: &ConfigData<'a>,
) -> Element<'a, Message> {
    let sp = d.sp;
    let (wtone, wwords) = writable_words(report, &g.name);
    let mut col = column![
        row![
            kit::emphasis(g.name.clone()).font(face::MONO),
            kit::status_chip(class_tone(g.class), class_words(g.class)),
            kit::status_chip(wtone, wwords),
            kit::muted(g.description.clone()),
        ]
        .spacing(sp.sm)
        .align_y(iced::Alignment::Center)
        .wrap(),
    ]
    .spacing(sp.sm);
    // A form only where the view says a set can reach the group: its
    // allowlist lets it, and its class is not the restart's (§2.16, §2.4).
    // The owner's validator still runs on the press.
    let editable = g.writable && g.class != Class::Contract;
    col = col.push(header_row(editable, sp));
    let form = d.state.forms.get(&g.name);
    for p in &g.parameters {
        let draft = form.and_then(|f| f.drafts.get(&p.name));
        col = col.push(param_row(g, p, editable.then_some(draft), d));
    }
    if editable {
        col = col.push(footer(g, view, d));
    }
    kit::card(col)
}

fn header_row<'a>(editable: bool, sp: Spacing) -> Element<'a, Message> {
    let cell = |s: &'static str, n: u16| {
        Element::from(iced::widget::container(kit::eyebrow(s)).width(Length::FillPortion(n)))
    };
    let mut r = row![cell("PARAMETER", 3), cell("VALUE", 3), cell("SOURCE", 2)].spacing(sp.sm);
    if editable {
        r = r.push(cell("NEW VALUE", 3));
    }
    r.push(cell("DESCRIPTION", 3)).into()
}

/// A value, as the view carries it.
fn plain(v: &Value) -> String {
    match v {
        Value::Text(t) => t.clone(),
        other => other.to_string(),
    }
}

/// One parameter row. A sensitive value is write-only by rule; any other
/// absent value is unknown — the owner could not read it back (§2.3).
fn param_row<'a>(
    g: &'a GroupView,
    p: &'a Parameter,
    editor: Option<Option<&'a Draft>>,
    d: &ConfigData<'a>,
) -> Element<'a, Message> {
    let sp = d.sp;
    let value: Element<'a, Message> = if p.sensitive {
        kit::status_chip(Tone::Neutral, "write-only")
    } else {
        match &p.value {
            None => kit::status_chip(Tone::Neutral, "unknown"),
            Some(v) => {
                let mut c = column![kit::mono(v.to_string())].spacing(sp.xs);
                if let Some(s) = &p.startup {
                    c = c.push(kit::muted(format!(
                        "startup {s} — what a restart runs (§2.3)"
                    )));
                }
                c.into()
            }
        }
    };
    let source: Element<'a, Message> = match p.source {
        Some(s) => kit::data_chip(s.as_str()),
        None => kit::muted("—"),
    };
    let cell = |e: Element<'a, Message>, n: u16| {
        Element::from(iced::widget::container(e).width(Length::FillPortion(n)))
    };
    let mut r = row![
        cell(
            column![
                kit::caption(p.name.clone()).font(face::MONO),
                kit::muted(configure::kind_words(&p.kind)),
            ]
            .into(),
            3
        ),
        cell(value, 3),
        cell(source, 2),
    ]
    .spacing(sp.sm)
    .align_y(iced::Alignment::Center);
    if let Some(draft) = editor {
        r = r.push(cell(edit_control(g, p, draft, d), 3));
    }
    r.push(cell(kit::muted(p.description.clone()), 3)).into()
}

/// The control a parameter is edited with, by its served kind: false/true
/// for a bool, which can show neither lit; a box for an integer or a text;
/// a masked box for a sensitive value, never prefilled (§2.15).
fn edit_control<'a>(
    g: &'a GroupView,
    p: &'a Parameter,
    draft: Option<&'a Draft>,
    d: &ConfigData<'a>,
) -> Element<'a, Message> {
    let enabled = d.state.in_flight.is_none() && !d.replaying;
    let group = g.name.clone();
    let param = p.name.clone();
    match &p.kind {
        Kind::Bool if !p.sensitive => {
            let lit = match draft {
                Some(Draft::Text(t)) => Some(t == "true"),
                _ => match p.value {
                    Some(Value::Bool(b)) => Some(b),
                    _ => None,
                },
            };
            if !enabled {
                return kit::muted(lit.map_or("—".to_owned(), |b| b.to_string()));
            }
            let segment = |v: bool| kit::Segment {
                value: v,
                label: v.to_string(),
                icon: None,
                count: None,
                tip: None,
            };
            kit::segmented(vec![segment(false), segment(true)], lit, move |v: bool| {
                msg(ConfigMsg::Draft {
                    group: group.clone(),
                    param: param.clone(),
                    draft: Draft::Text(v.to_string()),
                })
            })
        }
        _ if p.sensitive => {
            let typed = match draft {
                Some(Draft::Secret(m)) => m.expose(),
                _ => "",
            };
            let mut input = kit::input("write-only — type to replace", typed)
                .secure(true)
                .font(face::MONO)
                .size(font::CAPTION);
            if enabled {
                input = input.on_input(move |t| {
                    msg(ConfigMsg::Draft {
                        group: group.clone(),
                        param: param.clone(),
                        draft: Draft::Secret(Masked::new(t)),
                    })
                });
            }
            input.into()
        }
        _ => {
            let current = p.value.as_ref().map(plain).unwrap_or_default();
            let typed = match draft {
                Some(Draft::Text(t)) => t.as_str(),
                _ => current.as_str(),
            };
            let mut input = kit::input(&current, typed)
                .font(face::MONO)
                .size(font::CAPTION);
            if enabled {
                input = input.on_input(move |t| {
                    msg(ConfigMsg::Draft {
                        group: group.clone(),
                        param: param.clone(),
                        draft: Draft::Text(t),
                    })
                });
            }
            input.into()
        }
    }
}

/// A writable group's form footer: the window it asks — raised to the
/// floor, said before sending — the join, the consent a reach change needs,
/// and the buttons.
fn footer<'a>(g: &'a GroupView, view: &'a ConfigView, d: &ConfigData<'a>) -> Element<'a, Message> {
    let sp = d.sp;
    let s = d.state;
    let empty = configure::GroupForm::default();
    let form = s.forms.get(&g.name).unwrap_or(&empty);
    let floor = view.confirm_floor_s;
    let mut col = Column::new().spacing(sp.sm);
    let joining = form.join && view.pending.is_some();
    let help = match g.class {
        Class::Reach => format!(
            "required unless joining: the owner applies it at its apply_at and undoes it at the \
             deadline unless you confirm over the new link; a shorter window is raised to the \
             floor, {floor} s (§2.4, §2.8)"
        ),
        _ => format!(
            "empty: applied at once and permanent; with a window it is pending until confirmed, \
             and a shorter one is raised to the floor, {floor} s (§2.8)"
        ),
    };
    let group = g.name.clone();
    if !joining {
        let mut window = kit::input(&floor.to_string(), &form.window)
            .font(face::MONO)
            .size(font::CAPTION)
            .width(Length::Fixed(120.0));
        if s.in_flight.is_none() && !d.replaying {
            window = window.on_input(move |t| msg(ConfigMsg::WindowChanged(group.clone(), t)));
        }
        col = col.push(kit::form_field("WINDOW (S)", window, Some(help)));
        if let Ok(Some(c)) = configure::seconds(&form.window, "the window") {
            let w = zenkey_model::config::window(c, floor);
            col = col.push(kit::muted(if w > c {
                format!(
                    "asks {c} s, raised to the floor: the window is {w} s from the owner's \
                     receipt (§2.8)"
                )
            } else {
                format!("the window is {w} s from the owner's receipt (§2.8)")
            }));
        }
    }
    if let Some(p) = &view.pending {
        let group = g.name.clone();
        let mut join = kit::check(form.join)
            .label(format!(
                "join pending change {} — its window covers this change too; joining never \
                 moves the deadline (§2.7)",
                p.token
            ))
            .size(font::CAPTION)
            .text_size(font::CAPTION);
        if s.in_flight.is_none() && !d.replaying {
            join = join.on_toggle(move |b| msg(ConfigMsg::JoinToggled(group.clone(), b)));
        }
        col = col.push(join);
    }
    // The consent's words are the engine's (§2.4): what a reach change can
    // cut, and why the class asks.
    if g.class == Class::Reach {
        let probe = SetRequest {
            confirm_s: Some(floor),
            ..SetRequest::default()
        };
        let addr = s
            .target
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        if let Some(claim) = mc::consent(Some(view), &addr, &g.name, &probe, "") {
            col = col.push(kit::callout(Tone::Caution, kit::caption(claim)));
        }
        let group = g.name.clone();
        let mut yes = kit::check(form.consent)
            .label(
                "send it, and confirm over the new link — it is undone at the deadline otherwise",
            )
            .size(font::CAPTION)
            .text_size(font::CAPTION);
        if s.in_flight.is_none() && !d.replaying {
            yes = yes.on_toggle(move |b| msg(ConfigMsg::ConsentToggled(group.clone(), b)));
        }
        col = col.push(yes);
    }
    let free = s.in_flight.is_none() && !d.replaying;
    let edited = !form.drafts.is_empty();
    let mut apply = kit::primary(kit::caption(if joining {
        "apply — joins the pending change"
    } else {
        "apply"
    }));
    let mut dry = kit::secondary(kit::caption("dry run"));
    let mut discard = kit::ghost(kit::caption("discard"));
    // Nothing edited is nothing to send: the owner would accept an empty
    // `values` as a change of nothing (§2.5), an act and an event of its
    // own — not what an untouched form means.
    if free && edited {
        apply = apply.on_press(msg(ConfigMsg::Apply(g.name.clone())));
        dry = dry.on_press(msg(ConfigMsg::DryRun(g.name.clone())));
    }
    if edited && s.in_flight.is_none() {
        discard = discard.on_press(msg(ConfigMsg::Discard(g.name.clone())));
    }
    let mut buttons = row![apply, dry, discard]
        .spacing(sp.sm)
        .align_y(iced::Alignment::Center);
    if form.holds_key() {
        buttons = buttons.push(kit::muted(
            "apply again resends the same request under the same idempotency key: the owner \
             answers it once (§2.14)",
        ));
    }
    col.push(buttons).into()
}

/// One edit of a change, as its event carries it (§2.17): a sensitive
/// parameter's is redacted, with neither value.
fn edit_words(e: &Edit) -> String {
    if e.redacted {
        return format!("{}.{} (write-only)", e.group, e.parameter);
    }
    let v = |v: &Option<Value>| v.as_ref().map_or("unknown".to_owned(), ToString::to_string);
    format!("{}.{} {} → {}", e.group, e.parameter, v(&e.old), v(&e.new))
}

fn outcome_tone(o: Outcome) -> Tone {
    match o {
        Outcome::Applied | Outcome::Confirmed => Tone::Positive,
        Outcome::RolledBack => Tone::Info,
        Outcome::Partial => Tone::Negative,
    }
}

/// The change events heard for this service, newest first: one per change,
/// as it ended (§2.17).
fn events_section<'a>(s: &'a ConfigState, sp: Spacing) -> Element<'a, Message> {
    let mut col = column![kit::eyebrow("CHANGE EVENTS HEARD (§2.17)")].spacing(sp.xs);
    if s.events.is_empty() {
        return col
            .push(kit::muted(
                "none heard — an event is heard while a reading listens (across a deadline, \
                 after a reach reply) or replayed from a union storage; none is no verdict",
            ))
            .into();
    }
    for e in s.events.iter().rev() {
        col = col.push(event_row(e, sp));
    }
    if s.events_dropped > 0 {
        col = col.push(kit::muted(format!(
            "{} older not kept (display bound)",
            kit::plural(s.events_dropped as usize, "event")
        )));
    }
    col.into()
}

fn event_row<'a>(e: &'a ConfigChangeEvent, sp: Spacing) -> Element<'a, Message> {
    let edits: Vec<String> = e.edits.iter().map(edit_words).collect();
    let mut r = row![
        kit::status_chip(outcome_tone(e.outcome), e.outcome.as_str()),
        kit::data_chip(e.token.clone()),
        kit::muted(format!("revision {}", e.revision)),
        kit::muted(if edits.is_empty() {
            "no edits".to_owned()
        } else {
            edits.join(" · ")
        }),
    ]
    .spacing(sp.sm)
    .align_y(iced::Alignment::Center);
    if let Some(actor) = &e.actor {
        r = r.push(kit::muted(format!("actor {actor} (claimed)")));
    }
    r.wrap().into()
}

/// The last act's answer, by its union (§3) — or its refusal in the form,
/// before anything was sent.
fn answer_card<'a>(a: &'a Answered, sp: Spacing) -> Element<'a, Message> {
    let head = kit::eyebrow("LAST ACT");
    let label = kit::mono(a.act.label());
    let body: Element<'a, Message> = match &a.answer {
        // Refused before any task existed: provably nothing on the wire.
        Answer::RefusedHere(why) => column![
            kit::status_chip(Tone::Negative, "refused here — nothing was sent"),
            kit::caption(why.clone()),
        ]
        .spacing(sp.xs)
        .into(),
        Answer::Failed(e) => kit::error(format!("the call could not be made: {e}")),
        Answer::Landed(r) => landed(r, &a.act, sp),
    };
    kit::card(
        column![
            row![head, label]
                .spacing(sp.sm)
                .align_y(iced::Alignment::Center),
            body
        ]
        .spacing(sp.xs),
    )
}

/// A one-line summary of a view an act answered.
fn view_summary(v: &ConfigView) -> String {
    let pending = match &v.pending {
        Some(p) => format!(
            "change {} pending, its deadline {}{}",
            p.token,
            p.deadline,
            if p.applied { ", applied" } else { "" }
        ),
        None => "nothing pending".to_owned(),
    };
    let last = v
        .last_change
        .as_ref()
        .map(|l| format!(" · last change {}", l.token))
        .unwrap_or_default();
    format!("revision {} · {pending}{last}", v.revision)
}

fn landed<'a>(r: &'a ConfigActReport, act: &Act, sp: Spacing) -> Element<'a, Message> {
    let mut col = column![kit::mono(format!("→ {}", r.key))].spacing(sp.xs);
    let dry = matches!(act, Act::Set { dry_run: true, .. });
    col = col.push(match &r.answer {
        ConfigActAnswer::Applied { view } if dry => {
            let mut c = column![kit::status_chip(
                Tone::Info,
                "dry run — the view as it would read; nothing was touched, minted or published \
                 (§2.9)"
            )]
            .spacing(sp.xs);
            if let Act::Set { group, .. } = act
                && let Some(g) = view.groups.iter().find(|g| &g.name == group)
            {
                for p in &g.parameters {
                    c = c.push(kit::muted(format!(
                        "{}.{}: {}",
                        g.name,
                        p.name,
                        if p.sensitive {
                            "write-only".to_owned()
                        } else {
                            p.value
                                .as_ref()
                                .map_or("unknown".to_owned(), ToString::to_string)
                        }
                    )));
                }
            }
            c.into()
        }
        ConfigActAnswer::Applied { view } => column![
            kit::status_chip(
                Tone::Positive,
                "applied — the read-back after the apply (§2.9)"
            ),
            kit::muted(view_summary(view)),
        ]
        .spacing(sp.xs)
        .into(),
        ConfigActAnswer::Pending { pending } => column![
            kit::status_chip(
                Tone::Info,
                format!(
                    "pending — change {}, answered before its apply (§2.9)",
                    pending.token
                ),
            ),
            kit::muted(format!(
                "applies at {}, deadline {} — the owner's instants: confirm over the new link \
                 once the view shows it applied",
                pending.apply_at, pending.deadline
            )),
        ]
        .spacing(sp.xs)
        .into(),
        ConfigActAnswer::View { view } => column![
            kit::status_chip(
                Tone::Positive,
                "answered — the owner's view after the act (§2.10)"
            ),
            kit::muted(view_summary(view)),
        ]
        .spacing(sp.xs)
        .into(),
        ConfigActAnswer::Status { status } => column![
            kit::status_chip(
                Tone::Positive,
                "answered — the owner's status after the act (§2.19)"
            ),
            kit::muted(format!(
                "revision {} · {}",
                status.revision,
                status
                    .pending
                    .as_ref()
                    .map_or("nothing pending".to_owned(), |p| format!(
                        "change {} pending",
                        p.token
                    ))
            )),
        ]
        .spacing(sp.xs)
        .into(),
        ConfigActAnswer::Refused { refusal } => {
            let mut c = column![
                row![
                    kit::status_chip(Tone::Negative, format!("refused: {}", refusal.code)),
                    kit::caption(refusal.message.clone()),
                ]
                .spacing(sp.sm)
                .align_y(iced::Alignment::Center)
                .wrap()
            ]
            .spacing(sp.xs);
            if let Some(cause) = &refusal.cause {
                c = c.push(kit::muted(format!("cause: {cause}")));
            }
            if let Some(e) = &refusal.error {
                c = c.push(kit::muted(format!(
                    "ConfigError {}{}",
                    e.code.as_str(),
                    e.detail
                        .as_ref()
                        .map(|d| format!(" — {d}"))
                        .unwrap_or_default()
                )));
            }
            if let Some(u) = &refusal.detail_undecodable {
                c = c.push(kit::muted(format!("detail: {u}")));
            }
            c.into()
        }
        ConfigActAnswer::Malformed { malformed } => column![
            kit::status_chip(Tone::Caution, "malformed envelope — a broken owner"),
            kit::muted(format!("{}: {}", malformed.encoding, malformed.error)),
        ]
        .spacing(sp.xs)
        .into(),
        ConfigActAnswer::Undecodable { reason, size } => column![
            kit::status_chip(Tone::Caution, "not the operation's reply type"),
            kit::muted(format!("{} — {reason}", kit::plural(*size, "byte"))),
        ]
        .spacing(sp.xs)
        .into(),
        // Silence is not an error (O5): no failure chip, no danger edge.
        ConfigActAnswer::Silent { silence } => kit::empty(
            kit::EmptyKind::Silent,
            "No answer",
            format!(
                "no value and no envelope within {}s, after {} — a non-verdict, never an \
                 error (O5): a set that drew nothing started nothing a tool can name (§5). \
                 Presence: {}.{}",
                r.timeout_s,
                kit::plural(silence.attempts as usize, "attempt"),
                attribution_sentence(silence.presence),
                silence
                    .transport
                    .as_ref()
                    .map(|t| format!(" Transport: {t}."))
                    .unwrap_or_default()
            ),
        ),
    });
    if let Some(actor) = &r.actor {
        col = col.push(kit::muted(format!(
            "claimed actor {actor}{} — call metadata, never authentication (core O7)",
            r.request_id
                .as_ref()
                .map(|id| format!(", request {id}"))
                .unwrap_or_default()
        )));
    }
    col.into()
}
