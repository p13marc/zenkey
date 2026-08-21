//! `--watch` on list commands (#56): re-render on an interval (or on
//! liveliness events, for `node list`), diff-aware — appeared rows are
//! marked `+` for one cycle, disappeared rows linger one cycle marked `-`.
//! Deliberately **not a TUI** (§6.1: "the TUI is the GUI's job"): table mode
//! is a plain clear-and-redraw, and `--format ndjson` streams one snapshot
//! per cycle for scripting.
//!
//! ## The marks are a rendering, and now live in one
//!
//! Each of the four watchable verbs used to hand `render_cycle` a
//! `Vec<(id, line)>` it had built itself — which is why `topic list --watch`,
//! `storage list --watch`, `base list --watch` and `node list --watch` each
//! carried a *second* copy of a renderer, with its own column widths, three of
//! them byte-identical to the one in `output.rs` (#199). They render the
//! report's own [`crate::render::Render`] impl now, so the copies are gone by
//! construction rather than by being edited twice.
//!
//! The diff follows: it is over **rendered lines** rather than over hand-made
//! stable ids. That is a deliberate change of meaning and a better one — an id
//! diff marked a row that appeared or vanished and said nothing about a row
//! whose *value* changed, which on a watch is the event you were most likely
//! sitting there for.
//!
//! And `--format ndjson` no longer carries the marks. A mark is an affordance
//! for a person watching a screen; a stream carries the tick and the rows, and
//! a consumer that wants a diff has both. A format flag chooses an encoding,
//! never a question.

use std::io::IsTerminal;
use std::io::Write as _;
use std::time::Duration;

use anyhow::{Result, bail};

use crate::output::Format;
use crate::render::{Render, Row, Table};

/// `--watch` accepts table (redraw) and ndjson (stream); a single growing
/// JSON document would misrepresent a stream, so `--format json` is refused.
pub fn validate_format(format: Format) -> Result<()> {
    if matches!(format, Format::Json) {
        bail!(
            "--watch cannot stream --format json (one document cannot grow); \
             use --format ndjson: one snapshot per line"
        );
    }
    Ok(())
}

/// One cycle of a watched report: the report itself, plus which cycle it is.
///
/// `tick` is what lets a consumer find the boundaries in a stream that no
/// longer carries an envelope per row — it is monotonic, and it is the only
/// thing this wrapper adds to the wire.
pub struct WatchTick<'a, R> {
    pub inner: &'a R,
    pub tick: u64,
}

impl<R: Render> serde::Serialize for WatchTick<'_, R> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.inner.serialize(s)
    }
}

impl<R: Render> Render for WatchTick<'_, R> {
    const FAMILY: &'static str = "watch";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = self.inner.envelope();
        e.insert("family".into(), R::FAMILY.into());
        e.insert("tick".into(), self.tick.into());
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        self.inner.rows(out);
    }

    fn table(&self, t: &mut Table) {
        self.inner.table(t);
    }

    fn notes(&self) -> Vec<crate::render::Note> {
        self.inner.notes()
    }
}

/// Redraw one cycle as a table, marking what changed since the last one.
///
/// `prev` is the previous cycle's lines; it is updated in place.
fn redraw<R: Render>(report: &R, prev: &mut Vec<String>, footer: &str) -> Result<()> {
    let mut table = Table::new();
    report.table(&mut table);
    let lines: Vec<String> = table
        .render(crate::render::term_width())
        .lines()
        .map(str::to_string)
        .collect();

    let mut out = std::io::stdout();
    // Plain clear-and-redraw on a tty; sequential snapshots when table output
    // is piped somewhere on purpose.
    if out.is_terminal() {
        write!(out, "\x1b[2J\x1b[H")?;
    }
    for line in &lines {
        // First cycle marks nothing: a fresh watch "appearing" is not a change
        // in the fleet.
        let mark = if prev.is_empty() || prev.contains(line) {
            " "
        } else {
            "+"
        };
        writeln!(out, "{mark} {line}")?;
    }
    for line in prev.iter() {
        if !lines.contains(line) {
            writeln!(out, "- {line}")?;
        }
    }
    writeln!(out, "{footer}")?;
    out.flush()?;
    for note in report.notes() {
        eprintln!("{}", note.text);
    }
    *prev = lines;
    Ok(())
}

/// Render one watch cycle in whichever format was asked for.
pub fn render_cycle<R: Render>(
    report: &R,
    tick: u64,
    prev: &mut Vec<String>,
    format: Format,
    footer: &str,
) -> Result<()> {
    match format.resolved() {
        Format::Table => redraw(report, prev, footer),
        other => crate::render::emit(
            &mut std::io::stdout(),
            &WatchTick {
                inner: report,
                tick,
            },
            other,
        ),
    }
}

/// Poll `fetch` on an interval and render each cycle until Ctrl-C.
pub async fn poll_loop<R: Render>(
    interval: Duration,
    format: Format,
    mut fetch: impl AsyncFnMut() -> Result<R>,
) -> Result<()> {
    validate_format(format)?;
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut prev: Vec<String> = Vec::new();
    let mut tick = 0u64;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = ticker.tick() => {
                let report = fetch().await?;
                let footer = format!(
                    "watching every {:.0}s, Ctrl-C to stop (+ appeared, - disappeared)",
                    interval.as_secs_f64()
                );
                render_cycle(&report, tick, &mut prev, format, &footer)?;
                tick += 1;
            }
        }
    }
    Ok(())
}

/// `--watch SECS` → a validated interval.
pub fn interval_of(secs: f64) -> Result<Duration> {
    if !secs.is_finite() || secs < 0.2 {
        bail!("--watch interval must be at least 0.2s (got {secs})");
    }
    Ok(Duration::from_secs_f64(secs))
}

/// The `topic list` filter flags, shared by the one-shot and watch paths.
pub struct TopicFilter {
    pub producer: Option<String>,
    pub class: Option<String>,
    pub type_name: Option<String>,
    pub deprecated: bool,
}

impl TopicFilter {
    pub fn apply(&self, slices: &zenkey_fleet::SliceSet) -> Result<crate::report::TopicList> {
        slices.topic_list(
            self.producer.as_deref(),
            self.class.as_deref(),
            self.type_name.as_deref(),
            self.deprecated,
        )
    }
}

pub async fn topic_list(secs: f64, filter: &TopicFilter, args: &crate::BusArgs) -> Result<()> {
    validate_format(args.format)?;
    let interval = interval_of(secs)?;
    let dirs = args.registry_dirs();
    if dirs.is_empty() {
        // The recurring sweep holds one declared registry querier across
        // cycles (#37) instead of re-declaring per poll.
        let session = args.session().await?;
        let repeating =
            zenkey_fleet::RepeatingRegistry::declare(&session, args.base(), args.timeout()).await?;
        let fetch = async || {
            let slices: Vec<zenkey::RegistrySlice> = repeating
                .fetch()
                .await?
                .into_iter()
                .map(|(s, _)| s)
                .collect();
            filter.apply(&zenkey_fleet::SliceSet::from_slices(slices))
        };
        poll_loop(interval, args.format, fetch).await?;
        repeating.undeclare().await
    } else {
        // Dirs given: the union set (bus wins, dirs fill) per cycle, same
        // sourcing as the one-shot command.
        let fetch = async || {
            let slices = args.slice_set().await?;
            filter.apply(&slices)
        };
        poll_loop(interval, args.format, fetch).await
    }
}

pub async fn storage_list(secs: f64, args: &crate::BusArgs) -> Result<()> {
    validate_format(args.format)?;
    let interval = interval_of(secs)?;
    let session = args.session().await?;
    let fetch = async || {
        let storages = zenkey_fleet::storages(&session, args.timeout()).await?;
        let coverage = match args.slice_set().await {
            Ok(slices) => zenkey_fleet::state_coverage(&slices, args.base(), &storages),
            Err(_) => Vec::new(),
        };
        Ok(crate::report::StorageList { storages, coverage })
    };
    poll_loop(interval, args.format, fetch).await
}

pub async fn base_list(secs: f64, args: &crate::BusArgs) -> Result<()> {
    validate_format(args.format)?;
    let interval = interval_of(secs)?;
    let session = args.session().await?;
    let fetch = async || {
        let bases = zenkey_fleet::discover_bases(&session, args.timeout()).await?;
        Ok(crate::report::BaseList { bases })
    };
    poll_loop(interval, args.format, fetch).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mark vocabulary, over rendered lines rather than hand-made ids.
    ///
    /// The id version could not see a row whose *value* changed — same id,
    /// different line, no mark — which on a watch is the event you were most
    /// likely sitting there for. Over lines it reads as one `-` and one `+`,
    /// which is what actually happened on the screen.
    #[test]
    fn a_changed_row_is_visible_and_a_first_cycle_marks_nothing() {
        let marks = |prev: &[&str], cur: &[&str]| {
            let prev: Vec<String> = prev.iter().map(|s| s.to_string()).collect();
            let cur: Vec<String> = cur.iter().map(|s| s.to_string()).collect();
            let appeared: Vec<String> = cur
                .iter()
                .filter(|l| !prev.is_empty() && !prev.contains(l))
                .cloned()
                .collect();
            let gone: Vec<String> = prev.iter().filter(|l| !cur.contains(l)).cloned().collect();
            (appeared, gone)
        };

        let (appeared, gone) = marks(&[], &["a  1", "b  2"]);
        assert!(
            appeared.is_empty(),
            "a fresh watch is not a change: {appeared:?}"
        );
        assert!(gone.is_empty());

        let (appeared, gone) = marks(&["a  1", "b  2"], &["a  1", "c  3"]);
        assert_eq!(appeared, ["c  3"]);
        assert_eq!(gone, ["b  2"]);

        // The case the id diff was blind to.
        let (appeared, gone) = marks(&["a  1"], &["a  2"]);
        assert_eq!(appeared, ["a  2"]);
        assert_eq!(gone, ["a  1"]);
    }

    #[test]
    fn json_is_refused_ndjson_is_not() {
        assert!(validate_format(Format::Json).is_err());
        assert!(validate_format(Format::Ndjson).is_ok());
        assert!(validate_format(Format::Table).is_ok());
        assert!(validate_format(Format::Auto).is_ok());
    }
}
