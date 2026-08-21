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
    let set = zenkey_fleet::SliceSet::read_cache(&dir);
    // A report rather than four `println!`s, so `cache show --format json |
    // jq -r .dir` works — the cache's whole justification is that a tool
    // leaving files on a user's disk can be asked about them, and a script is
    // a user too (#54, #198).
    let report = crate::render::CacheReport {
        dir: dir.display().to_string(),
        slices: set
            .slices()
            .iter()
            .map(|s| crate::render::CachedSlice {
                producer: s.name.clone(),
                registry_version: s.version.clone(),
                subjects: s.subjects.len(),
                procedures: s.procedures.len(),
            })
            .collect(),
    };
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())
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
