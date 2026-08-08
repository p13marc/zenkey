//! `topic hz` / `topic bw` — watch a window, report rates (ros2-style),
//! through the typed [`RateReport`](crate::report::RateReport) so `--format
//! json` applies (issue #46) and the O6 eviction count is printed, never
//! swallowed.

use std::time::Duration;

use anyhow::Result;

use crate::{BusArgs, output, report};

#[allow(clippy::too_many_arguments)]
pub async fn run(
    selector: Option<&str>,
    origin: Option<&str>,
    class: Option<&str>,
    producer: Option<&str>,
    window: u64,
    per_key: bool,
    loss: bool,
    bandwidth: bool,
    args: &BusArgs,
) -> Result<()> {
    let selector = match selector {
        Some(s) => s.to_string(),
        None => super::compose_selector(args, origin, class, producer)?,
    };
    let session = args.session().await?;
    let monitor = zenkey_fleet::Monitor::start(
        &session,
        zenkey_fleet::MonitorSpec {
            selectors: vec![selector.clone()],
            ..Default::default()
        },
    )
    .await?;

    eprintln!("measuring {selector} for {window}s…");
    tokio::time::sleep(Duration::from_secs(window)).await;

    let rep = monitor.core().with_stats(|stats| {
        let (total_count, total_bytes, _) = stats.totals();
        let mut rows = Vec::new();
        if per_key {
            rows = stats
                .iter()
                .map(|(k, s)| report::RateRow {
                    key: k.to_string(),
                    count: s.count,
                    bytes: s.bytes,
                    sn_gaps: s.sn_gaps,
                })
                .collect::<Vec<_>>();
            rows.sort_by_key(|r| std::cmp::Reverse(r.count));
        }
        report::RateReport {
            selector: selector.clone(),
            window_s: window,
            rows,
            total_count,
            total_bytes,
            keys: stats.len(),
            evicted: stats.evicted(),
            max_keys: stats.max_keys(),
            sn_gaps: loss.then(|| stats.iter().map(|(_, s)| s.sn_gaps).sum()),
        }
    });
    monitor.stop();
    output::rate(&rep, args.format, bandwidth, loss);
    Ok(())
}
