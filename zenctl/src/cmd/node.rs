//! `node list` — the liveliness roster (RFC 04 §5); `--verbose` joins each
//! producer against its served introspect slice.
//! `node info <origin>` — the per-node capability inventory (issue #49).

use anyhow::{Result, anyhow};
use zenkey_fleet as bus;

use crate::BusArgs;

pub async fn info(origin: &str, args: &BusArgs) -> Result<()> {
    // The identity bridge, enforced: an origin id or nothing (RFC 06 §6).
    if !zenkey::grammar::is_valid_host_origin(origin) && !origin.starts_with('@') {
        return Err(anyhow!(
            "{origin:?} is not an origin id — a hostname must be resolved to its \
             origin first (RFC 06 §6); `zenctl node list` shows the roster"
        ));
    }
    let session = args.session().await?;
    let info = zenkey_fleet::node_info(&session, args.base(), origin, args.timeout(), true).await?;

    crate::render::emit(&mut std::io::stdout(), &info, args.format)
}

pub async fn list(verbose: bool, args: &BusArgs) -> Result<()> {
    let session = args.session().await?;
    let roster = bus::roster(&session, args.base(), args.timeout()).await?;

    if roster.is_empty() {
        eprintln!(
            "nothing held a liveliness token on {} — check --connect (and that \
             producers are actually running).",
            zenkey::selector::all_liveliness(zenkey::selector::Scope::fleet())
        );
    }
    let slices = if verbose {
        Some(args.slice_set().await?)
    } else {
        None
    };
    crate::render::emit(
        &mut std::io::stdout(),
        &bus::node_rows(&roster, slices.as_ref()),
        args.format,
    )
}

/// `node list --watch` — event-driven, not polled: the liveliness roster is
/// pushed by the bus, so the monitor subscribes (history-backed tokens seed
/// the initial view) and every NodeUp/NodeDown re-renders. A producer
/// stopping is reflected within one liveliness event (#56 acceptance).
pub async fn watch(verbose: bool, args: &BusArgs) -> Result<()> {
    use std::collections::BTreeMap;

    use crate::cmd::watch::{render_cycle, validate_format};

    validate_format(args.format)?;
    let session = args.session().await?;
    // The roster is pushed by the bus, so this does not poll. The subscribe/
    // seed/coalesce loop lives in the engine (#207) — both explorers were
    // running their own copy of it, and zenctl's had drifted into a second
    // copy of the polling driver's cycle body besides.
    let mut watch = bus::RosterWatch::start(&session, args.base(), args.timeout()).await?;

    let mut slices = if verbose {
        Some(args.slice_set().await?)
    } else {
        None
    };
    let mut prev: Vec<String> = Vec::new();
    let mut tick = 0u64;

    // The report renders itself (#199): this used to build a second copy of
    // `node list`'s table here, with its own column widths.
    let render = |roster: &BTreeMap<String, Vec<String>>,
                  slices: Option<&zenkey_fleet::SliceSet>,
                  prev: &mut Vec<String>,
                  tick: &mut u64|
     -> Result<()> {
        let report = bus::node_rows(roster, slices);
        let footer = format!(
            "{} producer(s) — watching liveliness events, Ctrl-C to stop \
             (+ appeared, - disappeared)",
            report.nodes.len()
        );
        render_cycle(&report, *tick, prev, args.format, &footer)?;
        *tick += 1;
        Ok(())
    };

    // First frame, even when empty: "0 producers" is a statement, not silence
    // (RFC 05 §3.1).
    render(watch.roster(), slices.as_ref(), &mut prev, &mut tick)?;

    loop {
        let change = tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            change = watch.next_change() => change,
        };
        let Some(change) = change else { break };
        // A new producer may serve a slice nothing has read yet — refreshed on
        // the way up only, and once per coalesced burst rather than per event.
        if verbose && change.node_up {
            slices = Some(args.slice_set().await?);
        }
        render(watch.roster(), slices.as_ref(), &mut prev, &mut tick)?;
    }
    watch.stop().await;
    Ok(())
}
