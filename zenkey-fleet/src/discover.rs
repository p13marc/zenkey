//! Deployment-base discovery — the sweep behind `zenctl base list`.
//!
//! An observer that does not yet know a deployment's base can recover the
//! bases in use from the wire itself (RFC 09 §5: this is exactly what an
//! un-namespaced session is for). Two independent signals:
//!
//! * **liveliness tokens** (RFC 04 §5): every producer holds
//!   `<base>/v1/<origin>/state/<producer>/alive`, so a sweep with the base
//!   wildcarded finds every fleet that is *up* — including one on the empty
//!   base (a wire whose keys start at `v1/`, off-convention for a deployment
//!   per RFC 03 §1.1 but precisely what a debug tool must be able to name);
//! * **storage configs** (router admin space): a storage's `key_expr` /
//!   `strip_prefix` names the base it captures, so a configured-but-idle
//!   deployment is still discoverable while its producers are down.
//!
//! Blind spot, stated honestly: `*`/`**` never match a verbatim `@` chunk
//! (property D4), so service origins are only swept for the well-known
//! `@catalog` by name. A base populated *only* by other service origins, with
//! no host producers and no storage config, is not discoverable here.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use anyhow::Result;
use serde::Serialize;
use zenkey::grammar::{self, ClassOrPlane, SUBJECT_ALIVE, VERSION_CHUNK};
use zenoh::Session;

use crate::admin::StorageInfo;

/// Host-form sweep: `**` matches zero or more chunks, so every base depth is
/// covered — including the empty base. A verbatim origin is never matched
/// (D4), hence the separate catalog sweep.
const HOST_ALIVE_SWEEP: &str = "**/v1/*/state/*/alive";
/// `@catalog` asked for by name at any base depth, mirroring [`crate::roster`]
/// (a unit test pins this against the typed `selector::service_alive`).
const CATALOG_ALIVE_SWEEP: &str = "**/v1/@catalog/state/alive";

/// One alive token attributed to its base — the pure result of
/// [`parse_alive_key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliveToken {
    /// `""` is the empty base.
    pub base: String,
    /// The origin chunk (`h-…` or `@<service>`).
    pub origin: String,
    /// `None` for service tokens — the service *is* the producer (RFC 03 §1.5).
    pub producer: Option<String>,
}

/// Attribute a wire liveliness key to its base.
///
/// Fixed-arity from the right — the host form has a 5-chunk tail
/// (`v1/<origin>/state/<producer>/alive`), the service form a 4-chunk tail
/// (`v1/@<svc>/state/alive`) — with the tail validated by [`grammar::parse`].
/// Never a "find the first `v1`" scan, so a base that itself contains a
/// literal `v1` chunk attributes correctly. The two forms cannot collide:
/// `v1` is not a valid origin chunk, so at most one tail parses.
pub fn parse_alive_key(key: &str) -> Option<AliveToken> {
    let chunks: Vec<&str> = key.split('/').collect();
    // Host form (tail 5), then service form (tail 4).
    for tail_len in [5usize, 4] {
        let Some(split) = chunks.len().checked_sub(tail_len) else {
            continue;
        };
        if chunks[split] != VERSION_CHUNK {
            continue;
        }
        let tail = chunks[split..].join("/");
        let Ok(parsed) = grammar::parse(&tail) else {
            continue;
        };
        if !matches!(parsed.class, ClassOrPlane::Class(grammar::Class::State))
            || parsed.subject != [SUBJECT_ALIVE]
        {
            continue;
        }
        // The 5-chunk tail is the host form (producer present), the 4-chunk
        // tail the service form (no producer chunk).
        if (tail_len == 5) != parsed.producer.is_some() {
            continue;
        }
        return Some(AliveToken {
            base: chunks[..split].join("/"),
            origin: parsed.origin.chunk().to_string(),
            producer: parsed.producer.as_ref().map(|p| p.chunk()),
        });
    }
    None
}

/// The base a storage config names, if it names one.
///
/// `raw["strip_prefix"]` ending in a `v1` chunk is exact; otherwise the
/// all-literal prefix of `key_expr` up to its first `v1` chunk (a heuristic —
/// a base whose *own* trailing chunk is literally `v1` is ambiguous here,
/// which is why `strip_prefix` wins when present). `None` when a wildcard or
/// verbatim chunk precedes `v1`.
pub fn base_of_storage(storage: &StorageInfo) -> Option<String> {
    if let Some(prefix) = storage.raw.get("strip_prefix").and_then(|v| v.as_str()) {
        if prefix == VERSION_CHUNK {
            return Some(String::new());
        }
        if let Some(base) = prefix.strip_suffix("/v1") {
            return Some(base.to_string());
        }
        // A strip_prefix not ending at the v1 boundary tells us nothing;
        // fall through to the key_expr heuristic.
    }
    let key_expr = storage.key_expr.as_deref()?;
    let mut base_chunks: Vec<&str> = Vec::new();
    for chunk in key_expr.split('/') {
        if chunk == VERSION_CHUNK {
            return Some(base_chunks.join("/"));
        }
        if chunk.contains('*') || chunk.starts_with('@') {
            return None;
        }
        base_chunks.push(chunk);
    }
    None
}

/// One discovered base and the evidence for it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DiscoveredBase {
    /// `""` is the empty base (keys start at `v1/` on the wire).
    pub base: String,
    /// Origins holding alive tokens under this base.
    pub origins: BTreeSet<String>,
    /// Producer names alive under this base.
    pub producers: BTreeSet<String>,
    /// Storages whose config names this base, as `name@zid`.
    pub storages: Vec<String>,
}

/// Merge the two signals into sorted, deduped rows (the empty base sorts
/// first). Pure — [`discover_bases`] is the thin session wrapper.
pub fn merge_signals(
    tokens: impl IntoIterator<Item = AliveToken>,
    storages: &[StorageInfo],
) -> Vec<DiscoveredBase> {
    let mut bases: BTreeMap<String, DiscoveredBase> = BTreeMap::new();
    fn entry<'m>(
        bases: &'m mut BTreeMap<String, DiscoveredBase>,
        base: &str,
    ) -> &'m mut DiscoveredBase {
        bases
            .entry(base.to_string())
            .or_insert_with(|| DiscoveredBase {
                base: base.to_string(),
                ..DiscoveredBase::default()
            })
    }
    for token in tokens {
        let row = entry(&mut bases, &token.base);
        // A service token's producer is the service itself (RFC 03 §1.5).
        let producer = token
            .producer
            .unwrap_or_else(|| token.origin.trim_start_matches('@').to_string());
        row.origins.insert(token.origin);
        row.producers.insert(producer);
    }
    for storage in storages {
        let Some(base) = base_of_storage(storage) else {
            continue;
        };
        entry(&mut bases, &base)
            .storages
            .push(format!("{}@{}", storage.name, storage.zid));
    }
    for row in bases.values_mut() {
        row.storages.sort();
        row.storages.dedup();
    }
    bases.into_values().collect()
}

/// The sweep: the two liveliness gets plus the router storage configs, merged
/// by [`merge_signals`]. Best-effort throughout — a peer-only mesh (no admin
/// space) or a failed selector narrows the evidence, never errors. Zero rows
/// is *not* proof of an empty mesh (RFC 05 §3.1): the caller renders that
/// silence honestly.
pub async fn discover_bases(session: &Session, timeout: Duration) -> Result<Vec<DiscoveredBase>> {
    let mut tokens = Vec::new();
    for sweep in [HOST_ALIVE_SWEEP, CATALOG_ALIVE_SWEEP] {
        let Ok(replies) = session.liveliness().get(sweep).timeout(timeout).await else {
            continue;
        };
        while let Ok(reply) = replies.recv_async().await {
            let Ok(sample) = reply.result() else { continue };
            if let Some(token) = parse_alive_key(sample.key_expr().as_str()) {
                tokens.push(token);
            }
        }
    }
    let storages = crate::admin::storages(session, timeout)
        .await
        .unwrap_or_default();
    Ok(merge_signals(tokens, &storages))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(base: &str, origin: &str, producer: Option<&str>) -> AliveToken {
        AliveToken {
            base: base.into(),
            origin: origin.into(),
            producer: producer.map(str::to_string),
        }
    }

    #[test]
    fn alive_keys_attribute_by_fixed_arity() {
        // Host form at every base depth, including the empty base.
        assert_eq!(
            parse_alive_key("v1/h-3fa9c2d41b7e/state/sysinfo/alive"),
            Some(token("", "h-3fa9c2d41b7e", Some("sysinfo")))
        );
        assert_eq!(
            parse_alive_key("zensight/v1/h-3fa9c2d41b7e/state/sysinfo/alive"),
            Some(token("zensight", "h-3fa9c2d41b7e", Some("sysinfo")))
        );
        assert_eq!(
            parse_alive_key("acme/fleet-a/v1/h-aaaaaaaaaaaa/state/netring/alive"),
            Some(token("acme/fleet-a", "h-aaaaaaaaaaaa", Some("netring")))
        );
        // Fixed arity from the right: a base containing a literal `v1` chunk
        // attributes correctly (a "first v1" scan would split too early).
        assert_eq!(
            parse_alive_key("acme/v1/v1/h-3fa9c2d41b7e/state/tc/alive"),
            Some(token("acme/v1", "h-3fa9c2d41b7e", Some("tc")))
        );
        // Service form (4-chunk tail, no producer) at each depth.
        assert_eq!(
            parse_alive_key("v1/@catalog/state/alive"),
            Some(token("", "@catalog", None))
        );
        assert_eq!(
            parse_alive_key("acme/v1/v1/@catalog/state/alive"),
            Some(token("acme/v1", "@catalog", None))
        );
    }

    #[test]
    fn alive_key_rejects_foreign_shapes() {
        for key in [
            "other/junk/alive",                                   // no v1 tail
            "alive",                                              // too short
            "zensight/v1/notanorigin/state/p/alive",              // invalid origin
            "zensight/v1/h-3fa9c2d41b7e/telemetry/p/alive",       // wrong class
            "zensight/v2/h-3fa9c2d41b7e/state/p/alive",           // wrong version
            "zensight/v1/h-3fa9c2d41b7e/state/p/health",          // not an alive leaf
            "zensight/v1/h-3fa9c2d41b7e/state/p/device/d0/alive", // device form (extra arity)
        ] {
            assert_eq!(parse_alive_key(key), None, "{key}");
        }
    }

    #[test]
    fn catalog_sweep_pins_to_the_typed_builder() {
        assert_eq!(
            CATALOG_ALIVE_SWEEP,
            format!(
                "**/{}",
                zenkey::selector::service_alive(&zenkey::ServiceOrigin::catalog())
            )
        );
        assert_eq!(
            HOST_ALIVE_SWEEP,
            format!(
                "**/{}",
                zenkey::selector::all_liveliness(zenkey::selector::Scope::fleet())
            )
        );
    }

    fn storage(strip_prefix: Option<&str>, key_expr: Option<&str>) -> StorageInfo {
        StorageInfo {
            zid: "z1".into(),
            name: "latest".into(),
            key_expr: key_expr.map(str::to_string),
            raw: match strip_prefix {
                Some(p) => serde_json::json!({ "strip_prefix": p }),
                None => serde_json::Value::Null,
            },
        }
    }

    #[test]
    fn storage_bases_prefer_strip_prefix() {
        assert_eq!(
            base_of_storage(&storage(Some("zensight/v1"), None)),
            Some("zensight".into())
        );
        assert_eq!(base_of_storage(&storage(Some("v1"), None)), Some("".into()));
        // strip_prefix wins over key_expr when both are present.
        assert_eq!(
            base_of_storage(&storage(Some("acme/fleet-a/v1"), Some("other/v1/**"))),
            Some("acme/fleet-a".into())
        );
        // A strip_prefix not ending at the v1 boundary falls back to key_expr.
        assert_eq!(
            base_of_storage(&storage(Some("zensight"), Some("zensight/v1/*/state/**"))),
            Some("zensight".into())
        );
        // key_expr heuristic alone.
        assert_eq!(
            base_of_storage(&storage(None, Some("acme/fleet-a/v1/*/state/**"))),
            Some("acme/fleet-a".into())
        );
        assert_eq!(
            base_of_storage(&storage(None, Some("v1/*/state/**"))),
            Some("".into())
        );
        // A wildcard or verbatim chunk before v1 names no single base.
        assert_eq!(base_of_storage(&storage(None, Some("**"))), None);
        assert_eq!(base_of_storage(&storage(None, Some("*/v1/**"))), None);
        assert_eq!(base_of_storage(&storage(None, None)), None);
    }

    #[test]
    fn signals_merge_sorted_and_deduped() {
        let tokens = vec![
            token("zensight", "h-3fa9c2d41b7e", Some("sysinfo")),
            token("zensight", "h-aaaaaaaaaaaa", Some("sysinfo")), // dedup producer
            token("zensight", "@catalog", None),                  // service: producer = catalog
            token("", "h-3fa9c2d41b7e", Some("tc")),
        ];
        let storages = [
            storage(Some("zensight/v1"), None),
            storage(Some("zensight/v1"), None), // dedup name@zid
            storage(Some("acme/v1"), None),     // storage-only base
        ];
        let rows = merge_signals(tokens, &storages);
        // The empty base sorts first; output is deterministic.
        let bases: Vec<&str> = rows.iter().map(|r| r.base.as_str()).collect();
        assert_eq!(bases, vec!["", "acme", "zensight"]);
        let zs = &rows[2];
        assert_eq!(zs.origins.len(), 3);
        assert_eq!(
            zs.producers.iter().collect::<Vec<_>>(),
            vec!["catalog", "sysinfo"]
        );
        assert_eq!(zs.storages, vec!["latest@z1"]);
        let acme = &rows[1];
        assert!(acme.origins.is_empty(), "storage-only base has no origins");
        assert_eq!(acme.storages, vec!["latest@z1"]);
    }
}
