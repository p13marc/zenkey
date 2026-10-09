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

/// zk2's doctor (#612, FJ6): every verdict pole — a finding of each
/// severity, clean checks, one left unobservable with the subject it could
/// not decide, one not asked — over a scope whose every count is non-zero,
/// because a renderer that sums two counts passes any fixture where one is
/// zero (tooling guide §7).
pub fn doctor_report() -> DoctorReport {
    let netem = format!("sha256:{}", "e".repeat(64));
    let finding = |check, severity, subject: &str, evidence: &str| DoctorFinding {
        severity,
        check,
        subject: subject.into(),
        evidence: evidence.into(),
    };
    let clean = |check, why: &str| CheckReport::of(check, vec![], vec![], why);
    DoctorReport {
        scope: DoctorScope {
            namespace: "acme".into(),
            presence: Asked::Asked(DoctorPresence {
                selector: "zk2/*/*/@zk/**".into(),
                complete: true,
                grace_s: 2.0,
                services: 3,
                instances: 4,
                tokens: 9,
                undescribed: 1,
                revisions: 3,
                held: 2,
            }),
            routers: Asked::Asked(2),
        },
        checks: vec![
            CheckReport::of(
                CheckId::SplitBrain,
                vec![finding(
                    CheckId::SplitBrain,
                    DoctorSeverity::Error,
                    "host-a/tc tc.netif.v1",
                    "2 instances hold its interface token in two presence reads 2.0s apart \
                     (3fa9c2d41b7e0012, 3fa9c2d41b7e0013), and at least two expose an exclusive \
                     resource",
                )],
                vec![],
                "unused",
            ),
            CheckReport::of(
                CheckId::BindingUnsatisfied,
                vec![finding(
                    CheckId::BindingUnsatisfied,
                    DoctorSeverity::Warning,
                    "ws-01/tcgui-frontend scenario",
                    "its bindings (*/tc) select no provider of tc.scenario.v1 visible to this \
                     reader",
                )],
                vec![],
                "unused",
            ),
            CheckReport::of(
                CheckId::ContractDrift,
                vec![],
                vec![Unjudged {
                    subject: "tc.netem.v1 eeeeeeeeeeeeeeee 5d1c0a9b2e3f4a6b".into(),
                    reason: format!("not classified: tc.netem.v1 {netem} is unavailable"),
                }],
                "unused",
            ),
            CheckReport::of(
                CheckId::ContractUnavailable,
                vec![finding(
                    CheckId::ContractUnavailable,
                    DoctorSeverity::Error,
                    &format!("tc.netem.v1 {netem}"),
                    "named by host-b/tc@3fa9c2d41b7e0014; no holder served a bundle that \
                     verified (no reply)",
                )],
                vec![],
                "unused",
            ),
            clean(
                CheckId::DescriptorInvalid,
                "3 descriptor(s) pass the descriptor check against the contracts they name",
            ),
            clean(
                CheckId::TokenMissing,
                "3 instance(s): every token agrees with its descriptor",
            ),
            clean(
                CheckId::PresenceOverBudget,
                "9 token(s) visible to this reader in the presence domain; within the budget \
                 10000",
            ),
            clean(
                CheckId::StorageOnState,
                "1 storage(s) on 2 router(s), none answering on an owner's state keys",
            ),
            clean(
                CheckId::ArchiveUnaligned,
                "no archive.v1 provider visible to this reader: nothing to align",
            ),
            CheckReport::not_asked(CheckId::StateStampForeign),
            CheckReport::of(
                CheckId::ShmMemlockLow,
                vec![finding(
                    CheckId::ShmMemlockLow,
                    DoctorSeverity::Info,
                    "this host",
                    "RLIMIT_MEMLOCK is 64 KiB, below the 8 MiB floor",
                )],
                vec![],
                "unused",
            ),
            clean(
                CheckId::AdminUnreachable,
                "2 router(s) answered `@/*/router`",
            ),
            clean(CheckId::RouterVersionSkew, "2 router(s), all at 1.10.1"),
        ],
        unobservable: None,
    }
}

/// A `zenctl field` window (#223; zk2's since #612, FJ8b): one path the
/// declared type never declares (`field-new`), one declared small-domain
/// path beside it, one key no contract resolved, and a path table that hit
/// its bound — the report must carry the bound's cost, not just its rows.
pub fn field_report() -> FieldReport {
    let key = "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0".to_owned();
    FieldReport {
        selector: "zk2/*/tc/tc.netif.v1/state/**".into(),
        window_s: 30.0,
        lens: LensScope {
            namespace: String::new(),
            presence: Some(LensPresence {
                selector: "zk2/*/*/@zk/**".into(),
                complete: true,
                services: 1,
            }),
            contracts: 1,
        },
        samples: 42,
        keys_seen: 2,
        dropped: 0,
        undocumented: 2,
        unread: 0,
        unresolved: 3,
        paths: 2,
        max_paths: 2,
        paths_dropped: 3,
        paths_dropped_examples: vec![format!("{key} · debug.trace")],
        rows: vec![
            FieldRow {
                key: key.clone(),
                path: "mtu".into(),
                declared: Some(true),
                seen: 40,
                documents: 40,
                kinds: vec!["number".into()],
                changes: 0,
                last_change_s: None,
                min: Some(1500.0),
                max: Some(1500.0),
                last: Some(1500.0),
                values: Some(vec!["1500".into()]),
            },
            FieldRow {
                key: key.clone(),
                path: "driver_hint".into(),
                declared: Some(false),
                seen: 40,
                documents: 40,
                kinds: vec!["string".into()],
                changes: 1,
                last_change_s: Some(12.0),
                min: None,
                max: None,
                last: None,
                values: Some(vec!["\"e1000\"".into(), "\"virtio\"".into()]),
            },
        ],
        findings: vec![FieldFinding {
            severity: DoctorSeverity::Warning,
            check: FieldCheck::New,
            subject: format!("{key} · driver_hint"),
            evidence: "present in 40 of 40 document sample(s) but never declared by the \
                       contract's type json:NetworkInterface — drift at field granularity"
                .into(),
        }],
    }
}

/// A zk2 fan-out benched (#612, FJ8a): latency per replier key, and every
/// non-answer counted apart from it, each count non-zero so none can hide
/// in another — averaging a non-answer into a latency figure is how a
/// benchmark lies. Refusals, malformed envelopes, the transport's errors,
/// silent calls, R6's discards and a call that panicked inside the tool
/// (#329) are seven populations; a holder that sent no value in every call
/// is the attribution silence gets.
pub fn bench_report() -> BenchReport {
    let latency = |min_ms, p50_ms, p95_ms, p99_ms, max_ms| Latency {
        min_ms,
        p50_ms,
        p95_ms,
        p99_ms,
        max_ms,
    };
    BenchReport {
        address: "*/tc".into(),
        iface: "tc.netif.v1".into(),
        fingerprint: format!("sha256:{}", "4f".repeat(32)),
        operation: "@op/diagnostics".into(),
        values: std::collections::BTreeMap::new(),
        selectors: vec!["zk2/*/tc/tc.netif.v1/@op/diagnostics".into()],
        mode: CallMode::Fanout,
        requested: 100,
        completed: 98,
        concurrency: 8,
        timeout_s: 2.0,
        elapsed_s: 2.5,
        calls_per_s: 39.2,
        clock: LatencyClock::RoundTrip,
        repliers: vec![
            ReplierLatency {
                address: "host-a/tc".into(),
                key: "zk2/host-a/tc/tc.netif.v1/@op/diagnostics".into(),
                replies: 64,
                latency: latency(0.8, 1.9, 12.4, 40.1, 123.456),
            },
            ReplierLatency {
                address: "host-b/tc".into(),
                key: "zk2/host-b/tc/tc.netif.v1/@op/diagnostics".into(),
                replies: 34,
                latency: latency(1.1, 2.2, 9.9, 11.0, 12.5),
            },
        ],
        refusals: RefusalTally {
            count: 2,
            codes: [("busy".to_owned(), 2)].into(),
            latency: Some(latency(0.4, 0.5, 0.6, 0.6, 0.6)),
        },
        malformed: 1,
        transport: 3,
        silent: 1,
        discarded: 4,
        panicked: 1,
        presence: BenchPresence {
            selector: "zk2/*/tc/@zk/alive/tc.netif.v1/**".into(),
            complete: true,
            error: None,
            holders: vec![
                HolderTally {
                    address: "host-a/tc".into(),
                    without_value: 34,
                },
                HolderTally {
                    address: "host-c/tc".into(),
                    without_value: 98,
                },
            ],
        },
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

/// A probe that heard nothing from a provider presence shows up (#612,
/// FJ8b): attributable silence, the finding.
pub fn probe_report() -> ProbeReport {
    ProbeReport {
        address: "host-a/tc".into(),
        iface: "tc.netif.v1".into(),
        fingerprint: format!("sha256:{}", "ab".repeat(32)),
        resource: "state/interfaces/{ns}/{iface}".into(),
        selectors: vec!["zk2/host-a/tc/tc.netif.v1/state/interfaces/*/*".into()],
        window_s: 5.0,
        elapsed_s: 5.0,
        current: Some(ProbeCurrent {
            answered: 0,
            conforming: 0,
            error: None,
        }),
        received: 0,
        conforming: 0,
        nonconforming: 0,
        keys_seen: 0,
        lagged: 0,
        discarded: 2,
        unresolved: 1,
        first: None,
        presence: Some(ExpectPresence {
            selector: "zk2/host-a/tc/@zk/alive/tc.netif.v1/**".into(),
            holders: vec!["host-a/tc".into()],
            complete: true,
            error: None,
        }),
        verdict: Judgement::Established,
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
        address: "*/tc".into(),
        iface: "tc.netif.v1".into(),
        fingerprint: format!("sha256:{}", "ab".repeat(32)),
        resource: Some("stream/bandwidth/{ns}/{iface}".into()),
        selectors: vec!["zk2/*/tc/tc.netif.v1/stream/bandwidth/*/*".into()],
        window_s: 5.0,
        ended_early: false,
        samples: 120,
        keys_seen: 4,
        dropped: 17,
        discarded: 3,
        unresolved: 2,
        rate_hz: Some(24.0),
        presence: None,
        violations: vec![
            "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0: priority declared data, \
             observed real_time"
                .into(),
        ],
        violations_total: 9,
        unmet: vec!["17 sample(s) were dropped while behind".into()],
        verdict: ExpectVerdict::Impaired,
    }
}

/// A `check conform` run (#222) with one assertion of every kind: met, met
/// and exempt, not met, and unknowable with its reason — over a window that
/// dropped, so the lower-bound caveat has something to say.
pub fn conform_report() -> ConformReport {
    let assertions = vec![
        Assertion {
            id: "procedure/introspect".into(),
            subject: "@rpc/introspect".into(),
            state: AssertionState::Met,
            evidence: format!("{ORIGIN}: a value reply"),
            citation: Some("RFC 08 §6".into()),
            exempt: None,
        },
        Assertion {
            id: "procedure/dns".into(),
            subject: "@rpc/dns".into(),
            state: AssertionState::Met,
            evidence: format!("{ORIGIN}: error/gated — conditional, and said so"),
            citation: Some("RFC 08 §6.1".into()),
            exempt: Some("when: config:collect.dns".into()),
        },
        Assertion {
            id: "qos-observed-mismatch/health".into(),
            subject: "state/health".into(),
            state: AssertionState::NotMet,
            evidence: format!(
                "v1/{ORIGIN}/state/sysinfo/health: 4 of 4 sample(s) did not ride the \
                 declared transition"
            ),
            citation: Some("RFC 04 §3".into()),
            exempt: None,
        },
        Assertion {
            id: "observed/disk/{mount}/used".into(),
            subject: "telemetry/disk/{mount}/used".into(),
            state: AssertionState::Unknowable {
                reason: "a window proves presence, never absence".into(),
            },
            evidence: "not seen in 10s".into(),
            citation: Some("RFC 13 §3".into()),
            exempt: None,
        },
    ];
    let summary = ConformSummary::of(&assertions);
    ConformReport {
        producer: "sysinfo".into(),
        slice_source: ConformSource::Dirs,
        origins_asked: vec![ORIGIN.into()],
        verdict: ConformVerdict::of(&summary),
        summary,
        assertions,
        observation: Some(ObservationSummary {
            window_s: 10.0,
            scopes: vec!["v1/*/state/**".into()],
            samples: 40,
            keys_seen: 2,
            dropped: 3,
            synthetic_marked: 40,
            facts_evicted: 0,
        }),
        deep: false,
        not_asked: vec!["stale-state/*, budget: not asked without --deep".into()],
    }
}

fn header() -> zenkey_fleet::ZrecHeader {
    zenkey_fleet::ZrecHeader {
        zrec: 1,
        selectors: vec!["acme/v1/**".into()],
        base: "acme".into(),
        captured_at: "2026-08-21T00:00:00Z".into(),
        excluded: None,
        preamble: None,
        pre_roll: None,
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
        trigger: None,
        preamble: None,
        pre_roll: None,
        preamble_rows: 0,
    }
}

/// A dry replay of that capture — a partial view of a partial view.
pub fn replay_report() -> ReplayReport {
    ReplayReport {
        header: header(),
        dry_run: true,
        speed: 1.0,
        namespace: None,
        published: 4_800,
        tombstones: 20,
        malformed: 2,
        refused: 1,
        capture_dropped: 31,
        first_errors: vec![
            "line 41: unknown QoS profile \"data/drop/reliable\"".into(),
            "line 88: refused delete on a telemetry key".into(),
        ],
        preamble_skipped: 0,
        preamble_seeded: 0,
        triggers: 0,
    }
}

/// A rate report with the key table bounded and hit — the O6 count that a
/// trailing envelope used to lose to `| head` — grouped by zk2 address and
/// resource, a foreign key in a group of its own (#612, FJ8b).
pub fn rate_report() -> RateReport {
    let stream = KeyGroup::Resource {
        address: "host-a/tc".into(),
        iface: "tc.netif.v1".into(),
        token: "stream".into(),
        resource: Some("stream/bandwidth/{ns}/{iface}".into()),
    };
    RateReport {
        selector: "zk2/**".into(),
        window_s: 10.0,
        lens: LensScope {
            namespace: String::new(),
            presence: Some(LensPresence {
                selector: "zk2/*/*/@zk/**".into(),
                complete: true,
                services: 1,
            }),
            contracts: 1,
        },
        groups: vec![
            RateGroup {
                group: stream.clone(),
                keys: 2,
                count: 100,
                bytes: 4_050,
            },
            RateGroup {
                group: KeyGroup::NotZk2,
                keys: 1,
                count: 20,
                bytes: 80,
            },
        ],
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
                key: "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0".into(),
                identity: KeyIdentity {
                    group: stream.clone(),
                    values: [
                        ("iface".to_owned(), vec!["eth0".to_owned()]),
                        ("ns".to_owned(), vec!["default".to_owned()]),
                    ]
                    .into(),
                    unresolved: None,
                },
                count: 50,
                bytes: 2_250,
                sn_gaps: Asked::Asked(0),
                latency: None,
                unstamped: Asked::Asked(50),
                clocks: Asked::Asked(Default::default()),
            },
            RateRow {
                key: "rt/chatter".into(),
                identity: KeyIdentity {
                    group: KeyGroup::NotZk2,
                    values: Default::default(),
                    unresolved: Some(Unresolved::NotZk2 {
                        detail: "it does not start with zk2/".into(),
                    }),
                },
                count: 20,
                bytes: 80,
                sn_gaps: Asked::Asked(0),
                latency: None,
                unstamped: Asked::Asked(0),
                clocks: Asked::Asked(Default::default()),
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

// ── acl gen (#612, FJ7) ───────────────────────────────────────────────────

/// The interface the ACL fixtures plan over: one state template, one
/// operation, typed so the contract needs no schema file.
const ACL_TC: &str = r#"[interface]
name = "tc.netif"
major = 1
minor = 0

[resources."interfaces/{iface}"]
kind = "state"
type = { raw = "application/json" }
params = { iface = "string" }
cardinality = 64

[resources."interfaces/{iface}/set"]
kind = "operation"
request = "google.protobuf.Empty"
response = "google.protobuf.Empty"
params = { iface = "string" }
cardinality = 64
"#;

/// Two tc backends and a frontend bound to `*/tc` that calls them, planned
/// under deny: Own, the fan-in egress the frontend's wildcard selectors
/// need, Consume, Call, presence, and the open contract bundles. Built
/// through the planner itself rather than by hand: what the corpora pin is
/// what the verb draws.
pub fn acl_plan() -> AclPlan {
    let contract = zenkey_model::contract::load_str(ACL_TC, std::path::Path::new("."), None)
        .contract
        .expect("the fixture contract loads");
    let mut contracts = zenkey_fleet::ContractSet::new();
    contracts.insert(zenkey_fleet::Revision::from_contract(
        contract,
        ContractSource::File,
    ));
    let backend = |host: &str| ServiceSpec {
        address: format!("{host}/tc"),
        implements: vec!["tc.netif.v1".into()],
        ..Default::default()
    };
    let enrollment = Enrollment {
        namespace: None,
        principal: vec![
            PrincipalSpec {
                user: Some("tc-h1".into()),
                services: vec!["h1/tc".into()],
                ..Default::default()
            },
            PrincipalSpec {
                user: Some("tc-h2".into()),
                services: vec!["h2/tc".into()],
                ..Default::default()
            },
            PrincipalSpec {
                cn: Some("frontend.ops".into()),
                id: Some("frontend".into()),
                services: vec!["ops/frontend".into()],
                ..Default::default()
            },
        ],
        service: vec![
            backend("h1"),
            backend("h2"),
            ServiceSpec {
                address: "ops/frontend".into(),
                bindings: [(
                    "netif".to_string(),
                    BindingSpec {
                        interface: Some("tc.netif.v1".into()),
                        providers: vec!["*/tc".into()],
                        ..Default::default()
                    },
                )]
                .into(),
                calls: vec![CallsSpec {
                    interface: "tc.netif.v1".into(),
                    providers: vec!["*/tc".into()],
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    zenkey_fleet::plan_acl(
        &enrollment,
        &contracts,
        &zenkey_fleet::AclOptions::default(),
    )
    .expect("the fixture enrollment plans")
}

/// The same plan with one principal refused: a zid, which nothing
/// authenticates.
pub fn acl_plan_refused() -> AclPlan {
    let mut plan = acl_plan();
    plan.refusals.push(AclRefusal {
        principal: "bench-rig".into(),
        reason: "zid \"a1b2c3\": a zid is not backed by authentication, so it binds nothing".into(),
        cite: "§11.3: `zids` subjects are unauthenticated".into(),
    });
    plan
}

/// A check with two findings: a backend's fan-in egress grant missing (a
/// fan-in GET then gets 0 replies, §11.2) and a user the enrollment never
/// enrolled.
pub fn acl_check() -> AclCheck {
    AclCheck {
        namespace: String::new(),
        against: "router.json5".into(),
        planned_rules: 18,
        observed_rules: 17,
        planned_subjects: 3,
        observed_subjects: 4,
        findings: vec![
            AclFinding {
                kind: AclFindingKind::RuleMissing,
                id: "fan-in:h1/tc".into(),
                planned: Some(
                    "allow egress declare_subscriber,query zk2/*/tc/tc.netif.v1/state/**".into(),
                ),
                observed: None,
            },
            AclFinding {
                kind: AclFindingKind::UnknownIdentity,
                id: "user stranger".into(),
                planned: None,
                observed: Some("bound by subject \"stranger\"".into()),
            },
        ],
        judgement: Judgement::Established,
    }
}

/// A clean check.
pub fn acl_check_clean() -> AclCheck {
    AclCheck {
        findings: vec![],
        observed_rules: 18,
        observed_subjects: 3,
        judgement: Judgement::NotEstablished {
            reason: "router.json5 carries the plan whole: 18 rule(s), 3 subject(s), 3 polic(y/ies), enabled, default deny".into(),
        },
        ..acl_check()
    }
}

/// The frontend's fan-in GET over every backend's interfaces: allowed on
/// ingress by its Consume grant, and denied on egress, where no rule of its
/// own includes it (egress toward a provider is the provider's subject's).
pub fn acl_explain() -> AclExplain {
    zenkey_fleet::explain_acl(
        &acl_plan(),
        "frontend",
        "zk2/*/tc/tc.netif.v1/state/**",
        AclMessage::Query,
    )
    .expect("an enrolled principal and a valid key")
}

// ── The fleet timeline (#216) ─────────────────────────────────────────────

fn timeline_lane() -> LaneId {
    LaneId::Resource {
        address: "host-a/tc".into(),
        iface: "tc.netif.v1".into(),
        token: "stream".into(),
        resource: Some("stream/bandwidth/{ns}/{iface}".into()),
    }
}

fn timeline_lens() -> LensScope {
    LensScope {
        namespace: "acme".into(),
        presence: Some(LensPresence {
            selector: "zk2/*/*/@zk/**".into(),
            complete: true,
            services: 1,
        }),
        contracts: 1,
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
        provenance: hlc.map(|_| Provenance::Owner),
        kind: RowKind::Put,
    }
}

const TL_A: &str = "acme/zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0";
const TL_B: &str = "acme/zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth1";
const TL_PLAIN: &str = "plain/key";

/// A ten-second window on the arrival axis: two samples of one resource,
/// stamped by its owner's clock (its descriptor's `meta.zid`), that arrived
/// in the *opposite* order to their HLCs, an unstamped sample in its own
/// lane, and a drop of 3 between the second and third — the reorder is
/// visible against [`timeline_report_hlc`].
pub fn timeline_report_arrival() -> TimelineReport {
    let lane = timeline_lane();
    TimelineReport {
        order_by: OrderLabel::Arrival,
        axis: AxisLabel::Arrival {
            clock: ARRIVAL_CLOCK,
        },
        scopes: vec!["acme/zk2/**".into()],
        window_s: Some(10.0),
        source: TimelineSource::Live,
        lens: timeline_lens(),
        lanes: vec![
            LaneSummary {
                lane: lane.clone(),
                samples: 2,
                first_t_us: 1_000,
                last_t_us: 2_000,
                stampers: ["33".to_string()].into_iter().collect(),
                provenance: ProvenanceCounts {
                    owner: 2,
                    other: 0,
                    unattributable: 0,
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
        scopes: vec!["acme/zk2/**".into()],
        window_s: Some(10.0),
        source: TimelineSource::Live,
        lens: timeline_lens(),
        lanes: vec![LaneSummary {
            lane: lane.clone(),
            samples: 2,
            first_t_us: 1_000,
            last_t_us: 2_000,
            stampers: ["33".to_string()].into_iter().collect(),
            provenance: ProvenanceCounts {
                owner: 2,
                other: 0,
                unattributable: 0,
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

// ─── snapshots (RFC 13 §4.4, #219; zk2's since #612, FJ8b) ────────────────

fn zsnap_header(
    base: &str,
    collected_at: &str,
    span: f64,
    answered: u64,
    superseded: u64,
) -> ZsnapHeader {
    ZsnapHeader {
        zsnap: 2,
        selectors: vec![format!("{base}/zk2/*/*/*/state/**")],
        base: base.into(),
        collected_at: collected_at.into(),
        collection_span_s: span,
        asked: 1,
        answered,
        elided: 0,
        errors: 0,
        discarded: 0,
        superseded,
        presence: Some(LensPresence {
            selector: "zk2/*/*/@zk/**".into(),
            complete: true,
            services: 2,
        }),
    }
}

/// A resolved state key's identity.
fn state_identity(address: &str, resource: &str, values: &[(&str, &str)]) -> KeyIdentity {
    KeyIdentity {
        group: KeyGroup::Resource {
            address: address.into(),
            iface: "tc.netif.v1".into(),
            token: "state".into(),
            resource: Some(resource.into()),
        },
        values: values
            .iter()
            .map(|(k, v)| ((*k).to_owned(), vec![(*v).to_owned()]))
            .collect(),
        unresolved: None,
    }
}

fn snapshot_row(key: &str, identity: KeyIdentity, bytes: &str, holder: Holder) -> SnapshotRow {
    SnapshotRow {
        key: key.into(),
        identity,
        delete: false,
        bytes: Some(bytes.into()),
        encoding: Some("application/json".into()),
        timestamp: Some("7f3b2a1c00000001/ab12".into()),
        stamper: Some(StamperWire::Owner { id: "ab12".into() }),
        source_zid: Some("ab12".into()),
        conformance: Conformance::Valid,
        holder,
    }
}

/// Two services under `acme`, four rows: an owner answering its own
/// namespaces, an interface whose payload fails its type, a deletion within
/// the owner's window, and an address no instance holds — answered anyway,
/// which S4 forbids. Every holder and stamper kind is there.
pub fn snapshot() -> Snapshot {
    let live = |by| Holder::Live {
        address: "host-a/tc".into(),
        answered_by: by,
    };
    Snapshot {
        header: zsnap_header("acme", "2026-10-09T00:00:00Z", 1.25, 5, 1),
        rows: vec![
            snapshot_row(
                "acme/zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0",
                state_identity(
                    "host-a/tc",
                    "state/interfaces/{ns}/{iface}",
                    &[("ns", "default"), ("iface", "eth0")],
                ),
                "eyJuYW1lIjoiZXRoMCIsImlzX3VwIjoieWVzIn0=",
                live(AnsweredBy::Owner),
            )
            .nonconforming(),
            SnapshotRow {
                delete: true,
                bytes: None,
                encoding: None,
                conformance: Conformance::NotChecked {
                    reason: "a deletion carries no value".into(),
                },
                ..snapshot_row(
                    "acme/zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth9",
                    state_identity(
                        "host-a/tc",
                        "state/interfaces/{ns}/{iface}",
                        &[("ns", "default"), ("iface", "eth9")],
                    ),
                    "",
                    live(AnsweredBy::Owner),
                )
            },
            snapshot_row(
                "acme/zk2/host-a/tc/tc.netif.v1/state/namespaces",
                state_identity("host-a/tc", "state/namespaces", &[]),
                "WyJkZWZhdWx0Il0=",
                live(AnsweredBy::Owner),
            ),
            SnapshotRow {
                stamper: Some(StamperWire::Other { id: "cd34".into() }),
                source_zid: Some("cd34".into()),
                ..snapshot_row(
                    "acme/zk2/host-b/tc/tc.netif.v1/state/namespaces",
                    state_identity("host-b/tc", "state/namespaces", &[]),
                    "WyJkZWZhdWx0Il0=",
                    Holder::NoInstance {
                        address: "host-b/tc".into(),
                    },
                )
            },
        ],
    }
}

/// A row's payload failing its type, applied after the shared constructor.
trait Nonconforming {
    fn nonconforming(self) -> SnapshotRow;
}

impl Nonconforming for SnapshotRow {
    fn nonconforming(mut self) -> SnapshotRow {
        self.conformance = Conformance::Invalid {
            violations: vec!["/is_up: expected boolean".into()],
        };
        self
    }
}

/// The same deployment five minutes on: `eth0` was fixed and conforms, the
/// deletion aged out of the owner's window, `host-b/tc` came back, and a new
/// interface appeared.
pub fn snapshot_b() -> Snapshot {
    let mut b = snapshot();
    b.header = zsnap_header("acme", "2026-10-09T00:05:00Z", 0.8, 4, 0);
    b.rows.retain(|r| !r.delete);
    for row in &mut b.rows {
        if row.key.ends_with("/default/eth0") {
            row.bytes = Some("eyJuYW1lIjoiZXRoMCIsImlzX3VwIjp0cnVlfQ==".into());
            row.timestamp = Some("7f3b2a1c00000002/ab12".into());
            row.conformance = Conformance::Valid;
        }
        if row.key.contains("/host-b/") {
            row.stamper = Some(StamperWire::Owner { id: "cd34".into() });
            row.holder = Holder::Live {
                address: "host-b/tc".into(),
                answered_by: AnsweredBy::Owner,
            };
        }
    }
    b.rows.push(snapshot_row(
        "acme/zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth1",
        state_identity(
            "host-a/tc",
            "state/interfaces/{ns}/{iface}",
            &[("ns", "default"), ("iface", "eth1")],
        ),
        "eyJuYW1lIjoiZXRoMSIsImlzX3VwIjpmYWxzZX0=",
        Holder::Live {
            address: "host-a/tc".into(),
            answered_by: AnsweredBy::Owner,
        },
    ));
    b.rows.sort_by(|x, y| x.key.cmp(&y.key));
    b
}

/// What taking [`snapshot`] reported: written to a file, every holder
/// counted, and the one payload that failed its type.
pub fn snapshot_report() -> SnapshotReport {
    SnapshotReport {
        header: snapshot().header,
        out: Some("deployment.zsnap".into()),
        live: 3,
        no_instance: 1,
        unattributed: 0,
        nonconforming: 1,
        incomplete: Vec::new(),
    }
}

/// [`snapshot`] against [`snapshot_b`], through the engine's own comparison
/// at its default bounds.
pub fn snapshot_diff() -> SnapshotDiff {
    zenkey_fleet::diff_snapshots(
        &snapshot(),
        &snapshot_b(),
        zenkey_fleet::DiffOpts::default(),
    )
}

/// [`snapshot`] against itself moved to another namespace: the zk2 keys
/// line up across the two, and the two deployments' clocks are not compared
/// — nothing differs.
pub fn snapshot_diff_namespaces() -> SnapshotDiff {
    let a = snapshot();
    let mut b = snapshot();
    b.header = zsnap_header("staging", "2026-10-09T01:00:00Z", 0.9, 5, 1);
    for row in &mut b.rows {
        row.key = row.key.replacen("acme/", "staging/", 1);
        row.timestamp = row.timestamp.as_ref().map(|t| t.replace("/ab12", "/ef56"));
    }
    zenkey_fleet::diff_snapshots(&a, &b, zenkey_fleet::DiffOpts::default())
}

/// [`snapshot`] against itself: identical on every facet.
pub fn snapshot_diff_identity() -> SnapshotDiff {
    zenkey_fleet::diff_snapshots(&snapshot(), &snapshot(), zenkey_fleet::DiffOpts::default())
}

/// One exporter fold with every honesty pole exercised at once: a live
/// series, a `{var}` series whose origin went down (no value, labels kept), a
/// field series the observer evicted, a `state` series past its declared
/// ttl; every O6 population non-zero; every payload population non-zero; two
/// suppression reasons; a doctor that ran and found something.
pub fn export_snapshot() -> ExportSnapshot {
    use std::collections::BTreeMap;
    ExportSnapshot {
        scopes: vec!["acme/v1/*/**".into()],
        excluded: zenkey_fleet::WILDCARD_EXCLUDES
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
        registry: Asked::Asked(RegistryInfo { producers: 2 }),
        max_series: 10_000,
        started_at_unix_s: 1_700_000_000,
        taken_at_unix_s: 1_700_000_120,
        series: vec![
            SeriesRow {
                name: "zenkey_subject_sysinfo_cpu_usage_percent".into(),
                key: format!("acme/v1/{ORIGIN}/telemetry/sysinfo/cpu/usage"),
                origin: ORIGIN.into(),
                producer: "sysinfo".into(),
                class: "telemetry".into(),
                subject: "cpu/usage".into(),
                labels: BTreeMap::new(),
                field: None,
                kind: Some("gauge".into()),
                unit: Some("percent".into()),
                value: Some(12.5),
                last_seen_unix_s: 1_700_000_119,
                state: SeriesState::Live,
                samples: 240,
                drop_exposed: 2,
            },
            SeriesRow {
                name: "zenkey_subject_sysinfo_disk_used_bytes".into(),
                key: "acme/v1/h-0000deadbeef/telemetry/sysinfo/disk/var-log/used".into(),
                origin: "h-0000deadbeef".into(),
                producer: "sysinfo".into(),
                class: "telemetry".into(),
                subject: "disk/{mount}/used".into(),
                labels: [("mount".to_string(), "var-log".to_string())]
                    .into_iter()
                    .collect(),
                field: None,
                kind: None,
                unit: Some("bytes".into()),
                value: None,
                last_seen_unix_s: 1_700_000_040,
                state: SeriesState::OriginDown,
                samples: 80,
                drop_exposed: 0,
            },
            SeriesRow {
                name: "zenkey_subject_netlink_iface_rx_bytes_total".into(),
                key: format!("acme/v1/{ORIGIN}/telemetry/netlink/iface/eth0/rx_bytes"),
                origin: ORIGIN.into(),
                producer: "netlink".into(),
                class: "telemetry".into(),
                subject: "iface/{iface}/rx_bytes".into(),
                labels: [("iface".to_string(), "eth0".to_string())]
                    .into_iter()
                    .collect(),
                field: Some("rx".into()),
                kind: Some("counter".into()),
                unit: Some("bytes".into()),
                value: None,
                last_seen_unix_s: 1_700_000_100,
                state: SeriesState::Evicted,
                samples: 5,
                drop_exposed: 0,
            },
            SeriesRow {
                name: "zenkey_subject_sysinfo_health".into(),
                key: format!("acme/v1/{ORIGIN}/state/sysinfo/health"),
                origin: ORIGIN.into(),
                producer: "sysinfo".into(),
                class: "state".into(),
                subject: "health".into(),
                labels: BTreeMap::new(),
                field: Some("uptime_s".into()),
                kind: None,
                unit: None,
                value: Some(4242.0),
                last_seen_unix_s: 1_700_000_060,
                state: SeriesState::Quiet,
                samples: 4,
                drop_exposed: 0,
            },
        ],
        observer: ObserverCounters {
            dropped: 3,
            evicted_keys: 5,
            evicted_bytes: 7,
            expired: 11,
            unwatched: 13,
            coalesced: 17,
            unstamped: 19,
        },
        contract: ContractCounters {
            qos_judged: 320,
            qos_mismatch: 2,
            qos_mismatch_by_subject: vec![QosMismatchRow {
                producer: "sysinfo".into(),
                subject: "cpu/usage".into(),
                n: 2,
            }],
            payload_valid: 200,
            payload_invalid: 1,
            payload_not_validated: 128,
        },
        suppressed: [("cardinality".to_string(), 4u64), ("fields".to_string(), 1)]
            .into_iter()
            .collect(),
        unregistered_keys: 3,
        doctor: Asked::Asked(DoctorSummary {
            ran_at_unix_s: 1_700_000_090,
            findings: vec![DoctorFindingRef {
                check: CheckId::SplitBrain,
                severity: DoctorSeverity::Error,
                subject: "host-a/tc tc.netif.v1".into(),
            }],
        }),
    }
}
