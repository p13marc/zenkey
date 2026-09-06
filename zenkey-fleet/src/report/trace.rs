//! The RPC trace window (#215): one call, then everything observed on the
//! called origin for a window after it — each sample tagged by how far the
//! registry can *relate* it to the procedure, never by what caused it.
//!
//! RFC 05 §3's long-running idiom is a declared causal chain — `GET
//! @rpc/<p>/artifact/request` → `state/<p>/artifact/<kind>` →
//! `events/<p>/artifact/<ulid>` → `@blob` — and nothing followed it: an
//! operator called a write procedure and then hunted three panes for what it
//! did. This suite makes the call, so it owns the request instant; the
//! grammar fixes the origin's position, so "this origin's keyspace" is one
//! selector; the registry declares which subjects the producer owns. Those
//! three facts are what a trace has. What it does **not** have is causality:
//! the wording throughout is *observed after the call*, never *caused*, and
//! there is no edge, no arrow, and no trace-id attachment — the ratified
//! posture forecloses one.
//!
//! [`TraceReport::subscribed_before_call`] is always `true` and is pinned
//! anyway: the order of operations — subscribe, then call, then hold — is
//! normative (RFC 09 §5.1 O4). A window opened *after* the call converts
//! "not asked" into "no", and a refactor that inverts the order must change
//! the document to do it.

use serde::Serialize;

use crate::report::{CallReport, RowKind};

/// What the two Δ columns are measured against, and what excluded the
/// `@blob` bytes — spelled once each, so the report and the renderer cite
/// the same sentence.
pub const TRACE_CHAIN_RULE: &str =
    "first-chunk naming heuristic (RFC 05 §3 idiom); a naming coincidence is tagged the same way";

/// The planes a `**` window cannot reach (RFC 03 §4 D2), as the report
/// states it. The artifact bytes on `@blob` are outside the trace by
/// construction — excluded, not empty — and the window is deliberately not
/// widened to reach them: the idiom's own step 4 is a pull, not a sample.
pub const TRACE_EXCLUDED: &str = "the verbatim planes (`@rpc`, `@blob`, `@media`, `@adv`, \
     `@catalog`): `**` never crosses an `@`-chunk (RFC 03 §4 D2), so the artifact bytes on \
     `@blob` are outside this window — excluded, not empty";

/// How a same-origin sample relates to the procedure that was called.
///
/// Three states, and the third is the one a `bool` cannot spell: with no
/// registry loaded the chain is *unjudgeable*, which is not "undeclared"
/// (RFC 09 §5.1 O4 — [`crate::Registration::Unknown`] kept distinct from
/// [`crate::Registration::Unregistered`], one layer up).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceRelation {
    /// The registry refines the subject under the called producer **and**
    /// its first chunk is the procedure's first chunk (`artifact/request`
    /// ↔ `state/<p>/artifact/<kind>`). A naming heuristic, stated as one in
    /// [`TRACE_CHAIN_RULE`]: a coincidence of names is tagged the same way.
    DeclaredChain,
    /// Same origin; the registry does not put it in the chain — a different
    /// producer, a different first chunk, or an unregistered subject.
    SameOriginUndeclared,
    /// Same origin, and no registry was loaded, so the chain could not be
    /// judged either way.
    SameOriginRegistryNotLoaded,
}

/// What the HLC Δ column is measured against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HlcReference {
    /// The reply sample carried an HLC; every stamped effect's `hlc_delta_ms`
    /// is measured from it.
    Reply,
    /// The reply carried none — the caller's own session mints no HLC (see
    /// [`crate::model::timeline`]) — so no `hlc_delta_ms` is computed at all,
    /// and the column is absent rather than defaulted to arrival.
    None,
}

/// One sample observed on the called origin after the call.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceRow {
    pub key: String,
    pub relation: TraceRelation,
    /// Arrival on this observer's monotonic clock, ms since `t0` — the
    /// instant after the watches were declared and before the GET left.
    /// Always present.
    pub arrival_delta_ms: f64,
    /// The sample's HLC, `<ntp64>/<stamper>`, when it carried one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hlc: Option<String>,
    /// Sample HLC minus the reply's HLC, ms — present only when both exist.
    /// Signed: a negative value is a sample stamped *before* the reply on
    /// the stamper's clock, which is a fact to show, not to clamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hlc_delta_ms: Option<i64>,
    /// Who stamped `hlc`: `self`, `foreign:<id>` or `unattributable:<id>`
    /// (RFC 09 §5.1 O7). Present exactly when `hlc` is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stamped_by: Option<String>,
    pub kind: RowKind,
    pub payload_bytes: usize,
    /// Samples this observer missed while behind, between the previous row
    /// of this lane and this one (O6). A break rides the row that follows
    /// it in *every* lane, because the broadcast does not know whose
    /// samples it lost.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub break_before: Option<u64>,
}

/// Samples from **other** origins during the window: a count and a few
/// keys, never rows. They are stated so that the attributed lane cannot be
/// read as "the only thing that happened" — and they are attributed to
/// nothing, because nothing in hand can.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConcurrentLane {
    pub samples: u64,
    /// Distinct keys among them.
    pub keys: u64,
    /// The first few keys, capped at [`crate::EXPANSION_CAP`].
    pub examples: Vec<String>,
    /// Samples the fleet-wide watch dropped while behind — its own counter,
    /// so a busy fleet's lag never reads as a break in the origin's lanes.
    pub dropped: u64,
}

/// The call, then what was observed after it.
#[derive(Debug, Clone, Serialize)]
pub struct TraceReport {
    pub call: CallReport,
    /// The selectors actually watched (O5): the origin's subtree, then the
    /// fleet-wide window the concurrent lane is counted from.
    pub scopes: Vec<String>,
    /// [`TRACE_EXCLUDED`].
    pub excluded: &'static str,
    pub window_s: f64,
    /// Always `true`; pinned so that inverting the order changes the document.
    pub subscribed_before_call: bool,
    /// `t0` on the wall clock, seconds since the Unix epoch — the one wall
    /// reading in the report, so a script can place the window beside logs.
    pub t0_unix_s: f64,
    /// When the GET returned, ms after `t0`: the reply arrived at or before
    /// this instant (the fan-in waits for the query to finalize, which is not
    /// the reply's own arrival — that instant the query layer does not hand
    /// out).
    pub call_returned_ms: f64,
    pub hlc_reference: HlcReference,
    /// The reply's HLC, `<ntp64>/<stamper>`, when `hlc_reference` is `reply`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_hlc: Option<String>,
    /// [`TRACE_CHAIN_RULE`].
    pub chain_rule: &'static str,
    /// The procedure's declared `kind` token — `long-running`, `write`,
    /// `read` — or `undeclared` when no loaded slice declares it.
    pub idiom: String,
    /// Same origin, in the declared chain — in arrival order.
    pub attributed: Vec<TraceRow>,
    /// Same origin, not in the chain (or not judgeable) — in arrival order.
    pub same_origin: Vec<TraceRow>,
    pub concurrent: ConcurrentLane,
    /// Samples the origin's watch dropped while behind (O6); each is also a
    /// `break_before` on the next row of every lane.
    pub dropped: u64,
    /// Keys the origin watch's statistics table retired during the window.
    pub keys_evicted: u64,
}

impl TraceReport {
    /// The call's own exit code, unchanged: a trace is an act's observation,
    /// not a judgement, and nothing seen in the window moves it
    /// ([`CallReport::exit_code`]).
    pub fn exit_code(&self) -> i32 {
        self.call.exit_code()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exit code is the call's: an empty window after a clean reply is
    /// still 0, and a refused call is 1 however busy the origin was.
    #[test]
    fn a_trace_exits_as_its_call_does() {
        let call = |answers| CallReport {
            key: "k".into(),
            timeout_s: 5.0,
            answers,
        };
        let report = |call| TraceReport {
            call,
            scopes: vec![],
            excluded: TRACE_EXCLUDED,
            window_s: 10.0,
            subscribed_before_call: true,
            t0_unix_s: 0.0,
            call_returned_ms: 0.0,
            hlc_reference: HlcReference::None,
            reply_hlc: None,
            chain_rule: TRACE_CHAIN_RULE,
            idiom: "undeclared".into(),
            attributed: vec![],
            same_origin: vec![],
            concurrent: ConcurrentLane {
                samples: 0,
                keys: 0,
                examples: vec![],
                dropped: 0,
            },
            dropped: 0,
            keys_evicted: 0,
        };
        assert_eq!(report(call(vec![])).exit_code(), 2);
        assert_eq!(
            report(call(vec![crate::report::CallAnswer {
                origin: "h-1".into(),
                outcome: crate::report::CallOutcome::Ok {
                    value: None,
                    text: None,
                },
                attachment: None,
                attachment_bytes: None,
            }]))
            .exit_code(),
            0
        );
    }

    /// Every optional on a row is absent, never null: the arrival Δ is the
    /// one column every sample has.
    #[test]
    fn an_unstamped_row_carries_only_its_arrival() {
        let row = TraceRow {
            key: "v1/h-3fa9c2d41b7e/telemetry/other/noise".into(),
            relation: TraceRelation::SameOriginUndeclared,
            arrival_delta_ms: 1.5,
            hlc: None,
            hlc_delta_ms: None,
            stamped_by: None,
            kind: RowKind::Put,
            payload_bytes: 3,
            break_before: None,
        };
        assert_eq!(
            serde_json::to_value(&row).unwrap(),
            serde_json::json!({
                "key": "v1/h-3fa9c2d41b7e/telemetry/other/noise",
                "relation": "same_origin_undeclared",
                "arrival_delta_ms": 1.5,
                "kind": "put",
                "payload_bytes": 3
            })
        );
    }
}
