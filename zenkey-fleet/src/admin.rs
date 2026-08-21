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
    /// The literal prefix stripped before the volume sees a key (RFC 09 §2 —
    /// zenoh requires a wildcard-free prefix here). Absent when the layout
    /// does not say, which is not the same as "none configured".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strip_prefix: Option<String>,
    /// The backing volume's id — `memory` is volatile and loses late-joiner
    /// seeds on a router restart, `fs`/`rocksdb` are the durable LWW stores
    /// (RFC 09 §2). Spelled either as a bare string or as `{ id: "fs", … }`
    /// depending on version; both are absorbed here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<String>,
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
    let text = |field: &str| {
        value
            .get(field)
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    // The volume is a bare string in some layouts and an object with an `id`
    // in others. Absorbing both here is what this pure parser is for.
    let volume = text("volume").or_else(|| {
        value
            .get("volume")
            .and_then(|v| v.get("id"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    });
    Some(StorageInfo {
        zid: (*zid).to_string(),
        name: (*name).to_string(),
        key_expr: text("key_expr"),
        strip_prefix: text("strip_prefix"),
        volume,
        raw: value.clone(),
    })
}

/// One row per `(zid, name)`, merging field by field.
///
/// The config and status subtrees both answer a storage sweep, and neither is
/// reliably the richer one — so a row that names a `key_expr` and a row that
/// names a `volume` must combine rather than one winning outright. Pure, and
/// separated from [`storages`] because with three optional fields a hand-rolled
/// `dedup_by` is where a quietly-dropped field would hide.
pub fn merge_storage_rows(mut rows: Vec<StorageInfo>) -> Vec<StorageInfo> {
    rows.sort_by(|a, b| (&a.zid, &a.name).cmp(&(&b.zid, &b.name)));
    let mut out: Vec<StorageInfo> = Vec::with_capacity(rows.len());
    for row in rows {
        match out.last_mut() {
            Some(prev) if prev.zid == row.zid && prev.name == row.name => {
                prev.key_expr = prev.key_expr.take().or(row.key_expr);
                prev.strip_prefix = prev.strip_prefix.take().or(row.strip_prefix);
                prev.volume = prev.volume.take().or(row.volume);
                // Keep the document that said more, so the raw disclosure is
                // the useful one.
                if prev.raw.as_object().map(|o| o.len()).unwrap_or(0)
                    < row.raw.as_object().map(|o| o.len()).unwrap_or(0)
                {
                    prev.raw = row.raw;
                }
            }
            _ => out.push(row),
        }
    }
    out
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
    let rows: Vec<StorageInfo> = entries
        .iter()
        .filter_map(|e| storage_from_admin_entry(&e.key, &e.value))
        .collect();
    Ok(merge_storage_rows(rows))
}

/// How a declared state family relates to the configured storages.
///
/// `rename_all` is not decoration: without it this enum inherited Rust's
/// variant spelling and serialized `"Covered"` while every other vocabulary in
/// the report surface — `TopicVerdict`, `DoctorSeverity`, `CutoverVerdict`,
/// `ExpectVerdict` — was snake_case (#232). A consumer could not learn the
/// file's conventions from one document and apply them to the next.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "coverage", content = "storage", rename_all = "snake_case")]
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

/// What kind of declared entity an admin reply describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Subscriber,
    Publisher,
    Queryable,
    Querier,
    Token,
}

impl EntityKind {
    fn from_chunk(chunk: &str) -> Option<EntityKind> {
        Some(match chunk {
            "subscriber" => EntityKind::Subscriber,
            "publisher" => EntityKind::Publisher,
            "queryable" => EntityKind::Queryable,
            "querier" => EntityKind::Querier,
            "token" => EntityKind::Token,
            _ => return None,
        })
    }

    fn chunk(self) -> &'static str {
        match self {
            EntityKind::Subscriber => "subscriber",
            EntityKind::Publisher => "publisher",
            EntityKind::Queryable => "queryable",
            EntityKind::Querier => "querier",
            EntityKind::Token => "token",
        }
    }
}

/// One declared entity, as the admin space reports it: the reply key is
/// `@/<zid>/<whatami>/<kind>/<declared-keyexpr...>`, so the keyexpr every
/// session declared is readable **without subscribing to any data** — the
/// payload-free discovery leg of issue #84.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DeclaredEntity {
    pub kind: EntityKind,
    /// The declared key expression, verbatim.
    pub keyexpr: String,
    /// The node whose admin space answered.
    pub node_zid: String,
    /// The raw payload (`Sources { routers, peers, clients }`-shaped in
    /// zenoh 1.9) — kept as-is; layouts vary by version.
    pub sources: serde_json::Value,
}

/// The declared-entity sweep result.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DeclaredEntities {
    pub entities: Vec<DeclaredEntity>,
}

/// Parse one admin entry (`@/<zid>/<whatami>/<kind>/<keyexpr...>`) into a
/// declared entity. Pure; unknown shapes yield `None` (the admin space is
/// version-dependent surface — tolerate, never fail).
pub fn declared_from_admin_entry(key: &str, value: &serde_json::Value) -> Option<DeclaredEntity> {
    let mut chunks = key.split('/');
    if chunks.next()? != "@" {
        return None;
    }
    let zid = chunks.next()?;
    let _whatami = chunks.next()?;
    let kind = EntityKind::from_chunk(chunks.next()?)?;
    let keyexpr: Vec<&str> = chunks.collect();
    if keyexpr.is_empty() {
        return None;
    }
    Some(DeclaredEntity {
        kind,
        keyexpr: keyexpr.join("/"),
        node_zid: zid.to_string(),
        sources: value.clone(),
    })
}

/// Enumerate declared subscribers/publishers/queryables/tokens from every
/// reachable admin space.
///
/// `Ok(None)` when **nothing answered at all**: zenoh's `adminspace.enabled`
/// defaults to *false* (routers ship with it on; a pure peer mesh has none),
/// so an empty sweep means "not available", never "nothing declared" —
/// callers MUST render the difference (RFC 09 §5.1 O4). A *publisher* is
/// visible only if it was declared (P7's rule for the data planes); a bare
/// `session.put()` never appears here.
pub async fn declared_entities(
    session: &Session,
    timeout: Duration,
) -> Result<Option<DeclaredEntities>> {
    let mut entities = Vec::new();
    let mut any_reply = false;
    for kind in [
        EntityKind::Subscriber,
        EntityKind::Publisher,
        EntityKind::Queryable,
        EntityKind::Querier,
        EntityKind::Token,
    ] {
        let selector = format!("@/*/*/{}/**", kind.chunk());
        let entries = admin_get(session, &selector, timeout).await?;
        any_reply |= !entries.is_empty();
        entities.extend(
            entries
                .iter()
                .filter_map(|e| declared_from_admin_entry(&e.key, &e.value)),
        );
    }
    if !any_reply {
        return Ok(None);
    }
    Ok(Some(DeclaredEntities { entities }))
}

/// One link of the mesh, seen as undirected.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MeshLink {
    /// The lower zid of the pair — the ordering is arbitrary but stable, so a
    /// renderer can group without re-sorting.
    pub a: String,
    pub b: String,
    /// Both ends reported this link. Reciprocal reports are corroboration,
    /// not duplication, and the distinction is worth keeping: a link only one
    /// end mentions is weaker evidence than one both do.
    pub corroborated: bool,
    /// Endpoints as the **first** reporter described them. A second report's
    /// links are not merged: the two ends name the same link from opposite
    /// sides, and concatenating them would read as twice the links.
    pub links: Vec<String>,
}

/// Collapse the per-reporter edges into undirected links.
///
/// [`TopologyEdge`]'s own doc has anticipated this since it was written — "a
/// renderer that wants an undirected mesh dedups by unordered zid pair" — and
/// two renderers now want it, which is why it stopped being a private helper
/// in the CLI (issue #207).
pub fn mesh_links(report: &TopologyReport) -> Vec<MeshLink> {
    let mut out: Vec<MeshLink> = Vec::new();
    for e in &report.edges {
        let (a, b) = if e.reporter <= e.peer {
            (e.reporter.clone(), e.peer.clone())
        } else {
            (e.peer.clone(), e.reporter.clone())
        };
        match out.iter_mut().find(|l| l.a == a && l.b == b) {
            Some(link) => link.corroborated = true,
            None => out.push(MeshLink {
                a,
                b,
                corroborated: false,
                links: e.links.clone(),
            }),
        }
    }
    out
}

/// Graphviz, self-contained: routers as boxes, peers/clients as ellipses,
/// heard-of nodes dashed, our own session bold.
pub fn render_dot(report: &TopologyReport, attachments: &[OriginAttachment]) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("graph zenoh_mesh {\n");
    for n in &report.nodes {
        let shape = if n.whatami == "router" {
            "box"
        } else {
            "ellipse"
        };
        let mut style = Vec::new();
        if !n.answered {
            style.push("dashed");
        }
        if n.zid == report.self_zid {
            style.push("bold");
        }
        let label = match (&n.answered, &n.version) {
            (false, _) => format!("{}\\n{} (heard of)", n.zid, n.whatami),
            (true, Some(v)) => format!("{}\\n{} {v}", n.zid, n.whatami),
            (true, None) => format!("{}\\n{}", n.zid, n.whatami),
        };
        let _ = writeln!(
            out,
            "  \"{}\" [shape={shape}, label=\"{label}\"{}];",
            n.zid,
            if style.is_empty() {
                String::new()
            } else {
                format!(", style=\"{}\"", style.join(","))
            }
        );
    }
    for link in mesh_links(report) {
        let proto = link
            .links
            .first()
            .and_then(|l| l.split('/').next())
            .unwrap_or("");
        let _ = writeln!(
            out,
            "  \"{}\" -- \"{}\"{};",
            link.a,
            link.b,
            if proto.is_empty() {
                String::new()
            } else {
                format!(" [label=\"{proto}\"]")
            }
        );
    }
    // Origins as their own small nodes (#131): attached by a solid edge to
    // the session the admin sources named, or by a dotted one to the mere
    // reporter — the picture keeps the evidence distinction the join made.
    for (i, a) in attachments.iter().enumerate() {
        let id = format!("origin_{i}");
        let _ = writeln!(
            out,
            "  \"{id}\" [shape=hexagon, label=\"{}\", fontsize=10];",
            a.origin
        );
        match &a.session_zid {
            Some(z) => {
                let _ = writeln!(out, "  \"{id}\" -- \"{z}\";");
            }
            None => {
                let _ = writeln!(
                    out,
                    "  \"{id}\" -- \"{}\" [style=dotted, label=\"reported\"];",
                    a.reporter_zid
                );
            }
        }
    }
    out.push('}');
    out
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
        assert_eq!(s.volume.as_deref(), Some("fs"), "a bare-string volume");
        // The other spelling of the same field.
        let v = serde_json::json!({
            "key_expr": "zs/v1/*/state/**",
            "strip_prefix": "zs/v1",
            "volume": {"id": "rocksdb", "dir": "latest"},
        });
        let s = storage_from_admin_entry(
            "@/abc123/router/config/plugins/storage_manager/storages/durable",
            &v,
        )
        .unwrap();
        assert_eq!(s.volume.as_deref(), Some("rocksdb"), "an object volume");
        assert_eq!(s.strip_prefix.as_deref(), Some("zs/v1"));
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

    /// Config and status subtrees both answer, neither is reliably richer, and
    /// a field named by only one of them must survive either arrival order.
    #[test]
    fn merging_a_storage_keeps_every_field_either_side_named() {
        let config = StorageInfo {
            zid: "z1".into(),
            name: "latest".into(),
            key_expr: Some("zs/**".into()),
            strip_prefix: Some("zs".into()),
            volume: None,
            raw: serde_json::json!({"key_expr": "zs/**", "strip_prefix": "zs"}),
        };
        let status = StorageInfo {
            zid: "z1".into(),
            name: "latest".into(),
            key_expr: None,
            strip_prefix: None,
            volume: Some("fs".into()),
            raw: serde_json::json!({"volume": "fs"}),
        };
        for rows in [
            vec![config.clone(), status.clone()],
            vec![status, config.clone()],
        ] {
            let merged = merge_storage_rows(rows);
            assert_eq!(merged.len(), 1, "one row per (zid, name)");
            assert_eq!(merged[0].key_expr.as_deref(), Some("zs/**"));
            assert_eq!(merged[0].strip_prefix.as_deref(), Some("zs"));
            assert_eq!(merged[0].volume.as_deref(), Some("fs"));
        }

        // Two names under one zid stay two rows.
        let other = StorageInfo {
            zid: "z1".into(),
            name: "history".into(),
            key_expr: None,
            strip_prefix: None,
            volume: None,
            raw: serde_json::Value::Null,
        };
        assert_eq!(merge_storage_rows(vec![config, other]).len(), 2);
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
            strip_prefix: None,
            volume: None,
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

    /// The zenoh 1.9 admin key shape (`@/<zid>/<whatami>/<kind>/<keyexpr...>`)
    /// parses into a declared entity; foreign shapes are tolerated as None.
    #[test]
    fn declared_entities_parse_the_admin_key_shape() {
        let v = serde_json::json!({"routers": [], "peers": ["p1"], "clients": []});
        let e = declared_from_admin_entry("@/a1b2c3/router/subscriber/zensight/v1/*/state/**", &v)
            .unwrap();
        assert_eq!(e.kind, EntityKind::Subscriber);
        assert_eq!(e.keyexpr, "zensight/v1/*/state/**");
        assert_eq!(e.node_zid, "a1b2c3");

        let e = declared_from_admin_entry("@/z/peer/publisher/v1/h-a/telemetry/x/m", &v).unwrap();
        assert_eq!(e.kind, EntityKind::Publisher);
        assert_eq!(e.keyexpr, "v1/h-a/telemetry/x/m");

        // Tolerated, never fatal:
        assert!(declared_from_admin_entry("@/z/router/config/x", &v).is_none());
        assert!(declared_from_admin_entry("@/z/router/subscriber", &v).is_none());
        assert!(declared_from_admin_entry("not/admin/at/all", &v).is_none());
    }

    fn report() -> TopologyReport {
        TopologyReport {
            nodes: vec![
                TopologyNode {
                    zid: "aaa".into(),
                    whatami: "router".into(),
                    version: Some("1.9.0".into()),
                    locators: vec!["tcp/10.0.0.1:7447".into()],
                    answered: true,
                },
                TopologyNode {
                    zid: "bbb".into(),
                    whatami: "peer".into(),
                    version: None,
                    locators: vec![],
                    answered: false,
                },
            ],
            edges: vec![
                TopologyEdge {
                    reporter: "aaa".into(),
                    peer: "bbb".into(),
                    whatami: "peer".into(),
                    links: vec!["tcp/10.0.0.1:7447 -> tcp/10.0.0.2:53210".into()],
                },
                TopologyEdge {
                    reporter: "bbb".into(),
                    peer: "aaa".into(),
                    whatami: "router".into(),
                    links: vec![],
                },
            ],
            asked: "@/*/*".into(),
            answered: 1,
            self_zid: "bbb".into(),
        }
    }

    /// Reciprocal reports collapse to one undirected edge, marked as
    /// corroborated — not drawn twice, not silently merged.
    #[test]
    fn reciprocal_reports_corroborate_one_edge() {
        let edges = mesh_links(&report());
        assert_eq!(edges.len(), 1);
        let link = &edges[0];
        assert_eq!((link.a.as_str(), link.b.as_str()), ("aaa", "bbb"));
        assert!(link.corroborated, "both ends reported it");
    }

    /// The DOT form: routers boxed, heard-of nodes dashed, our session
    /// bold, edges labeled by protocol — pipeable to `dot -Tsvg` as-is.
    #[test]
    fn the_dot_form_marks_what_the_join_knows() {
        let dot = render_dot(&report(), &[]);
        assert!(dot.starts_with("graph zenoh_mesh {"), "{dot}");
        assert!(dot.contains("\"aaa\" [shape=box"), "{dot}");
        assert!(dot.contains("heard of"), "{dot}");
        assert!(dot.contains("style=\"dashed,bold\""), "{dot}");
        assert!(dot.contains("\"aaa\" -- \"bbb\" [label=\"tcp\"]"), "{dot}");
        assert!(dot.ends_with('}'), "{dot}");
    }

    /// The origin overlay (#131): a sources-named attachment is a solid
    /// edge; a reporter-only one is dotted and says "reported" — the DOT
    /// keeps the evidence distinction the join made.
    #[test]
    fn the_dot_form_keeps_the_attachment_evidence_distinction() {
        let attachments = vec![
            OriginAttachment {
                origin: "h-cccccccccccc".into(),
                session_zid: Some("bbb".into()),
                reporter_zid: "aaa".into(),
                token_key: "v1/h-cccccccccccc/state/demo/alive".into(),
            },
            OriginAttachment {
                origin: "h-dddddddddddd".into(),
                session_zid: None,
                reporter_zid: "aaa".into(),
                token_key: "v1/h-dddddddddddd/state/demo/alive".into(),
            },
        ];
        let dot = render_dot(&report(), &attachments);
        assert!(dot.contains("label=\"h-cccccccccccc\""), "{dot}");
        assert!(dot.contains("\"origin_0\" -- \"bbb\";"), "{dot}");
        assert!(
            dot.contains("\"origin_1\" -- \"aaa\" [style=dotted, label=\"reported\"]"),
            "{dot}"
        );
    }
}

/// One liveliness origin attached to the session that declared its token —
/// the #131 join, evidence-first: an attachment is made only from what the
/// admin space actually said, never guessed (a guessed attachment would be
/// the O4 failure on a picture).
#[derive(Debug, Clone, serde::Serialize)]
pub struct OriginAttachment {
    /// The origin the token names (`h-…` or `@service`).
    pub origin: String,
    /// The declaring session's zid, when the token's admin `sources` names
    /// exactly one. `None` = the sources were absent or ambiguous — the
    /// origin is then only *reported by* the answering admin space, and a
    /// renderer says so instead of drawing a line it cannot back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_zid: Option<String>,
    /// The admin space that reported the token: the origin's own session in
    /// a peer mesh serving its admin space, a router in a routed one.
    pub reporter_zid: String,
    /// The token key the evidence rode — the audit trail.
    pub token_key: String,
}

/// Collect every zid string under the zenoh 1.9 `Sources` shape
/// (`{ routers: [...], peers: [...], clients: [...] }`) — tolerant of the
/// layout varying by version: unknown shapes yield nothing, never an error.
fn source_zids(sources: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    for kind in ["routers", "peers", "clients"] {
        if let Some(list) = sources.get(kind).and_then(|v| v.as_array()) {
            out.extend(list.iter().filter_map(|z| z.as_str().map(str::to_string)));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Join the admin space's declared liveliness tokens against the keyspace:
/// which origin hangs off which session (#131).
///
/// One `@/*/*/token/**` sweep; each token whose keyexpr parses under `base`
/// as an `alive` leaf yields an attachment. The session zid is taken from
/// the token's `sources` **only when they name exactly one** — several
/// candidates or none degrade to reporter-only, stated rather than guessed.
/// An empty result means the admin space served no tokens (or none parse
/// under this base) — an observation, not an empty fleet (O4).
pub async fn origin_attachments(
    session: &Session,
    base: &str,
    timeout: Duration,
) -> Result<Vec<OriginAttachment>> {
    let entries = admin_get(session, "@/*/*/token/**", timeout).await?;
    let mut out: Vec<OriginAttachment> = Vec::new();
    for e in &entries {
        let Some(decl) = declared_from_admin_entry(&e.key, &e.value) else {
            continue;
        };
        if decl.kind != EntityKind::Token {
            continue;
        }
        let Some(parsed) = zenkey::grammar::parse_full(base, &decl.keyexpr) else {
            continue;
        };
        // The framework liveliness shape: an `alive` leaf on the state
        // class (RFC 04 §5). Anything else declared as a token is not an
        // origin claim and is left alone.
        if parsed.subject.last().copied() != Some("alive") {
            continue;
        }
        let origin = parsed.origin.chunk().to_string();
        let zids = source_zids(&decl.sources);
        let session_zid = match zids.as_slice() {
            [only] => Some(only.clone()),
            _ => None,
        };
        let attachment = OriginAttachment {
            origin,
            session_zid,
            reporter_zid: decl.node_zid.clone(),
            token_key: decl.keyexpr.clone(),
        };
        // One origin can hold several sessions (one per producer process);
        // dedup only exact repeats.
        if !out.iter().any(|a| {
            a.origin == attachment.origin
                && a.session_zid == attachment.session_zid
                && a.reporter_zid == attachment.reporter_zid
        }) {
            out.push(attachment);
        }
    }
    Ok(out)
}

/// One node of the mesh, as the topology join sees it (#118).
#[derive(Debug, Clone, serde::Serialize)]
pub struct TopologyNode {
    pub zid: String,
    /// `router` | `peer` | `client`, as the admin key (or a neighbour's
    /// session list) spells it.
    pub whatami: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub locators: Vec<String>,
    /// `true` = this node's own admin space answered; `false` = only heard
    /// of via a neighbour's session list — "heard of, not queryable",
    /// rendered as such rather than omitted (the issue's honesty rule).
    pub answered: bool,
}

/// One reported link. Kept per-reporter — a renderer that wants an
/// undirected mesh dedups by unordered zid pair, and reciprocal reports
/// are corroboration, not duplication.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TopologyEdge {
    /// The zid whose admin doc reported this session.
    pub reporter: String,
    /// The far end's zid.
    pub peer: String,
    /// The far end's whatami, as the reporter says it.
    pub whatami: String,
    /// Link endpoints, `src -> dst`, protocol included.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
}

/// The mesh as the admin space answered it, joined with nothing invented
/// (#118): what answered, what was only mentioned, and who we are.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TopologyReport {
    pub nodes: Vec<TopologyNode>,
    pub edges: Vec<TopologyEdge>,
    /// The selector the sweep asked.
    pub asked: String,
    /// Root docs that answered. Zero is "the admin space did not answer" —
    /// a reading about reachability, never an empty mesh.
    pub answered: usize,
    /// This session's own zid — the "you are here" marker.
    pub self_zid: String,
}

/// Join the admin root docs (`@/<zid>/<whatami>`) into a topology: every
/// answering node with its locators and version, every session it reports
/// as an edge, and every zid that is *only* mentioned as a
/// heard-of-not-queryable node.
pub async fn topology(session: &Session, timeout: Duration) -> Result<TopologyReport> {
    const ASKED: &str = "@/*/*";
    let entries = admin_get(session, ASKED, timeout).await?;
    let mut nodes: Vec<TopologyNode> = Vec::new();
    let mut edges: Vec<TopologyEdge> = Vec::new();
    for e in &entries {
        // Root docs only: @/<zid>/<whatami>. Anything deeper is a
        // different handler and not a node document.
        let mut chunks = e.key.split('/');
        let (Some("@"), Some(zid), Some(whatami), None) =
            (chunks.next(), chunks.next(), chunks.next(), chunks.next())
        else {
            continue;
        };
        let doc = &e.value;
        nodes.push(TopologyNode {
            zid: doc
                .get("zid")
                .and_then(|v| v.as_str())
                .unwrap_or(zid)
                .to_string(),
            whatami: whatami.to_string(),
            version: doc
                .get("version")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            locators: doc
                .get("locators")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|l| l.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            answered: true,
        });
        for s in doc
            .get("sessions")
            .and_then(|v| v.as_array())
            .map(|a| a.as_slice())
            .unwrap_or_default()
        {
            let Some(peer) = s.get("peer").and_then(|v| v.as_str()) else {
                continue;
            };
            edges.push(TopologyEdge {
                reporter: zid.to_string(),
                peer: peer.to_string(),
                whatami: s
                    .get("whatami")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                links: s
                    .get("links")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|l| {
                                Some(format!(
                                    "{} -> {}",
                                    l.get("src")?.as_str()?,
                                    l.get("dst")?.as_str()?
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            });
        }
    }
    let answered = nodes.len();
    // Heard-of nodes: mentioned as a session peer, but no root doc answered
    // for them (admin space off, or out of reach). Shown, never omitted.
    for e in &edges {
        if !nodes.iter().any(|n| n.zid == e.peer) {
            nodes.push(TopologyNode {
                zid: e.peer.clone(),
                whatami: e.whatami.clone(),
                version: None,
                locators: Vec::new(),
                answered: false,
            });
        }
    }
    nodes.sort_by(|a, b| a.zid.cmp(&b.zid));
    nodes.dedup_by(|a, b| a.zid == b.zid);
    Ok(TopologyReport {
        nodes,
        edges,
        asked: ASKED.to_string(),
        answered,
        self_zid: session.zid().to_string(),
    })
}
