//! Consumers and blast radius (#224): who *declares* a reader of a subject,
//! and what changing that subject would reach.
//!
//! Every shape here is a join over the admin space (RFC 09 §5.1): declared
//! subscribers and queriers with their verbatim keyexprs, the origin
//! attachments the tokens back, and the topology's node roster. None of it
//! is matching status — RFC 12 §9 defers foreign matching permanently, and
//! a declaration is evidence that a session *asked for* a key, never proof
//! that anything reads it. The wording rule follows from that: nothing in
//! these reports says "listening", "matching" or "unmatched".

use serde::Serialize;

use super::admin::{CoverageRow, EntityKind};

/// The consumers of one target selector, as the admin space declared them.
#[derive(Debug, Clone, Serialize)]
pub struct ConsumersReport {
    /// The wire selector the declarations were related to.
    pub target: String,
    /// The admin selectors actually put to the bus (RFC 13 §3 O5) —
    /// coverage is exactly this list.
    pub asked: Vec<String>,
    /// This session's own zid: the tool's own declarations appear in its
    /// own results, and are named rather than hidden.
    pub self_zid: String,
    /// Whether any admin space answered, and how many. Flattened so the
    /// discriminator rides at the top of the document beside the rows.
    #[serde(flatten)]
    pub admin: AdminAnswer,
    /// One row per declaring session per declaration, ranked by relation.
    /// Empty under [`AdminAnswer::NotAvailable`] is *not asked*; empty under
    /// [`AdminAnswer::Answered`] is *nothing declared in the answering admin
    /// spaces* — the renderers say which.
    pub rows: Vec<ConsumerRow>,
    /// Admin replies past the bound, not kept (RFC 13 §3 O6). A declaration
    /// past the bound is a consumer this report cannot show.
    pub reply_elided: u64,
}

/// Whether the admin space answered at all (RFC 13 §3 O4).
///
/// zenoh's `adminspace.enabled` defaults to *false* (routers ship with it
/// on; a pure peer mesh has none), so a sweep nobody answered is *not
/// asked*, never *nobody declared anything*. The two must not share a
/// spelling anywhere, which is why this is an enum and not an `answered: 0`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "admin", rename_all = "snake_case")]
pub enum AdminAnswer {
    /// `answered` admin spaces served root docs; `nodes` is the topology's
    /// roster, heard-of nodes included — the sessions behind an admin space
    /// that did not answer are not in the rows.
    Answered { answered: usize, nodes: usize },
    /// No admin space answered any sweep.
    NotAvailable,
}

/// One declared reader, per session.
#[derive(Debug, Clone, Serialize)]
pub struct ConsumerRow {
    /// The session that declared it — or, under
    /// [`Attribution::ReportedOnly`], the admin space that reported it.
    pub zid: String,
    /// `router` | `peer` | `client`, when the topology sweep heard of the
    /// zid. Absent when it did not — unknown, not "no kind".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub whatami: Option<String>,
    /// The origins whose alive tokens the admin space attributes to this
    /// session (#131). Several is legitimate (one process, several
    /// producers); empty means *session only, unattributed*.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub origins: Vec<String>,
    pub attribution: Attribution,
    /// `subscriber` or `querier` — the two entity kinds that read.
    pub kind: EntityKind,
    /// The declared key expression, verbatim.
    pub keyexpr: String,
    pub relation: Relation,
    /// This tool's own session. Shown, and named.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub is_self: bool,
    /// The declaration reaches the whole base (`**`, or a prefix that
    /// includes every `v1` key): it intersects everything, and is shown as
    /// such rather than as a consumer of this subject in particular.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub total_wildcard: bool,
}

/// How a row's zid was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Attribution {
    /// The admin `sources` named the declaring session.
    Session,
    /// The sources named nobody: the zid is the reporting admin space's,
    /// and the declaration is only known to exist behind it.
    ReportedOnly,
}

/// How a declared keyexpr relates to the target, by key algebra
/// (`zenoh-keyexpr`'s `includes`/`intersects`). Ranked: the ordering is the
/// row order, most specific first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    /// The declaration and the target are the same set.
    Exact,
    /// The target includes the declaration: it hears a subset.
    Narrower,
    /// The declaration includes the target, and is not the whole base.
    Wider,
    /// The two overlap without either including the other.
    Intersects,
    /// The declaration includes the whole base (`**`).
    Total,
}

impl Relation {
    /// The wire token, for renderers that write it into prose.
    pub fn as_str(self) -> &'static str {
        match self {
            Relation::Exact => "exact",
            Relation::Narrower => "narrower",
            Relation::Wider => "wider",
            Relation::Intersects => "intersects",
            Relation::Total => "total",
        }
    }
}

/// The blast radius of one declared subject (#224): its consumers, its
/// storage coverage, what else declares on it, and its ledger entry.
///
/// Not `ImpactReport` — that name is the zenwatch attribution's
/// (RFC 06 §5.6), a different question about a different plane.
#[derive(Debug, Clone, Serialize)]
pub struct SubjectImpact {
    pub producer: String,
    /// The subject path as the registry spells it (`disk/{mount}/used`).
    pub path: String,
    /// The declared class chunk; `*` when the path is known only from the
    /// `[[deprecated]]` ledger, whose entries carry no class.
    pub class: String,
    /// The wire selector the subject's family resolves to under the base.
    pub selector: String,
    pub consumers: ConsumersReport,
    /// The RFC 04 §2 coverage rows for this subject. `None` = the storage
    /// sweep was not made (no admin space answered, so an empty list would
    /// read as "uncovered"); `Some([])` = asked, and the subject is not a
    /// state family — coverage does not apply.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage: Option<Vec<CoverageRow>>,
    /// Distinct sessions declaring a publisher intersecting the selector.
    /// Absent when no admin space answered (not asked, O4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declared_publishers: Option<usize>,
    /// Distinct sessions declaring a queryable intersecting the selector.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declared_queryables: Option<usize>,
    /// The subject's `[[deprecated]]` entry, when the ledger carries one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<DeprecationFact>,
}

/// What the registry ledger says about a retired subject (RFC 08 §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeprecationFact {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaced_by: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row() -> ConsumerRow {
        ConsumerRow {
            zid: "eeff0011".into(),
            whatami: Some("peer".into()),
            origins: vec!["h-3fa9c2d41b7e".into()],
            attribution: Attribution::Session,
            kind: EntityKind::Subscriber,
            keyexpr: "v1/h-3fa9c2d41b7e/state/sysinfo/health".into(),
            relation: Relation::Narrower,
            is_self: false,
            total_wildcard: false,
        }
    }

    /// The two admin answers never share a spelling: `not_available`
    /// carries no counts at all, and `answered` always carries both.
    #[test]
    fn the_admin_answer_is_a_discriminator_not_a_zero() {
        let mut r = ConsumersReport {
            target: "v1/*/state/sysinfo/health".into(),
            asked: vec!["@/*/*".into()],
            self_zid: "ffffffff".into(),
            admin: AdminAnswer::Answered {
                answered: 1,
                nodes: 2,
            },
            rows: vec![],
            reply_elided: 0,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["admin"], "answered");
        assert_eq!(v["answered"], 1);
        assert_eq!(v["nodes"], 2);

        r.admin = AdminAnswer::NotAvailable;
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["admin"], "not_available");
        assert!(v.get("answered").is_none(), "{v}");
        assert!(v.get("nodes").is_none(), "{v}");
    }

    /// `origins` is absent when empty (session only, unattributed) and
    /// `is_self`/`total_wildcard` are absent when false — news, not
    /// non-news (O4).
    #[test]
    fn a_row_omits_what_is_not_news() {
        let v = serde_json::to_value(row()).unwrap();
        assert_eq!(
            v,
            json!({
                "zid": "eeff0011",
                "whatami": "peer",
                "origins": ["h-3fa9c2d41b7e"],
                "attribution": "session",
                "kind": "subscriber",
                "keyexpr": "v1/h-3fa9c2d41b7e/state/sysinfo/health",
                "relation": "narrower",
            })
        );
        let bare = ConsumerRow {
            whatami: None,
            origins: vec![],
            attribution: Attribution::ReportedOnly,
            is_self: true,
            total_wildcard: true,
            relation: Relation::Total,
            keyexpr: "**".into(),
            ..row()
        };
        let v = serde_json::to_value(bare).unwrap();
        assert!(v.get("origins").is_none(), "{v}");
        assert!(v.get("whatami").is_none(), "{v}");
        assert_eq!(v["is_self"], true);
        assert_eq!(v["total_wildcard"], true);
        assert_eq!(v["attribution"], "reported_only");
        assert_eq!(v["relation"], "total");
    }

    /// Relations rank most-specific first — the row order.
    #[test]
    fn relations_rank_specific_first() {
        assert!(Relation::Exact < Relation::Narrower);
        assert!(Relation::Narrower < Relation::Wider);
        assert!(Relation::Wider < Relation::Intersects);
        assert!(Relation::Intersects < Relation::Total);
    }
}
