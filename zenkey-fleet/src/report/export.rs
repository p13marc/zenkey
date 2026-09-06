//! The metrics-surface snapshot (#228, RFC 13 §3 *Exporter obligations*,
//! v1.34): what `zenctl export` serves, as one document.
//!
//! The Prometheus exposition is a **pure function of this struct**
//! ([`crate::model::prom::exposition`]), so `--format json` and the text a
//! scraper reads can never disagree about a value, a state or a bound. The
//! three exporter obligations are visible in the shape rather than in
//! prose: every O6 kind is its own counter under [`ObserverCounters`] and is
//! never summed; a series carries a [`SeriesState`] and loses its value —
//! not its labels — when it stopped, so absence and silence are different
//! bytes; the scopes watched and the planes a wildcard cannot reach ride the
//! document, and a payload verdict is three counted populations, the third
//! being *not validated*.

use std::collections::BTreeMap;

use serde::Serialize;

use super::asked::{Asked, u64_is_zero};
use super::doctor::{CheckId, DoctorSeverity};

/// One snapshot of the exporter's ledger, folded at scrape time.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExportSnapshot {
    /// The exact selectors watched — coverage is this list and no wider
    /// (RFC 13 §3 O5).
    pub scopes: Vec<String>,
    /// The planes a wildcard in `scopes` cannot reach, named verbatim; empty
    /// when no scope carries a wildcard. Never implied by the selector alone.
    pub excluded: Vec<String>,
    /// The registry the contract came from: how many producers' slices were
    /// loaded. *Not asked* when no registry was loaded at all — then no key
    /// refines and every key counts under `unregistered_keys`, which is a
    /// statement about the exporter, not about the fleet (O4).
    #[serde(skip_serializing_if = "Asked::is_not_asked", default)]
    pub registry: Asked<RegistryInfo>,
    /// The ledger's own bound on distinct series; overflow is counted under
    /// `suppressed["max_series"]`, never silent.
    pub max_series: usize,
    /// When the observer started watching (unix seconds). A claim about a
    /// span longer than `taken_at - started_at` is unobservable.
    pub started_at_unix_s: u64,
    /// When this fold was taken (unix seconds). Deliberately **not** in the
    /// exposition: two scrapes with no traffic between them must be
    /// byte-identical, and the scrape time is the scraper's to record.
    pub taken_at_unix_s: u64,
    /// Every series the ledger holds, in a deterministic order.
    pub series: Vec<SeriesRow>,
    pub observer: ObserverCounters,
    pub contract: ContractCounters,
    /// Samples that produced no series, by reason — `cardinality`
    /// (over the declared budget), `max_series`, `fields` (past the
    /// per-subject field cap), `text` (a declared text kind), `non_numeric`,
    /// `undecodable`, `unparsed` (not a v1 data key). Absent when nothing was
    /// suppressed.
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub suppressed: BTreeMap<String, u64>,
    /// Distinct keys the registry does not declare (or, with no registry
    /// loaded, every key): counted and never exported — the contract is the
    /// registry, and the count keeps the omission visible (O4).
    pub unregistered_keys: u64,
    /// The last doctor run, when one was asked for (`--doctor-every`).
    #[serde(skip_serializing_if = "Asked::is_not_asked", default)]
    pub doctor: Asked<DoctorSummary>,
}

/// The loaded registry, in one number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RegistryInfo {
    pub producers: usize,
}

/// One exported series: a contract-derived identity, its last value, and
/// the state that says whether the value is current.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SeriesRow {
    /// The metric name the exposition uses — derived from the registry's
    /// producer, pattern, `unit` and `kind`, never from the leaf's spelling.
    pub name: String,
    /// The concrete wire key this series was last fed from.
    pub key: String,
    pub origin: String,
    pub producer: String,
    pub class: String,
    /// The declared pattern, e.g. `disk/{mount}/used`.
    pub subject: String,
    /// The `{var}` bindings by their declared names.
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub labels: BTreeMap<String, String>,
    /// The top-level field this series reads, when the payload was an object
    /// carrying several numerics rather than one leaf value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// The declared kind token, when the registry declares one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// The declared unit, verbatim, when the registry declares one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// The newest value seen. **Absent** when the state says the series
    /// stopped — a stopped series exposes its state and its labels, never a
    /// stale number that reads as current.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    /// When the newest sample arrived, unix seconds on the observer's clock
    /// — captured at ingest, so it does not move between scrapes.
    pub last_seen_unix_s: u64,
    pub state: SeriesState,
    /// Samples folded into this series since the observer started.
    pub samples: u64,
    /// Folds during which this series was fed **and** the observer dropped
    /// samples — the interval where its value may not be the newest.
    #[serde(skip_serializing_if = "u64_is_zero", default)]
    pub drop_exposed: u64,
}

/// Why a series is, or is not, current (RFC 13 §3 — "a series that stopped
/// is not a series that went quiet").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeriesState {
    /// Fed, and nothing has said it stopped. For telemetry, which declares
    /// no period, this is all an observer can say — `last_seen_unix_s`
    /// carries the fact.
    Live,
    /// A `state` subject silent past its declared `ttl_s`. Judged only where
    /// the registry declares the period; never for telemetry (O4).
    Quiet,
    /// The observer chose to forget the key at its own bound (O6); the
    /// labels are kept so the disappearance is named.
    Evicted,
    /// The producer's `alive` token went away (RFC 04 §5) after this series
    /// was fed.
    OriginDown,
    /// The producer retired the key with a tombstone (RFC 04 §1.2) — its
    /// own statement, which outranks every inference above.
    Retired,
}

impl SeriesState {
    /// The wire token, exactly as it serializes — the `state` label.
    pub fn as_str(self) -> &'static str {
        match self {
            SeriesState::Live => "live",
            SeriesState::Quiet => "quiet",
            SeriesState::Evicted => "evicted",
            SeriesState::OriginDown => "origin_down",
            SeriesState::Retired => "retired",
        }
    }

    /// Whether a value line is exposed in this state.
    pub fn exposes_value(self) -> bool {
        matches!(self, SeriesState::Live | SeriesState::Quiet)
    }
}

/// What the bounded observer cost, by kind — never summed (RFC 13 §3 O6).
/// Every field is always present: a scraper expects a series to exist at
/// zero, and "absent" is the one thing an exporter must not let a series do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct ObserverCounters {
    /// Samples missed while the ledger's receiver was behind.
    pub dropped: u64,
    /// Keys retired from the statistics table at its bound.
    pub evicted_keys: u64,
    /// Retained samples dropped because the byte budget bit.
    pub evicted_bytes: u64,
    /// Retained samples that aged past the retention window.
    pub expired: u64,
    /// Keys retired because their watch was released.
    pub unwatched: u64,
    /// Samples folded into a newer one between two scrapes — the exporter
    /// keeps the newest value per series and counts the rest here.
    pub coalesced: u64,
    /// Samples that carried no HLC — counted, never defaulted to arrival.
    pub unstamped: u64,
}

/// Declared-versus-observed, counted (RFC 13 §3 — "a payload verdict is
/// three counted populations").
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct ContractCounters {
    /// Samples whose subject declares a QoS profile this build knows.
    pub qos_judged: u64,
    /// Of those, samples that did not ride the declared profile.
    pub qos_mismatch: u64,
    /// The mismatches by declared subject, for the labelled series.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub qos_mismatch_by_subject: Vec<QosMismatchRow>,
    pub payload_valid: u64,
    pub payload_invalid: u64,
    /// The third population: without `--validate` every sample lands here,
    /// and so does every sample past the decode budget.
    pub payload_not_validated: u64,
}

/// One declared subject's QoS mismatches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QosMismatchRow {
    pub producer: String,
    pub subject: String,
    pub n: u64,
}

/// The last doctor run, reduced to what a series can carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorSummary {
    pub ran_at_unix_s: u64,
    pub findings: Vec<DoctorFindingRef>,
}

/// One finding as a series identity: which check, how bad, on what.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorFindingRef {
    pub check: CheckId,
    pub severity: DoctorSeverity,
    pub subject: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The whole document, pinned: a stopped series carries no `value`, an
    /// unasked doctor and an unloaded registry are absent, a zero
    /// `drop_exposed` is absent, and every observer counter is present at
    /// zero.
    #[test]
    fn an_export_snapshot_is_pinned() {
        let snapshot = ExportSnapshot {
            scopes: vec!["v1/*/**".into()],
            excluded: vec!["@rpc".into(), "service origins".into()],
            registry: Asked::Asked(RegistryInfo { producers: 2 }),
            max_series: 10_000,
            started_at_unix_s: 1_700_000_000,
            taken_at_unix_s: 1_700_000_060,
            series: vec![
                SeriesRow {
                    name: "zenkey_subject_sysinfo_cpu_usage_percent".into(),
                    key: "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu/usage".into(),
                    origin: "h-3fa9c2d41b7e".into(),
                    producer: "sysinfo".into(),
                    class: "telemetry".into(),
                    subject: "cpu/usage".into(),
                    labels: BTreeMap::new(),
                    field: None,
                    kind: Some("gauge".into()),
                    unit: Some("percent".into()),
                    value: Some(12.5),
                    last_seen_unix_s: 1_700_000_059,
                    state: SeriesState::Live,
                    samples: 7,
                    drop_exposed: 0,
                },
                SeriesRow {
                    name: "zenkey_subject_sysinfo_disk_used_bytes".into(),
                    key: "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used".into(),
                    origin: "h-3fa9c2d41b7e".into(),
                    producer: "sysinfo".into(),
                    class: "telemetry".into(),
                    subject: "disk/{mount}/used".into(),
                    labels: [("mount".to_string(), "var-log".to_string())]
                        .into_iter()
                        .collect(),
                    field: Some("free".into()),
                    kind: None,
                    unit: Some("bytes".into()),
                    value: None,
                    last_seen_unix_s: 1_700_000_010,
                    state: SeriesState::OriginDown,
                    samples: 3,
                    drop_exposed: 1,
                },
            ],
            observer: ObserverCounters::default(),
            contract: ContractCounters {
                qos_judged: 7,
                qos_mismatch: 1,
                qos_mismatch_by_subject: vec![QosMismatchRow {
                    producer: "sysinfo".into(),
                    subject: "cpu/usage".into(),
                    n: 1,
                }],
                payload_valid: 0,
                payload_invalid: 0,
                payload_not_validated: 10,
            },
            suppressed: [("cardinality".to_string(), 4u64)].into_iter().collect(),
            unregistered_keys: 1,
            doctor: Asked::NotAsked,
        };
        assert_eq!(
            serde_json::to_value(&snapshot).unwrap(),
            json!({
                "scopes": ["v1/*/**"],
                "excluded": ["@rpc", "service origins"],
                "registry": {"producers": 2},
                "max_series": 10000,
                "started_at_unix_s": 1_700_000_000,
                "taken_at_unix_s": 1_700_000_060,
                "series": [
                    {
                        "name": "zenkey_subject_sysinfo_cpu_usage_percent",
                        "key": "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu/usage",
                        "origin": "h-3fa9c2d41b7e",
                        "producer": "sysinfo",
                        "class": "telemetry",
                        "subject": "cpu/usage",
                        "kind": "gauge",
                        "unit": "percent",
                        "value": 12.5,
                        "last_seen_unix_s": 1_700_000_059,
                        "state": "live",
                        "samples": 7,
                    },
                    {
                        "name": "zenkey_subject_sysinfo_disk_used_bytes",
                        "key": "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used",
                        "origin": "h-3fa9c2d41b7e",
                        "producer": "sysinfo",
                        "class": "telemetry",
                        "subject": "disk/{mount}/used",
                        "labels": {"mount": "var-log"},
                        "field": "free",
                        "unit": "bytes",
                        "last_seen_unix_s": 1_700_000_010,
                        "state": "origin_down",
                        "samples": 3,
                        "drop_exposed": 1,
                    },
                ],
                "observer": {
                    "dropped": 0,
                    "evicted_keys": 0,
                    "evicted_bytes": 0,
                    "expired": 0,
                    "unwatched": 0,
                    "coalesced": 0,
                    "unstamped": 0,
                },
                "contract": {
                    "qos_judged": 7,
                    "qos_mismatch": 1,
                    "qos_mismatch_by_subject": [
                        {"producer": "sysinfo", "subject": "cpu/usage", "n": 1}
                    ],
                    "payload_valid": 0,
                    "payload_invalid": 0,
                    "payload_not_validated": 10,
                },
                "suppressed": {"cardinality": 4},
                "unregistered_keys": 1,
            })
        );
    }

    /// The asked poles: a doctor that ran and a registry that was never
    /// loaded — one present with its findings, the other absent rather than
    /// `{"producers": 0}`.
    #[test]
    fn a_doctor_run_and_an_unloaded_registry_are_pinned() {
        let doctor = DoctorSummary {
            ran_at_unix_s: 1_700_000_030,
            findings: vec![DoctorFindingRef {
                check: CheckId::StaleState,
                severity: DoctorSeverity::Warning,
                subject: "h-3fa9c2d41b7e/sysinfo".into(),
            }],
        };
        assert_eq!(
            serde_json::to_value(&doctor).unwrap(),
            json!({
                "ran_at_unix_s": 1_700_000_030,
                "findings": [
                    {"check": "stale-state", "severity": "warning", "subject": "h-3fa9c2d41b7e/sysinfo"}
                ],
            })
        );
        let unloaded: Asked<RegistryInfo> = Asked::NotAsked;
        assert!(unloaded.is_not_asked());
    }

    #[test]
    fn a_stopped_state_exposes_no_value() {
        for s in [
            SeriesState::Evicted,
            SeriesState::OriginDown,
            SeriesState::Retired,
        ] {
            assert!(!s.exposes_value(), "{}", s.as_str());
        }
        assert!(SeriesState::Live.exposes_value());
        assert!(SeriesState::Quiet.exposes_value());
    }
}
