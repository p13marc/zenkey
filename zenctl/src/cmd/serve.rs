//! `zenctl serve` (#612, FJ8a) — one operation of an interface, served by a
//! mock owner, every call logged.
//!
//! The mock is a real zk2 service at the address you name (P3, spec §6),
//! brought up by the runtime (`zenkey_fleet::serve_operation`): the
//! operation answers every call with one fixed reply — JSON encoded as the
//! contract's response type, as `call` encodes a request — or refuses it
//! with the envelope `--refuse` names; every other operation of the
//! interface is answered too, never silent (O3). The log is half the
//! feature: each call as it was answered, its key, what it binds, the
//! request decoded through the bundle and the metadata the caller claims
//! (O7) — a "who is calling this operation" probe.
//!
//! It replaced v1's `serve <keyexpr> <reply>`, a queryable on any key with
//! one body run through the registry's encoder: on a zk2 key that is a
//! second answerer beside the owner (P3), and its encoding was v1's
//! registry. Like `gen`, it refuses an address an instance already runs at
//! unless `--i-know` ([`guard_address`]).

use std::time::Instant;

use anyhow::Result;
use zenkey_fleet::report::{ServeEnd, ServeSummary, ServedAnswer, ServedCall};
use zenkey_fleet::{MockAnswer, ServeSpec, Synth};
use zenkey_model::authoring::Kind;
use zenkey_model::contract::{Body, Replies};
use zenkey_model::descriptor::Cause;
use zenkey_model::grammar::Addr;

use crate::bus::Deployment;
use crate::cli::{CauseArg, RefuseCode};
use crate::cmd::zk2;
use crate::exit::unaskable;
use crate::render::{Mode, Sink};

/// The address guard `gen` and `serve` share: one presence read of the
/// address's instance tokens, refused when one is there unless `i_know`
/// (`zenkey_fleet::check_address`). A read that may be incomplete, or that
/// access control may have emptied, is said, not read as "free" (§8.1).
pub(crate) async fn guard_address(
    session: &zenoh::Session,
    address: &Addr,
    dep: &Deployment,
    i_know: bool,
) -> Result<()> {
    let presence = zenkey_fleet::address_presence(session, address, dep.timeout()).await?;
    zenkey_fleet::check_address(&presence, address, i_know)?;
    if !presence.instances.is_empty() {
        eprintln!(
            "--i-know: {address} is running already (instance {}); this mock starts beside it",
            presence.instances.join(", ")
        );
    } else if !presence.complete {
        eprintln!(
            "note: the presence read of {address} may be incomplete — an instance this reader \
             did not see may be running there"
        );
    }
    Ok(())
}

/// The refusal `--refuse` names (§5.2), in the engine's vocabulary:
/// `unavailable` with its cause, and a cause on no other code.
fn refusal(
    code: RefuseCode,
    message: Option<String>,
    cause: Option<CauseArg>,
) -> Result<MockAnswer> {
    let code = match code {
        RefuseCode::InvalidRequest => "invalid_request",
        RefuseCode::NotFound => "not_found",
        RefuseCode::Unavailable => "unavailable",
        RefuseCode::Forbidden => "forbidden",
        RefuseCode::Busy => "busy",
        RefuseCode::Internal => "internal",
        RefuseCode::App => "app",
    };
    let cause = cause.map(|c| match c {
        CauseArg::Build => Cause::Build,
        CauseArg::Config => Cause::Config,
        CauseArg::Capability => Cause::Capability,
    });
    Ok(MockAnswer::refusal(
        code,
        message.unwrap_or_else(|| "refused by zenctl serve".to_owned()),
        cause,
    )?)
}

pub async fn run(cli: crate::cli::ServeArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let crate::cli::ServeArgs {
        address,
        target,
        operation,
        reply,
        refuse,
        message,
        cause,
        binds,
        seed,
        count,
        for_secs,
        i_know,
        contracts,
        ns: _,
    } = cli;
    // Everything the command line can be refused for, before a session.
    let contracts = zk2::load_contracts(&contracts)?;
    let bindings = zk2::binds(&binds)?;
    let window = for_secs
        .map(|s| super::positive_secs("--for", s))
        .transpose()?;
    if count == Some(0) {
        return Err(unaskable!(
            "--count is a stop bound and must be at least 1; leave it out to run until \
             interrupted"
        ));
    }
    let refused = refuse
        .map(|code| refusal(code, message, cause))
        .transpose()?;
    let typed = reply
        .as_ref()
        .map(|src| {
            src.read()
                .map_err(|e| unaskable!("the reply could not be read: {e:#}"))
        })
        .transpose()?;

    let mut session = None;
    let revision = zk2::revision(&dep, &contracts, &target, &mut session).await?;
    let r = zenkey_fleet::resolve_resource(&revision, &operation, &[Kind::Operation])?;
    let name = format!("{}/{}", r.token, r.template);
    let answer = match refused {
        Some(refusal) => refusal,
        None => {
            let synth = Synth::new(seed);
            let synthesized = |member| {
                synth
                    .sample(&revision, r, member, 0)
                    .map(|s| s.bytes)
                    .map_err(|why| unaskable!("{name}: {why}"))
            };
            let bytes = match &typed {
                Some(input) => zenkey_fleet::encode_response(&revision, r, input)?,
                None => synthesized(zenkey_fleet::Member::Response)?,
            };
            let summary = match &r.body {
                Body::Operation(op) if op.replies == Replies::Many && op.summary.is_some() => {
                    Some(synthesized(zenkey_fleet::Member::Summary)?)
                }
                _ => None,
            };
            MockAnswer::Reply { bytes, summary }
        }
    };

    let session = match session {
        Some(s) => s,
        None => dep.session().await?,
    };
    guard_address(&session, &address, &dep, i_know).await?;
    let mut served = zenkey_fleet::serve_operation(
        &session,
        ServeSpec {
            address: address.clone(),
            revision: std::sync::Arc::clone(&revision),
            operation: name.clone(),
            answer,
            bindings,
            tool: "zenctl serve".into(),
        },
    )
    .await?;

    let mode = Mode::of(dep.format());
    let (mut out, mut err) = (std::io::stdout(), std::io::stderr());
    let mut sink = Sink::with_color(
        &mut out,
        &mut err,
        dep.format(),
        crate::render::term_width(),
        dep.color(),
    );
    if !mode.machine() {
        eprintln!(
            "serving {name} of {}@{} at {address} as instance {}, in {} (ctrl-c to stop)",
            revision.iface(),
            &revision.fingerprint().hex().as_str()[..12],
            served.instance(),
            zk2::namespace_phrase(dep.namespace())
        );
    }

    let started = Instant::now();
    let deadline = window.map(|w| tokio::time::Instant::now() + w);
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    let mut calls = 0u64;
    let ended = loop {
        tokio::select! {
            biased;
            _ = &mut ctrl_c => break ServeEnd::Interrupted,
            () = async {
                match deadline {
                    Some(d) => tokio::time::sleep_until(d).await,
                    None => std::future::pending().await,
                }
            } => break ServeEnd::Window,
            call = served.next() => {
                let Some(call) = call else { break ServeEnd::Interrupted };
                calls += 1;
                sink.row("call", serde_json::to_value(&call)?)?;
                if !sink.machine() {
                    sink.line(call_line(&call))?;
                }
                sink.flush()?;
                if count.is_some_and(|n| calls >= n) {
                    break ServeEnd::Count;
                }
            }
        }
    };
    let summary = ServeSummary {
        address: address.to_string(),
        instance: served.instance(),
        iface: revision.iface().to_string(),
        fingerprint: revision.fingerprint().to_string(),
        operation: name,
        calls,
        elapsed_s: started.elapsed().as_secs_f64(),
        ended,
    };
    served.close().await?;
    sink.row("summary", serde_json::to_value(&summary)?)?;
    sink.flush()?;
    if !mode.machine() {
        eprintln!(
            "{} call(s) served in {:.1}s ({})",
            summary.calls,
            summary.elapsed_s,
            match summary.ended {
                ServeEnd::Count => "--count reached",
                ServeEnd::Window => "--for elapsed",
                ServeEnd::Interrupted => "interrupted",
            }
        );
    }
    Ok(())
}

/// One served call, on one line, for a person.
fn call_line(c: &ServedCall) -> String {
    let answer = match &c.answer {
        ServedAnswer::Reply { summary: true } => "→ reply, then the summary".to_owned(),
        ServedAnswer::Reply { summary: false } => "→ reply".to_owned(),
        ServedAnswer::Refused { code } => format!("→ refused {code}"),
        ServedAnswer::Failed { error } => format!("→ failed ({error}): answered internal"),
    };
    let who = c
        .metadata
        .as_ref()
        .map(|m| {
            format!(
                "  (claims actor {}, request {})",
                m.actor.as_deref().unwrap_or("—"),
                m.request_id.as_deref().unwrap_or("—")
            )
        })
        .unwrap_or_default();
    format!(
        "[{}] {}{}\n  request {}\n  {answer}",
        c.n,
        c.key,
        who,
        crate::render::payload_text(&c.request)
    )
}
