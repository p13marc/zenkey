//! `zenctl watchdog` (#227) — transitions, not states.
//!
//! The continuous observer over the engine's closed condition vocabulary
//! (`zenkey_fleet::judge::condition`): every genuine state change is one ndjson
//! line on stdout, an unchanged tick prints nothing. A **foreground**
//! process, explicitly launched, one per invocation, no shared state — the
//! redesign ledger's "no daemon" decision rejected a hidden discovery-caching
//! server, not this (`docs/redesign-2026-07.md` §6.1).
//!
//! zk2's since #612 (FJ8b): the rule selectors are wire keys, watched on a
//! session in no namespace, and the rules that read the deployment —
//! `invalid-payload` and `qos-mismatch` through the lens, `instance-gone`
//! through presence, `doctor` — read it through a second session in its
//! namespace (`--namespace`, alias `--base`).
//!
//! Output is ndjson regardless of `--format`: a stream of transitions has
//! one honest encoding — a table redraw would be a state display, which is
//! exactly what this verb exists not to be.
//!
//! A run bounded by `--count` exits on how its rules *ended* (#511): 1 if any
//! is firing, else 2 if any is unobservable, else 0 — through
//! [`crate::exit::verdict`]. An unbounded run has no end to judge and exits 0.

use std::io::Write as _;

use anyhow::Result;
use zenkey_fleet::Sipper as _;
use zenkey_fleet::judge::condition::{Condition, WatchdogSpec};

use crate::bus::Deployment;
use crate::cmd::zk2;

/// Whether a rule reads the deployment — its doctor, an address's instance
/// tokens, a key's contract or declared QoS — and so needs a session in
/// its namespace beside the raw one.
pub(crate) fn reads_deployment(rule: &Condition) -> bool {
    matches!(
        rule,
        Condition::DoctorCheck { .. }
            | Condition::InstanceGone { .. }
            | Condition::InvalidPayload { .. }
            | Condition::QosMismatch { .. }
    )
}

pub async fn run(cli: crate::cli::WatchdogArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let contracts = zk2::load_contracts(&cli.contracts)?;
    let crate::cli::WatchdogArgs {
        rules,
        every,
        count,
        contracts: _,
        ns: _,
    } = cli;
    // `--every`/`--count`, not `--tick`/`--ticks` (#307): one period flag and
    // one stop-bound flag across the whole tool.
    let tick = super::positive_secs("--every", every)?;
    let rules: Vec<Condition> = rules
        .iter()
        .map(|r| Condition::parse(r))
        .collect::<std::result::Result<_, _>>()
        .map_err(anyhow::Error::from)?;

    let raw = dep.link().session().await?;
    // The deployment's own session, beside the raw one: opened for every run,
    // because the rules that read the deployment are judged through it, and
    // a run with none of them pays one idle session for a simpler shape.
    let bus = zenkey_fleet::DoctorBus {
        session: dep.session().await?,
        raw,
        namespace: dep.namespace().to_owned(),
    };
    let store = zk2::store(&dep, &contracts);
    let spec = WatchdogSpec {
        rules,
        tick,
        ticks: count,
        timeout: dep.timeout(),
    };

    eprintln!(
        "watchdog: {} rule(s), every {every}s — one ndjson line per genuine state \
         change, none per unchanged tick; three states, ok/firing/unobservable \
         (tooling guide O4/O6)",
        spec.rules.len()
    );
    let mut out = std::io::stdout();
    // The engine yields transitions and returns the summary (#397), so the
    // write happens where its error can be answered for (#360).
    let mut run = zenkey_fleet::watchdog(&bus, &store, &contracts, &spec).pin();
    let interrupted = loop {
        let next = tokio::select! {
            t = run.sip() => t,
            _ = tokio::signal::ctrl_c() => {
                eprintln!("watchdog: interrupted");
                break true;
            }
        };
        let Some(t) = next else { break false };
        // Tagged (`"row":"transition"`) like every non-sample line of an
        // explorer stream, so a consumer can select or skip them by kind.
        let written = writeln!(
            out,
            "{}",
            crate::render::Row::of("transition", &t).into_line()
        )
        .and_then(|()| out.flush());
        if let Err(e) = written {
            // A closed pipe is the consumer saying "enough"; anything else is
            // this verb's output not arriving, which is not a clean run.
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                return Ok(());
            }
            return Err(anyhow::Error::new(e).context("failed to write the transition stream"));
        }
    };
    if interrupted {
        return Ok(());
    }
    // Awaiting the run is what performs the acknowledged monitor teardown and
    // hands back the summary.
    let summary = run.await?;
    eprintln!(
        "watchdog: {} tick(s), {} transition(s)",
        summary.ticks, summary.transitions,
    );
    // A bounded run is a question with an answer (#511): how did its rules
    // end? Any firing is the 1, else any unobservable the 2, else 0 —
    // projected from the summary's own judgement. An unbounded run ends on
    // Ctrl-C or a closed stream, and keeps its 0.
    if count.is_none() {
        return Ok(());
    }
    if !summary.firing.is_empty() {
        eprintln!(
            "watchdog: {} rule(s) ended firing: {}",
            summary.firing.len(),
            summary.firing.join("; ")
        );
    }
    let judgement = summary.judgement();
    if let zenkey_fleet::Judgement::Unobservable { reason } = &judgement {
        eprintln!("watchdog: {reason} — exit 2, the reserved non-verdict");
    }
    crate::exit::verdict(&judgement)
}
