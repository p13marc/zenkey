//! The registry plane (RFC 08 §5/§6): a served slice against a local one,
//! per producer and in aggregate.

use super::asked::Asked;
use serde::Serialize;

/// One producer, as the bus serves it versus as the checkout declares it
/// (issue #50). A `None` version means "not present on that side", which is a
/// fact with two very different explanations — the findings say which.
#[derive(Debug, Clone, Serialize)]
pub struct ProducerDiff {
    pub producer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub served_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_version: Option<String>,
    /// RFC 08 §6 findings, rendered. Empty = the two agree.
    pub findings: Vec<String>,
}

/// `zenctl registry diff` (issue #50).
#[derive(Debug, Clone, Serialize)]
pub struct RegistryDiff {
    pub producers: Vec<ProducerDiff>,
    /// Producers several origins answered for, and whether they agreed
    /// (#399).
    ///
    /// The diff is computed from **one slice per producer**, so where the
    /// fleet is mid-rollout it is computed from one arbitrary host's answer.
    /// Without this the result read as fleet-wide truth: two hosts on last
    /// month's registry and three on this month's produced a diff with no
    /// indication that four other answers existed, let alone disagreed.
    ///
    /// `NotAsked` = the served side did not come from the bus, so there was
    /// no origin to collapse and the question was never put; `Asked([])` =
    /// asked, and every producer had exactly one origin answer. Empty must
    /// not read as agreement (RFC 13 §3 O4).
    #[serde(skip_serializing_if = "Asked::is_not_asked", default)]
    pub collapsed: Asked<Vec<CollapsedProducer>>,
}

impl RegistryDiff {
    /// Producers whose two sides disagree.
    pub fn disagreeing(&self) -> usize {
        self.producers
            .iter()
            .filter(|p| !p.findings.is_empty())
            .count()
    }

    /// Producers the fleet does not agree with *itself* about (#399).
    ///
    /// A different question from [`disagreeing`](Self::disagreeing), which is
    /// the fleet against the checkout. `NotAsked` yields zero, and a caller
    /// that renders the number must say which zero it is.
    pub fn self_disagreeing(&self) -> usize {
        self.collapsed
            .as_deref()
            .map(|c| c.iter().filter(|c| !c.agreed).count())
            .unwrap_or(0)
    }
}

/// One producer where the served slice and the on-disk slice disagree.
///
/// A disagreement is **data**, not an error: served wins in the union (the
/// bus is the runtime truth, RFC 08 §6.1), and the difference is retained for
/// `doctor` to report instead of being silently overwritten.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SliceDisagreement {
    pub producer: String,
    pub bus_version: String,
    pub dirs_version: String,
    /// Whether anything beyond the version string differs (subjects,
    /// procedures, blob tiers).
    pub shape_differs: bool,
}

/// What one producer's fleet-wide answers looked like *before* a
/// [`SliceSet`](crate::SliceSet) kept one of them (#385).
///
/// A set is indexed by producer name, so N hosts running one producer
/// collapse to one entry. For a decoder that is right — refining a key needs
/// *a* slice per producer and which host served it is irrelevant. What was
/// wrong is that the collapse was silent: the resulting set looked complete
/// and was one arbitrary host's answer, and a `diff` computed from it read
/// as fleet-wide truth. This is the receipt.
///
/// It lives here rather than beside the fold that builds it because it is
/// rendered: `zenctl registry diff --format json` carries it, so it is a wire
/// shape, and every wire shape in this crate lives under `report/` (#399).
///
/// It loses the `#[non_exhaustive]` it carried in `model/` on the same move,
/// and for the same reason: nothing under `report/` has one. A wire shape is
/// pinned by a test rather than by a marker, and a shape a fixture crate
/// cannot spell is one no corpus can pin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CollapsedProducer {
    /// The producer (or service) base name the collapse happened on.
    pub producer: String,
    /// Every origin that served a slice for it, in reply order.
    pub origins: Vec<String>,
    /// The `[registry] version` each of those served, index-parallel with
    /// [`origins`](Self::origins).
    pub versions: Vec<String>,
    /// Whether every origin served byte-identical TOML.
    ///
    /// `false` is the finding: the fleet does not agree about what this
    /// producer declares — mid-rollout, or a host running last month's build
    /// — and this set kept one of the answers. Which one is arrival order,
    /// which is not a fact about the fleet.
    pub agreed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The receipt is a wire shape the moment `registry diff --format json`
    /// carries it (#399), and the half that matters is the absence: a served
    /// side that never came off the bus must serialize *no* `collapsed` key,
    /// not an empty list, because an empty list is the answer "asked, and
    /// nothing collapsed" (RFC 13 §3 O4).
    #[test]
    fn the_collapse_receipt_keeps_not_asked_off_the_wire() {
        let asked = RegistryDiff {
            producers: vec![],
            collapsed: Asked::Asked(vec![CollapsedProducer {
                producer: "sysinfo".into(),
                origins: vec!["h-3fa9c2d41b7e".into(), "h-8b1e07af22c9".into()],
                versions: vec!["1.1".into(), "1.0".into()],
                agreed: false,
            }]),
        };
        assert_eq!(
            serde_json::to_value(&asked).expect("serialize"),
            serde_json::json!({
                "producers": [],
                "collapsed": [{
                    "producer": "sysinfo",
                    "origins": ["h-3fa9c2d41b7e", "h-8b1e07af22c9"],
                    "versions": ["1.1", "1.0"],
                    "agreed": false,
                }],
            })
        );
        assert_eq!(asked.self_disagreeing(), 1);

        let asked_clean = RegistryDiff {
            producers: vec![],
            collapsed: Asked::Asked(vec![]),
        };
        assert_eq!(
            serde_json::to_value(&asked_clean).expect("serialize"),
            serde_json::json!({"producers": [], "collapsed": []}),
            "asked and nothing collapsed is a present, empty list"
        );

        let not_asked = RegistryDiff {
            producers: vec![],
            collapsed: Asked::NotAsked,
        };
        assert_eq!(
            serde_json::to_value(&not_asked).expect("serialize"),
            serde_json::json!({"producers": []}),
            "not asked is absence — never `[]`, never `null`"
        );
        assert_eq!(
            not_asked.self_disagreeing(),
            0,
            "and its zero is a different zero, which the renderer says out loud"
        );
    }
}
