//! RFC 09 §6 cutover acceptance, half one (issue #59): prove the retired key
//! family **silent** while the new planes carry traffic.
//!
//! One without the other is not evidence — a quiet old root on a dead fleet
//! proves only that the fleet is dead, which is why the verdict has three
//! states and not two.
//!
//! The leak check states its meaning explicitly — *anything outside
//! `<base>/v1/`* — rather than riding on key algebra, because the version
//! chunk is plain (`v1`, not `@v1`) and `<base>/**` reaches the new keys too
//! (RFC 09 §6's own note). And the scope is honest per RFC 09 §5.1 O5: a `**`
//! subscription cannot cross `@`-chunks, so the verbatim planes and the admin
//! space are outside this check by construction.
//!
//! Lives here rather than in a frontend (issue #206) because every line of it
//! is judgement over bus traffic: the sample-bucketing rule, the tie-break and
//! the verdict ladder are the check, and a second explorer must not have to
//! re-derive them. The frontend keeps the session, the rendering and the exit
//! code.

use std::collections::BTreeMap;

use anyhow::Result;
use zenoh::Session;

use crate::report::{CutoverReport, CutoverVerdict};

/// How many distinct offending keys each bucket names in the report.
const EXAMPLE_CAP: usize = 20;

/// The scope sentence this check operates under — rendered by the caller
/// before the window opens, because a user watching a 30-second silence
/// deserves to know what was and was not being watched (O5).
pub fn scope_note(old_root: &str, new_prefix: &str, window: u64) -> String {
    format!(
        "cutover check: {window}s window — asserting {old_root} silent while \
         {new_prefix}** carries traffic (RFC 09 §6). `**` cannot cross \
         `@`-chunks: verbatim planes and the admin space are outside this \
         check by construction (O5)."
    )
}

/// The stated meaning of "the new plane": keys under `<base>/v1/`.
pub fn new_prefix(base: &str) -> String {
    format!("{}/", zenkey::grammar::with_base(base, "v1"))
}

/// Watch the whole bus for `window` seconds and judge the cutover.
pub async fn run_cutover(
    session: &Session,
    base: &str,
    old_root: &str,
    window: u64,
) -> Result<CutoverReport> {
    let old_expr = zenoh::key_expr::KeyExpr::try_from(old_root.to_string())
        .map_err(|e| anyhow::anyhow!("--old-root {old_root:?} is not a key expression: {e}"))?;
    let new_prefix = new_prefix(base);

    let monitor = crate::Monitor::start(session, crate::MonitorSpec::default()).await?;
    let mut events = monitor.events();
    monitor.watch("**").await?;

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
            Some(crate::StreamItem::Event(crate::FleetEvent::Sample(s))) => {
                // Old-root membership is key-expression inclusion (the root
                // may be a wildcard family); the new plane is a stated
                // prefix. Old wins ties: a root inside <base>/v1/ is being
                // *retired*, and its traffic is the failure.
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
            Some(crate::StreamItem::Dropped(n)) => dropped += n,
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
    Ok(CutoverReport {
        old_root: old_root.to_string(),
        new_prefix,
        window_s: window,
        old_samples,
        old_keys_seen: old_keys.len(),
        old_examples: cap(&old_keys),
        new_samples,
        leak_samples,
        leaked_keys_seen: leaked.len(),
        leak_examples: cap(&leaked),
        dropped,
        verdict: verdict(old_samples, new_samples),
    })
}

/// The three-state ladder, pure so it can be exercised without a bus.
///
/// Order matters: the old root speaking is a failure whatever else is true,
/// and a silent old root on a silent bus is *unproven* rather than a pass —
/// a dead fleet passes the silence half for free (RFC 05 §3.1).
pub fn verdict(old_samples: u64, new_samples: u64) -> CutoverVerdict {
    if old_samples > 0 {
        CutoverVerdict::OldStillSpeaks
    } else if new_samples == 0 {
        CutoverVerdict::Unproven
    } else {
        CutoverVerdict::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ladder's three states, including the one nothing exercised: a
    /// quiet old root on a quiet bus is not evidence of a finished migration.
    #[test]
    fn silence_on_both_planes_is_unproven_not_a_pass() {
        assert_eq!(verdict(0, 12), CutoverVerdict::Pass);
        assert_eq!(verdict(3, 12), CutoverVerdict::OldStillSpeaks);
        assert_eq!(verdict(0, 0), CutoverVerdict::Unproven);
        // Old wins ties: traffic on the retired family is the failure even
        // when the new plane is busy.
        assert_eq!(verdict(1, 10_000), CutoverVerdict::OldStillSpeaks);
    }

    #[test]
    fn the_new_plane_is_a_stated_prefix_not_key_algebra() {
        assert_eq!(new_prefix("acme"), "acme/v1/");
        // The base-less deployment (RFC v1.6) is a real one.
        assert_eq!(new_prefix(""), "v1/");
        assert_eq!(new_prefix("a/b"), "a/b/v1/");
    }

    #[test]
    fn the_scope_note_states_what_it_cannot_see() {
        let note = scope_note("old/**", "acme/v1/", 30);
        assert!(note.contains("30s window"));
        assert!(
            note.contains("cannot cross"),
            "a wildcard scope must not be presented as total coverage (O5): {note}"
        );
    }
}
