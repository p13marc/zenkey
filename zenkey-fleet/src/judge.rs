//! **Layer 3 — the judges.** Where an observation becomes a verdict.
//!
//! One rule places a module here: *it says whether something is wrong*.
//! Everything below reads what [`crate::bus`] gathered and [`crate::model`]
//! projected, and returns a document that takes a position — a doctor
//! finding, an expectation met or unmet, a probe's answer, a watchdog
//! rule's state, a path a type does not declare.
//!
//! Because a verdict is the only thing here worth having, the honesty rules
//! (RFC 13, v1.24; RFC 09 §5.1 O1–O7 for the individual rules) bite hardest
//! in this layer. Three of them shape every module:
//!
//! * **Not asked is not answered no** (O4). A check that did not run reports
//!   that it did not run.
//! * **An observation that cannot carry the claim is neither pass nor fail**
//!   (O6) — a drop under a completeness claim, a window shorter than the
//!   claim's span.
//! * **Silence is never a verdict** (RFC 05 §2.1). A judge that heard
//!   nothing says what it asked and what did not answer.
//!
//! [`crate::report::Judgement`] is where those three become a type: the
//! four-pole core every surface verdict maps onto, and the 0/1/2 exit
//! projection beside it. It sits under `report/` rather than here because it
//! is serde-pinned — `{"answer": "not_asked"}` reaches a script — and the
//! placement rule on [`crate::report`] admits no exceptions, not even for the
//! vocabulary the judges are written in.
//!
//! [`common`] holds the rest of that vocabulary: the caps on how many
//! offenders a report names. A judge importing another judge is the signal
//! it should be: it means one of them is doing the other's work.
//!
//! What a judge returns is a **report**, and every serialized report shape
//! lives in [`crate::report`], not here (see that module's placement rule).

pub mod common;
pub mod condition;
pub mod conform;
pub mod doctor;
pub mod doctor_delta;
pub mod expect;
pub mod field;
pub mod probe;
pub mod why;
