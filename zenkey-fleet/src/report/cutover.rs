//! The cutover plane (RFC 09 §6): did a migration finish?
//!
//! [`CutoverVerdict`] has three states, not two, and that is the whole
//! point: "the old family is quiet on a fleet that is provably speaking"
//! and "everything is quiet" are different facts, and only the first is
//! evidence.

use serde::Serialize;

/// The RFC 09 §6 cutover-acceptance verdict (issue #59). Three states, not
/// two: "the old family is quiet on a fleet that is provably speaking" and
/// "everything is quiet" are different facts, and only the first is
/// evidence a migration finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CutoverVerdict {
    /// Old root silent, new plane carrying traffic — both halves held.
    Pass,
    /// The retired family still speaks — the migration is not done.
    OldStillSpeaks,
    /// The old root was silent but so was the new plane: a non-verdict
    /// (RFC 05 §3.1) — a dead fleet passes the silence half for free.
    Unproven,
}

impl CutoverVerdict {
    /// The [`Judgement`](crate::report::Judgement) mapping (RFC 13,
    /// v1.24). The judged claim is the finding — "the retired family still
    /// speaks":
    ///
    /// | verdict | judgement | exit (RFC 13) |
    /// |---|---|---|
    /// | `OldStillSpeaks` | `Established` (finding) | 1 |
    /// | `Pass` | `NotEstablished` (clean) | 0 |
    /// | `Unproven` | `Unobservable` | 2 |
    pub fn to_judgement(self) -> crate::report::Judgement {
        use crate::report::Judgement;
        match self {
            CutoverVerdict::OldStillSpeaks => Judgement::Established,
            CutoverVerdict::Pass => Judgement::NotEstablished {
                reason: "the retired family is silent while the new plane carries traffic".into(),
            },
            CutoverVerdict::Unproven => Judgement::Unobservable {
                reason: "both planes were silent — a dead fleet passes the silence half \
                         for free (RFC 05 §3.1)"
                    .into(),
            },
        }
    }
}

/// The inverse of [`CutoverVerdict::to_judgement`]. Both unestablished poles
/// fold to `Unproven`: a question that was not put (or could not be carried)
/// proves no migration.
impl From<crate::report::Judgement> for CutoverVerdict {
    fn from(j: crate::report::Judgement) -> CutoverVerdict {
        use crate::report::Judgement;
        match j {
            Judgement::Established => CutoverVerdict::OldStillSpeaks,
            Judgement::NotEstablished { .. } => CutoverVerdict::Pass,
            Judgement::NotAsked | Judgement::Unobservable { .. } => CutoverVerdict::Unproven,
        }
    }
}

/// The `zenctl cutover` report (issue #59; RFC 09 §6 half one).
#[derive(Debug, Clone, Serialize)]
pub struct CutoverReport {
    pub old_root: String,
    /// The stated meaning of "new plane": keys under this prefix. Stated,
    /// not inferred — the version chunk is plain, so key algebra cannot
    /// separate old from new (RFC 09 §6's note).
    pub new_prefix: String,
    pub window_s: f64,
    /// Samples heard on the old root — every one is a failure fact.
    pub old_samples: u64,
    pub old_keys_seen: usize,
    /// Up to a cap of offending keys, with per-key counts.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub old_examples: Vec<String>,
    /// Samples on the new plane over the window.
    pub new_samples: u64,
    /// Samples that were neither: outside `<base>/v1/` and not the old
    /// root. Leaks by this check's stated definition.
    pub leak_samples: u64,
    pub leaked_keys_seen: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub leak_examples: Vec<String>,
    /// Samples the bounded observer missed (O6): non-zero weakens the
    /// silence claim and the report says so.
    pub dropped: u64,
    pub verdict: CutoverVerdict,
}
