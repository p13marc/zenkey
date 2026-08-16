//! `doctor` — diff what the fleet *serves* against local registry files, and
//! check the fleet against the RFC contracts it claims to follow.
//!
//! Since #55 the checks live in the engine (`zenkey_fleet::doctor`), where
//! the GUI doctor panel calls the exact same [`zenkey_fleet::run_doctor`];
//! this command is orchestration and rendering: load the local slices,
//! run, print, and apply the opt-in `--fail-on` exit policy.

use anyhow::Result;
use zenkey_fleet::DoctorSpec;

use crate::report::DoctorSeverity;
use crate::{BusArgs, FailOn, offline, output};

pub async fn run(
    deep: bool,
    sample: Option<usize>,
    listen: Option<f64>,
    fail_on: Option<FailOn>,
    args: &BusArgs,
) -> Result<()> {
    let session = args.session().await?;
    // The context's registry dirs count too — resolving through
    // `registry_dirs()` (not the raw flag) was the fix for doctor silently
    // ignoring a named context's `registry=`.
    let dirs = args.registry_dirs();
    if dirs.is_empty() {
        eprintln!(
            "note: no --registry <dir> given — skipping the served-vs-declared diff; only the \
             roster-vs-introspect check runs."
        );
    }
    let locals = offline::load_slices(&dirs)?;

    let report = zenkey_fleet::run_doctor(
        &session,
        args.base(),
        &locals,
        &DoctorSpec {
            deep,
            sample,
            timeout: args.timeout(),
            listen: listen.map(std::time::Duration::from_secs_f64),
        },
    )
    .await?;
    output::doctor(&report, args.format)?;

    let failed = match fail_on {
        Some(FailOn::Error) => report.count(DoctorSeverity::Error) > 0,
        Some(FailOn::Warning) => {
            report.count(DoctorSeverity::Error) > 0 || report.count(DoctorSeverity::Warning) > 0
        }
        None => false,
    };
    if failed {
        std::process::exit(1);
    }
    Ok(())
}
