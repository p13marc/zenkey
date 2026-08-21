//! The row shape the explorers emit and read back (#125): one schema, both
//! directions.
//!
//! [`SampleRow`] is the only writer and [`parse_row`] the only reader, which
//! is what makes the pipe symmetric. It was not, until #235: the claim lived
//! in this doc comment while three hand-built writers — `.zrec`
//! ([`mod@crate::record`]), `zenctl topic echo --format ndjson`, and zengui's
//! echo export — each assembled the object with `serde_json::json!` and two
//! of them disagreed with this reader. `echo` wrote the zenoh *wire axes*
//! under `"qos"`, where [`parse_row`] resolves a profile *name*, so every
//! row of `topic echo --format ndjson | topic pub --from ndjson` was counted
//! malformed; zengui wrote a payload byte *count* under `"bytes"`, which has
//! meant base64 of the wire payload since RFC 09 §5.2. Both carried a doc
//! comment asserting conformance. One struct is the repair — a dialect with
//! one writer cannot drift from itself.
//!
//! A row names its key and payload, and optionally its encoding, QoS
//! profile, tombstone-ness, and attachment. Unknown fields are ignored
//! (rows carry observer-side extras like `type`/`typed`); a row that
//! cannot be published is an error *naming the reason*, and callers MUST
//! count those rather than silently skipping (zenoh-cli logs-and-drops;
//! we count).
//!
//! A row MAY carry its payload lossless as `"bytes"` (base64 of the exact
//! wire bytes — the `.zrec` dialect, RFC 09 §5.2), which wins over
//! `"value"`: a `value` is a decoded *rendering* and does not round-trip a
//! binary payload. Same for `"attachment_b64"` over `"attachment"`.

use serde::Serialize;

/// One sample, as the explorers write it (#235).
///
/// The write side of this module's dialect: `.zrec` rows (RFC 09 §5.2),
/// `zenctl topic echo --format ndjson`, and zengui's echo export are all
/// this struct, so [`parse_row`] reads back what any of them wrote. Every
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
    /// This is the field [`parse_row`] resolves through
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
    /// which is why [`parse_row`] prefers it over [`SampleRow::value`].
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
    /// sample. Usually absent: zenoh 1.9 delivers none to a subscriber
    /// (RFC 09 §5.1 O7's practical note).
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

impl SampleRow {
    /// The identity fields every writer shares: the wire key, and what the
    /// convention could make of it under this base.
    ///
    /// A key that does not parse still yields a row — O1: a key that does
    /// not parse is a fact, not an error — it simply carries no `origin`
    /// or `subject`.
    pub fn of_key(key: &str, base: &str) -> SampleRow {
        let parsed = zenkey::grammar::parse_full(base, key);
        SampleRow {
            key: key.to_string(),
            origin: parsed.as_ref().map(|p| p.origin.chunk().to_string()),
            subject: parsed.as_ref().map(|p| p.subject.join("/")),
            ..SampleRow::default()
        }
    }

    /// The wire facts a [`crate::SampleView`] carries, filled in.
    ///
    /// Payload and attachment are left to the caller: an observer renders
    /// them (`value`), a capture stores them (`bytes`), and which one a
    /// writer owes is the difference between the two dialect halves.
    pub fn with_wire(mut self, view: &crate::SampleView) -> SampleRow {
        self.delete = view.kind == zenoh::sample::SampleKind::Delete;
        if !view.encoding.is_empty() {
            self.encoding = Some(view.encoding.clone());
        }
        self.timestamp = view.timestamp.map(|t| t.to_string());
        // The profile name only where the axes actually match one, exactly
        // as `.zrec` has always done it: a name the reader cannot resolve
        // is worse than no name (#235).
        self.qos = zenkey::qos::QosProfile::ALL
            .into_iter()
            .find(|p| view.qos_matches(*p))
            .map(|p| p.name().to_string());
        self
    }

    /// The lossless payload, base64 — what a capture owes and a live
    /// rendering does not (RFC 09 §5.2).
    pub fn with_payload_bytes(mut self, payload: &[u8]) -> SampleRow {
        self.bytes = Some(b64(payload));
        self
    }

    /// One line of ndjson, newline excluded.
    pub fn to_line(&self) -> String {
        serde_json::to_string(self).expect("a sample row serializes")
    }
}

/// One publishable row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestRow {
    /// Full wire key — explorers are un-namespaced, so rows carry what was
    /// (or will be) on the wire.
    pub key: String,
    /// The payload bytes: a JSON `value` re-serializes compactly, a string
    /// `value` publishes its raw bytes (the same asymmetry `structural`
    /// introduced on the way out, undone).
    pub payload: Vec<u8>,
    /// The row's declared encoding, when it carries one.
    pub encoding: Option<String>,
    /// The row's QoS profile name (RFC 04 §3), when it carries one.
    pub qos: Option<String>,
    /// A tombstone row (RFC 04 §1.2): publish a delete, not the payload.
    pub delete: bool,
    /// The row's attachment, when it carries one (#117), same value rules
    /// as the payload.
    pub attachment: Option<Vec<u8>>,
}

fn value_bytes(v: &serde_json::Value) -> Vec<u8> {
    match v {
        serde_json::Value::String(s) => s.clone().into_bytes(),
        other => serde_json::to_vec(other).unwrap_or_default(),
    }
}

/// Base64 for the wire-payload fields, beside the decoder that reads them
/// back. `.zrec` writes through here too, so one alphabet is spelled once.
pub(crate) fn b64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Decode a base64 field, naming the field in the error.
fn b64_bytes(
    obj: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<Option<Vec<u8>>, String> {
    use base64::Engine as _;
    match obj.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) => base64::engine::general_purpose::STANDARD
            .decode(s)
            .map(Some)
            .map_err(|e| format!("\"{field}\" is not base64: {e}")),
        Some(_) => Err(format!("\"{field}\" is not a base64 string")),
    }
}

/// Parse one ndjson line into a publishable row.
pub fn parse_row(line: &str) -> Result<IngestRow, String> {
    let v: serde_json::Value =
        serde_json::from_str(line).map_err(|e| format!("not a JSON object: {e}"))?;
    let obj = v.as_object().ok_or("not a JSON object")?;
    let key = obj
        .get("key")
        .and_then(|k| k.as_str())
        .ok_or("no \"key\" field")?
        .to_string();
    if key.is_empty() {
        return Err("empty \"key\"".into());
    }
    let delete = obj.get("delete").and_then(|d| d.as_bool()).unwrap_or(false);
    // The lossless bytes win over the rendering (`.zrec` rows carry both
    // clocks and neither payload shape lies — RFC 09 §5.2).
    let payload = match (b64_bytes(obj, "bytes")?, obj.get("value")) {
        (Some(raw), _) => raw,
        (None, Some(serde_json::Value::Null) | None) if delete => Vec::new(),
        (None, Some(serde_json::Value::Null) | None) => {
            return Err("no \"value\" or \"bytes\" field (and not a delete row)".into());
        }
        (None, Some(v)) => value_bytes(v),
    };
    let attachment = match b64_bytes(obj, "attachment_b64")? {
        Some(raw) => Some(raw),
        None => obj.get("attachment").map(value_bytes),
    };
    Ok(IngestRow {
        key,
        payload,
        encoding: obj
            .get("encoding")
            .and_then(|e| e.as_str())
            .filter(|e| !e.is_empty())
            .map(str::to_string),
        qos: obj.get("qos").and_then(|q| q.as_str()).map(str::to_string),
        delete,
        attachment,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parser's leniency, over a hand-written row: JSON values
    /// re-serialize compactly, string values publish their raw bytes, and
    /// observer-side extras are ignored.
    ///
    /// This was called `an_echo_row_reads_back` and its row was typed by
    /// hand, so it proved nothing about what `echo` emitted — which is how
    /// #235 shipped. `a_row_this_crate_wrote_is_a_row_this_crate_reads`
    /// below makes the claim this name used to.
    #[test]
    fn a_hand_written_row_reads_back_with_its_extras_ignored() {
        let row = parse_row(
            r#"{"key":"v1/h-1/state/p/health","origin":"h-1","type":"Health","typed":true,
                "encoding":"application/json","timestamp":null,"delete":false,
                "value":{"status":"ok"}}"#,
        )
        .unwrap();
        assert_eq!(row.key, "v1/h-1/state/p/health");
        assert_eq!(row.payload, br#"{"status":"ok"}"#);
        assert_eq!(row.encoding.as_deref(), Some("application/json"));
        assert!(!row.delete);

        let text = parse_row(r#"{"key":"k","value":"just words"}"#).unwrap();
        assert_eq!(text.payload, b"just words");
    }

    /// A tombstone row needs no value; a non-delete row without one is a
    /// counted error, never a silent skip.
    #[test]
    fn tombstones_and_malformed_rows_are_told_apart() {
        let del = parse_row(r#"{"key":"k","delete":true,"value":null}"#).unwrap();
        assert!(del.delete);
        assert!(del.payload.is_empty());

        let err = parse_row(r#"{"key":"k"}"#).unwrap_err();
        assert!(err.contains("value"), "{err}");
        let err = parse_row(r#"{"value":1}"#).unwrap_err();
        assert!(err.contains("key"), "{err}");
        let err = parse_row("not json").unwrap_err();
        assert!(err.contains("JSON"), "{err}");
    }

    /// The `.zrec` dialect: `bytes` is the exact wire payload and wins over
    /// the `value` rendering; `attachment_b64` likewise. Bad base64 is a
    /// counted error, not a skip (RFC 09 §5.2).
    #[test]
    fn lossless_bytes_win_over_the_rendering() {
        let row = parse_row(
            r#"{"key":"k","t":125000,"bytes":"AAEC/w==","value":"lossy render",
                "attachment_b64":"3q0=","encoding":"application/octet-stream"}"#,
        )
        .unwrap();
        assert_eq!(row.payload, vec![0x00, 0x01, 0x02, 0xff]);
        assert_eq!(row.attachment.as_deref(), Some([0xde, 0xad].as_ref()));

        // A bytes-only row needs no value at all.
        let row = parse_row(r#"{"key":"k","bytes":"aGk="}"#).unwrap();
        assert_eq!(row.payload, b"hi");

        let err = parse_row(r#"{"key":"k","bytes":"not base64!"}"#).unwrap_err();
        assert!(err.contains("base64"), "{err}");
        let err = parse_row(r#"{"key":"k","bytes":7}"#).unwrap_err();
        assert!(err.contains("base64"), "{err}");
    }

    /// A `SampleView` built from the wire, so `with_wire`'s rules are
    /// exercised rather than restated.
    fn view(payload: &[u8], profile: Option<zenkey::qos::QosProfile>) -> crate::SampleView {
        use zenkey::qos::QosProfile;
        // Axes that match no profile, unless one was asked for: the case
        // `.zrec` always handled and `echo` did not (#235).
        let p = profile.unwrap_or(QosProfile::Sampled);
        crate::SampleView {
            key: "v1/h-3fa9c2d41b7e/state/sysinfo/health".into(),
            payload: zenoh::bytes::ZBytes::from(payload.to_vec()),
            encoding: "application/json".into(),
            kind: zenoh::sample::SampleKind::Put,
            timestamp: None,
            stamped_by: None,
            attachment: None,
            priority: if profile.is_some() {
                p.priority()
            } else {
                zenoh::qos::Priority::Background
            },
            congestion_control: p.congestion_control(),
            reliability: p.reliability(),
            express: p.express(),
            source: None,
            received: std::time::Instant::now(),
        }
    }

    /// The bug #235 was: `qos` is the field the reader resolves through
    /// `QosProfile::from_name`, so anything a writer puts there must be a
    /// name — never the wire axes, which match no profile by design.
    #[test]
    fn the_qos_field_only_ever_carries_a_name_the_reader_can_resolve() {
        let named = SampleRow::of_key("k", "")
            .with_wire(&view(b"{}", Some(zenkey::qos::QosProfile::Alert)));
        assert_eq!(named.qos.as_deref(), Some("alert"));
        assert!(
            zenkey::qos::QosProfile::from_name(named.qos.as_deref().unwrap()).is_some(),
            "whatever lands in `qos` must resolve, or every row of the pipe is malformed"
        );

        // Axes matching no declared profile are not approximated: the field
        // is absent, which the reader treats as "no profile stated" and the
        // publisher's own ladder then resolves.
        let unnamed = SampleRow::of_key("k", "").with_wire(&view(b"{}", None));
        assert_eq!(
            unnamed.qos, None,
            "axes matching no profile are omitted, never spelled into `qos`"
        );
    }

    /// The round trip the README advertises and RFC 09 §5.2 rests on, over
    /// a row this crate wrote rather than one a test hand-typed — which is
    /// exactly what `an_echo_row_reads_back` above could not check.
    #[test]
    fn a_row_this_crate_wrote_is_a_row_this_crate_reads() {
        let v = view(
            br#"{"status":"ok"}"#,
            Some(zenkey::qos::QosProfile::Refreshed),
        );

        // The observer's dialect: a rendering under `value`, the wire axes
        // under their own key, the profile name under `qos`.
        let mut observed = SampleRow::of_key(&v.key, "").with_wire(&v);
        observed.value = Some(serde_json::json!({"status": "ok"}));
        observed.qos_axes = Some("data/drop/reliable".into());
        observed.payload_bytes = Some(15);
        let back = parse_row(&observed.to_line()).expect("the observer's row reads back");
        assert_eq!(back.payload, br#"{"status":"ok"}"#);
        assert_eq!(back.qos.as_deref(), Some("refreshed"));
        assert_eq!(back.encoding.as_deref(), Some("application/json"));

        // The capture dialect: the lossless bytes, which win over any
        // rendering (RFC 09 §5.2).
        let captured = SampleRow {
            key: v.key.clone(),
            t: Some(1_250),
            ..SampleRow::default()
        }
        .with_wire(&v)
        .with_payload_bytes(&v.payload.to_bytes());
        let back = parse_row(&captured.to_line()).expect("the capture's row reads back");
        assert_eq!(back.payload, br#"{"status":"ok"}"#);
        assert_eq!(back.qos.as_deref(), Some("refreshed"));
    }

    /// A writer that does not hold a fact omits it. Asserted as a whole
    /// document, because `json["absent"]` is `Null` and a field-by-field
    /// check cannot tell absent from null-when-unknown (RFC 09 §5.1 O4) —
    /// the same reason `report_contract.rs` compares whole documents.
    #[test]
    fn an_unheld_fact_is_absent_from_the_row_rather_than_null() {
        let row = SampleRow {
            key: "demo/foreign".into(),
            delete: true,
            ..SampleRow::default()
        };
        assert_eq!(
            serde_json::to_value(&row).unwrap(),
            serde_json::json!({"key": "demo/foreign", "delete": true}),
            "a key that did not parse carries no origin/subject, an unstamped \
             sample carries no timestamp, and an undecoded one carries no type"
        );
    }

    /// Attachments ride the same value rules (#117).
    #[test]
    fn attachments_ride_rows() {
        let row =
            parse_row(r#"{"key":"k","value":1,"attachment":{"who":"me"},"qos":"alert"}"#).unwrap();
        assert_eq!(row.attachment.as_deref(), Some(br#"{"who":"me"}"#.as_ref()));
        assert_eq!(row.qos.as_deref(), Some("alert"));
    }
}
