//! `zenctl iface list|show` (#612, FJ4): zk2 interfaces across a
//! deployment — who provides each, who requires it, and what each revision's
//! contract says.
//!
//! They replace v1's `interface` (a payload type across producers) and
//! `topic` (what a registry declares): a zk2 interface *is* its contract, so
//! both questions are answered by the descriptors that name it and the
//! bundle its fingerprint retrieves (spec §8.4).

use anyhow::Result;
use zenkey_fleet::{Contracts as _, PresenceScope};

use crate::bus::Deployment;
use crate::cmd::zk2;
use crate::exit::unanswered;

/// `iface list`.
pub async fn list(cli: crate::cli::IfaceListArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let session = dep.session().await?;
    let catalog = zk2::presence(&dep, &session, &PresenceScope::all()).await?;
    crate::render::emit_with(
        &mut std::io::stdout(),
        &catalog.ifaces(),
        dep.format(),
        dep.color(),
    )
}

/// `iface show <iface>[@<fingerprint>]`.
///
/// The whole namespace's presence is read, not the interface's tokens: a
/// tokenless provider (U22) holds no interface token to select, so only its
/// descriptor says it provides the interface (spec §8.1). Every revision a
/// descriptor names is retrieved once (or found in `--contracts`), so each
/// provider's exposure can be computed by the compact rule.
pub async fn show(cli: crate::cli::IfaceShowArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let contracts = zk2::load_contracts(&cli.contracts)?;
    let session = dep.session().await?;
    let catalog = zk2::presence(&dep, &session, &PresenceScope::all()).await?;
    let store = zk2::store(&dep, &contracts);
    let iface = &cli.target.iface;
    let wanted: Vec<_> = catalog
        .wanted()
        .into_iter()
        .filter(|(i, _)| i == iface)
        .collect();
    for (r, (_, fp)) in store
        .fetch_all(&session, &wanted)
        .await
        .into_iter()
        .zip(&wanted)
    {
        // A retrieval that could not be put on the bus leaves its revision
        // not asked, which the view shows as such; say why here.
        if let Err(e) = r {
            eprintln!("iface show: {iface}@{fp}: {}", zenkey_fleet::one_line(&e));
        }
    }
    let view = match &cli.target.fingerprint {
        None => catalog.iface(iface, &store),
        Some(_) => {
            let mut known = catalog.fingerprints_of(iface);
            known.extend(contracts.of_iface(iface).map(|r| r.fingerprint().clone()));
            let Some(fp) = zk2::pick(&cli.target, &known)? else {
                return Err(unanswered!(
                    "no revision of {iface} matches {}: --contracts holds none, and no \
                     provider's descriptor in {} names one",
                    cli.target,
                    zk2::namespace_phrase(dep.namespace())
                ));
            };
            if store.state(iface, &fp).is_none()
                && let Err(e) = store.fetch(&session, iface, &fp).await
            {
                eprintln!("iface show: {iface}@{fp}: {}", zenkey_fleet::one_line(&e));
            }
            catalog.iface_at(iface, &fp, &store)
        }
    };
    crate::render::emit_with(&mut std::io::stdout(), &view, dep.format(), dep.color())?;
    use zenkey_fleet::report::{Asked, ContractAnswer};
    let described = view
        .revisions
        .iter()
        .any(|r| matches!(r.contract, Asked::Asked(ContractAnswer::Held { .. })));
    if view.providers.is_empty() && view.consumers.is_empty() && !described {
        std::process::exit(crate::exit::NO_VERDICT);
    }
    Ok(())
}
