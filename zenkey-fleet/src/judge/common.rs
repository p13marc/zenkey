//! The vocabulary every judge is written in.
//!
//! Before this module existed, each of these lived wherever it was first
//! needed and the others reached across for it: `doctor` reached into
//! `field` for `producer_of`, `expect` and `condition` reached into
//! `doctor` for `is_synthetic_marker` and the check ids, `retired` reached
//! into `cutover` for the new-plane prefix, and `doctor` reached into
//! `budget` for a cap. Each reach was individually reasonable and
//! collectively said that the judging layer had no shared vocabulary — only
//! a first-mover for every word in it.
//!
//! What belongs here: the things **more than one judge must agree about**.
//! A marker that decides whether traffic is real, the prefix that defines
//! "the new plane", the ceilings on how many offenders a report names. What
//! does not: the id vocabularies, which went to [`crate::report`] with #347
//! because they are serde-pinned wire shapes and that is where those live;
//! any check's own logic; and
//! any sentence written in one judge's voice — `cutover::scope_note` and
//! `retired::scope_note` stay where they are, because they are two different
//! O5 statements about two different windows, not one statement said twice.

use zenkey::grammar::with_base;

use crate::SliceSet;
use crate::model::facts::{KeyFacts, KeyShape, OriginKind};

// The stable id vocabularies used to live here as two `[&str; N]`. They are
// `report::CheckId` and `report::RungId` now (#347) — serde-pinned wire
// shapes, so `CLAUDE.md`'s placement rule puts them under `report/`, with
// their stability tests beside them.

// ─── the caps ───────────────────────────────────────────────────────────────
//
// Three ceilings over three different populations. They are here because
// more than one judge uses each — not because they are one number: the
// values are three separate policies that today happen to be 20, 5 and 3,
// and a future change to one must not drag the others. The *mechanism*
// (count everything offered, keep the first `cap`) is
// [`Examples`](crate::model::examples::Examples)'s and always was.

/// How many offending keys a check names before it says "… and N more".
///
/// Shared by the doctor's per-check findings, `field`'s per-path ones,
/// `expect`'s violations and `cutover`'s leaked keys — four judges that had
/// four constants of the same value under two different names.
pub(crate) const FINDING_CAP: usize = 20;

/// How many evidence lines one `why` rung carries. Smaller than
/// [`FINDING_CAP`] on purpose: a rung's evidence is read as prose under the
/// answer, not scanned as a table.
pub(crate) const EVIDENCE_CAP: usize = 5;

/// How many example expansions a budget finding or cell carries — enough to
/// recognise the family member that exploded, without pasting the
/// population.
pub const EXPANSION_CAP: usize = 3;

// ─── the shared judgements ──────────────────────────────────────────────────

/// Does an attachment carry the RFC 09 §5.3 synthetic-traffic marker
/// (`{"synthetic": true, …}`, #162)?
///
/// Generated traffic judged as real would be a self-inflicted finding, so
/// every judge that watches a window counts it separately — the doctor's
/// listen phase (#161) and the watchdog's windows (#227) alike. One
/// spelling, because two would eventually disagree about what a rehearsal
/// looks like.
pub(crate) fn is_synthetic_marker(attachment: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(attachment)
        .ok()
        .and_then(|v| v.get("synthetic").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

/// The producer name behind a key's facts — a service origin's slice is
/// found by the origin it serves (RFC 03 §1.5).
///
/// Used by `field` to attribute a path and by `doctor` to attribute a
/// finding, and the two must attribute identically or the same key gets two
/// producers in one report.
pub(crate) fn producer_of(facts: &KeyFacts, slices: Option<&SliceSet>) -> Option<String> {
    let KeyShape::V1(v) = &facts.shape else {
        return None;
    };
    match v.origin_kind {
        OriginKind::Host => v.producer.clone(),
        OriginKind::Service => {
            slices.and_then(|s| s.by_service_origin(&v.origin).map(|s| s.name.clone()))
        }
    }
}

/// The stated meaning of "the new plane": keys under `<base>/v1/`.
///
/// `cutover` asserts traffic on it; `retired` asserts a replacement under
/// it. Two judges, one definition — a second spelling would let a migration
/// pass one check and fail the other over the same bus.
pub fn new_prefix(base: &str) -> String {
    format!("{}/", with_base(base, "v1"))
}

/// The data-plane scopes a passive observation must watch: the three data
/// classes for host origins, plus each declared service origin's three —
/// `**` never crosses an `@` chunk (RFC 03 §4 D2), so the service planes
/// must be named to be seen. This is the O5 scope statement the doctor's
/// listen phase and the `--budget` observation share (#161, #221).
pub fn data_plane_scopes(base: &str, slices: &SliceSet) -> Vec<String> {
    let mut scopes = Vec::new();
    for class in ["telemetry", "state", "events"] {
        scopes.push(with_base(base, format!("v1/*/{class}/**")));
    }
    for slice in slices.slices() {
        if let Some(origin) = &slice.service_origin {
            for class in ["telemetry", "state", "events"] {
                scopes.push(with_base(base, format!("v1/{origin}/{class}/**")));
            }
        }
    }
    scopes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #162's marker as #161 reads it: a JSON object with `"synthetic": true`.
    /// Anything else — other attachments, non-JSON bytes — is real traffic.
    #[test]
    fn the_synthetic_marker_is_recognised_and_nothing_else_is() {
        assert!(is_synthetic_marker(
            br#"{"synthetic":true,"tool":"zenctl gen"}"#
        ));
        assert!(!is_synthetic_marker(br#"{"synthetic":false}"#));
        assert!(!is_synthetic_marker(br#"{"tool":"zenctl gen"}"#));
        assert!(!is_synthetic_marker(b"meta"));
        assert!(!is_synthetic_marker(b""));
    }

    /// One definition of "the new plane", so `cutover` and `retired` cannot
    /// disagree about what a migration moved to.
    #[test]
    fn the_new_plane_is_v1_under_the_base() {
        assert_eq!(new_prefix("acme"), "acme/v1/");
        assert_eq!(new_prefix(""), "v1/");
        assert_eq!(new_prefix("a/b"), "a/b/v1/");
    }

    #[test]
    fn scopes_name_the_service_planes_explicitly() {
        let toml = r#"
            [registry]
            version = "1.0"
            app = "t"
            convention = 1
            [service]
            name = "catalog"
            origin = "@catalog"
        "#;
        let slices = SliceSet::from_toml_for_tests(toml);
        let scopes = data_plane_scopes("zs", &slices);
        assert!(scopes.contains(&"zs/v1/*/telemetry/**".to_string()));
        assert!(
            scopes.contains(&"zs/v1/@catalog/state/**".to_string()),
            "`*` never matches `@catalog` (D4), so it must be named: {scopes:?}"
        );
        assert_eq!(scopes.len(), 6);
    }
}
