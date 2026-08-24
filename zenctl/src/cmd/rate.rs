//! `zenctl rate` — watch a window, report rates (ros2-style), through the
//! typed [`RateReport`](crate::report::RateReport) so `--format json` applies
//! (issue #46) and the O6 eviction count is printed, never swallowed.
//!
//! One verb since #307. `topic hz` and `topic bw` were two spellings of one
//! observation: the same Monitor, the same window, the same eviction ledger,
//! differing only in which number the table led with. `--bytes` is that
//! choice, and it is a rendering flag, which is what it always was.

use anyhow::Result;

use crate::cli::SelectorArgs;
use crate::{Bus, report};

#[allow(clippy::too_many_arguments)]
pub async fn run(
    sel: &SelectorArgs,
    for_secs: f64,
    per_key: bool,
    loss: bool,
    latency: bool,
    bandwidth: bool,
    args: &Bus,
) -> Result<()> {
    let selector = super::selector_of(sel, args)?;
    let window = super::positive_secs("--for", for_secs)?;
    let session = args.session().await?;
    let monitor = zenkey_fleet::Monitor::start(
        &session,
        zenkey_fleet::MonitorSpec {
            selectors: vec![selector.clone()],
            ..Default::default()
        },
    )
    .await?;

    eprintln!("measuring {selector} for {for_secs}s…");
    tokio::time::sleep(window).await;

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
            window_s: for_secs,
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
