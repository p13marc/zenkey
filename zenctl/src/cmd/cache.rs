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

use crate::Bus;

/// `zenctl cache <verb>` — the three, dispatched.
///
/// One arm in the dispatch rather than three, on the pattern `context` set:
/// each verb resolves the same `Bus` the same way, and three copies of that
/// line is three chances for one of them to drift.
pub async fn dispatch(cmd: crate::cli::CacheCmd) -> Result<()> {
    use crate::cli::CacheCmd;
    match cmd {
        CacheCmd::Show { bus } => show(&Bus::resolve(&bus)?),
        CacheCmd::Refresh { bus } => refresh(&Bus::resolve(&bus)?).await,
        CacheCmd::Clear { bus } => clear(&Bus::resolve(&bus)?),
    }
}

/// Where this invocation's cache lives.
fn dir(args: &Bus) -> std::path::PathBuf {
    zenkey_fleet::cache_dir(zenkey_fleet::active_name(args.context_name()).as_deref())
}

pub fn show(args: &Bus) -> Result<()> {
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

pub async fn refresh(args: &Bus) -> Result<()> {
    // Loading is what writes the cache, so this is a load with no output.
    let set = args.slice_set().await?;
    emit(
        args,
        crate::render::CacheAction {
            action: "refreshed",
            dir: dir(args).display().to_string(),
            slices: Some(set.slices().len()),
            existed: true,
        },
    )
}

/// One cache action out, however the user asked for it.
fn emit(args: &Bus, action: crate::render::CacheAction) -> Result<()> {
    crate::render::emit_with(&mut std::io::stdout(), &action, args.format(), args.color())
}

pub fn clear(args: &Bus) -> Result<()> {
    let dir = dir(args);
    let existed = match std::fs::remove_dir_all(&dir) {
        Ok(()) => true,
        // Nothing to remove is the desired end state, not a failure — but it
        // is a different fact, and the report says which happened rather than
        // leaving a script to parse two sentences apart.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(e.into()),
    };
    emit(
        args,
        crate::render::CacheAction {
            action: "cleared",
            dir: dir.display().to_string(),
            slices: None,
            existed,
        },
    )
}
