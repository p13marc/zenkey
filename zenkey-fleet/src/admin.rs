//! Zenoh admin-space access (issue #14): browse `@/**` — the middleware's
//! own introspection — from the same un-namespaced session the convention
//! tooling already holds (a namespaced session's admin selector would be
//! rewritten and match nothing, RFC 09 §5).
//!
//! Admin key layouts vary between zenoh versions (report §3.1's caveat), so
//! this module stays a thin, honest transport: keys + JSON values, no
//! hardcoded schema. `routers` extracts the few fields every 1.x layout
//! carries, and leaves the rest visible in `raw`.

use std::time::Duration;

use anyhow::{Result, anyhow};
use zenoh::Session;
use zenoh::query::{ConsolidationMode, QueryTarget};

/// One admin-space entry.
#[derive(Debug, Clone)]
pub struct AdminEntry {
    pub key: String,
    pub value: serde_json::Value,
}

/// GET an admin selector (default `@/**`). Fans to every node (target All,
/// consolidation None — several routers may answer).
pub async fn admin_get(
    session: &Session,
    selector: &str,
    timeout: Duration,
) -> Result<Vec<AdminEntry>> {
    let replies = session
        .get(selector)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::None)
        .timeout(timeout)
        .await
        .map_err(|e| anyhow!("admin get {selector}: {e}"))?;
    let mut out = Vec::new();
    while let Ok(reply) = replies.recv_async().await {
        let Ok(sample) = reply.result() else { continue };
        let bytes = sample.payload().to_bytes();
        let value = serde_json::from_slice(&bytes).unwrap_or_else(|_| {
            serde_json::Value::String(String::from_utf8_lossy(&bytes).to_string())
        });
        out.push(AdminEntry {
            key: sample.key_expr().as_str().to_string(),
            value,
        });
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(out)
}

/// A router (or peer) as the admin space reports it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RouterInfo {
    pub zid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub locators: Vec<String>,
    /// The full admin document, untrimmed — layouts vary by version.
    pub raw: serde_json::Value,
}

/// Enumerate routers/peers from `@/*/router` (and the fields every layout
/// carries).
pub async fn routers(session: &Session, timeout: Duration) -> Result<Vec<RouterInfo>> {
    let entries = admin_get(session, "@/*/router", timeout).await?;
    Ok(entries
        .into_iter()
        .map(|e| {
            let zid = e
                .value
                .get("zid")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    // Fall back to the key's zid chunk: @/<zid>/router.
                    e.key.split('/').nth(1).unwrap_or("?").to_string()
                });
            let version = e
                .value
                .get("version")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let locators = e
                .value
                .get("locators")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|l| l.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            RouterInfo {
                zid,
                version,
                locators,
                raw: e.value,
            }
        })
        .collect())
}
