//! The retirement plane (RFC 04 §1.2, RFC 09 §6): what a retired key family
//! left behind, and how much of that was even asked about.

use super::asked::Asked;
use super::cutover::CutoverVerdict;
use serde::Serialize;

/// One `[[deprecated]]` ledger entry, judged (issue #226): the four facts of
/// the burn-down, each honest about whether it was even obtainable, and a
/// per-entry verdict in [`CutoverVerdict`]'s vocabulary — `retired` is
/// `cutover` run per ledger line, and a fourth vocabulary would only give the
/// same three states new names.
#[derive(Debug, Clone, Serialize)]
pub struct RetiredEntry {
    /// The producer (or service) whose local slice carries the ledger entry.
    pub producer: String,
    /// The retired subject pattern, as the ledger spells it.
    pub path: String,
    /// Registry version the retirement was recorded in, when the ledger says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// The declared replacement subject, if any (RFC 08 §3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaced_by: Option<String>,
    /// The base-relative wire family this entry maps to. The ledger records
    /// no class, so the class position is `*` — which by D2/D4 still cannot
    /// reach a verbatim plane.
    pub selector: String,
    /// Fact 1: samples heard on the retired family over the listen window.
    /// `NotAsked` = no window ran — not listened is not absent (RFC 09 §5.1
    /// O4).
    #[serde(skip_serializing_if = "Asked::is_not_asked", default)]
    pub wire_samples: Asked<u64>,
    /// Fact 2: a live producer's served introspect slice still declares the
    /// retired path as an **active** subject — the RFC 08 §6.1 lie, a finding
    /// in its own right. `None` = no served slice for this producer answered
    /// (which, for a fully retired producer, is the desired end state).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub still_declared: Option<bool>,
    /// Fact 3: sessions declaring a subscriber intersecting the family
    /// (admin space). `None` = no admin space answered — unknown, not zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscribers: Option<usize>,
    /// Fact 4: samples heard on `replaced_by` over the window — the per-entry
    /// `cutover` pair. `NotAsked` = not listened, or no replacement declared
    /// (a question that does not exist for this entry was not put — the
    /// [`Asked`] reading covers both).
    #[serde(skip_serializing_if = "Asked::is_not_asked", default)]
    pub replacement_samples: Asked<u64>,
    pub verdict: CutoverVerdict,
}

/// The `zenctl check retired` report (issue #226): the deprecation
/// burn-down. The append-only ledger records dozens of individual
/// retirements; this says which ones are actually *finished* — a migration
/// without a burn-down list is a belief.
#[derive(Debug, Clone, Serialize)]
pub struct RetiredReport {
    /// The registry directories the ledger was read from. Stated because the
    /// coverage claim is exactly these files: a `--registry` dir may be one
    /// team's slice of the fleet's ledger, and the report must not read as
    /// fleet totality.
    pub registries: Vec<String>,
    pub entries: Vec<RetiredEntry>,
    /// The listen window, when one ran. `NotAsked` = wire facts were not
    /// asked.
    #[serde(skip_serializing_if = "Asked::is_not_asked", default)]
    pub window_s: Asked<f64>,
    /// Samples heard under `<base>/v1/` over the window — the fleet's proof
    /// of life, which is what lets a silent no-replacement entry pass rather
    /// than a dead fleet passing every silence check for free (RFC 05 §3.1).
    /// Gated with `window_s`.
    #[serde(skip_serializing_if = "Asked::is_not_asked", default)]
    pub plane_samples: Asked<u64>,
    /// Samples the bounded observer missed during the window (O6): non-zero
    /// weakens every silence claim and the report says so. `NotAsked` = no
    /// listen window ran, so there was no observer to miss anything —
    /// matching its sibling wire facts (`window_s`/`plane_samples`); an
    /// unconditional `0` used to claim a clean observation nobody made
    /// (RFC 09 §5.1 O4, review finding R6).
    #[serde(skip_serializing_if = "Asked::is_not_asked", default)]
    pub dropped: Asked<u64>,
    /// Served introspect slices that answered (RFC 08 §6).
    pub introspect_answered: usize,
    /// Declared entities the admin sweep returned; `None` = no admin space
    /// answered (`adminspace.enabled` defaults to false) — "not available",
    /// never "nothing declared" (RFC 09 §5.1 O4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin_entities: Option<usize>,
    /// Worst entry verdict: any failure fails, else any unproven, else pass.
    pub verdict: CutoverVerdict,
}
