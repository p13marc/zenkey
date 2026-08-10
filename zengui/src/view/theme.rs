//! Theme-aware colors.
//!
//! Every color in zengui originates here or in [`super::kit`]. Anywhere else a
//! raw `Color::from_rgb(…)` is a bug — the same rule zensight enforces with a
//! CI grep, and worth having from day one rather than retrofitting.
//!
//! Most colors delegate to iced's extended palette so light and dark come for
//! free; only the explorer-specific semantics (the registration badge scale)
//! are named constants.

use iced::Color;
use iced::theme::palette::Extended;

/// Accessor over the active theme's palette.
pub struct ThemeColors<'a> {
    theme: &'a iced::Theme,
}

/// The entry point every view uses: `colors(theme).text_muted()`.
pub fn colors(theme: &iced::Theme) -> ThemeColors<'_> {
    ThemeColors { theme }
}

impl ThemeColors<'_> {
    fn palette(&self) -> &Extended {
        self.theme.extended_palette()
    }

    pub fn is_dark(&self) -> bool {
        self.palette().is_dark
    }

    pub fn background(&self) -> Color {
        self.palette().background.base.color
    }

    pub fn surface(&self) -> Color {
        self.palette().background.weak.color
    }

    pub fn border(&self) -> Color {
        self.palette().background.strong.color
    }

    pub fn text(&self) -> Color {
        self.palette().background.base.text
    }

    /// Secondary text: labels, units, counts.
    pub fn text_muted(&self) -> Color {
        let p = self.palette();
        mix(p.background.base.text, p.background.base.color, 0.35)
    }

    /// Tertiary text: the "we have not asked" state. Deliberately dimmer than
    /// [`Self::text_muted`] so an unknown reads as absence of information
    /// rather than as a value.
    pub fn text_dim(&self) -> Color {
        let p = self.palette();
        mix(p.background.base.text, p.background.base.color, 0.6)
    }

    pub fn primary(&self) -> Color {
        self.palette().primary.base.color
    }

    pub fn success(&self) -> Color {
        self.palette().success.base.color
    }

    pub fn danger(&self) -> Color {
        self.palette().danger.base.color
    }

    pub fn warning(&self) -> Color {
        // iced's extended palette has no warning slot; derive one that keeps
        // its relationship to the theme rather than hard-coding an amber.
        let d = self.palette().danger.base.color;
        let s = self.palette().success.base.color;
        mix(d, s, 0.45)
    }

    /// The registration badge scale (see [`crate::keyfacts::Registration`]).
    /// The tri-state is only honest if the three states *look* different.
    pub fn registration(&self, kind: RegistrationTone) -> Color {
        match kind {
            RegistrationTone::Registered => self.success(),
            RegistrationTone::Unregistered => self.warning(),
            RegistrationTone::NoSlice => self.danger(),
            // "Not asked" and "not applicable" must not read as a verdict.
            RegistrationTone::Unknown | RegistrationTone::NotApplicable => self.text_dim(),
        }
    }

    /// The presence badge scale (#61).
    pub fn presence(&self, kind: PresenceTone) -> Color {
        match kind {
            PresenceTone::Alive => self.success(),
            PresenceTone::Suspect => self.danger(),
            PresenceTone::Unknown => self.text_dim(),
        }
    }

    /// A plotted series (#64): the value line.
    ///
    /// The chart's own accent rather than a borrowed semantic colour — a line
    /// drawn in `success()` or `danger()` would read as a verdict about the
    /// numbers, which a plot of arbitrary telemetry has no business implying.
    pub fn series(&self, kind: SeriesTone) -> Color {
        match kind {
            SeriesTone::Value => self.primary(),
            // The rate is the observer's own measurement, not the publisher's
            // data, and is dimmed to say so.
            SeriesTone::Rate => mix(self.primary(), self.background(), 0.4),
        }
    }

    /// The baseline and gridline of a chart — structure, never data.
    pub fn axis(&self) -> Color {
        self.border()
    }

    /// The doctor severity scale (#71), mirroring the CLI's ✗/⚠/· marks.
    pub fn severity(&self, kind: SeverityTone) -> Color {
        match kind {
            SeverityTone::Error => self.danger(),
            SeverityTone::Warning => self.warning(),
            SeverityTone::Info => self.text_muted(),
        }
    }

    /// The storage-coverage scale (#70), mirroring the CLI's ✓/~/· marks.
    pub fn coverage(&self, kind: CoverageTone) -> Color {
        match kind {
            CoverageTone::Covered => self.success(),
            CoverageTone::Partial => self.warning(),
            CoverageTone::Uncovered => self.text_dim(),
        }
    }
}

/// How storage coverage should read (#70).
///
/// `Uncovered` is **dimmed, never `danger()`**: RFC 04 §3.5 makes an uncovered
/// ttl'd family perfectly legitimate — volatile state may be seeded by the
/// advanced pub/sub cache — so a red badge would report a verdict the tool
/// never obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageTone {
    Covered,
    Partial,
    Uncovered,
}

/// Which series a chart is drawing (#64).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesTone {
    /// A numeric field of the payload, over time.
    Value,
    /// The observed sample rate, over time.
    Rate,
}

/// How a doctor finding's severity should read (#71).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeverityTone {
    Error,
    Warning,
    Info,
}

/// How a registration state should read at a glance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationTone {
    Registered,
    Unregistered,
    NoSlice,
    Unknown,
    NotApplicable,
}

/// How a liveliness presence should read (#61) — same discipline as
/// [`RegistrationTone`]: "unknown" must not look like a verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceTone {
    Alive,
    Suspect,
    Unknown,
}

/// Linear blend, `t` from `a` (0.0) to `b` (1.0).
fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color::from_rgba(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        a.a + (b.a - a.a) * t,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_interpolates_between_the_endpoints() {
        let a = Color::from_rgb(0.0, 0.0, 0.0);
        let b = Color::from_rgb(1.0, 1.0, 1.0);
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
        assert!((mix(a, b, 0.5).r - 0.5).abs() < 1e-6);
        // Out-of-range factors clamp rather than overshoot into invalid colors.
        assert_eq!(mix(a, b, 2.0), b);
        assert_eq!(mix(a, b, -1.0), a);
    }

    /// The tri-state is only honest if the states are visually distinct.
    #[test]
    fn registration_tones_are_distinguishable() {
        let theme = iced::Theme::Dark;
        let c = colors(&theme);
        let registered = c.registration(RegistrationTone::Registered);
        let unregistered = c.registration(RegistrationTone::Unregistered);
        let unknown = c.registration(RegistrationTone::Unknown);
        assert_ne!(registered, unregistered);
        assert_ne!(unregistered, unknown);
        assert_ne!(registered, unknown);
    }

    /// Both themes must resolve — a panic here would only show up at runtime.
    #[test]
    fn both_themes_resolve() {
        for theme in [iced::Theme::Light, iced::Theme::Dark] {
            let c = colors(&theme);
            let _ = (c.background(), c.text(), c.text_muted(), c.text_dim());
            let _ = (c.primary(), c.success(), c.danger(), c.warning());
        }
        assert!(colors(&iced::Theme::Dark).is_dark());
        assert!(!colors(&iced::Theme::Light).is_dark());
    }
}
