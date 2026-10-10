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

use crate::report::Asked;

/// What one seed leg's GET brought back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SeedLeg {
    /// Replies received, the error replies aside. Zero is "ran, and nobody
    /// answered": an observation, never a verdict (O5).
    pub replies: usize,
    /// Whether the GET ran to its final reply with no error reply. `false`
    /// is possibly incomplete: in zenoh 1.10.1 a GET that reached its
    /// timeout ends with an error reply (core Appendix B), so a replier the
    /// GET did not wait for may hold state this seed lacks.
    pub complete: bool,
}

/// What each seed leg contributed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SeedCoverage {
    /// The history leg: the `@adv` caches of the advanced publishers present
    /// (core §2.5), for resources that declare `history`. Absent when it was
    /// switched off.
    #[serde(default, skip_serializing_if = "Asked::is_not_asked")]
    pub history: Asked<SeedLeg>,
    /// The state leg: S4's GET on the watched selector, target `All` and
    /// consolidation `Latest` (core §4.2). Answered by each key's owner, and
    /// for a foreign key by whatever answers it. Absent when it was switched
    /// off.
    #[serde(default, skip_serializing_if = "Asked::is_not_asked")]
    pub state: Asked<SeedLeg>,
    /// Samples suppressed by the merge — not newer, by the owner's stamp,
    /// than what was already held for their key (0.26's merge): a reply
    /// older than a live sample, a live sample older than a reply, or one
    /// mutation that both paths carried. Counted, never silently absorbed.
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
            }),
            state: Asked::Asked(SeedLeg {
                replies: 0,
                complete: false,
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
            }),
            superseded: 0,
        };
        assert_eq!(
            serde_json::to_value(state_only).expect("serializes"),
            json!({"state": {"replies": 3, "complete": true}, "superseded": 0})
        );
    }
}
