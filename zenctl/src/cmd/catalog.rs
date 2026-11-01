//! `zenctl catalog show|check` (#980 C1c) — the deployment catalog, a file
//! the deployment provisions to its owners and to the far side of a
//! constrained face (core §8.6).
//!
//! A catalog is never fetched and never served on the bus, so neither verb
//! opens a session: the catalog is `--catalog`'s, read as §8.6 reads it
//! (`zenkey_fleet::Catalogs::load`), and the contracts in hand are
//! `--contracts`'. `show` reads one; `check` is a judgement, exit-coded
//! under `crate::exit`'s contract: 1 when the document is refused, when it
//! does not succeed `--previous`, or when a revision in hand is not in it;
//! 0 when none applies; 2 when a file cannot be read or nothing is given.
//! `zk2 catalog new|add` writes one.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Result;
use zenkey_fleet::DeploymentCatalog;
use zenkey_model::canonical::Fingerprint;
use zenkey_model::grammar::IfaceId;

use crate::cli::{CatalogCheckArgs, CatalogShowArgs, ContractArgs};
use crate::cmd::zk2;
use crate::exit::unaskable;
use crate::render::{
    CatalogAncestor, CatalogEntryRow, CatalogMissing, CatalogReport, CatalogSuccession,
};

/// The verdict verb's name, spelled once (#355).
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("catalog check");

/// `zenctl catalog show --catalog <PATH> [--contracts …]`.
pub fn show(cli: CatalogShowArgs) -> Result<()> {
    let path = one(&cli.contracts)?;
    let contracts = zk2::load_contracts(&cli.contracts)?;
    let catalog = zenkey_fleet::Catalogs::load(path).map_err(|e| unaskable!("--catalog {e}"))?;
    let report = report(path, &catalog, held(&cli.contracts, &contracts));
    crate::render::emit_with(
        &mut std::io::stdout(),
        &report,
        cli.out.format,
        cli.out.color,
    )
}

/// `zenctl catalog check --catalog <PATH> [--previous <PATH>] [--contracts …]`.
pub fn check(cli: CatalogCheckArgs) -> Result<()> {
    let path = ASKING.ask(one(&cli.contracts)).to_owned();
    let contracts = ASKING.ask(zk2::load_contracts(&cli.contracts));
    let previous = cli
        .previous
        .as_ref()
        .map(|p| zenkey_fleet::Catalogs::load(p).map_err(|e| unaskable!("--previous {e}")));
    let previous = ASKING.ask(previous.transpose());
    let bytes = ASKING
        .ask(std::fs::read(&path).map_err(|e| unaskable!("--catalog {}: {e}", path.display())));
    let report = match DeploymentCatalog::parse_bytes(&bytes) {
        Err(e) => CatalogReport {
            path: path.display().to_string(),
            refused: Some(e.to_string()),
            id: None,
            epoch: None,
            epoch_utc: None,
            lineage: Vec::new(),
            entries: Vec::new(),
            succession: None,
            missing: Vec::new(),
            judged: true,
        },
        Ok(catalog) => {
            let mut report = report(&path, &catalog, held(&cli.contracts, &contracts));
            report.judged = true;
            report.succession = previous.as_ref().map(|p| match catalog.succeeds(p) {
                Ok(()) => CatalogSuccession {
                    previous: p.id().to_string(),
                    verdict: "succeeds".to_owned(),
                    detail: None,
                },
                Err(s) => CatalogSuccession {
                    previous: p.id().to_string(),
                    verdict: s.token().to_owned(),
                    detail: Some(s.to_string()),
                },
            });
            report.missing = contracts
                .iter()
                .filter(|r| catalog.index_of(r.iface(), r.fingerprint()).is_none())
                .map(|r| CatalogMissing {
                    iface: r.iface().to_string(),
                    contract: r.fingerprint().to_string(),
                })
                .collect();
            report
        }
    };
    crate::render::emit_with(
        &mut std::io::stdout(),
        &report,
        cli.out.format,
        cli.out.color,
    )?;
    if report.finding() {
        std::process::exit(crate::exit::FINDING);
    }
    Ok(())
}

/// The one `--catalog` a catalog verb reads.
fn one(args: &ContractArgs) -> Result<&Path> {
    match args.catalog.as_slice() {
        [one] => Ok(one),
        [] => Err(unaskable!(
            "--catalog is required: the catalog file to read"
        )),
        several => Err(unaskable!(
            "{} catalogs given: a catalog verb reads one --catalog",
            several.len()
        )),
    }
}

/// The revisions `--contracts` holds, when it was given.
fn held(
    args: &ContractArgs,
    contracts: &zenkey_fleet::ContractSet,
) -> Option<BTreeSet<(IfaceId, Fingerprint)>> {
    (!args.contracts.is_empty()).then(|| {
        contracts
            .iter()
            .map(|r| (r.iface().clone(), r.fingerprint().clone()))
            .collect()
    })
}

/// A catalog read, each entry with whether it is in hand.
fn report(
    path: &Path,
    catalog: &DeploymentCatalog,
    held: Option<BTreeSet<(IfaceId, Fingerprint)>>,
) -> CatalogReport {
    let epoch = catalog.epoch();
    CatalogReport {
        path: path.display().to_string(),
        refused: None,
        id: Some(catalog.id().to_string()),
        epoch: Some(epoch),
        epoch_utc: Some(zenkey_fleet::rfc3339_from_unix(
            u64::try_from(epoch).unwrap_or_default(),
        )),
        lineage: catalog
            .lineage()
            .iter()
            .map(|a| CatalogAncestor {
                id: a.id.to_string(),
                entries: a.entries,
            })
            .collect(),
        entries: catalog
            .entries()
            .iter()
            .zip(0u16..)
            .map(|(e, index)| CatalogEntryRow {
                index,
                iface: e.iface.to_string(),
                contract: e.contract.to_string(),
                held: held
                    .as_ref()
                    .map(|h| h.contains(&(e.iface.clone(), e.contract.clone()))),
            })
            .collect(),
        succession: None,
        missing: Vec::new(),
        judged: false,
    }
}
