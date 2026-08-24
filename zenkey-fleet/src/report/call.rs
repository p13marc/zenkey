//! The call plane (RFC 05 §3): one invocation, every origin that answered,
//! and the exit code that follows from the set.
//!
//! [`CallReport::exit_code`] is the interesting part: silence has an exit
//! code of its own, distinct from "answered, and refused" — a caller that
//! collapsed the two would be making silence a verdict (RFC 05 §2.1).

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CallError {
    pub name: String,
    pub message: String,
}

/// How one origin answered: a value reply or an RFC 05 §3 error envelope.
///
/// An enum, not `{ok: bool, error: Option<CallError>}` — the flat shape
/// could spell `ok: false` with no error attached, and the renderers
/// silently dropped exactly that row. A state that cannot be rendered
/// honestly must not be representable.
#[derive(Debug, Clone)]
pub enum CallOutcome {
    /// A value reply: the JSON document when it parses, the raw text
    /// otherwise (TOML introspect replies…).
    Ok {
        value: Option<serde_json::Value>,
        text: Option<String>,
    },
    /// An RFC 05 §3 error envelope.
    Err(CallError),
}

impl CallOutcome {
    /// Whether this is a value reply (the wire's `ok` field).
    pub fn is_ok(&self) -> bool {
        matches!(self, CallOutcome::Ok { .. })
    }
}

#[derive(Debug, Clone)]
pub struct CallAnswer {
    pub origin: String,
    pub outcome: CallOutcome,
    /// The reply's attachment, projected (JSON if it parses, UTF-8 text if
    /// it decodes, else a size tag) — never schema-decoded, an attachment is
    /// outside the registry's vocabulary (#117, #126). **Present only when
    /// the wire carried one** — absent, never null-when-unknown (O4); both
    /// fields are additive, so scripts on the old shape keep parsing.
    pub attachment: Option<serde_json::Value>,
    /// Its true size, regardless of how the projection reads.
    pub attachment_bytes: Option<usize>,
}

/// The wire shape is unchanged by the enum (pinned by
/// `tests/report_contract.rs`): `origin`, then `ok`, then the outcome's own
/// fields, then the attachment pair, with `error` last — every optional
/// absent rather than null, exactly as the derived struct serialized.
impl Serialize for CallAnswer {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut m = serializer.serialize_map(None)?;
        m.serialize_entry("origin", &self.origin)?;
        m.serialize_entry("ok", &self.outcome.is_ok())?;
        if let CallOutcome::Ok { value, text } = &self.outcome {
            if let Some(v) = value {
                m.serialize_entry("value", v)?;
            }
            if let Some(t) = text {
                m.serialize_entry("text", t)?;
            }
        }
        if let Some(a) = &self.attachment {
            m.serialize_entry("attachment", a)?;
        }
        if let Some(n) = self.attachment_bytes {
            m.serialize_entry("attachment_bytes", &n)?;
        }
        if let CallOutcome::Err(e) = &self.outcome {
            m.serialize_entry("error", e)?;
        }
        m.end()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CallReport {
    pub key: String,
    /// Seconds the GET waited — the other half of the coverage claim
    /// (zenctl's `GetReport` is the model), and what makes a silent result
    /// legible: the renderer's silence note used to name a timeout the
    /// document never stated (RFC 09 §5.1 O5, review finding R5). Additive,
    /// so scripts on the old shape keep parsing.
    pub timeout_s: f64,
    pub answers: Vec<CallAnswer>,
}

impl CallReport {
    /// The process exit code discipline (issue #12): 0 = at least one answer
    /// and no error replies; 1 = at least one error reply; 2 = zero replies
    /// (silence stays a distinct non-verdict — RFC 05 §3.1).
    pub fn exit_code(&self) -> i32 {
        if self.answers.is_empty() {
            2
        } else if self
            .answers
            .iter()
            .any(|a| matches!(a.outcome, CallOutcome::Err(_)))
        {
            1
        } else {
            0
        }
    }
}

/// The `zenctl probe` report (issue #59; RFC 09 §6 half two): how the
/// identity resolved, and what the origin-scoped concrete-key call said.
#[derive(Debug, Clone, Serialize)]
pub struct ProbeReport {
    /// What the operator typed (an origin id or a human label).
    pub input: String,
    /// The origin actually called.
    pub origin: String,
    /// `direct`, or `bridge:<key>` naming the self-certifying health
    /// document that resolved it (RFC 06 §6.2).
    pub via: String,
    pub call: CallReport,
}

/// Which rung of the fetch ladder produced a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueSource {
    /// A GET on the concrete key answered — a router storage (or any plain
    /// queryable standing at that key).
    Storage,
    /// The publisher's AdvancedPublisher cache answered on `<key>/@adv/**`.
    Cache,
    /// A brief bounded subscription caught a live sample.
    Window,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_exit_codes() {
        let mut r = CallReport {
            key: "k".into(),
            timeout_s: 5.0,
            answers: vec![],
        };
        assert_eq!(r.exit_code(), 2, "silence is its own exit code");
        r.answers.push(CallAnswer {
            origin: "h-1".into(),
            outcome: CallOutcome::Ok {
                value: None,
                text: Some("x".into()),
            },
            attachment: None,
            attachment_bytes: None,
        });
        assert_eq!(r.exit_code(), 0);
        r.answers.push(CallAnswer {
            origin: "h-2".into(),
            outcome: CallOutcome::Err(CallError {
                name: "error/busy".into(),
                message: "later".into(),
            }),
            attachment: None,
            attachment_bytes: None,
        });
        assert_eq!(r.exit_code(), 1, "any refusal fails the invocation");
    }
}
