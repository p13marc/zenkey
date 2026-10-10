//! How each report is drawn (#201, layer 1).
//!
//! Pure: no process, no bus, no network. A report value goes in, two strings
//! come out, and both are pinned — the table because a regression there is
//! otherwise invisible to CI, and the ndjson because it is the half users
//! script against.
//!
//! ## The values come from `zenkey-report-fixtures`
//!
//! The same constructors `zenkey-fleet/tests/report_contract.rs` uses. That
//! corpus pins what a report *serializes to*; this one pins how it is *drawn*.
//! Sharing the values is what keeps the two from drifting about what a report
//! is: add a field, and one function breaks, and both corpora re-run against
//! the same thing.
//!
//! ## Two kinds of assertion, and the split is deliberate
//!
//! **Snapshots for layout.** `snapbox`'s inline `str![[…]]`, accepted with
//! `just snapshots`. Captured at a fixed width so they are terminal-
//! independent. A snapshot review is the tax #201 accepted, and layout is what
//! it should be paid for.
//!
//! **Claims for the honesty invariants**, in `emit.rs` and below — the things
//! that must never be waved through by someone hitting "accept" on a diff:
//! that `—` is not an empty cell, that an unasked field is absent rather than
//! null, that a bounded report says what it dropped.

use snapbox::assert_data_eq;
use snapbox::str;
use zenctl::render::{Format, Render, Width, to_string};
use zenkey_report_fixtures as fx;

/// Unbounded, which is both terminal-independent *and* what a real piped
/// table gets: `term_width()` returns `Unbounded` when stdout is not a tty,
/// deliberately, so that `--format table | cat` is byte-stable. A snapshot
/// captured at a guessed column count would be a fact about the guess.
///
/// The squeeze has its own tests in `table.rs`, against a synthetic report,
/// where a fixture's real column widths cannot make them incidental.
const W: Width = Width::Unbounded;

fn table<R: Render>(r: &R) -> String {
    to_string(r, Format::Table, W).expect("render").0
}

fn notes<R: Render>(r: &R) -> String {
    to_string(r, Format::Table, W).expect("render").1
}

fn ndjson<R: Render>(r: &R) -> String {
    to_string(r, Format::Ndjson, W).expect("render").0
}

/// One row kind on the stream, tagged like every family's, and the field an
/// admin document omitted drawn as `—`, never as an agreeing value.
#[test]
fn a_storage_list_tags_its_rows() {
    let out = ndjson(&fx::storage_list());
    let kinds: Vec<String> = out
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v.get("row").and_then(|r| r.as_str()).map(str::to_string))
        .collect();
    assert_eq!(kinds, ["storage", "storage"]);
    assert_data_eq!(
        table(&fx::storage_list()),
        str![[r#"
configured storages:

  events  @aabbccdd  acme/zk2/*/*/*/events/**
    strip —  ·  volume memory
  plant   @aabbccdd  acme/plant/**
    strip acme/plant  ·  volume fs

"#]]
    );
}

/// zk2's doctor (#612, FJ6): one row per check, every verdict pole spelled
/// apart in every medium — a word and a mark in the table, an `answer` in
/// the row — and the scope it read reaching the machine formats as notes.
/// The fixture has every pole and every count non-zero (tooling guide §7):
/// a renderer that summed two counts, or drew not asked as clean, fails here.
#[test]
fn a_doctor_run_spells_every_verdict_pole_apart_in_every_medium() {
    let report = fx::doctor_report();
    assert_data_eq!(
        table(&report),
        str![[r#"
✗  split-brain (§6)                      finding — 1 subject(s)
    ✗ error: host-a/tc tc.netif.v1 — 2 instances hold its interface token in two presence reads 2.0s apart (3fa9c2d41b7e0012, 3fa9c2d41b7e0013), and at least two expose an exclusive resource
⚠  binding-unsatisfied (§3.2 R5)         finding — 1 subject(s)
    ⚠ warning: ws-01/tcgui-frontend scenario — its bindings (*/tc) select no provider of tc.scenario.v1 visible to this reader
?  contract-drift (§9.8)                 unobservable — 1 subject(s) unjudged
    ? unjudged tc.netem.v1 eeeeeeeeeeeeeeee 5d1c0a9b2e3f4a6b: not classified: tc.netem.v1 sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee is unavailable
✗  contract-unavailable (§8.4)           finding — 1 subject(s)
    ✗ error: tc.netem.v1 sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee — named by host-b/tc@3fa9c2d41b7e0014; no holder served a bundle that verified (no reply)
✓  descriptor-invalid (§3.3)             clean — 3 descriptor(s) pass the descriptor check against the contracts they name
✓  token-missing (§8.1)                  clean — 3 instance(s): every token agrees with its descriptor
✓  presence-over-budget (§8.3)           clean — 9 token(s) visible to this reader in the presence domain; within the budget 10000
✓  storage-on-state (§4.2 S4)            clean — 1 storage(s) on 2 router(s), none answering on an owner's state keys
✓  archive-unaligned (§4.4)              clean — no archive.v1 provider visible to this reader: nothing to align
—  state-stamp-foreign (§4.2 S1–S2)      not asked
·  shm-memlock-low (§7.4)                finding — 1 subject(s)
    · info: this host — RLIMIT_MEMLOCK is 64 KiB, below the 8 MiB floor
✓  admin-unreachable (§4.2)              clean — 2 router(s) answered `@/*/router`
✓  router-version-skew (App. B)          clean — 2 router(s), all at 1.10.1
✗  health-failed (health.v1 §2.1)        finding — 1 subject(s)
    ✗ error: host-b/tc — its status is FAILED, fresh: present, and unable to do its primary job, as its owner says (health.v1 §2.1) — its reason: "netns gone"
✓  health-degraded (health.v1 §2.1)      clean — 1 service(s) implementing health.v1, none at DEGRADED: each fresh status, and the checks it vouches for, read better
?  health-stale (health.v1 §2.4)         unobservable — 1 subject(s) unjudged
    ? unjudged host-a/tc: its status's freshness could not be read: this reader's clock is not trusted to the HLC delta against the stamping clock (clock_untrusted)
✓  health-inconsistent (health.v1 §2.2)  clean — 1 service(s) implementing health.v1, each fresh status no better than its current checks
⚠  hostid-duplicate (hostid.v1 §2.12)    finding — 1 subject(s)
    ⚠ warning: h-bbd1aa1db10b/sysinfo — in both presence reads 2.0s apart, instances of h-bbd1aa1db10b/sysinfo on a system their descriptors declare minted state 2 session zids: the cause is undecided
—  population-over-bound (§2.7)          not asked

"#]]
    );

    let lines: Vec<serde_json::Value> = ndjson(&report)
        .lines()
        .map(|l| serde_json::from_str(l).expect("one object per line"))
        .collect();
    let envelope = &lines[0];
    assert_eq!(envelope["report"], "doctor");
    assert_eq!(envelope["scope"]["namespace"], "acme");
    assert!(
        !envelope.as_object().unwrap().contains_key("checks"),
        "checks are rows, not an envelope field"
    );
    let rows = &lines[1..];
    assert_eq!(rows.len(), zenkey_fleet::report::CheckId::ALL.len());
    assert!(rows.iter().all(|r| r["row"] == "check"));
    let answers: std::collections::BTreeSet<&str> = rows
        .iter()
        .map(|r| r["verdict"]["answer"].as_str().expect("an answer"))
        .collect();
    assert_eq!(
        answers,
        [
            "established",
            "not_asked",
            "not_established",
            "unobservable"
        ]
        .into(),
        "four poles, four spellings"
    );
    let not_asked = rows
        .iter()
        .find(|r| r["verdict"]["answer"] == "not_asked")
        .expect("a check not asked");
    assert!(
        not_asked.get("findings").is_none() && not_asked.get("unjudged").is_none(),
        "not asked carries no lists, never empty ones: {not_asked}"
    );
    let said = notes(&report);
    assert!(
        said.contains("3 service(s), 4 instance(s), 9 token(s)"),
        "{said}"
    );
    assert!(
        said.contains("not asked: state-stamp-foreign (pass --deep)"),
        "{said}"
    );
    assert!(
        said.contains("6 finding(s): 3 error(s), 2 warning(s), 1 info"),
        "{said}"
    );

    // The empty scope: a silence note naming it, and no verdict in the
    // summary — never a clean bill.
    let empty = zenkey_fleet::report::DoctorReport {
        unobservable: Some("no zk2 token visible to this reader in namespace \"x\"".into()),
        ..fx::doctor_report()
    };
    let n = notes(&empty);
    assert!(n.contains("no zk2 token visible"), "{n}");
    assert!(n.contains("no verdict on the deployment"), "{n}");
    let envelope: serde_json::Value =
        serde_json::from_str(ndjson(&empty).lines().next().unwrap()).unwrap();
    assert!(
        envelope["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|note| note["text"]
                .as_str()
                .unwrap()
                .contains("no zk2 token visible")),
        "the empty scope reaches a script too: {envelope}"
    );
}

/// zk2's `why` (#702): one row per rung, every pole spelled apart in every
/// medium — a mark and a word in the table, an `answer` in the row, the
/// cause only on the rung that established it — and the stop, the verdict
/// and what was asked on the envelope. Between the two fixtures every pole
/// appears; an archive's value is said to be last-known, never current.
#[test]
fn a_why_ladder_spells_every_rung_pole_apart_in_every_medium() {
    let cause = fx::why_report_cause();
    assert_data_eq!(
        table(&cause),
        str![[r#"
why acme/zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0 — a cause, at descriptor
✓  namespace (§1.6)      healthy — the key sits under namespace "acme"
✓  key (§1.1)            healthy — a stream key of host-a/tc, interface tc.netif.v1
✓  presence (§8.1)       healthy — 1 instance(s) of host-a/tc hold their token; 1 hold tc.netif.v1's interface token
✗  descriptor (§3.3)     cause — stream/bandwidth/{ns}/{iface} is unavailable at host-a/tc@3fa9c2d41b7e0012 (config: no bandwidth probe)
—  contract (§8.4)       not asked
—  answer (§2.1)         not asked
—  last-known (§4.2 S6)  not asked

"#]]
    );
    let silent = fx::why_report_silent();
    let mut answers = std::collections::BTreeSet::new();
    for report in [&cause, &silent] {
        let lines: Vec<serde_json::Value> = ndjson(report)
            .lines()
            .map(|l| serde_json::from_str(l).expect("one object per line"))
            .collect();
        let envelope = &lines[0];
        assert_eq!(envelope["report"], "why");
        assert!(envelope.get("rungs").is_none(), "rungs are rows");
        assert!(envelope.get("verdict").is_some() && envelope.get("stopped_at").is_some());
        let rows = &lines[1..];
        assert_eq!(rows.len(), 7, "every rung, asked or not");
        for r in rows {
            assert_eq!(r["row"], "rung");
            let answer = r["verdict"]["answer"].as_str().expect("an answer");
            answers.insert(answer.to_owned());
            assert_eq!(
                r.get("cause").is_some(),
                answer == "established",
                "a cause rides only the rung that established it: {r}"
            );
        }
    }
    assert_eq!(
        answers,
        [
            "established",
            "not_asked",
            "not_established",
            "unobservable"
        ]
        .map(str::to_owned)
        .into(),
        "four poles, four spellings"
    );
    let said = notes(&silent);
    assert!(said.contains("last-known, never current"), "{said}");
    assert!(said.contains("no verdict"), "{said}");
    assert!(notes(&cause).contains("a cause at descriptor"));
}

/// zk2's `check conform` (#703): one row per case, every pole spelled apart
/// in every medium — a mark and a word in the table, an `answer` in the
/// row, `detail` on a violation and on a case not asked — and the run's
/// own judgement on the envelope. The fixture has every pole non-empty.
#[test]
fn a_conform_suite_spells_every_case_pole_apart_in_every_medium() {
    let report = fx::conform_report();
    assert_data_eq!(
        table(&report),
        str![[r#"
conform host-a/tc tc.netif.v1 at sha256:5d1c0a9b2e3f4a6b5d1c0a9b2e3f4a6b5d1c0a9b2e3f4a6b5d1c0a9b2e3f4a6b
✓  contract-served (§8.4)       tc.netif.v1 sha256:5d1c0a9b2e3f4a6b5d1c0a9b2e3f4a6b5d1c0a9b2e3f4a6b5d1c0a9b2e3f4a6b  passed — a holder served the bundle the descriptor names, and it verified
✓  resource-served (§8.2)       stream/bandwidth/{ns}/{iface}                                                        passed — 12 sample(s) in the 5s window
✗  payload-type (§7.2)          stream/bandwidth/{ns}/{iface}                                                        violation — 1 of 12 value(s) do not conform to the declared type; the first, zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0: /stats: not an object
?  resource-served (§8.2)       state/namespaces                                                                     unobservable — nothing heard in the 5s window, and the state GET drew no reply
✗  operation (§5.1)             @op/diagnostics                                                                      violation — host-a/tc holds its tokens, and the call drew neither a value nor an envelope within 1s: never silence (O3)
—  operation (§5.1)             @op/interfaces/{ns}/{iface}/set                                                      not asked — not idempotent: each call is a write, which this suite makes only under --i-know
✗  freshness (freshness.v1)     state/interfaces/{ns}/{iface}                                                        violation — 1 of 2 member(s) stale against its horizon of 60 s: zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth1: not confirmed within its horizon
—  freshness (freshness.v1)     stream/bandwidth/{ns}/{iface}                                                        not asked — it declares no freshness.ttl_s
✗  budget (§2.7)                state/interfaces/{ns}/{iface}                                                        violation — 3 member(s) answered with a value by the owner's GET, above its bound of 2 (its descriptor's; the contract's is 1024): an owner MUST NOT hold more live members than its bound (§2.7)
?  budget-window (§2.7 window)  stream/bandwidth/{ns}/{iface}                                                        unobservable — 2 member(s) heard within one hour in the 5.0s window, within its bound of 1024 (the contract's): a window shows a population within its bound only after one liveness span, so a window of at least 3600 s (`--for 3600`), or --skip budget-window (§2.7)

"#]]
    );
    let lines: Vec<serde_json::Value> = ndjson(&report)
        .lines()
        .map(|l| serde_json::from_str(l).expect("one object per line"))
        .collect();
    let envelope = &lines[0];
    assert_eq!(envelope["report"], "conform");
    assert_eq!(envelope["judgement"]["answer"], "established");
    assert!(envelope.get("cases").is_none(), "cases are rows");
    let rows = &lines[1..];
    assert_eq!(rows.len(), report.cases.len());
    let answers: std::collections::BTreeSet<&str> = rows
        .iter()
        .map(|r| r["verdict"]["answer"].as_str().expect("an answer"))
        .collect();
    assert_eq!(
        answers,
        [
            "established",
            "not_asked",
            "not_established",
            "unobservable"
        ]
        .into(),
        "four poles, four spellings"
    );
    for r in rows {
        let answer = r["verdict"]["answer"].as_str().unwrap();
        assert_eq!(
            r.get("detail").is_some(),
            answer == "established" || answer == "not_asked",
            "{r}"
        );
    }
    let said = notes(&report);
    assert!(said.contains("pass --i-know"), "{said}");
    assert!(
        said.contains("1 budget-window case(s) unobservable")
            && said.contains("over a whole liveness span")
            && said.contains("--skip budget-window"),
        "{said}"
    );
    assert!(
        said.contains("10 case(s): 4 violation(s), 2 passed, 2 unobservable, 2 not asked."),
        "{said}"
    );
}

/// `zenctl health` (#721, PF): one row per service, every verdict of "is
/// this service healthy?" spelled apart in every medium — a mark and a word
/// in the table, the profile's own `verdict` token in a `service` row — with
/// §5's other answers beside it, stale never drawn as a level, an archive's
/// status last-known and never current, and the run's judgement leading the
/// envelope. The fixture has every verdict, every answer and every roll-up
/// count non-zero (tooling guide §7).
#[test]
fn a_health_reading_spells_every_verdict_apart_in_every_medium() {
    let report = fx::health_report();
    assert_data_eq!(
        table(&report),
        str![[r#"
health in namespace "acme" — worst FAILED: 1 healthy, 1 unhealthy, 1 stale, 1 unobservable, 1 not asked
✗  lab/liar   unhealthy (inconsistent, FAILED) — a check is worse than its status, which health.v1 §2.2 forbids
    status OK "serving" — its owner's reply, stamped 2026-10-10T08:00:00.000000000Z by a1
    checks disk FAILED "full"
    agrees with its checks: no — a current check is worse than its fresh status in each of 2 readings: its owner breaks health.v1 §2.2
    clock ahead: no — its status was confirmed after its last clock_ahead fault, or with none heard
✓  lab/ok     healthy (ok, OK) — its status is OK and confirmed, and no check is worse
    status OK "serving" — the last put heard, stamped 2026-10-10T08:00:00.000000000Z by a2
    agrees with its checks: yes — its status is fresh at a level, and no current check is worse
    clock ahead: no — its status was confirmed after its last clock_ahead fault, or with none heard
⚠  lab/ahead  stale (beyond_horizon) — not confirmed within its horizon
    agrees with its checks: unobservable — not confirmed within its horizon
    clock ahead: yes — a clock_ahead fault was heard, and no confirmation of its status since
    2 fault(s) heard, the last clock_ahead (profile) FAILED "2000 ms ahead"
?  lab/new    unobservable (unknown_level) — its status's level is unknown
    status UNSPECIFIED "starting" — its owner's reply, stamped 2026-10-10T08:00:00.000000000Z by a3
    agrees with its checks: unobservable — its status's level is unknown
    clock ahead: no — its status was confirmed after its last clock_ahead fault, or with none heard
—  lab/svc    not asked (absent) — it is absent: presence's word, not a level
    agrees with its checks: not asked — it is absent: presence's word, not a level
    clock ahead: not asked — it is absent: presence's word
    last-known at lab/archive (confirmed by alignment): DEGRADED "upstream lost" — last-known, never current

"#]]
    );
    let lines: Vec<serde_json::Value> = ndjson(&report)
        .lines()
        .map(|l| serde_json::from_str(l).expect("one object per line"))
        .collect();
    let envelope = &lines[0];
    assert_eq!(envelope["report"], "health");
    assert_eq!(envelope["judgement"]["answer"], "established");
    assert_eq!(envelope["rollup"]["worst"], "failed");
    assert!(envelope.get("services").is_none(), "services are rows");
    let rows = &lines[1..];
    assert_eq!(rows.len(), report.services.len());
    assert!(rows.iter().all(|r| r["row"] == "service"));
    let verdicts: std::collections::BTreeSet<&str> = rows
        .iter()
        .map(|r| r["verdict"].as_str().expect("a verdict"))
        .collect();
    assert_eq!(
        verdicts,
        ["healthy", "not_asked", "stale", "unhealthy", "unobservable"].into(),
        "five verdicts, five spellings"
    );
    for r in rows {
        let established = r["verdict"] == "healthy" || r["verdict"] == "unhealthy";
        assert_eq!(
            r.get("level").is_some(),
            established,
            "a level rides only an established verdict, never stale: {r}"
        );
    }
    let gone = rows.iter().find(|r| r["verdict"] == "not_asked").unwrap();
    assert_eq!(gone["last_known"]["archive"], "lab/archive");
    let said = notes(&report);
    assert!(said.contains("never by its interface token"), "{said}");
    assert!(
        said.contains("stale is its own finding, never a level"),
        "{said}"
    );
    assert!(
        said.contains("5 service(s): 1 healthy, 1 unhealthy, 1 stale, 1 unobservable, 1 not asked"),
        "{said}"
    );
}

/// Every family renders a table that is byte-stable at a fixed width, with no
/// trailing whitespace anywhere — the property that makes the snapshots above
/// reviewable at all.
#[test]
fn no_family_emits_trailing_whitespace() {
    let catalog = zk2fx::catalog();
    let renderings = [
        table(&fx::storage_list()),
        table(&fx::doctor_report()),
        table(&catalog.services()),
        table(&catalog.service(&"host-a/tc".parse().expect("an address"))),
        table(&catalog.ifaces()),
        table(&zk2fx::iface_view()),
        table(&catalog.graph()),
        table(&zk2fx::schema_view()),
        table(&zk2fx::compat_report()),
        table(&zk2fx::namespace_listing()),
        table(&zenctl::render::TopologyView {
            report: &fx::topology_with_instances(),
        }),
        table(&fx::why_report_cause()),
        table(&fx::why_report_silent()),
        table(&fx::conform_report()),
        table(&fx::health_report()),
    ];
    for r in &renderings {
        for line in r.lines() {
            assert_eq!(line, line.trim_end(), "trailing padding on {line:?}");
        }
    }
}

// ── Every remaining family ────────────────────────────────────────────────
//
// #201 asks for a table snapshot and an ndjson snapshot per `Render` impl, and
// this is the rest of them. They read as a wall of captured output, which is
// the point: a table regression was invisible to CI, and the only way it stops
// being invisible is for somebody to have written down what the table is.
//
// The families whose report type is zenctl-local or feature-gated build their
// fixtures here rather than in `zenkey-report-fixtures` — that crate cannot
// see a `KeyRelation`, and it must not enable the `decode` feature `GenReport`
// lives behind (#204).

/// Every non-answer population stays apart, in both media (#612, FJ8a): the
/// latency rows are value replies by replier key, and refusals, malformed
/// envelopes, transport errors, silent calls, R6's discards and panicked
/// calls are counted beside them. The fixture has every count non-zero, so
/// a renderer that summed two of them would show it.
#[test]
fn a_bench_report_right_aligns_its_numbers_and_counts_non_answers_apart() {
    assert_data_eq!(
        table(&fx::bench_report()),
        str![[r#"
bench */tc tc.netif.v1@sha256:4f4f4f4f4f4f4f4f… @op/diagnostics  (fan-out, 2s per call)
98 call(s), concurrency 8, 2.50s — 39.2 calls/s
  2 of 100 calls did not complete

replier    replies  min ms  p50 ms  p95 ms  p99 ms  max ms
host-a/tc       64    0.80    1.90   12.40   40.10  123.46
host-b/tc       34    1.10    2.20    9.90   11.00   12.50
refused 2 time(s), unattributed: busy ×2 (p50 0.50 ms)
host-a/tc holds the interface's token and sent no value in 34 call(s): refused or silent, which a caller cannot tell apart
host-c/tc holds the interface's token and sent no value in 98 call(s): refused or silent, which a caller cannot tell apart

"#]]
    );
    let n = notes(&fx::bench_report());
    for each in [
        "2 refusal(s)",
        "1 malformed envelope(s)",
        "3 transport error(s)",
        "1 silent call(s)",
        "4 discarded value(s)",
        "1 call(s) that panicked",
    ] {
        assert!(n.contains(each), "{each}: {n}");
    }
    assert!(n.contains("round trip"), "whose clock (O7): {n}");
    let lines: Vec<serde_json::Value> = ndjson(&fx::bench_report())
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines[0]["report"], "bench");
    assert!(lines[0].get("repliers").is_none(), "the repliers are rows");
    assert_eq!(lines[0]["silent"], 1);
    assert_eq!(lines[0]["refusals"]["count"], 2);
    assert_eq!(lines[1]["row"], "replier");
}

/// `check probe` over zk2 (#612, FJ8b): a consumer-shaped read of one
/// resource, no longer a call. The three verdicts each have their own
/// word, the presence read that attributes a silence is drawn only when it
/// was made, and an up-and-silent owner is told apart from no token at all.
#[test]
fn a_probe_names_its_verdict_and_attributes_its_silence() {
    use zenkey_fleet::Judgement;
    use zenkey_fleet::report::{ExpectPresence, ProbeReport};

    let silent = fx::probe_report();
    let t = table(&silent);
    assert!(
        t.starts_with(
            "probe host-a/tc tc.netif.v1 state/interfaces/{ns}/{iface}: 0 value(s) in 5.0s \
             (0 conforming, 0 not); current state: 0 key(s), 0 conforming\n"
        ),
        "{t}"
    );
    assert!(
        t.contains("presence: held by host-a/tc — up, and silent"),
        "{t}"
    );
    assert!(t.contains("NOTHING USABLE ARRIVED — the finding"), "{t}");
    assert_eq!(silent.verdict, Judgement::Established);
    let n = notes(&silent);
    assert!(
        n.contains("2 sample(s) on a wildcard key discarded by rule"),
        "{n}"
    );
    assert!(n.contains("resolved to no member of the resource"), "{n}");

    // No token, with a read that completed: the access-control caveat.
    let nobody = ProbeReport {
        presence: Some(ExpectPresence {
            holders: vec![],
            ..silent.presence.clone().unwrap()
        }),
        ..silent.clone()
    };
    assert!(table(&nobody).contains("presence: no token visible to this reader"));
    assert!(notes(&nobody).contains("a read access control refused"));
    // No token, and the read ran to its timeout: never "absent".
    let short = ProbeReport {
        presence: Some(ExpectPresence {
            holders: vec![],
            complete: false,
            ..silent.presence.clone().unwrap()
        }),
        ..silent.clone()
    };
    assert!(table(&short).contains("the read ended at its timeout"));

    // A value arrived: the presence read was not made, so it is not drawn.
    let arrived = ProbeReport {
        received: 1,
        conforming: 1,
        keys_seen: 1,
        presence: None,
        current: None,
        verdict: Judgement::NotEstablished {
            reason: "a value arrived".into(),
        },
        ..silent.clone()
    };
    let t = table(&arrived);
    assert!(t.contains("ARRIVED") && !t.contains("NOTHING"), "{t}");
    assert!(!t.contains("presence:"), "not asked is not drawn:\n{t}");
    assert!(
        !t.contains("current state"),
        "a stream has none to read:\n{t}"
    );

    let unobservable = ProbeReport {
        verdict: Judgement::Unobservable {
            reason: "the presence read failed".into(),
        },
        ..silent.clone()
    };
    let t = table(&unobservable);
    assert!(
        t.contains("UNOBSERVABLE — the silence cannot be attributed"),
        "{t}"
    );
    assert!(t.contains("  ! the presence read failed"), "{t}");

    // The verdict rides the envelope, and the exits are 1, 0, 2.
    let envelope: serde_json::Value =
        serde_json::from_str(ndjson(&silent).lines().next().unwrap()).unwrap();
    assert_eq!(envelope["discarded"], 2);
    assert_eq!(envelope["unresolved"], 1);
    for (report, code) in [(&silent, 1), (&arrived, 0), (&unobservable, 2)] {
        assert_eq!(
            zenkey_fleet::judgement_exit_code(&report.verdict),
            code,
            "{:?}",
            report.verdict
        );
    }
}

/// The field window (#223): per-path stats beside their findings, the path
/// table's bound stated in every format. Over zk2 (#612, FJ8b) a path is
/// declared by the contract's type or not, and field-stuck is *not asked*
/// until freshness has a profile (#613) — said, never silently dropped.
#[test]
fn a_field_report_states_its_bound_and_what_it_did_not_ask() {
    assert_data_eq!(
        table(&fx::field_report()),
        str![[r#"
40/40  zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0 · mtu          number  declared  unchanged  min 1500 max 1500 last 1500  values {1500}
40/40  zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0 · driver_hint  string  NOT declared by its type  1 change(s), last at 12.0s  values {"e1000", "virtio"}

⚠  field-new: zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0 · driver_hint — present in 40 of 40 document sample(s) but never declared by the contract's type json:NetworkInterface — drift at field granularity

"#]]
    );
    let stderr = notes(&fx::field_report());
    // Deliberately reworded by the bounds() migration (RFC 13, v1.24):
    // the cost is now an auto-appended bound note — count first, wording
    // from the declared BoundCost; `max_paths` and the refused examples
    // ride the document.
    assert!(
        stderr.contains("3 path observation(s) refused at the path-table bound"),
        "the bound's cost is stated (O6): {stderr}"
    );
    assert!(
        stderr.contains("2 sample(s) carried no document"),
        "undocumented is counted apart from absence (O5): {stderr}"
    );
    assert!(
        stderr.contains("3 sample(s) on keys no contract resolved"),
        "unresolved is unjudgeable, not clean (O4): {stderr}"
    );
    assert!(
        stderr.contains("field-stuck was not asked"),
        "not asked is stated, never read as clean (O4): {stderr}"
    );

    // The envelope leads the ndjson with the bound claim; rows and findings
    // ride behind it, tagged apart.
    let out = ndjson(&fx::field_report());
    let envelope: serde_json::Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    assert_eq!(envelope["report"], "field");
    assert_eq!(envelope["paths_dropped"], 3);
    assert!(
        !envelope.as_object().unwrap().contains_key("rows"),
        "rows are rows, not an envelope field"
    );
    let kinds: Vec<&str> = out
        .lines()
        .skip(1)
        .map(|l| {
            serde_json::from_str::<serde_json::Value>(l).unwrap()["row"]
                .as_str()
                .unwrap()
                .to_string()
                .leak() as &str
        })
        .collect();
    assert_eq!(kinds, ["path", "path", "finding"]);
}

/// `IMPAIRED` is the absence of a verdict, and the note says so in every
/// format. The expectation names a zk2 resource of an address, and a QoS
/// violation names the axis that differs, declared beside observed (§2.4).
#[test]
fn an_impaired_expectation_says_it_is_not_a_verdict_either_way() {
    assert_data_eq!(
        table(&fx::expect_report()),
        str![[r#"
*/tc tc.netif.v1 stream/bandwidth/{ns}/{iface}: 120 sample(s) on 4 key(s) over 5.0s, 24.00 Hz over the full window
violations (1 shown of 9):
  ✗  zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0: priority declared data, observed real_time
IMPAIRED — the observation cannot carry the claim:
  !  17 sample(s) were dropped while behind

"#]]
    );
    assert!(notes(&fx::expect_report()).contains("not a verdict either way"));
}

/// The fleet timeline (#216), arrival axis: lanes per zk2 resource of one
/// address (#612, FJ8b) with the stamper in the heading and whose clock it
/// is, the unstamped lane beside them, `pos` from the merged ordering, and
/// the drop as a break at its arrival position.
#[test]
fn a_timeline_on_arrival_groups_lanes_and_places_the_break() {
    assert_data_eq!(
        table(&fx::timeline_report_arrival()),
        str![[r#"
host-a/tc tc.netif.v1 stream/bandwidth/{ns}/{iface} · arrival · stamper 33 (2 on the owner's clock)
0  +1.000ms  200/33      acme/zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0
1  +2.000ms  100/33      acme/zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth1

unstamped (arrival axis only) · arrival · no stamper
3  +3.000ms              plain/key

breaks · arrival positions
2            dropped ×3

"#]]
    );
    let out = ndjson(&fx::timeline_report_arrival());
    let lines: Vec<serde_json::Value> = out
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let envelope = &lines[0];
    assert_eq!(envelope["report"], "timeline");
    assert_eq!(envelope["order_by"], "arrival");
    assert_eq!(
        envelope["lens"]["presence"]["complete"], true,
        "the lens the lanes were resolved with rides the envelope"
    );
    assert_eq!(
        envelope["lanes"][0]["provenance"],
        serde_json::json!({"owner": 2, "other": 0, "unattributable": 0})
    );
    let rows: Vec<&str> = lines[1..]
        .iter()
        .map(|r| r["row"].as_str().unwrap())
        .collect();
    assert_eq!(rows, ["sample", "sample", "break", "sample"]);
    assert_eq!(lines[1]["provenance"], "owner", "whose clock, per row (O7)");
    assert!(
        lines[4].get("provenance").is_none(),
        "an unstamped row names no stamper: {}",
        lines[4]
    );
    let n = notes(&fx::timeline_report_arrival());
    assert!(n.contains("ordered by arrival"), "{n}");
    assert!(n.contains("sequence-number lane is unavailable"), "{n}");
    assert!(n.contains("deliberately no edges"), "{n}");
}

/// The same window on the HLC axis: the reorder shows as a non-monotonic
/// `t` column, the claim names the one stamper, the unstamped sample is a
/// count and a note rather than a row, and the drop is a total with no
/// break row (it has no position on this clock).
#[test]
fn a_timeline_on_hlc_states_its_claim_and_excludes_the_unstamped() {
    assert_data_eq!(
        table(&fx::timeline_report_hlc()),
        str![[r#"
host-a/tc tc.netif.v1 stream/bandwidth/{ns}/{iface} · hlc · stamper 33 (2 on the owner's clock)
0  +2.000ms  100/33  acme/zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth1
1  +1.000ms  200/33  acme/zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0

"#]]
    );
    let out = ndjson(&fx::timeline_report_hlc());
    let envelope: serde_json::Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    assert_eq!(envelope["claim"], "happens_before");
    assert_eq!(envelope["stamper"], "33");
    assert_eq!(envelope["unstamped_excluded"], 1);
    assert_eq!(
        out.lines()
            .filter(|l| l.contains("\"row\":\"break\""))
            .count(),
        0,
        "a drop has no position on this clock"
    );
    let n = notes(&fx::timeline_report_hlc());
    assert!(n.contains("happened-before"), "{n}");
    assert!(
        n.contains("1 unstamped sample(s) are not on this axis"),
        "{n}"
    );
    assert!(n.contains("have no position on the HLC axis"), "{n}");
}

/// The claim, not the layout: an HLC table never draws an unstamped row.
/// The report has none to give it (`Placed<HlcAxis>` refused them in the
/// engine), and this pins that the renderer does not invent one from the
/// lane summaries or the exclusion count.
#[test]
fn an_hlc_timeline_never_draws_an_unstamped_row() {
    let report = fx::timeline_report_hlc();
    assert!(report.rows.iter().all(|r| !matches!(
        r,
        zenkey_fleet::report::TimelineEntry::Sample { hlc: None, .. }
    )));
    let drawn = table(&report);
    assert!(!drawn.contains("plain/key"), "{drawn}");
    assert!(!drawn.contains("unstamped ("), "{drawn}");
    // Every emitted row says which axis its position is on.
    for line in ndjson(&report).lines().filter(|l| l.contains("\"row\"")) {
        assert!(line.contains("\"order_by\":\"hlc\""), "{line}");
    }
    for line in ndjson(&fx::timeline_report_arrival())
        .lines()
        .filter(|l| l.contains("\"row\""))
    {
        assert!(line.contains("\"order_by\":\"arrival\""), "{line}");
    }
}

#[test]
fn a_record_and_a_replay_carry_their_drop_ledgers() {
    assert_data_eq!(
        table(&fx::record_report()),
        str![[r#"
recorded 4820 sample(s) in 10.0s to bus.zrec

"#]]
    );
    assert!(notes(&fx::record_report()).contains("dropped while behind"));
    assert_data_eq!(
        table(&fx::replay_report()),
        str![[r#"
would replay 4800 put(s) and 20 tombstone(s) from acme/v1/** (captured 2026-08-21T00:00:00Z)
  line 41: unknown QoS profile "data/drop/reliable"
  line 88: refused delete on a telemetry key

"#]]
    );
    assert!(notes(&fx::replay_report()).contains("partial view of a partial view"));
}

/// A snapshot states its span in every format (RFC 13 §4.4, #219): the
/// table's second line, and a caveat note the machine formats carry. Its
/// holders are zk2's (#612, FJ8b): live owners, addresses no instance held,
/// and the payloads that failed their type, each counted apart.
#[test]
fn a_snapshot_states_its_span_and_its_holders() {
    assert_data_eq!(
        table(&fx::snapshot_report()),
        str![[r#"
snapshot of acme/zk2/*/*/*/state/**: 4 key(s) — 3 live, 1 with no instance, 0 unattributed; 1 not conforming to their type → deployment.zsnap
collected over 1.25s from 2026-10-09T00:00:00Z (1 asked, 5 answered)

"#]]
    );
    let n = notes(&fx::snapshot_report());
    assert!(n.contains("not at an instant"), "{n}");
    assert!(n.contains("lost to a newer one"), "{n}");
    assert!(
        n.contains("no instance visible to this reader") && n.contains("S4 forbids"),
        "an answer for an address nobody holds is named, never counted live: {n}"
    );
    assert!(n.contains("@state keys are excluded, not empty"), "{n}");
    assert!(
        !n.contains("presence not read"),
        "the fixture asked presence: {n}"
    );
    assert_data_eq!(
        ndjson(&fx::snapshot_report()),
        str![[r#"
{"header":{"answered":5,"asked":1,"base":"acme","collected_at":"2026-10-09T00:00:00Z","collection_span_s":1.25,"presence":{"complete":true,"selector":"zk2/*/*/@zk/**","services":2},"selectors":["acme/zk2/*/*/*/state/**"],"superseded":1,"zsnap":2},"live":3,"no_instance":1,"nonconforming":1,"notes":[{"cite":"RFC 13 §4.4","text":"collected over 1.25s, not at an instant — a fan-in GET has no single moment"},{"cite":"tooling guide O5","text":"@state keys are excluded, not empty: no selector names `@state`, and `*`/`**` never match a verbatim chunk"},{"cite":"spec §4.2 S4","text":"1 value(s) answered for an address with no instance visible to this reader: an owner gone, or a store answering on its keys, which S4 forbids"},{"cite":"RFC 09 §5.1 O6","text":"1 answer(s) for one key lost to a newer one from another selector"}],"out":"deployment.zsnap","report":"snapshot","unattributed":0}

"#]]
    );
}

/// A diff states both spans, keeps the facets apart in its rows — value,
/// conformance, holder — and its human verdict word is the exit code's
/// carrier (#219). Keys compare by their zk2 key (#612, FJ8b).
#[test]
fn a_snapshot_diff_states_both_spans_and_tags_every_row() {
    assert_data_eq!(
        table(&fx::snapshot_diff()),
        str![[r#"
a: 4 key(s), span 1.25s at 2026-10-09T00:00:00Z in namespace "acme" (acme/zk2/*/*/*/state/**)
b: 4 key(s), span 0.80s at 2026-10-09T00:05:00Z in namespace "acme" (acme/zk2/*/*/*/state/**)
1 added, 1 removed, 2 changed, 1 unchanged
  +  zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth1
  -  zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth9
  ~  zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0  value: 1 change(s) (is_up: "yes" → true); conformance: invalid(1) → valid
  ~  zk2/host-b/tc/tc.netif.v1/state/namespaces               holder: no_instance → live(owner)
DIFFERENT

"#]]
    );
    let n = notes(&fx::snapshot_diff());
    assert!(n.contains("a: span 1.25s"), "{n}");
    assert!(n.contains("b: span 0.80s"), "{n}");
    assert_data_eq!(
        ndjson(&fx::snapshot_diff()),
        str![[r#"
{"a":{"answered":5,"asked":1,"base":"acme","collected_at":"2026-10-09T00:00:00Z","collection_span_s":1.25,"presence":{"complete":true,"selector":"zk2/*/*/@zk/**","services":2},"selectors":["acme/zk2/*/*/*/state/**"],"superseded":1,"zsnap":2},"b":{"answered":4,"asked":1,"base":"acme","collected_at":"2026-10-09T00:05:00Z","collection_span_s":0.8,"presence":{"complete":true,"selector":"zk2/*/*/@zk/**","services":2},"selectors":["acme/zk2/*/*/*/state/**"],"zsnap":2},"notes":[{"cite":"RFC 13 §4.4","text":"a: span 1.25s at 2026-10-09T00:00:00Z, b: span 0.80s at 2026-10-09T00:05:00Z — each side was collected over its span, not at an instant"}],"report":"snapshot-diff","unchanged":1}
{"key":"zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth1","row":"added"}
{"key":"zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth9","row":"removed"}
{"conformance":[{"state":"invalid","violations":["/is_up: expected boolean"]},{"state":"valid"}],"key":"zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0","row":"changed","timestamp":["7f3b2a1c00000001/ab12","7f3b2a1c00000002/ab12"],"value":{"changes":[{"new":true,"old":"yes","op":"changed","path":"is_up"}],"truncated":0}}
{"holder":[{"address":"host-b/tc","kind":"no_instance"},{"address":"host-b/tc","answered_by":"owner","kind":"live"}],"key":"zk2/host-b/tc/tc.netif.v1/state/namespaces","row":"changed","timestamp":["7f3b2a1c00000001/cd34","7f3b2a1c00000001/cd34"]}

"#]]
    );
    // Identity: the clean word, and nothing listed.
    let same = table(&fx::snapshot_diff_identity());
    assert!(same.contains("IDENTICAL"), "{same}");
    assert!(!same.contains("  ~"), "{same}");
}

/// Two namespaces line up on their zk2 keys — what v1 needed an origin
/// alignment for, zk2's key carries — and the two deployments' clocks are
/// not compared: a stamp that moved alone is not a change.
#[test]
fn two_namespaces_diff_by_zk2_key_and_say_so() {
    let d = fx::snapshot_diff_namespaces();
    let t = table(&d);
    assert!(t.contains("(acme/zk2/*/*/*/state/**)"), "{t}");
    assert!(t.contains("(staging/zk2/*/*/*/state/**)"), "{t}");
    assert!(t.contains("IDENTICAL"), "{t}");
    assert!(
        !t.contains("  ~") && !t.contains("  +") && !t.contains("  -"),
        "{t}"
    );
    let n = notes(&d);
    assert!(n.contains("clocks are not compared"), "{n}");
}

/// The `.zsnap` files the CLI corpus diffs (`tests/cmd/snapshot-diff.trycmd`)
/// are the shared fixtures, written through the engine's own writer — so a
/// change to the fixture or the dialect moves both corpora together.
/// `SNAPSHOTS=overwrite` (`just snapshots`) rewrites them; otherwise they
/// must already match.
#[test]
fn the_zsnap_corpus_fixtures_are_the_shared_fixtures() {
    for (name, snapshot) in [
        ("a.zsnap", fx::snapshot()),
        ("b.zsnap", fx::snapshot_b()),
        // The two-namespace diff (`snapshot-diff.trycmd`).
        ("staging.zsnap", fx::snapshot_staging()),
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/cmd/fixtures")
            .join(name);
        let mut bytes = Vec::new();
        let mut w = zenkey_fleet::ZsnapWriter::new(&mut bytes, &snapshot.header).unwrap();
        for row in &snapshot.rows {
            w.write_row(row).unwrap();
        }
        w.finish().unwrap();
        if std::env::var("SNAPSHOTS").as_deref() == Ok("overwrite") {
            std::fs::write(&path, &bytes).unwrap();
        }
        let on_disk = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("{}: {e} — run `just snapshots`", path.display()));
        assert_eq!(
            String::from_utf8(on_disk).unwrap(),
            String::from_utf8(bytes).unwrap(),
            "{name} drifted from the shared fixture — run `just snapshots`"
        );
    }
}

/// The O6 eviction count leads, so `| head -5` cannot lose it. Keys group
/// by zk2 address and resource (#612, FJ8b) ahead of the per-key lines, and
/// a key that is not zk2 data is its own group, never folded into one.
#[test]
fn a_rate_reports_bound_leads_its_ndjson() {
    let view = zenctl::render::RateView {
        report: &fx::rate_report(),
        bandwidth: false,
    };
    let first = ndjson(&view).lines().next().unwrap().to_string();
    let envelope: serde_json::Value = serde_json::from_str(&first).unwrap();
    assert_eq!(envelope["evicted"], 912);
    assert_data_eq!(
        table(&view),
        str![[r#"
10.00 Hz  host-a/tc tc.netif.v1 stream/bandwidth/{ns}/{iface}  (2 key(s))
 2.00 Hz  not a zk2 key  (1 key(s))

5.00 Hz  zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0  (0 sn gap(s))  lat — (50 unstamped: no HLC, no latency — not zero)
2.00 Hz  rt/chatter  (0 sn gap(s))
total: 960.00 Hz over 50000 key(s) (9600 samples / 10s)

"#]]
    );

    let bw = zenctl::render::RateView {
        report: &fx::rate_report(),
        bandwidth: true,
    };
    assert_data_eq!(
        table(&bw),
        str![[r#"
405.0 B/s  host-a/tc tc.netif.v1 stream/bandwidth/{ns}/{iface}  (2 key(s))
  8.0 B/s  not a zk2 key  (1 key(s))

225.0 B/s  zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0
  8.0 B/s  rt/chatter
total: 52800.0 B/s over 50000 key(s) (528000 bytes / 10s)

"#]]
    );
}

/// An empty scout heard a boundary, and says so in the document — not as
/// prose on stdout, which is what it used to do (#236).
#[test]
fn an_empty_scout_and_an_empty_router_list_are_boundaries_not_absences() {
    assert_eq!(table(&fx::scout_report_empty()), "");
    assert!(notes(&fx::scout_report_empty()).contains("boundary"));
    assert_data_eq!(
        table(&fx::scout_report()),
        str![[r#"
aabbccdd  router  tcp/10.0.0.1:7447

"#]]
    );

    assert_eq!(table(&fx::router_list_empty()), "");
    assert!(notes(&fx::router_list_empty()).contains("peer-only mesh"));
    assert_data_eq!(
        table(&fx::router_list()),
        str![[r#"
aabbccdd  1.9.0  tcp/10.0.0.1:7447

"#]]
    );
}

/// Two row kinds on one stream, told apart by a tag rather than by probing
/// for fields. v1's origin attachments (`--origins`) left at FJ9 (#612).
#[test]
fn an_admin_graph_tags_its_row_kinds() {
    let report = fx::topology();
    let view = zenctl::render::TopologyView { report: &report };
    let kinds: Vec<String> = ndjson(&view)
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v.get("row").and_then(|r| r.as_str()).map(str::to_string))
        .collect();
    assert_eq!(
        kinds,
        ["node", "node", "edge"],
        "nodes and edges used to be one untagged stream"
    );
    assert_data_eq!(
        table(&view),
        str![[r#"
aabbccdd  router  1.9.0  tcp/10.0.0.1:7447
eeff0011  peer    —      (heard of, not queryable)
  aabbccdd —— eeff0011  [tcp]

"#]]
    );
}

/// zk2's instances on the mesh (#705): each of the three attachments is
/// spelled apart in every medium — a mark and a word in the table, an
/// `attachment` tag on its row — a missing zid is `—`, never an empty
/// one, and what the join read (namespace, verified routers, the answers
/// that count for nothing) reaches a script as notes and an envelope.
/// Every count in the fixture is non-zero, so a renderer that merged two
/// attachments fails here (tooling guide §7).
#[test]
fn an_admin_graph_spells_each_instance_attachment_apart() {
    let report = fx::topology_with_instances();
    let view = zenctl::render::TopologyView { report: &report };
    assert_data_eq!(
        table(&view),
        str![[r#"
aabbccdd  router  1.9.0  tcp/10.0.0.1:7447
eeff0011  peer    —      (heard of, not queryable)
  aabbccdd —— eeff0011  [tcp]

zk2 instances:
host-a/tc@3fa9c2d41b7e0012             eeff0011  → aabbccdd (as peer)
host-b/tc@3fa9c2d41b7e0013             c0ffee    ✗ unattached: no verified router (aabbccdd) lists zid c0ffee among its sessions, compared by value
ws-01/tcgui-frontend@3fa9c2d41b7e0014  —         ? unattributable: its descriptor names no session zid (`meta.zid`)

"#]]
    );
    let lines: Vec<serde_json::Value> = ndjson(&view)
        .lines()
        .map(|l| serde_json::from_str(l).expect("one object per line"))
        .collect();
    let envelope = &lines[0];
    assert_eq!(envelope["report"], "admin-graph");
    assert_eq!(envelope["instances"]["namespace"], "acme");
    assert_eq!(
        envelope["instances"]["verified"],
        serde_json::json!(["aabbccdd"])
    );
    assert!(
        envelope["instances"].get("instances").is_none(),
        "instances are rows, not an envelope field"
    );
    let attachments: Vec<&str> = lines
        .iter()
        .filter(|r| r["row"] == "instance")
        .map(|r| r["attachment"].as_str().expect("a tag"))
        .collect();
    assert_eq!(attachments, ["attached", "unattached", "unattributable"]);
    let unattributable = lines
        .iter()
        .find(|r| r["attachment"] == "unattributable")
        .expect("one");
    assert!(
        unattributable.get("zid").is_none(),
        "no zid is absence: {unattributable}"
    );
    let said = notes(&view);
    assert!(
        said.contains("3 zk2 instance(s) read in namespace \"acme\""),
        "{said}"
    );
    assert!(
        said.contains("1 attached, 1 unattached, 1 unattributable"),
        "{said}"
    );
    assert!(said.contains("attach nothing"), "{said}");
}

/// The ACL plan draws its three lists as two tables (principals with what
/// they run and their rules, rules with their grant and key expressions)
/// and tags its row kinds on the stream.
#[test]
fn an_acl_plan_draws_principals_then_rules_and_tags_its_row_kinds() {
    let plan = fx::acl_plan();
    let out = ndjson(&plan);
    let mut kinds: Vec<String> = out
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v.get("row").and_then(|r| r.as_str()).map(str::to_string))
        .collect();
    kinds.dedup();
    assert_eq!(kinds, ["rule", "subject", "policy"]);
    let envelope: serde_json::Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    assert_eq!(envelope["report"], "acl-plan");
    assert_eq!(envelope["default_permission"], "deny");
    assert_eq!(envelope["contracts"].as_array().unwrap().len(), 1);
    assert!(envelope.get("face").is_none(), "no face is absent");
    assert!(envelope.get("rules").is_none(), "the lists are rows");
    assert_data_eq!(
        table(&plan),
        str![[r#"
principals under namespace "" (default deny):

  subject   bound by         runs          rules
  tc-h1     user tc-h1       h1/tc         own-in:h1/tc, own-out:h1/tc, fan-in:h1/tc, fan-in-reply:h1/tc, contracts-in, contracts-out
  tc-h2     user tc-h2       h2/tc         own-in:h2/tc, own-out:h2/tc, fan-in:h2/tc, fan-in-reply:h2/tc, contracts-in, contracts-out
  frontend  cn frontend.ops  ops/frontend  own-in:ops/frontend, own-out:ops/frontend, consume-in:ops/frontend, consume-out:ops/frontend, presence-in:ops/frontend, presence-out:ops/frontend, call-in:ops/frontend, call-out:ops/frontend, contracts-in, contracts-out

rules:

  rule                       grant         flows    messages
  own-in:h1/tc               own           ingress  put, delete, declare_queryable, reply, liveliness_token
      zk2/h1/tc/**
      zk2/h1/tc/*/@stream/**
      zk2/h1/tc/*/@state/**
      zk2/h1/tc/*/@op/**
      zk2/h1/tc/@zk/**
  own-out:h1/tc              own           egress   query, declare_subscriber
      zk2/h1/tc/**
      zk2/h1/tc/*/@stream/**
      zk2/h1/tc/*/@state/**
      zk2/h1/tc/*/@op/**
      zk2/h1/tc/@zk/**
  fan-in:h1/tc               fan_in        egress   query, declare_subscriber
      zk2/*/tc/tc.netif.v1/state/**
      zk2/*/tc/tc.netif.v1/@op/interfaces/*/set
  fan-in-reply:h1/tc         fan_in_reply  ingress  reply
      zk2/*/tc/tc.netif.v1/state/**
      zk2/*/tc/tc.netif.v1/@op/interfaces/*/set
  own-in:h2/tc               own           ingress  put, delete, declare_queryable, reply, liveliness_token
      zk2/h2/tc/**
      zk2/h2/tc/*/@stream/**
      zk2/h2/tc/*/@state/**
      zk2/h2/tc/*/@op/**
      zk2/h2/tc/@zk/**
  own-out:h2/tc              own           egress   query, declare_subscriber
      zk2/h2/tc/**
      zk2/h2/tc/*/@stream/**
      zk2/h2/tc/*/@state/**
      zk2/h2/tc/*/@op/**
      zk2/h2/tc/@zk/**
  fan-in:h2/tc               fan_in        egress   query, declare_subscriber
      zk2/*/tc/tc.netif.v1/state/**
      zk2/*/tc/tc.netif.v1/@op/interfaces/*/set
  fan-in-reply:h2/tc         fan_in_reply  ingress  reply
      zk2/*/tc/tc.netif.v1/state/**
      zk2/*/tc/tc.netif.v1/@op/interfaces/*/set
  own-in:ops/frontend        own           ingress  put, delete, declare_queryable, reply, liveliness_token
      zk2/ops/frontend/**
      zk2/ops/frontend/*/@stream/**
      zk2/ops/frontend/*/@state/**
      zk2/ops/frontend/*/@op/**
      zk2/ops/frontend/@zk/**
  own-out:ops/frontend       own           egress   query, declare_subscriber
      zk2/ops/frontend/**
      zk2/ops/frontend/*/@stream/**
      zk2/ops/frontend/*/@state/**
      zk2/ops/frontend/*/@op/**
      zk2/ops/frontend/@zk/**
  consume-in:ops/frontend    consume       ingress  declare_subscriber, query
      zk2/*/tc/tc.netif.v1/state/**
  consume-out:ops/frontend   consume       egress   put, delete, reply
      zk2/*/tc/tc.netif.v1/state/**
  presence-in:ops/frontend   presence      ingress  declare_liveliness_subscriber, liveliness_query, query, declare_subscriber
      zk2/*/tc/@zk/**
  presence-out:ops/frontend  presence      egress   liveliness_token, reply, put
      zk2/*/tc/@zk/**
  call-in:ops/frontend       call          ingress  query
      zk2/*/tc/tc.netif.v1/@op/interfaces/*/set
  call-out:ops/frontend      call          egress   reply
      zk2/*/tc/tc.netif.v1/@op/interfaces/*/set
  contracts-in               contracts     ingress  declare_queryable, reply, query
      zk2/@zk/contract/**
  contracts-out              contracts     egress   query, reply
      zk2/@zk/contract/**

"#]]
    );
    assert_data_eq!(
        notes(&plan),
        str![[r#"
18 rule(s), 3 subject(s), 3 polic(y/ies), compiled from 1 contract revision(s)
write the fragment: `zenctl acl gen --enrollment … --contracts … --json5 > router-acl.json5`, merge it at the router config's top level, restart the router, and regenerate on every contract revision (spec §11.2)

"#]]
    );
    // A refusal is a note in every format, and a row in the machine ones.
    let refused = fx::acl_plan_refused();
    assert!(notes(&refused).contains("REFUSED bench-rig"));
    assert!(ndjson(&refused).contains(r#""row":"refusal""#));
}

/// The check puts the verdict word beside its findings, and states what it
/// could not see: the running block.
#[test]
fn an_acl_check_names_its_findings_and_what_it_could_not_observe() {
    assert_data_eq!(
        table(&fx::acl_check()),
        str![[r#"
router.json5: 17 rule(s) and 4 subject(s) configured; 18 and 3 planned
  ✗ rule_missing      fan-in:h1/tc   planned allow egress declare_subscriber,query zk2/*/tc/tc.netif.v1/state/**
  ✗ unknown_identity  user stranger  configured bound by subject "stranger"
FAIL

"#]]
    );
    assert_data_eq!(
        table(&fx::acl_check_clean()),
        str![[r#"
router.json5: 18 rule(s) and 3 subject(s) configured; 18 and 3 planned
PASS

"#]]
    );
    let n = notes(&fx::acl_check());
    assert!(n.contains("not observable on the bus"));
    let out = ndjson(&fx::acl_check());
    let envelope: serde_json::Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    assert_eq!(envelope["judgement"]["answer"], "established");
    assert_eq!(out.lines().count(), 3, "envelope + two findings");
}

/// The explanation answers per direction with the rules that decided, deny
/// first, and its ndjson is one row per direction, tagged with the flow.
#[test]
fn an_acl_explain_answers_per_direction() {
    assert_data_eq!(
        table(&fx::acl_explain()),
        str![[r#"
frontend  query  zk2/*/tc/tc.netif.v1/state/**
  ingress  ALLOWED            consume-in:ops/frontend includes it for query on ingress
      ✓ consume-in:ops/frontend  allow  zk2/*/tc/tc.netif.v1/state/**  (consume)
  egress   denied by default  no rule of frontend's policies includes zk2/*/tc/tc.netif.v1/state/** for query on egress: default_permission deny

"#]]
    );
    let out = ndjson(&fx::acl_explain());
    let rows: Vec<serde_json::Value> = out
        .lines()
        .skip(1)
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["flow"], "ingress");
    assert_eq!(rows[0]["decision"], "allowed");
    assert_eq!(rows[0]["via"][0]["grant"], "consume");
    assert_eq!(rows[1]["flow"], "egress");
    assert_eq!(rows[1]["decision"], "denied_by_default");
}
// ── The zenctl-local families ─────────────────────────────────────────────
//
// Their fixtures are here rather than in `zenkey-report-fixtures`: that crate
// cannot see a `KeyRelation`, and it must not enable the `decode` feature
// `GenReport` lives behind (#204).

#[test]
fn a_key_relation_carries_its_convention_note_once() {
    let no = zenctl::render::KeyRelation {
        op: zenctl::render::KeyOp::Includes,
        a: "v1/**".into(),
        b: "v1/h-3fa9/@rpc/sysinfo/introspect".into(),
        answer: false,
        note: Some("`**` never crosses an `@`-chunk (RFC 03 §4 D2)".into()),
    };
    assert_data_eq!(
        table(&no),
        str![[r#"
no — v1/** does not include all of v1/h-3fa9/@rpc/sysinfo/introspect
`**` never crosses an `@`-chunk (RFC 03 §4 D2)

"#]]
    );
    // Once: the note is a field of this report, so returning it from `notes()`
    // as well would put the same sentence in the document twice.
    let line = ndjson(&no);
    let doc: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert!(doc.get("note").is_some());
    assert!(doc.get("notes").is_none(), "not twice: {doc}");
}

/// `key canon` is a filter: `$(zenctl key canon "$x")` has to be the answer
/// and nothing else.
#[test]
fn a_key_canon_prints_the_answer_alone_when_it_changed() {
    let changed = zenctl::render::KeyCanon {
        input: "v1/**/**/a".into(),
        canon: "v1/**/a".into(),
        changed: true,
    };
    assert_eq!(table(&changed), "v1/**/a\n");
    let same = zenctl::render::KeyCanon {
        input: "v1/**/a".into(),
        canon: "v1/**/a".into(),
        changed: false,
    };
    assert_eq!(table(&same), "v1/**/a is already canonical\n");
}

/// `zenctl hostid` (#719): the system leads, alone on its line, then where
/// it came from and every input read. An id from the shared file has no v1
/// origin, and the note says why, in every format.
#[test]
fn a_hostid_report_leads_with_the_system() {
    use zenctl::render::{HostIdInput, HostIdReport, V1Origin};
    let input = |path: &str, outcome: &str| HostIdInput {
        path: path.into(),
        outcome: outcome.into(),
    };
    let r = HostIdReport {
        system: "h-504c6767c349".into(),
        from: "/var/lib/zk2/hostid".into(),
        salt: "zk2-hostid-v1".into(),
        inputs: vec![
            input("/etc/machine-id", "refused"),
            input("/var/lib/dbus/machine-id", "absent"),
            input("/var/lib/zk2/hostid", "id"),
        ],
        v1: vec![V1Origin {
            salt: "tcgui-host-id-v1".into(),
            origin: None,
        }],
    };
    assert_data_eq!(
        table(&r),
        str![[r#"
h-504c6767c349
from                        /var/lib/zk2/hostid
salt                        zk2-hostid-v1
  /etc/machine-id           refused
  /var/lib/dbus/machine-id  absent
  /var/lib/zk2/hostid       id
v1 tcgui-host-id-v1         no mapping

"#]]
    );
    assert_data_eq!(
        notes(&r),
        str![[r#"
the id came from the shared file, which no v1 application read: this host's v1 origins have no mapping by derivation

"#]]
    );
    assert_data_eq!(
        ndjson(&r),
        str![[r#"
{"from":"/var/lib/zk2/hostid","notes":[{"text":"the id came from the shared file, which no v1 application read: this host's v1 origins have no mapping by derivation"}],"report":"hostid","salt":"zk2-hostid-v1","system":"h-504c6767c349"}
{"outcome":"refused","path":"/etc/machine-id","row":"input"}
{"outcome":"absent","path":"/var/lib/dbus/machine-id","row":"input"}
{"outcome":"id","path":"/var/lib/zk2/hostid","row":"input"}
{"row":"v1_origin","salt":"tcgui-host-id-v1"}

"#]]
    );
}

/// `check schema` over zk2 (#612, FJ8b): the three verdicts each have
/// their own word, the violations follow one per line, and nothing to say
/// is *absent* in the document, not an empty list meaning the same thing.
#[test]
fn a_schema_check_names_its_verdict_and_omits_what_it_lacks() {
    use zenkey_fleet::report::{Conformance, PayloadCheck};
    let valid = PayloadCheck {
        iface: "tc.netif.v1".into(),
        fingerprint: actfx::fp(),
        resource: "state/interfaces/{ns}/{iface}".into(),
        member: "type".into(),
        declared: "json:NetworkInterface".into(),
        encoding: None,
        size: 58,
        conformance: Conformance::Valid,
        value: None,
    };
    let doc: serde_json::Value = serde_json::from_str(ndjson(&valid).trim()).unwrap();
    assert_eq!(
        doc,
        serde_json::json!({
            "report": "schema-check",
            "iface": "tc.netif.v1",
            "fingerprint": actfx::fp(),
            "resource": "state/interfaces/{ns}/{iface}",
            "member": "type",
            "declared": "json:NetworkInterface",
            "size": 58,
            "conformance": {"state": "valid"},
        }),
        "no encoding given and no value kept: both absent"
    );
    assert_eq!(
        table(&valid),
        "json:NetworkInterface tc.netif.v1@sha256:4f534f534f534f53… \
         state/interfaces/{ns}/{iface} type: valid (58 B)\n"
    );

    let invalid = PayloadCheck {
        conformance: Conformance::Invalid {
            violations: vec!["/is_up: expected boolean".into()],
        },
        ..valid.clone()
    };
    let t = table(&invalid);
    assert!(t.contains("type: invalid (58 B)"), "{t}");
    assert!(t.contains("\n  /is_up: expected boolean"), "{t}");

    let undecodable = PayloadCheck {
        encoding: Some("application/cbor".into()),
        conformance: Conformance::Undecodable {
            declared: "json:NetworkInterface".into(),
            reason: "not CBOR".into(),
        },
        ..valid
    };
    let t = table(&undecodable);
    assert!(
        t.contains("type: undecodable (58 B, read as application/cbor)"),
        "the encoding read as is stated when given:\n{t}"
    );
    assert!(t.contains("\n  not CBOR"), "{t}");
}

/// The cache is this tool's own disk footprint, and a script is a user:
/// `cache show --format json | jq -r .dir` is the point of the command (#54).
#[test]
fn a_cache_report_names_its_directory_in_both_formats() {
    let full = zenctl::render::CacheReport {
        dir: "/home/u/.cache/zenkey-explorer/lab/slices".into(),
        listed: vec![String::new(), "prod".into()],
        seen: vec![
            zenctl::render::CachedNamespace {
                namespace: String::new(),
                services: 1,
                ifaces: 1,
            },
            zenctl::render::CachedNamespace {
                namespace: "prod".into(),
                services: 3,
                ifaces: 2,
            },
        ],
    };
    assert_data_eq!(
        table(&full),
        str![[r#"
/home/u/.cache/zenkey-explorer/lab/slices
  (empty)  1 service(s), 1 interface(s)
  prod     3 service(s), 2 interface(s)
namespaces listed: (empty), prod

"#]]
    );
    let doc: serde_json::Value =
        serde_json::from_str(ndjson(&full).lines().next().unwrap()).unwrap();
    assert_eq!(doc["dir"], "/home/u/.cache/zenkey-explorer/lab/slices");
    assert_eq!(doc["namespaces"], 2);

    let empty = zenctl::render::CacheReport {
        dir: "/home/u/.cache/zenkey-explorer/default/slices".into(),
        listed: vec![],
        seen: vec![],
    };
    assert!(notes(&empty).contains("falls back to the static command tree"));
}

/// A GET's coverage claim is exactly its selector and its window, and a silent
/// one has to say so — three different silences, not one.
#[test]
fn a_get_with_no_replies_names_the_three_silences() {
    let silent = zenctl::render::GetReport {
        selector: "acme/zk2/**".into(),
        timeout_s: 5.0,
        elided: 0,
        answers: vec![],
    };
    let n = notes(&silent);
    assert!(n.contains("Nothing may hold it"), "{n}");
    assert!(n.contains("the three are different"), "{n}");
    let doc: serde_json::Value =
        serde_json::from_str(ndjson(&silent).lines().next().unwrap()).unwrap();
    assert_eq!(doc["selector"], "acme/zk2/**");
    // `5.0`: the seconds unification (#218) — see the budget window above.
    assert_eq!(doc["timeout_s"], 5.0);
}

/// A mock owner still publishes, so the plan is a dry run made visible
/// before anything is brought up — the `replay --dry-run` precedent — and
/// it says where the synthetic marker rides: the descriptor's `meta`, not a
/// sample's attachment (#612, FJ8a).
#[test]
fn a_gen_plan_states_every_key_and_where_the_marker_rides() {
    use zenkey_fleet::report::{GenInterface, GenPlan, GenPlanEntry, MemberSource, QosView};
    use zenkey_model::authoring::{Congestion, Kind, Priority, Reliability};
    let stream = GenPlanEntry {
        iface: "tc.netif.v1".into(),
        resource: "stream/bandwidth/{ns}/{iface}".into(),
        kind: Kind::Stream,
        key: "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/ns-1/iface-1".into(),
        values: [
            ("iface".to_owned(), vec!["iface-1".to_owned()]),
            ("ns".to_owned(), vec!["ns-1".to_owned()]),
        ]
        .into(),
        members: MemberSource::Default,
        declared: "json:BandwidthUpdate".into(),
        encoding: "application/json".into(),
        qos: Some(QosView {
            reliability: Reliability::BestEffort,
            congestion: Congestion::Drop,
            priority: Priority::Data,
            express: false,
        }),
        rate_hz: Some(1.0),
        events_cap: None,
        member_token: false,
        note: Some("synthetic members (name them with --member)".into()),
    };
    let op = GenPlanEntry {
        resource: "@op/diagnostics".into(),
        kind: Kind::Operation,
        key: "zk2/host-a/tc/tc.netif.v1/@op/diagnostics".into(),
        values: Default::default(),
        members: MemberSource::Fixed,
        declared: "json:DiagnosticsResponse".into(),
        qos: None,
        rate_hz: None,
        note: Some("answers each call with one synthesized response".into()),
        ..stream.clone()
    };
    let plan = GenPlan {
        address: "host-a/tc".into(),
        interfaces: vec![GenInterface {
            iface: "tc.netif.v1".into(),
            fingerprint: format!("sha256:{}", "4f".repeat(32)),
        }],
        duration_s: 10.0,
        seed: 42,
        marker: serde_json::json!({"synthetic": true, "tool": "zenctl gen", "seed": 42}),
        entries: vec![stream, op],
    };
    assert_data_eq!(
        table(&plan),
        str![[r#"
plan: a mock owner at host-a/tc implementing tc.netif.v1@sha256:4f4f4f4f4f4f4f4f…, for 10s (seed 42)
  zk2/host-a/tc/tc.netif.v1/stream/bandwidth/ns-1/iface-1 [json:BandwidthUpdate, application/json] 1.00 Hz, qos data/drop/best_effort
    ↳ synthetic members (name them with --member)
  zk2/host-a/tc/tc.netif.v1/@op/diagnostics [json:DiagnosticsResponse, application/json] answers calls
    ↳ answers each call with one synthesized response

"#]]
    );
    let n = notes(&plan);
    assert!(n.contains("meta carries the synthetic marker"), "{n}");
    assert!(n.contains("<param>-1"), "{n}");
    // The envelope counts the entries; the entries are the rows.
    let lines: Vec<serde_json::Value> = ndjson(&plan)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines[0]["report"], "gen-plan");
    assert_eq!(lines[0]["entries"], 2);
    assert_eq!(lines[0]["marker"]["synthetic"], true);
    assert_eq!(lines[1]["row"], "entry");

    let report = zenkey_fleet::report::GenReport {
        address: "host-a/tc".into(),
        instance: "0123456789abcdef".into(),
        duration_s: 5.0,
        entries: 12,
        sent: 240,
        calls: 3,
        failed: 2,
        first_errors: vec!["zk2/host-a/tc/tc.netif.v1/state/namespaces: clock ahead".into()],
    };
    assert_data_eq!(
        table(&report),
        str![[r#"
host-a/tc as instance 0123456789abcdef: sent 240 sample(s) over 5.0s across 12 entr(y|ies), answered 3 call(s); 2 not sent
  ✗  zk2/host-a/tc/tc.netif.v1/state/namespaces: clock ahead

"#]]
    );
    assert!(notes(&report).contains("counted, never skipped"));
}

/// Every `Render` impl is drawn somewhere in this file — the mechanical floor
/// #201 asks for, rather than a habit.
///
/// It reads the `FAMILY` constants out of `render/impls/` as text, because a
/// trait impl cannot be enumerated at runtime. That is coarser than reflection
/// and finer than nothing: adding a family fails here until somebody has
/// written down what it draws, which is the moment to write the snapshot.
///
/// `COVERED` is not a second source of truth — it is a checklist, and the
/// assertion is that the checklist and the code agree.
#[test]
fn every_render_impl_is_drawn_somewhere_in_this_file() {
    const COVERED: &[&str] = &[
        "acl-check",
        "acl-explain",
        "acl-plan",
        "admin-graph",
        "admin-routers",
        "bench",
        "cache",
        "cache-action",
        "compat",
        "conform",
        "context",
        "context-action",
        "context-list",
        "doctor",
        "expect",
        "field",
        "gen",
        "gen-plan",
        "get",
        "graph",
        "health",
        "hostid",
        "iface-list",
        "iface-show",
        "key-canon",
        "key-relation",
        "namespace-list",
        "operation",
        "probe",
        "rate",
        "record",
        "replay",
        "schema-check",
        "schema-show",
        "scout",
        "service-list",
        "service-show",
        "snapshot",
        "snapshot-diff",
        "state",
        "storage-check",
        "storage-explain",
        "storage-list",
        "storage-plan",
        "timeline",
        "why",
    ];

    fn families(dir: &std::path::Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("the impls directory") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                families(&path, out);
                continue;
            }
            let src = std::fs::read_to_string(&path).expect("an impl file");
            for line in src.lines() {
                if let Some(rest) = line.trim().strip_prefix("const FAMILY: &'static str = \"")
                    && let Some(name) = rest.split('"').next()
                {
                    out.push(name.to_string());
                }
            }
        }
    }

    let mut found = Vec::new();
    families(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/render/impls"),
        &mut found,
    );
    found.sort();
    // `watch` wraps another family and takes its rendering wholesale, so it
    // has no drawing of its own to pin; `cmd/watch.rs` owns its one test.
    assert_eq!(
        found, COVERED,
        "a Render impl was added or removed without the checklist in this test \
         moving with it — and the moment to write its snapshot is now, while \
         you still remember what it draws"
    );
}

/// The other half of the mechanical floor (RFC 13, v1.24): every family
/// whose verb subscribes or GETs states its observed scope — what was asked,
/// and over what window — as data (`Render::scope`), not only as prose in a
/// note. The checklist below is the list of observing families; adding an
/// observing verb means adding its fixture here, which is the moment to
/// decide what its scope claim is.
#[test]
fn every_observing_family_states_its_scope() {
    fn scoped<R: Render>(r: &R) -> zenctl::render::ObservedScope {
        r.scope().unwrap_or_else(|| {
            panic!(
                "{} subscribes or GETs and must state its observed scope",
                R::FAMILY
            )
        })
    }

    // Window-bearing subscribers.
    let s = scoped(&fx::expect_report());
    assert_eq!(s.asked, ["zk2/*/tc/tc.netif.v1/stream/bandwidth/*/*"]);
    assert_eq!(s.window_s, Some(5.0));
    let s = scoped(&fx::field_report());
    assert_eq!(s.window_s, Some(30.0));
    let s = scoped(&zenctl::render::RateView {
        report: &fx::rate_report(),
        bandwidth: false,
    });
    assert_eq!(s.window_s, Some(10.0));
    scoped(&fx::record_report());
    // The timeline's scope is every selector it watched, over the window;
    // a `.zrec` window has no `window_s` (nothing was asked, O4).
    let s = scoped(&fx::timeline_report_arrival());
    assert_eq!(s.asked, ["acme/zk2/**"]);
    assert_eq!(s.window_s, Some(10.0));

    // A snapshot's window is its collection span — the RFC 13 §4.4 fact
    // every rendering states (#219).
    let s = scoped(&fx::snapshot_report());
    assert_eq!(s.asked, ["acme/zk2/*/*/*/state/**"]);
    assert_eq!(s.window_s, Some(1.25));

    // The doctor's scope is what it read: presence in the namespace, the
    // admin space in none, and the owners' health.v1 state (#721, PF).
    let s = scoped(&fx::doctor_report());
    assert_eq!(
        s.asked,
        ["zk2/*/*/@zk/**", "@/*/router", "zk2/*/*/health.v1/state/**"]
    );
    assert_eq!(s.window_s, None);
    // `health` (#721, PF): presence, the readings' GET, the window's
    // subscriptions, over the window.
    let s = scoped(&fx::health_report());
    assert_eq!(s.asked.len(), 4);
    assert_eq!(s.window_s, Some(31.0));
    // `storage gen --check` sweeps the admin space once, no window.
    let s = scoped(&fx::storage_check());
    assert_eq!(s.asked, ["@/*/router/**/storage_manager/storages/**"]);
    assert_eq!(s.window_s, None);

    // GET-shaped asks: the wait is the window (R5/P1's `timeout_s`).
    // A probe subscribes like a consumer, and reads presence only to
    // attribute a silence: both are what it asked.
    let s = scoped(&fx::probe_report());
    assert_eq!(
        s.asked,
        [
            "zk2/host-a/tc/tc.netif.v1/state/interfaces/*/*",
            "zk2/host-a/tc/@zk/alive/tc.netif.v1/**"
        ]
    );
    assert_eq!(s.window_s, Some(5.0));
    let s = scoped(&zenctl::render::GetReport {
        selector: "acme/zk2/*/*/*/state/**".into(),
        timeout_s: 5.0,
        elided: 0,
        answers: vec![],
    });
    assert_eq!(s.asked, ["acme/zk2/*/*/*/state/**"]);
    scoped(&fx::bench_report());

    // Sweeps: asked is the claim; a one-shot sweep has no window.
    scoped(&fx::scout_report());
    let s = scoped(&fx::router_list());
    assert_eq!(s.window_s, None);
    let report = fx::topology();
    scoped(&zenctl::render::TopologyView { report: &report });
    // `check conform` (#703): what the suite put to the bus, over its
    // window.
    let s = scoped(&fx::conform_report());
    assert_eq!(s.asked.len(), 3);
    assert_eq!(s.window_s, Some(5.0));
    // `why` (#702): what the ladder put to the bus — presence, the key, the
    // archives after a silence — and no window unless a stream was heard.
    let s = scoped(&fx::why_report_silent());
    assert_eq!(
        s.asked,
        [
            "zk2/host-a/tc/@zk/**",
            "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0",
            "zk2/*/*/@zk/alive/archive.v1/**"
        ]
    );
    assert_eq!(s.window_s, None);
    // With the instance join (#705), the presence selector it read too.
    let report = fx::topology_with_instances();
    let s = scoped(&zenctl::render::TopologyView { report: &report });
    assert_eq!(s.asked, ["@/*/router", "zk2/*/*/@zk/**"]);
    // zk2's acts and reads (#612, FJ5): the keys a call or a state GET
    // went out on, over its reply wait.
    let s = scoped(&actfx::value());
    assert_eq!(s.asked, ["zk2/host-a/tc/tc.netif.v1/@op/diagnostics"]);
    assert_eq!(s.window_s, Some(5.0));
    let s = scoped(&actfx::state(
        zenkey_fleet::report::StateReading::Current,
        vec![],
    ));
    assert_eq!(s.asked, ["zk2/host-a/tc/tc.netif.v1/state/interfaces/*/*"]);
    // zk2's presence reads (#612, FJ4): one liveliness selector, no window —
    // namespaced for the resolved verbs, across namespaces for `namespace
    // list`.
    let s = scoped(&zk2fx::catalog().services());
    assert_eq!(s.asked, ["zk2/*/*/@zk/**"]);
    assert_eq!(s.window_s, None);
    scoped(&zk2fx::catalog().service(&"host-a/tc".parse().expect("an address")));
    scoped(&zk2fx::catalog().ifaces());
    scoped(&zk2fx::iface_view());
    scoped(&zk2fx::catalog().graph());
    let s = scoped(&zk2fx::namespace_listing());
    assert_eq!(s.asked, ["**/zk2/*/*/@zk/instance/*"]);

    // And the deliberate negatives: replay *publishes*; it observes nothing,
    // so a scope claim would be an invented observation — and a snapshot
    // diff opens no session at all (its two spans ride the envelope).
    assert!(fx::replay_report().scope().is_none());
    assert!(fx::snapshot_diff().scope().is_none());
    // A revision's schemas and a comparison of two read bundles and files:
    // whatever was retrieved to get them, the reports observe nothing.
    assert!(zk2fx::schema_view().scope().is_none());
    assert!(zk2fx::compat_report().scope().is_none());
}

/// The context family, which until #242 had no machine surface at all.
///
/// The two things worth pinning are the three-state base — a stored empty
/// base (`""`, the legal bus-root deployment) must not read like an unset one
/// — and the *flat* json envelope, because the whole point is
/// `zenctl context show --format json | jq -r .base`.
#[test]
fn a_context_list_keeps_an_empty_base_distinct_from_an_absent_one() {
    let list = zenctl::render::ContextList {
        path: "/home/u/.config/zenkey-explorer/config.toml".into(),
        contexts: vec![
            zenctl::render::ContextRow {
                name: "lab".into(),
                current: true,
                base: Some("zensight".into()),
                connect: vec!["tcp/127.0.0.1:7447".into()],
            },
            zenctl::render::ContextRow {
                name: "root".into(),
                current: false,
                // A deployment, not an absence.
                base: Some(String::new()),
                connect: vec![],
            },
            zenctl::render::ContextRow {
                name: "unset".into(),
                current: false,
                base: None,
                connect: vec![],
            },
        ],
    };
    assert_data_eq!(
        table(&list),
        str![[r#"
* lab    base=zensight  connect=["tcp/127.0.0.1:7447"]
  root   base=""  connect=[]
  unset  base=-  connect=[]

"#]]
    );
    let lines: Vec<serde_json::Value> = ndjson(&list)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines[0]["contexts"], 3);
    assert_eq!(
        lines[2]["base"], "",
        "the empty base is carried, not dropped"
    );
    assert!(
        lines[3].get("base").is_none(),
        "an unset base is absent, never null (RFC 09 §5.1 O4)"
    );
}

#[test]
fn a_context_show_puts_the_settings_flat_on_the_envelope() {
    let show = zenctl::render::ContextShow {
        name: "lab".into(),
        current: true,
        context: zenkey_explorer_config::StoredContext {
            base: Some("zensight".into()),
            connect: vec!["tcp/127.0.0.1:7447".into()],
            timeout: Some(30),
            ..Default::default()
        },
    };
    assert_data_eq!(
        table(&show),
        str![[r#"
base = "zensight"
connect = ["tcp/127.0.0.1:7447"]
timeout = 30

"#]]
    );
    let doc: serde_json::Value =
        serde_json::from_str(ndjson(&show).lines().next().unwrap()).unwrap();
    assert_eq!(
        doc["base"], "zensight",
        "`jq -r .base`, not `.context.base`"
    );
    assert_eq!(doc["name"], "lab");
    assert_eq!(doc["current"], true);
}

/// The four file-changing verbs answer "it happened", which is an envelope and
/// a sentence — no rows, and nothing on the table.
#[test]
fn a_context_action_is_an_envelope_and_a_sentence() {
    let created = zenctl::render::ContextAction {
        action: "created",
        name: "lab".into(),
        current: true,
    };
    assert_eq!(table(&created), "", "nothing to draw");
    assert!(notes(&created).contains(r#"created context "lab" (selected)"#));
    let removed = zenctl::render::ContextAction {
        action: "removed",
        name: "lab".into(),
        current: false,
    };
    assert!(
        !notes(&removed).contains("selected"),
        "a removed context is not the selected one"
    );
}

/// `cache clear` has two outcomes that used to differ only in prose. `existed`
/// is what a script branches on: removing a cache that was not there is the
/// desired end state, and not the same event.
#[test]
fn a_cache_clear_says_whether_there_was_anything_to_clear() {
    let removed = zenctl::render::CacheAction {
        action: "cleared",
        dir: "/home/u/.cache/zenkey-explorer/lab/slices".into(),
        services: None,
        existed: true,
    };
    assert!(notes(&removed).starts_with("removed /home/u"));
    let absent = zenctl::render::CacheAction {
        action: "cleared",
        dir: "/home/u/.cache/zenkey-explorer/lab/slices".into(),
        services: None,
        existed: false,
    };
    assert!(notes(&absent).contains("does not exist — nothing to clear"));
    let doc: serde_json::Value =
        serde_json::from_str(ndjson(&absent).lines().next().unwrap()).unwrap();
    assert_eq!(doc["existed"], false);
    assert!(
        doc.get("services").is_none(),
        "clear counts nothing — absent, not zero (RFC 09 §5.1 O4)"
    );
}

// ── The storage-plan families (#393) ──────────────────────────────────────

/// The plan as a table: one row per volume and storage, the derivation and
/// every warning as detail lines, the refusal as a note — and, on the wire,
/// the same facts under `row` tags.
#[test]
fn a_storage_plan_shows_its_derivations_and_names_its_refusals() {
    assert_data_eq!(
        table(&fx::storage_plan()),
        str![[r#"
storage plan for namespace "acme"

volumes:
  fs        fs        durable · latest
  influxdb  influxdb  durable · all     url="http://localhost:8086"

storages:
  events      acme/zk2/*/*/*/events/**       fs (latest)     replicated, complete
    strip acme/zk2
    gc lifespan 86400 s (period 30 s): zenoh's default 86400 s — no contract declares a tombstone lifetime to derive one from
    ! overlap: overlaps links (acme/zk2/*/*/*/events/link/**): a GET under both selectors is answered by both (RFC 09 §2)
  links       acme/zk2/*/*/*/events/link/**  fs (latest)
    strip acme/zk2
    gc lifespan 3600 s (period 30 s): declared gc_lifespan_s 3600
    ! complete_refused: complete = true refused: it is not replicated — emitted as false (RFC 09 §2.2)
    ! overlap: overlaps events (acme/zk2/*/*/*/events/**): a GET under both selectors is answered by both (RFC 09 §2)
  timeseries  acme/zk2/*/*/*/stream/**       influxdb (all)
    strip acme/zk2
    gc lifespan 86400 s (period 30 s): zenoh's default 86400 s — no contract declares a tombstone lifetime to derive one from
    ! retention_is_the_databases: retention is the database's policy, not zenoh config (RFC 09 §2.3)

"#]]
    );
    let notes = notes(&fx::storage_plan());
    assert!(notes.contains("refused storage plant:"), "{notes}");
    assert!(notes.contains("3 storage(s) on 2 volume(s) planned, 1 refused"));
    let out = ndjson(&fx::storage_plan());
    let mut lines = out.lines();
    let envelope: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(envelope["report"], "storage-plan");
    assert!(
        envelope.get("registry").is_none(),
        "v1's registry claim left at FJ9"
    );
    assert!(
        envelope.get("storages").is_none(),
        "rows do not ride the envelope"
    );
    let kinds: Vec<String> = lines
        .map(|l| {
            serde_json::from_str::<serde_json::Value>(l).unwrap()["row"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "volume", "volume", "storage", "storage", "storage", "refusal"
        ]
    );
}

/// The check: one line per finding, the unjudged comparison as a coverage
/// note, and the empty admin sweep as a non-verdict rather than a pass.
#[test]
fn a_storage_check_draws_each_finding_and_keeps_unjudged_apart() {
    assert_data_eq!(
        table(&fx::storage_check()),
        str![[r#"
storage check for namespace "acme" against the admin space: 3 planned, 3 observed row(s) — 4 finding(s)
  ✗ events@aabbccdd  strip_prefix differs          planned acme/zk2, observed acme
  ✗ events@aabbccdd  gc.lifespan below the plan's  planned 86400, observed 600
  ✗ timeseries       missing                       planned acme/zk2/*/*/*/stream/**
  ✗ state@aabbccdd   extra                         observed acme/zk2/*/*/*/state/**

"#]]
    );
    assert!(notes(&fx::storage_check()).contains("not judged, which is not the same as agreeing"));
    let out = ndjson(&fx::storage_check());
    let envelope: serde_json::Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    assert_eq!(envelope["judgement"]["answer"], "established");
    assert_eq!(out.lines().count(), 5, "envelope + four findings");

    assert_data_eq!(
        table(&fx::storage_check_unobservable()),
        str![[r#"
storage check for namespace "acme" against the admin space: 3 planned, 0 observed row(s) — no verdict — the admin space answered no storages

"#]]
    );
    assert!(notes(&fx::storage_check_unobservable()).contains("RFC 05 §3.1"));
}

/// #704: a plan derived from an enrollment names where each union storage
/// came from, and every list the derivation keeps — interfaces whose
/// contract was not given, interfaces without events, archives never
/// planned, the S4 refusal — reaches the notes, a script included, each
/// its own sentence; the derived storage's row carries `derived`.
#[test]
fn an_enrolled_storage_plan_names_its_union_storages_and_what_it_left_out() {
    let plan = fx::storage_plan_enrolled();
    assert_data_eq!(
        table(&plan),
        str![[r#"
storage plan for namespace "acme"

volumes:
  memory  memory  volatile · latest
    ! implicit_volume: no deployment file names an [events] volume: volatile (RFC 09 §2.1)

storages:
  events-tc.netem.v1-applied  acme/zk2/*/*/tc.netem.v1/events/applied/*  memory (latest)
    union storage for tc.netem.v1 events/applied, implemented by h-20609002f7b6/tc, h-3fa9c2d41b7e/tc
    strip acme/zk2
    gc lifespan 604800 s (period 30 s): the contract's retention for tc.netem.v1 events/applied, 7d (604800 s, spec §2.6)
    ! retention_not_enforced: the contract's retention (7d) bounds a replay, not this storage (spec §2.6)

"#]]
    );
    let said = notes(&plan);
    for want in [
        "refused storage latest:",
        "from the enrollment: 1 union storage(s)",
        "not planned: nav.v2",
        "tc.netif.v1 declare(s) no event resource",
        "archive ground/archive: not planned",
    ] {
        assert!(said.contains(want), "{want:?} in {said}");
    }
    let lines: Vec<serde_json::Value> = ndjson(&plan)
        .lines()
        .map(|l| serde_json::from_str(l).expect("one object per line"))
        .collect();
    let envelope = &lines[0];
    assert_eq!(
        envelope["enrollment"]["archives"],
        serde_json::json!(["ground/archive"])
    );
    let notes_text = envelope["notes"].to_string();
    assert!(
        notes_text.contains("not planned: nav.v2") && notes_text.contains("archive ground/archive"),
        "a script reads what was left out: {envelope}"
    );
    let storage = lines.iter().find(|l| l["row"] == "storage").expect("a row");
    assert_eq!(storage["derived"]["resource"], "events/applied");
    let refusal = lines.iter().find(|l| l["row"] == "refusal").expect("a row");
    assert_eq!(refusal["cite"], "spec §4.2 S4");
}

/// #704: a check against a router file names the file, carries no zid on
/// its findings, keeps a storage on owners' state its own kind, and claims
/// no scope on the bus — a file is read, not observed.
#[test]
fn a_storage_check_against_a_file_names_it_and_its_s4_finding() {
    let check = fx::storage_check_file();
    assert_data_eq!(
        table(&check),
        str![[r#"
storage check for namespace "acme" against router.json5: 1 planned, 2 observed row(s) — 3 finding(s)
  ✗ events-tc.netem.v1-applied  gc.lifespan below the plan's  planned 604800, observed 86400
  ✗ latest                      extra                         observed acme/zk2/*/*/*/state/**
  ✗ latest                      on owners' state (S4)         observed acme/zk2/*/*/*/state/** (intersects `acme/zk2/*/*/*/state/**`, spec §4.2 S4)

"#]]
    );
    assert!(notes(&check).contains("what zenohd would run"));
    assert!(check.scope().is_none(), "a file is not a bus observation");
    let out = ndjson(&check);
    let envelope: serde_json::Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    assert_eq!(envelope["source"], "file");
    assert!(out.contains("\"kind\":\"on_owner_state\""), "{out}");
}

/// `--explain`: the taker with its reason, or the reason there is none.
#[test]
fn a_storage_explain_names_the_taker_or_the_reason() {
    assert_data_eq!(
        table(&fx::storage_explain()),
        str![[r#"
acme/zk2/host-a/tc/tc.netif.v1/events/reset/01k0
  → events  acme/zk2/*/*/*/events/**  includes it
      its selector under namespace "acme": acme/zk2/*/*/*/events/** includes every key it names; stored under strip_prefix "acme/zk2" on volume fs (latest)

"#]]
    );
    assert_data_eq!(
        table(&fx::storage_explain_none()),
        str![[r#"
acme/plant/line-1/temp
  none: no planned storage's selector includes it; refused storage(s) plant would have

"#]]
    );
    let out = ndjson(&fx::storage_explain_none());
    let envelope: serde_json::Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    assert_eq!(envelope["refused_takers"], serde_json::json!(["plant"]));
    assert_eq!(out.lines().count(), 1, "no takers, no rows");
}

// ── The consumers join (#224) ─────────────────────────────────────────────

// ── The metrics surface (#228) ───────────────────────────────────────────────

// ── zk2 (#612, FJ4) ────────────────────────────────────────────────────────
//
// The zk2 reports are built here, from the repository's own contracts
// (`examples/zk2/.history`) and a presence read written out by hand, through
// the fleet's real projections (`Catalog`, `Revision::schema_view`,
// `compat`): `zenkey-report-fixtures` sees neither `zenkey-model` nor the
// catalog, and a hand-built view could not catch the projection drifting
// from what it renders.

mod zk2fx {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use serde_json::json;
    use zenkey_fleet::report::{CompatReport, IfaceView, NamespaceListing, SchemaView};
    use zenkey_fleet::{Catalog, ContractSet, DescriptorRead, ObservedPresence};
    use zenkey_model::descriptor::Descriptor;

    fn examples() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2")
    }

    /// Every published revision, as `--contracts examples/zk2/.history`.
    pub fn contracts() -> ContractSet {
        let (set, problems) = ContractSet::load_path(&examples().join(".history"));
        assert!(problems.is_empty(), "{problems:?}");
        set
    }

    /// The one revision `contracts()` holds of `iface`.
    pub fn fp(iface: &str) -> String {
        let id = iface.parse().expect("an interface");
        contracts()
            .of_iface(&id)
            .next()
            .unwrap_or_else(|| panic!("{iface} is published"))
            .fingerprint()
            .to_string()
    }

    fn fp16(iface: &str) -> String {
        fp(iface)["sha256:".len()..][..16].to_owned()
    }

    fn descriptor(service: &str, instance: &str, rest: serde_json::Value) -> Descriptor {
        let mut v = json!({
            "format": "zk2-descriptor/0.1",
            "service": service,
            "instance": instance,
            "interfaces": [],
        });
        v.as_object_mut()
            .expect("an object")
            .extend(rest.as_object().expect("an object").clone());
        serde_json::from_value(v).expect("a descriptor")
    }

    pub const TC_A: &str = "8f3a5c2e9b1d4f70";
    pub const CAM: &str = "0000000000000002";
    pub const GUI: &str = "0000000000000003";
    pub const TC_B: &str = "0000000000000004";

    /// One presence read: host-a's tc backend by token and descriptor,
    /// a camera serving `health.v1` in the tokenless set, the tcgui
    /// frontend binding two roles (one of which selects nothing), and
    /// host-b's tc backend, whose descriptor did not answer.
    pub fn catalog() -> Catalog {
        let keys: Vec<String> = vec![
            format!("zk2/host-a/tc/@zk/instance/{TC_A}"),
            format!(
                "zk2/host-a/tc/@zk/alive/tc.netif.v1/{TC_A}/{}",
                fp16("tc.netif.v1")
            ),
            format!(
                "zk2/host-a/tc/@zk/alive/tc.netem.v1/{TC_A}/{}",
                fp16("tc.netem.v1")
            ),
            format!("zk2/vehicle-01/cam-front/@zk/instance/{CAM}"),
            format!(
                "zk2/vehicle-01/cam-front/@zk/alive/camera.v1/{CAM}/{}",
                fp16("camera.v1")
            ),
            format!("zk2/ws-01/tcgui-frontend/@zk/instance/{GUI}"),
            format!("zk2/host-b/tc/@zk/instance/{TC_B}"),
            format!(
                "zk2/host-b/tc/@zk/alive/tc.netif.v1/{TC_B}/{}",
                fp16("tc.netif.v1")
            ),
        ];
        let mut o = ObservedPresence::from_keys("zk2/*/*/@zk/**", &keys, true);
        let served = |addr: &str, inst: &str, d: Descriptor| {
            (
                (addr.parse().expect("addr"), inst.parse().expect("id")),
                DescriptorRead::Served(Box::new(d)),
            )
        };
        o.descriptors = Some(BTreeMap::from([
            served(
                "host-a/tc",
                TC_A,
                descriptor(
                    "host-a/tc",
                    TC_A,
                    json!({
                        "interfaces": [
                            {
                                "iface": "tc.netif.v1",
                                "contract": fp("tc.netif.v1"),
                                "minor": 0,
                                "unavailable": [{"resource": "@op/diagnostics", "cause": "config", "reason": "disabled here"}],
                                "cardinality": {"state/interfaces/{ns}/{iface}": 8},
                            },
                            {"iface": "tc.netem.v1", "contract": fp("tc.netem.v1"), "minor": 0},
                        ],
                        "capabilities": ["shaping"],
                    }),
                ),
            ),
            served(
                "vehicle-01/cam-front",
                CAM,
                descriptor(
                    "vehicle-01/cam-front",
                    CAM,
                    json!({"interfaces": [
                        {"iface": "camera.v1", "contract": fp("camera.v1"), "minor": 0},
                        {"iface": "health.v1", "contract": fp("health.v1"), "minor": 0, "token": false},
                    ]}),
                ),
            ),
            served(
                "ws-01/tcgui-frontend",
                GUI,
                descriptor(
                    "ws-01/tcgui-frontend",
                    GUI,
                    json!({"requires": [
                        {"role": "netif", "interface": "tc.netif.v1", "bindings": ["*/tc"]},
                        {"role": "scenario", "interface": "tc.scenario.v1", "bindings": ["*/tc"]},
                    ]}),
                ),
            ),
            (
                (
                    "host-b/tc".parse().expect("addr"),
                    TC_B.parse().expect("id"),
                ),
                DescriptorRead::Silent,
            ),
        ]));
        Catalog::new(&o)
    }

    /// `iface show tc.netif.v1`, its revision held from `--contracts`.
    pub fn iface_view() -> IfaceView {
        catalog().iface(&"tc.netif.v1".parse().expect("iface"), &contracts())
    }

    /// `schema show tc.netif.v1 bandwidth/{ns}/{iface}`.
    pub fn schema_view() -> SchemaView {
        let set = contracts();
        let id = "tc.netif.v1".parse().expect("iface");
        let r = set.of_iface(&id).next().expect("published");
        r.schema_view(Some("bandwidth/{ns}/{iface}"), true)
            .expect("the resource is declared")
    }

    /// `compat tc.netif.v1 tests/cmd/fixtures/zk2/review/tc.netif.v1.toml
    /// --contracts examples/zk2/.history`: one review change.
    pub fn compat_report() -> CompatReport {
        use zenkey_fleet::report::{CompatSide, ContractSource};
        let set = contracts();
        let id = "tc.netif.v1".parse().expect("iface");
        let old = set.of_iface(&id).next().expect("published");
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/cmd/fixtures/zk2/review/tc.netif.v1.toml");
        let l = zenkey_model::contract::load_path(&path);
        let new = l.contract.unwrap_or_else(|| panic!("{}", l.report));
        zenkey_fleet::compat(
            (
                CompatSide {
                    input: "tc.netif.v1".into(),
                    source: ContractSource::History,
                    iface: "tc.netif.v1".into(),
                    fingerprint: old.fingerprint().to_string(),
                },
                &zenkey_model::compat::Revision::of_bundle(old.bundle()),
            ),
            (
                CompatSide {
                    input: "tests/cmd/fixtures/zk2/review/tc.netif.v1.toml".into(),
                    source: ContractSource::File,
                    iface: "tc.netif.v1".into(),
                    fingerprint: zenkey_model::canonical::Fingerprint::of(&new).to_string(),
                },
                &zenkey_model::compat::Revision::of(&new),
            ),
        )
    }

    /// `namespace list` over the bus root and one two-chunk namespace.
    pub fn namespace_listing() -> NamespaceListing {
        let keys: Vec<String> = vec![
            format!("zk2/host-a/tc/@zk/instance/{TC_A}"),
            format!("site/prod/zk2/host-a/tc/@zk/instance/{TC_A}"),
            format!("site/prod/zk2/host-b/tc/@zk/instance/{TC_B}"),
            format!("site/prod/zk2/host-b/tc/@zk/instance/{CAM}"),
        ];
        zenkey_fleet::namespaces(zenkey_fleet::NAMESPACE_SELECTOR, &keys, true)
    }
}

#[test]
fn a_service_listing_keeps_the_token_and_the_descriptor_apart() {
    let listing = zk2fx::catalog().services();
    assert_data_eq!(
        table(&listing),
        str![[r#"
host-a/tc
  instance 8f3a5c2e9b1d4f70                    descriptor served
    tc.netem.v1              30b491116ef6a534  sha256:30b491116ef6a534…  minor 0
    tc.netif.v1              4f531ebec4c4eee0  sha256:4f531ebec4c4eee0…  minor 0

host-b/tc
  instance 0000000000000004                    descriptor silent
    tc.netif.v1              4f531ebec4c4eee0  —

vehicle-01/cam-front
  instance 0000000000000002                    descriptor served
    camera.v1                0f66b421dc6decf2  sha256:0f66b421dc6decf2…  minor 0
    health.v1                tokenless         sha256:e9dbcdcb2fc325ca…  minor 0

ws-01/tcgui-frontend
  instance 0000000000000003                    descriptor served

"#]]
    );
    assert_data_eq!(
        notes(&listing),
        str![[r#"
4 service(s), 4 instance(s).
one service in full: zenctl service show <system>/<service>

"#]]
    );
    // One row per instance, tagged and carrying its address; the envelope
    // leads with the completeness claim.
    let out = ndjson(&listing);
    let first: serde_json::Value =
        serde_json::from_str(out.lines().next().expect("an envelope")).expect("json");
    assert_eq!(first["report"], "service-list");
    assert_eq!(first["complete"], true);
    let rows: Vec<serde_json::Value> = out
        .lines()
        .skip(1)
        .map(|l| serde_json::from_str(l).expect("json"))
        .collect();
    assert_eq!(rows.len(), 4, "{out}");
    assert!(
        rows.iter()
            .all(|r| r["row"] == "instance" && r["address"].is_string())
    );
}

#[test]
fn a_service_view_lays_its_descriptor_out() {
    let view = zk2fx::catalog().service(&"host-a/tc".parse().expect("an address"));
    assert_data_eq!(
        table(&view),
        str![[r#"
host-a/tc  instance 8f3a5c2e9b1d4f70
  descriptor: served (zk2-descriptor/0.1)
  interfaces
    tc.netem.v1  30b491116ef6a534  sha256:30b491116ef6a534a3533741dc4003fe332c52ec84fcadcbd5dfa6f1be866245  minor 0
    tc.netif.v1  4f531ebec4c4eee0  sha256:4f531ebec4c4eee015c18f11aefbce9138b413cfdca6bf6b777ea0eb6f43b4e0  minor 0
      unavailable @op/diagnostics (config): disabled here
      cardinality state/interfaces/{ns}/{iface} ≤ 8
  capabilities: shaping

"#]]
    );
    // An address presence did not show: no instance, and the note says
    // why that is not a verdict — and that the verb exits 2.
    let none = zk2fx::catalog().service(&"host-z/tc".parse().expect("an address"));
    assert!(table(&none).is_empty());
    let n = notes(&none);
    assert!(
        n.contains("silence is not a verdict, and this exits 2"),
        "{n}"
    );
}

#[test]
fn an_iface_listing_names_providers_consumers_and_revisions() {
    let listing = zk2fx::catalog().ifaces();
    assert_data_eq!(
        table(&listing),
        str![[r#"
INTERFACE       PROVIDERS                          CONSUMERS             REVISIONS
camera.v1       vehicle-01/cam-front                                     sha256:0f66b421dc6decf2…
health.v1       vehicle-01/cam-front  [tokenless]                        sha256:e9dbcdcb2fc325ca…
tc.netem.v1     host-a/tc                                                sha256:30b491116ef6a534…
tc.netif.v1     host-a/tc, host-b/tc               ws-01/tcgui-frontend  sha256:4f531ebec4c4eee0…
tc.scenario.v1                                     ws-01/tcgui-frontend

"#]]
    );
    assert_data_eq!(
        notes(&listing),
        str![[r#"
5 interface(s).
one interface in full: zenctl iface show <iface>[@<fingerprint>]
1 instance(s) did not serve a readable descriptor (host-b/tc@0000000000000004 silent): their roles, and any interface they provide without a token, are missing here (spec §8.1)

"#]]
    );
}

#[test]
fn an_iface_view_shows_exposure_and_the_contract_resources() {
    let view = zk2fx::iface_view();
    assert_data_eq!(
        table(&view),
        str![[r#"
tc.netif.v1

providers
  host-a/tc@8f3a5c2e9b1d4f70  4f531ebec4c4eee0  sha256:4f531ebec4c4eee0…  minor 0
    exposes @op/interfaces/{ns}/{iface}/set, state/interfaces/{ns}/{iface}, state/namespaces, stream/bandwidth/{ns}/{iface}
    unavailable @op/diagnostics (config): disabled here
    cardinality state/interfaces/{ns}/{iface} ≤ 8
  host-b/tc@0000000000000004  4f531ebec4c4eee0  —
    exposes — (needs its descriptor and its revision in hand)

consumers
  ws-01/tcgui-frontend@0000000000000003  role netif  ← */tc

revision sha256:4f531ebec4c4eee015c18f11aefbce9138b413cfdca6bf6b777ea0eb6f43b4e0
  named by host-a/tc@8f3a5c2e9b1d4f70
  contract: held (history)
  uses freshness.v1
  resources
    @op/diagnostics                  operation  json:DiagnosticsRequest → json:DiagnosticsResponse                           fanout allowed serving exclusive replies one interactive_high idempotent
    @op/interfaces/{ns}/{iface}/set  operation  json:InterfaceControlRequest → json:InterfaceControlResponse | json:TcError  fanout forbidden serving exclusive replies one interactive_high
      cardinality ≤ 1024
    state/interfaces/{ns}/{iface}    state      json:NetworkInterface                                                        reliable block data
      cardinality ≤ 1024
    state/namespaces                 state      json:Namespaces                                                              reliable block data
    stream/bandwidth/{ns}/{iface}    stream     json:BandwidthUpdate                                                         best_effort drop data
      cardinality ≤ 1024
  schemas
    tc  jsonschema  sha256:b1e64344ce96a42218cc0ed0f45d72d6f601c46c6639d39c67a78e686d28054e

"#]]
    );
    let out = ndjson(&view);
    for kind in ["provider", "consumer", "revision", "undescribed"] {
        assert!(
            out.contains(&format!(r#""row":"{kind}""#)),
            "{kind} rows are tagged: {out}"
        );
    }
}

#[test]
fn a_graph_draws_bindings_and_the_roles_that_select_nothing() {
    let graph = zk2fx::catalog().graph();
    assert_data_eq!(
        table(&graph),
        str![[r#"
bindings
  ws-01/tcgui-frontend  netif  tc.netif.v1  → host-a/tc
  ws-01/tcgui-frontend  netif  tc.netif.v1  → host-b/tc

roles that select no provider present now
  ws-01/tcgui-frontend  scenario  tc.scenario.v1  ← */tc

services
  host-a/tc             1 instance(s)  provides tc.netem.v1, tc.netif.v1
  host-b/tc             1 instance(s)  provides tc.netif.v1
  vehicle-01/cam-front  1 instance(s)  provides camera.v1, health.v1
  ws-01/tcgui-frontend  1 instance(s)  provides nothing

"#]]
    );
    assert_data_eq!(
        notes(&graph),
        str![[r#"
4 service(s), 2 binding(s).
edges come from declared bindings and present providers, never from traffic; a role with no edge is shown, and judging it is `doctor`'s
1 instance(s) did not serve a readable descriptor (host-b/tc@0000000000000004 silent): their roles, and any interface they provide without a token, are missing here (spec §8.1)

"#]]
    );
    // A plain comparison: Graphviz's `\n` is a backslash, which a snapshot
    // would normalize as a path separator.
    assert_eq!(
        zenctl::render::graph_dot(&graph),
        r#"digraph zk2 {
  rankdir=LR;
  node [shape=box];
  "host-a/tc" [label="host-a/tc\ntc.netem.v1, tc.netif.v1"];
  "host-b/tc" [label="host-b/tc\ntc.netif.v1"];
  "vehicle-01/cam-front" [label="vehicle-01/cam-front\ncamera.v1, health.v1"];
  "ws-01/tcgui-frontend" [label="ws-01/tcgui-frontend"];
  "host-a/tc" -> "ws-01/tcgui-frontend" [label="netif (tc.netif.v1)"];
  "host-b/tc" -> "ws-01/tcgui-frontend" [label="netif (tc.netif.v1)"];
  unbound0 [shape=point];
  unbound0 -> "ws-01/tcgui-frontend" [style=dashed, label="scenario (tc.scenario.v1) ← */tc: no provider"];
}
"#
    );
}

#[test]
fn a_schema_view_lists_members_then_documents() {
    let view = zk2fx::schema_view();
    let t = table(&view);
    // The member rows are pinned; the JSON Schema document is the bundle's
    // own and long, so only its presence is.
    assert_data_eq!(
        t.lines().take(2).collect::<Vec<_>>().join("\n"),
        str![[r#"
tc.netif.v1 sha256:4f531ebec4c4eee015c18f11aefbce9138b413cfdca6bf6b777ea0eb6f43b4e0  (history)
  stream/bandwidth/{ns}/{iface}  type  json:BandwidthUpdate  sha256:b1e64344ce96a422…
"#]]
    );
    assert!(t.contains("\"BandwidthUpdate\""), "{t}");
    let out = ndjson(&view);
    assert!(out.contains(r#""row":"artifact""#), "{out}");
}

#[test]
fn a_compat_report_names_each_side_and_its_class() {
    let report = zk2fx::compat_report();
    assert_data_eq!(
        table(&report),
        str![[r#"
old  tc.netif.v1  sha256:4f531ebec4c4eee0…  history  tc.netif.v1
new  tc.netif.v1  sha256:94b92cc2ff1b030a…  file     tests/cmd/fixtures/zk2/review/tc.netif.v1.toml

review  cardinality_changed  resources."bandwidth/{ns}/{iface}"
  `cardinality` changed

review

"#]]
    );
    // Parsed rather than snapshotted: the finding's `at` carries escaped
    // quotes, and a snapshot normalizes backslashes as path separators.
    let lines: Vec<serde_json::Value> = ndjson(&report)
        .lines()
        .map(|l| serde_json::from_str(l).expect("json"))
        .collect();
    assert_eq!(
        lines,
        [
            serde_json::json!({
                "report": "compat",
                "class": "review",
                "old": {
                    "input": "tc.netif.v1",
                    "source": "history",
                    "iface": "tc.netif.v1",
                    "fingerprint": zk2fx::fp("tc.netif.v1"),
                },
                "new": {
                    "input": "tests/cmd/fixtures/zk2/review/tc.netif.v1.toml",
                    "source": "file",
                    "iface": "tc.netif.v1",
                    "fingerprint": report.new.fingerprint,
                },
            }),
            serde_json::json!({
                "row": "finding",
                "class": "review",
                "rule": "cardinality_changed",
                "at": "resources.\"bandwidth/{ns}/{iface}\"",
                "detail": "`cardinality` changed",
            }),
        ]
    );
}

#[test]
fn a_namespace_listing_spells_the_bus_root() {
    let listing = zk2fx::namespace_listing();
    assert_data_eq!(
        table(&listing),
        str![[r#"
(empty)    1 service(s)  1 instance(s)  host-a/tc
site/prod  2 service(s)  3 instance(s)  host-a/tc, host-b/tc

"#]]
    );
    assert_data_eq!(
        ndjson(&listing),
        str![[r#"
{"complete":true,"report":"namespace-list","selector":"**/zk2/*/*/@zk/instance/*"}
{"instances":1,"namespace":"","row":"namespace","services":["host-a/tc"]}
{"instances":3,"namespace":"site/prod","row":"namespace","services":["host-a/tc","host-b/tc"]}

"#]]
    );
}

/// The completeness claim (spec §8.1) rides every presence family, in
/// every format: a read that ran to its timeout is possibly incomplete,
/// never a quiet short list.
#[test]
fn an_incomplete_presence_read_says_so_in_every_format() {
    let mut listing = zk2fx::catalog().services();
    listing.complete = false;
    let n = notes(&listing);
    assert!(n.contains("possibly incomplete"), "{n}");
    let out = ndjson(&listing);
    let envelope: serde_json::Value =
        serde_json::from_str(out.lines().next().expect("an envelope")).expect("json");
    assert_eq!(envelope["complete"], false);
    assert!(
        envelope["notes"]
            .as_array()
            .is_some_and(|ns| ns.iter().any(|n| n["text"]
                .as_str()
                .is_some_and(|t| t.contains("possibly incomplete")))),
        "{envelope}"
    );
    let mut graph = zk2fx::catalog().graph();
    graph.complete = false;
    assert!(zenctl::render::graph_dot(&graph).contains("possibly incomplete"));
}

// ── zk2's acts and reads (#612, FJ5) ─────────────────────────────────────────

mod actfx {
    use std::collections::BTreeMap;

    use serde_json::json;
    use zenkey_fleet::report::{
        CallMode, EnvelopeView, OperationAnswer, OperationReport, PayloadRendering,
        PresenceAttribution, Rendered, ReplierView, RepliesView, ResolvedResource,
        SelectionPresence, SilenceView, Stamp, StateReading, StateReport, StateRow, StateValue,
        Unresolved,
    };

    pub fn fp() -> String {
        format!("sha256:{}", "4f53".repeat(16))
    }

    pub fn reply(
        key: &str,
        member: &str,
        resource: &str,
        value: serde_json::Value,
    ) -> PayloadRendering {
        PayloadRendering {
            key: key.into(),
            size: serde_json::to_vec(&value).expect("json").len(),
            resource: Some(ResolvedResource {
                iface: "tc.netif.v1".into(),
                fingerprint: fp(),
                resource: resource.into(),
                member: member.into(),
                values: BTreeMap::new(),
            }),
            rendered: Rendered::Value {
                declared: "json:DiagnosticsResponse".into(),
                value,
            },
        }
    }

    pub fn call(answer: OperationAnswer, mode: CallMode, address: &str) -> OperationReport {
        OperationReport {
            address: address.into(),
            iface: "tc.netif.v1".into(),
            fingerprint: fp(),
            operation: "@op/diagnostics".into(),
            values: BTreeMap::new(),
            selectors: vec![format!("zk2/{address}/tc.netif.v1/@op/diagnostics")],
            mode,
            timeout_s: 5.0,
            answer,
        }
    }

    pub fn value() -> OperationReport {
        let key = "zk2/host-a/tc/tc.netif.v1/@op/diagnostics";
        call(
            OperationAnswer::Value {
                reply: reply(key, "response", "@op/diagnostics", json!({"ok": true})),
            },
            CallMode::Concrete,
            "host-a/tc",
        )
    }

    pub fn refused() -> OperationReport {
        call(
            OperationAnswer::Refused {
                envelope: EnvelopeView {
                    code: "app".into(),
                    message: "the kernel refused".into(),
                    cause: None,
                    detail: Some(Rendered::Value {
                        declared: "json:TcError".into(),
                        value: json!({"kind": "kernel"}),
                    }),
                },
            },
            CallMode::Concrete,
            "host-a/tc",
        )
    }

    pub fn silent(presence: PresenceAttribution) -> OperationReport {
        call(
            OperationAnswer::Silent {
                silence: SilenceView {
                    attempts: 3,
                    transport: Some("zenoh/string: Timeout".into()),
                    presence,
                },
            },
            CallMode::Concrete,
            "host-a/tc",
        )
    }

    pub fn fanout() -> OperationReport {
        let (k1, k2) = (
            "zk2/host-a/tc/tc.netif.v1/@op/diagnostics",
            "zk2/host-b/tc/tc.netif.v1/@op/diagnostics",
        );
        let replier = |addr: &str, key: &str, summaries: usize| ReplierView {
            address: addr.into(),
            key: key.into(),
            values: BTreeMap::new(),
            replies: vec![reply(
                key,
                "response",
                "@op/diagnostics",
                json!({"ok": true}),
            )],
            summaries: (0..summaries)
                .map(|_| reply(key, "summary", "@op/diagnostics", json!({"n": 1})))
                .collect(),
            possibly_partial: Some(summaries != 1),
        };
        call(
            OperationAnswer::Replies {
                replies: RepliesView {
                    summary_declared: true,
                    repliers: vec![replier("host-a/tc", k1, 1), replier("host-b/tc", k2, 2)],
                    refusals: vec![EnvelopeView {
                        code: "busy".into(),
                        message: "a scan is running".into(),
                        cause: None,
                        detail: None,
                    }],
                    malformed: vec![],
                    transport: vec![],
                    discarded: 1,
                    presence: SelectionPresence {
                        selector: "zk2/*/tc/@zk/alive/tc.netif.v1/**".into(),
                        complete: true,
                        unheard: vec!["host-c/tc".into()],
                        error: None,
                    },
                },
            },
            CallMode::Fanout,
            "*/tc",
        )
    }

    pub fn state(reading: StateReading, rows: Vec<StateRow>) -> StateReport {
        StateReport {
            reading,
            address: "host-a/tc".into(),
            iface: "tc.netif.v1".into(),
            fingerprint: fp(),
            resource: "state/interfaces/{ns}/{iface}".into(),
            values: BTreeMap::new(),
            selectors: vec!["zk2/host-a/tc/tc.netif.v1/state/interfaces/*/*".into()],
            archive: (reading == StateReading::LastKnown).then(|| "ground/archive".into()),
            timeout_s: 5.0,
            rows,
        }
    }

    pub fn rows(last_known: bool) -> Vec<StateRow> {
        let key = "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0";
        let gone = "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth9";
        vec![
            StateRow {
                key: key.into(),
                value: StateValue::Value {
                    payload: Box::new(PayloadRendering {
                        rendered: Rendered::Value {
                            declared: "json:NetworkInterface".into(),
                            value: json!({"is_up": true}),
                        },
                        ..reply(
                            key,
                            "type",
                            "state/interfaces/{ns}/{iface}",
                            json!({"is_up": true}),
                        )
                    }),
                },
                timestamp: Some(Stamp {
                    time: "2026-10-08T12:00:00.000000000Z".into(),
                    clock: "a1b2c3".into(),
                }),
                confirmed: last_known.then_some(false),
                identity: last_known.then(|| json!({"iface": "tc.netif.v1"})),
            },
            StateRow {
                key: gone.into(),
                value: StateValue::Deleted,
                timestamp: None,
                confirmed: last_known.then_some(true),
                identity: None,
            },
        ]
    }

    pub fn structural() -> PayloadRendering {
        PayloadRendering {
            key: "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0".into(),
            size: 3,
            resource: None,
            rendered: Rendered::Structural {
                why: Unresolved::ContractNotHeld { fingerprint: fp() },
                value: None,
                text: "abc".into(),
            },
        }
    }
}

/// O5's four cases are four first words in a table and four `answer` tags
/// in a document: a value, an envelope, a malformed envelope and silence
/// never read as one another (spec §5.1, the tooling guide's §2).
#[test]
fn a_call_keeps_a_value_an_envelope_and_silence_apart() {
    use zenkey_fleet::report::PresenceAttribution;
    assert_data_eq!(
        table(&actfx::value()),
        str![[r#"
call host-a/tc tc.netif.v1@sha256:4f534f534f534f53… @op/diagnostics  (one address, 5s)
key    zk2/host-a/tc/tc.netif.v1/@op/diagnostics
value  json:DiagnosticsResponse {"ok":true}

"#]]
    );
    assert_data_eq!(
        table(&actfx::refused()),
        str![[r#"
call host-a/tc tc.netif.v1@sha256:4f534f534f534f53… @op/diagnostics  (one address, 5s)
refused  app — the kernel refused
  detail: json:TcError {"kind":"kernel"}

"#]]
    );
    let silent = actfx::silent(PresenceAttribution::NoTokenVisible);
    assert_data_eq!(
        table(&silent),
        str![[r#"
call host-a/tc tc.netif.v1@sha256:4f534f534f534f53… @op/diagnostics  (one address, 5s)
no answer  after 3 attempt(s) — zenoh/string: Timeout
presence   no token of the service is visible to this reader: it may be gone, or this reader may not see its presence (a refused read is empty too)

"#]]
    );
    let tag = |r: &zenkey_fleet::report::OperationReport| -> serde_json::Value {
        serde_json::from_str::<serde_json::Value>(ndjson(r).lines().next().expect("a line"))
            .expect("json")["answer"]
            .clone()
    };
    assert_eq!(
        [
            tag(&actfx::value()),
            tag(&actfx::refused()),
            tag(&silent),
            tag(&actfx::fanout())
        ],
        ["value", "refused", "silent", "replies"].map(serde_json::Value::from)
    );
    // Silence is a note in every format, and only silence is.
    assert!(notes(&silent).contains("never \"no such operation\""));
    assert!(notes(&actfx::value()).is_empty());
    assert!(ndjson(&silent).contains(r#""presence":"no_token_visible""#));
    assert_eq!(actfx::value().exit_code(), 0);
    assert_eq!(actfx::refused().exit_code(), 1);
    assert_eq!(silent.exit_code(), 2);
}

/// Silence is attributed through presence as what this reader could see
/// (§8.1, 0.8): every attribution has its own sentence, and "no token
/// visible" never claims the service is gone.
#[test]
fn a_silence_is_attributed_as_what_the_reader_could_see() {
    use zenkey_fleet::report::PresenceAttribution as P;
    let line = |p: P| {
        table(&actfx::silent(p))
            .lines()
            .find(|l| l.starts_with("presence"))
            .expect("a presence line")
            .to_owned()
    };
    let lines: std::collections::BTreeSet<String> = [
        P::Present,
        P::InstanceOnly,
        P::NoTokenVisible,
        P::Unknown,
        P::Unobservable,
    ]
    .into_iter()
    .map(line)
    .collect();
    assert_eq!(lines.len(), 5, "{lines:#?}");
    assert!(line(P::NoTokenVisible).contains("visible to this reader"));
    assert!(line(P::Unknown).contains("may be incomplete"));
}

/// A fan-out's values are attributed by their key; its envelopes are not,
/// because a `reply_err` carries none; a replier that did not end with one
/// summary is possibly partial (§5.1, O6). The repliers are the rows, and
/// everything else rides the envelope.
#[test]
fn a_fan_out_attributes_values_by_key_and_leaves_envelopes_unattributed() {
    let r = actfx::fanout();
    assert_data_eq!(
        table(&r),
        str![[r#"
call */tc tc.netif.v1@sha256:4f534f534f534f53… @op/diagnostics  (fan-out, 5s)
REPLIER    KEY                                        REPLY
host-a/tc  zk2/host-a/tc/tc.netif.v1/@op/diagnostics  json:DiagnosticsResponse {"ok":true}
  summary: json:DiagnosticsResponse {"n":1}
host-b/tc  zk2/host-b/tc/tc.netif.v1/@op/diagnostics  json:DiagnosticsResponse {"ok":true}
  summary: json:DiagnosticsResponse {"n":1}
  summary: json:DiagnosticsResponse {"n":1}
  possibly partial: 2 summaries, not exactly one
refused (unattributed): busy — a scan is running
no value from host-c/tc: each holds the interface's token, and refused or was silent — a caller cannot tell which

"#]]
    );
    let lines: Vec<serde_json::Value> = ndjson(&r)
        .lines()
        .map(|l| serde_json::from_str(l).expect("json"))
        .collect();
    assert_eq!(lines.len(), 3, "an envelope and a row per replier");
    assert_eq!(lines[0]["report"], "operation");
    assert_eq!(lines[0]["mode"], "fanout");
    assert!(
        lines[0]["replies"].get("repliers").is_none(),
        "rows, not envelope"
    );
    assert_eq!(lines[0]["replies"]["refusals"][0]["code"], "busy");
    assert_eq!(lines[0]["replies"]["presence"]["unheard"][0], "host-c/tc");
    assert_eq!(lines[1]["row"], "replier");
    assert_eq!(lines[1]["possibly_partial"], false);
    assert_eq!(lines[2]["possibly_partial"], true);
    let n = notes(&r);
    assert!(n.contains("unattributed"), "{n}");
    assert!(n.contains("possibly partial"), "{n}");
    assert!(n.contains("(R6)"), "{n}");
}

/// Current and last-known state are different questions (S6), and no
/// medium lets one pass for the other: the table's first line, the
/// document's `reading`, and a note in every format.
#[test]
fn current_and_last_known_state_never_look_alike() {
    use zenkey_fleet::report::StateReading;
    let current = actfx::state(StateReading::Current, actfx::rows(false));
    let last = actfx::state(StateReading::LastKnown, actfx::rows(true));
    assert_data_eq!(
        table(&current),
        str![[r#"
CURRENT state, the owner's answer: tc.netif.v1@sha256:4f534f534f534f53… state/interfaces/{ns}/{iface} at host-a/tc
KEY                                                      STATE                                 STAMP
zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0  json:NetworkInterface {"is_up":true}  2026-10-08T12:00:00.000000000Z (clock a1b2c3)
zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth9  deleted                               —

"#]]
    );
    assert_data_eq!(
        table(&last),
        str![[r#"
LAST-KNOWN state from archive ground/archive, never current: tc.netif.v1@sha256:4f534f534f534f53… state/interfaces/{ns}/{iface} at host-a/tc
KEY                                                      STATE                                 STAMP
zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0  json:NetworkInterface {"is_up":true}  2026-10-08T12:00:00.000000000Z (clock a1b2c3)
  NOT confirmed: alignment has not confirmed this key
zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth9  deleted                               —
  confirmed by the archive's alignment

"#]]
    );
    let envelope = |r: &zenkey_fleet::report::StateReport| -> serde_json::Value {
        serde_json::from_str(ndjson(r).lines().next().expect("a line")).expect("json")
    };
    assert_eq!(envelope(&current)["reading"], "current");
    assert_eq!(envelope(&last)["reading"], "last_known");
    assert!(envelope(&current).get("archive").is_none());
    assert_eq!(envelope(&last)["archive"], "ground/archive");
    assert!(notes(&last).contains("last-known, never current"));
    assert!(ndjson(&last).contains("last-known, never current"));
    assert!(!ndjson(&current).contains("last-known, never current"));
    // A current read keeps no confirmation: absent, not false (O4).
    assert!(!ndjson(&current).contains("confirmed"));
    // Silence is no rows and a note, never "no value".
    let silent = actfx::state(StateReading::Current, vec![]);
    assert!(notes(&silent).contains("silence, not \"no value\""));
    assert_eq!(ndjson(&silent).lines().count(), 1, "the envelope alone");
}

/// A watched sample says how far its contract reached, and the summary
/// keeps R6's discards apart from what the tool lagged — two counts, two
/// lines, both non-zero here so a renderer that summed them would show.
#[test]
fn a_watch_keeps_r6_discards_apart_from_its_lag() {
    use zenkey_fleet::report::{WatchEnd, WatchEvent, WatchSample, WatchSummary};
    let sample = WatchSample {
        provider: "host-a/tc".into(),
        key: "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0".into(),
        values: Default::default(),
        timestamp: None,
        event: WatchEvent::Put {
            payload: Box::new(actfx::structural()),
            conformance: zenkey_fleet::report::Conformance::NotChecked {
                reason: "contract not held".into(),
            },
            attachment: None,
        },
        qos_mismatch: None,
    };
    assert_eq!(
        zenctl::render::sample_lines(&sample),
        [
            "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0  [host-a/tc]",
            &format!(
                "  abc  (structural: contract {} not held)",
                "sha256:4f534f534f534f53…"
            ),
        ]
    );
    let delete = WatchSample {
        event: WatchEvent::Delete,
        ..sample.clone()
    };
    assert!(zenctl::render::sample_lines(&delete)[1].contains("not an empty value"));
    // A sample that failed its type, off its declared QoS: each its own line,
    // the violation and the axis that differs named.
    let off = WatchSample {
        event: WatchEvent::Put {
            payload: Box::new(actfx::structural()),
            conformance: zenkey_fleet::report::Conformance::Invalid {
                violations: vec!["/is_up: expected boolean".into()],
            },
            attachment: None,
        },
        qos_mismatch: Some(zenkey_fleet::report::QosMismatch {
            declared: zenkey_fleet::report::QosAxes {
                priority: "data".into(),
                congestion: "drop".into(),
                express: false,
            },
            observed: zenkey_fleet::report::QosAxes {
                priority: "real_time".into(),
                congestion: "drop".into(),
                express: false,
            },
            differs: vec!["priority".into()],
        }),
        ..sample
    };
    let lines = zenctl::render::sample_lines(&off);
    assert_eq!(
        lines[2],
        "  invalid against its type: /is_up: expected boolean"
    );
    assert!(
        lines[3].starts_with("  QoS not as declared (spec §2.4): priority")
            && lines[3].contains("data")
            && lines[3].contains("real_time"),
        "{lines:#?}"
    );
    let summary = WatchSummary {
        address: "*/tc".into(),
        iface: "tc.netif.v1".into(),
        fingerprint: actfx::fp(),
        resource: "stream/bandwidth/{ns}/{iface}".into(),
        selectors: vec!["zk2/*/tc/tc.netif.v1/stream/bandwidth/*/*".into()],
        received: 4,
        discarded: 10,
        unresolved: 0,
        qos_mismatched: 0,
        nonconforming: 0,
        lagged: 3,
        elapsed_s: 2.0,
        ended: WatchEnd::Window,
    };
    let lines = zenctl::render::summary_lines(&summary);
    assert_eq!(lines.len(), 3, "{lines:#?}");
    assert!(lines[1].starts_with("10 sample(s)") && lines[1].contains("(R6)"));
    assert!(lines[2].starts_with("3 sample(s)") && lines[2].contains("lower bound"));
    // #671's unresolved, §2.4's QoS and §7's types: three more counts, each
    // its own line, none folded into another.
    let counted = WatchSummary {
        unresolved: 1,
        qos_mismatched: 2,
        nonconforming: 5,
        ..summary.clone()
    };
    let lines = zenctl::render::summary_lines(&counted);
    assert_eq!(lines.len(), 6, "{lines:#?}");
    assert!(lines[2].starts_with("1 sample(s) on a key that resolved to no member"));
    assert!(lines[3].starts_with("2 sample(s) did not ride the resource's declared QoS"));
    assert!(lines[4].starts_with("5 payload(s) failed their declared type"));
    let silent = WatchSummary {
        received: 0,
        discarded: 0,
        lagged: 0,
        ..summary
    };
    assert!(
        zenctl::render::summary_lines(&silent)
            .last()
            .is_some_and(|l| l.contains("never a verdict"))
    );
}
