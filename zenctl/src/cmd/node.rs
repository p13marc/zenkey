//! `node list` — the liveliness roster (RFC 04 §5); `--verbose` joins each
//! producer against its served introspect slice.
//! `node info <origin>` — the per-node capability inventory (issue #49).

use std::time::Duration;

use anyhow::{Result, anyhow};
use zenkey_fleet as bus;

use crate::{BusArgs, output, report};

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
                         {} procedure(s){}{}",
                        p.name,
                        p.subjects,
                        p.procedures,
                        if p.blob_tiers.is_empty() {
                            String::new()
                        } else {
                            format!(" · blob: {}", p.blob_tiers.join(","))
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
    output::node_list(&node_rows(&roster, slices.as_ref()), args.format)
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
    let base = args.base().to_string();
    let liveliness = vec![
        args.wire(zenkey::selector::all_liveliness(
            zenkey::selector::Scope::fleet(),
        ))?,
        args.wire(zenkey::selector::service_alive(
            &zenkey::ServiceOrigin::catalog(),
        ))?,
    ];
    // Liveliness only — zero data-plane subscribers (the lazy contract).
    let monitor = bus::Monitor::start(
        &session,
        bus::MonitorSpec {
            selectors: vec![],
            liveliness,
            ..Default::default()
        },
    )
    .await?;
    let mut events = monitor.events();

    let mut slices = if verbose {
        Some(args.slice_set().await?)
    } else {
        None
    };
    // Seeded below by the one-shot liveliness GET (see the comment there).
    let mut roster: BTreeMap<String, Vec<String>>;
    let mut prev: Option<BTreeSet<String>> = None;

    let render = |roster: &BTreeMap<String, Vec<String>>,
                  slices: Option<&zenkey_fleet::SliceSet>,
                  prev: &mut Option<BTreeSet<String>>| {
        let report = node_rows(roster, slices);
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

    let apply = |roster: &mut BTreeMap<String, Vec<String>>, key: &str, up: bool| {
        let Some(parsed) = zenkey::grammar::parse_full(&base, key) else {
            return false;
        };
        let origin = parsed.origin.chunk().to_string();
        let producer = parsed
            .producer
            .as_ref()
            .map(|p| p.chunk())
            .unwrap_or_else(|| origin.trim_start_matches('@').to_string());
        if up {
            let entry = roster.entry(origin).or_default();
            if entry.contains(&producer) {
                return false;
            }
            entry.push(producer);
            entry.sort();
            true
        } else {
            let Some(entry) = roster.get_mut(&origin) else {
                return false;
            };
            let before = entry.len();
            entry.retain(|p| p != &producer);
            let changed = entry.len() != before;
            if entry.is_empty() {
                roster.remove(&origin);
            }
            changed
        }
    };

    // Seed the initial roster with the one-shot liveliness GET: the
    // monitor's history-backed events can land in the broadcast before this
    // task subscribes to it, so history alone would race. Duplicates from a
    // late history event are absorbed by `apply`'s dedup.
    roster = bus::roster(&session, args.base(), args.timeout()).await?;
    // First frame, even when empty: "0 producers" is a statement, not
    // silence (RFC 05 §3.1).
    render(&roster, slices.as_ref(), &mut prev);

    loop {
        let item = tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            item = events.recv() => item,
        };
        let mut changed = false;
        let mut refreshed_join = false;
        let mut pending = item;
        // Coalesce a burst of events into one render.
        loop {
            match pending {
                None => return Ok(()),
                Some(bus::StreamItem::Dropped(_)) => {}
                Some(bus::StreamItem::Event(ev)) => match ev {
                    bus::FleetEvent::NodeUp(key) => {
                        if apply(&mut roster, &key, true) {
                            changed = true;
                            // A new producer may serve a new slice; refresh
                            // the join on NodeUp only (never per render).
                            if verbose && !refreshed_join {
                                slices = Some(args.slice_set().await?);
                                refreshed_join = true;
                            }
                        }
                    }
                    bus::FleetEvent::NodeDown(key) => {
                        changed |= apply(&mut roster, &key, false);
                    }
                    _ => {}
                },
            }
            match tokio::time::timeout(Duration::from_millis(50), events.recv()).await {
                Ok(next) => pending = next,
                Err(_) => break,
            }
        }
        if !changed {
            continue;
        }
        render(&roster, slices.as_ref(), &mut prev);
    }
    monitor.stop();
    Ok(())
}

/// Roster → typed rows, joining the slice facts when given (`--verbose`).
/// Absent slice = `None` fields, never a default (O4).
pub fn node_rows(
    roster: &std::collections::BTreeMap<String, Vec<String>>,
    slices: Option<&zenkey_fleet::SliceSet>,
) -> report::NodeList {
    let mut nodes = Vec::new();
    for (origin, producers) in roster {
        for producer in producers {
            let joined = slices.and_then(|s| {
                // Instance suffixes share the base slice (RFC 03 §1.5).
                let base_name = zenkey::grammar::Producer::parse_chunk(producer)
                    .map(|pr| pr.name().to_string())
                    .unwrap_or_else(|_| producer.clone());
                s.get(&base_name)
            });
            nodes.push(report::NodeRow {
                origin: origin.clone(),
                producer: producer.clone(),
                app: joined.map(|s| s.app.clone()),
                registry_version: joined.map(|s| s.version.clone()),
            });
        }
    }
    report::NodeList {
        nodes,
        slices_joined: slices.is_some(),
    }
}
