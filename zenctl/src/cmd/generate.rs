//! `zenctl gen` (#612, FJ8a) — a contract-driven mock owner.
//!
//! Orchestration only: the plan, the bring-up, the schedule and the
//! synthesis live in `zenkey_fleet::tape::{generate, mock, synth}`. This
//! command's job is the etiquette: print the whole plan before anything is
//! brought up (the replay dry-run precedent), and refuse an address an
//! instance already runs at unless `--i-know` — a mock owner beside a real
//! one is a second writer of its keys (P3) and a split-brain on every
//! exclusive resource (spec §6).
//!
//! It replaced v1's registry-driven generator (#162/#163). What went with
//! it: the registry walk (`--producer`, `--subject`, `--var`), the
//! impersonated origin (`--origin`; a mock owner is a service of its own at
//! the address you name), `--serve-describe` (a zk2 owner serves its
//! descriptor and bundle by itself), `--schema-set` (the bundle is the
//! schema), `--wide`, and the fault injector (`--fault`): a zk2 mock writes
//! through the runtime's writers, which keep the contract's QoS, `Encoding`
//! and stamps, and a fault that bypassed them would be a second write path.
//! The `v1` branch keeps all of it.

use anyhow::Result;
use zenkey_fleet::report::GenPlan;
use zenkey_fleet::{GenPattern, GenSpec, MemberArg};

use crate::bus::Deployment;
use crate::cli::Pattern;
use crate::cmd::zk2;
use crate::exit::unaskable;

impl From<Pattern> for GenPattern {
    fn from(p: Pattern) -> GenPattern {
        match p {
            Pattern::Steady => GenPattern::Steady,
            Pattern::Jitter => GenPattern::Jitter,
            Pattern::Burst => GenPattern::Burst,
            Pattern::Ramp => GenPattern::Ramp,
        }
    }
}

pub async fn run(cli: crate::cli::GenArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let crate::cli::GenArgs {
        address,
        ifaces,
        members,
        binds,
        rate,
        pattern,
        duration,
        seed,
        dry_run,
        i_know,
        contracts,
        ns: _,
    } = cli;
    // Everything the command line can be refused for, before a session.
    let contracts = zk2::load_contracts(&contracts)?;
    let members = members
        .iter()
        .map(|m| MemberArg::parse(m))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let bindings = zk2::binds(&binds)?;
    // The one `--duration` in the tool, and it bounds *output* rather than
    // an observation — which is why it kept the word while every window
    // became `--for` (#307).
    let run_for = super::positive_secs("--duration", duration)?;
    if let Some(hz) = rate
        && !(hz.is_finite() && hz > 0.0)
    {
        return Err(unaskable!(
            "--rate must be a positive number of Hz, got {hz}"
        ));
    }

    let mut session = None;
    let revisions = if ifaces.is_empty() {
        zk2::every_revision(&contracts)?
    } else {
        let mut out = Vec::new();
        for spec in &ifaces {
            out.push(zk2::revision(&dep, &contracts, spec, &mut session).await?);
        }
        out
    };
    let spec = GenSpec {
        address,
        revisions,
        members,
        bindings,
        rate_hz: rate,
        pattern: pattern.into(),
        duration: run_for,
        seed,
        tool: "zenctl gen".into(),
    };
    let plan: GenPlan = zenkey_fleet::gen_plan(&spec)?;

    // The plan, before anything is brought up (the replay dry-run precedent).
    crate::render::emit_with(&mut std::io::stdout(), &plan, dep.format(), dep.color())?;
    if dry_run {
        eprintln!("--dry-run: nothing brought up");
        return Ok(());
    }

    let session = match session {
        Some(s) => s,
        None => dep.session().await?,
    };
    super::serve::guard_address(&session, &spec.address, &dep, i_know).await?;
    let report = zenkey_fleet::run_gen(&session, &spec, &plan, |instance| {
        eprintln!(
            "up: {} as instance {instance}, in {}, for {}s",
            spec.address,
            zk2::namespace_phrase(dep.namespace()),
            duration
        );
    })
    .await?;
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    // An act's finding (`crate::exit`): samples the mock could not send.
    if report.failed > 0 {
        std::process::exit(crate::exit::FINDING);
    }
    Ok(())
}
