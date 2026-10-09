//! `zenctl bench call` (#612, FJ8a) — the engine's benchmark of a zk2
//! operation, driven.
//!
//! Planned and refused exactly as `call` is — an address that is not one,
//! an operation the revision does not declare, a fan-out to an operation
//! that forbids one (O2), a request that does not encode — and then
//! refused once more if the operation is not `idempotent`, unless
//! `--i-know`: a benchmark repeats a call, and repeating a write is a
//! different act from measuring it (O4's reasoning). It replaced v1's
//! `bench rpc`, which timed an `@rpc` procedure per origin.

use std::time::Duration;

use anyhow::Result;
use zenkey_fleet::{BenchSpec, ResolvedTarget};
use zenkey_model::schema::TypeId;

use crate::bus::Deployment;
use crate::cmd::zk2;
use crate::exit::unaskable;

/// A ceiling on `--calls`, so a typo is a bounded mistake. Not a policy about
/// how much load a deployment can take — the operator knows that, and
/// `--calls` lifts it.
const DEFAULT_COUNT: usize = 100;

pub async fn call(cli: crate::cli::BenchCallArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let crate::cli::BenchCallArgs {
        address,
        target: spec,
        operation,
        request,
        params,
        calls,
        concurrency,
        i_know,
        contracts,
        ns: _,
    } = cli;
    let contracts = zk2::load_contracts(&contracts)?;
    let target = ResolvedTarget::parse(&address)?;
    let values = zk2::bindings(&params);
    let request = request
        .as_ref()
        .map(|src| {
            src.read()
                .map_err(|e| unaskable!("the request could not be read: {e:#}"))
        })
        .transpose()?;

    let mut session = None;
    let revision = zk2::revision_at(&dep, &contracts, &spec, Some(&target), &mut session).await?;
    let plan = zenkey_fleet::plan_call(&revision, target, &operation, values)?;
    let input = request.unwrap_or_else(|| match plan.operation.request {
        TypeId::Raw { .. } => Vec::new(),
        _ => b"{}".to_vec(),
    });
    let encoded = zenkey_fleet::encode_request(&revision, &plan, &input)?;
    let calls = calls.unwrap_or(DEFAULT_COUNT);
    // Refused before anything is asked of the bus (and asked again by the
    // engine): a repeated write, or a bench of nothing.
    zenkey_fleet::check_bench(&revision, &plan, calls, i_know)?;
    let session = match session {
        Some(s) => s,
        None => dep.session().await?,
    };
    // The caller's timeout, else the contract's recommendation, else the
    // tool's default (spec §5.1, "The timeout"), as `call` waits.
    let timeout = dep
        .chosen_timeout()
        .or(plan.operation.timeout_ms.map(Duration::from_millis))
        .unwrap_or_else(|| dep.timeout());
    // The note is this verb's own preamble, not the dispatcher's (#354).
    eprintln!("{}", note(timeout));
    let report = zenkey_fleet::run_bench(
        &session,
        BenchSpec {
            revision: &revision,
            plan: &plan,
            request: encoded,
            calls,
            concurrency,
            timeout,
            force: i_know,
        },
    )
    .await?;
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    match report.exit_code() {
        0 => Ok(()),
        code => std::process::exit(code),
    }
}

/// The per-call timeout, said up front: a benchmark whose timeout is
/// shorter than the operation's p99 measures the timeout instead.
pub fn note(timeout: Duration) -> String {
    format!(
        "each call waits {:.1}s for its replies — a p99 at or near that number is measuring \
         the timeout, not the operation",
        timeout.as_secs_f64()
    )
}
