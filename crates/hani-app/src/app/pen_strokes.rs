// SPDX-License-Identifier: AGPL-3.0-only

//! What a pen's stroke leaves, measured, and the one thing that was wrong
//! with it.
//!
//! Found by the maintainer on 2026-11-27, the day the pen first reached the
//! brush on a desktop of theirs (`pen.rs`): with Clay strips, "the build-up
//! setting lays down smoother than the 'one layer' setting. I see a lot more
//! 'noise' and grain in it. Also the 'build up' setting is often over-eager,
//! rapidly building up and sometimes shooting off into space." And of the
//! mouse: "It's a lot less than using the tablet. A lot lot less."
//!
//! # A plane is not tipped by the pen's lean
//!
//! *Tilt steers stroke* leans every stamp's normal the way the pen leans, by
//! up to sixty degrees ([`hani_core::lean_normal`]). For a brush that pushes
//! that is what it is for: the clay lands to the side the pen leans to. A
//! brush that levels to a plane, Clay, Flatten and everything made of them,
//! has its plane tipped by it, about each stamp's own centre. That was meant
//! to be "so a surface can be flattened at an angle", and it is not what
//! comes of it:
//!
//! * a stroke that lays **one layer** is a row of planes all tipped alike
//!   and each through its own stamp, a quarter of a radius apart: a row of
//!   wedges, where one plane was wanted. With a pen held as a hand holds
//!   one, forty degrees over, the layer was five times as rough as a
//!   mouse's;
//! * a stroke that **builds up** sets each stamp down on the face the last
//!   one tipped, takes the way that face looks for the surface's, and tips
//!   it again. Flatten, scribbled over a ball with the pen leaning across
//!   the stroke, stood 5.5 mm off it; with a mouse, 0.2.
//!
//! No pen is held upright. Until the pen came from the compositor, tilt
//! reached the brush only for a user in the `input` group, and nobody had
//! seen this.
//!
//! So [`Hani::stroke_lean`]: a brush that levels takes the surface's own
//! normal, whatever the pen does, and a brush that only pushes, inflates or
//! pinches is steered as before.
//!
//! # What was measured, and what was left
//!
//! By `measure_a_pen_as_a_hand_holds_it`, below, on the build machine (the
//! roughness is how far each height along a stroke's patch is from the mean
//! of the four a pixel round it, in thousandths of a millimetre; a stroke
//! of Clay strips, 3 mm, strength 0.3):
//!
//! | One layer | before | after |
//! |---|---|---|
//! | mouse | 0.9 | 0.9 |
//! | pen upright, pressed as a hand | 0.8 | 0.8 |
//! | pen as a hand, 40 degrees over | 4.3 to 5.6 | 0.7 to 0.8 |
//!
//! Flatten, building up, scribbled, the pen leaning across: 5.52 mm tall
//! before, 0.31 after, which is what an upright pen leaves.
//!
//! Tried and dropped: not laying a stamp whose centre is off the surface
//! (walked to through the air when the pointer's ray jumps in depth). It
//! changed no number of any stroke measured, so it is not what shot off.
//!
//! Pressure changes in steps too, one a stamp, and was measured not to
//! matter: a stamp is a quarter of a radius on and a pen reports a dozen
//! times in that.
//!
//! # How fast clay builds up: the strength sets it
//!
//! Left as it was by the change above, and decided by the maintainer on
//! 2026-11-27 (issue 179): "strength should affect build up". Clay held its
//! plane 0.35 of the radius off the surface as the surface is now, whatever
//! the strength, and nothing capped what one stamp lifted, where Draw is
//! held to a voxel a stamp. With a 3 mm brush at 0.25 mm voxels the plane
//! was 4.2 voxels up, a pass was 4.4 times as tall as Draw's at the same
//! strength, and from about a half up it ran away: that, and not only the
//! lean, was the "shooting off into space", and the mouse had it too.
//!
//! Now a stroke that builds up holds its plane the strength times the
//! pressure of that height off the surface, and never more than a voxel
//! off it (`hani_core`'s `brush/build_up.rs`). A stroke that lays one layer
//! is as it was. Measured with `measure_what_strength_does_to_a_pass`,
//! below, on the build machine: a mouse, Clay strips (Clay is the same to
//! the bit), 3 mm, the tallest the surface stands off the 30 mm ball, in
//! millimetres.
//!
//! | Strength | one pass, before | after | fifteen passes, before | after | Draw, one pass |
//! |---:|---:|---:|---:|---:|---:|
//! | 0.05 | 0.21 | 0.01 | 3.99 | 0.23 | 0.05 |
//! | 0.10 | 0.42 | 0.05 | 8.02 | 0.87 | 0.10 |
//! | 0.15 | 0.62 | 0.10 | 11.84 | 2.00 | 0.15 |
//! | 0.30 | 1.32 | 0.32 | 23.60 | 6.80 | 0.30 |
//! | 0.50 | 2.56 | 0.54 | 43.25 | 12.26 | 0.49 |
//! | 0.75 | 27.92 | 0.82 | 57.76 | 21.57 | 0.73 |
//! | 1.00 | 59.25 | 1.13 | 59.25 | 25.67 | 0.96 |
//!
//! One layer, before and after alike: 0.21, 0.42, 0.64, 1.06, 1.09, 1.10,
//! 1.12. A pass now rises with the strength all the way up, and from 0.3
//! up is within a fifth of Draw's. By arithmetic, with this brush the plane
//! reaches its cap of a voxel at a strength of 0.24 (4.2 voxels times 0.24
//! is one), and from there the blend's weight alone sets the lift, as
//! Draw's weight sets its push. Below that the plane's height and the
//! blend both follow the strength, so a pass goes as its square and is
//! thinner than Draw's: Clay at its own strength of 0.15 lays a tenth of a
//! millimetre a pass where it laid six tenths. Fifteen passes still stand
//! taller than Draw's (6.80 mm at 0.3, where Draw's stand 4.04). Why was
//! not looked into; the likeliest reading, not tested, is that clay fills
//! toward its plane at a ridge's flanks too, where a push only moves what
//! is there.
//!
//! Tried and dropped: capping the lift (the blend's weight times the
//! plane's height) at a voxel, and not the plane's height. One pass was
//! then 0.01, 0.05, 0.10, 0.40, 1.04, 1.08 and 1.13 mm at the strengths
//! above, fifteen at 0.3 stood 8.01: from a half up the slider did nothing
//! to how tall a pass is.

#[cfg(test)]
mod tests {
    use glam::{Vec2, Vec3};
    use iced::Vector;

    use crate::app::{Hani, MODEL_RADIUS_MM};
    use crate::message::{Message, PointerButton, PointerEvent, StudioMessage};
    use crate::tablet::{PenState, Tablet};
    use hani_core::BrushKind;

    const SIZE: Vector = Vector { x: 800.0, y: 600.0 };

    /// A number between -1 and 1 that is the same every run.
    fn noise(seed: u32) -> f32 {
        let mut x = seed.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
        x ^= x >> 15;
        x = x.wrapping_mul(0x2C1B_3C6D);
        x ^= x >> 12;
        x = x.wrapping_mul(0x297A_2D39);
        x ^= x >> 15;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    fn held(pressure: f32, tilt: Vec2) -> PenState {
        PenState { in_proximity: true, pressure, eraser: false, tilt }
    }

    /// A pen held as a hand holds one: forty degrees over, up the screen
    /// and to the right.
    const AS_A_HAND: Vec2 = Vec2::new(0.5, 0.45);

    fn choose(app: &mut Hani, name: &str) {
        let index = app
            .studio
            .presets
            .palette()
            .iter()
            .position(|(preset, _)| preset.name == name)
            .unwrap_or_else(|| panic!("there is no brush called {name}"));
        drop(app.update(Message::Studio(StudioMessage::PresetChosen(index))));
    }

    /// One stroke of `brush` along `path`, in pixels, the pen held as `pen`
    /// says at each sample, and no pen at all where it says none.
    fn drawn(
        brush: &str,
        build_up: bool,
        path: &[Vec2],
        pen: &dyn Fn(usize) -> Option<PenState>,
    ) -> Hani {
        drawn_at(brush, build_up, None, path, pen)
    }

    /// The same, at `strength` where it says one and at the brush's own
    /// where it does not.
    fn drawn_at(
        brush: &str,
        build_up: bool,
        strength: Option<f32>,
        path: &[Vec2],
        pen: &dyn Fn(usize) -> Option<PenState>,
    ) -> Hani {
        let mut app = Hani::with_tablet(Tablet::inert());
        app.viewport_size = Vec2::new(SIZE.x, SIZE.y);
        choose(&mut app, brush);
        drop(app.update(Message::Studio(StudioMessage::BuildUpToggled(build_up))));
        if let Some(strength) = strength {
            drop(app.update(Message::BrushStrengthChanged(strength)));
        }
        let at = |index: usize| Vector::new(path[index].x, path[index].y);
        if let Some(state) = pen(0) {
            app.tablet.simulate(state);
        }
        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: at(0),
            size: SIZE,
        });
        for index in 1..path.len() {
            if let Some(state) = pen(index) {
                app.tablet.simulate(state);
            }
            app.on_pointer(PointerEvent::Moved { position: at(index), size: SIZE });
        }
        app.on_pointer(PointerEvent::Released { button: PointerButton::Left });
        app
    }

    /// How high the surface stands over the ball under a pixel, in
    /// millimetres.
    fn height(app: &Hani, x: f32, y: f32) -> Option<f32> {
        app.surface_under(Vec2::new(x, y)).map(|point| point.length() - MODEL_RADIUS_MM)
    }

    /// How rough the patch a stroke covered is, in thousandths of a
    /// millimetre, and the tallest it stands, in millimetres.
    fn patch(app: &Hani) -> (f32, f32) {
        let (mut rough, mut count, mut most) = (0.0, 0, 0.0_f32);
        for y in (270..=330).step_by(2) {
            for x in (330..=470).step_by(2) {
                let (x, y) = (x as f32, y as f32);
                let (Some(h), Some(a), Some(b), Some(c), Some(d)) = (
                    height(app, x, y),
                    height(app, x - 1.0, y),
                    height(app, x + 1.0, y),
                    height(app, x, y - 1.0),
                    height(app, x, y + 1.0),
                ) else {
                    continue;
                };
                most = most.max(h);
                // Only where the stroke was: the ball itself is smooth.
                if h < 0.05 {
                    continue;
                }
                rough += (h - (a + b + c + d) / 4.0).abs();
                count += 1;
            }
        }
        (rough / count.max(1) as f32 * 1000.0, most)
    }

    /// The tallest the surface stands anywhere a stroke could have reached.
    fn tallest(app: &Hani) -> f32 {
        let mut most = 0.0_f32;
        for y in (240..=360).step_by(4) {
            for x in (240..=560).step_by(4) {
                most = most.max(height(app, x as f32, y as f32).unwrap_or(0.0));
            }
        }
        most
    }

    fn straight() -> Vec<Vec2> {
        (0..200).map(|i| Vec2::new(300.0 + i as f32, 300.0)).collect()
    }

    /// Over the same ground `passes` times, as a hand filling in does.
    fn scribble(passes: usize) -> Vec<Vec2> {
        (0..passes * 60)
            .map(|i| {
                let t = (i as f32 / 60.0 * std::f32::consts::PI).sin();
                let shake = Vec2::new(noise(i as u32 + 100), noise(i as u32 + 200));
                Vec2::new(400.0 + 40.0 * t, 300.0) + shake
            })
            .collect()
    }

    /// Whether two strokes left the same field, to the bit, where either
    /// could have reached.
    fn same_field(one: &Hani, other: &Hani) -> bool {
        (240..=360).step_by(3).all(|y| {
            (240..=560).step_by(3).all(|x| {
                let pixel = Vec2::new(x as f32, y as f32);
                one.surface_under(pixel) == other.surface_under(pixel)
            })
        })
    }

    /// The fault as it was found: Clay strips, one layer, under a pen held
    /// as a hand holds one.
    #[test]
    fn a_leaning_pen_lays_the_layer_an_upright_one_does() {
        let upright = drawn("Clay strips", false, &straight(), &|_| Some(held(0.6, Vec2::ZERO)));
        let leaning = drawn("Clay strips", false, &straight(), &|_| Some(held(0.6, AS_A_HAND)));
        assert!(same_field(&upright, &leaning), "the lean reached the layer's plane");
        let (rough, _) = patch(&leaning);
        let (of_a_mouse, _) = patch(&drawn("Clay strips", false, &straight(), &|_| None));
        assert!(
            rough <= of_a_mouse * 1.5,
            "a pen's layer is {rough} rough and a mouse's {of_a_mouse}: it was five times"
        );
    }

    /// And the one that shot off: each stamp set down on the face the last
    /// one tipped, and tipped again.
    ///
    /// Four passes and two brushes, where the measurement below takes
    /// fifteen and four: this is run at every merge, and at fifteen passes
    /// and three brushes it was the sixth slowest test of the eleven hundred
    /// (73 s of the suite's time, measured). What it holds does not need
    /// them: the two fields are the same to the bit or they are not, from
    /// the first stamp on.
    #[test]
    fn a_leaning_pen_does_not_raise_what_it_flattens() {
        let across = Vec2::new(0.0, 0.67);
        let path = scribble(4);
        for brush in ["Flatten", "Clay strips"] {
            let upright = drawn(brush, true, &path, &|_| Some(held(0.6, Vec2::ZERO)));
            let leaning = drawn(brush, true, &path, &|_| Some(held(0.6, across)));
            assert!(same_field(&upright, &leaning), "{brush}: the lean reached its plane");
            if brush == "Flatten" {
                assert!(
                    tallest(&leaning) < 0.5,
                    "Flatten stood {} mm off a ball it was flattening",
                    tallest(&leaning)
                );
            }
        }
    }

    /// What tilt is for is kept: a brush that pushes is steered by it.
    /// (One that inflates never was: it moves the surface along its own
    /// normals and asks nothing of the stamp's.)
    #[test]
    fn a_leaning_pen_still_steers_a_brush_that_pushes() {
        let upright = drawn("Draw", true, &straight(), &|_| Some(held(0.6, Vec2::ZERO)));
        let leaning = drawn("Draw", true, &straight(), &|_| Some(held(0.6, AS_A_HAND)));
        assert!(!same_field(&upright, &leaning), "Draw took no notice of the pen's lean");
    }

    #[test]
    fn the_lean_is_kept_from_every_brush_that_levels_and_from_no_other() {
        let lean = Vec3::new(0.3, 0.2, 0.0);
        let mut app = Hani::with_tablet(Tablet::inert());
        for (kind, steered) in [
            (BrushKind::Draw, true),
            (BrushKind::Inflate, true),
            (BrushKind::Pinch, true),
            (BrushKind::Smooth, true),
            (BrushKind::Clay, false),
            (BrushKind::Flatten, false),
        ] {
            drop(app.update(Message::BrushKindChanged(kind)));
            let kept = app.stroke_lean(lean);
            assert_eq!(kept, if steered { lean } else { Vec3::ZERO }, "{kind:?}");
        }
        // A brush of the palette's that levels by amounts of its own.
        choose(&mut app, "Scrape");
        assert_eq!(app.stroke_lean(lean), Vec3::ZERO, "Scrape levels");
        // Shift is Smooth, whatever is in hand, and Smooth has no plane.
        drop(app.update(Message::BrushKindChanged(BrushKind::Clay)));
        app.shift = true;
        assert_eq!(app.stroke_lean(lean), lean, "Smooth under shift");
    }

    /// How tall a pass stands for each strength, building up and laying one
    /// layer. `cargo test -p hani-app measure_what_strength -- --ignored
    /// --nocapture`.
    /// Half as long as `straight`, which is all a pass needs to stand as
    /// tall as it will: the tallest is in the middle of it, and these tests
    /// run at every merge.
    fn short() -> Vec<Vec2> {
        (0..100).map(|i| Vec2::new(350.0 + i as f32, 300.0)).collect()
    }

    /// The tallest one short pass of `brush` leaves at `strength`, a mouse
    /// or a pen held upright and pressed `pressed`.
    fn pass(brush: &str, build_up: bool, strength: f32, pressed: Option<f32>) -> f32 {
        let pen = move |_: usize| pressed.map(|pressure| held(pressure, Vec2::ZERO));
        tallest(&drawn_at(brush, build_up, Some(strength), &short(), &pen))
    }

    /// Issue 179, as the maintainer decided it on 2026-11-27: "strength
    /// should affect build up". A pass of clay that builds up is taller
    /// the stronger it is, all the way up, and no strength runs away. It
    /// did: one pass at 0.75 stood 28 mm off the 30 mm ball, and 0.5 and
    /// 1.0 were 2.6 and 59 mm.
    #[test]
    fn building_up_the_strength_sets_how_tall_a_pass_of_clay_is() {
        let radius = {
            let mut app = Hani::with_tablet(Tablet::inert());
            choose(&mut app, "Clay strips");
            app.brush.radius
        };
        let mut last = 0.0;
        for strength in [0.1, 0.3, 0.5, 0.8, 1.0] {
            let tall = pass("Clay strips", true, strength, None);
            assert!(tall > last, "a pass at {strength} stood {tall} mm, one weaker {last}");
            assert!(tall < radius, "a pass at {strength} stood {tall} mm off a {radius} mm brush");
            last = tall;
        }
    }

    /// And the pen's pressure is a share of the strength, as it always
    /// was: a light pen lays a thinner pass.
    #[test]
    fn half_the_pressure_lays_less_clay_than_full() {
        let (light, full) =
            (pass("Clay strips", true, 0.5, Some(0.5)), pass("Clay strips", true, 0.5, Some(1.0)));
        assert!(0.0 < light && light < full, "half the pressure {light} mm, full {full}");
    }

    /// One layer is not changed by any of it: Clay strips at its own 0.3
    /// lays the 1.06 mm it did before (`measure_what_strength_does_to_a_pass`,
    /// before the change, along the same line).
    #[test]
    fn one_layer_of_clay_is_as_tall_as_it_was() {
        let tall = tallest(&drawn_at("Clay strips", false, Some(0.3), &straight(), &|_| None));
        assert!((tall - 1.06).abs() < 0.01, "one layer of Clay strips stood {tall} mm");
    }

    #[test]
    #[ignore = "a measurement, printed"]
    fn measure_what_strength_does_to_a_pass() {
        let at = |brush: &str, build_up: bool, strength: f32, path: &[Vec2]| {
            tallest(&drawn_at(brush, build_up, Some(strength), path, &|_| None))
        };
        for brush in ["Clay strips", "Clay", "Draw"] {
            for build_up in [true, false] {
                println!(
                    "--- {brush}, build up {build_up}: strength, one pass (mm), five passes, fifteen"
                );
                for strength in [0.05, 0.1, 0.15, 0.3, 0.5, 0.75, 1.0] {
                    println!(
                        "{strength:5.2}  {:6.2}  {:6.2}  {:6.2}",
                        at(brush, build_up, strength, &straight()),
                        at(brush, build_up, strength, &scribble(5)),
                        at(brush, build_up, strength, &scribble(15)),
                    );
                }
            }
        }
    }

    /// The instrument the module's table is from. `cargo test -p hani-app
    /// measure_a_pen -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement, printed"]
    fn measure_a_pen_as_a_hand_holds_it() {
        let hand: Vec<Vec2> = (0..400)
            .map(|i| {
                let x = 300.0 + i as f32 * 0.5;
                let shake = Vec2::new(noise(i as u32 + 100), noise(i as u32 + 200)) * 0.8;
                Vec2::new(x, 300.0 + 6.0 * (i as f32 / 40.0).sin()) + shake
            })
            .collect();
        // Pressure that comes on, wavers and goes off, as a stroke's does.
        let pressed = |i: usize| {
            let swell = (i as f32 / 400.0 * std::f32::consts::PI).sin().powf(0.5);
            (0.15 + 0.6 * swell + 0.04 * noise(i as u32)).clamp(0.02, 1.0)
        };
        type Pen = Box<dyn Fn(usize) -> Option<PenState>>;
        let pens: Vec<(&str, Pen)> = vec![
            ("mouse", Box::new(|_| None)),
            ("pen upright, steady 0.6", Box::new(|_| Some(held(0.6, Vec2::ZERO)))),
            (
                "pen upright, pressed as a hand",
                Box::new(move |i| Some(held(pressed(i), Vec2::ZERO))),
            ),
            ("pen leaning 40 along, 0.6", Box::new(|_| Some(held(0.6, Vec2::new(0.67, 0.0))))),
            ("pen leaning 40 across, 0.6", Box::new(|_| Some(held(0.6, Vec2::new(0.0, 0.67))))),
            (
                "pen as a hand: 40 over, wavering",
                Box::new(move |i| {
                    let shake = Vec2::new(noise(i as u32 + 300), noise(i as u32 + 400)) * 0.03;
                    Some(held(pressed(i), AS_A_HAND + shake))
                }),
            ),
        ];
        for brush in ["Clay strips", "Clay", "Flatten", "Draw"] {
            for build_up in [false, true] {
                println!("--- {brush}, build up {build_up}: rough (um) / tallest (mm)");
                for (name, pen) in &pens {
                    let a = patch(&drawn(brush, build_up, &straight(), pen));
                    let b = patch(&drawn(brush, build_up, &hand, pen));
                    let c = tallest(&drawn(brush, build_up, &scribble(15), pen));
                    println!(
                        "{name:34} straight {:5.1}/{:5.2}  hand {:5.1}/{:5.2}  scribbled {c:6.2}",
                        a.0, a.1, b.0, b.1
                    );
                }
            }
        }
    }
}
