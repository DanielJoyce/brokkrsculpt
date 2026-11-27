// SPDX-License-Identifier: AGPL-3.0-only

//! Boolean groups in the bodies list and on the model.
//!
//! Asked for by the maintainer on 2026-11-28: groups that combine their
//! children by boolean operations, in an order that matters and is shown,
//! transformed as one and baked into one body. `docs/BOOLEANS.md` is the
//! design and `hani_core::combine` the fields.
//!
//! In the list:
//! - a folder row has a chip that makes it combine its children, or stop;
//!   while it combines the chip's tooltip spells the fold out with every
//!   bracket, as `((Body 1 − Hole) + Boss)`;
//! - each row inside a combining folder has a chip with its operation, the
//!   first one "Base", pressed in turn through add, cut and keep;
//! - in Transform, beside merge, three buttons make the body in hand and
//!   the one below it a combining folder, the body in hand added, cut or
//!   kept, and Bake collapses the folder the body in hand is in.
//!
//! On the model the folder's result is drawn in place of its operands, and
//! while the body in hand is inside it the rows that cut or keep are drawn
//! see-through, so that what does the cutting can be seen and sculpted.

use hani_core::NodeId;
use hani_core::combine::{BakeRefusal, BoolOp, BoolState, MOST_BLEND_HALVES};
use iced::Element;
use iced::widget::{button, row, text};

use super::Hani;
use crate::message::Message;
use crate::theme;

/// What the boolean controls ask for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BooleanMessage {
    /// The body in hand's soft join, in voxels, as its slider moves.
    Soft(f32),
    /// The slider let go: one undo entry for the whole drag.
    SoftDone,
    /// A folder combines its children, or stops.
    Combine(NodeId),
    /// A row's operation, to the next one.
    Op(NodeId),
    /// The body in hand and the one below, made a combining folder, the
    /// body in hand joining it by this operation.
    WithBelow(BoolOp),
    /// The combining folder the body in hand is in, collapsed into one body.
    Bake,
}

/// What the soft join's slider was before its drag, for the one entry the
/// drag makes.
#[derive(Debug, Default)]
pub(super) struct SoftDrag {
    before: Option<(NodeId, BoolState)>,
}

/// How see-through a row that cuts or keeps is drawn: as the Others button
/// draws the rest. See `app/explode.rs`.
const GHOST: f32 = 0.7;

impl Hani {
    pub(super) fn boolean_changed(&mut self, message: BooleanMessage) {
        let changes = match message {
            BooleanMessage::Combine(folder) => {
                let state = self.doc.bool_state(folder);
                let on = !state.combines;
                let change = self.doc.set_bool_state(folder, BoolState { combines: on, ..state });
                self.status = match (&change, on) {
                    (None, _) => "only a folder can combine its rows".to_string(),
                    (Some(_), true) => "the folder combines its rows, top to bottom".to_string(),
                    (Some(_), false) => "the folder is a plain folder again".to_string(),
                };
                change.into_iter().collect()
            }
            BooleanMessage::Op(id) => {
                let state = self.doc.bool_state(id);
                let op = state.op.next();
                let change = self.doc.set_bool_state(id, BoolState { op, ..state });
                if change.is_some() {
                    self.status = format!("{}: {}", self.name_of(id), op.label().to_lowercase());
                }
                change.into_iter().collect()
            }
            BooleanMessage::WithBelow(op) => self.combine_with_below(op),
            BooleanMessage::Soft(voxels) => {
                let id = self.doc.active();
                let state = self.doc.bool_state(id);
                let blend = (voxels * 2.0).round().clamp(0.0, f32::from(MOST_BLEND_HALVES)) as u8;
                if let Some(hani_core::Change::Boolean { state: before, .. }) =
                    self.doc.set_bool_state(id, BoolState { blend, ..state })
                    && self.soft_drag.before.is_none()
                {
                    self.soft_drag.before = Some((id, before));
                }
                self.unsaved = true;
                self.remesh_dirty();
                Vec::new()
            }
            BooleanMessage::SoftDone => match self.soft_drag.before.take() {
                Some((id, before)) if self.doc.bool_state(id) != before => {
                    vec![hani_core::Change::Boolean { id, state: before }]
                }
                _ => Vec::new(),
            },
            BooleanMessage::Bake => self.bake_boolean(),
        };
        if changes.is_empty() {
            return;
        }
        let before = self.history.stats();
        self.history.push(hani_core::Entry::new(changes));
        self.record_history(before);
        self.unsaved = true;
        self.remesh_dirty();
    }

    fn name_of(&self, id: NodeId) -> String {
        self.doc.node(id).map_or_else(String::new, |node| node.name.clone())
    }

    /// The body in hand and the body below it, in a new folder that
    /// combines: the one below the base, the one in hand joining it by `op`.
    fn combine_with_below(&mut self, op: BoolOp) -> Vec<hani_core::Change> {
        let source = self.doc.active();
        let hani_core::MergeTarget::Body(below) = self.doc.merge_target(source) else {
            self.status = "nothing to combine with: the row below is not a body".to_string();
            return Vec::new();
        };
        if self.doc.node_count() >= hani_core::MAX_NODES {
            self.status =
                "could not combine: the document holds as many rows as it can".to_string();
            return Vec::new();
        }
        let name = self.next_group_name();
        let Some((folder, mut changes)) = self.doc.group(below, name.clone()) else {
            self.status = format!(
                "could not combine: {} folders deep is as far as the panel goes",
                hani_core::MAX_DEPTH
            );
            return Vec::new();
        };
        if let Some(moved) = self.doc.move_to_folder(source, Some(folder)) {
            changes.extend(moved);
        }
        changes.extend(
            self.doc
                .set_bool_state(folder, BoolState { combines: true, op: BoolOp::Add, blend: 0 }),
        );
        changes
            .extend(self.doc.set_bool_state(source, BoolState { combines: false, op, blend: 0 }));
        self.status =
            format!("{name}: {} {} {}", self.name_of(below), op.symbol(), self.name_of(source));
        changes
    }

    /// The combining folder nearest above the body in hand.
    fn folder_in_hand(&self) -> Option<NodeId> {
        self.doc.combining_ancestors(self.doc.active()).first().copied()
    }

    fn bake_boolean(&mut self) -> Vec<hani_core::Change> {
        let Some(folder) = self.folder_in_hand() else {
            self.status = "nothing to bake: the body in hand is in no combining folder".to_string();
            return Vec::new();
        };
        self.bake_folder(folder)
    }

    /// Bake a combining folder, telling the renderer of the rows it took
    /// away; the changes, or none with the status saying why.
    pub(super) fn bake_folder(&mut self, folder: NodeId) -> Vec<hani_core::Change> {
        let name = self.name_of(folder);
        // Every row under the folder but the one that becomes the result
        // leaves the document, and the renderer has to be told, as a merge
        // tells it.
        let under: Vec<NodeId> = self
            .doc
            .subtree_of(folder)
            .map(|range| self.doc.nodes()[range].iter().map(|node| node.id).collect())
            .unwrap_or_default();
        match self.doc.bake_boolean(folder) {
            Ok(changes) => {
                for id in under {
                    if self.doc.node(id).is_none() {
                        self.shared.forget_body(id);
                    }
                }
                self.status = format!("baked {name} into one body");
                changes
            }
            Err(BakeRefusal::Hidden) => {
                self.status = format!("not baked: a row in {name} is hidden; show it or delete it");
                Vec::new()
            }
            Err(BakeRefusal::Empty | BakeRefusal::NotBoolean) => {
                self.status = format!("not baked: {name} holds nothing to combine");
                Vec::new()
            }
        }
    }

    /// The chip a row of the list carries: on a folder, whether it
    /// combines; on a row inside a combining folder, its operation.
    pub(super) fn boolean_chip(&self, node: &hani_core::Node) -> Option<Element<'_, Message>> {
        let chip = |label: String, lit: bool, message: BooleanMessage, tip: String| {
            crate::ui::tip(
                button(text(label).size(theme::LABEL_SIZE))
                    .padding(iced::Padding { top: 0.0, bottom: 0.0, left: 4.0, right: 4.0 })
                    .style(if lit { theme::tool_button_active } else { theme::section_heading })
                    .on_press(Message::Boolean(message)),
                tip,
            )
        };
        if !node.is_body() {
            let combines = self.doc.combines(node.id);
            let tip = match self.doc.boolean_formula(node.id) {
                Some(formula) => format!("Combines, top to bottom: {formula}"),
                None => "Combine the rows in this folder, top to bottom".to_string(),
            };
            return Some(chip(
                "\u{222a}".to_string(),
                combines,
                BooleanMessage::Combine(node.id),
                tip,
            ));
        }
        let parent = self.doc.combining_parent(node.id)?;
        let first = self.doc.operands(parent).first().map(|(id, _)| *id) == Some(node.id);
        if first {
            return Some(crate::ui::tip(
                text("Base").size(theme::LABEL_SIZE).color(theme::TEXT_MUTE),
                "The first row: what the rows under it add to, cut or keep",
            ));
        }
        let op = self.doc.bool_state(node.id).op;
        let tip = match op {
            BoolOp::Add => "Added to the rows above; press for cut",
            BoolOp::Cut => "Cut from the rows above; press for keep",
            BoolOp::Keep => "Keeps only what the rows above share with it; press for add",
        };
        Some(chip(
            op.symbol().to_string(),
            op != BoolOp::Add,
            BooleanMessage::Op(node.id),
            tip.to_string(),
        ))
    }

    /// The boolean verbs, under the list in Transform.
    pub(super) fn boolean_verbs(&self) -> Element<'_, Message> {
        let verb = |label: &'static str, tip: &'static str, message: BooleanMessage| {
            crate::ui::tip(
                button(text(label).size(theme::LABEL_SIZE))
                    .padding(theme::PAD_BUTTON)
                    .style(theme::tool_button)
                    .on_press(Message::Boolean(message)),
                tip,
            )
        };
        let mut verbs = row![
            verb(
                "+",
                "Add this body to the body below, in a folder that combines them",
                BooleanMessage::WithBelow(BoolOp::Add)
            ),
            verb(
                "\u{2212}",
                "Cut this body from the body below, in a folder that combines them",
                BooleanMessage::WithBelow(BoolOp::Cut)
            ),
            verb(
                "\u{2229}",
                "Keep where this body and the body below meet, in a folder that combines them",
                BooleanMessage::WithBelow(BoolOp::Keep)
            ),
        ]
        .spacing(theme::S2);
        if self.folder_in_hand().is_some() {
            verbs = verbs.push(verb(
                "Bake",
                "Collapse the combining folder this body is in into one body",
                BooleanMessage::Bake,
            ));
        }
        // The body in hand's soft join, where it joins a combining folder
        // and is not its base.
        let active = self.doc.active();
        let joins = self.doc.combining_parent(active).is_some_and(|folder| {
            self.doc.operands(folder).first().map(|(id, _)| *id) != Some(active)
        });
        if !joins {
            return verbs.into();
        }
        let soft = self.doc.bool_state(active).blend_voxels();
        iced::widget::column![
            verbs,
            crate::ui::number(
                &self.drafts,
                "boolean.soft",
                "Soft join",
                soft,
                0.0..=f32::from(MOST_BLEND_HALVES) * 0.5,
                |voxels| Message::Boolean(BooleanMessage::Soft(voxels)),
            )
            .step(0.5)
            .shown(format!("{soft:.1} vx"))
            .on_release(Message::Boolean(BooleanMessage::SoftDone))
            .tip("How far the join to the rows above is rounded, in voxels"),
        ]
        .spacing(theme::S2)
        .into()
    }

    /// The rows the renderer must not draw because a combining folder's
    /// result stands for them, and the results of folders that are not
    /// shown. Added to the hidden list by `publish_visibility`.
    pub(super) fn hide_boolean_rows(&mut self) {
        let ghosts = self.boolean_ghosts();
        for (node, shown) in self.doc.nodes().iter().zip(&self.shown) {
            if !node.is_body() {
                if self.doc.combines(node.id) && !shown {
                    self.hidden_bodies.push(node.id);
                }
                continue;
            }
            if *shown
                && self.doc.inside_boolean(node.id)
                && !ghosts.iter().any(|(ghost, _)| *ghost == node.id)
            {
                self.hidden_bodies.push(node.id);
            }
        }
    }

    /// The rows drawn see-through: those that cut or keep in the combining
    /// folder the body in hand is in, and the body in hand itself when it
    /// is one of them.
    fn boolean_ghosts(&self) -> Vec<(NodeId, BoolOp)> {
        let Some(folder) = self.folder_in_hand() else {
            return Vec::new();
        };
        self.doc
            .operands(folder)
            .into_iter()
            .skip(1)
            .filter(|(id, op)| {
                *op != BoolOp::Add && self.doc.node(*id).is_some_and(|node| node.is_body())
            })
            .collect()
    }

    /// Add the ghosts to what the spread draws. Called by `publish_spread`.
    pub(super) fn add_boolean_ghosts(&mut self) {
        for (id, op) in self.boolean_ghosts() {
            // Tinted by what the row does. See `hani_gpu::tint`.
            let tint = if op == BoolOp::Cut { hani_gpu::tint::CUT } else { hani_gpu::tint::KEEP };
            let w = hani_gpu::tint::screened_and_tinted(GHOST, tint);
            if let Some(drawn) = self.explode.drawn.iter_mut().find(|(drawn, _)| *drawn == id) {
                drawn.1.w = w;
            } else {
                self.explode.drawn.push((id, glam::Vec4::new(0.0, 0.0, 0.0, w)));
            }
        }
    }

    /// Build the results again when the arrangement changed without any
    /// brick changing: a chip pressed, a row moved, an eye closed, an undo.
    /// Called on every message, from `publish_visibility`; nothing to do
    /// costs one hash of the rows.
    pub(super) fn refresh_booleans(&mut self) {
        if self.doc.booleans_stale() {
            self.remesh_dirty();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    /// The first body and a cube added below it, the first in hand.
    fn two() -> (Hani, NodeId, NodeId) {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::PrimitiveAdded(hani_core::PrimitiveKind::Cube));
        let cube = app.doc.active();
        let first = app.doc.nodes()[0].id;
        update(&mut app, Message::BodySelected(first));
        (app, first, cube)
    }

    /// Cut from the body below: a folder that combines, the one below the
    /// base and the one in hand cutting; each chip press turns the
    /// operation on; one undo puts the rows back as they were.
    #[test]
    fn a_body_is_cut_from_the_one_below_and_the_cut_is_undone() {
        let (mut app, first, cube) = two();
        update(&mut app, Message::Boolean(BooleanMessage::WithBelow(BoolOp::Cut)));
        let folder = app.doc.nodes()[0].id;
        assert!(app.doc.combines(folder));
        let order: Vec<NodeId> = app.doc.children_of(folder);
        assert_eq!(order, [cube, first], "the one below is the base");
        assert_eq!(app.doc.bool_state(first).op, BoolOp::Cut);
        update(&mut app, Message::Boolean(BooleanMessage::Op(first)));
        assert_eq!(app.doc.bool_state(first).op, BoolOp::Keep);
        update(&mut app, Message::Undo);
        assert_eq!(app.doc.bool_state(first).op, BoolOp::Cut);
        update(&mut app, Message::Undo);
        let rows: Vec<NodeId> = app.doc.nodes().iter().map(|node| node.id).collect();
        assert_eq!(rows, [first, cube], "the folder is gone and the order is back");
        assert!(!app.doc.combines(folder));
    }

    /// The base is drawn through the folder's result and not on its own;
    /// the row that cuts, in hand, is drawn see-through.
    #[test]
    fn operands_are_drawn_through_the_result_and_a_cutter_as_a_ghost() {
        let (mut app, first, cube) = two();
        update(&mut app, Message::Boolean(BooleanMessage::WithBelow(BoolOp::Cut)));
        let folder = app.doc.nodes()[0].id;
        assert!(app.doc.boolean_result(folder).is_some_and(|result| result.brick_count() > 0));
        assert!(app.hidden_bodies.contains(&cube), "the base is drawn as well as the result");
        assert!(!app.hidden_bodies.contains(&first), "the cutter in hand is not drawn");
        assert!(
            app.explode.drawn.iter().any(|(id, at)| *id == first && at.w > 0.0),
            "the cutter is not see-through"
        );
        let drawn = app.explode.drawn.iter().find(|(id, _)| *id == first).expect("drawn");
        let (share, tint) = hani_gpu::tint::untinted(drawn.1.w);
        assert_eq!(tint, hani_gpu::tint::CUT, "the cutter is not tinted as one");
        assert!((share - GHOST).abs() < 1.0e-6, "the tint changed how much is screened");
        // With the cube in hand the cutter is still drawn see-through.
        update(&mut app, Message::BodySelected(cube));
        assert!(app.explode.drawn.iter().any(|(id, _)| *id == first));
        // Hidden, the folder's result is not drawn. (The eye refuses to
        // hide the last bodies shown, so it is closed here directly.)
        let mut meta = app.doc.meta(folder).expect("the folder");
        meta.visible = false;
        app.doc.set_meta(&meta);
        app.publish_visibility();
        assert!(app.hidden_bodies.contains(&folder));
    }

    /// The soft join's slider sets the body in hand's join, and a drag of it
    /// is one undo entry.
    #[test]
    fn the_soft_join_is_set_by_its_slider_in_one_entry() {
        let (mut app, first, _) = two();
        update(&mut app, Message::Boolean(BooleanMessage::WithBelow(BoolOp::Cut)));
        let entries = app.history_stats.undo_entries;
        update(&mut app, Message::Boolean(BooleanMessage::Soft(1.0)));
        update(&mut app, Message::Boolean(BooleanMessage::Soft(2.5)));
        update(&mut app, Message::Boolean(BooleanMessage::SoftDone));
        assert_eq!(app.doc.bool_state(first).blend, 5);
        assert_eq!(app.history_stats.undo_entries, entries + 1, "not one entry for the drag");
        update(&mut app, Message::Undo);
        assert_eq!(app.doc.bool_state(first).blend, 0);
        assert_eq!(app.doc.bool_state(first).op, BoolOp::Cut, "the undo took the cut as well");
    }

    /// Baked: one body under the folder's name; one undo brings the folder
    /// and both rows back.
    #[test]
    fn a_bake_leaves_one_body_and_is_undone() {
        let (mut app, first, cube) = two();
        update(&mut app, Message::Boolean(BooleanMessage::WithBelow(BoolOp::Cut)));
        let folder = app.doc.nodes()[0].id;
        let name = app.doc.node(folder).expect("folder").name.clone();
        update(&mut app, Message::Boolean(BooleanMessage::Bake));
        assert_eq!(app.doc.node_count(), 1);
        assert_eq!(app.doc.nodes()[0].name, name);
        assert_eq!(app.doc.nodes()[0].id, cube, "the base holds the result");
        update(&mut app, Message::Undo);
        let rows: Vec<NodeId> = app.doc.nodes().iter().map(|node| node.id).collect();
        assert_eq!(rows, [folder, cube, first]);
        assert!(app.doc.combines(folder));
    }
}
