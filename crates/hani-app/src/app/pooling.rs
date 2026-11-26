// SPDX-License-Identifier: AGPL-3.0-only

//! How strongly washes and contrasts pool: one number for the paint
//! engine, set on every body. See `hani_core::paint::occlusion`.
//!
//! Asked for by the maintainer on 2026-11-25: "Contrast / speed paints /
//! washes tend to pull in and around details ... I think maybe we just need
//! a AO intensity slider for the paint engine." It is *Pooling* in Paint's
//! LAYERS block, which sets it on every body at once; a body made later (a
//! split, a new body, a file opened) is given it on the next frame. It is
//! a setting, `paint.conf`, kept when the slider is let go.

use iced::Element;

use super::Hani;
use crate::message::{Message, PaintingMessage};

/// The setting's file.
const FILE: &str = "paint.conf";

/// The pooling the settings hold, or the usual. Not read by a test, which
/// must not depend on the user's files.
pub(super) fn kept() -> f32 {
    use hani_core::paint::occlusion::{POOLING, POOLING_RANGE};
    if cfg!(test) {
        return POOLING;
    }
    crate::paths::config_file(FILE)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| {
            crate::paths::entries(&text)
                .find(|(key, _)| *key == "pooling")
                .and_then(|(_, value)| value.parse::<f32>().ok())
        })
        .filter(|value| value.is_finite())
        .map_or(POOLING, |value| value.clamp(*POOLING_RANGE.start(), *POOLING_RANGE.end()))
}

fn keep(pooling: f32) -> Result<(), String> {
    if cfg!(test) {
        return Ok(());
    }
    let path = crate::paths::config_file(FILE).ok_or("no settings directory")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|why| why.to_string())?;
    }
    std::fs::write(path, format!("pooling = {pooling}\n")).map_err(|why| why.to_string())
}

impl Hani {
    /// A new pooling from the slider: on every body now, and the bricks
    /// that pool put together again.
    pub(super) fn pool(&mut self, pooling: f32) {
        use hani_core::paint::occlusion::POOLING_RANGE;
        self.painting.pooling = pooling.clamp(*POOLING_RANGE.start(), *POOLING_RANGE.end());
        self.keep_pooling();
        self.remesh_dirty();
    }

    /// The slider let go: keep the setting.
    pub(super) fn pooling_kept(&mut self) {
        if let Err(why) = keep(self.painting.pooling) {
            self.status = format!("the pooling was not kept: {why}");
        }
    }

    /// Give every body the pooling in hand, where one has another: run on
    /// every frame, so that a body made since is given it. A comparison a
    /// body when nothing changed.
    pub(super) fn keep_pooling(&mut self) {
        let pooling = self.painting.pooling;
        for (_, volume) in self.doc.bodies_mut() {
            if volume.pooling() != pooling {
                volume.set_pooling(pooling);
            }
        }
    }

    /// The number in the LAYERS block.
    pub(super) fn pooling_number(&self) -> Element<'_, Message> {
        use hani_core::paint::occlusion::POOLING_RANGE;
        crate::ui::number(
            &self.drafts,
            "paint.pooling",
            "Pooling",
            self.painting.pooling,
            POOLING_RANGE,
            |pooling| Message::Painting(PaintingMessage::Pooling(pooling)),
        )
        .on_release(Message::Painting(PaintingMessage::PoolingKept))
        .shown(format!("{:.2}", self.painting.pooling))
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::LayersMessage;
    use hani_core::paint::layers::Blend;

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    /// The slider sets every body, and a body made after it is given it on
    /// the next frame; the LAYERS block draws with it.
    #[test]
    fn the_pooling_is_on_every_body() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::BrushKindChanged(hani_core::BrushKind::Paint));
        update(&mut app, Message::Layers(LayersMessage::Added));
        update(&mut app, Message::Layers(LayersMessage::Blended(1, Blend::Wash)));
        update(&mut app, Message::Painting(PaintingMessage::Pooling(2.5)));
        update(&mut app, Message::Painting(PaintingMessage::PoolingKept));
        assert!(app.doc.bodies().all(|(_, volume)| volume.pooling() == 2.5));
        let mut other = hani_core::Volume::new(app.doc.voxel_size());
        other.seed_sphere(glam::Vec3::new(40.0, 0.0, 0.0), 5.0);
        app.doc.add_body("Other", other);
        update(&mut app, Message::Frame);
        assert!(app.doc.bodies().all(|(_, volume)| volume.pooling() == 2.5));
        update(&mut app, Message::Painting(PaintingMessage::Pooling(99.0)));
        assert_eq!(app.painting.pooling, 3.0);
        drop(app.pooling_number());
    }
}
