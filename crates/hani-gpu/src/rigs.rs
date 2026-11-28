// SPDX-License-Identifier: AGPL-3.0-only

//! The rigs the Lighting block offers, each one press, and the two angles
//! a person sets a light's direction by: issue 203, stage two.
//!
//! Here and not in the application so that the offscreen contact sheet
//! (`tests/offscreen.rs`, `the_contact_sheet_of_the_lights`) draws the very
//! rigs the block hands the renderer, and the numbers below were tuned by
//! looking at it.
//!
//! - [`studio`] is [`Rig::default`], the matcap's lights with the camera:
//!   the look the viewport has always had.
//! - [`lightbox`] is a key, a fill and a rim fixed in the world, as in a
//!   light tent: turning the model moves the highlights over it, which is
//!   what tells a gloss from a matte.
//! - [`overhead`] is one strong light from above, fixed, and a sky: the
//!   light a miniature is painted for, bright on top and dark under.
//! - [`sky`] is the global light alone: no highlights from a light, only
//!   the sky above and the ground below.
//!
//! # Two angles
//!
//! A direction is *round* the model and *up* from level, in degrees. Round
//! 0 is in front, toward the eye for a light with the camera and toward
//! the world's +z (where the camera starts) for one fixed in the world; 90
//! is to the right, 180 behind and 270 to the left. Up 0 is level, 90
//! straight above and -90 straight below. The same two numbers mean the
//! same place for both kinds of light, the one in the view's frame and the
//! other in the world's, since both have y up and z toward where the
//! camera starts.

use glam::Vec3;

use crate::lights::{Light, Rig};

/// Toward a light `round` degrees round the model and `up` degrees up from
/// level, of unit length.
pub fn toward(round: f32, up: f32) -> Vec3 {
    let (sin_round, cos_round) = round.to_radians().sin_cos();
    let (sin_up, cos_up) = up.to_radians().sin_cos();
    Vec3::new(sin_round * cos_up, sin_up, cos_round * cos_up)
}

/// The two angles of a direction, in degrees: round in `0..360` and up in
/// `-90..=90`. A direction straight up or down is round 0. The inverse of
/// [`toward`].
pub fn angles(direction: Vec3) -> (f32, f32) {
    let way = direction.normalize_or(Vec3::Z);
    let up = way.y.clamp(-1.0, 1.0).asin().to_degrees();
    let level = Vec3::new(way.x, 0.0, way.z);
    let round = if level.length_squared() < 1.0e-12 {
        0.0
    } else {
        way.x.atan2(way.z).to_degrees().rem_euclid(360.0)
    };
    // `rem_euclid` of a hair under nought can round to 360 itself.
    (if round >= 360.0 { 0.0 } else { round }, up)
}

/// A plain light fixed in the world, by its angles.
fn fixed(round: f32, up: f32, colour: [f32; 3], strength: f32) -> Light {
    Light::new(toward(round, up), colour, strength, false)
}

/// The viewport's own lights, with the camera: [`Rig::default`].
pub fn studio() -> Rig {
    Rig::default()
}

/// A key high on the front left, a fill low on the right and a rim from
/// behind, fixed in the world, as in a light tent; a little of the room
/// round them. The key and the fill are soft, wrapping past side-on as a
/// large diffuser does, and the fill casts half a highlight.
pub fn lightbox() -> Rig {
    let key = Light { wrap: 0.15, ..fixed(320.0, 35.0, [1.0, 0.97, 0.92], 1.2) };
    let fill = Light { wrap: 0.3, highlight: 0.4, ..fixed(70.0, 0.0, [1.0, 0.95, 0.88], 0.25) };
    let rim = fixed(160.0, 35.0, [1.0, 0.88, 0.74], 0.55);
    Rig {
        lights: vec![key, fill, rim],
        global: 0.08,
        sky: [0.95, 0.97, 1.0],
        ground: [0.4, 0.37, 0.34],
    }
}

/// One strong warm light from nearly straight above, fixed, and a cool sky
/// over a dark ground: the top of every form lit and its underside in
/// shade.
pub fn overhead() -> Rig {
    let sun = fixed(20.0, 75.0, [1.0, 0.96, 0.88], 1.5);
    Rig { lights: vec![sun], global: 0.18, sky: [0.75, 0.85, 1.0], ground: [0.12, 0.1, 0.08] }
}

/// The global light alone: a bright sky over a dark ground, and no light
/// of its own.
pub fn sky() -> Rig {
    Rig::sky_only(0.95, [0.95, 0.97, 1.0], [0.2, 0.18, 0.16])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two angles of a direction give it back, round every way but
    /// straight up and down, where round says nothing.
    #[test]
    fn the_angles_of_a_direction_give_it_back() {
        for round in (0..360).step_by(15) {
            for up in (-75..=75).step_by(15) {
                let (round, up) = (round as f32, up as f32);
                let way = toward(round, up);
                assert!((way.length() - 1.0).abs() < 1.0e-5);
                let (r, u) = angles(way);
                assert!((u - up).abs() < 1.0e-3, "up {up} came back as {u}");
                let apart = (r - round).rem_euclid(360.0);
                assert!(apart.min(360.0 - apart) < 1.0e-3, "round {round} came back as {r}");
            }
        }
        assert!((toward(0.0, 90.0) - Vec3::Y).length() < 1.0e-6);
        assert!((toward(270.0, 0.0) - Vec3::NEG_X).length() < 1.0e-6);
        assert_eq!(angles(Vec3::Y), (0.0, 90.0));
        assert_eq!(angles(Vec3::NEG_Y), (0.0, -90.0));
    }

    /// Studio is the default; the other three are fixed in the world, and
    /// Sky has no light.
    #[test]
    fn the_presets_are_what_they_say() {
        assert_eq!(studio(), Rig::default());
        assert!(studio().lights.iter().all(|light| light.with_camera));
        assert_eq!(lightbox().lights.len(), 3);
        assert!(lightbox().lights.iter().all(|light| !light.with_camera));
        assert_eq!(overhead().lights.len(), 1);
        assert!(overhead().lights[0].direction.y > 0.9, "the overhead light is not overhead");
        assert!(!overhead().lights[0].with_camera);
        assert!(sky().lights.is_empty() && sky().global > 0.5);
    }
}
