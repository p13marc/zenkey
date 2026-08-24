//! `doctor` — diff what the fleet *serves* against local registry files, and
//! check the fleet against the RFC contracts it claims to follow.
//!
//! Since #55 the checks live in the engine (`zenkey_fleet::doctor`), where
//! the GUI doctor panel calls the exact same [`zenkey_fleet::run_doctor`];
//! this command is orchestration and rendering: load the local slices,
//! run, print, and apply the opt-in `--fail-on` exit policy.
//!
//! `--transitions` (#227) re-runs the checks on an interval and reports
//! **check-id transitions** as ndjson through the engine's delta machinery
//! ([`zenkey_fleet::DoctorWatch`]): the first run states the baseline (one
//! line per stable check id, from `null`), every later run prints only
//! genuine changes — and a run that *fails* flips every check to
//! `unobservable`, which is the third state doing its job: a doctor that
//! could not run has not said the fleet is healthy.
//!
//! It was spelled `--watch` until #307, and that was the wrong word twice
//! over: `--watch` elsewhere in this tool is a bare bool that re-renders a
//! *state* on a list verb, and this emits a stream of *changes*. Folding it
//! into `watchdog --rule 'doctor <check-id>'` was the alternative and does
//! not fit — that rule names one check id, while the baseline printed here is
//! one line per check id there is.

use std::io::Write as _;

use anyhow::Result;
use zenkey_fleet::DoctorSpec;

use crate::Bus;
use crate::cli::{DoctorArgs, FailOn};
use crate::report::DoctorSeverity;

pub async fn run(cli: DoctorArgs) -> Result<()> {
    let bus = Bus::resolve(&cli.bus)?;
    let args = &bus;
    let DoctorArgs {
        deep,
        sample,
        for_secs,
        fail_on,
        transitions,
        every,
        count,
        bus: _,
    } = cli;
    let session = args.session().await?;
    // The context's registry dirs count too — resolving through
    // `registry_dirs()` (not the raw flag) was the fix for doctor silently
    // ignoring a named context's `registry=`.
    let dirs = args.registry_dirs();
    // No `--registry` warning here: the degradation rides the report itself —
    // `DoctorReport.synced: None` plus the O4 coverage note in its renderer —
    // so every format carries it, not just a tty's stderr (review finding R1).
    // `None` when no dirs were given: the engine distinguishes "no registry
    // loaded" from "a registry that declares nothing", and an empty set said
    // the second about the first.
    let locals = (!dirs.is_empty())
        .then(|| zenkey_fleet::SliceSet::from_dirs(&dirs))
        .transpose()?;

    let listen = match for_secs {
        Some(secs) => Some(super::positive_secs("--for", secs)?),
        None => None,
    };
    let spec = DoctorSpec {
        deep,
        sample,
        timeout: args.timeout(),
        listen,
    };
    if transitions {
        return transition_loop(&session, locals.as_ref(), &spec, every, count, args).await;
    }

    let report = zenkey_fleet::run_doctor(&args.fleet(&session), locals.as_ref(), &spec).await?;
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())?;

    let failed = match fail_on {
        Some(FailOn::Error) => report.count(DoctorSeverity::Error) > 0,
        Some(FailOn::Warning) => {
            report.count(DoctorSeverity::Error) > 0 || report.count(DoctorSeverity::Warning) > 0
        }
        None => false,
    };
    if failed {
        std::process::exit(crate::exit::FINDING);
    }
    Ok(())
}

/// The `--transitions` loop: run, delta, say only what changed. A failed run
/// is a transition to `unobservable`, not a process death — the stream stays
/// honest across a fleet that comes and goes.
async fn transition_loop(
    session: &zenoh::Session,
    locals: Option<&zenkey_fleet::SliceSet>,
    spec: &DoctorSpec,
    every: f64,
    count: Option<u64>,
    args: &Bus,
) -> Result<()> {
    let period = super::positive_secs("--every", every)?;
    eprintln!(
        "doctor --transitions: re-running every {every}s — the first run states \
         the baseline (one ndjson line per check id), later runs print only \
         genuine transitions (#227)"
    );
    let mut watch = zenkey_fleet::DoctorWatch::new();
    let mut out = std::io::stdout();
    let mut done = 0u64;
    loop {
        let outcome = zenkey_fleet::run_doctor(&args.fleet(session), locals, spec).await;
        let at = zenkey_fleet::record::rfc3339_now();
        let transitions = match &outcome {
            Ok(report) => watch.observe(Ok(report), &at),
            Err(e) => watch.observe(Err(&e.to_string()), &at),
        };
        for t in &transitions {
            // Tagged (`"row":"transition"`) like every non-sample line of an
            // explorer stream — the same kind tag `watchdog` writes.
            if let Ok(v) = serde_json::to_value(t) {
                let _ = writeln!(
                    out,
                    "{}",
                    crate::render::Row::tagged("transition", v).into_line()
                );
            }
        }
        let _ = out.flush();
        done += 1;
        if count.is_some_and(|n| done >= n) {
            return Ok(());
        }
        tokio::select! {
            _ = tokio::time::sleep(period) => {}
            _ = tokio::signal::ctrl_c() => {
                eprintln!("doctor --transitions: interrupted after {done} run(s)");
                return Ok(());
            }
        }
    }
}
