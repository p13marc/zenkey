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

/// The persisted document.
///
/// Every field is `#[serde(default)]`-shaped so a file written by an older
/// build, or hand-edited down to two lines, still loads — the same
/// forward/backward tolerance the slice parser has, for the same reason.
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
    /// Left/right split ratio, 0..1.
    pub split: f32,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            theme: ThemeChoice::default(),
            zoom: 1.0,
            window: None,
            context: None,
            scope: ScopePreset::Everything,
            split: 0.5,
        }
    }
}

impl Prefs {
    /// Where the file lives: beside the shared context config, so one
    /// directory holds everything an explorer remembers.
    pub fn path() -> PathBuf {
        let config = zenkey_fleet::context_store::config_path();
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
        self.split = self.split.clamp(0.15, 0.85);
        if !self.split.is_finite() {
            self.split = 0.5;
        }
        self.window = self
            .window
            .filter(|(w, h)| w.is_finite() && h.is_finite() && *w >= 320.0 && *h >= 240.0);
        self
    }

    /// Persist, best-effort — a preference that cannot be saved must not fail
    /// the action the user actually took.
    pub fn save(&self) {
        let _ = self.save_to(&Self::path());
    }

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
            split: 0.4,
        };
        prefs.save_to(&path).unwrap();
        let (back, note) = Prefs::load_from(&path);
        assert_eq!(back, prefs);
        assert!(note.is_none());
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
    /// discard the `theme` sitting next to it.
    #[test]
    fn hand_edited_nonsense_is_clamped_field_by_field() {
        let path = tmp("nonsense.toml");
        std::fs::write(
            &path,
            "theme = \"light\"\nzoom = 99.0\nsplit = -3.0\nwindow = [1.0, 1.0]\n",
        )
        .unwrap();
        let (prefs, note) = Prefs::load_from(&path);
        assert!(note.is_none(), "it parsed; the values were just silly");
        assert_eq!(prefs.theme, ThemeChoice::Light, "the good field survives");
        assert_eq!(prefs.zoom, MAX_ZOOM);
        assert_eq!(prefs.split, 0.15);
        assert!(prefs.window.is_none(), "a 1x1 window is dropped, not kept");
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
}
