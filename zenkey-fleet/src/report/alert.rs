//! The alert plane as an observer sees it (#388): one producer's alert
//! document changing state on its stable key.
//!
//! RFC 04 §1.2 makes alerts **state**: a `put` on
//! `…/state/<producer>/alert/<alert_key>` is the alert firing, a `delete`
//! is the tombstone that resolves it, and the key is the identity across
//! both. [`AlertTransition`] is that observation as a wire shape — what a
//! notifier (zenwatch) routes and what a script reads — and
//! [`crate::model::alert::alert_transition`] is the one place it is built.

use std::collections::BTreeMap;

use serde::Serialize;

/// Which way one alert key just moved (RFC 04 §1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertState {
    /// A `put` landed on the alert key: the alert is firing (or re-fired).
    Firing,
    /// A `delete` landed on the alert key: the producer retired the alert.
    Resolved,
}

/// Where the fields riding an [`AlertTransition`] came from — said out loud,
/// because "no severity" reads differently under each (RFC 13 §3 O4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderSource {
    /// Decoded through the producer's served schema (RFC 08 §7).
    Schema,
    /// No served schema: a structural reading of the bytes (JSON if they
    /// parse, CBOR diagnostic otherwise), which may still carry the fields.
    Structural,
    /// Nothing was decoded — a tombstone carries no payload, and a payload
    /// that read as nothing structured carries no fields.
    KeyOnly,
}

/// One alert key's state change, with everything the key and the payload
/// said about it.
///
/// `origin`, `producer` and `alert_key` are read from the **key**, never
/// the payload (RFC 11 §3.2 — a proxy producer's `source` is the polled
/// device). The optional fields are lifted from the decoded document when it
/// carried them and are `None` otherwise; a `Resolved` transition carries
/// none of them, because a tombstone carries no document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AlertTransition {
    /// The publishing origin chunk (`h-<12hex>`, or a service origin).
    pub origin: String,
    /// The producer chunk.
    pub producer: String,
    /// The `<alert_key>` chunk (RFC 11 §3.1 for the reference profile).
    pub alert_key: String,
    /// `origin.producer.alert_key` (RFC 11 §3.2) — the one-chunk identity an
    /// acknowledgement is keyed by.
    pub alert_ref: String,
    pub state: AlertState,
    /// The document's `severity`, verbatim, when it carried one.
    pub severity: Option<String>,
    /// The document's `rule`, when it carried one.
    pub rule: Option<String>,
    /// The document's `labels` (string-valued; the `host` label is dropped —
    /// the origin already says which host, RFC 11 §3.1).
    pub labels: BTreeMap<String, String>,
    /// The document's `summary` or `message`, when it carried one.
    pub summary: Option<String>,
    /// The sample's HLC timestamp as the engine renders it, when the sample
    /// carried one — whose clock that is is the [`crate::StampProvenance`]
    /// question, not answered here.
    pub timestamp: Option<String>,
    /// RFC 3339 wall clock of the observation.
    pub at: String,
    pub rendering: RenderSource,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ndjson shape a notifier's consumers read: field names, snake_case
    /// states and sources, `null` for what the document did not say.
    #[test]
    fn alert_transition_json_shape_is_pinned() {
        let mut labels = BTreeMap::new();
        labels.insert("port".to_string(), "eth0".to_string());
        let t = AlertTransition {
            origin: "h-3fa9c2d41b7e".into(),
            producer: "netlink".into(),
            alert_key: "a659f813308ad1da".into(),
            alert_ref: "h-3fa9c2d41b7e.netlink.a659f813308ad1da".into(),
            state: AlertState::Firing,
            severity: Some("warning".into()),
            rule: Some("link_down".into()),
            labels,
            summary: Some("eth0 is down".into()),
            timestamp: None,
            at: "2026-09-06T00:00:00Z".into(),
            rendering: RenderSource::Schema,
        };
        assert_eq!(
            serde_json::to_value(&t).unwrap(),
            serde_json::json!({
                "origin": "h-3fa9c2d41b7e",
                "producer": "netlink",
                "alert_key": "a659f813308ad1da",
                "alert_ref": "h-3fa9c2d41b7e.netlink.a659f813308ad1da",
                "state": "firing",
                "severity": "warning",
                "rule": "link_down",
                "labels": {"port": "eth0"},
                "summary": "eth0 is down",
                "timestamp": null,
                "at": "2026-09-06T00:00:00Z",
                "rendering": "schema",
            })
        );
        let resolved = AlertTransition {
            state: AlertState::Resolved,
            severity: None,
            rule: None,
            labels: BTreeMap::new(),
            summary: None,
            rendering: RenderSource::KeyOnly,
            ..t
        };
        let json = serde_json::to_value(&resolved).unwrap();
        assert_eq!(json["state"], "resolved");
        assert_eq!(json["rendering"], "key_only");
        assert_eq!(json["labels"], serde_json::json!({}));
        assert_eq!(json["severity"], serde_json::Value::Null);
    }
}
