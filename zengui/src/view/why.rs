//! The Inspector's Why section (#214): why is this key silent — the
//! non-verdict, itemised.
//!
//! "Silence is never a verdict" (RFC 05 §3.1) is correct and it is also
//! where the user is abandoned; the why ladder ([`zenkey_fleet::why`]) turns
//! the refusal into rungs over facts the engine already holds. This section
//! renders the same ladder `zenctl why` prints — same rung ids, same three
//! answers — and its one law is O4's: a rung whose input was not fetched
//! renders **"not asked"**, never "no". A ladder that prints `No` where it
//! means `NotAsked` becomes the exact thing it was built to replace.
//!
//! Cost posture: the frugal default, always. One button, one control-plane
//! run (liveliness + admin sweeps + one bounded GET); no subscriber is ever
//! declared from here, so `wire-heard` reads "not asked" and says so. The
//! opt-in listen window stays a `zenctl why --listen` affair.

use std::sync::Arc;

use iced::widget::{Column, row};
use zenkey_fleet::why::{RungAnswer, WhyReport, WhyVerdict};

use crate::message::{Message, PaneMsg, SlotId};
use crate::view::kit;
use crate::view::theme::RungTone;
use crate::view::tokens::Spacing;

/// The section's interactions.
#[derive(Debug, Clone)]
pub enum WhyMsg {
    /// The "why?" button — and the palette's entry. Runs on the subject key.
    Run,
    /// A ladder finished.
    Done(Result<Arc<WhyReport>, String>),
}

/// The section's state — follows the subject.
#[derive(Default)]
pub struct WhyState {
    pub in_flight: bool,
    /// The key the running (or landed) ladder was asked about — the
    /// staleness guard.
    pub asked: Option<String>,
    pub report: Option<Result<Arc<WhyReport>, String>>,
}

impl WhyState {
    /// A new subject: the old ladder explained the old key.
    pub fn forget_subject(&mut self) {
        self.in_flight = false;
        self.asked = None;
        self.report = None;
    }
}

fn msg(slot: SlotId, m: WhyMsg) -> Message {
    Message::Pane(PaneMsg::Why(slot, m))
}

/// The tone one rung's answer reads as — the single mapping, so no rung can
/// wear another answer's swatch.
pub fn rung_tone(answer: &RungAnswer) -> RungTone {
    match answer {
        RungAnswer::Established => RungTone::Established,
        RungAnswer::NotEstablished { .. } => RungTone::NotEstablished,
        // Both unestablished poles of the RFC 13 (v1.24) core wear the
        // "absence of an answer" tone — an uncarriable observation is not a
        // milder `✗`.
        RungAnswer::NotAsked | RungAnswer::Unobservable { .. } => RungTone::NotAsked,
    }
}

/// The badge word for one rung's answer. "not asked" is spelled out — the
/// whole section exists so it never reads as "no" (O4).
pub fn rung_label(answer: &RungAnswer) -> &'static str {
    match answer {
        RungAnswer::Established => "established",
        RungAnswer::NotEstablished { .. } => "not established",
        RungAnswer::NotAsked => "not asked",
        RungAnswer::Unobservable { .. } => "unobservable",
    }
}

/// The Why section for a key subject. `slot` is the subject slot the
/// surface is bound to (#257) — `Run` carries it home. `sp` is the dock's
/// resolved spacing grid (#192): a section spends it, it never resolves one.
pub fn section(state: &WhyState, slot: SlotId, sp: Spacing) -> Column<'_, Message> {
    let mut col = Column::new().spacing(sp.sm);

    let run_label = if state.in_flight { "asking…" } else { "why?" };
    let mut run = kit::action(kit::caption(run_label)).padding(sp.xs);
    if !state.in_flight {
        run = run.on_press(msg(slot, WhyMsg::Run));
    }
    col = col.push(kit::section_header("Why", Some(run.into())));

    let Some(report) = &state.report else {
        // Never-run is not a ladder of "no" (O4).
        col = col.push(kit::muted(
            "not asked yet — \"why?\" runs the silence ladder on this key: \
             control-plane sweeps and one bounded GET, no subscriber \
             (RFC v1.18 frugality)",
        ));
        return col;
    };
    let report = match report {
        Ok(r) => r,
        Err(e) => {
            return col.push(kit::muted(format!("why run failed: {e}")));
        }
    };

    // The overall reading first — the same three-way verdict the CLI exits
    // with, worded so "healthy" never over-claims.
    let causes = report.causes();
    col = col.push(kit::muted(match report.verdict {
        WhyVerdict::Explained => format!("explained — cause established by: {}", causes.join(", ")),
        WhyVerdict::Healthy => "no cause established, and everything checked looks healthy \
             (\"declared, alive, never published\" lands here on purpose — \
             publishers declare lazily, RFC 08 §6.1)"
            .to_string(),
        WhyVerdict::Impaired => "no cause established, and the observation was impaired — \
             \"healthy\" cannot be claimed over questions that could not be \
             asked"
            .to_string(),
    }));
    for impairment in &report.impairments {
        col = col.push(kit::muted(format!("impaired: {impairment}")));
    }
    if let Some(s) = report.listened_s {
        col = col.push(kit::muted(format!("listened {s:.0}s (opt-in window)")));
    }

    for rung in &report.rungs {
        let header = row![
            kit::badge_rung(rung_tone(&rung.answer), rung_label(&rung.answer)),
            kit::mono(rung.id.to_string()),
            kit::muted(rung.question),
        ]
        .spacing(sp.sm)
        .align_y(iced::Alignment::Center);
        col = col.push(header);
        if let RungAnswer::NotEstablished { reason } = &rung.answer {
            col = col.push(kit::muted(format!("  — {reason}")));
        }
        for evidence in &rung.evidence {
            col = col.push(kit::muted(format!("  · {evidence}")));
        }
    }
    col
}
