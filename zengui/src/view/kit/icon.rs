//! Chrome icons (#533).

use iced::widget::{Text, text};

use crate::view::theme::colors;
use crate::view::tokens::{face, font};

/// A chrome icon (#533): one Lucide glyph from the bundled, cut-down icon
/// font.
///
/// **Chrome only.** An icon marks a control or a region — a dock, a verb,
/// an empty state's kind. It never carries a state: every badge keeps its
/// own glyph from `theme`'s tone scales, drawn as text in Inter, because
/// that glyph is the honesty carrier and the tests read it off the screen.
/// An icon is drawn beside words, or alone only on a stateless verb that
/// also has a tooltip and a palette entry.
///
/// Typed rather than a `char` at the call site, so an icon the font was not
/// cut for cannot be written; `view::fonts`'s tests hold this list, the
/// font and `assets/fonts/lucide-icons.txt` to one set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    // Window chrome.
    Close,
    TearOff,
    ChevronRight,
    ChevronDown,
    Watched,
    Unwatched,
    Logo,
    // The connection.
    Plug,
    Unplug,
    Reconnect,
    // Verbs.
    Record,
    Stop,
    Replay,
    Play,
    Pause,
    Settings,
    Light,
    Dark,
    Density,
    ZoomOut,
    ZoomIn,
    Search,
    Filter,
    FilterOff,
    Copy,
    Export,
    Clear,
    Pin,
    Save,
    Open,
    GoTo,
    Retire,
    Sort,
    Check,
    Selectors,
    Keyboard,
    Command,
    // Docks and tools.
    Locator,
    Inspector,
    Activity,
    Workbench,
    Echo,
    Send,
    Doctor,
    Traffic,
    Nodes,
    Admin,
    Mesh,
    // Layout presets.
    Explore,
    Watch,
    Diagnose,
    // Why a surface is empty.
    NotAsked,
    Silent,
    Empty,
    Unselected,
    Info,
    // Quantities.
    Rate,
    Keys,
    Samples,
    Volume,
    Dropped,
    Clock,
    // Node roles.
    Router,
    Peer,
    Client,
}

impl Icon {
    pub const ALL: [Icon; 65] = [
        Icon::Close,
        Icon::TearOff,
        Icon::ChevronRight,
        Icon::ChevronDown,
        Icon::Watched,
        Icon::Unwatched,
        Icon::Logo,
        Icon::Plug,
        Icon::Unplug,
        Icon::Reconnect,
        Icon::Record,
        Icon::Stop,
        Icon::Replay,
        Icon::Play,
        Icon::Pause,
        Icon::Settings,
        Icon::Light,
        Icon::Dark,
        Icon::Density,
        Icon::ZoomOut,
        Icon::ZoomIn,
        Icon::Search,
        Icon::Filter,
        Icon::FilterOff,
        Icon::Copy,
        Icon::Export,
        Icon::Clear,
        Icon::Pin,
        Icon::Save,
        Icon::Open,
        Icon::GoTo,
        Icon::Retire,
        Icon::Sort,
        Icon::Check,
        Icon::Selectors,
        Icon::Keyboard,
        Icon::Command,
        Icon::Locator,
        Icon::Inspector,
        Icon::Activity,
        Icon::Workbench,
        Icon::Echo,
        Icon::Send,
        Icon::Doctor,
        Icon::Traffic,
        Icon::Nodes,
        Icon::Admin,
        Icon::Mesh,
        Icon::Explore,
        Icon::Watch,
        Icon::Diagnose,
        Icon::NotAsked,
        Icon::Silent,
        Icon::Empty,
        Icon::Unselected,
        Icon::Info,
        Icon::Rate,
        Icon::Keys,
        Icon::Samples,
        Icon::Volume,
        Icon::Dropped,
        Icon::Clock,
        Icon::Router,
        Icon::Peer,
        Icon::Client,
    ];

    /// The Lucide icon this draws — the name `lucide-icons.txt` lists.
    pub const fn lucide_name(self) -> &'static str {
        self.entry().0
    }

    /// Its codepoint in lucide-static 1.52.0's font (Private Use Area).
    pub const fn codepoint(self) -> char {
        self.entry().1
    }

    const fn entry(self) -> (&'static str, char) {
        match self {
            Icon::Close => ("x", '\u{E1B2}'),
            Icon::TearOff => ("square-arrow-out-up-right", '\u{E5A4}'),
            Icon::ChevronRight => ("chevron-right", '\u{E06F}'),
            Icon::ChevronDown => ("chevron-down", '\u{E06D}'),
            Icon::Watched => ("eye", '\u{E0BA}'),
            Icon::Unwatched => ("eye-off", '\u{E0BB}'),
            Icon::Logo => ("orbit", '\u{E3E7}'),
            Icon::Plug => ("plug", '\u{E37F}'),
            Icon::Unplug => ("unplug", '\u{E45D}'),
            Icon::Reconnect => ("refresh-cw", '\u{E145}'),
            Icon::Record => ("circle-dot", '\u{E345}'),
            Icon::Stop => ("square", '\u{E167}'),
            Icon::Replay => ("history", '\u{E1F5}'),
            Icon::Play => ("play", '\u{E13C}'),
            Icon::Pause => ("pause", '\u{E12E}'),
            Icon::Settings => ("settings", '\u{E154}'),
            Icon::Light => ("sun", '\u{E178}'),
            Icon::Dark => ("moon", '\u{E11E}'),
            Icon::Density => ("rows-3", '\u{E58A}'),
            Icon::ZoomOut => ("minus", '\u{E11C}'),
            Icon::ZoomIn => ("plus", '\u{E13D}'),
            Icon::Search => ("search", '\u{E151}'),
            Icon::Filter => ("list-filter", '\u{E460}'),
            Icon::FilterOff => ("funnel-x", '\u{E3B5}'),
            Icon::Copy => ("copy", '\u{E09E}'),
            Icon::Export => ("download", '\u{E0B2}'),
            Icon::Clear => ("trash-2", '\u{E18E}'),
            Icon::Pin => ("pin", '\u{E259}'),
            Icon::Save => ("save", '\u{E14D}'),
            Icon::Open => ("folder-open", '\u{E247}'),
            Icon::GoTo => ("arrow-right", '\u{E049}'),
            Icon::Retire => ("archive-x", '\u{E50C}'),
            Icon::Sort => ("arrow-up-down", '\u{E37D}'),
            Icon::Check => ("check", '\u{E06C}'),
            Icon::Selectors => ("braces", '\u{E36A}'),
            Icon::Keyboard => ("keyboard", '\u{E284}'),
            Icon::Command => ("command", '\u{E09A}'),
            Icon::Locator => ("folder-tree", '\u{E33C}'),
            Icon::Inspector => ("scan-search", '\u{E537}'),
            Icon::Activity => ("activity", '\u{E038}'),
            Icon::Workbench => ("wrench", '\u{E1B1}'),
            Icon::Echo => ("radio", '\u{E142}'),
            Icon::Send => ("send", '\u{E152}'),
            Icon::Doctor => ("stethoscope", '\u{E2F1}'),
            Icon::Traffic => ("chart-column", '\u{E2A3}'),
            Icon::Nodes => ("server", '\u{E153}'),
            Icon::Admin => ("database", '\u{E0AD}'),
            Icon::Mesh => ("network", '\u{E125}'),
            Icon::Explore => ("compass", '\u{E09B}'),
            Icon::Watch => ("binoculars", '\u{E621}'),
            Icon::Diagnose => ("heart-pulse", '\u{E36E}'),
            Icon::NotAsked => ("circle-dashed", '\u{E4B0}'),
            Icon::Silent => ("signal-zero", '\u{E263}'),
            Icon::Empty => ("inbox", '\u{E0F7}'),
            Icon::Unselected => ("mouse-pointer-click", '\u{E120}'),
            Icon::Info => ("info", '\u{E0F9}'),
            Icon::Rate => ("gauge", '\u{E1BF}'),
            Icon::Keys => ("key-round", '\u{E4A3}'),
            Icon::Samples => ("layers", '\u{E529}'),
            Icon::Volume => ("hard-drive", '\u{E0ED}'),
            Icon::Dropped => ("circle-minus", '\u{E07E}'),
            Icon::Clock => ("clock", '\u{E087}'),
            Icon::Router => ("router", '\u{E3BF}'),
            Icon::Peer => ("share-2", '\u{E156}'),
            Icon::Client => ("laptop", '\u{E1CD}'),
        }
    }
}

/// An icon at BODY size — beside a control's word or alone in a header.
pub fn icon<'a>(i: Icon) -> Text<'a> {
    glyph_text(i).size(font::BODY)
}

/// An icon at CAPTION size — the only size a virtualized row may hold, so a
/// row's height model stays the caption line it was measured against.
pub fn icon_caption<'a>(i: Icon) -> Text<'a> {
    glyph_text(i).size(font::CAPTION)
}

fn glyph_text<'a>(i: Icon) -> Text<'a> {
    text(i.codepoint().to_string())
        .font(face::ICONS)
        // A lone private-use glyph needs no shaping, and basic shaping
        // keeps the icon font out of fallback entirely.
        .shaping(iced::widget::text::Shaping::Basic)
        .line_height(iced::widget::text::LineHeight::Relative(1.0))
}

/// A breadcrumb separator: a muted chevron between the location bar's
/// stations (context ▸ base ▸ scope ▸ key).
pub fn separator<'a>() -> Text<'a> {
    icon_caption(Icon::ChevronRight).style(|theme: &iced::Theme| text::Style {
        color: Some(colors(theme).text_muted()),
    })
}
