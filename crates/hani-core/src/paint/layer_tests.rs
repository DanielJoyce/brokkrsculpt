// SPDX-License-Identifier: AGPL-3.0-only

//! Layers on a body: what a stroke, an undo, a move, a split and the file
//! do to a stack. See [`super::layers`].

use glam::{IVec3, Vec3};
use proptest::prelude::*;

use super::field::{BARE, PaintField, Rgba};
use super::layers::{Blend, LayerMeta, composite_voxel};
use crate::body::Document;
use crate::undo::{Change, Entry, History};
use crate::volume::Volume;

const RED: Rgba = [200, 30, 20, 255];
const BLUE: Rgba = [20, 40, 210, 128];
const ON: IVec3 = IVec3::new(12, 0, 0);

fn ball() -> Volume {
    let mut volume = Volume::new(1.0);
    volume.seed_sphere(Vec3::ZERO, 12.0);
    volume
}

fn coat(volume: &mut Volume, rgba: Rgba) {
    volume.edit_paint(IVec3::splat(-20), IVec3::splat(20), |_, _, _, _| rgba);
}

/// Each layer's paint at a cell, bottom to top.
fn at(volume: &Volume, cell: IVec3) -> Vec<Rgba> {
    volume.layer_fields().iter().map(|field| field.at(cell)).collect()
}

/// Red on the bottom layer, a half blue wash over it on a second.
fn two_layers() -> Volume {
    let mut volume = ball();
    coat(&mut volume, RED);
    volume.add_layer("Wash", Blend::Wash).expect("room");
    coat(&mut volume, BLUE);
    volume
}

fn drained(volume: &mut Volume) {
    let mut dirty = Vec::new();
    volume.take_dirty(&mut dirty);
}

/// A stroke goes to the active layer only, and its undo takes it off that
/// layer only.
#[test]
fn a_stroke_goes_to_the_active_layer_and_comes_off_it() {
    let mut volume = two_layers();
    assert_eq!(at(&volume, ON), vec![RED, BLUE]);
    volume.set_active_layer(0);
    assert_eq!(volume.paint().at(ON), RED, "the active layer is not the volume's paint");

    volume.begin_stroke();
    coat(&mut volume, [0, 200, 0, 255]);
    let edit = volume.end_stroke().expect("a stroke");
    assert_eq!(at(&volume, ON), vec![[0, 200, 0, 255], BLUE]);
    let redo = volume.apply_edit(edit);
    assert_eq!(at(&volume, ON), vec![RED, BLUE], "undo touched the wrong layer");
    volume.apply_edit(redo);
    assert_eq!(at(&volume, ON), vec![[0, 200, 0, 255], BLUE]);
}

/// A stroke that reaches every layer, as a Move does, undoes on every
/// layer, even with another made active between.
#[test]
fn a_stroke_on_every_layer_undoes_on_every_layer() {
    let mut volume = two_layers();
    volume.begin_stroke();
    volume.edit_each_layer(|volume, _| coat(volume, BARE));
    let edit = volume.end_stroke().expect("a stroke");
    assert_eq!(at(&volume, ON), vec![BARE, BARE]);
    volume.set_active_layer(0);
    volume.apply_edit(edit);
    assert_eq!(at(&volume, ON), vec![RED, BLUE]);
}

/// What is seen is the stack put together, voxel by voxel, once the
/// dirty bricks are taken.
#[test]
fn what_is_seen_is_the_stack_put_together() {
    let mut volume = two_layers();
    drained(&mut volume);
    let metas = volume.layers().metas();
    let shown = volume.display_paint().at(ON);
    let flat = composite_voxel([(&metas[0], RED), (&metas[1], BLUE)].into_iter(), {
        // The crease the composite read there, from the field.
        let sample = |offset: IVec3| volume.sample_voxel(ON + offset);
        super::layers::crease_from(
            sample(IVec3::ZERO),
            [
                sample(IVec3::X),
                sample(-IVec3::X),
                sample(IVec3::Y),
                sample(-IVec3::Y),
                sample(IVec3::Z),
                sample(-IVec3::Z),
            ],
        )
    });
    assert_eq!(shown, flat);
    assert_ne!(shown, RED, "the wash did nothing");

    // Hiding the wash shows the red under it.
    let mut meta = metas[1].clone();
    meta.visible = false;
    volume.set_layer_meta(1, meta);
    drained(&mut volume);
    assert_eq!(volume.display_paint().at(ON), RED);
}

/// A body moved, turned or rebuilt carries every layer, each where the
/// surface took it.
#[test]
fn a_moved_body_carries_every_layer() {
    let volume = two_layers();
    let shifted = volume.shifted(IVec3::new(32, 0, 0));
    assert_eq!(shifted.layers().metas(), volume.layers().metas());
    assert_eq!(at(&shifted, ON + IVec3::new(32, 0, 0)), vec![RED, BLUE]);
    assert_eq!(shifted.layers().active(), volume.layers().active());
}

/// Both halves of a split keep every layer of their own half.
#[test]
fn both_halves_of_a_split_keep_every_layer() {
    let mut volume = two_layers();
    let (lo, hi) = (IVec3::splat(-20), IVec3::splat(20));
    volume.edit_mask(lo, hi, |_, at, _| if at.x > 0.0 { crate::PROTECTED } else { 0 });
    let mut doc = Document::from_volume(volume);
    let id = doc.active();
    let outcome = doc.split_masked(id).expect("a half masked ball divides");
    for (part, cell) in [(outcome.masked, ON), (outcome.rest, -ON)] {
        let part = doc.volume(part).expect("a part");
        assert_eq!(part.layers().len(), 2, "a part lost a layer");
        assert_eq!(at(part, cell), vec![RED, BLUE], "a part lost its paint at {cell}");
    }
}

/// A change to the stack goes on the history whole, and comes off it.
#[test]
fn a_change_to_the_stack_undoes() {
    let mut doc = Document::from_volume(two_layers());
    let body = doc.active();
    let before = doc.volume(body).expect("a body").layer_state();
    let mut history = History::new(1 << 30);
    doc.volume_mut(body).expect("a body").flatten_layers();
    assert_eq!(doc.volume(body).expect("a body").layers().len(), 1);
    history.push(Entry::new(vec![Change::Layers { body, state: Box::new(before.clone()) }]));
    history.undo(&mut doc, &[true]);
    let volume = doc.volume(body).expect("a body");
    assert_eq!(volume.layers().metas(), before.metas);
    assert_eq!(at(volume, ON), vec![RED, BLUE]);
    history.redo(&mut doc, &[true]);
    assert_eq!(doc.volume(body).expect("a body").layers().len(), 1);
}

/// A stack saved and opened is the stack, and what an older reader finds
/// is the paint as it was seen.
#[test]
fn a_stack_is_saved_with_the_document() {
    use crate::project::{ProjectState, read, write};
    let mut volume = two_layers();
    drained(&mut volume);
    let seen = volume.display_paint().at(ON);
    let mut doc = Document::from_volume(volume);
    let mut bytes = Vec::new();
    write(&mut bytes, &doc, &ProjectState::default()).expect("writing to memory");
    let (back, _) = read(&mut &bytes[..]).expect("a file just written");
    let body = back.volume(back.active()).expect("the body");
    assert_eq!(body.layers().metas(), doc.volume(doc.active()).expect("a body").layers().metas());
    assert_eq!(at(body, ON), vec![RED, BLUE]);

    // Without its layers section, the file opens as the seen paint.
    let tag = bytes
        .windows(8)
        .position(|window| window == super::layer_file::TAG)
        .expect("a layers section");
    let (older, _) = read(&mut &bytes[..tag]).expect("the file before the section");
    let body = older.volume(older.active()).expect("the body");
    assert!(body.layers().is_plain());
    assert_eq!(body.paint().at(ON), seen);

    // And a document flattened back to one plain layer writes no section.
    doc.volume_mut(doc.active()).expect("a body").flatten_layers();
    let mut plain = Vec::new();
    write(&mut plain, &doc, &ProjectState::default()).expect("writing to memory");
    assert!(!plain.windows(8).any(|window| window == super::layer_file::TAG));
}

/// One change to a stack.
#[derive(Debug, Clone)]
enum Step {
    Add(u8),
    Duplicate(usize),
    Delete(usize),
    Reorder(usize, usize),
    MergeDown(usize),
    Activate(usize),
    Opacity(usize, u8),
    Paint(u8),
    Flatten,
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        (0u8..6).prop_map(Step::Add),
        (0usize..20).prop_map(Step::Duplicate),
        (0usize..20).prop_map(Step::Delete),
        (0usize..20, 0usize..20).prop_map(|(a, b)| Step::Reorder(a, b)),
        (0usize..20).prop_map(Step::MergeDown),
        (0usize..20).prop_map(Step::Activate),
        (0usize..20, any::<u8>()).prop_map(|(a, o)| Step::Opacity(a, o)),
        any::<u8>().prop_map(Step::Paint),
        Just(Step::Flatten),
    ]
}

fn same(one: &PaintField, other: &PaintField) -> bool {
    let mut a: Vec<_> = one.bricks().keys().copied().collect();
    let mut b: Vec<_> = other.bricks().keys().copied().collect();
    a.sort_unstable();
    b.sort_unstable();
    a == b && a.iter().all(|coord| one.bricks()[coord] == other.bricks()[coord])
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    /// Whatever is done to a stack, it stays a stack: at least one layer,
    /// at most the cap, the active one in it, the ids each once, and the
    /// whole of it undone by putting back what it was before each change.
    #[test]
    fn any_changes_to_a_stack_undo_back_to_it(steps in proptest::collection::vec(step(), 1..12)) {
        let mut volume = ball();
        coat(&mut volume, RED);
        let mut states = Vec::new();
        for step in steps {
            let before = volume.layer_state();
            let len = volume.layers().len();
            match step {
                Step::Add(blend) => {
                    let _ = volume.add_layer("Layer", Blend::ALL[blend as usize % Blend::ALL.len()]);
                }
                Step::Duplicate(at) => {
                    let _ = volume.duplicate_layer(at % len);
                }
                Step::Delete(at) => volume.delete_layer(at % len),
                Step::Reorder(from, to) => volume.reorder_layer(from % len, to % len),
                Step::MergeDown(at) => volume.merge_layer_down(at % len),
                Step::Activate(at) => volume.set_active_layer(at % len),
                Step::Opacity(at, opacity) => {
                    let meta = volume.layers().metas()[at % len].clone();
                    volume.set_layer_meta(at % len, LayerMeta { opacity: f32::from(opacity) / 255.0, ..meta });
                }
                Step::Paint(value) => coat(&mut volume, [value, 255 - value, 9, value.max(1)]),
                Step::Flatten => volume.flatten_layers(),
            }
            let layers = volume.layers();
            prop_assert!((1..=super::layers::MAX_LAYERS).contains(&layers.len()));
            prop_assert!(layers.active() < layers.len());
            let mut ids: Vec<_> = layers.metas().iter().map(|meta| meta.id).collect();
            ids.sort_unstable();
            ids.dedup();
            prop_assert_eq!(ids.len(), layers.len(), "an id is on two layers");
            prop_assert_eq!(volume.layer_fields().len(), layers.len());
            states.push(before);
        }
        let first = states.first().cloned().expect("a step");
        for state in states.into_iter().rev() {
            volume.restore_layers(state);
        }
        prop_assert_eq!(volume.layers().metas(), first.metas.clone());
        prop_assert_eq!(volume.layers().active(), first.active);
        for (field, was) in volume.layer_fields().iter().zip(&first.fields) {
            prop_assert!(same(field, was), "a layer's paint did not come back");
        }
    }
}

/// The cells the surface passes through, near the ball's equator.
fn shell(volume: &Volume) -> Vec<IVec3> {
    let mut cells = Vec::new();
    for z in -2..=2 {
        for y in -14..=14 {
            for x in -14..=14 {
                let cell = IVec3::new(x, y, z);
                if volume.sample_voxel(cell).abs() < 0.5 {
                    cells.push(cell);
                }
            }
        }
    }
    cells
}

/// A blend that does not read the shape, so a flatten keeps it exactly.
fn flat_blend() -> impl Strategy<Value = Blend> {
    prop_oneof![Just(Blend::Normal), Just(Blend::Multiply), Just(Blend::Screen), Just(Blend::Glaze)]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 16, ..ProptestConfig::default() })]

    /// A stack flattened is what was seen, where no blend reads the shape;
    /// and a layer moved up and back down leaves what is seen as it was.
    #[test]
    fn a_flattened_stack_is_what_was_seen(
        layers in proptest::collection::vec(
            (flat_blend(), 0u8..=255, (any::<u8>(), any::<u8>(), any::<u8>(), 1u8..=255)),
            1..5,
        ),
    ) {
        let mut volume = ball();
        for (index, (blend, opacity, (r, g, b, a))) in layers.iter().enumerate() {
            if index > 0 {
                volume.add_layer("Layer", *blend).expect("room");
            }
            let meta = volume.layers().metas()[index].clone();
            volume.set_layer_meta(
                index,
                LayerMeta { blend: *blend, opacity: f32::from(*opacity) / 255.0, ..meta },
            );
            coat(&mut volume, [*r, *g, *b, *a]);
        }
        drained(&mut volume);
        let cells = shell(&volume);
        let seen: Vec<Rgba> = cells.iter().map(|cell| volume.display_paint().at(*cell)).collect();

        if volume.layers().len() > 1 {
            volume.reorder_layer(0, 1);
            volume.reorder_layer(1, 0);
            drained(&mut volume);
            let again: Vec<Rgba> =
                cells.iter().map(|cell| volume.display_paint().at(*cell)).collect();
            prop_assert_eq!(&again, &seen, "up and back down changed what is seen");
        }

        volume.flatten_layers();
        drained(&mut volume);
        prop_assert!(volume.layers().is_plain());
        let flat: Vec<Rgba> = cells.iter().map(|cell| volume.display_paint().at(*cell)).collect();
        prop_assert_eq!(flat, seen, "a flatten changed what is seen");
    }
}

/// Layers taken out come back where they were, with their paint, and the
/// stack between is the stack before they were added.
#[test]
fn layers_taken_out_come_back_as_they_were() {
    let mut volume = two_layers();
    volume.set_active_layer(0);
    let before = volume.layer_state();
    let first = volume.add_layer("Coat", Blend::Multiply).expect("room");
    coat(&mut volume, [9, 90, 9, 200]);
    let at = volume.layers().active();
    volume.reorder_layer(at, 0);
    let second = volume.add_layer("Glaze", Blend::Glaze).expect("room");
    coat(&mut volume, [90, 9, 9, 100]);
    let after = volume.layer_state();

    let taken = volume.take_layers(&[first, second]);
    assert_eq!(volume.layers().metas(), before.metas);
    assert_eq!(at_all(&volume), vec![RED, BLUE]);
    volume.put_layers_back(taken);
    assert_eq!(volume.layers().metas(), after.metas);
    assert_eq!(volume.layers().active(), after.active);
    for (field, was) in volume.layer_fields().iter().zip(&after.fields) {
        assert!(same(field, was), "a layer's paint did not come back");
    }

    // Taking every real layer leaves the implicit one.
    let mut fresh = ball();
    let only = fresh.add_layer("Only", Blend::Normal).expect("room");
    let _ = fresh.take_layers(&[only]);
    assert!(fresh.layers().is_plain());
    assert_eq!(fresh.layers().len(), 1);
}

fn at_all(volume: &Volume) -> Vec<Rgba> {
    at(volume, ON)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    /// Any order and any settings, arranged and arranged back, are the
    /// stack as it was, paint and all.
    #[test]
    fn layers_arranged_and_back_are_as_they_were(
        order in Just(vec![0usize, 1, 2, 3]).prop_shuffle(),
        opacity in 0.0f32..1.0,
        active in 0usize..4,
    ) {
        let mut volume = two_layers();
        volume.add_layer("Third", Blend::Screen).expect("room");
        coat(&mut volume, [1, 2, 3, 40]);
        volume.add_layer("Fourth", Blend::Glaze).expect("room");
        let before = volume.layer_state();
        let metas = volume.layers().metas();
        let wanted: Vec<LayerMeta> = order
            .iter()
            .map(|at| LayerMeta { opacity, ..metas[*at].clone() })
            .collect();
        let (had, had_active) = volume.arrange_layers(wanted.clone(), metas[active].id);
        prop_assert_eq!(volume.layers().metas(), wanted);
        prop_assert_eq!(volume.layers().active_id(), metas[active].id);
        volume.arrange_layers(had, had_active);
        prop_assert_eq!(volume.layers().metas(), before.metas.clone());
        prop_assert_eq!(volume.layers().active(), before.active);
        for (field, was) in volume.layer_fields().iter().zip(&before.fields) {
            prop_assert!(same(field, was), "a layer's paint moved");
        }
    }
}

/// A metal layer's paint has the metallic finish where it is on top: a
/// plain coat over it takes the finish away, and a varnish over that gives
/// its own. The flag goes through the file.
#[test]
fn a_metal_layer_is_metallic_where_it_is_on_top() {
    use crate::paint::Finish;
    let mut volume = ball();
    coat(&mut volume, RED);
    volume.add_layer("Gold", Blend::Normal).expect("room");
    let meta = volume.layers().metas()[1].clone();
    volume.set_layer_meta(1, LayerMeta { metal: true, ..meta });
    coat(&mut volume, [200, 160, 40, 255]);
    drained(&mut volume);
    assert_eq!(volume.finish_at(ON), Finish::Metallic.code());

    volume.add_layer("Over", Blend::Normal).expect("room");
    coat(&mut volume, [20, 40, 210, 100]);
    drained(&mut volume);
    assert_eq!(volume.finish_at(ON), Finish::Metallic.code(), "less than half a coat over it");
    coat(&mut volume, [20, 40, 210, 255]);
    drained(&mut volume);
    assert_eq!(volume.finish_at(ON), 0, "a full coat over the metal left it metallic");

    volume.add_layer("Varnish", Blend::Varnish).expect("room");
    coat(&mut volume, [Finish::Gloss.code(), 0, 0, 255]);
    drained(&mut volume);
    assert_eq!(volume.finish_at(ON), Finish::Gloss.code());

    let bytes = super::layer_file::encode(&[Some(volume.layer_state())]).expect("a section");
    let back = super::layer_file::decode(&bytes, 1).expect("it reads");
    assert!(back[0].as_ref().expect("a stack").metas[1].metal, "the metal flag was lost");
}

/// A chrome layer is a metal layer whose finish is chrome: where its paint
/// is on top the surface has the chrome finish, and the same layer without
/// the flag is a metallic paint's again.
#[test]
fn a_chrome_layer_is_chrome_where_it_is_on_top() {
    use crate::paint::Finish;
    let mut volume = ball();
    volume.add_layer("Mirror", Blend::Normal).expect("room");
    let meta = volume.layers().metas()[1].clone();
    volume.set_layer_meta(1, LayerMeta { metal: true, chrome: true, ..meta.clone() });
    coat(&mut volume, [210, 210, 220, 255]);
    drained(&mut volume);
    assert_eq!(volume.finish_at(ON), Finish::Chrome.code());
    volume.set_layer_meta(1, LayerMeta { metal: true, chrome: false, ..meta.clone() });
    drained(&mut volume);
    assert_eq!(volume.finish_at(ON), Finish::Metallic.code());
    // Chrome means nothing without metal.
    volume.set_layer_meta(1, LayerMeta { metal: false, chrome: true, ..meta });
    drained(&mut volume);
    assert_eq!(volume.finish_at(ON), 0);
}
