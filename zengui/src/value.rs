//! One fetched value, checked through the lens and rendered (#345; zk2's
//! lens since #614, FL6a).
//!
//! The counterpart of [`crate::services::value`], which produces this: the
//! check is a `Task` because a decode is unbounded work and resolving a key
//! may nudge the lens feed into a re-read, and the *rendering* rides along
//! in the same task for the same reason — unbounded work does not belong on
//! a render path.

use zenkey_fleet::report::Rendered;

/// A checked sample and the document the Detail pane draws from it.
///
/// The document is rendered **once**, in the task that produced the check,
/// and never again (#345). The pane used to re-run
/// `serde_json::to_string_pretty` over the whole decoded value on every
/// redraw and throw the result away — unbounded work at the frame rate, for a
/// string identical to the one the last frame built.
///
/// What the pane still decides is how much of it to *draw*. That bound lives
/// with the hex pane's, in `view::detail`, because it is a question about a
/// reading surface rather than about a value.
#[derive(Debug)]
pub struct DecodedValue {
    /// The check itself (`zenkey_fleet::Lens::check`): the key's identity,
    /// what the payload rendered as, and its conformance — one of four
    /// poles, never folded.
    pub checked: zenkey_fleet::Checked,
    /// The rendered document: pretty-printed for a contract decode, the
    /// structural rendering where the ladder stopped, and the structural
    /// sniff of bytes that did not decode as their declared type. Empty for
    /// a raw type — the contract says those bytes are not this tool's to
    /// read — and for a payload that rendered to nothing; the pane states
    /// either as a byte count.
    pub document: String,
}

impl DecodedValue {
    /// Render `checked`'s document once. `bytes` is the payload it checked:
    /// what an undecodable rendering is sniffed from, so the pane still
    /// shows what arrived beside saying it is not the declared type.
    pub fn new(checked: zenkey_fleet::Checked, bytes: &[u8]) -> DecodedValue {
        let document = match &checked.rendering.rendered {
            Rendered::Value { value, .. } => {
                serde_json::to_string_pretty(value).unwrap_or_default()
            }
            Rendered::Structural { text, .. } => text.clone(),
            Rendered::Undecodable { .. } => zenkey_fleet::structural(bytes),
            Rendered::Opaque { .. } => String::new(),
        };
        DecodedValue { checked, document }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A check through a lens over nothing — no namespace stripped, no
    /// presence read, no contract: every key stops at a rung, and the
    /// payload renders structurally.
    fn structural(bytes: &[u8]) -> DecodedValue {
        let set = zenkey_fleet::ContractSet::new();
        let lens = zenkey_fleet::Lens::new("", None, &set);
        let checked = lens.check("demo/example/foo", zenkey_fleet::Member::Type, None, bytes);
        DecodedValue::new(checked, bytes)
    }

    /// The document exists before any frame does (#345) — which is the whole
    /// claim: the pane reads a string, and a redraw costs no serialization.
    #[test]
    fn the_document_is_rendered_with_the_check_not_with_the_frame() {
        let v = structural(br#"{"value":42.0}"#);
        assert_eq!(v.document, r#"{"value":42.0}"#);
        assert!(
            !v.checked.conformance.is_checked(),
            "a key no contract reaches is not checked, and says so"
        );
    }

    /// A payload that renders to nothing renders to nothing here too — the
    /// pane says the byte count instead, and must be able to tell.
    #[test]
    fn an_empty_rendering_says_its_byte_count() {
        assert_eq!(structural(b"").document, "<0 bytes>");
    }
}
