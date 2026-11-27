// SPDX-License-Identifier: AGPL-3.0-only
//
// The hierarchy's three view buttons: Explode, which pushes every drawn body
// apart on screen so that a small part buried among larger ones can be seen
// and picked; Solo active, which shows the active body alone; and Others
// see-through, which draws every body but the active one see-through and out
// of reach of the pointer, so that the active one can be worked on in place
// with the rest there to see. Choosing another row then makes that one solid
// and the rest see-through. Asked for by the maintainer on 2026-11-26, with
// Explode animated both ways.
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
//
// The parts travel apart and back over `TRAVEL_MS`, eased, and the view with
// them. While they travel back nothing is where it is drawn, so a press then
// is taken and does nothing, as a press on nothing apart does.
//
// See-through is the ring cut's screen-door: a share of each pixel of the
// body discarded by a 4x4 dither, in the shader, so there is no sorting and
// the depth buffer stays right. The share rides in the fourth float of the
// body's Explode entry. The pick leaves those bodies out.

use glam::{Vec2, Vec3, Vec4};

use iced::Element;
use iced::widget::{button, row};

use super::Hani;
use crate::message::{Message, PointerButton};
use crate::{icon, theme};
use hani_core::NodeId;

/// The gap left between a part and the anchor, as a fraction of the
/// half-diagonal of everything drawn. Chosen on a screen, not measured.
const GAP: f32 = 0.15;

/// How long the parts take to travel apart or back, in milliseconds. Chosen
/// on a screen, not measured.
const TRAVEL_MS: f32 = 320.0;

/// The share of a see-through body's pixels screened away. Chosen on a
/// screen, not measured.
const SEE_THROUGH: f32 = 0.7;

/// Ease in and out, nought to one.
fn eased(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

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
    /// And the view framing the parts apart, which the view travels to.
    pub(super) view_apart: Option<(Vec3, f32)>,
    /// How far apart the parts are drawn, nought together to one apart.
    pub(super) apart: f32,
    /// Every body but the active one see-through and out of reach.
    pub(super) see_through: bool,
    /// What the renderer is told: each body's offset as far as it has
    /// travelled, and the share of it screened away. Kept, as `spread` is.
    pub(super) drawn: Vec<(NodeId, Vec4)>,
    /// Which rows the pointer may pick: the drawn ones, less the
    /// see-through ones. One a row, as `Hani::shown` is.
    pub(super) pickable: Vec<bool>,
}

impl Hani {
    /// Whether the bodies are drawn apart, or on their way apart or back.
    pub(super) fn exploded(&self) -> bool {
        self.explode.on || self.explode.apart > 0.0
    }

    /// Carry the parts and the view a frame further apart or back.
    pub(super) fn explode_frame(&mut self, elapsed_ms: f32) {
        let goal = if self.explode.on { 1.0 } else { 0.0 };
        if self.explode.apart == goal {
            return;
        }
        let step = elapsed_ms.clamp(0.0, 50.0) / TRAVEL_MS;
        self.explode.apart = if goal > self.explode.apart {
            (self.explode.apart + step).min(1.0)
        } else {
            (self.explode.apart - step).max(0.0)
        };
        if let (Some(before), Some(apart)) = (self.explode.view_before, self.explode.view_apart) {
            let t = eased(self.explode.apart);
            self.camera.target = before.0.lerp(apart.0, t);
            self.camera.distance = before.1 + (apart.1 - before.1) * t;
            self.refresh_camera_lattice();
            self.publish_camera();
        }
        if self.explode.apart == 0.0 {
            self.together();
        }
        self.publish_spread();
    }

    /// Back together: the view as it was, and the gizmo back on the active
    /// body if the tool wants one.
    fn together(&mut self) {
        if let Some((target, distance)) = self.explode.view_before.take() {
            self.camera.target = target;
            self.camera.distance = distance;
            self.refresh_camera_lattice();
            self.publish_camera();
        }
        self.explode.view_apart = None;
        if self.tool == super::Tool::Transform
            && self.gizmo.is_none()
            && let Err(why) = self.arm_gizmo()
        {
            self.status = why;
        }
        self.refresh_gizmo();
    }

    /// Run the travel to its end at once, for the tests.
    #[cfg(test)]
    pub(super) fn settle_explode(&mut self) {
        for _ in 0..100 {
            self.explode_frame(16.0);
        }
        self.publish_visibility();
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
        // Kept when they come apart again on their way back, so each part
        // turns round where it is rather than setting off another way.
        if self.explode.apart == 0.0 {
            self.explode.across = (self.camera.right(), self.camera.up());
        }
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
        // Coming apart again on the way back, the view to come back to is
        // the one from before the first time.
        let before = self.explode.view_before.unwrap_or((self.camera.target, self.camera.distance));
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
        self.explode.view_before = Some(before);
        let framed = super::OrbitCamera::framing((low + high) * 0.5, ((high - low) * 0.5).length());
        let distance = framed
            .distance
            .max(before.1)
            .clamp(self.camera.min_distance(), self.camera.max_distance());
        self.explode.view_apart = Some((framed.target, distance));
    }

    /// Start the parts back together. They travel there by the frame; see
    /// [`Self::explode_frame`].
    pub(super) fn unexplode(&mut self) {
        if !self.explode.on {
            return;
        }
        self.explode.on = false;
        self.publish_spread();
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
        if self.exploded() {
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
        // What is drawn: the offsets as far as the parts have travelled, and
        // the bodies drawn see-through. And what may be picked.
        let travelled = eased(self.explode.apart);
        let active = self.doc.active();
        self.explode.drawn.clear();
        self.explode.pickable.clear();
        for (node, shown) in self.doc.nodes().iter().zip(&self.shown) {
            let clear = self.explode.see_through && *shown && node.is_body() && node.id != active;
            self.explode.pickable.push(*shown && !clear);
            let offset = self.explode.spread.iter().find(|(id, _)| *id == node.id);
            if offset.is_some() || clear {
                let at = offset.map_or(Vec3::ZERO, |(_, offset)| *offset * travelled);
                self.explode
                    .drawn
                    .push((node.id, at.extend(if clear { SEE_THROUGH } else { 0.0 })));
            }
        }
        // The rows that cut, drawn see-through. See `app/booleans.rs`.
        self.add_boolean_ghosts();
        self.shared.set_spread(&self.explode.drawn);
    }

    /// Which rows the pointer may pick: the drawn ones, less those drawn
    /// see-through.
    pub(super) fn pick_mask(&self) -> &[bool] {
        if self.explode.see_through && self.explode.pickable.len() == self.shown.len() {
            &self.explode.pickable
        } else {
            &self.shown
        }
    }

    /// The Others see-through button: on, or off.
    pub(super) fn toggle_see_through(&mut self) {
        self.explode.see_through = !self.explode.see_through;
        if self.explode.see_through {
            // Solo hides what this shows through, so the two are one or the
            // other.
            self.exit_solo();
            self.status = format!(
                "{} solid, the rest see-through — escape leaves it",
                self.active_body_name()
            );
        } else {
            self.status = "every body solid again".to_string();
        }
        self.publish_visibility();
    }

    /// Escape: leave see-through, and say whether it was on.
    pub(super) fn see_through_escaped(&mut self) -> bool {
        if !self.explode.see_through {
            return false;
        }
        self.toggle_see_through();
        true
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
        let travelled = eased(self.explode.apart);
        for &(body, offset) in &self.explode.spread {
            let offset = offset * travelled;
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
        if !self.exploded() || button != PointerButton::Left {
            return false;
        }
        // On the way back nothing is where it is drawn.
        if !self.explode.on {
            return true;
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
        let icon_toggle = |name: icon::IconName, on: bool, message: Message, tip: &'static str| {
            crate::ui::tip(
                // Bigger than an inline icon, and a fixed square to hit:
                // `theme::HIT`.
                button(
                    iced::widget::container(icon::icon(name, theme::ICON_BUTTON, theme::TEXT_DIM))
                        .center(iced::Length::Fill),
                )
                .padding(0)
                .width(iced::Length::Fixed(theme::HIT))
                .height(iced::Length::Fixed(theme::HIT))
                .style(if on { theme::tool_button_active } else { theme::tool_button })
                .on_press(message),
                tip,
            )
        };
        row![
            icon_toggle(
                icon::IconName::Explode,
                self.explode.on,
                Message::ExplodeToggled,
                "Explode: draw the parts apart to pick one"
            ),
            icon_toggle(
                icon::IconName::Solo,
                self.solo.is_some(),
                Message::SoloActiveToggled,
                "Show only the active body"
            ),
            icon_toggle(
                icon::IconName::SeeThrough,
                self.explode.see_through,
                Message::OthersSeeThroughToggled,
                "The others see-through: only the active body can be picked"
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
            self.explode.see_through = false;
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
        app.settle_explode();
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
        app.settle_explode();
        assert!(!app.exploded());
        assert!(app.shared.spread_snapshot().is_empty(), "together again, still offset");
    }

    /// A press where the bead is DRAWN apart selects it, brings the parts
    /// back together and carves nothing.
    #[test]
    fn a_press_on_a_part_apart_selects_it_and_brings_them_back() {
        let (mut app, ball, bead) = ball_and_bead();
        update(&mut app, Message::ExplodeToggled);
        app.settle_explode();
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
        app.settle_explode();
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
        app.settle_explode();
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
        app.settle_explode();
        assert!(app.exploded() && app.gizmo.is_none(), "the gizmo stayed up while apart");
        let offset = app.shared.spread_snapshot().iter().find(|(id, _)| *id == bead).unwrap().1;
        let (min, max) = app.doc.node(bead).and_then(|node| node.bounds()).expect("bead");
        let drawn = pixel_of(&app, (min + max) * 0.5 + offset);
        press(&mut app, drawn);
        app.settle_explode();
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
        app.settle_explode();
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
        app.settle_explode();
        assert_eq!((app.camera.target, app.camera.distance), before, "the view did not come back");
    }

    /// The parts travel: a frame in, the bead is drawn part of the way out,
    /// and further the next; on the way back a press on it is taken and
    /// selects nothing.
    #[test]
    fn the_parts_travel_apart_and_a_press_on_the_way_back_does_nothing() {
        let (mut app, ball, bead) = ball_and_bead();
        update(&mut app, Message::ExplodeToggled);
        let at = |app: &Hani| {
            app.shared.spread_snapshot().iter().find(|(id, _)| *id == bead).map_or(0.0, |e| e.1.x)
        };
        app.explode_frame(TRAVEL_MS * 0.25);
        app.publish_visibility();
        let early = at(&app);
        app.explode_frame(TRAVEL_MS * 0.25);
        app.publish_visibility();
        let later = at(&app);
        app.settle_explode();
        let full = at(&app);
        assert!(0.0 < early && early < later && later < full, "{early} {later} {full}");

        update(&mut app, Message::ExplodeToggled);
        app.explode_frame(TRAVEL_MS * 0.1);
        app.publish_visibility();
        assert!(app.exploded(), "back together at once");
        let offset = app.shared.spread_snapshot().iter().find(|(id, _)| *id == bead).unwrap().1;
        let (min, max) = app.doc.node(bead).and_then(|node| node.bounds()).expect("bead");
        let drawn = pixel_of(&app, (min + max) * 0.5 + offset);
        press(&mut app, drawn);
        assert_eq!(app.doc.active(), ball, "a press on the way back selected");
        app.settle_explode();
        assert!(!app.exploded());
    }

    /// See-through: every body but the active one is drawn see-through and
    /// cannot be picked; choosing the other row swaps which; Solo and
    /// see-through are one or the other; Escape leaves it.
    #[test]
    fn the_others_see_through_cannot_be_picked_and_follow_the_active_body() {
        let (mut app, ball, bead) = ball_and_bead();
        app.viewport_size = Vec2::new(SIZE.x, SIZE.y);
        // On the bead and past the ball's edge, so nothing else is under it:
        // the bead as `ball_and_bead` made it.
        let (_, ball_max) = app.doc.node(ball).and_then(|node| node.bounds()).expect("ball");
        let (ball_min, _) = app.doc.node(ball).and_then(|node| node.bounds()).expect("ball");
        let radius = (ball_max.x - ball_min.x) * 0.15;
        let on_bead = pixel_of(&app, Vec3::new(ball_max.x - radius * 0.5 + radius * 0.7, 0.0, 0.0));
        let picked = |app: &Hani| app.pick(Vec2::new(on_bead.x, on_bead.y)).map(|(id, _)| id);
        assert_eq!(picked(&app), Some(bead), "the bead is not under the pixel");

        update(&mut app, Message::OthersSeeThroughToggled);
        assert_eq!(app.shared.see_through_snapshot(), vec![bead]);
        assert_ne!(picked(&app), Some(bead), "a see-through body was picked");
        assert!(app.shared.spread_snapshot().iter().all(|(_, at)| *at == Vec3::ZERO));

        update(&mut app, Message::BodySelected(bead));
        assert_eq!(app.shared.see_through_snapshot(), vec![ball], "did not follow the row");
        assert_eq!(picked(&app), Some(bead));

        update(&mut app, Message::SoloActiveToggled);
        assert!(app.solo.is_some() && !app.explode.see_through, "solo and see-through both on");
        update(&mut app, Message::OthersSeeThroughToggled);
        assert!(app.solo.is_none() && app.explode.see_through, "see-through left solo on");

        assert!(app.see_through_escaped());
        assert!(app.shared.see_through_snapshot().is_empty(), "still see-through");
        drop(app.hierarchy_view_buttons());
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
