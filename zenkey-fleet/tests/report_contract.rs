//! The frontend contract, pinned (issue #202).
//!
//! `report.rs` is what `zenctl --format json` emits and what zengui's panes
//! render. Field renames and removals break users, so this makes changing one
//! a deliberate act rather than a silent one — the same job
//! `doctor_report_json_shape_is_pinned` has done for two types since #161, now
//! extended to the rest.
//!
//! **Whole-document `assert_eq!` against a `json!` literal, never
//! field-by-field.** Two reasons, and both are the point of the file:
//! comparing fields cannot catch an *added* one, and `json["absent"]` yields
//! `Value::Null`, so a field-by-field test passes identically for "absent" and
//! "explicitly null". That distinction is RFC 09 §5.1 **O4** — a question not
//! asked must not render as a negative answer — and in these documents it is
//! carried entirely by `skip_serializing_if`. A test that cannot see it is not
//! guarding it.
//!
//! The values come from `zenkey-report-fixtures`, shared with `zenctl`'s
//! render corpus: this file pins what a report *serializes to*, that one pins
//! how the same value is *drawn*, and sharing the constructors is what stops
//! the two drifting about what a report is. Where a shape needs an edge case
//! the shared value does not cover, the test builds that one inline — the
//! fixtures are a baseline, not a ceiling.
//!
//! Where a shape here looks inconsistent with its neighbours, it is pinned as
//! it *is* and the inconsistency is filed. That promise has now been kept
//! once: #232 named three, chunk AI changed them, and each change had to state
//! itself in this file to land. `Coverage`'s tags are snake_case below because
//! of it. The rule stands for the next one.

use serde_json::json;
use zenkey_fleet::report::*;
use zenkey_fleet::{Coverage, CoverageRow};
use zenkey_report_fixtures as fx;

/// Three states in one `Option<Vec<_>>`: absent = the roster was never asked
/// (O4), `[]` = asked and nobody answered (RFC 05 §3.1), non-empty = alive.
/// Collapsing the first two is the single easiest way to make this document
/// lie.
#[test]
fn blob_origins_distinguish_not_asked_from_nobody_answered() {
    let base = BlobTierRow {
        producer: "artifacts".into(),
        registry_version: "1.0".into(),
        tier: "artifact".into(),
        known_tier: true,
        endpoints: vec!["manifest".into(), "slice".into()],
        algo: None,
        reference: Some("BlobRef".into()),
        encoding: None,
        since: None,
        description: None,
        origins: Asked::NotAsked,
    };
    let not_asked = serde_json::to_value(&base).unwrap();
    assert_eq!(
        not_asked,
        json!({
            "producer": "artifacts",
            "registry_version": "1.0",
            "tier": "artifact",
            "known_tier": true,
            "endpoints": ["manifest", "slice"],
            "reference": "BlobRef",
        }),
        "roster not asked: the key is absent"
    );

    let asked_silent = BlobTierRow {
        origins: Asked::Asked(vec![]),
        ..base.clone()
    };
    assert_eq!(
        serde_json::to_value(&asked_silent).unwrap()["origins"],
        json!([]),
        "asked, nobody answered: an empty list, which is not absence"
    );

    let alive = BlobTierRow {
        origins: Asked::Asked(vec!["h-3fa9c2d41b7e".into()]),
        ..base
    };
    assert_eq!(
        serde_json::to_value(&alive).unwrap()["origins"],
        json!(["h-3fa9c2d41b7e"])
    );
}

/// The trickiest shape in the file: a `#[serde(flatten)]` over an adjacently
/// tagged enum.
///
/// Its tag values were **PascalCase** while every other enum in this surface
/// was snake or kebab — pinned here as it was, and filed as #232 rather than
/// quietly changed, because it is a wire contract. #232 is that change, and
/// this is where it states itself.
#[test]
fn coverage_flattens_into_its_row_with_snake_case_tags() {
    let covered = CoverageRow {
        producer: "sysinfo".into(),
        path: "health".into(),
        ttl_s: Some(30),
        coverage: Coverage::Covered("main@abc".into()),
    };
    assert_eq!(
        serde_json::to_value(&covered).unwrap(),
        json!({
            "producer": "sysinfo",
            "path": "health",
            "ttl_s": 30,
            "coverage": "covered",
            "storage": "main@abc",
        })
    );

    let uncovered = CoverageRow {
        producer: "sysinfo".into(),
        path: "health".into(),
        ttl_s: None,
        coverage: Coverage::Uncovered,
    };
    assert_eq!(
        serde_json::to_value(&uncovered).unwrap(),
        json!({"producer": "sysinfo", "path": "health", "coverage": "uncovered"}),
        "no storage key at all when nothing covers it, and no ttl when none \
         is declared"
    );

    assert_eq!(
        serde_json::to_value(Coverage::Partial("main@abc".into())).unwrap(),
        json!({"coverage": "partial", "storage": "main@abc"})
    );
}

/// Every optional is a wire fact that is absent rather than null when it did
/// not ride — and the outcome enum serializes to the exact shape the old
/// `{ok, error: Option}` struct pinned here, so scripts keep parsing.
#[test]
fn a_call_answer_omits_every_part_the_wire_did_not_carry() {
    let bare = CallAnswer {
        origin: "h-3fa9c2d41b7e".into(),
        outcome: CallOutcome::Ok {
            value: None,
            text: None,
        },
        attachment: None,
        attachment_bytes: None,
    };
    assert_eq!(
        serde_json::to_value(&bare).unwrap(),
        json!({"origin": "h-3fa9c2d41b7e", "ok": true})
    );

    let failed = CallAnswer {
        outcome: CallOutcome::Err(CallError {
            name: "unsupported".into(),
            message: "not built with that feature".into(),
        }),
        ..bare
    };
    assert_eq!(
        serde_json::to_value(&failed).unwrap(),
        json!({
            "origin": "h-3fa9c2d41b7e",
            "ok": false,
            "error": {"name": "unsupported", "message": "not built with that feature"},
        })
    );

    // The full shape, field order included: origin, ok, the outcome's own
    // fields, the attachment pair, error last — byte-identical to what the
    // derived struct serialized before the enum (C7).
    let rich = CallAnswer {
        origin: "h-3fa9c2d41b7e".into(),
        outcome: CallOutcome::Ok {
            value: Some(json!({"count": 214})),
            text: None,
        },
        attachment: Some(json!({"trace": "abc123"})),
        attachment_bytes: Some(18),
    };
    assert_eq!(
        serde_json::to_string(&rich).unwrap(),
        r#"{"origin":"h-3fa9c2d41b7e","ok":true,"value":{"count":214},"attachment":{"trace":"abc123"},"attachment_bytes":18}"#
    );

    // Silence is exit 2 and an empty answer list — never an error reply.
    // R5: `timeout_s` is new in the report-honesty batch — the silence note
    // named a timeout the document never stated. Additive; old consumers
    // keep parsing.
    let silent = CallReport {
        key: "v1/*/@rpc/sysinfo/introspect".into(),
        timeout_s: 5.0,
        answers: vec![],
    };
    assert_eq!(silent.exit_code(), 2);
    assert_eq!(
        serde_json::to_value(&silent).unwrap(),
        json!({
            "key": "v1/*/@rpc/sysinfo/introspect",
            // `5.0`, not `5`: since #218 every window/timeout in this surface
            // is `f64` seconds, so an integral one renders with its point.
            "timeout_s": 5.0,
            "answers": [],
        })
    );
    assert_eq!(
        CallReport {
            answers: vec![failed],
            ..silent
        }
        .exit_code(),
        1
    );
}

/// #213's three populations, and the two counters beside them. `stampers_dropped`
/// skips on zero, so the common case carries no bound-accounting noise while a
/// tripped bound still says so (O6).
#[test]
fn a_rate_row_keeps_its_latency_populations_apart() {
    // R3 (#238's twin): `sn_gaps`/`unstamped` are Options since the
    // report-honesty batch — present iff `--loss`/`--latency` asked, exactly
    // like the report-level `sn_gaps` and the row's own `latency`. The pin
    // change is the visible act: a row from an unasked run used to serialize
    // an uncaveated `"sn_gaps": 0`.
    let identity = KeyIdentity {
        group: KeyGroup::NotZk2,
        values: Default::default(),
        unresolved: Some(Unresolved::NotZk2 { detail: "x".into() }),
    };
    let quiet = RateRow {
        key: "v1/h-a/telemetry/p/m".into(),
        identity: identity.clone(),
        count: 12,
        bytes: 480,
        sn_gaps: Asked::Asked(0),
        latency: None,
        unstamped: Asked::Asked(12),
        clocks: Asked::Asked(Default::default()),
    };
    assert_eq!(
        serde_json::to_value(&quiet).unwrap(),
        json!({
            "key": "v1/h-a/telemetry/p/m",
            "identity": {"is": "not_zk2", "unresolved": {"reason": "not_zk2", "detail": "x"}},
            "count": 12,
            "bytes": 480,
            "sn_gaps": 0,
            "unstamped": 12,
            "clocks": {},
        }),
        "nothing stamped: the latency key is absent, which is not zero latency"
    );

    let unasked = RateRow {
        sn_gaps: Asked::NotAsked,
        unstamped: Asked::NotAsked,
        clocks: Asked::NotAsked,
        ..quiet.clone()
    };
    assert_eq!(
        serde_json::to_value(&unasked).unwrap(),
        json!({
            "key": "v1/h-a/telemetry/p/m",
            "identity": {"is": "not_zk2", "unresolved": {"reason": "not_zk2", "detail": "x"}},
            "count": 12,
            "bytes": 480,
        }),
        "no --loss and no --latency: the counters and the clocks are absent (O4), never zero"
    );

    let dist = zenkey_fleet::LatencySummary {
        min_us: -200,
        median_us: 900,
        p95_us: 1500,
        max_us: 5000,
        samples: 4,
    };
    let stamped = RateRow {
        latency: Some(zenkey_fleet::LatencyReport {
            self_stamped: Some(dist),
            foreign: None,
            unattributable: None,
            stampers: vec![],
            stampers_dropped: 0,
        }),
        unstamped: Asked::Asked(1),
        ..quiet
    };
    assert_eq!(
        serde_json::to_value(&stamped).unwrap()["latency"],
        json!({
            "self_stamped": {
                "min_us": -200, "median_us": 900, "p95_us": 1500,
                "max_us": 5000, "samples": 4,
            },
        }),
        "one population, and the empty stamper bookkeeping stays out of the way"
    );
}

/// The three-state verdict. Silence has its own word, and that is the whole
/// reason it is not a boolean.
#[test]
fn the_three_state_verdicts_keep_their_third_state() {
    for (v, wire) in [
        (ExpectVerdict::Met, "met"),
        (ExpectVerdict::NotMet, "not_met"),
        (ExpectVerdict::Impaired, "impaired"),
    ] {
        assert_eq!(serde_json::to_value(v).unwrap(), json!(wire));
    }

    let clean = ExpectReport {
        address: "host-a/tc".into(),
        iface: "tc.netif.v1".into(),
        fingerprint: "sha256:ab".into(),
        resource: Some("stream/bandwidth/{ns}/{iface}".into()),
        selectors: vec!["zk2/host-a/tc/tc.netif.v1/stream/bandwidth/*/*".into()],
        window_s: 5.0,
        ended_early: false,
        samples: 0,
        keys_seen: 0,
        dropped: 0,
        discarded: 0,
        unresolved: 0,
        rate_hz: None,
        presence: None,
        violations: vec![],
        violations_total: 0,
        unmet: vec!["0 sample(s) observed, 1 required".into()],
        verdict: ExpectVerdict::NotMet,
    };
    assert_eq!(
        serde_json::to_value(&clean).unwrap(),
        json!({
            "address": "host-a/tc",
            "iface": "tc.netif.v1",
            "fingerprint": "sha256:ab",
            "resource": "stream/bandwidth/{ns}/{iface}",
            "selectors": ["zk2/host-a/tc/tc.netif.v1/stream/bandwidth/*/*"],
            "window_s": 5.0,
            "ended_early": false,
            "samples": 0,
            "keys_seen": 0,
            "dropped": 0,
            "discarded": 0,
            "unresolved": 0,
            "violations_total": 0,
            "unmet": ["0 sample(s) observed, 1 required"],
            "verdict": "not_met",
        }),
        "an empty violation list is absent, `rate_hz: None` means not asked, and \
         presence not asked is absent"
    );
}

/// The `@blob` documents must serialize identically whether or not the binary
/// was built with the transport — `report.rs`'s own header says so, and this
/// file is compiled without the `blob` feature, which is the proof.
#[test]
fn a_blob_probe_reports_what_it_asked_and_what_answered() {
    // R7: `slices_considered` is new in the report-honesty batch —
    // BlobList's own solution, so an empty `declared_by` no longer conflates
    // "no slice declares this tier" with "no registry was loaded" (O4).
    // Additive and unconditional, like BlobList's.
    let empty = BlobProbeReport {
        target: "01hq9k".into(),
        tier: "artifact".into(),
        asked: vec!["v1/*/@blob/artifact/01hq9k/manifest".into()],
        not_probed: None,
        holders: vec![],
        answered: 0,
        roots: vec![],
        declared_by: vec!["artifacts".into()],
        slices_considered: 3,
    };
    assert_eq!(
        serde_json::to_value(&empty).unwrap(),
        json!({
            "target": "01hq9k",
            "tier": "artifact",
            "asked": ["v1/*/@blob/artifact/01hq9k/manifest"],
            "holders": [],
            "answered": 0,
            "roots": [],
            "declared_by": ["artifacts"],
            "slices_considered": 3,
        }),
        "asked but unanswered: the selector is on record, so silence is \
         visibly a non-verdict rather than an absent question"
    );
    let no_registry = BlobProbeReport {
        declared_by: vec![],
        slices_considered: 0,
        ..empty
    };
    let v = serde_json::to_value(&no_registry).unwrap();
    assert!(
        v.get("declared_by").is_none(),
        "an empty capability list stays absent"
    );
    assert_eq!(
        v["slices_considered"], 0,
        "…and the zero slice count is what says it was never a verdict (R7)"
    );

    let holder = BlobHolder {
        origin: "h-3fa9c2d41b7e".into(),
        key: "v1/h-3fa9c2d41b7e/@blob/artifact/01hq9k/manifest".into(),
        availability: None,
        manifest: None,
        note: Some("answered, said nothing readable".into()),
        unreadable: None,
        error: None,
    };
    assert_eq!(
        serde_json::to_value(&holder).unwrap(),
        json!({
            "origin": "h-3fa9c2d41b7e",
            "key": "v1/h-3fa9c2d41b7e/@blob/artifact/01hq9k/manifest",
            "note": "answered, said nothing readable",
        })
    );

    assert_eq!(
        serde_json::to_value(BlobListSource::RegistryDirs).unwrap(),
        json!("registry-dirs"),
        "kebab here, snake elsewhere — pinned as it is (see #202's note)"
    );
}

/// zk2's doctor (#612, FJ6): `doctor_report_json_shape_is_pinned` covers
/// the document; this covers the severity vocabulary its findings branch
/// on, one finding's shape, and the shared fixture's every verdict pole —
/// each spelled apart, so not asked never reads as clean or unobservable.
#[test]
fn doctor_severities_findings_and_verdicts_are_the_stable_vocabulary() {
    for (s, wire) in [
        (DoctorSeverity::Error, "error"),
        (DoctorSeverity::Warning, "warning"),
        (DoctorSeverity::Info, "info"),
    ] {
        assert_eq!(serde_json::to_value(s).unwrap(), json!(wire));
    }
    let finding = DoctorFinding {
        severity: DoctorSeverity::Info,
        check: CheckId::ShmMemlockLow,
        subject: "this host".into(),
        evidence: "RLIMIT_MEMLOCK is 64 KiB, below the 8 MiB floor".into(),
    };
    assert_eq!(
        serde_json::to_value(&finding).unwrap(),
        json!({
            "severity": "info",
            "check": "shm-memlock-low",
            "subject": "this host",
            "evidence": "RLIMIT_MEMLOCK is 64 KiB, below the 8 MiB floor",
        })
    );
    let report = serde_json::to_value(fx::doctor_report()).unwrap();
    let answer = |id: &str| {
        report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["check"] == id)
            .unwrap_or_else(|| panic!("{id} is reported"))["verdict"]["answer"]
            .clone()
    };
    assert_eq!(answer("split-brain"), "established");
    assert_eq!(answer("descriptor-invalid"), "not_established");
    assert_eq!(answer("contract-drift"), "unobservable");
    assert_eq!(answer("state-stamp-foreign"), "not_asked");
    assert_eq!(
        report["checks"].as_array().unwrap().len(),
        CheckId::ALL.len(),
        "every check id has its verdict"
    );
}

/// Every enum in this surface, its wire spelling, behind an exhaustive
/// `match`.
///
/// A naming *lint* cannot exist — serde attributes are not reflectable — so
/// this is the mechanism that works instead, and it is stronger: the `match`
/// arms below are exhaustive, so **adding a variant fails to compile** until
/// its wire string is written down here. A list of values would not do that;
/// the `match` is the whole guard, and the values merely exercise it.
///
/// Two vocabularies, deliberately, and documented rather than accidental:
/// **snake_case** for the verdict and severity vocabularies, **kebab-case**
/// for the blob plane's event and source tags, which shipped as a coherent
/// pair. A third would be drift; a documented second is not.
#[test]
fn every_enum_in_the_surface_names_its_wire_vocabulary() {
    fn topic(v: &TopicVerdict) -> &'static str {
        match v {
            TopicVerdict::Registered => "registered",
            TopicVerdict::Unregistered => "unregistered",
            TopicVerdict::NoSliceForProducer => "no_slice_for_producer",
            TopicVerdict::NotADataClass => "not_a_data_class",
            TopicVerdict::NotV1 => "not_v1",
            TopicVerdict::NotUnderBase => "not_under_base",
            TopicVerdict::RegistryNotLoaded => "registry_not_loaded",
        }
    }
    fn severity(v: &DoctorSeverity) -> &'static str {
        match v {
            DoctorSeverity::Error => "error",
            DoctorSeverity::Warning => "warning",
            DoctorSeverity::Info => "info",
        }
    }
    fn expect(v: &ExpectVerdict) -> &'static str {
        match v {
            ExpectVerdict::Met => "met",
            ExpectVerdict::NotMet => "not_met",
            ExpectVerdict::Impaired => "impaired",
        }
    }
    // #232: this one was PascalCase, alone in the file.
    fn coverage(v: &Coverage) -> &'static str {
        match v {
            Coverage::Covered(_) => "covered",
            Coverage::Partial(_) => "partial",
            Coverage::Uncovered => "uncovered",
        }
    }
    // The blob plane's pair: kebab, and staying kebab.
    fn blob_source(v: &BlobListSource) -> &'static str {
        match v {
            BlobListSource::Bus => "bus",
            BlobListSource::RegistryDirs => "registry-dirs",
            BlobListSource::Union => "union",
        }
    }
    fn blob_progress(v: &BlobProgress) -> &'static str {
        match v {
            BlobProgress::Started { .. } => "started",
            BlobProgress::Resumed { .. } => "resumed",
            BlobProgress::Chunk { .. } => "chunk",
            BlobProgress::Verifying => "verifying",
            BlobProgress::Completed { .. } => "completed",
            BlobProgress::Cancelled { .. } => "cancelled",
            BlobProgress::Failed { .. } => "failed",
        }
    }

    /// The tag as it actually serializes — a bare string for an externally
    /// tagged vocabulary, the tag field for an adjacently or internally
    /// tagged one.
    fn wire<T: serde::Serialize>(v: &T, tag: &str) -> String {
        match serde_json::to_value(v).unwrap() {
            serde_json::Value::String(s) => s,
            serde_json::Value::Object(o) => o
                .get(tag)
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| panic!("no `{tag}` tag on {o:?}"))
                .to_string(),
            other => panic!("not a vocabulary: {other}"),
        }
    }

    for v in [
        TopicVerdict::Registered,
        TopicVerdict::Unregistered,
        TopicVerdict::NoSliceForProducer,
        TopicVerdict::NotADataClass,
        TopicVerdict::NotV1,
        TopicVerdict::NotUnderBase,
        TopicVerdict::RegistryNotLoaded,
    ] {
        assert_eq!(wire(&v, ""), topic(&v));
    }
    for v in [
        DoctorSeverity::Error,
        DoctorSeverity::Warning,
        DoctorSeverity::Info,
    ] {
        assert_eq!(wire(&v, ""), severity(&v));
    }
    for v in [
        ExpectVerdict::Met,
        ExpectVerdict::NotMet,
        ExpectVerdict::Impaired,
    ] {
        assert_eq!(wire(&v, ""), expect(&v));
    }
    for v in [
        Coverage::Covered("s".into()),
        Coverage::Partial("s".into()),
        Coverage::Uncovered,
    ] {
        assert_eq!(wire(&v, "coverage"), coverage(&v));
    }
    for v in [
        BlobListSource::Bus,
        BlobListSource::RegistryDirs,
        BlobListSource::Union,
    ] {
        assert_eq!(wire(&v, ""), blob_source(&v));
    }
    for v in [
        BlobProgress::Started {
            total_len: 1,
            chunk_count: 1,
        },
        BlobProgress::Resumed {
            received: 1,
            total: 2,
        },
        BlobProgress::Chunk {
            index: 0,
            received: 1,
            total: 2,
            bytes_received: 3,
        },
        BlobProgress::Verifying,
        BlobProgress::Completed { path: "p".into() },
        BlobProgress::Cancelled {
            received: 1,
            total: 2,
        },
        BlobProgress::Failed { error: "e".into() },
    ] {
        assert_eq!(wire(&v, "event"), blob_progress(&v));
    }
}

// ── The consumers join (#224) ─────────────────────────────────────────────

// ── The fleet timeline (#216; zk2's lanes and clocks since #612, FJ8b) ────

/// The arrival axis: every row carries `order_by`, the break sits at its
/// position with no lane, a lane is one zk2 resource of one address, the
/// unstamped lane exists, each stamp names whose clock it is (the owner's,
/// here), the lens the lanes were resolved with rides the envelope, and the
/// sequence-number lane is *unavailable* with its fixed reason — never an
/// empty list. `unstamped_excluded` is absent at zero.
#[test]
fn a_timeline_on_the_arrival_axis_is_pinned() {
    let lane = json!({
        "kind": "resource",
        "address": "host-a/tc",
        "iface": "tc.netif.v1",
        "token": "stream",
        "resource": "stream/bandwidth/{ns}/{iface}",
    });
    assert_eq!(
        serde_json::to_value(fx::timeline_report_arrival()).unwrap(),
        json!({
            "order_by": "arrival",
            "axis": "arrival",
            "clock": "observer monotonic, µs since window start",
            "scopes": ["acme/zk2/**"],
            "window_s": 10.0,
            "source": {"kind": "live"},
            "lens": {
                "namespace": "acme",
                "presence": {"selector": "zk2/*/*/@zk/**", "complete": true, "services": 1},
                "contracts": 1,
            },
            "lanes": [
                {
                    "lane": lane,
                    "samples": 2,
                    "first_t_us": 1000,
                    "last_t_us": 2000,
                    "stampers": ["33"],
                    "provenance": {"owner": 2, "other": 0, "unattributable": 0}
                },
                {
                    "lane": {"kind": "unstamped"},
                    "samples": 1,
                    "first_t_us": 3000,
                    "last_t_us": 3000,
                    "stampers": [],
                    "provenance": {"owner": 0, "other": 0, "unattributable": 0}
                }
            ],
            "sn_lane": {
                "state": "unavailable",
                "reason": "zenoh 1.9/1.10 deliver no SourceInfo to subscribers (eclipse-zenoh/zenoh#2563); `tests/stamper.rs` pins it"
            },
            "dropped": 3,
            "keys_evicted": 0,
            "rows": [
                {
                    "row": "sample", "order_by": "arrival", "pos": 0,
                    "lane": lane,
                    "key": "acme/zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0",
                    "t_us": 1000, "hlc": "200/33", "stamped_by": "33",
                    "provenance": "owner", "kind": "put"
                },
                {
                    "row": "sample", "order_by": "arrival", "pos": 1,
                    "lane": lane,
                    "key": "acme/zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth1",
                    "t_us": 2000, "hlc": "100/33", "stamped_by": "33",
                    "provenance": "owner", "kind": "put"
                },
                {"row": "break", "order_by": "arrival", "pos": 2, "kind": "dropped", "n": 3},
                {
                    "row": "sample", "order_by": "arrival", "pos": 3,
                    "lane": {"kind": "unstamped"},
                    "key": "plain/key", "t_us": 3000, "kind": "put"
                }
            ]
        })
    );
}

/// The HLC axis: the claim is flattened into the envelope beside
/// `order_by`, the unstamped sample is a count rather than a row, and the
/// drop is a total with no row — a break has no position on this clock.
#[test]
fn a_timeline_on_the_hlc_axis_is_pinned() {
    let v = serde_json::to_value(fx::timeline_report_hlc()).unwrap();
    assert_eq!(v["order_by"], "hlc");
    assert_eq!(v["claim"], "happens_before");
    assert_eq!(v["stamper"], "33");
    assert_eq!(v["unstamped_excluded"], 1);
    assert_eq!(v["dropped"], 3);
    let rows = v["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "no break row, no unstamped row");
    assert_eq!(
        rows[0]["key"], "acme/zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth1",
        "the earlier stamp leads on this axis"
    );
    assert!(rows.iter().all(|r| r["order_by"] == "hlc"));
    // The other two claims, so a rename of either is a diff here too.
    assert_eq!(
        serde_json::to_value(AxisLabel::Hlc {
            claim: HlcClaim::SkewedWallClock {
                stampers: ["33".to_string(), "44".to_string()].into_iter().collect()
            }
        })
        .unwrap(),
        json!({"axis": "hlc", "claim": "skewed_wall_clock", "stampers": ["33", "44"]})
    );
    assert_eq!(
        serde_json::to_value(AxisLabel::Hlc {
            claim: HlcClaim::NoStampedSamples
        })
        .unwrap(),
        json!({"axis": "hlc", "claim": "no_stamped_samples"})
    );
}

// ─── snapshots (RFC 13 §4.4, #219; zk2's since #612, FJ8b) ────────────────

/// The `.zsnap` header, version 2: the span is the fact a capture header
/// does not carry, the O6 counters are absent at zero, and the presence
/// read is carried when made and absent when not.
#[test]
fn a_zsnap_header_states_its_span_and_omits_what_did_not_happen() {
    let h = fx::snapshot().header;
    assert_eq!(
        serde_json::to_value(&h).unwrap(),
        json!({
            "zsnap": 2,
            "selectors": ["acme/zk2/*/*/*/state/**"],
            "base": "acme",
            "collected_at": "2026-10-09T00:00:00Z",
            "collection_span_s": 1.25,
            "asked": 1,
            "answered": 5,
            "superseded": 1,
            "presence": {"selector": "zk2/*/*/@zk/**", "complete": true, "services": 2},
        })
    );
    let unasked = ZsnapHeader {
        presence: None,
        superseded: 0,
        ..h
    };
    let v = serde_json::to_value(&unasked).unwrap();
    assert!(v.get("presence").is_none(), "not asked is absent: {v}");
    assert!(v.get("superseded").is_none(), "zero is absent: {v}");
    assert!(v.get("elided").is_none() && v.get("errors").is_none());
    assert!(v.get("discarded").is_none());
}

/// Every row facet on the wire, pinned on the two rows that exercise the
/// most: a payload that fails its type answered by its owner, and a
/// deletion within the owner's window.
#[test]
fn a_snapshot_row_carries_its_facets_tagged() {
    let s = fx::snapshot();
    let eth0 = &s.rows[0];
    assert_eq!(
        serde_json::to_value(eth0).unwrap(),
        json!({
            "key": "acme/zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0",
            "identity": {
                "is": "resource",
                "address": "host-a/tc",
                "iface": "tc.netif.v1",
                "token": "state",
                "resource": "state/interfaces/{ns}/{iface}",
                "values": {"iface": ["eth0"], "ns": ["default"]},
            },
            "delete": false,
            "bytes": "eyJuYW1lIjoiZXRoMCIsImlzX3VwIjoieWVzIn0=",
            "encoding": "application/json",
            "timestamp": "7f3b2a1c00000001/ab12",
            "stamper": {"kind": "owner", "id": "ab12"},
            "source_zid": "ab12",
            "conformance": {"state": "invalid", "violations": ["/is_up: expected boolean"]},
            "holder": {"kind": "live", "address": "host-a/tc", "answered_by": "owner"},
        })
    );
    let deletion = &s.rows[1];
    let v = serde_json::to_value(deletion).unwrap();
    assert_eq!(v["delete"], true);
    assert!(
        v.get("bytes").is_none() && v.get("encoding").is_none(),
        "a deletion carries no bytes and no encoding — the delete is the whole fact"
    );
    assert_eq!(v["conformance"]["state"], "not_checked");
    assert_eq!(
        serde_json::to_value(&s.rows[3]).unwrap()["holder"],
        json!({"kind": "no_instance", "address": "host-b/tc"})
    );
    // Every row reads back to itself: the file is a contract both ways.
    for row in &s.rows {
        let back: SnapshotRow = serde_json::from_value(serde_json::to_value(row).unwrap()).unwrap();
        assert_eq!(&back, row);
    }
}

#[test]
fn a_snapshot_report_is_the_header_plus_holder_counts() {
    let r = fx::snapshot_report();
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["header"]["collection_span_s"], 1.25);
    assert_eq!(v["out"], "deployment.zsnap");
    assert_eq!(v["live"], 3);
    assert_eq!(v["no_instance"], 1);
    assert_eq!(v["unattributed"], 0, "a count, present at zero");
    assert_eq!(v["nonconforming"], 1);
    assert!(v.get("incomplete").is_none(), "empty is absent");
}

/// A diff carries both headers whole (both spans, RFC 13 §4.4), lists the
/// three key sets by zk2 key, and keeps the facets apart.
#[test]
fn a_snapshot_diff_keeps_both_spans_and_its_facets_apart() {
    let d = fx::snapshot_diff();
    let v = serde_json::to_value(&d).unwrap();
    assert_eq!(v["a"]["collection_span_s"], 1.25);
    assert_eq!(v["b"]["collection_span_s"], 0.8);
    assert_eq!(
        v["added"],
        json!(["zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth1"])
    );
    assert_eq!(
        v["removed"],
        json!(["zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth9"])
    );
    assert_eq!(v["unchanged"], 1);
    assert!(v.get("truncated").is_none());

    let changed = v["changed"].as_array().unwrap();
    assert_eq!(changed.len(), 2);
    assert_eq!(
        changed[0]["key"],
        "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0"
    );
    assert_eq!(changed[0]["value"]["changes"][0]["path"], "is_up");
    assert_eq!(changed[0]["conformance"][0]["state"], "invalid");
    assert_eq!(changed[0]["conformance"][1]["state"], "valid");
    assert!(
        changed[0].get("holder").is_none(),
        "the holder did not move"
    );
    assert_eq!(
        changed[1]["holder"],
        json!([
            {"kind": "no_instance", "address": "host-b/tc"},
            {"kind": "live", "address": "host-b/tc", "answered_by": "owner"},
        ]),
        "the holder facet as its own pair"
    );
    assert!(d.differs());
    assert_eq!(judgement_exit_code(&d.to_judgement()), 1);
    assert_eq!(
        judgement_exit_code(&fx::snapshot_diff_identity().to_judgement()),
        0
    );
}

/// Two namespaces line up on their zk2 keys, and the two deployments'
/// clocks are not compared: the same state there reads identical.
#[test]
fn two_namespaces_compare_by_zk2_key() {
    let d = fx::snapshot_diff_namespaces();
    assert!(!d.differs(), "{d:?}");
    assert_eq!(d.unchanged, 4);
    assert_eq!(judgement_exit_code(&d.to_judgement()), 0);
}

/// The exporter fold, whole: a stopped series has no `value`, a zero
/// `drop_exposed` is absent, every observer counter is present, the doctor
/// and registry poles are present because they were asked.
#[test]
fn an_export_snapshot_is_pinned() {
    assert_eq!(
        serde_json::to_value(fx::export_snapshot()).unwrap(),
        json!({
            "scopes": ["acme/v1/*/**"],
            "excluded": ["@rpc", "@media", "@blob", "@adv", "service origins"],
            "registry": {"producers": 2},
            "max_series": 10000,
            "started_at_unix_s": 1_700_000_000,
            "taken_at_unix_s": 1_700_000_120,
            "series": [
                {
                    "name": "zenkey_subject_sysinfo_cpu_usage_percent",
                    "key": "acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu/usage",
                    "origin": "h-3fa9c2d41b7e",
                    "producer": "sysinfo",
                    "class": "telemetry",
                    "subject": "cpu/usage",
                    "kind": "gauge",
                    "unit": "percent",
                    "value": 12.5,
                    "last_seen_unix_s": 1_700_000_119,
                    "state": "live",
                    "samples": 240,
                    "drop_exposed": 2,
                },
                {
                    "name": "zenkey_subject_sysinfo_disk_used_bytes",
                    "key": "acme/v1/h-0000deadbeef/telemetry/sysinfo/disk/var-log/used",
                    "origin": "h-0000deadbeef",
                    "producer": "sysinfo",
                    "class": "telemetry",
                    "subject": "disk/{mount}/used",
                    "labels": {"mount": "var-log"},
                    "unit": "bytes",
                    "last_seen_unix_s": 1_700_000_040,
                    "state": "origin_down",
                    "samples": 80,
                },
                {
                    "name": "zenkey_subject_netlink_iface_rx_bytes_total",
                    "key": "acme/v1/h-3fa9c2d41b7e/telemetry/netlink/iface/eth0/rx_bytes",
                    "origin": "h-3fa9c2d41b7e",
                    "producer": "netlink",
                    "class": "telemetry",
                    "subject": "iface/{iface}/rx_bytes",
                    "labels": {"iface": "eth0"},
                    "field": "rx",
                    "kind": "counter",
                    "unit": "bytes",
                    "last_seen_unix_s": 1_700_000_100,
                    "state": "evicted",
                    "samples": 5,
                },
                {
                    "name": "zenkey_subject_sysinfo_health",
                    "key": "acme/v1/h-3fa9c2d41b7e/state/sysinfo/health",
                    "origin": "h-3fa9c2d41b7e",
                    "producer": "sysinfo",
                    "class": "state",
                    "subject": "health",
                    "field": "uptime_s",
                    "value": 4242.0,
                    "last_seen_unix_s": 1_700_000_060,
                    "state": "quiet",
                    "samples": 4,
                },
            ],
            "observer": {
                "dropped": 3,
                "evicted_keys": 5,
                "evicted_bytes": 7,
                "expired": 11,
                "unwatched": 13,
                "coalesced": 17,
                "unstamped": 19,
            },
            "contract": {
                "qos_judged": 320,
                "qos_mismatch": 2,
                "qos_mismatch_by_subject": [{"producer": "sysinfo", "subject": "cpu/usage", "n": 2}],
                "payload_valid": 200,
                "payload_invalid": 1,
                "payload_not_validated": 128,
            },
            "suppressed": {"cardinality": 4, "fields": 1},
            "unregistered_keys": 3,
            "doctor": {
                "ran_at_unix_s": 1_700_000_090,
                "findings": [
                    {"check": "split-brain", "severity": "error", "subject": "host-a/tc tc.netif.v1"}
                ],
            },
        })
    );
}

/// The exposition is a pure function of the snapshot, pinned whole: the
/// four evicted populations are four lines, the three verdicts three, a
/// stopped series keeps its state line and has no value line, and nothing
/// in it moves without traffic (no scrape time).
#[test]
fn the_exposition_of_the_fixture_is_pinned() {
    let text = zenkey_fleet::exposition(&fx::export_snapshot());
    let expected = include_str!("fixtures/export.prom");
    assert_eq!(text, expected, "--- got ---\n{text}");
    assert!(
        !text.contains("1700000120"),
        "the scrape time is the scraper's"
    );
}
