//! `zenctl record` (issue #53): capture a selector's traffic to a `.zrec`
//! file through the Monitor — the same bounded broadcast every other
//! consumer runs on, so a bus that outruns the disk surfaces as drop
//! records *in the file* (RFC 09 §5.1 O6 applied to a capture, §5.2 for
//! the format). Progress rides stderr; the final counts are the report's
//! job, rendered by `output.rs`.

use std::io::BufWriter;

use anyhow::{Context, Result};
use zenkey_fleet::{RecordBounds, RecordReport, ZREC_VERSION, ZrecHeader, ZrecWriter};

use crate::BusArgs;

#[allow(clippy::too_many_arguments)] // clap surface, mirrored from echo
pub async fn run(
    selector: Option<&str>,
    origin: Option<&str>,
    class: Option<&str>,
    producer: Option<&str>,
    out: &str,
    duration: Option<u64>,
    count: u64,
    args: &BusArgs,
) -> Result<()> {
    let selector = match selector {
        Some(s) => s.to_string(),
        None => super::compose_selector(args, origin, class, producer)?,
    };
    let header = ZrecHeader {
        zrec: ZREC_VERSION,
        selectors: vec![selector.clone()],
        base: args.base().to_string(),
        captured_at: zenkey_fleet::record::rfc3339_now(),
    };
    let file = std::fs::File::create(out).with_context(|| format!("create {out}"))?;
    let mut writer = ZrecWriter::new(BufWriter::new(file), &header)?;

    let session = args.session().await?;
    let monitor =
        zenkey_fleet::Monitor::start(&session, zenkey_fleet::MonitorSpec::default()).await?;
    let mut events = monitor.events();
    monitor.watch(&selector).await?;

    // A `**` selector never crosses an `@`-chunk: say what the capture
    // cannot contain up front, not after someone replays it (O5).
    eprintln!(
        "recording {selector} to {out}{} (ctrl-c to stop){}",
        match (duration, count) {
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
        max_duration: duration.map(std::time::Duration::from_secs),
    };
    let started = std::time::Instant::now();
    let mut last_line = std::time::Instant::now();
    let recording = zenkey_fleet::record(&mut events, &mut writer, bounds, |samples, dropped| {
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

    let (samples, dropped) = writer.counts();
    writer.finish()?;
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
