//! Key-population budgets (#221): the declared `cardinality` bound joined to
//! what a bounded observation actually saw.
//!
//! Every `{var}` subject has declared a `cardinality` since v1.0 — RFC 08 §2
//! makes the field mandatory on any pattern with a variable, and RFC 04 §1.2
//! makes an unbudgeted population-keyed subject a registry-review reject —
//! and until this module nothing ever compared the declaration to reality.
//! The join is deliberately engine-side: the doctor's
//! `cardinality-over-declared` check and `zenctl topic list --budget` both
//! read it, and a later zengui window gets its tree badge from the same
//! numbers (deferred there; this chunk ships the engine and zenctl halves).
//!
//! The honesty rules are the substance (RFC 09 §5.1):
//!
//! - Observed **over** declared is a finding. Observed **under** declared is
//!   **not** — a bounded window proves a lower bound on the population,
//!   never the population, and an idle host declares nothing wrong (O4/O6).
//! - `{path...}` rest-variable families are unbounded by construction and
//!   are **exempt and say so** — the RFC 08 §6.1 (v1.20) shape: an
//!   exemption renders as "exempt: rest-variable", never as a silent skip
//!   and never as a pass.
//! - Every number states its window and scopes (O5).

use std::collections::{BTreeMap, BTreeSet};

use zenkey::grammar::with_base;

use crate::SliceSet;
use crate::facts::{KeyFacts, KeyShape, OriginKind};
use crate::report::{BudgetCell, BudgetWindow, TopicList};

/// How many example expansions a finding or budget cell carries — enough to
/// recognise the family member that exploded, without pasting the population.
pub const EXAMPLE_CAP: usize = 3;

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

/// Observed expansions of every `{var}` subject family, grouped
/// per origin — RFC 04 §1's table bounds cardinality *per producer*, so one
/// origin exceeding the bound is conclusive on its own and two origins'
/// mounts are never summed into a fake violation.
#[derive(Debug, Clone, Default)]
pub struct BudgetObservation {
    /// (slice name, declared subject path) → origin → distinct concrete keys.
    families: BTreeMap<(String, String), BTreeMap<String, BTreeSet<String>>>,
}

impl BudgetObservation {
    /// Group observed concrete wire keys into `{var}` subject families.
    ///
    /// `keys` is whatever population the caller holds — the stats table /
    /// key-tree snapshot of a monitor window, or the doctor listen phase's
    /// key cache. Keys that do not parse, refine, or land on a variable
    /// pattern contribute nothing here (they have their own checks).
    pub fn observe<'a>(
        base: &str,
        slices: &SliceSet,
        keys: impl IntoIterator<Item = &'a str>,
    ) -> BudgetObservation {
        let mut families: BTreeMap<(String, String), BTreeMap<String, BTreeSet<String>>> =
            BTreeMap::new();
        for key in keys {
            let facts = KeyFacts::project(base, key);
            let KeyShape::V1(v) = &facts.shape else {
                continue;
            };
            if !v.class_kind.is_data_class() {
                continue;
            }
            // A service origin omits the producer chunk (RFC 03 §1.5); its
            // slice is found by the origin it serves.
            let producer = match v.origin_kind {
                OriginKind::Host => v.producer.clone(),
                OriginKind::Service => slices.by_service_origin(&v.origin).map(|s| s.name.clone()),
            };
            let Some(producer) = producer else {
                continue;
            };
            let tail: Vec<&str> = v.subject.iter().map(String::as_str).collect();
            let Some((decl, _)) = slices.refine(&producer, &v.class, &tail) else {
                continue;
            };
            if !decl.path.contains('{') {
                continue; // a literal subject's population is 1 by construction
            }
            families
                .entry((producer, decl.path.clone()))
                .or_default()
                .entry(v.origin.clone())
                .or_default()
                .insert(key.to_string());
        }
        BudgetObservation { families }
    }

    /// One family's per-origin expansions, when anything was observed.
    pub fn family(
        &self,
        producer: &str,
        path: &str,
    ) -> Option<&BTreeMap<String, BTreeSet<String>>> {
        self.families.get(&(producer.to_string(), path.to_string()))
    }
}

/// Join an observation onto a `topic list` report: every `{var}` row gets a
/// [`BudgetCell`], the list gets the [`BudgetWindow`] coverage statement.
///
/// Literal rows and ledger rows get no cell — their population is fixed by
/// construction, and an empty cell claims nothing (which is not a pass).
pub fn join_budget(list: &mut TopicList, obs: &BudgetObservation, window: BudgetWindow) {
    for row in &mut list.subjects {
        if row.deprecated || !row.path.contains('{') {
            continue;
        }
        let empty = BTreeMap::new();
        let origins = obs.family(&row.producer, &row.path).unwrap_or(&empty);
        let observed: usize = origins.values().map(BTreeSet::len).sum();
        let (worst_origin, worst_observed) = origins
            .iter()
            .max_by_key(|(_, keys)| keys.len())
            .map(|(o, keys)| (Some(o.clone()), keys.len()))
            .unwrap_or((None, 0));
        let examples = worst_origin
            .as_ref()
            .and_then(|o| origins.get(o))
            .map(|keys| keys.iter().take(EXAMPLE_CAP).cloned().collect())
            .unwrap_or_default();
        let exempt = row
            .path
            .contains("...")
            .then(|| "rest-variable".to_string());
        let over = exempt.is_none()
            && row
                .cardinality
                .is_some_and(|declared| worst_observed as i64 > declared);
        row.budget = Some(BudgetCell {
            declared: row.cardinality,
            observed,
            origins: origins.len(),
            worst_origin,
            worst_observed,
            exempt,
            over,
            examples,
        });
    }
    list.budget = Some(window);
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLICE: &str = r#"
        [registry]
        version = "1.0"
        app = "t"
        convention = 1
        [producer]
        name = "sysinfo"
        [[subject]]
        path = "disk/{mount}/used"
        class = "telemetry"
        type = "Point"
        cardinality = 16
        [[subject]]
        path = "health"
        class = "state"
        type = "Health"
    "#;

    #[test]
    fn observation_groups_per_origin_and_skips_literals() {
        let slices = SliceSet::from_toml_for_tests(SLICE);
        let keys = [
            "v1/h-aaaaaaaaaaaa/telemetry/sysinfo/disk/root/used",
            "v1/h-aaaaaaaaaaaa/telemetry/sysinfo/disk/var/used",
            "v1/h-bbbbbbbbbbbb/telemetry/sysinfo/disk/root/used",
            // A literal subject and an unregistered key contribute nothing.
            "v1/h-aaaaaaaaaaaa/state/sysinfo/health",
            "v1/h-aaaaaaaaaaaa/telemetry/sysinfo/not/registered",
        ];
        let obs = BudgetObservation::observe("", &slices, keys);
        let fam = obs.family("sysinfo", "disk/{mount}/used").unwrap();
        assert_eq!(fam.len(), 2, "two origins expanded the family");
        assert_eq!(fam["h-aaaaaaaaaaaa"].len(), 2);
        assert_eq!(fam["h-bbbbbbbbbbbb"].len(), 1);
        assert!(
            obs.family("sysinfo", "health").is_none(),
            "literals excluded"
        );
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
