//! One zk2 interface across a deployment (spec §8.1, §3.3, §8.4): who
//! provides it at which revision, who requires it, and what each revision's
//! contract says — `iface show`.
//!
//! v1's `interface` domain is a *type* seen across producers
//! (v1's `interface show`, retired at FJ4); a zk2 interface is a contract, named
//! `<name>.v<major>`, and this is its view. Providers come from interface
//! tokens and, for the tokenless set (U22), from descriptors only, so an
//! instance whose descriptor did not answer is listed under `undescribed`:
//! it may provide the interface without a token, and nothing here can say.

use std::collections::BTreeMap;

use serde::Serialize;
use zenkey_model::descriptor::Unavailable;

use crate::report::{Asked, ContractAnswer};

/// `iface list`: every interface a presence read names — provided by a
/// token or a descriptor, or required by a role (R3).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IfaceListing {
    /// The liveliness selector read, base-relative.
    pub selector: String,
    /// Whether the presence read completed before its timeout (§8.1).
    pub complete: bool,
    /// By interface, sorted.
    pub interfaces: Vec<IfaceSummary>,
    /// Instances whose descriptor did not read: a tokenless interface they
    /// provide, and every role they declare, is missing here (§8.1).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub undescribed: Vec<InstanceRef>,
}

/// One interface across the deployment, summarised.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IfaceSummary {
    /// `<name>.v<major>`.
    pub iface: String,
    /// Service addresses providing it, by token or descriptor, sorted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<String>,
    /// Service addresses with a role bound to it, sorted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consumers: Vec<String>,
    /// The fingerprints providers' descriptors name, sorted. More than one
    /// is a rolling upgrade or a split brain.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revisions: Vec<String>,
    /// Some provider serves it in the tokenless set (U22).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tokenless: bool,
}

/// `iface show <iface>`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IfaceView {
    /// `<name>.v<major>`.
    pub iface: String,
    /// Whether the presence read completed before its timeout (§8.1).
    pub complete: bool,
    /// By address, then instance.
    pub providers: Vec<IfaceProvider>,
    /// Every role bound to this interface, from the descriptors (R3).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consumers: Vec<IfaceConsumer>,
    /// Every revision a provider's descriptor names, by fingerprint.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revisions: Vec<IfaceRevision>,
    /// Instances whose descriptor was asked for and did not read: a
    /// tokenless provider among them is invisible here (§8.1).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub undescribed: Vec<InstanceRef>,
}

/// One instance providing the interface.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IfaceProvider {
    /// `<system>/<service>`.
    pub address: String,
    pub instance: String,
    /// The fingerprint prefix the interface token carries; absent without a
    /// token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// The full fingerprint the descriptor names; absent without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minor: Option<u64>,
    /// The descriptor marks the interface tokenless (U22).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tokenless: bool,
    /// Optional resources absent here although no missing capability
    /// implies it, as the descriptor lists them (§3.3).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unavailable: Vec<Unavailable>,
    /// Lowered population bounds, keyed `<kind token>/<template>`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub cardinality: BTreeMap<String, u64>,
    /// The resources this instance exposes by the compact rule (r3.3 D8):
    /// the contract's, minus optional ones gated on a capability it does not
    /// hold, minus `unavailable`. Asked only when the descriptor and its
    /// revision are both in hand.
    #[serde(default, skip_serializing_if = "Asked::is_not_asked")]
    pub exposes: Asked<Vec<String>>,
}

/// One role bound to the interface (R3), as a descriptor declares it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IfaceConsumer {
    /// The consumer's `<system>/<service>`.
    pub address: String,
    pub instance: String,
    pub role: String,
    /// Service addresses, `<system>/<service>`, either chunk may be `*`.
    pub bindings: Vec<String>,
    /// Template parameters bound by the binding (R2).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
    /// The interface whose contract declares the role; absent when the
    /// component's own manifest does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_by: Option<String>,
}

/// One revision of the interface, and what asking for its contract found.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IfaceRevision {
    /// `sha256:…`.
    pub fingerprint: String,
    /// The providers whose descriptor names it, as `<address>@<instance>`.
    pub providers: Vec<String>,
    /// Absent when the contract was never looked for.
    #[serde(default, skip_serializing_if = "Asked::is_not_asked")]
    pub contract: Asked<ContractAnswer>,
}

/// An instance, and why it is listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstanceRef {
    pub address: String,
    pub instance: String,
    /// The descriptor answer's tag: `silent`, `invalid` or `failed`.
    pub descriptor: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use zenkey_model::descriptor::Cause;

    /// The document `iface list --format json` prints: an interface only
    /// required has consumers and nothing else.
    #[test]
    fn iface_listing_json_shape_is_pinned() {
        let listing = IfaceListing {
            selector: "zk2/*/*/@zk/**".into(),
            complete: true,
            interfaces: vec![
                IfaceSummary {
                    iface: "health.v1".into(),
                    providers: vec!["vehicle-01/cam-front".into()],
                    consumers: vec![],
                    revisions: vec![format!("sha256:{}", "cd".repeat(32))],
                    tokenless: true,
                },
                IfaceSummary {
                    iface: "tc.scenario.v1".into(),
                    providers: vec![],
                    consumers: vec!["ws-01/tcgui-frontend".into()],
                    revisions: vec![],
                    tokenless: false,
                },
            ],
            undescribed: vec![],
        };
        assert_eq!(
            serde_json::to_value(&listing).expect("serialize"),
            json!({
                "selector": "zk2/*/*/@zk/**",
                "complete": true,
                "interfaces": [
                    {
                        "iface": "health.v1",
                        "providers": ["vehicle-01/cam-front"],
                        "revisions": [format!("sha256:{}", "cd".repeat(32))],
                        "tokenless": true,
                    },
                    {"iface": "tc.scenario.v1", "consumers": ["ws-01/tcgui-frontend"]},
                ],
            })
        );
    }

    /// The document `iface show --format json` prints. A provider known by
    /// its token alone carries only the token; one known by its descriptor
    /// carries the fingerprint, the exceptions and (when the revision was in
    /// hand) its exposure.
    #[test]
    fn iface_view_json_shape_is_pinned() {
        let fp = format!("sha256:{}", "ab".repeat(32));
        let view = IfaceView {
            iface: "tc.netif.v1".into(),
            complete: true,
            providers: vec![
                IfaceProvider {
                    address: "host-a/tc".into(),
                    instance: "8f3a5c2e9b1d4f70".into(),
                    token: Some("abababababababab".into()),
                    contract: Some(fp.clone()),
                    minor: Some(0),
                    tokenless: false,
                    unavailable: vec![Unavailable {
                        resource: "@op/diagnostics".into(),
                        cause: Cause::Config,
                        reason: Some("disabled here".into()),
                    }],
                    cardinality: [("state/interfaces/{ns}/{iface}".to_owned(), 8)].into(),
                    exposes: Asked::Asked(vec!["state/namespaces".into()]),
                },
                IfaceProvider {
                    address: "host-b/tc".into(),
                    instance: "0000000000000002".into(),
                    token: Some("abababababababab".into()),
                    contract: None,
                    minor: None,
                    tokenless: false,
                    unavailable: vec![],
                    cardinality: BTreeMap::new(),
                    exposes: Asked::NotAsked,
                },
            ],
            consumers: vec![IfaceConsumer {
                address: "ws-01/tcgui-frontend".into(),
                instance: "0000000000000003".into(),
                role: "netif".into(),
                bindings: vec!["*/tc".into()],
                params: BTreeMap::new(),
                declared_by: None,
            }],
            revisions: vec![IfaceRevision {
                fingerprint: fp.clone(),
                providers: vec!["host-a/tc@8f3a5c2e9b1d4f70".into()],
                contract: Asked::Asked(ContractAnswer::Unavailable { refused: vec![] }),
            }],
            undescribed: vec![InstanceRef {
                address: "host-c/tc".into(),
                instance: "0000000000000004".into(),
                descriptor: "silent".into(),
            }],
        };
        assert_eq!(
            serde_json::to_value(&view).expect("serialize"),
            json!({
                "iface": "tc.netif.v1",
                "complete": true,
                "providers": [
                    {
                        "address": "host-a/tc",
                        "instance": "8f3a5c2e9b1d4f70",
                        "token": "abababababababab",
                        "contract": fp,
                        "minor": 0,
                        "unavailable": [{"resource": "@op/diagnostics", "cause": "config", "reason": "disabled here"}],
                        "cardinality": {"state/interfaces/{ns}/{iface}": 8},
                        "exposes": ["state/namespaces"],
                    },
                    {"address": "host-b/tc", "instance": "0000000000000002", "token": "abababababababab"},
                ],
                "consumers": [{
                    "address": "ws-01/tcgui-frontend",
                    "instance": "0000000000000003",
                    "role": "netif",
                    "bindings": ["*/tc"],
                }],
                "revisions": [{
                    "fingerprint": fp,
                    "providers": ["host-a/tc@8f3a5c2e9b1d4f70"],
                    "contract": {"answer": "unavailable", "refused": []},
                }],
                "undescribed": [{"address": "host-c/tc", "instance": "0000000000000004", "descriptor": "silent"}],
            })
        );
    }
}
