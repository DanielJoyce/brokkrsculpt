// SPDX-License-Identifier: AGPL-3.0-only
//
// A see-through row of a boolean group, tinted by what it does: red for a
// row that cuts, blue for one that keeps.
//
// Asked for by the maintainer on 2026-11-28, of the boolean groups'
// not-done list: "continue on those gaps". The rows that cut or keep are
// drawn see-through while their group is in hand (`app/booleans.rs`), and
// until now in clay like everything else, so which did what could only be
// read off the list.
//
// Carried in the spread entry's fourth component beside the share screened
// away, which is below one: two for each step of the tint's code, so that a
// body not tinted keeps the number it had. `sculpt.wgsl` takes it apart with
// [`untinted`]'s arithmetic, and the test below holds the two together.

/// Drawn as it is.
pub const NONE: u8 = 0;
/// A row that cuts: shifted toward red.
pub const CUT: u8 = 1;
/// A row that keeps: shifted toward blue.
pub const KEEP: u8 = 2;

/// A spread entry's fourth component for a body screened by `screened` and
/// tinted by `tint`.
pub fn screened_and_tinted(screened: f32, tint: u8) -> f32 {
    screened.clamp(0.0, 0.99) + 2.0 * f32::from(tint)
}

/// The screened share and the tint back out of a fourth component, as the
/// shader takes them.
pub fn untinted(w: f32) -> (f32, u8) {
    let tint = (w * 0.5).floor();
    (w - tint * 2.0, tint as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every share and tint comes back out as it went in, and a body not
    /// tinted keeps the number it had.
    #[test]
    fn a_share_and_a_tint_come_back_out() {
        for tint in [NONE, CUT, KEEP] {
            for screened in [0.0, 0.25, 0.7, 0.99] {
                let (share, back) = untinted(screened_and_tinted(screened, tint));
                assert_eq!(back, tint);
                assert!((share - screened).abs() < 1.0e-6, "{screened} came back {share}");
            }
        }
        assert_eq!(screened_and_tinted(0.7, NONE), 0.7);
        let shader = include_str!("shaders/sculpt.wgsl");
        assert!(shader.contains("let tint = floor(spread.w * 0.5);"), "the shader's decode moved");
        assert!(shader.contains("out.screened = spread.w - tint * 2.0;"));
    }
}
