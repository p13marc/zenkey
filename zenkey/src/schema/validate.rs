//! Payload conformance verdicts (#159) — the three-state answer to "does
//! this payload conform to its declared schema?".
//!
//! Three states, never a boolean, for the same reason
//! [`Registration`](https://docs.rs/zenkey-fleet) has four: "I did not
//! check" must never render like "I checked and it passed". The
//! [`Verdict`] rides every [`DecodedPayload`](super::decode::DecodedPayload),
//! so every consumer of a decode sees the same answer.
//!
//! What "valid" means is kind-dependent, and honestly so:
//!
//! - `json-schema` — real draft 2020-12 validation (feature `validate-json`;
//!   without it the verdict is [`NotValidated::FeatureOff`]). This is the
//!   thick tier: required fields, types, enums, bounds.
//! - `protobuf` / `cdr` — a successful decode already proves structural
//!   conformance to the served descriptor / field list; that decode **is**
//!   the check, and its `Valid` is the thinner claim. There is no schema
//!   language underneath to violate while still decoding.

/// Did the payload conform to its declared schema?
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Checked and conformant (see the module doc for the per-kind depth).
    Valid,
    /// Checked and non-conformant; each violation is one human-readable
    /// sentence with its instance path.
    Invalid(Vec<String>),
    /// Not checked — and here is why. Never collapse this into either answer.
    NotValidated(NotValidated),
}

/// Why a payload was not validated. A reason is not a failure: most of these
/// are ordinary states of a live bus (O4 — "not asked" is not "no").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotValidated {
    /// A registry was consulted and no schema is served/known for the
    /// observed type. This is "asked, and the answer was silence about the
    /// type" — never the same fact as [`NoRegistry`](Self::NoRegistry)'s
    /// "nobody looked" (RFC 09 §5.1 O4; #246).
    NoSchema,
    /// No registry was loaded, so no type was ever looked up. "Not asked"
    /// must not masquerade as a fact about the type (RFC 09 §5.1 O4): a run
    /// whose registry was merely unreachable used to emit
    /// [`NoSchema`](Self::NoSchema) on every row, which reads as a claim
    /// about the *types* (#246).
    NoRegistry,
    /// The `validate-json` feature is compiled out of this binary.
    FeatureOff,
    /// The schema kind has no validator beyond its own decode.
    KindUnsupported,
    /// The bytes did not decode, so conformance was never reachable.
    Undecodable,
    /// The schema document itself did not compile as a schema.
    BadSchema,
}

impl std::fmt::Display for NotValidated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            NotValidated::NoSchema => "no schema served for this type",
            NotValidated::NoRegistry => "no registry loaded, so no type was looked up",
            NotValidated::FeatureOff => "validation compiled out (validate-json)",
            NotValidated::KindUnsupported => "schema kind has no validator beyond decode",
            NotValidated::Undecodable => "bytes did not decode",
            NotValidated::BadSchema => "served schema does not compile",
        })
    }
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Verdict::Valid => f.write_str("valid"),
            Verdict::Invalid(errors) => write!(f, "invalid ({} violation(s))", errors.len()),
            Verdict::NotValidated(reason) => write!(f, "not validated — {reason}"),
        }
    }
}

/// Validate a decoded JSON value against a `json-schema` document.
///
/// Compiled per call site's cache — callers hold the compiled validator via
/// [`super::compiled::CompiledCache`], keyed by schema hash, exactly like the
/// protobuf descriptor pools (#100): compiling per sample would put a schema
/// compile on every echo line.
#[cfg(feature = "validate-json")]
pub fn validate_json(validator: &jsonschema::Validator, value: &serde_json::Value) -> Verdict {
    let errors: Vec<String> = validator
        .iter_errors(value)
        .map(|e| {
            let path = e.instance_path().to_string();
            if path.is_empty() {
                e.to_string()
            } else {
                format!("{path}: {e}")
            }
        })
        .collect();
    if errors.is_empty() {
        Verdict::Valid
    } else {
        Verdict::Invalid(errors)
    }
}

#[cfg(test)]
mod vocabulary_tests {
    use super::*;

    /// The reason strings are wire vocabulary: every ndjson `verdict` field
    /// carries `not-validated: <reason>` verbatim, so a consumer greps them.
    /// In particular the two silences must never share a spelling —
    /// "no registry loaded" is "not asked", "no schema served" is "asked,
    /// and the type has none" (RFC 09 §5.1 O4; #246).
    #[test]
    fn the_two_silences_have_distinct_wire_spellings() {
        assert_eq!(
            NotValidated::NoSchema.to_string(),
            "no schema served for this type"
        );
        assert_eq!(
            NotValidated::NoRegistry.to_string(),
            "no registry loaded, so no type was looked up"
        );
    }
}

#[cfg(all(test, feature = "validate-json"))]
mod tests {
    use super::*;
    use serde_json::json;

    fn validator() -> jsonschema::Validator {
        jsonschema::validator_for(&json!({
            "type": "object",
            "required": ["x"],
            "properties": {
                "x": { "type": "integer", "minimum": 0 },
                "name": { "type": "string" },
            },
        }))
        .expect("fixture schema compiles")
    }

    #[test]
    fn conformant_and_nonconformant_values_get_opposite_verdicts() {
        let v = validator();
        assert_eq!(validate_json(&v, &json!({"x": 3})), Verdict::Valid);
        match validate_json(&v, &json!({"x": -2, "name": 7})) {
            Verdict::Invalid(errors) => {
                assert_eq!(errors.len(), 2, "{errors:?}");
                assert!(errors.iter().any(|e| e.contains("/x")), "{errors:?}");
                assert!(errors.iter().any(|e| e.contains("/name")), "{errors:?}");
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
        // A missing required field is a violation, not a shrug.
        assert!(matches!(validate_json(&v, &json!({})), Verdict::Invalid(_)));
    }
}
