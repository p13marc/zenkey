//! **The exit contract.** One doctrine, written here once, cited everywhere.
//!
//! `zenctl` is a tool CI branches on, so its exit codes are part of its wire
//! surface. Before the #264 restructure there were two doctrines pinned side
//! by side in the corpus — `session-config-error.trycmd` argued *your input is
//! a 1*, `key-algebra.trycmd` argued *an invalid expression is a 2* — and a
//! script could not hold both. This module is the single statement of the one
//! that survived, and every verb that exits non-zero cites it.
//!
//! ## The three codes
//!
//! * **0 — asked, and the answer is clean.** Values came back; the assertion
//!   held; the payload validated; the fleet looks healthy; the act completed.
//!   A listing that found nothing to list is still a 0: an empty answer to a
//!   question that *was* asked is an answer.
//! * **1 — asked, and the answer is a finding.** The assertion did not hold;
//!   the retired family still speaks; the payload does not conform; a reply
//!   was an error envelope (RFC 05 §3); rows were refused or malformed;
//!   `--fail-on` tripped; an act failed. **`why` exits 1 when it finds a
//!   cause** — a cause *is* the finding, and the healthy fleet is the clean
//!   run.
//! * **2 — no verdict: the question could not be asked, or could not be
//!   proven.** Clap usage errors (immovable — clap owns that code, so every
//!   other refusal of *your input* joins it rather than competing with it);
//!   an expression that does not parse; a `--context` that names nothing; a
//!   `--qos`, a class, or a `--fault` kind outside its vocabulary; a
//!   `--zenoh-config` this tool refuses; silence under a fan-out; an
//!   observation too impaired to carry the claim (RFC 13 §1, the O4/O6
//!   rules); and, on the verdict verbs, *any* failure before the question was
//!   put.
//!
//! ## Where each code comes from, mechanically
//!
//! Three seams, and nothing outside them decides an exit code:
//!
//! 1. [`Unaskable`] — an input **this tool itself refuses**, before the bus is
//!    ever asked. Wrapped into the `anyhow` chain, recognised by
//!    [`code_for`], and rendered like any other error. This is the seam that
//!    moved `--context`/`--qos`/class/`--fault`/`$*` from 1 to 2.
//! 2. [`asked`] — the verdict verbs' pre-run guard. `check expect`, `check
//!    cutover`, `check retired`, `check probe`, `check schema` and `why` give
//!    their 0 **and their 1** meanings, so a `?` on their setup path would
//!    *claim a verdict the run never reached*. Every pre-run failure of
//!    theirs — resolution, session, registry, the observation itself — lands
//!    on the reserved 2 instead.
//! 3. [`verdict`] — the engine's own projection,
//!    [`zenkey_fleet::judgement_exit_code`] over the RFC 13 §1.2 [`Judgement`]
//!    shape. A verb that has a verdict does **not** hand-roll a `match`: it
//!    maps to `Judgement` engine-side (every vocabulary carries a
//!    `to_judgement()`) and exits through this function. `why`'s polarity flip
//!    falls out of that mapping rather than being spelled again here.
//!
//! ## Acts keep their 1
//!
//! `pub`, `retire`, `replay`, `gen` and `blob fetch` *do* something. "Could
//! not be proven" has no meaning for an act — either it went out or it did
//! not — so their failures are 1, and only an input **they** refuse is a 2.

use zenkey_fleet::judgement::{Judgement, judgement_exit_code};

/// Asked, clean.
pub const CLEAN: i32 = 0;
/// Asked, and the answer is a finding.
pub const FINDING: i32 = 1;
/// No verdict: the question could not be asked, or could not be proven.
pub const NO_VERDICT: i32 = 2;

/// An input **`zenctl` itself refuses** — the exit-2 half that is not clap's.
///
/// Clap already exits 2 for every mis-shaped command line, which fixes the
/// meaning of 2 for user input whether the rest of the tool agrees or not. The
/// refusals clap cannot express — a context name that is not in the config
/// file, a QoS profile outside RFC 04 §3's five, a class outside RFC 04 §1's
/// three, a `--fault` kind outside `gen`'s vocabulary, a `$*` selector RFC 03
/// §2 forbids, a window given as zero seconds — are the same kind of mistake
/// and now exit the same way.
///
/// It is an ordinary `std::error::Error`, so it rides an `anyhow` chain
/// unchanged and renders through [`crate::errors::render`] like everything
/// else; only [`code_for`] treats it specially.
#[derive(Debug)]
pub struct Unaskable(pub String);

impl std::fmt::Display for Unaskable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unaskable {}

/// Build an [`Unaskable`] error: `unaskable!("--qos takes … got {name:?}")`.
macro_rules! unaskable {
    ($($arg:tt)*) => {
        ::anyhow::Error::new($crate::exit::Unaskable(format!($($arg)*)))
    };
}
pub(crate) use unaskable;

/// The exit code an error chain deserves: [`NO_VERDICT`] when anything in it
/// is an [`Unaskable`], [`FINDING`] otherwise.
///
/// Called from `main`, once, so the choice is made in exactly one place.
pub fn code_for(err: &anyhow::Error) -> i32 {
    if err.chain().any(|c| c.is::<Unaskable>()) {
        NO_VERDICT
    } else {
        FINDING
    }
}

/// The verdict verbs' pre-run guard (seam 2 of the module doc).
///
/// `check cutover` against a bus that would not open used to exit 1 through
/// main's anyhow edge — which reads "the old family still speaks" about a bus
/// nobody listened to, and told CI exactly that. Wrap every fallible step
/// *before* the verdict in this instead: the error is rendered in the one
/// shape and the process takes the reserved 2. Listings and acts keep their 1;
/// their exit codes carry no verdict to protect.
pub fn asked<T>(verb: &str, result: anyhow::Result<T>) -> T {
    match result {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{}", crate::errors::render(&e));
            eprintln!(
                "{verb}: the question could not be asked — exit 2, the reserved \
                 non-verdict (1 here would claim a verdict this run never reached)"
            );
            std::process::exit(NO_VERDICT);
        }
    }
}

/// Exit on a verdict, through the engine's own projection (seam 3).
///
/// [`zenkey_fleet::judgement_exit_code`] is the RFC 13 §1.2 mapping and the
/// only place 0/1/2 is spelled against the [`Judgement`] shape. A verb hands
/// it the judgement its report already carries; the surface vocabularies —
/// `ExpectVerdict`, `CutoverVerdict`, `WhyVerdict`, `CondState` — each do
/// their own `to_judgement()` engine-side, which is where `why`'s inverted
/// polarity is resolved. Re-deriving any of that here would be the second
/// spelling that lets the two drift.
pub fn verdict(j: &Judgement) -> anyhow::Result<()> {
    match judgement_exit_code(j) {
        CLEAN => Ok(()),
        code => std::process::exit(code),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one thing `code_for` decides, and both answers.
    #[test]
    fn an_unaskable_anywhere_in_the_chain_is_a_two() {
        let plain = anyhow::anyhow!("the bus said no");
        assert_eq!(code_for(&plain), FINDING);

        let refused = unaskable!("unknown class {:?}", "alerts");
        assert_eq!(code_for(&refused), NO_VERDICT);

        // Wrapped by a caller's context, the way `.with_context` leaves it.
        let wrapped = refused.context("loading the selector");
        assert_eq!(code_for(&wrapped), NO_VERDICT);
    }

    /// The projection is the engine's, not a copy: assert the three poles
    /// land where the contract says, so a change engine-side fails here.
    #[test]
    fn the_projection_is_the_engines() {
        assert_eq!(
            judgement_exit_code(&Judgement::NotEstablished {
                reason: "clean".into()
            }),
            CLEAN
        );
        assert_eq!(judgement_exit_code(&Judgement::Established), FINDING);
        assert_eq!(judgement_exit_code(&Judgement::NotAsked), NO_VERDICT);
    }
}
