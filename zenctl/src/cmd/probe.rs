//! `zenctl check probe` (#59; zk2's since #612, FJ8b): the consumer-shaped
//! acceptance probe.
//!
//! > A probe MUST build its keys the way the product builds them.
//!
//! v1's probe called one origin's procedure on a concrete key. A zk2
//! consumer binds a role to providers and reads through the runtime's
//! consumer (spec §3.2), so this probe reads one resource exactly that way —
//! `Consumer::for_tool` with the address as its binding and the template
//! values as R2 bindings, a state resource's current state from its owner
//! first (S4) — and judges whether values arrive within the window
//! ([`zenkey_fleet::run_probe`]). Silence is attributed through presence,
//! never read as an answer: a provider that holds its token and sent
//! nothing, and no token visible to this reader, are both the finding; a
//! presence read that timed out is the reserved 2.

use anyhow::Result;
use zenkey_fleet::{ExpectAim, ResolvedTarget};
use zenkey_model::authoring::Kind;

use crate::bus::Deployment;
use crate::cmd::zk2;

/// The verdict verb's name, spelled once (#355) — the dispatcher
/// uses it too.
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("check probe");

pub async fn run(cli: crate::cli::CheckProbeArgs) -> Result<()> {
    let dep = ASKING.ask(Deployment::resolve(&cli.ns));
    let contracts = ASKING.ask(zk2::load_contracts(&cli.contracts));
    let crate::cli::CheckProbeArgs {
        address,
        target: spec,
        resource,
        params,
        for_secs,
        contracts: _,
        ns: _,
    } = cli;
    let window = ASKING.ask(super::positive_secs("--for", for_secs));
    let target = ASKING.ask(ResolvedTarget::parse(&address));
    let values = zk2::bindings(&params);
    let mut session = None;
    let revision =
        ASKING.ask(zk2::revision_at(&dep, &contracts, &spec, Some(&target), &mut session).await);
    let r = ASKING.ask(zenkey_fleet::resolve_resource(
        &revision,
        &resource,
        &[Kind::Stream, Kind::State, Kind::Event],
    ));
    ASKING.ask(zenkey_fleet::check_values(r, &values));
    let session = match session {
        Some(s) => s,
        None => ASKING.ask(dep.session().await),
    };
    eprintln!(
        "{}: reading {} {} {} as a consumer for up to {for_secs}s",
        ASKING.verb(),
        target.address,
        revision.iface(),
        format_args!("{}/{}", r.token, r.template)
    );
    let report = match zenkey_fleet::run_probe(
        &session,
        ExpectAim {
            revision: &revision,
            target: &target,
            resource: Some(r),
            values: &values,
        },
        window,
        dep.timeout(),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => ASKING.unobservable(format_args!("the probe could not stand up: {e}")),
    };
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    crate::exit::verdict(&report.verdict)
}
