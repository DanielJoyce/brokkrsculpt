// SPDX-License-Identifier: AGPL-3.0-only

//! Layers of paint: a stack of colour fields over one body, each with a
//! name, a blend, an opacity, and whether it is shown.
//!
//! `docs/PAINT-RACK.md` section 1 is the design, and the maintainer's word
//! of 2026-11-25 was to build it first and properly. Every coat, stroke,
//! decal and varnish of the paint rack goes on a layer.
//!
//! # Where each layer lives
//!
//! **The active layer is the volume's paint field**, the one every paint
//! tool, every edit and the stroke's undo recorder already work on
//! ([`crate::volume::Volume::paint`]). So nothing that paints had to learn
//! about layers: it paints the active one. The other layers are held here,
//! each with a recorder of its own for the stroke that is open.
//!
//! An operation that has to act on every layer (Move carrying the paint
//! with the surface, Bulk, a shift, a warp, a resample) runs once a layer
//! through [`crate::volume::Volume::edit_each_layer`], which swaps each
//! layer in turn into the volume's paint field with its recorder, runs, and
//! swaps it back. The code that moves one field's paint moves all of them.
//!
//! A body that has never had a second layer holds nothing here at all: an
//! empty `metas` is one implicit layer, "Paint", normal, fully opaque and
//! shown, and costs what the paint field cost before there were layers.
//!
//! # The composite
//!
//! What the renderer and the exports read is the stack put together: the
//! **composite**, one colour field like any other. For the one implicit
//! layer it is the paint field itself, so a body with one plain layer pays
//! nothing. Otherwise it is put together a brick at a time, in the bricks a
//! change made dirty, just before they are meshed
//! ([`crate::volume::Volume::drain_dirty`]).
//!
//! **The blends that depend on what is under a layer are worked out here,
//! and not baked into the layer.** A glaze, a wash and a contrast paint are
//! films that what is under them is seen through, thicker in a crease and
//! thinner on an edge. Put together at the moment of drawing, a change to a
//! lower layer changes the look of every layer over it, as it does on a
//! real model.
//!
//! # The arithmetic
//!
//! The shader lays paint over the surface as `surface * (1 - a) + p`, with
//! `p` the colour already multiplied by its coverage `a` (`sculpt.wgsl`).
//! The stack is put together in the same terms, bottom to top, starting
//! from nothing (`p = 0`, `a = 0`). For each shown layer with paint at a
//! voxel, its strength there is `k = coverage * opacity`, and what is
//! visible under it is `under = CLAY * (1 - a) + p`. The blend gives a
//! target colour from the layer's colour and `under`: for a film (Glaze,
//! Wash, Contrast) a full coat of it over `under`, as thick as the crease
//! there makes it (`Film`, below); for the others a colour of their own. Then:
//!
//! ```text
//! p' = (1 - k) p + k target
//! a' = a + k (1 - a)
//! ```
//!
//! which is exactly "the visible colour moves toward the target by `k`"
//! with the clay showing through by `1 - a'`, whatever the blend: less than
//! a full coat of a film is that share of the way from what is under it to
//! a full coat. The clay here is the export's ([`crate::export::texture`]).
//! The mixing by `k` is done on sRGB values as the paint field stores them,
//! and a film's own arithmetic in linear light; the shader lights the
//! result as it lights any paint.
//!
//! Until 2026-11-27 the strength was `k = coverage * opacity * pooling`,
//! the pooling a share by the crease, and each seen-through blend had a
//! rule of its own on sRGB values. What was wrong with that, and what was
//! tried: `Film`, below, and `docs/PAINT-RACK.md` section 1.

use contracts::debug_ensures;
use rustc_hash::FxHashMap;

use super::field::{BARE, PaintBrick, PaintField, Rgba};
use crate::brick::{BRICK_DIM, BRICK_VOXELS, BrickCoord};

/// A layer's name for itself, kept however the stack is reordered: what an
/// undo entry names a layer by.
pub type LayerId = u32;

/// The most layers a body may hold, to start (`docs/PAINT-RACK.md`).
pub const MAX_LAYERS: usize = 16;

/// The bare clay, as the exports draw it. Under a blend where nothing is
/// painted yet, this is what is under it.
pub const CLAY: [u8; 3] = [150, 140, 130];

/// How a layer lies over what is under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Blend {
    /// Covers what is under it by its coverage.
    #[default]
    Normal,
    /// Darkens what is under it by its colour.
    Multiply,
    /// Lightens what is under it.
    Screen,
    /// A thin film of the colour that what is under it is seen through:
    /// a third of a contrast coat on a flat, more in a crease. See `Film`.
    Glaze,
    /// A film that is hardly there on a flat, runs off an edge and pools
    /// in a crease. See `Film`.
    Wash,
    /// A full coat of film on a flat, which over white is the paint's
    /// colour and over another colour tints it; nearly two in a crease,
    /// under half on an edge. See `Film`.
    Contrast,
    /// No colour at all: a finish, matte, satin or gloss, over what is
    /// under it. Its paint's red is the finish's code
    /// ([`crate::paint::Finish::code`]). Stage 6 of `docs/PAINT-RACK.md`.
    Varnish,
}

impl std::fmt::Display for Blend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

impl Blend {
    pub const ALL: [Blend; 7] = [
        Blend::Normal,
        Blend::Multiply,
        Blend::Screen,
        Blend::Glaze,
        Blend::Wash,
        Blend::Contrast,
        Blend::Varnish,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Blend::Normal => "Normal",
            Blend::Multiply => "Multiply",
            Blend::Screen => "Screen",
            Blend::Glaze => "Glaze",
            Blend::Wash => "Wash",
            Blend::Contrast => "Contrast",
            Blend::Varnish => "Varnish",
        }
    }

    /// Its byte in a file.
    pub(crate) fn code(self) -> u8 {
        Blend::ALL.iter().position(|blend| *blend == self).unwrap_or(0) as u8
    }

    pub(crate) fn from_code(code: u8) -> Option<Self> {
        Blend::ALL.get(usize::from(code)).copied()
    }

    /// Whether how this blend lies depends on the shape of the surface,
    /// so that a change to the field changes the composite.
    fn reads_the_shape(self) -> bool {
        self.film().is_some_and(|film| film.into != 0.0 || film.off != 0.0)
    }

    /// The film this blend's paint is, if it is one: a paint that is seen
    /// through. See `Film`.
    fn film(self) -> Option<Film> {
        match self {
            Blend::Glaze => Some(Film::GLAZE),
            Blend::Wash => Some(Film::WASH),
            Blend::Contrast => Some(Film::CONTRAST),
            _ => None,
        }
    }

    /// The colour this blend moves toward, from the layer's colour and what
    /// is visible under it, both 0..=1, for a blend that is not a film.
    fn target(self, colour: [f32; 3], under: [f32; 3]) -> [f32; 3] {
        let each = |f: &dyn Fn(f32, f32) -> f32| {
            [f(colour[0], under[0]), f(colour[1], under[1]), f(colour[2], under[2])]
        };
        match self {
            Blend::Multiply => each(&|c, u| c * u),
            Blend::Screen => each(&|c, u| 1.0 - (1.0 - c) * (1.0 - u)),
            // A varnish lays no colour and a film is put on by `Film::over`:
            // neither comes here. See `composite_voxel`.
            Blend::Normal | Blend::Varnish | Blend::Glaze | Blend::Wash | Blend::Contrast => colour,
        }
    }
}

/// A paint that is seen through: a glaze, a wash, a contrast paint.
///
/// # What it is
///
/// A film of pigment over what is under it does two things, and a paint is
/// more of one or the other by its kind:
///
/// * **It tints.** Light goes down through the film, off what is under it,
///   and back up, and the film takes its share of each colour on the way:
///   `under * colour ^ thickness`, by Beer and Lambert, in linear light.
///   The film's colour is what one coat of it looks like over white, which
///   is what a pot of contrast paint is sold by. Over white a coat is its
///   colour; over a dark undercoat nearly nothing comes back; over a
///   yellow, a blue film leaves what both let through. Where the film is
///   thicker (pooled in a crease) it is deeper AND more saturated, and
///   where it is thinner (run off an edge) what is under it shows.
/// * **It covers.** A film has some body of its own: pigment that sends
///   light back before it reaches what is under. By that share the film's
///   own colour is seen whatever is under it, which is why a contrast paint
///   over black is not nothing, and a glaze thinned from a paint still
///   lightens a dark base.
///
/// So, a film of thickness `t` and body `b`, over `under`:
///
/// ```text
/// hidden = 1 - (1 - b) ^ t
/// seen   = (1 - hidden) * under * colour ^ t  +  hidden * colour
/// ```
///
/// # What it replaced, and why
///
/// Until 2026-11-27 each of the three had arithmetic of its own, on sRGB
/// values, none of it a film. A wash multiplied. A contrast laid its colour
/// scaled by how LIGHT what was under it was, so a blue contrast over a
/// yellow undercoat was blue: the undercoat's hue was never read. A glaze
/// took the hue of its colour only (the colour over its brightest
/// channel), so a black glaze over white was a pale peach, and then gave
/// back half the lightness it had taken. The maintainer: "It's been a mess
/// for a while... Mostly I want contrast paints and glazes to work
/// properly tinting / covering paint underneath." Seen in pictures of the
/// arithmetic (`charts_of_the_paints` in the application's
/// `app/paint_preview.rs`) before a line of this was written.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Film {
    /// How thick one full coat lies on a flat, where 1 is the thickness at
    /// which the film over white is its own colour.
    flat: f32,
    /// How much thicker it lies in a full crease, and thinner on a full
    /// edge.
    into: f32,
    off: f32,
    /// How much of what is under it a coat of thickness 1 hides, 0..=1.
    body: f32,
}

impl Film {
    /// A thin tint: a third of a contrast coat on a flat, with the body of
    /// a paint thinned with medium. It pools as well, less than a contrast
    /// does: asked for by the maintainer on 2026-11-27, "the contrast
    /// paints and glazes need to pool more around details".
    const GLAZE: Film = Film { flat: 0.35, into: 0.7, off: 0.15, body: 0.15 };
    /// Hardly there on a flat, most of two coats in a crease, and off an
    /// edge altogether. Nearly no body: a wash is ink and medium.
    const WASH: Film = Film { flat: 0.2, into: 1.5, off: 0.15, body: 0.05 };
    /// One coat on a flat, which over white is the pot's colour; nearly
    /// two in a crease; under half on an edge, where the undercoat shows as
    /// the highlight. Asked for by the maintainer on 2026-11-25: speed
    /// paints "pull in and around details".
    const CONTRAST: Film = Film { flat: 1.0, into: 0.9, off: 0.55, body: 0.12 };

    /// The least a film is taken to pass of a colour, so that a paint with
    /// nothing in a channel still has a thickness to be thinner at.
    const LEAST: f32 = 0.002;

    /// How thick a full coat lies where the surface is `crease` (-1 an
    /// edge to 1 shut in).
    #[debug_ensures(ret >= 0.0)]
    fn thickness(self, crease: f32) -> f32 {
        (self.flat + self.into * crease.max(0.0) - self.off * (-crease).max(0.0)).max(0.0)
    }

    /// What is seen where a full coat of `colour` lies over `under`, both
    /// sRGB 0..=1, and so is the answer.
    fn over(self, colour: [f32; 3], under: [f32; 3], crease: f32) -> [f32; 3] {
        let thickness = self.thickness(crease);
        let hidden = 1.0 - (1.0 - self.body).powf(thickness);
        std::array::from_fn(|channel| {
            let paint = to_light(colour[channel]).max(Film::LEAST);
            let through = to_light(under[channel]) * paint.powf(thickness);
            from_light((1.0 - hidden) * through + hidden * paint)
        })
    }
}

/// An sRGB value, 0..=1, as linear light, and back. The film's arithmetic
/// is in light; the rest of the stack's is on the values as stored.
fn to_light(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn from_light(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// What a layer is, apart from its paint.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerMeta {
    pub id: LayerId,
    pub name: String,
    pub blend: Blend,
    /// Over the whole layer, 0..=1.
    pub opacity: f32,
    pub visible: bool,
    /// A locked layer takes no paint.
    pub locked: bool,
    /// Its paint is metal: where it is on top, the surface has the metallic
    /// finish ([`crate::paint::Finish::Metallic`]). What a metallic paint
    /// from the rack is laid on.
    pub metal: bool,
}

impl LayerMeta {
    /// A plain layer: normal, fully opaque, shown, unlocked.
    pub fn plain(id: LayerId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            blend: Blend::Normal,
            opacity: 1.0,
            visible: true,
            locked: false,
            metal: false,
        }
    }

    /// Whether a layer lying like this over nothing is its paint as it is.
    fn is_plain(&self) -> bool {
        self.blend == Blend::Normal && self.opacity >= 1.0 && self.visible && !self.metal
    }
}

/// A stroke's record of one layer's bricks, as they were before it.
pub(crate) type LayerRecorder = Option<FxHashMap<BrickCoord, Option<PaintBrick>>>;

/// The layers of one body, but for the paint of the active one, which is
/// the volume's paint field. See the module documentation.
#[derive(Debug, Clone, Default)]
pub struct Layers {
    /// Bottom to top. Empty is one implicit plain layer, "Paint".
    metas: Vec<LayerMeta>,
    /// One a layer, in the same order; the active one's is kept empty, its
    /// paint being the volume's.
    fields: Vec<PaintField>,
    /// The open stroke's record of each layer but the active one.
    pub(crate) recorders: Vec<LayerRecorder>,
    active: usize,
    next_id: LayerId,
    /// The stack put together. Empty while the stack is plain.
    composite: PaintField,
    /// The finish of each voxel: the top shown varnish over it, as its red.
    /// Empty while no varnish is shown.
    finish: PaintField,
}

/// The paint of a whole stack, with what each layer is: what an undo entry
/// holds of a change to the stack, and what the project file writes.
#[derive(Debug, Clone)]
pub struct LayerState {
    pub metas: Vec<LayerMeta>,
    pub fields: Vec<PaintField>,
    pub active: usize,
}

/// Layers taken out of a stack, each with its place, and which layer was
/// active when they were: what undoing a coat keeps for its redo.
#[derive(Debug, Clone)]
pub struct TakenLayers {
    /// Bottom to top, each at the place it had.
    pub layers: Vec<(usize, LayerMeta, PaintField)>,
    pub active: LayerId,
}

impl TakenLayers {
    /// What it holds.
    pub fn bytes(&self) -> usize {
        let mut stats = crate::volume::VolumeStats::default();
        for (_, meta, field) in &self.layers {
            field.add_to_stats(&mut stats);
            stats.resident_bytes += size_of::<LayerMeta>() + meta.name.len();
        }
        stats.resident_bytes
    }
}

impl LayerState {
    /// What it holds: its paint bricks and its names.
    pub fn bytes(&self) -> usize {
        let mut stats = crate::volume::VolumeStats::default();
        for field in &self.fields {
            field.add_to_stats(&mut stats);
        }
        stats.resident_bytes
            + self.metas.iter().map(|meta| size_of::<LayerMeta>() + meta.name.len()).sum::<usize>()
    }
}

impl Layers {
    /// The layers, bottom to top. The implicit one when there are none.
    pub fn metas(&self) -> Vec<LayerMeta> {
        if self.metas.is_empty() { vec![LayerMeta::plain(0, "Paint")] } else { self.metas.clone() }
    }

    /// How many layers there are, the implicit one counted.
    pub fn len(&self) -> usize {
        self.metas.len().max(1)
    }

    /// There is always at least the implicit layer.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Which layer strokes go to, bottom being 0.
    pub fn active(&self) -> usize {
        self.active
    }

    /// The active layer's id.
    pub fn active_id(&self) -> LayerId {
        self.metas.get(self.active).map_or(0, |meta| meta.id)
    }

    /// Whether the active layer takes paint.
    pub fn active_is_locked(&self) -> bool {
        self.metas.get(self.active).is_some_and(|meta| meta.locked)
    }

    /// Whether the stack is one plain layer, whose paint is the composite.
    pub fn is_plain(&self) -> bool {
        self.metas.len() <= 1 && self.metas.first().is_none_or(LayerMeta::is_plain)
    }

    /// Whether the composite reads the shape of the surface, so that a
    /// change to the field has to be put together again.
    pub(crate) fn reads_the_shape(&self) -> bool {
        self.metas.iter().any(|meta| meta.visible && meta.blend.reads_the_shape())
    }

    /// The composite, while the stack is not plain.
    pub(crate) fn composite(&self) -> &PaintField {
        &self.composite
    }

    /// The finish of each voxel, as put together: its red is the code of
    /// the top shown varnish over it. Empty with no varnish shown.
    pub(crate) fn finish(&self) -> &PaintField {
        &self.finish
    }

    /// Whether a varnish is shown anywhere in the stack.
    /// Whether a varnish or a metal is shown anywhere in the stack: whether
    /// the surface has a finish anywhere.
    pub(crate) fn has_varnish(&self) -> bool {
        self.metas.iter().any(|meta| meta.visible && (meta.blend == Blend::Varnish || meta.metal))
    }

    /// Make the implicit layer a real one, so it can be named and stacked.
    fn realise(&mut self) {
        if self.metas.is_empty() {
            self.metas.push(LayerMeta::plain(0, "Paint"));
            self.fields.push(PaintField::default());
            self.recorders.push(None);
            self.active = 0;
            self.next_id = 1;
        }
    }

    /// A layer's paint, the active one's being `active_paint`.
    pub(crate) fn field<'a>(
        &'a self,
        index: usize,
        active_paint: &'a PaintField,
    ) -> &'a PaintField {
        if index == self.active { active_paint } else { &self.fields[index] }
    }

    /// The state of the whole stack, the active layer's paint given.
    pub(crate) fn state(&self, active_paint: &PaintField) -> LayerState {
        let metas = self.metas();
        let fields =
            (0..metas.len()).map(|index| self.field(index, active_paint).clone()).collect();
        LayerState { metas, fields, active: self.active }
    }

    /// Put a whole state back, and hand out the active layer's paint for
    /// the volume to hold.
    pub(crate) fn restore(&mut self, state: LayerState) -> PaintField {
        let LayerState { metas, mut fields, active } = state;
        let active = active.min(metas.len().saturating_sub(1));
        let active_paint = std::mem::take(&mut fields[active]);
        self.next_id = metas.iter().map(|meta| meta.id + 1).max().unwrap_or(1).max(self.next_id);
        self.recorders = vec![None; metas.len()];
        if metas.len() == 1 && metas[0].is_plain() && metas[0].name == "Paint" && metas[0].id == 0 {
            self.metas.clear();
            self.fields.clear();
            self.recorders.clear();
        } else {
            self.metas = metas;
            self.fields = fields;
        }
        self.active = if self.metas.is_empty() { 0 } else { active };
        self.composite = PaintField::default();
        self.finish = PaintField::default();
        active_paint
    }

    /// A new layer above the active one, made active. Its paint is empty.
    /// `None` when the stack is full.
    pub(crate) fn add(
        &mut self,
        name: impl Into<String>,
        blend: Blend,
        active_paint: &mut PaintField,
    ) -> Option<LayerId> {
        self.realise();
        if self.metas.len() >= MAX_LAYERS {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        let at = self.active + 1;
        self.metas.insert(at, LayerMeta { blend, ..LayerMeta::plain(id, name) });
        self.fields.insert(at, PaintField::default());
        self.recorders.insert(at, None);
        self.swap_active(at, active_paint);
        Some(id)
    }

    /// A copy of the layer at `index` over it, made active. `None` when
    /// the stack is full.
    pub(crate) fn duplicate(
        &mut self,
        index: usize,
        active_paint: &mut PaintField,
    ) -> Option<LayerId> {
        self.realise();
        if self.metas.len() >= MAX_LAYERS || index >= self.metas.len() {
            return None;
        }
        let field = self.field(index, active_paint).clone();
        let id = self.next_id;
        self.next_id += 1;
        let meta = LayerMeta {
            id,
            name: format!("{} copy", self.metas[index].name),
            ..self.metas[index].clone()
        };
        let at = index + 1;
        self.metas.insert(at, meta);
        self.fields.insert(at, field);
        self.recorders.insert(at, None);
        if self.active >= at {
            self.active += 1;
        }
        self.swap_active(at, active_paint);
        Some(id)
    }

    /// Take the layers with these ids out of the stack, with their places,
    /// and the id of the layer active before. The one that was active stays
    /// so if it is left, or the one nearest it below. A stack left with one
    /// plain layer named "Paint" is the implicit one again.
    pub(crate) fn take(&mut self, ids: &[LayerId], active_paint: &mut PaintField) -> TakenLayers {
        let active = self.active_id();
        if self.metas.is_empty() {
            return TakenLayers { layers: Vec::new(), active };
        }
        // Every layer's paint in `fields` while the stack is rearranged.
        std::mem::swap(&mut self.fields[self.active], active_paint);
        let mut places: Vec<usize> = self
            .metas
            .iter()
            .enumerate()
            .filter(|(_, meta)| ids.contains(&meta.id))
            .map(|(at, _)| at)
            .collect();
        places.sort_unstable();
        let was = self.active;
        let mut layers = Vec::with_capacity(places.len());
        for &at in places.iter().rev() {
            let meta = self.metas.remove(at);
            let field = self.fields.remove(at);
            self.recorders.remove(at);
            layers.push((at, meta, field));
        }
        layers.reverse();
        if self.metas.is_empty() {
            self.metas.push(LayerMeta::plain(0, "Paint"));
            self.fields.push(PaintField::default());
            self.recorders.push(None);
        }
        let below = places.iter().filter(|at| **at < was).count();
        self.active = self
            .metas
            .iter()
            .position(|meta| meta.id == active)
            .unwrap_or(was.saturating_sub(below).min(self.metas.len() - 1));
        std::mem::swap(&mut self.fields[self.active], active_paint);
        if self.metas.len() == 1
            && self.metas[0].is_plain()
            && self.metas[0].id == 0
            && self.metas[0].name == "Paint"
        {
            self.metas.clear();
            self.fields.clear();
            self.recorders.clear();
            self.active = 0;
        }
        self.composite = PaintField::default();
        self.finish = PaintField::default();
        TakenLayers { layers, active }
    }

    /// Give the stack these layers' settings in this order, the same layers
    /// as it has, and make the one with `active` active; and hand back the
    /// settings and order it had. What undoing a change to a layer's
    /// settings or place does, without the paint.
    pub(crate) fn arrange(
        &mut self,
        metas: Vec<LayerMeta>,
        active: LayerId,
        active_paint: &mut PaintField,
    ) -> (Vec<LayerMeta>, LayerId) {
        let had = (self.metas(), self.active_id());
        let same = {
            let mut now: Vec<LayerId> = had.0.iter().map(|meta| meta.id).collect();
            let mut then: Vec<LayerId> = metas.iter().map(|meta| meta.id).collect();
            now.sort_unstable();
            then.sort_unstable();
            now == then
        };
        if !same {
            return had;
        }
        self.realise();
        std::mem::swap(&mut self.fields[self.active], active_paint);
        let mut fields = Vec::with_capacity(metas.len());
        let mut recorders = Vec::with_capacity(metas.len());
        for meta in &metas {
            let at = self.metas.iter().position(|held| held.id == meta.id).expect("the same ids");
            fields.push(std::mem::take(&mut self.fields[at]));
            recorders.push(self.recorders[at].take());
        }
        self.metas = metas;
        self.fields = fields;
        self.recorders = recorders;
        self.active = self.metas.iter().position(|meta| meta.id == active).unwrap_or(0);
        std::mem::swap(&mut self.fields[self.active], active_paint);
        if self.metas.len() == 1
            && self.metas[0].is_plain()
            && self.metas[0].id == 0
            && self.metas[0].name == "Paint"
        {
            self.metas.clear();
            self.fields.clear();
            self.recorders.clear();
            self.active = 0;
        }
        self.composite = PaintField::default();
        self.finish = PaintField::default();
        had
    }

    /// Put layers taken out back at their places, and make the one that was
    /// active then active again.
    pub(crate) fn put_back(&mut self, taken: TakenLayers, active_paint: &mut PaintField) {
        if taken.layers.is_empty() {
            return;
        }
        self.realise();
        std::mem::swap(&mut self.fields[self.active], active_paint);
        for (at, meta, field) in taken.layers {
            let at = at.min(self.metas.len());
            self.next_id = self.next_id.max(meta.id + 1);
            self.metas.insert(at, meta);
            self.fields.insert(at, field);
            self.recorders.insert(at, None);
        }
        self.active = self.metas.iter().position(|meta| meta.id == taken.active).unwrap_or(0);
        std::mem::swap(&mut self.fields[self.active], active_paint);
        self.composite = PaintField::default();
        self.finish = PaintField::default();
    }

    /// Make another layer the active one.
    pub(crate) fn swap_active(&mut self, to: usize, active_paint: &mut PaintField) {
        if self.metas.is_empty() || to >= self.metas.len() || to == self.active {
            return;
        }
        std::mem::swap(&mut self.fields[self.active], active_paint);
        std::mem::swap(&mut self.fields[to], active_paint);
        self.active = to;
    }

    /// Change what a layer is (its name, blend, opacity, shown, locked).
    pub(crate) fn set_meta(&mut self, index: usize, meta: LayerMeta) {
        self.realise();
        if let Some(held) = self.metas.get_mut(index) {
            *held = LayerMeta { id: held.id, ..meta };
        }
    }

    /// Delete a layer. The last one cannot go: it is emptied instead.
    pub(crate) fn delete(&mut self, index: usize, active_paint: &mut PaintField) {
        self.realise();
        if index >= self.metas.len() {
            return;
        }
        if self.metas.len() == 1 {
            *active_paint = PaintField::default();
            return;
        }
        if index == self.active {
            // The one below becomes active, or the one above at the bottom.
            let next = if index > 0 { index - 1 } else { 1 };
            self.swap_active(next, active_paint);
        }
        self.metas.remove(index);
        self.fields.remove(index);
        self.recorders.remove(index);
        if self.active > index {
            self.active -= 1;
        }
    }

    /// Move a layer from one place in the stack to another, the active one
    /// staying active wherever it goes.
    pub(crate) fn reorder(&mut self, from: usize, to: usize) {
        self.realise();
        if from >= self.metas.len() || to >= self.metas.len() || from == to {
            return;
        }
        let active_id = self.active_id();
        let meta = self.metas.remove(from);
        let field = self.fields.remove(from);
        let recorder = self.recorders.remove(from);
        self.metas.insert(to, meta);
        self.fields.insert(to, field);
        self.recorders.insert(to, recorder);
        self.active = self.metas.iter().position(|meta| meta.id == active_id).unwrap_or(0);
    }

    /// Where the layer with this id is.
    pub fn index_of(&self, id: LayerId) -> Option<usize> {
        if self.metas.is_empty() {
            return (id == 0).then_some(0);
        }
        self.metas.iter().position(|meta| meta.id == id)
    }

    /// A layer's paint by its id, to be written by an undo, the active
    /// one's being `active_paint`.
    pub(crate) fn field_mut<'a>(
        &'a mut self,
        id: LayerId,
        active_paint: &'a mut PaintField,
    ) -> Option<&'a mut PaintField> {
        let index = self.index_of(id)?;
        if index == self.active || self.metas.is_empty() {
            Some(active_paint)
        } else {
            Some(&mut self.fields[index])
        }
    }

    /// Every layer, the active one's paint given, with what it is, bottom
    /// to top.
    pub(crate) fn stack<'a>(
        &'a self,
        active_paint: &'a PaintField,
    ) -> Vec<(LayerMeta, &'a PaintField)> {
        self.metas()
            .into_iter()
            .enumerate()
            .map(|(index, meta)| (meta, self.field(index, active_paint)))
            .collect()
    }

    /// Every layer's paint, the active one's given, mapped through `f`,
    /// as a new stack whose active paint is handed back beside it. For an
    /// operation that rebuilds a body (a shift, a warp, a copy).
    pub(crate) fn mapped(
        &self,
        active_paint: &PaintField,
        f: impl Fn(&PaintField) -> PaintField,
    ) -> (Layers, PaintField) {
        let mut out = self.clone();
        out.recorders = vec![None; out.recorders.len()];
        out.composite = PaintField::default();
        out.finish = PaintField::default();
        for (index, field) in out.fields.iter_mut().enumerate() {
            if index != self.active {
                *field = f(field);
            }
        }
        (out, f(active_paint))
    }

    /// The same layers, every one empty: what a body rebuilt by a pass
    /// that fills each layer from its source starts from.
    pub(crate) fn emptied(&self) -> Layers {
        let mut out = self.clone();
        out.fields = vec![PaintField::default(); out.fields.len()];
        out.recorders = vec![None; out.recorders.len()];
        out.composite = PaintField::default();
        out.finish = PaintField::default();
        out
    }

    /// One brick of the stack put together, the colour and the finish,
    /// without storing either: what [`Layers::put_together`] stores, and
    /// what the volume works out for many bricks across every core.
    ///
    /// `crease` gives how concave the surface is at a voxel of the brick
    /// (-1..=1), and is asked only where a blend reads the shape.
    pub(crate) fn composed(
        &self,
        coord: BrickCoord,
        active_paint: &PaintField,
        crease: impl Fn(usize) -> f32,
    ) -> (Option<PaintBrick>, Option<PaintBrick>) {
        let metas = self.metas();
        let field =
            |index: usize| if index == self.active { active_paint } else { &self.fields[index] };

        // The finish: from the top down, the first shown layer with half a
        // coat or more decides. A varnish gives its own; a metal layer's
        // paint is metallic; any other paint covers what was under it and
        // has none.
        let shown: Vec<(&LayerMeta, &PaintBrick)> = metas
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, meta)| meta.visible && meta.opacity > 0.0)
            .filter_map(|(index, meta)| field(index).brick(coord).map(|brick| (meta, brick)))
            .collect();
        let finishing = shown.iter().any(|(meta, _)| meta.blend == Blend::Varnish || meta.metal);
        let finish = finishing
            .then(|| {
                let mut out = PaintBrick::dense_filled(BARE);
                let data = out.make_dense();
                let mut any = false;
                for (voxel, slot) in data.iter_mut().enumerate() {
                    let top = shown
                        .iter()
                        .map(|(meta, brick)| (meta, brick.rgba_at(voxel)))
                        .find(|(_, rgba)| rgba[3] >= 128);
                    let code = match top {
                        Some((meta, rgba)) if meta.blend == Blend::Varnish => rgba[0],
                        Some((meta, _)) if meta.metal => crate::paint::Finish::Metallic.code(),
                        _ => 0,
                    };
                    if code != 0 {
                        *slot = [code, 0, 0, 255];
                        any = true;
                    }
                }
                any.then_some(out)
            })
            .flatten();

        let layers: Vec<(&LayerMeta, &PaintBrick)> = metas
            .iter()
            .enumerate()
            .filter(|(_, meta)| meta.visible && meta.opacity > 0.0 && meta.blend != Blend::Varnish)
            .filter_map(|(index, meta)| field(index).brick(coord).map(|brick| (meta, brick)))
            .collect();
        if layers.is_empty() {
            return (None, finish);
        }
        let shaped = layers.iter().any(|(meta, _)| meta.blend.reads_the_shape());
        let mut out = PaintBrick::dense_filled(BARE);
        let data = out.make_dense();
        let mut any = false;
        for (voxel, slot) in data.iter_mut().enumerate() {
            // The crease only where a layer that pools has paint: it is a
            // march of rays, and most voxels of a brick have none.
            let pools = shaped
                && layers.iter().any(|(meta, brick)| {
                    meta.blend.reads_the_shape() && brick.rgba_at(voxel)[3] != 0
                });
            let crease = if pools { crease(voxel) } else { 0.0 };
            let rgba = composite_voxel(
                layers.iter().map(|(meta, brick)| (*meta, brick.rgba_at(voxel))),
                crease,
            );
            any |= rgba[3] != 0;
            *slot = rgba;
        }
        (any.then_some(out), finish)
    }

    /// Store one brick of the stack put together.
    pub(crate) fn store_composed(
        &mut self,
        coord: BrickCoord,
        colour: Option<PaintBrick>,
        finish: Option<PaintBrick>,
    ) {
        self.composite.restore_brick(coord, colour);
        self.finish.restore_brick(coord, finish);
    }

    /// Swap the layer at `index` with the volume's paint field and its
    /// recorder: what [`crate::volume::Volume::edit_each_layer`] does
    /// either side of running on that layer.
    pub(crate) fn swap_with(
        &mut self,
        index: usize,
        paint: &mut PaintField,
        recorder: &mut LayerRecorder,
    ) {
        std::mem::swap(&mut self.fields[index], paint);
        std::mem::swap(&mut self.recorders[index], recorder);
    }

    /// Whether there are real layers, and not only the implicit one.
    pub(crate) fn has_layers(&self) -> bool {
        !self.metas.is_empty()
    }

    /// Open a stroke's record on every layer but the active one.
    pub(crate) fn open_recorders(&mut self) {
        for (index, recorder) in self.recorders.iter_mut().enumerate() {
            if index != self.active {
                *recorder = Some(FxHashMap::default());
            }
        }
    }

    /// Close the stroke's record on every layer but the active one, and
    /// hand out what each held, by the layer's id.
    pub(crate) fn take_records(&mut self) -> Vec<(LayerId, BrickCoord, Option<PaintBrick>)> {
        let mut out = Vec::new();
        for (index, recorder) in self.recorders.iter_mut().enumerate() {
            if let Some(record) = recorder.take() {
                let id = self.metas[index].id;
                out.extend(record.into_iter().map(|(coord, brick)| (id, coord, brick)));
            }
        }
        out
    }

    /// Collapse every layer's bricks but the active one's, at a stroke's
    /// end.
    pub(crate) fn collapse_all(&mut self) {
        for (index, field) in self.fields.iter_mut().enumerate() {
            if index != self.active {
                field.collapse();
            }
        }
    }

    /// Add every layer's bricks but the active one's to the volume's stats.
    pub(crate) fn add_to_stats(&self, stats: &mut crate::volume::VolumeStats) {
        for (index, field) in self.fields.iter().enumerate() {
            if index != self.active {
                field.add_to_stats(stats);
            }
        }
        self.composite.add_to_stats(stats);
    }

    /// Forget the composite, so every brick is put together again.
    pub(crate) fn forget_composite(&mut self) {
        self.composite = PaintField::default();
        self.finish = PaintField::default();
    }

    /// Every brick any layer has paint in.
    pub(crate) fn painted_bricks(&self, active_paint: &PaintField) -> Vec<BrickCoord> {
        let mut coords: Vec<BrickCoord> = active_paint.bricks().keys().copied().collect();
        for (index, field) in self.fields.iter().enumerate() {
            if index != self.active {
                coords.extend(field.bricks().keys().copied());
            }
        }
        coords.sort_unstable_by_key(|coord| (coord.0.x, coord.0.y, coord.0.z));
        coords.dedup();
        coords
    }

    /// Merge the layer at `index` into the one under it, which takes the
    /// two as the composite of the pair, and becomes a normal layer. The
    /// active one stays active, or becomes the merged one if it was the
    /// one merged.
    pub(crate) fn merge_down(&mut self, index: usize, active_paint: &mut PaintField) {
        self.realise();
        if index == 0 || index >= self.metas.len() {
            return;
        }
        // A varnish holds a finish and a paint a colour: the two do not
        // merge into one layer.
        if self.metas[index].blend == Blend::Varnish
            || self.metas[index - 1].blend == Blend::Varnish
        {
            return;
        }
        let below = index - 1;
        let pair = [self.metas[below].clone(), self.metas[index].clone()];
        let merged = {
            let lower = self.field(below, active_paint);
            let upper = self.field(index, active_paint);
            merge_fields(&pair, &[lower, upper])
        };
        let was_active = self.active_id();
        if self.active == below || self.active == index {
            *active_paint = merged;
            // The lower layer's own field is empty while it is active.
            self.fields[below] = PaintField::default();
            self.active = below;
        } else {
            self.fields[below] = merged;
        }
        self.metas[below] = LayerMeta {
            blend: Blend::Normal,
            opacity: 1.0,
            visible: true,
            ..self.metas[below].clone()
        };
        self.metas.remove(index);
        self.fields.remove(index);
        self.recorders.remove(index);
        self.active = self
            .metas
            .iter()
            .position(|meta| meta.id == was_active)
            .unwrap_or(below.min(self.metas.len() - 1));
    }

    /// The whole stack into one layer, the composite, named "Paint".
    pub(crate) fn flatten(&mut self, active_paint: &mut PaintField) {
        if self.metas.is_empty() {
            return;
        }
        let metas = self.metas();
        let fields: Vec<&PaintField> =
            (0..metas.len()).map(|index| self.field(index, active_paint)).collect();
        let flat = merge_fields(&metas, &fields);
        // Every varnish stays, over the paint put together: a finish is not
        // a colour to merge.
        let varnishes: Vec<(LayerMeta, PaintField)> = metas
            .iter()
            .zip(&fields)
            .filter(|(meta, _)| meta.blend == Blend::Varnish)
            .map(|(meta, field)| (meta.clone(), (*field).clone()))
            .collect();
        *active_paint = flat;
        self.metas.clear();
        self.fields.clear();
        self.recorders.clear();
        self.active = 0;
        if !varnishes.is_empty() {
            self.metas.push(LayerMeta::plain(self.next_id, "Paint"));
            self.next_id += 1;
            self.fields.push(PaintField::default());
            self.recorders.push(None);
            for (meta, field) in varnishes {
                self.metas.push(meta);
                self.fields.push(field);
                self.recorders.push(None);
            }
        }
        self.composite = PaintField::default();
        self.finish = PaintField::default();
    }
}

/// A set of layers put together into one field, with no crease anywhere:
/// what a merge and a flatten keep. A wash or a contrast loses its pooling
/// in a merge, which is why the panel says what merging does.
pub(crate) fn merge_fields(metas: &[LayerMeta], fields: &[&PaintField]) -> PaintField {
    let mut coords: Vec<BrickCoord> =
        fields.iter().flat_map(|field| field.bricks().keys().copied()).collect();
    coords.sort_unstable_by_key(|coord| (coord.0.x, coord.0.y, coord.0.z));
    coords.dedup();
    let mut out = PaintField::default();
    for coord in coords {
        let bricks: Vec<(&LayerMeta, &PaintBrick)> = metas
            .iter()
            .zip(fields)
            .filter(|(meta, _)| meta.visible && meta.opacity > 0.0)
            .filter_map(|(meta, field)| field.brick(coord).map(|brick| (meta, brick)))
            .collect();
        let mut brick = PaintBrick::dense_filled(BARE);
        let data = brick.make_dense();
        let mut any = false;
        for (voxel, slot) in data.iter_mut().enumerate() {
            *slot = composite_voxel(bricks.iter().map(|(meta, b)| (*meta, b.rgba_at(voxel))), 0.0);
            any |= slot[3] != 0;
        }
        if any {
            out.insert(coord, brick);
        }
    }
    out.collapse();
    out
}

/// One voxel of the stack put together, from its layers bottom to top with
/// what each holds there, and how concave the surface is there. See the
/// module documentation for the arithmetic.
#[debug_ensures(ret[3] != 0 || ret == BARE, "a voxel with no coverage has no colour")]
pub(crate) fn composite_voxel<'a>(
    layers: impl Iterator<Item = (&'a LayerMeta, Rgba)>,
    crease: f32,
) -> Rgba {
    let clay = [f32::from(CLAY[0]) / 255.0, f32::from(CLAY[1]) / 255.0, f32::from(CLAY[2]) / 255.0];
    let mut p = [0.0_f32; 3];
    let mut a = 0.0_f32;
    for (meta, rgba) in layers {
        // A varnish lays a finish and no colour.
        if rgba[3] == 0 || meta.blend == Blend::Varnish {
            continue;
        }
        let k = f32::from(rgba[3]) / 255.0 * meta.opacity.clamp(0.0, 1.0);
        if k <= 0.0 {
            continue;
        }
        let colour =
            [f32::from(rgba[0]) / 255.0, f32::from(rgba[1]) / 255.0, f32::from(rgba[2]) / 255.0];
        let under =
            [clay[0] * (1.0 - a) + p[0], clay[1] * (1.0 - a) + p[1], clay[2] * (1.0 - a) + p[2]];
        // A film's target is a full coat of it over what is under, as thick
        // as the crease makes it; less than a full coat is that much of the
        // way there.
        let target = match meta.blend.film() {
            Some(film) => film.over(colour, under, crease),
            None => meta.blend.target(colour, under),
        };
        for channel in 0..3 {
            p[channel] = (1.0 - k) * p[channel] + k * target[channel];
        }
        a += k * (1.0 - a);
    }
    if a <= 0.0 {
        return BARE;
    }
    let coverage = (a * 255.0).round().clamp(0.0, 255.0) as u8;
    if coverage == 0 {
        return BARE;
    }
    let straight = |channel: f32| ((channel / a).clamp(0.0, 1.0) * 255.0).round() as u8;
    [straight(p[0]), straight(p[1]), straight(p[2]), coverage]
}

/// `paint` laid at full coverage by a layer of `blend` over `under`, an
/// opaque coat, where the surface is `crease` (-1 an edge to 1 shut in):
/// what a paint looks like over an undercoat, by the stack's own
/// arithmetic. A preview of a paint is made of this (`app/paint_preview.rs`).
pub fn over_undercoat(under: [u8; 3], paint: [u8; 3], blend: Blend, crease: f32) -> [u8; 3] {
    let below = LayerMeta::plain(0, "Under");
    let above = LayerMeta { blend, ..LayerMeta::plain(1, "Paint") };
    let [r, g, b, _] = composite_voxel(
        [
            (&below, [under[0], under[1], under[2], 255]),
            (&above, [paint[0], paint[1], paint[2], 255]),
        ]
        .into_iter(),
        crease,
    );
    [r, g, b]
}

/// How concave the surface is at a voxel, -1..=1, from the field's six
/// neighbours: the field's Laplacian, which a distance field has positive
/// over a convex form and negative in a crease, turned so a crease is
/// positive.
pub(crate) fn crease_from(centre: f32, neighbours: [f32; 6]) -> f32 {
    let sum: f32 = neighbours.iter().sum();
    let laplacian = sum - 6.0 * centre;
    (-laplacian).clamp(-1.0, 1.0)
}

/// Every voxel of a brick, as (x, y, z) within it, for the crease.
pub(crate) fn voxel_of(index: usize) -> (i32, i32, i32) {
    let dim = BRICK_DIM;
    ((index % dim) as i32, ((index / dim) % dim) as i32, (index / (dim * dim)) as i32)
}

const _: () = assert!(BRICK_VOXELS == BRICK_DIM * BRICK_DIM * BRICK_DIM);

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn meta(blend: Blend, opacity: f32) -> LayerMeta {
        LayerMeta { blend, opacity, ..LayerMeta::plain(1, "test") }
    }

    fn rgba() -> impl Strategy<Value = Rgba> {
        (any::<u8>(), any::<u8>(), any::<u8>(), any::<u8>()).prop_map(|(r, g, b, a)| [r, g, b, a])
    }

    /// The lightness of what is seen, the clay showing through by what the
    /// paint leaves uncovered, by the Rec. 709 weights on the stored values.
    fn lightness(rgba: Rgba) -> f32 {
        let clay = CLAY.map(f32::from);
        let a = f32::from(rgba[3]) / 255.0;
        let visible = |c: usize| f32::from(rgba[c]) * a + clay[c] * (1.0 - a);
        0.2126 * visible(0) + 0.7152 * visible(1) + 0.0722 * visible(2)
    }

    const FILMS: [Blend; 3] = [Blend::Glaze, Blend::Wash, Blend::Contrast];

    /// `paint` as a layer of `blend`, `coverage` of it, over an opaque
    /// `under`, where the surface is `crease`.
    fn filmed(under: [u8; 3], paint: [u8; 3], blend: Blend, coverage: u8, crease: f32) -> [u8; 3] {
        let (base, top) = (meta(Blend::Normal, 1.0), meta(blend, 1.0));
        let [r, g, b, _] = composite_voxel(
            [
                (&base, [under[0], under[1], under[2], 255]),
                (&top, [paint[0], paint[1], paint[2], coverage]),
            ]
            .into_iter(),
            crease,
        );
        [r, g, b]
    }

    fn sum(colour: [u8; 3]) -> i32 {
        colour.iter().map(|c| i32::from(*c)).sum()
    }

    /// A glaze, a wash or a contrast pooled in a crease is deeper than the
    /// same paint on a flat, and a plain paint is the same in both.
    #[test]
    fn pooled_paint_is_deeper_in_a_crease() {
        let lightness = |rgba: Rgba| u32::from(rgba[0]) + u32::from(rgba[1]) + u32::from(rgba[2]);
        let under = [200, 200, 200];
        for blend in FILMS {
            let flat = over_undercoat(under, [150, 40, 30], blend, 0.0);
            let shut = over_undercoat(under, [150, 40, 30], blend, 1.0);
            let flat = lightness([flat[0], flat[1], flat[2], 255]);
            let shut = lightness([shut[0], shut[1], shut[2], 255]);
            // Forty levels in the three together. A contrast, already a full
            // coat on a flat, is 53 deeper in a crease; the 60 asked before
            // 2026-11-27 was the old arithmetic's.
            assert!(shut + 40 < flat, "{blend:?}: {shut} in a crease and {flat} on a flat");
        }
        assert_eq!(
            over_undercoat(under, [150, 40, 30], Blend::Normal, 1.0),
            over_undercoat(under, [150, 40, 30], Blend::Normal, 0.0)
        );
    }

    #[test]
    fn one_plain_layer_is_its_paint() {
        for paint in [[200, 10, 30, 255], [0, 0, 0, 255], [255, 255, 255, 255]] {
            let plain = meta(Blend::Normal, 1.0);
            assert_eq!(composite_voxel([(&plain, paint)].into_iter(), 0.0), paint);
        }
        let plain = meta(Blend::Normal, 1.0);
        assert_eq!(composite_voxel([(&plain, BARE)].into_iter(), 0.0), BARE);
    }

    /// Over white, a full coat of a contrast paint on a flat is the colour
    /// of the pot, to a level or two: what a contrast paint is sold by.
    /// Of a channel below about 16 levels the film is taken to pass a
    /// little (`Film::LEAST`), and over white such a channel comes out up
    /// to 7 levels light; the paints here keep clear of that.
    #[test]
    fn a_contrast_over_white_is_its_own_colour() {
        for paint in [[200, 30, 20], [30, 60, 160], [90, 140, 40], [40, 35, 30], [230, 200, 60]] {
            let seen = filmed([255; 3], paint, Blend::Contrast, 255, 0.0);
            for channel in 0..3 {
                let off = (i32::from(seen[channel]) - i32::from(paint[channel])).abs();
                assert!(off <= 2, "{paint:?} over white is {seen:?}");
            }
        }
    }

    /// A contrast over a yellow and over a blue as light as it are not the
    /// same: the undercoat's hue is read, and not only how light it is.
    /// The rule before 2026-11-27 took only the lightness, and a blue
    /// contrast over yellow stayed blue.
    #[test]
    fn a_contrast_reads_the_hue_under_it() {
        let yellow = [230, 200, 40];
        let blue = [150, 205, 255];
        let light = |c: [u8; 3]| lightness([c[0], c[1], c[2], 255]);
        assert!((light(yellow) - light(blue)).abs() < 4.0, "the two undercoats differ in light");
        let paint = [40, 80, 190];
        let over_yellow = filmed(yellow, paint, Blend::Contrast, 255, 0.0);
        let over_blue = filmed(blue, paint, Blend::Contrast, 255, 0.0);
        let apart = (0..3)
            .map(|c| (i32::from(over_yellow[c]) - i32::from(over_blue[c])).abs())
            .max()
            .unwrap_or(0);
        assert!(apart > 30, "over yellow {over_yellow:?} and over blue {over_blue:?}");
        // Over the yellow the blue film lets through what both pass: less
        // blue than over the blue undercoat.
        assert!(over_yellow[2] + 30 < over_blue[2], "{over_yellow:?} against {over_blue:?}");
    }

    /// A black glaze over white darkens it, past a mid grey. The rule
    /// before 2026-11-27, a film of the hue that gave back half the
    /// lightness, left it lighter than a mid grey and pale peach.
    #[test]
    fn a_black_glaze_darkens_white() {
        let seen = filmed([255; 3], [0; 3], Blend::Glaze, 255, 0.0);
        assert!(seen.iter().all(|c| *c < 128), "a black glaze over white is {seen:?}");
        assert!(seen[0] == seen[1] && seen[1] == seen[2], "a black glaze over white has a hue");
    }

    /// A contrast over black is not black, the body of the paint showing,
    /// and is far darker than the same paint over white.
    #[test]
    fn a_contrast_over_black_shows_a_little_of_itself() {
        for paint in [[200, 30, 20], [30, 60, 160], [230, 200, 60]] {
            let over_black = filmed([0; 3], paint, Blend::Contrast, 255, 0.0);
            let over_white = filmed([255; 3], paint, Blend::Contrast, 255, 0.0);
            assert!(sum(over_black) > 30, "{paint:?} over black is {over_black:?}");
            assert!(
                sum(over_black) * 2 < sum(over_white),
                "{paint:?}: {over_black:?} over black and {over_white:?} over white"
            );
        }
    }

    proptest! {
        /// A normal layer at full coverage and opacity hides what is under
        /// it, whatever that is.
        #[test]
        fn a_full_normal_layer_hides_what_is_under_it(under in rgba(), over in rgba()) {
            let over = [over[0], over[1], over[2], 255];
            let (a, b) = (meta(Blend::Normal, 1.0), meta(Blend::Normal, 1.0));
            let out = composite_voxel([(&a, under), (&b, over)].into_iter(), 0.0);
            prop_assert_eq!(out, over);
        }

        /// A multiply never makes what is under it lighter, and a screen
        /// never darker.
        #[test]
        fn multiply_darkens_and_screen_lightens(
            under in rgba(), over in rgba(), opacity in 0.0_f32..=1.0, crease in -1.0_f32..=1.0,
        ) {
            let base = meta(Blend::Normal, 1.0);
            let alone = lightness(composite_voxel([(&base, under)].into_iter(), crease));
            for (blend, darker) in [(Blend::Multiply, true), (Blend::Screen, false)] {
                let top = meta(blend, opacity);
                let both = lightness(composite_voxel([(&base, under), (&top, over)].into_iter(), crease));
                if darker {
                    prop_assert!(both <= alone + 1.0, "{:?} lightened {} to {}", blend, alone, both);
                } else {
                    prop_assert!(both + 1.0 >= alone, "{:?} darkened {} to {}", blend, alone, both);
                }
            }
        }

        /// A film never makes a channel of what is under it lighter than
        /// both the undercoat and the paint are in it, to a level or two:
        /// it tints and covers, and never shines.
        #[test]
        fn a_film_is_never_lighter_than_both(
            under in any::<[u8; 3]>(),
            paint in any::<[u8; 3]>(),
            blend in 0usize..3,
            coverage in any::<u8>(),
            crease in -1.0_f32..=1.0,
        ) {
            let seen = filmed(under, paint, FILMS[blend], coverage, crease);
            for channel in 0..3 {
                let most = under[channel].max(paint[channel]);
                prop_assert!(
                    seen[channel] <= most.saturating_add(2),
                    "{:?} of {:?} over {:?} is {:?}", FILMS[blend], paint, under, seen
                );
            }
        }

        /// Over an undercoat lighter than the paint, every film is darker
        /// in a crease than on a flat, where it lies thicker, and lighter
        /// on an edge, where it runs thin, to a level or two.
        #[test]
        fn a_film_is_darker_in_a_crease_and_lighter_on_an_edge(
            paint in any::<[u8; 3]>(),
            lift in any::<[u8; 3]>(),
            blend in 0usize..3,
        ) {
            let under: [u8; 3] = std::array::from_fn(|c| {
                paint[c].saturating_add(40).saturating_add(lift[c] / 2)
            });
            let blend = FILMS[blend];
            let at = |crease: f32| sum(filmed(under, paint, blend, 255, crease));
            let (edge, flat, shut) = (at(-1.0), at(0.0), at(1.0));
            prop_assert!(shut <= flat + 3, "{:?} of {:?} over {:?}: {} in a crease, {} on a flat",
                blend, paint, under, shut, flat);
            prop_assert!(edge + 3 >= flat, "{:?} of {:?} over {:?}: {} on an edge, {} on a flat",
                blend, paint, under, edge, flat);
        }

        /// Less of a film is less of it: as the coverage falls, each
        /// channel of what is seen goes back toward the undercoat, never
        /// past it and never the other way.
        #[test]
        fn less_coverage_is_less_of_the_film(
            under in any::<[u8; 3]>(),
            paint in any::<[u8; 3]>(),
            blend in 0usize..3,
            one in any::<u8>(),
            other in any::<u8>(),
            crease in -1.0_f32..=1.0,
        ) {
            let blend = FILMS[blend];
            let (less, more) = (one.min(other), one.max(other));
            let thin = filmed(under, paint, blend, less, crease);
            let thick = filmed(under, paint, blend, more, crease);
            for channel in 0..3 {
                let u = i32::from(under[channel]);
                let (t, k) = (i32::from(thin[channel]), i32::from(thick[channel]));
                // `thin` lies between the undercoat and `thick`, to a level.
                prop_assert!(
                    t >= u.min(k) - 1 && t <= u.max(k) + 1,
                    "{:?}: {} of it is {:?} and {} is {:?} over {:?}",
                    blend, less, thin, more, thick, under
                );
            }
        }

        /// A contrast paint over a lighter undercoat comes out lighter than
        /// over a darker one: it takes its value from what is under it.
        #[test]
        fn a_contrast_is_lighter_over_a_lighter_undercoat(
            colour in rgba(), dark in 0u8..120, light in 136u8..=255, crease in -1.0_f32..=1.0,
        ) {
            let colour = [colour[0], colour[1], colour[2], 255];
            let base = meta(Blend::Normal, 1.0);
            let top = meta(Blend::Contrast, 1.0);
            let over = |value: u8| {
                lightness(composite_voxel(
                    [(&base, [value, value, value, 255]), (&top, colour)].into_iter(),
                    crease,
                ))
            };
            prop_assert!(over(light) + 1.0 >= over(dark), "{} under {}", over(light), over(dark));
        }

        /// A wash lands more in a crease than on an edge.
        #[test]
        fn a_wash_pools_in_a_crease(under in 60u8..=255, wash in rgba()) {
            let wash = [wash[0] / 2, wash[1] / 2, wash[2] / 2, 255];
            let base = meta(Blend::Normal, 1.0);
            let top = meta(Blend::Wash, 1.0);
            let at = |crease: f32| lightness(composite_voxel(
                [(&base, [under, under, under, 255]), (&top, wash)].into_iter(), crease));
            prop_assert!(at(1.0) <= at(-1.0) + 1.0, "crease {} edge {}", at(1.0), at(-1.0));
        }

        /// A hidden layer, or one at no opacity, changes nothing.
        #[test]
        fn a_hidden_layer_changes_nothing(under in rgba(), over in rgba(), blend in 0usize..6) {
            let base = meta(Blend::Normal, 1.0);
            let alone = composite_voxel([(&base, under)].into_iter(), 0.0);
            let hidden = LayerMeta { visible: false, ..meta(Blend::ALL[blend], 1.0) };
            let none = meta(Blend::ALL[blend], 0.0);
            // A hidden layer is left out before this is reached; at no
            // opacity it is passed in and must still change nothing.
            let _ = hidden;
            prop_assert_eq!(composite_voxel([(&base, under), (&none, over)].into_iter(), 0.0), alone);
        }
    }

    /// The crease is positive in a crease and negative over a convex form.
    #[test]
    fn a_crease_is_positive_and_an_edge_negative() {
        // A point just outside a ball: every neighbour further out than a
        // straight line through it would put it, so the Laplacian is
        // positive and the crease negative.
        assert!(crease_from(0.0, [1.2, -0.8, 0.2, 0.2, 0.2, 0.2]) < 0.0);
        // The inside of a groove: the neighbours across it are nearer.
        assert!(crease_from(0.0, [-0.2, 0.0, -0.3, -0.3, 0.0, 0.0]) > 0.0);
    }

    /// Adding, reordering, merging and flattening keep the active layer's
    /// paint where the volume can find it.
    #[test]
    fn the_stack_keeps_its_layers_straight() {
        let mut layers = Layers::default();
        let mut active = PaintField::default();
        active.write(glam::IVec3::new(1, 1, 1), [255, 0, 0, 255]);
        assert!(layers.is_plain());
        let second = layers.add("Wash", Blend::Wash, &mut active).expect("room");
        assert_eq!(layers.len(), 2);
        assert_eq!(layers.active_id(), second);
        assert!(active.is_empty(), "the new layer came with paint");
        active.write(glam::IVec3::new(1, 1, 1), [0, 0, 255, 255]);
        layers.swap_active(0, &mut active);
        assert_eq!(active.at(glam::IVec3::new(1, 1, 1)), [255, 0, 0, 255]);
        layers.reorder(1, 0);
        assert_eq!(layers.metas()[0].id, second);
        assert_eq!(layers.active(), 1, "the active layer did not follow its move");
        layers.flatten(&mut active);
        assert!(layers.is_plain());
        // The red one is on top now, normal and opaque: it is what shows.
        assert_eq!(active.at(glam::IVec3::new(1, 1, 1)), [255, 0, 0, 255]);
    }
}
