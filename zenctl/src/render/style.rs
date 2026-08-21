//! Colour, narrowly (#200).
//!
//! > **Colour may only re-encode a distinction the plain text already makes.
//! > Stripping every escape sequence must leave output with identical
//! > information content.**
//!
//! That is the whole rule, and it is a safety rule rather than an aesthetic
//! one. `zenctl` shipped no colour at all for its first four versions and that
//! default was right, because the thing colour breaks is pipes. What makes it
//! safe to add now is that every distinction it marks is already carried by a
//! word or a glyph — `DEPRECATED`, `PASS`/`FAIL`/`UNPROVEN`, `✗ ⚠ ·` — so a
//! terminal that shows no colour, a log file, and `| grep` all read the same.
//!
//! Not covered by the rule, and therefore not done: colouring a row because it
//! is interesting, a heading because it is a heading, or a number because it is
//! large. Those add a channel the plain text does not have, and the next reader
//! to pipe the output loses it silently.
//!
//! ## The machine formats cannot carry an escape, structurally
//!
//! Not "must not" — *cannot*. [`Styling`] is constructed inside
//! [`super::Sink::report`]'s table arm and reaches the writer only through
//! [`super::Table`]; the json and ndjson arms have no `Styling` in scope, and
//! [`super::Render::rows`] returns `serde_json::Value`, which has nowhere to
//! put one. `--color always --format json` is not a case that is handled; it
//! is a case that has no spelling.
//!
//! Deliberately **not** `anstream::AutoStream`: its `RawStream` bound cannot
//! wrap `emit`'s generic `impl Write`, and a wrap-and-strip design would also
//! rewrite *payload bytes* that legitimately contain escapes — `topic echo` on
//! a key whose value is a terminal capture, say. Styles are emitted or they
//! are not; nothing is ever stripped from data.

use anstyle::{AnsiColor, Color, Style};

/// Whether a rendering may carry escapes.
///
/// Two states rather than a `bool` so that a reader of a signature can tell
/// which way round it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Styling {
    Plain,
    Colour,
}

impl Styling {
    /// Apply a style, or don't. The single place an escape is written.
    ///
    /// **An empty string is never painted**, and that is not an optimisation.
    /// A style over no text re-encodes nothing — there is no word for it to
    /// repeat — and the escapes would sit where the line ends, so the
    /// trailing-whitespace trim could no longer see the line as ending in
    /// whitespace. The result is a rendering that looks identical, strips to
    /// something *different*, and breaks the rule this module exists to keep.
    pub fn paint(self, text: &str, style: Option<Style>) -> String {
        match (self, style) {
            (Styling::Colour, Some(s)) if !text.is_empty() => {
                format!("{}{text}{}", s.render(), s.render_reset())
            }
            _ => text.to_string(),
        }
    }
}

/// What the user asked for on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum ColorChoice {
    /// A terminal gets colour; a pipe does not. Honours `NO_COLOR` and
    /// `CLICOLOR_FORCE`.
    #[default]
    Auto,
    Always,
    Never,
}

impl ColorChoice {
    /// Resolve against the environment.
    pub fn resolve(self) -> Styling {
        match self {
            ColorChoice::Always => Styling::Colour,
            ColorChoice::Never => Styling::Plain,
            ColorChoice::Auto => {
                // `NO_COLOR` first, because a user who set it is asking every
                // tool on the machine and should not have to ask twice. Then
                // `CLICOLOR_FORCE`, for the CI that wants colour in a captured
                // log. Then the terminal.
                if anstyle_query::no_color() {
                    Styling::Plain
                } else if anstyle_query::clicolor_force()
                    || std::io::IsTerminal::is_terminal(&std::io::stdout())
                {
                    Styling::Colour
                } else {
                    Styling::Plain
                }
            }
        }
    }
}

/// A contract violation: the fleet disagrees with the RFCs or with itself.
///
/// Every constant below marks a distinction the text already makes. There is
/// deliberately no `EMPHASIS`, no `HEADING` and no `INTERESTING`: a style with
/// no word behind it is a channel that vanishes in a pipe.
pub const ERROR: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Red)));
/// Suspicious but explainable — judgement degraded, not wrong.
pub const WARNING: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Yellow)));
/// A claim that held.
pub const PASS: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Green)));
/// Not a verdict either way — the observation could not carry the claim.
/// Dim rather than yellow, because "unproven" is not a milder failure.
pub const UNPROVEN: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::BrightBlack)));
/// Retired, and still here.
pub const DEPRECATED: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Magenta)));
/// Worth knowing; not a defect.
pub const INFO: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::BrightBlack)));

/// The severity vocabulary, styled. Kept beside the constants so the mapping
/// is one thing to read.
pub fn severity(s: zenkey_fleet::report::DoctorSeverity) -> Style {
    use zenkey_fleet::report::DoctorSeverity as S;
    match s {
        S::Error => ERROR,
        S::Warning => WARNING,
        S::Info => INFO,
    }
}
