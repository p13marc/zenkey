//! What a raw observer makes of a wire key and its payload (#612, FJ8b):
//! `echo`, `rate`, `field`, `timeline`, `snapshot`, the watchdog's
//! conditions and `check expect` all read the bus un-namespaced, and every
//! one of them says how far each key resolved (the tooling guide's O2).
//!
//! [`KeyIdentity`] is that answer for one key: the rung the ladder reached,
//! spelled so a script branches on it, never on prose. [`KeyGroup`] is the
//! same answer without the member values — what a lane, a rate group or a
//! snapshot diff keys by. [`Conformance`] is a payload's verdict against its
//! declared type (§7.2, §7.3), four poles kept apart, and [`QosMismatch`]
//! compares the QoS a sample rode with the one its contract declares
//! (§2.4). [`LensScope`] is what the observer had to resolve with: a
//! namespace, a presence read or none, and the revisions in hand.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::payload::Unresolved;

/// What a wire key is, as far as resolution got (O2's rungs).
///
/// A key is never refused (O1): a key outside the namespace, or no zk2 key
/// at all, still has an identity — the rung it stopped at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyIdentity {
    /// Which group the key falls in. Tagged `is`.
    #[serde(flatten)]
    pub group: KeyGroup,
    /// The resource template's values on the key, unslugged: present only
    /// when a contract resolved the resource (rung 6).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, Vec<String>>,
    /// Where the ladder stopped, when it stopped short of the resource:
    /// absent on a key resolved through its contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unresolved: Option<Unresolved>,
}

impl KeyIdentity {
    /// The address the key names, when it names one.
    pub fn address(&self) -> Option<&str> {
        self.group.address()
    }
}

/// A key's group: one resource of one address, an address's control key,
/// or the two ways a key is not this deployment's zk2 data. Tagged `is`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "is", rename_all = "snake_case")]
pub enum KeyGroup {
    /// A zk2 data key in the namespace (rungs 1–2), and its resource when a
    /// contract resolved it (rung 6).
    Resource {
        /// `<system>/<service>`.
        address: String,
        /// `<name>.v<major>`.
        iface: String,
        /// The kind token (`stream`, `@state`, …, §1.3).
        token: String,
        /// `<kind token>/<template>`, when a contract resolved it; absent
        /// when the contract was not in hand, with the reason on the
        /// identity.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resource: Option<String>,
    },
    /// A zk2 control key (`@zk`): an instance, a token, a descriptor put,
    /// or a contract bundle, which names no address.
    Control {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        address: Option<String>,
        /// `instance`, `alive`, `member` or `contract`.
        form: String,
    },
    /// Not under the stated namespace (rung 1). Not guessed at (O3).
    NotInNamespace,
    /// In the namespace, and not a zk2 key (rung 2): the bus is shared.
    NotZk2,
}

impl KeyGroup {
    /// The address the group names, when it names one.
    pub fn address(&self) -> Option<&str> {
        match self {
            KeyGroup::Resource { address, .. } => Some(address),
            KeyGroup::Control { address, .. } => address.as_deref(),
            KeyGroup::NotInNamespace | KeyGroup::NotZk2 => None,
        }
    }

    /// The group as a label a person reads.
    pub fn label(&self) -> String {
        match self {
            KeyGroup::Resource {
                address,
                iface,
                resource: Some(r),
                ..
            } => format!("{address} {iface} {r}"),
            KeyGroup::Resource {
                address,
                iface,
                token,
                resource: None,
            } => format!("{address} {iface} {token}/… (contract not in hand)"),
            KeyGroup::Control {
                address: Some(a),
                form,
            } => format!("{a} @zk/{form}"),
            KeyGroup::Control {
                address: None,
                form,
            } => format!("@zk/{form}"),
            KeyGroup::NotInNamespace => "not in this namespace".to_owned(),
            KeyGroup::NotZk2 => "not a zk2 key".to_owned(),
        }
    }
}

/// A payload against its declared type (§7.2, §7.3). Tagged `state`.
///
/// Four poles, never folded: a value that validates, one that does not, bytes
/// that do not decode as their type at all, and a payload that could not be
/// checked, with why — "not checked" is never "valid" (O4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Conformance {
    /// Decoded as its declared type; for a JSON Schema type, the value also
    /// satisfied it (`zenkey_model::validate`).
    Valid,
    /// Decoded, and the value fails its JSON Schema type: one line per
    /// violation, its JSON pointer first.
    Invalid { violations: Vec<String> },
    /// The bytes do not decode as the declared type.
    Undecodable { declared: String, reason: String },
    /// Not checked: no contract resolved the key, a raw type declares no
    /// structure, or the sample was a deletion.
    NotChecked { reason: String },
}

impl Conformance {
    /// Whether the payload is a finding for an `invalid-payload` question:
    /// it was checked, and it failed.
    pub fn is_violation(&self) -> bool {
        matches!(
            self,
            Conformance::Invalid { .. } | Conformance::Undecodable { .. }
        )
    }

    /// Whether the payload was checked at all.
    pub fn is_checked(&self) -> bool {
        !matches!(self, Conformance::NotChecked { .. })
    }

    /// One token a row carries: `valid`, `invalid`, `undecodable`, or
    /// `not-checked: <reason>`.
    pub fn token(&self) -> String {
        match self {
            Conformance::Valid => "valid".to_owned(),
            Conformance::Invalid { .. } => "invalid".to_owned(),
            Conformance::Undecodable { .. } => "undecodable".to_owned(),
            Conformance::NotChecked { reason } => format!("not-checked: {reason}"),
        }
    }
}

/// The QoS axes a contract declares for a resource (§2.4), as the contract
/// spells them, and as a sample rode them.
///
/// Three axes: priority, congestion control and express travel with the
/// sample. Reliability is the link's, as received, and is not compared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QosAxes {
    /// `real_time` … `background`.
    pub priority: String,
    /// `drop` or `block`.
    pub congestion: String,
    pub express: bool,
}

/// A sample that did not ride its resource's declared QoS (§2.4: an owner
/// MUST publish with it): the declared axes, the observed ones, and which
/// differ.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QosMismatch {
    pub declared: QosAxes,
    pub observed: QosAxes,
    /// The axes that differ: `priority`, `congestion`, `express`.
    pub differs: Vec<String>,
}

impl QosMismatch {
    /// One line: `priority declared data, observed real_time`.
    pub fn summary(&self) -> String {
        self.differs
            .iter()
            .map(|axis| {
                let (d, o) = match axis.as_str() {
                    "priority" => (
                        self.declared.priority.clone(),
                        self.observed.priority.clone(),
                    ),
                    "congestion" => (
                        self.declared.congestion.clone(),
                        self.observed.congestion.clone(),
                    ),
                    _ => (
                        self.declared.express.to_string(),
                        self.observed.express.to_string(),
                    ),
                };
                format!("{axis} declared {d}, observed {o}")
            })
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// One payload checked against one type of a contract, offline (#612,
/// FJ8b): `zenctl check schema`. The bytes are decoded as the declared type
/// through the bundle — JSON or CBOR for a JSON Schema type, then
/// validated against it (§7.3); protobuf through the bundle's descriptor
/// set — and the conformance is the verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PayloadCheck {
    /// `<name>.v<major>`.
    pub iface: String,
    /// The revision checked against, `sha256:…`.
    pub fingerprint: String,
    /// `<kind token>/<template>`.
    pub resource: String,
    /// Which member of the resource: `type`, `attachment`, `request`,
    /// `response`, `error` or `summary`.
    pub member: String,
    /// The declared type, in the authoring spelling.
    pub declared: String,
    /// The encoding the bytes were read as: the one given, else the
    /// contract's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    /// The payload's size in bytes.
    pub size: usize,
    /// Valid, invalid with its violations, or undecodable with why.
    pub conformance: Conformance,
    /// The decoded value, when the bytes decoded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
}

/// What a raw observer had to resolve keys with: the namespace it stripped,
/// the presence read it attributed addresses with, and the revisions it
/// decoded through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LensScope {
    /// The namespace stated (`--namespace`); empty is the bus root.
    pub namespace: String,
    /// The presence read resolution used; absent when none was made, and
    /// then no key resolves past its address (O4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence: Option<LensPresence>,
    /// Revisions in hand to decode with.
    pub contracts: usize,
}

/// The presence read behind a [`LensScope`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LensPresence {
    /// The liveliness selector, base-relative.
    pub selector: String,
    /// `false` when the read ended at its timeout: a provider it did not
    /// see may be there (§8.1).
    pub complete: bool,
    /// Services seen.
    pub services: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A resolved key carries its group, its values and no reason; an
    /// unresolved one carries the rung it stopped at, and no values.
    #[test]
    fn identity_json_shape_is_pinned() {
        let resolved = KeyIdentity {
            group: KeyGroup::Resource {
                address: "host-a/tc".into(),
                iface: "tc.netif.v1".into(),
                token: "state".into(),
                resource: Some("state/interfaces/{ns}/{iface}".into()),
            },
            values: [("iface".to_owned(), vec!["eth0".to_owned()])].into(),
            unresolved: None,
        };
        let v = serde_json::to_value(&resolved).unwrap();
        assert_eq!(
            v,
            json!({
                "is": "resource",
                "address": "host-a/tc",
                "iface": "tc.netif.v1",
                "token": "state",
                "resource": "state/interfaces/{ns}/{iface}",
                "values": {"iface": ["eth0"]},
            })
        );
        assert_eq!(serde_json::from_value::<KeyIdentity>(v).unwrap(), resolved);

        let short = KeyIdentity {
            group: KeyGroup::Resource {
                address: "host-a/tc".into(),
                iface: "tc.netif.v1".into(),
                token: "state".into(),
                resource: None,
            },
            values: BTreeMap::new(),
            unresolved: Some(Unresolved::NoProvider),
        };
        let v = serde_json::to_value(&short).unwrap();
        assert_eq!(v["unresolved"], json!({"reason": "no_provider"}));
        assert!(v.get("resource").is_none() && v.get("values").is_none());
        assert_eq!(serde_json::from_value::<KeyIdentity>(v).unwrap(), short);

        let foreign = KeyIdentity {
            group: KeyGroup::NotInNamespace,
            values: BTreeMap::new(),
            unresolved: Some(Unresolved::NotInNamespace {
                namespace: "prod".into(),
            }),
        };
        assert_eq!(
            serde_json::to_value(&foreign).unwrap(),
            json!({
                "is": "not_in_namespace",
                "unresolved": {"reason": "not_in_namespace", "namespace": "prod"},
            })
        );
    }

    /// The four poles of a payload's conformance, tagged `state`.
    #[test]
    fn conformance_json_shape_is_pinned() {
        assert_eq!(
            serde_json::to_value(Conformance::Valid).unwrap(),
            json!({"state": "valid"})
        );
        assert_eq!(
            serde_json::to_value(Conformance::Invalid {
                violations: vec!["/rx_bps: expected integer".into()]
            })
            .unwrap(),
            json!({"state": "invalid", "violations": ["/rx_bps: expected integer"]})
        );
        assert_eq!(
            serde_json::to_value(Conformance::Undecodable {
                declared: "json:Status".into(),
                reason: "as json: EOF".into()
            })
            .unwrap(),
            json!({"state": "undecodable", "declared": "json:Status", "reason": "as json: EOF"})
        );
        let not = Conformance::NotChecked {
            reason: "a raw type".into(),
        };
        assert_eq!(
            serde_json::to_value(&not).unwrap(),
            json!({"state": "not_checked", "reason": "a raw type"})
        );
        assert!(!not.is_checked() && !not.is_violation());
        assert!(
            Conformance::Undecodable {
                declared: String::new(),
                reason: String::new()
            }
            .is_violation()
        );
    }

    /// A mismatch carries both sides whole, and names what differs.
    #[test]
    fn qos_mismatch_json_shape_is_pinned() {
        let m = QosMismatch {
            declared: QosAxes {
                priority: "data".into(),
                congestion: "drop".into(),
                express: false,
            },
            observed: QosAxes {
                priority: "real_time".into(),
                congestion: "drop".into(),
                express: true,
            },
            differs: vec!["priority".into(), "express".into()],
        };
        assert_eq!(
            serde_json::to_value(&m).unwrap(),
            json!({
                "declared": {"priority": "data", "congestion": "drop", "express": false},
                "observed": {"priority": "real_time", "congestion": "drop", "express": true},
                "differs": ["priority", "express"],
            })
        );
        assert_eq!(
            m.summary(),
            "priority declared data, observed real_time; express declared false, observed true"
        );
    }

    /// A checked payload names what it was checked against, and carries its
    /// value only when the bytes decoded.
    #[test]
    fn payload_check_json_shape_is_pinned() {
        let c = PayloadCheck {
            iface: "tc.netif.v1".into(),
            fingerprint: format!("sha256:{}", "ab".repeat(32)),
            resource: "stream/bandwidth/{ns}/{iface}".into(),
            member: "type".into(),
            declared: "json:BandwidthUpdate".into(),
            encoding: None,
            size: 9,
            conformance: Conformance::Undecodable {
                declared: "json:BandwidthUpdate".into(),
                reason: "as json: EOF".into(),
            },
            value: None,
        };
        assert_eq!(
            serde_json::to_value(&c).unwrap(),
            json!({
                "iface": "tc.netif.v1",
                "fingerprint": format!("sha256:{}", "ab".repeat(32)),
                "resource": "stream/bandwidth/{ns}/{iface}",
                "member": "type",
                "declared": "json:BandwidthUpdate",
                "size": 9,
                "conformance": {
                    "state": "undecodable",
                    "declared": "json:BandwidthUpdate",
                    "reason": "as json: EOF"
                },
            })
        );
    }

    /// A lens with no presence read says so by absence (O4).
    #[test]
    fn a_lens_without_presence_omits_it() {
        let s = LensScope {
            namespace: "prod".into(),
            presence: None,
            contracts: 2,
        };
        assert_eq!(
            serde_json::to_value(&s).unwrap(),
            json!({"namespace": "prod", "contracts": 2})
        );
    }
}
