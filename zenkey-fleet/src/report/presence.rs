//! The zk2 presence plane (spec §8.1, §3.3): services, their instances, and
//! what each instance's tokens and descriptor say about it — `service list`
//! and `service show`.
//!
//! v1's `service` domain is the `@rpc` plane of a producer
//! (v1's `service list|info`), which FJ4 retired with the v1 noun.
//! This one is named for where its evidence comes from: liveliness tokens,
//! then the descriptor each instance serves. Two sources, and every row
//! keeps them apart — a token's `fp16` beside a descriptor's fingerprint —
//! because a tool that merged them could no longer say which one was
//! missing, and "token missing" and "descriptor silent" are different
//! findings (FJ6).
//!
//! **Live presence** (#614): an address's three states and the moves of
//! [`crate::model::presence_live`]'s projection — [`AddressStatus`],
//! [`PresenceTransition`] and what it carries — live here since FL2,
//! because zenwatch's notifications carry them to a webhook or a program
//! (`zenwatch/src/sinks/mod.rs`): a script switches on `change.kind` and
//! reads the address without parsing a sentence.

use std::fmt;
use std::time::SystemTime;

use serde::{Serialize, Serializer};
use zenkey_model::descriptor::Descriptor;
use zenkey_model::grammar::{Addr, Fp16, IfaceId, InstanceId};

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
    /// The read's session was cut off while it ran (core §8.1, 0.38): zenoh
    /// answered it at the cut, so `complete` is false. Absent otherwise.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cut_off_during: bool,
    /// By address, sorted.
    pub services: Vec<ServiceSighting>,
    /// Keys the selector matched that are not zk2 tokens, verbatim. A
    /// non-conforming key under `@zk` is a fact about the bus, so it is
    /// shown rather than dropped.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unparsed: Vec<String>,
}

/// One service (`service show`): its instances, each with its tokens and
/// the descriptor it serves, read from a presence scope of that address
/// alone.
///
/// No instance is an answer, not a verdict: with `complete` false the
/// service may still be up (O5), and even a complete read only says that
/// no token was held when it ran.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ServiceView {
    /// `<system>/<service>`.
    pub address: String,
    /// The liveliness selector read, base-relative
    /// (`zk2/<system>/<service>/@zk/**`).
    pub selector: String,
    /// Whether the read completed before its timeout (§8.1).
    pub complete: bool,
    /// The read's session was cut off while it ran (core §8.1, 0.38): zenoh
    /// answered it at the cut, so `complete` is false. Absent otherwise.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cut_off_during: bool,
    /// By instance id, as [`ServiceSighting::instances`].
    pub instances: Vec<InstanceSighting>,
    /// Member tokens, as [`ServiceSighting::members`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<MemberSighting>,
    /// Keys under the scope that are not zk2 tokens, verbatim.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unparsed: Vec<String>,
}

/// Every deployment namespace in which a zk2 service holds an instance
/// token (`namespace list`), read un-namespaced.
///
/// The read is the one place a zk2 tool looks across namespaces: the
/// resolved verbs run **in** one (decided 2026-10-08), so finding which
/// ones exist is asked of the raw bus, like the admin space.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NamespaceListing {
    /// The liveliness selector read, un-namespaced
    /// (`**/zk2/*/*/@zk/instance/*`).
    pub selector: String,
    /// Whether the read completed before its timeout (§8.1).
    pub complete: bool,
    /// The read's session was cut off while it ran (core §8.1, 0.38): zenoh
    /// answered it at the cut, so `complete` is false. Absent otherwise.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cut_off_during: bool,
    /// By namespace, sorted; the bus root (no namespace) sorts first.
    pub namespaces: Vec<NamespaceSighting>,
    /// Keys the selector matched that are not instance tokens, verbatim.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unparsed: Vec<String>,
}

/// One namespace and the services holding instance tokens in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NamespaceSighting {
    /// The key prefix before `zk2/`, as a session's `namespace` names it;
    /// empty for the bus-root deployment.
    pub namespace: String,
    /// `<system>/<service>` of every instance token, sorted, once each.
    pub services: Vec<String>,
    /// Instance tokens: more than the services when an address has
    /// several (a re-mint's overlap, a redundant pair, a split brain).
    pub instances: usize,
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

// ─── live presence (#614) ────────────────────────────────────────────────────

/// An address's presence, in the three states (spec §8.1, O5, R7):
/// `"up"`, `"down"`, `"unobservable"` on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressStatus {
    /// At least one instance token held, heard since the last loss.
    Up,
    /// No instance token held, with a complete read behind the claim: the
    /// last complete one showed none (core §8.1, 0.37).
    Down,
    /// Neither established: no seed, a seed possibly incomplete, events
    /// lost since the last complete one, a read not made through a session
    /// cut off, or a withdrawal heard live with no complete read since
    /// (`model::presence_live`'s doc).
    Unobservable,
}

impl AddressStatus {
    /// The status as a person reads it, and as it serializes: `up`,
    /// `down`, `unobservable`.
    pub fn as_str(self) -> &'static str {
        match self {
            AddressStatus::Up => "up",
            AddressStatus::Down => "down",
            AddressStatus::Unobservable => "unobservable",
        }
    }
}

impl fmt::Display for AddressStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How a transition was learned: `"event"`, `"seed"`, `"dropped"`,
/// `"withdrawn"`, `"cut_off"`, `"moved"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceSource {
    /// Heard on the subscription, as it happened.
    Event,
    /// Found by a seed, against what was held before it: it happened at or
    /// before the time the transition carries.
    Seed,
    /// The basis was lost to a `Dropped(n)`: what follows is unobservable.
    Dropped,
    /// A withdrawal heard live left the address with no instance token
    /// (core §8.1, 0.37): a status move to `unobservable`, never to `down`,
    /// until the read it owes decides it. Its token's own move is an
    /// `event`.
    Withdrawn,
    /// A read was not made: the reader's session reaches no other zenoh node
    /// (core §8.1, 0.37). What follows is unobservable until a complete
    /// read.
    CutOff,
    /// The reader's watch session's transport moved (core §8.1, 0.42,
    /// F-229): its subscribers may not have heard what moved while it was
    /// down. What follows is unobservable until a complete read.
    Moved,
}

/// One member of an interface's `epoch` template (§8.1): the interface and
/// the member's chunk. Its epoch is not part of its identity: a new epoch
/// is the same member, its continuity broken.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct MemberId {
    pub iface: IfaceId,
    /// The slugged value of the template's `epoch` parameter, as the key
    /// carries it.
    pub member: String,
}

/// One move of one address's presence, tagged by `kind` on the wire
/// (`{"kind": "status", "from": "up", "to": "down"}`); instance ids,
/// epochs and fingerprint prefixes are their 16 hex digits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PresenceChange {
    /// The address's status moved, in the three states. `to: Down` is
    /// presence's word, never a verdict about health.
    Status {
        from: AddressStatus,
        to: AddressStatus,
    },
    /// An instance token appeared (§8.1).
    InstanceAppeared {
        #[serde(serialize_with = "display")]
        instance: InstanceId,
    },
    /// An instance token went.
    InstanceGone {
        #[serde(serialize_with = "display")]
        instance: InstanceId,
    },
    /// An interface token appeared.
    InterfaceAppeared {
        #[serde(serialize_with = "display")]
        instance: InstanceId,
        iface: IfaceId,
        #[serde(serialize_with = "display")]
        fp: Fp16,
    },
    /// An interface token went.
    InterfaceGone {
        #[serde(serialize_with = "display")]
        instance: InstanceId,
        iface: IfaceId,
        #[serde(serialize_with = "display")]
        fp: Fp16,
    },
    /// A member's first token appeared: the member exists from here (§8.1).
    MemberAppeared {
        member: MemberId,
        #[serde(serialize_with = "display")]
        epoch: InstanceId,
    },
    /// A member's current epoch moved while it held a token throughout:
    /// make-before-break, never a gap (§8.1).
    MemberHandover {
        member: MemberId,
        #[serde(serialize_with = "display")]
        from: InstanceId,
        #[serde(serialize_with = "display")]
        to: InstanceId,
    },
    /// A member's last token went.
    MemberGone {
        member: MemberId,
        #[serde(serialize_with = "display")]
        epoch: InstanceId,
    },
}

/// One transition: what moved, where, when it was folded and how it was
/// learned. `at` serializes as RFC 3339, to the second.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PresenceTransition {
    /// The time the caller folded it with.
    #[serde(serialize_with = "rfc3339")]
    pub at: SystemTime,
    pub source: PresenceSource,
    pub address: Addr,
    pub change: PresenceChange,
}

/// A grammar id as its key spelling: the hex ids carry no serde of their
/// own, and their text form is the one a key, a log line and a script share.
fn display<T: fmt::Display, S: Serializer>(v: &T, s: S) -> Result<S::Ok, S::Error> {
    s.collect_str(v)
}

/// A fold time as RFC 3339, through the one formatter the engine writes
/// every timestamp with.
fn rfc3339<S: Serializer>(at: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
    let secs = at
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    s.serialize_str(&crate::tape::record::rfc3339_from_unix(secs))
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
            cut_off_during: false,
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
        // A read cut off while it ran says so beside `complete`, and only
        // then (core §8.1, 0.38).
        let cut = ServiceListing {
            cut_off_during: true,
            services: vec![],
            unparsed: vec![],
            ..listing
        };
        assert_eq!(
            serde_json::to_value(&cut).expect("serialize"),
            json!({
                "selector": "zk2/*/*/@zk/**",
                "complete": false,
                "cut_off_during": true,
                "services": [],
            })
        );
    }

    /// The documents `service show` and `namespace list` print: a service
    /// nobody holds a token for is an empty instance list beside the
    /// completeness flag, and the bus root's namespace is the empty string.
    #[test]
    fn service_view_and_namespace_listing_json_shapes_are_pinned() {
        let view = ServiceView {
            address: "host-a/tc".into(),
            selector: "zk2/host-a/tc/@zk/**".into(),
            complete: false,
            cut_off_during: false,
            instances: vec![],
            members: vec![],
            unparsed: vec![],
        };
        assert_eq!(
            serde_json::to_value(&view).expect("serialize"),
            json!({
                "address": "host-a/tc",
                "selector": "zk2/host-a/tc/@zk/**",
                "complete": false,
                "instances": [],
            })
        );
        let listing = NamespaceListing {
            selector: "**/zk2/*/*/@zk/instance/*".into(),
            complete: true,
            cut_off_during: false,
            namespaces: vec![
                NamespaceSighting {
                    namespace: String::new(),
                    services: vec!["host-a/tc".into()],
                    instances: 1,
                },
                NamespaceSighting {
                    namespace: "site/prod".into(),
                    services: vec!["host-a/tc".into(), "host-b/tc".into()],
                    instances: 3,
                },
            ],
            unparsed: vec!["x/zk2/a/b/@zk/instance/nope".into()],
        };
        assert_eq!(
            serde_json::to_value(&listing).expect("serialize"),
            json!({
                "selector": "**/zk2/*/*/@zk/instance/*",
                "complete": true,
                "namespaces": [
                    {"namespace": "", "services": ["host-a/tc"], "instances": 1},
                    {"namespace": "site/prod", "services": ["host-a/tc", "host-b/tc"], "instances": 3},
                ],
                "unparsed": ["x/zk2/a/b/@zk/instance/nope"],
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

    /// The live presence shapes zenwatch's notifications carry (#614, FL2):
    /// the three states and the three sources in their wire words, every
    /// change tagged by `kind`, hex ids as their key spelling, and the fold
    /// time as RFC 3339.
    #[test]
    fn presence_transition_json_shape_is_pinned() {
        use std::time::Duration;
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_791_331_200);
        let addr: Addr = "host-a/tc".parse().expect("an address");
        let iface: IfaceId = "tc.netif.v1".parse().expect("an interface");
        let i1 = InstanceId::from_u64(0x8f3a_5c2e_9b1d_4f70);
        let i2 = InstanceId::from_u64(2);
        let fp = Fp16::new("abababababababab").expect("a prefix");
        let member = MemberId {
            iface: iface.clone(),
            member: "eth0".into(),
        };
        let t = PresenceTransition {
            at,
            source: PresenceSource::Event,
            address: addr,
            change: PresenceChange::Status {
                from: AddressStatus::Up,
                to: AddressStatus::Down,
            },
        };
        assert_eq!(
            serde_json::to_value(&t).expect("serialize"),
            json!({
                "at": "2026-10-07T00:00:00Z",
                "source": "event",
                "address": "host-a/tc",
                "change": {"kind": "status", "from": "up", "to": "down"},
            })
        );
        let tagged = |kind: &str, rest: serde_json::Value| {
            let mut v = json!({ "kind": kind });
            v.as_object_mut()
                .expect("an object")
                .extend(rest.as_object().expect("an object").clone());
            v
        };
        let member_wire = json!({"iface": "tc.netif.v1", "member": "eth0"});
        for (change, wire) in [
            (
                PresenceChange::InstanceAppeared {
                    instance: i1.clone(),
                },
                tagged("instance_appeared", json!({"instance": "8f3a5c2e9b1d4f70"})),
            ),
            (
                PresenceChange::InstanceGone {
                    instance: i1.clone(),
                },
                tagged("instance_gone", json!({"instance": "8f3a5c2e9b1d4f70"})),
            ),
            (
                PresenceChange::InterfaceAppeared {
                    instance: i1.clone(),
                    iface: iface.clone(),
                    fp: fp.clone(),
                },
                tagged(
                    "interface_appeared",
                    json!({"instance": "8f3a5c2e9b1d4f70", "iface": "tc.netif.v1",
                           "fp": "abababababababab"}),
                ),
            ),
            (
                PresenceChange::InterfaceGone {
                    instance: i1.clone(),
                    iface,
                    fp,
                },
                tagged(
                    "interface_gone",
                    json!({"instance": "8f3a5c2e9b1d4f70", "iface": "tc.netif.v1",
                           "fp": "abababababababab"}),
                ),
            ),
            (
                PresenceChange::MemberAppeared {
                    member: member.clone(),
                    epoch: i2.clone(),
                },
                tagged(
                    "member_appeared",
                    json!({"member": member_wire.clone(), "epoch": "0000000000000002"}),
                ),
            ),
            (
                PresenceChange::MemberHandover {
                    member: member.clone(),
                    from: i2.clone(),
                    to: i1,
                },
                tagged(
                    "member_handover",
                    json!({"member": member_wire.clone(), "from": "0000000000000002",
                           "to": "8f3a5c2e9b1d4f70"}),
                ),
            ),
            (
                PresenceChange::MemberGone { member, epoch: i2 },
                tagged(
                    "member_gone",
                    json!({"member": member_wire, "epoch": "0000000000000002"}),
                ),
            ),
        ] {
            assert_eq!(serde_json::to_value(&change).expect("serialize"), wire);
        }
        for (status, word) in [
            (AddressStatus::Up, "up"),
            (AddressStatus::Down, "down"),
            (AddressStatus::Unobservable, "unobservable"),
        ] {
            assert_eq!(serde_json::to_value(status).expect("serialize"), word);
            assert_eq!(status.as_str(), word);
        }
        for (source, word) in [
            (PresenceSource::Event, "event"),
            (PresenceSource::Seed, "seed"),
            (PresenceSource::Dropped, "dropped"),
            (PresenceSource::Withdrawn, "withdrawn"),
            (PresenceSource::CutOff, "cut_off"),
            (PresenceSource::Moved, "moved"),
        ] {
            assert_eq!(serde_json::to_value(source).expect("serialize"), word);
        }
    }
}
