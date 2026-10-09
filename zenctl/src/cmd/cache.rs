//! `zenctl cache show|refresh|clear` (issue #54) — the name cache, visible.
//!
//! Every presence read leaves the names it saw here — service addresses and
//! interfaces per namespace — and `namespace list` the namespaces, which
//! means this tool leaves files on a user's disk without being asked. That
//! is only acceptable if the user can see them, refresh them and delete
//! them — so these three commands exist alongside the caching itself rather
//! than after somebody complains.
//!
//! Nothing reads the cache except shell completion. It is never a source of
//! truth, and no command answers from it. v1's slice cache (the registry
//! slices every slice-loading command wrote) left with the v1 registry
//! (#612, FJ9).

use anyhow::Result;
use zenkey_fleet::PresenceScope;

use crate::bus::{Deployment, Link};

/// `zenctl cache <verb>` — the three, dispatched.
pub async fn dispatch(cmd: crate::cli::CacheCmd) -> Result<()> {
    use crate::cli::CacheCmd;
    match cmd {
        CacheCmd::Show { session } => show(&Link::resolve(&session)?),
        CacheCmd::Refresh { ns } => refresh(&Deployment::resolve(&ns)?).await,
        CacheCmd::Clear { session } => clear(&Link::resolve(&session)?),
    }
}

/// Where this invocation's cache lives.
fn dir(context: Option<&str>) -> std::path::PathBuf {
    zenkey_explorer_config::cache_dir(zenkey_explorer_config::active_name(context).as_deref())
}

pub fn show(link: &Link) -> Result<()> {
    let dir = dir(link.context_name());
    let names = crate::completion::cached_names(link.context_name());
    // A report rather than four `println!`s, so `cache show --format json |
    // jq -r .dir` works — the cache's whole justification is that a tool
    // leaving files on a user's disk can be asked about them, and a script is
    // a user too (#54, #198).
    let report = crate::render::CacheReport {
        dir: dir.display().to_string(),
        listed: names.namespaces.iter().cloned().collect(),
        seen: names
            .seen
            .iter()
            .map(|(ns, seen)| crate::render::CachedNamespace {
                namespace: ns.clone(),
                services: seen.services.len(),
                ifaces: seen.ifaces.len(),
            })
            .collect(),
    };
    crate::render::emit_with(&mut std::io::stdout(), &report, link.format(), link.color())
}

/// One presence read of the whole namespace — the read `service list`
/// makes, which is what writes the cache.
pub async fn refresh(dep: &Deployment) -> Result<()> {
    let session = dep.session().await?;
    let catalog = crate::cmd::zk2::presence(dep, &session, &PresenceScope::all()).await?;
    let action = crate::render::CacheAction {
        action: "refreshed",
        dir: dir(dep.link().context_name()).display().to_string(),
        services: Some(catalog.addresses().count()),
        existed: true,
    };
    crate::render::emit_with(&mut std::io::stdout(), &action, dep.format(), dep.color())
}

pub fn clear(link: &Link) -> Result<()> {
    let dir = dir(link.context_name());
    let existed = match std::fs::remove_dir_all(&dir) {
        Ok(()) => true,
        // Nothing to remove is the desired end state, not a failure — but it
        // is a different fact, and the report says which happened rather than
        // leaving a script to parse two sentences apart.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(e.into()),
    };
    let action = crate::render::CacheAction {
        action: "cleared",
        dir: dir.display().to_string(),
        services: None,
        existed,
    };
    crate::render::emit_with(&mut std::io::stdout(), &action, link.format(), link.color())
}
