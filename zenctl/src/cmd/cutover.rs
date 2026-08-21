//! `zenctl cutover` (issue #59): the RFC 09 §6 acceptance, half one.
//!
//! Thin by design (issue #206). The sample-bucketing rule, the old-wins-ties
//! tie-break and the three-state verdict ladder are judgement over bus
//! traffic, so they live in `zenkey_fleet::cutover` where the other explorer
//! can reach them. What is left here is what only a CLI has: the session, the
//! rendering, and the exit code.

use anyhow::Result;

use crate::BusArgs;

pub async fn run(old_root: &str, window: u64, args: &BusArgs) -> Result<()> {
    let session = args.session().await?;
    let base = args.base().to_string();

    // Stated before the window opens, not after: a user watching a 30-second
    // silence deserves to know what was being watched (O5).
    eprintln!(
        "{}",
        zenkey_fleet::cutover::scope_note(
            old_root,
            &zenkey_fleet::cutover::new_prefix(&base),
            window
        )
    );

    let report = zenkey_fleet::run_cutover(&session, &base, old_root, window).await?;
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())?;
    // The one part that cannot move: a library returns a verdict, a command
    // exits with it (0 = pass, 1 = the old root still speaks, 2 = unproven —
    // silence is not a pass).
    match report.verdict {
        zenkey_fleet::report::CutoverVerdict::Pass => Ok(()),
        zenkey_fleet::report::CutoverVerdict::OldStillSpeaks => std::process::exit(1),
        zenkey_fleet::report::CutoverVerdict::Unproven => std::process::exit(2),
    }
}
