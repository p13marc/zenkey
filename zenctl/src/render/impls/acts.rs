//! zk2's acts and reads through a contract (#612, FJ5): `call`, `get
//! state` and `watch`.
//!
//! Three rules, each with a place in the rendering:
//!
//! * **O5's cases stay apart.** A value, an envelope, a malformed envelope
//!   and silence are four different first words, and silence is never
//!   drawn as an empty value. Its attribution through presence is worded as
//!   what this reader could see: a refused presence read is empty too
//!   (spec §8.1, 0.8).
//! * **Current and last-known never look alike** (S6). The first line of a
//!   state read says which question was asked, and a last-known read says
//!   which archive answered and whether alignment confirmed each key.
//! * **A payload is rendered as far as its contract reaches**, and the rung
//!   it stopped at is said (the tooling guide's O2): a decoded value with
//!   its declared type, a raw type as its media type and size, bytes that
//!   do not decode with the reason, the structural ladder with why.

use zenkey_fleet::report::{
    CallMode, EnvelopeView, OperationAnswer, OperationReport, PayloadRendering,
    PresenceAttribution, Rendered, RepliesView, Stamp, StateReading, StateReport, StateValue,
    Unresolved, WatchEvent, WatchSample, WatchSummary,
};

use super::zk2::short_fp;
use crate::render::style;
use crate::render::{Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_without};

// ── payloads ───────────────────────────────────────────────────────────────

/// Why a payload fell to the structural ladder, in a few words: the
/// engine's one spelling of each rung.
pub fn unresolved_words(why: &Unresolved) -> String {
    why.words()
}

/// One rendering, on one line: the declared type and the value, or what
/// stopped the decode.
pub fn rendered_text(r: &Rendered, size: usize) -> String {
    match r {
        Rendered::Value { declared, value } => format!(
            "{declared} {}",
            serde_json::to_string(value).unwrap_or_default()
        ),
        Rendered::Opaque { media_type } => format!("<{media_type}, {size} B>"),
        Rendered::Undecodable { declared, reason } => {
            format!("<{size} B that do not decode as {declared}: {reason}>")
        }
        Rendered::Structural { why, text, .. } => {
            format!("{text}  (structural: {})", unresolved_words(why))
        }
    }
}

/// A payload rendering, on one line.
pub fn payload_text(p: &PayloadRendering) -> String {
    rendered_text(&p.rendered, p.size)
}

/// A stamp and its clock (the tooling guide's O7).
pub fn stamp_text(s: &Stamp) -> String {
    format!("{} (clock {})", s.time, s.clock)
}

// ── call ───────────────────────────────────────────────────────────────────

fn envelope_text(e: &EnvelopeView) -> String {
    let mut s = format!("{} — {}", e.code, e.message);
    if let Some(c) = &e.cause {
        s.push_str(&format!(" (cause: {c})"));
    }
    s
}

fn envelope_lines(t: &mut Table, e: &EnvelopeView, lead: &str) {
    t.line_styled(format!("{lead}{}", envelope_text(e)), style::ERROR);
    if let Some(d) = &e.detail {
        t.line(format!("  detail: {}", rendered_text(d, 0)));
    }
}

/// The presence attribution, as what this reader could see.
fn attribution_words(p: PresenceAttribution) -> &'static str {
    match p {
        PresenceAttribution::Present => {
            "the service holds the interface's token: it is up and did not answer \
             (an access-control refusal, a frozen server, or a lost reply)"
        }
        PresenceAttribution::InstanceOnly => {
            "the service holds an instance token and not this interface's: a standby, \
             or the interface is in its tokenless set"
        }
        PresenceAttribution::NoTokenVisible => {
            "no token of the service is visible to this reader: it may be gone, or this \
             reader may not see its presence (a refused read is empty too)"
        }
        PresenceAttribution::Unknown => {
            "the presence read may be incomplete and saw no interface token: \
             unknown"
        }
        PresenceAttribution::Unobservable => "presence cannot be observed from here",
    }
}

fn replies_table(t: &mut Table, r: &RepliesView) {
    if !r.repliers.is_empty() {
        let mut g = Grid::new(["REPLIER", "KEY", "REPLY"]);
        for p in &r.repliers {
            let first = p.replies.first().map(payload_text);
            g.row([
                Cell::text(&p.address),
                Cell::text(&p.key),
                match first {
                    Some(text) => Cell::text(text),
                    None => Cell::text("(no value)"),
                },
            ]);
            let mut more: Vec<String> = p
                .replies
                .iter()
                .skip(1)
                .map(|v| format!("  {}", payload_text(v)))
                .collect();
            more.extend(
                p.summaries
                    .iter()
                    .map(|s| format!("  summary: {}", payload_text(s))),
            );
            if p.possibly_partial == Some(true) {
                more.push(format!(
                    "  possibly partial: {} summaries, not exactly one",
                    p.summaries.len()
                ));
            }
            g.detail(more);
        }
        t.grid(g);
    }
    for e in &r.refusals {
        envelope_lines(t, e, "refused (unattributed): ");
    }
    for m in &r.malformed {
        t.line_styled(
            format!("malformed envelope ({}): {}", m.encoding, m.error),
            style::ERROR,
        );
    }
    if !r.presence.unheard.is_empty() {
        t.line(format!(
            "no value from {}: each holds the interface's token, and refused or was \
             silent — a caller cannot tell which",
            r.presence.unheard.join(", ")
        ));
    }
}

impl Render for OperationReport {
    const FAMILY: &'static str = "operation";

    /// The whole report, less the repliers, which are the rows.
    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = crate::render::envelope_of(self);
        if let Some(serde_json::Value::Object(r)) = e.get_mut("replies") {
            r.remove("repliers");
        }
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        if let OperationAnswer::Replies { replies } = &self.answer {
            for p in &replies.repliers {
                out(Row::of("replier", p));
            }
        }
    }

    fn table(&self, t: &mut Table) {
        t.line(format!(
            "call {} {}@{} {}  ({}, {}s)",
            self.address,
            self.iface,
            short_fp(&self.fingerprint),
            self.operation,
            match self.mode {
                CallMode::Concrete => "one address",
                CallMode::Fanout => "fan-out",
            },
            self.timeout_s
        ));
        match &self.answer {
            OperationAnswer::Value { reply } => {
                t.line(format!("key    {}", reply.key));
                t.line_styled(format!("value  {}", payload_text(reply)), style::PASS);
            }
            OperationAnswer::Refused { envelope } => envelope_lines(t, envelope, "refused  "),
            OperationAnswer::Malformed { malformed } => {
                t.line_styled(
                    format!(
                        "malformed envelope ({}): {}",
                        malformed.encoding, malformed.error
                    ),
                    style::ERROR,
                );
            }
            OperationAnswer::Silent { silence } => {
                t.line_styled(
                    format!(
                        "no answer  after {} attempt(s){}",
                        silence.attempts,
                        silence
                            .transport
                            .as_ref()
                            .map(|e| format!(" — {e}"))
                            .unwrap_or_default()
                    ),
                    style::UNPROVEN,
                );
                t.line(format!(
                    "presence   {}",
                    attribution_words(silence.presence)
                ));
            }
            OperationAnswer::Replies { replies } => replies_table(t, replies),
        }
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        match &self.answer {
            OperationAnswer::Silent { .. } => notes.push(
                Note::silence(
                    "no value and no envelope: silence, never \"no such operation\" — exit 2",
                )
                .cite("spec §5.1 O5"),
            ),
            OperationAnswer::Replies { replies } => {
                let silent = replies.repliers.is_empty()
                    && replies.refusals.is_empty()
                    && replies.malformed.is_empty();
                if silent {
                    notes.push(
                        Note::silence("no reply from any service selected: exit 2")
                            .cite("spec §5.1 O5"),
                    );
                }
                if !replies.refusals.is_empty() {
                    notes.push(
                        Note::caveat(format!(
                            "{} envelope(s), unattributed: a reply_err carries no key",
                            replies.refusals.len()
                        ))
                        .cite("spec §5.1"),
                    );
                }
                let partial = replies
                    .repliers
                    .iter()
                    .filter(|p| p.possibly_partial == Some(true))
                    .count();
                if partial > 0 {
                    notes.push(
                        Note::caveat(format!(
                            "{partial} replier(s) possibly partial: no summary (cut off or \
                             broken off), or several (several instances on one key)"
                        ))
                        .cite("spec §5.1 O6"),
                    );
                }
                if !replies.summary_declared && !replies.repliers.is_empty() {
                    notes.push(Note::caveat(
                        "the operation declares no summary: whether a replier finished \
                         cannot be told",
                    ));
                }
                if !replies.presence.complete {
                    notes.push(
                        Note::coverage(match &replies.presence.error {
                            Some(e) => format!(
                                "the selection's presence could not be read ({e}): who sent \
                                 no value is not known"
                            ),
                            None => "the selection's presence read may be incomplete: a \
                                     holder that sent no value may be missing from the \
                                     list"
                                .into(),
                        })
                        .cite("spec §8.1"),
                    );
                }
                if replies.discarded > 0 {
                    notes.push(Note::caveat(format!(
                        "{} value reply(ies) discarded: on a key that is not concrete (R6), \
                         or not a member of this operation",
                        replies.discarded
                    )));
                }
                if !replies.transport.is_empty() {
                    notes.push(Note::caveat(format!(
                        "transport: {}",
                        replies.transport.join("; ")
                    )));
                }
            }
            _ => {}
        }
        notes
    }

    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: self.selectors.clone(),
            window_s: Some(self.timeout_s),
        })
    }
}

// ── get state ──────────────────────────────────────────────────────────────

impl Render for StateReport {
    const FAMILY: &'static str = "state";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_without(self, &["rows"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for r in &self.rows {
            out(Row::of("key", r));
        }
    }

    fn table(&self, t: &mut Table) {
        let head = format!(
            "{}@{} {} at {}",
            self.iface,
            short_fp(&self.fingerprint),
            self.resource,
            self.address
        );
        match (self.reading, &self.archive) {
            (StateReading::Current, _) => {
                t.line(format!("CURRENT state, the owner's answer: {head}"));
            }
            (StateReading::LastKnown, archive) => {
                t.line_styled(
                    format!(
                        "LAST-KNOWN state from archive {}, never current: {head}",
                        archive.as_deref().unwrap_or("?")
                    ),
                    style::WARNING,
                );
            }
        }
        if self.rows.is_empty() {
            return;
        }
        let mut g = Grid::new(["KEY", "STATE", "STAMP"]);
        for r in &self.rows {
            let state = match &r.value {
                StateValue::Value { payload } => Cell::text(payload_text(payload)),
                StateValue::Deleted => Cell::styled("deleted", style::DEPRECATED),
            };
            g.row([
                Cell::text(&r.key),
                state,
                Cell::asked(r.timestamp.as_ref().map(stamp_text)),
            ]);
            let mut more = Vec::new();
            if let Some(c) = r.confirmed {
                more.push(if c {
                    "  confirmed by the archive's alignment".to_owned()
                } else {
                    "  NOT confirmed: alignment has not confirmed this key".to_owned()
                });
            }
            g.detail(more);
        }
        t.grid(g);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if self.rows.is_empty() {
            notes.push(
                Note::silence(match self.reading {
                    StateReading::Current => {
                        "the owner gave no reply within the timeout: silence, not \"no \
                         value\" — exit 2. An archive's last-known state is \
                         `--last-known <ARCHIVE>`"
                    }
                    StateReading::LastKnown => {
                        "the archive gave no reply within the timeout: silence — exit 2"
                    }
                })
                .cite("spec §4.2 S6"),
            );
        }
        if self.reading == StateReading::LastKnown {
            notes.push(
                Note::caveat(
                    "last-known, never current: what the archive recorded, which the \
                     owner may since have changed",
                )
                .cite("spec §4.2 S6"),
            );
        }
        notes
    }

    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: self.selectors.clone(),
            window_s: Some(self.timeout_s),
        })
    }
}

// ── watch ──────────────────────────────────────────────────────────────────

/// A watched sample, for a person: its key and provider, then its
/// rendering.
pub fn sample_lines(s: &WatchSample) -> Vec<String> {
    let mut lines = vec![format!(
        "{}  [{}]{}",
        s.key,
        s.provider,
        s.timestamp
            .as_ref()
            .map(|t| format!("  {}", stamp_text(t)))
            .unwrap_or_default()
    )];
    match &s.event {
        WatchEvent::Put {
            payload,
            conformance,
            attachment,
        } => {
            lines.push(format!("  {}", payload_text(payload)));
            if let Some(a) = attachment {
                lines.push(format!("  attachment: {}", payload_text(a)));
            }
            if let zenkey_fleet::report::Conformance::Invalid { violations } = conformance {
                for v in violations {
                    lines.push(format!("  invalid against its type: {v}"));
                }
            }
        }
        WatchEvent::Delete => lines.push("  <delete — not an empty value>".to_owned()),
    }
    if let Some(m) = &s.qos_mismatch {
        lines.push(format!(
            "  QoS not as declared (spec §2.4): {}",
            m.summary()
        ));
    }
    lines
}

/// The closing summary, for a person.
pub fn summary_lines(s: &WatchSummary) -> Vec<String> {
    let mut lines = vec![format!(
        "watched {} {}@{} {} for {:.1}s ({}): {} sample(s)",
        s.address,
        s.iface,
        short_fp(&s.fingerprint),
        s.resource,
        s.elapsed_s,
        match s.ended {
            zenkey_fleet::report::WatchEnd::Count => "--count reached",
            zenkey_fleet::report::WatchEnd::Window => "--for elapsed",
            zenkey_fleet::report::WatchEnd::Interrupted => "interrupted",
        },
        s.received
    )];
    if s.discarded > 0 {
        lines.push(format!(
            "{} sample(s) put on a wildcard key, discarded by rule (R6) — not losses",
            s.discarded
        ));
    }
    if s.unresolved > 0 {
        lines.push(format!(
            "{} sample(s) on a key that resolved to no member of the resource — the bus \
             and the contract disagree (#671)",
            s.unresolved
        ));
    }
    if s.qos_mismatched > 0 {
        lines.push(format!(
            "{} sample(s) did not ride the resource's declared QoS (spec §2.4)",
            s.qos_mismatched
        ));
    }
    if s.nonconforming > 0 {
        lines.push(format!(
            "{} payload(s) failed their declared type (spec §7.2, §7.3)",
            s.nonconforming
        ));
    }
    if s.lagged > 0 {
        lines.push(format!(
            "{} sample(s) dropped while this tool was behind: the count above is a \
             lower bound (O6)",
            s.lagged
        ));
    }
    if s.received == 0 {
        lines.push("nothing arrived: silence, never a verdict (spec §5.1 O5) — exit 2".to_owned());
    }
    lines
}

// ── check schema ───────────────────────────────────────────────────────────

/// `check schema` (#612, FJ8b): one payload against one type of a revision.
/// The verdict's word leads; the violations or the decode failure follow,
/// one per line; the decoded value rides the document, not the table.
impl Render for zenkey_fleet::report::PayloadCheck {
    const FAMILY: &'static str = "schema-check";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        crate::render::envelope_of(self)
    }

    fn rows(&self, _out: &mut dyn FnMut(Row)) {}

    fn table(&self, t: &mut Table) {
        use zenkey_fleet::report::Conformance;
        t.line(format!(
            "{} {}@{} {} {}: {} ({} B{})",
            self.declared,
            self.iface,
            short_fp(&self.fingerprint),
            self.resource,
            self.member,
            match &self.conformance {
                Conformance::Valid => "valid",
                Conformance::Invalid { .. } => "invalid",
                Conformance::Undecodable { .. } => "undecodable",
                Conformance::NotChecked { .. } => "not checked",
            },
            self.size,
            self.encoding
                .as_deref()
                .map(|e| format!(", read as {e}"))
                .unwrap_or_default(),
        ));
        let mut g = Grid::unheaded(1);
        match &self.conformance {
            Conformance::Invalid { violations } => {
                for v in violations {
                    g.row([Cell::text(format!("  {v}"))]);
                }
            }
            Conformance::Undecodable { reason, .. } | Conformance::NotChecked { reason } => {
                g.row([Cell::text(format!("  {reason}"))]);
            }
            Conformance::Valid => {}
        }
        t.grid(g);
    }
}
