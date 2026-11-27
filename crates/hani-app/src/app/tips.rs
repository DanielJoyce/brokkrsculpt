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

use crate::message::{MaskGenerator, PaintMode, PaintTouch};

pub(super) fn paint_mode(mode: PaintMode) -> &'static str {
    match mode {
        PaintMode::Paint => "Covers what is under it",
        PaintMode::Glaze => "Tints what is under it in the paint's hue, without hiding it",
        PaintMode::Erase => "Takes paint off, back toward the clay",
    }
}

pub(super) fn touch(touch: PaintTouch) -> &'static str {
    match touch {
        PaintTouch::Loaded => "A loaded brush: everything under it",
        PaintTouch::Drybrush => "A nearly dry brush: only what stands up catches paint",
        PaintTouch::Sponge => "Dabbed with a sponge: specks, more on edges",
        PaintTouch::Fill => "Poured: runs over a panel until a crease, an edge or a paint stops it",
        PaintTouch::Wash => "A thin wash on a layer of its own that pools in the recesses",
        PaintTouch::Contrast => "A contrast paint: strong over light, pools in recesses",
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
        Blend::Glaze => "Tints what is under it in its hue",
        Blend::Wash => "Darkens, more in the creases and less on the edges",
        Blend::Contrast => "Strong over light, faint over dark, pools in the creases",
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
        for tip in all {
            assert!(!tip.is_empty() && tip.len() <= 80, "{tip:?}");
            assert!(!tip.contains('\u{2014}') && !tip.contains('\n'), "{tip:?}");
            assert!(!tip.ends_with('.'), "a tip is a phrase: {tip:?}");
        }
    }
}
