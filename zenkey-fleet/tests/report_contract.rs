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
    let quiet = RateRow {
        key: "v1/h-a/telemetry/p/m".into(),
        count: 12,
        bytes: 480,
        sn_gaps: Asked::Asked(0),
        latency: None,
        unstamped: Asked::Asked(12),
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

    let unasked = RateRow {
        sn_gaps: Asked::NotAsked,
        unstamped: Asked::NotAsked,
        ..quiet.clone()
    };
    assert_eq!(
        serde_json::to_value(&unasked).unwrap(),
        json!({
            "key": "v1/h-a/telemetry/p/m",
            "count": 12,
            "bytes": 480,
        }),
        "no --loss and no --latency: both counters are absent (O4), never zero"
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

/// The conformance report (#222), whole: each assertion's state flat under
/// `state`, the reason only where the answer is "could not say", `exempt`
/// only where an exemption was claimed, and the window's drops carried — a
/// script keys on `id` and `state`, and CI on the verdict.
#[test]
fn a_conform_report_keeps_unknowable_apart_from_not_met() {
    assert_eq!(
        serde_json::to_value(fx::conform_report()).unwrap(),
        json!({
            "producer": "sysinfo",
            "slice_source": "dirs",
            "origins_asked": ["h-3fa9c2d41b7e"],
            "assertions": [
                {
                    "id": "procedure/introspect",
                    "subject": "@rpc/introspect",
                    "state": "met",
                    "evidence": "h-3fa9c2d41b7e: a value reply",
                    "citation": "RFC 08 §6",
                },
                {
                    "id": "procedure/dns",
                    "subject": "@rpc/dns",
                    "state": "met",
                    "evidence": "h-3fa9c2d41b7e: error/gated — conditional, and said so",
                    "citation": "RFC 08 §6.1",
                    "exempt": "when: config:collect.dns",
                },
                {
                    "id": "qos-observed-mismatch/health",
                    "subject": "state/health",
                    "state": "not_met",
                    "evidence": "v1/h-3fa9c2d41b7e/state/sysinfo/health: 4 of 4 sample(s) \
                                 did not ride the declared transition",
                    "citation": "RFC 04 §3",
                },
                {
                    "id": "observed/disk/{mount}/used",
                    "subject": "telemetry/disk/{mount}/used",
                    "state": "unknowable",
                    "reason": "a window proves presence, never absence",
                    "evidence": "not seen in 10s",
                    "citation": "RFC 13 §3",
                },
            ],
            "summary": {"met": 2, "not_met": 1, "unknowable": 1, "exempt": 1},
            "verdict": "violates",
            "observation": {
                "window_s": 10.0,
                "scopes": ["v1/*/state/**"],
                "samples": 40,
                "keys_seen": 2,
                "dropped": 3,
                "synthetic_marked": 40,
            },
            "deep": false,
            "not_asked": ["stale-state/*, budget: not asked without --deep"],
        })
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

/// The listen phase is additive: a report from a run without `--for` must
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
    let finding = V1Finding {
        severity: DoctorSeverity::Info,
        check: zenkey_fleet::report::V1CheckId::TimestampStampedElsewhere,
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

/// A `retired` run without a listen window and without a reachable admin
/// space serializes *no* wire facts at all — "not asked" is carried by
/// absence, never by zero or null (RFC 09 §5.1 O4, issue #226).
#[test]
fn a_retired_entry_omits_every_fact_that_was_never_asked() {
    let entry = RetiredEntry {
        producer: "logs".into(),
        path: "logs/errors_total".into(),
        since: None,
        replaced_by: None,
        selector: "v1/*/*/logs/logs/errors_total".into(),
        wire_samples: Asked::NotAsked,
        still_declared: None,
        subscribers: None,
        replacement_samples: Asked::NotAsked,
        verdict: CutoverVerdict::Unproven,
    };
    assert_eq!(
        serde_json::to_value(&entry).unwrap(),
        json!({
            "producer": "logs",
            "path": "logs/errors_total",
            "selector": "v1/*/*/logs/logs/errors_total",
            "verdict": "unproven",
        }),
        "an unasked fact is absent, not zero and not null"
    );
    // R6: `dropped` joined the window-gated wire facts — it serialized an
    // unconditional `0` here, claiming a clean observation on a run that
    // never observed. The pin change is the visible act.
    let report = RetiredReport {
        registries: vec!["registry".into()],
        entries: vec![],
        window_s: Asked::NotAsked,
        plane_samples: Asked::NotAsked,
        dropped: Asked::NotAsked,
        introspect_answered: 0,
        admin_entities: None,
        verdict: CutoverVerdict::Pass,
    };
    assert_eq!(
        serde_json::to_value(&report).unwrap(),
        json!({
            "registries": ["registry"],
            "entries": [],
            "introspect_answered": 0,
            "verdict": "pass",
        }),
        "no window: every wire fact — dropped included — stays absent; the \
         registries always state themselves"
    );
    // The shared fixture exercises the listened case: every fact present.
    let full = serde_json::to_value(fx::retired_report()).unwrap();
    // `30.0`: the seconds unification (#218) — see `timeout_s` above.
    assert_eq!(full["window_s"], 30.0);
    assert_eq!(full["dropped"], 5, "a listened run carries its drop count");
    assert_eq!(full["entries"][0]["still_declared"], true);
    assert_eq!(full["entries"][2]["verdict"], "unproven");
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
    fn cutover(v: &CutoverVerdict) -> &'static str {
        match v {
            CutoverVerdict::Pass => "pass",
            CutoverVerdict::OldStillSpeaks => "old_still_speaks",
            CutoverVerdict::Unproven => "unproven",
        }
    }
    fn expect(v: &ExpectVerdict) -> &'static str {
        match v {
            ExpectVerdict::Met => "met",
            ExpectVerdict::NotMet => "not_met",
            ExpectVerdict::Impaired => "impaired",
        }
    }
    // #222: the conformance suite's two vocabularies — the verdict, and the
    // per-assertion state it folds (a `state` tag, flattened).
    fn conform(v: &ConformVerdict) -> &'static str {
        match v {
            ConformVerdict::Conforms => "conforms",
            ConformVerdict::Violates => "violates",
            ConformVerdict::Unproven => "unproven",
        }
    }
    fn assertion_state(v: &AssertionState) -> &'static str {
        match v {
            AssertionState::Met => "met",
            AssertionState::NotMet => "not_met",
            AssertionState::Unknowable { .. } => "unknowable",
        }
    }
    fn conform_source(v: &ConformSource) -> &'static str {
        match v {
            ConformSource::Bus => "bus",
            ConformSource::Dirs => "dirs",
            ConformSource::Union => "union",
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
        CutoverVerdict::Pass,
        CutoverVerdict::OldStillSpeaks,
        CutoverVerdict::Unproven,
    ] {
        assert_eq!(wire(&v, ""), cutover(&v));
    }
    for v in [
        ExpectVerdict::Met,
        ExpectVerdict::NotMet,
        ExpectVerdict::Impaired,
    ] {
        assert_eq!(wire(&v, ""), expect(&v));
    }
    for v in [
        ConformVerdict::Conforms,
        ConformVerdict::Violates,
        ConformVerdict::Unproven,
    ] {
        assert_eq!(wire(&v, ""), conform(&v));
    }
    for v in [
        AssertionState::Met,
        AssertionState::NotMet,
        AssertionState::Unknowable { reason: "r".into() },
    ] {
        assert_eq!(wire(&v, "state"), assertion_state(&v));
    }
    for v in [
        ConformSource::Bus,
        ConformSource::Dirs,
        ConformSource::Union,
    ] {
        assert_eq!(wire(&v, ""), conform_source(&v));
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

/// The `why` ladder's wire shape (#214): one rung per stable id in order, the
/// three-state answer flattened as a snake_case tag — and `not_asked` carries
/// **no** `reason`, because a question that was not put has no negative
/// answer to spell (RFC 09 §5.1 O4). Absent optionals stay absent: an empty
/// `impairments` and an unrequested listen window serialize as nothing, not
/// as `[]`/`null`.
#[test]
fn a_why_rung_keeps_not_asked_distinct_on_the_wire() {
    let v = serde_json::to_value(fx::why_report()).unwrap();
    assert_eq!(v["verdict"], "healthy");
    assert!(
        v.get("impairments").is_none(),
        "no impairments is absence, not an empty list"
    );
    assert!(
        v.get("listened_s").is_none(),
        "not listened is absence (O4), never null"
    );
    let ids: Vec<&str> = v["rungs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        zenkey_fleet::report::RungId::ALL.map(zenkey_fleet::report::RungId::as_str),
        "one rung per id, in order"
    );

    let established = &v["rungs"][0];
    assert_eq!(established["answer"], "established");
    assert!(established.get("reason").is_none());

    let lazy = &v["rungs"][4];
    assert_eq!(lazy["id"], "publisher-declared");
    assert_eq!(lazy["answer"], "not_established");
    assert!(
        lazy["reason"]
            .as_str()
            .unwrap()
            .contains("publishers declare lazily"),
        "the RFC 08 §6.1 wording rides the wire: {lazy}"
    );
    assert!(
        lazy.get("evidence").is_none(),
        "empty evidence is absent, not []"
    );

    let not_asked = &v["rungs"][9];
    assert_eq!(not_asked["id"], "wire-heard");
    assert_eq!(not_asked["answer"], "not_asked");
    assert!(
        not_asked.get("reason").is_none(),
        "not_asked has no negative answer to spell"
    );
}

// ── The consumers join (#224) ─────────────────────────────────────────────

// ── The fleet timeline (#216) ─────────────────────────────────────────────

/// The arrival axis: every row carries `order_by`, the break sits at its
/// position with no lane, the unstamped lane exists, and the
/// sequence-number lane is *unavailable* with its fixed reason — never an
/// empty list. `unstamped_excluded` is absent at zero.
#[test]
fn a_timeline_on_the_arrival_axis_is_pinned() {
    assert_eq!(
        serde_json::to_value(fx::timeline_report_arrival()).unwrap(),
        json!({
            "order_by": "arrival",
            "axis": "arrival",
            "clock": "observer monotonic, µs since window start",
            "scopes": ["acme/v1/**"],
            "window_s": 10.0,
            "source": {"kind": "live"},
            "lanes": [
                {
                    "lane": {"kind": "origin", "origin": "h-3fa9c2d41b7e", "producer": "sysinfo"},
                    "samples": 2,
                    "first_t_us": 1000,
                    "last_t_us": 2000,
                    "stampers": ["33"],
                    "provenance": {"self_stamped": 0, "foreign": 0, "unattributable": 2}
                },
                {
                    "lane": {"kind": "unstamped"},
                    "samples": 1,
                    "first_t_us": 3000,
                    "last_t_us": 3000,
                    "stampers": [],
                    "provenance": {"self_stamped": 0, "foreign": 0, "unattributable": 0}
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
                    "lane": {"kind": "origin", "origin": "h-3fa9c2d41b7e", "producer": "sysinfo"},
                    "key": "acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu",
                    "t_us": 1000, "hlc": "200/33", "stamped_by": "33",
                    "provenance": "unattributable", "kind": "put"
                },
                {
                    "row": "sample", "order_by": "arrival", "pos": 1,
                    "lane": {"kind": "origin", "origin": "h-3fa9c2d41b7e", "producer": "sysinfo"},
                    "key": "acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/mem",
                    "t_us": 2000, "hlc": "100/33", "stamped_by": "33",
                    "provenance": "unattributable", "kind": "put"
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
    assert_eq!(
        serde_json::to_value(fx::timeline_report_hlc()).unwrap(),
        json!({
            "order_by": "hlc",
            "axis": "hlc",
            "claim": "happens_before",
            "stamper": "33",
            "scopes": ["acme/v1/**"],
            "window_s": 10.0,
            "source": {"kind": "live"},
            "lanes": [{
                "lane": {"kind": "origin", "origin": "h-3fa9c2d41b7e", "producer": "sysinfo"},
                "samples": 2,
                "first_t_us": 1000,
                "last_t_us": 2000,
                "stampers": ["33"],
                "provenance": {"self_stamped": 0, "foreign": 0, "unattributable": 2}
            }],
            "sn_lane": {
                "state": "unavailable",
                "reason": "zenoh 1.9/1.10 deliver no SourceInfo to subscribers (eclipse-zenoh/zenoh#2563); `tests/stamper.rs` pins it"
            },
            "unstamped_excluded": 1,
            "dropped": 3,
            "keys_evicted": 0,
            "rows": [
                {
                    "row": "sample", "order_by": "hlc", "pos": 0,
                    "lane": {"kind": "origin", "origin": "h-3fa9c2d41b7e", "producer": "sysinfo"},
                    "key": "acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/mem",
                    "t_us": 2000, "hlc": "100/33", "stamped_by": "33",
                    "provenance": "unattributable", "kind": "put"
                },
                {
                    "row": "sample", "order_by": "hlc", "pos": 1,
                    "lane": {"kind": "origin", "origin": "h-3fa9c2d41b7e", "producer": "sysinfo"},
                    "key": "acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu",
                    "t_us": 1000, "hlc": "200/33", "stamped_by": "33",
                    "provenance": "unattributable", "kind": "put"
                }
            ]
        })
    );
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

// ─── snapshots (RFC 13 §4.4, #219) ───────────────────────────────────────

/// The `.zsnap` header: the span is the fact a capture header does not
/// carry, the O6 counters are absent at zero, and `roster` is a count when
/// asked and absent when not.
#[test]
fn a_zsnap_header_states_its_span_and_omits_what_did_not_happen() {
    let h = fx::snapshot().header;
    assert_eq!(
        serde_json::to_value(&h).unwrap(),
        json!({
            "zsnap": 1,
            "selectors": ["acme/v1/**"],
            "base": "acme",
            "collected_at": "2026-09-06T00:00:00Z",
            "collection_span_s": 1.25,
            "asked": 1,
            "answered": 6,
            "superseded": 1,
            "roster": 2,
        })
    );
    let unasked = ZsnapHeader {
        roster: Asked::NotAsked,
        superseded: 0,
        ..h
    };
    let v = serde_json::to_value(&unasked).unwrap();
    assert!(v.get("roster").is_none(), "not asked is absent: {v}");
    assert!(v.get("superseded").is_none(), "zero is absent: {v}");
    assert!(v.get("elided").is_none() && v.get("errors").is_none());
}

/// Every row facet on the wire, pinned on the two rows that exercise the
/// most: a live host answering its own stamped value, and a tombstone.
#[test]
fn a_snapshot_row_carries_its_four_facets_tagged() {
    let s = fx::snapshot();
    let health = &s.rows[1];
    assert_eq!(
        serde_json::to_value(health).unwrap(),
        json!({
            "key": "acme/v1/h-3fa9c2d41b7e/state/sysinfo/health",
            "delete": false,
            "bytes": "eyJzb3VyY2UiOiJub2RlLWEiLCJob3N0X2lkIjoiaC0zZmE5YzJkNDFiN2UiLCJzdGF0dXMiOiJvayJ9",
            "encoding": "application/json",
            "timestamp": "7f3b2a1c00000001/ab12",
            "stamper": {"kind": "unattributable", "id": "ab12"},
            "source_zid": "ab12",
            "registration": "registered",
            "verdict": {"state": "valid"},
            "holder": {"kind": "live", "origin": "h-3fa9c2d41b7e", "answered_by": "stamper"},
        })
    );
    let tombstone = &s.rows[3];
    assert_eq!(
        serde_json::to_value(tombstone).unwrap(),
        json!({
            "key": "acme/v1/h-9b2e4c7a1d05/state/logs/rotated",
            "delete": true,
            "timestamp": "7f3b2a1c00000001/ab12",
            "stamper": {"kind": "unattributable", "id": "ab12"},
            "source_zid": "ab12",
            "registration": "registered",
            "verdict": {"state": "not_validated", "reason": "tombstone"},
            "holder": {"kind": "storage_only", "origin": "h-9b2e4c7a1d05"},
        }),
        "a tombstone carries no bytes and no encoding — the delete is the whole fact"
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
    assert_eq!(v["out"], "fleet.zsnap");
    assert_eq!(v["live"], 2);
    assert_eq!(v["storage_only"], 2);
    assert_eq!(v["unattributed"], 1);
    assert!(v.get("incomplete").is_none(), "empty is absent");
}

/// A diff carries both headers whole (both spans, RFC 13 §4.4), lists the
/// three key sets, keeps the facets apart, and — with no alignment asked —
/// carries neither the origin map nor the roll-up.
#[test]
fn a_snapshot_diff_keeps_both_spans_and_its_facets_apart() {
    let d = fx::snapshot_diff();
    let v = serde_json::to_value(&d).unwrap();
    assert_eq!(v["a"]["collection_span_s"], 1.25);
    assert_eq!(v["b"]["collection_span_s"], 0.8);
    assert_eq!(
        v["added"],
        json!(["acme/v1/h-9b2e4c7a1d05/telemetry/sysinfo/disk/var-log/used"])
    );
    assert_eq!(
        v["removed"],
        json!(["acme/v1/h-9b2e4c7a1d05/state/logs/rotated"])
    );
    assert_eq!(v["unchanged"], 2);
    for absent in ["truncated", "origin_map", "unmapped", "by_subject"] {
        assert!(v.get(absent).is_none(), "{absent} not asked: {v}");
    }

    let changed = v["changed"].as_array().unwrap();
    assert_eq!(changed.len(), 2);
    assert_eq!(
        changed[0],
        json!({
            "key": "acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used",
            "value": {
                "changes": [{"op": "changed", "path": "value", "old": 41.0, "new": 42.0}],
                "truncated": 0,
            },
            "timestamp": ["7f3b2a1c00000001/ab12", "7f3b2a1c00000002/ab12"],
        }),
        "a value change carries the value facet and nothing else"
    );
    assert_eq!(
        changed[1]["holder"],
        json!([
            {"kind": "storage_only", "origin": "h-9b2e4c7a1d05"},
            {"kind": "live", "origin": "h-9b2e4c7a1d05", "answered_by": "unknown"},
        ]),
        "the holder facet rides beside the value facet, as its own pair"
    );
    assert_eq!(changed[1]["value"]["changes"][0]["path"], "status");
    assert!(changed[1].get("verdict").is_none() && changed[1].get("registration").is_none());
    assert!(d.differs());
    assert_eq!(judgement_exit_code(&d.to_judgement()), 1);
    assert_eq!(
        judgement_exit_code(&fx::snapshot_diff_identity().to_judgement()),
        0
    );
}

/// An alignment that was asked and refused (#220) lists what it paired
/// *and* what it could not (RFC 13 §4.4: "MUST list, never drop"), carries
/// no roll-up — the comparison was not made — and projects to the reserved
/// non-verdict.
#[test]
fn a_refused_alignment_lists_its_pairs_and_its_unpaired_and_compares_nothing() {
    let d = fx::snapshot_diff_unmapped();
    let v = serde_json::to_value(&d).unwrap();
    assert_eq!(
        v["origin_map"],
        json!([{
            "a": "h-3fa9c2d41b7e",
            "b": "h-c0ffee00c0de",
            "evidence": {"kind": "explicit"},
        }])
    );
    assert_eq!(
        v["unmapped"],
        json!([
            {
                "origin": "h-9b2e4c7a1d05",
                "side": "a",
                "reason": "label `db` claimed by no origin in b; producer set {logs, sysinfo} matches no origin in b",
            },
            {
                "origin": "h-0badcafe1234",
                "side": "b",
                "reason": "label `node` claimed by no origin in a; producer set {sysinfo} matches no origin in a",
            },
        ])
    );
    for absent in ["by_subject", "truncated"] {
        assert!(v.get(absent).is_none(), "{absent}: {v}");
    }
    assert_eq!(v["added"], json!([]));
    assert_eq!(v["changed"], json!([]));
    assert_eq!(v["unchanged"], 0);
    assert!(d.refused());
    assert_eq!(judgement_exit_code(&d.to_judgement()), 2);
}

/// An alignment that completed carries every pair with its evidence and
/// one `by_subject` row per subject — the acceptance case: disjoint
/// origins, zero differences, exit 0.
#[test]
fn a_completed_alignment_carries_its_pairs_and_the_subject_roll_up() {
    let d = fx::snapshot_diff_aligned();
    let v = serde_json::to_value(&d).unwrap();
    assert_eq!(
        v["origin_map"],
        json!([
            {"a": "h-3fa9c2d41b7e", "b": "h-c0ffee00c0de", "evidence": {"kind": "label", "source": "web"}},
            {"a": "h-9b2e4c7a1d05", "b": "h-0badcafe1234", "evidence": {"kind": "label", "source": "db"}},
        ])
    );
    assert!(v.get("unmapped").is_none(), "empty is absent: {v}");
    assert_eq!(v["added"], json!([]));
    assert_eq!(v["removed"], json!([]));
    assert_eq!(v["changed"], json!([]));
    assert_eq!(v["unchanged"], 6);
    let subjects = v["by_subject"].as_array().unwrap();
    assert_eq!(subjects.len(), 4);
    assert_eq!(
        subjects[2],
        json!({"subject": "state/sysinfo/health", "compared": 2, "differing": 0, "only_in_a": 0, "only_in_b": 0}),
        "no example when nothing differs"
    );
    assert!(!d.differs() && !d.refused());
    assert_eq!(judgement_exit_code(&d.to_judgement()), 0);

    // Mapped by hand and differing: the subject carries its example.
    let v = serde_json::to_value(fx::snapshot_diff_normalized()).unwrap();
    assert_eq!(v["origin_map"][0]["evidence"], json!({"kind": "explicit"}));
    let health = v["by_subject"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["subject"] == "state/sysinfo/health")
        .unwrap();
    assert_eq!(
        (health["compared"].as_u64(), health["differing"].as_u64()),
        (Some(2), Some(2))
    );
    assert_eq!(health["example"]["value"]["changes"][0]["path"], "source");
    let logs = v["by_subject"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["subject"] == "state/logs/rotated")
        .unwrap();
    assert_eq!(logs["only_in_a"], 1);
}

// ── The metrics surface (#228) ───────────────────────────────────────────────

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
                    {"check": "stale-state", "severity": "warning", "subject": "h-3fa9c2d41b7e/sysinfo"}
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
