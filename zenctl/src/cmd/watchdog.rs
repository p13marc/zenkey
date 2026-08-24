//! `zenctl watchdog` (#227) — transitions, not states.
//!
//! The continuous observer over the engine's closed condition vocabulary
//! (`zenkey_fleet::condition`): every genuine state change is one ndjson
//! line on stdout, an unchanged tick prints nothing. A **foreground**
//! process, explicitly launched, one per invocation, no shared state — the
//! redesign ledger's "no daemon" decision rejected a hidden discovery-caching
//! server, not this (`docs/redesign-2026-07.md` §6.1).
//!
//! Output is ndjson regardless of `--format`: a stream of transitions has
//! one honest encoding — a table redraw would be a state display, which is
//! exactly what this verb exists not to be.

use std::io::Write as _;

use anyhow::Result;
use zenkey_fleet::condition::{Condition, Transition, WatchdogSpec, run_watchdog};

use crate::Bus;

pub async fn run(rules: &[String], every: f64, count: Option<u64>, args: &Bus) -> Result<()> {
    // `--every`/`--count`, not `--tick`/`--ticks` (#264): one period flag and
    // one stop-bound flag across the whole tool.
    let tick = super::positive_secs("--every", every)?;
    let rules: Vec<Condition> = rules
        .iter()
        .map(|r| Condition::parse(r))
        .collect::<Result<_>>()?;

    let session = args.session().await?;
    // Slices enrich: `qos-mismatch` and `invalid-payload` judge against the
    // registry; with none loaded they observe and say what they could not
    // judge (O4), and doctor rules run their own asks.
    let slices = args.slices_optional().await?;
    let store = zenkey_fleet::decode::SchemaStore::new(args.base(), args.timeout());
    let spec = WatchdogSpec {
        rules,
        tick,
        ticks: count,
        timeout: args.timeout(),
    };

    eprintln!(
        "watchdog: {} rule(s), every {every}s — one ndjson line per genuine state \
         change, none per unchanged tick; three states, ok/firing/unobservable \
         (RFC 09 §5.1 O4/O6)",
        spec.rules.len()
    );
    let mut out = std::io::stdout();
    let mut emit = |t: &Transition| {
        // Tagged (`"row":"transition"`) like every non-sample line of an
        // explorer stream, so a consumer can select or skip them by kind.
        if let Ok(v) = serde_json::to_value(t) {
            let _ = writeln!(
                out,
                "{}",
                crate::render::Row::tagged("transition", v).into_line()
            );
            let _ = out.flush();
        }
    };
    let fleet = args.fleet(&session);
    let summary = tokio::select! {
        r = run_watchdog(&fleet, slices.as_ref(), &store, &spec, &mut emit) => r?,
        _ = tokio::signal::ctrl_c() => {
            eprintln!("watchdog: interrupted");
            return Ok(());
        }
    };
    eprintln!(
        "watchdog: {} tick(s), {} transition(s){}",
        summary.ticks,
        summary.transitions,
        // The bounded facts cache's cost (RFC 09 §5.1 O6): said when paid,
        // silent when not.
        if summary.facts_evicted > 0 {
            format!(
                ", {} key projection(s) retired at the cache bound",
                summary.facts_evicted
            )
        } else {
            String::new()
        }
    );
    Ok(())
}
