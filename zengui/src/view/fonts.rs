//! The faces zengui ships, embedded (#533).
//!
//! Until #533 every glyph came from whatever the host's fontdb offered —
//! DejaVu on one machine, Cantarell on another — with no weight ever
//! requested, so the same build looked different everywhere and hierarchy was
//! carried by size and colour alone. Now the binary carries its type:
//!
//! | face | for | licence |
//! |---|---|---|
//! | Inter Regular / Medium / SemiBold 4.1 | chrome: labels, prose, headers | OFL-1.1 |
//! | JetBrains Mono **NL** 2.304 | data: keys, payloads, ids, hex | OFL-1.1 |
//! | Lucide 1.52.0, cut to [`super::kit::Icon`] | chrome icons | ISC |
//!
//! JetBrains Mono's *NL* cut, deliberately: the ligature build draws `->`,
//! `!=` and `==` as single glyphs, and an explorer that disguises the bytes
//! of a key or a payload is lying in the one place it must not.
//!
//! [`settings`] is the one way these reach a renderer — the daemon and the
//! shots harness both take it, so a screenshot is drawn with the faces the
//! app is.

use std::borrow::Cow;

use super::tokens::{face, font};

pub const INTER_REGULAR: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
pub const INTER_MEDIUM: &[u8] = include_bytes!("../../assets/fonts/Inter-Medium.ttf");
pub const INTER_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");
pub const MONO: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMonoNL-Regular.ttf");
pub const ICONS: &[u8] = include_bytes!("../../assets/fonts/lucide.ttf");

/// Every face, in load order.
pub const ALL: [&[u8]; 5] = [INTER_REGULAR, INTER_MEDIUM, INTER_SEMIBOLD, MONO, ICONS];

/// The renderer settings every zengui surface is drawn with: the bundled
/// faces loaded, Inter as the default, and BODY as the default size — so a
/// widget that never names a size (a pick list's menu) still lands on the
/// scale rather than iced's 16.
pub fn settings() -> iced::Settings {
    iced::Settings {
        fonts: ALL.iter().map(|f| Cow::Borrowed(*f)).collect(),
        default_font: face::SANS,
        default_text_size: iced::Pixels(font::BODY),
        ..iced::Settings::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::kit::Icon;
    use crate::view::theme::{
        CoverageTone, PresenceTone, RegistrationTone, RungTone, SeverityTone, VerdictTone,
    };

    fn face_of(bytes: &[u8]) -> ttf_parser::Face<'_> {
        ttf_parser::Face::parse(bytes, 0).expect("a bundled face parses")
    }

    /// The family a face answers to, the way fontdb resolves it: the
    /// typographic family (name 16) when present, else the legacy one (1).
    fn family(face: &ttf_parser::Face<'_>) -> String {
        let names: Vec<_> = face.names().into_iter().collect();
        let by = |id| {
            names
                .iter()
                .filter(|n| n.name_id == id && n.is_unicode())
                .find_map(|n| n.to_string())
        };
        by(ttf_parser::name_id::TYPOGRAPHIC_FAMILY)
            .or_else(|| by(ttf_parser::name_id::FAMILY))
            .expect("a family name")
    }

    fn name_of(font: iced::Font) -> &'static str {
        match font.family {
            iced::font::Family::Name(n) => n,
            other => panic!("a bundled face is named, not {other:?}"),
        }
    }

    /// A `face::` constant that names a family no bundled file answers to
    /// silently falls back to a host font — the exact failure #533 exists to
    /// end. "lucide" vs "Lucide" is enough to cause it.
    #[test]
    fn every_face_constant_names_a_bundled_family() {
        assert_eq!(family(&face_of(INTER_REGULAR)), name_of(face::SANS));
        assert_eq!(family(&face_of(INTER_MEDIUM)), name_of(face::MEDIUM));
        assert_eq!(family(&face_of(INTER_SEMIBOLD)), name_of(face::SEMIBOLD));
        assert_eq!(family(&face_of(MONO)), name_of(face::MONO));
        assert_eq!(family(&face_of(ICONS)), name_of(face::ICONS));
    }

    /// The three Inter files are one family told apart by weight — which is
    /// what lets `face::MEDIUM` and `face::SEMIBOLD` differ only in
    /// `weight`.
    #[test]
    fn the_inter_weights_are_the_weights_the_constants_ask_for() {
        let weight = |b| face_of(b).weight().to_number();
        assert_eq!(weight(INTER_REGULAR), 400);
        assert_eq!(weight(INTER_MEDIUM), 500);
        assert_eq!(weight(INTER_SEMIBOLD), 600);
        assert_eq!(face::MEDIUM.weight, iced::font::Weight::Medium);
        assert_eq!(face::SEMIBOLD.weight, iced::font::Weight::Semibold);
    }

    fn covers(bytes: &[u8], s: &str) -> Vec<char> {
        let f = face_of(bytes);
        s.chars()
            .filter(|c| !c.is_whitespace() && f.glyph_index(*c).is_none())
            .collect()
    }

    /// Host independence, proven glyph by glyph: every badge glyph is drawn
    /// in Inter (the badges are sans captions), so Inter must carry each
    /// one — a glyph it lacked would fall back to whatever the host has,
    /// which is the honesty carrier rendered by luck.
    #[test]
    fn inter_carries_every_badge_glyph() {
        let mut glyphs = String::new();
        glyphs.extend(RegistrationTone::ALL.map(|t| t.glyph()));
        glyphs.extend(PresenceTone::ALL.map(|t| t.glyph()));
        glyphs.extend(SeverityTone::ALL.map(|t| t.glyph()));
        glyphs.extend(CoverageTone::ALL.map(|t| t.glyph()));
        glyphs.extend(VerdictTone::ALL.map(|t| t.glyph()));
        glyphs.extend(RungTone::ALL.map(|t| t.glyph()));
        // The sentences' own punctuation: separators, arrows, units.
        glyphs.push_str("·—…→←↑↓⏎µ§×~");
        assert_eq!(covers(INTER_REGULAR, &glyphs), Vec::<char>::new());
        assert_eq!(covers(INTER_SEMIBOLD, &glyphs), Vec::<char>::new());
    }

    /// The glyphs drawn in mono that Inter lacks: the history marker
    /// ("▸ t-1"), the mesh roll-call's storage mark ("⌂ storage"), and the
    /// box-drawing a payload may carry.
    #[test]
    fn the_mono_face_carries_the_glyphs_mono_text_draws() {
        assert_eq!(covers(MONO, "▸▾⌂◉●○✓✗·—…→←~"), Vec::<char>::new());
    }

    /// The icon font is cut to `kit::Icon` (scripts/subset-icons.py), so the
    /// enum and the cut must be the same set: an icon missing from the font
    /// draws as tofu, and one missing from the enum is dead weight.
    #[test]
    fn the_icon_font_is_exactly_kit_icon() {
        let manifest: std::collections::BTreeSet<&str> =
            include_str!("../../assets/fonts/lucide-icons.txt")
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .collect();
        let named: std::collections::BTreeSet<&str> =
            Icon::ALL.iter().map(|i| i.lucide_name()).collect();
        assert_eq!(named, manifest, "kit::Icon and lucide-icons.txt disagree");
        let font = face_of(ICONS);
        for icon in Icon::ALL {
            assert!(
                font.glyph_index(icon.codepoint()).is_some(),
                "{icon:?} ({}) is not in the cut font",
                icon.lucide_name()
            );
        }
    }

    /// Icons live in the Private Use Area and never collide — so no icon can
    /// ever be read as one of the badge glyphs, which are ordinary Unicode.
    #[test]
    fn icons_are_private_use_and_distinct() {
        let mut seen = std::collections::BTreeSet::new();
        for icon in Icon::ALL {
            let c = icon.codepoint();
            assert!(('\u{E000}'..='\u{F8FF}').contains(&c), "{icon:?}");
            assert!(seen.insert(c), "{icon:?} shares a codepoint");
        }
    }
}
