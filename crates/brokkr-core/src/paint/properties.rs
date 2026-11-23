// SPDX-License-Identifier: AGPL-3.0-only

//! Rules of the painting engine, each stated for every input and tried on
//! a few hundred drawn at random.
//!
//! The tests beside each module say what it does on fixtures chosen by
//! hand, with the numbers worked out. These say what must hold whatever
//! the fixture: the things a hand-picked case is good at missing, because
//! whoever picked it had the same idea of the input as whoever wrote the
//! code. A rule that fails is shrunk to the smallest input that breaks it
//! and printed.
//!
//! The contracts on the functions themselves (`contracts::debug_ensures`
//! and its kin) are checked on every one of these runs as well, since a
//! test build is a debug build unless it is asked not to be.

use glam::{IVec3, Vec3};
use proptest::prelude::*;

use super::field::{BARE, PaintBrick, PaintField, Rgba, stored};
use super::file::{decode, encode};
use super::stroke::{Catch, Lay, Mode, PaintStroke, Pigment, laid};
use super::touch::Sponge;
use crate::brick::{BRICK_VOXELS, BrickCoord};
use crate::brush::{Brush, BrushDirection, BrushKind, Stamp};
use crate::orientation::AxisRotation;
use crate::volume::Volume;

/// Paint as a voxel may hold it.
fn rgba() -> impl Strategy<Value = Rgba> {
    any::<[u8; 4]>().prop_map(stored)
}

fn mode() -> impl Strategy<Value = Mode> {
    prop_oneof![Just(Mode::Paint), Just(Mode::Glaze), Just(Mode::Erase)]
}

fn pigment() -> impl Strategy<Value = Pigment> {
    (any::<[u8; 3]>(), 0.0_f32..=1.0, mode()).prop_map(|(colour, opacity, mode)| Pigment {
        colour,
        opacity,
        mode,
    })
}

/// A brick of paint: one value, or a scatter of voxels over one value.
fn brick() -> impl Strategy<Value = PaintBrick> {
    let scattered = (rgba(), prop::collection::vec((0..BRICK_VOXELS, rgba()), 1..80)).prop_map(
        |(base, voxels)| {
            let mut brick = PaintBrick::dense_filled(base);
            let data = brick.make_dense();
            for (index, paint) in voxels {
                data[index] = paint;
            }
            brick
        },
    );
    prop_oneof![rgba().prop_map(PaintBrick::Uniform), scattered]
}

/// A field of a few bricks, stored the way an edit would have left it.
fn field() -> impl Strategy<Value = PaintField> {
    let coord = (-3..3_i32, -3..3_i32, -3..3_i32).prop_map(|(x, y, z)| BrickCoord::new(x, y, z));
    prop::collection::vec((coord, brick()), 0..5).prop_map(|bricks| {
        let mut field = PaintField::default();
        for (coord, brick) in bricks {
            field.insert(coord, brick);
        }
        field.collapse();
        field
    })
}

/// The 24 turns that take the lattice onto itself.
fn turn() -> impl Strategy<Value = AxisRotation> {
    let mut turns = Vec::new();
    let axes = [IVec3::X, IVec3::Y, IVec3::Z];
    for order in [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
        for signs in 0..8 {
            let sign = |bit: usize| if signs & (1 << bit) == 0 { 1 } else { -1 };
            let columns =
                [axes[order[0]] * sign(0), axes[order[1]] * sign(1), axes[order[2]] * sign(2)];
            turns.extend(AxisRotation::from_columns(columns));
        }
    }
    assert_eq!(turns.len(), 24, "the fixture: there are 24 turns of a cube");
    prop::sample::select(turns)
}

/// The same paint in the same places, however each stores it.
fn same(one: &PaintField, other: &PaintField) -> bool {
    one.bricks() == other.bricks()
}

proptest! {
    /// Nothing laid is what was there, to the bit: no amount of it, or no
    /// opacity.
    #[test]
    fn nothing_laid_changes_nothing(before in rgba(), pigment in pigment()) {
        prop_assert_eq!(laid(before, pigment, 0.0), before);
        prop_assert_eq!(laid(before, Pigment { opacity: 0.0, ..pigment }, 1.0), before);
    }

    /// What is laid is stored as paint is stored, and coverage moves one
    /// way: paint and glaze never take any off, an eraser never adds any
    /// and never changes the colour of what it leaves.
    #[test]
    fn coverage_moves_one_way(before in rgba(), pigment in pigment(), amount in 0.0_f32..=1.0) {
        let after = laid(before, pigment, amount);
        prop_assert_eq!(after, stored(after));
        match pigment.mode {
            Mode::Erase => {
                prop_assert!(after[3] <= before[3]);
                if after[3] > 0 {
                    prop_assert_eq!(&after[..3], &before[..3]);
                }
            }
            Mode::Paint | Mode::Glaze => prop_assert!(after[3] >= before[3]),
        }
    }

    /// More of a stroke is never less of its effect on the coverage.
    #[test]
    fn more_is_never_less(
        before in rgba(),
        pigment in pigment(),
        one in 0.0_f32..=1.0,
        other in 0.0_f32..=1.0,
    ) {
        let less = laid(before, pigment, one.min(other))[3];
        let more = laid(before, pigment, one.max(other))[3];
        match pigment.mode {
            Mode::Erase => prop_assert!(more <= less),
            Mode::Paint | Mode::Glaze => prop_assert!(more >= less),
        }
    }

    /// All of a paint, at full opacity, is the paint: whatever was there.
    #[test]
    fn a_full_coat_is_the_colour(before in rgba(), colour in any::<[u8; 3]>()) {
        let all = laid(before, Pigment { colour, opacity: 1.0, mode: Mode::Paint }, 1.0);
        prop_assert_eq!(all, [colour[0], colour[1], colour[2], 255]);
    }

    /// The opacity is a scale on the amount and nothing else.
    #[test]
    fn opacity_scales_the_amount(before in rgba(), pigment in pigment(), amount in 0.0_f32..=1.0) {
        let at_full = Pigment { opacity: 1.0, ..pigment };
        prop_assert_eq!(laid(before, pigment, amount), laid(before, at_full, amount * pigment.opacity));
    }

    /// A glaze over a full coat hides none of it and lightens none of it.
    #[test]
    fn a_glaze_never_lightens(
        under in any::<[u8; 3]>(),
        colour in any::<[u8; 3]>(),
        amount in 0.0_f32..=1.0,
    ) {
        let glaze = Pigment { colour, opacity: 1.0, mode: Mode::Glaze };
        let after = laid([under[0], under[1], under[2], 255], glaze, amount);
        prop_assert_eq!(after[3], 255);
        for channel in 0..3 {
            prop_assert!(after[channel] <= under[channel], "channel {}: {:?}", channel, after);
        }
    }

    /// Paint comes back from a file's section as it went in, for any
    /// bodies' worth of it, and the same paint is always the same bytes.
    #[test]
    fn a_section_holds_what_was_put_in_it(fields in prop::collection::vec(field(), 1..4)) {
        match encode(fields.iter()) {
            None => prop_assert!(fields.iter().all(PaintField::is_empty)),
            Some(section) => {
                let again = encode(fields.iter());
                prop_assert_eq!(again.as_ref(), Some(&section));
                let back = decode(&section, fields.len());
                prop_assert!(back.is_ok(), "a section just written: {:?}", back.err());
                for (read, written) in back.unwrap_or_default().iter().zip(&fields) {
                    prop_assert!(same(read, written));
                }
            }
        }
    }

    /// No bytes at all make the reader do anything but answer.
    #[test]
    fn any_bytes_get_an_answer(
        bytes in prop::collection::vec(any::<u8>(), 0..400),
        bodies in 0_usize..4,
    ) {
        if let Ok(fields) = decode(&bytes, bodies) {
            prop_assert_eq!(fields.len(), bodies);
        }
    }

    /// Nor does a real section with a few of its bytes changed, which gets
    /// much further into the reader than noise does.
    #[test]
    fn a_damaged_section_gets_an_answer(
        field in field(),
        damage in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..4),
    ) {
        if let Some(mut section) = encode(std::iter::once(&field)) {
            for (at, byte) in damage {
                let at = at.index(section.len());
                section[at] = byte;
            }
            if let Ok(fields) = decode(&section, 1) {
                prop_assert_eq!(fields.len(), 1);
            }
        }
    }

    /// A body moved and moved back has the paint it had, by any number of
    /// voxels in any direction.
    #[test]
    fn there_and_back_is_no_move(
        field in field(),
        offset in (-70..70_i32, -70..70_i32, -70..70_i32),
    ) {
        let offset = IVec3::new(offset.0, offset.1, offset.2);
        let back = field.shifted(offset).shifted(-offset);
        prop_assert!(same(&back, &field));
    }

    /// A paint is where its voxel went: after a move, every voxel holds
    /// what the voxel it came from held.
    #[test]
    fn a_move_takes_each_voxel_with_it(
        field in field(),
        offset in (-70..70_i32, -70..70_i32, -70..70_i32),
        probes in prop::collection::vec((-96..96_i32, -96..96_i32, -96..96_i32), 1..40),
    ) {
        let offset = IVec3::new(offset.0, offset.1, offset.2);
        let moved = field.shifted(offset);
        for (x, y, z) in probes {
            let cell = IVec3::new(x, y, z);
            prop_assert_eq!(moved.at(cell + offset), field.at(cell));
        }
    }

    /// A body turned and turned back has the paint it had, for each of the
    /// 24 turns.
    #[test]
    fn a_turn_and_its_inverse_is_no_turn(field in field(), turn in turn()) {
        let back = field.rotated(turn).rotated(turn.inverse());
        prop_assert!(same(&back, &field));
    }

    /// And after a turn every voxel holds what the voxel it came from
    /// held.
    #[test]
    fn a_turn_takes_each_voxel_with_it(
        field in field(),
        turn in turn(),
        probes in prop::collection::vec((-96..96_i32, -96..96_i32, -96..96_i32), 1..40),
    ) {
        let turned = field.rotated(turn);
        for (x, y, z) in probes {
            let cell = IVec3::new(x, y, z);
            prop_assert_eq!(turned.at(turn.apply_voxel(cell)), field.at(cell));
        }
    }
}

/// A ball to paint on, and a stamp on its top at `(x, y)`.
fn ball() -> Volume {
    let mut volume = Volume::new(0.5);
    volume.seed_sphere(Vec3::ZERO, 10.0);
    volume
}

fn on_top(x: f32, y: f32, pressure: f32) -> Stamp {
    let centre = Vec3::new(x, y, (100.0 - x * x - y * y).sqrt());
    Stamp::new(centre, centre.normalize(), BrushDirection::Add).with_pressure(pressure)
}

/// What a brush with no direction catches. A drybrush is left out: its
/// bristles lie along the stroke, so which way a path was walked is part
/// of what it leaves, and a real one is the same.
fn catch() -> impl Strategy<Value = Catch> {
    prop_oneof![Just(Catch::All), Just(Catch::Recesses), Just(Catch::Specks(Sponge::default()))]
}

proptest! {
    // Each case paints a ball several times over: fewer of them.
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// A stroke is a layer over the path it took: the same paint whichever
    /// way the path is walked, and walked there and back again in one
    /// stroke, for any pigment over any paint, any pressures along it, and
    /// anything a brush with no direction catches.
    #[test]
    fn a_path_is_the_same_paint_walked_either_way(
        stamps in prop::collection::vec((-4.0_f32..4.0, -4.0_f32..4.0, 0.1_f32..=1.0), 1..7),
        radius in 1.0_f32..4.0,
        pigment in pigment(),
        catch in catch(),
        under in rgba(),
    ) {
        let brush = Brush { kind: BrushKind::Paint, radius, ..Brush::default() };
        let lay = Lay { catch, seed: 3, crease_gain: 4.0, ..Lay::loaded(pigment) };
        let stamps: Vec<Stamp> = stamps.iter().map(|(x, y, p)| on_top(*x, *y, *p)).collect();
        let painted = |order: &mut dyn Iterator<Item = &Stamp>| {
            let mut volume = ball();
            volume.edit_paint(IVec3::splat(-30), IVec3::splat(30), |_, _, _, _| under);
            let mut stroke = PaintStroke::new();
            volume.begin_stroke();
            stroke.begin();
            for stamp in order {
                stroke.stamp(&mut volume, &brush, stamp, &lay);
            }
            let _ = volume.end_stroke();
            volume
        };
        let forwards = painted(&mut stamps.iter());
        let backwards = painted(&mut stamps.iter().rev());
        let twice = painted(&mut stamps.iter().chain(stamps.iter().rev()));
        prop_assert!(same(forwards.paint(), backwards.paint()), "the direction mattered");
        prop_assert!(same(forwards.paint(), twice.paint()), "walking it back again mattered");
    }

    /// A stroke undone is the paint that was there, and done again is the
    /// stroke.
    #[test]
    fn a_stroke_undoes_to_what_was_there(
        stamps in prop::collection::vec((-4.0_f32..4.0, -4.0_f32..4.0, 0.1_f32..=1.0), 1..5),
        pigment in pigment(),
        under in rgba(),
    ) {
        let brush = Brush { kind: BrushKind::Paint, radius: 3.0, ..Brush::default() };
        let mut volume = ball();
        volume.edit_paint(IVec3::splat(-30), IVec3::splat(30), |_, _, _, _| under);
        let before = ball_paint(&volume);
        let mut stroke = PaintStroke::new();
        volume.begin_stroke();
        stroke.begin();
        for (x, y, pressure) in &stamps {
            stroke.stamp(&mut volume, &brush, &on_top(*x, *y, *pressure), &Lay::loaded(pigment));
        }
        let after = ball_paint(&volume);
        match volume.end_stroke() {
            // A stroke that changed nothing is no entry.
            None => prop_assert_eq!(&after, &before),
            Some(edit) => {
                let redo = volume.apply_edit(edit);
                prop_assert_eq!(&ball_paint(&volume), &before);
                volume.apply_edit(redo);
                prop_assert_eq!(&ball_paint(&volume), &after);
            }
        }
    }
}

/// Every voxel's paint in the box the ball is in.
fn ball_paint(volume: &Volume) -> Vec<Rgba> {
    let mut paint = Vec::new();
    for z in -26..=26 {
        for y in -26..=26 {
            for x in -26..=26 {
                paint.push(volume.paint().at(IVec3::new(x, y, z)));
            }
        }
    }
    debug_assert!(paint.iter().any(|rgba| *rgba != BARE) || volume.paint().is_empty());
    paint
}
