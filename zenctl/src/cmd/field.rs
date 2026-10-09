//! `zenctl field` (#223; zk2's since #612, FJ8b) — field intelligence over a
//! window, through the engine's [`zenkey_fleet::run_field`]. All judgement
//! lives engine-side; this resolves the deployment, opens the two sessions
//! a raw observer needs — the watch in no namespace, the lens in the
//! namespace — and renders the report.
//!
//! Each payload is decoded through its contract exactly as `echo` decodes
//! it, and a path is judged against the paths its declared type declares,
//! read from the bundle itself.
//!
//! Findings are output, not verdicts — exit 0, the `doctor` discipline,
//! unless you opt in with `--fail-on` (#307).

use anyhow::Result;
use zenkey_fleet::Lens;

use crate::bus::Deployment;
use crate::cli::FailOn;
use crate::cmd::zk2;
use crate::exit::unaskable;
use crate::report::DoctorSeverity;

pub async fn run(cli: crate::cli::FieldArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let contracts = zk2::load_contracts(&cli.contracts)?;
    let crate::cli::FieldArgs {
        selector,
        for_secs,
        max_paths,
        fail_on,
        contracts: _,
        ns: _,
    } = cli;
    let namespace = dep.namespace().to_owned();
    let selector = zk2::wire_selector(selector.as_deref(), &namespace)?;
    let window = super::positive_secs("--for", for_secs)?;
    if max_paths == 0 {
        return Err(unaskable!(
            "--max-paths must be at least 1 — a zero bound observes nothing"
        ));
    }

    let store = zk2::store(&dep, &contracts);
    let ns_session = dep.session().await?;
    let catalog = zk2::read_lens(&dep, &ns_session, &store).await;
    let held = store.held().len();
    let lens = Lens::new(&namespace, catalog.as_ref(), &store)
        .offline(&contracts)
        .held(held);
    let session = dep.link().session().await?;
    let spec = zenkey_fleet::FieldSpec {
        selector: selector.clone(),
        window,
        max_paths,
    };

    eprintln!(
        "field: watching {selector} for {for_secs}s — subscriber declared before \
         the window opened (O4)"
    );
    let report = zenkey_fleet::run_field(&session, &lens, &spec).await?;
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;

    if trips(&report.findings, fail_on) {
        std::process::exit(crate::exit::FINDING);
    }
    Ok(())
}

/// Whether a finding list trips an opt-in `--fail-on` ceiling — `doctor`'s
/// rule, spelled once here.
fn trips(findings: &[zenkey_fleet::report::FieldFinding], fail_on: Option<FailOn>) -> bool {
    let any = |s: DoctorSeverity| findings.iter().any(|f| f.severity == s);
    match fail_on {
        Some(FailOn::Error) => any(DoctorSeverity::Error),
        Some(FailOn::Warning) => any(DoctorSeverity::Error) || any(DoctorSeverity::Warning),
        None => false,
    }
}
