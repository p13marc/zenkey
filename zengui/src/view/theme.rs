//! Theme-aware colors.
//!
//! Every color in zengui originates here — [`super::kit`] may name
//! `Color::TRANSPARENT` and nothing else — and `scripts/check-color.sh`
//! enforces it (#534): a raw `Color::from_rgb(…)` anywhere else fails CI.
//!
//! **The palette is ours (#534).** Until then zengui rendered on iced's stock
//! `Theme::Dark`/`Light`, whose `#2B2D31` put the 12px verdict badges near
//! 2:1 contrast, and whose missing warning slot had been faked by mixing
//! danger into success. Now [`Tokens`] is the single source — zinc surfaces
//! layered well → panel → raised, where a border is the same step as a
//! raised surface (what makes the layering read as soft rather than ruled),
//! and one indigo primary — and the iced [`iced::Theme`] is *built from* it
//! ([`iced_theme`]), so iced's own widgets (scrollables, pick-list menus,
//! sliders, the pane grid) wear the same palette as ours.
//!
//! Every text role is tested to reach WCAG AA (4.5:1) on the panel and on its
//! own chip fill, in both themes; the syntax and accent hues are tested to sit
//! at least 30° from every verdict hue, for the reason [`ThemeColors::series`]
//! gives.
//!
//! **A mode is not an alarm (#544).** Replay — the panes reading a file, the
//! live link off — and a recording in progress are states the operator chose,
//! not findings, yet they used to wear danger red and caution amber, the
//! colours of "something is wrong". They have a hue of their own now,
//! [`Tokens::mode`] through [`Tone::Mode`]: loud enough that the mode cannot
//! be missed, and tested to sit clear of every verdict hue and of the
//! primary, so it never reads as a verdict or as a selection.

use std::sync::LazyLock;

use iced::Color;
use iced::theme::palette::{self, Extended, Pair};

use crate::prefs::ThemeChoice;

/// One theme's colors, by role. `const`, so both palettes are plain data
/// a test can read without a renderer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tokens {
    /// The window behind everything; docks and cards float on it.
    pub well: Color,
    /// A dock, a card, an overlay.
    pub panel: Color,
    /// A step above the panel: a chip's fill, a segmented control's track,
    /// the hover base.
    pub raised: Color,
    /// Hairlines. Deliberately the same step as `raised` in dark — a border
    /// that is a surface tone, not a rule.
    pub line: Color,
    pub text: Color,
    /// Labels, units, counts — secondary, still read.
    pub text_muted: Color,
    /// "Not asked": absence of information, dimmer than commentary.
    pub text_dim: Color,
    pub primary: Color,
    /// Hover/press on a primary fill.
    pub primary_strong: Color,
    /// Text on a primary fill.
    pub on_primary: Color,
    /// The three verdict hues, each text-safe on the panel (the light theme
    /// takes the 700 shades for that reason). Chip fills and borders are
    /// derived from them by alpha.
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
    /// A tombstone (RFC 04 §1.2): retirement is a fact, not a negative
    /// verdict, so it does not borrow danger's hue.
    pub retired: Color,
    /// Payload syntax: keys and strings. Literals take the text colour and
    /// punctuation the dim one, so a number never lands near amber.
    pub syntax_key: Color,
    pub syntax_string: Color,
    /// A non-verdict hue for telling a thing apart without a claim (a peer
    /// on the mesh).
    pub accent: Color,
    /// A mode the operator chose (#544): replay, a recording. Never a
    /// verdict, never the primary — see [`Tone::Mode`].
    pub mode: Color,
    /// Behind a modal.
    pub scrim: Color,
    /// Under a modal, lifting it off the scrim.
    pub shadow: Color,
}

const fn rgb(hex: u32) -> Color {
    Color::from_rgb8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// Zinc + indigo, dark (the default).
pub const DARK: Tokens = Tokens {
    well: rgb(0x09090b),
    panel: rgb(0x18181b),
    raised: rgb(0x27272a),
    line: rgb(0x27272a),
    text: rgb(0xfafafa),
    text_muted: rgb(0xa1a1aa),
    // Not zinc-500: that is 3.7:1 on the panel, and "not asked" must stay
    // legible to stay honest.
    text_dim: rgb(0x8a8a93),
    primary: rgb(0x818cf8),
    primary_strong: rgb(0x6366f1),
    // Dark text: white on indigo-400 is 3:1.
    on_primary: rgb(0x09090b),
    success: rgb(0x34d399),
    warning: rgb(0xfbbf24),
    danger: rgb(0xf87171),
    retired: rgb(0xa8a29e),
    syntax_key: rgb(0x7dd3fc),
    syntax_string: rgb(0xf0abfc),
    accent: rgb(0x38bdf8),
    mode: rgb(0xe879f9),
    scrim: Color::from_rgba8(0, 0, 0, 0.6),
    shadow: Color::from_rgba8(0, 0, 0, 0.5),
};

/// Zinc + indigo, light.
pub const LIGHT: Tokens = Tokens {
    well: rgb(0xf4f4f5),
    panel: rgb(0xffffff),
    raised: rgb(0xf4f4f5),
    line: rgb(0xe4e4e7),
    text: rgb(0x09090b),
    text_muted: rgb(0x52525b),
    text_dim: rgb(0x6b6b74),
    primary: rgb(0x4f46e5),
    primary_strong: rgb(0x4338ca),
    on_primary: rgb(0xffffff),
    // The 700s (amber a shade past, so its word reads on its own chip): the
    // 600s are 3.8, 3.2 and 4.8:1 on white.
    success: rgb(0x047857),
    warning: rgb(0xa34d07),
    danger: rgb(0xb91c1c),
    retired: rgb(0x6f6862),
    syntax_key: rgb(0x0369a1),
    syntax_string: rgb(0xa21caf),
    accent: rgb(0x0284c7),
    // The 700: the 600 (#c026d3) does not read at AA on its own chip.
    mode: rgb(0xa21caf),
    scrim: Color::from_rgba8(9, 9, 11, 0.35),
    shadow: Color::from_rgba8(9, 9, 11, 0.18),
};

impl ThemeChoice {
    /// The tokens this choice renders with.
    pub fn tokens(self) -> &'static Tokens {
        match self {
            ThemeChoice::Dark => &DARK,
            ThemeChoice::Light => &LIGHT,
        }
    }
}

/// The iced theme for a choice, built from its [`Tokens`] — once: `theme()`
/// runs every frame, and `custom_with_fn` allocates.
pub fn iced_theme(choice: ThemeChoice) -> iced::Theme {
    static DARK_THEME: LazyLock<iced::Theme> = LazyLock::new(|| build("zengui dark", &DARK));
    static LIGHT_THEME: LazyLock<iced::Theme> = LazyLock::new(|| build("zengui light", &LIGHT));
    match choice {
        ThemeChoice::Dark => DARK_THEME.clone(),
        ThemeChoice::Light => LIGHT_THEME.clone(),
    }
}

fn build(name: &'static str, t: &'static Tokens) -> iced::Theme {
    let seed = palette::Palette {
        background: t.well,
        text: t.text,
        primary: t.primary,
        success: t.success,
        warning: t.warning,
        danger: t.danger,
    };
    iced::Theme::custom_with_fn(name, seed, move |p| {
        let mut e = Extended::generate(p);
        // iced's ladder, re-seated on ours: base is the well, weak the
        // panel, strong the line — what `surface()`/`border()` and every
        // stock widget read.
        let pair = |color| Pair {
            color,
            text: t.text,
        };
        let bg = &mut e.background;
        bg.base = pair(t.well);
        bg.weakest = pair(mix(t.well, t.panel, 0.33));
        bg.weaker = pair(mix(t.well, t.panel, 0.66));
        bg.weak = pair(t.panel);
        bg.neutral = pair(mix(t.panel, t.line, 0.5));
        bg.strong = pair(t.line);
        bg.stronger = pair(mix(t.line, t.text_dim, 0.25));
        bg.strongest = pair(mix(t.line, t.text_dim, 0.5));
        e.primary.base = Pair {
            color: t.primary,
            text: t.on_primary,
        };
        e.primary.strong = Pair {
            color: t.primary_strong,
            text: t.on_primary,
        };
        e
    })
}

/// A payload syntax role's colour, resolved for a theme choice rather than
/// from a live theme (#538): a rich text's spans take colours, not
/// closures, so the echo preview resolves them one step early. Same tokens
/// as [`ThemeColors::syntax`].
pub fn syntax_ink(choice: ThemeChoice, role: SyntaxRole) -> Color {
    let t = choice.tokens();
    match role {
        SyntaxRole::Key => t.syntax_key,
        SyntaxRole::String => t.syntax_string,
        SyntaxRole::Literal => t.text,
        SyntaxRole::Punct => t.text_dim,
    }
}

/// `c` at opacity `a` — a tint over whatever is behind it.
pub(crate) fn alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

/// Accessor over the active theme's palette.
pub struct ThemeColors<'a> {
    theme: &'a iced::Theme,
}

/// The entry point every view uses: `colors(theme).text_muted()`.
pub fn colors(theme: &iced::Theme) -> ThemeColors<'_> {
    ThemeColors { theme }
}

impl ThemeColors<'_> {
    /// The tokens behind the active theme. Two themes, told apart by
    /// darkness — which is all `Extended` records, and all there is to tell.
    pub fn tokens(&self) -> &'static Tokens {
        if self.is_dark() { &DARK } else { &LIGHT }
    }

    pub fn is_dark(&self) -> bool {
        self.theme.extended_palette().is_dark
    }

    /// The window behind everything.
    pub fn background(&self) -> Color {
        self.tokens().well
    }

    pub fn well(&self) -> Color {
        self.tokens().well
    }

    /// A dock, a card, an overlay — what `surface()` always meant.
    pub fn panel(&self) -> Color {
        self.tokens().panel
    }

    pub fn surface(&self) -> Color {
        self.panel()
    }

    /// A chip's fill, a track, a step above the panel.
    pub fn raised(&self) -> Color {
        self.tokens().raised
    }

    pub fn line(&self) -> Color {
        self.tokens().line
    }

    pub fn border(&self) -> Color {
        self.line()
    }

    pub fn text(&self) -> Color {
        self.tokens().text
    }

    /// Secondary text: labels, units, counts.
    pub fn text_muted(&self) -> Color {
        self.tokens().text_muted
    }

    /// Tertiary text: the "we have not asked" state. Deliberately dimmer than
    /// [`Self::text_muted`] so an unknown reads as absence of information
    /// rather than as a value.
    pub fn text_dim(&self) -> Color {
        self.tokens().text_dim
    }

    pub fn primary(&self) -> Color {
        self.tokens().primary
    }

    pub fn success(&self) -> Color {
        self.tokens().success
    }

    pub fn danger(&self) -> Color {
        self.tokens().danger
    }

    /// The palette's own warning — no longer danger mixed into success.
    pub fn warning(&self) -> Color {
        self.tokens().warning
    }

    /// The stronger primary shade — hover feedback on a primary surface.
    pub fn primary_strong(&self) -> Color {
        self.tokens().primary_strong
    }

    /// Text on a primary-filled surface.
    pub fn on_primary(&self) -> Color {
        self.tokens().on_primary
    }

    /// A hover wash: visible on the well *and* on a panel (#193).
    pub fn hover(&self) -> Color {
        let t = self.tokens();
        mix(t.panel, t.line, 0.6)
    }

    /// A selected row: a primary tint, so selection stops reading as a
    /// stuck hover (#534). The row's own text still says which it is.
    pub fn selected(&self) -> Color {
        alpha(self.primary(), 0.16)
    }

    /// How fresh a key is, as a single-hue ramp (#538): primary when seen
    /// just now, fading to dim. One hue on purpose — the old dot went green
    /// then amber, verdict colours for something that is not a verdict. The
    /// age word beside it carries the claim.
    pub fn freshness(&self, age_s: f32) -> Color {
        if age_s < 3.0 {
            self.primary()
        } else if age_s < 30.0 {
            mix(self.primary(), self.text_muted(), 0.6)
        } else {
            self.text_dim()
        }
    }

    /// A tombstone's colour (RFC 04 §1.2).
    pub fn retired(&self) -> Color {
        self.tokens().retired
    }

    /// Behind a modal.
    pub fn scrim(&self) -> Color {
        self.tokens().scrim
    }

    /// Under a modal.
    pub fn shadow(&self) -> Color {
        self.tokens().shadow
    }

    /// A payload's syntax role.
    pub fn syntax(&self, role: SyntaxRole) -> Color {
        let t = self.tokens();
        match role {
            SyntaxRole::Key => t.syntax_key,
            SyntaxRole::String => t.syntax_string,
            SyntaxRole::Literal => t.text,
            SyntaxRole::Punct => t.text_dim,
        }
    }

    /// The non-verdict accent.
    pub fn accent(&self) -> Color {
        self.tokens().accent
    }

    /// A chosen mode (#544) — see [`Tone::Mode`].
    pub fn mode(&self) -> Color {
        self.tokens().mode
    }

    /// The one swatch resolver (#193): every badge scale maps into [`Tone`]
    /// and every `Tone` resolves here, so "not asked" ([`Tone::Neutral`]) has
    /// its own slot by construction and can never borrow a verdict's colour.
    /// Text-safe in both themes: this is what a badge's glyph and word draw
    /// in.
    pub fn tone(&self, tone: Tone) -> Color {
        match tone {
            Tone::Positive => self.success(),
            Tone::Caution => self.warning(),
            Tone::Negative => self.danger(),
            // Absence of information, not a value — dimmer than commentary.
            Tone::Neutral => self.text_dim(),
            Tone::Info => self.text_muted(),
            Tone::Mode => self.mode(),
        }
    }

    /// A chip's fill for a tone (#535). `None` for [`Tone::Neutral`]: a
    /// question not asked has no answer to tint, so its chip is an outline.
    pub fn tone_fill(&self, tone: Tone) -> Option<Color> {
        match tone {
            Tone::Positive | Tone::Caution | Tone::Negative | Tone::Mode => {
                Some(alpha(self.tone(tone), 0.10))
            }
            Tone::Info => Some(self.raised()),
            Tone::Neutral => None,
        }
    }

    /// A chip's hairline for a tone (#535).
    pub fn tone_border(&self, tone: Tone) -> Color {
        match tone {
            Tone::Positive | Tone::Caution | Tone::Negative | Tone::Mode => {
                alpha(self.tone(tone), 0.28)
            }
            Tone::Info => Color::TRANSPARENT,
            Tone::Neutral => self.line(),
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

    /// A series' area fill (#540): its own colour fading to nothing, top to
    /// baseline — emphasis under the line, never a second encoding of it.
    /// The rate's is fainter, as its line is.
    pub fn series_area(&self, kind: SeriesTone) -> (Color, Color) {
        let c = self.series(kind);
        let top = match kind {
            SeriesTone::Value => 0.22,
            SeriesTone::Rate => 0.12,
        };
        (alpha(c, top), alpha(c, 0.0))
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

    /// The payload-conformance verdict scale (#164) — the GUI face of
    /// [`zenkey::schema::validate::Verdict`]'s three states.
    pub fn verdict(&self, kind: VerdictTone) -> Color {
        self.tone(kind.tone())
    }

    /// The why-ladder rung scale (#214) — Established / NotEstablished /
    /// NotAsked, the three answers a rung may give.
    pub fn rung(&self, kind: RungTone) -> Color {
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
    /// A mode the operator chose (#544): the panes are replaying a file, a
    /// recording is running. Loud, because a mode must not be missed — and
    /// a hue of its own, because it is no verdict: replay is not an error
    /// and a recording is not a warning.
    Mode,
}

/// What a span of a payload preview is (#538 draws them; the colours are
/// decided here, beside the verdicts they must not resemble).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxRole {
    Key,
    String,
    /// Numbers, booleans, null — the text colour: a number in amber would
    /// read as a caution.
    Literal,
    Punct,
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

/// How a payload-conformance verdict should read (#164) — the same
/// discipline as [`RegistrationTone`]: three states, never a boolean, and
/// `NotValidated` is [`Tone::Neutral`] because "not checked" must never wear
/// either answer's swatch (RFC 09 §5.1 O4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictTone {
    Valid,
    Invalid,
    NotValidated,
}

impl VerdictTone {
    /// Every state of the scale, for the uniqueness tests.
    pub const ALL: [Self; 3] = [Self::Valid, Self::Invalid, Self::NotValidated];

    /// The swatch this state resolves through.
    pub fn tone(self) -> Tone {
        match self {
            Self::Valid => Tone::Positive,
            Self::Invalid => Tone::Negative,
            Self::NotValidated => Tone::Neutral,
        }
    }

    /// The badge glyph, carried by the type (#193): checked-and-passed,
    /// checked-and-failed, and not checked — the CLI's own three marks.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Valid => "✓",
            Self::Invalid => "✗",
            Self::NotValidated => "·",
        }
    }
}

/// How one why-ladder rung's answer should read (#214).
///
/// `NotAsked` is [`Tone::Neutral`] and its own glyph: a rung whose input was
/// never fetched must not look like "asked, and the answer was no" — printing
/// `No` where it means `NotAsked` is the exact failure the ladder was built
/// to replace (RFC 09 §5.1 O4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RungTone {
    Established,
    NotEstablished,
    NotAsked,
}

impl RungTone {
    /// Every state of the scale, for the uniqueness tests.
    pub const ALL: [Self; 3] = [Self::Established, Self::NotEstablished, Self::NotAsked];

    /// The swatch this state resolves through. `NotEstablished` is a verdict
    /// *obtained* — caution, not danger: for five of the ten rungs it is the
    /// explanation the user came for, not an error.
    pub fn tone(self) -> Tone {
        match self {
            Self::Established => Tone::Positive,
            Self::NotEstablished => Tone::Caution,
            Self::NotAsked => Tone::Neutral,
        }
    }

    /// The badge glyph, carried by the type (#193).
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Established => "✓",
            Self::NotEstablished => "✗",
            Self::NotAsked => "·",
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
        all_distinct("verdict", &VerdictTone::ALL.map(VerdictTone::glyph));
        all_distinct("rung", &RungTone::ALL.map(RungTone::glyph));
    }

    /// #164's and #214's shared honesty: the not-checked / not-asked state of
    /// each new scale is `Neutral`, never a verdict's swatch.
    #[test]
    fn not_checked_and_not_asked_resolve_neutral() {
        assert_eq!(VerdictTone::NotValidated.tone(), Tone::Neutral);
        assert_eq!(RungTone::NotAsked.tone(), Tone::Neutral);
        for theme in [iced::Theme::Light, iced::Theme::Dark] {
            let c = colors(&theme);
            assert_ne!(
                c.verdict(VerdictTone::NotValidated),
                c.verdict(VerdictTone::Invalid),
                "not checked must not read as checked-and-failed"
            );
            assert_ne!(
                c.rung(RungTone::NotAsked),
                c.rung(RungTone::NotEstablished),
                "not asked must not read as answered-no"
            );
        }
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

    // ── The palette (#534) ────────────────────────────────────────────

    fn luminance(c: Color) -> f32 {
        let lin = |v: f32| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }

    /// WCAG 2 contrast ratio of an opaque `fg` on an opaque `bg`.
    fn contrast(fg: Color, bg: Color) -> f32 {
        let (a, b) = (luminance(fg), luminance(bg));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// `fg` composited over `bg` — what a tinted fill looks like on screen.
    fn over(fg: Color, bg: Color) -> Color {
        mix(Color { a: 1.0, ..bg }, Color { a: 1.0, ..fg }, fg.a)
    }

    fn hue(c: Color) -> f32 {
        let (max, min) = (c.r.max(c.g).max(c.b), c.r.min(c.g).min(c.b));
        let d = max - min;
        if d == 0.0 {
            return 0.0;
        }
        let h = if max == c.r {
            ((c.g - c.b) / d).rem_euclid(6.0)
        } else if max == c.g {
            (c.b - c.r) / d + 2.0
        } else {
            (c.r - c.g) / d + 4.0
        };
        h * 60.0
    }

    fn hue_distance(a: Color, b: Color) -> f32 {
        let d = (hue(a) - hue(b)).abs() % 360.0;
        d.min(360.0 - d)
    }

    /// The iced theme is built *from* the tokens, so iced's stock widgets
    /// read the same surfaces ours do — and `ThemeColors` dispatches back to
    /// the right tokens from it.
    #[test]
    fn the_iced_theme_agrees_with_its_tokens() {
        for choice in ThemeChoice::ALL {
            let theme = iced_theme(choice);
            let t = choice.tokens();
            let e = theme.extended_palette();
            assert_eq!(e.background.base.color, t.well);
            assert_eq!(e.background.weak.color, t.panel);
            assert_eq!(e.background.strong.color, t.line);
            assert_eq!(e.primary.base.color, t.primary);
            assert_eq!(e.primary.base.text, t.on_primary);
            assert_eq!(colors(&theme).tokens(), t, "{choice:?}");
        }
    }

    /// Every text role reads at AA (4.5:1) on the surfaces it is drawn on —
    /// the panel and the well — and a verdict's word reads at AA on its own
    /// chip fill. Under iced's stock Dark these were near 2:1 (#534).
    #[test]
    fn every_text_role_meets_aa() {
        for choice in ThemeChoice::ALL {
            let theme = iced_theme(choice);
            let c = colors(&theme);
            let t = choice.tokens();
            let roles = [
                ("text", t.text),
                ("muted", t.text_muted),
                ("dim", t.text_dim),
                ("primary", t.primary),
                ("success", t.success),
                ("warning", t.warning),
                ("danger", t.danger),
                ("retired", t.retired),
                ("syntax key", t.syntax_key),
                ("syntax string", t.syntax_string),
                ("mode", t.mode),
            ];
            for (name, fg) in roles {
                for (ground, bg) in [("panel", t.panel), ("well", t.well)] {
                    let r = contrast(fg, bg);
                    assert!(r >= 4.5, "{choice:?} {name} on {ground}: {r:.2}");
                }
            }
            for tone in [
                Tone::Positive,
                Tone::Caution,
                Tone::Negative,
                Tone::Info,
                Tone::Mode,
            ] {
                let fill = over(c.tone_fill(tone).expect("tinted"), t.panel);
                let r = contrast(c.tone(tone), fill);
                assert!(r >= 4.5, "{choice:?} {tone:?} chip: {r:.2}");
            }
            let r = contrast(t.on_primary, t.primary);
            assert!(r >= 4.5, "{choice:?} on_primary: {r:.2}");
        }
    }

    /// The chart's argument (see [`ThemeColors::series`]), extended to every
    /// colour that is not a verdict: a syntax hue or an accent near green,
    /// amber or red would read as a claim about the data.
    #[test]
    fn syntax_and_accents_stay_clear_of_the_verdict_hues() {
        for choice in ThemeChoice::ALL {
            let t = choice.tokens();
            let neutral = [
                ("syntax key", t.syntax_key),
                ("syntax string", t.syntax_string),
                ("accent", t.accent),
                ("mode", t.mode),
            ];
            for (name, c) in neutral {
                for (verdict, v) in [
                    ("success", t.success),
                    ("warning", t.warning),
                    ("danger", t.danger),
                ] {
                    let d = hue_distance(c, v);
                    assert!(d >= 30.0, "{choice:?} {name} is {d:.0}° from {verdict}");
                }
            }
        }
    }

    /// #544: a mode must not read as a selection either — the primary is
    /// what the user picked, the mode what the panes are doing.
    #[test]
    fn the_mode_is_not_the_primary() {
        for choice in ThemeChoice::ALL {
            let t = choice.tokens();
            let d = hue_distance(t.mode, t.primary);
            assert!(d >= 30.0, "{choice:?} mode is {d:.0}° from primary");
        }
    }

    /// iced 0.14 has a warning slot; the palette's own amber replaces the
    /// danger-into-success blend #193 had to fake.
    #[test]
    fn warning_is_the_palettes_own() {
        for choice in ThemeChoice::ALL {
            let theme = iced_theme(choice);
            assert_eq!(colors(&theme).warning(), choice.tokens().warning);
            assert_eq!(
                theme.extended_palette().warning.base.color,
                choice.tokens().warning
            );
        }
    }

    /// A selected row and a hovered one used to paint the same shade.
    #[test]
    fn selection_is_not_a_stuck_hover() {
        for choice in ThemeChoice::ALL {
            let theme = iced_theme(choice);
            let c = colors(&theme);
            assert_ne!(c.selected(), c.hover());
        }
    }

    /// A question not asked has no answer to tint: Neutral's chip is an
    /// outline, every answered tone a fill (#535's anatomy, decided here).
    #[test]
    fn only_an_answer_is_tinted() {
        for choice in ThemeChoice::ALL {
            let theme = iced_theme(choice);
            let c = colors(&theme);
            assert_eq!(c.tone_fill(Tone::Neutral), None);
            for tone in [Tone::Positive, Tone::Caution, Tone::Negative, Tone::Mode] {
                assert!(c.tone_fill(tone).is_some());
            }
        }
    }
}
