//! Key-population budgets in the tree (#221): the declared `cardinality`
//! bound joined against the observed population, per `{var}` family and per
//! origin — the GUI half of [`zenkey_fleet::budget`].
//!
//! The join itself is the engine's ([`zenkey_fleet::budget::BudgetObservation`]);
//! this module walks the observed key tree into the key list the engine
//! wants, and turns each judged family into a badge keyed by the family's
//! **subtree display path** — the literal prefix under one origin — so the
//! tree can look a row up by path, off the render path.
//!
//! The honesty rules ride along unchanged (RFC 09 §5.1):
//!
//! - Observed **over** declared is a badge. Observed **under** declared is
//!   **not** — a bounded window proves a lower bound, never the population,
//!   and an idle host declares nothing wrong (O4/O6).
//! - `{path...}` rest-variable families are unbounded by construction and
//!   badge as *exempt, and say so* — never a silent skip, never a pass.
//! - Badges exist **only where a registry is loaded**: with no declarations
//!   there is no budget to be over, and a badge would be a claim nobody made.
//!
//! Computed in a `Task` on a throttled cadence (`update/bus.rs`), never per
//! frame: the walk is O(keys) and the refinement O(keys × slices).

use std::collections::BTreeMap;

use zenkey_fleet::budget::BudgetObservation;
use zenkey_fleet::{KeyTreeSnapshot, SliceSet};

/// One family's badge, at one origin's subtree path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetBadge {
    /// This origin's observed expansion count exceeds the declared
    /// `cardinality` (RFC 04 §1 bounds it per producer, so one origin over
    /// is conclusive on its own).
    Over { declared: i64, observed: usize },
    /// A `{path...}` rest-variable family: unbounded by construction, exempt
    /// from the budget — and it says so rather than passing silently
    /// (the RFC 08 §6.1 v1.20 shape).
    Exempt,
}

impl BudgetBadge {
    /// The badge word — glyph comes from the severity tone at the call site;
    /// the words carry the numbers so colour is never the only carrier.
    pub fn label(&self) -> String {
        match self {
            BudgetBadge::Over { declared, observed } => {
                format!("over budget: observed {observed} of {declared} declared")
            }
            BudgetBadge::Exempt => "exempt: rest-variable".to_string(),
        }
    }
}

/// Subtree display path → badge, as of one observation of the key tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BudgetBadges {
    map: BTreeMap<String, BudgetBadge>,
}

impl BudgetBadges {
    pub fn get(&self, path: &str) -> Option<&BudgetBadge> {
        self.map.get(path)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Every concrete key the snapshot holds — the nodes a sample landed exactly
/// on. The engine's observation wants wire keys; the tree stores them as
/// chunk paths, so this is the inverse walk of `KeyTreeSnapshot::build`.
pub fn observed_keys(tree: &KeyTreeSnapshot) -> Vec<String> {
    fn walk(prefix: &str, node: &zenkey_fleet::model::tree::TreeNode, out: &mut Vec<String>) {
        for (chunk, child) in &node.children {
            let path = if prefix.is_empty() {
                chunk.clone()
            } else {
                format!("{prefix}/{chunk}")
            };
            // `last_seen` is set exactly where a sample landed — a key can be
            // both a leaf and a prefix of deeper keys, and both are keys.
            if child.last_seen.is_some() {
                out.push(path.clone());
            }
            walk(&path, child, out);
        }
    }
    let mut out = Vec::new();
    walk("", &tree.root, &mut out);
    out
}

/// Join one observation of the tree against the loaded registry.
///
/// Pure, so the throttle and the `Task` around it stay trivial — the
/// engine's [`BudgetObservation`] does the grouping and this derives, per
/// (family, origin), the badge and the subtree path it hangs on.
pub fn badges(base: &str, slices: &SliceSet, tree: &KeyTreeSnapshot) -> BudgetBadges {
    let keys = observed_keys(tree);
    let obs = BudgetObservation::observe(base, slices, keys.iter().map(String::as_str));
    let base_chunks = if base.is_empty() {
        0
    } else {
        base.split('/').count()
    };
    let mut map = BTreeMap::new();
    for slice in slices.slices() {
        for subject in &slice.subjects {
            if !subject.path.contains('{') {
                continue; // a literal subject's population is 1 by construction
            }
            let Some(origins) = obs.family(&slice.name, &subject.path) else {
                continue; // nothing observed: no subtree row to badge (O4)
            };
            let exempt = subject.path.contains("...");
            let literal_chunks = subject
                .path
                .split('/')
                .take_while(|c| !c.contains('{'))
                .count();
            for (origin, keys) in origins {
                let badge = if exempt {
                    BudgetBadge::Exempt
                } else {
                    // Under is never flagged: a window proves a lower bound
                    // on the population, not the population (O4/O6).
                    match subject.cardinality {
                        Some(declared) if keys.len() as i64 > declared => BudgetBadge::Over {
                            declared,
                            observed: keys.len(),
                        },
                        _ => continue,
                    }
                };
                let Some(example) = keys.iter().next() else {
                    continue;
                };
                // The family's subtree under this origin: base + v1 + origin
                // + class (+ producer for host origins, RFC 03 §1.5) + the
                // declared path's literal prefix.
                let head = base_chunks + 3 + usize::from(!origin.starts_with('@'));
                let prefix: Vec<&str> = example.split('/').take(head + literal_chunks).collect();
                map.insert(prefix.join("/"), badge);
            }
        }
    }
    BudgetBadges { map }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use zenkey_fleet::model::stats::StatsTable;

    fn subject(path: &str, class: &str, cardinality: Option<i64>) -> zenkey::slice::SubjectDecl {
        zenkey::slice::SubjectDecl {
            path: path.into(),
            class: class.into(),
            type_name: "T".into(),
            common: None,
            since: None,
            description: None,
            qos: None,
            ttl_s: None,
            unit: None,
            rate: None,
            cardinality,
            encoding: None,
        }
    }

    fn fixture_slices() -> SliceSet {
        SliceSet::from_slices(vec![zenkey::slice::RegistrySlice {
            version: "1.0".into(),
            app: "t".into(),
            convention: 1,
            name: "sysinfo".into(),
            service_origin: None,
            description: None,
            subjects: vec![
                subject("disk/{mount}/used", "telemetry", Some(2)),
                subject("log/{path...}", "events", Some(1)),
                subject("health", "state", None),
            ],
            procedures: vec![],
            blob: vec![],
            media: vec![],
            deprecated: vec![],
        }])
    }

    fn snapshot(keys: &[&str]) -> KeyTreeSnapshot {
        let mut stats = StatsTable::new();
        let now = Instant::now();
        for k in keys {
            stats.record(k, 1, None, now, None, None);
        }
        KeyTreeSnapshot::build(&stats)
    }

    /// The walk recovers exactly the recorded keys — including a key that is
    /// also a prefix of deeper keys.
    #[test]
    fn observed_keys_invert_the_tree_build() {
        let snap = snapshot(&["a/b", "a/b/c", "x"]);
        let mut keys = observed_keys(&snap);
        keys.sort();
        assert_eq!(keys, ["a/b", "a/b/c", "x"]);
    }

    /// The acceptance shape: a family over its declared cardinality badges
    /// its subtree; under is never flagged; `{path...}` badges as exempt and
    /// never as a pass; a literal subject badges nothing.
    #[test]
    fn over_is_badged_under_is_not_and_rest_variables_are_exempt() {
        let slices = fixture_slices();
        let snap = snapshot(&[
            // h-a expands disk/{mount}/used three times: over its 2.
            "v1/h-aaaaaaaaaaaa/telemetry/sysinfo/disk/root/used",
            "v1/h-aaaaaaaaaaaa/telemetry/sysinfo/disk/var/used",
            "v1/h-aaaaaaaaaaaa/telemetry/sysinfo/disk/tmp/used",
            // h-b expands it once: under, and not flagged.
            "v1/h-bbbbbbbbbbbb/telemetry/sysinfo/disk/root/used",
            // The rest-variable family, observed: exempt, said out loud.
            "v1/h-aaaaaaaaaaaa/events/sysinfo/log/var/log/syslog",
            // A literal subject: no badge, its population is 1 by shape.
            "v1/h-aaaaaaaaaaaa/state/sysinfo/health",
        ]);
        let b = badges("", &slices, &snap);
        assert_eq!(
            b.get("v1/h-aaaaaaaaaaaa/telemetry/sysinfo/disk"),
            Some(&BudgetBadge::Over {
                declared: 2,
                observed: 3
            }),
            "{b:?}"
        );
        assert!(
            b.get("v1/h-bbbbbbbbbbbb/telemetry/sysinfo/disk").is_none(),
            "under declared is not a finding — an idle host declares nothing wrong"
        );
        assert_eq!(
            b.get("v1/h-aaaaaaaaaaaa/events/sysinfo/log"),
            Some(&BudgetBadge::Exempt),
            "a rest-variable family is exempt and says so, never a silent pass"
        );
        assert!(
            b.get("v1/h-aaaaaaaaaaaa/state/sysinfo/health").is_none(),
            "a literal subject has no budget to be over"
        );

        // The words carry the numbers — colour is never the only carrier.
        assert_eq!(
            BudgetBadge::Over {
                declared: 2,
                observed: 3
            }
            .label(),
            "over budget: observed 3 of 2 declared"
        );
        assert_eq!(BudgetBadge::Exempt.label(), "exempt: rest-variable");
    }

    /// The badge path carries the deployment base — the tree's display paths
    /// are full wire keys.
    #[test]
    fn badge_paths_carry_the_base() {
        let slices = fixture_slices();
        let snap = snapshot(&[
            "zs/v1/h-aaaaaaaaaaaa/telemetry/sysinfo/disk/root/used",
            "zs/v1/h-aaaaaaaaaaaa/telemetry/sysinfo/disk/var/used",
            "zs/v1/h-aaaaaaaaaaaa/telemetry/sysinfo/disk/tmp/used",
        ]);
        let b = badges("zs", &slices, &snap);
        assert!(
            b.get("zs/v1/h-aaaaaaaaaaaa/telemetry/sysinfo/disk")
                .is_some(),
            "{b:?}"
        );
    }
}
