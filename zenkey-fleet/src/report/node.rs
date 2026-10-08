//! The node plane (RFC 04 §5): what an origin's `introspect` reply said
//! when it could not be read. The roster and node listings it once sat
//! beside left with v1's `node` noun at FJ4 (#612).

use serde::Serialize;

/// An `introspect` reply that arrived and could not be read as a registry
/// slice (RFC 08 §6, v1.44; #491): what it declared, and the first line of
/// why it did not read.
///
/// Three causes land here, and the shape does not tell them apart because
/// the reader does not need it to — the error line does: a declared
/// `Encoding` that is neither spelling (`application/json`), a document
/// malformed in the spelling it declared (a TOML file declared
/// `application/kdl` is read *as KDL*, never sniffed back), or one that
/// parses but is not a slice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnreadableSlice {
    /// The reply's `Encoding`, verbatim — `None` when it declared none,
    /// which is itself the fact (the undeclared row of RFC 08 §6's table
    /// reads as TOML), not an unasked question.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    /// The parse error's first line. A TOML error's later lines are a
    /// caret diagram of the source, which a table row cannot carry and a
    /// script does not want.
    pub error: String,
}

impl UnreadableSlice {
    /// From the reply's declared encoding and whatever refused it — the
    /// first line of its `Display`, trimmed.
    pub fn new(encoding: Option<String>, error: &dyn std::fmt::Display) -> UnreadableSlice {
        let error = error.to_string();
        UnreadableSlice {
            encoding,
            error: error.lines().next().unwrap_or_default().trim().to_string(),
        }
    }

    /// The one sentence both explorers draw (#491): "introspect answered,
    /// slice unreadable (`application/kdl`: …)" — the house counterpart of
    /// "no introspect reply — capabilities unknown, not absent", and like
    /// it a statement about the fetch, never about the producer's
    /// capabilities.
    pub fn sentence(&self) -> String {
        let declared = match &self.encoding {
            Some(e) => format!("`{e}`"),
            None => "no encoding declared".to_string(),
        };
        format!(
            "introspect answered, slice unreadable ({declared}: {})",
            self.error
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The third pole, pinned (#491): present only when the reply answered
    /// unreadably, carrying the declaration verbatim and one line of error —
    /// and an undeclared reply's absent `encoding` is absence, not `null`.
    #[test]
    fn an_unreadable_slice_is_its_declaration_and_one_line() {
        let toml_err =
            "malformed registry slice: TOML parse error at line 1, column 1\n  |\n1 | {\n  | ^";
        let u = UnreadableSlice::new(Some("application/kdl".into()), &toml_err);
        assert_eq!(
            serde_json::to_value(&u).unwrap(),
            json!({
                "encoding": "application/kdl",
                "error": "malformed registry slice: TOML parse error at line 1, column 1",
            })
        );
        assert_eq!(
            u.sentence(),
            "introspect answered, slice unreadable (`application/kdl`: malformed registry \
             slice: TOML parse error at line 1, column 1)"
        );
        let undeclared = UnreadableSlice::new(None, &"malformed registry slice: no [registry]");
        assert_eq!(
            serde_json::to_value(&undeclared).unwrap(),
            json!({"error": "malformed registry slice: no [registry]"})
        );
        assert!(undeclared.sentence().contains("(no encoding declared: "));
    }
}
