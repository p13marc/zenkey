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
//!
//! Two things are deliberately **not** here. Types behind `zenkey-fleet`'s
//! `decode` feature — `GenReport` — because this crate must not request a
//! feature (see the manifest, and #204). And zenctl-local report types, which
//! this crate cannot see; their fixtures live beside their tests.

use zenkey_fleet::report::*;
use zenkey_fleet::{Coverage, CoverageRow, RecordReport, ReplayReport, StorageInfo};

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

/// Two producers' procedures, with one that declares no reply type — the O4
/// cell the table used to spell `-`.
pub fn service_list() -> ServiceList {
    ServiceList {
        procedures: vec![
            ServiceRow {
                producer: "sysinfo".into(),
                registry_version: "1.0".into(),
                kind: "read".into(),
                path: "processes".into(),
                request: Some("ProcessQuery".into()),
                reply: Some("ProcessList".into()),
            },
            ServiceRow {
                producer: "sysinfo".into(),
                registry_version: "1.0".into(),
                kind: "write".into(),
                path: "gc".into(),
                request: None,
                reply: None,
            },
            ServiceRow {
                producer: "catalog".into(),
                registry_version: "1.1".into(),
                kind: "read".into(),
                path: "names".into(),
                request: None,
                reply: Some("Vec<NameVal>".into()),
            },
        ],
    }
}

/// A service origin, with a forbidden fan-out — the clause worth seeing
/// *before* reaching for `*` (RFC 08 §1.1 G2).
pub fn service_info() -> ServiceInfo {
    ServiceInfo {
        producer: "catalog".into(),
        registry_version: "1.1".into(),
        service_origin: Some("@catalog".into()),
        description: Some("the fleet's entity registry".into()),
        procedures: vec![
            ServiceProcedure {
                path: "link".into(),
                kind: "write".into(),
                key: "v1/@catalog/@rpc/catalog/link".into(),
                request: Some("LinkRequest".into()),
                reply: Some("OperatorAssertion".into()),
                fanout: Some("forbidden".into()),
                idempotent: Some(false),
                encoding: None,
                since: Some("1.0".into()),
                description: Some("assert that two ids are one entity".into()),
            },
            ServiceProcedure {
                path: "names".into(),
                kind: "read".into(),
                key: "v1/@catalog/@rpc/catalog/names".into(),
                request: None,
                reply: Some("Vec<NameVal>".into()),
                fanout: None,
                idempotent: Some(true),
                encoding: None,
                since: None,
                description: None,
            },
        ],
    }
}

pub fn interface_list() -> InterfaceList {
    InterfaceList {
        types: vec![
            InterfaceTypeRow {
                name: "TelemetryPoint".into(),
                carriers: 42,
            },
            InterfaceTypeRow {
                name: "HealthSnapshot".into(),
                carriers: 3,
            },
        ],
    }
}

/// Two producers serving the same type name with **different hashes** — the
/// RFC 08 §7 drift finding, on the type's own page rather than only in
/// `doctor`.
pub fn interface_show() -> InterfaceShow {
    InterfaceShow {
        type_name: "HealthSnapshot".into(),
        carriers: vec![
            CarrierRow {
                producer: "sysinfo".into(),
                class: "state".into(),
                path: "health".into(),
            },
            CarrierRow {
                producer: "gnmi".into(),
                class: "state".into(),
                path: "health".into(),
            },
        ],
        schemas: vec![
            SchemaRow {
                producer: "sysinfo".into(),
                type_name: "HealthSnapshot".into(),
                kind: "json-schema".into(),
                hash: "sha256:aaaa".into(),
                document: None,
            },
            SchemaRow {
                producer: "gnmi".into(),
                type_name: "HealthSnapshot".into(),
                kind: "json-schema".into(),
                hash: "sha256:bbbb".into(),
                document: None,
            },
        ],
    }
}

/// The empty base beside a named one, and a base known only from a storage
/// config — three different facts on three rows.
pub fn base_list() -> BaseList {
    use std::collections::BTreeSet;
    let set = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
    BaseList {
        bases: vec![
            zenkey_fleet::DiscoveredBase {
                base: "acme".into(),
                origins: set(&[ORIGIN, "@catalog"]),
                producers: set(&["sysinfo", "catalog"]),
                storages: vec!["main@aabbccdd".into()],
            },
            zenkey_fleet::DiscoveredBase {
                base: String::new(),
                origins: set(&[ORIGIN]),
                producers: set(&["probe"]),
                storages: vec![],
            },
            zenkey_fleet::DiscoveredBase {
                base: "staging".into(),
                origins: BTreeSet::new(),
                producers: BTreeSet::new(),
                storages: vec!["archive@eeff0011".into()],
            },
        ],
    }
}

/// Latency per origin, and errors counted apart from it — averaging a
/// non-answer into a latency figure is how a benchmark lies.
pub fn bench_report() -> BenchReport {
    BenchReport {
        key: format!("v1/{ORIGIN}/@rpc/sysinfo/processes"),
        requested: 100,
        completed: 98,
        concurrency: 8,
        errors: 1,
        silent: 1,
        elapsed_s: 2.5,
        calls_per_s: 39.2,
        origins: vec![
            OriginLatency {
                origin: ORIGIN.into(),
                replies: 64,
                min_ms: 0.8,
                p50_ms: 1.9,
                p95_ms: 12.4,
                p99_ms: 40.1,
                max_ms: 123.456,
            },
            OriginLatency {
                origin: "h-bbbbbbbbbbbb".into(),
                replies: 34,
                min_ms: 1.1,
                p50_ms: 2.2,
                p95_ms: 9.9,
                p99_ms: 11.0,
                max_ms: 12.5,
            },
        ],
    }
}

/// One producer agreeing, one disagreeing, and one the bus serves that the
/// checkout does not have — the `served x · local —` case.
pub fn registry_diff() -> RegistryDiff {
    RegistryDiff {
        producers: vec![
            ProducerDiff {
                producer: "catalog".into(),
                served_version: Some("1.1".into()),
                local_version: Some("1.1".into()),
                findings: vec![],
            },
            ProducerDiff {
                producer: "sysinfo".into(),
                served_version: Some("1.1".into()),
                local_version: Some("1.0".into()),
                findings: vec![
                    "served declares telemetry disk/{mount}/inodes; local does not".into(),
                ],
            },
            ProducerDiff {
                producer: "parallax".into(),
                served_version: Some("1.3".into()),
                local_version: None,
                findings: vec!["no local slice for this producer".into()],
            },
        ],
    }
}

/// A served schema set with a totality gap — RFC 08 §7 says the set MUST
/// cover every referenced name, and this one does not.
pub fn schema_dump() -> SchemaDump {
    SchemaDump {
        producer: "sysinfo".into(),
        served: true,
        app: Some("zensight".into()),
        types: vec![SchemaRow {
            producer: "sysinfo".into(),
            type_name: "HealthSnapshot".into(),
            kind: "json-schema".into(),
            hash: "sha256:aaaa".into(),
            document: None,
        }],
        missing: Some(vec!["TelemetryPoint".into()]),
    }
}

/// A producer that serves no `describe` at all — undescribed shapes, which is
/// not the same as having none (§7 is a SHOULD).
pub fn schema_dump_unserved() -> SchemaDump {
    SchemaDump {
        producer: "parallax".into(),
        served: false,
        app: None,
        types: vec![],
        missing: None,
    }
}

/// A served set dumped with no registry loaded — totality was never checked,
/// which must not render as "nothing missing" (RFC 09 §5.1 O4, #246).
pub fn schema_dump_unchecked() -> SchemaDump {
    let mut dump = schema_dump();
    dump.missing = None;
    dump
}

/// A declared tier whose roster was asked and answered nothing, beside one
/// nobody asked about — the two `—`s that mean different things.
pub fn blob_list() -> BlobList {
    BlobList {
        source: BlobListSource::RegistryDirs,
        slices_considered: 11,
        slices_without_blob: 9,
        tiers: vec![
            BlobTierRow {
                producer: "logs".into(),
                registry_version: "2.0".into(),
                tier: "store".into(),
                known_tier: true,
                endpoints: vec!["have".into(), "chunk".into()],
                algo: Some("blake3".into()),
                reference: None,
                encoding: None,
                since: Some("1.1".into()),
                description: None,
                origins: Some(vec![]),
            },
            BlobTierRow {
                producer: "parallax".into(),
                registry_version: "1.3".into(),
                tier: "artifact".into(),
                known_tier: true,
                endpoints: vec!["manifest".into()],
                algo: Some("blake3".into()),
                reference: Some("ArtifactRef".into()),
                encoding: Some("application/octet-stream".into()),
                since: None,
                description: Some("build artifacts".into()),
                origins: None,
            },
        ],
    }
}

/// Two holders at **two different content roots** — a finding, not a
/// tie-break: the id is a name and the root is what disambiguates it
/// (RFC 07 §2.1).
pub fn blob_probe() -> BlobProbeReport {
    BlobProbeReport {
        target: "artifact/01jqz3demo0001".into(),
        tier: "artifact".into(),
        asked: vec![
            "v1/*/@blob/artifact/01jqz3demo0001/manifest".into(),
            "v1/*/@blob/artifact/01jqz3demo0001/have".into(),
        ],
        not_probed: None,
        answered: 2,
        roots: vec!["60e03a78c0e0".into(), "97ac2e30aa77".into()],
        declared_by: vec!["parallax".into()],
        holders: vec![
            BlobHolder {
                origin: ORIGIN.into(),
                key: format!("v1/{ORIGIN}/@blob/artifact/01jqz3demo0001/manifest"),
                availability: Some(BlobAvailability {
                    chunk_count: 8,
                    have: 8,
                    complete: true,
                }),
                manifest: Some(BlobManifest {
                    id: "01jqz3demo0001".into(),
                    filename: Some("bundle.bin".into()),
                    total_len: 65536,
                    chunk_size: 8192,
                    chunk_count: 8,
                    root: "60e03a78c0e0c5be5f18".into(),
                    created_ms: 0,
                }),
                note: None,
                unreadable: None,
                error: None,
            },
            BlobHolder {
                origin: "h-bbbbbbbbbbbb".into(),
                key: "v1/h-bbbbbbbbbbbb/@blob/artifact/01jqz3demo0001/have".into(),
                availability: Some(BlobAvailability {
                    chunk_count: 8,
                    have: 3,
                    complete: false,
                }),
                manifest: None,
                note: Some("every chunk but no index".into()),
                unreadable: None,
                error: None,
            },
        ],
    }
}

/// A probe that was never issued — the O4 case, which must never read as
/// "nobody holds it".
pub fn blob_probe_unissued() -> BlobProbeReport {
    BlobProbeReport {
        target: "tree/deadbeef".into(),
        tier: "tree".into(),
        asked: vec![],
        not_probed: Some("the reference client does not speak this store algorithm".into()),
        answered: 0,
        roots: vec![],
        declared_by: vec!["parallax".into()],
        holders: vec![],
    }
}

pub fn blob_tree() -> BlobTreeIndexReport {
    BlobTreeIndexReport {
        origin: ORIGIN.into(),
        key: format!("v1/{ORIGIN}/@blob/tree/deadbeef/index"),
        root: "deadbeefcafe".into(),
        entries: 12,
        files: 9,
        total_size: 1_048_576,
        chunks: 40,
        elapsed_ms: 31,
        priority: "data_low/block/reliable".into(),
    }
}

/// A fetch with no pinned root and a replier whose bytes did not verify — the
/// transfer succeeded, which is the point of verifying before disk, but a
/// replier served bytes that did not verify.
pub fn blob_fetch() -> BlobFetchReport {
    BlobFetchReport {
        origin: ORIGIN.into(),
        key: format!("v1/{ORIGIN}/@blob/artifact/01jqz3demo0001/chunk"),
        dest: "bundle.bin".into(),
        bytes: 65536,
        chunks: 8,
        chunks_resumed: 2,
        rejected: 1,
        retries: 1,
        elapsed_ms: 412,
        root: "60e03a78c0e0c5be5f18".into(),
        root_pinned: false,
        priority: "data_low/block/reliable".into(),
    }
}

/// One origin that answered, one that returned an RFC 05 §3 error envelope,
/// and a reply carrying an attachment — the clause `probe` used to drop.
pub fn call_report() -> CallReport {
    CallReport {
        key: "v1/*/@rpc/sysinfo/processes".to_string(),
        answers: vec![
            CallAnswer {
                origin: ORIGIN.into(),
                ok: true,
                value: Some(serde_json::json!({"count": 214})),
                text: None,
                attachment: Some(serde_json::json!({"trace": "abc123"})),
                attachment_bytes: Some(18),
                error: None,
            },
            CallAnswer {
                origin: "h-bbbbbbbbbbbb".into(),
                ok: false,
                value: None,
                text: None,
                attachment: None,
                attachment_bytes: None,
                error: Some(CallError {
                    name: "unsupported".into(),
                    message: "this build serves no `processes`".into(),
                }),
            },
        ],
    }
}

/// The same call, reached through a bridge — the resolution provenance is
/// what `probe` adds over `service call` (RFC 06 §6.2).
pub fn probe_report() -> ProbeReport {
    ProbeReport {
        input: "web-07".into(),
        origin: ORIGIN.into(),
        via: format!("bridge:v1/{ORIGIN}/state/sysinfo/health"),
        call: call_report(),
    }
}

/// The old plane still speaking: a migration you can assert the absence of is
/// a migration you can finish (RFC 09 §6).
pub fn cutover_report() -> CutoverReport {
    CutoverReport {
        old_root: "acme/legacy".into(),
        new_prefix: "acme/v1/".into(),
        window_s: 30,
        old_samples: 12,
        old_keys_seen: 2,
        old_examples: vec![
            "acme/legacy/sysinfo/health".into(),
            "acme/legacy/sysinfo/disk".into(),
        ],
        new_samples: 480,
        leak_samples: 3,
        leaked_keys_seen: 1,
        leak_examples: vec!["acme/scratch/tmp".into()],
        dropped: 5,
        verdict: CutoverVerdict::OldStillSpeaks,
    }
}

/// The burn-down with all three entry states (issue #226): a retired subject
/// still served (the RFC 08 §6.1 lie), one finished, and one whose
/// replacement was silent — `Unproven`, not a pass.
pub fn retired_report() -> RetiredReport {
    let entries = vec![
        RetiredEntry {
            producer: "logs".into(),
            path: "logs/errors_total".into(),
            since: Some("2.0".into()),
            replaced_by: Some("logs/journald/errors_total".into()),
            selector: "v1/*/*/logs/logs/errors_total".into(),
            wire_samples: Some(3),
            still_declared: Some(true),
            subscribers: Some(1),
            replacement_samples: Some(480),
            verdict: CutoverVerdict::OldStillSpeaks,
        },
        RetiredEntry {
            producer: "logs".into(),
            path: "logs/by_unit/{unit}/burn_rate".into(),
            since: Some("2.0".into()),
            replaced_by: Some("logs/journald/burn_rate".into()),
            selector: "v1/*/*/logs/logs/by_unit/*/burn_rate".into(),
            wire_samples: Some(0),
            still_declared: Some(false),
            subscribers: Some(0),
            replacement_samples: Some(120),
            verdict: CutoverVerdict::Pass,
        },
        RetiredEntry {
            producer: "logs".into(),
            path: "logs/units_in_failure".into(),
            since: Some("2.0".into()),
            replaced_by: Some("logs/journald/units_in_failure".into()),
            selector: "v1/*/*/logs/logs/units_in_failure".into(),
            wire_samples: Some(0),
            still_declared: Some(false),
            subscribers: Some(0),
            replacement_samples: Some(0),
            verdict: CutoverVerdict::Unproven,
        },
    ];
    RetiredReport {
        registries: vec!["../zensight/zensight-common/registry".into()],
        entries,
        window_s: Some(30),
        plane_samples: Some(960),
        dropped: 5,
        introspect_answered: 2,
        admin_entities: Some(14),
        verdict: CutoverVerdict::OldStillSpeaks,
    }
}

/// A window that could not carry its claim: `Impaired` is the absence of a
/// verdict, not a milder failure (O6).
pub fn expect_report() -> ExpectReport {
    ExpectReport {
        selector: "acme/v1/**/state/**".into(),
        window_s: 5.0,
        ended_early: false,
        samples: 120,
        keys_seen: 4,
        dropped: 17,
        rate_hz: Some(24.0),
        violations: vec!["v1/h-3fa9c2d41b7e/state/sysinfo/health: qos data/drop".into()],
        violations_total: 9,
        unmet: vec!["17 sample(s) were dropped while behind".into()],
        verdict: ExpectVerdict::Impaired,
    }
}

fn header() -> zenkey_fleet::ZrecHeader {
    zenkey_fleet::ZrecHeader {
        zrec: 1,
        selectors: vec!["acme/v1/**".into()],
        base: "acme".into(),
        captured_at: "2026-08-21T00:00:00Z".into(),
    }
}

/// A capture that fell behind: the file carries the gaps where they happened
/// *and* the total, so a partial view says so (O6).
pub fn record_report() -> RecordReport {
    RecordReport {
        header: header(),
        out: Some("bus.zrec".into()),
        samples: 4_820,
        dropped: 31,
        duration_ms: 10_000,
    }
}

/// A dry replay of that capture — a partial view of a partial view.
pub fn replay_report() -> ReplayReport {
    ReplayReport {
        header: header(),
        dry_run: true,
        speed: 1.0,
        published: 4_800,
        tombstones: 20,
        malformed: 2,
        refused: 1,
        capture_dropped: 31,
        first_errors: vec![
            "line 41: unknown QoS profile \"data/drop/reliable\"".into(),
            "line 88: refused delete on a telemetry key".into(),
        ],
    }
}

/// A rate report with the key table bounded and hit — the O6 count that a
/// trailing envelope used to lose to `| head`.
pub fn rate_report() -> RateReport {
    RateReport {
        selector: "acme/v1/**".into(),
        window_s: 10,
        total_count: 9_600,
        total_bytes: 528_000,
        keys: 50_000,
        evicted: 912,
        max_keys: 50_000,
        sn_gaps: Some(0),
        rows: vec![
            RateRow {
                key: format!("v1/{ORIGIN}/telemetry/sysinfo/disk/var-log/used"),
                count: 50,
                bytes: 2_250,
                sn_gaps: 0,
                latency: None,
                unstamped: 50,
            },
            RateRow {
                key: format!("v1/{ORIGIN}/state/sysinfo/health"),
                count: 50,
                bytes: 1_800,
                sn_gaps: 0,
                latency: None,
                unstamped: 0,
            },
        ],
    }
}

/// A quiet segment: an empty scout heard a **boundary**, not an absence.
pub fn scout_report_empty() -> ScoutReport {
    ScoutReport {
        asked: vec!["router".into(), "peer".into(), "client".into()],
        timeout_s: 3,
        heard: vec![],
    }
}

pub fn scout_report() -> ScoutReport {
    ScoutReport {
        asked: vec!["router".into()],
        timeout_s: 3,
        heard: vec![zenkey_fleet::HelloView {
            zid: "aabbccdd".into(),
            whatami: "router".into(),
            locators: vec!["tcp/10.0.0.1:7447".into()],
        }],
    }
}

/// A peer-only mesh: `[]` cannot tell this from an admin space that is off.
pub fn router_list_empty() -> RouterList {
    RouterList {
        asked: "@/*/router".into(),
        routers: vec![],
    }
}

pub fn router_list() -> RouterList {
    RouterList {
        asked: "@/*/router".into(),
        routers: vec![zenkey_fleet::RouterInfo {
            zid: "aabbccdd".into(),
            version: Some("1.9.0".into()),
            locators: vec!["tcp/10.0.0.1:7447".into()],
            raw: serde_json::json!({"zid": "aabbccdd"}),
        }],
    }
}

/// A node inventory where one producer answered `introspect` and another did
/// not — "capabilities unknown, not absent" — plus a stale state family and
/// one that has never been seen at all.
pub fn node_info() -> zenkey_fleet::NodeInfo {
    zenkey_fleet::NodeInfo {
        origin: ORIGIN.into(),
        producers: vec![
            zenkey_fleet::ProducerInfo {
                name: "sysinfo".into(),
                alive: true,
                app: Some("zensight".into()),
                registry_version: Some("1.0".into()),
                subjects: 41,
                procedures: 3,
                blob_tiers: vec!["store".into()],
                media: vec![],
                deprecated_served: 2,
            },
            zenkey_fleet::ProducerInfo {
                name: "parallax".into(),
                alive: true,
                app: None,
                registry_version: None,
                subjects: 0,
                procedures: 0,
                blob_tiers: vec![],
                media: vec![],
                deprecated_served: 0,
            },
            zenkey_fleet::ProducerInfo {
                name: "probe".into(),
                alive: false,
                app: Some("zensight".into()),
                registry_version: Some("1.0".into()),
                subjects: 2,
                procedures: 1,
                blob_tiers: vec![],
                media: vec![],
                deprecated_served: 0,
            },
        ],
        freshness: vec![
            zenkey_fleet::Freshness {
                producer: "sysinfo".into(),
                path: "health".into(),
                ttl_s: 120,
                age_s: Some(240),
                stale: true,
            },
            zenkey_fleet::Freshness {
                producer: "sysinfo".into(),
                path: "errors".into(),
                ttl_s: 300,
                // Never seen is not zero seconds old.
                age_s: None,
                stale: false,
            },
        ],
    }
}

/// A mesh where one node answered the admin space and one was only heard of,
/// and an origin whose sources named no single session.
pub fn topology() -> zenkey_fleet::TopologyReport {
    zenkey_fleet::TopologyReport {
        asked: "@/*/router".into(),
        answered: 1,
        self_zid: "ffffffff".into(),
        nodes: vec![
            zenkey_fleet::TopologyNode {
                zid: "aabbccdd".into(),
                whatami: "router".into(),
                version: Some("1.9.0".into()),
                locators: vec!["tcp/10.0.0.1:7447".into()],
                answered: true,
            },
            zenkey_fleet::TopologyNode {
                zid: "eeff0011".into(),
                whatami: "peer".into(),
                version: None,
                locators: vec![],
                answered: false,
            },
        ],
        edges: vec![zenkey_fleet::TopologyEdge {
            reporter: "aabbccdd".into(),
            peer: "eeff0011".into(),
            whatami: "peer".into(),
            links: vec!["tcp".into()],
        }],
    }
}

pub fn attachments() -> Vec<zenkey_fleet::OriginAttachment> {
    vec![
        zenkey_fleet::OriginAttachment {
            origin: ORIGIN.into(),
            session_zid: Some("eeff0011".into()),
            reporter_zid: "aabbccdd".into(),
            token_key: format!("v1/{ORIGIN}/state/sysinfo/alive"),
        },
        zenkey_fleet::OriginAttachment {
            origin: "h-bbbbbbbbbbbb".into(),
            session_zid: None,
            reporter_zid: "aabbccdd".into(),
            token_key: "v1/h-bbbbbbbbbbbb/state/logs/alive".into(),
        },
    ]
}
