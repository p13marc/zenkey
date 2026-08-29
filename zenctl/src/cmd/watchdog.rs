//! `zenctl watchdog` (#227) — transitions, not states.
//!
//! The continuous observer over the engine's closed condition vocabulary
//! (`zenkey_fleet::judge::condition`): every genuine state change is one ndjson
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
use zenkey_fleet::Sipper as _;
use zenkey_fleet::judge::condition::{Condition, WatchdogSpec};

use crate::Bus;

pub async fn run(cli: crate::cli::WatchdogArgs) -> Result<()> {
    let bus = Bus::resolve(&cli.bus)?;
    let args = &bus;
    let crate::cli::WatchdogArgs {
        rules,
        every,
        count,
        bus: _,
    } = cli;
    let rules = rules.as_slice();
    // `--every`/`--count`, not `--tick`/`--ticks` (#307): one period flag and
    // one stop-bound flag across the whole tool.
    let tick = super::positive_secs("--every", every)?;
    let rules: Vec<Condition> = rules
        .iter()
        .map(|r| Condition::parse(r))
        .collect::<std::result::Result<_, _>>()
        .map_err(anyhow::Error::from)?;

    let session = args.session().await?;
    // Slices enrich: `qos-mismatch` and `invalid-payload` judge against the
    // registry; with none loaded they observe and say what they could not
    // judge (O4), and doctor rules run their own asks.
    let slices = args.slices_optional().await?;
    let store = zenkey_fleet::SchemaStore::new(args.base(), args.timeout());
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
    let fleet = args.fleet(&session);
    // The engine yields transitions and returns the summary (#397), so the
    // write happens where its error can be answered for. This verb used to
    // keep the first `io::Error` in a local and answer after the run, because
    // the engine's callback was infallible — a workaround every next caller
    // would have rediscovered (#360).
    let mut run = zenkey_fleet::watchdog(&fleet, slices.as_ref(), &store, &spec).pin();
    let interrupted = loop {
        let next = tokio::select! {
            t = run.sip() => t,
            _ = tokio::signal::ctrl_c() => {
                eprintln!("watchdog: interrupted");
                break true;
            }
        };
        let Some(t) = next else { break false };
        // Tagged (`"row":"transition"`) like every non-sample line of an
        // explorer stream, so a consumer can select or skip them by kind.
        let written = writeln!(
            out,
            "{}",
            crate::render::Row::of("transition", &t).into_line()
        )
        .and_then(|()| out.flush());
        if let Err(e) = written {
            // A closed pipe is the consumer saying "enough"; anything else is
            // this verb's output not arriving, which is not a clean run.
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                return Ok(());
            }
            return Err(anyhow::Error::new(e).context("failed to write the transition stream"));
        }
    };
    if interrupted {
        return Ok(());
    }
    // Awaiting the run is what performs the acknowledged monitor teardown and
    // hands back the summary — the two the callback shape had nowhere to put.
    let summary = run.await?;
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
