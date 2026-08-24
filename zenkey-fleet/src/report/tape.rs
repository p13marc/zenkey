//! The tape plane (RFC 09 §5.2): the `.zrec` header, the row dialect a
//! capture is made of, and what a capture or a replay reports afterwards.
//!
//! [`ZrecHeader`] is the one shape in this module that is read as well as
//! written — a `.zrec` on disk outlives the process that wrote it, so the
//! header is a contract in both directions and is the only report shape
//! deriving `Deserialize`.

use serde::{Deserialize, Serialize};

/// The first line of a `.zrec` file: what was asked, under which base, and
/// when (RFC 09 §5.1 O4 — a capture names its question). The `base` is the
/// operator's *stated* deployment base at capture time; recorded keys are
/// full wire keys and are never re-derived from it (O3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
}

/// One sample, as the explorers write it (#235).
///
/// The write side of this module's dialect: `.zrec` rows (RFC 09 §5.2),
/// `zenctl topic echo --format ndjson`, and zengui's echo export are all
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
}
