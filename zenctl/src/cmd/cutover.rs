//! `zenctl check cutover` (issue #59): the RFC 09 §6 acceptance, half one.
//!
//! Thin by design (issue #206). The sample-bucketing rule, the old-wins-ties
//! tie-break and the three-state verdict ladder are judgement over bus
//! traffic, so they live in `zenkey_fleet::judge::cutover` where the other explorer
//! can reach them. What is left here is what only a CLI has: the session, the
//! rendering, and the exit — through [`crate::exit::verdict`], the one place
//! 0/1/2 is spelled.

use anyhow::Result;

use crate::Bus;

/// This verb's name, spelled once (#355) — the dispatcher uses it too.
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("check cutover");

pub async fn run(old_root: &str, for_secs: f64, args: &Bus) -> Result<()> {
    // The flag is seconds; the engine takes a `Duration`, which is what a
    // window *is* — the conversion belongs at this edge and nowhere deeper.
    let window = ASKING.ask(super::positive_secs("--for", for_secs));
    // A verdict verb: a session that will not open is `asked`'s exit 2 — an
    // exit 1 here would read "the old family still speaks" about a bus
    // nobody listened to.
    let session = ASKING.ask(args.session().await);
    let base = args.base().to_string();

    // Stated before the window opens, not after: a user watching a 30-second
    // silence deserves to know what was being watched (O5).
    eprintln!(
        "{}",
        zenkey_fleet::cutover_scope_note(old_root, &zenkey_fleet::new_prefix(&base), window)
    );

    let report =
        ASKING.ask(zenkey_fleet::run_cutover(&args.fleet(&session), old_root, window).await);
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())?;
    // The one part that cannot move: a library returns a verdict, a command
    // exits with it.
    crate::exit::verdict(&report.verdict.to_judgement())
}
