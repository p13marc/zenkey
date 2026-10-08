//! A zk2 operation call (spec §5.1): `zenctl call`, through the runtime's
//! `Client` (one address) or `Fleet` (a fan-out).
//!
//! The answer keeps O5's cases apart in every medium: a value, an envelope,
//! a malformed envelope and silence are four tags, never one field read
//! four ways. Silence carries what presence said of the service, worded as
//! what this reader could see (§8.1, 0.8). A many-reply or fan-out call
//! keeps every reply attributed by its key (O3, O6); its envelopes are
//! unattributed, because a `reply_err` carries no key (§5.1,
//! "Attribution").
//!
//! v1's `@rpc` call is `call`'s domain (`CallReport`); the names here do
//! not collide with it.

use std::collections::BTreeMap;

use serde::Serialize;

use super::payload::{PayloadRendering, Rendered};

/// One call of one operation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OperationReport {
    /// The address called, `<system>/<service>`; a `*` position is a
    /// fan-out's selection.
    pub address: String,
    /// `<name>.v<major>`.
    pub iface: String,
    /// The revision the call was planned and its request encoded against.
    pub fingerprint: String,
    /// `@op/<template>`.
    pub operation: String,
    /// The template values given, unslugged. A parameter not given is a
    /// wildcard, which makes the call a fan-out.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, Vec<String>>,
    /// The key expressions the call went out on, base-relative: the one
    /// concrete key, or a selector per selected provider.
    pub selectors: Vec<String>,
    /// How the query went out.
    pub mode: CallMode,
    /// How long the call waited for replies, seconds.
    pub timeout_s: f64,
    /// What came back. Tagged `answer`.
    #[serde(flatten)]
    pub answer: OperationAnswer,
}

/// How a call's query went out (spec §5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CallMode {
    /// One concrete key: target `BestMatching`, consolidation `None` (O1).
    Concrete,
    /// A key expression with a wildcard: target `All`, consolidation `None`,
    /// only to an operation declaring `fanout = "allowed"` (O2).
    Fanout,
}

/// What a call got back. Tagged `answer`; each case is distinct (O5).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum OperationAnswer {
    /// A value reply on the call's own key (O3), rendered through the
    /// contract's `response` type.
    Value { reply: PayloadRendering },
    /// A `reply_err` carrying a valid envelope (O3, §5.2): an answer.
    Refused { envelope: EnvelopeView },
    /// A `reply_err` claiming an envelope encoding that does not decode: a
    /// broken server, not a silent one (§5.2).
    Malformed { malformed: MalformedReply },
    /// No value and no envelope: silence, never a verdict (O5).
    Silent { silence: SilenceView },
    /// A many-reply call or a fan-out: every reply, attributed (O3, O6).
    Replies { replies: RepliesView },
}

/// An error envelope (§5.2), decoded without the contract and its detail
/// rendered through the operation's `error` type when there is one.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EnvelopeView {
    /// `invalid_request`, `not_found`, `unavailable`, `forbidden`,
    /// `fanout_forbidden`, `busy`, `internal` or `app`.
    pub code: String,
    /// For a human, never parsed.
    pub message: String,
    /// With `unavailable` only: `build`, `config` or `capability`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// With `app` only, when the owner sent one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Rendered>,
}

/// A reply that claims an envelope encoding and is not an envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MalformedReply {
    /// The reply's `Encoding`.
    pub encoding: String,
    /// Why it did not decode (§5.2's refusals).
    pub error: String,
}

/// A call that got no answer (O5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SilenceView {
    /// Calls made: more than one only for an idempotent operation with
    /// retries asked for (O4).
    pub attempts: u32,
    /// The transport's last error reply, such as `zenoh/string: Timeout`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    /// What presence said of the service, read after the silence.
    pub presence: PresenceAttribution,
}

/// What presence says of a service that sent no answer (O5, §8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceAttribution {
    /// It holds the interface's token: up, and did not answer. The access
    /// control refused the call, the server is frozen, or the reply was
    /// lost.
    Present,
    /// It holds an instance token and not this interface's: a standby, or
    /// the interface is in its tokenless set (§8.1).
    InstanceOnly,
    /// No token of the service is visible to this reader, on a read that
    /// completed. A refused read is complete and empty too (0.8): this is
    /// what the reader could see, not a verdict that the service is gone.
    NoTokenVisible,
    /// The presence read was possibly incomplete and saw no interface
    /// token.
    Unknown,
    /// Presence cannot be observed from here (R7).
    Unobservable,
}

/// Every reply to a many-reply call or a fan-out (O6).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RepliesView {
    /// Whether the operation declares a `summary`. Without one, a
    /// replier's completion cannot be told, and no replier carries
    /// `possibly_partial`.
    pub summary_declared: bool,
    /// One per concrete key a value or summary arrived on, in order of
    /// first arrival.
    pub repliers: Vec<ReplierView>,
    /// Envelopes, unattributed: a `reply_err` carries no key (§5.1).
    pub refusals: Vec<EnvelopeView>,
    pub malformed: Vec<MalformedReply>,
    /// The transport's error replies, such as a timeout.
    pub transport: Vec<String>,
    /// Value replies discarded: on a key that is not concrete (R6), or not
    /// a member of the operation called.
    pub discarded: u64,
    /// The selection's presence, read after the call: who holds the
    /// interface's token and sent no value.
    pub presence: SelectionPresence,
}

/// The replies one concrete key carried (O6).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReplierView {
    /// `<system>/<service>`, from the key.
    pub address: String,
    /// The operation key the replies went on.
    pub key: String,
    /// The template's values on that key, unslugged.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, Vec<String>>,
    /// Value replies, rendered through the `response` type.
    pub replies: Vec<PayloadRendering>,
    /// Summary replies, rendered through the `summary` type.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub summaries: Vec<PayloadRendering>,
    /// With a declared summary: whether the replier ended without exactly
    /// one — cut off, broken off, or several instances on one key (O6).
    /// Absent without a declared summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub possibly_partial: Option<bool>,
}

/// Who a fan-out selected and did not hear from (§5.1, "Attribution").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SelectionPresence {
    /// The liveliness selector read, base-relative.
    pub selector: String,
    /// Whether the read completed (§8.1). Possibly incomplete: a holder it
    /// missed is not listed below.
    pub complete: bool,
    /// Services holding the interface's token that sent no value: each
    /// refused or was silent, which the caller cannot tell apart. A
    /// provider with the interface in its tokenless set holds no token and
    /// is not listed (§8.1).
    pub unheard: Vec<String>,
    /// Why the read could not be made, when it could not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl OperationReport {
    /// The exit code `call` keeps from v1's `service call`: 0 a value and
    /// no error reply, 1 an envelope or a malformed one (the finding), 2
    /// silence — never a verdict (O5).
    pub fn exit_code(&self) -> i32 {
        match &self.answer {
            OperationAnswer::Value { .. } => 0,
            OperationAnswer::Refused { .. } | OperationAnswer::Malformed { .. } => 1,
            OperationAnswer::Silent { .. } => 2,
            OperationAnswer::Replies { replies } => {
                if replies.repliers.is_empty()
                    && replies.refusals.is_empty()
                    && replies.malformed.is_empty()
                {
                    2
                } else if !replies.refusals.is_empty() || !replies.malformed.is_empty() {
                    1
                } else {
                    0
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::ResolvedResource;
    use serde_json::json;

    fn report(answer: OperationAnswer, mode: CallMode) -> OperationReport {
        OperationReport {
            address: "host-a/tc".into(),
            iface: "tc.netif.v1".into(),
            fingerprint: format!("sha256:{}", "ab".repeat(32)),
            operation: "@op/diagnostics".into(),
            values: BTreeMap::new(),
            selectors: vec!["zk2/host-a/tc/tc.netif.v1/@op/diagnostics".into()],
            mode,
            timeout_s: 5.0,
            answer,
        }
    }

    fn reply(key: &str) -> PayloadRendering {
        PayloadRendering {
            key: key.into(),
            size: 9,
            resource: Some(ResolvedResource {
                iface: "tc.netif.v1".into(),
                fingerprint: format!("sha256:{}", "ab".repeat(32)),
                resource: "@op/diagnostics".into(),
                member: "response".into(),
                values: BTreeMap::new(),
            }),
            rendered: Rendered::Value {
                declared: "json:DiagnosticsResponse".into(),
                value: json!({"ok": true}),
            },
        }
    }

    /// The four O5 cases are four tags: a script branches on `answer`.
    #[test]
    fn operation_report_json_shape_is_pinned() {
        let key = "zk2/host-a/tc/tc.netif.v1/@op/diagnostics";
        let v = serde_json::to_value(report(
            OperationAnswer::Value { reply: reply(key) },
            CallMode::Concrete,
        ))
        .unwrap();
        assert_eq!(
            v,
            json!({
                "address": "host-a/tc",
                "iface": "tc.netif.v1",
                "fingerprint": format!("sha256:{}", "ab".repeat(32)),
                "operation": "@op/diagnostics",
                "selectors": [key],
                "mode": "concrete",
                "timeout_s": 5.0,
                "answer": "value",
                "reply": {
                    "key": key,
                    "size": 9,
                    "resource": {
                        "iface": "tc.netif.v1",
                        "fingerprint": format!("sha256:{}", "ab".repeat(32)),
                        "resource": "@op/diagnostics",
                        "member": "response",
                    },
                    "as": "value",
                    "declared": "json:DiagnosticsResponse",
                    "value": {"ok": true},
                },
            })
        );

        let refused = serde_json::to_value(report(
            OperationAnswer::Refused {
                envelope: EnvelopeView {
                    code: "unavailable".into(),
                    message: "no such NIC here".into(),
                    cause: Some("capability".into()),
                    detail: None,
                },
            },
            CallMode::Concrete,
        ))
        .unwrap();
        assert_eq!(refused["answer"], "refused");
        assert_eq!(
            refused["envelope"],
            json!({"code": "unavailable", "message": "no such NIC here", "cause": "capability"})
        );

        let silent = serde_json::to_value(report(
            OperationAnswer::Silent {
                silence: SilenceView {
                    attempts: 1,
                    transport: Some("zenoh/string: Timeout".into()),
                    presence: PresenceAttribution::NoTokenVisible,
                },
            },
            CallMode::Concrete,
        ))
        .unwrap();
        assert_eq!(silent["answer"], "silent");
        assert_eq!(
            silent["silence"],
            json!({"attempts": 1, "transport": "zenoh/string: Timeout", "presence": "no_token_visible"})
        );

        let malformed = serde_json::to_value(report(
            OperationAnswer::Malformed {
                malformed: MalformedReply {
                    encoding: "application/json".into(),
                    error: "decode".into(),
                },
            },
            CallMode::Concrete,
        ))
        .unwrap();
        assert_eq!(malformed["answer"], "malformed");
        assert_eq!(
            malformed["malformed"],
            json!({"encoding": "application/json", "error": "decode"})
        );
    }

    /// A fan-out: repliers by key, envelopes unattributed, the unheard
    /// holders from presence; `possibly_partial` only with a summary.
    #[test]
    fn replies_json_shape_is_pinned() {
        let k1 = "zk2/h1/tc/tc.netif.v1/@op/diagnostics";
        let r = report(
            OperationAnswer::Replies {
                replies: RepliesView {
                    summary_declared: false,
                    repliers: vec![ReplierView {
                        address: "h1/tc".into(),
                        key: k1.into(),
                        values: BTreeMap::new(),
                        replies: vec![reply(k1)],
                        summaries: vec![],
                        possibly_partial: None,
                    }],
                    refusals: vec![EnvelopeView {
                        code: "busy".into(),
                        message: "later".into(),
                        cause: None,
                        detail: None,
                    }],
                    malformed: vec![],
                    transport: vec![],
                    discarded: 0,
                    presence: SelectionPresence {
                        selector: "zk2/*/tc/@zk/alive/tc.netif.v1/**".into(),
                        complete: true,
                        unheard: vec!["h3/tc".into()],
                        error: None,
                    },
                },
            },
            CallMode::Fanout,
        );
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["mode"], "fanout");
        assert_eq!(v["answer"], "replies");
        let replies = &v["replies"];
        assert_eq!(replies["summary_declared"], false);
        assert_eq!(replies["repliers"][0]["address"], "h1/tc");
        assert!(replies["repliers"][0].get("possibly_partial").is_none());
        assert!(replies["repliers"][0].get("summaries").is_none());
        assert_eq!(
            replies["refusals"],
            json!([{"code": "busy", "message": "later"}])
        );
        assert_eq!(
            replies["presence"],
            json!({
                "selector": "zk2/*/tc/@zk/alive/tc.netif.v1/**",
                "complete": true,
                "unheard": ["h3/tc"],
            })
        );
        assert_eq!(r.exit_code(), 1, "an envelope is the finding");
    }

    /// v1's mapping, kept: a value 0, an envelope 1, silence 2.
    #[test]
    fn the_exit_code_keeps_silence_apart() {
        let silent = OperationAnswer::Silent {
            silence: SilenceView {
                attempts: 1,
                transport: None,
                presence: PresenceAttribution::Present,
            },
        };
        assert_eq!(report(silent, CallMode::Concrete).exit_code(), 2);
        let empty = OperationAnswer::Replies {
            replies: RepliesView {
                summary_declared: true,
                repliers: vec![],
                refusals: vec![],
                malformed: vec![],
                transport: vec!["zenoh/string: Timeout".into()],
                discarded: 3,
                presence: SelectionPresence {
                    selector: "zk2/*/tc/@zk/alive/tc.netif.v1/**".into(),
                    complete: false,
                    unheard: vec![],
                    error: None,
                },
            },
        };
        assert_eq!(report(empty, CallMode::Fanout).exit_code(), 2);
        let value = OperationAnswer::Value {
            reply: reply("zk2/host-a/tc/tc.netif.v1/@op/diagnostics"),
        };
        assert_eq!(report(value, CallMode::Concrete).exit_code(), 0);
    }
}
