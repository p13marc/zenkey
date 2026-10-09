//! The structural ladder: what bytes honestly say when no schema resolves
//! (RFC 08 §7 for v1; spec §7.2's "then sniffing" for zk2).
//!
//! JSON if it parses, then CBOR that accounts for every byte, then UTF-8
//! text, else a byte count. Schema-free and sync, so it runs on render paths
//! and observation loops alike, and it is the same ladder for both
//! generations: v1's decode seam (`model::decode`) and zk2's rendering
//! ([`crate::model::render`]) fall back to it, and neither owns it. It moved
//! here from `model/decode.rs` (#612, FJ3), which re-exports it, so the v1
//! paths keep resolving until FJ9 retires that module.
//!
//! Nothing here knows a key, a registry or a contract: a caller that has one
//! decodes through it first, and reaches for this only when it has not.

/// How many bytes an *observation* path will structurally decode.
///
/// `structural_value` parses the whole payload into a `serde_json::Value`, and
/// the observation paths call it **per sample** on a drain loop — field
/// intelligence has to, because a field that stopped moving is only visible
/// sample by sample. Unbounded, a multi-megabyte payload spends that parse on
/// every one of them, on the loop whose whole job is to keep up (#337's
/// lesson, applied to CPU rather than to I/O).
///
/// The number and the doctrine are `zengui`'s, from #345 — *"past it the size
/// is reported and the decode is skipped, which is stated, never silently
/// empty"* — moved here because both frontends and the engine's own judges
/// need it, and three copies of one limit would be three answers to one
/// question (the #353 lesson).
pub const OBSERVE_LIMIT: usize = 64 * 1024;

/// The structural sniff as a **value** rather than as text — the same ladder
/// [`structural`] renders, stopped one step earlier.
///
/// `Some` means the bytes carry a self-describing document (JSON, or CBOR that
/// accounts for every byte and is not the text-vs-scalar ambiguity below).
/// `None` means they do not: plain text, or opaque bytes. That distinction is
/// what lets a caller diff two payloads field-by-field when it can, and say so
/// honestly — a byte comparison — when it cannot.
///
/// Deliberately sync and schema-free: this runs on render paths, where no
/// fetch may sit, and where a zk2 rendering ([`crate::model::render`]) falls
/// back to it when no contract is in hand.
pub fn structural_value(bytes: &[u8]) -> Option<serde_json::Value> {
    let looks_json = bytes.first().is_some_and(|b| {
        matches!(
            b,
            b'{' | b'[' | b'"' | b'-' | b'0'..=b'9' | b't' | b'f' | b'n'
        )
    });
    if looks_json && let Ok(v) = serde_json::from_slice::<serde_json::Value>(bytes) {
        return Some(v);
    }
    let is_text = std::str::from_utf8(bytes).is_ok_and(|t| !t.is_empty());
    if let Some(v) = cbor_whole(bytes)
        // A bare CBOR scalar over bytes that are *also* valid text is the
        // ambiguous case, and plain text is the likelier reading on a bus that
        // carries anything. Structured CBOR (a map, an array) is unambiguous
        // and still wins.
        && !(is_text && is_scalar(&v))
        // A CBOR map keyed by anything but strings has no JSON form; that is a
        // failure of the *rendering*, not of the payload, so it degrades to
        // text like any other unreadable shape rather than being invented.
        && let Ok(value) = serde_json::to_value(&v)
    {
        return Some(value);
    }
    None
}

/// Structural fallback rendering — what the wire honestly says when no
/// schema resolves.
pub fn structural(bytes: &[u8]) -> String {
    if let Some(v) = structural_value(bytes) {
        return serde_json::to_string(&v).unwrap_or_default();
    }
    match std::str::from_utf8(bytes).ok().filter(|t| !t.is_empty()) {
        Some(text) => text.to_string(),
        None => format!("<{} bytes>", bytes.len()),
    }
}

/// Decode CBOR only if it accounts for **every** byte.
///
/// `ciborium::from_reader` decodes one value from the front and ignores the
/// rest, which makes it a false-positive machine on plain text: `j` is `0x6A`,
/// "text string of length 10", so `just a plain string` decodes as the CBOR
/// text `"ust a plai"` with eight bytes left over — and an explorer that shows
/// that has silently corrupted the payload it was asked to display. Any
/// lowercase-initial ASCII text is a candidate. Requiring total consumption is
/// what makes the sniff honest (RFC 08 §7 — sniffing is the last resort, so it
/// must at least be self-consistent).
fn cbor_whole(bytes: &[u8]) -> Option<ciborium::Value> {
    let mut cursor = std::io::Cursor::new(bytes);
    let value = ciborium::from_reader::<ciborium::Value, _>(&mut cursor).ok()?;
    (cursor.position() as usize == bytes.len()).then_some(value)
}

/// A single scalar, as opposed to a map or array.
fn is_scalar(v: &ciborium::Value) -> bool {
    !matches!(v, ciborium::Value::Map(_) | ciborium::Value::Array(_))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structural_rendering_is_honest() {
        assert_eq!(structural(b"{\"a\":1}"), "{\"a\":1}");
        // CBOR map {1: 2} renders as structure.
        let mut cbor = Vec::new();
        ciborium::into_writer(&serde_json::json!({"x": 1}), &mut cbor).unwrap();
        assert!(structural(&cbor).contains("\"x\""));
        assert_eq!(structural(&[0xff, 0xfe, 0x00]), "<3 bytes>");
    }

    /// The value form answers the question a diff actually asks: is there a
    /// document here to compare field by field, or only bytes?
    #[test]
    fn structural_value_yields_documents_and_nothing_else() {
        assert_eq!(
            structural_value(br#"{"value":42.0}"#),
            Some(serde_json::json!({"value": 42.0}))
        );
        let mut cbor = Vec::new();
        ciborium::into_writer(&serde_json::json!({"x": 1}), &mut cbor).unwrap();
        assert_eq!(structural_value(&cbor), Some(serde_json::json!({"x": 1})));
        // Plain text and opaque bytes are not documents — the caller falls
        // back to a byte comparison rather than being handed a fake one.
        assert_eq!(structural_value(b"just a plain string"), None);
        assert_eq!(structural_value(&[0xff, 0xfe, 0x00]), None);
        assert_eq!(structural_value(b""), None);
    }

    /// The two must not drift: `structural` is the rendering of
    /// `structural_value` wherever one exists.
    #[test]
    fn the_rendering_agrees_with_the_value() {
        for payload in [
            &br#"{"a":1}"#[..],
            &b"[1,2,3]"[..],
            &b"just a plain string"[..],
            &[0xff, 0xfe, 0x00][..],
        ] {
            if let Some(v) = structural_value(payload) {
                assert_eq!(structural(payload), serde_json::to_string(&v).unwrap());
            }
        }
    }

    /// Regression: plain text must not be eaten by the CBOR sniff.
    ///
    /// `ciborium` decodes one value from the front and ignores trailing bytes,
    /// so `just a plain string` used to render as `"ust a plai"` — `j` is
    /// `0x6A`, "text string of length 10". Every lowercase-initial ASCII
    /// payload was a candidate, which on an arbitrary bus is most of them.
    #[test]
    fn plain_text_is_not_mistaken_for_cbor() {
        assert_eq!(structural(b"just a plain string"), "just a plain string");
        assert_eq!(
            structural(b"a v2 key: not this convention"),
            "a v2 key: not this convention"
        );
        // The whole lowercase range is the danger zone (0x60..=0x7b).
        for first in b'a'..=b'z' {
            let mut payload = vec![first];
            payload.extend_from_slice(b" some trailing words here");
            let text = String::from_utf8(payload.clone()).unwrap();
            assert_eq!(structural(&payload), text, "mangled {text:?}");
        }
    }

    /// The ambiguous case: bytes that are *both* a complete CBOR text string
    /// and valid UTF-8. Plain text is the likelier reading on a bus that
    /// carries anything, and it is the lossless one.
    #[test]
    fn an_exact_cbor_text_string_still_reads_as_text() {
        // 0x6A = text(10), followed by exactly 10 bytes: fully consumed CBOR.
        let payload = b"just a plai";
        assert!(cbor_whole(payload).is_some(), "setup: this is valid CBOR");
        assert_eq!(structural(payload), "just a plai");
    }

    /// …but structured CBOR is unambiguous and must still win, even when the
    /// bytes happen to be valid UTF-8.
    #[test]
    fn structured_cbor_still_wins_over_text() {
        let mut cbor = Vec::new();
        ciborium::into_writer(&serde_json::json!({"ok": true}), &mut cbor).unwrap();
        let rendered = structural(&cbor);
        assert!(rendered.contains("\"ok\""), "{rendered}");
        assert!(rendered.starts_with('{'), "{rendered}");
    }

    /// Trailing bytes mean the buffer is not one CBOR value, whatever the
    /// front of it looks like.
    #[test]
    fn cbor_must_account_for_every_byte() {
        let mut cbor = Vec::new();
        ciborium::into_writer(&serde_json::json!({"x": 1}), &mut cbor).unwrap();
        assert!(cbor_whole(&cbor).is_some());
        cbor.push(0x00);
        assert!(cbor_whole(&cbor).is_none(), "trailing byte must reject");
    }
}
