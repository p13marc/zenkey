//! `node list` — the liveliness roster (RFC 04 §5); `--verbose` joins each
//! producer against its served introspect slice.
//! `node info <origin>` — the per-node capability inventory (issue #49).

use anyhow::{Result, anyhow};
use zenkey_fleet as bus;

use crate::{BusArgs, output};

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

    match args.format.resolved() {
        output::Format::Json | output::Format::Ndjson => {
            println!("{}", serde_json::to_string_pretty(&info)?);
        }
        _ => {
            println!("origin    {}", info.origin);
            if info.producers.is_empty() {
                println!(
                    "  no liveliness token and no introspect reply — not on the \
                     roster (which is not proof it does not exist; RFC 05 §3.1)"
                );
            }
            for p in &info.producers {
                let alive = if p.alive { "alive" } else { "no token" };
                match (&p.app, &p.registry_version) {
                    (Some(app), Some(v)) => println!(
                        "  {}  [{alive}]  app {app} · registry v{v} · {} subject(s) · \
                         {} procedure(s){}{}{}",
                        p.name,
                        p.subjects,
                        p.procedures,
                        if p.blob_tiers.is_empty() {
                            String::new()
                        } else {
                            format!(" · blob: {}", p.blob_tiers.join(","))
                        },
                        if p.media.is_empty() {
                            String::new()
                        } else {
                            format!(
                                " · media: {}",
                                p.media
                                    .iter()
                                    .map(|m| format!("{} ({})", m.path, m.encoding))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        },
                        if p.deprecated_served > 0 {
                            format!(" · {} DEPRECATED still served", p.deprecated_served)
                        } else {
                            String::new()
                        },
                    ),
                    _ => println!(
                        "  {}  [{alive}]  (no introspect reply — capabilities unknown, \
                         not absent)",
                        p.name
                    ),
                }
            }
            if !info.freshness.is_empty() {
                println!("state freshness (declared ttl_s):");
                for f in &info.freshness {
                    let age = f
                        .age_s
                        .map(|a| format!("{a}s old"))
                        .unwrap_or_else(|| "no sample seen".into());
                    println!(
                        "  {}/{}  {}  (ttl {}s){}",
                        f.producer,
                        f.path,
                        age,
                        f.ttl_s,
                        if f.stale { "  STALE" } else { "" }
                    );
                }
            }
        }
    }
    Ok(())
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
    output::node_list(&bus::node_rows(&roster, slices.as_ref()), args.format)
}

/// `node list --watch` — event-driven, not polled: the liveliness roster is
/// pushed by the bus, so the monitor subscribes (history-backed tokens seed
/// the initial view) and every NodeUp/NodeDown re-renders. A producer
/// stopping is reflected within one liveliness event (#56 acceptance).
pub async fn watch(verbose: bool, args: &BusArgs) -> Result<()> {
    use std::collections::{BTreeMap, BTreeSet};

    use crate::cmd::watch::{Marks, diff_marks, render_cycle, validate_format};

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
    let mut prev: Option<BTreeSet<String>> = None;

    let render = |roster: &BTreeMap<String, Vec<String>>,
                  slices: Option<&zenkey_fleet::SliceSet>,
                  prev: &mut Option<BTreeSet<String>>| {
        let report = bus::node_rows(roster, slices);
        let rows: Vec<(String, String)> = report
            .nodes
            .iter()
            .map(|r| {
                (
                    format!("{}/{}", r.origin, r.producer),
                    match (&r.app, &r.registry_version) {
                        (Some(app), Some(v)) => {
                            format!("{:<16} {}  (app {app}, registry {v})", r.origin, r.producer)
                        }
                        _ if report.slices_joined => {
                            format!("{:<16} {}  (no served slice)", r.origin, r.producer)
                        }
                        _ => format!("{:<16} {}", r.origin, r.producer),
                    },
                )
            })
            .collect();
        let cur: BTreeSet<String> = rows.iter().map(|(id, _)| id.clone()).collect();
        let marks = match &prev {
            Some(p) => diff_marks(p, &cur),
            None => Marks::new(),
        };
        let footer = format!(
            "{} producer(s) — watching liveliness events, Ctrl-C to stop \
             (+ appeared, - disappeared)",
            rows.len()
        );
        render_cycle(&report, &rows, &marks, args.format, &footer);
        *prev = Some(cur);
    };

    // First frame, even when empty: "0 producers" is a statement, not silence
    // (RFC 05 §3.1).
    render(watch.roster(), slices.as_ref(), &mut prev);

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
        render(watch.roster(), slices.as_ref(), &mut prev);
    }
    watch.stop().await;
    Ok(())
}
