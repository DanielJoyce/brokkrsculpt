// SPDX-License-Identifier: AGPL-3.0-only

//! How strongly washes and contrasts pool: one number for the paint
//! engine, set on every body. See `hani_core::paint::occlusion`.
//!
//! Asked for by the maintainer on 2026-11-25: "Contrast / speed paints /
//! washes tend to pull in and around details ... I think maybe we just need
//! a AO intensity slider for the paint engine." It is *Pooling* in Paint's
//! LAYERS block, which sets it on every body at once (and, since
//! 2026-11-28, how far it reaches round a recess, beside it, in the Wash
//! and Contrast touches and in Preferences too); a body made later (a
//! split, a new body, a file opened) is given it on the next frame. It is
//! a setting, `paint.conf`, kept when the slider is let go.

use iced::Element;

use super::Hani;
use crate::message::{Message, PaintingMessage};

/// The setting's file.
const FILE: &str = "paint.conf";

/// One number the settings hold, or `usual`: within `range`, and never
/// read by a test, which must not depend on the user's files.
fn kept_value(key: &str, usual: f32, range: std::ops::RangeInclusive<f32>) -> f32 {
    if cfg!(test) {
        return usual;
    }
    crate::paths::config_file(FILE)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| {
            crate::paths::entries(&text)
                .find(|(each, _)| *each == key)
                .and_then(|(_, value)| value.parse::<f32>().ok())
        })
        .filter(|value| value.is_finite())
        .map_or(usual, |value| value.clamp(*range.start(), *range.end()))
}

/// The pooling the settings hold, or the usual.
pub(super) fn kept() -> f32 {
    use hani_core::paint::occlusion::{POOLING, POOLING_RANGE};
    kept_value("pooling", POOLING, POOLING_RANGE)
}

/// How far the pooling reaches, as the settings hold it, or the usual.
pub(super) fn kept_reach() -> f32 {
    use hani_core::paint::occlusion::{POOLING_REACH, POOLING_REACH_RANGE};
    kept_value("reach", POOLING_REACH, POOLING_REACH_RANGE)
}

fn keep(pooling: f32, reach: f32) -> Result<(), String> {
    if cfg!(test) {
        return Ok(());
    }
    let path = crate::paths::config_file(FILE).ok_or("no settings directory")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|why| why.to_string())?;
    }
    std::fs::write(path, format!("pooling = {pooling}\nreach = {reach}\n"))
        .map_err(|why| why.to_string())
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

    /// A new reach from the slider, on every body as the pooling is.
    pub(super) fn pool_reach(&mut self, reach: f32) {
        use hani_core::paint::occlusion::{POOLING_REACH, POOLING_REACH_RANGE};
        let reach = if reach.is_nan() { POOLING_REACH } else { reach };
        self.painting.pooling_reach =
            reach.clamp(*POOLING_REACH_RANGE.start(), *POOLING_REACH_RANGE.end());
        self.keep_pooling();
        self.remesh_dirty();
    }

    /// The slider let go: keep the setting.
    pub(super) fn pooling_kept(&mut self) {
        if let Err(why) = keep(self.painting.pooling, self.painting.pooling_reach) {
            self.status = format!("the pooling was not kept: {why}");
        }
    }

    /// Give every body the pooling in hand, where one has another: run on
    /// every frame, so that a body made since is given it. A comparison a
    /// body when nothing changed.
    pub(super) fn keep_pooling(&mut self) {
        let (pooling, reach) = (self.painting.pooling, self.painting.pooling_reach);
        for (_, volume) in self.doc.bodies_mut() {
            if volume.pooling() != pooling {
                volume.set_pooling(pooling);
            }
            if volume.pooling_reach() != reach {
                volume.set_pooling_reach(reach);
            }
        }
    }

    /// How strongly and how far washes and contrasts pool: the two numbers
    /// shown in Paint's LAYERS block, in the Wash and Contrast touches'
    /// settings and in Preferences, all the same setting.
    pub(super) fn pooling_numbers(&self) -> Element<'_, Message> {
        use hani_core::paint::occlusion::{POOLING_RANGE, POOLING_REACH_RANGE};
        iced::widget::column![
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
            .tip("How dark washes and contrasts pool in recesses"),
            crate::ui::number(
                &self.drafts,
                "paint.pooling_reach",
                "Pooling reach",
                self.painting.pooling_reach,
                POOLING_REACH_RANGE,
                |reach| Message::Painting(PaintingMessage::PoolingReach(reach)),
            )
            .step(0.05)
            .on_release(Message::Painting(PaintingMessage::PoolingKept))
            .shown(format!("{:.1} mm", self.painting.pooling_reach * 4.0))
            .tip("How far round a recess washes and contrasts pool"),
        ]
        .spacing(crate::theme::S2)
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
        update(&mut app, Message::Painting(PaintingMessage::PoolingReach(2.0)));
        assert!(app.doc.bodies().all(|(_, volume)| volume.pooling_reach() == 2.0));
        update(&mut app, Message::Painting(PaintingMessage::PoolingReach(50.0)));
        assert_eq!(app.painting.pooling_reach, 4.0);
        drop(app.pooling_numbers());
    }
}
