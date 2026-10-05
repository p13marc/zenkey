//! The Config tool's model (#481): what a form addresses, what came back,
//! and how a reply is read — pure, so the pane, its tests and the update
//! handler share one reading of the RFC 05 §5.1 convention.
//!
//! **Named `configure`**, not `config`: `crate::config` is the command line
//! and the launch settings.
//!
//! The tool renders only what the producer serves. The schema is the
//! read-back's (RFC 05 §5.1: "the schema is served, not documented"), the
//! values are the read-back's, and a reply that is not a read-back is shown
//! as what it is rather than coerced into one — so the tool is as useful
//! against a producer this build has never heard of as against the double.

use std::sync::Arc;
use std::time::Instant;

use zenkey::config::{ConfigView, ParamClass, ParamKind};
use zenkey_fleet::SliceSet;
use zenkey_fleet::report::{CallOutcome, CallReport};

use crate::services::ServiceError;

/// One configuration resource: an origin, a producer, a resource — the
/// three chunks every RFC 05 §5.1 key names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigTarget {
    pub origin: String,
    pub producer: String,
    pub resource: String,
}

impl ConfigTarget {
    pub fn new(
        origin: impl Into<String>,
        producer: impl Into<String>,
        resource: impl Into<String>,
    ) -> Self {
        ConfigTarget {
            origin: origin.into(),
            producer: producer.into(),
            resource: resource.into(),
        }
    }

    /// Why this target cannot be asked yet, if it cannot: the first empty
    /// field, or a fleet origin — configuration is one host's (RFC 05 §2.1:
    /// a write never fans out, and the form that reads is the form that
    /// writes).
    pub fn unaskable(&self) -> Option<&'static str> {
        if self.origin.trim().is_empty() {
            Some("name the origin — one host id")
        } else if self.origin.trim() == "*" {
            Some("a configuration is one origin's: `*` would fan a write out (RFC 05 §2.1)")
        } else if self.producer.trim().is_empty() {
            Some("name the producer")
        } else if self.resource.trim().is_empty() {
            Some("name the resource — the chunk the producer configures by")
        } else {
            None
        }
    }

    /// The read procedure, under `@rpc/<producer>/`.
    pub fn read_path(&self) -> String {
        format!("config/{}", self.resource.trim())
    }
}

/// What one ask came back as.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    /// The read-back document (RFC 05 §5.1).
    Document(Box<ConfigView>),
    /// The producer refused: its error name and its words, verbatim.
    Refused { name: String, message: String },
    /// Nothing answered within the timeout — a non-verdict (RFC 05 §3.1).
    Silent,
    /// An answer that is not a read-back, as it arrived.
    Other(String),
    /// The call never left: a refused input, a session error.
    Failed(String),
}

/// Read a call's report as one origin's reply. A form addresses one origin,
/// so the first answer is the answer; more than one would be a fan-out the
/// target refused before the call left.
pub fn classify(r: &Result<Arc<CallReport>, ServiceError>) -> Reply {
    let report = match r {
        Ok(report) => report,
        Err(e) => return Reply::Failed(e.one_line()),
    };
    let Some(answer) = report.answers.first() else {
        return Reply::Silent;
    };
    match &answer.outcome {
        CallOutcome::Err(e) => Reply::Refused {
            name: e.name.clone(),
            message: e.message.clone(),
        },
        CallOutcome::Ok {
            value: Some(v),
            text,
        } => match serde_json::from_value::<ConfigView>(v.clone()) {
            Ok(view) => Reply::Document(Box::new(view)),
            Err(_) => Reply::Other(text.clone().unwrap_or_else(|| v.to_string())),
        },
        CallOutcome::Ok { value: None, text } => Reply::Other(
            text.clone()
                .unwrap_or_else(|| "(an empty reply)".to_string()),
        ),
    }
}

/// A landed read: the reply, the target it was for, and when it landed.
#[derive(Debug, Clone)]
pub struct ConfigRead {
    pub reply: Reply,
    pub target: ConfigTarget,
    pub at: Instant,
}

/// What the form is waiting on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigAct {
    Read,
}

/// The Config tool's state (#481), in the workbench: what the user typed
/// and asked. A base switch owes it nothing — and when a change is pending
/// (#481's later chunks) dropping the form would drop the only surface that
/// knows a rollback window is running.
#[derive(Debug, Default)]
pub struct ConfigForm {
    pub target: ConfigTarget,
    /// The last read; `None` = never asked (O4: not "nothing configured").
    pub read: Option<ConfigRead>,
    /// A call in flight, by act.
    pub in_flight: Option<ConfigAct>,
}

impl ConfigForm {
    /// The document on screen, if the last read was one.
    pub fn view(&self) -> Option<&ConfigView> {
        match &self.read.as_ref()?.reply {
            Reply::Document(v) => Some(v),
            _ => None,
        }
    }

    /// Whether the target was edited since the last read — the document on
    /// screen is then about another resource, and says so.
    pub fn stale(&self) -> bool {
        self.read.as_ref().is_some_and(|r| r.target != self.target)
    }
}

/// Producers whose slice declares a `config/…` procedure — the picker's
/// offer. Empty with no registry, and the producer is then typed: a slice
/// is a convenience here, never a gate.
pub fn producers(slices: Option<&SliceSet>) -> Vec<String> {
    let Some(slices) = slices else {
        return Vec::new();
    };
    let mut out: Vec<String> = slices
        .slices()
        .iter()
        .filter(|s| s.procedures.iter().any(|p| p.path.starts_with("config/")))
        .map(|s| s.name.clone())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Origins the roster has seen running `producer` — any producer when none
/// is named. Liveliness-fed, so an origin that never announced is typed.
pub fn origins(roster: &crate::nodes::NodeRoster, producer: &str) -> Vec<String> {
    roster
        .iter()
        .filter(|(_, producers)| producer.is_empty() || producers.contains_key(producer))
        .map(|(origin, _)| origin.clone())
        .collect()
}

/// A class as the form says it: the word, and what it means for a change.
pub fn class_words(class: ParamClass) -> &'static str {
    match class {
        ParamClass::Hot => "hot — applies at once",
        ParamClass::Reach => "reach — can cut the link",
        ParamClass::Contract => "contract — restart required",
        // `#[non_exhaustive]`: a fourth class arrives with the producer
        // that needs it, and this build says it does not know it.
        _ => "unknown class — read-only here",
    }
}

/// The kind as a person reads it: `bool`, `integer 0..=30 dBm`, `text` —
/// zenctl's spelling.
pub fn kind_text(kind: &ParamKind) -> String {
    match kind {
        ParamKind::Bool => "bool".to_string(),
        ParamKind::Integer { min, max, unit } => {
            let mut s = "integer".to_string();
            match (min, max) {
                (Some(lo), Some(hi)) => s.push_str(&format!(" {lo}..={hi}")),
                (Some(lo), None) => s.push_str(&format!(" {lo}..")),
                (None, Some(hi)) => s.push_str(&format!(" ..={hi}")),
                (None, None) => {}
            }
            if let Some(u) = unit {
                s.push(' ');
                s.push_str(u);
            }
            s
        }
        ParamKind::Text => "text".to_string(),
        _ => "unknown kind".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey_fleet::report::{CallAnswer, CallError};

    fn report(outcome: Option<CallOutcome>) -> Result<Arc<CallReport>, ServiceError> {
        Ok(Arc::new(CallReport {
            key: "v1/h-a/@rpc/radio/config/wlan0".into(),
            timeout_s: 5.0,
            answers: outcome
                .into_iter()
                .map(|outcome| CallAnswer {
                    origin: "h-a".into(),
                    outcome,
                    attachment: None,
                    attachment_bytes: None,
                })
                .collect(),
        }))
    }

    /// Four replies, four readings — and silence is its own (RFC 05 §3.1),
    /// never an empty document.
    #[test]
    fn a_reply_is_read_as_what_it_is() {
        let view = ConfigView::of("wlan0", &zenkey::config::ConfigSchema::new());
        let doc = report(Some(CallOutcome::Ok {
            value: Some(serde_json::to_value(&view).unwrap()),
            text: None,
        }));
        assert_eq!(classify(&doc), Reply::Document(Box::new(view)));
        assert_eq!(classify(&report(None)), Reply::Silent);
        let refused = report(Some(CallOutcome::Err(CallError {
            name: "error/not-found".into(),
            message: "no resource eth9".into(),
        })));
        assert_eq!(
            classify(&refused),
            Reply::Refused {
                name: "error/not-found".into(),
                message: "no resource eth9".into()
            }
        );
        let other = report(Some(CallOutcome::Ok {
            value: Some(serde_json::json!({"token": "chg-1"})),
            text: None,
        }));
        assert!(matches!(classify(&other), Reply::Other(_)));
    }

    #[test]
    fn a_target_says_why_it_cannot_be_asked() {
        let mut t = ConfigTarget::default();
        assert!(t.unaskable().is_some());
        t = ConfigTarget::new("*", "radio", "wlan0");
        assert!(t.unaskable().unwrap().contains("RFC 05 §2.1"));
        t = ConfigTarget::new("h-a", "radio", "wlan0");
        assert_eq!(t.unaskable(), None);
        assert_eq!(t.read_path(), "config/wlan0");
    }
}
