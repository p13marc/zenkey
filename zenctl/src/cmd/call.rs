//! `zenctl call` (#612, FJ5): one operation of a zk2 service, through its
//! contract — at one address, or fanned out over a selection.
//!
//! It replaced v1's `service call` (the `@rpc` plane). Everything that can
//! be refused is refused before anything is sent, and with `--contracts`
//! before a session opens: an address that is not one, an operation the
//! revision does not declare, a parameter its template does not have, a
//! fan-out to an operation that forbids one (spec §5.1 O2), a request that
//! does not encode as its type. Each is exit 2 (`crate::exit`).
//!
//! The call itself is the runtime's (`zenkey_fleet::call_operation` over
//! its `Client` and `Fleet`), and the exit code keeps v1's mapping: 0 a
//! value and no error reply, 1 an envelope (the finding), 2 silence —
//! never a verdict (O5).

use anyhow::Result;
use zenkey_fleet::{OperationCall, ResolvedTarget};
use zenkey_model::schema::TypeId;

use crate::bus::Deployment;
use crate::cmd::zk2;
use crate::exit::unaskable;

/// `call <address> <iface>[@fp] <operation> [request]`.
pub async fn run(cli: crate::cli::CallArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let contracts = zk2::load_contracts(&cli.contracts)?;
    let target = ResolvedTarget::parse(&cli.address)?;
    let values = zk2::bindings(&cli.params);
    // A request that cannot be read is this tool refusing its input, before
    // anything is asked of the bus.
    let request = cli
        .request
        .as_ref()
        .map(|src| {
            src.read()
                .map_err(|e| unaskable!("the request could not be read: {e:#}"))
        })
        .transpose()?;

    let mut session = None;
    let revision =
        zk2::revision_at(&dep, &contracts, &cli.target, Some(&target), &mut session).await?;
    let plan = zenkey_fleet::plan_call(&revision, target, &cli.operation, values)?;
    let input = request.unwrap_or_else(|| match plan.operation.request {
        TypeId::Raw { .. } => Vec::new(),
        _ => b"{}".to_vec(),
    });
    let encoded = zenkey_fleet::encode_request(&revision, &plan, &input)?;
    let session = match session {
        Some(s) => s,
        None => dep.session().await?,
    };
    // The caller's timeout, else the contract's recommendation, else the
    // tool's default (spec §5.1, "The timeout").
    let timeout = dep
        .chosen_timeout()
        .or(plan
            .operation
            .timeout_ms
            .map(std::time::Duration::from_millis))
        .unwrap_or_else(|| dep.timeout());
    let report = zenkey_fleet::call_operation(
        &session,
        OperationCall {
            revision: &revision,
            plan: &plan,
            request: encoded,
            timeout,
            retries: cli.retries,
        },
    )
    .await?;
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    match report.exit_code() {
        0 => Ok(()),
        code => std::process::exit(code),
    }
}
