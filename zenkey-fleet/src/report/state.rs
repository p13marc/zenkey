//! A zk2 state read (spec §4): `zenctl get state`, the owner's answer
//! (S4) or an archive's last-known one (S5).
//!
//! The two are different questions, and the shape says which was asked
//! before it says anything else: `reading` is `current` or `last_known`, in
//! every medium, and an archive's answer is never rendered as current (S6).
//! Silence from the owner is no rows and is never "no value": the verb
//! exits 2 on it.
//!
//! [`Stamp`] is shared with `watch`'s samples: a time is shown with the id
//! of the clock that issued it (§4.1), the owner's own under S1.
//!
//! A row keeps the reply's bytes beside its rendering (#765), and never
//! serializes them: the shape is the rendering's, and a byte view (a hex
//! dump, the sniff of bytes that did not decode) is a renderer's concern.

use std::collections::BTreeMap;

use serde::Serialize;

use super::payload::PayloadRendering;

/// One state read.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StateReport {
    /// Which question was asked: the owner's current state, or an
    /// archive's last-known state.
    pub reading: StateReading,
    /// The owner, `<system>/<service>`.
    pub address: String,
    /// `<name>.v<major>`.
    pub iface: String,
    /// The revision the values were decoded against.
    pub fingerprint: String,
    /// `<kind token>/<template>`.
    pub resource: String,
    /// The template values given, unslugged; a parameter not given is a
    /// wildcard (current reads only).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, Vec<String>>,
    /// The key expressions the GET went out on, base-relative: the owner's
    /// keys (S4), or the archive's key for them (§4.4).
    pub selectors: Vec<String>,
    /// The archive asked, `<system>/<service>`: last-known reads only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<String>,
    /// How long the GET waited for replies, seconds.
    pub timeout_s: f64,
    /// Every key answered, sorted. Empty is silence: no reply within the
    /// timeout, which is not a verdict about the key (S6, O5).
    pub rows: Vec<StateRow>,
}

/// Which state question a read asked (S6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StateReading {
    /// The owner's answer to a GET on its keys (S4): current state.
    Current,
    /// An archive's answer (S5): last-known, never current.
    LastKnown,
}

/// One key of a state read.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StateRow {
    /// The owner's key, base-relative (for a last-known read, the key the
    /// archive recorded).
    pub key: String,
    /// The value, or the key's deletion. Tagged `state`.
    #[serde(flatten)]
    pub value: StateValue,
    /// The reply's payload as it arrived, refcounted, so a clone is a
    /// refcount bump: the owner's sample (S4), or the bytes the archive
    /// recorded (S5). `None` on a deletion, which carries none.
    ///
    /// Never serialized (#765): `get state --format json` and the pinned
    /// shape are the rendering's. A byte view is a renderer's concern, not
    /// this report's contract. Equality compares the bytes' content.
    #[serde(skip)]
    pub bytes: Option<zenoh::bytes::ZBytes>,
    /// The mutation's stamp; absent when the reply carried none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<Stamp>,
    /// Last-known reads only: whether the archive's alignment confirmed
    /// the key (§4.4). `false` is kept, served, and not confirmed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed: Option<bool>,
    /// Last-known reads only: the value's type identity as the archive
    /// recorded it, `{"iface", "contract", "type"}` (§7.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<serde_json::Value>,
}

/// A key's state. Tagged `state`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum StateValue {
    /// The key's value, rendered through the contract's type.
    Value { payload: Box<PayloadRendering> },
    /// Deleted: a `reply_del` within the owner's tombstone window (S3), or
    /// a tombstone an archive holds. Not an empty value.
    Deleted,
}

/// A timestamp, and whose clock issued it (§4.1, §4.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Stamp {
    /// The stamp's time, RFC 3339 with nanoseconds, UTC.
    pub time: String,
    /// The id of the clock that issued it: the issuing session's zid. An
    /// owner's own stamp carries its session's (S1); a router's, the
    /// router's.
    pub clock: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Rendered, ResolvedResource};
    use serde_json::json;

    fn payload(key: &str) -> PayloadRendering {
        PayloadRendering {
            key: key.into(),
            size: 2,
            resource: Some(ResolvedResource {
                iface: "tc.netif.v1".into(),
                fingerprint: format!("sha256:{}", "ab".repeat(32)),
                resource: "state/namespaces".into(),
                member: "type".into(),
                values: BTreeMap::new(),
            }),
            rendered: Rendered::Value {
                declared: "json:Namespaces".into(),
                value: json!([]),
            },
        }
    }

    /// `reading` leads, so a script never mistakes an archive's answer for
    /// the owner's; a deletion is a `state`, not an empty value. The bytes a
    /// row keeps (#765) reach no serialized shape: the row below carries
    /// them, and its JSON is the rendering's alone.
    #[test]
    fn state_report_json_shape_is_pinned() {
        let key = "zk2/host-a/tc/tc.netif.v1/state/namespaces";
        let r = StateReport {
            reading: StateReading::LastKnown,
            address: "host-a/tc".into(),
            iface: "tc.netif.v1".into(),
            fingerprint: format!("sha256:{}", "ab".repeat(32)),
            resource: "state/namespaces".into(),
            values: BTreeMap::new(),
            selectors: vec![
                "zk2/ground/archive/archive.v1/@state/host-a/tc/tc.netif.v1/state/namespaces"
                    .into(),
            ],
            archive: Some("ground/archive".into()),
            timeout_s: 5.0,
            rows: vec![StateRow {
                key: key.into(),
                value: StateValue::Value {
                    payload: Box::new(payload(key)),
                },
                bytes: Some(zenoh::bytes::ZBytes::from(&b"[]"[..])),
                timestamp: Some(Stamp {
                    time: "2026-10-08T12:00:00.000000000Z".into(),
                    clock: "a1b2".into(),
                }),
                confirmed: Some(false),
                identity: Some(json!({"iface": "tc.netif.v1"})),
            }],
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["reading"], "last_known");
        assert_eq!(v["archive"], "ground/archive");
        assert_eq!(
            v["rows"][0],
            json!({
                "key": key,
                "state": "value",
                "payload": serde_json::to_value(payload(key)).unwrap(),
                "timestamp": {"time": "2026-10-08T12:00:00.000000000Z", "clock": "a1b2"},
                "confirmed": false,
                "identity": {"iface": "tc.netif.v1"},
            })
        );

        assert_eq!(
            r.rows[0].bytes.as_ref().map(|b| b.to_bytes().into_owned()),
            Some(b"[]".to_vec()),
            "kept beside the rendering, for a renderer to show"
        );

        let current = StateRow {
            key: key.into(),
            value: StateValue::Deleted,
            bytes: None,
            timestamp: None,
            confirmed: None,
            identity: None,
        };
        assert_eq!(
            serde_json::to_value(&current).unwrap(),
            json!({"key": key, "state": "deleted"})
        );
        assert_eq!(
            serde_json::to_value(StateReading::Current).unwrap(),
            json!("current")
        );
    }
}
