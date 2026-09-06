//! Sinks: where a notification goes.
//!
//! One vocabulary ([`SinkKind`]), four deliveries — [`ntfy`], [`smtp`],
//! [`webhook`], and [`exec`], deliberately last in every list because it is
//! the one that runs code — plus [`dry_run`], which is what every sink
//! becomes under `--dry-run`. A sink takes an [`Outgoing`] and answers
//! delivered or not; it never decides *whether* to notify. That is the
//! engine's, and keeping it out of here is what lets `test-sink` push one
//! synthetic notification through a real sink.
//!
//! [`Notification`] and [`Outgoing`] are **this daemon's wire**: the JSON a
//! webhook receives and an `exec` program reads on stdin. They are pinned
//! here, beside the sinks that emit them, and not in the engine — the engine
//! never sees them.

pub mod dry_run;
pub mod exec;
pub mod http;
pub mod ntfy;
pub mod smtp;
pub mod webhook;

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Serialize;
use zenkey_fleet::{CondState, RenderSource};

use crate::config::{Config, SinkConfig};

/// One notification — the unit a sink delivers.
///
/// `state`/`prior` are the RFC 13 three states, never two: `unobservable`
/// rides through to the sink payload distinguishable from `ok`, because a
/// consumer that folds them has turned "I could not tell" into "fine".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Notification {
    /// `<rule_id>-<n>`, unique within one process run.
    pub id: String,
    /// The rule's name, as configured.
    pub rule: String,
    /// The rule's head word (`silent-for`, `alerts`, `liveliness-gone`, …).
    pub kind: String,
    pub state: CondState,
    /// `null` on the first observation — the baseline stated, not invented (O4).
    pub prior: Option<CondState>,
    pub severity: String,
    pub title: String,
    pub message: String,
    /// The rule's labels, with an alert document's own on top.
    pub labels: BTreeMap<String, String>,
    /// RFC 3339 wall clock.
    pub at: String,
    /// The one-line why.
    pub evidence: String,
    pub rendering: RenderSource,
    pub truncated: bool,
}

/// A notification and the sinks it is routed to. Every named sink gets the
/// same `notification`; a sink's delivery is one per `Outgoing`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Outgoing {
    pub notification: Notification,
    pub sinks: Vec<String>,
}

/// A delivery that went out, with what the far side said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub detail: String,
}

/// A delivery that did not.
#[derive(Debug, thiserror::Error)]
pub enum SinkError {
    #[error("HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("transport: {0}")]
    Transport(String),
    #[error("timed out after {0:?}")]
    Timeout(Duration),
    #[error("{program} exited {status}: {stderr}")]
    Exec {
        program: String,
        status: String,
        stderr: String,
    },
    #[error("smtp: {0}")]
    Smtp(String),
    #[error("probe: {0}")]
    Probe(String),
}

/// The four kinds, and the fifth every one of them becomes under `--dry-run`.
#[derive(Debug)]
pub enum SinkKind {
    Ntfy(ntfy::Ntfy),
    Webhook(webhook::Webhook),
    Smtp(smtp::Smtp),
    Exec(exec::Exec),
    DryRun(dry_run::DryRun),
}

/// A configured sink, ready to deliver.
#[derive(Debug)]
pub struct Sink {
    pub name: String,
    pub kind: SinkKind,
    /// The per-delivery bound; a delivery past it is a [`SinkError::Timeout`].
    pub timeout: Duration,
}

/// Why a sink could not be built from its config — a secret that did not
/// resolve, an address the transport refused. [`crate::config::check`] has
/// already caught what it can; this is the transport's own refusal.
#[derive(Debug, thiserror::Error)]
#[error("sink {name}: {message}")]
pub struct BuildError {
    pub name: String,
    pub message: String,
}

impl Sink {
    /// Build one sink, resolving its secrets **now** — a credential that
    /// cannot be read is a startup refusal, not a 3am delivery failure.
    pub fn build(name: &str, cfg: &SinkConfig, http: &reqwest::Client) -> Result<Sink, BuildError> {
        let fail = |message: String| BuildError {
            name: name.to_string(),
            message,
        };
        let kind = match cfg {
            SinkConfig::Ntfy { .. } => SinkKind::Ntfy(ntfy::Ntfy::build(cfg, http).map_err(fail)?),
            SinkConfig::Webhook { .. } => {
                SinkKind::Webhook(webhook::Webhook::build(cfg, http).map_err(fail)?)
            }
            SinkConfig::Exec { .. } => SinkKind::Exec(exec::Exec::build(cfg).map_err(fail)?),
            SinkConfig::Smtp { .. } => SinkKind::Smtp(smtp::Smtp::build(cfg).map_err(fail)?),
        };
        Ok(Sink {
            name: name.to_string(),
            kind,
            timeout: cfg.timeout(),
        })
    }

    /// The `--dry-run` stand-in for a configured sink: prints, sends nothing.
    pub fn dry_run(name: &str) -> Sink {
        Sink {
            name: name.to_string(),
            kind: SinkKind::DryRun(dry_run::DryRun::stdout(name)),
            timeout: Duration::from_secs(5),
        }
    }

    /// A dry-run sink that captures into `into` instead of printing — the
    /// test seam.
    pub fn capturing(name: &str, into: dry_run::Captured) -> Sink {
        Sink {
            name: name.to_string(),
            kind: SinkKind::DryRun(dry_run::DryRun::capturing(name, into)),
            timeout: Duration::from_secs(5),
        }
    }

    /// The kind word, for logs.
    pub fn kind_name(&self) -> &'static str {
        match &self.kind {
            SinkKind::Ntfy(_) => "ntfy",
            SinkKind::Webhook(_) => "webhook",
            SinkKind::Smtp(_) => "smtp",
            SinkKind::Exec(_) => "exec",
            SinkKind::DryRun(_) => "dry-run",
        }
    }

    /// Deliver one, within [`Sink::timeout`].
    pub async fn deliver(&self, o: &Outgoing) -> Result<Delivery, SinkError> {
        let fut = async {
            match &self.kind {
                SinkKind::Ntfy(s) => s.deliver(o).await,
                SinkKind::Webhook(s) => s.deliver(o).await,
                SinkKind::Smtp(s) => s.deliver(o).await,
                SinkKind::Exec(s) => s.deliver(o, self.timeout).await,
                SinkKind::DryRun(s) => s.deliver(o),
            }
        };
        match tokio::time::timeout(self.timeout, fut).await {
            Ok(r) => r,
            Err(_) => Err(SinkError::Timeout(self.timeout)),
        }
    }

    /// Ask the far side whether it is there without sending anything: an
    /// SMTP `test_connection`, a program on the path, a URL that parses.
    pub async fn probe(&self) -> Result<String, SinkError> {
        let fut = async {
            match &self.kind {
                SinkKind::Ntfy(s) => s.probe(),
                SinkKind::Webhook(s) => s.probe(),
                SinkKind::Smtp(s) => s.probe().await,
                SinkKind::Exec(s) => s.probe(),
                SinkKind::DryRun(_) => Ok("dry-run: nothing to probe".into()),
            }
        };
        match tokio::time::timeout(self.timeout, fut).await {
            Ok(r) => r,
            Err(_) => Err(SinkError::Timeout(self.timeout)),
        }
    }
}

/// Every configured sink, built — or every one swapped for a dry-run.
pub fn build_all(cfg: &Config, dry_run: bool) -> Result<Vec<Sink>, BuildError> {
    if dry_run {
        return Ok(cfg.sinks.keys().map(|n| Sink::dry_run(n)).collect());
    }
    let http = http::client();
    cfg.sinks
        .iter()
        .map(|(name, sc)| Sink::build(name, sc, &http))
        .collect()
}

/// The synthetic notification `test-sink` pushes: unmistakably a test, and
/// shaped exactly like a real one so a webhook consumer sees the contract.
pub fn test_outgoing(sink: &str) -> Outgoing {
    Outgoing {
        notification: Notification {
            id: "test-sink-0".into(),
            rule: "test-sink".into(),
            kind: "test".into(),
            state: CondState::Ok,
            prior: None,
            severity: "info".into(),
            title: format!("info test-sink ok ({sink})"),
            message: format!(
                "state: ok (first observation)\nzenwatch test-sink {sink}: this is a test \
                 delivery; nothing on the bus produced it"
            ),
            labels: BTreeMap::new(),
            at: zenkey_fleet::rfc3339_now(),
            evidence: "zenwatch test-sink — a synthetic notification".into(),
            rendering: RenderSource::KeyOnly,
            truncated: false,
        },
        sinks: vec![sink.to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The webhook/exec wire shape: field names, snake_case states, `null`
    /// prior on a baseline, `unobservable` distinguishable from `ok`.
    #[test]
    fn outgoing_json_shape_is_pinned() {
        let o = Outgoing {
            notification: Notification {
                id: "hosts-gone-1".into(),
                rule: "hosts-gone".into(),
                kind: "liveliness-gone".into(),
                state: CondState::Unobservable,
                prior: None,
                severity: "error".into(),
                title: "error hosts-gone unobservable".into(),
                message: "state: unobservable (first observation)\nthe observer dropped 3".into(),
                labels: BTreeMap::from([("team".to_string(), "infra".to_string())]),
                at: "2026-09-06T00:00:00Z".into(),
                evidence: "the observer dropped 3".into(),
                rendering: RenderSource::KeyOnly,
                truncated: false,
            },
            sinks: vec!["ops".into(), "mail".into()],
        };
        assert_eq!(
            serde_json::to_value(&o).unwrap(),
            serde_json::json!({
                "notification": {
                    "id": "hosts-gone-1",
                    "rule": "hosts-gone",
                    "kind": "liveliness-gone",
                    "state": "unobservable",
                    "prior": null,
                    "severity": "error",
                    "title": "error hosts-gone unobservable",
                    "message": "state: unobservable (first observation)\nthe observer dropped 3",
                    "labels": {"team": "infra"},
                    "at": "2026-09-06T00:00:00Z",
                    "evidence": "the observer dropped 3",
                    "rendering": "key_only",
                    "truncated": false,
                },
                "sinks": ["ops", "mail"],
            })
        );
        let mut firing = o.clone();
        firing.notification.state = CondState::Firing;
        firing.notification.prior = Some(CondState::Ok);
        let j = serde_json::to_value(&firing).unwrap();
        assert_eq!(j["notification"]["state"], "firing");
        assert_eq!(j["notification"]["prior"], "ok");
    }
}
