//! A zk2 subscription's stream (spec §3.2): `zenctl watch`, through the
//! runtime's `Consumer::for_tool`.
//!
//! A stream has rows and no envelope: one [`WatchSample`] per delivered
//! sample, and one [`WatchSummary`] when it ends. R6's discards — samples
//! put on a wildcard key, which a consumer drops by rule — are counted
//! apart from every loss (the tooling guide's O6): they are the rule
//! working, not the observer falling behind.

use std::collections::BTreeMap;

use serde::Serialize;

use super::observe::{Conformance, QosMismatch};
use super::payload::PayloadRendering;
use super::state::Stamp;

/// One delivered sample.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WatchSample {
    /// `<system>/<service>` whose key carried it: R3's edge, observed.
    pub provider: String,
    /// The concrete key, base-relative.
    pub key: String,
    /// The template's values on that key, unslugged.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, Vec<String>>,
    /// The sample's stamp and whose clock issued it; absent when it
    /// carried none (a stream sample may not).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<Stamp>,
    /// The QoS the sample rode against its resource's (spec §2.4: an owner
    /// MUST publish with it), present only when they differ: the declared
    /// axes, the observed ones, and which (#612, FJ8b).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qos_mismatch: Option<QosMismatch>,
    /// A put or a delete. Tagged `kind`.
    #[serde(flatten)]
    pub event: WatchEvent,
}

/// What a sample was. Tagged `kind`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WatchEvent {
    /// A value, rendered through the contract's type, and its attachment
    /// through the declared attachment type when one rode along.
    Put {
        payload: Box<PayloadRendering>,
        /// The payload against its declared type (§7.2, §7.3): valid,
        /// invalid, undecodable, or not checked with why.
        conformance: Conformance,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attachment: Option<Box<PayloadRendering>>,
    },
    /// A delete: on state, the key's retirement. Not an empty value.
    Delete,
}

/// How a watch went, when it ends.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WatchSummary {
    /// The address or pattern watched, `<system>/<service>`.
    pub address: String,
    /// `<name>.v<major>`.
    pub iface: String,
    /// The revision samples were decoded against.
    pub fingerprint: String,
    /// `<kind token>/<template>`.
    pub resource: String,
    /// The key expressions subscribed, base-relative: what the watch could
    /// see, and no wider.
    pub selectors: Vec<String>,
    /// Samples delivered.
    pub received: u64,
    /// Samples put on a wildcard key and discarded by rule (R6). Not
    /// losses.
    pub discarded: u64,
    /// Samples on a concrete key that resolved to no member of the
    /// resource through a bound provider (#671): the bus and the contract
    /// disagree. Counted apart from R6's discards and from lag (O6).
    pub unresolved: u64,
    /// Delivered samples whose QoS differed from the resource's (§2.4).
    pub qos_mismatched: u64,
    /// Delivered puts that failed their declared type: invalid, or
    /// undecodable.
    pub nonconforming: u64,
    /// Samples dropped because this tool fell behind its own buffer:
    /// losses, which make `received` a lower bound (O6).
    pub lagged: u64,
    /// How long the subscription was open, seconds.
    pub elapsed_s: f64,
    /// Why it ended.
    pub ended: WatchEnd,
}

/// Why a watch ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchEnd {
    /// `--count` samples arrived.
    Count,
    /// The `--for` window closed.
    Window,
    /// The operator interrupted it.
    Interrupted,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Rendered, Unresolved};
    use serde_json::json;

    #[test]
    fn watch_rows_json_shape_is_pinned() {
        let key = "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0";
        let payload = PayloadRendering {
            key: key.into(),
            size: 3,
            resource: None,
            rendered: Rendered::Structural {
                why: Unresolved::NoResource {
                    fingerprint: format!("sha256:{}", "ab".repeat(32)),
                },
                value: None,
                text: "abc".into(),
            },
        };
        let s = WatchSample {
            provider: "host-a/tc".into(),
            key: key.into(),
            values: [("iface".to_owned(), vec!["eth0".to_owned()])].into(),
            timestamp: None,
            qos_mismatch: None,
            event: WatchEvent::Put {
                payload: Box::new(payload.clone()),
                conformance: Conformance::NotChecked {
                    reason: "no resource matches".into(),
                },
                attachment: None,
            },
        };
        assert_eq!(
            serde_json::to_value(&s).unwrap(),
            json!({
                "provider": "host-a/tc",
                "key": key,
                "values": {"iface": ["eth0"]},
                "kind": "put",
                "payload": serde_json::to_value(&payload).unwrap(),
                "conformance": {"state": "not_checked", "reason": "no resource matches"},
            })
        );
        let d = WatchSample {
            event: WatchEvent::Delete,
            values: BTreeMap::new(),
            ..s
        };
        assert_eq!(
            serde_json::to_value(&d).unwrap(),
            json!({"provider": "host-a/tc", "key": key, "kind": "delete"})
        );

        let summary = WatchSummary {
            address: "*/tc".into(),
            iface: "tc.netif.v1".into(),
            fingerprint: format!("sha256:{}", "ab".repeat(32)),
            resource: "stream/bandwidth/{ns}/{iface}".into(),
            selectors: vec!["zk2/*/tc/tc.netif.v1/stream/bandwidth/*/*".into()],
            received: 4,
            discarded: 10,
            unresolved: 2,
            qos_mismatched: 1,
            nonconforming: 0,
            lagged: 0,
            elapsed_s: 1.5,
            ended: WatchEnd::Window,
        };
        let v = serde_json::to_value(&summary).unwrap();
        assert_eq!(v["discarded"], 10);
        assert_eq!(v["unresolved"], 2, "apart from R6's discards (#671)");
        assert_eq!(v["qos_mismatched"], 1);
        assert_eq!(v["nonconforming"], 0, "a count, present at zero");
        assert_eq!(v["received"], 4);
        assert_eq!(
            v["lagged"], 0,
            "a count, present at zero: the claim it bounds is made"
        );
        assert_eq!(v["ended"], "window");
    }
}
