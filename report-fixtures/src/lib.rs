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
use zenkey_fleet::{
    Coverage, CoverageRow, RecordReport, ReplayReport, StorageInfo, TimelineReport,
};

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
        cardinality: None,
        budget: None,
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
        budget: None,
    }
}

/// The same list under `--budget` (#221): a declared bound that held, one
/// that was exceeded, and a rest-variable exemption — plus the window
/// statement that keeps every number honest.
pub fn topic_list_budget() -> TopicList {
    let mut list = topic_list();
    list.budget = Some(BudgetWindow {
        window_s: 10.0,
        scopes: vec!["v1/*/telemetry/**".into(), "v1/*/state/**".into()],
        keys: 44,
        evicted: 0,
    });
    // disk/{mount}/used: declared 16, one origin at 40 — the finding.
    list.subjects[0].cardinality = Some(16);
    list.subjects[0].budget = Some(BudgetCell {
        declared: Some(16),
        observed: 41,
        origins: 2,
        worst_origin: Some(ORIGIN.into()),
        worst_observed: 40,
        exempt: None,
        over: true,
        examples: vec![
            format!("v1/{ORIGIN}/telemetry/sysinfo/disk/m00/used"),
            format!("v1/{ORIGIN}/telemetry/sysinfo/disk/m01/used"),
        ],
    });
    // by_unit/{unit}/messages_total: a rest family — exempt, and saying so.
    list.subjects[2].cardinality = Some(500);
    list.subjects[2].budget = Some(BudgetCell {
        declared: Some(500),
        observed: 3,
        origins: 1,
        worst_origin: Some(ORIGIN.into()),
        worst_observed: 3,
        exempt: Some("rest-variable".into()),
        over: false,
        examples: vec![],
    });
    list
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
        kind: None,
        qos: Some("refreshed".into()),
        ttl_s: Some(120),
        rate: None,
        cardinality: None,
        encoding: None,
        since: Some("1.0".into()),
        description: None,
    }
}

/// Every `Option` on the report populated — the NO-DEAD-FIELD PIN's fixture
/// (report-honesty finding R2, third recurrence of the class: `cardinality`
/// sat dead until #221, `rate`/`since`/`description` until this batch).
/// `report_contract.rs` asserts the constructor path can reach every field
/// this serializes; a field addable here but unreachable there is dead on
/// arrival.
pub fn topic_info_full() -> TopicInfo {
    TopicInfo {
        key: format!("v1/{ORIGIN}/telemetry/sysinfo/disk/var-log/used"),
        verdict: TopicVerdict::Registered,
        note: String::new(),
        origin: Some(ORIGIN.into()),
        producer: Some("sysinfo".into()),
        class: Some("telemetry".into()),
        subject: Some("disk/{mount}/used".into()),
        variables: [("mount".to_string(), "var-log".to_string())]
            .into_iter()
            .collect(),
        payload_type: Some("TelemetryPoint".into()),
        unit: Some("bytes".into()),
        kind: Some("gauge".into()),
        qos: Some("sampled".into()),
        ttl_s: Some(120),
        rate: Some("low".into()),
        cardinality: Some(16),
        encoding: Some("application/cbor".into()),
        since: Some("1.0".into()),
        description: Some("bytes used per mount".into()),
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
                check: CheckId::SliceSync,
                subject: format!("{ORIGIN}/sysinfo"),
                evidence: "does not serve state health".into(),
                citation: Some("RFC 08 §6".into()),
            },
            DoctorFinding {
                severity: DoctorSeverity::Warning,
                check: CheckId::QosObservedMismatch,
                subject: format!("{ORIGIN}/sysinfo/health"),
                evidence: "declared refreshed, observed data/drop/reliable".into(),
                citation: None,
            },
            DoctorFinding {
                severity: DoctorSeverity::Info,
                check: CheckId::TimestampStampedElsewhere,
                subject: "fleet".into(),
                evidence: "stamped by 1 node that is not the publisher".into(),
                citation: Some("RFC 09 §5.1 O7".into()),
            },
        ],
        synced: Asked::Asked(vec![format!("{ORIGIN}/catalog (registry 1.1)")]),
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
            field_paths_dropped: 0,
            facts_evicted: 0,
        }),
    }
}

/// A `zenctl field` window (#223): one frozen numeric flagged stuck, one
/// healthy small-domain path beside it, and a path table that hit its bound
/// — the report must carry the bound's cost, not just its rows.
pub fn field_report() -> FieldReport {
    let key = format!("v1/{ORIGIN}/state/sysinfo/health");
    FieldReport {
        selector: "v1/*/state/sysinfo/health".into(),
        window_s: 30.0,
        samples: 42,
        keys_seen: 1,
        dropped: 0,
        undocumented: 2,
        unread: 0,
        registry_loaded: true,
        paths: 2,
        max_paths: 2,
        paths_dropped: 3,
        paths_dropped_examples: vec![format!("{key} · debug.trace")],
        facts_evicted: 0,
        rows: vec![
            FieldRow {
                key: key.clone(),
                path: "temperature_c".into(),
                seen: 40,
                documents: 40,
                kinds: vec!["number".into()],
                changes: 0,
                last_change_s: None,
                min: Some(21.5),
                max: Some(21.5),
                last: Some(21.5),
                values: Some(vec!["21.5".into()]),
            },
            FieldRow {
                key: key.clone(),
                path: "status".into(),
                seen: 40,
                documents: 40,
                kinds: vec!["string".into()],
                changes: 1,
                last_change_s: Some(12.0),
                min: None,
                max: None,
                last: None,
                values: Some(vec!["\"degraded\"".into(), "\"ok\"".into()]),
            },
        ],
        findings: vec![DoctorFinding {
            severity: DoctorSeverity::Warning,
            check: CheckId::FieldStuck,
            subject: format!("{key} · temperature_c"),
            evidence: "value 21.5 unchanged across 40 sample(s) spanning 29.5s — at \
                       least 3× the declared ttl_s 5s — while the key kept publishing. \
                       An observation over this 30s window, not a verdict: a \
                       constant-by-design field always reads this way"
                .into(),
            citation: Some("RFC 04 §1.2".into()),
        }],
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
/// `doctor`. The verdict rides in `drift`, attributed per origin (#410): the
/// rows alone name a producer and never a host.
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
        // `Some` = `--schema` was asked (R4); `None` is the unasked run.
        schemas: Asked::Asked(vec![
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
        ]),
        drift: vec![SchemaDrift {
            type_name: "HealthSnapshot".into(),
            servers: vec![
                SchemaServer {
                    producer: "sysinfo".into(),
                    origin: "h-aaaaaaaaaaaa".into(),
                    hash: Asked::Asked("sha256:aaaa".into()),
                },
                SchemaServer {
                    producer: "gnmi".into(),
                    origin: "h-bbbbbbbbbbbb".into(),
                    hash: Asked::Asked("sha256:bbbb".into()),
                },
            ],
            verdict: DriftVerdict::Disagree,
        }],
    }
}

/// The same type without `--schema` — the bus was never asked, and the report
/// says so instead of an empty list that reads as "none served" (R4). No
/// drift either: a verdict on an unasked question is the thing O4 forbids.
pub fn interface_show_unasked() -> InterfaceShow {
    InterfaceShow {
        schemas: Asked::NotAsked,
        drift: Vec::new(),
        ..interface_show()
    }
}

/// One producer on two hosts, and the second host served **no identity**:
/// agreement is not established, and it is not a disagreement either — the
/// third state #370 pulled apart. The rows show one host's hash (the fold
/// keeps the first answer per producer), which is exactly why the rows could
/// never carry this verdict (#410).
pub fn interface_show_unjudgeable() -> InterfaceShow {
    InterfaceShow {
        type_name: "HealthSnapshot".into(),
        carriers: vec![CarrierRow {
            producer: "sysinfo".into(),
            class: "state".into(),
            path: "health".into(),
        }],
        schemas: Asked::Asked(vec![SchemaRow {
            producer: "sysinfo".into(),
            type_name: "HealthSnapshot".into(),
            kind: "json-schema".into(),
            hash: "sha256:aaaa".into(),
            document: None,
        }]),
        drift: vec![SchemaDrift {
            type_name: "HealthSnapshot".into(),
            servers: vec![
                SchemaServer {
                    producer: "sysinfo".into(),
                    origin: "h-aaaaaaaaaaaa".into(),
                    hash: Asked::Asked("sha256:aaaa".into()),
                },
                SchemaServer {
                    producer: "sysinfo".into(),
                    origin: "h-bbbbbbbbbbbb".into(),
                    hash: Asked::NotAsked,
                },
            ],
            verdict: DriftVerdict::Unjudgeable,
        }],
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

/// Latency per origin, and three non-answers counted apart from it — averaging
/// a non-answer into a latency figure is how a benchmark lies. The three are
/// deliberately distinct: an error reply, a call nobody answered, and a call
/// that panicked inside the tool (#329).
pub fn bench_report() -> BenchReport {
    BenchReport {
        key: format!("v1/{ORIGIN}/@rpc/sysinfo/processes"),
        requested: 100,
        completed: 98,
        concurrency: 8,
        errors: 1,
        silent: 1,
        panicked: 1,
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
///
/// Plus the case #399 exists for: `catalog` agrees with the checkout *and*
/// two hosts serve it at different versions, so the row that reads "agree"
/// is computed from one of them. A fixture where the two disagreements are
/// on the same producer is the one that proves the renderer keeps them
/// apart — the fleet against the checkout, and the fleet against itself.
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
        collapsed: Asked::Asked(vec![CollapsedProducer {
            producer: "catalog".into(),
            origins: vec!["h-3fa9c2d41b7e".into(), "h-8b1e07af22c9".into()],
            versions: vec!["1.1".into(), "1.0".into()],
            agreed: false,
        }]),
    }
}

/// The same diff, from a served side that never came off the bus — so the
/// collapse question was never put. The pair with [`registry_diff`] is what
/// pins that "not asked" and "asked, and nothing collapsed" render and
/// serialize differently (RFC 13 §3 O4).
pub fn registry_diff_not_asked() -> RegistryDiff {
    RegistryDiff {
        collapsed: Asked::NotAsked,
        ..registry_diff()
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
        missing: Asked::Asked(vec!["TelemetryPoint".into()]),
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
        missing: Asked::NotAsked,
    }
}

/// A served set dumped with no registry loaded — totality was never checked,
/// which must not render as "nothing missing" (RFC 09 §5.1 O4, #246).
pub fn schema_dump_unchecked() -> SchemaDump {
    let mut dump = schema_dump();
    dump.missing = Asked::NotAsked;
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
                origins: Asked::Asked(vec![]),
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
                origins: Asked::NotAsked,
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
        // R7: the count behind `declared_by` — same numbers as blob_list().
        slices_considered: 11,
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
        slices_considered: 11,
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
/// and a reply carrying an attachment — the clause `check probe` used to drop.
pub fn call_report() -> CallReport {
    CallReport {
        key: "v1/*/@rpc/sysinfo/processes".to_string(),
        timeout_s: 5.0,
        answers: vec![
            CallAnswer {
                origin: ORIGIN.into(),
                outcome: CallOutcome::Ok {
                    value: Some(serde_json::json!({"count": 214})),
                    text: None,
                },
                attachment: Some(serde_json::json!({"trace": "abc123"})),
                attachment_bytes: Some(18),
            },
            CallAnswer {
                origin: "h-bbbbbbbbbbbb".into(),
                outcome: CallOutcome::Err(CallError {
                    name: "unsupported".into(),
                    message: "this build serves no `processes`".into(),
                }),
                attachment: None,
                attachment_bytes: None,
            },
        ],
    }
}

/// Two bounded replies that stopped early (RFC 05 §3.2, #424): one that
/// offers a cursor to continue from, with the advisory fields present, and
/// one that offers `next_cursor: null` — the contract violation the RFC
/// says an observer MAY report.
pub fn call_report_partial_page() -> CallReport {
    CallReport {
        key: "v1/*/@rpc/historian/events/search".to_string(),
        timeout_s: 5.0,
        answers: vec![
            CallAnswer {
                origin: ORIGIN.into(),
                outcome: CallOutcome::Ok {
                    value: Some(serde_json::json!({
                        "items": [{"id": "e-41"}],
                        "next_cursor": "e-41",
                        "partial": true,
                        "scanned": 4096,
                        "covers_from": "2026-09-06T10:00:00Z"
                    })),
                    text: None,
                },
                attachment: None,
                attachment_bytes: None,
            },
            CallAnswer {
                origin: "h-bbbbbbbbbbbb".into(),
                outcome: CallOutcome::Ok {
                    value: Some(serde_json::json!({
                        "items": [],
                        "next_cursor": null,
                        "partial": true
                    })),
                    text: None,
                },
                attachment: None,
                attachment_bytes: None,
            },
        ],
    }
}

/// The same call, reached through a bridge — the resolution provenance is
/// what `check probe` adds over `service call` (RFC 06 §6.2).
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
        window_s: 30.0,
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
            wire_samples: Asked::Asked(3),
            still_declared: Some(true),
            subscribers: Some(1),
            replacement_samples: Asked::Asked(480),
            verdict: CutoverVerdict::OldStillSpeaks,
        },
        RetiredEntry {
            producer: "logs".into(),
            path: "logs/by_unit/{unit}/burn_rate".into(),
            since: Some("2.0".into()),
            replaced_by: Some("logs/journald/burn_rate".into()),
            selector: "v1/*/*/logs/logs/by_unit/*/burn_rate".into(),
            wire_samples: Asked::Asked(0),
            still_declared: Some(false),
            subscribers: Some(0),
            replacement_samples: Asked::Asked(120),
            verdict: CutoverVerdict::Pass,
        },
        RetiredEntry {
            producer: "logs".into(),
            path: "logs/units_in_failure".into(),
            since: Some("2.0".into()),
            replaced_by: Some("logs/journald/units_in_failure".into()),
            selector: "v1/*/*/logs/logs/units_in_failure".into(),
            wire_samples: Asked::Asked(0),
            still_declared: Some(false),
            subscribers: Some(0),
            replacement_samples: Asked::Asked(0),
            verdict: CutoverVerdict::Unproven,
        },
    ];
    RetiredReport {
        registries: vec!["../zensight/zensight-common/registry".into()],
        entries,
        window_s: Asked::Asked(30.0),
        plane_samples: Asked::Asked(960),
        dropped: Asked::Asked(5),
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
        window_s: 10.0,
        total_count: 9_600,
        total_bytes: 528_000,
        keys: 50_000,
        evicted: 912,
        max_keys: 50_000,
        sn_gaps: Asked::Asked(0),
        rows: vec![
            // R3: the row-level counters ride the same ask-gates as their
            // report-level siblings — this fixture is a `--loss --latency`
            // run where nothing was stamped.
            RateRow {
                key: format!("v1/{ORIGIN}/telemetry/sysinfo/disk/var-log/used"),
                count: 50,
                bytes: 2_250,
                sn_gaps: Asked::Asked(0),
                latency: None,
                unstamped: Asked::Asked(50),
            },
            RateRow {
                key: format!("v1/{ORIGIN}/state/sysinfo/health"),
                count: 50,
                bytes: 1_800,
                sn_gaps: Asked::Asked(0),
                latency: None,
                unstamped: Asked::Asked(0),
            },
        ],
    }
}

/// A quiet segment: an empty scout heard a **boundary**, not an absence.
pub fn scout_report_empty() -> ScoutReport {
    ScoutReport {
        asked: vec!["router".into(), "peer".into(), "client".into()],
        timeout_s: 3.0,
        heard: vec![],
    }
}

pub fn scout_report() -> ScoutReport {
    ScoutReport {
        asked: vec!["router".into()],
        timeout_s: 3.0,
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
                locators_via_links: vec![],
                answered: true,
            },
            zenkey_fleet::TopologyNode {
                zid: "eeff0011".into(),
                whatami: "peer".into(),
                version: None,
                locators: vec![],
                locators_via_links: vec![],
                answered: false,
            },
        ],
        edges: vec![zenkey_fleet::TopologyEdge {
            reporter: "aabbccdd".into(),
            peer: "eeff0011".into(),
            whatami: "peer".into(),
            region: None,
            links: vec!["tcp".into()],
        }],
    }
}

/// The consumers join (#224) with an admin space answering: one narrow
/// subscriber attributed to a session and its origin, one `**` subscriber
/// known only through its reporter, and the tool's own querier — named.
pub fn consumers_report() -> ConsumersReport {
    ConsumersReport {
        target: "acme/v1/*/state/sysinfo/health".into(),
        asked: vec![
            "@/*/*".into(),
            "@/*/*/subscriber/**".into(),
            "@/*/*/publisher/**".into(),
            "@/*/*/queryable/**".into(),
            "@/*/*/querier/**".into(),
            "@/*/*/token/**".into(),
        ],
        self_zid: "ffffffff".into(),
        admin: AdminAnswer::Answered {
            answered: 1,
            nodes: 2,
        },
        rows: vec![
            ConsumerRow {
                zid: "eeff0011".into(),
                whatami: Some("peer".into()),
                origins: vec![ORIGIN.into()],
                attribution: Attribution::Session,
                kind: EntityKind::Subscriber,
                keyexpr: format!("acme/v1/{ORIGIN}/state/sysinfo/health"),
                relation: Relation::Narrower,
                is_self: false,
                total_wildcard: false,
            },
            ConsumerRow {
                zid: "ffffffff".into(),
                whatami: Some("peer".into()),
                origins: vec![],
                attribution: Attribution::Session,
                kind: EntityKind::Querier,
                keyexpr: "acme/v1/*/state/sysinfo/health".into(),
                relation: Relation::Exact,
                is_self: true,
                total_wildcard: false,
            },
            ConsumerRow {
                zid: "aabbccdd".into(),
                whatami: Some("router".into()),
                origins: vec![],
                attribution: Attribution::ReportedOnly,
                kind: EntityKind::Subscriber,
                keyexpr: "**".into(),
                relation: Relation::Total,
                is_self: false,
                total_wildcard: true,
            },
        ],
        reply_elided: 0,
    }
}

/// The same ask with no admin space answering: *not asked*, no rows.
pub fn consumers_not_available() -> ConsumersReport {
    ConsumersReport {
        admin: AdminAnswer::NotAvailable,
        rows: vec![],
        ..consumers_report()
    }
}

/// The blast radius of one state subject (#224): the consumers above, its
/// coverage row, what else declares on it, and a ledger entry.
pub fn subject_impact() -> SubjectImpact {
    SubjectImpact {
        producer: "sysinfo".into(),
        path: "health".into(),
        class: "state".into(),
        selector: "acme/v1/*/state/sysinfo/health".into(),
        consumers: consumers_report(),
        coverage: Some(vec![CoverageRow {
            producer: "sysinfo".into(),
            path: "health".into(),
            ttl_s: Some(120),
            coverage: Coverage::Covered("main@aabbccdd".into()),
        }]),
        declared_publishers: Some(2),
        declared_queryables: Some(0),
        deprecated: Some(DeprecationFact {
            since: Some("2.0".into()),
            replaced_by: Some("status".into()),
        }),
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

/// The `why` ladder (#214), in its most instructive posture: a producer that
/// is declared and alive but has never published the subject. All three
/// answer states appear — established rungs with evidence, the
/// `publisher-declared` rung carrying the lazy-declaration wording that must
/// never read as a bug (RFC 08 §6.1), and `NotAsked` rungs that say why they
/// were not asked (RFC 09 §5.1 O4) — under the `Healthy` verdict (exit 1).
pub fn why_report() -> zenkey_fleet::WhyReport {
    use zenkey_fleet::report::{Rung, RungAnswer, RungId};
    // The question is the id's own now (#347), so this fixture no longer
    // restates it — and can no longer restate it *wrongly*.
    let rung = |id: RungId, answer, evidence: &[&str]| Rung {
        id,
        question: id.question(),
        answer,
        evidence: evidence.iter().map(|e| (*e).to_string()).collect(),
    };
    zenkey_fleet::WhyReport {
        key: format!("v1/{ORIGIN}/telemetry/sysinfo/disk/root/used"),
        base: String::new(),
        rungs: vec![
            rung(
                RungId::ScopeReach,
                RungAnswer::Established,
                &["the `v1/**` explorer scope intersects this key"],
            ),
            rung(
                RungId::KeyParse,
                RungAnswer::Established,
                &[
                    "origin h-3fa9c2d41b7e (host), class telemetry, producer sysinfo, \
                     subject disk/root/used",
                ],
            ),
            rung(
                RungId::RegistryDeclared,
                RungAnswer::Established,
                &["declared as disk/{mount}/used (TelemetryPoint)"],
            ),
            rung(
                RungId::OriginAlive,
                RungAnswer::Established,
                &["h-3fa9c2d41b7e is on the roster with producer(s): sysinfo"],
            ),
            rung(
                RungId::PublisherDeclared,
                RungAnswer::NotEstablished {
                    reason: "declared, alive, never published — publishers declare \
                             lazily (RFC 08 §6.1): no publisher declaration exists \
                             until the first publication, so this is not evidence \
                             of a bug"
                        .into(),
                },
                &[],
            ),
            rung(
                RungId::StorageCoverage,
                RungAnswer::Established,
                &[
                    "storage latest@aabbccdd (v1/*/telemetry/**) captures every key \
                   this expression names",
                ],
            ),
            rung(
                RungId::StoredValue,
                RungAnswer::NotEstablished {
                    reason: "none of get, @adv cache returned a value — which is \
                             silence, not proof no value exists (RFC 05 §3.1)"
                        .into(),
                },
                &[],
            ),
            rung(
                RungId::SampleFreshness,
                RungAnswer::NotAsked,
                &["no sample in hand to age — the stored-value rung found none"],
            ),
            rung(
                RungId::AdminAnswered,
                RungAnswer::Established,
                &["1 admin root document(s) answered @/*/*"],
            ),
            rung(
                RungId::WireHeard,
                RungAnswer::NotAsked,
                &["not listened — the data plane costs one deliberate action \
                     (RFC 09 §5.1, v1.18 frugality); pass --for <SECS> to watch \
                     the wire"],
            ),
        ],
        verdict: zenkey_fleet::WhyVerdict::Healthy,
        impairments: vec![],
        listened_s: None,
    }
}

/// The RFC 09 §2 sketch as a plan (#393): two volumes, three storages, the
/// documented `catalog`/`pdns_history` overlap, a refused `complete`, and one
/// storage refused outright.
pub fn storage_plan() -> StoragePlan {
    use std::collections::BTreeMap;
    let params = |pairs: &[(&str, &str)]| -> BTreeMap<String, serde_json::Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), serde_json::json!(v)))
            .collect()
    };
    let warn = |kind: WarningKind, text: &str, cite: &str| PlanWarning {
        kind,
        text: text.into(),
        cite: cite.into(),
    };
    StoragePlan {
        base: "acme".into(),
        registry: Asked::Asked(RegistryFacts {
            slices: 3,
            max_ttl_s: Some(900),
            ttl_source: Some("sysinfo/alert/{alert_key}".into()),
        }),
        volumes: vec![
            PlannedVolume {
                id: "fs".into(),
                plugin: "fs".into(),
                history: HistoryMode::Latest,
                persistence: Some(Persistence::Durable),
                params: BTreeMap::new(),
                warnings: vec![],
            },
            PlannedVolume {
                id: "influxdb".into(),
                plugin: "influxdb".into(),
                history: HistoryMode::All,
                persistence: Some(Persistence::Durable),
                params: params(&[("url", "http://localhost:8086")]),
                warnings: vec![],
            },
        ],
        storages: vec![
            PlannedStorage {
                name: "catalog".into(),
                class: Some(StorageClass::Catalog),
                key_expr: "acme/v1/@catalog/state/**".into(),
                strip_prefix: "acme/v1/@catalog/state".into(),
                volume: "fs".into(),
                history: HistoryMode::Latest,
                replication: None,
                complete: false,
                garbage_collection: GarbageCollection {
                    period_s: 30,
                    lifespan_s: 172_800,
                    derivation: "max ttl_s 86400 (catalog/pdns/{ip_slug}) × 2.0 = 172800 s".into(),
                },
                retention: None,
                params: params(&[("dir", "catalog")]),
                covers: Asked::Asked(3),
                warnings: vec![
                    warn(
                        WarningKind::CompleteRefused,
                        "complete = true refused: it is not the fully covering latest storage (class state) — emitted as false",
                        "RFC 09 §2.2",
                    ),
                    warn(
                        WarningKind::Overlap,
                        "overlaps pdns_history (acme/v1/@catalog/state/pdns/**): a GET under both selectors is answered by both",
                        "RFC 09 §2",
                    ),
                ],
            },
            PlannedStorage {
                name: "latest".into(),
                class: Some(StorageClass::State),
                key_expr: "acme/v1/*/state/**".into(),
                strip_prefix: "acme/v1".into(),
                volume: "fs".into(),
                history: HistoryMode::Latest,
                replication: Some(
                    [
                        ("interval", serde_json::json!(10.0)),
                        ("propagation_delay", serde_json::json!(250)),
                    ]
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
                ),
                complete: true,
                garbage_collection: GarbageCollection {
                    period_s: 30,
                    lifespan_s: 1800,
                    derivation: "max ttl_s 900 (sysinfo/alert/{alert_key}) × 2.0 = 1800 s".into(),
                },
                retention: None,
                params: params(&[("dir", "latest")]),
                covers: Asked::Asked(12),
                warnings: vec![],
            },
            PlannedStorage {
                name: "pdns_history".into(),
                class: Some(StorageClass::CatalogPdns),
                key_expr: "acme/v1/@catalog/state/pdns/**".into(),
                strip_prefix: "acme/v1/@catalog/state/pdns".into(),
                volume: "influxdb".into(),
                history: HistoryMode::All,
                replication: None,
                complete: false,
                garbage_collection: GarbageCollection {
                    period_s: 30,
                    lifespan_s: 172_800,
                    derivation: "max ttl_s 86400 (catalog/pdns/{ip_slug}) × 2.0 = 172800 s".into(),
                },
                retention: None,
                params: params(&[("db", "pdns")]),
                covers: Asked::Asked(1),
                warnings: vec![warn(
                    WarningKind::RetentionIsTheDatabases,
                    "retention is the database's policy, not zenoh config",
                    "RFC 09 §2.3",
                )],
            },
        ],
        refusals: vec![Refusal {
            storage: Some("events".into()),
            volume: None,
            key_expr: Some("acme/v1/*/events/**".into()),
            reason: "the registry declares no subject under \"acme/v1/*/events/**\" — empty coverage is a finding, not a plan".into(),
            cite: "RFC 13 §3".into(),
        }],
    }
}

/// A check with one of each finding kind the diff can draw, and one
/// comparison the admin document could not carry.
pub fn storage_check() -> StorageCheck {
    let finding =
        |kind, storage: &str, zid: Option<&str>, planned: Option<&str>, observed: Option<&str>| {
            CheckFinding {
                kind,
                storage: storage.into(),
                zid: zid.map(Into::into),
                planned: planned.map(Into::into),
                observed: observed.map(Into::into),
            }
        };
    StorageCheck {
        base: "acme".into(),
        asked: "@/*/router/**/storage_manager/storages/**".into(),
        planned: 3,
        observed: 3,
        findings: vec![
            finding(
                CheckKind::StripPrefixDiffers,
                "latest",
                Some("aabbccdd"),
                Some("acme/v1"),
                Some("acme"),
            ),
            finding(
                CheckKind::LifespanBelowMinimum,
                "latest",
                Some("aabbccdd"),
                Some("1800"),
                Some("600"),
            ),
            finding(
                CheckKind::Missing,
                "pdns_history",
                None,
                Some("acme/v1/@catalog/state/pdns/**"),
                None,
            ),
            finding(
                CheckKind::Extra,
                "blobs",
                Some("aabbccdd"),
                None,
                Some("acme/v1/*/@blob/**"),
            ),
        ],
        unjudged: vec![
            "catalog@aabbccdd: the admin document does not carry garbage_collection.lifespan"
                .into(),
        ],
        judgement: Judgement::Established,
    }
}

/// The same check against an admin space that answered nothing.
pub fn storage_check_unobservable() -> StorageCheck {
    StorageCheck {
        base: "acme".into(),
        asked: "@/*/router/**/storage_manager/storages/**".into(),
        planned: 3,
        observed: 0,
        findings: vec![],
        unjudged: vec![],
        judgement: Judgement::Unobservable {
            reason: "the admin space answered no storages".into(),
        },
    }
}

/// One key, taken by the latest storage.
pub fn storage_explain() -> StorageExplain {
    StorageExplain {
        key: "acme/v1/h-3fa9c2d41b7e/state/sysinfo/health".into(),
        base: "acme".into(),
        takers: vec![Taker {
            storage: "latest".into(),
            key_expr: "acme/v1/*/state/**".into(),
            class: Some(StorageClass::State),
            relation: TakerRelation::Includes,
            why: "class state under base \"acme\": acme/v1/*/state/** includes every key it names; stored under strip_prefix \"acme/v1\" on volume fs (latest)".into(),
        }],
        refused_takers: vec![],
        none_reason: None,
    }
}

/// A key nothing takes, because a refused storage would have.
pub fn storage_explain_none() -> StorageExplain {
    StorageExplain {
        key: "acme/v1/h-3fa9c2d41b7e/events/netring/capture/01J".into(),
        base: "acme".into(),
        takers: vec![],
        refused_takers: vec!["events".into()],
        none_reason: Some(
            "no planned storage's selector includes it; refused storage(s) events would have"
                .into(),
        ),
    }
}

// ── acl gen (#392) ────────────────────────────────────────────────────────

/// The plan of a small fleet — one host on every plane, a catalog on the
/// advanced tier, a console, a watch — with no registry asked, so the
/// write set is the convention's unnarrowed `set` leaf and the plan says so.
/// Built through the planner itself rather than by hand: what the corpora
/// pin is what the verb draws.
pub fn acl_plan() -> AclPlan {
    let enrollment: Enrollment = Enrollment {
        base: Some("zensight".into()),
        fleet: FleetSpec {
            catalog_adv: true,
            salt: None,
        },
        principal: vec![
            PrincipalSpec {
                cn: Some(ORIGIN.into()),
                role: Role::Host,
                origin: Some(ORIGIN.into()),
                adv: true,
                blob_seed: true,
                media: true,
                ..Default::default()
            },
            PrincipalSpec {
                cn: Some("zensight-catalog".into()),
                role: Role::Catalog,
                ..Default::default()
            },
            PrincipalSpec {
                cn: Some("zensight-console".into()),
                role: Role::Console,
                adv: true,
                ..Default::default()
            },
            PrincipalSpec {
                cn: Some("zensight-watch".into()),
                role: Role::Watch,
                ..Default::default()
            },
        ],
    };
    zenkey_fleet::plan_acl(
        &enrollment,
        "zensight",
        None,
        zenkey_fleet::AclOptions::default(),
    )
}

/// The same plan with one principal refused: a host enrolled with neither
/// origin nor machine-id.
pub fn acl_plan_refused() -> AclPlan {
    let mut plan = acl_plan();
    plan.refusals.push(AclRefusal {
        principal: "bare-host".into(),
        reason: "a host needs `origin` or `machine_id`".into(),
        cite: "RFC 03 §4 D6".into(),
    });
    plan
}

/// A check with two findings: the shared `interest-prop` rule missing (the
/// fifth fact's failure, exactly) and a CN the enrollment never enrolled.
pub fn acl_check() -> AclCheck {
    AclCheck {
        base: "zensight".into(),
        against: "router.json5".into(),
        planned_rules: 15,
        observed_rules: 10,
        planned_subjects: 4,
        observed_subjects: 5,
        findings: vec![
            AclFinding {
                kind: AclFindingKind::RuleMissing,
                id: "interest-prop".into(),
                planned: Some(
                    "allow egress declare_liveliness_subscriber,declare_subscriber,liveliness_query,query zensight/v1/**".into(),
                ),
                observed: None,
            },
            AclFinding {
                kind: AclFindingKind::UnknownCn,
                id: "stranger.example".into(),
                planned: None,
                observed: Some("bound by subject \"stranger\"".into()),
            },
        ],
        interest_probe: Judgement::NotAsked,
        judgement: Judgement::Established,
    }
}

/// A clean check.
pub fn acl_check_clean() -> AclCheck {
    AclCheck {
        findings: vec![],
        observed_rules: 15,
        observed_subjects: 4,
        judgement: Judgement::NotEstablished {
            reason: "router.json5 carries the plan whole: 15 rule(s), 4 subject(s), 4 polic(y/ies), enabled, default deny".into(),
        },
        ..acl_check()
    }
}

/// The console asking a write procedure: denied on ingress by the deny that
/// beat `ops-sub`, and nothing at all on egress.
pub fn acl_explain() -> AclExplain {
    zenkey_fleet::explain_acl(
        &acl_plan(),
        "zensight-console",
        "zensight/v1/h-3fa9c2d41b7e/@rpc/systemd/action/set",
        AclMessage::Query,
    )
    .expect("an enrolled principal and a valid key")
}

// ── The fleet timeline (#216) ─────────────────────────────────────────────

fn timeline_lane() -> LaneId {
    LaneId::Origin {
        origin: ORIGIN.into(),
        producer: Some("sysinfo".into()),
    }
}

fn timeline_sample(
    order_by: OrderLabel,
    pos: usize,
    lane: LaneId,
    key: &str,
    t_us: u64,
    hlc: Option<(u64, &str)>,
) -> TimelineEntry {
    TimelineEntry::Sample {
        order_by,
        pos,
        lane,
        key: key.into(),
        t_us,
        hlc: hlc.map(|(n, s)| format!("{n}/{s}")),
        stamped_by: hlc.map(|(_, s)| s.to_string()),
        provenance: hlc.map(|_| Provenance::Unattributable),
        kind: RowKind::Put,
    }
}

const TL_A: &str = "acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu";
const TL_B: &str = "acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/mem";
const TL_PLAIN: &str = "plain/key";

/// A ten-second window on the arrival axis: two stamped samples from one
/// producer that arrived in the *opposite* order to their HLCs, an
/// unstamped sample in its own lane, and a drop of 3 between the second
/// and third — the reorder is visible against [`timeline_report_hlc`].
pub fn timeline_report_arrival() -> TimelineReport {
    let lane = timeline_lane();
    TimelineReport {
        order_by: OrderLabel::Arrival,
        axis: AxisLabel::Arrival {
            clock: ARRIVAL_CLOCK,
        },
        scopes: vec!["acme/v1/**".into()],
        window_s: Some(10.0),
        source: TimelineSource::Live,
        lanes: vec![
            LaneSummary {
                lane: lane.clone(),
                samples: 2,
                first_t_us: 1_000,
                last_t_us: 2_000,
                stampers: ["33".to_string()].into_iter().collect(),
                provenance: ProvenanceCounts {
                    self_stamped: 0,
                    foreign: 0,
                    unattributable: 2,
                },
            },
            LaneSummary {
                lane: LaneId::Unstamped,
                samples: 1,
                first_t_us: 3_000,
                last_t_us: 3_000,
                stampers: Default::default(),
                provenance: ProvenanceCounts::default(),
            },
        ],
        sn_lane: SnLaneReport::Unavailable {
            reason: SN_UNAVAILABLE_REASON,
        },
        unstamped_excluded: 0,
        dropped: 3,
        coalesced: 0,
        keys_evicted: 0,
        rows: vec![
            timeline_sample(
                OrderLabel::Arrival,
                0,
                lane.clone(),
                TL_A,
                1_000,
                Some((200, "33")),
            ),
            timeline_sample(
                OrderLabel::Arrival,
                1,
                lane.clone(),
                TL_B,
                2_000,
                Some((100, "33")),
            ),
            TimelineEntry::Break {
                order_by: OrderLabel::Arrival,
                pos: 2,
                lane: None,
                kind: BreakKind::Dropped,
                n: 3,
            },
            timeline_sample(
                OrderLabel::Arrival,
                3,
                LaneId::Unstamped,
                TL_PLAIN,
                3_000,
                None,
            ),
        ],
    }
}

/// The same window on the HLC axis: one stamper, so the order is that
/// node's happened-before; the unstamped sample is *excluded*, not placed;
/// the drop is a total with no position on this clock.
pub fn timeline_report_hlc() -> TimelineReport {
    let lane = timeline_lane();
    TimelineReport {
        order_by: OrderLabel::Hlc,
        axis: AxisLabel::Hlc {
            claim: HlcClaim::HappensBefore {
                stamper: "33".into(),
            },
        },
        scopes: vec!["acme/v1/**".into()],
        window_s: Some(10.0),
        source: TimelineSource::Live,
        lanes: vec![LaneSummary {
            lane: lane.clone(),
            samples: 2,
            first_t_us: 1_000,
            last_t_us: 2_000,
            stampers: ["33".to_string()].into_iter().collect(),
            provenance: ProvenanceCounts {
                self_stamped: 0,
                foreign: 0,
                unattributable: 2,
            },
        }],
        sn_lane: SnLaneReport::Unavailable {
            reason: SN_UNAVAILABLE_REASON,
        },
        unstamped_excluded: 1,
        dropped: 3,
        coalesced: 0,
        keys_evicted: 0,
        rows: vec![
            timeline_sample(
                OrderLabel::Hlc,
                0,
                lane.clone(),
                TL_B,
                2_000,
                Some((100, "33")),
            ),
            timeline_sample(OrderLabel::Hlc, 1, lane, TL_A, 1_000, Some((200, "33"))),
        ],
    }
}

// ─── snapshots (RFC 13 §4.4, #219) ───────────────────────────────────────

/// The second origin the two-origin snapshots below carry.
pub const ORIGIN_B: &str = "h-9b2e4c7a1d05";

fn zsnap_header(collected_at: &str, span: f64, answered: u64, superseded: u64) -> ZsnapHeader {
    ZsnapHeader {
        zsnap: 1,
        selectors: vec!["acme/v1/**".into()],
        base: "acme".into(),
        collected_at: collected_at.into(),
        collection_span_s: span,
        asked: 1,
        answered,
        elided: 0,
        errors: 0,
        superseded,
        roster: Asked::Asked(2),
    }
}

fn snapshot_row(key: &str, bytes: &str, holder: Holder) -> SnapshotRow {
    SnapshotRow {
        key: key.into(),
        delete: false,
        bytes: Some(bytes.into()),
        encoding: Some("application/json".into()),
        timestamp: Some("7f3b2a1c00000001/ab12".into()),
        stamper: Some(StamperWire::Unattributable { id: "ab12".into() }),
        source: None,
        source_zid: Some("ab12".into()),
        registration: RegistrationWire::Registered,
        verdict: VerdictWire::Valid,
        holder,
    }
}

/// Two origins under `acme`, five rows: a live host answering its own
/// health, a live host's telemetry answered by a storage, a second host
/// remembered only by a storage, a tombstone, and a leaked non-v1 key
/// (O1). The `state/sysinfo/health` bodies carry `source` and `host_id` —
/// the labels an origin alignment can read (chunk DD).
pub fn snapshot() -> Snapshot {
    Snapshot {
        header: zsnap_header("2026-09-06T00:00:00Z", 1.25, 6, 1),
        rows: vec![
            snapshot_row(
                "acme/plain/leak",
                "bGVha2Vk",
                Holder::Unattributed {
                    reason: "the key names no origin: not a v1 key".into(),
                },
            )
            .into_leak(),
            snapshot_row(
                "acme/v1/h-3fa9c2d41b7e/state/sysinfo/health",
                "eyJzb3VyY2UiOiJub2RlLWEiLCJob3N0X2lkIjoiaC0zZmE5YzJkNDFiN2UiLCJzdGF0dXMiOiJvayJ9",
                Holder::Live {
                    origin: ORIGIN.into(),
                    answered_by: AnsweredBy::Stamper,
                },
            ),
            snapshot_row(
                "acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used",
                "eyJ2YWx1ZSI6NDEuMCwidW5pdCI6InBlcmNlbnQifQ==",
                Holder::Live {
                    origin: ORIGIN.into(),
                    answered_by: AnsweredBy::Other,
                },
            ),
            SnapshotRow {
                delete: true,
                bytes: None,
                encoding: None,
                verdict: VerdictWire::NotValidated {
                    reason: "tombstone".into(),
                },
                ..snapshot_row(
                    "acme/v1/h-9b2e4c7a1d05/state/logs/rotated",
                    "",
                    Holder::StorageOnly {
                        origin: ORIGIN_B.into(),
                    },
                )
            },
            snapshot_row(
                "acme/v1/h-9b2e4c7a1d05/state/sysinfo/health",
                "eyJzb3VyY2UiOiJub2RlLWIiLCJob3N0X2lkIjoiaC05YjJlNGM3YTFkMDUiLCJzdGF0dXMiOiJkZWdyYWRlZCJ9",
                Holder::StorageOnly {
                    origin: ORIGIN_B.into(),
                },
            ),
        ],
    }
}

/// The leaked row's non-v1 facets, applied after the shared constructor.
trait IntoLeak {
    fn into_leak(self) -> SnapshotRow;
}

impl IntoLeak for SnapshotRow {
    fn into_leak(mut self) -> SnapshotRow {
        self.encoding = Some("text/plain".into());
        self.timestamp = None;
        self.stamper = None;
        self.registration = RegistrationWire::NotV1;
        self.verdict = VerdictWire::NotValidated {
            reason: "no_schema".into(),
        };
        self
    }
}

/// The same fleet five minutes on: the disk value moved, the second host
/// came up and recovered, the tombstoned key is gone, and the second host
/// grew a telemetry key. The leaked key is untouched.
pub fn snapshot_b() -> Snapshot {
    let mut b = snapshot();
    b.header = zsnap_header("2026-09-06T00:05:00Z", 0.8, 5, 0);
    b.rows.retain(|r| !r.delete);
    for row in &mut b.rows {
        match row.key.as_str() {
            "acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used" => {
                row.bytes = Some("eyJ2YWx1ZSI6NDIuMCwidW5pdCI6InBlcmNlbnQifQ==".into());
                row.timestamp = Some("7f3b2a1c00000002/ab12".into());
            }
            "acme/v1/h-9b2e4c7a1d05/state/sysinfo/health" => {
                row.bytes = Some(
                    "eyJzb3VyY2UiOiJub2RlLWIiLCJob3N0X2lkIjoiaC05YjJlNGM3YTFkMDUiLCJzdGF0dXMiOiJvayJ9"
                        .into(),
                );
                row.timestamp = Some("7f3b2a1c00000002/cd34".into());
                row.holder = Holder::Live {
                    origin: ORIGIN_B.into(),
                    answered_by: AnsweredBy::Unknown,
                };
            }
            _ => {}
        }
    }
    b.rows.push(snapshot_row(
        "acme/v1/h-9b2e4c7a1d05/telemetry/sysinfo/disk/var-log/used",
        "eyJ2YWx1ZSI6Ny41LCJ1bml0IjoicGVyY2VudCJ9",
        Holder::Live {
            origin: ORIGIN_B.into(),
            answered_by: AnsweredBy::Unknown,
        },
    ));
    b.rows.sort_by(|x, y| x.key.cmp(&y.key));
    b
}

/// What taking [`snapshot`] reported: written to a file, every holder
/// counted.
pub fn snapshot_report() -> SnapshotReport {
    SnapshotReport {
        header: snapshot().header,
        out: Some("fleet.zsnap".into()),
        live: 2,
        storage_only: 2,
        unattributed: 1,
        incomplete: Vec::new(),
    }
}

/// [`snapshot`] against [`snapshot_b`], through the engine's own comparison
/// at its default bounds — no origin alignment asked.
pub fn snapshot_diff() -> SnapshotDiff {
    zenkey_fleet::diff_snapshots(
        &snapshot(),
        &snapshot_b(),
        zenkey_fleet::DiffOpts::default(),
    )
}

// ─── two deployments, one diff (#220) ────────────────────────────────────

/// The two origins of the *other* deployment in the pairs below: the same
/// fleet as [`snapshot_pair_renamed`]'s `a`, every origin re-minted.
pub const ORIGIN_C: &str = "h-c0ffee00c0de";
pub const ORIGIN_D: &str = "h-0badcafe1234";

/// One host of a two-deployment fixture: its health document (the identity
/// bridge, `host_id` naming the origin it sits under and `source` naming
/// the host), one telemetry key, and any extra producers.
fn fleet_host(
    base: &str,
    origin: &str,
    label: &str,
    extra: &[&str],
    stamp: &str,
) -> Vec<SnapshotRow> {
    use base64::Engine as _;
    let b64 = |body: String| base64::engine::general_purpose::STANDARD.encode(body);
    let key = |rel: &str| format!("{base}/v1/{origin}/{rel}");
    let live = Holder::Live {
        origin: origin.into(),
        answered_by: AnsweredBy::Stamper,
    };
    let mut rows = vec![
        snapshot_row(
            &key("state/sysinfo/health"),
            &b64(format!(
                r#"{{"host_id":"{origin}","source":"{label}","status":"ok"}}"#
            )),
            live.clone(),
        ),
        snapshot_row(
            &key("telemetry/sysinfo/disk/var-log/used"),
            &b64(r#"{"value":41.0,"unit":"percent"}"#.into()),
            live.clone(),
        ),
    ];
    for p in extra {
        rows.push(snapshot_row(
            &key(&format!("state/{p}/rotated")),
            &b64(r#"{"count":3}"#.into()),
            live.clone(),
        ));
    }
    for r in &mut rows {
        r.timestamp = Some(stamp.into());
    }
    rows
}

/// A two-host fleet under `base`: `web` (`sysinfo` only) and `db`
/// (`sysinfo` + `logs`), plus a leaked bus-root key both deployments carry
/// — under no base, so it compares verbatim on both sides (O1).
fn fleet(base: &str, web: &str, db: &str, labels: (&str, &str), stamp: &str, at: &str) -> Snapshot {
    let mut rows = fleet_host(base, web, labels.0, &[], stamp);
    rows.extend(fleet_host(base, db, labels.1, &["logs"], stamp));
    rows.push(
        snapshot_row(
            "plain/leak",
            "bGVha2Vk",
            Holder::Unattributed {
                reason: "the key names no origin: not a v1 key".into(),
            },
        )
        .into_leak(),
    );
    rows.sort_by(|x, y| x.key.cmp(&y.key));
    let n = rows.len() as u64;
    Snapshot {
        header: ZsnapHeader {
            selectors: vec![format!("{base}/v1/**")],
            base: base.into(),
            ..zsnap_header(at, 0.9, n, 0)
        },
        rows,
    }
}

/// The acceptance pair (#220): one fleet, two deployments. `a` is `prod`
/// with [`ORIGIN`] (`web`) and [`ORIGIN_B`] (`db`); `b` is `stg` with
/// [`ORIGIN_C`] and [`ORIGIN_D`] under the same labels — different base,
/// different origins, different clocks, the same values. Verbatim they
/// share nothing; aligned they diff to zero.
pub fn snapshot_pair_renamed() -> (Snapshot, Snapshot) {
    (
        fleet(
            "prod",
            ORIGIN,
            ORIGIN_B,
            ("web", "db"),
            "7f3b2a1c00000001/ab12",
            "2026-09-06T00:00:00Z",
        ),
        fleet(
            "stg",
            ORIGIN_C,
            ORIGIN_D,
            ("web", "db"),
            "7f3b2a1c00000009/ef56",
            "2026-09-06T00:05:00Z",
        ),
    )
}

/// The pair the alignment must refuse: `b`'s two hosts both call
/// themselves `node`, and both publish only `sysinfo` — no label and no
/// producer set tells them apart. `a` is [`snapshot_pair_renamed`]'s.
pub fn snapshot_pair_ambiguous() -> (Snapshot, Snapshot) {
    let (a, _) = snapshot_pair_renamed();
    let mut rows = fleet_host("stg", ORIGIN_C, "node", &[], "7f3b2a1c00000009/ef56");
    rows.extend(fleet_host(
        "stg",
        ORIGIN_D,
        "node",
        &[],
        "7f3b2a1c00000009/ef56",
    ));
    rows.sort_by(|x, y| x.key.cmp(&y.key));
    let n = rows.len() as u64;
    let b = Snapshot {
        header: ZsnapHeader {
            selectors: vec!["stg/v1/**".into()],
            base: "stg".into(),
            ..zsnap_header("2026-09-06T00:05:00Z", 0.9, n, 0)
        },
        rows,
    };
    (a, b)
}

/// The renamed pair with no health documents on either side: the label
/// was never asked, and the two distinct producer sets are the only
/// evidence left.
pub fn snapshot_pair_unlabelled() -> (Snapshot, Snapshot) {
    let (mut a, mut b) = snapshot_pair_renamed();
    for s in [&mut a, &mut b] {
        s.rows.retain(|r| !r.key.ends_with("/health"));
        s.header.answered = s.rows.len() as u64;
    }
    (a, b)
}

/// [`snapshot_pair_renamed`] aligned: two pairs on their labels, nothing
/// unpaired, zero differences — exit 0, with every subject rolled up.
pub fn snapshot_diff_aligned() -> SnapshotDiff {
    let (a, b) = snapshot_pair_renamed();
    let plan = zenkey_fleet::plan_map(
        &zenkey_fleet::origin_profiles(&a),
        &zenkey_fleet::origin_profiles(&b),
        &[],
    )
    .unwrap();
    zenkey_fleet::diff_normalized(&a, &b, &plan, zenkey_fleet::DiffOpts::default())
}

/// [`snapshot_pair_ambiguous`] with one explicit `--map`: the pair the
/// operator stated rides as `explicit`, and the alignment still cannot
/// place `a`'s `db` or `b`'s second `node` — so the comparison is refused,
/// the two unpaired origins listed, never dropped (RFC 13 §4.4), and the
/// roll-up not asked. Exit 2.
pub fn snapshot_diff_unmapped() -> SnapshotDiff {
    let (a, b) = snapshot_pair_ambiguous();
    let plan = zenkey_fleet::plan_map(
        &zenkey_fleet::origin_profiles(&a),
        &zenkey_fleet::origin_profiles(&b),
        &[(
            zenkey::origin::HostId::parse(ORIGIN).unwrap(),
            zenkey::origin::HostId::parse(ORIGIN_C).unwrap(),
        )],
    )
    .unwrap();
    zenkey_fleet::diff_normalized(&a, &b, &plan, zenkey_fleet::DiffOpts::default())
}

/// [`snapshot_pair_ambiguous`] with both pairs stated: the comparison
/// runs, and the subject roll-up says where the deployments disagree —
/// every health document (the labels differ), and `logs` only in `a`.
/// Exit 1.
pub fn snapshot_diff_normalized() -> SnapshotDiff {
    let (a, b) = snapshot_pair_ambiguous();
    let plan = zenkey_fleet::plan_map(
        &zenkey_fleet::origin_profiles(&a),
        &zenkey_fleet::origin_profiles(&b),
        &[
            (
                zenkey::origin::HostId::parse(ORIGIN).unwrap(),
                zenkey::origin::HostId::parse(ORIGIN_C).unwrap(),
            ),
            (
                zenkey::origin::HostId::parse(ORIGIN_B).unwrap(),
                zenkey::origin::HostId::parse(ORIGIN_D).unwrap(),
            ),
        ],
    )
    .unwrap();
    zenkey_fleet::diff_normalized(&a, &b, &plan, zenkey_fleet::DiffOpts::default())
}

/// [`snapshot`] against itself: the clean answer, exit 0.
pub fn snapshot_diff_identity() -> SnapshotDiff {
    zenkey_fleet::diff_snapshots(&snapshot(), &snapshot(), zenkey_fleet::DiffOpts::default())
}
