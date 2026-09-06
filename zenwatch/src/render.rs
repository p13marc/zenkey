//! Rendering: one notification's title and message, **bounded**.
//!
//! A 4 KB alert document must not become a 4 KB push, so every rendering
//! is cut at [`RenderConfig::max_message_bytes`] and says so — `truncated`
//! rides the wire shape, never a silent ellipsis. The payload goes in last,
//! after the facts that fit in a glance (state, evidence, labels, key,
//! timestamp), so the cut lands on the part a reader can fetch from the bus
//! anyway.
//!
//! Three sources, said out loud (RFC 13 §3 O4): typed fields through the
//! producer's served schema (RFC 08 §7); a structural reading when no
//! schema is served, prefixed with why and bounded separately at
//! [`RenderConfig::max_structural_bytes`]; and key-plus-timestamp when
//! nothing was decoded, with the reason. A producer with no schema still
//! yields a usable notification.

use std::collections::BTreeMap;

use serde::Deserialize;
use zenkey_fleet::{CondState, RenderSource};

/// The two byte bounds.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RenderConfig {
    /// The whole message, title excluded.
    #[serde(default = "default_message_bytes")]
    pub max_message_bytes: usize,
    /// The structural payload reading alone, before it joins the message.
    #[serde(default = "default_structural_bytes")]
    pub max_structural_bytes: usize,
}

fn default_message_bytes() -> usize {
    2048
}
fn default_structural_bytes() -> usize {
    512
}

impl Default for RenderConfig {
    fn default() -> Self {
        RenderConfig {
            max_message_bytes: default_message_bytes(),
            max_structural_bytes: default_structural_bytes(),
        }
    }
}

/// What the payload rendered to, if anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payload {
    /// Named fields through the served schema.
    Typed(serde_json::Value),
    /// No served schema: the structural reading, and what was asked for
    /// (the registered type name, or the key when nothing refined).
    Structural { text: String, what: String },
    /// Nothing decoded, and why.
    None { reason: String },
}

impl Payload {
    pub fn source(&self) -> RenderSource {
        match self {
            Payload::Typed(_) => RenderSource::Schema,
            Payload::Structural { .. } => RenderSource::Structural,
            Payload::None { .. } => RenderSource::KeyOnly,
        }
    }
}

/// Everything one rendering is built from.
#[derive(Debug, Clone)]
pub struct Draft<'a> {
    pub rule: &'a str,
    pub state: CondState,
    pub prior: Option<CondState>,
    pub severity: &'a str,
    pub evidence: &'a str,
    pub labels: &'a BTreeMap<String, String>,
    pub key: Option<&'a str>,
    /// The producer's (HLC) timestamp, when the sample carried one.
    pub timestamp: Option<&'a str>,
    pub payload: &'a Payload,
}

/// The rendered notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    /// `<severity> <rule> <state>`.
    pub title: String,
    pub message: String,
    /// The message was cut at the bound.
    pub truncated: bool,
    pub source: RenderSource,
}

/// The wire word for a state.
pub fn state_word(s: CondState) -> &'static str {
    match s {
        CondState::Ok => "ok",
        CondState::Firing => "firing",
        CondState::Unobservable => "unobservable",
    }
}

/// The suffix a cut message ends with.
pub const TRUNCATED: &str = "… [truncated]";

/// Cut `s` to at most `max` bytes on a char boundary, marking the cut.
/// Returns the text and whether anything was cut.
pub fn truncate_bytes(s: &str, max: usize) -> (String, bool) {
    if s.len() <= max {
        return (s.to_string(), false);
    }
    let room = max.saturating_sub(TRUNCATED.len());
    let mut end = room.min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (format!("{}{TRUNCATED}", &s[..end]), true)
}

/// Render one draft under the bounds.
pub fn render(d: &Draft<'_>, cfg: &RenderConfig) -> Rendered {
    let title = format!("{} {} {}", d.severity, d.rule, state_word(d.state));
    let mut lines: Vec<String> = Vec::new();
    lines.push(match d.prior {
        Some(p) => format!("state: {} (was: {})", state_word(d.state), state_word(p)),
        None => format!("state: {} (first observation)", state_word(d.state)),
    });
    if !d.evidence.is_empty() {
        lines.push(d.evidence.to_string());
    }
    if !d.labels.is_empty() {
        let kv: Vec<String> = d.labels.iter().map(|(k, v)| format!("{k}={v}")).collect();
        lines.push(format!("labels: {}", kv.join(" ")));
    }
    if let Some(k) = d.key {
        lines.push(format!("key: {k}"));
    }
    if let Some(t) = d.timestamp {
        lines.push(format!("timestamp: {t}"));
    }
    let mut truncated = false;
    match d.payload {
        Payload::Typed(v) => {
            lines.push(format!(
                "payload: {}",
                serde_json::to_string(v).unwrap_or_default()
            ));
        }
        Payload::Structural { text, what } => {
            let (cut, was_cut) = truncate_bytes(text, cfg.max_structural_bytes);
            truncated |= was_cut;
            lines.push(format!("no served schema for {what}: structural rendering"));
            lines.push(cut);
        }
        Payload::None { reason } => lines.push(format!("payload not rendered: {reason}")),
    }
    let (message, was_cut) = truncate_bytes(&lines.join("\n"), cfg.max_message_bytes);
    Rendered {
        title,
        message,
        truncated: truncated || was_cut,
        source: d.payload.source(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("team".to_string(), "infra".to_string()),
            ("port".to_string(), "eth0".to_string()),
        ])
    }

    fn draft<'a>(payload: &'a Payload, labels: &'a BTreeMap<String, String>) -> Draft<'a> {
        Draft {
            rule: "fleet-alerts",
            state: CondState::Firing,
            prior: Some(CondState::Ok),
            severity: "warning",
            evidence: "alert h-3fa9c2d41b7e.netlink.a659f813308ad1da firing",
            labels,
            key: Some("v1/h-3fa9c2d41b7e/state/netlink/alert/a659f813308ad1da"),
            timestamp: Some("7/1"),
            payload,
        }
    }

    /// The three sources, each said in the message, and the title's shape.
    #[test]
    fn the_three_sources_render_and_say_which_they_are() {
        let l = labels();
        let cfg = RenderConfig::default();
        let typed = Payload::Typed(serde_json::json!({"severity": "warning"}));
        let r = render(&draft(&typed, &l), &cfg);
        assert_eq!(r.title, "warning fleet-alerts firing");
        assert_eq!(r.source, RenderSource::Schema);
        assert!(
            r.message.starts_with("state: firing (was: ok)\n"),
            "{}",
            r.message
        );
        assert!(
            r.message.contains("labels: port=eth0 team=infra"),
            "sorted k=v"
        );
        assert!(
            r.message
                .contains("key: v1/h-3fa9c2d41b7e/state/netlink/alert/a659f813308ad1da")
        );
        assert!(r.message.contains("timestamp: 7/1"));
        assert!(
            r.message.ends_with("payload: {\"severity\":\"warning\"}"),
            "{}",
            r.message
        );
        assert!(!r.truncated);

        let structural = Payload::Structural {
            text: "{\"a\":1}".into(),
            what: "Alert".into(),
        };
        let r = render(&draft(&structural, &l), &cfg);
        assert_eq!(r.source, RenderSource::Structural);
        assert!(
            r.message
                .contains("no served schema for Alert: structural rendering\n{\"a\":1}"),
            "{}",
            r.message
        );

        let none = Payload::None {
            reason: "a tombstone carries no payload".into(),
        };
        let r = render(&draft(&none, &l), &cfg);
        assert_eq!(r.source, RenderSource::KeyOnly);
        assert!(
            r.message
                .ends_with("payload not rendered: a tombstone carries no payload")
        );
    }

    /// The bounds bite: a big structural reading is cut at its own bound and
    /// a big message at the message bound, both on a char boundary, both
    /// flagged.
    #[test]
    fn rendering_is_bounded_and_says_so() {
        let l = labels();
        let big = Payload::Structural {
            text: "é".repeat(600),
            what: "k".into(),
        };
        let cfg = RenderConfig {
            max_message_bytes: 2048,
            max_structural_bytes: 100,
        };
        let r = render(&draft(&big, &l), &cfg);
        assert!(r.truncated);
        assert!(r.message.ends_with(TRUNCATED));
        assert!(r.message.len() < 400);

        let typed = Payload::Typed(serde_json::json!({"blob": "x".repeat(4096)}));
        let cfg = RenderConfig {
            max_message_bytes: 256,
            max_structural_bytes: 512,
        };
        let r = render(&draft(&typed, &l), &cfg);
        assert!(r.truncated);
        assert!(r.message.len() <= 256, "{}", r.message.len());
        assert!(r.message.ends_with(TRUNCATED));

        // Under the bound, nothing is touched.
        let (s, cut) = truncate_bytes("short", 100);
        assert_eq!((s.as_str(), cut), ("short", false));
    }

    /// A first observation says so rather than inventing a prior (O4).
    #[test]
    fn a_first_observation_states_the_baseline() {
        let l = BTreeMap::new();
        let none = Payload::None { reason: "r".into() };
        let mut d = draft(&none, &l);
        d.prior = None;
        d.state = CondState::Unobservable;
        let r = render(&d, &RenderConfig::default());
        assert!(
            r.message
                .starts_with("state: unobservable (first observation)")
        );
        assert!(!r.message.contains("labels:"));
    }
}
