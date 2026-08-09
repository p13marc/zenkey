//! `node list` — the liveliness roster (RFC 04 §5); `--verbose` joins each
//! producer against its served introspect slice.
//! `node info <origin>` — the per-node capability inventory (issue #49).

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
