//! `zenctl storage list` — the storages this bus admits to — and `zenctl
//! storage gen`, the block that makes a router admit to the right ones
//! (#393).
//!
//! v1's coverage join (which declared state families a storage holds) and
//! the registry `ttl_s` a lifespan was derived from left with the v1
//! registry (#612, FJ9). The doctor's `storage-on-state` is the zk2
//! question about a running storage: does it answer on an owner's state
//! keys (S4)?

use anyhow::Result;

use crate::bus::{Deployment, Link};
use crate::report;

/// `storage list`, with the `--watch` decision where the verb is (#354).
pub async fn list(cli: crate::cli::StorageListArgs) -> Result<()> {
    let link = Link::resolve(&cli.session)?;
    let crate::cli::StorageListArgs {
        watch,
        every,
        session: _,
    } = cli;
    if watch {
        return crate::cmd::watch::storage_list(every, &link).await;
    }
    once(&link).await
}

async fn once(link: &Link) -> Result<()> {
    let session = link.session().await?;
    let storages = zenkey_fleet::storages(&session, link.timeout()).await?;
    crate::render::emit_with(
        &mut std::io::stdout(),
        &report::StorageList { storages },
        link.format(),
        link.color(),
    )
}

/// The verdict half's name, spelled once (#355).
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("storage gen --check");

/// Read the deployment file. Both failures are refusals of the caller's
/// input — exit 2, `crate::exit` — and the TOML error carries serde's own
/// wording, which names the key it did not know.
fn read_deployment(path: &std::path::Path) -> Result<zenkey_fleet::report::Deployment> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        crate::exit::unaskable!("--deployment: cannot read {}: {e}", path.display())
    })?;
    toml::from_str(&text)
        .map_err(|e| crate::exit::unaskable!("--deployment {}: {e}", path.display()))
}

/// `storage gen` (#393; `gen` is a reserved word in edition 2024, hence
/// `plan`): the plan, the zenohd block, the check, or the explanation —
/// one deployment file, four ways out. Only `--check` opens a session.
pub async fn plan(cli: crate::cli::StorageGenArgs) -> Result<()> {
    let crate::cli::StorageGenArgs {
        deployment,
        json5,
        check,
        explain,
        ns,
    } = cli;
    if check {
        return check_run(&deployment, &ns).await;
    }
    let bus = Deployment::resolve(&ns)?;
    let dep = read_deployment(&deployment)?;
    let plan = zenkey_fleet::plan_storages(bus.namespace(), &dep);

    if let Some(key) = explain {
        let report = zenkey_fleet::explain_storage(&plan, &key);
        return crate::render::emit_with(
            &mut std::io::stdout(),
            &report,
            bus.format(),
            bus.color(),
        );
    }
    if json5 {
        // The block on stdout, and everything *about* it on stderr — the
        // same sentences the report rendering says, so a reader who piped
        // the file still learns what was refused and what could not be
        // verified.
        print!("{}", zenkey_fleet::storage_plan_json5(&plan));
        for note in crate::render::notes_with_bounds(&plan) {
            // All but the pointer at `--json5` itself, which is where the
            // reader already is.
            if note.kind != crate::render::NoteKind::NextStep {
                eprintln!("{}", note.to_line());
            }
        }
    } else {
        crate::render::emit_with(&mut std::io::stdout(), &plan, bus.format(), bus.color())?;
    }
    if plan.storages.is_empty() && !plan.refusals.is_empty() {
        // An act with nothing left to do: the input was refused whole, so
        // this is a 2 and not the 1 an act's failure would be (`crate::exit`).
        return Err(crate::exit::unaskable!(
            "every storage was refused — nothing to emit (the refusals are above)"
        ));
    }
    Ok(())
}

/// `--check`: a verdict verb, so every step before the comparison lands on
/// the reserved 2 through [`ASKING`] rather than claiming a difference the
/// run never measured.
async fn check_run(deployment: &std::path::Path, args: &crate::cli::NamespaceArgs) -> Result<()> {
    let bus = ASKING.ask(Deployment::resolve(args));
    let dep = ASKING.ask(read_deployment(deployment));
    // The admin space sits outside every namespace: the session is in none.
    let session = ASKING.ask(bus.link().session().await);
    let observed = ASKING.ask(zenkey_fleet::storages(&session, bus.timeout()).await);
    let plan = zenkey_fleet::plan_storages(bus.namespace(), &dep);
    // A refused storage is not compared — say which, so a clean check over
    // three of four storages is not read as a clean check over four.
    for note in crate::render::refusal_notes(&plan) {
        eprintln!("{}", note.to_line());
    }
    let report = zenkey_fleet::check_storages(&plan, &observed);
    crate::render::emit_with(&mut std::io::stdout(), &report, bus.format(), bus.color())?;
    crate::exit::verdict(&report.judgement)
}
