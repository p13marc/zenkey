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

/// One configured storage, as the admin space reports it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StorageInfo {
    pub zid: String,
    pub name: String,
    /// The key expression the storage captures, when the layout exposes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_expr: Option<String>,
    /// The full admin document, untrimmed — layouts vary by version.
    pub raw: serde_json::Value,
}

/// Extract a storage from one admin entry, tolerantly: the key shape is
/// `@/<zid>/router/…/storage_manager/storages/<name>[…]`, the value a config
/// document whose `key_expr` field names what it captures. Pure — the
/// version-variance lives here, unit-tested.
pub fn storage_from_admin_entry(key: &str, value: &serde_json::Value) -> Option<StorageInfo> {
    let chunks: Vec<&str> = key.split('/').collect();
    let storages_pos = chunks.iter().position(|c| *c == "storages")?;
    // Only storage_manager subtrees qualify (volumes etc. share the plugin).
    if chunks.get(storages_pos.checked_sub(1)?) != Some(&"storage_manager") {
        return None;
    }
    let name = chunks.get(storages_pos + 1)?;
    let zid = chunks.get(1).unwrap_or(&"?");
    let key_expr = value
        .get("key_expr")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Some(StorageInfo {
        zid: (*zid).to_string(),
        name: (*name).to_string(),
        key_expr,
        raw: value.clone(),
    })
}

/// Enumerate configured storages across the mesh (issue #14). Zero routers
/// (peer mesh, admin disabled) is an empty vec, never an error.
pub async fn storages(session: &Session, timeout: Duration) -> Result<Vec<StorageInfo>> {
    let entries = admin_get(
        session,
        "@/*/router/**/storage_manager/storages/**",
        timeout,
    )
    .await?;
    let mut out: Vec<StorageInfo> = entries
        .iter()
        .filter_map(|e| storage_from_admin_entry(&e.key, &e.value))
        .collect();
    // One row per (zid, name): config and status subtrees can both answer.
    out.sort_by(|a, b| (&a.zid, &a.name).cmp(&(&b.zid, &b.name)));
    out.dedup_by(|a, b| {
        if a.zid == b.zid && a.name == b.name {
            // Keep the richer entry (the one that names a key_expr).
            if b.key_expr.is_none() {
                b.key_expr = a.key_expr.take();
            }
            true
        } else {
            false
        }
    });
    Ok(out)
}

/// How a declared state family relates to the configured storages.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "coverage", content = "storage")]
pub enum Coverage {
    /// Some storage's key expression includes every key of the family.
    Covered(String),
    /// A storage overlaps the family but does not include all of it.
    Partial(String),
    /// No storage touches the family. For volatile (ttl'd) state this can be
    /// legitimate — advanced-pub/sub cache seeding (RFC 04 §3.5); storage is
    /// authoritative for durable data.
    Uncovered,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CoverageRow {
    pub producer: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_s: Option<i64>,
    #[serde(flatten)]
    pub coverage: Coverage,
}

/// Judge every declared **state** family against the configured storages
/// (issue #14): the family's wire selector vs each storage's key expression,
/// by key algebra (`includes` ⇒ covered, `intersects` ⇒ partial). Pure.
pub fn state_coverage(
    slices: &crate::registry::SliceSet,
    base: &str,
    storages: &[StorageInfo],
) -> Vec<CoverageRow> {
    use zenoh::key_expr::keyexpr;
    let storage_kes: Vec<(&StorageInfo, &keyexpr)> = storages
        .iter()
        .filter_map(|s| {
            let ke = s.key_expr.as_deref()?;
            keyexpr::new(ke).ok().map(|ke| (s, ke))
        })
        .collect();
    let mut rows = Vec::new();
    for slice in slices.slices() {
        for subject in &slice.subjects {
            if subject.class != "state" {
                continue;
            }
            let Ok(pattern) = zenkey::pattern::SubjectPattern::parse(&subject.path) else {
                continue;
            };
            // Composed via `with_base` so the empty base stays a valid
            // keyexpr (`format!("{base}/…")` would grow a leading slash and
            // silently drop every family below).
            let selector = match &slice.service_origin {
                Some(origin) => zenkey::grammar::with_base(
                    base,
                    format!("v1/{origin}/state/{}", pattern.selector_tail()),
                ),
                None => zenkey::grammar::with_base(
                    base,
                    format!("v1/*/state/{}/{}", slice.name, pattern.selector_tail()),
                ),
            };
            let Ok(family) = keyexpr::new(selector.as_str()) else {
                continue;
            };
            let mut coverage = Coverage::Uncovered;
            for (info, ke) in &storage_kes {
                if ke.includes(family) {
                    coverage = Coverage::Covered(format!("{}@{}", info.name, info.zid));
                    break;
                }
                if ke.intersects(family) && coverage == Coverage::Uncovered {
                    coverage = Coverage::Partial(format!("{}@{}", info.name, info.zid));
                }
            }
            rows.push(CoverageRow {
                producer: slice.name.clone(),
                path: subject.path.clone(),
                ttl_s: subject.ttl_s,
                coverage,
            });
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_extraction_tolerates_layouts() {
        // 1.x config-subtree shape.
        let v = serde_json::json!({"key_expr": "zs/v1/*/state/**", "volume": "fs"});
        let s = storage_from_admin_entry(
            "@/abc123/router/config/plugins/storage_manager/storages/latest",
            &v,
        )
        .unwrap();
        assert_eq!(s.zid, "abc123");
        assert_eq!(s.name, "latest");
        assert_eq!(s.key_expr.as_deref(), Some("zs/v1/*/state/**"));
        // Status-subtree shape without key_expr still names the storage.
        let s = storage_from_admin_entry(
            "@/abc123/router/status/plugins/storage_manager/storages/latest/info",
            &serde_json::json!("ok"),
        )
        .unwrap();
        assert_eq!(s.name, "latest");
        assert!(s.key_expr.is_none());
        // Non-storage subtrees do not match.
        assert!(
            storage_from_admin_entry(
                "@/abc123/router/config/plugins/storage_manager/volumes/fs",
                &serde_json::json!({}),
            )
            .is_none()
        );
    }

    fn slices_with_state() -> crate::registry::SliceSet {
        let toml = r#"
            [registry]
            version = "1.0"
            app = "t"
            convention = 1
            [producer]
            name = "tc"
            [[subject]]
            path = "health"
            class = "state"
            type = "Health"
            ttl_s = 60
            [[subject]]
            path = "config/{iface}"
            class = "state"
            type = "Config"
            ttl_s = 120
            [[subject]]
            path = "bandwidth"
            class = "telemetry"
            type = "Point"
        "#;
        crate::registry::SliceSet::from_toml_for_tests(toml)
    }

    fn storage(name: &str, key_expr: &str) -> StorageInfo {
        StorageInfo {
            zid: "z1".into(),
            name: name.into(),
            key_expr: Some(key_expr.into()),
            raw: serde_json::Value::Null,
        }
    }

    #[test]
    fn coverage_judges_covered_partial_uncovered() {
        let slices = slices_with_state();
        // Full state storage: everything covered; telemetry not judged.
        let rows = state_coverage(&slices, "zs", &[storage("latest", "zs/v1/*/state/**")]);
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|r| matches!(r.coverage, Coverage::Covered(_)))
        );

        // A one-interface storage: config/{iface} is partial, health uncovered.
        let rows = state_coverage(
            &slices,
            "zs",
            &[storage("one", "zs/v1/*/state/tc/config/eth0")],
        );
        let health = rows.iter().find(|r| r.path == "health").unwrap();
        assert_eq!(health.coverage, Coverage::Uncovered);
        let config = rows.iter().find(|r| r.path == "config/{iface}").unwrap();
        assert!(matches!(config.coverage, Coverage::Partial(_)));

        // No storages at all.
        let rows = state_coverage(&slices, "zs", &[]);
        assert!(rows.iter().all(|r| r.coverage == Coverage::Uncovered));

        // The empty base composes a valid selector (`v1/…`, no leading
        // slash) instead of silently dropping every family.
        let rows = state_coverage(&slices, "", &[storage("latest", "v1/*/state/**")]);
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|r| matches!(r.coverage, Coverage::Covered(_)))
        );
    }
}
