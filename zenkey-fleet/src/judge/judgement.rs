//! The judgement core (RFC 13, v1.24) — one shape under every verdict.
//!
//! Every tool verdict in this workspace is a surface naming of one four-pole
//! shape:
//!
//! > `Established(yes) | Established(no) | Unestablished(NotAsked) |
//! > Unestablished(Unobservable{reason})`
//!
//! RFC 13 (v1.24) is the normative chapter for this shape; before v1.24 the
//! material lived in RFC 09 §5.1 (the O1–O7 observation rules), and the O#
//! citations across this crate still point there for the individual rules.
//! The poles:
//!
//! * [`Judgement::Established`] — Established(yes): the question was put and
//!   the claim holds, conclusively.
//! * [`Judgement::NotEstablished`] — Established(no): the question was put
//!   and the claim conclusively does not hold, with the reason.
//! * [`Judgement::NotAsked`] — Unestablished: the question was never put.
//!   "Not asked" is not "answered no" (O4).
//! * [`Judgement::Unobservable`] — Unestablished: the question was put and
//!   the observation could not carry the claim (a drop under a completeness
//!   claim, a window shorter than the claim's span, an ask that failed),
//!   with the reason. Neither "fine" nor "fire" (O6).
//!
//! Domain vocabularies — [`crate::judge::condition::CondState`],
//! [`crate::report::ExpectVerdict`], [`crate::report::CutoverVerdict`],
//! [`crate::judge::why::WhyVerdict`], the `why` ladder's per-rung answer — remain
//! surface namings with documented mappings onto this core; each mapping
//! lives beside its vocabulary. The mapping convention every verdict-level
//! `to_judgement()` follows: **the judged claim is the finding** — a verdict
//! that found something maps to `Established`, a clean one to
//! `NotEstablished`. That convention is what makes the exit projection below
//! a pure function; a vocabulary whose own polarity is inverted
//! ([`crate::judge::why::WhyVerdict`]: `Explained` is the *finding* and its CLI
//! historically exits 0) does the flip at its mapping, never downstream.
//!
//! ## Serialized form
//!
//! `Judgement` serializes with an `answer` tag —
//! `{"answer": "established"}`, `{"answer": "not_established", "reason": …}`,
//! `{"answer": "not_asked"}`, `{"answer": "unobservable", "reason": …}` —
//! byte-identical, for the three poles it had, to the `why` ladder's shipped
//! rung answer (#214), whose wire shape this type now carries directly.

use serde::Serialize;

/// One question's judgement — the four-pole core every tool verdict maps
/// onto (RFC 13, v1.24; RFC 09 §5.1 pre-v1.24). See the module doc.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum Judgement {
    /// Established(yes): the question was put and the claim holds — positive
    /// evidence, conclusive.
    Established,
    /// Established(no): the question was put and the claim conclusively does
    /// not hold — with the reason, which is where the honesty lives.
    NotEstablished { reason: String },
    /// Unestablished: the question was not put — the input was not fetched,
    /// was not requested, or does not exist for this subject. Not asked is
    /// not answered no (RFC 09 §5.1 O4).
    NotAsked,
    /// Unestablished: the question was put and the observation cannot carry
    /// the claim — a drop under a completeness claim, a window shorter than
    /// the claim's span, or an ask that failed (RFC 09 §5.1 O6).
    Unobservable { reason: String },
}

impl Judgement {
    /// Whether the question reached a conclusive answer: `Some(true)` =
    /// Established(yes), `Some(false)` = Established(no), `None` = neither
    /// pole of Unestablished says anything.
    pub fn conclusive(&self) -> Option<bool> {
        match self {
            Judgement::Established => Some(true),
            Judgement::NotEstablished { .. } => Some(false),
            Judgement::NotAsked | Judgement::Unobservable { .. } => None,
        }
    }

    /// The observation was made and could not carry the claim.
    pub fn is_unobservable(&self) -> bool {
        matches!(self, Judgement::Unobservable { .. })
    }

    /// The question was never put.
    pub fn is_not_asked(&self) -> bool {
        matches!(self, Judgement::NotAsked)
    }
}

/// The RFC 13 (v1.24) exit projection: `0` = established-clean, `1` =
/// established-finding, `2` = unestablished (not asked, or unobservable).
///
/// The projection reads the core convention (module doc): the judged claim
/// is the **finding**, so `Established` is the finding exit and
/// `NotEstablished` the clean one. A vocabulary with inverted surface
/// polarity handles the flip in its own `to_judgement()` mapping
/// ([`crate::judge::why::WhyVerdict`] is the documented case), never here — this
/// function has exactly one spelling per pole.
pub fn judgement_exit_code(j: &Judgement) -> i32 {
    match j.conclusive() {
        Some(false) => 0,
        Some(true) => 1,
        None => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire vocabulary of the core, pinned: the three poles the `why`
    /// ladder shipped are byte-identical to #214's `RungAnswer`, and the
    /// fourth pole gets its own tag.
    #[test]
    fn the_four_poles_serialize_with_the_shipped_answer_tags() {
        assert_eq!(
            serde_json::to_value(Judgement::Established).unwrap(),
            serde_json::json!({"answer": "established"})
        );
        assert_eq!(
            serde_json::to_value(Judgement::NotEstablished { reason: "r".into() }).unwrap(),
            serde_json::json!({"answer": "not_established", "reason": "r"})
        );
        assert_eq!(
            serde_json::to_value(Judgement::NotAsked).unwrap(),
            serde_json::json!({"answer": "not_asked"})
        );
        assert_eq!(
            serde_json::to_value(Judgement::Unobservable { reason: "r".into() }).unwrap(),
            serde_json::json!({"answer": "unobservable", "reason": "r"})
        );
    }

    /// The 0/1/2 projection (RFC 13 v1.24): established-clean /
    /// established-finding / unestablished — and both unestablished poles
    /// share the exit, because neither is a verdict.
    #[test]
    fn the_exit_projection_is_zero_one_two() {
        assert_eq!(
            judgement_exit_code(&Judgement::NotEstablished {
                reason: "clean".into()
            }),
            0
        );
        assert_eq!(judgement_exit_code(&Judgement::Established), 1);
        assert_eq!(judgement_exit_code(&Judgement::NotAsked), 2);
        assert_eq!(
            judgement_exit_code(&Judgement::Unobservable {
                reason: "drops".into()
            }),
            2
        );
    }
}
