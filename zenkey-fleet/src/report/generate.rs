//! The generator plane (#162): what a mock producer was asked to publish,
//! and what it actually did.
//!
//! Every plan entry and every report here describes traffic that **did not
//! happen on a real fleet**. The synthetic marker rides the samples
//! themselves; these shapes are the paper trail that says a run was a
//! rehearsal.

use serde::Serialize;
use zenkey::schema::TypeSchema;

/// A single deliberate deviation from a known-valid synthesized sample (#163).
///
/// Fault injection is a *mode of the generator*, not a sibling tool: it reuses
/// the whole registry walk, synthesis, scheduling, and guard machinery, then
/// perturbs one dimension of the output **after synthesis** — so the delta
/// from valid is always known, printable (`Fault::delta`), and stamped into
/// the marker (`"fault": "<kind>"`, RFC 09 §5.3). The point is
/// consumer-robustness testing: a consumer that crashes on a truncated payload
/// fails RFC 09 §5.1 O1's spirit — a non-conforming sample is a fact to
/// report, not an error to die on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Fault {
    /// Cut the encoded payload to half its bytes — a partial frame the
    /// decoder meets mid-value.
    Truncate,
    /// Replace the body with a JSON value of the wrong shape for the declared
    /// type (a bare string where a structured type is declared).
    WrongType,
    /// Add an undeclared field to the (JSON) body — the extra key a strict
    /// schema must reject or a lenient one must ignore, never choke on.
    ExtraField,
    /// Publish on a key that matches no registered subject (a trailing chunk
    /// the registry never declared).
    UnregisteredKey,
    /// Publish under a QoS profile other than the subject's declared one
    /// (RFC 04 §3) — the observed-vs-declared mismatch a doctor listen flags.
    WrongQos,
    /// Publish with no wire `Encoding` set, though the subject declares one —
    /// a consumer keyed on the encoding meets a blank.
    MissingEncoding,
    /// Publish a state sample carrying no HLC timestamp: LWW cannot order it
    /// (RFC 04 §4), and freshness is unjudgeable.
    Unstamped,
}

/// One subject the run will publish, fully resolved — the plan is printed
/// before anything touches the bus (the replay dry-run precedent).
#[derive(Debug, Clone, Serialize)]
pub struct GenPlanEntry {
    pub key: String,
    pub class: String,
    pub producer: String,
    pub type_name: String,
    pub qos: String,
    /// `declared` or `default` — where the profile came from (#158's rule:
    /// a generator that picks QoS silently is the write-side O4 mistake).
    pub qos_source: &'static str,
    pub rate_hz: f64,
    /// `describe` / `schema-set` / `placeholder` — where the body shape
    /// came from.
    pub body_source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    /// For `events`: the hard cap on sends within the run (the declared
    /// budget, RFC 04 §1.3) — a generator must not out-shout the registry
    /// it claims to follow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events_cap: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The fault this entry injects (#163), `None` for a conforming entry —
    /// stamped into every one of its samples' markers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fault: Option<Fault>,
    /// The printable delta from valid this fault introduces (#163) — stated
    /// in the plan before a byte moves.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fault_delta: Option<String>,
    #[serde(skip)]
    pub schema: Option<TypeSchema>,
    /// Absolute chunk index (into the full wire key) of the per-send unique
    /// id — set for events, whose keys MUST be write-once (RFC 04 §1.3).
    #[serde(skip)]
    pub unique_chunk: Option<usize>,
}

/// What one run did.
#[derive(Debug, Clone, Serialize)]
pub struct GenReport {
    pub duration_s: f64,
    pub entries: usize,
    pub sent: u64,
    /// Bodies the schema refused to encode — counted, never silently
    /// skipped (these are a bug in the synthesizer or the schema, and
    /// either way the operator hears about it).
    pub refused: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub first_errors: Vec<String>,
}
