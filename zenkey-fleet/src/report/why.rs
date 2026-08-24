//! The `why` plane (#214): the ladder a silence is explained by.
//!
//! One [`Rung`] per question, each carrying a [`RungAnswer`] — which is the
//! judgement core itself, so "not asked" and "asked and unobservable" survive
//! all the way onto the wire instead of collapsing into a missing field.
//! [`WhyVerdict`] is the surface naming, and it is the documented case of
//! inverted polarity: `Explained` is the *finding*, and its CLI exits 0.

use serde::Serialize;

use crate::judge::why::is_cause;

/// One rung's answer — the [`Judgement`](crate::report::Judgement) core
/// (RFC 13, v1.24; RFC 09 §5.1 pre-v1.24), carried directly: since v1.24 the
/// ladder's three shipped states *are* three of the core's four poles, and
/// this alias is the fold. The serde tags are byte-identical to what #214
/// shipped (`established` / `not_established` + `reason` / `not_asked`).
///
/// A rung's judgement is over **its own question** (the rung's fact), not
/// over "is there a finding?" — which of its poles constitutes a finding is
/// per-rung policy, and [`is_cause`](crate::judge::why::is_cause) is where that policy lives. The rungs
/// currently never answer [`Unobservable`](crate::report::Judgement::Unobservable): an observation the
/// ladder could not obtain degrades the rung to `NotAsked` and rides
/// [`WhyReport::impairments`] instead.
///
/// A rung whose input was not fetched says
/// [`NotAsked`](crate::report::Judgement::NotAsked), never
/// `NotEstablished` (RFC 09 §5.1 O4).
pub type RungAnswer = crate::report::Judgement;

/// One rung of the ladder.
#[derive(Debug, Clone, Serialize)]
pub struct Rung {
    /// From [`RUNG_IDS`](crate::judge::common::RUNG_IDS) — stable, script-keyable.
    pub id: &'static str,
    /// The question this rung puts, as prose.
    pub question: &'static str,
    #[serde(flatten)]
    pub answer: RungAnswer,
    /// What the answer rests on, one fact per line.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
}

/// The report's overall reading — what the CLI exits with (see the module
/// doc's exit table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WhyVerdict {
    /// An explanation of the silence was established (exit 0).
    Explained,
    /// No cause, and everything checked looks healthy (exit 1).
    Healthy,
    /// No cause, and the observation was impaired: an input this ladder
    /// wanted could not be obtained, so "healthy" cannot be claimed (exit 2).
    Impaired,
}

impl WhyVerdict {
    /// The [`Judgement`](crate::report::Judgement) mapping (RFC 13,
    /// v1.24), and it is **THE inverted one — read this before wiring exit
    /// codes**: `Explained` is *established-finding* (`Established`), because
    /// the thing `why` establishes is a cause — a finding about the fleet —
    /// even though this family's own historical CLI contract exits **0** for
    /// it (the module doc's table). The RFC 13 exit projection
    /// ([`crate::report::judgement_exit_code`]) therefore gives `why`'s
    /// three verdicts 1 / 0 / 2 in this order — the flip between the two
    /// contracts is carried **here, at the mapping**, never special-cased by
    /// a consumer downstream.
    ///
    /// | verdict | judgement | RFC 13 exit | historical `zenctl why` exit |
    /// |---|---|---|---|
    /// | `Explained` | `Established` (finding) | 1 | 0 |
    /// | `Healthy` | `NotEstablished` (clean) | 0 | 1 |
    /// | `Impaired` | `Unobservable` | 2 | 2 |
    pub fn to_judgement(self) -> crate::report::Judgement {
        use crate::report::Judgement;
        match self {
            WhyVerdict::Explained => Judgement::Established,
            WhyVerdict::Healthy => Judgement::NotEstablished {
                reason: "no cause established, and everything checked looks healthy".into(),
            },
            WhyVerdict::Impaired => Judgement::Unobservable {
                reason: "an input the ladder wanted could not be obtained — \"healthy\" \
                         cannot be claimed over questions it could not ask"
                    .into(),
            },
        }
    }
}

/// The inverse of [`WhyVerdict::to_judgement`], same (inverted) polarity:
/// an established finding is `Explained`, established-clean is `Healthy`,
/// and both unestablished poles fold to `Impaired` — a ladder nobody asked
/// is exactly a ladder that cannot claim health.
impl From<crate::report::Judgement> for WhyVerdict {
    fn from(j: crate::report::Judgement) -> WhyVerdict {
        use crate::report::Judgement;
        match j {
            Judgement::Established => WhyVerdict::Explained,
            Judgement::NotEstablished { .. } => WhyVerdict::Healthy,
            Judgement::NotAsked | Judgement::Unobservable { .. } => WhyVerdict::Impaired,
        }
    }
}

/// The ladder, assembled. One rung per [`RUNG_IDS`](crate::judge::common::RUNG_IDS) entry, in order, always —
/// a rung is never omitted, it degrades to `NotAsked`.
#[derive(Debug, Clone, Serialize)]
pub struct WhyReport {
    /// The key (or selector) as asked, verbatim.
    pub key: String,
    /// The base the ladder judged under. Empty is the bus-root deployment.
    pub base: String,
    pub rungs: Vec<Rung>,
    pub verdict: WhyVerdict,
    /// Inputs the ladder wanted and could not obtain — what makes a
    /// no-cause run [`WhyVerdict::Impaired`] rather than healthy.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub impairments: Vec<String>,
    /// The listen window that ran, seconds. Absent = not listened — which
    /// the `wire-heard` rung states rather than hiding (O4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listened_s: Option<f64>,
}

impl WhyReport {
    /// The rung ids whose answers established a cause — what exit 0 rests on.
    pub fn causes(&self) -> Vec<&'static str> {
        self.rungs
            .iter()
            .filter(|r| is_cause(r.id, &r.answer))
            .map(|r| r.id)
            .collect()
    }
}
