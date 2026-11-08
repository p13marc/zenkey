//! A zk2 subscription's stream (spec §3.2): `zenctl watch`, through the
//! runtime's `Consumer::for_tool`.
//!
//! A stream has rows and no envelope: one [`WatchSample`] per delivered
//! sample, and per sample the runtime refused as undecodable (core §7.5,
//! §7.6), its payload undecodable with its refusal named (observer §7.3);
//! and one [`WatchSummary`] when it ends. R6's discards — samples
//! put on a wildcard key, which a consumer drops by rule — are counted
//! apart from every loss (the tooling guide's O6): they are the rule
//! working, not the observer falling behind.

use std::collections::BTreeMap;

use serde::Serialize;

use super::observe::{Conformance, QosMismatch};
use super::payload::PayloadRendering;
use super::state::{InbandStamp, Stamp};

/// One sample: delivered, or refused as undecodable, its conformance
/// `undecodable` and its refusal named.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WatchSample {
    /// `<system>/<service>` whose key carried it: R3's edge, observed.
    pub provider: String,
    /// The concrete key, base-relative.
    pub key: String,
    /// The template's values on that key, unslugged.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, Vec<String>>,
    /// The sample's in-band stamp, its envelope's run and seq (core §4.3):
    /// absent when it carried no envelope, as an `@stream` sample does not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inband: Option<InbandStamp>,
    /// An event's time (core §7.5's form 4; observer §7.3), as
    /// `crate::model::occurrence::words` spells it with its stamp: `t, run
    /// r, seq s`, `time unknown` for an unknown clock. Absent for every
    /// sample that is no event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred: Option<String>,
    /// The sample's Zenoh timestamp and whose clock issued it; absent when
    /// it carried none, as a sample off core §4.3's table may not.
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

/// One heartbeat the watch's view read for the collection watched (core
/// §7.5's form 6; #978's F2; #990 V6): its provider, its key and in-band
/// stamp, the members it vouched for whose digest was the view's, in key
/// order, whether one was lost before it and whether its section diverged.
/// A row of its own, never a sample: no value moved. A loss and a
/// divergence are the reader's word, no finding about the owner
/// (`freshness.v2` §5).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WatchConfirmation {
    /// `<system>/<service>` whose heartbeat it was.
    pub provider: String,
    /// The heartbeat's key, base-relative.
    pub key: String,
    /// The heartbeat key's run and seq.
    pub heartbeat: InbandStamp,
    /// The members confirmed, base-relative, in key order: none where it
    /// diverged, or its collection was absent.
    pub confirmed: Vec<String>,
    /// Its key's seqs skipped since the last heartbeat the view read: one
    /// or more were lost.
    pub lost: bool,
    /// Its section's digest, or its live count, was not the view's, so it
    /// confirmed nothing (`freshness.v2` §2.5).
    pub diverged: bool,
    /// Its map bit was 0: the collection not available, nothing confirmed.
    /// Omitted when false.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub absent: bool,
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
    /// Samples shown: delivered, or refused as undecodable.
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
    /// Puts that failed their declared type: invalid, or undecodable,
    /// which a refused one is.
    pub nonconforming: u64,
    /// Samples dropped because this tool fell behind its own buffer:
    /// losses, which make `received` a lower bound (O6).
    pub lagged: u64,
    /// Heartbeats the view read for the collection (#990 V6): none for a
    /// resource that is no collection, or one whose parameters are bound.
    pub heartbeats: u64,
    /// Of them, those read after a gap in their key's seqs: a heartbeat
    /// lost before each (`freshness.v2` §2.5). The reader's word.
    pub heartbeats_lost: u64,
    /// Of them, those whose section diverged from the view: nothing
    /// confirmed. The reader's word, no finding about the owner (§5).
    pub heartbeats_diverged: u64,
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
    fn a_confirmation_rows_json_shape_is_pinned() {
        let c = WatchConfirmation {
            provider: "lab/n1".into(),
            key: "zk2/lab/n1/freshness.v2/stream/heartbeat/hb.v1".into(),
            heartbeat: crate::report::InbandStamp {
                run: "0007".into(),
                seq: 3,
            },
            confirmed: vec!["zk2/lab/n1/hb.v1/state/link".into()],
            lost: false,
            diverged: false,
            absent: false,
        };
        assert_eq!(
            serde_json::to_value(&c).unwrap(),
            json!({
                "provider": "lab/n1",
                "key": "zk2/lab/n1/freshness.v2/stream/heartbeat/hb.v1",
                "heartbeat": {"run": "0007", "seq": 3},
                "confirmed": ["zk2/lab/n1/hb.v1/state/link"],
                "lost": false,
                "diverged": false,
            })
        );
    }

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
            inband: Some(crate::report::InbandStamp {
                run: "0003".into(),
                seq: 41,
            }),
            occurred: None,
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
                "inband": {"run": "0003", "seq": 41},
                "kind": "put",
                "payload": serde_json::to_value(&payload).unwrap(),
                "conformance": {"state": "not_checked", "reason": "no resource matches"},
            })
        );
        let d = WatchSample {
            event: WatchEvent::Delete,
            values: BTreeMap::new(),
            inband: None,
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
            heartbeats: 3,
            heartbeats_lost: 1,
            heartbeats_diverged: 0,
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
        assert_eq!(
            (
                v["heartbeats"].clone(),
                v["heartbeats_lost"].clone(),
                v["heartbeats_diverged"].clone()
            ),
            (json!(3), json!(1), json!(0)),
            "counts, present at zero (#990 V6)"
        );
    }
}
