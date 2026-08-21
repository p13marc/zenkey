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
