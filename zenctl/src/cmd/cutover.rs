//! `zenctl cutover` (issue #59): the RFC 09 §6 acceptance, half one —
//! prove the retired key family **silent** while the new planes carry
//! traffic. One without the other is not evidence: a quiet old root on a
//! dead fleet proves only that the fleet is dead.
//!
//! The leak check states its meaning explicitly — *anything outside
//! `<base>/v1/`* — rather than riding on key algebra, because the version
//! chunk is plain (`v1`, not `@v1`) and `<base>/**` reaches the new keys
//! too (RFC 09 §6's own note). And the scope statement is honest per O5:
//! a `**` subscription cannot cross `@`-chunks, so the verbatim planes and
//! the admin space are outside this check by construction.

use std::collections::BTreeMap;

use anyhow::Result;

use crate::BusArgs;
use crate::output;

/// How many distinct offending keys each bucket names in the report.
const EXAMPLE_CAP: usize = 20;

pub async fn run(old_root: &str, window: u64, args: &BusArgs) -> Result<()> {
    let old_expr = zenoh::key_expr::KeyExpr::try_from(old_root.to_string())
        .map_err(|e| anyhow::anyhow!("--old-root {old_root:?} is not a key expression: {e}"))?;
    let base = args.base().to_string();
    // The stated meaning of "new plane": under <base>/v1/. Everything else
    // that is not the old root is a leak *by this check's definition*.
    let new_prefix = format!("{}/", zenkey::grammar::with_base(&base, "v1"));

    let session = args.session().await?;
    let monitor =
        zenkey_fleet::Monitor::start(&session, zenkey_fleet::MonitorSpec::default()).await?;
    let mut events = monitor.events();
    monitor.watch("**").await?;

    eprintln!(
        "cutover check: {window}s window — asserting {old_root} silent while \
         {new_prefix}** carries traffic (RFC 09 §6). `**` cannot cross \
         `@`-chunks: verbatim planes and the admin space are outside this \
         check by construction (O5)."
    );

    let mut old_keys: BTreeMap<String, u64> = BTreeMap::new();
    let mut leaked: BTreeMap<String, u64> = BTreeMap::new();
    let (mut old_samples, mut new_samples, mut leak_samples, mut dropped) =
        (0u64, 0u64, 0u64, 0u64);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(window);
    loop {
        let item = tokio::select! {
            item = events.recv() => item,
            _ = tokio::time::sleep_until(deadline) => break,
        };
        match item {
            Some(zenkey_fleet::StreamItem::Event(zenkey_fleet::FleetEvent::Sample(s))) => {
                // Old-root membership is key-expression inclusion (the root
                // may be a wildcard family); the new plane is a stated
                // prefix. Old wins ties: a root inside <base>/v1/ is
                // being *retired*, and its traffic is the failure.
                if zenoh::key_expr::KeyExpr::try_from(s.key.as_str())
                    .map(|k| old_expr.includes(&k))
                    .unwrap_or(false)
                {
                    old_samples += 1;
                    *old_keys.entry(s.key.clone()).or_default() += 1;
                } else if s.key.starts_with(&new_prefix) {
                    new_samples += 1;
                } else {
                    leak_samples += 1;
                    *leaked.entry(s.key.clone()).or_default() += 1;
                }
            }
            Some(zenkey_fleet::StreamItem::Dropped(n)) => dropped += n,
            Some(_) => continue,
            None => break,
        }
    }
    monitor.stop();

    let cap = |m: &BTreeMap<String, u64>| {
        m.iter()
            .take(EXAMPLE_CAP)
            .map(|(k, n)| format!("{k} ({n})"))
            .collect::<Vec<_>>()
    };
    let report = zenkey_fleet::report::CutoverReport {
        old_root: old_root.to_string(),
        new_prefix: new_prefix.clone(),
        window_s: window,
        old_samples,
        old_keys_seen: old_keys.len(),
        old_examples: cap(&old_keys),
        new_samples,
        leak_samples,
        leaked_keys_seen: leaked.len(),
        leak_examples: cap(&leaked),
        dropped,
        verdict: if old_samples > 0 {
            zenkey_fleet::report::CutoverVerdict::OldStillSpeaks
        } else if new_samples == 0 {
            zenkey_fleet::report::CutoverVerdict::Unproven
        } else {
            zenkey_fleet::report::CutoverVerdict::Pass
        },
    };
    output::cutover(&report, args.format);
    match report.verdict {
        zenkey_fleet::report::CutoverVerdict::Pass => Ok(()),
        zenkey_fleet::report::CutoverVerdict::OldStillSpeaks => std::process::exit(1),
        zenkey_fleet::report::CutoverVerdict::Unproven => std::process::exit(2),
    }
}
