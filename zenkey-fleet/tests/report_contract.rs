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
//! Where a shape here looks inconsistent with its neighbours, it is pinned as
//! it *is* and the inconsistency is filed. Changing a wire contract is chunk
//! AI's job, under `breaking`.

use serde_json::json;
use zenkey_fleet::report::*;
use zenkey_fleet::{Coverage, CoverageRow};

/// `open_ended: false` serializes; `deprecated: false` does **not**. Both are
/// bools on the same struct, and the asymmetry is deliberate — a deprecation
/// is news, a non-deprecation is not — but it is exactly the kind of thing a
/// refactor unifies by accident.
#[test]
fn a_false_deprecated_flag_is_absent_while_a_false_open_ended_flag_is_not() {
    let plain = TopicRow {
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
    };
    assert_eq!(
        serde_json::to_value(&plain).unwrap(),
        json!({
            "producer": "sysinfo",
            "registry_version": "1.0",
            "class": "telemetry",
            "path": "disk/{mount}/used",
            "type_name": "TelemetryPoint",
            "open_ended": false,
        }),
        "a false `deprecated` is absent, a false `open_ended` is present"
    );

    let retired = TopicRow {
        open_ended: true,
        since: Some("1.0".into()),
        deprecated: true,
        deprecated_since: Some("2.0".into()),
        replaced_by: Some("disk/{mount}/bytes_used".into()),
        ..plain
    };
    assert_eq!(
        serde_json::to_value(&retired).unwrap(),
        json!({
            "producer": "sysinfo",
            "registry_version": "1.0",
            "class": "telemetry",
            "path": "disk/{mount}/used",
            "type_name": "TelemetryPoint",
            "open_ended": true,
            "since": "1.0",
            "deprecated": true,
            "deprecated_since": "2.0",
            "replaced_by": "disk/{mount}/bytes_used",
        })
    );
}

/// The ladder's rungs are the wire vocabulary scripts branch on, and a
/// half-climbed ladder omits the fields it never reached rather than nulling
/// them (O2 + O4 in one document).
#[test]
fn the_topic_verdict_vocabulary_is_snake_case_and_partial_reports_omit() {
    for (verdict, wire) in [
        (TopicVerdict::Registered, "registered"),
        (TopicVerdict::Unregistered, "unregistered"),
        (TopicVerdict::NoSliceForProducer, "no_slice_for_producer"),
        (TopicVerdict::NotADataClass, "not_a_data_class"),
        (TopicVerdict::NotV1, "not_v1"),
        (TopicVerdict::NotUnderBase, "not_under_base"),
        (TopicVerdict::RegistryNotLoaded, "registry_not_loaded"),
    ] {
        assert_eq!(serde_json::to_value(verdict).unwrap(), json!(wire));
    }

    // A key that did not get past rung 1: everything below is absent.
    let stopped = TopicInfo {
        key: "demo/example/foo".into(),
        verdict: TopicVerdict::NotUnderBase,
        note: "not under this base".into(),
        origin: None,
        producer: None,
        class: None,
        subject: None,
        variables: Default::default(),
        payload_type: None,
        unit: None,
        qos: None,
        ttl_s: None,
        rate: None,
        cardinality: None,
        encoding: None,
        since: None,
        description: None,
    };
    assert_eq!(
        serde_json::to_value(&stopped).unwrap(),
        json!({
            "key": "demo/example/foo",
            "verdict": "not_under_base",
            "note": "not under this base",
        }),
        "fields below the failure point are absent, never defaulted"
    );
}

/// The flag that disambiguates the two `None`s beside it. Without it, "asked
/// and nothing was served" and "never asked" are the same document.
#[test]
fn a_node_list_says_whether_the_slice_join_was_even_attempted() {
    let row = NodeRow {
        origin: "h-3fa9c2d41b7e".into(),
        producer: "sysinfo".into(),
        app: None,
        registry_version: None,
    };
    let unasked = NodeList {
        nodes: vec![row.clone()],
        slices_joined: false,
    };
    let asked = NodeList {
        nodes: vec![row],
        slices_joined: true,
    };
    assert_eq!(
        serde_json::to_value(&unasked).unwrap(),
        json!({
            "nodes": [{"origin": "h-3fa9c2d41b7e", "producer": "sysinfo"}],
            "slices_joined": false,
        })
    );
    assert_eq!(
        serde_json::to_value(&asked).unwrap()["slices_joined"],
        json!(true),
        "same rows, different meaning — the flag is the whole difference"
    );
}

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
        origins: None,
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
        origins: Some(vec![]),
        ..base.clone()
    };
    assert_eq!(
        serde_json::to_value(&asked_silent).unwrap()["origins"],
        json!([]),
        "asked, nobody answered: an empty list, which is not absence"
    );

    let alive = BlobTierRow {
        origins: Some(vec!["h-3fa9c2d41b7e".into()]),
        ..base
    };
    assert_eq!(
        serde_json::to_value(&alive).unwrap()["origins"],
        json!(["h-3fa9c2d41b7e"])
    );
}

/// The trickiest shape in the file: a `#[serde(flatten)]` over an adjacently
/// tagged enum. Note the tag values are **PascalCase** while every other enum
/// in `report.rs` is snake or kebab — pinned as it is, and filed rather than
/// quietly changed, because it is a wire contract.
#[test]
fn coverage_flattens_into_its_row_with_pascal_case_tags() {
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
            "coverage": "Covered",
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
        json!({"producer": "sysinfo", "path": "health", "coverage": "Uncovered"}),
        "no storage key at all when nothing covers it, and no ttl when none \
         is declared"
    );

    assert_eq!(
        serde_json::to_value(Coverage::Partial("main@abc".into())).unwrap(),
        json!({"coverage": "Partial", "storage": "main@abc"})
    );
}

/// Six optionals on one struct, every one of them a wire fact that is absent
/// rather than null when it did not ride.
#[test]
fn a_call_answer_omits_every_part_the_wire_did_not_carry() {
    let bare = CallAnswer {
        origin: "h-3fa9c2d41b7e".into(),
        ok: true,
        value: None,
        text: None,
        attachment: None,
        attachment_bytes: None,
        error: None,
    };
    assert_eq!(
        serde_json::to_value(&bare).unwrap(),
        json!({"origin": "h-3fa9c2d41b7e", "ok": true})
    );

    let failed = CallAnswer {
        ok: false,
        error: Some(CallError {
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

    // Silence is exit 2 and an empty answer list — never an error reply.
    let silent = CallReport {
        key: "v1/*/@rpc/sysinfo/introspect".into(),
        answers: vec![],
    };
    assert_eq!(silent.exit_code(), 2);
    assert_eq!(
        serde_json::to_value(&silent).unwrap(),
        json!({"key": "v1/*/@rpc/sysinfo/introspect", "answers": []})
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
    let quiet = RateRow {
        key: "v1/h-a/telemetry/p/m".into(),
        count: 12,
        bytes: 480,
        sn_gaps: 0,
        latency: None,
        unstamped: 12,
    };
    assert_eq!(
        serde_json::to_value(&quiet).unwrap(),
        json!({
            "key": "v1/h-a/telemetry/p/m",
            "count": 12,
            "bytes": 480,
            "sn_gaps": 0,
            "unstamped": 12,
        }),
        "nothing stamped: the latency key is absent, which is not zero latency"
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
        unstamped: 1,
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

/// The three-state verdicts. Silence has its own word in both, and that is the
/// whole reason they are not booleans.
#[test]
fn the_three_state_verdicts_keep_their_third_state() {
    for (v, wire) in [
        (CutoverVerdict::Pass, "pass"),
        (CutoverVerdict::OldStillSpeaks, "old_still_speaks"),
        (CutoverVerdict::Unproven, "unproven"),
    ] {
        assert_eq!(serde_json::to_value(v).unwrap(), json!(wire));
    }
    for (v, wire) in [
        (ExpectVerdict::Met, "met"),
        (ExpectVerdict::NotMet, "not_met"),
        (ExpectVerdict::Impaired, "impaired"),
    ] {
        assert_eq!(serde_json::to_value(v).unwrap(), json!(wire));
    }

    let clean = ExpectReport {
        selector: "v1/**".into(),
        window_s: 5.0,
        ended_early: false,
        samples: 0,
        keys_seen: 0,
        dropped: 0,
        rate_hz: None,
        violations: vec![],
        violations_total: 0,
        unmet: vec!["saw 0 sample(s), wanted at least 1".into()],
        verdict: ExpectVerdict::NotMet,
    };
    assert_eq!(
        serde_json::to_value(&clean).unwrap(),
        json!({
            "selector": "v1/**",
            "window_s": 5.0,
            "ended_early": false,
            "samples": 0,
            "keys_seen": 0,
            "dropped": 0,
            "violations_total": 0,
            "unmet": ["saw 0 sample(s), wanted at least 1"],
            "verdict": "not_met",
        }),
        "an empty violation list is absent, and `rate_hz: None` means not asked"
    );
}

/// The `@blob` documents must serialize identically whether or not the binary
/// was built with the transport — `report.rs`'s own header says so, and this
/// file is compiled without the `blob` feature, which is the proof.
#[test]
fn a_blob_probe_reports_what_it_asked_and_what_answered() {
    let empty = BlobProbeReport {
        target: "01hq9k".into(),
        tier: "artifact".into(),
        asked: vec!["v1/*/@blob/artifact/01hq9k/manifest".into()],
        not_probed: None,
        holders: vec![],
        answered: 0,
        roots: vec![],
        declared_by: vec!["artifacts".into()],
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
        }),
        "asked but unanswered: the selector is on record, so silence is \
         visibly a non-verdict rather than an absent question"
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

/// The listen phase is additive: a report from a run without `--listen-for` must
/// be byte-identical to one from before the phase existed, so a pre-#161
/// consumer keeps parsing. `doctor_report_json_shape_is_pinned` covers the
/// document; this covers the severity vocabulary its findings branch on.
#[test]
fn doctor_severities_are_the_stable_lowercase_vocabulary() {
    for (s, wire) in [
        (DoctorSeverity::Error, "error"),
        (DoctorSeverity::Warning, "warning"),
        (DoctorSeverity::Info, "info"),
    ] {
        assert_eq!(serde_json::to_value(s).unwrap(), json!(wire));
    }
    let finding = DoctorFinding {
        severity: DoctorSeverity::Info,
        check: "timestamp-stamped-elsewhere".into(),
        subject: "fleet".into(),
        evidence: "stamped by 1 node that is not the publisher".into(),
        citation: None,
    };
    assert_eq!(
        serde_json::to_value(&finding).unwrap(),
        json!({
            "severity": "info",
            "check": "timestamp-stamped-elsewhere",
            "subject": "fleet",
            "evidence": "stamped by 1 node that is not the publisher",
        }),
        "an uncited finding omits the key rather than nulling it"
    );
}
