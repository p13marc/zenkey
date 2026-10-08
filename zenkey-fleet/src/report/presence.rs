//! The zk2 presence plane (spec §8.1, §3.3): services, their instances, and
//! what each instance's tokens and descriptor say about it — `service list`
//! and `service show`.
//!
//! v1's `service` domain is the `@rpc` plane of a producer
//! ([`crate::report::ServiceList`]), and FJ4 retires it with the v1 noun.
//! This one is named for where its evidence comes from: liveliness tokens,
//! then the descriptor each instance serves. Two sources, and every row
//! keeps them apart — a token's `fp16` beside a descriptor's fingerprint —
//! because a tool that merged them could no longer say which one was
//! missing, and "token missing" and "descriptor silent" are different
//! findings (FJ6).

use serde::Serialize;
use zenkey_model::descriptor::Descriptor;

use crate::report::Asked;

/// Every zk2 service one presence read saw (`service list`).
///
/// `complete` is the honesty field: a liveliness GET that ran to its
/// timeout may have missed tokens (§8.1), so `false` means a service absent
/// here may still be up — never that it is down (O5).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ServiceListing {
    /// The liveliness selector read, base-relative (`zk2/*/*/@zk/**`).
    pub selector: String,
    /// Whether the read completed before its timeout (§8.1).
    pub complete: bool,
    /// By address, sorted.
    pub services: Vec<ServiceSighting>,
    /// Keys the selector matched that are not zk2 tokens, verbatim. A
    /// non-conforming key under `@zk` is a fact about the bus, so it is
    /// shown rather than dropped.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unparsed: Vec<String>,
}

/// One service address and its instances.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ServiceSighting {
    /// `<system>/<service>`.
    pub address: String,
    /// By instance id, sorted. More than one is a re-mint's overlap, a
    /// redundant pair, or a split brain: telling them apart is a judgement
    /// (`doctor`, FJ6), not this listing's.
    pub instances: Vec<InstanceSighting>,
    /// Member tokens (r3.3 D9b), by interface and member. They belong to
    /// the service, not to an instance: a member's key carries its own
    /// epoch, never the instance id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<MemberSighting>,
}

/// One instance: its tokens, and its descriptor when it was asked for.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InstanceSighting {
    /// 16 lowercase hex digits.
    pub instance: String,
    /// The instance token was read (§8.1). `false` with interface or member
    /// tokens present: the instance is known only from those, which is a
    /// make-before-break overlap caught mid-way or an owner that broke
    /// §8.2's order.
    pub instance_token: bool,
    /// Every interface its tokens or its descriptor name, by interface.
    pub interfaces: Vec<IfaceSighting>,
    /// The descriptor GET's answer (§3.3). Absent when it was not asked.
    #[serde(default, skip_serializing_if = "Asked::is_not_asked")]
    pub descriptor: Asked<DescriptorAnswer>,
}

/// One interface of one instance, as its token and its descriptor name it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IfaceSighting {
    /// `<name>.v<major>`.
    pub iface: String,
    /// The first 64 bits of the contract fingerprint, from the interface
    /// token. Absent when no token was read: a tokenless interface, or a
    /// missing token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// The full fingerprint (`sha256:…`) the descriptor names. Absent when
    /// the descriptor was not read or does not list the interface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<String>,
    /// The contract's informative minor, from the descriptor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minor: Option<u64>,
    /// The descriptor marks the interface tokenless (`"token": false`, U22):
    /// its provider holds no interface token for it, by configuration.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tokenless: bool,
}

/// One member token (`…/@zk/member/<iface>/<member>/<epoch>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemberSighting {
    pub iface: String,
    /// The slugged value of the template's `epoch` parameter.
    pub member: String,
    /// The member's continuity epoch.
    pub epoch: String,
}

/// What a descriptor GET came back with (§3.3). Silence is its own answer,
/// never folded into "invalid" or into "no interfaces".
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum DescriptorAnswer {
    /// A descriptor that passes the syntax checks, as the spec spells it
    /// (`zk2-descriptor/0.1`). Exposure is not checked here: that needs the
    /// contracts (§3.3).
    Served { descriptor: Box<Descriptor> },
    /// A reply that fails the descriptor check, with its findings.
    Invalid { findings: String },
    /// No reply within the timeout: the instance may be gone, or the reply
    /// may not have crossed. Not a verdict.
    Silent,
    /// The GET could not be put on the bus.
    Failed { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn descriptor() -> Descriptor {
        serde_json::from_value(json!({
            "format": "zk2-descriptor/0.1",
            "service": "host-a/tc",
            "instance": "8f3a5c2e9b1d4f70",
            "interfaces": [{"iface": "tc.netif.v1", "contract": format!("sha256:{}", "ab".repeat(32)), "minor": 0}],
        }))
        .expect("a descriptor")
    }

    /// The document `service list --format json` prints: absences are
    /// absent (no `token` without a token, no `descriptor` when not asked),
    /// and `tokenless` appears only when the descriptor says so.
    #[test]
    fn service_listing_json_shape_is_pinned() {
        let listing = ServiceListing {
            selector: "zk2/*/*/@zk/**".into(),
            complete: false,
            services: vec![ServiceSighting {
                address: "host-a/tc".into(),
                instances: vec![
                    InstanceSighting {
                        instance: "8f3a5c2e9b1d4f70".into(),
                        instance_token: true,
                        interfaces: vec![
                            IfaceSighting {
                                iface: "health.v1".into(),
                                token: None,
                                contract: Some(format!("sha256:{}", "cd".repeat(32))),
                                minor: Some(2),
                                tokenless: true,
                            },
                            IfaceSighting {
                                iface: "tc.netif.v1".into(),
                                token: Some("abababababababab".into()),
                                contract: None,
                                minor: None,
                                tokenless: false,
                            },
                        ],
                        descriptor: Asked::Asked(DescriptorAnswer::Silent),
                    },
                    InstanceSighting {
                        instance: "0000000000000002".into(),
                        instance_token: false,
                        interfaces: vec![],
                        descriptor: Asked::NotAsked,
                    },
                ],
                members: vec![MemberSighting {
                    iface: "tc.netif.v1".into(),
                    member: "eth0".into(),
                    epoch: "0000000000000001".into(),
                }],
            }],
            unparsed: vec!["zk2/host-a/tc/@zk/bogus".into()],
        };
        assert_eq!(
            serde_json::to_value(&listing).expect("serialize"),
            json!({
                "selector": "zk2/*/*/@zk/**",
                "complete": false,
                "services": [{
                    "address": "host-a/tc",
                    "instances": [
                        {
                            "instance": "8f3a5c2e9b1d4f70",
                            "instance_token": true,
                            "interfaces": [
                                {
                                    "iface": "health.v1",
                                    "contract": format!("sha256:{}", "cd".repeat(32)),
                                    "minor": 2,
                                    "tokenless": true,
                                },
                                {"iface": "tc.netif.v1", "token": "abababababababab"},
                            ],
                            "descriptor": {"answer": "silent"},
                        },
                        {"instance": "0000000000000002", "instance_token": false, "interfaces": []},
                    ],
                    "members": [{"iface": "tc.netif.v1", "member": "eth0", "epoch": "0000000000000001"}],
                }],
                "unparsed": ["zk2/host-a/tc/@zk/bogus"],
            })
        );
    }

    /// A served descriptor rides verbatim, in the spec's own spelling, under
    /// the answer tag; the other answers carry their reason.
    #[test]
    fn descriptor_answers_json_shape_is_pinned() {
        let served = serde_json::to_value(DescriptorAnswer::Served {
            descriptor: Box::new(descriptor()),
        })
        .expect("serialize");
        assert_eq!(served["answer"], "served");
        assert_eq!(served["descriptor"]["service"], "host-a/tc");
        assert_eq!(served["descriptor"]["format"], "zk2-descriptor/0.1");
        assert_eq!(
            serde_json::to_value(DescriptorAnswer::Invalid {
                findings: "D001 bad format".into()
            })
            .expect("serialize"),
            json!({"answer": "invalid", "findings": "D001 bad format"})
        );
        assert_eq!(
            serde_json::to_value(DescriptorAnswer::Failed {
                reason: "session closed".into()
            })
            .expect("serialize"),
            json!({"answer": "failed", "reason": "session closed"})
        );
    }
}
