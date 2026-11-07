//! A zk2 operation call (spec §5.1): `zenctl call`, through the runtime's
//! `Client` (one address) or `Fleet` (a fan-out).
//!
//! The answer keeps O5's cases apart in every medium: a value, an owner's
//! error and silence are three tags, never one field read three ways. A
//! value or an error that came in its form carries the replying instance's
//! as-of mark, `mark` (core §5.1, observer §7.3); a malformed value
//! carries none. Core 0.99 removed the bare wire (#985's C4d), and with it
//! the `malformed` answer, an envelope its decoding refused: a reply error
//! that is no error form is the transport's, and silence. Silence carries
//! what presence said of the service, worded as
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
use super::state::InbandStamp;

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
    /// The deployment epoch, in seconds, the request's packed `time` fields
    /// were written under because no catalog stated one (core §7.6, §8.6):
    /// the owner reads them under its own, so they are right only where it
    /// is this one. Absent for every other request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_epoch_assumed: Option<i64>,
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
    /// contract's `response` type: a value form's record, or undecodable
    /// when malformed, refused by the form's parse or under another
    /// `Encoding` (core §5.1).
    Value {
        reply: PayloadRendering,
        /// The value form's as-of mark (core §5.1): the run as the
        /// replier's descriptor spells it, and the seq. Absent for a
        /// malformed reply.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mark: Option<InbandStamp>,
    },
    /// A `reply_err` carrying an owner's error in its error form (O3,
    /// §5.2): an answer.
    Refused { envelope: EnvelopeView },
    /// No value and no owner's error: silence, never a verdict (O5).
    Silent { silence: SilenceView },
    /// A many-reply call or a fan-out: every reply, attributed (O3, O6).
    Replies { replies: RepliesView },
}

/// An owner's error (§5.2), read without the contract from its error form,
/// its detail rendered through the operation's `error` type when there is
/// one.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EnvelopeView {
    /// `invalid_request`, `not_found`, `unavailable`, `forbidden`,
    /// `fanout_forbidden`, `busy`, `internal` or `app`.
    pub code: String,
    /// For a human, never parsed; empty when an error form carried none.
    pub message: String,
    /// With `unavailable` only: `build`, `config` or `capability`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// With `app` only, when the owner sent one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Rendered>,
    /// The error form's as-of mark (core §5.1). It attributes nothing: a
    /// `reply_err` carries no key. Absent only for a refusal made before
    /// any wire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark: Option<InbandStamp>,
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
    /// Owners' errors, unattributed: a `reply_err` carries no key (§5.1).
    pub refusals: Vec<EnvelopeView>,
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
    /// The as-of mark of each reply, then of each summary, in their order
    /// (core §5.1): `null` for a malformed one, refused by the form's
    /// parse or under another `Encoding`. Absent when none came in the
    /// value form. Two runs on
    /// one key are two instances.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<Option<InbandStamp>>,
    /// With a declared summary: whether the replier ended without exactly
    /// one — cut off, broken off, or several instances on one key (O6).
    /// Absent without a declared summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub possibly_partial: Option<bool>,
}

/// Who a fan-out selected and did not hear from (§5.1, "Attribution").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SelectionPresence {
    /// The interface-token selector read, base-relative. A provider with
    /// the interface in its tokenless set holds no token there, and is read
    /// by its instance token and descriptor beside it (§8.1, #1015).
    pub selector: String,
    /// Whether the reads completed (§8.1). Possibly incomplete: a provider
    /// they missed is not listed below.
    pub complete: bool,
    /// Services serving the interface that sent no value, by its token or,
    /// tokenless, by instance token and descriptor: each refused or was
    /// silent, which the caller cannot tell apart.
    pub unheard: Vec<String>,
    /// Why the read could not be made, when it could not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl OperationReport {
    /// The exit code `call` keeps from v1's `service call`: 0 a value and
    /// no owner's error, 1 an owner's error (the finding), 2 silence —
    /// never a verdict (O5).
    pub fn exit_code(&self) -> i32 {
        match &self.answer {
            OperationAnswer::Value { .. } => 0,
            OperationAnswer::Refused { .. } => 1,
            OperationAnswer::Silent { .. } => 2,
            OperationAnswer::Replies { replies } => {
                if replies.repliers.is_empty() && replies.refusals.is_empty() {
                    2
                } else if !replies.refusals.is_empty() {
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
            request_epoch_assumed: None,
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
                epoch_assumed: None,
            },
        }
    }

    /// The four O5 cases are four tags: a script branches on `answer`.
    #[test]
    fn operation_report_json_shape_is_pinned() {
        let key = "zk2/host-a/tc/tc.netif.v1/@op/diagnostics";
        let v = serde_json::to_value(report(
            OperationAnswer::Value {
                reply: reply(key),
                mark: None,
            },
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
        // A value form's as-of mark rides beside its reply (core §5.1).
        let mark = InbandStamp {
            run: "0001".into(),
            seq: 7,
        };
        let marked = serde_json::to_value(report(
            OperationAnswer::Value {
                reply: reply(key),
                mark: Some(mark.clone()),
            },
            CallMode::Concrete,
        ))
        .unwrap();
        assert_eq!(marked["mark"], json!({"run": "0001", "seq": 7}));

        let refused = serde_json::to_value(report(
            OperationAnswer::Refused {
                envelope: EnvelopeView {
                    code: "unavailable".into(),
                    message: "no such NIC here".into(),
                    cause: Some("capability".into()),
                    detail: None,
                    mark: None,
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
        // An error form's: its mark, and no message when it sent none.
        let form = serde_json::to_value(report(
            OperationAnswer::Refused {
                envelope: EnvelopeView {
                    code: "busy".into(),
                    message: String::new(),
                    cause: None,
                    detail: None,
                    mark: Some(mark),
                },
            },
            CallMode::Concrete,
        ))
        .unwrap();
        assert_eq!(
            form["envelope"],
            json!({"code": "busy", "message": "", "mark": {"run": "0001", "seq": 7}})
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
                        marks: vec![],
                        possibly_partial: None,
                    }],
                    refusals: vec![EnvelopeView {
                        code: "busy".into(),
                        message: "later".into(),
                        cause: None,
                        detail: None,
                        mark: None,
                    }],
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
        assert!(
            replies["repliers"][0].get("marks").is_none(),
            "no value form"
        );
        assert!(replies.get("malformed").is_none(), "core 0.99");
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
        assert_eq!(r.exit_code(), 1, "an owner's error is the finding");
    }

    /// v1's mapping, kept: a value 0, an owner's error 1, silence 2.
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
            mark: None,
        };
        assert_eq!(report(value, CallMode::Concrete).exit_code(), 0);
    }
}
