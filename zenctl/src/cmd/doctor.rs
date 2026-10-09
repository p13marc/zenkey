//! `doctor` (#612, FJ6) — a zk2 deployment judged against the core, one
//! verdict per check.
//!
//! The checks live in the engine ([`zenkey_fleet::run_doctor`]), so a GUI
//! panel or a notifier asks the exact same questions; this command is
//! orchestration and rendering: resolve the deployment, open its two
//! sessions — one **in** its namespace for the deployment, one in **no**
//! namespace for the routers' admin space and the presence domain (decided
//! 2026-10-08) — run, print, and exit through the report's own judgement
//! under `--fail-on`. A verdict verb: every failure before the run is the
//! reserved 2 (`crate::exit::asked`), never a 1 that would claim a finding.
//!
//! `--transitions` (#227) re-runs the checks on an interval and reports
//! **check-id transitions** as ndjson through the engine's delta machinery
//! ([`zenkey_fleet::DoctorWatch`]): the first run states the baseline (one
//! line per check asked, from `null`), every later run prints only genuine
//! changes. A check is firing on a finding, ok when established clean, and
//! unobservable otherwise — a check the run could not establish has not
//! said the deployment is healthy.
//!
//! It was spelled `--watch` until #307, and that was the wrong word twice
//! over: `--watch` elsewhere in this tool is a bare bool that re-renders a
//! *state* on a list verb, and this emits a stream of *changes*. Folding it
//! into `watchdog --rule 'doctor <check-id>'` was the alternative and does
//! not fit — that rule names one check id, while the baseline printed here is
//! one line per check id there is.

use std::collections::BTreeSet;
use std::io::Write as _;

use anyhow::Result;
use zenkey_fleet::report::{CheckId, DoctorSeverity};
use zenkey_fleet::{BundleStore, DoctorBus, DoctorSpec};

use crate::bus::Deployment;
use crate::cli::{DoctorArgs, FailOn};

/// The verdict verb's name, spelled once (#355).
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("doctor");

pub async fn run(cli: DoctorArgs) -> Result<()> {
    let DoctorArgs {
        grace,
        deep,
        presence_budget,
        checks,
        skip,
        fail_on,
        transitions,
        every,
        count,
        ns,
    } = cli;
    let dep = ASKING.ask(Deployment::resolve(&ns));
    let grace = ASKING.ask(super::positive_secs("--grace", grace));
    let asked: BTreeSet<CheckId> = if checks.is_empty() {
        CheckId::ALL.into_iter().collect()
    } else {
        checks.into_iter().collect()
    };
    let spec = DoctorSpec {
        timeout: dep.timeout(),
        grace,
        presence_budget,
        deep,
        checks: asked.into_iter().filter(|c| !skip.contains(c)).collect(),
    };
    if deep_without_its_check(&spec) {
        eprintln!(
            "doctor: --deep asks state-stamp-foreign, which --check/--skip leave out: it \
             costs nothing here"
        );
    }
    let bus = DoctorBus {
        session: ASKING.ask(dep.session().await),
        raw: ASKING.ask(dep.link().session().await),
        namespace: dep.namespace().to_owned(),
    };
    let store = BundleStore::new(dep.timeout());
    if transitions {
        return transition_loop(&bus, &store, &spec, every, count).await;
    }

    let report = zenkey_fleet::run_doctor(&bus, &store, &spec).await;
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;

    // The exit is the report's own judgement (RFC 13 §1.2), projected by the
    // one seam: a finding at or above the floor is the 1; below it, a check
    // left unobservable or an empty scope is the 2 — it could be hiding the
    // finding — and every check asked established clean is the 0.
    let judgement = report.judgement(floor(fail_on));
    if let zenkey_fleet::Judgement::Unobservable { reason } = &judgement {
        eprintln!("doctor: {reason} — exit 2, the reserved non-verdict");
    }
    crate::exit::verdict(&judgement)
}

/// The doctor a `doctor <CHECK-ID>` rule of `watchdog` or `record --on`
/// runs (#612, FJ6): zk2's doctor reads the deployment through a session in
/// its namespace — the verb's `--base` — beside the verb's own un-namespaced
/// one, which reads the admin space. Opened only when a rule wants it.
pub(crate) async fn bus_for_rules(
    rules: &[zenkey_fleet::Condition],
    args: &crate::Bus,
    raw: &zenoh::Session,
) -> Result<Option<DoctorBus>> {
    if !rules
        .iter()
        .any(|r| matches!(r, zenkey_fleet::Condition::DoctorCheck { .. }))
    {
        return Ok(None);
    }
    Ok(Some(DoctorBus {
        session: args.session_in(args.base()).await?,
        raw: raw.clone(),
        namespace: args.base().to_owned(),
    }))
}

/// The `--fail-on` floor: warning unless told otherwise.
fn floor(fail_on: Option<FailOn>) -> DoctorSeverity {
    match fail_on {
        Some(FailOn::Error) => DoctorSeverity::Error,
        Some(FailOn::Warning) | None => DoctorSeverity::Warning,
    }
}

/// `--deep` given with a check set that leaves its one check out.
fn deep_without_its_check(spec: &DoctorSpec) -> bool {
    spec.deep && !spec.checks.contains(&CheckId::StateStampForeign)
}

/// The `--transitions` loop: run, delta, say only what changed. A check the
/// run could not establish is a transition to `unobservable`, not a process
/// death — the stream stays honest across a deployment that comes and goes.
async fn transition_loop(
    bus: &DoctorBus,
    store: &BundleStore,
    spec: &DoctorSpec,
    every: f64,
    count: Option<u64>,
) -> Result<()> {
    let period = super::positive_secs("--every", every)?;
    eprintln!(
        "doctor --transitions: re-running every {every}s — the first run states \
         the baseline (one ndjson line per check asked), later runs print only \
         genuine transitions (#227)"
    );
    let mut watch = zenkey_fleet::DoctorWatch::of(spec.checks.iter().copied());
    let mut out = std::io::stdout();
    let mut done = 0u64;
    // One listener for the whole loop, held across every iteration (#334).
    // Constructed per-iteration it was registered only while the `select!`
    // was parked: a SIGINT arriving during a run cleared tokio's `pending`
    // flag, failed its send with no receiver to send to, and was gone — while
    // the first `ctrl_c()` had already taken SIGINT's default disposition
    // away, so the process did not die either.
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        // The run is *inside* the select, not before it: a doctor run is the
        // long part of a cycle, and Ctrl-C during one has to end it.
        let report = tokio::select! {
            biased;
            _ = &mut ctrl_c => {
                eprintln!("doctor --transitions: interrupted after {done} run(s)");
                return Ok(());
            }
            report = zenkey_fleet::run_doctor(bus, store, spec) => report,
        };
        let at = zenkey_fleet::rfc3339_now();
        let transitions = watch.observe(Ok(&report), &at);
        // Tagged (`"row":"transition"`) like every non-sample line of an
        // explorer stream — the same kind tag `watchdog` writes. Checked
        // (#360): this is the verb's primary output.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The floor's default is warning: an info finding is worth knowing, not
    /// a failure; `--fail-on error` lowers what fails, never what is shown.
    #[test]
    fn the_floor_defaults_to_warning() {
        assert_eq!(floor(None), DoctorSeverity::Warning);
        assert_eq!(floor(Some(FailOn::Warning)), DoctorSeverity::Warning);
        assert_eq!(floor(Some(FailOn::Error)), DoctorSeverity::Error);
    }
}
