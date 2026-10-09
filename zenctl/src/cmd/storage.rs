//! `zenctl storage list` — the storages this bus admits to — and `zenctl
//! storage gen`, the block that makes a router admit to the right ones
//! (#393).
//!
//! v1's coverage join (which declared state families a storage holds) and
//! the registry `ttl_s` a lifespan was derived from left with the v1
//! registry (#612, FJ9). zk2's derivation (#704) reads the enrollment `acl
//! gen` reads, and the contracts: a union storage per event resource
//! (spec §2.6), its lifespan the contract's retention, and nothing on any
//! owner's state (S4) — a deployment file's selector there is refused, exit
//! 2. The doctor's `storage-on-state` judges a running storage the same way;
//! `--check --against` judges a router config file.

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

/// The plan from what the verb was given (#704): the deployment file, the
/// enrollment and its contracts, or both. Every failure is a refusal of the
/// caller's input — exit 2 — and so is a plan that refused a storage on
/// owners' state (S4), which the caller sees in the plan first.
fn plan_of(
    bus: &Deployment,
    deployment: Option<&std::path::Path>,
    enrollment: Option<&std::path::Path>,
    contracts: &crate::cli::ContractArgs,
) -> Result<zenkey_fleet::report::StoragePlan> {
    let dep = deployment.map(read_deployment).transpose()?;
    let enrolled = enrollment
        .map(crate::cmd::acl::load_enrollment)
        .transpose()?;
    if enrolled.is_some() && contracts.contracts.is_empty() {
        return Err(crate::exit::unaskable!(
            "--enrollment needs --contracts: the event resources and their retentions are \
             the contracts'"
        ));
    }
    let set = crate::cmd::zk2::load_contracts(contracts)?;
    zenkey_fleet::plan_storages_from(
        bus.namespace(),
        zenkey_fleet::StorageInputs {
            deployment: dep.as_ref(),
            enrollment: enrolled.as_ref(),
            contracts: &set,
        },
    )
    .map_err(anyhow::Error::from)
}

/// The S4 refusals of a plan (#704): a storage on owners' state is input
/// this tool refuses outright, whatever else the plan holds.
fn s4_refused(plan: &zenkey_fleet::report::StoragePlan) -> Vec<&str> {
    plan.refusals
        .iter()
        .filter(|r| r.cite == zenkey_fleet::model::storage::S4)
        .filter_map(|r| r.storage.as_deref())
        .collect()
}

/// The refusal an S4 plan exits with: exit 2, citing the rule.
fn refuse_s4(names: &[&str]) -> anyhow::Error {
    crate::exit::unaskable!(
        "storage {} would answer on owners' state keys: a deployment MUST NOT run a storage \
         there (spec §4.2 S4) — current state is the owner's answer, and last-known state an \
         archive's (§4.4)",
        names.join(", ")
    )
}

/// `storage gen` (#393, #704; `gen` is a reserved word in edition 2024,
/// hence `plan`): the plan, the zenohd block, the check, or the
/// explanation — four ways out. Only `--check` without `--against` opens a
/// session.
pub async fn plan(cli: crate::cli::StorageGenArgs) -> Result<()> {
    let crate::cli::StorageGenArgs {
        enrollment,
        deployment,
        contracts,
        json5,
        check,
        against,
        explain,
        ns,
    } = cli;
    if check {
        return check_run(
            deployment.as_deref(),
            enrollment.as_deref(),
            &contracts,
            against.as_deref(),
            &ns,
        )
        .await;
    }
    let bus = Deployment::resolve(&ns)?;
    let plan = plan_of(
        &bus,
        deployment.as_deref(),
        enrollment.as_deref(),
        &contracts,
    )?;
    let s4 = s4_refused(&plan);

    if let Some(key) = explain {
        if !s4.is_empty() {
            for note in crate::render::refusal_notes(&plan) {
                eprintln!("{}", note.to_line());
            }
            return Err(refuse_s4(&s4));
        }
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
    if !s4.is_empty() {
        // S4 (#704): the plan is shown — the refusal is in it — and the
        // input is refused, exit 2: a storage on owners' state is not a
        // caveat to emit around.
        return Err(refuse_s4(&s4));
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

/// The router config file's `plugins.storage_manager` block, as zenoh's
/// own loader parsed the file (#704): what `zenohd` would run. `None`
/// when the file configures no storage manager.
fn load_against(path: &std::path::Path) -> Result<Option<serde_json::Value>> {
    let config = zenoh::Config::from_file(path).map_err(|e| {
        crate::exit::unaskable!(
            "--against {}: zenoh refused the config: {e}",
            path.display()
        )
    })?;
    let plugins: Option<serde_json::Value> = config
        .get_json("plugins")
        .ok()
        .and_then(|p| serde_json::from_str(&p).ok());
    Ok(plugins.and_then(|p| p.get("storage_manager").cloned()))
}

/// `--check`: a verdict verb, so every step before the comparison lands on
/// the reserved 2 through [`ASKING`] rather than claiming a difference the
/// run never measured — a plan that refused a storage on owners' state
/// (S4) included: the question it would ask is not one to put.
async fn check_run(
    deployment: Option<&std::path::Path>,
    enrollment: Option<&std::path::Path>,
    contracts: &crate::cli::ContractArgs,
    against: Option<&std::path::Path>,
    args: &crate::cli::NamespaceArgs,
) -> Result<()> {
    let bus = ASKING.ask(Deployment::resolve(args));
    let plan = ASKING.ask(plan_of(&bus, deployment, enrollment, contracts));
    // A refused storage is not compared — say which, so a clean check over
    // three of four storages is not read as a clean check over four.
    for note in crate::render::refusal_notes(&plan) {
        eprintln!("{}", note.to_line());
    }
    let s4 = s4_refused(&plan);
    if !s4.is_empty() {
        ASKING.ask::<(), _>(Err(refuse_s4(&s4)));
    }
    let report = match against {
        // A file, not the bus: what is compared is what zenohd would parse.
        Some(file) => {
            let block = ASKING.ask(load_against(file));
            zenkey_fleet::check_storages_against(&plan, block.as_ref(), &file.display().to_string())
        }
        None => {
            // The admin space sits outside every namespace: the session is
            // in none.
            let session = ASKING.ask(bus.link().session().await);
            let observed = ASKING.ask(zenkey_fleet::storages(&session, bus.timeout()).await);
            zenkey_fleet::check_storages(&plan, &observed)
        }
    };
    crate::render::emit_with(&mut std::io::stdout(), &report, bus.format(), bus.color())?;
    crate::exit::verdict(&report.judgement)
}
