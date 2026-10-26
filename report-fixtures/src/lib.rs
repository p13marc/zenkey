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
use zenkey_fleet::{RecordReport, ReplayReport, StorageInfo, TimelineReport};

pub const ORIGIN: &str = "h-3fa9c2d41b7e";

/// A storage list: one storage whose admin document omitted its strip
/// prefix, and one that states every field.
pub fn storage_list() -> StorageList {
    StorageList {
        storages: vec![
            StorageInfo {
                name: "events".into(),
                zid: "aabbccdd".into(),
                key_expr: Some("acme/zk2/*/*/*/events/**".into()),
                strip_prefix: None,
                volume: Some("memory".into()),
                // The untrimmed admin document. A fixture carries a realistic
                // one rather than `null`, because it is what a layout change
                // would arrive as.
                raw: serde_json::json!({
                    "key_expr": "acme/zk2/*/*/*/events/**",
                    "volume": {"id": "memory"},
                }),
            },
            StorageInfo {
                name: "plant".into(),
                zid: "aabbccdd".into(),
                key_expr: Some("acme/plant/**".into()),
                strip_prefix: Some("acme/plant".into()),
                volume: Some("fs".into()),
                raw: serde_json::json!({
                    "key_expr": "acme/plant/**",
                    "strip_prefix": "acme/plant",
                    "volume": {"id": "fs"},
                }),
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
            health: Asked::Asked(DoctorHealth {
                selector: "zk2/*/*/health.v1/state/**".into(),
                services: 2,
                clocks_synced: false,
            }),
            config: Asked::Asked(DoctorConfig {
                selector: "zk2/*/*/config.v1/state/**".into(),
                services: 1,
                readings: 2,
                clocks_synced: false,
            }),
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
            CheckReport::of(
                CheckId::HealthFailed,
                vec![finding(
                    CheckId::HealthFailed,
                    DoctorSeverity::Error,
                    "host-b/tc",
                    "its status is FAILED, fresh: present, and unable to do its primary job, as \
                     its owner says (health.v1 §2.1) — its reason: \"netns gone\"",
                )],
                vec![],
                "unused",
            ),
            clean(
                CheckId::HealthDegraded,
                "1 service(s) implementing health.v1, none at DEGRADED: each fresh status, and \
                 the checks it vouches for, read better",
            ),
            CheckReport::of(
                CheckId::HealthStale,
                vec![],
                vec![Unjudged {
                    subject: "host-a/tc".into(),
                    reason: "its status's freshness could not be read: this reader's clock is \
                             not trusted to the HLC delta against the stamping clock \
                             (clock_untrusted)"
                        .into(),
                }],
                "unused",
            ),
            clean(
                CheckId::HealthInconsistent,
                "1 service(s) implementing health.v1, each fresh status no better than its \
                 current checks",
            ),
            CheckReport::of(
                CheckId::HostidDuplicate,
                vec![finding(
                    CheckId::HostidDuplicate,
                    DoctorSeverity::Warning,
                    "h-bbd1aa1db10b/sysinfo",
                    "in both presence reads 2.0s apart, instances of h-bbd1aa1db10b/sysinfo on a \
                     system their descriptors declare minted state 2 session zids: the cause is \
                     undecided",
                )],
                vec![],
                "unused",
            ),
            // Deep, like state-stamp-foreign: not asked without --deep.
            CheckReport::not_asked(CheckId::PopulationOverBound),
            // config.v1 (#768, CC): a status off its view's projection in
            // both readings; a pending change in its deadline's grace; the
            // two that conclude from a fresh view unobservable without
            // --clocks-synced.
            CheckReport::of(
                CheckId::ConfigProjection,
                vec![finding(
                    CheckId::ConfigProjection,
                    DoctorSeverity::Error,
                    "lab/rf0",
                    "in both of two readings 2.0s apart, its status is not its view's \
                     projection: an owner keeps the status equal to the view's projection \
                     (config.v1 §2.19)",
                )],
                vec![],
                "unused",
            ),
            clean(
                CheckId::ConfigOverdue,
                "1 service(s) implementing config.v1, none listing a change pending past its \
                 deadline by its own clock; 3 present service(s) do not list config.v1, and are \
                 not asked",
            ),
            CheckReport::of(
                CheckId::ConfigUnpersisted,
                vec![],
                vec![Unjudged {
                    subject: "lab/rf0".into(),
                    reason: "no fresh view: the status carries no class, no writable and no \
                             startup value — the view this reader's clock is not trusted to the \
                             HLC delta against the stamping clock (unobservable) — the doctor \
                             measures no clock: pass --clocks-synced on the deployment's word \
                             that this host's clock and the owners' agree (freshness.v1 §2.6)"
                        .into(),
                }],
                "unused",
            ),
            CheckReport::not_asked(CheckId::ConfigDesired),
            // alarms.v1 (#800, AC): a phantom key, an error; the stale
            // question unobservable without --clocks-synced.
            CheckReport::of(
                CheckId::AlarmKey,
                vec![finding(
                    CheckId::AlarmKey,
                    DoctorSeverity::Error,
                    "zk2/lab/liar/alarms.v1/state/alarms/0000000000000001",
                    "the key is not its document's: the document derives a659f813308ad1da — a \
                     phantom of another derivation, of a renamed rule, or an owner that hashed a \
                     host-scoped label (alarms.v1 §2.2, §2.9)",
                )],
                vec![],
                "unused",
            ),
            CheckReport::of(
                CheckId::AlarmStale,
                vec![],
                vec![Unjudged {
                    subject: "zk2/lab/svc/alarms.v1/state/alarms/a659f813308ad1da".into(),
                    reason: "this reader's clock is not trusted to the HLC delta against the \
                             stamping clock (clock_untrusted); a reply is aged only on the \
                             operator's word that the clocks agree (--clocks-synced, \
                             freshness.v1 §2.6)"
                        .into(),
                }],
                "unused",
            ),
            // telemetry.v1 (#819, TC): an invalid declaration, a linter's
            // warning; the samples not asked without --deep.
            CheckReport::of(
                CheckId::TelemetryDeclaration,
                vec![finding(
                    CheckId::TelemetryDeclaration,
                    DoctorSeverity::Warning,
                    "meter.v1 sha256:5d1c stream/level",
                    "implemented by lab/meter; its declaration is invalid: the type cannot carry \
                     the declared kind (not_a_carrier) — no reader reads its members' kind, \
                     conformance or continuity (telemetry.v1 §2.2)",
                )],
                vec![],
                "unused",
            ),
            CheckReport::not_asked(CheckId::TelemetrySample),
            // desired.v1 (#840, DD): a refused act, a target not yet
            // converged with no trusted clock to say how long, and no
            // last-known.
            CheckReport::of(
                CheckId::DesiredRefused,
                vec![finding(
                    CheckId::DesiredRefused,
                    DoctorSeverity::Warning,
                    "lab/worker task",
                    "its last act, on 200/a1, was refused: code reach at \"/endpoint\" \
                     (desired.v1 §2.7)",
                )],
                vec![],
                "unused",
            ),
            CheckReport::of(
                CheckId::DesiredBehind,
                vec![],
                vec![Unjudged {
                    subject: "lab/w3 task".into(),
                    reason: "its report's observed 100/a1 is earlier than its author lab/author's \
                             current version, its put 300/a1, the same id: not yet — how long it \
                             has been so cannot be told from one reading without a trusted \
                             clock: pass --clocks-synced"
                        .into(),
                }],
                "unused",
            ),
            CheckReport::of(
                CheckId::DesiredLastKnown,
                vec![],
                vec![],
                "2 desired.v1 role(s), none running a document read from an archive",
            ),
            CheckReport::of(
                CheckId::RedundancyVacancy,
                vec![],
                vec![],
                "1 interface(s) redundancy.v1 governs, none without an incumbent in both reads \
                 2.0s apart",
            ),
        ],
        unobservable: None,
    }
}

/// `zenctl health` (#721, PF): every verdict of "is this service healthy?"
/// and every answer of §5's other questions, over a scope whose counts are
/// non-zero — an unhealthy service breaking §2.2 in both readings, a
/// healthy one, a stale one whose clock is ahead, one unobservable, and an
/// absent one, not asked, with its archive's status last-known.
pub fn health_report() -> HealthReport {
    let answer = |answer, reason: &str, says: &str| HealthAnswer {
        answer,
        reason: reason.into(),
        says: says.into(),
    };
    let reading = |verdict, reason: &str, level| HealthReadingVerdict {
        verdict,
        reason: reason.into(),
        level,
    };
    let stamp = |clock: &str| Stamp {
        time: "2026-10-10T08:00:00.000000000Z".into(),
        clock: clock.into(),
    };
    let row = |address: &str, verdict, reason: &str, level, says: &str| HealthRow {
        address: address.into(),
        verdict,
        reason: reason.into(),
        level,
        says: says.into(),
        readings: vec![
            reading(verdict, reason, level),
            reading(verdict, reason, level),
        ],
        agrees: answer(
            HealthAnswerToken::Yes,
            "ok",
            "its status is fresh at a level, and no current check is worse",
        ),
        clock_ahead: answer(
            HealthAnswerToken::No,
            "confirmed",
            "its status was confirmed after its last clock_ahead fault, or with none heard",
        ),
        status: None,
        checks: vec![],
        last_known: None,
        faults: 0,
        last_fault: None,
    };
    let mut liar = row(
        "lab/liar",
        HealthVerdict::Unhealthy,
        "inconsistent",
        Some(HealthLevel::Failed),
        "a check is worse than its status, which health.v1 §2.2 forbids",
    );
    liar.agrees = answer(
        HealthAnswerToken::No,
        "inconsistent",
        "a current check is worse than its fresh status in each of 2 readings: its owner \
         breaks health.v1 §2.2",
    );
    liar.status = Some(StatusSeen {
        level: LevelRead::Token("ok"),
        reason: "serving".into(),
        since_ns: 1_791_700_000_000_000_000,
        stamp: Some(stamp("a1")),
        via: StatusVia::Get,
    });
    liar.checks = vec![CheckSeen {
        check: "disk".into(),
        level: LevelRead::Token("failed"),
        detail: "full".into(),
    }];
    let mut ok = row(
        "lab/ok",
        HealthVerdict::Healthy,
        "ok",
        Some(HealthLevel::Ok),
        "its status is OK and confirmed, and no check is worse",
    );
    ok.status = Some(StatusSeen {
        level: LevelRead::Token("ok"),
        reason: "serving".into(),
        since_ns: 1_791_700_000_000_000_000,
        stamp: Some(stamp("a2")),
        via: StatusVia::Subscription,
    });
    let mut ahead = row(
        "lab/ahead",
        HealthVerdict::Stale,
        "beyond_horizon",
        None,
        "not confirmed within its horizon",
    );
    ahead.agrees = answer(
        HealthAnswerToken::Unobservable,
        "beyond_horizon",
        "not confirmed within its horizon",
    );
    ahead.clock_ahead = answer(
        HealthAnswerToken::Yes,
        "clock_ahead",
        "a clock_ahead fault was heard, and no confirmation of its status since",
    );
    ahead.faults = 2;
    ahead.last_fault = Some(FaultSeen {
        code: "clock_ahead".into(),
        class: "profile",
        level: LevelRead::Token("failed"),
        detail: "2000 ms ahead".into(),
    });
    let mut unknown = row(
        "lab/new",
        HealthVerdict::Unobservable,
        "unknown_level",
        None,
        "its status's level is unknown",
    );
    unknown.agrees = answer(
        HealthAnswerToken::Unobservable,
        "unknown_level",
        "its status's level is unknown",
    );
    unknown.status = Some(StatusSeen {
        level: LevelRead::Token("unspecified"),
        reason: "starting".into(),
        since_ns: 1_791_700_000_000_000_000,
        stamp: Some(stamp("a3")),
        via: StatusVia::Get,
    });
    let mut gone = row(
        "lab/svc",
        HealthVerdict::NotAsked,
        "absent",
        None,
        "it is absent: presence's word, not a level",
    );
    gone.agrees = answer(
        HealthAnswerToken::NotAsked,
        "absent",
        "it is absent: presence's word, not a level",
    );
    gone.clock_ahead = answer(
        HealthAnswerToken::NotAsked,
        "absent",
        "it is absent: presence's word",
    );
    gone.last_known = Some(LastKnownStatus {
        archive: "lab/archive".into(),
        level: LevelRead::Token("degraded"),
        reason: "upstream lost".into(),
        since_ns: 1_791_600_000_000_000_000,
        stamp: Some(stamp("a4")),
        confirmed: true,
    });
    HealthReport {
        namespace: "acme".into(),
        service: None,
        presence: Some(HealthPresence {
            selector: "zk2/*/*/@zk/**".into(),
            complete: true,
            cut_off_during: false,
            services: 4,
        }),
        face: None,
        window_s: Some(31.0),
        apart_s: 31.0,
        clock: ClockGround::Measured,
        asked: vec![
            "zk2/*/*/@zk/**".into(),
            "zk2/*/*/health.v1/state/**".into(),
            "zk2/*/*/health.v1/state/status".into(),
            "zk2/*/*/health.v1/stream/faults".into(),
        ],
        services: vec![liar, ok, ahead, unknown, gone],
        rollup: HealthRollup {
            worst: Some(HealthLevel::Failed),
            healthy: 1,
            unhealthy: 1,
            stale: 1,
            unobservable: 1,
            not_asked: 1,
        },
        unobservable: None,
    }
}

/// `zenctl telemetry` (#819, TC): §5's answers in every token — a Point
/// counter whose epoch rides in-band, a bare counter whose epoch is had by
/// presence and that went backwards within it, a gauge with no continuity
/// and a sample of the wrong type, and a resource whose declaration is
/// invalid — the decrease and the non-conforming sample the findings.
pub fn telemetry_report() -> TelemetryReport {
    use serde_json::json;
    use std::collections::BTreeMap;
    let declared = |carrier: &str,
                    kind: Option<&str>,
                    unit: Option<&str>,
                    epoch_in: Option<&str>,
                    says: &str| {
        TelemetryDeclaration {
            answer: DeclarationToken::Declared,
            reasons: vec![],
            carrier: Some(carrier.into()),
            kind: kind.map(Into::into),
            unit: unit.map(Into::into),
            buckets: None,
            semantic: None,
            epoch_in: epoch_in.map(Into::into),
            says: says.into(),
        }
    };
    let resource = |resource: &str, declaration, members| TelemetryResource {
        address: "lab/meter".into(),
        iface: "meter.v1".into(),
        fingerprint: "sha256:cd33b1f7".into(),
        resource: resource.into(),
        selector: format!("zk2/lab/meter/meter.v1/{resource}"),
        declaration,
        members,
    };
    let samples = |received, conforms, does_not: &[(&str, u64)]| TelemetrySamples {
        received,
        conforms,
        does_not: does_not.iter().map(|(_, n)| n).sum(),
        unobservable: 0,
        unobservable_reason: None,
        not_kept: 0,
        deletes: 0,
        problems: does_not
            .iter()
            .map(|(p, n)| ((*p).to_owned(), *n))
            .collect::<BTreeMap<_, _>>(),
        latest_does_not: does_not.iter().map(|(p, _)| (*p).to_owned()).collect(),
    };
    let continuity =
        |same, reset, decreased, not_asked, in_band, by_presence, assumed| TelemetryContinuity {
            answer: Judgement::NotAsked,
            pairs: same + reset + decreased + not_asked,
            pairs_listed: 0,
            same_epoch: same,
            reset,
            decreased,
            unobservable: 0,
            not_asked,
            in_band,
            by_presence,
            assumed,
        };
    #[allow(clippy::too_many_arguments)]
    let pair = |a: u64,
                b: u64,
                continuity,
                epoch,
                via: Option<&str>,
                rate: Option<f64>,
                rate_unit: Option<&str>,
                no_rate: Option<&str>,
                base: &str| TelemetryPair {
        earlier: json!(a),
        later: json!(b),
        continuity,
        epoch,
        via: via.map(Into::into),
        presence: PairPresenceToken::Observed,
        rate,
        rate_unit: rate_unit.map(Into::into),
        no_rate: no_rate.map(Into::into),
        base: base.into(),
        elapsed_s: 1.0,
    };
    let member = |resource: &str, kind: &str, unit: Option<&str>| TelemetryMember {
        key: format!("zk2/lab/meter/meter.v1/{resource}"),
        address: "lab/meter".into(),
        iface: "meter.v1".into(),
        resource: resource.into(),
        kind: Some(kind.into()),
        kind_from: Some("declared".into()),
        unit: unit.map(Into::into),
        unit_from: unit.map(|_| "declared".into()),
        samples: samples(0, 0, &[]),
        latest: None,
        continuity: continuity(0, 0, 0, 0, 0, 0, 0),
        pairs: vec![],
        rate: None,
    };
    let mut frames = member("stream/frames", "counter", Some("1"));
    frames.samples = samples(4, 3, &[("tag_disagrees", 1)]);
    frames.latest = Some(json!(3));
    frames.continuity = continuity(1, 1, 0, 0, 2, 0, 0);
    frames.pairs = vec![
        pair(
            10,
            20,
            ContinuityToken::SameEpoch,
            Some(EpochToken::Established),
            Some("in_band"),
            Some(10.0),
            Some("1/s"),
            None,
            "timestamp",
        ),
        pair(
            20,
            3,
            ContinuityToken::Reset,
            Some(EpochToken::Established),
            Some("in_band"),
            None,
            None,
            Some("reset"),
            "timestamp",
        ),
    ];
    frames.continuity.pairs_listed = frames.pairs.len() as u64;
    frames.continuity.answer = Judgement::NotEstablished {
        reason: "2 pair(s) whose epoch was established (2 in-band, 0 by presence), none \
                 decreased; 0 assumed, 1 reset"
            .into(),
    };
    frames.rate = Some(TelemetryRate {
        per_s: 10.0,
        unit: Some("1/s".into()),
    });
    let mut drops = member("stream/drops", "counter", None);
    drops.samples = samples(3, 3, &[]);
    drops.latest = Some(json!(7));
    drops.continuity = continuity(1, 0, 1, 0, 0, 2, 0);
    drops.pairs = vec![
        pair(
            5,
            10,
            ContinuityToken::SameEpoch,
            Some(EpochToken::Established),
            Some("presence"),
            Some(5.0),
            None,
            None,
            "arrival",
        ),
        pair(
            10,
            7,
            ContinuityToken::Decreased,
            Some(EpochToken::Established),
            Some("presence"),
            None,
            None,
            Some("decreased"),
            "arrival",
        ),
    ];
    drops.continuity.pairs_listed = drops.pairs.len() as u64;
    drops.continuity.answer = Judgement::Established;
    drops.rate = Some(TelemetryRate {
        per_s: 5.0,
        unit: None,
    });
    let mut level = member("stream/level", "gauge", Some("dBm"));
    level.samples = samples(3, 2, &[("wrong_type", 1)]);
    level.latest = Some(json!(-70.5));
    level.continuity = continuity(0, 0, 0, 1, 0, 0, 0);
    level.pairs = vec![TelemetryPair {
        earlier: json!(-71.0),
        later: json!(-70.5),
        continuity: ContinuityToken::NotAsked,
        epoch: None,
        via: None,
        presence: PairPresenceToken::Observed,
        rate: None,
        rate_unit: None,
        no_rate: Some("kind".into()),
        base: "arrival".into(),
        elapsed_s: 1.0,
    }];
    level.continuity.pairs_listed = 1;
    TelemetryReport {
        namespace: "acme".into(),
        selector: None,
        presence: Some(TelemetryPresence {
            selector: "zk2/*/*/@zk/**".into(),
            complete: true,
            cut_off_during: false,
            services: 1,
            basis: "complete".into(),
        }),
        across_face: false,
        window_s: 5.0,
        asked: vec![
            "zk2/*/*/@zk/**".into(),
            "zk2/*/*/@zk/instance/*".into(),
            "zk2/*/*/@zk/member/**".into(),
            "zk2/lab/meter/meter.v1/stream/drops".into(),
            "zk2/lab/meter/meter.v1/stream/frames".into(),
            "zk2/lab/meter/meter.v1/stream/level".into(),
        ],
        totals: TelemetryTotals {
            resources: 4,
            invalid: 1,
            members: 3,
            samples: 10,
            conforms: 8,
            does_not: 2,
            unobservable: 0,
            pairs: 5,
            decreased: 1,
            established: 4,
            assumed: 0,
            unanswered: 0,
        },
        resources: vec![
            resource(
                "stream/drops",
                declared(
                    "bare",
                    Some("counter"),
                    None,
                    None,
                    "declared: a bare counter, unit unknown; no in-band epoch: presence is its \
                     epoch's evidence",
                ),
                1,
            ),
            resource(
                "stream/frames",
                declared(
                    "point",
                    Some("counter"),
                    Some("1"),
                    Some("point"),
                    "declared: a Point declared counter, unit 1; its epoch in-band, in the Point",
                ),
                1,
            ),
            resource(
                "stream/level",
                declared(
                    "bare",
                    Some("gauge"),
                    Some("dBm"),
                    None,
                    "declared: a bare gauge, unit dBm; no epoch: a gauge, text, boolean or \
                     binary has none",
                ),
                1,
            ),
            resource(
                "stream/odd",
                TelemetryDeclaration {
                    answer: DeclarationToken::Invalid,
                    reasons: vec!["not_a_carrier".into()],
                    carrier: None,
                    kind: None,
                    unit: None,
                    buckets: None,
                    semantic: None,
                    epoch_in: None,
                    says: "invalid: the type cannot carry the declared kind (not_a_carrier) — its \
                           members' kind, conformance and continuity are unobservable \
                           (telemetry.v1 §2.2)"
                        .into(),
                },
                0,
            ),
        ],
        members: vec![drops, frames, level],
        lost: 0,
        refused: 0,
        unresolved: 0,
        skipped: vec![],
        unobservable: None,
    }
}

/// `zenctl alarms` (#800, AC): §5's answers in every token — a firing
/// alarm, a phantom key, an unconfirmed one past its horizon, a resolved
/// one, and an absent owner's alarm shown last-known — over owners each
/// counted by its verdict, the scope's answer the finding.
pub fn alarms_report() -> AlarmsReport {
    use zenkey_model::alarms::{Alarm, AlarmRef, Severity, State};
    let answer = |answer, reason: &str, says: &str| AlarmAnswer {
        answer,
        reason: reason.into(),
        says: says.into(),
    };
    let doc = |rule: &str, severity, state, summary: &str, labels: &[(&str, &str)]| Alarm {
        timestamp: 1_760_950_000_000,
        observed_at_ms: None,
        rule: rule.into(),
        severity,
        state,
        summary: summary.into(),
        category: None,
        labels: labels
            .iter()
            .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
            .collect(),
    };
    let a = doc(
        "link_down",
        Severity::Warning,
        State::Firing,
        "link to r2 down on eth0",
        &[("peer", "r2"), ("port", "eth0")],
    );
    let stamp = |clock: &str| {
        Some(Stamp {
            time: "2026-10-20T08:00:00.000000000Z".into(),
            clock: clock.into(),
        })
    };
    let r = |system: &str, service: &str, key: &str| AlarmRef {
        system: system.into(),
        service: service.into(),
        alert_key: key.into(),
    };
    let absent = || {
        answer(
            AlarmAnswerToken::NotAsked,
            "absent",
            "its owner is absent: presence's word, and its alarms are last-known",
        )
    };
    let row = |system: &str, service: &str, key: &str| AlarmRow {
        key: format!("zk2/{system}/{service}/alarms.v1/state/alarms/{key}"),
        alarm: r(system, service, key),
        firing: absent(),
        stale: absent(),
        rederives: absent(),
        severity: None,
        latest: None,
        problems: vec![],
        foreign_stamp: false,
        last_known: None,
    };
    let seen = |latest, document: Option<Alarm>, clock: &str, via| {
        Some(AlarmSeen {
            latest,
            document,
            invalid: None,
            stamp: stamp(clock),
            via,
        })
    };
    let firing = AlarmRow {
        firing: answer(
            AlarmAnswerToken::Yes,
            "firing",
            "its owner says it fires, and confirmed it within 900 s",
        ),
        stale: answer(
            AlarmAnswerToken::No,
            "fresh",
            "its owner confirmed it within 900 s",
        ),
        rederives: answer(
            AlarmAnswerToken::Yes,
            "strict",
            "the key is the document's, by the strict recipe",
        ),
        severity: Some(Severity::Warning),
        latest: seen(AlarmLatest::Firing, Some(a.clone()), "a1", AlarmVia::Get),
        ..row("lab", "a", "a659f813308ad1da")
    };
    let phantom = AlarmRow {
        firing: answer(
            AlarmAnswerToken::Yes,
            "firing",
            "its owner says it fires, and confirmed it within 900 s",
        ),
        stale: answer(
            AlarmAnswerToken::No,
            "fresh",
            "its owner confirmed it within 900 s",
        ),
        rederives: answer(
            AlarmAnswerToken::No,
            "mismatch",
            "the key is not the one its document derives",
        ),
        severity: Some(Severity::Warning),
        latest: seen(
            AlarmLatest::Firing,
            Some(a.clone()),
            "b2",
            AlarmVia::Subscription,
        ),
        ..row("lab", "liar", "0000000000000001")
    };
    let disk = doc(
        "disk_full",
        Severity::Critical,
        State::Firing,
        "/var is 97% full",
        &[("mount", "/var")],
    );
    let hung = AlarmRow {
        firing: answer(
            AlarmAnswerToken::Unobservable,
            "stale",
            "its owner has not confirmed it within 900 s",
        ),
        stale: answer(
            AlarmAnswerToken::Yes,
            "stale",
            "its owner has not confirmed it within 900 s",
        ),
        rederives: answer(
            AlarmAnswerToken::Yes,
            "strict",
            "the key is the document's, by the strict recipe",
        ),
        severity: Some(Severity::Critical),
        latest: seen(AlarmLatest::Firing, Some(disk), "c3", AlarmVia::Get),
        ..row(
            "lab",
            "hung",
            &zenkey_model::alarms::derive("disk_full", [("mount", "/var")]).key,
        )
    };
    let mut fan = doc(
        "fan_failed",
        Severity::Info,
        State::Resolved,
        "fan 2 back",
        &[("fan", "2")],
    );
    fan.category = Some("sensor_health".into());
    let resolved = AlarmRow {
        firing: answer(
            AlarmAnswerToken::No,
            "resolved",
            "its owner put it resolved",
        ),
        stale: answer(AlarmAnswerToken::NotAsked, "not_firing", "it is not firing"),
        rederives: answer(
            AlarmAnswerToken::Yes,
            "strict",
            "the key is the document's, by the strict recipe",
        ),
        severity: Some(Severity::Info),
        latest: seen(
            AlarmLatest::Resolved,
            Some(fan),
            "a1",
            AlarmVia::Subscription,
        ),
        ..row(
            "lab",
            "a",
            &zenkey_model::alarms::derive("fan_failed", [("fan", "2")]).key,
        )
    };
    let gone = AlarmRow {
        last_known: Some(LastKnownAlarm {
            archive: "lab/archive".into(),
            document: Some(a),
            deleted: false,
            stamp: stamp("d4"),
            confirmed: true,
        }),
        ..row("lab", "svc", "a659f813308ad1da")
    };
    let owner = |address: &str, known_by: Vec<KnownBy>, verdict, reason: &str, says: &str, n| {
        AlarmOwnerRow {
            address: address.into(),
            known_by,
            verdict,
            reason: reason.into(),
            says: says.into(),
            alarms: n,
            window_s: None,
        }
    };
    AlarmsReport {
        namespace: "acme".into(),
        selector: "zk2/lab/*/alarms.v1/state/alarms/*".into(),
        presence: Some(AlarmsPresence {
            selector: "zk2/lab/*/@zk/**".into(),
            complete: true,
            cut_off_during: false,
            services: 3,
            listing: 3,
        }),
        face: None,
        window_s: None,
        clock: ClockGround::DeploymentWord,
        asked: vec![
            "zk2/lab/*/@zk/**".into(),
            "zk2/lab/*/alarms.v1/state/alarms/*".into(),
        ],
        any_firing: AlarmScopeAnswer {
            at_least: Severity::Warning,
            answer: AlarmAnswerToken::Yes,
            reason: "any_firing".into(),
            says: "an alarm at or above the severity fires".into(),
            firing: 2,
            established: 2,
            unobservable: 1,
            not_asked: 1,
        },
        owners: vec![
            owner(
                "lab/a",
                vec![KnownBy::Presence],
                AlarmSetVerdict::Established,
                "answered",
                "its owner answered a complete GET",
                2,
            ),
            owner(
                "lab/hung",
                vec![KnownBy::Presence],
                AlarmSetVerdict::Established,
                "answered",
                "its owner answered a complete GET",
                1,
            ),
            owner(
                "lab/liar",
                vec![KnownBy::Presence],
                AlarmSetVerdict::Unobservable,
                "seed_incomplete",
                "the GET of its alarms ended at its timeout, or with an error",
                1,
            ),
            owner(
                "lab/svc",
                vec![KnownBy::Archive],
                AlarmSetVerdict::NotAsked,
                "absent",
                "its owner is absent: presence's word, and its alarms are last-known",
                0,
            ),
        ],
        alarms: {
            let mut v = vec![resolved, firing, hung, phantom, gone];
            v.sort_by(|a, b| a.key.cmp(&b.key));
            v
        },
        lost: 0,
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
                cut_off_during: false,
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
            cut_off_during: false,
            error: None,
        }),
        verdict: Judgement::Established,
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
                cut_off_during: false,
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
        instances: zenkey_fleet::report::Asked::NotAsked,
    }
}

/// [`topology`] with zk2's instances joined onto it (#705): one attached
/// to the verified router that lists its session, one no verified router
/// lists (an unverified answer claims it), one whose descriptor names no
/// zid — every attachment non-empty, so a renderer that merged two fails.
pub fn topology_with_instances() -> zenkey_fleet::TopologyReport {
    use zenkey_fleet::report::{AttachedTo, Attachment, InstanceAttachment, InstanceJoin};
    zenkey_fleet::TopologyReport {
        instances: zenkey_fleet::report::Asked::Asked(InstanceJoin {
            namespace: "acme".into(),
            selector: "zk2/*/*/@zk/**".into(),
            complete: true,
            cut_off_during: false,
            verified: vec!["aabbccdd".into()],
            unverified: vec![
                "`@/99887766/router`, answered by 12345678: its replier is not the router its key \
                 names"
                    .into(),
            ],
            unobservable: None,
            instances: vec![
                InstanceAttachment {
                    address: "host-a/tc".into(),
                    instance: "3fa9c2d41b7e0012".into(),
                    zid: Some("eeff0011".into()),
                    attachment: Attachment::Attached {
                        routers: vec![AttachedTo {
                            router: "aabbccdd".into(),
                            listed_as: "peer".into(),
                        }],
                    },
                },
                InstanceAttachment {
                    address: "host-b/tc".into(),
                    instance: "3fa9c2d41b7e0013".into(),
                    zid: Some("c0ffee".into()),
                    attachment: Attachment::Unattached {
                        reason: "no verified router (aabbccdd) lists zid c0ffee among its \
                                 sessions, compared by value"
                            .into(),
                    },
                },
                InstanceAttachment {
                    address: "ws-01/tcgui-frontend".into(),
                    instance: "3fa9c2d41b7e0014".into(),
                    zid: None,
                    attachment: Attachment::Unattributable {
                        reason: "its descriptor names no session zid (`meta.zid`)".into(),
                    },
                },
            ],
        }),
        ..topology()
    }
}

/// A zk2 deployment's storages as a plan (#393): two volumes, three
/// storages, an overlap warned on both sides, a refused `complete`, and one
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
    let default_gc = || GarbageCollection {
        period_s: 30,
        lifespan_s: 86_400,
        derivation:
            "zenoh's default 86400 s — no contract declares a tombstone lifetime to derive one from"
                .into(),
    };
    StoragePlan {
        base: "acme".into(),
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
                name: "events".into(),
                key_expr: "acme/zk2/*/*/*/events/**".into(),
                strip_prefix: "acme/zk2".into(),
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
                garbage_collection: default_gc(),
                retention: None,
                params: params(&[("dir", "events")]),
                warnings: vec![warn(
                    WarningKind::Overlap,
                    "overlaps links (acme/zk2/*/*/*/events/link/**): a GET under both selectors is answered by both",
                    "RFC 09 §2",
                )],
                derived: None,
            },
            PlannedStorage {
                name: "links".into(),
                key_expr: "acme/zk2/*/*/*/events/link/**".into(),
                strip_prefix: "acme/zk2".into(),
                volume: "fs".into(),
                history: HistoryMode::Latest,
                replication: None,
                complete: false,
                garbage_collection: GarbageCollection {
                    period_s: 30,
                    lifespan_s: 3600,
                    derivation: "declared gc_lifespan_s 3600".into(),
                },
                retention: None,
                params: params(&[("dir", "links")]),
                warnings: vec![
                    warn(
                        WarningKind::CompleteRefused,
                        "complete = true refused: it is not replicated — emitted as false",
                        "RFC 09 §2.2",
                    ),
                    warn(
                        WarningKind::Overlap,
                        "overlaps events (acme/zk2/*/*/*/events/**): a GET under both selectors is answered by both",
                        "RFC 09 §2",
                    ),
                ],
                derived: None,
            },
            PlannedStorage {
                name: "timeseries".into(),
                key_expr: "acme/zk2/*/*/*/stream/**".into(),
                strip_prefix: "acme/zk2".into(),
                volume: "influxdb".into(),
                history: HistoryMode::All,
                replication: None,
                complete: false,
                garbage_collection: default_gc(),
                retention: None,
                params: params(&[("db", "streams")]),
                warnings: vec![warn(
                    WarningKind::RetentionIsTheDatabases,
                    "retention is the database's policy, not zenoh config",
                    "RFC 09 §2.3",
                )],
                derived: None,
            },
        ],
        refusals: vec![Refusal {
            storage: Some("plant".into()),
            volume: None,
            key_expr: Some("acme/plant/**".into()),
            reason: "names volume \"rocks\", which [volumes] does not declare".into(),
            cite: "RFC 09 §2".into(),
        }],
        enrollment: zenkey_fleet::report::Asked::NotAsked,
    }
}

/// `why` stopped at a cause (#702): the namespace, the key and presence
/// healthy, the descriptor listing the resource `unavailable` with its
/// cause, and every rung past it not asked.
pub fn why_report_cause() -> WhyReport {
    WhyReport {
        target: "acme/zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0".into(),
        subject: WhySubject::Key,
        namespace: "acme".into(),
        asked: vec!["zk2/host-a/tc/@zk/**".into()],
        window_s: None,
        rungs: vec![
            WhyRung::healthy(
                RungId::Namespace,
                "§1.6",
                "the key sits under namespace \"acme\"",
            ),
            WhyRung::healthy(
                RungId::Key,
                "§1.1",
                "a stream key of host-a/tc, interface tc.netif.v1",
            ),
            WhyRung::healthy(
                RungId::Presence,
                "§8.1",
                "1 instance(s) of host-a/tc hold their token; 1 hold tc.netif.v1's interface token",
            ),
            WhyRung::cause(
                RungId::Descriptor,
                "§3.3",
                "stream/bandwidth/{ns}/{iface} is unavailable at host-a/tc@3fa9c2d41b7e0012 \
                 (config: no bandwidth probe)",
            ),
            WhyRung::not_asked(RungId::Contract, "§8.4"),
            WhyRung::not_asked(RungId::Answer, "§2.1"),
            WhyRung::not_asked(RungId::LastKnown, "§4.2 S6"),
        ],
        verdict: Judgement::Established,
        stopped_at: Some(RungId::Descriptor),
        value: None,
        last_known: None,
    }
}

/// `why` at the owner's silence (#702): every rung healthy up to the
/// answer, the S4 GET unanswered, and an archive's last-known value read
/// after it — last-known, never current.
pub fn why_report_silent() -> WhyReport {
    let key = "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0";
    WhyReport {
        target: format!("acme/{key}"),
        subject: WhySubject::Key,
        namespace: "acme".into(),
        asked: vec![
            "zk2/host-a/tc/@zk/**".into(),
            key.into(),
            "zk2/*/*/@zk/alive/archive.v1/**".into(),
        ],
        window_s: None,
        rungs: vec![
            WhyRung::healthy(
                RungId::Namespace,
                "§1.6",
                "the key sits under namespace \"acme\"",
            ),
            WhyRung::healthy(
                RungId::Key,
                "§1.1",
                "a state key of host-a/tc, interface tc.netif.v1",
            ),
            WhyRung::healthy(
                RungId::Presence,
                "§8.1",
                "1 instance(s) of host-a/tc hold their token",
            ),
            WhyRung::healthy(
                RungId::Descriptor,
                "§3.3",
                "implements tc.netif.v1 at 5d1c0a9b2e3f4a6b, exposing state/interfaces/{ns}/{iface}",
            ),
            WhyRung::healthy(
                RungId::Contract,
                "§8.4",
                "tc.netif.v1, retrieved from a holder and verified: the key is \
                 state/interfaces/{ns}/{iface}",
            ),
            WhyRung::unobservable(
                RungId::Answer,
                "§4.2 S4",
                "host-a/tc holds its tokens and sent no reply within 2s: silence is not a verdict",
            ),
            WhyRung::healthy(
                RungId::LastKnown,
                "§4.2 S6",
                "ground/archive holds its last-known value, not confirmed by alignment: \
                 last-known, never current (S6)",
            ),
        ],
        verdict: Judgement::Unobservable {
            reason: "answer: host-a/tc holds its tokens and sent no reply within 2s".into(),
        },
        stopped_at: Some(RungId::Answer),
        value: None,
        last_known: Some(WhyLastKnown {
            archive: "ground/archive".into(),
            key: key.into(),
            value: None,
            timestamp: Some(Stamp {
                time: "2026-10-09T10:00:00.000000000Z".into(),
                clock: "a1b2c3".into(),
            }),
            confirmed: false,
        }),
    }
}

/// `check conform` over one service (#703): every pole on some case — a
/// pass, a violation, a resource unobservable in its window, an operation
/// not called, a resource with no horizon not asked, a state over its bound
/// and a best-effort stream's population, whose window is never lossless
/// (§2.7, #735; 0.25, #740)
/// — so a renderer that merged two fails.
pub fn conform_report() -> ConformReport {
    let fp = format!("sha256:{}", "5d1c0a9b2e3f4a6b".repeat(4));
    ConformReport {
        address: "host-a/tc".into(),
        iface: "tc.netif.v1".into(),
        fingerprint: Some(fp.clone()),
        namespace: "acme".into(),
        window_s: 5.0,
        asked: vec![
            "zk2/host-a/tc/@zk/**".into(),
            "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/*/*".into(),
            "zk2/host-a/tc/tc.netif.v1/@op/diagnostics".into(),
        ],
        cases: vec![
            ConformCase::passed(
                CaseId::ContractServed,
                format!("tc.netif.v1 {fp}"),
                "a holder served the bundle the descriptor names, and it verified",
            ),
            ConformCase::passed(
                CaseId::ResourceServed,
                "stream/bandwidth/{ns}/{iface}",
                "12 sample(s) in the 5s window",
            ),
            ConformCase::failed(
                CaseId::PayloadType,
                "stream/bandwidth/{ns}/{iface}",
                "1 of 12 value(s) do not conform to the declared type; the first, \
                 zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0: /stats: not an object",
            ),
            ConformCase::unobservable(
                CaseId::ResourceServed,
                "state/namespaces",
                "nothing heard in the 5s window, and the state GET drew no reply",
            ),
            ConformCase::failed(
                CaseId::Operation,
                "@op/diagnostics",
                "host-a/tc holds its tokens, and the call drew neither a value nor an envelope \
                 within 1s: never silence (O3)",
            ),
            ConformCase::not_asked(
                CaseId::Operation,
                "@op/interfaces/{ns}/{iface}/set",
                "not idempotent: each call is a write, which this suite makes only under --i-know",
            ),
            ConformCase::failed(
                CaseId::Freshness,
                "state/interfaces/{ns}/{iface}",
                "1 of 2 member(s) stale against its horizon of 60 s: \
                 zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth1: not confirmed within \
                 its horizon",
            ),
            ConformCase::not_asked(
                CaseId::Freshness,
                "stream/bandwidth/{ns}/{iface}",
                "it declares no freshness.ttl_s",
            ),
            ConformCase::failed(
                CaseId::Budget,
                "state/interfaces/{ns}/{iface}",
                "3 member(s) answered with a value by the owner's GET, above its bound of 2 (its \
                 descriptor's; the contract's is 1024): an owner MUST NOT hold more live members \
                 than its bound (§2.7)",
            ),
            ConformCase::unobservable(
                CaseId::BudgetWindow,
                "stream/bandwidth/{ns}/{iface}",
                "2 member(s) heard within one hour in the 5.0s window, within its bound of 1024 \
                 (the contract's), and the window is not shown to have lost no delivery: \
                 best-effort: a dropped sample leaves no trace, so no window of it is lossless — \
                 a member it did not hear may be live (§2.7)",
            ),
        ],
        unobservable: None,
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
        source: zenkey_fleet::report::CheckSource::AdminSpace,
        asked: "@/*/router/status/plugins/storage_manager/storages/**".into(),
        planned: 3,
        observed: 3,
        findings: vec![
            finding(
                CheckKind::StripPrefixDiffers,
                "events",
                Some("aabbccdd"),
                Some("acme/zk2"),
                Some("acme"),
            ),
            finding(
                CheckKind::LifespanBelowMinimum,
                "events",
                Some("aabbccdd"),
                Some("86400"),
                Some("600"),
            ),
            finding(
                CheckKind::Missing,
                "timeseries",
                None,
                Some("acme/zk2/*/*/*/stream/**"),
                None,
            ),
            finding(
                CheckKind::Extra,
                "state",
                Some("aabbccdd"),
                None,
                Some("acme/zk2/*/*/*/state/**"),
            ),
        ],
        unjudged: vec![
            "links@aabbccdd: the admin document does not carry garbage_collection.lifespan".into(),
        ],
        judgement: Judgement::Established,
    }
}

/// The same check against an admin space that answered nothing.
pub fn storage_check_unobservable() -> StorageCheck {
    StorageCheck {
        base: "acme".into(),
        source: zenkey_fleet::report::CheckSource::AdminSpace,
        asked: "@/*/router/status/plugins/storage_manager/storages/**".into(),
        planned: 3,
        observed: 0,
        findings: vec![],
        unjudged: vec![],
        judgement: Judgement::Unobservable {
            reason: "the admin space answered no storages".into(),
        },
    }
}

/// A plan derived from an enrollment (#704): the tcgui pilot's audit
/// event as a union storage on the implicit memory volume, an interface
/// whose contract was not given, one without events, an archive never
/// planned, and an S4 refusal of the file's state storage — every list
/// non-empty, so a renderer that dropped one fails.
pub fn storage_plan_enrolled() -> StoragePlan {
    use zenkey_fleet::report::{DerivedEvent, EnrollmentDerivation};
    StoragePlan {
        base: "acme".into(),
        volumes: vec![PlannedVolume {
            id: "memory".into(),
            plugin: "memory".into(),
            history: HistoryMode::Latest,
            persistence: Some(Persistence::Volatile),
            params: std::collections::BTreeMap::new(),
            warnings: vec![PlanWarning {
                kind: WarningKind::ImplicitVolume,
                text: "no deployment file names an [events] volume: volatile".into(),
                cite: "RFC 09 §2.1".into(),
            }],
        }],
        storages: vec![PlannedStorage {
            name: "events-tc.netem.v1-applied".into(),
            key_expr: "acme/zk2/*/*/tc.netem.v1/events/applied/*".into(),
            strip_prefix: "acme/zk2".into(),
            volume: "memory".into(),
            history: HistoryMode::Latest,
            replication: None,
            complete: false,
            garbage_collection: GarbageCollection {
                period_s: 30,
                lifespan_s: 604_800,
                derivation: "the contract's retention for tc.netem.v1 events/applied, 7d \
                             (604800 s, spec §2.6)"
                    .into(),
            },
            retention: None,
            params: std::collections::BTreeMap::new(),
            warnings: vec![PlanWarning {
                kind: WarningKind::RetentionNotEnforced,
                text: "the contract's retention (7d) bounds a replay, not this storage".into(),
                cite: "spec §2.6".into(),
            }],
            derived: Some(DerivedEvent {
                iface: "tc.netem.v1".into(),
                resource: "events/applied".into(),
                retention_s: 604_800,
                providers: vec!["h-20609002f7b6/tc".into(), "h-3fa9c2d41b7e/tc".into()],
            }),
        }],
        refusals: vec![Refusal {
            storage: Some("latest".into()),
            volume: None,
            key_expr: Some("acme/zk2/*/*/*/state/**".into()),
            reason: "\"acme/zk2/*/*/*/state/**\" intersects owners' state keys".into(),
            cite: "spec §4.2 S4".into(),
        }],
        enrollment: zenkey_fleet::report::Asked::Asked(EnrollmentDerivation {
            contract_not_given: vec!["nav.v2".into()],
            without_events: vec!["tc.netif.v1".into()],
            archives: vec!["ground/archive".into()],
        }),
    }
}

/// `--check --against` a router file (#704): the derived storage's
/// lifespan below the retention, and a storage the file runs on owners'
/// state — both from the file, so no zid.
pub fn storage_check_file() -> StorageCheck {
    StorageCheck {
        base: "acme".into(),
        source: zenkey_fleet::report::CheckSource::File,
        asked: "router.json5".into(),
        planned: 1,
        observed: 2,
        findings: vec![
            CheckFinding {
                kind: CheckKind::LifespanBelowMinimum,
                storage: "events-tc.netem.v1-applied".into(),
                zid: None,
                planned: Some("604800".into()),
                observed: Some("86400".into()),
            },
            CheckFinding {
                kind: CheckKind::Extra,
                storage: "latest".into(),
                zid: None,
                planned: None,
                observed: Some("acme/zk2/*/*/*/state/**".into()),
            },
            CheckFinding {
                kind: CheckKind::OnOwnerState,
                storage: "latest".into(),
                zid: None,
                planned: None,
                observed: Some(
                    "acme/zk2/*/*/*/state/** (intersects `acme/zk2/*/*/*/state/**`, spec §4.2 S4)"
                        .into(),
                ),
            },
        ],
        unjudged: vec![],
        judgement: Judgement::Established,
    }
}

/// One key, taken by the events storage.
pub fn storage_explain() -> StorageExplain {
    StorageExplain {
        key: "acme/zk2/host-a/tc/tc.netif.v1/events/reset/01k0".into(),
        base: "acme".into(),
        takers: vec![Taker {
            storage: "events".into(),
            key_expr: "acme/zk2/*/*/*/events/**".into(),
            relation: TakerRelation::Includes,
            why: "its selector under namespace \"acme\": acme/zk2/*/*/*/events/** includes every key it names; stored under strip_prefix \"acme/zk2\" on volume fs (latest)".into(),
        }],
        refused_takers: vec![],
        none_reason: None,
    }
}

/// A key nothing takes, because a refused storage would have.
pub fn storage_explain_none() -> StorageExplain {
    StorageExplain {
        key: "acme/plant/line-1/temp".into(),
        base: "acme".into(),
        takers: vec![],
        refused_takers: vec!["plant".into()],
        none_reason: Some(
            "no planned storage's selector includes it; refused storage(s) plant would have".into(),
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

/// `link.v1`'s example (`examples/zk2/link/`, #769 LB): the lab router with
/// its radio face to the ground, as a client, the plan built through the
/// planner from the example's contract and face policy. The enrollment is
/// the example's, cut to one radio: `radio-a`, and the console the near
/// side's `local` and the far side's `ground` both run.
pub fn acl_plan_face() -> AclPlan {
    let contract = zenkey_model::contract::load_str(
        include_str!("../../examples/zk2/link/radio.v1.toml"),
        std::path::Path::new("."),
        None,
    )
    .contract
    .expect("the example contract loads");
    let mut contracts = zenkey_fleet::ContractSet::new();
    contracts.insert(zenkey_fleet::Revision::from_contract(
        contract,
        ContractSource::File,
    ));
    let console = |user: &str| PrincipalSpec {
        user: Some(user.into()),
        tools: vec!["console".into()],
        ..Default::default()
    };
    let enrollment = Enrollment {
        namespace: None,
        principal: vec![
            PrincipalSpec {
                user: Some("radio-a".into()),
                services: vec!["lab/radio-a".into()],
                ..Default::default()
            },
            console("local"),
            console("ground"),
        ],
        service: vec![ServiceSpec {
            address: "lab/radio-a".into(),
            implements: vec!["radio.v1".into()],
            ..Default::default()
        }],
        tool: vec![ToolSpec {
            name: "console".into(),
            bindings: [(
                "radio".to_string(),
                BindingSpec {
                    interface: Some("radio.v1".into()),
                    providers: vec!["lab/radio-a".into()],
                    ..Default::default()
                },
            )]
            .into(),
            calls: vec![CallsSpec {
                interface: "radio.v1".into(),
                providers: vec!["lab/radio-a".into()],
                operations: Some(vec!["reset".into()]),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    zenkey_fleet::plan_acl(
        &enrollment,
        &contracts,
        &zenkey_fleet::AclOptions {
            face_policy: Some(zenkey_model::link::parse(include_str!(
                "../../examples/zk2/link/radio.face.toml"
            ))),
            ..zenkey_fleet::AclOptions::default()
        },
    )
    .expect("the example's face policy plans")
}

/// `check face` over a hand-edited router config: the `zk` deny taken out
/// of the far principal's policy, and a wildcard downsampling rule in place
/// of the face's two (link.v1 §2.15), with the comparison naming both.
pub fn face_check() -> FaceCheck {
    use zenkey_model::link::{Code, Finding};
    FaceCheck {
        namespace: String::new(),
        against: "router.json5".into(),
        faces: vec![FaceBlock {
            face: "radio".into(),
            far: "ground".into(),
            subjects: vec!["ground".into()],
            denies: 7,
            rules: 1,
        }],
        findings: vec![
            Finding::new(Code::ZkNotDenied, Some("radio"), "@zk"),
            Finding::new(
                Code::WildcardDownsampling,
                Some("radio"),
                "zk2/lab/*/radio.v1/stream/air/dropped_total",
            ),
        ],
        windows: Asked::Asked(vec![FaceWindow {
            face: "radio".into(),
            service: "lab/radio-a".into(),
            window_s: 6,
            given: true,
            min_s: 6,
        }]),
        comparison: AclCheck {
            namespace: String::new(),
            against: "router.json5".into(),
            planned_rules: 18,
            observed_rules: 18,
            planned_subjects: 3,
            observed_subjects: 3,
            findings: vec![
                AclFinding {
                    kind: AclFindingKind::PolicyMissing,
                    id: "ground".into(),
                    planned: Some("rules [call-in:tool.console, face:radio:zk] subjects [ground]".into()),
                    observed: None,
                },
                AclFinding {
                    kind: AclFindingKind::DownsamplingDiffers,
                    id: "radio".into(),
                    planned: Some(
                        "link_protocols unixsock-stream egress put zk2/lab/radio-a/radio.v1/@stream/frames@1Hz zk2/lab/radio-a/radio.v1/stream/air/dropped_total@1Hz"
                            .into(),
                    ),
                    observed: Some(
                        "link_protocols unixsock-stream egress put zk2/lab/*/radio.v1/stream/air/dropped_total@1Hz"
                            .into(),
                    ),
                },
            ],
            judgement: Judgement::Established,
        },
        judgement: Judgement::Established,
    }
}

/// A clean `check face`: the generated block, as the plan wrote it;
/// windows not asked.
pub fn face_check_clean() -> FaceCheck {
    let mut c = face_check();
    c.faces[0].denies = 8;
    c.faces[0].rules = 2;
    c.findings.clear();
    c.windows = Asked::NotAsked;
    c.comparison.findings.clear();
    c.comparison.judgement = Judgement::NotEstablished {
        reason: "router.json5 carries the plan whole: 18 rule(s), 3 subject(s), 3 polic(y/ies), enabled, default deny, 1 downsampling item(s)".into(),
    };
    c.judgement = Judgement::NotEstablished {
        reason: "router.json5 guards 1 face(s) as planned".into(),
    };
    c
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
            cut_off_during: false,
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
            cut_off_during: false,
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
                timestamp: Some("7f3b2a1c00000001/cd34".into()),
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
    zenkey_fleet::diff_snapshots(
        &snapshot(),
        &snapshot_staging(),
        zenkey_fleet::DiffOpts::default(),
    )
}

/// [`snapshot`]'s deployment under the `staging` namespace an hour on: the
/// same state, other stamps.
pub fn snapshot_staging() -> Snapshot {
    let mut b = snapshot();
    b.header = zsnap_header("staging", "2026-10-09T01:00:00Z", 0.9, 5, 1);
    for row in &mut b.rows {
        row.key = row.key.replacen("acme/", "staging/", 1);
        row.timestamp = row.timestamp.as_ref().map(|t| t.replace("/ab12", "/ef56"));
    }
    b
}

/// [`snapshot`] against itself: identical on every facet.
pub fn snapshot_diff_identity() -> SnapshotDiff {
    zenkey_fleet::diff_snapshots(&snapshot(), &snapshot(), zenkey_fleet::DiffOpts::default())
}

// --- config.v1 (#768, CC) ---------------------------------------------------

/// `lab/rf0`'s view as `scenarios.md` §1 leaves it: `duty` changed at
/// runtime from its startup 10000, `radio` from its file, `pin` sensitive,
/// `queue` outside the allowlist, a reach change on `radio` pending.
pub fn config_view() -> zenkey_model::config::ConfigView {
    serde_json::from_value(serde_json::json!({
        "service": "lab/rf0", "instance": "3fa9c2d41b7e0012", "revision": 1,
        "confirm_floor_s": 10,
        "pending": {"token": "3fa9c2d41b7e0012-2", "groups": ["radio"],
                    "deadline": "2026-10-18T12:02:00.000Z", "applied": false},
        "last_change": {"token": "3fa9c2d41b7e0012-1", "groups": ["duty"]},
        "groups": [
            {"name": "duty", "class": "hot", "description": "the transmit duty cycle",
             "writable": true,
             "parameters": [{"name": "fraction_ppm", "kind": "integer", "min": 0,
                             "max": 1000000, "unit": "ppm", "description": "the share of airtime",
                             "value": 20000, "source": "runtime", "startup": 10000}]},
            {"name": "radio", "class": "reach", "description": "the radio channel",
             "writable": true,
             "parameters": [{"name": "frequency_hz", "kind": "integer", "unit": "Hz",
                             "description": "the carrier frequency", "value": 868100000,
                             "source": "file"}]},
            {"name": "sim", "class": "hot", "description": "the SIM", "writable": true,
             "parameters": [{"name": "pin", "kind": "text", "description": "the SIM's PIN",
                             "sensitive": true, "source": "file"}]},
            {"name": "queue", "class": "hot", "description": "the transmit queue",
             "writable": false,
             "parameters": [{"name": "tx_queue_len", "kind": "integer", "unit": "packets",
                             "description": "the queue's length", "value": 100,
                             "source": "default"}]}
        ]
    }))
    .expect("a ConfigView")
}

fn config_answer(answer: ConfigAnswerToken, reason: &str, says: &str) -> ConfigAnswer {
    ConfigAnswer::new(answer, reason, says)
}

/// A document confirmed within its horizon.
fn fresh() -> ConfigFreshness {
    ConfigFreshness {
        verdict: FreshnessToken::Fresh,
        reason: "within_horizon".into(),
        says: "confirmed within its horizon".into(),
    }
}

/// `zenctl config get lab/rf0 --token … --clocks-synced`: the view and the
/// status fresh, a reach change pending, the change asked about not landed
/// yet, every group's and parameter's answer — a sensitive value
/// unobservable, a runtime change that would not survive a restart — and
/// one change event read.
pub fn config_report() -> ConfigReport {
    use ConfigAnswerToken as A;
    let view = config_view();
    let status = zenkey_model::config::ConfigStatus::of(&view);
    let stamp = Stamp {
        time: "2026-10-18T12:00:05.000000000Z".into(),
        clock: "751033f1fbbb816bc6f7bad180995fa2".into(),
    };
    ConfigReport {
        namespace: "acme".into(),
        service: "lab/rf0".into(),
        premise: ConfigPremise::Listed,
        presence: Some(ConfigPresenceRead {
            selector: "zk2/lab/rf0/@zk/**".into(),
            complete: true,
            cut_off_during: false,
            instances: vec!["3fa9c2d41b7e0012".into()],
        }),
        window_s: None,
        clock: ClockGround::DeploymentWord,
        asked: vec![
            "zk2/lab/rf0/@zk/**".into(),
            "zk2/lab/rf0/config.v1/state/**".into(),
            "zk2/lab/rf0/config.v1/events/change/*".into(),
        ],
        pending_change: view.pending.clone(),
        view: Asked::Asked(ConfigDocument {
            value: Some(view),
            undecodable: None,
            via: Some(ConfigVia::Get),
            stamp: Some(stamp.clone()),
            freshness: fresh(),
        }),
        status: ConfigDocument {
            value: Some(status),
            undecodable: None,
            via: Some(ConfigVia::Get),
            stamp: Some(stamp),
            freshness: fresh(),
        },
        projection: config_answer(
            A::Yes,
            "projection",
            "the status is the view's projection (config.v1 §2.19)",
        ),
        pending: config_answer(
            A::Yes,
            "pending",
            "change 3fa9c2d41b7e0012-2 is pending on radio, its deadline \
             2026-10-18T12:02:00.000Z (the owner's clock), not every group applied yet — by the \
             fresh view",
        ),
        change: Some(ChangeAnswers {
            token: "3fa9c2d41b7e0012-2".into(),
            instance: Some("3fa9c2d41b7e0012".into()),
            landed: config_answer(
                A::NotYet,
                "still_pending",
                "it is still pending, its deadline 2026-10-18T12:02:00.000Z (the owner's \
                 clock): confirm it, or it is undone then",
            ),
            undo_clean: config_answer(
                A::NotAsked,
                "not_ended",
                "it is still pending: no undo has run yet",
            ),
            event: None,
        }),
        writable: vec![
            GroupAnswer {
                group: "duty".into(),
                answer: config_answer(A::Yes, "hot", "duty is hot and writable"),
            },
            GroupAnswer {
                group: "queue".into(),
                answer: config_answer(
                    A::No,
                    "outside_allowlist",
                    "queue is outside this service's startup allowlist (config.v1 §2.16)",
                ),
            },
        ],
        survives_restart: vec![
            ParameterAnswer {
                group: "duty".into(),
                parameter: "fraction_ppm".into(),
                answer: config_answer(
                    A::No,
                    "startup_differs",
                    "it runs 20000, and a restart would run 10000: a divergence an operator \
                     may want persisted or undone (config.v1 §2.13)",
                ),
            },
            ParameterAnswer {
                group: "sim".into(),
                parameter: "pin".into(),
                answer: config_answer(
                    A::Unobservable,
                    "sensitive",
                    "write-only: its startup value is never shown (config.v1 §2.15)",
                ),
            },
        ],
        converged: DesiredAnswer {
            revision: None,
            answer: config_answer(
                A::NotAsked,
                "no_desired",
                "no desired member: the owner converges no desired state (config.v1 §2.18)",
            ),
        },
        events: vec![
            serde_json::from_value(serde_json::json!({
                "service": "lab/rf0", "instance": "3fa9c2d41b7e0012", "revision": 1,
                "token": "3fa9c2d41b7e0012-1", "outcome": "applied",
                "edits": [{"group": "duty", "parameter": "fraction_ppm",
                           "old": 10000, "new": 20000}],
                "actor": "ops", "request_id": "r-1"
            }))
            .expect("a ConfigChangeEvent"),
        ],
        events_undecodable: 0,
        unobservable: None,
    }
}

/// `zenctl config get lab/rf0 --status --across-face`: the status alone,
/// read by GET, with no trusted clock: every answer unobservable, never no.
pub fn config_status_report() -> ConfigReport {
    use ConfigAnswerToken as A;
    let mut status = zenkey_model::config::ConfigStatus::of(&config_view());
    status.pending = None;
    let untrusted = ConfigFreshness {
        verdict: FreshnessToken::Unobservable,
        reason: "clock_untrusted".into(),
        says: "this reader's clock is not trusted to the HLC delta against the stamping clock"
            .into(),
    };
    let none_fresh = "neither the view nor the status is fresh — the view was not asked \
                      (--status); the status this reader's clock is not trusted to the HLC \
                      delta against the stamping clock (unobservable)";
    ConfigReport {
        namespace: String::new(),
        service: "lab/rf0".into(),
        premise: ConfigPremise::AcrossFace,
        presence: None,
        window_s: None,
        clock: ClockGround::None,
        asked: vec!["zk2/lab/rf0/config.v1/state/status".into()],
        view: Asked::NotAsked,
        status: ConfigDocument {
            value: Some(status),
            undecodable: None,
            via: Some(ConfigVia::Get),
            stamp: Some(Stamp {
                time: "2026-10-18T12:00:05.000000000Z".into(),
                clock: "751033f1fbbb816bc6f7bad180995fa2".into(),
            }),
            freshness: untrusted,
        },
        projection: config_answer(
            A::NotAsked,
            "view_not_asked",
            "only the status was read (--status)",
        ),
        pending: config_answer(A::Unobservable, "no_fresh_document", none_fresh),
        pending_change: None,
        change: None,
        writable: vec![],
        survives_restart: vec![],
        converged: DesiredAnswer {
            revision: None,
            answer: config_answer(A::Unobservable, "no_fresh_document", none_fresh),
        },
        events: vec![],
        events_undecodable: 0,
        unobservable: None,
    }
}

fn config_act(
    operation: &str,
    request: serde_json::Value,
    answer: ConfigActAnswer,
) -> ConfigActReport {
    ConfigActReport {
        namespace: "acme".into(),
        service: "lab/rf0".into(),
        operation: operation.into(),
        key: format!("zk2/lab/rf0/config.v1/@op/{operation}"),
        request,
        actor: Some("ops".into()),
        request_id: Some("r-2".into()),
        checked: None,
        across_face: false,
        timeout_s: 2.0,
        answer,
    }
}

/// `zenctl config set lab/rf0 radio frequency_hz=869525000 --confirm-s 120
/// --i-know`: checked against the view, answered `pending` before the apply.
pub fn config_act_pending() -> ConfigActReport {
    ConfigActReport {
        checked: Some(ConfigChecked {
            read: ConfigCheckedRead::View,
            typed_by: ConfigTyping::Schema,
            instance: Some("3fa9c2d41b7e0012".into()),
            revision: Some(1),
            class: Some(zenkey_model::config::Class::Reach),
        }),
        ..config_act(
            "set/radio",
            serde_json::json!({"confirm_s": 120, "values": {"frequency_hz": 869525000}}),
            ConfigActAnswer::Pending {
                pending: zenkey_model::config::PendingReply {
                    token: "3fa9c2d41b7e0012-2".into(),
                    apply_at: "2026-10-18T12:00:01.000Z".into(),
                    deadline: "2026-10-18T12:02:00.000Z".into(),
                },
            },
        )
    }
}

/// `zenctl config set lab/rf0 sim pin=@pin.txt`: the value redacted in the
/// request shown, and the device's refusal, its words left out (§2.15).
pub fn config_act_refused() -> ConfigActReport {
    config_act(
        "set/sim",
        serde_json::json!({"values": {"pin": "<redacted>"}}),
        ConfigActAnswer::Refused {
            refusal: ConfigRefusal {
                code: "app".into(),
                message: "the device refused group \"sim\"".into(),
                cause: None,
                error: Some(zenkey_model::config::ConfigError {
                    code: zenkey_model::config::ErrorCode::DeviceRefused,
                    detail: None,
                }),
                detail_undecodable: None,
            },
        },
    )
}

/// `zenctl config confirm lab/rf0 … --status --across-face`: the compact
/// status after the act.
pub fn config_act_status() -> ConfigActReport {
    let mut status = zenkey_model::config::ConfigStatus::of(&config_view());
    status.pending = None;
    status.last_change = Some(zenkey_model::config::LastChange {
        token: "3fa9c2d41b7e0012-2".into(),
        groups: vec!["radio".into()],
    });
    ConfigActReport {
        across_face: true,
        ..config_act(
            "confirm",
            serde_json::json!({"answer": "status", "token": "3fa9c2d41b7e0012-2"}),
            ConfigActAnswer::Status {
                status: Box::new(status),
            },
        )
    }
}

/// `zenctl config persist lab/rf0 …` that drew nothing: silence, attributed
/// through presence, never a failure (core O5).
pub fn config_act_silent() -> ConfigActReport {
    config_act(
        "set/duty",
        serde_json::json!({"values": {"fraction_ppm": 30000}}),
        ConfigActAnswer::Silent {
            silence: SilenceView {
                attempts: 1,
                transport: Some("zenoh/string: Timeout".into()),
                presence: PresenceAttribution::Present,
            },
        },
    )
}

/// `zenctl desired` (#840, DD): five targets' `task` roles, judged by the
/// engine from values a reading would have read, so every word is the
/// judge's — one converged, one refused, one behind its author's version
/// past the window by a trusted clock, one whose author is absent beside an
/// archive's last-known, and one the kill switch disarms.
pub fn desired_report() -> DesiredReport {
    use std::time::{Duration, Instant, SystemTime};
    use zenkey_fleet::model::desired::{
        Archived, DesiredRead, ReportReply, ReportsGet, RoleRead, RoleResolution, SliceFrom,
        SliceGet, SliceReply, TargetRead,
    };
    use zenkey_model::desired::{self as dm, Listing, Seen};
    use zenkey_model::seed::Stamp;
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_761_264_000);
    let st = |t: u64| Stamp::parse(t, "a1").expect("a stamp");
    let target =
        |name: &str, seen_author: Seen, report: &str, current: Option<(u64, SystemTime)>| {
            let addr: zenkey_model::grammar::Addr =
                format!("lab/{name}").parse().expect("an address");
            let slice = format!("zk2/lab/author/tasking.v1/state/tasks/lab/{name}");
            let rp = format!("zk2/lab/{name}/worker.v1/state/desired/task");
            let mut res = RoleResolution::bare("task", Some(Listing::Listed { bound: true }));
            res.interface = Some("tasking.v1".parse().expect("an interface"));
            res.author = Some("lab/author".parse().expect("an address"));
            res.resource = Some("state/tasks/{system}/{service}".into());
            res.slice = vec![slice.clone()];
            res.slice_from = Some(SliceFrom::AuthorContract);
            let report = report.replace("$D", &slice);
            TargetRead {
                addr: addr.clone(),
                seen: Seen::Present,
                seen_why: None,
                word: None,
                descriptor: Ok(Some(
                    serde_json::from_value(serde_json::json!({
                        "format": "zk2-descriptor/0.1", "service": addr.to_string(),
                        "instance": "0123456789abcdef", "interfaces": []
                    }))
                    .expect("a descriptor"),
                )),
                roles: vec![RoleRead {
                    resolution: res,
                    author_seen: Some((seen_author, None)),
                    current: current.map(|(t, at)| {
                        Ok(SliceGet {
                            selector: slice.clone(),
                            complete: true,
                            replies: vec![SliceReply {
                                key: slice.clone(),
                                delete: false,
                                stamp: Some((st(t), at)),
                            }],
                            at: now,
                        })
                    }),
                    archived: (seen_author == Seen::Absent).then(|| {
                        Ok(vec![Archived {
                            archive: "lab/archive".parse().expect("an address"),
                            deleted: false,
                            stamp: Some(st(100)),
                            revision: Some(1),
                            confirmed: true,
                        }])
                    }),
                }],
                reports: Some(Ok(ReportsGet {
                    selectors: vec![rp.clone()],
                    complete: true,
                    replies: vec![ReportReply {
                        key: rp,
                        read: dm::decode(report.as_bytes()),
                        stamp: None,
                        horizon: zenkey_model::freshness::Horizon::Undeclared,
                    }],
                    at: now,
                })),
            }
        };
    let converged = r#"{"key":"$D","source":"desired","in_force":{"stamp":"100/a1","revision":1},"observed":{"stamp":"100/a1","revision":1},"outcome":"converged"}"#;
    let refused = r#"{"key":"$D","source":"desired","in_force":{"stamp":"100/a1","revision":1},"observed":{"stamp":"200/a1","revision":2},"outcome":"refused","refusal":{"code":"reach","pointer":"/endpoint","detail":"the worker's endpoint is what it reaches the bus by"}}"#;
    let last_known = r#"{"key":"$D","source":"desired","in_force":{"stamp":"100/a1","revision":1},"observed":{"stamp":"100/a1","revision":1},"outcome":"converged","last_known":true}"#;
    let mut unbound = target("spare", Seen::Present, r#"{"source":"baseline"}"#, None);
    let r = &mut unbound.roles[0].resolution;
    r.listing = Some(Listing::Listed { bound: false });
    r.author = None;
    r.slice.clear();
    r.slice_from = None;
    unbound.roles[0].author_seen = None;
    let read = DesiredRead {
        began: Instant::now(),
        at: now,
        cut_off: None,
        presence: None,
        targets: vec![
            target("worker", Seen::Present, converged, Some((100, now))),
            target("w2", Seen::Present, refused, Some((200, now))),
            target(
                "w3",
                Seen::Present,
                converged,
                Some((300, now - Duration::from_secs(60))),
            ),
            target("w4", Seen::Absent, last_known, None),
            unbound,
        ],
        asked: vec![
            "zk2/*/*/@zk/**".into(),
            "zk2/lab/worker/worker.v1/state/desired/task".into(),
            "zk2/lab/author/tasking.v1/state/tasks/lab/worker".into(),
        ],
    };
    let mut spec = zenkey_fleet::DesiredSpec::new(Duration::from_secs(1));
    spec.window = Some(Duration::from_secs(10));
    spec.clocks_synced = true;
    let mut behind = zenkey_fleet::DesiredBehindTracker::default();
    let mut report = zenkey_fleet::judge_desired("acme", None, &spec, &read, &mut behind, 1);
    report.presence = Some(DesiredPresence {
        selector: "zk2/*/*/@zk/**".into(),
        complete: true,
        cut_off_during: false,
        services: 7,
    });
    report
}

/// `zenctl redundancy` (#841, RC): four addresses of `ledger.v1`, which
/// `redundancy.v1` governs (election `claim`), judged by the engine from
/// two reads a grace apart, so every word is the model's — one incumbent
/// with its standby, a split whose second holder is marked standby, a
/// lasting vacancy between two standbys, and an incumbent beside an
/// unmarked twin.
pub fn redundancy_report() -> RedundancyReport {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::time::Duration;
    use zenkey_fleet::model::catalog::{ContractState, DescriptorRead, Observed, Revision};
    let text = "[interface]\nname = \"ledger\"\nmajor = 1\nminor = 0\nuses = [\"redundancy.v1\"]\n\
                [defaults]\nannotations = { \"redundancy.election\" = \"claim\" }\n\
                [resources.balance]\nkind = \"state\"\ntype = { raw = \"text/plain\" }\n\
                [resources.append]\nkind = \"operation\"\nrequest = { raw = \"text/plain\" }\n\
                response = { raw = \"text/plain\" }\n";
    let l = zenkey_model::contract::load_str(text, std::path::Path::new("."), None);
    let rev = Arc::new(Revision::from_contract(
        l.contract.unwrap_or_else(|| panic!("{}", l.report)),
        ContractSource::Bus,
    ));
    let fp = rev.fingerprint().clone();
    let a = "3fa9c2d41b7e0012";
    let b = "3fa9c2d41b7e0013";
    // (service, instance, holds, marked standby)
    let shape: [(&str, &str, bool, bool); 8] = [
        ("lab/ledger", a, true, false),
        ("lab/ledger", b, false, true),
        ("lab/split", a, true, false),
        ("lab/split", b, true, true),
        ("lab/vacant", a, false, true),
        ("lab/vacant", b, false, true),
        ("lab/twin", a, true, false),
        ("lab/twin", b, false, false),
    ];
    let read = || {
        let mut keys = Vec::new();
        let mut descriptors = BTreeMap::new();
        for (svc, id, holds, standby) in shape {
            keys.push(format!("zk2/{svc}/@zk/instance/{id}"));
            if holds {
                keys.push(format!(
                    "zk2/{svc}/@zk/alive/ledger.v1/{id}/{}",
                    fp.hex().fp16()
                ));
            }
            let d = serde_json::from_value(serde_json::json!({
                "format": "zk2-descriptor/0.1", "service": svc, "instance": id,
                "interfaces": [{"iface": "ledger.v1", "contract": fp.to_string(), "minor": 0,
                                "standby": standby}],
                "meta": {"zid": format!("{id}0")},
            }))
            .expect("a descriptor");
            descriptors.insert(
                (svc.parse().expect("an address"), id.parse().expect("an id")),
                DescriptorRead::Served(Box::new(d)),
            );
        }
        let mut o = Observed::from_keys("zk2/*/*/@zk/**", &keys, true);
        o.descriptors = Some(descriptors);
        o
    };
    let mut contracts = BTreeMap::new();
    contracts.insert(
        ("ledger.v1".parse().expect("an interface"), fp.clone()),
        Ok(ContractState::Held(Arc::clone(&rev))),
    );
    let obs = zenkey_fleet::RedundancyObservation {
        cut_off: None,
        first: Some(Ok(read())),
        second: Some(Ok(read())),
        apart: Duration::from_secs(2),
        contracts,
        asked: vec!["zk2/*/*/@zk/**".into()],
    };
    let spec = zenkey_fleet::RedundancySpec::new(Duration::from_secs(1));
    zenkey_fleet::judge_redundancy("acme", None, &spec, &obs)
}
