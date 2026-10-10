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
//! never sees them. What they carry of the engine's is the engine's own
//! pinned shape: a presence notification's [`PresenceTransition`]
//! (`zenkey_fleet::report`, #614 FL2).

pub mod dry_run;
pub mod exec;
pub mod http;
pub mod ntfy;
pub mod smtp;
pub mod webhook;

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use zenkey_fleet::{CondState, PresenceTransition};

use crate::config::{Config, SinkConfig};
use crate::render::RenderSource;

/// What a notification *is* (#389) — the discipline's verdict on a
/// notice, and the first thing a sink consumer switches on.
///
/// The base kinds are what a rule saw; the rest are what the discipline
/// made of it. A resolve is one of three, kept apart because a phone must
/// be able to tell "it is fixed" from "I can see it again": `firing → ok`
/// is [`Resolved`](NoticeKind::Resolved), `unobservable → ok` is
/// [`ObservableAgain`](NoticeKind::ObservableAgain), `firing →
/// unobservable` is [`LostSight`](NoticeKind::LostSight) and `ok →
/// unobservable` is [`Unobservable`](NoticeKind::Unobservable).
///
/// `alert` left with the dark `alerts` rule (#613); it returns with
/// zk2's alarm profile, deliberately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    /// An engine condition changed state.
    Transition,
    /// An address went down (`instance-gone`, #614): no instance token
    /// visible to this reader, a complete read behind the claim.
    Presence,
    /// A liveliness token went (`liveliness-gone`).
    Liveliness,
    /// A presence rule's first look (#614, FL2): what is down at start,
    /// what could not be seen — said once, the addresses down remembered
    /// rather than paged one by one, as the doctor's baseline is.
    Baseline,
    /// Several notices in one window from one group key, folded into one.
    Group,
    /// `firing → ok`.
    Resolved,
    /// `unobservable → ok`.
    ObservableAgain,
    /// `firing → unobservable`.
    LostSight,
    /// `ok → unobservable` — the observer could not tell, and says so.
    Unobservable,
    /// The scheduled doctor (#390): the baseline, or a finding new since
    /// the last run. A finding gone since the last run is a
    /// [`Resolved`](NoticeKind::Resolved); a run that could not happen is
    /// [`Unobservable`](NoticeKind::Unobservable), and the run after it
    /// [`ObservableAgain`](NoticeKind::ObservableAgain).
    Doctor,
}

impl NoticeKind {
    /// The wire word.
    pub fn as_str(self) -> &'static str {
        match self {
            NoticeKind::Transition => "transition",
            NoticeKind::Presence => "presence",
            NoticeKind::Liveliness => "liveliness",
            NoticeKind::Baseline => "baseline",
            NoticeKind::Group => "group",
            NoticeKind::Resolved => "resolved",
            NoticeKind::ObservableAgain => "observable_again",
            NoticeKind::LostSight => "lost_sight",
            NoticeKind::Unobservable => "unobservable",
            NoticeKind::Doctor => "doctor",
        }
    }
}

/// One notification — the unit a sink delivers.
///
/// `state`/`prior` are the RFC 13 three states, never two: `unobservable`
/// rides through to the sink payload distinguishable from `ok`, because a
/// consumer that folds them has turned "I could not tell" into "fine".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Notification {
    /// The notice's **identity** (#389), stable across runs and restarts:
    /// `<rule_id>:<what>` — the address for `instance-gone`, the token's
    /// wire key (or, on zk2 presence tokens, the address and what the
    /// token names) for `liveliness-gone`, `observer` for a presence rule's
    /// own sight, the sorted labels for an engine condition,
    /// `<check>:<subject>` under `doctor`. Dedup, repeat and the state file
    /// key on it.
    pub id: String,
    /// The rule's name, as configured.
    pub rule: String,
    /// The rule's head word (`silent-for`, `instance-gone`,
    /// `liveliness-gone`, …).
    pub rule_kind: String,
    /// What this notification is (#389).
    pub kind: NoticeKind,
    pub state: CondState,
    /// `null` on the first observation — the baseline stated, not invented (O4).
    pub prior: Option<CondState>,
    pub severity: String,
    pub title: String,
    pub message: String,
    /// The rule's labels.
    pub labels: BTreeMap<String, String>,
    /// RFC 3339 wall clock.
    pub at: String,
    /// The one-line why.
    pub evidence: String,
    pub rendering: RenderSource,
    pub truncated: bool,
    /// How many times this still-firing notice has been re-sent on the
    /// repeat interval; `0` on the first announcement.
    pub repeat: u32,
    /// For a [`NoticeKind::Group`]: the member notice ids, in order.
    pub group: Option<Vec<String>>,
    /// The presence move that decided a notice read from the live
    /// projection (#614, FL2) — an address's status, an interface or member
    /// token — as the engine pins it (`zenkey_fleet::report`): `at`,
    /// `source` (`event` | `seed` | `dropped`), `address`, and `change`
    /// tagged by `kind`. `null` for everything else.
    pub presence: Option<PresenceTransition>,
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
            id: "test-sink:synthetic".into(),
            rule: "test-sink".into(),
            rule_kind: "test".into(),
            kind: NoticeKind::Transition,
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
            repeat: 0,
            group: None,
            presence: None,
        },
        sinks: vec![sink.to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey_fleet::{AddressStatus, PresenceChange, PresenceSource};

    /// The webhook/exec wire shape: field names, snake_case states, `null`
    /// prior on a baseline, `unobservable` distinguishable from `ok`, and a
    /// presence notification's transition in the engine's pinned shape.
    #[test]
    fn outgoing_json_shape_is_pinned() {
        let o = Outgoing {
            notification: Notification {
                id: "tc-gone:observer".into(),
                rule: "tc-gone".into(),
                rule_kind: "instance-gone".into(),
                kind: NoticeKind::Unobservable,
                state: CondState::Unobservable,
                prior: None,
                severity: "error".into(),
                title: "error tc-gone unobservable".into(),
                message: "state: unobservable (first observation)\nthe projection lost 3".into(),
                labels: BTreeMap::from([("team".to_string(), "infra".to_string())]),
                at: "2026-09-06T00:00:00Z".into(),
                evidence: "the projection lost 3".into(),
                rendering: RenderSource::KeyOnly,
                truncated: false,
                repeat: 0,
                group: None,
                presence: None,
            },
            sinks: vec!["ops".into(), "mail".into()],
        };
        assert_eq!(
            serde_json::to_value(&o).unwrap(),
            serde_json::json!({
                "notification": {
                    "id": "tc-gone:observer",
                    "rule": "tc-gone",
                    "rule_kind": "instance-gone",
                    "kind": "unobservable",
                    "state": "unobservable",
                    "prior": null,
                    "severity": "error",
                    "title": "error tc-gone unobservable",
                    "message": "state: unobservable (first observation)\nthe projection lost 3",
                    "labels": {"team": "infra"},
                    "at": "2026-09-06T00:00:00Z",
                    "evidence": "the projection lost 3",
                    "rendering": "key_only",
                    "truncated": false,
                    "repeat": 0,
                    "group": null,
                    "presence": null,
                },
                "sinks": ["ops", "mail"],
            })
        );
        let mut firing = o.clone();
        firing.notification.id = "tc-gone:host-a/tc".into();
        firing.notification.state = CondState::Firing;
        firing.notification.prior = Some(CondState::Ok);
        firing.notification.kind = NoticeKind::Presence;
        firing.notification.repeat = 2;
        firing.notification.presence = Some(PresenceTransition {
            at: std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_791_331_200),
            source: PresenceSource::Event,
            address: "host-a/tc".parse().unwrap(),
            change: PresenceChange::Status {
                from: AddressStatus::Up,
                to: AddressStatus::Down,
            },
        });
        let j = serde_json::to_value(&firing).unwrap();
        assert_eq!(j["notification"]["state"], "firing");
        assert_eq!(j["notification"]["prior"], "ok");
        assert_eq!(j["notification"]["kind"], "presence");
        assert_eq!(j["notification"]["repeat"], 2);
        assert_eq!(
            j["notification"]["presence"],
            serde_json::json!({
                "at": "2026-10-07T00:00:00Z",
                "source": "event",
                "address": "host-a/tc",
                "change": {"kind": "status", "from": "up", "to": "down"},
            })
        );
        assert!(
            j["notification"].get("inhibited_by").is_none(),
            "inhibition was removed (#614, FL2)"
        );
        // Every kind spells itself in snake_case, the way a consumer
        // switches on it.
        for k in [
            NoticeKind::Transition,
            NoticeKind::Presence,
            NoticeKind::Liveliness,
            NoticeKind::Baseline,
            NoticeKind::Group,
            NoticeKind::Resolved,
            NoticeKind::ObservableAgain,
            NoticeKind::LostSight,
            NoticeKind::Unobservable,
            NoticeKind::Doctor,
        ] {
            assert_eq!(serde_json::to_value(k).unwrap(), k.as_str());
        }
    }
}
