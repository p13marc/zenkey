//! `zenctl call` (#612, FJ5): one operation of a zk2 service, through its
//! contract — at one address, or fanned out over a selection.
//!
//! It replaced v1's `service call` (the `@rpc` plane). Everything that can
//! be refused is refused before anything is sent, and with `--contracts`
//! before a session opens: an address that is not one, an operation the
//! revision does not declare, a parameter its template does not have, a
//! fan-out to an operation that forbids one (spec §5.1 O2), a request that
//! does not satisfy its JSON Schema type (§7.3, #671) or does not encode as
//! its type. Each is exit 2 (`crate::exit`).
//!
//! The call itself is the runtime's (`zenkey_fleet::call_operation` over
//! its `Client` and `Fleet`), and the exit code keeps v1's mapping: 0 a
//! value and no error reply, 1 an owner's error (the finding), 2 silence
//! — never a verdict (O5).
//!
//! The request travels in core §7.5's request form, written by the
//! runtime's client; a packed one is §7.6's record of the JSON given, its
//! `time` fields under the `--catalog`'s epoch, else 0, assumed, which the
//! report says (`request_epoch_assumed`).

use anyhow::Result;
use zenkey_fleet::{OperationCall, ResolvedTarget};
use zenkey_model::schema::TypeId;

use crate::bus::Deployment;
use crate::cmd::zk2;
use crate::exit::unaskable;

/// `call <address> <iface>[@fp] <operation> [request]`.
pub async fn run(cli: crate::cli::CallArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    // #990 V8: across a face's policy, the face's transport and timeout.
    let dep = zk2::across(
        dep,
        cli.face.face_policy.as_deref(),
        cli.face.face.as_deref(),
    )?;
    let contracts = zk2::load_contracts(&cli.contracts)?;
    if cli.face.across_face {
        zk2::held_across(&contracts, &cli.target)?;
    }
    // A packed request's `time` fields count from the deployment epoch
    // (core §7.6, §8.6): a `--catalog`'s, the deployment's word, else 0,
    // assumed and said so in the report.
    let epoch = zk2::load_catalogs(&cli.contracts)?.epoch(zenkey_fleet::CatalogStated::Unread);
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
    let encoded = zenkey_fleet::encode_request_in(&revision, &plan, &input, epoch)?;
    let session = match session {
        Some(s) => s,
        None => dep.session().await?,
    };
    // The caller's timeout, else the contract's recommendation, else the
    // tool's default (spec §5.1, "The timeout"); across a face's policy,
    // the face's `reply_timeout_ms` a floor under the last two (core §8.1).
    let timeout = match dep.chosen_timeout() {
        Some(t) => t,
        None => {
            let t = plan
                .operation
                .timeout_ms
                .map_or_else(|| dep.timeout(), std::time::Duration::from_millis);
            dep.far().map_or(t, |f| f.at_least(t))
        }
    };
    let report = zenkey_fleet::call_operation(
        &session,
        OperationCall {
            revision: &revision,
            plan: &plan,
            request: encoded,
            epoch,
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
