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
    let catalogs = zk2::load_catalogs(&cli.contracts)?;
    let crate::cli::FieldArgs {
        selector,
        for_secs,
        max_paths,
        face: crate::cli::AcrossFaceArgs { across_face },
        fail_on,
        contracts: _,
        ns: _,
    } = cli;
    let namespace = dep.namespace().to_owned();
    let face = zk2::FaceScope::of(&across_face, &contracts, &namespace, zk2::Reads::Samples)?;
    let typed = selector.is_some();
    let selector = zk2::wire_selector(selector.as_deref(), &namespace)?;
    // Across a face, the scope the contracts give (core §8.5); the report
    // names it as one selector, its parts joined.
    let watched = match &face {
        Some(face) => face.narrow(std::slice::from_ref(&selector), typed)?,
        None => vec![selector],
    };
    let selector = watched.join(" + ");
    let window = super::positive_secs("--for", for_secs)?;
    if max_paths == 0 {
        return Err(unaskable!(
            "--max-paths must be at least 1 — a zero bound observes nothing"
        ));
    }

    let store = zk2::store(&dep, &contracts);
    // Across a face no presence is read: no descriptor crosses, and
    // --contracts resolves every key (observer §7.5).
    let catalog = match &face {
        Some(face) => {
            eprintln!("{}", face.note(&watched));
            None
        }
        None => {
            let ns_session = dep.session().await?;
            zk2::read_lens(&dep, &ns_session, &store).await
        }
    };
    let held = store.held().len();
    let lens = Lens::new(&namespace, catalog.as_ref(), &store)
        .offline(&contracts)
        .catalogs(&catalogs)
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
    let report = zenkey_fleet::run_field_over(&session, &lens, &spec, &watched).await?;
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
