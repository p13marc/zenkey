//! `zenctl storage list` — the storages this bus admits to, and which
//! declared state keys they actually cover.
//!
//! Lived inline in the dispatch until #209, and the coverage join lived there
//! *twice*: once here and once in `cmd/watch.rs`'s polling form, which had
//! dropped the note about its own degradation on the way. One join now, one
//! sentence, and the watch says it each cycle because that is what every other
//! note in `watch::redraw` does.

use anyhow::Result;

use crate::Bus;
use crate::report;

/// The RFC 04 §2 coverage join: which declared state keys a storage holds.
///
/// Slices only **enrich** this — the storages are the answer, and the join is
/// an extra column — so a fleet with no reachable registry degrades to
/// storages-only rather than failing. It says so: an empty coverage column
/// that means "not asked" and one that means "nothing covered" are different
/// claims, and a reader cannot tell them apart from the table (RFC 09 §5.1
/// O4).
pub async fn coverage(
    args: &Bus,
    storages: &[zenkey_fleet::StorageInfo],
) -> Vec<zenkey_fleet::CoverageRow> {
    match args.slices_optional().await {
        Ok(Some(slices)) => zenkey_fleet::state_coverage(&slices, args.base(), storages),
        // `slices_optional` has already said why, once.
        Ok(None) => Vec::new(),
        // A source the user named, failing: not this function's to swallow,
        // but not worth losing the storages over either — they are the answer.
        Err(e) => {
            eprintln!("error: {e:#}");
            Vec::new()
        }
    }
}

/// The storages and their coverage, once.
pub async fn list(args: &Bus) -> Result<()> {
    let session = args.session().await?;
    let storages = zenkey_fleet::storages(&session, args.timeout()).await?;
    let coverage = coverage(args, &storages).await;
    crate::render::emit_with(
        &mut std::io::stdout(),
        &report::StorageList { storages, coverage },
        args.format(),
        args.color(),
    )
}
