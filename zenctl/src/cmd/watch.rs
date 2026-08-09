//! `--watch` on list commands (#56): re-render on an interval (or on
//! liveliness events, for `node list`), diff-aware — appeared rows are
//! marked `+` for one cycle, disappeared rows linger one cycle marked `-`.
//! Deliberately **not a TUI** (§6.1: "the TUI is the GUI's job"): table mode
//! is a plain clear-and-redraw, and `--format ndjson` streams one
//! machine-readable snapshot object per cycle for scripting.

use std::collections::{BTreeMap, BTreeSet};
use std::io::IsTerminal;
use std::time::Duration;

use anyhow::{Result, bail};
use serde::Serialize;

use crate::output::Format;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Appeared,
    Disappeared,
}

/// Row-id → mark, for one render cycle.
pub type Marks = BTreeMap<String, Mark>;

/// What changed between two cycles, by stable row id.
pub fn diff_marks(prev: &BTreeSet<String>, cur: &BTreeSet<String>) -> Marks {
    let mut marks = Marks::new();
    for id in cur.difference(prev) {
        marks.insert(id.clone(), Mark::Appeared);
    }
    for id in prev.difference(cur) {
        marks.insert(id.clone(), Mark::Disappeared);
    }
    marks
}

/// `--watch` accepts table (redraw) and ndjson (stream); a single growing
/// JSON document would misrepresent a stream, so `--format json` is refused.
pub fn validate_format(format: Format) -> Result<()> {
    if matches!(format, Format::Json) {
        bail!(
            "--watch cannot stream --format json (one document cannot grow); \
             use --format ndjson: one snapshot object per line"
        );
    }
    Ok(())
}

/// Render one watch cycle.
///
/// `rows` are `(stable id, rendered line)` in display order; `marks` is this
/// cycle's diff (empty on the first cycle — everything "appearing" at start
/// is not a change).
pub fn render_cycle<R: Serialize>(
    report: &R,
    rows: &[(String, String)],
    marks: &Marks,
    format: Format,
    footer: &str,
) {
    match format.resolved() {
        Format::Ndjson => {
            let appeared: Vec<&str> = marks
                .iter()
                .filter(|(_, m)| **m == Mark::Appeared)
                .map(|(id, _)| id.as_str())
                .collect();
            let disappeared: Vec<&str> = marks
                .iter()
                .filter(|(_, m)| **m == Mark::Disappeared)
                .map(|(id, _)| id.as_str())
                .collect();
            let line = serde_json::json!({
                "snapshot": report,
                "appeared": appeared,
                "disappeared": disappeared,
            });
            println!("{line}");
        }
        _ => {
            // Plain clear-and-redraw on a tty; sequential snapshots when
            // table output is piped somewhere on purpose.
            if std::io::stdout().is_terminal() {
                print!("\x1b[2J\x1b[H");
            }
            for (id, line) in rows {
                let prefix = match marks.get(id) {
                    Some(Mark::Appeared) => "+",
                    _ => " ",
                };
                println!("{prefix} {line}");
            }
            for (id, mark) in marks {
                if *mark == Mark::Disappeared {
                    println!("- {id}  (disappeared)");
                }
            }
            println!("\n{footer}");
        }
    }
}

/// The polling driver: fetch → diff → render, until Ctrl-C. Commands whose
/// data the bus *pushes* (the liveliness roster) do not poll — `node list
/// --watch` is event-driven in `cmd::node` and only shares the render path.
pub async fn poll_loop<R: Serialize>(
    interval: Duration,
    format: Format,
    mut fetch: impl AsyncFnMut() -> Result<R>,
    rows: impl Fn(&R) -> Vec<(String, String)>,
) -> Result<()> {
    validate_format(format)?;
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut prev: Option<BTreeSet<String>> = None;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = ticker.tick() => {
                let report = fetch().await?;
                let rendered = rows(&report);
                let cur: BTreeSet<String> =
                    rendered.iter().map(|(id, _)| id.clone()).collect();
                // First cycle: no diff — a fresh watch "appearing" is not a
                // change in the fleet.
                let marks = match &prev {
                    Some(p) => diff_marks(p, &cur),
                    None => Marks::new(),
                };
                let footer = format!(
                    "{} row(s) — watching every {:.0}s, Ctrl-C to stop \
                     (+ appeared, - disappeared)",
                    rendered.len(),
                    interval.as_secs_f64()
                );
                render_cycle(&report, &rendered, &marks, format, &footer);
                prev = Some(cur);
            }
        }
    }
    Ok(())
}

/// `--watch SECS` → a validated interval.
pub fn interval_of(secs: f64) -> Result<Duration> {
    if !secs.is_finite() || secs < 0.2 {
        bail!("--watch interval must be at least 0.2s (got {secs})");
    }
    Ok(Duration::from_secs_f64(secs))
}

fn topic_rows(report: &crate::report::TopicList) -> Vec<(String, String)> {
    report
        .subjects
        .iter()
        .map(|s| {
            (
                format!("{}/{}/{}", s.producer, s.class, s.path),
                format!(
                    "{:<12} {:<10} {:<40} {}{}",
                    s.producer,
                    s.class,
                    s.path,
                    s.type_name,
                    if s.open_ended { "  [open-ended]" } else { "" }
                ),
            )
        })
        .collect()
}

pub async fn topic_list(
    secs: f64,
    producer: Option<&str>,
    class: Option<&str>,
    args: &crate::BusArgs,
) -> Result<()> {
    validate_format(args.format)?;
    let interval = interval_of(secs)?;
    let dirs = args.registry_dirs();
    if dirs.is_empty() {
        // The recurring sweep holds one declared registry querier across
        // cycles (#37) instead of re-declaring per poll.
        let session = args.session().await?;
        let repeating =
            zenkey_fleet::RepeatingRegistry::declare(&session, args.base(), args.timeout()).await?;
        let fetch = async || {
            let slices: Vec<zenkey::RegistrySlice> = repeating
                .fetch()
                .await?
                .into_iter()
                .map(|(s, _)| s)
                .collect();
            crate::offline::topic_list(&slices, producer, class)
        };
        poll_loop(interval, args.format, fetch, topic_rows).await?;
        repeating.undeclare().await
    } else {
        // Dirs given: the union set (bus wins, dirs fill) per cycle, same
        // sourcing as the one-shot command.
        let fetch = async || {
            let slices = args.slices().await?;
            crate::offline::topic_list(&slices, producer, class)
        };
        poll_loop(interval, args.format, fetch, topic_rows).await
    }
}

pub async fn storage_list(secs: f64, args: &crate::BusArgs) -> Result<()> {
    validate_format(args.format)?;
    let interval = interval_of(secs)?;
    let session = args.session().await?;
    let fetch = async || {
        let storages = zenkey_fleet::storages(&session, args.timeout()).await?;
        let coverage = match args.slice_set().await {
            Ok(slices) => zenkey_fleet::state_coverage(&slices, args.base(), &storages),
            Err(_) => Vec::new(),
        };
        Ok(crate::report::StorageList { storages, coverage })
    };
    let rows = |report: &crate::report::StorageList| {
        let mut rows: Vec<(String, String)> = report
            .storages
            .iter()
            .map(|s| {
                (
                    format!("storage:{}@{}", s.name, s.zid),
                    format!(
                        "storage   {:<16} @{}  {}",
                        s.name,
                        s.zid,
                        s.key_expr.as_deref().unwrap_or("-")
                    ),
                )
            })
            .collect();
        rows.extend(report.coverage.iter().map(|row| {
            use zenkey_fleet::Coverage;
            let detail = match &row.coverage {
                Coverage::Covered(s) => format!("covered by {s}"),
                Coverage::Partial(s) => format!("PARTIAL via {s}"),
                Coverage::Uncovered => "uncovered".to_string(),
            };
            (
                format!("coverage:{}/{}", row.producer, row.path),
                format!("coverage  {:<10} {:<36} {detail}", row.producer, row.path),
            )
        }));
        rows
    };
    poll_loop(interval, args.format, fetch, rows).await
}

pub async fn base_list(secs: f64, args: &crate::BusArgs) -> Result<()> {
    validate_format(args.format)?;
    let interval = interval_of(secs)?;
    let session = args.session().await?;
    let fetch = async || {
        let bases = zenkey_fleet::discover_bases(&session, args.timeout()).await?;
        Ok(crate::report::BaseList { bases })
    };
    let rows = |report: &crate::report::BaseList| {
        report
            .bases
            .iter()
            .map(|b| {
                let display = if b.base.is_empty() {
                    "(empty)"
                } else {
                    b.base.as_str()
                };
                (
                    b.base.clone(),
                    format!(
                        "{display:<24} {:>3} origin(s)  {:>3} producer(s)",
                        b.origins.len(),
                        b.producers.len()
                    ),
                )
            })
            .collect()
    };
    poll_loop(interval, args.format, fetch, rows).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_classify_transitions() {
        let prev: BTreeSet<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
        let cur: BTreeSet<String> = ["b", "c"].iter().map(|s| s.to_string()).collect();
        let marks = diff_marks(&prev, &cur);
        assert_eq!(marks.get("c"), Some(&Mark::Appeared));
        assert_eq!(marks.get("a"), Some(&Mark::Disappeared));
        assert_eq!(marks.get("b"), None, "unchanged rows carry no mark");
    }

    #[test]
    fn json_is_refused_ndjson_is_not() {
        assert!(validate_format(Format::Json).is_err());
        assert!(validate_format(Format::Ndjson).is_ok());
        assert!(validate_format(Format::Table).is_ok());
        assert!(validate_format(Format::Auto).is_ok());
    }
}
