// SPDX-License-Identifier: AGPL-3.0-only

//! What a button does, shown when the pointer rests on it.
//!
//! Found by the maintainer on 2026-11-27: "Many buttons are missing
//! tooltips." Every button that showed only an icon already had one (a
//! test in `ui.rs` holds them to it); a button with a word on it had none,
//! and a word such as Glaze, Wash or Sharpen does not say what it does.
//! The interface is controls, not prose (the house style), so what a
//! control does is said here, on hover, and not in a caption beside it.
//!
//! One function an enum, over all of its variants, so a variant added
//! later cannot be missed: the match is exhaustive. A test holds every tip
//! to a line a tooltip can carry.

use hani_core::FalloffCurve;
use hani_core::MaskFilter;
use hani_core::paint::layers::Blend;
use hani_core::paint::touch::Foam;

use crate::message::{ExportFormat, MaskGenerator, PaintMode, PaintTouch, TopMenu};

pub(super) fn paint_mode(mode: PaintMode) -> &'static str {
    match mode {
        PaintMode::Paint => "Covers what is under it, on a Normal layer",
        PaintMode::Glaze => "Tints every layer under it, on a Glaze layer of its own",
        PaintMode::Erase => "Takes paint off the active layer, back toward what is under it",
    }
}

pub(super) fn touch(touch: PaintTouch) -> &'static str {
    match touch {
        PaintTouch::Loaded => "A loaded brush: everything under it",
        PaintTouch::Drybrush => "A nearly dry brush: only what stands up catches paint",
        PaintTouch::Sponge => "Dabbed with a sponge: specks, more on edges",
        PaintTouch::Fill => "Poured: runs over a panel until a crease, an edge or a paint stops it",
        PaintTouch::Wash => "A thin wash on a layer of its own that pools in the recesses",
        PaintTouch::Contrast => "A contrast paint on a layer of its own: tints, pools in recesses",
        PaintTouch::Airbrush => "Sprayed while held, building up by flow and pressure",
        PaintTouch::Decal => "A picture dabbed on where you click",
    }
}

pub(super) fn foam(foam: Foam) -> &'static str {
    match foam {
        Foam::Fine => "Small, even specks",
        Foam::Coarse => "Specks twice the size",
        Foam::Torn => "Specks with ragged edges",
        Foam::Smooth => "No specks: an even, thinner coat",
    }
}

pub(super) fn falloff(curve: FalloffCurve) -> &'static str {
    match curve {
        FalloffCurve::Smooth => "Soft from the middle to the rim",
        FalloffCurve::Linear => "Even all the way out",
        FalloffCurve::Sharp => "Strong in the middle, for detail",
        FalloffCurve::Wide => "Strong nearly to the rim, for filling areas",
    }
}

pub(super) fn mask_filter(filter: MaskFilter) -> &'static str {
    match filter {
        MaskFilter::Blur => "Softens the mask's edge; drag Amount",
        MaskFilter::Sharpen => "Hardens the mask's edge; drag Amount",
        MaskFilter::Grow => "Spreads the mask outward; drag Amount",
        MaskFilter::Shrink => "Pulls the mask in; drag Amount",
    }
}

pub(super) fn mask_generator(generator: MaskGenerator) -> &'static str {
    match generator {
        MaskGenerator::Cavity => "Masks the creases narrower than Feature",
        MaskGenerator::Smoothness => "Masks what is flat at the Feature size",
        MaskGenerator::Thickness => "Masks what is thinner than Thickness",
    }
}

pub(super) fn blend(blend: Blend) -> &'static str {
    match blend {
        Blend::Normal => "Covers what is under it",
        Blend::Multiply => "Darkens what is under it by its colour",
        Blend::Screen => "Lightens what is under it",
        Blend::Glaze => "A thin film that tints what is under it, deeper in the creases",
        Blend::Wash => "A film that runs off the flats and edges and pools in the creases",
        Blend::Contrast => "A coat that tints what is under it: strong over light, pools",
        Blend::Varnish => "Lays no colour: changes how shiny what is under it is",
    }
}

pub(super) fn key_shape(shape: hani_core::part::KeyShape) -> &'static str {
    match shape {
        hani_core::part::KeyShape::Round => "Round pegs",
        hani_core::part::KeyShape::Square => "Square pegs, which cannot turn in their sockets",
    }
}

pub(super) fn side(side: hani_core::brush::compound::Side) -> &'static str {
    use hani_core::brush::compound::Side;
    match side {
        Side::Both => "Up to the plane and down to it, as Flatten does",
        Side::Fill => "Only up to the plane: adds, takes nothing away",
        Side::Cut => "Only down to the plane: takes off what stands proud",
    }
}

pub(super) fn reference_plane(plane: hani_core::reference::Plane) -> &'static str {
    use hani_core::reference::Plane;
    match plane {
        Plane::Front => "The picture the Front view looks straight at",
        Plane::Side => "The picture the Right view looks straight at",
        Plane::Top => "The picture the Top view looks straight at",
    }
}

pub(super) fn dock_tab(tab: crate::message::DockTab) -> &'static str {
    match tab {
        crate::message::DockTab::Library => "What this workspace offers to pick from",
        crate::message::DockTab::Inspector => "The settings of what is in hand",
    }
}

pub(super) fn cable_end(end: hani_core::solid::End) -> &'static str {
    use hani_core::solid::End;
    match end {
        End::Round => "The tube's own round end",
        End::Collar => "A short sleeve wider than the cable: a ferrule, a plug",
        End::Ball => "A ball: a knob, a joint",
        End::Flange => "A thin wide disc: a flange, a washer",
    }
}

pub(super) fn top_menu(menu: TopMenu) -> &'static str {
    match menu {
        TopMenu::File => "New, open, save, import and export",
        TopMenu::Edit => "Undo, the mask and Preferences",
        TopMenu::View => "What is drawn over the model, and framing it",
        TopMenu::Help => "Help, the keys, a bug report, About and updates",
    }
}

pub(super) fn export(format: ExportFormat) -> &'static str {
    match format {
        ExportFormat::Stl => "A mesh any slicer reads, without units or colour",
        ExportFormat::Obj => "A mesh for other 3D programs, without colour",
        ExportFormat::ThreeMf => "A mesh for a slicer, in millimetres",
        ExportFormat::Ply => "A mesh with the paint as a colour at every corner",
        ExportFormat::Textured => "A small mesh with the paint as a picture beside it",
        ExportFormat::Glb => "One file: a small mesh and the paint as a picture inside",
    }
}

pub(super) fn pattern(kind: hani_core::PatternKind) -> &'static str {
    use hani_core::PatternKind;
    match kind {
        PatternKind::None => "The brush as it is, with no pattern",
        PatternKind::Noise => "Roughens the surface",
        PatternKind::Scales => "Domed scales in offset rows",
        PatternKind::Hair => "Combed strands along the stroke",
        PatternKind::Weave => "A basket weave of bands over and under",
        PatternKind::Cracks => "A thin network of lines, for crazing",
    }
}

pub(super) fn primitive(kind: hani_core::PrimitiveKind) -> &'static str {
    use hani_core::PrimitiveKind;
    match kind {
        PrimitiveKind::Cube => "A new body: a cube",
        PrimitiveKind::Sphere => "A new body: a ball",
        PrimitiveKind::Cylinder => "A new body: a cylinder",
    }
}

pub(super) fn profile(shape: hani_core::brush::compound::Shape) -> &'static str {
    use hani_core::brush::compound::Shape;
    match shape {
        Shape::Round => "A dome: full in the middle, rounding off to the rim",
        Shape::Smooth => falloff(FalloffCurve::Smooth),
        Shape::Linear => falloff(FalloffCurve::Linear),
        Shape::Sharp => falloff(FalloffCurve::Sharp),
        Shape::Wide => falloff(FalloffCurve::Wide),
    }
}

pub(super) fn deform(kind: hani_core::deform::DeformKind) -> &'static str {
    use hani_core::deform::DeformKind;
    match kind {
        DeformKind::Twist => "Turns each slice about the axis, more toward the far end",
        DeformKind::Bend => "Curls the axis into an arc",
        DeformKind::Taper => "Narrows or widens the body toward the far end",
    }
}

pub(super) fn deform_axis(axis: hani_core::deform::Axis) -> &'static str {
    use hani_core::deform::Axis;
    match axis {
        Axis::X => "Along the X axis",
        Axis::Y => "Along the Y axis",
        Axis::Z => "Along the Z axis",
    }
}

pub(super) fn finish(finish: hani_core::paint::Finish) -> &'static str {
    use hani_core::paint::Finish;
    match finish {
        Finish::Matte => "No shine",
        Finish::Satin => "A soft sheen",
        Finish::Gloss => "Wet-looking shine",
        Finish::Metallic => "Shines as metal does",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every tip is one line a tooltip can carry, in the house's style.
    #[test]
    fn every_tip_is_a_short_line() {
        let mut all: Vec<&str> = Vec::new();
        all.extend(PaintMode::ALL.map(paint_mode));
        all.extend(PaintTouch::ALL.map(touch));
        all.extend(Foam::ALL.map(foam));
        all.extend(FalloffCurve::ALL.map(falloff));
        all.extend(MaskFilter::ALL.map(mask_filter));
        all.extend(MaskGenerator::ALL.map(mask_generator));
        all.extend(Blend::ALL.map(blend));
        all.extend(hani_core::part::KeyShape::ALL.map(key_shape));
        all.extend(hani_core::brush::compound::Side::ALL.map(side));
        all.extend(hani_core::reference::Plane::ALL.map(reference_plane));
        all.extend(
            [crate::message::DockTab::Library, crate::message::DockTab::Inspector].map(dock_tab),
        );
        all.extend(hani_core::solid::End::ALL.map(cable_end));
        all.extend(TopMenu::ALL.map(top_menu));
        all.extend(ExportFormat::ALL.map(export));
        all.extend(hani_core::PatternKind::ALL.map(pattern));
        all.extend(hani_core::PrimitiveKind::ALL.map(primitive));
        all.extend(hani_core::brush::compound::Shape::ALL.map(profile));
        all.extend(hani_core::deform::DeformKind::ALL.map(deform));
        all.extend(hani_core::deform::Axis::ALL.map(deform_axis));
        all.extend(hani_core::paint::Finish::ALL.map(finish));
        for tip in all {
            assert!(!tip.is_empty() && tip.len() <= 80, "{tip:?}");
            assert!(!tip.contains('\u{2014}') && !tip.contains('\n'), "{tip:?}");
            assert!(!tip.ends_with('.'), "a tip is a phrase: {tip:?}");
        }
    }
}
