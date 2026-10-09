//! The row shape the explorers emit and read back (#125): one schema, both
//! directions.
//!
//! [`SampleRow`] is the only writer and [`parse_row`] the only reader, which
//! is what makes the pipe symmetric. It was not, until #235: the claim lived
//! in this doc comment while three hand-built writers — `.zrec`
//! ([`mod@crate::tape::record`]), `zenctl echo --format ndjson`, and zengui's
//! echo export — each assembled the object with `serde_json::json!` and two
//! of them disagreed with this reader. `echo` wrote the zenoh *wire axes*
//! under `"qos"`, where [`parse_row`] resolves a profile *name*, so every
//! row of `echo --format ndjson | pub --from ndjson` was counted
//! malformed; zengui wrote a payload byte *count* under `"bytes"`, which has
//! meant base64 of the wire payload since RFC 09 §5.2. Both carried a doc
//! comment asserting conformance. One struct is the repair — a dialect with
//! one writer cannot drift from itself.
//!
//! A row names its key and payload, and optionally its encoding, QoS
//! axes, tombstone-ness, and attachment. Unknown fields are ignored
//! (rows carry observer-side extras like `type`/`typed`); a row that
//! cannot be published is an error *naming the reason*, and callers MUST
//! count those rather than silently skipping (zenoh-cli logs-and-drops;
//! we count).
//!
//! A row MAY carry its payload lossless as `"bytes"` (base64 of the exact
//! wire bytes — the `.zrec` dialect, RFC 09 §5.2), which wins over
//! `"value"`: a `value` is a decoded *rendering* and does not round-trip a
//! binary payload. Same for `"attachment_b64"` over `"attachment"`.
//!
//! **Re-cut for zk2** (#612, FJ8a, FJ9). A row's QoS is its wire axes,
//! `"qos_axes"` (`priority/congestion/reliability[+express]`): a zk2
//! owner's QoS is per resource (§2.4). `echo` has written the axes since
//! #120 and `.zrec` rows since version 3, so a pipe and a replay publish a
//! foreign row with exactly the QoS it was seen with. v1's profile name in
//! `"qos"`, and the `origin` and `subject` the v1 grammar parsed out of a
//! key, left with the v1 dependency (FJ9): a row that carries them reads,
//! and they are ignored like any other observer-side extra. A row whose key
//! a zk2 service owns parses like any other: the writer refuses it (P3,
//! spec §6), not the dialect.

use crate::report::SampleRow;

impl SampleRow {
    /// A row on `key`, the wire key as received. What a lens made of the
    /// key rides [`SampleRow::identity`], which the writer that resolved it
    /// fills in.
    pub fn of_key(key: &str) -> SampleRow {
        SampleRow {
            key: key.to_string(),
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
        // The axes as they rode, which a replay and a pipe publish with
        // (a zk2 owner's QoS is per resource, §2.4).
        self.qos_axes = Some(crate::report::qos_axes_token(
            view.priority,
            view.congestion_control,
            view.reliability,
            view.express,
        ));
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
    /// The row's wire QoS axes, when it carries them (#612, FJ8a); a row
    /// without them publishes with its writer's default.
    pub qos_axes: Option<crate::bus::write::WireQos>,
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
    let qos_axes = match obj.get("qos_axes") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(t)) => {
            Some(crate::bus::write::WireQos::parse(t).ok_or_else(|| {
                format!("\"qos_axes\" {t:?} is not priority/congestion/reliability[+express]")
            })?)
        }
        Some(_) => return Err("\"qos_axes\" is not a string".into()),
    };
    Ok(IngestRow {
        qos_axes,
        key,
        payload,
        encoding: obj
            .get("encoding")
            .and_then(|e| e.as_str())
            .filter(|e| !e.is_empty())
            .map(str::to_string),
        delete,
        attachment,
    })
}

/// One line of an explorer ndjson **stream**, as the input side reads it.
///
/// A stream interleaves its samples with tagged non-sample rows — `{"row":
/// "dropped",…}` where the bus outran the observer (RFC 09 §5.1 O6),
/// `{"row":"seed",…}` at the seed boundary — and a consumer that wants the
/// samples must tell those apart from a row it cannot parse: metadata is
/// *skipped and counted as skipped*, a malformed row is an error naming the
/// reason. Before this split, echo's own honesty lines poisoned the
/// `echo | pub --from ndjson` round trip the dialect exists for (#235).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamLine {
    /// A publishable sample row.
    Sample(IngestRow),
    /// A tagged non-sample row; the value is the `"row"` tag (`"dropped"`,
    /// `"seed"`, …). Stream metadata — skip it, count the skip.
    Meta(String),
}

/// Parse one line of an explorer stream: [`parse_row`], with the stream's
/// tagged meta rows told apart from its samples.
///
/// The `"row"` key is the explorers' kind tag (zenctl's `render::Row`
/// convention: every non-sample line of a heterogeneous stream carries one).
/// A sample row never carries the tag today; `"sample"` is reserved so a
/// future writer that tags its samples still round-trips.
pub fn parse_stream_line(line: &str) -> Result<StreamLine, String> {
    if let Ok(serde_json::Value::Object(obj)) = serde_json::from_str::<serde_json::Value>(line)
        && let Some(tag) = obj.get("row").and_then(serde_json::Value::as_str)
        && tag != "sample"
    {
        return Ok(StreamLine::Meta(tag.to_string()));
    }
    parse_row(line).map(StreamLine::Sample)
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
            r#"{"key":"prod/zk2/host-a/tc/tc.netif.v1/state/namespaces","type":"json:Namespaces",
                "typed":true,"encoding":"application/json","timestamp":null,"delete":false,
                "value":{"status":"ok"}}"#,
        )
        .unwrap();
        assert_eq!(row.key, "prod/zk2/host-a/tc/tc.netif.v1/state/namespaces");
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
    fn view(payload: &[u8]) -> crate::SampleView {
        crate::SampleView {
            key: "plant/line-1/temp".into(),
            payload: zenoh::bytes::ZBytes::from(payload.to_vec()),
            encoding: "application/json".into(),
            kind: zenoh::sample::SampleKind::Put,
            timestamp: None,
            stamped_by: None,
            attachment: None,
            priority: zenoh::qos::Priority::DataHigh,
            congestion_control: zenoh::qos::CongestionControl::Block,
            reliability: zenoh::qos::Reliability::Reliable,
            express: false,
            source: None,
            received: std::time::Instant::now(),
        }
    }

    /// The round trip the README advertises and RFC 09 §5.2 rests on, over
    /// a row this crate wrote rather than one a test hand-typed — and the
    /// axes the sample rode with ride the row both ways (#235, FJ8a).
    #[test]
    fn a_row_this_crate_wrote_is_a_row_this_crate_reads() {
        let v = view(br#"{"status":"ok"}"#);

        // The observer's dialect: a rendering under `value`, the wire axes
        // under their own key.
        let mut observed = SampleRow::of_key(&v.key).with_wire(&v);
        observed.value = Some(serde_json::json!({"status": "ok"}));
        observed.payload_bytes = Some(15);
        assert_eq!(
            observed.qos_axes.as_deref(),
            Some("data_high/block/reliable")
        );
        let back = parse_row(&observed.to_line()).expect("the observer's row reads back");
        assert_eq!(back.payload, br#"{"status":"ok"}"#);
        assert_eq!(back.encoding.as_deref(), Some("application/json"));
        let axes = back.qos_axes.expect("the axes read back");
        assert_eq!(
            (
                axes.priority,
                axes.congestion,
                axes.reliability,
                axes.express
            ),
            (
                zenoh::qos::Priority::DataHigh,
                zenoh::qos::CongestionControl::Block,
                zenoh::qos::Reliability::Reliable,
                false
            )
        );

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
        assert_eq!(back.qos_axes, Some(axes));
    }

    /// A version-1 or 2 row's v1 extras — a profile name under `qos`, the
    /// parsed `origin` and `subject` — read as ignored extras: the row
    /// still publishes, with its writer's default axes (#612, FJ9).
    #[test]
    fn a_v1_rows_profile_name_is_an_ignored_extra() {
        let row = parse_row(
            r#"{"key":"v1/h-1/state/p/health","origin":"h-1","subject":"health",
                "qos":"refreshed","value":{"status":"ok"}}"#,
        )
        .unwrap();
        assert_eq!(row.key, "v1/h-1/state/p/health");
        assert_eq!(row.qos_axes, None, "a profile name is no axes");
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
            "an unresolved key carries no identity, an unstamped sample carries \
             no timestamp, and an undecoded one carries no type"
        );
    }

    /// The stream reader's three-way split: a tagged non-sample row is Meta
    /// (skipped, not malformed), an untagged sample row is a Sample, and a
    /// line that is neither is still an error naming the reason.
    #[test]
    fn tagged_meta_rows_are_skipped_not_malformed() {
        assert_eq!(
            parse_stream_line(r#"{"dropped":7,"row":"dropped"}"#).unwrap(),
            StreamLine::Meta("dropped".into())
        );
        assert_eq!(
            parse_stream_line(r#"{"row":"seed","seed_complete":{"superseded":0}}"#).unwrap(),
            StreamLine::Meta("seed".into())
        );
        // A tag whose value is not a string is not the convention's tag: the
        // line falls through to the row parser and errors like any other.
        assert!(parse_stream_line(r#"{"row":7}"#).is_err());
        assert!(matches!(
            parse_stream_line(r#"{"key":"k","value":1}"#).unwrap(),
            StreamLine::Sample(r) if r.key == "k"
        ));
        let err = parse_stream_line(r#"{"key":"k"}"#).unwrap_err();
        assert!(err.contains("value"), "{err}");
    }

    /// A zk2 row's QoS is its axes (FJ8a): one the writer could not have
    /// spelled is a counted error naming the field, never a guessed profile.
    #[test]
    fn qos_axes_read_back_or_are_refused_by_name() {
        let row =
            parse_row(r#"{"key":"k","value":1,"qos_axes":"real_time/block/reliable+express"}"#)
                .unwrap();
        let axes = row.qos_axes.expect("axes");
        assert_eq!(axes.priority, zenoh::qos::Priority::RealTime);
        assert!(axes.express);
        let err = parse_row(r#"{"key":"k","value":1,"qos_axes":"fast"}"#).unwrap_err();
        assert!(err.contains("qos_axes"), "{err}");
        let err = parse_row(r#"{"key":"k","value":1,"qos_axes":3}"#).unwrap_err();
        assert!(err.contains("not a string"), "{err}");
    }

    /// Attachments ride the same value rules (#117).
    #[test]
    fn attachments_ride_rows() {
        let row = parse_row(r#"{"key":"k","value":1,"attachment":{"who":"me"}}"#).unwrap();
        assert_eq!(row.attachment.as_deref(), Some(br#"{"who":"me"}"#.as_ref()));
    }
}
