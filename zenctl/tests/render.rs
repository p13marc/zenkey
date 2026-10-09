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

/// Two row kinds on one stream, told apart by a tag rather than by guessing at
/// fields — the defect this family had before the seam.
#[test]
fn a_storage_lists_two_row_kinds_are_tagged() {
    let out = ndjson(&fx::storage_list());
    let kinds: Vec<String> = out
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v.get("row").and_then(|r| r.as_str()).map(str::to_string))
        .collect();
    assert_eq!(kinds, ["storage", "coverage", "coverage", "coverage"]);
    assert_data_eq!(
        table(&fx::storage_list()),
        str![[r#"
configured storages:

  main  @aabbccdd  acme/v1/**/state/**
    strip —  ·  volume memory

declared state families vs storage coverage:

  ✓ sysinfo   health        covered by main@aabbccdd
  ~ logs      state/{unit}  PARTIAL via main@aabbccdd
  · parallax  stream/{id}   uncovered

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
✗  split-brain (§6)                  finding — 1 subject(s)
    ✗ error: host-a/tc tc.netif.v1 — 2 instances hold its interface token in two presence reads 2.0s apart (3fa9c2d41b7e0012, 3fa9c2d41b7e0013), and at least two expose an exclusive resource
⚠  binding-unsatisfied (§3.2 R5)     finding — 1 subject(s)
    ⚠ warning: ws-01/tcgui-frontend scenario — its bindings (*/tc) select no provider of tc.scenario.v1 visible to this reader
?  contract-drift (§9.8)             unobservable — 1 subject(s) unjudged
    ? unjudged tc.netem.v1 eeeeeeeeeeeeeeee 5d1c0a9b2e3f4a6b: not classified: tc.netem.v1 sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee is unavailable
✗  contract-unavailable (§8.4)       finding — 1 subject(s)
    ✗ error: tc.netem.v1 sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee — named by host-b/tc@3fa9c2d41b7e0014; no holder served a bundle that verified (no reply)
✓  descriptor-invalid (§3.3)         clean — 3 descriptor(s) pass the descriptor check against the contracts they name
✓  token-missing (§8.1)              clean — 3 instance(s): every token agrees with its descriptor
✓  presence-over-budget (§8.3)       clean — 9 token(s) visible to this reader in the presence domain; within the budget 10000
✓  storage-on-state (§4.2 S4)        clean — 1 storage(s) on 2 router(s), none answering on an owner's state keys
✓  archive-unaligned (§4.4)          clean — no archive.v1 provider visible to this reader: nothing to align
—  state-stamp-foreign (§4.2 S1–S2)  not asked
·  shm-memlock-low (§7.4)            finding — 1 subject(s)
    · info: this host — RLIMIT_MEMLOCK is 64 KiB, below the 8 MiB floor
✓  admin-unreachable (§4.2)          clean — 2 router(s) answered `@/*/router`
✓  router-version-skew (App. B)      clean — 2 router(s), all at 1.10.1

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
        said.contains("4 finding(s): 2 error(s), 1 warning(s), 1 info"),
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

/// The `why` ladder (#214): one line per rung, the three answer states drawn
/// as three marks — `✓` established, `✗`/`·` not-established (a cause / a
/// mere fact), `?` NOT ASKED — reasons and evidence indented, verdict word
/// last. The fixture is the acceptance posture: declared, alive, never
/// published, under the `Healthy` verdict.
#[test]
fn a_why_ladder_draws_one_rung_per_line_with_its_three_states() {
    assert_data_eq!(
        table(&fx::why_report()),
        str![[r#"
✓  scope-reach         does a `**` explorer scope reach this key?
      the `v1/**` explorer scope intersects this key
✓  key-parse           does it parse as a v1 key under the base?
      origin h-3fa9c2d41b7e (host), class telemetry, producer sysinfo, subject disk/root/used
✓  registry-declared   does a loaded registry slice declare it?
      declared as disk/{mount}/used (TelemetryPoint)
✓  origin-alive        is the origin on the liveliness roster?
      h-3fa9c2d41b7e is on the roster with producer(s): sysinfo
·  publisher-declared  did any session declare a matching publisher?
      ↳ declared, alive, never published — publishers declare lazily (RFC 08 §6.1): no publisher declaration exists until the first publication, so this is not evidence of a bug
✓  storage-coverage    is a storage configured to capture it?
      storage latest@aabbccdd (v1/*/telemetry/**) captures every key this expression names
·  stored-value        does a stored value answer a bounded GET?
      ↳ none of get, @adv cache returned a value — which is silence, not proof no value exists (RFC 05 §3.1)
?  sample-freshness    is the last known sample within its declared ttl?
      no sample in hand to age — the stored-value rung found none
✓  admin-answered      is the admin space answering at all?
      1 admin root document(s) answered @/*/*
?  wire-heard          did the key speak during a listen window?
      not listened — the data plane costs one deliberate action (RFC 09 §5.1, v1.18 frugality); pass --for <SECS> to watch the wire
NO CAUSE ESTABLISHED

"#]]
    );
}

/// The `why` notes carry the honesty sentences into every format: the
/// RFC 05 §3.1 framing, the exit-code meaning, and the next step for the one
/// rung that was not asked.
#[test]
fn a_why_ladders_notes_state_the_non_verdict_and_the_exit() {
    let stderr = notes(&fx::why_report());
    assert!(stderr.contains("silence is never a verdict"), "{stderr}");
    assert!(stderr.contains("exit 1"), "{stderr}");
    assert!(stderr.contains("--for"), "{stderr}");
}

/// The `why` ndjson: the envelope leads with the verdict and the cause ids
/// (so a script need not re-derive the exit-0 policy), then one tagged row
/// per rung — `not_asked` rows carrying no `reason`.
#[test]
fn a_why_ladders_ndjson_leads_with_the_verdict_then_tags_every_rung() {
    let out = ndjson(&fx::why_report());
    let envelope: serde_json::Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    assert_eq!(envelope["report"], "why");
    assert_eq!(envelope["verdict"], "healthy");
    assert_eq!(envelope["causes"], serde_json::json!([]));
    assert!(
        !envelope.as_object().unwrap().contains_key("rungs"),
        "rungs are rows, not an envelope field"
    );
    let rows: Vec<serde_json::Value> = out
        .lines()
        .skip(1)
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(rows.len(), 10, "one row per rung");
    let lazy = rows
        .iter()
        .find(|r| r["id"] == "publisher-declared")
        .unwrap();
    assert_eq!(lazy["answer"], "not_established");
    assert!(
        lazy["reason"]
            .as_str()
            .unwrap()
            .contains("publishers declare lazily"),
        "{lazy}"
    );
    let unasked = rows.iter().find(|r| r["id"] == "wire-heard").unwrap();
    assert_eq!(unasked["answer"], "not_asked");
    assert!(
        unasked.get("reason").is_none(),
        "not asked has no negative answer to spell (O4)"
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
        table(&fx::why_report()),
        table(&catalog.services()),
        table(&catalog.service(&"host-a/tc".parse().expect("an address"))),
        table(&catalog.ifaces()),
        table(&zk2fx::iface_view()),
        table(&catalog.graph()),
        table(&zk2fx::schema_view()),
        table(&zk2fx::compat_report()),
        table(&zk2fx::namespace_listing()),
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

/// The three non-answer populations stay apart, in both media. The panicked
/// line and its note are new with #329: a `JoinError` used to skip `completed`,
/// `errors` *and* `silent`, so a whole population vanished from a report whose
/// every other counter exists to stop exactly that.
#[test]
fn a_bench_report_right_aligns_its_numbers_and_counts_non_answers_apart() {
    assert_data_eq!(
        table(&fx::bench_report()),
        str![[r#"
→ v1/h-3fa9c2d41b7e/@rpc/sysinfo/processes
98 call(s), concurrency 8, 2.50s — 39.2 calls/s
  2 of 100 calls did not complete
  1 of those panicked in this tool — measured nothing

origin          replies  min ms  p50 ms  p95 ms  p99 ms  max ms
h-3fa9c2d41b7e       64    0.80    1.90   12.40   40.10  123.46
h-bbbbbbbbbbbb       34    1.10    2.20    9.90   11.00   12.50

"#]]
    );
    assert!(notes(&fx::bench_report()).contains("drew no reply"));
    assert!(notes(&fx::bench_report()).contains("panicked inside this tool"));
}

/// Three `origins` outcomes, three sentences: answered-and-empty, not asked,
/// and a real list.
#[test]
fn a_blob_list_tells_three_kinds_of_empty_apart() {
    assert_data_eq!(
        table(&fx::blob_list()),
        str![[r#"
declared @blob tiers:

  logs      store     v2.0
      endpoints  have, chunk
      algo       blake3
      origins    — (no liveliness token answered; silence is not a verdict — RFC 05 §3.1)
  parallax  artifact  v1.3
      endpoints  manifest
      algo       blake3
      reference  ArtifactRef  (carries the content root — RFC 07 §2.1)
      encoding   application/octet-stream
      origins    — (roster not asked)
      build artifacts

"#]]
    );
}

#[test]
fn a_blob_probe_reports_two_roots_as_a_finding() {
    assert_data_eq!(
        table(&fx::blob_probe()),
        str![[r#"
target  artifact/01jqz3demo0001  (tier artifact)

asked:
  v1/*/@blob/artifact/01jqz3demo0001/manifest
  v1/*/@blob/artifact/01jqz3demo0001/have

holders:

  h-3fa9c2d41b7e  8/8 chunks · 65536 bytes · root 60e03a78c0e0…
      v1/h-3fa9c2d41b7e/@blob/artifact/01jqz3demo0001/manifest
  h-bbbbbbbbbbbb  3/8 chunks · no manifest reply
      note: every chunk but no index
      v1/h-bbbbbbbbbbbb/@blob/artifact/01jqz3demo0001/have

"#]]
    );
    assert!(notes(&fx::blob_probe()).contains("distinct content roots"));
    // A probe that was never issued must never read as "nobody holds it".
    assert_data_eq!(
        table(&fx::blob_probe_unissued()),
        str![[r#"
target  tree/deadbeef  (tier tree)

"#]]
    );
    assert!(notes(&fx::blob_probe_unissued()).contains("not probed"));

    // R7: a silent probe over zero registry slices names the third silence —
    // "nobody declares this tier" was never established either.
    let silent_no_registry = zenkey_fleet::report::BlobProbeReport {
        holders: vec![],
        answered: 0,
        roots: vec![],
        declared_by: vec![],
        slices_considered: 0,
        ..fx::blob_probe()
    };
    let n = notes(&silent_no_registry);
    assert!(n.contains("no registry loaded"), "{n}");
    // …while a silent probe with slices read keeps the two-silence wording:
    // an empty declared_by over a real sweep IS "nobody declares it".
    let silent_swept = zenkey_fleet::report::BlobProbeReport {
        declared_by: vec![],
        slices_considered: 11,
        ..silent_no_registry
    };
    let n = notes(&silent_swept);
    assert!(!n.contains("no registry loaded"), "{n}");
    assert!(n.contains("no replies"), "{n}");
}

#[test]
fn a_blob_tree_and_a_blob_fetch_are_one_ndjson_line_each() {
    for out in [ndjson(&fx::blob_tree()), ndjson(&fx::blob_fetch())] {
        assert_eq!(out.lines().count(), 1, "not a document:\n{out}");
    }
    assert_data_eq!(
        table(&fx::blob_tree()),
        str![[r#"
tree/deadbeefcafe
  from      h-3fa9c2d41b7e (v1/h-3fa9c2d41b7e/@blob/tree/deadbeef/index)
  index     12 entries, 9 file(s)
  content   1048576 bytes in 40 distinct chunk(s)
  priority  data_low/block/reliable
  root      deadbeefcafe

"#]]
    );
    assert_data_eq!(
        table(&fx::blob_fetch()),
        str![[r#"
bundle.bin
  from      h-3fa9c2d41b7e (v1/h-3fa9c2d41b7e/@blob/artifact/01jqz3demo0001/chunk)
  bytes     65536 in 8 chunk(s), 2 resumed
  priority  data_low/block/reliable
  root      trust-on-first-use — this origin chose the content
  retries                                                                        1

"#]]
    );
    assert!(notes(&fx::blob_fetch()).contains("failed verification before disk"));
}

/// One reply, one error envelope, and the attachment clause `check probe` used
/// to drop (#237).
#[test]
fn a_call_and_a_probe_render_a_reply_identically() {
    let call = table(&fx::call_report());
    let probe = table(&fx::probe_report());
    assert_data_eq!(
        call.clone(),
        str![[r#"
h-3fa9c2d41b7e:
{
  "count": 214
}
  attachment (18 B): {"trace":"abc123"}
h-bbbbbbbbbbbb: ✗ unsupported — this build serves no `processes`

"#]]
    );
    assert!(
        probe.ends_with(&call),
        "probe is the call plus one provenance line:\n{probe}"
    );
    assert!(call.contains("attachment (18 B)"), "the clause probe lost");

    // R5: a silent call's note names the wait, and the document states it —
    // it used to say "the timeout too short" about a timeout the report
    // never carried (O5). The probe inherits both by delegation.
    let silent = zenkey_fleet::report::CallReport {
        answers: vec![],
        ..fx::call_report()
    };
    let n = notes(&silent);
    assert!(n.contains("within 5s"), "{n}");
    let envelope: serde_json::Value =
        serde_json::from_str(ndjson(&silent).lines().next().unwrap()).unwrap();
    // `5.0`: the seconds unification (#218) — see the budget window above.
    assert_eq!(envelope["timeout_s"], 5.0);
    let silent_probe = zenkey_fleet::report::ProbeReport {
        call: silent,
        ..fx::probe_report()
    };
    assert!(notes(&silent_probe).contains("within 5s"));
    let envelope: serde_json::Value =
        serde_json::from_str(ndjson(&silent_probe).lines().next().unwrap()).unwrap();
    // `5.0`: the seconds unification (#218) — a probe wraps a `CallReport`,
    // so it moved with it, which is the point of rendering them identically.
    assert_eq!(envelope["timeout_s"], 5.0);
}

/// RFC 05 §3.2 (#424): a bounded reply that stopped early says so on the
/// line — a caller MUST NOT read a short page as the end — and `partial:
/// true` with `next_cursor: null` is the contract violation the RFC says an
/// observer MAY report. A caveat, never an exit code: `call` is an act, and
/// both replies arrived.
#[test]
fn a_partial_page_says_stopped_early_and_a_null_cursor_is_a_caveat() {
    let report = fx::call_report_partial_page();
    let t = table(&report);
    assert!(
        t.contains(
            "stopped early (partial=true, next_cursor=e-41, scanned=4096, \
             covers_from=2026-09-06T10:00:00Z) — RFC 05 §3.2"
        ),
        "{t}"
    );
    assert!(
        t.contains("stopped early (partial=true, next_cursor=null) — RFC 05 §3.2"),
        "the optional fields are omitted when the wire omitted them:\n{t}"
    );
    assert_eq!(t.matches("stopped early").count(), 2, "{t}");

    let n = notes(&report);
    assert!(
        n.contains(
            "h-bbbbbbbbbbbb: partial=true with next_cursor=null — the reply says it \
             stopped early and offers no way to continue (a contract violation) \
             (RFC 05 §3.2)"
        ),
        "{n}"
    );
    assert!(
        !n.contains("h-3fa9c2d41b7e: partial=true"),
        "a cursor is a way on — no caveat for the first answer:\n{n}"
    );
    assert_eq!(report.exit_code(), 0, "a call is an act, not a judgement");

    // `check probe` delegates, so it inherits both the line and the caveat.
    let probe = zenkey_fleet::report::ProbeReport {
        call: report,
        ..fx::probe_report()
    };
    assert!(table(&probe).contains("stopped early (partial=true, next_cursor=null)"));
    assert!(notes(&probe).contains("a contract violation) (RFC 05 §3.2)"));
    assert_eq!(probe.call.exit_code(), 0);
}

#[test]
fn a_cutover_puts_the_verdict_word_beside_its_evidence() {
    assert_data_eq!(
        table(&fx::cutover_report()),
        str![[r#"
old root acme/legacy: 12 sample(s) on 2 key(s) over 30s
  ✗  acme/legacy/sysinfo/health
  ✗  acme/legacy/sysinfo/disk
new plane acme/v1/**: 480 sample(s)
leaks (outside acme/v1/ and not the old root): 3 sample(s) on 1 key(s)
  !  acme/scratch/tmp
FAIL

"#]]
    );
    assert!(notes(&fx::cutover_report()).contains("still speaks"));
}

/// The burn-down (#226): each ledger entry carries its four facts, each fact
/// honest about whether it was even asked, and the verdict word closes the
/// table exactly as `check cutover`'s does — same vocabulary, same exit
/// discipline.
#[test]
fn a_retired_report_puts_four_facts_beside_each_ledger_entry() {
    assert_data_eq!(
        table(&fx::retired_report()),
        str![[r#"
✗  logs: logs/errors_total              wire 3 sample(s) · STILL SERVED · 1 subscriber(s) · → logs/journald/errors_total: 480 sample(s)
✓  logs: logs/by_unit/{unit}/burn_rate  wire silent · not served · 0 subscriber(s) · → logs/journald/burn_rate: 120 sample(s)
?  logs: logs/units_in_failure          wire silent · not served · 0 subscriber(s) · → logs/journald/units_in_failure: 0 sample(s)
FAIL

"#]]
    );
    let notes = notes(&fx::retired_report());
    assert!(
        notes.contains("one checkout's slice"),
        "the report must state which registries it read: {notes}"
    );
    // The envelope leads the ndjson, with the coverage claim intact and the
    // entries reduced to a count (the rows carry them).
    let first = ndjson(&fx::retired_report())
        .lines()
        .next()
        .unwrap()
        .to_string();
    let envelope: serde_json::Value = serde_json::from_str(&first).unwrap();
    assert_eq!(envelope["report"], "registry-retired");
    assert_eq!(envelope["entries"], 3);
    assert_eq!(
        envelope["registries"][0],
        "../zensight/zensight-common/registry"
    );
}

/// The field window (#223): per-path stats beside their findings, the path
/// table's bound stated in every format, and the stuck caveat — an
/// observation with a window, not a verdict — where a reader will see it.
#[test]
fn a_field_report_states_its_bound_and_its_stuck_caveat() {
    assert_data_eq!(
        table(&fx::field_report()),
        str![[r#"
40/40  v1/h-3fa9c2d41b7e/state/sysinfo/health · temperature_c  number  unchanged  min 21.5 max 21.5 last 21.5  values {21.5}
40/40  v1/h-3fa9c2d41b7e/state/sysinfo/health · status         string  1 change(s), last at 12.0s  values {"degraded", "ok"}

⚠  field-stuck: v1/h-3fa9c2d41b7e/state/sysinfo/health · temperature_c — value 21.5 unchanged across 40 sample(s) spanning 29.5s — at least 3× the declared ttl_s 5s — while the key kept publishing. An observation over this 30s window, not a verdict: a constant-by-design field always reads this way  [RFC 04 §1.2]

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
        stderr.contains("2 sample(s) carried no structural document"),
        "undocumented is counted apart from absence (O4): {stderr}"
    );
    assert!(
        stderr.contains("not a verdict"),
        "the stuck caveat rides every rendering: {stderr}"
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
/// format.
#[test]
fn an_impaired_expectation_says_it_is_not_a_verdict_either_way() {
    assert_data_eq!(
        table(&fx::expect_report()),
        str![[r#"
acme/v1/**/state/**: 120 sample(s) on 4 key(s) over 5.0s, 24.00 Hz over the full window
violations (1 shown of 9):
  ✗  v1/h-3fa9c2d41b7e/state/sysinfo/health: qos data/drop
IMPAIRED — the observation cannot carry the claim:
  !  17 sample(s) were dropped while behind

"#]]
    );
    assert!(notes(&fx::expect_report()).contains("not a verdict either way"));
}

/// The conformance suite (#222): the state word leads every row, an
/// exemption is named beside its evidence, unknowable carries its reason —
/// and the verdict closes the table. The notes carry what was not asked,
/// and the drop, in every format.
#[test]
fn a_conform_report_keeps_three_states_and_names_the_exemption() {
    assert_data_eq!(
        table(&fx::conform_report()),
        str![[r#"
✓ met         procedure/introspect          h-3fa9c2d41b7e: a value reply  [RFC 08 §6]
✓ exempt      procedure/dns                 when: config:collect.dns — h-3fa9c2d41b7e: error/gated — conditional, and said so  [RFC 08 §6.1]
✗ not met     qos-observed-mismatch/health  v1/h-3fa9c2d41b7e/state/sysinfo/health: 4 of 4 sample(s) did not ride the declared transition  [RFC 04 §3]
? unknowable  observed/disk/{mount}/used    a window proves presence, never absence — not seen in 10s  [RFC 13 §3]
VIOLATES

"#]]
    );
    let notes = notes(&fx::conform_report());
    assert!(notes.contains("not asked: stale-state"), "{notes}");
    assert!(notes.contains("3 sample(s) dropped"), "{notes}");
    assert!(notes.contains("synthetic marker"), "{notes}");
    let lines: Vec<serde_json::Value> = ndjson(&fx::conform_report())
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines[0]["report"], "conform");
    assert_eq!(lines[0]["verdict"], "violates");
    assert!(lines[0].get("assertions").is_none(), "rows are rows");
    let states: Vec<&str> = lines[1..]
        .iter()
        .map(|l| {
            assert_eq!(l["row"], "assertion");
            l["state"].as_str().unwrap()
        })
        .collect();
    assert_eq!(states, ["met", "met", "not_met", "unknowable"]);
}

/// The fleet timeline (#216), arrival axis: lanes per origin/producer with
/// the stamper in the heading, the unstamped lane beside them, `pos` from
/// the merged ordering, and the drop as a break at its arrival position.
#[test]
fn a_timeline_on_arrival_groups_lanes_and_places_the_break() {
    assert_data_eq!(
        table(&fx::timeline_report_arrival()),
        str![[r#"
h-3fa9c2d41b7e/sysinfo · arrival · stamper 33 (2 unattributable)
0  +1.000ms  200/33      acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu
1  +2.000ms  100/33      acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/mem

unstamped (arrival axis only) · arrival · no stamper
3  +3.000ms              plain/key

breaks · arrival positions
2            dropped ×3

"#]]
    );
    assert_data_eq!(
        ndjson(&fx::timeline_report_arrival()),
        str![[r#"
{"axis":"arrival","clock":"observer monotonic, µs since window start","dropped":3,"keys_evicted":0,"lanes":[{"first_t_us":1000,"lane":{"kind":"origin","origin":"h-3fa9c2d41b7e","producer":"sysinfo"},"last_t_us":2000,"provenance":{"foreign":0,"self_stamped":0,"unattributable":2},"samples":2,"stampers":["33"]},{"first_t_us":3000,"lane":{"kind":"unstamped"},"last_t_us":3000,"provenance":{"foreign":0,"self_stamped":0,"unattributable":0},"samples":1,"stampers":[]}],"notes":[{"cite":"RFC 09 §5.1 O7","text":"ordered by arrival — observer monotonic, µs since window start; a position says when this observer saw a sample, never when it was produced"},{"cite":"RFC 09 §5.1 O4","text":"the per-publisher sequence-number lane is unavailable: zenoh 1.9/1.10 deliver no SourceInfo to subscribers (eclipse-zenoh/zenoh#2563); `tests/stamper.rs` pins it"},{"cite":"RFC 03 §4 D2","text":"a `**` selector never crosses an `@`-chunk: the verbatim planes (`@rpc`, `@media`, `@blob`, `@catalog`) are excluded from this window, not empty"},{"cite":"RFC 09 §5.1 O6","text":"3 sample(s) dropped while behind — the ordering covers only what was seen"}],"order_by":"arrival","report":"timeline","scopes":["acme/v1/**"],"sn_lane":{"reason":"zenoh 1.9/1.10 deliver no SourceInfo to subscribers (eclipse-zenoh/zenoh#2563); `tests/stamper.rs` pins it","state":"unavailable"},"source":{"kind":"live"},"window_s":10.0}
{"hlc":"200/33","key":"acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu","kind":"put","lane":{"kind":"origin","origin":"h-3fa9c2d41b7e","producer":"sysinfo"},"order_by":"arrival","pos":0,"provenance":"unattributable","row":"sample","stamped_by":"33","t_us":1000}
{"hlc":"100/33","key":"acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/mem","kind":"put","lane":{"kind":"origin","origin":"h-3fa9c2d41b7e","producer":"sysinfo"},"order_by":"arrival","pos":1,"provenance":"unattributable","row":"sample","stamped_by":"33","t_us":2000}
{"kind":"dropped","n":3,"order_by":"arrival","pos":2,"row":"break"}
{"key":"plain/key","kind":"put","lane":{"kind":"unstamped"},"order_by":"arrival","pos":3,"row":"sample","t_us":3000}

"#]]
    );
    let n = notes(&fx::timeline_report_arrival());
    assert!(n.contains("ordered by arrival"), "{n}");
    assert!(n.contains("sequence-number lane is unavailable"), "{n}");
    assert!(n.contains("never crosses an `@`-chunk"), "{n}");
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
h-3fa9c2d41b7e/sysinfo · hlc · stamper 33 (2 unattributable)
0  +2.000ms  100/33  acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/mem
1  +1.000ms  200/33  acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu

"#]]
    );
    assert_data_eq!(
        ndjson(&fx::timeline_report_hlc()),
        str![[r#"
{"axis":"hlc","claim":"happens_before","dropped":3,"keys_evicted":0,"lanes":[{"first_t_us":1000,"lane":{"kind":"origin","origin":"h-3fa9c2d41b7e","producer":"sysinfo"},"last_t_us":2000,"provenance":{"foreign":0,"self_stamped":0,"unattributable":2},"samples":2,"stampers":["33"]}],"notes":[{"cite":"RFC 09 §5.1 O7","text":"ordered by HLC — every stamped sample was stamped by 33, so the order is that node's happened-before (its HLC is monotonic and updated by what it forwarded)"},{"text":"1 unstamped sample(s) are not on this axis — an unstamped sample has no HLC position and is never defaulted to its arrival time; see `--order arrival`"},{"text":"3 dropped sample(s) have no position on the HLC axis (a drop is something this observer suffered, on its own clock); see `--order arrival` for where they fell"},{"cite":"RFC 09 §5.1 O4","text":"the per-publisher sequence-number lane is unavailable: zenoh 1.9/1.10 deliver no SourceInfo to subscribers (eclipse-zenoh/zenoh#2563); `tests/stamper.rs` pins it"},{"cite":"RFC 03 §4 D2","text":"a `**` selector never crosses an `@`-chunk: the verbatim planes (`@rpc`, `@media`, `@blob`, `@catalog`) are excluded from this window, not empty"},{"cite":"RFC 09 §5.1 O6","text":"3 sample(s) dropped while behind — the ordering covers only what was seen"}],"order_by":"hlc","report":"timeline","scopes":["acme/v1/**"],"sn_lane":{"reason":"zenoh 1.9/1.10 deliver no SourceInfo to subscribers (eclipse-zenoh/zenoh#2563); `tests/stamper.rs` pins it","state":"unavailable"},"source":{"kind":"live"},"stamper":"33","unstamped_excluded":1,"window_s":10.0}
{"hlc":"100/33","key":"acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/mem","kind":"put","lane":{"kind":"origin","origin":"h-3fa9c2d41b7e","producer":"sysinfo"},"order_by":"hlc","pos":0,"provenance":"unattributable","row":"sample","stamped_by":"33","t_us":2000}
{"hlc":"200/33","key":"acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu","kind":"put","lane":{"kind":"origin","origin":"h-3fa9c2d41b7e","producer":"sysinfo"},"order_by":"hlc","pos":1,"provenance":"unattributable","row":"sample","stamped_by":"33","t_us":1000}

"#]]
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
/// table's second line, and a caveat note the machine formats carry.
#[test]
fn a_snapshot_states_its_span_and_its_holders() {
    assert_data_eq!(
        table(&fx::snapshot_report()),
        str![[r#"
snapshot of acme/v1/**: 5 key(s) — 2 live, 2 storage-only, 1 unattributed → fleet.zsnap
collected over 1.25s from 2026-09-06T00:00:00Z (1 asked, 6 answered)

"#]]
    );
    let n = notes(&fx::snapshot_report());
    assert!(n.contains("not at an instant"), "{n}");
    assert!(n.contains("`**` cannot cross"), "{n}");
    assert!(n.contains("lost last-writer-wins"), "{n}");
    assert!(
        !n.contains("roster not asked"),
        "the fixture asked the roster: {n}"
    );
    assert_data_eq!(
        ndjson(&fx::snapshot_report()),
        str![[r#"
{"header":{"answered":6,"asked":1,"base":"acme","collected_at":"2026-09-06T00:00:00Z","collection_span_s":1.25,"roster":2,"selectors":["acme/v1/**"],"superseded":1,"zsnap":1},"live":2,"notes":[{"cite":"RFC 13 §4.4","text":"collected over 1.25s, not at an instant — a fan-in GET has no single moment"},{"cite":"RFC 03 §4 D2","text":"`**` cannot cross `@`-planes; they are excluded, not empty"},{"cite":"RFC 09 §5.1 O6","text":"1 answer(s) lost last-writer-wins to a newer reply on the same key"}],"out":"fleet.zsnap","report":"snapshot","storage_only":2,"unattributed":1}

"#]]
    );
}

/// A diff states both spans, keeps the facets apart in its rows, and its
/// human verdict word is the exit code's carrier (#219).
#[test]
fn a_snapshot_diff_states_both_spans_and_tags_every_row() {
    assert_data_eq!(
        table(&fx::snapshot_diff()),
        str![[r#"
a: 5 key(s), span 1.25s at 2026-09-06T00:00:00Z (acme/v1/**)
b: 5 key(s), span 0.80s at 2026-09-06T00:05:00Z (acme/v1/**)
1 added, 1 removed, 2 changed, 2 unchanged
  +  acme/v1/h-9b2e4c7a1d05/telemetry/sysinfo/disk/var-log/used
  -  acme/v1/h-9b2e4c7a1d05/state/logs/rotated
  ~  acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used  value: 1 change(s) (value: 41.0 → 42.0)
  ~  acme/v1/h-9b2e4c7a1d05/state/sysinfo/health                 value: 1 change(s) (status: "degraded" → "ok"); holder: storage_only → live(unknown)
DIFFERENT

"#]]
    );
    let n = notes(&fx::snapshot_diff());
    assert!(n.contains("a: span 1.25s"), "{n}");
    assert!(n.contains("b: span 0.80s"), "{n}");
    assert!(n.contains("no origin alignment was asked"), "{n}");
    assert_data_eq!(
        ndjson(&fx::snapshot_diff()),
        str![[r#"
{"a":{"answered":6,"asked":1,"base":"acme","collected_at":"2026-09-06T00:00:00Z","collection_span_s":1.25,"roster":2,"selectors":["acme/v1/**"],"superseded":1,"zsnap":1},"b":{"answered":5,"asked":1,"base":"acme","collected_at":"2026-09-06T00:05:00Z","collection_span_s":0.8,"roster":2,"selectors":["acme/v1/**"],"zsnap":1},"notes":[{"cite":"RFC 13 §4.4","text":"a: span 1.25s at 2026-09-06T00:00:00Z, b: span 0.80s at 2026-09-06T00:05:00Z — each side was collected over its span, not at an instant"},{"cite":"RFC 09 §5.1 O4","text":"keys compared verbatim — no origin alignment was asked, so the same host under a different origin reads as removed and added"}],"report":"snapshot-diff","unchanged":2}
{"key":"acme/v1/h-9b2e4c7a1d05/telemetry/sysinfo/disk/var-log/used","row":"added"}
{"key":"acme/v1/h-9b2e4c7a1d05/state/logs/rotated","row":"removed"}
{"key":"acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used","row":"changed","timestamp":["7f3b2a1c00000001/ab12","7f3b2a1c00000002/ab12"],"value":{"changes":[{"new":42.0,"old":41.0,"op":"changed","path":"value"}],"truncated":0}}
{"holder":[{"kind":"storage_only","origin":"h-9b2e4c7a1d05"},{"answered_by":"unknown","kind":"live","origin":"h-9b2e4c7a1d05"}],"key":"acme/v1/h-9b2e4c7a1d05/state/sysinfo/health","row":"changed","timestamp":["7f3b2a1c00000001/ab12","7f3b2a1c00000002/cd34"],"value":{"changes":[{"new":"ok","old":"degraded","op":"changed","path":"status"}],"truncated":0}}

"#]]
    );
    // Identity: the clean word, and nothing listed.
    let same = table(&fx::snapshot_diff_identity());
    assert!(same.contains("IDENTICAL"), "{same}");
    assert!(!same.contains("  ~"), "{same}");
}

/// An alignment that was asked and could not place every origin is
/// **refused** (#220): the pairs it had and — the RFC 13 §4.4 MUST — every
/// origin it could not pair, each with its reason, under NOT COMPARED
/// rather than a count line that would read as a comparison; and as rows.
#[test]
fn a_refused_snapshot_diff_lists_every_unpaired_origin_and_compares_nothing() {
    assert_data_eq!(
        table(&fx::snapshot_diff_unmapped()),
        str![[r#"
a: 6 key(s), span 0.90s at 2026-09-06T00:00:00Z (prod/v1/**)
b: 4 key(s), span 0.90s at 2026-09-06T00:05:00Z (stg/v1/**)
origins aligned: 1
  =  h-3fa9c2d41b7e ↔ h-c0ffee00c0de  explicit
origins not paired: 2
  ?  h-9b2e4c7a1d05 (in a)  label `db` claimed by no origin in b; producer set {logs, sysinfo} matches no origin in b
  ?  h-0badcafe1234 (in b)  label `node` claimed by no origin in a; producer set {sysinfo} matches no origin in a
NOT COMPARED

"#]]
    );
    let n = notes(&fx::snapshot_diff_unmapped());
    assert!(n.contains("could not be paired"), "{n}");
    assert!(n.contains("--map A=B"), "{n}");
    let rows: Vec<serde_json::Value> = ndjson(&fx::snapshot_diff_unmapped())
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        rows.iter().filter(|r| r["row"] == "unmapped").count(),
        2,
        "one row per unpaired origin"
    );
    assert!(
        rows.iter()
            .any(|r| r["row"] == "unmapped" && r["side"] == "b")
    );
    assert!(
        !rows.iter().any(|r| r["row"] == "subject"),
        "no roll-up: not compared"
    );
    assert_eq!(rows[0]["origin_map"][0]["evidence"]["kind"], "explicit");
}

/// The acceptance case (#220): one fleet under two deployments, every
/// origin re-minted, aligned on its label — the map table with its
/// evidence column, every subject identical, the clean word.
#[test]
fn an_aligned_snapshot_diff_draws_its_map_and_rolls_up_per_subject() {
    assert_data_eq!(
        table(&fx::snapshot_diff_aligned()),
        str![[r#"
a: 6 key(s), span 0.90s at 2026-09-06T00:00:00Z (prod/v1/**)
b: 6 key(s), span 0.90s at 2026-09-06T00:05:00Z (stg/v1/**)
0 added, 0 removed, 0 changed, 6 unchanged
origins aligned: 2
  =  h-3fa9c2d41b7e ↔ h-c0ffee00c0de  label `web`
  =  h-9b2e4c7a1d05 ↔ h-0badcafe1234  label `db`
4 subject(s) identical on every origin, 0 not
IDENTICAL

"#]]
    );
    let n = notes(&fx::snapshot_diff_aligned());
    assert!(n.contains("re-based from `stg` onto `prod`"), "{n}");
    assert!(!n.contains("no origin alignment was asked"), "{n}");
    assert!(!n.contains("producer set alone"), "labels paired it: {n}");
}

/// One line per subject that differs — "differs on N of M origin(s)" with
/// the example's facets, and only-in counts — and the agreeing subjects
/// counted, not listed.
#[test]
fn a_normalized_snapshot_diff_says_per_subject_on_how_many_origins() {
    assert_data_eq!(
        table(&fx::snapshot_diff_normalized()),
        str![[r#"
a: 6 key(s), span 0.90s at 2026-09-06T00:00:00Z (prod/v1/**)
b: 4 key(s), span 0.90s at 2026-09-06T00:05:00Z (stg/v1/**)
0 added, 2 removed, 2 changed, 2 unchanged
  -  plain/leak
  -  prod/v1/h-9b2e4c7a1d05/state/logs/rotated
  ~  prod/v1/h-3fa9c2d41b7e/state/sysinfo/health  value: 1 change(s) (source: "web" → "node")
  ~  prod/v1/h-9b2e4c7a1d05/state/sysinfo/health  value: 1 change(s) (source: "db" → "node")
origins aligned: 2
  =  h-3fa9c2d41b7e ↔ h-c0ffee00c0de  explicit
  =  h-9b2e4c7a1d05 ↔ h-0badcafe1234  explicit
plain/leak            1 only in a
state/logs/rotated    1 only in a
state/sysinfo/health  differs on 2 of 2 origin(s)  value: 1 change(s) (source: "web" → "node")
1 subject(s) identical on every origin, 3 not
DIFFERENT

"#]]
    );
    let rows: Vec<serde_json::Value> = ndjson(&fx::snapshot_diff_normalized())
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let health = rows
        .iter()
        .find(|r| r["row"] == "subject" && r["subject"] == "state/sysinfo/health")
        .unwrap();
    assert_eq!(
        (health["compared"].as_u64(), health["differing"].as_u64()),
        (Some(2), Some(2))
    );
    assert_eq!(health["example"]["value"]["changes"][0]["path"], "source");
}

/// No health documents at all: the pairing rests on producer sets alone,
/// and the O4 note says the label was never asked rather than absent.
#[test]
fn a_producer_set_pairing_says_the_label_was_not_asked() {
    let (a, b) = fx::snapshot_pair_unlabelled();
    let plan = zenkey_fleet::plan_map(
        &zenkey_fleet::origin_profiles(&a),
        &zenkey_fleet::origin_profiles(&b),
        &[],
    )
    .unwrap();
    let d = zenkey_fleet::diff_normalized(&a, &b, &plan, zenkey_fleet::DiffOpts::default());
    let t = table(&d);
    assert!(t.contains("  producer set"), "{t}");
    assert!(t.contains("IDENTICAL"), "{t}");
    let n = notes(&d);
    assert!(n.contains("paired by producer set alone"), "{n}");
    assert!(n.contains("never asked"), "{n}");
    assert!(n.contains("RFC 06 §6.2"), "{n}");
}

/// The two `.zsnap` files the CLI corpus diffs (`tests/cmd/snapshot-diff.trycmd`)
/// are the shared fixtures, written through the engine's own writer — so a
/// change to the fixture or the dialect moves both corpora together.
/// `SNAPSHOTS=overwrite` (`just snapshots`) rewrites them; otherwise they
/// must already match.
#[test]
fn the_zsnap_corpus_fixtures_are_the_shared_fixtures() {
    let (renamed_a, renamed_b) = fx::snapshot_pair_renamed();
    let (_, ambiguous_b) = fx::snapshot_pair_ambiguous();
    let (unlabelled_a, unlabelled_b) = fx::snapshot_pair_unlabelled();
    for (name, snapshot) in [
        ("a.zsnap", fx::snapshot()),
        ("b.zsnap", fx::snapshot_b()),
        // The two-deployment pairs (#220, `snapshot-diff-normalized.trycmd`).
        ("prod.zsnap", renamed_a),
        ("stg.zsnap", renamed_b),
        ("stg-ambiguous.zsnap", ambiguous_b),
        ("prod-unlabelled.zsnap", unlabelled_a),
        ("stg-unlabelled.zsnap", unlabelled_b),
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

/// The O6 eviction count leads, so `| head -5` cannot lose it.
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
5.00 Hz  v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used  (0 sn gap(s))  lat — (50 unstamped: no HLC, no latency — not zero)
5.00 Hz  v1/h-3fa9c2d41b7e/state/sysinfo/health  (0 sn gap(s))
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
225.0 B/s  v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used
180.0 B/s  v1/h-3fa9c2d41b7e/state/sysinfo/health
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

/// Three row kinds on one stream, told apart by a tag rather than by probing
/// for fields — and an origin whose sources named no single session is
/// *reported*, not attached.
#[test]
fn an_admin_graph_tags_its_three_row_kinds() {
    let report = fx::topology();
    let attachments = fx::attachments();
    let view = zenctl::render::TopologyView {
        report: &report,
        attachments: &attachments,
    };
    let kinds: Vec<String> = ndjson(&view)
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v.get("row").and_then(|r| r.as_str()).map(str::to_string))
        .collect();
    assert_eq!(
        kinds,
        ["node", "node", "edge", "attachment", "attachment"],
        "nodes, edges and attachments used to be one untagged stream"
    );
    assert_data_eq!(
        table(&view),
        str![[r#"
aabbccdd  router  1.9.0  tcp/10.0.0.1:7447
eeff0011  peer    —      (heard of, not queryable)
  h-3fa9c2d41b7e  ⚓ session eeff0011  (token v1/h-3fa9c2d41b7e/state/sysinfo/alive)
  h-bbbbbbbbbbbb  reported by aabbccdd — sources named no single session; shown as reported, not attached
  aabbccdd —— eeff0011  [tcp]

"#]]
    );
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
  presence-in:ops/frontend   presence      ingress  declare_liveliness_subscriber, liveliness_query
      zk2/*/tc/@zk/**
  presence-out:ops/frontend  presence      egress   liveliness_token
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

/// A read-back as a modem serves it: two groups of different classes, a
/// write-only parameter, a value the producer has not read yet, one that
/// differs from its startup file, and a change pending on the reach group.
/// Built here: `zenkey-report-fixtures` pins `zenkey-fleet`'s report
/// shapes, and this document is `zenkey::config`'s (RFC 05 §5.1).
fn config_report() -> zenctl::render::ConfigReport {
    use zenkey::config::{
        ConfigGroup, ConfigSchema, ConfigView, ParamClass, ParamKind, ParamSpec, ParamValue,
        PendingChange, ValueSource,
    };
    let dbm = ParamKind::Integer {
        min: Some(0),
        max: Some(30),
        unit: Some("dBm".into()),
    };
    let schema = ConfigSchema::new()
        .with(
            ConfigGroup::new("radio", ParamClass::Reach, "the carrier")
                .with(ParamSpec::new(
                    "frequency_khz",
                    ParamKind::Integer {
                        min: Some(863_000),
                        max: Some(870_000),
                        unit: Some("kHz".into()),
                    },
                    "centre frequency",
                ))
                .with(ParamSpec::new("tx_power", dbm, "transmit power")),
        )
        .with(
            ConfigGroup::new("access", ParamClass::Hot, "who may join")
                .with(ParamSpec::new(
                    "open",
                    ParamKind::Bool,
                    "accept unknown peers",
                ))
                .with(ParamSpec::new("psk", ParamKind::Text, "the shared key").sensitive()),
        );
    let mut view = ConfigView::of("rf0", &schema);
    view.revision = 7;
    view.pending = Some(PendingChange::new("chg-01j9", ["radio"]).until("2026-09-27T10:15:00Z"));
    let radio = &mut view.groups[0];
    radio.parameters[0].value = Some(ParamValue::Integer(868_100));
    radio.parameters[0].source = Some(ValueSource::Runtime);
    radio.parameters[0].startup = Some(ParamValue::Integer(868_300));
    radio.parameters[1].value = Some(ParamValue::Integer(14));
    radio.parameters[1].source = Some(ValueSource::File);
    let access = &mut view.groups[1];
    access.parameters[1].source = Some(ValueSource::File);
    zenctl::render::ConfigReport {
        key: "acme/v1/h-3fa9c2d41b7e/@rpc/modem/config/rf0".into(),
        timeout_s: 5.0,
        documents: vec![zenctl::render::ConfigDocument {
            origin: "h-3fa9c2d41b7e".into(),
            view,
        }],
        other: vec![],
    }
}

#[test]
fn a_config_read_back_draws_the_schema_beside_every_value() {
    assert_data_eq!(
        table(&config_report()),
        str![[r#"
h-3fa9c2d41b7e  rf0  revision 7  pending chg-01j9 on radio until 2026-09-27T10:15:00Z
parameter      value                    source   kind                         description

radio  (reach) the carrier
frequency_khz  868100 (startup 868300)  runtime  integer 863000..=870000 kHz  centre frequency
tx_power       14                       file     integer 0..=30 dBm           transmit power

access  (hot) who may join
open           —                        —        bool                         accept unknown peers
psk            (write-only)             file     text                         the shared key

"#]]
    );
    assert_data_eq!(
        ndjson(&config_report()),
        str![[r#"
{"documents":1,"key":"acme/v1/h-3fa9c2d41b7e/@rpc/modem/config/rf0","other":0,"report":"config","timeout_s":5.0}
{"class":"reach","description":"centre frequency","group":"radio","kind":"integer","max":870000,"min":863000,"name":"frequency_khz","origin":"h-3fa9c2d41b7e","resource":"rf0","revision":7,"row":"parameter","source":"runtime","startup":868300,"unit":"kHz","value":868100}
{"class":"reach","description":"transmit power","group":"radio","kind":"integer","max":30,"min":0,"name":"tx_power","origin":"h-3fa9c2d41b7e","resource":"rf0","revision":7,"row":"parameter","source":"file","unit":"dBm","value":14}
{"class":"hot","description":"accept unknown peers","group":"access","kind":"bool","name":"open","origin":"h-3fa9c2d41b7e","resource":"rf0","revision":7,"row":"parameter"}
{"class":"hot","description":"the shared key","group":"access","kind":"text","name":"psk","origin":"h-3fa9c2d41b7e","resource":"rf0","revision":7,"row":"parameter","sensitive":true,"source":"file"}
{"deadline":"2026-09-27T10:15:00Z","groups":["radio"],"origin":"h-3fa9c2d41b7e","resource":"rf0","row":"pending","token":"chg-01j9"}

"#]]
    );
}

/// RFC v1.50: the change `persist` takes with no token is named on the
/// document's head line and as a `last_change` row.
#[test]
fn a_config_read_back_names_its_last_change() {
    let mut r = config_report();
    let view = &mut r.documents[0].view;
    view.pending = None;
    view.last_change = Some(zenkey::config::LastChange::new("chg-01j8", ["access"]));
    let table = table(&r);
    assert_eq!(
        table.lines().next(),
        Some("h-3fa9c2d41b7e  rf0  revision 7  last change chg-01j8 on access"),
        "{table}"
    );
    let rows = ndjson(&r);
    assert!(
        rows.lines().any(|l| l
            == r#"{"groups":["access"],"origin":"h-3fa9c2d41b7e","resource":"rf0","row":"last_change","token":"chg-01j8"}"#),
        "{rows}"
    );
}

/// The reply that is not a document is kept and drawn as a reply, and the
/// caveat says so in every format; an empty report says silence.
#[test]
fn a_config_report_keeps_a_reply_that_is_not_a_document() {
    use zenkey_fleet::report::{CallAnswer, CallError, CallOutcome};
    let mut r = config_report();
    r.documents.clear();
    r.other.push(CallAnswer {
        origin: "h-3fa9c2d41b7e".into(),
        outcome: CallOutcome::Ok {
            value: Some(
                serde_json::json!({"token": "chg-01j9", "apply_at": "2026-09-27T10:14:30Z"}),
            ),
            text: None,
        },
        attachment: None,
        attachment_bytes: None,
    });
    r.other.push(CallAnswer {
        origin: "h-5c1d2e3f4a5b".into(),
        outcome: CallOutcome::Err(CallError {
            name: "error/busy".into(),
            message: "a change is pending on this resource (token chg-0ff1)".into(),
        }),
        attachment: None,
        attachment_bytes: None,
    });
    assert_data_eq!(
        table(&r),
        str![[r#"
h-3fa9c2d41b7e:
{
  "apply_at": "2026-09-27T10:14:30Z",
  "token": "chg-01j9"
}
h-5c1d2e3f4a5b: ✗ error/busy — a change is pending on this resource (token chg-0ff1)

"#]]
    );
    assert_data_eq!(
        notes(&r),
        str![[r#"
2 repl(y|ies) not shaped as a read-back document, shown as sent (RFC 05 §5.1)

"#]]
    );
    let silent = zenctl::render::ConfigReport {
        documents: vec![],
        other: vec![],
        ..config_report()
    };
    assert!(
        notes(&silent).contains("no replies to"),
        "{}",
        notes(&silent)
    );
}

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

/// Nothing to say is *absent*, not an empty array meaning the same thing.
#[test]
fn a_schema_check_omits_an_empty_detail_list() {
    let valid = zenctl::render::SchemaCheck {
        type_name: "Health".into(),
        kind: "json-schema".into(),
        verdict: zenctl::render::SchemaCheckVerdict::Valid,
        detail: vec![],
    };
    let doc: serde_json::Value = serde_json::from_str(ndjson(&valid).trim()).unwrap();
    assert_eq!(
        doc,
        serde_json::json!({
            "report": "schema-check",
            "type": "Health",
            "kind": "json-schema",
            "verdict": "valid",
        })
    );
    assert_eq!(table(&valid), "Health (json-schema): valid\n");

    let invalid = zenctl::render::SchemaCheck {
        verdict: zenctl::render::SchemaCheckVerdict::Invalid,
        detail: vec![r#"/status: "melted" is not one of "ok", "degraded" or "down""#.into()],
        ..valid
    };
    assert_data_eq!(
        table(&invalid),
        str![[r#"
Health (json-schema): invalid
  /status: "melted" is not one of "ok", "degraded" or "down"

"#]]
    );
}

/// The cache is this tool's own disk footprint, and a script is a user:
/// `cache show --format json | jq -r .dir` is the point of the command (#54).
#[test]
fn a_cache_report_names_its_directory_in_both_formats() {
    let full = zenctl::render::CacheReport {
        dir: "/home/u/.cache/zenkey-explorer/lab/slices".into(),
        slices: vec![zenctl::render::CachedSlice {
            producer: "sysinfo".into(),
            registry_version: "1.0".into(),
            subjects: 41,
            procedures: 3,
        }],
    };
    assert_data_eq!(
        table(&full),
        str![[r#"
/home/u/.cache/zenkey-explorer/lab/slices
  sysinfo  registry 1.0  41 subject(s), 3 procedure(s)

"#]]
    );
    let doc: serde_json::Value =
        serde_json::from_str(ndjson(&full).lines().next().unwrap()).unwrap();
    assert_eq!(doc["dir"], "/home/u/.cache/zenkey-explorer/lab/slices");

    let empty = zenctl::render::CacheReport {
        dir: "/home/u/.cache/zenkey-explorer/default/slices".into(),
        slices: vec![],
    };
    assert!(notes(&empty).contains("falls back to the static command tree"));
}

/// A GET's coverage claim is exactly its selector and its window, and a silent
/// one has to say so — three different silences, not one.
#[test]
fn a_get_with_no_replies_names_the_three_silences() {
    let silent = zenctl::render::GetReport {
        selector: "acme/v1/**/state/**".into(),
        timeout_s: 5.0,
        elided: 0,
        answers: vec![],
    };
    let n = notes(&silent);
    assert!(n.contains("Nobody is registered for it"), "{n}");
    assert!(n.contains("the three are different"), "{n}");
    let doc: serde_json::Value =
        serde_json::from_str(ndjson(&silent).lines().next().unwrap()).unwrap();
    assert_eq!(doc["selector"], "acme/v1/**/state/**");
    // `5.0`: the seconds unification (#218) — see the budget window above.
    assert_eq!(doc["timeout_s"], 5.0);
}

/// Synthetic traffic is still publishing, so the plan is a dry run made
/// visible before a byte moves — the `replay --dry-run` precedent
/// (RFC 09 §5.3).
#[test]
fn a_gen_plan_states_the_synthetic_marker_before_anything_is_published() {
    let entries: Vec<zenkey_fleet::report::GenPlanEntry> = vec![];
    let plan = zenctl::render::GenPlan {
        origin: "h-3fa9c2d41b7e",
        duration_s: 5.0,
        entries: &entries,
    };
    let n = notes(&plan);
    assert!(n.contains("synthetic"), "{n}");
    assert!(n.contains("RFC 09 §5.3"), "{n}");

    // A faulted plan states each fault it will inject, per key and in the
    // notes — the tool says what it is about to do to the bus (#163).
    let faulted = vec![zenkey_fleet::report::GenPlanEntry {
        key: "v1/h-3fa9c2d41b7e/state/demo/health".into(),
        class: "state".into(),
        producer: "demo".into(),
        type_name: "Health".into(),
        qos: "transition".into(),
        qos_source: "declared",
        rate_hz: 1.0,
        body_source: "schema-set",
        encoding: Some("application/json".into()),
        events_cap: None,
        note: None,
        fault: Some(zenkey_fleet::report::Fault::Truncate),
        fault_delta: Some("payload truncated to half its encoded bytes — a partial frame".into()),
        schema: None,
        unique_chunk: None,
    }];
    let faulted_plan = zenctl::render::GenPlan {
        origin: "h-3fa9c2d41b7e",
        duration_s: 5.0,
        entries: &faulted,
    };
    let t = table(&faulted_plan);
    assert!(t.contains("FAULT[truncate]"), "{t}");
    assert!(t.contains("partial frame"), "{t}");
    assert!(notes(&faulted_plan).contains("injecting fault(s): truncate"));

    let report = zenkey_fleet::report::GenReport {
        duration_s: 5.0,
        entries: 12,
        sent: 240,
        refused: 2,
        first_errors: vec!["health: enum `status` has no synthesizable member".into()],
    };
    assert_data_eq!(
        table(&report),
        str![[r#"
sent 240 sample(s) over 5.0s across 12 subject(s); 2 refused by schema
  ✗  health: enum `status` has no synthesizable member

"#]]
    );
    assert!(notes(&report).contains("counted, never silently"));
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
        "blob-fetch",
        "blob-list",
        "blob-probe",
        "blob-tree",
        "cache",
        "cache-action",
        "call",
        "compat",
        "config",
        "conform",
        "context",
        "context-action",
        "context-list",
        "cutover",
        "doctor",
        "expect",
        "export",
        "field",
        "gen",
        "gen-plan",
        "get",
        "graph",
        "iface-list",
        "iface-show",
        "key-canon",
        "key-relation",
        "namespace-list",
        "operation",
        "probe",
        "rate",
        "record",
        "registry-retired",
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
    assert_eq!(s.asked, ["acme/v1/**/state/**"]);
    assert_eq!(s.window_s, Some(5.0));
    let s = scoped(&fx::cutover_report());
    assert_eq!(s.asked.len(), 2, "both halves of the claim: {:?}", s.asked);
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
    assert_eq!(s.asked, ["acme/v1/**"]);
    assert_eq!(s.window_s, Some(10.0));

    // A snapshot's window is its collection span — the RFC 13 §4.4 fact
    // every rendering states (#219).
    let s = scoped(&fx::snapshot_report());
    assert_eq!(s.asked, ["acme/v1/**"]);
    assert_eq!(s.window_s, Some(1.25));

    // The exporter's scope is its selector, over the span it has been
    // watching (taken_at - started_at).
    let s = scoped(&fx::export_snapshot());
    assert_eq!(s.asked, ["acme/v1/*/**"]);
    assert_eq!(s.window_s, Some(120.0));
    // The conformance suite's scope is every origin it called, plus its
    // listen window's selectors.
    let s = scoped(&fx::conform_report());
    assert_eq!(s.asked, ["h-3fa9c2d41b7e/@rpc/sysinfo", "v1/*/state/**"]);
    assert_eq!(s.window_s, Some(10.0));
    // The doctor's scope is what it read: presence in the namespace, the
    // admin space in none.
    let s = scoped(&fx::doctor_report());
    assert_eq!(s.asked, ["zk2/*/*/@zk/**", "@/*/router"]);
    assert_eq!(s.window_s, None);
    // `storage gen --check` sweeps the admin space once, no window.
    let s = scoped(&fx::storage_check());
    assert_eq!(s.asked, ["@/*/router/**/storage_manager/storages/**"]);
    assert_eq!(s.window_s, None);
    // The burn-down: one asked selector per ledger entry.
    let s = scoped(&fx::retired_report());
    assert_eq!(s.asked.len(), 3);
    assert_eq!(s.window_s, Some(30.0));

    // GET-shaped asks: the wait is the window (R5/P1's `timeout_s`).
    let s = scoped(&fx::call_report());
    assert_eq!(s.window_s, Some(5.0));
    let s = scoped(&fx::probe_report());
    assert_eq!(s.window_s, Some(5.0), "the probe's observation IS the call");
    let s = scoped(&zenctl::render::GetReport {
        selector: "acme/v1/**/state/**".into(),
        timeout_s: 5.0,
        elided: 0,
        answers: vec![],
    });
    assert_eq!(s.asked, ["acme/v1/**/state/**"]);
    scoped(&fx::bench_report());
    scoped(&fx::why_report());
    // A config read is a GET: its key, over its wait.
    let s = scoped(&config_report());
    assert_eq!(s.asked, ["acme/v1/h-3fa9c2d41b7e/@rpc/modem/config/rf0"]);
    assert_eq!(s.window_s, Some(5.0));

    // Sweeps: asked is the claim; a one-shot sweep has no window.
    scoped(&fx::scout_report());
    let s = scoped(&fx::router_list());
    assert_eq!(s.window_s, None);
    let report = fx::topology();
    let attachments = fx::attachments();
    scoped(&zenctl::render::TopologyView {
        report: &report,
        attachments: &attachments,
    });
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
    let s = scoped(&fx::blob_probe());
    assert_eq!(
        s.asked.len(),
        2,
        "probe wide: both selectors state themselves"
    );

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
        slices: None,
        existed: true,
    };
    assert!(notes(&removed).starts_with("removed /home/u"));
    let absent = zenctl::render::CacheAction {
        action: "cleared",
        dir: "/home/u/.cache/zenkey-explorer/lab/slices".into(),
        slices: None,
        existed: false,
    };
    assert!(notes(&absent).contains("does not exist — nothing to clear"));
    let doc: serde_json::Value =
        serde_json::from_str(ndjson(&absent).lines().next().unwrap()).unwrap();
    assert_eq!(doc["existed"], false);
    assert!(
        doc.get("slices").is_none(),
        "clear counts nothing — absent, not zero (RFC 09 §5.1 O4)"
    );
}

// ── The storage-plan families (#393) ──────────────────────────────────────

/// The plan as a table: one row per volume and storage, the derivation and
/// every warning as detail lines, the refusal and the registry claim as notes
/// — and, on the wire, the same facts under `row` tags.
#[test]
fn a_storage_plan_shows_its_derivations_and_names_its_refusals() {
    assert_data_eq!(
        table(&fx::storage_plan()),
        str![[r#"
storage plan for base "acme"  (registry: 3 slice(s), longest state ttl_s 900 (sysinfo/alert/{alert_key}))

volumes:
  fs        fs        durable · latest
  influxdb  influxdb  durable · all     url="http://localhost:8086"

storages:
  catalog       acme/v1/@catalog/state/**       fs (latest)
    strip acme/v1/@catalog/state  ·  covers 3 declared subject(s)
    gc lifespan 172800 s (period 30 s): max ttl_s 86400 (catalog/pdns/{ip_slug}) × 2.0 = 172800 s
    ! complete_refused: complete = true refused: it is not the fully covering latest storage (class state) — emitted as false (RFC 09 §2.2)
    ! overlap: overlaps pdns_history (acme/v1/@catalog/state/pdns/**): a GET under both selectors is answered by both (RFC 09 §2)
  latest        acme/v1/*/state/**              fs (latest)     replicated, complete
    strip acme/v1  ·  covers 12 declared subject(s)
    gc lifespan 1800 s (period 30 s): max ttl_s 900 (sysinfo/alert/{alert_key}) × 2.0 = 1800 s
  pdns_history  acme/v1/@catalog/state/pdns/**  influxdb (all)
    strip acme/v1/@catalog/state/pdns  ·  covers 1 declared subject(s)
    gc lifespan 172800 s (period 30 s): max ttl_s 86400 (catalog/pdns/{ip_slug}) × 2.0 = 172800 s
    ! retention_is_the_databases: retention is the database's policy, not zenoh config (RFC 09 §2.3)

"#]]
    );
    let notes = notes(&fx::storage_plan());
    assert!(notes.contains("refused storage events:"), "{notes}");
    assert!(notes.contains("3 storage(s) on 2 volume(s) planned, 1 refused"));
    let out = ndjson(&fx::storage_plan());
    let mut lines = out.lines();
    let envelope: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(envelope["report"], "storage-plan");
    assert_eq!(envelope["registry"]["max_ttl_s"], 900);
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

/// Without a registry the plan says so in every format — the O4 sentence is
/// a coverage note, so it rides the json document too.
#[test]
fn a_storage_plan_without_a_registry_says_what_it_could_not_verify() {
    let plan = zenkey_fleet::report::StoragePlan {
        registry: zenkey_fleet::report::Asked::NotAsked,
        ..fx::storage_plan()
    };
    assert!(table(&plan).contains("(registry: not asked)"));
    let notes = notes(&plan);
    assert!(notes.contains("no registry was asked"), "{notes}");
    let out = ndjson(&plan);
    let envelope: serde_json::Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    assert!(envelope.get("registry").is_none(), "not asked is absence");
    assert!(envelope["notes"].to_string().contains("RFC 09 §5.1 O4"));
}

/// The check: one line per finding, the unjudged comparison as a coverage
/// note, and the empty admin sweep as a non-verdict rather than a pass.
#[test]
fn a_storage_check_draws_each_finding_and_keeps_unjudged_apart() {
    assert_data_eq!(
        table(&fx::storage_check()),
        str![[r#"
storage check for base "acme": 3 planned, 3 observed row(s) — 4 finding(s)
  ✗ latest@aabbccdd  strip_prefix differs           planned acme/v1, observed acme
  ✗ latest@aabbccdd  gc.lifespan below the minimum  planned 1800, observed 600
  ✗ pdns_history     missing                        planned acme/v1/@catalog/state/pdns/**
  ✗ blobs@aabbccdd   extra                          observed acme/v1/*/@blob/**

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
storage check for base "acme": 3 planned, 0 observed row(s) — no verdict — the admin space answered no storages

"#]]
    );
    assert!(notes(&fx::storage_check_unobservable()).contains("RFC 05 §3.1"));
}

/// `--explain`: the taker with its reason, or the reason there is none.
#[test]
fn a_storage_explain_names_the_taker_or_the_reason() {
    assert_data_eq!(
        table(&fx::storage_explain()),
        str![[r#"
acme/v1/h-3fa9c2d41b7e/state/sysinfo/health
  → latest  acme/v1/*/state/**  includes it
      class state under base "acme": acme/v1/*/state/** includes every key it names; stored under strip_prefix "acme/v1" on volume fs (latest)

"#]]
    );
    assert_data_eq!(
        table(&fx::storage_explain_none()),
        str![[r#"
acme/v1/h-3fa9c2d41b7e/events/netring/capture/01J
  none: no planned storage's selector includes it; refused storage(s) events would have

"#]]
    );
    let out = ndjson(&fx::storage_explain_none());
    let envelope: serde_json::Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    assert_eq!(envelope["refused_takers"], serde_json::json!(["events"]));
    assert_eq!(out.lines().count(), 1, "no takers, no rows");
}

// ── The consumers join (#224) ─────────────────────────────────────────────

// ── The metrics surface (#228) ───────────────────────────────────────────────

/// `export --once`: series grouped by producer, an empty value cell where
/// the state says the series stopped, and the state beside every row.
#[test]
fn an_export_snapshot_groups_series_by_producer_and_blanks_a_stopped_value() {
    assert_data_eq!(
        table(&fx::export_snapshot()),
        str![[r#"
series                                       labels                                        value  state        last seen   samples

sysinfo  (telemetry)
zenkey_subject_sysinfo_cpu_usage_percent     origin=h-3fa9c2d41b7e                        12.500  live         1700000119      240
zenkey_subject_sysinfo_disk_used_bytes       origin=h-0000deadbeef mount=var-log                  origin_down  1700000040       80

netlink  (telemetry)
zenkey_subject_netlink_iface_rx_bytes_total  origin=h-3fa9c2d41b7e iface=eth0 field=rx            evicted      1700000100        5

sysinfo  (state)
zenkey_subject_sysinfo_health                origin=h-3fa9c2d41b7e field=uptime_s       4242.000  quiet        1700000060        4

"#]]
    );
}

/// The ndjson leads with the envelope — scopes, exclusions, the observer's
/// counters as separate fields — then one tagged row per series.
#[test]
fn an_export_snapshots_ndjson_leads_with_the_envelope_then_tags_every_series() {
    assert_data_eq!(
        ndjson(&fx::export_snapshot()),
        str![[r#"
{"contract":{"payload_invalid":1,"payload_not_validated":128,"payload_valid":200,"qos_judged":320,"qos_mismatch":2,"qos_mismatch_by_subject":[{"n":2,"producer":"sysinfo","subject":"cpu/usage"}]},"doctor":{"findings":[{"check":"split-brain","severity":"error","subject":"host-a/tc tc.netif.v1"}],"ran_at_unix_s":1700000090},"excluded":["@rpc","@media","@blob","@adv","service origins"],"max_series":10000,"notes":[{"cite":"RFC 03 §4 D2","text":"a wildcard selector never crosses an `@`-chunk: @rpc, @media, @blob, @adv, service origins are excluded from this surface, not empty"},{"cite":"RFC 09 §5.1 O4","text":"3 distinct key(s) the registry does not declare are counted, never exported — the contract is the registry"},{"text":"128 sample(s) not validated (past the decode budget, or no schema) — a third population beside 200 valid and 1 invalid, never folded into a ratio"},{"text":"2 series stopped (evicted, origin_down or retired): each keeps its labels and state and exposes no value, so a scraper sees a named absence rather than a flat line"},{"text":"quiet is judged only for `state` subjects against their declared ttl_s; telemetry declares no period and is never called quiet"},{"cite":"RFC 09 §5.1 O7","text":"last seen is this observer's wall clock at arrival, never the producer's"},{"cite":"RFC 09 §5.1 O6","text":"3 sample(s) dropped while behind — every value is a lower bound while this moves"},{"cite":"RFC 09 §5.1 O6","text":"5 key(s) retired at the stats-table bound; their series read `evicted`"},{"cite":"RFC 09 §5.1 O6","text":"7 retained sample(s) dropped at the byte budget"},{"cite":"RFC 09 §5.1 O6","text":"11 retained sample(s) aged out of the retention window"},{"cite":"RFC 09 §5.1 O6","text":"13 key(s) retired because their watch was released"},{"cite":"RFC 09 §5.1 O6","text":"17 sample(s) coalesced between scrapes — only the newest value per series is exposed"},{"cite":"RFC 09 §5.1 O6","text":"4 sample(s) refused a series past the declared `cardinality` budget"},{"cite":"RFC 09 §5.1 O6","text":"1 field(s) refused a series past the per-subject field cap"}],"observer":{"coalesced":17,"dropped":3,"evicted_bytes":7,"evicted_keys":5,"expired":11,"unstamped":19,"unwatched":13},"registry":{"producers":2},"report":"export","scopes":["acme/v1/*/**"],"started_at_unix_s":1700000000,"suppressed":{"cardinality":4,"fields":1},"taken_at_unix_s":1700000120,"unregistered_keys":3}
{"class":"telemetry","drop_exposed":2,"key":"acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu/usage","kind":"gauge","last_seen_unix_s":1700000119,"name":"zenkey_subject_sysinfo_cpu_usage_percent","origin":"h-3fa9c2d41b7e","producer":"sysinfo","row":"series","samples":240,"state":"live","subject":"cpu/usage","unit":"percent","value":12.5}
{"class":"telemetry","key":"acme/v1/h-0000deadbeef/telemetry/sysinfo/disk/var-log/used","labels":{"mount":"var-log"},"last_seen_unix_s":1700000040,"name":"zenkey_subject_sysinfo_disk_used_bytes","origin":"h-0000deadbeef","producer":"sysinfo","row":"series","samples":80,"state":"origin_down","subject":"disk/{mount}/used","unit":"bytes"}
{"class":"telemetry","field":"rx","key":"acme/v1/h-3fa9c2d41b7e/telemetry/netlink/iface/eth0/rx_bytes","kind":"counter","labels":{"iface":"eth0"},"last_seen_unix_s":1700000100,"name":"zenkey_subject_netlink_iface_rx_bytes_total","origin":"h-3fa9c2d41b7e","producer":"netlink","row":"series","samples":5,"state":"evicted","subject":"iface/{iface}/rx_bytes","unit":"bytes"}
{"class":"state","field":"uptime_s","key":"acme/v1/h-3fa9c2d41b7e/state/sysinfo/health","last_seen_unix_s":1700000060,"name":"zenkey_subject_sysinfo_health","origin":"h-3fa9c2d41b7e","producer":"sysinfo","row":"series","samples":4,"state":"quiet","subject":"health","value":4242.0}

"#]]
    );
}

/// The honesty floor for the surface, asserted rather than snapshotted:
/// every O6 population is its own bound note, never a sum; the three
/// payload populations stay three; the unasked poles are coverage notes.
#[test]
fn an_export_snapshot_states_every_bound_by_kind_and_never_sums_them() {
    let n = notes(&fx::export_snapshot());
    for expected in [
        "3 sample(s) dropped while behind",
        "5 key(s) retired at the stats-table bound",
        "7 retained sample(s) dropped at the byte budget",
        "11 retained sample(s) aged out",
        "13 key(s) retired because their watch was released",
        "17 sample(s) coalesced between scrapes",
        "4 sample(s) refused a series past the declared `cardinality` budget",
        "1 field(s) refused a series past the per-subject field cap",
    ] {
        assert!(n.contains(expected), "missing `{expected}` in:\n{n}");
    }
    assert!(
        !n.contains(" 23 ") && !n.contains(" 36 ") && !n.contains(" 56 "),
        "no sum of the kinds:\n{n}"
    );
    assert!(
        n.contains("128 sample(s) not validated") && n.contains("200 valid and 1 invalid"),
        "{n}"
    );
    assert!(n.contains("2 series stopped"), "{n}");
    assert!(
        n.contains("@rpc, @media, @blob, @adv, service origins are excluded"),
        "{n}"
    );

    // The unasked poles, on a snapshot that asked for nothing.
    let mut bare = fx::export_snapshot();
    bare.doctor = zenkey_fleet::report::Asked::NotAsked;
    bare.registry = zenkey_fleet::report::Asked::NotAsked;
    bare.contract.payload_valid = 0;
    bare.contract.payload_invalid = 0;
    let n = notes(&bare);
    assert!(n.contains("doctor not asked"), "{n}");
    assert!(n.contains("no registry loaded"), "{n}");
    assert!(n.contains("payload verdicts not asked"), "{n}");
    assert!(n.contains("RFC 09 §5.1 O4"), "{n}");
}

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
            attachment: None,
        },
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
        ..sample
    };
    assert!(zenctl::render::sample_lines(&delete)[1].contains("not an empty value"));
    let summary = WatchSummary {
        address: "*/tc".into(),
        iface: "tc.netif.v1".into(),
        fingerprint: actfx::fp(),
        resource: "stream/bandwidth/{ns}/{iface}".into(),
        selectors: vec!["zk2/*/tc/tc.netif.v1/stream/bandwidth/*/*".into()],
        received: 4,
        discarded: 10,
        lagged: 3,
        elapsed_s: 2.0,
        ended: WatchEnd::Window,
    };
    let lines = zenctl::render::summary_lines(&summary);
    assert_eq!(lines.len(), 3, "{lines:#?}");
    assert!(lines[1].starts_with("10 sample(s)") && lines[1].contains("(R6)"));
    assert!(lines[2].starts_with("3 sample(s)") && lines[2].contains("lower bound"));
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
