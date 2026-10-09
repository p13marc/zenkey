//! `zenctl check expect` (#160; zk2's since #612, FJ8b) — the CI assertion
//! verb over the engine's [`zenkey_fleet::run_expect`], written over a zk2
//! address and resource: their rates, their payloads against the contract,
//! their QoS against the resource's, and whether the address is present.
//! All judgement lives engine-side; this resolves the revision and the
//! resource, opens a session in the deployment's namespace, and hands the
//! three-state verdict to [`crate::exit::verdict`], which is the only place
//! 0/1/2 is spelled.

use anyhow::Result;
use zenkey_fleet::{ExpectAim, ResolvedTarget};
use zenkey_model::authoring::Kind;

use crate::bus::Deployment;
use crate::cmd::zk2;

/// This verb's name, spelled once (#355) — the dispatcher uses it too.
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("check expect");

pub async fn run(cli: crate::cli::CheckExpectArgs) -> Result<()> {
    // A verdict verb: a failure before the question is asked is the reserved
    // exit 2, never 1 (`exit::asked`'s rule).
    let dep = ASKING.ask(Deployment::resolve(&cli.ns));
    let contracts = ASKING.ask(zk2::load_contracts(&cli.contracts));
    let crate::cli::CheckExpectArgs {
        address,
        target: spec_target,
        resource,
        params,
        for_secs,
        at_least,
        rate_min,
        rate_max,
        valid_payload,
        qos,
        absent,
        present,
        contracts: _,
        ns: _,
    } = cli;
    let within = ASKING.ask(super::positive_secs("--for", for_secs));
    let target = ASKING.ask(ResolvedTarget::parse(&address).map_err(anyhow::Error::from));
    let values = zk2::bindings(&params);
    if resource.is_none() && !present {
        ASKING.ask(Err::<(), _>(crate::exit::unaskable!(
            "name a resource to expect samples of, or ask --present alone"
        )));
    }

    let mut session = None;
    let revision = ASKING
        .ask(zk2::revision_at(&dep, &contracts, &spec_target, Some(&target), &mut session).await);
    let r = resource.as_ref().map(|name| {
        ASKING.ask(
            zenkey_fleet::resolve_resource(
                &revision,
                name,
                &[Kind::Stream, Kind::State, Kind::Event],
            )
            .map_err(anyhow::Error::from),
        )
    });
    if let Some(r) = r {
        ASKING.ask(zenkey_fleet::check_values(r, &values).map_err(anyhow::Error::from));
    }
    // A session that will not open is the impaired exit (`asked`'s 2), never
    // 1 — "not met" is a claim about a window that was actually watched.
    let session = match session {
        Some(s) => s,
        None => ASKING.ask(dep.session().await),
    };
    let spec = zenkey_fleet::ExpectSpec {
        within,
        count: at_least,
        rate_min,
        rate_max,
        valid_payload,
        qos_declared: qos.is_some(),
        absent,
        present,
    };

    eprintln!(
        "{}: watching {} {} {} for {for_secs}s — subscriber declared before the window \
         opened (O4)",
        ASKING.verb(),
        target.address,
        revision.iface(),
        resource.as_deref().unwrap_or("(presence)"),
    );
    let report = match zenkey_fleet::run_expect(
        &session,
        ExpectAim {
            revision: &revision,
            target: &target,
            resource: r,
            values: &values,
        },
        &spec,
    )
    .await
    {
        Ok(r) => r,
        // The observation never stood up — that is the impaired exit,
        // never "not met".
        Err(e) => ASKING.unobservable(format_args!("observation could not be established: {e}")),
    };
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    crate::exit::verdict(&report.verdict.to_judgement())
}
