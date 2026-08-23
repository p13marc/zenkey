//! Design tokens: the typographic and spacing scales.
//!
//! Color tokens live in [`super::theme`] (theme-aware `ThemeColors`). This module
//! holds the *dimensional* tokens — font sizes and an 8pt spacing grid — so that
//! every view draws from one scale instead of ad-hoc literals. Use these instead
//! of bare `.size(13)` / `.padding(10)` / `.spacing(15)` calls.
//!
//! Type scale (5 steps) and an 8pt spacing scale.
//!
//! Ported verbatim from zensight's `src/view/tokens.rs`, tests included, so the
//! two GUIs in this ecosystem share one scale. The colorless-by-design split is
//! the point: it is what makes "no raw colors outside `theme`/`kit`" a rule a
//! grep can enforce.

/// Typographic scale (pixels). Five steps, used app-wide — via the role
/// constructors in `kit` (#191). `f32` so it feeds `.size(..)` (Iced
/// `Pixels`) directly.
pub mod font {
    /// Captions, labels, dense table cells, metadata.
    pub const CAPTION: f32 = 12.0;
    /// Default body text.
    pub const BODY: f32 = 14.0;
    /// Emphasis / card titles / key values.
    pub const EMPHASIS: f32 = 16.0;
    /// Section headers within a page.
    pub const SECTION: f32 = 20.0;
    /// Page title (one per screen).
    pub const TITLE: f32 = 24.0;
}

/// Spacing scale (pixels) on an 8pt grid. Use for `padding` and `spacing`.
/// `XS` (4) is reserved for tight icon/label gaps; everything else is a multiple
/// of 8. `f32` so it feeds `.padding(..)`/`.spacing(..)` directly.
///
/// **One rule, four numbers, no exceptions** (#192): `SM` inside a card, `MD`
/// between cards and for dock padding, `LG` between sections, `XL`
/// page-level. These constants are the *comfortable* grid — inside a
/// workspace dock, spacing comes off a [`Spacing`] resolved from the dock's
/// density, so the same four roles tighten together. The window chrome (the
/// location bar, the status strip, the palette) uses the constants directly:
/// it is one row tall and has no rows to trade for air.
pub mod space {
    /// Tight gap (icon↔label). Use sparingly.
    pub const XS: f32 = 4.0;
    /// Default gap between related elements — inside a card.
    pub const SM: f32 = 8.0;
    /// Gap between cards / dock padding.
    pub const MD: f32 = 16.0;
    /// Gap between sections.
    pub const LG: f32 = 24.0;
    /// Page-level padding / large separations.
    pub const XL: f32 = 32.0;
}

/// How much of the comfortable grid Compact keeps (#192). Three quarters:
/// enough to visibly trade air for rows, not enough to let adjacent rows
/// touch — and it divides the whole scale without leaving sub-pixel values
/// anywhere but a row height, where `f32` geometry absorbs it.
const COMPACT: f32 = 0.75;

/// The spacing grid, resolved for one density (#192) — the dimensional
/// sibling of `theme::colors`: a view never asks *which* density it is in,
/// it takes a `Spacing` and spends it.
///
/// The five fields are [`space`]'s five roles times the density's factor.
/// **Fonts are deliberately absent**: density multiplies the spacing grid and
/// the virtualized row heights ([`Spacing::row`]) and nothing else — shrinking
/// type is not density, it is illegibility. The type-scale gate (#191) keeps
/// every text size on `font::`, and `font::` does not take a density, so the
/// two scales cannot be coupled by accident.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spacing {
    /// Tight gap (icon↔label). Use sparingly.
    pub xs: f32,
    /// Inside a card.
    pub sm: f32,
    /// Between cards / dock padding.
    pub md: f32,
    /// Between sections.
    pub lg: f32,
    /// Page-level.
    pub xl: f32,
    factor: f32,
}

impl Spacing {
    /// Resolve the grid for a density. A dock resolves once, at its `pane`
    /// entry, from [`crate::prefs::DockRole::density`]; everything below it
    /// takes the resolved struct.
    pub fn of(density: crate::prefs::Density) -> Spacing {
        let factor = match density {
            crate::prefs::Density::Comfortable => 1.0,
            crate::prefs::Density::Compact => COMPACT,
        };
        Spacing {
            xs: space::XS * factor,
            sm: space::SM * factor,
            md: space::MD * factor,
            lg: space::LG * factor,
            xl: space::XL * factor,
            factor,
        }
    }

    /// A virtualized list's row height at this density: the *air* scales,
    /// the text never does — the row-height spelling of the same rule that
    /// keeps density off the type scale.
    ///
    /// `base` is the list's comfortable height (`ROW_HEIGHT` in `tree`/
    /// `history`/`echo`) and `text` the laid-out height of the type inside it
    /// ([`CAPTION_LINE`] per line). A bare `base * factor` would push Echo's
    /// 20px row under its own 15.6px line — compacting a row means shaving
    /// its padding, and a row that is already mostly text (History's
    /// two-liner) honestly compacts almost not at all.
    pub fn row(self, base: f32, text: f32) -> f32 {
        text + (base - text) * self.factor
    }
}

/// One caption line as laid out: [`font::CAPTION`] at iced's default 1.3
/// relative line height. The text floor of every virtualized row — what
/// [`Spacing::row`] refuses to squeeze.
pub const CAPTION_LINE: f32 = font::CAPTION * 1.3;

impl Default for Spacing {
    /// The comfortable grid — what the constants in [`space`] spell.
    fn default() -> Spacing {
        Spacing::of(crate::prefs::Density::Comfortable)
    }
}

#[cfg(test)]
mod tests {
    // These guard the ordering of compile-time design-token constants; the
    // assertions are deliberately constant-valued (they fail the build if the
    // scale is ever reordered).
    #![allow(clippy::assertions_on_constants)]

    use super::*;

    #[test]
    fn type_scale_is_monotonic() {
        assert!(font::CAPTION < font::BODY);
        assert!(font::BODY < font::EMPHASIS);
        assert!(font::EMPHASIS < font::SECTION);
        assert!(font::SECTION < font::TITLE);
    }

    #[test]
    fn spacing_is_on_8pt_grid() {
        // XS is the only sub-8 value; the rest are multiples of 8.
        for v in [space::SM, space::MD, space::LG, space::XL] {
            assert_eq!(v % 8.0, 0.0, "{v} is off the 8pt grid");
        }
        assert!(space::XS < space::SM);
    }

    use crate::prefs::Density;

    /// Comfortable *is* the scale: resolving it changes nothing, so every
    /// pre-#192 layout renders byte-identically at the default density.
    #[test]
    fn comfortable_is_the_constants() {
        let sp = Spacing::of(Density::Comfortable);
        assert_eq!(
            (sp.xs, sp.sm, sp.md, sp.lg, sp.xl),
            (space::XS, space::SM, space::MD, space::LG, space::XL)
        );
        assert_eq!(sp, Spacing::default());
        assert_eq!(sp.row(24.0, CAPTION_LINE), 24.0);
    }

    /// One factor for the whole grid — Compact tightens every role and the
    /// row heights by the same ratio, so the four roles stay four *ordered*
    /// roles at both densities.
    #[test]
    fn compact_scales_the_whole_grid_by_one_factor() {
        let c = Spacing::of(Density::Comfortable);
        let k = Spacing::of(Density::Compact);
        let f = k.sm / c.sm;
        assert!(f < 1.0, "compact must actually be tighter");
        for (comfortable, compact) in [
            (c.xs, k.xs),
            (c.sm, k.sm),
            (c.md, k.md),
            (c.lg, k.lg),
            (c.xl, k.xl),
        ] {
            assert_eq!(compact, comfortable * f, "one grid, one factor");
        }
        assert!(k.xs < k.sm && k.sm < k.md && k.md < k.lg && k.lg < k.xl);
    }

    /// A row compacts by its *air*, never below its text: the same factor
    /// scales the air, and a row that is mostly text barely moves — which is
    /// honest, not a bug.
    #[test]
    fn a_compact_row_shaves_air_and_never_squeezes_text() {
        let c = Spacing::of(Density::Comfortable);
        let k = Spacing::of(Density::Compact);
        // The three lists' real baselines: tree 24 (one line), echo 20 (one
        // line), history 34 (two lines).
        for (base, text) in [
            (24.0, CAPTION_LINE),
            (20.0, CAPTION_LINE),
            (34.0, 2.0 * CAPTION_LINE),
        ] {
            let compact = k.row(base, text);
            assert!(compact < base, "compact must be tighter than {base}");
            assert!(
                compact >= text,
                "a {base} row compacted below its {text} of text"
            );
            // The air scales by the grid's factor (to float precision).
            let air = compact - text;
            assert!(
                (air - (base - text) * (k.sm / c.sm)).abs() < 1e-3,
                "air {air} is not the factor-scaled remainder of {base}"
            );
        }
    }

    /// The acceptance's third clause (#192): font sizes are byte-identical
    /// across densities. Structurally pinned rather than screenshotted:
    /// [`Spacing`] has no font field to vary, `font::`'s constants take no
    /// density — the same values feed both modes — and the type-scale gate
    /// (`scripts/check-type-scale.sh`, #191) keeps every `.size(` in the
    /// crate on `font::`. A density that touched type would have to add a
    /// field here, in the one module that documents why it must not.
    #[test]
    fn density_never_reaches_the_type_scale() {
        // The whole type scale, spelled at both densities — trivially the
        // same constants, which is the point: there is no density-shaped way
        // to spell a font size.
        for _d in Density::ALL {
            assert_eq!(
                [
                    font::CAPTION,
                    font::BODY,
                    font::EMPHASIS,
                    font::SECTION,
                    font::TITLE
                ],
                [12.0, 14.0, 16.0, 20.0, 24.0]
            );
        }
    }
}
