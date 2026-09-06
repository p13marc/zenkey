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

/// What a bounded reply says about itself (RFC 05 §3.2, v1.31): the
/// envelope's `partial` flag and the three fields that qualify it.
///
/// **Derived, not serialized.** The wire already carries the envelope
/// inside [`CallOutcome::Ok`]'s `value`; this is the renderer's reading of
/// it, so it has no serde derive and no place in the pinned contract — a
/// script that wants the flag reads the reply it is in.
///
/// `Some` only when the reply is a JSON object carrying a boolean `partial`
/// — which is the envelope's one required marker. A procedure that replies
/// with a bare list, a scalar, or TOML text (the introspect procedures) is
/// not paginated and gets no signal at all rather than a synthetic
/// `partial: false`: an absent envelope and a complete walk are different
/// facts, and the caller must not be told the second when only the first
/// is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageSignal {
    /// The producer stopped before completing the walk — scan cap, tier
    /// coverage, time budget — and a short page is not the end.
    pub partial: bool,
    /// Non-null means more; `null` means the walk is complete for the
    /// filter given. Opaque to the caller (a value cursor, never a
    /// position).
    pub next_cursor: Option<String>,
    /// Advisory: what the page cost, so an expensive empty page can be
    /// told from a cheap one. Omitted by procedures that do not count.
    pub scanned: Option<u64>,
    /// The oldest instant the answer *could* have covered, for a computed
    /// answer narrower than what was asked. Omitted by procedures with no
    /// notion of coverage.
    pub covers_from: Option<String>,
}

impl PageSignal {
    /// `partial: true` **with** `next_cursor: null`: the producer says it
    /// stopped early and offers no way on. RFC 05 §3.2 names this a
    /// contract violation an observer MAY report (RFC 13 §3); `zenctl
    /// call` says it as a caveat and does not move the exit code, because
    /// a call is an act, not a judgement (#424).
    pub fn is_contract_violation(&self) -> bool {
        self.partial && self.next_cursor.is_none()
    }
}

impl CallAnswer {
    /// The RFC 05 §3.2 envelope fields of this answer, when it is one.
    ///
    /// `None` for an error envelope, a text reply, a non-object value, and
    /// an object with no boolean `partial` — see [`PageSignal`] for why an
    /// absent envelope is not reported as a complete one. A `next_cursor`
    /// that is present but not a string is read as null: the RFC makes the
    /// cursor opaque, and an opaque value the caller cannot pass back is
    /// no way on.
    pub fn page_signal(&self) -> Option<PageSignal> {
        let CallOutcome::Ok {
            value: Some(serde_json::Value::Object(o)),
            ..
        } = &self.outcome
        else {
            return None;
        };
        let partial = o.get("partial")?.as_bool()?;
        Some(PageSignal {
            partial,
            next_cursor: o
                .get("next_cursor")
                .and_then(|c| c.as_str())
                .map(str::to_string),
            scanned: o.get("scanned").and_then(|n| n.as_u64()),
            covers_from: o
                .get("covers_from")
                .and_then(|c| c.as_str())
                .map(str::to_string),
        })
    }
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

    fn answer(outcome: CallOutcome) -> CallAnswer {
        CallAnswer {
            origin: "h-1".into(),
            outcome,
            attachment: None,
            attachment_bytes: None,
        }
    }

    fn value(v: serde_json::Value) -> CallAnswer {
        answer(CallOutcome::Ok {
            value: Some(v),
            text: None,
        })
    }

    #[test]
    fn page_signal_reads_only_object_replies_with_partial() {
        // The envelope, whole (RFC 05 §3.2).
        let full = value(serde_json::json!({
            "items": [1, 2],
            "next_cursor": "k-2",
            "partial": true,
            "scanned": 4096,
            "covers_from": "2026-09-06T10:00:00Z"
        }));
        assert_eq!(
            full.page_signal(),
            Some(PageSignal {
                partial: true,
                next_cursor: Some("k-2".into()),
                scanned: Some(4096),
                covers_from: Some("2026-09-06T10:00:00Z".into()),
            })
        );
        assert!(!full.page_signal().unwrap().is_contract_violation());

        // The minimal envelope: the optional fields absent, and a null
        // cursor after `partial: true` is the violation the RFC names.
        let stuck = value(serde_json::json!({"items": [], "next_cursor": null, "partial": true}));
        let p = stuck
            .page_signal()
            .expect("an object with a boolean partial");
        assert!(p.is_contract_violation());
        assert_eq!((p.scanned, p.covers_from), (None, None));

        // A complete page is a signal too, and not a violation.
        let done = value(serde_json::json!({"items": [], "next_cursor": null, "partial": false}));
        assert!(!done.page_signal().unwrap().is_contract_violation());

        // Not envelopes: a bare list, a scalar, an object without the flag,
        // a non-boolean flag, a text reply, an error. None of these is a
        // complete walk, so none is told as one.
        for a in [
            value(serde_json::json!([1, 2, 3])),
            value(serde_json::json!(42)),
            value(serde_json::json!({"count": 214})),
            value(serde_json::json!({"partial": "yes"})),
            answer(CallOutcome::Ok {
                value: None,
                text: Some("partial = true".into()),
            }),
            answer(CallOutcome::Err(CallError {
                name: "error/busy".into(),
                message: "later".into(),
            })),
        ] {
            assert_eq!(a.page_signal(), None, "{a:?}");
        }
    }
}
