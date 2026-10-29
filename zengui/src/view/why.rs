//! The Inspector's Why section (#214): why is this key — or this service —
//! silent, the non-verdict itemised. zk2's ladder since #614 (FL8b): the
//! engine's [`zenkey_fleet::run_why`], the one `zenctl why` prints (#702).
//!
//! "Silence is never a verdict" (observer §2) is correct and it is also
//! where the user is abandoned; the ladder turns the refusal into rungs —
//! namespace, key, presence, descriptor, contract, answer, last-known — each
//! asking whether the silence is caused *there*, so a rung's finding is the
//! yes: a cause. Its one law is O4's: a rung the ladder never reached
//! renders **"not asked"**, and a rung whose observation could not be had
//! **"unobservable"** — never "no", and never each other.
//!
//! Cost posture: one button, one run. Only what the ladder reaches is read
//! — presence, the descriptors, the contracts, then the key's answer as its
//! kind is read (S4's GET for state, a short subscription for a stream, a
//! bounded GET for an event) and, after an owner's silence only, the
//! archives (S6). An operation is never called, and nothing runs under
//! `@op` or during a replay.

use std::sync::Arc;

use iced::widget::{Column, row};
use zenkey_fleet::report::{Judgement, WhyReport, WhyRung};

use crate::message::{Message, PaneMsg, SlotId};
use crate::view::kit;
use crate::view::theme::{JudgementTone, Tone};
use crate::view::tokens::Spacing;

/// The section's interactions.
#[derive(Debug, Clone)]
pub enum WhyMsg {
    /// The "why?" button — and the palette's entry. Runs on the subject.
    Run,
    /// A ladder finished.
    Done(Result<Arc<WhyReport>, crate::services::ServiceError>),
}

/// The section's state — follows the subject.
#[derive(Default)]
pub struct WhyState {
    pub in_flight: bool,
    /// What the running (or landed) ladder was asked about — the
    /// staleness guard.
    pub asked: Option<String>,
    pub report: Option<Result<Arc<WhyReport>, crate::services::ServiceError>>,
}

impl WhyState {
    /// A new subject: the old ladder explained the old one.
    pub fn forget_subject(&mut self) {
        self.in_flight = false;
        self.asked = None;
        self.report = None;
    }
}

fn msg(slot: SlotId, m: WhyMsg) -> Message {
    Message::Pane(PaneMsg::Why(slot, m))
}

/// The badge word for one rung's answer, `zenctl why`'s own: a cause, a
/// healthy rung, one that could not be observed, one never reached. "not
/// asked" is spelled out — the whole section exists so it never reads as
/// "no" (O4).
pub fn rung_label(verdict: &Judgement) -> &'static str {
    match verdict {
        Judgement::Established => "cause",
        Judgement::NotEstablished { .. } => "healthy",
        Judgement::Unobservable { .. } => "unobservable",
        Judgement::NotAsked => "not asked",
    }
}

/// The ladder's reading in one sentence, `zenctl why`'s headline: a cause
/// at its rung, every rung healthy, or no verdict and why.
pub fn headline(report: &WhyReport) -> String {
    match (&report.verdict, report.stopped_at) {
        (Judgement::Established, Some(at)) => match report.cause() {
            Some((_, cause)) => format!("a cause, at {at}: {cause}"),
            None => format!("a cause, at {at}"),
        },
        (Judgement::NotEstablished { .. }, _) => {
            "every rung healthy, and the key answers".to_string()
        }
        (Judgement::Unobservable { reason }, Some(at)) => {
            format!("no verdict, at {at} — {reason}")
        }
        (Judgement::NotAsked, _) => {
            "no verdict: the answer was not asked — why calls nothing".to_string()
        }
        (Judgement::Unobservable { reason }, None) => format!("no verdict — {reason}"),
        (Judgement::Established, None) => "a cause".to_string(),
    }
}

/// The Why section for a key or an address subject. `slot` is the subject
/// slot the surface is bound to (#257) — `Run` carries it home. `sp` is the
/// dock's resolved spacing grid (#192): a section spends it, it never
/// resolves one.
pub fn section(state: &WhyState, slot: SlotId, sp: Spacing) -> Column<'_, Message> {
    let mut col = Column::new().spacing(sp.sm);

    let run_label = if state.in_flight { "asking…" } else { "why?" };
    let mut run = kit::primary(kit::caption(run_label)).padding(sp.xs);
    if !state.in_flight {
        run = run.on_press(msg(slot, WhyMsg::Run));
    }
    col = col.push(kit::section_header("Why", Some(run.into())));

    let Some(report) = &state.report else {
        // Never-run is not a ladder of "no" (O4).
        col = col.push(kit::muted(
            "not asked yet — \"why?\" runs zk2's silence ladder here: presence, the \
             descriptor, the contract, then the answer as the key's kind is read (S4's \
             GET, a short listen, a bounded GET) and, after an owner's silence, the \
             archives — nothing is called",
        ));
        return col;
    };
    let report = match report {
        Ok(r) => r,
        Err(e) => {
            return col.push(kit::error(format!("why run failed: {e}")));
        }
    };

    // The overall reading first — the verdict the CLI exits with. A callout
    // in its tone (#564): a cause is a finding (Caution), every rung
    // healthy is commentary (Info), no verdict is the outline of a question
    // that could not be answered (Neutral).
    let tone = match report.verdict {
        Judgement::Established => Tone::Caution,
        Judgement::NotEstablished { .. } => Tone::Info,
        Judgement::Unobservable { .. } | Judgement::NotAsked => Tone::Neutral,
    };
    col = col.push(kit::callout(
        tone,
        kit::caption(format!("why {} — {}", report.target, headline(report))),
    ));
    // What the ladder put to the bus, and no wider (O5).
    if report.asked.is_empty() {
        col = col.push(kit::muted(
            "asked the bus nothing — the key alone decided where the ladder stopped",
        ));
    } else {
        col = col.push(kit::muted(format!("asked {}", report.asked.join(" · "))));
    }
    if let Some(s) = report.window_s {
        col = col.push(kit::muted(format!(
            "listened {s:.0}s for a sample on the stream"
        )));
    }

    // The ladder, a rung a row with echo's leading edge (#564): the badge
    // carries the answer, its reason or cause sits under the rung it
    // belongs to.
    for rung in &report.rungs {
        col = col.push(rung_row(rung, sp));
    }

    // What an archive still holds after an owner's silence: last-known,
    // never current, and no explanation of the silence (S6).
    if let Some(lk) = &report.last_known {
        col = col.push(kit::callout(
            Tone::Info,
            kit::caption(format!(
                "{} still holds a last-known value of {} ({}): last-known, never current — \
                 the owner's silence is not explained by it (S6)",
                lk.archive,
                lk.key,
                if lk.confirmed {
                    "confirmed by alignment"
                } else {
                    "not confirmed by alignment"
                }
            )),
        ));
    }
    if report.verdict.is_not_asked() {
        col = col.push(kit::muted(
            "an operation's answer is not asked: why calls nothing — the Workbench's call \
             asks it, and its silence is attributed through presence",
        ));
    }
    col
}

/// One rung: its answer's badge, its id and the core section it reads, the
/// question it asks — then the cause it established or the reason behind a
/// healthy or unobservable answer.
fn rung_row<'a>(rung: &'a WhyRung, sp: Spacing) -> iced::Element<'a, Message> {
    let mut body = Column::new().spacing(sp.xs).push(
        row![
            kit::badge_judgement(JudgementTone::of(&rung.verdict), rung_label(&rung.verdict)),
            kit::mono(format!("{} ({})", rung.rung, rung.section)),
            kit::muted(rung.rung.question()),
        ]
        .spacing(sp.sm)
        .align_y(iced::Alignment::Center),
    );
    match (&rung.verdict, &rung.cause) {
        (Judgement::Established, Some(cause)) => {
            body = body.push(kit::caption(format!("— {cause}")));
        }
        (Judgement::NotEstablished { reason }, _) | (Judgement::Unobservable { reason }, _) => {
            body = body.push(kit::caption(format!("— {reason}")));
        }
        _ => {}
    }
    kit::edge_row(
        kit::Edge::Put,
        iced::widget::container(body).padding(iced::Padding::ZERO.left(sp.sm)),
    )
}
