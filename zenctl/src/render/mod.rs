//! One report, three renderings — and exactly one place that knows which
//! (#198).
//!
//! `output.rs` matched `format.resolved()` in 24 renderers and `cmd/*` in 16
//! more, each re-deriving the same three-way decision. Six of them collapsed
//! `Json | Ndjson` into a single arm and printed a *pretty, multi-line*
//! document under `--format ndjson` — which is not ndjson, because it cannot
//! be read a line at a time, and being readable a line at a time is the only
//! reason the format exists.
//!
//! [`emit`] is the only `match` on [`Format`] in the crate. A [`Render`] impl
//! never learns which format it is in: it decomposes itself into rows, draws
//! itself into a [`Table`], and states what is true *about* it as [`Note`]s.
//!
//! ## The ndjson convention, in three lines
//!
//! > Every line is a JSON object carrying a kind tag. A **report** emits its
//! > envelope first, then its tagged rows. A **stream** emits tagged rows and
//! > no envelope.
//!
//! * envelope — `{"report": "<family>", …report-level facts…, "notes": [...]}`
//! * row — `{"row": "<kind>", …fields…}`
//! * a consumer tells them apart by which tag key is present:
//!   `jq -c 'select(.row)'`.
//!
//! **The envelope leads**, and the reason is truncation rather than taste. A
//! consumer piping to `head`, or a pipe that closes early, still gets the
//! coverage claim — what was asked (O5), and why nothing was (O4). A trailing
//! envelope is lost in exactly the case where honesty matters most: today
//! `zenctl topic hz --format ndjson | head -5` silently drops "we retired 900
//! keys to stay within the bound". `blob probe` already argued for a leading
//! envelope, in a comment; it was right, it just was not the rule.
//!
//! **A stream gets no envelope**, and that is principled twice over. A
//! report's coverage is known before the first row; a stream's is not — its
//! coverage statement is the `seed_complete` line, which arrives when it
//! becomes true. And `topic echo --format ndjson` is an *input* format:
//! `zenkey_fleet::ingest::parse_row` reads it back for `topic pub --from
//! ndjson` and `.zrec` (RFC 09 §5.2), and it counts a line it cannot parse as
//! *malformed*. Extra fields on a row are ignored; a prepended envelope line
//! would break the round trip (#235).
//!
//! ## stdout carries the data; stderr carries everything about it
//!
//! One rule, replacing a habit. It is why [`emit_to`] takes two writers: five
//! honesty paragraphs were already on stderr and a dozen more were on stdout,
//! where they broke `| awk` for anyone who piped a table.

mod impls;
pub use impls::RateView;
pub use impls::local::{CacheReport, CachedSlice};
pub mod table;

use std::io::Write;

use anyhow::Result;
use serde::Serialize;

pub use table::{Cell, Grid, Table, Width};

/// Which rendering the user asked for.
///
/// Moved here from `output.rs` (#198): the type belongs next to the single
/// place that matches on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    Auto,
    Table,
    Json,
    Ndjson,
}

impl Format {
    /// Resolve `auto` against the output stream: a table for a person, ndjson
    /// for whatever is on the other end of the pipe.
    pub fn resolved(self) -> Format {
        match self {
            Format::Auto => {
                if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
                    Format::Table
                } else {
                    Format::Ndjson
                }
            }
            other => other,
        }
    }
}

/// What kind of thing a note is.
///
/// The kind is what turns twenty grepped paragraphs into something a test can
/// assert. A test can say *"a probe that was never issued yields a
/// [`NoteKind::Coverage`] citing O4"* without pinning wording that a
/// copy-edit will change — and pinning wording is how honesty tests become
/// the thing people delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteKind {
    /// RFC 09 §5.1 O4/O5 — what was asked, and what was not.
    Coverage,
    /// RFC 05 §3.1 — a zero result that is not a verdict.
    Silence,
    /// RFC 09 §5.1 O6 — what the bounded observer dropped or retired.
    Bound,
    /// How to read a number: which clock, which population (O7).
    Caveat,
    /// A count or a total. Derivable from the data by anyone who has it.
    Summary,
    /// "call one with: …" — an affordance for a person at a terminal.
    NextStep,
    /// About *this rendering*, not about the fleet.
    Rendering,
}

impl NoteKind {
    /// Whether a note of this kind belongs in the machine-readable formats.
    ///
    /// The honesty kinds do: a script that cannot see the coverage claim is
    /// exactly the consumer O5 was written for. `Summary` does not — the
    /// envelope already carries the numbers it restates — and `NextStep` and
    /// `Rendering` are advice for a person and a statement about a table that
    /// is not there.
    fn machine_readable(self) -> bool {
        matches!(
            self,
            NoteKind::Coverage | NoteKind::Silence | NoteKind::Bound | NoteKind::Caveat
        )
    }
}

/// Something true *about* a report, which must survive every format.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub kind: NoteKind,
    pub text: String,
    /// The clause it discharges — `"RFC 09 §5.1 O4"`, `"RFC 05 §3.1"`.
    pub cite: Option<&'static str>,
}

impl Note {
    pub fn new(kind: NoteKind, text: impl Into<String>) -> Note {
        Note {
            kind,
            text: text.into(),
            cite: None,
        }
    }

    pub fn cite(mut self, cite: &'static str) -> Note {
        self.cite = Some(cite);
        self
    }

    pub fn coverage(text: impl Into<String>) -> Note {
        Note::new(NoteKind::Coverage, text)
    }
    pub fn silence(text: impl Into<String>) -> Note {
        Note::new(NoteKind::Silence, text).cite("RFC 05 §3.1")
    }
    pub fn bound(text: impl Into<String>) -> Note {
        Note::new(NoteKind::Bound, text).cite("RFC 09 §5.1 O6")
    }
    pub fn caveat(text: impl Into<String>) -> Note {
        Note::new(NoteKind::Caveat, text)
    }
    pub fn summary(text: impl Into<String>) -> Note {
        Note::new(NoteKind::Summary, text)
    }
    pub fn next_step(text: impl Into<String>) -> Note {
        Note::new(NoteKind::NextStep, text)
    }
    pub fn rendering(text: impl Into<String>) -> Note {
        Note::new(NoteKind::Rendering, text)
    }

    fn as_json(&self) -> serde_json::Value {
        let mut o = serde_json::Map::new();
        o.insert("text".into(), self.text.clone().into());
        if let Some(c) = self.cite {
            o.insert("cite".into(), c.into());
        }
        serde_json::Value::Object(o)
    }
}

/// One ndjson row: a JSON object that carries its own kind tag.
#[derive(Debug, Clone)]
pub struct Row(serde_json::Map<String, serde_json::Value>);

impl Row {
    /// `Row::of("subject", s)` — the value's fields, plus `"row": "subject"`.
    ///
    /// The tag is what makes a heterogeneous stream readable. `storage list`
    /// emitted storages and coverage rows onto one stream with nothing to
    /// tell them apart, so a consumer had to identify a line by guessing at
    /// its fields; `admin graph` emitted three such streams.
    pub fn of<T: Serialize>(kind: &'static str, value: &T) -> Row {
        let v = serde_json::to_value(value).expect("a report row serializes");
        let mut map = match v {
            serde_json::Value::Object(m) => m,
            other => {
                let mut m = serde_json::Map::new();
                m.insert("value".into(), other);
                m
            }
        };
        map.insert("row".into(), kind.into());
        Row(map)
    }

    fn into_line(self) -> String {
        serde_json::to_string(&serde_json::Value::Object(self.0)).expect("a row serializes")
    }
}

/// A report, and the three ways it can be shown.
pub trait Render: Serialize {
    /// The family name: the envelope's `report` field, and the key a snapshot
    /// is filed under.
    const FAMILY: &'static str;

    /// The report-level facts — everything that is *not* a row. What was
    /// asked (O5), why nothing was (O4), what the bound cost (O6), totals.
    ///
    /// Empty is the honest answer for a family that is nothing but a vector;
    /// its envelope is then the bare `{"report": "…"}` discriminator.
    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        serde_json::Map::new()
    }

    /// The row decomposition. **Required, with no default.**
    ///
    /// There is deliberately nothing to fall through to: an impl that wants
    /// to emit the whole document as one line has to say so by emitting no
    /// rows, which is a reviewable act rather than an omission. Four families
    /// legitimately do (`record`, `replay`, `cutover`, `expect`) — they have
    /// no rows — and the test floor keeps an explicit list of them, so adding
    /// a fifth is a diff.
    fn rows(&self, out: &mut dyn FnMut(Row));

    /// The human rendering. Total: no early returns, no `Format`.
    ///
    /// An empty result renders **nothing** here, not even a header — the
    /// sentence explaining it is a [`Note`], which is what gets it into json
    /// and ndjson too. Eleven renderers used to `return` early from the table
    /// arm with that sentence, which is exactly why `zenctl topic list
    /// --format ndjson` against an empty fleet printed nothing at all.
    fn table(&self, t: &mut Table);

    /// What is true about this report.
    fn notes(&self) -> Vec<Note> {
        Vec::new()
    }
}

/// The terminal budget, decided once.
///
/// `$COLUMNS` first (a user who exports it means it), then the terminal
/// itself, then [`Width::Unbounded`] — never a guessed 80 for a stream that
/// is not a terminal.
pub fn term_width() -> Width {
    if let Ok(cols) = std::env::var("COLUMNS")
        && let Ok(n) = cols.trim().parse::<usize>()
        && n >= 20
    {
        return Width::Cells(n);
    }
    #[cfg(unix)]
    {
        use std::io::IsTerminal as _;
        if std::io::stdout().is_terminal() {
            let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
            // SAFETY: TIOCGWINSZ writes a `winsize` and nothing else; the
            // pointer is to a live local of exactly that type.
            let rc = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
            if rc == 0 && ws.ws_col >= 20 {
                return Width::Cells(ws.ws_col as usize);
            }
        }
    }
    Width::Unbounded
}

/// Render one report to a writer.
///
/// The convenience form: notes go to stderr, and the width is the terminal's.
/// [`emit_to`] is the one the tests use, because capturing both streams is
/// what makes "the honesty paragraph is present in ndjson mode" assertable.
pub fn emit<R: Render>(w: &mut impl Write, r: &R, format: Format) -> Result<()> {
    let mut err = std::io::stderr();
    emit_to(w, &mut err, r, format, term_width())
}

/// Render one report, with both streams and the width supplied.
///
/// The only `match` on [`Format`] in the crate.
pub fn emit_to<R: Render>(
    out: &mut impl Write,
    err: &mut impl Write,
    r: &R,
    format: Format,
    width: Width,
) -> Result<()> {
    let notes = match format.resolved() {
        Format::Table => {
            let mut table = Table::new();
            r.table(&mut table);
            write!(out, "{}", table.render(width))?;
            // The table's own notes first — they are about the rendering the
            // reader just looked at — then the report's, which are about the
            // fleet.
            let mut notes = table.rendering_notes().to_vec();
            notes.extend(r.notes());
            notes
        }
        Format::Json => {
            let mut doc = r.envelope();
            doc.insert("report".into(), R::FAMILY.into());
            let mut rows = Vec::new();
            r.rows(&mut |row| rows.push(serde_json::Value::Object(row.0)));
            if !rows.is_empty() {
                doc.insert("rows".into(), serde_json::Value::Array(rows));
            }
            let notes = r.notes();
            insert_notes(&mut doc, &notes);
            writeln!(
                out,
                "{}",
                serde_json::to_string_pretty(&serde_json::Value::Object(doc))?
            )?;
            notes
        }
        Format::Ndjson => {
            let mut envelope = r.envelope();
            envelope.insert("report".into(), R::FAMILY.into());
            let notes = r.notes();
            insert_notes(&mut envelope, &notes);
            writeln!(
                out,
                "{}",
                serde_json::to_string(&serde_json::Value::Object(envelope))?
            )?;
            let mut wrote = Ok(());
            r.rows(&mut |row| {
                if wrote.is_ok() {
                    wrote = writeln!(out, "{}", row.into_line());
                }
            });
            wrote?;
            notes
        }
        // `resolved()` never yields it, and the compiler cannot know that.
        Format::Auto => unreachable!("Format::Auto is resolved before it is matched"),
    };
    for note in &notes {
        if format.resolved() != Format::Table && !note.kind.machine_readable() {
            continue;
        }
        match note.cite {
            Some(c) => writeln!(err, "{} ({c})", note.text)?,
            None => writeln!(err, "{}", note.text)?,
        }
    }
    // `process::exit` is called after rendering at twenty sites and does not
    // run destructors, so a buffered writer would lose the document it was
    // handed. Flush before returning rather than trusting a drop that will
    // not happen.
    out.flush()?;
    err.flush()?;
    Ok(())
}

fn insert_notes(doc: &mut serde_json::Map<String, serde_json::Value>, notes: &[Note]) {
    let carried: Vec<serde_json::Value> = notes
        .iter()
        .filter(|n| n.kind.machine_readable())
        .map(Note::as_json)
        .collect();
    if !carried.is_empty() {
        doc.insert("notes".into(), serde_json::Value::Array(carried));
    }
}

/// Render a report into a `String`, for a caller that is not writing to a
/// stream — the completion cache, a test, a future `--output <file>`.
pub fn to_string<R: Render>(r: &R, format: Format, width: Width) -> Result<(String, String)> {
    let mut out = Vec::new();
    let mut err = Vec::new();
    emit_to(&mut out, &mut err, r, format, width)?;
    Ok((String::from_utf8(out)?, String::from_utf8(err)?))
}
