//! `zenctl timeline` (#216): a window of the fleet as one merged ordering,
//! lanes per origin/producer, the clock stated per report — live through
//! the Monitor, or from a `.zrec` through the same projection.
//!
//! Two sources, one shape. The live drain and the file reader both hand
//! [`zenkey_fleet::timeline`] the same `Window`: rows (`TimelineRow` from a
//! `SampleView`, `Ingested::from_zrec` from a line) and breaks at the row
//! count they interrupted (`StreamItem::Dropped` and a `.zrec` drop record
//! are one `Break`). Nothing here decides an order — the engine does, and
//! `tests/timeline_identity.rs` there is what makes "the same window from
//! the file" a claim.
//!
//! The epoch is the instant *after* the last watch is declared, so `t_us`
//! is measured from the moment the observer could have seen anything; a
//! sample that arrives between two declarations saturates to 0, exactly as
//! `ZrecWriter` would have written it.

use std::io::BufReader;
use std::time::Instant;

use anyhow::{Context, Result};
use zenkey_fleet::report::TimelineSource;
use zenkey_fleet::{
    Break, FleetEvent, Ingested, Order, PlacedBreak, StreamItem, TimelineRow, Window, ZrecReader,
    timeline,
};

use crate::Bus;
use crate::cli::OrderArg;

pub async fn run(cli: crate::cli::TimelineArgs) -> Result<()> {
    let bus = Bus::resolve(&cli.bus)?;
    let args = &bus;
    let crate::cli::TimelineArgs {
        selectors,
        for_secs,
        order,
        from,
        bus: _,
    } = cli;
    let order = match order {
        OrderArg::Arrival => Order::Arrival,
        OrderArg::Hlc => Order::Hlc,
    };
    let window = match from {
        Some(path) => from_zrec(&path)?,
        None => {
            // Clap has already required `--for` without `--from`; the
            // positivity check is the one every window shares (#307).
            let secs = for_secs.expect("clap requires --for without --from");
            live(&selectors, secs, args).await?
        }
    };
    let report = timeline(&window, order);
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())?;
    Ok(())
}

/// Watch the selectors for the window and collect what arrived, in order.
async fn live(selectors: &[String], for_secs: f64, args: &Bus) -> Result<Window> {
    let duration = super::positive_secs("--for", for_secs)?;
    let selectors: Vec<String> = selectors
        .iter()
        .map(|s| super::raw_selector(s).map(str::to_string))
        .collect::<Result<_>>()?;

    let session = args.session().await?;
    let monitor =
        zenkey_fleet::Monitor::start(&session, zenkey_fleet::MonitorSpec::default()).await?;
    let mut events = monitor.events();
    for selector in &selectors {
        monitor.watch(selector).await?;
    }
    // The window starts once every watch is declared — the first instant
    // the observer could have seen all of what it was asked to.
    let epoch = Instant::now();
    eprintln!(
        "timeline over {} selector(s) for {for_secs}s…{}",
        selectors.len(),
        if selectors.iter().any(|s| s.contains("**")) {
            " (`**` cannot cross `@`-planes; they are excluded, not empty)"
        } else {
            ""
        }
    );

    let base = args.base();
    let mut rows = Vec::new();
    let mut breaks = Vec::new();
    let deadline = tokio::time::sleep(duration);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            () = &mut deadline => break,
            item = events.recv() => match item {
                None => break,
                Some(StreamItem::Dropped(n)) => breaks.push(PlacedBreak {
                    after: rows.len(),
                    lane: None,
                    kind: Break::Dropped(n),
                }),
                Some(StreamItem::Event(FleetEvent::Sample(view))) => {
                    rows.push(TimelineRow::from_view(&view, epoch, base));
                }
                Some(StreamItem::Event(_)) => {}
            },
        }
    }
    let keys_evicted = monitor.core().keys_evicted();
    monitor.stop();
    Ok(Window {
        rows,
        breaks,
        scopes: selectors,
        window_s: Some(for_secs),
        source: TimelineSource::Live,
        keys_evicted,
    })
}

/// Read a capture through the same projection, under the base the capture
/// itself states: a file outlives the session that wrote it, and the keys
/// in it were recorded whole under *that* base (RFC 09 §5.2).
fn from_zrec(path: &std::path::Path) -> Result<Window> {
    let file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut reader = ZrecReader::new(BufReader::new(file))
        .with_context(|| format!("read {} as a .zrec", path.display()))?;
    let header = reader.header().clone();
    let mut rows = Vec::new();
    let mut breaks = Vec::new();
    let mut malformed = 0u64;
    let mut preamble = 0u64;
    while let Some(item) = reader.next() {
        match item {
            Ok(item) => match Ingested::from_zrec(&item, &header.base) {
                Ingested::Row(row) => rows.push(row),
                Ingested::Break(kind) => breaks.push(PlacedBreak {
                    after: rows.len(),
                    lane: None,
                    kind,
                }),
                // A version-2 preamble row is state at capture start, not an
                // arrival: it sits on neither axis, and is counted rather
                // than placed (RFC 13 §4.1). A trigger record is a marker
                // the timeline has no lane for.
                Ingested::Preamble { .. } => preamble += 1,
                Ingested::Trigger { .. } => {}
            },
            Err(reason) => {
                // Counted, never skipped in silence (`tape::ingest`'s rule).
                malformed += 1;
                eprintln!("{}: {reason}", path.display());
            }
        }
    }
    if preamble > 0 {
        eprintln!(
            "{}: {preamble} preamble row(s) not placed — state at capture start is not \
             an arrival (RFC 13 §4.1)",
            path.display()
        );
    }
    if malformed > 0 {
        eprintln!(
            "{malformed} malformed line(s) in {} are not on the timeline",
            path.display()
        );
    }
    Ok(Window {
        rows,
        breaks,
        scopes: header.selectors,
        window_s: None,
        source: TimelineSource::Zrec {
            path: path.display().to_string(),
        },
        // A file carries no statistics table; nothing was retired from one.
        keys_evicted: 0,
    })
}
