//! `doctor` — diff what the fleet *serves* against local registry files, and
//! check the fleet against the RFC contracts it claims to follow.
//!
//! Since #55 the checks live in the engine (`zenkey_fleet::judge::doctor`), where
//! the GUI doctor panel calls the exact same [`zenkey_fleet::run_v1_doctor`];
//! this command is orchestration and rendering: load the local slices,
//! run, print, and exit through the report's own judgement — the opt-in
//! `--fail-on` threshold, and the reserved 2 for a run that judged nothing
//! (#510).
//!
//! `--transitions` (#227) re-runs the checks on an interval and reports
//! **check-id transitions** as ndjson through the engine's delta machinery
//! ([`zenkey_fleet::V1DoctorWatch`]): the first run states the baseline (one
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
use zenkey_fleet::V1DoctorSpec;

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
    // `V1DoctorReport.synced: None` plus the O4 coverage note in its renderer —
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
    let spec = V1DoctorSpec {
        deep,
        sample,
        timeout: args.timeout(),
        listen,
    };
    if transitions {
        return transition_loop(&session, locals.as_ref(), &spec, every, count, args).await;
    }

    let report = zenkey_fleet::run_v1_doctor(&args.fleet(&session), locals.as_ref(), &spec).await?;
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())?;

    // The exit is the report's own judgement (RFC 13 §1.2), projected by
    // the one seam: a finding at or above `--fail-on` is the 1, none is the
    // 0 — and with no `--fail-on`, findings are output, so 0 — unless the
    // run judged nothing, which is the 2 under every threshold (#510). An
    // empty bus used to exit 0 under `--fail-on error`, so a monitoring job
    // pointed at the wrong endpoint or base stayed green forever.
    let judgement = report.judgement(fail_on.map(|f| match f {
        FailOn::Error => DoctorSeverity::Error,
        FailOn::Warning => DoctorSeverity::Warning,
    }));
    if judgement.is_unobservable() {
        eprintln!(
            "doctor: nothing was judged — exit 2, the reserved non-verdict (0 here would \
             call an empty bus a healthy fleet)"
        );
    }
    crate::exit::verdict(&judgement)
}

/// The `--transitions` loop: run, delta, say only what changed. A failed run
/// is a transition to `unobservable`, not a process death — the stream stays
/// honest across a fleet that comes and goes.
async fn transition_loop(
    session: &zenoh::Session,
    locals: Option<&zenkey_fleet::SliceSet>,
    spec: &V1DoctorSpec,
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
    let mut watch = zenkey_fleet::V1DoctorWatch::new();
    let mut out = std::io::stdout();
    let mut done = 0u64;
    // One listener for the whole loop, held across every iteration (#334).
    // Constructed per-iteration it was registered only while the `select!`
    // was parked: a SIGINT arriving during a sweep cleared tokio's `pending`
    // flag, failed its send with no receiver to send to, and was gone — while
    // the first `ctrl_c()` had already taken SIGINT's default disposition
    // away, so the process did not die either.
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        // The sweep is *inside* the select, not before it: a `doctor` run is
        // the long part of a cycle, and Ctrl-C during one has to end it.
        let fleet = args.fleet(session);
        let outcome = tokio::select! {
            biased;
            _ = &mut ctrl_c => {
                eprintln!("doctor --transitions: interrupted after {done} run(s)");
                return Ok(());
            }
            outcome = zenkey_fleet::run_v1_doctor(&fleet, locals, spec) => outcome,
        };
        let at = zenkey_fleet::rfc3339_now();
        let transitions = match &outcome {
            Ok(report) => watch.observe(Ok(report), &at),
            Err(e) => watch.observe(Err(&e.to_string()), &at),
        };
        // Tagged (`"row":"transition"`) like every non-sample line of an
        // explorer stream — the same kind tag `watchdog` writes.
        //
        // Checked, unlike the `let _ = writeln!` this replaced (#360): this
        // is the verb's primary output, and a run whose transitions went
        // nowhere used to finish and exit 0 as if it had reported them.
        // `Row::of` rather than `if let Ok(v) = serde_json::to_value(t)` for
        // the same reason — serializing a `Transition` cannot fail, and the
        // `if let` read as a line this verb was willing to drop.
        let written = (|| -> std::io::Result<()> {
            for t in &transitions {
                writeln!(
                    out,
                    "{}",
                    crate::render::Row::of("transition", t).into_line()
                )?;
            }
            out.flush()
        })();
        if let Err(e) = written {
            // A closed pipe is the consumer saying "enough" — `| head -1` is
            // not a failure of the checks.
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                return Ok(());
            }
            return Err(anyhow::Error::new(e).context("failed to write the transition stream"));
        }
        done += 1;
        if count.is_some_and(|n| done >= n) {
            return Ok(());
        }
        tokio::select! {
            biased;
            _ = &mut ctrl_c => {
                eprintln!("doctor --transitions: interrupted after {done} run(s)");
                return Ok(());
            }
            _ = tokio::time::sleep(period) => {}
        }
    }
}
