//! `zenctl record` (issue #53): capture a selector's traffic to a `.zrec`
//! file through the Monitor — the same bounded broadcast every other
//! consumer runs on, so a bus that outruns the disk surfaces as drop
//! records *in the file* (RFC 09 §5.1 O6 applied to a capture, §5.2 for
//! the format). Progress rides stderr; the final counts are the report's
//! job, rendered by `output.rs`.

use std::io::BufWriter;

use anyhow::{Context, Result};
use zenkey_fleet::{RecordBounds, RecordReport, ZREC_VERSION, ZrecHeader, ZrecSink};

use crate::Bus;
use crate::cli::SelectorArgs;

pub async fn run(cli: crate::cli::RecordArgs) -> Result<()> {
    let bus = Bus::resolve(&cli.bus)?;
    let args = &bus;
    let crate::cli::RecordArgs {
        selector,
        out,
        for_secs,
        count,
        bus: _,
    } = cli;
    run_inner(&selector, &out, for_secs, count, args).await
}

async fn run_inner(
    sel: &SelectorArgs,
    out: &str,
    for_secs: Option<f64>,
    count: u64,
    args: &Bus,
) -> Result<()> {
    let selector = super::selector_of(sel, args)?;
    let duration = match for_secs {
        Some(secs) => Some(super::positive_secs("--for", secs)?),
        None => None,
    };
    let header = ZrecHeader {
        zrec: ZREC_VERSION,
        selectors: vec![selector.clone()],
        base: args.base().to_string(),
        captured_at: zenkey_fleet::rfc3339_now(),
    };
    // Both halves off the runtime (#332): the create through `tokio::fs`,
    // and every row after it on the blocking pool behind the sink's queue.
    // A capture that stalls its own drain records drops it caused itself.
    let file = tokio::fs::File::create(out)
        .await
        .with_context(|| format!("create {out}"))?
        .into_std()
        .await;
    let sink = ZrecSink::spawn(BufWriter::new(file), &header).await?;

    let session = args.session().await?;
    let monitor =
        zenkey_fleet::Monitor::start(&session, zenkey_fleet::MonitorSpec::default()).await?;
    let mut events = monitor.events();
    monitor.watch(&selector).await?;

    // A `**` selector never crosses an `@`-chunk: say what the capture
    // cannot contain up front, not after someone replays it (O5).
    eprintln!(
        "recording {selector} to {out}{} (ctrl-c to stop){}",
        match (for_secs, count) {
            (Some(d), 0) => format!(" for {d}s"),
            (None, n) if n > 0 => format!(" for {n} sample(s)"),
            (Some(d), n) => format!(" for {d}s or {n} sample(s)"),
            _ => String::new(),
        },
        if selector.contains("**") {
            " — `**` cannot cross `@`-planes; they are excluded, not empty"
        } else {
            ""
        }
    );

    let bounds = RecordBounds {
        max_samples: (count > 0).then_some(count),
        max_duration: duration,
    };
    let started = std::time::Instant::now();
    let mut last_line = std::time::Instant::now();
    let recording = zenkey_fleet::record(&mut events, &sink, bounds, |samples, dropped| {
        // Progress on stderr, throttled — completion and failure are the
        // report's job, not a progress line's.
        if last_line.elapsed() >= std::time::Duration::from_secs(1) {
            last_line = std::time::Instant::now();
            eprintln!("  {samples} sample(s), {dropped} dropped…");
        }
    });
    tokio::select! {
        r = recording => r?,
        _ = tokio::signal::ctrl_c() => {}
    }

    // `finish` drains the queue before it flushes and reports what actually
    // reached the file — not what the capture handed the queue.
    let (samples, dropped) = sink.finish().await?;
    let report = RecordReport {
        header,
        out: Some(out.to_string()),
        samples,
        dropped,
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    };
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())?;
    Ok(())
}
