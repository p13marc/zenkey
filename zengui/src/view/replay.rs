//! The replay surface (issue #74): an unmistakable banner, and the one
//! thing the CLI cannot do — a time scrubber. Since #217 the scrubber has
//! two sources: a `.zrec` file, and the monitor's retained window.
//!
//! Mode honesty is the whole design: while a `.zrec` feeds the panes, the
//! banner names the file, its selectors, its base, when it was captured
//! and what the capture dropped (a replay is a partial view and says so —
//! RFC 09 §5.1 O6), and the live link is off. A retained window owes a
//! different set of statements (#217): the budget in force, that the
//! window covers only **watched** keys — a retained window over three
//! watches presented as "the bus" is an O5 failure with a nicer UI — and
//! what retention itself evicted, as its own number beside stats-table
//! eviction and `Dropped(n)`, never folded into either (O6; v1.18 R1).
//! The scrubber's axis is the **capture clock** (each row's `t`, the
//! recording observer's arrival offsets) and the label says so, because a
//! consumer plotting a time axis states which clock it plotted
//! (RFC 09 §5.2).

use iced::widget::{row, text};
use iced::{Element, Length};
use zenkey_fleet::RetentionStats;

use super::kit;
use super::theme::colors;
use super::tokens::{Spacing, font, space};
use crate::message::{Message, WorkspaceMsg};
use crate::replay::{ReplaySource, ReplayState};
use crate::state::workspace::ReplayMode;

/// Replay-mode interactions.
#[derive(Debug, Clone, PartialEq)]
pub enum ReplayMsg {
    /// The path input changed (the open row's text box).
    PathChanged(String),
    /// Load the typed path (off the update thread, #255 — lands on
    /// [`ReplayMsg::Loaded`]).
    Open,
    /// The parse finished (#255): the path it ran against, and the loaded
    /// state or why not. Clears the loading claim either way.
    Loaded(
        String,
        Result<crate::replay::LoadedReplay, crate::services::ServiceError>,
    ),
    /// Show or hide the open row.
    OpenToggled,
    /// Play/pause.
    Toggled,
    /// The speed picker changed.
    SpeedSelected(Speed),
    /// The scrubber moved (an absolute instant on the capture clock, µs).
    Scrubbed(u64),
    /// Leave replay mode — the live link resumes.
    Exit,
    /// The play clock fired (subscription-driven while playing).
    Advance,
    /// Start or stop recording the current watches to a `.zrec`.
    RecordToggled,
    /// A recording finished (or failed): what it wrote — or why not.
    RecordFinished(Result<Recorded, crate::services::ServiceError>),
    /// Enter the retained window from live (#217) — or, from inside it,
    /// back to live. The live/retained toggle on the scrubber.
    RetainedToggled,
    /// Write the retained window to a `.zrec` through the ordinary writer
    /// (#217): the file is indistinguishable from a deliberate recording.
    /// Lands on [`ReplayMsg::RecordFinished`], like a capture.
    SaveWindow,
    /// Show or hide the snapshot open row (#219).
    SnapshotOpenToggled,
    /// The snapshot path input changed.
    SnapshotPathChanged(String),
    /// Load the typed `.zsnap` path (off the update thread — lands on
    /// [`ReplayMsg::SnapshotLoaded`]).
    SnapshotOpen,
    /// The `.zsnap` parse finished: the path it ran against, and the
    /// snapshot or why not. Clears the loading claim either way.
    SnapshotLoaded(
        String,
        Result<std::sync::Arc<zenkey_fleet::Snapshot>, crate::services::ServiceError>,
    ),
    /// Forget the loaded snapshot.
    SnapshotClosed,
}

/// What a finished capture wrote (#357).
///
/// `samples` and `dropped` are both `u64` and both count samples, and this
/// travels service → message → `WorkspaceState::recorded` → the Activity
/// replay tab. Positionally that is four chances to transpose the two and one
/// renderer that would happily say "recorded 3 sample(s) (17 dropped)".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    /// Samples written to the file.
    pub samples: u64,
    /// Samples the capture dropped, per the in-file ledger.
    pub dropped: u64,
    /// Where the file landed.
    pub path: String,
}

fn msg(m: ReplayMsg) -> Message {
    Message::Workspace(WorkspaceMsg::Replay(m))
}

/// A pacing scale the picker can display.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Speed(pub f64);

impl std::fmt::Display for Speed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x", self.0)
    }
}

/// The speeds on offer. A replay never runs unpaced — "as fast as
/// possible" is a bulk write, not a replay.
pub const SPEEDS: [Speed; 6] = [
    Speed(0.25),
    Speed(0.5),
    Speed(1.0),
    Speed(2.0),
    Speed(4.0),
    Speed(8.0),
];

/// The spelled-out age half of a retention budget: minutes when round,
/// seconds otherwise.
fn age_label(d: std::time::Duration) -> String {
    let s = d.as_secs();
    if s >= 60 && s.is_multiple_of(60) {
        format!("{} min", s / 60)
    } else {
        format!("{s}s")
    }
}

/// A retention budget, spelled the one way every surface states it (#217):
/// the banner, the dock and the status strip must not disagree about what
/// the bound is.
pub fn budget_label(b: zenkey_fleet::RetentionBudget) -> String {
    format!(
        "{} / {}",
        kit::human_bytes(b.max_bytes as u64),
        age_label(b.max_age)
    )
}

/// What the retained banner states (#217), split from the widget so the
/// honesty statements are testable as text: the measured span, the budget
/// in force, and that the window covers only **watched** keys — presenting
/// it as the bus would be an O5 failure with a nicer UI.
pub fn retained_meta(taken: &RetentionStats, rows: usize, span_secs: f64) -> String {
    format!(
        "the last {span_secs:.1}s of watched traffic — {} · budget {} · \
         watched keys only, not the bus",
        kit::plural(rows, "sample"),
        budget_label(taken.budget),
    )
}

/// The retention bound's own cost, as its own number (RFC 09 §5.1 O6;
/// v1.18 R1): shown when the **byte** budget bit, and never folded into
/// the stats-table eviction count or into `Dropped(n)`. `None` while the
/// window still holds everything its age claim promises.
pub fn retained_evicted_note(taken: &RetentionStats) -> Option<String> {
    (taken.evicted > 0).then(|| {
        format!(
            "{} evicted by the byte budget — the window is narrower than {} claims",
            kit::plural(taken.evicted as usize, "sample"),
            age_label(taken.budget.max_age),
        )
    })
}

/// What the banner says about a version-2 capture's preamble (#218;
/// RFC 13 §4.1): the rows that seed the fold, counted apart from observed
/// rows, and — when the file states a pre-roll — that it covers only the
/// watched selectors and how much of the asked-for window the ring could
/// give (O5/O6). `None` on a version-1 file, which has neither.
pub fn preamble_note(state: &ReplayState) -> Option<String> {
    let (ReplaySource::File { header, .. }, true) = (
        &state.source,
        state.preamble_rows > 0 || header_pre_roll(state).is_some(),
    ) else {
        return None;
    };
    let mut parts = Vec::new();
    if state.preamble_rows > 0 {
        parts.push(format!(
            "{} seed the state (a pane replay seeds its own fold; nothing is published)",
            kit::plural(state.preamble_rows as usize, "preamble row"),
        ));
    }
    if let Some(pre) = &header.pre_roll {
        parts.push(format!(
            "pre-roll {:.1}s of {:.1}s asked, watched selectors only",
            pre.covered_s, pre.asked_s
        ));
    }
    Some(parts.join(" · "))
}

fn header_pre_roll(state: &ReplayState) -> Option<&zenkey_fleet::PreRollInfo> {
    match &state.source {
        ReplaySource::File { header, .. } => header.pre_roll.as_ref(),
        ReplaySource::Retained { .. } => None,
    }
}

/// A trigger marker's label (#218): where on the capture clock a rule
/// fired, which rule, and to what.
pub fn trigger_label(t_us: u64, t: &zenkey_fleet::Transition) -> String {
    format!(
        "▲ {:.1}s {} → {}",
        t_us as f64 / 1e6,
        t.rule,
        serde_json::to_value(t.to)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default()
    )
}

/// The REPLAY/RETAINED banner. Rendered only in replay mode, directly
/// under the location bar — the panes below it are showing the window,
/// not the bus.
pub fn banner(state: &ReplayState) -> Element<'_, Message> {
    let (mode, what) = match &state.source {
        ReplaySource::File { path, header } => (
            "REPLAY",
            format!(
                "{} — {} under base {:?}, captured {} · {} row(s)",
                path,
                header.selectors.join(" + "),
                header.base,
                header.captured_at,
                state.rows.len(),
            ),
        ),
        ReplaySource::Retained { taken } => (
            "RETAINED",
            retained_meta(taken, state.rows.len(), state.span_us as f64 / 1e6),
        ),
    };
    // The mode indicator is the loudest claim on this row: EMPHASIS, in the
    // danger tone — everything else in the banner is metadata about it.
    let title = kit::emphasis(mode).style(|theme: &iced::Theme| text::Style {
        color: Some(colors(theme).danger()),
    });
    let mut meta = row![title, kit::muted(what)].spacing(space::SM);
    if let ReplaySource::Retained { taken } = &state.source
        && let Some(note) = retained_evicted_note(taken)
    {
        meta = meta.push(kit::caption(note).style(|theme: &iced::Theme| text::Style {
            color: Some(colors(theme).warning()),
        }));
    }
    if state.capture_dropped > 0 {
        meta = meta.push(
            kit::caption(format!(
                "capture dropped {} sample(s) — partial view",
                state.capture_dropped
            ))
            .style(|theme: &iced::Theme| text::Style {
                color: Some(colors(theme).warning()),
            }),
        );
    }
    if state.malformed > 0 {
        meta = meta.push(
            kit::caption(format!(
                "{} malformed row(s) skipped-and-counted",
                state.malformed
            ))
            .style(|theme: &iced::Theme| text::Style {
                color: Some(colors(theme).warning()),
            }),
        );
    }
    if let Some(note) = preamble_note(state) {
        meta = meta.push(kit::caption(note));
    }
    meta = meta.push(iced::widget::space::horizontal());
    meta = meta.push(kit::muted("live link off"));
    meta = meta.push(
        kit::action(kit::caption(match &state.source {
            ReplaySource::File { .. } => "exit replay",
            ReplaySource::Retained { .. } => "back to live",
        }))
        .on_press(msg(ReplayMsg::Exit))
        .padding(space::XS),
    );

    iced::widget::column![meta].spacing(space::XS).into()
}

/// The transport: play/pause, speed, and the scrubber.
///
/// Split from [`banner`] for the Activity dock (#183). The banner is a *mode*
/// indicator and stays between the location bar and the panes, where it cannot be
/// put away; the transport is a stream control and lives in the dock's Replay
/// tab.
pub fn scrubber(state: &ReplayState, sp: Spacing) -> Element<'_, Message> {
    let (pos, span) = state.clock();
    let mut transport = row![
        kit::action(kit::caption(if state.playing { "pause" } else { "play" }))
            .on_press(msg(ReplayMsg::Toggled))
            .padding(sp.xs),
        kit::picker(SPEEDS, Some(Speed(state.speed)), |s| msg(
            ReplayMsg::SpeedSelected(s)
        ))
        .text_size(font::CAPTION),
        kit::scrub(
            0.0..=(state.span_us.max(1) as f64 / 1e6),
            state.position_us as f64 / 1e6,
            |secs: f64| msg(ReplayMsg::Scrubbed((secs * 1e6) as u64)),
        )
        .step(0.1)
        .width(Length::Fill),
        // The axis, named: `t` is the capture clock — the recording
        // observer's arrival offsets — not the publishers' HLC.
        kit::muted(format!("{pos:.1}s / {span:.1}s (capture clock t)")),
    ]
    .spacing(sp.sm)
    .align_y(iced::Alignment::Center);
    // A retained window can become a file (#217): through the ordinary
    // writer, so the result is indistinguishable from a deliberate capture.
    if let ReplaySource::Retained { .. } = &state.source {
        transport = transport.push(
            kit::action(kit::caption("save window as .zrec"))
                .on_press(msg(ReplayMsg::SaveWindow))
                .padding(sp.xs),
        );
    }
    if state.triggers.is_empty() {
        return transport.into();
    }
    // The trigger markers (#218): one per record, on the capture clock,
    // each a jump to where the rule fired — the scrubber's axis has no
    // notion of a marker, so they ride under it, labelled.
    let mut markers = row![kit::muted("triggers")]
        .spacing(sp.sm)
        .align_y(iced::Alignment::Center);
    for (t_us, t) in &state.triggers {
        markers = markers.push(
            kit::action(kit::caption(trigger_label(*t_us, t)))
                .on_press(msg(ReplayMsg::Scrubbed(*t_us)))
                .padding(sp.xs),
        );
    }
    iced::widget::column![transport, markers]
        .spacing(sp.xs)
        .into()
}

/// What the replay tab says while a `.zrec` parses (#255). A pinned string,
/// like [`retained_meta`], because it is an honesty statement: a load in
/// flight is not an empty capture and not a hung window (RFC 09 §5.1 O4 —
/// "loading" is a state the UI must say, not leave to be inferred from
/// blankness).
pub fn loading_note(path: &str) -> String {
    format!("loading {path}\u{2026} — parsing the capture, the panes still show live")
}

/// The open row: a path box, shown on demand from the location bar.
pub fn open_row(path: &str, sp: Spacing) -> Element<'_, Message> {
    row![
        kit::caption("replay file"),
        kit::input(".zrec path", path)
            .on_input(|s| msg(ReplayMsg::PathChanged(s)))
            .on_submit(msg(ReplayMsg::Open))
            .size(font::CAPTION)
            .width(Length::Fill),
        kit::action(kit::caption("open"))
            .on_press(msg(ReplayMsg::Open))
            .padding(sp.xs),
        kit::action(kit::caption("cancel"))
            .on_press(msg(ReplayMsg::OpenToggled))
            .padding(sp.xs),
    ]
    .spacing(sp.sm)
    .align_y(iced::Alignment::Center)
    .into()
}

/// The snapshot open row (#219): a path box, shown on demand from the
/// Replay tab. The same shape as [`open_row`], because a `.zsnap` is the
/// `.zrec`'s sibling and is opened the same way.
pub fn snapshot_open_row(path: &str, sp: Spacing) -> Element<'_, Message> {
    row![
        kit::caption("snapshot file"),
        kit::input(".zsnap path", path)
            .on_input(|s| msg(ReplayMsg::SnapshotPathChanged(s)))
            .on_submit(msg(ReplayMsg::SnapshotOpen))
            .size(font::CAPTION)
            .width(Length::Fill),
        kit::action(kit::caption("open"))
            .on_press(msg(ReplayMsg::SnapshotOpen))
            .padding(sp.xs),
        kit::action(kit::caption("cancel"))
            .on_press(msg(ReplayMsg::SnapshotOpenToggled))
            .padding(sp.xs),
    ]
    .spacing(sp.sm)
    .align_y(iced::Alignment::Center)
    .into()
}

/// What the Replay tab says about a loaded snapshot (#219): its moment
/// **and its span** — RFC 13 §4.4's one non-negotiable — and what it holds.
pub fn snapshot_label(s: &zenkey_fleet::Snapshot) -> String {
    format!(
        "snapshot {} (over {:.2}s) — {} key(s) from {}{}",
        s.header.collected_at,
        s.header.collection_span_s,
        s.rows.len(),
        s.header.selectors.join(" + "),
        if s.header.roster.is_not_asked() {
            " · roster not asked: every holder unattributed"
        } else {
            ""
        },
    )
}

/// Everything replay mode puts between the location bar and the panes (#74).
///
/// Since #183 that is the banner and nothing else. The open row, the scrubber
/// and the capture line moved into the Activity dock's Replay tab, because
/// they are stream controls. The banner did not, because it is a *mode*
/// indicator, and one you can put away behind a tab is one that can lie about
/// what the panes are showing.
///
/// A `Vec` rather than one composed element: the shell stacks them into its
/// own column, and it is empty on the ordinary path.
pub(crate) fn surfaces(replay: &ReplayMode) -> Vec<Element<'_, Message>> {
    let mut out: Vec<Element<'_, Message>> = Vec::new();
    if let Some(state) = &replay.replay {
        out.push(banner(state));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use zenkey_fleet::RetentionBudget;

    fn stats(evicted: u64) -> RetentionStats {
        RetentionStats {
            budget: RetentionBudget::default(),
            retained: 42,
            retained_bytes: 1024,
            span: Duration::from_secs(90),
            evicted,
            expired: 7,
        }
    }

    /// #217's three banner statements, as text: the budget, the measured
    /// span, and that the window covers watched keys only — a retained
    /// window presented as "the bus" is an O5 failure with a nicer UI.
    #[test]
    fn the_retained_banner_states_budget_and_coverage() {
        let meta = retained_meta(&stats(0), 42, 90.0);
        assert!(meta.contains("MiB"), "{meta}");
        assert!(meta.contains("2 min"), "{meta}");
        assert!(meta.contains("90.0s"), "{meta}");
        assert!(meta.contains("watched keys only"), "{meta}");
        assert!(meta.contains("not the bus"), "{meta}");
    }

    /// Retention eviction is its own number (RFC 09 §5.1 O6; v1.18 R1):
    /// silent while the bound has not bitten, and named as *this* bound's
    /// cost when it has — never a shared figure with the stats table or
    /// the broadcast.
    #[test]
    fn retention_eviction_speaks_only_when_it_bit_and_names_its_bound() {
        assert!(retained_evicted_note(&stats(0)).is_none());
        let note = retained_evicted_note(&stats(9)).unwrap();
        assert!(note.contains("9 samples"), "{note}");
        assert!(note.contains("byte budget"), "{note}");
        assert!(note.contains("narrower"), "{note}");
    }

    /// #255's O4 statement, as text: a parse in flight names its file and
    /// says the panes are still live — stated, never inferred from a blank
    /// tab.
    #[test]
    fn the_loading_note_names_the_file_and_what_the_panes_show() {
        let note = loading_note("cap.zrec");
        assert!(note.starts_with("loading cap.zrec"), "{note}");
        assert!(note.contains("still show live"), "{note}");
    }

    /// A version-2 capture (#218) loads with its preamble seeding the fold
    /// and its trigger kept: `scrub_to(0)` already holds the preamble key
    /// (a pane replay seeds its own fold — nothing is published), the
    /// trigger sits at the last pre-roll row's instant, and the banner says
    /// how many rows seed and what the pre-roll covers.
    #[test]
    fn a_version_two_capture_seeds_the_fold_and_keeps_its_trigger() {
        let body = [
            r#"{"zrec":2,"selectors":["v1/**"],"base":"","captured_at":"x","preamble":{"count":1,"collected_over_s":0.1,"selectors":["v1/*/state/**"],"semantics":"absent_from_window"},"pre_roll":{"asked_s":30.0,"covered_s":1.0,"watched":["v1/**"]}}"#,
            r#"{"key":"v1/h-0123456789ab/state/p/config","t":0,"preamble":true,"bytes":"MQ=="}"#,
            r#"{"key":"v1/h-0123456789ab/state/p/health","t":0,"bytes":"Mg=="}"#,
            r#"{"key":"v1/h-0123456789ab/state/p/health","t":1000000,"bytes":"Mw=="}"#,
            r#"{"trigger":{"rule":"silent-for v1/h-0123456789ab/state/p/health 0.7","from":"ok","to":"firing","at":"x","evidence":"e"}}"#,
            r#"{"key":"v1/h-0123456789ab/state/p/health","t":2000000,"bytes":"NA=="}"#,
        ]
        .join("\n");
        let mut state = ReplayState::load("t.zrec", body.as_bytes()).unwrap();
        assert_eq!(state.preamble_rows, 1);
        assert_eq!(state.rows.len(), 4, "the preamble row is a row to the fold");
        assert_eq!(state.triggers.len(), 1);
        assert_eq!(state.triggers[0].0, 1_000_000, "at the last pre-roll row");

        let at_start = state.scrub_to(0);
        // Walk the tree by chunk: the preamble key is a leaf at t=0.
        let node = ["v1", "h-0123456789ab", "state", "p", "config"]
            .iter()
            .try_fold(&at_start.tree.root, |n, chunk| n.children.get(*chunk));
        assert!(
            node.is_some_and(|n| n.count == 1),
            "the preamble key is in the tree at t=0: {:?}",
            at_start.tree.root.children.keys().collect::<Vec<_>>()
        );

        let note = preamble_note(&state).unwrap();
        assert!(note.contains("1 preamble row"), "{note}");
        assert!(note.contains("nothing is published"), "{note}");
        assert!(note.contains("1.0s of 30.0s"), "{note}");
        assert!(note.contains("watched selectors only"), "{note}");
        let label = trigger_label(state.triggers[0].0, &state.triggers[0].1);
        assert!(
            label.contains("1.0s") && label.contains("silent-for") && label.ends_with("firing"),
            "{label}"
        );
    }

    /// One spelling of the budget for every surface.
    #[test]
    fn the_budget_label_is_shared_and_readable() {
        assert_eq!(
            budget_label(RetentionBudget::default()),
            format!("{} / 2 min", kit::human_bytes(64 * 1024 * 1024))
        );
        let odd = RetentionBudget {
            max_bytes: 1024,
            max_age: Duration::from_secs(90),
        };
        assert!(budget_label(odd).ends_with("/ 90s"));
    }
}
