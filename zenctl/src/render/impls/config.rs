//! `config.v1` (#768, CC): `config get`'s reading and the acts' replies.
//!
//! Three rules, each with a place in the rendering:
//!
//! * **§5's answers keep their kinds apart.** `yes`, `no`, `not yet`,
//!   `unobservable` and `not asked` are five words, each answer with its
//!   reason; a stale document answers nothing, and says why.
//! * **The owner's instants are the owner's.** A deadline and an `apply_at`
//!   are shown as written, named as the owner's clock (§2.8).
//! * **A sensitive value is never drawn** (§2.15): a write-only parameter
//!   reads `<sensitive: write-only>`, whatever a document carried.

use zenkey_fleet::report::{
    Asked, ConfigActAnswer, ConfigActReport, ConfigAnswer, ConfigAnswerToken, ConfigCheckedRead,
    ConfigDocument, ConfigFreshness, ConfigPremise, ConfigRefusal, ConfigReport, ConfigTyping,
    ConfigVia, FreshnessToken,
};
use zenkey_model::config::{ConfigStatus, ConfigView, Kind, Parameter, Value};

use super::acts::attribution_words;
use crate::render::style;
use crate::render::{Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_without};

fn namespace_words(ns: &str) -> String {
    if ns.is_empty() {
        "the bus root".to_owned()
    } else {
        format!("namespace {ns:?}")
    }
}

/// An answer's word.
fn answer_word(a: ConfigAnswerToken) -> &'static str {
    match a {
        ConfigAnswerToken::Yes => "yes",
        ConfigAnswerToken::No => "no",
        ConfigAnswerToken::NotYet => "not yet",
        ConfigAnswerToken::Unobservable => "unobservable",
        ConfigAnswerToken::NotAsked => "not asked",
    }
}

fn answer_text(a: &ConfigAnswer) -> String {
    format!("{} — {}", answer_word(a.answer), a.says)
}

/// A document's freshness, and how it came.
fn document_words<T>(name: &str, d: &ConfigDocument<T>) -> String {
    let f: &ConfigFreshness = &d.freshness;
    let how = match (d.via, &d.stamp) {
        (Some(ConfigVia::Get), Some(s)) => {
            format!(", its owner's reply stamped {} by {}", s.time, s.clock)
        }
        (Some(ConfigVia::Get), None) => ", its owner's reply, unstamped".to_owned(),
        (Some(ConfigVia::Subscription), Some(s)) => {
            format!(", the last put heard, stamped {} by {}", s.time, s.clock)
        }
        (Some(ConfigVia::Subscription), None) => ", the last put heard".to_owned(),
        (None, _) => String::new(),
    };
    let what = match (&d.undecodable, f.verdict) {
        (Some(why), _) => format!("undecodable ({why})"),
        (None, FreshnessToken::Fresh) => "fresh".to_owned(),
        (None, v) => format!("{} — {}", v.as_str().replace('_', " "), f.says),
    };
    format!("{name}: {what}{how}")
}

/// A value as written, with its unit.
fn value_text(p: &Parameter, v: &Value) -> String {
    match &p.kind {
        Kind::Integer { unit: Some(u), .. } => format!("{v} {u}"),
        _ => v.to_string(),
    }
}

/// One parameter's line: its value, source and startup value, or why there
/// is none to draw.
fn parameter_cells(p: &Parameter) -> [Cell; 3] {
    let value = if p.sensitive {
        Cell::styled("<sensitive: write-only>", style::UNPROVEN)
    } else {
        match &p.value {
            Some(v) => Cell::text(value_text(p, v)),
            None => Cell::styled("<unknown>", style::UNPROVEN),
        }
    };
    let mut rest = p
        .source
        .map(|s| s.as_str().to_owned())
        .unwrap_or_else(|| "-".to_owned());
    if let (Some(startup), false) = (&p.startup, p.sensitive) {
        rest.push_str(&format!(
            "  startup {} — a restart would run it",
            value_text(p, startup)
        ));
    }
    [Cell::text(format!("  {}", p.name)), value, Cell::text(rest)]
}

/// The view as a grid: a heading per group, its class and `writable`, a
/// row per parameter.
fn view_grid(t: &mut Table, v: &ConfigView) {
    let mut g = Grid::unheaded(3);
    for group in &v.groups {
        g.group(format!(
            "{} ({}, {}) — {}",
            group.name,
            group.class.as_str(),
            if group.writable {
                "writable"
            } else {
                "not writable: outside the startup allowlist"
            },
            group.description
        ));
        for p in &group.parameters {
            g.row(parameter_cells(p));
        }
    }
    t.grid(g);
}

/// The status as a grid: a row per value, by group (§2.19).
fn status_grid(t: &mut Table, s: &ConfigStatus) {
    let mut g = Grid::unheaded(2);
    for (group, values) in &s.values {
        for (name, v) in values {
            g.row([
                Cell::text(format!("{group}.{name}")),
                Cell::text(v.to_string()),
            ]);
        }
    }
    t.grid(g);
}

/// The head facts every document carries: instance, revision, floor, the
/// pending change, the last change, the desired report.
fn facts_lines(
    t: &mut Table,
    instance: &str,
    revision: u64,
    floor: u64,
    pending: Option<&zenkey_model::config::Pending>,
    last: Option<&zenkey_model::config::LastChange>,
    desired: Option<&zenkey_model::config::DesiredReport>,
) {
    t.line(format!(
        "  instance {instance}, revision {revision}, floor {floor} s"
    ));
    match pending {
        Some(p) => t.line_styled(
            format!(
                "  pending change: {} on {}, deadline {} (the owner's clock), {}",
                p.token,
                p.groups.join(", "),
                p.deadline,
                if p.applied {
                    "applied"
                } else {
                    "not every group applied yet"
                }
            ),
            style::WARNING,
        ),
        None => t.line("  pending change: none"),
    };
    if let Some(l) = last {
        t.line(format!(
            "  last change: {} on {}",
            l.token,
            l.groups.join(", ")
        ));
    }
    if let Some(d) = desired {
        let refusal = d
            .refusal
            .as_ref()
            .map(|r| {
                format!(
                    ": {} on {}{}",
                    r.code.as_str(),
                    r.group,
                    r.parameter
                        .as_deref()
                        .map(|p| format!(".{p}"))
                        .unwrap_or_default()
                )
            })
            .unwrap_or_default();
        let outcome = match d.outcome {
            zenkey_model::config::DesiredOutcome::Converged => "converged",
            zenkey_model::config::DesiredOutcome::Refused => "refused",
            zenkey_model::config::DesiredOutcome::Failed => "failed",
        };
        t.line(format!(
            "  desired document: {outcome} at revision {}{refusal}",
            d.observed_revision
        ));
    }
}

/// `config get` (#768, CC): the view drawn as its schema beside every
/// running value, or the status's values, each document's freshness said;
/// then §5's answers, each in its own word.
impl Render for ConfigReport {
    const FAMILY: &'static str = "config";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_without(self, &["writable", "survives_restart", "events"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for g in &self.writable {
            out(Row::of("group", g));
        }
        for p in &self.survives_restart {
            out(Row::of("parameter", p));
        }
        for e in &self.events {
            out(Row::of("event", e));
        }
    }

    fn table(&self, t: &mut Table) {
        t.line(format!(
            "config.v1 of {} in {}",
            self.service,
            namespace_words(&self.namespace)
        ));
        let view = match &self.view {
            Asked::Asked(d) => {
                t.line(format!("  {}", document_words("view", d)));
                d.value.as_ref()
            }
            Asked::NotAsked => None,
        };
        t.line(format!("  {}", document_words("status", &self.status)));
        if let Some(v) = view {
            facts_lines(
                t,
                &v.instance,
                v.revision,
                v.confirm_floor_s,
                v.pending.as_ref(),
                v.last_change.as_ref(),
                v.desired.as_ref(),
            );
            view_grid(t, v);
        } else if let Some(s) = &self.status.value {
            facts_lines(
                t,
                &s.instance,
                s.revision,
                s.confirm_floor_s,
                s.pending.as_ref(),
                s.last_change.as_ref(),
                s.desired.as_ref(),
            );
            status_grid(t, s);
        }
        t.blank();
        t.line("what a tool may conclude (config.v1 §5):");
        t.line(format!(
            "  is a change pending: {}",
            answer_text(&self.pending)
        ));
        if let Some(c) = &self.change {
            t.line(format!(
                "  did {} land: {}",
                c.token,
                answer_text(&c.landed)
            ));
            t.line(format!(
                "  was its undo clean: {}",
                answer_text(&c.undo_clean)
            ));
        }
        for g in &self.writable {
            t.line(format!(
                "  is {} writable: {}",
                g.group,
                answer_text(&g.answer)
            ));
        }
        for p in &self.survives_restart {
            t.line(format!(
                "  will {}.{} survive a restart: {}",
                p.group,
                p.parameter,
                answer_text(&p.answer)
            ));
        }
        t.line(format!(
            "  has it converged to its desired document{}: {}",
            self.converged
                .revision
                .map(|r| format!(" at revision {r}"))
                .unwrap_or_default(),
            answer_text(&self.converged.answer)
        ));
        if !self.events.is_empty() {
            t.blank();
            t.line("change events read:");
            for e in &self.events {
                let edits: Vec<String> = e
                    .edits
                    .iter()
                    .map(|x| {
                        if x.redacted {
                            format!("{}.{} <redacted>", x.group, x.parameter)
                        } else {
                            format!(
                                "{}.{} {} → {}",
                                x.group,
                                x.parameter,
                                x.old.as_ref().map_or("?".to_owned(), ToString::to_string),
                                x.new.as_ref().map_or("?".to_owned(), ToString::to_string)
                            )
                        }
                    })
                    .collect();
                t.line(format!(
                    "  {} {} at revision {}{}: {}",
                    e.token,
                    e.outcome.as_str(),
                    e.revision,
                    e.actor
                        .as_deref()
                        .map(|a| format!(", by {a} (claimed)"))
                        .unwrap_or_default(),
                    if edits.is_empty() {
                        "no edits".to_owned()
                    } else {
                        edits.join(", ")
                    }
                ));
            }
        }
    }

    fn notes(&self) -> Vec<Note> {
        use zenkey_fleet::report::ClockGround;
        let mut notes = Vec::new();
        match (&self.presence, self.premise) {
            (_, ConfigPremise::AcrossFace) => notes.push(
                Note::coverage(
                    "across a constrained face: presence and descriptors do not cross, and the \
                     deployment's word stands for the descriptor",
                )
                .cite("config.v1 §2.19"),
            ),
            (Some(p), premise) => {
                notes.push(
                    Note::coverage(format!(
                        "presence read through `{}`: {} instance(s) of {}; {}",
                        p.selector,
                        p.instances.len(),
                        self.service,
                        match premise {
                            ConfigPremise::Listed => "its descriptor lists config.v1",
                            ConfigPremise::NotListed =>
                                "no descriptor of it lists config.v1, so nothing is asked of it",
                            ConfigPremise::DescriptorUnread =>
                                "no descriptor of it read, so whether it implements config.v1 \
                                 cannot be told",
                            ConfigPremise::Absent =>
                                "absent from a complete read, so nothing is asked of it",
                            ConfigPremise::Incomplete if p.cut_off_during =>
                                "the read was cut off while it ran, without it: possibly \
                                 incomplete",
                            ConfigPremise::Incomplete =>
                                "the read ran to its timeout without it: possibly incomplete",
                            ConfigPremise::AcrossFace => "",
                        }
                    ))
                    .cite("config.v1 §2.1"),
                );
            }
            (None, _) => {}
        }
        notes.push(
            Note::caveat(match self.clock {
                ClockGround::DeploymentWord => {
                    "a reply's stamp is aged against this host's clock, on the deployment's \
                     word that it and the owner's agree (--clocks-synced)"
                }
                ClockGround::Measured => {
                    "a reply's stamp is aged only against a clock this reading measured on a \
                     live put of the same clock"
                }
                ClockGround::None => {
                    "no clock was trusted — no window, no --clocks-synced — so a reply's age is \
                     unobservable, and a document read only by GET answers nothing of §5"
                }
            })
            .cite("freshness.v1 §2.6"),
        );
        if self.projection.answer == ConfigAnswerToken::No {
            notes.push(Note::caveat(self.projection.says.clone()).cite("config.v1 §2.3"));
        }
        if let Some(why) = &self.unobservable {
            notes.push(Note::silence(why.clone()));
        }
        notes
    }

    /// What the reading put to the bus, over its window.
    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: self.asked.clone(),
            window_s: self.window_s,
        })
    }
}

fn refusal_lines(t: &mut Table, r: &ConfigRefusal) {
    let mut head = format!("refused — {}: {}", r.code, r.message);
    if let Some(c) = &r.cause {
        head.push_str(&format!(" (cause: {c})"));
    }
    t.line_styled(head, style::ERROR);
    if let Some(e) = &r.error {
        t.line(format!(
            "  ConfigError {}{}",
            e.code.as_str(),
            e.detail
                .as_deref()
                .map(|d| format!(": {d}"))
                .unwrap_or_default()
        ));
    }
    if let Some(why) = &r.detail_undecodable {
        t.line(format!("  detail: {why}"));
    }
}

/// `config set|confirm|cancel|extend|persist` (#768, CC): the reply union
/// as the text names it, the request as sent, and what was checked first.
impl Render for ConfigActReport {
    const FAMILY: &'static str = "config-act";

    /// The whole report: one act, nothing to decompose into rows.
    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        crate::render::envelope_of(self)
    }

    fn rows(&self, _out: &mut dyn FnMut(Row)) {}

    fn table(&self, t: &mut Table) {
        t.line(format!(
            "{} on {} in {}  ({}s{})",
            self.operation,
            self.service,
            namespace_words(&self.namespace),
            self.timeout_s,
            if self.across_face {
                ", across a constrained face"
            } else {
                ""
            }
        ));
        t.line(format!(
            "  request {}",
            serde_json::to_string(&self.request).unwrap_or_default()
        ));
        if let Some(c) = &self.checked {
            let read = match c.read {
                ConfigCheckedRead::View => "checked first against its view",
                ConfigCheckedRead::Status => "typed against its status (no schema, no class)",
                ConfigCheckedRead::Nothing => "sent unchecked (--no-precheck)",
            };
            let typed = match c.typed_by {
                ConfigTyping::Schema => "values typed by the schema",
                ConfigTyping::Status => "values typed by the status's",
                ConfigTyping::Spelling => "values typed by their spelling",
            };
            let at = match (&c.instance, c.revision) {
                (Some(i), Some(r)) => format!(" (instance {i}, revision {r})"),
                _ => String::new(),
            };
            let class = c
                .class
                .map(|k| format!("; the group is {}", k.as_str()))
                .unwrap_or_default();
            t.line(format!("  {read}{at}{class}; {typed}"));
        }
        match &self.answer {
            ConfigActAnswer::Applied { view } => {
                t.line_styled(
                    format!(
                        "applied — the read-back after the apply, revision {}",
                        view.revision
                    ),
                    style::PASS,
                );
                facts_lines(
                    t,
                    &view.instance,
                    view.revision,
                    view.confirm_floor_s,
                    view.pending.as_ref(),
                    view.last_change.as_ref(),
                    view.desired.as_ref(),
                );
                view_grid(t, view);
            }
            ConfigActAnswer::Pending { pending } => {
                t.line_styled(
                    format!(
                        "pending — change {} applies at {}, and is undone at {} unless \
                         confirmed (the owner's clock)",
                        pending.token, pending.apply_at, pending.deadline
                    ),
                    style::WARNING,
                );
            }
            ConfigActAnswer::View { view } => {
                t.line_styled(
                    format!(
                        "answered — its view after the act, revision {}",
                        view.revision
                    ),
                    style::PASS,
                );
                facts_lines(
                    t,
                    &view.instance,
                    view.revision,
                    view.confirm_floor_s,
                    view.pending.as_ref(),
                    view.last_change.as_ref(),
                    view.desired.as_ref(),
                );
                view_grid(t, view);
            }
            ConfigActAnswer::Status { status } => {
                t.line_styled(
                    format!(
                        "answered — its compact status after the act, revision {}",
                        status.revision
                    ),
                    style::PASS,
                );
                facts_lines(
                    t,
                    &status.instance,
                    status.revision,
                    status.confirm_floor_s,
                    status.pending.as_ref(),
                    status.last_change.as_ref(),
                    status.desired.as_ref(),
                );
                status_grid(t, status);
            }
            ConfigActAnswer::Refused { refusal } => refusal_lines(t, refusal),
            ConfigActAnswer::Malformed { malformed } => {
                t.line_styled(
                    format!(
                        "malformed envelope ({}): {}",
                        malformed.encoding, malformed.error
                    ),
                    style::ERROR,
                );
            }
            ConfigActAnswer::Undecodable { reason, size } => {
                t.line_styled(
                    format!("a value reply of {size} B that is not the reply type: {reason}"),
                    style::ERROR,
                );
            }
            ConfigActAnswer::Silent { silence } => {
                t.line_styled(
                    format!(
                        "no answer within {}s — silence, never a failure: {}",
                        self.timeout_s,
                        attribution_words(silence.presence)
                    ),
                    style::UNPROVEN,
                );
            }
        }
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if let (ConfigActAnswer::Silent { .. }, true) =
            (&self.answer, self.operation.starts_with("set/"))
        {
            notes.push(
                Note::silence(
                    "a set that drew nothing started nothing a tool can name: resent without an \
                     idempotency key, it may make a second change",
                )
                .cite("config.v1 §5"),
            );
        }
        if let ConfigActAnswer::Pending { pending } = &self.answer {
            notes.push(Note::next_step(format!(
                "read its view over the new link, then: zenctl config confirm {} {}",
                self.service, pending.token
            )));
        }
        notes
    }

    /// The one key the act went out on, waiting for its reply.
    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: vec![self.key.clone()],
            window_s: Some(self.timeout_s),
        })
    }
}
