//! The describe sweep (RFC 08 §7): ask every producer a slice set names for
//! its served schema set, and keep **every** answer, attributed to the host
//! that gave it (#398, #410).
//!
//! `describe` is asked at `@rpc/*/describe` — a wildcard origin — so it fans
//! in across every host running the producer, and RFC 05 §2.1 attributes each
//! reply by its own key. Keeping one reply per producer is how a schema
//! disagreement came to name a producer and never a host (#398), and this
//! sweep is the one place in the engine that does the asking: the doctor's
//! drift check and `interface show --schema` used to each hold a copy of
//! "do these carriers agree about this type", and only one of them had been
//! fixed (#410).
//!
//! Observations only. Whether the answers *agree* is
//! [`schema_drift`](crate::model::decode::schema_drift)'s verdict, pure over
//! [`DescribeSweep::answers`]; whether a producer that answered nothing is a
//! finding is the doctor's.

use std::time::Duration;

use zenkey::schema::SchemaSet;

use crate::Result;
use crate::bus::query::{Answer, GetOpts, fleet_get};
use crate::model::decode::DescribedSchema;
use crate::model::registry::{SliceSet, rpc_key};

/// What one describe sweep gathered.
#[derive(Debug, Clone, Default)]
pub struct DescribeSweep {
    /// Every parseable answer, attributed. N hosts running one producer are
    /// N entries — the shape [`schema_drift`](crate::model::decode::schema_drift)
    /// compares, and nothing here deduplicates it.
    pub answers: Vec<DescribedSchema>,
    /// The producers for which **no** host gave a parseable answer, in slice
    /// order. A producer where one host answered and another did not is
    /// *described* — RFC 08 §7's SHOULD is met — and the gap between the two
    /// hosts is the drift check's business, not this list's.
    pub undescribed: Vec<String>,
}

impl DescribeSweep {
    /// One set per producer — the **first** host that answered, in arrival
    /// order.
    ///
    /// For the consumers whose question *is* the producer: totality, a
    /// listen window's [`SchemaStore`](crate::model::decode::SchemaStore),
    /// a served count, a field table's declared-path join. Arrival order is
    /// not a fact about the fleet, which is why nothing that judges
    /// *agreement* may read this instead of [`answers`](Self::answers)
    /// (#398).
    pub fn first_per_producer(&self) -> Vec<(String, SchemaSet)> {
        let mut seen = std::collections::BTreeSet::new();
        self.answers
            .iter()
            .filter(|d| seen.insert(d.producer.as_str()))
            .map(|d| (d.producer.clone(), d.set.clone()))
            .collect()
    }
}

/// Ask every producer `slices` names for its `describe` document.
///
/// One GET per slice, each through [`fleet_get`] (target `All`, consolidation
/// `None`) so every host answers and every answer is attributed. A reply that
/// is not a value, or does not parse as a [`SchemaSet`], is dropped here
/// rather than reported: RFC 08 §7 makes `describe` a SHOULD, so an
/// unparseable reply is a producer that has said nothing usable about its
/// types, and it is counted under [`DescribeSweep::undescribed`] when no
/// other host spoke for the producer.
///
/// `slices` decides who is asked, and only those. A caller that already knows
/// which producers carry a type (`interface show --schema`) hands a set of
/// just those and this is never a fleet fan-out.
pub async fn describe_sweep(
    fleet: &crate::Fleet<'_>,
    slices: &SliceSet,
    timeout: Duration,
) -> Result<DescribeSweep> {
    let base = fleet.base();
    let mut answers: Vec<DescribedSchema> = Vec::new();
    let mut undescribed: Vec<String> = Vec::new();
    // One `GetOpts` for the sweep rather than one per producer: the elision
    // ledger is per-options, so a fresh one per slice could never accumulate
    // the fan-out's cost.
    let opts = GetOpts::new(timeout);
    for slice in slices.slices() {
        let key = rpc_key(base, slice, "describe")?;
        let replies = fleet_get(fleet, &key, &opts).await?;
        let before = answers.len();
        for reply in replies {
            let origin = reply.origin;
            let Answer::Value(bytes) = reply.answer else {
                continue;
            };
            let cow = bytes.to_bytes();
            let Some(set) = std::str::from_utf8(&cow)
                .ok()
                .and_then(|t| SchemaSet::parse(t).ok())
            else {
                continue;
            };
            answers.push(DescribedSchema::new(origin, slice.name.clone(), set));
        }
        if answers.len() == before {
            undescribed.push(slice.name.clone());
        }
    }
    Ok(DescribeSweep {
        answers,
        undescribed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey::schema::TypeSchema;

    fn set(name: &str) -> SchemaSet {
        SchemaSet::builder("t")
            .entry(
                name,
                TypeSchema::json_schema(serde_json::json!({"type": "object"})),
            )
            .build()
    }

    /// The fold to one set per producer keeps arrival order and drops
    /// nothing from `answers` — the two lists answer different questions.
    #[test]
    fn first_per_producer_keeps_the_first_host_and_every_answer_stays() {
        let sweep = DescribeSweep {
            answers: vec![
                DescribedSchema::new("h-aaaaaaaaaaaa", "sysinfo", set("A")),
                DescribedSchema::new("h-bbbbbbbbbbbb", "sysinfo", set("B")),
                DescribedSchema::new("h-aaaaaaaaaaaa", "gnmi", set("C")),
            ],
            undescribed: vec![],
        };
        let first = sweep.first_per_producer();
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].0, "sysinfo");
        assert!(first[0].1.get("A").is_some(), "the first host's set wins");
        assert_eq!(first[1].0, "gnmi");
        assert_eq!(sweep.answers.len(), 3, "the attributed list is untouched");
    }
}
