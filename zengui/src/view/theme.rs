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

    /// The stronger primary shade — hover feedback on a primary surface.
    pub fn primary_strong(&self) -> Color {
        self.palette().primary.strong.color
    }

    /// Text on a primary-filled surface.
    pub fn on_primary(&self) -> Color {
        self.palette().primary.base.text
    }

    /// A hover wash: between `surface()` and `border()`, so it is visible on
    /// both the pane background and a card (#193).
    pub fn hover(&self) -> Color {
        let p = self.palette();
        mix(p.background.weak.color, p.background.strong.color, 0.5)
    }

    /// The one swatch resolver (#193): every badge scale maps into [`Tone`]
    /// and every `Tone` resolves here, so "not asked" ([`Tone::Neutral`]) has
    /// its own slot by construction and can never borrow a verdict's colour.
    pub fn tone(&self, tone: Tone) -> Color {
        match tone {
            Tone::Positive => self.success(),
            Tone::Caution => self.warning(),
            Tone::Negative => self.danger(),
            // Absence of information, not a value — dimmer than commentary.
            Tone::Neutral => self.text_dim(),
            Tone::Info => self.text_muted(),
        }
    }

    /// The registration badge scale (see [`crate::keyfacts::Registration`]).
    /// The tri-state is only honest if the three states *look* different.
    pub fn registration(&self, kind: RegistrationTone) -> Color {
        self.tone(kind.tone())
    }

    /// The presence badge scale (#61).
    pub fn presence(&self, kind: PresenceTone) -> Color {
        self.tone(kind.tone())
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
        self.tone(kind.tone())
    }

    /// The storage-coverage scale (#70), mirroring the CLI's ✓/~/· marks.
    pub fn coverage(&self, kind: CoverageTone) -> Color {
        self.tone(kind.tone())
    }
}

/// The swatch scale every badge resolves through (#193).
///
/// Five slots, not four: `Neutral` is "we have not asked" (or "the question
/// does not apply"), structurally its own slot — so an unanswered question can
/// never borrow the swatch of "asked, and the answer was no". RFC 05 §3.1
/// forbids exactly that false verdict, and the whole five-state
/// [`crate::keyfacts::Registration`] exists to prevent it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// A positive verdict, obtained: registered, alive, covered.
    Positive,
    /// A qualified verdict, obtained: unregistered, partial, a warning.
    Caution,
    /// A negative verdict, obtained: no slice, suspect, an error.
    Negative,
    /// No verdict. Dim, reading as absence of information — never as "no".
    Neutral,
    /// Commentary — muted, but still information (the doctor's `Info`).
    Info,
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

impl CoverageTone {
    /// Every state of the scale, for the uniqueness tests.
    pub const ALL: [Self; 3] = [Self::Covered, Self::Partial, Self::Uncovered];

    /// The swatch this state resolves through. `Uncovered` is
    /// [`Tone::Neutral`] by the doc above: dim, never `danger()`.
    pub fn tone(self) -> Tone {
        match self {
            Self::Covered => Tone::Positive,
            Self::Partial => Tone::Caution,
            Self::Uncovered => Tone::Neutral,
        }
    }

    /// The badge glyph — the CLI's own ✓/~/· marks, so the two explorers
    /// read alike. Carried by the type (#193): no call site can omit it or
    /// give two states of this scale the same mark.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Covered => "✓",
            Self::Partial => "~",
            Self::Uncovered => "·",
        }
    }
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

impl SeverityTone {
    /// Every state of the scale, for the uniqueness tests.
    pub const ALL: [Self; 3] = [Self::Error, Self::Warning, Self::Info];

    /// The swatch this state resolves through.
    pub fn tone(self) -> Tone {
        match self {
            Self::Error => Tone::Negative,
            Self::Warning => Tone::Caution,
            Self::Info => Tone::Info,
        }
    }

    /// The badge glyph — the CLI's ✗/⚠/· marks, carried by the type (#193).
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Error => "✗",
            Self::Warning => "⚠",
            Self::Info => "·",
        }
    }
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

impl RegistrationTone {
    /// Every state of the scale, for the uniqueness tests.
    pub const ALL: [Self; 5] = [
        Self::Registered,
        Self::Unregistered,
        Self::NoSlice,
        Self::Unknown,
        Self::NotApplicable,
    ];

    /// The swatch this state resolves through. "Not asked" and "not
    /// applicable" are [`Tone::Neutral`] — never a verdict's swatch.
    pub fn tone(self) -> Tone {
        match self {
            Self::Registered => Tone::Positive,
            Self::Unregistered => Tone::Caution,
            Self::NoSlice => Tone::Negative,
            Self::Unknown | Self::NotApplicable => Tone::Neutral,
        }
    }

    /// The badge glyph, carried by the type (#193): a filled dot is a "yes"
    /// obtained, a hollow one a "no" obtained, ✗ a broken registry claim, and
    /// the two non-verdicts get the CLI's not-asked marks — no call site can
    /// omit a glyph or give two states the same one.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Registered => "●",
            Self::Unregistered => "○",
            Self::NoSlice => "✗",
            Self::Unknown => "·",
            Self::NotApplicable => "—",
        }
    }
}

/// How a liveliness presence should read (#61) — same discipline as
/// [`RegistrationTone`]: "unknown" must not look like a verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceTone {
    Alive,
    Suspect,
    Unknown,
}

impl PresenceTone {
    /// Every state of the scale, for the uniqueness tests.
    pub const ALL: [Self; 3] = [Self::Alive, Self::Suspect, Self::Unknown];

    /// The swatch this state resolves through — "unknown" is
    /// [`Tone::Neutral`], the same discipline as registration.
    pub fn tone(self) -> Tone {
        match self {
            Self::Alive => Tone::Positive,
            Self::Suspect => Tone::Negative,
            Self::Unknown => Tone::Neutral,
        }
    }

    /// The badge glyph, carried by the type (#193): a token seen, a token
    /// retracted, and a question never asked.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Alive => "●",
            Self::Suspect => "✗",
            Self::Unknown => "·",
        }
    }
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

    /// #193's second acceptance: within one scale, no two states may share a
    /// glyph — the glyph is the carrier that survives when colour does not.
    #[test]
    fn no_two_tones_in_a_scale_share_a_glyph() {
        fn all_distinct(scale: &str, glyphs: &[&'static str]) {
            for (i, a) in glyphs.iter().enumerate() {
                for b in &glyphs[i + 1..] {
                    assert_ne!(a, b, "{scale}: two states share the glyph {a:?}");
                }
            }
        }
        all_distinct(
            "registration",
            &RegistrationTone::ALL.map(RegistrationTone::glyph),
        );
        all_distinct("presence", &PresenceTone::ALL.map(PresenceTone::glyph));
        all_distinct("severity", &SeverityTone::ALL.map(SeverityTone::glyph));
        all_distinct("coverage", &CoverageTone::ALL.map(CoverageTone::glyph));
    }

    /// "Not asked" must not share a swatch with any verdict (#193): `Neutral`
    /// is its own slot in [`Tone`], and both not-asked registration states
    /// resolve there — never to `Unregistered`'s caution.
    #[test]
    fn not_asked_never_shares_a_swatch_with_a_verdict() {
        for theme in [iced::Theme::Light, iced::Theme::Dark] {
            let c = colors(&theme);
            for verdict in [Tone::Positive, Tone::Caution, Tone::Negative] {
                assert_ne!(c.tone(Tone::Neutral), c.tone(verdict));
            }
            for not_asked in [RegistrationTone::Unknown, RegistrationTone::NotApplicable] {
                assert_eq!(not_asked.tone(), Tone::Neutral);
                assert_ne!(
                    c.registration(not_asked),
                    c.registration(RegistrationTone::Unregistered),
                    "not asked must not read as 'asked, and the answer was no'"
                );
            }
        }
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
