//! Persisted UI preferences (issue #73) — what the window remembers.
//!
//! Every launch used to start from scratch: theme hardcoded Dark, no zoom,
//! geometry forgotten, scope and context reset. Both siblings in this family
//! already persist (zensight: JSON5 + env overrides; tcgui: zoom clamped on
//! load), so the pattern is proven — this is the zengui shape of it.
//!
//! **What belongs here and what does not.** Preferences are how the *window*
//! is set up; a [`crate::config::Settings`] is what the *session* connects to.
//! The overlap — context, base, scope — is stored as "what was on screen last
//! time", and a command-line flag always wins: a user who typed `--base acme`
//! meant it, and a remembered base silently overriding it would be the worst
//! kind of persistence.
//!
//! **A user-editable file cannot be trusted to parse.** A malformed or
//! half-written prefs file degrades to defaults with a note, never a crash —
//! the same posture the context store takes, for the same reason: this file
//! lives where a human can open it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::scope::ScopePreset;

/// Zoom bounds. Below ~0.5 the UI is unreadable and above ~2.5 nothing fits;
/// a stored value outside the range is clamped rather than rejected, because
/// the file is hand-editable and a typo should not empty the window.
pub const MIN_ZOOM: f32 = 0.6;
pub const MAX_ZOOM: f32 = 2.5;
const ZOOM_STEP: f32 = 0.1;

/// Which theme to render in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    Light,
    #[default]
    Dark,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 2] = [ThemeChoice::Dark, ThemeChoice::Light];

    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
        }
    }

    pub fn theme(self) -> iced::Theme {
        match self {
            ThemeChoice::Light => iced::Theme::Light,
            ThemeChoice::Dark => iced::Theme::Dark,
        }
    }

    /// The other one — what a toggle switches to.
    pub fn toggled(self) -> ThemeChoice {
        match self {
            ThemeChoice::Light => ThemeChoice::Dark,
            ThemeChoice::Dark => ThemeChoice::Light,
        }
    }
}

/// How tightly the workspace packs (#192): a multiplier on the spacing grid
/// and the virtualized row heights — **never** on a font size. Shrinking type
/// is not density, it is illegibility, and it would undo the type-scale work
/// (#191) in one keystroke.
///
/// The stored value is the *global* mode, toggled by Ctrl+Shift+D. What a
/// dock actually renders at is [`DockRole::density`]: Compact takes the whole
/// window dense; Comfortable restores each dock's own default — which for the
/// Locator is Compact, because a tree wants rows however airy the rest is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    #[default]
    Comfortable,
    Compact,
}

impl Density {
    pub const ALL: [Density; 2] = [Density::Comfortable, Density::Compact];

    pub fn label(self) -> &'static str {
        match self {
            Density::Comfortable => "comfortable",
            Density::Compact => "compact",
        }
    }

    /// The other one — what Ctrl+Shift+D switches to.
    pub fn toggled(self) -> Density {
        match self {
            Density::Comfortable => Density::Compact,
            Density::Compact => Density::Comfortable,
        }
    }
}

/// The four dock roles the workspace grid arranges (#180, epic #172).
///
/// A role, not a pane: the Workbench shows whichever tool `right_pane`
/// selects, and the Activity dock holds its own tab strip. The grid decides
/// *where* each region is and how much of the window it gets — never what is
/// inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DockRole {
    /// The key tree, find and pivot — the left dock of every preset.
    Locator,
    /// The one surface that follows the subject (#182).
    Inspector,
    /// The session's parallel streams (#183): echo, publish log, doctor,
    /// replay.
    Activity,
    /// The tools: Send, nodes, admin — #184 merged call and publish into the
    /// one Send form.
    Workbench,
}

impl DockRole {
    pub const ALL: [DockRole; 4] = [
        DockRole::Locator,
        DockRole::Inspector,
        DockRole::Activity,
        DockRole::Workbench,
    ];

    pub fn label(self) -> &'static str {
        match self {
            DockRole::Locator => "locator",
            DockRole::Inspector => "inspector",
            DockRole::Activity => "activity",
            DockRole::Workbench => "workbench",
        }
    }

    /// What the dock renders at under `global` (#192).
    ///
    /// Compact is a floor, not a suggestion: the global toggle takes every
    /// dock dense. Comfortable gives each dock its own default — Compact in
    /// the Locator (a 40,000-key tree wants rows), Comfortable everywhere a
    /// form wants air. That is why Ctrl+Shift+D visibly moves the Inspector
    /// and leaves the Locator alone: the tree was already as dense as the
    /// grid goes.
    pub fn density(self, global: Density) -> Density {
        match (global, self) {
            (Density::Compact, _) => Density::Compact,
            (Density::Comfortable, DockRole::Locator) => Density::Compact,
            (Density::Comfortable, _) => Density::Comfortable,
        }
    }
}

/// A split's direction, as persisted. Mirrors `pane_grid::Axis`, and spelled
/// out here so the prefs file never depends on a widget crate's serde story:
/// `horizontal` is a horizontal split *line* — `a` above `b`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutAxis {
    /// `a` over `b`.
    Horizontal,
    /// `a` left of `b`.
    Vertical,
}

/// The persisted shape of the workspace grid (#180): the same binary tree
/// `pane_grid::State` keeps, minus the widget-internal ids — which is exactly
/// the part that cannot be serialized and does not need to be. The grid is
/// rebuilt from this on launch and captured back into it on every layout
/// change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutNode {
    /// A split of the available space, `ratio` to `a`.
    Split {
        axis: LayoutAxis,
        ratio: f32,
        a: Box<LayoutNode>,
        b: Box<LayoutNode>,
    },
    /// A dock.
    Dock(DockRole),
}

impl LayoutNode {
    fn collect(&self, out: &mut Vec<DockRole>) {
        match self {
            LayoutNode::Split { a, b, .. } => {
                a.collect(out);
                b.collect(out);
            }
            LayoutNode::Dock(role) => out.push(*role),
        }
    }

    /// The docks this layout shows, in tree order.
    pub fn roles(&self) -> Vec<DockRole> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    /// A hand-editable tree can say anything; a *sane* one names each dock at
    /// most once. (At least one is structural: a `LayoutNode` cannot be
    /// empty.) A duplicate would make "close the locator" ambiguous, so the
    /// whole layout degrades to the default rather than guessing.
    pub fn is_sane(&self) -> bool {
        let mut roles = self.roles();
        roles.sort_by_key(|r| r.label());
        let len = roles.len();
        roles.dedup();
        roles.len() == len
    }

    /// Clamp every ratio a hand edit may have pushed out of range. The bounds
    /// are looser than the old `split`'s 0.15..0.85 because a grid pane has a
    /// minimum pixel size of its own; the clamp only has to keep a dock from
    /// vanishing entirely.
    fn clamped(self) -> LayoutNode {
        match self {
            LayoutNode::Split { axis, ratio, a, b } => LayoutNode::Split {
                axis,
                ratio: if ratio.is_finite() {
                    ratio.clamp(0.05, 0.95)
                } else {
                    0.5
                },
                a: Box::new(a.clamped()),
                b: Box::new(b.clamped()),
            },
            dock => dock,
        }
    }
}

/// A dock torn off into its own window (#186), as the named layout keeps it:
/// the role, and the geometry its window had. The list this sits in is the
/// layout's other half — [`WorkspaceLayout::root`] says where the docked
/// regions are, `torn` says which regions left the grid for a window of
/// their own, so a restart rebuilds both.
///
/// Geometry is remembered only while the dock *is* torn: closing the window
/// re-docks the role and drops the entry, the same way closing a dock in the
/// grid forgets the splits that held it (#180).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TornDock {
    pub role: DockRole,
    /// Window size in logical pixels, once a resize reported one.
    #[serde(default)]
    pub size: Option<(f32, f32)>,
    /// Window position, when the platform reports one (Wayland never does,
    /// so `None` stays the honest common case).
    #[serde(default)]
    pub position: Option<(f32, f32)>,
}

/// The smallest torn-off window worth restoring: below this a stored size is
/// a hand edit, and it is dropped rather than kept — the same posture as the
/// main window's 320×240 floor.
const MIN_TORN: (f32, f32) = (200.0, 120.0);

impl TornDock {
    /// Field-by-field sanity, like the zoom clamp: a silly size or position
    /// is dropped, never a reason to refuse the entry beside it.
    fn sanitised(mut self) -> TornDock {
        self.size = self.size.filter(|(w, h)| {
            w.is_finite() && h.is_finite() && *w >= MIN_TORN.0 && *h >= MIN_TORN.1
        });
        self.position = self
            .position
            .filter(|(x, y)| x.is_finite() && y.is_finite());
        self
    }
}

/// The three saved layouts (epic #172), on Alt+1/2/3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutPreset {
    /// Locator 30% + Inspector 70% — reading one bus, one key at a time.
    Explore,
    /// Locator + Inspector over a tall Activity dock (40% height) — watching
    /// the streams while the subject stays on screen.
    Watch,
    /// Like Watch, but the Locator pivots by origin and the Activity dock
    /// opens on the doctor — "why is this node silent?".
    Diagnose,
}

impl LayoutPreset {
    pub const ALL: [LayoutPreset; 3] = [
        LayoutPreset::Explore,
        LayoutPreset::Watch,
        LayoutPreset::Diagnose,
    ];

    pub fn label(self) -> &'static str {
        match self {
            LayoutPreset::Explore => "explore",
            LayoutPreset::Watch => "watch",
            LayoutPreset::Diagnose => "diagnose",
        }
    }

    /// The preset's tree. The Workbench is deliberately in none of them: the
    /// send/nodes/admin tools are opened when wanted (the dock strip, the
    /// palette, or any `PaneSelected`), not paid for by default.
    pub fn root(self) -> LayoutNode {
        let dock = |r| Box::new(LayoutNode::Dock(r));
        let split = |axis, ratio, a, b| LayoutNode::Split { axis, ratio, a, b };
        match self {
            LayoutPreset::Explore => split(
                LayoutAxis::Vertical,
                0.30,
                dock(DockRole::Locator),
                dock(DockRole::Inspector),
            ),
            // Locator 25% + Inspector 35% of the window, Activity 40% height:
            // a 60% top row split 25:35, over a full-width Activity dock.
            LayoutPreset::Watch => split(
                LayoutAxis::Horizontal,
                0.60,
                Box::new(split(
                    LayoutAxis::Vertical,
                    0.42,
                    dock(DockRole::Locator),
                    dock(DockRole::Inspector),
                )),
                dock(DockRole::Activity),
            ),
            // Locator 30% + Inspector 35%, Activity 35% height.
            LayoutPreset::Diagnose => split(
                LayoutAxis::Horizontal,
                0.65,
                Box::new(split(
                    LayoutAxis::Vertical,
                    0.46,
                    dock(DockRole::Locator),
                    dock(DockRole::Inspector),
                )),
                dock(DockRole::Activity),
            ),
        }
    }

    pub fn layout(self) -> WorkspaceLayout {
        WorkspaceLayout {
            preset: Some(self),
            root: self.root(),
            // A preset is one of the three named arrangements, and all three
            // are fully docked: applying one re-docks every torn window.
            torn: Vec::new(),
        }
    }
}

/// The named workspace layout (#180) — what superseded the scalar `split`,
/// which was stored, clamped, round-trip tested and read by nothing.
///
/// `preset` is the name while the layout still *is* that preset; the first
/// drag, close or restore clears it, because a layout the user has bent is no
/// longer Explore however it started.
///
/// The defaults are per-field, **not** the struct-level `#[serde(default)]`
/// the rest of the file uses: that would fill a missing `preset` from
/// `Default::default()` — `Some(Explore)` — and a custom tree saved without a
/// name would load renamed Explore and be regenerated out of existence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceLayout {
    /// Absent means custom, not "the default preset".
    #[serde(default)]
    pub preset: Option<LayoutPreset>,
    /// Absent (a hand-written `preset = "…"`-only table) is filled from the
    /// preset by `sanitised`, which regenerates a named layout's tree anyway.
    #[serde(default = "explore_root")]
    pub root: LayoutNode,
    /// Docks torn off into windows of their own (#186), with the geometry
    /// each window had — a restart reopens them. Absent in an older file, so
    /// it defaults to the fully-docked reading that file was written under.
    #[serde(default)]
    pub torn: Vec<TornDock>,
}

fn explore_root() -> LayoutNode {
    LayoutPreset::Explore.root()
}

impl Default for WorkspaceLayout {
    fn default() -> Self {
        LayoutPreset::Explore.layout()
    }
}

impl WorkspaceLayout {
    /// A layout that is no preset any more, fully docked.
    pub fn custom(root: LayoutNode) -> WorkspaceLayout {
        WorkspaceLayout {
            preset: None,
            root,
            torn: Vec::new(),
        }
    }

    /// The torn entries a hand-edited file cannot be trusted to keep sane:
    /// the Locator is never torn (it *is* the navigation, #186), a role
    /// cannot be both docked and torn, and no role is torn twice. Dropped
    /// entry by entry, like an invalid selector — never a refused launch.
    fn sanitise_torn(&mut self) {
        let docked = self.root.roles();
        let mut seen: Vec<DockRole> = Vec::new();
        let torn = std::mem::take(&mut self.torn);
        self.torn = torn
            .into_iter()
            .filter(|t| {
                let keep = t.role != DockRole::Locator
                    && !docked.contains(&t.role)
                    && !seen.contains(&t.role);
                if keep {
                    seen.push(t.role);
                }
                keep
            })
            .map(TornDock::sanitised)
            .collect();
    }
}

/// The persisted document.
///
/// Every field is `#[serde(default)]`-shaped so a file written by an older
/// build, or hand-edited down to two lines, still loads — the same
/// forward/backward tolerance the slice parser has, for the same reason.
/// (That is also how the retired `split` key ages out: an old file's value is
/// simply not a field any more, and is ignored.)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub theme: ThemeChoice,
    /// UI scale factor, clamped to [`MIN_ZOOM`]..=[`MAX_ZOOM`] on load.
    pub zoom: f32,
    /// Window size in logical pixels, when one was recorded.
    pub window: Option<(f32, f32)>,
    /// The context selected last time (`None` = whatever the config's
    /// `current` pointer says).
    pub context: Option<String>,
    /// The scope preset last selected.
    pub scope: ScopePreset,
    /// The custom selectors last applied (#187) — what makes a remembered
    /// `custom` scope restorable rather than dropped. Kept even while a
    /// preset is selected, so switching back to custom recovers them.
    pub selectors: Vec<String>,
    /// The workspace grid (#180), superseding the scalar `split`.
    pub layout: WorkspaceLayout,
    /// How many echo lines to retain (#188). `None` until set in the
    /// Settings overlay: a command-line flag is a one-launch choice and is
    /// deliberately not written back here — only the overlay's apply is,
    /// so a flag never silently becomes the new default.
    pub echo_lines: Option<usize>,
    /// How many history entries the selected key's recorder retains (#188).
    pub history_entries: Option<usize>,
    /// How many distinct keys the monitor tracks statistics for (#188).
    /// Applied on the next (re)connect, and remembered here so the raise
    /// survives the restart it takes effect through.
    pub max_keys: Option<usize>,
    /// Whether to observe the scope immediately on connect (#188).
    pub eager: Option<bool>,
    /// The global density mode (#192), toggled by Ctrl+Shift+D.
    pub density: Density,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            theme: ThemeChoice::default(),
            zoom: 1.0,
            window: None,
            context: None,
            scope: ScopePreset::Everything,
            selectors: Vec::new(),
            layout: WorkspaceLayout::default(),
            echo_lines: None,
            history_entries: None,
            max_keys: None,
            eager: None,
            density: Density::default(),
        }
    }
}

impl Prefs {
    /// Where the file lives: beside the shared context config, so one
    /// directory holds everything an explorer remembers.
    pub fn path() -> PathBuf {
        let config = zenkey_explorer_config::config_path();
        config
            .parent()
            .map(|d| d.join("zengui.toml"))
            .unwrap_or_else(|| PathBuf::from("zengui.toml"))
    }

    /// Load, or explain in the returned note why the defaults are in force.
    ///
    /// Never `Err`: a GUI that refuses to open because its preferences file is
    /// broken has turned a cosmetic problem into an outage.
    pub fn load() -> (Prefs, Option<String>) {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &std::path::Path) -> (Prefs, Option<String>) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            // Absent is the normal first-run case, and says nothing.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Prefs::default(), None),
            Err(e) => {
                return (
                    Prefs::default(),
                    Some(format!(
                        "{} unreadable ({e}) — using defaults",
                        path.display()
                    )),
                );
            }
        };
        match toml::from_str::<Prefs>(&text) {
            Ok(prefs) => (prefs.sanitised(), None),
            Err(e) => (
                Prefs::default(),
                Some(format!(
                    "{} does not parse ({e}) — using defaults; the file is left as it is",
                    path.display()
                )),
            ),
        }
    }

    /// Clamp what a hand-edited file may have put out of range. Clamping, not
    /// rejecting: a typo in `zoom` should not discard the theme next to it.
    fn sanitised(mut self) -> Prefs {
        self.zoom = self.zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        if !self.zoom.is_finite() {
            self.zoom = 1.0;
        }
        // A named preset regenerates its tree: the name is the claim, and a
        // hand-edited `preset = "watch"` should *be* Watch rather than
        // whatever tree happened to sit beside it.
        self.layout = match self.layout.preset {
            Some(preset) => preset.layout(),
            None if self.layout.root.is_sane() => {
                let mut layout = WorkspaceLayout {
                    preset: None,
                    root: self.layout.root.clamped(),
                    torn: self.layout.torn,
                };
                layout.sanitise_torn();
                layout
            }
            // A tree naming one dock twice cannot be closed or restored
            // coherently — degrade to the default, like a broken file does.
            None => WorkspaceLayout::default(),
        };
        self.window = self
            .window
            .filter(|(w, h)| w.is_finite() && h.is_finite() && *w >= 320.0 && *h >= 240.0);
        // A hand-edited selector that no longer validates (empty, `$*`, not a
        // key expression) is dropped rather than allowed to refuse the next
        // launch — the same field-by-field posture as the zoom clamp. What
        // that leaves of a remembered custom scope is `config.rs`'s question.
        self.selectors
            .retain(|s| crate::scope::validate_selector(s).is_ok());
        // A remembered zero bound is a hand edit, not a choice the overlay
        // can make (the boundary rejects zeros): drop it to unset rather
        // than refuse the launch a typed `--echo-lines 0` rightly refuses.
        self.echo_lines = self.echo_lines.filter(|n| *n > 0);
        self.history_entries = self.history_entries.filter(|n| *n > 0);
        self.max_keys = self.max_keys.filter(|n| *n > 0);
        self
    }

    /// Persist. The only caller outside tests is [`crate::services::prefs`]
    /// (#255): the write runs as a task, never on the update thread, and
    /// stays best-effort — its failure is noted, not raised.
    pub fn save_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, text)
    }

    pub fn zoom_in(&mut self) {
        self.zoom = (self.zoom + ZOOM_STEP).min(MAX_ZOOM);
    }

    pub fn zoom_out(&mut self) {
        self.zoom = (self.zoom - ZOOM_STEP).max(MIN_ZOOM);
    }

    pub fn zoom_reset(&mut self) {
        self.zoom = 1.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("zengui-prefs-tests");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn a_round_trip_preserves_every_field() {
        let path = tmp("round-trip.toml");
        let prefs = Prefs {
            theme: ThemeChoice::Light,
            zoom: 1.3,
            window: Some((1440.0, 900.0)),
            context: Some("lab".into()),
            scope: ScopePreset::Deployment,
            selectors: vec!["demo/**".into(), "v1/*/state/**".into()],
            layout: LayoutPreset::Watch.layout(),
            echo_lines: Some(5000),
            history_entries: Some(400),
            max_keys: Some(100_000),
            eager: Some(true),
            density: Density::Compact,
        };
        prefs.save_to(&path).unwrap();
        let (back, note) = Prefs::load_from(&path);
        assert_eq!(back, prefs);
        assert!(note.is_none());
    }

    /// The layout the acceptance is about: a *dragged* (custom) tree — nested
    /// splits, odd ratios — comes back exactly, which is the half of "a drag
    /// survives restart" that the file is responsible for.
    #[test]
    fn a_custom_layout_tree_round_trips_exactly() {
        let path = tmp("layout-round-trip.toml");
        let root = LayoutNode::Split {
            axis: LayoutAxis::Horizontal,
            ratio: 0.42,
            a: Box::new(LayoutNode::Split {
                axis: LayoutAxis::Vertical,
                ratio: 0.27,
                a: Box::new(LayoutNode::Dock(DockRole::Locator)),
                b: Box::new(LayoutNode::Dock(DockRole::Workbench)),
            }),
            b: Box::new(LayoutNode::Dock(DockRole::Activity)),
        };
        let prefs = Prefs {
            layout: WorkspaceLayout::custom(root.clone()),
            ..Prefs::default()
        };
        prefs.save_to(&path).unwrap();
        let (back, note) = Prefs::load_from(&path);
        assert!(note.is_none());
        assert_eq!(back.layout.preset, None, "a bent layout is no preset");
        assert_eq!(back.layout.root, root);
    }

    /// First run is silent: an absent file is the normal case, not a problem
    /// worth telling anybody about.
    #[test]
    fn an_absent_file_is_not_a_note() {
        let (prefs, note) = Prefs::load_from(&tmp("definitely-not-written.toml"));
        assert_eq!(prefs, Prefs::default());
        assert!(note.is_none());
    }

    /// A broken file degrades to defaults **with** a note, and is left on disk
    /// — overwriting a user's hand-edited file because we could not read it
    /// would destroy the thing they were trying to fix.
    #[test]
    fn a_malformed_file_degrades_loudly_and_is_not_clobbered() {
        let path = tmp("broken.toml");
        std::fs::write(&path, "this is not = = toml").unwrap();
        let (prefs, note) = Prefs::load_from(&path);
        assert_eq!(prefs, Prefs::default());
        let note = note.expect("a broken file must say so");
        assert!(note.contains("does not parse"), "{note}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "this is not = = toml",
            "the file must be left as the user wrote it"
        );
    }

    /// A partial file loads: an older build's prefs, or a two-line hand edit.
    #[test]
    fn a_partial_file_fills_the_rest_from_defaults() {
        let path = tmp("partial.toml");
        std::fs::write(&path, "theme = \"light\"\n").unwrap();
        let (prefs, note) = Prefs::load_from(&path);
        assert!(note.is_none());
        assert_eq!(prefs.theme, ThemeChoice::Light);
        assert_eq!(prefs.zoom, 1.0, "the rest is the default");
    }

    /// Out-of-range values are clamped, not rejected — a bad `zoom` must not
    /// discard the `theme` sitting next to it. The retired `split` key (an
    /// older build's file) is ignored rather than a parse error.
    #[test]
    fn hand_edited_nonsense_is_clamped_field_by_field() {
        let path = tmp("nonsense.toml");
        std::fs::write(
            &path,
            "theme = \"light\"\nzoom = 99.0\nsplit = -3.0\nwindow = [1.0, 1.0]\n\
             [layout]\n[layout.root.split]\naxis = \"vertical\"\nratio = 7.0\n\
             [layout.root.split.a]\ndock = \"locator\"\n\
             [layout.root.split.b]\ndock = \"inspector\"\n",
        )
        .unwrap();
        let (prefs, note) = Prefs::load_from(&path);
        assert!(note.is_none(), "it parsed; the values were just silly");
        assert_eq!(prefs.theme, ThemeChoice::Light, "the good field survives");
        assert_eq!(prefs.zoom, MAX_ZOOM);
        assert!(prefs.window.is_none(), "a 1x1 window is dropped, not kept");
        let LayoutNode::Split { ratio, .. } = prefs.layout.root else {
            panic!("the edited tree survives, clamped");
        };
        assert_eq!(ratio, 0.95, "a ratio of 7.0 is a clamp, not a rejection");
    }

    /// A layout naming one dock twice cannot be closed or restored
    /// coherently, so the whole layout — and only the layout — degrades.
    #[test]
    fn a_duplicate_dock_degrades_the_layout_to_the_default() {
        let path = tmp("duplicate-dock.toml");
        std::fs::write(
            &path,
            "theme = \"light\"\n\
             [layout]\n[layout.root.split]\naxis = \"vertical\"\nratio = 0.5\n\
             [layout.root.split.a]\ndock = \"locator\"\n\
             [layout.root.split.b]\ndock = \"locator\"\n",
        )
        .unwrap();
        let (prefs, _) = Prefs::load_from(&path);
        assert_eq!(prefs.theme, ThemeChoice::Light, "the good field survives");
        assert_eq!(prefs.layout, WorkspaceLayout::default());
    }

    /// `preset = "watch"` *is* Watch: the name regenerates the tree, so a
    /// hand-edited name never ships with a stale tree beside it.
    #[test]
    fn a_named_preset_regenerates_its_tree_on_load() {
        let path = tmp("named-preset.toml");
        std::fs::write(&path, "[layout]\npreset = \"watch\"\n").unwrap();
        let (prefs, note) = Prefs::load_from(&path);
        assert!(note.is_none());
        assert_eq!(prefs.layout, LayoutPreset::Watch.layout());
    }

    /// A remembered selector that no longer validates is dropped on load,
    /// field by field (#187) — like the zoom clamp, a typo in one row must
    /// not discard the valid rows beside it, and must never refuse a launch.
    #[test]
    fn an_invalid_remembered_selector_is_dropped_not_fatal() {
        let path = tmp("bad-selector.toml");
        std::fs::write(
            &path,
            "scope = \"custom\"\nselectors = [\"demo/**\", \"demo/$*/x\", \"\"]\n",
        )
        .unwrap();
        let (prefs, note) = Prefs::load_from(&path);
        assert!(note.is_none(), "it parsed; one row was just wrong");
        assert_eq!(prefs.scope, ScopePreset::Custom);
        assert_eq!(
            prefs.selectors,
            ["demo/**"],
            "the `$*` row (RFC 03 §2) and the empty row are dropped"
        );
    }

    /// A hand-edited zero bound unsets rather than strands (#188): the CLI
    /// boundary rejects a *typed* zero, but a stale file must never refuse
    /// the launch — same soft/hard split as the selectors above.
    #[test]
    fn a_remembered_zero_bound_is_dropped_not_fatal() {
        let path = tmp("zero-bounds.toml");
        std::fs::write(&path, "echo_lines = 0\nmax_keys = 40000\n").unwrap();
        let (prefs, note) = Prefs::load_from(&path);
        assert!(note.is_none());
        assert_eq!(prefs.echo_lines, None, "zero unsets");
        assert_eq!(prefs.max_keys, Some(40_000), "the good field survives");
    }

    #[test]
    fn zoom_steps_stay_inside_the_bounds() {
        let mut p = Prefs::default();
        for _ in 0..100 {
            p.zoom_in();
        }
        assert_eq!(p.zoom, MAX_ZOOM);
        for _ in 0..100 {
            p.zoom_out();
        }
        assert_eq!(p.zoom, MIN_ZOOM);
        p.zoom_reset();
        assert_eq!(p.zoom, 1.0);
    }

    #[test]
    fn the_theme_toggle_is_an_involution() {
        for t in ThemeChoice::ALL {
            assert_eq!(t.toggled().toggled(), t);
        }
    }

    #[test]
    fn the_density_toggle_is_an_involution() {
        for d in Density::ALL {
            assert_eq!(d.toggled().toggled(), d);
        }
    }

    /// The acceptance's persistence half (#192): the toggled mode is a field
    /// like any other, so the round-trip test above carries it — this one
    /// pins the *file spelling*, because a hand-editable file is an API.
    #[test]
    fn density_survives_restart_as_a_plain_word() {
        let path = tmp("density.toml");
        let prefs = Prefs {
            density: Density::Compact,
            ..Prefs::default()
        };
        prefs.save_to(&path).unwrap();
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("density = \"compact\""),
        );
        let (back, note) = Prefs::load_from(&path);
        assert!(note.is_none());
        assert_eq!(back.density, Density::Compact);
    }

    /// Global Compact takes every dock dense; global Comfortable restores
    /// each dock's own default — and the Locator's default *is* Compact,
    /// because a tree wants rows (#192).
    #[test]
    fn compact_is_a_floor_and_the_locator_never_rises_above_it() {
        for role in DockRole::ALL {
            assert_eq!(role.density(Density::Compact), Density::Compact);
        }
        assert_eq!(
            DockRole::Locator.density(Density::Comfortable),
            Density::Compact,
            "the locator's default is Compact"
        );
        for role in [DockRole::Inspector, DockRole::Activity, DockRole::Workbench] {
            assert_eq!(role.density(Density::Comfortable), Density::Comfortable);
        }
    }

    /// A torn-off dock rides the named layout (#186): the role and its
    /// window's geometry round-trip, and the file spelling is pinned — a
    /// hand-editable file is an API.
    #[test]
    fn a_torn_dock_rides_the_layout_round_trip() {
        let path = tmp("torn-round-trip.toml");
        let mut layout = WorkspaceLayout::custom(LayoutNode::Split {
            axis: LayoutAxis::Vertical,
            ratio: 0.3,
            a: Box::new(LayoutNode::Dock(DockRole::Locator)),
            b: Box::new(LayoutNode::Dock(DockRole::Inspector)),
        });
        layout.torn = vec![TornDock {
            role: DockRole::Activity,
            size: Some((960.0, 380.0)),
            position: Some((1920.0, 0.0)),
        }];
        let prefs = Prefs {
            layout: layout.clone(),
            ..Prefs::default()
        };
        prefs.save_to(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[[layout.torn]]"), "{text}");
        assert!(text.contains("role = \"activity\""), "{text}");
        let (back, note) = Prefs::load_from(&path);
        assert!(note.is_none());
        assert_eq!(back.layout, layout);
    }

    /// The torn list a hand edit cannot be trusted to keep sane (#186):
    /// the Locator is never torn, a role cannot be docked and torn at once,
    /// no role twice, and a silly size drops to the role's default — entry
    /// by entry, like an invalid selector, never a refused launch.
    #[test]
    fn hand_edited_torn_entries_degrade_entry_by_entry() {
        let path = tmp("torn-nonsense.toml");
        std::fs::write(
            &path,
            "[layout]\n\
             [layout.root.split]\naxis = \"vertical\"\nratio = 0.5\n\
             [layout.root.split.a]\ndock = \"locator\"\n\
             [layout.root.split.b]\ndock = \"inspector\"\n\
             [[layout.torn]]\nrole = \"locator\"\n\
             [[layout.torn]]\nrole = \"inspector\"\n\
             [[layout.torn]]\nrole = \"activity\"\nsize = [8.0, 8.0]\n\
             [[layout.torn]]\nrole = \"activity\"\n\
             [[layout.torn]]\nrole = \"workbench\"\nsize = [520.0, 700.0]\n",
        )
        .unwrap();
        let (prefs, note) = Prefs::load_from(&path);
        assert!(note.is_none(), "it parsed; the entries were just wrong");
        assert_eq!(
            prefs.layout.torn.iter().map(|t| t.role).collect::<Vec<_>>(),
            [DockRole::Activity, DockRole::Workbench],
            "locator never tears off; inspector is docked; activity once"
        );
        assert_eq!(
            prefs.layout.torn[0].size, None,
            "an 8x8 window is a hand edit, dropped to the default"
        );
        assert_eq!(prefs.layout.torn[1].size, Some((520.0, 700.0)));
    }

    /// `preset = "watch"` is fully docked: the three presets are the three
    /// named arrangements, and naming one re-docks whatever was torn.
    #[test]
    fn a_named_preset_wipes_the_torn_list_on_load() {
        let path = tmp("torn-preset.toml");
        std::fs::write(
            &path,
            "[layout]\npreset = \"watch\"\n\
             [[layout.torn]]\nrole = \"activity\"\n",
        )
        .unwrap();
        let (prefs, _) = Prefs::load_from(&path);
        assert_eq!(prefs.layout, LayoutPreset::Watch.layout());
        assert!(prefs.layout.torn.is_empty());
    }

    /// Each preset is sane, and the three trees are three different layouts —
    /// a copy-paste that made Watch and Diagnose the same tree would make
    /// Alt+2 and Alt+3 the same key.
    #[test]
    fn the_three_presets_are_sane_and_distinct() {
        for p in LayoutPreset::ALL {
            let layout = p.layout();
            assert_eq!(layout.preset, Some(p));
            assert!(layout.root.is_sane(), "{} names a dock twice", p.label());
            let roles = layout.root.roles();
            assert!(
                roles.contains(&DockRole::Locator) && roles.contains(&DockRole::Inspector),
                "{}: every preset shows the locator and the inspector",
                p.label()
            );
        }
        assert_ne!(LayoutPreset::Explore.root(), LayoutPreset::Watch.root());
        assert_ne!(LayoutPreset::Watch.root(), LayoutPreset::Diagnose.root());
        assert_ne!(LayoutPreset::Explore.root(), LayoutPreset::Diagnose.root());
    }
}
