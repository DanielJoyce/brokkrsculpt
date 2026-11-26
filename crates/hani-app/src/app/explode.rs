// SPDX-License-Identifier: AGPL-3.0-only
//
// The hierarchy's two view buttons: Explode, which pushes every drawn body
// apart on screen so that a small part buried among larger ones can be seen
// and picked, and Solo active, which shows the active body alone.
//
// Both are VIEWS. Explode moves where a body is drawn and nothing else: the
// document's fields stay where they are, the offsets live in the renderer
// (`hani_gpu::spread`), and the pick marches each body with the ray moved
// back by that body's offset, so what is pressed is what is drawn. A press on
// a body while apart selects it and brings everything back together, which
// is the whole of what the mode is for; a press on nothing does nothing, and
// Escape or the button again bring them back as well. Solo active is the
// existing solo mode, entered on the active body from the header rather than
// from the row.

use glam::{Vec2, Vec3};

use iced::Element;
use iced::widget::{button, row};

use super::Hani;
use crate::message::{Message, PointerButton};
use crate::{icon, theme};
use hani_core::NodeId;

/// The gap left between a part and the anchor, as a fraction of the
/// half-diagonal of everything drawn. Chosen on a screen, not measured.
const GAP: f32 = 0.15;

/// The golden angle, in radians, which sends each part centred on the anchor
/// a different way round the view.
const GOLDEN: f32 = 2.399_963;

/// Explode's state.
#[derive(Debug, Default)]
pub(super) struct Explode {
    pub(super) on: bool,
    /// Every drawn body and how far it is drawn from where it is, the anchor
    /// at zero. Refilled by `publish_spread` after every message; kept so
    /// that the refill allocates nothing.
    pub(super) spread: Vec<(NodeId, Vec3)>,
    /// The view's right and up when Explode was pressed, which parts centred
    /// on the anchor are sent across. Held rather than read each time so
    /// that turning the view does not move them.
    pub(super) across: (Vec3, Vec3),
    /// Each body's box, measured from its bricks when Explode was pressed.
    /// Not the document's cached box, which is a superset that only grows:
    /// a piece just split off a ball kept the whole ball's box, was taken
    /// for the largest part, and the ball was pushed away from it (seen on
    /// the screen).
    pub(super) boxes: Vec<(NodeId, (Vec3, Vec3))>,
    /// The view's target and distance before Explode framed the parts
    /// apart, put back when they come together.
    pub(super) view_before: Option<(Vec3, f32)>,
}

impl Hani {
    /// Whether the bodies are drawn apart.
    pub(super) fn exploded(&self) -> bool {
        self.explode.on
    }

    /// The Explode button: apart, or back together.
    pub(super) fn toggle_explode(&mut self) {
        if self.explode.on {
            self.unexplode();
            return;
        }
        self.explode.boxes.clear();
        for (node, shown) in self.doc.nodes().iter().zip(&self.shown) {
            if let Some(found) = node.volume().filter(|_| *shown).and_then(|v| v.world_bounds()) {
                self.explode.boxes.push((node.id, found));
            }
        }
        let drawn = self.explode.boxes.len();
        if drawn < 2 {
            self.status = "only one body is drawn".to_string();
            return;
        }
        // The gizmo is drawn where the body IS, which while apart is not
        // where it is drawn. A move in hand is kept, as choosing another
        // tool keeps it, and the gizmo comes back on whatever is picked.
        self.disarm_gizmo(true);
        self.explode.on = true;
        self.explode.across = (self.camera.right(), self.camera.up());
        self.hover = None;
        self.hover_body = None;
        self.status = "pick a part — escape brings them back".to_string();
        self.publish_spread();
        self.frame_apart();
    }

    /// Frame the parts as they are drawn apart, keeping the way the view
    /// faces and only ever moving out: apart, a model the view held whole
    /// ran past its edges (seen on the screen). What the view was is kept
    /// to put back.
    fn frame_apart(&mut self) {
        let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for &(body, offset) in &self.explode.spread {
            if let Some((min, max)) = self.box_of(body) {
                low = low.min(min + offset);
                high = high.max(max + offset);
            }
        }
        if low.x > high.x {
            return;
        }
        self.explode.view_before = Some((self.camera.target, self.camera.distance));
        let framed = super::OrbitCamera::framing((low + high) * 0.5, ((high - low) * 0.5).length());
        self.camera.target = framed.target;
        self.camera.distance = framed
            .distance
            .max(self.camera.distance)
            .clamp(self.camera.min_distance(), self.camera.max_distance());
        self.refresh_camera_lattice();
        self.publish_camera();
    }

    pub(super) fn unexplode(&mut self) {
        if !self.explode.on {
            return;
        }
        self.explode.on = false;
        self.publish_spread();
        if let Some((target, distance)) = self.explode.view_before.take() {
            self.camera.target = target;
            self.camera.distance = distance;
            self.refresh_camera_lattice();
            self.publish_camera();
        }
        if self.tool == super::Tool::Transform
            && let Err(why) = self.arm_gizmo()
        {
            self.status = why;
        }
        self.refresh_gizmo();
    }

    /// Work out every drawn body's offset and hand them to the renderer.
    /// Called from `publish_visibility`, after every message, so a body
    /// hidden, added, deleted or soloed while apart is laid out again.
    ///
    /// The anchor is the largest drawn body, by the volume of its box, and
    /// stays where it is. Every other body is moved along the line from the
    /// anchor's middle through its own, to twice its distance or to where
    /// the two boxes' half-widths and a gap clear each other, whichever is
    /// further: a chain of parts (an arm, a hand) opens out along itself,
    /// and a part inside another comes out of it. A part centred on the
    /// anchor has no line of its own and is sent across the view as it was
    /// when Explode was pressed, each such part a different way round, so
    /// none is pushed toward the eye and hidden behind the anchor.
    pub(super) fn publish_spread(&mut self) {
        self.explode.spread.clear();
        if self.explode.on {
            let mut anchor: Option<(f32, NodeId, Vec3, f32)> = None;
            let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for (node, shown) in self.doc.nodes().iter().zip(&self.shown) {
                let Some((min, max)) = self.box_of(node.id).filter(|_| *shown) else {
                    continue;
                };
                low = low.min(min);
                high = high.max(max);
                let size = (max - min).max(Vec3::ZERO);
                let volume = size.x * size.y * size.z;
                if anchor.is_none_or(|(most, ..)| volume > most) {
                    anchor = Some((volume, node.id, (min + max) * 0.5, size.max_element() * 0.5));
                }
            }
            if let Some((_, still, middle, anchor_half)) = anchor {
                let gap = (high - low).length() * 0.5 * GAP;
                let (right, up) = self.explode.across;
                let mut centred = 0;
                for (node, shown) in self.doc.nodes().iter().zip(&self.shown) {
                    let Some((min, max)) = self.box_of(node.id).filter(|_| *shown) else {
                        continue;
                    };
                    let away = (min + max) * 0.5 - middle;
                    let half = (max - min).max_element() * 0.5;
                    let offset = if node.id == still {
                        Vec3::ZERO
                    } else {
                        let way = if away.length() > anchor_half * 0.05 {
                            away.normalize()
                        } else {
                            centred += 1;
                            let turn = GOLDEN * centred as f32;
                            right * turn.cos() + up * turn.sin()
                        };
                        let to = (away.length() * 2.0).max(anchor_half + half + gap);
                        way * to - away
                    };
                    self.explode.spread.push((node.id, offset));
                }
            }
        }
        self.shared.set_spread(&self.explode.spread);
    }

    /// A body's box as measured when Explode was pressed.
    fn box_of(&self, body: NodeId) -> Option<(Vec3, Vec3)> {
        self.explode.boxes.iter().find(|(id, _)| *id == body).map(|(_, found)| *found)
    }

    /// The nearest drawn surface under a pixel, each body marched where it
    /// is DRAWN: the ray is moved back by the body's offset, so the hit is a
    /// point on the body itself.
    pub(super) fn pick_apart(&self, pixel: Vec2) -> Option<(NodeId, hani_core::Hit)> {
        let (origin, ray) = self.ray_through(pixel);
        let mut best: Option<(NodeId, hani_core::Hit)> = None;
        for &(body, offset) in &self.explode.spread {
            let reach = best.map_or(self.camera.far(), |(_, hit)| hit.distance);
            if let Some(hit) = self.doc.pick_body(body, origin - offset, ray, reach) {
                best = Some((body, hit));
            }
        }
        best
    }

    /// A press while apart: a left press on a part selects it and brings
    /// them back together; a left press on nothing does nothing. Either way
    /// the press is taken, so nothing is carved through a picture that is
    /// not where the bodies are. Other buttons still turn the view.
    pub(super) fn explode_pressed(&mut self, button: PointerButton, pixel: Vec2) -> bool {
        if !self.explode.on || button != PointerButton::Left {
            return false;
        }
        if let Some((body, _)) = self.pick_apart(pixel) {
            let name = self.doc.node(body).map_or("a body", |node| node.name.as_str());
            let note = format!("selected {name}");
            if body == self.doc.active() {
                self.status = note;
            } else {
                self.select_body(body);
            }
            self.unexplode();
        }
        true
    }

    /// The hierarchy's view buttons, in every mode: Explode and Solo active,
    /// each lit while on.
    pub(super) fn hierarchy_view_buttons(&self) -> Element<'_, Message> {
        let toggle = |name: icon::IconName, on: bool, message: Message, tip: &'static str| {
            crate::ui::tip(
                button(icon::icon(name, theme::ICON_INLINE, theme::TEXT_DIM))
                    .padding(theme::PAD_BUTTON)
                    .style(if on { theme::tool_button_active } else { theme::tool_button })
                    .on_press(message),
                tip,
            )
        };
        row![
            toggle(
                icon::IconName::Explode,
                self.explode.on,
                Message::ExplodeToggled,
                "Explode: draw the parts apart to pick one"
            ),
            toggle(
                icon::IconName::Solo,
                self.solo.is_some(),
                Message::SoloActiveToggled,
                "Show only the active body"
            ),
        ]
        .spacing(theme::S2)
        .into()
    }

    /// The Solo active button: solo on the active body, or out of solo.
    pub(super) fn toggle_solo_active(&mut self) {
        if self.solo.is_some() {
            self.exit_solo();
        } else {
            self.enter_solo(self.doc.active());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::PointerEvent;
    use hani_core::Volume;
    use iced::Vector;

    const SIZE: Vector = Vector { x: 800.0, y: 600.0 };

    fn update(app: &mut Hani, message: Message) {
        drop(app.update(message));
    }

    /// The opening ball, and a small one to its right that sits half inside
    /// it, the way a part sits against the body it was cut from.
    fn ball_and_bead() -> (Hani, NodeId, NodeId) {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        update(
            &mut app,
            Message::Pointer(PointerEvent::Moved {
                position: Vector { x: 1.0, y: 1.0 },
                size: SIZE,
            }),
        );
        let ball = app.doc.active();
        let (min, max) = app.doc.node(ball).and_then(|node| node.bounds()).expect("the ball");
        let mut bead = Volume::new(app.doc.voxel_size());
        let radius = (max.x - min.x) * 0.15;
        bead.seed_sphere(Vec3::new(max.x - radius * 0.5, 0.0, 0.0), radius);
        bead.mark_everything_dirty();
        let bead = app.doc.add_body("Bead", bead);
        app.publish_visibility();
        app.remesh_dirty();
        (app, ball, bead)
    }

    fn pixel_of(app: &Hani, point: Vec3) -> Vector {
        let at = super::super::measure::to_pixels(&app.camera, point, app.viewport_size)
            .expect("on screen");
        Vector { x: at.x, y: at.y }
    }

    fn press(app: &mut Hani, at: Vector) {
        update(
            app,
            Message::Pointer(PointerEvent::Pressed {
                button: PointerButton::Left,
                position: at,
                size: SIZE,
            }),
        );
        update(app, Message::Pointer(PointerEvent::Released { button: PointerButton::Left }));
    }

    /// Apart, the largest body stays where it is and the other is drawn
    /// clear of it, along the line from the one to the other; together
    /// again, nothing is offset. No field moves either way and nothing is
    /// written to the history.
    #[test]
    fn explode_draws_the_parts_apart_and_moves_no_field() {
        let (mut app, ball, bead) = ball_and_bead();
        let undo = app.history.stats().undo_entries;
        update(&mut app, Message::ExplodeToggled);
        assert!(app.exploded());
        let spread = app.shared.spread_snapshot();
        let of = |id| spread.iter().find(|(body, _)| *body == id).map(|(_, at)| *at);
        assert_eq!(of(ball), Some(Vec3::ZERO), "the anchor moved: {spread:?}");
        let moved = of(bead).expect("the bead has an offset");
        assert!(moved.x > 0.0 && moved.y.abs() < 1e-3 && moved.z.abs() < 1e-3, "{moved}");
        // Clear of the ball: the bead's near side is past the ball's far side.
        let (_, ball_max) = app.doc.node(ball).and_then(|node| node.bounds()).expect("ball");
        let (bead_min, _) = app.doc.node(bead).and_then(|node| node.bounds()).expect("bead");
        assert!(bead_min.x + moved.x > ball_max.x, "the bead is still inside the ball");
        assert_eq!(app.history.stats().undo_entries, undo, "a view pushed an undo entry");

        update(&mut app, Message::ExplodeToggled);
        assert!(!app.exploded());
        assert!(app.shared.spread_snapshot().is_empty(), "together again, still offset");
    }

    /// A press where the bead is DRAWN apart selects it, brings the parts
    /// back together and carves nothing.
    #[test]
    fn a_press_on_a_part_apart_selects_it_and_brings_them_back() {
        let (mut app, ball, bead) = ball_and_bead();
        update(&mut app, Message::ExplodeToggled);
        let offset = app.shared.spread_snapshot().iter().find(|(id, _)| *id == bead).unwrap().1;
        let (min, max) = app.doc.node(bead).and_then(|node| node.bounds()).expect("bead");
        let middle = (min + max) * 0.5;
        let undo = app.history.stats().undo_entries;
        let sample = |app: &Hani| app.doc.volume(bead).expect("bead").sample_world(max);
        let before = sample(&app);

        let drawn = pixel_of(&app, middle + offset);
        let under = app.pick_apart(Vec2::new(drawn.x, drawn.y));
        assert_eq!(under.map(|(id, _)| id), Some(bead), "the drawn bead does not pick");

        press(&mut app, drawn);
        assert_eq!(app.doc.active(), bead, "the press did not select the bead");
        assert!(!app.exploded(), "the press did not bring them back");
        assert_eq!(app.history.stats().undo_entries, undo, "the press carved");
        assert_eq!(sample(&app), before, "the press carved");
        assert_ne!(ball, bead);
    }

    /// A part centred inside the anchor, with no line of its own out of it,
    /// is sent across the view and not toward the eye, and far enough that
    /// it clears the anchor.
    #[test]
    fn a_part_inside_the_anchor_is_sent_across_the_view_and_clear_of_it() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        let ball = app.doc.active();
        let (min, max) = app.doc.node(ball).and_then(|node| node.bounds()).expect("the ball");
        let mut inner = Volume::new(app.doc.voxel_size());
        inner.seed_sphere((min + max) * 0.5, (max.x - min.x) * 0.3);
        inner.mark_everything_dirty();
        let inner = app.doc.add_body("Inner", inner);
        app.publish_visibility();
        // The way the view looks, which framing the parts apart keeps.
        let toward_eye = (app.camera.eye() - app.camera.target).normalize();
        update(&mut app, Message::ExplodeToggled);
        let spread = app.shared.spread_snapshot();
        let moved = spread.iter().find(|(id, _)| *id == inner).expect("an offset").1;
        assert!(moved.normalize().dot(toward_eye).abs() < 1e-3, "sent toward the eye: {moved}");
        let half = |id| {
            let (low, high) = app.doc.node(id).and_then(|node| node.bounds()).expect("bounds");
            (high - low).max_element() * 0.5
        };
        assert!(moved.length() > half(ball) + half(inner), "not clear: {moved}");
    }

    /// With the gizmo up, Explode takes it away, since it would be drawn
    /// where the body is and not where it is drawn, and picking a part puts
    /// it back on that part.
    #[test]
    fn the_gizmo_goes_while_apart_and_comes_back_on_the_part_picked() {
        let (mut app, ball, bead) = ball_and_bead();
        update(&mut app, Message::ToolChanged(super::super::Tool::Transform));
        assert_eq!(app.gizmo.map(|gizmo| gizmo.body), Some(ball));
        update(&mut app, Message::ExplodeToggled);
        assert!(app.exploded() && app.gizmo.is_none(), "the gizmo stayed up while apart");
        let offset = app.shared.spread_snapshot().iter().find(|(id, _)| *id == bead).unwrap().1;
        let (min, max) = app.doc.node(bead).and_then(|node| node.bounds()).expect("bead");
        let drawn = pixel_of(&app, (min + max) * 0.5 + offset);
        press(&mut app, drawn);
        assert!(!app.exploded());
        assert_eq!(app.gizmo.map(|gizmo| gizmo.body), Some(bead), "not back on the bead");
    }

    /// Apart, every part is framed on the screen, and together again the
    /// view is what it was.
    #[test]
    fn apart_every_part_is_in_view_and_the_view_comes_back() {
        let (mut app, _, _) = ball_and_bead();
        app.viewport_size = Vec2::new(SIZE.x, SIZE.y);
        // Close in on the ball, so that apart the bead would be off screen.
        app.camera.distance *= 0.4;
        let before = (app.camera.target, app.camera.distance);
        update(&mut app, Message::ExplodeToggled);
        for (body, offset) in app.shared.spread_snapshot() {
            let (min, max) = app.explode.boxes.iter().find(|(id, _)| *id == body).unwrap().1;
            for corner in [min, max] {
                let at = super::super::measure::to_pixels(
                    &app.camera,
                    corner + offset,
                    app.viewport_size,
                )
                .expect("in front of the eye");
                assert!(
                    (0.0..=SIZE.x).contains(&at.x) && (0.0..=SIZE.y).contains(&at.y),
                    "a part is off the screen at {at}"
                );
            }
        }
        update(&mut app, Message::ExplodeToggled);
        assert_eq!((app.camera.target, app.camera.distance), before, "the view did not come back");
    }

    /// With one body there is nothing to push apart, and it says so.
    #[test]
    fn one_body_does_not_explode() {
        let mut app = Hani::with_tablet(crate::tablet::Tablet::inert());
        update(&mut app, Message::ExplodeToggled);
        assert!(!app.exploded());
        assert_eq!(app.status, "only one body is drawn");
    }

    /// Solo active shows the active body alone, and pressed again shows
    /// everything as it was.
    #[test]
    fn solo_active_shows_the_active_body_alone() {
        let (mut app, ball, bead) = ball_and_bead();
        update(&mut app, Message::SoloActiveToggled);
        assert_eq!(app.solo, Some(ball));
        let (at_ball, at_bead) = (app.doc.index_of(ball).unwrap(), app.doc.index_of(bead).unwrap());
        assert!(app.shown[at_ball] && !app.shown[at_bead]);
        update(&mut app, Message::SoloActiveToggled);
        assert_eq!(app.solo, None);
        assert!(app.shown[at_ball] && app.shown[at_bead]);
    }
}
