//! The liveliness roster (RFC 04 §5).

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::Result;
use zenkey::grammar::with_base;
use zenoh::Session;

/// The fleet-presence roster: who is up, and what they run.
///
/// RFC 04 §5 — a liveliness query on `<base>/v1/*/state/*/alive`. Zero
/// payload bytes: the token *key* is the record. `@catalog` is asked for by
/// name because `*` can never match a verbatim service origin (property D4).
pub async fn roster(
    session: &Session,
    base: &str,
    timeout: Duration,
) -> Result<BTreeMap<String, Vec<String>>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();

    let catalog_alive = zenkey::selector::service_alive(&zenkey::ServiceOrigin::catalog());
    // The builders are base-relative; this session is deliberately
    // un-namespaced, so it must spell the base itself.
    for expr in [
        with_base(
            base,
            zenkey::selector::all_liveliness(zenkey::selector::Scope::fleet()),
        ),
        with_base(base, catalog_alive),
    ] {
        let Ok(replies) = session.liveliness().get(&expr).timeout(timeout).await else {
            continue;
        };
        while let Ok(reply) = replies.recv_async().await {
            let Ok(sample) = reply.result() else { continue };
            let key = sample.key_expr().as_str();
            let Some(parsed) = zenkey::grammar::parse_full(base, key) else {
                continue;
            };
            let origin = parsed.origin.chunk().to_string();
            // `@catalog`'s token has no producer chunk — the service *is* the
            // producer. Everything else names its producer in position 5.
            let producer = parsed
                .producer
                .as_ref()
                .map(|p| p.chunk())
                .unwrap_or_else(|| origin.trim_start_matches('@').to_string());
            out.entry(origin).or_default().push(producer);
        }
    }
    for producers in out.values_mut() {
        producers.sort();
        producers.dedup();
    }
    Ok(out)
}
