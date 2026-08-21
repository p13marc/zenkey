//! Report values, once, for both corpora (#201).
//!
//! `zenkey-fleet/tests/report_contract.rs` asserts what these serialize to;
//! `zenctl/tests/render.rs` asserts how they are drawn. Sharing the
//! constructors is what keeps the two corpora talking about the same thing: a
//! field added to a report breaks one function here, and both then re-run
//! against the same value.
//!
//! The values are *realistic* rather than minimal — a `TopicList` with two
//! producers and an open-ended subject, a `NodeList` where one producer served
//! a slice and another did not — because a fixture that never exercises the
//! interesting case pins nothing about it. Where a corpus needs a variant
//! these do not cover, it builds that one inline; these are the shared
//! baseline, not a ceiling.

use zenkey_fleet::report::*;
use zenkey_fleet::{Coverage, CoverageRow, StorageInfo};

pub const ORIGIN: &str = "h-3fa9c2d41b7e";

/// A plain registered subject.
pub fn topic_row() -> TopicRow {
    TopicRow {
        producer: "sysinfo".into(),
        registry_version: "1.0".into(),
        class: "telemetry".into(),
        path: "disk/{mount}/used".into(),
        type_name: "TelemetryPoint".into(),
        open_ended: false,
        since: None,
        deprecated: false,
        deprecated_since: None,
        replaced_by: None,
    }
}

/// The same subject, retired — every optional field populated.
pub fn topic_row_retired() -> TopicRow {
    TopicRow {
        open_ended: true,
        since: Some("1.0".into()),
        deprecated: true,
        deprecated_since: Some("2.0".into()),
        replaced_by: Some("disk/{mount}/bytes_used".into()),
        ..topic_row()
    }
}

/// Two producers, one open-ended subject, one retirement: enough to exercise
/// the grouping, the `[open-ended]` tail and the `DEPRECATED` tail at once.
pub fn topic_list() -> TopicList {
    TopicList {
        subjects: vec![
            topic_row(),
            TopicRow {
                path: "health".into(),
                class: "state".into(),
                type_name: "HealthSnapshot".into(),
                ..topic_row()
            },
            TopicRow {
                producer: "logs".into(),
                registry_version: "2.0".into(),
                path: "by_unit/{unit}/messages_total".into(),
                open_ended: true,
                ..topic_row()
            },
            TopicRow {
                producer: "logs".into(),
                registry_version: "2.0".into(),
                path: "ingest/legacy_total".into(),
                ..topic_row_retired()
            },
        ],
    }
}

/// A roster where the slice join **was** attempted and one producer answered
/// nothing — the O4 case the table used to render as a blank.
pub fn node_list() -> NodeList {
    NodeList {
        slices_joined: true,
        nodes: vec![
            NodeRow {
                origin: ORIGIN.into(),
                producer: "sysinfo".into(),
                app: Some("zensight".into()),
                registry_version: Some("1.0".into()),
            },
            NodeRow {
                origin: ORIGIN.into(),
                producer: "parallax".into(),
                app: None,
                registry_version: None,
            },
            NodeRow {
                origin: "@catalog".into(),
                producer: "catalog".into(),
                app: Some("zensight".into()),
                registry_version: Some("1.1".into()),
            },
        ],
    }
}

/// The same roster with **no** join attempted: the other half of O4, and the
/// only thing distinguishing it is the envelope's `slices_joined`.
pub fn node_list_unjoined() -> NodeList {
    NodeList {
        slices_joined: false,
        nodes: vec![NodeRow {
            origin: ORIGIN.into(),
            producer: "sysinfo".into(),
            app: None,
            registry_version: None,
        }],
    }
}

/// A storage list carrying all three coverage verdicts, and a storage whose
/// admin document omitted its strip prefix.
pub fn storage_list() -> StorageList {
    StorageList {
        storages: vec![StorageInfo {
            name: "main".into(),
            zid: "aabbccdd".into(),
            key_expr: Some("acme/v1/**/state/**".into()),
            strip_prefix: None,
            volume: Some("memory".into()),
            // The untrimmed admin document. A fixture carries a realistic one
            // rather than `null`, because it is what a layout change would
            // arrive as.
            raw: serde_json::json!({
                "key_expr": "acme/v1/**/state/**",
                "volume": {"id": "memory"},
            }),
        }],
        coverage: vec![
            CoverageRow {
                producer: "sysinfo".into(),
                path: "health".into(),
                ttl_s: Some(120),
                coverage: Coverage::Covered("main@aabbccdd".into()),
            },
            CoverageRow {
                producer: "logs".into(),
                path: "state/{unit}".into(),
                ttl_s: None,
                coverage: Coverage::Partial("main@aabbccdd".into()),
            },
            CoverageRow {
                producer: "parallax".into(),
                path: "stream/{id}".into(),
                ttl_s: Some(30),
                coverage: Coverage::Uncovered,
            },
        ],
    }
}

/// A ladder verdict that reached the bottom rung, with a ttl and a payload
/// type — the shape `topic info` prints most often.
pub fn topic_info() -> TopicInfo {
    TopicInfo {
        key: format!("v1/{ORIGIN}/state/sysinfo/health"),
        verdict: TopicVerdict::Registered,
        note: String::new(),
        origin: Some(ORIGIN.into()),
        producer: Some("sysinfo".into()),
        class: Some("state".into()),
        subject: Some("health".into()),
        variables: Default::default(),
        payload_type: Some("HealthSnapshot".into()),
        unit: None,
        qos: Some("refreshed".into()),
        ttl_s: Some(120),
        rate: None,
        cardinality: None,
        encoding: None,
        since: Some("1.0".into()),
        description: None,
    }
}

/// A key that parses and that nothing declares — the rung above the bottom,
/// and the one whose fields are absent rather than null.
pub fn topic_info_unregistered() -> TopicInfo {
    TopicInfo {
        key: format!("v1/{ORIGIN}/telemetry/sysinfo/not/a/real/subject"),
        verdict: TopicVerdict::Unregistered,
        note: "the producer serves a slice and it does not declare this subject".into(),
        payload_type: None,
        qos: None,
        ttl_s: None,
        since: None,
        ..topic_info()
    }
}

/// A doctor run with one finding of each severity, and the listen phase's
/// bounded observation.
pub fn doctor_report() -> DoctorReport {
    DoctorReport {
        findings: vec![
            DoctorFinding {
                severity: DoctorSeverity::Error,
                check: "slice-sync".into(),
                subject: format!("{ORIGIN}/sysinfo"),
                evidence: "does not serve state health".into(),
                citation: Some("RFC 08 §6".into()),
            },
            DoctorFinding {
                severity: DoctorSeverity::Warning,
                check: "qos-observed-mismatch".into(),
                subject: format!("{ORIGIN}/sysinfo/health"),
                evidence: "declared refreshed, observed data/drop/reliable".into(),
                citation: None,
            },
            DoctorFinding {
                severity: DoctorSeverity::Info,
                check: "timestamp-stamped-elsewhere".into(),
                subject: "fleet".into(),
                evidence: "stamped by 1 node that is not the publisher".into(),
                citation: Some("RFC 09 §5.1 O7".into()),
            },
        ],
        synced: vec![format!("{ORIGIN}/catalog (registry 1.1)")],
        introspect_answered: 2,
        live_producers: 3,
        describe_served: 1,
        describe_missing: 1,
        routers: 1,
        router_version: Some("1.9.0".into()),
        deep: false,
        observation: Some(ObservationSummary {
            window_s: 10.0,
            scopes: vec!["v1/**".into()],
            samples: 412,
            keys_seen: 7,
            dropped: 3,
            synthetic_marked: 0,
        }),
    }
}
