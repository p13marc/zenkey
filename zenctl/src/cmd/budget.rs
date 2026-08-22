//! `topic list --budget` (#221): the declared `cardinality` bound joined to
//! an observed key population, in the same table `topic list` already draws.
//!
//! Registry knowledge comes from wherever [`crate::Bus::slice_set`] found it
//! (live introspect, `--registry` dirs, or the union); the *population* can
//! only come from the wire, so this verb watches the data planes for a
//! bounded window first — the RFC 09 §5.1 discipline throughout: the window
//! and scopes ride with the numbers (O5), the observer's bound is reported
//! (O6), and observed-under-declared is never rendered as a pass (O4 — a
//! bounded window proves a lower bound, never the population).

use std::time::Duration;

use anyhow::Result;

use crate::Bus;

pub async fn topic_list_budget(
    filter: &super::watch::TopicFilter,
    secs: u64,
    args: &Bus,
) -> Result<()> {
    let slices = args.slice_set().await?;
    let mut report = filter.apply(&slices)?;

    let session = args.session().await?;
    // The same scope statement the doctor's listen phase watches: `**`
    // never crosses an `@` chunk (RFC 03 §4 D2), so each declared service
    // origin's planes are named explicitly.
    let scopes = zenkey_fleet::data_plane_scopes(args.base(), &slices);
    let monitor = zenkey_fleet::Monitor::start(
        &session,
        zenkey_fleet::MonitorSpec {
            selectors: scopes.clone(),
            ..Default::default()
        },
    )
    .await?;
    eprintln!("observing the key population for {secs}s…");
    tokio::time::sleep(Duration::from_secs(secs)).await;
    let (keys, evicted, observed_keys) = monitor.core().with_stats(|stats| {
        (
            stats.len(),
            stats.evicted(),
            stats
                .iter()
                .map(|(k, _)| k.to_string())
                .collect::<Vec<String>>(),
        )
    });
    monitor.stop();

    let obs = zenkey_fleet::BudgetObservation::observe(
        args.base(),
        &slices,
        observed_keys.iter().map(String::as_str),
    );
    zenkey_fleet::join_budget(
        &mut report,
        &obs,
        zenkey_fleet::report::BudgetWindow {
            window_s: secs,
            scopes,
            keys,
            evicted,
        },
    );
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())
}
