//! One fetched value, decoded and rendered (#345).
//!
//! The counterpart of [`crate::services::value`], which produces this: the
//! decode is a `Task` because `decode_sample` may touch the bus, and the
//! *rendering* rides along in the same task for the same reason the decode
//! does — it is unbounded work, and unbounded work does not belong on a
//! render path.

/// A decoded sample and the document the Detail pane draws from it.
///
/// The document is rendered **once**, in the decode task that produced the
/// sample, and never again (#345). The pane used to re-run
/// `serde_json::to_string_pretty` over the whole decoded value on every
/// redraw and throw the result away — unbounded work at the frame rate, for a
/// string identical to the one the last frame built. `Rendering::Structural`'s
/// side was cheaper only by degree: a full `String` clone per frame.
///
/// What the pane still decides is how much of it to *draw*. That bound lives
/// with the hex pane's, in `view::detail`, because it is a question about a
/// reading surface rather than about a value.
#[derive(Debug)]
pub struct DecodedValue {
    /// The decode itself: rendering, verdict, and the decode error behind an
    /// `Undecodable` (#164).
    pub sample: zenkey_fleet::model::decode::DecodedSample,
    /// The rendered document: pretty-printed for a schema decode, the
    /// structural rendering otherwise. Empty means the payload rendered to
    /// nothing, which the pane states as a byte count.
    pub document: String,
}

impl DecodedValue {
    pub fn new(sample: zenkey_fleet::model::decode::DecodedSample) -> DecodedValue {
        use zenkey_fleet::model::decode::Rendering;
        let document = match &sample.rendering {
            Rendering::Typed(d) => serde_json::to_string_pretty(&d.value).unwrap_or_default(),
            Rendering::Structural(s) => s.clone(),
        };
        DecodedValue { sample, document }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey_fleet::model::decode::{DecodedSample, Rendering};

    fn structural(s: &str) -> DecodedValue {
        DecodedValue::new(DecodedSample {
            type_name: None,
            rendering: Rendering::Structural(s.to_string()),
            verdict: zenkey::schema::validate::Verdict::NotValidated(
                zenkey::schema::validate::NotValidated::NoRegistry,
            ),
            decode_error: None,
        })
    }

    /// The document exists before any frame does (#345) — which is the whole
    /// claim: the pane reads a string, and a redraw costs no serialization.
    #[test]
    fn the_document_is_rendered_with_the_decode_not_with_the_frame() {
        let v = structural(r#"{"value":42.0}"#);
        assert_eq!(v.document, r#"{"value":42.0}"#);
    }

    /// A payload that renders to nothing renders to nothing here too — the
    /// pane says the byte count instead, and must be able to tell.
    #[test]
    fn an_empty_rendering_stays_empty() {
        assert!(structural("").document.is_empty());
    }
}
