//! `topic hz` / `topic bw` — watch a window, report rates (ros2-style),
//! through the typed [`RateReport`](crate::report::RateReport) so `--format
//! json` applies (issue #46) and the O6 eviction count is printed, never
//! swallowed.

use std::time::Duration;

use anyhow::Result;

use crate::{Bus, report};

#[allow(clippy::too_many_arguments)]
pub async fn run(
    selector: Option<&str>,
    origin: Option<&str>,
    class: Option<&str>,
    producer: Option<&str>,
    window: u64,
    per_key: bool,
    loss: bool,
    latency: bool,
    bandwidth: bool,
    args: &Bus,
) -> Result<()> {
    let selector = match selector {
        // Typed selectors pass the raw seam (`$*` refusal, RFC 03 §2);
        // composed ones cannot spell it.
        Some(s) => super::raw_selector(s)?.to_string(),
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
                    // R3 (#238's twin): only when asked, like the
                    // report-level `sn_gaps` always was — the row used to
                    // serialize an uncaveated `"sn_gaps": 0` without
                    // `--loss`.
                    sn_gaps: loss.then_some(s.sn_gaps).into(),
                    // #238: only when asked. It used to be unconditional,
                    // so `--format json` carried a latency distribution
                    // nobody requested — and without the O7 caveat naming
                    // which clock it came from, which printed only under
                    // `--latency`.
                    latency: latency.then(|| s.latency()).flatten(),
                    // The other half of the latency observation rides the
                    // same gate (R3).
                    unstamped: latency.then_some(s.unstamped).into(),
                })
                .collect::<Vec<_>>();
            rows.sort_by_key(|r| std::cmp::Reverse(r.count));
        }
        report::RateReport {
            selector: selector.clone(),
            window_s: window as f64,
            rows,
            total_count,
            total_bytes,
            keys: stats.len(),
            evicted: stats.evicted(),
            max_keys: stats.max_keys(),
            sn_gaps: loss
                .then(|| stats.iter().map(|(_, s)| s.sn_gaps).sum())
                .into(),
        }
    });
    monitor.stop();
    crate::render::emit_with(
        &mut std::io::stdout(),
        &crate::render::RateView {
            report: &rep,
            bandwidth,
        },
        args.format(),
        args.color(),
    )?;
    Ok(())
}
