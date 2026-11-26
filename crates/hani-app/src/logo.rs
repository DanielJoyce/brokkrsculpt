// SPDX-License-Identifier: AGPL-3.0-only

//! The brand: Haniko, HaniSculpt's mascot, as the mark in the title bar, the
//! lockup on the welcome screen, and the window's icon.
//!
//! The pictures are `assets/brand/haniko/`, made from the originals by
//! `scripts/brand-assets.py`, and who Haniko is is `docs/HANIKO.md`. Each is
//! a PNG compiled into the binary and decoded once, on first use: the
//! `image` crate already reads PNG for decals, so this costs no crate.
//!
//! This replaced a mark drawn one rectangle at a time (a voxel with a chisel,
//! upstream's), on 2026-11-26 at the maintainer's word: *"switch the branding
//! to haniko"*.
//!
//! # The window icon
//!
//! Set from the badge. X11 and Windows show it; Wayland has no protocol for a
//! client to hand its compositor an icon, and takes it from the `.desktop`
//! file whose name is the window's app id, `hanisculpt`, and the icon theme.
//! `desktop_entry.rs` installs both for the user on Linux.

use std::sync::OnceLock;

use iced::widget::image;
use iced::widget::image::Handle;
use iced::{Element, Length};

/// The badge, Haniko in a ring, at 128 pixels: sharp at every size the
/// interface draws it, at twice that on a HiDPI screen.
pub(crate) const BADGE: &[u8] =
    include_bytes!("../../../assets/brand/haniko/hanisculpt-icon-128.png");
/// The badge at 256, for the window icon.
pub(crate) const ICON: &[u8] =
    include_bytes!("../../../assets/brand/haniko/hanisculpt-icon-256.png");
/// Haniko and the name, the ground transparent and every letter in one
/// orange, so it reads on a light background or a dark one.
const LOCKUP: &[u8] = include_bytes!("../../../assets/brand/haniko/hanisculpt-lockup.png");

/// A PNG as width, height and RGBA bytes. These are compiled in, so one that
/// does not decode is a broken build, and a test below says so.
fn decoded(png: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let picture = ::image::load_from_memory_with_format(png, ::image::ImageFormat::Png).ok()?;
    let rgba = picture.to_rgba8();
    Some((rgba.width(), rgba.height(), rgba.into_raw()))
}

/// The handle for a compiled-in picture, made once so its id is stable and
/// the renderer uploads it once.
fn handle(cell: &'static OnceLock<Handle>, png: &'static [u8]) -> Handle {
    cell.get_or_init(|| match decoded(png) {
        Some((width, height, pixels)) => Handle::from_rgba(width, height, pixels),
        None => Handle::from_rgba(1, 1, vec![0; 4]),
    })
    .clone()
}

/// The mark: the badge, `side` pixels square.
pub fn mark<'a, Message: 'a>(side: f32) -> Element<'a, Message> {
    static CELL: OnceLock<Handle> = OnceLock::new();
    image(handle(&CELL, BADGE)).width(Length::Fixed(side)).height(Length::Fixed(side)).into()
}

/// The lockup, Haniko and the name, `height` pixels tall.
pub fn lockup<'a, Message: 'a>(height: f32) -> Element<'a, Message> {
    static CELL: OnceLock<Handle> = OnceLock::new();
    image(handle(&CELL, LOCKUP)).height(Length::Fixed(height)).into()
}

/// The window's icon, from the badge.
pub fn window_icon() -> Option<iced::window::Icon> {
    let (width, height, pixels) = decoded(ICON)?;
    iced::window::icon::from_rgba(pixels, width, height).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every compiled-in picture decodes, at the size its name says, and the
    /// badge's corners are transparent while its middle is not.
    #[test]
    fn the_pictures_decode() {
        let (width, height, pixels) = decoded(BADGE).expect("the badge decodes");
        assert_eq!((width, height), (128, 128));
        assert_eq!(pixels[3], 0, "the badge's corner is transparent");
        let middle = ((64 * 128 + 64) * 4 + 3) as usize;
        assert_eq!(pixels[middle], 255, "the badge's middle is not");
        assert_eq!(decoded(ICON).map(|(w, h, _)| (w, h)), Some((256, 256)));
        let (width, height, _) = decoded(LOCKUP).expect("the lockup decodes");
        assert!(width > height * 2, "the lockup is wide: {width} by {height}");
        assert!(window_icon().is_some());
    }
}
