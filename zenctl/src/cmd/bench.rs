//! `zenctl bench rpc` (issue #52) — the engine's benchmark, driven.
//!
//! The safe default target is `introspect`: RFC 08 §6 makes it a read that
//! every producer serves, and the registry declares it idempotent, so the one
//! call this tool can bench without asking permission is the one it already
//! fans out on by design. Everything else has to pass the registry guard, or
//! say `--i-know`.

use std::time::Duration;

use anyhow::{Result, anyhow};

use crate::Bus;

/// A ceiling on `--count`, so a typo is a bounded mistake. Not a policy about
/// how much load a fleet can take — the operator knows that and `--i-know`
/// does not lift this; `--count` explicitly can.
const DEFAULT_COUNT: usize = 100;

#[allow(clippy::too_many_arguments)]
pub async fn rpc(
    origin: &str,
    producer: &str,
    procedure: &str,
    calls: Option<usize>,
    concurrency: usize,
    i_know: bool,
    args: &Bus,
) -> Result<()> {
    let target = zenkey_fleet::CallTarget::parse(origin)?;
    let slices = args.slices_optional().await?;
    let session = args.session().await?;
    let count = calls.unwrap_or(DEFAULT_COUNT);

    let report = zenkey_fleet::run_bench(
        &args.fleet(&session),
        zenkey_fleet::BenchSpec {
            target: &target,
            producer,
            procedure,
            count,
            concurrency,
            timeout: args.timeout(),
            force: i_know,
        },
        slices.as_ref(),
    )
    .await
    .map_err(|e| anyhow!("{e}"))?;
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())?;
    // A benchmark that reached nobody is not a benchmark. Exit 2 matches
    // `service call`'s "zero replies" code — silence keeps its own meaning
    // (`crate::exit`).
    if report.origins.is_empty() {
        std::process::exit(crate::exit::NO_VERDICT);
    }
    // An error reply is a finding, and a benchmark that measured nothing but
    // error envelopes used to exit **0** with a latency distribution over
    // failures (#264). The numbers are still printed — they are what makes
    // the finding legible — and the exit says what they are made of.
    if report.errors > 0 {
        eprintln!(
            "bench: {} of {} completed call(s) came back as an error envelope \
             (RFC 05 §3) — the latencies above are timings of failures",
            report.errors, report.completed
        );
        std::process::exit(crate::exit::FINDING);
    }
    Ok(())
}

/// The default per-call timeout is the bus timeout; documented here because a
/// benchmark whose timeout is shorter than the fleet's p99 measures the
/// timeout instead.
pub fn note(timeout: Duration) -> String {
    format!(
        "each call times out after {:.1}s — a p99 at or near that number is measuring the \
         timeout, not the fleet",
        timeout.as_secs_f64()
    )
}
