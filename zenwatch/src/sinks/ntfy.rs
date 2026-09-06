//! The ntfy sink: `POST {url}/{topic}`, body = message, the rest in headers.
//!
//! Priority is mapped from the notification's severity — and from its
//! *state*: a notification whose state is `ok` is a resolve and takes the
//! `resolved` priority, whatever the rule's severity, because a phone that
//! buzzes as hard for "back to normal" as for "down" trains its owner to
//! ignore it. `unobservable` has its own key for the same reason.

use std::collections::BTreeMap;

use super::{Delivery, Outgoing, SinkError};
use crate::config::SinkConfig;
use zenkey_fleet::CondState;

/// The priority for a severity word the config does not map.
pub const DEFAULT_PRIORITY: u8 = 3;

/// ntfy's five, as its docs name them.
pub const DEFAULT_PRIORITIES: [(&str, u8); 6] = [
    ("critical", 5),
    ("error", 4),
    ("warning", 3),
    ("info", 2),
    ("resolved", 1),
    ("unobservable", 3),
];

#[derive(Debug)]
pub struct Ntfy {
    client: reqwest::Client,
    /// `{url}/{topic}`, joined once.
    endpoint: String,
    token: Option<String>,
    priority: BTreeMap<String, u8>,
    tags: Vec<String>,
}

/// The priority key a notification maps through: `resolved` when its state
/// is `ok`, `unobservable` when it could not tell, its severity otherwise.
pub fn priority_key(state: CondState, severity: &str) -> &str {
    match state {
        CondState::Ok => "resolved",
        CondState::Unobservable => "unobservable",
        CondState::Firing => severity,
    }
}

/// Map a notification to 1–5: the config's table, then the defaults, then
/// [`DEFAULT_PRIORITY`].
pub fn priority(map: &BTreeMap<String, u8>, state: CondState, severity: &str) -> u8 {
    let key = priority_key(state, severity);
    map.get(key)
        .copied()
        .or_else(|| {
            DEFAULT_PRIORITIES
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, p)| *p)
        })
        .unwrap_or(DEFAULT_PRIORITY)
}

/// The `Tags` header: the config's tags, the severity, the state.
pub fn tags(configured: &[String], state: CondState, severity: &str) -> String {
    let mut all: Vec<String> = configured.to_vec();
    all.push(severity.to_string());
    all.push(crate::render::state_word(state).to_string());
    all.join(",")
}

impl Ntfy {
    pub fn build(cfg: &SinkConfig, client: &reqwest::Client) -> Result<Ntfy, String> {
        let SinkConfig::Ntfy {
            url,
            topic,
            token,
            priority,
            tags,
            ..
        } = cfg
        else {
            return Err("not an ntfy config".into());
        };
        let token = token
            .as_ref()
            .map(|s| s.resolve())
            .transpose()
            .map_err(|e| e.to_string())?;
        Ok(Ntfy {
            client: client.clone(),
            endpoint: format!("{}/{topic}", url.trim_end_matches('/')),
            token,
            priority: priority.clone(),
            tags: tags.clone(),
        })
    }

    /// The headers one notification rides with — pure, for the test.
    pub fn headers(&self, o: &Outgoing) -> Vec<(&'static str, String)> {
        let n = &o.notification;
        let mut h = vec![
            ("Title", super::http::header_safe(&n.title)),
            (
                "Priority",
                priority(&self.priority, n.state, &n.severity).to_string(),
            ),
            (
                "Tags",
                super::http::header_safe(&tags(&self.tags, n.state, &n.severity)),
            ),
        ];
        if let Some(t) = &self.token {
            h.push(("Authorization", format!("Bearer {t}")));
        }
        h
    }

    pub async fn deliver(&self, o: &Outgoing) -> Result<Delivery, SinkError> {
        let mut req = self
            .client
            .request(reqwest::Method::POST, &self.endpoint)
            .header("Content-Type", "text/plain; charset=utf-8");
        for (k, v) in self.headers(o) {
            req = req.header(k, v);
        }
        let resp = req
            .body(o.notification.message.clone())
            .send()
            .await
            .map_err(|e| SinkError::Transport(e.to_string()))?;
        let status = resp.status();
        if status.is_success() {
            Ok(Delivery {
                detail: format!("HTTP {status}"),
            })
        } else {
            Err(SinkError::Http {
                status: status.as_u16(),
                body: super::http::body_head(resp, 256).await,
            })
        }
    }

    /// Nothing to send without sending: the endpoint parses, and that is all
    /// a probe can say about a push service.
    pub fn probe(&self) -> Result<String, SinkError> {
        reqwest::Url::parse(&self.endpoint)
            .map(|u| format!("endpoint {u} parses; ntfy has no side-effect-free probe"))
            .map_err(|e| SinkError::Probe(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The header mapping: severity → priority through the config table, the
    /// resolve and unobservable keys overriding severity, tags carrying both.
    #[test]
    fn priority_follows_state_then_severity() {
        let map = BTreeMap::from([
            ("error".to_string(), 5u8),
            ("warning".to_string(), 4),
            ("info".to_string(), 2),
            ("resolved".to_string(), 1),
        ]);
        assert_eq!(priority(&map, CondState::Firing, "error"), 5);
        assert_eq!(priority(&map, CondState::Firing, "warning"), 4);
        assert_eq!(
            priority(&map, CondState::Ok, "error"),
            1,
            "a resolve is a resolve"
        );
        assert_eq!(
            priority(&map, CondState::Unobservable, "error"),
            3,
            "unobservable takes its default when unmapped"
        );
        assert_eq!(
            priority(&map, CondState::Firing, "custom"),
            DEFAULT_PRIORITY
        );
        assert_eq!(priority(&BTreeMap::new(), CondState::Firing, "critical"), 5);
        assert_eq!(
            tags(&["zenoh".to_string()], CondState::Firing, "warning"),
            "zenoh,warning,firing"
        );
    }
}
