//! A zk2 sample, rendered for a tool that was never compiled against its
//! contract (spec §7.2): `watch` and resolved `get`.
//!
//! The decode order: a payload whose declared type the tool holds is
//! decoded by that type, on the wire its sample's `Encoding` names, and any
//! other payload is sniffed (observer §7.3). The rendering is honest at
//! every step: a value decoded field by field, a raw type as its media type
//! and size, bytes that do not decode as their declared type as that type,
//! the size and the reason —
//! and, when no contract is in hand, the structural ladder *with the reason
//! it was reached*. Never garbage, never nothing, and never a sniffed
//! document that looks like a decoded one.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
        /// The deployment epoch, in seconds, a packed record's `time`
        /// fields were read under because the tool holds none (core §7.6,
        /// which carries it nowhere yet; #980): those values are right
        /// only where the deployment's epoch is this one. Absent for every
        /// other value.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        epoch_assumed: Option<i64>,
        /// A packed record's `time` fields dated, by field name (the
        /// tooling guide's O8): each a [`Dated`], beside the value, which
        /// keeps core §7.6's JSON reading. Absent where the record holds
        /// no `time` field, and for every other type.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dated: Option<serde_json::Value>,
        /// A packed record's bounded fields' bounds, by field name (O8):
        /// each a [`Bound`]. Absent where none is bounded.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bounds: Option<serde_json::Value>,
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

impl Rendered {
    /// What a person reads beside a value whose `time` fields rest on an
    /// epoch assumed (`epoch_assumed`): one spelling for every frontend.
    #[must_use]
    pub fn epoch_note(&self) -> Option<String> {
        match self {
            Rendered::Value {
                epoch_assumed: Some(e),
                ..
            } => Some(format!("epoch {e} assumed, #980")),
            _ => None,
        }
    }

    /// What a person reads beside a value of what it promises (the tooling
    /// guide's O8): a packed record's dated `time` fields and its bounds, a
    /// heartbeat's dated time. `None` where it promises nothing more.
    #[must_use]
    pub fn promise_note(&self) -> Option<String> {
        let Rendered::Value {
            value,
            dated,
            bounds,
            ..
        } = self
        else {
            return None;
        };
        let mut parts = Vec::new();
        if let Some(d) = value.get("dated").and_then(Dated::read) {
            parts.push(format!("time {}", d.words()));
        }
        for (name, d) in dated.iter().flat_map(|d| d.as_object()).flatten() {
            if let Some(d) = Dated::read(d) {
                parts.push(format!("{name} {}", d.words()));
            }
        }
        for (name, b) in bounds.iter().flat_map(|b| b.as_object()).flatten() {
            if let Ok(b) = serde_json::from_value::<Bound>(b.clone()) {
                parts.push(format!("{name} {}", b.words()));
            }
        }
        (!parts.is_empty()).then(|| parts.join("; "))
    }
}

/// A time's date, beside the wire's number (the tooling guide's O8): RFC
/// 3339 UTC at the wire's precision, and the epoch it counts from, named.
/// An assumed epoch and a refused one date nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dated {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub utc: Option<String>,
    pub epoch: DatedEpoch,
    /// The catalog the epoch is read from: the one the owner's descriptor
    /// names, or with no descriptor the one the tool holds. Absent for an
    /// owner that names none, and where nothing states the epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog: Option<String>,
}

/// Which epoch a [`Dated`] counts from: the fleet's `Epoch` words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatedEpoch {
    /// The owner's descriptor names a catalog the tool holds, or a
    /// successor of; or names none, and counts from 0.
    Stated,
    /// No descriptor in hand: the catalog the tool holds, the deployment's
    /// word.
    Deployment,
    /// Nothing states it, and the tool holds no catalog: 0, never dated.
    Assumed,
    /// The owner names a catalog the tool neither holds nor holds a
    /// successor of: never dated.
    UnknownCatalog,
    /// The owner's instances name two catalogs, or the catalogs held state
    /// two epochs: never dated.
    Ambiguous,
}

impl Dated {
    /// A `dated` member read back, `None` where it is no [`Dated`].
    pub fn read(v: &serde_json::Value) -> Option<Dated> {
        serde_json::from_value(v.clone()).ok()
    }

    /// A person's words for it: the date and whose catalog, or why none.
    pub fn words(&self) -> String {
        let catalog = self.catalog.as_deref();
        match (self.epoch, &self.utc) {
            (DatedEpoch::Stated, Some(utc)) => match catalog {
                Some(c) => format!("{utc}, catalog {c}"),
                None => format!("{utc}, the owner's epoch"),
            },
            (DatedEpoch::Deployment, Some(utc)) => match catalog {
                Some(c) => format!("{utc}, the deployment's catalog {c}"),
                None => format!("{utc}, the deployment's epoch"),
            },
            (DatedEpoch::UnknownCatalog, _) => format!(
                "undated: catalog {} is not held (unknown_catalog)",
                catalog.unwrap_or("?")
            ),
            (DatedEpoch::Ambiguous, _) => "undated: two catalogs disagree".to_owned(),
            (_, _) => "undated: the epoch is assumed".to_owned(),
        }
    }
}

/// What a decoded value's declaration bounds it by (core §7.6; the tooling
/// guide's O8): a `fixed` field reads back within half its resolution, a
/// `counter` modulo 2^width.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bound {
    /// ±resolution/2, an exact decimal.
    HalfResolution(String),
    /// The count modulo 2^width: a fall may be a wrap.
    Width(u32),
}

impl Bound {
    /// A person's words for it: `±0.125`, `(mod 2^24)`.
    pub fn words(&self) -> String {
        match self {
            Bound::HalfResolution(h) => format!("±{h}"),
            Bound::Width(w) => format!("(mod 2^{w})"),
        }
    }
}

/// Why a sample was not decoded through a contract. Tagged `reason`.
///
/// One variant per rung of the tooling guide's O2 ladder, so a script
/// branches on the rung, never on prose. Read back as well as written: a
/// `.zsnap` row carries it (#612, FJ8b).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Unresolved {
    /// Rung 1: the wire key does not sit under the stated namespace. A raw
    /// verb sees full keys; one outside the namespace is another
    /// deployment's, or none, and is not attributed by guessing (O3).
    NotInNamespace { namespace: String },
    /// The key is not a zk2 data key (the bus is shared, §1.7).
    NotZk2 { detail: String },
    /// A zk2 control key (`@zk`, §1.1): an instance, a token, a member or a
    /// contract bundle. A zk2 key, and no contract types its payload.
    ControlKey { form: String },
    /// Rung 3 was not asked: no presence read stood behind this rendering,
    /// so no provider, and no revision, could be found for the key (O4).
    PresenceNotRead,
    /// Presence shows no instance of this address providing the interface.
    NoProvider,
    /// A provider is present, and no descriptor named its revision: a
    /// token's fingerprint prefix is not enough to retrieve a bundle.
    NoRevision,
    /// More than one revision could have written the key, and nothing
    /// chooses one: the provider's instances name several and a data key
    /// carries no instance; or, with no descriptor naming the owner's
    /// revision (across a face, a capture read offline), several revisions
    /// held are left that read the sample differently, or no catalog
    /// orders them (observer §7.5). Decoding with either could mislead.
    Ambiguous { fingerprints: Vec<String> },
    /// With no descriptor naming the owner's revision (across a face, a
    /// capture read offline), no revision held is left to read the key
    /// with (observer §7.5): `step` names the step that left none, and
    /// `held` the revisions of the interface held, sorted.
    NoneLeft {
        step: NoneStep,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        held: Vec<String>,
    },
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
    /// The decode was not asked (`--no-decode`): the bytes are rendered
    /// structurally on purpose, and no verdict is claimed about them (O4).
    DecodeNotAsked,
}

/// The step of the revision of a key that left no revision held
/// (observer §7.5), as a tool names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoneStep {
    /// The tool holds no revision of the key's interface.
    NotHeld,
    /// None of the revisions held gives the key a resource.
    NoResource,
    /// The catalog given lists none of those that do.
    NotListed,
    /// None of several left carries CH's tag.
    Tag,
}

impl NoneStep {
    /// Its token, as `spec/conformance/envelope/tags.json` spells it.
    pub fn token(self) -> &'static str {
        match self {
            NoneStep::NotHeld => "not_held",
            NoneStep::NoResource => "no_resource",
            NoneStep::NotListed => "not_listed",
            NoneStep::Tag => "tag",
        }
    }
}

impl Unresolved {
    /// The rung, in a few words: one spelling for every frontend and for
    /// the engine's own reasons (a `not_checked` conformance, a field the
    /// declared paths could not judge).
    pub fn words(&self) -> String {
        let short = |fp: &str| {
            let hex = fp.strip_prefix("sha256:").unwrap_or(fp);
            match hex.get(..16) {
                Some(head) if hex.len() > 16 => format!("sha256:{head}…"),
                _ => fp.to_owned(),
            }
        };
        match self {
            Unresolved::NotInNamespace { namespace } if namespace.is_empty() => {
                "not under the bus-root deployment's keys".into()
            }
            Unresolved::NotInNamespace { namespace } => {
                format!("not in namespace {namespace:?}")
            }
            Unresolved::NotZk2 { detail } => format!("not a zk2 data key: {detail}"),
            Unresolved::ControlKey { form } => {
                format!("a zk2 control key (@zk/{form}): no contract types it")
            }
            Unresolved::PresenceNotRead => {
                "presence was not read, so no revision is named for the key".into()
            }
            Unresolved::NoProvider => "no provider of this address in presence".into(),
            Unresolved::NoRevision => "no descriptor named the provider's revision".into(),
            Unresolved::Ambiguous { fingerprints } => format!(
                "{} revisions could have written it and nothing chooses one: a data key names \
                 no instance, and across a face neither CH's tag nor a catalog does (observer \
                 §7.5)",
                fingerprints.len()
            ),
            Unresolved::NoneLeft { step, held } => match step {
                NoneStep::NotHeld => {
                    "no revision of its interface is held, and no descriptor names one".into()
                }
                NoneStep::NoResource => format!(
                    "none of the {} revisions held gives the key a resource",
                    held.len()
                ),
                NoneStep::NotListed => {
                    "the catalog given lists none of the revisions held that give the key a \
                     resource (observer §7.5)"
                        .into()
                }
                NoneStep::Tag => {
                    "none of the revisions held that give the key a resource carries CH's tag \
                     (observer §7.5)"
                        .into()
                }
            },
            Unresolved::ContractNotHeld { fingerprint } => {
                format!("contract {} not held", short(fingerprint))
            }
            Unresolved::ContractUnavailable { fingerprint, .. } => {
                format!("contract {} unavailable (§8.4)", short(fingerprint))
            }
            Unresolved::ContractUnreadable {
                fingerprint,
                detail,
            } => format!("contract {} unreadable: {detail}", short(fingerprint)),
            Unresolved::NoResource { fingerprint } => {
                format!("no resource of {} matches the key", short(fingerprint))
            }
            Unresolved::NoMember { resource, member } => {
                format!("{resource} declares no {member}")
            }
            Unresolved::DecodeNotAsked => "decoding was not asked".into(),
        }
    }
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
                epoch_assumed: None,
                dated: None,
                bounds: None,
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

    /// Across a face, no revision left (observer §7.5; #990 V2): the step
    /// that left none by its token, and the revisions held.
    #[test]
    fn none_left_json_shape_is_pinned() {
        let why = Unresolved::NoneLeft {
            step: NoneStep::Tag,
            held: vec![format!("sha256:{}", "ab".repeat(32))],
        };
        let v = serde_json::to_value(&why).expect("serialize");
        assert_eq!(
            v,
            json!({"reason": "none_left", "step": "tag",
                   "held": [format!("sha256:{}", "ab".repeat(32))]})
        );
        assert_eq!(serde_json::from_value::<Unresolved>(v).unwrap(), why);
        let none = Unresolved::NoneLeft {
            step: NoneStep::NotHeld,
            held: vec![],
        };
        assert_eq!(
            serde_json::to_value(&none).unwrap(),
            json!({"reason": "none_left", "step": "not_held"})
        );
        for s in [
            NoneStep::NotHeld,
            NoneStep::NoResource,
            NoneStep::NotListed,
            NoneStep::Tag,
        ] {
            assert_eq!(serde_json::to_value(s).unwrap(), json!(s.token()));
        }
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
