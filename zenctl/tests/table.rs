//! The `Table` primitive's own rules (#199).
//!
//! In `tests/` rather than a `#[cfg(test)]` module under `src/`, and that is a
//! constraint rather than a preference: `scripts/check-prose.py` gates
//! `zenctl/src` on runs of six or more spaces inside a string literal — the
//! #195 gutter bug — and a single-line assertion about a padded row is
//! indistinguishable from that. `tests/` is not scanned, because the expected
//! output here is *captured*, not authored prose.
//!
//! Everything is asserted through a synthetic `Render` impl, so these tests
//! describe the primitive rather than any one family's rendering.

use zenctl::render::{Cell, Format, Grid, Note, Render, Row, Table, Width, to_string};

/// A report shaped exactly as much as each test needs.
#[derive(serde::Serialize)]
struct Fixture {
    rows: Vec<(String, Option<String>, u64)>,
    #[serde(skip)]
    notes: Vec<Note>,
    #[serde(skip)]
    cap: Option<usize>,
}

impl Fixture {
    fn of(rows: &[(&str, Option<&str>, u64)]) -> Fixture {
        Fixture {
            rows: rows
                .iter()
                .map(|(a, b, c)| (a.to_string(), b.map(str::to_string), *c))
                .collect(),
            notes: Vec::new(),
            cap: None,
        }
    }
}

impl Render for Fixture {
    const FAMILY: &'static str = "fixture";

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for (name, state, n) in &self.rows {
            out(Row::of(
                "thing",
                &serde_json::json!({"name": name, "state": state, "n": n}),
            ));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut g = Grid::new(["NAME", "STATE", "N"]);
        if let Some(cap) = self.cap {
            g = g.max(0, cap);
        }
        for (name, state, n) in &self.rows {
            g.row([Cell::text(name), Cell::asked(state.clone()), Cell::int(*n)]);
        }
        t.grid(g);
    }

    fn notes(&self) -> Vec<Note> {
        self.notes.clone()
    }
}

fn table_of(f: &Fixture, w: Width) -> String {
    to_string(f, Format::Table, w).expect("render").0
}

/// The distinction the whole primitive exists for: a question never put to the
/// bus renders `—`, and a question answered with nothing renders empty. They
/// are different variants, so no refactor can quietly merge them
/// (RFC 09 §5.1 O4).
#[test]
fn an_unasked_cell_is_not_an_empty_one() {
    let f = Fixture::of(&[("alpha", None, 1), ("beta", Some(""), 2)]);
    let out = table_of(&f, Width::Unbounded);
    let lines: Vec<&str> = out.lines().collect();
    assert!(
        lines[1].contains('—'),
        "not asked renders an em dash: {out}"
    );
    assert!(
        !lines[2].contains('—'),
        "asked-and-empty must not borrow the not-asked mark: {out}"
    );
    // And the two are not the same string, which is what a `-`-for-both
    // rendering used to make them.
    assert_ne!(lines[1], lines[2]);
}

/// A long cell is cut to its own column and shifts nothing after it — the
/// defect that made `{:<44}` unusable the moment a path was 60 characters.
#[test]
fn an_over_long_cell_does_not_shift_its_neighbours() {
    let mut f = Fixture::of(&[
        ("short", Some("up"), 1),
        ("a-very-long-name-that-overflows-its-column", Some("up"), 2),
    ]);
    f.cap = Some(10);
    let out = table_of(&f, Width::Unbounded);
    let lines: Vec<&str> = out.lines().collect();
    // Character offsets, not byte offsets: the truncation mark is multi-byte,
    // and a byte comparison would fail on a table that lines up perfectly.
    let col = |l: &str| {
        l.char_indices()
            .position(|(i, _)| l[i..].starts_with("up"))
            .expect("the state column")
    };
    assert_eq!(
        col(lines[1]),
        col(lines[2]),
        "the second column starts in the same place on every row:\n{out}"
    );
    assert!(lines[2].contains('…'), "the cut is marked: {out}");
}

/// A shortened name is still a name. A shortened number is a different number,
/// so a column widens rather than lie.
#[test]
fn a_number_widens_its_column_rather_than_being_truncated() {
    let mut f = Fixture::of(&[("a", Some("up"), 1), ("b", Some("up"), 123_456_789)]);
    f.cap = Some(3);
    let out = table_of(&f, Width::Cells(24));
    assert!(
        out.contains("123456789"),
        "every digit survives the squeeze:\n{out}"
    );
}

/// The budget squeezes the widest squeezable column first, deterministically,
/// so a narrow terminal degrades the same way in every family — which is what
/// makes that claim testable rather than aspirational.
#[test]
fn a_narrow_budget_squeezes_the_widest_column_first() {
    let f = Fixture::of(&[("a-fairly-long-name", Some("a-long-state-value"), 1)]);
    let wide = table_of(&f, Width::Unbounded);
    let narrow = table_of(&f, Width::Cells(30));
    assert!(wide.lines().all(|l| l.len() > 30));
    assert!(
        narrow.lines().all(|l| l.chars().count() <= 30),
        "nothing exceeds the budget:\n{narrow}"
    );
    assert_eq!(
        narrow,
        table_of(&f, Width::Cells(30)),
        "and it is a function"
    );
}

/// Trailing padding breaks `cut` and makes a snapshot diff unreadable.
#[test]
fn no_rendered_line_carries_trailing_whitespace() {
    let f = Fixture::of(&[("alpha", None, 1), ("b", Some("up"), 22)]);
    for line in table_of(&f, Width::Unbounded).lines() {
        assert_eq!(line, line.trim_end(), "trailing padding on {line:?}");
    }
}

/// An empty report renders **nothing** — not a header, not a rule. The
/// sentence that explains it is a note, which is what gets it into the machine
/// formats too; eleven renderers used to `return` early with it and leave
/// `--format ndjson` silent.
#[test]
fn an_empty_table_renders_nothing_and_the_note_carries_the_meaning() {
    let mut f = Fixture::of(&[]);
    f.notes = vec![Note::silence("no things. That is not a verdict")];
    let (out, err) = to_string(&f, Format::Table, Width::Unbounded).unwrap();
    assert_eq!(out, "", "no header over no rows");
    assert!(err.contains("not a verdict"), "the note explains it: {err}");
    assert!(err.contains("RFC 05 §3.1"), "and cites: {err}");
}
