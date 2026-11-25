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
//! and not baked into the layer.** A wash darkens what is under it, more in
//! a crease; a contrast paint tints by how light what is under it is. Put
//! together at the moment of drawing, a change to a lower layer changes
//! the look of every layer over it, as it does on a real model.
//!
//! # The arithmetic
//!
//! The shader lays paint over the surface as `surface * (1 - a) + p`, with
//! `p` the colour already multiplied by its coverage `a` (`sculpt.wgsl`).
//! The stack is put together in the same terms, bottom to top, starting
//! from nothing (`p = 0`, `a = 0`). For each shown layer with paint at a
//! voxel, its strength there is `k = coverage * opacity * pooling`, and
//! what is visible under it is `under = CLAY * (1 - a) + p`. The blend
//! gives a target colour from the layer's colour and `under`, and:
//!
//! ```text
//! p' = (1 - k) p + k target
//! a' = a + k (1 - a)
//! ```
//!
//! which is exactly "the visible colour moves toward the target by `k`"
//! with the clay showing through by `1 - a'`, whatever the blend. The clay
//! here is the export's ([`crate::export::texture`]), and the arithmetic is
//! done on sRGB values as the paint field stores them: the shader lights
//! the result as it lights any paint.

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
    /// A tint over everything that keeps what is under it showing: the
    /// colour's hue at what is under it's lightness.
    Glaze,
    /// Multiply, thicker in a crease and thinner on an edge.
    Wash,
    /// Tints by how light what is under it is: full over light, nearly gone
    /// over dark; pools in creases and thins on edges.
    Contrast,
}

impl std::fmt::Display for Blend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

impl Blend {
    pub const ALL: [Blend; 6] =
        [Blend::Normal, Blend::Multiply, Blend::Screen, Blend::Glaze, Blend::Wash, Blend::Contrast];

    pub fn label(self) -> &'static str {
        match self {
            Blend::Normal => "Normal",
            Blend::Multiply => "Multiply",
            Blend::Screen => "Screen",
            Blend::Glaze => "Glaze",
            Blend::Wash => "Wash",
            Blend::Contrast => "Contrast",
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
        matches!(self, Blend::Wash | Blend::Contrast)
    }

    /// How much of the layer lands at a point, as a share, by how concave
    /// the surface is there (`crease` in -1..=1: positive in a crease,
    /// negative on an edge).
    #[debug_ensures((0.0..=1.0).contains(&ret))]
    fn pooling(self, crease: f32) -> f32 {
        let into = crease.max(0.0);
        let off = (-crease).max(0.0);
        match self {
            Blend::Wash => (0.35 + 0.9 * into - 0.3 * off).clamp(0.0, 1.0),
            Blend::Contrast => (0.75 + 0.4 * into - 0.45 * off).clamp(0.0, 1.0),
            _ => 1.0,
        }
    }

    /// The colour this blend moves toward, from the layer's colour and what
    /// is visible under it, both 0..=1.
    fn target(self, colour: [f32; 3], under: [f32; 3]) -> [f32; 3] {
        let each = |f: &dyn Fn(f32, f32) -> f32| {
            [f(colour[0], under[0]), f(colour[1], under[1]), f(colour[2], under[2])]
        };
        match self {
            Blend::Normal => colour,
            Blend::Multiply | Blend::Wash => each(&|c, u| c * u),
            Blend::Screen => each(&|c, u| 1.0 - (1.0 - c) * (1.0 - u)),
            Blend::Glaze => {
                let tint = luma(colour).max(0.05);
                let light = luma(under);
                each(&|c, _| (c / tint * light).min(1.0))
            }
            Blend::Contrast => {
                // Dark under it keeps a little of the colour, light under it
                // all of it: what a translucent paint over a grey value
                // sketch does.
                let value = 0.12 + 0.88 * luma(under);
                each(&|c, _| c * value)
            }
        }
    }
}

/// The lightness of an sRGB colour, 0..=1, by the Rec. 709 weights.
fn luma(colour: [f32; 3]) -> f32 {
    0.2126 * colour[0] + 0.7152 * colour[1] + 0.0722 * colour[2]
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
        }
    }

    /// Whether a layer lying like this over nothing is its paint as it is.
    fn is_plain(&self) -> bool {
        self.blend == Blend::Normal && self.opacity >= 1.0 && self.visible
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
}

/// The paint of a whole stack, with what each layer is: what an undo entry
/// holds of a change to the stack, and what the project file writes.
#[derive(Debug, Clone)]
pub struct LayerState {
    pub metas: Vec<LayerMeta>,
    pub fields: Vec<PaintField>,
    pub active: usize,
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
        out
    }

    /// Put one brick of the composite together from the stack.
    ///
    /// `crease` gives how concave the surface is at a voxel of the brick
    /// (-1..=1), and is asked only where a blend reads the shape.
    pub(crate) fn put_together(
        &mut self,
        coord: BrickCoord,
        active_paint: &PaintField,
        crease: impl Fn(usize) -> f32,
    ) {
        if self.is_plain() {
            return;
        }
        let metas = self.metas();
        let layers: Vec<(&LayerMeta, &PaintBrick)> = metas
            .iter()
            .enumerate()
            .filter(|(_, meta)| meta.visible && meta.opacity > 0.0)
            .filter_map(|(index, meta)| {
                let field = if index == self.active { active_paint } else { &self.fields[index] };
                field.brick(coord).map(|brick| (meta, brick))
            })
            .collect();
        if layers.is_empty() {
            self.composite.restore_brick(coord, None);
            return;
        }
        let shaped = layers.iter().any(|(meta, _)| meta.blend.reads_the_shape());
        let mut out = PaintBrick::dense_filled(BARE);
        let data = out.make_dense();
        let mut any = false;
        for (voxel, slot) in data.iter_mut().enumerate() {
            let crease = if shaped { crease(voxel) } else { 0.0 };
            let rgba = composite_voxel(
                layers.iter().map(|(meta, brick)| (*meta, brick.rgba_at(voxel))),
                crease,
            );
            any |= rgba[3] != 0;
            *slot = rgba;
        }
        self.composite.restore_brick(coord, any.then_some(out));
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
        *active_paint = flat;
        self.metas.clear();
        self.fields.clear();
        self.recorders.clear();
        self.active = 0;
        self.composite = PaintField::default();
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
        if rgba[3] == 0 {
            continue;
        }
        let k =
            f32::from(rgba[3]) / 255.0 * meta.opacity.clamp(0.0, 1.0) * meta.blend.pooling(crease);
        if k <= 0.0 {
            continue;
        }
        let colour =
            [f32::from(rgba[0]) / 255.0, f32::from(rgba[1]) / 255.0, f32::from(rgba[2]) / 255.0];
        let under =
            [clay[0] * (1.0 - a) + p[0], clay[1] * (1.0 - a) + p[1], clay[2] * (1.0 - a) + p[2]];
        let target = meta.blend.target(colour, under);
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

    fn lightness(rgba: Rgba) -> f32 {
        let clay = CLAY.map(f32::from);
        let a = f32::from(rgba[3]) / 255.0;
        let visible = |c: usize| f32::from(rgba[c]) * a + clay[c] * (1.0 - a);
        luma([visible(0), visible(1), visible(2)])
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

        /// A multiply or a wash never makes what is under it lighter, and a
        /// screen never darker.
        #[test]
        fn multiply_darkens_and_screen_lightens(
            under in rgba(), over in rgba(), opacity in 0.0_f32..=1.0, crease in -1.0_f32..=1.0,
        ) {
            let base = meta(Blend::Normal, 1.0);
            let alone = lightness(composite_voxel([(&base, under)].into_iter(), crease));
            for (blend, darker) in [(Blend::Multiply, true), (Blend::Wash, true), (Blend::Screen, false)] {
                let top = meta(blend, opacity);
                let both = lightness(composite_voxel([(&base, under), (&top, over)].into_iter(), crease));
                if darker {
                    prop_assert!(both <= alone + 1.0, "{:?} lightened {} to {}", blend, alone, both);
                } else {
                    prop_assert!(both + 1.0 >= alone, "{:?} darkened {} to {}", blend, alone, both);
                }
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
