//! `zenctl storage list` — the storages this bus admits to, and which
//! declared state keys they actually cover — and `zenctl storage gen`, the
//! block that makes a router admit to the right ones (#393).
//!
//! Lived inline in the dispatch until #209, and the coverage join lived there
//! *twice*: once here and once in `cmd/watch.rs`'s polling form, which had
//! dropped the note about its own degradation on the way. One join now, one
//! sentence, and the watch says it each cycle because that is what every other
//! note in `watch::redraw` does.

use anyhow::Result;

use crate::Bus;
use crate::report;

/// The RFC 04 §2 coverage join: which declared state keys a storage holds.
///
/// Slices only **enrich** this — the storages are the answer, and the join is
/// an extra column — so a fleet with no reachable registry degrades to
/// storages-only rather than failing. It says so: an empty coverage column
/// that means "not asked" and one that means "nothing covered" are different
/// claims, and a reader cannot tell them apart from the table (RFC 09 §5.1
/// O4).
pub async fn coverage(
    args: &Bus,
    storages: &[zenkey_fleet::StorageInfo],
) -> Vec<zenkey_fleet::CoverageRow> {
    match args.slices_optional().await {
        Ok(Some(slices)) => zenkey_fleet::state_coverage(&slices, args.base(), storages),
        // `slices_optional` has already said why, once.
        Ok(None) => Vec::new(),
        // A source the user named, failing: not this function's to swallow,
        // but not worth losing the storages over either — they are the
        // answer. Rendered through the one error shape (`errors::render`),
        // like every other error this tool prints.
        Err(e) => {
            eprintln!("{}", crate::errors::render(&e));
            Vec::new()
        }
    }
}

/// The storages and their coverage, once.
/// `storage list`, with the `--watch` decision where the verb is (#354).
pub async fn list(cli: crate::cli::StorageListArgs) -> Result<()> {
    let bus = Bus::resolve(&cli.bus)?;
    let crate::cli::StorageListArgs {
        watch,
        every,
        bus: _,
    } = cli;
    if watch {
        return crate::cmd::watch::storage_list(every, &bus).await;
    }
    once(&bus).await
}

async fn once(args: &Bus) -> Result<()> {
    let session = args.session().await?;
    let storages = zenkey_fleet::storages(&session, args.timeout()).await?;
    let coverage = coverage(args, &storages).await;
    crate::render::emit_with(
        &mut std::io::stdout(),
        &report::StorageList { storages, coverage },
        args.format(),
        args.color(),
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
/// `plan`): the plan, the zenohd block, the check, or the
/// explanation — one deployment file, one registry read, four ways out.
///
/// The registry only **enriches** the plan: without one the lifespans fall
/// back to RFC 09 §2.3's default and every derivation says so
/// (`slices_optional`, #210). A registry the user *named* that will not
/// read stays fatal, as everywhere.
pub async fn plan(cli: crate::cli::StorageGenArgs) -> Result<()> {
    let crate::cli::StorageGenArgs {
        deployment,
        json5,
        check,
        explain,
        bus,
    } = cli;
    if check {
        return check_run(&deployment, &bus).await;
    }
    let bus = Bus::resolve(&bus)?;
    let dep = read_deployment(&deployment)?;
    let slices = bus.slices_optional().await?;
    let plan = zenkey_fleet::plan_storages(slices.as_ref(), bus.base(), &dep);

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
async fn check_run(deployment: &std::path::Path, args: &crate::cli::BusArgs) -> Result<()> {
    let bus = ASKING.ask(Bus::resolve(args));
    let dep = ASKING.ask(read_deployment(deployment));
    let session = ASKING.ask(bus.session().await);
    let observed = ASKING.ask(zenkey_fleet::storages(&session, bus.timeout()).await);
    let slices = ASKING.ask(bus.slices_optional().await);
    let plan = zenkey_fleet::plan_storages(slices.as_ref(), bus.base(), &dep);
    // A refused storage is not compared — say which, so a clean check over
    // three of four storages is not read as a clean check over four.
    for note in crate::render::refusal_notes(&plan) {
        eprintln!("{}", note.to_line());
    }
    let report = zenkey_fleet::check_storages(&plan, &observed);
    crate::render::emit_with(&mut std::io::stdout(), &report, bus.format(), bus.color())?;
    crate::exit::verdict(&report.judgement)
}
