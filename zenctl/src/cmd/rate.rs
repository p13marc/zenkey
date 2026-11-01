//! `zenctl rate` — watch a window, report rates (ros2-style), through the
//! typed [`RateReport`](zenkey_fleet::RateReport) so `--format json` applies
//! (issue #46) and the O6 eviction count is printed, never swallowed.
//!
//! One verb since #307. `topic hz` and `topic bw` were two spellings of one
//! observation: the same Monitor, the same window, the same eviction ledger,
//! differing only in which number the table led with. `--bytes` is that
//! choice, and it is a rendering flag, which is what it always was.
//!
//! zk2's since #612 (FJ8b): a raw verb — the wire selector on a session in
//! no namespace — whose keys are resolved through a lens on the
//! deployment's namespace and grouped by address and resource, the unit a
//! zk2 service publishes as; a key that is not this deployment's zk2 data
//! is counted in a group of its own, never dropped (O1). The projection is
//! the engine's ([`zenkey_fleet::model::stats::rate_report`]).

use anyhow::Result;
use zenkey_fleet::Lens;
use zenkey_fleet::model::stats::{RateAsk, rate_report};

use crate::bus::Deployment;
use crate::cmd::zk2;

pub async fn run(cli: crate::cli::RateArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let contracts = zk2::load_contracts(&cli.contracts)?;
    let catalogs = zk2::load_catalogs(&cli.contracts)?;
    let crate::cli::RateArgs {
        selector,
        for_secs,
        bytes: bandwidth,
        per_key,
        loss,
        face: crate::cli::AcrossFaceArgs { across_face },
        latency,
        contracts: _,
        ns: _,
    } = cli;
    // `--latency` implies `--per-key`: a latency figure is a per-key
    // statistic, and the aggregate table has no column to print it in.
    let per_key = per_key || latency;
    let namespace = dep.namespace().to_owned();
    let face = zk2::FaceScope::of(&across_face, &contracts, &namespace, zk2::Reads::Samples)?;
    let typed = selector.is_some();
    let selector = zk2::wire_selector(selector.as_deref(), &namespace)?;
    // Across a face, the scope the contracts give (core §8.5); the report
    // names it as one selector, its parts joined.
    let watched = match &face {
        Some(face) => face.narrow(std::slice::from_ref(&selector), typed)?,
        None => vec![selector],
    };
    let selector = watched.join(" + ");
    let window = super::positive_secs("--for", for_secs)?;

    // The lens before the window: presence and the revisions it names, but
    // across a face, where no descriptor crosses (observer §7.5).
    let store = zk2::store(&dep, &contracts);
    let catalog = match &face {
        Some(face) => {
            eprintln!("{}", face.note(&watched));
            None
        }
        None => {
            let ns_session = dep.session().await?;
            zk2::read_lens(&dep, &ns_session, &store).await
        }
    };

    let session = dep.link().session().await?;
    let monitor = zenkey_fleet::Monitor::start(
        &session,
        zenkey_fleet::MonitorSpec {
            selectors: watched,
            ..Default::default()
        },
    )
    .await?;

    eprintln!("measuring {selector} for {for_secs}s…");
    tokio::time::sleep(window).await;

    let held = store.held().len();
    let lens = Lens::new(&namespace, catalog.as_ref(), &store)
        .offline(&contracts)
        .catalogs(&catalogs)
        .held(held);
    let ask = RateAsk {
        selector: selector.clone(),
        window_s: for_secs,
        per_key,
        loss,
        latency,
    };
    let rep = monitor
        .core()
        .with_stats(|stats| rate_report(stats, &ask, &lens));
    monitor.stop();
    crate::render::emit_with(
        &mut std::io::stdout(),
        &crate::render::RateView {
            report: &rep,
            bandwidth,
        },
        dep.format(),
        dep.color(),
    )?;
    Ok(())
}
