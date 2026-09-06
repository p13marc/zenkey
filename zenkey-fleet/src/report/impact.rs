//! Impact attribution as a wire shape (#389, RFC 06 §5.6): which down
//! entities are **roots** and which firing sites are **symptoms** of one —
//! what a notifier writes beside an inhibited notification and what a
//! script reads to ask "cause or symptom". Computed by
//! [`crate::model::impact::attribute`].

use serde::Serialize;

/// A down entity with no down containment ancestor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Root {
    pub entity: String,
    /// Every entity the bounded walk reached from this root, ordered.
    pub reached: Vec<String>,
}

/// A firing or down site that a root explains.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Symptom {
    pub entity: String,
    /// The nearest root (ties by root id, ascending).
    pub explained_by: String,
}

/// The whole attribution, ordered (RFC 06 §5.6: two consumers with the
/// same inputs render the same thing), with what the bounds cost (RFC 13
/// §3 O6): a walk cut at the depth cap, a walk that came back to its own
/// root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImpactReport {
    pub roots: Vec<Root>,
    pub symptoms: Vec<Symptom>,
    /// Walks that still had unvisited edges at the depth cap.
    pub walks_capped: u64,
    /// Downward walks that returned to their own root.
    pub cycles_seen: u64,
    pub depth_cap: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The JSON a script reads: ordered arrays, snake_case counters.
    #[test]
    fn impact_report_json_shape_is_pinned() {
        let r = ImpactReport {
            roots: vec![Root {
                entity: "ent-a".into(),
                reached: vec!["ent-b".into(), "ent-c".into()],
            }],
            symptoms: vec![Symptom {
                entity: "ent-c".into(),
                explained_by: "ent-a".into(),
            }],
            walks_capped: 0,
            cycles_seen: 1,
            depth_cap: 4,
        };
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            serde_json::json!({
                "roots": [{"entity": "ent-a", "reached": ["ent-b", "ent-c"]}],
                "symptoms": [{"entity": "ent-c", "explained_by": "ent-a"}],
                "walks_capped": 0,
                "cycles_seen": 1,
                "depth_cap": 4,
            })
        );
    }
}
