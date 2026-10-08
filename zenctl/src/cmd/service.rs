//! `zenctl service list|show` (#612, FJ4): zk2's running services, from
//! their tokens and the descriptors they serve (spec §8.1, §3.3).
//!
//! They replace v1's `node` (who is alive) and v1's `service list|info`
//! (what a producer's `@rpc` plane declares): in zk2 both questions have one
//! answer, the instance's descriptor. `service call` is still v1's, in
//! `cmd/call.rs`, until FJ5's `call`.

use anyhow::Result;
use zenkey_fleet::PresenceScope;

use crate::bus::Deployment;
use crate::cmd::zk2;

/// `service list [--system S]`.
pub async fn list(cli: crate::cli::ServiceListArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let scope = match &cli.system {
        Some(s) => PresenceScope::system(s)?,
        None => PresenceScope::all(),
    };
    let session = dep.session().await?;
    let catalog = zk2::presence(&dep, &session, &scope).await?;
    crate::render::emit_with(
        &mut std::io::stdout(),
        &catalog.services(),
        dep.format(),
        dep.color(),
    )
}

/// `service show <system>/<service>`: exit 2 when presence shows no
/// instance — silence is not an answer (`crate::exit`).
pub async fn show(cli: crate::cli::ServiceShowArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let session = dep.session().await?;
    let catalog = zk2::presence(&dep, &session, &PresenceScope::service(&cli.address)).await?;
    let view = catalog.service(&cli.address);
    crate::render::emit_with(&mut std::io::stdout(), &view, dep.format(), dep.color())?;
    if view.instances.is_empty() {
        std::process::exit(crate::exit::NO_VERDICT);
    }
    Ok(())
}
