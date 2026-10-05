//! The virtualized window the tree, echo and history share (#183, #360).

/// Rows rendered beyond each window edge, so scrolling never shows a gap.
pub(crate) const OVERSCAN: usize = 8;

/// The visible row window for a [`Viewport`]: `(first, last)` indices into
/// a fixed-height row list, with overscan.
///
/// Pure, so it is testable without a renderer — and shared, so the three
/// virtualized lists cannot drift into three different ideas of "visible".
/// The tree had this and nothing else did: Echo capped *drawing* at 300 rows
/// with no window, and History drew every retained entry (#183).
///
/// A list of `rows` rows is `rows * row_height` tall whether it is drawn or
/// not, which is what the spacers above and below the window are for. Callers
/// that forget them get a window that scrolls against the wrong extent.
/// Where a virtualized list is scrolled to, and how much of it is on screen
/// (#360).
///
/// The four lists carried this as a bare `(f32, f32)` in eight places while
/// [`TreeData`](crate::view::tree::TreeData) carried the same two numbers as
/// two named fields — so the pair had to be transposed correctly at every
/// seam, and "which one is the height" was answered by position.
///
/// [`Viewport::scrolled`] is the only way one arrives from a scrollable, and
/// it applies the floor the three handlers each used to apply by hand: a
/// height of zero (a pane laid out but never drawn) would make the window
/// empty and the list look like it had no rows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// Absolute offset from the top, in logical pixels.
    pub offset: f32,
    /// Visible height, in logical pixels.
    pub height: f32,
}

impl Viewport {
    /// The height a list assumes before iced has ever measured it.
    pub const DEFAULT_HEIGHT: f32 = 600.0;
    /// The smallest height a window is ever computed for.
    const MIN_HEIGHT: f32 = 100.0;

    /// What a scrollable just reported, floored.
    pub fn scrolled(offset: f32, height: f32) -> Viewport {
        Viewport {
            offset,
            height: height.max(Viewport::MIN_HEIGHT),
        }
    }

    /// The same viewport, back at the top — a new subject starts there rather
    /// than wherever the last one happened to be scrolled (#183).
    pub fn to_top(self) -> Viewport {
        Viewport {
            offset: 0.0,
            ..self
        }
    }
}

impl Default for Viewport {
    fn default() -> Viewport {
        Viewport {
            offset: 0.0,
            height: Viewport::DEFAULT_HEIGHT,
        }
    }
}

impl From<iced::widget::scrollable::Viewport> for Viewport {
    fn from(v: iced::widget::scrollable::Viewport) -> Viewport {
        Viewport::scrolled(v.absolute_offset().y, v.bounds().height)
    }
}

pub fn window(rows: usize, vp: Viewport, row_height: f32) -> (usize, usize) {
    let first = (vp.offset / row_height).floor() as usize;
    let visible = (vp.height / row_height).ceil() as usize + 1;
    let first = first.saturating_sub(OVERSCAN);
    let last = (first + visible + 2 * OVERSCAN).min(rows);
    (first.min(rows), last)
}
