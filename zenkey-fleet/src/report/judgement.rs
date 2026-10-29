//! The judgement core (observer §1) — one shape under every verdict.
//!
//! Every tool verdict in this workspace is a surface naming of one four-pole
//! shape:
//!
//! > `Established(yes) | Established(no) | Unestablished(NotAsked) |
//! > Unestablished(Unobservable{reason})`
//!
//! `spec/observer.md` §1 is the normative text for this shape. The O#
//! citations across this crate are the tooling guide's observer
//! obligations (its §3), v1's numbers kept; O1, O4, O5 and O7 are
//! observer §8.1, §1, §8.2 and §8.3.
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
//! Domain vocabularies — [`crate::report::CondState`],
//! [`crate::report::ExpectVerdict`], [`crate::report::CutoverVerdict`],
//! [`crate::report::WhyVerdict`], the `why` ladder's per-rung answer — remain
//! surface namings with documented mappings onto this core; each mapping
//! lives beside its vocabulary. The mapping convention every verdict-level
//! `to_judgement()` follows: **the judged claim is the finding** — a verdict
//! that found something maps to `Established`, a clean one to
//! `NotEstablished`. That convention is what makes the exit projection below
//! a pure function; a vocabulary whose own polarity is inverted
//! ([`crate::report::WhyVerdict`]: `Explained` is the *finding* and its CLI
//! historically exits 0) does the flip at its mapping, never downstream.
//!
//! ## Serialized form
//!
//! `Judgement` serializes with an `answer` tag —
//! `{"answer": "established"}`, `{"answer": "not_established", "reason": …}`,
//! `{"answer": "not_asked"}`, `{"answer": "unobservable", "reason": …}` —
//! byte-identical, for the three poles it had, to the `why` ladder's shipped
//! rung answer (#214), whose wire shape this type now carries directly.

use schemars::JsonSchema;
use serde::Serialize;

/// One question's judgement — the four-pole core every tool verdict maps
/// onto (observer §1). See the module doc.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
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
    /// not answered no (observer §1).
    NotAsked,
    /// Unestablished: the question was put and the observation cannot carry
    /// the claim — a drop under a completeness claim, a window shorter than
    /// the claim's span, or an ask that failed (tooling guide O6).
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

/// The exit projection (tooling guide §1): `0` = established-clean, `1` =
/// established-finding, `2` = unestablished (not asked, or unobservable).
///
/// The projection reads the core convention (module doc): the judged claim
/// is the **finding**, so `Established` is the finding exit and
/// `NotEstablished` the clean one. A vocabulary with inverted surface
/// polarity handles the flip in its own `to_judgement()` mapping (v1's
/// `WhyVerdict` was the documented case), never here — this
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

    /// The 0/1/2 projection (tooling guide §1): established-clean /
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
