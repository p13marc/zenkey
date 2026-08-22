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

#[test]
fn a_topic_list_groups_by_producer_and_tails_what_is_open_ended() {
    assert_data_eq!(
        table(&fx::topic_list()),
        str![[r#"
sysinfo  (registry 1.0)
  telemetry  disk/{mount}/used              TelemetryPoint
  state      health                         HealthSnapshot

logs  (registry 2.0)
  telemetry  by_unit/{unit}/messages_total  TelemetryPoint  [open-ended]
  telemetry  ingest/legacy_total            TelemetryPoint  [open-ended]  DEPRECATED since 2.0 → disk/{mount}/bytes_used

"#]]
    );
}

#[test]
fn a_topic_lists_ndjson_leads_with_the_envelope_then_tags_every_row() {
    assert_data_eq!(
        ndjson(&fx::topic_list()),
        str![[r#"
{"notes":[{"text":"2 are open-ended ({var...}): the registry fixes their shape, not their members. Use `zenctl topic echo` to see what a live fleet actually publishes"}],"report":"topic-list"}
{"class":"telemetry","open_ended":false,"path":"disk/{mount}/used","producer":"sysinfo","registry_version":"1.0","row":"subject","type_name":"TelemetryPoint"}
{"class":"state","open_ended":false,"path":"health","producer":"sysinfo","registry_version":"1.0","row":"subject","type_name":"HealthSnapshot"}
{"class":"telemetry","open_ended":true,"path":"by_unit/{unit}/messages_total","producer":"logs","registry_version":"2.0","row":"subject","type_name":"TelemetryPoint"}
{"class":"telemetry","deprecated":true,"deprecated_since":"2.0","open_ended":true,"path":"ingest/legacy_total","producer":"logs","registry_version":"2.0","replaced_by":"disk/{mount}/bytes_used","row":"subject","since":"1.0","type_name":"TelemetryPoint"}

"#]]
    );
}

/// The O4 distinction, drawn: a producer whose slice was read and said nothing
/// reads "(no served slice)"; one nobody asked about reads `—`. The two used
/// to be the same blank.
#[test]
fn a_node_list_draws_no_slice_served_differently_from_never_asked() {
    assert_data_eq!(
        table(&fx::node_list()),
        str![[r#"
h-3fa9c2d41b7e
  sysinfo   (app zensight, registry 1.0)
  parallax  (no served slice)

@catalog
  catalog   (app zensight, registry 1.1)

"#]]
    );
    assert_data_eq!(
        table(&fx::node_list_unjoined()),
        str![[r#"
h-3fa9c2d41b7e
  sysinfo  —

"#]]
    );
    // And the envelope is what tells a *script* which of the two it is looking
    // at, since a row carries no `app` either way.
    assert!(ndjson(&fx::node_list()).contains(r#""slices_joined":true"#));
    assert!(ndjson(&fx::node_list_unjoined()).contains(r#""slices_joined":false"#));
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

/// `topic info --format ndjson` is **one line**. It used to be a pretty
/// multi-line document, which is not ndjson (#232 item 3).
#[test]
fn a_topic_info_is_one_ndjson_line_and_its_labels_line_up() {
    let out = ndjson(&fx::topic_info());
    assert_eq!(out.lines().count(), 1, "one line, not a document:\n{out}");
    serde_json::from_str::<serde_json::Value>(out.trim()).expect("and it parses");

    assert_data_eq!(
        table(&fx::topic_info()),
        str![[r#"
key       v1/h-3fa9c2d41b7e/state/sysinfo/health
verdict   registered
origin    h-3fa9c2d41b7e
producer  sysinfo
class     state
subject   health
payload   HealthSnapshot
  (`zenctl interface show HealthSnapshot --schema` for the served shape)
qos       refreshed
since     1.0
ttl       120s  (refresh <= 60s; stale after 120s)

"#]]
    );
}

/// The fields below the ladder's failure point are **absent**, not null — and
/// asserted as a whole document, because `json["absent"]` is `Null` and a
/// field-by-field check cannot tell the two apart.
#[test]
fn an_unregistered_key_omits_what_it_never_reached() {
    let line = ndjson(&fx::topic_info_unregistered());
    let doc: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(
        doc,
        serde_json::json!({
            "report": "topic-info",
            "key": "v1/h-3fa9c2d41b7e/telemetry/sysinfo/not/a/real/subject",
            "verdict": "unregistered",
            "note": "the producer serves a slice and it does not declare this subject",
            "origin": "h-3fa9c2d41b7e",
            "producer": "sysinfo",
            "class": "state",
            "subject": "health",
        }),
        "no payload_type, no qos, no ttl_s — the ladder never got there"
    );
}

/// A doctor run says what it *checked*, not only what it found — and the
/// coverage paragraph reaches the machine formats, which is what it never did
/// before the seam.
#[test]
fn a_doctor_run_carries_its_coverage_and_its_bound_into_every_format() {
    let stderr = notes(&fx::doctor_report());
    assert!(stderr.contains("2 introspect repl"), "{stderr}");
    assert!(stderr.contains("3 dropped"), "the O6 bound: {stderr}");

    let envelope: serde_json::Value =
        serde_json::from_str(ndjson(&fx::doctor_report()).lines().next().unwrap()).unwrap();
    let notes = envelope["notes"]
        .as_array()
        .expect("notes ride the envelope");
    assert!(
        notes
            .iter()
            .any(|n| n["text"].as_str().unwrap().contains("dropped")),
        "the bound reaches a script too: {notes:?}"
    );
    assert!(
        !envelope.as_object().unwrap().contains_key("findings"),
        "findings are rows, not an envelope field"
    );
}

/// Every family renders a table that is byte-stable at a fixed width, with no
/// trailing whitespace anywhere — the property that makes the snapshots above
/// reviewable at all.
#[test]
fn no_family_emits_trailing_whitespace() {
    let renderings = [
        table(&fx::topic_list()),
        table(&fx::node_list()),
        table(&fx::storage_list()),
        table(&fx::topic_info()),
        table(&fx::doctor_report()),
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

#[test]
fn a_service_list_marks_a_procedure_that_declares_no_reply() {
    assert_data_eq!(
        table(&fx::service_list()),
        str![[r#"
sysinfo  (registry 1.0)
  read   processes  → ProcessList
  write  gc         —

catalog  (registry 1.1)
  read   names      → Vec<NameVal>

"#]]
    );
    assert_data_eq!(
        ndjson(&fx::service_list()),
        str![[r#"
{"report":"service-list"}
{"kind":"read","path":"processes","producer":"sysinfo","registry_version":"1.0","reply":"ProcessList","request":"ProcessQuery","row":"procedure"}
{"kind":"write","path":"gc","producer":"sysinfo","registry_version":"1.0","row":"procedure"}
{"kind":"read","path":"names","producer":"catalog","registry_version":"1.1","reply":"Vec<NameVal>","row":"procedure"}

"#]]
    );
}

#[test]
fn a_service_info_lays_a_procedure_out_over_four_lines() {
    assert_data_eq!(
        table(&fx::service_info()),
        str![[r#"
producer  catalog  (registry 1.1)
origin    @catalog  (a service origin — its keys carry no producer chunk)
about     the fleet's entity registry

2 procedure(s):

  write  link
           v1/@catalog/@rpc/catalog/link
           LinkRequest → OperatorAssertion
           fanout forbidden — no fleet spelling exists; call one origin
           idempotent false
           assert that two ids are one entity
  read   names
           v1/@catalog/@rpc/catalog/names
           → Vec<NameVal>
           idempotent true

"#]]
    );
}

#[test]
fn an_interface_list_counts_carriers() {
    assert_data_eq!(
        table(&fx::interface_list()),
        str![[r#"
declared payload types:

  TelemetryPoint  42 carrier(s)
  HealthSnapshot  3 carrier(s)

"#]]
    );
}

/// Two producers, same type name, different hashes — the RFC 08 §7 drift
/// finding on the type's own page.
#[test]
fn an_interface_show_names_a_schema_disagreement() {
    assert_data_eq!(
        table(&fx::interface_show()),
        str![[r#"
type      HealthSnapshot

carried by 2 subject(s)/procedure(s):
  sysinfo  state  health
  gnmi     state  health

served schema (RFC 08 §7):
  sysinfo  json-schema  sha256:aaaa
  gnmi     json-schema  sha256:bbbb

"#]]
    );
    assert!(notes(&fx::interface_show()).contains("disagree about"));
}

/// The empty base is a real deployment, not a missing value, so it renders
/// `(empty)` rather than `—`.
#[test]
fn a_base_list_names_the_empty_base_rather_than_dashing_it() {
    assert_data_eq!(
        table(&fx::base_list()),
        str![[r#"
acme     2 origin(s)  2 producer(s)    storage: main@aabbccdd
(empty)  1 origin(s)  1 producer(s)
staging  0 origin(s)  0 producer(s)    storage: archive@eeff0011  (storage config only — nothing alive)

"#]]
    );
}

#[test]
fn a_bench_report_right_aligns_its_numbers_and_counts_non_answers_apart() {
    assert_data_eq!(
        table(&fx::bench_report()),
        str![[r#"
→ v1/h-3fa9c2d41b7e/@rpc/sysinfo/processes
98 call(s), concurrency 8, 2.50s — 39.2 calls/s
  2 of 100 calls did not complete

origin          replies  min ms  p50 ms  p95 ms  p99 ms  max ms
h-3fa9c2d41b7e       64    0.80    1.90   12.40   40.10  123.46
h-bbbbbbbbbbbb       34    1.10    2.20    9.90   11.00   12.50

"#]]
    );
    assert!(notes(&fx::bench_report()).contains("drew no reply"));
}

#[test]
fn a_registry_diff_dashes_the_side_that_has_no_version() {
    assert_data_eq!(
        table(&fx::registry_diff()),
        str![[r#"
   catalog   registry 1.1   agree
✗  sysinfo   served 1.1 · local 1.0
      served declares telemetry disk/{mount}/inodes; local does not
✗  parallax  served 1.3 · local —
      no local slice for this producer

"#]]
    );
}

#[test]
fn a_schema_dump_carries_its_totality_gap_and_its_unserved_case() {
    assert_data_eq!(
        table(&fx::schema_dump()),
        str![[r#"
producer  sysinfo   (app zensight)

1 type(s):

  HealthSnapshot  json-schema  sha256:aaaa

"#]]
    );
    assert!(notes(&fx::schema_dump()).contains("does not cover"));
    // Serving no `describe` renders nothing at all — the sentence is the note,
    // which is what gets it into the machine formats.
    assert_data_eq!(
        table(&fx::schema_dump_unserved()),
        str![[r#"

"#]]
    );
    assert!(notes(&fx::schema_dump_unserved()).contains("not the same as having none"));
    // No registry loaded: `missing` is absent, not `[]`, and the note says
    // totality was not checked — not asked is not answered no
    // (RFC 09 §5.1 O4, #246).
    assert!(notes(&fx::schema_dump_unchecked()).contains("totality not checked"));
    let unchecked: serde_json::Value =
        serde_json::from_str(ndjson(&fx::schema_dump_unchecked()).lines().next().unwrap()).unwrap();
    assert!(unchecked.get("missing").is_none());
    let checked: serde_json::Value =
        serde_json::from_str(ndjson(&fx::schema_dump()).lines().next().unwrap()).unwrap();
    assert_eq!(checked["missing"], serde_json::json!(["TelemetryPoint"]));
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

/// One reply, one error envelope, and the attachment clause `probe` used to
/// drop (#237).
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
/// table exactly as `cutover`'s does — same vocabulary, same exit discipline.
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

/// #198's headline example: `node info --format ndjson` was a pretty
/// multi-line document, so the one command whose answer *is* a list of
/// producers could not be read a producer at a time.
#[test]
fn a_node_info_has_two_row_kinds_and_never_says_zero_for_never_seen() {
    let out = ndjson(&fx::node_info());
    let kinds: Vec<String> = out
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v.get("row").and_then(|r| r.as_str()).map(str::to_string))
        .collect();
    assert_eq!(
        kinds,
        ["producer", "producer", "producer", "freshness", "freshness"]
    );
    assert_data_eq!(
        table(&fx::node_info()),
        str![[r#"
origin    h-3fa9c2d41b7e
  sysinfo   [alive]     app zensight · registry v1.0 · 41 subject(s) · 3 procedure(s) · blob: store · 2 DEPRECATED still served
  parallax  [alive]     (no introspect reply — capabilities unknown, not absent)
  probe     [no token]  app zensight · registry v1.0 · 2 subject(s) · 1 procedure(s)
state freshness (declared ttl_s):
  sysinfo/health  240s old  (ttl 120s)  STALE
  sysinfo/errors  —         (ttl 300s)

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

// ── The zenctl-local families ─────────────────────────────────────────────
//
// Their fixtures are here rather than in `zenkey-report-fixtures`: that crate
// cannot see a `KeyRelation`, and it must not enable the `decode` feature
// `GenReport` lives behind (#204).

#[test]
fn a_key_relation_carries_its_convention_note_once() {
    let no = zenctl::render::KeyRelation {
        op: "includes".into(),
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
        verdict: "valid".into(),
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
        verdict: "invalid".into(),
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
        timeout_s: 5,
        answers: vec![],
    };
    let n = notes(&silent);
    assert!(n.contains("Nobody is registered for it"), "{n}");
    assert!(n.contains("the three are different"), "{n}");
    let doc: serde_json::Value =
        serde_json::from_str(ndjson(&silent).lines().next().unwrap()).unwrap();
    assert_eq!(doc["selector"], "acme/v1/**/state/**");
    assert_eq!(doc["timeout_s"], 5);
}

/// Synthetic traffic is still publishing, so the plan is a dry run made
/// visible before a byte moves — the `replay --dry-run` precedent
/// (RFC 09 §5.3).
#[test]
fn a_gen_plan_states_the_synthetic_marker_before_anything_is_published() {
    let entries: Vec<zenkey_fleet::generate::GenPlanEntry> = vec![];
    let plan = zenctl::render::GenPlan {
        origin: "h-3fa9c2d41b7e",
        duration_s: 5.0,
        entries: &entries,
    };
    let n = notes(&plan);
    assert!(n.contains("synthetic"), "{n}");
    assert!(n.contains("RFC 09 §5.3"), "{n}");

    let report = zenkey_fleet::generate::GenReport {
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
        "admin-graph",
        "admin-routers",
        "base-list",
        "bench",
        "blob-fetch",
        "blob-list",
        "blob-probe",
        "blob-tree",
        "cache",
        "cache-action",
        "call",
        "context",
        "context-action",
        "context-list",
        "cutover",
        "doctor",
        "expect",
        "gen",
        "gen-plan",
        "get",
        "interface-list",
        "interface-show",
        "key-canon",
        "key-relation",
        "node-info",
        "node-list",
        "probe",
        "rate",
        "record",
        "registry-diff",
        "registry-lint",
        "registry-lock",
        "registry-retired",
        "replay",
        "schema-check",
        "schema-dump",
        "scout",
        "service-info",
        "service-list",
        "storage-list",
        "topic-info",
        "topic-list",
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
        context: zenkey_fleet::context_store::StoredContext {
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

/// `registry lint` stays thin on purpose: its stated value is the build's own
/// wording, and a `{dir, passed, message}` struct would be a second shape to
/// keep in step with `zenkey_build`'s over an exit code and a sentence.
#[test]
fn a_registry_lint_is_notes_only_and_says_so_in_json() {
    let pass = zenctl::render::LintReport {
        dir: "registry".into(),
    };
    assert_eq!(table(&pass), "");
    assert!(notes(&pass).contains("registry lints pass (RFC 08 §5)."));
    let doc: serde_json::Value =
        serde_json::from_str(ndjson(&pass).lines().next().unwrap()).unwrap();
    assert_eq!(doc["passed"], true);
    assert_eq!(doc["dir"], "registry");
}

/// A forced break is loud by contract (RFC 08 §3.1): every broken pin is a row
/// *and* a note, so neither a script nor a person can miss one.
#[test]
fn a_forced_lock_break_is_a_row_and_a_note() {
    let forced = zenctl::render::LockReport {
        path: "registry/registry.lock".into(),
        created: false,
        added: 1,
        retired: 0,
        forced: vec!["sysinfo/health: Health -> HealthV2".into()],
    };
    assert!(notes(&forced).contains("1 pinned entry added, 0 released"));
    assert!(notes(&forced).contains("FORCED BREAK"));
    assert!(notes(&forced).contains("RFC 08 §3.1"));
    let lines: Vec<serde_json::Value> = ndjson(&forced)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines[1]["row"], "forced-break");
    assert_eq!(lines[1]["detail"], "sysinfo/health: Health -> HealthV2");
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
