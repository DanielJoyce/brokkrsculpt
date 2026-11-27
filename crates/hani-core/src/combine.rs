// SPDX-License-Identifier: AGPL-3.0-only

//! Boolean groups: a folder whose children are combined, top to bottom, into
//! one solid. `docs/BOOLEANS.md` is the design.
//!
//! A folder that **combines** reads its shown children as operands: the
//! first is the base, and each after it is added to, cut from or kept with
//! the result so far by its own [`BoolOp`]. A child that is a folder stands
//! for its own shape: a combining folder for its result, a plain one for the
//! union of what it holds. Hidden children are left out, which is how a cut
//! is tried with and without.
//!
//! Every body shares one lattice, so each operation is a pointwise `min` or
//! `max` of two fields, and a brick of a result depends on that brick of
//! each operand and on nothing else. The results are kept here, built whole
//! when the arrangement changes (which rows, in what order, shown or not,
//! with what operation) and a brick at a time when an operand's bricks do.
//! They are not saved: the arrangement is, in a section of its own
//! (`project.rs`), and a result is built again from it on load.
//!
//! What `body.rs` holds for this: one field on [`Document`], its value in
//! the two constructors, a line at the end of [`Document::take_dirty`], and
//! a fallback in the mesher's lookup so that a result is meshed as a body
//! is. `undo.rs` has one change, [`Change::Boolean`].

use std::hash::{Hash, Hasher};

use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHasher};

use crate::body::{Document, Node, NodeId};
use crate::brick::{BRICK_VOXELS, Brick, BrickCoord, INSIDE, OUTSIDE};
use crate::undo::Change;
use crate::volume::Volume;

/// The tag of the file's section that holds the boolean groups.
pub const TAG: &[u8; 8] = b"HANBOOLS";

/// The most a section can hold: a count and five bytes a row.
pub const MOST_SECTION_BYTES: u64 = 4 + 5 * crate::body::MAX_NODES as u64;

/// The section for a document: for each row whose state is not the default,
/// its place in the table (u32 LE) and one byte, bit 0 set where it
/// combines and bits 1 and 2 its operation. `None` where no row has a
/// state, so that a document without boolean groups gains no byte.
pub(crate) fn encode(doc: &Document) -> Option<Vec<u8>> {
    let rows: Vec<(u32, u8)> = doc
        .nodes()
        .iter()
        .enumerate()
        .filter_map(|(index, node)| {
            let state = doc.bool_state(node.id);
            (state != BoolState::default())
                .then(|| (index as u32, u8::from(state.combines) | (state.op.code() << 1)))
        })
        .collect();
    if rows.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(4 + rows.len() * 5);
    out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    for (index, flags) in rows {
        out.extend_from_slice(&index.to_le_bytes());
        out.push(flags);
    }
    Some(out)
}

/// The section read back, for a table of `rows` rows: each row's place and
/// state. Forgiving, as the other tagged sections are: a row out of range or
/// a byte that names no operation is passed over, and a section cut short
/// gives what it holds.
pub(crate) fn decode(bytes: &[u8], rows: usize) -> Vec<(usize, BoolState)> {
    let Some((count, mut rest)) = bytes.split_first_chunk::<4>() else {
        return Vec::new();
    };
    let count = u32::from_le_bytes(*count) as usize;
    let mut out = Vec::new();
    for _ in 0..count.min(crate::body::MAX_NODES) {
        let Some((index, after)) = rest.split_first_chunk::<4>() else {
            break;
        };
        let Some((flags, after)) = after.split_first() else {
            break;
        };
        rest = after;
        let index = u32::from_le_bytes(*index) as usize;
        let Some(op) = BoolOp::from_code((flags >> 1) & 0b11) else {
            continue;
        };
        if index < rows && flags >> 3 == 0 {
            out.push((index, BoolState { combines: flags & 1 == 1, op }));
        }
    }
    out
}

/// How a row joins the result of the rows above it, in a combining folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BoolOp {
    /// Union: the row's solid is added.
    #[default]
    Add,
    /// Difference: the row's solid is taken away.
    Cut,
    /// Intersection: only what is also inside the row is kept.
    Keep,
}

impl BoolOp {
    pub const ALL: [BoolOp; 3] = [BoolOp::Add, BoolOp::Cut, BoolOp::Keep];

    pub fn label(self) -> &'static str {
        match self {
            BoolOp::Add => "Add",
            BoolOp::Cut => "Cut",
            BoolOp::Keep => "Keep",
        }
    }

    /// The sign the list and the formula show.
    pub fn symbol(self) -> &'static str {
        match self {
            BoolOp::Add => "+",
            BoolOp::Cut => "\u{2212}",
            BoolOp::Keep => "\u{2229}",
        }
    }

    /// The one after this, for a chip pressed in turn.
    pub fn next(self) -> Self {
        match self {
            BoolOp::Add => BoolOp::Cut,
            BoolOp::Cut => BoolOp::Keep,
            BoolOp::Keep => BoolOp::Add,
        }
    }

    /// The result so far and an operand, to the new result, at one voxel.
    #[inline]
    pub fn apply(self, so_far: f32, operand: f32) -> f32 {
        match self {
            BoolOp::Add => so_far.min(operand),
            BoolOp::Cut => so_far.max(-operand),
            BoolOp::Keep => so_far.max(operand),
        }
    }

    pub(crate) fn code(self) -> u8 {
        match self {
            BoolOp::Add => 0,
            BoolOp::Cut => 1,
            BoolOp::Keep => 2,
        }
    }

    pub(crate) fn from_code(code: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|op| op.code() == code)
    }
}

/// One row's boolean state: whether it combines its children (a folder
/// only), and how it joins the rows above it in a combining parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct BoolState {
    pub combines: bool,
    pub op: BoolOp,
}

/// The document's boolean arrangement and the results built from it.
#[derive(Default)]
pub struct Booleans {
    /// Only rows whose state is not the default.
    states: FxHashMap<NodeId, BoolState>,
    /// One per combining folder, built from the arrangement.
    results: FxHashMap<NodeId, Volume>,
    /// The arrangement the results were built for.
    built_for: Option<u64>,
}

/// A brick of a shape being worked out: one value, or every voxel's.
enum Tile {
    Uniform(f32),
    Dense(Box<[f32; BRICK_VOXELS]>),
}

impl Tile {
    fn of(brick: Option<&Brick>) -> TileView<'_> {
        match brick {
            None => TileView::Uniform(OUTSIDE),
            Some(Brick::Uniform(value)) => TileView::Uniform(*value),
            Some(Brick::Dense(data)) => TileView::Dense(data),
        }
    }

    fn view(&self) -> TileView<'_> {
        match self {
            Tile::Uniform(value) => TileView::Uniform(*value),
            Tile::Dense(data) => TileView::Dense(data),
        }
    }

    /// `self = op(self, operand)`, voxel by voxel.
    fn apply(&mut self, op: BoolOp, operand: TileView<'_>) {
        match (&mut *self, operand) {
            (Tile::Uniform(held), TileView::Uniform(value)) => *held = op.apply(*held, value),
            (Tile::Uniform(held), TileView::Dense(data)) => {
                let held = *held;
                let mut out = Box::new([0.0; BRICK_VOXELS]);
                for (out, value) in out.iter_mut().zip(data.iter()) {
                    *out = op.apply(held, *value);
                }
                *self = Tile::Dense(out);
            }
            (Tile::Dense(held), TileView::Uniform(value)) => {
                for held in held.iter_mut() {
                    *held = op.apply(*held, value);
                }
            }
            (Tile::Dense(held), TileView::Dense(data)) => {
                for (held, value) in held.iter_mut().zip(data.iter()) {
                    *held = op.apply(*held, *value);
                }
            }
        }
        self.settle();
    }

    /// A dense tile that holds one saturated value throughout is that value.
    fn settle(&mut self) {
        if let Tile::Dense(data) = self {
            let first = data[0];
            if (first >= OUTSIDE || first <= INSIDE) && data.iter().all(|value| *value == first) {
                *self = Tile::Uniform(first);
            }
        }
    }

    /// As a brick, or nothing where it is all outside.
    fn into_brick(self) -> Option<Brick> {
        match self {
            Tile::Uniform(value) if value >= OUTSIDE => None,
            Tile::Uniform(value) => Some(Brick::Uniform(value)),
            Tile::Dense(data) => Some(Brick::Dense(data)),
        }
    }
}

#[derive(Clone, Copy)]
enum TileView<'a> {
    Uniform(f32),
    Dense(&'a [f32; BRICK_VOXELS]),
}

/// Why a boolean group was not baked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BakeRefusal {
    /// The row is not a folder that combines.
    NotBoolean,
    /// A row inside it is hidden, and would be lost or left out.
    Hidden,
    /// There is nothing in it.
    Empty,
}

impl Document {
    /// A row's boolean state; the default for a row that has none.
    pub fn bool_state(&self, id: NodeId) -> BoolState {
        self.booleans.states.get(&id).copied().unwrap_or_default()
    }

    /// Whether a row is a folder that combines its children.
    pub fn combines(&self, id: NodeId) -> bool {
        self.bool_state(id).combines && self.node(id).is_some_and(|node| !node.is_body())
    }

    /// Set a row's boolean state, and the change that puts it back; `None`
    /// when nothing changes, the row is not here, or a body is asked to
    /// combine.
    pub fn set_bool_state(&mut self, id: NodeId, state: BoolState) -> Option<Change> {
        let node = self.node(id)?;
        if state.combines && node.is_body() {
            return None;
        }
        let before = self.bool_state(id);
        if before == state {
            return None;
        }
        self.put_bool_state(id, state);
        Some(Change::Boolean { id, state: before })
    }

    /// Put a state in place and hand back the one it replaces. The one
    /// writer: a default state is not kept, so the map holds only what the
    /// file has to say.
    pub(crate) fn put_bool_state(&mut self, id: NodeId, state: BoolState) -> BoolState {
        let before = if state == BoolState::default() {
            self.booleans.states.remove(&id)
        } else {
            self.booleans.states.insert(id, state)
        };
        before.unwrap_or_default()
    }

    /// The combining folder a row is an operand of: its parent, when that
    /// combines.
    pub fn combining_parent(&self, id: NodeId) -> Option<NodeId> {
        self.parent_of(id).filter(|parent| self.combines(*parent))
    }

    /// Every combining folder a row is inside, the nearest first.
    pub fn combining_ancestors(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut at = self.parent_of(id);
        while let Some(folder) = at {
            if self.combines(folder) {
                out.push(folder);
            }
            at = self.parent_of(folder);
        }
        out
    }

    /// Whether a row is drawn only through a combining folder's result.
    pub fn inside_boolean(&self, id: NodeId) -> bool {
        !self.combining_ancestors(id).is_empty()
    }

    /// A folder's direct children, in order.
    pub fn children_of(&self, folder: NodeId) -> Vec<NodeId> {
        let Some(at) = self.index_of(folder) else {
            return Vec::new();
        };
        let depth = self.nodes()[at].depth();
        self.nodes()[at + 1..]
            .iter()
            .take_while(|node| node.depth() > depth)
            .filter(|node| node.depth() == depth + 1)
            .map(|node| node.id)
            .collect()
    }

    /// A combining folder's operands, in order, each with the operation it
    /// joins by: the shown children, the first of them the base, whose
    /// operation is [`BoolOp::Add`] whatever its row says.
    pub fn operands(&self, folder: NodeId) -> Vec<(NodeId, BoolOp)> {
        let mut out: Vec<(NodeId, BoolOp)> = self
            .children_of(folder)
            .into_iter()
            .filter(|id| self.node(*id).is_some_and(|node| node.visible))
            .map(|id| (id, self.bool_state(id).op))
            .collect();
        if let Some(first) = out.first_mut() {
            first.1 = BoolOp::Add;
        }
        out
    }

    /// A combining folder's result, as last built.
    pub fn boolean_result(&self, folder: NodeId) -> Option<&Volume> {
        self.booleans.results.get(&folder)
    }

    /// The fold written out by names with every bracket, as
    /// `((Body 1 − Hole) + Boss)`; a folder among the operands in brackets
    /// of its own.
    pub fn boolean_formula(&self, folder: NodeId) -> Option<String> {
        if !self.combines(folder) {
            return None;
        }
        Some(self.formula_of(folder))
    }

    fn formula_of(&self, id: NodeId) -> String {
        let Some(node) = self.node(id) else {
            return String::new();
        };
        if node.is_body() {
            return node.name.clone();
        }
        if !self.combines(id) {
            let names: Vec<String> = self
                .children_of(id)
                .into_iter()
                .filter(|child| self.node(*child).is_some_and(|node| node.visible))
                .map(|child| self.formula_of(child))
                .collect();
            return format!("({})", names.join(" + "));
        }
        let mut out = String::new();
        for (index, (operand, op)) in self.operands(id).into_iter().enumerate() {
            let named = self.formula_of(operand);
            out = if index == 0 { named } else { format!("({out} {} {named})", op.symbol()) };
        }
        out
    }

    /// The arrangement the results depend on: the rows, their places and
    /// eyes, and their boolean states.
    fn boolean_signature(&self) -> u64 {
        let mut hasher = FxHasher::default();
        for node in self.nodes() {
            (node.id, node.depth(), node.is_body(), node.visible).hash(&mut hasher);
            self.bool_state(node.id).hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Whether the arrangement has changed since the results were built, so
    /// that the next [`Document::take_dirty`] builds them again.
    pub fn booleans_stale(&self) -> bool {
        self.booleans.built_for != Some(self.boolean_signature())
    }

    /// One operand's shape at a brick, joined into `tile` by `op`.
    fn join_operand(&self, id: NodeId, coord: BrickCoord, op: BoolOp, tile: &mut Tile) {
        let Some(node) = self.node(id) else {
            return;
        };
        if let Some(volume) = node.volume() {
            tile.apply(op, Tile::of(volume.brick(coord)));
        } else if self.combines(id) {
            let brick = self.booleans.results.get(&id).and_then(|result| result.brick(coord));
            tile.apply(op, Tile::of(brick));
        } else {
            let mut plain = Tile::Uniform(OUTSIDE);
            for child in self.children_of(id) {
                if self.node(child).is_some_and(|node| node.visible) {
                    self.join_operand(child, coord, BoolOp::Add, &mut plain);
                }
            }
            tile.apply(op, plain.view());
        }
    }

    /// A combining folder's result at one brick, from its operands as they
    /// stand (and from its nested folders' results as last built).
    fn fold_brick(&self, folder: NodeId, coord: BrickCoord) -> Tile {
        let mut tile = Tile::Uniform(OUTSIDE);
        for (operand, op) in self.operands(folder) {
            self.join_operand(operand, coord, op, &mut tile);
        }
        tile
    }

    /// Every brick any body inside a folder holds.
    fn coords_under(&self, folder: NodeId) -> Vec<BrickCoord> {
        let Some(range) = self.subtree_of(folder) else {
            return Vec::new();
        };
        let mut coords: Vec<BrickCoord> = self.nodes()[range]
            .iter()
            .filter_map(Node::volume)
            .flat_map(|volume| volume.brick_coords())
            .collect();
        coords.sort_unstable_by_key(|coord| (coord.0.x, coord.0.y, coord.0.z));
        coords.dedup();
        coords
    }

    /// Bring the results up to date with the document, and add each brick of
    /// a result that changed to `dirty`, for the mesher. Called with the
    /// bricks of the bodies that changed, from [`Document::take_dirty`].
    pub(crate) fn refresh_booleans(&mut self, dirty: &mut Vec<(NodeId, BrickCoord)>) {
        let signature = self.boolean_signature();
        if self.booleans.built_for != Some(signature) {
            self.rebuild_booleans(dirty);
            self.booleans.built_for = Some(signature);
            return;
        }
        if self.booleans.results.is_empty() || dirty.is_empty() {
            return;
        }
        // Each changed brick, in each combining folder it is inside: the
        // deepest folders first, so that a folder reads its nested ones'
        // results already brought up to date.
        let mut work: Vec<(u8, NodeId, BrickCoord)> = Vec::new();
        let mut seen_body = None;
        let mut ancestors = Vec::new();
        for (body, coord) in dirty.iter() {
            if seen_body != Some(*body) {
                seen_body = Some(*body);
                ancestors = self
                    .combining_ancestors(*body)
                    .into_iter()
                    .map(|folder| (self.node(folder).map_or(0, Node::depth), folder))
                    .collect();
            }
            for (depth, folder) in &ancestors {
                work.push((*depth, *folder, *coord));
            }
        }
        if work.is_empty() {
            return;
        }
        work.sort_unstable_by_key(|(depth, folder, coord)| {
            (u8::MAX - depth, folder.0, coord.0.x, coord.0.y, coord.0.z)
        });
        work.dedup();
        for (_, folder, coord) in work {
            let tile = self.fold_brick(folder, coord);
            if let Some(result) = self.booleans.results.get_mut(&folder) {
                match tile.into_brick() {
                    Some(brick) => result.insert_brick(coord, brick),
                    None => drop(result.take_brick(coord)),
                }
            }
            dirty.push((folder, coord));
        }
    }

    /// Build every result again from nothing: after the arrangement changed.
    fn rebuild_booleans(&mut self, dirty: &mut Vec<(NodeId, BrickCoord)>) {
        let old = std::mem::take(&mut self.booleans.results);
        // The deepest folders first, for the same reason as above.
        let mut folders: Vec<(u8, NodeId)> = self
            .nodes()
            .iter()
            .filter(|node| !node.is_body() && self.combines(node.id))
            .map(|node| (node.depth(), node.id))
            .collect();
        folders.sort_unstable_by_key(|(depth, id)| (u8::MAX - depth, id.0));
        for (_, folder) in folders {
            let coords = self.coords_under(folder);
            let tiles: Vec<(BrickCoord, Option<Brick>)> = coords
                .par_iter()
                .map(|coord| (*coord, self.fold_brick(folder, *coord).into_brick()))
                .collect();
            let mut result = Volume::new(self.voxel_size());
            for (coord, brick) in tiles {
                if let Some(brick) = brick {
                    result.insert_brick(coord, brick);
                }
                dirty.push((folder, coord));
            }
            // What the folder showed before and does not now, cleared.
            if let Some(before) = old.get(&folder) {
                for coord in before.brick_coords() {
                    if result.brick(coord).is_none() {
                        dirty.push((folder, coord));
                    }
                }
            }
            self.booleans.results.insert(folder, result);
        }
        // A folder that combined and no longer does: what it showed, cleared.
        for (folder, before) in &old {
            if !self.booleans.results.contains_key(folder) {
                dirty.extend(before.brick_coords().map(|coord| (*folder, coord)));
            }
        }
    }

    /// Collapse a combining folder into one body holding its result, in the
    /// folder's place and under its name. The changes one undo puts back.
    ///
    /// Nested folders are collapsed first, a plain one as the union of what
    /// it holds. The base keeps its paint and its mask; what an added row
    /// brings is carried as a merge carries it; a cut face is bare.
    pub fn bake_boolean(&mut self, folder: NodeId) -> Result<Vec<Change>, BakeRefusal> {
        if !self.combines(folder) {
            return Err(BakeRefusal::NotBoolean);
        }
        let range = self.subtree_of(folder).ok_or(BakeRefusal::NotBoolean)?;
        if self.nodes()[range.clone()].iter().skip(1).any(|node| !node.visible) {
            return Err(BakeRefusal::Hidden);
        }
        if !self.nodes()[range].iter().any(Node::is_body) {
            return Err(BakeRefusal::Empty);
        }
        let mut changes = Vec::new();
        self.collapse(folder, &mut changes);
        Ok(changes)
    }

    /// Collapse one folder into its first body, recursing into folders
    /// among its children first, and record every change.
    fn collapse(&mut self, folder: NodeId, changes: &mut Vec<Change>) -> Option<NodeId> {
        let combines = self.combines(folder);
        for child in self.children_of(folder) {
            if self.node(child).is_some_and(|node| !node.is_body()) {
                // An empty folder is dropped; it adds nothing.
                if self.collapse(child, changes).is_none()
                    && let Some(at) = self.index_of(child)
                {
                    let node = self.remove(at);
                    changes.push(Change::NodeRemoved { at, node: Box::new(node) });
                }
            }
        }
        let children = self.children_of(folder);
        let (&base, rest) = children.split_first()?;
        let ops: Vec<(NodeId, BoolOp)> = rest
            .iter()
            .map(|id| (*id, if combines { self.bool_state(*id).op } else { BoolOp::Add }))
            .collect();

        let mut removed = Vec::new();
        self.volume_mut(base)?.begin_stroke();
        for (operand, op) in ops {
            let at = self.index_of(operand)?;
            let node = self.remove(at);
            if let (Some(from), Some(into)) = (node.volume(), self.volume_mut(base)) {
                match op {
                    BoolOp::Add => drop(into.union_from(from)),
                    BoolOp::Cut | BoolOp::Keep => into.combine_from(from, op),
                }
            }
            removed.push(Change::NodeRemoved { at, node: Box::new(node) });
        }
        if let Some(edit) = self.volume_mut(base)?.end_stroke().filter(|edit| !edit.is_empty()) {
            changes.push(Change::Bricks { body: base, edit });
        }
        changes.extend(removed);

        // The base rises out of the folder, and takes the folder's name.
        let name = self.node(folder)?.name.clone();
        changes.extend(self.ungroup(folder)?);
        let before = self.outline();
        let mut meta = before.iter().find(|meta| meta.id == base)?.clone();
        meta.name = name;
        self.set_meta(&meta);
        let after = self.outline();
        changes.push(Change::Outline { before, after });
        // The folder's own state goes with it; a row that combined no longer
        // is a row.
        Some(base)
    }
}

impl Volume {
    /// `self = op(self, other)` for a cut or a keep, brick by brick, on one
    /// lattice, recording for undo. The colour and the mask are left as
    /// they are: a cut face is bare.
    pub(crate) fn combine_from(&mut self, other: &Volume, op: BoolOp) {
        debug_assert!(self.is_recording(), "an unbracketed boolean records nothing");
        let coords: Vec<BrickCoord> = match op {
            // Where the cutter has no brick it is all outside, and cuts
            // nothing.
            BoolOp::Cut => other.brick_coords().collect(),
            // Where the keeper has no brick it is all outside, and keeps
            // nothing of this brick.
            BoolOp::Keep => self.brick_coords().collect(),
            BoolOp::Add => return,
        };
        for coord in coords {
            let mut tile = match self.brick(coord) {
                None => Tile::Uniform(OUTSIDE),
                Some(Brick::Uniform(value)) => Tile::Uniform(*value),
                Some(Brick::Dense(data)) => Tile::Dense(data.clone()),
            };
            let before = tile.view_value();
            tile.apply(op, Tile::of(other.brick(coord)));
            if before.is_some() && before == tile.view_value() {
                continue;
            }
            self.record_for_undo(coord);
            match tile.into_brick() {
                Some(brick) => self.insert_brick(coord, brick),
                None => drop(self.take_brick(coord)),
            }
            self.mark_dirty_voxel_range(coord.origin(), coord.max_voxel());
        }
    }
}

impl Tile {
    /// The one value of a uniform tile.
    fn view_value(&self) -> Option<f32> {
        match self {
            Tile::Uniform(value) => Some(*value),
            Tile::Dense(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use glam::{IVec3, Vec3};

    use super::*;
    use crate::brick::{INSIDE, OUTSIDE};
    use crate::undo::{Entry, History};

    const VOXEL: f32 = 0.5;

    /// A ball of radius `radius` about `centre`, on the document's lattice.
    fn ball(centre: Vec3, radius: f32) -> Volume {
        let mut volume = Volume::new(VOXEL);
        let reach = Vec3::splat(radius + 2.0);
        volume.edit_box(centre - reach, centre + reach, move |position, _| {
            ((position.distance(centre) - radius) / VOXEL).clamp(INSIDE, OUTSIDE)
        });
        volume
    }

    fn inside(volume: &Volume, point: Vec3) -> bool {
        volume.sample_world(point) < 0.0
    }

    /// A document with the three balls named A, B and C, A at the origin, B
    /// beside it overlapping, C over the place where they overlap. Each a
    /// top-level body; the first body of `Document::new` is A.
    fn three() -> (Document, [NodeId; 3]) {
        let mut doc = Document::from_volume(ball(Vec3::ZERO, 4.0));
        let a = doc.active();
        let b = doc.insert_body(1, 0, "B", ball(Vec3::new(5.0, 0.0, 0.0), 4.0));
        let c = doc.insert_body(2, 0, "C", ball(Vec3::new(2.5, 0.0, 0.0), 2.0));
        (doc, [a, b, c])
    }

    /// Put the three into one folder, combining, and refresh.
    fn combined(doc: &mut Document, [a, b, c]: [NodeId; 3]) -> NodeId {
        let (folder, _) = doc.group(a, "Group").expect("room for a folder");
        doc.move_to_folder(b, Some(folder)).expect("moved");
        doc.move_to_folder(c, Some(folder)).expect("moved");
        doc.set_bool_state(folder, BoolState { combines: true, op: BoolOp::Add });
        let mut dirty = Vec::new();
        doc.take_dirty(&mut dirty);
        folder
    }

    fn refresh(doc: &mut Document) {
        let mut dirty = Vec::new();
        doc.take_dirty(&mut dirty);
    }

    /// The order is the order applied: A + B − C cuts C out of both, and
    /// with C moved above B (A − C + B) B fills back what C cut from it.
    #[test]
    fn the_order_of_the_rows_is_the_order_applied() {
        let (mut doc, [a, b, c]) = three();
        let folder = combined(&mut doc, [a, b, c]);
        doc.set_bool_state(c, BoolState { combines: false, op: BoolOp::Cut });
        refresh(&mut doc);
        let result = doc.boolean_result(folder).expect("a result");
        let middle = Vec3::new(2.5, 0.0, 0.0);
        assert!(!inside(result, middle), "C did not cut the middle");
        assert!(inside(result, Vec3::new(-3.0, 0.0, 0.0)), "A is gone");
        assert!(inside(result, Vec3::new(8.0, 0.0, 0.0)), "B is gone");
        assert_eq!(doc.boolean_formula(folder).as_deref(), Some("((Body 1 + B) \u{2212} C)"));

        // C above B: the cut happens to A alone, and B is added after it.
        let changes = doc.move_to_folder(c, Some(folder));
        drop(changes);
        let at_b = doc.index_of(b).expect("B");
        let c_node = doc.remove(doc.index_of(c).expect("C"));
        let depth = doc.nodes()[at_b].depth();
        drop(c_node);
        let c2 = doc.insert_body(at_b, depth, "C", ball(Vec3::new(2.5, 0.0, 0.0), 2.0));
        doc.set_bool_state(c2, BoolState { combines: false, op: BoolOp::Cut });
        refresh(&mut doc);
        let result = doc.boolean_result(folder).expect("a result");
        assert!(inside(result, middle), "B did not fill back the cut it came after");
        assert!(!inside(result, Vec3::new(1.0, 0.0, 0.0)), "the cut in A not left to see");
        assert_eq!(doc.boolean_formula(folder).as_deref(), Some("((Body 1 \u{2212} C) + B)"));
    }

    /// A nested folder is a bracket: A + (B − C) keeps all of A, where
    /// (A + B) − C does not.
    #[test]
    fn a_nested_folder_is_a_bracket() {
        let (mut doc, [a, b, c]) = three();
        let folder = combined(&mut doc, [a, b, c]);
        // B and C into a folder of their own, inside, combining, C cutting.
        let (inner, _) = doc.group(b, "Inner").expect("room");
        doc.move_to_folder(c, Some(inner)).expect("moved");
        doc.set_bool_state(inner, BoolState { combines: true, op: BoolOp::Add });
        doc.set_bool_state(c, BoolState { combines: false, op: BoolOp::Cut });
        refresh(&mut doc);
        assert_eq!(doc.parent_of(inner), Some(folder));
        let result = doc.boolean_result(folder).expect("a result");
        // Where C is but only A is: A is whole.
        assert!(inside(result, Vec3::new(1.5, 0.0, 0.0)), "the cut reached A");
        // Where C and B are and A is not: cut.
        assert!(!inside(result, Vec3::new(4.2, 0.0, 0.0)), "the cut missed B");
        assert_eq!(doc.boolean_formula(folder).as_deref(), Some("(Body 1 + (B \u{2212} C))"));
    }

    /// Every voxel of a result is the fold of its operands' voxels, for any
    /// choice of operations; and a brick changed in an operand is changed in
    /// the result to what building it again gives.
    #[test]
    fn a_result_is_the_fold_of_its_operands_at_every_voxel() {
        for (op_b, op_c) in [
            (BoolOp::Add, BoolOp::Cut),
            (BoolOp::Cut, BoolOp::Keep),
            (BoolOp::Keep, BoolOp::Add),
            (BoolOp::Cut, BoolOp::Cut),
        ] {
            let (mut doc, ids) = three();
            let folder = combined(&mut doc, ids);
            doc.set_bool_state(ids[1], BoolState { combines: false, op: op_b });
            doc.set_bool_state(ids[2], BoolState { combines: false, op: op_c });
            refresh(&mut doc);
            // An operand edited after the result was built: a lump on B.
            doc.volume_mut(ids[1]).expect("B").edit_box(
                Vec3::new(8.0, -1.0, -1.0),
                Vec3::new(10.0, 1.0, 1.0),
                |_, _| INSIDE,
            );
            refresh(&mut doc);
            let result = doc.boolean_result(folder).expect("a result");
            for z in -14..=24 {
                for y in -14..=14 {
                    for x in -14..=24 {
                        let voxel = IVec3::new(x, y, z);
                        let value = |id| doc.volume(id).expect("a body").sample_voxel(voxel);
                        let expected =
                            op_c.apply(op_b.apply(value(ids[0]), value(ids[1])), value(ids[2]));
                        assert_eq!(
                            result.sample_voxel(voxel),
                            expected,
                            "{op_b:?} {op_c:?} at {voxel}"
                        );
                    }
                }
            }
        }
    }

    /// A hidden operand is left out, and shown again it is back.
    #[test]
    fn a_hidden_operand_is_left_out() {
        let (mut doc, ids) = three();
        let folder = combined(&mut doc, ids);
        doc.set_bool_state(ids[2], BoolState { combines: false, op: BoolOp::Cut });
        refresh(&mut doc);
        let middle = Vec3::new(2.5, 0.0, 0.0);
        assert!(!inside(doc.boolean_result(folder).expect("result"), middle));
        let mut meta = doc.outline().into_iter().find(|meta| meta.id == ids[2]).expect("C");
        meta.visible = false;
        doc.set_meta(&meta);
        refresh(&mut doc);
        assert!(inside(doc.boolean_result(folder).expect("result"), middle), "C still cuts");
    }

    /// A state change is one undo away, and the result follows the undo.
    #[test]
    fn a_state_change_is_undone() {
        let (mut doc, ids) = three();
        let folder = combined(&mut doc, ids);
        let mut history = History::new(crate::DEFAULT_HISTORY_BUDGET);
        let change = doc
            .set_bool_state(ids[2], BoolState { combines: false, op: BoolOp::Cut })
            .expect("a change");
        history.push(Entry::new(vec![change]));
        refresh(&mut doc);
        let middle = Vec3::new(2.5, 0.0, 0.0);
        assert!(!inside(doc.boolean_result(folder).expect("result"), middle));
        let visible = vec![true; doc.node_count()];
        let _ = history.undo(&mut doc, &visible);
        refresh(&mut doc);
        assert_eq!(doc.bool_state(ids[2]).op, BoolOp::Add);
        assert!(inside(doc.boolean_result(folder).expect("result"), middle));
        // A folder that stops combining has no result.
        doc.set_bool_state(folder, BoolState::default());
        refresh(&mut doc);
        assert!(doc.boolean_result(folder).is_none());
        // And a body cannot be made to combine.
        assert!(
            doc.set_bool_state(ids[0], BoolState { combines: true, op: BoolOp::Add }).is_none()
        );
    }

    /// Saved and read back, the arrangement is as it was; a document with
    /// none gains no section.
    #[test]
    fn the_arrangement_is_saved() {
        let (mut doc, ids) = three();
        let mut plain = Vec::new();
        crate::project::write(&mut plain, &doc, &crate::project::ProjectState::default())
            .expect("written");
        assert!(!plain.windows(8).any(|window| window == TAG), "a section with nothing in it");
        let folder = combined(&mut doc, ids);
        doc.set_bool_state(ids[2], BoolState { combines: false, op: BoolOp::Keep });
        let mut bytes = Vec::new();
        crate::project::write(&mut bytes, &doc, &crate::project::ProjectState::default())
            .expect("written");
        let (mut read, _) = crate::project::read(&mut bytes.as_slice()).expect("read");
        refresh(&mut read);
        let names = |doc: &Document, id: NodeId| doc.node(id).map(|node| node.name.clone());
        let folder_read = read
            .nodes()
            .iter()
            .find(|node| Some(node.name.clone()) == names(&doc, folder))
            .expect("the folder")
            .id;
        assert!(read.combines(folder_read));
        let c = read.nodes().iter().find(|node| node.name == "C").expect("C").id;
        assert_eq!(read.bool_state(c).op, BoolOp::Keep);
        assert!(read.boolean_result(folder_read).is_some());
    }

    /// Baked, the folder is one body in its place, under its name, with
    /// the result's field; one undo puts it all back.
    #[test]
    fn a_bake_is_the_result_and_is_undone_whole() {
        let (mut doc, ids) = three();
        let folder = combined(&mut doc, ids);
        doc.set_bool_state(ids[2], BoolState { combines: false, op: BoolOp::Cut });
        refresh(&mut doc);
        let rows_before = doc.node_count();
        let expected: Vec<f32> = (-14..=24)
            .flat_map(|x| (-10..=10).map(move |y| IVec3::new(x, y, 0)))
            .map(|voxel| doc.boolean_result(folder).expect("result").sample_voxel(voxel))
            .collect();

        let changes = doc.bake_boolean(folder).expect("baked");
        let mut history = History::new(crate::DEFAULT_HISTORY_BUDGET);
        history.push(Entry::new(changes));
        refresh(&mut doc);
        assert_eq!(doc.node_count(), 1, "the folder and its operands are one body");
        let baked = doc.nodes()[0].id;
        assert_eq!(doc.nodes()[0].name, "Group");
        let got: Vec<f32> = (-14..=24)
            .flat_map(|x| (-10..=10).map(move |y| IVec3::new(x, y, 0)))
            .map(|voxel| doc.volume(baked).expect("body").sample_voxel(voxel))
            .collect();
        assert_eq!(got, expected);

        let visible = vec![true; doc.node_count()];
        let _ = history.undo(&mut doc, &visible);
        refresh(&mut doc);
        assert_eq!(doc.node_count(), rows_before);
        assert!(doc.combines(folder));
        let names: Vec<&str> = doc.nodes().iter().map(|node| node.name.as_str()).collect();
        assert_eq!(names, ["Group", "Body 1", "B", "C"]);
        // A's own field is back: the middle, which C cut, is inside A again.
        assert!(inside(doc.volume(ids[0]).expect("A"), Vec3::new(2.0, 0.0, 0.0)));
    }

    /// A hidden row stops a bake, which would lose it or leave it out.
    #[test]
    fn a_bake_with_a_hidden_row_is_refused() {
        let (mut doc, ids) = three();
        let folder = combined(&mut doc, ids);
        let mut meta = doc.outline().into_iter().find(|meta| meta.id == ids[1]).expect("B");
        meta.visible = false;
        doc.set_meta(&meta);
        assert_eq!(doc.bake_boolean(folder).err(), Some(BakeRefusal::Hidden));
        assert_eq!(doc.bake_boolean(ids[0]).err(), Some(BakeRefusal::NotBoolean));
    }
}
