//! `zenctl check retired` (issue #226) — the deprecation burn-down.
//!
//! It lived beside the `registry` noun, whose ledger it reads, until FJ4
//! retired that noun (#612); it answers under `check` because it is an
//! exit-coded assertion (#307): one contract, one family.

use anyhow::{Result, anyhow};

use crate::Bus;

/// The verdict verb's name, spelled once (#355) — the dispatcher
/// uses it too.
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("check retired");

/// `check retired`: each `[[deprecated]]` entry of the `--registry` dirs,
/// judged against the bus.
///
/// Thin for `check cutover`'s reason (#206): the per-entry ladder, the wire
/// bucketing and the worst-of verdict are judgement over bus traffic and live
/// in `zenkey_fleet::judge::retired`. What is left here is what only a CLI has: the
/// session, the rendering, and the exit code.
pub async fn run(for_secs: Option<f64>, args: &Bus) -> Result<()> {
    // Seconds off the flag, a `Duration` from here in.
    let listen = for_secs.map(|secs| ASKING.ask(super::positive_secs("--for", secs)));
    // A verdict verb: every pre-run failure below goes through `asked`'s
    // exit 2 — an exit 1 here would read "a retired subject still speaks"
    // about a ledger nobody could walk.
    let dirs = args.registry_dirs();
    // The ledger source is the dirs alone, never the bus union: the served
    // slices are a *fact to check against* (§6.1), not a second ledger.
    let local = ASKING.ask(if dirs.is_empty() {
        Err(anyhow!(
            "check retired walks the [[deprecated]] ledger of local registry \
                 files — pass --registry <dir> (or set one on the active context)"
        ))
    } else {
        zenkey_fleet::SliceSet::from_dirs(&dirs).map_err(anyhow::Error::from)
    });
    let session = ASKING.ask(args.session().await);
    let entries: usize = local.slices().iter().map(|s| s.deprecated.len()).sum();
    if let Some(window) = listen {
        // Stated before the window opens, not after (O5).
        eprintln!(
            "{}",
            zenkey_fleet::retired_scope_note(
                entries,
                &zenkey_fleet::new_prefix(args.base()),
                window
            )
        );
    }
    let registries: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
    let report = ASKING.ask(
        zenkey_fleet::run_retired(
            &args.fleet(&session),
            &local,
            registries,
            listen,
            args.timeout(),
        )
        .await,
    );
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())?;
    // The exit discipline shared with `check cutover` and `check expect`: a
    // library returns a verdict, a command exits with it, through the one
    // projection in `crate::exit` (0 = pass, 1 = a retired subject still
    // speaks, 2 = unproven — silence is not a pass).
    crate::exit::verdict(&report.verdict.to_judgement())
}
