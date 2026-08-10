//! Sparklines (issue #64) — the drawing, and nothing else.
//!
//! Every decision worth testing lives in [`crate::series`]; this file turns a
//! `Series` into a path. Two properties it must not lose:
//!
//! - **A gap is a break.** `None` ends the current stroke and the next
//!   measurement starts a new one. Bridging a gap draws data that was never
//!   observed, and on a rate series it specifically draws liveness.
//! - **Never colour alone.** Each chart ships with a caption naming its
//!   current, min and max — which is also the only part `iced_test` can see,
//!   the canvas being opaque to a find-by-text simulator.
//!
//! No `Cache`, and deliberately no `frames()`/animation subscription: the
//! canvas is rebuilt when the Elm loop rebuilds the view, i.e. on the 250 ms
//! bus tick. A permanently animating explorer is the thing zensight learned
//! not to ship.

use iced::widget::canvas;
use iced::{Element, Length, Point, Renderer, Theme, mouse};

use crate::message::Message;
use crate::series::Series;
use crate::view::kit;
use crate::view::theme::{SeriesTone, colors};
use crate::view::tokens::space;

/// The plot's height. Small on purpose — this is a shape, not a dashboard.
const HEIGHT: f32 = 44.0;

/// Line weight.
const STROKE: f32 = 1.5;

/// A labelled sparkline: caption, then the plot.
///
/// `unit` is appended to the caption's numbers when the registry declared one;
/// `rate` switches the caption to `kit::human_rate`, whose "—" for zero already
/// means "no measured rate" rather than "measured zero".
pub fn chart<'a>(
    label: &str,
    series: &Series,
    tone: SeriesTone,
    unit: Option<&str>,
) -> Element<'a, Message> {
    let caption = caption(label, series, tone, unit);
    let mut col = iced::widget::Column::new().spacing(2);
    col = col.push(kit::muted(caption));
    if series.has_data() {
        col = col.push(
            canvas(Spark {
                points: series.points().to_vec(),
                bounds: series.bounds(),
                tone,
            })
            .width(Length::Fill)
            .height(Length::Fixed(HEIGHT)),
        );
    }
    col.spacing(space::XS).into()
}

/// What the chart says in words — the whole of it, for a reader and for a test.
fn caption(label: &str, series: &Series, tone: SeriesTone, unit: Option<&str>) -> String {
    let fmt = |v: f64| match tone {
        SeriesTone::Rate => kit::human_rate(v),
        SeriesTone::Value => match unit {
            Some(u) => format!("{}{u}", trim(v)),
            None => trim(v),
        },
    };
    let Some((lo, hi)) = series.bounds() else {
        return format!(
            "{label}: no measurement in the window — {} sampled, all gaps",
            series.len()
        );
    };
    let now = series.last().map(fmt).unwrap_or_else(|| "—".to_string());
    let mut out = format!(
        "{label}: {now} now · {} min · {} max · {}",
        fmt(lo),
        fmt(hi),
        kit::plural(series.len(), "point"),
    );
    if series.gaps() > 0 {
        out.push_str(&format!(
            " · {} drawn as gaps, not interpolated",
            series.gaps()
        ));
    }
    if series.dropped() > 0 {
        out.push_str(&format!(
            " · {} scrolled out of the window",
            series.dropped()
        ));
    }
    out
}

/// Compact number rendering: integers stay integers.
fn trim(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{v:.0}")
    } else {
        format!("{v:.3}")
    }
}

struct Spark {
    points: Vec<Option<f64>>,
    bounds: Option<(f64, f64)>,
    tone: SeriesTone,
}

impl canvas::Program<Message> for Spark {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let (w, h) = (bounds.width, bounds.height);
        let Some((lo, hi)) = self.bounds else {
            return vec![frame.into_geometry()];
        };

        // A baseline, so a flat series is visibly *at* a level rather than
        // floating in an unlabelled box.
        let axis = canvas::Path::line(Point::new(0.0, h - 1.0), Point::new(w, h - 1.0));
        frame.stroke(
            &axis,
            canvas::Stroke::default()
                .with_width(1.0)
                .with_color(colors(theme).axis()),
        );

        // A constant series would divide by zero; centre it instead.
        let span = hi - lo;
        let y_of = |v: f64| {
            let t = if span.abs() < f64::EPSILON {
                0.5
            } else {
                (v - lo) / span
            };
            // Inset by the stroke so the extremes are not clipped.
            let usable = (h - STROKE * 2.0).max(1.0);
            (h - STROKE) - (t as f32) * usable
        };
        let x_of = |i: usize| {
            if self.points.len() <= 1 {
                w / 2.0
            } else {
                (i as f32) * w / (self.points.len() - 1) as f32
            }
        };

        let stroke = canvas::Stroke::default()
            .with_width(STROKE)
            .with_color(colors(theme).series(self.tone));

        // One path per contiguous run of measurements: the gaps between runs
        // are drawn by *not* drawing.
        let mut run: Vec<Point> = Vec::new();
        let flush = |run: &mut Vec<Point>, frame: &mut canvas::Frame| {
            match run.len() {
                0 => {}
                // A lone measurement between two gaps is a dot; a zero-length
                // line would render as nothing and lose the sample.
                1 => frame.fill(
                    &canvas::Path::circle(run[0], STROKE),
                    colors(theme).series(self.tone),
                ),
                _ => {
                    let path = canvas::Path::new(|b| {
                        b.move_to(run[0]);
                        for p in &run[1..] {
                            b.line_to(*p);
                        }
                    });
                    frame.stroke(&path, stroke);
                }
            }
            run.clear();
        };
        for (i, point) in self.points.iter().enumerate() {
            match point {
                Some(v) => run.push(Point::new(x_of(i), y_of(*v))),
                None => flush(&mut run, &mut frame),
            }
        }
        flush(&mut run, &mut frame);

        vec![frame.into_geometry()]
    }
}

/// The caption alone, for callers that want the words without the plot.
pub fn caption_text(label: &str, series: &Series, tone: SeriesTone, unit: Option<&str>) -> String {
    caption(label, series, tone, unit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series_of(points: &[Option<f64>]) -> Series {
        let mut s = Series::new();
        for p in points {
            match p {
                Some(v) => s.push(*v),
                None => s.push_gap(),
            }
        }
        s
    }

    /// The caption is the accessible surface: it carries the numbers, so the
    /// chart never means anything by colour alone — and it is the only part a
    /// find-by-text simulator can assert on.
    #[test]
    fn the_caption_carries_the_numbers_and_names_the_gaps() {
        let s = series_of(&[Some(1.0), None, Some(3.0)]);
        assert_eq!(
            caption_text("value", &s, SeriesTone::Value, Some("%")),
            "value: 3% now · 1% min · 3% max · 3 points · 1 drawn as gaps, not interpolated"
                .to_string()
        );
    }

    /// A window of nothing but gaps must not report a zero — "no measurement"
    /// and "measured zero" are different facts (`human_rate`'s own rule).
    #[test]
    fn an_all_gap_window_reports_no_measurement() {
        let s = series_of(&[None, None]);
        assert_eq!(
            caption_text("rate", &s, SeriesTone::Rate, None),
            "rate: no measurement in the window — 2 sampled, all gaps"
        );
    }

    #[test]
    fn a_rate_caption_uses_the_shared_rate_wording() {
        let s = series_of(&[Some(4.0)]);
        assert_eq!(
            caption_text("rate", &s, SeriesTone::Rate, None),
            "rate: 4.0/s now · 4.0/s min · 4.0/s max · 1 point"
        );
    }
}
