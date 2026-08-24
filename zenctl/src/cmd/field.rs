//! `zenctl field` (#223) — field intelligence over a window, through the
//! engine's [`zenkey_fleet::run_field`]. All judgement lives engine-side
//! (RFC 09 §5.1's rules are the engine's to keep); this maps flags in and
//! renders the report.
//!
//! Findings are output, not verdicts — exit 0, the `doctor` discipline. What
//! changed in #264 is the *opt-in*: `doctor` had `--fail-on` and this verb
//! did not, so the one place these findings could gate CI was
//! `watchdog --rule 'doctor field-stuck'`. The asymmetry was the bug, never
//! the default; the default is still 0.

use anyhow::Result;

use crate::Bus;
use crate::cli::{FailOn, SelectorArgs};
use crate::exit::unaskable;
use crate::report::DoctorSeverity;

pub async fn run(
    sel: &SelectorArgs,
    for_secs: f64,
    max_paths: usize,
    fail_on: Option<FailOn>,
    args: &Bus,
) -> Result<()> {
    let selector = super::selector_of(sel, args)?;
    let window = super::positive_secs("--for", for_secs)?;
    if max_paths == 0 {
        return Err(unaskable!(
            "--max-paths must be at least 1 — a zero bound observes nothing"
        ));
    }

    let session = args.session().await?;
    // Slices enrich: declared ttl_s (field-stuck's yardstick) and type names
    // (field-new's schema lookup) come from the registry. `None` stays `None`
    // into the engine so the report says stuck/new were unjudgeable rather
    // than reading clean (RFC 09 §5.1 O4; #246).
    let slices = args.slices_optional().await?;
    let store = zenkey_fleet::decode::SchemaStore::new(args.base(), args.timeout());
    let spec = zenkey_fleet::FieldSpec {
        selector: selector.clone(),
        window,
        max_paths,
    };

    eprintln!(
        "field: watching {selector} for {for_secs}s — subscriber declared before \
         the window opened (RFC 09 §5.1 O4)"
    );
    let report =
        zenkey_fleet::run_field(&args.fleet(&session), slices.as_ref(), &store, &spec).await?;
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())?;

    if trips(&report.findings, fail_on) {
        std::process::exit(crate::exit::FINDING);
    }
    Ok(())
}

/// Whether a finding list trips an opt-in `--fail-on` ceiling — `doctor`'s
/// rule, spelled once here because the findings are the same type.
fn trips(findings: &[crate::report::DoctorFinding], fail_on: Option<FailOn>) -> bool {
    let any = |s: DoctorSeverity| findings.iter().any(|f| f.severity == s);
    match fail_on {
        Some(FailOn::Error) => any(DoctorSeverity::Error),
        Some(FailOn::Warning) => any(DoctorSeverity::Error) || any(DoctorSeverity::Warning),
        None => false,
    }
}
