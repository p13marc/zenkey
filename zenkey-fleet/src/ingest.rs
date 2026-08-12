//! Reading back the row shape the explorers emit (#125): one schema, both
//! directions.
//!
//! `topic echo --format ndjson` and the zengui export write rows; this
//! module is the only parser of them, so the pipe is symmetric by
//! construction — and when `.zrec` replay lands (#39), it shares this
//! reader rather than growing a second row dialect.
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

    /// The echo row shape reads back: JSON values re-serialize compactly,
    /// string values publish their raw bytes, extras are ignored.
    #[test]
    fn an_echo_row_reads_back() {
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

    /// Attachments ride the same value rules (#117).
    #[test]
    fn attachments_ride_rows() {
        let row =
            parse_row(r#"{"key":"k","value":1,"attachment":{"who":"me"},"qos":"alert"}"#).unwrap();
        assert_eq!(row.attachment.as_deref(), Some(br#"{"who":"me"}"#.as_ref()));
        assert_eq!(row.qos.as_deref(), Some("alert"));
    }
}
