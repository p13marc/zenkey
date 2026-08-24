//! The registry plane (RFC 08 §5/§6): a served slice against a local one,
//! per producer and in aggregate.

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
}

impl RegistryDiff {
    /// Producers whose two sides disagree.
    pub fn disagreeing(&self) -> usize {
        self.producers
            .iter()
            .filter(|p| !p.findings.is_empty())
            .count()
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
