//! The alert projection (#388): one sample on the alert plane → one
//! [`AlertTransition`], from values in hand.
//!
//! RFC 04 §1.2 — *alerts are state*: a `put` on
//! `…/state/<producer>/alert/<alert_key>` is firing, a `delete` is resolved,
//! and the key is the identity through both. This module reads that off a
//! wire key and a sample kind, and lifts what the decoded document says about
//! itself (severity, rule, labels, a summary) when there is a document to
//! read. It takes no session — the decode that produces the value is
//! [`crate::model::decode::decode_sample`]'s, and a `.zrec` replays through
//! this exactly as live traffic does.
//!
//! Routing the transition — which rule it matches, which sink it reaches,
//! whether it is a duplicate — is the notifier's job, not the engine's.

use std::collections::BTreeMap;

use zenoh::sample::SampleKind;

use crate::report::{AlertState, AlertTransition, RenderSource};

/// Project one sample on the alert plane into an [`AlertTransition`].
///
/// `None` when `wire_key` is not an alert key under `base` — another
/// deployment's key, a key that does not parse, a state subject that is not
/// `alert/<alert_key>` — which for an observer is the meaningful answer,
/// not an error.
///
/// * `kind`: a [`SampleKind::Put`] is [`AlertState::Firing`], a
///   [`SampleKind::Delete`] is [`AlertState::Resolved`] (the tombstone).
/// * `decoded`: the document as a JSON value with where it came from —
///   `Schema` off the served schema, `Structural` off the bytes. `None` when
///   nothing structured was read (every `Delete` — a tombstone has no
///   body — and a `Put` whose bytes read as nothing), which renders as
///   [`RenderSource::KeyOnly`].
/// * `stamped`: the sample's HLC as the engine renders it, when it carried
///   one.
/// * `at`: the observation's RFC 3339 wall clock.
///
/// What is lifted from the document, when present and string-valued:
/// `severity`, `rule`, `labels` (a string-valued object; the `host` label is
/// dropped because the origin in the key already names the host, RFC 11
/// §3.1), and `summary` — or `message`, the incumbent spelling — as the
/// summary. Anything else in the document is the renderer's to show.
pub fn alert_transition(
    base: &str,
    wire_key: &str,
    kind: SampleKind,
    decoded: Option<(RenderSource, &serde_json::Value)>,
    stamped: Option<&str>,
    at: &str,
) -> Option<AlertTransition> {
    use zenkey::grammar::{Class, ClassOrPlane};

    let parsed = zenkey::grammar::parse_full(base, wire_key)?;
    if parsed.class != ClassOrPlane::Class(Class::State) {
        return None;
    }
    // `alert/<alert_key>` and nothing else: the family's fixed prefix plus
    // exactly its one population variable (RFC 04 §1.4).
    let prefix = zenkey::CommonFamily::Alert.prefix();
    let alert_key = match parsed.subject.as_slice() {
        [head, key] if [*head] == prefix[..] && !key.is_empty() => *key,
        _ => return None,
    };
    let origin = parsed.origin.chunk().to_string();
    // A service origin has no producer chunk — the service *is* the
    // producer, the same reading `token_identity` gives a liveliness token.
    let producer = parsed
        .producer()
        .map(|p| p.chunk())
        .unwrap_or_else(|| origin.trim_start_matches('@').to_string());
    let alert_ref = zenkey::alert::alert_ref(&origin, &producer, alert_key).ok()?;

    let state = match kind {
        SampleKind::Put => AlertState::Firing,
        SampleKind::Delete => AlertState::Resolved,
    };
    let mut out = AlertTransition {
        origin,
        producer,
        alert_key: alert_key.to_string(),
        alert_ref,
        state,
        severity: None,
        rule: None,
        labels: BTreeMap::new(),
        summary: None,
        timestamp: stamped.map(str::to_string),
        at: at.to_string(),
        rendering: RenderSource::KeyOnly,
    };
    // A tombstone carries no document, whatever bytes rode with it.
    if state == AlertState::Resolved {
        return Some(out);
    }
    if let Some((source, doc)) = decoded {
        out.rendering = match source {
            // "Decoded from nothing" is not a source; a value in hand was
            // read from *somewhere*, and structural is the honest floor.
            RenderSource::KeyOnly => RenderSource::Structural,
            s => s,
        };
        let field = |name: &str| doc.get(name).and_then(|v| v.as_str()).map(str::to_string);
        out.severity = field("severity");
        out.rule = field("rule");
        out.summary = field("summary").or_else(|| field("message"));
        if let Some(labels) = doc.get("labels").and_then(|v| v.as_object()) {
            for (k, v) in labels {
                if k == "host" {
                    continue;
                }
                // Non-string label values are rendered, not dropped — a
                // numeric `port: 22` is still a discriminating label.
                let v = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                out.labels.insert(k.clone(), v);
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "v1/h-3fa9c2d41b7e/state/netlink/alert/a659f813308ad1da";

    /// A `put` with a document: firing, with severity, rule, labels and the
    /// message lifted; `host` dropped; the ref minted off the key.
    #[test]
    fn a_put_is_firing_with_the_documents_fields_lifted() {
        let doc = serde_json::json!({
            "severity": "warning",
            "rule": "link_down",
            "labels": {"port": "eth0", "host": "h-3fa9c2d41b7e", "vlan": 7},
            "message": "eth0 is down",
        });
        let t = alert_transition(
            "",
            KEY,
            SampleKind::Put,
            Some((RenderSource::Schema, &doc)),
            Some("7/…"),
            "t0",
        )
        .expect("an alert key");
        assert_eq!(t.state, AlertState::Firing);
        assert_eq!(t.origin, "h-3fa9c2d41b7e");
        assert_eq!(t.producer, "netlink");
        assert_eq!(t.alert_key, "a659f813308ad1da");
        assert_eq!(t.alert_ref, "h-3fa9c2d41b7e.netlink.a659f813308ad1da");
        assert_eq!(t.severity.as_deref(), Some("warning"));
        assert_eq!(t.rule.as_deref(), Some("link_down"));
        assert_eq!(t.summary.as_deref(), Some("eth0 is down"));
        assert_eq!(t.timestamp.as_deref(), Some("7/…"));
        assert_eq!(t.rendering, RenderSource::Schema);
        assert_eq!(
            t.labels,
            BTreeMap::from([
                ("port".to_string(), "eth0".to_string()),
                ("vlan".to_string(), "7".to_string()),
            ]),
            "host is dropped, a numeric label is rendered"
        );
    }

    /// A `delete` is resolved and carries no document fields, whatever was
    /// handed in beside it — a tombstone has no body.
    #[test]
    fn a_delete_is_resolved_with_no_fields() {
        let doc = serde_json::json!({"severity": "error"});
        let t = alert_transition(
            "",
            KEY,
            SampleKind::Delete,
            Some((RenderSource::Structural, &doc)),
            None,
            "t1",
        )
        .expect("an alert key");
        assert_eq!(t.state, AlertState::Resolved);
        assert_eq!(t.severity, None);
        assert!(t.labels.is_empty());
        assert_eq!(t.rendering, RenderSource::KeyOnly);
        assert_eq!(t.timestamp, None);
    }

    /// Under a base the key is stripped first; a key from another deployment,
    /// a non-state class, or a state subject that is not `alert/<key>` is
    /// `None`, never a transition.
    #[test]
    fn a_non_alert_key_is_none() {
        let based = format!("zensight/{KEY}");
        assert!(alert_transition("zensight", &based, SampleKind::Put, None, None, "t").is_some());
        assert!(alert_transition("other", &based, SampleKind::Put, None, None, "t").is_none());
        for key in [
            "v1/h-3fa9c2d41b7e/state/netlink/health",
            "v1/h-3fa9c2d41b7e/telemetry/netlink/alert/a659f813308ad1da",
            "v1/h-3fa9c2d41b7e/state/netlink/alert/a659f813308ad1da/extra",
            "not/a/key",
        ] {
            assert!(
                alert_transition("", key, SampleKind::Put, None, None, "t").is_none(),
                "{key}"
            );
        }
        // A put with nothing decoded is still firing — key-only.
        let t = alert_transition("", KEY, SampleKind::Put, None, None, "t").unwrap();
        assert_eq!(t.state, AlertState::Firing);
        assert_eq!(t.rendering, RenderSource::KeyOnly);
    }
}
