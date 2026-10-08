//! A zk2 sample, rendered for a tool that was never compiled against its
//! contract (spec §7.2): `watch` and resolved `get`.
//!
//! The decode order is the sample's `Encoding`, then the contract's type,
//! then sniffing, and the rendering is honest at every step: a value decoded
//! field by field, a raw type as its media type and size, bytes that do not
//! decode as their declared type as that type, the size and the reason —
//! and, when no contract is in hand, the structural ladder *with the reason
//! it was reached*. Never garbage, never nothing, and never a sniffed
//! document that looks like a decoded one.

use std::collections::BTreeMap;

use serde::Serialize;

/// One sample's rendering.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PayloadRendering {
    /// The base-relative key the sample arrived on.
    pub key: String,
    /// The payload's size in bytes, whatever it rendered as.
    pub size: usize,
    /// Which contract resource the key resolved to; absent when it did not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<ResolvedResource>,
    /// What the payload rendered as.
    #[serde(flatten)]
    pub rendered: Rendered,
}

/// The contract resource a key resolved to (D1: most literal first).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedResource {
    /// `<name>.v<major>`.
    pub iface: String,
    /// The revision decoded against, `sha256:…`.
    pub fingerprint: String,
    /// `<kind token>/<template>`.
    pub resource: String,
    /// Which member of the resource the payload is: `type`, `attachment`,
    /// `request`, `response`, `error` or `summary`.
    pub member: String,
    /// The template's parameters, unslugged.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, Vec<String>>,
}

/// What a payload rendered as. Tagged `as`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "as", rename_all = "snake_case")]
pub enum Rendered {
    /// Decoded through the contract's type, field by field, as JSON.
    Value {
        /// The declared type in the authoring spelling: the message name
        /// (`nav.v2.Pose`), `json:<name>`, or the raw media type.
        declared: String,
        value: serde_json::Value,
    },
    /// A raw type: its media type (for a family like `video/*`, the
    /// concrete one the sample's `Encoding` carries). The bytes are not
    /// shown: the contract says they are not this tool's to read.
    Opaque { media_type: String },
    /// Bytes that do not decode as their declared type.
    Undecodable { declared: String, reason: String },
    /// No contract type applied, so the structural ladder (JSON, CBOR,
    /// text, else a byte count) — and why it was reached.
    Structural {
        why: Unresolved,
        /// The structural sniff's document, when the bytes carry one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<serde_json::Value>,
        /// The structural rendering: the document, the text, or
        /// `<N bytes>`.
        text: String,
    },
}

/// Why a sample was not decoded through a contract. Tagged `reason`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Unresolved {
    /// The key is not a zk2 data key (the bus is shared, §1.7).
    NotZk2 { detail: String },
    /// Presence shows no instance of this address providing the interface.
    NoProvider,
    /// A provider is present, and no descriptor named its revision: a
    /// token's fingerprint prefix is not enough to retrieve a bundle.
    NoRevision,
    /// The provider's instances name more than one revision, and a data key
    /// carries no instance: decoding with either could mislead.
    Ambiguous { fingerprints: Vec<String> },
    /// The revision was not looked for, or not yet.
    ContractNotHeld { fingerprint: String },
    /// Retrieval found no valid bundle (§8.4).
    ContractUnavailable {
        fingerprint: String,
        refused: Vec<String>,
    },
    /// A verified bundle whose contract this build cannot read.
    ContractUnreadable { fingerprint: String, detail: String },
    /// The contract declares no resource whose template matches the key.
    NoResource { fingerprint: String },
    /// The resource has no such member (an attachment it does not declare,
    /// a `request` asked of a stream).
    NoMember { resource: String, member: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The decoded pole: the resolution beside the value, the rendering's
    /// tag flattened next to the key.
    #[test]
    fn decoded_payload_json_shape_is_pinned() {
        let r = PayloadRendering {
            key: "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0".into(),
            size: 22,
            resource: Some(ResolvedResource {
                iface: "tc.netif.v1".into(),
                fingerprint: format!("sha256:{}", "ab".repeat(32)),
                resource: "stream/bandwidth/{ns}/{iface}".into(),
                member: "type".into(),
                values: [
                    ("iface".to_owned(), vec!["eth0".to_owned()]),
                    ("ns".to_owned(), vec!["default".to_owned()]),
                ]
                .into(),
            }),
            rendered: Rendered::Value {
                declared: "json:BandwidthUpdate".into(),
                value: json!({"rx_bps": 1, "tx_bps": 2}),
            },
        };
        assert_eq!(
            serde_json::to_value(&r).expect("serialize"),
            json!({
                "key": "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0",
                "size": 22,
                "resource": {
                    "iface": "tc.netif.v1",
                    "fingerprint": format!("sha256:{}", "ab".repeat(32)),
                    "resource": "stream/bandwidth/{ns}/{iface}",
                    "member": "type",
                    "values": {"iface": ["eth0"], "ns": ["default"]},
                },
                "as": "value",
                "declared": "json:BandwidthUpdate",
                "value": {"rx_bps": 1, "tx_bps": 2},
            })
        );
    }

    /// The honest poles: opaque and undecodable say what was declared, and
    /// the structural fallback says why it was reached.
    #[test]
    fn honest_renderings_json_shape_is_pinned() {
        let opaque = Rendered::Opaque {
            media_type: "image/jpeg".into(),
        };
        assert_eq!(
            serde_json::to_value(&opaque).expect("serialize"),
            json!({"as": "opaque", "media_type": "image/jpeg"})
        );
        let bad = Rendered::Undecodable {
            declared: "nav.v2.Pose".into(),
            reason: "buffer underflow".into(),
        };
        assert_eq!(
            serde_json::to_value(&bad).expect("serialize"),
            json!({"as": "undecodable", "declared": "nav.v2.Pose", "reason": "buffer underflow"})
        );
        let r = PayloadRendering {
            key: "zk2/host-a/tc/tc.netif.v1/state/namespaces".into(),
            size: 2,
            resource: None,
            rendered: Rendered::Structural {
                why: Unresolved::ContractUnavailable {
                    fingerprint: format!("sha256:{}", "ab".repeat(32)),
                    refused: vec!["fingerprint".into()],
                },
                value: Some(json!([])),
                text: "[]".into(),
            },
        };
        assert_eq!(
            serde_json::to_value(&r).expect("serialize"),
            json!({
                "key": "zk2/host-a/tc/tc.netif.v1/state/namespaces",
                "size": 2,
                "as": "structural",
                "why": {
                    "reason": "contract_unavailable",
                    "fingerprint": format!("sha256:{}", "ab".repeat(32)),
                    "refused": ["fingerprint"],
                },
                "value": [],
                "text": "[]",
            })
        );
        assert_eq!(
            serde_json::to_value(Unresolved::NotZk2 {
                detail: "it does not start with zk2/".into()
            })
            .expect("serialize"),
            json!({"reason": "not_zk2", "detail": "it does not start with zk2/"})
        );
    }
}
