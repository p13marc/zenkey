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

/// Strip zenoh's trailing ` at <path>.rs:<line>.` source locations — the
/// same trim `zenctl/src/errors.rs` applies (#240): the path is one inside
/// the **cargo registry of the machine that built the binary**, and passed
/// through it is the longest part of an error that exists on nobody's
/// computer. Conservative: only a slash-bearing `.rs` path with a line
/// number is a source location.
pub fn without_source_locations(text: &str) -> String {
    text.lines().map(trim_line).collect::<Vec<_>>().join("\n")
}

fn trim_line(line: &str) -> String {
    let mut out = line;
    while let Some(i) = out.rfind(" at ") {
        if is_source_location(&out[i + 4..]) {
            out = out[..i].trim_end();
        } else {
            break;
        }
    }
    out.to_string()
}

fn is_source_location(tail: &str) -> bool {
    let tail = tail.strip_suffix('.').unwrap_or(tail);
    let Some((path, line)) = tail.rsplit_once(':') else {
        return false;
    };
    path.contains('/')
        && path.ends_with(".rs")
        && !line.is_empty()
        && line.bytes().all(|b| b.is_ascii_digit())
}

/// Render an error chain the way zenctl does: `Error: …`, then each cause
/// under `Caused by:` — one shape for every exit path, every source
/// location trimmed.
pub fn render(err: &anyhow::Error) -> String {
    let mut out = format!("Error: {}", without_source_locations(&err.to_string()));
    let causes: Vec<String> = err
        .chain()
        .skip(1)
        .map(|c| without_source_locations(&c.to_string()))
        .collect();
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
        // The build machine's registry path goes; a sentence ending in
        // "at" and a user's own file stay (#240).
        assert_eq!(
            without_source_locations(
                "Unable to connect!  at /srv/dev/cargo/registry/src/x/zenoh-1.10.0/src/net/runtime/orchestrator.rs:442."
            ),
            "Unable to connect!"
        );
        assert_eq!(without_source_locations("look at"), "look at");
        assert_eq!(
            without_source_locations("read at /etc/zenoh.json5"),
            "read at /etc/zenoh.json5"
        );
    }
}
