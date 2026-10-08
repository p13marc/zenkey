//! `zenctl watch` (#612, FJ5): a zk2 subscription through a contract.
//!
//! Named `subscribe` here because `cmd/watch.rs` is the `--watch` flag's
//! re-render loop on the listing verbs (its ticks are the `watch` report
//! family), a different thing: that re-renders one report on an interval,
//! this streams samples as they arrive. The verb's rows are tagged
//! `sample`, `discarded`, `lagged` and `summary`, and no envelope leads
//! them — a stream has none (`crate::render`).
//!
//! The subscription is the runtime's (`Consumer::for_tool`, via
//! `zenkey_fleet::watch_resource`): it resolves at once, without presence
//! (spec §3.2 R1, R5), and discards a sample put on a wildcard key (R6),
//! which this verb counts and reports apart from what it lagged behind.
//! It ends at `--count` samples, after `--for` seconds, or on ctrl-c, and
//! exits 0 when a sample arrived, 2 when none did: silence is never a
//! verdict (O5).

use std::time::Instant;

use anyhow::Result;
use zenkey_fleet::ResolvedTarget;
use zenkey_fleet::report::{WatchEnd, WatchSummary};
use zenkey_model::authoring::Kind;

use crate::bus::Deployment;
use crate::cmd::zk2;
use crate::exit::unaskable;
use crate::render::{Mode, Sink};

/// `watch <address> <iface>[@fp] <resource> [--for S] [--count N]`.
pub async fn run(cli: crate::cli::WatchArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let contracts = zk2::load_contracts(&cli.contracts)?;
    let target = ResolvedTarget::parse(&cli.address)?;
    let values = zk2::bindings(&cli.params);
    let window = cli
        .for_secs
        .map(|s| super::positive_secs("--for", s))
        .transpose()?;
    if cli.count == Some(0) {
        return Err(unaskable!(
            "--count is a stop bound and must be at least 1; leave it out to run until \
             interrupted"
        ));
    }

    let mut session = None;
    let revision =
        zk2::revision_at(&dep, &contracts, &cli.target, Some(&target), &mut session).await?;
    let r = zenkey_fleet::resolve_resource(
        &revision,
        &cli.resource,
        &[Kind::Stream, Kind::State, Kind::Event],
    )?;
    zenkey_fleet::check_values(r, &values)?;
    let session = match session {
        Some(s) => s,
        None => dep.session().await?,
    };
    let mut watch = zenkey_fleet::watch_resource(&session, &revision, &target, r, &values).await?;

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
            "watching {} (ctrl-c to stop)",
            watch.selectors().join(" + ")
        );
    }

    let started = Instant::now();
    let deadline = window.map(|w| tokio::time::Instant::now() + w);
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(250));
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    let (mut received, mut discarded, mut lagged) = (0u64, 0u64, 0u64);
    let ended = loop {
        tokio::select! {
            next = watch.next() => {
                let Some(sample) = next else { break WatchEnd::Interrupted };
                received += 1;
                sink.row("sample", serde_json::to_value(&sample)?)?;
                for line in crate::render::sample_lines(&sample) {
                    sink.line(line)?;
                }
                sink.flush()?;
                if cli.count.is_some_and(|n| received >= n) {
                    break WatchEnd::Count;
                }
            }
            () = async {
                match deadline {
                    Some(d) => tokio::time::sleep_until(d).await,
                    None => std::future::pending().await,
                }
            } => break WatchEnd::Window,
            _ = &mut ctrl_c => break WatchEnd::Interrupted,
            _ = tick.tick() => {
                counts(&mut sink, &watch, &mut discarded, &mut lagged)?;
            }
        }
    };
    counts(&mut sink, &watch, &mut discarded, &mut lagged)?;
    let summary = WatchSummary {
        address: target.address.clone(),
        iface: revision.iface().to_string(),
        fingerprint: revision.fingerprint().to_string(),
        resource: format!("{}/{}", r.token, r.template),
        selectors: watch.selectors().to_vec(),
        received,
        discarded,
        lagged,
        elapsed_s: started.elapsed().as_secs_f64(),
        ended,
    };
    sink.row("summary", serde_json::to_value(&summary)?)?;
    sink.flush()?;
    if !mode.machine() {
        for line in crate::render::summary_lines(&summary) {
            eprintln!("{line}");
        }
    }
    if received == 0 {
        std::process::exit(crate::exit::NO_VERDICT);
    }
    Ok(())
}

/// Say when R6's discards or the lag moved: a row for a program, a line on
/// stderr for a person.
fn counts(
    sink: &mut Sink<'_>,
    watch: &zenkey_fleet::Watch,
    discarded: &mut u64,
    lagged: &mut u64,
) -> Result<()> {
    let (d, l) = (watch.discarded(), watch.lagged());
    if d > *discarded {
        sink.row(
            "discarded",
            serde_json::json!({ "discarded": d - *discarded }),
        )?;
        if !sink.machine() {
            eprintln!(
                "-- {} sample(s) put on a wildcard key, discarded by rule (R6) --",
                d - *discarded
            );
        }
        *discarded = d;
    }
    if l > *lagged {
        sink.row("lagged", serde_json::json!({ "lagged": l - *lagged }))?;
        if !sink.machine() {
            eprintln!(
                "-- {} sample(s) dropped: this tool fell behind --",
                l - *lagged
            );
        }
        *lagged = l;
    }
    Ok(())
}
