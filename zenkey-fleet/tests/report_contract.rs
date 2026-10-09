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
//! itself in this file to land. The rule stands for the next one.

use serde_json::json;
use zenkey_fleet::report::*;
use zenkey_report_fixtures as fx;

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
/// One vocabulary: **snake_case** for the verdict and severity tags. The
/// blob plane's kebab-case pair, the documented second, left with `blob`
/// (#612, FJ9).
#[test]
fn every_enum_in_the_surface_names_its_wire_vocabulary() {
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
