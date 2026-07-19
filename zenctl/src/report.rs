//! Typed reports (issue #12): every list/info command assembles one of these,
//! and the output layer renders it as a table (humans) or JSON/NDJSON
//! (scripts). The struct is the contract — `--format json` output is stable
//! serde, not scraped table text.

use std::collections::BTreeMap;

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct TopicRow {
    pub producer: String,
    pub registry_version: String,
    pub class: String,
    pub path: String,
    pub type_name: String,
    /// Trailing `{var...}` family: the registry fixes the shape, not the
    /// members.
    pub open_ended: bool,
}

#[derive(Debug, Serialize)]
pub struct TopicList {
    pub subjects: Vec<TopicRow>,
}

#[derive(Debug, Serialize)]
pub struct TopicInfo {
    pub key: String,
    pub origin: String,
    pub producer: String,
    pub class: String,
    pub subject: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub variables: BTreeMap<String, String>,
    pub payload_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qos: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_s: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cardinality: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ServiceRow {
    pub producer: String,
    pub registry_version: String,
    pub kind: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ServiceList {
    pub procedures: Vec<ServiceRow>,
}

#[derive(Debug, Serialize)]
pub struct InterfaceTypeRow {
    pub name: String,
    pub carriers: usize,
}

#[derive(Debug, Serialize)]
pub struct InterfaceList {
    pub types: Vec<InterfaceTypeRow>,
}

#[derive(Debug, Serialize)]
pub struct CarrierRow {
    pub producer: String,
    pub class: String,
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct InterfaceShow {
    pub type_name: String,
    pub carriers: Vec<CarrierRow>,
}

#[derive(Debug, Serialize)]
pub struct NodeList {
    /// origin → live producer names (from liveliness tokens; zero payload).
    pub origins: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct CallError {
    pub name: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct CallAnswer {
    pub origin: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    /// Raw text when the value is not JSON-shaped (TOML introspect replies…).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CallError>,
}

#[derive(Debug, Serialize)]
pub struct CallReport {
    pub key: String,
    pub answers: Vec<CallAnswer>,
}

impl CallReport {
    /// The process exit code discipline (issue #12): 0 = at least one answer
    /// and no error replies; 1 = at least one error reply; 2 = zero replies
    /// (silence stays a distinct non-verdict — RFC 05 §3.1).
    pub fn exit_code(&self) -> i32 {
        if self.answers.is_empty() {
            2
        } else if self.answers.iter().any(|a| !a.ok) {
            1
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_exit_codes() {
        let mut r = CallReport {
            key: "k".into(),
            answers: vec![],
        };
        assert_eq!(r.exit_code(), 2, "silence is its own exit code");
        r.answers.push(CallAnswer {
            origin: "h-1".into(),
            ok: true,
            value: None,
            text: Some("x".into()),
            error: None,
        });
        assert_eq!(r.exit_code(), 0);
        r.answers.push(CallAnswer {
            origin: "h-2".into(),
            ok: false,
            value: None,
            text: None,
            error: Some(CallError {
                name: "error/busy".into(),
                message: "later".into(),
            }),
        });
        assert_eq!(r.exit_code(), 1, "any refusal fails the invocation");
    }
}
