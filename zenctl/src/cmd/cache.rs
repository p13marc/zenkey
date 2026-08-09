//! `zenctl cache show|refresh|clear` (issue #54) — the slice cache, visible.
//!
//! Any command that loads registry slices writes them here, which means this
//! tool now leaves files on a user's disk without being asked. That is only
//! acceptable if the user can see them, refresh them and delete them — so
//! these three commands exist alongside the caching itself rather than after
//! somebody complains.
//!
//! Nothing reads the cache except shell completion. It is never a source of
//! truth, and no command answers from it.

use anyhow::Result;

use crate::BusArgs;

/// Where this invocation's cache lives.
fn dir(args: &BusArgs) -> std::path::PathBuf {
    zenkey_fleet::cache_dir(zenkey_fleet::active_name(args.context_name()).as_deref())
}

pub fn show(args: &BusArgs) -> Result<()> {
    let dir = dir(args);
    println!("{}", dir.display());
    let set = zenkey_fleet::SliceSet::read_cache(&dir);
    if set.slices().is_empty() {
        println!(
            "  empty — completion falls back to the static command tree. Any command that \
             loads slices fills it (or `zenctl cache refresh`)."
        );
        return Ok(());
    }
    for slice in set.slices() {
        println!(
            "  {:<16} registry {:<8} {} subject(s), {} procedure(s)",
            slice.name,
            slice.version,
            slice.subjects.len(),
            slice.procedures.len()
        );
    }
    println!(
        "\n{} producer(s), from the last sighting — suggestions, not an inventory.",
        set.slices().len()
    );
    Ok(())
}

pub async fn refresh(args: &BusArgs) -> Result<()> {
    // Loading is what writes the cache, so this is a load with no output.
    let set = args.slice_set().await?;
    println!(
        "cached {} producer(s) to {}",
        set.slices().len(),
        dir(args).display()
    );
    Ok(())
}

pub fn clear(args: &BusArgs) -> Result<()> {
    let dir = dir(args);
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => println!("removed {}", dir.display()),
        // Nothing to remove is the desired end state, not a failure.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("{} does not exist — nothing to clear", dir.display());
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}
