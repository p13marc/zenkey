//! `zenctl service list|show` (#612, FJ4): zk2's running services, from
//! their tokens and the descriptors they serve (spec §8.1, §3.3).
//!
//! They replace v1's `node` (who is alive) and v1's `service list|info`
//! (what a producer's `@rpc` plane declares): in zk2 both questions have one
//! answer, the instance's descriptor. Calling one of its operations is
//! `zenctl call` (`cmd/call.rs`, FJ5), which replaced v1's `service call`.

use anyhow::Result;
use zenkey_fleet::PresenceScope;

use crate::bus::Deployment;
use crate::cli::ContractArgs;
use crate::cmd::zk2;

/// `service list [--system S] [--face-policy F]`: with a face policy, each
/// device of its device faces in scope beside, its link up, closed (never
/// down) or unobservable (observer §3.11; #982 H3).
pub async fn list(cli: crate::cli::ServiceListArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let scope = match &cli.system {
        Some(s) => PresenceScope::system(s)?,
        None => PresenceScope::all(),
    };
    let hop = zk2::first_hop(cli.first_hop.face_policy.as_deref())?;
    let describing = describing(&dep, &cli.contracts)?;
    let session = dep.session().await?;
    let catalog = zk2::presence_through(&dep, &session, &scope, &describing).await?;
    let mut listing = catalog.services();
    listing.first_hop = catalog.first_hop(&hop, &scope);
    crate::render::emit_with(&mut std::io::stdout(), &listing, dep.format(), dep.color())
}

/// `service show <system>/<service>`: exit 2 when presence shows no
/// instance — silence is not an answer (`crate::exit`), and a first-hop
/// device's link closed is none either (observer §3.11).
pub async fn show(cli: crate::cli::ServiceShowArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let hop = zk2::first_hop(cli.first_hop.face_policy.as_deref())?;
    let describing = describing(&dep, &cli.contracts)?;
    let session = dep.session().await?;
    let catalog = zk2::presence_through(
        &dep,
        &session,
        &PresenceScope::service(&cli.address),
        &describing,
    )
    .await?;
    let view = catalog.service_given(&cli.address, &hop);
    crate::render::emit_with(&mut std::io::stdout(), &view, dep.format(), dep.color())?;
    if view.instances.is_empty() {
        std::process::exit(crate::exit::NO_VERDICT);
    }
    Ok(())
}

/// What an owner's compact descriptor (core §3.3) is read through: the
/// catalogs `--catalog` loads, and a store seeded with `--contracts` that
/// retrieves each other revision it names (§8.4). A JSON one needs
/// neither, and a compact one with no catalog that reads it is not read
/// (`unknown_catalog`), never guessed.
fn describing(dep: &Deployment, args: &ContractArgs) -> Result<zenkey_fleet::Describing> {
    let offline = zk2::load_contracts(args)?;
    let catalogs = zk2::load_catalogs(args)?;
    Ok(zenkey_fleet::Describing::new(
        &catalogs,
        &zk2::store(dep, &offline),
    ))
}
