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
{"notes":[{"text":"2 are open-ended ({var...}): the registry fixes their shape, not their members. Use `zenctl echo` to see what a live fleet actually publishes"}],"report":"topic-list"}
{"class":"telemetry","open_ended":false,"path":"disk/{mount}/used","producer":"sysinfo","registry_version":"1.0","row":"subject","type_name":"TelemetryPoint"}
{"class":"state","open_ended":false,"path":"health","producer":"sysinfo","registry_version":"1.0","row":"subject","type_name":"HealthSnapshot"}
{"class":"telemetry","open_ended":true,"path":"by_unit/{unit}/messages_total","producer":"logs","registry_version":"2.0","row":"subject","type_name":"TelemetryPoint"}
{"class":"telemetry","deprecated":true,"deprecated_since":"2.0","open_ended":true,"path":"ingest/legacy_total","producer":"logs","registry_version":"2.0","replaced_by":"disk/{mount}/bytes_used","row":"subject","since":"1.0","type_name":"TelemetryPoint"}

"#]]
    );
}

/// `--budget` (#221): the declared-vs-observed column says "saw", never
/// "has" — an over-bound row is the finding, a rest-variable row is exempt
/// and says so, a literal row claims nothing — and the coverage note states
/// the window the numbers rest on.
#[test]
fn a_budgeted_topic_list_judges_over_exempts_rest_and_states_its_window() {
    assert_data_eq!(
        table(&fx::topic_list_budget()),
        str![[r#"
sysinfo  (registry 1.0)
  telemetry  disk/{mount}/used              TelemetryPoint                OVER declared 16: saw 40 on h-3fa9c2d41b7e (max of 2 origins)
  state      health                         HealthSnapshot

logs  (registry 2.0)
  telemetry  by_unit/{unit}/messages_total  TelemetryPoint  [open-ended]  exempt: rest-variable (saw 3)
  telemetry  ingest/legacy_total            TelemetryPoint  [open-ended]                                                                 DEPRECATED since 2.0 → disk/{mount}/bytes_used

"#]]
    );
    assert_data_eq!(
        notes(&fx::topic_list_budget()),
        str![[r#"
4 registered subject(s).
2 are open-ended ({var...}): the registry fixes their shape, not their members. Use `zenctl echo` to see what a live fleet actually publishes
budget: observed for 10s over 2 scope(s), 44 distinct key(s) retained; counts are lower bounds on the population, so under the declared cardinality is not a verdict — only over is (RFC 04 §1.2), and {var...} families are exempt: rest-variable (RFC 08 §6.1)

"#]]
    );
}

/// The budgeted ndjson stream: the coverage statement rides the envelope —
/// a fact about the observation, not about any row — and each judged row
/// carries its cell, examples included.
#[test]
fn a_budgeted_topic_list_ndjson_carries_the_window_in_the_envelope() {
    let out = ndjson(&fx::topic_list_budget());
    let envelope: serde_json::Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    // `10.0`, not `10`: every window/timeout field in the report set is
    // `f64` seconds since #218, so serde_json renders an integral one with
    // its point. The number is the same; the type is now one type.
    assert_eq!(envelope["budget"]["window_s"], 10.0);
    assert_eq!(envelope["budget"]["scopes"][0], "v1/*/telemetry/**");
    assert_eq!(envelope["budget"]["evicted"], 0);
    let over: serde_json::Value = serde_json::from_str(out.lines().nth(1).unwrap()).unwrap();
    assert_eq!(over["budget"]["declared"], 16);
    assert_eq!(over["budget"]["worst_observed"], 40);
    assert_eq!(over["budget"]["over"], true);
    assert_eq!(
        over["budget"]["examples"][0],
        "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/m00/used"
    );
    let exempt: serde_json::Value = serde_json::from_str(out.lines().nth(3).unwrap()).unwrap();
    assert_eq!(exempt["budget"]["exempt"], "rest-variable");
    assert_eq!(exempt["budget"]["over"], false);
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
    let envelope_notes = envelope["notes"]
        .as_array()
        .expect("notes ride the envelope");
    assert!(
        envelope_notes
            .iter()
            .any(|n| n["text"].as_str().unwrap().contains("dropped")),
        "the bound reaches a script too: {envelope_notes:?}"
    );
    assert!(
        !envelope.as_object().unwrap().contains_key("findings"),
        "findings are rows, not an envelope field"
    );

    // R1: with no registry the served-vs-declared diff never ran, and the
    // degradation is a note in the report — it used to be a bare eprintln in
    // the command, invisible to every machine format.
    let unchecked = zenkey_fleet::report::DoctorReport {
        synced: zenkey_fleet::report::Asked::NotAsked,
        ..fx::doctor_report()
    };
    let n = notes(&unchecked);
    assert!(n.contains("diff never ran"), "{n}");
    assert!(n.contains("RFC 09 §5.1 O4"), "{n}");
    let envelope: serde_json::Value =
        serde_json::from_str(ndjson(&unchecked).lines().next().unwrap()).unwrap();
    assert!(
        !envelope.as_object().unwrap().contains_key("synced"),
        "diff never ran: the key is absent (O4), never an empty list"
    );
    assert!(
        envelope["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|note| note["text"].as_str().unwrap().contains("diff never ran")),
        "the degradation reaches a script too: {envelope}"
    );
    // …and the fixture's checked run keeps the key.
    let envelope: serde_json::Value =
        serde_json::from_str(ndjson(&fx::doctor_report()).lines().next().unwrap()).unwrap();
    assert!(envelope["synced"].is_array(), "{envelope}");
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
    let renderings = [
        table(&fx::topic_list()),
        table(&fx::node_list()),
        table(&fx::storage_list()),
        table(&fx::topic_info()),
        table(&fx::doctor_report()),
        table(&fx::why_report()),
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
/// finding on the type's own page, naming **which host** serves which
/// identity (#410): the note reads the engine's verdict, never a recompute
/// over the rows, which have no host to name.
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
    let n = notes(&fx::interface_show());
    assert!(
        n.contains("HealthSnapshot is served under 2 identities"),
        "{n}"
    );
    assert!(n.contains("sysinfo@h-aaaaaaaaaaaa (sha256:aaaa)"), "{n}");
    assert!(n.contains("gnmi@h-bbbbbbbbbbbb (sha256:bbbb)"), "{n}");
    assert!(n.contains("RFC 08 §7"), "{n}");
    // …and the verdict reaches a script as its own row kind, per origin.
    let drift_row = ndjson(&fx::interface_show())
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .find(|v| v["row"] == "drift")
        .expect("a drift row");
    assert_eq!(drift_row["verdict"], "disagree");
    assert_eq!(drift_row["servers"][1]["origin"], "h-bbbbbbbbbbbb");

    // R4: without --schema the bus was never asked, and the document must not
    // carry the old unconditional `"schemas": 0` — an unasked bus is not one
    // serving nothing (O4).
    let unasked = fx::interface_show_unasked();
    let envelope: serde_json::Value =
        serde_json::from_str(ndjson(&unasked).lines().next().unwrap()).unwrap();
    assert!(
        !envelope.as_object().unwrap().contains_key("schemas"),
        "not asked: the count is absent, never 0 — {envelope}"
    );
    assert!(
        !envelope.as_object().unwrap().contains_key("drift"),
        "not asked: no drift count either, for the same reason — {envelope}"
    );
    let n = notes(&unasked);
    assert!(n.contains("not asked"), "{n}");
    assert!(n.contains("RFC 09 §5.1 O4"), "{n}");
    // …and asked-with-silence is the third state, distinct from both.
    let silent = zenkey_fleet::report::InterfaceShow {
        schemas: zenkey_fleet::report::Asked::Asked(vec![]),
        ..fx::interface_show_unasked()
    };
    let envelope: serde_json::Value =
        serde_json::from_str(ndjson(&silent).lines().next().unwrap()).unwrap();
    assert_eq!(envelope["schemas"], 0, "asked, none served: a real zero");
    assert_eq!(
        envelope["drift"], 0,
        "asked, nothing to compare: a real zero"
    );
    assert!(notes(&silent).contains("no carrier served one"));
}

/// One producer on two hosts, one of which served no identity: the page says
/// agreement **cannot be established** and cites O4, rather than reading the
/// one row it has as agreement — the recompute over rows this replaced could
/// only ever see one hash here, and called it clean (#410, #370).
#[test]
fn an_interface_show_says_when_agreement_cannot_be_judged() {
    let show = fx::interface_show_unjudgeable();
    let n = notes(&show);
    assert!(
        n.contains("agreement on HealthSnapshot cannot be established"),
        "{n}"
    );
    assert!(
        n.contains("sysinfo@h-bbbbbbbbbbbb served no identity"),
        "{n}"
    );
    assert!(n.contains("RFC 09 §5.1 O4"), "{n}");
    assert!(
        !n.contains("identities"),
        "unjudgeable is not a disagreement: {n}"
    );
    let drift_row = ndjson(&show)
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .find(|v| v["row"] == "drift")
        .expect("a drift row");
    assert_eq!(drift_row["verdict"], "unjudgeable");
    assert!(
        drift_row["servers"][1].get("hash").is_none(),
        "no identity is absent on the wire, never \"\" — {drift_row}"
    );
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

#[test]
fn a_registry_diff_dashes_the_side_that_has_no_version() {
    assert_data_eq!(
        table(&fx::registry_diff()),
        str![[r#"
✗  catalog   registry 1.1
      the fleet does not agree with itself: h-3fa9c2d41b7e serves 1.1, h-8b1e07af22c9 serves 1.0
✗  sysinfo   served 1.1 · local 1.0
      served declares telemetry disk/{mount}/inodes; local does not
✗  parallax  served 1.3 · local —
      no local slice for this producer

"#]]
    );
}

/// #399: `catalog` matches the checkout, so it used to read `agree` — while
/// two hosts served it at different versions and the row was computed from
/// one of them. The word is the lie, and it is gone where it would be one.
///
/// The pair below is the O4 split: a served side that never came off the bus
/// cannot say whether the fleet agrees with itself, and must not print the
/// silence as agreement.
#[test]
fn a_registry_diff_says_when_the_fleet_disagrees_with_itself() {
    let asked = notes(&fx::registry_diff());
    assert!(
        asked.contains("does not agree with itself"),
        "the fleet-vs-itself note must name the count: {asked}"
    );
    assert!(
        asked.contains("arrival order"),
        "and say which answer the diff used: {asked}"
    );

    let not_asked = notes(&fx::registry_diff_not_asked());
    assert!(
        not_asked.contains("never asked"),
        "not asked is not \"the fleet agrees\": {not_asked}"
    );
    assert!(
        !not_asked.contains("does not agree with itself"),
        "and it is not a finding either: {not_asked}"
    );
    // The unasked side still renders the diff itself, and `catalog` is
    // `agree` there — against the checkout, which is the only comparison
    // that side actually made.
    assert_data_eq!(
        table(&fx::registry_diff_not_asked()),
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

/// `service call --trace` (#215): the reply block first, exactly as `service
/// call` draws it, then the three lanes — the declared chain with both
/// clocks and the stamper, the same-origin lane, and the concurrent count
/// as one line — with a drop rendered as a break before the row it
/// preceded. The ndjson stream is the call's answers, then every effect
/// tagged with its lane, then the concurrent lane as one row.
#[test]
fn a_traced_call_draws_the_reply_then_three_lanes_and_never_an_edge() {
    assert_data_eq!(
        table(&fx::trace_report()),
        str![[r#"
h-3fa9c2d41b7e:
{
  "id": "01HZY"
}
observed after the call · declared chain (long-running) · Δarrival · ΔHLC (stamper) · key
 +12.500ms    +8ms (foreign:33)  acme/v1/h-3fa9c2d41b7e/state/demo/artifact/pcap    declared-chain
                                 ⋯ 3 sample(s) dropped while behind
+250.000ms  +246ms (foreign:33)  acme/v1/h-3fa9c2d41b7e/events/demo/artifact/01HZY  declared-chain

observed after the call · same origin, not declared · Δarrival · ΔHLC (stamper) · key
  +1.000ms                       acme/v1/h-3fa9c2d41b7e/telemetry/other/noise       same-origin, not declared

concurrent, not attributed
40 sample(s) on 2 key(s) from other origins during the window — e.g. acme/v1/h-bbbbbbbbbbbb/telemetry/sysinfo/cpu, acme/v1/h-bbbbbbbbbbbb/telemetry/sysinfo/mem

"#]]
    );
    assert_data_eq!(
        ndjson(&fx::trace_report()),
        str![[r#"
{"answers":1,"attributed":2,"call_returned_ms":4.2,"chain_rule":"first-chunk naming heuristic (RFC 05 §3 idiom); a naming coincidence is tagged the same way","dropped":3,"excluded":"the verbatim planes (`@rpc`, `@blob`, `@media`, `@adv`, `@catalog`): `**` never crosses an `@`-chunk (RFC 03 §4 D2), so the artifact bytes on `@blob` are outside this window — excluded, not empty","hlc_reference":"reply","idiom":"long-running","key":"acme/v1/h-3fa9c2d41b7e/@rpc/demo/artifact/request","keys_evicted":0,"notes":[{"cite":"RFC 05 §3","text":"chain rule: first-chunk naming heuristic (RFC 05 §3 idiom); a naming coincidence is tagged the same way"},{"cite":"RFC 09 §5.1 O7","text":"ΔHLC is measured against the reply's HLC (7680000000000000000/33) — the stamping node's clock, which is not necessarily the responder's and is never this caller's (it mints none); a different stamper on a row is a different clock"},{"cite":"RFC 03 §4 D2","text":"the verbatim planes (`@rpc`, `@blob`, `@media`, `@adv`, `@catalog`): `**` never crosses an `@`-chunk (RFC 03 §4 D2), so the artifact bytes on `@blob` are outside this window — excluded, not empty"},{"cite":"RFC 09 §5.1 O6","text":"3 sample(s) dropped while behind on the origin's window — each is a break before the next row of every lane"}],"registry_loaded":true,"reply_hlc":"7680000000000000000/33","report":"trace","same_origin":1,"scopes":["acme/v1/h-3fa9c2d41b7e/**","acme/v1/*/**"],"subscribed_before_call":true,"t0_unix_s":1788000000.5,"timeout_s":5.0,"window_s":10.0}
{"ok":true,"origin":"h-3fa9c2d41b7e","row":"answer","value":{"id":"01HZY"}}
{"arrival_delta_ms":12.5,"hlc":"7680000000034359738/33","hlc_delta_ms":8,"key":"acme/v1/h-3fa9c2d41b7e/state/demo/artifact/pcap","kind":"put","lane":"attributed","payload_bytes":40,"relation":"declared_chain","row":"effect","stamped_by":"foreign:33"}
{"arrival_delta_ms":250.0,"break_before":3,"hlc":"7680000001056964608/33","hlc_delta_ms":246,"key":"acme/v1/h-3fa9c2d41b7e/events/demo/artifact/01HZY","kind":"put","lane":"attributed","payload_bytes":40,"relation":"declared_chain","row":"effect","stamped_by":"foreign:33"}
{"arrival_delta_ms":1.0,"key":"acme/v1/h-3fa9c2d41b7e/telemetry/other/noise","kind":"put","lane":"same_origin","payload_bytes":40,"relation":"same_origin_undeclared","row":"effect"}
{"dropped":0,"examples":["acme/v1/h-bbbbbbbbbbbb/telemetry/sysinfo/cpu","acme/v1/h-bbbbbbbbbbbb/telemetry/sysinfo/mem"],"keys":2,"row":"concurrent","samples":40}

"#]]
    );
    let n = notes(&fx::trace_report());
    assert!(
        n.contains("chain rule: first-chunk naming heuristic"),
        "{n}"
    );
    assert!(n.contains("never crosses an `@`-chunk"), "{n}");
    assert!(n.contains("never caused"), "{n}");
    assert!(n.contains("3 sample(s) dropped while behind"), "{n}");
    assert!(!n.contains("registry not loaded"), "{n}");
    assert!(
        !n.contains("→") && !n.contains("caused by"),
        "no edge, no arrow: {n}"
    );

    // No registry, no reply HLC: an empty attributed lane is a *coverage*
    // sentence, never a silent header; the chain is unjudgeable and says so;
    // ΔHLC is absent, so the stamped rows show `—` (not asked) rather than
    // an empty cell that would read as unstamped.
    let report = fx::trace_report_no_registry();
    assert_data_eq!(
        table(&report),
        str![[r#"
h-3fa9c2d41b7e:
{
  "id": "01HZY"
}
observed after the call · same origin — registry not loaded, chain unjudgeable · Δarrival · ΔHLC (stamper) · key
 +12.500ms  —  acme/v1/h-3fa9c2d41b7e/state/demo/artifact/pcap    same-origin, registry not loaded
+250.000ms  —  acme/v1/h-3fa9c2d41b7e/events/demo/artifact/01HZY  same-origin, registry not loaded
  +1.000ms     acme/v1/h-3fa9c2d41b7e/telemetry/other/noise       same-origin, registry not loaded

concurrent, not attributed
40 sample(s) on 2 key(s) from other origins during the window — e.g. acme/v1/h-bbbbbbbbbbbb/telemetry/sysinfo/cpu, acme/v1/h-bbbbbbbbbbbb/telemetry/sysinfo/mem

"#]]
    );
    let n = notes(&report);
    assert!(n.contains("registry not loaded — chain unjudgeable"), "{n}");
    assert!(
        n.contains("nothing in the declared chain was observed"),
        "{n}"
    );
    assert!(n.contains("the reply carried no HLC"), "{n}");
    assert!(
        !n.contains("dropped while behind"),
        "zero costs emit nothing: {n}"
    );
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

/// The ACL plan draws its three lists as two tables — principals with their
/// rules, rules with their key expressions — and tags five row kinds on the
/// stream. The one warning here is the not-asked registry, which is a
/// coverage note and therefore rides the machine formats too.
#[test]
fn an_acl_plan_draws_principals_then_rules_and_tags_five_row_kinds() {
    let plan = fx::acl_plan();
    let out = ndjson(&plan);
    let mut kinds: Vec<String> = out
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v.get("row").and_then(|r| r.as_str()).map(str::to_string))
        .collect();
    kinds.dedup();
    assert_eq!(kinds, ["rule", "subject", "policy", "warning"]);
    let envelope: serde_json::Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    assert_eq!(envelope["report"], "acl-plan");
    assert_eq!(envelope["default_permission"], "deny");
    assert!(envelope.get("registry").is_none(), "not asked is absent");
    assert!(envelope.get("rules").is_none(), "the lists are rows");
    assert_data_eq!(
        table(&plan),
        str![[r#"
principals under base "zensight" (default deny):

  subject           role     bound by             rules
  h-3fa9c2d41b7e    host     cn h-3fa9c2d41b7e    host-data-h-3fa9c2d41b7e, host-serve-h-3fa9c2d41b7e, host-media-h-3fa9c2d41b7e, host-blob-seed-h-3fa9c2d41b7e, host-adv-h-3fa9c2d41b7e, interest-prop
  zensight-catalog  catalog  cn zensight-catalog  catalog-own-catalog, catalog-intake-declare-catalog, catalog-intake-recv-catalog, interest-prop
  zensight-console  console  cn zensight-console  ops-sub, ops-recv, ops-own-token, no-remote-actions
  zensight-watch    watch    cn zensight-watch    watch-sub, watch-recv, no-remote-actions

rules:

  rule                            permission  flows    messages
  host-data-h-3fa9c2d41b7e        allow       ingress  put, delete, liveliness_token
      zensight/v1/h-3fa9c2d41b7e/**
  host-serve-h-3fa9c2d41b7e       allow       both     declare_queryable, reply, query
      zensight/v1/h-3fa9c2d41b7e/@rpc/**
      zensight/v1/h-3fa9c2d41b7e/@blob/**
  host-media-h-3fa9c2d41b7e       allow       ingress  put
      zensight/v1/h-3fa9c2d41b7e/@media/**
  host-blob-seed-h-3fa9c2d41b7e   allow       ingress  put
      zensight/v1/h-3fa9c2d41b7e/@blob/store/**
      zensight/v1/h-3fa9c2d41b7e/@blob/tree/**
  host-adv-h-3fa9c2d41b7e         allow       both     put, liveliness_token, declare_queryable, reply, query
      zensight/v1/h-3fa9c2d41b7e/**/@adv/**
  interest-prop                   allow       egress   declare_subscriber, declare_liveliness_subscriber, liveliness_query, query
      zensight/v1/**
      zensight/v1/@catalog/**
      zensight/v1/*/@rpc/**
      zensight/v1/@catalog/@rpc/**
      zensight/v1/*/@blob/**
      zensight/v1/*/@media/**
      zensight/v1/**/@adv/**
      zensight/v1/@catalog/**/@adv/**
  catalog-own-catalog             allow       both     put, delete, liveliness_token, declare_queryable, reply, query
      zensight/v1/@catalog/**
      zensight/v1/@catalog/@rpc/**
      zensight/v1/@catalog/**/@adv/**
  catalog-intake-declare-catalog  allow       ingress  declare_subscriber, declare_liveliness_subscriber, liveliness_query, query
      zensight/v1/**
      zensight/v1/**/@adv/**
  catalog-intake-recv-catalog     allow       egress   put, delete, reply, liveliness_token
      zensight/v1/**
      zensight/v1/**/@adv/**
  ops-sub                         allow       ingress  declare_subscriber, declare_liveliness_subscriber, liveliness_query, query
      zensight/v1/**
      zensight/v1/@catalog/**
      zensight/v1/*/@rpc/**
      zensight/v1/@catalog/@rpc/**
      zensight/v1/*/@blob/**
      zensight/v1/*/@media/**
      zensight/v1/**/@adv/**
      zensight/v1/@catalog/**/@adv/**
  ops-recv                        allow       egress   put, delete, reply, liveliness_token
      zensight/v1/**
      zensight/v1/@catalog/**
      zensight/v1/*/@rpc/**
      zensight/v1/@catalog/@rpc/**
      zensight/v1/*/@blob/**
      zensight/v1/*/@media/**
      zensight/v1/**/@adv/**
      zensight/v1/@catalog/**/@adv/**
  ops-own-token                   allow       ingress  liveliness_token
      zensight/v1/**/@adv/**
      zensight/v1/@catalog/**/@adv/**
✗ no-remote-actions               deny        both     query
      zensight/v1/*/@rpc/*/**/set
  watch-sub                       allow       ingress  declare_subscriber, declare_liveliness_subscriber, liveliness_query, query
      zensight/v1/**
      zensight/v1/@catalog/**
      zensight/v1/*/@rpc/**
      zensight/v1/@catalog/@rpc/**
      zensight/v1/**/@adv/**
      zensight/v1/@catalog/**/@adv/**
  watch-recv                      allow       egress   put, delete, reply, liveliness_token
      zensight/v1/**
      zensight/v1/@catalog/**
      zensight/v1/*/@rpc/**
      zensight/v1/@catalog/@rpc/**
      zensight/v1/**/@adv/**
      zensight/v1/@catalog/**/@adv/**

"#]]
    );
    assert_data_eq!(
        notes(&plan),
        str![[r#"
no registry asked: the planes are as the enrollment claims, and no-remote-actions denies the convention's write leaf zensight/v1/*/@rpc/*/**/set rather than the declared write procedures — pass --registry <dir> to narrow both to what the fleet actually declares (RFC 13 §3 O4)
15 rule(s), 4 subject(s), 4 polic(y/ies)
write the block: `zenctl acl gen --enrollment … --json5 > router-acl.json5`, merge it at the router config's top level, restart the router — ACL config is not runtime-reloadable (RFC 03 §4 D6)

"#]]
    );
    // A refusal is a note in every format, and a row in the machine ones.
    let refused = fx::acl_plan_refused();
    assert!(notes(&refused).contains("REFUSED bare-host"));
    assert!(ndjson(&refused).contains(r#""row":"refusal""#));
}

/// The check puts the verdict word beside its findings, and states twice
/// what it could not see: the running block, and interest propagation.
#[test]
fn an_acl_check_names_its_findings_and_what_it_could_not_observe() {
    assert_data_eq!(
        table(&fx::acl_check()),
        str![[r#"
router.json5: 10 rule(s) and 5 subject(s) configured; 15 and 4 planned
  ✗ rule_missing  interest-prop     planned allow egress declare_liveliness_subscriber,declare_subscriber,liveliness_query,query zensight/v1/**
  ✗ unknown_cn    stranger.example  configured bound by subject "stranger"
FAIL

"#]]
    );
    assert_data_eq!(
        table(&fx::acl_check_clean()),
        str![[r#"
router.json5: 15 rule(s) and 4 subject(s) configured; 15 and 4 planned
PASS

"#]]
    );
    let n = notes(&fx::acl_check());
    assert!(n.contains("serves no GET"));
    assert!(n.contains("interest propagation not asked"));
    let out = ndjson(&fx::acl_check());
    let envelope: serde_json::Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    assert_eq!(envelope["interest_probe"]["answer"], "not_asked");
    assert_eq!(envelope["judgement"]["answer"], "established");
    assert_eq!(out.lines().count(), 3, "envelope + two findings");
}

/// The explanation answers per direction with the rules that decided, deny
/// first — and its ndjson is one row per direction, tagged with the flow.
#[test]
fn an_acl_explain_answers_per_direction_deny_first() {
    assert_data_eq!(
        table(&fx::acl_explain()),
        str![[r#"
zensight-console  query  zensight/v1/h-3fa9c2d41b7e/@rpc/systemd/action/set
  ingress  DENIED  no-remote-actions denies query on ingress — deny wins over ops-sub
      ✗ no-remote-actions  deny  zensight/v1/*/@rpc/*/**/set
      ✓ ops-sub  allow  zensight/v1/*/@rpc/**
  egress   DENIED  no-remote-actions denies query on egress — deny wins
      ✗ no-remote-actions  deny  zensight/v1/*/@rpc/*/**/set

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
    assert_eq!(rows[0]["decision"], "denied");
    assert_eq!(rows[0]["via"][0]["permission"], "deny");
    assert_eq!(rows[1]["flow"], "egress");
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
        "export",
        "field",
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
        "registry-consumers",
        "registry-diff",
        "registry-impact",
        "registry-lint",
        "registry-lock",
        "registry-retired",
        "replay",
        "schema-check",
        "schema-dump",
        "scout",
        "service-info",
        "service-list",
        "snapshot",
        "snapshot-diff",
        "storage-check",
        "storage-explain",
        "storage-list",
        "storage-plan",
        "timeline",
        "topic-info",
        "topic-list",
        "trace",
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
    // The doctor's scope is its listen phase; the fixture ran one.
    let s = scoped(&fx::doctor_report());
    assert_eq!(s.asked, ["v1/**"]);
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
    // The trace's scope is both watches over the window — the origin's
    // subtree and the fleet's; the GET's own ask is inside `call`.
    let s = scoped(&fx::trace_report());
    assert_eq!(s.asked, ["acme/v1/h-3fa9c2d41b7e/**", "acme/v1/*/**"]);
    assert_eq!(s.window_s, Some(10.0));
    let s = scoped(&zenctl::render::GetReport {
        selector: "acme/v1/**/state/**".into(),
        timeout_s: 5.0,
        elided: 0,
        answers: vec![],
    });
    assert_eq!(s.asked, ["acme/v1/**/state/**"]);
    scoped(&fx::bench_report());
    scoped(&fx::why_report());

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
    // The consumers join: the six admin selectors, no window; the impact
    // adds the storage sweep when it was made.
    let s = scoped(&fx::consumers_report());
    assert_eq!(s.asked.len(), 6, "{:?}", s.asked);
    assert_eq!(s.window_s, None);
    let s = scoped(&fx::subject_impact());
    assert_eq!(s.asked.len(), 7, "{:?}", s.asked);
    let s = scoped(&zenkey_fleet::report::SubjectImpact {
        consumers: fx::consumers_not_available(),
        coverage: None,
        ..fx::subject_impact()
    });
    assert_eq!(s.asked.len(), 6, "an unmade storage sweep is not claimed");
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

/// `registry lint` stays thin on purpose: its stated value is the build's own
/// wording, and a `{dir, passed, message}` struct would be a second shape to
/// keep in step with `zenkey_build`'s over an exit code and a sentence.
#[test]
fn a_registry_lint_is_notes_only_and_says_so_in_json() {
    let pass = zenctl::render::LintReport {
        dir: "registry".into(),
        warnings: Vec::new(),
    };
    assert_eq!(table(&pass), "");
    assert!(notes(&pass).contains("registry lints pass (RFC 08 §5)."));
    let doc: serde_json::Value =
        serde_json::from_str(ndjson(&pass).lines().next().unwrap()).unwrap();
    assert_eq!(doc["passed"], true);
    assert_eq!(doc["dir"], "registry");
    assert_eq!(doc["warnings"], serde_json::json!([]));
}

/// A registry that opted out of RFC 08 §3.1 checking is *reported*, in both
/// renderings, and still passes — it is a warning, not a finding (#319).
///
/// It reported nothing at all before: the warning sat behind
/// `emit_rerun_if_changed`, and this command turns that off to keep cargo
/// directives out of its stdout. The flag governs directives only now.
#[test]
fn a_registry_lint_reports_the_build_s_warnings_and_still_passes() {
    let warned = zenctl::render::LintReport {
        dir: "registry".into(),
        warnings: vec![
            "legacy declares compat = \"none\" — its entries are unpinned and \
             incompatible edits pass unchecked (RFC 08 §3.1)"
                .to_string(),
        ],
    };
    // The person sees it…
    let text = notes(&warned);
    assert!(text.contains("with warnings:"), "{text}");
    assert!(text.contains("compat = \"none\""), "{text}");
    // …and so does the script, without it becoming a failure.
    let doc: serde_json::Value =
        serde_json::from_str(ndjson(&warned).lines().next().unwrap()).unwrap();
    assert_eq!(doc["passed"], true);
    assert_eq!(doc["warnings"].as_array().unwrap().len(), 1);
    assert!(
        doc["warnings"][0]
            .as_str()
            .unwrap()
            .contains("compat = \"none\"")
    );
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

#[test]
fn consumers_rank_by_relation_and_name_the_tool_itself() {
    assert_data_eq!(
        table(&fx::consumers_report()),
        str![[r#"
declared readers of acme/v1/*/state/sysinfo/health

zid                              whatami  origin                            declared                                                relation
eeff0011                         peer     h-3fa9c2d41b7e                    subscriber acme/v1/h-3fa9c2d41b7e/state/sysinfo/health  narrower — declared on a subset of the target
ffffffff  (this zenctl session)  peer     session only, unattributed        querier acme/v1/*/state/sysinfo/health                  exact — declared on the target itself
aabbccdd                         router   reported only — no session named  subscriber **                                           total — a whole-base declaration, intersects everything

"#]]
    );
    assert_data_eq!(
        notes(&fx::consumers_report()),
        str![[r#"
1 admin space(s) answered (2 node(s) heard of): a declared subscriber or querier is a declaration, not proof of use, and sessions behind an admin space that did not answer are not shown (RFC 13 §3 O5)
a whole-base declaration (`**`) intersects every key under the base and says nothing about this subject in particular; it still never crosses an `@`-chunk, so `@rpc`/`@media`/`@blob` sidecars and service origins are outside it (RFC 03 §4 D2)

"#]]
    );
}

#[test]
fn consumers_ndjson_flattens_the_admin_answer_and_tags_every_row() {
    assert_data_eq!(
        ndjson(&fx::consumers_report()),
        str![[r#"
{"admin":"answered","answered":1,"asked":["@/*/*","@/*/*/subscriber/**","@/*/*/publisher/**","@/*/*/queryable/**","@/*/*/querier/**","@/*/*/token/**"],"nodes":2,"notes":[{"cite":"RFC 13 §3 O5","text":"1 admin space(s) answered (2 node(s) heard of): a declared subscriber or querier is a declaration, not proof of use, and sessions behind an admin space that did not answer are not shown"},{"cite":"RFC 03 §4 D2","text":"a whole-base declaration (`**`) intersects every key under the base and says nothing about this subject in particular; it still never crosses an `@`-chunk, so `@rpc`/`@media`/`@blob` sidecars and service origins are outside it"}],"reply_elided":0,"report":"registry-consumers","self_zid":"ffffffff","target":"acme/v1/*/state/sysinfo/health"}
{"attribution":"session","keyexpr":"acme/v1/h-3fa9c2d41b7e/state/sysinfo/health","kind":"subscriber","origins":["h-3fa9c2d41b7e"],"relation":"narrower","row":"consumer","whatami":"peer","zid":"eeff0011"}
{"attribution":"session","is_self":true,"keyexpr":"acme/v1/*/state/sysinfo/health","kind":"querier","relation":"exact","row":"consumer","whatami":"peer","zid":"ffffffff"}
{"attribution":"reported_only","keyexpr":"**","kind":"subscriber","relation":"total","row":"consumer","total_wildcard":true,"whatami":"router","zid":"aabbccdd"}

"#]]
    );
}

/// No admin space answering draws no rows and says *not asked* — in every
/// format, since the sentence is a note.
#[test]
fn consumers_without_an_admin_space_are_not_asked() {
    assert_data_eq!(
        table(&fx::consumers_not_available()),
        str![[r#"
declared readers of acme/v1/*/state/sysinfo/health

"#]]
    );
    assert_data_eq!(
        notes(&fx::consumers_not_available()),
        str![[r#"
no admin space answered @/*/*, @/*/*/subscriber/**, @/*/*/publisher/**, @/*/*/queryable/**, @/*/*/querier/**, @/*/*/token/** — zenoh's `adminspace.enabled` is off by default (routers ship with it on); the declared readers of acme/v1/*/state/sysinfo/health are *not asked*, never none (RFC 13 §3 O4)

"#]]
    );
    assert_data_eq!(
        ndjson(&fx::consumers_not_available()),
        str![[r#"
{"admin":"not_available","asked":["@/*/*","@/*/*/subscriber/**","@/*/*/publisher/**","@/*/*/queryable/**","@/*/*/querier/**","@/*/*/token/**"],"notes":[{"cite":"RFC 13 §3 O4","text":"no admin space answered @/*/*, @/*/*/subscriber/**, @/*/*/publisher/**, @/*/*/queryable/**, @/*/*/querier/**, @/*/*/token/** — zenoh's `adminspace.enabled` is off by default (routers ship with it on); the declared readers of acme/v1/*/state/sysinfo/health are *not asked*, never none"}],"reply_elided":0,"report":"registry-consumers","self_zid":"ffffffff","target":"acme/v1/*/state/sysinfo/health"}

"#]]
    );
}

#[test]
fn an_impact_nests_the_readers_the_coverage_and_the_ledger() {
    assert_data_eq!(
        table(&fx::subject_impact()),
        str![[r#"
impact of sysinfo state health  (selector acme/v1/*/state/sysinfo/health)
  DEPRECATED since 2.0 → replaced by status

declared readers:

zid                              whatami  origin                            declared                                                relation
eeff0011                         peer     h-3fa9c2d41b7e                    subscriber acme/v1/h-3fa9c2d41b7e/state/sysinfo/health  narrower — declared on a subset of the target
ffffffff  (this zenctl session)  peer     session only, unattributed        querier acme/v1/*/state/sysinfo/health                  exact — declared on the target itself
aabbccdd                         router   reported only — no session named  subscriber **                                           total — a whole-base declaration, intersects everything

also declared on the family:
  publishers  2 session(s)
  queryables  0 session(s)

storage coverage:
  ✓ health  covered by main@aabbccdd  (ttl_s 120)

"#]]
    );
    assert_data_eq!(
        notes(&fx::subject_impact()),
        str![[r#"
1 admin space(s) answered (2 node(s) heard of): a declared subscriber or querier is a declaration, not proof of use, and sessions behind an admin space that did not answer are not shown (RFC 13 §3 O5)
a whole-base declaration (`**`) intersects every key under the base and says nothing about this subject in particular; it still never crosses an `@`-chunk, so `@rpc`/`@media`/`@blob` sidecars and service origins are outside it (RFC 03 §4 D2)
the subject is retired in the registry ledger; a declared reader of it is the burn-down `zenctl check retired` counts (RFC 08 §3)

"#]]
    );
    assert_data_eq!(
        ndjson(&fx::subject_impact()),
        str![[r#"
{"admin":"answered","answered":1,"asked":["@/*/*","@/*/*/subscriber/**","@/*/*/publisher/**","@/*/*/queryable/**","@/*/*/querier/**","@/*/*/token/**"],"class":"state","declared_publishers":2,"declared_queryables":0,"deprecated":{"replaced_by":"status","since":"2.0"},"nodes":2,"notes":[{"cite":"RFC 13 §3 O5","text":"1 admin space(s) answered (2 node(s) heard of): a declared subscriber or querier is a declaration, not proof of use, and sessions behind an admin space that did not answer are not shown"},{"cite":"RFC 03 §4 D2","text":"a whole-base declaration (`**`) intersects every key under the base and says nothing about this subject in particular; it still never crosses an `@`-chunk, so `@rpc`/`@media`/`@blob` sidecars and service origins are outside it"},{"cite":"RFC 08 §3","text":"the subject is retired in the registry ledger; a declared reader of it is the burn-down `zenctl check retired` counts"}],"path":"health","producer":"sysinfo","reply_elided":0,"report":"registry-impact","selector":"acme/v1/*/state/sysinfo/health","self_zid":"ffffffff"}
{"attribution":"session","keyexpr":"acme/v1/h-3fa9c2d41b7e/state/sysinfo/health","kind":"subscriber","origins":["h-3fa9c2d41b7e"],"relation":"narrower","row":"consumer","whatami":"peer","zid":"eeff0011"}
{"attribution":"session","is_self":true,"keyexpr":"acme/v1/*/state/sysinfo/health","kind":"querier","relation":"exact","row":"consumer","whatami":"peer","zid":"ffffffff"}
{"attribution":"reported_only","keyexpr":"**","kind":"subscriber","relation":"total","row":"consumer","total_wildcard":true,"whatami":"router","zid":"aabbccdd"}
{"coverage":"covered","path":"health","producer":"sysinfo","row":"coverage","storage":"main@aabbccdd","ttl_s":120}

"#]]
    );
}

/// An impact whose admin space did not answer draws `—` for every admin
/// fact and states the unmade storage sweep.
#[test]
fn an_impact_without_an_admin_space_draws_not_asked_everywhere() {
    let unasked = zenkey_fleet::report::SubjectImpact {
        consumers: fx::consumers_not_available(),
        coverage: None,
        declared_publishers: None,
        declared_queryables: None,
        deprecated: None,
        ..fx::subject_impact()
    };
    assert_data_eq!(
        table(&unasked),
        str![[r#"
impact of sysinfo state health  (selector acme/v1/*/state/sysinfo/health)

declared readers:

also declared on the family:
  publishers  —
  queryables  —

storage coverage:
  —  (not asked: no admin space answered)

"#]]
    );
    assert_data_eq!(
        notes(&unasked),
        str![[r#"
no admin space answered @/*/*, @/*/*/subscriber/**, @/*/*/publisher/**, @/*/*/queryable/**, @/*/*/querier/**, @/*/*/token/** — zenoh's `adminspace.enabled` is off by default (routers ship with it on); the declared readers of acme/v1/*/state/sysinfo/health are *not asked*, never none (RFC 13 §3 O4)
the storage sweep was not made because no admin space answered: an empty storage list would read as "uncovered", which nobody established (RFC 13 §3 O4)

"#]]
    );
}

/// The wording rule (RFC 12 §9): foreign matching status is deferred
/// permanently, and "nobody is listening" is the standing false verdict.
/// Nothing either family draws — table, notes or ndjson, answered or not —
/// may say "matching", "listening", "unmatched" or "no consumers".
#[test]
fn the_consumer_families_never_speak_of_matching_or_listening() {
    const FORBIDDEN: &[&str] = &["matching", "listening", "unmatched", "no consumers"];
    let unasked = zenkey_fleet::report::SubjectImpact {
        consumers: fx::consumers_not_available(),
        coverage: None,
        declared_publishers: None,
        declared_queryables: None,
        ..fx::subject_impact()
    };
    let drawings = [
        table(&fx::consumers_report()),
        notes(&fx::consumers_report()),
        ndjson(&fx::consumers_report()),
        table(&fx::consumers_not_available()),
        notes(&fx::consumers_not_available()),
        ndjson(&fx::consumers_not_available()),
        table(&fx::subject_impact()),
        notes(&fx::subject_impact()),
        ndjson(&fx::subject_impact()),
        table(&unasked),
        notes(&unasked),
        ndjson(&unasked),
    ];
    for drawing in &drawings {
        let lower = drawing.to_lowercase();
        for word in FORBIDDEN {
            assert!(
                !lower.contains(word),
                "{word:?} is matching-status vocabulary (RFC 12 §9) in:\n{drawing}"
            );
        }
    }
}

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
{"contract":{"payload_invalid":1,"payload_not_validated":128,"payload_valid":200,"qos_judged":320,"qos_mismatch":2,"qos_mismatch_by_subject":[{"n":2,"producer":"sysinfo","subject":"cpu/usage"}]},"doctor":{"findings":[{"check":"stale-state","severity":"warning","subject":"h-3fa9c2d41b7e/sysinfo"}],"ran_at_unix_s":1700000090},"excluded":["@rpc","@media","@blob","@adv","service origins"],"max_series":10000,"notes":[{"cite":"RFC 03 §4 D2","text":"a wildcard selector never crosses an `@`-chunk: @rpc, @media, @blob, @adv, service origins are excluded from this surface, not empty"},{"cite":"RFC 09 §5.1 O4","text":"3 distinct key(s) the registry does not declare are counted, never exported — the contract is the registry"},{"text":"128 sample(s) not validated (past the decode budget, or no schema) — a third population beside 200 valid and 1 invalid, never folded into a ratio"},{"text":"2 series stopped (evicted, origin_down or retired): each keeps its labels and state and exposes no value, so a scraper sees a named absence rather than a flat line"},{"text":"quiet is judged only for `state` subjects against their declared ttl_s; telemetry declares no period and is never called quiet"},{"cite":"RFC 09 §5.1 O7","text":"last seen is this observer's wall clock at arrival, never the producer's"},{"cite":"RFC 09 §5.1 O6","text":"3 sample(s) dropped while behind — every value is a lower bound while this moves"},{"cite":"RFC 09 §5.1 O6","text":"5 key(s) retired at the stats-table bound; their series read `evicted`"},{"cite":"RFC 09 §5.1 O6","text":"7 retained sample(s) dropped at the byte budget"},{"cite":"RFC 09 §5.1 O6","text":"11 retained sample(s) aged out of the retention window"},{"cite":"RFC 09 §5.1 O6","text":"13 key(s) retired because their watch was released"},{"cite":"RFC 09 §5.1 O6","text":"17 sample(s) coalesced between scrapes — only the newest value per series is exposed"},{"cite":"RFC 09 §5.1 O6","text":"4 sample(s) refused a series past the declared `cardinality` budget"},{"cite":"RFC 09 §5.1 O6","text":"1 field(s) refused a series past the per-subject field cap"}],"observer":{"coalesced":17,"dropped":3,"evicted_bytes":7,"evicted_keys":5,"expired":11,"unstamped":19,"unwatched":13},"registry":{"producers":2},"report":"export","scopes":["acme/v1/*/**"],"started_at_unix_s":1700000000,"suppressed":{"cardinality":4,"fields":1},"taken_at_unix_s":1700000120,"unregistered_keys":3}
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
