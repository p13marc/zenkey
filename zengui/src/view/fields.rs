//! The Inspector's Fields section (#223): per-path statistics over one
//! bounded observation window — the stuck sensor that passes every check.
//!
//! Validation is per-sample and per-key stats are about arrival; between
//! them sits the field that stopped moving while both stayed green. The
//! engine's [`zenkey_fleet::judge::field`] observation is the judge; this section
//! is its GUI face, and it follows the doctor's cost posture: **run on
//! demand, never ambient** — the button declares one subscriber on exactly
//! the subject key, holds it for the stated window, and provably releases
//! it (`services::value::field`).
//!
//! Every bound reports its cost (RFC 09 §5.1 O6): the path table states its
//! cap and names what it refused; the observer states its drops; the window
//! states itself. Sparklines ride where `spark.rs` fits — numeric paths the
//! subject's own recorded history can plot — and each states that its window
//! is the history ring's, not the observation's.

use std::sync::Arc;

use iced::widget::{Column, row};
use zenkey_fleet::report::{DoctorSeverity, FieldReport};

use crate::message::{Message, PaneMsg, SlotId};
use crate::series::Series;
use crate::view::kit;
use crate::view::theme::{SeriesTone, SeverityTone};
use crate::view::tokens::{Spacing, font};

/// Rows the table draws before stopping and saying so — a display bound on
/// top of the engine's path-table bound, disclosed like every other (O6).
const FIELD_ROWS: usize = 40;

/// Sparklines drawn under the table, at most. Each canvas retains geometry,
/// so an unbounded list would be an unbounded cache.
const SPARK_ROWS: usize = 4;

/// The section's interactions.
#[derive(Debug, Clone)]
pub enum FieldsMsg {
    /// The observe button — the only way a window opens (the doctor's
    /// run-on-demand posture).
    Run,
    /// The window input, seconds.
    WindowChanged(String),
    /// A window finished.
    Done(Result<Arc<FieldReport>, crate::services::ServiceError>),
}

/// One numeric path's sparkline, built when the report lands — from the
/// subject's recorded history, which is the only per-sample series the app
/// retains (#64's rule: no schema decode on a render path).
pub struct PathSpark {
    pub path: String,
    pub series: Series,
    pub cache: iced::widget::canvas::Cache,
}

/// The section's state — follows the subject; `update/subject.rs` drops the
/// report on re-selection because it is evidence about the old key.
pub struct FieldsState {
    /// The window input, seconds. User input, kept across subjects the way
    /// the doctor keeps its listen field.
    pub window: String,
    pub in_flight: bool,
    /// The key the running (or landed) observation was asked about — the
    /// staleness guard: a report for a superseded subject never lands.
    pub asked: Option<String>,
    pub report: Option<Result<Arc<FieldReport>, crate::services::ServiceError>>,
    /// Sparklines for up to `SPARK_ROWS` numeric paths, built at landing.
    pub sparks: Vec<PathSpark>,
}

impl Default for FieldsState {
    fn default() -> FieldsState {
        FieldsState {
            window: "10".to_string(),
            in_flight: false,
            asked: None,
            report: None,
            sparks: Vec::new(),
        }
    }
}

impl FieldsState {
    /// The parsed window, clamped to something a subscriber may be held for.
    pub fn window_secs(&self) -> Option<u64> {
        self.window
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|s| (1..=300).contains(s))
    }

    /// A new subject: the old report is evidence about the old key. The
    /// window input survives — it is the user's, not the fleet's.
    pub fn forget_subject(&mut self) {
        self.in_flight = false;
        self.asked = None;
        self.report = None;
        self.sparks = Vec::new();
    }
}

fn msg(slot: SlotId, m: FieldsMsg) -> Message {
    Message::Pane(PaneMsg::Fields(slot, m))
}

fn severity_tone(severity: DoctorSeverity) -> SeverityTone {
    match severity {
        DoctorSeverity::Error => SeverityTone::Error,
        DoctorSeverity::Warning => SeverityTone::Warning,
        DoctorSeverity::Info => SeverityTone::Info,
    }
}

/// The Fields section for a key subject. `slot` is the subject slot the
/// surface is bound to (#257) — the messages carry it home. `sp` is the
/// dock's resolved spacing grid (#192): a section spends it, it never
/// resolves one.
pub fn section(state: &FieldsState, slot: SlotId, sp: Spacing) -> Column<'_, Message> {
    let mut col = Column::new().spacing(sp.sm);

    let run_label = if state.in_flight {
        "observing…"
    } else {
        "observe fields"
    };
    let mut run = kit::action(kit::caption(run_label)).padding(sp.xs);
    if !state.in_flight {
        run = run.on_press(msg(slot, FieldsMsg::Run));
    }
    let window = kit::input("window (s)", &state.window)
        .on_input(move |t| msg(slot, FieldsMsg::WindowChanged(t)))
        .size(font::CAPTION)
        .width(iced::Length::Fixed(80.0));
    col = col.push(kit::section_header(
        "Fields",
        Some(
            row![run, window]
                .spacing(sp.sm)
                .align_y(iced::Alignment::Center)
                .into(),
        ),
    ));
    if state.window_secs().is_none() {
        col = col.push(kit::muted("window must be 1–300 seconds"));
    }

    let Some(report) = &state.report else {
        // Never-run is not "no fields" (O4).
        col = col.push(kit::muted(
            "no field observation yet — the button subscribes to exactly this \
             key for the window, then releases; nothing ambient",
        ));
        return col;
    };
    let report = match report {
        Ok(r) => r,
        Err(e) => {
            return col.push(kit::muted(format!("field observation failed: {e}")));
        }
    };

    // The coverage statement (O5/O6): the window, what it saw, and every
    // bound's cost.
    col = col.push(kit::muted(format!(
        "watched {} for {:.0}s: {} on {} · {} dropped · {} without a \
         structural document",
        report.selector,
        report.window_s,
        kit::plural(report.samples as usize, "sample"),
        kit::plural(report.keys_seen, "key"),
        report.dropped,
        report.undocumented,
    )));
    col = col.push(kit::muted(format!(
        "{} tracked (bound {}) · {} refused for the bound{}",
        kit::plural(report.paths, "path"),
        report.max_paths,
        report.paths_dropped,
        if report.paths_dropped_examples.is_empty() {
            String::new()
        } else {
            format!(" — e.g. {}", report.paths_dropped_examples.join(", "))
        },
    )));
    if !report.registry_loaded {
        col = col.push(kit::muted(
            "no registry loaded — declared ttl_s and types are unknown, so \
             field-stuck and field-new are unjudgeable here (O4), not clean",
        ));
    }

    for f in &report.findings {
        col = col.push(
            row![
                kit::badge_severity(severity_tone(f.severity), f.check.as_str()),
                kit::mono(f.subject.clone()),
            ]
            .spacing(sp.sm)
            .align_y(iced::Alignment::Center),
        );
        col = col.push(kit::muted(f.evidence.clone()));
    }

    let mut shown = 0usize;
    for r in &report.rows {
        if shown >= FIELD_ROWS {
            break;
        }
        shown += 1;
        let mut line = format!(
            "{} · seen {}/{} · {}",
            r.path,
            r.seen,
            r.documents,
            r.kinds.join("|"),
        );
        match r.last_change_s {
            Some(at) => {
                line.push_str(&format!(" · {} changes, last at {at:.1}s", r.changes));
            }
            None => line.push_str(" · unchanged in the window"),
        }
        if let (Some(min), Some(max), Some(last)) = (r.min, r.max, r.last) {
            line.push_str(&format!(" · min {min} · max {max} · last {last}"));
        }
        if let Some(values) = &r.values
            && !values.is_empty()
        {
            line.push_str(&format!(" · values {{{}}}", values.join(", ")));
        }
        col = col.push(kit::mono(line));
    }
    if report.rows.len() > shown {
        col = col.push(kit::muted(format!(
            "+{} more not shown (display bound)",
            report.rows.len() - shown
        )));
    }

    // Sparklines from the recorded history — a different window than the
    // observation's, and each caption's numbers are the ring's. Stated, so
    // the two windows cannot be read as one.
    if !state.sparks.is_empty() {
        col = col.push(kit::muted(
            "sparklines from the recorded history ring (its own window, not \
             the observation's)",
        ));
        for s in &state.sparks {
            col = col.push(crate::view::spark::chart(
                &s.path,
                &s.series,
                SeriesTone::Value,
                None,
                &s.cache,
                sp,
            ));
        }
    }
    col
}

/// Build the landing's sparklines: numeric report paths the subject's
/// recorded history can actually plot, bounded at `SPARK_ROWS`.
pub fn sparks_for(
    report: &FieldReport,
    history: Option<&crate::history::HistoryRecorder>,
) -> Vec<PathSpark> {
    let Some(rec) = history else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for row in &report.rows {
        if out.len() >= SPARK_ROWS {
            break;
        }
        if row.last.is_none() {
            continue;
        }
        let series = crate::series::value_series(&rec.ring, &row.path);
        if series.has_data() {
            out.push(PathSpark {
                path: row.path.clone(),
                series,
                cache: iced::widget::canvas::Cache::default(),
            });
        }
    }
    out
}
