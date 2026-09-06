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

/// `open_ended: false` serializes; `deprecated: false` does **not**. Both are
/// bools on the same struct, and the asymmetry is deliberate — a deprecation
/// is news, a non-deprecation is not — but it is exactly the kind of thing a
/// refactor unifies by accident.
#[test]
fn a_false_deprecated_flag_is_absent_while_a_false_open_ended_flag_is_not() {
    let plain = fx::topic_row();
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

    let retired = fx::topic_row_retired();
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
        kind: None,
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

/// The NO-DEAD-FIELD PIN (report-honesty finding R2, third recurrence of the
/// class: `cardinality` was declared and never filled until #221; `rate`
/// reached `SubjectFacts` and died at the report boundary; `since` and
/// `description` never left the slice at all).
///
/// The guard is structural: `fx::topic_info_full()` populates **every**
/// `Option` the shape can serialize, and this test drives the one real
/// constructor path — slice TOML → `describe_key` → `from_description` — and
/// requires the two documents to be identical. A field added to `TopicInfo`
/// that the constructor cannot fill fails here the day it lands, instead of
/// serializing as a permanent absence that reads like "not declared" (O4).
#[test]
fn no_topic_info_field_is_dead_the_constructor_reaches_them_all() {
    let toml = r#"
        [registry]
        version = "1.0"
        app = "t"
        convention = 1
        [producer]
        name = "sysinfo"
        [[subject]]
        path = "disk/{mount}/used"
        class = "telemetry"
        type = "TelemetryPoint"
        unit = "bytes"
        kind = "gauge"
        qos = "sampled"
        ttl_s = 120
        rate = "low"
        cardinality = 16
        encoding = "application/cbor"
        since = "1.0"
        description = "bytes used per mount"
    "#;
    let slices =
        zenkey_fleet::SliceSet::from_slices(vec![zenkey::parse_slice(toml).expect("slice parses")]);
    let described = zenkey_fleet::model::facts::describe_key(
        "",
        &format!("v1/{}/telemetry/sysinfo/disk/var-log/used", fx::ORIGIN),
        Some(&slices),
    );
    let built = serde_json::to_value(TopicInfo::from_description(&described)).unwrap();
    let full = serde_json::to_value(fx::topic_info_full()).unwrap();
    for key in full.as_object().unwrap().keys() {
        assert!(
            built.get(key).is_some(),
            "TopicInfo.{key} is dead: the fixture serializes it, but the \
             constructor path never fills it"
        );
    }
    assert_eq!(
        built, full,
        "the constructor's document IS the every-field fixture — no field \
         reachable only by literal construction"
    );
}

/// The flag that disambiguates the two `None`s beside it. Without it, "asked
/// and nothing was served" and "never asked" are the same document.
#[test]
fn a_node_list_says_whether_the_slice_join_was_even_attempted() {
    let unasked = fx::node_list_unjoined();
    let asked = fx::node_list();
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
        "a row with no app means two different things, and only the flag says which"
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
    let finding = DoctorFinding {
        severity: DoctorSeverity::Info,
        check: zenkey_fleet::report::CheckId::TimestampStampedElsewhere,
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

/// `interface show` carries the engine's drift verdict beside its rows
/// (#410), and only when there is one: absent on an unasked run (where
/// `schemas` is absent too) and absent when the carriers agree, so the
/// pre-#410 document is unchanged for both. When present it is the same
/// `SchemaDrift` the doctor serialises, origin and all — one shape, two
/// pages.
#[test]
fn an_interface_show_omits_drift_until_something_disagrees() {
    assert_eq!(
        serde_json::to_value(fx::interface_show_unasked()).unwrap(),
        json!({
            "type_name": "HealthSnapshot",
            "carriers": [
                {"producer": "sysinfo", "class": "state", "path": "health"},
                {"producer": "gnmi", "class": "state", "path": "health"},
            ],
        }),
        "unasked: neither schemas nor drift, and never an empty list of either"
    );
    assert_eq!(
        serde_json::to_value(fx::interface_show()).unwrap(),
        json!({
            "type_name": "HealthSnapshot",
            "carriers": [
                {"producer": "sysinfo", "class": "state", "path": "health"},
                {"producer": "gnmi", "class": "state", "path": "health"},
            ],
            "schemas": [
                {
                    "producer": "sysinfo",
                    "type_name": "HealthSnapshot",
                    "kind": "json-schema",
                    "hash": "sha256:aaaa",
                },
                {
                    "producer": "gnmi",
                    "type_name": "HealthSnapshot",
                    "kind": "json-schema",
                    "hash": "sha256:bbbb",
                },
            ],
            "drift": [{
                "type_name": "HealthSnapshot",
                "servers": [
                    {"producer": "sysinfo", "origin": "h-aaaaaaaaaaaa", "hash": "sha256:aaaa"},
                    {"producer": "gnmi", "origin": "h-bbbbbbbbbbbb", "hash": "sha256:bbbb"},
                ],
                "verdict": "disagree",
            }],
        })
    );
    // The third state: served, and one host said nothing — its `hash` is
    // absent, never `""` or `null` (#370).
    let value = serde_json::to_value(fx::interface_show_unjudgeable()).unwrap();
    assert_eq!(value["drift"][0]["verdict"], "unjudgeable");
    assert_eq!(
        value["drift"][0]["servers"][1],
        json!({"producer": "sysinfo", "origin": "h-bbbbbbbbbbbb"})
    );
}

// ── The consumers join (#224) ─────────────────────────────────────────────

/// The admin discriminator rides flattened at the top of the document, and
/// its two states never share a spelling: `answered` carries both counts,
/// `not_available` carries none. Per row, `origins` is absent when empty
/// (session only, unattributed — not `[]`, not `null`) and `is_self` /
/// `total_wildcard` are absent when false (O4: news, not non-news).
#[test]
fn consumers_report_json_shape_is_pinned() {
    let v = serde_json::to_value(fx::consumers_report()).unwrap();
    assert_eq!(
        v,
        json!({
            "target": "acme/v1/*/state/sysinfo/health",
            "asked": [
                "@/*/*",
                "@/*/*/subscriber/**",
                "@/*/*/publisher/**",
                "@/*/*/queryable/**",
                "@/*/*/querier/**",
                "@/*/*/token/**",
            ],
            "self_zid": "ffffffff",
            "admin": "answered",
            "answered": 1,
            "nodes": 2,
            "rows": [
                {
                    "zid": "eeff0011",
                    "whatami": "peer",
                    "origins": [fx::ORIGIN],
                    "attribution": "session",
                    "kind": "subscriber",
                    "keyexpr": format!("acme/v1/{}/state/sysinfo/health", fx::ORIGIN),
                    "relation": "narrower",
                },
                {
                    "zid": "ffffffff",
                    "whatami": "peer",
                    "attribution": "session",
                    "kind": "querier",
                    "keyexpr": "acme/v1/*/state/sysinfo/health",
                    "relation": "exact",
                    "is_self": true,
                },
                {
                    "zid": "aabbccdd",
                    "whatami": "router",
                    "attribution": "reported_only",
                    "kind": "subscriber",
                    "keyexpr": "**",
                    "relation": "total",
                    "total_wildcard": true,
                },
            ],
            "reply_elided": 0,
        })
    );

    let v = serde_json::to_value(fx::consumers_not_available()).unwrap();
    assert_eq!(v["admin"], "not_available");
    assert!(v.get("answered").is_none(), "{v}");
    assert!(v.get("nodes").is_none(), "{v}");
    assert_eq!(v["rows"], json!([]));
}

/// The blast radius: the consumers document nested whole, coverage and the
/// two declaration counts present only when the admin space answered,
/// the ledger entry only when there is one.
#[test]
fn subject_impact_json_shape_is_pinned() {
    let v = serde_json::to_value(fx::subject_impact()).unwrap();
    assert_eq!(
        v,
        json!({
            "producer": "sysinfo",
            "path": "health",
            "class": "state",
            "selector": "acme/v1/*/state/sysinfo/health",
            "consumers": serde_json::to_value(fx::consumers_report()).unwrap(),
            "coverage": [{
                "producer": "sysinfo",
                "path": "health",
                "ttl_s": 120,
                "coverage": "covered",
                "storage": "main@aabbccdd",
            }],
            "declared_publishers": 2,
            "declared_queryables": 0,
            "deprecated": { "since": "2.0", "replaced_by": "status" },
        })
    );

    // Not asked: every admin-derived field is absent, never zero or `[]`.
    let unasked = SubjectImpact {
        consumers: fx::consumers_not_available(),
        coverage: None,
        declared_publishers: None,
        declared_queryables: None,
        deprecated: None,
        ..fx::subject_impact()
    };
    let v = serde_json::to_value(unasked).unwrap();
    for absent in [
        "coverage",
        "declared_publishers",
        "declared_queryables",
        "deprecated",
    ] {
        assert!(v.get(absent).is_none(), "{absent} must be absent: {v}");
    }
    assert_eq!(v["consumers"]["admin"], "not_available");
}

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
