//! The mock owner (#612, FJ8a): what `gen` brings up and publishes, what
//! `serve` answers, and what each did.
//!
//! A mock owner is a real zk2 service (P3, spec §6): it comes up through the
//! runtime with an instance, a descriptor and tokens, at the address the
//! operator names. These shapes are the paper trail that says a run was a
//! rehearsal: the plan before anything is published, the calls as they are
//! served, the counts at the end. v1's generator plane (`GenPlanEntry` by
//! registry subject, the fault kinds) went with v1's generator.

use std::collections::BTreeMap;

use serde::Serialize;
use zenkey_model::authoring::{Congestion, Kind, Priority, Reliability};

use super::payload::PayloadRendering;

/// What `gen` will bring up and publish, printed before anything is.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GenPlan {
    /// The address the mock owner runs at, `<system>/<service>`.
    pub address: String,
    /// The interfaces it implements, each at its revision.
    pub interfaces: Vec<GenInterface>,
    /// How long it publishes, seconds.
    pub duration_s: f64,
    /// The synthesis seed: the same seed is the same run.
    pub seed: u64,
    /// The marker its descriptor carries in `meta` (§3.3). One writer per
    /// key (P3) makes it a statement about every sample on the mock's keys.
    pub marker: serde_json::Value,
    pub entries: Vec<GenPlanEntry>,
}

/// One interface a mock owner implements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GenInterface {
    /// `<name>.v<major>`.
    pub iface: String,
    /// The revision served, `sha256:…`.
    pub fingerprint: String,
}

/// One resource member the mock publishes, or one operation it answers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GenPlanEntry {
    /// `<name>.v<major>`.
    pub iface: String,
    /// `<kind token>/<template>`, the descriptor's naming (§3.3).
    pub resource: String,
    pub kind: Kind,
    /// Base-relative. A member's concrete key; an event's prefix, below
    /// which each occurrence gets a fresh ULID chunk (§2.6); an
    /// operation's template, every parameter `*`.
    pub key: String,
    /// The member's template values, unslugged.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, Vec<String>>,
    /// Where those values came from.
    pub members: MemberSource,
    /// The payload's type (an operation's response), named as §7.2 names
    /// a type.
    pub declared: String,
    /// The `Encoding` every sample or reply carries (§7.2).
    pub encoding: String,
    /// The contract's QoS (§2.4), which every sample is published with.
    /// Absent for an operation: its replies inherit the caller's QoS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qos: Option<QosView>,
    /// Samples per second; absent for an operation, which answers calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_hz: Option<f64>,
    /// An event's cap on occurrences in the run: its declared rate, over
    /// the run's length (§2.6). A mock never out-publishes its contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events_cap: Option<u64>,
    /// Whether the member holds a member token (§8.1): set on the template
    /// that declares `epoch`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub member_token: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A resource's QoS, in the spec's spelling (§2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct QosView {
    pub reliability: Reliability,
    pub congestion: Congestion,
    pub priority: Priority,
    pub express: bool,
}

/// Where a member's template values came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberSource {
    /// `--member`, as the operator gave them.
    Given,
    /// A small synthetic set (`<param>-1`, `<param>-2`), stated in the plan.
    Default,
    /// The template has no parameters: the resource is its one member.
    Fixed,
}

/// What one `gen` run did.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GenReport {
    pub address: String,
    /// The instance id the mock owner came up as (§1.5).
    pub instance: String,
    pub duration_s: f64,
    /// Plan entries.
    pub entries: usize,
    /// Samples put on streams, states and events.
    pub sent: u64,
    /// Calls answered, every operation together.
    pub calls: u64,
    /// Samples not sent: a value the synthesizer could not make valid, or
    /// a put the runtime refused. Counted, never skipped.
    pub failed: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub first_errors: Vec<String>,
}

/// One call a mock owner served, logged as it was answered (O7).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ServedCall {
    /// Calls served so far, this one included.
    pub n: u64,
    /// `<name>.v<major>`.
    pub iface: String,
    /// `@op/<template>`.
    pub operation: String,
    /// The key expression called, base-relative: a concrete key, or a
    /// fan-out's selector (O2).
    pub key: String,
    /// Whether the key expression was concrete.
    pub concrete: bool,
    /// What the key binds of the template, unslugged (§5.1, "Over a
    /// template"): every parameter for a concrete call.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bound: BTreeMap<String, Vec<String>>,
    /// The request, decoded through the bundle (§7.2), or the rung it
    /// stopped at.
    pub request: PayloadRendering,
    /// The call metadata the caller claims (O7). Claimed, never
    /// authentication; absent when the attachment is not that object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<CallMetadataView>,
    /// What the mock answered. Tagged `answer`.
    #[serde(flatten)]
    pub answer: ServedAnswer,
}

/// The call metadata a request carries (O7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CallMetadataView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

/// What a mock answered one call with. Tagged `answer`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum ServedAnswer {
    /// The fixed reply, on the member's key (O3), and the summary when the
    /// operation declares one (O6).
    Reply { summary: bool },
    /// A refusal with this envelope code (§5.2).
    Refused { code: String },
    /// The reply could not be sent; the runtime answered `internal`, never
    /// silence (O3).
    Failed { error: String },
}

/// How a `serve` run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServeEnd {
    /// `--count` calls were served.
    Count,
    /// `--for` ran out.
    Window,
    /// Interrupted.
    Interrupted,
}

/// What one `serve` run did: its last row.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ServeSummary {
    pub address: String,
    /// The instance id the mock owner came up as (§1.5).
    pub instance: String,
    pub iface: String,
    pub fingerprint: String,
    /// `@op/<template>`.
    pub operation: String,
    pub calls: u64,
    pub elapsed_s: f64,
    pub ended: ServeEnd,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Rendered, ResolvedResource};
    use serde_json::json;

    fn entry() -> GenPlanEntry {
        GenPlanEntry {
            iface: "tc.netif.v1".into(),
            resource: "stream/bandwidth/{ns}/{iface}".into(),
            kind: Kind::Stream,
            key: "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/ns-1/iface-1".into(),
            values: [
                ("iface".to_owned(), vec!["iface-1".to_owned()]),
                ("ns".to_owned(), vec!["ns-1".to_owned()]),
            ]
            .into(),
            members: MemberSource::Default,
            declared: "json:BandwidthUpdate".into(),
            encoding: "application/json".into(),
            qos: Some(QosView {
                reliability: Reliability::BestEffort,
                congestion: Congestion::Drop,
                priority: Priority::Data,
                express: false,
            }),
            rate_hz: Some(1.0),
            events_cap: None,
            member_token: false,
            note: None,
        }
    }

    /// The plan names the address, each revision, the marker and every
    /// entry; an operation's entry has no QoS and no rate, and the absent
    /// facts are absent, never null.
    #[test]
    fn gen_plan_json_shape_is_pinned() {
        let plan = GenPlan {
            address: "host-a/tc".into(),
            interfaces: vec![GenInterface {
                iface: "tc.netif.v1".into(),
                fingerprint: format!("sha256:{}", "ab".repeat(32)),
            }],
            duration_s: 10.0,
            seed: 42,
            marker: json!({"synthetic": true, "tool": "zenctl gen", "seed": 42}),
            entries: vec![
                entry(),
                GenPlanEntry {
                    resource: "@op/diagnostics".into(),
                    kind: Kind::Operation,
                    key: "zk2/host-a/tc/tc.netif.v1/@op/diagnostics".into(),
                    values: BTreeMap::new(),
                    members: MemberSource::Fixed,
                    declared: "json:DiagnosticsResponse".into(),
                    qos: None,
                    rate_hz: None,
                    member_token: true,
                    note: Some("answers each call".into()),
                    ..entry()
                },
            ],
        };
        assert_eq!(
            serde_json::to_value(&plan).unwrap(),
            json!({
                "address": "host-a/tc",
                "interfaces": [{"iface": "tc.netif.v1",
                                "fingerprint": format!("sha256:{}", "ab".repeat(32))}],
                "duration_s": 10.0,
                "seed": 42,
                "marker": {"synthetic": true, "tool": "zenctl gen", "seed": 42},
                "entries": [
                    {
                        "iface": "tc.netif.v1",
                        "resource": "stream/bandwidth/{ns}/{iface}",
                        "kind": "stream",
                        "key": "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/ns-1/iface-1",
                        "values": {"iface": ["iface-1"], "ns": ["ns-1"]},
                        "members": "default",
                        "declared": "json:BandwidthUpdate",
                        "encoding": "application/json",
                        "qos": {"reliability": "best_effort", "congestion": "drop",
                                "priority": "data", "express": false},
                        "rate_hz": 1.0,
                    },
                    {
                        "iface": "tc.netif.v1",
                        "resource": "@op/diagnostics",
                        "kind": "operation",
                        "key": "zk2/host-a/tc/tc.netif.v1/@op/diagnostics",
                        "members": "fixed",
                        "declared": "json:DiagnosticsResponse",
                        "encoding": "application/json",
                        "member_token": true,
                        "note": "answers each call",
                    },
                ],
            })
        );
    }

    #[test]
    fn gen_report_json_shape_is_pinned() {
        let r = GenReport {
            address: "host-a/tc".into(),
            instance: "0123456789abcdef".into(),
            duration_s: 2.0,
            entries: 3,
            sent: 9,
            calls: 1,
            failed: 0,
            first_errors: vec![],
        };
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            json!({"address": "host-a/tc", "instance": "0123456789abcdef", "duration_s": 2.0,
                   "entries": 3, "sent": 9, "calls": 1, "failed": 0})
        );
    }

    /// A served call: the key, what it binds, the request rendered, the
    /// claimed metadata, and the answer tagged `answer`; each answer is its
    /// own tag.
    #[test]
    fn served_call_json_shape_is_pinned() {
        let key = "zk2/host-a/tc/tc.netif.v1/@op/diagnostics";
        let call = ServedCall {
            n: 1,
            iface: "tc.netif.v1".into(),
            operation: "@op/diagnostics".into(),
            key: key.into(),
            concrete: true,
            bound: BTreeMap::new(),
            request: PayloadRendering {
                key: key.into(),
                size: 2,
                resource: Some(ResolvedResource {
                    iface: "tc.netif.v1".into(),
                    fingerprint: format!("sha256:{}", "ab".repeat(32)),
                    resource: "@op/diagnostics".into(),
                    member: "request".into(),
                    values: BTreeMap::new(),
                }),
                rendered: Rendered::Value {
                    declared: "json:DiagnosticsRequest".into(),
                    value: json!({}),
                },
            },
            metadata: Some(CallMetadataView {
                actor: Some("ops".into()),
                request_id: None,
            }),
            answer: ServedAnswer::Reply { summary: false },
        };
        let v = serde_json::to_value(&call).unwrap();
        assert_eq!(v["answer"], "reply");
        assert_eq!(v["summary"], false);
        assert_eq!(v["metadata"], json!({"actor": "ops"}));
        assert!(v.get("bound").is_none());
        assert_eq!(v["request"]["as"], "value");
        let refused = ServedCall {
            answer: ServedAnswer::Refused {
                code: "busy".into(),
            },
            metadata: None,
            ..call.clone()
        };
        let v = serde_json::to_value(&refused).unwrap();
        assert_eq!(
            (v["answer"].clone(), v["code"].clone()),
            (json!("refused"), json!("busy"))
        );
        assert!(v.get("metadata").is_none());
        let failed = ServedCall {
            answer: ServedAnswer::Failed {
                error: "no member".into(),
            },
            ..call
        };
        assert_eq!(serde_json::to_value(&failed).unwrap()["answer"], "failed");
    }

    #[test]
    fn serve_summary_json_shape_is_pinned() {
        let s = ServeSummary {
            address: "host-a/tc".into(),
            instance: "0123456789abcdef".into(),
            iface: "tc.netif.v1".into(),
            fingerprint: format!("sha256:{}", "ab".repeat(32)),
            operation: "@op/diagnostics".into(),
            calls: 2,
            elapsed_s: 1.5,
            ended: ServeEnd::Count,
        };
        assert_eq!(
            serde_json::to_value(&s).unwrap(),
            json!({"address": "host-a/tc", "instance": "0123456789abcdef",
                   "iface": "tc.netif.v1", "fingerprint": format!("sha256:{}", "ab".repeat(32)),
                   "operation": "@op/diagnostics", "calls": 2, "elapsed_s": 1.5,
                   "ended": "count"})
        );
    }
}
