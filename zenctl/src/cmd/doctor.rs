//! `doctor` — diff what the fleet *serves* against local registry files, and
//! check the fleet against the RFC contracts it claims to follow.
//!
//! Since #55 the checks live in the engine (`zenkey_fleet::doctor`), where
//! the GUI doctor panel calls the exact same [`zenkey_fleet::run_doctor`];
//! this command is orchestration and rendering: load the local slices,
//! run, print, and apply the opt-in `--fail-on` exit policy.
//!
//! `--watch` (#227) re-runs the checks on an interval and reports **check-id
//! transitions** as ndjson through the engine's delta machinery
//! ([`zenkey_fleet::DoctorWatch`]): the first run states the baseline (one
//! line per stable check id, from `null`), every later run prints only
//! genuine changes — and a run that *fails* flips every check to
//! `unobservable`, which is the third state doing its job: a doctor that
//! could not run has not said the fleet is healthy.

use std::io::Write as _;

use anyhow::Result;
use zenkey_fleet::DoctorSpec;

use crate::Bus;
use crate::cli::FailOn;
use crate::report::DoctorSeverity;

#[allow(clippy::too_many_arguments)]
pub async fn run(
    deep: bool,
    sample: Option<usize>,
    listen: Option<f64>,
    fail_on: Option<FailOn>,
    watch: bool,
    every: f64,
    runs: Option<u64>,
    args: &Bus,
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
    let locals = zenkey_fleet::SliceSet::from_dirs(&dirs)?.slices().to_vec();

    let spec = DoctorSpec {
        deep,
        sample,
        timeout: args.timeout(),
        listen: listen.map(std::time::Duration::from_secs_f64),
    };
    if watch {
        return watch_loop(&session, &locals, &spec, every, runs, args).await;
    }

    let report = zenkey_fleet::run_doctor(&session, args.base(), &locals, &spec).await?;
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())?;

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

/// The `--watch` loop: run, delta, say only what changed. A failed run is a
/// transition to `unobservable`, not a process death — the stream stays
/// honest across a fleet that comes and goes.
async fn watch_loop(
    session: &zenoh::Session,
    locals: &[zenkey::RegistrySlice],
    spec: &DoctorSpec,
    every: f64,
    runs: Option<u64>,
    args: &Bus,
) -> Result<()> {
    if every <= 0.0 {
        anyhow::bail!("--every must be a positive number of seconds");
    }
    eprintln!(
        "doctor --watch: re-running every {every}s — the first run states the \
         baseline (one ndjson line per check id), later runs print only genuine \
         transitions (#227)"
    );
    let mut watch = zenkey_fleet::DoctorWatch::new();
    let mut out = std::io::stdout();
    let mut done = 0u64;
    loop {
        let outcome = zenkey_fleet::run_doctor(session, args.base(), locals, spec).await;
        let at = zenkey_fleet::record::rfc3339_now();
        let transitions = match &outcome {
            Ok(report) => watch.observe(Ok(report), &at),
            Err(e) => watch.observe(Err(&e.to_string()), &at),
        };
        for t in &transitions {
            if let Ok(line) = serde_json::to_string(t) {
                let _ = writeln!(out, "{line}");
            }
        }
        let _ = out.flush();
        done += 1;
        if runs.is_some_and(|n| done >= n) {
            return Ok(());
        }
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs_f64(every)) => {}
            _ = tokio::signal::ctrl_c() => {
                eprintln!("doctor --watch: interrupted after {done} run(s)");
                return Ok(());
            }
        }
    }
}
