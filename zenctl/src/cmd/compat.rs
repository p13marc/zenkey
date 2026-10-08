//! `zenctl compat <old> <new>` (#612, FJ4): two contract revisions, the
//! classifier's verdict (spec §9.8).
//!
//! The classifier is `zenkey_model::compat` — the one the contract CI binary
//! (`zk2 contract compat`) and the codegen's history gate run — through the
//! fleet's projection onto `CompatReport`. What is here is what only a CLI
//! has: reading each side from whatever was named, and the exit code.
//!
//! A side is an authoring file (`*.toml`), a bundle file (`*.bundle.json`,
//! verified against its own content), or `<iface>[@<fingerprint>]`: a
//! revision `--contracts` holds, or one the deployment's descriptors name,
//! retrieved from its holders. A verdict verb, so every failure before the
//! comparison is the reserved 2 (`crate::exit::asked`), never a 1 that would
//! read as "breaking".

use std::path::Path;

use anyhow::Result;
use zenkey_fleet::report::{CompatSide, ContractSource};
use zenkey_model::bundle::Bundle;
use zenkey_model::canonical::Fingerprint;
use zenkey_model::compat::Revision;

use crate::bus::Deployment;
use crate::cmd::zk2;
use crate::exit::unaskable;

/// The verdict verb's name, spelled once (#355).
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("compat");

/// `compat <old> <new>`: exit 0 compatible, 1 review or breaking, 2 no
/// verdict.
pub async fn run(cli: crate::cli::CompatArgs) -> Result<()> {
    let dep = ASKING.ask(Deployment::resolve(&cli.ns));
    let contracts = ASKING.ask(zk2::load_contracts(&cli.contracts));
    let mut session = None;
    let old = ASKING.ask(side(&cli.old, &dep, &contracts, &mut session).await);
    let new = ASKING.ask(side(&cli.new, &dep, &contracts, &mut session).await);
    let report = zenkey_fleet::compat((old.0, &old.1), (new.0, &new.1));
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    crate::exit::verdict(&report.to_judgement())
}

/// One side, read from what was named.
async fn side(
    input: &str,
    dep: &Deployment,
    contracts: &zenkey_fleet::ContractSet,
    session: &mut Option<zenoh::Session>,
) -> Result<(CompatSide, Revision)> {
    let path = Path::new(input);
    if input.ends_with(".bundle.json") {
        let bytes = std::fs::read(path).map_err(|e| unaskable!("{input}: {e}"))?;
        let b = Bundle::verify(&bytes)
            .map_err(|e| unaskable!("{input}: not a bundle that verifies: {e}"))?;
        let iface = b.contract["interface"].as_str().unwrap_or("?").to_owned();
        return Ok((
            CompatSide {
                input: input.to_owned(),
                source: ContractSource::Bundle,
                iface,
                fingerprint: b.fingerprint().to_string(),
            },
            Revision::of_bundle(&b),
        ));
    }
    if input.ends_with(".toml") || path.is_file() {
        if !path.is_file() {
            return Err(unaskable!("{input}: no such file"));
        }
        let l = zenkey_model::contract::load_path(path);
        let Some(c) = l.contract else {
            return Err(unaskable!(
                "{input} is not a valid contract:\n{}",
                l.report.to_string().trim_end()
            ));
        };
        return Ok((
            CompatSide {
                input: input.to_owned(),
                source: ContractSource::File,
                iface: c.iface.to_string(),
                fingerprint: Fingerprint::of(&c).to_string(),
            },
            Revision::of(&c),
        ));
    }
    let spec = crate::cli::revision_arg(input).map_err(|e| {
        unaskable!("{input:?} is neither a file (`*.toml`, `*.bundle.json`) nor a revision: {e}")
    })?;
    let r = zk2::revision(dep, contracts, &spec, session).await?;
    Ok((
        CompatSide {
            input: input.to_owned(),
            source: r.source(),
            iface: r.iface().to_string(),
            fingerprint: r.fingerprint().to_string(),
        },
        Revision::of_bundle(r.bundle()),
    ))
}
