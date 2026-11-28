// SPDX-License-Identifier: AGPL-3.0-only

//! Application state, input handling and the widget tree.

mod about;
// The graphics adapter, for the About card. See `app/adapter.rs`.
mod adapter;
mod adding;
// The crash net, written without stopping the drawing. See `app/autosave.rs`.
mod autosave;
// Boolean groups in the list and on the model. See `app/booleans.rs`.
mod booleans;
// Radial symmetry. See `app/radial.rs`.
mod radial;
// Symmetry local to the body. See `app/symmetry_frame.rs`.
mod symmetry_frame;
pub(crate) use radial::RadialMessage;
// A shape set down in Sculpt, placed and joined when done. See
// `app/placed.rs`.
mod placed;
pub(crate) use booleans::BooleanMessage;
mod cards;
// The gizmo on a folder. See `app/group_gizmo.rs`.
mod context;
mod group_gizmo;
mod kept_settings;
mod keys;
pub(crate) use kept_settings::KeptMessage;
// The active body's layers of paint. See `app/layers.rs`.
mod layers;
// Racks of paints, and the index they are filled from. See `app/rack.rs`.
mod rack;
// The airbrush, and the Paint mode's brush sizes. See `app/airbrush.rs`.
mod airbrush;
// The one-click coats. See `app/coats.rs`.
mod coats;
// The Decal touch. See `app/decal.rs`.
mod decal;
// The ring cut. See `app/ring_cut.rs`.
mod ring_cut;
// The hierarchy's Explode and Solo active. See `app/explode.rs`.
mod explode;
pub use ring_cut::CutWay;
// How many worker threads, and a measurement of it. See `app/cores.rs`.
pub(crate) mod cores;
pub use cores::ThreadsChoice;
// The card a paint is made or changed in. See `app/designer.rs`.
mod designer;
// What a drybrush would catch, under the ring. See `app/dry_preview.rs`.
mod dry_preview;
// The coarse preview of a shape being dragged out. See `app/insert_preview.rs`.
mod insert_preview;
// A paint shown on a slapchopped ball. See `app/paint_preview.rs`.
mod paint_preview;
// How strongly washes and contrasts pool. See `app/pooling.rs`.
mod pooling;
mod rail;
mod workspace;
pub use adding::Insert;
// Vector displacement maps, for the brushes that add: `app/maps.rs`.
mod maps;
pub use maps::MapName;
// The user's own brush pictures, kept and named by a brush:
// `app/brush_pictures.rs`.
mod brush_pictures;
mod fit_test;
// Twist, bend and taper. See `app/deform.rs`.
mod deform;
// Reference pictures and the floor. See `app/references.rs`.
mod references;
// Save, Save as new and Revert for the brush in hand: `app/keeping.rs`.
mod keeping;
// How big the interface is drawn: `app/ui_scale.rs`.
mod ui_scale;
// Which way up an insert goes on, and how deep: `app/insert_turn.rs`.
mod insert_turn;
pub use references::Taken as ReferenceTaken;
mod form;
// A line drawn beside an axis, spun into a body. See `app/revolve.rs`.
mod revolve;
// Shell and extract. See `app/hollow.rs`.
mod hollow;
// Help: Markdown pages in a card. See `app/help.rs`.
mod help;
mod measure;
mod moving;
mod overhang;
mod paint_carry;
mod painting;
mod paints;
mod panel;
mod shelf;
mod snip;
mod tips;
mod touch;
// A pen's samples, beside the moves they belong to. See `app/pen.rs`.
mod pen;
// What a pen's stroke leaves, measured. See `app/pen_strokes.rs`.
mod pen_strokes;
// Where a frame's time goes. See `app/phases.rs`.
mod phases;
mod presets;
// A stroke kept in a file and played back. See `app/replay.rs`.
mod replay;
mod studio;
mod swatch;
mod workbench;
pub use workbench::Piece;

// The parts of what was one `app.rs`, by what each is about. See issue 189.
mod build_stamp;
pub use build_stamp::{build_commit, build_label, build_number};
mod camera_ops;
mod cutting;
mod diagnostics;
mod document_io;
mod gizmo_ops;
mod input;
mod lifecycle;
mod mask_ops;
mod meshing;
mod outliner;
mod overlay;
mod strokes;
mod working_size;

use std::collections::VecDeque;
use std::sync::Arc;
use web_time::Instant;

use glam::{Vec2, Vec3};
use hani_core::{
    BrickCoord, BrickMesh, Brush, BrushDirection, BrushKind, BrushScratch, Change, Document,
    DropRefusal, DropTarget, Entry, History, HistoryStats, MAX_VOLUME_BYTES, MaskField, MaskFilter,
    MaskOp, MaskRecipe, MirrorAxis, MoveStroke, NodeId, NodeMeta, Stamp, Stroke, Symmetry,
    UndoOutcome, Volume, VolumeStats, lean_normal,
};
use iced::{Subscription, Task};

use crate::camera::OrbitCamera;
use crate::cursor;
use crate::cut;
use crate::cut_preview;
use crate::gizmo;
use crate::message::{
    ConfirmChoice, ExportDetail, ExportFormat, KeySetting, MaskGenerator, Message, PanelSection,
    PointerButton, PointerEvent, SpaceMouseSetting, TopMenu,
};
use crate::navcube;
use crate::spacemouse::{
    Action as PuckAction, AxisBinding, ButtonAction, Config as SpaceMouseConfig, SpaceMouse,
};
use crate::tablet::Tablet;
use crate::viewport::{SharedFrame, ThumbRequest};

/// World units are millimetres, because the output of this program is meant to
/// be printed.
///
/// A 60 mm ball at a quarter millimetre voxel is 240 voxels across, which is
/// the 256 cubed effective volume the milestones are measured against.
pub(crate) const MODEL_RADIUS_MM: f32 = 30.0;
const VOXEL_SIZE_MM: f32 = 0.25;

/// The point every enabled mirror plane passes through: the lattice origin.
///
/// **One constant, so there is one answer.** `Symmetry::mirrors` and
/// `Symmetry::flips` take the centre as a parameter rather than assuming it,
/// and `cursor::build` draws the planes where this says they are, so the
/// mirroring the engine performs and the mirroring the viewport promises come
/// from the same number.
///
/// The value is the origin because **the axis and the centre must have the same
/// scope**. The three axis switches are global -- one setting for the whole
/// document -- and a global switch whose plane came from the selected body
/// would make "X on" a different physical plane depending on which row is
/// highlighted, with the only evidence on screen a translucent patch that
/// shrinks with that body's radius. A single world-fixed mirror on a shared
/// lattice is defensible and it is what the interface has always drawn.
///
/// **Be honest about what that does not fix.** With the centre pinned here, a
/// dent carved into a body sitting at x = +80 still gets its twin at x = -80:
/// free-floating geometry that exports as an extra, unprintable shell. The
/// parameter alone buys nothing today. What stops that happening is
/// [`Hani::mirror_refusal`], which will not let an axis be enabled while the
/// active body sits wholly to one side of it. The real fix is a per-body
/// centre, and it is deferred with the per-body axis it has to move with.
const MIRROR_CENTRE: Vec3 = Vec3::ZERO;

/// Range the brush radius may be nudged to with the keyboard, in millimetres.
/// The same range the slider offers, so the two cannot disagree.
///
/// It was 12 mm, which was too small to grab a form with Move. The new ceiling
/// is measured rather than chosen, and `cargo bench -p hani-core` is what
/// measured it: a brush covers a fixed world radius, so its cost grows with the
/// cube of it, and what a drag has to fit inside is the 16 ms frame -- edit plus
/// remesh, per pointer event.
///
/// 30 mm was tried first, on an estimate that undercounted the remesh, and the
/// bench refused it flatly. 25 mm passed every single stamp row and then failed
/// the **fast drag**, which is the case that decides this: fewer pointer
/// samples means more interpolated stamps per event, and it came to 15.7 ms p95
/// with a 17.8 ms worst against the 16 ms frame. A brush that is fluid until
/// you hurry is not fluid.
///
/// So 20 mm, where all of it passes, and the bench's own sweep tops out at the
/// same place. **Move the two together or neither** -- a slider that goes past
/// what the gate covers is a promise nothing checks.
///
/// The cap is the same for every brush, deliberately. Move is among the
/// cheapest and could afford more, but a per-brush ceiling means the radius
/// jumps when the tool changes, and a number that moves on its own is worse
/// than a number that is merely smaller than you wanted.
pub(crate) const MIN_RADIUS_MM: f32 = 0.25;
pub(crate) const MAX_RADIUS_MM: f32 = 20.0;

/// The most voxels of radius a brush may reach, whatever that is in
/// millimetres.
///
/// **A brush costs what it costs in VOXELS, and the ceiling beside this one is
/// in millimetres, so the two disconnect the moment the voxel size moves.**
/// Measured 2026-08-21, one Draw stamp, with the application running so
/// pessimistic: 80 voxels of radius took 5.01 ms, 96 took 5.22 ms, and 192 took
/// **21.04 ms**. The cost tracks the voxel radius and nothing else -- doubling
/// it costs four times as much, because the work is the surface inside the
/// brush.
///
/// At the 0.25 mm default, 100 voxels is 25 mm, which is above
/// [`MAX_RADIUS_MM`] and so changes nothing at all. At the 0.03125 mm a resin
/// print wants, it is 3.1 mm -- and without it the 20 mm slider would reach 640
/// voxels, which is roughly a quarter of a second per stamp and not a usable
/// tool.
///
/// The budget bench sweeps radius in millimetres at one voxel size, so it has
/// never covered this. Sweeping voxel radius instead is the honest fix and is
/// not done.
pub(crate) const MAX_RADIUS_VOXELS: f32 = 100.0;

/// Range the brush strength may take, matching the slider.
pub(crate) const MIN_STRENGTH: f32 = 0.02;
pub(crate) const MAX_STRENGTH: f32 = 0.80;

/// The most a MASK stroke's strength may be, which is higher than a sculpt
/// brush's ceiling.
///
/// A sculpt brush stops at 0.80 because a stroke is many overlapping stamps and
/// one at full weight is a gouge. A mask stamp blends toward an exact target --
/// fully protected, or fully free -- so reaching it in one stamp is a legal and
/// frequently wanted result: "protect this limb" is a thing a user means
/// literally. It is also the default the mask arrives with, which is
/// [`BrushKind::default_strength`]'s 0.15 for Draw seen from the other end.
pub(crate) const MAX_MASK_STRENGTH: f32 = 1.0;

/// Where the mask's own strength lives in [`Hani::strengths`].
///
/// One past the last brush, because the table is keyed by brush and the mask is
/// not one. See the field.
pub(crate) const MASK_STRENGTH_SLOT: usize = BrushKind::ALL.len();

/// How strongly protection is tinted on the model when the tint is shown.
///
/// Full, because the tint was measured to be readable rather than chosen to be
/// pretty: the matcap's own luminance swing is 102 levels and the shift at full
/// strength puts a masked lit pixel at b - r = +61 against the matcap's own
/// maximum of +17. Anything less starts from inside the clay's own gamut.
/// The slider is there for a user who wants to judge form THROUGH a mask, which
/// is a different job from seeing where one is.
pub(crate) const DEFAULT_MASK_TINT: f32 = 1.0;

/// How strongly creases are darkened and edges lifted when that is shown.
///
/// Full strength in the shader takes a full crease 55% of the way to black,
/// which reads as a drawing of the model and not as the model. This is the
/// strength at which a fold is plainly darker than the matcap alone makes it
/// and a flat is not visibly touched.
pub(crate) const CAVITY_STRENGTH: f32 = 0.7;

/// The feature size that draws as a full crease, in millimetres.
///
/// A size and not a curvature, for the reason the cavity MASK's slider is one:
/// it means the same thing at every voxel size. Narrower features than this
/// saturate; broader ones fade in proportion, so a fold four times as wide is
/// drawn a quarter as dark. One millimetre is about the finest thing a
/// filament printer resolves, which makes "drawn fully dark" and "will be a
/// real crease in the print" roughly the same statement.
pub(crate) const CAVITY_FEATURE_MM: f32 = 1.0;

/// The feature size a cavity mask starts at, in millimetres.
///
/// A millimetre and a half, which is the plan's own worked example and is about
/// the width below which a 0.4 mm nozzle stops resolving detail cleanly.
pub(crate) const DEFAULT_MASK_FEATURE_MM: f32 = 1.5;

/// The narrowest and widest feature the slider can ask for, in millimetres.
///
/// Fixed millimetres rather than a multiple of the voxel, so the slider does not
/// move under a resample. The core clamps the low end to two voxels of whatever
/// body it is asked about, which is the resolution floor and is a different
/// limit from this one.
pub(crate) const MASK_FEATURE_RANGE_MM: std::ops::RangeInclusive<f32> = 0.2..=10.0;

/// The wall thickness a thickness mask starts at, in voxels.
///
/// Three, which is half the band and so the middle of the only range the field
/// can answer at all.
pub(crate) const DEFAULT_MASK_THICKNESS_VOXELS: u32 = 3;

/// How wide a half-space mask's feather is, in voxels.
///
/// Two, which is the narrowest edge that still carries intermediate values on
/// both sides of the plane at every voxel size. A step here would be a fold in
/// the geometry under Move -- see `hani_core::mask`.
pub(crate) const HALFSPACE_FEATHER_VOXELS: f32 = 2.0;

/// Range the voxel size may be resampled to, in millimetres.
///
/// **The lower bound is a resin printing target, not a pool limit.** It used to
/// be 0.06 and justified as the most the mesh pool could hold -- which stopped
/// being the reason the moment `resample` grew a preflight of its own. What
/// fits is now decided per model, against the real reservation on the GPU, and
/// refused with a message naming the size that would work. So this bound exists
/// only to stop the interface offering something absurd.
///
/// 0.03 rather than 0.035, which is roughly what a consumer resin printer
/// resolves, because **the halving ladder from the 0.25 default lands on
/// 0.03125 and never on 0.035**. A floor of 0.035 would leave the finest step
/// permanently one press out of reach. The print size field reaches
/// intermediate sizes continuously; the buttons only halve.
const FINEST_VOXEL_MM: f32 = 0.03;
const COARSEST_VOXEL_MM: f32 = 2.0;

/// Largest angle a fully tilted pen steers the stroke by.
///
/// Tablets report tilt against their own range, so this is applied to the
/// normalised value rather than trusting a device to report degrees. Sixty
/// degrees is about as far as a pen can be leaned while still drawing.
const MAX_TILT: f32 = std::f32::consts::PI / 3.0;

/// Frame intervals kept for the rate readout. At 60 fps this averages over
/// about a second, which is long enough to be steady and short enough to react.
const FRAME_HISTORY: usize = 60;

const CLICK_SLOP_PX: f32 = 4.0;

/// What a held pointer button is currently doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragKind {
    Orbit,
    Pan,
    Sculpt(BrushDirection),
    /// The pointer is resizing the brush, not using it.
    Sizing,
    /// The pointer is dragging the line of a plane cut. Nothing happens to the
    /// model until the button comes back up, so the line can be adjusted.
    Cutting,
    /// The pointer is painting the mask rather than the field.
    ///
    /// **No payload, where [`DragKind::Sculpt`] carries its direction.** That
    /// is deliberate: the mask has three operations rather than two, and one of
    /// them -- blur, on shift -- has to be live within a gesture because shift
    /// is live for Smooth on the field side and one rule is better than two. So
    /// mask mode reads all of its modifiers per event, through
    /// [`Hani::mask_op`], and there is nothing to lock at the press.
    Masking,
    /// The press landed on a body that was not the active one, so it chose that
    /// body and nothing else.
    ///
    /// It is a drag kind rather than nothing at all because the release has to
    /// know what the press meant: a gesture that never opened a recorder must
    /// not be finished as if it had. Dragging after it does nothing, which is
    /// the correct answer -- the body under the cursor changed, and a stroke
    /// the user had not chosen a target for is exactly the surprise this
    /// ordering exists to remove.
    Selecting,
    /// The pointer is dragging a gizmo handle, locked at the press.
    ///
    /// **The handle is a payload for the same reason [`DragKind::Sculpt`]
    /// carries its direction and [`DragKind::Masking`] deliberately does not.**
    /// Which handle is being dragged is decided once, by where the button went
    /// down, and re-picking it per motion event would hand the gesture to a
    /// different axis the moment the pointer wandered off the shaft -- which it
    /// does immediately, because the whole point of an axis drag is to move
    /// away from where you started.
    Transforming(gizmo::Handle),
    /// The press had nothing it was allowed to do: the pointer was over
    /// nothing that is drawn, and the body edits would land on is hidden.
    ///
    /// A drag kind rather than no drag at all for the same reason
    /// [`DragKind::Selecting`] is one -- the release has to know that this
    /// press opened no recorder -- and a kind of its own rather than
    /// `Selecting` because no body was chosen. **Without it the press falls
    /// through to a stroke**, and a stroke whose surface comes from
    /// `pick_body`, which does not consult the eye, carves the invisible body
    /// and marks the document unsaved with nothing on screen changing.
    Refused,
}

/// A drag in progress, tagged with the button that started it so that
/// releasing a different button does not cancel it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Drag {
    button: PointerButton,
    kind: DragKind,
    /// Where the button went down, and when.
    origin: Vec2,
    pressed_at: Instant,
    /// Whether the pointer has travelled far enough to count as a drag rather
    /// than a click. Once true it stays true: a drag that returns to where it
    /// started is still a drag.
    moved: bool,
    /// The world point a camera drag turns about, latched at the press.
    ///
    /// **Latched, and not asked for again per motion event.** The pivot is the
    /// surface under the cursor when the button went down; re-picking it as the
    /// pointer moves would hand the orbit a new centre every frame, which is a
    /// camera that slides away under a steady drag rather than one that turns a
    /// model in your hand. `None` for a drag that is not a camera drag.
    pivot: Option<Vec3>,
}

/// Timings for the debug overlay.
#[derive(Debug, Default)]
struct Perf {
    last_frame: Option<Instant>,
    frame_ms: VecDeque<f32>,
    edit_ms: f32,
    remesh_ms: f32,
    dirty_bricks: usize,
    stamps: usize,
    /// Pressure the last stroke step ran at, so the overlay can show a pen
    /// working without the user having to guess.
    pressure: f32,
    /// Cost of the one off full mesh at load. Kept apart from the per stroke
    /// numbers so a 70 ms load does not sit in the slot that is supposed to
    /// show an 8 ms budget.
    load_ms: f32,
}

impl Perf {
    /// Record a presented frame and return how long it took, in milliseconds.
    ///
    /// The SpaceMouse scales its motion by this, so it reads the same clock
    /// the overlay does rather than keeping a second one that could disagree.
    fn record_frame(&mut self) -> f32 {
        let now = Instant::now();
        let Some(previous) = self.last_frame.replace(now) else {
            return 0.0;
        };
        let elapsed_ms = now.duration_since(previous).as_secs_f32() * 1000.0;
        if self.frame_ms.len() == FRAME_HISTORY {
            self.frame_ms.pop_front();
        }
        self.frame_ms.push_back(elapsed_ms);
        elapsed_ms
    }

    fn average_frame_ms(&self) -> f32 {
        if self.frame_ms.is_empty() {
            return 0.0;
        }
        self.frame_ms.iter().sum::<f32>() / self.frame_ms.len() as f32
    }

    fn worst_frame_ms(&self) -> f32 {
        self.frame_ms.iter().copied().fold(0.0, f32::max)
    }
}

pub struct Hani {
    /// Every body in the sculpt, on one lattice.
    ///
    /// This was a bare `Volume`. The application still holds exactly one body
    /// and nothing in the interface can create a second, but the voxel size,
    /// the dirty set and the meshing all belong to the document from here --
    /// which is what makes "the volume" stop being a thing a call site can mean
    /// by accident.
    doc: Document,
    /// A build worth telling the user about, once the check has answered.
    /// `None` until then, and `None` for ever if there is nothing to say.
    ///
    /// Not necessarily a *newer* build: during a rollback the offer names a
    /// LOWER ordinal, and the wording must not call that an upgrade.
    offer: Option<crate::update::Offer>,
    /// The update preference, read once at startup and written through on every
    /// change. Held rather than re-read so the two tick surfaces cannot
    /// disagree about what is currently set.
    update_settings: crate::update::Settings,
    /// A download is in flight. Guarded rather than merely disabled in the
    /// view, for the reason `signing_in` is: a second press would start a
    /// second 33 MB transfer into the same file.
    downloading_update: bool,
    /// An install is in flight. The Windows path can spend up to a minute
    /// waiting for antivirus to let go of the staged file, so a second press
    /// during that window would start a second swap.
    installing_update: bool,
    /// The last update FAILURE, shown ON the welcome card rather than only in
    /// the status line.
    ///
    /// **The status line is not always readable, and the install button is on
    /// the screen that hides it.** `welcome_card` draws over a full-window
    /// scrim and returns before every other card, so an install started from
    /// the welcome screen reports its outcome into a line covered by the very
    /// card the button is on. That is why the Windows spawn failure has been
    /// reported more than once and its error text has never been read.
    ///
    /// Failures only, and that is the whole scope. A successful install closes
    /// the window, and a successful download changes the button's own label, so
    /// neither has an outcome anyone needs to go looking for. Only a refusal
    /// leaves the user on the same screen with nothing to read.
    ///
    /// Not a general notification system and deliberately not a new layer: the
    /// other install entry point is Help > Install the update and restart,
    /// where `self.welcome` is false and the status line is fully visible. The
    /// hole is exactly one screen wide and this fills exactly that hole.
    update_failure: Option<String>,
    /// Where a verified payload landed, once one has. Only this is installable.
    downloaded_update: Option<std::path::PathBuf>,
    /// The executable to replace, resolved ONCE at startup.
    ///
    /// **An invariant, not a convenience.** After a swap, `current_exe`
    /// resolves through the running inode, which is no longer at the target
    /// name -- Linux reports the old path with ` (deleted)` appended. A second
    /// update in one session that re-resolved would overwrite the rollback copy
    /// and never touch the real binary.
    exe: Option<crate::update::apply::Target>,
    /// A marker left by an update that has not yet proved it starts. Cleared on
    /// the first drawn frame; still present on the next launch means the build
    /// installed here died before drawing anything.
    update_pending_marker: Option<std::path::PathBuf>,
    /// A build to offer going back to, when a crash report and the update
    /// state agree that the build which crashed is the one we installed.
    crash_revert: Option<u64>,
    /// A crash report was found at startup. Set in `with_devices`, which is
    /// where the report is consumed, and read in `new`, which is where `exe`
    /// exists -- the two halves of a decision that cannot be made in one place.
    crashed_last_session: bool,
    camera: OrbitCamera,
    brush: Brush,
    tablet: Tablet,
    spacemouse: SpaceMouse,
    /// Whether stylus pressure scales the brush. Off means every stamp runs at
    /// full strength, which is also what happens when there is no pen.
    pressure_enabled: bool,
    /// Exponent applied to raw pressure. Below 1 makes light touches bite
    /// harder, above 1 gives finer control at the light end.
    pressure_curve: f32,
    /// Whether leaning the pen steers the stroke.
    tilt_enabled: bool,
    stroke: Stroke,
    history: History,
    shared: Arc<SharedFrame>,
    /// Buffers the dirty bricks are meshed into, refilled from the renderer's
    /// recycled ones so a stroke never allocates.
    mesh_buffers: Vec<BrickMesh>,
    brush_scratch: BrushScratch,
    /// The mask stroke under way: the most each voxel has been given, so a
    /// stroke ends in the brush's falloff. See `hani_core::mask_stroke`.
    mask_stroke: hani_core::mask_stroke::MaskStroke,
    /// Strength per brush, so switching tools restores what that tool was last
    /// set to rather than carrying a number that means something different.
    ///
    /// Strength is not the same quantity for every brush -- for Move it is the
    /// fraction of the drag the surface follows -- so one shared slider value
    /// made Move look broken at Draw's default. Indexed by `BrushKind::ALL`,
    /// **plus one slot on the end for the mask**, which is the whole reason the
    /// length is written as an expression.
    ///
    /// The mask's own slot is not a nicety. Draw's default strength is 0.15,
    /// which barely deposits, so a user painting a mask drags the slider to 1.0
    /// to get a usable one -- and the very next Draw stroke after leaving mask
    /// mode would be a gouge at full strength on a scan they were carefully
    /// repairing, with the slider showing exactly what they set and no memory
    /// that it belonged to the other tool.
    strengths: [f32; MASK_STRENGTH_SLOT + 1],
    /// Where a Move gesture grabbed the surface, fixed for the whole drag.
    ///
    /// The drag target is the pointer projected into the view plane through
    /// this point, so the surface follows the cursor across the screen rather
    /// than crawling around the form. See `view_plane_point`.
    move_grab: Option<Vec3>,
    /// The field Move locked when the current gesture began, if the current
    /// gesture is a Move. Kept here rather than rebuilt per event, because
    /// holding it across the whole gesture is the entire point of the brush.
    move_stroke: MoveStroke,
    /// Stamp centres produced by the current pointer event. Reused so a stroke
    /// does not allocate.
    stamp_centres: Vec<Vec3>,
    /// The whole path the pointer has taken during a live cut, in widget
    /// pixels.
    ///
    /// **The whole path and not just its ends**, because the shape is inferred
    /// from what was drawn and the ends of a lasso are the same two points as
    /// the ends of a click.
    ///
    /// It lives here rather than in [`Drag`] because `Drag` is `Copy` and is
    /// read by value at every pointer event; a `Vec` in it would be cloned
    /// several times per motion, which is exactly the allocation a per-event
    /// path most needs to avoid. Cleared at the press and drained at the
    /// release, so it holds nothing between gestures.
    cut_path: Vec<Vec2>,
    /// Bricks waiting to be remeshed, each tagged with the body it belongs to.
    ///
    /// One list across the whole document rather than one per body, because
    /// [`Document::mesh_dirty`] has to make the serial-or-parallel decision
    /// once over the real total -- see its documentation.
    dirty: Vec<(NodeId, BrickCoord)>,
    /// What is drawn, indexed by node position, and the bodies among it that
    /// are not. Both are kept rather than rebuilt because
    /// [`Hani::publish_visibility`] runs on the frame tick.
    shown: Vec<bool>,
    hidden_bodies: Vec<NodeId>,
    /// The bodies whose mask polarity differs from the active body's, which is
    /// the polarity the viewport uniform carries. Kept, and refilled by
    /// [`Hani::publish_mask_view`], for the reason `hidden_bodies` is kept.
    opposite_polarity: Vec<NodeId>,
    /// What the standing mask card says, or `None` when nothing is masked.
    ///
    /// See [`MaskCard`]. Held rather than computed in `view()` because `view()`
    /// runs at display rate and the percentage is a pass over the mask.
    pub(crate) mask_card: Option<MaskCard>,
    /// The one row solo is showing, or `None` for the whole document.
    ///
    /// **A field of the APPLICATION, never of the document.** Solo is the third
    /// input to [`hani_core::resolve_visibility`] and it reaches it as a
    /// parameter, which is what makes it structurally impossible to persist:
    /// `project::write` takes a `&Document` and a `&ProjectState`, so there is
    /// no expression that hands it solo. Do not move this onto `Document` or
    /// `View` to tidy a call site — the guarantee is the type signature, not
    /// the discipline, and `View` is restored by a reopen.
    ///
    /// It is also why leaving solo restores the hand-set eyes bit for bit:
    /// nothing was ever written, so there is nothing to restore. Every shipped
    /// version of the save-a-vector design loses that set — Photoshop's
    /// alt-click eye remembers it only "if you haven't changed anything else",
    /// and Plasticity's own manual says Unisolate makes everything visible
    /// instead.
    solo: Option<NodeId>,
    drag: Option<Drag>,
    /// Last pointer position in widget pixels, for drag deltas.
    cursor: Option<Vec2>,
    viewport_size: Vec2,
    shift: bool,
    control: bool,
    /// Alt, which inverts the brush the same way control does.
    alt: bool,
    perf: Perf,
    /// Where a frame's time goes, phase by phase. See `app/phases.rs`.
    phases: phases::Phases,
    /// A stroke being recorded or played back. See `app/replay.rs`.
    replay: replay::Replay,
    /// What the whole document costs, summed over every body, refreshed on
    /// remesh.
    ///
    /// **There is deliberately no `voxel_size` field beside this.** There used
    /// to be, and it was an application-side copy of a `Volume` property that
    /// four separate sites re-derived; with a document lattice there is exactly
    /// one correct value and `doc.voxel_size()` is it.
    doc_stats: VolumeStats,
    /// Whether a stroke's remeshes have left `doc_stats` and `model_radius`
    /// to be taken again when it ends. See [`Hani::remesh_dirty`].
    measures_stale: bool,
    /// Pointer moves that have come in since the last frame, to be handled
    /// together on the next. See [`Hani::pointer_in`].
    moves: Vec<PointerEvent>,
    /// How the pen was held at each of them. See `app/pen.rs`.
    pen: pen::PenTrail,
    /// Set while all but the last of those moves are handled: the remesh,
    /// the pick for the ring and the ring itself wait for the last.
    batching: bool,
    history_stats: HistoryStats,
    /// What the last export or resample did, for the interface to show.
    status: String,
    /// Which blocks of the properties panel are open, in `PanelSection::ALL`
    /// order.
    expanded: [bool; PanelSection::ALL.len()],
    /// Whether the stats readout over the viewport is showing.
    ///
    /// Closed to begin with: it is seven lines of monospace across the corner
    /// of the model, and the answers it holds are wanted occasionally rather
    /// than continuously. The icon that opens it stays put either way.
    stats_open: bool,
    /// The brush ring and mirror planes, handed to the renderer through
    /// `SharedFrame`. Held here and swapped rather than rebuilt into a fresh
    /// allocation each time.
    overlay: hani_gpu::OverlayBatch,
    /// Where the pointer last met the surface, which is where the ring goes.
    hover: Option<Vec3>,
    /// Which body that surface belonged to, or `None` when the pointer is off
    /// the model.
    ///
    /// The ring is built against this body rather than the active one. Without
    /// it, hovering a second body draws a ring sampled from a field that does
    /// not contain the point -- a confident flat ring below nothing -- and the
    /// mood cannot say that a press there would select rather than carve.
    hover_body: Option<NodeId>,
    /// A hold-and-drag brush resize in progress.
    sizing: Option<Sizing>,
    /// Whether the radius tracks the model's size rather than staying a fixed
    /// number of millimetres.
    dynamic_radius: bool,
    /// How big the active body is, refreshed on remesh.
    /// [`Volume::content_radius`] walks the brick map, so it is not something
    /// to call per frame.
    model_radius: f32,
    /// Which body `model_radius` was last measured on.
    ///
    /// **This is what makes "Dynamic never resizes the brush because the
    /// SELECTION changed" unrepresentable rather than merely avoided.** Without
    /// it, a selection handler that does the obvious thing -- set the active
    /// body, then remesh -- would compare a 5 mm rivet's radius against the
    /// 200 mm bust's and scale the brush by forty. See
    /// [`Hani::rescale_radius`].
    model_radius_body: NodeId,
    /// What is typed in the working-size field, before it is committed. Held as
    /// text so a half-typed number is not rounded or rejected mid-keystroke.
    pub(crate) working_size_field: String,
    /// The cached one-line readout of what the current resolution means.
    ///
    /// CACHED because computing it walks every dense brick, and the widget tree
    /// is rebuilt every frame. Refreshed only when something that changes the
    /// answer happens -- import, open, resample, resize, re-orient. Sculpting
    /// moves the model's extent slightly and deliberately does NOT refresh it:
    /// a per-stroke scan of the whole volume is exactly the shape of work this
    /// engine exists to avoid, and a millimetre of drift in a readout that
    /// exists to answer "is 0.25 mm fine enough" changes nothing.
    pub(crate) detail_advice: String,
    /// Stored views on the strip above the viewport, and the state of
    /// scrubbing through them.
    /// What this session has done, for a bug report.
    breadcrumbs: crate::breadcrumbs::Breadcrumbs,
    /// The status line as it was when it was last recorded as a breadcrumb.
    ///
    /// The trail is taken by watching `status` change rather than by calling
    /// `crumb` at each of the two dozen places that set it. A trail that
    /// depends on remembering to add a line is a trail that goes quietly
    /// incomplete, and the failure is invisible until the one report that
    /// needed it.
    crumbed_status: String,
    /// The bug report dialog, open or not.
    bug_report: Option<BugReport>,
    /// Whether the once-per-session facts have been recorded yet.
    ///
    /// Not done in the constructor: the renderer has not looked at the adapter
    /// by then, and the devices are still being scanned. The first frame is the
    /// earliest point at which there is anything true to say.
    facts_recorded: bool,
    /// The navigation cube's geometry, swapped to the renderer the same way.
    cube: hani_gpu::OverlayBatch,
    /// The transform gizmo's geometry, in a batch of its own because it is
    /// drawn in a pass of its own -- the sculpt's matrix, over the sculpt's
    /// depth. See `SculptRenderer::overlay_pass`.
    gizmo_batch: hani_gpu::OverlayBatch,
    /// The gizmo, when [`Tool::Transform`] has taken a body.
    gizmo: Option<gizmo::Gizmo>,
    /// The active body's field as it was when the gizmo armed.
    ///
    /// **Every bake is from this, and that is the whole anti-degradation
    /// design.** See [`Hani::rebake_gizmo`]. Held out of the document while
    /// armed -- the document holds the baked result -- so peak memory while the
    /// gizmo is up is twice the body, which is what
    /// [`Hani::arm_gizmo`] checks for before it allocates anything.
    gizmo_base: Option<Box<hani_core::Volume>>,
    /// The placement [`Hani::rebake_gizmo`] last actually baked.
    ///
    /// A press and release that moves nothing still reaches the Released arm,
    /// and the bake always restarts from `gizmo_base`, so without this it
    /// reproduces a bit-identical field at full price -- 159 ms of warp and
    /// 7.1 s of repair on the largest arm-able body, for an accidental click.
    /// `Bake::Identity` does not cover it: after any earlier transform in the
    /// same session the placement is not the identity, it is merely the same
    /// one that was baked a moment ago.
    ///
    /// Deliberately NOT gated on `Drag::moved`, which is only set past
    /// `CLICK_SLOP_PX` while `transform_to` runs on every `Moved` event -- a
    /// sub-slop drag really does change the placement, and dropping it would be
    /// silently discarding something the user asked for.
    gizmo_baked: Option<hani_core::Similarity>,
    /// The folder in hand for the gizmo, and its session. See
    /// `app/group_gizmo.rs`.
    group_gizmo: group_gizmo::GroupGizmo,
    /// The body and its symmetry before a drag of Radial copies, for the one
    /// undo step the drag makes. See `app/symmetry_frame.rs`.
    symmetry_drag: Option<(NodeId, Symmetry)>,
    /// The soft join's slider mid-drag. See `app/booleans.rs`.
    soft_drag: booleans::SoftDrag,
    /// Whether the welcome screen is up. See [`crate::welcome`].
    welcome: bool,
    /// The Preferences card is up: the pen, the SpaceMouse and how the
    /// application behaves, out of the panel. See `docs/WORKSPACES.md`.
    preferences: bool,
    /// The palette's brush that was in hand when the rail went to Move or
    /// Paint, for its Brush button to come back to. See `app/rail.rs`.
    rail_last_brush: Option<usize>,
    /// The open workspace and what each remembers. See
    /// `app/workspace.rs`.
    desk: workspace::Desk,
    /// Set while `keep_to_the_mode` runs, so the messages it sends do not
    /// run it again.
    keeping: bool,
    /// Which brush each of the keys 1 to 8 picks in each mode. See
    /// `app/keys.rs`.
    keys: keys::Keys,
    /// The palette's brush whose "put on a key" row is up, by its place.
    key_menu: Option<usize>,
    /// The keyboard shortcuts card is up.
    shortcuts: bool,
    /// Help, open or not. See `app/help.rs`.
    help: help::HelpState,
    /// Whether the About card is up. See `app/about.rs`.
    about_open: bool,
    /// The graphics adapter's name, once asked. See `app/adapter.rs`.
    graphics: Option<String>,
    /// The fingers on a touch screen. See `app/touch.rs`.
    touches: touch::Touches,
    /// What is typed in the printer's and the slicer's fields. See
    /// `app/kept_settings.rs`.
    kept: kept_settings::KeptSettings,
    /// Saving in a browser. See `app/shelf.rs`.
    shelf: shelf::ShelfState,
    /// Snip and report. See `app/snip.rs`.
    snip: snip::SnipState,
    /// Whether it opens by itself next time.
    ///
    /// Held rather than read where it is drawn: the tick is inside `view()`,
    /// which runs every frame, and `welcome::on_startup` opens and parses a
    /// file. Every other setting in this application is loaded once into state
    /// the same way -- see `recent` above and `spacemouse.config`.
    welcome_on_startup: bool,
    /// The filament loaded in each of the printer's slots. See
    /// [`crate::palette`] for why this is machine state rather than part of the
    /// document: a sculpt stores a slot NUMBER, and which filament that is
    /// belongs to the machine in front of you.
    ///
    /// Loaded once into state for the same reason `welcome_on_startup` is --
    /// the panel draws it every frame and must not open a file to do so.
    palette: crate::palette::Palette,
    /// The cube part under the pointer, lit so a click's effect is visible
    /// before the click.
    cube_hover: Option<navcube::Part>,
    /// A camera move in progress after clicking the cube.
    flight: Option<Flight>,
    /// Where the right-click menu is open, in widget pixels.
    menu: Option<Vec2>,
    /// The navigation cube's own menu: where it is drawn and the face it acts
    /// on. See [`CubeMenu`].
    pub(crate) cube_menu: Option<CubeMenu>,
    /// An imported model whose own up is not up, and which way it points.
    ///
    /// Offered rather than applied: nothing in a mesh file states which axis is
    /// up, so this is a guess, and a guess that silently turned the model would
    /// be worse than one the user waves away. `None` once answered either way.
    pub(crate) orient_prompt: Option<hani_core::Facing>,
    /// Which top bar menu is open, if any.
    pub(crate) top_menu: Option<TopMenu>,
    /// The file this sculpt was opened from or last saved to. `None` until it
    /// has one, which is what makes the first Save behave as Save As.
    project_path: Option<std::path::PathBuf>,
    /// Whether the field has changed since it was last written to a file.
    ///
    /// Named `unsaved` rather than `dirty` on purpose: `Hani::dirty` is the
    /// remesh scratch list and `Volume` has its own `mark_dirty`/`take_dirty`
    /// vocabulary, so a second meaning for the word would cost a reader every
    /// time.
    ///
    /// Deliberately tracks the *field* only, not the camera, brush or mirror
    /// state that a `.hani` file also carries. `rescale_radius` mutates
    /// `brush.radius` from inside `remesh_dirty`, which runs on every stroke,
    /// undo, redo, resample, open and reset, so a flag covering brush state
    /// would be set permanently and mean nothing. A save still records the
    /// newer camera; losing an unsaved camera nudge is not worth a prompt.
    unsaved: bool,
    /// An action waiting on the user's answer to "you have unsaved work".
    ///
    /// Its own field rather than a variant of `menu`, because the two dismissal
    /// routes that close a menu -- a press in the viewport and
    /// `Message::MenuClosed` -- must NOT close this. A prompt that a stray click
    /// dismisses is worse than no prompt: the user learns to ignore it.
    confirm: Option<PendingAction>,
    /// What a left drag in the viewport does: sculpt, paint the mask, or cut.
    ///
    /// A mode rather than a modifier, and one enum rather than a bool per mode.
    /// See [`Tool`] for both arguments; neither is a preference.
    pub(crate) tool: Tool,
    /// Sculpts opened or saved recently, for the File menu.
    recent: crate::recent::Recent,
    /// Where the crash net is written.
    ///
    /// Carried rather than recomputed from the environment so a test can point
    /// it at a temporary directory. Without that, running the suite would
    /// delete a real autosave belonging to a real session.
    autosave_file: Option<std::path::PathBuf>,
    /// When the crash net was last written. See [`Hani::maybe_autosave`].
    last_autosave: Instant,
    /// A crash net being written a piece a frame. See `app/autosave.rs`.
    autosave: autosave::Autosave,
    /// When the pointer last did anything, so the autosave can wait for a pause.
    last_activity: Instant,
    /// A numeric field in the menu being typed into.
    ///
    /// Held as text rather than parsed on every keystroke, because a half typed
    /// value like `2.` does not parse and snapping the field back to the old
    /// number mid-edit makes it unusable.
    /// What is being typed into the panels' number fields, by key. See
    /// `ui.rs`.
    pub(crate) drafts: crate::ui::Drafts,
    /// Whether the body list draws its thumbnail column.
    ///
    /// **Session state, and deliberately not in the file.** Photoshop's Panel
    /// Options offers None alongside three sizes and the standard advice for a
    /// heavy document is None, so an off switch is faithful to the reference
    /// rather than a retreat from it -- and by the rule that nothing outside the
    /// file may dirty the document, toggling it must not set `unsaved`.
    ///
    /// # It defaults OFF, and that is the go/no-go gate showing rather than a
    /// preference
    ///
    /// A live picture is a full-body render riding ON TOP of an ordinary frame,
    /// 200 ms after every button release. The plan budgets it at 8 ms and makes
    /// the increment conditional on measuring it: `thumbnail_row` in
    /// `hani-gpu/benches/render.rs` times one and extrapolates to the 45,567
    /// bricks the mesh pool is sized for. **That bench has never been run**, so
    /// there is no number, and shipping an unmeasured per-stroke cost switched
    /// on for everyone is the one thing the gate exists to prevent -- on the
    /// defective imported scans this application is for, which carry several
    /// times more geometry per brick than the sculpted sphere the bench
    /// measures.
    ///
    /// **Flip this to `true` when, and only when, someone has run
    /// `cargo bench -p hani-gpu --bench render` on a quiet machine and the
    /// extrapolated line came in under the budget.** The bench exits 1 if it
    /// did not. `the_thumbnail_switch_does_not_dirty_the_document` pins the
    /// default so the flip cannot happen by accident.
    thumbnails: bool,
    /// Whether protection is tinted on the model, and how strongly.
    ///
    /// **Session state, and view state twice over: it is not in the file, and
    /// it is not protection.** Nothing here changes what a stroke does, which
    /// is worth saying because a shipping professional tool got it wrong --
    /// 3D-Coat has to warn in its own documentation that its Freeze Opacity
    /// "does not affect the freezing strength of the current stroke".
    ///
    /// # The toggle governs the TINT ONLY, and that is the whole safety argument
    ///
    /// This plan argued for a held-only peek key on the grounds that a tint
    /// which can be switched off will be left off. Thomas decided otherwise:
    /// a normal toggle ships. What makes that safe is that [`MaskCard`] is
    /// **unconditional** -- it is up whenever the mask is non-empty, whatever
    /// the tool and whatever this is set to -- so the state "a mask is active
    /// and nothing on screen says so" is still unreachable. Anything added here
    /// that could also silence the card would undo that; the card takes no
    /// input from these fields at all, which is the structural half of it.
    ///
    /// The residual is accepted knowingly: a user who leaves the tint off for a
    /// long session paints blind, and the card is a smaller signal than the
    /// tint it replaces. Revisit on the first report of a stroke that "did
    /// nothing" while the tint was off.
    show_mask: bool,
    /// Whether painted slots are drawn in their filament colours.
    ///
    /// View state: see [`hani_gpu::Uniforms::paint_shown`]. Nothing about it
    /// changes what a stroke writes, and a document saved with it off carries
    /// every slot it was painted with.
    show_paint: bool,
    /// Whether creases are drawn darker and edges lighter.
    ///
    /// View state, and the one view toggle that costs a remesh: the signal is
    /// measured per vertex at mesh time (`hani_core::signals`) and a mesh
    /// made while this was off does not carry it. [`Hani::remesh_dirty`]
    /// reads this to choose which mesher to call, and the toggle's handler
    /// marks everything dirty so the choice reaches every brick. Off costs a
    /// user who never looks at it nothing at all.
    show_cavity: bool,
    /// How finely an export is meshed. See [`ExportDetail`].
    export_detail: ExportDetail,
    /// The key a parting puts between two parts: its kind, its size, and the
    /// clearance, which belongs to the printer. See `hani_core::part`.
    key_spec: hani_core::part::KeySpec,
    /// How the paint brush touches the model, its settings, and where the
    /// stroke in progress has got to. See `app/paints.rs`.
    touch: paints::TouchState,
    /// Whether paint is drawn flat, with no light on it. View state.
    flat_paint: bool,
    /// The studio brushes: an operator upstream has no brush for, a lazy
    /// stroke, size from pressure. See `app/studio.rs`.
    studio: studio::StudioState,
    /// The measurement being taken or last taken. See `app/measure.rs`.
    measure: measure::MeasureState,
    /// The document, set aside while the workbench is up. See
    /// `app/workbench.rs`.
    bench: Option<Box<workbench::Bench>>,
    /// The brushes that add a shape or a cable. See `app/adding.rs`.
    inserts: adding::AddState,
    /// The form tools' settings. See `app/form.rs`.
    form: form::FormState,
    /// The revolve's axis and the line being drawn. See `app/revolve.rs`.
    revolve: revolve::RevolveState,
    /// The deformers' settings. See `app/deform.rs`.
    deform: deform::DeformState,
    /// The reference pictures and the floor, the document's: saved in the
    /// file. See `app/references.rs`.
    references: hani_core::reference::References,
    /// Which plane the references panel places, and the pictures decoded.
    reference_view: references::ReferencesState,
    /// Which palette brush is in hand as. See `app/keeping.rs`.
    kept_as: keeping::KeepingState,
    /// How big the interface is drawn. See `app/ui_scale.rs`.
    ui_scale: ui_scale::UiScale,
    /// Shell and extract settings. See `app/hollow.rs`.
    hollow: hollow::HollowState,
    /// The overhang view: whether it is on, and how steep a lean the
    /// printer manages. See `app/overhang.rs`.
    overhang: overhang::OverhangState,
    /// The paint brush, painting in colour. See `app/painting.rs`.
    painting: painting::PaintingState,
    /// The worker threads. See `app/cores.rs`.
    cores: cores::CoresState,
    /// The ring cut. See `app/ring_cut.rs`.
    ring_cut: ring_cut::RingCut,
    /// The bodies drawn apart. See `app/explode.rs`.
    explode: explode::Explode,
    /// What a drybrush would catch, under the ring. See `app/dry_preview.rs`.
    dry_preview: dry_preview::DryPreview,
    /// How many texels on a side a baked texture is. See `app/paints.rs`.
    texture_size: u32,
    /// How strong the tint is when it is shown at all, 0..1.
    ///
    /// The shader lifts a light mask under this (`1 - (1 - m)^2`), with no
    /// step at nought; this scales the result and reaches zero only through
    /// the toggle.
    mask_tint: f32,
    /// Whether the held peek key is down.
    ///
    /// Kept alongside the toggle rather than replaced by it, because it costs
    /// nothing and is the faster gesture for a momentary look -- and because it
    /// is the way back to seeing the mask for a user who has switched the tint
    /// off and forgotten they did.
    mask_peek: bool,
    /// Which of the four absolute filters the amount slider drives.
    ///
    /// One slider for four verbs rather than four sliders, because the tool
    /// menu is 210 px wide and `const HEIGHT` is sized once for the whole mask
    /// block. Choosing a different filter mid-drag commits the drag first --
    /// see `mask_amount_changed`.
    mask_filter: MaskFilter,
    /// Where the amount slider sits, and **zero between gestures**.
    ///
    /// It is a gesture parameter and not a stored setting: the filter is
    /// absolute, so leaving the slider at 1.0 would make the next grab re-blur
    /// an already-blurred mask the moment it was touched. Releasing it puts it
    /// back to zero, which is the amount at which every filter is the identity.
    mask_amount: f32,
    /// The mask as it stood when the current filter gesture began.
    ///
    /// **This is what makes the filter absolute rather than accumulative.**
    /// Every change of the amount re-applies from here, so dragging out and
    /// back lands exactly where it started, and dragging to zero puts this very
    /// field object back on the body -- bit for bit, because it is moved and
    /// never rebuilt. ZBrush's BlurMask is the accumulative alternative and it
    /// has been its top masking complaint since 2007.
    mask_grab: Option<MaskGrab>,
    /// The narrowest feature a cavity or smoothness mask holds, in MILLIMETRES.
    ///
    /// Session state and never in the document: it is a question the user asks,
    /// not a property of the body. In millimetres because a body has one fixed
    /// voxel size, which is what makes "narrower than 1.5 mm" mean the same
    /// thing at every resolution.
    mask_feature_mm: f32,
    /// The thickest wall a thickness mask selects, in VOXELS.
    ///
    /// Voxels because the ceiling is the narrow band's and not the model's --
    /// see [`Message::MaskThicknessChanged`].
    mask_thickness_voxels: u32,
    /// Which body's picture lives in which cell of the atlas, and which of them
    /// are out of date. See [`crate::thumbnails::Thumbnails`], whose header is
    /// where the refresh rule is written down.
    thumbs: crate::thumbnails::Thumbnails,
    /// Whether the `+` button's little menu of primitives is open.
    ///
    /// A plain flag rather than a `stack!` layer: the menu is drawn inside the
    /// panel column, under the verb row, so it needs no scrim and cannot fall
    /// into the trap that iced 0.14 stack layers do not block what is beneath
    /// them.
    adding: bool,
    /// A body delete waiting on an answer, because it is large enough that undo
    /// may not be able to hold it.
    pending_delete: Option<PendingDelete>,
    /// How large a delete has to be before it asks first.
    ///
    /// [`hani_core::DEFAULT_RECLAIM_BUDGET`] by default, and one number rather
    /// than two that happen to coincide: a delete that would be evicted from
    /// history before it could be undone is exactly the delete that has to warn.
    ///
    /// Carried rather than a constant read in place so that a test can point it
    /// at a small number -- the alternative is a fixture that really does hold
    /// 512 MB of bricks, which is half a gigabyte of allocation inside a test
    /// process running alongside every other test in the crate.
    delete_prompt_bytes: usize,
    /// A merge waiting on an answer, because the one entry it would push is
    /// large enough that undo may not be able to hold it.
    pending_merge: Option<PendingMerge>,
    /// How large the entry a merge would push has to be before it asks first.
    ///
    /// The same [`hani_core::DEFAULT_RECLAIM_BUDGET`] as the delete prompt,
    /// carried rather than read in place for the same reason: a test that
    /// pointed this at the real 512 MB would have to build half a gigabyte of
    /// bricks.
    ///
    /// **One number against the SUM of the two halves, which is an
    /// approximation, and here is where it is loose.** The bricks half is
    /// charged to the 256 MB stroke budget and the consumed body to the 512 MB
    /// reclaim allowance, so the exactly-right test is two comparisons. The sum
    /// against 512 MB is within a factor of two of both and errs on the side of
    /// asking: a fully overlapping merge records its priors twice over -- once
    /// as bricks and once as the body -- so anything that would breach the
    /// 256 MB stroke budget on its own is already over 512 MB summed. What it
    /// permits is a large DISJOINT merge, whose bricks half is 32 bytes each;
    /// that one is exactly as recoverable as deleting the same body, which does
    /// not prompt below the same number either.
    merge_prompt_bytes: usize,
    /// A split waiting on an answer.
    ///
    /// **Always raised, and never on a size threshold**, which is the one place
    /// this differs from the delete and merge prompts. A split's headline is
    /// how many loose parts there turn out to be, and that number is not
    /// something anybody can predict from looking at the model -- an imported
    /// scan is one row in the panel whether it is one shell or four thousand.
    /// So the card is the operation's only sensible shape: it reports the
    /// count, says which parts become bodies and which are swept up, and only
    /// then does anything happen.
    pending_split: Option<PendingSplit>,
    /// The row being renamed and what has been typed into its field so far.
    ///
    /// The typed text is held HERE rather than written into the node on every
    /// keystroke, so that one rename is one undo entry rather than one per
    /// letter, and so that Escape has something to revert to. It is already
    /// clamped to [`hani_core::MAX_NAME_BYTES`] -- see
    /// [`Hani::commit_rename`] for why the clamp is on the way in and not on
    /// the way out.
    ///
    /// **Never persisted, and there is nowhere it could be**: nothing in
    /// `ProjectState` names it, and a rename in flight when the application
    /// quits is simply a rename that did not happen.
    renaming: Option<(NodeId, String)>,
    /// A body row being dragged to a new place in the tree. See [`RowDrag`].
    row_drag: Option<RowDrag>,
}

/// A row the pointer picked up, and where letting go would put it.
///
/// # Why the target is held here and not worked out when the button comes up
///
/// **The value the indicator drew is the value the commit performs.** Every
/// tree drag that puts a line in one place and the row in another has two
/// copies of the arithmetic -- one in the drawing code and one in the drop
/// handler -- and they disagree at the edges. Here there is one:
/// [`hani_core::drop_target`] runs on each pointer event, its answer is
/// stored, the panel draws that, and [`Hani::finish_row_drag`] hands the same
/// value to [`Document::reparent`].
///
/// # It is armed by the row's press and released by the GLOBAL one
///
/// `mouse_area::on_press` arms it, but the commit rides on
/// `PointerEvent::Released`, which `viewport::route_pointer` publishes for
/// every release anywhere in the window without capturing it. The row's own
/// `on_release` only fires while the pointer is still over that row, so a drag
/// that ended over the tool strip -- or outside the window -- would stick.
#[derive(Debug, Clone, Copy)]
struct RowDrag {
    /// The row that was pressed, by id and not by index, so a queued message
    /// that arrives after the list has moved cannot name a different row.
    id: NodeId,
    /// Where a release would put it, or `None` while the pointer is over a gap
    /// that refuses the block -- which is also the state a press that has not
    /// moved yet is in, so a click that never became a drag commits nothing.
    target: Option<DropTarget>,
    /// The row `target` is the INSIDE of, when it is a closed folder's middle
    /// band. Purely how the indicator draws: the filled row rather than the
    /// line. Worked out here because `view()` may not work anything out.
    into: Option<usize>,
    /// The refusal already on the status line.
    ///
    /// A pointer crossing a 214 px panel raises dozens of move events, and
    /// rewriting the same sentence on each of them would allocate per event and
    /// fill the breadcrumb trail with one line repeated. The line is written
    /// when the reason CHANGES.
    said: Option<DropRefusal>,
}

impl RowDrag {
    /// Whether the pointer has actually moved since the press.
    ///
    /// **A press is not yet a drag**, and the difference is visible: this is
    /// what decides whether the block folds away on screen. Folding on the
    /// press itself would snap a folder shut and open it again on every plain
    /// click on its row.
    ///
    /// It is derived rather than stored because there is nothing to store: one
    /// of the two fields is written by every move message and neither is
    /// written by anything else.
    fn under_way(self) -> bool {
        self.target.is_some() || self.said.is_some()
    }
}

/// A delete the user has been asked about, held until they answer.
///
/// It carries the size it measured rather than re-measuring on the way out:
/// `Volume::stats` walks a body's whole brick map, and the number the prompt
/// showed has to be the number the decision was made about.
pub(crate) struct PendingDelete {
    pub(crate) id: NodeId,
    pub(crate) name: String,
    pub(crate) bytes: usize,
    /// How many bodies go with it, which for a folder is not one.
    ///
    /// **Folders make the prompt the common path rather than the exception**,
    /// and a card that says "Delete Group 1?" over a size without saying how
    /// many parts are inside it is asking the user to remember what they put
    /// there. It is counted here beside the bytes, and for the same reason:
    /// the number the prompt shows has to be the number the decision was made
    /// about.
    pub(crate) bodies: usize,
}

/// A merge the user has been asked about, held until they answer.
///
/// It carries everything the card shows, measured once, for the reason
/// [`PendingDelete`] carries its size: the walk that produced the number is a
/// walk of a brick map, and the number the prompt showed has to be the number
/// the decision was made about.
///
/// The `source` id and not an index: a queued message could in principle arrive
/// after the list has moved under it, and an index would then name a different
/// row. `apply_merge` re-resolves the target from the id, so a source that has
/// gone simply does nothing.
pub(crate) struct PendingMerge {
    pub(crate) source: hani_core::NodeId,
    pub(crate) source_name: String,
    pub(crate) target_name: String,
    /// The whole entry, which is the number the user is being asked about.
    pub(crate) bytes: usize,
    /// The bricks half, charged to the stroke budget.
    pub(crate) stroke_bytes: usize,
    /// The consumed body, charged to the reclaim allowance.
    pub(crate) reclaim_bytes: usize,
}

/// A split the user has been asked about, held until they answer.
///
/// **It carries the whole plan, and the plan carries the labelling.** The walk
/// that produced "4,812 loose parts" is a walk of every voxel of the body, and
/// the emit has to use that same labelling or the card described one split and
/// the button performed another. It is also far too expensive to run twice.
///
/// The `name` is copied for the card rather than looked up at draw time,
/// because `view()` runs at display rate and a lookup there is a scan of the
/// node list per frame.
pub(crate) struct PendingSplit {
    pub(crate) name: String,
    pub(crate) plan: hani_core::SplitPlan,
}

/// An animated camera move to an orientation.
///
/// Instant would be disorienting on a sculpt: with nothing but a shaded form to
/// go on there is no way to tell a jump from a different model. A quarter second
/// is enough to follow and short enough not to be in the way.
#[derive(Debug, Clone, Copy)]
struct Flight {
    from: (f32, f32, f32),
    to: (f32, f32, f32),
    elapsed_ms: f32,
}

/// The navigation cube's right-click menu while it is open.
///
/// Fusion's ViewCube, and the answer to "down isn't down": a left click on the
/// cube moves the camera, a right click asks what that face of the *model*
/// should become. The two need different state because the second one has to
/// remember which face was picked until the answer arrives.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CubeMenu {
    /// Where to draw it, in widget pixels.
    pub(crate) at: Vec2,
    /// The face the menu was opened on, which is the one that will be moved.
    pub(crate) facing: hani_core::Facing,
}

/// The bug report dialog while it is open.
///
/// Held on the application rather than passed around because the description
/// editor owns its own cursor and selection state, which has to outlive a
/// redraw.
pub(crate) struct BugReport {
    description: iced::widget::text_editor::Content,
    /// Whether to attach the diagnostics and the session trail.
    ///
    /// On by default, and the reason the dialog shows the whole payload: a
    /// report without them is rarely answerable, and a user who can read
    /// exactly what is attached can decide for themselves rather than being
    /// asked to trust a sentence about it.
    with_detail: bool,
}

impl BugReport {
    fn new() -> Self {
        Self { description: iced::widget::text_editor::Content::new(), with_detail: true }
    }
}

/// Something the user asked for that would discard unsaved work, held until
/// they say what to do about it.
///
/// Each of these throws the current document away: New reseeds the sphere, Open
/// swaps a file in, and Quit ends the process.
///
/// **They are not, as this comment used to claim, "the complete set of ways
/// work can be lost", and that sentence was becoming a lie a reader would
/// trust.** Two things are true instead. The gate lives in one place, so the
/// list of *guarded* actions is complete by construction. And what counts as
/// something to lose is no longer just `unsaved`: importing a mesh replaces
/// every body in the document, so with a saved multi-body project open it used
/// to discard all of them silently, having asked nothing, because nothing was
/// unsaved. See [`Hani::would_lose_work`].
///
/// `Clone` rather than `Copy` because one variant carries a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PendingAction {
    NewSculpt,
    /// Open, by way of the file dialog.
    Open,
    /// Open a specific file, chosen from the recent list, so no dialog.
    OpenRecent(std::path::PathBuf),
    /// Open a sculpt kept in the browser, by its name. See `app/shelf.rs`.
    OpenStored(String),
    /// Load the crash net. Its own variant rather than an `OpenRecent` of the
    /// autosave path, because the document that comes back must NOT be owned by
    /// that path -- see [`Hani::recover_autosave`].
    RecoverAutosave,
    /// Import a mesh, REPLACING the document. It discards every body exactly as
    /// Open does, so it goes through the same gate -- and, unlike Open, it does
    /// not put a file the user chose in their place, which is why it is guarded
    /// on the body count as well as on `unsaved`.
    Import,
    /// Carries the window, because with `exit_on_close_request(false)` the
    /// close has to be issued by us against that specific window.
    Quit(iced::window::Id),
    /// Install a downloaded update and restart into it.
    ///
    /// **The update question is NOT asked here.** That is its own always-shown
    /// modal, because `guard` only prompts `if would_lose_work`, so routing the
    /// whole thing through it would restart the app under anyone whose document
    /// happened to be saved. What goes through `guard` is the *consequence* --
    /// the work question, asked only when there is work -- which is exactly what
    /// this gate is for.
    ///
    /// Carries the window for the same reason `Quit` does.
    Restart(iced::window::Id),
}

impl PendingAction {
    /// What the prompt says is about to happen.
    pub(crate) fn describe(&self) -> &'static str {
        match self {
            PendingAction::NewSculpt => "Starting a new sculpt",
            PendingAction::Open | PendingAction::OpenRecent(_) | PendingAction::OpenStored(_) => {
                "Opening another file"
            }
            PendingAction::RecoverAutosave => "Recovering the autosave",
            PendingAction::Import => "Importing a mesh in place of every body",
            PendingAction::Quit(_) => "Quitting",
            // A fixed phrase, not the ordinal: `describe` returns `&'static
            // str` and has one call site.
            PendingAction::Restart(_) => "Restarting into the new build",
        }
    }
}

/// Whether a world space box has material on both sides of a mirror plane
/// through [`MIRROR_CENTRE`].
///
/// Touching counts as straddling: a body whose surface reaches the plane
/// exactly has its twin land against it rather than out in space, which is a
/// join and not a free-floating shell.
fn straddles((low, high): (Vec3, Vec3), axis: MirrorAxis, frame: hani_core::SymmetryFrame) -> bool {
    let component = match axis {
        MirrorAxis::X => 0,
        MirrorAxis::Y => 1,
        MirrorAxis::Z => 2,
    };
    // Asked in the body's own frame, the box's corners carried into it. See
    // `app/symmetry_frame.rs`.
    let (mut least, mut most) = (f32::MAX, f32::MIN);
    for corner in 0..8 {
        let pick = |bit: usize, axis: usize| if corner & bit == 0 { low[axis] } else { high[axis] };
        let at = Vec3::new(pick(1, 0), pick(2, 1), pick(4, 2));
        let local = frame.to_local(at - MIRROR_CENTRE)[component];
        least = least.min(local);
        most = most.max(local);
    }
    least <= 0.0 && 0.0 <= most
}

/// Where bug reports go.
pub const ISSUE_URL: &str = "https://github.com/DanielJoyce/hanisculpt/issues/new";

/// The extension a sculpt is saved with.
pub const PROJECT_EXTENSION: &str = "hani";

/// The extension a sculpt was saved with before the name was HaniSculpt.
/// The Open dialog offers both; the file inside is read either way (see
/// `hani_core::project`).
pub const LEGACY_PROJECT_EXTENSION: &str = "brokkr";

/// Ask for a sculpt to open.
///
/// Async, and driven through a `Task`, because the portal dialog is a round trip
/// to another process: the blocking form would stall the event loop, which is
/// the same thread that draws.
///
/// The dialog opens unparented. iced 0.14 gives no way to hand `rfd` a window
/// handle -- the one route, `iced::window::run`, hands back a borrowed handle on
/// the event loop thread and its return type must be `Send`, so it cannot reach
/// an async task. In practice the portal still centres the dialog on the right
/// screen; verify that rather than assume it.
async fn pick_project_to_open() -> Option<std::path::PathBuf> {
    let handle = rfd::AsyncFileDialog::new()
        .set_title("Open sculpt")
        .add_filter("HaniSculpt", &[PROJECT_EXTENSION, LEGACY_PROJECT_EXTENSION])
        .pick_file()
        .await?;
    // In a browser the file has no path: its bytes are held under its
    // name. See `files.rs`.
    #[cfg(target_arch = "wasm32")]
    return Some(crate::files::hold(&handle.file_name(), handle.read().await));
    #[cfg(not(target_arch = "wasm32"))]
    Some(handle.path().to_path_buf())
}

// Not in a browser, which keeps a save itself. See `app/shelf.rs`.
#[cfg(not(target_arch = "wasm32"))]
async fn pick_project_to_save() -> Option<std::path::PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_title("Save sculpt")
        .add_filter("HaniSculpt", &[PROJECT_EXTENSION])
        .set_file_name(format!("sculpt.{PROJECT_EXTENSION}"))
        .save_file()
        .await
        .map(|handle| {
            let path = handle.path().to_path_buf();
            // A portal can hand back a name with no extension. Adding it keeps
            // the file recognisable to the open dialog's own filter.
            if path.extension().is_some() { path } else { path.with_extension(PROJECT_EXTENSION) }
        })
}

async fn pick_export_target(format: ExportFormat) -> Option<std::path::PathBuf> {
    let extension = format.extension();
    // In a browser the export is a download, and the browser asks where it
    // goes, or puts it with the others. See `files.rs`.
    #[cfg(target_arch = "wasm32")]
    return Some(std::path::PathBuf::from(format!("sculpt.{extension}")));
    #[cfg(not(target_arch = "wasm32"))]
    rfd::AsyncFileDialog::new()
        .set_title(format!("Export {}", format.label()))
        .add_filter(format.label(), &[extension])
        .set_file_name(format!("sculpt.{extension}"))
        .save_file()
        .await
        .map(|handle| {
            let path = handle.path().to_path_buf();
            if path.extension().is_some() { path } else { path.with_extension(extension) }
        })
}

/// Ask for a mesh to import.
///
/// `pick_file`, so unlike the two save dialogs there is no extension fixup: the
/// file already exists and its name is not ours to correct.
async fn pick_mesh_to_import() -> Option<std::path::PathBuf> {
    let handle = rfd::AsyncFileDialog::new()
        .set_title("Import mesh")
        .add_filter("Mesh", &hani_core::MESH_EXTENSIONS)
        .pick_file()
        .await?;
    // As `pick_project_to_open`.
    #[cfg(target_arch = "wasm32")]
    return Some(crate::files::hold(&handle.file_name(), handle.read().await));
    #[cfg(not(target_arch = "wasm32"))]
    Some(handle.path().to_path_buf())
}

/// A hold-and-drag gesture adjusting one brush number.
///
/// Holds where the pointer started and what the value was, so the gesture is
/// absolute: a drag out and back returns to exactly where it began, where
/// accumulating per-event deltas would drift.
#[derive(Debug, Clone, Copy)]
struct Sizing {
    what: SizingTarget,
    from_pixel: Vec2,
    original: f32,
}

/// What the standing mask overlay says, cached.
///
/// # Why there is a card at all, and why it is unconditional
///
/// This is the mechanism that makes "a mask is active and nothing on screen
/// says so" unreachable. It is up **whenever anything is masked, whatever tool
/// is live and whatever the tint is set to** -- the show-mask toggle governs the
/// tint only. That is the whole of the safety argument for shipping a toggle
/// that can switch the tint off: the card cannot be switched off. A masked
/// surface reading as a broken brush is the failure the whole masking design is
/// arranged around, and an invisible mask is the single most-reported ZBrush
/// masking complaint.
///
/// # Why it names the body, and counts the ones off screen
///
/// The mask is per body. Mask the head, hide the head, and a card scoped to the
/// active body disappears while the document still protects material. A
/// document-wide `MASK 62%` while the user sculpts an unmasked torso is worse
/// still: it teaches them to ignore it, which is exactly how ZBrush's
/// darker-thumbnail cue eroded. So it names the body the percentage is about
/// and appends a count of the masked bodies that are not on screen.
///
/// # Why it is a cache
///
/// `view()` runs at display rate. Formatting a percentage there would be the
/// `detail_advice` mistake this project has already made twice; computing one
/// would be a pass over up to 45,567 mask bricks per frame. So the strings are
/// built when something changes and `revision` is what says whether it has --
/// see [`hani_core::MaskField::revision`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MaskCard {
    /// The body the percentage is about, and the mask revision it was computed
    /// from. Together these are the cache key.
    body: NodeId,
    revision: u64,
    /// How many masked bodies were off screen when the strings were built, and
    /// how many were on screen but were not the one the headline is about.
    ///
    /// Part of the key rather than part of the answer: the counts come from a
    /// walk of the node rows, which is cheap, but turning them into words is a
    /// `format!` and `refresh_mask_card` runs on the frame tick. Keeping them
    /// here is what lets the rebuild be skipped rather than merely be short.
    ///
    /// **Both counts, and that is increment 24's review finding.** Only the
    /// hidden one was kept, so a body that was masked, VISIBLE and not active
    /// was counted nowhere: press Mask all on a cube, click another body, and
    /// the card went away entirely while a fully protected cube sat on screen.
    /// The first the user heard of it was a cut sparing the whole thing.
    off_screen_count: usize,
    elsewhere_count: usize,
    /// `MASK — head, 62%`, or `MASK — 2 masked, hidden` when the active body
    /// carries none itself.
    pub(crate) headline: String,
    /// `+2 masked, hidden`, or empty. Its own string so the card can dim it.
    pub(crate) off_screen: String,
    /// `+2 masked, elsewhere`, or empty. Masked bodies that are on screen but
    /// are not the one the headline is about.
    pub(crate) elsewhere: String,
}

/// The revision a card about a body with no mask of its own is keyed on.
///
/// [`hani_core::MaskField::revision`] counts writes, so a mask that is not
/// free has had at least one and can never be mistaken for this.
const NO_MASK_HERE: u64 = 0;

/// What to press next, for the mask cutter's status line.
///
/// Lifted out of the `format!` it was written in so that it can be asserted
/// without an application. That is not hypothetical: the sentence this replaces
/// sent users to the BODIES panel, whose verb row is five icons and whose Split
/// fires [`Message::BodySplit`] -- the connected-component split, a different
/// operation -- and nothing in the suite could see it, because the sentence
/// only ever existed inside a gesture handler. A beta user found it in an hour.
///
/// Two things it must never say.
///
/// It must not point at the mask card's verb row unless that row will be
/// showing. The row is gated on [`MaskCard::names_the_active_body`], so asking
/// whether THIS gesture masked the ACTIVE body answers that without keeping a
/// second copy of the gate: if it did, `refresh_mask_card` runs on the
/// every-message tail before `view` and cannot leave the card without verbs.
/// It is also the stronger question -- the card's own gate is satisfied by a
/// mask left over from an earlier gesture, and advice resting on that would
/// point at a button that splits along a mask this stroke never drew.
///
/// And it must not read as an offer to lift them all off at once:
/// `split_the_mask_off` takes `doc.active()` and nothing else, so N masked
/// bodies are N presses with a click in between.
///
/// The `false` arms still say BODIES, and this time it is true: BODIES is where
/// a body is CHOSEN. The verb is named on the card either way. Say so here, or
/// the next reader will helpfully "fix" it back.
fn split_advice(touched_active: bool, touched: usize) -> &'static str {
    match (touched_active, touched) {
        (true, 1) => "split off on the mask card",
        (true, _) => "split off on the mask card, one body at a time",
        (false, 1) => "click it in BODIES, then split off on the mask card",
        (false, _) => "click one of them in BODIES, then split off on the mask card",
    }
}

impl MaskCard {
    /// `+2 masked, hidden`, or empty when none are.
    fn hidden_count_line(count: usize) -> String {
        match count {
            0 => String::new(),
            count => format!("+{count} masked, hidden"),
        }
    }

    /// `+2 masked, elsewhere`, or empty when none are.
    fn elsewhere_count_line(count: usize) -> String {
        match count {
            0 => String::new(),
            count => format!("+{count} masked, elsewhere"),
        }
    }

    /// The headline and the second line for a card that is up only because
    /// bodies OTHER than the active one are masked.
    ///
    /// One of the two counts is promoted into the headline and the other stays
    /// a detail line, so neither can go unsaid and the card still opens with a
    /// sentence rather than with a bare "MASK". "Elsewhere" wins the headline
    /// when there is a choice, because a masked body the user can SEE is the
    /// one they are about to try to sculpt.
    fn about_other_bodies(off_screen: usize, elsewhere: usize) -> (String, String) {
        match (elsewhere, off_screen) {
            (0, hidden) => (format!("MASK — {hidden} masked, hidden"), String::new()),
            (elsewhere, 0) => (format!("MASK — {elsewhere} masked, elsewhere"), String::new()),
            (elsewhere, hidden) => {
                (format!("MASK — {elsewhere} masked, elsewhere"), Self::hidden_count_line(hidden))
            }
        }
    }

    /// Say new counts of masked bodies that are not the active one, keeping
    /// whatever the card already says about the active one.
    ///
    /// The one thing that can go stale without the mask itself moving, which is
    /// why it is a separate write rather than a reason to rebuild: hiding a
    /// masked body, or making another one active, changes these lines and
    /// nothing else on the card. Reached only when a count has actually
    /// changed, so the `format!`s in here are never on an idle frame.
    fn rewrite_the_other_counts(&mut self, off_screen: usize, elsewhere: usize) {
        self.off_screen_count = off_screen;
        self.elsewhere_count = elsewhere;
        if self.names_the_active_body() {
            self.off_screen = Self::hidden_count_line(off_screen);
            self.elsewhere = Self::elsewhere_count_line(elsewhere);
        } else {
            // This card IS the counts -- there is no percentage to keep.
            let (headline, hidden) = Self::about_other_bodies(off_screen, elsewhere);
            self.headline = headline;
            self.off_screen = hidden;
            self.elsewhere = String::new();
        }
    }

    /// Whether the percentage on this card is about the ACTIVE body.
    ///
    /// What the `[invert]`, `[clear]` and `[split off]` buttons are gated on,
    /// and they need gating: all three act on the active body, and a card that
    /// is up purely because something is masked OFF screen would otherwise
    /// offer to clear a mask the user cannot see -- on a body that has none.
    pub(crate) fn names_the_active_body(&self) -> bool {
        self.revision != NO_MASK_HERE
    }
}

/// The mask a live filter gesture re-applies from.
///
/// Held by the application rather than by the volume, because the volume is
/// showing the FILTERED mask for the length of the drag and there is nowhere in
/// it for a second one. It is moved out of the body on grab and moved back --
/// or moved into the history entry -- when the drag ends, so a gesture holds
/// two masks and never three. That is the arithmetic
/// [`hani_core::MaskField::would_fit`] is asked about before the grab starts.
struct MaskGrab {
    /// Which body it came off. A drag that outlives a change of active body
    /// commits rather than following it: the snapshot belongs to one mask.
    body: NodeId,
    filter: MaskFilter,
    snapshot: MaskField,
}

/// Which number a sizing drag is moving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizingTarget {
    Radius,
    Strength,
}

/// What a left drag in the viewport currently does.
///
/// # An enum rather than two bools, and it is the fix rather than tidiness
///
/// This replaced `cut_armed: bool`. Adding masking as a second bool would have
/// been four states with one of them illegal -- cut armed *and* mask active,
/// reachable from two buttons sixty pixels apart in the same strip -- and the
/// press dispatch would have let the cut win while MASK stayed highlighted
/// doing nothing. The enum makes that state unrepresentable, and it is no more
/// code than the bool it replaced: [`Message::ToolChanged`] toggles back to
/// [`Tool::Sculpt`] when it lands on the live tool, so the cut costs net zero
/// message variants.
///
/// # Why masking is a mode and not a modifier
///
/// There is no free modifier left. Control and alt are deliberate synonyms for
/// "invert the brush" and were made synonyms so that neither habit is wrong;
/// shift is smooth; and `viewport::shortcut` is not even handed alt for a bare
/// key. Worse, the modifier flags are written in exactly one place -- inside a
/// handler that returns early while a modal is open -- and nothing resets them,
/// because this application handles no focus event at all. A stale invert flag
/// today still sculpts and the ring is red about it. **A stale MASK flag would
/// be a stroke that changes no geometry and silently paints protection over the
/// model**, which is strictly worse. ZBrush needs two modifiers for mask and
/// unmask, so its gesture is simply unavailable here and a mode is the answer.
///
/// # The collision with the gizmo, named rather than left to be discovered
///
/// `docs/BUILD-SPEC.md` parks a `Tool` extraction behind "a third
/// gesture-scoped operation" and awards it to the transform gizmo. This is not
/// that one: that one is about the press-lock-commit lifecycle Move and the
/// plane cut share. When the gizmo lands it adds a VARIANT here rather than a
/// second concept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// The brush in [`Hani::brush`] writes the field. The default.
    Sculpt,
    /// The brush paints protection instead. Key `m`.
    Mask,
    /// The next left drag is a plane cut line. One shot: [`Hani::finish_cut`]
    /// puts the tool back to [`Tool::Sculpt`].
    Cut,
    /// The transform gizmo moves, turns and scales the active body. Key `w`.
    ///
    /// **The predicted fourth variant**, and it arrived as the paragraph above
    /// said it would: a variant here rather than a second concept beside
    /// [`DragKind`]. The press-lock-commit lifecycle it shares with the plane
    /// cut is [`DragKind::Transforming`], which locks the handle at the press
    /// and commits on the release.
    Transform,
}

/// Turn a window event the widget tree did not want into a message.
///
/// This is the callback `Hani::subscription` hands to
/// `iced::event::listen_with`, lifted out of it and named so that it can be
/// tested. It costs nothing to lift: `listen_with` takes a bare `fn` pointer
/// (`iced_futures-0.14.0/src/event.rs:26`), so the inline version was already
/// a non-capturing closure and this is the same value with a name on it.
///
/// It was NOT tested while it was inline, and that was the gap: every keyboard
/// guard test synthesises `Message::KeyPressed` directly, which is downstream
/// of here. Deleting the `KeyPressed` arm would have killed the entire
/// keyboard -- no undo, no brush digits, no mirror keys -- with all of those
/// tests still green, reporting a guard working perfectly on a message the
/// application could no longer produce.
///
/// The two things it decides, and neither belongs anywhere else:
///
/// * Captured events are dropped. That is what makes the shortcuts
///   focus-aware -- a focused text input consumes its own keystrokes -- and
///   inverting it is how `1`-`7`, `s`, `u`, `x`, `y` and `z` were stolen from
///   every text field for a year.
/// * A press forwards the raw key; it does not decide what the key MEANS.
///   There is no `self` here and no way to get one, so whether a modal is up
///   is invisible from inside. `Hani::on_key` decides, and holds the guard.
///
/// It is also where a left press that nobody wanted becomes
/// [`Message::PressedNothing`], for the same reason and by the same test: this
/// is the application's "nobody claimed that" tap, and an unclaimed left press
/// is exactly what blurs a focused `text_input`. See that message.
fn key_event(
    event: iced::Event,
    status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    if status == iced::event::Status::Captured {
        return None;
    }
    match event {
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) => {
            Some(Message::KeyPressed { key, modifiers })
        }
        // An ignored LEFT press is a blur, and nothing else in the application
        // can see one. Left only, because that is the button `text_input`
        // blurs on.
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)) => {
            Some(Message::PressedNothing)
        }
        // Releasing a sizing key ends the gesture. Without this the pointer
        // would keep resizing the brush instead of going back to sculpting.
        // The mask's peek key is here for the same reason and with the same
        // exemption: a held key that never comes back up leaves the tint stuck
        // on, which is the safe direction to fail but is still not what the
        // user asked for.
        //
        // Not routed through `on_key` and deliberately not guarded by the
        // modal check: a modal that opened mid-gesture must still let the
        // gesture end, and ending one that never started is already a no-op.
        iced::Event::Keyboard(iced::keyboard::Event::KeyReleased {
            key: iced::keyboard::Key::Character(character),
            ..
        }) => match character.to_ascii_lowercase().as_str() {
            "s" | "u" => Some(Message::SizingEnded),
            "h" => Some(Message::MaskPeekEnded),
            _ => None,
        },
        _ => None,
    }
}

/// The rename field's widget id, so that beginning a rename can focus it.
///
/// **The only widget in the application with an id at all.** One constant and
/// not one per row: there is at most one rename in flight, so the field exists
/// in exactly one row at a time and `operation::focus` finds it wherever that
/// row happens to be. An id per row would also break the moment folders let a
/// row move.
pub(crate) const RENAME_FIELD: &str = "hani-body-rename";

/// The field `base` makes at `placement`, by the route the placement takes:
/// a body's under the gizmo, and each of a folder's (`app/group_gizmo.rs`).
fn bake_placed(
    base: &hani_core::Volume,
    placement: hani_core::Similarity,
    route: hani_core::Bake,
) -> hani_core::Volume {
    match route {
        // Unreachable: the identity route returns above, having put the
        // base back rather than deep copying it. Left as the harmless
        // answer rather than a panic, because the cost of being wrong here
        // is one wasted copy and the cost of a panic is the sculpt.
        hani_core::Bake::Identity => base.shifted(glam::IVec3::ZERO),
        // Turn first, then carry it to where the pivot asked for. Both are
        // exact; see `Similarity::route`, which works out why the shift is
        // not simply the translation in voxels.
        hani_core::Bake::Exact { turns, voxel_offset } => {
            let turned = if turns.is_identity() { None } else { Some(base.rotated(turns)) };
            match turned {
                Some(turned) => turned.shifted(voxel_offset),
                None => base.shifted(voxel_offset),
            }
        }
        hani_core::Bake::Resample => {
            let mut warped = base.warped(placement);
            // **A per-axis scale leaves a field that is no longer a
            // distance field, and this is where that is repaired.**
            // `Volume::warped` multiplies the sampled distance by
            // `min_scale`, which keeps the zero set exact and
            // underestimates everywhere else -- sound for a sphere trace,
            // wrong for everything that reads a gradient. The drift is
            // `s_min/s_max` and it would compound across bakes with
            // nothing to reset it, which is the whole reason
            // `similarity.rs` refused a per-axis scale until this pass
            // existed.
            //
            // Run on EVERY resample, uniform scale included. The tempting
            // reading is that a uniform scale leaves the field exactly the
            // transformed solid and so needs nothing -- that is true of the
            // measured distances and false of the band. `Volume::warped`
            // writes the saturation plateau back at the band edge rather
            // than scaling it, which is a deliberate cliff, and this pass
            // is what re-solves the band outward from the interface and
            // removes it. Skipping it for a uniform scale left one drag of
            // the centre handle with a field whose plateau had been carried
            // from 3.0 voxels down to 0.15 and nothing to put it back.
            //
            // Costing nothing when it is not needed is `redistance`'s own
            // job: it gates on measured drift and returns `None` after one
            // read-only sweep, which is the cost the rotation-only case
            // already accepts a few lines above.
            warped.redistance();
            warped
        }
    }
}

/// Whether a message leaves a rename in flight alone, or ends it.
///
/// **The list is inverted on purpose, and that is the whole reason this is a
/// function rather than a call at each site that ends a rename.** Committing
/// keeps what the user typed and closing without committing throws it away, so
/// the safe default for a message nobody thought about is to commit. Written
/// the other way round -- a list of messages that DO commit -- a message added
/// next year would default to leaving a field open over a document that has
/// moved on underneath it, and nothing would fail.
///
/// The five that keep it open:
///
/// * the field's own keystrokes, which are the rename;
/// * every key press, because Escape is captured by the focused field itself
///   (`text_input.rs:1235-1244`) and only reaches the application once the
///   field has already been blurred -- at which point it must still REVERT,
///   and a commit here would have eaten it first. Keeping every key press
///   rather than picking Escape out of them keeps the "no key is decoded
///   outside `viewport::shortcut`" rule intact;
/// * `MenuClosed`, which is what Escape becomes and which reverts;
/// * the frame tick and a pointer merely moving. A pointer PRESS is the user
///   leaving, and does commit;
/// * the two gesture-END messages, and this one is not symmetry -- it is a
///   real bug's fix. A press is captured by the focused field, but a RELEASE
///   is not: iced 0.14 captures `KeyReleased` for the literal `v` and nothing
///   else (`text_input.rs:1249-1261`), so releasing the `s`, `u` or `h` of a
///   body called "Head" or "Skull" arrives here Ignored, decoded by
///   [`key_event`] into [`Message::SizingEnded`] or
///   [`Message::MaskPeekEnded`], and the default committed the half-typed
///   name and closed the field. Both messages clear one field of session
///   state and by construction touch no document, so keeping the rename open
///   across them is unconditionally right rather than a lesser evil.
///
/// Note which press that is: `Message::Pointer(Pressed)` is raised only for a
/// press INSIDE the viewport (`viewport::route_pointer` bounds-checks it), so
/// it is not the whole of "the user clicked away". A press that landed on
/// nothing -- the empty scrollable under the last body row, the panel
/// background -- arrives as [`Message::PressedNothing`], which is on no list
/// here and therefore commits. That message exists for exactly this, because
/// the field has already blurred itself by then.
/// Whether a message belongs to a gizmo session, or ends it.
///
/// The allowlist is deliberately short and deliberately errs toward COMMITTING:
/// a message wrongly on this list acts on a document whose last change is not
/// yet on the undo stack, and a message wrongly off it costs the user one extra
/// press of `w`. Those two mistakes are not the same size.
///
/// `KeyPressed` is on it because every keyboard shortcut arrives as one first
/// and is re-dispatched as the message it means -- which is then judged here on
/// its own merits. Escape's `MenuClosed` is on it because Escape is how a
/// session is ABANDONED, and committing on the way past would be the opposite
/// of what was pressed.
fn keeps_the_gizmo_armed(message: &Message) -> bool {
    match message {
        // The gesture itself, and the camera moves that happen around it.
        Message::Pointer(_) | Message::Frame => true,
        Message::KeyPressed { .. } | Message::MenuClosed => true,
        Message::ToolChanged(_) => true,
        Message::SizingEnded | Message::MaskPeekEnded => true,
        // Framing the active body is how a user gets back to what they are
        // placing; it changes no document state at all.
        Message::CameraFramedOnActive => true,
        _ => false,
    }
}

/// Whether a message is one of the welcome screen's own actions, and so
/// dismisses it.
///
/// **It is a starting point, not a mode.** Pressing New sculpt has answered the
/// question the screen was asking -- which is why SindriCAD's buttons call
/// `close()` before their action rather than after. It is not only tidiness:
/// the welcome card ranks above every other card, so leaving it up draws the
/// unsaved-work prompt that `New sculpt` raises UNDERNEATH it, raised,
/// unanswerable and invisible.
///
/// **Asked as "what closes it" and NOT as "what keeps it open", which is the
/// opposite of the gizmo's guard beside it.** That one denies by default
/// because an armed gizmo holds a gesture that must not survive an unrelated
/// message. This holds nothing, so the two failure directions are not
/// symmetric: a screen that lingers one message too long is a click, and a
/// screen that never appears is invisible. Written the other way round it was
/// dismissed before it could be seen -- by a layout message the window emits
/// as it lays itself out (the timeline's resize, when there was one), which no
/// allow-list would have thought to name.
///
/// The list is knowable and complete by construction: the card is `opaque`, so
/// while it is up nothing else on screen can produce a message at all, and
/// these are exactly the controls it offers.
fn dismisses_the_welcome(message: &Message) -> bool {
    matches!(
        message,
        Message::WelcomeClosed
            | Message::NewSculpt
            | Message::OpenRequested
            | Message::ImportRequested
            | Message::OpenRecent(_)
            // The sculpts a browser keeps. See `app/shelf.rs`.
            | Message::Shelf(crate::message::ShelfMessage::Opened)
    )
}

/// What the File menu's entries send. Choosing one is done with the menu.
fn closes_the_file_menu(message: &Message) -> bool {
    matches!(
        message,
        Message::NewSculpt
            | Message::OpenRequested
            | Message::ImportRequested
            | Message::OpenRecent(_)
            | Message::RecoverAutosave
            | Message::SaveRequested
            | Message::SaveAsRequested
            | Message::ExportRequested(_)
            | Message::OpenInSlicer
            | Message::PrinterChecked
            | Message::Shelf(crate::message::ShelfMessage::ToComputer)
    )
}

fn keeps_the_rename_open(message: &Message) -> bool {
    match message {
        Message::BodyRenameEdited(_) => true,
        Message::KeyPressed { .. } | Message::MenuClosed => true,
        Message::SizingEnded | Message::MaskPeekEnded => true,
        Message::Frame => true,
        Message::Pointer(event) => !matches!(event, PointerEvent::Pressed { .. }),
        _ => false,
    }
}

impl Hani {
    /// The application, and whatever it must do before the first frame.
    ///
    /// **Returns a `Task` because iced takes one**: `IntoBoot` is implemented
    /// for `(State, Task<Message>)`, so there is a place in construction to
    /// start work from. The first version of this claimed otherwise and put a
    /// guard in `update` instead, which meant re-entering the one function
    /// every message passes through -- so every other guard in it had to be
    /// safe against running twice, to serve one request at startup.
    pub fn new() -> (Self, Task<Message>) {
        let mut app = Self::with_devices(Tablet::start(), SpaceMouse::start());
        // The keys a user set, from the settings directory. Not in
        // `with_devices`, which the tests build through.
        app.keys = keys::Keys::load();
        // **Read here and not in `with_devices`, which is what the tests
        // build through.** The welcome screen is modal, so an application
        // constructed with it up has every keyboard and pointer path blocked by
        // `modal_open` -- and a test that built one would silently stop
        // sculpting. Worse, it would do so depending on whether the developer
        // running the suite happens to have dismissed the screen, which is a
        // test outcome that varies by machine.
        //
        // Set last, so it opens over a document that is already whole: it ranks
        // above every other card, and anything raised during construction would
        // be drawn underneath it and be unreachable.
        app.welcome_on_startup = crate::welcome::on_startup();
        app.welcome = app.welcome_on_startup;
        // The palette, read here for the reason the welcome flag is -- a suite that read
        // the developer's own `filaments.conf` would assert against whatever
        // happens to be loaded in their printer.
        app.palette = crate::palette::Palette::load();
        // And the brush presets, for it a third time.
        app.studio.presets = presets::Presets::load();
        // And the interface's size: `app/ui_scale.rs`.
        app.ui_scale = ui_scale::UiScale::kept();
        // The maps that have been taken in: `app/maps.rs`.
        app.inserts.maps = maps::kept(maps::directory().as_deref());
        // And the brush pictures: `app/brush_pictures.rs`.
        app.studio.own = brush_pictures::kept(brush_pictures::directory().as_deref());
        // A recording or a replay asked for by the environment, after the
        // welcome flag it overrides: `app/replay.rs`.
        app.replay_from_environment();
        let mut boot = Vec::new();
        // **Asked whether the welcome screen is up or not**, unlike the
        // articles. The user who most needs to hear that their build is stale
        // is the one who turned the welcome screen off, and the answer goes to
        // the status line as well as the card.
        //
        // In `new` and not in `with_devices`, for the same reason as the two
        // above: `with_devices` is what the tests build through, and a suite
        // that reached out to GitHub on every construction would be slow,
        // rate limited, and would fail on a machine with no network.
        //
        // Read here rather than inside the task so the file access is on this
        // thread with the other startup reads, and so a test that ever does
        // reach `new` fails loudly rather than racing a background reader.
        // Resolved here and not in `with_devices`, for the same reason the
        // update check is: `with_devices` is what ~640 tests build through, and
        // a suite that resolved the test binary's own path would be one
        // mistake away from a test that swaps it.
        app.exe = crate::update::apply::Target::resolve().ok();
        // Sweep whatever a killed attempt left beside the binary before doing
        // anything else, so a failed update does not leak a 33 MB inode for ever.
        if let Some(target) = &app.exe {
            crate::update::apply::sweep(&target.directory);
        }
        // **Did the last update start?** Everything here is code inside the
        // NEW build, so it only ever runs for a payload that starts and then
        // dies. A payload that cannot `exec` at all runs none of it, which is
        // what `RECOVER-HANISCULPT.txt` beside the binary is for.
        if let Some(target) = &app.exe
            && let Some(marker) = crate::update::apply::marker_path(target)
            && let Ok(text) = std::fs::read_to_string(&marker)
            && let Some(pending) = crate::update::apply::marker_from(&text)
        {
            if pending.attempt == 0 {
                // First launch after the swap. Record the attempt and let the
                // frame tick clear it.
                let _ = crate::update::apply::write_marker(
                    &marker,
                    &crate::update::apply::Pending { attempt: 1, ..pending.clone() },
                );
                app.update_pending_marker = Some(marker);
                // The document the user had open, back where they left it.
                if let Some(resume) = &pending.resume {
                    app.open_project(resume);
                }
            } else {
                // The previous launch died before it drew anything.
                let recorded = crate::paths::state_file("update.state")
                    .and_then(|path| std::fs::read_to_string(path).ok())
                    .and_then(|text| crate::update::previous_sha256_from(&text));
                match recorded {
                    Some(digest) => match crate::update::apply::revert(target, &digest) {
                        Ok(()) => {
                            // Stop revert -> notify -> update -> crash -> revert
                            // becoming a loop with a network fetch per cycle.
                            app.update_settings.skip_build = Some(pending.build);
                            crate::update::store(&app.update_settings);
                            app.status = format!(
                                "build {} would not start, so the build you had is back",
                                pending.build
                            );
                        }
                        Err(why) => app.status = why.to_string(),
                    },
                    None => {
                        app.status = format!(
                            "build {} would not start, and there is no verified copy to go back to",
                            pending.build
                        );
                    }
                }
                let _ = std::fs::remove_file(&marker);
            }
        }
        // **The failure the automatic revert structurally cannot catch.**
        //
        // The marker is cleared on the first drawn frame, so a build that starts
        // fine and then dies -- opening the user's document, on a second GPU
        // init, on one particular tablet -- legitimately cleared its own marker
        // and will never be reverted automatically. No amount of keeping `.old`
        // longer changes that; the signal has to be the crash report, which
        // already exists and was not being used.
        //
        // **Offered, not performed.** The application drew a frame, the user is
        // at a window, and they can decide. The automatic path exists precisely
        // for the user who never got one.
        //
        // Decided HERE and not beside the crash notice that sets the flag: this
        // needs `app.exe`, and `new` resolves that after `with_devices` has
        // already returned. An earlier revision made the decision there, where
        // `exe` is unconditionally `None`, so the offer could never appear in
        // any build -- correct logic whose caller never ran, which is the same
        // shape as the anti-rollback floor that never advanced.
        if app.crashed_last_session
            && let Some(target) = &app.exe
            && crate::update::apply::target_is_the_installed_build(target)
            && std::fs::metadata(target.old()).is_ok_and(|old| old.is_file())
        {
            app.crash_revert = crate::update::installed_build();
        }

        let settings = crate::update::settings();
        app.update_settings = settings.clone();
        let floor = crate::update::floor();
        let running = build_number();
        let welcome_up = app.welcome;
        if settings.when != crate::update::When::Never
            && (settings.when == crate::update::When::Always || welcome_up)
        {
            boot.push(Task::perform(
                async move { crate::update::check(running, &settings, floor).ok().flatten() },
                Message::UpdateChecked,
            ));
        }
        (app, Task::batch(boot))
    }

    /// Build with a given pressure source and an inert puck, which is what
    /// almost every test wants.
    ///
    /// Points the recent list and the crash net at a per-test temporary
    /// directory. This is not tidiness: the defaults are the real
    /// `~/.config/hanisculpt/recent` and the real autosave, and `save_project`
    /// both records into one and deletes the other -- so a plain `cargo test`
    /// would rewrite someone's recent list and destroy a crash net belonging to
    /// a live session.
    #[cfg(test)]
    fn with_tablet(tablet: Tablet) -> Self {
        let mut app = Self::with_devices(tablet, SpaceMouse::inert());
        let scratch = std::env::temp_dir().join(format!(
            "hani-test-state-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        app.recent = crate::recent::Recent::load_from(Some(scratch.join("recent")));
        app.autosave_file = Some(scratch.join("autosave.hani"));
        // `with_devices` has already looked for a crash net, and at that point
        // it was still looking at the real one -- so on a machine that has an
        // autosave the status line would arrive pre-filled and every test
        // asserting on it would fail for a reason outside its own scope.
        app.status = String::new();
        app
    }

    /// Build with given input devices, so tests do not go looking through
    /// `/dev/input` and spawn reader threads.
    fn with_devices(tablet: Tablet, spacemouse: SpaceMouse) -> Self {
        let shared = SharedFrame::new();
        let mut volume = Volume::new(VOXEL_SIZE_MM);
        volume.seed_sphere(Vec3::ZERO, MODEL_RADIUS_MM);
        // Everything the sphere touches plus a one brick margin, because bricks
        // with no voxels of their own still own the quads on their low faces.
        volume.mark_everything_dirty();

        let doc = Document::from_volume(volume);
        let mut app = Self {
            model_radius_body: doc.active(),
            offer: None,
            // Default rather than a read: `with_devices` is what the tests
            // build through, and a suite that picked up the config of whoever
            // ran it would pass or fail by machine. `new` overwrites this.
            update_settings: crate::update::Settings::default(),
            downloading_update: false,
            installing_update: false,
            update_failure: None,
            downloaded_update: None,
            exe: None,
            update_pending_marker: None,
            crash_revert: None,
            crashed_last_session: false,
            doc,
            camera: OrbitCamera::framing(Vec3::ZERO, MODEL_RADIUS_MM),
            brush: Brush::default(),
            tablet,
            spacemouse,
            pressure_enabled: true,
            pressure_curve: 1.0,
            tilt_enabled: true,
            stroke: Stroke::new(),
            history: History::default(),
            shared,
            mesh_buffers: Vec::new(),
            brush_scratch: BrushScratch::new(),
            mask_stroke: hani_core::mask_stroke::MaskStroke::new(),
            strengths: {
                let mut seeded = [MAX_MASK_STRENGTH; MASK_STRENGTH_SLOT + 1];
                seeded[..MASK_STRENGTH_SLOT]
                    .copy_from_slice(&BrushKind::ALL.map(BrushKind::default_strength));
                seeded
            },
            move_grab: None,
            move_stroke: MoveStroke::new(),
            stamp_centres: Vec::new(),
            cut_path: Vec::new(),
            dirty: Vec::new(),
            shown: Vec::new(),
            hidden_bodies: Vec::new(),
            opposite_polarity: Vec::new(),
            mask_card: None,
            solo: None,
            drag: None,
            cursor: None,
            viewport_size: Vec2::new(1280.0, 720.0),
            shift: false,
            control: false,
            alt: false,
            perf: Perf::default(),
            phases: phases::Phases::default(),
            replay: replay::Replay::default(),
            doc_stats: VolumeStats::default(),
            measures_stale: false,
            moves: Vec::new(),
            pen: pen::PenTrail::default(),
            batching: false,
            history_stats: HistoryStats::default(),
            status: String::new(),
            expanded: PanelSection::ALL.map(PanelSection::open_by_default),
            stats_open: false,
            overlay: hani_gpu::OverlayBatch::default(),
            hover: None,
            hover_body: None,
            sizing: None,
            dynamic_radius: false,
            model_radius: MODEL_RADIUS_MM,
            working_size_field: String::new(),
            detail_advice: String::new(),
            breadcrumbs: crate::breadcrumbs::Breadcrumbs::new(),
            crumbed_status: String::new(),
            bug_report: None,
            facts_recorded: false,
            cube: hani_gpu::OverlayBatch::default(),
            gizmo_batch: hani_gpu::OverlayBatch::default(),
            gizmo: None,
            gizmo_base: None,
            gizmo_baked: None,
            group_gizmo: Default::default(),
            symmetry_drag: None,
            soft_drag: Default::default(),
            welcome: false,
            preferences: false,
            rail_last_brush: None,
            desk: workspace::Desk::default(),
            keeping: false,
            keys: keys::Keys::default(),
            key_menu: None,
            shortcuts: false,
            help: help::HelpState::default(),
            about_open: false,
            graphics: None,
            touches: touch::Touches::default(),
            kept: kept_settings::KeptSettings::read(),
            shelf: shelf::ShelfState::default(),
            snip: snip::SnipState::default(),
            welcome_on_startup: true,
            palette: crate::palette::Palette::default(),
            cube_hover: None,
            flight: None,
            menu: None,
            cube_menu: None,
            orient_prompt: None,
            top_menu: None,
            project_path: None,
            unsaved: false,
            confirm: None,
            tool: Tool::Sculpt,
            recent: crate::recent::Recent::load(),
            autosave_file: Self::default_autosave_path(),
            last_autosave: Instant::now(),
            autosave: autosave::Autosave::default(),
            last_activity: Instant::now(),
            drafts: crate::ui::Drafts::new(),
            // ON, which is what Thomas asked for, and the bench now supports
            // it: 8.03, 7.89 and 7.88 ms across three runs on a quiet machine
            // against an 8.0 ms budget, so the median passes and the one
            // failure is 0.4% over.
            //
            // The number is also a WORST CASE rather than a typical one. It is
            // extrapolated to 45,567 bricks -- the largest model this pool can
            // hold -- while the bench's own model, 5,435 bricks, costs 0.94 to
            // 0.96 ms. Ordinary work is nowhere near the budget.
            //
            // An earlier reading of this defaulted it OFF on the grounds that
            // the bench measures a SCULPTED sphere while the application is
            // "really" for imported scans, which carry several times more
            // geometry per brick. That premise was wrong: this is a sculpting
            // tool first and scan import is one thing it does, so the sculpted
            // anchor is the representative case, not the flattering one. The
            // switch in the panel covers the heavy import.
            thumbnails: true,
            show_mask: true,
            show_paint: true,
            show_cavity: false,
            export_detail: ExportDetail::default(),
            key_spec: hani_core::part::KeySpec::peg(),
            touch: paints::TouchState::default(),
            flat_paint: false,
            studio: studio::StudioState::default(),
            measure: measure::MeasureState::default(),
            bench: None,
            inserts: adding::AddState::default(),
            form: form::FormState::default(),
            revolve: revolve::RevolveState::default(),
            deform: deform::DeformState::default(),
            references: hani_core::reference::References::default(),
            reference_view: references::ReferencesState::default(),
            kept_as: keeping::KeepingState::default(),
            ui_scale: ui_scale::UiScale::default(),
            hollow: hollow::HollowState::default(),
            overhang: overhang::OverhangState::default(),
            painting: painting::PaintingState::default(),
            cores: cores::CoresState::default(),
            ring_cut: ring_cut::RingCut::default(),
            explode: explode::Explode::default(),
            dry_preview: dry_preview::DryPreview::default(),
            texture_size: paints::TEXTURE_SIZES[1],
            mask_tint: DEFAULT_MASK_TINT,
            mask_peek: false,
            mask_filter: MaskFilter::Blur,
            mask_amount: 0.0,
            mask_grab: None,
            mask_feature_mm: DEFAULT_MASK_FEATURE_MM,
            mask_thickness_voxels: DEFAULT_MASK_THICKNESS_VOXELS,
            thumbs: crate::thumbnails::Thumbnails::new(),
            adding: false,
            pending_delete: None,
            delete_prompt_bytes: hani_core::DEFAULT_RECLAIM_BUDGET,
            pending_merge: None,
            merge_prompt_bytes: hani_core::DEFAULT_RECLAIM_BUDGET,
            pending_split: None,
            renaming: None,
            row_drag: None,
        };
        app.remesh_dirty();
        // Otherwise the overlay reports a zero byte budget until the first
        // stroke happens to refresh it.
        app.history_stats = app.history.stats();
        app.perf.load_ms = app.perf.remesh_ms;
        app.perf.remesh_ms = 0.0;
        app.perf.dirty_bricks = 0;
        // Before the first publish, so the camera's floor and far plane are in
        // the document's units from the first frame rather than from the
        // defaults `framing` leaves behind.
        app.refresh_camera_lattice();
        app.publish_camera();
        app.refresh_detail_advice();
        app.refresh_overlay();
        // Before the first message, so the renderer is never acting on an empty
        // hidden set it was never given.
        app.publish_visibility();
        // A crash net left by a previous session is worth nothing if nobody
        // knows it is there, and the File menu is not somewhere you look
        // unprompted.
        if app.has_autosave() {
            // Checked BEFORE the offer, so nobody is invited to recover a file
            // this build cannot open -- and, far more importantly, before the
            // 120-second autosave timer gets a chance to overwrite it.
            app.status = match app.preserve_unreadable_autosave() {
                Some(aside) => format!(
                    "an autosave from a previous session could not be read by this build — \
                     it has been kept as {}",
                    aside.display()
                ),
                None => "an autosave from a previous session is in File > Recover".to_string(),
            };
        }
        // **And a crash outranks it**, because it is the rarer message and the
        // one with something to do about it. Taken rather than read, so it is
        // said once; see `crash::take_pending`.
        if let Some(report) = crate::crash::take_pending() {
            app.status =
                "the last session crashed — Help > Report a bug opens an issue for it".to_string();
            // The path goes to the log rather than the status line: it is long
            // enough to wrap the title bar onto two lines, and it is the one
            // part of the message a user does not need in order to act.
            //
            // The summary is found by looking for the `message:` heading and
            // not by counting lines. Counting picked line five, which is
            // `os: linux` -- a summary that is the same on every crash, which
            // is worse than none.
            let summary = report
                .lines()
                .skip_while(|line| !line.starts_with("message:"))
                .nth(1)
                .unwrap_or("(no message)")
                .trim();
            match crate::crash::report_path() {
                Some(path) => {
                    log::warn!(
                        "the previous session crashed: {summary} (report in {})",
                        path.display()
                    )
                }
                None => log::warn!("the previous session crashed: {summary}"),
            }

            // A crash report was pending. Whether that becomes an OFFER to go
            // back is decided in `new`, because it needs `exe`, which is not
            // resolved until then -- see `crashed_last_session`.
            app.crashed_last_session = true;
        }
        app
    }

    /// Take the history's counters, and say out loud when an eviction has taken
    /// a deleted body with it.
    ///
    /// Entries are only ever dropped by a push, so this belongs at the push
    /// sites and nowhere else. The reason a body gets a status line when a
    /// dropped stroke gets only a counter in the stats readout: a dropped
    /// stroke costs the user a redo they did not ask for, and a dropped body
    /// costs them the body. Two folder deletes that each pass the reclaim
    /// allowance on their own and then evict each other are the case no
    /// per-operation prompt can catch, which is why the eviction has to be the
    /// one that speaks.
    fn record_history(&mut self, before: HistoryStats) {
        self.history_stats = self.history.stats();
        if self.history_stats.dropped_bodies > before.dropped_bodies {
            self.status =
                "undo history is full — a deleted body can no longer be brought back".to_string();
        }
    }

    /// Undo and redo both count as changes.
    ///
    /// Undoing back to exactly the state that was last saved leaves the flag
    /// set, so the prompt on the way out is a false positive. Telling that case
    /// apart needs a save point recorded in the history, and the history is
    /// cleared by open, resample and reset, so the marker would have to be
    /// invalidated in three more places. Over-prompting costs a click;
    /// under-prompting costs the sculpt.
    fn undo(&mut self) {
        // A drag of Radial copies still open is a step first. See
        // `app/symmetry_frame.rs`.
        self.finish_symmetry_drag();
        let shown = self.saved_nodes();
        let before = self.body_ids();
        match self.history.undo(&mut self.doc, &shown) {
            UndoOutcome::Applied(_) => self.after_history_step(&before),
            UndoOutcome::Refused(node) => self.refuse_history_step("undo", node),
            UndoOutcome::Nothing => {}
        }
    }

    fn redo(&mut self) {
        // See `app/symmetry_frame.rs`.
        self.finish_symmetry_drag();
        let shown = self.saved_nodes();
        let before = self.body_ids();
        match self.history.redo(&mut self.doc, &shown) {
            UndoOutcome::Applied(_) => self.after_history_step(&before),
            UndoOutcome::Refused(node) => self.refuse_history_step("redo", node),
            UndoOutcome::Nothing => {}
        }
    }

    /// Every body the document currently holds, for comparing across a history
    /// step. Bodies only -- a folder owns no slots in the mesh pool.
    fn body_ids(&self) -> Vec<NodeId> {
        self.doc.nodes().iter().filter(|node| node.is_body()).map(|node| node.id).collect()
    }

    /// Which rows the FILE would keep, indexed by node position.
    ///
    /// Undo and redo are the two callers, and they use the saved answer rather
    /// than the drawn one on purpose. **Solo must never veto a structural
    /// operation.** The refusal these feed -- "undo would change X, which is
    /// hidden" -- is about an eye the user set and can see in the panel; a
    /// transient view mode turning ctrl+Z off, with a message calling a body
    /// "hidden" whose eye is plainly open, is a different thing wearing the
    /// same words.
    ///
    /// This is [`Document::saved_visibility`], and it is the allocating wrapper
    /// the keystroke paths want: both are gestures rather than frames. The
    /// resolver fills a buffer the application keeps, which is what increment
    /// 6's GPU mask needs and what this must not become.
    fn saved_nodes(&self) -> Vec<bool> {
        let mut shown = Vec::new();
        self.doc.saved_visibility(&mut shown);
        shown
    }

    /// What undo and redo both do once something has actually moved.
    ///
    /// `refresh_overlay` is in here because the brush ring and the mirror
    /// planes are built from the field: sixteen other sites already refresh
    /// them, and undo not doing so has been a staleness bug the whole time
    /// there has been one body to see it on.
    fn after_history_step(&mut self, bodies_before: &[NodeId]) {
        // **A body the step REMOVED has to be forgotten, or it is drawn for
        // ever.** Undoing an added primitive applies `Change::NodeAdded`, which
        // takes the node back out of the document -- and its bricks are then in
        // nobody's dirty set, so `remesh_dirty` below never touches them and
        // the pool goes on drawing a body the BODIES list no longer shows.
        // Reported from the running application: add a primitive, ctrl+Z, and
        // the cube leaves the list and stays on screen.
        //
        // `remove_body` has always done this; the history had no equivalent
        // because `UndoOutcome::Applied` names only the FIRST node an entry
        // touched, which is not the same question.
        //
        // **Diffed against the document rather than read off the `Change`s**,
        // deliberately. A match on `Change::NodeAdded` would cover the reported
        // case and quietly miss redo-of-a-delete, an entry mixing removals with
        // other changes, and whatever the next variant is; the diff is a
        // function of what actually happened and cannot drift from the enum.
        // It is a walk over at most 128 nodes, once per keystroke.
        let mut forgot = false;
        for id in bodies_before {
            if self.doc.node(*id).is_none() {
                self.shared.forget_body(*id);
                forgot = true;
            }
        }
        if forgot {
            // The other half, for the reason `remove_body` records: freeing
            // pool space without re-offering everything takes the pool-full
            // banner down while leaving the geometry it refused missing.
            self.doc.mark_everything_dirty();
        }

        self.history_stats = self.history.stats();
        self.unsaved = true;
        self.remesh_dirty();
        self.refresh_overlay();
        if forgot {
            self.refresh_model_radius();
        }
    }

    /// Say which body is in the way, and change nothing else.
    ///
    /// **Deliberately does not reveal or select it.** The eye bit is persisted,
    /// undoable document state, so writing it from inside undo would destroy a
    /// deliberate hide, set `unsaved` for a change the user never made, and
    /// still not reveal the body if an ancestor folder were the one hiding it.
    /// The user reveals it and presses again; the entry is untouched and costs
    /// nothing while it waits.
    fn refuse_history_step(&mut self, verb: &str, node: NodeId) {
        let name = self.doc.node(node).map_or("a hidden body", |node| node.name.as_str());
        self.status = format!("{verb} would change {name}, which is hidden");
        self.refresh_overlay();
    }

    /// Handle one message.
    ///
    /// Returns a [`Task`] because anything that touches the filesystem -- a
    /// dialog, a save, an export -- must not run here. The event loop is the
    /// same thread that draws, so a blocking call freezes the window, which is
    /// exactly the export freeze this replaces.
    ///
    /// **The visibility pass is here rather than inside the arms that change
    /// an eye, and that placement is the guarantee.** `dispatch` returns early
    /// from several arms, and the set of messages that can change what is drawn
    /// is not a list anyone can keep correct -- open, reset, import, undo, redo,
    /// a delete and a solo all belong to it, and increments 9 to 13 add more.
    /// Recomputing it once, after whatever the message did, cannot go out of
    /// date. It costs a walk of at most `MAX_NODES` rows and no allocation.
    ///
    /// **The rename commit is here for the same reason**, and it is the whole
    /// of what the plan calls "blur commits": iced 0.14's `text_input` has no
    /// blur event to hang it on -- it has `on_input`, `on_submit` and
    /// `on_paste` and nothing else (`text_input.rs:172-224`) -- so the only
    /// honest definition of blur is "the user did something that was not
    /// typing in the field", and that is a question about the message, asked
    /// once, before the message is acted on. See [`keeps_the_rename_open`].
    pub fn update(&mut self, message: Message) -> Task<Message> {
        // How a pen was held, which is no action and passes every guard
        // below. See `app/pen.rs`.
        if let Message::PenSample(state) = message {
            self.pen_sample(state);
            return Task::none();
        }
        // A number's slider and its field send what they set inside a
        // `FieldSet`, which says which draft to let go. Unwrapped first, so
        // every guard below meets the message it carries and not the wrapper.
        // See `ui.rs`.
        let message = match message {
            Message::FieldSet(key, carried) => {
                self.drafts.remove(key);
                *carried
            }
            message => message,
        };
        if self.renaming.is_some() && !keeps_the_rename_open(&message) {
            self.commit_rename();
        }
        // A browser warns before the page is left with work unsaved; on
        // the desktop this does nothing. See `app/shelf.rs`.
        self.warn_if_unsaved();
        // **A crash net being written in pieces is of the document as it
        // was when the first piece was made.** Anything but the frame tick
        // may be about to change it, so the write is given up before the
        // message is acted on and begun again at the next pause. See
        // `app/autosave.rs`.
        if self.autosave.in_flight() && !matches!(message, Message::Frame) {
            self.give_up_autosave();
        }
        // The pointer moves kept since the last frame come before whatever
        // this is, the frame tick included: they happened first. See
        // `pointer_in`.
        if !matches!(message, Message::Pointer(PointerEvent::Moved { .. })) {
            self.handle_moves();
        }
        // **The same pattern as the rename above, and it is here for a
        // stronger reason than tidiness.** While the gizmo is armed the
        // DOCUMENT already holds the baked field and the HISTORY does not yet
        // hold the entry that undoes it -- the entry is pushed once per
        // session, at disarm, because one per drag would be a whole field each
        // and thirty of them would evict every stroke behind them. So any
        // message that is not part of the gesture has to close the session
        // first, or it acts on a document whose last change is not on the
        // stack. Undo is the sharpest case and by no means the only one: save,
        // export, delete, resample and every future operation belong to the
        // same set, and that set is not a list anyone can keep correct.
        // Asking the question once, before the message is acted on, cannot go
        // out of date.
        // Before the gizmo guard and before dispatch, so that a card this
        // message goes on to raise is drawn over a screen that has already
        // gone rather than under one that has not.
        if self.welcome && dismisses_the_welcome(&message) {
            self.welcome = false;
        }
        // An entry of the File menu chosen closes the menu, which stayed
        // open over the viewport, and over the browser's file chooser.
        if self.top_menu.is_some() && closes_the_file_menu(&message) {
            self.top_menu = None;
        }
        // Whatever is about the document as something to keep finds the
        // document in its place and not a piece off the workbench. See
        // `app/workbench.rs`.
        if self.on_the_bench() && workbench::leaves_the_bench(&message) {
            self.leave_workbench();
        }
        if self.gizmo.is_some() && !keeps_the_gizmo_armed(&message) {
            self.disarm_gizmo(true);
        }
        let task = self.dispatch(message);
        // An insert put away is done with the shape it placed. See
        // `app/placed.rs`.
        self.finish_placed_when_put_away();
        // What the open mode does with a drag is the mode's: see
        // `app/workspace.rs`.
        let task = Task::batch([task, self.keep_to_the_mode()]);
        // **Visibility is published BEFORE the re-arm, and the order is load
        // bearing.** `arm_gizmo` refuses a body that is not drawn, and it asks
        // `active_is_drawn`, which indexes `self.shown` by the node's position
        // -- and `publish_visibility` is the only thing that writes it. Run the
        // other way round, the message that ADDS a body arms against a `shown`
        // that is one entry short, `get(index)` is `None`, `unwrap_or(false)`
        // says hidden, and adding a cube with the Transform tool up refused
        // with "that body is hidden -- turn its eye on to move it" and dropped
        // the user back to Sculpt. The body was in the list, visible, eye on.
        // Observed in the running application.
        self.publish_visibility();
        // And the gizmo follows the active body. The guard above committed
        // whatever was in flight; this re-arms on whatever is active NOW, so
        // choosing a different row while the tool is up moves the gizmo there
        // rather than leaving the mode lit with nothing under it.
        // Not while the bodies are drawn apart. See `app/explode.rs`.
        if self.tool == Tool::Transform
            && self.gizmo.is_none()
            && !self.exploded()
            && let Err(why) = self.arm_gizmo()
        {
            self.status = why;
            self.tool = Tool::Sculpt;
        }
        task
    }

    /// What one message actually does. See [`Hani::update`], which is the
    /// only caller and which is where the visibility pass lives.
    fn dispatch(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Pointer(event) => {
                // Into a recording, and into the frame's count. See
                // `app/replay.rs` and `app/phases.rs`.
                self.record_pointer(event);
                self.pointer_in(event);
            }
            Message::Frame => {
                let elapsed_ms = self.perf.record_frame();
                // A finger held still on the model. See `app/touch.rs`.
                self.touch_held();
                // A body made since is given the pooling. See `app/pooling.rs`.
                self.keep_pooling();
                // **The update took.** Cleared on the first drawn frame, which
                // is the earliest honest evidence that this build starts. A
                // build that dies before here leaves the marker behind, and the
                // next launch reverts. The cost is stated in the plan and is the
                // right way round: a session killed inside its first frame --
                // a broken driver, a headless run, a window closed instantly --
                // reverts a good update, and being stuck on an older build
                // beats being stuck with one that will not start.
                if let Some(marker) = self.update_pending_marker.take() {
                    let _ = std::fs::remove_file(marker);
                }
                if !self.facts_recorded {
                    self.facts_recorded = true;
                    self.record_session_facts();
                }
                self.record_status_change();
                if self.advance_flight(elapsed_ms) {
                    self.publish_camera();
                }
                // Explode's parts on their way apart or back. See
                // `app/explode.rs`.
                self.explode_frame(elapsed_ms);
                self.drive_from_spacemouse(elapsed_ms);
                // The airbrush, held, sprays by the frame. See
                // `app/airbrush.rs`.
                self.airbrush_frame();
                self.settle_an_orphaned_mask_grab();
                let timed = self.phases.enter(phases::Phase::Save);
                self.maybe_autosave();
                self.phases.leave(timed);
                self.request_a_thumbnail();
                // The frame's timings are closed off, and a replay takes its
                // next frame's worth. See `app/phases.rs` and `app/replay.rs`.
                self.note_a_stroke();
                let cost = self.shared.take_cost();
                self.phases.add(phases::Phase::Upload, cost.prepare_ms);
                self.phases.add(phases::Phase::Encode, cost.encode_ms);
                self.phases.count(|frame| {
                    frame.uploaded += cost.uploaded;
                    frame.uploaded_bytes += cost.uploaded_bytes;
                });
                self.phases.end_frame();
                // The monitor's size, once there is a window on one. See
                // `app/ui_scale.rs`.
                // And the compositor for the pen, as that is asked: once.
                // See `app/pen.rs`.
                return Task::batch([
                    self.ask_the_monitor(),
                    self.ask_for_the_pen(),
                    self.replay_frame(),
                ]);
            }
            Message::BrushKindChanged(kind) => {
                // Remember what the outgoing brush was set to, and restore what
                // this one was last on. While the MASK tool is live both sides
                // resolve to the mask's own slot, so choosing a different brush
                // mid-mask leaves the mask strength exactly where the user put
                // it -- which is the whole point of the mask having a slot.
                self.swap_strength(|app| app.brush.kind = kind);
                // A classic brush chosen is the classic brush wanted.
                self.studio.put_away();
                // And it is the one in hand as. See `app/keeping.rs`.
                self.kept_as.from = None;
                self.inserts.put_away();
            }
            Message::FalloffChanged(curve) => {
                self.brush.falloff = curve;
                // One of upstream's curves, and not the studio's own shape.
                self.studio.falloff_chosen();
            }
            // Clamped rather than trusted. The slider's own range is already
            // voxel-aware, but a message can arrive from a typed field or the
            // right-click menu too, and at a resin lattice an unclamped 20 mm
            // is a quarter of a second per stamp.
            Message::BrushRadiusChanged(radius) => {
                self.brush.radius = radius.clamp(MIN_RADIUS_MM, self.max_radius())
            }
            Message::BrushStrengthChanged(strength) => {
                self.brush.strength = strength;
                let slot = self.live_strength_slot();
                self.strengths[slot] = strength;
            }
            // Cards of the menus: each takes the menu it was chosen from
            // away, as the welcome screen does below.
            Message::RailChosen(which) => return self.rail_chosen(which),
            Message::WorkspaceChosen(which) => return self.enter_workspace(which),
            Message::WorkspaceBack => return self.enter_workspace(self.desk.last),
            Message::DockTabChosen(tab) => self.desk.memory[self.desk.now as usize].tab = tab,
            Message::LibraryKey(index) => return self.library_key(index),
            Message::KeyMenuOpened(place) => self.key_menu = Some(place),
            Message::KeyMenuClosed => self.key_menu = None,
            Message::KeyPut(key) => self.put_on_key(key),
            Message::PreferencesOpened => {
                self.top_menu = None;
                self.preferences = true;
            }
            Message::PreferencesClosed => self.preferences = false,
            Message::ShortcutsOpened => {
                self.top_menu = None;
                self.shortcuts = true;
            }
            Message::ShortcutsClosed => self.shortcuts = false,
            // See `app/help.rs`.
            Message::Help(message) => self.help_changed(message),
            Message::About(open) => return self.about_changed(open),
            // See `app/booleans.rs`.
            Message::Boolean(message) => self.boolean_changed(message),
            Message::GraphicsNamed(name) => self.graphics = Some(name),
            Message::Kept(message) => self.kept_changed(message),
            Message::Shelf(message) => return self.shelf_changed(message),
            Message::Snip(message) => return self.snip_changed(message),
            Message::WelcomeOpened => {
                // The menu it was chosen from has to go, or it is left drawn
                // beneath the scrim: reachable to nothing, and still there when
                // the screen is dismissed.
                self.top_menu = None;
                self.welcome = true;
            }
            Message::UpdateChecked(answer) => {
                // Also to the status line, and ONLY when the card is not up.
                // The card carries the same notice with a button on it, so
                // saying it twice would be noise -- and the person who needs
                // the status line is exactly the one who turned the card off.
                //
                // `status.is_empty()` guards it because the startup crash and
                // autosave notices are already in the line by now, and those
                // have something for the user to DO. An update is the least
                // urgent thing that can be said here.
                if let Some(offer) = &answer
                    && !self.welcome
                    && self.status.is_empty()
                {
                    self.status = format!("{} -- {}", offer.headline(), crate::links::RELEASE_PAGE);
                }
                self.offer = answer;
            }
            Message::LinkOpened(link) => {
                // The screen stays up: reading the release page is not
                // choosing what to do with the application, and coming back to
                // a dismissed welcome screen would be a surprise.
                if let Err(why) = crate::links::open_in_browser(&link) {
                    self.status = why;
                }
            }
            // Nothing to do: `dismisses_the_welcome` already cleared the flag
            // before dispatch, which is what closes it for every other button
            // on the card too.
            Message::WelcomeClosed => {}
            Message::UpdateDownloadRequested => {
                if self.downloading_update {
                    return Task::none();
                }
                let Some(offer) = self.offer.clone() else {
                    return Task::none();
                };
                // A build that cannot be reached by replacing one file is not
                // downloadable here at all: the payload is a bare executable and
                // that user needs the full archive. Say so and send them to the
                // page rather than handing them something that will not work.
                if offer.requires_reinstall || offer.payload.is_none() {
                    self.status = format!(
                        "build {} needs a full download -- {}",
                        offer.build,
                        crate::links::RELEASE_PAGE
                    );
                    return Task::none();
                }
                // **Beside the binary when we can replace it, in the state
                // directory when we cannot.** `install` ends with
                // `rename(staged, target)`, and a cross-filesystem rename
                // degrades to a copy -- exactly the operation that fails against
                // a running image. So the destination is decided by whether this
                // install is replaceable at all, and the gates answer that
                // BEFORE 33 MB is transferred rather than after.
                let replaceable = self
                    .exe
                    .as_ref()
                    .filter(|target| {
                        crate::update::apply::gates(target, build_number(), build_commit()).is_ok()
                    })
                    .map(|target| target.staging_directory());
                let Some(into) = replaceable.or_else(crate::update::download_directory) else {
                    self.status = "there is nowhere to put the download".to_string();
                    return Task::none();
                };
                self.downloading_update = true;
                self.status = format!("downloading build {}…", offer.build);
                return Task::perform(
                    async move { crate::update::download(&offer, &into).map_err(|why| why.to_string()) },
                    Message::UpdateDownloaded,
                );
            }
            Message::UpdateDownloaded(answer) => {
                self.downloading_update = false;
                match answer {
                    // **Where it is, not just that it worked.** A user handed a
                    // verified file and not told where it landed has been given
                    // nothing they can act on, and there is no terminal here to
                    // go looking with.
                    Ok(path) => {
                        self.status = format!("verified and saved to {}", path.display());
                        self.downloaded_update = Some(path);
                        // A download that worked answers the previous refusal.
                        self.update_failure = None;
                        crate::update::journal::ok(
                            crate::update::journal::Step::Download,
                            self.offer.as_ref().map(|o| o.build),
                        );
                    }
                    Err(why) => self.update_says(crate::update::journal::Step::Download, why),
                }
            }
            Message::UpdateRevertRequested => {
                let Some(build) = self.crash_revert.take() else {
                    return Task::none();
                };
                let Some(target) = self.exe.clone() else {
                    return Task::none();
                };
                let recorded = crate::paths::state_file("update.state")
                    .and_then(|path| std::fs::read_to_string(path).ok())
                    .and_then(|text| crate::update::previous_sha256_from(&text));
                let Some(digest) = recorded else {
                    self.update_says_of(
                        crate::update::journal::Step::Revert,
                        Some(build),
                        "there is no verified copy to go back to".to_string(),
                    );
                    return Task::none();
                };
                match crate::update::apply::revert(&target, &digest) {
                    Ok(()) => {
                        // Stop revert -> notify -> update -> crash -> revert
                        // becoming a loop with a network fetch per cycle.
                        self.update_settings.skip_build = Some(build);
                        crate::update::store(&self.update_settings);
                        crate::update::journal::ok(
                            crate::update::journal::Step::Revert,
                            Some(build),
                        );
                        self.status =
                            format!("went back from build {build} — restart HaniSculpt to use it");
                    }
                    Err(why) => self.update_says_of(
                        crate::update::journal::Step::Revert,
                        Some(build),
                        why.to_string(),
                    ),
                }
            }
            Message::UpdateInstalled(id, outcome) => {
                self.installing_update = false;
                match outcome {
                    Ok(_) => return iced::window::close(id),
                    // The install refused, and this is the one that has never
                    // been read: on Windows the swap completes and the relaunch
                    // is what fails, so the user is left looking at the old
                    // build with the reason behind the welcome card. Already
                    // written to the journal by the task that raised it, which
                    // is the only place that knows which step refused.
                    Err(why) => self.update_shows(why),
                }
            }
            Message::UpdateInstallPressed => {
                return iced::window::latest()
                    .and_then(|id| Task::done(Message::UpdateInstallRequested(id)));
            }
            Message::UpdateInstallRequested(id) => {
                if self.downloaded_update.is_none() {
                    // Through the one writer like every other update outcome:
                    // this is reachable from the welcome card's own button, so
                    // the status line alone would put it behind the card.
                    self.update_says(
                        crate::update::journal::Step::Install,
                        "download the update first".to_string(),
                    );
                    return Task::none();
                }
                return self.guard(PendingAction::Restart(id));
            }
            Message::UpdateCheckSet(on) => {
                // `Always` rather than `Welcome` when switched back on, matching
                // the default settled 2026-08-30: the person who most needs to
                // hear their build is stale is the one who turned the welcome
                // screen off, so tying the check to that screen would silence it
                // for exactly them.
                self.update_settings.when =
                    if on { crate::update::When::Always } else { crate::update::When::Never };
                crate::update::store(&self.update_settings);
                // Turning it off also clears what a past check said. Leaving the
                // card up would be an update notice on a screen whose switch
                // now reads "off", which is the sort of thing that gets filed as
                // "it still checks".
                if !on {
                    self.offer = None;
                }
            }
            Message::WelcomeOnStartupSet(show) => {
                self.welcome_on_startup = show;
                crate::welcome::set_on_startup(show);
            }
            Message::SymmetryAxisToggled(axis) => self.toggle_mirror(axis),
            // See `app/radial.rs`.
            Message::Radial(message) => self.radial_changed(message),
            Message::PatternChanged(kind) => self.brush.pattern.kind = kind,
            Message::PatternScaleChanged(scale) => self.brush.pattern.scale_mm = scale,
            Message::PatternDepthChanged(depth) => self.brush.pattern.depth = depth,
            Message::SizingStarted(what) => {
                // Nothing to anchor the gesture to until the pointer has been
                // seen; and a gesture already running must not restart, or
                // holding the key through a drag would reset it every event.
                if self.sizing.is_none()
                    && let Some(from_pixel) = self.cursor
                {
                    self.sizing = Some(Sizing {
                        what,
                        from_pixel,
                        original: match what {
                            SizingTarget::Radius => self.brush.radius,
                            SizingTarget::Strength => self.brush.strength,
                        },
                    });
                }
            }
            Message::SizingEnded => self.sizing = None,
            Message::TopMenuToggled(which) => {
                // Clicking the open one closes it, which is what a menu bar
                // does everywhere.
                self.top_menu = (self.top_menu != Some(which)).then_some(which);
            }
            Message::DiagnosticsCopied => {
                let report = self.diagnostics();
                self.status = "diagnostics copied".to_string();
                self.top_menu = None;
                return iced::clipboard::write(report);
            }
            Message::IssueOpened => {
                self.top_menu = None;
                self.status = format!("report bugs at {ISSUE_URL}");
                log::info!("{}", self.diagnostics());
            }
            Message::OpenInSlicer => {
                self.top_menu = None;
                self.hand_to_slicer();
            }
            Message::PrinterChecked => {
                self.top_menu = None;
                let Some(printer) =
                    crate::printer::configured(crate::printer::config_path().as_deref())
                else {
                    // Says where to write it rather than only that it is
                    // missing: a setting with no interface needs its file named
                    // or it may as well not exist.
                    self.status = match crate::printer::config_path() {
                        Some(path) => format!(
                            "no printer set — put `host = 192.0.2.46` in {}",
                            path.display()
                        ),
                        None => "could not work out where the printer config lives".to_string(),
                    };
                    return Task::none();
                };
                self.status = format!("asking {}…", printer.host);
                return Task::perform(
                    async move {
                        crate::printer::status(&printer.host, printer.port)
                            .map(|status| status.summary())
                    },
                    Message::PrinterAnswered,
                );
            }
            Message::PrinterAnswered(answer) => {
                self.status = match answer {
                    Ok(summary) => summary,
                    Err(why) => format!("could not reach the printer: {why}"),
                };
            }
            Message::PaletteSyncRequested => {
                self.top_menu = None;
                let Some(printer) =
                    crate::printer::configured(crate::printer::config_path().as_deref())
                else {
                    // Same wording as `PrinterChecked`, and for the same
                    // reason: a setting with no interface needs its file named.
                    self.status = match crate::printer::config_path() {
                        Some(path) => format!(
                            "no printer set — put `host = 192.0.2.46` in {}",
                            path.display()
                        ),
                        None => "could not work out where the printer config lives".to_string(),
                    };
                    return Task::none();
                };
                self.status = format!("asking {} what is loaded…", printer.host);
                return Task::perform(
                    async move { crate::printer::filaments(&printer.host, printer.port) },
                    Message::PaletteSynced,
                );
            }
            Message::PaletteSynced(answer) => {
                self.status = match answer {
                    Ok(Some(reported)) => {
                        let skipped = self.palette.sync_from_printer(&reported);
                        let loaded = reported.len() - skipped.len();
                        // The pictures in the body list are drawn through the
                        // palette and refresh on brick changes only, so a
                        // painted body's thumbnail would keep the old colours
                        // while the viewport showed the new ones.
                        self.thumbs.stale_everything();
                        let plural = |count: usize| if count == 1 { "" } else { "s" };
                        // Saved here rather than on quit: this is the only
                        // moment the palette is known to match the machine, and
                        // a crash between here and quitting would lose the one
                        // thing the user pressed a button to get.
                        let mut said = match self.palette.save() {
                            Ok(()) => format!("synced {loaded} filament{}", plural(loaded)),
                            Err(why) => format!("synced, but not saved: {why}"),
                        };
                        // Never a silent partial sync: an empty head is left
                        // alone deliberately, and the user should hear which.
                        if !skipped.is_empty() {
                            let list: Vec<String> = skipped.iter().map(usize::to_string).collect();
                            said.push_str(&format!(
                                " — slot{} {} empty, left alone",
                                plural(skipped.len()),
                                list.join(", ")
                            ));
                        }
                        said
                    }
                    Ok(None) => "that printer does not report which filament is loaded".to_string(),
                    Err(why) => format!("could not reach the printer: {why}"),
                };
            }
            Message::BugReportOpened => {
                self.top_menu = None;
                self.bug_report = Some(BugReport::new());
            }
            Message::BugReportEdited(action) => {
                if let Some(draft) = &mut self.bug_report {
                    draft.description.perform(action);
                }
            }
            Message::BugReportDetailToggled(on) => {
                if let Some(draft) = &mut self.bug_report {
                    draft.with_detail = on;
                }
            }
            Message::BugReportDismissed => self.bug_report = None,
            Message::BugReportCopied => {
                let Some(report) = self.assemble_report() else {
                    self.status =
                        "could not build the report: describe the problem first".to_string();
                    return Task::none();
                };
                self.status = "report copied — paste it into an issue".to_string();
                return iced::clipboard::write(report.body());
            }
            // A new issue on GitHub, opened in the browser with as much of
            // the report as a link carries, and the whole of it on the
            // clipboard. See `report.rs`.
            Message::BugReportSubmitted => {
                let Some(report) = self.assemble_report() else {
                    self.status = "could not open an issue: describe the problem first".to_string();
                    return Task::none();
                };
                self.bug_report = None;
                let (link, cut) = report.link();
                self.status = match crate::links::open_in_browser(&link) {
                    Ok(()) if cut => {
                        "opened a new issue on GitHub; it was cut to fit, and the whole report \
                         is on the clipboard"
                            .to_string()
                    }
                    Ok(()) => "opened a new issue on GitHub; the report is on the clipboard too"
                        .to_string(),
                    Err(why) => {
                        format!(
                            "could not open the issue page ({why}); the report is on the clipboard"
                        )
                    }
                };
                return iced::clipboard::write(report.body());
            }
            Message::KeyPressed { key, modifiers } => return self.on_key(key, modifiers),
            // **Deliberately empty, and it must stay empty.** The work this
            // message does happens before `dispatch` is called at all: it is
            // not on `keeps_the_rename_open`'s list, so `Hani::update`
            // commits the rename in flight and then arrives here with nothing
            // left to do. Giving it a body would make every click on the panel
            // background do something, which is the opposite of what it is.
            Message::PressedNothing => {}
            Message::MenuClosed => {
                // Escape is the only sender, by way of `on_key`. Against an
                // open prompt it means Cancel, which is the harmless answer --
                // an explicit arm rather than letting the clears below reach
                // `confirm`, since dismissing the prompt by accident is how
                // work gets lost.
                if self.confirm.is_some() {
                    return self.answer_confirm(ConfirmChoice::Cancel);
                }
                // Same reasoning one card down: Escape against the delete
                // prompt means "don't", which is the harmless answer.
                if self.pending_delete.take().is_some() {
                    return Task::none();
                }
                // And the merge prompt, which asks the same kind of question
                // about the same kind of size. Escape means "don't", which
                // again changes nothing.
                if self.pending_merge.take().is_some() {
                    return Task::none();
                }
                // And the split preview, which is the third card of the same
                // family. Escape means "don't", and the walk it throws away
                // cost nothing but time.
                if self.pending_split.take().is_some() {
                    return Task::none();
                }
                // And one card further down: Escape against a rename means
                // "keep the old name", which is the harmless answer, and it
                // must not also disarm the cut on its way past.
                //
                // **This is the SECOND Escape, not the first, and the plan says
                // otherwise.** A focused `text_input` handles Escape itself --
                // it clears its own focus and captures the event
                // (`text_input.rs:1235-1244`) -- so the first press never
                // reaches `key_event` at all. The field is left open and
                // unfocused, still showing what was typed, and the next Escape
                // arrives here. Nothing in the widget tree can be put in front
                // of that: iced hands an event to the content before the
                // container, so the innermost widget always wins.
                if self.renaming.is_some() {
                    self.cancel_rename();
                    return Task::none();
                }
                // The palette of brushes at the pointer, and a measurement
                // being taken: Escape puts either away, which changes
                // nothing. See `app/studio.rs` and `app/measure.rs`.
                if self.close_palette() || self.measure_escaped() || self.revolve_escaped() {
                    return Task::none();
                }
                // The bodies drawn apart come back together. See
                // `app/explode.rs`.
                if self.exploded() {
                    self.unexplode();
                    return Task::none();
                }
                // And one further down again: solo, BEFORE the armed cut.
                //
                // The order is the point. Both are modes, and with both on, one
                // Escape has to pick -- so it picks the one whose exit costs
                // nothing. Leaving solo puts nineteen bodies back on screen and
                // changes not a byte; disarming the cut throws away a mode the
                // user deliberately armed and would have to arm again. The
                // second Escape then reaches the cut, which is the order every
                // other card here already follows: the harmless answer first.
                //
                // **This is also solo's only exit that is not a document
                // change.** `ctrl+alt+comma` leaves the mode too, but it turns
                // every eye on as it goes -- silently revealing the six bodies
                // the user hid an hour ago -- and exiting a mode must not be an
                // edit. That is why Escape is here rather than left to it.
                if self.solo.is_some() {
                    self.exit_solo();
                    return Task::none();
                }
                // Others see-through, the same way. See `app/explode.rs`.
                if self.see_through_escaped() {
                    return Task::none();
                }
                // Escape is also the way out of an armed cut.
                //
                // **And out of the cut ONLY, never out of mask mode.** A cut is
                // a pending destructive thing and Escape is the standard way to
                // call one off. A mask is neither pending nor destructive: it is
                // expensive user work, and Escape is also the key that closes
                // every menu below, so a user dismissing a dropdown would
                // silently drop the mode they are painting in. The next reader
                // will want to add it here for symmetry; this paragraph is why
                // not.
                // Escape ABANDONS the gizmo, where choosing another tool
                // commits it. Ahead of the cut in the cascade for the reason
                // every card here is ordered by: this one is the answer to a
                // question the user can see being asked -- a body sitting
                // somewhere it was dragged to -- and the cut is a mode with
                // nothing pending on screen.
                //
                // A LIVE drag is cancelled first and on its own, without
                // disarming: Escape mid-drag means "not that one", not "not any
                // of it", and the gesture is undone by throwing it away rather
                // than by inverting it -- `pinned` is what the placement was
                // before the button went down.
                if let Some(gizmo) = &mut self.gizmo
                    && gizmo.grabbed.is_some()
                {
                    gizmo.placement = gizmo.pinned;
                    gizmo.grabbed = None;
                    self.drag = None;
                    self.refresh_gizmo();
                    self.status = "cancelled the drag".to_string();
                    return Task::none();
                }
                if self.tool == Tool::Transform {
                    self.disarm_gizmo(false);
                    self.tool = Tool::Sculpt;
                    return Task::none();
                }
                // A ring or the cut it proposed is dropped, the tool still in
                // hand. See `app/ring_cut.rs`.
                if self.tool == Tool::Cut && self.ring_escaped() {
                    self.cut_path.clear();
                    self.drag = None;
                    return Task::none();
                }
                if self.tool == Tool::Cut {
                    // A LIVE path is thrown away without disarming, exactly as
                    // a live gizmo drag is above and for the same reason:
                    // Escape mid-gesture means "not that shape", not "not any
                    // cut". The tool stays armed so the next drag is still a
                    // cut, which is what a user who mis-drew a lasso wants.
                    if !self.cut_path.is_empty() {
                        self.cut_path.clear();
                        self.drag = None;
                        self.refresh_overlay();
                        self.status = "cancelled the cut".to_string();
                        return Task::none();
                    }
                    self.tool = Tool::Sculpt;
                }
                self.adding = false;
                self.menu = None;
                self.top_menu = None;
                self.cube_menu = None;
                // Unlike `confirm` above, this one IS cleared: declining to
                // turn the model is the safe answer and leaves it exactly as
                // imported, where declining to answer "you have unsaved work"
                // loses the work.
                self.orient_prompt = None;
            }
            Message::FieldEdited(key, typed) => {
                self.drafts.insert(key, typed);
            }
            Message::FieldDropped(key) => {
                self.drafts.remove(key);
            }
            // Unwrapped in `update` before it reaches here. Kept as an arm
            // rather than `unreachable!`, so a test that dispatches one
            // directly still sees the message it carries acted on.
            Message::FieldSet(key, carried) => {
                self.drafts.remove(key);
                return self.dispatch(*carried);
            }
            Message::OrientFace(to) => {
                // Taken rather than read: the menu asked its question and has
                // been answered, and leaving it open over a model that just
                // moved would invite a second turn from a face that is no
                // longer there.
                if let Some(menu) = self.cube_menu.take() {
                    self.orient(hani_core::AxisRotation::taking(menu.facing, to));
                }
            }
            Message::OrientPromptAnswered(accept) => {
                if let Some(up) = self.orient_prompt.take()
                    && accept
                {
                    self.orient(hani_core::AxisRotation::taking(up, hani_core::Facing::Up));
                }
            }
            // Pressing the live tool goes back to sculpting, which is what
            // makes one variant cover arming and disarming both.
            Message::ToolChanged(tool) => {
                let tool = if tool == self.tool { Tool::Sculpt } else { tool };
                // Leaving the gizmo COMMITS, where Escape abandons. That split
                // is the convention every modelling tool already has, and it is
                // the safe way round: choosing another tool is not a statement
                // about the placement, and silently undoing it would throw away
                // work the user can see on screen.
                if self.tool == Tool::Transform && tool != Tool::Transform {
                    self.disarm_gizmo(true);
                }
                if tool == Tool::Transform
                    && let Err(why) = self.arm_gizmo()
                {
                    // Refused BEFORE the tool changes, so the strip does not
                    // light a mode that has no gizmo under it.
                    self.status = why;
                    return Task::none();
                }
                // Through the swap so the mask's own strength is put away and
                // the brush's is brought back, and vice versa. See `strengths`.
                self.swap_strength(|app| app.tool = tool);
                self.status = match tool {
                    Tool::Cut => {
                        // Names all three readings, because the shape is
                        // inferred and a user who is not told a loop is
                        // available will never draw one. The line is named
                        // first: it is the default, and it is what the tool
                        // did before it learned the other two.
                        "cut armed: drag a line and the left of the arrow goes, or draw a \
                         loop around what to remove"
                            .to_string()
                    }
                    Tool::Mask => "mask: drag to protect, ctrl or alt to unprotect, \
                                   shift to soften"
                        .to_string(),
                    Tool::Transform => "move: drag an arrow, a square or a ring — snapped to \
                                        the voxel and to quarter turns, hold shift for free"
                        .to_string(),
                    Tool::Sculpt => String::new(),
                };
            }
            Message::DynamicRadiusToggled(on) => self.dynamic_radius = on,

            // --- the body panel ----------------------------------------------
            Message::BodySelected(id) => {
                // One press, two jobs: it chooses the row AND picks it up. The
                // row's `mouse_area` has one `on_press` message to give, and
                // the second job costs nothing until the pointer moves.
                self.select_body(id);
                // A folder row takes the folder in hand for the gizmo. See
                // `app/group_gizmo.rs`.
                self.row_pressed(id);
                self.arm_row_drag(id);
            }
            Message::BodyRowDragged { over, fraction } => self.track_row_drag(over, fraction),
            Message::BodyVisibilityToggled(id) => self.toggle_visibility(id),
            Message::ActiveBodyVisibilityToggled => {
                let active = self.doc.active();
                self.toggle_visibility(active);
            }
            Message::EveryBodyShown => self.show_everything(),
            Message::PrimitiveMenuToggled => self.adding = !self.adding,
            Message::PrimitiveAdded(kind) => {
                // Closed whatever happens, including on a refusal: a menu that
                // stays open after a press that was refused reads as a press
                // that never landed.
                self.adding = false;
                let before = self.doc.body_count();
                self.add_primitive(kind);

                // **A primitive arrives with its gizmo already on it**, which
                // is ZBrush's behaviour and the reason Thomas asked for it: you
                // add a cube in order to put it somewhere and give it a shape,
                // so making that the default saves reaching for the tool every
                // single time. It also puts the thing you just made under an
                // obvious handle, which is the clearest possible answer to
                // "where did it go" -- a primitive is placed clear of the
                // model, so it can arrive outside the view.
                //
                // Only when a body was actually added: `add_primitive` refuses
                // on the body ceiling and on the mesh pool, and arming a gizmo
                // over a refusal would light a mode for a body that does not
                // exist. And only from Sculpt, so adding a primitive mid-mask
                // or with a cut armed does not silently change what the next
                // drag does.
                if self.doc.body_count() > before && self.tool == Tool::Sculpt {
                    if let Err(why) = self.arm_gizmo() {
                        // Not a failure of the add, so the add's own status
                        // line survives and this is only logged.
                        log::info!("the new primitive kept the sculpt tool: {why}");
                    } else {
                        self.swap_strength(|app| app.tool = Tool::Transform);
                    }
                }
            }
            Message::BodyDeleted => self.delete_active_body(),
            Message::BodyDuplicated => self.duplicate_active_body(),
            Message::BodyGrouped => self.group_active_body(),
            Message::BodyUngrouped => self.ungroup_active_body(),
            Message::FolderCollapseToggled(id) => self.toggle_collapse(id),
            Message::FolderDeleted(id) => self.delete_folder(id),
            Message::BodyDeleteConfirmed => {
                if let Some(pending) = self.pending_delete.take() {
                    self.remove_body(pending.id);
                }
            }
            Message::BodyDeleteCancelled => {
                // Nothing else: cancelling changes not one byte of the
                // document, which is the whole promise of the prompt.
                self.pending_delete = None;
            }
            Message::BodyMergedDown => self.merge_active_body_down(),
            Message::BodyMergeConfirmed => {
                if let Some(pending) = self.pending_merge.take() {
                    self.apply_merge(pending.source);
                }
            }
            Message::BodyMergeCancelled => {
                // As above: cancelling a merge changes nothing at all, which is
                // the only reason it is safe to ask.
                self.pending_merge = None;
            }
            Message::BodySplit => self.split_active_body(),
            Message::BodySplitConfirmed => {
                if let Some(pending) = self.pending_split.take() {
                    self.apply_split(pending);
                }
            }
            Message::BodySplitCancelled => {
                // The plan goes with it, and with the plan the labelling. That
                // is the whole cost of cancelling: a walk that has to be done
                // again if they press the button once more.
                self.pending_split = None;
            }
            // Session state, so deliberately NOT `unsaved`: nothing about it is
            // written to the file.
            Message::ThumbnailsToggled => {
                self.thumbnails = !self.thumbnails;
                if self.thumbnails {
                    // Nothing was kept up to date while they were off, so every
                    // picture is a guess until it is drawn again. Marking them
                    // stale is not a document change and does not dirty it.
                    self.thumbs.stale_everything();
                }
            }
            // The tint, and nothing but the tint. None of these three marks a
            // brick dirty, touches a protection value or dirties the document:
            // the polarity and the strength are uniforms the shader reads, and
            // the standing card is unconditional and does not consult them. See
            // the `show_mask` field for the whole of that argument.
            Message::ShowMaskToggled => self.show_mask = !self.show_mask,
            Message::MaskTintChanged(tint) => self.mask_tint = tint.clamp(0.0, 1.0),
            Message::CameraFramedOnActive => self.frame_active(),
            Message::MaskPeekStarted => self.mask_peek = true,
            Message::MaskPeekEnded => self.mask_peek = false,
            Message::ShowPaintToggled => self.show_paint = !self.show_paint,
            // A remesh of everything, because the signal travels in the mesh:
            // on, every brick has to be measured, and off, every brick has to
            // stop paying for bytes nothing reads. No voxel changes, so no
            // undo entry and no dirtied document.
            Message::ExportDetailChanged(detail) => self.export_detail = detail,
            // Clamped on arrival to the sliders' own ranges, so a value from
            // anywhere else cannot ask the engine for a key of no size.
            Message::KeySettingChanged(setting) => {
                match setting {
                    KeySetting::Kind(kind) => self.key_spec.kind = kind,
                    KeySetting::Radius(mm) => self.key_spec.radius_mm = mm.clamp(0.5, 6.0),
                    KeySetting::Depth(mm) => self.key_spec.depth_mm = mm.clamp(0.5, 8.0),
                    KeySetting::Clearance(mm) => {
                        self.key_spec.clearance_mm = mm.clamp(0.0, 0.6);
                    }
                    KeySetting::Shape(shape) => self.key_spec.shape = shape,
                }
                // A ring cut's pegs are planned when it is proposed: planned
                // again, so the change shows. See `app/ring_cut.rs`.
                self.replan_ring_keys();
            }
            // With paints in use the meshes carry the signal whatever this
            // says, because a wash reads it, and then this is the view toggle
            // it looks like and there is nothing to remesh.
            Message::ShowCavityToggled => {
                self.show_cavity = !self.show_cavity;
                if self.doc.paints().is_empty() {
                    self.doc.mark_everything_dirty();
                    self.remesh_dirty();
                }
            }
            Message::Overhang(message) => self.overhang_changed(message),
            Message::Painting(message) => self.painting_changed(message),
            Message::Layers(message) => self.layers_changed(message),
            Message::Rack(message) => self.rack_changed(message),
            // See `app/designer.rs`.
            Message::Designer(message) => self.designer_changed(message),
            // See `app/cores.rs`.
            Message::Cores(message) => return self.cores_changed(message),
            // See `app/ring_cut.rs`.
            Message::Ring(message) => self.ring_changed(message),
            Message::Airbrush(message) => self.airbrush_changed(message),
            Message::Coats(message) => self.coats_changed(message),
            Message::Decal(message) => return self.decal_changed(message),
            Message::Studio(message) => return self.studio_changed(message),
            Message::Form(message) => self.form_changed(message),
            // See `app/hollow.rs`.
            Message::Hollow(message) => self.hollow_changed(message),
            // See `app/deform.rs`.
            Message::Deform(message) => self.deform_changed(message),
            Message::Reference(message) => return self.reference_changed(message),
            Message::Keep(message) => self.keep_changed(message),
            Message::Scale(message) => self.scale_changed(message),
            Message::Turn(message) => self.turn_changed(message),
            // See `app/revolve.rs`.
            Message::Revolve(message) => self.revolve_changed(message),
            Message::Measure(message) => self.measure_changed(message),
            Message::Add(message) => self.add_changed(message),
            // See `app/maps.rs`.
            Message::Maps(message) => return self.maps_changed(message),
            Message::Workbench(message) => self.workbench_changed(message),
            // One of the sizes on offer, or the setting stays as it is.
            Message::TextureSizeChanged(size) => {
                if paints::TEXTURE_SIZES.contains(&size) {
                    self.texture_size = size;
                }
            }
            Message::FlatPaintToggled => self.flat_paint = !self.flat_paint,
            Message::PaintTouchChosen(touch) => {
                self.touch.kind = touch;
                // A wash or a contrast goes on a layer of its own blend. See
                // `app/layers.rs`.
                match touch {
                    crate::message::PaintTouch::Wash => {
                        self.suit_the_layer("Wash", hani_core::paint::layers::Blend::Wash, false)
                    }
                    crate::message::PaintTouch::Contrast => self.suit_the_layer(
                        "Contrast",
                        hani_core::paint::layers::Blend::Contrast,
                        false,
                    ),
                    _ => {}
                }
            }
            Message::PaintTouchChanged(setting) => {
                self.touch.set(setting);
                // What a drybrush catches follows its Catch. See
                // `app/dry_preview.rs`.
                self.refresh_overlay();
            }
            // And the verbs, which DO change protection and all of which are
            // undoable in one entry apiece.
            Message::MaskCleared => self.clear_mask(),
            Message::MaskInverted => self.invert_mask(),
            Message::MaskAllApplied => self.mask_all(),
            Message::MaskFilterChosen(filter) => {
                // A live gesture belongs to the filter it started under, so
                // this commits it rather than switching under it.
                self.commit_mask_filter();
                self.mask_amount = 0.0;
                self.mask_filter = filter;
            }
            Message::MaskAmountChanged(amount) => self.mask_amount_changed(amount),
            Message::MaskAmountReleased => self.mask_amount_released(),
            Message::MaskGenerated(which) => self.generate_mask(which),
            Message::MaskFeatureChanged(mm) => {
                self.mask_feature_mm =
                    mm.clamp(*MASK_FEATURE_RANGE_MM.start(), *MASK_FEATURE_RANGE_MM.end());
            }
            Message::MaskThicknessChanged(voxels) => {
                self.mask_thickness_voxels =
                    (voxels.round().max(1.0) as u32).min(hani_core::MAX_THICKNESS_VOXELS);
            }
            Message::BodySplitMasked => self.split_the_mask_off(),
            // A view mode, and likewise not `unsaved` -- with the one exception
            // `enter_solo` documents, where soloing a row whose eye is off turns
            // that eye on and THAT is an ordinary edit.
            Message::SoloEntered(id) => self.enter_solo(id),
            Message::SoloExited => self.exit_solo(),
            // See `app/explode.rs`.
            Message::ExplodeToggled => self.toggle_explode(),
            Message::SoloActiveToggled => self.toggle_solo_active(),
            Message::OthersSeeThroughToggled => self.toggle_see_through(),
            Message::BodyRenameBegan(id) => return self.begin_rename(id),
            Message::BodyRenameEdited(text) => {
                if let Some((_, held)) = &mut self.renaming {
                    // Clamped as it is typed, and not at commit. The field's
                    // job is to show what the file will hold: a thirty-third
                    // byte that looks accepted and is dropped on the next save
                    // is exactly the silent loss `name_that_fits` exists to
                    // stop, moved one layer up where it is more convincing.
                    held.clear();
                    held.push_str(hani_core::name_that_fits(&text));
                }
            }
            // Nothing, and that is not an oversight: `update`'s guard has
            // already committed by the time this arrives, because
            // `keeps_the_rename_open` does not list it. Committing here as well
            // would be a second commit point, which is the one thing the guard
            // exists to prevent.
            Message::BodyRenameSubmitted => {}

            Message::BrushRadiusScaled(factor) => {
                self.brush.radius =
                    (self.brush.radius * factor).clamp(MIN_RADIUS_MM, self.max_radius());
            }
            Message::PressureToggled(on) => self.pressure_enabled = on,
            Message::PressureCurveChanged(curve) => self.pressure_curve = curve,
            Message::TiltToggled(on) => self.tilt_enabled = on,
            Message::ResetPressurePeak => self.tablet.reset_peak(),
            // Taken at the top, past the guards. See `app/pen.rs`.
            Message::PenSample(state) => self.pen_sample(state),
            Message::Undo => self.undo(),
            Message::Redo => self.redo(),
            Message::Export(format) => {
                return Task::perform(pick_export_target(format), move |path| {
                    Message::ExportChosen(format, path)
                });
            }
            Message::Resample(voxel_size) => self.resample(voxel_size),
            Message::WorkingSizeTyped(text) => self.working_size_field = text,
            Message::WorkingSizeCommitted => {
                match self.working_size_field.trim().parse::<f32>() {
                    Ok(mm) => self.set_working_size(mm),
                    Err(_) => {
                        self.status = "could not resize: type a size in millimetres".into();
                    }
                }
                self.working_size_field.clear();
            }
            Message::SpaceMouse(setting) => self.configure_spacemouse(setting),
            Message::SectionToggled(section) => {
                let open = &mut self.expanded[section as usize];
                *open = !*open;
            }
            Message::StatsToggled => self.stats_open = !self.stats_open,
            Message::ViewRebuilt => self.rebuild_view(),
            // Both the panel button and File > New come here, so the two
            // cannot drift apart. Both discard the document, so both ask.
            Message::ResetSphere | Message::NewSculpt => {
                return self.guard(PendingAction::NewSculpt);
            }

            // --- the unsaved-work prompt -------------------------------------
            // The window is undecorated, so these four are the title bar the
            // compositor is no longer drawing. Each needs the window's id and
            // none of them has it, so each asks for it first.
            Message::TitleBarDragged => {
                return iced::window::latest().and_then(iced::window::drag);
            }
            // In a browser the host has the window: told, and the page left
            // alone. See `web::tell_host`.
            Message::TitleBarDoubleClicked if cfg!(target_arch = "wasm32") => {
                crate::web::tell_host("maximise");
            }
            Message::WindowMinimise if cfg!(target_arch = "wasm32") => {
                crate::web::tell_host("minimise");
            }
            Message::TitleBarDoubleClicked => {
                return iced::window::latest().and_then(iced::window::toggle_maximize);
            }
            Message::WindowMinimise => {
                return iced::window::latest().and_then(|id| iced::window::minimize(id, true));
            }
            // Routed through CloseRequested rather than closing directly, so
            // the unsaved-work prompt is the same one the compositor's close
            // button used to reach.
            Message::ResizeStarted(direction) => {
                return iced::window::latest()
                    .and_then(move |id| iced::window::drag_resize(id, direction));
            }
            Message::WindowClose => {
                return iced::window::latest()
                    .and_then(|id| Task::done(Message::CloseRequested(id)));
            }
            // Not asked in a browser tab, where there is no quitting: see the
            // `Quit` arm of `run_pending`. A hosting webview is asked for.
            Message::CloseRequested(_)
                if cfg!(target_arch = "wasm32") && !crate::web::window_buttons() => {}
            Message::CloseRequested(id) => return self.guard(PendingAction::Quit(id)),
            Message::ConfirmAnswered(choice) => return self.answer_confirm(choice),
            Message::SavedThenContinue(path) => {
                let Some(path) = path else {
                    // The save dialog was dismissed. That is not an answer to
                    // the prompt, so the prompt stays up.
                    return Task::none();
                };
                self.save_project(&path);
                if self.unsaved {
                    // The write failed and `status` says why. Leave the prompt
                    // up rather than quitting on a file that was never written.
                    return Task::none();
                }
                if let Some(action) = self.confirm.take() {
                    return self.run_pending(action);
                }
            }

            // --- files -------------------------------------------------------
            // Each is two halves: ask for a path off the UI thread, then act on
            // whatever came back. A `None` means the dialog was dismissed, which
            // is not an error and must leave everything alone.
            Message::OpenRequested => return self.guard(PendingAction::Open),
            Message::OpenRecent(path) => return self.guard(PendingAction::OpenRecent(path)),
            Message::RecentCleared => {
                self.recent.clear();
                self.top_menu = None;
                self.status = "recent files cleared".to_string();
            }
            Message::RecoverAutosave => return self.guard(PendingAction::RecoverAutosave),

            // --- importing a mesh --------------------------------------------
            Message::ImportRequested => return self.guard(PendingAction::Import),
            Message::ImportChosen(path) => {
                let Some(path) = path else {
                    return Task::none();
                };
                self.status = format!("importing {}…", path.display());
                let voxel_size = self.doc.voxel_size();
                // Off the update loop, unlike open and save. Measured by
                // `cargo bench -p hani-core --bench import`: a 542k triangle
                // sphere reads in 62 ms and voxelises in 148 to 210 ms
                // depending on voxel size -- so not the seconds this was first
                // written for, but still twelve dropped frames if it ran on the
                // thread that draws, and a scan or a fine voxel size scales it
                // straight past that.
                return Task::perform(
                    async move {
                        let started = Instant::now();
                        // A file picked in a browser is held by name. See
                        // `files.rs`.
                        let outcome = crate::files::read_mesh(&path).and_then(|mesh| {
                            // Measured on the mesh, because by the time there
                            // is a volume the mesh is gone and its bounding box
                            // has been centred on the origin -- which destroys
                            // the one tell there is.
                            let resting_up = hani_core::resting_up(&mesh.positions);
                            // `already_reserved` stays at zero, which is what
                            // `at` gives and what a REPLACING import needs:
                            // `adopt_import` swaps the whole document and
                            // `rebuild_everything` empties the pool, so the
                            // model on screen while this runs is not competing
                            // with the one being built. An import that JOINS a
                            // document -- "Import as a new body", deferred out
                            // of this arc -- has to pass the pool's watermark
                            // here instead, or the two will overflow it
                            // between them with nothing reporting it.
                            let options = hani_core::voxelise::VoxeliseOptions::at(voxel_size);
                            // Asked of the MESH, not of the document. The
                            // document's size is whatever the last thing on
                            // screen was built at -- 0.25 mm for the starting
                            // ball -- and inheriting it meant a three million
                            // triangle model was voxelised at the resolution
                            // chosen for a sphere, and that the same file
                            // imported twice gave two different results.
                            let options = hani_core::voxelise::VoxeliseOptions::at(
                                hani_core::voxelise::voxel_for_mesh(&mesh, &options),
                            );
                            hani_core::voxelise::voxelise(&mesh, &options).map(
                                |(volume, report)| crate::message::Imported {
                                    volume,
                                    report,
                                    source: path.clone(),
                                    elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
                                    resting_up,
                                },
                            )
                        });
                        crate::message::ImportPayload::new(outcome)
                    },
                    Message::ImportLoaded,
                );
            }
            Message::ImportLoaded(payload) => {
                match payload.take() {
                    Some(Ok(imported)) => self.adopt_import(imported),
                    // The "could not" prefix is load bearing: the header colours
                    // the status line by that substring, so a message without it
                    // renders in muted grey as though the import had worked.
                    Some(Err(error)) => {
                        self.status = format!("could not import: {error}");
                        log::error!("{}", self.status);
                    }
                    // Already taken, which can only happen if the message were
                    // delivered twice. Nothing to do, and nothing is lost.
                    None => {}
                }
            }
            Message::OpenChosen(path) => {
                if let Some(path) = path {
                    self.open_project(&path);
                }
            }
            // In a browser, kept there. See `app/shelf.rs`.
            #[cfg(target_arch = "wasm32")]
            Message::SaveRequested => {
                return self.shelf_save(crate::message::AfterSave::Stay);
            }
            #[cfg(target_arch = "wasm32")]
            Message::SaveAsRequested => {
                return self.shelf_changed(crate::message::ShelfMessage::Opened);
            }
            #[cfg(not(target_arch = "wasm32"))]
            Message::SaveRequested => match self.project_path.clone() {
                // Save over the file it came from; with no file yet this is a
                // Save As, which is what a first Save should do.
                Some(path) => self.save_project(&path),
                None => return Task::perform(pick_project_to_save(), Message::SaveChosen),
            },
            #[cfg(not(target_arch = "wasm32"))]
            Message::SaveAsRequested => {
                return Task::perform(pick_project_to_save(), Message::SaveChosen);
            }
            Message::SaveChosen(path) => {
                if let Some(path) = path {
                    self.save_project(&path);
                }
            }
            Message::ExportRequested(format) => {
                return Task::perform(pick_export_target(format), move |path| {
                    Message::ExportChosen(format, path)
                });
            }
            Message::ExportChosen(format, path) => {
                if let Some(path) = path {
                    self.export(format, &path);
                }
            }
            Message::FitTestRequested => return self.fit_test_requested(),
            Message::FitTestChosen(path) => {
                if let Some(path) = path {
                    self.write_fit_test(&path);
                }
            }
        }
        Task::none()
    }
}

impl Default for Hani {
    /// The application without its boot task, which is what a `Default` can
    /// promise. `Hani::new` is what `main` uses, and it returns the work to
    /// be done before the first frame as well.
    fn default() -> Self {
        Self::new().0
    }
}

#[cfg(test)]
mod camera_tests;
#[cfg(test)]
mod cut_tests;
#[cfg(test)]
mod import_tests;
#[cfg(test)]
mod prompt_tests;
#[cfg(test)]
mod saving_tests;
#[cfg(test)]
mod stroke_tests;
#[cfg(test)]
mod test_support;
