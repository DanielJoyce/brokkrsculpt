// SPDX-License-Identifier: AGPL-3.0-only

//! The layers of paint in a project file: one optional section after the
//! paint's own. See [`super::layers`].
//!
//! The paint section ([`super::file`]) holds each body's paint as it is
//! seen, every layer put together, so that a reader from before layers
//! opens a layered document looking as it did. This section, after it,
//! holds the stack itself, and a reader that knows it puts each body's
//! stack back over what the paint section gave. A document whose every
//! body is one plain layer writes no section, and gains no bytes.
//!
//! # The bytes
//!
//! ```text
//! u32   bodies            every body of the document, in its own order
//! for each body:
//!   u8  0: one plain layer, nothing follows; 1: a stack follows
//!   u32 active            the active layer, bottom being 0
//!   u32 layers
//!   for each layer, bottom to top:
//!     u32 id
//!     u16 name length, then that many bytes of UTF-8
//!     u8  blend           see `Blend::code`
//!     f32 opacity
//!     u8  1: shown; 2: locked
//!   u64 length of the layers' paint, then a paint section of that many
//!       bytes holding one field a layer (0 when every layer is empty)
//! ```
//!
//! All little endian. Strict as the paint section is: a refusal costs the
//! stack, and each body keeps the paint as it was seen, as one layer.

use super::layers::{Blend, LayerMeta, LayerState, MAX_LAYERS};

/// What marks the section.
pub(crate) const TAG: &[u8; 8] = b"BRKLAYR1";

const SHOWN: u8 = 1;
const LOCKED: u8 = 2;

/// The section for these bodies' stacks, `None` for a plain one, or `None`
/// when every body is plain.
pub(crate) fn encode(bodies: &[Option<LayerState>]) -> Option<Vec<u8>> {
    if bodies.iter().all(Option::is_none) {
        return None;
    }
    let mut out = Vec::new();
    out.extend_from_slice(&(bodies.len() as u32).to_le_bytes());
    for body in bodies {
        let Some(state) = body else {
            out.push(0);
            continue;
        };
        out.push(1);
        out.extend_from_slice(&(state.active as u32).to_le_bytes());
        out.extend_from_slice(&(state.metas.len() as u32).to_le_bytes());
        for meta in &state.metas {
            out.extend_from_slice(&meta.id.to_le_bytes());
            let name = meta.name.as_bytes();
            let name = &name[..name.len().min(u16::MAX as usize)];
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(name);
            out.push(meta.blend.code());
            out.extend_from_slice(&meta.opacity.to_le_bytes());
            out.push(if meta.visible { SHOWN } else { 0 } | if meta.locked { LOCKED } else { 0 });
        }
        let paint = super::file::encode(state.fields.iter()).unwrap_or_default();
        out.extend_from_slice(&(paint.len() as u64).to_le_bytes());
        out.extend_from_slice(&paint);
    }
    Some(out)
}

/// Why a section could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Refused;

/// Read a section back into a stack, or `None` for a plain one, for each of
/// `bodies` bodies.
pub(crate) fn decode(bytes: &[u8], bodies: usize) -> Result<Vec<Option<LayerState>>, Refused> {
    let mut rest = bytes;
    let mut take = |count: usize| -> Result<&[u8], Refused> {
        if rest.len() < count {
            return Err(Refused);
        }
        let (head, tail) = rest.split_at(count);
        rest = tail;
        Ok(head)
    };
    let word = |bytes: &[u8]| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    if word(take(4)?) as usize != bodies {
        return Err(Refused);
    }
    let mut out = Vec::with_capacity(bodies);
    for _ in 0..bodies {
        match take(1)?[0] {
            0 => {
                out.push(None);
                continue;
            }
            1 => {}
            _ => return Err(Refused),
        }
        let active = word(take(4)?) as usize;
        let count = word(take(4)?) as usize;
        if count == 0 || count > MAX_LAYERS || active >= count {
            return Err(Refused);
        }
        let mut metas = Vec::with_capacity(count);
        for _ in 0..count {
            let id = word(take(4)?);
            let length = take(2)?;
            let length = u16::from_le_bytes([length[0], length[1]]) as usize;
            let name = String::from_utf8(take(length)?.to_vec()).map_err(|_| Refused)?;
            let blend = Blend::from_code(take(1)?[0]).ok_or(Refused)?;
            let opacity = f32::from_le_bytes(take(4)?.try_into().map_err(|_| Refused)?);
            if !(0.0..=1.0).contains(&opacity) {
                return Err(Refused);
            }
            let flags = take(1)?[0];
            metas.push(LayerMeta {
                id,
                name,
                blend,
                opacity,
                visible: flags & SHOWN != 0,
                locked: flags & LOCKED != 0,
            });
        }
        let mut ids: Vec<u32> = metas.iter().map(|meta| meta.id).collect();
        ids.sort_unstable();
        ids.dedup();
        if ids.len() != count {
            return Err(Refused);
        }
        let length = take(8)?;
        let length = u64::from_le_bytes(length.try_into().map_err(|_| Refused)?);
        let length = usize::try_from(length).map_err(|_| Refused)?;
        let fields = if length == 0 {
            vec![super::field::PaintField::default(); count]
        } else {
            super::file::decode(take(length)?, count).map_err(|_| Refused)?
        };
        out.push(Some(LayerState { metas, fields, active }));
    }
    if !rest.is_empty() {
        return Err(Refused);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Volume;
    use crate::paint::field::PaintField;
    use crate::paint::field::Rgba;
    use glam::{IVec3, Vec3};

    fn painted_ball() -> Volume {
        let mut volume = Volume::new(1.0);
        volume.seed_sphere(Vec3::ZERO, 10.0);
        volume
    }

    fn coat(volume: &mut Volume, rgba: Rgba) {
        volume.edit_paint(IVec3::splat(-16), IVec3::splat(16), |_, _, _, _| rgba);
    }

    fn same(one: &PaintField, other: &PaintField) -> bool {
        let mut a: Vec<_> = one.bricks().keys().copied().collect();
        let mut b: Vec<_> = other.bricks().keys().copied().collect();
        a.sort_unstable();
        b.sort_unstable();
        a == b && a.iter().all(|coord| one.bricks()[coord] == other.bricks()[coord])
    }

    /// A stack written and read back is the stack, and a plain body stays
    /// plain.
    #[test]
    fn a_stack_reads_back_as_it_was_written() {
        let mut volume = painted_ball();
        coat(&mut volume, [200, 30, 30, 255]);
        volume.add_layer("Shade", Blend::Wash).expect("room");
        coat(&mut volume, [20, 20, 60, 128]);
        let mut meta = volume.layers().metas()[1].clone();
        meta.opacity = 0.25;
        meta.locked = true;
        volume.set_layer_meta(1, meta);
        volume.add_layer("Empty", Blend::Glaze).expect("room");
        volume.set_active_layer(1);
        let state = volume.layer_state();

        let bytes = encode(&[None, Some(state.clone())]).expect("a section");
        let back = decode(&bytes, 2).expect("it reads");
        assert!(back[0].is_none());
        let back = back[1].as_ref().expect("a stack");
        assert_eq!(back.metas, state.metas);
        assert_eq!(back.active, state.active);
        for (one, other) in back.fields.iter().zip(&state.fields) {
            assert!(same(one, other), "a layer's paint changed");
        }
        assert!(encode(&[None, None]).is_none(), "plain bodies gained bytes");
    }

    /// Anything cut short or out of its bounds is refused, never a panic.
    #[test]
    fn a_damaged_section_is_refused() {
        let mut volume = painted_ball();
        coat(&mut volume, [200, 30, 30, 255]);
        volume.add_layer("Top", Blend::Multiply).expect("room");
        let bytes = encode(&[Some(volume.layer_state())]).expect("a section");
        for cut in 0..bytes.len() {
            assert!(decode(&bytes[..cut], 1).is_err(), "cut at {cut}");
        }
        assert!(decode(&bytes, 2).is_err(), "the wrong count of bodies");
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(decode(&longer, 1).is_err(), "bytes after the end");
    }
}
