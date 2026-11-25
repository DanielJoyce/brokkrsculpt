// SPDX-License-Identifier: AGPL-3.0-only

//! LAYERS, in the Paint mode's inspector: the active body's stack of paint
//! layers. Stage 1 of `docs/PAINT-RACK.md`; the stack itself is
//! [`brokkr_core::paint::layers`].
//!
//! Strokes go to the active layer. A layer that is hidden or locked takes
//! no paint, and the status line says so. Every change to the stack is one
//! undo entry holding the whole stack before it; a drag of a layer's
//! opacity, or its name typed, is one entry however many messages it is,
//! by [`brokkr_core::History::revision`].

use brokkr_core::paint::layers::{Blend, LayerId, LayerMeta};
use brokkr_core::{Change, Entry, NodeId};
use iced::widget::{button, column, pick_list, row, text, text_input};
use iced::{Alignment, Element, Length};

use super::Brokkr;
use crate::icon;
use crate::message::{LayersMessage, Message};
use crate::theme;

/// A gesture on one layer that is one undo entry however many messages it
/// takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gesture {
    Opacity,
    Name,
}

/// The gesture under way, if any: on which body's which layer, and the
/// history's revision when its entry was pushed. While the history has
/// not moved, the next message of the same gesture adds no entry.
#[derive(Debug, Default)]
pub(super) struct LayersHeld {
    going: Option<(NodeId, LayerId, Gesture, u64)>,
}

impl Brokkr {
    pub(super) fn layers_changed(&mut self, message: LayersMessage) {
        let body = self.doc.active();
        let before = self.doc.active_volume().layer_state();
        let metas = self.doc.active_volume().layers().metas();
        let len = metas.len();
        let meta_of = |index: usize| metas.get(index).cloned();
        let mut gesture = None;
        let status: Option<String> = {
            let volume = self.doc.active_volume_mut();
            match message {
                LayersMessage::Added => {
                    let name = format!("Layer {}", len + 1);
                    match volume.add_layer(name.clone(), Blend::Normal) {
                        Some(_) => Some(format!("added {name}")),
                        None => {
                            self.status = format!(
                                "a body holds {} layers at most",
                                brokkr_core::paint::layers::MAX_LAYERS
                            );
                            return;
                        }
                    }
                }
                LayersMessage::Chosen(index) => {
                    // Not an edit: which layer is active is where the brush
                    // goes, as which body is active is.
                    volume.set_active_layer(index);
                    return;
                }
                LayersMessage::Shown(index) => {
                    let Some(meta) = meta_of(index) else { return };
                    volume.set_layer_meta(index, LayerMeta { visible: !meta.visible, ..meta });
                    None
                }
                LayersMessage::Locked(index) => {
                    let Some(meta) = meta_of(index) else { return };
                    volume.set_layer_meta(index, LayerMeta { locked: !meta.locked, ..meta });
                    None
                }
                LayersMessage::Blended(index, blend) => {
                    let Some(meta) = meta_of(index) else { return };
                    volume.set_layer_meta(index, LayerMeta { blend, ..meta });
                    None
                }
                LayersMessage::Opacity(index, opacity) => {
                    let Some(meta) = meta_of(index) else { return };
                    gesture = Some((meta.id, Gesture::Opacity));
                    let opacity = opacity.clamp(0.0, 1.0);
                    volume.set_layer_meta(index, LayerMeta { opacity, ..meta });
                    None
                }
                LayersMessage::Renamed(index, name) => {
                    let Some(meta) = meta_of(index) else { return };
                    gesture = Some((meta.id, Gesture::Name));
                    volume.set_layer_meta(index, LayerMeta { name, ..meta });
                    None
                }
                LayersMessage::Raised(index) => {
                    if index + 1 >= len {
                        return;
                    }
                    volume.reorder_layer(index, index + 1);
                    None
                }
                LayersMessage::Lowered(index) => {
                    if index == 0 || index >= len {
                        return;
                    }
                    volume.reorder_layer(index, index - 1);
                    None
                }
                LayersMessage::Duplicated(index) => {
                    if volume.duplicate_layer(index).is_none() {
                        self.status = format!(
                            "a body holds {} layers at most",
                            brokkr_core::paint::layers::MAX_LAYERS
                        );
                        return;
                    }
                    Some("duplicated the layer".to_string())
                }
                LayersMessage::MergedDown(index) => {
                    if index == 0 || index >= len {
                        return;
                    }
                    volume.merge_layer_down(index);
                    Some("merged the layer down".to_string())
                }
                LayersMessage::Deleted(index) => {
                    if index >= len {
                        return;
                    }
                    volume.delete_layer(index);
                    Some(if len == 1 {
                        "emptied the layer".to_string()
                    } else {
                        "deleted the layer".to_string()
                    })
                }
                LayersMessage::Flattened => {
                    if len == 1 {
                        return;
                    }
                    volume.flatten_layers();
                    Some("flattened the layers".to_string())
                }
            }
        };
        self.push_layers_entry(body, before, gesture);
        self.remesh_dirty();
        if let Some(status) = status {
            self.status = status;
        }
    }

    /// Put the stack as it was before a change on the history, unless the
    /// change is the next message of a gesture already there.
    fn push_layers_entry(
        &mut self,
        body: NodeId,
        before: brokkr_core::paint::layers::LayerState,
        gesture: Option<(LayerId, Gesture)>,
    ) {
        let revision = self.history.revision();
        let going = gesture.map(|(layer, kind)| (body, layer, kind));
        if let (Some((body, layer, kind)), Some((was_body, was_layer, was_kind, at))) =
            (going, self.painting.layers.going)
            && (body, layer, kind) == (was_body, was_layer, was_kind)
            && at == revision
        {
            return;
        }
        self.history.push(Entry::new(vec![Change::Layers { body, state: Box::new(before) }]));
        self.painting.layers.going =
            going.map(|(body, layer, kind)| (body, layer, kind, self.history.revision()));
    }

    /// Whether the active layer refuses paint, being hidden or locked; and
    /// if so the status line says why. Asked before a stroke, a fill or a
    /// coat in colour.
    pub(super) fn layer_refuses_paint(&mut self) -> bool {
        let volume = self.doc.active_volume();
        let layers = volume.layers();
        let Some(meta) = layers.metas().get(layers.active()).cloned() else {
            return false;
        };
        let why = match (meta.locked, meta.visible) {
            (true, _) => "is locked",
            (_, false) => "is hidden",
            _ => return false,
        };
        self.status = format!("{} {why}", meta.name);
        true
    }

    /// LAYERS, with a heading that does not fold: which layer a stroke
    /// goes to is chosen here, as which body is in the bodies block.
    pub(super) fn layers_block(&self) -> Element<'_, Message> {
        column![
            text("LAYERS").size(theme::HEADING_SIZE).color(theme::TEXT_MUTE),
            self.layers_panel()
        ]
        .spacing(theme::S2)
        .into()
    }

    /// The LAYERS block: the stack top first, each with its eye and its
    /// lock, the active one's blend and opacity, and what can be done to
    /// the stack.
    pub(super) fn layers_panel(&self) -> Element<'_, Message> {
        let volume = self.doc.active_volume();
        let layers = volume.layers();
        let metas = layers.metas();
        let active = layers.active();
        let len = metas.len();
        let send = |message: LayersMessage| Message::Layers(message);

        let mut list = column![].spacing(theme::S1);
        for (index, meta) in metas.iter().enumerate().rev() {
            let on = index == active;
            let eye = if meta.visible { icon::IconName::Eye } else { icon::IconName::EyeOff };
            let ink = if meta.visible { theme::TEXT } else { theme::TEXT_MUTE };
            let name: Element<'_, Message> = if on {
                text_input("Layer", &meta.name)
                    .on_input(move |typed| send(LayersMessage::Renamed(index, typed)))
                    .size(theme::LABEL_SIZE)
                    .padding(theme::PAD_FIELD)
                    .width(Length::Fill)
                    .into()
            } else {
                button(text(meta.name.clone()).size(theme::LABEL_SIZE).color(ink))
                    .width(Length::Fill)
                    .padding(theme::PAD_CHOICE)
                    .style(theme::tool_button)
                    .on_press(send(LayersMessage::Chosen(index)))
                    .into()
            };
            let blend = text(meta.blend.label()).size(theme::LABEL_SIZE).color(theme::TEXT_MUTE);
            let line = row![
                crate::ui::tip(
                    button(icon::icon(eye, theme::ICON_INLINE, ink))
                        .padding(theme::PAD_CHOICE)
                        .style(theme::section_heading)
                        .on_press(send(LayersMessage::Shown(index))),
                    "Show or hide",
                ),
                name,
                blend,
                crate::ui::toggle("Lock", meta.locked, send(LayersMessage::Locked(index)))
                    .width(Length::Shrink),
            ]
            .spacing(theme::S2)
            .align_y(Alignment::Center);
            list = list.push(iced::widget::container(line).padding(2).style(if on {
                theme::body_row_active
            } else {
                theme::body_row
            }));
        }

        let meta = &metas[active];
        let blend = row![
            text("Blend").size(theme::LABEL_SIZE).color(theme::TEXT_DIM),
            pick_list(Blend::ALL, Some(meta.blend), move |blend| {
                send(LayersMessage::Blended(active, blend))
            })
            .text_size(theme::LABEL_SIZE)
            .width(Length::Fill),
        ]
        .spacing(theme::S2)
        .align_y(Alignment::Center);
        let opacity = crate::ui::number(
            &self.drafts,
            "layer.opacity",
            "Layer opacity",
            meta.opacity,
            0.0..=1.0,
            move |opacity| send(LayersMessage::Opacity(active, opacity)),
        )
        .shown(format!("{:.2}", meta.opacity));

        // Each a share of the row by what it shows, so that the row fills
        // the dock and no label is squeezed.
        let icon_button = |name: icon::IconName, tip: &'static str, message: Option<Message>| {
            let ink = if message.is_some() { theme::TEXT } else { theme::TEXT_MUTE };
            crate::ui::tip(
                button(
                    iced::widget::container(icon::icon(name, theme::ICON_INLINE, ink))
                        .center_x(Length::Fill),
                )
                .width(Length::FillPortion(2))
                .padding(theme::PAD_CHOICE)
                .style(theme::tool_button)
                .on_press_maybe(message),
                tip,
            )
        };
        let word = |label: &'static str, message: Option<Message>| {
            let ink = if message.is_some() { theme::TEXT } else { theme::TEXT_MUTE };
            button(
                text(label)
                    .size(theme::LABEL_SIZE)
                    .color(ink)
                    .align_x(Alignment::Center)
                    .width(Length::Fill),
            )
            .width(Length::FillPortion(label.len() as u16))
            .padding(theme::PAD_CHOICE)
            .style(theme::tool_button)
            .on_press_maybe(message)
        };
        let full = len >= brokkr_core::paint::layers::MAX_LAYERS;
        let actions = row![
            icon_button(
                icon::IconName::Plus,
                "New layer",
                (!full).then(|| send(LayersMessage::Added))
            ),
            icon_button(
                icon::IconName::Copy,
                "Duplicate",
                (!full).then(|| send(LayersMessage::Duplicated(active)))
            ),
            word("Up", (active + 1 < len).then(|| send(LayersMessage::Raised(active)))),
            word("Down", (active > 0).then(|| send(LayersMessage::Lowered(active)))),
            icon_button(
                icon::IconName::MergeDown,
                "Merge down",
                (active > 0).then(|| send(LayersMessage::MergedDown(active))),
            ),
            word("Flatten", (len > 1).then(|| send(LayersMessage::Flattened))),
            icon_button(
                icon::IconName::Trash,
                "Delete layer",
                Some(send(LayersMessage::Deleted(active)))
            ),
        ]
        .spacing(theme::S1)
        .align_y(Alignment::Center);

        column![list, actions, blend, opacity].spacing(theme::S2).into()
    }
}

/// The layers done the way a user does them: messages in, the body's
/// stack and the history looked at afterwards.
#[cfg(test)]
mod tests {
    use super::*;
    use brokkr_core::paint::field::BARE;
    use brokkr_core::{BrushKind, PROTECTED};
    use glam::IVec3;

    fn update(app: &mut Brokkr, message: Message) {
        drop(app.update(message));
    }

    fn layers(app: &mut Brokkr, message: LayersMessage) {
        update(app, Message::Layers(message));
    }

    fn app() -> Brokkr {
        let mut app = Brokkr::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::BrushKindChanged(BrushKind::Paint));
        app
    }

    fn names(app: &Brokkr) -> Vec<String> {
        app.doc.active_volume().layers().metas().into_iter().map(|meta| meta.name).collect()
    }

    /// Some voxel of the surface, and the paint of each layer there.
    fn painted_cell(app: &Brokkr) -> IVec3 {
        let volume = app.doc.active_volume();
        for coord in volume.brick_coords() {
            let origin = coord.origin();
            for index in 0..32 * 32 * 32 {
                let cell = origin + IVec3::new(index % 32, (index / 32) % 32, index / 1024);
                if volume.layer_fields().iter().any(|field| field.at(cell) != BARE) {
                    return cell;
                }
            }
        }
        panic!("nothing is painted");
    }

    fn at(app: &Brokkr, cell: IVec3) -> Vec<[u8; 4]> {
        app.doc.active_volume().layer_fields().iter().map(|field| field.at(cell)).collect()
    }

    /// A coat goes on the active layer; a new layer over it takes the next
    /// coat and leaves the first; undo takes them off in turn.
    #[test]
    fn a_coat_goes_on_the_active_layer_and_each_undoes() {
        let mut app = app();
        update(&mut app, Message::Painting(crate::message::PaintingMessage::Colour([200, 0, 0])));
        update(&mut app, Message::Painting(crate::message::PaintingMessage::Coated));
        let cell = painted_cell(&app);
        let red = at(&app, cell)[0];

        layers(&mut app, LayersMessage::Added);
        assert_eq!(names(&app), ["Paint", "Layer 2"]);
        update(&mut app, Message::Painting(crate::message::PaintingMessage::Colour([0, 0, 200])));
        update(&mut app, Message::Painting(crate::message::PaintingMessage::Coated));
        let both = at(&app, cell);
        assert_eq!(both[0], red, "the coat on the new layer reached the one under it");
        assert_ne!(both[1], BARE);

        update(&mut app, Message::Undo);
        assert_eq!(at(&app, cell), vec![red, BARE]);
        update(&mut app, Message::Undo);
        assert_eq!(names(&app), ["Paint"], "undo did not take the layer off");
        update(&mut app, Message::Redo);
        update(&mut app, Message::Redo);
        assert_eq!(at(&app, cell), both);
    }

    /// A locked or hidden layer takes no paint, and the status says why.
    #[test]
    fn a_locked_or_hidden_layer_takes_no_paint() {
        for message in [LayersMessage::Locked(0), LayersMessage::Shown(0)] {
            let mut app = app();
            layers(&mut app, message);
            update(&mut app, Message::Painting(crate::message::PaintingMessage::Coated));
            assert!(app.doc.active_volume().paint().is_empty(), "a refused layer took paint");
            assert!(app.status.starts_with("Paint is"), "the status said {}", app.status);
        }
    }

    /// A drag of the opacity, however many steps, is one undo entry, and so
    /// is a name typed; a second drag after anything else is another.
    #[test]
    fn a_drag_of_the_opacity_is_one_undo() {
        let mut app = app();
        layers(&mut app, LayersMessage::Added);
        let revision = app.history.revision();
        for step in 1..=10 {
            layers(&mut app, LayersMessage::Opacity(1, 1.0 - step as f32 * 0.05));
        }
        for typed in ["S", "Sh", "Sha", "Shade"] {
            layers(&mut app, LayersMessage::Renamed(1, typed.to_string()));
        }
        assert_eq!(app.history.revision(), revision + 2, "the drag and the name were not one each");
        let meta = &app.doc.active_volume().layers().metas()[1];
        assert_eq!(meta.name, "Shade");
        assert!((meta.opacity - 0.5).abs() < 1e-6);
        update(&mut app, Message::Undo);
        assert_eq!(names(&app), ["Paint", "Layer 2"]);
        update(&mut app, Message::Undo);
        assert!((app.doc.active_volume().layers().metas()[1].opacity - 1.0).abs() < 1e-6);
    }

    /// The stack's order, a merge and a flatten, by their buttons.
    #[test]
    fn the_stack_moves_merges_and_flattens() {
        let mut app = app();
        layers(&mut app, LayersMessage::Added);
        layers(&mut app, LayersMessage::Added);
        assert_eq!(names(&app), ["Paint", "Layer 2", "Layer 3"]);
        layers(&mut app, LayersMessage::Lowered(2));
        assert_eq!(names(&app), ["Paint", "Layer 3", "Layer 2"]);
        assert_eq!(app.doc.active_volume().layers().active(), 1, "the moved layer is not active");
        layers(&mut app, LayersMessage::MergedDown(1));
        assert_eq!(names(&app), ["Paint", "Layer 2"]);
        layers(&mut app, LayersMessage::Flattened);
        assert_eq!(names(&app), ["Paint"]);
        assert!(app.doc.active_volume().layers().is_plain());
        // The flatten, the merge and the move.
        for _ in 0..3 {
            update(&mut app, Message::Undo);
        }
        assert_eq!(names(&app), ["Paint", "Layer 2", "Layer 3"]);
    }

    /// A mask still holds paint back on a layer.
    #[test]
    fn a_mask_holds_paint_off_a_layer() {
        let mut app = app();
        layers(&mut app, LayersMessage::Added);
        let volume = app.doc.active_volume_mut();
        let (low, high) = volume.world_bounds().expect("a body");
        let (lo, hi) = volume.voxel_bounds(low, high);
        volume.edit_mask(lo, hi, |_, _, _| PROTECTED);
        update(&mut app, Message::Painting(crate::message::PaintingMessage::Coated));
        assert!(app.doc.active_volume().paint().is_empty(), "paint went under the mask");
    }

    /// The block draws with one layer and with many.
    #[test]
    fn the_block_draws() {
        let mut app = app();
        drop(app.layers_block());
        layers(&mut app, LayersMessage::Added);
        layers(&mut app, LayersMessage::Duplicated(1));
        assert_eq!(names(&app), ["Paint", "Layer 2", "Layer 2 copy"]);
        assert_eq!(app.doc.active_volume().layers().active(), 2);
        drop(app.layers_block());
    }
}
