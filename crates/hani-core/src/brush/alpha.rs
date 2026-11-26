// SPDX-License-Identifier: AGPL-3.0-only

//! A stamp image: a square of heights a brush can press into a surface.
//!
//! A rivet, a scale, a rune, a row of stitches. Sculpted by hand each one is
//! a minute's careful work and a model wants forty of them. As an image it
//! is a drag. Sculpting programs call the image an alpha, and so does this.
//!
//! An alpha here is nothing but heights, 0 for the surface as it is and 1
//! for the top of the stamp, on a square grid. Where the heights come from
//! is not this crate's business. The ones built in are worked out from a
//! formula, and a picture is decoded by the application and handed over as
//! numbers ([`Alpha::from_heights`]), so this crate stays free of file
//! formats.
//!
//! # Reading one
//!
//! [`Alpha::sample`] takes a point in the stamp's own square, each axis from
//! -1 to 1, and blends the four heights round it. Outside the square is 0,
//! and so that a stamp never ends in a step the caller did not draw, the
//! outermost ring of every alpha is read as 0 as well: an image whose edge
//! pixels are not black fades to the surface over its last texel instead of
//! leaving a cliff the shape of the square.

/// The most texels along one side of an alpha. A stamp is a few dozen voxels
/// across on the model, so anything finer than this is detail no voxel will
/// hold, and a 16 megapixel photograph dropped onto the window is not 64 MB
/// of heights.
pub const MAX_ALPHA_SIZE: usize = 512;

/// A square of heights in `0..=1`.
#[derive(Debug, Clone, PartialEq)]
pub struct Alpha {
    size: usize,
    heights: Vec<f32>,
}

/// Why a set of heights is not an alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaError {
    /// Fewer than two texels a side: nothing to blend between.
    TooSmall,
    /// The number of heights is not `width * height`.
    WrongCount { expected: usize, found: usize },
    /// A height is not a number.
    NotFinite,
}

impl std::fmt::Display for AlphaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AlphaError::TooSmall => f.write_str("a stamp needs at least two pixels a side"),
            AlphaError::WrongCount { expected, found } => {
                write!(f, "a stamp of that size has {expected} heights and {found} were given")
            }
            AlphaError::NotFinite => f.write_str("a stamp holds a height that is not a number"),
        }
    }
}

/// The alphas that come with the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltIn {
    /// A smooth dome: a stud, a boss, a wart.
    Dome,
    /// A flat-topped disc with a bevelled side: a rivet head, a button.
    Rivet,
    /// A dome inside a raised ring: a bolt in its washer.
    Bolt,
    /// A rounded scale, proud at its lower edge: dragon hide, roof tiles.
    Scale,
    /// A four-pointed star.
    Star,
    /// A straight bar with rounded ends: a stitch, a slot, a staple.
    Bar,
}

impl BuiltIn {
    pub const ALL: [BuiltIn; 6] =
        [BuiltIn::Dome, BuiltIn::Rivet, BuiltIn::Bolt, BuiltIn::Scale, BuiltIn::Star, BuiltIn::Bar];

    pub fn label(self) -> &'static str {
        match self {
            BuiltIn::Dome => "Dome",
            BuiltIn::Rivet => "Rivet",
            BuiltIn::Bolt => "Bolt",
            BuiltIn::Scale => "Scale",
            BuiltIn::Star => "Star",
            BuiltIn::Bar => "Bar",
        }
    }

    /// The height at a point of the stamp's square, each axis in `-1..=1`.
    fn height(self, x: f32, y: f32) -> f32 {
        let r = (x * x + y * y).sqrt();
        // 1 inside `edge`, 0 past `edge + bevel`, a smooth ramp between.
        let disc = |r: f32, edge: f32, bevel: f32| 1.0 - smoothstep(edge, edge + bevel, r);
        match self {
            BuiltIn::Dome => (1.0 - (r / 0.9) * (r / 0.9)).max(0.0).sqrt(),
            BuiltIn::Rivet => disc(r, 0.6, 0.25),
            BuiltIn::Bolt => {
                let washer = 0.45 * disc(r, 0.75, 0.15);
                let head = (1.0 - (r / 0.5) * (r / 0.5)).max(0.0).sqrt();
                washer.max(head)
            }
            BuiltIn::Scale => {
                // A disc, tilted: highest toward the bottom of the square,
                // where it would overlap the next row.
                disc(r, 0.7, 0.2) * (0.35 + 0.65 * (0.5 - 0.5 * y))
            }
            BuiltIn::Star => {
                // Distance from the nearer diagonal arm, thinned toward the
                // tips.
                let (ax, ay) = (x.abs(), y.abs());
                let along = ax.max(ay);
                let across = ax.min(ay);
                let width = 0.28 * (1.0 - along / 0.92).max(0.0);
                if width <= 0.0 { 0.0 } else { (1.0 - across / width).clamp(0.0, 1.0) }
            }
            BuiltIn::Bar => {
                // A capsule lying along x.
                let dx = (x.abs() - 0.55).max(0.0);
                let d = (dx * dx + y * y).sqrt();
                (1.0 - (d / 0.28) * (d / 0.28)).max(0.0).sqrt()
            }
        }
    }
}

fn smoothstep(from: f32, to: f32, x: f32) -> f32 {
    let t = ((x - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Alpha {
    /// An alpha from heights given a row at a time, the top row first.
    ///
    /// Each height is clamped to `0..=1`. A rectangle is centred in a square
    /// of its longer side, the rest of which is 0, so a wide image is a wide
    /// stamp and not a stretched one. An image larger than
    /// [`MAX_ALPHA_SIZE`] a side is averaged down to it.
    pub fn from_heights(width: usize, height: usize, heights: &[f32]) -> Result<Self, AlphaError> {
        if width < 2 || height < 2 {
            return Err(AlphaError::TooSmall);
        }
        let expected = width.checked_mul(height).ok_or(AlphaError::TooSmall)?;
        if heights.len() != expected {
            return Err(AlphaError::WrongCount { expected, found: heights.len() });
        }
        if heights.iter().any(|height| !height.is_finite()) {
            return Err(AlphaError::NotFinite);
        }

        let longer = width.max(height);
        // How many source texels one texel of the alpha stands for.
        let step = longer.div_ceil(MAX_ALPHA_SIZE).max(1);
        let size = longer.div_ceil(step).max(2);
        // Where the rectangle sits in the square, in source texels.
        let (left, top) = ((longer - width) / 2, (longer - height) / 2);
        let mut out = vec![0.0_f32; size * size];
        for (index, texel) in out.iter_mut().enumerate() {
            let (column, row) = (index % size, index / size);
            let (mut sum, mut count) = (0.0, 0);
            for sy in row * step..((row + 1) * step).min(longer) {
                for sx in column * step..((column + 1) * step).min(longer) {
                    count += 1;
                    if sx >= left && sx < left + width && sy >= top && sy < top + height {
                        sum += heights[(sy - top) * width + (sx - left)].clamp(0.0, 1.0);
                    }
                }
            }
            if count > 0 {
                *texel = sum / count as f32;
            }
        }
        Ok(Self { size, heights: out })
    }

    /// One of the alphas that come with the application, at 96 texels a
    /// side: finer than any stamp is voxels across at the sizes a model is
    /// sculpted at, and a formula costs nothing to keep.
    pub fn built_in(kind: BuiltIn) -> Self {
        const SIZE: usize = 96;
        let mut heights = Vec::with_capacity(SIZE * SIZE);
        for row in 0..SIZE {
            for column in 0..SIZE {
                // Texel centres, with +y up the square and the first row at
                // the top.
                let x = (column as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0;
                let y = 1.0 - (row as f32 + 0.5) / SIZE as f32 * 2.0;
                heights.push(kind.height(x, y).clamp(0.0, 1.0));
            }
        }
        Self { size: SIZE, heights }
    }

    /// Texels along one side.
    pub fn size(&self) -> usize {
        self.size
    }

    /// The height at a texel, or 0 for one off the square or on its
    /// outermost ring. See the module documentation for why the ring.
    fn texel(&self, column: i64, row: i64) -> f32 {
        let last = self.size as i64 - 1;
        if column <= 0 || row <= 0 || column >= last || row >= last {
            return 0.0;
        }
        self.heights[row as usize * self.size + column as usize]
    }

    /// The height at a point of the stamp's square: `u` from -1 at the left
    /// to 1 at the right, `v` from -1 at the bottom to 1 at the top. 0
    /// outside it.
    pub fn sample(&self, u: f32, v: f32) -> f32 {
        // The bounds are a shortcut and not the rule: every texel off the
        // square already reads as 0, so without them the answer is the same
        // and a stamp's whole box pays for four lookups a voxel. The NaN is
        // the rule: it would otherwise become a texel index.
        if !(u.is_finite() && v.is_finite()) || u.abs() >= 1.0 || v.abs() >= 1.0 {
            return 0.0;
        }
        // Texel centres are at half-integers, so the texel a point is in is
        // `floor`, and the four to blend are the ones whose centres surround
        // it.
        let x = (u + 1.0) * 0.5 * self.size as f32 - 0.5;
        let y = (1.0 - v) * 0.5 * self.size as f32 - 0.5;
        let (column, row) = (x.floor(), y.floor());
        let (fx, fy) = (x - column, y - row);
        let (column, row) = (column as i64, row as i64);
        let top = self.texel(column, row) * (1.0 - fx) + self.texel(column + 1, row) * fx;
        let bottom =
            self.texel(column, row + 1) * (1.0 - fx) + self.texel(column + 1, row + 1) * fx;
        top * (1.0 - fy) + bottom * fy
    }

    /// The heights as bytes, a row at a time from the top, for a thumbnail.
    pub fn as_bytes(&self) -> Vec<u8> {
        self.heights.iter().map(|height| (height * 255.0).round() as u8).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Noise;

    /// Every built-in is a stamp: something in the middle, nothing at the
    /// rim, every height a height.
    #[test]
    fn every_built_in_is_a_raised_shape_that_ends_at_the_surface() {
        for kind in BuiltIn::ALL {
            let alpha = Alpha::built_in(kind);
            assert!(!kind.label().is_empty());
            let mut highest: f32 = 0.0;
            for step in 0..=40 {
                for other in 0..=40 {
                    let (u, v) = (step as f32 / 20.0 - 1.0, other as f32 / 20.0 - 1.0);
                    let height = alpha.sample(u, v);
                    assert!((0.0..=1.0).contains(&height), "{kind:?} at {u} {v} is {height}");
                    highest = highest.max(height);
                    if u.abs().max(v.abs()) > 0.97 {
                        assert!(height < 0.02, "{kind:?} is {height} high at its rim, {u} {v}");
                    }
                }
            }
            assert!(highest > 0.9, "{kind:?} rises only to {highest}");
            assert_eq!(alpha.sample(1.5, 0.0), 0.0, "{kind:?} is not flat outside its square");
            assert_eq!(alpha.sample(f32::NAN, 0.0), 0.0);
        }
        // Each is its own shape: no two agree everywhere.
        for (index, one) in BuiltIn::ALL.iter().enumerate() {
            for other in &BuiltIn::ALL[index + 1..] {
                assert_ne!(Alpha::built_in(*one), Alpha::built_in(*other), "{one:?} is {other:?}");
            }
        }
    }

    /// Which way up and which way round: the scale is proud at the bottom of
    /// its square, and an image's first row is the top of the stamp.
    #[test]
    fn the_square_is_the_right_way_up_and_the_right_way_round() {
        let scale = Alpha::built_in(BuiltIn::Scale);
        assert!(
            scale.sample(0.0, -0.4) > scale.sample(0.0, 0.4) + 0.2,
            "a scale is not higher toward the bottom of its square"
        );

        // Four by four, one bright texel in the second row and third column:
        // above the middle and to the right of it.
        let mut heights = vec![0.0; 16];
        heights[4 + 2] = 1.0;
        let alpha = Alpha::from_heights(4, 4, &heights).unwrap();
        assert_eq!(alpha.size(), 4);
        // The texel's centre is at u = 0.25, v = 0.25.
        assert!((alpha.sample(0.25, 0.25) - 1.0).abs() < 1.0e-6);
        assert_eq!(alpha.sample(-0.25, 0.25), 0.0, "the image is mirrored left to right");
        assert_eq!(alpha.sample(0.25, -0.25), 0.0, "the image is upside down");
        // Half way to the next texel, half the height.
        assert!((alpha.sample(0.0, 0.25) - 0.5).abs() < 1.0e-6);
    }

    /// An image's edge pixels do not make a cliff: the outer ring reads as
    /// the surface, however bright the image is there.
    #[test]
    fn a_stamp_never_ends_in_a_step_the_shape_of_its_square() {
        let alpha = Alpha::from_heights(8, 8, &[1.0; 64]).unwrap();
        assert!((alpha.sample(0.0, 0.0) - 1.0).abs() < 1.0e-6, "the middle of a white image");
        for at in [-0.99, 0.99] {
            assert!(alpha.sample(at, 0.0) < 0.1, "a white image is a cliff at u = {at}");
            assert!(alpha.sample(0.0, at) < 0.1, "a white image is a cliff at v = {at}");
        }
    }

    /// A wide image is a wide stamp, centred, and a huge one is brought down
    /// to size with its heights averaged and not sampled.
    #[test]
    fn a_rectangle_is_centred_and_a_huge_image_is_averaged_down() {
        // Eight wide, four high, all white: the stamp is the middle half of
        // the square's height.
        let wide = Alpha::from_heights(8, 4, &[1.0; 32]).unwrap();
        assert_eq!(wide.size(), 8);
        assert!(wide.sample(0.0, 0.1) > 0.9);
        assert_eq!(wide.sample(0.0, 0.8), 0.0, "a wide image was stretched to the square");

        // A checkerboard far larger than an alpha may be: every texel of
        // the result stands for a block that is half white.
        let side = MAX_ALPHA_SIZE * 2;
        let board: Vec<f32> =
            (0..side * side).map(|index| ((index % side + index / side) % 2) as f32).collect();
        let small = Alpha::from_heights(side, side, &board).unwrap();
        assert_eq!(small.size(), MAX_ALPHA_SIZE);
        assert!((small.sample(0.1, 0.1) - 0.5).abs() < 1.0e-6, "a checkerboard was not averaged");
    }

    #[test]
    fn heights_that_are_not_an_image_are_refused_by_name() {
        assert_eq!(Alpha::from_heights(1, 5, &[0.0; 5]), Err(AlphaError::TooSmall));
        assert_eq!(Alpha::from_heights(5, 0, &[]), Err(AlphaError::TooSmall));
        assert_eq!(
            Alpha::from_heights(3, 3, &[0.0; 8]),
            Err(AlphaError::WrongCount { expected: 9, found: 8 })
        );
        assert_eq!(Alpha::from_heights(usize::MAX, 2, &[0.0; 4]), Err(AlphaError::TooSmall));
        let mut heights = [0.5; 9];
        heights[4] = f32::NAN;
        assert_eq!(Alpha::from_heights(3, 3, &heights), Err(AlphaError::NotFinite));
        // Out of range is not an error. It is clamped.
        let alpha = Alpha::from_heights(4, 4, &[7.0; 16]).unwrap();
        assert!((alpha.sample(0.0, 0.0) - 1.0).abs() < 1.0e-6);
        let alpha = Alpha::from_heights(4, 4, &[-7.0; 16]).unwrap();
        assert_eq!(alpha.sample(0.0, 0.0), 0.0);
        for error in [
            AlphaError::TooSmall,
            AlphaError::NotFinite,
            AlphaError::WrongCount { expected: 1, found: 2 },
        ] {
            assert!(!error.to_string().is_empty());
        }
    }

    /// Whatever heights and whatever sizes, an alpha is either refused or
    /// reads as heights everywhere it is asked.
    #[test]
    fn any_heights_at_all_give_an_alpha_that_reads_as_heights_or_none() {
        let mut noise = Noise::seeded(0xA1FA);
        let (mut made, mut refused) = (0, 0);
        for _ in 0..400 {
            let (width, height) = (noise.below(40), noise.below(40));
            let count = if noise.below(6) == 0 { noise.below(1700) } else { width * height };
            // One image in five has something in it that is not a height.
            // Every texel rolling for that, which the first version did,
            // poisons every image of more than a few texels and leaves the
            // accepting half of this test with nothing to look at: the
            // control below caught that, at 0 of 400.
            let poisoned = noise.below(5) == 0;
            let heights: Vec<f32> = (0..count)
                .map(|_| match noise.below(if poisoned { 12 } else { 3 }) + 3 {
                    12 => f32::NAN,
                    13 => f32::INFINITY,
                    14 => -1.0e30,
                    _ => noise.byte() as f32 / 200.0,
                })
                .collect();
            match Alpha::from_heights(width, height, &heights) {
                Ok(alpha) => {
                    made += 1;
                    assert!(alpha.size() >= 2 && alpha.size() <= MAX_ALPHA_SIZE);
                    for _ in 0..20 {
                        let (u, v) =
                            (noise.byte() as f32 / 100.0 - 1.3, noise.byte() as f32 / 100.0 - 1.3);
                        let read = alpha.sample(u, v);
                        assert!((0.0..=1.0).contains(&read), "{read} at {u} {v}");
                    }
                    assert_eq!(alpha.as_bytes().len(), alpha.size() * alpha.size());
                }
                Err(_) => refused += 1,
            }
        }
        // The control: a maker that refused everything would pass the loop.
        assert!(made > 20, "only {made} of 400 were accepted");
        assert!(refused > 100, "only {refused} of 400 were refused");
    }
}
