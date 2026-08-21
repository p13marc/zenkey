//! The `emit` contract (#198): one format switch, and what each format owes.
//!
//! `Render::rows` being required with no default makes the `Json | Ndjson`
//! collapse *visible* — an impl that wants one document per line has to say
//! so by emitting no rows — but visible is not the same as impossible, so the
//! guarantee is here. These are claims about the seam, asserted rather than
//! snapshotted, because a snapshot review can wave through exactly the change
//! these forbid.

use zenctl::render::{Cell, Format, Grid, Note, Render, Row, Table, Width, to_string};

#[derive(serde::Serialize)]
struct Doc {
    asked: Vec<String>,
    /// The field that must be **absent** when nothing asked for it, never
    /// `null` (RFC 09 §5.1 O4).
    #[serde(skip_serializing_if = "Option::is_none")]
    not_probed: Option<String>,
    #[serde(skip)]
    items: Vec<&'static str>,
    #[serde(skip)]
    notes: Vec<Note>,
}

impl Render for Doc {
    const FAMILY: &'static str = "doc";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        // Derived from the serialized struct, never hand-assembled: the three
        // hand-built envelopes in `output.rs` each null'd a field their struct
        // deliberately omits, and the golden corpus could not see it because
        // it pins the struct and they were built beside it (#232).
        match serde_json::to_value(self).expect("serializes") {
            serde_json::Value::Object(m) => m,
            _ => unreachable!("a report is an object"),
        }
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for i in &self.items {
            out(Row::of("item", &serde_json::json!({ "name": i })));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut g = Grid::unheaded(1);
        for i in &self.items {
            g.row([Cell::text(*i)]);
        }
        t.grid(g);
    }

    fn notes(&self) -> Vec<Note> {
        self.notes.clone()
    }
}

fn doc() -> Doc {
    Doc {
        asked: vec!["v1/**".into()],
        not_probed: None,
        items: vec!["one", "two"],
        notes: Vec::new(),
    }
}

fn ndjson(d: &Doc) -> (Vec<serde_json::Value>, String) {
    let (out, err) = to_string(d, Format::Ndjson, Width::Unbounded).expect("render");
    let lines = out
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("line {l:?} is not JSON: {e}")))
        .collect();
    (lines, err)
}

/// The headline bug: `--format ndjson` used to emit a pretty multi-line
/// document for six families, which cannot be read a line at a time — the only
/// reason the format exists.
#[test]
fn every_ndjson_line_parses_on_its_own() {
    let (lines, _) = ndjson(&doc());
    assert!(lines.len() > 1, "a report with rows is more than one line");
}

/// One envelope, then one line per row. A consumer can count.
#[test]
fn the_ndjson_line_count_is_one_envelope_plus_the_rows() {
    let d = doc();
    let (lines, _) = ndjson(&d);
    assert_eq!(lines.len(), 1 + d.items.len());
    assert_eq!(lines[0]["report"], "doc", "the envelope leads");
    assert!(lines[0].get("row").is_none(), "an envelope is not a row");
    for row in &lines[1..] {
        assert_eq!(row["row"], "item", "every row carries its kind");
        assert!(row.get("report").is_none(), "a row is not an envelope");
    }
}

/// The envelope leads so that a truncated stream still carries the coverage
/// claim. `zenctl topic hz --format ndjson | head -5` used to lose the O6
/// eviction count, which is exactly the number a shortened stream needs.
#[test]
fn the_first_line_carries_the_coverage_even_if_the_rest_is_cut() {
    let (lines, _) = ndjson(&doc());
    let head = &lines[0];
    assert_eq!(head["asked"], serde_json::json!(["v1/**"]));
}

/// A question nobody asked is **absent**, not `null` — asserted as a whole
/// document, because `json["absent"]` is `Null` and a field-by-field check
/// passes identically for the two.
#[test]
fn an_unasked_field_is_absent_from_the_envelope_rather_than_null() {
    let (lines, _) = ndjson(&doc());
    assert_eq!(
        lines[0],
        serde_json::json!({"report": "doc", "asked": ["v1/**"]}),
        "no `not_probed: null` — the struct's `skip_serializing_if` has to \
         survive the envelope, which is what the hand-built ones did not"
    );

    let mut d = doc();
    d.not_probed = Some("no probe endpoint for this tier".into());
    let (lines, _) = ndjson(&d);
    assert_eq!(lines[0]["not_probed"], "no probe endpoint for this tier");
}

/// Honesty notes reach the machine formats; advice for a person does not.
#[test]
fn a_coverage_note_rides_the_document_and_a_next_step_does_not() {
    let mut d = doc();
    d.notes = vec![
        Note::coverage("only the two tiers named above were probed").cite("RFC 09 §5.1 O5"),
        Note::next_step("call one with: zenctl service call …"),
        Note::summary("2 item(s)."),
    ];
    let (lines, err) = ndjson(&d);
    let notes = lines[0]["notes"]
        .as_array()
        .expect("notes ride the envelope");
    assert_eq!(notes.len(), 1, "only the honesty note: {notes:?}");
    assert_eq!(notes[0]["cite"], "RFC 09 §5.1 O5");

    // All three still reach a person, and nothing about the data reaches
    // stdout: stdout carries what a script consumes, stderr what is true
    // about it.
    assert!(err.contains("only the two tiers"), "{err}");

    let (table_out, table_err) = to_string(&d, Format::Table, Width::Unbounded).unwrap();
    assert!(!table_out.contains("call one with"), "notes are not rows");
    assert!(table_err.contains("call one with"), "{table_err}");
    assert!(table_err.contains("2 item(s)"), "{table_err}");
}

/// The two machine formats are the same values, differently packaged — so a
/// consumer can switch on stream size rather than on shape.
#[test]
fn the_json_document_holds_what_the_ndjson_stream_holds() {
    let d = doc();
    let (json, _) = to_string(&d, Format::Json, Width::Unbounded).unwrap();
    let json: serde_json::Value = serde_json::from_str(&json).expect("one document");
    let (lines, _) = ndjson(&d);

    assert_eq!(json["report"], lines[0]["report"]);
    assert_eq!(json["asked"], lines[0]["asked"]);
    let rows = json["rows"].as_array().expect("rows ride the document");
    assert_eq!(rows.len(), lines.len() - 1);
    assert_eq!(rows[0], lines[1]);
}

/// A family with nothing to decompose emits its envelope and stops. Legitimate
/// — `record`, `replay`, `cutover` and `expect` have no rows — and listed here
/// so a fifth cannot join them by accident.
#[test]
fn a_row_less_family_emits_exactly_one_line() {
    let mut d = doc();
    d.items.clear();
    let (lines, _) = ndjson(&d);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].get("rows").is_none(), "and no empty rows array");
}
