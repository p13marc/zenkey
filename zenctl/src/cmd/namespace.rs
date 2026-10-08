//! `zenctl namespace list` (#612, FJ4): the deployment namespaces zk2
//! services hold instance tokens in.
//!
//! The raw half of the FJ decision (2026-10-08): every resolved verb runs in
//! one namespace, so finding which namespaces exist is asked of a session in
//! none — through a namespaced one, every other namespace is invisible. It
//! replaces v1's `base list`, which swept `**/v1/*/state/*/alive` the same
//! way.

use anyhow::Result;

use crate::bus::Link;

/// `namespace list`.
pub async fn list(cli: crate::cli::NamespaceListArgs) -> Result<()> {
    let link = Link::resolve(&cli.session)?;
    let session = link.session().await?;
    let listing = zenkey_fleet::namespace_listing(&session, link.timeout()).await?;
    crate::completion::remember_namespaces(
        link.context_name(),
        listing.namespaces.iter().map(|n| n.namespace.clone()),
    );
    crate::render::emit_with(
        &mut std::io::stdout(),
        &listing,
        link.format(),
        link.color(),
    )
}
