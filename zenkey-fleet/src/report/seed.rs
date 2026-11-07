//! The seeding plane: what each leg of a seeded subscription contributed
//! (core §4.2's late joiner, 0.26; #614, FL4).
//!
//! A leg reads three ways on purpose — absent is a leg that was switched
//! off ([`Asked::NotAsked`]), zero replies is a leg that ran and nobody
//! answered, and a number is a number (the tooling guide's O4) — and a leg
//! that ran says whether it ran to its final reply, since a GET that
//! reached its timeout seeds possibly incompletely (§8.1's reading, applied
//! to state by 0.26).

use serde::Serialize;

use crate::report::{Asked, CollectionSnapshot};

/// What one seed leg's GET brought back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SeedLeg {
    /// Replies received, the error replies aside, a snapshot's page one
    /// reply. Zero is "ran, and nobody answered": an observation, never a
    /// verdict (O5).
    pub replies: usize,
    /// Whether the GET ran to its final reply with no error reply, and
    /// every snapshot it heard is complete. `false` is possibly incomplete:
    /// in zenoh 1.10.1 a GET that reached its timeout ends with an error
    /// reply (core Appendix B), so a replier the GET did not wait for may
    /// hold state this seed lacks; and a snapshot held incomplete may lack
    /// members.
    pub complete: bool,
    /// Each collection's snapshot the state leg's pages made (core §7.5;
    /// #977's E2), in the order of its first page, each with its own run
    /// and asof: the seed holds the collection as of that seq of that run.
    /// Absent when no page answered, so a leg of per-key replies keeps its
    /// shape.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub snapshots: Vec<CollectionSnapshot>,
}

/// What each seed leg contributed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SeedCoverage {
    /// The history leg: the `@adv` caches of the advanced publishers present
    /// (core §2.5), for resources that declare `history`. Absent when it was
    /// switched off.
    #[serde(default, skip_serializing_if = "Asked::is_not_asked")]
    pub history: Asked<SeedLeg>,
    /// The state leg: S4's GET on the watched selector, target `All` and
    /// consolidation `Latest` (core §4.2). Answered by each key's owner, and
    /// for a foreign key by whatever answers it, a snapshot's page read as
    /// its entries ([`SeedLeg::snapshots`]). Absent when it was switched
    /// off.
    #[serde(default, skip_serializing_if = "Asked::is_not_asked")]
    pub state: Asked<SeedLeg>,
    /// Samples suppressed by the merge — not newer, by the owner's in-band
    /// stamp (a Zenoh timestamp for a sample with no envelope), than what
    /// was already held for their key (core §4.2, §4.3): a reply older than
    /// a live sample within its run, a live sample older than a reply, or
    /// one mutation that both paths carried. Counted, never silently
    /// absorbed.
    pub superseded: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The coverage's wire shape, pinned: a leg switched off is absent, a
    /// leg that ran carries its replies and whether it ran to its end, and
    /// v1's `storage_replies` is gone with the storage it named.
    #[test]
    fn seed_coverage_json_shape_is_pinned() {
        let both = SeedCoverage {
            history: Asked::Asked(SeedLeg {
                replies: 1,
                complete: true,
                snapshots: Vec::new(),
            }),
            state: Asked::Asked(SeedLeg {
                replies: 0,
                complete: false,
                snapshots: Vec::new(),
            }),
            superseded: 2,
        };
        assert_eq!(
            serde_json::to_value(both).expect("serializes"),
            json!({
                "history": {"replies": 1, "complete": true},
                "state": {"replies": 0, "complete": false},
                "superseded": 2,
            })
        );
        let state_only = SeedCoverage {
            history: Asked::NotAsked,
            state: Asked::Asked(SeedLeg {
                replies: 3,
                complete: true,
                snapshots: Vec::new(),
            }),
            superseded: 0,
        };
        assert_eq!(
            serde_json::to_value(state_only).expect("serializes"),
            json!({"state": {"replies": 3, "complete": true}, "superseded": 0})
        );
        // A state leg that heard a snapshot says which, with its run and
        // asof (#977's E2); the shapes above stand.
        let paged = SeedLeg {
            replies: 1,
            complete: false,
            snapshots: vec![CollectionSnapshot {
                key: "zk2/g/fm/mission_plan.v1/state/plans".into(),
                coll: 0,
                run: "0005".into(),
                asof: 7,
                pages: vec![2],
                complete: false,
            }],
        };
        assert_eq!(
            serde_json::to_value(paged).expect("serializes"),
            json!({"replies": 1, "complete": false, "snapshots": [{
                "key": "zk2/g/fm/mission_plan.v1/state/plans",
                "coll": 0, "run": "0005", "asof": 7, "pages": [2], "complete": false,
            }]})
        );
    }
}
