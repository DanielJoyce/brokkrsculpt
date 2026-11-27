// SPDX-License-Identifier: AGPL-3.0-only

//! The fit test: a small print that says what clearance a printer needs.
//!
//! A key's clearance is the gap left between a peg and its socket, and the
//! right number for it belongs to the printer, the filament and the day:
//! 0.2 mm is usual for filament, under 0.1 for resin, and a part that
//! needed 0.15 comes off another machine fused solid. Nobody can be told
//! the number. It has to be printed.
//!
//! So this makes the thing to print: a plate with a row of sockets, each
//! cut at a different clearance, and one peg to try in them. The socket the
//! peg goes into the way its owner wants (pressed, pushed, dropped) is the
//! clearance to set.
//!
//! # Why it can be trusted
//!
//! The sockets and the peg are cut by the same lines that cut a real part's
//! ([`super::Key::solid`], and the socket as that solid grown by the
//! clearance), at the voxel size of the document they are for. The test is
//! the thing itself, six times, and not a model of it.
//!
//! # Telling the sockets apart
//!
//! The plate has one corner cut off, at the end where the sockets are
//! tightest, and they loosen in order from there
//! ([`CLEARANCES_MM`]). A printed plate has no other markings: letters
//! small enough to fit would not survive the lattice or the nozzle.
//!
//! For a pin hole or a magnet pocket there is no peg to print: the pin or
//! the magnet is the peg, and the plate is the whole test.

use glam::Vec3;

use super::{Key, KeyKind, KeySpec, MIN_KEY_VOXELS};
use crate::brick::{INSIDE, NARROW_BAND, OUTSIDE};
use crate::clip::ClipPlane;
use crate::volume::Volume;

/// The clearances a fit test is cut at, tightest first, in millimetres.
/// From what a resin printer holds to more than a worn filament printer
/// needs.
pub const CLEARANCES_MM: [f32; 6] = [0.05, 0.10, 0.15, 0.20, 0.30, 0.40];

/// How thick the peg's base is: enough to hold, and a few layers of any
/// printer.
const BASE_MM: f32 = 2.0;

/// A fit test, ready to export. Both pieces stand on `y = 0` as they print:
/// the plate with its sockets opening upward, the peg pointing up.
pub struct Coupon {
    /// The plate of sockets, holes or pockets. They run along +X from the
    /// end with the corner cut off, tightest first.
    pub plate: Volume,
    /// The peg to try in them, or `None` for a kind of key whose peg is not
    /// printed.
    pub peg: Option<Volume>,
    /// Where each socket's axis meets the top of the plate, in the order
    /// of [`CLEARANCES_MM`].
    pub sockets: Vec<Vec3>,
}

/// Why a fit test could not be made.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CouponError {
    /// The key is narrower than the lattice holds. Holds the voxel size, in
    /// millimetres, at which it would not be.
    TooNarrow { voxel_mm: f32 },
}

impl std::fmt::Display for CouponError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CouponError::TooNarrow { voxel_mm } => write!(
                f,
                "that key is too narrow to cut at this detail: it needs a voxel of {voxel_mm:.2} \
                 mm or finer"
            ),
        }
    }
}

/// A box as a signed distance, in world units: negative inside.
fn block(at: Vec3, low: Vec3, high: Vec3) -> f32 {
    let centre = (low + high) * 0.5;
    let outside = (at - centre).abs() - (high - low) * 0.5;
    outside.max(Vec3::ZERO).length() + outside.max_element().min(0.0)
}

/// A volume holding one box.
fn boxed(voxel: f32, low: Vec3, high: Vec3) -> Volume {
    let mut volume = Volume::new(voxel);
    let band = Vec3::splat((NARROW_BAND + 1.0) * voxel);
    volume.edit_box(low - band, high + band, move |position, _| {
        (block(position, low, high) / voxel).clamp(INSIDE, OUTSIDE)
    });
    volume
}

/// The fit test for keys of `spec`'s kind and size, at a document's voxel
/// size. `spec`'s own clearance is not used: the test is of every
/// clearance in [`CLEARANCES_MM`].
pub fn coupon(spec: KeySpec, voxel_size: f32) -> Result<Coupon, CouponError> {
    if spec.radius_mm * 2.0 < MIN_KEY_VOXELS * voxel_size {
        return Err(CouponError::TooNarrow { voxel_mm: spec.radius_mm * 2.0 / MIN_KEY_VOXELS });
    }
    let loosest = CLEARANCES_MM[CLEARANCES_MM.len() - 1];
    // One socket's share of the plate: the socket, its loosest clearance
    // and a wall, either side.
    let pitch = 2.0 * (spec.radius_mm + loosest + spec.wall_mm);
    let height = spec.depth_mm + loosest + spec.wall_mm;
    let length = pitch * CLEARANCES_MM.len() as f32;

    let mut plate = boxed(
        voxel_size,
        Vec3::new(0.0, 0.0, -pitch * 0.5),
        Vec3::new(length, height, pitch * 0.5),
    );
    let mut sockets = Vec::with_capacity(CLEARANCES_MM.len());
    for (index, clearance) in CLEARANCES_MM.into_iter().enumerate() {
        let centre = Vec3::new(pitch * (index as f32 + 0.5), height, 0.0);
        // The axis runs from the part with the peg into the part with the
        // socket, as a real key's does: down, into the plate.
        let key = Key {
            centre,
            axis: Vec3::NEG_Y,
            kind: spec.kind,
            radius_mm: spec.radius_mm,
            depth_mm: spec.depth_mm,
            clearance_mm: clearance,
            shape: spec.shape,
        };
        plate.carve(key.bounds(voxel_size), move |at| key.solid(at, voxel_size) - clearance);
        sockets.push(centre);
    }
    // The mark: the upright corner at the tight end, cut off.
    let cut = pitch * 0.25;
    plate.clip(ClipPlane {
        point: Vec3::new(cut, 0.0, -pitch * 0.5),
        normal: Vec3::new(-1.0, 0.0, -1.0).normalize(),
    });

    let peg = (spec.kind == KeyKind::Peg).then(|| {
        // Beside the plate, clear of it, on a base to hold it by.
        let middle = -pitch;
        let half = pitch * 0.4;
        let mut peg = boxed(
            voxel_size,
            Vec3::new(middle - half, 0.0, -half),
            Vec3::new(middle + half, BASE_MM, half),
        );
        let key = Key {
            centre: Vec3::new(middle, BASE_MM, 0.0),
            axis: Vec3::Y,
            kind: KeyKind::Peg,
            radius_mm: spec.radius_mm,
            depth_mm: spec.depth_mm,
            clearance_mm: 0.0,
            shape: spec.shape,
        };
        peg.unite(key.bounds(voxel_size), move |at| key.solid(at, voxel_size));
        peg
    });
    Ok(Coupon { plate, peg, sockets })
}

#[cfg(test)]
mod tests {
    use super::*;

    const VOXEL: f32 = 0.1;

    /// How far from an upright axis through `centre` the surface is, at
    /// `height`: found by walking out from the axis along +Z until the
    /// field changes sign. For a socket that is its radius, and for a peg.
    fn radius_at(volume: &Volume, centre: Vec3, height: f32) -> f32 {
        let inside = |out: f32| volume.sample_world(Vec3::new(centre.x, height, out)) < 0.0;
        let started = inside(0.0);
        let mut out = 0.0;
        while out < 20.0 && inside(out) == started {
            out += 0.005;
        }
        out
    }

    /// Each socket is wider than the peg by its own clearance, at the
    /// height they meet at, and they come in order from the marked end.
    #[test]
    fn each_socket_is_the_peg_and_its_own_clearance() {
        let spec = KeySpec::peg();
        let made = coupon(spec, VOXEL).expect("a peg of 2 mm at 0.1 mm");
        let peg = made.peg.as_ref().expect("a peg key has a peg to print");
        assert_eq!(made.sockets.len(), CLEARANCES_MM.len());
        let top = made.sockets[0].y;

        // Halfway down the socket, which on the peg is halfway up it.
        let down = spec.depth_mm * 0.5;
        let peg_base = Vec3::new(-2.0 * (spec.radius_mm + 0.4 + spec.wall_mm), 0.0, 0.0);
        let peg_radius = radius_at(peg, peg_base, BASE_MM + down);
        // A peg tapers to 0.8 of its radius at the tip: at half way, 0.9.
        assert!(
            (peg_radius - spec.radius_mm * 0.9).abs() < 0.3 * VOXEL,
            "the peg is {peg_radius} mm in radius half way up"
        );
        let mut last = 0.0;
        for (socket, clearance) in made.sockets.iter().zip(CLEARANCES_MM) {
            let radius = radius_at(&made.plate, *socket, top - down);
            assert!(
                (radius - peg_radius - clearance).abs() < 0.3 * VOXEL,
                "the socket for {clearance} mm is {radius} mm in radius against a peg of \
                 {peg_radius}"
            );
            assert!(radius > last, "the sockets do not loosen in order");
            last = radius;
        }
        // Control: the loosest and the tightest differ by far more than the
        // tolerance they were each held to.
        let widest = made.sockets.last().map(|socket| radius_at(&made.plate, *socket, top - down));
        let narrowest = radius_at(&made.plate, made.sockets[0], top - down);
        assert!(widest.expect("six sockets") - narrowest > 3.0 * VOXEL);
    }

    /// Both pieces print: closed, standing on the bed, the sockets open at
    /// the top and closed underneath, and the peg clear of the plate.
    #[test]
    fn both_pieces_are_closed_and_stand_on_the_bed() {
        let spec = KeySpec::peg();
        let made = coupon(spec, VOXEL).expect("a peg of 2 mm at 0.1 mm");
        let peg = made.peg.as_ref().expect("a peg");
        for (name, volume) in [("plate", &made.plate), ("peg", peg)] {
            let (mesh, report) = volume.export_mesh();
            assert!(report.is_printable(), "the {name} is not closed: {}", report.summary());
            let lowest = mesh.lowest().expect("a mesh");
            assert!(lowest.abs() < 0.3 * VOXEL, "the {name} stands at {lowest} mm");
        }
        for socket in &made.sockets {
            // Open at the top, at the axis.
            assert!(made.plate.sample_world(*socket - Vec3::Y * 0.5) > 0.0, "a socket is shut");
            // And a wall's worth of plate under it.
            assert!(made.plate.sample_world(Vec3::new(socket.x, 0.4, 0.0)) < 0.0, "no floor");
        }
        // The peg's piece is all to one side of the plate's.
        let (plate_low, _) = made.plate.surface_bounds().expect("a plate");
        let (_, peg_high) = peg.surface_bounds().expect("a peg");
        assert!(peg_high.x + 1.0 < plate_low.x, "the peg at {} meets the plate", peg_high.x);
        // And is as tall as its base and a peg.
        let tall = peg.export_mesh().0.positions.iter().map(|p| p.y).fold(f32::MIN, f32::max);
        assert!(
            (tall - (BASE_MM + spec.depth_mm)).abs() < 0.5 * VOXEL,
            "the peg is {tall} mm tall"
        );
    }

    /// The tight end is the one with the corner cut off, and only that
    /// corner is.
    #[test]
    fn the_tight_end_has_its_corner_cut_off() {
        let spec = KeySpec::peg();
        let made = coupon(spec, VOXEL).expect("a peg of 2 mm at 0.1 mm");
        let pitch = 2.0 * (spec.radius_mm + 0.4 + spec.wall_mm);
        let length = pitch * 6.0;
        let just_inside = |x: f32, z: f32| {
            let inward =
                Vec3::new(if x > 1.0 { -0.3 } else { 0.3 }, 0.0, if z > 0.0 { -0.3 } else { 0.3 });
            made.plate.sample_world(Vec3::new(x, 1.0, z) + inward) < 0.0
        };
        assert!(!just_inside(0.0, -pitch * 0.5), "the marked corner is still there");
        assert!(just_inside(0.0, pitch * 0.5), "the other corner of the tight end is cut too");
        assert!(just_inside(length, -pitch * 0.5) && just_inside(length, pitch * 0.5));
        // The mark does not reach the first socket's wall.
        let first = made.sockets[0];
        let toward = Vec3::new(-1.0, 0.0, -1.0).normalize();
        let rim = spec.radius_mm + CLEARANCES_MM[0] + spec.wall_mm * 0.5;
        assert!(made.plate.sample_world(first + toward * rim - Vec3::Y * 1.0) < 0.0);
    }

    /// A pin hole and a magnet pocket are tested with the pin and the
    /// magnet: a plate of holes and no peg, each hole the key's own size
    /// and its clearance.
    #[test]
    fn a_pin_hole_and_a_magnet_pocket_are_a_plate_and_no_peg() {
        for kind in [KeyKind::PinHole, KeyKind::MagnetPocket] {
            let spec = KeySpec { kind, ..KeySpec::peg() };
            let made = coupon(spec, VOXEL).expect("a hole of 2 mm at 0.1 mm");
            assert!(made.peg.is_none(), "a {kind} came with a peg");
            assert!(made.plate.export_mesh().1.is_printable());
            let top = made.sockets[0].y;
            for (socket, clearance) in made.sockets.iter().zip(CLEARANCES_MM) {
                let radius = radius_at(&made.plate, *socket, top - spec.depth_mm * 0.5);
                assert!(
                    (radius - spec.radius_mm - clearance).abs() < 0.3 * VOXEL,
                    "the {kind} for {clearance} mm is {radius} mm in radius"
                );
            }
        }
    }

    /// A key the lattice cannot hold is refused, with the voxel size that
    /// would hold it, and one it can hold at that size is made.
    #[test]
    fn a_key_too_narrow_for_the_lattice_is_refused_with_the_voxel_that_would_hold_it() {
        let spec = KeySpec { radius_mm: 0.5, ..KeySpec::peg() };
        let Err(CouponError::TooNarrow { voxel_mm }) = coupon(spec, 0.5).map(|_| ()) else {
            panic!("a key 1 mm across was made on a lattice of 0.5 mm");
        };
        assert!((voxel_mm - 1.0 / 3.0).abs() < 1e-6, "{voxel_mm}");
        assert!(CouponError::TooNarrow { voxel_mm }.to_string().contains("0.33 mm"));
        assert!(coupon(spec, voxel_mm).is_ok());
        assert!(coupon(spec, voxel_mm * 1.01).is_err());
    }
}
