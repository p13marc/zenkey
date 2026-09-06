//! **The exit contract** — zenctl's doctrine (`zenctl/src/exit.rs`), cited
//! rather than re-argued: three codes, one meaning each, and nothing outside
//! this module decides one.
//!
//! * **0 — clean.** `run` watched until it was told to stop (SIGINT,
//!   SIGTERM, or `--once`'s one tick) and stopped cleanly; `check-config`
//!   found nothing to refuse; `test-sink` delivered.
//! * **1 — the act failed.** The transport would not open; the test
//!   delivery did not go out. zenwatch's verbs are *acts* — a daemon either
//!   ran or it did not — so "could not be proven" has no meaning for them,
//!   and their failures are 1 the way `zenctl pub`'s are. A sink that fails
//!   *during* a run is never fatal: it is counted, logged, and the run goes
//!   on, because a notifier that dies on one bad delivery pages nobody.
//! * **2 — refused input.** Clap's usage errors (immovable — clap owns the
//!   code), a config that does not parse or does not pass [`crate::config::check`],
//!   a `--context` that names nothing, a `--zenoh-config` the engine refuses,
//!   a sink name `test-sink` cannot find. The same code clap already exits
//!   with for every mis-shaped command line, so every other refusal of your
//!   input joins it rather than competing with it.
//!
//! Mechanically: [`Unaskable`] rides the `anyhow` chain and [`code_for`]
//! recognises it (and the engine's own `is_unaskable`); everything else on
//! the error path is a 1. `main` asks once.

/// Clean.
pub const CLEAN: i32 = 0;
/// The act failed.
pub const FAILED: i32 = 1;
/// Refused input — the same code as clap's usage errors.
pub const REFUSED: i32 = 2;

/// An input **zenwatch itself refuses** — the exit-2 half that is not clap's.
#[derive(Debug)]
pub struct Unaskable(pub String);

impl std::fmt::Display for Unaskable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unaskable {}

/// Build an [`Unaskable`] error: `unaskable!("sink {name:?} is not configured")`.
macro_rules! unaskable {
    ($($arg:tt)*) => {
        ::anyhow::Error::new($crate::exit::Unaskable(format!($($arg)*)))
    };
}
pub(crate) use unaskable;

/// The exit code an error chain deserves: [`REFUSED`] when anything in it is
/// an [`Unaskable`] (ours, or the engine's), [`FAILED`] otherwise.
pub fn code_for(err: &anyhow::Error) -> i32 {
    let refused = err.chain().any(|c| {
        c.is::<Unaskable>()
            || c.downcast_ref::<zenkey_fleet::Error>()
                .is_some_and(zenkey_fleet::Error::is_unaskable)
    });
    if refused { REFUSED } else { FAILED }
}

/// Render an error chain the way zenctl does: `Error: …`, then each cause
/// under `Caused by:` — one shape for every exit path.
pub fn render(err: &anyhow::Error) -> String {
    let mut out = format!("Error: {err}");
    let causes: Vec<String> = err.chain().skip(1).map(|c| c.to_string()).collect();
    match causes.len() {
        0 => {}
        1 => out.push_str(&format!("\n\nCaused by:\n    {}", causes[0])),
        _ => {
            out.push_str("\n\nCaused by:");
            for (i, c) in causes.iter().enumerate() {
                out.push_str(&format!("\n    {i}: {c}"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one thing `code_for` decides, and both answers — wrapped or not.
    #[test]
    fn a_refusal_anywhere_in_the_chain_is_a_two() {
        assert_eq!(code_for(&anyhow::anyhow!("the bus said no")), FAILED);
        let refused = unaskable!("unknown sink {:?}", "pager");
        assert_eq!(code_for(&refused), REFUSED);
        assert_eq!(code_for(&refused.context("loading the config")), REFUSED);
        let engine = anyhow::Error::new(zenkey_fleet::Error::unaskable(
            "--zenoh-config",
            "sets a namespace",
        ));
        assert_eq!(code_for(&engine), REFUSED);
    }

    #[test]
    fn the_rendering_is_the_explorers_shape() {
        let e = anyhow::anyhow!("inner").context("outer");
        assert_eq!(render(&e), "Error: outer\n\nCaused by:\n    inner");
    }
}
