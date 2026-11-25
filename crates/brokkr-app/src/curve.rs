// SPDX-License-Identifier: AGPL-3.0-only

//! A curve drawn in a panel: how a brush's edge falls away, or how pen
//! pressure becomes strength.
//!
//! `docs/WORKSPACES.md` (stage U5) and issues 76 and 78 of the fork. A
//! falloff was five buttons with names on them and a pressure curve was a
//! number; a user had to try a stroke to see what either meant. This draws
//! them. It draws only: the curves are still set by those buttons and that
//! number.
//!
//! The samples are worked out by the caller, from the same functions the
//! brush and the pen use (`brokkr_core::brush::compound::Profile::weight`,
//! `tablet::shape`), so the drawing is the curve and not a picture of one.

use iced::widget::canvas;
use iced::{Color, Element, Length, Point, Renderer, Theme};

use crate::theme;

/// How many samples a curve is drawn from across its width.
pub const SAMPLES: usize = 64;

/// How tall a curve is drawn in a panel.
pub const HEIGHT: f32 = 56.0;

/// A curve through `points`, each `(x, y)` with both in `0..=1`, `y` up,
/// with the area under it tinted.
pub fn curve<'a, Message: 'a>(points: Vec<(f32, f32)>) -> Element<'a, Message> {
    canvas(Curve { points }).width(Length::Fill).height(Length::Fixed(HEIGHT)).into()
}

/// The points of a falloff's cross-section: the weight at each distance
/// from the middle, from one rim through the middle to the other.
pub fn cross_section(weight: impl Fn(f32) -> f32) -> Vec<(f32, f32)> {
    (0..=SAMPLES)
        .map(|at| {
            let x = at as f32 / SAMPLES as f32;
            let distance = (x * 2.0 - 1.0).abs();
            (x, weight(distance).clamp(0.0, 1.0))
        })
        .collect()
}

/// The points of a curve from 0 to 1: what each input becomes.
pub fn response(map: impl Fn(f32) -> f32) -> Vec<(f32, f32)> {
    (0..=SAMPLES)
        .map(|at| {
            let x = at as f32 / SAMPLES as f32;
            (x, map(x).clamp(0.0, 1.0))
        })
        .collect()
}

/// The canvas program. No `update`, so it never takes a press: see
/// `icon.rs`'s `IconProgram` for why that matters.
struct Curve {
    points: Vec<(f32, f32)>,
}

impl<Message> canvas::Program<Message> for Curve {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let inset = 3.0;
        let width = (bounds.width - inset * 2.0).max(1.0);
        let height = (bounds.height - inset * 2.0).max(1.0);
        let at = |(x, y): (f32, f32)| Point::new(inset + x * width, inset + (1.0 - y) * height);

        // The well it is drawn in, and the base line.
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), theme::BG_DEEP);
        let base = canvas::Path::line(at((0.0, 0.0)), at((1.0, 0.0)));
        frame.stroke(
            &base,
            canvas::Stroke::default().with_color(theme::LINE_STRONG).with_width(1.0),
        );

        let Some(first) = self.points.first().copied() else {
            return vec![frame.into_geometry()];
        };
        let line = canvas::Path::new(|builder| {
            builder.move_to(at(first));
            for point in &self.points[1..] {
                builder.line_to(at(*point));
            }
        });
        let under = canvas::Path::new(|builder| {
            builder.move_to(at((first.0, 0.0)));
            for point in &self.points {
                builder.line_to(at(*point));
            }
            if let Some(last) = self.points.last() {
                builder.line_to(at((last.0, 0.0)));
            }
            builder.close();
        });
        frame.fill(&under, Color { a: 0.14, ..theme::ACCENT });
        frame.stroke(
            &line,
            canvas::Stroke::default()
                .with_color(theme::ACCENT)
                .with_width(1.5)
                .with_line_join(canvas::LineJoin::Round),
        );
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// A cross-section is the weight at the distance from the middle,
        /// so it is the same either side of the middle, whatever the curve.
        #[test]
        fn a_cross_section_is_the_same_either_side(plateau in 0.0_f32..0.9, shape in 0usize..5) {
            let shape = brokkr_core::brush::compound::Shape::ALL[shape];
            let profile = brokkr_core::brush::compound::Profile { shape, plateau };
            let points = cross_section(|distance| profile.weight(distance));
            prop_assert_eq!(points.len(), SAMPLES + 1);
            for (left, right) in points.iter().zip(points.iter().rev()) {
                prop_assert!((left.1 - right.1).abs() < 1e-5, "{:?} against {:?}", left, right);
            }
            // Full in the middle and next to nothing at the rims. Not
            // nothing: Round with a flat middle gives about 4e-4 at the
            // rim, because the distance past the plateau comes out a hair
            // under 1 and Round's square root makes that much of it. Found
            // by this test; it is the brush's own arithmetic, and a stamp
            // that touches its rim voxel by 4e-4 of its strength changes
            // nothing anyone can see.
            prop_assert!((points[SAMPLES / 2].1 - 1.0).abs() < 1e-5);
            prop_assert!(points[0].1.abs() < 1e-3, "{}", points[0].1);
        }

        /// A pressure curve drawn is the pen's own: what it draws at each
        /// pressure is what `tablet::shape` gives, and it runs from 0 to 1.
        #[test]
        fn a_pressure_curve_is_the_pens(curve in 0.3_f32..3.0) {
            let points = response(|raw| crate::tablet::shape(raw, curve));
            for (x, y) in &points {
                prop_assert_eq!(*y, crate::tablet::shape(*x, curve).clamp(0.0, 1.0));
            }
            prop_assert_eq!(points[0].1, 0.0);
            prop_assert!((points[SAMPLES].1 - 1.0).abs() < 1e-6);
        }
    }
}
