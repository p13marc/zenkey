//! The consumers join (#224): declared readers, related to a target by key
//! algebra and attributed to origins on admin evidence — from values in
//! hand, no session.
//!
//! "Who consumes this subject?" decides whether a schema change is safe,
//! and on this bus it is a join over data other sweeps already fetch:
//! [`crate::declared_entities`] enumerates every declared subscriber,
//! querier and queryable with its verbatim keyexpr per zid;
//! `zenoh-keyexpr` gives exact `includes`/`intersects`;
//! [`crate::origin_attachments`] maps zid → origin on token evidence; the
//! registry names the subject. Nothing here is matching status (RFC 12 §9,
//! deferred permanently): a declared subscriber is a declaration, not proof
//! of use, and the vocabulary says so.

use zenoh::key_expr::keyexpr;

use crate::report::{
    Attribution, ConsumerRow, DeclaredEntities, DeprecationFact, EntityKind, OriginAttachment,
    Relation, TopologyReport,
};

/// Relate every declared subscriber and querier to `target`, one row per
/// declaring session, ranked most-specific first.
///
/// * A relation is computed with `zenoh-keyexpr`: the target including the
///   declaration is [`Relation::Narrower`], the reverse [`Relation::Wider`],
///   both [`Relation::Exact`], neither but overlapping
///   [`Relation::Intersects`]; a declaration that includes the whole base
///   (`**`, or any prefix that includes `<base>/v1/**`) is
///   [`Relation::Total`] — it intersects everything and is shown as such.
///   A declaration that does not intersect the target is not a row.
/// * The zid comes from the entity's admin `sources` when they name any
///   session ([`Attribution::Session`], one row each); when they name none
///   the row carries the reporting node's zid as
///   [`Attribution::ReportedOnly`] — stated, never guessed.
/// * Origins are the attachments whose `session_zid` is the row's zid;
///   `whatami` comes from the topology's roster when it heard of the zid.
///
/// A `target` that is not a key expression yields no rows; the bus layer
/// refuses it before asking anything ([`crate::consumers`]).
pub fn join_consumers(
    target: &str,
    declared: &DeclaredEntities,
    attachments: &[OriginAttachment],
    topology: Option<&TopologyReport>,
    self_zid: &str,
) -> Vec<ConsumerRow> {
    let Ok(target_ke) = keyexpr::new(target) else {
        return Vec::new();
    };
    let base_tree = base_tree_of(target);
    let base_ke = keyexpr::new(base_tree.as_str()).ok();

    let mut rows: Vec<ConsumerRow> = Vec::new();
    for entity in &declared.entities {
        if !matches!(entity.kind, EntityKind::Subscriber | EntityKind::Querier) {
            continue;
        }
        let Ok(declared_ke) = keyexpr::new(entity.keyexpr.as_str()) else {
            // The admin surface is version-dependent; a shape this build
            // cannot read is skipped, never a failure.
            continue;
        };
        let Some(relation) = relate(target_ke, declared_ke, base_ke) else {
            continue;
        };
        let zids = crate::bus::admin::source_zids(&entity.sources);
        let attributed: Vec<(String, Attribution)> = if zids.is_empty() {
            vec![(entity.node_zid.clone(), Attribution::ReportedOnly)]
        } else {
            zids.into_iter()
                .map(|z| (z, Attribution::Session))
                .collect()
        };
        for (zid, attribution) in attributed {
            // Several admin spaces can report one declaration (a router and
            // the declaring peer both serve it): one row per (session,
            // kind, keyexpr), not one per reporter.
            if rows
                .iter()
                .any(|r| r.zid == zid && r.kind == entity.kind && r.keyexpr == entity.keyexpr)
            {
                continue;
            }
            let mut origins: Vec<String> = attachments
                .iter()
                .filter(|a| a.session_zid.as_deref() == Some(zid.as_str()))
                .map(|a| a.origin.clone())
                .collect();
            origins.sort();
            origins.dedup();
            let whatami = topology
                .and_then(|t| t.nodes.iter().find(|n| n.zid == zid))
                .map(|n| n.whatami.clone());
            rows.push(ConsumerRow {
                is_self: zid == self_zid,
                zid,
                whatami,
                origins,
                attribution,
                kind: entity.kind,
                keyexpr: entity.keyexpr.clone(),
                relation,
                total_wildcard: relation == Relation::Total,
            });
        }
    }
    rows.sort_by(|a, b| {
        a.relation
            .cmp(&b.relation)
            .then_with(|| a.keyexpr.cmp(&b.keyexpr))
            .then_with(|| a.zid.cmp(&b.zid))
    });
    rows
}

/// The relation of one declaration to the target, or `None` when the two
/// do not intersect at all.
fn relate(target: &keyexpr, declared: &keyexpr, base: Option<&keyexpr>) -> Option<Relation> {
    if !declared.intersects(target) {
        return None;
    }
    if base.is_some_and(|b| declared.includes(b)) {
        return Some(Relation::Total);
    }
    let wider = declared.includes(target);
    let narrower = target.includes(declared);
    Some(match (wider, narrower) {
        (true, true) => Relation::Exact,
        (false, true) => Relation::Narrower,
        (true, false) => Relation::Wider,
        (false, false) => Relation::Intersects,
    })
}

/// The whole-base tree the target sits in: everything up to and including
/// the `v1` chunk, then `**` (`acme/v1/**`, or `v1/**` at the bus root). A
/// target outside the grammar has no base to speak of, and only a bare `**`
/// is total for it.
fn base_tree_of(target: &str) -> String {
    let chunks: Vec<&str> = target.split('/').collect();
    match chunks.iter().position(|c| *c == "v1") {
        Some(i) => {
            let mut prefix = chunks[..=i].join("/");
            prefix.push_str("/**");
            prefix
        }
        None => "**".to_string(),
    }
}

/// Count the distinct sessions declaring an entity of `kind` that
/// intersects `target` — the `impact` report's publisher and queryable
/// columns. A session is its `sources` zids, or the reporting node when the
/// sources name none, exactly as [`join_consumers`] attributes a row.
pub fn declaring_sessions(target: &str, declared: &DeclaredEntities, kind: EntityKind) -> usize {
    let Ok(target_ke) = keyexpr::new(target) else {
        return 0;
    };
    let mut zids: Vec<String> = Vec::new();
    for entity in declared.entities.iter().filter(|e| e.kind == kind) {
        let Ok(declared_ke) = keyexpr::new(entity.keyexpr.as_str()) else {
            continue;
        };
        if !declared_ke.intersects(target_ke) {
            continue;
        }
        let sources = crate::bus::admin::source_zids(&entity.sources);
        if sources.is_empty() {
            zids.push(entity.node_zid.clone());
        } else {
            zids.extend(sources);
        }
    }
    zids.sort();
    zids.dedup();
    zids.len()
}

/// One registry subject resolved to the wire (#224): the selector its
/// family answers under `base`, its class, and its ledger entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectTarget {
    /// The class chunk; `*` for a path known only from the ledger.
    pub class: String,
    /// The full wire selector, composed through `with_base` so the empty
    /// base stays a valid keyexpr.
    pub selector: String,
    pub deprecated: Option<DeprecationFact>,
}

/// Resolve `<producer>/<path>` against the slices.
///
/// `None` when no slice names the producer, or when neither its subjects
/// nor its `[[deprecated]]` ledger carry the path. A retired subject that
/// survives only in the ledger still resolves — "who still reads it" is
/// exactly the question the ledger exists for (RFC 08 §3) — with the class
/// wildcarded, because a ledger entry declares none.
pub fn subject_target(
    slices: &crate::SliceSet,
    base: &str,
    producer: &str,
    path: &str,
) -> Option<SubjectTarget> {
    let slice = slices.get(producer)?;
    let deprecated = slice
        .deprecated
        .iter()
        .find(|d| d.path == path && d.kind == zenkey::slice::DeprecatedKind::Subject)
        .map(|d| DeprecationFact {
            since: d.since.clone(),
            replaced_by: d.replaced_by.clone(),
        });
    let subject = slice.subjects.iter().find(|s| s.path == path);
    let class = match subject {
        Some(s) => s.class.token().to_string(),
        None => {
            deprecated.as_ref()?;
            "*".to_string()
        }
    };
    let tail = zenkey::pattern::SubjectPattern::parse(path)
        .ok()?
        .selector_tail();
    // A service's keys carry no producer chunk (RFC 06 §5); a host
    // producer's origin is any host — `*` never matches a service origin
    // (RFC 03 §4 D4), which is what makes this selector the family and not
    // more.
    let relative = match &slice.service_origin {
        Some(origin) => format!("v1/{origin}/{class}/{tail}"),
        None => format!("v1/*/{class}/{}/{tail}", slice.name),
    };
    Some(SubjectTarget {
        class,
        selector: zenkey::grammar::with_base(base, relative),
        deprecated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{DeclaredEntity, TopologyNode};
    use serde_json::json;

    fn entity(
        kind: EntityKind,
        keyexpr: &str,
        node: &str,
        sources: serde_json::Value,
    ) -> DeclaredEntity {
        DeclaredEntity {
            kind,
            keyexpr: keyexpr.into(),
            node_zid: node.into(),
            sources,
        }
    }

    fn peers(zids: &[&str]) -> serde_json::Value {
        json!({ "routers": [], "peers": zids, "clients": [] })
    }

    const TARGET: &str = "v1/h-cccccccccccc/state/demo/**";

    /// The acceptance fixture: one narrow subscriber and one `**`
    /// subscriber rank narrower-then-total, and the wildcard is flagged.
    #[test]
    fn a_narrow_and_a_total_subscriber_rank_by_relation() {
        let declared = DeclaredEntities {
            entities: vec![
                entity(
                    EntityKind::Subscriber,
                    "**",
                    "router1",
                    peers(&["zid-wide"]),
                ),
                entity(
                    EntityKind::Subscriber,
                    "v1/h-cccccccccccc/state/demo/health",
                    "router1",
                    peers(&["zid-narrow"]),
                ),
                // A publisher is not a consumer.
                entity(
                    EntityKind::Publisher,
                    "v1/h-cccccccccccc/state/demo/health",
                    "router1",
                    peers(&["zid-pub"]),
                ),
                // Unrelated: not a row.
                entity(
                    EntityKind::Subscriber,
                    "v1/h-dddddddddddd/state/demo/health",
                    "router1",
                    peers(&["zid-other"]),
                ),
            ],
        };
        let attachments = vec![OriginAttachment {
            origin: "h-cccccccccccc".into(),
            session_zid: Some("zid-narrow".into()),
            reporter_zid: "router1".into(),
            token_key: "v1/h-cccccccccccc/state/demo/alive".into(),
        }];
        let rows = join_consumers(TARGET, &declared, &attachments, None, "me");
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0].relation, Relation::Narrower);
        assert_eq!(rows[0].zid, "zid-narrow");
        assert_eq!(rows[0].origins, ["h-cccccccccccc"]);
        assert_eq!(rows[0].attribution, Attribution::Session);
        assert!(!rows[0].total_wildcard);
        assert_eq!(rows[1].relation, Relation::Total);
        assert!(rows[1].total_wildcard);
        assert!(rows[1].origins.is_empty(), "session only, unattributed");
    }

    /// Exact, wider and intersects each get their relation; a prefix that
    /// includes the whole base is total even when it is not a bare `**`.
    #[test]
    fn every_relation_is_reachable() {
        let declared = DeclaredEntities {
            entities: vec![
                entity(EntityKind::Subscriber, TARGET, "n", peers(&["exact"])),
                entity(
                    EntityKind::Querier,
                    "v1/*/state/demo/**",
                    "n",
                    peers(&["wider"]),
                ),
                entity(
                    EntityKind::Subscriber,
                    "v1/h-cccccccccccc/*/demo/health",
                    "n",
                    peers(&["intersects"]),
                ),
                entity(EntityKind::Subscriber, "v1/**", "n", peers(&["total"])),
            ],
        };
        let rows = join_consumers(TARGET, &declared, &[], None, "me");
        let got: Vec<(&str, Relation)> =
            rows.iter().map(|r| (r.zid.as_str(), r.relation)).collect();
        assert_eq!(
            got,
            [
                ("exact", Relation::Exact),
                ("wider", Relation::Wider),
                ("intersects", Relation::Intersects),
                ("total", Relation::Total),
            ]
        );
        assert_eq!(rows[1].kind, EntityKind::Querier);
    }

    /// Sources naming nobody: the reporter's zid, marked reported-only. Two
    /// reporters of one declaration are one row. The tool's own session is
    /// named. The topology supplies `whatami` only for zids it heard of.
    #[test]
    fn attribution_and_self_are_stated_not_guessed() {
        let declared = DeclaredEntities {
            entities: vec![
                entity(EntityKind::Subscriber, TARGET, "reporter", json!({})),
                entity(EntityKind::Subscriber, TARGET, "router1", peers(&["me"])),
                entity(EntityKind::Subscriber, TARGET, "router2", peers(&["me"])),
            ],
        };
        let topology = TopologyReport {
            nodes: vec![TopologyNode {
                zid: "me".into(),
                whatami: "peer".into(),
                version: None,
                locators: vec![],
                locators_via_links: vec![],
                answered: true,
            }],
            edges: vec![],
            asked: "@/*/*".into(),
            answered: 1,
            self_zid: "me".into(),
        };
        let rows = join_consumers(TARGET, &declared, &[], Some(&topology), "me");
        assert_eq!(rows.len(), 2, "{rows:?}");
        let me = rows.iter().find(|r| r.zid == "me").unwrap();
        assert!(me.is_self);
        assert_eq!(me.whatami.as_deref(), Some("peer"));
        assert_eq!(me.attribution, Attribution::Session);
        let reported = rows.iter().find(|r| r.zid == "reporter").unwrap();
        assert_eq!(reported.attribution, Attribution::ReportedOnly);
        assert!(reported.whatami.is_none(), "not heard of is not a kind");
    }

    /// `**` never crosses an `@`-chunk (RFC 03 §4 D2): a total subscriber
    /// is not a row for a verbatim-plane target, and the base tree of a
    /// service origin's key is still the `v1` tree.
    #[test]
    fn a_total_wildcard_does_not_reach_a_verbatim_plane() {
        let declared = DeclaredEntities {
            entities: vec![entity(EntityKind::Subscriber, "**", "n", peers(&["wide"]))],
        };
        let rows = join_consumers("v1/@catalog/state/entity/**", &declared, &[], None, "me");
        assert!(rows.is_empty(), "{rows:?}");
        assert_eq!(base_tree_of("acme/v1/@catalog/state/**"), "acme/v1/**");
        assert_eq!(base_tree_of("@/*/*"), "**");
    }

    #[test]
    fn declaring_sessions_count_distinct_zids() {
        let declared = DeclaredEntities {
            entities: vec![
                entity(EntityKind::Publisher, TARGET, "n", peers(&["a", "b"])),
                entity(EntityKind::Publisher, TARGET, "n", peers(&["a"])),
                entity(EntityKind::Publisher, TARGET, "reporter", json!({})),
                entity(EntityKind::Queryable, "v1/**", "n", peers(&["q"])),
                entity(EntityKind::Queryable, "other/**", "n", peers(&["r"])),
            ],
        };
        assert_eq!(
            declaring_sessions(TARGET, &declared, EntityKind::Publisher),
            3
        );
        assert_eq!(
            declaring_sessions(TARGET, &declared, EntityKind::Queryable),
            1
        );
    }

    fn slices() -> crate::SliceSet {
        let toml = r#"
[registry]
version = "1.0"
app = "acme"
convention = 1
[producer]
name = "sysinfo"

[[subject]]
path = "disk/{mount}/used"
class = "telemetry"
type = "TelemetryPoint"

[[subject]]
path = "health"
class = "state"
type = "HealthSnapshot"

[[deprecated]]
path = "ingest/legacy_total"
since = "2.0"
replaced_by = "disk/{mount}/bytes_used"
"#;
        let slice = zenkey::parse_slice(toml).expect("a slice");
        crate::SliceSet::from_slices(vec![slice])
    }

    #[test]
    fn a_subject_resolves_to_its_family_selector() {
        let set = slices();
        let t = subject_target(&set, "acme", "sysinfo", "disk/{mount}/used").unwrap();
        assert_eq!(t.class, "telemetry");
        assert_eq!(t.selector, "acme/v1/*/telemetry/sysinfo/disk/*/used");
        assert!(t.deprecated.is_none());
        let t = subject_target(&set, "", "sysinfo", "health").unwrap();
        assert_eq!(t.selector, "v1/*/state/sysinfo/health");
        // Ledger-only: class wildcarded, the fact carried.
        let t = subject_target(&set, "", "sysinfo", "ingest/legacy_total").unwrap();
        assert_eq!(t.class, "*");
        assert_eq!(t.selector, "v1/*/*/sysinfo/ingest/legacy_total");
        assert_eq!(
            t.deprecated,
            Some(DeprecationFact {
                since: Some("2.0".into()),
                replaced_by: Some("disk/{mount}/bytes_used".into()),
            })
        );
        assert!(subject_target(&set, "", "sysinfo", "nope").is_none());
        assert!(subject_target(&set, "", "logs", "health").is_none());
    }
}
