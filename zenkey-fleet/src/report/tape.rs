//! The tape plane (RFC 09 §5.2): the `.zrec` header, the row dialect a
//! capture is made of, and what a capture or a replay reports afterwards.
//!
//! [`ZrecHeader`] — with the two version-2 blocks it may carry,
//! [`PreambleInfo`] and [`PreRollInfo`] — is read as well as written: a
//! `.zrec` on disk outlives the process that wrote it, so the header is a
//! contract in both directions and derives `Deserialize`. (The trigger
//! record a version-2 file interleaves is [`super::Transition`], the
//! watchdog's own shape, which reads back for the same reason.)

use serde::{Deserialize, Serialize};

/// The first line of a `.zrec` file: what was asked, under which base, and
/// when (RFC 09 §5.1 O4 — a capture names its question). The `base` is the
/// operator's *stated* deployment base at capture time; recorded keys are
/// full wire keys and are never re-derived from it (O3).
///
/// `PartialEq` only, since v1.34: the version-2 blocks carry measured
/// spans as `f64`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZrecHeader {
    /// Format version ([`ZREC_VERSION`](crate::tape::record::ZREC_VERSION)).
    pub zrec: u32,
    /// The full wire selectors the capture watched. A wildcard selector
    /// never crosses an `@`-chunk, so a `**` capture excludes the verbatim
    /// planes by construction (O5) — the reader states that rather than
    /// letting the file claim "everything".
    pub selectors: Vec<String>,
    /// The deployment base the operator resolved at capture time
    /// (may be empty: the base-less bus-root deployment).
    pub base: String,
    /// Capture start, RFC 3339 wall clock — provenance, not a pacing clock
    /// (pacing rides each row's `t`).
    pub captured_at: String,
    /// Version 2 (RFC 13 §4.1, v1.34; #218): the state preamble this
    /// capture carries — the bounded fetch that produced the `preamble`
    /// rows and what it could not fetch. Absent on a version-1 file and on
    /// a capture that asked for none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preamble: Option<PreambleInfo>,
    /// Version 2: the retained window this capture's pre-roll came from —
    /// what was asked, what the ring could give, and the ring's two
    /// eviction kinds kept apart (O6). Absent on a live capture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_roll: Option<PreRollInfo>,
}

/// What a version-2 preamble is a snapshot *of* (RFC 13 §4.3's pre-roll
/// bullet): the values at the moment the ring began are not recoverable,
/// so the honest substitute has to be named rather than implied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreambleSemantics {
    /// The current state of what the ring cannot show: only keys **absent**
    /// from the retained window were fetched at trigger time. A key the
    /// ring holds already has its story in the pre-roll rows.
    AbsentFromWindow,
    /// The full current state under the watched selectors at trigger time,
    /// ring or no ring — every fetched key, so a reader that seeds from the
    /// preamble alone has the whole base.
    Full,
}

/// The state preamble's account of itself (RFC 13 §4.1, version 2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreambleInfo {
    /// Preamble rows written — counted apart from observed rows (O6).
    pub count: u64,
    /// How long the fan-in GET took: a preamble is collected *over* a span,
    /// never at an instant (RFC 13 §4.4's obligation, inherited).
    pub collected_over_s: f64,
    /// The selectors actually fetched — the state-class projection of the
    /// watched set, so a watch that reaches no `state` key fetched nothing.
    pub selectors: Vec<String>,
    pub semantics: PreambleSemantics,
    /// Replies the bounded fetch could not keep or could not read: error
    /// envelopes plus replies elided past the reply bound (O6).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub incomplete: u64,
    /// Watched selectors whose state could not be fetched at all — a GET
    /// that could not be issued, or a selector that reaches no `state`
    /// key — named rather than folded into a count (O5).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed: Vec<String>,
}

/// The retained window's account of itself at trigger time (RFC 13 §4.3's
/// pre-roll bullet): what `--pre` asked for and what the ring could give.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreRollInfo {
    /// The pre-roll the operator asked for, seconds.
    pub asked_s: f64,
    /// The span the ring actually held when the trigger fired — shorter
    /// than `asked_s` while the ring was still filling, or when the byte
    /// budget bit (`evicted`).
    pub covered_s: f64,
    /// The selectors the ring was fed under: the pre-roll covers these and
    /// nothing wider (O5).
    pub watched: Vec<String>,
    /// Samples the ring dropped because its **byte** budget bit — the
    /// window is then narrower than `asked_s` claims (O6).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub evicted: u64,
    /// Samples that aged past the pre-roll — the window sliding as
    /// declared, counted apart from `evicted` (v1.18 R1 forbids the fold).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub expired: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// What a capture did — the shared report shape both frontends render.
#[derive(Debug, Clone, Serialize)]
pub struct RecordReport {
    /// The header as written: a capture names its question (O4).
    pub header: ZrecHeader,
    /// Where the capture went, when it went to a file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out: Option<String>,
    /// Samples written.
    pub samples: u64,
    /// Samples the capture missed while behind — stored in the file as
    /// interleaved drop records *and* totalled here (O6).
    pub dropped: u64,
    /// Wall-clock capture length.
    pub duration_ms: u64,
    /// The transition that fired a triggered capture (#218) — absent on a
    /// plain capture, and on a triggered run that never fired.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger: Option<super::Transition>,
    /// The state preamble written ahead of the pre-roll, as the header
    /// states it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preamble: Option<PreambleInfo>,
    /// The retained window the pre-roll came from, as the header states it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_roll: Option<PreRollInfo>,
    /// Preamble rows written — never added to `samples` (O6, applied to
    /// rows: the kinds are counted apart).
    #[serde(skip_serializing_if = "is_zero")]
    pub preamble_rows: u64,
}

/// What a replay did — the shared report shape both frontends render.
#[derive(Debug, Clone, Serialize)]
pub struct ReplayReport {
    /// The capture header, echoed: a replay names what it replayed.
    pub header: ZrecHeader,
    pub dry_run: bool,
    pub speed: f64,
    /// Rows published (dry run: rows that would have been).
    pub published: u64,
    /// Tombstones sent (dry run: would have been).
    pub tombstones: u64,
    /// Rows that could not be parsed — counted, never skipped.
    pub malformed: u64,
    /// Delete rows the retire gate refused.
    pub refused: u64,
    /// Samples the *capture* missed (summed from the file's drop records):
    /// this replay is a partial view and says so (O6).
    pub capture_dropped: u64,
    /// The first few malformed/refused reasons, for the human render.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub first_errors: Vec<String>,
    /// Version-2 preamble rows **not** published — the default: re-stamping
    /// state-at-capture-start republishes a snapshot over the live fleet
    /// (RFC 13 §4.2), so the replayer skips them unless told `--seed-state`
    /// and says how many.
    #[serde(skip_serializing_if = "is_zero")]
    pub preamble_skipped: u64,
    /// Preamble rows published (dry run: would have been) under
    /// `--seed-state`, through the same retire gate as any row.
    #[serde(skip_serializing_if = "is_zero")]
    pub preamble_seeded: u64,
    /// Trigger records met in the file — never published; a marker, not a
    /// row.
    #[serde(skip_serializing_if = "is_zero")]
    pub triggers: u64,
}

/// One sample, as the explorers write it (#235).
///
/// The write side of this module's dialect: `.zrec` rows (RFC 09 §5.2),
/// `zenctl echo --format ndjson`, and zengui's echo export are all
/// this struct, so [`parse_row`](crate::tape::ingest::parse_row) reads back what any of them wrote. Every
/// optional field is `skip_serializing_if`: a writer that does not hold a
/// fact omits it rather than nulling it, because a `null` here would claim
/// a question was asked and answered negatively (RFC 09 §5.1 O4). That is
/// also what keeps the three writers' rows a *subset* relationship rather
/// than three shapes — a `.zrec` row carries no `origin`, an echo row
/// carries no pacing offset, and neither is lying about the other.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SampleRow {
    /// Full wire key, as received — explorers run un-namespaced (RFC 09 §5).
    pub key: String,
    /// The convention-parsed origin chunk, when the key parses at all.
    ///
    /// Absent means the key did not parse under the observer's base, which
    /// is a fact about the key and not a claim about the fleet (O1/O3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// The parsed subject tail, joined — absent on the same terms as
    /// [`SampleRow::origin`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Microseconds since the capture epoch: the **observer's arrival
    /// clock**, and the only thing replay paces by (RFC 09 §5.2). A live
    /// stream has no epoch to be relative to, so it omits this.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t: Option<u64>,
    /// The registry-declared type name, when a decode was asked for and
    /// resolved one. `--no-decode` never asks, so it omits this rather than
    /// nulling it (O4).
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    /// Whether [`SampleRow::value`] is a schema decode rather than a
    /// structural rendering. Absent when nothing was decoded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub typed: Option<bool>,
    /// The sample's declared encoding, verbatim (RFC 08 §7: sample beats
    /// registry beats sniff).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    /// The sample's HLC, when one rode it. Whose clock it is depends on who
    /// stamped it (RFC 09 §5.1 O7); this field says only that it exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// The RFC 04 §3 QoS profile **name**, and only when the wire's actual
    /// axes match one.
    ///
    /// This is the field [`parse_row`](crate::tape::ingest::parse_row) resolves through
    /// `zenkey::qos::QosProfile::from_name`, so it must never carry
    /// anything else — axes matching no profile are not approximated, they
    /// ride [`SampleRow::qos_axes`] instead. Writing the axes here is
    /// exactly the bug in #235.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qos: Option<String>,
    /// The wire's actual QoS axes as one token,
    /// `priority/congestion/reliability[+express]` (#120).
    ///
    /// A fact worth carrying and *not* a profile name: a fleet is free to
    /// publish axes no profile declares, and the declared-vs-observed
    /// comparison is the point.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qos_axes: Option<String>,
    /// A tombstone: authoritative retirement, never an empty put
    /// (RFC 04 §1.2). Always written — every sample is one or the other,
    /// and that is a fact rather than an unanswered question.
    pub delete: bool,
    /// Base64 of the exact wire payload — lossless and round-trippable,
    /// which is why [`parse_row`](crate::tape::ingest::parse_row) prefers it over [`SampleRow::value`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<String>,
    /// The payload as a *rendering*: a schema decode when one was asked
    /// for and succeeded, else the structural degradation. It does not
    /// round-trip a binary payload, and RFC 09 §5.2 says so.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    /// True payload size, whatever the rendering above shows.
    ///
    /// Deliberately not spelled `bytes`: that key has meant the base64
    /// payload since RFC 09 §5.2, and a byte count under it makes the row
    /// unreadable rather than merely lossy (#235).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_bytes: Option<usize>,
    /// The publishing entity, `zid:eid#sn`, when `SourceInfo` rode the
    /// sample. Usually absent: zenoh 1.9 and 1.10 deliver none to a
    /// subscriber — 1.10 even dropped setting it through the advanced API
    /// (eclipse-zenoh/zenoh#2563) — RFC 09 §5.1 O7's practical note.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The attachment as a rendering (#117), on the same terms as
    /// [`SampleRow::value`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<serde_json::Value>,
    /// Base64 of the exact attachment bytes; wins over the rendering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment_b64: Option<String>,
    /// True attachment size, whatever the rendering above shows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment_bytes: Option<usize>,
    /// The RFC 08 §7 validation verdict, present only when the pipeline
    /// was asked — and then always, so "valid" and "not checked" cannot be
    /// confused by a shared absence (#159).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<String>,
    /// The failed constraints, when the verdict is `invalid`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub violations: Option<Vec<String>>,
    /// Why a decode that was asked for did not happen.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decode_error: Option<String>,
    /// A version-2 preamble row (RFC 13 §4.1, #218): state fetched at
    /// trigger time and written ahead of the first observed row, at
    /// `t: 0`. Only ever written as `true`; an observed row omits it
    /// rather than saying `false`, on the same terms as every other
    /// optional field here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preamble: Option<bool>,
}

/// The wire's QoS axes as one stable token: `priority/congestion/reliability`,
/// `+express` when set — lowercase, cut/awk-friendly, never `Debug`.
///
/// **The engine's word, not a frontend's.** This is the spelling
/// [`SampleRow::qos_axes`] carries, which
/// [`parse_row`](crate::tape::ingest::parse_row) reads back for
/// `pub --from ndjson` and `.zrec` replay (RFC 09 §5.2) — so it is a
/// round-trip contract, not a rendering choice. Both frontends used to spell
/// it independently, fifteen literals each, agreeing by a comment that said
/// they agreed (#353). The precedent is `LatencyReport::caveat`, worded here
/// in #213 for exactly this reason: two frontends must not be able to
/// describe one fact differently.
pub fn qos_axes_token(
    priority: zenoh::qos::Priority,
    congestion_control: zenoh::qos::CongestionControl,
    reliability: zenoh::qos::Reliability,
    express: bool,
) -> String {
    use zenoh::qos::{CongestionControl as Cc, Priority as P, Reliability as R};
    let p = match priority {
        P::RealTime => "real_time",
        P::InteractiveHigh => "interactive_high",
        P::InteractiveLow => "interactive_low",
        P::DataHigh => "data_high",
        P::Data => "data",
        P::DataLow => "data_low",
        P::Background => "background",
    };
    let c = match congestion_control {
        Cc::Drop => "drop",
        Cc::Block => "block",
        // `CongestionControl` is `#[non_exhaustive]` upstream: a variant this
        // build has never heard of renders as `other` rather than as one of
        // the two it knows.
        _ => "other",
    };
    let r = match reliability {
        R::BestEffort => "best_effort",
        R::Reliable => "reliable",
    };
    format!("{p}/{c}/{r}{}", if express { "+express" } else { "" })
}

#[cfg(test)]
mod qos_axes_tests {
    use super::*;
    use zenoh::qos::{CongestionControl as Cc, Priority as P, Reliability as R};

    /// The token is a round-trip contract, not a rendering: `.zrec` replay
    /// and `pub --from ndjson` read it back (RFC 09 §5.2). Pinned here rather
    /// than in either frontend, because it is neither frontend's (#353).
    #[test]
    fn the_axes_token_is_stable() {
        assert_eq!(
            qos_axes_token(P::Data, Cc::Drop, R::BestEffort, false),
            "data/drop/best_effort"
        );
        assert_eq!(
            qos_axes_token(P::RealTime, Cc::Block, R::Reliable, true),
            "real_time/block/reliable+express"
        );
        assert_eq!(
            qos_axes_token(P::InteractiveHigh, Cc::Block, R::Reliable, false),
            "interactive_high/block/reliable"
        );
        assert_eq!(
            qos_axes_token(P::Background, Cc::Drop, R::BestEffort, true),
            "background/drop/best_effort+express"
        );
    }

    /// Every declared profile renders a token, and the five are distinct —
    /// which is what makes declared-vs-observed a comparison at all (#120).
    #[test]
    fn every_qos_profile_has_a_distinct_axes_token() {
        let tokens: Vec<String> = zenkey::QosProfile::ALL
            .iter()
            .map(|p| {
                qos_axes_token(
                    p.priority(),
                    p.congestion_control(),
                    p.reliability(),
                    p.express(),
                )
            })
            .collect();
        let mut unique = tokens.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), tokens.len(), "{tokens:?}");
    }
}
